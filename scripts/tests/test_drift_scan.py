#!/usr/bin/env python3
# [OPUS-4.8] Hermetic tests for the periodic drift scanner (bead sq-ncvq.11,
# epic sq-ncvq). Authored by Opus 4.8 (Fable unavailable; flag for re-review
# when Fable returns).
#
# Fully hermetic: imports scripts/drift-scan.py and runs each scanner against
# FIXTURE repo layouts built in a tmp dir — NO live `gh` / `git` / network. We
# exercise the scanners directly + main(--dry-run) (which makes no gh calls) +
# the dedup-key stability contract.
#
# Run:  python3 scripts/tests/test_drift_scan.py
# (stdlib only; no pytest required — mirrors test_flow_on.py.)

from __future__ import annotations

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
DRIFT_SCAN = REPO_ROOT / "scripts" / "drift-scan.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("drift_scan", DRIFT_SCAN)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["drift_scan"] = mod
    spec.loader.exec_module(mod)
    return mod


drift_scan = _load_module()


# --------------------------------------------------------------------------- #
# Fixture-repo builder
# --------------------------------------------------------------------------- #
def _write(root: Path, rel: str, content: str = "") -> None:
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content, encoding="utf-8")


def make_crate(root: Path, name: str, *, public: bool = True) -> None:
    body = f'[package]\nname = "{name}"\n'
    if not public:
        body += "publish = false\n"
    _write(root, f"crates/{name}/Cargo.toml", body)


def make_registry(
    root: Path,
    sources: list[str],
    ids: list[str] | None = None,
    featured_false: set[str] | None = None,
) -> None:
    """Write a minimal bench/benchmarks.toml with the given `source` lines (and
    optionally `id` lines for the dashboard scanner). Any id in `featured_false`
    gets a `featured = false` line in its block (sq-5o5.2 dashboard disposition)."""
    ids = ids or []
    featured_false = featured_false or set()
    lines = []
    n = max(len(sources), len(ids))
    for i in range(n):
        lines.append("[[benchmark]]")
        if i < len(ids):
            lines.append(f'id          = "{ids[i]}"')
            if ids[i] in featured_false:
                lines.append("featured    = false")
        if i < len(sources):
            lines.append(f'source      = "{sources[i]}"')
        lines.append("")
    _write(root, "bench/benchmarks.toml", "\n".join(lines))


def make_dashboard(root: Path, featured_tokens: list[str]) -> None:
    aliases = ", ".join(f"'{t}'" for t in featured_tokens)
    js = (
        "var FEATURED_SUITES = [\n"
        f"  {{ key: 'X', title: 'X', aliases: [{aliases}] }}\n"
        "];\n"
    )
    _write(root, "bench/dashboard/dashboard.js", js)


def make_skill(root: Path, surface: str, text: str) -> None:
    _write(root, f"skills/{surface}/SKILL.md", text)


