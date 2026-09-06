//! # `sparq-secprop-vocab` — the sparq `sec-prop:` extension vocabulary
//!
//! The IRIs of the sparq **security-properties ontology** as `const &str` Rust
//! constants, plus the canonical machine-readable form
//! (`ontologies/secprop-ext.ttl`) and the **single** TTL↔constant drift test that
//! pins the two together.
//!
//! ## Why this is its own crate (the sq-3705 de-duplication)
//!
//! The same `sec-prop:` IRI strings used to be declared **three** times —
//! `sparq-trust::secprop` (the declared owner), `sparq-policy::secprop` (the
//! `secx:overDimension` targets), and `sparq-zk::secprop` (the annotation graph's
//! `secx:` terms). `sparq-trust` **depends on** `sparq-zk`, so `sparq-zk` could not
//! import from it (a cycle), and `sparq-policy` could not either without dragging
//! the whole `sparq-zk`/`sparq-canon`/`sparq-shacl`/`sparq-reason` graph into its
//! lean one. Every copy existed because **there was no leaf below all three**.
//!
//! This crate is that leaf: it has **zero dependencies** (the Turtle parser the
//! drift tests need is a `dev-dependency`), so all three can take the edge without
//! a cycle and without growing their shipping graph by one byte. The
//! `include_str!` reads of `../../sparq-trust/…` that stood in for the
//! missing edge — and that broke `cargo package` file inclusion for `sparq-zk` —
//! are gone with it.
//!
//! ## What this vocabulary is — and is NOT
//!
//! It **records** a (method, property) → (level, assurance, audit-status, assumption)
//! claim and its **epistemic basis**; it is **NOT** a proof of any property. The
//! **assurance axis** ([`SECX_PROVEN`] ⊐ [`SECX_CLAIMED`] ⊐ [`SECX_CONJECTURED`]) is
//! the honesty mechanism: it is **one** axis, orthogonal to every property (stated
//! once, not multiplied across dimensions). The **default** assurance for a
//! sparq-asserted ZK property is [`SECX_CLAIMED`] (issue #1001 Option A) and the live
//! audit status is [`SECX_EXTERNAL_SIGN_OFF_PENDING`] — sparq's ZK estate is
//! research-grade and **externally UNAUDITED** (`sq-qhy4`, pending an external
//! accredited-cryptographer sign-off). **No sparq ZK method may be labelled
//! [`SECX_PROVEN`] while `sq-qhy4` is open.**
//!
//! ## Extends, does not fork
//!
//! Reused terms keep the vendored `sec-prop:` IRIs of the ISWC 2025 ZKP-SPARQL
//! companion ontology (`sec-prop:Unlinkability`, …); new terms are minted under the
//! **same** namespace (`https://w3id.org/zkp-sparql/sec-prop#`) and named with a
//! `SECX_` constant prefix so the his-vs-new split is visible in the Rust as it is in
//! the prose (`secx:` is a prose-only sub-prefix of `sec-prop:`, not a distinct
//! namespace). Design record `research/security-properties-ontology-design.md` §4.1;
//! provenance + attribution for the vendored namespace this extends:
//! `ontologies/PROVENANCE.md` and `crates/sparq-trust/ontologies/zkp-sparql/`.
//!
//! ## DPV alignment — Light (issue #1002 Option 2)
//!
//! New property classes cross-reference W3C DPV `CryptographicMethods` IRIs with
//! `skos:closeMatch` where a near-match exists (in the Turtle); the full
//! regulation→requirement chain is deliberately not modelled here.
//!
//! ## Lean by construction
//!
//! Plain `const &str` data — no dependency, no runtime cost, nothing to feature-gate.
//! Consumers keep their own default-OFF gates (`sparq-trust`'s `secprop-vocab`,
//! `sparq-policy`'s `secprop-leftoperands`, `sparq-zk`'s `secprop-annotations`), so
//! their lean default builds are byte-unchanged.
//!
//! [OPUS-5] sq-3705 (extracted from `sparq-trust::secprop`, bead sq-5oru9, epic
//! sq-0dksu). 🤖 SPARQ agent — security-properties ontology.

/// The `sec-prop:` namespace base (the vendored ZKP-SPARQL namespace this extends —
/// a real `w3id.org` permanent identifier, NOT a placeholder).
pub const SEC_PROP_NS: &str = "https://w3id.org/zkp-sparql/sec-prop#";

// ── reused [his] property classes (the vendored eight; IRIs preserved) ───────

