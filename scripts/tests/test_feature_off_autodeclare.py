#!/usr/bin/env python3
"""
[OPUS-5] sq-v3nel-v3: test suite for scripts/feature_off_autodeclare.py.

The tool decides whether a feature-OFF wasm-bundle difference is line-position churn
(comments moved, an off-by-default `#[cfg]` item inserted) or a real change to the code the
default build compiles. Getting that wrong in the permissive direction turns the
`artifact-exact-equality` gate into a rubber stamp, so this suite is built around ONE
question: does a NON-benign drift still get REFUSED?

Two things make the suite non-vacuous:

  * A FAKE COMPILER (`fake_build`) that reproduces the real phenomenon rather than
    hand-waving it. It emits, per COMPILED source line, a FIXED-WIDTH record carrying that
    line's POSITION plus a hash of its content. Comments, blanks and lines under an inactive
    `#[cfg(feature = "...")]` emit nothing but still occupy a position. Consequently a
    comment insertion produces a SAME-LENGTH, different-bytes artefact (delta 0) and a real
    code insertion produces a LONGER one — which is exactly what `cargo build --profile
    release-wasm -p sparq-wasm` was MEASURED to do on this repo (34 inserted comment lines
    in crates/sparq-core/src/compress.rs => 3 differing bytes at delta 0; four ungated code
    lines in crates/sparq-core/src/lib.rs => delta +228 and 373,027 differing bytes).

  * A MUTATION HARNESS (`--mutate`). It removes ONE call site of the decision logic at a
    time and asserts that a NAMED test for that site goes red. A suite-wide "something
    fails" assertion would give 1/N coverage at N/N confidence; each mutant here names the
    single test that must catch it.

Usage:
    python3 scripts/tests/test_feature_off_autodeclare.py            # run the suite
    python3 scripts/tests/test_feature_off_autodeclare.py --mutate   # + the mutation table
"""

from __future__ import annotations

import hashlib
import json
import importlib.util as _ilu
import os
import re
import shutil
import subprocess
import sys
import tempfile

_SCRIPTS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_MODULE_PATH = os.path.join(_SCRIPTS, "feature_off_autodeclare.py")


def _load(path: str = _MODULE_PATH):
    spec = _ilu.spec_from_file_location("feature_off_autodeclare", path)
    mod = _ilu.module_from_spec(spec)  # type: ignore[arg-type]
    spec.loader.exec_module(mod)  # type: ignore[union-attr]
    return mod


A = _load()

_FAILURES: list[str] = []
_RUN: list[str] = []


def check(name: str, cond: bool, detail: str = "") -> None:
    _RUN.append(name)
    if not cond:
        _FAILURES.append(f"{name}: {detail}")


# ---------------------------------------------------------------------------
# The fake compiler.
# ---------------------------------------------------------------------------

_CFG_FEATURE = re.compile(r'#\[cfg\(feature\s*=\s*"([^"]+)"\)\]')
_DEFAULT_FEATURES = re.compile(r'^\s*default\s*=\s*\[([^\]]*)\]', re.M)
_INCLUDE_STR = re.compile(r'include_str!\("([^"]+)"\)')


def _enabled_features(tree: str) -> set[str]:
    """Features listed in the root Cargo.toml's `default = [...]`."""
    manifest = os.path.join(tree, "Cargo.toml")
    if not os.path.exists(manifest):
        return set()
    with open(manifest, encoding="utf-8") as fh:
        m = _DEFAULT_FEATURES.search(fh.read())
    if not m:
        return set()
    return {t.strip().strip('"') for t in m.group(1).split(",") if t.strip()}


def fake_build(tree: str) -> bytes | None:
    """Model of rustc for the feature-OFF wasm bundle.

    Emits one FIXED-WIDTH record per compiled line: `path:LINE(6 digits):HASH(16 hex)`.
    Because the record width does not depend on the line number's magnitude, moving a
    compiled line changes BYTES but never LENGTH — the real `core::panic::Location`
    behaviour this tool exists to attribute. A line that emits nothing (blank, `//`
    comment, or governed by an inactive `#[cfg(feature = ...)]`) still consumes its
    position, so it shifts everything below it.

    Returns None when the tree does not "compile": a `#[cfg(...)]` attribute with no item
    under it, mirroring a build failure.
    """
    enabled = _enabled_features(tree)
    out: list[str] = []
    included: list[str] = []
    roots = []
    for dirpath, dirnames, filenames in os.walk(tree):
        dirnames[:] = [d for d in dirnames if d not in ("target", ".git")]
        for fn in filenames:
            if fn.endswith(".rs"):
                roots.append(os.path.join(dirpath, fn))
    for path in sorted(roots):
        rel = os.path.relpath(path, tree)
        with open(path, encoding="utf-8", errors="surrogateescape") as fh:
            lines = fh.read().split("\n")
        gated_off = False
        for i, raw in enumerate(lines, start=1):
            s = raw.strip()
            m = _CFG_FEATURE.match(s)
            if m:
                if i == len(lines) or not any(l.strip() for l in lines[i:]):
                    return None  # dangling cfg attribute => does not compile
                gated_off = m.group(1) not in enabled
                continue
            if not s or s.startswith("//"):
                continue
            if gated_off:
                gated_off = False
                continue
            gated_off = False
            inc = _INCLUDE_STR.search(s)
            if inc:
                # `include_str!` pulls a NON-.rs file's bytes into the artefact — the case
                # that makes "decide inertness by file extension" unsound.
                target = os.path.normpath(os.path.join(os.path.dirname(path), inc.group(1)))
                if os.path.exists(target):
                    with open(target, encoding="utf-8", errors="surrogateescape") as fh2:
                        body = "".join(l for l in fh2.read().split("\n") if l.strip())
                    included.append(hashlib.sha256(body.encode()).hexdigest()[:16])
            h = hashlib.sha256(s.encode("utf-8", "surrogateescape")).hexdigest()[:16]
            out.append(f"{rel}:{i:06d}:{h}")
    return ("\n".join(out + included)).encode()


# ---------------------------------------------------------------------------
# Tree + diff fixtures.
# ---------------------------------------------------------------------------

_ROOT_MANIFEST = '[workspace]\n[features]\ndefault = ["core"]\ngated = []\n'

_BASE_LIB = """\
// crate root
pub fn alpha(x: u64) -> u64 {
    x.wrapping_mul(3)
}

pub fn beta(x: u64) -> u64 {
    x.wrapping_add(7)
}

pub fn gamma(x: u64) -> u64 {
    alpha(x) ^ beta(x)
}
"""


def make_tree(files: dict[str, str]) -> str:
    d = tempfile.mkdtemp(prefix="featoff-test-")
    for rel, content in files.items():
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(content)
    return d


def base_files(**over: str) -> dict[str, str]:
    f = {"Cargo.toml": _ROOT_MANIFEST, "crates/c/src/lib.rs": _BASE_LIB}
    f.update(over)
    return f


def diff_of(base: str, head: str) -> str:
    """A real `git diff --no-index -U0` between two trees, so the parser under test is
    exercised on genuine git output rather than a hand-written approximation."""
    proc = subprocess.run(
        ["git", "diff", "--no-index", "--no-color", "--unified=0", base, head],
        capture_output=True, text=True,
    )
    # Rewrite the temp-dir prefixes to repo-relative paths.
    out = proc.stdout
    out = out.replace(base.lstrip("/") + "/", "").replace(head.lstrip("/") + "/", "")
    out = out.replace(base + "/", "").replace(head + "/", "")
    return out


