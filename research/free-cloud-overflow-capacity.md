# Overflowing onto free cloud compute — contingency design for a bottleneck we do not have

> 🤖 **SPARQ agent** design record (research + design only; authored by Opus 5, 1M context).
> No implementation, no beads. `[OPUS-5]`
> Commissioned by the maintainer (2026-07-26): *"in the hopes that we do get to a throughput
> where number of workers becomes the bottleneck … research good options for where we can get
> free cloud compute … and design a way that we can cleanly overflow onto those cloud providers
> when needed — without outages or capacity limits in those providers causing problems in the
> workflows."*
> Companion records: [`research/ec2-build-farm-design.md`](ec2-build-farm-design.md),
> [`research/ci-ec2-design.md`](ci-ec2-design.md),
> [`research/ci-runner-consolidation-2026-07.md`](ci-runner-consolidation-2026-07.md).

## 0. Read this section before any other

**Runner capacity is not currently a constraint, and the free-tier VM providers named in the
request would give us less capacity than we already have for free.** Both conclusions are
measured or cited below.

To be precise about what that does and does not overturn: the maintainer's framing — *"in the
hopes that we do get to"* a worker-bound throughput — is exactly right, and this record does not
dispute it. What the findings overturn is the **implicit assumption that always-free cloud VMs
are where the extra capacity would come from.** They are not; the capacity is already sitting
unused inside our GitHub account. So the recommendation is to **not build the overflow substrate
now**, and instead install a cheap tripwire that says when — and whether — it would ever pay.

Three findings drive everything else:

1. **Actions capacity is not binding.** Measured on 2026-07-26 across both live repositories:
   job-level queue wait p99 was 5 s (registry, n=340) and 0 s median / 6 s max (sparq, n=39).
   Nothing is waiting for a runner.
2. **The free GitHub public-repo runner is already 4 vCPU / 16 GB, on x86 *and* arm64, with
   unlimited minutes.** Oracle's always-free ARM allocation — the headline candidate — was
   *halved* on 2026-06-15 to **2 OCPU / 12 GB total per tenancy**. One Oracle tenancy therefore
   buys **less than a single GitHub runner we already get for nothing**, and it costs us the
   entire public-repo self-hosted-runner threat model to collect.
3. **The one real ceiling is the account-level concurrent-job cap, and we are on the wrong side
   of a free config change.** `sparq-org` is on **Enterprise Cloud (500 concurrent jobs)**.
   The worker fleet runs in `jeswr/agent-account-registry` — a **personal** account, capped at
   **20 (Free) / 40 (Pro)**. Observed peak there: **17 concurrent jobs.** So the worker
   substrate is plausibly at 43–85 % of its ceiling *while the enterprise org next door sits at
   1 of 50 seats with a 500-job limit.*

The honest consequence: **the first ~12–25× of worker-capacity headroom is available for free,
by moving where the workers run — not by adding a cloud provider.** Any free-cloud overflow
design is strictly downstream of that, and should not be built until that headroom is consumed.

### Correction to the brief's premise

The brief's own diagnosis was largely right, and its "~16 GB free-runner envelope" figure is
accurate. One addition and one genuine gap:

- The brief said the binding constraint is worker yield (~20 %), behind it batch fill and a
  self-imposed dispatch concurrency group, "none of them capacity". I agree, and add the
  quantitative form: **a yield fix multiplies effective capacity by up to 5× at zero
  infrastructure cost and zero new attack surface.** Overflow capacity poured into a 20 %-yield
  bucket delivers 20 % of its nominal value. Fixing yield strictly dominates buying capacity,
  and it dominates it by a large factor. (Yield figure is as reported in the brief — I did not
  independently measure it.)
- The brief did not identify the personal-vs-enterprise concurrency asymmetry in §0.3. That is
  the actual capacity cliff, it is nearer than anyone thought, and its fix is a config change.

## 1. What "worker" means here, and the job-class taxonomy

"Number of workers" in the maintainer's request means **agent sessions** (headless model runs
that check out a target repo, author code, and run its gate), not CI jobs. That distinction is
load-bearing for security, because a worker is by construction a **credential-bearing process
that executes model-authored code**. Six classes, with what each holds and what triggers it:

| # | Class | Where | Trigger | Secrets held | Runs untrusted code? |
|---|---|---|---|---|---|
| A | Trusted-ref build/test | sparq `ci.yml` | `push: main`, `merge_group` | none beyond read `GITHUB_TOKEN` | yes (build scripts, tests) |
| B | Same-repo PR build/test | sparq, 30 workflows | `pull_request` (same-org branch) | none needed for pure build legs | yes (model-authored) |
| C | **Fork PR** build/test | sparq (latent) | `pull_request` from a fork | none (GitHub withholds them) | **yes, attacker-chosen** |
| D | Benchmark / heavy measurement | EC2 | `schedule`, `workflow_dispatch` | AWS via OIDC, short-lived | yes |
| E | **Worker (agent session)** | registry `worker.yml` | `workflow_dispatch` only | **model credential + GitHub App private key + `PROVENANCE_SALT`** | **yes, host-side by design** |
| F | Registry write / provenance | registry | `workflow_dispatch`, schedule | `PROVENANCE_SALT`, App token | no |

