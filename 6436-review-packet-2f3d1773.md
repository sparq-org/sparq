# Issue6436 whole-change review packet

Review exact head `2f3d1773bb3cebd36186692025912a8377c85db6` against `e44b08c59aea0755cfda1cbf633e25971db787f7`. Implementation: actual GPT-6 Astra, xhigh; independent actual Opus5 xhigh review is pending. Assess the whole diff and executable producer/consumer context, including trust, API bounds, availability, and advisory-policy preservation. Return blockers separately from nonblocking nits. This is source review; do not authorize deployment, rerun, labels, permissions or registry changes.

The four-file source was reconstructed from this agent's retained edit inputs after temporary-directory loss. Original source hashes did not survive, so byte-exact restoration is not claimed. Every result below was rerun after restoration. Source was preserved promptly in recovery commit9a8351ea4, followed by the malformed-step-inventory correction and the bounded census for aged-out proof.

The full raw diff includes both newly added files in full. Critical CI and test wiring below are complete parsed projections of the affected jobs (same expressions, matrix values, run bodies, needs, permissions and limits; comments/formatting omitted). The manifest also hashes unmodified raw copies of all four current source files for root-side inspection.

## Report

```json
{
  "issue": "https://github.com/sparq-org/sparq/issues/6436",
  "base": "e44b08c59aea0755cfda1cbf633e25971db787f7",
  "head": "2f3d1773bb3cebd36186692025912a8377c85db6",
  "branch": "codex/ci-heavy-nightly-freshness",
  "implementation": {
    "model": "GPT-6 Astra",
    "reasoning_effort": "xhigh",
    "independent_review": "pending actual Opus5 xhigh review by root"
  },
  "recovery": {
    "preserved_baseline": "e44b08c59aea0755cfda1cbf633e25971db787f7",
    "method": "Empty worktree recreated via surviving .git pointer/index and checkout-index; four-file source reconstructed from this agent retained conversation edit inputs.",
    "byte_verified_against_lost_source": false,
    "recovery_commit": "9a8351ea41600a363c1fd1bba4bf8c02a3af4d49",
    "old_tmp_artifacts_used_as_current_test_evidence": false,
    "cause_of_tmp_loss": "unverified"
  },
  "changed_files": [
    ".github/workflows/ci.yml",
    ".github/workflows/docs-quality.yml",
    "scripts/ci_nightly_freshness.py",
    "scripts/tests/test_ci_nightly_freshness.py"
  ],
  "diff_stat": " .github/workflows/ci.yml                   |  49 +--\n .github/workflows/docs-quality.yml         |   4 +\n scripts/ci_nightly_freshness.py            | 250 ++++++++++++++\n scripts/tests/test_ci_nightly_freshness.py | 537 +++++++++++++++++++++++++++++\n 4 files changed, 819 insertions(+), 21 deletions(-)\n",
  "contract": {
    "output": "On schedule, fresh=true admits heavy work; fresh=false with exit0 skips using verified proof, and with exit1 blocks on unreadable/unsafe evidence. Other events remain isolated. The existing output name is retained.",
    "completion": "Scheduled ci.yml/main run at the exact full head SHA, completed and with a supported terminal conclusion; attempt-scoped full job inventory; successful heavy coverage job and measurement step; all51 distinct mutation matrix job names successful with a successful completion marker. Overall workflow success alone supplies no proof, and unrelated workflow failures do not erase real completed heavy work.",
    "mutation_completion": "cargo-mutants25.3.1 exits0/2/3 are eligible only if strict nonnegative caught/missed/timeout/unviable counts and total_mutants equal the prewritten mutants.json length, with success0 for the actual testing command. Missing/invalid/truncated files cannot create a true completion output. This output is recorded before the advisory ratchet verdict; the ratchet's original execution and exit behavior remain separate and unchanged.",
    "scheduled_recovery": "Readable incomplete work is eligible at the next existing ordinary cron, without API reruns, shard retries or polling loops. Skipped follow-ups are traversed and never count as completion proof. Nontruncated skipped-only history admits the ordinary measurement.",
    "aged_out_proof": "When the fully readable history exceeds10 rows and cannot supply proof, check all five documented active statuses beyond the history window before ordinary periodic remeasurement. Also perform this census before admitting visible incomplete work from truncated history. Only current_id is excluded after repository/workflow/head/status identity checks; any other active run, malformed/truncated inventory, unknown/mismatched status or API error blocks admission.",
    "request_bounds": "At most1 history GET (10 rows),20 attempt-job GETs (2 pages of100 per inspected run), and5 active-status GETs (queued/in_progress/waiting/requested/pending,2 rows each):26 GET subprocesses maximum,30 seconds each, no pagination of the census and no application retries. Any known API failure stops the helper immediately.",
    "event_boundary": "Manual dispatch remains explicit force with no history reads. Other events, including merge_group, do not admit heavy work; the nightly job if-condition and output consumers remain unchanged.",
    "unchanged_policy": "Existing cron, workflow/mutation concurrency, max-parallel8, full51-leg matrix, coverage/mutation timeouts, advisory ratchet semantics, permissions and required aggregate gate policy remain intact. No new review/approval/registry/label/release mutation path exists."
  },
  "validation": {
    "focused_tests": 41,
    "selection_wiring_tests": 88,
    "runner_reservation_tests": 19,
    "actionlint": "PASS",
    "diff_check": "PASS",
    "line_coverage": {
      "source": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6436/scripts/ci_nightly_freshness.py",
      "executable_lines": 192,
      "covered_lines": 191,
      "never_executed_lines": [
        250
      ],
      "calibration": {
        "never_called_body_executed": false,
        "called_body_executed": true
      },
      "limitation": "Parent-process trace only. The __main__ entry executes in producer subprocesses but is not counted in the parent trace. Python3.14 synthetic line0/None entries are excluded."
    },
    "mutation_counts": {
      "KILLED": 22,
      "SURVIVED": 0,
      "INVALID_OR_ERROR": 0
    },
    "mutation_total_tests_per_run": 41,
    "preflight": "G1/G2/G6/guard-untested pass; privacy-claims fails on existing macOS Bash3 mapfile absence. No bypass or gate weakening."
  },
  "composition": {
    "pinned_gate_recovery_54dd9e9_files": [
      "scripts/ci_summary_gate.py",
      "scripts/tests/test_ci_summary_gate.py"
    ],
    "overlap_with_gate_recovery": [],
    "locally_observed_origin_main": "4d880d6a4a37b7d28a4259dfdbceea4a8db07eb0",
    "main_overlap": "Only docs-quality.yml has a separate three-line dashboard-publisher selftest addition near line729; this patch adds its step near1086. ci.yml/helper/test unchanged on that observed main.",
    "main_verified_by_root": true,
    "checkout_rebase_or_merge_performed": false,
    "clean_three_way_composition": {
      "method": "git merge-tree --write-tree (Git objects only; no checkout, branch, index, or remote mutation)",
      "current_main": "4d880d6a4a37b7d28a4259dfdbceea4a8db07eb0",
      "candidate_head": "2f3d1773bb3cebd36186692025912a8377c85db6",
      "merged_tree": "15429dcd17e47aa2f5c7b66d5c563adcd3a57067",
      "exit_code": 0,
      "both_bench_and_nightly_tests_wired_exactly_once": true,
      "candidate_files_identical_in_composition": [
        ".github/workflows/ci.yml",
        "scripts/ci_nightly_freshness.py",
        "scripts/tests/test_ci_nightly_freshness.py"
      ],
      "merged_docs_sha256": "8540fb4b537761d09f3816259e5029d81d50cc214b2147c62bebdfe211f2a594",
      "main_docs_delta": "diff --git a/.github/workflows/docs-quality.yml b/.github/workflows/docs-quality.yml\nindex 22359ebab..c2d47f674 100644\n--- a/.github/workflows/docs-quality.yml\n+++ b/.github/workflows/docs-quality.yml\n@@ -729,6 +729,9 @@ jobs:\n         run: python3 scripts/check-dashboard-publish-wiring.py --self-test\n       - name: \"Enforce every served bench/dashboard asset is wired into the Pages publisher \u2014 GATING\"\n         run: python3 scripts/check-dashboard-publish-wiring.py\n+      # [GPT-6-ASTRA] Exercise the actual publisher shell with file-only Git races.\n+      - name: Test bounded dashboard publication recovery (local Git)\n+        run: python3 scripts/tests/test_bench_dashboard_publish.py\n       # [OPUS-5] issue #5022: the SAME silent-drop class for the mdBook guide. GitHub Pages has\n       # one deploy slot, so the guide has no workflow of its own \u2014 docs.yml VALIDATES it on PRs\n       # and pages.yml BUILDS + OVERLAYS it into the single Pages artifact at out/guide/. Three\n"
    }
  },
  "trust": "Server-authored scheduled-run/job metadata tied to repository, ci.yml/main, full head SHA and run attempt. No comments, labels, reviewer receipts or advisory forgiveness enter the decision. Runtime helper needs only actions:read and contents:read; new checkout disables persisted credentials. Mutation proof is produced by the same pinned-head workflow and pinned cargo-mutants tool.",
  "limits": [
    "No live Actions run, cargo build, heavy benchmark or API permission probe was performed.",
    "Mutation completion counts rely on cargo-mutants25.3.1 output semantics, not a new independent mutation engine. Future schema/tool changes need review.",
    "The constant51 matrix cardinality is pinned to actual source expansion; step and job names are completion API contracts, with no job-name changes in this patch.",
    "Historical cancellation artifacts were lost. Offline baseline proof and new fixtures establish the source defect; no claim is made about a particular historical step log.",
    "Bounded controls target the headline completion/admission guards and production seams; they are not an exhaustive mutation audit.",
    "This candidate has not yet received independent Opus approval or protected CI/queue admission.",
    "The active-run census uses sequential read-only snapshots, not an atomic server reservation. Existing workflow/mutation concurrency remains unchanged; no new remote lock or permission is introduced."
  ],
  "primary_sources": [
    {
      "url": "https://raw.githubusercontent.com/sourcefrog/cargo-mutants/v25.3.1/src/exit_code.rs",
      "finding": "Pinned version distinguishes successful execution with caught, missed or timed-out mutants (0/2/3) from baseline/usage/internal failures."
    },
    {
      "url": "https://raw.githubusercontent.com/sourcefrog/cargo-mutants/v25.3.1/src/outcome.rs",
      "finding": "LabOutcome increments completed-mutant counts per finished scenario; missed and timeout counts determine exit2/3."
    },
    {
      "url": "https://raw.githubusercontent.com/sourcefrog/cargo-mutants/v25.3.1/src/output.rs",
      "finding": "Fresh outputs rotate the prior mutants.out directory, planned mutants are written separately, and completed outcomes are updated incrementally."
    },
    {
      "url": "https://docs.github.com/en/rest/actions/workflow-runs#list-workflow-runs-for-a-workflow",
      "finding": "The workflow-specific read endpoint supports the event/head_sha filters and queued, in_progress, waiting, requested and pending status filters. The bounded census checks each documented non-completed status."
    }
  ]
}
```