/// `sec-prop:Unlinkability` — the vendored unlinkability property (refined below
/// into the [`SECX_UNLINKABILITY_STRENGTH`] × [`SECX_UNLINKABILITY_SCOPE`] sub-axes,
/// both `rdfs:subPropertyOf` this IRI).
pub const SEC_PROP_UNLINKABILITY: &str = "https://w3id.org/zkp-sparql/sec-prop#Unlinkability";
/// `sec-prop:SecurityProperty` — the vendored superclass every dimension instantiates.
pub const SEC_PROP_SECURITY_PROPERTY: &str =
    "https://w3id.org/zkp-sparql/sec-prop#SecurityProperty";

// ── the annotation shape (§4.2.2) ────────────────────────────────────────────

/// `secx:PropertyAssertion` — the reified (method, property)→(level, assurance,
/// audit-status, assumption, evidence) node (`rdfs:subClassOf sig-impl:Assertion`).
pub const SECX_PROPERTY_ASSERTION: &str = "https://w3id.org/zkp-sparql/sec-prop#PropertyAssertion";
/// `secx:hasProperty` — attaches a [`SECX_PROPERTY_ASSERTION`] to a method IRI.
pub const SECX_HAS_PROPERTY: &str = "https://w3id.org/zkp-sparql/sec-prop#hasProperty";
/// `secx:property` — the dimension the assertion is about.
pub const SECX_PROPERTY: &str = "https://w3id.org/zkp-sparql/sec-prop#property";
/// `secx:level` — the level/class held within the asserted dimension.
pub const SECX_LEVEL: &str = "https://w3id.org/zkp-sparql/sec-prop#level";
/// `secx:parameter` — an optional numeric parameter (`nistLevel`, `anonymitySet`).
pub const SECX_PARAMETER: &str = "https://w3id.org/zkp-sparql/sec-prop#parameter";
/// `secx:assurance` — the epistemic-basis axis of the claim.
pub const SECX_ASSURANCE: &str = "https://w3id.org/zkp-sparql/sec-prop#assurance";
/// `secx:auditStatus` — the independent-review state of the claim.
pub const SECX_AUDIT_STATUS: &str = "https://w3id.org/zkp-sparql/sec-prop#auditStatus";
/// `secx:assumption` — the assumption the claim rests on.
pub const SECX_ASSUMPTION: &str = "https://w3id.org/zkp-sparql/sec-prop#assumption";
/// `secx:auditEvidence` — `rdfs:seeAlso` to the audit doc / gap-register row.
pub const SECX_AUDIT_EVIDENCE: &str = "https://w3id.org/zkp-sparql/sec-prop#auditEvidence";
/// `secx:scope` — the layer an assertion is realised at ([`SECX_QUERY_PROOF_LAYER`]
/// — the default when omitted — vs [`SECX_SOURCE_LAYER_ONLY`]). A source-layer-only
/// property must NOT satisfy a query-proof constraint (the §5a non-transfer rule).
pub const SECX_SCOPE: &str = "https://w3id.org/zkp-sparql/sec-prop#scope";

// ── the scope axis (§5a rule 2 — source-layer non-transfer marker) ────────────

/// `secx:PropertyScope` — the scope-axis class.
pub const SECX_PROPERTY_SCOPE: &str = "https://w3id.org/zkp-sparql/sec-prop#PropertyScope";
/// `secx:QueryProofLayer` — re-verifiable in the sparq query proof (the DEFAULT
/// scope when [`SECX_SCOPE`] is omitted).
pub const SECX_QUERY_PROOF_LAYER: &str = "https://w3id.org/zkp-sparql/sec-prop#QueryProofLayer";
/// `secx:SourceLayerOnly` — a property of the SOURCE credential's cryptosuite that
/// does NOT transfer to the query proof (`zk:sourceCryptosuite` is provenance, not
/// an in-proof property; design §5.3).
pub const SECX_SOURCE_LAYER_ONLY: &str = "https://w3id.org/zkp-sparql/sec-prop#SourceLayerOnly";

// ── the assurance axis (§4.2.2): Proven ⊐ Claimed ⊐ Conjectured ──────────────

