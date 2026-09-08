Root corroboration added after the author froze this packet: I read existing parse job101671686325 from scheduled run34099890455. Its installation log prints cargo-mutants25.3.1 and the completed measurement reports102 mutants tested:15missed,74caught,1unviable,12timeouts, exactly matching the original artifact. No new measurement was run, and no actual process exit code is inferred. The packet statement that no log was read describes the author evidence at freeze; this root read is additional evidence. Current source remains unchanged.

# Issue6436: focused re-review after actual Opus request_changes

Assess this exact GPT-6 Astra remediation against the original reviewed whole change. The frozen prior packet/head remain unchanged. B1 is answered with source and real artifact counterevidence, not a relaxed guard. B2/B3 have narrow changes. No new review verdict is implied.

## Dispositions and validation

```json
{
  "issue": 6436,
  "implementation_model": "OpenAI GPT-6 Astra, xhigh runtime",
  "head": "0fc2faf6d4255e9e27b8a17bc43e8ba848c29368",
  "prior_reviewed_head": "2f3d1773bb3cebd36186692025912a8377c85db6",
  "base": "e44b08c59aea0755cfda1cbf633e25971db787f7",
  "prior_packet_sha256": "5fcca5c091120c1be3987e4c0ca185d0f2103a75c9ad09119b7be65a76019843",
  "delta_stat": "scripts/ci_nightly_freshness.py                    |  15 ++-\n .../mutants-engine-10036242958.zip                 | Bin 0 -> 29016 bytes\n .../mutants-parse-10010502882.zip                  | Bin 0 -> 9720 bytes\n .../fixtures/ci-nightly-freshness/provenance.json  |  31 ++++++\n scripts/tests/test_ci_nightly_freshness.py         | 115 ++++++++++++++++++++-\n 5 files changed, 157 insertions(+), 4 deletions(-)",
  "clean_worktree": true,
  "B1": {
    "disposition": "Premise refuted; strict runtime completion arithmetic unchanged. Added genuine original unsharded and sharded fixtures, bounded ZIP reads, SHA pins, baseline/count assertions and partial-plan/success-counter negatives.",
    "pinned_source_counterevidence": "cargo-mutants v25.3.1 outcome.rs increments all aggregate counters only inside scenario.is_mutant(); baseline appends an outcome row but leaves top-level success0. main.rs applies shard.select before calling test_mutants; lab.rs writes that selected list before baseline/mutation testing. Real artifacts independently demonstrate both shapes.",
    "limits": "Captured artifacts are from existing run34099890455, not a newly executed cargo-mutants measurement. Accepted exit3 is a hermetic test input; no observed process exit is asserted and no log was read."
  },
  "B2": {
    "disposition": "Added conservative full-workflow/local-reusable inventory capacity guard with20spare slots and ordinary-matrix growth/dynamic-matrix negative controls. Runtime 2x100 cap unchanged.",
    "counts": "Real complete page:85jobs including all51mutation names and coverage. Conservative source bound101 (conditions/excludes ignored, includes may overcount enrichment), leaving99slots within200."
  },
  "B3": {
    "disposition": "Known terminal conclusions now support readable incomplete evidence. Empty job inventory is incomplete. Omitted steps provide no completion proof. Attempt-scoped endpoint binds omitted run_attempt; present null/bool/nonpositive/different values still reject.",
    "preserved_fail_closed": "API/quota/timeout/JSON errors, malformed/truncated history/census/jobs/steps, unknown conclusions and contradictory identities still block immediately without retries. Explicit steps:null remains malformed, not equivalent to omission.",
    "attempt_contract": "Official attempt endpoint binds run_id + attempt_number; its published example omits run_attempt. No filter=latest/all is added (that parameter belongs the other endpoint). Real run1 inventory includes consistent attempt1. No live partial-rerun sample was obtained; the speculation that mismatching older attempts should be accepted is not adopted.",
    "availability": "Known readable incomplete work is eligible at the next existing cron. API error is not converted into permission to spend. A later successful read can recover; skipped-only history whose proof ages out gets the existing bounded active census and periodic measurement. No permanent/manual-only same-head hold or new immediate retry mechanism."
  },
  "sources": [
    "https://github.com/sourcefrog/cargo-mutants/blob/v25.3.1/src/outcome.rs",
    "https://github.com/sourcefrog/cargo-mutants/blob/v25.3.1/src/main.rs",
    "https://github.com/sourcefrog/cargo-mutants/blob/v25.3.1/src/lab.rs",
    "https://docs.github.com/en/rest/actions/workflow-jobs#list-jobs-for-a-workflow-run-attempt",
    "https://docs.github.com/en/rest/actions/workflow-runs#list-workflow-runs"
  ],
  "validation": {
    "focused_tests": 47,
    "focused_result": "PASS",
    "selection_wiring_tests": 88,
    "selection_wiring_result": "PASS",
    "runner_reservation_tests": 19,
    "runner_reservation_result": "PASS",
    "actionlint": "PASS for ci.yml and docs-quality.yml",
    "diff_check": "PASS",
    "coverage": {
      "source": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6436/scripts/ci_nightly_freshness.py",
      "executable_lines": 196,
      "covered_lines": 195,
      "never_executed_lines": [
        259
      ],
      "calibration": {
        "never_called_body_executed": false,
        "called_body_executed": true
      },
      "limitation": "Parent-process trace only. The __main__ entry executes in producer subprocesses but is not counted in the parent trace. Python3.14 synthetic line0/None entries are excluded."
    },
    "mutation_controls": {
      "baseline": {
        "name": "baseline",
        "classification": "PASS",
        "exit_code": 0,
        "tests": 47,
        "failures": 0,
        "errors": 0,
        "mutation": null
      },
      "counts": {
        "KILLED": 15,
        "SURVIVED": 0,
        "INVALID_OR_ERROR": 0
      },
      "names": [
        "active_census_guard_deleted",
        "baseline_wrongly_counted_as_mutant",
        "capacity_guard_deleted",
        "capacity_guard_inert",
        "completed_advisory_rejected",
        "contradictory_attempt_accepted",
        "empty_jobs_rejected",
        "known_conclusions_rejected",
        "missing_steps_false_proof",
        "missing_steps_rejected",
        "optional_attempt_required",
        "planned_equality_deleted",
        "planned_equality_inert",
        "quota_error_admits",
        "skipped_is_completion"
      ]
    },
    "preflight": "Only failure: macOS Bash3.2 has no mapfile, scripts/check-privacy-claims.sh:92. Final staged whole-change preflight otherwise ran G1/G2/G6/guard-untested; no gate weakened.",
    "harness_calibration": "Initial copied harness accidentally passed a workflow path as a unittest name; it ran 1 error test, failed its baseline assertion and ran no mutants. Corrected launcher baseline ran all47; all15 subsequent controls ran47 with zero errors. The rejected attempt is not mutation evidence."
  },
  "composition": {
    "head": "0fc2faf6d4255e9e27b8a17bc43e8ba848c29368",
    "main": "48fefb6c3f025cc3bf5a104cfb5e93e61ecd3045",
    "merge_tree": "434c21a7f4c885a28fc7a746bf0a876017360f14",
    "merge_exit_code": 0,
    "working_tree_switched": false
  },
  "invariants": "Schedule/manual boundaries,51heavy legs, advisory/ratchet semantics, max-parallel8, timeouts, permissions and no heavy merge_group work are unchanged. Max26 read-only GET subprocesses,30s each; stop on first known failure; no application retries. No remote mutations/model calls/install/build/EC2/registry/approval operations. Root owns publication and actual Opus rereview.",
  "recovery_provenance": "Original uncommitted pre-usage-limit files were lost with /private/tmp;9a8351 reconstructed them from own retained inputs, not byte-verified recovery. This follow-up is an exact delta from retained Git head2f3d1773; real fixtures are exact downloaded bytes, separately hash pinned."
}
```

## Exact delta from2f3d1773 (binary ZIP bytes supplied separately with SHA pins)

