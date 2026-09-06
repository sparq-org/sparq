<!-- [OPUS-4.8] The untrusted-planner → MPC-routing seam: how sparq-fedplan source
selection + a disclosed/hidden partition feed the sparq-mpc pipeline WITHOUT the
cryptographic layer trusting the plan for soundness. Originally design-for-review (no code),
Opus 4.8 (Fable unavailable) — re-review when Fable returns. Date: 2026-06-19. Bead sq-pwr /
sq-0jsc. GREENLIT via issue #755; Phases 1-4 + 6, and the canonicalisation half of Phase 5
(sq-1fo4, [OPUS-5]), have since landed in
crates/sparq-fedplan-mpc/ — see the decision log under the Status line. -->

# The untrusted-planner → MPC-routing seam (federated source selection + disclosed/hidden partition)

**Status:** Deep-research design record — **greenlit and PARTLY IMPLEMENTED** (see the
decision log below). *Originally* filed as a doc-only proposal for the maintainer, not a
fait accompli. Author: Opus 4.8 (Fable unavailable — flag for re-review).
Date: 2026-06-19. Parent epics: **sq-pwr** (MPC over federated SPARQL with ZKP of correctness
+ attested-source derivation), **sq-0jsc** (MPC research track).

> ## Decision log — [issue #755](https://github.com/sparq-org/sparq/issues/755)
>
> #755 held implementation pending a greenlight plus the two structural choices of §9.
> Both are now **RESOLVED as recommended**, and the resolution is recorded here so this
> record stops presenting as open a question the shipped code has already answered:
>
> 1. **§9 Q1 — crate vs feature → the STANDALONE CRATE (§7 Option (A)).** `sparq-fedplan-mpc`
>    is a workspace member (`publish = false`), *additionally* behind its own `fedplan-mpc`
>    cargo feature which is **OFF by default**; the default build compiles an empty crate and
>    pulls in neither upstream, and neither `sparq-fedplan` nor `sparq-mpc` gains a dependency
>    on the other. Evidence: `crates/sparq-fedplan-mpc/Cargo.toml`.
> 2. **§9 Q2 — disclosure posture → DEFAULT-DENY**, in the two-layer shape the code actually
>    took (the question was posed as a binary; the answer is not):
>    * **Descriptor / predicate layer — default-deny, no exceptions.** A predicate is `Private`
>      unless its source *explicitly* marks it `Public`; `SourcePrivacyDescriptor::deny_all`
>      discloses nothing, and an operator is routed `Disclosed` only when **every** operand is
>      disclosable and **every** contributing source opted it in (the most-private source wins).
>      Evidence: `crates/sparq-fedplan-mpc/src/privacy.rs`, `…/src/routing.rs`.
>    * **Global-IRI layer — the convention-#6 shortcut survives as the DEFAULT policy.**
>      `RoutingPolicy::Default` discloses a global-IRI operand unconditionally (the cheap,
>      demo-matching route); `RoutingPolicy::Strict` is the §5 "hide even a public term" knob,
>      under which a global IRI is disclosed only when no contributing source marks its
>      predicate private. Evidence: `routing::operand_disclosable`.
>
> **Landed since** (each its own bead, all opt-in, none in the default build): Phase 1
> `sq-2q1x`, Phase 2 `sq-fix4`, Phase 3 `sq-i1wh2`, Phase 4 `sq-pwr.2`, Phase 6 `sq-pwr.3` +
> `sq-xkrt`, and Phase 5 `sq-1fo4` — **its canonicalisation half ONLY** (a canonical plan
> commitment + a fail-closed re-validation; the *soundness* half, the binding into a collaborative
> proof, is deferred and makes no soundness claim).
> See §8 for the per-phase status and `crates/sparq-fedplan-mpc/README.md` for the honest
> boundary. **§9 Q3** (where the Phase-5 untrusted-plan re-validation binds) is now **PARTLY
> ANSWERED** — `sq-1fo4` settled *what* is bound and confirmed no landed plumbing had to change;
> *where* it attaches is still open. **§9 Q4** (B7 authorisation scope) is now **RESOLVED**
> (`sq-lzvl`): B7 belongs with `sparq-solid`, which owns WAC/ACP, and only *references* this
> seam — see Phase 7 in §8. No dependency edge was created in either direction and nothing in
> `sparq-fedplan-mpc` changed.
>
> **The audit posture is unchanged by any of this.** Everything landed is *plumbing* —
> source selection, a combination prune, a disclosed/hidden partition, and leakage
> accounting + a plan-time policy gate. It performs no MPC and runs no privacy-bearing
> cryptographic logic. The MPC estate remains research-grade, honest-majority **semi-honest
> only**, and **NOT externally audited**: the accredited-cryptographer sign-off (`sq-qhy4`)
> and the collaborative-coZK re-audit (`sq-9hrn`) are pending. Nothing shipped under this
> record claims soundness or privacy, and the privacy-bearing Phase 5 stays gated on those
> audits.