# --------------------------------------------------------------------------- #
# bench-missing (§5.A)
# --------------------------------------------------------------------------- #
class BenchMissingTest(unittest.TestCase):
    def _root(self) -> Path:
        root = Path(self._tmp.name)
        make_crate(root, "sparq-nlq")
        make_crate(root, "sparq-engine")  # exempt: query stack
        make_crate(root, "sparq-bench")  # exempt: the harness itself
        # registry references engine (via cli harness) but NOT nlq.
        make_registry(root, sources=["crates/sparq-cli/src/main.rs"])
        return root

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def test_flags_unbenched_crate(self):
        items = drift_scan.scan_bench_missing(self._root())
        keys = {it.dedup_key for it in items}
        self.assertIn("bench-missing:sparq-nlq", keys)

    def test_does_not_flag_exempt_query_stack(self):
        items = drift_scan.scan_bench_missing(self._root())
        keys = {it.dedup_key for it in items}
        self.assertNotIn("bench-missing:sparq-engine", keys)
        self.assertNotIn("bench-missing:sparq-bench", keys)

    def test_registered_crate_not_flagged(self):
        root = Path(self._tmp.name)
        make_crate(root, "sparq-geo")
        make_registry(root, sources=["bench/geo/run.sh; crates/sparq-geo/examples/bench_geo.rs"])
        items = drift_scan.scan_bench_missing(root)
        self.assertEqual([], [it for it in items if it.dedup_key == "bench-missing:sparq-geo"])

    # [OPUS-4.8] sq-bif.5: scan_bench_missing must honor the SAME `publish = false`
    # stub exemption that gate G1 (gate-new-crate.py) applies to its bench
    # requirement, so the merge-time gate and the reactive scanner cannot diverge
    # and mint spurious bench-missing drift for legitimately benchless stubs.
    def test_publish_false_crate_without_bench_not_flagged(self):
        # A `publish = false` stub with no registered bench is EXEMPT (mirrors G1).
        root = Path(self._tmp.name)
        make_crate(root, "sparq-fedstub", public=False)
        make_registry(root, sources=["crates/sparq-cli/src/main.rs"])  # does NOT ref the stub
        items = drift_scan.scan_bench_missing(root)
        keys = {it.dedup_key for it in items}
        self.assertNotIn("bench-missing:sparq-fedstub", keys)

    def test_publishable_crate_without_bench_still_flagged(self):
        # The exemption is NARROW: a publishable (no `publish = false`) crate with
        # no registered bench is STILL flagged — only stubs drop out.
        root = Path(self._tmp.name)
        make_crate(root, "sparq-pubcrate", public=True)
        make_registry(root, sources=["crates/sparq-cli/src/main.rs"])  # does NOT ref the crate
        items = drift_scan.scan_bench_missing(root)
        keys = {it.dedup_key for it in items}
        self.assertIn("bench-missing:sparq-pubcrate", keys)

    def test_stub_and_public_side_by_side(self):
        # Both in one tree: only the publishable benchless crate surfaces.
        root = Path(self._tmp.name)
        make_crate(root, "sparq-pubcrate", public=True)
        make_crate(root, "sparq-fedstub", public=False)
        make_registry(root, sources=["crates/sparq-cli/src/main.rs"])
        items = drift_scan.scan_bench_missing(root)
        keys = {it.dedup_key for it in items}
        self.assertIn("bench-missing:sparq-pubcrate", keys)
        self.assertNotIn("bench-missing:sparq-fedstub", keys)

    def test_exemption_predicate_matches_gate_g1(self):
        # Single-source-of-truth guard: the stub predicate scan_bench_missing uses
        # (crate_is_public is False) must be the EXACT inverse of G1's
        # gate_new_crate.crate_is_stub for the same Cargo.toml content. We assert on
        # the shared regex behaviour: a `publish = false` line => stub => exempt; a
        # bare `publish = true` (or no publish key) => not a stub => not exempt.
        root = Path(self._tmp.name)
        make_crate(root, "sparq-stub", public=False)   # writes `publish = false`
        make_crate(root, "sparq-open", public=True)    # no publish key
        self.assertFalse(drift_scan.crate_is_public(root, "sparq-stub"))
        self.assertTrue(drift_scan.crate_is_public(root, "sparq-open"))


# --------------------------------------------------------------------------- #
# skill-missing (§5.B)
# --------------------------------------------------------------------------- #
class SkillMissingTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def test_flags_public_crate_in_no_skill(self):
        make_crate(self.root, "sparq-orphan", public=True)
        make_crate(self.root, "sparq-known", public=True)
        make_skill(self.root, "known", "covers sparq-known nicely")
        items = drift_scan.scan_skill_missing(self.root)
        keys = {it.dedup_key for it in items}
        self.assertIn("skill-missing:sparq-orphan", keys)
        self.assertNotIn("skill-missing:sparq-known", keys)

    def test_private_crate_exempt(self):
        make_crate(self.root, "sparq-internal", public=False)
        # no skill names it, but it's private → not a public surface.
        items = drift_scan.scan_skill_missing(self.root)
        keys = {it.dedup_key for it in items}
        self.assertNotIn("skill-missing:sparq-internal", keys)

    def test_mention_anywhere_counts(self):
        make_crate(self.root, "sparq-sim", public=True)
        # named only in the top-level index skill — still counts as covered.
        make_skill(self.root, "", "index mentions sparq-sim")  # skills/SKILL.md
        items = drift_scan.scan_skill_missing(self.root)
        self.assertNotIn("skill-missing:sparq-sim", {it.dedup_key for it in items})


