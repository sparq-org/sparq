<!-- [OPUS-4.8] Design-for-review authored by Opus 4.8 (1M context) (Fable unavailable) — re-review when Fable returns. -->
<!-- 🤖 SPARQ agent — security-properties ontology design record. -->
# A security-properties ontology for ZK proofs/systems — ODRL-driven proof admissibility

Maintainer-review design record for epic **sq-0dksu** (the maintainer's flagship
trust-graph follow-up). It designs an ontology that **describes the security/privacy
properties of a given proof or system** — unlinkability, post-quantum security,
zero-knowledge type, soundness, hiding/binding, anonymity-set semantics, trusted-vs-
transparent setup, interactive-vs-non-interactive — and a reasoning bridge so that an
**N3/ODRL** reasoner can decide which proofs are **admissible** given a user's ODRL-
expressed privacy preferences (e.g. *"require unlinkable AND pq-secure"* → the
admissible-proof set).

It sits **on top of** three existing in-repo artifacts and does not restate or
contradict them:

- the **trust-graph authorisation** design + PoC (`research/solid-trust-graph-authz-design.md`
  and `crates/sparq-trust`, epic **sq-pfae** / issue #940 — both currently on the
  unmerged `feat-trust-graph-poc` branch, PRs #951/#966) — the *admission gate* that
  this ontology plugs a property-admissibility check into;
- the **configurable ZK commitment/circuit/signature framework** (`research/zk-configurable-commitment-design.md`,
  epic **sq-1s2.5**) and the per-method **security-properties write-up**
  (`research/zk-configurable-commitment-security.md`, bead **sq-pkrl** — the prose
  the maintainer called out as the thing to make machine-readable; the brief refers
  to it via the in-flight PR #919) — the *methods* this ontology annotates;
- the **ZK↔ODRL constraint-discharge envelope** (bead **sq-yh427**, the #890(b)
  coordination with the PSS sibling) — this ontology is the **machine-readable
  realisation** of that envelope's property half.

> **HONESTY FRAMING (load-bearing — read first).** This ontology describes **CLAIMED**
> properties; it does **not** establish them. A `secx:Unlinkable` or
> `secx:PostQuantum` assertion on a sparq proof method is only as good as the
> underlying ZK estate, which is **remediated and internally re-audited but has NO
> external accredited-cryptographer sign-off** (`sq-qhy4`); `sparq-mpc` is honest-
> majority **semi-honest only**. So every property annotation on a *sparq* method must
> carry an **assurance/provenance** qualifier (`secx:assurance secx:Claimed`,
> `secx:auditStatus secx:ExternalSignOffPending`) and the reasoner must be able
> to treat *unaudited-claimed* differently from *audited*. The ontology's job is to
> make the gap **explicit and machine-checkable**, not to paper over it. No row of this
> document asserts a settled ZK/MPC privacy-or-soundness guarantee.

---

## 0. The request (verbatim)

> "When it comes to the security properties you are describing, I would like to have an
> ontology to describe the security properties of a given system / proof; that can be
> used to reason e.g. about what proofs can/cannot be used based on privacy preferences
> that a user has expressed using ODRL. This should capture things like unlinkability,
> post quantum security, etc. I had already thought about and built some of this out in
> the context of the work I had previously done with zero knowledge proof and SPARQL.
> Can you build this in and use it in the appropriate places. Can you also provide a
> critical evaluation of what the right next steps for things to build out in this area
> are." — @jeswr, 2026-06-20 (bead sq-0dksu)

---

## 1. Premise corrections (be honest up front)

Three premises in the task brief / epic need correcting against the actual repo state,
so the design rests on reality:

1. **`crates/sparq-trust` is real but NOT on `main`.** The brief and the epic say
   "wired into the trust-graph PoC admission gate (`crates/sparq-trust`)". That crate
   *exists* — it was authored under bead sq-pfae.10 — but it lives on the unmerged
   branch `feat-trust-graph-poc` (PR #966); `research/solid-trust-graph-authz-design.md`
   lives on the unmerged PR #951 (latest content at commit `fa112a59`). Neither is on
   `origin/main` as of this writing. **Consequence:** the wiring this ontology
   specifies (§5b) is *designed against* the PoC's `admit.rs` gate, but the impl beads
   must depend on those PRs landing first (or be sequenced behind them). This is stated,
   not assumed.

2. **The maintainer's "I had already built some of this out" is TRUE — and the thing he
   built is genuinely good, so this ontology BUILDS ON it rather than minting fresh.**
   The prior-work survey (§3 / appendix §A) found a real, CI-validated, SHACL-shaped
   security-property vocabulary in his **private** repo `jeswr/sparql-zkp-ontologies`
   — the companion to his **ISWC 2025** research-track paper *"A SPARQL extension for
   Zero Knowledge Query over Verifiable Credentials"* (Wright, Shadbolt, Jun Zhao, Rui
   Zhao, Oxford). It defines **four** vocabularies under placeholder IRIs
   `https://w3id.org/zkp-sparql/`: **`sec-prop:`** (eight `skos:Concept` security
   properties, each with threat / defence / openQuestion / category / paperSection +
   `prov:wasDerivedFrom` bibliography), **`sig-impl:`** (reified `sig-impl:Assertion`
   verdicts — yes/no/partial + justification + provenance, per (signature-implementation,
   property) pair, for BBS+/SD-JWT-VC/ed25519/ECDSA), **`sec-req:`** (regulatory
   requirements — eIDAS 2.0 / NIST PQC / UK DVS — linked to properties via
   `sec-req:pulls`), and **`prov-ext:bibtexKey`**. **This is the "some of this" he
   referred to, and it is the strongest single artefact in the prior art.** So the
   honest design move is **BUILD-ON `sec-prop:` + adopt the `sig-impl:Assertion` reified
   pattern**, *extending* it with the orthogonal dimensions it lacks — not reinventing a
   parallel `secx:` namespace. §3 and §4.2 reconcile his 8 properties with the fuller
   dimension set. The `zk:`/`trust:` registry vocabularies he shipped in *this* repo are
   *authorisation/registry* vocabularies (the annotation **targets**), not the property
   ontology. (His "a lot of that could be crap" caveat is taken at face value: §3 keeps
   only what survives scrutiny — and most of `sparql-zkp-ontologies` survives.)

3. **"Supersedes the prose #919" is the right intent, but the prose is not deleted.**
   The per-method write-up stays as the human-readable rationale; this ontology gives
   it a **machine-readable, reasoner-consumable** form. A research doc that is later
   superseded graduates per the repo's "documents must stay current" rule — but the
   *security argument* in the prose (the INV-VL downgrade, the audit obligations) is
   durable and is *referenced*, not duplicated, by the ontology's per-method
   annotations.

---

## 2. Problem framing — what the ontology must do, and what it must NOT

**Must do.**

- Give every sparq proof-producing **method** (a commitment method, a circuit family,
  a signature scheme, a VC cryptosuite, an MPC protocol) a set of machine-readable
  **security-property annotations** at resolvable IRIs.
- Let a user express a **privacy preference** as an **ODRL constraint** over those
  properties (`odrl:leftOperand secx:requiresUnlinkability ; odrl:operator
  odrl:gteq ; odrl:rightOperand secx:CrossPresentation`), and let an N3/ODRL
  reasoner compute the **admissible-proof set** — the methods whose annotations satisfy
  every constraint.
- Carry, on every annotation, the **assurance/provenance** qualifier so *unaudited-
  claimed* is never silently treated as *audited-proven* (the sq-qhy4 gate, in the
  data model rather than only in prose).

**Must NOT do (scope discipline).**

- It is **not** a proof of any property; it records claims + their assurance basis.
- It is **not** an authorisation model (that is `trust:`/`acp:`/`odrl:`) — it is an
  **orthogonal** property vocabulary the authorisation layer *consults*.
- It is **not** a crypto-algorithm registry (that is the IANA/COSE/`zk:scheme` job) —
  it *annotates* those algorithm IRIs; it does not re-mint them.
- It is **minimal and orthogonal** — the same de-dup / irreducibility discipline the
  trust ontology applied (the `forShape` "one statement-type primitive, `forPredicate`
  is sugar" move): no two terms express the same dimension, and every term earns its
  place by an irreducibility argument (§4.4).

---

## 3. The maintainer's own prior ZK+SPARQL work — critical fold-in

The prior-work survey read his GitHub estate, the ISWC 2025 paper, and the in-repo
digests of his `lws-acp` work. The full raw findings + exact sources are in appendix §A;
the critical KEEP/REJECT take is here. The headline: **he already built a rigorous
security-property vocabulary (`sparql-zkp-ontologies`), and this design builds on it.**

### 3.1 KEEP — build directly on these

1. **`sec-prop:` — the eight-property vocabulary** (`jeswr/sparql-zkp-ontologies`,
   `vocab/sec-prop.yaml.ld`, namespace `https://w3id.org/zkp-sparql/sec-prop#`). **The
   best artefact in the prior art.** Eight `skos:Concept`-typed `sec-prop:SecurityProperty`
   instances, each with `threat` / `defence` / `openQuestion` / `category`
   (`formalAnalysisPriority` / `informationDisclosure` / `deploymentSpec`) / `paperSection`
   and `prov:wasDerivedFrom` bibliography. The eight:
   `Unlinkability`, `SourceCredentialDisclosure`, `PostQuantumForgery`,
   `PostQuantumSnooping`, `SignatureTypeLeakage`, `ProofSizeLeakage`, `CircuitAudit`,
   `ValidityPeriodLeakage`. **KEEP wholesale — reuse the eight IRIs and the
   threat/defence/openQuestion structure.** (The IRIs are now **bound hard**: per the
   2026-06-20 decision (§10.1) these ontologies are **vendored into sparq** at
   `crates/sparq-trust/ontologies/zkp-sparql/` and the `w3id.org/zkp-sparql/` IRIs are
   kept as-is — stable under the external repo's archive.)
2. **`sig-impl:Assertion` — the reified per-(implementation, property) verdict pattern**
   (`vocab/sig-impl.yaml.ld`). Each (scheme, property) verdict is a node with
   `sig-impl:yes`/`no`/`partial` + a justification string + provenance. **KEEP the
   pattern wholesale** — it is *exactly* the right shape for a machine-readable per-method
   security posture, and §4.2.2's `PropertyAssertion` adopts it (it is strictly better
   than a prose table or a flat triple). His encoded verdicts (BBS+ `Unlinkability=yes,
   PostQuantumForgery=no`; ed25519 `Unlinkability=no` (deterministic, needs PoK lift);
   SD-JWT-VC `Unlinkability=no` (replay)) are reused as the **source-layer** annotations.
3. **`sec-req:` → `sec-prop:` `pulls` linkage** (`vocab/sec-req.yaml.ld`) — eIDAS 2.0 /
   NIST PQC (IR 8547 / FIPS 204) / UK DVS as `sec-req:Requirement` instances with dated
   deadlines, each `pulls`-ing the cryptographic properties it demands. **KEEP** — a
   genuinely-uncommon and useful regulation→property bridge; it composes with the ODRL
   preference layer (a regulator's requirement and a user's preference are both
   constraints over the same property dimensions).
4. **The per-method INV-VL framing** from this repo's `zk-configurable-commitment-security.md`
   (the prose #919 / sq-pkrl) — value↔lexical agreement machine-enforced (string-canonical)
   vs trusted-issuer-honesty (dual-leaf), and `CommitmentMethod::removes_inv_vl()`. **KEEP**
   — these are the per-method annotation content; the ontology serialises them (§5a).
5. **The ten-term `trust:` ontology + `forShape` irreducibility discipline** (trust-graph
   design §2.3). Not a security-property vocabulary, but the **design method** to copy:
   one primitive per dimension, sugar desugars to it, every term has an irreducibility
   argument. Applied in §4.4. **KEEP — as method.**
6. **The adversarial `zk-comp`→`gap` reclassification** (trust-graph design §5.4.3) — the
   recognition that clear-WebID holder binding breaks *presentation* unlinkability
   regardless of added ZK. **KEEP** — it is why §5a annotates sparq query proofs at most
   `PerPresentation`, never `CrossPresentation`, today.

### 3.2 REJECT (with reasons)

1. **The ISWC 2025 `.tex` as a source of technical content** — the submitted LaTeX is a
   late-stage skeleton (§3–7 placeholders; two abstract typos); the real content is in
   the companion repos. REJECT the `.tex`; KEEP the paper's *existence* as validation of
   the research line and as the eventual home of the published vocabulary.
2. **`risc0-ed25519-zk-sparql` / `circomkit-sparql`** — earlier ZKVM/Circom iterations,
   **superseded** by the Noir `sparql_noir_modular` approach (and by sparq's own
   `sparq-zk`/`sparq-zk-compose`). REJECT as design anchors.
3. **The `lws-acp` 15-layer taxonomy as a normative unit** — over-built; layers 5–14 are
   references to existing standards. KEEP only the Layer-0 (truth conditions) / Layer-4
   (evidence admission) split, which maps onto the trust-graph admission/derivation
   strata. REJECT the layer numbering.
4. **`lws:allOf`/`lws:not` combinators** — collide with shipped `acp:allOf`/`acp:noneOf`.
   REJECT in favour of the ACP terms (not relevant to `sec-prop:` anyway).
5. **The unqualified "superset of ZKaps" claim** (from the brief framing, not an artefact)
   — false as stated; the defensible claim is *policy-expressivity superset + composition
   (not supersession) of unlinkability* (trust-graph §5.3). Already corrected in the
   trust-graph design; reaffirmed here.

### 3.3 The gaps in his `sec-prop:` that this design fills (the genuine delta)

His eight properties are **paper-§7.7-specific** — scoped to the ZK-over-VC query setting.
For a general ZK-proof/system property ontology that an ODRL gate reasons over, they are
**missing** several orthogonal dimensions (confirmed by the survey):

- **soundness / knowledge-soundness / completeness** (his set assumes the circuit is
  sound — `CircuitAudit` is about *the generator being bug-free*, not the proof system's
  soundness *type*);
- **hiding / binding** for commitments (the perfectly-hiding-XOR-perfectly-binding axis);
- **trusted-vs-transparent setup** and **interactive-vs-NI**;
- **anonymity-set semantics** as a parameter (his `Unlinkability` is boolean-ish);
- **single-use / nullifier** (flagged as a gap elsewhere in his work but absent from the
  vocabulary itself);
- and crucially the **assurance / audit-status axis** (§4.2.2) — his properties carry an
  `openQuestion` but not a machine-reasonable *epistemic basis* (`Claimed` vs `Proven` vs
  `ExternalSignOffPending`) that an admissibility reasoner can gate on.

So the design = **`sec-prop:` (his 8) ∪ the orthogonal dimensions above ∪ the assurance
axis ∪ the ODRL→admissibility reduction**. The 8 are reused; the delta is the contribution
(§7.4). This is "build it in," done honestly.

---

## 4. The ontology design

### 4.1 Namespace and shape — extend `sec-prop:`, do not fork it

**Decision: extend the maintainer's existing `sec-prop:` namespace
(`https://w3id.org/zkp-sparql/sec-prop#`), not mint a parallel `sparq.dev/ns/secprop#`.**
He already owns and CI-validates this namespace (§3.1); forking it into a sparq-local
namespace would split the vocabulary and contradict "build it in." The reused terms keep
his IRIs; the new orthogonal dimensions (§3.3) are added **under the same namespace** as
new `sec-prop:` individuals/classes; the per-method annotation graph (§5a) lives in
sparq under the existing `zk:` namespace it keys on.

The earlier nuance here (§10.1) is now **RESOLVED (2026-06-20)**: rather than make the
`sparql-zkp-ontologies` repo public, the maintainer chose to **vendor its ontologies
verbatim into sparq** at `crates/sparq-trust/ontologies/zkp-sparql/` (from SHA
`0fe80ea7`) and archive the external repo. The `w3id.org/zkp-sparql/` IRIs are **kept
as-is and bound hard** — w3id IRIs redirect via the permanent-identifier service
independently of the source repo's visibility, so they resolve and stay stable under
archive; no thin indirection or sparq-local mirror is needed. The
prose in this document uses the `sec-prop:` prefix for reused terms and a `secx:`
("sec-prop extension") prefix for *new* terms the design adds, so it is visible which
came from him vs which are new — but both resolve under his namespace once released:

```text
@prefix sec-prop: <https://w3id.org/zkp-sparql/sec-prop#> .   # his 8 + the new dims
@prefix sig-impl: <https://w3id.org/zkp-sparql/sig-impl#> .   # his reified-verdict pattern
@prefix sec-req:  <https://w3id.org/zkp-sparql/sec-req#> .    # his regulation->property bridge
@prefix zk:       <https://sparq.dev/ns/zk#> .                # the annotation TARGETS (sparq)
```

**External alignment (from the survey, §6 — the load-bearing finding).** Almost no
external vocabulary models *per-proof cryptographic properties* at resolvable IRIs; the
crypto literature is prose-only and the security ontologies are org/control-level. So the
core property IRIs are genuinely net-new (his + the §3.3 delta), but two external sources
are real reuse:

- **W3C DPV (`https://w3id.org/dpv#`) — the sole BUILD-ON.** Its `dpv:CryptographicMethods`
  tree (in *core* `dpv:`, **not** a `dpv-tech` module — correcting the brief's
  assumption) publishes stable IRIs for the *applied technique* layer:
  `dpv:Anonymisation`, `dpv:Pseudonymisation` (+ subtypes),
  `dpv:SecureMultiPartyComputation`, `dpv:HomomorphicEncryption`,
  `dpv:DifferentialPrivacy`, `dpv:PostQuantumCryptography`,
  `dpv:ZeroKnowledgeAuthentication`, `dpv:TrustedExecutionEnvironment`. Reuse these to
  tag a *method's technique class*; **but DPV stops at applied techniques** — it has **no**
  general `ZeroKnowledgeProof`, `SelectiveDisclosure`, `Unlinkability`, `AnonymitySet`, or
  `(ε,δ)` terms (confirmed absent), which is exactly the per-proof-property gap this
  ontology fills. Each minted property anchors to DPV with `skos:closeMatch`/`rdfs:seeAlso`
  where a near-match exists (e.g. `secx:PostQuantumForgery skos:closeMatch
  dpv:PostQuantumCryptography`).
- **W3C Security Vocabulary (`https://w3id.org/security#`) — BUILD-ON for proof
  *structure*** (`sec:Proof`, `sec:DataIntegrityProof`, `sec:cryptosuite`,
  `sec:verificationMethod`) and **PROV-O** for proof *provenance* (`prov:wasGeneratedBy`,
  `prov:wasDerivedFrom` — which his `sig-impl:` already uses). Neither carries a security
  *property*; they carry the scaffold the annotations hang on.

Other sources are **ALIGN-only** by citation (Pfitzmann–Hansen for anonymity/unlinkability
definitions; ZKProof/IRTF for ZK/soundness; Privacy Pass RFC 9576 for the four named
unlinkability classes; NIST FIPS-203/204/205 for the security-category 1–5 numbers; the VC
cryptosuite *string* identifiers `bbs-2023`/`ecdsa-sd-2023`) — all prose-only/non-IRI, so
they are cited, not imported. The full per-source verdicts are in §6 + appendix §B.

The ontology has **three kinds of thing**:

1. **Property classes** — the security/privacy dimensions (his `sec-prop:Unlinkability`,
   the new `sec-prop:ZeroKnowledgeType`, …), each with an enumerated set of
   **levels/classes** (e.g. unlinkability scope has `PerPresentation` ⊂
   `CrossPresentation`; ZK-ness has `Computational` ⊂ `Statistical` ⊂ `Perfect`).
2. **A `secx:hasProperty` annotation** (his `sig-impl:Assertion` pattern, generalised)
   attaching a (property, level, assurance, audit-status, assumption) bundle to a
   **method IRI** (a `zk:scheme`, `zk:cryptosuite`, a circuit-family IRI, an `mpc:`
   protocol IRI).
3. **ODRL bridge terms** — `secx:` **leftOperand** IRIs (one per requireable dimension)
   so an `odrl:Constraint` can reference a property, plus the N3 rules that reduce
   "constraint over leftOperand X" to "method's annotation for X satisfies it".

### 4.2 The term set (the irreducible core)

The core is **orthogonal**: each property is one dimension; levels within a dimension
form a (usually total) order so `gteq`/`lteq` ODRL operators work directly. The
**assurance** axis is *separate* from every property (so "claimed unlinkable" and
"audited unlinkable" differ in one slot, not in N copies of every property).

#### 4.2.1 Property dimensions and their levels

The **`src`** column marks provenance: **[his]** = reused from his `sec-prop:` (§3.1, kept
verbatim where his term is boolean we *refine* into ordered levels but keep the IRI);
**[new]** = an orthogonal dimension this design adds (§3.3 delta), minted under the same
namespace. Levels run most-… to least- as a partial/total order (`⊐` = "stronger than").

| Property class | src | Levels (`⊐` = stronger than) | Notes / grounded source |
|---|---|---|---|
| **`Unlinkability`** | [his] | `EverlastingUnlinkable` ⊐ `ComputationalUnlinkable` ; **and** orthogonally `CrossPresentation` (multi-show) ⊐ `PerPresentation` ⊐ `Linkable` | His `sec-prop:Unlinkability` (cross-presentation/cross-credential linkage; BBS+ defence) — **refined** from boolean into a level structure. Honesty note (survey §6): the everlasting/computational and per/cross-presentation split is the **academic** unlinkability taxonomy (grounded in Pfitzmann–Hansen v0.34 for the definitions), **not** terms in the IRTF CFRG BBS draft — so these levels are minted + cited, not lifted from a spec. Two sub-axes (strength × scope) ⇒ two ordered properties `UnlinkabilityStrength`, `UnlinkabilityScope`, both `rdfs:subPropertyOf` his `Unlinkability` so the original IRI is preserved. |
| **`PostQuantumForgery`** | [his] | `PQForgeryResistant` / `PQForgeable` ; with `nistLevel` ∈ {1..5} | His `sec-prop:PostQuantumForgery` (CRQC recovers issuer keys → forges credentials; today BBS+/Schnorr/EdDSA all Shor-broken; ML-DSA migration). KEEP his IRI; add the NIST-level parameter. |
| **`PostQuantumSnooping`** | [his] | `PQHiding` / `PQRevealable` | His `sec-prop:PostQuantumSnooping` (harvest-now-decrypt-later; Pedersen/Poseidon2 statistical hiding holds, but Pedersen *binding* breaks under CRQC). KEEP his IRI — this is the subtle hiding-vs-binding-under-PQ split he already modelled, which a naive "PQSecure boolean" would lose. |
| **`SourceCredentialDisclosure`** | [his] | `NoIssuerDisclosure` ⊐ `IssuerSetDisclosure` ⊐ `FullSourceDisclosure` | His `sec-prop:SourceCredentialDisclosure` (what the verifier learns about which credentials sourced a result). KEEP; refine into ordered levels (his Merkle-over-dataset = `IssuerSetDisclosure`). |
| **`SignatureTypeLeakage`** | [his] | `SchemeHidden` / `SchemeRevealed` | His `sec-prop:SignatureTypeLeakage` (PoK transcript reveals which sig scheme; generic-PoK-adapter defence). KEEP his IRI. |
| **`ProofSizeLeakage`** | [his] | `FixedSize` / `StructureLeaking` | His `sec-prop:ProofSizeLeakage` (variable-size circuits leak structure; bounded-unrolling defence). KEEP his IRI. |
| **`CircuitAudit`** | [his] | `MechanisedProof` ⊐ `ManualAudit` ⊐ `Unaudited` | His `sec-prop:CircuitAudit` (generator-bug → circuit not implying claimed SPARQL semantics; Lean-4 mechanised soundness defence). KEEP his IRI; this is **distinct** from the new `Soundness` dimension (his = "the generator is correct"; new = "the proof system's soundness *type*"). |
| **`ValidityPeriodLeakage`** | [his] | `ValidityHidden` / `ValidityRevealed` | His `sec-prop:ValidityPeriodLeakage` (validity periods reveal which credential sourced an answer). KEEP his IRI. |
| **`ZeroKnowledgeType`** | [new] | `PerfectZK` ⊐ `StatisticalZK` ⊐ `ComputationalZK` ⊐ `NotZK` | ZKProof reference / IRTF terminology. (His set assumed ZK; did not type it.) |
| **`Soundness`** | [new] | `KnowledgeSound` ⊐ `Sound` ⊐ `Unsound` ; orthogonally `StatisticalSoundness` ⊐ `ComputationalSoundness` | ZKProof reference; "argument of knowledge" = computational + knowledge-sound. Distinct from his `CircuitAudit` (generator correctness). |
| **`Completeness`** | [new] | `Complete` / `Incomplete` | ZKProof reference. |
| **`Hiding`** (commitments) | [new] | `PerfectHiding` ⊐ `StatisticalHiding` ⊐ `ComputationalHiding` ⊐ `NotHiding` | Commitment terminology; relates to his `PostQuantumSnooping` (the PQ-time slice of hiding) but is the general-time hiding axis. |
| **`Binding`** (commitments) | [new] | `PerfectBinding` ⊐ `StatisticalBinding` ⊐ `ComputationalBinding` ⊐ `NotBinding` | dual of Hiding; the perfectly-hiding-XOR-perfectly-binding impossibility is *why* both are needed. |
| **`Anonymity`** | [new] | `Anonymous` ⊐ `Pseudonymous` ⊐ `Identified` ; with `anonymitySet` parameter | Pfitzmann–Hansen anonymity + anonymity set. The trust-graph §3.4 clear-WebID path is `Identified` — this lets the ontology *say* that. |
| **`Setup`** | [new] | `Transparent` ⊐ `UniversalTrustedSetup` ⊐ `PerCircuitTrustedSetup` | STARK/Bulletproofs transparent; PLONK/KZG universal+updatable; Groth16 per-circuit. |
| **`Interactivity`** | [new] | `NonInteractive` / `Interactive` | NI via Fiat–Shamir; load-bearing for "presentable as a credential". |
| **`SelectiveDisclosure`** | [new] | `SelectivelyDisclosable` / `AllOrNothing` | BBS / ecdsa-sd vs plain suites; matches sparq's per-leaf disclosure. |
| **`SingleUse`** | [new] | `SingleUse` (nullifier-enforced) / `Replayable` | the absent ZKAPs/Privacy-Pass anti-replay primitive (trust-graph §5.3(3)); his work flagged it as a gap but did not put it in the vocabulary. Lets a preference *require* it and the gate *deny* every current sparq method (honest: all are `Replayable` today). |

**Reused [his]: 8 dimensions** (his complete `sec-prop:` set). **New [new]: 9 dimensions**
(`Unlinkability` contributing a Strength+Scope refinement on top). The reused set covers
the information-disclosure + PQ + linkability concerns of the ZK-over-VC setting; the new
set covers the proof-system-theoretic dimensions (ZK-type, soundness, hiding/binding,
setup, interactivity) and the anti-replay/anonymity-set gaps the brief explicitly names.
This is the minimal set that covers the brief's list **without** redundancy (§4.4) **and**
without forking his vocabulary.

#### 4.2.2 The annotation shape + the assurance axis (orthogonal to every property)

The annotation shape **adopts his `sig-impl:Assertion` reified pattern** (§3.1.2),
generalised from "(signature-impl, property) → verdict" to "(any method IRI, property) →
(level, assurance, audit-status, assumption, evidence)". A `secx:PropertyAssertion`
(`rdfs:subClassOf sig-impl:Assertion`) is the reified node; the **assurance** of the
claim is **one axis, orthogonal to every property** (the key de-dup move — stated once,
not multiplied across all dimensions):

| Term | src | Meaning |
|---|---|---|
| **`secx:hasProperty`** | [new] | attaches a `secx:PropertyAssertion` node to a method IRI (generalises `sig-impl:` reification to any method). |
| `secx:property` | [new] | the dimension asserted (e.g. `sec-prop:UnlinkabilityScope`). |
| `secx:level` | [new] | the level/class held (e.g. `secx:CrossPresentation`). |
| `secx:parameter` | [new] | optional numeric parameter (`secx:nistLevel`, `secx:anonymitySet`, an `(ε,δ)` for DP). |
| **`secx:assurance`** | [new] | one of `secx:Proven` ⊐ `secx:Claimed` ⊐ `secx:Conjectured` — the **epistemic basis** of the claim. |
| **`secx:auditStatus`** | [new] | `secx:ExternallyAudited` ⊐ `secx:InternallyReviewed` ⊐ `secx:Unreviewed` ; plus `secx:ExternalSignOffPending` for the live sq-qhy4 state. |
| `secx:assumption` | [new] | IRI naming the assumption the claim rests on (`secx:IssuerHonesty`, `secx:DiscreteLog`, `secx:RandomOracle`, `secx:HonestMajority`, `secx:SemiHonest`). |
| `secx:auditEvidence` | [new] | `rdfs:seeAlso` to the audit doc / gap-register row (e.g. `compliance/cryptoreview/gap-register.md#CR-G1`). |
| (`sig-impl:justification`, `prov:wasDerivedFrom`) | [his] | the justification string + bibliography his pattern already carries — reused unchanged. |

The assurance axis is the contribution his vocabulary lacks (§3.3): his properties carry
an `openQuestion` prose field, but not a **machine-reasonable epistemic basis** a reasoner
can gate on. This single split is the honesty mechanism: a sparq ZK method is annotated,
today, as `secx:assurance secx:Claimed ; secx:auditStatus secx:ExternalSignOffPending ;
secx:auditEvidence <…sq-qhy4>` — so a reasoner can be configured to **require `assurance
gteq secx:Proven`** and thereby admit *nothing* from the unaudited estate, the correct
conservative default until sq-qhy4 closes.

#### 4.2.3 Issuer-trust assumption

Issuer-trust is captured as an **assumption** on the relevant property (`secx:assumption
secx:IssuerHonesty`) rather than a separate property class — because it is not a property
of the *proof* but a *precondition* of a property's validity (exactly the dual-leaf
INV-VL case from `zk-configurable-commitment-security.md`: term-identity soundness holds
*only under* issuer honesty). This keeps the dimension count minimal (§4.4) and makes the
trust assumption first-class and queryable. (His `sig-impl:` verdicts already encode an
analogous justification per scheme — e.g. ed25519 `Unlinkability=no` "deterministic,
requires PoK lift"; the `secx:assumption` slot makes that machine-reasonable, not just a
string.)

### 4.3 The ODRL reasoning bridge

#### 4.3.1 How a preference is expressed — via a published ODRL Profile

An ODRL `Constraint` references a `secx:` **leftOperand** IRI (one per requireable
dimension), a standard `odrl:operator`, and a level IRI (or a literal parameter) as
`rightOperand`. **The load-bearing ODRL nuance** (survey §6): a `rightOperand` natively
accepts **any IRI** with no declaration, but a **custom `leftOperand`** (and a custom
operator) **MUST be declared in a published ODRL Profile** — as `owl:NamedIndividual,
odrl:LeftOperand, skos:Concept` with `rdfs:isDefinedBy` → the profile — and the policy
must assert `odrl:profile <profileIRI>` (a conforming processor may flag an undeclared
leftOperand). Real precedents exist (the ODRL Regulatory-Compliance Profile, the 2025
Spatial-Axis Profile with 15 custom leftOperands). So the bridge ships a **tiny sparq
ODRL profile** that declares the `secx:requires…` leftOperands; it is a small,
standards-conformant addition, not ad-hoc.

sparq's ODRL model (`crates/sparq-policy/src/model.rs`) already supports **custom
leftOperand IRIs** (parsed via `Operator::from_iri` + an opaque leftOperand string), the
common operator set (`eq`/`gteq`/…), and an **IRI-valued rightOperand** (`Value::Iri`) —
so it can *evaluate* such a profile today; Phase 4 (§8) makes the `secx:` leftOperands
first-class and ships the profile document.

Worked preference — *"require unlinkable (cross-presentation) AND post-quantum, audited"*:

```turtle
@prefix odrl:     <http://www.w3.org/ns/odrl/2/> .
@prefix secx:     <https://w3id.org/zkp-sparql/sec-prop#> .   # extension terms (see §4.1)
@prefix sparqorl: <https://sparq.dev/ns/odrl-secprop-profile#> .

<urn:pref:alice-privacy> a odrl:Policy ;
  odrl:profile sparqorl: ;                       # declares the secx:requires* leftOperands
  odrl:permission [
    odrl:action odrl:read ;
    odrl:constraint
      [ odrl:leftOperand  secx:requiresUnlinkabilityScope ;
        odrl:operator      odrl:gteq ;
        odrl:rightOperand  secx:CrossPresentation ] ,
      [ odrl:leftOperand  secx:requiresPostQuantumForgery ;
        odrl:operator      odrl:eq ;
        odrl:rightOperand  secx:PQForgeryResistant ] ,
      # the conservative assurance gate — opt in to "only audited claims count"
      [ odrl:leftOperand  secx:requiresAssurance ;
        odrl:operator      odrl:gteq ;
        odrl:rightOperand  secx:Proven ] ] .
```

#### 4.3.2 The N3 rule shape (runs on sparq-reason's N3 engine)

`sparq-reason` ships a real N3 forward engine (`=>` rules, `math:` comparisons, `log:`
builtins — `crates/sparq-reason/src/n3/`), so the admissibility reduction is a genuinely
runnable N3 ruleset, not pseudocode. The reduction has two parts: a **level-order**
fact base (the `⊐` orderings, materialised as `secx:atLeast` facts) and a
**constraint-discharge** rule per operator.

A level order is stated once per dimension as ground facts (transitively closed by a
single rule), e.g. for unlinkability scope:

```n3
@prefix secx: <https://w3id.org/zkp-sparql/sec-prop#> .

# the order: CrossPresentation > PerPresentation > Linkable  (materialised once)
secx:CrossPresentation secx:strongerThan secx:PerPresentation .
secx:PerPresentation   secx:strongerThan secx:Linkable .
# transitive closure (one rule, all dimensions):
{ ?a secx:strongerThan ?b . ?b secx:strongerThan ?c } => { ?a secx:strongerThan ?c } .
{ ?a secx:strongerThan ?b } => { ?a secx:atLeast ?b } .
{ ?x secx:atLeast ?x } .   # reflexive (every level is at-least itself)
```

The **discharge rule** for a `gteq` constraint over a property dimension — *a method
`M` satisfies constraint `C` iff `M`'s asserted level for `C`'s dimension is `atLeast`
`C`'s required level*:

```n3
@prefix secx: <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .

# requiresUnlinkabilityScope : leftOperand -> dimension mapping (one fact per leftOperand)
secx:requiresUnlinkabilityScope secx:overDimension secx:UnlinkabilityScope .

{ ?c odrl:leftOperand  ?lo ;
     odrl:operator      odrl:gteq ;
     odrl:rightOperand  ?required .
  ?lo secx:overDimension ?dim .
  ?m  secx:hasProperty [ secx:property ?dim ; secx:level ?have ] .
  ?have secx:atLeast ?required .
} => { ?m secx:satisfies ?c } .
```

A method is **admissible** for a policy iff it satisfies **every** constraint of that
policy (a closed-world "no unsatisfied constraint" check — the same default-deny shape
as the trust-graph admission gate; note N3 is monotone, so the "satisfies every
constraint" universal is computed by the Rust side-condition over the materialised
`secx:satisfies` facts, exactly as the trust gate keeps freshness/holder-binding as
Rust side-conditions, *not* in-reasoner negation):

```text
admissible(M, P) :=  forall c in constraints(P):  (M secx:satisfies c)   # default-deny
```

The **assurance gate** is just another dimension with its own order
(`Proven ⊐ Claimed ⊐ Conjectured`) and the **same** discharge rule — so
`requiresAssurance gteq Proven` mechanically removes every unaudited sparq method from
the admissible set, with no special-casing. This is how sq-qhy4 enters the *data flow*,
not merely the prose.

#### 4.3.3 Worked end-to-end example

- **Methods annotated** (from §5a): `zk:poseidon2-rdfc10-v1` (string-canonical) is
  annotated `ZeroKnowledgeType ComputationalZK ; Soundness KnowledgeSound (assurance
  Claimed, auditStatus ExternalSignOffPending) ; PostQuantumForgery PQForgeable`
  (Schnorr/Baby-JubJub issuer signature is discrete-log ⇒ Shor-broken, so the **method as
  a whole** is `PQForgeable` — matching his `sec-prop:PostQuantumForgery=no` verdict for
  Schnorr/EdDSA); `SelectiveDisclosure SelectivelyDisclosable ; SingleUse Replayable ;
  UnlinkabilityScope PerPresentation` (hidden-issuer member built-but-not-yet-sound → at
  best `PerPresentation`, `assurance Claimed`); `bbs-2023` (VC cryptosuite, *source*
  layer) is annotated, **reusing his `sig-impl:` BBS+ verdict** (`Unlinkability=yes`),
  `UnlinkabilityScope CrossPresentation` — but **only with `secx:scope
  secx:SourceLayerOnly`** because sparq verifies the VC's proof *off-circuit at ingest*
  and does **not** re-prove it in a query circuit (the `zk:sourceCryptosuite`-is-provenance
  rule of the ZK config design §5.3), so the BBS unlinkability does **not** transfer to
  the query proof.
- **Alice's preference** (above): `UnlinkabilityScope gteq CrossPresentation ∧
  PostQuantumForgery eq PQForgeryResistant ∧ Assurance gteq Proven`.
- **Result:** the admissible set is **empty** for the current sparq estate — and that is
  the **correct, honest** answer. `string-canonical` fails on PQ-forgery (DL signature is
  Shor-broken), on unlinkability scope (`PerPresentation` < `CrossPresentation`), and on
  assurance (`Claimed` < `Proven`); `bbs-2023`'s `CrossPresentation` is `SourceLayerOnly`
  and a rule (§5a.2) refuses to let it satisfy a query-proof constraint. The ontology
  lets the system *say "no admissible proof"* rather than silently serve a non-conforming
  one — which is exactly the value: a user who demands cross-presentation-unlinkable, PQ,
  audited proofs gets a **principled refusal** today, and automatically gets admittance
  the day an audited PQ-unlinkable method is annotated. (Relax Alice's preference to
  `Assurance gteq Claimed ∧ drop PQ ∧ UnlinkabilityScope gteq PerPresentation` and
  `string-canonical` becomes admissible — demonstrating the gate is not vacuously empty.)

### 4.4 Why no term is redundant (the irreducibility argument)

Mirroring the trust ontology's §2.3.2 discipline — each dimension is shown to be
**non-derivable** from the others:

- **Unlinkability ≠ Anonymity.** A pseudonymous-but-unlinkable scheme (fresh pseudonym
  per presentation) and an anonymous-but-linkable scheme (same anonymous token reused)
  are both realisable, so the two dimensions are independent.
- **UnlinkabilityStrength ≠ UnlinkabilityScope.** Everlasting-vs-computational (strength)
  is orthogonal to per-vs-cross-presentation (scope): BBS is computational-but-cross;
  an information-theoretic single-show token is everlasting-but-per-presentation.
- **ZeroKnowledgeType ≠ Soundness.** A perfectly-ZK but unsound protocol and a sound-but-
  not-ZK protocol both exist; they constrain different parties (verifier-learns-nothing
  vs prover-cannot-cheat).
- **Soundness ≠ KnowledgeSoundness.** Soundness (no proof of a false statement) is weaker
  than knowledge-soundness (a valid proof implies the prover *knows* a witness);
  modelled as a level within `Soundness`, not a separate dimension.
- **Hiding ≠ Binding**, and neither is derivable from the other (the perfectly-hiding-XOR-
  perfectly-binding impossibility is *why* both are needed).
- **PostQuantumForgery ≠ Setup/Interactivity.** A transparent-setup scheme can be non-PQ
  (Bulletproofs, discrete-log) and a trusted-setup scheme can be PQ-flavoured — so PQ is
  not derivable from setup. (And his `PostQuantumForgery` ≠ `PostQuantumSnooping`: the
  forgeability of the *signature* under a CRQC is independent of the hiding-side
  harvest-now-decrypt-later risk — which is why keeping his two PQ dimensions is correct.)
- **SingleUse ≠ Unlinkability.** A nullifier gives single-use *without* anonymity
  (linkable nullifier) or *with* it (ZK nullifier) — independent of the unlinkability
  axis.
- **assurance/auditStatus ≠ any property** — it is the *epistemic basis*, deliberately
  orthogonal so it is stated once, not multiplied across every property (the key de-dup
  move).
- **`forPredicate`-style sugar:** a `secx:requiresPostQuantumForgery` leftOperand is
  **sugar** for the generic `requiresProperty` over the `PostQuantumForgery` dimension;
  the generic `secx:overDimension` mapping (one fact per leftOperand) is the single
  primitive, so a working group standardises only the generic
  `hasProperty`/`overDimension`/`atLeast` machinery, not one bespoke leftOperand per
  dimension. (This is the brief's "forShape-style irreducibility" applied: convenience
  leftOperands desugar to the generic rule.)

---

## 5. Integration points (where it plugs in)

### 5a. Annotating each sparq-zk method (ties to sq-1s2.5 + #919/sq-pkrl)

The `zk:` registry (`crates/sparq-zk/src/registry.rs`) already records, per credential,
`zk:scheme` (commitment method) and `zk:cryptosuite` (signature suite) as IRIs. The
security-properties ontology publishes a **static annotation graph** keyed on those same
method IRIs — i.e. the machine-readable form of `zk-configurable-commitment-security.md`
§4 (the per-method posture table). One annotation block per method IRI:

```turtle
@prefix zk:      <https://sparq.dev/ns/zk#> .
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .

zk:poseidon2-rdfc10-v1            # the string-canonical method (the only one E2E today)
  secx:hasProperty
    [ secx:property secx:ZeroKnowledgeType ; secx:level secx:ComputationalZK ;
      secx:assurance secx:Claimed ; secx:auditStatus secx:ExternalSignOffPending ;
      secx:auditEvidence <…/gap-register.md#CR-G1> ] ,
    [ secx:property secx:Soundness ; secx:level secx:KnowledgeSound ;
      secx:assurance secx:Claimed ; secx:auditStatus secx:ExternalSignOffPending ] ,
    [ secx:property secx:PostQuantumForgery ; secx:level secx:PQForgeable ;
      secx:assurance secx:Proven ;       # Schnorr/Baby-JubJub is DL ⇒ Shor-broken: settled
      sigimpl:justification "discrete-log issuer signature; recoverable by a CRQC" ] ,
    [ secx:property secx:SingleUse ; secx:level secx:Replayable ;
      secx:assurance secx:Proven ] ,     # no nullifier primitive exists ⇒ settled "replayable"
    [ secx:property secx:UnlinkabilityScope ; secx:level secx:PerPresentation ;
      secx:assurance secx:Claimed ; secx:auditStatus secx:ExternalSignOffPending ] .
```

Two design rules make this honest and non-duplicative:

1. **Negative properties are `Proven`, positive privacy properties are `Claimed`.**
   "PQ-forgeable" and "Replayable" are *settled facts about the construction* (a DL
   signature is Shor-broken; no nullifier exists), so they are `assurance secx:Proven`
   (matching his `sig-impl:` Schnorr/EdDSA `PostQuantumForgery=no` verdict). Every
   *positive* ZK/soundness/unlinkability claim about the sparq estate is `Claimed` +
   `ExternalSignOffPending` until sq-qhy4. This is the privacy-claims gate encoded in the
   data: the only `Proven` rows are the conservative/negative ones.
2. **Source-cryptosuite properties do NOT transfer to the query proof.** BBS
   `CrossPresentation` unlinkability is annotated on `bbs-2023` with
   `secx:scope secx:SourceLayerOnly` (a marker), and a rule **refuses** to let a
   source-layer property satisfy a *query-proof* constraint — encoding the ZK config
   design §5.3 "`zk:sourceCryptosuite` is provenance, not a re-verifiable in-proof
   property". Without this, the ontology would over-claim BBS unlinkability for sparq
   query proofs.

The annotation graph is **generated/maintained alongside** the method registry, and a
test asserts every production-selectable `zk:scheme` has an annotation block (the
machine-readable analogue of the G5 "every circuit has a gate-count baseline" gate).

### 5b. The trust-graph PoC admission gate (ties to sq-pfae)

The PoC's `admit.rs` gate (on `feat-trust-graph-poc`) currently verifies a CHECKED
issuer signature and binds the holder; it has **no** property-admissibility step. The
ontology adds **one** optional pre-admission check: given the requester's ODRL privacy
preference `P` and the method IRI the presented proof was produced under (recorded in
the `zk:` registry / the credential's `zk:scheme`), compute `admissible(method, P)` via
§4.3 and **deny if the method's annotations do not satisfy `P`** — *before* the existing
signature/holder checks (fail-closed, default-deny, consistent with the gate's existing
short-circuit). The check is **opt-in**: with no privacy preference expressed, behaviour
is byte-identical to the current gate (the trust-graph "strict additivity" rule).

This is the **machine-readable realisation of the ZK↔ODRL envelope** (sq-yh427): the
user's ODRL preference is discharged against the proof's claimed properties to yield an
admit/deny — *with the honest caveat that a `Claimed`-assurance method admitted under a
`requiresAssurance gteq Claimed` preference is admitted on an UNAUDITED basis* (sq-qhy4),
which the gate records in its decision provenance.

### 5c. MPC methods (ties to sparq-mpc)

`sparq-mpc` protocols are annotated the same way, with `secx:assumption
secx:HonestMajority` and `secx:SemiHonest` — so a preference requiring malicious-
security or dishonest-majority mechanically excludes them. This is the honest encoding of
"semi-honest only" in the property data, not just in prose.

---

## 6. Survey verdict table — existing literature + ontologies

Two independent survey passes (appendix §B has per-source detail + URLs) **agree** on the
verdicts below. **BUILD-ON** = reuse its IRIs; **ALIGN** = `skos:closeMatch`/`rdfs:seeAlso`
map our terms to it (prose-only or different shape); **REJECT** = wrong scope.

| Source | Reusable IRIs? | Verdict | One-line reason |
|---|---|---|---|
| **W3C DPV core (`w3id.org/dpv#`)** — `CryptographicMethods` tree | **Yes** (applied-technique layer) | **BUILD-ON** | the **sole** vocabulary with crypto-method IRIs: `dpv:Anonymisation`/`Pseudonymisation`/`SecureMultiPartyComputation`/`HomomorphicEncryption`/`DifferentialPrivacy`/`PostQuantumCryptography`/`ZeroKnowledgeAuthentication`. Stops at applied techniques — no general ZKP/unlinkability/anonymity-set terms (the gap we fill). Crypto lives in **core `dpv:`, NOT `dpv-tech`**. |
| **W3C Security Vocabulary (`w3id.org/security#`)** | **Yes** (proof structure) | **BUILD-ON** | `sec:Proof`, `sec:DataIntegrityProof`, `sec:cryptosuite`, `sec:verificationMethod` — the scaffold annotations hang on; **no** security-*property* terms. |
| **PROV-O (`prov:`)** | **Yes** (provenance) | **BUILD-ON/ALIGN** | proof provenance (`prov:wasGeneratedBy`/`wasDerivedFrom`) — his `sig-impl:` already uses `prov:wasDerivedFrom`; anti-anonymity by design (no anonymity terms). |
| **VC Data Integrity cryptosuite registry** | String IDs (not IRIs) | **ALIGN** | `bbs-2023` (SD + unlinkability), `ecdsa-sd-2023` (SD, explicitly NOT unlinkable), `eddsa-rdfc-2022` — the only standardised per-proof capability *labels*; unlinkability is spec prose, not a term. |
| **Pfitzmann–Hansen v0.34** | No | **ALIGN (mint, ground here)** | authoritative prose defs of anonymity/unlinkability/unobservability/pseudonymity/anonymity-set; no OWL encoding (only COPri imitates, no stable NS). |
| **ZKProof CR + IRTF/CFRG ZK terms** | No | **ALIGN (mint)** | soundness/ZK/knowledge-soundness/setup are PDF prose only — zero IRIs (the single biggest mint block). |
| **BBS (CFRG draft-10)** | No | **ALIGN (mint)** | unlinkability/SD/ZK prose; the everlasting/multi-show *taxonomy is academic, not in the draft* (honesty correction). |
| **NIST PQC (FIPS-203/204/205) + IANA COSE/JOSE** | No | **ALIGN (mint)** | security categories 1–5 are PDF prose; registries hold algorithm IDs, not security-property fields ⇒ `nistLevel` numeric parameter. |
| **IETF Privacy Pass (RFC 9576/77/78)** | No | **ALIGN (mint)** | four named unlinkability classes (origin-client/issuer-client/attester-origin/redemption-context), all prose; no registry. |
| **Differential privacy (ε,δ)** | Class only (`dpv:DifferentialPrivacy`) | **ALIGN (mint params)** | the DPV class is categorical; no ontology encodes ε/δ/sensitivity as typed properties ⇒ reuse the class, mint params. |
| **GDPRov (`w3id.org/GDPRov#`)** | Yes (`gdprov:AnonymityLevel`) | **ALIGN** | stable `AnonymityLevel`/`Anonymised`/`PseudoAnonymised` individuals — GDPR-surface scope; align `Anonymity` to it. |
| **ACP / WAC / VC·DID** | Yes (authz/infra) | **ALIGN** | `acp:vc` is a proof-gated-access hook; DID verification relationships — none model crypto *properties*. |
| **STIX 2.1 / UCO·CASE / OSCAL-800-53 / Herzog 2007 / Fenz 2009** | Yes (wrong scope) | **REJECT** | threat-intel / forensic / org-control granularity; crypto only as bare-string leaves; no per-proof properties. A 2026 survey (arXiv:2510.16610) explicitly excludes cryptology — the gap is structural. |

> **The load-bearing finding (both passes agree).** Almost no existing vocabulary models
> **per-proof cryptographic properties** at resolvable IRIs. The security-ontology
> literature is org/control/threat-level (REJECT); the crypto-terminology sources
> (Pfitzmann–Hansen, ZKProof, NIST, BBS, Privacy Pass) are **prose-only** (ALIGN by
> citation, mint IRIs). The **three real BUILD-ON anchors** are DPV core (applied-technique
> classes), the W3C Security Vocab (proof structure), and PROV-O (provenance) — none of
> which carries a security *property*. So the property IRIs are genuinely net-new
> (= the maintainer's `sec-prop:` minting was the right call, not reinvention), anchored to
> DPV/`sec:`/PROV-O where a near-match exists. This ontology is the **first machine-readable
> assembly of per-proof security-property terminology that otherwise exists only as prose.**

---

## 7. Critical evaluation of next steps (honest, prioritised)

### 7.1 Build first (low-risk, high-value, unblocked)

- **The ontology vocabulary itself** — the `secx:` term set as a Turtle file + a Rust
  constants module (mirroring `trust/vocab.rs` and `zk/registry.rs`). Pure data + the
  irreducibility doc. **Unblocked**, no audit dependency. This is the foundation.
- **The per-method annotation graph for the methods that exist today** — annotate
  `zk:poseidon2-rdfc10-v1` (the only E2E method) and the registered-but-unbuilt
  `dual-leaf`/`value-only` IRIs, *with the conservative `Claimed`/`ExternalSignOffPending`
  assurance and the `Proven`-only-for-negatives rule* (§5a). **Unblocked** (it records
  *claims*, not guarantees), and immediately useful for the §5b gate.
- **The N3 admissibility ruleset + a test harness** over `sparq-reason` — the
  `atLeast`/`satisfies`/`overDimension` rules of §4.3 with the §4.3.3 worked example as a
  golden test (admissible-set empty under the strict preference; non-empty under the
  relaxed one). **Unblocked** — it reasons over *annotations*, not over crypto.

### 7.2 Build next (depends on the trust PoC landing)

- **The opt-in admission-gate check (§5b)** — depends on `feat-trust-graph-poc` (PR #966)
  merging, since it edits `admit.rs`. Sequence behind it. Strictly additive (opt-in;
  off ⇒ unchanged gate).
- **The ODRL leftOperand registration in sparq-policy** — add the `secx:` leftOperand
  IRIs to the policy model's recognised set (they already parse as custom leftOperands;
  this is making them *first-class* + documented). Touches `sparq-policy` — coordinate
  with the PSS sibling's #890(b) envelope (sq-yh427).

### 7.3 Research-risky / blocked

- **Anything that turns a `Claimed` annotation into `Proven`** is **hard-blocked on the
  external audit sq-qhy4** — out of agent scope by definition. The ontology must *never*
  flip a sparq ZK property to `assurance Proven` without that sign-off; a test should
  assert no sparq ZK method carries `Proven` on a positive privacy/soundness property
  while sq-qhy4 is open (a machine guard on over-claiming).
- **The `SingleUse`/nullifier property is annotatable but the underlying primitive is
  ABSENT** (trust-graph §5.3(3)). The ontology can *require* it (and correctly deny
  everything), but making any method `SingleUse` needs the nullifier gadget built first
  (a separate, audit-gated crypto bead) — research-risky.
- **Cross-presentation unlinkability for sparq query proofs is an architecture problem,
  not a missing annotation** (trust-graph §5.4.3 item 5 — the clear-WebID holder binding
  leaks identity). The ontology can *say* `UnlinkabilityScope PerPresentation` truthfully,
  but raising it to `CrossPresentation` needs the §3.4 holder-binding redesign (sq-wvne),
  not an annotation edit. Honest: the ontology exposes the gap; it does not close it.

### 7.4 Genuinely novel vs incremental

- **Incremental:** the property *terms* themselves — unlinkability/PQ/ZK-type are
  textbook; minting IRIs for prose terminology is assembly, not invention.
- **Genuinely novel (the contribution):** (a) the **assurance/audit-status axis as a
  first-class, reasoner-consumable dimension** that makes "claimed-but-unaudited" a
  computable predicate — this is what lets sq-qhy4 govern the *data flow*, and is not
  something the surveyed vocabularies do; (b) the **ODRL-constraint → proof-admissibility
  reduction over RDF property annotations**, executed on a real N3 engine — a working
  bridge from a user privacy preference to a machine-checked admissible-proof set; (c) the
  **source-layer-vs-query-proof property-transfer rule** (§5a.2) that prevents over-
  claiming a source cryptosuite's unlinkability for the derived query proof. (a)+(b)+(c)
  are the defensible novelty; the term set is the table-stakes substrate.

### 7.5 The honest limitation (restate — non-negotiable)

**The ontology describes CLAIMED properties; it does not establish them.** Every positive
privacy/soundness annotation on a sparq method is only as trustworthy as the unaudited ZK
estate (sq-qhy4), and `sparq-mpc` is semi-honest-only. The value is precisely that the
ontology makes this gap **explicit, queryable, and enforceable** (the assurance axis +
the `Proven`-only-for-negatives rule + the no-`Proven`-while-sq-qhy4-open guard), so the
system can *refuse* rather than *over-serve*. A reviewer should attack exactly here: an
implementation that quietly annotated a sparq method `assurance Proven` on a positive ZK
property would be an honesty defect, and the §7.3 machine guard exists to catch it.

---

## 8. Phased plan (each phase a future bead under sq-0dksu)

Ordered; later phases depend on earlier; the privacy-promotion phase is hard-gated on
sq-qhy4.

1. **Phase 1 — ontology vocabulary** (`secx:` Turtle + Rust constants module +
   irreducibility note). Unblocked. *Deliverable:* the term set of §4.2 + §9 skeleton as
   a shipped vocab; a test that the level orders are acyclic + total where claimed.
2. **Phase 2 — N3 admissibility ruleset + golden test** over `sparq-reason` (the
   `atLeast`/`overDimension`/`satisfies` rules + the §4.3.3 worked example: empty under
   strict, non-empty under relaxed). Depends on Phase 1.
3. **Phase 3 — per-method annotation graph for sparq-zk** (annotate every
   production-selectable `zk:scheme`/`zk:cryptosuite`; the `Proven`-only-for-negatives
   rule; the source-layer-only marker; a completeness test that every selectable method
   has an annotation + a guard that no sparq ZK method is `Proven`-positive while sq-qhy4
   is open). Depends on Phase 1; ties to sq-1s2.5 / sq-pkrl.
4. **Phase 4 — ODRL leftOperand registration in sparq-policy** (first-class `secx:`
   leftOperands + docs; coordinate sq-yh427/#890(b) with PSS). Depends on Phase 1.
5. **Phase 5 — opt-in admission-gate check in the trust PoC** (`admit.rs` property pre-
   check; strict additivity; default-deny). **Depends on `feat-trust-graph-poc`/PR #966
   landing** AND Phases 2–4.
6. **Phase 6 — MPC method annotations** (`sparq-mpc` protocols with `HonestMajority`/
   `SemiHonest` assumptions). Depends on Phase 1.
7. **Phase 7 (HARD-GATED on sq-qhy4) — assurance promotion** — flip the relevant sparq ZK
   annotations from `Claimed`/`ExternalSignOffPending` toward `ExternallyAudited` *only*
   as the external auditor signs off specific properties. Out of agent scope until
   sq-qhy4 delivers; tracked, not started.

---

## 9. Turtle skeleton (impl builds directly on this)

A minimal, self-contained skeleton — the class/level hierarchy, the annotation
properties, the assurance axis, the external alignments, the ODRL profile, and the
level-order facts. Impl turns this into the shipped vocab + the per-method annotation
graph.

> Namespace note: `secx:` is the **same namespace as his `sec-prop:`**
> (`https://w3id.org/zkp-sparql/sec-prop#`) — one prefix in the skeleton for brevity. The
> **8 reused dimensions keep his exact `sec-prop:` IRIs** (`Unlinkability`,
> `PostQuantumForgery`, `PostQuantumSnooping`, `SourceCredentialDisclosure`,
> `SignatureTypeLeakage`, `ProofSizeLeakage`, `CircuitAudit`, `ValidityPeriodLeakage` —
> §3.1); the rest are the §3.3 extension, added under the same namespace.

```turtle
@prefix secx:    <https://w3id.org/zkp-sparql/sec-prop#> .   # his ns; reused + extension terms
@prefix sigimpl: <https://w3id.org/zkp-sparql/sig-impl#> .   # his reified-verdict pattern
@prefix dpv:     <https://w3id.org/dpv#> .                   # BUILD-ON: applied-technique classes
@prefix sec:     <https://w3id.org/security#> .              # BUILD-ON: proof structure
@prefix prov:    <http://www.w3.org/ns/prov#> .
@prefix skos:    <http://www.w3.org/2004/02/skos/core#> .
@prefix rdf:     <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl:     <http://www.w3.org/2002/07/owl#> .
@prefix odrl:    <http://www.w3.org/ns/odrl/2/> .
@prefix xsd:     <http://www.w3.org/2001/XMLSchema#> .

# === property dimensions (each a class; levels are individuals ordered by strongerThan) ===
secx:Property a owl:Class ; rdfs:comment "A security/privacy dimension of a proof/system." .
secx:Level    a owl:Class ; rdfs:comment "A value a property can hold; ordered by secx:strongerThan." .

# --- the 8 REUSED [his sec-prop:] dimensions (refined from boolean into ordered levels) ---
secx:Unlinkability              a owl:Class ; rdfs:subClassOf secx:Property .  # [his]
secx:UnlinkabilityScope    a owl:Class ; rdfs:subClassOf secx:Unlinkability .  # refinement
secx:UnlinkabilityStrength a owl:Class ; rdfs:subClassOf secx:Unlinkability .  # refinement
secx:PostQuantumForgery         a owl:Class ; rdfs:subClassOf secx:Property ;  # [his]
    skos:closeMatch dpv:PostQuantumCryptography .
secx:PostQuantumSnooping        a owl:Class ; rdfs:subClassOf secx:Property .  # [his]
secx:SourceCredentialDisclosure a owl:Class ; rdfs:subClassOf secx:Property .  # [his]
secx:SignatureTypeLeakage       a owl:Class ; rdfs:subClassOf secx:Property .  # [his]
secx:ProofSizeLeakage           a owl:Class ; rdfs:subClassOf secx:Property .  # [his]
secx:CircuitAudit               a owl:Class ; rdfs:subClassOf secx:Property .  # [his]
secx:ValidityPeriodLeakage      a owl:Class ; rdfs:subClassOf secx:Property .  # [his]

# --- the NEW [§3.3 delta] dimensions ---
secx:Anonymity             a owl:Class ; rdfs:subClassOf secx:Property ;
    rdfs:seeAlso <https://w3id.org/GDPRov#AnonymityLevel> .
secx:ZeroKnowledgeType     a owl:Class ; rdfs:subClassOf secx:Property ;
    skos:closeMatch dpv:ZeroKnowledgeAuthentication .   # DPV stops at ZK-auth; this is general
secx:Soundness             a owl:Class ; rdfs:subClassOf secx:Property .
secx:Completeness          a owl:Class ; rdfs:subClassOf secx:Property .
secx:Hiding                a owl:Class ; rdfs:subClassOf secx:Property .
secx:Binding               a owl:Class ; rdfs:subClassOf secx:Property .
secx:Assurance             a owl:Class ; rdfs:subClassOf secx:Property .   # the honesty axis (new)

# === levels (individuals) + orderings (strongerThan; transitively closed by N3 rule) ===
# unlinkability — two orthogonal sub-axes (scope x strength); levels minted, grounded in
# Pfitzmann-Hansen v0.34 (NOT the CFRG BBS draft — survey honesty note):
secx:CrossPresentation a secx:Level ; secx:strongerThan secx:PerPresentation .
secx:PerPresentation   a secx:Level ; secx:strongerThan secx:Linkable .
secx:Linkable          a secx:Level .
secx:EverlastingUnlinkable a secx:Level ; secx:strongerThan secx:ComputationalUnlinkable .
secx:ComputationalUnlinkable a secx:Level .

secx:PerfectZK        a secx:Level ; secx:strongerThan secx:StatisticalZK .
secx:StatisticalZK    a secx:Level ; secx:strongerThan secx:ComputationalZK .
secx:ComputationalZK  a secx:Level ; secx:strongerThan secx:NotZK .
secx:NotZK            a secx:Level .

secx:KnowledgeSound a secx:Level ; secx:strongerThan secx:Sound .
secx:Sound          a secx:Level ; secx:strongerThan secx:Unsound .
secx:Unsound        a secx:Level .

# his PostQuantumForgery (issuer-key forgeability under a CRQC): boolean + NIST-level param
secx:PQForgeryResistant a secx:Level ; secx:strongerThan secx:PQForgeable .
secx:PQForgeable        a secx:Level .
# his PostQuantumSnooping (harvest-now-decrypt-later on the hiding side):
secx:PQHiding    a secx:Level ; secx:strongerThan secx:PQRevealable .
secx:PQRevealable a secx:Level .

secx:Transparent           a secx:Level ; secx:strongerThan secx:UniversalTrustedSetup .
secx:UniversalTrustedSetup a secx:Level ; secx:strongerThan secx:PerCircuitTrustedSetup .
secx:PerCircuitTrustedSetup a secx:Level .

secx:SingleUse  a secx:Level ; secx:strongerThan secx:Replayable .   # nullifier-enforced
secx:Replayable a secx:Level .

secx:SelectivelyDisclosable a secx:Level ; secx:strongerThan secx:AllOrNothing .
secx:AllOrNothing           a secx:Level .

# his CircuitAudit (generator correctness — distinct from Soundness):
secx:MechanisedProof a secx:Level ; secx:strongerThan secx:ManualAudit .
secx:ManualAudit     a secx:Level ; secx:strongerThan secx:Unaudited .
secx:Unaudited       a secx:Level .

# the honesty axis, as a first-class ordered dimension:
secx:Proven      a secx:Level ; secx:strongerThan secx:Claimed .
secx:Claimed     a secx:Level ; secx:strongerThan secx:Conjectured .
secx:Conjectured a secx:Level .
secx:ExternallyAudited    a secx:Level ; secx:strongerThan secx:InternallyReviewed .
secx:InternallyReviewed   a secx:Level ; secx:strongerThan secx:Unreviewed .
secx:Unreviewed           a secx:Level .
secx:ExternalSignOffPending a secx:Level .   # the live sq-qhy4 state (≈ InternallyReviewed, audit pending)

# === the annotation shape (adopts his sig-impl:Assertion reified pattern) ===
secx:PropertyAssertion a owl:Class ; rdfs:subClassOf sigimpl:Assertion .  # generalises his (impl,prop) verdict node
secx:hasProperty   a owl:ObjectProperty ; rdfs:comment "Attach a PropertyAssertion to a method IRI." .
secx:property      a owl:ObjectProperty ; rdfs:domain secx:PropertyAssertion ; rdfs:range secx:Property .
secx:level         a owl:ObjectProperty ; rdfs:domain secx:PropertyAssertion ; rdfs:range secx:Level .
secx:assurance     a owl:ObjectProperty ; rdfs:domain secx:PropertyAssertion ; rdfs:range secx:Level .
secx:auditStatus   a owl:ObjectProperty ; rdfs:domain secx:PropertyAssertion ; rdfs:range secx:Level .
secx:assumption    a owl:ObjectProperty ; rdfs:domain secx:PropertyAssertion .   # e.g. secx:IssuerHonesty
secx:parameter     a owl:DatatypeProperty ; rdfs:domain secx:PropertyAssertion . # e.g. nistLevel, anonymitySet
secx:auditEvidence a owl:AnnotationProperty .                                     # rdfs:seeAlso to the audit doc
secx:scope         a owl:ObjectProperty ; rdfs:comment "secx:SourceLayerOnly marks a non-transferring source-cryptosuite property." .
# (sigimpl:justification + prov:wasDerivedFrom are inherited from his pattern, unchanged)

secx:IssuerHonesty a secx:Assumption . secx:HonestMajority a secx:Assumption .
secx:SemiHonest    a secx:Assumption . secx:DiscreteLog    a secx:Assumption .
secx:RandomOracle  a secx:Assumption .
secx:SourceLayerOnly a secx:Scope .

# === ODRL profile: declares the custom leftOperands (REQUIRED — survey §6) ===
# A policy that uses these must assert `odrl:profile <…/odrl-secprop-profile#>`.
<https://sparq.dev/ns/odrl-secprop-profile#> a odrl:Profile ;
    rdfs:label "sparq security-property ODRL profile" .
secx:requiresUnlinkabilityScope a owl:NamedIndividual, odrl:LeftOperand, skos:Concept ;
    rdfs:isDefinedBy <https://sparq.dev/ns/odrl-secprop-profile#> ;
    secx:overDimension secx:UnlinkabilityScope .
secx:requiresPostQuantumForgery a owl:NamedIndividual, odrl:LeftOperand, skos:Concept ;
    rdfs:isDefinedBy <https://sparq.dev/ns/odrl-secprop-profile#> ;
    secx:overDimension secx:PostQuantumForgery .
secx:requiresZeroKnowledge   a odrl:LeftOperand ; secx:overDimension secx:ZeroKnowledgeType .
secx:requiresSoundness       a odrl:LeftOperand ; secx:overDimension secx:Soundness .
secx:requiresSelectiveDisclosure a odrl:LeftOperand ; secx:overDimension secx:SelectiveDisclosure .
secx:requiresSingleUse       a odrl:LeftOperand ; secx:overDimension secx:SingleUse .
secx:requiresAssurance       a odrl:LeftOperand ; secx:overDimension secx:Assurance .
# (one leftOperand per requireable dimension; all reduce to the single overDimension rule;
#  declared in the profile so a conforming ODRL processor does not reject them)

# === external alignment anchors (BUILD-ON: DPV / sec: / PROV-O) ===
secx:ZeroKnowledgeType   skos:closeMatch dpv:ZeroKnowledgeAuthentication .
secx:SelectiveDisclosure rdfs:seeAlso    <https://www.w3.org/TR/vc-di-bbs/> .   # bbs-2023 cryptosuite
# annotated methods are sec:DataIntegrityProof-bearing; the method IRI is the sec:cryptosuite value.
```

The N3 admissibility rules (`strongerThan`-closure, `atLeast`, `overDimension`,
`satisfies`) of §4.3.2 ship alongside this skeleton as the runnable ruleset.

---

## 10. Open questions that genuinely need the maintainer

> **Maintainer decision (2026-06-20) — vendor, do not publicise.** The maintainer
> decided **not** to make the private `jeswr/sparql-zkp-ontologies` repo public.
> Instead its CI-validated, SHACL-shaped ontologies are **vendored verbatim into the
> sparq codebase** (with the license + attribution intact) at
> `crates/sparq-trust/ontologies/zkp-sparql/` — vendored from SHA
> `0fe80ea7d858de9f02bd29df29f6e50cdada14a0` — and the external repo is **archived**
> afterward. This **resolves open question #1 below** (release publicly? → **no,
> vendored**). The **`https://w3id.org/zkp-sparql/...` namespace is kept as-is**: w3id
> IRIs resolve via the permanent-identifier redirect, independent of the repo's
> visibility, so they remain stable under archive — no re-minting into a sparq-local
> namespace. The two remaining maintainer decisions (#2 assurance default, #3 DPV
> alignment depth) are tracked as GitHub issues **#1001** and **#1002** respectively.
> See the vendored `PROVENANCE.md` for the full provenance/attribution record.

1. ~~**Namespace + public release of `sparql-zkp-ontologies`.**~~ **RESOLVED
   (2026-06-20): NO — vendored, not publicised.** This design extends the maintainer's
   existing `https://w3id.org/zkp-sparql/sec-prop#` namespace. The original repo was
   private with placeholder IRIs; the maintainer chose neither option (a) "make it
   public" nor option (b) "mirror under a sparq-local namespace with `rdfs:seeAlso`",
   but a **third path: vendor the ontologies into sparq verbatim and keep the
   `w3id.org/zkp-sparql/` IRIs**. w3id IRIs are independent of the source repo's
   visibility (they redirect via the w3id permanent-identifier service), so they
   resolve and stay stable even after the external repo is archived — preserving the
   single-source benefit without publishing the repo. The vendored copy lives at
   `crates/sparq-trust/ontologies/zkp-sparql/`; this design **binds hard** against
   those IRIs.
2. **Assurance default → tracked as issue [#1001](https://github.com/sparq-org/sparq/issues/1001).**
   Should the *shipped default* admissibility policy require `assurance gteq Proven`
   (admit nothing from the unaudited estate — maximally conservative, but admits
   nothing) or `gteq Claimed` (admit on an explicitly-unaudited basis with the decision
   recorded)? This is a product/safety call, not a technical one. **Decision pending in
   #1001** (the SPARQ agent's recommendation there is **default `Claimed`**, promoting
   to `Proven` only after the external audit `sq-qhy4` or a real formal proof). The
   vocab extension (`sq-5oru9`) is **blocked** on this answer.
3. **DPV alignment depth → tracked as issue [#1002](https://github.com/sparq-org/sparq/issues/1002).**
   The survey confirmed DPV core (not `dpv-tech`) is the sole reusable crypto-method
   vocabulary, but it stops at applied techniques
   (`dpv:ZeroKnowledgeAuthentication`, `dpv:PostQuantumCryptography`) and has no per-proof
   property terms. Anchor each minted property to DPV with `skos:closeMatch` only (the
   plan), or push harder to get the per-proof terms *into* DPV (a DPVCG contribution)?
   The latter is more work but standardises the gap rather than mirroring it.
   **Decision pending in #1002** (the SPARQ agent's recommendation there is **Light** —
   `skos:closeMatch`/`rdfs:seeAlso` cross-reference only, no full regulation→requirement
   chain yet).
4. **Source-layer property transfer.** Confirm the §5a.2 rule (a source cryptosuite's
   unlinkability never satisfies a query-proof constraint) is the behaviour you want —
   it is conservative and correct given the off-circuit ingest, but it means a BBS-issued
   credential's cross-presentation unlinkability is *invisible* to the admissibility
   check for the query proof. Acceptable?
5. **Differential privacy.** Out of scope for v1 (no DP in the estate), or reserve the
   `(ε,δ)` `parameter` shape now for a future DP-over-aggregates story?

---

## 11. Verdict

The right thing to build is **not a fresh ontology — it is an extension of the
maintainer's own `sec-prop:` vocabulary** (his 8 properties reused; ~9 orthogonal
dimensions added) plus a **first-class assurance axis**, a **runnable N3 admissibility
reduction** over `sparq-reason`, a **published ODRL profile** declaring the `secx:requires…`
leftOperands, and a **per-method annotation graph** keyed on the existing `zk:scheme`/
`zk:cryptosuite` IRIs — with the **honesty machinery baked into the data model**
(`Proven`-only-for-negatives, `ExternalSignOffPending`, the no-`Proven`-while-sq-qhy4-open
guard, the source-layer-only non-transfer rule). The genuinely novel contribution is the
**assurance axis + ODRL→admissibility reduction + source-vs-query non-transfer rule**;
the term set is assembled from his existing vocabulary + prose-only prior art (DPV core,
the W3C Security Vocab, and PROV-O are the only external reuse). Build Phases 1–4 now
(unblocked); sequence Phase 5 behind the trust PoC; hard-gate Phase 7 on the external
audit (sq-qhy4). The ontology's whole value is that it makes the unaudited-claim gap
**explicit and enforceable** — it describes claimed properties; it does not, and must
never pretend to, establish them.

---

## Appendix A — maintainer prior-work survey (sources + verdicts)

Read verbatim (not inferred): `gh repo list jeswr`; `jeswr/sparql-zkp-ontologies`
(private) — all four vocab files (`sec-prop.yaml.ld`, `sig-impl.yaml.ld`,
`sec-req.yaml.ld`, `prov-ext.yaml.ld`) + README; `jeswr/lws-acp` (public) `docs/` +
`vocabs/` + `REQUIREMENTS.md`; `jeswr/ISWC2025-…-Zero-Knowledge-Query-over-Verifiable-Credentials`
`samplepaper.tex` (Wright, Shadbolt, Jun Zhao, Rui Zhao, Oxford);
`jeswr/sparql_noir_modular` / `jeswr/zkSPARQL-bench` / `jeswr/risc0-ed25519-zk-sparql`;
and the in-repo digests on the unmerged trust-graph branch (commits `ed2a16fe`,
`09da8188`, `62322c86`, `c715a3eb`, `fa112a59`).

**The key artefact — `jeswr/sparql-zkp-ontologies`** (companion to the ISWC 2025 paper,
namespace placeholder `https://w3id.org/zkp-sparql/`, CI-validated YAML-LD → SHACL →
Turtle): `sec-prop:` (8 `skos:Concept` properties — `Unlinkability`,
`SourceCredentialDisclosure`, `PostQuantumForgery`, `PostQuantumSnooping`,
`SignatureTypeLeakage`, `ProofSizeLeakage`, `CircuitAudit`, `ValidityPeriodLeakage` —
each with threat/defence/openQuestion/category/paperSection + `prov:wasDerivedFrom`);
`sig-impl:` (reified `sig-impl:Assertion` yes/no/partial verdicts per (scheme, property)
for BBS+/SD-JWT-VC/ed25519/ECDSA — e.g. BBS+ `Unlinkability=yes, PostQuantumForgery=no`;
ed25519 `Unlinkability=no` "deterministic, needs PoK lift"); `sec-req:` (eIDAS 2.0 / NIST
PQC / UK DVS `Requirement`s with deadlines, `pulls`-ing properties); `prov-ext:bibtexKey`.

**KEEP** (build on): `sec-prop:` 8 properties (wholesale); the `sig-impl:Assertion` reified
pattern; the `sec-req:`→`sec-prop:` `pulls` linkage; the INV-VL framing
(`zk-configurable-commitment-security.md`); the 10-term `trust:` ontology + `forShape`
irreducibility discipline; the adversarial `zk-comp`→`gap` reclassification. **REJECT** (with
reasons): the ISWC `.tex` skeleton (§3–7 empty placeholders — content is in the companion
repos); `risc0-ed25519-zk-sparql`/`circomkit-sparql` (superseded by Noir); the `lws-acp`
15-layer taxonomy as a unit (keep only Layer-0/Layer-4); `lws:allOf`/`lws:not` (collide
with shipped `acp:` terms); the unqualified "superset of ZKaps" framing (already corrected).
**Caveat:** the `sparql-zkp-ontologies` IRIs are private placeholders — see open question
§10.1. **Gaps his vocabulary lacks** (the §3.3 delta this design adds): soundness-type,
hiding/binding, trusted-vs-transparent setup, interactive-vs-NI, anonymity-set parameter,
single-use/nullifier, and the machine-reasonable assurance/audit-status axis.

## Appendix B — external literature/ontology survey (sources + verdicts)

Two independent passes; the consolidated verdict table is §6. Key sources + URLs:

- **W3C DPV** — `https://w3id.org/dpv#` (v2.x, W3C CG, actively maintained). The
  `dpv:CryptographicMethods` tree (in **core `dpv:`**, *not* a `dpv-tech` module —
  correcting the brief) gives `dpv:Anonymisation`, `dpv:Pseudonymisation` (+ subtypes),
  `dpv:SecureMultiPartyComputation`, `dpv:HomomorphicEncryption`, `dpv:DifferentialPrivacy`,
  `dpv:PostQuantumCryptography`, `dpv:ZeroKnowledgeAuthentication`,
  `dpv:TrustedExecutionEnvironment`. **ABSENT** (confirmed): general `ZeroKnowledgeProof`,
  `SelectiveDisclosure`, `Unlinkability`, `AnonymitySet`, ε/δ. **BUILD-ON.**
- **W3C Security Vocabulary** — `https://w3id.org/security#`: `sec:Proof`,
  `sec:DataIntegrityProof`, `sec:cryptosuite`, `sec:verificationMethod`, etc. Proof
  *structure*, no security *properties*. **BUILD-ON (structure).**
- **VC Data Integrity cryptosuites** — `w3.org/TR/vc-di-bbs` (`bbs-2023`: SD + documented
  unlinkability), `w3.org/TR/vc-di-ecdsa` (`ecdsa-sd-2023`: SD, *explicitly NOT*
  unlinkable; `ecdsa-rdfc-2019`; `eddsa-rdfc-2022`). String IDs, prose properties. **ALIGN.**
- **Pfitzmann–Hansen v0.34** (`dud.inf.tu-dresden.de/literatur/Anon_Terminology_v0.34.pdf`)
  — authoritative prose defs of anonymity/unlinkability/unobservability/pseudonymity/
  anonymity-set; no OWL encoding (only COPri imitates, no stable NS). **ALIGN (mint, ground).**
- **ZKProof Community Reference** (`zkproof.org`) + IRTF `draft-irtf-cfrg-sigma-protocols`
  — soundness/completeness/ZK/knowledge-soundness/setup, prose only. **ALIGN (mint).**
- **BBS** (`draft-irtf-cfrg-bbs-signatures-10`) — unlinkability/SD/ZK prose; the
  everlasting/multi-show *taxonomy is academic, not in the draft*. **ALIGN (mint).**
- **NIST PQC** — FIPS-203/204/205 + IR 8413 security categories 1–5 (prose); IANA
  COSE/JOSE registries hold algorithm IDs, no security-property fields. **ALIGN (mint param).**
- **IETF Privacy Pass** — RFC 9576 §3.3 names four unlinkability classes (origin-client /
  issuer-client / attester-origin / redemption-context), prose only. **ALIGN (mint).**
- **GDPRov** (`https://w3id.org/GDPRov#`) — `gdprov:AnonymityLevel` +
  `Anonymised`/`PseudoAnonymised` individuals (stable). **ALIGN** (anonymity).
- **PROV-O** (`http://www.w3.org/ns/prov#`) — proof provenance (`wasGeneratedBy`/
  `wasDerivedFrom`); anti-anonymity by design. **BUILD-ON/ALIGN.**
- **ODRL** (`http://www.w3.org/ns/odrl/2/`) — rightOperand takes any IRI free; custom
  leftOperand/operator MUST be declared in a published profile (`odrl:profile`).
  **BUILD-ON (bridge).** Precedents: ODRL Regulatory-Compliance Profile; 2025 Spatial-Axis
  Profile.
- **STIX 2.1 / UCO·CASE / NIST OSCAL-800-53 / Herzog 2007 / Fenz 2009** — org/control/
  threat/forensic level; crypto only as bare-string leaves; no per-proof properties (a
  2026 survey, arXiv:2510.16610, explicitly excludes cryptology). **REJECT (scope).**

**Load-bearing finding (both passes agree):** almost nothing models per-proof
cryptographic properties at resolvable IRIs; the three BUILD-ON anchors (DPV core, W3C
Security Vocab, PROV-O) carry no security *property* — so the property IRIs are genuinely
net-new, which is exactly why the maintainer minted `sec-prop:` in the first place.