def run_decide(base_files_d: dict[str, str], head_files_d: dict[str, str], builder=fake_build):
    base = make_tree(base_files_d)
    head = make_tree(head_files_d)
    try:
        return A.decide(base, head, diff_of(base, head), builder)
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def insert_at(text: str, line: int, new_lines: list[str]) -> str:
    L = text.split("\n")
    return "\n".join(L[:line] + new_lines + L[line:])


# ---------------------------------------------------------------------------
# CONTROL arm — genuinely benign, cfg/comment-only drift must be DECLARED.
# ---------------------------------------------------------------------------

def test_comment_only_drift_is_declared() -> None:
    """CONTROL: pure comment insertion mid-file drifts the bundle but must be declared."""
    head = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["// explanatory comment " + str(i) for i in range(12)])})
    v = run_decide(base_files(), head)
    check("test_comment_only_drift_is_declared", v.outcome == A.OUTCOME_DECLARED,
          f"got {v.outcome}: {v.reason}")
    check("test_comment_only_drift_is_declared__delta_zero",
          v.evidence.get("size_delta_bytes") == 0,
          f"expected delta 0, got {v.evidence.get('size_delta_bytes')}")
    check("test_comment_only_drift_is_declared__actually_drifted",
          v.evidence.get("differing_bytes", 0) > 0,
          "the fixture must actually move bytes, else the control is vacuous")


def test_inactive_cfg_item_drift_is_declared() -> None:
    """CONTROL: an off-by-default `#[cfg(feature)]` item inserted mid-file is declared."""
    head = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4,
        ["// the measure-first arm, off by default",
         '#[cfg(feature = "gated")]',
         "pub fn delta_arm(x: u64) -> u64 { x.rotate_left(9) }"])})
    v = run_decide(base_files(), head)
    check("test_inactive_cfg_item_drift_is_declared", v.outcome == A.OUTCOME_DECLARED,
          f"got {v.outcome}: {v.reason}")
    check("test_inactive_cfg_item_drift_is_declared__actually_drifted",
          v.evidence.get("differing_bytes", 0) > 0,
          "the fixture must actually move bytes, else the control is vacuous")


def test_comment_deletion_is_declared() -> None:
    """CONTROL: deleting a comment line is a deletion, but emits nothing — declare it."""
    base = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["// a comment that the head removes"])})
    head = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["// a comment that the head removes", "// and one it adds"])})
    # Head removes the first comment and adds two others, so both obligations run.
    head["crates/c/src/lib.rs"] = insert_at(_BASE_LIB, 4, ["// and one it adds", "// plus another"])
    v = run_decide(base, head)
    check("test_comment_deletion_is_declared", v.outcome == A.OUTCOME_DECLARED,
          f"got {v.outcome}: {v.reason}")
    check("test_comment_deletion_is_declared__deletion_obligation_ran",
          v.evidence.get("deleted_nonblank_lines", 0) > 0,
          "fixture must delete a non-blank line, else the deletion obligation is untested")


def test_identical_bundles_need_no_declaration() -> None:
    """A diff that does not move the bundle is `no-drift`, not a declaration.

    The fixture appends a comment BELOW the last compiled line, so no line position moves
    and the two bundles come out byte-identical — the common case leg 2 passes on its own.
    """
    head = base_files(**{"crates/c/src/lib.rs": _BASE_LIB + "// trailing note\n"})
    v = run_decide(base_files(), head)
    check("test_identical_bundles_need_no_declaration", v.outcome == A.OUTCOME_NO_DRIFT,
          f"got {v.outcome}: {v.reason}")


# ---------------------------------------------------------------------------
# RED arm — a drift that is NOT benign must still be REFUSED (the leg stays red).
# ---------------------------------------------------------------------------

def test_added_code_lines_are_refused() -> None:
    """RED TEST: real, ungated code added to the default build must NOT be auto-declared."""
    head = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["pub fn leaked(x: u64) -> u64 { x.rotate_right(5) }"])})
    v = run_decide(base_files(), head)
    check("test_added_code_lines_are_refused", v.outcome == A.REFUSE_ADDED_LINES_ARE_SEMANTIC,
          f"got {v.outcome}: {v.reason}")
    check("test_added_code_lines_are_refused__is_a_refusal", v.refused,
          "an unattributable drift must report refused=True")


def test_added_code_lines_are_refused_even_at_zero_size_delta() -> None:
    """RED TEST, hardest form: a semantic change whose bundle SIZE is unchanged.

    Swapping one always-compiled line for another of the same shape leaves the size delta at
    0 — the same signature a comment insertion has. Size alone therefore cannot be the
    discriminator, and this test pins that the neutral-tree REBUILD is what refuses it.
    """
    changed = _BASE_LIB.replace("x.wrapping_mul(3)", "x.wrapping_mul(5)")
    v = run_decide(base_files(), base_files(**{"crates/c/src/lib.rs": changed}))
    check("test_added_code_lines_are_refused_even_at_zero_size_delta",
          v.outcome == A.REFUSE_ADDED_LINES_ARE_SEMANTIC, f"got {v.outcome}: {v.reason}")
    check("test_added_code_lines_are_refused_even_at_zero_size_delta__delta_is_zero",
          v.evidence.get("size_delta_bytes") == 0,
          "fixture must hold size constant, else it does not test what it claims")


def test_deleted_code_lines_are_refused() -> None:
    """RED TEST: removing always-compiled code must NOT be auto-declared.

    Deleted lines are absent from both the head tree and the additions-neutral tree, so
    only the base-side deletion obligation can catch this one.
    """
    base = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["pub fn doomed(x: u64) -> u64 { x ^ 0xff }"])})
    v = run_decide(base, base_files())
    check("test_deleted_code_lines_are_refused",
          v.outcome == A.REFUSE_DELETED_LINES_ARE_SEMANTIC, f"got {v.outcome}: {v.reason}")


def test_manifest_change_that_enables_a_feature_is_refused() -> None:
    """RED TEST: turning a gated arm ON via the manifest is a real default-build change.

    The `.rs` diff can be empty here — only `default = [...]` moved — so the refusal has to
    come from the neutral tree taking its manifests from BASE.
    """
    base = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ['#[cfg(feature = "gated")]',
                       "pub fn arm(x: u64) -> u64 { x.rotate_left(9) }"])})
    head = dict(base)
    head["Cargo.toml"] = _ROOT_MANIFEST.replace('default = ["core"]', 'default = ["core", "gated"]')
    v = run_decide(base, head)
    check("test_manifest_change_that_enables_a_feature_is_refused",
          v.outcome == A.REFUSE_ADDED_LINES_ARE_SEMANTIC, f"got {v.outcome}: {v.reason}")


def _scripted_builder(*results):
    """A builder returning `results` in call order — lets a test drive ONE build to failure
    while the others succeed, which tree fixtures alone cannot reliably arrange."""
    seq = list(results)

    def builder(tree):
        return seq.pop(0) if seq else b"unexpected-extra-build"
    return builder


