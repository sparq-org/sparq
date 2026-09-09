#!/usr/bin/env python3
"""
[OPUS-5] sq-v3nel-v3: DERIVE a feature-OFF wasm-bundle declaration instead of asserting one.

The `artifact-exact-equality (wasm bundle feature-OFF)` leg
(`.github/workflows/vectorized-feature-off.yml`, leg 2) compares the feature-OFF
`sparq-wasm` bundle built from the BASE tree against the one built from the HEAD tree,
BYTE-FOR-BYTE, and fails an undeclared difference. That is the right guard — it is what
catches `vectorized`/default-path code leaking into the default build.

It also fires on a class of change that carries NO semantic content at all. MEASURED on
this repo (see `bench/feature-off-declarations/README.md` § "Why the gate fires on
comment-only edits"): inserting 34 pure comment lines into `crates/sparq-core/src/compress.rs`
and rebuilding produces a bundle that DIFFERS from base in exactly 3 bytes at exactly the
same length, because `core::panic::Location` records embed the LINE NUMBER of every
panicking call site and those numbers move when lines above them move. Adding an
off-by-default `#[cfg(feature = ...)]` item mid-file does the same thing: the item itself
compiles out, but every line below it shifts. A worker therefore has to hand-write a
declaration for a diff that changed no compiled code whatsoever.

This tool decides that case MECHANICALLY, and — this is the whole point — it decides it by
RE-DERIVING the drift from the compiler, never by asserting a tolerance:

    1. Build the BASE tree's feature-OFF bundle and the HEAD tree's. If they are identical
       there is nothing to declare (the leg already passes).

    2. Construct a NEUTRAL tree: the HEAD tree with every line the diff ADDED replaced by an
       EMPTY line, and with every changed manifest (`Cargo.toml` / `Cargo.lock`) taken from
       the BASE tree. By construction the neutral tree holds the BASE tree's compiled
       content at the HEAD tree's LINE POSITIONS.

    3. Build it. If `neutral == head` byte-for-byte, then blanking every added line changed
       nothing the compiler emits — i.e. the added lines contributed no compiled code, and
       the whole base->head drift is line-position metadata. That is a POSITIVE PROOF from
       the compiler, not an inference from the diff's shape.

    4. Deleted lines are absent from both the head and the neutral tree, so step 3 cannot
       speak for them. When the diff removes any non-blank line from a `.rs` file, build a
       second neutral tree — the BASE tree with exactly those lines blanked — and require it
       to equal the BASE bundle. Same proof, run backwards.

Only when every obligation holds is a declaration written, and its `reason` records the
MEASURED evidence (byte counts, differing-byte count, which obligations were discharged).
Anything else REFUSES with a named reason and leaves the leg RED — an unexplained diff must
never be auto-declared, because that would convert the guard into a rubber stamp.

Deliberately NOT done here: this tool does not make the gate pass by itself and it is not
wired to any write-scoped token. It writes a file into the working tree; a human or the
authoring agent commits it, so the escape hatch stays inside the reviewed diff exactly as
mechanism V2 intends.

Usage (report only — what CI runs):
    python3 scripts/feature_off_autodeclare.py --repo . --base-sha <sha> --head-sha <sha> \
        --pr 1234 --report-only

Usage (write the declaration into the working tree):
    python3 scripts/feature_off_autodeclare.py --repo . --base-sha <sha> --head-sha <sha> \
        --pr 1234 --write
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

# ---------------------------------------------------------------------------
# Refusal reasons. Every one of these is a NAMED outcome so a census row can say
# *why* a PR was refused, and so each has its own test.
# ---------------------------------------------------------------------------
REFUSE_BASE_BUILD_FAILED = "base-build-failed"
REFUSE_HEAD_BUILD_FAILED = "head-build-failed"
REFUSE_NEUTRAL_BUILD_FAILED = "neutral-build-failed"
REFUSE_DELETION_BUILD_FAILED = "deletion-neutral-build-failed"
REFUSE_ADDED_LINES_ARE_SEMANTIC = "added-lines-are-semantic"
REFUSE_DELETED_LINES_ARE_SEMANTIC = "deleted-lines-are-semantic"
REFUSE_UNSUPPORTED_FILE_CHANGE = "unsupported-file-change"
REFUSE_VACUOUS_PROOF = "proof-would-be-vacuous"
REFUSE_DIFF_TREE_MISMATCH = "diff-paths-absent-from-both-trees"
REFUSE_NO_DIFF = "no-diff-between-base-and-head"

OUTCOME_NO_DRIFT = "no-drift"
OUTCOME_DECLARED = "declared"

_RUST_SUFFIX = ".rs"
_MANIFESTS = ("Cargo.toml", "Cargo.lock")


def _is_manifest(path: str) -> bool:
    return os.path.basename(path) in _MANIFESTS


def _is_rust(path: str) -> bool:
    return path.endswith(_RUST_SUFFIX)


# Root-level inputs that reach every build regardless of which crate they sit next to.
_ROOT_BUILD_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rust-toolchain")


def closure_dirs(tree: str, package: str = "sparq-wasm") -> list[str] | None:
    """The IN-REPO crate directories `package`'s DEFAULT wasm build actually compiles.

    DERIVED from cargo, never hardcoded: `cargo tree` for the wasm32 target with default
    features names the closure, and `cargo metadata` maps each package to its directory.

    REPORTING ONLY. This once decided which paths the proof covered, and that was the
    source of a live false pass — see `classify_paths`. Soundness no longer depends on it:
    every changed non-manifest path is blanked regardless of what this returns. It is kept
    because naming which changed files the bundle actually compiles is useful evidence in
    the declaration and in a refusal.

    Returns None when cargo cannot answer, which now costs nothing but the evidence line.
    """
    try:
        tree_out = subprocess.run(
            ["cargo", "tree", "-p", package, "--target", "wasm32-unknown-unknown",
             "--edges", "normal", "--prefix", "none"],
            cwd=tree, capture_output=True, text=True, check=True).stdout
        # WITH deps, deliberately. `--no-deps` returns WORKSPACE MEMBERS ONLY, which misses
        # any crate pulled in by `[patch.crates-io]` from a path the workspace `exclude`s —
        # `vendor/spargebra` on this repo, which really is compiled into the bundle.
        meta = json.loads(subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=tree, capture_output=True, text=True, check=True).stdout)
    except Exception:
        return None
    return closure_dirs_from(tree_out, meta, tree)


def closure_dirs_from(tree_out: str, meta: dict, tree: str) -> list[str]:
    """Pure half of `closure_dirs`: intersect a `cargo tree` listing with package dirs."""
    in_closure = {line.split()[0] for line in tree_out.splitlines() if line.strip()}
    real_tree = os.path.realpath(tree)
    dirs = []
    for pkg in meta.get("packages", []):
        if pkg["name"] not in in_closure:
            continue
        pkg_dir = os.path.realpath(os.path.dirname(pkg["manifest_path"]))
        # Registry crates live under ~/.cargo and cannot be touched by a PR diff; only
        # in-repo sources are addressable, so only those become directories to report.
        if pkg_dir == real_tree or not pkg_dir.startswith(real_tree + os.sep):
            continue
        dirs.append(os.path.relpath(pkg_dir, real_tree).rstrip("/") + "/")
    return sorted(set(dirs))


def classify_paths(paths: list[str],
                   closure: list[str] | None = None) -> tuple[list[str], list[str], list[str]]:
    """Split changed paths into (blankable, manifest, in_closure_report).

    EVERY changed path that is not a manifest is BLANKABLE. The classification is
    deliberately NOT used to decide what the proof covers, because that is precisely how
    this tool produced a live false pass:

        sparq's root manifest carries `exclude = ["vendor/spargebra", ...]` together with
        `[patch.crates-io] spargebra = { path = "vendor/spargebra" }`, so spargebra IS
        compiled into the feature-OFF bundle while NOT being a workspace member. An earlier
        version scoped blanking to a closure derived from `cargo metadata --no-deps` —
        which returns workspace members only — so `vendor/spargebra/src/parser.rs`
        classified INERT, was never blanked, and `neutral == head` held BY CONSTRUCTION.
        Three genuinely non-benign changes there all came back `declared`.

    A skipped path cannot be proved harmless, so nothing is skipped. Blanking a file the
    build never reads is free — the bundle is unchanged either way — while blanking one it
    DOES read changes the bundle and the derivation refuses. Letting the compiler answer
    costs nothing and removes an entire class of classification bug.

    Manifests are handled separately (restored from the base tree) because blanking a line
    out of a `Cargo.toml` yields a manifest, not an absence.

    `closure` is reported as evidence only; it no longer gates anything.
    """
    blankable: list[str] = []
    manifest: list[str] = []
    in_closure: list[str] = []
    for p in paths:
        root_input = p in _ROOT_BUILD_INPUTS or p.startswith(".cargo/")
        if _is_manifest(p) or root_input:
            manifest.append(p)
        else:
            blankable.append(p)
        if closure is not None and (root_input or any(p.startswith(d) for d in closure)):
            in_closure.append(p)
    return blankable, manifest, in_closure


# ---------------------------------------------------------------------------
# Diff parsing: which NEW-file lines were added, which OLD-file lines were removed.
# ---------------------------------------------------------------------------

_HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def parse_unified_diff(diff_text: str) -> dict[str, dict[str, list[int]]]:
    """Map path -> {"added": [1-based new-file line numbers],
                    "removed": [1-based old-file line numbers]}.

    Only the line NUMBERS are needed: the content is read back from the real trees, so a
    truncated or context-less diff cannot smuggle content past the obligations.
    """
    out: dict[str, dict[str, list[int]]] = {}
    path: str | None = None
    old_path: str | None = None
    old_ln = new_ln = 0
    for line in diff_text.split("\n"):
        if line.startswith("diff --git "):
            path = old_path = None
            continue
        if line.startswith("--- "):
            src = line[4:].strip()
            old_path = None if src == "/dev/null" else (src[2:] if src.startswith("a/") else src)
            continue
        if line.startswith("+++ "):
            target = line[4:].strip()
            if target == "/dev/null":
                # File DELETED by this diff. Its removed lines must still be attributed, so
                # key them under the OLD path rather than dropping the hunk on the floor.
                path = old_path
            else:
                path = target[2:] if target.startswith("b/") else target
            if path is not None:
                out.setdefault(path, {"added": [], "removed": []})
            continue
        m = _HUNK.match(line)
        if m:
            old_ln = int(m.group(1))
            new_ln = int(m.group(3))
            continue
        if path is None:
            continue
        if line.startswith("+"):
            out[path]["added"].append(new_ln)
            new_ln += 1
        elif line.startswith("-"):
            out[path]["removed"].append(old_ln)
            old_ln += 1
        elif line.startswith(" ") or line == "":
            old_ln += 1
            new_ln += 1
        # "\\ No newline at end of file" and everything else: no line consumed.
    return out


def blank_lines(text: str, line_numbers: list[int]) -> str:
    """Replace the given 1-based lines of `text` with EMPTY lines, preserving line count.

    Preserving the line count is the load-bearing property: it is what makes the neutral
    tree hold the base tree's content at the head tree's line POSITIONS, so a byte-identical
    build proves the blanked lines emitted nothing.
    """
    lines = text.split("\n")
    for n in line_numbers:
        if 1 <= n <= len(lines):
            lines[n - 1] = ""
    return "\n".join(lines)


def nonblank_count(text: str, line_numbers: list[int]) -> int:
    """How many of the given 1-based lines carry non-whitespace content."""
    lines = text.split("\n")
    return sum(1 for n in line_numbers if 1 <= n <= len(lines) and lines[n - 1].strip())


# ---------------------------------------------------------------------------
# Tree materialisation + the real cargo builder.
# ---------------------------------------------------------------------------

def export_tree(repo: str, sha: str, dest: str) -> None:
    """Materialise `sha` into `dest` via `git archive` (never a checkout of the caller's tree)."""
    os.makedirs(dest, exist_ok=True)
    archive = subprocess.run(
        ["git", "-C", repo, "archive", "--format=tar", sha],
        capture_output=True, check=True,
    )
    subprocess.run(["tar", "-x", "-C", dest], input=archive.stdout, check=True)


def cargo_wasm_builder(tree: str) -> bytes | None:
    """Build the feature-OFF sparq-wasm bundle in `tree`; return its bytes, or None on failure.

    Mirrors the leg-2 build exactly: default features only (no `--features vectorized`),
    the `release-wasm` profile, the wasm32 target.
    """
    proc = subprocess.run(
        ["cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm",
         "--target", "wasm32-unknown-unknown",
         "--target-dir", os.path.join(tree, "target")],
        cwd=tree, capture_output=True,
        env={**os.environ, "CARGO_TARGET_DIR": os.path.join(tree, "target")},
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr.decode("utf-8", "replace")[-4000:])
        return None
    # PER-TREE target directory, deliberately. Pointing several materialised trees at ONE
    # shared CARGO_TARGET_DIR to keep the extra builds warm was TRIED and REVERTED: it
    # produced a demonstrably WRONG verdict. Run against #4350 — a PR whose merge-base diff
    # is a 299-line rewrite of crates/sparq-engine/src/exec.rs — the shared-directory run
    # reported the base and head bundles BYTE-IDENTICAL ("no-drift"), i.e. a stale artefact
    # was read back as a fresh build. `git archive` stamps each exported tree's files with
    # its COMMIT time, and cargo's freshness check is mtime-based, so an out-of-order
    # timestamp can leave a previous tree's `sparq_wasm.wasm` in place.
    #
    # A false "identical" is the worst failure this tool has: it would auto-declare a real
    # code change as line-position churn. Cold builds are the price of the proof meaning
    # anything, so `--target-dir` stays inside the tree being built and the environment
    # cannot override it.
    env_override = os.environ.get("CARGO_TARGET_DIR")
    if env_override:
        sys.stderr.write(
            f"[autodeclare] ignoring CARGO_TARGET_DIR={env_override!r}: each tree must build "
            "into its own target directory (a shared one has been observed returning a stale "
            "bundle, which reads as a false 'identical').\n")
    out = os.path.join(tree, "target", "wasm32-unknown-unknown", "release-wasm", "sparq_wasm.wasm")
    if not os.path.exists(out):
        return None
    with open(out, "rb") as fh:
        return fh.read()


def differing_bytes(a: bytes, b: bytes) -> int:
    """Count positions at which two byte strings differ (length difference counts as differing)."""
    n = min(len(a), len(b))
    return sum(1 for i in range(n) if a[i] != b[i]) + abs(len(a) - len(b))


# ---------------------------------------------------------------------------
# The decision.
# ---------------------------------------------------------------------------

class Verdict:
    def __init__(self, outcome: str, reason: str = "", evidence: dict | None = None):
        self.outcome = outcome            # OUTCOME_* or a REFUSE_* name
        self.reason = reason              # human-readable one-liner
        self.evidence = evidence or {}

    @property
    def declared(self) -> bool:
        return self.outcome == OUTCOME_DECLARED

    @property
    def refused(self) -> bool:
        return self.outcome not in (OUTCOME_DECLARED, OUTCOME_NO_DRIFT)


def decide(base_tree: str, head_tree: str, diff_text: str, builder,
           base_bytes: bytes | None = None, head_bytes: bytes | None = None) -> Verdict:
    """Derive whether the base->head feature-OFF drift is line-position churn ONLY.

    `builder(tree_dir) -> bytes | None` compiles a materialised tree. Injected so the
    decision logic is testable without cargo; production passes `cargo_wasm_builder`.

    `base_bytes` / `head_bytes` let a caller hand in bundles the leg-2 job already built,
    so the derivation costs one extra build (two when the diff deletes code) rather than
    four. They are used verbatim — the obligations below still rebuild the NEUTRAL trees
    with the same builder, so a handed-in bundle cannot short-circuit any proof.
    """
    changes = parse_unified_diff(diff_text)
    if not changes:
        return Verdict(REFUSE_NO_DIFF, "the base..head diff is empty — nothing to attribute")

    closure = closure_dirs(head_tree)
    rust_paths, manifest_paths, closure_paths = classify_paths(sorted(changes), closure)

    # The diff and the trees must actually describe the same thing. A changed path that
    # exists in NEITHER tree means they disagree — a mis-parsed diff, a bad path prefix, a
    # stale export. Every such path would be silently skipped by the blanking loop, so the
    # proof would quietly cover less than it claims. Caught for real: an ad-hoc harness
    # rewrote `git diff --no-index` prefixes in the wrong order, yielding paths like
    # `bvendor/spargebra/src/parser.rs`; nothing matched, nothing was blanked, and only the
    # non-vacuity guard stood between that and a false declaration. Name it instead.
    missing = [p for p in rust_paths
               if not os.path.exists(os.path.join(head_tree, p))
               and not os.path.exists(os.path.join(base_tree, p))]
    if missing:
        return Verdict(REFUSE_DIFF_TREE_MISMATCH,
                       "the diff names paths that exist in neither the base nor the head "
                       "tree, so the diff and the trees disagree and the proof would cover "
                       "less than it claims: " + ", ".join(sorted(missing)[:10]))

    # Blanking rewrites files in place, which is meaningless for a symlink or a submodule
    # gitlink — the proof would silently cover nothing. Refuse rather than pretend.
    nonregular = [p for p in rust_paths
                  if os.path.islink(os.path.join(head_tree, p))
                  or os.path.islink(os.path.join(base_tree, p))]
    if nonregular:
        return Verdict(REFUSE_UNSUPPORTED_FILE_CHANGE,
                       "changed inside the wasm build closure but not a regular file, so "
                       "blanking cannot speak for it: " + ", ".join(sorted(nonregular)))

    if base_bytes is None:
        base_bytes = builder(base_tree)
    if base_bytes is None:
        return Verdict(REFUSE_BASE_BUILD_FAILED, "the BASE tree did not build")
    if head_bytes is None:
        head_bytes = builder(head_tree)
    if head_bytes is None:
        return Verdict(REFUSE_HEAD_BUILD_FAILED, "the HEAD tree did not build")

    if base_bytes == head_bytes:
        return Verdict(OUTCOME_NO_DRIFT,
                       "base and head bundles are byte-identical; leg 2 already passes",
                       {"bundle_bytes": len(head_bytes)})

    evidence = {
        "base_bundle_bytes": len(base_bytes),
        "head_bundle_bytes": len(head_bytes),
        "size_delta_bytes": len(head_bytes) - len(base_bytes),
        "differing_bytes": differing_bytes(base_bytes, head_bytes),
        "closure_files_changed": rust_paths,
        "manifest_files_changed": manifest_paths,
        "files_in_compiled_closure": closure_paths,
        "closure_dirs": closure,
    }

    # ---- Obligation 1: additions contribute no compiled code ----------------
    # Neutral tree = HEAD with every added .rs line blanked and every changed manifest
    # restored from BASE. It therefore holds BASE's compiled content at HEAD's line
    # positions. If its bundle equals HEAD's, the additions emitted nothing.
    removed_nonblank = 0
    to_blank: dict[str, list[int]] = {}
    for path in rust_paths:
        removed = changes[path]["removed"]
        if not removed:
            continue
        src = os.path.join(base_tree, path)
        if not os.path.exists(src):
            continue
        with open(src, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        n = nonblank_count(text, removed)
        if n:
            removed_nonblank += n
            to_blank[path] = removed
    evidence["deleted_nonblank_lines"] = removed_nonblank

    neutral = os.path.join(tempfile.mkdtemp(prefix="featoff-neutral-"), "tree")
    shutil.copytree(head_tree, neutral, symlinks=True)
    added_nonblank = 0
    mutated: list[str] = []
    for path in rust_paths:
        added = changes[path]["added"]
        if not added:
            continue
        target = os.path.join(neutral, path)
        if not os.path.exists(target):
            continue
        with open(target, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        added_nonblank += nonblank_count(text, added)
        blanked = blank_lines(text, added)
        if blanked != text:
            mutated.append(path)
        with open(target, "w", encoding="utf-8", errors="surrogateescape") as fh:
            fh.write(blanked)
    for path in manifest_paths:
        src = os.path.join(base_tree, path)
        dst = os.path.join(neutral, path)
        head_bytes_of = open(dst, "rb").read() if os.path.exists(dst) else None
        if os.path.exists(src):
            shutil.copyfile(src, dst)
            if open(dst, "rb").read() != head_bytes_of:
                mutated.append(path)
        elif os.path.exists(dst):
            os.remove(dst)
            mutated.append(path)
    evidence["added_nonblank_lines_blanked"] = added_nonblank
    evidence["neutral_tree_files_mutated"] = len(mutated)

    # NON-VACUITY. Obligation 1 concludes "the additions emitted nothing" from
    # `build(neutral) == build(head)`. If the neutral tree is IDENTICAL to the head tree,
    # that comparison is `head == head` — it holds by construction and proves nothing, so a
    # declaration derived from it would be exactly the rubber stamp this leg exists to
    # prevent. This is the general form of the vendored-crate false pass: whenever the diff
    # is real but nothing got blanked, the drift came from somewhere the proof does not
    # reach. Refuse, and say where the diff actually was.
    if not mutated and not to_blank:
        return Verdict(
            REFUSE_VACUOUS_PROOF,
            "neither obligation has anything to prove: the neutral tree is identical to the "
            "head tree (nothing was blanked) and the diff deletes no non-blank line, so "
            "comparing the bundles would be `head == head` and would hold by construction. "
            "The bundle moved anyway, so the drift originates outside what the proof "
            "reaches — do not declare it. Changed paths: "
            + ", ".join(sorted(changes)[:20]),
            evidence)

    neutral_bytes = builder(neutral)
    if neutral_bytes is None:
        # Blanking the added lines broke the build => at least one of them was load-bearing.
        return Verdict(REFUSE_NEUTRAL_BUILD_FAILED,
                       "blanking the added lines broke the build, so they are compiled code, "
                       "not comments or compiled-out cfg-gated tokens",
                       evidence)
    if neutral_bytes != head_bytes:
        evidence["neutral_vs_head_size_delta"] = len(head_bytes) - len(neutral_bytes)
        evidence["neutral_vs_head_differing_bytes"] = differing_bytes(neutral_bytes, head_bytes)
        return Verdict(REFUSE_ADDED_LINES_ARE_SEMANTIC,
                       "blanking the added lines CHANGED the compiled bundle, so the diff adds "
                       "code to the default build; an intentional always-compiled change must "
                       "be declared by its author, not derived",
                       evidence)

    # ---- Obligation 2: deletions removed no compiled code -------------------
    # Deleted lines are absent from BOTH head and neutral, so obligation 1 cannot speak for
    # them. Run the same proof on the base side: blank exactly those lines in BASE and
    # require the bundle to be unchanged.
    if to_blank:
        del_neutral = os.path.join(tempfile.mkdtemp(prefix="featoff-delneutral-"), "tree")
        shutil.copytree(base_tree, del_neutral, symlinks=True)
        for path, removed in to_blank.items():
            target = os.path.join(del_neutral, path)
            with open(target, encoding="utf-8", errors="surrogateescape") as fh:
                text = fh.read()
            with open(target, "w", encoding="utf-8", errors="surrogateescape") as fh:
                fh.write(blank_lines(text, removed))
        del_bytes = builder(del_neutral)
        if del_bytes is None:
            return Verdict(REFUSE_DELETION_BUILD_FAILED,
                           "blanking the deleted lines in the BASE tree broke the build, so the "
                           "diff removes compiled code",
                           evidence)
        if del_bytes != base_bytes:
            evidence["delneutral_vs_base_size_delta"] = len(base_bytes) - len(del_bytes)
            evidence["delneutral_vs_base_differing_bytes"] = differing_bytes(del_bytes, base_bytes)
            return Verdict(REFUSE_DELETED_LINES_ARE_SEMANTIC,
                           "blanking the deleted lines in the BASE tree CHANGED the compiled "
                           "bundle, so the diff removes code from the default build; that is an "
                           "always-compiled change its author must declare",
                           evidence)

    return Verdict(
        OUTCOME_DECLARED,
        "every added line was proved to emit nothing (neutral bundle == head bundle) and "
        "every deleted line was proved to have emitted nothing (base-neutral bundle == base "
        "bundle); the drift is line-position metadata only",
        evidence,
    )


def _obligation_sentence(ev: dict) -> str:
    """Say which obligation actually carried the proof.

    A count of zero is normal on one side — a pure-deletion diff blanks no added line, and
    an addition-only diff deletes none — but phrasing it as "all 0 lines blanked produced an
    identical bundle" reads as a confession that nothing was checked. It also read that way
    when something really HAD been missed: the vendored-crate false pass emitted exactly
    that sentence with both counts at zero. The non-vacuity guard now refuses that state
    outright; this wording makes the remaining zeroes unambiguous.
    """
    added = ev.get("added_nonblank_lines_blanked", 0)
    deleted = ev.get("deleted_nonblank_lines", 0)
    parts = []
    if added:
        parts.append(f"rebuilding the head tree with all {added} added non-blank line(s) "
                     "blanked produced a BYTE-IDENTICAL bundle")
    if deleted:
        parts.append(f"rebuilding the base tree with all {deleted} deleted non-blank line(s) "
                     "blanked left the base bundle unchanged")
    if not parts:  # unreachable: the non-vacuity guard refuses before this point
        return ("NO obligation was discharged — this declaration must not have been derived."
                " Treat it as a bug in the derivation, not as evidence.")
    joined = ", and ".join(parts)
    covered = ("Both directions were covered" if len(parts) == 2
               else "The diff changes lines in one direction only, so that is the whole "
                    "obligation")
    return f"Specifically: {joined}. {covered}."


def declaration_json(pr: int, verdict: Verdict, date: str | None = None) -> dict:
    """The per-PR declaration file content, carrying the MEASURED evidence."""
    ev = verdict.evidence
    return {
        "pr": pr,
        "date": date or datetime.date.today().isoformat(),
        "reason": (
            f"[OPUS-5] #{pr}: DERIVED declaration (scripts/feature_off_autodeclare.py). The "
            f"feature-OFF bundle moved {ev.get('differing_bytes')} of "
            f"{ev.get('head_bundle_bytes')} bytes at a size delta of "
            f"{ev.get('size_delta_bytes'):+d}, and the drift was proved to carry no compiled "
            f"code. {_obligation_sentence(ev)} What moved is line-position metadata "
            "(core::panic::Location line numbers shift when lines above them move); no "
            "always-compiled code entered or left the default build. Bundle SIZE remains "
            "governed separately by the wasm_bundle_bytes ratchet."
        ),
        "derived": True,
        "evidence": ev,
    }


def census_row(pr: int, verdict: Verdict) -> str:
    """One machine-greppable census line, emitted on every run."""
    return (
        "FEATURE-OFF-CENSUS "
        f"pr={pr} outcome={verdict.outcome} "
        f"size_delta={verdict.evidence.get('size_delta_bytes', 'na')} "
        f"differing_bytes={verdict.evidence.get('differing_bytes', 'na')} "
        f"reason={verdict.reason.split(';')[0][:160]!r}"
    )


# ---------------------------------------------------------------------------
# Census sweep: how big is this class right now, and what happened to each member?
# ---------------------------------------------------------------------------

LEG_NAME = "artifact-exact-equality (wasm bundle feature-OFF)"


def _gh_json(path: str) -> object:
    proc = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip()[:400])
    return json.loads(proc.stdout)


def _default_pr_lister(repo: str) -> list[dict]:
    proc = subprocess.run(
        ["gh", "pr", "list", "--repo", repo, "--state", "open", "--limit", "200",
         "--json", "number,headRefOid,isDraft,title"],
        capture_output=True, text=True, check=True,
    )
    return json.loads(proc.stdout)


def _default_check_lister(repo: str, sha: str) -> list[dict]:
    runs: list[dict] = []
    page = 1
    while page <= 5:
        d = _gh_json(f"/repos/{repo}/commits/{sha}/check-runs?per_page=100&page={page}")
        batch = d.get("check_runs", [])  # type: ignore[union-attr]
        runs.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return runs


def census_sweep(repo: str, declarations_dir_listing: set[str],
                 pr_lister=_default_pr_lister, check_lister=_default_check_lister) -> dict:
    """Tally the live population of the feature-OFF declaration class.

    Membership is decided by the LEG's own conclusion on each PR's CURRENT head SHA (matched
    by NAME EQUALITY, and paginated), never by a cached label — a stale classification is how
    this class goes quietly wrong. Returns counts plus the per-PR rows.
    """
    rows: list[dict] = []
    prs = pr_lister(repo)
    for pr in prs:
        try:
            checks = check_lister(repo, pr["headRefOid"])
        except Exception as exc:
            rows.append({"pr": pr["number"], "state": "check-lookup-failed", "detail": str(exc)})
            continue
        # NAME EQUALITY, newest run wins. A substring match would sweep in an advisory twin,
        # and a first-run-wins scan would read a superseded red on a re-run head.
        leg: dict | None = None
        for c in checks:
            if c["name"] != LEG_NAME:
                continue
            if leg is None or (c.get("started_at") or "") >= (leg.get("started_at") or ""):
                leg = c
        if leg is None:
            state = "leg-absent"
        elif leg.get("conclusion") == "failure":
            state = "in-class-red"
        elif leg.get("status") != "completed":
            state = "leg-running"
        else:
            state = "leg-green"
        rows.append({
            "pr": pr["number"], "draft": pr.get("isDraft"), "state": state,
            "declared": f"{pr['number']}.json" in declarations_dir_listing,
        })
    in_class = [r for r in rows if r.get("state") == "in-class-red"]
    return {
        "open_prs": len(prs),
        "in_class_red": len(in_class),
        "in_class_already_declared": sum(1 for r in in_class if r.get("declared")),
        "in_class_undeclared": sum(1 for r in in_class if not r.get("declared")),
        "leg_running": sum(1 for r in rows if r.get("state") == "leg-running"),
        "leg_absent": sum(1 for r in rows if r.get("state") == "leg-absent"),
        "lookup_failed": sum(1 for r in rows if r.get("state") == "check-lookup-failed"),
        "rows": rows,
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--repo", default=".", help="git repository to export trees from")
    ap.add_argument("--base-sha")
    ap.add_argument("--head-sha")
    ap.add_argument("--pr", type=int)
    ap.add_argument("--declarations-dir", default="bench/feature-off-declarations")
    ap.add_argument("--base-wasm", help="a base-tree bundle the caller already built")
    ap.add_argument("--head-wasm", help="a head-tree bundle the caller already built")
    ap.add_argument("--write", action="store_true",
                    help="write the declaration file into --repo's working tree")
    ap.add_argument("--report-only", action="store_true",
                    help="print the verdict and the declaration that WOULD be written")
    ap.add_argument("--census", metavar="OWNER/REPO",
                    help="sweep the live open-PR population of this class and exit")
    ap.add_argument("--summary-file", default=os.environ.get("GITHUB_STEP_SUMMARY", ""))
    args = ap.parse_args(argv)

    if args.census:
        decl_dir = os.path.join(args.repo, args.declarations_dir)
        listing = set(os.listdir(decl_dir)) if os.path.isdir(decl_dir) else set()
        rep = census_sweep(args.census, listing)
        print("FEATURE-OFF-CENSUS-SWEEP " + " ".join(
            f"{k}={v}" for k, v in rep.items() if k != "rows"))
        for r in rep["rows"]:
            if r.get("state") in ("in-class-red", "check-lookup-failed"):
                print("  " + json.dumps(r))
        return 0

    missing = [n for n in ("base_sha", "head_sha", "pr") if getattr(args, n) is None]
    if missing:
        ap.error("required unless --census is given: " + ", ".join("--" + m.replace("_", "-")
                                                                   for m in missing))

    prebuilt: dict[str, bytes | None] = {"base": None, "head": None}
    for key, path in (("base", args.base_wasm), ("head", args.head_wasm)):
        if path:
            with open(path, "rb") as fh:
                prebuilt[key] = fh.read()

    # ATTRIBUTION BASE = the MERGE BASE, not `pull_request.base.sha`. Those are different
    # commits whenever the branch is behind its base: GitHub sets `base.sha` to the base
    # BRANCH TIP at the last sync, so a `base.sha`..`head` diff also carries, in reverse,
    # every base-branch commit the PR does not have. MEASURED on the live population: 3 of
    # the 6 open PRs red on this leg had a non-ancestor `base.sha`, and one of them showed
    # 22 changed files against `base.sha` for a ONE-file PR. Attributing that to the PR
    # would be wrong in both directions, so the proof runs against the merge base and the
    # difference is reported rather than hidden.
    merge_base = subprocess.run(
        ["git", "-C", args.repo, "merge-base", args.base_sha, args.head_sha],
        capture_output=True, text=True, check=True).stdout.strip()
    if merge_base != args.base_sha:
        print(f"[autodeclare] NOTE: leg 2's base ({args.base_sha[:12]}) is not the merge base "
              f"({merge_base[:12]}); attributing against the merge base, and the supplied "
              "base bundle is ignored because it was built from a different tree.")
        prebuilt["base"] = None

    workdir = tempfile.mkdtemp(prefix="featoff-trees-")
    base_tree = os.path.join(workdir, "base")
    head_tree = os.path.join(workdir, "head")
    export_tree(args.repo, merge_base, base_tree)
    export_tree(args.repo, args.head_sha, head_tree)
    diff_text = subprocess.run(
        ["git", "-C", args.repo, "diff", "--no-color", "--unified=0",
         merge_base, args.head_sha],
        capture_output=True, text=True, check=True,
    ).stdout

    verdict = decide(base_tree, head_tree, diff_text, cargo_wasm_builder,
                     base_bytes=prebuilt["base"], head_bytes=prebuilt["head"])

    print(census_row(args.pr, verdict))
    print(f"[autodeclare] outcome: {verdict.outcome}")
    print(f"[autodeclare] {verdict.reason}")
    print("[autodeclare] evidence: " + json.dumps(verdict.evidence, indent=2, default=str))

    lines = [
        "### feature-OFF declaration derivation",
        "",
        f"* **outcome**: `{verdict.outcome}`",
        f"* {verdict.reason}",
        "",
        "```json",
        json.dumps(verdict.evidence, indent=2, default=str),
        "```",
    ]
    if verdict.declared:
        doc = declaration_json(args.pr, verdict)
        rel = os.path.join(args.declarations_dir, f"{args.pr}.json")
        lines += ["", f"Commit this as `{rel}`:", "", "```json",
                  json.dumps(doc, indent=2), "```"]
        if args.write:
            dest = os.path.join(args.repo, rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "w", encoding="utf-8") as fh:
                json.dump(doc, fh, indent=2)
                fh.write("\n")
            print(f"[autodeclare] wrote {dest}")
        else:
            print(json.dumps(doc, indent=2))
    if args.summary_file:
        try:
            with open(args.summary_file, "a", encoding="utf-8") as fh:
                fh.write("\n".join(lines) + "\n")
        except OSError:
            pass

    # Exit 0 when a declaration was derived or none was needed; non-zero on a REFUSAL so a
    # caller that wired this in cannot mistake "refused" for "handled".
    return 0 if not verdict.refused else 2


if __name__ == "__main__":
    raise SystemExit(main())
