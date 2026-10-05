# Feature research — ODRL usage-control + policy-over-queries for sparq

> Model: Opus 4.8 (Fable 5 unavailable — flag for re-review/upgrade when Fable returns).
> Deep-research record for epic **sq-3183**. NON-CANONICAL timing; no measured numbers here.
> [OPUS-4.8]

## TL;DR

sparq has **access control** today (`sparq-solid`: WAC/ACP → a materialized auth view →
per-session graph-set restriction by query rewriting / a zero-copy `DatasetView`) and a
**privacy/disclosure** estate (`sparq-zk`, `sparq-zk-compose`, `sparq-mpc`: prove-without-
disclosing, attested-source set-membership, minimal-disclosure federated evaluation). It has
**no usage control** and **no ODRL**. ODRL is the missing *declarative policy layer above both*:
WAC/ACP answers "may this agent read graph G?"; ODRL answers "may this party use this asset
*for purpose P, with obligation O, until time T, disclosing only to recipient R*?" — and, in the
sparq-specific novel angle, **"which other federation nodes may this node disclose WHAT to, and
under what proof obligation."**

The headline opportunity: an **ODRL-driven disclosure-control layer for federated SPARQL**, where
ODRL `Constraint`s (purpose / recipient / time / `informedConsent` duty) and `Prohibition`s
*decide* the per-operator disclosed-vs-hidden split that sparq's MPC layer already implements
(RQ2a, `research/mpc-zkp-research-and-architecture.md` §4.3 step 3), and ODRL `Duty`s become
**proof obligations** discharged by the existing ZK manifest. This composes ODRL (the standard the
EU data-spaces stack — IDSA, Gaia-X, Solid — has converged on) with sparq's differentiator
(verifiable, minimal-disclosure federated SPARQL). No published system does this for SPARQL.

Honest framing: the **policy model + a single-node usage-control gate over `sparq-solid`** is
buildable today and a clear fit. The **disclosure-control-in-federation** design is **research-grade**
and inherits every blocker of the MPC layer (honest-majority, LAN, ≤10³–10⁴ triples) plus the
unsolved RQ1 verifier-soundness remediation.

---

## 1. Landscape

### 1.1 ODRL — the policy model

ODRL 2.2 (W3C Recommendation) is the de-facto standard for expressing access *and usage* policies
over digital assets. Core model:

- **Policy** (`Set` / `Offer` / `Agreement`) — a container of rules.
- **Permission / Prohibition** — deontic rules that allow / forbid an **Action** on an **Asset**,
  optionally bound to a **Party** (assigner / assignee).
- **Duty** (a.k.a. Obligation / Condition / Consequence / Remedy) — an action that must be performed
  for a permission to be exercised, or as a consequence of a prohibition violation. This is the
  "usage control" kernel that pure access control lacks.
- **Action** — the regulated operation (`use`, `read`, `distribute`, `aggregate`, `anonymize`, …),
  optionally narrowed by **Refinement**.
- **Constraint** — a `(leftOperand, operator, rightOperand)` triple (`purpose`, `recipient`,
  `dateTime`, `spatial`, `count`, …) evaluated against the **state of the world**; refinements
  evaluate against the **evaluation request**.
- **Party / Asset** — the agent performing the action and its target; both refine-able.