/// `secx:AssuranceLevel` — the assurance-axis class.
pub const SECX_ASSURANCE_LEVEL: &str = "https://w3id.org/zkp-sparql/sec-prop#AssuranceLevel";
/// `secx:Proven` — backed by a formal proof or an external audit (incl. a provable
/// NEGATIVE result). **NOT applicable to a sparq ZK property while `sq-qhy4` is open.**
pub const SECX_PROVEN: &str = "https://w3id.org/zkp-sparql/sec-prop#Proven";
/// `secx:Claimed` — asserted, not independently verified. **The DEFAULT** for a
/// sparq ZK property (#1001 Option A).
pub const SECX_CLAIMED: &str = "https://w3id.org/zkp-sparql/sec-prop#Claimed";
/// `secx:Conjectured` — believed plausible but not established to the `Claimed` bar.
pub const SECX_CONJECTURED: &str = "https://w3id.org/zkp-sparql/sec-prop#Conjectured";

/// `secx:AuditStatus` — the audit-status class.
pub const SECX_AUDIT_STATUS_CLASS: &str = "https://w3id.org/zkp-sparql/sec-prop#AuditStatus";
/// `secx:ExternallyAudited` — reviewed by an external accredited party.
pub const SECX_EXTERNALLY_AUDITED: &str = "https://w3id.org/zkp-sparql/sec-prop#ExternallyAudited";
/// `secx:InternallyReviewed` — reviewed internally only.
pub const SECX_INTERNALLY_REVIEWED: &str =
    "https://w3id.org/zkp-sparql/sec-prop#InternallyReviewed";
/// `secx:Unreviewed` — not reviewed.
pub const SECX_UNREVIEWED: &str = "https://w3id.org/zkp-sparql/sec-prop#Unreviewed";
/// `secx:ExternalSignOffPending` — external audit underway/pending. **The live state
/// of the sparq ZK estate (`sq-qhy4`).**
pub const SECX_EXTERNAL_SIGN_OFF_PENDING: &str =
    "https://w3id.org/zkp-sparql/sec-prop#ExternalSignOffPending";

// ── assumptions (§4.2.3) ─────────────────────────────────────────────────────

/// `secx:Assumption` — the assumption class.
pub const SECX_ASSUMPTION_CLASS: &str = "https://w3id.org/zkp-sparql/sec-prop#Assumption";
/// `secx:IssuerHonesty` — the issuer-honesty precondition (the dual-leaf INV-VL case).
pub const SECX_ISSUER_HONESTY: &str = "https://w3id.org/zkp-sparql/sec-prop#IssuerHonesty";
/// `secx:DiscreteLog` — discrete-log hardness.
pub const SECX_DISCRETE_LOG: &str = "https://w3id.org/zkp-sparql/sec-prop#DiscreteLog";
/// `secx:RandomOracle` — the random-oracle model.
pub const SECX_RANDOM_ORACLE: &str = "https://w3id.org/zkp-sparql/sec-prop#RandomOracle";
/// `secx:HonestMajority` — the MPC honest-majority assumption.
pub const SECX_HONEST_MAJORITY: &str = "https://w3id.org/zkp-sparql/sec-prop#HonestMajority";
/// `secx:SemiHonest` — the MPC semi-honest-adversary assumption (sparq-mpc's model).
pub const SECX_SEMI_HONEST: &str = "https://w3id.org/zkp-sparql/sec-prop#SemiHonest";

// ── [his] Unlinkability refinement (strength × scope) ─────────────────────────

/// `secx:UnlinkabilityStrength` — `rdfs:subPropertyOf sec-prop:Unlinkability`.
pub const SECX_UNLINKABILITY_STRENGTH: &str =
    "https://w3id.org/zkp-sparql/sec-prop#UnlinkabilityStrength";
/// `secx:EverlastingUnlinkable` — the stronger strength level.
pub const SECX_EVERLASTING_UNLINKABLE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#EverlastingUnlinkable";
/// `secx:ComputationalUnlinkable` — the weaker strength level.
pub const SECX_COMPUTATIONAL_UNLINKABLE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#ComputationalUnlinkable";
/// `secx:UnlinkabilityScope` — `rdfs:subPropertyOf sec-prop:Unlinkability`.
pub const SECX_UNLINKABILITY_SCOPE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#UnlinkabilityScope";
/// `secx:CrossPresentation` — multi-show scope (strongest).
pub const SECX_CROSS_PRESENTATION: &str = "https://w3id.org/zkp-sparql/sec-prop#CrossPresentation";
/// `secx:PerPresentation` — single-show scope.
pub const SECX_PER_PRESENTATION: &str = "https://w3id.org/zkp-sparql/sec-prop#PerPresentation";
/// `secx:Linkable` — no unlinkability scope (weakest).
pub const SECX_LINKABLE: &str = "https://w3id.org/zkp-sparql/sec-prop#Linkable";