# --------------------------------------------------------------------------- #
# explain-asymmetry (§5.C)
# --------------------------------------------------------------------------- #
class ExplainAsymmetryTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def test_flags_when_rust_has_explain_but_wasm_does_not(self):
        _write(self.root, "crates/sparq-engine/src/explain.rs", "pub fn explain() {}")
        _write(self.root, "crates/sparq-server/src/http.rs", "// /explain route")
        _write(self.root, "crates/sparq-wasm/src/lib.rs", "pub fn query() {}")  # no explain
        items = drift_scan.scan_explain_asymmetry(self.root)
        self.assertEqual(["explain-asymmetry:wasm"], [it.dedup_key for it in items])

    def test_no_flag_when_wasm_exports_explain(self):
        _write(self.root, "crates/sparq-engine/src/explain.rs", "pub fn explain() {}")
        _write(self.root, "crates/sparq-wasm/src/lib.rs", "pub fn explain() {}")
        items = drift_scan.scan_explain_asymmetry(self.root)
        self.assertEqual([], items)

    def test_no_flag_when_rust_lacks_explain(self):
        _write(self.root, "crates/sparq-wasm/src/lib.rs", "pub fn query() {}")
        items = drift_scan.scan_explain_asymmetry(self.root)
        self.assertEqual([], items)


# --------------------------------------------------------------------------- #
# dashboard-row (§5.D)
# --------------------------------------------------------------------------- #
class DashboardRowTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def test_flags_unfeatured_family(self):
        make_registry(
            self.root,
            sources=["s", "s"],
            ids=["zk-commit-throughput", "lubm"],
        )
        make_dashboard(self.root, featured_tokens=["lubm"])  # zk NOT featured
        items = drift_scan.scan_dashboard_row(self.root)
        keys = {it.dedup_key for it in items}
        self.assertIn("dashboard-row:zk", keys)
        self.assertNotIn("dashboard-row:lubm", keys)

    def test_family_dedup(self):
        # two zk benches → ONE dashboard-row:zk item.
        make_registry(
            self.root,
            sources=["s", "s"],
            ids=["zk-commit-throughput", "zk-compose-gates"],
        )
        make_dashboard(self.root, featured_tokens=["lubm"])
        items = drift_scan.scan_dashboard_row(self.root)
        zk = [it for it in items if it.dedup_key == "dashboard-row:zk"]
        self.assertEqual(1, len(zk))

    def test_exempt_families_skipped(self):
        make_registry(
            self.root,
            sources=["s", "s"],
            ids=["serve-spikes", "cli-bench-suite"],
        )
        make_dashboard(self.root, featured_tokens=["lubm"])
        items = drift_scan.scan_dashboard_row(self.root)
        self.assertEqual([], items)

    # [OPUS-4.8] sq-5o5.2: a `featured = false` entry is an intentional trend-only
    # DISPOSITION that clears the dashboard-row drift WITHOUT a FEATURED_SUITES
    # competitor card and WITHOUT any perf claim. drift-scan must honor it so it
    # stays in lock-step with the merge-time gate G3 (check-new-bench-registered.py),
    # which already accepts the same flag.
    def test_featured_false_clears_drift(self):
        make_registry(
            self.root,
            sources=["s", "s"],
            ids=["gpu-bench", "lubm"],
            featured_false={"gpu-bench"},  # gpu marked trend-only
        )
        make_dashboard(self.root, featured_tokens=["lubm"])  # gpu NOT in FEATURED_SUITES
        items = drift_scan.scan_dashboard_row(self.root)
        keys = {it.dedup_key for it in items}
        self.assertNotIn("dashboard-row:gpu", keys)

    def test_featured_false_collects_helper_ids(self):
        # The helper must report exactly the ids whose block carries featured=false.
        make_registry(
            self.root,
            sources=["s", "s", "s"],
            ids=["gpu-bench", "operator-coverage", "lubm"],
            featured_false={"gpu-bench", "operator-coverage"},
        )
        self.assertEqual(
            {"gpu-bench", "operator-coverage"},
            drift_scan.featured_false_bench_ids(self.root),
        )

    def test_featured_false_clears_multi_id_family(self):
        # A multi-id family is only cleared when EVERY non-exempt id is
        # dispositioned (the bead marks all 4 inference ids); marking only one
        # leaves the sibling to surface the gap, matching the FEATURED_SUITES skip.
        make_registry(
            self.root,
            sources=["s", "s"],
            ids=["inference-eye-comparison", "inference-owl-bench"],
            featured_false={"inference-eye-comparison"},  # only ONE of two
        )
        make_dashboard(self.root, featured_tokens=["lubm"])
        keys = {it.dedup_key for it in drift_scan.scan_dashboard_row(self.root)}
        self.assertIn("dashboard-row:inference", keys)  # sibling still flags it
        # Now disposition BOTH → family clears.
        make_registry(
            self.root,
            sources=["s", "s"],
            ids=["inference-eye-comparison", "inference-owl-bench"],
            featured_false={"inference-eye-comparison", "inference-owl-bench"},
        )
        make_dashboard(self.root, featured_tokens=["lubm"])
        keys = {it.dedup_key for it in drift_scan.scan_dashboard_row(self.root)}
        self.assertNotIn("dashboard-row:inference", keys)