Sources: [ODRL Information Model 2.2 / Formal Semantics — w3.org](https://w3c.github.io/odrl/formal-semantics/),
[ODRL Landscape — w3c.github.io](https://w3c.github.io/odrl/landscape/).

### 1.2 ODRL evaluation semantics + engines (this is the load-bearing part — ODRL is *evaluable*)

ODRL historically lacked a normative evaluation semantics; that gap is now closing, which is what
makes "an ODRL gate in front of a query" feasible rather than hand-wavy:

- **W3C ODRL Formal Semantics (CG)** — defines evaluation as: inputs = `{Policy, State-of-the-World,
  Evaluation Request, open/closed behavior}`; output = a **Compliance/Evaluation Report** of per-rule
  activation states (Permission: active + permit/deny; Prohibition: activated/violated; Obligation:
  fulfilled/violated/not-set; Constraint: satisfied/unsatisfied). Its **open/closed behavior** input is
  what makes an unlicensed request denied (**closed system**); prohibitions "carve out" sub-sets of
  broader permissions.
  [ODRL Formal Semantics — w3.org](https://w3c.github.io/odrl/formal-semantics/).
  **Correction ([OPUS-5] sq-ilk2q):** an earlier revision of this bullet called the closed-system
  behavior a *"conflict default"*, and that phrasing propagated into the `usage-control-policy` skill.
  It is wrong on both halves: open/closed behavior is not conflict resolution, and the CG report's
  conflict-resolution machinery is explicitly marked **pending**, so **no conflict default can be
  attributed to it at all**. The normative default for an unset `odrl:conflict` is `invalid`, and it
  comes from [ODRL IM 2.2 §conflict](https://www.w3.org/TR/odrl-model/#conflict) — not from the CG
  report. sparq's evaluator hard-wires the `prohibit` (deny-overrides) strategy instead; that
  divergence is deliberate and disclosed (crate README's ODRL conformance note, issue
  [#1375](https://github.com/sparq-org/sparq/issues/1375), odrl-policy-bridge paper Limitation #1).
- **Evaluation and Comparison Semantics for ODRL** (arXiv 2025) — a clean **query-answering
  semantics**: the state of the world is a relation of *events*, an ODRL rule is a boolean query over
  it, policy comparison/refinement is **query containment**. Reference impl `ODRL2SHACL` (deployed in
  the UPCAST data-marketplace project). Crucially: "the query-based semantics enables straightforward
  implementation in **SQL or SPARQL**" — i.e. ODRL evaluation *is itself a SPARQL/SHACL workload*, a
  natural fit for an RDF engine. [arXiv 2509.05139](https://arxiv.org/html/2509.05139v1).
- **SolidLab ODRL-Evaluator** (`odrl-evaluator`, npm/TS) — open implementation: inputs = three RDF
  quad-lists `{Policy, Request, State-of-the-World}`; SHACL pre-validation + cardinality + policy
  decomposition; evaluation via **Notation3 rules on the EYE reasoner** (eye-js WASM); output = a
  Compliance Report as quads. Part of the **FORCE** suite (KNoWS, maintainers Slabbinck & Esteves).
  [SolidLabResearch/ODRL-Evaluator — GitHub](https://github.com/SolidLabResearch/ODRL-Evaluator),
  [Interoperable Interpretation and Evaluation of ODRL Policies — Springer 2025](https://link.springer.com/chapter/10.1007/978-3-031-94578-6_11).
  **Direct relevance to sparq:** sparq already ships an N3/EYE-class reasoner used by `sparq-solid`'s
  `materialize_*` (the WAC/ACP rules are N3 strata). An ODRL evaluator built on the *same* reasoning
  primitive is an incremental, in-family addition — not a new dependency stack.

### 1.3 ODRL ∩ Solid (the existing bridge sparq-solid would compose with)

- **OAC — ODRL Profile for Access Control** (Esteves et al.) — extends WAC by layering ODRL policies
  beneath it; maps ODRL processing operations to access modes (`Use`/`Collect` → Read,
  `Store`/`MakeAvailable` → Write); uses the **Data Privacy Vocabulary (DPV)** for GDPR-aligned
  constraints (`purpose`, `legalBasis`, `recipient`, technical/organisational measures). Distinguishes
  **Requirement** (hard) vs **Preference** (soft) policies; matching a request policy against stored
  user preferences derives ACL/ACP authorizations + an audit-trail `Agreement`.
  [OAC profile](https://besteves4.github.io/odrl-access-control-profile/oac.html).
- **ODRL→ACP consent profile** (Pandit et al.) — translates ODRL consent policies into Solid **ACP**
  grants/matchers; matches request-policy vs preference-policy across {data category, processing op,
  purpose, recipient}; hierarchical resolution up the container tree. Operationalizes *consent* as a
  stored, queryable, revocable ODRL policy in the Pod.
  [ODRL profile for consent in Solid — harshp.com](https://harshp.com/research/publications/048-odrl-profile-consent-solid-acp).
- **Usage-control enforcement in Solid** — Slabbinck, Esteves et al., *enforcing usage-control
  policies in Solid using a rule-based software agent* (2nd Solid Symposium 2024); an Authorization
  Server computes permissions from ODRL policies. This is exactly the "Authorization-Server-style
  ODRL PDP" sparq could host. [ODRL Landscape](https://w3c.github.io/odrl/landscape/).
- **From Access Control to Usage Control with UMA** (arXiv 2026) — bridges access→usage control via
  User-Managed Access; relevant to obligation/duty enforcement after grant.
  [arXiv 2601.18761](https://arxiv.org/pdf/2601.18761).

### 1.4 Policy-aware querying — usage control *over queries* (the academic line sparq slots into)

- **Query-Based Access Control for Linked Data** (Kirrane et al.) — the canonical **query-rewriting**
  enforcement: policies as triple/quad-pattern constraints, translated into added graph patterns so
  unauthorized triples are unretrievable; covers SPARQL 1.1 subqueries, negation, and *update*
  rewriting; formal soundness/completeness ("correctness criteria for non-monotonic queries / for
  updates"). [arXiv 2007.00461](https://arxiv.org/pdf/2007.00461),
  [Rewriting of SPARQL/Update Queries for Securing Data Access — Springer](https://link.springer.com/chapter/10.1007/978-3-642-17650-0_2).
  **sparq already does the access-control half of this** — `sparq-solid::rewrite_for` /
  `wrap_for_view` is exactly a soundness-preserving rewrite restricting the visible graph set. The
  ODRL extension is to drive *triple/quad-level* and *result-transforming* (anonymize, redact,
  aggregate-only) rewrites from declarative policy, not just graph-set restriction.
- **SAFE — Policy-Aware SPARQL Federation over RDF Data Cubes** (Khan et al.) — federation engine that
  enforces policy-based access on sensitive statistical data across sources; the closest prior art to
  "policy-aware *federation*", though it is access-policy + cube-shaped, not ODRL usage control or
  privacy-preserving compute. [SAFE — PMC](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5288952/).
- **Access-Control Obligations for SPARQL-DL** (Sacco/Passant-line) — modeling/enforcing *obligations*
  (not just permits) on SPARQL queries — the duty half.
  [Modeling and Enforcing Access Control Obligations for SPARQL-DL — ACM](https://dl.acm.org/doi/10.1145/2993318.2993337).
- **Privacy-preserving decentralized querying** — translating access policies into **SHACL shapes**
  that *query results* are validated against (a "shape gate" on outputs) and privacy-preserving
  aggregation over decentralized stores. [Towards Querying in Decentralized Environments with
  Privacy-Preserving Aggregation — arXiv 2008.06265](https://arxiv.org/pdf/2008.06265),
  [A SPARQL-based framework to preserve privacy of sensitive RDF — Springer](https://link.springer.com/article/10.1007/s11761-023-00368-6).

### 1.5 Data-spaces usage control (where ODRL is the standard, and where MPC is entering)

- **IDSA + Gaia-X** both standardize on **ODRL** for asset usage policies. IDSA's Usage Control
  Policy carries permissions/prohibitions/obligations ("IDS Rules"); enforcement translates ODRL →
  MYDATA (Event-Condition-Action). Gaia-X/IDS connectors (Eclipse EDC) handle contracting + access +
  usage control + logging.
  [IDS Usage Control — Dataspace Connector](https://international-data-spaces-association.github.io/DataspaceConnector/Documentation/v5/UsageControl),
  [Interoperable and Continuous Usage Control Enforcement in Dataspaces — CEUR Vol-3705](https://ceur-ws.org/Vol-3705/paper10.pdf),
  [Policy Patterns for Usage Control in Data Spaces — CEUR Vol-3510](https://ceur-ws.org/Vol-3510/paper_sem4tra_1.pdf).
- **Secure Computation & Trustless Data Intermediaries in Data Spaces** (arXiv 2410.16442) — the
  bridge: data spaces are starting to combine **MPC/FHE** with policy enforcement so that
  *intermediaries facilitate transactions without accessing user data* (aligned to the EU Data
  Governance Act); "policies guide what computations occur and what gets revealed." This is the
  external validation of sparq's exact thesis — but it does **not** give a SPARQL-level,
  ODRL-drives-the-disclosure-split, ZK-attested construction. **That gap is sparq's opening.**
  [arXiv 2410.16442](https://arxiv.org/abs/2410.16442).
- **Policy-Driven AI in Dataspaces** (taxonomy/explainability, arXiv 2507.20014) and **ontology-guided
  ODRL generation** (arXiv 2506.03301) round out the active 2025–26 ODRL-in-dataspaces momentum.

---

## 2. Headline design sketch — **ODRL-driven disclosure control in federated SPARQL**

The user's novel angle: a node declares, **via ODRL**, which other federation nodes it will disclose
**what** information to, as part of federated computation — and sparq makes that declaration *binding*
and *verifiable* by wiring it into the ZK/MPC disclosure split it already has.

### 2.1 The composition (where each existing primitive plugs in)

```text
        ODRL policy (per-asset / per-graph)            ← declarative, standard, auditable
        Permission(action=query, purpose=P,
                   recipient∈{nodeB,nodeC})
        Prohibition(action=disclose rawValue to nodeD)
        Duty(action=proveAttestation | anonymize | logUse)
                          │
                          ▼  evaluate (N3/EYE, same reasoner as materialize_*)
        ┌─────────────────────────────────────────────────────────────┐
        │ sparq-policy (NEW): ODRL PDP → a per-(party,purpose,asset)    │
        │ DisclosureDecision: {graph-set ✓ (→ sparq-solid view),        │
        │   per-operator disclose|hide split, required Duties}          │
        └─────────────────────────────────────────────────────────────┘
              │ graph-set                │ disclose/hide split        │ duties
              ▼                          ▼                            ▼
        sparq-solid DatasetView    sparq-mpc RQ2a step 3        sparq-zk manifest
        (WAC/ACP access gate;      (which operator outputs       (Duty=proveAttestation
         O(1) visibility)           go into MPC vs are            → issuer-set membership
                                    disclosed in clear)            proof; the Duty is
                                                                   *discharged by a proof
                                                                   in the ProofManifest*)
```

Three clean seams, each onto code that already exists:

1. **Access gate (today, exact fit).** ODRL `Permission(action=read, party=assignee)` with no
   purpose/recipient constraint is *isomorphic* to a WAC/ACP grant. `sparq-solid` already materializes
   grants into a graph-set and enforces it via `DatasetView` (O(1) per-graph visibility, fail-closed).
   So the **base case of ODRL evaluation reduces to the current `sparq-solid` path** — ODRL is a
   *strictly richer policy source feeding the same enforcement*.

2. **Disclosure split (research, the headline).** ODRL `Constraint(recipient=...)` and
   `Prohibition(disclose ... to ...)` *decide* `sparq-mpc`'s RQ2a per-operator "disclosed vs hidden"
   question (`research/mpc-zkp-research-and-architecture.md` §4.3 step 3). Today that split is a
   protocol design choice; under this design it is **policy-derived and node-local**: each node's ODRL
   policy says which of *its* operator outputs may be revealed in clear to which peer, and which must
   stay inside the secret-shared MPC. The "no-proof-of-revealed-properties" rule (arch §4.3 #4) means
   anything ODRL *permits* disclosing is recomputed by the recipient outside the circuit; anything
   ODRL *prohibits* disclosing must be computed under MPC or proven in ZK — so **ODRL literally draws
   the cryptographic core boundary.**

3. **Duty as proof obligation (research, the elegant part).** An ODRL `Duty` attached to a permission
   ("you may query my graph **provided** you prove the result derives only from issuer-attested
   sources" / "provided the value is anonymized" / "provided use is logged") maps onto sparq's
   **ProofManifest**: the Duty's fulfilment is a *public-input obligation in the proof*. The arch
   already notes the composition obligation is "a pure-data Lean theorem `ProofManifest × Query → Bool`,
   detached from crypto" (§ on the modular dispatcher) — **the natural place to also check
   `Duties(policy) ⊆ DischargedObligations(manifest)`.** Verifying the response = verify the ZK proof
   **and** check every ODRL Duty has a corresponding discharged obligation in the manifest.

### 2.2 What this buys (the differentiator)

- **Standard in, verifiable out.** Inputs are plain ODRL (the language IDSA / Gaia-X / Solid already
  speak), so sparq plugs into the data-spaces ecosystem; outputs are sparq's verifiable,
  minimal-disclosure responses. The bridge "ODRL Duty → ZK proof obligation" is, to the cited
  literature, **unbuilt for SPARQL**.
- **Node-sovereign disclosure.** Each node's own ODRL policy controls its own disclosure — no central
  PDP must be trusted, matching the trustless-intermediary direction of arXiv 2410.16442 and the EU
  DGA.
- **Purpose-binding + retention as first-class.** ODRL `purpose` (via DPV) and time/`count`
  constraints give purpose-bound and retention-limited querying that WAC/ACP cannot express — directly
  useful to the Solid/PSS sibling.

### 2.3 Honest blockers (research-grade, must be stated)

- **Inherits the whole MPC envelope.** The disclosure-split seam only means anything under
  honest-majority, LAN, ≤10³–10⁴ triples/party, few-pattern BGPs (arch §"Viable regime"), and is
  **hard-blocked** on the outstanding RQ1 verifier-soundness remediation
  (`research/zk-soundness-audit.md`). ODRL on top of an unsound proof proves nothing.
- **ODRL conflict/abrogation semantics** are still stabilizing (Formal-Semantics CG is recent); a
  malformed/conflicting policy must fail **closed**, consistent with `sparq-solid`'s fail-closed
  posture. The query-answering semantics (§1.2) is the safest formal basis to adopt.
- **Duty→obligation mapping is bespoke per Duty type.** "prove attestation" maps cleanly (it *is* the
  existing set-membership proof); "anonymize" / "delete after T" / "notify" are real obligations whose
  *enforcement* (vs *declaration*) needs runtime support a query engine cannot fully give alone
  (post-hoc retention/deletion is environmental). Be honest that sparq can *declare + prove
  pre-conditions*, not enforce all post-conditions.

---

## 3. CANDIDATE FEATURE TABLE

FIT legend: `clear-fit:<component>` = lands in an existing/obvious crate;
`new-component-but-fits` = warrants a new crate but composes cleanly with the estate;
`ambiguous-ask-user` = scope/positioning decision for @jeswr. Impact 1–5, Effort S/M/L.

| # | Feature | FIT | Impact | Effort | Rationale + source |
|---|---------|-----|--------|--------|--------------------|
| 1 | **ODRL evaluator** (`Policy × Request × State-of-World → Compliance Report`) over RDF, on the existing N3/EYE reasoner | `new-component-but-fits` (new `sparq-policy`; reuses `sparq-reason`/`sparq-solid` N3) | 4 | M | The kernel everything else needs; ODRL eval *is* a SPARQL/SHACL/N3 workload — in-family for sparq. Mirror the SolidLab `odrl-evaluator` (N3+EYE+SHACL) but native-Rust. [arXiv 2509.05139](https://arxiv.org/html/2509.05139v1), [SolidLab ODRL-Evaluator](https://github.com/SolidLabResearch/ODRL-Evaluator) |
| 2 | **ODRL→sparq-solid bridge**: evaluate ODRL access permissions, materialize into the existing auth-view graph-set (`AUTH_GRAPH`), enforce via `DatasetView` | `clear-fit:sparq-solid` (+ `sparq-policy`) | 4 | M | The base case of ODRL reduces to the WAC/ACP path sparq already enforces fail-closed; OAC/ACP profiles show the mapping. Lowest-risk, immediately useful (purpose/recipient richer than WAC). [OAC](https://besteves4.github.io/odrl-access-control-profile/oac.html), [ODRL→ACP consent](https://harshp.com/research/publications/048-odrl-profile-consent-solid-acp) |
| 3 | **Purpose-binding + DPV constraints** on queries (purpose / recipient / time / count constraints gate a query; fail-closed) | `clear-fit:sparq-solid` (consumes #1) | 4 | M | GDPR-aligned usage control WAC/ACP cannot express; high value to the Solid/PSS sibling. DPV+ODRL is the established vocabulary. [OAC + DPV](https://besteves4.github.io/odrl-access-control-profile/oac.html) |
| 4 | **Triple/quad-level usage-control query rewriting** driven by ODRL (extend `rewrite_for`/`wrap_for_view` beyond graph-set to predicate/value-level constraints + result transforms: redact/anonymize/aggregate-only) | `clear-fit:sparq-solid` | 4 | L | sparq already owns a sound rewrite; Kirrane et al. give the formal triple-level construction incl. updates & negation. The "transform results" (anonymize/aggregate-only) part is the usage-control delta over access control. [arXiv 2007.00461](https://arxiv.org/pdf/2007.00461) |
| 5 | **Obligation / Duty model + manifest discharge check** (`Duties(policy) ⊆ Discharged(manifest)`; `Duty=proveAttestation` → existing issuer-set-membership proof) | `new-component-but-fits` (`sparq-policy` × `sparq-zk-compose` manifest) | 5 | L | The elegant ZK composition: ODRL Duty becomes a proof obligation; arch already frames manifest-covers-query as a pure-data theorem — extend it to cover Duties. Headline differentiator, but research-grade. [mpc-zkp arch §dispatcher], [ACM obligations for SPARQL-DL](https://dl.acm.org/doi/10.1145/2993318.2993337) |
| 6 | **ODRL-driven disclosure split in federation** (per-node ODRL policy decides `sparq-mpc` RQ2a disclosed-vs-hidden per operator + recipient) | `ambiguous-ask-user` (`sparq-policy` × `sparq-mpc`; positioning + scope call) | 5 | L | THE novel angle; no published system does ODRL-drives-MPC-disclosure for SPARQL ([arXiv 2410.16442](https://arxiv.org/abs/2410.16442) validates the direction, not the construction). Inherits the entire MPC honest-majority/LAN/small-data envelope + RQ1 soundness block — needs @jeswr's call on whether this is v-next research or a paper. [mpc-zkp arch §4.3] |
| 7 | **ODRL policy conflict detection** (permission/prohibition conflict, requester-vs-provider containment) via N3 reasoning, fail-closed | `clear-fit:sparq-policy` | 3 | M | Required for correctness once #1 exists; query-containment comparison semantics give the algorithm; SolidLab validator already does conflict detection via N3. [arXiv 2509.05139 §comparison](https://arxiv.org/html/2509.05139v1) |
| 8 | **SHACL-shape result gate** (validate query results against a policy-derived SHACL shape before disclosure) | `clear-fit:sparq-shacl` (consumes #1) | 3 | M | sparq has `sparq-shacl`; a policy-to-shape output gate is a cheap, sound "egress filter" complementary to rewriting. [arXiv 2008.06265](https://arxiv.org/pdf/2008.06265) |
| 9 | **ODRL `Agreement` audit trail** (record matched request-vs-policy as a queryable provenance graph) | `clear-fit:sparq-solid` | 2 | S | GDPR accountability; cheap (just assert an `Agreement` graph, queryable like `AUTH_GRAPH`). Nice-to-have. [OAC Agreements](https://besteves4.github.io/odrl-access-control-profile/oac.html) |
| 10 | **Data-spaces / IDSA-Gaia-X interop profile** (ingest ODRL usage policies from EDC connectors; speak the dataspace dialect) | `ambiguous-ask-user` (ecosystem-scope call) | 3 | L | Big strategic upside (plug sparq into the EU data-spaces stack) but large surface + depends on #1–#6; is this in sparq's mission scope? @jeswr call. [IDS Usage Control](https://international-data-spaces-association.github.io/DataspaceConnector/Documentation/v5/UsageControl) |

**Suggested sequencing:** #1 → #2/#3 (clear-fit, ship usage-control single-node value fast) → #7 →
# 4/#8/#9 → then the research arc #5 → #6 (gated on RQ1 soundness + an @jeswr scope decision) → #10
(ecosystem, optional).

**Clear-fit vs ambiguous, summarized:**
- **Clear-fit / buildable now:** #1 (new `sparq-policy` crate, but unambiguously in-family), #2, #3,
  #4, #7, #8, #9 — a single-node ODRL usage-control layer over `sparq-solid`/`sparq-shacl`.
- **Ambiguous / needs @jeswr:** #6 (the federated disclosure-control headline — research-grade, MPC
  envelope, paper-vs-product call) and #10 (data-spaces ecosystem scope). #5 straddles: the
  mechanism is clear, but it only *means* something once the ZK estate is sound.

---

## 4. Sources

- ODRL Formal Semantics — https://w3c.github.io/odrl/formal-semantics/
- ODRL Landscape — https://w3c.github.io/odrl/landscape/
- Evaluation and Comparison Semantics for ODRL (arXiv 2509.05139) — https://arxiv.org/html/2509.05139v1
- SolidLab ODRL-Evaluator — https://github.com/SolidLabResearch/ODRL-Evaluator
- Interoperable Interpretation and Evaluation of ODRL Policies (Springer 2025) — https://link.springer.com/chapter/10.1007/978-3-031-94578-6_11
- OAC — ODRL Profile for Access Control — https://besteves4.github.io/odrl-access-control-profile/oac.html
- ODRL Profile for Consent in Solid (ACP) — https://harshp.com/research/publications/048-odrl-profile-consent-solid-acp
- From Access Control to Usage Control with UMA (arXiv 2601.18761) — https://arxiv.org/pdf/2601.18761
- Query-Based Access Control for Linked Data (Kirrane et al., arXiv 2007.00461) — https://arxiv.org/pdf/2007.00461
- Rewriting of SPARQL/Update Queries for Securing Data Access (Springer) — https://link.springer.com/chapter/10.1007/978-3-642-17650-0_2
- SAFE: Policy-Aware SPARQL Federation over RDF Data Cubes (PMC) — https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5288952/
- Modeling and Enforcing Access Control Obligations for SPARQL-DL (ACM) — https://dl.acm.org/doi/10.1145/2993318.2993337
- Towards Querying in Decentralized Environments with Privacy-Preserving Aggregation (arXiv 2008.06265) — https://arxiv.org/pdf/2008.06265
- A SPARQL-based framework to preserve privacy of sensitive RDF (Springer) — https://link.springer.com/article/10.1007/s11761-023-00368-6
- Secure Computation and Trustless Data Intermediaries in Data Spaces (arXiv 2410.16442) — https://arxiv.org/abs/2410.16442
- IDS Usage Control — Dataspace Connector — https://international-data-spaces-association.github.io/DataspaceConnector/Documentation/v5/UsageControl
- Interoperable and Continuous Usage Control Enforcement in Dataspaces (CEUR Vol-3705) — https://ceur-ws.org/Vol-3705/paper10.pdf
- Policy Patterns for Usage Control in Data Spaces (CEUR Vol-3510) — https://ceur-ws.org/Vol-3510/paper_sem4tra_1.pdf
- sparq internal: `research/mpc-zkp-research-and-architecture.md`, `research/zk-soundness-audit.md`,
  `crates/sparq-solid/src/{lib,rewrite}.rs`, `crates/sparq-mpc/src/lib.rs`,
  `crates/sparq-zk-compose/README.md`.