## Raw whole-change diff

```diff
diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
index a3e457575..2291a011c 100644
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -3873,12 +3873,11 @@ jobs:
           fi
 
   nightly-gate:
-    # [OPUS-4.8] Freshness gate for ALL nightly (schedule-triggered) jobs: run them
-    # only if HEAD changed since the last nightly run — so the heavy nightly tier does
-    # not re-run on an unchanged repo. Compares github.sha to the head_sha of the most
-    # recent COMPLETED schedule-triggered run of this workflow (ci.yml). A manual
-    # workflow_dispatch always passes; if the lookup fails or no prior nightly exists,
-    # we FAIL-OPEN (run) so a transient API error never silently disables the nightly.
+    # [GPT-6 Astra] #6436: skip only with verified heavy completion at this head.
+    # A green skipped follow-up is not proof. Incomplete work remains eligible on
+    # the next ordinary schedule; unreadable history blocks admission. Manual force
+    # remains explicit. No rerun API, shard retry or polling loop is introduced.
+    # The helper bounds history and job reads and never retries an API failure.
     if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'
     runs-on: ubuntu-latest
     permissions:
@@ -3887,22 +3886,14 @@ jobs:
     outputs:
       fresh: ${{ steps.gate.outputs.fresh }}
     steps:
+      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
+        with:
+          persist-credentials: false
+          sparse-checkout: scripts
       - id: gate
         env:
           GH_TOKEN: ${{ github.token }}
-        run: |
-          if [ "${{ github.event_name }}" = "workflow_dispatch" ]; then
-            echo "fresh=true" >> "$GITHUB_OUTPUT"; echo "manual dispatch — running nightly"; exit 0
-          fi
-          CUR='${{ github.sha }}'
-          PREV=$(gh api "repos/${{ github.repository }}/actions/workflows/ci.yml/runs?event=schedule&status=completed&per_page=1" \
-                   --jq '.workflow_runs[0].head_sha' 2>/dev/null || echo "")
-          echo "current HEAD=$CUR ; last nightly HEAD=${PREV:-<none>}"
-          if [ -z "$PREV" ] || [ "$PREV" != "$CUR" ]; then
-            echo "fresh=true" >> "$GITHUB_OUTPUT"; echo "new commit(s) since last nightly — running"
-          else
-            echo "fresh=false" >> "$GITHUB_OUTPUT"; echo "no new commit since last nightly ($PREV) — skipping"
-          fi
+        run: python3 scripts/ci_nightly_freshness.py
 
   coverage-nightly:
     # [OPUS-4.8] NIGHTLY heavy-coverage tier (sq-hbg7). Runs the FULL per-crate
@@ -3918,7 +3909,7 @@ jobs:
     # while the per-commit `coverage` job still gates the cheaper test-only `floor`. The
     # gate driver reads the summary's `tier` and applies nightly_floor automatically.
     # Triggered by the `schedule:` cron (or manual dispatch), and only when the
-    # nightly-gate says HEAD changed since the last nightly.
+    # nightly-gate finds no verified heavy completion at this HEAD (or manual force).
     name: coverage (nightly, full incl. heavy vectors)
     needs: nightly-gate
     if: needs.nightly-gate.outputs.fresh == 'true'
@@ -4421,9 +4412,18 @@ jobs:
           feat=()
           [ -n "$FEATURES" ] && feat=(--features "$FEATURES")
           echo "::group::cargo-mutants $CRATE ${feat[*]}${SHARD:+ --shard $SHARD}"
+          # [GPT-6 Astra] Preserve advisory handling, but do not let a masked command
+          # failure become evidence for skipping later heavy work at the same head.
+          mutation_exit=0
           cargo mutants -p "$CRATE" "${feat[@]}" "${shard_args[@]}" -o "$output_dir" \
-            || echo "WARN: cargo-mutants $CRATE did not complete cleanly (advisory)"
+            || mutation_exit=$?
+          if [ "$mutation_exit" -ne 0 ]; then
+            echo "WARN: cargo-mutants $CRATE did not complete cleanly (advisory)"
+          fi
           echo "::endgroup::"
+          # [GPT-6 Astra] Counts must cover the prewritten planned mutant list.
+          # Exit 2 (missed) / 3 (mutant timeouts) can be complete advisory results.
+          python3 scripts/ci_nightly_freshness.py --mutation-complete "$mutation_exit" "$output_dir/mutants.out"
           # Run the ratchet --check on THIS crate's outcomes. --check NEVER fails on an
           # UNSEEDED crate (it reports it for the next --seed); it exits 1 only if a SEEDED
           # crate now has MORE survivors than its committed ceiling. While advisory
@@ -4439,6 +4439,13 @@ jobs:
             echo "no outcomes produced for $CRATE${SHARD:+ shard=$SHARD} — nothing to check" \
               | tee -a "$GITHUB_STEP_SUMMARY"
           fi
+      # [GPT-6 Astra] This separate step is server-visible proof: continue-on-error
+      # may green the job, but cannot manufacture a successful completion marker.
+      # A complete measurement may still fail the advisory ratchet; do not change
+      # that policy or hide its original step result. Cancellation supplies no proof.
+      - name: Nightly mutation work completed
+        if: ${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'true' }}
+        run: echo "Mutation measurement completed; advisory ratchet verdict remains separate."
       - name: Upload mutation outcomes
         uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
         if: always()
diff --git a/.github/workflows/docs-quality.yml b/.github/workflows/docs-quality.yml
index 22359ebab..015d12aae 100644
--- a/.github/workflows/docs-quality.yml
+++ b/.github/workflows/docs-quality.yml
@@ -1082,6 +1082,10 @@ jobs:
       # (installed in the shared setup above).
       - name: Hosted-runner reservation for the miri/kani nightly lanes (issue 6349)
         run: python3 scripts/tests/test_nightly_runner_reservation.py
+      # [GPT-6 Astra] #6436: exercise the actual nightly admission helper and
+      # mutation evidence producer offline, including green skipped follow-ups.
+      - name: Heavy nightly completion freshness (issue 6436)
+        run: python3 scripts/tests/test_ci_nightly_freshness.py
       # [OPUS-5] sq-6vshe.7 phase 4: the HEAVY-set DRIFT alarm
       # (scripts/heavy_set_alarm.py, .github/workflows/heavy-set-alarm.yml). Hermetic
       # (no cargo, no gh, no network) and stdlib-only. Runs the script's own fixture
diff --git a/scripts/ci_nightly_freshness.py b/scripts/ci_nightly_freshness.py
new file mode 100644
index 000000000..278aa2451
--- /dev/null
+++ b/scripts/ci_nightly_freshness.py
@@ -0,0 +1,250 @@
+#!/usr/bin/env python3
+"""[GPT-6 Astra] Prove heavy nightly completion before skipping an unchanged head.
+
+Only scheduled ci.yml runs on main are evidence. A successful workflow with skipped
+heavy jobs is NOT evidence; follow it back to real work within the fixed read budget.
+Incomplete work remains eligible at the next ordinary schedule; there is no rerun API
+or retry loop. Unreadable evidence blocks admission. If proof ages out of the read
+window, a bounded active-run census permits periodic remeasurement. Manual dispatch
+remains an explicit force and makes no history requests.
+"""
+
+from __future__ import annotations
+
+import json
+import os
+from pathlib import Path
+import re
+import subprocess
+import sys
+from urllib.parse import urlencode
+
+HISTORY_LIMIT = 10
+JOB_PAGE_SIZE = 100
+JOB_PAGE_LIMIT = 2
+MUTATION_JOBS = 51  # Pinned to the live matrix by test_ci_nightly_freshness.py.
+COVERAGE = "coverage (nightly, full incl. heavy vectors)"
+COVERAGE_STEP = "Measure + enforce per-crate coverage (FULL tier, max-remeasure gate)"
+MUTATION = "mutation ratchet (cargo-mutants, advisory)"
+MUTATION_STEP = "Nightly mutation work completed"
+# [GPT-6 Astra] All non-completed statuses supported by the workflow-runs API.
+ACTIVE_STATUSES = ("queued", "in_progress", "waiting", "requested", "pending")
+
+
+class EvidenceError(Exception):
+    """No automatic admission or freshness claim is justified."""
+
+
+def positive_int(value):
+    return type(value) is int and value > 0
+
+
+def mutation_complete(exit_code, directory):
+    """Prove completion, independently of the existing advisory ratchet verdict.
+
+    cargo-mutants v25.3.1 src/{exit_code,outcome}.rs: 0/2/3 are completed
+    runs (caught/missed/mutant timeouts). Other exits do not prove execution.
+    mutants.json is written before testing; outcomes counts grow incrementally.
+    Their equality is necessary even for an accepted exit; partial files are not
+    evidence. A new tool schema fails closed until its contract is reviewed.
+    """
+    if exit_code not in (0, 2, 3):
+        return False
+    try:
+        planned = json.loads((Path(directory) / "mutants.json").read_text())
+        doc = json.loads((Path(directory) / "outcomes.json").read_text())
+    except (OSError, ValueError):
+        return False
+    if not isinstance(planned, list) or not isinstance(doc, dict):
+        return False
+    counts = [doc.get(key) for key in ("caught", "missed", "timeout", "unviable")]
+    if any(type(n) is not int or n < 0 for n in counts):
+        return False
+    return (type(doc.get("total_mutants")) is int
+            and doc["total_mutants"] == len(planned) == sum(counts)
+            and type(doc.get("success")) is int and doc["success"] == 0)
+
+
+def gh_json(endpoint):
+    """One bounded GET, no retries (including rate-limit/permission failures)."""
+    try:
+        result = subprocess.run(
+            ["gh", "api", "--method", "GET", endpoint], capture_output=True,
+            text=True, timeout=30, check=False,
+        )
+        if result.returncode:
+            raise EvidenceError(f"GitHub read failed (exit {result.returncode}); no retry")
+        return json.loads(result.stdout)
+    except (OSError, subprocess.TimeoutExpired, ValueError) as exc:
+        raise EvidenceError("GitHub evidence unavailable or unreadable; no retry") from exc
+
+
+def nightly_identity(run, repo, head):
+    return (isinstance(run, dict) and positive_int(run.get("id"))
+            and positive_int(run.get("run_attempt")) and run.get("event") == "schedule"
+            and run.get("head_sha") == head and run.get("head_branch") == "main"
+            and run.get("path") == ".github/workflows/ci.yml"
+            and isinstance(run.get("repository"), dict)
+            and run["repository"].get("full_name") == repo)
+
+
+def ensure_no_active(get, repo, head, current_id):
+    """Five GETs maximum, no pagination/retries. Check beyond the history window.
+
+    Two rows suffice: the current run and any competing run. Larger, malformed,
+    filtered-status-mismatched or incomplete inventories cannot authorize work.
+    """
+    for status in ACTIVE_STATUSES:
+        query = urlencode({"event": "schedule", "head_sha": head,
+                           "status": status, "per_page": 2})
+        doc = get(f"repos/{repo}/actions/workflows/ci.yml/runs?{query}")
+        if (not isinstance(doc, dict) or type(doc.get("total_count")) is not int
+                or not isinstance(doc.get("workflow_runs"), list)
+                or not 0 <= doc["total_count"] <= 2
+                or len(doc["workflow_runs"]) != doc["total_count"]):
+            raise EvidenceError("active-run census malformed or truncated")
+        seen = set()
+        for run in doc["workflow_runs"]:
+            if (not nightly_identity(run, repo, head) or run["id"] in seen
+                    or run.get("status") != status or run.get("conclusion") is not None):
+                raise EvidenceError("active-run census identity or status unreadable")
+            seen.add(run["id"])
+            if run["id"] != current_id:
+                raise EvidenceError("another same-head schedule is active")
+
+
+def read_jobs(get, repo, run):
+    jobs = []
+    expected = None
+    for page in range(1, JOB_PAGE_LIMIT + 1):
+        doc = get(f"repos/{repo}/actions/runs/{run['id']}/attempts/"
+                  f"{run['run_attempt']}/jobs?per_page={JOB_PAGE_SIZE}&page={page}")
+        if not isinstance(doc, dict) or type(doc.get("total_count")) is not int:
+            raise EvidenceError("missing job inventory")
+        total, batch = doc["total_count"], doc.get("jobs")
+        if (total <= 0 or total > JOB_PAGE_SIZE * JOB_PAGE_LIMIT
+                or not isinstance(batch, list) or len(batch) > JOB_PAGE_SIZE
+                or (expected is not None and total != expected)):
+            raise EvidenceError("unbounded or inconsistent job inventory")
+        expected = total
+        jobs.extend(batch)
+        if len(jobs) == total:
+            break
+        if len(batch) != JOB_PAGE_SIZE or len(jobs) > total:
+            raise EvidenceError("truncated job inventory")
+    ids = set()
+    for job in jobs:
+        if (not isinstance(job, dict) or not positive_int(job.get("id"))
+                or job["id"] in ids or job.get("run_id") != run["id"]
+                or job.get("run_attempt") != run["run_attempt"]
+                or job.get("head_sha") != run["head_sha"]):
+            raise EvidenceError("job identity mismatch")
+        ids.add(job["id"])
+    return jobs
+
+
+def successful_step(job, name):
+    steps = job.get("steps")
+    if not isinstance(steps, list) or any(not isinstance(s, dict) for s in steps):
+        raise EvidenceError("heavy step inventory unreadable")
+    matches = [s for s in steps if s.get("name") == name]
+    return (len(matches) == 1 and matches[0].get("status") == "completed"
+            and matches[0].get("conclusion") == "success")
+
+
+def heavy_state(jobs):
+    coverage = [j for j in jobs if j.get("name") == COVERAGE]
+    mutations = [j for j in jobs if isinstance(j.get("name"), str)
+                 and (j["name"] == MUTATION or j["name"].startswith(MUTATION + " ("))]
+    if len(coverage) != 1 or not mutations:
+        return "incomplete"
+    heavy = coverage + mutations
+    # [GPT-6 Astra] GitHub represents a job skipped before matrix expansion with
+    # one base-name placeholder. Never accept a partial/mixed matrix as a skip.
+    if (len(mutations) == 1 and mutations[0]["name"] == MUTATION
+            and all(j.get("status") == "completed" and j.get("conclusion") == "skipped"
+                    for j in heavy)):
+        return "skipped"
+    if (len(mutations) != MUTATION_JOBS
+            or len({j["name"] for j in mutations}) != MUTATION_JOBS
+            or any(j["name"] == MUTATION for j in mutations)
+            or any(j.get("status") != "completed" or j.get("conclusion") != "success"
+                   for j in heavy)
+            or not successful_step(coverage[0], COVERAGE_STEP)
+            or not all(successful_step(j, MUTATION_STEP) for j in mutations)):
+        return "incomplete"
+    return "complete"
+
+
+def decide(event, repo, head, current_id, get=gh_json):
+    """Return (run_heavy, reason); only this ordinary schedule may admit new work."""
+    if event == "workflow_dispatch":
+        return True, "manual dispatch explicitly requests heavy work"
+    if event != "schedule":
+        return False, "heavy nightly work is isolated from this event"
+    if (not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo)
+            or not re.fullmatch(r"[0-9a-f]{40}", head) or not positive_int(current_id)):
+        raise EvidenceError("invalid scheduled run identity")
+    query = urlencode({"event": "schedule", "head_sha": head, "per_page": HISTORY_LIMIT})
+    doc = get(f"repos/{repo}/actions/workflows/ci.yml/runs?{query}")
+    if (not isinstance(doc, dict) or type(doc.get("total_count")) is not int
+            or doc["total_count"] < 0 or not isinstance(doc.get("workflow_runs"), list)):
+        raise EvidenceError("missing scheduled history")
+    runs = doc["workflow_runs"]
+    if len(runs) != min(doc["total_count"], HISTORY_LIMIT):
+        raise EvidenceError("truncated scheduled history")
+    seen = set()
+    for run in runs:
+        if not nightly_identity(run, repo, head) or run["id"] in seen:
+            raise EvidenceError("scheduled history identity mismatch")
+        seen.add(run["id"])
+    # [GPT-6 Astra] Run IDs order creation, not the update time of an old rerun.
+    # A newer same-head schedule means this tick is stale; do not start more work.
+    if any(run["id"] > current_id for run in runs):
+        raise EvidenceError("newer same-head schedule exists")
+    prior = sorted((r for r in runs if r["id"] != current_id),
+                   key=lambda r: r["id"], reverse=True)
+    if any(r.get("status") != "completed" for r in prior):
+        raise EvidenceError("same-head schedule still active or status unreadable")
+    for run in prior:
+        if run.get("conclusion") not in ("success", "failure", "cancelled", "timed_out"):
+            raise EvidenceError("scheduled conclusion unreadable or unsupported")
+        state = heavy_state(read_jobs(get, repo, run))
+        if state == "complete":
+            return False, f"heavy completion verified in run {run['id']} attempt {run['run_attempt']}"
+        if state == "incomplete":
+            if doc["total_count"] > HISTORY_LIMIT:
+                ensure_no_active(get, repo, head, current_id)
+            return True, f"heavy work incomplete in run {run['id']}; admit this ordinary schedule"
+    if doc["total_count"] > HISTORY_LIMIT:
+        ensure_no_active(get, repo, head, current_id)
+        return True, "completion proof aged out; admit periodic measurement on this ordinary schedule"
+    if prior:
+        return True, "only skipped follow-ups exist; this schedule must perform the heavy work"
+    return True, "no prior same-head schedule; admit its first heavy attempt"
+
+
+def main():
+    if len(sys.argv) == 4 and sys.argv[1] == "--mutation-complete":
+        complete = mutation_complete(int(sys.argv[2]), sys.argv[3])
+        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
+            output.write(f"measurement_completed={str(complete).lower()}\n")
+        print(f"Mutation completion evidence: {str(complete).lower()}")
+        return 0  # Evidence only; preserve the existing advisory command/ratchet policy.
+    fresh = False
+    try:
+        fresh, reason = decide(os.environ.get("GITHUB_EVENT_NAME", ""),
+                               os.environ.get("GITHUB_REPOSITORY", ""),
+                               os.environ.get("GITHUB_SHA", ""),
+                               int(os.environ.get("GITHUB_RUN_ID", "0")))
+        code = 0
+    except (EvidenceError, ValueError, TypeError) as exc:
+        reason, code = f"nightly admission blocked: {exc}", 1
+    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
+        output.write(f"fresh={str(fresh).lower()}\n")
+    print(reason)
+    return code
+
+
+if __name__ == "__main__":
+    sys.exit(main())
diff --git a/scripts/tests/test_ci_nightly_freshness.py b/scripts/tests/test_ci_nightly_freshness.py
new file mode 100644
index 000000000..47e8a1b13
--- /dev/null
+++ b/scripts/tests/test_ci_nightly_freshness.py
@@ -0,0 +1,537 @@
+#!/usr/bin/env python3
+"""[GPT-6 Astra] Offline behavior, real producer and CI wiring for issue #6436."""
+
+from __future__ import annotations
+
+import copy
+import importlib.util
+import json
+import os
+from pathlib import Path
+import subprocess
+import sys
+import tempfile
+import unittest
+from unittest.mock import patch
+from urllib.parse import parse_qs, urlsplit
+
+import yaml
+
+ROOT = Path(__file__).resolve().parents[2]
+SPEC = importlib.util.spec_from_file_location("freshness", ROOT / "scripts/ci_nightly_freshness.py")
+freshness = importlib.util.module_from_spec(SPEC)
+SPEC.loader.exec_module(freshness)
+REPO = "sparq-org/sparq"
+HEAD = "a" * 40
+CURRENT = 1000
+
+
+def run(run_id=999, **changes):
+    item = dict(id=run_id, run_attempt=1, event="schedule", head_sha=HEAD,
+                head_branch="main", path=".github/workflows/ci.yml",
+                repository={"full_name": REPO}, status="completed", conclusion="success")
+    item.update(changes)
+    return item
+
+
+def job(name, index, step=None, **changes):
+    item = dict(id=index, run_id=999, run_attempt=1, head_sha=HEAD,
+                name=name, status="completed", conclusion="success", steps=[])
+    if step:
+        item["steps"] = [dict(name=step, status="completed", conclusion="success")]
+    item.update(changes)
+    return item
+
+
+def completed_jobs():
+    return ([job(freshness.COVERAGE, 1, freshness.COVERAGE_STEP)]
+            + [job(f"{freshness.MUTATION} (shard-{i})", i + 2, freshness.MUTATION_STEP)
+               for i in range(freshness.MUTATION_JOBS)])
+
+
+def skipped_jobs():
+    return [job(freshness.COVERAGE, 1, conclusion="skipped"),
+            job(freshness.MUTATION, 2, conclusion="skipped")]
+
+
+class FakeAPI:
+    def __init__(self, runs=None, jobs=None, history=None, active=None):
+        self.history = history if history is not None else {
+            "total_count": len(runs or []), "workflow_runs": runs or []}
+        self.jobs = jobs if jobs is not None else {999: completed_jobs()}
+        self.calls = []
+        self.active = active or {}
+
+    def __call__(self, endpoint):
+        self.calls.append(endpoint)
+        if "/workflows/" in endpoint:
+            status = parse_qs(urlsplit(endpoint).query).get("status")
+            if status:
+                result = self.active.get(status[0], {"total_count": 0, "workflow_runs": []})
+                if isinstance(result, Exception):
+                    raise result
+                return copy.deepcopy(result)
+            return copy.deepcopy(self.history)
+        parts = endpoint.split("/")
+        run_id = int(parts[5])
+        self.assert_attempt(parts, run_id)
+        jobs = self.jobs[run_id]
+        page = int(endpoint.rsplit("page=", 1)[1])
+        return {"total_count": len(jobs), "jobs": copy.deepcopy(jobs[(page - 1) * 100:page * 100])}
+
+    def assert_attempt(self, parts, run_id):
+        expected = next(r["run_attempt"] for r in self.history["workflow_runs"] if r["id"] == run_id)
+        assert parts[6:8] == ["attempts", str(expected)], parts
+
+
+class Admission(unittest.TestCase):
+    def decide(self, api, event="schedule"):
+        return freshness.decide(event, REPO, HEAD, CURRENT, api)
+
+    def blocked(self, api, pattern=None):
+        with self.assertRaisesRegex(freshness.EvidenceError, pattern or "."):
+            self.decide(api)
+
+    def test_first_schedule_at_new_head_runs_once(self):
+        for history in [[], [run(CURRENT, status="in_progress", conclusion=None)]]:
+            with self.subTest(history=history):
+                api = FakeAPI(history)
+                self.assertTrue(self.decide(api)[0])
+                self.assertEqual(len(api.calls), 1)
+                self.assertIn("head_sha=" + HEAD, api.calls[0])
+                self.assertNotIn("status=completed", api.calls[0])
+
+    def test_verified_same_head_skips(self):
+        api = FakeAPI([run()])
+        self.assertFalse(self.decide(api)[0])
+        self.assertEqual(len(api.calls), 2)
+
+    def test_failed_cancelled_incomplete_work_remains_eligible_next_schedule(self):
+        for conclusion in ["failure", "cancelled", "timed_out", "success"]:
+            with self.subTest(conclusion=conclusion):
+                jobs = completed_jobs()
+                jobs[-1]["conclusion"] = "cancelled"
+                api = FakeAPI([run(conclusion=conclusion)], {999: jobs})
+                self.assertTrue(self.decide(api)[0])
+                self.assertEqual(len(api.calls), 2)
+
+    def test_active_or_unreadable_same_head_never_admits(self):
+        for status, conclusion in [("in_progress", None), ("queued", None), ("completed", None)]:
+            with self.subTest(status=status, conclusion=conclusion):
+                api = FakeAPI([run(status=status, conclusion=conclusion)])
+                self.blocked(api)
+                self.assertEqual(len(api.calls), 1)
+
+    def test_manual_force_and_other_events_never_read_history(self):
+        for event in ["workflow_dispatch", "merge_group", "push", "pull_request", "unknown", ""]:
+            with self.subTest(event=event):
+                api = FakeAPI([run(conclusion="cancelled")])
+                self.assertEqual(self.decide(api, event)[0], event == "workflow_dispatch")
+                self.assertEqual(api.calls, [])
+
+    def test_skipped_followup_needs_real_older_proof(self):
+        old = completed_jobs()
+        for item in old:
+            item["run_id"] = 998
+        api = FakeAPI([run(), run(998)], {999: skipped_jobs(), 998: old})
+        self.assertFalse(self.decide(api)[0])
+        self.assertIn("998 attempt 1", self.decide(api)[1])
+        self.assertTrue(self.decide(FakeAPI([run()], {999: skipped_jobs()}))[0])
+
+    def test_green_skipped_followup_cannot_launder_failed_cancelled_run(self):
+        for conclusion in ["failure", "cancelled"]:
+            with self.subTest(conclusion=conclusion):
+                incomplete = completed_jobs()
+                for item in incomplete:
+                    item["run_id"] = 998
+                incomplete[-1]["steps"] = []
+                api = FakeAPI([run(), run(998, conclusion=conclusion)],
+                              {999: skipped_jobs(), 998: incomplete})
+                self.assertTrue(self.decide(api)[0])
+                self.assertEqual(len(api.calls), 3)
+
+    def test_missing_unreadable_or_truncated_history_blocks(self):
+        for doc in [None, {}, [], {"total_count": True, "workflow_runs": []},
+                    {"total_count": 1, "workflow_runs": []},
+                    {"total_count": 0, "workflow_runs": [run()]}]:
+            with self.subTest(doc=doc):
+                self.blocked(lambda endpoint: doc)
+
+    def test_identity_and_different_head_evidence_rejected(self):
+        for changes in [{"head_sha": "b" * 40}, {"head_branch": "feature"},
+                        {"event": "workflow_dispatch"}, {"path": ".github/workflows/other.yml"},
+                        {"repository": {"full_name": "elsewhere/repo"}}, {"repository": None},
+                        {"run_attempt": 0}, {"id": True}]:
+            with self.subTest(changes=changes):
+                self.blocked(FakeAPI([run(**changes)]), "identity")
+        self.blocked(FakeAPI([run(), run()]), "identity")
+
+    def test_stale_schedule_does_not_compete_with_newer_tick(self):
+        self.blocked(FakeAPI([run(CURRENT + 1)]), "newer")
+
+    def test_invalid_input_identity_never_reads_github(self):
+        for repo, head, current in [("bad repo", HEAD, CURRENT), (REPO, "short", CURRENT),
+                                    (REPO, HEAD, 0)]:
+            with self.subTest(repo=repo, head=head, current=current):
+                api = FakeAPI()
+                with self.assertRaises(freshness.EvidenceError):
+                    freshness.decide("schedule", repo, head, current, api)
+                self.assertEqual(api.calls, [])
+
+    def test_unreadable_step_inventory_blocks_admission(self):
+        for steps in [None, [None], [{"name": freshness.COVERAGE_STEP}]]:
+            with self.subTest(steps=steps):
+                jobs = completed_jobs()
+                jobs[0]["steps"] = steps
+                if steps in (None, [None]):
+                    self.blocked(FakeAPI([run()], {999: jobs}), "inventory unreadable")
+                else:
+                    self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])
+
+    def test_attempt_scoped_job_lookup_and_mismatched_attempt_blocks(self):
+        jobs = completed_jobs()
+        for item in jobs:
+            item["run_attempt"] = 2
+        api = FakeAPI([run(run_attempt=2)], {999: jobs})
+        self.assertFalse(self.decide(api)[0])
+        self.assertIn("/attempts/2/jobs?", api.calls[-1])
+        jobs[0]["run_attempt"] = 1
+        self.blocked(FakeAPI([run(run_attempt=2)], {999: jobs}), "identity")
+
+    def test_each_heavy_leg_and_marker_is_required(self):
+        for index in range(len(completed_jobs())):
+            for change in ["absent", "failure", "cancelled", "skipped", "missing_step", "failed_step"]:
+                with self.subTest(index=index, change=change):
+                    jobs = completed_jobs()
+                    if change == "absent":
+                        jobs.pop(index)
+                    elif change == "missing_step":
+                        jobs[index]["steps"] = []
+                    elif change == "failed_step":
+                        jobs[index]["steps"][0]["conclusion"] = "failure"
+                    else:
+                        jobs[index]["conclusion"] = change
+                    self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])
+
+    def test_duplicate_wrong_head_wrong_run_jobs_cannot_fill_matrix(self):
+        for field, value in [("id", 1), ("name", completed_jobs()[1]["name"]),
+                             ("head_sha", "b" * 40), ("run_id", 998)]:
+            with self.subTest(field=field):
+                jobs = completed_jobs()
+                jobs[-1][field] = value
+                if field == "name":
+                    self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])
+                else:
+                    self.blocked(FakeAPI([run()], {999: jobs}))
+
+    def test_mixed_skipped_matrix_and_missing_placeholders_block(self):
+        for jobs in [skipped_jobs()[:1], skipped_jobs()[1:],
+                     skipped_jobs() + [job(freshness.MUTATION + " (extra)", 3)],
+                     [job(freshness.COVERAGE, 1), skipped_jobs()[1]]]:
+            with self.subTest(jobs=jobs):
+                self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])
+
+    def test_full_inventory_pagination_and_hard_bound(self):
+        jobs = completed_jobs() + [job(f"other-{i}", 100 + i) for i in range(100)]
+        api = FakeAPI([run()], {999: jobs})
+        self.assertFalse(self.decide(api)[0])
+        self.assertEqual(len(api.calls), 3)
+        jobs += [job(f"excess-{i}", 300 + i) for i in range(100)]
+        api = FakeAPI([run()], {999: jobs})
+        self.blocked(api, "unbounded")
+        self.assertEqual(len(api.calls), 2)
+
+    def test_truncated_job_page_and_missing_inventory_block(self):
+        for doc in [{}, {"total_count": 100, "jobs": []},
+                    {"total_count": 1, "jobs": completed_jobs()},
+                    {"total_count": True, "jobs": []}]:
+            with self.subTest(doc=doc):
+                self.blocked(lambda endpoint: ({"total_count": 1, "workflow_runs": [run()]}
+                                               if "/workflows/" in endpoint else doc))
+
+    def aged_out_api(self, active=None):
+        runs = [run(999 - i) for i in range(freshness.HISTORY_LIMIT)]
+        jobs = {}
+        for item in runs:
+            jobs[item["id"]] = skipped_jobs()
+            for entry in jobs[item["id"]]:
+                entry["run_id"] = item["id"]
+        api = FakeAPI(runs, jobs, active=active)
+        api.history["total_count"] = 100
+        return api
+
+    def test_aged_out_proof_allows_bounded_periodic_measurement(self):
+        api = self.aged_out_api()
+        self.assertTrue(self.decide(api)[0])
+        self.assertEqual(len(api.calls), 16)
+        statuses = [parse_qs(urlsplit(call).query)["status"][0] for call in api.calls[-5:]]
+        self.assertEqual(statuses, ["queued", "in_progress", "waiting", "requested", "pending"])
+        self.assertTrue(all("per_page=2" in call and "head_sha=" + HEAD in call
+                            and "event=schedule" in call for call in api.calls[-5:]))
+
+    def test_active_run_older_than_window_blocks_every_supported_status(self):
+        for status in ["queued", "in_progress", "waiting", "requested", "pending"]:
+            with self.subTest(status=status):
+                other = run(50, status=status, conclusion=None)
+                api = self.aged_out_api({status: {"total_count": 1, "workflow_runs": [other]}})
+                self.blocked(api, "another same-head schedule is active")
+                self.assertLessEqual(len(api.calls), 16)
+
+    def test_census_api_failure_stops_without_retry(self):
+        api = self.aged_out_api({"queued": freshness.EvidenceError("API rate limit exceeded")})
+        self.blocked(api, "rate limit")
+        self.assertEqual(len(api.calls), 12)
+
+    def test_census_malformed_truncated_unknown_rows_block(self):
+        current = run(CURRENT, status="queued", conclusion=None)
+        for doc in [None, {}, {"total_count": True, "workflow_runs": []},
+                    {"total_count": 3, "workflow_runs": []}, {"total_count": 1, "workflow_runs": []},
+                    {"total_count": 2, "workflow_runs": [current, current]},
+                    {"total_count": 1, "workflow_runs": [run(CURRENT, status="unknown", conclusion=None)]},
+                    {"total_count": 1, "workflow_runs": [dict(current, head_sha="b" * 40)]},
+                    {"total_count": 1, "workflow_runs": [dict(current, conclusion="success")]}]:
+            with self.subTest(doc=doc):
+                api = self.aged_out_api({"queued": doc})
+                self.blocked(api, "census")
+                self.assertEqual(len(api.calls), 12)
+
+    def test_census_excludes_only_current_run(self):
+        current = run(CURRENT, status="queued", conclusion=None)
+        api = self.aged_out_api({"queued": {"total_count": 1, "workflow_runs": [current]}})
+        freshness.ensure_no_active(api, REPO, HEAD, CURRENT)
+        self.assertEqual(len(api.calls), 5)
+        api.calls.clear()
+        self.assertTrue(self.decide(api)[0])
+        self.assertEqual(len(api.calls), 16)
+
+    def test_truncated_incomplete_history_also_checks_older_activity(self):
+        other = run(50, status="queued", conclusion=None)
+        api = self.aged_out_api({"queued": {"total_count": 1, "workflow_runs": [other]}})
+        api.jobs[999] = completed_jobs()
+        api.jobs[999][-1]["steps"] = []
+        self.blocked(api, "another same-head schedule is active")
+        self.assertEqual(len(api.calls), 3)
+
+    def test_api_failure_json_and_timeout_each_make_one_read(self):
+        failures = [subprocess.CompletedProcess([], 1, "", "API rate limit exceeded"),
+                    subprocess.CompletedProcess([], 0, "not json", ""),
+                    subprocess.TimeoutExpired("gh", 30), OSError("unavailable")]
+        for failure in failures:
+            with self.subTest(failure=failure):
+                kwargs = ({"side_effect": failure} if isinstance(failure, Exception)
+                          else {"return_value": failure})
+                with patch.object(freshness.subprocess, "run", **kwargs) as call:
+                    with self.assertRaises(freshness.EvidenceError):
+                        freshness.gh_json("repos/sparq-org/sparq/actions/workflows/ci.yml/runs")
+                    self.assertEqual(call.call_count, 1)
+                    self.assertEqual(call.call_args.kwargs["timeout"], 30)
+                    self.assertEqual(call.call_args.args[0][1:4], ["api", "--method", "GET"])
+
+    def test_cli_error_emits_false_and_failure(self):
+        with tempfile.TemporaryDirectory() as directory:
+            output = Path(directory) / "output"
+            with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_RUN_ID": "bad",
+                                         "GITHUB_OUTPUT": str(output)}):
+                self.assertEqual(freshness.main(), 1)
+            self.assertEqual(output.read_text(), "fresh=false\n")
+
+    def test_cli_schedule_emits_actual_decision_and_never_writes_api(self):
+        for history, expected in [([], "true"), ([run()], "false")]:
+            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as directory:
+                output = Path(directory) / "output"
+                api = FakeAPI(history)
+                def command(argv, **kwargs):
+                    self.assertEqual(argv[:4], ["gh", "api", "--method", "GET"])
+                    return subprocess.CompletedProcess(argv, 0, json.dumps(api(argv[-1])), "")
+                with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_REPOSITORY": REPO,
+                                             "GITHUB_SHA": HEAD, "GITHUB_RUN_ID": str(CURRENT),
+                                             "GITHUB_OUTPUT": str(output)}), \
+                        patch.object(freshness.subprocess, "run", side_effect=command):
+                    self.assertEqual(freshness.main(), 0)
+                self.assertEqual(output.read_text(), f"fresh={expected}\n")
+
+    def test_cli_unreadable_evidence_emits_false_and_stops(self):
+        with tempfile.TemporaryDirectory() as directory:
+            output = Path(directory) / "output"
+            with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_REPOSITORY": REPO,
+                                         "GITHUB_SHA": HEAD, "GITHUB_RUN_ID": str(CURRENT),
+                                         "GITHUB_OUTPUT": str(output)}), \
+                    patch.object(freshness.subprocess, "run", return_value=
+                                 subprocess.CompletedProcess([], 1, "", "API rate limit exceeded")) as call:
+                self.assertEqual(freshness.main(), 1)
+                self.assertEqual(call.call_count, 1)
+            self.assertEqual(output.read_text(), "fresh=false\n")
+
+
+class ProductionWiring(unittest.TestCase):
+    @classmethod
+    def setUpClass(cls):
+        cls.workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
+        cls.jobs = cls.workflow["jobs"]
+        cls.mutation = cls.jobs["mutants-nightly-advisory"]
+        cls.producer = next(s for s in cls.mutation["steps"] if s.get("id") == "run_mutants")
+
+    def test_schedule_manual_boundary_and_unchanged_cost_limits(self):
+        gate = self.jobs["nightly-gate"]
+        self.assertEqual(gate["if"], "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'")
+        self.assertEqual(gate["permissions"], {"actions": "read", "contents": "read"})
+        checkout = gate["steps"][0]
+        self.assertTrue(checkout["uses"].startswith("actions/checkout@"))
+        self.assertFalse(checkout["with"]["persist-credentials"])
+        self.assertEqual(gate["steps"][-1]["run"], "python3 scripts/ci_nightly_freshness.py")
+        for key in ["coverage-nightly", "mutants-nightly-advisory"]:
+            self.assertEqual(self.jobs[key]["needs"], "nightly-gate")
+            self.assertEqual(self.jobs[key]["if"], "needs.nightly-gate.outputs.fresh == 'true'")
+        self.assertEqual(self.mutation["strategy"]["max-parallel"], 8)
+        self.assertFalse(self.mutation["strategy"]["fail-fast"])
+        self.assertTrue(self.mutation["continue-on-error"])
+        self.assertTrue(self.producer["continue-on-error"])
+        self.assertEqual(self.jobs["coverage-nightly"]["timeout-minutes"], 60)
+        self.assertEqual((freshness.HISTORY_LIMIT, freshness.JOB_PAGE_SIZE, freshness.JOB_PAGE_LIMIT),
+                         (10, 100, 2))
+        self.assertEqual(freshness.ACTIVE_STATUSES,
+                         ("queued", "in_progress", "waiting", "requested", "pending"))
+
+    def test_expected_matrix_count_matches_live_expansion(self):
+        matrix = self.mutation["strategy"]["matrix"]
+        self.assertEqual(set(matrix), {"crate", "include"})
+        base = [{"crate": crate} for crate in matrix["crate"]]
+        extra = []
+        for include in matrix["include"]:
+            matches = [leg for leg in base if leg["crate"] == include["crate"]]
+            if matches:
+                for leg in matches:
+                    leg.update(include)
+            else:
+                extra.append(include)
+        legs = base + extra
+        self.assertEqual(len(legs), freshness.MUTATION_JOBS)
+        self.assertEqual(len({(x["crate"], x.get("shard", "")) for x in legs}), len(legs))
+        self.assertEqual(self.mutation["name"], freshness.MUTATION)
+        coverage = self.jobs["coverage-nightly"]
+        self.assertEqual(coverage["name"], freshness.COVERAGE)
+        self.assertEqual(sum(s.get("name") == freshness.COVERAGE_STEP for s in coverage["steps"]), 1)
+
+    def test_marker_requires_unmasked_success_and_ci_runs_suite(self):
+        marker = next(s for s in self.mutation["steps"] if s.get("name") == freshness.MUTATION_STEP)
+        self.assertEqual(marker["if"], "${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'true' }}")
+        self.assertNotIn("continue-on-error", marker)
+        docs = yaml.safe_load((ROOT / ".github/workflows/docs-quality.yml").read_text())
+        self.assertTrue(any(s.get("run") == "python3 scripts/tests/test_ci_nightly_freshness.py"
+                            for s in docs["jobs"]["quick-gates"]["steps"]))
+
+    def producer_result(self, cargo_exit=0, ratchet_exit=0, outcomes=True, planned=3):
+        # [GPT-6 Astra] Execute the actual YAML shell under GitHub's bash -e mode.
+        # PATH contains only explicit stubs plus system utilities; no real cargo,
+        # Python ratchet, network, build or GitHub request is reachable.
+        with tempfile.TemporaryDirectory() as directory:
+            root = Path(directory)
+            bin_dir = root / "bin"
+            bin_dir.mkdir()
+            (root / "scripts").mkdir()
+            (root / "scripts/ci_nightly_freshness.py").write_text(
+                (ROOT / "scripts/ci_nightly_freshness.py").read_text())
+            cargo = bin_dir / "cargo"
+            cargo.write_text("#!/bin/sh\nif [ \"$FIXTURE_OUTCOMES\" = true ]; then\n"
+                             "mkdir -p target/mutants/test/mutants.out\n"
+                             "echo '{\"caught\":1,\"missed\":1,\"timeout\":1,\"unviable\":0,\"success\":0,\"total_mutants\":3}' > target/mutants/test/mutants.out/outcomes.json\n"
+                             "echo \"$FIXTURE_PLANNED\" > target/mutants/test/mutants.out/mutants.json\nfi\n"
+                             "exit \"$FIXTURE_CARGO_EXIT\"\n")
+            ratchet = bin_dir / "python3"
+            ratchet.write_text("#!/bin/sh\nif [ \"$1\" = scripts/ci_nightly_freshness.py ]; then\n"
+                               "exec \"$FIXTURE_PYTHON\" \"$@\"\nfi\nexit \"$FIXTURE_RATCHET_EXIT\"\n")
+            for path in [cargo, ratchet]:
+                path.chmod(0o755)
+            output = root / "output"
+            env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "CRATE": "test", "FEATURES": "", "SHARD": "",
+                   "GITHUB_OUTPUT": str(output), "GITHUB_STEP_SUMMARY": str(root / "summary"),
+                   "FIXTURE_CARGO_EXIT": str(cargo_exit), "FIXTURE_RATCHET_EXIT": str(ratchet_exit),
+                   "FIXTURE_OUTCOMES": str(outcomes).lower(), "FIXTURE_PYTHON": sys.executable,
+                   "FIXTURE_PLANNED": json.dumps([{}] * planned)}
+            result = subprocess.run(["/bin/bash", "--noprofile", "--norc", "-eo", "pipefail", "-c",
+                                     self.producer["run"]], cwd=root, env=env,
+                                    capture_output=True, text=True, timeout=10)
+            return result.returncode, output.read_text()
+
+    def test_actual_producer_clean_measurement_supplies_evidence(self):
+        code, output = self.producer_result()
+        self.assertEqual(code, 0)
+        self.assertIn("measurement_completed=true\n", output)
+
+    def test_actual_producer_masked_cargo_failure_never_supplies_evidence(self):
+        for exit_code in [1, 4, 5, 6, 70, 124, 137]:
+            with self.subTest(exit_code=exit_code):
+                code, output = self.producer_result(cargo_exit=exit_code)
+                self.assertEqual(code, 0)  # Existing advisory behavior is preserved.
+                self.assertNotIn("measurement_completed=true", output)
+
+    def test_actual_producer_missing_outcomes_never_supplies_evidence(self):
+        code, output = self.producer_result(outcomes=False)
+        self.assertEqual(code, 0)
+        self.assertNotIn("measurement_completed=true", output)
+
+    def test_actual_producer_completed_advisory_results_supply_evidence(self):
+        for exit_code in [2, 3]:
+            with self.subTest(exit_code=exit_code):
+                code, output = self.producer_result(cargo_exit=exit_code)
+                self.assertEqual(code, 0)
+                self.assertIn("measurement_completed=true\n", output)
+
+    def test_actual_producer_partial_outcomes_never_supply_evidence(self):
+        code, output = self.producer_result(planned=4)
+        self.assertEqual(code, 0)
+        self.assertNotIn("measurement_completed=true", output)
+
+    def test_actual_producer_failed_advisory_ratchet_keeps_completion_evidence(self):
+        code, output = self.producer_result(ratchet_exit=1)
+        self.assertEqual(code, 1)
+        self.assertIn("measurement_completed=true\n", output)
+
+
+class MutationCompletion(unittest.TestCase):
+    def check(self, doc, planned=None, exit_code=0):
+        with tempfile.TemporaryDirectory() as directory:
+            root = Path(directory)
+            if planned is not None:
+                (root / "mutants.json").write_text(json.dumps(planned))
+            (root / "outcomes.json").write_text(json.dumps(doc))
+            return freshness.mutation_complete(exit_code, directory)
+
+    def test_complete_counts_include_missed_timeout_and_unviable(self):
+        doc = dict(caught=1, missed=2, timeout=3, unviable=4, success=0, total_mutants=10)
+        for exit_code in [0, 2, 3]:
+            with self.subTest(exit_code=exit_code):
+                self.assertTrue(self.check(doc, [{}] * 10, exit_code))
+        for exit_code in [1, 4, 5, 6, 70, 124, 137]:
+            with self.subTest(exit_code=exit_code):
+                self.assertFalse(self.check(doc, [{}] * 10, exit_code))
+
+    def test_mutation_cli_records_false_without_changing_advisory_exit(self):
+        with tempfile.TemporaryDirectory() as directory:
+            output = Path(directory) / "output"
+            with patch.object(sys, "argv", ["ci_nightly_freshness.py", "--mutation-complete", "4", directory]), \
+                    patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
+                self.assertEqual(freshness.main(), 0)
+            self.assertEqual(output.read_text(), "measurement_completed=false\n")
+
+    def test_missing_malformed_partial_or_inconsistent_counts_are_not_evidence(self):
+        doc = dict(caught=1, missed=0, timeout=0, unviable=0, success=0, total_mutants=1)
+        for key, value in [("caught", True), ("caught", -1), ("missed", "0"),
+                           ("total_mutants", 0), ("success", 1), ("success", False)]:
+            with self.subTest(key=key, value=value):
+                bad = dict(doc, **{key: value})
+                self.assertFalse(self.check(bad, [{}]))
+        for planned in [None, {}, [], [{}, {}]]:
+            self.assertFalse(self.check(doc, planned))
+        for bad in [None, [], {}, {"caught": 1}]:
+            self.assertFalse(self.check(bad, [{}]))
+
+    def test_invalid_json_or_missing_outcomes_are_not_evidence(self):
+        with tempfile.TemporaryDirectory() as directory:
+            self.assertFalse(freshness.mutation_complete(0, directory))
+            (Path(directory) / "mutants.json").write_text("not json")
+            self.assertFalse(freshness.mutation_complete(0, directory))
+
+
+if __name__ == "__main__":
+    unittest.main()

```