# --------------------------------------------------------------------------- #
# conformance-split (§5.E)
# --------------------------------------------------------------------------- #
class ConformanceSplitTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def _register_in_scoreboard(self, *crates: str) -> None:
        """[OPUS-4.8] sq-ncvq.16 — write a minimal scoreboard registry whose
        `Runner::CrateTest { krate: "<crate>" }` rows mark the given crates as
        CONSOLIDATED into the central scoreboard (so they're no longer a split)."""
        rows = "\n".join(
            f'    Runner::CrateTest {{ krate: "{c}", target: "t" }},' for c in crates
        )
        _write(
            self.root,
            "crates/sparq-conformance/src/scoreboard.rs",
            f"pub const SUITES: &[Suite] = &[\n{rows}\n];\n",
        )

    def test_flags_unconsolidated_crate_local_ratchet(self):
        # A crate with a conformance-named ratchet that is NOT registered in the
        # central scoreboard IS still a split.
        make_crate(self.root, "sparq-other")
        _write(self.root, "crates/sparq-other/tests/w3c_thing.rs", "// ratchet")
        items = drift_scan.scan_conformance_split(self.root)
        keys = {it.dedup_key for it in items}
        self.assertIn("conformance-split:sparq-other", keys)

    def test_consolidated_crates_not_flagged(self):
        # sq-ncvq.16: SHACL + geo ratchets are registered in the central scoreboard
        # registry, so they are NO LONGER a conformance-split even though their
        # runners stay crate-local.
        make_crate(self.root, "sparq-shacl")
        _write(self.root, "crates/sparq-shacl/tests/w3c_core.rs", "// ratchet")
        make_crate(self.root, "sparq-geo")
        _write(self.root, "crates/sparq-geo/tests/ogc_compliance_ratchet.rs", "// ratchet")
        self._register_in_scoreboard("sparq-shacl", "sparq-geo")
        items = drift_scan.scan_conformance_split(self.root)
        keys = {it.dedup_key for it in items}
        self.assertNotIn("conformance-split:sparq-shacl", keys)
        self.assertNotIn("conformance-split:sparq-geo", keys)

    def test_fallback_exempts_known_crates_without_registry(self):
        # Even with no registry source present, the literal fallback exempts the
        # known consolidated crates (so a fixture tree without the registry behaves).
        make_crate(self.root, "sparq-shacl")
        _write(self.root, "crates/sparq-shacl/tests/w3c_core.rs", "// ratchet")
        items = drift_scan.scan_conformance_split(self.root)
        self.assertNotIn(
            "conformance-split:sparq-shacl", {it.dedup_key for it in items}
        )

    def test_central_scoreboard_exempt(self):
        make_crate(self.root, "sparq-conformance")
        _write(self.root, "crates/sparq-conformance/tests/conformance_smoke.rs", "// ok")
        items = drift_scan.scan_conformance_split(self.root)
        self.assertEqual([], items)

    def test_non_conformance_tests_ignored(self):
        make_crate(self.root, "sparq-shacl")
        _write(self.root, "crates/sparq-shacl/tests/constraints.rs", "// unit test")
        items = drift_scan.scan_conformance_split(self.root)
        self.assertEqual([], items)


