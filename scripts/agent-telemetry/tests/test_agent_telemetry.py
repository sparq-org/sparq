#!/usr/bin/env python3
# Hermetic regression tests for the agent-telemetry harness
# (bead sq-dhss, epic sq-lhwo).
#
# Fully hermetic: imports scripts/agent-telemetry/agent_telemetry.py and runs the
# parser/aggregator against a CHECKED-IN SYNTHETIC fixture transcript (a handful of
# JSONL lines, NO real session data). Exercises: per-agent + wave token rollups,
# the cache-hit-ratio math (the key efficiency signal), TTL split, tool/turn counts,
# timestamp-derived durations, robustness to malformed/bare/usage-less lines, the
# optional price-table cost model, and the JSON + table renderers.
#
# Run:  python3 scripts/agent-telemetry/tests/test_agent_telemetry.py
# (stdlib only; no pytest required.)

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
HARNESS = HERE.parent / "agent_telemetry.py"
FIXTURE = HERE / "fixture_transcript.jsonl"


def _load_module():
    spec = importlib.util.spec_from_file_location("agent_telemetry", HARNESS)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["agent_telemetry"] = mod
    spec.loader.exec_module(mod)
    return mod


at = _load_module()


class TestParsing(unittest.TestCase):
    def setUp(self):
        self.report = at.parse_transcript(str(FIXTURE))

    def test_fixture_exists(self):
        self.assertTrue(FIXTURE.is_file(), "synthetic fixture must be checked in")

    def test_only_usage_bearing_assistant_records_counted(self):
        # 5 assistant records carry a usage object; the no-usage assistant, the
        # user record, the bare-string line and the invalid-json line are skipped.
        self.assertEqual(self.report.records_parsed, 5)
        # records_seen counts every well-formed JSON OBJECT (bare string + invalid
        # json are NOT objects, so they are not seen): summary, 3 orchestrator
        # assistants (one usage-less), 1 user, 2 sub-agent A, 1 sub-agent B = 8.
        self.assertEqual(self.report.records_seen, 8)

    def test_agent_bucketing(self):
        # orchestrator + two sub-agents.
        self.assertEqual(len(self.report.agents), 3)
        self.assertIn(at.ORCHESTRATOR_KEY, self.report.agents)
        self.assertIn("agentAAA0000001", self.report.agents)
        self.assertIn("agentBBB0000002", self.report.agents)
        self.assertEqual(
            sum(1 for s in self.report.agents.values() if s.is_sidechain), 2
        )

    def test_orchestrator_totals(self):
        orch = self.report.agents[at.ORCHESTRATOR_KEY]
        self.assertFalse(orch.is_sidechain)
        self.assertEqual(orch.assistant_turns, 2)  # usage-less turn not counted
        self.assertEqual(orch.tokens.input_tokens, 1300)
        self.assertEqual(orch.tokens.output_tokens, 350)
        self.assertEqual(orch.tokens.cache_read_input_tokens, 6000)
        self.assertEqual(orch.tokens.cache_creation_input_tokens, 5000)
        self.assertEqual(orch.tokens.cache_creation_1h, 5000)
        self.assertEqual(orch.tokens.cache_creation_5m, 0)
        self.assertEqual(orch.tool_calls, 1)  # one Task tool_use in turn 1

    def test_subagent_a_totals_and_attribution(self):
        a = self.report.agents["agentAAA0000001"]
        self.assertTrue(a.is_sidechain)
        # attribution present on first turn, absent on second — must persist.
        self.assertEqual(a.attribution_agent, "sparq-rust-feature")
        self.assertEqual(a.assistant_turns, 2)
        self.assertEqual(a.tokens.input_tokens, 2500)
        self.assertEqual(a.tokens.output_tokens, 650)
        self.assertEqual(a.tokens.cache_read_input_tokens, 9000)
        self.assertEqual(a.tokens.cache_creation_input_tokens, 9000)
        self.assertEqual(a.tokens.cache_creation_5m, 9000)  # both writes 5m TTL
        self.assertEqual(a.tool_calls, 3)  # Read, Bash, Edit

    def test_duration_from_timestamps(self):
        a = self.report.agents["agentAAA0000001"]
        # 10:01:00 -> 10:02:00 = 60s.
        self.assertEqual(a.duration_seconds, 60.0)
        orch = self.report.agents[at.ORCHESTRATOR_KEY]
        # 10:00:00 -> 10:00:30 = 30s (the usage-less 10:03 turn is not counted,
        # so it does not stretch the orchestrator window).
        self.assertEqual(orch.duration_seconds, 30.0)

    def test_wave_duration_spans_all_agents(self):
        # earliest parsed turn 10:00:00, latest 10:02:00 -> 120s.
        self.assertEqual(self.report.wave_duration_seconds, 120.0)