```diff
diff --git a/scripts/ci_nightly_freshness.py b/scripts/ci_nightly_freshness.py
index 278aa2451..58fbcdb6a 100644
--- a/scripts/ci_nightly_freshness.py
+++ b/scripts/ci_nightly_freshness.py
@@ -29,6 +29,8 @@ MUTATION = "mutation ratchet (cargo-mutants, advisory)"
 MUTATION_STEP = "Nightly mutation work completed"
 # [GPT-6 Astra] All non-completed statuses supported by the workflow-runs API.
 ACTIVE_STATUSES = ("queued", "in_progress", "waiting", "requested", "pending")
+COMPLETED_CONCLUSIONS = ("success", "failure", "cancelled", "timed_out", "startup_failure",
+                         "stale", "neutral", "action_required", "skipped")
 
 
 class EvidenceError(Exception):
@@ -122,7 +124,7 @@ def read_jobs(get, repo, run):
         if not isinstance(doc, dict) or type(doc.get("total_count")) is not int:
             raise EvidenceError("missing job inventory")
         total, batch = doc["total_count"], doc.get("jobs")
-        if (total <= 0 or total > JOB_PAGE_SIZE * JOB_PAGE_LIMIT
+        if (total < 0 or total > JOB_PAGE_SIZE * JOB_PAGE_LIMIT
                 or not isinstance(batch, list) or len(batch) > JOB_PAGE_SIZE
                 or (expected is not None and total != expected)):
             raise EvidenceError("unbounded or inconsistent job inventory")
@@ -136,7 +138,10 @@ def read_jobs(get, repo, run):
     for job in jobs:
         if (not isinstance(job, dict) or not positive_int(job.get("id"))
                 or job["id"] in ids or job.get("run_id") != run["id"]
-                or job.get("run_attempt") != run["run_attempt"]
+                # [GPT-6 Astra] The attempt-scoped endpoint supplies the binding
+                # when this optional field is absent. A contradictory value does not.
+                or ("run_attempt" in job and (not positive_int(job["run_attempt"])
+                                              or job["run_attempt"] != run["run_attempt"]))
                 or job.get("head_sha") != run["head_sha"]):
             raise EvidenceError("job identity mismatch")
         ids.add(job["id"])
@@ -144,6 +149,10 @@ def read_jobs(get, repo, run):
 
 
 def successful_step(job, name):
+    # [GPT-6 Astra] The API may omit optional steps: readable absence is no proof.
+    # Null/non-array/malformed values are still unreadable evidence, not admission.
+    if "steps" not in job:
+        return False
     steps = job.get("steps")
     if not isinstance(steps, list) or any(not isinstance(s, dict) for s in steps):
         raise EvidenceError("heavy step inventory unreadable")
@@ -207,7 +216,7 @@ def decide(event, repo, head, current_id, get=gh_json):
     if any(r.get("status") != "completed" for r in prior):
         raise EvidenceError("same-head schedule still active or status unreadable")
     for run in prior:
-        if run.get("conclusion") not in ("success", "failure", "cancelled", "timed_out"):
+        if run.get("conclusion") not in COMPLETED_CONCLUSIONS:
             raise EvidenceError("scheduled conclusion unreadable or unsupported")
         state = heavy_state(read_jobs(get, repo, run))
         if state == "complete":
diff --git a/scripts/tests/fixtures/ci-nightly-freshness/mutants-engine-10036242958.zip b/scripts/tests/fixtures/ci-nightly-freshness/mutants-engine-10036242958.zip
new file mode 100644
index 000000000..cabdaabc9
Binary files /dev/null and b/scripts/tests/fixtures/ci-nightly-freshness/mutants-engine-10036242958.zip differ
diff --git a/scripts/tests/fixtures/ci-nightly-freshness/mutants-parse-10010502882.zip b/scripts/tests/fixtures/ci-nightly-freshness/mutants-parse-10010502882.zip
new file mode 100644
index 000000000..4b6eb5a63
Binary files /dev/null and b/scripts/tests/fixtures/ci-nightly-freshness/mutants-parse-10010502882.zip differ
diff --git a/scripts/tests/fixtures/ci-nightly-freshness/provenance.json b/scripts/tests/fixtures/ci-nightly-freshness/provenance.json
new file mode 100644
index 000000000..4bfee1e05
--- /dev/null
+++ b/scripts/tests/fixtures/ci-nightly-freshness/provenance.json
@@ -0,0 +1,31 @@
+{
+  "_provenance": "[GPT-6 Astra] Original GitHub Actions artifact ZIPs, downloaded read-only by root; not synthetic fixtures or new measurements. No logs used to infer an observed process exit.",
+  "repository": "sparq-org/sparq",
+  "run_id": 34099890455,
+  "run_attempt": 1,
+  "head_sha": "4d880d6a4a37b7d28a4259dfdbceea4a8db07eb0",
+  "workflow": ".github/workflows/ci.yml",
+  "cargo_mutants_version": "25.3.1 (pinned by workflow at the recorded head)",
+  "fixtures": [
+    {
+      "file": "mutants-parse-10010502882.zip",
+      "artifact_id": 10010502882,
+      "sha256": "7d60fdfa6ce36bdf3f5d5105c476ebf7fa61c273a5735f53b15e7b0eeea0aa63",
+      "planned_and_completed": 102,
+      "shard": null
+    },
+    {
+      "file": "mutants-engine-10036242958.zip",
+      "artifact_id": 10036242958,
+      "sha256": "77bc3dd50465d6c7987141b49eb9d4b656add76129ca0e79ac4734f597673752",
+      "planned_and_completed": 197,
+      "shard": "23/24"
+    }
+  ],
+  "sources": [
+    "https://github.com/sparq-org/sparq/actions/runs/34099890455",
+    "https://github.com/sourcefrog/cargo-mutants/blob/v25.3.1/src/outcome.rs",
+    "https://github.com/sourcefrog/cargo-mutants/blob/v25.3.1/src/main.rs",
+    "https://github.com/sourcefrog/cargo-mutants/blob/v25.3.1/src/lab.rs"
+  ]
+}
diff --git a/scripts/tests/test_ci_nightly_freshness.py b/scripts/tests/test_ci_nightly_freshness.py
index 47e8a1b13..96e2e738f 100644
--- a/scripts/tests/test_ci_nightly_freshness.py
+++ b/scripts/tests/test_ci_nightly_freshness.py
@@ -4,14 +4,17 @@
 from __future__ import annotations
 
 import copy
+import hashlib
 import importlib.util
 import json
+import math
 import os
 from pathlib import Path
 import subprocess
 import sys
 import tempfile
 import unittest
+import zipfile
 from unittest.mock import patch
 from urllib.parse import parse_qs, urlsplit
 
@@ -26,6 +29,38 @@ HEAD = "a" * 40
 CURRENT = 1000
 
 
+def workflow_job_bound(workflow, seen=()):
+    """[GPT-6 Astra] Conservative full inventory, including local reusable jobs.
+
+    Ignore job conditions/excludes; count each include as an extra even when it
+    merely enriches an existing leg. This overcounts rather than understating
+    API inventory. Unknown dynamic/remote expansion requires an explicit review.
+    """
+    total = 0
+    for item in workflow["jobs"].values():
+        matrix = item.get("strategy", {}).get("matrix", {})
+        assert isinstance(matrix, dict), "dynamic matrix needs a reviewed inventory bound"
+        axes = [value for key, value in matrix.items() if key not in ("include", "exclude")]
+        assert all(isinstance(axis, list) and axis for axis in axes), "unbounded matrix axis"
+        includes = matrix.get("include", [])
+        assert isinstance(includes, list) and all(isinstance(x, dict) for x in includes)
+        count = (math.prod(map(len, axes)) if axes else int(not includes)) + len(includes)
+        if "uses" in item:
+            path = item["uses"]
+            assert path.startswith("./.github/workflows/") and path not in seen, path
+            child = yaml.safe_load((ROOT / path).read_text())
+            count *= workflow_job_bound(child, (*seen, path))
+        total += count
+    return total
+
+
+def assert_job_capacity(workflow):
+    bound = workflow_job_bound(workflow)
+    capacity = freshness.JOB_PAGE_SIZE * freshness.JOB_PAGE_LIMIT
+    assert bound + 20 <= capacity, f"full workflow bound {bound} needs 20 spare slots within {capacity}"
+    return bound
+
+
 def run(run_id=999, **changes):
     item = dict(id=run_id, run_attempt=1, event="schedule", head_sha=HEAD,
                 head_branch="main", path=".github/workflows/ci.yml",
@@ -92,6 +127,12 @@ class Admission(unittest.TestCase):
         with self.assertRaisesRegex(freshness.EvidenceError, pattern or "."):
             self.decide(api)
 
+    def readable_decision(self, api):
+        try:
+            return self.decide(api)
+        except freshness.EvidenceError as exc:
+            self.fail(f"readable fixture rejected: {exc}")
+
     def test_first_schedule_at_new_head_runs_once(self):
         for history in [[], [run(CURRENT, status="in_progress", conclusion=None)]]:
             with self.subTest(history=history):
@@ -115,6 +156,17 @@ class Admission(unittest.TestCase):
                 self.assertTrue(self.decide(api)[0])
                 self.assertEqual(len(api.calls), 2)
 
+    def test_known_terminal_conclusions_with_zero_jobs_are_readable_incomplete(self):
+        for conclusion in ["success", "failure", "cancelled", "timed_out", "startup_failure",
+                           "stale", "neutral", "action_required", "skipped"]:
+            with self.subTest(conclusion=conclusion):
+                api = FakeAPI([run(conclusion=conclusion)], {999: []})
+                self.assertTrue(self.readable_decision(api)[0])
+                self.assertEqual(len(api.calls), 2)
+        api = FakeAPI([run(conclusion="future_unknown")], {999: []})
+        self.blocked(api, "conclusion")
+        self.assertEqual(len(api.calls), 1)
+
     def test_active_or_unreadable_same_head_never_admits(self):
         for status, conclusion in [("in_progress", None), ("queued", None), ("completed", None)]:
             with self.subTest(status=status, conclusion=conclusion):
@@ -188,16 +240,37 @@ class Admission(unittest.TestCase):
                 else:
                     self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])
 
+    def test_omitted_optional_steps_are_incomplete_not_completion(self):
+        for index in [0, 1]:
+            with self.subTest(index=index):
+                jobs = completed_jobs()
+                del jobs[index]["steps"]
+                api = FakeAPI([run()], {999: jobs})
+                self.assertTrue(self.readable_decision(api)[0])
+                self.assertEqual(len(api.calls), 2)
+
     def test_attempt_scoped_job_lookup_and_mismatched_attempt_blocks(self):
         jobs = completed_jobs()
         for item in jobs:
             item["run_attempt"] = 2
         api = FakeAPI([run(run_attempt=2)], {999: jobs})
-        self.assertFalse(self.decide(api)[0])
+        self.assertFalse(self.readable_decision(api)[0])
         self.assertIn("/attempts/2/jobs?", api.calls[-1])
         jobs[0]["run_attempt"] = 1
         self.blocked(FakeAPI([run(run_attempt=2)], {999: jobs}), "identity")
 
+    def test_attempt_endpoint_binds_omitted_optional_field_but_rejects_contradictions(self):
+        jobs = completed_jobs()
+        for item in jobs:
+            del item["run_attempt"]
+        api = FakeAPI([run(run_attempt=2)], {999: jobs})
+        self.assertFalse(self.readable_decision(api)[0])
+        self.assertIn("/attempts/2/jobs?", api.calls[-1])
+        for attempt in [None, True, 0, 1, 3]:
+            with self.subTest(attempt=attempt):
+                jobs[0]["run_attempt"] = attempt
+                self.blocked(FakeAPI([run(run_attempt=2)], {999: jobs}), "identity")
+
     def test_each_heavy_leg_and_marker_is_required(self):
         for index in range(len(completed_jobs())):
             for change in ["absent", "failure", "cancelled", "skipped", "missing_step", "failed_step"]:
@@ -412,6 +485,19 @@ class ProductionWiring(unittest.TestCase):
         self.assertEqual(coverage["name"], freshness.COVERAGE)
         self.assertEqual(sum(s.get("name") == freshness.COVERAGE_STEP for s in coverage["steps"]), 1)
 
+    def test_full_workflow_inventory_fits_bounded_job_reads_with_headroom(self):
+        # Unlike the 51-heavy-leg assertion, includes every job and every matrix.
+        assert_job_capacity(self.workflow)
+
+    def test_inventory_guard_catches_growth_outside_the_heavy_matrix(self):
+        workflow = copy.deepcopy(self.workflow)
+        workflow["jobs"]["test"]["strategy"]["matrix"] = {"shard": list(range(201))}
+        with self.assertRaisesRegex(AssertionError, "spare slots"):
+            assert_job_capacity(workflow)
+        workflow["jobs"]["test"]["strategy"]["matrix"] = "${{ fromJSON(needs.dynamic.outputs.matrix) }}"
+        with self.assertRaisesRegex(AssertionError, "dynamic matrix"):
+            assert_job_capacity(workflow)
+
     def test_marker_requires_unmasked_success_and_ci_runs_suite(self):
         marker = next(s for s in self.mutation["steps"] if s.get("name") == freshness.MUTATION_STEP)
         self.assertEqual(marker["if"], "${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'true' }}")
@@ -506,6 +592,33 @@ class MutationCompletion(unittest.TestCase):
             with self.subTest(exit_code=exit_code):
                 self.assertFalse(self.check(doc, [{}] * 10, exit_code))
 
+    def test_real_pinned_unsharded_and_sharded_artifacts_prove_completion(self):
+        fixtures = ROOT / "scripts/tests/fixtures/ci-nightly-freshness"
+        provenance = json.loads((fixtures / "provenance.json").read_text())
+        for fixture in provenance["fixtures"]:
+            with self.subTest(artifact=fixture["artifact_id"]), tempfile.TemporaryDirectory() as directory:
+                archive = fixtures / fixture["file"]
+                self.assertLess(archive.stat().st_size, 50000)
+                self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), fixture["sha256"])
+                with zipfile.ZipFile(archive) as bundle:
+                    self.assertEqual(sorted(bundle.namelist()), ["mutants.json", "outcomes.json"])
+                    for name in ["mutants.json", "outcomes.json"]:
+                        self.assertLess(bundle.getinfo(name).file_size, 400000)
+                        # Fixed member names only; never extract archive paths.
+                        (Path(directory) / name).write_bytes(bundle.read(name))
+                doc = json.loads((Path(directory) / "outcomes.json").read_text())
+                planned = json.loads((Path(directory) / "mutants.json").read_text())
+                count = fixture["planned_and_completed"]
+                self.assertEqual((len(planned), doc["total_mutants"]), (count, count))
+                self.assertEqual(len(doc["outcomes"]), count + 1)
+                self.assertEqual(doc["outcomes"][0]["scenario"], "Baseline")
+                self.assertEqual(doc["outcomes"][0]["summary"], "Success")
+                self.assertEqual(doc["success"], 0)  # Baseline is NOT a mutant counter.
+                # Accepted exit 3 is a test input, not a claim about the captured process exit.
+                self.assertTrue(freshness.mutation_complete(3, directory))
+                self.assertFalse(self.check(doc, planned + [{}], 3))
+                self.assertFalse(self.check(dict(doc, success=1), planned, 3))
+
     def test_mutation_cli_records_false_without_changing_advisory_exit(self):
         with tempfile.TemporaryDirectory() as directory:
             output = Path(directory) / "output"

```

