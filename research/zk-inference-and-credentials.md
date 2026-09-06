<!-- [OPUS-4.8] Design-for-review authored by Opus 4.8 (Fable 5 unavailable) — re-review when Fable returns. -->
# ZK inference and credentials: prove-query-under-entailment, query-over-N3-rules, delegation-binding, and unlinkable presentation

Design record for the **opt-in ZK feature family** (epic `sq-rsd3v`) that composes
the already-landed `sparq-zk` / `sparq-zk-compose` gadget estate with
`sparq-reason`'s off-circuit derivation oracle to close the four documented open
problems of the trust-graph authorisation design
(`research/solid-trust-graph-authz-design.md`, epic `sq-pfae`).

This is a **design-for-review record**, matching the rigour of
`research/zk-soundness-audit.md`, `research/zkp-query-proofs-plan.md`, and
`research/zk-holder-pop-design.md`. **NO production code is shipped by this
document.** Every feature it specifies is opt-in, fail-closed, and inherits the
estate's standing caveat (§1).

> 🤖 SPARQ agent (Opus 4.8). This record makes **no production security or privacy
> claim**. The composition verifier it builds on is the maintainer's **own**
> position not-yet-sound (§1), and external accredited-cryptographer sign-off is
> pending (`sq-qhy4`). Read §2 (proven-vs-assumed boundary) before §3–§6.

---

## 0. The maintainer's raw asks (verbatim)

Captured verbatim so the scope cannot drift in review:

> 1. **ZK proof that a SPARQL query holds under RDFS/OWL inference** — "Some
>    datasets only make sense after RDFS / OWL reasoning. Prove a query result
>    holds *over the entailed graph* without disclosing the base."
> 2. **Query-over-N3-rules** — "The same, but where the dataset ships its own
>    `{premise} => {conclusion}` N3 rules. Prove a SPARQL result over the
>    N3-materialised closure."
> 3. **Delegation-binding challenges** — "When an agent acts on behalf of a
>    delegator, prove the invoker *is* the terminal delegate of the chain,
>    key-proven, and the proof can't be replayed."
> 4. **Unlinkable presentation** — "A credential presentation that is unlinkable
>    across uses and unlinkable to issuance — ZKAPs / Privacy-Pass grade — not
>    just hidden-issuer."

These map one-to-one onto §3 (inference), §4 (N3), §5 (delegation), §6
(unlinkability), and §7 wires all four into the trust-graph admission gate.

---

## 1. Standing caveat: the estate is the maintainer's OWN not-yet-sound position

The single most important framing correction this record makes (per the SOUNDNESS
adversarial lens): **do not say "unaudited".** "Unaudited" reads as *"we believe it
sound; an external party has not confirmed."* The codebase's own comments are
stronger than that. Across `manifest.rs`, `build.rs`, and `verifier.rs` the
composition verifier is internally labelled, verbatim:

> *"NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2)"*

(see `crates/sparq-zk-compose/src/manifest.rs:619,1067,1089,1236,1285`;
`crates/sparq-zk-compose/src/build.rs:73,767`;
`crates/sparq-zk-compose/src/verifier.rs:340,484,3151`). That is the
**maintainer's own** position: soundness of the composition seam is *not
established*, not merely *not externally confirmed*.

Two distinct statuses must therefore never be conflated:

- **The binding LAYER is internally re-audited "sound as landed"** (`sq-gbp4`) —
  the per-gate checks (issuer-signature byte-binding, public-input
  reconstruction, canonical-vk gate) were re-audited and are sound for what they
  individually assert.
- **The composition verifier as a WHOLE is self-declared not-yet-sound**
  (`sq-qhy4` / `sq-9hrn`; remediation epic `sq-1s2`) — the *seam* that glues the
  per-circuit proofs into one admission decision has not been argued sound, and
  is awaiting external sign-off.

**No feature in this record may inherit a "sound" or "proven" label that the
underlying composition seam does not have.** Where this document says "proven
today" it means *exactly* "wired and internally re-audited as sound-as-landed for
that one binding check" — never "the ZK system proves this property end-to-end."
The `scripts/check-privacy-claims.sh` gate exists precisely to keep this honesty
in the outward-facing surface (`research/` is path-excluded so it can name the
properties to argue about them — this document does so deliberately).

**No production security claim is made anywhere in this record.**

---

## 2. Proven-vs-assumed soundness boundary

This is the load-bearing table. Each feature below cites which row it stands on.