def test_neutral_build_failure_is_refused() -> None:
    """A neutral tree that no longer compiles means a blanked addition was load-bearing.

    Driven with a scripted builder: base and head BOTH build, and only the third (neutral)
    build fails, so this exercises the neutral-build branch specifically rather than
    tripping the earlier head-build guard.
    """
    base = make_tree(base_files())
    head = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["// added comment"])}))
    try:
        v = A.decide(base, head, diff_of(base, head),
                     _scripted_builder(b"base-bundle", b"head-bundle", None))
        check("test_neutral_build_failure_is_refused",
              v.outcome == A.REFUSE_NEUTRAL_BUILD_FAILED, f"got {v.outcome}: {v.reason}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_deletion_neutral_build_failure_is_refused() -> None:
    """Same, for the base-side deletion obligation's build."""
    base = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["pub fn doomed(x: u64) -> u64 { x }"])}))
    head = make_tree(base_files())
    try:
        # base, head, neutral(==head so obligation 1 passes), deletion-neutral -> None
        v = A.decide(base, head, diff_of(base, head),
                     _scripted_builder(b"base-bundle", b"head-bundle", b"head-bundle", None))
        check("test_deletion_neutral_build_failure_is_refused",
              v.outcome == A.REFUSE_DELETION_BUILD_FAILED, f"got {v.outcome}: {v.reason}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_neutral_tree_blanks_the_added_lines() -> None:
    """The neutral tree handed to the compiler must really have the added lines EMPTIED.

    Pins the blanking call site directly: without it the neutral tree is just the head tree,
    obligation 1 compares head against head, and every drift would be declared. Asserted by
    reading the tree the builder is actually given, not by trusting the verdict.
    """
    seen: list[str] = []
    captured: dict[str, str] = {}

    def builder(tree):
        seen.append(tree)
        p = os.path.join(tree, "crates/c/src/lib.rs")
        if len(seen) == 3 and os.path.exists(p):
            captured["neutral"] = open(p, encoding="utf-8").read()
        # base and head MUST differ, or `decide` short-circuits on `no-drift` and the
        # neutral tree is never built (which would make this test vacuous).
        return b"base-bundle" if len(seen) == 1 else b"head-bundle"

    base = make_tree(base_files())
    head = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["pub fn added_code(x: u64) -> u64 { x }"])}))
    try:
        A.decide(base, head, diff_of(base, head), builder)
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)
    neutral = captured.get("neutral", "")
    check("test_neutral_tree_blanks_the_added_lines",
          "added_code" not in neutral and neutral.split("\n")[4] == "",
          "the added line must be an EMPTY line in the neutral tree, got line 5 = "
          f"{neutral.split(chr(10))[4]!r}")
    check("test_neutral_tree_blanks_the_added_lines__line_count_preserved",
          len(neutral.split("\n")) == len(
              insert_at(_BASE_LIB, 4, ["pub fn added_code(x: u64) -> u64 { x }"]).split("\n")),
          "blanking must preserve the head tree's line COUNT (that is what holds line "
          "positions fixed, which is the whole basis of the proof)")


def test_non_rust_file_inside_the_closure_is_blanked_not_refused() -> None:
    """CONTROL: a crate README the build does not read must not block a derivation."""
    base = base_files(**{"crates/c/README.md": "# c\n"})
    head = base_files(**{"crates/c/README.md": "# c\n\nA new paragraph.\n",
                         "crates/c/src/lib.rs": insert_at(_BASE_LIB, 4, ["// a comment"])})
    v = run_decide(base, head)
    check("test_non_rust_file_inside_the_closure_is_blanked_not_refused",
          v.outcome == A.OUTCOME_DECLARED, f"got {v.outcome}: {v.reason}")


def test_included_non_rust_file_is_refused() -> None:
    """RED TEST: the same README, but `include_str!`d — its content DOES reach the bundle.

    This is why inertness cannot be decided by file extension. Blanking the added lines and
    rebuilding is what tells the two cases apart, and it must refuse this one.
    """
    lib = _BASE_LIB + '\npub const DOC: &str = include_str!("../README.md");\n'
    base = base_files(**{"crates/c/README.md": "# c\n", "crates/c/src/lib.rs": lib})
    head = base_files(**{"crates/c/README.md": "# c\nA new documented invariant.\n",
                         "crates/c/src/lib.rs": lib})
    v = run_decide(base, head)
    check("test_included_non_rust_file_is_refused",
          v.outcome == A.REFUSE_ADDED_LINES_ARE_SEMANTIC, f"got {v.outcome}: {v.reason}")


def test_diff_paths_absent_from_both_trees_are_refused() -> None:
    """A diff naming paths that exist in NEITHER tree means the two disagree.

    Every such path is silently skipped by the blanking loop, so the proof would cover less
    than it claims. Found for real: a harness rewrote `git diff --no-index` prefixes in the
    wrong order and produced `bvendor/spargebra/src/parser.rs`; nothing matched, nothing was
    blanked. Only the non-vacuity guard stopped a false declaration — this names the cause.
    """
    base = make_tree(base_files())
    head = make_tree(base_files())
    diff = ("diff --git abc/x.rs bbc/x.rs\n"
            "--- abc/x.rs\n"
            "+++ bbc/x.rs\n"
            "@@ -0,0 +1 @@\n"
            "+pub fn mangled() {}\n")
    try:
        v = A.decide(base, head, diff, _scripted_builder(b"base-bundle", b"head-bundle"))
        check("test_diff_paths_absent_from_both_trees_are_refused",
              v.outcome == A.REFUSE_DIFF_TREE_MISMATCH, f"got {v.outcome}: {v.reason}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_symlink_inside_the_closure_is_refused() -> None:
    """Blanking rewrites files in place, which cannot speak for a symlink — refuse."""
    base = make_tree(base_files())
    head = make_tree(base_files())
    link = os.path.join(head, "crates/c/src/aliased.rs")
    os.symlink("lib.rs", link)
    with open(os.path.join(head, "crates/c/src/lib.rs"), "a") as fh:
        fh.write("// touched\n")
    try:
        v = A.decide(base, head, diff_of(base, head), fake_build)
        check("test_symlink_inside_the_closure_is_refused",
              v.outcome == A.REFUSE_UNSUPPORTED_FILE_CHANGE, f"got {v.outcome}: {v.reason}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_out_of_closure_semantic_change_is_refused() -> None:
    """RED TEST: a real code change in a path the closure query MISSES must still refuse.

    This is the live false pass this tool shipped with. sparq's root manifest carries
    `exclude = ["vendor/spargebra", ...]` together with `[patch.crates-io] spargebra =
    { path = "vendor/spargebra" }`, so spargebra IS compiled into the feature-OFF bundle
    while NOT being a workspace member. Blanking was scoped to a closure derived from
    `cargo metadata --no-deps` — workspace members only — so a change under `vendor/`
    was never blanked and `neutral == head` held BY CONSTRUCTION.

    The fixture deliberately hands `decide` a closure that does NOT contain the changed
    crate, which is exactly the failure mode. The refusal must not depend on the closure
    being right — that is the whole point of blanking every changed non-manifest path.
    """
    real_closure = A.closure_dirs
    A.closure_dirs = lambda tree, package="sparq-wasm": ["crates/c/"]  # deliberately WRONG
    try:
        base = base_files(**{"vendor/vend/src/lib.rs": _BASE_LIB})
        head = base_files(**{"vendor/vend/src/lib.rs": insert_at(
            _BASE_LIB, 4, ["pub fn smuggled(x: u64) -> u64 { x.rotate_right(3) }"])})
        v = run_decide(base, head)
    finally:
        A.closure_dirs = real_closure
    check("test_out_of_closure_semantic_change_is_refused",
          v.outcome == A.REFUSE_ADDED_LINES_ARE_SEMANTIC, f"got {v.outcome}: {v.reason}")


def test_vacuous_proof_is_refused() -> None:
    """RED TEST: if the neutral tree equals the head tree, the proof is `head == head`.

    The general form of the bug above: whenever the bundle moved but NOTHING the proof
    covers was blanked, the drift came from somewhere the proof does not reach. This guard
    catches it without anyone having to know which path class was missed.
    """
    base = make_tree(base_files())
    # The diff adds only a BLANK line: the path exists, so it is not a diff/tree mismatch,
    # but blanking an already-empty line is a no-op — so the neutral tree comes out
    # identical to the head tree and the comparison would be `head == head`. The bundle
    # moved anyway (the builder says so), which means the drift came from somewhere the
    # proof does not reach.
    head = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(_BASE_LIB, 4, [""])}))
    try:
        v = A.decide(base, head, diff_of(base, head),
                     _scripted_builder(b"base-bundle", b"head-bundle"))
        check("test_vacuous_proof_is_refused", v.outcome == A.REFUSE_VACUOUS_PROOF,
              f"got {v.outcome}: {v.reason}")
        check("test_vacuous_proof_is_refused__names_the_paths",
              "lib.rs" in v.reason,
              "the refusal must name where the unexplained diff actually was")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_closure_query_includes_patched_out_of_workspace_crates() -> None:
    """`cargo metadata --no-deps` returns WORKSPACE MEMBERS ONLY — the wrong instrument.

    A crate reached through `[patch.crates-io]` from an `exclude`d path is compiled into
    the bundle but is not a member, so `--no-deps` silently drops it. Pins both halves:
    the flag is absent from the query, and such a package's directory comes back.
    """
    tree = tempfile.mkdtemp(prefix="featoff-closure-")
    os.makedirs(os.path.join(tree, "vendor/spargebra"))
    argvs: list[list[str]] = []

    class _R:
        returncode = 0
        stderr = ""

        def __init__(self, stdout):
            self.stdout = stdout

    def _stub(argv, **k):
        argvs.append(argv)
        if argv[1] == "tree":
            return _R("sparq-wasm v0.1.0\nspargebra v0.4.6\nserde v1.0.0\n")
        return _R(json.dumps({"packages": [
            {"name": "spargebra",
             "manifest_path": os.path.join(tree, "vendor/spargebra/Cargo.toml")},
            {"name": "serde", "manifest_path": "/home/u/.cargo/registry/serde/Cargo.toml"},
        ]}))

    real = A.subprocess.run
    try:
        A.subprocess.run = _stub
        got = A.closure_dirs(tree)
    finally:
        A.subprocess.run = real
        shutil.rmtree(tree, ignore_errors=True)
    meta_argv = next((a for a in argvs if a[1] == "metadata"), [])
    check("test_closure_query_includes_patched_out_of_workspace_crates",
          got == ["vendor/spargebra/"],
          f"expected the patched vendored crate's dir, got {got}")
    check("test_closure_query_includes_patched_out_of_workspace_crates__no_no_deps",
          "--no-deps" not in meta_argv,
          "cargo metadata must NOT use --no-deps (it hides patched non-member crates)")