## Full affected runtime helper

```python
#!/usr/bin/env python3
"""[GPT-6 Astra] Prove heavy nightly completion before skipping an unchanged head.

Only scheduled ci.yml runs on main are evidence. A successful workflow with skipped
heavy jobs is NOT evidence; follow it back to real work within the fixed read budget.
Incomplete work remains eligible at the next ordinary schedule; there is no rerun API
or retry loop. Unreadable evidence blocks admission. If proof ages out of the read
window, a bounded active-run census permits periodic remeasurement. Manual dispatch
remains an explicit force and makes no history requests.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlencode

HISTORY_LIMIT = 10
JOB_PAGE_SIZE = 100
JOB_PAGE_LIMIT = 2
MUTATION_JOBS = 51  # Pinned to the live matrix by test_ci_nightly_freshness.py.
COVERAGE = "coverage (nightly, full incl. heavy vectors)"
COVERAGE_STEP = "Measure + enforce per-crate coverage (FULL tier, max-remeasure gate)"
MUTATION = "mutation ratchet (cargo-mutants, advisory)"
MUTATION_STEP = "Nightly mutation work completed"
# [GPT-6 Astra] All non-completed statuses supported by the workflow-runs API.
ACTIVE_STATUSES = ("queued", "in_progress", "waiting", "requested", "pending")
COMPLETED_CONCLUSIONS = ("success", "failure", "cancelled", "timed_out", "startup_failure",
                         "stale", "neutral", "action_required", "skipped")


class EvidenceError(Exception):
    """No automatic admission or freshness claim is justified."""


def positive_int(value):
    return type(value) is int and value > 0


def mutation_complete(exit_code, directory):
    """Prove completion, independently of the existing advisory ratchet verdict.

    cargo-mutants v25.3.1 src/{exit_code,outcome}.rs: 0/2/3 are completed
    runs (caught/missed/mutant timeouts). Other exits do not prove execution.
    mutants.json is written before testing; outcomes counts grow incrementally.
    Their equality is necessary even for an accepted exit; partial files are not
    evidence. A new tool schema fails closed until its contract is reviewed.
    """
    if exit_code not in (0, 2, 3):
        return False
    try:
        planned = json.loads((Path(directory) / "mutants.json").read_text())
        doc = json.loads((Path(directory) / "outcomes.json").read_text())
    except (OSError, ValueError):
        return False
    if not isinstance(planned, list) or not isinstance(doc, dict):
        return False
    counts = [doc.get(key) for key in ("caught", "missed", "timeout", "unviable")]
    if any(type(n) is not int or n < 0 for n in counts):
        return False
    return (type(doc.get("total_mutants")) is int
            and doc["total_mutants"] == len(planned) == sum(counts)
            and type(doc.get("success")) is int and doc["success"] == 0)


def gh_json(endpoint):
    """One bounded GET, no retries (including rate-limit/permission failures)."""
    try:
        result = subprocess.run(
            ["gh", "api", "--method", "GET", endpoint], capture_output=True,
            text=True, timeout=30, check=False,
        )
        if result.returncode:
            raise EvidenceError(f"GitHub read failed (exit {result.returncode}); no retry")
        return json.loads(result.stdout)
    except (OSError, subprocess.TimeoutExpired, ValueError) as exc:
        raise EvidenceError("GitHub evidence unavailable or unreadable; no retry") from exc


def nightly_identity(run, repo, head):
    return (isinstance(run, dict) and positive_int(run.get("id"))
            and positive_int(run.get("run_attempt")) and run.get("event") == "schedule"
            and run.get("head_sha") == head and run.get("head_branch") == "main"
            and run.get("path") == ".github/workflows/ci.yml"
            and isinstance(run.get("repository"), dict)
            and run["repository"].get("full_name") == repo)


def ensure_no_active(get, repo, head, current_id):
    """Five GETs maximum, no pagination/retries. Check beyond the history window.

    Two rows suffice: the current run and any competing run. Larger, malformed,
    filtered-status-mismatched or incomplete inventories cannot authorize work.
    """
    for status in ACTIVE_STATUSES:
        query = urlencode({"event": "schedule", "head_sha": head,
                           "status": status, "per_page": 2})
        doc = get(f"repos/{repo}/actions/workflows/ci.yml/runs?{query}")
        if (not isinstance(doc, dict) or type(doc.get("total_count")) is not int
                or not isinstance(doc.get("workflow_runs"), list)
                or not 0 <= doc["total_count"] <= 2
                or len(doc["workflow_runs"]) != doc["total_count"]):
            raise EvidenceError("active-run census malformed or truncated")
        seen = set()
        for run in doc["workflow_runs"]:
            if (not nightly_identity(run, repo, head) or run["id"] in seen
                    or run.get("status") != status or run.get("conclusion") is not None):
                raise EvidenceError("active-run census identity or status unreadable")
            seen.add(run["id"])
            if run["id"] != current_id:
                raise EvidenceError("another same-head schedule is active")


def read_jobs(get, repo, run):
    jobs = []
    expected = None
    for page in range(1, JOB_PAGE_LIMIT + 1):
        doc = get(f"repos/{repo}/actions/runs/{run['id']}/attempts/"
                  f"{run['run_attempt']}/jobs?per_page={JOB_PAGE_SIZE}&page={page}")
        if not isinstance(doc, dict) or type(doc.get("total_count")) is not int:
            raise EvidenceError("missing job inventory")
        total, batch = doc["total_count"], doc.get("jobs")
        if (total < 0 or total > JOB_PAGE_SIZE * JOB_PAGE_LIMIT
                or not isinstance(batch, list) or len(batch) > JOB_PAGE_SIZE
                or (expected is not None and total != expected)):
            raise EvidenceError("unbounded or inconsistent job inventory")
        expected = total
        jobs.extend(batch)
        if len(jobs) == total:
            break
        if len(batch) != JOB_PAGE_SIZE or len(jobs) > total:
            raise EvidenceError("truncated job inventory")
    ids = set()
    for job in jobs:
        if (not isinstance(job, dict) or not positive_int(job.get("id"))
                or job["id"] in ids or job.get("run_id") != run["id"]
                # [GPT-6 Astra] The attempt-scoped endpoint supplies the binding
                # when this optional field is absent. A contradictory value does not.
                or ("run_attempt" in job and (not positive_int(job["run_attempt"])
                                              or job["run_attempt"] != run["run_attempt"]))
                or job.get("head_sha") != run["head_sha"]):
            raise EvidenceError("job identity mismatch")
        ids.add(job["id"])
    return jobs


def successful_step(job, name):
    # [GPT-6 Astra] The API may omit optional steps: readable absence is no proof.
    # Null/non-array/malformed values are still unreadable evidence, not admission.
    if "steps" not in job:
        return False
    steps = job.get("steps")
    if not isinstance(steps, list) or any(not isinstance(s, dict) for s in steps):
        raise EvidenceError("heavy step inventory unreadable")
    matches = [s for s in steps if s.get("name") == name]
    return (len(matches) == 1 and matches[0].get("status") == "completed"
            and matches[0].get("conclusion") == "success")


def heavy_state(jobs):
    coverage = [j for j in jobs if j.get("name") == COVERAGE]
    mutations = [j for j in jobs if isinstance(j.get("name"), str)
                 and (j["name"] == MUTATION or j["name"].startswith(MUTATION + " ("))]
    if len(coverage) != 1 or not mutations:
        return "incomplete"
    heavy = coverage + mutations
    # [GPT-6 Astra] GitHub represents a job skipped before matrix expansion with
    # one base-name placeholder. Never accept a partial/mixed matrix as a skip.
    if (len(mutations) == 1 and mutations[0]["name"] == MUTATION
            and all(j.get("status") == "completed" and j.get("conclusion") == "skipped"
                    for j in heavy)):
        return "skipped"
    if (len(mutations) != MUTATION_JOBS
            or len({j["name"] for j in mutations}) != MUTATION_JOBS
            or any(j["name"] == MUTATION for j in mutations)
            or any(j.get("status") != "completed" or j.get("conclusion") != "success"
                   for j in heavy)
            or not successful_step(coverage[0], COVERAGE_STEP)
            or not all(successful_step(j, MUTATION_STEP) for j in mutations)):
        return "incomplete"
    return "complete"


def decide(event, repo, head, current_id, get=gh_json):
    """Return (run_heavy, reason); only this ordinary schedule may admit new work."""
    if event == "workflow_dispatch":
        return True, "manual dispatch explicitly requests heavy work"
    if event != "schedule":
        return False, "heavy nightly work is isolated from this event"
    if (not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo)
            or not re.fullmatch(r"[0-9a-f]{40}", head) or not positive_int(current_id)):
        raise EvidenceError("invalid scheduled run identity")
    query = urlencode({"event": "schedule", "head_sha": head, "per_page": HISTORY_LIMIT})
    doc = get(f"repos/{repo}/actions/workflows/ci.yml/runs?{query}")
    if (not isinstance(doc, dict) or type(doc.get("total_count")) is not int
            or doc["total_count"] < 0 or not isinstance(doc.get("workflow_runs"), list)):
        raise EvidenceError("missing scheduled history")
    runs = doc["workflow_runs"]
    if len(runs) != min(doc["total_count"], HISTORY_LIMIT):
        raise EvidenceError("truncated scheduled history")
    seen = set()
    for run in runs:
        if not nightly_identity(run, repo, head) or run["id"] in seen:
            raise EvidenceError("scheduled history identity mismatch")
        seen.add(run["id"])
    # [GPT-6 Astra] Run IDs order creation, not the update time of an old rerun.
    # A newer same-head schedule means this tick is stale; do not start more work.
    if any(run["id"] > current_id for run in runs):
        raise EvidenceError("newer same-head schedule exists")
    prior = sorted((r for r in runs if r["id"] != current_id),
                   key=lambda r: r["id"], reverse=True)
    if any(r.get("status") != "completed" for r in prior):
        raise EvidenceError("same-head schedule still active or status unreadable")
    for run in prior:
        if run.get("conclusion") not in COMPLETED_CONCLUSIONS:
            raise EvidenceError("scheduled conclusion unreadable or unsupported")
        state = heavy_state(read_jobs(get, repo, run))
        if state == "complete":
            return False, f"heavy completion verified in run {run['id']} attempt {run['run_attempt']}"
        if state == "incomplete":
            if doc["total_count"] > HISTORY_LIMIT:
                ensure_no_active(get, repo, head, current_id)
            return True, f"heavy work incomplete in run {run['id']}; admit this ordinary schedule"
    if doc["total_count"] > HISTORY_LIMIT:
        ensure_no_active(get, repo, head, current_id)
        return True, "completion proof aged out; admit periodic measurement on this ordinary schedule"
    if prior:
        return True, "only skipped follow-ups exist; this schedule must perform the heavy work"
    return True, "no prior same-head schedule; admit its first heavy attempt"


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--mutation-complete":
        complete = mutation_complete(int(sys.argv[2]), sys.argv[3])
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(f"measurement_completed={str(complete).lower()}\n")
        print(f"Mutation completion evidence: {str(complete).lower()}")
        return 0  # Evidence only; preserve the existing advisory command/ratchet policy.
    fresh = False
    try:
        fresh, reason = decide(os.environ.get("GITHUB_EVENT_NAME", ""),
                               os.environ.get("GITHUB_REPOSITORY", ""),
                               os.environ.get("GITHUB_SHA", ""),
                               int(os.environ.get("GITHUB_RUN_ID", "0")))
        code = 0
    except (EvidenceError, ValueError, TypeError) as exc:
        reason, code = f"nightly admission blocked: {exc}", 1
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"fresh={str(fresh).lower()}\n")
    print(reason)
    return code


if __name__ == "__main__":
    sys.exit(main())

```

