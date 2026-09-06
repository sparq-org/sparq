# Activation runbook — turning on the self-managing orchestration

What's built (in reviewable PRs #2283/#2284/#2285 + the private registry `jeswr/agent-account-registry`)
and the exact ordered steps to activate it. Everything is currently **dry / unmerged**; nothing runs
autonomously yet. Do the steps in order — the trust safety layer goes on before any dispatch.
See [`issue-native-orchestration.md`](issue-native-orchestration.md) (design) +
[`issue-native-orchestration-review-gpt56.md`](issue-native-orchestration-review-gpt56.md) (review).

## 0. Two decisions that gate Phase 3

- **Automation identity.** A **GitHub App** (recommended) or a fine-grained **PAT**, because the
  default `GITHUB_TOKEN` cannot write secrets or trigger downstream workflows. Needed scopes:
  on `sparq-org/sparq` → Actions:write, Contents:write, Pull requests:write, Issues:write; on
  `jeswr/agent-account-registry` → Contents:write (the lease ledger), Issues:write. Store as
  `REGISTRY_ADMIN_TOKEN` (+ a worker-dispatch token). *A fine-grained PAT cannot span two owners, so
  an App with installations on both repos is the clean option.*
- **Token boundary (review S1/D4).** The masked-public-secret model (Option A) is **rejected** by the
  review. Adopt the **private broker**: the public worker presents GitHub **OIDC** to a private
  allocator that grants a lease and returns a **short-lived, model-scoped, single-claim** capability
  (or proxies the model call). The tokens are already private; the broker is the remaining build.

## 1. Enrollment: register the model accounts

Two are seeded (`acct01` codex/terra, `acct02` claude). For more, use the **web-login broker**: open a
`set up new account` issue in the registry + add the `set-up-account` label → sign in at the posted
URL → the token is captured + stored. Requires `REGISTRY_ADMIN_TOKEN` (step 0).

## 2. Trust safety layer FIRST (merge #2285)

Enable `triage-issue.yml` + `promote-on-approval.yml` before anything dispatches. This quarantines
third-party issues (`trust:untrusted`) and only acts on maintainer-approved / collaborator content.
It is safe to enable early — it never dispatches or merges.

## 3. Migrate the tracker (needs your go-ahead — bulk-creates ~895 issues)

`python3 scripts/bd-to-issues.py --apply --repo sparq-org/sparq` (idempotent two-pass; validated on a
throwaway repo). Test on a copy first if you like (`--limit N`, a scratch repo). Freeze `bd` after.

Then **re-verify**: `python3 scripts/bd-to-issues.py --verify --repo sparq-org/sparq` (read-only;
exit 1 on any gap). `--apply` cannot answer "did it finish" — it is idempotent, so a re-run prints
`0/0/0` both when it repaired nothing and when there was never anything wrong. `--verify` reconciles
the counts (over `bd-migration`-labelled issues only — a bare body marker is forgeable, so an
unlabelled one is reported as unauthenticated, never counted as migrated), reports any bd-id
mapped to two issues, and checks that pass 2's `Blocked-by:` /
`Parent:` markers actually landed — the pass that runs only *after* pass 1 completes, and is
therefore silently absent from a run that died partway through creating issues.

## 4. Build + enable Phase 3 (dispatch + worker)

Compose the three tested cores — `ready-issues.py` (frontier) → `route-resolve.py` (model_chain+agent)
→ `select-and-claim.py --claim` (lease + `secret_ref`) — then the **worker** = a GitHub Actions job on
the **free public runner** that (per the step-0 broker) obtains a short-lived credential, runs the
routed agent on a branch off the merge-queue tip, and opens a **draft** PR. This is the remaining build;
ping me once step 0 is done.

## 5. Phases 4–6

Batcher (`batch-merge.yml`: cluster disjoint draft PRs → one CI run → merge / bisect-on-failure) +
main-CI debounce; groomer (backlog + the already-built `groom-leases.yml`); the `on-issue-dispatch`
trigger for full hands-off autonomy.

## Launch gate (from the review — verify before real-credential autonomy)

1. Brokered credential boundary; no long-lived provider/registry/App-key/admin secret in the public repo.
2. Positive, revision-bound trust attestation at the last step before model invocation + provenance filtering.
3. Transactional unique lease + idempotent dispatch claim with expiry/heartbeat/reclaim (built — verify wired).
4. Completed, resumable migration tested against a real copy (built — run + reconcile).
5. Paginated, native-dependency-aware readiness with validated state + multi/global partitions.
6. Exact-commit batch CI that satisfies branch protection without bypass; never defers security checks.
7. Pinned/verified enrollment deps; isolated temp auth; runtime masking; unconditional cleanup (built).
8. End-to-end adversarial tests: a fork author / commenter / worker-branch / edited-approval / compromised
   model / cancelled run / two simultaneous dispatchers cannot get credentials, bypass trust, duplicate
   work, exceed capacity, or merge an untested tree.

Until the gate passes, run in dry-run/read-only with synthetic credentials and no merge permission.