def test_every_changed_non_manifest_path_is_blanked() -> None:
    """Nothing is skipped: a path the closure omits is still covered by the proof."""
    blankable, manifest, in_closure = A.classify_paths(
        ["crates/c/src/lib.rs", "vendor/vend/src/lib.rs", "research/notes.md", "Cargo.lock"],
        ["crates/c/"])
    check("test_every_changed_non_manifest_path_is_blanked",
          sorted(blankable) == ["crates/c/src/lib.rs", "research/notes.md",
                                "vendor/vend/src/lib.rs"]
          and manifest == ["Cargo.lock"],
          f"got blankable={blankable} manifest={manifest}")
    check("test_every_changed_non_manifest_path_is_blanked__closure_is_report_only",
          in_closure == ["crates/c/src/lib.rs", "Cargo.lock"],
          f"the closure list is evidence only, got {in_closure}")


def test_root_build_inputs_are_restored_from_base() -> None:
    """Cargo.lock / rust-toolchain / .cargo live outside every crate dir but reach the build.

    They must land in the MANIFEST bucket (restored wholesale from the base tree), never in
    the blankable one — blanking a line out of a lockfile yields a corrupt lockfile, not an
    absence — and they are always reported as part of the compiled closure.
    """
    blankable, manifest, in_closure = A.classify_paths(
        ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml"],
        ["crates/c/"])
    check("test_root_build_inputs_are_restored_from_base",
          blankable == [] and len(manifest) == 4 and len(in_closure) == 4,
          f"got blankable={blankable} manifest={manifest} in_closure={in_closure}")


def test_base_build_failure_is_refused() -> None:
    def builder(tree):
        return None if tree.endswith("base") or "base" in os.path.basename(tree) else b"x"
    base = make_tree(base_files())
    head = make_tree(base_files(**{"crates/c/src/lib.rs": _BASE_LIB + "// x\n"}))
    try:
        v = A.decide(base, head, diff_of(base, head), lambda t: None)
        check("test_base_build_failure_is_refused", v.outcome == A.REFUSE_BASE_BUILD_FAILED,
              f"got {v.outcome}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_head_build_failure_is_refused() -> None:
    base = make_tree(base_files())
    head = make_tree(base_files(**{"crates/c/src/lib.rs": _BASE_LIB + "// x\n"}))
    calls = {"n": 0}

    def builder(tree):
        calls["n"] += 1
        return b"base-bundle" if calls["n"] == 1 else None
    try:
        v = A.decide(base, head, diff_of(base, head), builder)
        check("test_head_build_failure_is_refused", v.outcome == A.REFUSE_HEAD_BUILD_FAILED,
              f"got {v.outcome}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


def test_empty_diff_is_refused() -> None:
    base = make_tree(base_files())
    try:
        v = A.decide(base, base, "", fake_build)
        check("test_empty_diff_is_refused", v.outcome == A.REFUSE_NO_DIFF, f"got {v.outcome}")
    finally:
        shutil.rmtree(base, ignore_errors=True)


# ---------------------------------------------------------------------------
# Unit-level obligations of the helpers the decision leans on.
# ---------------------------------------------------------------------------

def test_blank_lines_preserves_line_count() -> None:
    text = "a\nb\nc\nd\n"
    out = A.blank_lines(text, [2, 3])
    check("test_blank_lines_preserves_line_count",
          len(out.split("\n")) == len(text.split("\n")) and out.split("\n")[1] == "",
          f"got {out!r}")


def test_deleted_file_lines_are_attributed() -> None:
    """A diff that DELETES a file must have its removed lines attributed, not dropped.

    `+++ /dev/null` is the shape that silently loses a whole-file deletion if the parser
    only keys hunks off the `+++` header.
    """
    diff = ("diff --git a/crates/c/src/gone.rs b/crates/c/src/gone.rs\n"
            "--- a/crates/c/src/gone.rs\n"
            "+++ /dev/null\n"
            "@@ -1,2 +0,0 @@\n"
            "-pub fn gone() {}\n"
            "-pub fn also() {}\n")
    parsed = A.parse_unified_diff(diff)
    check("test_deleted_file_lines_are_attributed",
          parsed.get("crates/c/src/gone.rs", {}).get("removed") == [1, 2],
          f"got {parsed}")


def test_declaration_records_measured_evidence() -> None:
    """The written declaration must carry the MEASURED numbers, not a fixed sentence."""
    head = base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["// a comment"])})
    v = run_decide(base_files(), head)
    doc = A.declaration_json(4242, v, date="2026-07-28")
    ok = (doc["pr"] == 4242 and doc["derived"] is True
          and str(v.evidence["differing_bytes"]) in doc["reason"]
          and doc["evidence"]["size_delta_bytes"] == v.evidence["size_delta_bytes"])
    check("test_declaration_records_measured_evidence", ok, f"got {doc}")


def test_census_row_is_emitted_for_every_outcome() -> None:
    rows = []
    for v in (A.Verdict(A.OUTCOME_DECLARED, "d", {"size_delta_bytes": 0, "differing_bytes": 3}),
              A.Verdict(A.REFUSE_ADDED_LINES_ARE_SEMANTIC, "r", {}),
              A.Verdict(A.OUTCOME_NO_DRIFT, "n", {})):
        rows.append(A.census_row(1, v))
    check("test_census_row_is_emitted_for_every_outcome",
          all(r.startswith("FEATURE-OFF-CENSUS ") and "outcome=" in r for r in rows),
          f"got {rows}")


