<!-- [OPUS-4.8] CDMC capability-maturity scorecard for sparq (epic sq-toze, branch cert-cdmc). -->
<!-- NON-CANONICAL timing; no measured numbers baked here. Scores the codebase state at the head
     of cert-cdmc; re-score at consolidation if the codebase changes materially. -->
# CDMC capability-maturity scorecard — sparq

> 🤖 SPARQ agent. This is the **headline** CDMC deliverable: a per-capability maturity rating
> for sparq against the **EDM Council Cloud Data Management Capabilities (CDMC)** framework
> (6 components, 14 capabilities, 37 sub-capabilities, 14 key controls).

## How to read this

**Scope reframing (load-bearing).** CDMC was written for an organisation operating a *cloud data
platform* that ingests, catalogues, classifies, governs and retires **its own** business data.
sparq is **not** that. sparq is a **data engine** — a Rust library (`sparq-core`, `sparq-engine`,
…) + an HTTP query server (`sparq-server`/`sparq-serve`) + a WASM client — that a **deploying
operator** embeds to load, index and query **the operator's own RDF**. Under CDMC's own
accountability model, that makes the operator the **data owner / accountable party** for almost
every governance, cataloguing and classification *decision*; sparq supplies the **technical means**
to record and enforce those decisions. Each rating below therefore marks, explicitly, what is
**sparq's capability** vs **the operator's responsibility**. Inflating a score by crediting sparq
for an operator decision (e.g. "data is classified") is exactly the overclaiming the honesty
contract forbids — so where the substance is the operator's, sparq is scored on whether it gives
the operator the *hooks* to do it, not on the decision itself.

### Maturity scale (CDMC 1–5)

| Level | Name | Meaning for sparq |
|---|---|---|
| **1** | Not assessed / absent | No capability in the codebase; entirely the operator's to build. |
| **2** | Limited / ad-hoc | A partial or low-level primitive exists; no first-class, tested surface. |
| **3** | Defined / repeatable | A real, documented, tested capability exists and is exercisable today. |
| **4** | Managed / measured | The capability is enforced, gated and has regression/conformance coverage. |
| **5** | Optimised | Best-in-class, automated, continuously verified; little left to add. |

Ratings are **conservative by construction** (honesty contract). "Operator-owned" capabilities are
capped at the maturity of sparq's *enabling hooks*, not the (absent) governed outcome.

## Headline scorecard

