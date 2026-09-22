#!/usr/bin/env python3
# [OPUS-4.8] Periodic drift scanner (bead sq-ncvq.11, epic sq-ncvq).
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# The PERIODIC AUDIT half of the maintenance flow-on system
# (research/maintenance-flow-on-automation-design.md §5). The three halves:
#   - the merge-time GATES (scripts/gate-*.py, ci.yml) stop NEW drift landing;
#   - the merge-triggered ENGINE (scripts/flow-on.py, flow-on.yml) mints
#     follow-ons when a PR merges;
#   - THIS scanner sweeps the WHOLE repo on a schedule to find EXISTING drift
#     the gates predate or the heuristics missed. It complements, never gates.
#
# DRIFT CLASSES (the first five map to a §5 finding in the design doc):
#   bench-missing       a PUBLISHABLE crate with NO registered benchmark in
#                       bench/benchmarks.toml (cross-ref `source = crates/<x>` vs
#                       crates/*/). Honors gate G1's `publish = false` stub
#                       exemption (sq-bif.5), so benchless stubs are not flagged. §5.A
#   skill-missing       a PUBLIC crate named in NO skills/**/SKILL.md             §5.B
#   explain-asymmetry   a public-surface capability present in Rust/HTTP but
#                       absent from the WASM/JS binding (best-effort)             §5.C
#   dashboard-row       a registered bench suite with NO FEATURED_SUITES row in
#                       bench/dashboard/dashboard.js                              §5.D
#   conformance-split   a conformance ratchet living OUTSIDE the central
#                       sparq-conformance scoreboard                             §5.E
#   beads-export-stale  the committed `.beads/issues.jsonl` export has fallen
#                       behind the live bd DB — measured by the export's own
#                       newest record stamp plus the count of bead ids the repo
#                       cites that have no record in it. CI cannot diff the real
#                       DB (gitignored, orchestrator-local), so both are proxies;
#                       see scan_beads_export_stale.        bead sq-0b0sh
#
# NOTE ON THE §-REFS: classes A–E each map to a numbered finding in the design
# doc's §5. beads-export-stale does NOT — it post-dates the doc (bead sq-0b0sh,
# found by the 2026-07 program-level status review) and carries no § of its own.
#
# WHY GITHUB ISSUES, NOT `bd create` (identical reasoning to flow-on.py):
# (design §2.2) In CI there is NO `bd` dolt DB — it lives only in the
# orchestrator's main checkout and is gitignored; hand-editing `.beads/` is
# forbidden. So this script must NOT touch `bd` or `.beads/`. It emits each
# drift item as a GitHub issue labelled `drift` + `auto`; the orchestrator
# reconciles them into beads out-of-band.
#
# IDEMPOTENCY: each drift item has a STABLE dedup_key (e.g.
# `bench-missing:sparq-nlq`). The key is embedded in the issue body as
# `<!-- drift-key: <key> -->`. Before creating, the script searches OPEN issues
# for that marker and skips if one exists — so re-running the weekly sweep never
# duplicates an item. The key is deliberately content-free (no PR/date) so it is
# stable across runs.
#
# Usage:
#   drift-scan.py --dry-run              # print the drift report (text); no gh writes
#   drift-scan.py --dry-run --json out.json   # also write machine-readable JSON
#   drift-scan.py                        # CI: file/update `drift`+`auto` issues
#   drift-scan.py --root path/to/repo    # scan a different tree (tests use fixtures)
#
# Hermetic: with --dry-run the script makes NO gh calls at all, so it is fully
# testable against a fixture repo layout with no network/gh/git. stdlib-only.

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Labels every drift issue carries.
BASE_LABELS = ["drift", "auto"]

# --------------------------------------------------------------------------- #
# beads-export-stale (bead sq-0b0sh) tuning — see scan_beads_export_stale
# for what these two thresholds are proxies FOR and why neither can be exact.
# --------------------------------------------------------------------------- #
BEADS_EXPORT_REL = ".beads/issues.jsonl"

# Days the export's newest record stamp may lag before it counts as stale. The
# sweep is weekly (drift-scan.yml), so 14 = one full missed refresh cycle, not a
# single late day.
BEADS_EXPORT_STALE_DAYS = 14

# How many repo-cited-but-unexported bead ids are tolerated before the count
# alone trips the check. Not zero: closed beads that were later compacted/gc'd
# out of the DB stay cited in research records forever and are legitimately
# absent from the export. Scores of them, though, mean a real lag.
BEADS_MISSING_REF_THRESHOLD = 25

# A bd bead id: `sq-` + a 3-6 char hash, optionally `.N` sub-bead suffixes.
# The trailing lookahead (rather than a bare `\b`) rejects a longer hyphenated
# token such as `sq-bench-adapters`, which `\b` would happily truncate to a
# phantom `sq-bench` id.
BEAD_ID_RE = re.compile(r"\bsq-[0-9a-z]{3,6}(?:\.\d+)*(?![-\w])")

# The reference scan is bounded by suffix, size and directory so it stays a
# couple of seconds on the full tree — irrelevant against a weekly sweep, but
# the bounds also keep it off vendored/build trees where a bead id would be
# noise. `.beads/` is excluded on purpose: the export must not corroborate
# itself.
REF_SCAN_SUFFIXES = frozenset(
    {".md", ".rs", ".py", ".yml", ".yaml", ".toml", ".sh", ".ts", ".tsx", ".js", ".mjs", ".nr"}
)
REF_SCAN_SKIP_DIRS = frozenset(
    {".git", ".beads", "node_modules", "target", "dist", ".next", "__pycache__"}
)
REF_SCAN_MAX_BYTES = 1_000_000