# ---------------------------------------------------------------------------
# The fake compiler must itself reproduce the MEASURED real-world signature, otherwise
# every test above is testing a model that does not resemble rustc.
# ---------------------------------------------------------------------------

def test_fake_compiler_reproduces_the_measured_signature() -> None:
    base = make_tree(base_files())
    shifted = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["// c" + str(i) for i in range(12)])}))
    grown = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["pub fn extra(x: u64) -> u64 { x }"])}))
    try:
        b, s, g = fake_build(base), fake_build(shifted), fake_build(grown)
        check("test_fake_compiler_reproduces_the_measured_signature__comment_is_zero_delta",
              len(s) == len(b) and s != b,
              "a comment insertion must move bytes at delta 0, as MEASURED on sparq-wasm")
        check("test_fake_compiler_reproduces_the_measured_signature__code_grows",
              len(g) > len(b),
              "added compiled code must GROW the artefact, as MEASURED on sparq-wasm")
    finally:
        for d in (base, shifted, grown):
            shutil.rmtree(d, ignore_errors=True)


# ---------------------------------------------------------------------------
# Census sweep. Membership must be re-derived from the LIVE leg conclusion on each PR's
# current head, matched by NAME EQUALITY and taking the NEWEST run — a cached or
# fuzzily-matched classification is how this class silently miscounts.
# ---------------------------------------------------------------------------

def _sweep(checks_by_pr: dict[int, list[dict]], prs: list[dict] | None = None,
           declared: set[str] | None = None):
    prs = prs or [{"number": n, "headRefOid": f"sha{n}", "isDraft": True, "title": "t"}
                  for n in checks_by_pr]
    return A.census_sweep(
        "o/r", declared or set(),
        pr_lister=lambda repo: prs,
        check_lister=lambda repo, sha: checks_by_pr[int(sha[3:])],
    )


def _run(name: str, conclusion: str, started: str = "2026-07-28T00:00:00Z",
         status: str = "completed") -> dict:
    return {"name": name, "conclusion": conclusion, "status": status, "started_at": started}


def test_census_sweep_counts_the_live_class() -> None:
    rep = _sweep({
        1: [_run(A.LEG_NAME, "failure")],
        2: [_run(A.LEG_NAME, "failure")],
        3: [_run(A.LEG_NAME, "success")],
        4: [_run("some other check", "failure")],
    }, declared={"2.json"})
    ok = (rep["open_prs"] == 4 and rep["in_class_red"] == 2
          and rep["in_class_already_declared"] == 1 and rep["in_class_undeclared"] == 1
          and rep["leg_absent"] == 1)
    check("test_census_sweep_counts_the_live_class", ok, f"got {rep}")


def test_census_sweep_matches_leg_name_by_equality() -> None:
    """A differently-named check that merely CONTAINS the leg name must not be counted."""
    rep = _sweep({1: [_run(A.LEG_NAME + " (advisory)", "failure")]})
    check("test_census_sweep_matches_leg_name_by_equality",
          rep["in_class_red"] == 0 and rep["leg_absent"] == 1, f"got {rep}")


def test_census_sweep_uses_the_newest_run_per_name() -> None:
    """An old red superseded by a newer green must read GREEN, and vice versa.

    Both LIST ORDERS are exercised deliberately. The API does not promise chronological
    order, so a scan that simply keeps the first (or the last) element it sees agrees with
    the correct answer on one ordering and disagrees on the other; testing only one ordering
    would let that bug through.
    """
    old_red_new_green = [_run(A.LEG_NAME, "failure", "2026-07-01T00:00:00Z"),
                         _run(A.LEG_NAME, "success", "2026-07-28T00:00:00Z")]
    old_green_new_red = [_run(A.LEG_NAME, "success", "2026-07-01T00:00:00Z"),
                         _run(A.LEG_NAME, "failure", "2026-07-28T00:00:00Z")]
    cases = [
        ("old-red/new-green, chronological", old_red_new_green, 0),
        ("old-red/new-green, reversed", list(reversed(old_red_new_green)), 0),
        ("old-green/new-red, chronological", old_green_new_red, 1),
        ("old-green/new-red, reversed", list(reversed(old_green_new_red)), 1),
    ]
    for label, runs, want in cases:
        got = _sweep({1: runs})["in_class_red"]
        check("test_census_sweep_uses_the_newest_run_per_name",
              got == want, f"{label}: expected in_class_red={want}, got {got}")


def test_prebuilt_bundles_do_not_skip_the_neutral_proof() -> None:
    """Handing in already-built base/head bundles must NOT short-circuit the obligations.

    The leg-2 job passes the two bundles it already built; if that path ever stopped
    rebuilding the neutral tree, every drift would be auto-declared.
    """
    base = make_tree(base_files())
    head = make_tree(base_files(**{"crates/c/src/lib.rs": insert_at(
        _BASE_LIB, 4, ["pub fn leaked(x: u64) -> u64 { x }"])}))
    try:
        v = A.decide(base, head, diff_of(base, head), fake_build,
                     base_bytes=b"prebuilt-base", head_bytes=b"prebuilt-head")
        check("test_prebuilt_bundles_do_not_skip_the_neutral_proof",
              v.outcome == A.REFUSE_ADDED_LINES_ARE_SEMANTIC, f"got {v.outcome}: {v.reason}")
    finally:
        shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(head, ignore_errors=True)


# ---------------------------------------------------------------------------
# WORKFLOW WIRING. A YAML `if:` cannot be executed here, so its SHAPE is pinned — and the
# mutation matrix below deletes the `if:`, the STEP and the CALL SITE (`id: leg2`)
# separately, because a derivation that is wired to a step id nobody sets never runs and
# fails silently.
# ---------------------------------------------------------------------------

_REPO_ROOT = os.path.dirname(_SCRIPTS)
_WORKFLOW_PATH = os.path.join(_REPO_ROOT, ".github", "workflows", "vectorized-feature-off.yml")


def workflow_text() -> str:
    with open(_WORKFLOW_PATH, encoding="utf-8") as fh:
        return fh.read()


def _step_block(text: str, name_fragment: str) -> str:
    """The YAML block of the step whose `- name:` contains `name_fragment` (or "" if absent).

    COMMENT lines are stripped: a `#` line sitting between two steps belongs to the earlier
    block textually, so leaving them in would let a comment that merely MENTIONS a key
    satisfy an assertion about that key being set.
    """
    for s in re.split(r"\n      - ", text):
        if s.startswith("name:") and name_fragment in s.split("\n")[0]:
            return "\n".join(l for l in s.split("\n") if not l.lstrip().startswith("#"))
    return ""


def test_workflow_wires_the_derivation_step() -> None:
    block = _step_block(workflow_text(), "Derive the feature-OFF declaration")
    check("test_workflow_wires_the_derivation_step",
          bool(block) and "scripts/feature_off_autodeclare.py" in block
          and "--report-only" in block,
          "the artifact-exact-equality job must invoke the derivation in report-only mode")


def test_derivation_if_carries_always() -> None:
    """`always()` is load-bearing on the derivation's `if:`.

    Without it the step inherits the implicit `success()`, which is false once leg 2 has
    failed — so the derivation would be skipped EXACTLY when it is needed and would never
    run in production. It fails safe, which is precisely why nothing else would go red.
    """
    block = _step_block(workflow_text(), "Derive the feature-OFF declaration")
    check("test_derivation_if_carries_always", re.search(r"if:\s*always\(\)", block) is not None,
          "the derivation's `if:` must start with always(), or it never runs on a failed leg 2")


