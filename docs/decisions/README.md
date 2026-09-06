<!-- [OPUS-4.8] SPARQ agent — architectural decision ledger. Durable home for the
     proceed-and-document design calls that were taken autonomously and only awaited
     retroactive maintainer review. Terse: one row per decision. Do NOT narrate. -->

# Decision ledger (ADR-style)

> 🤖 SPARQ agent record. This is the durable home for design/judgment calls made under
> the **[`proceed-and-document`](../../.claude/skills/proceed-and-document/SKILL.md)**
> standing rule — the maintainer's greenlight was not required, the call was made and
> shipped, and a short GitHub "steer" issue tracked it for post-hoc review. Those issues
> were consolidated here and closed; this table is now the record. Each row is a decision
> that is **reflected in merged code/docs** (evidence linked). Status is `adopted` unless a
> later decision `superseded` it.

**Scope caveat (ZK/MPC/trust estate).** Several rows concern the ZK / MPC / trust estate.
They record *design choices only*. That estate is **not externally audited** (`sq-qhy4`
open), the MPC layer is honest-majority **semi-honest only**, and **no** production privacy
or soundness guarantee is claimed here — the load-bearing caveats live in `SECURITY.md` and
the `research/zk-*-audit.md` records and are unchanged by this ledger.

Columns: `Issue` = the steering issue that captured it · `Evidence` = the merged PR (and,
where relevant, the design record) · GitHub auto-links `#NNNN`.

**Separate estate — the imported LWS server.** `crates/sparq-lws-core` was imported from
another repository and brought its own, independently-numbered `decisions/` tree with it.
Those are **not** proceed-and-document calls made here, so they are not rows in this table;
their durable in-repo home is
[`research/lws-design-records.md`](../../research/lws-design-records.md). Read that record
before interpreting any bare `decisions/NNNN` citation inside `crates/sparq-lws-core/**` —
its §2 documents a genuine number collision between two upstream ADR trees.

## Reasoner (OWL RL / EL / QL / DL, RIF, N3)

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1668 | 2026-07-06 | RL workstream (sq-pbz04.1) audited complete-as-scoped, recommend close with zero new beads; a six-bead in-profile EL wave-2 elected under sq-pbz04.2 | all 13 RL divergences dispositioned permanent; most EL divergences are addressable inside the OWL 2 EL grammar; all opt-in/default-off | #1667 · `research/owl2-rl-el-wave2-disposition.md` | adopted |
| #1454 | 2026-07-03 | OWL 2 Direct Semantics v1 = layered profile-membership checker then ALCH tableau then guarded dispatch in a new opt-in `sparq-reason-dl` (no SROIQ(D)); impl beads created dependency-gated ahead of ratification | profile-identification tests dominate the corpus (zero reasoning risk); each bead points at the record so amending it steers | #1453 · `research/owl2-direct-semantics-scoping.md` | adopted |
| #1447 | 2026-07-03 | RIF builtins adopt `sparq_substrate::numeric` inside the N3 chainer (substrate becomes a non-optional dep of the opt-in `sparq-reason`); three sign-sensitive builtins deferred-not-mapped; a core-fidelity Equal-in-head bug filed | one adoption serves both RIF and N3 dialects; mapping negative-operand builtins would derive wrong values | #1444 · `research/rif-core-conformance-and-builtins.md` | adopted |
| #1307 | 2026-06-29 | hold the OWL 2 RL inference ratchet; all 13 conformance divergences are provably beyond-profile, so they stay documented divergences (not missing rules) | RL omits the contrapositive/class-expression rules to stay polynomial; adding them would be a beyond-RL extension | sq-350ms · `research/inference-completeness-audit.md` §2b | adopted |
| #1163 | 2026-06-22 | the OWL 2 EL classifier ships as a standalone opt-in crate `sparq-reason-el` (not an `el` feature); differential oracle = hand-derived CR1–CR5 closures, not ELK-on-CI | cleaner dependency isolation; a JVM plus a network-fetched ELK jar on the gate is fragile against the lean-default discipline | #1162 | adopted |
| #1201 | 2026-06-22 | the secprop admissible-proof-set N3 ruleset lives in a default-OFF `secprop-admissibility` feature on `sparq-trust`, not in `sparq-reason`; only `odrl:gteq` is reduced, every other operator fails closed | keep `sparq-reason` domain-agnostic; fail-closed never over-admits | #1200 | adopted |

