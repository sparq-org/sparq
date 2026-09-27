#!/usr/bin/env python3
# [GPT-5] #6293 — alarm/detector lanes must be explicitly kept outside the gate,
# either by schedule-event scoping or a registry declaration. 🤖 SPARQ agent.
#
# THE DEFECT THIS PINS. review-alarm.yml / formal-alarm.yml / selection-alarm.yml are
# detectors: they RED by design when they find a real blind spot, and they RED fail-loud
# when the detector itself breaks. Each one's header asserted it could never affect a
# merge, on this reasoning:
#
#     "it never creates a check-run on a PR head or merge-group ref, so it can never
#      enter the `ci-summary / gate` poll set"
#
# The premise is TRUE. The conclusion is FALSE. ci-summary.yml also runs on
# `push: branches: [main]` with `timeout-minutes: 80`, and a `schedule` (or
# `workflow_run`-on-main) alarm run puts its check-run on exactly that main head SHA —
# the one the push-triggered gate is polling. The gate deliberately waits for the
# check-run NAME SET to stabilise, so a sibling that appears mid-poll is picked up, not
# missed. Before #6292, the only applicable exclusion was a key in
# `.github/advisory-registry.json`, so undeclared detectors gated.
#
# MEASURED 2026-07-27, ci_summary_gate.render_verdict() over the real, paginated
# 477-check-run set for main head cb0c6739c7d85420474a2e67be83177660ee1be3:
#
#     ### ci-summary: FAILED — 7 non-passing gating check(s) of 462 gating
#     - ✗ review-lane blind-spot alarm: failure      <-- a DETECTOR gating main
#     - ✗ run + track benchmarks: failure
#
# WHY THIS MATTERS MORE THAN ONE RED SQUARE. An alarm that reds `main` for doing its job
# is an alarm somebody eventually mutes, and a muted alarm is strictly worse than no
# alarm — it looks like coverage. Schedule runs are now excluded by event scope;
# workflow_run detectors still need an explicit declaration.
#
# HOW THIS SUITE IS BUILT TO FAIL.
#   * Every behaviour test drives the REAL ci_summary_gate.render_verdict() — the exact
#     function the `gate` job runs — over a check-run set. No re-implementation of the
#     gating rule lives here, so the test cannot agree with a broken gate.
#   * test_control_* is the ANTI-VACUITY control: an UNDECLARED lane failing MUST still
#     RED the gate. Without it, a render_verdict() that returned 0 unconditionally (or a
#     fixture that silently contained no failure at all) would pass every other test
#     here. If the control ever goes green, this file is measuring nothing.
#   * test_every_alarm_workflow_has_an_explicit_scoping_mechanism DISCOVERS the alarm
#     workflows from the filesystem, then pins schedule scoping versus declaration.
#   * The suite pins its OWN call site in docs-quality.yml, so it cannot silently leave
#     CI's reachable set.
#
# Deleting the workflow_run alarm's registry entry REDs this file. Adding any swept
# schedule alarm back also REDs, protecting the registry's advisory-only meaning.
#
# Stdlib + PyYAML (already a docs-quality dependency). Run:
#   python3 scripts/tests/test_alarm_lanes_non_gating.py

from __future__ import annotations

import re
import sys
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts"))

import ci_summary_gate as gate  # noqa: E402

REGISTRY_PATH = REPO_ROOT / ".github" / "advisory-registry.json"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"

# The alarm/detector lanes. Each maps workflow file -> the bead that owns it. These are
# the lanes whose headers declare "NOT A GATE"; the point of this suite is that the
# CLAIM is bound to the MECHANISM that makes it true.
ALARM_WORKFLOWS = {
    "review-alarm.yml": "review-lane blind-spot alarm",
    "formal-alarm.yml": "formal-lane no-verdict alarm",
    "selection-alarm.yml": "nightly selection-bug alarm",
    "ci-latency-alarm.yml": "CI execution-latency alarm",
    "heavy-set-alarm.yml": "HEAVY-set drift alarm",
}
SCHEDULE_ALARM_WORKFLOWS = {
    workflow: lane
    for workflow, lane in ALARM_WORKFLOWS.items()
    if workflow != "selection-alarm.yml"
}
DECLARED_ALARM_WORKFLOWS = {
    "selection-alarm.yml": "nightly selection-bug alarm",
}

