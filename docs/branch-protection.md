# Branch protection — `main`

This is the **doc-of-record** for the branch-protection ruleset on `main`. The settings
themselves are configured **out-of-repo** by the repository owner (Settings → Rules →
Rulesets); this file records what they should be so the protection is reviewable and
reproducible.

## Protected branch

- **`main`** — the only long-lived branch. Changes land via pull request; direct pushes
  are disallowed for non-administrators. Repository administrators can always bypass the
  ruleset (see "Other settings").

## Required status checks

There is exactly **one** required status check (maintainer decision, 2026-10-05):

| Required check | Workflow | What it covers |
|---|---|---|
| **`ci-fast`** | [`.github/workflows/ci-fast.yml`](../.github/workflows/ci-fast.yml) | clippy `-D warnings`, nextest + doctests on the core crates, and the W3C SPARQL conformance ratchet. The `ci-fast` job is an ordinary `needs:` aggregator over those three jobs — no cross-workflow polling. |

`strict_required_status_checks_policy` is `false` (PRs need not be rebased onto `main`
before merging). `ci-fast` also runs on `merge_group`, so it works with or without the
merge queue.

### Everything else is non-blocking

All other workflows are informational. The heavy suites run **nightly and on demand**,
not on pull requests:

| Workflow | When it runs |
|---|---|
| `ci.yml` (full workspace build/test, wasm, MSRV, all conformance ratchets, coverage, mutants, geiger, container smoke) | nightly + `workflow_dispatch` |
| `feature-matrix.yml` (opt-in feature legs) | nightly + `workflow_dispatch` |
| `fuzz.yml` (cargo-fuzz + differential smoke) | nightly + `workflow_dispatch` |
| `formal-verification.yml` (Kani `pr_gate` suites) | nightly + `workflow_dispatch` |
| `supply-chain.yml` (cargo-deny, cargo-vet, SBOM, VEX) | push to `main` + nightly + `workflow_dispatch` |
| `bench.yml` (benchmarks + regression ratchet) | push to `main` + nightly + `workflow_dispatch` + PRs labelled `bench` |
| `nightly-full-sweep.yml` (site a11y / full-surface visual / cross-browser, tauri smoke) | nightly + `workflow_dispatch` |
| `container-scan.yml`, `vectorized-feature-off.yml` | push to `main` (path-filtered) + `workflow_dispatch` (container-scan also nightly) |

To run a heavy suite on a branch before merging, dispatch it on that branch, e.g.
`gh workflow run ci.yml --ref <branch>`. **Add the `bench` label to a PR to run
Benchmarks against `main`.**

Lightweight path-filtered PR workflows (docs-quality, js, python, gui, actionlint, …)
still run on pull requests where their paths change; they report but do not block.
Jobs whose name carries an `advisory`/`informational` token must be declared in
[`.github/advisory-registry.json`](../.github/advisory-registry.json)
(`scripts/check-advisory-registry.py` keeps names and declarations in sync).

## Required reviews

sparq is a **single-maintainer** repository: GitHub does not let an author approve their
own PR, so a human-approval requirement would deadlock merging. The ruleset therefore sets
`required_approving_review_count: 0` and `require_code_owner_review: false` deliberately,
and relies on automated review instead:

- **Copilot code review on push** (`copilot_code_review`, `review_on_push: true`).
- **CodeQL code-scanning alerts block** (`code_scanning` rule, `alerts_threshold:
  errors_and_warnings`, `security_alerts_threshold: all`).
- **Conversation resolution required** (`required_review_thread_resolution: true`).
- **Code-quality rule** (`code_quality`, `severity: all`).

[`CODEOWNERS`](../CODEOWNERS) records ownership of the high-risk paths so code-owner
review can be switched on if a second trusted reviewer is added.

## History and push rules

- **Linear history** — only the squash merge method is allowed
  (`allowed_merge_methods: ["squash"]`).
- **Block force pushes** (`non_fast_forward`) and **branch deletion** (`deletion`).

## Other settings

- **Repository administrators can always bypass the ruleset** (`RepositoryRole`,
  `actor_id: 5`, `bypass_mode: always`). This is an explicit exception, not a
  compensating control.
- **Merge queue** — optional. If enabled, its required check is the same `ci-fast`.

## Solo-maintainer & the Scorecard Code-Review / Branch-Protection score

OpenSSF Scorecard's `Code-Review` and `Branch-Protection` checks score below 10 here by
construction (gap **GX-OSSF-3**,
[`compliance/openssf/gap-register.md`](../compliance/openssf/gap-register.md)):
Scorecard discounts self-approval and rewards `required_approving_review_count ≥ 1`,
code-owner review and stale-review dismissal, which a one-human repo cannot use. The
compensating controls are the automated reviews above, the required `ci-fast` check,
squash-only history, and the force-push/deletion blocks. Repository administrators retain
the bypass documented above.

## Verifying the live ruleset matches this document

```sh
gh api repos/sparq-org/sparq/rulesets                       # find the `main` ruleset id
gh api repos/sparq-org/sparq/rulesets/<id> | python3 -m json.tool
```

Expected rules: `deletion`, `non_fast_forward`, `pull_request` (0 approvals, thread
resolution, squash only), `required_status_checks` (one context: `ci-fast`, not strict),
`code_quality`, `code_scanning` (CodeQL), `copilot_code_review`, and optionally
`merge_queue`. If the live ruleset drifts, update this file in the same change.