## Complete executable nightly workflow context

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

## CI test admission and dependency wiring

The docs-quality quick-gates job and added test step have no conditional or continue-on-error. Existing sibling hard gates may halt the sequential job on failure; the suite runs once preceding required steps succeed. Existing pinned dependency installation supplies PyYAML. This new test executes only synthetic/offline fixtures even when docs-quality itself runs on merge_group; the heavy jobs remain excluded from merge_group.

```yaml
name: docs-quality
permissions:
  contents: read
concurrency:
  group: docs-quality-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: true
'on':
  pull_request:
    types:
    - opened
    - synchronize
    - reopened
    - ready_for_review
  merge_group: null
  push:
    branches:
    - main
quick-gates:
  name: docs-quality quick-gates
  runs-on: ubuntu-latest
  relevant_steps_in_order:
  - uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97
    with:
      python-version: '3.12'
  - name: Install PyYAML
    run: pip install --require-hashes -r .github/requirements/docs-quality.txt
  - name: Heavy nightly completion freshness (issue 6436)
    run: python3 scripts/tests/test_ci_nightly_freshness.py

```

## Calibrated controls

Every row ran the same full41-test suite on an isolated copy; source worktree files were never mutated for these controls. Killed means assertion failures with0 test errors and all41 tests reported, excluding malformed/crashing controls.

