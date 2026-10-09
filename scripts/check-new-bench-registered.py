#!/usr/bin/env python3
# Gate G3 — new-bench->registry+dashboard (bead sq-ncvq.6, epic sq-ncvq).
#
# The PROACTIVE / merge-time half of the maintenance flow-on system
# (research/maintenance-flow-on-automation-design.md §2.1, gate G3). Sibling of
# scripts/gate-new-crate.py (G1) + scripts/gate-api-skill.py (G2). The reactive
# engine (scripts/flow-on.py, rule `new-bench-dashboard-row`) mints a follow-on
# ISSUE after a PR merges; THIS gate flags the gap BEFORE merge so a new bench
# suite never lands invisible to the registry + capability dashboard.
#
# sq-ncvq.10 doc-sync: this gate is the "Enforced by: **G3**" cell of
# the "new bench suite" row in the AGENTS.md "Post-batch re-evaluation checklist"
# table (alongside the reactive `flow-on:new-bench-dashboard-row` rule). That
# table row and this docstring are the two halves of the same rule — change one
# and update the other; the divergence is what sq-ncvq.10 exists to prevent.
#
# RULE (G3): when a PR introduces a NEW bench suite — either
#   (T1) a NEW `[[benchmark]]` id in bench/benchmarks.toml (an id present at HEAD
#        but not at the base ref), OR
#   (T2) a NEW `bench/<suite>/` directory (a file added under a suite dir that did
#        not exist at the base ref),
# then that suite must be
#   (a) REGISTERED in bench/benchmarks.toml — a `[[benchmark]]` entry whose `id`
#       (T1, registered by construction) or whose `source`/`invoke`/`dataset`
#       references `bench/<suite>/` (T2); AND
#   (b) for a CAPABILITY suite, either PROMOTED on the dashboard — its id or family
#       token appears in the FEATURED_SUITES block of bench/dashboard/dashboard.js —
#       OR explicitly flagged `featured = false` in its bench/benchmarks.toml entry
#       (the design's documented escape hatch).
#
# ESCAPE HATCH (design §2.1): `featured = false` on the benchmark's TOML entry
# marks it an intentionally-unfeatured suite (internal harness / micro-bench /
# external-cost / spike), exempt from the dashboard-row requirement (b). The
# registry requirement is never waived — a suite must always be catalogued.
#
# NON-CAPABILITY FAMILIES are exempt from (b) by default (mirrors
# scripts/drift-scan.py::DASHBOARD_EXEMPT_FAMILIES + the `sparq-bench` harness
# prefix): the CLI query-suite runners, the hw/ci probes, the serve/memtier
# spikes, the external-cost wikidata suites, and the parse/compress micro-benches
# that feed the ingest harness rather than standing as a capability card. They
# still must be REGISTERED (a), just not featured.
#
# G3 is HEURISTIC (path/id based) and — per design §2.1 — starts ADVISORY: a
# false positive is a nag, not a block, until tuned. Run with --advisory for the
# soft-launch (always exit 0, report only).
#
# DIFF SOURCE (CI): the PR's changed/added files + the base-ref toml content
#   git diff --name-status origin/<base>...HEAD     (added bench/<suite>/ dirs)
#   git show origin/<base>:bench/benchmarks.toml     (base ids, to diff vs HEAD)
# In tests / --dry-run every git/disk fact is injected via fixtures + overrides,
# so the evaluate() core is fully hermetic (no live git, no network). stdlib-only.
#
# Usage:
#   check-new-bench-registered.py                     # CI: diff vs origin/<base>
#   check-new-bench-registered.py --base main         # override base ref
#   check-new-bench-registered.py --advisory          # soft-launch (never exit 1)
#   check-new-bench-registered.py --dry-run --changed-files f.txt --base-toml b.toml
#       # hermetic: added files from f.txt, base registry from b.toml

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BENCH_REGISTRY = REPO_ROOT / "bench" / "benchmarks.toml"
DASHBOARD_JS = REPO_ROOT / "bench" / "dashboard" / "dashboard.js"

# A line in a status-prefixed diff: "A\tpath", "M\tpath", "R100\told\tnew", etc.
_STATUS_RE = re.compile(r"^([A-Z])\d*\t(.*)$")

# A bench suite DIR token: a single path component under bench/ — letters,
# digits, dot, underscore, hyphen (NO spaces/quotes). Used to extract `<suite>`
# from a `bench/<suite>/` reference that may sit inside a multi-word `invoke`
# command string, so we never capture trailing shell text.
_SUITE_TOKEN = r"[A-Za-z0-9._-]+"
_BENCH_DIR_RE = re.compile(rf"\bbench/({_SUITE_TOKEN})/")