# Crates whose absence of a DIRECT `source = crates/<x>` bench registration is
# by design, because they are the shared query-execution STACK that the CLI /
# differential harnesses already measure transitively (design §5.A: "Covered,
# not gaps: ... core/engine/cli via CLI harnesses"). A `[[benchmark]]` whose
# `source` references `crates/sparq-cli` or `crates/sparq-bench` exercises this
# whole stack, so flagging each of them individually would be a false positive.
#   - sparq-bench / sparq-conformance: the harness + conformance runner
#     themselves, not measured TARGETS;
#   - sparq-core / sparq-engine / sparq-cli: the query stack the CLI bench
#     suites (cli-bench-*, sparq-bench-compare, operator-coverage, …) drive;
#   - sparq-canon: a thin RDFC-1.0 lib surfaced via the rdf-canon skill, no
#     standalone perf surface.
#   - sparq-py: a thin pyo3 binding (`crate-type = ["cdylib"]`, publish = false).
#     It exposes NO native Rust-callable surface a Rust example/criterion bench
#     could touch — it is loadable ONLY as a CPython extension module (built by
#     maturin), and its engine logic is a thin wrapper over sparq-core/engine/
#     reason that the CLI / operator-coverage suites already measure. A meaningful
#     Python-level bench would need a built wheel + a pytest-benchmark harness in
#     CI (outside the Rust-bench registry model), so it is HONESTLY exempted here
#     rather than carrying a fake Rust bench. (sq-ncvq.12, drift catch-up A.)
# Everything NOT listed here is scanned — so genuinely unbenched surfaces
# still surface, matching the design's concrete §5.A gap list. The other §5.A
# crates are now COVERED by registered benches (sq-ncvq.12): sparq-mpc
# (mpc-bench-matrix, the existing counting-tier example), sparq-wasm (wasm-bundle,
# the deterministic browser-bundle-size measurement), sparq-nlq (nlq-offline-bench),
# and sparq-serve (serve-core-bench).
BENCH_EXEMPT_CRATES = frozenset(
    {
        "sparq-bench",
        "sparq-conformance",
        "sparq-core",
        "sparq-engine",
        "sparq-cli",
        "sparq-canon",
        "sparq-py",
    }
)

# Bench FAMILIES (leading id token) that the design (§5.D) accepts as
# intentionally unfeatured on the capability dashboard: internal harness
# runners, SPIKES, external-cost suites, and the raw ingest micro-benches.
# [OPUS-4.8] sq-ncvq.16 + sq-j174: crates whose crate-local conformance ratchet is
# now CONSOLIDATED into the central sparq-conformance scoreboard registry
# (crates/sparq-conformance/src/scoreboard.rs `SUITES`). Their runners stay
# crate-local (they depend on sparq-shacl / sparq-geo / sparq-solid, which the
# dev-only conformance crate must not take on as deps), but the central scoreboard
# now REPORTS them alongside SPARQL/inference with their ratchet floors. [SONNET-4.6]
# sq-z1xv8: those floors are now ONE shared const per ratchet (the zero-dependency
# `sparq-conformance-floors` crate, imported by both the runner and the registry row)
# instead of a copy kept in lock-step by the guard test
# (crates/sparq-conformance/tests/scoreboard_floors.rs) re-reading the runner's source.
# So these are no longer a `conformance-split` drift — the split the
# §5.E finding flagged is closed. The scanner derives the live set from the
# registry source (so adding a crate to `SUITES` auto-exempts it), falling back to
# this literal set if the registry can't be read.
CONFORMANCE_CONSOLIDATED_FALLBACK = frozenset({"sparq-shacl", "sparq-geo", "sparq-solid"})

DASHBOARD_EXEMPT_FAMILIES = frozenset(
    {
        "cli",  # CLI query-suite runners (cli-bench-*, cli-ingest, …)
        "hw",  # hardware probe (hw-bench)
        "ci",  # CI harness (ci-bench, ci-bench-ec2)
        "serve",  # serve spike (CATALOG: research SPIKE, not a maintained suite)
        "memtier",  # memtier spike
        "wikidata",  # external-cost suites (wikidata-8b) — acceptably unfeatured
        "parse",  # parse-baseline micro-bench feeds the ingest harness
        "compress",  # compression micro-bench, not a capability card
        "competitor",  # competitor-gather plumbing, not a suite
    }
)


@dataclass
class DriftItem:
    """One detected drift, ready to be reported or filed as an issue."""

    drift_class: str
    dedup_key: str
    title: str
    body: str

    def to_json(self) -> dict[str, str]:
        return {
            "class": self.drift_class,
            "dedup_key": self.dedup_key,
            "title": self.title,
            "body": self.body,
        }


# --------------------------------------------------------------------------- #
# Repo introspection helpers (all pure: take a root, read the tree)
# --------------------------------------------------------------------------- #
def list_crates(root: Path) -> list[str]:
    """Every crate dir name under crates/ that has a Cargo.toml, sorted."""
    crates_dir = root / "crates"
    if not crates_dir.is_dir():
        return []
    out = []
    for child in sorted(crates_dir.iterdir()):
        if (child / "Cargo.toml").is_file():
            out.append(child.name)
    return out


