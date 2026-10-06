#!/usr/bin/env python3
# Out-of-crate-input audit for change-based CI test-selection.
# Bead sq-fmx4u.2 (epic sq-fmx4u). Design: research/change-based-test-selection.md
# §4.2 (the ownership map + its audit).
#
# WHY THIS EXISTS (the soundness hole it closes):
#   The selector (scripts/ci_select.py) may SKIP a crate C's tests when no path
#   in C's dependency closure changed. That is UNSOUND if C reads an input that
#   lives OUTSIDE its own crate dir — a fetched W3C corpus, a bench fixture, a
#   sibling crate's data file — because a change to that input would not be
#   attributed to C and C would be skipped even though its test outcome depends
#   on the input. This audit greps every crate's src/tests/benches/build.rs for
#   such out-of-crate reads and PROVES each one is covered:
#     * a repo-level input  => covered by a §4.1 full-run trigger, or by a
#       ci/path-ownership.toml `[[map]]` entry whose `crates` list names C;
#     * a sibling-crate input (path under crates/<D>/, D != C) => covered iff C
#       is in the reverse-dependency closure of D (a change to D's dir already
#       runs C's tests).
#   Anything uncovered is a FINDING and the audit exits non-zero (fail-closed):
#   either the map must gain an entry, or the read is a genuine residual that
#   must be explicitly acknowledged below with a justification + a fix bead.
#
# It reuses ci_select.py (never edits it) for the map loader, the trigger table,
# the crate-prefix owner match, the metadata parse and the reverse closure.
#
# Usage:
#   ci_audit_inputs.py                          # real: cargo metadata + repo scan
#   ci_audit_inputs.py --metadata-file m.json   # hermetic metadata snapshot
#   ci_audit_inputs.py --repo-root DIR --map DIR/ci/path-ownership.toml
# Exit 0 == every out-of-crate input is covered; exit 1 == findings (printed);
# exit 2 == cargo metadata could not be loaded (fail-closed error, printed to stderr).
#
# Stdlib only (matches ci_select.py + the CI setup-python 3.12).

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import PurePosixPath

_HERE = os.path.dirname(os.path.abspath(__file__))


