#!/usr/bin/env python3
# [OPUS-5] 🤖 SPARQ agent — hosted-runner reservation for the scheduled safety lanes
# (sparq-org/sparq#6349).
#
# THE DEFECT THIS PINS. `miri.yml` and `kani.yml` are informational NIGHTLY lanes, but each
# used to admit its ENTIRE matrix to GitHub-hosted runners at once: 24 Miri shards + the
# doctest job, and every Kani suite. Their jobs are long-lived by construction (a 130-minute
# Miri shard backstop, a 90-minute Kani suite backstop), so a scheduled burst parks most of
# the account's standard-runner admissions for hours. During the PR-throughput incident that
# filed #6349, current PR-head checks queued with GitHub reporting they were waiting for a
# runner while those two lanes held the pool.
#
# WHY A PARALLELISM BOUND AND NOT A CRON OFFSET. The two lanes are already offset (05:11 /
# 06:11 UTC), and that is not a reservation: a delayed or long run simply overlaps the next
# lane's window, so the worst case is still both full matrices resident at once. Only a
# `max-parallel` bound on each strategy caps ADMISSIONS regardless of when a run actually
# starts. Coverage is untouched — `max-parallel` throttles admission, it does not drop legs;
# every shard and every suite still runs, with its own backstop and its own verdict.
#
# WHAT IS MACHINE-CHECKED HERE (the invariant #6349 asks for — it reads BOTH workflow files):
#   R1  each matrix job that expands to more than one leg DECLARES an explicit positive
#       integer `max-parallel` — deleting the cap reds here instead of silently restoring
#       the burst.
#   R2  neither lane's own admission footprint exceeds PER_WORKFLOW_CEILING, so one lane
#       cannot eat the whole scheduled-safety-lane allotment on its own.
#   R3  the COMBINED footprint of both lanes stays within SCHEDULED_SAFETY_LANE_CEILING, so
#       the documented majority of the allowance stays reserved for PR / merge-group checks.
#       R3 is not implied by R2: two lanes each legal under R2 (6 + 6) exceed R3 (8), and
#       `test_control_*` below proves that mutation reds on R3 alone.
#   R4  the properties the cap must not be paid for with: `fail-fast: false` survives in both
#       matrix jobs, and Miri's shard list still covers exactly the `--partition count:k/N`
#       denominator the steps use (a capped lane must not be "sped up" by deleting shards).
#   R5  neither lane grows a `pull_request` / `merge_group` / `push` trigger. The whole
#       reservation argument is "these are SCHEDULED lanes, so throttling them costs no PR
#       latency" — a PR-head trigger would make the cap throttle the critical path instead.
#
# A job's admission footprint is `min(max-parallel, legs)` for a matrix job and 1 for a plain
# job — the Miri doctest job is a real concurrent runner claim and is counted as one.
#
# HOW THIS SUITE IS BUILT TO FAIL. The rule engine is ONE function, `audit()`, and the same
# function judges the real tree and the hermetic fixtures — so a broken rule cannot pass the
# fixtures and quietly green the live files. Every fixture case asserts on WHICH rule fired,
# not merely that something did, and `TestMutationOfTheRealFiles` mutates the REAL parsed
# workflows (cap deleted, cap raised) and requires a finding: if that ever goes clean, the
# live-tree test below is measuring nothing.
#
# Stdlib + PyYAML (already installed in the docs-quality job that runs this). Run:
#   python3 scripts/tests/test_nightly_runner_reservation.py

from __future__ import annotations

import copy
import re
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
MIRI_YML = WORKFLOWS / "miri.yml"
KANI_YML = WORKFLOWS / "kani.yml"

# ---------------------------------------------------------------------------
# The documented reservation.
#
# HOSTED_RUNNER_ALLOWANCE is GitHub's documented concurrent-job limit for standard
# hosted runners on the FREE plan (20). It is deliberately the conservative FLOOR for
# this account: if the account is on a larger plan the real allowance is bigger, which
# only makes the reservation below more generous — never less. It is not read from the
# API, because a gate that needs a token to know its own budget is not a gate.
#
# Of that allowance, the two scheduled safety lanes may together hold at most
# SCHEDULED_SAFETY_LANE_CEILING admissions, leaving RESERVED_FOR_CURRENT_WORK for the
# merge critical path — current PR heads and merge-group refs. That reserved share must be
# a strict MAJORITY of the allowance; `test_the_reservation_is_a_documented_majority`
# pins the arithmetic so a future edit cannot raise the ceiling past a minority split
# without the constant relation going red.
# ---------------------------------------------------------------------------
HOSTED_RUNNER_ALLOWANCE = 20
SCHEDULED_SAFETY_LANE_CEILING = 8
RESERVED_FOR_CURRENT_WORK = HOSTED_RUNNER_ALLOWANCE - SCHEDULED_SAFETY_LANE_CEILING
# No single scheduled lane may claim more than this on its own, so one lane's burst cannot
# consume the whole shared ceiling and starve the other of its nightly verdict.
PER_WORKFLOW_CEILING = 6