Two facts about the current estate matter and are easy to miss:

- **`worker.yml` triggers on `workflow_dispatch` only** and its sensitive jobs sit behind
  `environment: dispatch-secrets`. A fork PR cannot reach it. This is already the right shape.
- **The worker job already treats its own cargo step as hostile.** Its inline comments say so
  explicitly — it "executes model-authored target code HOST-side (the cargo gate)" — and the
  design has already been hardened accordingly (registry-write tokens moved to separate jobs,
  App token minted before the gate, artifact paths kept out of reach of hostile cargo). Class E
  is *already* an untrusted-code-execution surface adjacent to high-value credentials.

- Class C is **latent, not hypothetical**: the repo is public, `allow_forking` is true, and 30
  workflows trigger on `pull_request`. It has simply never happened — all 100 sampled PRs came
  from `sparq-org`. A design must not rely on that continuing.

## 2. Security analysis — the part that decides the whole design

### 2.1 GitHub's position, verbatim

> "We recommend that you only use self-hosted runners with private repositories. This is because
> forks of your public repository can potentially run dangerous code on your self-hosted runner
> machine by creating a pull request that executes the code in a workflow."
>
> — <https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/manage-access>

The longer form is more explicit still, and draws the exact contrast this design turns on:

> "**GitHub**-hosted runners execute code within ephemeral and clean isolated virtual machines,
> meaning there is no way to persistently compromise this environment… **Self-hosted** runners
> for GitHub do not have guarantees around running in ephemeral clean virtual machines, and can
> be persistently compromised by untrusted code in a workflow. As a result, self-hosted runners
> should almost **never be used for public repositories** on GitHub, because any user can open
> pull requests against the repository and compromise the environment."
>
> — <https://docs.github.com/en/actions/reference/security/secure-use>

**There is no supported way to make a self-hosted runner safe on a public repository.** The only
place GitHub's documentation acknowledges the case at all is a runner-group setting where the
default is private-repos-only and public access is an override you deliberately enable.

Both sparq and the registry are public. Taken at face value this rules out the obvious design.
But the recommendation bundles two separable risks, and separating them is what makes a safe
subset possible:

- **(i) Persistence across jobs.** A self-managed runner is a long-lived host. Code from job *N*
  can plant something that reads the environment of job *N+1*. This is the risk that actually
  produced real-world compromises, and it is the one that matters for a credential-bearing class.
- **(ii) Network and host position.** The runner sits inside a network the maintainer owns, can
  reach internal services, and holds whatever the host holds.

### 2.2 The decisive structural point: free tiers are *persistent-instance* tiers

The property that makes today's Class E tolerable is that **GitHub destroys the runner after
every job**. Hostile cargo in a worker job gets one job's blast radius and then the machine
ceases to exist.

Always-free tiers do not sell that property. Oracle Always Free is *a fixed allocation you keep
running* — 2 OCPU / 12 GB, one or two instances, in your home region. It is a pet, not cattle.
Reusing one box across worker jobs reintroduces exactly risk (i), against the highest-value
credentials in the estate (a GitHub App private key and `PROVENANCE_SALT`).

Two refinements matter, because the obvious workaround does not work. GitHub states that
destroying the machine after each job is **not** sufficient on its own:

> "Some customers might attempt to partially mitigate these risks by implementing systems that
> automatically destroy the self-hosted runner after each job execution. However, this approach
> might not be as effective as intended, as **there is no way to guarantee that a self-hosted
> runner only runs one job**. Some jobs will use secrets as command-line arguments which can be
> seen by another job running on the same runner, such as `ps x -w`."

The single-job guarantee comes from **registering** the runner as ephemeral (`--ephemeral` or a
JIT config), not from tearing down the host afterwards — "GitHub only assigns one job to a
runner" holds only for ephemeral *registration*. And even then GitHub adds: "Re-using hardware to
host JIT runners can risk exposing information from the environment." A single always-free box
reused across jobs is precisely re-used hardware.

Worse, the free tier's own lifecycle policy is actively hostile to CI use. Oracle's documented
idle-reclamation criteria reclaim an instance whose 7-day 95th-percentile CPU, network **and**
memory utilisation are all under 20 %. **A burst-overflow runner is idle between bursts by
definition — it has precisely the profile the policy targets.** The free tier penalises the
exact usage pattern overflow requires.

### 2.3 Security verdict by job class

| Class | May overflow? | Why |
|---|---|---|
| A — trusted-ref build/test | **Yes** | Trusted trigger; no secrets to steal; result is advisory, not the authoritative gate |
| B — same-repo PR build/test | **Yes, with care** | Pure-build legs hold no secrets; must not carry a writable token |
| C — **fork PR** | **NEVER** | Attacker-chosen code. Structurally excluded by the pattern in §3, not by policy |
| D — benchmark / heavy | **Yes** | Already does, via the trusted-trigger EC2 pattern. Free shared-vCPU boxes are unfit for *canonical* numbers regardless |
| E — **worker (agent session)** | **NO** | Credential-bearing **and** executes model-authored code, and free tiers cannot provide the per-job destruction that makes that combination survivable |
| F — registry write / provenance | **NO** | Holds `PROVENANCE_SALT`; no capacity benefit; pure added risk |

