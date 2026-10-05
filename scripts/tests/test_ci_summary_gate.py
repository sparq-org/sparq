#!/usr/bin/env python3
# [FABLE-5] Hermetic unit tests for the ci-summary gate poll loop
# (scripts/ci_summary_gate.py — bead sq-90cv4). Authored by Claude Fable 5.
#
# The bead's mandate: prove the ADAPTIVE SATURATION BUDGET preserves the gate's
# real pass/fail semantics EXACTLY. The four load-bearing cases:
#   (a) all-green                      => SUCCESS   (test_all_green_success)
#   (b) real gating failure           => FAILURE   (test_real_failure_fails)
#   (c) saturation w/ queued siblings => still-settling, NOT a false RED, and the
#       eventual verdict is the REAL one (test_saturation_extension_then_green /
#       test_failure_during_extension_still_fails)
#   (d) genuine hang (no progress, idle queue) => FAILURE
#       (test_genuine_hang_fails), and saturation-forever is BOUNDED: the absolute
#       cap still REDs (test_saturation_forever_bounded_red) — the extension can
#       never convert a real failure into a pass NOR wait forever.
# Plus the pre-existing semantics that must not regress: the sq-ipkku/#997
# terminal-injection settle guard + graceful timeout, the sq-wjth advisory
# word-boundary exclusion, the anchored self-run exclusion, and the empty-set pass.
#
# Fully hermetic: fetchers are injected (no gh, no network, no sleep).
# Run:  python3 scripts/tests/test_ci_summary_gate.py   (stdlib only; no pytest)

from __future__ import annotations

import importlib.util
import io
import json
import re
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.dont_write_bytecode = True  # keep repeated local runs hermetic (no stale .pyc)

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "ci_summary_gate.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("ci_summary_gate", SCRIPT)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ci_summary_gate"] = mod
    spec.loader.exec_module(mod)
    return mod


g = _load_module()

# [OPUS-5] #3773 — advisory status is DECLARED, never inferred from a name. Every
# fixture below that expects a leg to be non-gating must therefore be DECLARED here,
# exactly as a real `.github/advisory-registry.json` entry would declare it. Names NOT
# in this set GATE even when they carry an "advisory"/"informational" token — which is
# the property TestDeclaredAdvisoryRule pins.
DECLARED_ADVISORY = (
    "vale (prose, advisory)",
    "unsafe report (cargo-geiger, informational)",
    "markdownlint-advisory (whole repo)",
    "external-links (lychee online, advisory)",
    "GUI build + clippy (${{ matrix.label }}, advisory)",
)
# An advisory/informational NAME token with NO declaration — the #3773 defect fixture.
UNDECLARED_ADVISORY_NAME = "site determinism grep-gate (advisory)"
g.set_declared_advisory(DECLARED_ADVISORY)


def R(name, status="completed", conclusion="success", url="", started="", rid=0,
      external_id=""):
    # started/rid feed the draft-tier superseded-run ordering (started_at, id);
    # pre-draft-tier tests omit them and keep their exact semantics.
    # external_id carries the finding-2 reporter correlation token (the triggering
    # feature-matrix run id); pre-finding-2 tests omit it (empty => no correlation).
    return {"name": name, "status": status, "conclusion": conclusion,
            "details_url": url, "html_url": "", "started_at": started, "id": rid,
            "external_id": external_id}


def W(run_id, workflow_id, *, name="CI", status="completed", conclusion="success",
      created="2026-07-21T14:00:00Z", attempt=1):
    """Actions workflow-run fixture for the #3505 authoritative resolver."""
    return {
        "id": run_id,
        "workflow_id": workflow_id,
        "name": name,
        "path": f".github/workflows/{name.lower().replace(' ', '-')}.yml",
        "head_sha": "deadbeef",
        "status": status,
        "conclusion": conclusion,
        "created_at": created,
        "run_started_at": created,
        "run_attempt": attempt,
        "html_url": f"https://github.test/o/r/actions/runs/{run_id}",
    }


GREEN = R("build + test")
GREEN2 = R("clippy")
PENDING = R("coverage", status="queued", conclusion=None)
IN_PROGRESS = R("coverage", status="in_progress", conclusion=None)
RED = R("clippy", conclusion="failure")


def tiny_cfg(**over):
    """Small budgets so the loop phases are reachable in a handful of polls:
    base budget = 4 polls, absolute cap = 8, settle = 2, floor = 2."""
    base = dict(self_run_id="999", interval=0, min_polls=2, settle_polls=2,
                base_polls=4, sat_interval=0, max_total_polls=8,
                reporter_grace_polls=3,
                sat_queue_min=5, progress_window=2, max_consec_fetch_failures=3,
                summary_path="")
    base.update(over)
    return g.Config(**base)


def scripted(polls, repeat_last=True):
    """fetch_runs() stub: returns polls[i] on call i (an Exception instance is
    raised instead); repeats the final entry once exhausted."""
    state = {"i": 0, "calls": 0}

    def fetch():
        state["calls"] += 1
        i = min(state["i"], len(polls) - 1) if repeat_last else state["i"]
        state["i"] += 1
        entry = polls[i]
        if isinstance(entry, Exception):
            raise entry
        return list(entry)

    fetch.state = state
    return fetch


def run(cfg, polls, depth=0, tier_ctx=None):
    """Drive run_gate with scripted polls + a constant queue depth (None = the
    depth API is unavailable). Returns (exit_code, captured_output). tier_ctx
    (default None) exercises the draft-tier integrity semantics."""
    out = io.StringIO()
    fetch = scripted(polls)

    def depth_fn():
        return depth

    with redirect_stdout(out):
        code = g.run_gate(cfg, fetch, depth_fn, sleep_fn=lambda s: None,
                          tier_ctx=tier_ctx)
    return code, out.getvalue()


class TestVerdictSemantics(unittest.TestCase):
    """The core pass/fail semantics — unchanged by the adaptive budget."""

    def test_all_green_success(self):
        code, out = run(tiny_cfg(), [[GREEN, GREEN2]])
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out)

    def test_real_failure_fails(self):
        code, out = run(tiny_cfg(), [[GREEN, RED]])
        self.assertEqual(code, 1)
        self.assertIn("FAILED", out)
        self.assertIn("clippy", out)

    def test_skipped_and_neutral_pass(self):
        runs = [R("docs", conclusion="skipped"), R("wasm", conclusion="neutral"), GREEN]
        code, _ = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0)

    def test_empty_set_passes(self):
        code, out = run(tiny_cfg(), [[]])
        self.assertEqual(code, 0)
        self.assertIn("stable empty set", out)

    def test_declared_advisory_failure_excluded(self):
        runs = [R("vale (prose, advisory)", conclusion="failure"),
                R("unsafe report (cargo-geiger, informational)", conclusion="failure"),
                GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0)
        self.assertIn("2 advisory check(s) excluded", out)

    def test_advisories_plural_still_gates(self):
        # sq-wjth: "cargo-deny (advisories, …)" is not declared, so it GATES. (Under
        # the removed name rule this depended on a word boundary; it is now simply the
        # default for everything undeclared.)
        runs = [R("cargo-deny (advisories, bans, licenses, sources)", conclusion="failure")]
        code, _ = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)

    def test_incomplete_conclusion_fails(self):
        # A terminal-status run with a null conclusion renders as "incomplete" and gates.
        runs = [R("weird", conclusion=None)]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)
        self.assertIn("incomplete", out)


class TestSettleAndGracefulTimeout(unittest.TestCase):
    """The sq-ipkku settle guard + the #997 graceful timeout must not regress."""

    def test_terminal_injection_does_not_starve_settle(self):
        polls = [
            [GREEN, PENDING],                    # pending holds the gate
            [GREEN, R("coverage")],              # all terminal, stable=1
            [GREEN, R("coverage"), R("late")],   # terminal INJECTION: stable=2 (no reset)
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out)
        self.assertNotIn("::error::", out)

    def test_graceful_timeout_all_green_passes(self):
        # pending flip-flops so a full quiet settle never happens; depth is HIGH so
        # the pending polls past the base budget extend instead of hanging-RED; the
        # ABSOLUTE budget then renders the real (green) verdict — the #997 fix.
        flip = [[GREEN, PENDING], [GREEN, R("coverage")]] * 4
        code, out = run(tiny_cfg(), flip, depth=20)
        self.assertEqual(code, 0)
        self.assertIn("rendering the verdict on the final all-terminal set", out)

    def test_graceful_timeout_real_failure_still_fails(self):
        flip = [[GREEN, PENDING], [GREEN, R("coverage")]] * 3 \
            + [[GREEN, PENDING], [GREEN, R("coverage", conclusion="failure")]]
        code, out = run(tiny_cfg(), flip, depth=20)
        self.assertEqual(code, 1)
        self.assertIn("FAILED", out)


class TestFailFast(unittest.TestCase):
    """[FABLE-5] Fail-fast on a concluded gating failure (2026-07-17 maintainer
    directive): the gate REDs the moment a gating leg's failure is confirmed by
    the grace re-poll, instead of waiting out every other sibling — WITHOUT ever
    firing on a superseded/forgiven run, an in-flight rerun's predecessor, or an
    advisory leg. Each guard's mutation is caught: dropping fail-fast breaks the
    immediacy assertions; dropping a guard turns a must-pass scenario red."""

    def test_gating_failure_with_others_pending_reds_immediately(self):
        # (i) One concluded gating failure + siblings still pending => FAILURE at
        # the grace-confirm poll (attempt 2), not after the pending legs settle.
        # Mutation coverage: without fail-fast these polls run to the base budget
        # and red as a "genuine hang" — the fail-fast marker + the poll-count
        # assertions below then fail.
        code, out = run(tiny_cfg(), [[RED, PENDING]])
        self.assertEqual(code, 1)
        self.assertIn("FAILED (fail-fast)", out)
        self.assertIn("clippy", out)
        self.assertIn("attempt 2", out)
        self.assertNotIn("attempt 3", out, "the red must land at the grace-confirm poll")
        self.assertNotIn("genuine hang", out)

    def test_failed_leg_with_inflight_same_name_rerun_does_not_fail_fast(self):
        # (ii) A red leg whose same-name rerun is already IN PROGRESS must NOT
        # fail-fast — the gate waits on the rerun (which wins: the check-runs
        # listing returns the latest attempt, so the old failure drops out).
        # Mutation coverage: without the in-flight guard the identical failure
        # is observed on polls 1+2 and the gate REDs before the rerun lands.
        failed = R("clippy", conclusion="failure", started="2026-07-17T10:00:00Z", rid=1)
        rerun = R("clippy", status="in_progress", conclusion=None,
                  started="2026-07-17T10:05:00Z", rid=2)
        rerun_green = R("clippy", started="2026-07-17T10:05:00Z", rid=2)
        polls = [
            [failed, rerun, PENDING],
            [failed, rerun, PENDING],
            [rerun_green, R("coverage")],
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertNotIn("fail-fast", out)

    def test_cancelled_race_loser_select_mid_poll_does_not_fail_fast(self):
        # (iii) A concurrency-cancel race-loser select observed MID-POLL (siblings
        # still pending) must not fail-fast the gate: forgive_superseded drops it
        # upstream of the fail-fast scan (same-tier same-name success, the
        # sq-fmx4u.3 pure-select rule), and a cancelled conclusion is not
        # `failure` in any case. The eventual verdict is the real (green) one.
        sel_win = R(SELECT_NAME, conclusion="success",
                    started="2026-07-17T10:00:10Z", rid=2)
        sel_lost = R(SELECT_NAME, conclusion="cancelled",
                     started="2026-07-17T10:00:20Z", rid=3)
        polls = [
            [sel_win, sel_lost, PENDING],
            [sel_win, sel_lost, R("coverage")],
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertNotIn("fail-fast", out)

    def test_cancelled_leg_awaiting_rerun_does_not_fail_fast(self):
        # (iii-b) spec guard (a): a cancelled-then-rerun leg mid-poll — cancelled
        # is never a fail-fast trigger, the successor supersedes it, gate greens.
        old = R("build + test", conclusion="cancelled",
                started="2026-07-17T10:00:00Z", rid=1)
        new_running = R("build + test", status="in_progress", conclusion=None,
                        started="2026-07-17T10:05:00Z", rid=2)
        new_green = R("build + test", started="2026-07-17T10:05:00Z", rid=2)
        polls = [[old, PENDING], [old, new_running, PENDING], [old, new_green, R("coverage")]]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertNotIn("fail-fast", out)

    def test_advisory_failure_mid_poll_does_not_fail_fast(self):
        # (iv) An advisory leg's failure while siblings are pending must neither
        # fail-fast nor gate at all (sq-wjth) — the verdict stays green.
        adv = R("vale (prose, advisory)", conclusion="failure")
        polls = [[adv, PENDING], [adv, R("coverage")]]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertNotIn("fail-fast", out)

    def test_grace_repoll_dodges_a_transient_failure_read(self):
        # The grace re-poll: a failure observed ONCE that a fresh fetch does not
        # re-observe (an API read race) must not red the gate.
        polls = [
            [RED, PENDING],                 # racy read: clippy "failure"
            [GREEN2, PENDING],              # fresh fetch: clippy is green
            [GREEN2, R("coverage")],
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertIn("grace re-poll", out)
        self.assertNotIn("FAILED (fail-fast)", out)

    def test_all_terminal_failure_keeps_the_normal_render_path(self):
        # pending == 0 => fail-fast stands down and the classic settle + full
        # render_verdict path (with its richer message) is byte-identical.
        code, out = run(tiny_cfg(), [[GREEN, RED]])
        self.assertEqual(code, 1)
        self.assertIn("non-passing gating check(s)", out)
        self.assertNotIn("fail-fast", out)

    def test_fail_fast_never_turns_a_verdict_green(self):
        # Exit-0 invariant: fail-fast is an exit-1-only path — a green set with
        # pending work must still settle normally and pass.
        polls = [[GREEN, PENDING], [GREEN, R("coverage")]]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0)
        self.assertNotIn("fail-fast", out)


class TestAdaptiveSaturationBudget(unittest.TestCase):
    """sq-90cv4: saturation extends, hangs RED, real verdicts are untouched."""

    def test_saturation_extension_then_green(self):
        # Siblings stay QUEUED through the base budget while the repo queue is deep
        # (the congestion-collapse shape) — the gate must extend, then pass for real.
        polls = [[GREEN, GREEN2, PENDING]] * 5 + [[GREEN, GREEN2, R("coverage")]]
        code, out = run(tiny_cfg(), polls, depth=20)
        self.assertEqual(code, 0)
        self.assertIn("Extending the wait (adaptive budget", out)
        self.assertIn("PASSED", out)
        self.assertNotIn("::error::", out)

    def test_genuine_hang_fails(self):
        # QUEUED forever, idle queue, zero progress => RED at the base budget.
        # [OPUS-5] #3783: this fixture used to await an `in_progress` sibling, which
        # is precisely the misfire the liveness veto fixes — a leg that is EXECUTING
        # is not hung. The genuine-hang shape is a leg that never STARTED (queued,
        # which is also what an evaporated check-run resolves to — #3677).
        polls = [[GREEN, PENDING]]
        code, out = run(tiny_cfg(), polls, depth=0)
        self.assertEqual(code, 1)
        self.assertIn("genuine hang", out)

    def test_saturation_forever_bounded_red(self):
        # The extension is BOUNDED: perpetual saturation still REDs at the absolute
        # cap — never an infinite wait, never a synthesized pass.
        polls = [[GREEN, PENDING]]
        code, out = run(tiny_cfg(), polls, depth=20)
        self.assertEqual(code, 1)
        self.assertIn("ABSOLUTE budget", out)

    def test_failure_during_extension_still_fails(self):
        # The adaptive budget must NEVER convert a real failure into a pass.
        polls = [[GREEN, PENDING]] * 5 + [[GREEN, R("coverage", conclusion="failure")]]
        code, out = run(tiny_cfg(), polls, depth=20)
        self.assertEqual(code, 1)
        self.assertIn("FAILED", out)

    def test_progress_signal_extends_when_depth_unknown(self):
        # Queue-depth API unavailable (None) but completions keep landing => the
        # progress signal alone extends; the eventual verdict is real.
        polls = [
            [R("a"), R("b"), PENDING, R("d", status="queued", conclusion=None),
             R("e", status="queued", conclusion=None), R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d", status="queued", conclusion=None),
             R("e", status="queued", conclusion=None), R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d"),
             R("e", status="queued", conclusion=None), R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d"), R("e"),
             R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d"), R("e"), R("f")],
        ]
        code, out = run(tiny_cfg(), polls, depth=None)
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out)

    def test_no_progress_and_depth_unknown_is_a_hang(self):
        # Unknown depth + flat completions + nothing EXECUTING must NOT extend
        # forever: it REDs at the base budget exactly like the old behaviour
        # (fail-closed on no evidence). [OPUS-5] #3783: queued, not in_progress.
        polls = [[GREEN, PENDING]]
        code, out = run(tiny_cfg(), polls, depth=None)
        self.assertEqual(code, 1)
        self.assertIn("genuine hang", out)


# [OPUS-5] The live #3783 shape: `gate` run 30149978128 on main (e8a41ab8c) declared a
# "genuine hang" while these three legs had been executing for 11 minutes.
KANI_LIVE = [
    R("kani sparq-core (dict mmap validator totality) (bounded proofs, informational)",
      status="in_progress", conclusion=None, started="2026-07-25T08:20:50Z", rid=1),
    R("kani sparq-engine (vectorized reducer kernels) (bounded proofs, informational)",
      status="in_progress", conclusion=None, started="2026-07-25T08:20:49Z", rid=2),
    R("kani sparq-vectors (.spqv validator) (bounded proofs, informational)",
      status="in_progress", conclusion=None, started="2026-07-25T08:20:50Z", rid=3),
]


class TestLivenessVeto(unittest.TestCase):
    """[OPUS-5] #3783 — an EXECUTING sibling is positive liveness evidence and must
    veto the genuine-hang verdict; a sibling that never STARTED must still be
    detected as a hang. That discrimination is the whole fix, so both directions are
    pinned here (deleting the veto REDs the first group; broadening it past
    `in_progress` REDs the second)."""

    def test_in_progress_siblings_are_not_a_hang(self):
        # The exact incident: idle queue (depth=0) + zero completions + three kani
        # legs EXECUTING. Both hang signals are satisfied by a healthy bounded proof.
        code, out = run(tiny_cfg(), [[GREEN] + KANI_LIVE], depth=0)
        self.assertNotIn(
            "genuine hang", out,
            "#3783 REGRESSION: the hang detector fired while awaited siblings were "
            "`in_progress` — an executing kani bounded proof was declared a hang. An "
            "idle Actions queue means those jobs DEQUEUED and STARTED, and kani emits "
            "no completion for tens of minutes by design, so neither signal is "
            "evidence of a hang. The liveness veto (live_siblings) is missing.",
        )
        self.assertIn("sibling(s) EXECUTING", out)
        # Bounded, and never green: the veto only postpones the red to the cap.
        self.assertEqual(code, 1, out)

    def test_one_live_sibling_among_queued_ones_vetoes(self):
        # The real set was mixed (some queued, some running). ONE executing sibling
        # is enough liveness evidence: nothing is demonstrably lost.
        code, out = run(tiny_cfg(), [[GREEN, PENDING, KANI_LIVE[0]]], depth=0)
        self.assertNotIn(
            "genuine hang", out,
            "#3783 REGRESSION: a mixed queued+in_progress sibling set was declared a "
            "hang; one executing leg proves the pipeline is alive.",
        )
        self.assertEqual(code, 1, out)

    def test_all_queued_siblings_are_still_a_hang(self):
        # THE DISCRIMINATION: nothing executing, idle queue, no completions => the
        # detector must still fire. This is what keeps the veto from blinding it.
        code, out = run(tiny_cfg(), [[GREEN, PENDING,
                                      R("shard-2", status="queued", conclusion=None)]],
                        depth=0)
        self.assertEqual(code, 1)
        self.assertIn(
            "genuine hang", out,
            "#3783 OVER-CORRECTION: awaited siblings that never STARTED (all `queued`, "
            "idle queue, no completions) are the genuine-hang case the detector exists "
            "for (#3677 evaporated check-runs resolve to `queued` too). The veto must "
            "key on `in_progress` ONLY — it has been broadened to swallow non-running "
            "legs, so a real hang now waits out the whole budget instead of REDing.",
        )

    def test_evaporated_check_run_resolves_to_queued_not_live(self):
        # #3677: an evaporated/pending leg is represented by the run-level synthetic
        # with force_pending=True. Its status MUST NOT read as liveness, or the hang
        # detector loses the only case it was built for.
        synthetic = g._workflow_summary_check(W(700, 7, status="in_progress",
                                                conclusion=None), force_pending=True)
        self.assertEqual(
            synthetic["status"], "queued",
            "#3677/#3783 REGRESSION: the force_pending run-level synthetic (how an "
            "EVAPORATED check-run is represented) no longer reports `queued`. If it "
            "reports `in_progress` it looks EXECUTING to the liveness veto, so a lost "
            "leg vetoes its own hang detection and the gate waits out the whole budget "
            "instead of REDing — the exact case the detector was built for.",
        )
        self.assertEqual(
            g.live_siblings([synthetic]), [],
            "#3677 REGRESSION: the force_pending run-level synthetic (an evaporated "
            "check-run) is being counted as an EXECUTING sibling, so a lost leg would "
            "veto its own hang detection and never RED.",
        )

    def test_live_siblings_counts_in_progress_only(self):
        # The predicate itself: only `in_progress` is liveness. `queued`/`waiting`/
        # `requested`/`pending` all mean the leg has not started.
        rows = [R("a", status="in_progress", conclusion=None),
                R("b", status="queued", conclusion=None),
                R("c", status="waiting", conclusion=None),
                R("d", status="requested", conclusion=None),
                R("e", status="pending", conclusion=None),
                R("f")]
        self.assertEqual(
            [r["name"] for r in g.live_siblings(rows)], ["a"],
            "#3783: live_siblings must select EXACTLY the `in_progress` rows — a "
            "`queued`/`waiting`/`requested`/`pending` leg has not started and is what "
            "an idle Actions queue is evidence about.",
        )

    def test_veto_never_turns_a_verdict_green(self):
        # Exit-0 invariant: the veto is a POSTPONEMENT, never a pass. Siblings that
        # execute forever exhaust the absolute cap and still exit non-zero.
        code, _ = run(tiny_cfg(), [[GREEN] + KANI_LIVE], depth=0)
        self.assertEqual(code, 1, "the liveness veto must never synthesise a pass")

    def test_real_failure_beside_a_live_sibling_still_reds(self):
        # The veto must not delay a real red: fail-fast still fires while a kani
        # proof runs.
        polls = [[RED] + KANI_LIVE, [RED] + KANI_LIVE]
        code, out = run(tiny_cfg(), polls, depth=0)
        self.assertEqual(code, 1)
        self.assertIn("FAILED (fail-fast)", out)


class TestVerdictTaxonomy(unittest.TestCase):
    """[OPUS-5] #3783 ask 3 — "the gate could not determine an answer" must not read
    like "a gating check failed". Both still exit 1 (fail-closed); only the words
    differ, and that difference is what stops a budget expiry being diagnosed as a
    broken tree (#3758/#3765/#3781/#3783)."""

    def test_budget_expiry_with_live_siblings_reads_undetermined(self):
        code, out = run(tiny_cfg(), [[GREEN] + KANI_LIVE], depth=0)
        self.assertEqual(code, 1, "fail-closed: an unobserved leg is never assumed green")
        self.assertIn(
            "UNDETERMINED (not a test failure)", out,
            "#3783 ask 3 REGRESSION: a budget expiry with siblings STILL EXECUTING is "
            "reported identically to a real gating failure. Nothing was shown to be "
            "broken — this outcome must say 'could not determine', not 'FAILED'.",
        )
        self.assertNotIn("### ci-summary: FAILED", out)
        # It must name WHICH legs were alive, so the reader is not sent hunting.
        self.assertIn("kani sparq-core", out)

    def test_saturation_absolute_budget_reads_undetermined(self):
        # The other could-not-determine: the runner pool never drained.
        code, out = run(tiny_cfg(), [[GREEN, PENDING]], depth=20)
        self.assertEqual(code, 1)
        self.assertIn("ABSOLUTE budget", out)
        self.assertIn(
            "UNDETERMINED (not a test failure)", out,
            "#3783 ask 3 REGRESSION: absolute-budget exhaustion under runner "
            "saturation reports as a failure though no gating check was shown to "
            "fail; it is a could-not-determine.",
        )

    def test_genuine_hang_still_reads_as_a_failure(self):
        # The taxonomy discriminates in BOTH directions: nothing executing + idle
        # queue + no completions IS a broken pipeline and keeps failing loudly.
        code, out = run(tiny_cfg(), [[GREEN, PENDING]], depth=0)
        self.assertEqual(code, 1)
        self.assertIn("genuine hang", out)
        self.assertNotIn(
            "UNDETERMINED", out,
            "#3783: a genuine hang must NOT be softened to 'could not determine' — "
            "nothing running with an idle queue and no completions is a real defect.",
        )

    def test_a_real_gating_failure_is_never_called_undetermined(self):
        code, out = run(tiny_cfg(), [[GREEN, RED]])
        self.assertEqual(code, 1)
        self.assertIn("FAILED", out)
        self.assertNotIn(
            "UNDETERMINED", out,
            "#3783: a genuinely failing gating check must keep reading as FAILED — the "
            "taxonomy must not launder a real defect into 'could not determine'.",
        )


class TestDescopedEc2Lane(unittest.TestCase):
    """[OPUS-5] #3784 — the AWS OIDC role `bench-ec2.yml` assumes was DELIBERATELY
    DESCOPED, so every automated tick of that workflow was a guaranteed GATING failure
    on `main` (12 credential retries, before any benchmark ran). The chosen fix is
    RETIREMENT of the automated lane, NOT an advisory-registry mute: #3774 landed the
    rule that advisory status is declared deliberately, and muting a permanently-broken
    gate to green a dashboard is the behaviour that rule exists to prevent. These tests
    pin BOTH halves — the lane cannot fire automatically, AND it was not muted."""

    WORKFLOW = REPO_ROOT / ".github" / "workflows" / "bench-ec2.yml"
    REGISTRY = REPO_ROOT / ".github" / "advisory-registry.json"

    def _wf(self) -> dict:
        # PyYAML is installed in the job that runs this file (docs-quality quick-gates);
        # the raw-text guards in test_the_permanently_failing_check_name_is_gone hold
        # unconditionally, so the two load-bearing properties gate even without it.
        try:
            import yaml
        except ImportError:  # pragma: no cover - PyYAML is present in CI
            self.skipTest("PyYAML unavailable")
        # `on:` is parsed by PyYAML 1.1 as the boolean True.
        doc = yaml.safe_load(self.WORKFLOW.read_text(encoding="utf-8"))
        doc["_on"] = doc.get(True, doc.get("on"))
        return doc

    def test_no_schedule_trigger(self):
        doc = self._wf()
        self.assertNotIn(
            "schedule", doc["_on"],
            "#3784 REGRESSION: bench-ec2.yml runs on a `schedule:` again. Its AWS OIDC "
            "role is DESCOPED, so every cron tick fails in `Configure AWS credentials "
            "(OIDC)` before any benchmark runs, posts a GATING check-run on main's head "
            "SHA (no advisory token, no registry entry => it GATES per #3773/#3774), and "
            "makes `main` permanently and uninformatively red. Re-adding a cadence "
            "requires re-provisioning vars.AWS_BENCH_ROLE_ARN in the same change.",
        )
        self.assertIn("workflow_dispatch", doc["_on"],
                      "maintainer-run EC2 benchmarking is NOT descoped and must stay dispatchable")

    def test_both_campaigns_stay_reachable_by_dispatch(self):
        # Retiring the cron must not delete a capability: BOTH lanes stay selectable,
        # so #3488's maintainer-run EC2 benchmarking keeps working once the role exists.
        doc = self._wf()
        options = doc["_on"]["workflow_dispatch"]["inputs"]["lane"]["options"]
        self.assertEqual(sorted(options), ["full-suite", "heavy"])
        conds = {jid: " ".join(job["if"].split()) for jid, job in doc["jobs"].items()}
        self.assertIn("github.event.inputs.lane == 'heavy'", conds["ec2-bench"])
        self.assertIn("github.event.inputs.lane == 'full-suite'", conds["nightly-full-bench"])

    def test_every_oidc_job_carries_the_role_present_guard(self):
        doc = self._wf()
        for jid, job in doc["jobs"].items():
            uses = [s.get("uses", "") for s in job.get("steps", [])]
            if not any("configure-aws-credentials" in u for u in uses):
                continue
            self.assertIn(
                "vars.AWS_BENCH_ROLE_ARN != ''", " ".join(job.get("if", "").split()),
                f"#3784 REGRESSION: job `{jid}` assumes the DESCOPED AWS OIDC role with "
                f"no role-present guard, so a dispatch while the role is absent burns 12 "
                f"credential retries and posts a gating failure instead of skipping.",
            )

    def test_the_lane_was_not_muted_via_the_advisory_registry(self):
        # The easy path we deliberately did NOT take. If a future change wants advisory
        # status for this lane it must DELETE this assertion with a written justification
        # — which is exactly the deliberate declaration #3774 demands.
        declared = json.loads(self.REGISTRY.read_text(encoding="utf-8"))["jobs"]
        for key, entry in declared.items():
            self.assertNotEqual(
                entry.get("workflow"), "bench-ec2.yml",
                f"#3784: `{key}` declares a bench-ec2.yml job advisory. The descoped "
                f"OIDC lane was RETIRED, not muted — declaring a permanently-failing "
                f"gate advisory to green the dashboard is precisely what #3774's "
                f"declared-not-inferred rule exists to prevent.",
            )

    def test_the_permanently_failing_check_name_is_gone(self):
        # The gate has no name-based escape hatch (#3773), so the ONLY way this check-run
        # stops gating `main` is for it to stop being produced.
        text = self.WORKFLOW.read_text(encoding="utf-8")
        self.assertNotIn(
            "name: nightly full-suite benchmark on EC2 (spot)", text,
            "#3784 REGRESSION: the check-run name that could never pass is back. It "
            "carries no advisory token and no registry declaration, so it GATES.",
        )
        # And the retirement is a behaviour change, not a rename: no automated trigger
        # can produce the check-run at all. (Raw text, so this holds without PyYAML.)
        self.assertNotIn(
            "cron:", text,
            "#3784 REGRESSION: a cron re-appeared in bench-ec2.yml — an automated tick "
            "against the descoped AWS OIDC role is a guaranteed gating failure on main.",
        )
        guard_lines = [ln for ln in text.splitlines()
                       if ln.strip() == "vars.AWS_BENCH_ROLE_ARN != ''"]
        self.assertEqual(
            len(guard_lines), 2,
            "#3784 REGRESSION: both EC2 jobs must carry the role-present guard, so a "
            "dispatch while the AWS OIDC role is descoped SKIPS instead of failing at "
            "`Configure AWS credentials (OIDC)` and posting a gating check-run.",
        )


class TestSelfExclusionAndFetch(unittest.TestCase):
    def test_self_run_excluded_by_anchored_id(self):
        me = R("gate", status="in_progress", conclusion=None,
               url="https://github.com/o/r/actions/runs/999/job/1")
        sibling = R("build + test", url="https://github.com/o/r/actions/runs/9991/job/2")
        # Without the exclusion the pending self would deadlock the gate; with it,
        # only the green sibling counts.
        code, _ = run(tiny_cfg(), [[me, sibling]])
        self.assertEqual(code, 0)

    def test_anchoring_does_not_match_prefix_ids(self):
        self.assertTrue(g.is_self({"details_url": "https://x/actions/runs/999/job/4"}, "999"))
        self.assertTrue(g.is_self({"details_url": "https://x/actions/runs/999"}, "999"))
        self.assertFalse(g.is_self({"details_url": "https://x/actions/runs/9991/job/4"}, "999"))
        self.assertFalse(g.is_self({"html_url": "https://x/actions/runs/1999/job/4"}, "999"))
        # html_url fallback when details_url is absent.
        self.assertTrue(g.is_self({"html_url": "https://x/actions/runs/999/job/4"}, "999"))

    def test_transient_fetch_failures_tolerated(self):
        polls = [g.FetchError("blip"), g.FetchError("blip"), [GREEN], [GREEN]]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0)
        self.assertIn("skipping this poll", out)

    def test_persistent_fetch_failures_fail_closed(self):
        polls = [g.FetchError("down")]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 1)
        self.assertIn("consecutive check-run fetch failures", out)