## ZK / VC / trust / security-properties estate

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1203 | 2026-06-22 | `sparq-vc` hand-rolls `eddsa-rdfc-2022` + `did:key`/`did:web` over `ed25519-dalek`; verifies over the RDF-dataset form (not raw JSON-LD); its standard-Ed25519 DID resolver coexists with the ZK Baby-JubJub one | lean-core on one vetted primitive; RDFC-1.0 canonical form is what the proof binds; the two resolvers serve interop vs custom-ZK | #1202 | adopted |
| #1195 | 2026-06-22 | the `secx:` IRI constants the sparq-zk annotation graph needs are declared locally and drift-pinned to `secprop-ext.ttl` via `include_str!` | `sparq-trust` already depends on `sparq-zk`, so the reverse edge would be a cycle; a compile-time file read is not a crate edge | #1194 | **superseded by #3705** |
| #3705 | 2026-08-01 | the `secx:`/`sec-prop:` IRI constants, the canonical `secprop-ext.ttl` and the single TTL drift test move to `sparq-secprop-vocab`, a dependency-free LEAF crate that `sparq-trust`, `sparq-policy` and `sparq-zk` each depend on behind their existing default-OFF secprop features | supersedes #1195: a leaf BELOW all three removes the cycle that forced the local copies, so there is one copy of every IRI instead of three; the cross-package `include_str!` it replaces also broke `cargo package` file inclusion for `sparq-zk`, and its `ci/path-ownership.toml` `readers` patches are retired because the ordinary reverse-dependency closure now attributes the vocabulary | #3705 | adopted |
| #1190 | 2026-06-22 | PROV-O delegation-audit plus human/AI-principal classification emitted directly via oxrdf behind default-OFF `delegation-prov`; the audit is a record, never an authority source | no `sparq-prov` dep (lean-core); the audit is produced only after a successful invoke | #1192 | adopted |
| #1165 | 2026-06-22 | DID resolution uses a sparq-private multicodec for Baby-JubJub (loudly non-interop), a pluggable `DidDocumentFetcher` trait (no shipped HTTP client), and first-verification-method doc parse | the key is non-Ed25519; lean-core plus offline-testable; PoC scope, narrows but does not anchor forgery | #1164 | adopted |
| #1209 | 2026-06-23 | the ODRL 2.2 security-property profile and its leftOperand surface ship in `sparq-policy` (default-OFF `secprop-leftoperands`), not `sparq-trust` | Phase 4 is verbatim "registration in sparq-policy"; avoids a policy to trust crate edge; `secx:` IRIs bound as fixed w3id strings | #1207 | adopted |
| #1092 | 2026-06-21 | VC import+query shipped as a SITE showcase; in-tab JSON-LD remote-`@context` is handled honestly (inline sample plus a visible caveat, never a "verified" claim) | reuses the in-tab WASM ingest path; the lean wasm parser has no remote-context loader | #1091 | adopted |
| #1166 | 2026-06-22 | sq-m3sm closed as a duplicate of sq-i1wh2 (MPC seam Phase 3) — no PR opened | the routing pass is fully delivered in `routing.rs`; the one nominal delta is caller-held metadata, so no sound non-empty PR exists | #985 | adopted |

