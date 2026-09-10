# PR6481 merge-group queue diagnosis

{
  "author": "GPT-6 Astra xhigh",
  "status": "BOUNDED_READ_ONLY_COMPLETE",
  "merge_group_sha": "4595388de9e389f5369d63828fbf90cfc16b62d9",
  "PR_head": "17594d4a6534c142cae764772fc42049e898eca3",
  "snapshots_utc": {
    "first": "2026-09-10T11:35:04.882468+00:00",
    "last": "2026-09-10T11:36:37.997433+00:00"
  },
  "main_finding": "The two workflow-run records say queued, but both workflows are admitted and executing. Their ready jobs are contending for GitHub-hosted ubuntu-latest runner admission. No whole-workflow concurrency lock, missing self-hosted label, review/approval hold, or source failure is demonstrated.",
  "certainty": {
    "verified": [
      "Exact merge_group head/event/attempt confirmed for both runs.",
      "CI:22successful,7skipped,2running,2queued jobs; matrix:22successful,1skipped,2running,32queued. Complete job pages, no observed failed MG job.",
      "Four MG jobs have nonzero assigned GitHub-hosted runner IDs; 44 jobs already succeeded. Later assignments through11:31 demonstrate progress.",
      "20 in-progress job rows across nearby named-run snapshots:8 older scheduled mutation jobs,4 Miri,2 Kani,4 MG,1 aggregate waiter,1 retriage.",
      "The relevant CI test matrix and feature groups have no max-parallel throttle; older mutation/Miri/Kani caps are8/4/2 and are fully occupied.",
      "CI/matrix/select workflow bytes at actual MG exactly match PR head. Per-workflow keys include workflow/ref; old scheduled CI uses its own run-ID suffix and main ref.",
      "Same-ref census returns exactly8 runs with only one CI and one feature-matrix run; no older competing same-group workflow found."
    ],
    "inference": "A shared hosted-runner concurrency/admission limit is the best-supported cause of queued ready jobs. The observed20 occupied slots is consistent with a20-slot capacity, but effective account/org capacity was not exposed or verified. This is contention with ongoing progress, not proof of a GitHub-wide outage or complete runner starvation.",
    "unknown": [
      "Exact account/org hosted concurrency entitlement and any activity in other repositories.",
      "Provider-side pending reason, fairness/order, or service-capacity limitation; queued check output is null and Jobs/Run REST fields do not state the cause.",
      "Whether every queued job will complete within the existing aggregate time budget."
    ]
  },
  "older_work": {
    "CI_run": 34453539472,
    "event": "schedule",
    "head": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
    "created_at": "2026-09-10T08:08:10Z",
    "active_mutation_jobs": 8,
    "same_concurrency_group_as_MG": false,
    "note": "Shares runner capacity only; its failures/cancellation are separate head evidence, not MG failures. No logs reread or cancellations."
  },
  "pending_state_caveat": "Queued Jobs API rows carry started_at equal to creation even with runner_id0/null. Do not treat that timestamp as actual runner execution. Assigned in-progress/completed rows establish real execution.",
  "runner_access": {
    "repository_self_hosted_total": 0,
    "organization_read": "HTTP403: org admin or runners/runner-groups permission required; gh also reports missing admin:org scope.",
    "disposition": "No scope grant/auth refresh or alternative privileged read attempted. Self-hosted runner APIs do not enumerate the public ubuntu-latest hosted fleet."
  },
  "gate": {
    "run": 34469862189,
    "job": 102846986203,
    "status": "in_progress",
    "started_at": "2026-09-10T11:10:46Z",
    "existing_job_timeout_minutes": 80,
    "source_budget": "base110x20s with conditional saturation/progress extension up to155polls; reporter-only tail15polls;80-minute job outer bound. Source-only budget observation, not a claim that live extension has armed."
  },
  "action_options": [
    {
      "priority": 1,
      "action": "Preserve this live merge group and have root observe exact job completions and the existing aggregate result, rather than act on top-level queued alone.",
      "reason": "Runner assignment and successful completions show forward progress; rerunning/requeueing would discard or duplicate healthy work."
    },
    {
      "priority": 2,
      "action": "If the same measured contention threatens the existing deadline or recurs, review scheduling priority/admission of the non-MG nightly lanes as a separate change using these actual job identities and existing cap tests.",
      "reason": "Older mutations plus Miri/Kani occupy14of20 observed slots. Any live cancellation/cap change needs separate root decision; no cancellation, gating relaxation or grant is justified merely by this report."
    }
  ],
  "logical_read_requests": 20,
  "read_limit": 25,
  "retries": 0,
  "remote_mutations": false,
  "live_logs_downloaded": false,
  "production_changes": false,
  "registry_state": "No read/change in this task; preserve standing OFF instruction.",
  "no_commands_pending": true
}

## Concurrency/source proof

{
  "head": "4595388de9e389f5369d63828fbf90cfc16b62d9",
  "PR_head": "17594d4a6534c142cae764772fc42049e898eca3",
  "workflow_bytes_match_PR": {
    "ci.yml": true,
    "feature-matrix.yml": true,
    "ci-select.yml": true
  },
  "groups": {
    "MG_CI": "ci-CI-refs/heads/gh-readonly-queue/main/pr-6481-781f667c19a8ebb779cfccb24b05ea432360b025-shared",
    "MG_matrix": "feature-matrix-feature-matrix-refs/heads/gh-readonly-queue/main/pr-6481-781f667c19a8ebb779cfccb24b05ea432360b025-shared",
    "older_scheduled_CI": "ci-CI-refs/heads/main-34453539472"
  },
  "line_evidence": {
    "ci_workflow_concurrency": ".github/workflows/ci.yml:67-87",
    "ci_shards_no_max_parallel": ".github/workflows/ci.yml:623-636",
    "matrix_workflow_concurrency": ".github/workflows/feature-matrix.yml:184-191",
    "matrix_needs_and_admission": ".github/workflows/feature-matrix.yml:699-743",
    "selector_runner": ".github/workflows/ci-select.yml:117",
    "nightly_mutants_cap8": ".github/workflows/ci.yml:4075",
    "nightly_mutants_ref_per_crate_group": ".github/workflows/ci.yml:4340-4349",
    "miri_cap4": ".github/workflows/miri.yml:180",
    "kani_cap2": ".github/workflows/kani.yml:194",
    "gate_timeout80": ".github/workflows/ci-summary.yml:272"
  },
  "older_CI_matches_MG_workflow": true
}

## Per-run job counts

[
  {
    "run": "ci",
    "active": 2,
    "queued": 2,
    "total": 33
  },
  {
    "run": "matrix",
    "active": 2,
    "queued": 32,
    "total": 57
  },
  {
    "run": "older-nightly",
    "active": 8,
    "queued": 0,
    "total": 66
  },
  {
    "run": "kani",
    "active": 2,
    "queued": 3,
    "total": 6
  },
  {
    "run": "miri",
    "active": 4,
    "queued": 16,
    "total": 25
  },
  {
    "run": "batch",
    "active": 0,
    "queued": 1,
    "total": 1
  },
  {
    "run": "retriage",
    "active": 1,
    "queued": 0,
    "total": 1
  },
  {
    "run": "gate",
    "active": 1,
    "queued": 0,
    "total": 1
  }
]
