#!/usr/bin/env python3
"""
[OPUS-4.8] sq-pntvh.8 / [OPUS-4.8] sq-v3nel: feature-OFF CI proof for M4 vectorization.

Proves compile-time absence (leg1, leg3) and artifact byte-DETERMINISM (leg2) of
the vectorized-OFF build. Does NOT prove runtime performance neutrality — that is
EC2-gated. This is the strongest claim deterministic ratchet infrastructure can
support.

Three legs:
  --leg1 <metadata-json>   cargo metadata check: `vectorized` absent from default features
  --leg2-dynamic <base.wasm> <head.wasm> <base-decl.json> <head-decl.json>
                           DYNAMIC byte-identity: compare the feature-OFF wasm built
                           from the BASE (target-branch/merge-base) tree against the one
                           built from the HEAD (PR/merged) tree IN THE SAME CI RUN. No
                           static byte pin => no merge-order dependence. Identical bytes
                           => pass. Differing bytes => pass only if DECLARED (see MECHANISM
                           V2 below); else fail.
     [--declarations-dirs <base-dir> <head-dir>]
                           MECHANISM V2 (sq-v3nel-v2): per-PR declaration FILES. A PR
                           declares by ADDING its own bench/feature-off-declarations/
                           <PR-number>.json (or .md) file. "declared" == the head-tree
                           declarations directory contains at least one <digits>.json/.md
                           file the base-tree directory does NOT (set difference). Different
                           PRs add different files, so git NEVER textually conflicts (the
                           old scalar change_token put every declaring PR on the SAME line).
  --leg3                   cfg-audit: every vectorized call site in lib.rs, exec.rs and
                           every child module of exec is gated; fails if exec declares a
                           child module the audit does not scan
  --self-test              run built-in tripwires that MUST fail (guard the guards)

[OPUS-4.8] sq-v3nel (2026-07-07): leg 2 was RE-DESIGNED from a static exact-equality
pin (bench/perf-baseline.json metrics.wasm_bundle_bytes.feature_off_exact) to this
DYNAMIC same-run base-vs-head byte comparison. The static pin was ORDER-DEPENDENT: any
PR touching always-compiled engine/core code changed the merged-tree bundle bytes, the
correct pin value depended on merge ORDER, and armed PRs cycled BLOCKED while serial
re-pin commits chased a moving target (see the retirement _comment in perf-baseline.json).
The dynamic check has no static pin: a PR that does not touch always-compiled code yields
byte-identical builds and passes deterministically; a PR that does must DECLARE the change
by bumping change_token (audit-visible, reviewed diff). The SIZE of an accepted change is
still governed, unchanged, by the wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml.

Declaration mechanism V2 (sq-v3nel-v2, 2026-07-07): per-PR FILES, not a shared scalar.
  A PR declares an intentional feature-OFF byte change by ADDING its own
  bench/feature-off-declarations/<PR-number>.json file (fields: {pr, date, reason}).
  The gate is satisfied iff the head tree's declarations directory contains at least one
  <digits>.json/.md file the base tree's does NOT — a set difference on the directory
  listing. Because different PRs add DIFFERENT files, two declaring PRs never touch the
  same line and git never marks them CONFLICTING.

  WHY V2 replaced the scalar change_token: the original mechanism stored ONE integer in
  bench/feature-off-declaration.json, so EVERY declaring PR edited the SAME line. The
  gate check was order-independent but the FILE was not: the first declared PR to merge
  made every other declared PR textually CONFLICTING (#1720 and #1718 both went DIRTY for
  exactly this while #1726's declaration merged). Per-PR files remove the shared line.

  TRANSITION WINDOW: check_leg2_dynamic accepts EITHER mechanism so in-flight branches
  that already bumped the scalar token do not break. "declared" is true if the scalar
  change_token differs between base and head (legacy) OR a new declaration file was added
  (V2). The legacy scalar file bench/feature-off-declaration.json is RETIRED (frozen, kept
  parseable so the script's fail-safe token read still works) but its inequality path stays
  live until all pre-V2 branches have merged or rebased.

  WHY A PR-body marker was still REJECTED: it is invisible in merge_group events (no PR
  body on the merged commit set) and is not part of the reviewed diff. A committed per-PR
  file is audit-visible, least-gameable, and conflict-free.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import tempfile

# ---------------------------------------------------------------------------
# Constants — the source files leg 3 audits.
# ---------------------------------------------------------------------------
_EXEC_RS = "crates/sparq-engine/src/exec.rs"
# exec.rs is split into child modules under exec/. Leg 3 scans every .rs file there.
_EXEC_DIR = "crates/sparq-engine/src/exec"
# Child modules of `exec` that live OUTSIDE exec/ because they are declared with
# `#[path = ...]`. A new out-of-tree child must be added here, or the
# child-module tripwire in leg 3 fails.
_EXEC_PATH_CHILDREN = ("crates/sparq-engine/src/eqjoin.rs",)
_LIB_RS = "crates/sparq-engine/src/lib.rs"
_CHUNK_RS = "crates/sparq-engine/src/chunk.rs"

# Patterns that, when appearing OUTSIDE chunk.rs, must be inside a
# `#[cfg(feature = "vectorized")]` guard.
_CHUNK_IMPORT_PATTERN = re.compile(r'\bchunk::|DataChunk\b|SelVec\b|VecCmp\b|apply_filter_columnar\b')
_MOD_CHUNK_PATTERN = re.compile(r'\b(?:mod|pub use)\s+chunk\b')
_CFG_VECTORIZED = re.compile(r'#\[cfg\(feature\s*=\s*"vectorized"\)\]')
# Out-of-line module declaration, after any same-line attributes are removed.
_MOD_DECL = re.compile(
    r'^(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;')
# Anything that looks like an out-of-line `mod x;`, used to fail closed on a
# declaration the parser above does not understand.
_MOD_DECL_LOOSE = re.compile(r'\bmod\s+[A-Za-z_][A-Za-z0-9_]*\s*;')
_PATH_ATTR = re.compile(r'^#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]$')
# `#[cfg_attr(<pred>, ..., path = "x.rs")]` switches a module's file per
# configuration. Leg 3 does not model that; such a declaration fails as unsupported.
_CFG_ATTR_HEAD = re.compile(r'^#\s*\[\s*cfg_attr\s*\(')
_STRING_LIT = re.compile(r'"(?:[^"\\]|\\.)*"')
_CFG_ATTR = re.compile(r'^#\s*\[\s*cfg\s*\((.*)\)\s*\]$', re.S)
_CFG_TOKEN = re.compile(r'\s*(?:([A-Za-z_][A-Za-z0-9_]*)|("(?:[^"\\]|\\.)*")|([(),=]))')


# ---------------------------------------------------------------------------
# Leg 1: cargo metadata — vectorized absent from resolved default features
# ---------------------------------------------------------------------------

def check_leg1(metadata_path: str) -> int:
    """
    Returns 0 if vectorized is absent everywhere it must be absent, 1 otherwise.
    Checks:
      a) No sparq-engine node in resolve has 'vectorized' in its enabled features.
      b) No sparq-wasm node in resolve has 'vectorized' in its transitively-enabled features.
      c) sparq-engine package definition does not list 'vectorized' in default feature set.
    """
    try:
        with open(metadata_path) as fh:
            meta = json.load(fh)
    except Exception as exc:
        print(f"[leg1] ERROR: cannot read metadata file {metadata_path!r}: {exc}", file=sys.stderr)
        return 1

    violations: list[str] = []

    # (a) + (b): check resolve nodes
    nodes = meta.get("resolve", {}).get("nodes", [])
    for node in nodes:
        node_id: str = node.get("id", "")
        features: list[str] = node.get("features", [])
        if "sparq-engine" in node_id:
            if "vectorized" in features:
                violations.append(
                    f"[leg1] VIOLATION: sparq-engine node {node_id!r} has 'vectorized' "
                    f"in resolved features: {features}"
                )
        if "sparq-wasm" in node_id:
            if "vectorized" in features:
                violations.append(
                    f"[leg1] VIOLATION: sparq-wasm node {node_id!r} has 'vectorized' "
                    f"in resolved features: {features} — wasm must never enable vectorized"
                )

    # (c): package definitions — default feature list
    packages = meta.get("packages", [])
    for pkg in packages:
        name: str = pkg.get("name", "")
        if "sparq-engine" in name:
            pkg_features: dict = pkg.get("features", {})
            default_features: list[str] = pkg_features.get("default", [])
            if "vectorized" in default_features:
                violations.append(
                    f"[leg1] VIOLATION: sparq-engine package {name!r} lists 'vectorized' "
                    f"in its default feature set: {default_features}"
                )

    if violations:
        for v in violations:
            print(v)
        print(f"\n[leg1] FAIL — {len(violations)} violation(s) found. "
              "The `vectorized` feature must NOT be in any resolved default feature set.")
        return 1

    print("[leg1] OK — 'vectorized' is absent from all resolved default feature sets "
          "(sparq-engine, sparq-wasm, package defaults). Compile-time absence confirmed.")
    return 0


# ---------------------------------------------------------------------------
# Leg 2: DYNAMIC byte-identity — feature-OFF wasm, base tree vs head tree (sq-v3nel)
# ---------------------------------------------------------------------------

def _read_change_token(decl_path: str) -> int:
    """Read the integer 'change_token' from a feature-off declaration JSON file (LEGACY).

    Retained for the TRANSITION WINDOW (sq-v3nel-v2): the scalar mechanism is retired but
    in-flight branches that already bumped the token must still be accepted. The V2
    per-PR-file mechanism (_new_declaration_files below) is the going-forward path.

    Fail-SAFE default: a missing file, missing field, or non-integer value reads as 0.
    Because an ABSENT declaration reads the same as an UN-bumped one, an undeclared byte
    change still fails the gate (the check compares base vs head token for INEQUALITY).
    """
    try:
        with open(decl_path) as fh:
            data = json.load(fh)
    except (OSError, ValueError):
        return 0
    tok = data.get("change_token", 0) if isinstance(data, dict) else 0
    try:
        return int(tok)
    except (TypeError, ValueError):
        return 0


# MECHANISM V2 (sq-v3nel-v2): a declaration FILE is bench/feature-off-declarations/
# <PR-number>.json (or .md). Only names matching <digits>.json|md count as declarations,
# so a README.md / .gitkeep in the directory is NOT mistaken for a declaration.
_DECL_FILE_RE = re.compile(r'^\d+\.(?:json|md)$')


def _list_declaration_files(dir_path: str | None) -> set[str]:
    """Return the set of per-PR declaration file names in a declarations directory.

    Fail-SAFE: a None / missing / non-directory path reads as the EMPTY set, so an absent
    directory can never be mistaken for a declaration (keeps the gate fail-closed).
    Only names matching <digits>.json|md are counted (README.md/.gitkeep are ignored).
    """
    if not dir_path or not os.path.isdir(dir_path):
        return set()
    return {name for name in os.listdir(dir_path) if _DECL_FILE_RE.match(name)}


def _new_declaration_files(base_dir: str | None, head_dir: str | None) -> set[str]:
    """Return declaration files the HEAD tree has that the BASE tree does NOT (set diff).

    A non-empty result means the PR ADDED at least one bench/feature-off-declarations/
    <PR-number>.json — i.e. it DECLARED an intentional feature-OFF byte change under
    mechanism V2. Because each PR adds a distinctly-named file, this never collides with
    another declaring PR (unlike the old shared scalar token line).
    """
    return _list_declaration_files(head_dir) - _list_declaration_files(base_dir)


def check_leg2_dynamic(base_wasm: str, head_wasm: str,
                       base_decl: str, head_decl: str,
                       base_decls_dir: str | None = None,
                       head_decls_dir: str | None = None) -> int:
    """DYNAMIC byte-identity gate for the feature-OFF wasm bundle (sq-v3nel / sq-v3nel-v2).

    Compares the feature-OFF wasm built from the BASE (target-branch / merge-base) tree
    against the one built from the HEAD (PR / merged) tree IN THE SAME CI RUN. Same
    toolchain + same runner in one run => a deterministic comparison with NO static byte
    pin, so NO merge-order dependence.

    Policy:
      * bytes byte-for-byte IDENTICAL  -> PASS. The PR touched no always-compiled code
        that reaches the feature-OFF bundle. (This is the common case for non-engine PRs.)
      * bytes DIFFER + change DECLARED -> PASS. 'declared' is satisfied by EITHER mechanism
        (transition window, sq-v3nel-v2):
          V2 (going-forward): the head-tree bench/feature-off-declarations/ directory
            contains a <PR-number>.json/.md file the base tree does NOT (set difference).
            Conflict-free: each PR adds its own file.
          LEGACY (retired, still accepted): change_token differs between the base-tree and
            head-tree bench/feature-off-declaration.json (a pre-V2 branch bumped it).
        The SIZE of the change is governed SEPARATELY, unchanged, by the wasm_bundle_bytes
        floor ratchet (+/-2% band) in bench.yml — this gate governs INTENT, not magnitude.
      * bytes DIFFER + NOT declared    -> FAIL. Either accidental default-path/vectorized
        code leaked into the feature-OFF build (remove / cfg-gate it) or an intentional
        always-compiled change was not declared (add a per-PR declaration file).

    Returns 0 on pass, 1 on fail/error.
    """
    try:
        with open(base_wasm, "rb") as fh:
            base_bytes = fh.read()
    except OSError as exc:
        print(f"[leg2] ERROR: cannot read base wasm {base_wasm!r}: {exc}", file=sys.stderr)
        return 1
    try:
        with open(head_wasm, "rb") as fh:
            head_bytes = fh.read()
    except OSError as exc:
        print(f"[leg2] ERROR: cannot read head wasm {head_wasm!r}: {exc}", file=sys.stderr)
        return 1

    base_len, head_len = len(base_bytes), len(head_bytes)

    if base_bytes == head_bytes:
        print(
            f"[leg2] OK — feature-OFF wasm is byte-for-byte IDENTICAL to the base tree "
            f"({head_len} bytes). No always-compiled code changed the default build; "
            "deterministic same-run comparison, no static pin, no merge-order dependence."
        )
        return 0

    # Bytes differ (size and/or content). Require an audit-visible declaration.
    delta = head_len - base_len
    pct = (delta / base_len * 100) if base_len else float("inf")

    print(
        f"[leg2] feature-OFF wasm DIFFERS from the base tree: "
        f"base={base_len} head={head_len} bytes, size delta={delta:+d} ({pct:+.3f}%). "
        "Comparison is byte-for-byte, so same-length content changes are also detected."
    )

    # V2 (going-forward): a per-PR declaration FILE was added under
    # bench/feature-off-declarations/ (set difference on the directory listing). Preferred
    # because each PR adds a distinct file, so declaring PRs never textually conflict.
    new_files = _new_declaration_files(base_decls_dir, head_decls_dir)
    if new_files:
        print(
            "[leg2] OK — change DECLARED (mechanism V2): the head tree added "
            f"bench/feature-off-declarations/{sorted(new_files)} not present in the base "
            "tree. The feature-OFF byte change is intentional. Its SIZE is governed by the "
            "wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate."
        )
        return 0

    # LEGACY (retired, accepted during the transition window): the scalar change_token in
    # bench/feature-off-declaration.json differs between base and head tree. Kept live so
    # pre-V2 branches that already bumped the token do not break.
    base_tok = _read_change_token(base_decl)
    head_tok = _read_change_token(head_decl)
    if head_tok != base_tok:
        print(
            f"[leg2] OK — change DECLARED (legacy scalar, transition window): "
            f"bench/feature-off-declaration.json change_token {base_tok} -> {head_tok}. "
            "The feature-OFF byte change is intentional. Its SIZE is governed by the "
            "wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate. "
            "NOTE: the scalar mechanism is retired — new PRs should add a per-PR declaration "
            "file under bench/feature-off-declarations/ (mechanism V2) instead."
        )
        return 0

    print(
        f"[leg2] VIOLATION: the feature-OFF wasm bundle changed vs the base tree "
        f"(size delta {delta:+d} bytes, {pct:+.3f}%) but the change is NOT declared "
        f"(no new bench/feature-off-declarations/<PR>.json file added, and the legacy scalar "
        f"change_token is unchanged at {base_tok}).\n"
        "  - If this is ACCIDENTAL vectorized / default-path code leaking into the "
        "feature-OFF build, REMOVE it or gate it behind #[cfg(feature = \"vectorized\")].\n"
        "  - If this is an INTENTIONAL change to always-compiled engine/core code, DECLARE "
        "it (mechanism V2): add bench/feature-off-declarations/<PR-number>.json with "
        "{pr, date, reason}. If the SIZE moves > +/-2%, ALSO raise "
        "metrics.wasm_bundle_bytes.floor in bench/perf-baseline.json (the bench.yml ratchet)."
    )
    print("\n[leg2] FAIL — undeclared feature-OFF bundle change. Declare it or remove it.")
    return 1


# ---------------------------------------------------------------------------
# Leg 3: cfg-audit — every vectorized registration and call site is gated
# ---------------------------------------------------------------------------

def _lines_have_cfg_guard(lines: list[str], target_lineno: int, window: int = 3) -> bool:
    """Return True if any of the `window` lines BEFORE target_lineno (1-indexed) match
    the `#[cfg(feature = "vectorized")]` pattern."""
    start = max(0, target_lineno - 1 - window)
    end = target_lineno - 1  # exclusive (line itself not checked)
    for i in range(start, end):
        if _CFG_VECTORIZED.search(lines[i]):
            return True
    return False


def _is_inside_cfg_vectorized_block(lines: list[str], target_lineno: int) -> bool:
    """
    Heuristic: scan backwards from target_lineno for an OPEN `#[cfg(feature = "vectorized")]`
    that hasn't been balanced by a closing `}`. Used for multi-line blocks in exec.rs.
    We track brace depth: when we see the cfg attribute and depth==0 at that point, we're in.
    """
    # Walk backwards from the line before target
    depth = 0
    for i in range(target_lineno - 2, -1, -1):
        line = lines[i]
        depth += line.count('}') - line.count('{')
        if _CFG_VECTORIZED.search(line) and depth <= 0:
            return True
        # If we went very far up or hit a module boundary, stop
        if i < max(0, target_lineno - 200):
            break
    return False


def _exec_audit_files(repo_root: str) -> list[str]:
    """exec.rs, every .rs file under exec/ (recursively), and the #[path] children.
    Repo-relative paths with `/` separators, exec.rs first."""
    files = [_EXEC_RS]
    exec_dir = os.path.join(repo_root, _EXEC_DIR)
    for dirpath, dirnames, names in os.walk(exec_dir):
        dirnames.sort()
        for n in sorted(names):
            if n.endswith(".rs"):
                rel = os.path.relpath(os.path.join(dirpath, n), repo_root)
                files.append(rel.replace(os.sep, "/"))
    for rel in _EXEC_PATH_CHILDREN:
        if os.path.exists(os.path.join(repo_root, rel)) and rel not in files:
            files.append(rel)
    return files


def _split_leading_attrs(text: str) -> tuple[list[str], str]:
    """Split `#[..] #[..] rest` into (["#[..]", "#[..]"], "rest"). Brackets inside
    string literals are skipped. An unterminated attribute returns ([], text)."""
    attrs: list[str] = []
    rest = text.lstrip()
    while rest.startswith("#"):
        j = 1
        while j < len(rest) and rest[j].isspace():
            j += 1
        if j >= len(rest) or rest[j] != "[":
            break
        depth, k, in_str = 0, j, False
        while k < len(rest):
            ch = rest[k]
            if in_str:
                if ch == "\\":
                    k += 1
                elif ch == '"':
                    in_str = False
            elif ch == '"':
                in_str = True
            elif ch == "[":
                depth += 1
            elif ch == "]":
                depth -= 1
                if depth == 0:
                    break
            k += 1
        if depth != 0:
            return [], text
        attrs.append(rest[:k + 1])
        rest = rest[k + 1:].lstrip()
    return attrs, rest


def _parse_cfg(pred: str):
    """Parse a cfg predicate into a tree: ("all"|"any", [..]), ("not", x),
    ("kv", key, value) or ("flag", name). Returns None on a syntax error."""
    toks: list[str] = []
    pos = 0
    pred = pred.strip()
    while pos < len(pred):
        m = _CFG_TOKEN.match(pred, pos)
        if not m or m.end() == pos:
            return None
        toks.append(m.group(1) or m.group(2) or m.group(3))
        pos = m.end()
        while pos < len(pred) and pred[pos].isspace():
            pos += 1
    i = 0

    def node():
        nonlocal i
        if i >= len(toks) or not re.match(r"[A-Za-z_]", toks[i]):
            raise ValueError
        name = toks[i]
        i += 1
        if i < len(toks) and toks[i] == "(":
            if name not in ("all", "any", "not"):
                raise ValueError
            i += 1
            kids = []
            while i < len(toks) and toks[i] != ")":
                kids.append(node())
                if i < len(toks) and toks[i] == ",":
                    i += 1
                elif i < len(toks) and toks[i] != ")":
                    raise ValueError
            if i >= len(toks):
                raise ValueError
            i += 1
            if name == "not":
                if len(kids) != 1:
                    raise ValueError
                return ("not", kids[0])
            return (name, kids)
        if i < len(toks) and toks[i] == "=":
            i += 1
            if i >= len(toks) or not toks[i].startswith('"'):
                raise ValueError
            val = toks[i][1:-1]
            i += 1
            return ("kv", name, val)
        return ("flag", name)

    try:
        tree = node()
    except ValueError:
        return None
    return tree if i == len(toks) else None


def _cfg_requires_vectorized(tree) -> bool:
    """True only if every configuration satisfying `tree` has feature "vectorized".
    Conservative: anything under `not(...)` counts as not requiring it."""
    if tree is None:
        return False
    kind = tree[0]
    if kind == "kv":
        return tree[1] == "feature" and tree[2] == "vectorized"
    if kind == "all":
        return any(_cfg_requires_vectorized(t) for t in tree[1])
    if kind == "any":
        return bool(tree[1]) and all(_cfg_requires_vectorized(t) for t in tree[1])
    return False


def _attr_gates_vectorized(attr: str) -> bool:
    """True if `attr` is a `#[cfg(...)]` whose predicate requires `vectorized`."""
    m = _CFG_ATTR.match(attr.strip())
    return bool(m) and _cfg_requires_vectorized(_parse_cfg(m.group(1)))


def _child_module_decls(repo_root: str, rel: str, lines: list[str]):
    """Yield (lineno, name, child_rel_or_None, gated_on_vectorized, problem) for every
    out-of-line `mod name;` declaration in `rel`, resolved with rustc's rules:
    `#[path]` is relative to the declaring file's directory; otherwise the child is
    <dir>/<stem>/<name>.rs (or <dir>/<name>.rs from a mod.rs/lib.rs), falling back to
    .../<name>/mod.rs. Attributes may sit on preceding lines or on the same line.
    When the declaration cannot be resolved (it is indented inside an inline
    `mod {}`, it does not parse, or the file does not exist) child is None and
    `problem` says why, so the caller fails closed instead of guessing."""
    d = os.path.dirname(rel)
    stem = os.path.splitext(os.path.basename(rel))[0]
    base = d if stem in ("mod", "lib", "main") else f"{d}/{stem}"
    for lineno, line in enumerate(lines, start=1):
        code = line.split("//", 1)[0] if not line.lstrip().startswith("//") else ""
        if not _MOD_DECL_LOOSE.search(code):
            continue
        same_line_attrs, rest = _split_leading_attrs(code)
        m = _MOD_DECL.match(rest)
        if not m:
            yield lineno, "?", None, False, (
                "unparseable out-of-line `mod` declaration: " + line.strip())
            continue
        name = m.group(1)
        attrs: list[str] = list(same_line_attrs)
        problem = None
        i = lineno - 2
        while i >= 0:
            prev = lines[i].strip()
            if not prev or prev.startswith("//"):
                i -= 1
                continue
            more, tail = _split_leading_attrs(prev)
            if prev.startswith("#") and more and not tail.strip():
                attrs.extend(more)
                i -= 1
                continue
            if prev.endswith("]"):
                # The tail of an attribute that spans several lines: its content
                # (cfg, path, cfg_attr) cannot be read line by line.
                problem = ("multi-line attribute before the `mod` declaration is "
                           "unsupported: " + prev)
            break
        is_gated = any(_attr_gates_vectorized(a) for a in attrs)
        if problem is None and any(
                _CFG_ATTR_HEAD.match(a.strip())
                and re.search(r'\bpath\b', _STRING_LIT.sub('""', a)) for a in attrs):
            problem = ("`cfg_attr(..., path = ...)` on a `mod` declaration is "
                       "unsupported: leg 3 cannot tell which file rustc compiles")
        if problem is not None:
            yield lineno, name, None, is_gated, problem
            continue
        if line[:len(line) - len(line.lstrip())]:
            yield lineno, name, None, is_gated, (
                "indented out-of-line `mod` declaration (inside an inline module)")
            continue
        path_attr = next((pm.group(1) for a in attrs
                          for pm in [_PATH_ATTR.match(a.strip())] if pm), None)
        if path_attr is not None:
            child = os.path.normpath(f"{d}/{path_attr}").replace(os.sep, "/")
        else:
            child = f"{base}/{name}.rs"
            alt = f"{base}/{name}/mod.rs"
            if (not os.path.exists(os.path.join(repo_root, child))
                    and os.path.exists(os.path.join(repo_root, alt))):
                child = alt
        if not os.path.exists(os.path.join(repo_root, child)):
            yield lineno, name, None, is_gated, f"resolved file {child} does not exist"
            continue
        yield lineno, name, child, is_gated, None


def check_leg3(repo_root: str = ".") -> int:
    """
    Audit that every reference to `vectorized` constructs outside chunk.rs is
    properly gated by `#[cfg(feature = "vectorized")]`.

    Checks:
    1. In lib.rs: every `mod chunk` or `pub use chunk` line has the cfg guard
       within 3 preceding lines.
    2. In exec.rs, every .rs file under exec/ and each #[path] child in
       _EXEC_PATH_CHILDREN: every line referencing vectorized call sites (chunk::,
       DataChunk, SelVec, VecCmp, apply_filter_columnar) is inside a
       #[cfg(feature="vectorized")] block, has the guard within 3 preceding lines, or
       sits in a module whose every `mod` declaration (or a gated ancestor) is
       gated on `vectorized`.
    3. Tripwire: every `mod x;` declared by exec.rs (or by a scanned child) resolves
       to a file in that scanned set; otherwise the audit fails. A declaration
       carrying `cfg_attr(..., path = ...)` or a multi-line attribute is
       unsupported and also fails.
    """
    violations: list[str] = []
    findings: list[str] = []

    # --- lib.rs: module registration ---
    lib_rs_path = os.path.join(repo_root, _LIB_RS)
    try:
        with open(lib_rs_path) as fh:
            lib_lines = fh.readlines()
    except FileNotFoundError:
        print(f"[leg3] WARNING: {lib_rs_path!r} not found — skipping lib.rs check")
        lib_lines = []

    for lineno, line in enumerate(lib_lines, start=1):
        # Skip comment lines
        stripped = line.lstrip()
        if stripped.startswith("//"):
            continue
        if _MOD_CHUNK_PATTERN.search(line):
            if _lines_have_cfg_guard(lib_lines, lineno, window=3):
                findings.append(
                    f"[leg3] OK  {_LIB_RS}:{lineno}: 'mod/use chunk' is cfg-guarded"
                )
            else:
                violations.append(
                    f"[leg3] VIOLATION: {_LIB_RS}:{lineno}: 'mod/use chunk' reference "
                    f"lacks #[cfg(feature = \"vectorized\")] within 3 preceding lines:\n"
                    f"       {line.rstrip()}"
                )

    # --- exec.rs and every child module of `exec`: vectorized call sites ---
    audited = _exec_audit_files(repo_root)
    audited_set = set(audited)
    texts: dict[str, list[str]] = {}
    for rel in audited:
        try:
            with open(os.path.join(repo_root, rel)) as fh:
                texts[rel] = fh.readlines()
        except FileNotFoundError:
            if rel == _EXEC_RS:
                print(f"[leg3] WARNING: {rel!r} not found — skipping exec check")
            texts[rel] = []

    # Tripwire: every `mod x;` that exec.rs (or a child) declares must resolve to a
    # file this audit scans. Otherwise a vectorized reference could hide in an
    # unscanned module.
    #
    # Gating: a file counts as gated only if EVERY declaration that reaches it is
    # gated, either by its own cfg or because the declaring file is itself gated. All
    # declarations are collected first, then the gated set is the greatest fixpoint
    # of that rule (exec.rs is never gated), so two declarations of one file, one
    # gated and one not, leave it ungated, and descendants inherit the result.
    incoming: dict[str, list[tuple[str, bool, str]]] = {}
    queue = [_EXEC_RS] if texts.get(_EXEC_RS) else []
    seen: set[str] = set(queue)
    decl_count = 0
    while queue:
        parent = queue.pop(0)
        for lineno, name, child, is_gated, problem in _child_module_decls(
                repo_root, parent, texts[parent]):
            where = f"{parent}:{lineno}"
            decl_count += 1
            if child is None:
                violations.append(
                    f"[leg3] VIOLATION: {where}: cannot resolve the file of child module "
                    f"`{name}` ({problem}), so leg 3 cannot scan it")
                continue
            if child not in audited_set:
                violations.append(
                    f"[leg3] VIOLATION: {where}: child module `{name}` resolves to "
                    f"{child}, which leg 3 does not scan. Move it under {_EXEC_DIR}/ or "
                    "add it to _EXEC_PATH_CHILDREN.")
                continue
            incoming.setdefault(child, []).append((parent, is_gated, where))
            if child not in seen:
                seen.add(child)
                queue.append(child)

    gated_set = {f for f in incoming if f != _EXEC_RS}
    changed = True
    while changed:
        changed = False
        for f in list(gated_set):
            if not all(g or p in gated_set for p, g, _ in incoming[f]):
                gated_set.discard(f)
                changed = True
    gated: dict[str, str] = {
        f: ", ".join(w for _, _, w in incoming[f]) for f in sorted(gated_set)}

    for rel in audited:
        lines = texts[rel]
        for lineno, line in enumerate(lines, start=1):
            stripped = line.lstrip()
            # Skip comment lines (single-line)
            if stripped.startswith("//"):
                continue
            # Skip the cfg attribute line itself
            if _CFG_VECTORIZED.search(line):
                continue
            if _CHUNK_IMPORT_PATTERN.search(line):
                # Accept if guarded by nearby preceding cfg, inside a cfg block, or the
                # whole file is a module gated on `vectorized`.
                if rel in gated:
                    findings.append(
                        f"[leg3] OK  {rel}:{lineno}: vectorized reference is in a module "
                        f"gated at {gated[rel]}"
                    )
                elif (_lines_have_cfg_guard(lines, lineno, window=3) or
                        _is_inside_cfg_vectorized_block(lines, lineno)):
                    findings.append(
                        f"[leg3] OK  {rel}:{lineno}: vectorized reference is cfg-guarded"
                    )
                else:
                    violations.append(
                        f"[leg3] VIOLATION: {rel}:{lineno}: vectorized reference "
                        f"lacks #[cfg(feature = \"vectorized\")] guard:\n"
                        f"       {line.rstrip()}"
                    )

    print(f"[leg3] scanned {len(audited)} exec file(s): {_EXEC_RS}, "
          f"{len(audited) - 1 - len([p for p in audited if p in _EXEC_PATH_CHILDREN])} "
          f"under {_EXEC_DIR}/, and {', '.join(p for p in audited if p in _EXEC_PATH_CHILDREN) or 'no'} "
          "#[path] child(ren)")
    print(f"[leg3] checked {decl_count} child module declaration(s) against that set")

    for f in findings:
        print(f)

    if violations:
        for v in violations:
            print(v)
        print(f"\n[leg3] FAIL — {len(violations)} violation(s): ungated vectorized "
              "reference(s) or child module(s) leg 3 does not scan. "
              "Every call site and registration of the `vectorized` feature must be "
              "inside #[cfg(feature = \"vectorized\")].")
        return 1

    guarded_count = len(findings)
    print(f"\n[leg3] OK — all {guarded_count} vectorized reference(s) are properly "
          "cfg-gated. Structural absence confirmed.")
    return 0


# ---------------------------------------------------------------------------
# Self-test: tripwires that MUST fire (the guards must be able to fail)
# ---------------------------------------------------------------------------

def _leg1_on_dict(meta: dict) -> int:
    """Run leg1 check on an in-memory metadata dict via a temp file."""
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as tmp:
        json.dump(meta, tmp)
        tmp_path = tmp.name
    try:
        return check_leg1(tmp_path)
    finally:
        os.unlink(tmp_path)


def _leg2_dynamic_on_bytes(base_bytes: bytes, head_bytes: bytes,
                           base_tok: int | None, head_tok: int | None,
                           base_decl_names: list[str] | None = None,
                           head_decl_names: list[str] | None = None) -> int:
    """Run the dynamic leg2 check on in-memory wasm bytes + declarations.

    A token of None writes an EMPTY declaration file (missing field) to exercise the
    fail-safe default (reads as 0). Otherwise writes {"change_token": <int>}.

    base_decl_names / head_decl_names exercise MECHANISM V2: each is a list of file names
    materialised inside a temp declarations directory (e.g. ["1720.json", "README.md"]).
    None => no directory passed (V2 disabled for that side).
    """
    paths: list[str] = []
    dirs: list[str] = []

    def _write(data: bytes | str, suffix: str) -> str:
        mode = "wb" if isinstance(data, bytes) else "w"
        with tempfile.NamedTemporaryFile(mode=mode, suffix=suffix, delete=False) as tmp:
            tmp.write(data)
            paths.append(tmp.name)
            return tmp.name

    def _mkdir(names: list[str] | None) -> str | None:
        if names is None:
            return None
        d = tempfile.mkdtemp()
        dirs.append(d)
        for n in names:
            with open(os.path.join(d, n), "w") as fh:
                fh.write("{}")
        return d

    base_wasm = _write(base_bytes, ".wasm")
    head_wasm = _write(head_bytes, ".wasm")
    base_decl = _write("{}" if base_tok is None else json.dumps({"change_token": base_tok}), ".json")
    head_decl = _write("{}" if head_tok is None else json.dumps({"change_token": head_tok}), ".json")
    base_dir = _mkdir(base_decl_names)
    head_dir = _mkdir(head_decl_names)
    try:
        return check_leg2_dynamic(base_wasm, head_wasm, base_decl, head_decl,
                                  base_dir, head_dir)
    finally:
        for p in paths:
            os.unlink(p)
        for d in dirs:
            for n in os.listdir(d):
                os.unlink(os.path.join(d, n))
            os.rmdir(d)


def _leg3_on_tree(files: dict[str, str]) -> int:
    """Run check_leg3 on a synthetic repo tree of {repo-relative path: content}."""
    root = tempfile.mkdtemp(prefix="leg3-")
    try:
        for rel, content in files.items():
            full = os.path.join(root, rel)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w") as fh:
                fh.write(content)
        return check_leg3(repo_root=root)
    finally:
        for dirpath, dirnames, names in os.walk(root, topdown=False):
            for n in names:
                os.unlink(os.path.join(dirpath, n))
            os.rmdir(dirpath)


# Synthetic leg-3 trees for the tripwires (also used by scripts/tests).
_LEG3_TREE_OK = {
    _EXEC_RS: "mod child;\n#[cfg(feature = \"vectorized\")]\nmod vec_only;\n",
    f"{_EXEC_DIR}/child.rs": "#[cfg(feature = \"vectorized\")]\nuse crate::chunk::DataChunk;\n",
    f"{_EXEC_DIR}/vec_only.rs": "use crate::chunk::DataChunk;\n",
}
_LEG3_TREE_UNGATED_CHILD = {
    _EXEC_RS: "mod child;\n",
    f"{_EXEC_DIR}/child.rs": "fn f(_c: &crate::chunk::DataChunk) {}\n",
}
_LEG3_TREE_UNSCANNED_CHILD = {
    _EXEC_RS: "#[path = \"elsewhere.rs\"]\nmod hidden;\n",
    "crates/sparq-engine/src/elsewhere.rs": "fn f(_c: &crate::chunk::DataChunk) {}\n",
}
_LEG3_UNGATED = "fn f(_c: &crate::chunk::DataChunk) {}\n"


def _leg3_tree_outside(decl: str) -> dict[str, str]:
    """exec.rs holds `decl`; an ungated reference sits in src/elsewhere.rs."""
    return {_EXEC_RS: decl, "crates/sparq-engine/src/elsewhere.rs": _LEG3_UNGATED}


def _leg3_tree_child(decl: str) -> dict[str, str]:
    """exec.rs holds `decl`; an ungated reference sits in exec/child.rs."""
    return {_EXEC_RS: decl, f"{_EXEC_DIR}/child.rs": _LEG3_UNGATED}


# `#[path]` on the SAME line as `mod`, in several spellings: each must be resolved
# (and so rejected as an unscanned child), not skipped.
_LEG3_TREES_SAME_LINE_ATTR = [
    _leg3_tree_outside('#[path = "elsewhere.rs"] mod hidden;\n'),
    _leg3_tree_outside('#[path="elsewhere.rs"]mod hidden;\n'),
    _leg3_tree_outside('#[cfg(test)]  #[path = "elsewhere.rs"]   pub(crate) mod hidden ;\n'),
    _leg3_tree_outside('#[allow(dead_code)] # [ path = "elsewhere.rs" ] pub mod hidden;\n'),
    _leg3_tree_outside('#[cfg(test)]\n#[allow(unused)] #[path = "elsewhere.rs"] pub(super) mod hidden;\n'),
]
# cfg predicates that do NOT require `vectorized`: the module is not gated, so its
# ungated reference must be rejected.
_LEG3_TREES_NON_GATING_CFG = [
    _leg3_tree_child('#[cfg(all(not (feature = "vectorized")))]\nmod child;\n'),
    _leg3_tree_child('#[cfg(all(not(feature="vectorized")))] mod child;\n'),
    _leg3_tree_child('#[cfg( not( feature = "vectorized" ) )]\nmod child;\n'),
    _leg3_tree_child('#[cfg(any(feature = "vectorized", test))]\nmod child;\n'),
    _leg3_tree_child('#[cfg(all(test, not(all(feature = "vectorized"))))]\nmod child;\n'),
    _leg3_tree_child('#[cfg(feature = "vectorized-lite")]\nmod child;\n'),
]
# A `mod x;` whose file cannot be resolved must fail closed.
_LEG3_TREE_UNRESOLVABLE = {_EXEC_RS: "mod nothere;\n"}


def _leg3_tree_cfg_attr(decl: str) -> dict[str, str]:
    """exec.rs holds `decl`; the default file exec/child.rs is clean and the
    cfg_attr target src/elsewhere.rs holds an ungated reference."""
    return {_EXEC_RS: decl, f"{_EXEC_DIR}/child.rs": "fn ok() {}\n",
            "crates/sparq-engine/src/elsewhere.rs": _LEG3_UNGATED}


# `cfg_attr(..., path = ...)` picks the module file per configuration; leg 3 must
# fail on it as unsupported instead of auditing the default path.
_LEG3_TREES_CFG_ATTR_PATH = [
    _leg3_tree_cfg_attr('#[cfg_attr(feature = "vectorized", path = "elsewhere.rs")]\nmod child;\n'),
    _leg3_tree_cfg_attr('#[cfg_attr(test, path="elsewhere.rs")] pub(crate) mod child;\n'),
    _leg3_tree_cfg_attr('# [ cfg_attr ( all(test), path = "elsewhere.rs" ) ]\n\nmod child;\n'),
    _leg3_tree_cfg_attr('#[cfg_attr(test, cfg_attr(unix, path = "elsewhere.rs"))]\nmod child;\n'),
    _leg3_tree_cfg_attr('#[cfg_attr(\n    test,\n    path = "elsewhere.rs"\n)]\nmod child;\n'),
]
# cfg_attr without `path` (a `path` inside a string does not count) is fine.
_LEG3_TREES_CFG_ATTR_OK = [
    _leg3_tree_cfg_attr('#[cfg_attr(test, allow(dead_code))]\nmod child;\n'),
    _leg3_tree_cfg_attr('#[cfg_attr(test, doc = "see path = x")] mod child;\n'),
]

_LEG3_GATED_DECL = '#[cfg(feature = "vectorized")]\n#[path = "exec/child.rs"]\nmod vec_child;\n'
_LEG3_PLAIN_DECL = '#[path = "exec/child.rs"]\nmod plain_child;\n'
# One file reached by a gated AND an ungated declaration is NOT gated, in either
# order, and neither is anything it declares.
_LEG3_TREES_MIXED_DECLS = [
    {_EXEC_RS: _LEG3_GATED_DECL + _LEG3_PLAIN_DECL, f"{_EXEC_DIR}/child.rs": _LEG3_UNGATED},
    {_EXEC_RS: _LEG3_PLAIN_DECL + _LEG3_GATED_DECL, f"{_EXEC_DIR}/child.rs": _LEG3_UNGATED},
    {_EXEC_RS: _LEG3_GATED_DECL + _LEG3_PLAIN_DECL, f"{_EXEC_DIR}/child.rs": "mod grand;\n",
     f"{_EXEC_DIR}/child/grand.rs": _LEG3_UNGATED},
]
# Every declaration gated (directly or through a gated parent): accepted.
_LEG3_TREES_ALL_DECLS_GATED = [
    {_EXEC_RS: _LEG3_GATED_DECL + '#[cfg(all(test, feature = "vectorized"))]\n'
     '#[path = "exec/child.rs"]\nmod again;\n', f"{_EXEC_DIR}/child.rs": _LEG3_UNGATED},
    {_EXEC_RS: _LEG3_GATED_DECL, f"{_EXEC_DIR}/child.rs": "mod grand;\n",
     f"{_EXEC_DIR}/child/grand.rs": _LEG3_UNGATED},
]
# cfg predicates that DO require `vectorized`, in several spellings: accepted.
_LEG3_TREES_GATING_CFG = [
    _leg3_tree_child('#[cfg(all(test, feature = "vectorized"))]\nmod child;\n'),
    _leg3_tree_child('#[cfg( feature  =  "vectorized" )] pub(crate) mod child;\n'),
    _leg3_tree_child('#[cfg(any(feature="vectorized", all(feature="vectorized", test)))]\n'
                     '#[allow(dead_code)]\nmod child;\n'),
]


def run_self_test() -> int:
    """
    Run built-in tripwires. Both MUST detect violations (exit non-zero).
    Returns 0 only if both tripwires correctly rejected their bad input.
    """
    all_passed = True

    # ----- Tripwire 1: Leg 1 must reject a metadata fixture with vectorized enabled -----
    bad_meta = {
        "packages": [
            {
                "name": "sparq-engine",
                "features": {
                    "default": [],
                    "vectorized": []
                }
            }
        ],
        "resolve": {
            "nodes": [
                {
                    "id": "sparq-engine 0.1.0 (path+file:///repo/crates/sparq-engine)",
                    "features": ["vectorized"]
                }
            ]
        }
    }
    rc = _leg1_on_dict(bad_meta)
    if rc != 0:
        print("TRIPWIRE 1 (leg1): PASS — guard correctly detected 'vectorized' in synthetic fixture")
    else:
        print("TRIPWIRE 1 (leg1): FAIL — guard did NOT detect 'vectorized'; the leg1 check is broken")
        all_passed = False

    # ----- Tripwire 2: Leg 2 (dynamic) must reject an UNDECLARED byte change -----
    # [OPUS-4.8] sq-v3nel: the dynamic check compares base-tree vs head-tree feature-OFF
    # wasm bytes in the SAME run. Differing bytes with an un-bumped change_token MUST fail.
    rc = _leg2_dynamic_on_bytes(b"\x00wasm-base", b"\x00wasm-headX", base_tok=0, head_tok=0)
    if rc != 0:
        print("TRIPWIRE 2 (leg2): PASS — dynamic check correctly rejected an UNDECLARED "
              "feature-OFF byte change (bytes differ, change_token un-bumped)")
    else:
        print("TRIPWIRE 2 (leg2): FAIL — dynamic check did NOT reject an undeclared change; "
              "the leg2 check is broken")
        all_passed = False

    # ----- Tripwire 3: Leg 2 (dynamic) must ACCEPT a declared byte change (guard sanity) -----
    rc = _leg2_dynamic_on_bytes(b"\x00wasm-base", b"\x00wasm-headX", base_tok=0, head_tok=1)
    if rc == 0:
        print("TRIPWIRE 3 (leg2): PASS — dynamic check accepted a DECLARED change "
              "(bytes differ, change_token 0 -> 1)")
    else:
        print("TRIPWIRE 3 (leg2): FAIL — dynamic check rejected a declared change; false positive")
        all_passed = False

    # ----- Tripwire 4: Leg 2 V2 must ACCEPT a per-PR declaration FILE added by head -----
    # [OPUS-4.8] sq-v3nel-v2: scalar token unchanged, but head added 1720.json not in base.
    rc = _leg2_dynamic_on_bytes(b"\x00wasm-base", b"\x00wasm-headX", base_tok=0, head_tok=0,
                                base_decl_names=["README.md"],
                                head_decl_names=["README.md", "1720.json"])
    if rc == 0:
        print("TRIPWIRE 4 (leg2 V2): PASS — dynamic check accepted a DECLARED change via a "
              "new per-PR file (bench/feature-off-declarations/1720.json added by head)")
    else:
        print("TRIPWIRE 4 (leg2 V2): FAIL — V2 file declaration rejected; false positive")
        all_passed = False

    # ----- Tripwire 5: Leg 2 V2 must REJECT when NO new file is added (README-only) -----
    # A README.md present on both sides is NOT a declaration; scalar token also unchanged.
    rc = _leg2_dynamic_on_bytes(b"\x00wasm-base", b"\x00wasm-headX", base_tok=0, head_tok=0,
                                base_decl_names=["README.md"],
                                head_decl_names=["README.md"])
    if rc != 0:
        print("TRIPWIRE 5 (leg2 V2): PASS — no new per-PR file (only README on both sides) "
              "correctly rejected as UNDECLARED")
    else:
        print("TRIPWIRE 5 (leg2 V2): FAIL — undeclared change slipped through V2; the gate is disabled")
        all_passed = False

    # ----- Tripwire 6: Leg 3 must reject an ungated reference in an exec child module -----
    rc = _leg3_on_tree(_LEG3_TREE_UNGATED_CHILD)
    if rc != 0:
        print("TRIPWIRE 6 (leg3): PASS — ungated vectorized reference in exec/child.rs rejected")
    else:
        print("TRIPWIRE 6 (leg3): FAIL — exec child modules are not audited")
        all_passed = False

    # ----- Tripwire 7: Leg 3 must reject a child module it does not scan -----
    rc = _leg3_on_tree(_LEG3_TREE_UNSCANNED_CHILD)
    if rc != 0:
        print("TRIPWIRE 7 (leg3): PASS — exec.rs child module outside the scanned set rejected")
    else:
        print("TRIPWIRE 7 (leg3): FAIL — an unscanned exec child module slipped through")
        all_passed = False

    # ----- Tripwire 8: Leg 3 must ACCEPT gated references and module-level gates -----
    rc = _leg3_on_tree(_LEG3_TREE_OK)
    if rc == 0:
        print("TRIPWIRE 8 (leg3): PASS — item-level and module-level gates accepted")
    else:
        print("TRIPWIRE 8 (leg3): FAIL — gated references rejected; false positive")
        all_passed = False

    # ----- Tripwire 9: Leg 3 must resolve `#[path]` written on the same line as `mod` -----
    missed = [i for i, t in enumerate(_LEG3_TREES_SAME_LINE_ATTR) if _leg3_on_tree(t) == 0]
    if not missed:
        print("TRIPWIRE 9 (leg3): PASS — same-line #[path] declarations resolved and rejected "
              f"({len(_LEG3_TREES_SAME_LINE_ATTR)} spellings)")
    else:
        print(f"TRIPWIRE 9 (leg3): FAIL — same-line #[path] declaration(s) {missed} skipped")
        all_passed = False

    # ----- Tripwire 10: a cfg that does not require `vectorized` must not gate a module -----
    missed = [i for i, t in enumerate(_LEG3_TREES_NON_GATING_CFG) if _leg3_on_tree(t) == 0]
    if not missed:
        print("TRIPWIRE 10 (leg3): PASS — not(...)/any(...)/other-feature cfgs do not gate "
              f"({len(_LEG3_TREES_NON_GATING_CFG)} spellings)")
    else:
        print(f"TRIPWIRE 10 (leg3): FAIL — non-gating cfg(s) {missed} accepted as gates")
        all_passed = False

    # ----- Tripwire 11: an unresolvable `mod x;` must fail closed -----
    rc = _leg3_on_tree(_LEG3_TREE_UNRESOLVABLE)
    if rc != 0:
        print("TRIPWIRE 11 (leg3): PASS — unresolvable child module rejected")
    else:
        print("TRIPWIRE 11 (leg3): FAIL — unresolvable child module accepted")
        all_passed = False

    # ----- Tripwire 12: cfgs that require `vectorized` are accepted in any spelling -----
    wrong = [i for i, t in enumerate(_LEG3_TREES_GATING_CFG) if _leg3_on_tree(t) != 0]
    if not wrong:
        print("TRIPWIRE 12 (leg3): PASS — gating cfgs accepted "
              f"({len(_LEG3_TREES_GATING_CFG)} spellings)")
    else:
        print(f"TRIPWIRE 12 (leg3): FAIL — gating cfg(s) {wrong} rejected; false positive")
        all_passed = False

    # ----- Tripwire 13: cfg_attr(..., path = ...) on a mod declaration is unsupported -----
    missed = [i for i, t in enumerate(_LEG3_TREES_CFG_ATTR_PATH) if _leg3_on_tree(t) == 0]
    wrong = [i for i, t in enumerate(_LEG3_TREES_CFG_ATTR_OK) if _leg3_on_tree(t) != 0]
    if not missed and not wrong:
        print("TRIPWIRE 13 (leg3): PASS — cfg_attr(..., path) rejected as unsupported "
              f"({len(_LEG3_TREES_CFG_ATTR_PATH)} spellings); path-free cfg_attr accepted")
    else:
        print(f"TRIPWIRE 13 (leg3): FAIL — cfg_attr path missed {missed}, "
              f"path-free cfg_attr rejected {wrong}")
        all_passed = False

    # ----- Tripwire 14: a file is gated only if EVERY declaration reaching it is -----
    missed = [i for i, t in enumerate(_LEG3_TREES_MIXED_DECLS) if _leg3_on_tree(t) == 0]
    wrong = [i for i, t in enumerate(_LEG3_TREES_ALL_DECLS_GATED) if _leg3_on_tree(t) != 0]
    if not missed and not wrong:
        print("TRIPWIRE 14 (leg3): PASS — gated+ungated declarations of one file leave it "
              "(and its descendants) ungated; all-gated declarations accepted")
    else:
        print(f"TRIPWIRE 14 (leg3): FAIL — mixed declarations exempted {missed}, "
              f"all-gated declarations rejected {wrong}")
        all_passed = False

    if all_passed:
        print("\n[self-test] OK — all tripwires fired correctly. The guards can fail (and "
              "the dynamic leg2 accepts a declared change).")
        return 0
    else:
        print("\n[self-test] FAIL — one or more tripwires did not fire. Fix the guard logic.")
        return 1


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="sq-pntvh.8 feature-OFF CI proof: three-leg check + tripwires"
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--leg1", metavar="METADATA_JSON",
                      help="cargo metadata JSON to check for vectorized absence")
    mode.add_argument("--leg2-dynamic", dest="leg2_dynamic", nargs=4,
                      metavar=("BASE_WASM", "HEAD_WASM", "BASE_DECL", "HEAD_DECL"),
                      help="DYNAMIC byte-identity: base-tree vs head-tree feature-OFF wasm "
                           "+ base/head feature-off-declaration.json (legacy scalar). Add "
                           "--declarations-dirs for mechanism V2 (per-PR files).")
    mode.add_argument("--leg3", action="store_true",
                      help="cfg-audit of vectorized call sites in lib.rs, exec.rs and "
                           "exec's child modules")
    mode.add_argument("--self-test", dest="self_test", action="store_true",
                      help="run built-in tripwires (both must exit non-zero to pass)")
    parser.add_argument("--declarations-dirs", dest="declarations_dirs", nargs=2,
                        metavar=("BASE_DIR", "HEAD_DIR"),
                        help="MECHANISM V2 (sq-v3nel-v2): base-tree and head-tree "
                             "bench/feature-off-declarations/ directories. A PR declares by "
                             "adding its own <PR-number>.json file (set difference). Combined "
                             "with --leg2-dynamic; either mechanism satisfies the gate.")
    parser.add_argument("--repo-root", default=".",
                        help="repo root for --leg3 (default: cwd)")
    args = parser.parse_args()

    if args.leg1:
        return check_leg1(args.leg1)
    elif args.leg2_dynamic:
        base_dir = args.declarations_dirs[0] if args.declarations_dirs else None
        head_dir = args.declarations_dirs[1] if args.declarations_dirs else None
        return check_leg2_dynamic(args.leg2_dynamic[0], args.leg2_dynamic[1],
                                  args.leg2_dynamic[2], args.leg2_dynamic[3],
                                  base_dir, head_dir)
    elif args.leg3:
        return check_leg3(repo_root=args.repo_root)
    elif args.self_test:
        return run_self_test()
    else:
        parser.print_help()
        return 2


if __name__ == "__main__":
    sys.exit(main())