class TestCacheHitRatio(unittest.TestCase):
    def test_cold_subagent_ratio_zero(self):
        b = at.parse_transcript(str(FIXTURE)).agents["agentBBB0000002"]
        # all cache-eligible tokens are writes -> hit ratio 0.0.
        self.assertEqual(b.tokens.cache_read_input_tokens, 0)
        self.assertEqual(b.tokens.cache_creation_input_tokens, 4000)
        self.assertEqual(b.tokens.cache_hit_ratio, 0.0)

    def test_partial_hit_ratio(self):
        a = at.parse_transcript(str(FIXTURE)).agents["agentAAA0000001"]
        # 9000 read / (9000 read + 9000 write) = 0.5.
        self.assertEqual(a.tokens.cache_hit_ratio, 0.5)

    def test_undefined_ratio_is_none_not_zero(self):
        # An agent with no cache-eligible tokens reports None, not a misleading 0.0.
        t = at.TokenTotals(input_tokens=100, output_tokens=10)
        self.assertIsNone(t.cache_hit_ratio)

    def test_wave_rollup_ratio(self):
        rep = at.parse_transcript(str(FIXTURE))
        roll = rep.rollup
        # reads: 6000 (orch) + 9000 (A) + 0 (B) = 15000
        # writes: 5000 (orch) + 9000 (A) + 4000 (B) = 18000
        self.assertEqual(roll.cache_read_input_tokens, 15000)
        self.assertEqual(roll.cache_creation_input_tokens, 18000)
        self.assertAlmostEqual(roll.cache_hit_ratio, 15000 / 33000)
        # input/output rollups
        self.assertEqual(roll.input_tokens, 1300 + 2500 + 1200)
        self.assertEqual(roll.output_tokens, 350 + 650 + 600)


class TestRobustness(unittest.TestCase):
    def test_blank_invalid_and_nondict_lines_skipped(self):
        lines = [
            "",
            "   ",
            "not json at all",
            '"a bare string"',
            "[1, 2, 3]",  # valid json but not an object
            '{"type":"assistant","message":{"usage":{"input_tokens":5}}}',
        ]
        rep = at.aggregate(at.iter_records(lines))
        self.assertEqual(rep.records_parsed, 1)
        self.assertEqual(rep.rollup.input_tokens, 5)

    def test_non_numeric_usage_values_coerced_to_zero(self):
        lines = [
            '{"type":"assistant","message":{"usage":'
            '{"input_tokens":"oops","output_tokens":null,'
            '"cache_read_input_tokens":7}}}',
        ]
        rep = at.aggregate(at.iter_records(lines))
        roll = rep.rollup
        self.assertEqual(roll.input_tokens, 0)
        self.assertEqual(roll.output_tokens, 0)
        self.assertEqual(roll.cache_read_input_tokens, 7)

    def test_missing_cache_creation_subobject(self):
        lines = [
            '{"type":"assistant","message":{"usage":'
            '{"input_tokens":1,"cache_creation_input_tokens":3}}}',
        ]
        rep = at.aggregate(at.iter_records(lines))
        self.assertEqual(rep.rollup.cache_creation_input_tokens, 3)
        self.assertEqual(rep.rollup.cache_creation_5m, 0)
        self.assertEqual(rep.rollup.cache_creation_1h, 0)


