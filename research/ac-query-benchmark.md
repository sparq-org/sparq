# Design — an access-controlled-query benchmark (WAC + ACP + ODRL) with built-in correctness oracles, plus the sparq evaluation and the paper

> 🤖 **SPARQ agent** — decomposition design record for maintainer issue
> [#1613](https://github.com/sparq-org/sparq/issues/1613) (epic `sq-i6du2`). [FABLE]
> DESIGN only: this PR changes no crate code. It surveys the estate + the literature,
> answers the six design questions the directive poses, and decomposes the program into
> file-disjoint child beads the fleet can implement in parallel.

**Status:** DESIGN / decomposition. **Epic:** `sq-i6du2` (issue #1613).
**Maintainer directive (verbatim, from #1613):** *"could you make one of the papers a
benchmark for access controlled queries (which can benchmark WAC, ACP and ODRL by having
benchmarks for each) on datasets that effectively represent different use cases (e.g.
personal data storage, commercial project management like graph metrix, financial
services usage, other ...) and an evaluation of sparq against that benchmark."*

---

## 0. Premise check — what already exists (verified against `origin/main`, 2026-07-06)

The directive's premise is sound: **no such benchmark exists**, in this repo or (per the
bounded literature survey in §1.2) in the published literature. What DOES exist in-repo is
the substrate the benchmark measures, plus one micro-benchmark and one oracle design:

1. **The system under test is real and layered.** `crates/sparq-solid` evaluates WAC and
   ACP as N3 inference rules (`rules/{common,wac,acp-a,acp-b,acp-c}.n3` via
   `sparq_reason::reason_n3`), materializing an allow-list auth view in
   `<urn:sparq:auth>` (fail-closed: absence = deny — `research/solid-access-control-design.md`
   D4). Hot paths: per-request decisions (`PodStore::decide` / `decide_batch`, `&self`,
   backed by the persistent `AclIndex`, #1577), access-controlled query
   (`query_as`/`query_json_as`/`ask_as` over the zero-copy `DatasetView`, with the
   spec-compliant EMPTY default graph + per-request `FROM <…#union-default-graph>` opt-in
   of #1593), ACL-write + re-materialization + scoped invalidation (`update_as`,
   #1584/#1585), and generation pinning for paginated reads (#1572). ODRL arrives through
   `crates/sparq-policy` (Permission/Prohibition/Duty + constraint evaluation,
   fail-closed) bridged into the same auth view by `sparq-solid`'s default-OFF
   `odrl-bridge` feature (`odrl_bridge.rs`, `BridgeLedger::refresh` for re-checked
   conditional grants).
2. **One micro-benchmark exists** — `solid-wac-bench` (`bench/benchmarks.toml` id,
   `cargo run -p sparq-solid --example bench`): auth-view materialization +
   re-materialization time on an in-crate ~1.1k-graph fixture (`fixture.rs`:
   `wac_fixture` / `wac_fixture_sized` / `acp_fixture`). It is trend-only, single-model
   per variant, has no ODRL lane, no use-case datasets, no query-result oracles, and no
   per-model comparability design. It is a component probe, not a benchmark suite; the new
   suite subsumes rather than replaces it.
3. **An oracle design exists** — `research/solid-acp-differential-oracle-design.md`
   designs decision-parity oracles for the WAC/ACP conformance corpus and verified two
   importable, server-free JS reference evaluators: `@solidlab/policy-engine` (WAC + ACP;
   pre-1.0, research-grade) and `@solid/acl-check` (WAC only). This benchmark reuses that
   result (§2.3).
4. **The papers program has two open dispositions this benchmark resolves** (§4):
   the venue-bar audit (`research/papers-venue-audit.md`) marked `solid-acl-conformance`
   MERGE (its merge target was itself killed — bead `sq-3ovwp` asks fold-or-kill), and the
   `odrl-policy-bridge` paper's §5.3 comparative decision-agreement study vs ODRE is
   *specified but not run* (the paper says so explicitly and stays non-submittable until
   it runs).
5. **In-flight work this design must not collide with:** PR #1612 (`&self` read-side +
   sharded session cache, closing #1569 — the last blocker of the PSS beyond-50k track
   #1604) touches `crates/sparq-solid/src`; issue #1582 (epic `sq-zgbso`) may move ODRL
   evaluation to compiled N3 rules and change `sparq-policy`/`odrl_bridge` internals.
   Consequence: **the benchmark lives in a NEW dev-only crate + `bench/ac/`, touches no
   `sparq-solid`/`sparq-policy` source file, and benchmarks at the public spec surface**
   (`decide*`/`query_as*`/`update_as*`/`materialize_odrl_*`) so #1582-class internal
   changes re-run under it unchanged.

## 1. Survey

### 1.1 What makes an RDF benchmark credible (the methodology we inherit)

The canonical RDF benchmarks — LUBM (Guo, Pan, Heflin, *LUBM: A benchmark for OWL
knowledge base systems*, JWS 2005), SP²Bench (Schmidt et al., ICDE 2009 /
[arXiv:0806.4627](https://arxiv.org/abs/0806.4627)), BSBM (Bizer & Schultz, IJSWIS 2009),
and WatDiv (Aluç et al., ISWC 2014) — share four properties this design adopts wholesale:
(i) a **parameterized deterministic generator** (scale factor + shape knobs, seeded);
(ii) **workload classes** derived from an explicit use-case analysis, not ad-hoc queries;
(iii) **published expected results** (SP²Bench and this repo's tiered
`bench/{sp2b,watdiv,bsbm,lubm}` harnesses gate on expected-rows diffs); and (iv) an
explicit statement of **what real-world property each parameter models** (WatDiv's
structuredness critique of LUBM/BSBM/SP²Bench is the cautionary tale: uniform generators
hide the workloads that hurt). This repo's own conventions (`bench/CATALOG.md`:
differential correctness as a gate, seeded generators, quiet-box discipline, tiered
per-commit vs EC2/nightly scale) are the local instantiation and apply unchanged.

### 1.2 Prior art on ACCESS-CONTROL benchmarking (bounded web survey, 2026-07-06)

Verified by bounded WebSearch; each claim below is at survey strength (what the cited
material states, not exhaustive proof of absence):

- **SolidBench** ([SolidBench/SolidBench.js](https://github.com/SolidBench/SolidBench.js);
  Taelman & Verborgh's link-traversal evaluation line,
  [arXiv:2302.06933](https://arxiv.org/abs/2302.06933)) simulates Solid vaults with
  LDBC-SNB-derived social-network data and is the nearest benchmark to this one — but its
  workloads query the fragmented data itself; **access-control policies are not a
  benchmark dimension** (no ACL-shape parameters, no per-model WAC/ACP/ODRL lanes, no
  decision oracles in its documented feature set).
- **SAFE** (Khan et al., *SPARQL Federation over RDF Data Cubes with Access Control*,
  J. Biomedical Semantics 2017) is a policy-aware federation *engine* with its own
  clinical-cube evaluation — a system paper's bespoke eval, not a reusable parameterized
  benchmark, and single-model (its own graph-level policy formalism, not WAC/ACP/ODRL).
- **ODRL correctness suites exist; ODRL benchmarks do not.** The ODRL Evaluator +
  Compliance Report Model line (*Interoperable Interpretation and Evaluation of ODRL
  Policies*, ESWC 2025) and the FORCE spec suite (CEUR OPAL 2025) define
  policy/request/state test cases with expected compliance reports — correctness
  artifacts, no datasets, scale factors, or performance workloads. **ODRE**
  ([arXiv:2409.17602](https://arxiv.org/abs/2409.17602), Python + Java implementations)
  reports its own enforcement experiments — again system-specific, not a shared benchmark.
- **The non-RDF AC world has the shape we want**: XACML PDP benchmarking is a mature
  line — Turkmen & Crispo (SWS 2008), XEngine (SIGMETRICS 2008), through **XACBench**
  (Soft Computing 2020: synthetic + real policy sets, request generators, PDP throughput
  comparison). Nothing analogous exists for the RDF/Solid access-control models.

**Novelty claim, at survey-verified strength:** we found no published benchmark that (a)
parameterizes datasets AND access-control shape for RDF/SPARQL engines, (b) covers WAC,
ACP, and ODRL side-by-side with a documented expressibility mapping, and (c) gates every
workload on built-in decision/result oracles. That absence — with SolidBench (no AC
dimension), SAFE (bespoke eval), ODRL-Evaluator/FORCE (correctness only), and XACBench
(XACML, non-RDF) as the nearest neighbours to cite — IS the paper's contribution claim,
and it is a Resources-track-shaped claim (ISWC Resources / ESWC Resources), consistent
with the venue audit's finding that this repo's strongest papers are measured artifacts.

### 1.3 Use cases (the maintainer's list, made concrete)

Four generators, each exercising a DIFFERENT region of the ACL-shape parameter space —
that difference is what makes them "effectively represent distinct use cases" rather than
one dataset with four vocabularies:

| Use case | Data shape | AC shape it stresses |
|---|---|---|
| **U1 personal data storage** (Solid pod) | many small pods: profile, contacts, photos, notes, health readings | owner-centric; deep container inheritance; friend/family groups (nesting); public/private/shared mix; app-restricted access (ACP `acp:client` native; WAC `acl:origin` approximation) |
| **U2 commercial project management** (Graphmetrix-inspired shape — construction/industrial doc management; **no trademark in benchmark names**: id `project-mgmt`) | orgs → projects → sites → document sets (drawings, RFIs, submittals); cross-org subcontractors | role/team groups with **cross-org group reuse**; wide flat containers; "all-except" intents (deny-shaped — ACP/ODRL native, WAC inexpressible); handover/revocation churn |
| **U3 financial services** | institution: clients, accounts, transactions, advisory docs; auditors + regulators | strict compartmentalization (low public mix); high policy fan-in per resource; ODRL duties/constraints (retention windows, purpose-of-use, count-limited access); audit-trail reads |
| **U4 research-data consortium** | datasets, papers-in-progress, instruments, consortium membership rolls | **temporal** ODRL constraints (embargo-until); very large flat groups; public-after-embargo flips (churn workload); authenticated-agent-wide grants |

## 2. The six design decisions

### 2.1 Benchmark dimensions (decision: one parameter vector, four workload classes)

One `GenParams` vector shared by all four generators (each use case pins different
defaults — that is its identity):

- `seed: u64` — SplitMix64 throughout (repo convention); same seed ⇒ byte-identical
  corpus.
- `sf: u32` — scale factor. SF=1 sizes each use case at roughly the repo's per-commit
  tier (order 10⁴ resources / 10⁵ triples, comparable to `watdiv` SF=1 / `bsbm`
  per-commit); SF∈{10, 100} are the EC2/nightly tiers. Resource, agent, and policy counts
  scale linearly; shape knobs do not.
- `container_depth: u8` (0–6) — LDP container-tree depth. Models pod folder hierarchies;
  drives the WAC nearest-ancestor vs ACP cumulative-ancestor inheritance divergence
  (§2.2).
- `acl_coverage: f32` — fraction of resources carrying their OWN `.acl`/`.acr` vs
  inheriting. Models "most resources inherit" reality; drives effective-ACL discovery
  cost.
- `group_nesting_depth: u8` (0–4) + `members_per_group: u32` — `vcard:Group` chains.
  Models org/team structure; drives group-closure cost and the ACP no-group-matcher
  expansion (§2.2).
- `mix: (public, private, shared)` — fractions summing to 1. Models audience
  distribution; drives auth-view size and query selectivity.
- `policies_per_resource: u8` — ODRL policy (and ACP policy) fan-in. Models accreted
  real-world policy sets (XACBench's headline dimension).
- `constraint_complexity: 0..=3` — ODRL: none / temporal (`dateTime` window) / purpose or
  count / compound `and`-of. Models usage-condition richness; levels 1–3 are ODRL-only by
  construction (§2.2).
- `n_agents: u32` — request-population size.

Four workload classes, each parameterized by (use case × model × SF):

- **W1 decision micro-benchmark** — batches of `(agent, client, resource, mode)` tuples
  through `decide`/`decide_batch` (WAC), the ACP materialized path, and the ODRL bridge;
  fixed allow:deny ratio by construction. Measures: decisions/s (trend), materialization
  time, `AclIndex` warm vs cold.
- **W2 access-controlled SPARQL** — `query_as` under four query classes: Q-point
  (explicit `GRAPH` lookup), Q-scan (container listing via the #1593 per-request
  `FROM <…#union-default-graph>` opt-in), Q-join (cross-pod/cross-project join), Q-agg
  (COUNT over the authorized subset). Every query ships an expected result set (§2.3).
  Also doubles as a source of upstream spec-test candidates for `solid-sparql-query`
  (standing rule, #1546).
- **W3 ACL-write + invalidation churn** — interleaved grant/revoke/group-membership/
  policy-add writes (`update_as`, ODRL `refresh_odrl_grants`) with W1/W2 probes between
  writes; expected post-write decisions by construction. Exercises exactly the
  #1584/#1585 re-materialization + scoped-invalidation surfaces; the oracle makes
  stale-grant bugs a benchmark FAILURE, not a speedup.
- **W4 concurrent readers** — N threads of W1 batches (available today: `decide_batch` is
  `&self`) and of W2 Q-point (gated on the #1612 `&self` read-side landing; the harness
  emits `SKIPPED(blocked: #1569)` for this sub-lane until then, never a fake number).
  This is the benchmark expression of the PSS beyond-50k scenario (#1604).

### 2.2 Per-model comparability (decision: intent-table IR + per-model compilers + an expressibility matrix as a reported RESULT)

WAC, ACP, and ODRL have genuinely different semantics; pretending one policy set is "the
same" in all three would be dishonest. Design:

- Each generator emits a model-agnostic **intent table**: rows
  `(audience, scope, mode, condition)` with `audience ∈ {owner, agent(a), group(g),
  public, authenticated, client-restricted(c), all-except(a)}`,
  `scope ∈ {resource, subtree}`, `condition ∈ {none, until(t), purpose(p), count(n)}`.
- Three **compilers** lower each intent to concrete policy triples where the model can
  express it: WAC (`acl:Authorization`; `acl:default` placement for subtree),
  ACP (`acp:Policy` + matchers; `acp:memberAccessControl` for subtree; `acp:deny` for
  all-except), ODRL (Permission/Prohibition, `odrl:PartyCollection` for groups,
  constraints for conditions).
- **Documented, deliberate asymmetries** (each an expected finding, not a bug):
  WAC has **no deny** ⇒ `all-except` compiles to enumerated allows (policy-size blowup
  measured) — and where enumeration would be unbounded, the intent is UNSUPPORTED in WAC;
  ACP has **no group matcher** (`acp:agent`/`acp:client`/`acp:issuer`/`acp:vc` only) ⇒
  `group(g)` expands to per-member matchers (blowup measured) while WAC uses
  `acl:agentGroup` natively; WAC `acl:origin` only approximates ACP's
  `acp:client`; `condition ≠ none` is **ODRL-only**; WAC nearest-ancestor vs ACP
  cumulative inheritance forces different placements for the same subtree intent.
- The **expressibility matrix** (intents × model → native / expansion(factor) /
  approximation / unsupported) is emitted by the generators as a deterministic artifact
  and reported in the paper as a first-class result. Per-model timing comparisons are
  restricted to the native∩native cells; expansion cells compare *cost of equivalent
  intent*, labeled as such.

### 2.3 Correctness oracles (decision: by-construction procedural oracle, fail-closed harness, independent second oracles where they exist)

The credibility feature: **an unsound implementation must FAIL this benchmark, not win
it.**

- **Primary oracle — by construction, independent of the system under test.** Expected
  decisions are computed at generation time by a small procedural evaluator over the
  intent table + group closure + container placement — it never touches sparq's N3
  rules, `AclIndex`, or `sparq-policy` evaluation. Per-model (not per-intent): where a
  compiler approximates (e.g. WAC enumeration), the oracle evaluates the COMPILED policy
  semantics for that model, so expectations stay exact. This is the "independent
  procedural reading" oracle shape `research/solid-acp-differential-oracle-design.md`
  recommends building first.
- **Query oracles.** Every W2 query has a closed-form expected result set: datasets are
  generated so Q-point/Q-scan/Q-join/Q-agg answers are computable as
  (generated data ∩ expected-allowed graphs) by the same procedural evaluator. W3 ships
  expected decision DELTAS per write.
- **Fail-closed harness.** Any decision mismatch, any result-set mismatch, any
  post-churn stale grant ⇒ nonzero exit; NO timing is reported for a failed lane. The
  workload engine's own test suite includes a deliberately-miscompiled policy fixture
  proving the harness fails on it (the oracle's oracle — anti-vacuity).
- **Second oracles (differential, optional lanes):** `@solidlab/policy-engine` (WAC+ACP)
  and `@solid/acl-check` (WAC) as offline pinned JS lanes; ODRE as the ODRL
  decision-agreement lane (§4.2); the W3C ODRL-Evaluator test cases as an imported ODRL
  correctness set (stretch, recorded as future work). These catch
  by-construction-oracle bugs; divergences are classified, never silently dropped.

### 2.4 Dataset generators (decision: one dev-only crate, four file-disjoint generator modules)

New workspace crate **`crates/sparq-acbench`** (dev/bench-only, `publish = false`,
adds NOTHING to `sparq-core`/`sparq-engine` dependencies — opt-in-crate architecture):
`GenParams` + SplitMix64 determinism + the intent-table IR + the three compilers + the
procedural oracle in the scaffold; one module per use case (`personal.rs`,
`project_mgmt.rs`, `financial.rs`, `consortium.rs`) each emitting: N-Quads data graphs,
per-model policy graphs, the intent table, request tuples + expected decisions, W2
queries + expected results, and a W3 churn script. Every `GenParams` knob documents the
real-world property it models (§2.1 wording goes into the rustdoc verbatim). Determinism
is a tested invariant (same seed ⇒ byte-identical output; golden hashes pinned).

### 2.5 Evaluation scope (decision: sparq now with honest labels; competitors only where comparison is real)

- **sparq evaluation now**: work-box runs are **non-canonical** (per
  `bench/CATALOG.md` quiet-box discipline) — report deterministic metrics (oracle
  pass-rates, corpus sizes, expressibility matrix, policy-blowup factors) as canonical,
  wall-clock as indicative trend ratios. Canonical wall-clock is EC2-gated (#1364);
  **the paper stays DRAFT until canonical runs**.
- **Competitors, honestly scoped**: decision-path lanes vs `@solidlab/policy-engine`
  (WAC+ACP) and `@solid/acl-check` (WAC) — feasible offline, but cross-language JS-vs-Rust
  wall-clock is reported as indicative context only, never a headline; ODRL vs **ODRE**
  (pinned Python implementation) as a decision-AGREEMENT study first, timing second
  (§4.2). **No comparable system is benchmarked for W2** in v1: CSS enforces per-HTTP-
  request (not SPARQL-over-pods), SolidBench/Comunica query without AC enforcement, and
  the proprietary engines with graph-level security (Stardog named-graph security,
  GraphDB FGAC) are out of v1 scope — the paper states W2 as a sparq-evaluation +
  open-harness contribution others can run, which is the honest framing.
- The benchmark harness registers in `bench/benchmarks.toml` (+ `bench/CATALOG.md`), so
  the run-all-benchmarks catalog runner (`sq-hz0g2`, in flight) enumerates it with no
  coupling to that script's implementation.

### 2.6 Where results live (decision: bench/ only; the paper cites)

`bench/ac/RESULTS.md` is the numeric home (environment-labeled, per repo convention);
`site/src/data/paper-evidence.json` receives ONLY deterministic records (oracle
pass-counts, expressibility-matrix counts, corpus sizes, blowup factors); the `.typ`
paper binds numbers exclusively through the evidence file (the build-papers.mjs honesty
gate enforces this). No perf number in any markdown outside `bench/`.

## 3. Decomposition — child beads (file-disjoint)

All beads live under epic `sq-i6du2`. **Disjointness:** no two beads touch the same
file. B2–B6 share the new crate but edit only their own pre-created module + test files
(the scaffold creates compiling stubs precisely so later beads never edit `lib.rs` or
`Cargo.toml`); they are dispatchable in parallel once B1 lands. B7/B8 sequence on
`bench/benchmarks.toml` (one writer at a time).

| # | Bead | Surface (files) | Tier | Invariant | Acceptance test |
|---|---|---|---|---|---|
| B1 | scaffold `sparq-acbench` | `crates/sparq-acbench/**` (new: Cargo.toml, README, `src/lib.rs` = GenParams + intent IR + compilers + procedural oracle + stub modules `personal/project_mgmt/financial/consortium/workload/oracle`, `tests/determinism.rs`), root `Cargo.toml` members | sonnet | determinism (seed ⇒ byte-identical) + zero new deps on sparq-core/engine | `cargo test -p sparq-acbench` |
| B2 | U1 personal-data-storage generator | `crates/sparq-acbench/src/personal.rs`, `tests/personal.rs` | sonnet | oracle ground truth by construction (independent of sparq's evaluators); fail-closed default (unlisted ⇒ deny) | `cargo test -p sparq-acbench --test personal` |
| B3 | U2 project-management generator | `src/project_mgmt.rs`, `tests/project_mgmt.rs` | sonnet | same as B2 + all-except intents carry per-model expressibility entries | `cargo test -p sparq-acbench --test project_mgmt` |
| B4 | U3 financial-services generator | `src/financial.rs`, `tests/financial.rs` | sonnet | same as B2 + constraint-bearing intents are ODRL-only in the matrix | `cargo test -p sparq-acbench --test financial` |
| B5 | U4 research-consortium generator | `src/consortium.rs`, `tests/consortium.rs` | sonnet | same as B2 + embargo flips produce exact W3 decision deltas | `cargo test -p sparq-acbench --test consortium` |
| B6 | workload + oracle engine (W1–W4) | `src/workload.rs`, `src/oracle.rs`, `tests/workloads.rs` | opus | fail-closed: ANY decision/result/churn mismatch ⇒ nonzero exit, no timing for failed lanes; includes the deliberately-miscompiled negative fixture proving the harness fails on it; W4 query sub-lane emits `SKIPPED(blocked: #1569)` until the `&self` read-side lands | `cargo test -p sparq-acbench --test workloads` |
| B7 | harness scripts + registry | `bench/ac/run.sh`, `bench/ac/README.md`, `bench/benchmarks.toml` (new entries), `bench/CATALOG.md` (row) | haiku | G3 registration gate passes; SF=1 smoke runs all models fail-closed; no perf number in markdown | `python3 scripts/check-new-bench-registered.py && bash bench/ac/run.sh --smoke` |
| B8 | ODRE decision-agreement lane (ODRL) | `bench/ac/odre/**` (runner, pinned requirements, mapping notes, `agreement-report` schema) | opus | honest divergence ledger: agreement claimed only from run output; every divergence classified (mapping-gap / semantics-gap / implementation-bug); no unqualified correctness claim for either system | `bash bench/ac/odre/run.sh --smoke` produces a complete agreement report; run fails on unclassified divergence |
| B9 | sparq evaluation run (provisional) | `bench/ac/RESULTS.md` (new) | haiku | every wall-clock number labeled non-canonical work-box; deterministic metrics separated; oracle pass required before any number is recorded | full `bench/ac/run.sh` at SF=1 (all four use cases × three models) exits 0; docs-quality gates pass |
| B10 | paper + `solid-acl-conformance` fold | `site/papers/ac-query-benchmark.typ`, `site/src/data/papers.ts`, `site/papers/solid-acl-conformance.typ` (retire/supersede) | opus | DRAFT status until canonical EC2 runs (#1364); numbers only via `paper-evidence.json` bindings; novelty stated at §1.2 survey strength; folded conformance content keeps its no-security-claim scoping | `node site/scripts/build-papers.mjs` compiles + both honesty gates pass |

**Dependency edges (real ordering only):** B1 → {B2,B3,B4,B5,B6}; {B2,B6} → B7;
{B4,B7} → B8; B7 → B9; B9 → B10. Dispatchable NOW: **B1**. After B1: B2–B6 in parallel
(file-disjoint within the crate).

**Review note:** B6 defines the ground truth the authorization surface is judged
against, and B8/B10 carry cross-system and publication claims — hold these three for
adversarial/Fable review before arming (arm-timing discipline); B2–B5/B7/B9 are
fleet-armable on green + Copilot-clean.

## 4. Dispositions this record decides (proceed-and-document)

1. **`sq-3ovwp` (solid-acl-conformance fold-or-kill): FOLD INTO THIS PAPER.** The
   24-scenario WAC/ACP decision-parity corpus + ratchet is exactly the
   oracle-validation evidence a benchmark paper needs (it validates the system under
   test agrees with hand-derived spec readings) and was only ever padding as a
   standalone paper. B10 retires the standalone entry and folds the content as the
   benchmark paper's conformance/oracle-validation section, preserving its honest
   scoping (decision parity, not wire conformance, no security property claimed).
2. **`odrl-policy-bridge` §5.3 pending decision-agreement study vs ODRE: THIS BENCHMARK
   IS THE VEHICLE.** B8 runs the study over the benchmark's ODRL corpora (U3/U4 are
   constraint-rich by design); the odrl-policy-bridge paper then cites the benchmark's
   study instead of running its own — one study, two papers strengthened, no
   duplication. The bridge paper remains non-submittable until B8 lands.
3. **#1582 (ODRL as compiled N3) does not block anything here**: the benchmark binds to
   the public spec surface, so a compiled-rules ODRL engine re-runs under the identical
   harness — and the benchmark is precisely the no-regression instrument that program
   needs.
4. **#1569/#1612 (concurrent readers)**: W4's query sub-lane is dep-marked on the
   `&self` read-side, emitted as SKIPPED until it lands — no fake concurrency numbers.
5. **Paper status**: DRAFT (family A/B Resources-shaped) until canonical EC2 wall-clock
   (#1364); submission targeting is a later maintainer call.

## 5. Honesty constraints (restated as build requirements)

- No security/soundness property of sparq (or any compared system) is claimed —
  benchmark PASS means "agrees with the by-construction oracle on this corpus", stated
  exactly that way in harness output, RESULTS.md, and the paper.
- No ZK/MPC surface is touched by this program; if a future lane composes with the
  privacy estate, it inherits the external-audit gating (`sq-qhy4`) before any claim.
- Work-box numbers non-canonical; deterministic metrics only as canonical; the paper's
  eval section binds through `paper-evidence.json` or does not exist.
- The novelty claim stays at survey-verified strength (§1.2) and names the nearest
  neighbours; if a closer benchmark surfaces during B10's related-work pass, the claim
  narrows rather than the citation disappearing.