def crate_is_public(root: Path, crate: str) -> bool:
    """A crate is PUBLIC iff its Cargo.toml does NOT carry `publish = false`
    (matching gate-new-crate.py's definition exactly)."""
    cargo = root / "crates" / crate / "Cargo.toml"
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError:
        return True  # absent/unreadable Cargo.toml: treat as public (safe default)
    return re.search(r"^\s*publish\s*=\s*false\b", text, re.MULTILINE) is None


def bench_registry_sources(root: Path) -> str:
    """The raw text of bench/benchmarks.toml (we match `source` lines textually,
    mirroring gate-new-crate.py — a full TOML parse of the array is unnecessary
    and the registry is hand-authored prose-heavy TOML)."""
    reg = root / "bench" / "benchmarks.toml"
    try:
        return reg.read_text(encoding="utf-8")
    except OSError:
        return ""


def crate_has_registered_bench(registry_text: str, crate: str) -> bool:
    """True iff any `source = ...` line references crates/<crate>/ or crates/<crate>".
    Identical predicate to gate-new-crate.py::crate_has_registered_bench."""
    pat = re.compile(
        rf"^\s*source\s*=.*crates/{re.escape(crate)}(?:/|\b)",
        re.MULTILINE,
    )
    return pat.search(registry_text) is not None


def registered_bench_ids(root: Path) -> list[str]:
    """Every benchmark `id` declared in bench/benchmarks.toml, in file order."""
    text = bench_registry_sources(root)
    ids = []
    for m in re.finditer(r'^\s*id\s*=\s*"([^"]+)"', text, re.MULTILINE):
        ids.append(m.group(1))
    return ids


def featured_false_bench_ids(root: Path) -> set[str]:
    """[OPUS-4.8] sq-5o5.2 — every benchmark `id` whose `[[benchmark]]` block sets
    `featured = false` in bench/benchmarks.toml.

    `featured = false` is the design's documented escape hatch (registry schema
    header; research/maintenance-flow-on-automation-design.md §2.1) for a suite
    that is intentionally trend-only — catalogued + visible in trend history, but
    NOT promoted as a head-to-head competitor card on the capability dashboard.
    It makes NO performance claim; it is purely a dashboard DISPOSITION.

    The merge-time gate G3 (scripts/check-new-bench-registered.py) already accepts
    this flag as a dashboard disposition; this reactive scanner did NOT, so the two
    halves of the maintenance-flow-on system could disagree (G3 would pass a
    suite that drift-scan still flagged). We mirror G3's predicate EXACTLY — split
    on the `[[benchmark]]` header, then per block test the same
    `^\\s*featured\\s*=\\s*false\\b` regex G3's registry_blocks() uses — so the two
    tools share one definition of an intentionally-unfeatured suite and cannot
    diverge again."""
    text = bench_registry_sources(root)
    ids: set[str] = set()
    # Split on the table-array header; chunk[0] is the file preamble (no id).
    for chunk in re.split(r"(?m)^\s*\[\[benchmark\]\]\s*$", text)[1:]:
        idm = re.search(r'^\s*id\s*=\s*"([^"]+)"', chunk, re.MULTILINE)
        if not idm:
            continue
        if re.search(r"^\s*featured\s*=\s*false\b", chunk, re.MULTILINE):
            ids.add(idm.group(1))
    return ids


def crates_named_in_skills(root: Path) -> set[str]:
    """Every sparq-<x> crate token mentioned in ANY skills/**/SKILL.md.

    Note these tokens are free text in the skill prose (e.g. `sparq-engine`),
    so a crate counts as 'covered' the moment any skill names it. We only treat
    EXACT crate dir names as coverage; aspirational tokens like `sparq-explain`
    (no such crate) are ignored by the caller (it intersects with real crates)."""
    skills_dir = root / "skills"
    named: set[str] = set()
    if not skills_dir.is_dir():
        return named
    for skill in skills_dir.rglob("SKILL.md"):
        try:
            text = skill.read_text(encoding="utf-8")
        except OSError:
            continue
        for tok in re.findall(r"\bsparq-[a-z0-9-]+", text):
            named.add(tok)
    return named


def dashboard_featured_keys(root: Path) -> str:
    """Raw text of the FEATURED_SUITES block in bench/dashboard/dashboard.js.

    Returns the slice from `FEATURED_SUITES = [` to the closing `];`, lowercased,
    so the caller can test whether a suite's name/aliases appear. We match
    textually (the file is JS, not data we should eval)."""
    dash = root / "bench" / "dashboard" / "dashboard.js"
    try:
        text = dash.read_text(encoding="utf-8")
    except OSError:
        return ""
    m = re.search(r"FEATURED_SUITES\s*=\s*\[(.*?)\];", text, re.DOTALL)
    return (m.group(1) if m else "").lower()