def test_mutation_tripwire_runs_when_leg2_fails() -> None:
    """The `--mutate` matrix must run on the population the derivation serves.

    Same trap as above: without `always()` the tripwire only ever runs on PRs where leg 2
    is already GREEN — i.e. never on a failing PR, which is the only population where the
    derivation does anything. It would be guarding an empty set.
    """
    block = _step_block(workflow_text(), "Tripwire self-test (the declaration derivation")
    check("test_mutation_tripwire_runs_when_leg2_fails",
          re.search(r"if:\s*always\(\)", block) is not None,
          "the derivation tripwire's `if:` must carry always()")


def test_ci_invocation_is_report_only() -> None:
    """CI must never pass `--write`.

    `--write` puts the declaration into the working tree. In a job that has the repo
    checked out this is the first step towards a gate that declares for you; the whole
    containment argument is that CI only ever REPORTS and a human commits.
    """
    block = _step_block(workflow_text(), "Derive the feature-OFF declaration")
    check("test_ci_invocation_is_report_only",
          "--report-only" in block and "--write" not in block,
          "the CI invocation must be --report-only and must not pass --write")


def test_ci_passes_the_bundles_in_the_right_order() -> None:
    """`--base-wasm` must get the BASE bundle and `--head-wasm` the HEAD one.

    Swapping them silently inverts the comparison: the reported size delta flips sign and
    the prebuilt bundles no longer correspond to the trees the obligations rebuild.
    """
    block = _step_block(workflow_text(), "Derive the feature-OFF declaration")
    ok = ("--base-wasm base-wasm/base.wasm" in block
          and "--head-wasm head.wasm" in block)
    check("test_ci_passes_the_bundles_in_the_right_order", ok,
          "expected --base-wasm base-wasm/base.wasm and --head-wasm head.wasm")


def test_derivation_step_runs_only_when_leg2_failed() -> None:
    block = _step_block(workflow_text(), "Derive the feature-OFF declaration")
    check("test_derivation_step_runs_only_when_leg2_failed",
          "steps.leg2.outcome == 'failure'" in block,
          "the derivation must be gated on leg 2 having FAILED, else it burns a wasm build "
          "on every green PR")


def test_leg2_step_has_the_id_the_derivation_depends_on() -> None:
    """CALL SITE: `steps.leg2.outcome` is empty unless leg 2 carries `id: leg2`.

    Without the id the guard above is never true and the derivation silently never runs —
    a dead step that still looks wired.
    """
    block = _step_block(workflow_text(), "Leg 2 — dynamic byte-identity")
    check("test_leg2_step_has_the_id_the_derivation_depends_on",
          re.search(r"^\s*id:\s*leg2\s*$", block, re.M) is not None,
          "leg 2 must carry `id: leg2` for the derivation's guard to ever evaluate true")


def test_derivation_cannot_change_the_leg2_verdict() -> None:
    """The derivation is advisory: it may not soften leg 2, and leg 2 may not be tolerant."""
    text = workflow_text()
    leg2 = _step_block(text, "Leg 2 — dynamic byte-identity")
    deriv = _step_block(text, "Derive the feature-OFF declaration")
    check("test_derivation_cannot_change_the_leg2_verdict",
          "continue-on-error" not in leg2 and "continue-on-error: true" in deriv,
          "leg 2 must stay strict and the derivation must not add a second red")
    check("test_derivation_cannot_change_the_leg2_verdict__runs_after_leg2",
          text.index("Derive the feature-OFF declaration") > text.index("Leg 2 — dynamic"),
          "the derivation must run after leg 2, not before it")


def test_derivation_suite_is_run_in_ci() -> None:
    """The derivation's own mutation matrix must run in CI, or it can rot into a stamp."""
    block = _step_block(workflow_text(), "Tripwire self-test (the declaration derivation")
    check("test_derivation_suite_is_run_in_ci",
          "scripts/tests/test_feature_off_autodeclare.py --mutate" in block,
          "CI must run the derivation suite WITH --mutate")


def test_workflow_sets_no_shared_target_dir() -> None:
    """The workflow must NOT hand the derivation a shared CARGO_TARGET_DIR.

    Sharing one warm target directory across the materialised trees was tried for speed and
    REVERTED: on #4350 it reported a 299-line `exec.rs` rewrite as byte-identical, because
    `git archive` stamps commit mtimes and cargo's freshness check is mtime-based. A false
    "identical" auto-declares a real code change, so the environment must not be able to
    reintroduce it.
    """
    block = _step_block(workflow_text(), "Derive the feature-OFF declaration")
    check("test_workflow_sets_no_shared_target_dir", "CARGO_TARGET_DIR" not in block,
          "the derivation step must not set CARGO_TARGET_DIR")


def test_builder_ignores_an_inherited_shared_target_dir() -> None:
    """An inherited CARGO_TARGET_DIR must NOT redirect the build or the read-back.

    Sharing one target directory across materialised trees returned a STALE bundle on #4350
    — a 299-line `exec.rs` rewrite read back as byte-identical to its base. Since a false
    "identical" is the one outcome that would auto-declare a real code change, the builder
    pins the target directory inside the tree and overrides the inherited variable.
    Exercised BEHAVIOURALLY with a stubbed cargo: the bundle must come from the TREE, and
    the env the subprocess receives must point there too.
    """
    tree = tempfile.mkdtemp(prefix="featoff-tgt-")
    decoy = tempfile.mkdtemp(prefix="featoff-decoy-target-")
    for root, payload in ((os.path.join(tree, "target"), b"bundle-from-the-tree"),
                          (decoy, b"STALE-bundle-from-the-shared-dir")):
        out = os.path.join(root, "wasm32-unknown-unknown", "release-wasm")
        os.makedirs(out)
        with open(os.path.join(out, "sparq_wasm.wasm"), "wb") as fh:
            fh.write(payload)

    seen: dict[str, str] = {}

    class _Ok:
        returncode = 0
        stderr = b""

    def _stub(*a, **k):
        seen.update(k.get("env") or {})
        seen["argv"] = " ".join(a[0]) if a else ""
        return _Ok()

    real_run, real_env = A.subprocess.run, os.environ.get("CARGO_TARGET_DIR")
    try:
        A.subprocess.run = _stub
        os.environ["CARGO_TARGET_DIR"] = decoy
        got = A.cargo_wasm_builder(tree)
    finally:
        A.subprocess.run = real_run
        if real_env is None:
            os.environ.pop("CARGO_TARGET_DIR", None)
        else:
            os.environ["CARGO_TARGET_DIR"] = real_env
        shutil.rmtree(tree, ignore_errors=True)
        shutil.rmtree(decoy, ignore_errors=True)
    check("test_builder_ignores_an_inherited_shared_target_dir",
          got == b"bundle-from-the-tree",
          f"expected the tree's own bundle, got {got!r}")
    check("test_builder_ignores_an_inherited_shared_target_dir__subprocess_env",
          seen.get("CARGO_TARGET_DIR", "").startswith(tree),
          f"cargo must be run with the tree's target dir, got "
          f"{seen.get('CARGO_TARGET_DIR')!r}")


def test_paths_filter_includes_the_new_scripts() -> None:
    """Editing the derivation must re-run the leg, in BOTH of the workflow's filter blocks."""
    text = workflow_text()
    check("test_paths_filter_includes_the_new_scripts",
          text.count("- 'scripts/feature_off_autodeclare.py'") == 2
          and text.count("- 'scripts/tests/test_feature_off_autodeclare.py'") == 2,
          "both paths-filter blocks must list the derivation script and its suite")