def _load_ci_select():
    """Import scripts/ci_select.py as a module WITHOUT editing it (DRY reuse)."""
    path = os.path.join(_HERE, "ci_select.py")
    spec = importlib.util.spec_from_file_location("ci_select", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules.setdefault("ci_select", mod)
    spec.loader.exec_module(mod)
    return mod


cs = _load_ci_select()

# The crate sub-trees the audit greps (design §4.2: "src/tests/benches/build.rs").
# `examples/` is deliberately EXCLUDED: `cargo test -p C` compiles examples but
# never runs an example `main()`, so an example's runtime file read is not a
# test input. (An example carrying its own `#[test]` would live under tests/ by
# convention.)
_SCAN_SUBDIRS = ("src", "tests", "benches")
_SCAN_FILES = ("build.rs",)

# include_str!/include_bytes! — target is relative to the SOURCE FILE's dir.
_INCLUDE_RE = re.compile(r"include_(?:str|bytes)!\(\s*\"([^\"]+)\"")
# env!("CARGO_MANIFEST_DIR") — target segments follow, relative to the CRATE dir.
_MANIFEST_RE = re.compile(r"env!\(\s*\"CARGO_MANIFEST_DIR\"\s*\)")
_STR_RE = re.compile(r"\"([^\"\n]+)\"")
# A `.join(` whose next non-space char is not a quote == a runtime (non-literal)
# path segment: the statically-resolved path is only a PREFIX of the real read.
_DYNAMIC_JOIN_RE = re.compile(r"\.join\(\s*[^\"\s)]")


@dataclass
class Ref:
    """One out-of-crate input reference discovered in a crate's sources."""

    crate: str  # the reading crate (workspace-member name)
    src: str  # repo-relative source file the reference lives in
    resolved: str  # repo-relative POSIX target ('.' == the workspace root)
    kind: str  # "include" | "manifest_join"
    dynamic: bool  # True => `resolved` is a prefix (a non-literal join followed)


@dataclass(frozen=True)
class Residual:
    """A KNOWN out-of-crate read the ownership map CANNOT cover.

    These are sibling-crate reads where the reader is not in the owner's
    reverse-dependency closure (so no `[[map]]` entry can attribute them — the
    selector resolves an in-crate path by prefix-ownership before it ever
    consults the map), or a statically-unresolvable runtime path. Each is
    backstopped by the nightly FULL run (design §6.1) and carries a fix bead.
    A residual that matches NO discovered read is flagged STALE (keeps this list
    honest); a NEW uncovered read that is not listed here FAILS the audit.
    """

    crate: str
    target: str  # repo-relative prefix ('.' matches the workspace-root dynamic read)
    reason: str


# --- acknowledged residuals (audit-proven; each has a fix bead) --------------
# EMPTY — every out-of-crate input in the workspace is now COVERED (a full-run
# trigger, an ownership-map attribution, a reverse-dependency closure, or an
# additional-readers union). Keep it that way: a NEW uncovered read must be fixed
# or attributed, not acknowledged, unless neither is genuinely available. If you do
# add an entry, give it a justification AND a fix bead — and note that a residual
# matching no discovered read is reported STALE, so a fixed one cannot linger.
#
# History (both closed by input-relocation-shaped fixes, not by weakening a gate):
# * sq-m4bxc closed the two sibling-crate include residuals (sparq-zk ->
#   secprop-ext.ttl, sparq-reason -> sparq-solid/rules) with the ci_select
#   additional-readers mechanism (a `readers` entry in ci/path-ownership.toml
#   unions the extra reader into the affected set — monotone, fail-safe). They are
#   now COVERED "additional-readers map".
# * sq-z1xv8 closed residual 3: sparq-conformance's
#   tests/scoreboard_floors.rs read SIBLING-CRATE TEST SOURCES at a
#   runtime-computed workspace path (statically unresolvable, so it collapsed to
#   '.' and no `readers` entry could attribute it). The eleven affected floors moved
#   into the zero-dependency `sparq-conformance-floors` crate, imported by BOTH the
#   enforcing runner and sparq-conformance's scoreboard registry, so the guard reads
#   no foreign source at all and the cargo edges put every enforcing crate in the
#   floors crate's reverse-dependency closure. The surviving textual reads are
#   crate-local (rooted at CARGO_MANIFEST_DIR), pinned by that guard's
#   `textual_guard_reads_only_crate_local_sources` test.
KNOWN_RESIDUALS: tuple[Residual, ...] = ()


# --- discovery ---------------------------------------------------------------
def enumerate_crates(repo_root: str) -> dict[str, str]:
    """Map crates/<dir> (repo-relative) -> crate NAME parsed from its Cargo.toml.

    Name is read from the manifest (not assumed == dir) so the audit stays
    correct if a crate's package name ever diverges from its directory.
    """
    crates_dir = os.path.join(repo_root, "crates")
    out: dict[str, str] = {}
    if not os.path.isdir(crates_dir):
        return out
    for entry in sorted(os.listdir(crates_dir)):
        manifest = os.path.join(crates_dir, entry, "Cargo.toml")
        if not os.path.isfile(manifest):
            continue
        name = entry
        with open(manifest, encoding="utf-8") as fh:
            for line in fh:
                m = re.match(r"\s*name\s*=\s*\"([^\"]+)\"", line)
                if m:
                    name = m.group(1)
                    break
        out[f"crates/{entry}"] = name
    return out


def fetched_roots(repo_root: str) -> set[str]:
    """Roots that a clean clone does NOT have on disk but a fetch script creates.

    Used by the map-validity check (#1392 refinement): a `tests/w3c/**` or
    `tests/odrl-test-suite/**` pattern must not falsely fail "pattern root does
    not exist" on a clean clone. The allowlist is DERIVED from the destination
    dir literals in scripts/fetch-*.sh (self-maintaining — a new fetched suite
    is picked up automatically), not hard-coded.
    """
    roots: set[str] = set()
    scripts_dir = os.path.join(repo_root, "scripts")
    if not os.path.isdir(scripts_dir):
        return roots
    dest_re = re.compile(r"(?:tests|bench)/[A-Za-z0-9._-]+(?:/[A-Za-z0-9._-]+)*")
    for fn in sorted(os.listdir(scripts_dir)):
        if not (fn.startswith("fetch-") and fn.endswith(".sh")):
            continue
        try:
            with open(os.path.join(scripts_dir, fn), encoding="utf-8", errors="replace") as fh:
                text = fh.read()
        except OSError:
            continue
        for m in dest_re.finditer(text):
            parts = m.group(0).split("/")
            roots.add("/".join(parts[:2]))
    return roots


def _crate_source_files(repo_root: str, crate_reldir: str) -> list[str]:
    base = os.path.join(repo_root, crate_reldir)
    files: list[str] = []
    for sub in _SCAN_SUBDIRS:
        for dirpath, _dirs, names in os.walk(os.path.join(base, sub)):
            files += [os.path.join(dirpath, n) for n in names if n.endswith(".rs")]
    for extra in _SCAN_FILES:
        p = os.path.join(base, extra)
        if os.path.isfile(p):
            files.append(p)
    return files


def _rel(repo_root: str, abspath: str) -> str:
    return PurePosixPath(os.path.relpath(abspath, repo_root)).as_posix()


def _pathlike(seg: str) -> bool:
    seg = seg.strip()
    if not seg or " " in seg:
        return False
    return "/" in seg or seg.startswith("..") or re.search(r"\.[A-Za-z0-9]{1,6}$", seg) is not None


def scan_refs(repo_root: str, crate_roots: dict[str, str]) -> list[Ref]:
    """Discover every out-of-crate input reference in every crate.

    Returns only references that ESCAPE the reading crate's own directory
    (in-crate `../README.md`-style includes are not test-selection inputs).
    Deduplicated per (crate, resolved).
    """
    refs: list[Ref] = []
    seen: set[tuple[str, str]] = set()

    def add(crate: str, src_abs: str, resolved_abs: str, kind: str, dynamic: bool) -> None:
        resolved = _rel(repo_root, resolved_abs)
        crate_dir = None
        for reldir, name in crate_roots.items():
            if name == crate:
                crate_dir = reldir
                break
        # Skip references that stay inside the reading crate's own dir.
        if crate_dir is not None and (resolved == crate_dir or resolved.startswith(crate_dir + "/")):
            return
        key = (crate, resolved)
        if key in seen:
            return
        seen.add(key)
        refs.append(Ref(crate=crate, src=_rel(repo_root, src_abs), resolved=resolved, kind=kind, dynamic=dynamic))

    for crate_reldir, crate_name in crate_roots.items():
        crate_dir_abs = os.path.join(repo_root, crate_reldir)
        for src_abs in _crate_source_files(repo_root, crate_reldir):
            try:
                with open(src_abs, encoding="utf-8", errors="replace") as fh:
                    text = fh.read()
            except OSError:
                continue
            src_dir = os.path.dirname(src_abs)
            for m in _INCLUDE_RE.finditer(text):
                resolved_abs = os.path.normpath(os.path.join(src_dir, m.group(1)))
                add(crate_name, src_abs, resolved_abs, "include", dynamic=False)
            for m in _MANIFEST_RE.finditer(text):
                window = text[m.end():m.end() + 240]
                window = window.split(";", 1)[0]
                segs = [s for s in _STR_RE.findall(window) if _pathlike(s)]
                if not segs:
                    continue
                joined = crate_dir_abs
                for seg in segs:
                    joined = os.path.join(joined, seg.lstrip("/"))
                resolved_abs = os.path.normpath(joined)
                dynamic = bool(_DYNAMIC_JOIN_RE.search(window))
                add(crate_name, src_abs, resolved_abs, "manifest_join", dynamic)
    return refs


# --- coverage classification -------------------------------------------------
@dataclass
class AuditResult:
    ok: bool
    findings: list[str] = field(default_factory=list)
    covered: list[tuple[str, str, str]] = field(default_factory=list)  # (crate, resolved, how)
    acknowledged: list[tuple[str, str]] = field(default_factory=list)  # (crate, resolved)


def _ack_for(ref: Ref, acks: tuple[Residual, ...]) -> Residual | None:
    for r in acks:
        if r.crate != ref.crate:
            continue
        if r.target == ".":
            if ref.resolved == ".":
                return r
        elif ref.resolved == r.target or ref.resolved.startswith(r.target + "/"):
            return r
    return None


def audit_refs(
    refs: list[Ref],
    crate_roots: dict[str, str],
    map_entries: list[dict],
    closures: dict[str, set[str]],
    acks: tuple[Residual, ...] = KNOWN_RESIDUALS,
) -> AuditResult:
    """Pure core: classify each discovered ref; findings => not ok.

    `closures[D]` is D's reverse-dependency closure (from ci_select); a missing
    key defaults to {D} (conservative: only D itself covers a D-owned path).
    """
    owners = sorted(((d, n) for d, n in crate_roots.items()), key=lambda p: len(p[0]), reverse=True)
    res = AuditResult(ok=True)
    used_acks: set[Residual] = set()

    for ref in refs:
        owner = cs.owning_package(ref.resolved, owners) if ref.resolved != "." else None

        if owner == ref.crate:
            res.covered.append((ref.crate, ref.resolved, "in-crate"))
            continue

        if owner is None:
            # Repo-level input: trigger, then map, then ack, then FAIL.
            trig = cs._trigger_match(ref.resolved)
            if trig is not None:
                res.covered.append((ref.crate, ref.resolved, "full-run trigger"))
                continue
            verdict = None
            try:
                verdict = cs.apply_ownership_map(ref.resolved, map_entries)
            except cs.SelectorError as exc:  # malformed entry => surface as a finding
                res.findings.append(f"{ref.crate}: malformed map entry for {ref.resolved}: {exc}")
                continue
            if verdict is not None:
                kind, crates = verdict
                if kind == "safe":
                    res.findings.append(
                        f"{ref.crate} reads {ref.resolved} (from {ref.src}) but it is SAFE-listed "
                        f"(proven-inert) — a SAFE path must not be read by any crate; remove the "
                        f"safe entry or attribute the path"
                    )
                    continue
                if ref.crate in crates:
                    res.covered.append((ref.crate, ref.resolved, "ownership map"))
                    continue
                # Mapped, but this reader is not listed -> silent unsound skip.
                ack = _ack_for(ref, acks)
                if ack is not None:
                    used_acks.add(ack)
                    res.acknowledged.append((ref.crate, ref.resolved))
                    continue
                res.findings.append(
                    f"{ref.crate} reads {ref.resolved} (from {ref.src}) which is mapped to "
                    f"{crates} but NOT to {ref.crate!r}; add it to that entry's `crates`"
                )
                continue
            # Unmapped repo-level input.
            ack = _ack_for(ref, acks)
            if ack is not None:
                used_acks.add(ack)
                res.acknowledged.append((ref.crate, ref.resolved))
                continue
            res.findings.append(
                f"{ref.crate} reads UNMAPPED repo-level input {ref.resolved} (from {ref.src}); "
                f"add a ci/path-ownership.toml [[map]] entry attributing it to {ref.crate!r}"
            )
            continue

        # owner is a different crate D: covered iff reader in reverse-closure(D),
        # OR the ownership map declares this reader an additional-reader for the
        # path (sq-m4bxc — the selector unions it into the affected set even
        # though D owns the path; the dep edge that reverse-closure would need is
        # a forbidden cycle).
        if ref.crate in closures.get(owner, {owner}):
            res.covered.append((ref.crate, ref.resolved, f"dep-closure of {owner}"))
            continue
        if ref.crate in cs.additional_readers(ref.resolved, map_entries):
            res.covered.append((ref.crate, ref.resolved, "additional-readers map"))
            continue
        ack = _ack_for(ref, acks)
        if ack is not None:
            used_acks.add(ack)
            res.acknowledged.append((ref.crate, ref.resolved))
            continue
        res.findings.append(
            f"{ref.crate} reads {ref.resolved} owned by {owner!r} (from {ref.src}) but is NOT in "
            f"{owner}'s reverse-dependency closure; a change to {owner}'s dir would skip {ref.crate}. "
            f"Add a dep edge, relocate the input, or acknowledge it as a residual"
        )

    for r in acks:
        if r not in used_acks:
            res.findings.append(
                f"STALE acknowledged residual: {r.crate} -> {r.target} matched no discovered read; "
                f"remove it from KNOWN_RESIDUALS"
            )

    res.ok = not res.findings
    return res


# --- real-workspace entrypoint ----------------------------------------------
def _closures_from_meta(meta: dict) -> dict[str, set[str]]:
    ws = cs.parse_workspace(meta)
    return {name: cs.reverse_closure(name, ws.reverse_adj) for name in ws.members}


def run_audit(repo_root: str, map_file: str | None, meta: dict) -> AuditResult:
    crate_roots = enumerate_crates(repo_root)
    map_entries = cs.load_ownership_map(map_file)
    closures = _closures_from_meta(meta)
    refs = scan_refs(repo_root, crate_roots)
    return audit_refs(refs, crate_roots, map_entries, closures)


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Out-of-crate-input audit (design §4.2).")
    p.add_argument("--repo-root", help="repo root (default: git toplevel)")
    p.add_argument("--map", dest="map_file", help="ownership map (default ci/path-ownership.toml)")
    p.add_argument("--metadata-file", help="cargo metadata JSON snapshot (default: run cargo)")
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    repo_root = args.repo_root or cs._resolve_repo_root(None) or os.getcwd()
    map_file = args.map_file or os.path.join(repo_root, "ci", "path-ownership.toml")
    try:
        meta = cs.load_metadata(args.metadata_file, repo_root)
    except Exception as exc:  # noqa: BLE001 - report cleanly, do not traceback
        print(f"out-of-crate-input audit: could not load cargo metadata: {exc}", file=sys.stderr)
        return 2
    result = run_audit(repo_root, map_file if os.path.exists(map_file) else None, meta)
    print(f"out-of-crate-input audit: {len(result.covered)} covered, "
          f"{len(result.acknowledged)} acknowledged residual(s), {len(result.findings)} finding(s)")
    for c, r, how in sorted(result.covered):
        print(f"  ok   {c} -> {r}  [{how}]")
    for c, r in sorted(result.acknowledged):
        print(f"  ack  {c} -> {r}")
    for f in result.findings:
        print(f"  FAIL {f}")
    return 0 if result.ok else 1


if __name__ == "__main__":
    sys.exit(main())