# Bench FAMILIES (the leading `bench/<suite>/` token) that the design (§5.D)
# accepts as intentionally unfeatured on the capability dashboard — kept in
# lock-step with scripts/drift-scan.py::DASHBOARD_EXEMPT_FAMILIES. Such a suite
# must still be REGISTERED, but it is exempt from the dashboard-row requirement
# (b) WITHOUT needing a `featured = false` flag.
NONCAPABILITY_SUITE_FAMILIES = frozenset(
    {
        "cli",  # CLI query-suite runners (cli-bench-*, cli-ingest, …)
        "src",  # bench/src — the sparq-bench harness sources, not a suite dir
        "hw",  # hardware probe (hw-bench)
        "ci",  # CI harness (ci-bench, ci-bench-ec2)
        "serve",  # serve spike (research SPIKE, not a maintained suite)
        "memtier",  # memtier spike
        "wikidata",  # external-cost suites (wikidata-8b) — acceptably unfeatured
        "parse",  # parse-baseline micro-bench feeds the ingest harness
        "compress",  # compression micro-bench, not a capability card
        "competitor",  # competitor-gather plumbing, not a suite
        "dashboard",  # the dashboard itself, not a measured suite
    }
)


# --------------------------------------------------------------------------- #
# Diff parsing (mirrors gate-new-crate.py exactly so the two gates agree)
# --------------------------------------------------------------------------- #
def parse_status_lines(lines: list[str]) -> tuple[list[str], list[str]]:
    """Split a `git diff --name-status`-style list into (changed, added).

    Each line is either "<STATUS>\t<path>" (or "<STATUS>\t<old>\t<new>" for
    renames/copies) or a bare path (a generic change, NOT provably added).
    `A` (added) and `C` (copied) both materialise a NEW destination path."""
    changed: list[str] = []
    added: list[str] = []
    for raw in lines:
        line = raw.rstrip("\n")
        if not line.strip():
            continue
        m = _STATUS_RE.match(line)
        if m:
            status, rest = m.group(1), m.group(2)
            path = rest.split("\t")[-1]  # rename/copy dest is the last field
            changed.append(path)
            if status in ("A", "C"):
                added.append(path)
        else:
            changed.append(line)
    return changed, added


def _normalize_base(base: str) -> str:
    """Accept a bare branch ('main'), a full ref ('refs/heads/main', as
    merge_group.base_ref supplies) or an origin/* ref and return the
    remote-tracking ref to diff against ('origin/main')."""
    base = base.strip()
    if base.startswith("refs/heads/"):
        base = base[len("refs/heads/") :]
    if base.startswith("origin/") or base == "HEAD":
        return base
    return f"origin/{base}"