// ── [his] PostQuantumForgery levels + the nistLevel parameter ─────────────────

/// `sec-prop:PostQuantumForgery` — the vendored post-quantum-forgery dimension
/// (paper §7.7 #3). The IRI, label and `sec-prop:SecurityProperty` type are the
/// original's; `secprop-ext.ttl` re-asserts them so the dimension is a declared
/// subject there — the precondition for retiring the vendored-dimension exemption
/// list in the cross-crate drift guards (`sq-mgxz8`).
pub const SEC_PROP_POST_QUANTUM_FORGERY: &str =
    "https://w3id.org/zkp-sparql/sec-prop#PostQuantumForgery";
/// `secx:PQForgeryResistant`.
pub const SECX_PQ_FORGERY_RESISTANT: &str =
    "https://w3id.org/zkp-sparql/sec-prop#PQForgeryResistant";
/// `secx:PQForgeable`.
pub const SECX_PQ_FORGEABLE: &str = "https://w3id.org/zkp-sparql/sec-prop#PQForgeable";
/// `secx:nistLevel` — the FIPS 203/204/205 security category 1–5 (a parameter value).
pub const SECX_NIST_LEVEL: &str = "https://w3id.org/zkp-sparql/sec-prop#nistLevel";

// ── [his] PostQuantumSnooping levels ──────────────────────────────────────────

/// `sec-prop:PostQuantumSnooping` — the vendored harvest-now-decrypt-later dimension
/// (paper §7.7 #4); the PQ-time slice of the general [`SECX_HIDING`] axis.
pub const SEC_PROP_POST_QUANTUM_SNOOPING: &str =
    "https://w3id.org/zkp-sparql/sec-prop#PostQuantumSnooping";
/// `secx:PQHiding`.
pub const SECX_PQ_HIDING: &str = "https://w3id.org/zkp-sparql/sec-prop#PQHiding";
/// `secx:PQRevealable`.
pub const SECX_PQ_REVEALABLE: &str = "https://w3id.org/zkp-sparql/sec-prop#PQRevealable";

// ── [his] SourceCredentialDisclosure levels ───────────────────────────────────

/// `secx:NoIssuerDisclosure` (strongest).
pub const SECX_NO_ISSUER_DISCLOSURE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#NoIssuerDisclosure";
/// `secx:IssuerSetDisclosure` (his Merkle-over-dataset default).
pub const SECX_ISSUER_SET_DISCLOSURE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#IssuerSetDisclosure";
/// `secx:FullSourceDisclosure` (weakest).
pub const SECX_FULL_SOURCE_DISCLOSURE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#FullSourceDisclosure";

// ── [his] SignatureTypeLeakage levels ─────────────────────────────────────────

/// `sec-prop:SignatureTypeLeakage` — the vendored signature-type-leakage dimension
/// (paper §7.7 #5).
pub const SEC_PROP_SIGNATURE_TYPE_LEAKAGE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#SignatureTypeLeakage";
/// `secx:SchemeHidden`.
pub const SECX_SCHEME_HIDDEN: &str = "https://w3id.org/zkp-sparql/sec-prop#SchemeHidden";
/// `secx:SchemeRevealed`.
pub const SECX_SCHEME_REVEALED: &str = "https://w3id.org/zkp-sparql/sec-prop#SchemeRevealed";

// ── [his] ProofSizeLeakage levels ─────────────────────────────────────────────

/// `secx:FixedSize`.
pub const SECX_FIXED_SIZE: &str = "https://w3id.org/zkp-sparql/sec-prop#FixedSize";
/// `secx:StructureLeaking`.
pub const SECX_STRUCTURE_LEAKING: &str = "https://w3id.org/zkp-sparql/sec-prop#StructureLeaking";

// ── [his] CircuitAudit levels (distinct from the new Soundness dimension) ─────

/// `secx:MechanisedProof` (strongest).
pub const SECX_MECHANISED_PROOF: &str = "https://w3id.org/zkp-sparql/sec-prop#MechanisedProof";
/// `secx:ManualAudit`.
pub const SECX_MANUAL_AUDIT: &str = "https://w3id.org/zkp-sparql/sec-prop#ManualAudit";
/// `secx:Unaudited` (weakest).
pub const SECX_UNAUDITED: &str = "https://w3id.org/zkp-sparql/sec-prop#Unaudited";

