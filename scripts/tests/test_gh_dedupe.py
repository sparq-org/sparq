#!/usr/bin/env python3
"""[OPUS-5] issue #5804 — the auto-filers' open-issue dedupe must not trust gh's search.

Hermetic: no gh, no network, no repo writes. Every lookup runs against an injected fake
`gh` whose two backends are DELIBERATELY INCONSISTENT — the label listing knows about an
issue the search index does not. That is the whole failure mode #5804 describes (a
tokeniser miss, or a lagging index), reproduced as a fixture:

  * with the label-listing lookup, every filer FINDS the issue it already opened and
    comments on it;
  * with the old `--search` lookup it would not, and the lane would re-file — the
    non-spam invariant failing open, quietly.

The suite drives the REAL `find_open_issue` of all five call sites, so deleting the
`gh_dedupe` routing from any one of them reds here rather than in production.
"""
from __future__ import annotations

import importlib.util
import json
import os
import sys
import unittest
from types import SimpleNamespace

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.dirname(HERE)
sys.path.insert(0, SCRIPTS)

import gh_dedupe  # noqa: E402


def _load(mod_name: str, filename: str):
    """Import a hyphenated script as a module (they are not importable by name)."""
    path = os.path.join(SCRIPTS, filename)
    spec = importlib.util.spec_from_file_location(mod_name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[mod_name] = mod
    spec.loader.exec_module(mod)
    return mod


DEMOTED = _load("_t_demoted", "ci-file-demoted-lane-failure.py")
DIFFERENTIAL = _load("_t_differential", "ci-file-differential-failure.py")
METAMORPH = _load("_t_metamorph", "ci-file-metamorph-failure.py")
BENCH = _load("_t_bench_triage", "bench-triage.py")
COVERAGE = _load("_t_coverage_gate", "coverage-gate.py")

# Every file whose dedupe #5804 re-pointed. `route_sweep_failures.py`, named in the
# issue as a sixth site, already lists by label and never used `--search`.
REWIRED = [
    "ci-file-demoted-lane-failure.py",
    "ci-file-differential-failure.py",
    "ci-file-metamorph-failure.py",
    "bench-triage.py",
    "coverage-gate.py",
]


class FakeGh:
    """A `gh` whose label listing and search index DISAGREE — the #5804 fixture.

    `by_label` is the ground truth (the database read). `by_search` is the index; leave
    it empty to model the tokeniser missing the punctuation, or the index lagging.
    """

    def __init__(self, by_label=None, by_search=None, fail_label=False, fail_search=False):
        self.by_label = by_label or {}
        self.by_search = by_search or {}
        self.fail_label = fail_label
        self.fail_search = fail_search
        self.calls: list[list] = []

    def __call__(self, argv):
        self.calls.append(list(argv))
        if argv[:2] == ["issue", "list"]:
            if "--label" in argv:
                if self.fail_label:
                    return SimpleNamespace(returncode=1, stdout="")
                rows = self.by_label.get(argv[argv.index("--label") + 1], [])
            else:
                if self.fail_search:
                    return SimpleNamespace(returncode=1, stdout="")
                rows = self.by_search.get(argv[argv.index("--search") + 1], [])
            return SimpleNamespace(returncode=0, stdout=json.dumps(rows))
        return SimpleNamespace(returncode=0, stdout="")

    # -- call-shape assertions -------------------------------------------------
    def searched(self) -> bool:
        return any("--search" in c for c in self.calls)

    def listed_labels(self) -> list:
        return [c[c.index("--label") + 1] for c in self.calls
                if c[:2] == ["issue", "list"] and "--label" in c]

    def edited(self) -> list:
        return [c for c in self.calls if c[:2] == ["issue", "edit"]]


def _read(*parts) -> str:
    with open(os.path.join(SCRIPTS, *parts), encoding="utf-8") as fh:
        return fh.read()


def issue(number, title, body=""):
    return {"number": number, "title": title, "body": body}


class _PatchGh(unittest.TestCase):
    """Base: route gh_dedupe's default runner at a fake for the duration of a test."""

    def use(self, fake):
        real = gh_dedupe._default_runner
        gh_dedupe._default_runner = fake
        self.addCleanup(lambda: setattr(gh_dedupe, "_default_runner", real))
        return fake


class TestMatchMarker(unittest.TestCase):
    """Local exact-substring matching: the punctuation is matched, not tokenised."""

    def test_every_needle_must_be_present(self):
        rows = [issue(1, "[demoted-lane] lane=fuzz-randomized: full-form CI run failed")]
        self.assertIsNotNone(
            gh_dedupe.match_marker(rows, "[demoted-lane]", "lane=fuzz-randomized"))
        # a DIFFERENT lane must not match — dedupe is per-key, not per-marker.
        self.assertIsNone(
            gh_dedupe.match_marker(rows, "[demoted-lane]", "lane=coverage-ratchet-main"))

    def test_punctuation_shapes_that_break_the_search_tokeniser(self):
        for title, needles in [
            ("[differential-fuzz] shard=equality: 3 mismatch(es) vs Oxigraph",
             ("[differential-fuzz]", "shard=equality")),
            ("[bench-regression] cluster=sparq-engine+sparq-core: nightly regression (P1)",
             ("[bench-regression]", "cluster=sparq-engine+sparq-core")),
            ("[metamorph] shard=nightly: 2 TLP/NoREC oracle failure(s) (first seed=17)",
             ("[metamorph]", "shard=nightly")),
        ]:
            with self.subTest(title=title):
                self.assertIsNotNone(gh_dedupe.match_marker([issue(9, title)], *needles))

    def test_matching_is_substring_not_whole_key(self):
        """Pinned as UNCHANGED, not as desirable. The five call sites already asked
        `key in title`, so a key that is a PREFIX of a sibling's key matches it — and
        bench-triage really does mint both `cluster=sparq-engine` (the `fts`/`text`/
        `vectors` prefixes map to that single crate) and `cluster=sparq-engine+sparq-core`
        (`watdiv`/`sp2b`/`bsbm`/…). #5804 moved which LIST the match runs over; it
        deliberately did not change the predicate, so this pins the pre-existing
        semantics against silent drift in either direction."""
        rows = [issue(3, "[bench-regression] cluster=sparq-engine+sparq-core: x")]
        self.assertIsNotNone(
            gh_dedupe.match_marker(rows, "[bench-regression]", "cluster=sparq-engine"))

    def test_malformed_rows_are_skipped_not_crashed(self):
        rows = ["not-a-dict", {"number": 1}, issue(2, "[m] shard=smoke")]
        self.assertEqual(gh_dedupe.match_marker(rows, "[m]", "shard=smoke")["number"], 2)


class TestFindOpenIssue(_PatchGh):
    def test_label_listing_is_the_primary_and_search_is_not_consulted_on_a_hit(self):
        fake = self.use(FakeGh(by_label={"demoted-lane": [
            issue(11, "[demoted-lane] lane=fuzz-randomized: full-form CI run failed")]}))
        res = gh_dedupe.find_open_issue(
            "demoted-lane", ("[demoted-lane]", "lane=fuzz-randomized"),
            legacy_search='in:title "[demoted-lane] lane=fuzz-randomized"')
        self.assertEqual((res.number, res.source), ("11", "label"))
        self.assertFalse(fake.searched(),
                         "a hit in the label listing must never reach the search index")

    def test_index_miss_still_dedupes(self):
        """THE #5804 REGRESSION. The index returns nothing; the label listing has it."""
        fake = self.use(FakeGh(
            by_label={"metamorph": [issue(42, "[metamorph] shard=nightly: oracle failure(s)")]},
            by_search={},  # tokeniser miss / lagging index
        ))
        res = gh_dedupe.find_open_issue(
            "metamorph", ("[metamorph]", "shard=nightly"),
            legacy_search='in:title "[metamorph] shard=nightly"')
        self.assertEqual(res.number, "42")
        self.assertEqual(fake.listed_labels(), ["metamorph"])

    def test_legacy_search_is_a_supplement_and_backfills_the_label(self):
        """Pre-label issues (filed before the lane labelled anything) still dedupe, and
        get the label so the next tick's primary lookup sees them."""
        fake = self.use(FakeGh(
            by_label={"metamorph": []},
            by_search={'in:title "[metamorph] shard=nightly"':
                       [issue(7, "[metamorph] shard=nightly: oracle failure(s)")]},
        ))
        res = gh_dedupe.find_open_issue(
            "metamorph", ("[metamorph]", "shard=nightly"),
            legacy_search='in:title "[metamorph] shard=nightly"', backfill_label=True)
        self.assertEqual((res.number, res.source), ("7", "legacy-search"))
        self.assertEqual(fake.edited(),
                         [["issue", "edit", "7", "--add-label", "metamorph"]])

    def test_read_only_caller_does_not_backfill(self):
        fake = self.use(FakeGh(
            by_label={"metamorph": []},
            by_search={"q": [issue(7, "[metamorph] shard=nightly")]},
        ))
        gh_dedupe.find_open_issue("metamorph", ("[metamorph]", "shard=nightly"),
                                  legacy_search="q")
        self.assertEqual(fake.edited(), [], "backfill_label defaults OFF")

    def test_no_match_anywhere_is_a_probed_miss(self):
        self.use(FakeGh(by_label={"metamorph": [issue(1, "[metamorph] shard=smoke")]}))
        res = gh_dedupe.find_open_issue("metamorph", ("[metamorph]", "shard=nightly"),
                                        legacy_search="q")
        self.assertIsNone(res.issue)
        self.assertTrue(res.probed, "a successful listing that matched nothing IS a probe")

    def test_both_lookups_failing_is_could_not_tell_not_no_issue(self):
        self.use(FakeGh(fail_label=True, fail_search=True))
        res = gh_dedupe.find_open_issue("metamorph", ("[metamorph]", "shard=nightly"),
                                        legacy_search="q")
        self.assertIsNone(res.issue)
        self.assertFalse(res.probed)

    def test_a_missing_label_falls_through_to_the_supplement(self):
        # `gh issue list --label X` errors when X does not exist yet (first-ever run).
        fake = self.use(FakeGh(
            fail_label=True,
            by_search={"q": [issue(5, "[metamorph] shard=nightly")]},
        ))
        res = gh_dedupe.find_open_issue("metamorph", ("[metamorph]", "shard=nightly"),
                                        legacy_search="q")
        self.assertEqual((res.number, res.source), ("5", "legacy-search"))
        self.assertTrue(fake.searched())

    def test_a_truncated_listing_is_reported_and_supplemented(self):
        filler = [issue(i, f"[metamorph] shard=other-{i}") for i in range(5)]
        fake = self.use(FakeGh(
            by_label={"metamorph": filler},
            by_search={"q": [issue(99, "[metamorph] shard=nightly")]},
        ))
        notes = []
        res = gh_dedupe.find_open_issue(
            "metamorph", ("[metamorph]", "shard=nightly"), legacy_search="q",
            limit=len(filler), log=notes.append)
        self.assertEqual(res.number, "99")
        self.assertTrue(any("truncated" in n for n in notes), notes)

    def test_the_listing_asks_for_open_issues_by_label_at_the_documented_width(self):
        fake = self.use(FakeGh(by_label={"metamorph": []}))
        gh_dedupe.find_open_issue("metamorph", ("[metamorph]",))
        self.assertEqual(fake.calls, [[
            "issue", "list", "--state", "open", "--label", "metamorph",
            "--json", "number,title,body", "--limit", str(gh_dedupe.LIST_LIMIT)]])


class TestCallSites(_PatchGh):
    """Drive each lane's REAL find_open_issue over the inconsistent-backends fixture."""

    def check(self, fn, label, title, args, number):
        fake = self.use(FakeGh(by_label={label: [issue(number, title)]}, by_search={}))
        self.assertEqual(fn(*args), str(number),
                         f"{label}: the lane must find the issue it already filed")
        self.assertEqual(fake.listed_labels(), [label])
        self.assertFalse(fake.searched())
        # ...and a DIFFERENT key must not be deduped against it (no over-matching).
        return fake

    def test_demoted_lane_filer(self):
        self.check(DEMOTED.find_open_issue, "demoted-lane",
                   "[demoted-lane] lane=coverage-ratchet-main: full-form CI run failed",
                   ("coverage-ratchet-main",), 101)
        self.use(FakeGh(by_label={"demoted-lane": [issue(
            101, "[demoted-lane] lane=coverage-ratchet-main: full-form CI run failed")]}))
        self.assertIsNone(DEMOTED.find_open_issue("fuzz-randomized"))

    def test_differential_filer(self):
        self.check(DIFFERENTIAL.find_open_issue, "differential-fuzz",
                   "[differential-fuzz] shard=equality: 3 differential mismatch(es) "
                   "vs Oxigraph (first seed=41, mode=baseline)",
                   ("equality",), 102)

    def test_metamorph_filer(self):
        self.check(METAMORPH.find_open_issue, "metamorph",
                   "[metamorph] shard=nightly: 2 TLP/NoREC oracle failure(s) "
                   "(first seed=17)",
                   ("nightly",), 103)

    def test_bench_triage_flake_and_regression(self):
        self.check(BENCH.find_open_issue, "bench-flake",
                   "[bench-flake] suite=sparq-engine+sparq-core: benchmarks in the "
                   "soft zone (possible runner noise)",
                   ("bench-flake", "[bench-flake]", "suite=sparq-engine+sparq-core"), 104)
        self.check(BENCH.find_open_issue, "bench-regression",
                   "[bench-regression] cluster=sparq-engine+sparq-core: nightly benchmark "
                   "regression (P1)",
                   ("bench-regression", "[bench-regression]",
                    "cluster=sparq-engine+sparq-core"), 105)

    def test_coverage_gate_alarm_probe(self):
        """The ratchet-ADVANCE pause reads the same alarm the demoted-lane filer opens.
        An index miss used to report `False` — no alarm — and silently un-pause it."""
        title = "[demoted-lane] lane=coverage-ratchet-main: full-form CI run failed"
        quiet = lambda *a, **k: None
        self.use(FakeGh(by_label={"demoted-lane": [issue(106, title)]}, by_search={}))
        self.assertIs(COVERAGE.open_alarm_issue_state(log=quiet), True)
        # a listing that matched nothing is a confident "no alarm"...
        self.use(FakeGh(by_label={"demoted-lane": []}))
        self.assertIs(COVERAGE.open_alarm_issue_state(log=quiet), False)
        # ...and an unavailable API is None, which advance_block_verdict FAILS OPEN on.
        self.use(FakeGh(fail_label=True, fail_search=True))
        self.assertIsNone(COVERAGE.open_alarm_issue_state(log=quiet))

    def test_coverage_gate_probe_never_writes(self):
        fake = self.use(FakeGh(
            by_label={"demoted-lane": []},
            by_search={'in:title "[demoted-lane] lane=coverage-ratchet-main"': [issue(
                7, "[demoted-lane] lane=coverage-ratchet-main: full-form CI run failed")]},
        ))
        self.assertIs(COVERAGE.open_alarm_issue_state(log=lambda *a, **k: None), True)
        self.assertEqual(fake.edited(), [],
                         "the coverage gate runs on PRs — the probe must stay read-only")


class TestWiring(unittest.TestCase):
    """The seam itself: no rewired site may fall back to search as its PRIMARY lookup."""

    def test_no_site_builds_a_search_lookup_by_hand(self):
        for name in REWIRED:
            with self.subTest(name=name):
                src = _read(name)
                self.assertIn("gh_dedupe.find_open_issue", src,
                              f"{name} must route its dedupe through gh_dedupe")
                self.assertNotIn('"--search"', src,
                                 f"{name} must not hand-build a --search lookup; pass "
                                 "legacy_search= to gh_dedupe.find_open_issue instead")

    def test_each_filer_labels_the_issues_it_opens(self):
        # The label is what the primary lookup reads back: a filer that opens an
        # unlabelled issue cannot find it again.
        for name, label in [("ci-file-demoted-lane-failure.py", "demoted-lane"),
                            ("ci-file-differential-failure.py", "differential-fuzz"),
                            ("ci-file-metamorph-failure.py", "metamorph")]:
            with self.subTest(name=name):
                src = _read(name)
                self.assertIn(f'LABEL = "{label}"', src)
                self.assertIn('"--label", LABEL', src,
                              f"{name} must stamp LABEL on the issue it creates")
                self.assertIn("gh_dedupe.ensure_label(", src,
                              f"{name} must upsert LABEL before listing by it")

    def test_the_siblings_that_already_refused_search_still_refuse_it(self):
        for name in ["ci_selection_alarm.py", "nightly/route_sweep_failures.py"]:
            with self.subTest(name=name):
                src = _read(name)
                self.assertNotIn('"--search"', src)

    def test_this_suite_is_wired_into_ci(self):
        wf = _read("..", ".github", "workflows", "docs-quality.yml")
        self.assertIn("scripts/tests/test_gh_dedupe.py", wf,
                      "the suite must not be able to leave CI silently")


if __name__ == "__main__":
    unittest.main(verbosity=2)
