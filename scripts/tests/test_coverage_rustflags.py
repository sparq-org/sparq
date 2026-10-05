#!/usr/bin/env python3
# [SONNET-4.6] Hermetic tests for scripts/check-coverage-rustflags.py (bead sq-6vshe.11).
#
# Three layers:
#   1. The gate's PURE logic (the same fixtures as its built-in --self-test, asserted here
#      so a regression in the job splitter / job-level-env reader / llvm-cov detection is
#      caught by pytest and by the docs-quality ci-scripts job).
#   2. A LIVE invariant: this repo's own .github/workflows/*.yml MUST currently pass — every
#      cargo-llvm-cov job sets a job-level RUSTFLAGS and they all agree. Committed-files
#      only, no git/network, so it stays deterministic.
#   3. A VACUOUS-PASS guard: the live scan must actually find cargo-llvm-cov jobs. If the
#      detector ever stops recognising them (a renamed install-action input, a moved
#      invocation), layer 2 would pass by finding nothing — which is the failure mode this
#      whole gate exists to prevent, so it is asserted against explicitly.
#
# Run:  python3 scripts/tests/test_coverage_rustflags.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


gate = _load("check_coverage_rustflags", "check-coverage-rustflags.py")


class PureLogic(unittest.TestCase):
    """scan_text() fixtures: a cargo-llvm-cov job with no JOB-LEVEL RUSTFLAGS is the only
    thing that trips the presence half of the gate."""

    def test_good_job_level_rustflags(self):
        offences, values = gate.scan_text(gate._GOOD)
        self.assertEqual(offences, [])
        self.assertEqual(values, {"coverage-measure": "-C link-arg=-fuse-ld=lld"})

    def test_bad_missing_rustflags(self):
        offences, values = gate.scan_text(gate._BAD_NO_RUSTFLAGS)
        self.assertEqual(offences, ["coverage-measure"])
        self.assertEqual(values, {})

    def test_step_level_env_is_not_enough(self):
        # The whole point of the gate: cargo-llvm-cov compiles happen across several steps
        # (and inside the scripts they shell out to), so only a job-level env covers them.
        offences, _ = gate.scan_text(gate._BAD_STEP_LEVEL_ENV)
        self.assertEqual(offences, ["coverage-measure"])

    def test_csv_tool_list_still_detected(self):
        # `tool: cargo-llvm-cov,nextest` is how coverage-engine-{run,merge} install both.
        offences, values = gate.scan_text(gate._GOOD_CSV_TOOL)
        self.assertEqual(offences, [])
        self.assertIn("coverage-engine-run", values)

    def test_detected_via_run_invocation(self):
        # No install-action step at all — a bare `run: cargo llvm-cov …` still makes it a
        # coverage job, so the detector cannot be dodged by installing the tool elsewhere.
        offences, _ = gate.scan_text(gate._BAD_RUN_INVOCATION)
        self.assertEqual(offences, ["adhoc-coverage"])

    def test_no_compile_coverage_job_out_of_scope(self):
        # ci.yml's `coverage-floors` shape: runs coverage SCRIPTS but builds nothing, so no
        # linker flag applies and it must not be flagged.
        offences, values = gate.scan_text(gate._OK_NO_COMPILE_JOB)
        self.assertEqual(offences, [])
        self.assertEqual(values, {})

    def test_non_coverage_job_never_flagged(self):
        offences, values = gate.scan_text(gate._OK_OTHER_JOB)
        self.assertEqual(offences, [])
        self.assertEqual(values, {})

    def test_mixed_flags_only_the_offender(self):
        offences, values = gate.scan_text(gate._MIXED)
        self.assertEqual(offences, ["b"])
        self.assertEqual(list(values), ["a"])

    def test_divergent_values_are_surfaced(self):
        # Presence passes; it is the SAMENESS check (in main()) that must reject this. The
        # scanner's job is to report both distinct values so the caller can see the drift.
        offences, values = gate.scan_text(gate._DIVERGENT_VALUES)
        self.assertEqual(offences, [])
        self.assertEqual(len(set(values.values())), 2)


class ScalarParsing(unittest.TestCase):
    """The env value must survive quoting and trailing comments unchanged — a mis-parsed
    value would make two identical jobs look divergent (or vice versa)."""

    def test_quoted(self):
        self.assertEqual(gate._scalar('"-C link-arg=-fuse-ld=lld"'), "-C link-arg=-fuse-ld=lld")

    def test_single_quoted(self):
        self.assertEqual(gate._scalar("'-C link-arg=-fuse-ld=lld'"), "-C link-arg=-fuse-ld=lld")

    def test_unquoted_with_trailing_comment(self):
        self.assertEqual(gate._scalar("-C debuginfo=0   # thin"), "-C debuginfo=0")

    def test_hash_inside_quotes_is_not_a_comment(self):
        self.assertEqual(gate._scalar('"-C link-arg=-Wl,--x#y"'), "-C link-arg=-Wl,--x#y")


class LiveWorkflows(unittest.TestCase):
    """The committed workflows must satisfy the gate right now."""

    def setUp(self):
        self.missing, self.values = gate.scan_workflows(REPO_ROOT)

    def test_every_coverage_job_sets_job_level_rustflags(self):
        self.assertEqual(
            self.missing,
            [],
            "cargo-llvm-cov job(s) with no job-level RUSTFLAGS — the workflow-level "
            "CARGO_TARGET_<triple>_RUSTFLAGS is shadowed there (sq-6vshe.11): "
            f"{self.missing}",
        )

    def test_all_coverage_jobs_agree_on_the_value(self):
        distinct = set(self.values.values())
        self.assertLessEqual(
            len(distinct),
            1,
            "coverage jobs disagree on RUSTFLAGS; the engine split's cross-runner "
            f".profraw merge requires identical compile inputs: {self.values}",
        )

    def test_not_vacuous(self):
        # If this ever fails, the detector stopped seeing the coverage jobs and layers 1-2
        # became decorative.
        self.assertGreaterEqual(
            len(self.values),
            1,
            "no cargo-llvm-cov jobs detected in .github/workflows — the gate would pass "
            "vacuously; fix the detector rather than deleting this assertion.",
        )

    def test_the_engine_split_pair_is_covered(self):
        # These two are the pair whose RUSTFLAGS MUST match for the .profraw merge to be
        # valid, so their presence in the scanned set is itself part of the contract.
        jobs = {key.split(":", 1)[1] for key in self.values}
        for job in ("coverage-engine-run", "coverage-engine-merge"):
            self.assertIn(job, jobs, f"`{job}` is not being checked by the gate")


if __name__ == "__main__":
    unittest.main(verbosity=2)
