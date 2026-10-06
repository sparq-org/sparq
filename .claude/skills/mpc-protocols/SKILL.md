---
name: mpc-protocols
description: Secure Multi-Party Computation primitives and the honest SOTA for MPC over (federated) SPARQL — secret sharing, garbled circuits, collaborative zk-SNARKs, authenticated-input MPC, and what is / is NOT achievable today. Use when working on the sparq-mpc crate (RQ2), choosing an MPC trust model or primitive, reasoning about the four distinct guarantees (confidentiality / correctness / attestation / malicious-security), composing MPC with the existing ZK estate, or sizing the feasibility envelope. Complements the ZK skills (noir-*, verifiable-credentials-zk, sparql-formal-semantics) — those cover single-prover ZK; this covers the multi-party layer.
---

# MPC protocols for federated SPARQL (RQ2)

Design-for-review. Ground
truth: `research/mpc-zkp-research-and-architecture.md` and the `sparq-mpc` crate
(`crates/sparq-mpc/{src/lib.rs,PLAN.md}`, currently a Milestone-0 scaffold). Every
crypto claim must trace to a citation in the architecture doc — do not invent.

## The frame: four guarantees, kept DISTINCT

The single most common error in this space is conflating guarantees. Keep them
separate at all times:

- **(A) Confidentiality** — no party learns another's inputs.
- **(B) Computation correctness** — the output is the right answer.
- **(C) Input authentication** — "derived from attested sources": the data is
  issuer-signed under a key in the disclosed key-set `K`.
- **(D) Malicious security** — guarantees hold against actively cheating parties,
  not just honest-but-curious ones.

Three pitfalls encoded as hard assumptions: **commitment ≠ attestation** (proving
over committed data says nothing about who *signed* it — needs a CP-SNARK/Dutta
link); **confidentiality ≠ correctness** (a "no party learns X" system can still
return wrong answers — most graph-MPC papers only deliver A); **honest-majority ≠
malicious-against-N−1**.

## MPC primitive families (pick by network + trust model)

- **Secret sharing.** Honest-majority 3PC *replicated* SS is the perf sweet spot
  (1 element/mult-gate; ~2× for malicious) but assumes ≥2-of-3 non-colluding
  compute parties — a *strong* assumption for a hostile federation.
  Dishonest-majority malicious (SPDZ / MASCOT / Overdrive) is the realistic
  cross-org model but pays expensive input-independent **preprocessing** usually
  excluded from headline numbers.
- **Garbled circuits (Yao / BMR).** Constant-round → wins on **WAN** (the
  federated setting). Secret-sharing is round-per-circuit-depth → wins on LAN.
  **Network model dictates protocol.**
- **PSI for private joins.** VOLE-PSI: 2²⁰ elements, malicious, 6.2 s. But it is
  *two-party, single equi-join, key-on-key* — it does **not** compose for free
  into a multi-pattern SPARQL BGP or non-key / unbound-variable joins.

## ZK ∩ MPC — where this project lives