TESTS = [
    test_workflow_wires_the_derivation_step,
    test_derivation_step_runs_only_when_leg2_failed,
    test_derivation_if_carries_always,
    test_mutation_tripwire_runs_when_leg2_fails,
    test_ci_invocation_is_report_only,
    test_ci_passes_the_bundles_in_the_right_order,
    test_leg2_step_has_the_id_the_derivation_depends_on,
    test_derivation_cannot_change_the_leg2_verdict,
    test_derivation_suite_is_run_in_ci,
    test_workflow_sets_no_shared_target_dir,
    test_builder_ignores_an_inherited_shared_target_dir,
    test_paths_filter_includes_the_new_scripts,
    test_census_sweep_counts_the_live_class,
    test_census_sweep_matches_leg_name_by_equality,
    test_census_sweep_uses_the_newest_run_per_name,
    test_prebuilt_bundles_do_not_skip_the_neutral_proof,
    test_comment_only_drift_is_declared,
    test_inactive_cfg_item_drift_is_declared,
    test_comment_deletion_is_declared,
    test_identical_bundles_need_no_declaration,
    test_added_code_lines_are_refused,
    test_added_code_lines_are_refused_even_at_zero_size_delta,
    test_deleted_code_lines_are_refused,
    test_manifest_change_that_enables_a_feature_is_refused,
    test_neutral_build_failure_is_refused,
    test_deletion_neutral_build_failure_is_refused,
    test_neutral_tree_blanks_the_added_lines,
    test_non_rust_file_inside_the_closure_is_blanked_not_refused,
    test_out_of_closure_semantic_change_is_refused,
    test_vacuous_proof_is_refused,
    test_closure_query_includes_patched_out_of_workspace_crates,
    test_every_changed_non_manifest_path_is_blanked,
    test_included_non_rust_file_is_refused,
    test_symlink_inside_the_closure_is_refused,
    test_diff_paths_absent_from_both_trees_are_refused,
    test_root_build_inputs_are_restored_from_base,
    test_base_build_failure_is_refused,
    test_head_build_failure_is_refused,
    test_empty_diff_is_refused,
    test_blank_lines_preserves_line_count,
    test_deleted_file_lines_are_attributed,
    test_declaration_records_measured_evidence,
    test_census_row_is_emitted_for_every_outcome,
    test_fake_compiler_reproduces_the_measured_signature,
]


def run_suite() -> list[str]:
    global _FAILURES, _RUN
    _FAILURES, _RUN = [], []
    for t in TESTS:
        try:
            t()
        except Exception as exc:  # a raising test is a failing test
            _FAILURES.append(f"{t.__name__}: raised {type(exc).__name__}: {exc}")
    return list(_FAILURES)


# ---------------------------------------------------------------------------
# Mutation harness: one call site at a time, each naming the test that must catch it.
# ---------------------------------------------------------------------------

# (mutant id, description, source substring -> replacement, the NAMED test that must go red)
MUTANTS = [
    ("M1-drop-added-lines-proof",
     "never compare the additions-neutral bundle to the head bundle",
     "    if neutral_bytes != head_bytes:", "    if False:",
     "test_added_code_lines_are_refused"),
    ("M2-drop-deleted-lines-proof",
     "never compare the deletions-neutral bundle to the base bundle",
     "        if del_bytes != base_bytes:", "        if False:",
     "test_deleted_code_lines_are_refused"),
    ("M3-skip-blanking-added-lines",
     "build the neutral tree without blanking anything",
     "            fh.write(blanked)", "            fh.write(text)",
     "test_neutral_tree_blanks_the_added_lines"),
    ("M4-keep-head-manifests",
     "do not restore the base tree's manifests into the neutral tree",
     "            shutil.copyfile(src, dst)", "            pass",
     "test_manifest_change_that_enables_a_feature_is_refused"),
    ("M5-accept-symlinks",
     "blank symlinks and submodule gitlinks as if they were regular files",
     "    if nonregular:", "    if False:",
     "test_symlink_inside_the_closure_is_refused"),
    ("M5d-only-blank-rust-files",
     "assume every non-.rs file in the closure is inert, leaving an include_str!'d asset unproven",
     "        else:\n            blankable.append(p)",
     "        elif _is_rust(p):\n            blankable.append(p)\n        else:\n            inert.append(p)",
     "test_included_non_rust_file_is_refused"),
    ("M5e-scope-blanking-to-the-closure",
     "only blank paths inside the derived closure, reinstating the vendored false pass",
     "        if _is_manifest(p) or root_input:\n            manifest.append(p)\n        else:\n            blankable.append(p)",
     "        if _is_manifest(p) or root_input:\n            manifest.append(p)\n"
     "        elif closure is None or any(p.startswith(d) for d in closure):\n            blankable.append(p)",
     "test_out_of_closure_semantic_change_is_refused"),
    ("M5h-ignore-diff-tree-mismatch",
     "skip paths the diff names but neither tree has, covering less than claimed",
     "    if missing:", "    if False:",
     "test_diff_paths_absent_from_both_trees_are_refused"),
    ("M5f-drop-the-non-vacuity-guard",
     "declare even when the neutral tree is identical to the head tree",
     "    if not mutated and not to_blank:", "    if False:",
     "test_vacuous_proof_is_refused"),
    ("M5g-closure-query-uses-no-deps",
     "ask cargo metadata --no-deps, hiding patched non-member crates",
     '            ["cargo", "metadata", "--format-version", "1"],',
     '            ["cargo", "metadata", "--format-version", "1", "--no-deps"],',
     "test_closure_query_includes_patched_out_of_workspace_crates"),
    ("M5c-root-inputs-fall-outside",
     "forget that Cargo.lock / rust-toolchain / .cargo reach every build",
     '        root_input = p in _ROOT_BUILD_INPUTS or p.startswith(".cargo/")',
     "        root_input = False",
     "test_root_build_inputs_are_restored_from_base"),
    ("M6-ignore-neutral-build-failure",
     "treat a neutral tree that fails to build as a pass",
     "    if neutral_bytes is None:", "    if False:",
     "test_neutral_build_failure_is_refused"),
    ("M7-drop-deleted-file-attribution",
     "drop hunks whose new-side is /dev/null (whole-file deletions)",
     "                path = old_path", "                path = None",
     "test_deleted_file_lines_are_attributed"),
    ("M8-blank-lines-drops-lines",
     "make blanking DELETE lines instead of emptying them (breaks position preservation)",
     '            lines[n - 1] = ""', "            lines[n - 1] = None  # type: ignore",
     "test_blank_lines_preserves_line_count"),
    ("M10-ignore-deletion-build-failure",
     "treat a deletion-neutral tree that fails to build as a pass",
     "        if del_bytes is None:", "        if False:",
     "test_deletion_neutral_build_failure_is_refused"),
    ("M11-census-substring-match",
     "match the leg by substring instead of by name equality",
     '            if c["name"] != LEG_NAME:', '            if LEG_NAME not in c["name"]:',
     "test_census_sweep_matches_leg_name_by_equality"),
    ("M12-census-first-run-wins",
     "keep the FIRST check run for a name instead of the newest",
     '            if leg is None or (c.get("started_at") or "") >= (leg.get("started_at") or ""):',
     "            if leg is None:",
     "test_census_sweep_uses_the_newest_run_per_name"),
    ("M12b-census-last-run-wins",
     "keep the LAST check run for a name regardless of its timestamp",
     '            if leg is None or (c.get("started_at") or "") >= (leg.get("started_at") or ""):',
     "            if True:",
     "test_census_sweep_uses_the_newest_run_per_name"),
    ("M14-builder-obeys-shared-target-dir",
     "let an inherited CARGO_TARGET_DIR redirect the build, reintroducing the stale-bundle bug",
     '        env={**os.environ, "CARGO_TARGET_DIR": os.path.join(tree, "target")},',
     "        env=dict(os.environ),",
     "test_builder_ignores_an_inherited_shared_target_dir"),
    ("M13-prebuilt-skips-neutral-proof",
     "reuse the head bundle as the neutral bundle instead of rebuilding",
     "    neutral_bytes = builder(neutral)", "    neutral_bytes = head_bytes",
     "test_prebuilt_bundles_do_not_skip_the_neutral_proof"),
    ("M9-ignore-base-build-failure",
     "treat a base tree that fails to build as an empty bundle",
     "    if base_bytes is None:\n        return Verdict(REFUSE_BASE_BUILD_FAILED",
     "    if base_bytes is None:\n        base_bytes = b''\n    if False:\n        return Verdict(REFUSE_BASE_BUILD_FAILED",
     "test_base_build_failure_is_refused"),
]