# The lanes under reservation: file -> the matrix jobs whose parallelism must be bounded.
SCHEDULED_SAFETY_LANES = {"miri.yml": "miri", "kani.yml": "kani"}

# Triggers that would put a lane on the merge critical path (R5).
CRITICAL_PATH_TRIGGERS = ("pull_request", "pull_request_target", "merge_group", "push")

# `--partition count:${{ matrix.shard }}/24` — the interpolation contains spaces, so the
# numerator is matched as "anything up to the slash on this line".
_PARTITION_DENOMINATOR = re.compile(r"--partition\s+count:[^/\n]+?/(\d+)")


def load_workflow(path: Path) -> dict:
    with open(path, encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def triggers(doc: dict) -> dict:
    """The `on:` block. PyYAML resolves the bare key `on` to the boolean True under the
    YAML 1.1 rules it implements, so both spellings have to be accepted."""
    block = doc.get("on", doc.get(True))
    return block if isinstance(block, dict) else {}


def matrix_legs(strategy: dict) -> int | None:
    """How many jobs this strategy expands to, or None when it cannot be counted
    statically (a `fromJSON(...)` matrix). None means "treat as unbounded"."""
    matrix = strategy.get("matrix")
    if not isinstance(matrix, dict):
        return None
    legs = 1
    for key, value in matrix.items():
        if key in ("include", "exclude"):
            continue
        if not isinstance(value, list):
            return None
        legs *= len(value)
    include = matrix.get("include")
    if isinstance(include, list):
        # `include` entries that match no axis add whole legs; entries that only extend an
        # existing leg do not. Counting them all is the conservative direction (it can only
        # over-estimate the burst), which is the right bias for a capacity guard.
        legs += len(include)
    return legs


def job_admissions(job: dict) -> tuple[int, str | None]:
    """-> (concurrent runner claims, R1 problem or None) for one job."""
    strategy = job.get("strategy") or {}
    if "matrix" not in strategy:
        return 1, None
    legs = matrix_legs(strategy)
    cap = strategy.get("max-parallel")
    if cap is None:
        # No cap: the whole matrix is admitted at once (unbounded if it is not countable).
        return (legs if legs is not None else HOSTED_RUNNER_ALLOWANCE), "missing"
    if not isinstance(cap, int) or isinstance(cap, bool) or cap < 1:
        return HOSTED_RUNNER_ALLOWANCE, "invalid"
    return (min(cap, legs) if legs is not None else cap), None


def audit(docs: dict[str, dict]) -> list[str]:
    """The whole rule engine. `docs` maps workflow filename -> parsed YAML.
    -> a list of findings; empty means the reservation holds."""
    problems: list[str] = []
    combined = 0

    for filename, matrix_job in sorted(SCHEDULED_SAFETY_LANES.items()):
        doc = docs.get(filename)
        if not isinstance(doc, dict) or not isinstance(doc.get("jobs"), dict):
            problems.append(f"R1: {filename}: cannot read a `jobs:` block")
            continue
        jobs = doc["jobs"]
        if matrix_job not in jobs:
            problems.append(
                f"R1: {filename}: job {matrix_job!r} is gone — the reservation is pinned "
                f"per job, so re-point this gate at whatever replaced it"
            )
            continue

        total = 0
        for name, job in jobs.items():
            if not isinstance(job, dict):
                continue
            admissions, defect = job_admissions(job)
            total += admissions
            if defect == "missing":
                problems.append(
                    f"R1: {filename}: job {name!r} runs a matrix with no `max-parallel` — "
                    f"the whole matrix is admitted at once ({admissions} concurrent hosted "
                    f"runners), which is the burst #6349 removed. Declare an explicit cap."
                )
            elif defect == "invalid":
                problems.append(
                    f"R1: {filename}: job {name!r} has a non-integer/non-positive "
                    f"`max-parallel` ({job.get('strategy', {}).get('max-parallel')!r}) — "
                    f"GitHub ignores it, so the matrix is admitted at once."
                )
        combined += total

        if total > PER_WORKFLOW_CEILING:
            problems.append(
                f"R2: {filename} claims up to {total} concurrent hosted runners, over the "
                f"{PER_WORKFLOW_CEILING}-runner per-lane ceiling — one scheduled safety "
                f"lane must not be able to consume the whole shared allotment."
            )

        # R4 — the cap must not be paid for by weakening the matrix.
        strategy = jobs[matrix_job].get("strategy") or {}
        if strategy.get("fail-fast") is not False:
            problems.append(
                f"R4: {filename}: jobs.{matrix_job}.strategy.fail-fast is "
                f"{strategy.get('fail-fast')!r}, not false — with parallelism bounded, one "
                f"leg's failure would cancel the queued legs and destroy their verdicts."
            )

    if combined > SCHEDULED_SAFETY_LANE_CEILING:
        problems.append(
            f"R3: the scheduled safety lanes ({', '.join(sorted(SCHEDULED_SAFETY_LANES))}) "
            f"together claim up to {combined} of the {HOSTED_RUNNER_ALLOWANCE}-runner "
            f"allowance, over the {SCHEDULED_SAFETY_LANE_CEILING} they are budgeted — that "
            f"eats into the {RESERVED_FOR_CURRENT_WORK} admissions reserved for current PR "
            f"and merge-group checks (#6349). Lower a `max-parallel`, do not raise this."
        )

    # R5 — the reservation argument only holds for lanes that are OFF the PR critical path.
    for filename, doc in sorted(docs.items()):
        if filename not in SCHEDULED_SAFETY_LANES or not isinstance(doc, dict):
            continue
        on_critical = [t for t in CRITICAL_PATH_TRIGGERS if t in triggers(doc)]
        if on_critical:
            problems.append(
                f"R5: {filename} now triggers on {on_critical} — capping its parallelism "
                f"would throttle the merge critical path instead of protecting it. Keep "
                f"this lane scheduled/dispatch-only, or re-derive the budget."
            )

    return problems


def shard_coverage_problems(doc: dict, text: str) -> list[str]:
    """R4, Miri half: the shard axis must still cover exactly the partition denominator the
    steps pass to nextest, so a capped lane cannot be "sped up" by dropping shards."""
    shards = ((doc.get("jobs", {}).get("miri", {}).get("strategy") or {})
              .get("matrix", {}).get("shard"))
    # Comment-aware: the workflow header DESCRIBES the partitioning in prose, and prose is
    # not what the runner executes.
    runnable = "\n".join(ln for ln in text.splitlines() if not ln.lstrip().startswith("#"))
    denominators = {int(d) for d in _PARTITION_DENOMINATOR.findall(runnable)}
    problems: list[str] = []
    if not isinstance(shards, list) or not shards:
        return ["R4: miri.yml has no `shard` matrix axis"]
    if not denominators:
        return ["R4: miri.yml passes no `--partition count:k/N` — re-point this check"]
    if len(denominators) > 1:
        problems.append(f"R4: miri.yml uses inconsistent partition denominators {denominators}")
    for denominator in denominators:
        if sorted(shards) != list(range(1, denominator + 1)):
            problems.append(
                f"R4: miri.yml runs shards {sorted(shards)} but partitions the test list "
                f"{denominator} ways — the union of the shards is no longer the whole test "
                f"set, so the lane's coverage silently shrank."
            )
    return problems


# ---------------------------------------------------------------------------
# Hermetic fixtures — a minimal, CONSISTENT pair of lanes that must audit clean, and one
# mutation per rule that must not.
# ---------------------------------------------------------------------------
_FIXTURE_MIRI = """\
on:
  workflow_dispatch:
  schedule:
    - cron: "11 5 * * *"
jobs:
  miri:
    strategy:
      fail-fast: false
      max-parallel: 4
      matrix:
        shard: [1, 2, 3, 4, 5, 6]
    steps:
      - run: cargo miri nextest run --partition count:1/6
  miri-doctests:
    steps:
      - run: cargo miri test --doc
"""

_FIXTURE_KANI = """\
on:
  workflow_dispatch:
  schedule:
    - cron: "11 6 * * *"
jobs:
  kani:
    strategy:
      fail-fast: false
      max-parallel: 2
      matrix:
        suite:
          - name: a
          - name: b
          - name: c
    steps:
      - run: cargo kani
"""


def fixtures(miri: str = _FIXTURE_MIRI, kani: str = _FIXTURE_KANI) -> dict[str, dict]:
    return {"miri.yml": yaml.safe_load(miri), "kani.yml": yaml.safe_load(kani)}


class TestFixtureRules(unittest.TestCase):
    """Each rule, on hermetic fixtures. Every case asserts WHICH rule fired."""

    def assertRule(self, problems: list[str], rule: str, needle: str = "") -> None:
        self.assertTrue(problems, f"expected a {rule} finding, got a clean audit")
        hits = [p for p in problems if p.startswith(f"{rule}:")]
        self.assertTrue(hits, f"no {rule} finding among {problems}")
        if needle:
            self.assertTrue(
                any(needle in p for p in hits), f"no {rule} finding mentions {needle!r}: {hits}"
            )

    def test_the_consistent_fixture_audits_clean(self) -> None:
        self.assertEqual(audit(fixtures()), [])

    def test_a_deleted_miri_cap_reds(self) -> None:
        self.assertRule(
            audit(fixtures(miri=_FIXTURE_MIRI.replace("      max-parallel: 4\n", ""))),
            "R1", "miri.yml",
        )

    def test_a_deleted_kani_cap_reds(self) -> None:
        self.assertRule(
            audit(fixtures(kani=_FIXTURE_KANI.replace("      max-parallel: 2\n", ""))),
            "R1", "kani.yml",
        )

    def test_a_zero_or_non_integer_cap_reds(self) -> None:
        # GitHub ignores a malformed `max-parallel`, so "present" is not the property.
        self.assertRule(
            audit(fixtures(kani=_FIXTURE_KANI.replace("max-parallel: 2", 'max-parallel: "all"'))),
            "R1",
        )

    def test_one_lane_over_its_own_ceiling_reds(self) -> None:
        self.assertRule(
            audit(fixtures(miri=_FIXTURE_MIRI.replace("max-parallel: 4", "max-parallel: 12"))),
            "R2", "miri.yml",
        )

    def test_control_the_combined_budget_reds_on_its_own(self) -> None:
        # ANTI-VACUITY. R3 must have teeth R2 does not: 6 (miri, at its per-lane ceiling)
        # + 3 (kani) = 9 > 8, while NEITHER lane breaches PER_WORKFLOW_CEILING. If this ever
        # produces an R2 finding, or none at all, R3 is a comment.
        problems = audit(fixtures(
            miri=_FIXTURE_MIRI.replace("max-parallel: 4", "max-parallel: 5"),  # + doctests = 6
            kani=_FIXTURE_KANI.replace("max-parallel: 2", "max-parallel: 3"),
        ))
        self.assertEqual([p.split(":", 1)[0] for p in problems], ["R3"], problems)

    def test_fail_fast_flipped_reds(self) -> None:
        self.assertRule(
            audit(fixtures(kani=_FIXTURE_KANI.replace("fail-fast: false", "fail-fast: true"))),
            "R4", "kani.yml",
        )

    def test_a_pr_head_trigger_reds(self) -> None:
        self.assertRule(
            audit(fixtures(miri=_FIXTURE_MIRI.replace(
                "  workflow_dispatch:\n", "  workflow_dispatch:\n  pull_request:\n"))),
            "R5", "miri.yml",
        )

    def test_an_uncapped_plain_job_still_counts_as_one_admission(self) -> None:
        # The Miri doctest job is a real runner claim; dropping it from the count would
        # under-report the lane's footprint by one and quietly widen the budget.
        legless = fixtures()
        self.assertEqual(job_admissions(legless["miri.yml"]["jobs"]["miri-doctests"]), (1, None))


class TestTheRealWorkflows(unittest.TestCase):
    """The live tree must satisfy the same function the fixtures exercise."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.docs = {"miri.yml": load_workflow(MIRI_YML), "kani.yml": load_workflow(KANI_YML)}

    def test_the_live_lanes_audit_clean(self) -> None:
        self.assertEqual(audit(copy.deepcopy(self.docs)), [])

    def test_both_lanes_declare_a_bounded_cap(self) -> None:
        for filename, job in sorted(SCHEDULED_SAFETY_LANES.items()):
            strategy = self.docs[filename]["jobs"][job]["strategy"]
            cap = strategy.get("max-parallel")
            self.assertIsInstance(cap, int, f"{filename}: jobs.{job} declares no integer cap")
            self.assertGreaterEqual(cap, 1, f"{filename}: jobs.{job} cap must admit a leg")
            self.assertLess(
                cap, matrix_legs(strategy),
                f"{filename}: jobs.{job}'s cap does not bound its matrix — it admits every "
                f"leg at once, which is the burst #6349 removed.",
            )

    def test_the_combined_footprint_leaves_the_reserved_majority(self) -> None:
        combined = sum(
            job_admissions(job)[0]
            for doc in self.docs.values()
            for job in doc["jobs"].values()
        )
        self.assertLessEqual(
            combined, SCHEDULED_SAFETY_LANE_CEILING,
            f"the scheduled safety lanes claim {combined} of {HOSTED_RUNNER_ALLOWANCE} "
            f"admissions; at most {SCHEDULED_SAFETY_LANE_CEILING} is budgeted.",
        )

    def test_the_reservation_is_a_documented_majority(self) -> None:
        # #6349 asks for a documented MAJORITY, not merely "some" headroom.
        self.assertGreater(
            RESERVED_FOR_CURRENT_WORK, HOSTED_RUNNER_ALLOWANCE // 2,
            f"only {RESERVED_FOR_CURRENT_WORK} of {HOSTED_RUNNER_ALLOWANCE} admissions are "
            f"left for PR and merge-group checks — that is not a majority.",
        )

    def test_miri_still_runs_every_shard_of_its_partition(self) -> None:
        self.assertEqual(
            shard_coverage_problems(self.docs["miri.yml"], MIRI_YML.read_text()), []
        )

    def test_kani_still_runs_every_suite(self) -> None:
        # Coverage completeness for Kani is owned by D3 in scripts/check-fv-manifest.py
        # (matrix == the manifest's nightly suites). Pin here only that the cap did not
        # arrive alongside a shrunken matrix, and that that gate still exists to be relied on.
        suites = self.docs["kani.yml"]["jobs"]["kani"]["strategy"]["matrix"]["suite"]
        self.assertGreaterEqual(len(suites), 6, "the kani suite matrix shrank")
        self.assertTrue(
            (REPO_ROOT / "scripts" / "check-fv-manifest.py").exists(),
            "check-fv-manifest.py is the gate that pins the kani matrix against the "
            "manifest; without it this test is the only thing watching suite coverage.",
        )


class TestMutationOfTheRealFiles(unittest.TestCase):
    """Mutate the REAL parsed workflows: if these go clean, the live-tree test is vacuous."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.docs = {"miri.yml": load_workflow(MIRI_YML), "kani.yml": load_workflow(KANI_YML)}

    def test_deleting_the_real_miri_cap_reds(self) -> None:
        docs = copy.deepcopy(self.docs)
        del docs["miri.yml"]["jobs"]["miri"]["strategy"]["max-parallel"]
        self.assertTrue(any(p.startswith("R1:") for p in audit(docs)))

    def test_deleting_the_real_kani_cap_reds(self) -> None:
        docs = copy.deepcopy(self.docs)
        del docs["kani.yml"]["jobs"]["kani"]["strategy"]["max-parallel"]
        self.assertTrue(any(p.startswith("R1:") for p in audit(docs)))

    def test_raising_both_real_caps_past_the_budget_reds(self) -> None:
        docs = copy.deepcopy(self.docs)
        docs["miri.yml"]["jobs"]["miri"]["strategy"]["max-parallel"] = 5
        docs["kani.yml"]["jobs"]["kani"]["strategy"]["max-parallel"] = 3
        self.assertTrue(any(p.startswith("R3:") for p in audit(docs)))


class TestSuiteIsWiredIntoCi(unittest.TestCase):
    """A structural test that never runs is a comment. Pin its own call site."""

    def test_docs_quality_invokes_this_suite(self) -> None:
        self.assertIn(
            "scripts/tests/test_nightly_runner_reservation.py",
            (WORKFLOWS / "docs-quality.yml").read_text(),
            "this suite must be invoked by docs-quality.yml or it silently stops gating.",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