class TestNewestWorkflowRunResolution(unittest.TestCase):
    """[GPT-5.6] #3505: workflow identity/attempt, not stale check presence, wins."""

    def test_newest_order_prefers_new_run_id_then_same_run_attempt(self):
        same_time = "2026-07-21T14:00:00Z"
        old_rerun = W(101, 7, created=same_time, attempt=2)
        new_run = W(102, 7, created=same_time, attempt=1)
        self.assertEqual(g.newest_workflow_runs([old_rerun, new_run])["id:7"]["id"], 102)
        attempt_one = W(102, 7, created=same_time, attempt=1)
        attempt_two = W(102, 7, created=same_time, attempt=2)
        self.assertEqual(
            g.newest_workflow_runs([attempt_one, attempt_two])["id:7"]["run_attempt"], 2
        )

    def test_superseded_cancelled_and_failure_are_non_events(self):
        old_cancel = W(101, 7, conclusion="cancelled", created="2026-07-21T13:00:00Z")
        old_failure = W(102, 7, conclusion="failure", created="2026-07-21T13:30:00Z")
        newest = W(103, 7, created="2026-07-21T14:00:00Z")
        checks = [
            R("test shard", conclusion="cancelled", rid=1001,
              url="https://github.test/o/r/actions/runs/101/job/1"),
            R("test shard", conclusion="failure", rid=1002,
              url="https://github.test/o/r/actions/runs/102/job/2"),
        ]
        resolved, dropped = g.resolve_newest_workflow_runs(
            checks, [old_cancel, old_failure, newest], "999"
        )
        self.assertEqual(dropped, 2)
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 0, out)
        self.assertNotIn("cancelled", [r.get("conclusion") for r in resolved])
        self.assertNotIn("failure", [r.get("conclusion") for r in resolved])

    def test_newest_failure_still_fails_when_job_check_evaporated(self):
        newest = W(103, 7, conclusion="failure")
        resolved, _ = g.resolve_newest_workflow_runs([], [newest], "999")
        self.assertEqual(len(resolved), 1, "run-level failure evidence must be synthesized")
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 1)
        self.assertIn("workflow-run verdict (id:7)", out)

    def test_newest_failure_cannot_be_advisory_excluded_by_workflow_name(self):
        newest = W(103, 7, name="all advisory", conclusion="failure")
        resolved, _ = g.resolve_newest_workflow_runs([], [newest], "999")
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 1, out)

    def test_authoritative_advisory_job_failure_remains_non_gating(self):
        newest = W(103, 7, conclusion="failure")
        advisory_job = {
            "id": 2001,
            "name": "vale (prose, advisory)",
            "status": "completed",
            "conclusion": "failure",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/103/job/2",
        }
        resolved, _ = g.resolve_newest_workflow_runs(
            [], [newest], "999", attempt_jobs={103: [advisory_job]}
        )
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 0, out)

    def test_green_run_with_evaporated_check_resolves_without_hang(self):
        newest = W(103, 7)
        resolved, _ = g.resolve_newest_workflow_runs([], [newest], "999")
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_completed_jobs_listing_recovers_evaporated_required_leg(self):
        newest = W(103, 7, conclusion="failure")
        failed_job = {
            "id": 2001,
            "name": "required test shard",
            "status": "completed",
            "conclusion": "failure",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/103/job/2",
        }
        resolved, _ = g.resolve_newest_workflow_runs(
            [], [newest], "999", attempt_jobs={103: [failed_job]}
        )
        self.assertEqual([r["name"] for r in resolved], ["required test shard"])
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 1, out)

    def test_evaporated_feature_group_still_requires_reporter(self):
        newest = W(103, 7)
        group_job = {
            "id": 2001,
            "name": "opt-in group (0)",
            "status": "completed",
            "conclusion": "success",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/103/job/2",
        }
        resolved, _ = g.resolve_newest_workflow_runs(
            [], [newest], "999", attempt_jobs={103: [group_job]}
        )
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 1)
        self.assertIn("reporter verdict never landed", out)

    def test_rerun_attempt_uses_attempt_jobs_not_old_same_run_id_checks(self):
        rerun = W(103, 7, attempt=2)
        old_failure = R(
            "test shard", conclusion="failure", rid=1001,
            url="https://github.test/o/r/actions/runs/103/job/1",
        )
        latest_job = {
            "id": 2001,
            "name": "test shard",
            "status": "completed",
            "conclusion": "success",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/103/job/2",
        }
        resolved, dropped = g.resolve_newest_workflow_runs(
            [old_failure], [rerun], "999", attempt_jobs={103: [latest_job]}
        )
        self.assertEqual(dropped, 1)
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 0, out)

    def test_rerun_attempt_keeps_only_current_manually_posted_run_checks(self):
        rerun = W(103, 7, attempt=2)
        rerun["run_started_at"] = "2026-07-21T14:05:00Z"
        old_report = R(
            g.FM_REPORT_NAME, started="2026-07-21T14:00:00Z", rid=1001,
            url="https://github.test/o/r/actions/runs/103", external_id="103",
        )
        current_report = R(
            g.FM_REPORT_NAME, started="2026-07-21T14:06:00Z", rid=1002,
            url="https://github.test/o/r/actions/runs/103", external_id="103",
        )
        resolved, dropped = g.resolve_newest_workflow_runs(
            [old_report, current_report], [rerun], "999", attempt_jobs={103: []}
        )
        reports = [r for r in resolved if r.get("name") == g.FM_REPORT_NAME]
        self.assertEqual([r["id"] for r in reports], [1002])
        self.assertEqual(dropped, 1)

    def test_duplicate_job_names_do_not_cross_workflow_identity(self):
        a = W(101, 7, name="CI A")
        b = W(102, 8, name="CI B")
        a_failure = R(
            "shared job", conclusion="failure", rid=1001,
            url="https://github.test/o/r/actions/runs/101/job/1",
        )
        b_running = R(
            "shared job", status="in_progress", conclusion=None, rid=1002,
            url="https://github.test/o/r/actions/runs/102/job/2",
        )
        resolved, _ = g.resolve_newest_workflow_runs(
            [a_failure, b_running], [a, b], "999"
        )
        self.assertEqual(len(g.failfast_failures(resolved)), 1,
                         "another workflow's same-name in-flight job must not mask failure")

    def test_cancelled_auto_redispatch_is_bounded_once(self):
        cancelled = W(104, 7, conclusion="cancelled")
        posts = []
        resolver = g.WorkflowRunResolver(
            self_run_id="999",
            fetch_checks=lambda: [],
            fetch_workflows=lambda: [cancelled],
            fetch_attempt_jobs=lambda run_id, attempt: [],
            redispatch=lambda run_id: posts.append(run_id),
            redispatch_settle_polls=2,
        )
        first = resolver()
        self.assertTrue(any(r.get("status") != "completed" for r in first),
                        "a dispatched cancellation must become pending, not failure")
        resolver()
        with self.assertRaisesRegex(g.SupersededLegsError, "superseded-legs"):
            resolver()
        self.assertEqual(posts, [104], "API lag must never cause a second POST")

    def test_completed_run_jobs_are_authoritative_and_cached(self):
        complete = W(104, 7)
        calls = []
        job = {
            "id": 2001,
            "name": "required test shard",
            "status": "completed",
            "conclusion": "success",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/104/job/2",
        }

        def fetch_jobs(run_id, attempt):
            calls.append((run_id, attempt))
            return [job]

        resolver = g.WorkflowRunResolver(
            self_run_id="999",
            fetch_checks=lambda: [],
            fetch_workflows=lambda: [complete],
            fetch_attempt_jobs=fetch_jobs,
            redispatch=lambda run_id: self.fail("green run must not redispatch"),
        )
        first = resolver()
        second = resolver()
        self.assertEqual([r["name"] for r in first], ["required test shard"])
        self.assertEqual([r["name"] for r in second], ["required test shard"])
        self.assertEqual(calls, [(104, 1)], "terminal job inventory should be cached")

    def test_cancelled_retry_attempt_fails_loud_without_third_attempt(self):
        cancelled_retry = W(104, 7, conclusion="cancelled", attempt=2)
        posts = []
        resolver = g.WorkflowRunResolver(
            self_run_id="999",
            fetch_checks=lambda: [],
            fetch_workflows=lambda: [cancelled_retry],
            fetch_attempt_jobs=lambda run_id, attempt: [],
            redispatch=lambda run_id: posts.append(run_id),
        )
        with self.assertRaisesRegex(g.SupersededLegsError, "attempt 2"):
            resolver()
        self.assertEqual(posts, [])

    def test_unrecoverable_cancellation_uses_distinct_loud_gate_message(self):
        code, out = run(
            tiny_cfg(),
            [g.SupersededLegsError("superseded-legs, re-run required (#3505): fixture")],
        )
        self.assertEqual(code, 1)
        self.assertIn("superseded-legs, re-run required", out)
        self.assertIn("did not dispatch it more than once", out)


class TestAdvisoryRule(unittest.TestCase):
    def test_declared_names_are_excluded(self):
        self.assertTrue(g.is_advisory("markdownlint-advisory (whole repo)"))
        self.assertTrue(g.is_advisory("external-links (lychee online, advisory)"))
        self.assertTrue(g.is_advisory("unsafe report (cargo-geiger, informational)"))
        self.assertFalse(g.is_advisory("cargo-deny (advisories, bans, licenses, sources)"))
        self.assertFalse(g.is_advisory("build + test"))
        self.assertFalse(g.is_advisory("gate"))