# --------------------------------------------------------------------------- #
# Drift scanners — one per class. Each returns a list[DriftItem].
# --------------------------------------------------------------------------- #
def scan_bench_missing(root: Path) -> list[DriftItem]:
    """§5.A — crates with NO registered benchmark in bench/benchmarks.toml.

    [OPUS-4.8] sq-bif.5: honor the SAME `publish = false` exemption the merge-time
    gate G1 (scripts/gate-new-crate.py) applies to its bench requirement (a). G1's
    `publish = false` escape hatch (design §2.1) marks an intentional stub that is
    exempt from needing a registered benchmark; the reactive scanner must mirror
    that or it mints spurious `bench-missing` drift for crates that are legitimately
    benchless by design. We reuse `crate_is_public` (whose predicate is the exact
    inverse of G1's `crate_is_stub` — the identical `^\\s*publish\\s*=\\s*false\\b`
    regex), so the two tools share one definition of "stub" and cannot diverge
    again. This is a tool-consistency fix, NOT new bench coverage — no benchmarks
    are added; the exempted crates remain genuinely unbenched, which is fine for a
    `publish = false` stub exactly as G1 allows at merge time."""
    registry = bench_registry_sources(root)
    items: list[DriftItem] = []
    for crate in list_crates(root):
        if crate in BENCH_EXEMPT_CRATES:
            continue
        if not crate_is_public(root, crate):
            # `publish = false` stub: exempt, mirroring gate-new-crate.py's G1
            # bench-requirement exemption (single source of truth: crate_is_public).
            continue
        if crate_has_registered_bench(registry, crate):
            continue
        key = f"bench-missing:{crate}"
        items.append(
            DriftItem(
                drift_class="bench-missing",
                dedup_key=key,
                title=f"drift: crate `{crate}` has no registered benchmark",
                body=(
                    f"`crates/{crate}` is not referenced by any `source = ...` line in "
                    f"`bench/benchmarks.toml`, so it is invisible to the CATALOG / CI / "
                    f"dashboard.\n\n"
                    f"Follow-on: register a `[[benchmark]]` entry whose `source` "
                    f"references `crates/{crate}/` (an example bench is enough — cf. the "
                    f"mpc/wasm example-bench gaps in the flow-on design §5.A), or, if the "
                    f"crate is intentionally unbenched, record that decision (e.g. mark it "
                    f"`publish = false` and note it in the crate README)."
                ),
            )
        )
    return items


def scan_skill_missing(root: Path) -> list[DriftItem]:
    """§5.B — PUBLIC crates named in NO skills/**/SKILL.md."""
    named = crates_named_in_skills(root)
    items: list[DriftItem] = []
    for crate in list_crates(root):
        if not crate_is_public(root, crate):
            continue  # private crates are not a public surface; exempt
        if crate in named:
            continue
        key = f"skill-missing:{crate}"
        items.append(
            DriftItem(
                drift_class="skill-missing",
                dedup_key=key,
                title=f"drift: public crate `{crate}` is named in no SKILL.md",
                body=(
                    f"`crates/{crate}` is a public surface (its Cargo.toml is not "
                    f"`publish = false`) yet it is mentioned in no `skills/**/SKILL.md`.\n\n"
                    f"Follow-on (AGENTS.md public-API → SKILL.md rule): either add a "
                    f"`skills/<surface>/SKILL.md` covering `{crate}` (or name it in an "
                    f"existing skill), or — if it is intentionally internal — make it "
                    f"`publish = false` and say so in the crate README."
                ),
            )
        )
    return items


def scan_explain_asymmetry(root: Path) -> list[DriftItem]:
    """§5.C — a public-surface capability present in Rust/HTTP but absent from
    the WASM/JS binding. Best-effort, EXPLAIN-shaped: we look for `explain`
    exposed in the engine + HTTP server but NOT exported from the wasm binding."""
    items: list[DriftItem] = []

    def file_text(rel: str) -> str:
        try:
            return (root / rel).read_text(encoding="utf-8")
        except OSError:
            return ""

    engine = file_text("crates/sparq-engine/src/explain.rs")
    http = file_text("crates/sparq-server/src/http.rs")
    wasm = file_text("crates/sparq-wasm/src/lib.rs")

    # Capability present on the Rust + HTTP surface?
    rust_has_explain = "explain" in engine or "explain" in http
    # Exported from the wasm binding? (a `pub fn`/exported `explain` symbol).
    wasm_has_explain = re.search(r"\bexplain\w*", wasm) is not None

    if rust_has_explain and not wasm_has_explain:
        key = "explain-asymmetry:wasm"
        items.append(
            DriftItem(
                drift_class="explain-asymmetry",
                dedup_key=key,
                title="drift: EXPLAIN exposed in Rust/HTTP but not in the WASM/JS binding",
                body=(
                    "`explain`/`explain_analyze` exist in `crates/sparq-engine/src/explain.rs` "
                    "and over HTTP (`crates/sparq-server/src/http.rs`), but the WASM/JS binding "
                    "(`crates/sparq-wasm/src/lib.rs`) exports no `explain` symbol — a "
                    "public-surface capability asymmetry (design §5.C).\n\n"
                    "Follow-on: either export EXPLAIN to the JS binding, or explicitly document "
                    "the omission in `skills/javascript-wasm/SKILL.md` so the asymmetry is "
                    "intentional and recorded."
                ),
            )
        )
    return items


