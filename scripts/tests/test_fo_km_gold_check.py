#!/usr/bin/env python3
# [SONNET-4.6] Hermetic tests for the FO-KM task validator's PER-CATEGORY gold check
# (bench/fo-km/validate_tasks.py, epic sq-mztg8).
#
# WHY: cc03 (`{event, artifact}`) and cc05 (`{claim, document, method}`) carry a
# dict-of-ints `gold_keys` behind a SINGLE `select` query, so they reached neither the
# list-gold row check nor the multi-part `gold_count` check — ANY non-empty result passed,
# even with wrong per-category values, so a refreshed gold was never materially proven.
# These tests pin the value comparison in BOTH directions (a correct table passes, a
# mutated category value fails) and pin the DISPATCH, so no task shape in the shipped
# tasks.jsonl silently escapes a gold check again.
#
# Hermetic w.r.t. cargo/network: `run()` (which shells out to the pkg-query binary) is
# monkeypatched with a table generator that reproduces pkg_query.rs::print_table's layout
# (cells joined by "  |  ", header, dashed separator, rows). No subprocess, no toolchain.
#
# Run:  python3 scripts/tests/test_fo_km_gold_check.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SEP = "  |  "  # pkg_query.rs::print_table joins cells with exactly this


def _load():
    path = REPO_ROOT / "bench" / "fo-km" / "validate_tasks.py"
    spec = importlib.util.spec_from_file_location("fo_km_validate_tasks", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["fo_km_validate_tasks"] = mod
    spec.loader.exec_module(mod)
    return mod


vt = _load()


def table(header_cells: list[str], rows: list[list[str]]) -> tuple[str, list[str]]:
    """Build a (header_line, data_rows) pair exactly as print_table would emit it."""
    return SEP.join(header_cells), [SEP.join(r) for r in rows]


def tasks() -> list[dict]:
    with open(REPO_ROOT / "bench" / "fo-km" / "tasks.jsonl") as fh:
        return [json.loads(line) for line in fh]


def task(tid: str) -> dict:
    return next(t for t in tasks() if t["id"] == tid)


# --- the two real result shapes, as pkg-query prints them --------------------------


def wide(gold: dict) -> tuple[str, list[str]]:
    """cc03: one row of aggregate columns, projection names PLURAL (`?events`)."""
    return table([f"{k}s" for k in gold], [[str(v) for v in gold.values()]])


def grouped(gold: dict) -> tuple[str, list[str]]:
    """cc05: `?kind (COUNT(DISTINCT ?x) AS ?n)` + GROUP BY — one row per category."""
    return table(["kind", "n"], [[k, str(v)] for k, v in sorted(gold.items())])


class TestCategoryGoldCc03(unittest.TestCase):
    """cc03 — the WIDE multi-column shape."""

    def setUp(self):
        self.gold = task("cc03")["gold_keys"]  # {"event": N, "artifact": N}

    def test_correct_result_passes(self):
        header, data = wide(self.gold)
        self.assertEqual(vt.check_category_gold("cc03", "gufo", header, data, self.gold), [])

    def test_mutated_event_count_fails(self):
        wrong = dict(self.gold, event=self.gold["event"] - 1)
        header, data = wide(wrong)
        failures = vt.check_category_gold("cc03", "gufo", header, data, self.gold)
        self.assertTrue(any("'event'" in f for f in failures), failures)
        self.assertTrue(any(str(self.gold["event"]) in f for f in failures), failures)

    def test_mutated_artifact_count_fails(self):
        wrong = dict(self.gold, artifact=self.gold["artifact"] + 7)
        header, data = wide(wrong)
        failures = vt.check_category_gold("cc03", "gufo", header, data, self.gold)
        self.assertTrue(any("'artifact'" in f for f in failures), failures)

    def test_swapped_columns_fail(self):
        """The value comparison is per-CATEGORY, not a bag of numbers."""
        header, _ = wide(self.gold)
        _, data = table([], [[str(self.gold["artifact"]), str(self.gold["event"])]])
        failures = vt.check_category_gold("cc03", "gufo", header, data, self.gold)
        self.assertEqual(len(failures), 2, failures)

    def test_non_empty_row_alone_does_not_pass(self):
        """The pre-fix hole: one non-empty row with arbitrary values used to pass."""
        header, data = table(["events", "artifacts"], [["1", "1"]])
        self.assertNotEqual(vt.check_category_gold("cc03", "gufo", header, data, self.gold), [])


class TestCategoryGoldCc05(unittest.TestCase):
    """cc05 — the GROUPED shape."""

    def setUp(self):
        self.gold = task("cc05")["gold_keys"]  # {"claim": N, "document": N, "method": N}

    def test_correct_result_passes(self):
        header, data = grouped(self.gold)
        self.assertEqual(vt.check_category_gold("cc05", "gufo", header, data, self.gold), [])

    def test_mutated_claim_count_fails(self):
        wrong = dict(self.gold, claim=self.gold["claim"] - 1)
        header, data = grouped(wrong)
        failures = vt.check_category_gold("cc05", "gufo", header, data, self.gold)
        self.assertTrue(any("'claim'" in f for f in failures), failures)

    def test_each_category_is_checked_independently(self):
        for cat in self.gold:
            wrong = dict(self.gold, **{cat: self.gold[cat] + 1})
            header, data = grouped(wrong)
            failures = vt.check_category_gold("cc05", "gufo", header, data, self.gold)
            self.assertTrue(any(f"'{cat}'" in f for f in failures), (cat, failures))

    def test_missing_group_fails(self):
        partial = {k: v for k, v in self.gold.items() if k != "method"}
        header, data = grouped(partial)
        failures = vt.check_category_gold("cc05", "gufo", header, data, self.gold)
        self.assertTrue(any("'method'" in f for f in failures), failures)

    def test_unexpected_group_fails(self):
        extra = dict(self.gold, gizmo=3)
        header, data = grouped(extra)
        failures = vt.check_category_gold("cc05", "gufo", header, data, self.gold)
        self.assertTrue(any("gizmo" in f for f in failures), failures)

    def test_right_row_count_wrong_values_fails(self):
        """gold_count for cc05 is the GROUP count (3) — matching it proves nothing."""
        header, data = grouped({k: 999 for k in self.gold})
        failures = vt.check_category_gold("cc05", "gufo", header, data, self.gold)
        self.assertEqual(len(data), task("cc05")["gold_count"])
        self.assertEqual(len(failures), len(self.gold), failures)


class TestEntityListGold(unittest.TestCase):
    """The list-shaped gold: the returned ENTITIES must be the gold entities, not merely
    the right NUMBER of rows."""

    def setUp(self):
        self.gold = task("th04")["gold_keys"]  # 5 Technique local names

    def rows(self, keys):
        return table(["x"], [[k] for k in keys])[1]

    def test_exact_gold_passes(self):
        self.assertEqual(
            vt.check_entity_gold("th04", "gufo", self.rows(self.gold), self.gold), [])

    def test_same_count_wrong_entity_fails(self):
        """The hole this closes: cardinality preserved, one entity swapped for a wrong one."""
        wrong = ["surface-not-a-real-thing"] + self.gold[1:]
        failures = vt.check_entity_gold("th04", "gufo", self.rows(wrong), self.gold)
        self.assertTrue(any("MISSING" in f and self.gold[0] in f for f in failures), failures)
        self.assertTrue(
            any("absent from gold_keys" in f and "surface-not-a-real-thing" in f
                for f in failures), failures)

    def test_entirely_wrong_answer_of_the_right_size_fails(self):
        wrong = [f"e{i}" for i in range(len(self.gold))]
        failures = vt.check_entity_gold("th04", "gufo", self.rows(wrong), self.gold)
        self.assertEqual(len(failures), 2, failures)
        self.assertTrue(any(f"{len(self.gold)} gold entity" in f for f in failures), failures)

    def test_missing_entity_is_reported(self):
        failures = vt.check_entity_gold("th04", "gufo", self.rows(self.gold[:-1]), self.gold)
        self.assertTrue(any(self.gold[-1] in f for f in failures), failures)

    def test_extra_entity_is_reported(self):
        failures = vt.check_entity_gold(
            "th04", "gufo", self.rows(self.gold + ["surface-extra"]), self.gold)
        self.assertTrue(any("surface-extra" in f for f in failures), failures)

    def test_pair_shaped_rows_compare_on_the_first_column(self):
        """cc04 projects `?f ?topic`: more ROWS than distinct findings, and the second
        column is a topic that is deliberately not in gold_keys."""
        gold = task("cc04")["gold_keys"]
        rows = [[f, "topic-x"] for f in gold] + [[gold[0], "topic-y"], [gold[1], "topic-z"]]
        _, data = table(["f", "topic"], rows)
        self.assertEqual(len(data), len(gold) + 2)
        self.assertEqual(vt.check_entity_gold("cc04", "gufo", data, gold), [])

    def test_large_failure_listing_is_bounded_but_counted(self):
        """th01 carries 91 gold entities — a wholly-wrong answer must not print 182 lines."""
        gold = task("th01")["gold_keys"]
        failures = vt.check_entity_gold(
            "th01", "gufo", self.rows([f"e{i}" for i in gold]), gold)
        self.assertEqual(len(failures), 2, failures)
        self.assertTrue(any(f"{len(gold)} gold entity" in f for f in failures), failures)
        self.assertTrue(any("more)" in f for f in failures), failures)


class TestEntityListGoldPredicate(unittest.TestCase):
    """Which list golds name entities (checkable) vs which are scalar SENTINELS."""

    ROW_SELECT = "SELECT DISTINCT ?x WHERE { ?x a fo:Object } ORDER BY ?x"

    def test_row_select_with_a_list_is_an_entity_gold(self):
        self.assertTrue(vt.is_entity_list_gold(["a", "b"], self.ROW_SELECT))

    def test_aggregate_query_is_not(self):
        self.assertFalse(vt.is_entity_list_gold(
            ["top-1349"], "SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a fo:Individual }"))

    def test_empty_and_non_list_are_not(self):
        self.assertFalse(vt.is_entity_list_gold([], self.ROW_SELECT))
        self.assertFalse(vt.is_entity_list_gold({"event": 1}, self.ROW_SELECT))

    def test_shipped_fixture_routes_exactly_the_entity_list_tasks_here(self):
        """Pins the dispatch against tasks.jsonl: the five row-returning entity-list tasks
        are value-checked, and every SENTINEL gold sits behind an aggregate query."""
        checked = set()
        for t in tasks():
            for q in t["select"].values():
                if isinstance(q, str) and vt.is_entity_list_gold(t["gold_keys"], q):
                    checked.add(t["id"])
        self.assertEqual(checked, {"th01", "th04", "cc01", "cc04", "th06"})

    def test_sentinel_golds_have_no_entity_to_check(self):
        """er02's `top-1349` and friends are descriptive tags, not local names — their
        real gold is `gold_count`, so the entity check must not be applied to them."""
        for tid in ("th02", "er01", "er02", "er03", "th05", "er04", "th07", "cc02"):
            t = task(tid)
            for q in t["select"].values():
                if isinstance(q, str):
                    self.assertFalse(vt.is_entity_list_gold(t["gold_keys"], q), tid)


class TestMultiPartGold(unittest.TestCase):
    """th03 — the MULTI-PART shape: a dict `select` whose parts each carry their own
    entity list in a dict-shaped `gold_keys`. The per-part `gold_count` check alone let
    either sub-query return arbitrary entities of the expected cardinality."""

    def setUp(self):
        self.t = task("th03")
        self.gold = self.t["gold_keys"]  # {"truth_bearers": [...], "info_bearers": [...]}

    def rows(self, keys):
        return table(["x"], [[k] for k in keys])[1]

    def test_both_parts_carry_a_checkable_entity_list(self):
        """Pins the dispatch: neither part is a scalar SENTINEL, so both must be
        value-checked against their own gold."""
        for arm, q in self.t["select"].items():
            self.assertIsInstance(q, dict, arm)
            for part, sub in q.items():
                self.assertTrue(vt.is_entity_list_gold(self.gold[part], sub), (arm, part))

    def test_correct_part_entities_pass(self):
        for part, keys in self.gold.items():
            self.assertEqual(
                vt.check_entity_gold("th03", "gufo", self.rows(keys), keys, part=part), [])

    def test_wrong_part_entity_fails_and_names_the_part(self):
        for part, keys in self.gold.items():
            wrong = ["surface-not-a-real-thing"] + keys[1:]
            failures = vt.check_entity_gold("th03", "gufo", self.rows(wrong), keys, part=part)
            self.assertTrue(any(f"th03/gufo/{part}" in f for f in failures), (part, failures))
            self.assertTrue(any(keys[0] in f for f in failures), (part, failures))

    def test_parts_are_not_interchangeable(self):
        """truth_bearers answered with info_bearers' entities is a FAILURE — the gold is
        per-part, not a union over the task."""
        failures = vt.check_entity_gold(
            "th03", "gufo", self.rows(self.gold["info_bearers"]),
            self.gold["truth_bearers"], part="truth_bearers")
        self.assertNotEqual(failures, [])

    def test_shipped_part_keys_align(self):
        for arm, q in self.t["select"].items():
            self.assertEqual(
                vt.check_part_keys("th03", arm, q, self.t["gold_count"], self.gold), [])

    def test_missing_gold_entry_for_a_part_is_reported(self):
        partial = {k: v for k, v in self.gold.items() if k != "info_bearers"}
        failures = vt.check_part_keys(
            "th03", "gufo", self.t["select"]["gufo"], self.t["gold_count"], partial)
        self.assertTrue(
            any("info_bearers" in f and "no gold_keys entry" in f for f in failures), failures)

    def test_extra_gold_entry_without_a_part_is_reported(self):
        extra = dict(self.t["gold_count"], phantom_bearers=3)
        failures = vt.check_part_keys(
            "th03", "gufo", self.t["select"]["gufo"], extra, self.gold)
        self.assertTrue(
            any("phantom_bearers" in f and "no matching select part" in f
                for f in failures), failures)

    def test_non_dict_gold_keys_is_reported_unchecked(self):
        failures = vt.check_part_keys(
            "th03", "gufo", self.t["select"]["gufo"], self.t["gold_count"], ["sentinel"])
        self.assertTrue(any("UNCHECKED" in f for f in failures), failures)


class TestUnparseableTableIsAFailure(unittest.TestCase):
    """An unreadable table must FAIL loudly, never pass as 'nothing to check'."""

    def test_entity_list_result_is_reported_unchecked(self):
        header, data = table(["x"], [["find-a"], ["find-b"]])
        failures = vt.check_category_gold("cc05", "gufo", header, data, {"claim": 2})
        self.assertTrue(any("UNCHECKED" in f for f in failures), failures)

    def test_empty_result_is_reported_unchecked(self):
        failures = vt.check_category_gold("cc03", "gufo", "events  |  artifacts", [], {"event": 1})
        self.assertTrue(any("UNCHECKED" in f for f in failures), failures)


class TestNormalisation(unittest.TestCase):
    def test_plural_projection_matches_singular_gold_key(self):
        self.assertEqual(vt.norm("events"), vt.norm("event"))
        self.assertEqual(vt.norm("artifacts"), vt.norm("artifact"))

    def test_distinct_categories_stay_distinct(self):
        self.assertNotEqual(vt.norm("claim"), vt.norm("document"))
        self.assertNotEqual(vt.norm("event"), vt.norm("artifact"))

    def test_colliding_gold_keys_are_reported(self):
        header, data = table(["kind", "n"], [["event", "1"]])
        failures = vt.check_category_gold("x", "gufo", header, data, {"event": 1, "events": 1})
        self.assertTrue(any("ambiguous" in f for f in failures), failures)


class TestShapePredicate(unittest.TestCase):
    def test_dict_of_ints_is_category_gold(self):
        self.assertTrue(vt.is_category_gold({"event": 1, "artifact": 2}))

    def test_dict_of_lists_is_not(self):
        """th03's gold_keys is a dict of ENTITY LISTS — the multi-part branch owns it."""
        self.assertFalse(vt.is_category_gold(task("th03")["gold_keys"]))

    def test_list_and_empty_are_not(self):
        self.assertFalse(vt.is_category_gold(["a", "b"]))
        self.assertFalse(vt.is_category_gold({}))

    def test_shipped_fixture_routes_cc03_and_cc05_here(self):
        """Pins the dispatch: these two are dict-of-ints gold behind a SINGLE query."""
        for tid in ("cc03", "cc05"):
            t = task(tid)
            self.assertTrue(vt.is_category_gold(t["gold_keys"]), tid)
            for arm, q in t["select"].items():
                if q is not None:
                    self.assertIsInstance(q, str, f"{tid}/{arm}")


# --- end-to-end: drive main() with a canned result table ---------------------------


def entity_rows(keys: list[str], n: int) -> tuple[str, list[str]]:
    """`n` rows whose first column ranges over exactly the distinct entities `keys`.

    cc04 projects `?f ?topic` PAIRS — 18 rows over 15 distinct findings — so when the gold
    row count exceeds the entity count the surplus rows REPEAT gold entities rather than
    inventing new ones, which is what the real arm returns.
    """
    rows = [keys[i % len(keys)] for i in range(max(n, len(keys)))]
    return table(["x"], [[k] for k in rows])


def fake_run_factory(overrides: dict | None = None):
    """A `run()` stand-in: answers each query from tasks.jsonl's own gold.

    `overrides` maps a task id to a deliberately WRONG result so a test can assert the
    validator rejects it — a {category: wrong_value} patch for a dict-of-ints gold
    (cc03/cc05), a replacement list of entity local names for a single-query entity-list
    gold, or a {part: replacement_list} patch for a MULTI-PART task (th03), which leaves
    the other part answering its own gold.
    """
    overrides = overrides or {}
    index = {}  # query string -> (task, part)
    for t in tasks():
        for q in t["select"].values():
            if q is None:
                continue
            for part, sub in (q.items() if isinstance(q, dict) else [("_", q)]):
                index[sub] = (t, part)

    def _run(overlay_path, sparql):
        if overlay_path == vt.OVERLAY["no-fo"]:
            return "x", []  # the no-FO arm cannot answer — that is the discrimination
        t, part = index[sparql]
        gold_keys, gold_count = t["gold_keys"], t["gold_count"]
        override = overrides.get(t["id"])
        if vt.is_category_gold(gold_keys):
            gold = dict(gold_keys, **(override or {}))
            return grouped(gold) if "GROUP BY" in sparql else wide(gold)
        n = gold_count[part] if isinstance(gold_count, dict) else gold_count
        if "GROUP BY" in sparql:
            return table(["cat", "n"], [[f"c{i}", "1"] for i in range(n)])
        if "COUNT(" in sparql.upper():
            return table(["n"], [[str(n)]])
        # A row-returning SELECT: return the task's OWN gold entities, so the end-to-end
        # pass is evidence the entity check accepts a correct answer rather than evidence
        # that arbitrary `e0, e1, ...` rows slip through it.
        keys = gold_keys[part] if isinstance(gold_keys, dict) else gold_keys
        if isinstance(override, list):
            keys = override
        elif isinstance(override, dict) and part in override:
            keys = override[part]
        if vt.is_entity_list_gold(keys, sparql):
            return entity_rows(keys, n)
        return table(["x"], [[f"e{i}"] for i in range(n)])

    return _run


@contextlib.contextmanager
def driven(overrides=None):
    """Run main() against the fake table generator from the repo root; yield its stdout."""
    original_run, cwd, out = vt.run, os.getcwd(), io.StringIO()
    vt.run = fake_run_factory(overrides)
    os.chdir(REPO_ROOT)
    try:
        with contextlib.redirect_stdout(out):
            yield out
    finally:
        vt.run, _ = original_run, os.chdir(cwd)


class TestEndToEndDispatch(unittest.TestCase):
    def test_gold_matching_results_validate(self):
        """Every shipped task passes when each arm returns its own gold — so the new
        `else` (unrecognised shape) does NOT fire on any of the 16 tasks."""
        with driven() as out:
            vt.main()
        self.assertIn("TASKS DISCRIMINATE", out.getvalue())
        self.assertNotIn("unrecognised gold shape", out.getvalue())

    def test_wrong_cc03_event_count_fails_validation(self):
        gold = task("cc03")["gold_keys"]
        with driven({"cc03": {"event": gold["event"] - 1}}) as out:
            with self.assertRaises(SystemExit) as exit_ctx:
                vt.main()
        self.assertEqual(exit_ctx.exception.code, 1)
        self.assertIn("cc03", out.getvalue())
        self.assertIn("'event'", out.getvalue())

    def test_wrong_cc05_document_count_fails_validation(self):
        gold = task("cc05")["gold_keys"]
        with driven({"cc05": {"document": gold["document"] + 1}}) as out:
            with self.assertRaises(SystemExit) as exit_ctx:
                vt.main()
        self.assertEqual(exit_ctx.exception.code, 1)
        self.assertIn("cc05", out.getvalue())
        self.assertIn("'document'", out.getvalue())

    def test_wrong_entity_at_the_right_row_count_fails_validation(self):
        """MUTATION: swap ONE returned entity for a wrong one, preserving the row count.

        The `gold_count` check therefore stays green — only the entity comparison can
        catch this, so the assertion that no count failure was reported is what makes the
        test non-vacuous for the invariant.
        """
        gold = task("th04")["gold_keys"]
        mutated = ["surface-not-a-real-thing"] + gold[1:]
        self.assertEqual(len(mutated), task("th04")["gold_count"])
        with driven({"th04": mutated}) as out:
            with self.assertRaises(SystemExit) as exit_ctx:
                vt.main()
        report = out.getvalue()
        self.assertEqual(exit_ctx.exception.code, 1)
        self.assertIn("th04", report)
        self.assertIn(gold[0], report)
        self.assertIn("surface-not-a-real-thing", report)
        self.assertNotIn("expected gold_count", report)

    def test_wrong_th03_part_entity_at_the_right_row_count_fails_validation(self):
        """MUTATION on the MULTI-PART dispatch path: swap ONE truth_bearers entity for a
        wrong one, preserving that part's row count (and leaving info_bearers correct).

        Every per-part `gold_count` therefore stays green — asserting no count mismatch
        was reported is what makes this non-vacuous for the per-part ENTITY invariant.
        """
        gold = task("th03")["gold_keys"]["truth_bearers"]
        mutated = ["surface-not-a-real-thing"] + gold[1:]
        self.assertEqual(len(mutated), task("th03")["gold_count"]["truth_bearers"])
        with driven({"th03": {"truth_bearers": mutated}}) as out:
            with self.assertRaises(SystemExit) as exit_ctx:
                vt.main()
        report = out.getvalue()
        self.assertEqual(exit_ctx.exception.code, 1)
        self.assertIn("th03", report)
        self.assertIn("truth_bearers", report)
        self.assertIn(gold[0], report)
        self.assertIn("surface-not-a-real-thing", report)
        self.assertNotIn("expected gold_count", report)

    def test_wrong_th03_info_bearers_entity_fails_validation(self):
        """The other part is checked independently, not merely the first one."""
        gold = task("th03")["gold_keys"]["info_bearers"]
        with driven({"th03": {"info_bearers": ["doc-not-a-real-doc"] + gold[1:]}}) as out:
            with self.assertRaises(SystemExit) as exit_ctx:
                vt.main()
        report = out.getvalue()
        self.assertEqual(exit_ctx.exception.code, 1)
        self.assertIn("info_bearers", report)
        self.assertIn("doc-not-a-real-doc", report)
        self.assertNotIn("expected gold_count", report)

    def test_wrong_entity_in_the_pair_shaped_cc04_fails_validation(self):
        """cc04's rows are (finding, topic) PAIRS, so its entity check reads the first
        column only — a wrong finding must still be caught."""
        gold = task("cc04")["gold_keys"]
        with driven({"cc04": ["find-not-a-real-finding"] + gold[1:]}) as out:
            with self.assertRaises(SystemExit) as exit_ctx:
                vt.main()
        self.assertEqual(exit_ctx.exception.code, 1)
        self.assertIn("find-not-a-real-finding", out.getvalue())
        self.assertIn(gold[0], out.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=2)