**This is the smaller, safer design the brief asked for, and it has an uncomfortable
implication: the only class where overflow would relieve the maintainer's stated bottleneck —
Class E, the workers — is the one class that must not overflow onto a self-managed free VM.**
Overflow can relieve *gate and benchmark* load. It cannot relieve *worker* load safely on
always-free infrastructure. If worker count truly becomes the ceiling, the answer is the §0.3
concurrency headroom and the managed providers of §3.4/§3.5 — not an Oracle box.

(Cross-reference convention in this record: `§N.M` where section *N* has no `N.M` heading refers
to numbered item *M* of section *N* — so §0.3 is the third finding in §0 and §4.1 is item 1 of §4.)

### 2.4 Do not use self-hosted runners at all

The recommendation is to **never register a self-managed runner against either public repo**,
and instead reuse the pattern this project has already built and proven in
`scripts/ec2-buildfarm.sh`: a **detached executor**.

A GitHub-hosted (or work-box) controller, running from a trusted trigger, launches an ephemeral
VM, ships it a script, polls for a sentinel, pulls the result, and terminates it. The overflow
box never registers with GitHub, never receives a `GITHUB_TOKEN`, and is never named in any
`runs-on:`.

The security properties that follow are structural rather than procedural, which is why this is
the right choice:

- **A fork PR cannot reach an overflow box, because no runner exists and it cannot mint the
  credential to make one.** This needs stating precisely, because the naive version of the
  argument is wrong. It is *not* that "no workflow names an overflow label": for a
  `pull_request` event **the workflow file that runs is the pull request's own copy**, so a fork
  PR can freely edit `runs-on:` to target any label it likes. That is exactly how the
  public-repo runner compromises in §2.5 worked. The protection is therefore the stronger,
  structural one: **there is no registered runner for any label to resolve to**, and obtaining
  overflow capacity requires assuming a cloud role, which needs `id-token: write` — a permission
  **a fork `pull_request` never receives**, along with no repo secrets. A fork PR can ask for an
  overflow box all it likes and get nothing. Risk (i) and (ii) are eliminated by construction,
  not by configuration.
- **No credential lives on the overflow box.** The cloud credential stays in the controller job
  on a GitHub-hosted runner, and should be OIDC-federated and short-lived (the pattern already
  designed in `ci-ec2-design.md`), never a long-lived key on the executor. Two available
  hardening levers are worth folding in, because they bind the role to *exactly* the trusted
  controller: the OIDC `runner_environment` claim (`github-hosted` | `self-hosted`) and
  `workflow_ref`. AWS IAM only evaluates `sub` and `aud`, so these must be folded into `sub` via
  `include_claim_keys` to be enforceable. A trust policy should also **never** admit the
  `…:pull_request` `sub` form for a role that can spend money. On the launched box, disable IMDS
  (or set hop-limit 1) so nothing can reach instance credentials.
- **Provisioning is gated by the trigger, not by a runner group** — and on this plan a runner
  group could not do the job anyway. Restricting a runner group to *selected workflows* is
  **GitHub Enterprise Cloud / GHES only**, and repository-level runners are not in a group at all,
  so they carry **no group policy whatsoever**. Leaning on runner groups would be a weaker
  guarantee than having no runner, and on a personal-account repo it is not even available.
- **A wrong or compromised result cannot merge anything.** The executor's verdict is
  pre-merge confidence only; `ci-summary` on GitHub-hosted runners remains the authoritative
  gate, exactly as `ec2-build-farm-design.md` §3 already scopes it.

Cleanup for a died-mid-job executor is the three-independent-watchdog design already validated
in `ec2-build-farm-design.md` §4 (`instance-initiated-shutdown-behavior=terminate`, a detached
sleep watchdog, and a `systemd-run` transient timer, all armed before any package install), plus
a tag-scoped orphan sweep. That is a solved problem in this estate and should be reused verbatim,
not redesigned.

**Network posture for an overflow executor:** egress to the public git remote and the package
registries it needs; **no** inbound except SSH from the controller's address, on an ephemeral
per-run keypair; no route to any maintainer-owned network. It is safe because it holds nothing
worth stealing and can reach nothing worth attacking — the same reasoning that makes the
existing benchmark boxes acceptable.

### 2.5 This threat is realised, repeatedly, and the recipe is always the same

The verdict above is not theoretical caution. Every documented public-repo self-hosted-runner
compromise in 2023–2024 followed **the same three conditions**, and sparq would satisfy two of
them the moment it registered a runner:

1. the default fork-PR approval setting (approval required only for *first-time* contributors);
2. a **non-ephemeral** runner;
3. contributor status, which is trivially earned.

Condition 3 is the part that defeats intuition. In the PyTorch compromise the researchers wrote:
*"We needed to be a contributor… Instead, we found a typo in a markdown file and submitted a
fix."* The Microsoft DeepSpeed entry point was "an extra 'the' in `SECURITY.md`". A merged typo is
enough. Outcomes included persistence on the runner, theft of `GITHUB_TOKEN` from the
`.git/config` that `actions/checkout` leaves on disk, escalation to organisation-wide PATs, and
write access to production release artifacts. Chia Network's own published post-mortem — the only
victim-side account — concluded they had to treat **code-signing** material as exfiltrated.