def git_diff(base: str) -> tuple[list[str], list[str]]:
    """Return (changed, added) from git vs the base ref using a 3-dot diff."""
    ref = _normalize_base(base)
    try:
        out = subprocess.run(
            ["git", "diff", "--name-status", f"{ref}...HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except subprocess.CalledProcessError as e:  # pragma: no cover - CI-only path
        sys.stderr.write(f"error: git diff failed: {e.stderr}\n")
        sys.exit(2)
    return parse_status_lines(out.splitlines())


def git_base_toml(base: str) -> str:
    """The base ref's bench/benchmarks.toml content ('' if it did not exist)."""
    ref = _normalize_base(base)
    try:
        return subprocess.run(
            ["git", "show", f"{ref}:bench/benchmarks.toml"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except subprocess.CalledProcessError:  # pragma: no cover - CI-only path
        return ""  # file absent at base => every HEAD id is new


def git_base_bench_suites(base: str) -> set[str]:
    """The set of `bench/<suite>/` dirs that already existed at the base ref.

    Without this, a file merely ADDED inside a PRE-EXISTING suite dir would be
    mis-read as a brand-new suite (a false positive). We list the base tree under
    bench/ and collect each immediate child component."""
    ref = _normalize_base(base)
    try:
        out = subprocess.run(
            ["git", "ls-tree", "-r", "--name-only", f"{ref}", "bench/"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except subprocess.CalledProcessError:  # pragma: no cover - CI-only path
        return set()
    suites: set[str] = set()
    for line in out.splitlines():
        m = re.match(rf"^bench/({_SUITE_TOKEN})/", line)
        if m:
            suites.add(m.group(1))
    return suites


# --------------------------------------------------------------------------- #
# Registry (benchmarks.toml) introspection — textual, mirroring the sibling
# gates (the registry is hand-authored prose-heavy TOML; a structural array
# parse is unnecessary and brittle).
# --------------------------------------------------------------------------- #
def _read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def registry_ids(text: str) -> list[str]:
    """Every benchmark `id` declared in the registry text, in file order."""
    return re.findall(r'^\s*id\s*=\s*"([^"]+)"', text, re.MULTILINE)


def registry_blocks(text: str) -> list[dict[str, object]]:
    """Split the registry into per-`[[benchmark]]` blocks, returning for each a
    dict of the fields we care about: id, the joined text of source/invoke/
    dataset/records_to (for dir↔entry mapping) and whether `featured = false` is
    set.

    Keys: {'id': str, 'refs': str, 'featured_false': bool}. Robust to the
    registry's aligned-`=` formatting and multi-line string values (it slices on
    the `[[benchmark]]` header, not on field boundaries)."""
    blocks: list[dict[str, object]] = []
    # Split on the table-array header; the first chunk is the file preamble.
    parts = re.split(r"(?m)^\s*\[\[benchmark\]\]\s*$", text)
    for chunk in parts[1:]:
        idm = re.search(r'^\s*id\s*=\s*"([^"]+)"', chunk, re.MULTILINE)
        bid = idm.group(1) if idm else ""
        # Collect every value text we use to map a bench/<suite>/ dir → entry.
        refs = " ".join(
            re.findall(
                r"^\s*(?:source|invoke|dataset|records_to)\s*=\s*(.*)$",
                chunk,
                re.MULTILINE,
            )
        )
        featured_false = (
            re.search(r"^\s*featured\s*=\s*false\b", chunk, re.MULTILINE) is not None
        )
        blocks.append({"id": bid, "refs": refs, "featured_false": featured_false})
    return blocks


def block_for_suite(
    blocks: list[dict[str, object]], suite: str
) -> dict[str, object] | None:
    """The first registry block whose refs reference `bench/<suite>/`, or None."""
    needle = re.compile(rf"\bbench/{re.escape(suite)}/")
    for b in blocks:
        if needle.search(str(b["refs"])):
            return b
    return None


def suite_for_block(block: dict[str, object]) -> str | None:
    """The first `bench/<suite>/` dir token a registry block references, or None.
    Restricted to a valid dir-name token so a multi-word `invoke` command never
    leaks shell text into the captured suite name."""
    m = _BENCH_DIR_RE.search(str(block.get("refs", "")))
    return m.group(1) if m else None


def block_for_id(
    blocks: list[dict[str, object]], bid: str
) -> dict[str, object] | None:
    for b in blocks:
        if b["id"] == bid:
            return b
    return None


# --------------------------------------------------------------------------- #
# Dashboard (FEATURED_SUITES) introspection.
# --------------------------------------------------------------------------- #
def dashboard_featured_text(text: str) -> str:
    """The FEATURED_SUITES array body, lowercased (matched textually — the file
    is JS, not data we should eval). '' if the block is absent."""
    m = re.search(r"FEATURED_SUITES\s*=\s*\[(.*?)\];", text, re.DOTALL)
    return (m.group(1) if m else "").lower()


def is_featured(featured_text: str, bid: str, family: str) -> bool:
    """True iff the suite's id OR its family token appears in the FEATURED_SUITES
    block. Loose by design (the JS matcher itself matches on aliases/tokens) —
    G3 flags whole FAMILIES with no promotion, not the exact JS alias logic."""
    if not featured_text:
        return False
    # Token forms the dashboard aliases use: `deep-taxonomy`, `deeptax`, `deep taxonomy`.
    candidates = {
        bid.lower(),
        family.lower(),
        family.replace("-", " ").lower(),
        family.replace("-", "").lower(),
    }
    return any(c and c in featured_text for c in candidates)


# --------------------------------------------------------------------------- #
# Suite detection from the diff.
# --------------------------------------------------------------------------- #
def suite_family(suite: str) -> str:
    """The leading slug token of a suite dir name (e.g. `zk` for `zk-compose`,
    `qlever` for `qlever-100m`) — the unit the dashboard features."""
    return suite.split("-", 1)[0]


def added_bench_suites(added: list[str], base_suites: set[str]) -> list[str]:
    """Suite dirs introduced by the diff: a file added under `bench/<suite>/`
    whose `<suite>` dir did not exist at the base ref. Stable, de-duplicated."""
    out: list[str] = []
    seen: set[str] = set()
    for p in added:
        m = re.match(rf"^bench/({_SUITE_TOKEN})/", p)
        if not m:
            continue
        suite = m.group(1)
        if suite in base_suites or suite in seen:
            continue
        seen.add(suite)
        out.append(suite)
    return out


def new_registry_ids(head_toml: str, base_toml: str) -> list[str]:
    """Benchmark ids present at HEAD but not at the base ref (newly registered)."""
    base = set(registry_ids(base_toml))
    return [i for i in registry_ids(head_toml) if i not in base]


# --------------------------------------------------------------------------- #
# Core evaluation (pure: every git/disk fact is an argument or override).
# --------------------------------------------------------------------------- #
def evaluate(
    added: list[str],
    *,
    head_toml: str,
    base_toml: str,
    featured_text: str,
    base_suites: set[str] | None = None,
) -> list[tuple[str, list[str]]]:
    """Return [(suite-or-id, [missing-reasons])] for every new bench suite that
    violates G3. An empty list means the gate PASSES.

    `added` is the diff's added-file list; `head_toml`/`base_toml` are the
    registry at HEAD/base; `featured_text` is the lowercased FEATURED_SUITES
    body; `base_suites` is the set of suite dirs that already existed at base
    (so a file added to a pre-existing suite is not a 'new suite')."""
    base_suites = base_suites if base_suites is not None else set()
    head_blocks = registry_blocks(head_toml)
    violations: list[tuple[str, list[str]]] = []

    # De-dup so a suite reached via BOTH a new dir and a new id is reported once.
    seen: set[str] = set()

    def check(
        label: str,
        block: dict[str, object] | None,
        family: str,
        *,
        registered: bool,
    ) -> None:
        if label in seen:
            return
        seen.add(label)
        missing: list[str] = []
        if not registered:
            missing.append(
                f"a registered [[benchmark]] entry in bench/benchmarks.toml whose "
                f"source/invoke/dataset references bench/{label}/"
            )
        # Dashboard requirement only for capability suites.
        if family not in NONCAPABILITY_SUITE_FAMILIES:
            featured_false = bool(block and block.get("featured_false"))
            bid = str((block or {}).get("id", ""))
            if not featured_false and not is_featured(featured_text, bid, family):
                missing.append(
                    f"a FEATURED_SUITES row in bench/dashboard/dashboard.js for the "
                    f"`{family}` suite — OR `featured = false` in its "
                    f"bench/benchmarks.toml entry if it is intentionally unfeatured"
                )
        if missing:
            violations.append((label, missing))

    # (T2) New bench/<suite>/ directories.
    for suite in added_bench_suites(added, base_suites):
        block = block_for_suite(head_blocks, suite)
        check(suite, block, suite_family(suite), registered=block is not None)

    # (T1) New benchmark ids. A new id is registered by construction; we only
    # check the dashboard requirement. Key the report on the suite dir the id
    # references (so it de-dups against a T2 hit) when resolvable, else the id.
    for bid in new_registry_ids(head_toml, base_toml):
        block = block_for_id(head_blocks, bid)
        suite = suite_for_block(block) if block else None
        label = suite or bid
        family = suite_family(suite) if suite else suite_family(bid)
        check(label, block, family, registered=True)

    return violations


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="G3 new-bench->registry+dashboard gate (sq-ncvq.6)."
    )
    ap.add_argument(
        "--base",
        default=os.environ.get("GATE_BASE_REF", "main"),
        help="base ref to diff against (origin/<base>); default 'main'.",
    )
    ap.add_argument(
        "--changed-files",
        help="hermetic input: a file of diff lines (status-prefixed or bare paths).",
    )
    ap.add_argument(
        "--base-toml",
        help="hermetic input: the base ref's bench/benchmarks.toml content.",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="never exit non-zero; print the verdict only (for local/testing).",
    )
    ap.add_argument(
        "--advisory",
        action="store_true",
        help="soft-launch: report violations but always exit 0.",
    )
    args = ap.parse_args(argv)

    if args.changed_files:
        lines = Path(args.changed_files).read_text(encoding="utf-8").splitlines()
        _, added = parse_status_lines(lines)
        # Hermetic mode: no base tree to list, so a file added under an existing
        # suite dir cannot be distinguished from a new one. Tests inject
        # base_suites via evaluate() directly when that distinction matters.
        base_suites: set[str] = set()
    else:
        _, added = git_diff(args.base)
        base_suites = git_base_bench_suites(args.base)

    head_toml = _read(BENCH_REGISTRY)
    if args.base_toml is not None:
        base_toml = Path(args.base_toml).read_text(encoding="utf-8")
    else:
        base_toml = git_base_toml(args.base)
    featured_text = dashboard_featured_text(_read(DASHBOARD_JS))

    violations = evaluate(
        added,
        head_toml=head_toml,
        base_toml=base_toml,
        featured_text=featured_text,
        base_suites=base_suites,
    )

    if not violations:
        print(
            "G3 new-bench->registry+dashboard: PASS — no new bench suite missing artifacts."
        )
        return 0

    print("G3 new-bench->registry+dashboard: FAIL")
    for label, missing in violations:
        print(f"\n  new bench suite `{label}` is missing:")
        for m in missing:
            print(f"    - {m}")
    print(
        "\nA new bench suite must be REGISTERED in bench/benchmarks.toml and "
        "(if a capability suite) either added to FEATURED_SUITES in "
        "bench/dashboard/dashboard.js OR flagged `featured = false` in its toml "
        "entry (research/maintenance-flow-on-automation-design.md §2.1, gate G3)."
    )

    if args.advisory or args.dry_run:
        print("\n(advisory/dry-run: not failing the build)")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