## Complete affected test functions and executable fixture dependencies

```python
#!/usr/bin/env python3
"""[GPT-6 Astra] Offline behavior, real producer and CI wiring for issue #6436."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

import yaml

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("freshness", ROOT / "scripts/ci_nightly_freshness.py")
freshness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(freshness)
REPO = "sparq-org/sparq"
HEAD = "a" * 40
CURRENT = 1000


def workflow_job_bound(workflow, seen=()):
    """[GPT-6 Astra] Conservative full inventory, including local reusable jobs.

    Ignore job conditions/excludes; count each include as an extra even when it
    merely enriches an existing leg. This overcounts rather than understating
    API inventory. Unknown dynamic/remote expansion requires an explicit review.
    """
    total = 0
    for item in workflow["jobs"].values():
        matrix = item.get("strategy", {}).get("matrix", {})
        assert isinstance(matrix, dict), "dynamic matrix needs a reviewed inventory bound"
        axes = [value for key, value in matrix.items() if key not in ("include", "exclude")]
        assert all(isinstance(axis, list) and axis for axis in axes), "unbounded matrix axis"
        includes = matrix.get("include", [])
        assert isinstance(includes, list) and all(isinstance(x, dict) for x in includes)
        count = (math.prod(map(len, axes)) if axes else int(not includes)) + len(includes)
        if "uses" in item:
            path = item["uses"]
            assert path.startswith("./.github/workflows/") and path not in seen, path
            child = yaml.safe_load((ROOT / path).read_text())
            count *= workflow_job_bound(child, (*seen, path))
        total += count
    return total


def assert_job_capacity(workflow):
    bound = workflow_job_bound(workflow)
    capacity = freshness.JOB_PAGE_SIZE * freshness.JOB_PAGE_LIMIT
    assert bound + 20 <= capacity, f"full workflow bound {bound} needs 20 spare slots within {capacity}"
    return bound


def run(run_id=999, **changes):
    item = dict(id=run_id, run_attempt=1, event="schedule", head_sha=HEAD,
                head_branch="main", path=".github/workflows/ci.yml",
                repository={"full_name": REPO}, status="completed", conclusion="success")
    item.update(changes)
    return item


def job(name, index, step=None, **changes):
    item = dict(id=index, run_id=999, run_attempt=1, head_sha=HEAD,
                name=name, status="completed", conclusion="success", steps=[])
    if step:
        item["steps"] = [dict(name=step, status="completed", conclusion="success")]
    item.update(changes)
    return item


def completed_jobs():
    return ([job(freshness.COVERAGE, 1, freshness.COVERAGE_STEP)]
            + [job(f"{freshness.MUTATION} (shard-{i})", i + 2, freshness.MUTATION_STEP)
               for i in range(freshness.MUTATION_JOBS)])


def skipped_jobs():
    return [job(freshness.COVERAGE, 1, conclusion="skipped"),
            job(freshness.MUTATION, 2, conclusion="skipped")]


class FakeAPI:
    def __init__(self, runs=None, jobs=None, history=None, active=None):
        self.history = history if history is not None else {
            "total_count": len(runs or []), "workflow_runs": runs or []}
        self.jobs = jobs if jobs is not None else {999: completed_jobs()}
        self.calls = []
        self.active = active or {}

    def __call__(self, endpoint):
        self.calls.append(endpoint)
        if "/workflows/" in endpoint:
            status = parse_qs(urlsplit(endpoint).query).get("status")
            if status:
                result = self.active.get(status[0], {"total_count": 0, "workflow_runs": []})
                if isinstance(result, Exception):
                    raise result
                return copy.deepcopy(result)
            return copy.deepcopy(self.history)
        parts = endpoint.split("/")
        run_id = int(parts[5])
        self.assert_attempt(parts, run_id)
        jobs = self.jobs[run_id]
        page = int(endpoint.rsplit("page=", 1)[1])
        return {"total_count": len(jobs), "jobs": copy.deepcopy(jobs[(page - 1) * 100:page * 100])}

    def assert_attempt(self, parts, run_id):
        expected = next(r["run_attempt"] for r in self.history["workflow_runs"] if r["id"] == run_id)
        assert parts[6:8] == ["attempts", str(expected)], parts


class Admission(unittest.TestCase):
    def decide(self, api, event="schedule"):
        return freshness.decide(event, REPO, HEAD, CURRENT, api)


    def blocked(self, api, pattern=None):
        with self.assertRaisesRegex(freshness.EvidenceError, pattern or "."):
            self.decide(api)


    def readable_decision(self, api):
        try:
            return self.decide(api)
        except freshness.EvidenceError as exc:
            self.fail(f"readable fixture rejected: {exc}")


    def test_known_terminal_conclusions_with_zero_jobs_are_readable_incomplete(self):
        for conclusion in ["success", "failure", "cancelled", "timed_out", "startup_failure",
                           "stale", "neutral", "action_required", "skipped"]:
            with self.subTest(conclusion=conclusion):
                api = FakeAPI([run(conclusion=conclusion)], {999: []})
                self.assertTrue(self.readable_decision(api)[0])
                self.assertEqual(len(api.calls), 2)
        api = FakeAPI([run(conclusion="future_unknown")], {999: []})
        self.blocked(api, "conclusion")
        self.assertEqual(len(api.calls), 1)


    def test_omitted_optional_steps_are_incomplete_not_completion(self):
        for index in [0, 1]:
            with self.subTest(index=index):
                jobs = completed_jobs()
                del jobs[index]["steps"]
                api = FakeAPI([run()], {999: jobs})
                self.assertTrue(self.readable_decision(api)[0])
                self.assertEqual(len(api.calls), 2)


    def test_attempt_scoped_job_lookup_and_mismatched_attempt_blocks(self):
        jobs = completed_jobs()
        for item in jobs:
            item["run_attempt"] = 2
        api = FakeAPI([run(run_attempt=2)], {999: jobs})
        self.assertFalse(self.readable_decision(api)[0])
        self.assertIn("/attempts/2/jobs?", api.calls[-1])
        jobs[0]["run_attempt"] = 1
        self.blocked(FakeAPI([run(run_attempt=2)], {999: jobs}), "identity")


    def test_attempt_endpoint_binds_omitted_optional_field_but_rejects_contradictions(self):
        jobs = completed_jobs()
        for item in jobs:
            del item["run_attempt"]
        api = FakeAPI([run(run_attempt=2)], {999: jobs})
        self.assertFalse(self.readable_decision(api)[0])
        self.assertIn("/attempts/2/jobs?", api.calls[-1])
        for attempt in [None, True, 0, 1, 3]:
            with self.subTest(attempt=attempt):
                jobs[0]["run_attempt"] = attempt
                self.blocked(FakeAPI([run(run_attempt=2)], {999: jobs}), "identity")


class ProductionWiring(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        cls.jobs = cls.workflow["jobs"]
        cls.mutation = cls.jobs["mutants-nightly-advisory"]
        cls.producer = next(s for s in cls.mutation["steps"] if s.get("id") == "run_mutants")


    def test_full_workflow_inventory_fits_bounded_job_reads_with_headroom(self):
        # Unlike the 51-heavy-leg assertion, includes every job and every matrix.
        assert_job_capacity(self.workflow)


    def test_inventory_guard_catches_growth_outside_the_heavy_matrix(self):
        workflow = copy.deepcopy(self.workflow)
        workflow["jobs"]["test"]["strategy"]["matrix"] = {"shard": list(range(201))}
        with self.assertRaisesRegex(AssertionError, "spare slots"):
            assert_job_capacity(workflow)
        workflow["jobs"]["test"]["strategy"]["matrix"] = "${{ fromJSON(needs.dynamic.outputs.matrix) }}"
        with self.assertRaisesRegex(AssertionError, "dynamic matrix"):
            assert_job_capacity(workflow)


class MutationCompletion(unittest.TestCase):
    def check(self, doc, planned=None, exit_code=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            if planned is not None:
                (root / "mutants.json").write_text(json.dumps(planned))
            (root / "outcomes.json").write_text(json.dumps(doc))
            return freshness.mutation_complete(exit_code, directory)


    def test_real_pinned_unsharded_and_sharded_artifacts_prove_completion(self):
        fixtures = ROOT / "scripts/tests/fixtures/ci-nightly-freshness"
        provenance = json.loads((fixtures / "provenance.json").read_text())
        for fixture in provenance["fixtures"]:
            with self.subTest(artifact=fixture["artifact_id"]), tempfile.TemporaryDirectory() as directory:
                archive = fixtures / fixture["file"]
                self.assertLess(archive.stat().st_size, 50000)
                self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), fixture["sha256"])
                with zipfile.ZipFile(archive) as bundle:
                    self.assertEqual(sorted(bundle.namelist()), ["mutants.json", "outcomes.json"])
                    for name in ["mutants.json", "outcomes.json"]:
                        self.assertLess(bundle.getinfo(name).file_size, 400000)
                        # Fixed member names only; never extract archive paths.
                        (Path(directory) / name).write_bytes(bundle.read(name))
                doc = json.loads((Path(directory) / "outcomes.json").read_text())
                planned = json.loads((Path(directory) / "mutants.json").read_text())
                count = fixture["planned_and_completed"]
                self.assertEqual((len(planned), doc["total_mutants"]), (count, count))
                self.assertEqual(len(doc["outcomes"]), count + 1)
                self.assertEqual(doc["outcomes"][0]["scenario"], "Baseline")
                self.assertEqual(doc["outcomes"][0]["summary"], "Success")
                self.assertEqual(doc["success"], 0)  # Baseline is NOT a mutant counter.
                # Accepted exit 3 is a test input, not a claim about the captured process exit.
                self.assertTrue(freshness.mutation_complete(3, directory))
                self.assertFalse(self.check(doc, planned + [{}], 3))
                self.assertFalse(self.check(dict(doc, success=1), planned, 3))



```