// ── [his] ValidityPeriodLeakage levels ────────────────────────────────────────

/// `secx:ValidityHidden`.
pub const SECX_VALIDITY_HIDDEN: &str = "https://w3id.org/zkp-sparql/sec-prop#ValidityHidden";
/// `secx:ValidityRevealed`.
pub const SECX_VALIDITY_REVEALED: &str = "https://w3id.org/zkp-sparql/sec-prop#ValidityRevealed";

// ── [new] orthogonal proof-system dimensions (§3.3 delta) ─────────────────────

/// `secx:ZeroKnowledgeType` dimension.
pub const SECX_ZERO_KNOWLEDGE_TYPE: &str = "https://w3id.org/zkp-sparql/sec-prop#ZeroKnowledgeType";
/// `secx:PerfectZK`.
pub const SECX_PERFECT_ZK: &str = "https://w3id.org/zkp-sparql/sec-prop#PerfectZK";
/// `secx:StatisticalZK`.
pub const SECX_STATISTICAL_ZK: &str = "https://w3id.org/zkp-sparql/sec-prop#StatisticalZK";
/// `secx:ComputationalZK`.
pub const SECX_COMPUTATIONAL_ZK: &str = "https://w3id.org/zkp-sparql/sec-prop#ComputationalZK";
/// `secx:NotZK`.
pub const SECX_NOT_ZK: &str = "https://w3id.org/zkp-sparql/sec-prop#NotZK";

/// `secx:Soundness` dimension (distinct from his `CircuitAudit`).
pub const SECX_SOUNDNESS: &str = "https://w3id.org/zkp-sparql/sec-prop#Soundness";
/// `secx:KnowledgeSound`.
pub const SECX_KNOWLEDGE_SOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#KnowledgeSound";
/// `secx:Sound`.
pub const SECX_SOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#Sound";
/// `secx:Unsound`.
pub const SECX_UNSOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#Unsound";
/// `secx:StatisticalSoundness`.
pub const SECX_STATISTICAL_SOUNDNESS: &str =
    "https://w3id.org/zkp-sparql/sec-prop#StatisticalSoundness";
/// `secx:ComputationalSoundness`.
pub const SECX_COMPUTATIONAL_SOUNDNESS: &str =
    "https://w3id.org/zkp-sparql/sec-prop#ComputationalSoundness";

/// `secx:Completeness` dimension.
pub const SECX_COMPLETENESS: &str = "https://w3id.org/zkp-sparql/sec-prop#Completeness";
/// `secx:Complete`.
pub const SECX_COMPLETE: &str = "https://w3id.org/zkp-sparql/sec-prop#Complete";
/// `secx:Incomplete`.
pub const SECX_INCOMPLETE: &str = "https://w3id.org/zkp-sparql/sec-prop#Incomplete";

/// `secx:Hiding` (commitments) dimension.
pub const SECX_HIDING: &str = "https://w3id.org/zkp-sparql/sec-prop#Hiding";
/// `secx:PerfectHiding`.
pub const SECX_PERFECT_HIDING: &str = "https://w3id.org/zkp-sparql/sec-prop#PerfectHiding";
/// `secx:StatisticalHiding`.
pub const SECX_STATISTICAL_HIDING: &str = "https://w3id.org/zkp-sparql/sec-prop#StatisticalHiding";
/// `secx:ComputationalHiding`.
pub const SECX_COMPUTATIONAL_HIDING: &str =
    "https://w3id.org/zkp-sparql/sec-prop#ComputationalHiding";
/// `secx:NotHiding`.
pub const SECX_NOT_HIDING: &str = "https://w3id.org/zkp-sparql/sec-prop#NotHiding";

/// `secx:Binding` (commitments) dimension (dual of [`SECX_HIDING`]).
pub const SECX_BINDING: &str = "https://w3id.org/zkp-sparql/sec-prop#Binding";
/// `secx:PerfectBinding`.
pub const SECX_PERFECT_BINDING: &str = "https://w3id.org/zkp-sparql/sec-prop#PerfectBinding";
/// `secx:StatisticalBinding`.
pub const SECX_STATISTICAL_BINDING: &str =
    "https://w3id.org/zkp-sparql/sec-prop#StatisticalBinding";
/// `secx:ComputationalBinding`.
pub const SECX_COMPUTATIONAL_BINDING: &str =
    "https://w3id.org/zkp-sparql/sec-prop#ComputationalBinding";