class TestCost(unittest.TestCase):
    def test_no_prices_means_no_cost(self):
        args = at.build_arg_parser().parse_args([str(FIXTURE)])
        self.assertIsNone(at.PriceTable.from_args(args))

    def test_price_input_derives_cache_multipliers(self):
        args = at.build_arg_parser().parse_args(
            [str(FIXTURE), "--price-input", "10", "--price-output", "40"]
        )
        pt = at.PriceTable.from_args(args)
        self.assertIsNotNone(pt)
        self.assertEqual(pt.input, 10.0)
        self.assertEqual(pt.output, 40.0)
        self.assertAlmostEqual(pt.cache_read, 1.0)  # 0.1x
        self.assertAlmostEqual(pt.cache_write, 12.5)  # 1.25x

    def test_explicit_cache_prices_override(self):
        args = at.build_arg_parser().parse_args(
            [
                str(FIXTURE),
                "--price-input",
                "10",
                "--price-cache-read",
                "2",
                "--price-cache-write",
                "20",
            ]
        )
        pt = at.PriceTable.from_args(args)
        self.assertEqual(pt.cache_read, 2.0)
        self.assertEqual(pt.cache_write, 20.0)

    def test_cost_math(self):
        pt = at.PriceTable(input=10.0, output=40.0, cache_read=1.0, cache_write=12.5)
        t = at.TokenTotals(
            input_tokens=1_000_000,
            output_tokens=1_000_000,
            cache_read_input_tokens=1_000_000,
            cache_creation_input_tokens=1_000_000,
        )
        # 1M each at $10 + $40 + $1 + $12.5 per 1M = $63.5
        self.assertAlmostEqual(pt.cost_for(t), 63.5)

    def test_prices_from_json_file(self):
        with tempfile.TemporaryDirectory() as d:
            pj = Path(d) / "prices.json"
            pj.write_text(json.dumps({"input": 3, "output": 15}))
            args = at.build_arg_parser().parse_args(
                [str(FIXTURE), "--prices", str(pj)]
            )
            pt = at.PriceTable.from_args(args)
            self.assertEqual(pt.input, 3.0)
            self.assertEqual(pt.output, 15.0)
            self.assertAlmostEqual(pt.cache_read, 0.3)
            self.assertAlmostEqual(pt.cache_write, 3.75)


class TestRendering(unittest.TestCase):
    def setUp(self):
        self.report = at.parse_transcript(str(FIXTURE))

    def test_table_is_deterministic_and_labelled(self):
        out = at.render_table(self.report)
        self.assertIn("AGENT", out)
        self.assertIn("HIT%", out)
        self.assertIn("TOTAL (wave)", out)
        # attribution-qualified label for the largest sub-agent.
        self.assertIn("sparq-rust-feature:agentAAA", out)
        # honesty note must always be present.
        self.assertIn("NON-CANONICAL", out)
        # deterministic: same input -> byte-identical output.
        self.assertEqual(out, at.render_table(at.parse_transcript(str(FIXTURE))))

    def test_table_sorted_by_input_side_desc(self):
        out = at.render_table(self.report)
        body_lines = out.splitlines()
        a_idx = next(i for i, ln in enumerate(body_lines) if "agentAAA" in ln)
        b_idx = next(i for i, ln in enumerate(body_lines) if "agentBBB" in ln)
        # sub-agent A has more total input-side tokens than B -> appears first.
        self.assertLess(a_idx, b_idx)

    def test_json_structure_and_non_canonical_flag(self):
        out = at.build_json(self.report, None)
        self.assertTrue(out["non_canonical"])
        self.assertIn("NON-CANONICAL", out["note"])
        self.assertEqual(out["agent_count"], 3)
        self.assertEqual(out["subagent_count"], 2)
        self.assertEqual(len(out["agents"]), 3)
        self.assertEqual(out["rollup"]["cache_read_input_tokens"], 15000)
        # round-trips through json.dumps cleanly (no non-serialisable values).
        json.dumps(out)

    def test_json_includes_cost_when_priced(self):
        pt = at.PriceTable(input=10.0, output=40.0, cache_read=1.0, cache_write=12.5)
        out = at.build_json(self.report, pt)
        self.assertIn("estimated_cost_usd", out["rollup"])
        self.assertIn("prices_usd_per_mtok", out)
        for entry in out["agents"]:
            self.assertIn("estimated_cost_usd", entry)


class TestMainCli(unittest.TestCase):
    def test_main_writes_json_and_prints_table(self):
        with tempfile.TemporaryDirectory() as d:
            jp = Path(d) / "report.json"
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = at.main([str(FIXTURE), "--json", str(jp)])
            self.assertEqual(rc, 0)
            self.assertIn("TOTAL (wave)", buf.getvalue())
            data = json.loads(jp.read_text())
            self.assertEqual(data["agent_count"], 3)
            self.assertTrue(data["non_canonical"])

    def test_main_missing_file_returns_2(self):
        errbuf = io.StringIO()
        with redirect_stderr(errbuf):
            rc = at.main(["/nonexistent/transcript.jsonl"])
        self.assertEqual(rc, 2)
        self.assertIn("not found", errbuf.getvalue())

    def test_main_warns_on_empty(self):
        with tempfile.TemporaryDirectory() as d:
            empty = Path(d) / "empty.jsonl"
            empty.write_text('"just a string"\n')
            errbuf = io.StringIO()
            outbuf = io.StringIO()
            with redirect_stdout(outbuf), redirect_stderr(errbuf):
                rc = at.main([str(empty), "--no-table"])
            self.assertEqual(rc, 0)
            self.assertIn("no assistant/usage records", errbuf.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=2)