| # | Property | Status TODAY | Where (file:line) |
|---|----------|--------------|-------------------|
| P1 | Issuer-signature byte-binding (audit-#3) | **sound as landed** (`sq-gbp4`) | `verifier.rs:1700` `bind_issuer_attestations` |
| P2 | Public-input reconstruction + canonical-vk gate (audit-#1/#2) | **sound as landed** (`sq-gbp4`) | `verifier.rs` (per-circuit derive + byte-equal) |
| P3 | Nonce / challenge replay-binding (audit-#4) | **sound as landed** — verifier-side field-0 byte-compare | `verifier.rs`; circuits do `let _ = challenge;` |
| P4 | Entailment-regime gate (`bind_entailment`) | **structural re-check, sound-as-landed; NOT ZK, NOT in-circuit, antecedents DISCLOSED** | `verifier.rs:4401`; `derivation.rs` |
| P5 | Hidden-issuer set-membership (`pk_i ∈ K`) | **wired, NOT-yet-sound** (`sq-qhy4`) | `issuer.rs`; `issuer.nr`; `verifier.rs:3002` |
| P6 | Hidden-holder PoK (`hpk = hsk·G`, digest match) | **wired, NOT-yet-sound** (`sq-qhy4`) | `holder.nr`; `verifier.rs:3158` |
| A1 | In-circuit ZK-over-hidden-antecedent derivation (`sq-rsd3v.2`) | **relation landed, NOT-yet-sound (`sq-qhy4`); no zk-trace mapper, no compiled member, no dispatch** (§3.3 Status) | `derivation.nr` |
| A2 | Witnessed-rule-shape N3 derivation step (`sq-rsd3v.3`) | **relation + host mirror landed; NOT-yet-sound (`sq-qhy4`), NOT `bb`-measured, NOT manifest-dispatched, no `ProofStep`→slot mapper** (§4.4) | `n3.nr`; `n3.rs` |
| A3 | Single-use nullifier (computed in-circuit) (`sq-rsd3v.1`) | **ABSENT — new circuit-soundness surface** | — (§6 deliverable) |
| A4 | Delegation chain-carry + per-request key-proof (`sq-rsd3v.4`) | **ZERO substrate today** | — (§5 deliverable) |
| A5 | `sparq-trust` admission gate (`admit`) (`sq-rsd3v.8`) | **PoC only** (`sq-pfae.10`, PR #966); not the generalised gate | `crates/sparq-trust` (PoC) |
| A6 | `owl:sameAs` in-circuit canonicalisation + representative-membership (`sq-rsd3v.6`) | **relation + host mirror landed; NOT-yet-sound (`sq-qhy4`), NOT `bb`-measured, NOT manifest-dispatched** | `sameas.nr`; `sameas.rs` (§3.5.1) |
| X1 | Completeness-under-entailment (saturation proof) (`sq-rsd3v.7`) | **UNBUILT anywhere; NOT claimed** | — |

**The crux honesty correction (per the SOUNDNESS lens):** rows P4 vs A1. What
ships today (`bind_entailment`) is a **host-side, verifier-side structural
re-check over DISCLOSED scan rows** — an antecedent must equal a disclosed scan
row or an earlier derived triple (`derivation.rs:18-29`,
`verifier.rs:4470-4475`). It buys **verifiable correctness with ZERO privacy over
antecedents** and **is not a zero-knowledge proof.** The actual ZK soundness
deliverable — an in-circuit `derivation_step` member proving *hidden*
committed-graph membership chained by encoding-equality — **does not exist** in
`zk/compose/compose_core/src/` (verified: that directory ships `scan.nr`,
`filter_int.nr`, `filter_signed.nr`, `filter_float.nr`, `join.nr`, `issuer.nr`,
`holder.nr`, `revoke.nr`, `hashes.nr` — and no derivation/entailment member). So
the thesis "three of the four features are not greenfield" is true only of the
*host-side scaffolding and witness source*; the **in-circuit relation is brand-new
Noir**, the hard and unbuilt part.

---

## 3. Feature 1 — prove a SPARQL query holds under RDFS/OWL inference (`sq-rsd3v.2`)

### 3.1 Two obligations the maintainer must never conflate

- **SOUNDNESS-of-derivation:** every disclosed answer is genuinely entailed by
  the committed, signed base under the named regime.
- **COMPLETENESS-under-entailment:** no entailed answer is missing — *proving a
  negative over a fixpoint* (the `I3` property of `zkp-query-proofs-plan.md §5`).

These have radically different cost. Soundness needs only a witness for the
**specific** derivations that produced disclosed rows (a bounded proof DAG).
Completeness needs reasoning about the **whole** closure (row X1, `sq-rsd3v.7`).
**This design targets SOUNDNESS first; completeness-under-entailment is explicitly
deferred and NOT claimed** — and that deferral should be stated loudly.

### 3.2 What ships today (the honest starting point — row P4, not row A1)

`crates/sparq-zk-compose/src/derivation.rs` ships `DerivationStep` +
`EntailmentRule` for the six fixed-shape RDFS rules of the v1 scope (§3.5) —
`rdfs9` (subClassOf-into-type transitivity), `rdfs7` (subPropertyOf), and (added
under `sq-rsd3v.2`) `rdfs2` (domain), `rdfs3` (range), `rdfs5` (subPropertyOf
transitivity), `rdfs11` (subClassOf transitivity) — each with a **fixed
antecedent/consequent shape** re-checked by `DerivationStep::is_well_formed`.
`verifier.rs:4401` `bind_entailment` enforces the manifest's `entailmentRegime`
**end-to-end and fail-closed**:

- the regime must be in the relying party's `EntailmentPolicy`
  (`verifier.rs:527`, default = `Simple`-only, `verifier.rs:541`) — else
  `CheckError::UnacceptedRegime`;
- `Simple` forbids steps; non-`Simple` **requires** them — else
  `MissingDerivationSteps` (`verifier.rs:4425`);
- each step is structurally re-checked as a valid rule instance;
- **every antecedent must be GROUNDED** — equal to an earlier step's derived
  triple or a **DISCLOSED** scan row — else `UngroundedDerivationAntecedent`
  (`verifier.rs:4471`).

This is sound-as-landed for the disclosed-base fragment and fail-closed otherwise.
**It is a structural re-check, not ZK, and discloses the antecedents.** That is
row P4, not a ZK property.

### 3.3 The deliverable (row A1, GREENFIELD): ZK-over-hidden-antecedents

The real feature replaces *"antecedent == disclosed scan row"* with *"antecedent
has a HIDDEN committed-graph membership proof."* Concretely, a **new in-circuit
`derivation_step` member** (brand-new Noir, not a tweak to the host-side skeleton):

1. one in-circuit `derivation_step` relation per `ProofNode`, with the rule's
   fixed variable-sharing equalities over term encodings (a few field equalities +
   array lookups per node — order ~10² gates/node by judgement, consistent with
   the measured `filter_eq=132`-gate / `bgp_match=1410`-gate anchors of
   `zkp-query-proofs-plan.md §4B`; **to be confirmed by `bb gates` before any cost
   claim**, per the `noir-optimisation` skill);
2. **premise/conclusion term-encodings chained by encoding-equality** for internal
   DAG nodes;
3. **DAG-leaf antecedents anchored to membership-against-`commitments[g]`** — i.e.
   the antecedent triple is proved to be in the signed base graph via the
   set-membership accumulator already used by `scan.nr` — *not* to a disclosed
   row. This is the privacy upgrade and the soundness anchor.

A derivation DAG proves SOUNDNESS but **not** completeness, and **not** that
leaves are themselves true unless each leaf is anchored — hence (3) is mandatory.

**Status (`sq-rsd3v.2`):** the in-circuit RELATION landed as
`zk/compose/compose_core/src/derivation.nr` (`derivation_check<K,N,M>`) —
per-node rule variable-sharing equalities (1), premise-chaining by
encoding-equality for internal DAG nodes (2), and DAG-leaf anchoring via the
`scan.nr` whole-graph-in-circuit membership accumulator against `commitments[g]`
(3), with a forge-and-verify Noir test (an ungrounded/forged antecedent fails
closed). It is **research-grade, NOT externally audited (`sq-qhy4`), and NOT yet
`bb`-compiled** in this tree — **no cost claim is made** (the per-node `~10²`
figure remains a judgement to be confirmed by `bb gates`). Still to land: the
off-circuit **zk-trace mapper** (`sparq-reason explain` `ProofTree` → witness
slot indices), a compiled **bin-package monomorphisation**, and the verifier
dispatch that binds a `derivation_check` proof into `verify_manifest`.

**The C(G)-membership story and its cost (the SOUNDNESS-lens correction — read
carefully).** Step (3) must NOT be misread as "reuse the hidden-issuer / holder-set
Poseidon2-Merkle gadget verbatim to prove a hidden antecedent is a *Merkle member of*
`C(G)`." It cannot, because of a concrete primitive mismatch: `C(G)` is committed as a
**flat Poseidon2 sponge** over the leaf sequence (`crates/sparq-zk/src/commit.rs`: "`C(G)`
= one Poseidon2 sponge over the leaf sequence"; "the per-graph Merkle fallback … is a
later deliverable"), so it has **no per-leaf authentication paths** to prove membership
against. The issuer/holder Merkle gadget (`issuer.nr key_set_membership`) proves
membership in a **separate** relying-party-owned key/holder tree (`key_set_root`), **not**
in `C(G)`. The one **sound** way to bind an *undisclosed* antecedent to `C(G)` on the
current substrate is therefore the **whole-graph-in-circuit scan-equality** path
`scan.nr` already uses ("the whole graph is in-circuit, so membership is equality against
witnessed slots … no Merkle machinery"). The cost consequence must be stated explicitly,
not glossed:

> - **Disclosed-antecedent inference** (row P4) = cheap, proof-tree-sized, already
>   shipped, sound-by-recheck — with **zero** antecedent privacy.
> - **Hidden-antecedent inference** (row A1) via the `scan.nr` accumulator = cost
>   proportional to **|C(G)|** (the whole graph is in-circuit), **not** proof-tree-sized.
>   The per-node `~10²`-gate figure above is for the rule-shape equalities only; the
>   leaf-grounding term scales with the committed graph and dominates.

To make the *hidden* path proof-tree-sized (one Poseidon2-Merkle path per hidden
antecedent), the commitment to `C(G)` must itself be a **per-graph Merkle tree with
authentication paths** — the deferred `commit.rs` "shape-2". That is an **explicit
PREREQUISITE deliverable** (its own bead under the epic); **every claim of
proof-tree-sized hidden grounding is conditional on it landing.** Until then, A1 is sound
but `|C(G)|`-cost.

**Schema-as-public is an ASSUMED downgrade, never silently used.** A tempting cost cut is
to treat the ontology schema (`subClassOf`/`subPropertyOf`/`domain`/`range`) as PUBLIC
inputs from a trusted/`explain`-checked reasoner and spend ZK gates only on instance
firings. This is **not** an optimisation — it is a **soundness assumption** (PROVEN =
false / ASSUMED): the schema antecedents are then *trusted*, not proven in `C(G)`, and the
residual attack is a prover supplying a schema edge the credential does not contain. This
design therefore grounds schema antecedents the same way as any other (above); a
schema-as-public dial is offered only as an opt-in, `EntailmentRegime`-recorded
**documented assumption**, never the default and never silent.

### 3.4 Witness source (reused, not reimplemented)

`sparq-reason`'s `explain` feature already emits the exact shape: `why()` returns
a `ProofTree` — a flat `Vec<ProofNode { conclusion, rule, premises }>` in
**premises-before-conclusion** order, a deduplicated DAG, capped by
`ExplainOpts { max_depth: 128, max_nodes: 65536 }`
(`crates/sparq-reason/src/explain.rs`; `incremental_explain.rs:114`;
re-exported `lib.rs:14`). A new **zk-trace** step maps `ProofNode` indices to
witness slot indices, one `ProofTree` per derived disclosed row. This is the
off-circuit witness generator; **the in-circuit relation that consumes it is the
greenfield part.**

### 3.5 Rule scope (phased, fail-closed outside it)

- **v1 (RDFS-first):** extend `EntailmentRule` from the shipped two to `rdfs2`
  (domain), `rdfs3` (range), `rdfs5`/`rdfs11` (transitivity) — all the same fixed
  variable-sharing shape.
- **v2 (OWL-RL-minus-sameAs):** the OWL-RL rules whose antecedents are
  fixed-shape Datalog over ids.
- **`owl:sameAs` is gated SEPARATELY behind its own bead (`sq-rsd3v.6`)** because
  encoding-equality re-checks are **UNSOUND under equality reasoning**: `sameAs`
  needs in-circuit union-find `canon(s) == canon(s')` with representative-
  membership witnesses (the cost cliff). It must NOT silently ride the v1/v2 path.
  See §3.5.1 for the landed gadget and its soundness argument.

### 3.5.1 `owl:sameAs`: the canonicalisation gadget and its soundness argument (`sq-rsd3v.6`)

**Why the v1/v2 path cannot absorb it.** Every rule in §3.5 is re-checked — host-side
in `derivation.rs`, in-circuit in `derivation.nr` — by **term-encoding equality**, and
an encoding equality is a term **identity**. That proxy is exact for RDFS and for the
OWL-RL rules whose antecedents are fixed-shape Datalog over ids. It breaks under
equality reasoning, because `owl:sameAs` **quotients the term universe**: under
OWL-RL's `eq-ref` / `eq-sym` / `eq-trans` / `eq-rep-{s,p,o}` two syntactically distinct
terms may denote the same thing, so term identity is strictly **finer** than the
entailment relation. Bolting `eq-rep-*` onto the fixed-shape path gives one of two bad
outcomes: (a) it never fires, because encoding equality does not cross a `sameAs` link;
or (b) — if the equality test is relaxed to "equal up to some claimed `sameAs`" — the
claimed equalities become **trusted rather than proved**, which is exactly the
PROVEN-vs-ASSUMED downgrade §3.3 refuses for the schema. Hence a separate member.

**The gadget (landed).** `zk/compose/compose_core/src/sameas.nr` — `canon_of`,
`canon_table_check`, `sameas_entails_row` — with the host mirror + witness builder in
`crates/sparq-zk-compose/src/sameas.rs` (`CanonTable::{from_committed_sameas, canon,
check, entails_row}`). The prover witnesses a **spanning forest** of the committed
`owl:sameAs` edge graph as a `(mem, rep, parent)` table; the circuit checks it and then
runs the ordinary encoding-equality test **over representatives**.

**The soundness argument.** Write `~` for the equivalence relation on terms generated
by `{(a, b) : (a, owl:sameAs, b) ∈ C(G)}` — precisely what `eq-ref`/`eq-sym`/`eq-trans`
derive. The relation's constraints are: (3a) a non-root's `parent[u] < u` strictly;
(3b) a root represents itself and a non-root shares its parent's representative;
(3c) every tree edge is witnessed by a committed `(mem[u], sameAs, mem[parent[u]])` or
`(mem[parent[u]], sameAs, mem[u])` triple; (4) active members are distinct.

- **L1 (no over-merge).** Every active entry has `mem[u] ~ rep[u]`. Induction on `u`,
  well-founded by (3a). Root: reflexivity via (3b). Non-root: (3c) gives
  `mem[u] ~ mem[p]` (either orientation, `eq-sym`), the IH gives `mem[p] ~ rep[p]`, and
  (3b) gives `rep[u] = rep[p]`; transitivity closes it.
- **L2 (`canon` is class-preserving).** `canon(t) ~ t` for every term: a term with no
  entry is its own representative (`eq-ref`); otherwise (4) makes the entry unique and
  L1 applies.
- **L3 (idempotence).** `canon(canon(t)) = canon(t)`, because `canon(t)` is a root's
  member and, by (4), that root is THE entry for it. This is the lemma (4) exists for;
  L1/L2 do not need it.
- **THEOREM — *encoding-equality is preserved under the canonical representative*
  (the bead's named deliverable).** `canon(x) = canon(y)` implies `x ~ y`:
  `x ~ canon(x) = canon(y) ~ y` by L2 plus symmetry and transitivity. So encoding
  equality **over canonical representatives** is a sound test for "the same thing under
  equality reasoning" — the property the raw test lacks, and the property any future
  lifting of the §3.5 fixed-shape rules over `~` would need.
- **COROLLARY — representative-membership.** If a committed triple `(s₀,p₀,o₀)`
  satisfies `canon(s₀)=canon(s)`, `canon(p₀)=canon(p)`, `canon(o₀)=canon(o)`, then
  `C(G) ⊨ (s,p,o)` under OWL-RL: by the THEOREM `s₀ ~ s`, `p₀ ~ p`, `o₀ ~ o`, and
  `eq-rep-s`/`eq-rep-p`/`eq-rep-o` rewrite the committed triple into the disclosed row.
  Every `sameAs` fact those rules consume is committed (3c) or an `eq-*` consequence of
  committed triples, so nothing is assumed. This is `sameas_entails_row`.

**What is NOT claimed.** The converse of the THEOREM is false **by construction**: a
prover may omit entries or edges, so `canon` is only a **refinement** of `~`.
Under-merging costs provability, never soundness — a row the gadget cannot show
entailed is **not** thereby shown non-entailed (the same existence-only posture as
`path.nr`). Fixpoint saturation remains §3.7 / `sq-rsd3v.7`, unbuilt. Blank-node
encodings are salt-dependent, so a `sameAs` edge touching a bnode is per-graph; the
estate's cross-graph non-bnode (Q6) obligation is **unchanged and not discharged here**.
The gadget is research-grade and **NOT externally audited** (`sq-qhy4`).

**Cost — measured before claimed, and it has NOT been measured.** The dominant terms
are `U·K·N` (each table edge looked up against the whole in-circuit graph — `C(G)` is a
flat Poseidon2 sponge with no per-leaf authentication paths, so the `scan.nr`
whole-graph discipline applies until the deferred `commit.rs` "shape-2" per-graph Merkle
tree lands), `U²` (distinct members), and `K·N + U` for the row anchor. Naming the base
triple by a witnessed private `(src_graph, src_slot)` — rather than canonicalising every
slot of every graph — is what keeps the row anchor at `K·N + U` instead of `K·N·U`.
**No gate figure is stated anywhere**: `bb gates` over a compiled monomorphisation MUST
be run first, and it has not been.

**Fail-closed: `owl:sameAs` cannot ride the v1/v2 path.** The refusal is enforced, not
merely documented. `DerivationStep::mentions_equality_predicate` rejects any step whose
`owl:sameAs` encoding stands in a predicate slot of an antecedent or of the derived
triple, and `verifier::bind_entailment` returns `CheckError::EqualityReasoningUnsupported`
for it **before** the shape check. That ordering matters, because the dangerous shapes
are shape-VALID: `rdfs7` with `p1 = owl:sameAs` **consumes** an equality
(`(sameAs subPropertyOf q), (x sameAs y) ⊢ (x q y)`) and with `p2 = owl:sameAs`
**introduces** one. Both are legitimate *RDFS* entailments, so refusing them costs only
provability — the conservative direction — while making a silent ride impossible.

**Still to land (follow-ups, not claimed here).** A compiled `sameas_k{K}_n{N}_u{U}` bin
package and its `bb gates` measurement; the `CircuitId` / `ProofInputs` / `verify_manifest`
dispatch that would bind such a proof into a manifest; and composition with
`derivation_check` (running the §3.5 rule shapes over canonical representatives), whose
statement is licensed by the THEOREM above but whose *cost* is the open question.

### 3.6 Rejected approaches (with reasons)

- **(a) commit-the-closure** (commit `C(closure(G))`, prove `Q` over it): the
  query-over-closure half reuses all `scan`/`filter`/`join` circuits **unchanged**
  — but it is **rejected as the primary credential path** because the issuer
  signed the **base**, not the holder's closure (`I1` of the plan §5). You would
  also have to prove the closure was *correctly and completely* derived — exactly
  the expensive completeness-over-fixpoint problem; you have not avoided it, only
  moved it. **It IS the right shape for an explicit, honestly-labelled
  trusted-materialiser SERVER mode**: a trusted party signs the closure commitment
  and the holder proves `Q` over it with **zero in-circuit reasoning** — cheapest
  deployable shape, distinct trust model ("entailment trusted to the materialiser
  signature", NOT in-circuit-proven). This mode must be labelled as such.
- **(c) folding (Nova/HyperNova/Protostar) / zkVM re-execution:** the only
  approach that natively gives completeness, but **measured-out at credential
  scale** — folding's edge needs long iteration streams (≥10⁴ steps); RDFS/OWL-RL
  credential closures are a handful of rounds over ~10² triples, so per-step
  recursion overhead dominates; zkVM is the baseline-to-beat, not a build target.
  Re-enters only on a documented trigger (huge closure + a verifier demanding full
  completeness — see `sq-rsd3v.7`).

### 3.7 Completeness-under-entailment (X1, `sq-rsd3v.7`) — explicitly NOT built

Scoped to credential scale ONLY, and it needs BOTH halves: (i) an in-circuit
closure-sweep over the flat full graph, AND (ii) a **fixpoint-SATURATION proof**
(no rule fires producing a new triple). The saturation half is **UNBUILT anywhere
in sparq and is NOT claimed.** A relying party that needs "the answer set is
complete under entailment" cannot get it from this design today.

**The deferral is now ENFORCED, not prose-only** (`sq-rsd3v.7`, landed): the two
obligations cannot be conflated by reading an accept.
`EntailmentPolicy::require_completeness_under_entailment()`
(`crates/sparq-zk-compose/src/verifier.rs`) is how a relying party DECLARES it needs
"no entailed answer is missing", and the verifier's entailment gate then REFUSES —
fail-closed, before any other entailment check, with
`CheckError::CompletenessUnderEntailmentUnavailable` — every non-`Simple` manifest.
The refusal message names both missing halves from the single source of truth
`derivation::COMPLETENESS_UNDER_ENTAILMENT_UNBUILT`, so the honest scope cannot decay
into a doc only one side remembers. Precisely what the refusal asserts: *no accepted
proof under that policy rests on entailment whose completeness sparq cannot check* —
nothing more. Two limits are deliberate and documented in the API:

- a `Simple` manifest is NOT refused (no entailment closure for completeness to range
  over), but passing one is **not** a completeness assertion either — that rests on
  the `scan.nr` per-pattern sweep and the rest of the not-externally-audited verifier
  (`sq-qhy4`);
- an off-circuit materialised closure presented as `Simple` over the materialised
  graph (§3.6(a) trusted-materialiser mode) is INVISIBLE to the dial: there the regime
  field is honestly `Simple` and entailment is trusted to the materialiser's
  signature — a different trust model the relying party must evaluate itself.

When the RE-ENTRY TRIGGER of §3.6(c) fires (a documented huge-closure case PLUS a
verifier demanding full completeness; `research/zkp-performance-landscape.md` §5
trigger 4), the unconditional refusal is precisely what a real check replaces. Until
then it is the honest answer, and soundness-first (`sq-rsd3v.2`/`.3`) is the path.

---

## 4. Feature 2 — query over N3-rule datasets (`sq-rsd3v.3`)

### 4.1 Why N3 is the harder sibling

For RDFS/OWL-RL the rule set is a **fixed public table** (a small in-circuit table
per regime, exactly what `derivation_step`'s rule semantics encode). For N3 the
rule set is **dataset-supplied** `{premise} => {conclusion}` Horn-ish rules. Two
design moves follow:

1. **The rule author becomes an issuer.** Commit the N3 rule-graph into the
   **signed-input set `K`** exactly like a TBox: a derivation is only as sound as
   its rules, and the proof says **whose** rules. (Reuses the `sq-z9l`
   key-set-membership machinery: `issuer.rs key_set_root` / `key_set_root_sparse`,
   `issuer.nr`.)
2. **Each N3 rule-firing is a GENERALISED `derivation_step` whose rule shape is
   itself a witness** (premise/conclusion pattern term-encodings) rather than a
   fixed enum tag. The circuit checks the conclusion is the premise pattern under a
   **consistent variable substitution** (encoding-equality over shared variable
   slots) AND that each premise instance is grounded by a hidden committed-graph
   membership proof (§3.3 step 3) or an earlier conclusion.

So **N3 is Feature 1's `derivation_step` generalised from a fixed-rule-table to a
witnessed-rule-shape**, plus a builtin gadget whitelist (§4.3). It is row A2
(greenfield), ships **after** RDFS in-circuit (A1, `sq-rsd3v.2`) lands, and
inherits every soundness caveat of §3.

### 4.2 Witness source (already the right shape)

`sparq-reason`'s N3 engine already emits a derivation step per entailed triple
(rule index + supporting ground premises): `reason_n3_proof` / the CLI `--proof`,
exposed as `ProofStep` and `n3_proof_tree` (`crates/sparq-reason/src/lib.rs:18`;
`explain.rs:260` `n3_proof_tree`). It feeds the **same `ProofTree` path** as
RDFS/OWL.

### 4.3 The PROVABLE N3 subset (declared; fail-closed outside it)

**INCLUDED (v1):** safe, ground-deriving, builtin-free **forward** rules —
conjunctive `{p1 . p2 . …} => {c1 . …}` with universals (`?x`), where **every
conclusion variable is bound in the premise** (range-restriction / safety), plus a
small whitelisted builtin set provable as fixed gadgets:

- `math:` comparisons (`greaterThan` / `lessThan` / `notGreaterThan` /
  `notLessThan` / `equalTo`) — these **reuse the EXISTING `filter_int.nr` /
  `filter_signed.nr` / decimal comparison circuits verbatim**;
- `math:` arithmetic that maps to field ops (`sum` / `difference` / `product` as
  field add/mul with range-checks).

Path syntax (`!` / `^`) is fine — `sparq-reason` already desugars it into
fresh-variable join triples **before** the proof step.

**EXCLUDED and fail-closed (v1):** existentials-in-conclusion (no fresh-blank-node
minting in-circuit); scoped negation-as-failure (`log:notIncludes` /
`log:collectAllIn` — proving a negative needs the saturation machinery, deferred
with completeness, `sq-rsd3v.7`); `math:quotient` / `exponentiation` / floats;
`string:` builtins (no in-circuit UTF-8 reasoning); `list:` generators; `time:`
decomposition; `log:semantics` / `log:includes`; backward rules that don't reduce
to a forward closure; any rule whose closure is unbounded. A rule outside the
subset MUST be rejected, not silently approximated.

### 4.4 Status (`sq-rsd3v.3`, row A2)

The in-circuit RELATION landed as `zk/compose/compose_core/src/n3.nr`
(`n3_derivation_check`), with the host mirror + commitment builder at
`crates/sparq-zk-compose/src/n3.rs` (`N3Slot` / `N3Builtin` / `N3Premise` /
`N3Rule` / `N3RuleSet` / `N3SubsetError`). What it states:

- **§4.1 move 1 (whose rules).** `rules_root` is a PUBLIC input, recomputed
  in-circuit by folding every witnessed rule shape (`rule_leaf` → `commit_fold`),
  so a fired rule must be one the rule author committed. Signing that root is the
  existing `sq-z9l` `issuer.nr` machinery and is NOT restated by this member.
- **§4.1 move 2 (witnessed shape).** Each pattern slot is `(kind, konst, var)`;
  the conclusion must be the rule's conclusion pattern under a substitution that
  is CONSISTENT by construction (the substitution is an array indexed by variable
  id, so one variable cannot take two values in a firing).
- **Grounding.** Every JOIN premise atom chains to a strictly earlier node's
  conclusion; leaves anchor to committed-graph membership, so the antecedents
  stay hidden (the row-A1 privacy property, inherited).
- **Safety is enforced in-circuit, and it is load-bearing.** The substitution is
  a private witness, so an unbound conclusion variable would let the prover CHOOSE
  the derived triple. `rule_subset_check` runs a BINDING SCHEDULE over the
  witnessed shape (join atoms bind their variable slots; arithmetic builtins
  require bound inputs and bind their output; comparisons bind nothing) and
  refuses any rule whose conclusion uses an unbound variable.
- **Builtin whitelist.** The `math:` comparisons reuse `filter_signed`'s
  `signed_verdict` and its canonical-`xsd:integer` operand binding VERBATIM (both
  were extracted to `pub(crate)` for exactly this, with the FILTER lane's assert
  order preserved); `sum`/`difference`/`product` are exact field equations, sound
  because every magnitude is `< 2^64` against a `~2^254` modulus, and fail CLOSED
  (no canonical witness) when a true result leaves the representable range.
- **Fail-closed outside the subset, structurally.** `N3RuleSet::commit` runs
  `admit` BEFORE it folds, so an out-of-subset rule graph has **no root** and can
  never reach the circuit; the circuit independently re-checks the same conditions
  so a hand-rolled witness cannot bypass the host gate.

**What is NOT built (do not read the above as more than it says):** the
off-circuit witness generator that maps `sparq-reason`'s `reason_n3_proof` /
`n3_proof_tree` `ProofStep`s onto these slots; a compiled
`n3_k{K}_n{N}_r{R}_m{M}` bin package; its `bb gates` cost measurement (so **NO**
cost figure is claimed — repo policy forbids an unmeasured one); and any
`ProofManifest` / `CircuitId` / `verify_manifest` dispatch that would bind such a
proof. Nothing here makes the composition verifier sound, and the whole member
inherits §3's caveats — research-grade, **NOT externally audited (`sq-qhy4`)**.
As with `derivation.nr` (§3.3) and `sameas.nr` (§3.5.1), the Noir `#[test]`
accept/forge suite in `compose_core/src/tests.nr` is compiled by the zk lane but
**executed by no CI lane** (nothing runs `nargo test`), so it is a
maintainer-run suite, not a standing gate. The soundness obligation this member
discharges is derivation-SOUNDNESS only; COMPLETENESS under entailment remains
§3.7 / `sq-rsd3v.7`, unbuilt and not claimed.

---

## 5. Feature 3 — delegation-binding challenges (`sq-rsd3v.4`, closes `sq-l5og`)

### 5.1 Reframing (and the honest scope correction)

The requirement — invoker == terminal-delegate, key-proven, non-replayable — is
structurally the **confused-deputy defence** of the trust design §4.3: carry the
chain WITH the invocation, bind the invocation to the holder key (GNAP/DPoP-style),
keep authority ⊆ delegator. UCAN/ZCAP separate **delegation** (the signed chain
doc) from **invocation** (exercising it, key-bound to this request); `sq-l5og`'s
open problem is precisely that today the chain captures *existence* but not the
**per-request invoker binding**, so an admitted delegation becomes a standing graph
fact any session reaching that graph can ride (replay / re-delegation escalation).

**HONEST SCOPE (row A4):** there is **ZERO delegation substrate today** (verified:
no dpop/gnap/zcap/ucan/proof-of-possession across `sparq-solid` + `sparq-server`;
`Session.client` is a bare IRI with no key binding; `sparq-prov` records a single
`prov:wasAssociatedWith` agent per activity, **not** `prov:actedOnBehalfOf`; and
`crates/sparq-zk-compose/src/revocation.rs` is a *per-link* status-list primitive
with **no chain-revocation**). So this feature **builds** the chain-carry +
per-request key-proofing + nullifier from scratch.

### 5.2 The three sub-properties as ZK obligations

1. **INVOKER == TERMINAL-DELEGATE.** The delegation chain is a sequence of signed
   graphs (ZCAP-LD / UCAN docs), each admitted on the same trust rail as a
   credential. The terminal delegation's `delegate` key `d_n` is folded into the
   issuer-signed object exactly as holder-binding folds `hpk` — a **new domain
   tag** (`ZKSIG_DG`, alongside the `C1/C2/C3/…` family in
   `crates/sparq-zk/src/sig.rs:87+`). The invoker proves possession of the secret
   matching `d_n` via the **SAME holder PoK relation** (one Baby-JubJub scalar-mul
   `hsk·G` + a Poseidon2 digest equality — strictly cheaper than `schnorr_verify`,
   `holder.nr` docstring confirms one scalar-mul vs Schnorr's two).
   **Residual honesty:** the soundness of "invoker == terminal delegate" rests on
   the NEW `ZKSIG_DG` variant AND on the predecessor-delegate's signature being
   verified on the **same composition rail that is NOT-yet-sound** (§1). v1
   supplies the per-request **key-binding PREREQUISITE**, NOT the completed
   confused-deputy property.
2. **KEY-PROVEN.** Identical to holder PoK/PoP — the invoker's key is
   issuer-attested (the predecessor delegate signed `d_n` into the chain link),
   never a free allow-list entry; reuses `bind_holder_pok`
   (`verifier.rs:3158`) / the `holder_pk_digest` cross-check, over the
   chain-terminal key.
3. **NON-REPLAYABLE.** Bind the invocation to the verifier's fresh `challenge`
   reconstructed into public-input field 0 and byte-equalled (audit-#4, row P3),
   AND enforce single-use via the **NEW nullifier** (§6.3, `sq-rsd3v.1`) so an
   admitted delegation cannot become a standing graph fact (the exact `sq-l5og`
   vector).

### 5.3 Explicitly NOT delivered by v1

- Attenuation (authority ⊆ delegator) is a per-link scope-containment check —
  **host-side and monotone, NOT a ZK obligation v1.**
- Confused-deputy in the full object-capability sense is **not** prevented v1 (the
  per-request key binding is a prerequisite supplied, not a completed property).
- Deep-chain **incremental revocation**: a revoked mid-chain link stays live until
  the next re-materialisation epoch — an **unbounded stale-authority window**,
  unsized.
- Which delegator-permissions **snapshot** the attenuation intersects is **not**
  bound.

---

## 6. Feature 4 — unlinkable presentation (`sq-rsd3v.5`, closes `sq-wvne`)

### 6.1 Unlinkability needs THREE pieces, not one axis

ZKAPs / Privacy-Pass give *presentation unlinkability* (redemptions mutually
unlinkable + unlinkable to issuance, enforced by single-use / rate-limit). The
common error is treating this as one axis. It is three (trust design §5.3,
code-verified):

| Piece | Status | Where |
|-------|--------|-------|
| (i) Hidden-issuer set-membership (`pk_i ∈ K`) | **BUILT, NOT-yet-sound** (`sq-z9l`; `sq-qhy4`) | `issuer.rs` (`key_set_root`, `key_membership_witness`, `hidden_issuer_prover_toml`); `issuer.nr` (Poseidon2-Merkle `key_set_membership`); `verifier.rs:3002` `bind_hidden_issuer_attestations` |
| (ii) Hidden-holder PoK (proof-of-possession of `hsk`) | **BUILT + WIRED, NOT-yet-sound** (`sq-xqfg`/`sq-i1dt`; `sq-qhy4`) | `holder.nr` (`hpk = hsk·G`, `Poseidon2([ZKSIG_HK,hpk.x,hpk.y]) == holder_pk_digest`); `verifier.rs:3158` `bind_holder_pok` |
| (iii) Single-use NULLIFIER / double-spend primitive | **ABSENT** (row A3) | — (§6.3 deliverable, `sq-rsd3v.1`) |

So two of three are built (both NOT-yet-sound, §1); the genuinely-absent primitive
is the nullifier. The clear-WebID holder binding of the trust-graph PoC (`admit`,
`sq-pfae.10`) authenticates the requester WebID **in the clear**, so even with
(i)+(ii) the verifier learns *which* WebID requests — presentations are trivially
linkable by requester identity. A ZKAPs-equivalent presentation must **replace
clear-WebID holder binding with in-ZK holder-PoP + nullifier** (`sq-wvne`).

### 6.2 The whole composite is NOT-yet-sound

Pieces (i) and (ii) are wired but the composition verifier is self-declared
not-yet-sound (§1). This feature does **not** ship a sound anonymiser today; it
ships the missing primitive and the wiring, gated.

### 6.3 The nullifier — its OWN soundness obligation (NOT a "generalised store")

Per the SOUNDNESS lens, the nullifier (`sq-rsd3v.1`) must **not** be framed as
"the audit-#4 single-use store generalised." The shipped audit-#4 nonce binding is
**purely verifier-side** (every circuit does `let _ = challenge;`; the binding is
the public-input field-0 byte-compare). The nullifier is **fundamentally different
and is new in-circuit soundness surface**:

- **(a) It is a NEW in-circuit constraint:** `nf = Poseidon2([ZKSIG_NF, hsk,
  epoch])` is **computed IN-CIRCUIT** from the WITNESSED `hsk` and asserted equal
  to the public `nf` (a new `ZKSIG_NF` domain tag in `sig.rs`).
- **(b) Its soundness depends on TWO things:** the holder-PoK DL-binding of `hsk`
  (row P6, itself NOT-yet-sound) AND Poseidon2 domain-separated collision-
  resistance over `(ZKSIG_NF, hsk, epoch)`.
- **(c) Granularity must be stated explicitly:** `nf` binds to `hsk` and `epoch`
  but **NOT** to the credential/commitment. Therefore one holder key reused across
  **distinct** credentials in the same epoch **collides** on `nf`. This is
  **per-holder-per-epoch rate-limit granularity**, usable as a feature — but it is
  **NOT a per-presentation single-use token.** A per-presentation nullifier would
  additionally fold the commitment into the hash; that is a separate, larger
  obligation and is **not** what v1 claims.
- **(d) Double-spend enforcement** is a verifier-side seen-set of `nf` values per
  epoch (host-side bookkeeping), fail-closed on collision.

---

## 7. Wiring into the trust-graph admission gate (`sq-rsd3v.8`)

### 7.1 `admit()` is a PoC, not a shipped generalised gate

Per the SOUNDNESS lens grounding correction: there is **no general `sparq-trust`
`admit()` artifact** to "wire the four features into" as if it were finished. What
exists is the **PoC** `crates/sparq-trust` (`sq-pfae.10`, PR #966) — a default-OFF
`trust-graph` cargo feature whose `admit.rs` does: RDFC-1.0 canonicalise →
**CHECKED** `sparq-zk` issuer signature over the commitment → `sparq-shacl`
statement-type scoping → freshness → **clear-WebID** holder binding → default-deny
short-circuit, then `wire.rs` N3-merges admitted facts with `.acr` rules via
`sparq-reason`. Its holder binding is the non-anonymous degraded path; it makes no
privacy claim.

The four "open problems" are **OPEN beads** (`sq-l5og`, `sq-wvne`, `sq-tu4e`,
`sq-xc4y` under epic `sq-pfae`), not solved properties. So the dependency ordering
must be surfaced:

> **Build the trust-admission stratum generalisation FIRST (the gate the ZK
> discharge paths plug into), THEN add the ZK discharge paths.** You cannot "wire
> into the already-landed `admit()`" because the generalised gate must first exist
> beyond the PoC.

### 7.2 The unifying discipline (the audit-#1/#2/#4 law)

Every feature obeys the same law, made universal:

- **every prover-supplied field is reconstructed into / byte-equalled against the
  `bb` public inputs** (audit-#1/#2);
- **every trust anchor is the verifier's, never the prover's** — the issuer
  key-set `K`, the verifier challenge/nonce, the TBox/rule-graph commitment, the
  `EntailmentPolicy`, the epoch;
- **every gate is fail-closed behind a relying-party policy object** (the
  `EntailmentPolicy` default-`Simple` precedent, `verifier.rs:541`).

### 7.3 How each feature closes which open problem

| Open bead | Feature | What it supplies | What it does NOT |
|-----------|---------|------------------|------------------|
| `sq-l5og` (delegation replay) | §5 (`sq-rsd3v.4`) | per-request key-proof + nullifier + chain-carry | full confused-deputy; deep-chain revocation |
| `sq-wvne` (unlinkability) | §6 (`sq-rsd3v.5`) | the missing nullifier + in-ZK holder-PoP wiring | a SOUND composite (NOT-yet-sound, §1) |
| `sq-tu4e` (in-the-clear entailment) | §3/§4 (`sq-rsd3v.2`/`.3`) | ZK-over-hidden-antecedent entailment | completeness-under-entailment (X1) |
| `sq-xc4y` (per-request admission) | §5/§6 | per-request key/freshness binding via challenge | the materialise-once vs per-request cache reconciliation (host design) |

Every closure is **partial and gated**, never a settled guarantee.

---

## 8. Crate / feature plan (separate, opt-in, fail-closed)

Each is a **separate opt-in crate or cargo feature** so the core
(`sparq-core`/`sparq-engine`) stays lean (per the opt-in-feature-architecture
mandate). Nothing in the workspace depends on these by default; feature-off is
byte-identical to today.

1. **`sparq-zk-inference`** (`sq-rsd3v.2`) — §3. The in-circuit `derivation_step`
   member (row A1, greenfield) + the zk-trace `ProofTree` → witness mapper.
   RDFS-first; `owl:sameAs` behind its own bead (`sq-rsd3v.6`).
2. **`sparq-zk-n3`** (`sq-rsd3v.3`) — §4. Witnessed-rule-shape `derivation_step` +
   the builtin gadget whitelist. Depends on (1).
3. **delegation-binding challenge** (`sq-rsd3v.4`) — §5. The `ZKSIG_DG` variant +
   per-request PoK reuse + chain-carry (host plumbing in `sparq-solid`). Depends
   on the nullifier (4).
4. **nullifier primitive** (`sq-rsd3v.1`) — §6.3. The one genuinely-new in-circuit
   gadget. Independent; prerequisite for §5 and §6.
5. **unlinkable presentation** (`sq-rsd3v.5`) — §6. Composes (i) hidden-issuer +
   (ii) hidden-holder-PoK + (iii) nullifier. Depends on (4).
6. **trust-graph integration** (`sq-rsd3v.8`) — §7. Generalise the `sparq-trust`
   PoC gate into the stratum the ZK discharge paths plug into; then wire the rest.

Build order honours the dependency DAG: **(4) nullifier → (1) inference →
(2) N3**, with **(3) delegation** and **(5) unlinkability** after (4), and
**(6) integration** last (it consumes the rest and must FIRST generalise the PoC
gate). Separately gated: `owl:sameAs` (`sq-rsd3v.6`) — its canonicalisation
relation + host mirror have now landed (§3.5.1), but with NO compiled member, NO
`bb gates` measurement, and NO manifest dispatch, so nothing composes it yet and
no cost or soundness property is claimed — and completeness-under-entailment
(`sq-rsd3v.7`), still deferred and NOT claimed.

---

## 9. Honesty ledger (what is NOT delivered / NOT claimed)

- The composition verifier is the maintainer's **own** not-yet-sound position
  (`sq-qhy4` / `sq-9hrn`; remediation `sq-1s2`); external sign-off pending. **No
  feature here is "proven" or "sound" end-to-end.** (§1)
- The in-circuit derivation/N3 relations (rows A1, A2) and the nullifier (A3) are
  **greenfield Noir / new circuit-soundness surface**, not tweaks to a skeleton.
  Every cost figure is a **judgement pending `bb gates`** measurement. (§2)
- **Completeness-under-entailment (X1, `sq-rsd3v.7`) is unbuilt and NOT claimed.**
  (§3.7)
- The nullifier is **per-holder-per-epoch**, NOT per-presentation single-use, and
  its soundness rides the NOT-yet-sound holder-PoK. (§6.3)
- Delegation v1 supplies the **key-binding prerequisite**, not confused-deputy
  closure; deep-chain revocation has an **unbounded stale-authority window**. (§5.3)
- There is **no shipped generalised `admit()` gate** — only the `sq-pfae.10` PoC;
  the gate must be generalised before the ZK paths plug in. (§7.1)
- **No production security or privacy claim is made.** This is a research-grade,
  design-for-review record.

---

## 10. References

- `research/zkp-query-proofs-plan.md` — the ZK SPARQL query-proof plan
  (`I1`/`I3`, §4B gate anchors, §5 entailment).
- `research/zk-soundness-audit.md`, `research/zk-verifier-reaudit.md` — the
  audit-#1…#12 discipline and the not-yet-sound verdict.
- `research/zk-holder-pop-design.md` — the holder-binding precedent reused by §5.
- `research/solid-trust-graph-authz-design.md` — the trust-graph design whose §4
  (delegation), §5 (unlinkability), §3.3/§3.5 (conflict / in-the-clear entailment)
  open problems this record closes; epic `sq-pfae`.
- `research/inference.md`, `research/inference-sota.md`,
  `research/inference-completeness-audit.md` — the reasoner's RDFS/OWL-RL/N3
  semantics and the completeness boundary.
- `research/zkp-performance-landscape.md` — the folding / zkVM measured-out
  verdict (§3.6 approach (c)).
- Code: `crates/sparq-zk-compose/src/{derivation,verifier,manifest,build,issuer,revocation}.rs`;
  `crates/sparq-zk/src/sig.rs`; `zk/compose/compose_core/src/{scan,holder,issuer,filter_int}.nr`;
  `crates/sparq-reason/src/{explain,incremental_explain,n3,lib}.rs`;
  `crates/sparq-trust` (PoC), `crates/sparq-solid/src/materialize.rs`.
- W3C / external: SPARQL 1.1; RDF 1.1 Semantics (RDFS/OWL 2 RL entailment regimes);
  Notation3 (N3) and the `cwm` builtin families (`math:`, `log:`, `string:`,
  `list:`, `time:`); ZCAP-LD and UCAN (delegation vs invocation); Privacy Pass
  (RFC 9576/9577) and the ZKAPs / Anonymous-Credentials-Light lineage
  (unlinkable presentation + nullifier double-spend).