# A name that is deliberately NOT in the registry, used by the anti-vacuity control.
UNDECLARED_LANE = "some undeclared lane that must gate"


def _load_registry() -> None:
    """Install the LIVE registry into the gate module, exactly as the gate does."""
    gate.load_advisory_registry(str(REGISTRY_PATH))


def _run(name: str, conclusion: str, run_id: int = 1) -> dict:
    """One terminal check-run, shaped as the checks API returns it."""
    return {
        "name": name,
        "status": "completed",
        "conclusion": conclusion,
        "id": abs(hash((name, conclusion))) % 10_000_000,
        "_workflow_id": run_id,
    }


def _healthy_siblings() -> list[dict]:
    """A green sibling set, including the selection pre-job the gate requires to be
    successful before it will trust any `skipped` conclusion."""
    return [
        _run("changed-file test selection", "success"),
        _run("build", "success"),
        _run("clippy", "success"),
    ]


def _job_name(workflow_file: str) -> str:
    doc = yaml.safe_load((WORKFLOWS / workflow_file).read_text())
    jobs = doc["jobs"]
    assert len(jobs) == 1, f"{workflow_file}: expected exactly one job, got {list(jobs)}"
    (job,) = jobs.values()
    return job["name"]


class TestAlarmLanesAreNonGating(unittest.TestCase):
    """Pin the distinct scoping mechanisms for schedule and workflow_run alarms."""

    def setUp(self) -> None:
        _load_registry()

    def test_every_alarm_workflow_has_an_explicit_scoping_mechanism(self) -> None:
        """Discover every alarm, then require schedule scope or a declaration."""
        discovered = sorted(p.name for p in WORKFLOWS.glob("*alarm*.yml"))
        self.assertTrue(discovered, "no *alarm*.yml workflows found — glob broke")
        self.assertEqual(
            discovered,
            sorted(ALARM_WORKFLOWS),
            "the alarm workflow set changed; classify its trigger mechanism here",
        )
        for wf, name in SCHEDULE_ALARM_WORKFLOWS.items():
            with self.subTest(workflow=wf):
                self.assertFalse(gate.is_advisory(name))
                doc = yaml.safe_load((WORKFLOWS / wf).read_text())
                triggers = doc.get(True) or doc.get("on")
                self.assertIn("schedule", triggers)
                self.assertNotIn("workflow_run", triggers)

        for wf, name in DECLARED_ALARM_WORKFLOWS.items():
            with self.subTest(workflow=wf):
                self.assertTrue(gate.is_advisory(name))

    def test_registry_key_matches_the_live_yaml_job_name(self) -> None:
        """C4's binding, asserted from this side too: a rename must not be able to
        silently re-arm a detector as a merge blocker."""
        for wf, expected in ALARM_WORKFLOWS.items():
            with self.subTest(workflow=wf):
                self.assertEqual(_job_name(wf), expected)