def scan_dashboard_row(root: Path) -> list[DriftItem]:
    """§5.D — registered bench suites with NO FEATURED_SUITES dashboard row.

    Heuristic: a registered benchmark `id` is 'featured' iff its id (or a
    leading token of it) appears in the FEATURED_SUITES block's aliases. This
    is intentionally loose (the dashboard matches on aliases/tokens too) — its
    job is to flag whole FAMILIES with no promotion (e.g. the entire ZK family),
    not to perfectly mirror the JS alias matcher.

    A suite also clears the drift if its `[[benchmark]]` entry sets
    `featured = false` (sq-5o5.2): that is the design's documented trend-only
    DISPOSITION — catalogued + in trend history but not a head-to-head competitor
    card — and it makes NO performance claim. Honoring it here keeps this reactive
    scanner in lock-step with the merge-time gate G3, which already accepts the same
    flag (scripts/check-new-bench-registered.py)."""
    featured = dashboard_featured_keys(root)
    if not featured:
        return []
    # [OPUS-4.8] sq-5o5.2: ids explicitly flagged `featured = false` in
    # bench/benchmarks.toml are an intentional trend-only DISPOSITION (the design's
    # documented escape hatch, honored by gate G3) — they clear the dashboard-row
    # drift without a FEATURED_SUITES competitor card and without any perf claim.
    featured_false = featured_false_bench_ids(root)
    items: list[DriftItem] = []
    seen_families: set[str] = set()
    for bid in registered_bench_ids(root):
        # Family = the leading slug token (e.g. `zk` for `zk-commit-throughput`).
        family = bid.split("-", 1)[0]
        # Skip families that are NOT a capability dashboard surface:
        #   - internal harness/runner plumbing (cli-*, sparq-bench-*, hw-*, ci-*);
        #   - the SPIKES + external-cost suites the design (§5.D) explicitly
        #     accepts as unfeatured: serve/memtier spikes, the external-cost
        #     wikidata suites, and the raw parse/compress micro-benches that feed
        #     the ingest harness rather than standing as a capability card.
        if family in DASHBOARD_EXEMPT_FAMILIES or bid.startswith("sparq-bench"):
            continue
        if family in seen_families:
            continue
        # An explicit `featured = false` on THIS bench's entry is a dashboard
        # disposition (trend-only) — it clears the drift like a FEATURED_SUITES
        # row would, with no perf claim. We do NOT add the family to seen_families
        # here (matching the featured-row skip below), so a sibling id in the same
        # family that is NOT dispositioned can still surface the gap.
        if bid in featured_false:
            continue
        # Featured iff the family token or the full id appears in the block.
        if family in featured or bid in featured:
            continue
        seen_families.add(family)
        key = f"dashboard-row:{family}"
        items.append(
            DriftItem(
                drift_class="dashboard-row",
                dedup_key=key,
                title=f"drift: bench family `{family}` has no dashboard row",
                body=(
                    f"Registered benchmark(s) in the `{family}` family (e.g. `{bid}`) have no "
                    f"`FEATURED_SUITES` row in `bench/dashboard/dashboard.js` while other "
                    f"capability surfaces are promoted (design §5.D).\n\n"
                    f"Follow-on: add a `FEATURED_SUITES` entry for the `{family}` suite, or flag "
                    f"it `featured = false` in its `bench/benchmarks.toml` entry if it is "
                    f"intentionally unpromoted."
                ),
            )
        )
    return items


def consolidated_conformance_crates(root: Path) -> set[str]:
    """[OPUS-4.8] sq-ncvq.16 — crates whose crate-local conformance ratchet is now
    registered in the central scoreboard registry
    (crates/sparq-conformance/src/scoreboard.rs `SUITES`). Parsed textually from the
    `Runner::CrateTest { krate: "<crate>", ... }` rows so adding a crate to the
    registry auto-exempts it here; falls back to the literal set if the source is
    unreadable (e.g. a fixture tree without the registry)."""
    reg = root / "crates" / "sparq-conformance" / "src" / "scoreboard.rs"
    try:
        text = reg.read_text(encoding="utf-8")
    except OSError:
        return set(CONFORMANCE_CONSOLIDATED_FALLBACK)
    found = set(re.findall(r'CrateTest\s*\{\s*krate:\s*"([^"]+)"', text))
    # Belt-and-braces: never report LESS coverage than the known fallback.
    return found | set(CONFORMANCE_CONSOLIDATED_FALLBACK) if found else set(
        CONFORMANCE_CONSOLIDATED_FALLBACK
    )


def scan_conformance_split(root: Path) -> list[DriftItem]:
    """§5.E — conformance ratchets living OUTSIDE the central sparq-conformance
    scoreboard. We detect crate-local `tests/*.rs` files whose names mark them as
    a conformance/compliance ratchet (w3c_*, ogc_*, *compliance*, *conformance*)
    in any crate OTHER than sparq-conformance — EXCEPT crates whose ratchet is now
    consolidated INTO the central scoreboard registry (sq-ncvq.16: the SHACL + geo
    ratchets are reported there alongside SPARQL/inference, so they are no longer a
    split even though their runners stay crate-local)."""
    items: list[DriftItem] = []
    crates_dir = root / "crates"
    if not crates_dir.is_dir():
        return items
    consolidated = consolidated_conformance_crates(root)
    name_pat = re.compile(
        r"(w3c[_-]|ogc[_-]|compliance|conformance)", re.IGNORECASE
    )
    for crate in list_crates(root):
        if crate == "sparq-conformance":
            continue  # the central scoreboard itself
        if crate in consolidated:
            continue  # ratchet registered in the central scoreboard (sq-ncvq.16)
        tests = crates_dir / crate / "tests"
        if not tests.is_dir():
            continue
        hits = sorted(
            p.name
            for p in tests.iterdir()
            if p.is_file() and p.suffix == ".rs" and name_pat.search(p.name)
        )
        if not hits:
            continue
        key = f"conformance-split:{crate}"
        joined = ", ".join(f"`{h}`" for h in hits)
        items.append(
            DriftItem(
                drift_class="conformance-split",
                dedup_key=key,
                title=f"drift: crate `{crate}` carries conformance ratchets outside the central scoreboard",
                body=(
                    f"`crates/{crate}/tests/` holds crate-local conformance/compliance ratchet "
                    f"test(s) ({joined}) that live OUTSIDE the central `sparq-conformance` "
                    f"scoreboard, which therefore under-reports the project's real conformance "
                    f"surface (design §5.E).\n\n"
                    f"Follow-on: either surface `{crate}`'s ratchet in the central conformance "
                    f"report, or document the split intentionally (a note in the conformance "
                    f"report / FINDINGS.md explaining why it stays crate-local)."
                ),
            )
        )
    return items