# --------------------------------------------------------------------------- #
# beads-export-stale (bead sq-0b0sh)
# --------------------------------------------------------------------------- #
NOW = datetime(2026, 8, 3, tzinfo=timezone.utc)
KEY = "beads-export-stale:issues-jsonl"


def make_beads_export(root: Path, records: list[dict], *, raw_extra: str = "") -> None:
    """Write a `.beads/issues.jsonl` fixture from bead records (+ optional raw
    trailing text, used to exercise malformed-line tolerance)."""
    body = "".join(json.dumps({"_type": "issue", **r}) + "\n" for r in records)
    _write(root, ".beads/issues.jsonl", body + raw_extra)


def bead(bead_id: str, stamp: str = "2026-08-01T00:00:00Z") -> dict:
    return {"id": bead_id, "status": "open", "created_at": stamp, "updated_at": stamp}


class BeadsExportStaleTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def _keys(self, now=NOW) -> set[str]:
        return {it.dedup_key for it in drift_scan.scan_beads_export_stale(self.root, now)}

    # -- the two triggers ---------------------------------------------------- #
    def test_fresh_and_complete_export_is_not_flagged(self):
        # Newest stamp 2 days old, and every id the repo cites is exported.
        make_beads_export(self.root, [bead("sq-aaa1"), bead("sq-bbb2")])
        _write(self.root, "research/notes.md", "see sq-aaa1 and sq-bbb2")
        self.assertEqual(set(), self._keys())

    def test_stale_stamp_is_flagged(self):
        # Freshness trigger ALONE: no missing references at all, but the newest
        # record stamp is well past BEADS_EXPORT_STALE_DAYS.
        make_beads_export(self.root, [bead("sq-aaa1", "2026-06-01T00:00:00Z")])
        _write(self.root, "research/notes.md", "see sq-aaa1")
        self.assertEqual({KEY}, self._keys())

    def test_many_cited_but_unexported_ids_are_flagged(self):
        # Missing-reference trigger ALONE: the stamp is fresh, but the repo cites
        # far more beads than the export knows about — the sq-0b0sh symptom.
        make_beads_export(self.root, [bead("sq-aaa1")])
        cited = " ".join(f"sq-m{n:04d}" for n in range(60))
        _write(self.root, "research/notes.md", f"see sq-aaa1 {cited}")
        self.assertEqual({KEY}, self._keys())

    def test_a_few_missing_ids_stay_under_threshold(self):
        # The threshold is NOT zero: compacted-out closed beads stay cited in
        # research records forever, and must not trip the check on their own.
        make_beads_export(self.root, [bead("sq-aaa1")])
        cited = " ".join(f"sq-m{n:04d}" for n in range(5))
        _write(self.root, "research/notes.md", f"see sq-aaa1 {cited}")
        self.assertEqual(set(), self._keys())

    def test_absent_export_is_a_no_op(self):
        # A tree with no `.beads/` (fixture repo, partial checkout) must produce
        # NO drift rather than a false positive — this is what keeps the existing
        # clean-repo test green.
        _write(self.root, "research/notes.md", "sq-aaa1 sq-bbb2 sq-ccc3")
        self.assertEqual(set(), self._keys())

    # -- reference-scan semantics -------------------------------------------- #
    def test_export_cannot_corroborate_itself(self):
        # Ids cited only from inside `.beads/` are NOT counted as references: the
        # export must not vouch for its own completeness.
        make_beads_export(self.root, [bead("sq-aaa1")])
        cited = " ".join(f"sq-m{n:04d}" for n in range(60))
        _write(self.root, ".beads/sidecar.md", cited)
        self.assertEqual(set(), self._keys())

    def test_reference_regex_accepts_sub_beads_and_rejects_hyphenated_tokens(self):
        # `sq-pbz04.7` is a real sub-bead id and must be seen; `sq-bench-adapters`
        # is a PATH, and a bare \b boundary would truncate it to a phantom
        # `sq-bench` id, inflating the missing count.
        found = set(drift_scan.BEAD_ID_RE.findall("sq-pbz04.7 sq-bench-adapters sq-aaa1"))
        self.assertEqual({"sq-pbz04.7", "sq-aaa1"}, found)

    def test_repo_bead_references_honours_suffix_and_dir_bounds(self):
        _write(self.root, "research/notes.md", "sq-aaa1")
        _write(self.root, "crates/x/src/lib.rs", "// sq-bbb2")
        _write(self.root, "node_modules/pkg/index.js", "sq-ccc3")  # skipped dir
        _write(self.root, "assets/blob.bin", "sq-ddd4")  # skipped suffix
        self.assertEqual({"sq-aaa1", "sq-bbb2"}, drift_scan.repo_bead_references(self.root))

    # -- parsing robustness --------------------------------------------------- #
    def test_malformed_lines_do_not_abort_the_parse(self):
        make_beads_export(
            self.root, [bead("sq-aaa1")], raw_extra="not json at all\n\n[1,2,3]\n"
        )
        ids, newest = drift_scan.beads_export_records(self.root)
        self.assertEqual({"sq-aaa1"}, ids)
        self.assertEqual("2026-08-01T00:00:00Z", newest)

    def test_newest_stamp_is_the_max_across_records_and_fields(self):
        make_beads_export(
            self.root,
            [bead("sq-aaa1", "2026-01-02T00:00:00Z"), bead("sq-bbb2", "2026-05-06T00:00:00Z")],
        )
        _, newest = drift_scan.beads_export_records(self.root)
        self.assertEqual("2026-05-06T00:00:00Z", newest)

    def test_a_malformed_stamp_cannot_outrank_a_valid_one(self):
        # `zzzz` sorts ABOVE every ISO-8601 stamp lexicographically but parses to
        # nothing, so a raw-string max() would elect it and then report the age
        # as unavailable — suppressing the stale finding on an export that is
        # provably months old. The valid stamp must win instead.
        make_beads_export(
            self.root,
            [bead("sq-aaa1", "2026-06-01T00:00:00Z"), bead("sq-bbb2", "zzzz")],
        )
        _, newest = drift_scan.beads_export_records(self.root)
        self.assertEqual("2026-06-01T00:00:00Z", newest)

    def test_stale_still_fires_when_a_malformed_stamp_is_present(self):
        # End to end, with NO missing references, so the freshness proxy is the
        # only thing that can trip: one junk stamp must not poison the evidence.
        make_beads_export(
            self.root,
            [bead("sq-aaa1", "2026-06-01T00:00:00Z"), bead("sq-bbb2", "zzzz")],
        )
        _write(self.root, "research/notes.md", "see sq-aaa1 and sq-bbb2")
        self.assertEqual({KEY}, self._keys())

    def test_newest_stamp_orders_by_instant_across_offsets(self):
        # `2026-08-01T05:00:00+10:00` is 2026-07-31T19:00Z — EARLIER than
        # `2026-08-01T00:00:00Z`, yet the LARGER of the two as raw text. Only
        # instant-wise comparison picks the right record.
        make_beads_export(
            self.root,
            [
                bead("sq-aaa1", "2026-08-01T05:00:00+10:00"),
                bead("sq-bbb2", "2026-08-01T00:00:00Z"),
            ],
        )
        _, newest = drift_scan.beads_export_records(self.root)
        self.assertEqual("2026-08-01T00:00:00Z", newest)

    def test_export_age_days(self):
        self.assertAlmostEqual(
            2.0, drift_scan.export_age_days("2026-08-01T00:00:00Z", NOW), places=3
        )
        self.assertIsNone(drift_scan.export_age_days(None, NOW))
        self.assertIsNone(drift_scan.export_age_days("not-a-date", NOW))

    def test_unparsable_stamp_still_reports_via_the_reference_count(self):
        # No usable timestamp => the age proxy is unavailable, but the missing-id
        # proxy must still fire (and the body must say the stamp was unusable
        # rather than print a bogus age).
        make_beads_export(self.root, [{"id": "sq-aaa1", "status": "open"}])
        cited = " ".join(f"sq-m{n:04d}" for n in range(60))
        _write(self.root, "research/notes.md", cited)
        items = drift_scan.scan_beads_export_stale(self.root, NOW)
        self.assertEqual([KEY], [it.dedup_key for it in items])
        self.assertIn("unparsable", items[0].body)

    # -- report contract ------------------------------------------------------ #
    def test_body_carries_the_measurements_and_the_no_hand_edit_rule(self):
        make_beads_export(self.root, [bead("sq-aaa1", "2026-06-01T00:00:00Z")])
        item = drift_scan.scan_beads_export_stale(self.root, NOW)[0]
        self.assertEqual("beads-export-stale", item.drift_class)
        self.assertIn("63 days old", item.body)
        self.assertIn("bd export", item.body)
        self.assertIn("chore-beads-resync", item.body)
        self.assertIn("Do NOT hand-edit", item.body)

    def test_scanner_is_registered(self):
        # Wiring guard: the class only ever files an issue because it is in
        # SCANNERS, and print_report derives its heading from the function name.
        self.assertIn(drift_scan.scan_beads_export_stale, drift_scan.SCANNERS)
        self.assertEqual(
            "beads-export-stale",
            drift_scan.scan_beads_export_stale.__name__.replace("scan_", "").replace("_", "-"),
        )