class TestAlarmFailureDoesNotRedTheGate(unittest.TestCase):
    """The BEHAVIOUR half: drive the REAL render_verdict() and assert its exit code."""

    def setUp(self) -> None:
        _load_registry()

    def test_alarm_failure_does_not_red_the_gate(self) -> None:
        """The workflow_run detector remains non-gating through its declaration."""
        for wf, lane in DECLARED_ALARM_WORKFLOWS.items():
            with self.subTest(lane=lane):
                runs = _healthy_siblings() + [_run(lane, "failure")]
                rc = gate.render_verdict(runs)
                self.assertEqual(
                    rc,
                    0,
                    f"{wf}: a `failure` from the {lane!r} DETECTOR red the gate "
                    f"(render_verdict returned {rc}). It must be declared in "
                    f".github/advisory-registry.json.",
                )

    def test_control_undeclared_lane_failure_does_red_the_gate(self) -> None:
        """ANTI-VACUITY CONTROL. Identical fixture shape, but the failing lane is NOT
        declared. This MUST red. If this test ever passes-as-green, the tests above are
        measuring nothing — a render_verdict() that always returned 0, or a fixture
        carrying no real failure, would satisfy them all."""
        runs = _healthy_siblings() + [_run(UNDECLARED_LANE, "failure")]
        self.assertFalse(
            gate.is_advisory(UNDECLARED_LANE),
            "the control lane must stay UNdeclared for this control to mean anything",
        )
        rc = gate.render_verdict(runs)
        self.assertEqual(
            rc,
            1,
            "CONTROL FAILED: an UNDECLARED lane concluding `failure` did not red the "
            "gate. The non-gating assertions in this file are therefore vacuous.",
        )

    def test_declaring_an_alarm_does_not_forgive_a_real_gating_failure(self) -> None:
        """QUANTIFIER DIRECTION. Declaring the detectors must make the ALARM non-gating,
        not make the commit's own legs non-gating. A genuine leg failing alongside a
        failing alarm must still RED — this is what proves the fix did not buy green by
        weakening the gate."""
        runs = _healthy_siblings() + [
            _run("nightly selection-bug alarm", "failure"),
            _run("run + track benchmarks", "failure"),
        ]
        self.assertTrue(gate.is_advisory("nightly selection-bug alarm"))
        self.assertFalse(gate.is_advisory("run + track benchmarks"))
        rc = gate.render_verdict(runs)
        self.assertEqual(
            rc, 1, "a real gating leg's failure was forgiven — the gate was weakened"
        )


class TestTheRefutedPremise(unittest.TestCase):
    """Pin the two workflow facts whose CONJUNCTION is what made the old header's
    reasoning wrong, so nobody re-derives 'it can never enter the poll set'."""

    def test_ci_summary_gate_also_runs_on_pushes_to_main(self) -> None:
        """Half 1 of the refutation: the gate does not only poll PR / merge-group heads.
        This remains the reason event scoping is necessary on main."""
        doc = yaml.safe_load((WORKFLOWS / "ci-summary.yml").read_text())
        triggers = doc.get(True) or doc.get("on")
        self.assertIn(
            "push",
            triggers,
            "ci-summary.yml no longer runs on push — re-derive the sq-huwr8 rationale",
        )
        self.assertIn("main", triggers["push"]["branches"])

    def test_alarm_lanes_run_on_main_so_they_share_that_head_sha(self) -> None:
        """Half 2: each alarm is triggered by `schedule` or `workflow_run` on main, i.e.
        on a commit that a push-triggered `gate` run polls. The exact trigger type now
        selects whether schedule scoping or the registry keeps the detector non-gating."""
        for wf in ALARM_WORKFLOWS:
            with self.subTest(workflow=wf):
                doc = yaml.safe_load((WORKFLOWS / wf).read_text())
                triggers = doc.get(True) or doc.get("on")
                self.assertTrue(
                    {"schedule", "workflow_run"} & set(triggers),
                    f"{wf}: expected a schedule/workflow_run trigger on main",
                )

    def test_no_alarm_header_still_claims_it_cannot_enter_the_poll_set(self) -> None:
        """The false claim itself, pinned as a string. Re-introducing it REDs — a stale
        comment asserting a safety property the code does not provide is how this defect
        survived review in the first place."""
        false_claim = re.compile(
            r"never\s+enter\s+the\s*\n?#?\s*`?ci-summary\s*/\s*gate`?\s*\n?#?\s*poll\s+set",
            re.IGNORECASE,
        )
        for wf in ALARM_WORKFLOWS:
            with self.subTest(workflow=wf):
                text = (WORKFLOWS / wf).read_text()
                flat = re.sub(r"\n#\s*", " ", text)
                self.assertIsNone(
                    false_claim.search(flat),
                    f"{wf}: header still claims the lane can never enter the gate poll "
                    f"set. It can (ci-summary runs on push to main); what makes it "
                    f"non-gating is its advisory-registry declaration.",
                )


class TestSuiteIsWiredIntoCI(unittest.TestCase):
    """Anti-vacuity anchor: without this, the whole file could leave CI unnoticed."""

    def test_docs_quality_runs_this_suite(self) -> None:
        text = (WORKFLOWS / "docs-quality.yml").read_text()
        self.assertIn(
            "scripts/tests/test_alarm_lanes_non_gating.py",
            text,
            "docs-quality.yml no longer invokes this suite — it would stop running",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