## Genuine artifact evidence (derived summary; original ZIPs in committed source)

```json
[
  {
    "file": "mutants-parse-10010502882.zip",
    "artifact_id": 10010502882,
    "sha256": "7d60fdfa6ce36bdf3f5d5105c476ebf7fa61c273a5735f53b15e7b0eeea0aa63",
    "planned_and_completed": 102,
    "shard": null,
    "zip_bytes": 9720,
    "planned": 102,
    "counters": {
      "total_mutants": 102,
      "caught": 74,
      "missed": 15,
      "timeout": 12,
      "unviable": 1,
      "success": 0
    },
    "outcome_rows": 103,
    "first_outcome": {
      "scenario": "Baseline",
      "summary": "Success"
    },
    "member_bytes": {
      "outcomes.json": 178845,
      "mutants.json": 59356
    }
  },
  {
    "file": "mutants-engine-10036242958.zip",
    "artifact_id": 10036242958,
    "sha256": "77bc3dd50465d6c7987141b49eb9d4b656add76129ca0e79ac4734f597673752",
    "planned_and_completed": 197,
    "shard": "23/24",
    "zip_bytes": 29016,
    "planned": 197,
    "counters": {
      "total_mutants": 197,
      "caught": 52,
      "missed": 130,
      "timeout": 1,
      "unviable": 14,
      "success": 0
    },
    "outcome_rows": 198,
    "first_outcome": {
      "scenario": "Baseline",
      "summary": "Success"
    },
    "member_bytes": {
      "outcomes.json": 339757,
      "mutants.json": 114965
    }
  }
]

```