Three details are directly load-bearing for this design, and each is easy to get wrong:

- **Approval gating is not a security control for self-hosted runners — GitHub says so.** Its
  docs warn that a malicious user "could meet this requirement by getting a simple typo or other
  innocuous change accepted", and that the approval policies are intended to limit compute abuse
  "when using GitHub-hosted runners"; for self-hosted, "potentially malicious user-controlled
  workflow code will execute automatically if the user is allowed to bypass approval". So
  "we would just require approval" is **not** an answer, and it would in any case reintroduce a
  human gate into an autonomous fleet.
- **PyTorch declined the approval fix.** It is often retold as having adopted it; the primary
  source says they "opted to implement a layer of controls" instead. The remediations that *were*
  adopted elsewhere — TensorFlow's is the clearest — were "require approval for **all** fork PRs"
  plus "**`GITHUB_TOKEN` read-only for workflows running on self-hosted runners**", and Chia's and
  DeepSpeed's were structural moves to ephemeral runners.
- **A rogue self-hosted runner is now a malware primitive.** The Shai-Hulud 2.0 npm worm
  (Nov 2025) registered its *own* self-hosted runner and used a deliberately injectable workflow
  as its command-and-control channel — chosen partly because all its traffic goes to
  `github.com`, so egress allowlisting does not see it. That is worth knowing defensively
  regardless of this design: an unexpected POST to the runner-registration API is a signal.

Note what is *not* in this list. The `tj-actions/changed-files`, Ultralytics, and Angular
compromises — and ArtiPACKED — involved **no self-hosted runners** at all; they were
`pull_request_target` misuse, unpinned action tags, cache poisoning, and artifact hygiene. They
are out of scope here, but they reinforce the same principle from another direction: the
dangerous thing is untrusted code reaching a privileged context.

## 3. Provider comparison

Verification is marked per row. `VERIFIED` means a provider documentation page states it;
`PRESS` means multiple independent outlets report it and the provider published nothing;
`COMMUNITY` means forum/blog reports only. Numbers I could not source are left blank rather
than guessed. All figures as of 2026-07-26.

### 3.1 The baseline we already have

| Runner | vCPU | RAM | Disk | Minutes | Verification |
|---|---|---|---|---|---|
| `ubuntu-latest` (public repo) | 4 | 16 GB | 14 GB | free, unlimited | VERIFIED |
| `ubuntu-24.04-arm` (public repo) | 4 | 16 GB | 14 GB | free, unlimited | VERIFIED |
| Larger runners | 8–64 | — | — | **always charged, even on public repos** | VERIFIED |

Concurrent-job caps: **Free 20, Pro 40, Team 60, Enterprise Cloud 500** (standard runners);
job cap 6 h. Source: <https://docs.github.com/en/actions/reference/limits>. The 14 GB disk, not
CPU or RAM, is the constraint most likely to bite a 37-crate Rust workspace.

### 3.2 Always-free VM tiers — all fail, and the reasons are instructive

| Provider | Free compute | Arch | Enough for a cargo build? | How it fails | Verification |
|---|---|---|---|---|---|
| **Oracle Always Free** | **2 OCPU / 12 GB total per tenancy** (1–2 A1 instances), 200 GB block, 10 TB/mo egress | ARM | Marginally — below the free GitHub runner | Hard `InternalError` "Out of host capacity" on launch, **no queue**; idle reclamation; retroactive quota cuts | VERIFIED |
| Oracle A1, historical | 4 OCPU / 24 GB | ARM | — | **Halved 2026-06-15 with no announcement**; over-limit instances disabled then deleted after 30 days | VERIFIED (docs); PRESS (the cut) |
| Oracle `E2.1.Micro` | 2 × (1/8 OCPU, 1 GB) | x86 | No | — | VERIFIED |
| **IBM Cloud** | No free general-purpose VM at all. Code Engine: 100k vCPU-s + 200k GB-s/mo, ephemeral storage only | — | **No** — ≈7 h/mo of a 4 vCPU/8 GB container, no persistent disk | Free Basic Support moved to self-service Jan 2026 | VERIFIED |
| Google Cloud | 1 × `e2-micro`, **1 GB RAM**, 1 GB/mo egress | x86 | No | Trial expiry closes billing account, resources stop | VERIFIED |
| AWS | **No always-free EC2.** Free Plan ≈ $200 over 6 months | — | Burst only | **Account closes 6 months in, or when credits run out** | VERIFIED |
| Azure | 750 h/mo of B1s / B2pts v2 / B2ats v2, 12 months, **1 GB RAM** | both | No | Credit expires at 30 days | COMMUNITY |

Three cross-cutting hazards make this whole category unsuitable as a dependency:

- **Oracle's June 2026 halving is the governing precedent.** A documented allocation was cut
  50 % with no blog post, email, or notice; users discovered it when instances were shut down.
  Any design that treats a free tier's published limits as stable is unsound.
- **No free-tier capacity guarantee exists.** Oracle capacity reservations are explicitly
  unavailable to Free Tier accounts, and reserved capacity is billed even when unused. The one
  useful primitive is `CreateComputeCapacityReport`, a pre-flight capacity check — which is
  exactly the "detect, do not assume" hook §5 needs.