# --------------------------------------------------------------------------- #
# dedup-key stability + issue body / JSON contract
# --------------------------------------------------------------------------- #
class DedupKeyStabilityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        make_crate(self.root, "sparq-nlq")
        make_registry(self.root, sources=["crates/sparq-cli/src/main.rs"])

    def test_dedup_key_is_stable_across_runs(self):
        keys1 = {it.dedup_key for it in drift_scan.scan_all(self.root)}
        keys2 = {it.dedup_key for it in drift_scan.scan_all(self.root)}
        self.assertEqual(keys1, keys2)
        self.assertIn("bench-missing:sparq-nlq", keys1)

    def test_dedup_key_is_content_free(self):
        # No PR number / date / volatile token in the key — purely class:subject.
        for it in drift_scan.scan_all(self.root):
            self.assertRegex(it.dedup_key, r"^[a-z-]+:[\w.-]+$")

    def test_issue_body_embeds_marker(self):
        items = drift_scan.scan_all(self.root)
        it = items[0]
        body = drift_scan.build_issue_body(it)
        self.assertIn(drift_scan.key_marker(it.dedup_key), body)
        self.assertIn("🤖 SPARQ agent", body)

    def test_key_marker_matches_open_issue_search_substring(self):
        # The search substring (used for idempotency) must be a substring of the
        # full HTML-comment marker, so a found issue's body always matches.
        key = "bench-missing:sparq-nlq"
        self.assertIn(f"drift-key: {key}", drift_scan.key_marker(key))