| # | CDMC Component | Capability | Maturity | One-line rationale |
|---|---|---|---|---|
| 1 | Governance & Accountability | 1.1 Data control compliance is established | **3** | SECURITY.md, threat model, certification estate, CODEOWNERS — strong for the *engine*; org-level data-ownership is the operator's. |
| 1 | Governance & Accountability | 1.2 Ownership is established for migrated & cloud data | **2** | sparq has no concept of business-data owner; named-graph provenance is the only hook. Operator-owned. |
| 1 | Governance & Accountability | 1.3 Sourcing & consumption are governed & supported | **3** | Per-graph load, VoID/Service-Description introspection, federation (`SERVICE`) with a documented outbound boundary (B4). |
| 2 | Cataloguing & Classification | 2.1 Data catalogues are implemented, used & interoperable | **3** | VoID + SPARQL Service-Description endpoints (#219), `sparq-introspect` effective-schema mining — a genuine, tested machine-readable catalogue surface. Still **opt-in / default-OFF** (behind the `federation-descriptors` cargo feature; `GET /.well-known/void` returns `404` in a stock build), and the `feature-matrix.yml` lane (#244, bead **sq-kzfi**) **builds + tests + clippies** `federation-descriptors` — but nightly only since the lean-CI change (no PR trigger, `ci-summary` aggregator deleted), so a regression is detected post-merge rather than breaking the merge. Re-scored 4 → 3: the level-4 "gated" condition no longer holds. |
| 2 | Cataloguing & Classification | 2.2 Data classifications are defined & used | **3** | SHACL (100% W3C core suite) + RDFS/OWL/N3 reasoning let an operator *declare & validate* classification; sparq does not impose a sensitivity taxonomy (operator-owned). |
| 3 | Accessibility & Usage | 3.1 Data entitlements are managed, enforced & tracked | **3** | `sparq-solid` WAC/ACP fail-closed per-session enforcement + an optional constant-time bearer-token write gate; ODRL usage-control is roadmap only. |
| 3 | Accessibility & Usage | 3.2 Data access is tracked, with audit trails | **2** | Prometheus request metrics + WAL of mutations exist; there is no per-subject/per-query access **audit log** with identity. Operator must front-log. |
| 4 | Protection & Privacy | 4.1 Data is secured & controls are evidenced | **4** | Deep, CI-gated security estate: `#![forbid(unsafe_code)]` (31/36 crates), Miri/fuzz, CodeQL, supply-chain attestation, DoS limits, distroless image. |
| 4 | Protection & Privacy | 4.2 A data privacy framework is defined & operational | **2** | sparq processes no personal data of its own (operator is controller); the ZK/MPC privacy story is **remediated but NOT externally audited** (no production guarantee — external sign-off `sq-qhy4` PENDING) and excluded from any crypto-protection score. |
| 4 | Protection & Privacy | 4.3 Sensitive data is protected (incl. encryption) | **2** | At-rest/in-transit encryption is **operator-deployment** (no built-in TLS; mmap files are plaintext). SHACL/classification gives the hook; sparq adds no crypto protection. |
| 5 | Data Lifecycle | 5.1 The data lifecycle is planned & managed | **3** | Full SPARQL 1.1 UPDATE (INSERT/DELETE/CLEAR/DROP/CREATE/LOAD) over the whole dataset, conformance-tested; lifecycle *policy* is the operator's. |
| 5 | Data Lifecycle | 5.2 Data quality is managed | **3** | SHACL Core/SPARQL validation (differential-fuzzed) + reasoning give the operator a real quality-rule engine; sparq runs no standing quality program. |
| 6 | Technical Architecture | 6.1 Technical design supports the data platform | **4** | Mature engine: sorted-permutation indexes, generation-ring MVCC snapshots, WAL durability, time-travel, zero-copy dataset views, HDT/compressed ingest. |
| 6 | Technical Architecture | 6.2 Data lineage is documented & auditable | **2** | Named-graph quad model + WAL/journal + reasoning `explain` give *partial* provenance; no first-class W3C-PROV lineage capture across ingest→index→result. |

**Key-control posture (the 14 CDMC key controls):** of the 14, the ones sparq can technically
*own* — **classification (KC trigger via SHACL)**, **entitlements (WAC/ACP + write-token)**,
**security controls (the certification estate)**, **lifecycle/retention mechanism (UPDATE + ring
retention)** — are at **3–4**. The ones that are **inherently the operator's accountable decision**
— *ownership assigned*, *authoritative-source registration*, *data-sovereignty/residency*,
*ethical-access/usage approval*, *cost-metric capture*, *encryption-at-rest enforcement*,
*data-retention-policy enforcement* — sit at **2** for sparq because sparq provides hooks but not
the governed outcome. **No key control is rated above its real evidence.**

## Component-level summary

| Component | Avg maturity | Verdict |
|---|---|---|
| 1 — Governance & Accountability | ~2.7 | Engine governance strong; data-ownership/sourcing is operator-accountable. |
| 2 — Cataloguing & Classification | ~3.0 | VoID/SD catalogue + SHACL classification hooks are real; the catalogue (2.1) is regression-tested **nightly** (`feature-matrix.yml`, #244 / bead `sq-kzfi`) but no longer merge-gated → level 3 (2.1 = 3, 2.2 = 3). |
| 3 — Accessibility & Usage | ~2.5 | Access *control* is real (WAC/ACP, token); access *audit* and ODRL usage-control are gaps. |
| 4 — Protection & Privacy | ~2.7 | Security is **excellent (4)**; privacy/crypto-protection is **deliberately low (2)** — ZK/MPC remediated but NOT externally audited (no production guarantee), encryption is operator-deployment. |
| 5 — Data Lifecycle | ~3.0 | Solid lifecycle *mechanism* (UPDATE/WAL/retention); lifecycle *policy* is operator-owned. |
| 6 — Technical Architecture | ~3.0 | **Engine architecture is a strength (4)**; lineage capture is the weak axis (2). |

**Overall honest posture:** sparq is **strongest where it is an engine** (technical architecture and
security controls — 4s; the cataloguing surface (2.1) is a **3** — the
`federation-descriptors` feature is built/tested/clippied nightly, not on every PR — #244) and **deliberately weak where the capability is a
governance *decision* an operator must make** (ownership, classification taxonomy, retention
policy, residency — 2s). The single most important honesty statement: **the ZK/MPC estate is
documented as remediated but NOT externally audited — no production cryptographic guarantee**
(originally found unsound, `sq-1s2` binding layer landed, internal re-audit "sound as landed for
the assumed threat model," external sign-off `sq-qhy4` PENDING — `research/zk-soundness-audit.md`,
`research/zk-verifier-reaudit.md`, `SECURITY.md`) and it contributes **zero** to any
protection-by-cryptography maturity here. Any future scorer who credits ZK/MPC as a production
privacy control before the external sign-off is overclaiming. <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->

## Recommendations

See `recommendations.md` (same directory) for the prioritised, evidence-linked recommendation set
that maps each sub-4 capability to a concrete remediation and its tracking bead under epic
`sq-toze`. Top three by leverage:

1. **Lineage (6.2 → 3):** ship a W3C-PROV (or VoID-linked) capture of *which named graph / load
   event* produced a binding, surfaced via the existing `explain` + descriptor endpoints. Lowest
   effort for the highest CDMC-credibility gain (lineage is a CDMC key-control theme).
2. **Access audit trail (3.2 → 3):** add a structured, opt-in per-query access log (identity from
   the WAC session / token, query digest, graphs touched, decision) — sparq already computes the
   authorised graph set; emit it.
3. **Classification & residency hooks (2.2/4.3):** document the operator-owned classification +
   encryption-at-rest pattern (SHACL sensitivity shapes → operator KMS/TLS termination) as a
   first-class deployment guide so the "operator-owned" 2s are *explicitly* handed off, not silently
   absent — and so an auditor can see the boundary.