- **Idle reclamation inverts the value proposition** (see §2.2).

### 3.3 Dead programmes — do not spend time on these

Four candidates in the brief or in common circulation are defunct. Recording them prevents a
future agent re-researching them:

| Programme | Status | Verification |
|---|---|---|
| **Cirrus CI** | **Ceased operations 2026-06-01** (Cirrus Labs joined OpenAI) | VERIFIED |
| **CNCF Community Infrastructure Lab** | **Decommissioned 2025-12-31**, repo archived. Its landing page is **STALE and still advertises the programme** | VERIFIED |
| **Equinix Metal / Works on Arm** bare metal | **Platform sunset 2026-06-30**. The Works-on-Arm page is now an aggregator; its "free access" post is from 2022 | VERIFIED |
| **GitHub for Startups** | Grants Enterprise **seats, not compute**; requires VC/accelerator referral | COMMUNITY |

Startup credit programmes (AWS Activate, Google for Startups, Microsoft Founders Hub, Oracle for
Startups) are all gated on being a privately-held for-profit entity, usually with a
domain-matched business email, and the large tiers require equity funding. **An unincorporated,
unfunded, single-maintainer OSS project fails the eligibility gate on essentially all of them.**
They should be treated as unavailable, not as a backlog.

### 3.4 What is actually obtainable — the OSS-programme category

This is the category the brief correctly guessed would be the best fit, and it is.

| Option | Offer | Shape | How it fails | Verification |
|---|---|---|---|---|
| **CircleCI open source** | **400k credits/mo** free, public repo | ≈28,500 min/mo of 4 vCPU/16 GB Linux, or ≈20,000 min/mo of 4 vCPU/16 GB ARM | **Hard cutoff** — blocked until credits refill next month. No surprise bill | VERIFIED |
| **Blacksmith** | 3,000 min/mo free, no card; explicit OSS sponsorship programme | Managed ephemeral Actions runners; Ubuntu ARM at 6/12 vCPU | Provider outage → job cannot start | VERIFIED |
| **Depot** | 2,000 Actions min/mo free, unlimited concurrency | Managed ephemeral Actions runners | Ditto | VERIFIED |
| **GitLab for Open Source** | Ultimate + 50k compute min/mo | Cost-factor multiplied → ≈16,600 effective min/mo at 4 vCPU/16 GB | Requires OSI licence across the whole namespace, all projects public | VERIFIED |
| **OSU Open Source Lab** aarch64 | Free ARM VMs, application required | Explicitly *"intended for … continuous integration"* but **"not ideal for performance testing"** | "May discontinue hosting for any reason at any time" | VERIFIED |
| **Modal** | $30/mo free credits | ≈95 h/mo of 4 core/16 GB | Reported to stop workloads rather than bill | VERIFIED (rates); COMMUNITY (stop-not-bill) |
| AWS Promotional Credits for OSS | Unpublished amount, 1-yr expiry, explicitly covers CI/CD | — | Prefers multi-entity/foundation projects — weak fit for one maintainer | VERIFIED (programme); UNVERIFIED (2026 status) |
| Oracle Arm Accelerator | ≈$3,000 OCI credits / 12 mo | Would buy substantial A1 | — | COMMUNITY (amount) |

Cheap-not-free baseline, for cost comparison only: Hetzner **CAX31** (8 vCPU / 16 GB, ARM64) at
about €16/mo is the only family that stayed cheap after two 2026 price rises; CCX13 (4 vCPU /
16 GB dedicated) went to €42.99/mo. CAX is **shared**-vCPU, so unsuitable for canonical
benchmarks — the existing ephemeral-EC2 protocol remains the right home for those.

### 3.5 Managed Actions runners are a materially different security proposition

Blacksmith, Depot and similar are *not* self-hosted runners in the risk sense that §2.1 warns
about. They supply a **fresh ephemeral VM per job, managed by the provider, on the provider's
network**. That eliminates persistence risk (i) and moves position risk (ii) off any
maintainer-owned network. A fork PR running on one still gets no repo secrets, because GitHub
withholds them from fork `pull_request` events.

They are therefore the **correct overflow substrate if overflow is ever needed for
GitHub-Actions-shaped work** — far safer and enormously cheaper operationally than a
self-managed Oracle box, at the cost of trusting one more vendor with build-time code execution.
They are, however, a *supply-chain* trust decision (a third party executes our build), so they
warrant the maintainer's explicit sign-off before adoption, not an agent's.

Two discriminators to apply when choosing one, because the category is not uniform:

- **Insist on per-job teardown of a *virtualised* unit, not a container.** Per-job VM or microVM
  destruction (Firecracker/EC2-backed) is the only bar any vendor is willing to call
  public-repo-safe. Kubernetes-based `actions-runner-controller` destroys a **pod**, not a VM,
  and never claims otherwise — and its container-job modes require a **privileged** DinD
  container, where root in the container is root on the host. ARC is the wrong shape for
  untrusted code.