## Unchanged production caller/producer/permissions projection from frozen packet

```yaml
name: CI
permissions:
  contents: read
concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}-${{ (github.event_name == 'schedule' || github.event_name ==
    'workflow_dispatch' || (github.event_name == 'pull_request' && contains(fromJSON('["labeled","unlabeled"]'),
    github.event.action))) && github.run_id || 'shared' }}
  cancel-in-progress: ${{ github.event_name != 'schedule' && github.event_name != 'workflow_dispatch' }}
env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1
  CARGO_NET_RETRY: '10'
  CARGO_INCREMENTAL: '0'
  CARGO_PROFILE_DEV_DEBUG: line-tables-only
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: -C link-arg=-fuse-ld=lld
'on':
  push:
    branches:
    - main
  pull_request:
    types:
    - opened
    - synchronize
    - reopened
    - labeled
    - unlabeled
    - ready_for_review
  merge_group: null
  workflow_dispatch: null
  schedule:
  - cron: 17 3 * * *
jobs:
  nightly-gate:
    if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'
    runs-on: ubuntu-latest
    permissions:
      actions: read
      contents: read
    outputs:
      fresh: ${{ steps.gate.outputs.fresh }}
    steps:
    - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0
      with:
        persist-credentials: false
        sparse-checkout: scripts
    - id: gate
      env:
        GH_TOKEN: ${{ github.token }}
      run: python3 scripts/ci_nightly_freshness.py
  coverage-nightly:
    name: coverage (nightly, full incl. heavy vectors)
    needs: nightly-gate
    if: needs.nightly-gate.outputs.fresh == 'true'
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: -C link-arg=-fuse-ld=lld
    timeout-minutes: 60
    steps:
    - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0
    - name: Free runner disk before build + instrumented test
      run: ./scripts/ci-free-disk.sh
    - uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8
      with:
        components: llvm-tools-preview
    - name: Ensure llvm-tools-preview on the pinned toolchain
      run: rustup component add llvm-tools-preview
    - uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32
      with:
        save-if: ${{ github.ref == 'refs/heads/main' }}
    - name: Install cargo-llvm-cov
      uses: taiki-e/install-action@18b1216eba7f8039b0f8d131d5473787f0edce68
      with:
        tool: cargo-llvm-cov
    - name: Test-presence gate
      run: python3 scripts/coverage-presence.py --check
    - name: Measure + enforce per-crate coverage (FULL tier, max-remeasure gate)
      env:
        COVERAGE_TIER: nightly
      run: 'set -o pipefail

        ./scripts/coverage.sh

        python3 scripts/coverage-gate.py --check-robust target/coverage/coverage-summary.json

        '
    - name: Upload coverage summary
      uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a
      if: always()
      with:
        name: coverage-summary-nightly
        path: target/coverage/coverage-summary.json
  mutants-nightly-advisory:
    name: mutation ratchet (cargo-mutants, advisory)
    needs: nightly-gate
    if: needs.nightly-gate.outputs.fresh == 'true'
    runs-on: ubuntu-latest
    timeout-minutes: ${{ fromJSON(matrix.timeout_minutes || '120') }}
    continue-on-error: true
    strategy:
      fail-fast: false
      max-parallel: 8
      matrix:
        crate:
        - sparq-canon
        - sparq-parse
        - sparq-introspect
        - sparq-prov
        - sparq-algos
        - sparq-core
        - sparq-shacl
        - sparq-geo
        - sparq-text
        - sparq-hdt
        - sparq-rsp
        - sparq-sim
        - sparq-solid
        - sparq-nlq
        - sparq-mpc
        - sparq-serve
        - sparq-server
        - sparq-zk
        - sparq-zk-compose
        - sparq-vectors
        - sparq-fedplan
        - sparq-fedclient
        - sparq-policy
        - sparq-substrate
        include:
        - crate: sparq-core
          features: mmap,dict-spill
          timeout_minutes: 360
        - crate: sparq-substrate
          features: numeric,join,compare,rows
        - crate: sparq-sim
          features: multi-hop,explain,sketch
        - crate: sparq-canon
          features: rdf12-triple-terms
        - crate: sparq-prov
          features: reason
        - crate: sparq-fedplan
          features: fedplan,adaptive-replan
        - crate: sparq-fedclient
          features: fedclient,fedclient-adaptive
        - crate: sparq-solid
          timeout_minutes: 360
        - crate: sparq-zk-compose
          timeout_minutes: 360
        - crate: sparq-vectors
          timeout_minutes: 360
        - crate: sparq-server
          features: audit-log,access-audit,change-stream,federation-descriptors,solid-authz,odrl-authz,solid-authz-trust,templates,tpf,brtpf,query-registry,facets,complete,service,time-travel,geo,terse,response-compression
          timeout_minutes: 360
        - crate: sparq-mpc
          timeout_minutes: 360
        - crate: sparq-reason
          shard: 1/3
        - crate: sparq-reason
          shard: 2/3
        - crate: sparq-reason
          shard: 3/3
        - crate: sparq-engine
          shard: 1/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 2/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 3/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 4/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 5/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 6/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 7/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 8/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 9/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 10/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 11/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 12/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 13/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 14/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 15/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 16/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 17/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 18/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 19/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 20/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 21/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 22/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 23/24
          timeout_minutes: 360
        - crate: sparq-engine
          shard: 24/24
          timeout_minutes: 360
    concurrency:
      group: mutants-${{ github.ref }}-${{ matrix.crate }}${{ matrix.shard && format('-shard-{0}', matrix.shard)
        || '' }}
      cancel-in-progress: false
    permissions:
      contents: read
    steps:
    - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0
    - name: Free runner disk before mutation build
      run: ./scripts/ci-free-disk.sh
    - name: Rust (preinstalled stable)
      run: 'rustup default stable

        rustup show active-toolchain

        '
    - uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32
      with:
        save-if: ${{ github.ref == 'refs/heads/main' }}
    - name: Install cargo-mutants
      uses: taiki-e/install-action@18b1216eba7f8039b0f8d131d5473787f0edce68
      with:
        tool: cargo-mutants@25.3.1
    - name: Run cargo-mutants for this crate + ratchet against the baseline
      id: run_mutants
      continue-on-error: true
      env:
        CRATE: ${{ matrix.crate }}
        FEATURES: ${{ matrix.features }}
        SHARD: ${{ matrix.shard }}
      run: "set -o pipefail\n# [SONNET-4.6] sq-qcnn.28: sub-shard support. SHARD is empty for most crates;\n# for\
        \ sparq-reason it is \"1/3\", \"2/3\", or \"3/3\". The shard suffix maps \"/\" to \"-\"\n# for use in file\
        \ paths and artifact names (\"1/3\" -> \"shard-1-3\").\nshard_args=()\nartifact_suffix=\"\"\nif [ -n \"\
        $SHARD\" ]; then\n  shard_args=(--shard \"$SHARD\")\n  artifact_suffix=\"-shard-${SHARD//\\//-}\"\nfi\n\
        echo \"artifact_suffix=${artifact_suffix}\" >> \"$GITHUB_OUTPUT\"\noutput_dir=\"target/mutants/${CRATE}${artifact_suffix}\"\
        \nmkdir -p \"$output_dir\"\nfeat=()\n[ -n \"$FEATURES\" ] && feat=(--features \"$FEATURES\")\necho \"::group::cargo-mutants\
        \ $CRATE ${feat[*]}${SHARD:+ --shard $SHARD}\"\n# [GPT-6 Astra] Preserve advisory handling, but do not let\
        \ a masked command\n# failure become evidence for skipping later heavy work at the same head.\nmutation_exit=0\n\
        cargo mutants -p \"$CRATE\" \"${feat[@]}\" \"${shard_args[@]}\" -o \"$output_dir\" \\\n  || mutation_exit=$?\n\
        if [ \"$mutation_exit\" -ne 0 ]; then\n  echo \"WARN: cargo-mutants $CRATE did not complete cleanly (advisory)\"\
        \nfi\necho \"::endgroup::\"\n# [GPT-6 Astra] Counts must cover the prewritten planned mutant list.\n# Exit\
        \ 2 (missed) / 3 (mutant timeouts) can be complete advisory results.\npython3 scripts/ci_nightly_freshness.py\
        \ --mutation-complete \"$mutation_exit\" \"$output_dir/mutants.out\"\n# Run the ratchet --check on THIS\
        \ crate's outcomes. --check NEVER fails on an\n# UNSEEDED crate (it reports it for the next --seed); it\
        \ exits 1 only if a SEEDED\n# crate now has MORE survivors than its committed ceiling. While advisory\n\
        # (continue-on-error) that regression is REPORTED but does not block; promotion to\n# gating (sq-qcnn.27)\
        \ drops continue-on-error + this job's \"advisory\" name token.\nOUTCOMES=$(find \"$output_dir\" -name outcomes.json)\n\
        echo \"outcomes: ${OUTCOMES:-<none>}\"\nif [ -n \"$OUTCOMES\" ]; then\n  # shellcheck disable=SC2086 # intentional\
        \ word-split: each path is a separate arg\n  python3 scripts/mutants-gate.py --check $OUTCOMES \\\n    |\
        \ tee -a \"$GITHUB_STEP_SUMMARY\"\nelse\n  echo \"no outcomes produced for $CRATE${SHARD:+ shard=$SHARD}\
        \ \u2014 nothing to check\" \\\n    | tee -a \"$GITHUB_STEP_SUMMARY\"\nfi\n"
    - name: Nightly mutation work completed
      if: ${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'true' }}
      run: echo "Mutation measurement completed; advisory ratchet verdict remains separate."
    - name: Upload mutation outcomes
      uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a
      if: always()
      with:
        name: mutants-outcomes-nightly-${{ matrix.crate }}${{ steps.run_mutants.outputs.artifact_suffix }}
        path: 'target/mutants/${{ matrix.crate }}${{ steps.run_mutants.outputs.artifact_suffix }}/mutants.out/outcomes.json

          target/mutants/${{ matrix.crate }}${{ steps.run_mutants.outputs.artifact_suffix }}/mutants.out/mutants.json

          '

```