def _parse_stamp(stamp: object) -> datetime | None:
    """One record stamp as a timezone-aware datetime, or `None` if unusable.

    Tolerant by design (this is an audit, not a validator): anything that is not
    a parsable ISO-8601 string is simply not evidence. A naive stamp is read as
    UTC so every candidate is directly comparable — bd writes Zulu, but a stamp
    carrying a real offset must still order by INSTANT against it."""
    if not isinstance(stamp, str):
        return None
    try:
        parsed = datetime.fromisoformat(stamp.replace("Z", "+00:00"))
    except ValueError:
        return None
    return parsed if parsed.tzinfo is not None else parsed.replace(tzinfo=timezone.utc)


def beads_export_records(root: Path) -> tuple[set[str], str | None]:
    """Parse `.beads/issues.jsonl` -> (bead ids present, newest ISO-8601 stamp).

    Returns `(set(), None)` when the export is absent (fixture repos, a checkout
    without `.beads/`) so the scanner degrades to a no-op rather than a false
    positive. Malformed lines are skipped, not fatal: this is an audit, and a
    half-parsable export is still evidence about the half that parsed — and that
    tolerance extends to the stamps themselves, which are compared as parsed
    instants rather than raw strings (see the loop below)."""
    path = root / BEADS_EXPORT_REL
    ids: set[str] = set()
    newest: str | None = None
    newest_at: datetime | None = None
    try:
        text = path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return ids, None
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        if not isinstance(rec, dict):
            continue
        bead_id = rec.get("id")
        if isinstance(bead_id, str):
            ids.add(bead_id)
        for field in ("created_at", "updated_at", "closed_at"):
            stamp = rec.get(field)
            # Compare PARSED instants, never the raw strings: a lexicographic
            # max() lets one malformed field (`zzzz`) outrank every valid stamp
            # and then fail to parse downstream, so a single junk value would
            # silently destroy the whole age proxy. Unparsable candidates are
            # skipped here instead; the raw winner is kept for reporting.
            parsed = _parse_stamp(stamp)
            if parsed is not None and (newest_at is None or parsed > newest_at):
                newest_at, newest = parsed, stamp
    return ids, newest


def repo_bead_references(root: Path) -> set[str]:
    """Every `sq-…` bead id referenced by a TRACKED-ish text file in the tree.

    This is the git-visible proxy for "which beads exist" that CI can compute
    WITHOUT the bd Dolt DB (which is gitignored and lives only in the
    orchestrator's checkout — see WHY GITHUB ISSUES, NOT `bd create` above).
    Deliberately excludes `.beads/` itself so the export cannot corroborate
    itself. Bounded by suffix + size + directory (see REF_SCAN_* above)."""
    refs: set[str] = set()
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in REF_SCAN_SKIP_DIRS]
        for name in filenames:
            if Path(name).suffix not in REF_SCAN_SUFFIXES:
                continue
            fp = Path(dirpath) / name
            try:
                if fp.stat().st_size > REF_SCAN_MAX_BYTES:
                    continue
                text = fp.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue
            refs.update(BEAD_ID_RE.findall(text))
    return refs


def export_age_days(newest: str | None, now: datetime | None = None) -> float | None:
    """Whole-ish days between the export's newest record stamp and `now`.

    `None` when there is no parsable stamp. `now` is injectable so the tests are
    deterministic (the scanner otherwise reads the wall clock)."""
    parsed = _parse_stamp(newest)
    if parsed is None:
        return None
    now = now or datetime.now(timezone.utc)
    if now.tzinfo is None:
        now = now.replace(tzinfo=timezone.utc)
    return (now - parsed).total_seconds() / 86400.0