## KB / FO-bridge / terse / NLQ / GenAI

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1520 | 2026-07-05 | research-KB decomposition: add `prov:generatedAtTime` to the contract; tier-separation v1 = per-tier Turtle plus tier IRIs (not in-store named graphs); dump repo created PRIVATE + fail-closed; pilot precision bar pre-registered; sq-t5f3l closed superseded | verified gaps; no persistent store exists to hold named graphs; unknown-license implies restricted-tier | #1519 | adopted |
| #1474 | 2026-07-04 | paper selection: zkSPARQL re-scoped from "write" to "submission-support hardening"; the assurance paper killed-as-framed and replaced with the first SPARQL logic-bug-testing paper | zkSPARQL is already a live ISWC submission; testing venues need confirmed third-party bugs the assurance framing cannot supply yet | #1473 · `research/paper-selection.md` | adopted |
| #1213 | 2026-06-23 | FO-bridge read-path URI-hiding mechanism shipped with only the model-free half measured; the answer-accuracy half registered UNMEASURED with a neutral null, not a proxy | no API key on the box; the project has reversed three char/byte proxy claims, so a fourth proxy would be the trap | #1212 · `bench/compose/RESULTS.md` | adopted |
| #1170 | 2026-06-22 | PKG write-path authoring = a standalone `*.yaml.ld` source plus a stdlib-only YAML-subset reader (no PyYAML) | matches the `sec-prop.yaml.ld` precedent and `ingest_pkg.py`'s stdlib posture; out-of-subset fails closed | #1169 | adopted |
| #1173 | 2026-06-22 | the terse Phase-5 A/B measured the deterministic input-authoring lever via a documented char-to-token proxy (fidelity-stamped, CONDITIONAL-only) because the harness cannot fan out sub-agents in one session | the proxy is explicit and can only return CONDITIONAL; the full-session fan-out (sq-bmpzd) is the arbiter | #1174 | adopted |
| #1152 | 2026-06-22 | the terse keyword lever ships a `K:` sigil form in the DEFAULT build (no `vectors` dep); the v1 legend is frozen at `pkg-keywords/v1`, scoped to the PKG hot terms | a bare-word scan is collision-prone; the layer has no model/engine dep; broad adoption is conditional on the Phase-5 A/B | #1151 | adopted |
| #1198 | 2026-06-22 | `EntityLinker` exact-label match is punctuation-PRESERVING (lower/space-normalise, keep punctuation) | stripping punctuation risks collapsing two distinct labels and weakening the unambiguity guarantee | #1197 | adopted |
| #1161 | 2026-06-22 | `Answer.citations` is a pay-as-you-go `citations(&graph)` accessor (opt-in feature), not a stored field | `Answer` is deliberately graph-lifetime-free; the default build stays byte-identical | #1160 | adopted |
| #1096 | 2026-06-21 | the gUFO closure-prior firm-up uses a denser SYNTHETIC schema-bearing slice plus an env-gated real-KG hook, not a vendored typed DBpedia/YAGO subset | the variance fix is dataset-independent; a real typed subset is large and needs external fetch | #1094 | adopted |

## GUI / site surfaces

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1508 | 2026-07-05 | GUI consolidation: N3 = query-time ground-closure via `reasonN3`; a browser File tab for plain RDF text; deferred stubs regrouped into a "Coming soon" rail (ZK/MPC stay honest stubs, audit pending); full-text/streaming tabs promoted in-scope | uses already-shipped surfaces; keeps the honest stubs honest | #1507 · `research/gui-consolidation-fix-plan.md` | adopted |
| #1206 | 2026-06-23 | ship GUI result pagination (the rendering half) now; gate the demand-driven page-wise EVALUATION half behind a pull-iterator exec-model decision | the wasm cursor slices already-materialised rows, so real lazy eval needs a Volcano iterator or an order-dependent rewrite | #1205 · `research/gui-design.md` §A.5.1 | adopted |
| #1148 | 2026-06-22 | the SHACL Compact-Syntax input toggle was added to the `/surface/shacl` playground (not `/try`); a framework-agnostic `sparqParseShaclCompact` helper, no new npm `SparqStore` API | that is where the SHACL UI lives; matches the existing site-helper shape | #1147 | adopted |

## Engine / core / RDF-JS / Solid API

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1338 | 2026-07-01 | the M4 byte-identity gate is OPERATOR-level (same input batch), not cross-query; the micro-bench uses the established `examples/bench_*.rs` metric idiom, not criterion | a sargable filter reorders the scan so a cross-query differential is unsound; the workspace has zero criterion (lean-core) | #1337 · `research/vector-at-a-time-m4.md` | adopted |
| #1158 | 2026-06-22 | `GROUP BY ?v` on a never-bound variable is treated-as-unbound (one group), not a query error | it is what SPARQL 1.1 §11.1 specifies and matches the existing empty-input handling | #1157 | adopted |
| #1146 | 2026-06-22 | `Dataset.contains` over blank nodes is a bounded backtracking homomorphism search that fails closed past a step budget (never a wrong true) | full blank-node subgraph homomorphism is NP-hard; a canon-based reduction wrongly reports relabelled disjoint copies | #1145 | adopted |
| #1150 | 2026-06-22 | `SharedGraph` ships as `Arc<RwLock<Graph>>` (not Mutex) with a minimal surface plus `From<Graph>`, no framework glue | serving is read-heavy and the docs steer to the lock-free snapshot; an axum extractor belongs in `sparq-server` | #1149 | adopted |
| #1154 | 2026-06-22 | `PodStore::wac_allow` is placed in the always-present `sparq-solid` public API, not behind a cargo feature | it adds zero deps and is a sibling of the already-public `accessible()`/`query_as()` | #1153 | adopted |
| #1210 | 2026-06-23 | `PodStore::put_acl`/`delete_acl` (plus `_acp`) ship always-compiled (no feature gate); they are a storage primitive that performs NO session authorization | zero new deps and reuse-only, matching the crate's other always-compiled write/decide paths | #1211 | adopted |