- **Prefer a vendor that needs no standing credential in our cloud.** The good end of the range
  authenticates via a GitHub App and runs the VM in *their* cloud, so a hostile fork PR gets a
  throwaway box with no path to our accounts. The bad end asks for long-lived
  administrator-grade cloud access keys pasted into a vendor dashboard, which is a strictly worse
  posture than what we have today. One vendor also markets per-job teardown as guaranteeing
  "absolute security", which is an overclaim — teardown does not address network position or
  cache poisoning. Treat vendor security pages as marketing, and check the credential model.

## 4. Recommendation

**Do not build free-cloud overflow now.** Consume the free headroom first, in this order. Each
step should be justified by the tripwire in §6 before it is taken.

1. **Raise the concurrency ceiling where the workers actually run** (§0.3). The worker fleet is
   in a personal account capped at 20/40 while `sparq-org` has Enterprise Cloud's 500. This is
   the single highest-leverage change available: roughly 12–25× worker-concurrency headroom for a
   configuration change, no new infrastructure, no new attack surface. **Caveat the maintainer
   must weigh:** a repository transfer does not carry secrets or environments across — the
   `dispatch-secrets` environment, `PROVENANCE_SALT`, and the App credentials would need
   recreating, and org-level Actions policy would then apply. Bounded work, but not zero, and it
   touches the credential estate, so it is the maintainer's call. Upgrading the personal account
   to Pro (40) is the trivial partial version.
2. **Fix worker yield.** At ~20 % yield, four in five worker launches produce nothing. This
   multiplies every other capacity lever and costs no infrastructure. It should precede all
   capacity work.
3. **Reduce demand, not just add supply.** `research/ci-runner-consolidation-2026-07.md`
   already established that most jobs are sub-minute and that runner-claim churn *feeds* the
   documented congestion-collapse mode. Consolidation is cheaper than capacity and reduces a
   real failure mode rather than masking it.
4. **Only then, if the tripwire fires:** add **managed** overflow — CircleCI's OSS plan for
   bulk gate legs, or Blacksmith/Depot as drop-in ephemeral Actions runners (§3.4, §3.5).
5. **Never:** register a self-managed runner against a public repo; overflow Class E or F; or
   place canonical benchmarks on shared-vCPU free capacity.

Top three providers, with the reason:

1. **GitHub itself** — the cheapest capacity is the concurrency headroom we are not using. Free,
   instant, no new trust boundary. It is not a third-party cloud, which is precisely why it wins.
2. **CircleCI open-source plan** — by far the largest genuinely free grant of the *exact* shape
   needed (≈28,500 min/mo at 4 vCPU/16 GB), with a hard-cutoff failure mode that cannot produce
   a surprise bill, and its own concurrency pool independent of GitHub's.
3. **Blacksmith / Depot** — managed ephemeral Actions runners: additive capacity that keeps the
   per-job destruction property, needs no runner operations, and never puts a self-managed host
   in front of a public repo. Modest free allowances, plus a real OSS sponsorship route.

Oracle Cloud, the brief's leading candidate, is **not recommended in any tier** — it now yields
less than one free GitHub runner, cannot guarantee provisioning, reclaims idle instances (the
overflow profile), and demonstrated in June 2026 that it will halve a documented allocation
without notice. IBM Cloud offers no usable free VM at all.

## 5. Failure semantics

The maintainer's requirement is that provider outages and capacity limits must not cause
problems in the workflows. That is achievable, but only if overflow is *additive by
construction* rather than by intention.

### 5.1 The load-bearing invariant

**No workflow may name an overflow runner.** `runs-on:` stays `ubuntu-latest`. Overflow is
requested by a controller job, never by the scheduler that assigns a job to a machine.

This is the whole failure-semantics design in one line, and it is why §2.4 picks the detached
executor over self-hosted runners. If a workflow ever says `runs-on: [self-hosted, oracle]`,
then a provider outage becomes a **permanently queued job** — the job cannot start, the gate
never concludes, and `ci-summary` hangs rather than fails. With no overflow label anywhere, that
failure mode does not exist to be mitigated. **Structural, not procedural.**

### 5.2 Fail-open to baseline — and precisely where that stops applying

This project has live experience on both sides. A *fail-closed* refusal on missing capacity data
caused a fleet-wide stall today; and separately, exit-zero fail-open behaviour has repeatedly
discarded earned hard failures. Neither posture is globally correct, so the rule must be stated
on the right axis:

> **Fail open on a *capacity* signal; fail closed on a *safety or authorisation* signal.**
> "Is there an overflow box available?" is capacity — unknown means *use baseline*. "Is this ref
> trusted?" / "did the gate pass?" is safety — unknown means *refuse*.

Today's stall happened because a capacity signal was wired to a safety-shaped response. Applied
here: a provider probe that errors, times out, or returns nothing means **no overflow this
time**, silently, and the job runs on a GitHub-hosted runner. It never means "error", and it
never means "skip the gate".

### 5.3 Required mechanisms

- **Provisioning failure is not job failure.** A controller that cannot obtain an overflow box
  falls through to baseline placement and reports success for the *placement* decision. The only
  thing that may fail the job is the gate's own verdict.
- **Detect capacity, never assume it.** Oracle returns a hard synchronous `InternalError` /
  "Out of host capacity" with no queue and no waitlist. Treat availability as a runtime
  observation with a short TTL, using `CreateComputeCapacityReport` as a pre-flight probe rather
  than blind launch-retry.