def scan_beads_export_stale(root: Path, now: datetime | None = None) -> list[DriftItem]:
    """sq-0b0sh — `.beads/issues.jsonl` has fallen behind the live bd DB.

    `.beads/issues.jsonl` is the ONLY git-visible / cross-checkout view of the
    backlog: every agent, digest and program review that cannot reach the
    orchestrator's Dolt DB reads it instead. When the export is not re-run, that
    view silently reports a backlog months out of date — the 2026-07 Fable status
    digest wrongly flagged already-closed survey items as unbeaded gaps for
    exactly this reason.

    CI CANNOT DIFF AGAINST THE LIVE DB (it is gitignored and local to the
    orchestrator), so this scanner measures the two proxies it CAN compute
    hermetically from the checkout:

      1. AGE — how old the newest record stamp in the export is. This is a LOWER
         bound on staleness: a re-export with no intervening bead mutation would
         not move it. Accepted, because beads in this repo mutate ~daily, so in
         practice the newest stamp tracks the last export closely.
      2. MISSING REFERENCES — bead ids the repo itself cites (in research
         records, code comments, workflows, SKILLs) that have NO record in the
         export. This is an UPPER bound on the true gap: a bead that was closed
         and later compacted/gc'd out of the DB is cited but legitimately absent.
         Hence the threshold below — a handful of these is normal; scores of them
         mean the export genuinely lags the DB.

    Either proxy tripping files ONE item (a single stable dedup key), because
    both have the same one fix: re-run `bd export` onto a `chore-beads-resync-*`
    branch, and automate it (a post-mutation hook or a scheduler-tick step) so it
    does not drift again. This scanner NEVER touches `bd` or `.beads/` — it only
    reads the committed export, per the standing no-hand-edits rule."""
    ids, newest = beads_export_records(root)
    if not ids:
        # No export in this tree (fixture / partial checkout): nothing to assert.
        return []

    age = export_age_days(newest, now)
    missing = sorted(r for r in repo_bead_references(root) if r not in ids)

    stale = age is not None and age > BEADS_EXPORT_STALE_DAYS
    orphaned = len(missing) > BEADS_MISSING_REF_THRESHOLD
    if not (stale or orphaned):
        return []

    age_line = (
        f"- newest record stamp: `{newest}` — **{age:.0f} days old** "
        f"(threshold: {BEADS_EXPORT_STALE_DAYS} days)"
        if age is not None
        else "- newest record stamp: *unparsable* — the export carries no usable timestamp"
    )
    sample = ", ".join(f"`{m}`" for m in missing[:20])
    return [
        DriftItem(
            drift_class="beads-export-stale",
            dedup_key="beads-export-stale:issues-jsonl",
            title="drift: `.beads/issues.jsonl` export has fallen behind the live bd DB",
            body=(
                f"`{BEADS_EXPORT_REL}` is the only git-visible view of the backlog, and it "
                f"reads as stale. Measured from this checkout ({len(ids)} exported "
                f"bead records):\n\n"
                f"{age_line}\n"
                f"- bead ids cited elsewhere in the repo with NO record in the export: "
                f"**{len(missing)}** (threshold: {BEADS_MISSING_REF_THRESHOLD})\n\n"
                f"Sample of cited-but-absent ids: {sample or '(none)'}\n\n"
                f"Both numbers are PROXIES — CI has no access to the bd Dolt DB (it is "
                f"gitignored and lives only in the orchestrator's checkout), so this lane "
                f"cannot diff the export against the real backlog. The age is a LOWER bound "
                f"on staleness (a re-export with no bead mutation would not move it); the "
                f"missing-reference count is an UPPER bound on the real gap (a bead that was "
                f"closed and later compacted out of the DB is cited but legitimately absent).\n\n"
                f"Why it matters: any agent, digest or program review that reads the export "
                f"instead of the DB gets a materially wrong picture of the backlog.\n\n"
                f"Follow-on: re-run `bd export` onto a dedicated `chore-beads-resync-*` branch "
                f"(never folded into a feature branch — AGENTS.md *Merge discipline*), and "
                f"automate the refresh so it cannot drift again: a post-mutation `bd` hook, or "
                f"an export step on the orchestrator's scheduler tick. Do NOT hand-edit "
                f"`{BEADS_EXPORT_REL}` (standing rule) — `bd export` regenerates it."
            ),
        )
    ]


SCANNERS = (
    scan_bench_missing,
    scan_skill_missing,
    scan_explain_asymmetry,
    scan_dashboard_row,
    scan_conformance_split,
    scan_beads_export_stale,
)


def scan_all(root: Path) -> list[DriftItem]:
    items: list[DriftItem] = []
    for scanner in SCANNERS:
        items.extend(scanner(root))
    return items


# --------------------------------------------------------------------------- #
# CI guard (bead sq-z0se)
# --------------------------------------------------------------------------- #
# [OPUS-4.8] An accidental dev-box run of this scanner minted 20 spurious GitHub
# issues (#397-416). This guard makes the ISSUE-MINTING path refuse to run
# anywhere but CI: it gates on the standard CI markers (`GITHUB_ACTIONS=true`,
# set by GitHub Actions, or a truthy `CI`). It is checked ONLY on the write path
# — `--dry-run` (which makes zero `gh` calls) stays runnable anywhere, so local
# audits and the hermetic test-suite are unaffected. An explicit
# `DRIFT_SCAN_ALLOW_LOCAL=1` escape hatch exists for the rare deliberate manual
# mint, so the guard is a guard-rail, not a hard wall.
CI_ENV_VARS = ("GITHUB_ACTIONS", "CI")
ALLOW_LOCAL_ENV_VAR = "DRIFT_SCAN_ALLOW_LOCAL"


def _is_truthy(val: str | None) -> bool:
    return val is not None and val.strip().lower() in {"1", "true", "yes", "on"}


def running_in_ci(env: dict[str, str] | None = None) -> bool:
    """True iff a recognised CI marker is set (GITHUB_ACTIONS or CI)."""
    e = os.environ if env is None else env
    return any(_is_truthy(e.get(v)) for v in CI_ENV_VARS)