## CI / infra / agent process

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1482 | 2026-07-04 | the feature-matrix "pyramid" is right-sized (not collapsed): mechanize the guard as a ratchet plus tier machinery; the honest demotion is one clear plus nine audit-pending, not "~50 to a dozen" | most opt-in legs already carry confirmed feature-gated tests; the big feature-leg bill is irreducible at the leg-set level | #1481 · `research/feature-matrix-pyramid.md` | adopted |
| #1168 | 2026-06-22 | disk-guard's completed-worktree reclaim is opt-in in the library script but ON-by-default in the per-tick `disk-guard.sh` delegation | the per-tick sweep must reclaim or disk starves; an in-use probe plus the harness LOCK keep it off live agents | #1167 | adopted |
| #1104 | 2026-06-21 | `proceed-and-document` lives in the internal `.claude/skills/` tree (not the public `skills/`); AGENTS.md reconciled to current truth (scheduler landed), not the bead's stale framing | it is an agent-process skill, not a usage surface; fix stale claims to the verified reality | #1103 | adopted |
| #1199 | 2026-06-22 | the docs guide single-sources its capability list by `{{#include}}`-ing the README plus a build-time link-rewriting mdBook preprocessor (option a) | keeps the single source in README with lychee-friendly relative links; avoids weakening the internal-links gate or adding a manifest | #1196 | adopted |
| #2759 | 2026-07-29 | merge-queue throughput (sq-6vshe.16): `max_entries_to_build` 3→5 approved in principle but NOT requested; `min_entries_to_merge_wait_minutes: 5` audit closes **inert** (no edit); CodeQL stays eligible for the queue-blocking path | the 3→5 precondition (`sq-6vshe.14` push-skip) has not landed, and raising parallelism first worsens the measured 43–225 s per-job queue delays; the wait field is a ceiling while `min_entries_to_merge` is unmet, not a floor, so it never binds at 1; CodeQL measured 3.8 m median on `merge_group` — a non-pole, so latency never justified demoting a security gate | `docs/branch-protection.md` §Merge-queue throughput settings · `research/ci-mergequeue-speedup-2026-07.md` §3.3 | adopted |

## E2E gating governance

| Issue | Date | Decision | Rationale | Evidence | Status |
|---|---|---|---|---|---|
| #1656 | 2026-07-06 | RATIFY the early promotion of the `gui-mock-ipc` Playwright lane — keep it gating `ci-summary`, backfill its probation-ledger row as ratified | it is a fully deterministic headless-Chromium lane (`retries: 0`, mocked IPC); demoting a green gate on an active surface reduces real enforcement. Rollback = the §6 demotion runbook: add a `.github/advisory-registry.json` entry for `gui.yml` · `gui-mock-ipc` (`owner_bead` + `promotion_criteria` + `job_id`) **and** `continue-on-error: true` on the run step. Since #3773 the registry declaration — not the job name — decides gating, so a name edit alone demotes nothing (and `scripts/check-advisory-registry.py` C2 would RED on an advisory-named job with no entry) | `.github/E2E-GATING-POLICY.md` §6 · §8 | adopted |

## Still-open steering items (NOT consolidated)

These related steering issues remain open on purpose — they are not settled retrospective
records but genuinely await a maintainer choice (or a still-open PR), so they keep their own
thread:

- **#1587** — the proposed next-phase work plan is a forward proposal with explicit un-made
  maintainer calls (commission the `sq-qhy4` external audit, arm the first live KB run, the
  EC2 bench re-greenlight, ratify/redirect several open flips). Its design-record PR #1586
  was closed unmerged; the plan itself still needs a maintainer pick.
- **#1110** — the provenance-driven GenAI-KB direction (`revisit-with-fable`) is an ongoing
  iteration umbrella with unresolved asks: the load-bearing-vocabulary quartet choice and
  literature-DB API access/keys (an external-credential block, not an autonomous call).
- **#1156** — the sq-9c5e VC cryptosuite bridge decision is not yet reflected in merged code:
  PR #1155 is still open and the `vc-bridge` feature is absent from `main`, so this stays with
  its open PR rather than the ledger.