```json
{
  "baseline": {
    "name": "baseline",
    "classification": "PASS",
    "exit_code": 0,
    "tests": 41,
    "failures": 0,
    "errors": 0,
    "mutation": null
  },
  "mutants": [
    {
      "name": "active_census_guard_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 6,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "if run[\"id\"] != current_id:",
        "if False:"
      ]
    },
    {
      "name": "active_census_guard_inert",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 6,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "if run[\"id\"] != current_id:",
        "if run[\"id\"] < 0 and run[\"id\"] != current_id:"
      ]
    },
    {
      "name": "active_census_shape_guard_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 1,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "or len(doc[\"workflow_runs\"]) != doc[\"total_count\"]",
        "or False"
      ]
    },
    {
      "name": "active_schedule_admitted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 2,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "raise EvidenceError(\"same-head schedule still active or status unreadable\")",
        "return True, \"active schedule admitted\""
      ]
    },
    {
      "name": "aged_out_census_call_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 17,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "if doc[\"total_count\"] > HISTORY_LIMIT:\n        ensure_no_active(get, repo, head, current_id)",
        "if doc[\"total_count\"] > HISTORY_LIMIT:\n        pass"
      ]
    },
    {
      "name": "cargo_failure_masked",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 7,
      "errors": 0,
      "mutation": [
        ".github/workflows/ci.yml",
        "|| mutation_exit=$?",
        "|| true"
      ]
    },
    {
      "name": "completed_advisory_rejected",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 4,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "if exit_code not in (0, 2, 3):",
        "if exit_code not in (0,):"
      ]
    },
    {
      "name": "completion_exit_guard_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 14,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "if exit_code not in (0, 2, 3):",
        "if False:"
      ]
    },
    {
      "name": "completion_exit_guard_inert",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 14,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "if exit_code not in (0, 2, 3):",
        "if exit_code < 0 and exit_code not in (0, 2, 3):"
      ]
    },
    {
      "name": "coverage_marker_guard_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 5,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "or not successful_step(coverage[0], COVERAGE_STEP)",
        "or False"
      ]
    },
    {
      "name": "history_budget_raised",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 18,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "HISTORY_LIMIT = 10",
        "HISTORY_LIMIT = 20"
      ]
    },
    {
      "name": "incomplete_work_skipped",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 324,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "return True, f\"heavy work incomplete in run",
        "return False, f\"heavy work incomplete in run"
      ]
    },
    {
      "name": "marker_condition_reversed",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 1,
      "errors": 0,
      "mutation": [
        ".github/workflows/ci.yml",
        "${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'true' }}",
        "${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'false' }}"
      ]
    },
    {
      "name": "mutation_marker_guard_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 105,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "or not all(successful_step(j, MUTATION_STEP) for j in mutations)",
        "or False"
      ]
    },
    {
      "name": "mutation_marker_guard_inert",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 105,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "or not all(successful_step(j, MUTATION_STEP) for j in mutations)",
        "or (False and not all(successful_step(j, MUTATION_STEP) for j in mutations))"
      ]
    },
    {
      "name": "non_nightly_event_admitted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 5,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "return False, \"heavy nightly work is isolated from this event\"",
        "return True, \"heavy nightly work is isolated from this event\""
      ]
    },
    {
      "name": "planned_count_guard_deleted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 2,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "doc[\"total_mutants\"] == len(planned) == sum(counts)",
        "doc[\"total_mutants\"] == sum(counts)"
      ]
    },
    {
      "name": "planned_count_guard_inert",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 2,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "doc[\"total_mutants\"] == len(planned) == sum(counts)",
        "doc[\"total_mutants\"] == (len(planned) if False else sum(counts)) == sum(counts)"
      ]
    },
    {
      "name": "producer_call_removed",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 4,
      "errors": 0,
      "mutation": [
        ".github/workflows/ci.yml",
        "python3 scripts/ci_nightly_freshness.py --mutation-complete \"$mutation_exit\" \"$output_dir/mutants.out\"",
        "true"
      ]
    },
    {
      "name": "skipped_is_proof",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 20,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "return \"skipped\"",
        "return \"complete\""
      ]
    },
    {
      "name": "wrong_attempt_accepted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 1,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "or job.get(\"run_attempt\") != run[\"run_attempt\"]",
        "or False"
      ]
    },
    {
      "name": "wrong_repository_accepted",
      "classification": "KILLED",
      "exit_code": 1,
      "tests": 41,
      "failures": 1,
      "errors": 0,
      "mutation": [
        "scripts/ci_nightly_freshness.py",
        "and run[\"repository\"].get(\"full_name\") == repo",
        "and True"
      ]
    }
  ],
  "counts": {
    "KILLED": 22,
    "SURVIVED": 0,
    "INVALID_OR_ERROR": 0
  }
}
```