/// `secx:NotBinding`.
pub const SECX_NOT_BINDING: &str = "https://w3id.org/zkp-sparql/sec-prop#NotBinding";

/// `secx:Anonymity` dimension.
pub const SECX_ANONYMITY: &str = "https://w3id.org/zkp-sparql/sec-prop#Anonymity";
/// `secx:Anonymous`.
pub const SECX_ANONYMOUS: &str = "https://w3id.org/zkp-sparql/sec-prop#Anonymous";
/// `secx:Pseudonymous`.
pub const SECX_PSEUDONYMOUS: &str = "https://w3id.org/zkp-sparql/sec-prop#Pseudonymous";
/// `secx:Identified` — the trust-graph §3.4 clear-WebID path level.
pub const SECX_IDENTIFIED: &str = "https://w3id.org/zkp-sparql/sec-prop#Identified";
/// `secx:anonymitySet` — the anonymity-set-size parameter.
pub const SECX_ANONYMITY_SET: &str = "https://w3id.org/zkp-sparql/sec-prop#anonymitySet";

/// `secx:Setup` dimension.
pub const SECX_SETUP: &str = "https://w3id.org/zkp-sparql/sec-prop#Setup";
/// `secx:Transparent`.
pub const SECX_TRANSPARENT: &str = "https://w3id.org/zkp-sparql/sec-prop#Transparent";
/// `secx:UniversalTrustedSetup`.
pub const SECX_UNIVERSAL_TRUSTED_SETUP: &str =
    "https://w3id.org/zkp-sparql/sec-prop#UniversalTrustedSetup";
/// `secx:PerCircuitTrustedSetup`.
pub const SECX_PER_CIRCUIT_TRUSTED_SETUP: &str =
    "https://w3id.org/zkp-sparql/sec-prop#PerCircuitTrustedSetup";

/// `secx:Interactivity` dimension.
pub const SECX_INTERACTIVITY: &str = "https://w3id.org/zkp-sparql/sec-prop#Interactivity";
/// `secx:NonInteractive`.
pub const SECX_NON_INTERACTIVE: &str = "https://w3id.org/zkp-sparql/sec-prop#NonInteractive";
/// `secx:Interactive`.
pub const SECX_INTERACTIVE: &str = "https://w3id.org/zkp-sparql/sec-prop#Interactive";

/// `secx:SelectiveDisclosure` dimension.
pub const SECX_SELECTIVE_DISCLOSURE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#SelectiveDisclosure";
/// `secx:SelectivelyDisclosable`.
pub const SECX_SELECTIVELY_DISCLOSABLE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#SelectivelyDisclosable";
/// `secx:AllOrNothing`.
pub const SECX_ALL_OR_NOTHING: &str = "https://w3id.org/zkp-sparql/sec-prop#AllOrNothing";

/// `secx:SingleUse` dimension (the anti-replay primitive; every sparq method is
/// [`SECX_REPLAYABLE`] today — honest).
pub const SECX_SINGLE_USE: &str = "https://w3id.org/zkp-sparql/sec-prop#SingleUse";
/// `secx:SingleUseEnforced` (nullifier-enforced).
pub const SECX_SINGLE_USE_ENFORCED: &str = "https://w3id.org/zkp-sparql/sec-prop#SingleUseEnforced";
/// `secx:Replayable`.
pub const SECX_REPLAYABLE: &str = "https://w3id.org/zkp-sparql/sec-prop#Replayable";