**The one-paragraph thesis.** The MPC estate already has a *hand-built* per-query routing
record — `sparq-mpc::pipeline::FederatedQuery` / `Routing` / `OperatorRouting` — whose own
docstrings call it *"the (untrusted, §4.1) planner's output, made into typed data."* But there
is **no planner that produces it**: it is constructed by hand for the four-flatmates demo, and
the mature cost-based federation planner (`sparq-fedplan`: `select_sources`, `plan_bgp`,
`SourceDescriptor`) has **zero** knowledge of MPC, while `sparq-mpc` has zero reference to
`sparq-fedplan`. This record fills the seam between them: an **untrusted** planner that
(a) selects which sources hold which patterns, and (b) partitions every operator into a
*disclosed* (recompute-in-the-clear) vs *hidden* (run-in-MPC) route — feeding the existing
`OperatorRouting` — while the cryptographic layer trusts the plan for **neither soundness nor
the leakage bound**. It surveys the relational-MPC planning line (Conclave / Senate / SMCQL /
Secrecy / ORQ) that already solved the cleartext/secure split for SQL, maps it onto sparq's
RDF-specific conventions (global-IRI join keys, no-proof-of-revealed-properties), and recommends
a **new opt-in glue crate** `sparq-fedplan-mpc` rather than coupling either existing crate.

---

## 1. Why this is a gap (the inventory, honest)

The MPC/ZK estate under `research/` is large and current. Before proposing anything, the
relevant existing records and what each *does* cover:

- [`mpc-zkp-research-and-architecture.md`](./mpc-zkp-research-and-architecture.md) — the
  four-guarantee framing, the six-step protocol flow, and §4.1's explicit decision to treat
  the **planner / coordinator as UNTRUSTED** ("its plan is an input the cryptographic layer
  must not have to trust for soundness"). It *names* the stance but does not specify the
  planning **algorithm**, the source-selection step, or how the untrusted plan is reconciled
  with leakage.
- [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) — the per-operator ×
  per-configuration capability tiers and the disclosed/hidden two-regime split (its §1.3). It
  resolves *what each operator costs* in MPC, not *who decides which regime each operator takes
  for a given query over a given set of sources*.
- [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) — cites
  the relational-MPC planning line (Conclave hybrid MPC+cleartext, Senate's
  party-subset decomposition planner, SMCQL split-execution) as **lessons for sparq's
  planner**, and §6.1 explicitly calls decomposition+planning "directly relevant to sparq's
  untrusted-planner stance." It catalogues the prior art but stops at "lessons" — it does not
  turn them into a sparq planning design.
- [`federation-client-design.md`](./federation-client-design.md) — the full `sparq-fedclient`
  design (capability discovery, source-type abstraction, cost-based pushdown, streaming
  operators). Its only mentions of MPC/ZK are a disclaimer (the ZK/MPC estate is "irrelevant"
  to the federation measurement) and listing `sparq-mpc` as a sibling opt-in crate. **It is
  deliberately MPC-agnostic.**
- [`feature-research-federation.md`](./feature-research-federation.md) — flags exactly this
  seam as undecided: item **Z1** ("privacy-preserving federation — combine the federation
  client with `sparq-mpc`/`sparq-zk`", impact 5, effort Large, `ambiguous-ask-user`), item
  **B7** (access-control-aware source skipping, "the natural seam for sparq's ZK/MPC privacy
  track"), and **Z2** (char-set-driven source selection). All three are marked needing a
  maintainer decision; **none has a design record.**

**Code ground truth (verified against `main`, 2026-06-19):**

- `sparq-mpc` has **no** dependency on, or reference to, `sparq-fedplan` or `sparq-fedclient`
  (`grep` for `sparq_fedplan` / `sparq_fedclient` in `crates/sparq-mpc/` returns nothing).
- `sparq-mpc::pipeline::FederatedQuery` is documented as the untrusted planner's output, but is
  hand-constructed per call; `Routing { Disclosed | Hidden(OperatorClass) }` and
  `OperatorRouting { operator, routing }` exist and are surfaced in `FederatedResponse::routing`
  for auditability — but **the routing is chosen by the caller**, with no algorithm deriving it.
- `sparq-fedplan` exposes `select_sources(bgp, &[SourceDescriptor]) -> Vec<PatternSources>`,
  `plan_bgp(...)` (cost-based join-tree planning), and `SourceDescriptor` (predicate/class/
  char-set partitions, `may_hold_authority`, `star_cardinality`) — a complete classic federated
  source-selection + join-ordering surface, with **no awareness** that a pattern might be
  private or that an operator might need to run under MPC.

So the seam is real, code-grounded, and the brief's flagged "federation-pushdown-under-MPC
question" lands here. **This is a genuine uncovered gap**, not a re-derivation. Everything the
existing records cover (the operator costs, the security taxonomy, the attestation interim) is
*downstream* of a routing decision that nothing currently makes.

---

## 2. Problem framing

### 2.1 What the seam must produce

Given a federated SPARQL query `Q`, a fresh verifier challenge `N`, and a set of holder sources
each with a `SourceDescriptor` plus a *privacy descriptor* (which predicates/columns are
private), produce a **typed plan** that the existing `sparq-mpc` pipeline can consume:

1. **Source selection** — for each triple pattern, which sources can answer it (the classic
   `select_sources` job), restricted by what each source is *willing and authorised* to expose.
2. **A per-operator routing** — the `Vec<OperatorRouting>` the pipeline already accepts: each
   operator tagged `Disclosed` (verifier/holder recomputes in the clear) or
   `Hidden(OperatorClass)` (runs under MPC). This is the load-bearing RQ2a "disclosure
   minimisation" decision (architecture §4.3 step 3), today made by hand.
3. **A declared leakage envelope** — the set of facts the chosen routing reveals (join keys,
   result cardinality, which-source-contributed, etc.), so the relying party can *see and
   accept* the cost-vs-leakage trade before the query runs (the Conclave lesson: a leak-for-
   speed choice must be a **declared bounded leak**, never an implicit one).

### 2.2 The two hard constraints the planner must respect

- **C-A — the plan is UNTRUSTED for soundness.** The cryptographic layer must verify the result
  is correct *regardless of whether the plan was honest*. A malicious or buggy planner that
  drops a source, mis-orders a join, or mis-routes an operator must at worst produce a
  **wrong-but-rejected** or **less-private-but-still-correct** answer — never a *forged-accepted*
  one. (Architecture §4.1; the `pipeline` already re-checks the disclosed-key join independently
  and anchors on a differential-vs-union-store test, so the mechanism exists single-query — the
  gap is generalising it to arbitrary planner output.)
- **C-B — the plan is ALSO untrusted for the leakage bound, but here trust is unavoidable in
  part.** Routing an operator `Disclosed` *reveals* its operands; a dishonest planner can leak
  more by over-disclosing. Soundness (C-A) is unaffected, but *confidentiality* is. The honest
  resolution: the **disclosure decision is the holders' / verifier's to make and ratify, not the
  planner's to impose.** The planner *proposes* a routing + its leakage envelope; each holder
  **independently re-derives** whether the proposed `Disclosed` route for its own data is
  consistent with its private-column policy, and **refuses** (fail-closed) if the plan tries to
  disclose something the holder marked private. The verifier sees the declared envelope and can
  reject a plan that leaks more than its policy allows.

These two constraints are *the* reason this cannot be folded into `sparq-fedplan` (which assumes
a trusted local cost model) nor into `sparq-mpc` (which should stay a primitive/protocol crate,
not a query optimiser). It is genuinely a third concern.

---

## 3. Prior art (what the relational-MPC line already solved)

The cleartext/secure split for *SQL* analytics is a mature line. sparq's contribution is the
RDF/SPARQL-specific reframing (global-IRI join keys, no-proof-of-revealed-properties,
attested sources) — **not** the split idea itself, which it should borrow honestly.

- **SMCQL (PVLDB'17)** — the original *split-execution* idea: a query is partitioned into a
  public slice run in plaintext and a secure slice run in MPC, with a **"split" annotation**
  deciding the boundary. This is exactly sparq's convention #4 (no-proof-of-revealed-
  properties) and exactly the `Routing::{Disclosed, Hidden}` enum — but SMCQL's split is
  schema-annotation-driven and 2-party. The lesson sparq inherits: **the disclosed/hidden
  boundary is a first-class, declared annotation, not an optimiser side-effect.**
- **Conclave (EuroSys'19)** — N-party semi-honest *hybrid MPC+cleartext*; its planner inserts
  **STP (selective trusted party) / hybrid annotations** that trade a *quantified* leak for an
  order-of-magnitude speedup, and it pushes as much work as possible into cleartext at the
  edges. The lesson: leakage-for-speed is real and worth it, but **must be a DECLARED bounded
  leak** the data owners ratify — this is sparq's §2.1(3) leakage envelope.
- **Senate (USENIX'21)** — N-party *maliciously* secure; its planner **decomposes** the
  computation so sub-circuits run on **party-subsets in parallel**, and it is the standout
  evidence that malicious N-party relational analytics is tractable *because of* planning, not
  despite it. The lesson directly relevant to sparq's untrusted-planner stance: decomposition is
  the scaling lever, and a *maliciously-secure* protocol tolerates an adversarial planner by
  construction.
- **Secrecy (NSDI'23) / ORQ (SOSP'25)** — the oblivious-operator cost model and join-aggregate
  fusion. ORQ is the honest performance anchor for "even SOTA relational MPC is minutes-to-tens-
  of-minutes per query on LAN; joins are the cost center." The planner's job is to keep work OUT
  of MPC precisely because the secure path is this expensive.
- **Cerebro (USENIX'21)** — contributes *compute/release policies* + cryptographic auditing
  (accountability) — the model for sparq's **per-holder private-column policy** that gates what
  a plan is allowed to disclose (the C-B fail-closed check).
- **Federated-SPARQL planners (non-crypto): FedX, CostFed, ANAPSID, FedUP (WWW'24).** The
  source-selection + decomposition + join-ordering pipeline sparq's classic planner already
  implements. FedUP's *result-aware* plans (provenance over quotient summaries) are the lever to
  **minimise the source-combination blow-up before any MPC is invoked** — the single most
  valuable pre-MPC optimisation, because every extra source candidate multiplies the secure-join
  cost.

**Honest gap in the prior art:** all of the above are SQL/relational with a *trusted* query
compiler. None binds the split decision to (a) RDF global-IRI join keys as the crypto-free join
substrate, (b) per-source *issuer attestation* of the inputs, or (c) an explicitly **untrusted**
planner whose output the crypto layer re-validates. That triple is the sparq-specific design
work below — and it is integration of known parts, **not** a new cryptographic primitive.

---

## 4. Proposed design

### 4.1 A new opt-in glue crate: `sparq-fedplan-mpc`

The seam is its own concern (§2.2), so it gets its own opt-in member rather than coupling the
two existing crates (consistent with the project's opt-in-feature-architecture rule: core stays
lean; each new capability is its own crate/feature). It depends on `sparq-fedplan` (for
`select_sources` / `SourceDescriptor` / `plan_bgp`) and on `sparq-mpc` (for `Routing` /
`OperatorClass` / `OperatorRouting` / `FederatedQuery`), and produces the typed plan the
`sparq-mpc::pipeline` already consumes. Neither existing crate gains an MPC↔federation
dependency; both stay independently usable. The crate is **off by default** (a `fedplan-mpc`
cargo feature); nothing in the lean core path changes.

**Why not extend `sparq-fedplan`?** Its cost model is local and *trusted*; bolting an untrusted-
plan-validation and a privacy-policy gate onto it would muddy a clean planner. **Why not extend
`sparq-mpc`?** That crate is the protocol/primitive layer; a query optimiser does not belong in
it. The glue is a genuinely separate, smaller concern.

### 4.2 The privacy descriptor (the new input)

`sparq-fedplan::SourceDescriptor` describes a source's *content* (predicates, classes,
char-sets, authorities). The seam needs an orthogonal **privacy descriptor** per source:

- **Private predicate/column set** — which terms must NEVER leave the source in the clear (the
  flatmate salary). Default-deny is the safe posture: a term is disclosable only if the source
  explicitly marks it public (global-IRI facts are public by convention #6).
- **Attestation key id** — the issuer key the source's graph is signed under, collected into the
  disclosed key-set `K` (the `Flatmate::issuer_key` field already models this as an opaque id at
  the current tier; the attestation half stays gated on ZK-remediation #3 and the audits).
- **Authorisation / willingness** — whether the source will participate at all for this
  verifier (the B7 access-control-aware-source-skipping hook; SAFE-style). Out of scope to fully
  design here, but the descriptor reserves the field.

This descriptor is **the source's own declaration** — the planner reads it but the source
re-enforces it (C-B fail-closed), so a lying planner cannot override it.

### 4.3 The routing algorithm (proposed, three passes)

A planner pass over the SPARQL algebra producing `Vec<OperatorRouting>` + a leakage envelope:

1. **Source selection (reuse).** Run `sparq-fedplan::select_sources` to get per-pattern
   candidate sources, then prune by the privacy/authorisation descriptor (a source that won't
   participate, or can't legally answer, drops out). FedUP-style result-aware pruning here is
   the highest-leverage pre-MPC win (§3) — fewer source combinations, less secure work.
2. **Disclosed/hidden partition (the core decision).** For each operator, apply convention #4
   greedily: **route `Disclosed` if every operand is a global IRI or a source-declared-public
   term** (verifier/holder recomputes in the clear — crypto-free); **route `Hidden(class)`
   otherwise**, tagging the `OperatorClass` so the pipeline reads off the per-operator security
   tier. The £100k threshold is the canonical `Hidden(Comparison)`; the membership join on the
   flat IRI is the canonical `Disclosed`. This is the decision `pipeline.rs` makes by hand
   today; the pass derives it from the descriptors.
3. **Leakage-envelope assembly + ratification.** Collect what every `Disclosed` route reveals
   (operands, cardinalities, source-attribution) into a declared envelope. Each holder
   **independently re-checks** that the proposed `Disclosed` routes for *its own* data are
   consistent with its private-column policy and **aborts (fail-closed)** otherwise; the
   verifier checks the envelope against its acceptance policy. Only a plan that survives both
   ratifications is executed.

The output is exactly the `FederatedQuery` + `Vec<OperatorRouting>` the pipeline consumes — so
this design *produces* the structure the pipeline today *receives by hand*, with no change to the
downstream MPC protocol.

### 4.4 How the two trust constraints are discharged

- **C-A (soundness under an untrusted plan).** Unchanged from the architecture's stance and the
  pipeline's existing mechanism: the disclosed-key join independently re-checks the join key, and
  the differential-vs-union-store anchor catches a wrong result. The proposed generalisation: the
  *eventual* collaborative proof (the gated, audit-pending step — `proof.rs` is honestly a
  `NotYetImplemented` stub) binds the result to `Eval_PAG(Q, D)` over the *actual* sources, so a
  plan that drops or mis-routes a source yields a proof that fails to verify, not a forged accept.
  **Until that proof lands and is audited, the soundness claim is NOT met** — see §6. `sq-1fo4` has
  since landed the *canonicalisation* half of that generalisation (`commit_plan` /
  `revalidate_plan`, §8 Phase 5): the plan is now reducible to one canonical public-input word, and
  a recomputed plan is checked against it fail-closed. That word is **not yet bound into any
  proof**, so it is a transcript-integrity check that a malicious planner passes trivially — C-A
  stays *designed, not delivered*.
- **C-B (leakage under an untrusted plan).** Discharged by the §4.3-pass-3 dual ratification:
  the disclosure decision belongs to the data owners and the verifier, not the planner. A
  dishonest planner can at most propose an over-disclosing route, which a holder's fail-closed
  policy check rejects before any secret leaves the source.

### 4.5 Worked example (the four-flatmates query, end to end through the seam)

- **Sources:** four holders, each `SourceDescriptor` declaring `ex:memberOf` (public,
  global-IRI) and a privacy descriptor marking `ex:salary` private.
- **Pass 1 (selection):** all four answer the `?h ex:memberOf ex:flat` pattern; none is pruned
  (all participate).
- **Pass 2 (partition):** the membership join on `ex:flat` (a global IRI) → `Disclosed`; the
  cumulative-salary `> £100k` comparison (private operands) → `Hidden(Comparison)`.
- **Pass 3 (envelope + ratify):** the envelope declares "membership multiset disclosed; only the
  threshold verdict bit disclosed; exact cumulative salary never reconstructed." Each holder
  confirms `ex:salary` stays in the `Hidden` route (its policy holds → no abort); the verifier
  accepts the one-bit-output envelope.
- **Execution:** the produced `FederatedQuery` + routing feeds the existing
  `sparq-mpc::pipeline` driver unchanged — which is precisely the four-flatmates scenario that
  driver already runs. **The seam's job is to derive, rather than hand-write, that input.**

---

## 5. Options considered + honest trade-offs

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **(A) New `sparq-fedplan-mpc` glue crate** (recommended) | clean separation; neither existing crate gains a cross-dependency; opt-in; matches the precedent of standalone opt-in members | one more crate to maintain; the routing pass is genuinely new code | **Recommend** |
| (B) Extend `sparq-fedplan` with MPC-awareness | reuses the cost model in place | couples a trusted local planner to an untrusted-plan + privacy-policy concern it shouldn't own; bloats the federation planner; breaks the "fedclient is MPC-agnostic" design it already states | Reject |
| (C) Extend `sparq-mpc::pipeline` to do its own planning | keeps it one crate | puts a query optimiser inside the protocol/primitive crate; pulls `sparq-fedplan` into the crypto crate's dependency graph; conflates concerns | Reject |
| (D) Do nothing — keep hand-writing `FederatedQuery` per query | zero new code; fine for the single demo | does not scale past the four-flatmates demo; no source selection; no declared leakage envelope; the RQ2a decision stays implicit and un-auditable across queries | Reject for the general case; acceptable ONLY for the existing single demo |

**The honest trade-off on the disclosed/hidden greedy heuristic (§4.3 pass 2):** greedily
disclosing every global-IRI operand is the *cheapest* route and is correct for the driving use
case, but a more-private deployment may want to *hide* a global IRI too (e.g. to avoid leaking
*which* flat). The pass must therefore be **policy-parameterised**, not hard-coded to "disclose
all IRIs" — the privacy descriptor's default-deny posture (§4.2) supports a strict mode where
even a public-by-convention term is hidden if the source marks it so. This is a real cost-vs-
privacy knob, and the leakage envelope makes the chosen point explicit.

---

## 6. Honesty / what this design does NOT claim

This is a **proposal**, and the whole construction sits on top of an estate with explicit,
load-bearing caveats that this record does not weaken:

- **The collaborative ZK proof that would make the result sound-and-attested under an untrusted
  plan does NOT exist** — `sparq-mpc::proof` is an honest `NotYetImplemented` stub, and the
  pipeline today produces a `FederatedResponse` with **no proof**. So C-A's soundness argument
  (§4.4) is *designed*, not *delivered*: until the proof lands, the seam produces a correct
  disclosed result with a differential anchor, not a third-party-verifiable one.
- **The MPC layer is honest-majority, semi-honest only.** Nothing here promotes that; a
  malicious planner is tolerated for *soundness* only to the extent the (future, audited) proof
  and the existing differential re-check provide, and the malicious-security work is a separate
  in-flight line (the IT-MAC beads).
- **The ZK/MPC estate is NOT externally audited.** The external accredited-cryptographer audit
  (sq-qhy4, P0) is **pending**, and a fresh coZK soundness re-audit of any collaborative path
  (sq-9hrn) is required before any soundness/attestation claim. This record makes **no**
  unqualified privacy, soundness, or security claim, and the routing it designs does not change
  the estate's audit posture by one inch.
- **No performance numbers.** The §3 cost framing ("minutes-to-tens-of-minutes", "joins are the
  cost center") is the *published-literature* anchor (ORQ), cited as such — not a sparq
  measurement. No work-box timing appears here; sparq's own numbers for any of this do not exist
  yet and the work-box is non-canonical regardless.
- **B7 authorisation and Z2 char-set source pruning are reserved, not designed.** The privacy
  descriptor (§4.2) names the hooks; their full design (SAFE-style WAC/ACP-aware skipping; the
  char-set miner reuse) is out of scope and stays `ambiguous-ask-user`.

---

## 7. Recommendation

> **ACCEPTED** (issue #755 — see the decision log at the top). Option (A) is what shipped:
> `crates/sparq-fedplan-mpc`, opt-in behind the `fedplan-mpc` feature, OFF by default.

Adopt **Option (A)**: a new opt-in `sparq-fedplan-mpc` glue crate that turns the
hand-written `FederatedQuery`/`OperatorRouting` into the *output of an untrusted, policy-gated
routing pass* over `sparq-fedplan` source selection + a per-source privacy descriptor — with the
disclosed/hidden partition **ratified by the data owners and the verifier**, not imposed by the
planner. This is integration of known parts (relational-MPC split-execution + sparq's existing
federation planner + the existing MPC routing enum), not new cryptography. It is **decoupled**
from, and **does not gate**, the audit-pending crypto: the seam can be built and differentially
tested today; it simply does not *claim* soundness/attestation until the proof + audits land.

Sequencing intent: build the **descriptor + greedy-partition + envelope** core first (it is
useful immediately and unblocks the demo's generalisation), defer the **untrusted-plan
re-validation** until it can ride the collaborative proof, and treat **B7 authorisation** as a
later, separately-scoped step.

---

## 8. Phased plan (each phase a future bead)

Ordered by leverage / dependency. Each is opt-in (`fedplan-mpc` feature), touches neither the
lean core nor the audit posture, and is differentially testable against the existing
four-flatmates pipeline.

> **Status as of 2026-08-01** (issue #755 greenlight — see the decision log at the top).
> **LANDED:** Phases 1 (`sq-2q1x`), 2 (`sq-fix4`), 3 (`sq-i1wh2`), 4 (`sq-pwr.2`),
> 6 (`sq-pwr.3` + `sq-xkrt`), in `crates/sparq-fedplan-mpc/`. **PARTLY LANDED:** Phase 5
> (`sq-1fo4`) — its *canonicalisation* half only; the **soundness** half stays audit-gated on
> `sq-qhy4` / `sq-9hrn` and on §9 Q3, and nothing in it may be presented as sound.
> **LANDED OUTSIDE THIS CRATE:** Phase 7 (`sq-lzvl`) — §9 Q4 resolved to `sparq-solid`, so
> the hook lives in `crates/sparq-solid/` and this crate is unchanged. Per-phase annotations
> below.

1. **Phase 1 — `sparq-fedplan-mpc` crate skeleton + privacy descriptor.** **[LANDED — `sq-2q1x`]** Off-by-default opt-in
   crate depending on `sparq-fedplan` + `sparq-mpc`; define `SourcePrivacyDescriptor`
   (private-term set, attestation key id, reserved authorisation field) with default-deny
   semantics. Dependency-boundary proof (default build pulls neither into the other). *(P2)*
2. **Phase 2 — source-selection adapter.** **[LANDED — `sq-fix4`]** Wrap `sparq-fedplan::select_sources` to consume the
   privacy/authorisation descriptor and prune non-participating / unauthorised sources; surface
   the candidate set. *(P2, depends Phase 1)*
3. **Phase 3 — disclosed/hidden routing pass.** **[LANDED — `sq-i1wh2`]** The policy-parameterised greedy partition (§4.3
   pass 2) emitting `Vec<OperatorRouting>` + a `FederatedQuery`; default mode = disclose
   global-IRI operands, strict mode = honour per-source "hide even public" marks. Differential
   test: the produced routing reproduces the hand-written four-flatmates routing exactly. *(P2,
   depends Phase 2)*
4. **Phase 4 — leakage-envelope assembly + dual ratification.** **[LANDED — `sq-pwr.2`]** Compute the declared leakage
   envelope; each holder's fail-closed private-column re-check; verifier-side envelope acceptance
   policy. Negative tests: a plan that tries to disclose a private term is rejected by the
   holder; an over-leaking envelope is rejected by the verifier. *(P2, depends Phase 3)*
5. **Phase 5 — untrusted-plan soundness re-validation (gated).** **[CANONICALISATION HALF LANDED
   — `sq-1fo4`, `crates/sparq-fedplan-mpc/src/binding.rs`; the SOUNDNESS HALF REMAINS BLOCKED]**
   Bind the produced plan to the
   eventual collaborative proof so a dropped/mis-routed source yields a verification failure, not
   a forged accept. **BLOCKED** on `proof.rs` ceasing to be a stub AND the external audit
   (sq-qhy4) + collaborative coZK re-audit (sq-9hrn) — do NOT present as sound until then. *(P3,
   depends Phase 4 + the audit gates)*

   **What `sq-1fo4` landed, and what it deliberately did not.** The phase splits in two, and only
   one half depends on the gates. The *canonicalisation* half — reducing the produced plan to a
   single canonical word a proof could later commit to — depends on nothing that is blocked, and
   is the move `sq-34ml` already made for the M4-v1 freshness binding ("scoping + layout only,
   still hard-gated on the ZK foundation"). So it landed: `commit_plan` folds the Phase-2
   per-pattern source assignment, the Phase-3 disclosed/hidden route and the Phase-4 declared
   participation set into a domain-separated `PlanCommitment` whose `plan_digest()` is a
   `sparq_mpc::FieldWord` — the estate's public-input word — and `revalidate_plan` fails closed
   against an independently recomputed commitment, naming the **dropped source** or **re-routed
   operator** rather than reporting a bare digest mismatch. It binds plan *content*, not the
   planner's enumeration order, and excludes `estimated_cardinality` (a cost estimate, not plan
   semantics).

   **This does NOT discharge C-A and is not presented as doing so.** The out-of-circuit check is a
   *canonicality + transcript-integrity* measure and defends against nothing a **malicious**
   planner does: the planner computes the plan *and* its commitment, so one that drops a source
   simply commits to the plan it actually ran and the check passes. What it catches is *accidental*
   divergence — a plan mutated between ratification and execution, a planner/verifier disagreement,
   a truncating transport. The half that *would* discharge C-A is `bind_plan_to_proof`, which
   carries its real signature and returns `SeamError::Deferred` naming every gate: there is no
   collaborative proof to bind to, and sq-qhy4 / sq-9hrn are pending. **Nothing in this phase may
   be presented as sound until that step lands and the audits clear.**
6. **Phase 6 — FedUP-style result-aware source-combination pruning.** **[LANDED — `sq-pwr.3`
   (Rule C1) + `sq-xkrt` (Rule C2, the quotient-summary rule); the value-overlap prune remains
   deliberately DECLINED as not recall-safely expressible from the public summary — see the
   crate README]** Add provenance/quotient-
   summary pruning to Phase 2 to cut the source-combination blow-up before MPC (the highest-
   leverage pre-MPC cost win). **Rule C1** (`sq-pwr.3`) collapses the whole combination space
   when any conjunct has no candidate source (`∅ ⋈ R = ∅`) — sound, but it only fires on an
   already-dead query. **Rule C2** (`sq-xkrt`) is the rule that fires on the **live** path, and
   is the actual FedUP *provenance-over-quotient-summaries* lever: a source's served
   characteristic sets (`scs:`) partition its subjects by their **exact** predicate set, so if
   two patterns share a **subject**-position join variable and no characteristic set of a source
   carries both their predicates, no subject of that source satisfies both conjuncts — that
   same-source pairing, and every source-combination containing it, is provably dead, while the
   source stays a valid candidate for each pattern *individually*. Recall-safety rests on a
   **completeness guard** (`Σ_{C ∋ p} subjects == void:distinctSubjects` over distinct set
   keys): served characteristic sets are routinely truncated, and truncation strictly lowers
   that sum, so a truncated / unknown / non-partition summary **declines** rather than
   over-prunes. *(P3, depends Phase 2)*
7. **Phase 7 — B7 authorisation hook (WAC/ACP-aware source skipping).** **[LANDED OUTSIDE
   THIS CRATE — `sq-lzvl`, `crates/sparq-solid/src/source_auth.rs`, opt-in `source-auth`
   feature]** §9 Q4 is answered as the second option: the decision lives with `sparq-solid`,
   which already owns WAC/ACP evaluation, and this seam only *references* it. `sparq-solid`
   gains `SourceDescriptor` (a source as the named graphs it serves) and
   `PodStore::authorize_source`/`authorize_sources` → `SourceAuthorization`, which
   participates iff the session may read ≥1 declared graph and carries the **narrowed**
   authorised subset. **No dependency edge was created in either direction** — the join is a
   plain `bool` an integrator that opts into both crates feeds to Phase 1's reserved
   `participates` field, so nothing in `sparq-fedplan-mpc` changed and Phase 2 still reads
   that field as a bare participation flag. The decision is **plan-time over a local auth
   view**: it enforces nothing at a remote source, authenticates no participant, and makes no
   MPC/privacy/ZK claim. *(P3, depends Phase 1)*

---

## 9. Open questions for the maintainer

*Q1 and Q2 were the two choices [#755](https://github.com/sparq-org/sparq/issues/755) held the build
on; both are **RESOLVED** (full rationale + code evidence in the decision log at the top of this
record). **Q3 is still OPEN; Q4 is RESOLVED by `sq-lzvl` — see below.***

1. ~~**Crate vs feature granularity.**~~ **RESOLVED (#755): the STANDALONE CRATE.**
   `sparq-fedplan-mpc` is its own `publish = false` workspace member, additionally gated behind
   an `fedplan-mpc` feature that is OFF by default, so the default build compiles an empty crate
   and `sparq-fedplan` / `sparq-mpc` stay decoupled from each other. The original question — a
   standalone member vs an `mpc` feature *inside* `sparq-fedplan` — was answered as §5/§7
   recommended, the coupling cost being the deciding factor.
2. ~~**Default disclosure posture.**~~ **RESOLVED (#755): DEFAULT-DENY**, in two layers, because
   the question as posed (disclose-all-global-IRIs *vs* default-deny) turned out not to be a
   single knob. **Predicates are default-deny with no exception** — private unless the holding
   source explicitly marks them public, and an operator discloses only if *every* operand is
   disclosable for *every* contributing source. **Global IRIs keep the convention-#6 shortcut in
   the default policy** (`RoutingPolicy::Default`, the cheap demo-matching route) and lose it
   under `RoutingPolicy::Strict`, the §5 "hide even a public term" knob. Either way the leakage
   envelope declares the result and Phase 4's dual ratification lets a holder veto it.
3. **OPEN — where does the untrusted-plan re-validation (Phase 5) actually bind?** It depends
   entirely on the shape the collaborative proof takes when `proof.rs` is built — so Phase 5 is
   genuinely blocked on a design that does not yet exist, and may need its own record then. Is
   deferring it still the right call? (Phases 1–4 + 6 have since landed *without* the binding
   sketched, so the question is now whether to sketch it before Phase 5 starts, and whether any
   of the landed plumbing needs to change to accommodate it. It also stays gated on `sq-qhy4` /
   `sq-9hrn` regardless of the answer.)

   **Partial answer from `sq-1fo4` — the *what* is settled, the *where* is not.** Building the
   canonicalisation half surfaced that the two sub-questions are separable. **What** must be bound
   is now fixed and implemented: the per-pattern source assignment (a *dropped* source perturbs
   it), the per-operator disclosed/hidden route (a *mis-routed* operator perturbs it), and the
   declared participation set. Binding the per-pattern assignment — not merely the flat
   participation set — turned out to be load-bearing: moving a source from one pattern to another
   leaves the participation set identical while changing the plan's meaning entirely. **No landed
   plumbing needed to change** to accommodate this; `commit_plan` reads the existing Phase-2/3/4
   outputs unmodified, which is itself a useful negative result.

   **Where** it attaches remains genuinely open and is the part still waiting on the proof's shape.
   `bind_plan_to_proof`'s signature *proposes* `sparq_mpc::FederatedStatement` — its M4-v1
   public-input layout already carries the analogous `query_digest` / `key_set_digest` words, so a
   plan word would sit naturally beside them — but that is a proposal for the maintainer, not a
   decision, and taking it would mean extending a byte layout `sq-34ml` deliberately pinned. **That
   layout was NOT touched.** The maintainer's call: accept the `FederatedStatement` attachment
   point (and schedule the layout extension), or defer the *where* to its own record once the proof
   exists.
4. ~~**OPEN — B7 scope.**~~ **RESOLVED (`sq-lzvl`, issue #3296): it belongs with `sparq-solid`
   and only REFERENCES this seam.** The question was whether access-control-aware source
   skipping is in-scope for this track at all, or belongs with the WAC/ACP crate. Answered as
   the latter, on the same coupling-cost ground that decided Q1: the policy lives where the
   evaluation engine already is, so this track gains no access-control code and `sparq-solid`
   gains no federation/MPC code. Evidence: `crates/sparq-solid/src/source_auth.rs`, behind
   the OFF-by-default `source-auth` feature.

   **What that buys, and what it deliberately is not.** `sparq-solid` decides participation
   from its materialized `<urn:sparq:auth>` view — a source declares the named graphs it
   serves, participates iff the session may read at least one of them, and is **narrowed** to
   that authorised subset (a decision can shrink what a source is asked for, never widen it).
   Fail-closed in both directions: an undeclared served-graph set and a no-readable-graph set
   are both skips. The hand-off to this seam is a plain `bool` fed to Phase 1's reserved
   `participates` field, so **no dependency edge exists either way** and nothing in
   `sparq-fedplan-mpc` had to change — the same useful negative result Q3 recorded.

   It is a **plan-time** decision over a **local** auth view. It enforces nothing at a remote
   source and authenticates no participant (the WebID/client/issuer stay caller-asserted, as
   throughout `sparq-solid`), so a source that is asked for a graph must still enforce its own
   access control; skipping is a confidentiality-and-cost measure, not a completeness
   guarantee. It makes no MPC, privacy or zero-knowledge claim, and the audit posture above is
   unchanged by it.