def require_ci(env: dict[str, str] | None = None) -> None:
    """Refuse to mint issues outside CI (bead sq-z0se).

    Raises SystemExit(2) with a clear message unless a CI marker is set or the
    explicit `DRIFT_SCAN_ALLOW_LOCAL` escape hatch is truthy. Only the write
    path calls this — `--dry-run` never does."""
    e = os.environ if env is None else env
    if running_in_ci(e) or _is_truthy(e.get(ALLOW_LOCAL_ENV_VAR)):
        return
    print(
        "drift-scan: refusing to file GitHub issues outside CI "
        f"(no {' / '.join(CI_ENV_VARS)} env var set). This guard exists because "
        "an accidental dev-box run minted 20 spurious issues (#397-416, bead "
        "sq-z0se). Use --dry-run to preview the drift report locally, or set "
        f"{ALLOW_LOCAL_ENV_VAR}=1 only if you really intend to file issues from here.",
        file=sys.stderr,
    )
    raise SystemExit(2)


# --------------------------------------------------------------------------- #
# gh I/O (mirrors flow-on.py exactly so the two halves behave identically)
# --------------------------------------------------------------------------- #
def key_marker(dedup_key: str) -> str:
    return f"<!-- drift-key: {dedup_key} -->"


def _gh(args: list[str]) -> str:
    proc = subprocess.run(["gh", *args], capture_output=True, text=True, check=True)
    return proc.stdout


def open_issue_exists(dedup_key: str) -> bool:
    marker = key_marker(dedup_key)
    # Search by the punctuation-light substring `drift-key: <key>` (GitHub's
    # tokeniser handles `<!--`/`-->` unreliably); verify the full marker against
    # each returned body so the dedup decision stays exact (cf. flow-on.py).
    search = f"drift-key: {dedup_key}"
    out = _gh(
        [
            "issue",
            "list",
            "--state",
            "open",
            "--label",
            "drift",
            "--search",
            search,
            "--json",
            "number,body",
            "--limit",
            "100",
        ]
    )
    for issue in json.loads(out):
        if marker in (issue.get("body") or ""):
            return True
    return False


_ENSURED_LABELS: set[str] = set()


def ensure_label(label: str) -> None:
    """Idempotently ensure `label` exists before it is applied (gh issue create
    ERRORS on an unknown label). Best-effort upsert; create_issue reports real
    failures. Mirrors flow-on.py::ensure_label."""
    if label in _ENSURED_LABELS:
        return
    try:
        _gh(["label", "create", label, "--force"])
    except Exception:  # noqa: BLE001 - best-effort; create_issue surfaces real errors
        pass
    _ENSURED_LABELS.add(label)


def build_issue_body(item: DriftItem) -> str:
    body = item.body.rstrip() + "\n\n" + key_marker(item.dedup_key)
    body += (
        f"\n\n> 🤖 SPARQ agent — auto-generated by the drift scanner "
        f"(class `{item.drift_class}`, scripts/drift-scan.py). Reconcile into a bead."
    )
    return body


def create_issue(item: DriftItem) -> str:
    args = ["issue", "create", "--title", item.title, "--body", build_issue_body(item)]
    for lbl in BASE_LABELS:
        ensure_label(lbl)
        args += ["--label", lbl]
    return _gh(args).strip()


# --------------------------------------------------------------------------- #
# Reporting
# --------------------------------------------------------------------------- #
def print_report(items: list[DriftItem]) -> None:
    by_class: dict[str, list[DriftItem]] = {}
    for it in items:
        by_class.setdefault(it.drift_class, []).append(it)
    if not items:
        print("drift-scan: no drift detected.")
        return
    print(f"drift-scan: {len(items)} drift item(s) across {len(by_class)} class(es).\n")
    for cls in (s.__name__.replace("scan_", "").replace("_", "-") for s in SCANNERS):
        bucket = by_class.get(cls, [])
        if not bucket:
            continue
        print(f"## {cls} ({len(bucket)})")
        for it in bucket:
            print(f"  - [{it.dedup_key}] {it.title}")
        print()


def write_json(items: list[DriftItem], path: Path) -> None:
    payload = {
        "drift_count": len(items),
        "items": [it.to_json() for it in items],
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Periodic repo-wide drift scanner.")
    ap.add_argument("--root", default=str(REPO_ROOT), help="repo root to scan")
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="print the drift report; make NO gh writes",
    )
    ap.add_argument("--json", dest="json_out", help="also write a machine-readable JSON report here")
    args = ap.parse_args(argv)

    root = Path(args.root).resolve()
    items = scan_all(root)

    print_report(items)
    if args.json_out:
        write_json(items, Path(args.json_out))
        print(f"drift-scan: wrote JSON report to {args.json_out}")

    if args.dry_run:
        return 0

    # Issue-minting path: refuse outside CI (bead sq-z0se).
    require_ci()

    created = 0
    skipped = 0
    for it in items:
        if open_issue_exists(it.dedup_key):
            print(f"drift-scan: skip (open issue exists) key={it.dedup_key}")
            skipped += 1
            continue
        url = create_issue(it)
        print(f"drift-scan: created {url} (class={it.drift_class}, key={it.dedup_key})")
        created += 1

    print(f"drift-scan: done — {created} created, {skipped} skipped (already open).")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as fh:
            fh.write("### drift-scan\n\n")
            fh.write(f"- drift items: {len(items)}\n")
            fh.write(f"- issues created: {created}\n")
            fh.write(f"- skipped (already open): {skipped}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