/// Every `secx:`/`sec-prop:` IRI this module declares, in declaration order. The
/// single source of truth the `tests` drift-check iterates — keeping the published
/// `secprop-ext.ttl` and these constants from diverging (the `sparq_trust::vocab`
/// discipline).
pub const ALL_SECPROP_IRIS: &[&str] = &[
    // reused [his] classes
    SEC_PROP_UNLINKABILITY,
    SEC_PROP_SECURITY_PROPERTY,
    // annotation shape
    SECX_PROPERTY_ASSERTION,
    SECX_HAS_PROPERTY,
    SECX_PROPERTY,
    SECX_LEVEL,
    SECX_PARAMETER,
    SECX_ASSURANCE,
    SECX_AUDIT_STATUS,
    SECX_ASSUMPTION,
    SECX_AUDIT_EVIDENCE,
    SECX_SCOPE,
    // scope axis
    SECX_PROPERTY_SCOPE,
    SECX_QUERY_PROOF_LAYER,
    SECX_SOURCE_LAYER_ONLY,
    // assurance axis
    SECX_ASSURANCE_LEVEL,
    SECX_PROVEN,
    SECX_CLAIMED,
    SECX_CONJECTURED,
    SECX_AUDIT_STATUS_CLASS,
    SECX_EXTERNALLY_AUDITED,
    SECX_INTERNALLY_REVIEWED,
    SECX_UNREVIEWED,
    SECX_EXTERNAL_SIGN_OFF_PENDING,
    // assumptions
    SECX_ASSUMPTION_CLASS,
    SECX_ISSUER_HONESTY,
    SECX_DISCRETE_LOG,
    SECX_RANDOM_ORACLE,
    SECX_HONEST_MAJORITY,
    SECX_SEMI_HONEST,
    // [his] Unlinkability refinement
    SECX_UNLINKABILITY_STRENGTH,
    SECX_EVERLASTING_UNLINKABLE,
    SECX_COMPUTATIONAL_UNLINKABLE,
    SECX_UNLINKABILITY_SCOPE,
    SECX_CROSS_PRESENTATION,
    SECX_PER_PRESENTATION,
    SECX_LINKABLE,
    // [his] PostQuantumForgery
    SEC_PROP_POST_QUANTUM_FORGERY,
    SECX_PQ_FORGERY_RESISTANT,
    SECX_PQ_FORGEABLE,
    SECX_NIST_LEVEL,
    // [his] PostQuantumSnooping
    SEC_PROP_POST_QUANTUM_SNOOPING,
    SECX_PQ_HIDING,
    SECX_PQ_REVEALABLE,
    // [his] SourceCredentialDisclosure
    SECX_NO_ISSUER_DISCLOSURE,
    SECX_ISSUER_SET_DISCLOSURE,
    SECX_FULL_SOURCE_DISCLOSURE,
    // [his] SignatureTypeLeakage
    SEC_PROP_SIGNATURE_TYPE_LEAKAGE,
    SECX_SCHEME_HIDDEN,
    SECX_SCHEME_REVEALED,
    // [his] ProofSizeLeakage
    SECX_FIXED_SIZE,
    SECX_STRUCTURE_LEAKING,
    // [his] CircuitAudit
    SECX_MECHANISED_PROOF,
    SECX_MANUAL_AUDIT,
    SECX_UNAUDITED,
    // [his] ValidityPeriodLeakage
    SECX_VALIDITY_HIDDEN,
    SECX_VALIDITY_REVEALED,
    // [new] ZeroKnowledgeType
    SECX_ZERO_KNOWLEDGE_TYPE,
    SECX_PERFECT_ZK,
    SECX_STATISTICAL_ZK,
    SECX_COMPUTATIONAL_ZK,
    SECX_NOT_ZK,
    // [new] Soundness
    SECX_SOUNDNESS,
    SECX_KNOWLEDGE_SOUND,
    SECX_SOUND,
    SECX_UNSOUND,
    SECX_STATISTICAL_SOUNDNESS,
    SECX_COMPUTATIONAL_SOUNDNESS,
    // [new] Completeness
    SECX_COMPLETENESS,
    SECX_COMPLETE,
    SECX_INCOMPLETE,
    // [new] Hiding
    SECX_HIDING,
    SECX_PERFECT_HIDING,
    SECX_STATISTICAL_HIDING,
    SECX_COMPUTATIONAL_HIDING,
    SECX_NOT_HIDING,
    // [new] Binding
    SECX_BINDING,
    SECX_PERFECT_BINDING,
    SECX_STATISTICAL_BINDING,
    SECX_COMPUTATIONAL_BINDING,
    SECX_NOT_BINDING,
    // [new] Anonymity
    SECX_ANONYMITY,
    SECX_ANONYMOUS,
    SECX_PSEUDONYMOUS,
    SECX_IDENTIFIED,
    SECX_ANONYMITY_SET,
    // [new] Setup
    SECX_SETUP,
    SECX_TRANSPARENT,
    SECX_UNIVERSAL_TRUSTED_SETUP,
    SECX_PER_CIRCUIT_TRUSTED_SETUP,
    // [new] Interactivity
    SECX_INTERACTIVITY,
    SECX_NON_INTERACTIVE,
    SECX_INTERACTIVE,
    // [new] SelectiveDisclosure
    SECX_SELECTIVE_DISCLOSURE,
    SECX_SELECTIVELY_DISCLOSABLE,
    SECX_ALL_OR_NOTHING,
    // [new] SingleUse
    SECX_SINGLE_USE,
    SECX_SINGLE_USE_ENFORCED,
    SECX_REPLAYABLE,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine-readable `secprop-ext.ttl` is the canonical form; keep it pinned
    /// to these constants so the two cannot drift (the `sparq_trust::vocab` discipline).
    const TTL: &str = include_str!("../ontologies/secprop-ext.ttl");

    /// Every constant is in the `sec-prop:` namespace.
    #[test]
    fn all_constants_share_the_sec_prop_namespace() {
        for &iri in ALL_SECPROP_IRIS {
            assert!(
                iri.starts_with(SEC_PROP_NS),
                "constant `{}` is not in the sec-prop: namespace",
                iri,
            );
        }
    }

    /// No duplicate IRI in the registry (a copy-paste guard).
    #[test]
    fn no_duplicate_iris() {
        let mut seen = std::collections::HashSet::new();
        for &iri in ALL_SECPROP_IRIS {
            assert!(
                seen.insert(iri),
                "duplicate IRI in ALL_SECPROP_IRIS: {}",
                iri
            );
        }
    }

    /// Every Rust constant is declared in `secprop-ext.ttl` as a `secx:LocalName`
    /// subject. The reused `sec-prop:` superclasses are referenced (range/import),
    /// not subjects of a fresh declaration here, so they are exempt from the
    /// declared-subject check.
    #[test]
    fn secprop_ext_iris_match_rust_constants() {
        // The two reused vendored class IRIs are referenced (owl:imports /
        // rdfs:range), not re-declared as subjects in the extension file.
        let reused_only = [SEC_PROP_UNLINKABILITY, SEC_PROP_SECURITY_PROPERTY];

        for &iri in ALL_SECPROP_IRIS {
            if reused_only.contains(&iri) {
                continue;
            }
            let local = iri
                .strip_prefix(SEC_PROP_NS)
                .expect("every constant is in the sec-prop: namespace");
            let decl = format!("secx:{} ", local);
            assert!(
                TTL.contains(&decl),
                "secprop-ext.ttl is missing a declaration for `secx:{}` ({}) — the \
                 published vocabulary drifted from the Rust constants (sq-5oru9)",
                local,
                iri,
            );
        }
    }

    /// And every `secx:LocalName a` declaration in the Turtle is backed by a Rust
    /// constant — no published term without a constant. Scans declaration lines (a
    /// `secx:Foo a …` start-of-line subject), the inverse of the `sparq_trust::vocab`
    /// sync test.
    #[test]
    fn every_ttl_term_has_a_rust_constant() {
        let declared_locals: Vec<&str> = TTL
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let rest = t.strip_prefix("secx:")?;
                // Skip the bare-namespace metadata line (no local name) if present.
                if rest.starts_with(char::is_whitespace) {
                    return None;
                }
                let name = rest.split_whitespace().next()?;
                // A term declaration line looks like `secx:Foo a …`.
                if rest[name.len()..].trim_start().starts_with("a ") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !declared_locals.is_empty(),
            "no secx: term declarations found in secprop-ext.ttl — parser/format drift",
        );

        let constant_locals: std::collections::HashSet<&str> = ALL_SECPROP_IRIS
            .iter()
            .filter_map(|iri| iri.strip_prefix(SEC_PROP_NS))
            .collect();

        for local in declared_locals {
            assert!(
                constant_locals.contains(local),
                "secprop-ext.ttl declares `secx:{}` but no Rust constant names it — \
                 add the constant or remove the term (sq-5oru9)",
                local,
            );
        }
    }

    /// The assurance default + the sq-qhy4 audit gate are documented in the Turtle —
    /// a load-bearing honesty invariant of the vocabulary (the default is `Claimed`,
    /// not `Proven`, and `Proven` is barred while the audit is open).
    #[test]
    fn assurance_default_and_audit_gate_are_documented() {
        assert!(
            TTL.contains("sq-qhy4"),
            "secprop-ext.ttl must reference the open external-audit gate (sq-qhy4)",
        );
        assert!(
            TTL.contains("DEFAULT for a sparq ZK property"),
            "secprop-ext.ttl must document Claimed as the default assurance (#1001 Option A)",
        );
        assert!(
            TTL.contains("NO sparq ZK method may be labelled\n# `secx:Proven`")
                || TTL.contains("no sparq ZK method may be labelled `Proven`"),
            "secprop-ext.ttl must document that Proven is barred while sq-qhy4 is open",
        );
    }
}