The production YAML is a parsed projection of the full unchanged ci.yml; complete raw workflow and full whole-change diff are provided in source/ and whole-change.diff. The unchanged docs-quality quick-gates step invokes `python3 scripts/tests/test_ci_nightly_freshness.py`; the prior packet holds its full wiring/provenance. Composition retains that step and main’s dashboard test exactly once. No unchanged reporter/review/hold source is touched.

## Full workflow inventory inputs for B2

```yaml
_note: Exact job names/uses/matrices from committed CI; omitted conditions and steps
  do not reduce the conservative bound. The referenced local ci-select workflow has
  one job and no matrix/uses nesting. Full raw ci.yml and ci-select.yml blobs accompany
  this context.
ci.yml jobs:
  changes:
    name: detect rust changes
  select:
    name: select
    uses: ./.github/workflows/ci-select.yml
  selection-backstop:
    name: nightly selection backstop (full-matrix guard)
  build-archive:
    name: build + archive test binaries (+ doctests once)
  test:
    name: test (load-aware shard ${{ matrix.name }})
    matrix:
      include:
      - name: heavy-diskann
        filter: test(=diskann_recall_at_10_vs_brute_force_on_50k)
        rayon_threads: '1'
        crate: sparq-vectors
      - name: heavy-hnsw
        filter: test(=hnsw_recall_at_10_vs_brute_force_on_50k)
        rayon_threads: '1'
        crate: sparq-vectors
      - name: bulk 1/3
        partition: count:1/3
        crate: ''
      - name: bulk 2/3
        partition: count:2/3
        crate: ''
      - name: bulk 3/3
        partition: count:3/3
        crate: ''
  heavy-recall-demoted-filer:
    name: heavy-recall demoted-lane safety-net (push-to-main)
  wasm:
    name: wasm build (sparq-wasm + sparq-reason-wasm + sparq-rsp-wasm + sparq-text-wasm
      + sparq-shacl-wasm + sparq-solid)
  lint:
    name: clippy (gate) + fmt (non-blocking)
  msrv:
    name: MSRV check (Rust 1.88, declared floor)
  conformance:
    name: W3C SPARQL conformance (ratchet >= 1229 pass+divergence)
  shacl-conformance:
    name: "W3C SHACL conformance (ratchet \u2014 core >= 98, sparql >= 5; full 1.2\
      \ ratchets in-runner)"
  geo-conformance:
    name: "W3C/OGC GeoSPARQL conformance (ratchet \u2014 topology >= 119, query-rewrite\
      \ >= 38)"
  solid-conformance:
    name: "Solid WAC/ACP conformance (ratchet \u2014 wac >= 13, acp >= 13)"
  odrl-conformance:
    name: "SolidLab ODRL conformance (ratchet \u2014 pass >= 59)"
  text-oracle:
    name: "sparq-text BM25 differential oracle (extension ratchet \u2014 assertions\
      \ >= 18750)"
  rsp-oracle:
    name: "sparq-rsp RSP expressivity / SRBench correctness (extension ratchet \u2014\
      \ assertions >= 149)"
  jsonld-conformance:
    name: "W3C JSON-LD 1.1 conformance (ratchet \u2014 toRdf >= 413, fromRdf >= 52,\
      \ compact >= 228, frame >= 92)"
  service-federation-conformance:
    name: SERVICE-federation harness (in-process loopback smoke + egress scoping)
  inference-conformance:
    name: Inference conformance (ratchet >= 1967 pass+divergence)
  coverage-floors:
    name: coverage floors (presence + monotonicity + shard-partition, no compile)
  coverage-measure:
    name: coverage ratchet (shard ${{ matrix.shard }}/3)
    matrix:
      shard:
      - 1
      - 2
      - 3
  coverage-engine-run:
    name: coverage engine run (partition ${{ matrix.part }}/3)
    matrix:
      part:
      - 1
      - 2
      - 3
  coverage-engine-merge:
    name: coverage engine merge + ratchet (per-crate)
  coverage:
    name: coverage ratchet + test-presence gate (per-crate)
  coverage-demoted-filer:
    name: coverage demoted-lane safety-net (push-to-main)
  nightly-gate: {}
  coverage-nightly:
    name: coverage (nightly, full incl. heavy vectors)
  mutants-nightly-advisory:
    name: mutation ratchet (cargo-mutants, advisory)
    matrix:
      crate:
      - sparq-canon
      - sparq-parse
      - sparq-introspect
      - sparq-prov
      - sparq-algos
      - sparq-core
      - sparq-shacl
      - sparq-geo
      - sparq-text
      - sparq-hdt
      - sparq-rsp
      - sparq-sim
      - sparq-solid
      - sparq-nlq
      - sparq-mpc
      - sparq-serve
      - sparq-server
      - sparq-zk
      - sparq-zk-compose
      - sparq-vectors
      - sparq-fedplan
      - sparq-fedclient
      - sparq-policy
      - sparq-substrate
      include:
      - crate: sparq-core
        features: mmap,dict-spill
        timeout_minutes: 360
      - crate: sparq-substrate
        features: numeric,join,compare,rows
      - crate: sparq-sim
        features: multi-hop,explain,sketch
      - crate: sparq-canon
        features: rdf12-triple-terms
      - crate: sparq-prov
        features: reason
      - crate: sparq-fedplan
        features: fedplan,adaptive-replan
      - crate: sparq-fedclient
        features: fedclient,fedclient-adaptive
      - crate: sparq-solid
        timeout_minutes: 360
      - crate: sparq-zk-compose
        timeout_minutes: 360
      - crate: sparq-vectors
        timeout_minutes: 360
      - crate: sparq-server
        features: audit-log,access-audit,change-stream,federation-descriptors,solid-authz,odrl-authz,solid-authz-trust,templates,tpf,brtpf,query-registry,facets,complete,service,time-travel,geo,terse,response-compression
        timeout_minutes: 360
      - crate: sparq-mpc
        timeout_minutes: 360
      - crate: sparq-reason
        shard: 1/3
      - crate: sparq-reason
        shard: 2/3
      - crate: sparq-reason
        shard: 3/3
      - crate: sparq-engine
        shard: 1/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 2/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 3/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 4/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 5/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 6/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 7/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 8/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 9/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 10/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 11/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 12/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 13/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 14/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 15/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 16/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 17/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 18/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 19/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 20/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 21/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 22/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 23/24
        timeout_minutes: 360
      - crate: sparq-engine
        shard: 24/24
        timeout_minutes: 360
  unsafe-register:
    name: unsafe-register (count ratchet)
  geiger:
    name: unsafe report (cargo-geiger, informational)
  docker-smoke:
    name: docker image smoke (run + curl /health + query)
ci-select.yml jobs:
  select:
    name: select (change-based test selection${{ (github.event_name == 'pull_request'
      && contains(fromJSON('["labeled","unlabeled"]'), github.event.action) && !contains(fromJSON('["ci-full","bench-full","fuzz-full"]'),
      github.event.label.name)) && ', no-leg' || (github.event.pull_request.draft
      == true && ', draft-tier' || '') }})

```