- **Collaborative zk-SNARKs (coZK).** Multiple provers over a *secret-shared*
  witness emit ONE proof; verifier unchanged. Ozdemir–Boneh (USENIX'22):
  malicious-minority ≈ single-prover cost — but only at 3 Gb/s LAN,
  honest-majority, preprocessing excluded. **Caution: 2025/1026 showed the coZK
  template had exploitable soundness/privacy pitfalls** (proving on invalid
  witnesses leaks honest inputs). Any build must use *patched* constructions and
  re-audit; "free malicious security" holds only honest-majority, post-patch.
- **Authenticated-input MPC — the attestation pillar (C).** Dutta et al.
  (Asiacrypt'24, eprint 2022/1648) is **essentially the only** work giving
  signature-authenticated inputs + verifiability together — and it is
  **honest-majority, linear-secret-sharing only**, and yields authenticated-MPC,
  *not* a succinct public ZK proof. The whole "attested sources" half rests on
  this one building block.
- **MPCitH ≠ coZK.** MPC-in-the-head (Picnic/FAEST) is *single-prover* simulated
  MPC giving transparent PQ NIZKs — do **not** conflate with real multi-prover
  coZK.

## Honest SOTA for graph / relational MPC

- Every published **graph/SPARQL** crypto system (GOOSE, SMPG/PPMQ, GORAM) is
  semi-honest **and/or** confidentiality-only **and/or** conjunctive-only.
  **Malicious-secure, attested-input, full-SPARQL federation has ZERO published <!-- privacy-claims-allow: negative literature statement (zero published instances); sq-toze.35 -->
  instances.** PPMQ's sub-ms numbers are single-desktop, co-located, no
  inter-party network — non-evidence for federation.
- Relational MPC (Senate, Secrecy, Conclave, **ORQ SOSP'25** — first full TPC-H
  under MPC) shows the honest envelope is **minutes-to-tens-of-minutes** (ORQ Q21
  ≈ 42 min LAN malicious), not "a few minutes". **Joins are the cost center** —
  obliviousness forces worst-case padding at every intermediate step.

## Feasibility verdict (state this honestly, never "seconds")

- **Viable regime:** honest-majority (or honest-but-curious among *cooperating*
  holders), LAN/datacenter co-location, **small committed datasets
  (≤10³–10⁴ triples/party)**, few-pattern BGPs, with disclosed-property
  aggregates recomputed by the verifier. Aligns with the existing
  sub-second-for-≤10³-triples RQ1 figure. A credible first target.
- **Impractical / unquantified-research-risk regime:** non-trivial multi-way BGP
  join over multi-party RDF under **dishonest-majority malicious on a WAN** — no
  published system does this; the full composition (coZK ⊕ malicious
  dishonest-majority MPC ⊕ oblivious BGP joins ⊕ attested inputs ⊕ WAN) has
  **zero performance data points**.
- **The contribution is the composition, and so is the risk.** This is
  *integration of three research lines* (collaborative proving + authenticated-
  input MPC + the RQ1 SPARQL-correctness circuit with global-IRI join keys), not
  wiring off-the-shelf parts. "The join nobody has built" = verifying a
  BBS+/EdDSA signature over a *secret-shared* witness inside a collaborative
  proof — unsolved in the literature.

## Hard dependency — do not start the MPC build prematurely

**No MPC+ZK build means anything to a relying party until the RQ1
verifier-soundness remediation is complete** (`research/zk-soundness-audit.md`).
The MPC attestation step IS issue #3 (issuer-signature + key-set membership)
generalised to multiple sources — it cannot exist before #3 lands single-source.
Prerequisites: #3/#4 (signature + replay/freshness binding), #5/#6 (FILTER
operator/bound/verdict binding — the £25k/£100k threshold IS the FILTER),
#8/#9 (per-row source-graph attribution + salt separation), #12 (revocation).
Until these land, the MPC layer would compose onto a verifier that returns
`Ok(())` for forged results — theatre.

## Load-bearing project conventions

- **No-proof-of-revealed-properties.** Anything that is a deterministic function
  of the *disclosed* multiset (DISTINCT, ORDER BY, LIMIT/OFFSET, COUNT, SUM, AVG,
  MIN, MAX) is **recomputed by the verifier outside the MPC**, never proven
  in-circuit. Cross-source join/aggregation over *disclosed* values stays OUT of
  the cryptographic core. HAVING / GROUP BY over hidden multisets are forbidden.
  This is what keeps the BGP-join obliviousness cost tractable.
- **Global IRIs as cross-credential join keys** — the distinguishing feature vs
  all prior graph-MPC (GOOSE/SMPG use node-local Cypher ids, disqualified for
  federation).
- **Modularity is the contribution.** MPC primitives parameterise on the same
  `commit` / signature interfaces as the RQ1 estate — never hardcode a concrete
  scheme.
- **No fake crypto.** The `sparq-mpc` M0 scaffold builds only fork-*invariant*
  parts (real per-holder local sub-eval via `sparq-engine`); everything
  fork-dependent returns `MpcError::NotYetImplemented` naming its gating
  milestone. Keep it that way.

Driving use case: four flatmates, each with a Solid Pod of payslip VCs, jointly
prove **cumulative salary > £100k** to a landlord without revealing individual
salaries (Wright, CEUR Vol-4085).