# --------------------------------------------------------------------------- #
# main(--dry-run) end-to-end (no gh)
# --------------------------------------------------------------------------- #
class MainDryRunTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        make_crate(self.root, "sparq-nlq")
        # An UNCONSOLIDATED crate-local conformance ratchet (sq-ncvq.16: sparq-shacl
        # is now exempt, so use a crate that is genuinely a split to exercise the
        # conformance-split class end-to-end).
        make_crate(self.root, "sparq-other")
        _write(self.root, "crates/sparq-other/tests/w3c_thing.rs", "// ratchet")
        make_registry(self.root, sources=["crates/sparq-cli/src/main.rs"])

    def test_dry_run_prints_and_writes_json(self):
        json_path = Path(self._tmp.name) / "out.json"
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = drift_scan.main(
                ["--root", str(self.root), "--dry-run", "--json", str(json_path)]
            )
        self.assertEqual(0, rc)
        out = buf.getvalue()
        self.assertIn("bench-missing", out)
        self.assertIn("conformance-split", out)
        payload = json.loads(json_path.read_text())
        self.assertGreaterEqual(payload["drift_count"], 2)
        classes = {it["class"] for it in payload["items"]}
        self.assertIn("bench-missing", classes)
        self.assertIn("conformance-split", classes)

    def test_no_drift_clean_repo(self):
        clean = tempfile.TemporaryDirectory()
        self.addCleanup(clean.cleanup)
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = drift_scan.main(["--root", clean.name, "--dry-run"])
        self.assertEqual(0, rc)
        self.assertIn("no drift detected", buf.getvalue())