- **Bounded attempts with a wall-clock deadline.** The capacity lottery invites infinite retry;
  community retry-loop tooling exists precisely because people do this. Cap attempts *and* set a
  hard deadline (order of a minute), after which baseline wins. An overflow path that spends
  longer looking for a box than the job takes to run is a net loss.
- **Leases expire; they are not held.** A half-provisioned executor must not sit holding a
  lease. A lease is a record with a hard expiry timestamp, swept by a reaper — never a lock a
  dead controller can keep. Provisioning must be idempotent and abandonable.
- **Per-provider circuit breaker with cooldown.** Consecutive failures open the breaker; an open
  breaker means the provider is silently skipped. **A provider that vanishes — programme ended,
  account closed, quota halved — degrades to baseline with no code change**, because a
  permanently open breaker and an absent provider are the same state. Breaker state must be
  observable, or a silently-open breaker becomes a silently-dead feature.
- **Every overflow verdict is advisory.** `ci-summary` on GitHub-hosted runners stays the
  authoritative gate. This bounds the damage from a wrong, stale, or tampered overflow result to
  zero merge authority.
- **Orphan-proofing is mandatory and already solved.** Reuse the three-watchdog design and
  tag-scoped orphan sweep from `ec2-build-farm-design.md` §4/§6 verbatim.

### 5.4 Observed while writing this record: each provider is a multiplier on transient risk

The PR carrying this document went RED on its first CI run. The cause was not the document: a
`dorny/paths-filter` step failed with GitHub's own `"Sorry, this diff is temporarily unavailable
due to heavy server load"` server error, one gating leg concluded failure, and `ci-summary`
fail-fast RED'd the whole gate — correctly, per its "a newest-run failure is never forgiven"
rule. A re-run cleared it.

That is worth recording because it is a live instance of the exact class the maintainer asked to
design against, arriving from the provider we already depend on. Two lessons follow:

- **Fail-fast on a gating leg is right for *correctness* signals and expensive for *transient*
  ones.** The gate cannot distinguish them, so every additional provider in the critical path
  adds another independent source of transient RED that a human or an agent then has to
  triage and re-run.
- **Therefore each added provider carries a cost that is not capacity-shaped.** This is a second,
  independent argument for §5.1's invariant: overflow must sit *outside* the gating path, as
  advisory capacity, so that a provider's bad minute cannot RED anything. An overflow design that
  put a third-party runner inside the gate would import that provider's transient rate directly
  into the merge train.

### 5.5 A failure mode specific to free tiers

Oracle's idle reclamation stops an instance after 7 days below the utilisation thresholds, and a
stopped A1 instance **may fail to restart** because restarting re-enters the capacity lottery.
So a free-tier overflow box can be lost *permanently* by the very idleness that overflow implies.
A design depending on a warm pool of free instances is therefore unsound; only
launch-on-demand-and-accept-failure is honest here — which is another argument for §4's
preference for managed providers over self-managed free VMs.

## 6. Staged rollout, gated on measurement

This project has measured two of three plausible context optimisations as worthless. The same
prior applies here, and the base rate for "capacity infrastructure that was never needed" is
high. So **Phase 0 is a measurement, and it is the only phase authorised by this record.**

- **Phase 0 — instrument and wait (build this, and nothing else).** Record, per Actions job,
  the queue wait and the account's concurrent-job count; alert when either approaches a
  threshold. Cost: a small periodic sampler. This is worth doing on its own merits, because it
  also tells us how close the registry is to its 20/40 cap — currently the least-understood risk
  in the estate.
- **Tripwire — the falsifiable trigger for Phase 1.** Proceed only if a *measured* week shows
  either (a) ≥5 % of jobs queueing >60 s, or (b) the account concurrent-job count pegged at its
  cap for ≥15 min/day. **Today both are comfortably zero** (§0.1). If the tripwire never fires,
  this design record correctly ends at Phase 0 and we have spent almost nothing.
- **Phase 1 — consume free headroom.** The §4 items 1–3: concurrency ceiling, worker yield, job
  consolidation. No overflow substrate. Re-measure against the tripwire afterwards; expect it to
  stop firing here, and if it does, stop.
- **Phase 2 — managed overflow, one provider, one job class.** Only Class A/B pure-build legs,
  only via a managed ephemeral provider (§3.5), behind the circuit breaker of §5.3. Success
  criterion is a measured reduction in the tripwire metric, plus a **deliberate outage drill**:
  disable the provider and confirm throughput degrades to baseline with zero job failures.
- **Phase 3 — second provider, only if Phase 2's breaker and fallback proved themselves.**
  Two providers are worth the complexity only once the one-provider fallback is evidenced.
- **Phase 4 — never.** Self-managed always-free VMs as self-hosted runners on public repos, and
  any overflow of Class E or F.

Per the brief, **no implementation beads are created**: everything after Phase 0 depends on the
maintainer accepting the §2.3 security verdict first.

## 7. Cost — the operational burden, honestly

The compute is free; the *maintenance* is the price, and for the self-managed variant it is
badly mispriced.