# [OPUS-5] #3773 — ADVISORY MUST BE DECLARED, NOT INFERRED FROM A JOB NAME.
# The gate used to drop any check-run whose DISPLAY NAME matched
# `\b(advisory|informational)\b`. That silently neutralised four REAL gates, so
# `gate: SUCCESS` over-promised on every merge it authorised. These tests are the
# regression barrier for the fix and are MUTATION-CHECKED: restoring
#   is_advisory = lambda n: bool(ADVISORY_NAME_TOKEN_RE.search(n.lower())) or …
# must turn test_undeclared_advisory_named_check_still_gates and
# test_renaming_a_job_cannot_flip_gating_status RED.
class TestDeclaredAdvisoryRule(unittest.TestCase):
    """Exclusion requires an explicit registry declaration — nothing else."""

    def test_undeclared_advisory_named_check_still_gates(self):
        # THE CORE REGRESSION TEST. A name carrying the token, no declaration:
        # the predicate must say "gating", and a FAILURE must RED the verdict.
        self.assertFalse(g.is_declared_advisory(UNDECLARED_ADVISORY_NAME))
        self.assertFalse(g.is_advisory(UNDECLARED_ADVISORY_NAME))
        runs = [R(UNDECLARED_ADVISORY_NAME, conclusion="failure"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, out)
        self.assertIn("FAILED", out)
        self.assertIn(f"- ✗ {UNDECLARED_ADVISORY_NAME}: failure", out)
        # ...and the exclusion count must not claim it was excluded.
        self.assertIn("0 advisory check(s) excluded", out)

    def test_the_four_neutralised_gates_gate_by_default(self):
        # The four checks #3773 found neutralised. None is declared, so each one's
        # failure must RED the gate on its own.
        for name in (
            "determinism gate + foundation smoke (advisory)",
            "GUI tauri-driver browserName tripwire (advisory)",
            "no-sleep-gate (advisory)",
            "A11y — axe WCAG 2.1 AA (advisory)",
        ):
            with self.subTest(name=name):
                self.assertFalse(g.is_advisory(name))
                code, out = run(tiny_cfg(), [[R(name, conclusion="failure"), GREEN]])
                self.assertEqual(code, 1, out)
                self.assertIn(name, out)

    def test_undeclared_token_is_reported_loudly_not_silently(self):
        # The formerly-SILENT exclusion is now a visible note in the gate summary.
        runs = [R(UNDECLARED_ADVISORY_NAME), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0, out)
        self.assertIn(UNDECLARED_ADVISORY_NAME, out)
        self.assertIn("has no declaration", out)
        self.assertIn("it GATES", out)
        # A DECLARED advisory check is not reported as undeclared.
        self.assertEqual(g.undeclared_token_names([R("vale (prose, advisory)")]), [])

    def test_renaming_a_job_cannot_flip_gating_status(self):
        # RENAME INVARIANCE, both directions.
        #  (a) ADDING the token to a gating job's name does NOT make it non-gating —
        #      the pre-#3773 defect, where a one-word rename silently disarmed a gate.
        for renamed in (
            "clippy (advisory)",
            "clippy — advisory",
            "clippy (informational)",
            "coverage ratchet (advisory)",
        ):
            with self.subTest(rename=renamed):
                self.assertFalse(g.is_advisory(renamed))
                code, out = run(tiny_cfg(), [[R(renamed, conclusion="failure")]])
                self.assertEqual(code, 1, out)
        #  (b) RENAMING a DECLARED job away from its declared name makes it GATE again
        #      (fail-closed): the declaration is bound to one exact identity, so drift
        #      can only ever over-gate. C4 in check-advisory-registry.py REDs on it.
        self.assertTrue(g.is_advisory("vale (prose, advisory)"))
        for drifted in (
            "vale (prose style, advisory)",
            "vale prose (advisory)",
            "vale (prose, advisory) v2",
            "docs vale (prose, advisory)",
        ):
            with self.subTest(rename=drifted):
                self.assertFalse(g.is_advisory(drifted))
                code, out = run(tiny_cfg(), [[R(drifted, conclusion="failure")]])
                self.assertEqual(code, 1, out)

    def test_declaration_is_a_whole_name_match_never_a_substring(self):
        self.assertTrue(g.is_advisory("  vale (prose, advisory)  "))  # trimmed
        self.assertTrue(g.is_advisory("VALE (PROSE, ADVISORY)"))      # case-insensitive
        self.assertFalse(g.is_advisory("vale"))
        self.assertFalse(g.is_advisory("re-run vale (prose, advisory) shard"))

    def test_matrix_expression_in_a_declaration_matches_its_expansion(self):
        # A registry key is the YAML `name:`, so it may embed `${{ matrix.x }}`.
        self.assertTrue(g.is_advisory("GUI build + clippy (x64-linux, advisory)"))
        self.assertTrue(g.is_advisory("GUI build + clippy (win-x64, advisory)"))
        # The expression matches a NON-EMPTY run, and the literal frame must hold.
        self.assertFalse(g.is_advisory("GUI build + clippy (, advisory)"))
        self.assertFalse(g.is_advisory("GUI build + clippy (x64-linux)"))

    def test_declared_set_is_empty_by_default_so_an_unwired_gate_over_gates(self):
        # Fail-closed default: with no registry installed NOTHING is advisory.
        saved = g._DECLARED_ADVISORY
        try:
            g.set_declared_advisory(())
            self.assertFalse(g.is_advisory("vale (prose, advisory)"))
            code, out = run(tiny_cfg(), [[R("vale (prose, advisory)", conclusion="failure")]])
            self.assertEqual(code, 1, out)
        finally:
            g._DECLARED_ADVISORY = saved
        self.assertTrue(g.is_advisory("vale (prose, advisory)"))

    def test_failfast_and_resolver_inherit_the_declared_rule(self):
        # is_advisory is the SINGLE classifier: fail-fast must red on an undeclared
        # advisory-named failure while siblings are pending...
        red = R(UNDECLARED_ADVISORY_NAME, conclusion="failure")
        self.assertEqual([r["name"] for r in g.failfast_failures([red, PENDING])],
                         [UNDECLARED_ADVISORY_NAME])
        # ...and must NOT red on a declared one.
        declared_red = R("vale (prose, advisory)", conclusion="failure")
        self.assertEqual(g.failfast_failures([declared_red, PENDING]), [])
        # The resolver's run-level synthetic check reads the same predicate: an
        # UNDECLARED advisory-named job failure is a visible_required_failure, so no
        # synthetic verdict is minted and the job's own red is what gates.
        newest = W(311, 9, name="site-e2e-foundation", conclusion="failure")
        job = {
            "id": 4001,
            "name": UNDECLARED_ADVISORY_NAME,
            "status": "completed",
            "conclusion": "failure",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/311/job/1",
        }
        resolved, _ = g.resolve_newest_workflow_runs(
            [], [newest], "999", attempt_jobs={311: [job]}
        )
        with redirect_stdout(io.StringIO()):
            self.assertEqual(g.render_verdict(resolved), 1)


class TestAdvisoryRegistryLoading(unittest.TestCase):
    """[OPUS-5] #3773 — the registry loader is the gate's single source of truth."""

    def _reload(self):
        g.set_declared_advisory(DECLARED_ADVISORY)

    # A COMPLETE entry carries all five REGISTRY_REQUIRED_FIELDS — the identity pair
    # (`workflow`/`job_id`) included, since #3774's review.
    COMPLETE = {"owner_bead": "sq-a", "promotion_criteria": "x",
                "registered": "2026-01-01", "workflow": "ci.yml", "job_id": "j"}

    def test_entry_without_required_fields_declares_nothing(self):
        payload = {"jobs": {
            "complete (advisory)": dict(self.COMPLETE),
            "no-owner (advisory)": {k: v for k, v in self.COMPLETE.items()
                                    if k != "owner_bead"},
            "blank-owner (advisory)": {**self.COMPLETE, "owner_bead": ""},
            "not-an-object (advisory)": "oops",
        }}
        declared, warnings = g.parse_advisory_registry(payload)
        self.assertEqual(declared, ["complete (advisory)"])
        self.assertEqual(len(warnings), 3)
        try:
            g.set_declared_advisory(declared)
            self.assertTrue(g.is_advisory("complete (advisory)"))
            # Fail-closed per entry: an under-specified declaration buys nothing.
            self.assertFalse(g.is_advisory("no-owner (advisory)"))
            self.assertFalse(g.is_advisory("blank-owner (advisory)"))
            self.assertFalse(g.is_advisory("not-an-object (advisory)"))
        finally:
            self._reload()

    # ---------------------------------------------------------------------
    # [OPUS-5] #3774 cross-provider review (gpt-5.6-sol), finding 2(a).
    # The GATE required 3 fields while scripts/check-advisory-registry.py required 5,
    # and C4 `continue`d past an identity-less entry believing C2 had reported it. C2
    # only inspects jobs whose NAME carries an advisory/informational token, so an
    # identity-less entry keyed on a NON-token name was reported by NOBODY: it
    # neutralised a real gate while the checker printed `all clear (C2 + C3 + C4)`.
    # These tests pin the fix ON THE GATE — being flagged by the checker is not
    # enough, because the checker is a separate job and the gate is what authorises
    # the merge.
    # ---------------------------------------------------------------------

    def test_gate_refuses_an_entry_with_no_job_identity(self):
        # MUTATION TARGET: drop "workflow"/"job_id" from REGISTRY_REQUIRED_FIELDS.
        # Behaviour is asserted FIRST so the mutant REDs on the real exclusion
        # decision (`declared` / `is_advisory` / the verdict), not merely on the
        # membership of a constant.
        for dropped in ("workflow", "job_id"):
            with self.subTest(missing=dropped):
                key = f"no-{dropped} (advisory)"
                payload = {"jobs": {key: {k: v for k, v in self.COMPLETE.items()
                                          if k != dropped}}}
                declared, warnings = g.parse_advisory_registry(payload)
                # The entry declares NOTHING and says so out loud.
                self.assertEqual(declared, [])
                self.assertEqual(len(warnings), 1)
                self.assertIn(dropped, warnings[0])
                self.assertIn("still GATES", warnings[0])
                try:
                    g.set_declared_advisory(declared)
                    self.assertFalse(g.is_advisory(key))
                    # ...and a FAILURE of that check REDs the gate.
                    code, out = run(tiny_cfg(), [[R(key, conclusion="failure")]])
                    self.assertEqual(code, 1, out)
                finally:
                    self._reload()
        # A BLANK identity value is as absent as a missing key (fail-closed).
        for blanked in ("workflow", "job_id"):
            with self.subTest(blank=blanked):
                payload = {"jobs": {"blank (advisory)":
                                    {**self.COMPLETE, blanked: ""}}}
                declared, _ = g.parse_advisory_registry(payload)
                self.assertEqual(declared, [])
        # Only now the constant itself, as documentation of the mechanism.
        self.assertIn("workflow", g.REGISTRY_REQUIRED_FIELDS)
        self.assertIn("job_id", g.REGISTRY_REQUIRED_FIELDS)

    def test_gate_and_registry_checker_require_the_same_fields(self):
        # The two must not drift again: an entry the checker rejects must not buy a
        # runtime exclusion, and vice versa.
        checker = REPO_ROOT / "scripts" / "check-advisory-registry.py"
        spec = importlib.util.spec_from_file_location("_car", checker)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        self.assertEqual(set(mod.REQUIRED_FIELDS), set(g.REGISTRY_REQUIRED_FIELDS))

    def test_reviewers_three_field_clippy_entry_cannot_neutralise_the_clippy_gate(self):
        # THE REVIEWER'S EXACT REPRODUCTION, end to end. Injecting this entry into the
        # LIVE registry used to make `is_advisory(...) == True` (the real clippy gate
        # dropped from the gating set) while check-advisory-registry.py exited 0.
        key = "clippy (gate) + fmt (non-blocking)"
        live = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(
                encoding="utf-8")
        )
        baseline, _ = g.parse_advisory_registry(live)
        live["jobs"][key] = {"owner_bead": "x", "promotion_criteria": "y",
                            "registered": "2026-07-25"}
        declared, warnings = g.parse_advisory_registry(live)
        try:
            # (1) The 3-field entry declares NOTHING — the declared set is unchanged.
            self.assertNotIn(key, declared)
            self.assertEqual(declared, baseline)
            # (2) It warns, naming the missing identity pair.
            hits = [w for w in warnings if key in w]
            self.assertEqual(len(hits), 1, warnings)
            self.assertIn("workflow", hits[0])
            self.assertIn("job_id", hits[0])
            g.set_declared_advisory(declared)
            # (3) The clippy leg still GATES: a failure REDs the verdict, and the
            #     exclusion count does not claim it was excluded.
            self.assertFalse(g.is_advisory(key))
            code, out = run(tiny_cfg(), [[R(key, conclusion="failure"), GREEN]])
            self.assertEqual(code, 1, out)
            self.assertIn(f"- ✗ {key}: failure", out)
            self.assertIn("0 advisory check(s) excluded", out)
        finally:
            self._reload()

    # ---------------------------------------------------------------------
    # [OPUS-5] #3774 review finding 2(b) — a `${{ … }}` compiles to an unbounded
    # `.+`, so an expression-ONLY key (the idiomatic `name: ${{ matrix.label }}`)
    # compiled to `.+` and whole-name-matched EVERY check-run — `gate` itself
    # included — neutralising the entire run from one registry line. C4 could not see
    # it: the key DID equal the live YAML `name:`.
    # ---------------------------------------------------------------------

    ANCHORLESS_KEYS = (
        "${{ matrix.label }}",
        "${{matrix.label}}",
        "${{ matrix.os }}${{ matrix.label }}",
        "  ${{ matrix.label }}  ",
        "${{ matrix.a }} ${{ matrix.b }}",   # only whitespace between expressions
    )

    def test_an_anchorless_registry_key_is_refused_not_compiled(self):
        # MUTATION TARGET: delete the registry_key_has_literal_anchor guard (i.e.
        # restore the unbounded `.+` compilation).
        for key in self.ANCHORLESS_KEYS:
            with self.subTest(key=key):
                self.assertFalse(g.registry_key_has_literal_anchor(key))
                # The compiler REFUSES it outright — fail-closed, never `.+`.
                with self.assertRaises(g.AdvisoryRegistryError):
                    g._compile_declared_name(key)
                # ...and the loader skips it with a warning, declaring nothing.
                payload = {"jobs": {key: dict(self.COMPLETE)}}
                declared, warnings = g.parse_advisory_registry(payload)
                self.assertEqual(declared, [])
                self.assertEqual(len(warnings), 1)
                self.assertIn("literal anchor", warnings[0])
                self.assertIn("still GATES", warnings[0])

    def test_an_anchorless_key_would_otherwise_neutralise_the_gate_itself(self):
        # The BLAST RADIUS the guard prevents, stated as an assertion: were an
        # expression-only key installable, `.+` would fullmatch every real check-run
        # name on the commit — `gate` (the required context) included. This is what
        # the mutant does, so the mutant must flip these.
        for key in self.ANCHORLESS_KEYS:
            with self.subTest(key=key):
                payload = {"jobs": {key: dict(self.COMPLETE)}}
                declared, _ = g.parse_advisory_registry(payload)
                try:
                    g.set_declared_advisory(declared)
                    for victim in ("gate", "clippy", "test (ubuntu-latest)",
                                   "coverage ratchet", "ci-select"):
                        self.assertFalse(g.is_advisory(victim), victim)
                    # A real gating FAILURE therefore still REDs.
                    code, out = run(tiny_cfg(), [[R("clippy", conclusion="failure")]])
                    self.assertEqual(code, 1, out)
                finally:
                    self._reload()

    def test_a_framed_expression_key_is_still_accepted(self):
        # The guard must not break the LEGITIMATE shape: a literal frame around the
        # expression, which is what every shipped matrix declaration uses.
        framed = "GUI build + clippy (${{ matrix.label }}, advisory)"
        self.assertTrue(g.registry_key_has_literal_anchor(framed))
        payload = {"jobs": {framed: dict(self.COMPLETE)}}
        declared, warnings = g.parse_advisory_registry(payload)
        self.assertEqual(declared, [framed])
        self.assertEqual(warnings, [])
        try:
            g.set_declared_advisory(declared)
            self.assertTrue(g.is_advisory("GUI build + clippy (x64-linux, advisory)"))
            self.assertFalse(g.is_advisory("gate"))
        finally:
            self._reload()

    def test_every_live_registry_key_carries_a_literal_anchor(self):
        # Vacuity guard on the real file: the guard is only meaningful if the shipped
        # registry actually satisfies it (it does — all 27 keys are framed).
        raw = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(
                encoding="utf-8")
        )["jobs"]
        self.assertGreater(len(raw), 5)
        for key in raw:
            with self.subTest(key=key):
                self.assertTrue(g.registry_key_has_literal_anchor(key), key)

    def test_malformed_root_raises(self):
        for payload in ([], "x", {}, {"jobs": []}, {"jobs": "x"}):
            with self.subTest(payload=payload):
                with self.assertRaises(g.AdvisoryRegistryError):
                    g.parse_advisory_registry(payload)

    def test_missing_or_unparseable_file_raises(self):
        with self.assertRaises(g.AdvisoryRegistryError):
            g.load_advisory_registry(str(REPO_ROOT / "does-not-exist.json"))
        self._reload()
        bad = REPO_ROOT / "scripts" / "ci_summary_gate.py"  # valid path, not JSON
        with self.assertRaises(g.AdvisoryRegistryError):
            g.load_advisory_registry(str(bad))
        self._reload()

    def test_the_live_registry_loads_and_declares_only_real_entries(self):
        # The REAL repo registry must parse, declare every complete entry, and
        # (vacuity guard) actually contain some declarations.
        path = REPO_ROOT / ".github" / "advisory-registry.json"
        try:
            declared = g.load_advisory_registry(str(path))
            self.assertGreater(len(declared), 5)
            # Every declared key must be one of the registry's own job keys.
            raw = json.loads(path.read_text(encoding="utf-8"))["jobs"]
            self.assertTrue(set(declared) <= set(raw))
            # NEGATIVE: the four gates #3773 restored must NOT be declared advisory.
            for restored in (
                "site e2e determinism gate (no waitForTimeout calls)",
                "GUI hermetic guards (browserName tripwire + no-sleep-gate)",
                "site a11y ratchet — axe WCAG 2.1 AA (headless Chromium)",
            ):
                self.assertNotIn(restored, raw, restored)
        finally:
            self._reload()


