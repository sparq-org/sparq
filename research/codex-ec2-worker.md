# codex-on-EC2 worker channel (`scripts/ec2-codex-worker.sh`)

**Status:** working tool, in use since 2026-07-12. Vendored in-repo by `sq-uhqah` for durability +
maintainer review. **⚠️ The OAuth-credential copy (see "Security model") needs an explicit
maintainer sign-off before this is treated as blessed infrastructure.**

## What it is

A way to run a **GPT-5.6 code writer (codex-cli)** on a throwaway EC2 instance that can build the
workspace freely, while the small RAM-constrained session box cannot. One bead in, one committed
branch out; the trusted box opens the PR.

Flow (`scripts/ec2-codex-worker.sh <bead>`):

1. Launch one on-demand instance (orphan-proof user-data: two independent hard shutdown caps armed
   before any install), install rust + node + codex, `git clone` + a fresh worktree branched off
   `origin/main`.
2. `scp` the local `~/.codex/{auth.json,config.toml}` to the instance, render the brief
   (`scripts/codex-ec2-briefs/cargo-brief.txt`, or `js-brief.txt` for wasm/npm beads via
   `CODEX_BRIEF_TEMPLATE`), run `codex exec`. Codex implements + runs the crate-scoped gates +
   commits (it never pushes).
3. `scp` a git **bundle** back, terminate the instance, delete the keypair/SG, orphan-sweep the tag.
4. **From the trusted box** (`gh` auth stays local): push the branch + open the PR. The controller
   refuses to push/PR unless the worker reported `committed=true && gates_green && !needs_architect`
   and the bundle has >0 commits.

The orchestrator reviews the PR and arms it — the worker never arms or merges.

## Security model — the part that needs maintainer review

- **`~/.codex/{auth.json,config.toml}` are copied to the ephemeral instance** over `scp` (contents
  never logged; `chmod 600` on arrival). This is a **codex OAuth token for the GPT-5.6 model
  endpoint**, not an AWS or GitHub credential. It is what lets codex run on the box.
- The instance is single-tenant, launched in the account's default VPC with a self-deleting
  security group, and is **terminated + swept within one bead's lifetime** (watchdog hard-cap
  `CODEX_MAX_LIFETIME_SEC`, default 7200 s). The keypair + SG are deleted on cleanup.
- **The GitHub token never leaves the trusted box** — the worker has no `gh` auth and cannot push
  or open PRs; the controller does that after pulling the bundle.
- Residual risk to weigh: a compromised/malicious AMI or a mid-run instance compromise could read
  the codex token from disk. Mitigations in place: ephemeral + short-lived + single-tenant + token
  is model-endpoint-scoped (revocable independently of AWS/GitHub). **Decision for the maintainer:**
  is scp-of-the-codex-token to an ephemeral instance acceptable, or should this move to a
  per-run short-lived token / a pull-based auth broker?

## Cost model

- On-demand `c7g.4xlarge` (**spot is blocked** on the `pss` profile — the spot service-linked role
  cannot be created), ~$0.58/hr → ~$0.15–0.45 per bead, worst-case orphan ~$1.16 (the 7200 s
  watchdog). `CODEX_MAX_FLEET` (default 1) caps concurrency.
- All EC2 ops use `AWS_PROFILE=pss` (the box's default instance role denies `DescribeInstances`).
  Tag `purpose=sparq-codex-worker` is distinct; prod + dev instances are hard-excluded from every
  terminate path; the tag is orphan-swept after every run.

## Knobs

`CODEX_MAX_FLEET` · `CODEX_MAX_LIFETIME_SEC` · `CODEX_NEEDS_WASM=1` (adds the wasm32 target +
wasm-pack to the user-data) · `CODEX_BRIEF_TEMPLATE` (select cargo vs js brief) · `CODEX_DISK_GB`.
`bash scripts/ec2-codex-worker.sh --self-test` runs the pure-logic checks (no AWS calls).

## Briefs

`scripts/codex-ec2-briefs/cargo-brief.txt` (Rust crate beads) and `js-brief.txt` (npm/wasm Solid
server beads) carry the crate-scoped gate contract + the `CODEX_RESULT={...}` parse line the
controller depends on. They encode the recurring gate traps (clippy pre-existing-drift rule,
feature-off byte-stability, readme-template cap, intra-doc-link-to-cfg-gated-item).