# YAML mutants. "Ask the deletion question of the YAML": delete the `if:`, delete the STEP,
# and delete the CALL SITE the guard reads — each must red its own named check.
_DERIV_IF = ("        if: always() && steps.changes.outputs.rust_changed == 'true'\n"
             "          && steps.leg2.outcome == 'failure' && github.event_name == 'pull_request'\n")

YAML_MUTANTS = [
    ("Y1-drop-leg2-id",
     "remove `id: leg2`, so the derivation's guard can never be true",
     "        id: leg2\n", "",
     "test_leg2_step_has_the_id_the_derivation_depends_on"),
    ("Y2-drop-derivation-if",
     "remove the leg2-failed guard, so the derivation burns a build on every PR",
     _DERIV_IF, "        if: always()\n",
     "test_derivation_step_runs_only_when_leg2_failed"),
    ("Y3-drop-derivation-call",
     "stop invoking the derivation script",
     "          python3 scripts/feature_off_autodeclare.py \\\n", "          true \\\n",
     "test_workflow_wires_the_derivation_step"),
    ("Y4-drop-mutation-tripwire",
     "run the derivation suite without its mutation matrix",
     "scripts/tests/test_feature_off_autodeclare.py --mutate",
     "scripts/tests/test_feature_off_autodeclare.py",
     "test_derivation_suite_is_run_in_ci"),
    ("Y7-reintroduce-shared-target-dir",
     "hand the derivation a shared CARGO_TARGET_DIR again",
     "          PR_NUMBER: ${{ github.event.pull_request.number }}",
     "          PR_NUMBER: ${{ github.event.pull_request.number }}\n"
     "          CARGO_TARGET_DIR: ${{ runner.temp }}/shared",
     "test_workflow_sets_no_shared_target_dir"),
    ("Y8-drop-always-from-derivation-if",
     "drop always() so the derivation is skipped exactly when leg 2 fails",
     "        if: always() && steps.changes.outputs.rust_changed == 'true'\n"
     "          && steps.leg2.outcome == 'failure' && github.event_name == 'pull_request'",
     "        if: steps.changes.outputs.rust_changed == 'true'\n"
     "          && steps.leg2.outcome == 'failure' && github.event_name == 'pull_request'",
     "test_derivation_if_carries_always"),
    ("Y9-disable-the-mutation-tripwire",
     "switch the --mutate tripwire off with if: false",
     "        if: always() && steps.changes.outputs.rust_changed == 'true'\n"
     "        # Runs the derivation's own suite INCLUDING the mutation matrix",
     "        if: false\n"
     "        # Runs the derivation's own suite INCLUDING the mutation matrix",
     "test_mutation_tripwire_runs_when_leg2_fails"),
    ("Y10-ci-invocation-gains-write",
     "append --write, so CI starts materialising declarations itself",
     "--base-wasm base-wasm/base.wasm --head-wasm head.wasm --report-only",
     "--base-wasm base-wasm/base.wasm --head-wasm head.wasm --report-only --write",
     "test_ci_invocation_is_report_only"),
    ("Y11-swap-the-bundle-arguments",
     "swap --base-wasm and --head-wasm, inverting the comparison",
     "--base-wasm base-wasm/base.wasm --head-wasm head.wasm",
     "--base-wasm head.wasm --head-wasm base-wasm/base.wasm",
     "test_ci_passes_the_bundles_in_the_right_order"),
    ("Y5-drop-paths-filter-entry",
     "stop re-running the leg when the derivation script itself changes",
     "              - 'scripts/feature_off_autodeclare.py'\n", "",
     "test_paths_filter_includes_the_new_scripts"),
    ("Y6-make-leg2-tolerant",
     "let leg 2 itself continue-on-error, which would silently un-gate the whole leg",
     "      - name: Leg 2 — dynamic byte-identity (base tree vs head tree)\n        id: leg2\n",
     "      - name: Leg 2 — dynamic byte-identity (base tree vs head tree)\n        id: leg2\n"
     "        continue-on-error: true\n",
     "test_derivation_cannot_change_the_leg2_verdict"),
]


def _report(mid: str, named: str, fails: list[str], desc: str) -> bool:
    failed_names = {f.split(":")[0].split("__")[0] for f in fails}
    if named in failed_names:
        others = sorted(failed_names - {named})
        extra = f" (+{len(others)} other)" if others else ""
        print(f"{mid:33} {named:58} KILLED{extra}")
        return True
    print(f"{mid:33} {named:58} SURVIVED  <-- {desc}")
    return False


def run_mutation_matrix() -> int:
    global A, _WORKFLOW_PATH
    src = open(_MODULE_PATH, encoding="utf-8").read()
    wf_src = workflow_text()
    wf_original = _WORKFLOW_PATH
    print("\n=== mutation table (one call site per row) ===")
    print(f"{'mutant':33} {'named check that must go red':58} result")
    bad = 0
    original = A
    for mid, desc, old, new, named in MUTANTS:
        if old not in src:
            print(f"{mid:33} {named:58} ANCHOR-MISSING")
            bad += 1
            continue
        tmp = tempfile.mkdtemp(prefix="featoff-mut-")
        mpath = os.path.join(tmp, "feature_off_autodeclare.py")
        with open(mpath, "w", encoding="utf-8") as fh:
            fh.write(src.replace(old, new, 1))
        try:
            A = _load(mpath)
            fails = run_suite()
        except Exception as exc:
            fails = [f"module-load: {exc}"]
        finally:
            A = original
            shutil.rmtree(tmp, ignore_errors=True)
        bad += 0 if _report(mid, named, fails, desc) else 1

    for mid, desc, old, new, named in YAML_MUTANTS:
        if old not in wf_src:
            print(f"{mid:33} {named:58} ANCHOR-MISSING")
            bad += 1
            continue
        tmp = tempfile.mkdtemp(prefix="featoff-wfmut-")
        wpath = os.path.join(tmp, "vectorized-feature-off.yml")
        with open(wpath, "w", encoding="utf-8") as fh:
            fh.write(wf_src.replace(old, new, 1))
        try:
            _WORKFLOW_PATH = wpath
            fails = run_suite()
        finally:
            _WORKFLOW_PATH = wf_original
            shutil.rmtree(tmp, ignore_errors=True)
        bad += 0 if _report(mid, named, fails, desc) else 1

    # Restore the pristine module for any later use.
    A = _load()
    return bad


def main() -> int:
    fails = run_suite()
    print(f"ran {len(set(n.split('__')[0] for n in _RUN))} named checks "
          f"({len(_RUN)} assertions) over {len(TESTS)} tests")
    for f in fails:
        print("FAIL " + f)
    rc = 1 if fails else 0
    if "--mutate" in sys.argv:
        if rc:
            print("refusing to run the mutation matrix on a red suite")
            return rc
        bad = run_mutation_matrix()
        if bad:
            print(f"\n{bad} mutant(s) SURVIVED — the suite does not pin those call sites")
            rc = 1
        else:
            print(f"\nall {len(MUTANTS) + len(YAML_MUTANTS)} mutants killed by their named check")
    if rc == 0:
        print("OK")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