**Recurring burden of a self-managed free-tier overflow substrate:** a machine image to build
and keep current; a toolchain to keep in step with the workspace's MSRV; capacity probing and
breaker tuning; orphan sweeping; lease reaping; credential rotation for the control path;
debugging failures on a substrate with **no support recourse** (IBM moved free Basic Support to
self-service in January 2026; Oracle free tenancies have no live support); and absorbing
unannounced terms changes like the June 2026 halving. Every one of those is a new source of
false CI signal, and this repo already documents how expensive a false gate signal is.

**Against that, the benefit:** one Oracle tenancy adds 2 OCPU / 12 GB — **less than one of the
4 vCPU / 16 GB runners GitHub already gives us free and unlimited** — to a system whose measured
queue wait is p99 5 s and whose enterprise org has 500 concurrent jobs available.

That comparison is not close, and it is the core finding of this record: **the self-managed
free-tier option costs a permanent operational tax and a new public-repo attack surface to buy a
fraction of one runner's worth of capacity we are not currently short of.**

The managed options price very differently. CircleCI's OSS plan and Blacksmith/Depot cost
roughly a day of integration, no ongoing host operations, no new self-managed attack surface,
and they fail by hard cutoff rather than by silent degradation. **If overflow is ever needed,
buy it there.** The one real cost is a supply-chain trust decision — a third party executing our
build — which is the maintainer's to make.

## 8. Open questions for the maintainer

1. **Is the §2.3 security verdict accepted?** Specifically: that Class E (workers) must not
   overflow onto self-managed free infrastructure. Everything downstream depends on it.
2. **Should the worker fleet move into `sparq-org`** to inherit Enterprise Cloud's 500-job
   ceiling (§4.1)? This is the largest free capacity win available, but it is a credential-estate
   change: secrets and environments do not transfer, and org Actions policy would then apply.
   What is `jeswr`'s current plan — Free (20) or Pro (40)? I could not read it with this token,
   and the answer sets how near the cliff actually is.
3. **Is third-party build execution acceptable?** Blacksmith/Depot/CircleCI all execute our
   build on their infrastructure. That is a supply-chain trust expansion, and it should be your
   decision rather than an agent's.
4. **Is the tripwire in §6 the right trigger**, and are its thresholds right? I chose ≥5 % of
   jobs queueing >60 s and cap-pegged ≥15 min/day as defensible-but-arbitrary; you may want them
   tighter or looser.
5. **Should Phase 0's sampler be built now?** It is cheap, it is useful independently of
   overflow, and it is the only thing this record recommends building.

## 9. Uncertainties and limits of this record

- **`jeswr`'s account plan is unverified** (the token lacks the scope). The 20-vs-40 concurrency
  cap materially changes how close the worker substrate is to its ceiling: observed peak 17 is
  85 % of Free but 43 % of Pro.
- **The ~20 % worker yield figure is taken from the brief**, not independently measured here.
- **Queue-wait measurements are a single-day snapshot** (2026-07-26, n=340 registry + n=39
  sparq). They are strong evidence that capacity is not binding *now*; they are not a claim
  about a future higher-throughput regime — which is exactly why Phase 0 is continuous
  instrumentation rather than a one-off check.
- **Peak-concurrency figures are lower bounds.** They are reconstructed from completed jobs in a
  recent window (sparq peak 44 in 9 min; registry peak 17 in 51 min), so they undercount.
- **Oracle's PAYG-unlocks-A1-capacity folklore is unverified** and contradicted by at least one
  unanswered Oracle-forum report. Do not design against it.
- **Oracle's idle-reclamation grace behaviour** (email, 7-day grace, stop-not-terminate) is
  community-sourced; only the utilisation thresholds are documented.
- Several provider pages were unreachable during research (Oracle's hosting-and-delivery-policies
  PDF 404'd; the Oracle CSA PDFs were not text-extractable; Cloudflare's billing page 404'd).
  Rows depending on them are marked COMMUNITY or left blank.
- **Two things a Phase-2 decision must test rather than assume.** (a) Whether pinning a
  runner-group workflow restriction to `…@refs/heads/main` actually denies a fork run, whose ref
  is `refs/pull/N/merge` — undocumented, and it is the one clean Enterprise-tier control, so it
  should be tested before anyone relies on it. (b) Whether `aws ec2 get-console-output` truncates
  the envelope stream the existing benchmark result-collection depends on: the 64 KB limit widely
  quoted for it **is not in current AWS documentation**, so it is folklore either way.
- **A pre-existing drift worth fixing independently of this design:**
  `research/ci-ec2-design.md` still specifies `jeswr/sparq` in both its `if:` guard and the OIDC
  trust-policy `sub`, while the remote is `sparq-org/sparq`. If the live IAM trust policy was
  built from that doc it would fail to assume; if it was widened to compensate, it is
  over-permissive. Filed separately rather than fixed here (this record is doc-only).
- **Ephemeral runners are reported to be flaky even when they work** — "lost communication with
  the server" on successful completion is a commonly reported failure mode. Any ephemeral design
  should budget for spurious failures, which is a further argument for keeping overflow out of
  the gating path (§5.4).
- **No provider account was created and nothing was provisioned** in producing this record. All
  provider figures are from documentation, not from observed provisioning — and given that
  Oracle's central failure mode is *provisioning*, a Phase-2 decision should include one manual
  attempt to actually obtain a box before committing to any design that assumes one.
