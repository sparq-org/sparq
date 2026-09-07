# G3 — cost-capped, secure EC2-deferred benchmark CI

The heavy benchmarks (many-core/NUMA scaling, large-scale ingestion) can't run on free GitHub
runners. They run on EC2, but a **public** repo with EC2-triggering CI is dangerous if done wrong,
and must stay **under $5/month**. This is the security + cost design.

> **STATUS ([OPUS-5], #3784) — the automated lane is RETIRED; this record is now the revival spec.**
> The AWS OIDC role this design depends on was deliberately descoped, so `bench-ec2.yml`'s crons
> were removed: every scheduled tick failed in `Configure AWS credentials (OIDC)` before any
> benchmark ran and — carrying no advisory declaration — that failure **gated** `main`. The workflow
> is now **manual dispatch only** (`lane: heavy` / `lane: full-suite`), guarded by
> `vars.AWS_BENCH_ROLE_ARN != ''` so a dispatch without the role skips instead of failing.
> Everything below still describes what to create to bring the lane back, so the `schedule:` block
> quoted later is **aspirational, not current** — re-add it in the same change that re-provisions the
> role. Maintainer-run EC2 benchmarking outside this CI lane is NOT descoped.

<!-- separate blockquotes (MD028) -->

> **G2 — free per-commit perf CI (Pages toggle).** `.github/workflows/bench.yml` runs
> `scripts/ci-bench.sh` on every push to `main` and on `pull_request` (PRs compute + comment but
> do **not** auto-push, so the published series isn't polluted), storing points via
> github-action-benchmark on the `benchmark-data` branch under `dev/bench`. **One-time owner
> action (cannot be set from a workflow):** Settings → Pages → Source = "Deploy from a branch",
> Branch = `benchmark-data` / `/ (root)` → Save. The dashboard then renders at
> <https://sparq.jeswr.org/dev/bench>. [OPUS-4.8]

## Threat model & the three hard rules

1. **No long-lived AWS keys in GitHub secrets.** Use **GitHub OIDC → a scoped IAM role** that CI
   assumes for a short-lived token. A leaked workflow log then exposes nothing reusable.
2. **Never run on untrusted triggers.** The EC2 workflow runs ONLY on `schedule` (weekly) and
   `workflow_dispatch` (a maintainer clicks "run"). **Never** `pull_request` — a fork PR must not be
   able to assume the role or spend money. Guard every job with `if: github.repository == 'sparq-org/sparq'`.
3. **Self-limiting spend.** Spot instance + hard `timeout-minutes` + **always-terminate** (even on
   failure/cancel) + a low weekly cadence. 4–5 runs/month × a ~30-min spot box ≈ **well under $5**.

## The OIDC trust (created once, in AWS — the browser/console step)

1. **IAM → Identity providers → Add provider → OpenID Connect**
   - Provider URL: `https://token.actions.githubusercontent.com`
   - Audience: `sts.amazonaws.com`
2. **IAM → Roles → Create role → Web identity** → the provider above, audience `sts.amazonaws.com`.
   Trust policy scoped to THIS repo + protected refs only:
   ```json
   {
     "Version": "2012-10-17",
     "Statement": [{
       "Effect": "Allow",
       "Principal": { "Federated": "arn:aws:iam::<ACCOUNT_ID>:oidc-provider/token.actions.githubusercontent.com" },
       "Action": "sts:AssumeRoleWithWebIdentity",
       "Condition": {
         "StringEquals": { "token.actions.githubusercontent.com:aud": "sts.amazonaws.com" },
         "StringLike": { "token.actions.githubusercontent.com:sub": "repo:sparq-org/sparq:ref:refs/heads/main" }
       }
     }]
   }
   ```
   (The `sub` condition pins it to the `main` branch of `sparq-org/sparq` — fork PRs and other refs
   cannot assume it.)
3. **Permissions policy** — least-privilege, tag-scoped so CI can only touch its own instances:
   ```json
   {
     "Version": "2012-10-17",
     "Statement": [
       { "Effect": "Allow", "Action": ["ec2:RunInstances"], "Resource": "*",
         "Condition": { "StringEquals": { "aws:RequestTag/purpose": "sparq-ci-bench" } } },
       { "Effect": "Allow", "Action": ["ec2:CreateTags"], "Resource": "*",
         "Condition": { "StringEquals": { "ec2:CreateAction": "RunInstances" } } },
       { "Effect": "Allow", "Action": ["ec2:TerminateInstances","ec2:DescribeInstances","ec2:DescribeInstanceStatus","ec2:DescribeSpotInstanceRequests"], "Resource": "*",
         "Condition": { "StringEquals": { "ec2:ResourceTag/purpose": "sparq-ci-bench" } } },
       { "Effect": "Allow", "Action": ["ec2:DescribeImages","ec2:DescribeSubnets","ec2:DescribeSecurityGroups"], "Resource": "*" }
     ]
   }
   ```
   No IAM, no S3-write beyond a single results prefix (add an S3 statement only if results go via
   S3). The role can launch/terminate ONLY `purpose=sparq-ci-bench`-tagged instances.
4. Set the role ARN as a **repo *variable*** (not a secret — an ARN isn't sensitive):
   `AWS_BENCH_ROLE_ARN`.
5. **Budget backstop:** an AWS Budgets monthly alarm at $5 on the `purpose=sparq-ci-bench` tag.

## The workflow (`.github/workflows/bench-ec2.yml`)

```yaml
on:
  schedule: [{ cron: '0 6 * * 1' }]   # Mondays 06:00 UTC — the rate limit (≈4–5/month)
  workflow_dispatch:                  # + manual maintainer trigger
permissions: { id-token: write, contents: write }
concurrency: { group: bench-ec2, cancel-in-progress: false }
jobs:
  ec2-bench:
    if: github.repository == 'sparq-org/sparq'   # never on forks
    runs-on: ubuntu-latest
    timeout-minutes: 60                       # hard wall-clock cap
    steps:
      - uses: actions/checkout@v4
      - uses: aws-actions/configure-aws-credentials@v4
        with: { role-to-assume: ${{ vars.AWS_BENCH_ROLE_ARN }}, aws-region: eu-west-2 }
      - run: bash scripts/ec2-bench.sh        # launch SPOT, run, ALWAYS terminate, fetch results
      - uses: benchmark-action/github-action-benchmark@v1   # track the EC2 series like G2
        with: { tool: customSmallerIsBetter, output-file-path: ec2-bench-results.json, auto-push: true, gh-pages-branch: benchmark-data, benchmark-data-dir-path: dev/bench-ec2 }
```

## `scripts/ec2-bench.sh` (to implement)

Since the repo is now **public**, the instance can `git clone` it with no credentials — no SSH key
in CI. Flow: `run-instances` a **spot** box (tagged `purpose=sparq-ci-bench`) with **user-data**
that installs Rust, clones the repo at the triggering SHA, runs the scaling sweep + an ingestion
bench, writes `ec2-bench-results.json`, and **self-terminates** (`shutdown -h` with
`InstanceInitiatedShutdownBehavior=terminate`). The runner polls instance state, pulls the results
(via S3 or SSM), and asserts termination. A `trap`/`always()` terminate-by-tag step guarantees no
orphaned instance even if the run is cancelled.

## Cost math

c7g.4xlarge spot ≈ $0.25/hr; a 30-min run ≈ $0.13. Weekly + occasional manual ≈ 6 runs/month ≈
**$0.80/month** — order of magnitude under the $5 cap, with the AWS Budget alarm as a hard backstop.
A rare 192-vCPU NUMA sweep (manual dispatch only) at spot ~$4/hr for ≤30 min ≈ $2, still within a
month's budget if used sparingly.

## Status

Design complete. Execution needs: (a) the IAM OIDC provider + role created in AWS (console/browser
— the `PSSSingleInstanceDeploy` SSO role lacks `iam:*`), (b) the `AWS_BENCH_ROLE_ARN` repo variable,
(c) `scripts/ec2-bench.sh` + `bench-ec2.yml` committed. (a) is the gating step.
