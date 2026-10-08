<!-- [OPUS-4.8] CDMC control → status → evidence table — epic sq-toze, branch cert-cdmc. -->
# CDMC controls — status & evidence

> 🤖 SPARQ agent. The spine the auditor checks. One row per CDMC capability/key-control theme,
> mapped to **how sparq's implementation addresses it**, with **repo-relative evidence**
> (`crates/<crate>/src/…` = enforcing file, `crates/<crate>/tests/…` / `#[test]` = regression
> test, `.github/workflows/<wf>.yml` = CI gate). Status legend:
> **Implemented & verified** (technical control + passing evidence) · **Partial** (a primitive
> exists, capability incomplete) · **Operator-owned** (sparq supplies the hook; the governed
> decision/outcome is the deploying operator's) · **Gap** (absent; see `gap-register.md`).
> Maturity column matches `scorecard.md`.

## Component 1 — Governance & Accountability

| Cap | Control theme | Status | Maturity | How sparq addresses it | Evidence |
|---|---|---|---|---|---|
| 1.1 | Data-control compliance established | Implemented & verified (engine); Operator-owned (org data) | 3 | Security policy + coordinated disclosure, STRIDE threat model, full certification estate, code ownership. The *engine's* compliance controls are real and CI-gated; org-level data-control compliance is the operator's. | `SECURITY.md`; `research/threat-model.md`; `compliance/` (this estate); `CODEOWNERS`; `CONTRIBUTING.md` |
| 1.2 | Ownership established for data | Operator-owned | 2 | sparq carries no business-data-owner concept. The only hook is the named-graph quad model (provenance-by-graph) + VoID/DCAT-style dataset headers the operator may load. | `crates/sparq-engine/src/dataset.rs` (named-graph model); `crates/sparq-introspect/src/lib.rs` (`to_void`) |
| 1.3 | Sourcing & consumption governed | Implemented & verified (partial) | 3 | Per-graph loading; effective-schema + VoID introspection of what was actually ingested; federated `SERVICE` consumption with a **documented outbound boundary (B4)** and no silent egress. | `crates/sparq-engine/src/service.rs`; `crates/sparq-introspect/src/lib.rs`; `research/threat-model.md` (B4) |

## Component 2 — Cataloguing & Classification

| Cap | Control theme | Status | Maturity | How sparq addresses it | Evidence |
|---|---|---|---|---|---|
| 2.1 | Data catalogues implemented & interoperable | Implemented & verified (opt-in, default-OFF, nightly-tested — no longer merge-gated) | 3 | **Machine-readable catalogue surface**: W3C VoID (`GET /.well-known/void`) + SPARQL 1.1 Service-Description (`sd:Service`) endpoints (#219), backed by `sparq-introspect` effective-schema mining (classes, predicates, characteristic sets, vocabularies, declared vs observed domain/range) — interoperable, standards-based, no extra state. **Posture:** still compiled only behind the `federation-descriptors` cargo feature (a stock build returns `404` for `/.well-known/void`), and the `feature-matrix.yml` lane (#244, bead **sq-kzfi**) builds + tests + clippies it — but since the lean-CI change that lane runs nightly + dispatch only and the `ci-summary` aggregator is deleted, so a regression is caught post-merge, not blocked. Re-scored **4 → 3**: the level-4 "gated" condition no longer holds pre-merge. | `.github/workflows/feature-matrix.yml` (`opt-in sparq-server (federation-descriptors, …)` leg — build+test+clippy, #244); `crates/sparq-server/src/descriptors.rs` (feature `federation-descriptors`, 6 `#[test]`s); `crates/sparq-introspect/src/lib.rs` (`to_void`, `to_json`, `to_text_summary`); `crates/sparq-introspect/README.md` |
| 2.2 | Data classifications defined & used | Implemented & verified (hook); Operator-owned (taxonomy) | 3 | SHACL Core + SHACL-SPARQL validation (**100% W3C core suite, differential-fuzzed**) and RDFS/OWL/N3 reasoning let an operator *declare, infer and validate* classifications. sparq imposes **no** sensitivity taxonomy — that decision is the operator's. | `crates/sparq-shacl/` (README: 98/98 W3C core); `crates/sparq-shacl/tests/`; `crates/sparq-reason/src/{rdfs.rs,owl.rs,n3}` |

## Component 3 — Accessibility & Usage

| Cap | Control theme | Status | Maturity | How sparq addresses it | Evidence |
|---|---|---|---|---|---|
| 3.1 | Entitlements managed, enforced & tracked | Implemented & verified (access control); Gap (usage control) | 3 | `sparq-solid` enforces **WAC + ACP fail-closed** per-`(WebID, client)` session (inheritance, agent classes, groups, deny-overrides), filtering every query to the authorised graph set. Server adds an optional **constant-time bearer-token write gate** (`AuthPosture`, sq-zcby). **ODRL usage control is roadmap only.** | `crates/sparq-solid/` (README, `authindex.rs`, `rewrite.rs`); `crates/sparq-server/src/http.rs` (`AuthPosture`, constant-time token cmp); `research/feature-research-odrl-policy.md` (roadmap) |
| 3.2 | Access tracked with audit trails | Implemented (opt-in) | 3 | Aggregate Prometheus request metrics + a WAL of every mutation, PLUS two opt-in per-request access trails: the flat `audit-log` `tracing` trail (`sq-0bxp`), and the **richer STRUCTURED access-audit sink** (`access-audit` feature, `sq-gos8`) that records every ENFORCED access decision as a typed JSON-Lines record through a pluggable sink — **actor** (WebID / agent IRI when known, else a Bearer-token fingerprint, never the raw token), **action** (query/update/graph read/graph write), **resource** (the named-graph IRI / dataset touched), **decision + policy-basis** (the actually-enforced auth outcome), timestamp + a non-reversible request fingerprint. Closes the "identity + graphs-touched" gap at the server enforcement seam. **Privacy boundary (honest):** identities + resource IRIs ARE recorded (that is the audit trail's purpose); query CONTENT is recorded only as a fingerprint, never the raw text (the #241 / sq-toze.34 redaction posture). Doubly opt-in (cargo feature + `--access-audit <file\|stderr>`); zero-cost when off. **Gap remaining:** still per-server-request, not an engine-internal per-graph read trace; ODRL usage control is roadmap. | `crates/sparq-server/src/access_audit.rs` + `tests/access_audit.rs` (`sq-gos8`); `crates/sparq-server/src/audit.rs` (`sq-0bxp`); `crates/sparq-server/src/metrics.rs` (aggregate); `crates/sparq-engine/src/update.rs` (WAL) — see gap CD-2 |

## Component 4 — Protection & Privacy

| Cap | Control theme | Status | Maturity | How sparq addresses it | Evidence |
|---|---|---|---|---|---|
| 4.1 | Data secured & controls evidenced | Implemented & verified | 4 | Deep, CI-gated security estate: `#![forbid(unsafe_code)]` in 31/36 crates with a justified unsafe register (GX-5/#217), Miri + cargo-fuzz lanes, CodeQL SAST, OpenSSF Scorecard, cargo-deny, CycloneDX SBOM, SLSA build provenance, distroless non-root SHA-pinned image, four-limit DoS guards (timeout/body/concurrency/results). | `crates/*/src/lib.rs` (`forbid(unsafe_code)`); `.github/workflows/{miri,fuzz,codeql,scorecard,supply-chain}.yml`; `crates/sparq-server/src/main.rs` (DoS limits); `Dockerfile`; the `asvs`/`cis`/`sbom`/`slsa`/`memsafety` slices |
| 4.2 | Privacy framework defined & operational | Operator-owned; Gap (crypto privacy) | 2 | sparq processes **no personal data of its own** (operator is the GDPR controller — see `compliance/data-flow.md`/`dpia.md`, privacy slice). The ZK/MPC "prove-without-disclosing" estate is **documented as remediated but NOT externally audited — no production guarantee** (external sign-off `sq-qhy4` PENDING) and contributes **zero** here. <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> | `research/zk-soundness-audit.md` (original audit) + `research/zk-verifier-reaudit.md` (`sq-gbp4`, "sound as landed for the assumed threat model"); `SECURITY.md` §"`sparq-zk` and `sparq-zk-compose` — ZK verifier: remediated, but NOT externally audited"; `compliance/data-flow.md` (privacy slice) |
| 4.3 | Sensitive data protected (encryption) | Operator-owned | 2 | At-rest (mmap `.spq`/dict files are **plaintext**) and in-transit (server is **plaintext HTTP**, no built-in TLS) encryption are **operator-deployment** concerns (TLS-terminating gateway + disk/KMS encryption). SHACL gives the classification hook; sparq adds no crypto protection. | `crates/sparq-core/src/store.rs` (plaintext mmap save); `crates/sparq-server/src/http.rs` (no TLS); threat-model B3/B5 — see CD-3 |

## Component 5 — Data Lifecycle

| Cap | Control theme | Status | Maturity | How sparq addresses it | Evidence |
|---|---|---|---|---|---|
| 5.1 | Lifecycle planned & managed | Implemented & verified (mechanism); Operator-owned (policy) | 3 | Full **SPARQL 1.1 UPDATE over the whole dataset** — INSERT/DELETE DATA, DELETE/INSERT…WHERE, CLEAR, DROP, CREATE, LOAD (ADD/COPY/MOVE desugared) — with **WAL + single-frame transactional commit point** for crash-atomic durability; conformance-tested. Retention/age-out via the generation ring. Lifecycle *policy* is the operator's. | `crates/sparq-engine/src/update.rs` (every `GraphUpdateOperation` + WAL/`txn.log` commit point); `crates/sparq-engine/src/update.rs` (`#[test]` crash-replay tests); `crates/sparq-serve/src/lib.rs` (`TimeTravelConfig` retention) |
| 5.2 | Data quality managed | Implemented & verified (hook); Operator-owned (program) | 3 | SHACL Core/SHACL-SPARQL validation (differential-fuzzed against reference engines) + RDFS/OWL/N3 reasoning give a real, standards-conformant quality-rule engine. sparq runs no standing quality *program* — the operator authors the shapes/rules. | `crates/sparq-shacl/` (validation + diff fuzz); `crates/sparq-reason/` |

## Component 6 — Technical Architecture

| Cap | Control theme | Status | Maturity | How sparq addresses it | Evidence |
|---|---|---|---|---|---|
| 6.1 | Technical design supports the platform | Implemented & verified | 4 | Mature engine: sorted-permutation indexes, **generation-ring MVCC** (arc-swapped immutable snapshots, bounded retention, lock-free readers), **WAL durability**, opt-in **time travel**, zero-copy dataset views, mmap for larger-than-RAM, HDT + fused compressed ingest. Conformance + fuzz + Miri gated. | `crates/sparq-core/src/store.rs`; `crates/sparq-serve/src/lib.rs` (generation ring); `crates/sparq-hdt/`; `crates/sparq-conformance/`; `.github/workflows/{ci,miri,fuzz}.yml` |
| 6.2 | Data lineage documented & auditable | Partial / Gap | 2 | Named-graph quad model + per-mutation WAL/journal + `sparq-reason` `explain` (derivation traces for inferred triples) give **partial** provenance. No first-class **W3C-PROV lineage** capturing ingest-event → graph → result across the pipeline. | `crates/sparq-engine/src/dataset.rs`; `crates/sparq-engine/src/update.rs` (WAL); `crates/sparq-reason/src/{explain.rs,incremental_explain.rs}` — see CD-1 |

## Honesty notes (auditor anchors)

- **No capability is scored above its evidence.** Operator-owned axes (1.2, 4.2, 4.3, the policy
  halves of 5.1/5.2) are capped at the maturity of sparq's *enabling hook*, never the governed
  outcome.
- **ZK/MPC is excluded** from 4.x protection-by-crypto. <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> The estate is **remediated but NOT
  externally audited** (originally unsound per `research/zk-soundness-audit.md`; `sq-1s2` landed;
  internal re-audit `research/zk-verifier-reaudit.md` "sound as landed for the assumed threat
  model"; external sign-off `sq-qhy4` PENDING; **no production guarantee**; MPC carries no
  guarantee). This is binding; any row crediting it as a production crypto-privacy control before
  the external sign-off is a finding.
- The optional bearer-token write gate (3.1) is a **real, constant-time-compared** control
  (sq-zcby) — it does **not** contradict threat-model boundary **B3** ("no per-user auth; front with
  a gateway"); it is a coarse write/read gate, not user authentication, and is documented as such.