# --------------------------------------------------------------------------- #
# CI guard (bead sq-z0se) — refuse to MINT issues outside CI
# --------------------------------------------------------------------------- #
class CiGuardTest(unittest.TestCase):
    """The issue-MINTING path must refuse to run outside CI; an accidental
    dev-box run minted 20 spurious issues (#397-416). --dry-run is NEVER
    guarded (it makes zero gh calls)."""

    def _drift_fixture(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        make_crate(root, "sparq-nlq")
        make_registry(root, sources=["crates/sparq-cli/src/main.rs"])
        return root

    def _clear_ci_env(self) -> None:
        backup = {k: os.environ.get(k) for k in ("GITHUB_ACTIONS", "CI",
                                                 "DRIFT_SCAN_ALLOW_LOCAL")}

        def restore() -> None:
            for k, v in backup.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

        self.addCleanup(restore)
        for k in backup:
            os.environ.pop(k, None)

    def test_running_in_ci_detects_markers(self):
        self.assertTrue(drift_scan.running_in_ci({"GITHUB_ACTIONS": "true"}))
        self.assertTrue(drift_scan.running_in_ci({"CI": "true"}))
        self.assertTrue(drift_scan.running_in_ci({"CI": "1"}))

    def test_running_in_ci_false_when_unset_or_falsey(self):
        self.assertFalse(drift_scan.running_in_ci({}))
        self.assertFalse(drift_scan.running_in_ci({"GITHUB_ACTIONS": "false"}))
        self.assertFalse(drift_scan.running_in_ci({"CI": ""}))

    def test_require_ci_passes_in_ci(self):
        drift_scan.require_ci({"GITHUB_ACTIONS": "true"})  # no raise

    def test_require_ci_passes_with_escape_hatch(self):
        drift_scan.require_ci({"DRIFT_SCAN_ALLOW_LOCAL": "1"})  # no raise

    def test_require_ci_refuses_outside_ci(self):
        err = io.StringIO()
        with redirect_stderr(err), self.assertRaises(SystemExit) as cm:
            drift_scan.require_ci({})
        self.assertEqual(cm.exception.code, 2)
        self.assertIn("refusing to file GitHub issues outside CI", err.getvalue())

    def test_main_mint_path_refuses_outside_ci_before_any_gh_call(self):
        # End-to-end: a real (non-dry-run) run with drift present must SystemExit
        # on the guard. If the guard were absent it would instead shell out to
        # `gh` to mint issues — exactly the #397-416 incident.
        root = self._drift_fixture()
        self._clear_ci_env()
        buf, err = io.StringIO(), io.StringIO()
        with redirect_stdout(buf), redirect_stderr(err), self.assertRaises(
            SystemExit
        ) as cm:
            drift_scan.main(["--root", str(root)])  # NO --dry-run
        self.assertEqual(cm.exception.code, 2)
        self.assertIn("refusing to file GitHub issues outside CI", err.getvalue())

    def test_main_dry_run_never_guarded_outside_ci(self):
        # --dry-run must succeed even with no CI markers (local audits / tests).
        root = self._drift_fixture()
        self._clear_ci_env()
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = drift_scan.main(["--root", str(root), "--dry-run"])
        self.assertEqual(0, rc)


if __name__ == "__main__":
    unittest.main()