class TestAdvisoryRegistryWiring(unittest.TestCase):
    """[OPUS-5] #3773 — ci-summary.yml must sparse-check-out the registry, or the
    gate exits 1 on every run. A workflow-inspection test, like the required-check
    anchor in test_ci_select_wiring.py."""

    def test_ci_summary_sparse_checkout_includes_the_registry(self):
        text = (REPO_ROOT / ".github" / "workflows" / "ci-summary.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("sparse-checkout:", text)
        block = text.split("sparse-checkout:", 1)[1].split("sparse-checkout-cone-mode", 1)[0]
        self.assertIn("scripts/ci_summary_gate.py", block)
        self.assertIn(g.ADVISORY_REGISTRY_PATH, block)


# [OPUS-5] PLATFORM-MANAGED advisory exclusion (the exact fail-closed allow-list).
# LIVE DEFECT: main gate run 30136978362 (2026-07-25T00:46Z) failed fast on
# "✗ Dependabot: failure" — the GitHub-managed Dependabot Updates job (run
# 30136987253) concluded `security_update_not_possible` for npm `brace-expansion`,
# an UPSTREAM condition with no in-repo remedy. Because the name is chosen by
# GitHub it cannot carry the "(advisory)" token, so the name-token rule could not
# reach it and it GATED. These tests pin all four halves of the fix:
#   (1) exactly "Dependabot" failing does NOT red the verdict;
#   (2) an unknown/new platform-ish name still REDs (fail-closed — no wildcards);
#   (3) the pre-existing advisory-name rule is untouched; and
#   (4) a REAL gating failure alongside a Dependabot failure still REDs.
class TestPlatformManagedAdvisoryRule(unittest.TestCase):
    DEPENDABOT = "Dependabot"

    def test_predicate_matches_only_the_exact_allow_listed_name(self):
        self.assertTrue(g.is_platform_managed_advisory(self.DEPENDABOT))
        # Case-insensitive + surrounding-whitespace tolerant whole-name match.
        self.assertTrue(g.is_platform_managed_advisory("  dependabot "))
        self.assertTrue(g.is_platform_managed_advisory("DEPENDABOT"))
        # (2) FAIL-CLOSED: no substring/prefix/suffix/wildcard reach. Every one of
        # these is an unknown name and must keep gating.
        for unknown in (
            "Dependabot Updates",
            "Dependabot alerts",
            "dependabot-security-check",
            "verify Dependabot lockfile",
            "Dependabot / npm_and_yarn",
            "supply-chain gates (deny + vet + SBOM + VEX + OpenSSF + js-sbom)",
            "build + test",
            "gate",
        ):
            self.assertFalse(g.is_platform_managed_advisory(unknown), unknown)
            self.assertFalse(g.is_advisory(unknown), unknown)
        # (3) The name-token rule is a SEPARATE concern and is not absorbed by the
        # allow-list — an advisory-named leg is not "platform managed".
        self.assertFalse(g.is_platform_managed_advisory("vale (prose, advisory)"))
        self.assertTrue(g.is_advisory("vale (prose, advisory)"))
        # ...and the union predicate every consumer reads sees both rules.
        self.assertTrue(g.is_advisory(self.DEPENDABOT))

    def test_dependabot_failure_does_not_red_the_verdict(self):
        # (1) The live defect, end to end through the real verdict path.
        runs = [R(self.DEPENDABOT, conclusion="failure"), GREEN, GREEN2]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertIn("1 advisory check(s) excluded", out)

    def test_unknown_platform_managed_name_still_reds(self):
        # (2) Fail-closed at the VERDICT level, not just the predicate: a renamed or
        # newly-added platform job is a gating leg until this allow-list is edited.
        for unknown in ("Dependabot Updates", "Dependabot alerts", "dependabot-security"):
            with self.subTest(name=unknown):
                code, out = run(tiny_cfg(), [[R(unknown, conclusion="failure"), GREEN]])
                self.assertEqual(code, 1, out)
                self.assertIn(unknown, out)

    def test_existing_advisory_name_rule_still_excludes(self):
        # (3) Regression guard for sq-wjth alongside the new rule, including the
        # plural "advisories" that must keep GATING.
        runs = [R("vale (prose, advisory)", conclusion="failure"),
                R("unsafe report (cargo-geiger, informational)", conclusion="failure"),
                R(self.DEPENDABOT, conclusion="failure"),
                GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0, out)
        self.assertIn("3 advisory check(s) excluded", out)
        code, _ = run(tiny_cfg(), [[
            R("cargo-deny (advisories, bans, licenses, sources)", conclusion="failure"),
            R(self.DEPENDABOT, conclusion="failure"),
        ]])
        self.assertEqual(code, 1)

    def test_real_failure_alongside_dependabot_still_reds(self):
        # (4) The exclusion must not become a blanket amnesty: a genuine gating
        # failure sharing the sibling set still REDs, and the Dependabot leg is not
        # what the verdict blames.
        runs = [R(self.DEPENDABOT, conclusion="failure"), RED, GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, out)
        self.assertIn("clippy", out)
        failing_lines = [ln for ln in out.splitlines() if ln.startswith("- ✗ ")]
        self.assertTrue(failing_lines)
        self.assertNotIn(f"- ✗ {self.DEPENDABOT}: failure", failing_lines)

    def test_dependabot_failure_does_not_fail_fast(self):
        # Fail-fast reuses is_advisory, so the exclusion must hold there too: a
        # Dependabot red while siblings are still running must not short-circuit.
        dependabot = R(self.DEPENDABOT, conclusion="failure")
        self.assertEqual(g.failfast_failures([dependabot, PENDING]), [])
        code, out = run(tiny_cfg(), [[dependabot, PENDING], [dependabot, GREEN]])
        self.assertEqual(code, 0, out)
        self.assertNotIn("fail-fast", out)

    def test_dependabot_only_workflow_failure_gets_no_synthetic_gating_verdict(self):
        # The resolver's run-level evidence path also reads is_advisory: GitHub's
        # Dependabot workflow RUN concludes failure, so without the exclusion a
        # synthetic "workflow-run verdict (…)" would red the gate even though the
        # only failing job is the excluded one.
        newest = W(103, 7, name="Dependabot Updates", conclusion="failure")
        dependabot_job = {
            "id": 2001,
            "name": self.DEPENDABOT,
            "status": "completed",
            "conclusion": "failure",
            "started_at": "2026-07-21T14:05:00Z",
            "html_url": "https://github.test/o/r/actions/runs/103/job/2",
        }
        resolved, _ = g.resolve_newest_workflow_runs(
            [], [newest], "999", attempt_jobs={103: [dependabot_job]}
        )
        self.assertNotIn(
            "workflow-run verdict (id:7)", [r.get("name") for r in resolved]
        )
        code, out = run(tiny_cfg(), [resolved])
        self.assertEqual(code, 0, out)


# [FABLE-5] sq-fmx4u.3: change-based test-selection semantics (design §5.3).
# The three load-bearing safety invariants, exactly as the bead states them:
#   (1) skipped-not-affected + select SUCCESS      => gate SUCCESS (no hang, no RED)
#   (2) a job that FAILED                          => gate FAILURE (selection never
#       masks a real failure, with or without skips present)
#   (3) select present-but-not-success             => gate FAILURE even if every
#       other sibling is green/skipped (a skip is only trustworthy under a
#       successful selection — fail-closed, design §4.3)
# plus the transition guarantee: select ABSENT (a pre-selection sibling set)
# preserves the previous semantics byte-for-byte (skipped is non-failing).
SELECT_NAME = "select / select (change-based test selection)"
SEL_OK = R(SELECT_NAME)


class TestSelectionSemantics(unittest.TestCase):
    def test_skipped_not_affected_with_select_success_passes(self):
        # Invariant 1: an enforced-selection run — wide lanes skipped, select green.
        runs = [SEL_OK,
                R("W3C/OGC GeoSPARQL conformance (ratchet)", conclusion="skipped"),
                R("opt-in sparq-rsp (rsp)", conclusion="skipped"),
                GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out)
        # The bead's transparency line: N of M ran, K skipped, select healthy.
        self.assertIn("2 skipped", out)
        self.assertIn("selection pre-job succeeded", out)

    def test_orchestration_only_pr_gate_passes_with_codeql_and_engine_skipped(self):
        # [OPUS-4.8] path-aware CI: the concrete orchestration-only PR (#3416 class —
        # routing.toml + triage.py) sibling set. CodeQL is `skipped` (its new
        # rust_changed guard), every engine lane + opt-in leg is `skipped` (empty
        # affected closure), the cheap gates run green, and `select` concludes
        # SUCCESS (mode=selected attributes the skips). The gate must render PASS —
        # a fast green for an orchestration PR with the whole Rust matrix skipped.
        runs = [
            SEL_OK,
            R("CodeQL analysis (rust)", conclusion="skipped"),
            R("test (load-aware shard bulk 1/3)", conclusion="skipped"),
            R("opt-in sparq-engine (paths)", conclusion="skipped"),
            R("bench (deterministic ratchet)", conclusion="skipped"),
            R("differential-smoke", conclusion="skipped"),
            R("docs-quality quick-gates", conclusion="success"),
        ]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out)
        self.assertIn("selection pre-job succeeded", out)
        # CodeQL's skip is attributed (select green), never a false RED.
        self.assertNotIn("FAILED", out)

    def test_real_failure_still_fails_under_selection(self):
        # Invariant 2: selection must never mask a genuine failure.
        runs = [SEL_OK, RED, R("docs", conclusion="skipped")]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)
        self.assertIn("FAILED", out)
        self.assertIn("clippy", out)

    def test_select_failure_reds_even_when_all_siblings_green(self):
        # Invariant 3: an unobservable selection blocks the merge outright.
        runs = [R(SELECT_NAME, conclusion="failure"), GREEN, GREEN2]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)
        self.assertIn("selection pre-job", out)

    def test_select_skipped_or_neutral_is_not_success(self):
        # "did not succeed" is anything but success — a skipped/neutral select
        # would otherwise sneak through the generic skipped/neutral tolerance.
        for concl in ("skipped", "neutral", "cancelled", None):
            runs = [R(SELECT_NAME, conclusion=concl),
                    R("x", conclusion="skipped"), GREEN]
            code, out = run(tiny_cfg(), [runs])
            self.assertEqual(code, 1, f"select conclusion={concl!r} must RED the gate")
            self.assertIn("selection pre-job", out)

    def test_select_absent_preserves_legacy_skip_semantics(self):
        # Transition guarantee: no selection check-run on the commit (old sibling
        # sets / other repos' shapes) => skipped stays non-failing, as before.
        runs = [R("docs", conclusion="skipped"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0)
        self.assertNotIn("selection pre-job", out)

    def test_multiple_select_runs_all_must_succeed(self):
        # ci.yml AND feature-matrix.yml each call the reusable select job — the
        # gate sees two same-named runs; ONE failing must RED the gate.
        runs = [SEL_OK, R(SELECT_NAME, conclusion="failure"), GREEN]
        code, _ = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)

    def test_cancelled_select_forgiven_by_same_name_success(self):
        # [OPUS-4.8] sq-fmx4u.3 hardening: the 2026-07-17 fleet jam. Under the
        # draft-tier + review-pipeline label churn a head SHA accretes many
        # concurrency-cancel rounds; a doomed select INSTANCE (cancelled) can
        # out-timestamp the winning run's already-concluded select SUCCESS, so
        # forgive_superseded's strictly-later rule leaves a residual cancelled
        # select. A same-normalized-name SUCCESS on the SHA proves the selection
        # was computed soundly (select is a deterministic pure pre-job over the
        # diff), so the gate must NOT RED over a provably-sound selection.
        runs = [R(SELECT_NAME, conclusion="success", started="2026-01-01T00:00:10Z", rid=2),
                R(SELECT_NAME, conclusion="cancelled", started="2026-01-01T00:00:20Z", rid=3),
                R("W3C conformance", conclusion="skipped"),
                GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0, "a cancelled select superseded by a same-name success must not RED")
        self.assertIn("PASSED", out)

    def test_cancelled_select_with_no_success_still_reds(self):
        # The forgiveness is narrow: a cancelled select with NO successful
        # same-name sibling is still an unobservable selection and REDs.
        runs = [R(SELECT_NAME, conclusion="cancelled"),
                R("x", conclusion="skipped"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)
        self.assertIn("selection pre-job", out)

    def test_failure_select_reds_even_with_a_success_sibling(self):
        # Only cancelled/stale race losers are forgiven — a genuine `failure`
        # select still REDs regardless of any success sibling (a real selection
        # failure is never masked by a concurrency winner).
        runs = [R(SELECT_NAME, conclusion="success"),
                R(SELECT_NAME, conclusion="failure"),
                R("x", conclusion="skipped"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1)
        self.assertIn("selection pre-job", out)

    def test_cancelled_draft_select_not_forgiven_by_full_success(self):
        # [OPUS-4.8] cross-provider review of PR #3417, finding 1: the any-success
        # rule is SAME-TIER only. A full-tier select success must NOT erase a
        # later cancelled DRAFT-tier instance — that instance must stay visible
        # (fail-closed: the draft_selects_unsuperseded hold accounts for draft
        # instances per-instance; erasing one by a cross-tier name collision
        # would under-count the hold). Strictly-later cross-tier supersession is
        # a different, still-supported path (the un-draft flow).
        runs = [R(SELECT_NAME, conclusion="success", started="2026-01-01T00:00:10Z", rid=2),
                R(SELECT_NAME + ", draft-tier", conclusion="cancelled",
                  started="2026-01-01T00:00:20Z", rid=3),
                R("x", conclusion="skipped"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, "a cross-tier success must not forgive a cancelled draft select")
        self.assertIn("selection pre-job", out)

    def test_cancelled_full_select_not_forgiven_by_draft_success(self):
        # [OPUS-4.8] finding 1, other direction: a DRAFT-tier select success must
        # never stand in for a cancelled FULL-tier selection (draft-assembled
        # evidence never satisfies a full-tier verdict, design #2537).
        runs = [R(SELECT_NAME + ", draft-tier", conclusion="success",
                  started="2026-01-01T00:00:10Z", rid=2),
                R(SELECT_NAME, conclusion="cancelled",
                  started="2026-01-01T00:00:20Z", rid=3),
                R("x", conclusion="skipped"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, "a draft-tier success must not forgive a cancelled full-tier select")
        self.assertIn("selection pre-job", out)

    def test_cancelled_compound_select_keeps_strictly_later_rule(self):
        # [OPUS-4.8] cross-provider review of PR #3417, finding 2: the compound
        # fv-select + fv-manifest job CONTAINS the selection phrase but carries
        # independent gating evidence (the proof-inventory manifest), so it is
        # NOT a pure select and must keep the strictly-later supersession rule:
        # an EARLIER success never forgives its later cancellation.
        compound = "fv-select (change-based test selection) + fv-manifest (proof inventory)"
        runs = [R(compound, conclusion="success", started="2026-01-01T00:00:10Z", rid=2),
                R(compound, conclusion="cancelled", started="2026-01-01T00:00:20Z", rid=3),
                R("x", conclusion="skipped"), GREEN]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, "an earlier success must not forgive a later cancelled compound select+evidence job")
        self.assertIn("selection pre-job", out)

    def test_pure_select_detection_contract(self):
        # is_pure_select gates ONLY the forgiveness widening: pure reusable
        # selector names (tier-marked or not) qualify; the compound
        # select+manifest job and non-select legs do not.
        self.assertTrue(g.is_pure_select(SELECT_NAME))
        self.assertTrue(g.is_pure_select(SELECT_NAME + ", draft-tier"))
        self.assertFalse(g.is_pure_select(
            "fv-select (change-based test selection) + fv-manifest (proof inventory)"))
        self.assertFalse(g.is_pure_select("build + test"))
        self.assertFalse(g.is_pure_select("gate"))

    def test_select_name_detection_contract(self):
        # The name-detection contract with .github/workflows/ci-select.yml (also
        # pinned cross-file by scripts/tests/test_ci_select_wiring.py).
        self.assertTrue(g.is_select(SELECT_NAME))
        self.assertFalse(g.is_select("build + test"))
        self.assertFalse(g.is_select("gate"))
        # And the select job must never be advisory-excluded.
        self.assertFalse(g.is_advisory(SELECT_NAME))


class TestRateLimitFailClosed(unittest.TestCase):
    """[GPT-6-ASTRA] Explicit gh refusals stop every API path, without network.

    Counts are gh api invocations, not hidden HTTP requests within --paginate.
    Fixtures are jq-projected stdout plus realistic gh error diagnostics; no
    response headers or numerical remaining quota are claimed.
    """

    @staticmethod
    def _proc(rows=(), error="", stdout=None):
        import subprocess
        return subprocess.CompletedProcess(
            ["gh", "api"], 1 if error else 0,
            stdout="\n".join(json.dumps(r) for r in rows) if stdout is None else stdout,
            stderr=error,
        )

    @classmethod
    def _limited(cls):
        return cls._proc(error="gh: API rate limit exceeded for 192.0.2.1. (HTTP 403)\n")

    @staticmethod
    def _drive(responses, fetch=None, depth=None, tier_ctx=None):
        from unittest.mock import patch
        # Repeat the final response so a weakened stop is an assertion failure,
        # never mock exhaustion that aborts the remaining test population.
        seen = []
        def gh(argv, **_kw):
            seen.append(argv)
            return responses[min(len(seen) - 1, len(responses) - 1)]
        out = io.StringIO()
        with patch("subprocess.run", side_effect=gh), redirect_stdout(out):
            try:
                code = g.run_gate(
                    tiny_cfg(reporter_grace_polls=4),
                    fetch or g.make_fetch_check_runs("o/r", "head"),
                    depth or (lambda: 0), sleep_fn=lambda _s: None, tier_ctx=tier_ctx,
                )
            except RuntimeError as exc:
                # A leaked fatal exception is also a contract failure, asserted
                # by callers; it must not truncate a calibrated control run.
                code = None
                print(f"unexpected escaping runtime error: {exc}")
        return code, out.getvalue(), seen

    def test_explicit_primary_secondary_429_and_json_error_signals(self):
        from unittest.mock import patch
        cases = (
            self._limited(),
            self._proc(error="gh: You have exceeded a secondary rate limit. (HTTP 403)\n"),
            self._proc(error="gh: Too Many Requests (HTTP 429)\n"),
            self._proc(error="HTTP 429: request refused (https://api.github.com/repos/o/r)\n"),
            self._proc(error="gh: Too Many Requests\n"),
            self._proc(error="gh: Forbidden (HTTP 403)\n",
                       stdout=json.dumps({"message": "API rate limit exceeded for user ID 7."})),
        )
        for response in cases:
            with self.subTest(stderr=response.stderr):
                with patch("subprocess.run", return_value=response) as api:
                    with self.assertRaises(RuntimeError) as raised:
                        g._gh_json_lines(["repos/o/r/commits/head/check-runs"])
                self.assertIsInstance(raised.exception, g.RateLimitError)
                self.assertEqual(api.call_count, 1)
                code, out, calls = self._drive(
                    [self._proc([_grp("222"), GREEN])] * 7 + [response])
                self.assertEqual(code, 1, out)
                self.assertEqual(len(calls), 8)
                self.assertNotIn("cap fetch recovery", out)
                self.assertNotIn("PASSED", out)

    def test_headerless_generic_failures_and_success_payload_do_not_prove_exhaustion(self):
        from unittest.mock import patch
        for error in ("gh: Bad credentials (HTTP 401)",
                      "gh: Resource not accessible by integration (HTTP 403)",
                      "gh: Internal Server Error (HTTP 500)"):
            with self.subTest(error=error), patch("subprocess.run", return_value=self._proc(error=error)):
                with self.assertRaises(RuntimeError) as raised:
                    g._gh_json_lines(["repos/o/r"])
                self.assertIsInstance(raised.exception, g.FetchError)
                self.assertNotIsInstance(raised.exception, g.RateLimitError)
        row = R("API rate limit exceeded")  # successful data is not a refusal
        with patch("subprocess.run", return_value=self._proc([row])):
            self.assertEqual(g._gh_json_lines(["repos/o/r"]), [row])

    def test_known_limit_before_at_and_after_cap_stops_at_that_invocation(self):
        waiting = [_grp("222"), GREEN]
        current = [*waiting, _rep("222")]
        for fault_at in (1, 8, 9):
            with self.subTest(fault_at=fault_at):
                responses = [self._proc(waiting)] * (fault_at - 1)
                code, out, calls = self._drive(responses + [self._limited(), self._proc(current)])
                self.assertEqual(code, 1, out)
                self.assertEqual(len(calls), fault_at)
                self.assertIn("no further polling or recovery", out)
                self.assertNotIn("cap fetch recovery", out)
                self.assertNotIn("PASSED", out)

    def test_generic_transient_http_error_still_recovers_at_cap(self):
        waiting = [_grp("222"), GREEN]
        current = [*waiting, _rep("222")]
        responses = [self._proc(waiting)] * 7
        responses += [self._proc(error="gh: Bad Gateway (HTTP 502)"), self._proc(current)]
        code, out, calls = self._drive(responses)
        self.assertEqual(code, 0, out)
        self.assertEqual(len(calls), 10)
        self.assertIn("cap fetch recovery", out)

    def test_resolver_list_self_and_jobs_refusals_stop_later_calls_in_same_poll(self):
        own = W(999, 99, name="ci-summary", status="in_progress", conclusion=None)
        work = [W(101, 7), W(102, 8)]
        prefix = [self._proc([]), self._proc([own, *work]), self._proc([own])]
        expected = ["repos/o/r/commits/head/check-runs",
                    "repos/o/r/actions/runs?head_sha=head&per_page=100",
                    "repos/o/r/actions/runs/999",
                    "repos/o/r/actions/runs/101/jobs?filter=latest&per_page=100"]
        for fault_at in (1, 2, 3, 4):
            with self.subTest(fault_at=fault_at):
                code, out, calls = self._drive(
                    prefix[:fault_at - 1] + [self._limited()],
                    fetch=g.make_fetch_runs("o/r", "head", "999"),
                )
                self.assertEqual(code, 1, out)
                self.assertEqual([argv[2] for argv in calls], expected[:fault_at])
                self.assertIn("no further polling or recovery", out)

    def test_redispatch_refusal_prevents_second_post_or_job_fetch(self):
        own = W(999, 99, name="ci-summary", status="in_progress", conclusion=None)
        cancelled = [W(101, 7, conclusion="cancelled"), W(102, 8, conclusion="cancelled")]
        responses = [self._proc([]), self._proc([own, *cancelled]), self._proc([own]), self._limited()]
        code, out, calls = self._drive(responses, fetch=g.make_fetch_runs("o/r", "head", "999"))
        self.assertEqual(code, 1, out)
        self.assertEqual(len(calls), 4)
        self.assertEqual(calls[-1], ["gh", "api", "--method", "POST", "repos/o/r/actions/runs/101/rerun"])
        self.assertIn("no further polling or recovery", out)

    def test_draft_finalization_and_full_hold_do_not_retry_known_limit(self):
        cases = (("draft", [GREEN], 2), ("full", [R(SELECT_DRAFT)], 4))
        for tier, rows, expected_polls in cases:
            with self.subTest(tier=tier):
                fetch = scripted([rows])
                ctx = g.TierContext(run_tier=tier, event_name="pull_request",
                                    fetch_pr_draft=g.make_fetch_pr_draft("o/r", "7"))
                code, out, calls = self._drive([self._limited()], fetch=fetch, tier_ctx=ctx)
                self.assertEqual(code, 1, out)
                self.assertEqual(calls, [["gh", "api", "repos/o/r/pulls/7", "--jq", ".draft"]])
                self.assertEqual(fetch.state["calls"], expected_polls)
                self.assertIn("no further polling or recovery", out)
                self.assertNotIn("PASSED", out)

    def test_queue_rate_limit_is_not_swallowed_into_liveness_extension(self):
        fetch = scripted([[GREEN, IN_PROGRESS]])
        code, out, calls = self._drive([self._limited()], fetch=fetch,
                                      depth=g.make_fetch_queue_depth("o/r"))
        self.assertEqual(code, 1, out)
        self.assertEqual(fetch.state["calls"], 4)
        self.assertEqual(len(calls), 1)
        self.assertIn("no further polling or recovery", out)
        self.assertNotIn("Extending the wait", out)


# [SONNET-4.6] Robustness-hardening tests (sq-90cv4 follow-up: Copilot gaps).
# Covers:
#   (a) _gh_json_lines converts subprocess raises (FileNotFoundError, TimeoutExpired)
#       → FetchError → routed into the existing bounded skip path, not a raw crash.
#   (b) fetch_queue_depth raising in run_gate → depth treated as None (unknown) →
#       NO saturation extension granted on depth alone (conservative branch).
# Mutation check: removing either try/except causes the test to go RED (the
# underlying exception propagates instead of being caught/re-raised as FetchError).
class TestSubprocessRobustness(unittest.TestCase):
    """[SONNET-4.6] subprocess raises in _gh_json_lines / fetch_queue_depth must be
    converted to graceful-degradation paths, never raw crashes."""

    def test_gh_json_lines_file_not_found_raises_fetch_error(self):
        """FileNotFoundError (gh not on PATH) must surface as FetchError so the
        caller's FetchError handler (bounded retry / skip-this-poll) applies."""
        from unittest.mock import patch
        with patch("subprocess.run", side_effect=FileNotFoundError("gh: not found")):
            with self.assertRaises(g.FetchError) as ctx:
                g._gh_json_lines(["repos/x/y"])
        self.assertIn("subprocess raised", str(ctx.exception))

    def test_gh_json_lines_timeout_raises_fetch_error(self):
        """TimeoutExpired must also surface as FetchError (same bounded-retry path)."""
        from unittest.mock import patch
        import subprocess as _sp
        with patch("subprocess.run", side_effect=_sp.TimeoutExpired("gh", 30)):
            with self.assertRaises(g.FetchError):
                g._gh_json_lines(["repos/x/y"])

    def test_subprocess_raise_routed_into_skip_tolerance(self):
        """A FileNotFoundError inside _gh_json_lines → FetchError → treated as a
        skipped poll (not a gate crash).  Two such errors then a green poll must pass
        (within the 3-failure tolerance of tiny_cfg)."""
        # Simulate _gh_json_lines raising FetchError (as it does after the fix for
        # FileNotFoundError) for the first two calls, then returning [GREEN].
        call_count = {"n": 0}

        def fake_fetch():
            n = call_count["n"]
            call_count["n"] += 1
            if n < 2:
                raise g.FetchError("subprocess raised: gh: not found")
            return [dict(GREEN)]

        out = io.StringIO()

        def depth_fn():
            return 0

        with redirect_stdout(out):
            code = g.run_gate(tiny_cfg(), fake_fetch, depth_fn, sleep_fn=lambda s: None)
        output = out.getvalue()
        self.assertEqual(code, 0)
        self.assertIn("skipping this poll", output)
        self.assertIn("PASSED", output)

    def test_fetch_queue_depth_raises_treated_as_unknown_no_extension(self):
        """When fetch_queue_depth() RAISES (subprocess spawn error inside the closure),
        run_gate must catch it, treat depth as None (unknown), and NOT grant a
        saturation extension — unknown depth with no progress is a genuine hang → RED.
        Mutation check: remove the try/except in run_gate → RuntimeError propagates
        instead of being caught → this test goes RED."""
        out = io.StringIO()
        # [OPUS-5] #3783: the awaited sibling must be QUEUED, not `in_progress` —
        # an executing sibling is positive liveness evidence and now VETOES the hang
        # verdict (see TestLivenessVeto). The property under test here is the
        # try/except around fetch_queue_depth, not the hang shape.
        fetch = scripted([[GREEN, PENDING]])

        def raising_depth():
            raise RuntimeError("subprocess.run raised: gh not found")

        with redirect_stdout(out):
            code = g.run_gate(tiny_cfg(), fetch, raising_depth, sleep_fn=lambda s: None)
        output = out.getvalue()
        self.assertEqual(code, 1, "unknown-depth + no-progress must be a genuine hang RED")
        self.assertIn("genuine hang", output)
        self.assertNotIn("Extending the wait", output)

    def test_fetch_queue_depth_raises_progress_still_extends(self):
        """Unknown depth (depth_fn raises) does NOT block the progress-signal path:
        if completions are landing the gate still extends (progress alone suffices)."""
        # Six polls with decreasing pending; completions rising → progress=True.
        # depth_fn always raises (unknown). Verify extension activates then passes.
        polls = [
            [R("a"), R("b"), PENDING, R("d", status="queued", conclusion=None),
             R("e", status="queued", conclusion=None), R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d", status="queued", conclusion=None),
             R("e", status="queued", conclusion=None), R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d"),
             R("e", status="queued", conclusion=None), R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d"), R("e"),
             R("f", status="queued", conclusion=None)],
            [R("a"), R("b"), R("coverage"), R("d"), R("e"), R("f")],
        ]
        out = io.StringIO()
        fetch = scripted(polls)

        def raising_depth():
            raise RuntimeError("depth API down")

        with redirect_stdout(out):
            code = g.run_gate(tiny_cfg(), fetch, raising_depth, sleep_fn=lambda s: None)
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out.getvalue())


SELECT_FULL = "select / select (change-based test selection)"
SELECT_DRAFT = "select / select (change-based test selection, draft-tier)"


def draft_ctx(fetch_pr_draft, run_tier="draft", event="pull_request", retries=3):
    return g.TierContext(run_tier=run_tier, event_name=event,
                         fetch_pr_draft=fetch_pr_draft, draft_check_retries=retries)


def counting(value):
    """fetch_pr_draft stub returning `value` (an Exception instance raises)."""
    state = {"calls": 0}

    def fetch():
        state["calls"] += 1
        if isinstance(value, Exception):
            raise value
        return value

    fetch.state = state
    return fetch


class TestDraftTierIntegrity(unittest.TestCase):
    """[FABLE-5] Draft-tier CI: the invariant that a draft-tier gate result can
    NEVER admit a PR to the merge queue — conclusion-time draft re-check,
    stale-draft-tier leg-set belt, and superseded-cancellation forgiveness."""

    # ---- conclusion-time draft re-check ------------------------------------
    def test_draft_tier_success_requires_still_draft(self):
        fetch = counting(True)
        code, out = run(tiny_cfg(), [[R(SELECT_DRAFT), GREEN]],
                        tier_ctx=draft_ctx(fetch))
        self.assertEqual(code, 0)
        self.assertIn("DRAFT-TIER verdict", out)
        self.assertEqual(fetch.state["calls"], 1)

    def test_draft_tier_stale_on_ready_pr_fails(self):
        """The core invariant: an all-green draft-tier run whose PR was un-drafted
        before conclusion must RED with the supersession message."""
        code, out = run(tiny_cfg(), [[R(SELECT_DRAFT), GREEN]],
                        tier_ctx=draft_ctx(counting(False)))
        self.assertEqual(code, 1)
        self.assertIn("stale draft-tier run, full run pending", out)

    def test_draft_tier_unverifiable_state_fails_closed_bounded(self):
        fetch = counting(g.FetchError("api down"))
        code, out = run(tiny_cfg(), [[GREEN]], tier_ctx=draft_ctx(fetch, retries=3))
        self.assertEqual(code, 1)
        self.assertIn("could not confirm", out)
        self.assertEqual(fetch.state["calls"], 3, "retries must be bounded")

    def test_draft_tier_no_fetcher_fails_closed(self):
        code, out = run(tiny_cfg(), [[GREEN]], tier_ctx=draft_ctx(None))
        self.assertEqual(code, 1)

    def test_draft_tier_red_legs_skip_the_api_check(self):
        """A failing verdict REDs regardless; the draft re-check (an API call)
        must not even run — a RED can never be latched by the queue."""
        fetch = counting(True)
        code, out = run(tiny_cfg(), [[R(SELECT_DRAFT), RED]],
                        tier_ctx=draft_ctx(fetch))
        self.assertEqual(code, 1)
        self.assertEqual(fetch.state["calls"], 0)

    def test_draft_tier_empty_set_still_rechecks(self):
        """The stable-empty pass path must apply the same conclusion-time
        re-check — no bypass via an empty sibling set."""
        code, out = run(tiny_cfg(), [[]], tier_ctx=draft_ctx(counting(False)))
        self.assertEqual(code, 1)
        self.assertIn("stale draft-tier run", out)

    def test_full_tier_run_never_calls_the_draft_api(self):
        fetch = counting(True)
        code, _ = run(tiny_cfg(), [[GREEN]],
                      tier_ctx=draft_ctx(fetch, run_tier="full"))
        self.assertEqual(code, 0)
        self.assertEqual(fetch.state["calls"], 0)

    # ---- stale draft-tier leg-set belt (full-tier pull_request runs) --------
    def test_full_tier_holds_then_reds_on_unsuperseded_draft_select(self):
        """A full-tier pull_request gate over a draft-tier-assembled leg set must
        WAIT (the ready_for_review re-run is expected) and must RED — never
        conclude success over draft-tier legs.

        [OPUS-5] #3781: with the PR STILL A DRAFT the hold is unsatisfiable, so the
        refusal now arrives via the fast path instead of at budget exhaustion. The
        verdict (exit 1) and the hold itself are unchanged; the budget-exhaustion
        belt is pinned by TestUnsatisfiableHoldFastFail
        ::test_unreadable_draft_state_falls_back_to_the_budget_belt."""
        polls = [[R(SELECT_DRAFT, started="2026-07-17T10:00:00Z", rid=1), GREEN]]
        code, out = run(tiny_cfg(), polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 1)
        self.assertIn("awaiting the full-tier re-run", out)
        self.assertIn("UNSATISFIABLE draft-tier hold", out)

    def test_full_tier_passes_once_full_select_supersedes(self):
        draft_sel = R(SELECT_DRAFT, started="2026-07-17T10:00:00Z", rid=1)
        full_sel = R(SELECT_FULL, started="2026-07-17T10:05:00Z", rid=2)
        polls = [
            [draft_sel, GREEN],                       # full re-run not registered yet
            [draft_sel, full_sel, GREEN],             # it lands; set goes stable
            [draft_sel, full_sel, GREEN],
            [draft_sel, full_sel, GREEN],
        ]
        code, out = run(tiny_cfg(), polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 0)
        self.assertIn("PASSED", out)

    def test_full_tier_requires_a_successor_per_draft_select_instance(self):
        """Cross-workflow collision: ci.yml, bench.yml, feature-matrix.yml and
        fuzz.yml all expose the IDENTICAL select check-run name, so a draft head
        carries FOUR draft-marked select instances. The hold must release only
        when EVERY instance has its own later full-tier successor — the first
        workflow's full-tier select must never release the other three."""
        drafts = [R(SELECT_DRAFT, started=f"2026-07-17T10:00:0{i}Z", rid=i)
                  for i in range(1, 5)]
        fulls = [R(SELECT_FULL, started=f"2026-07-17T10:05:0{i}Z", rid=10 + i)
                 for i in range(1, 5)]
        polls = [
            drafts + [GREEN],              # no full re-run registered yet: hold
            drafts + fulls[:1] + [GREEN],  # 1 of 4 registered: STILL hold
            drafts + fulls[:3] + [GREEN],  # 3 of 4 registered: STILL hold
            drafts + fulls + [GREEN],      # all four registered: settle + pass
            drafts + fulls + [GREEN],
            drafts + fulls + [GREEN],
        ]
        code, out = run(tiny_cfg(), polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        # The per-instance hold must have been observable while 1..3 full-tier
        # selects were registered (the collision would have released at 1).
        self.assertIn("awaiting the full-tier re-run", out)

    def test_first_full_select_must_not_release_all_draft_instances(self):
        """REGRESSION (critic finding 2): with four draft-marked selects and only
        ONE later full-tier select ever registering, the gate must hold and then
        RED — never conclude success over the three workflows whose full-tier runs
        never registered any check-runs. ([OPUS-5] #3781: the PR is still a draft, so
        the RED now arrives on the unsatisfiable-hold fast path; the COUNT — the
        load-bearing part of this regression — is asserted either way.)"""
        drafts = [R(SELECT_DRAFT, started=f"2026-07-17T10:00:0{i}Z", rid=i)
                  for i in range(1, 5)]
        one_full = R(SELECT_FULL, started="2026-07-17T10:05:00Z", rid=99)
        polls = [drafts + [one_full, GREEN]]
        code, out = run(tiny_cfg(), polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 1)
        self.assertIn("awaiting the full-tier re-run", out)
        self.assertIn("3 draft-marked select instance(s)", out)

    def test_full_tier_non_pr_event_ignores_draft_selects(self):
        """merge_group/push gates never see the belt (fresh ref; no PR payload)."""
        ctx = g.TierContext(run_tier="full", event_name="merge_group")
        code, _ = run(tiny_cfg(), [[R(SELECT_DRAFT), GREEN]], tier_ctx=ctx)
        self.assertEqual(code, 0)

    # ---- superseded-cancellation forgiveness --------------------------------
    def test_superseded_cancelled_forgiven(self):
        """A cancelled leg with a LATER same-named non-cancelled run (the
        ready_for_review concurrency-cancel artifact) must not RED the gate —
        tier-independent (matches branch protection's latest-run semantics)."""
        old = R("build + test", conclusion="cancelled",
                started="2026-07-17T10:00:00Z", rid=1)
        new = R("build + test", started="2026-07-17T10:05:00Z", rid=2)
        code, out = run(tiny_cfg(), [[old, new, GREEN2]])
        self.assertEqual(code, 0)
        self.assertIn("superseded-cancelled forgiven", out)

    def test_cancelled_without_successor_still_fails(self):
        code, _ = run(tiny_cfg(), [[R("build + test", conclusion="cancelled",
                                      started="2026-07-17T10:00:00Z", rid=1)]])
        self.assertEqual(code, 1)

    def test_genuine_failure_never_forgiven(self):
        """Only cancelled/stale are supersedable — a real FAILURE gates even with
        a later same-named success (no retry-away of a red)."""
        old = R("build + test", conclusion="failure",
                started="2026-07-17T10:00:00Z", rid=1)
        new = R("build + test", started="2026-07-17T10:05:00Z", rid=2)
        code, _ = run(tiny_cfg(), [[old, new]])
        self.assertEqual(code, 1)

    def test_cancelled_draft_select_forgiven_by_full_successor(self):
        """The draft-marked select cancelled mid-flight at un-draft has no later
        run under its OWN name — the tier-NORMALIZED name lets the full-tier
        select supersede it, so the select-health rule doesn't false-RED."""
        old = R(SELECT_DRAFT, conclusion="cancelled",
                started="2026-07-17T10:00:00Z", rid=1)
        new = R(SELECT_FULL, started="2026-07-17T10:05:00Z", rid=2)
        code, out = run(tiny_cfg(), [[old, new, GREEN]],
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 0)

    def test_cancelled_gate_predecessor_superseded_by_self(self):
        """The old draft-tier gate run cancelled by the ready_for_review
        concurrency group leaves a cancelled `gate` check-run on the SHA; THIS
        run's own (self-excluded) `gate` check-run must count as its superseder."""
        old_gate = R("gate", conclusion="cancelled",
                     started="2026-07-17T10:00:00Z", rid=1,
                     url="https://github.com/o/r/actions/runs/111/job/5")
        self_gate = R("gate", status="in_progress", conclusion=None,
                      started="2026-07-17T10:05:00Z", rid=2,
                      url="https://github.com/o/r/actions/runs/999/job/9")
        code, out = run(tiny_cfg(), [[old_gate, self_gate, GREEN]])
        self.assertEqual(code, 0, out)

    # ---- pure helpers -------------------------------------------------------
    def test_marker_helpers(self):
        self.assertTrue(g.is_draft_tier(SELECT_DRAFT))
        self.assertFalse(g.is_draft_tier(SELECT_FULL))
        self.assertEqual(g.normalized_name(SELECT_DRAFT), SELECT_FULL)
        self.assertTrue(g.is_select(SELECT_DRAFT),
                        "the draft-marked select must still satisfy SELECT_RE")
        self.assertFalse(g.is_advisory(SELECT_DRAFT))

    def test_draft_selects_unsuperseded_requires_strictly_later_full(self):
        draft_sel = R(SELECT_DRAFT, started="2026-07-17T10:05:00Z", rid=2)
        earlier_full = R(SELECT_FULL, started="2026-07-17T10:00:00Z", rid=1)
        self.assertEqual(g.draft_selects_unsuperseded([draft_sel, earlier_full]),
                         [SELECT_DRAFT],
                         "an EARLIER full select cannot supersede a draft one")
        later_full = R(SELECT_FULL, started="2026-07-17T10:06:00Z", rid=3)
        self.assertEqual(
            g.draft_selects_unsuperseded([draft_sel, earlier_full, later_full]), [])

    def test_draft_selects_unsuperseded_is_per_instance(self):
        """One full-tier successor covers exactly ONE draft-marked instance —
        same-named instances (the four selecting workflows) each need their own,
        and duplicates are preserved so the caller can report counts."""
        d1 = R(SELECT_DRAFT, started="2026-07-17T10:00:00Z", rid=1)
        d2 = R(SELECT_DRAFT, started="2026-07-17T10:00:01Z", rid=2)
        d3 = R(SELECT_DRAFT, started="2026-07-17T10:00:02Z", rid=3)
        f1 = R(SELECT_FULL, started="2026-07-17T10:05:00Z", rid=11)
        f2 = R(SELECT_FULL, started="2026-07-17T10:05:01Z", rid=12)
        f3 = R(SELECT_FULL, started="2026-07-17T10:05:02Z", rid=13)
        # 3 marked, 1 full => 2 instances remain unsuperseded (duplicates kept).
        self.assertEqual(g.draft_selects_unsuperseded([d1, d2, d3, f1]),
                         [SELECT_DRAFT, SELECT_DRAFT])
        # 3 marked, 3 later fulls => all matched.
        self.assertEqual(g.draft_selects_unsuperseded([d1, d2, d3, f1, f2, f3]), [])
        # An EARLIER full can never be a successor, even when otherwise unused.
        f_early = R(SELECT_FULL, started="2026-07-17T09:59:00Z", rid=10)
        self.assertEqual(g.draft_selects_unsuperseded([d1, d2, f_early, f1]),
                         [SELECT_DRAFT])
        # Interleaved rounds pair in start order: a full between two marked
        # instances supersedes the earlier one only.
        d_late = R(SELECT_DRAFT, started="2026-07-17T10:06:00Z", rid=4)
        self.assertEqual(g.draft_selects_unsuperseded([d1, f1, d_late]),
                         [SELECT_DRAFT])
        # started_at ties break on the check-run id (strictly-later still holds).
        d_tie = R(SELECT_DRAFT, started="2026-07-17T10:05:00Z", rid=20)
        f_tie = R(SELECT_FULL, started="2026-07-17T10:05:00Z", rid=21)
        self.assertEqual(g.draft_selects_unsuperseded([d_tie, f_tie]), [])
        self.assertEqual(
            g.draft_selects_unsuperseded(
                [d_tie, R(SELECT_FULL, started="2026-07-17T10:05:00Z", rid=19)]),
            [SELECT_DRAFT])

    # ---- draft-tier gate artifacts (the structural name tiering) -------------
    def test_gate_name_constants(self):
        """The gate's own tiered check-run name (ci-summary.yml renders
        `gate, draft-tier` on draft payloads) — pinned against the helpers."""
        self.assertEqual(g.GATE_CHECK_NAME, "gate")
        self.assertEqual(g.DRAFT_TIER_GATE_NAME, "gate, draft-tier")
        self.assertTrue(g.is_draft_gate_artifact("gate, draft-tier"))
        self.assertFalse(g.is_draft_gate_artifact("gate"),
                         "the full-tier gate name is NOT an artifact (a future "
                         "sibling job literally named `gate` must keep gating)")
        self.assertFalse(g.is_draft_gate_artifact("some gate, draft-tier"))
        self.assertEqual(g.normalized_name(g.DRAFT_TIER_GATE_NAME), "gate")
        self.assertTrue(g.is_draft_tier(g.DRAFT_TIER_GATE_NAME))
        self.assertFalse(g.is_advisory(g.DRAFT_TIER_GATE_NAME))
        self.assertFalse(g.is_select(g.DRAFT_TIER_GATE_NAME))

    def test_stale_draft_gate_failure_is_excluded_not_a_leg(self):
        """A COMPLETED draft-tier gate verdict left on the SHA is a tier
        artifact: its FAILURE must not permanently RED the full-tier gate on
        the same head (the live run re-derives the verdict over the real
        legs). Non-vacuous: without the exclusion this red would gate."""
        art = R(g.DRAFT_TIER_GATE_NAME, conclusion="failure",
                started="2026-07-17T10:00:00Z", rid=1,
                url="https://github.com/o/r/actions/runs/111/job/5")
        code, out = run(tiny_cfg(), [[art, GREEN, GREEN2]])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_cancelled_draft_gate_artifact_needs_no_successor(self):
        """A cancelled `gate, draft-tier` (concurrency-cancel at un-draft) is
        excluded as an artifact even before any successor registers — it must
        not RED the fresh full-tier gate while the sibling set settles."""
        art = R(g.DRAFT_TIER_GATE_NAME, conclusion="cancelled",
                started="2026-07-17T10:00:00Z", rid=1,
                url="https://github.com/o/r/actions/runs/111/job/5")
        code, out = run(tiny_cfg(), [[art, GREEN]])
        self.assertEqual(code, 0, out)


SELECT_NO_LEG = "select / select (change-based test selection, no-leg)"


def _job(name, *, run_id, status="completed", conclusion="success", jid=0,
         started="2026-07-25T07:15:40Z"):
    """An Actions Jobs-API payload as make_fetch_attempt_jobs returns it."""
    return {"id": jid or run_id * 10, "name": name, "status": status,
            "conclusion": conclusion, "started_at": started,
            "html_url": f"https://github.test/o/r/actions/runs/{run_id}/job/{jid or 1}"}


def _check(name, *, run_id, status="completed", conclusion="success", rid=0,
           started="2026-07-25T07:15:40Z"):
    """A commit check-run whose details_url locates its Actions run (the locator
    no_leg_run_ids + the newest-run resolver both key off)."""
    return R(name, status=status, conclusion=conclusion, rid=rid or run_id,
             started=started,
             url=f"https://github.test/o/r/actions/runs/{run_id}/job/{rid or 1}")


class TestNoLegRunExclusion(unittest.TestCase):
    """[OPUS-5] #3781 — a run that assembled NO LEGS is not evidence, so it must
    never become the authoritative newest run for its workflow.

    THE MEASURED SHAPE (sparq #3472, 2026-07-25). The PR was readied at 07:15:37 and
    four full-tier runs started (CI's real matrix ran until 07:34:08). At 07:28:50 the
    review pipeline re-drafted it and flipped `review:needs`; that label event started
    8 more runs which completed within ~90s with EVERY job skipped except the
    unconditional `select` — named `…, draft-tier`, because the PR was a draft again.
    Two independent harms followed, and both are pinned here:
      (1) four draft-marked select instances appeared with no possible full-tier
          successor, so the gate burned all 155 polls and refused (the #3781 deadlock);
      (2) newest-run resolution is per-workflow, so those vacuous runs BECAME
          authoritative for CI/Benchmarks/feature-matrix/fuzz — meaning the real,
          still-in-flight legs were discarded and replaced by `skipped` ones.

    Harm (2) is why the fix ignores the RUN rather than merely neutralising the select
    NAME: neutralising the name alone would have turned the deadlock into a GREEN gate
    rendered over legs that had not finished. Every test below drives the PRODUCTION
    call site (WorkflowRunResolver.__call__), not resolve_newest_workflow_runs
    directly, so dropping the `no_leg_ids=` argument there is caught."""

    def _resolver(self, checks_by_poll, workflows_by_poll):
        """A WorkflowRunResolver over scripted (checks, workflows) polls."""
        state = {"i": 0}

        def nth(seq):
            i = min(state["i"], len(seq) - 1)
            return [dict(x) for x in seq[i]]

        def fetch_checks():
            return nth(checks_by_poll)

        def fetch_workflows():
            out = nth(workflows_by_poll)
            state["i"] += 1
            return out

        jobs = {}
        posts = []

        def fetch_attempt_jobs(run_id, attempt):
            return [dict(j) for j in jobs.get(run_id, [])]

        resolver = g.WorkflowRunResolver(
            self_run_id="999",
            fetch_checks=fetch_checks,
            fetch_workflows=fetch_workflows,
            fetch_attempt_jobs=fetch_attempt_jobs,
            redispatch=posts.append,
        )
        resolver.jobs = jobs
        resolver.posts = posts
        return resolver

    def _drive(self, resolver, cfg=None, tier_ctx=None, depth=0):
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.run_gate(cfg or tiny_cfg(), resolver, lambda: depth,
                              sleep_fn=lambda s: None, tier_ctx=tier_ctx)
        return code, out.getvalue()

    # ---- (a) the suppression -------------------------------------------------
    def test_a_label_flip_no_op_run_cannot_erase_a_real_runs_FAILURE(self):
        """(a) THE SHARPEST FORM. The real full-tier run FAILED. A later label-flip
        no-op run of the same workflow must NOT supersede it — a red must survive a
        label flip. Mutating the suppression away (dropping `no_leg_ids=` at the call
        site, or making no_leg_run_ids return an empty set) makes the vacuous run
        newest, its `skipped` legs satisfy the gate, and this GREENS."""
        real = W(101, 7, name="CI", conclusion="failure",
                 created="2026-07-25T07:15:37Z")
        flip = W(102, 7, name="CI", conclusion="success",
                 created="2026-07-25T07:28:57Z")
        checks = [
            _check("test shard", run_id=101, conclusion="failure", rid=1),
            _check(SELECT_FULL, run_id=101, rid=2),
            _check("test shard", run_id=102, conclusion="skipped", rid=3,
                   started="2026-07-25T07:29:23Z"),
            _check(SELECT_NO_LEG, run_id=102, rid=4,
                   started="2026-07-25T07:29:23Z"),
        ]
        resolver = self._resolver([checks], [[real, flip]])
        resolver.jobs[101] = [_job("test shard", run_id=101, conclusion="failure"),
                              _job("select / select (change-based test selection)",
                                   run_id=101, jid=2)]
        resolver.jobs[102] = [_job("test shard", run_id=102, conclusion="skipped"),
                              _job(SELECT_NO_LEG, run_id=102, jid=2)]
        code, out = self._drive(resolver)
        self.assertEqual(
            code, 1,
            "a guarded label-flip no-op run must NOT supersede the real run and erase "
            "its FAILURE — the gate went GREEN over a vacuous all-skipped run "
            f"(#3781). Output:\n{out}")
        self.assertIn("declared NO LEGS", out)

    def test_a_label_flip_no_op_run_cannot_discard_an_inflight_real_run(self):
        """(a, second harm) The real run is STILL RUNNING when the flip lands (the
        measured #3472 timing: real CI finished 07:34:08, the flip runs completed by
        07:29:47). The gate must keep WAITING on the real legs and conclude on them —
        not conclude immediately over the flip run's skipped ones."""
        inflight = W(101, 7, name="CI", status="in_progress", conclusion=None,
                     created="2026-07-25T07:15:37Z")
        done = W(101, 7, name="CI", conclusion="success",
                 created="2026-07-25T07:15:37Z")
        flip = W(102, 7, name="CI", conclusion="success",
                 created="2026-07-25T07:28:57Z")
        pending_checks = [
            _check("test shard", run_id=101, status="in_progress", conclusion=None,
                   rid=1),
            _check(SELECT_FULL, run_id=101, rid=2),
            _check(SELECT_NO_LEG, run_id=102, rid=4,
                   started="2026-07-25T07:29:23Z"),
        ]
        done_checks = [
            _check("test shard", run_id=101, rid=1),
            _check(SELECT_FULL, run_id=101, rid=2),
            _check(SELECT_NO_LEG, run_id=102, rid=4,
                   started="2026-07-25T07:29:23Z"),
        ]
        resolver = self._resolver(
            [pending_checks, pending_checks, done_checks],
            [[inflight, flip], [inflight, flip], [done, flip]],
        )
        resolver.jobs[101] = [_job("test shard", run_id=101),
                              _job(SELECT_FULL, run_id=101, jid=2)]
        code, out = self._drive(resolver)
        self.assertEqual(code, 0, out)
        self.assertIn(
            "attempt 1: 3 check-run(s), 2 running", out,
            "the in-flight real leg must still be counted as PENDING — a vacuous "
            "label-flip run superseded it, so the gate stopped waiting for the real "
            f"matrix (#3781). Output:\n{out}")

    def test_the_no_leg_select_creates_no_draft_tier_hold(self):
        """(a, the deadlock itself) The exact #3472 head: a completed real full-tier
        run plus a later label-flip no-op run, gate at FULL tier, PR currently a
        DRAFT. The no-leg select must create NO hold — with the pre-fix
        `, draft-tier` name this is precisely the state that burned 155 polls."""
        real = W(101, 7, name="CI", created="2026-07-25T07:15:37Z")
        flip = W(102, 7, name="CI", created="2026-07-25T07:28:57Z")
        checks = [
            _check("test shard", run_id=101, rid=1),
            _check(SELECT_FULL, run_id=101, rid=2),
            _check(SELECT_NO_LEG, run_id=102, rid=4,
                   started="2026-07-25T07:29:23Z"),
        ]
        resolver = self._resolver([checks], [[real, flip]])
        resolver.jobs[101] = [_job("test shard", run_id=101),
                              _job(SELECT_FULL, run_id=101, jid=2)]
        code, out = self._drive(
            resolver, tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 0, out)
        self.assertNotIn(
            "awaiting the full-tier re-run", out,
            "a no-leg select must not manufacture a draft-tier hold: its run "
            "assembled zero legs and no full-tier successor can ever exist while "
            f"the PR is a draft (#3781). Output:\n{out}")
        self.assertIn("PASSED", out)

    def test_a_cancelled_no_leg_run_is_never_redispatched(self):
        """(a) The exclusion must reach the #3505 REDISPATCH decision too, not only the
        check-resolution pass. A cancelled label-flip no-op run would otherwise look
        like `the newest run of this workflow was cancelled`, burning the once-only
        `actions: write` re-run POST on a vacuous run and then REDding the gate with
        `superseded-legs, re-run required`. All three consumers of `newest` (redispatch,
        the attempt-jobs inventory, resolution) must agree on who is authoritative."""
        real = W(101, 7, name="CI", created="2026-07-25T07:15:37Z")
        cancelled_flip = W(102, 7, name="CI", conclusion="cancelled",
                           created="2026-07-25T07:28:57Z")
        checks = [
            _check("test shard", run_id=101, rid=1),
            _check(SELECT_FULL, run_id=101, rid=2),
            _check(SELECT_NO_LEG, run_id=102, conclusion="cancelled", rid=4,
                   started="2026-07-25T07:29:23Z"),
        ]
        resolver = self._resolver([checks], [[real, cancelled_flip]])
        resolver.jobs[101] = [_job("test shard", run_id=101),
                              _job(SELECT_FULL, run_id=101, jid=2)]
        code, out = self._drive(resolver)
        self.assertFalse(
            resolver.posts,
            "a cancelled label-flip NO-OP run must never be re-dispatched — it "
            "assembled no legs, so re-running it produces no evidence and its "
            f"cancellation is not a superseded-legs event (#3781). Output:\n{out}")
        self.assertEqual(code, 0, out)
        self.assertNotIn("superseded-legs, re-run required", out)

    # ---- (b) the discrimination: draft-tier CI must NOT be blinded -----------
    def test_a_genuine_draft_tier_run_stays_authoritative_and_still_holds(self):
        """(b) THE OTHER HALF. A real DRAFT-TIER run (a draft `synchronize`, which
        assembles a genuinely reduced leg set) must NOT be ignored: it stays the
        authoritative newest run AND its draft-marked select still creates the hold
        that keeps draft-assembled legs out of the merge queue. A fix that ignored
        draft-tier runs too would blind draft-tier CI completely."""
        draft_run = W(201, 7, name="CI", created="2026-07-25T06:00:00Z")
        checks = [
            _check("test shard", run_id=201, rid=1, started="2026-07-25T06:00:10Z"),
            _check(SELECT_DRAFT, run_id=201, rid=2, started="2026-07-25T06:00:10Z"),
        ]
        resolver = self._resolver([checks], [[draft_run]])
        resolver.jobs[201] = [_job("test shard", run_id=201,
                                   started="2026-07-25T06:00:10Z"),
                              _job(SELECT_DRAFT, run_id=201, jid=2,
                                   started="2026-07-25T06:00:10Z")]
        code, out = self._drive(
            resolver, tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 1, out)
        self.assertNotIn(
            "declared NO LEGS", out,
            "a genuinely draft-tier run assembles REAL (reduced) legs and must stay "
            f"authoritative — the fix must not blind draft-tier CI. Output:\n{out}")
        self.assertIn("awaiting the full-tier re-run", out,
                      "the draft-tier hold must still be created for a real "
                      "draft-assembled leg set")
        self.assertIn("test shard", [r.get("name") for r in resolver()],
                      "the draft-tier run's own legs must still be judged")

    # ---- predicate-level belts ----------------------------------------------
    def test_marker_predicates_discriminate_all_three_tiers(self):
        self.assertEqual(g.NO_LEG_MARKER, ", no-leg")
        self.assertEqual(g.select_tier(SELECT_FULL), "full")
        self.assertEqual(g.select_tier(SELECT_DRAFT), "draft")
        self.assertEqual(g.select_tier(SELECT_NO_LEG), "no-leg")
        self.assertTrue(g.is_no_leg_select(SELECT_NO_LEG))
        self.assertFalse(g.is_no_leg_select(SELECT_DRAFT))
        self.assertFalse(g.is_no_leg_select(SELECT_FULL))
        self.assertFalse(g.is_draft_tier(SELECT_NO_LEG),
                         "a no-leg select must never read as draft-tier")
        self.assertTrue(g.is_select(SELECT_NO_LEG))
        self.assertFalse(g.is_advisory(SELECT_NO_LEG))
        self.assertEqual(g.normalized_name(SELECT_NO_LEG), SELECT_FULL)

    def test_a_compound_select_can_never_declare_its_run_evidence_free(self):
        """FAIL-CLOSED hardening: only the PURE ci-select pre-job may make its run
        non-authoritative. A compound job that carries additional gating evidence
        (fv-select + fv-manifest) keeps its run authoritative even if the marker
        drifted onto its name."""
        compound = ("fv-select (change-based test selection, no-leg) + "
                    "fv-manifest (proof inventory)")
        self.assertFalse(g.is_no_leg_select(compound))
        self.assertEqual(
            g.no_leg_run_ids([_check(compound, run_id=303, rid=1)]), set(),
            "an evidence-bearing compound job must not be able to make its whole "
            "run a non-event")

    def test_no_leg_run_ids_reads_the_run_locator(self):
        self.assertEqual(
            g.no_leg_run_ids([_check(SELECT_NO_LEG, run_id=102, rid=4)]), {102})
        self.assertEqual(g.no_leg_run_ids([_check(SELECT_FULL, run_id=101, rid=1)]),
                         set())
        # No parseable run id => nothing to exclude (fail back to the old behaviour).
        self.assertEqual(g.no_leg_run_ids([R(SELECT_NO_LEG)]), set())

    def test_a_no_leg_select_can_neither_create_nor_discharge_a_hold(self):
        """The predicate-level belt (defence in depth: in production the whole run is
        already dropped upstream). A no-leg instance is in NEITHER pool."""
        no_leg = R(SELECT_NO_LEG, started="2026-07-25T07:29:23Z", rid=4)
        draft = R(SELECT_DRAFT, started="2026-07-25T06:00:00Z", rid=1)
        self.assertEqual(g.draft_selects_unsuperseded([no_leg]), [],
                         "a no-leg select must not CREATE a hold (#3781)")
        self.assertEqual(
            g.draft_selects_unsuperseded([draft, no_leg]), [SELECT_DRAFT],
            "a no-leg select must not DISCHARGE a real draft-tier hold — "
            "evidence-free selection can never stand in for the full-tier re-run")

    def test_a_no_leg_success_does_not_forgive_a_cancelled_real_select(self):
        """forgive_superseded is tier-aware in THREE values now: a later no-leg
        success must not excuse a cancelled draft-tier select (which would release
        the hold by deleting the instance that demands a successor)."""
        cancelled_draft = R(SELECT_DRAFT, conclusion="cancelled",
                            started="2026-07-25T06:00:00Z", rid=1)
        later_no_leg = R(SELECT_NO_LEG, started="2026-07-25T07:29:23Z", rid=4)
        kept, forgiven = g.forgive_superseded([cancelled_draft, later_no_leg])
        self.assertEqual(forgiven, [],
                         "a no-leg select must not forgive a cancelled real-tier "
                         "select")
        self.assertIn(cancelled_draft, kept)
        # The intended un-draft flow is untouched: a later FULL-tier select still does.
        later_full = R(SELECT_FULL, started="2026-07-25T07:29:23Z", rid=5)
        _, forgiven2 = g.forgive_superseded([cancelled_draft, later_full])
        self.assertEqual(forgiven2, [cancelled_draft])


class TestUnsatisfiableHoldFastFail(unittest.TestCase):
    """[OPUS-5] #3781 ask 2 — the gate can DETECT an unsatisfiable hold, so it must
    say so instead of burning the budget.

    Measured on #3472/#3468/#3681: the gate spent all 155 poll attempts printing
    `awaiting the full-tier re-run (draft-tier selection present)` and then emitted
    the refusal it could have emitted at poll 3 — ~37 minutes of silent burn per
    occurrence, on PRs with ZERO failing legs. The hold is a WAIT for the
    ready_for_review full-tier re-runs; when every sibling has concluded, a
    draft-marked select still lacks a successor, AND the PR is currently a draft, no
    such re-run can happen (only a non-draft payload produces a full-tier select), so
    the wait is unsatisfiable on arrival.

    This is a `return 1` — the same verdict the budget-exhaustion belt reaches, named
    honestly and ~1 minute in. It NEVER turns a would-be RED green."""

    def _stuck(self):
        """All-terminal set holding on one draft-marked select with no successor."""
        return [[R(SELECT_DRAFT, started="2026-07-25T07:29:23Z", rid=1), GREEN]]

    # ---- (c) detection + the naming diagnosis --------------------------------
    def test_unsatisfiable_hold_fails_fast_with_the_naming_diagnosis(self):
        cfg = tiny_cfg()
        fetch = scripted(self._stuck())
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.run_gate(cfg, fetch, lambda: 0, sleep_fn=lambda s: None,
                              tier_ctx=draft_ctx(counting(True), run_tier="full"))
        text = out.getvalue()
        self.assertEqual(code, 1, text)
        self.assertIn("UNSATISFIABLE draft-tier hold", text)
        self.assertIn("PR is CURRENTLY A DRAFT", text)
        self.assertIn("1 draft-marked select instance(s)", text)
        self.assertIn("gh pr ready", text, "the diagnosis must name the remedy")
        self.assertLess(
            fetch.state["calls"], cfg.max_total_polls,
            "the unsatisfiable hold must be DETECTED, not waited out: the gate "
            f"burned all {cfg.max_total_polls} polls on a refusal that was already "
            "decided at poll 3 (#3781 — measured 155 polls / ~37 min per "
            "occurrence, 3-for-3)")
        self.assertLessEqual(
            fetch.state["calls"], cfg.unsat_confirm_polls + cfg.min_polls,
            f"detection must fire within the confirm window; took "
            f"{fetch.state['calls']} polls")

    def test_the_fast_path_only_ever_reds(self):
        """No new exit-0 path: the detector's only outcome is the refusal."""
        source = (REPO_ROOT / "scripts" / "ci_summary_gate.py").read_text(
            encoding="utf-8")
        self.assertEqual(
            len(re.findall(r"^\s*return 0\b", source, re.M)), 5,
            "the #3781 detector must add only `return 1` paths — the gate's exit-0 "
            "surface is FIXED and enumerated in the module header (_draft_recheck's "
            "two, render_verdict's empty-set + PASS, and _self_test's)")

    # ---- (d) the discrimination: a still-settling set is NOT unsatisfiable ---
    def test_a_still_settling_set_is_not_declared_unsatisfiable(self):
        """(d) The hold with legs STILL RUNNING is exactly what the wait is for. It
        must not be called unsatisfiable, and once the full-tier select lands the
        gate must PASS."""
        draft_sel = R(SELECT_DRAFT, started="2026-07-25T07:29:23Z", rid=1)
        full_sel = R(SELECT_FULL, started="2026-07-25T07:35:00Z", rid=2)
        # The pending phase deliberately outlasts min_polls + unsat_confirm_polls, so a
        # detector that ignored `pending` WOULD fire here (and RED) before the re-run
        # registers. That is the whole discrimination.
        cfg = tiny_cfg(max_total_polls=12, base_polls=10)
        polls = [
            [draft_sel, PENDING],                    # legs still running: settling
            [draft_sel, PENDING],
            [draft_sel, PENDING],
            [draft_sel, PENDING],
            [draft_sel, PENDING],
            [draft_sel, PENDING],
            [draft_sel, full_sel, GREEN],            # the full-tier re-run registers
            [draft_sel, full_sel, GREEN],
            [draft_sel, full_sel, GREEN],
        ]
        code, out = run(cfg, polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertNotIn(
            "UNSATISFIABLE", out,
            "a set with siblings STILL RUNNING is genuinely settling — the hold is a "
            "WAIT for those legs, so declaring it unsatisfiable is a false RED "
            f"(#3781). Output:\n{out}")
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_a_hold_about_to_be_discharged_is_not_declared_unsatisfiable(self):
        """(d) The confirm window's REASON. The set is all-terminal and the hold looks
        stuck, but the full-tier select is merely a poll or two behind (check-run
        registration lag at the un-draft moment). Firing on the FIRST observation would
        RED a PR whose successor lands immediately afterwards; the state must persist
        for unsat_confirm_polls first."""
        draft_sel = R(SELECT_DRAFT, started="2026-07-25T07:29:23Z", rid=1)
        full_sel = R(SELECT_FULL, started="2026-07-25T07:35:00Z", rid=2)
        polls = [
            [draft_sel, GREEN],                      # all-terminal, hold looks stuck
            [draft_sel, GREEN],
            [draft_sel, full_sel, GREEN],            # lag over: the successor lands
            [draft_sel, full_sel, GREEN],
            [draft_sel, full_sel, GREEN],
        ]
        code, out = run(tiny_cfg(), polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertNotIn(
            "UNSATISFIABLE", out,
            "the confirm window exists so a successor that is merely LATE is not "
            f"mistaken for one that can never come (#3781). Output:\n{out}")
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_a_readied_pr_is_not_declared_unsatisfiable(self):
        """(d) The PR reads NON-draft: the ready_for_review full-tier re-runs may
        still be registering, so the hold is satisfiable and the detector must stand
        down. The pre-#3781 budget-exhaustion belt then renders the refusal.

        [OPUS-5] #4614 ask 2 confirms this as DECIDED, not an oversight: the idle
        head gets no exit of its own, because the exit is licensed by a causal fact
        (only a non-draft payload produces a full-tier select) that does not hold
        here — the successor may merely be late."""
        fetch_draft = counting(False)
        code, out = run(tiny_cfg(), self._stuck(),
                        tier_ctx=draft_ctx(fetch_draft, run_tier="full"))
        self.assertEqual(code, 1)
        self.assertNotIn("UNSATISFIABLE", out)
        self.assertIn("stale draft-tier run, full run pending", out)
        self.assertGreater(fetch_draft.state["calls"], 0,
                           "the detector must actually READ the live draft state")

    def test_unreadable_draft_state_falls_back_to_the_budget_belt(self):
        """An API failure must not manufacture a NEW failure mode: the detector
        stands down and the gate behaves exactly as it did pre-#3781 (hold, then RED
        at budget exhaustion via render_verdict's stale-draft-tier belt)."""
        cfg = tiny_cfg()
        fetch = scripted(self._stuck())
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.run_gate(
                cfg, fetch, lambda: 0, sleep_fn=lambda s: None,
                tier_ctx=draft_ctx(counting(g.FetchError("api down")),
                                   run_tier="full"))
        text = out.getvalue()
        self.assertEqual(code, 1, text)
        self.assertNotIn("UNSATISFIABLE", text)
        self.assertIn("stale draft-tier run, full run pending", text)
        self.assertEqual(fetch.state["calls"], cfg.max_total_polls,
                         "an unreadable draft state must fall back to the full "
                         "pre-#3781 budget, never to a fast RED")

    def test_no_hold_means_the_draft_api_is_never_touched(self):
        """Cost + blast-radius discipline: a full-tier run with no draft-tier hold
        must not read the PR's draft state at all (the pre-#3781 invariant)."""
        fetch_draft = counting(True)
        code, out = run(tiny_cfg(), [[GREEN, GREEN2]],
                        tier_ctx=draft_ctx(fetch_draft, run_tier="full"))
        self.assertEqual(code, 0, out)
        self.assertEqual(fetch_draft.state["calls"], 0)

    def test_the_startup_floor_is_respected(self):
        """min_polls guards a startup race: no verdict — including this one — before
        the floor, so a check set that has barely begun registering cannot be
        declared unsatisfiable."""
        cfg = tiny_cfg(min_polls=6, unsat_confirm_polls=1, max_total_polls=8)
        fetch = scripted(self._stuck())
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.run_gate(cfg, fetch, lambda: 0, sleep_fn=lambda s: None,
                              tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 1, out.getvalue())
        self.assertGreaterEqual(fetch.state["calls"], 6,
                                "the detector must not fire before min_polls")

    def test_a_failing_leg_still_fails_FAST_ahead_of_the_hold_diagnosis(self):
        """Precedence: a genuine failing leg is the better diagnosis, and the
        existing fail-fast path must still win."""
        polls = [[R(SELECT_DRAFT, started="2026-07-25T07:29:23Z", rid=1), RED,
                  PENDING]]
        code, out = run(tiny_cfg(), polls,
                        tier_ctx=draft_ctx(counting(True), run_tier="full"))
        self.assertEqual(code, 1)
        self.assertIn("FAILED (fail-fast)", out)
        self.assertIn("- ✗ clippy: failure", out)
        self.assertNotIn("UNSATISFIABLE", out)

    def test_a_non_pr_event_never_reaches_the_detector(self):
        """merge_group/push run on a fresh ref and have no PR to read."""
        fetch_draft = counting(True)
        ctx = g.TierContext(run_tier="full", event_name="merge_group",
                            fetch_pr_draft=fetch_draft)
        code, out = run(tiny_cfg(), [[R(SELECT_DRAFT), GREEN]], tier_ctx=ctx)
        self.assertEqual(code, 0, out)
        self.assertEqual(fetch_draft.state["calls"], 0)


class TestUnsatHoldRemedyIsStated(unittest.TestCase):
    """[OPUS-5] #4614 ask 1 (carry-over from the superseded #3765) — BOTH draft-tier
    refusals must say that re-running the gate is futile.

    Re-running `ci-summary` re-runs no SELECTING workflow, so a bare `gh run rerun`
    cannot make the missing full-tier select appear. Without that sentence an
    automated repair lane — whose reflex for any RED is "re-run it" — burns runner
    time on a verdict its re-run cannot move. The sentence must reach the reader, so
    these assert the RENDERED message, not just the constant's existence."""

    # The distinguishing clause: "re-running THIS workflow re-runs no selecting
    # workflow". Asserting on it (not on the constant name) is what makes these
    # tests go red if the tail is dropped, blanked, or reworded into vagueness.
    CLAIM = "re-runs no selecting workflow"

    def test_the_unsatisfiable_hold_red_carries_the_futility_remedy(self):
        """Site 1: the #3781 fail-fast RED."""
        cfg = tiny_cfg()
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.run_gate(cfg, scripted([[R(SELECT_DRAFT, rid=1), GREEN]]),
                              lambda: 0, sleep_fn=lambda s: None,
                              tier_ctx=draft_ctx(counting(True), run_tier="full"))
        text = out.getvalue()
        self.assertEqual(code, 1, text)
        self.assertIn("UNSATISFIABLE draft-tier hold", text)
        self.assertIn(self.CLAIM, text)
        self.assertIn("Do not re-run `ci-summary` for this verdict.", text)

    def test_the_stale_draft_tier_belt_carries_the_futility_remedy(self):
        """Site 2: render_verdict's stale-draft-tier belt — the budget-exhaustion
        refusal an idle head reaches (rule 4), which a repair lane sees just as
        often as site 1."""
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.render_verdict(
                [R(SELECT_DRAFT, rid=1), GREEN], "",
                g.TierContext(run_tier="full", event_name="pull_request"))
        text = out.getvalue()
        self.assertEqual(code, 1, text)
        self.assertIn("stale draft-tier run, full run pending", text)
        self.assertIn(self.CLAIM, text)

    def test_an_ordinary_failing_leg_does_not_carry_it(self):
        """The tail is scoped to the two draft-tier refusals. A plain failing leg IS
        cleared by a re-run (a flake re-runs green), so telling that reader not to
        re-run would be wrong — this catches a blanket append."""
        code, out = run(tiny_cfg(), [[RED, GREEN]])
        self.assertEqual(code, 1, out)
        self.assertNotIn(self.CLAIM, out)


class TestMergeGroupChangeClassAccounting(unittest.TestCase):
    """[FABLE-5] merge-group change-class gate (extends #3420/#3421): the expected
    per-class leg accounting for a MERGE-GROUP sibling set. On a docs-only/
    orchestration-only queued batch the rust_changed layer (ci.yml lint/msrv/
    geiger/docker-smoke/coverage-floors + feature-matrix setup/check-tier/
    fedclient-boundary) now SKIPS alongside the select-gated engine legs; every
    such leg still registers a check-run with conclusion `skipped` — an
    ATTRIBUTED skip under the green same-diff `select` pre-job — and the gate
    renders PASS. The fail-closed half: those same skips with a non-success
    select are UNATTRIBUTABLE and must RED (an engine batch could only see its
    legs skipped through a select that did not soundly conclude — the classify
    step and the select job compute the same classify_change over the same
    base_sha...head_sha, so on any resolved diff they cannot disagree)."""

    # The #2533-shaped docs-only merge-group batch: what the queue's merge_group
    # ref shows after this change — every engine-lane check-run PRESENT (never
    # missing) with conclusion `skipped`, cheap prose gates green, select green.
    _DOCS_ONLY_GROUP_SKIPS = [
        # ci.yml select-conjunct legs (already skipped via the batch-diff selection):
        "test (load-aware shard bulk 1/3)",
        "build + archive test binaries (+ doctests once)",
        "W3C SPARQL 1.1 conformance (ratchet)",
        # ci.yml rust_changed-only legs (NEWLY class-gated on merge_group):
        "lint (fmt + clippy + doc)",
        "MSRV build (workspace)",
        "cargo-geiger unsafe audit",
        "docker image smoke (bind posture + /health)",
        # feature-matrix.yml rust_changed-only legs (NEWLY class-gated):
        "assemble feature matrix",
        "feature-matrix check-tier (engine)",
        "fedclient dependency-boundary guard",
        "opt-in sparq-engine (paths)",
        # fuzz.yml (already select-gated on the batch diff):
        "cargo-fuzz (parsers + mmap loader, bounded)",
        "differential smoke (sparq vs Oxigraph, fixed regression windows)",
    ]

    def _group_runs(self, select_run):
        runs = [select_run]
        runs += [R(n, conclusion="skipped") for n in self._DOCS_ONLY_GROUP_SKIPS]
        runs += [
            R("docs-quality quick-gates"),
            R("supply-chain gates (deny + vet + SBOM + VEX + OpenSSF + js-sbom)"),
            # Deliberately FULL on merge groups (the documented #3421 unsound-skip
            # exception): wasm feature-OFF byte-equality on the MERGED bundle.
            R("vectorized wasm feature-OFF equality"),
        ]
        return runs

    def test_docs_only_merge_group_passes_with_attributed_skips(self):
        # The headline fixture: class=docs-only batch => gate PASS, every engine
        # leg an attributed skip, nothing missing. (No tier_ctx: merge_group runs
        # use the pre-draft-tier semantics, as in production.)
        runs = self._group_runs(SEL_OK)
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertIn(f"{len(self._DOCS_ONLY_GROUP_SKIPS)} skipped", out)
        self.assertIn("selection pre-job succeeded", out)
        self.assertNotIn("FAILED", out)

    def test_same_skips_with_failed_select_red(self):
        # Fail-closed: the identical skip set under a FAILED select is
        # unattributable => RED. This is what stops an engine batch from merging
        # on skipped legs — its legs can only skip through the select/classify
        # pair, and a select that did not conclude success reds the whole set.
        runs = self._group_runs(R(SELECT_NAME, conclusion="failure"))
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, out)
        self.assertIn("cannot be attributed to a sound selection", out)

    def test_same_skips_with_missing_select_conclusion_red(self):
        # A cancelled (unsuperseded) select is equally unattributable => RED.
        runs = self._group_runs(R(SELECT_NAME, conclusion="cancelled"))
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, out)

    def test_engine_leg_failure_never_masked_by_class_skips(self):
        # A genuinely failing sibling on the merge_group ref (e.g. the always-on
        # supply-chain SBOM steps, or a lane the class kept running) still REDs
        # the gate regardless of how many class-attributed skips surround it.
        runs = self._group_runs(SEL_OK) + [R("vectorized wasm feature-OFF equality (run)",
                                             conclusion="failure")]
        code, out = run(tiny_cfg(), [runs])
        self.assertEqual(code, 1, out)
        self.assertIn("FAILED", out)


FM_GROUP = R("opt-in group (sparq-engine 1/2)")          # a group job's own conclusion
FM_GROUP2 = R("opt-in group (sparq-server 1/1)")
FM_REPORT_OK = R("feature-matrix report")                 # reporter posted green
FM_REPORT_FAIL = R("feature-matrix report", conclusion="failure")
FM_REPORT_PENDING = R("feature-matrix report", status="in_progress", conclusion=None)


class TestFeatureMatrixReporterAwait(unittest.TestCase):
    """[FABLE-5] PR #3511 finding 1 (HIGH): ci-summary must STRUCTURALLY await the
    trusted feature-matrix reporter. When `opt-in group (…)` legs ran for this head
    (the reporter-timing-independent proof that legs were selected), the
    `feature-matrix report` summary check-run MUST exist and be terminal-SUCCESS
    before the gate can conclude green; a delayed/crashed reporter must never race
    past the gate — absence keeps the gate polling and FAILS CLOSED at timeout."""

    # ---- pure predicates ----------------------------------------------------
    def test_zero_leg_skeleton_placeholder_is_not_group_presence(self):
        """The unexpanded `${{ matrix.group }}` skeleton (zero-leg run, skipped) must
        NOT trigger the reporter requirement — production incident PR #3524: every
        docs/config-only PR timed out RED awaiting a verdict the reporter correctly
        never posts on a zero-leg run."""
        runs = [
            {"name": "opt-in group (${{ matrix.group }})", "status": "completed",
             "conclusion": "skipped", "details_url": ""},
        ]
        self.assertFalse(g.is_real_fm_group(runs[0]))
        self.assertEqual(g.fm_report_status(runs), "n/a")

    def test_forged_placeholder_name_with_success_still_requires_reporter(self):
        """SECURITY (sol on #3525): a REAL successful group whose PR-controlled name
        embeds `${{` must NOT masquerade as the skeleton — the exclusion requires
        the server-set skipped conclusion too."""
        runs = [
            {"name": "opt-in group (g01 ${{ attacker)", "status": "completed",
             "conclusion": "success", "details_url": ""},
        ]
        self.assertTrue(g.is_real_fm_group(runs[0]))
        self.assertEqual(g.fm_report_status(runs), "pending")

    def test_real_group_name_still_counts(self):
        runs = [
            {"name": "opt-in group (g01 sparq-engine)", "status": "completed",
             "conclusion": "success", "details_url": "https://github.com/o/r/actions/runs/123/job/9"},
        ]
        self.assertTrue(g.is_fm_group(runs[0]["name"]))
        self.assertEqual(g.fm_report_status(runs), "pending")

    def test_predicate_contract(self):
        self.assertTrue(g.is_fm_group("opt-in group (sparq-engine 1/2)"))
        self.assertFalse(g.is_fm_group("opt-in sparq-engine (paths)"),
                         "a per-LEG `opt-in <name>` check is not a group job")
        self.assertFalse(g.is_fm_group("feature-matrix report"))
        self.assertTrue(g.is_fm_report("feature-matrix report"))
        self.assertFalse(g.is_fm_report("feature-matrix report (extra)"))
        # Neither is advisory-excluded (both gate/participate normally).
        self.assertFalse(g.is_advisory("opt-in group (sparq-engine 1/2)"))
        self.assertFalse(g.is_advisory("feature-matrix report"))

    def test_status_helper(self):
        self.assertEqual(g.fm_report_status([GREEN, GREEN2]), "n/a")
        self.assertEqual(g.fm_report_status([FM_GROUP, GREEN]), "pending")
        self.assertEqual(g.fm_report_status([FM_GROUP, FM_REPORT_PENDING]), "pending")
        self.assertEqual(g.fm_report_status([FM_GROUP, FM_REPORT_OK]), "ok")
        self.assertEqual(g.fm_report_status([FM_GROUP, FM_REPORT_FAIL]), "failed")

    # ---- present-success => green ------------------------------------------
    def test_reporter_present_success_passes(self):
        code, out = run(tiny_cfg(), [[FM_GROUP, FM_GROUP2, FM_REPORT_OK, GREEN]])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_no_group_jobs_needs_no_reporter(self):
        """A doc-only PR (or a fully change-selected-out matrix / a merge_group
        that skipped the lane) has NO `opt-in group (…)` check-run, so no reporter
        is expected — the gate must not invent a requirement and false-RED."""
        code, out = run(tiny_cfg(), [[GREEN, GREEN2]])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    # ---- crashed reporter (failure conclusion) => gate fails ----------------
    def test_reporter_failure_reds(self):
        code, out = run(tiny_cfg(), [[FM_GROUP, FM_REPORT_FAIL, GREEN]])
        self.assertEqual(code, 1, out)
        self.assertIn("FAILED", out)
        # The dedicated reporter belt names the reporter (belt-and-braces on top of
        # the normal gating-set render, which would also RED the failed check).
        self.assertIn("feature-matrix report", out)

    # ---- absent reporter => not concludable; fail-closed at timeout ---------
    def test_absent_reporter_never_concludes_green_holds_then_reds(self):
        """The core finding: group legs green, but the reporter's check-run NEVER
        lands. The gate must NOT conclude green in the settle window (it holds,
        still-settling), and at budget exhaustion FAILS CLOSED — never a
        conclude-by-timing over the group jobs' bare successes."""
        # Every poll: group jobs terminal-green, but NO `feature-matrix report`.
        code, out = run(tiny_cfg(), [[FM_GROUP, GREEN]])
        self.assertEqual(code, 1, out)
        self.assertIn("awaiting the feature-matrix reporter verdict", out)
        self.assertIn("reporter verdict never landed", out)
        self.assertNotIn("PASSED", out)

    def test_absent_reporter_holds_settle_window_open(self):
        """Non-vacuity: the SAME green group set WITH the reporter present passes
        immediately, proving the RED above is caused by the missing reporter, not
        by an unrelated hang."""
        code, out = run(tiny_cfg(), [[FM_GROUP, FM_REPORT_OK, GREEN]])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_reporter_lands_late_then_passes(self):
        """The realistic race: the reporter (workflow_run) posts its verdict a few
        polls AFTER the group jobs finish. The gate holds until it lands, then the
        settle completes and it passes — the whole point of the structural await."""
        polls = [
            [FM_GROUP, GREEN],                    # groups done; reporter not in yet
            [FM_GROUP, GREEN],                    # still awaiting
            [FM_GROUP, FM_REPORT_OK, GREEN],      # reporter verdict lands
            [FM_GROUP, FM_REPORT_OK, GREEN],      # settle
            [FM_GROUP, FM_REPORT_OK, GREEN],
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("awaiting the feature-matrix reporter verdict", out)
        self.assertIn("PASSED", out)

    def test_reporter_pending_nonterminal_holds(self):
        """A present-but-in-progress reporter check must also hold (not conclude)."""
        polls = [
            [FM_GROUP, FM_REPORT_PENDING, GREEN],           # reporter in flight
            [FM_GROUP, FM_REPORT_PENDING, GREEN],
            [FM_GROUP, FM_REPORT_OK, GREEN],                # it finishes green
            [FM_GROUP, FM_REPORT_OK, GREEN],
            [FM_GROUP, FM_REPORT_OK, GREEN],
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)

    def test_failing_leg_still_fails_fast_while_awaiting_reporter(self):
        """A genuine failing gating leg must still fail FAST even while the gate
        is (correctly) holding for a not-yet-landed reporter verdict — the
        reporter await never masks a real failure."""
        polls = [
            [FM_GROUP, RED],   # group present (await armed) + a red leg, no reporter
            [FM_GROUP, RED],   # grace re-poll re-observes the same failure
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 1, out)
        self.assertIn("fail-fast", out)


# --- PR #3511 finding 2 (HIGH): same-SHA stale-report correlation ---------------
# feature-matrix reruns on the SAME head SHA (ready_for_review / label events), so a
# STALE `feature-matrix report` (success) from an EARLIER run can sit on the commit
# while the CURRENT run's reporter is delayed/crashed. The fix binds each report to
# its triggering feature-matrix run id (external_id) and requires it to equal the
# CURRENT `opt-in group (…)` run id (max `/actions/runs/<id>` across the group checks).
def _grp(run_id, name="opt-in group (sparq-engine 1/2)"):
    return R(name, url=f"https://github.com/o/r/actions/runs/{run_id}/job/7")


def _rep(run_id, conclusion="success", status="completed"):
    return R("feature-matrix report", status=status, conclusion=conclusion,
             external_id=str(run_id))


class TestFeatureMatrixReporterPostCapGrace(unittest.TestCase):
    """[GPT-5.6] #6299: only the correlated reporter gets one bounded tail."""

    @staticmethod
    def _drive(cfg, polls, depth=0, tier_ctx=None):
        fetch = scripted(polls)
        out = io.StringIO()
        with redirect_stdout(out):
            code = g.run_gate(
                cfg,
                fetch,
                lambda: depth,
                sleep_fn=lambda _s: None,
                tier_ctx=tier_ctx,
            )
        return code, out.getvalue(), fetch.state["calls"]

    def test_current_report_lands_after_ordinary_cap_and_settles(self):
        cfg = tiny_cfg(reporter_grace_polls=4)
        waiting = [_grp("222"), GREEN]
        in_progress = [
            _grp("222"),
            _rep("222", status="in_progress", conclusion=None),
            GREEN,
        ]
        current = [_grp("222"), _rep("222"), GREEN]
        # The live #6223 reporter check was absent at the cap; also pin the
        # equivalent states where it is visible but non-terminal, or first lands
        # terminal-success on the cap and still needs its second settle poll.
        cap_cases = (
            ("absent", waiting, [current, current], cfg.settle_polls),
            ("in progress", in_progress, [current, current], cfg.settle_polls),
            ("terminal", current, [current], 1),
        )
        for state, cap_state, tail, extra_calls in cap_cases:
            with self.subTest(report_at_cap=state):
                polls = [waiting] * (cfg.max_total_polls - 1) + [cap_state, *tail]

                code, out, calls = self._drive(cfg, polls)

                self.assertEqual(code, 0, out)
                self.assertEqual(calls, cfg.max_total_polls + extra_calls)
                self.assertIn("bounded reporter-only grace", out)
                self.assertIn("PASSED", out)

    def test_absent_reporter_exhausts_only_the_bounded_grace_then_reds(self):
        cfg = tiny_cfg(reporter_grace_polls=3)

        code, out, calls = self._drive(cfg, [[_grp("222"), GREEN]])

        self.assertEqual(code, 1, out)
        self.assertEqual(
            calls,
            cfg.max_total_polls + cfg.reporter_grace_polls,
            "the reporter-only grace must have a fixed, non-rearming bound",
        )
        self.assertIn("bounded reporter-only grace exhausted", out)
        self.assertIn("reporter verdict never landed", out)

    def test_report_success_on_final_grace_poll_cannot_skip_settle(self):
        # [GPT-6-ASTRA] A last-poll success has only one terminal observation;
        # the timeout fallback must not turn it green without the normal settle.
        cfg = tiny_cfg(reporter_grace_polls=3)
        waiting = [_grp("222"), GREEN]
        current = [_grp("222"), _rep("222"), GREEN]
        polls = [waiting] * (cfg.max_total_polls + cfg.reporter_grace_polls - 1)

        code, out, calls = self._drive(cfg, [*polls, current])

        self.assertEqual(code, 1, out)
        self.assertEqual(calls, cfg.max_total_polls + cfg.reporter_grace_polls)
        self.assertIn("reporter grace ended before the normal settle window", out)
        self.assertIn("reached only 1/2 poll(s)", out)
        self.assertNotIn("PASSED", out)

    def test_current_report_failure_during_grace_still_reds(self):
        cfg = tiny_cfg(reporter_grace_polls=4)
        waiting = [_grp("222"), GREEN]
        failed = [_grp("222"), _rep("222", conclusion="failure"), GREEN]
        polls = [waiting] * cfg.max_total_polls + [failed, failed]

        code, out, calls = self._drive(cfg, polls)

        self.assertEqual(code, 1, out)
        self.assertEqual(calls, cfg.max_total_polls + cfg.settle_polls)
        self.assertIn("reporter concluded a NON-SUCCESS verdict", out)
        self.assertNotIn("PASSED", out)

    def test_ordinary_pending_sibling_or_full_tier_hold_gets_no_reporter_grace(self):
        cfg = tiny_cfg(reporter_grace_polls=3)
        cases = (
            ("ordinary pending", [_grp("222"), GREEN, PENDING], 20, None),
            (
                "awaiting full tier",
                [_grp("222"), GREEN, R(SELECT_DRAFT, started="2026-07-25T07:00:00Z")],
                0,
                draft_ctx(lambda: False, run_tier="full"),
            ),
        )
        for name, runs, depth, tier_ctx in cases:
            with self.subTest(state=name):
                code, out, calls = self._drive(
                    cfg, [runs], depth=depth, tier_ctx=tier_ctx
                )

                self.assertEqual(code, 1, out)
                self.assertEqual(calls, cfg.max_total_polls)
                self.assertNotIn("bounded reporter-only grace", out)

    def test_post_cap_grace_stop_reason_is_written_to_step_summary(self):
        waiting = [_grp("222"), GREEN]
        ordinary_work_appears = [_grp("222"), GREEN, PENDING]
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "step-summary.md"
            cfg = tiny_cfg(reporter_grace_polls=3, summary_path=str(summary))
            polls = [waiting] * cfg.max_total_polls + [ordinary_work_appears]

            code, out, calls = self._drive(cfg, polls)

            self.assertEqual(code, 1, out)
            self.assertEqual(calls, cfg.max_total_polls + 1)
            message = "reporter-only grace stopped because ordinary pending work"
            self.assertIn(message, out)
            self.assertIn(message, summary.read_text(encoding="utf-8"))

    def test_stale_report_cannot_satisfy_or_outlive_the_bounded_grace(self):
        cfg = tiny_cfg(reporter_grace_polls=3)
        stale_only = [_grp("222"), _rep("111"), GREEN]

        code, out, calls = self._drive(cfg, [stale_only])

        self.assertEqual(code, 1, out)
        self.assertEqual(calls, cfg.max_total_polls + cfg.reporter_grace_polls)
        self.assertIn("reporter verdict never landed", out)
        self.assertNotIn("PASSED", out)


    # [GPT-6-ASTRA] #6437: failed cap observations may consume the existing
    # reporter-only tail, never buy extra ordinary work or assume a verdict.
    def test_cap_fetch_error_and_neighboring_controls_have_exact_counts(self):
        cfg = tiny_cfg(reporter_grace_polls=4)
        waiting = [_grp("222"), GREEN]
        current = [_grp("222"), _rep("222"), GREEN]
        cases = (
            ("healthy", [waiting] * 8 + [current, current]),
            ("cap error", [waiting] * 7 + [g.FetchError("cap"), current, current]),
            ("pre-cap error", [waiting] * 6 + [g.FetchError("pre-cap"), waiting, current, current]),
        )
        for name, polls in cases:
            with self.subTest(case=name):
                code, out, calls = self._drive(cfg, polls)
                self.assertEqual(code, 0, out)
                self.assertEqual(calls, 10)
                self.assertIn("PASSED", out)
                self.assertEqual(out.count("cap fetch recovery"), int(name == "cap error"))

    def test_cap_fetch_error_requires_last_observed_reporter_only_eligibility(self):
        waiting = [_grp("222"), GREEN]
        cases = (
            ("ordinary work", [*waiting, PENDING], None, 4),
            ("full-tier hold", [*waiting, R(SELECT_DRAFT)],
             draft_ctx(lambda: False, run_tier="full"), 4),
            ("failed report", [*waiting, _rep("222", conclusion="failure")], None, 4),
            ("grace disabled", waiting, None, 0),
        )
        for name, last_observation, tier_ctx, grace in cases:
            with self.subTest(case=name):
                cfg = tiny_cfg(reporter_grace_polls=grace)
                polls = [waiting] * 6 + [last_observation, g.FetchError("cap")]
                code, out, calls = self._drive(cfg, polls, depth=20, tier_ctx=tier_ctx)
                self.assertEqual(code, 1, out)
                self.assertEqual(calls, 8)
                self.assertNotIn("cap fetch recovery", out)
                self.assertNotIn("PASSED", out)

    def test_cap_fetch_error_preserves_normal_successful_observation_settle(self):
        waiting = [_grp("222"), GREEN]
        current = [*waiting, _rep("222")]
        polls = [waiting] * 6 + [current, g.FetchError("cap"), current]
        code, out, calls = self._drive(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertEqual(calls, 9)
        self.assertIn("all-terminal stable for 2/2", out)
        self.assertIn("cap fetch recovery", out)

    def test_cap_recovery_keeps_the_consecutive_fetch_failure_limit(self):
        waiting = [_grp("222"), GREEN]
        for failure_limit, calls_expected in ((3, 10), (1, 8)):
            with self.subTest(limit=failure_limit):
                cfg = tiny_cfg(reporter_grace_polls=4,
                               max_consec_fetch_failures=failure_limit)
                code, out, calls = self._drive(
                    cfg, [waiting] * 7 + [g.FetchError("still down")])
                self.assertEqual(code, 1, out)
                self.assertEqual(calls, calls_expected)
                self.assertIn(f"{failure_limit} consecutive check-run fetch failures", out)
                self.assertNotIn("PASSED", out)

    def test_cap_recovery_errors_consume_the_fixed_tail_without_rearming(self):
        cfg = tiny_cfg(reporter_grace_polls=4)
        waiting = [_grp("222"), GREEN]
        polls = [waiting] * 7 + [g.FetchError("cap"), waiting,
                                g.FetchError("tail"), waiting, g.FetchError("last")]
        code, out, calls = self._drive(cfg, polls)
        self.assertEqual(code, 1, out)
        self.assertEqual(calls, 12)
        self.assertEqual(out.count("cap fetch recovery"), 1)
        self.assertIn("bounded reporter-only grace exhausted", out)
        self.assertNotIn("PASSED", out)

    def test_cap_recovery_fresh_ordinary_work_or_full_hold_stops_immediately(self):
        waiting = [_grp("222"), GREEN]
        cases = (
            ("ordinary work", [*waiting, PENDING], None),
            ("full-tier hold", [*waiting, R(SELECT_DRAFT)],
             draft_ctx(lambda: False, run_tier="full")),
        )
        for name, fresh, tier_ctx in cases:
            with self.subTest(case=name):
                polls = [waiting] * 7 + [g.FetchError("cap"), fresh]
                code, out, calls = self._drive(tiny_cfg(), polls, tier_ctx=tier_ctx)
                self.assertEqual(code, 1, out)
                self.assertEqual(calls, 9)
                self.assertIn("reporter-only grace stopped", out)
                self.assertNotIn("PASSED", out)

    def test_cap_recovery_never_accepts_stale_or_non_success_reporters(self):
        waiting = [_grp("222"), GREEN]
        cases = (("stale", _rep("111"), 12),
                 ("failed", _rep("222", conclusion="failure"), 10),
                 ("action required", _rep("222", conclusion="action_required"), 10))
        for name, report, expected_calls in cases:
            with self.subTest(case=name):
                polls = [waiting] * 7 + [g.FetchError("cap"), [*waiting, report]]
                code, out, calls = self._drive(tiny_cfg(reporter_grace_polls=4), polls)
                self.assertEqual(code, 1, out)
                self.assertEqual(calls, expected_calls)
                self.assertNotIn("PASSED", out)

    def test_cap_recovery_final_success_still_requires_normal_settle(self):
        waiting = [_grp("222"), GREEN]
        for grace in (1, 4):
            with self.subTest(grace=grace):
                cfg = tiny_cfg(reporter_grace_polls=grace)
                polls = [waiting] * 7 + [g.FetchError("cap")]
                polls += [waiting] * (grace - 1) + [[*waiting, _rep("222")]]
                code, out, calls = self._drive(cfg, polls)
                self.assertEqual(code, 1, out)
                self.assertEqual(calls, 8 + grace)
                self.assertIn("reached only 1/2 poll(s)", out)
                self.assertNotIn("PASSED", out)


class TestFeatureMatrixReporterCorrelation(unittest.TestCase):
    """[FABLE-5] PR #3511 finding 2 (HIGH): a `feature-matrix report` verdict must
    correlate to the CURRENT feature-matrix run — a stale same-SHA report from an
    earlier run can never satisfy the reporter-await for a fresh group run."""

    def test_group_run_id_extraction_takes_the_latest(self):
        # Two group runs on the same SHA (stale 111, fresh 222); the max wins.
        runs = [_grp("111"), _grp("222", "opt-in group (sparq-server 1/1)"), GREEN]
        self.assertEqual(g.fm_group_run_id(runs), "222")
        # No parseable url => "" (graceful degradation, never a crash).
        self.assertEqual(g.fm_group_run_id([FM_GROUP, GREEN]), "")
        # html_url fallback when details_url is absent.
        hurl = {"name": "opt-in group (x 1/1)", "status": "completed",
                "conclusion": "success", "details_url": "",
                "html_url": "https://github.com/o/r/actions/runs/333/job/1"}
        self.assertEqual(g.fm_group_run_id([hurl]), "333")

    def test_matching_run_success_is_ok(self):
        runs = [_grp("222"), _rep("222"), GREEN]
        self.assertEqual(g.fm_report_status(runs), "ok")

    def test_matching_run_failure_is_failed(self):
        runs = [_grp("222"), _rep("222", conclusion="failure"), GREEN]
        self.assertEqual(g.fm_report_status(runs), "failed")

    def test_duplicate_run_same_sha_stale_success_still_awaiting(self):
        """THE CORE RACE: the fresh group run is 222, but the only report on the SHA
        is the STALE run 111's green verdict. Its external_id (111) != current run id
        (222), so it is IGNORED — the gate is still awaiting the CURRENT run's
        reporter, NOT satisfied by the stale success."""
        runs = [_grp("222"), _rep("111"), GREEN]  # stale success masquerading
        self.assertEqual(g.fm_report_status(runs), "pending")

    def test_stale_plus_fresh_uses_fresh_verdict(self):
        # Stale 111 success sits alongside the fresh 222 report; the fresh one is
        # judged. Fresh green => ok; fresh red => failed (stale success cannot save).
        self.assertEqual(
            g.fm_report_status([_grp("222"), _rep("111"), _rep("222"), GREEN]), "ok")
        self.assertEqual(
            g.fm_report_status(
                [_grp("222"), _rep("111"), _rep("222", conclusion="failure"), GREEN]),
            "failed")

    def test_fresh_pending_alongside_stale_success_holds(self):
        # Stale 111 is green but the CURRENT 222 reporter is still in flight => hold.
        runs = [_grp("222"), _rep("111"),
                _rep("222", status="in_progress", conclusion=None), GREEN]
        self.assertEqual(g.fm_report_status(runs), "pending")

    def test_no_external_id_falls_back_to_any_report(self):
        # Legacy reporter (no external_id) + resolvable group run id: degrade to the
        # pre-finding-2 any-report match (never a false RED). FM_REPORT_OK has no
        # external_id; the group has a run id.
        runs = [_grp("222"), FM_REPORT_OK, GREEN]
        self.assertEqual(g.fm_report_status(runs), "ok")

    def test_unresolvable_group_run_id_falls_back_to_any_report(self):
        # Group check has no parseable run id (FM_GROUP has empty url) but the report
        # carries an external_id: with no current run id to bind to, degrade to
        # any-report matching rather than hang forever.
        runs = [FM_GROUP, _rep("999"), GREEN]
        self.assertEqual(g.fm_report_status(runs), "ok")

    def test_stale_success_gate_holds_then_reds_end_to_end(self):
        """End-to-end through the poll loop: every poll has the fresh group run (222)
        green + only the STALE run 111's green report. The gate must NOT conclude on
        the stale success — it holds, then FAILS CLOSED at budget exhaustion."""
        code, out = run(tiny_cfg(), [[_grp("222"), _rep("111"), GREEN]])
        self.assertEqual(code, 1, out)
        self.assertIn("awaiting the feature-matrix reporter verdict", out)
        self.assertIn("reporter verdict never landed", out)

    def test_fresh_report_lands_after_stale_then_passes_end_to_end(self):
        """The realistic recovery: a stale green report sits from run 111, then the
        CURRENT run 222's reporter posts its own green verdict; the gate correlates,
        settles on the fresh verdict, and passes."""
        polls = [
            [_grp("222"), _rep("111"), GREEN],                 # only stale => holding
            [_grp("222"), _rep("111"), GREEN],                 # still awaiting fresh
            [_grp("222"), _rep("111"), _rep("222"), GREEN],    # fresh verdict lands
            [_grp("222"), _rep("111"), _rep("222"), GREEN],    # settle
            [_grp("222"), _rep("111"), _rep("222"), GREEN],
        ]
        code, out = run(tiny_cfg(), polls)
        self.assertEqual(code, 0, out)
        self.assertIn("awaiting the feature-matrix reporter verdict", out)
        self.assertIn("PASSED", out)


# --- Fix round 3 (fable): latest-run-relative presence -------------------------
# Same-SHA zero-leg rerun (ready_for_review / label) leaves an OLDER real group run's
# check-runs on the commit alongside a NEWER zero-leg skeleton run. Keying presence
# off ANY real group (the old run's) while keying the run id off the newer skeleton
# deadlocked the gate: current_run_id = the skeleton's id, no report ever carries it
# (zero-leg posts none), matched=[] => "pending" => timeout RED. FIX: presence AND the
# reporter requirement are decided relative to the LATEST feature-matrix run.
def _skel(run_id="", conclusion="skipped"):
    """The zero-leg skeleton check-run (unexpanded placeholder + server-set skipped),
    optionally carrying its own run id url (it is a check-run of that run)."""
    url = f"https://github.com/o/r/actions/runs/{run_id}/job/1" if run_id else ""
    return R("opt-in group (${{ matrix.group }})", conclusion=conclusion, url=url)


class TestLatestRunRelativePresence(unittest.TestCase):
    """[FABLE-5] fix round 3: a same-SHA zero-leg rerun must CONCLUDE (n/a), not
    deadlock awaiting a reporter the zero-leg run correctly never posts."""

    def test_a_mixed_old_real_plus_new_skeleton_is_na(self):
        """(a) THE DEADLOCK: an OLD real group run (111) + its report(111) sit on the
        SHA next to a NEWER zero-leg skeleton run (222). The latest run (222) is
        zero-leg, so NO reporter is expected — the gate must conclude n/a, NOT await
        run 222's absent report and time out RED."""
        runs = [_grp("111"), _rep("111"), _skel("222")]
        # current run id is the newer skeleton's (222) — the very shape that deadlocked.
        self.assertEqual(g.fm_group_run_id(runs), "222")
        self.assertEqual(g.fm_report_status(runs), "n/a")

    def test_a_end_to_end_concludes_without_awaiting(self):
        """(a) end-to-end: the mixed old-real + new-skeleton head PASSES immediately
        (no reporter await), never RED-on-timeout."""
        code, out = run(tiny_cfg(), [[_grp("111"), _rep("111"), _skel("222"), GREEN]])
        self.assertEqual(code, 0, out)
        self.assertIn("PASSED", out)
        self.assertNotIn("reporter verdict never landed", out)

    def test_b_old_skeleton_plus_new_real_awaits_new_reporter(self):
        """(b) reversed order: an OLD zero-leg skeleton (111) sits next to the NEWER
        real run (222). The latest run has real legs => require ITS reporter. Absent
        => pending; present with external_id == 222 => ok; a stale-only 111 report
        (there is none here) could never satisfy it."""
        pending = [_skel("111"), _grp("222"), GREEN]
        self.assertEqual(g.fm_group_run_id(pending), "222")
        self.assertEqual(g.fm_report_status(pending), "pending")
        ok = [_skel("111"), _grp("222"), _rep("222"), GREEN]
        self.assertEqual(g.fm_report_status(ok), "ok")
        # A report only for the OLD run id does NOT satisfy the latest real run.
        stale_only = [_skel("111"), _grp("222"), _rep("111"), GREEN]
        self.assertEqual(g.fm_report_status(stale_only), "pending")

    def test_c_isolated_skeleton_is_na(self):
        """(c) existing invariant preserved: an isolated zero-leg skeleton (with OR
        without a locating url) requires no reporter."""
        self.assertEqual(g.fm_report_status([_skel()]), "n/a")
        self.assertEqual(g.fm_report_status([_skel("222")]), "n/a")

    def test_d_forged_placeholder_success_is_latest_still_requires_reporter(self):
        """(d) SECURITY (r2 carried forward): a REAL successful group whose PR-controlled
        name embeds the `${{` placeholder counts as REAL for the run it belongs to. When
        it is the LATEST run, the reporter is STILL required — it must not drop the
        requirement by masquerading as the zero-leg skeleton (which needs a server-set
        `skipped`, not `success`)."""
        forged = R("opt-in group (g01 ${{ attacker)", conclusion="success",
                   url="https://github.com/o/r/actions/runs/222/job/3")
        self.assertTrue(g.is_real_fm_group(forged))
        # Latest run 222 is this forged-but-real group => require reporter.
        self.assertEqual(g.fm_report_status([forged, GREEN]), "pending")
        # Its reporter, correlated to 222, satisfies it.
        self.assertEqual(g.fm_report_status([forged, _rep("222"), GREEN]), "ok")
        # A zero-leg skeleton on an OLDER run (111) does not drop the requirement.
        self.assertEqual(g.fm_report_status([_skel("111"), forged, GREEN]), "pending")

    def test_e_real_group_unparseable_url_fails_closed(self):
        """(e) fail-closed semantics: a REAL group check with NO parseable run id means
        real legs ran on a run we cannot identify — we must NOT declare the latest run
        zero-leg. The reporter requirement is kept (degrading to any-report correlation),
        never n/a. FM_GROUP has an empty url; alone it holds pending, and it must keep
        the requirement even when a NEWER skeleton carries a parseable id."""
        # Real group, no url => pending (require reporter), never n/a.
        self.assertEqual(g.fm_report_status([FM_GROUP, GREEN]), "pending")
        # A real-unparseable group is NOT masked away by a parseable skeleton: the
        # skeleton (222) would otherwise be "the latest run" and look zero-leg, but the
        # unparseable real leg forbids that n/a — fail closed to require the reporter.
        self.assertEqual(g.fm_report_status([FM_GROUP, _skel("222"), GREEN]), "pending")
        # With any legacy report present it degrades to any-report (never a false RED).
        self.assertEqual(g.fm_report_status([FM_GROUP, FM_REPORT_OK, GREEN]), "ok")


if __name__ == "__main__":
    unittest.main(verbosity=2)
