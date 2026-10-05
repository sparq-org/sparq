//! # `secprop` — first-class `secx:requires…` ODRL leftOperands (the sparq
//! security-property profile)
//!
//! Makes the custom `secx:requires…` ODRL leftOperands of the sparq
//! **security-property profile** first-class in the policy model: their IRIs as
//! Rust constants, the leftOperand → security-property-dimension mapping
//! ([`over_dimension`]), a recogniser ([`is_secprop_left_operand`]), and the
//! published profile IRI ([`PROFILE_IRI`]).
//!
//! ## Why this exists (the ODRL profile nuance)
//!
//! An [`crate::Constraint`] over a `leftOperand` may carry **any** IRI — sparq-policy
//! already parses and evaluates a `secx:requires…` constraint as an opaque
//! custom-leftOperand string (design §4.3.1). But a **conforming ODRL processor MAY
//! reject an undeclared custom leftOperand**: the W3C ODRL 2.2 contract is that a
//! custom leftOperand MUST be declared in a published [`odrl:Profile`](PROFILE_IRI)
//! (as `owl:NamedIndividual, odrl:LeftOperand, skos:Concept` with `rdfs:isDefinedBy`
//! → the profile), and a policy using them must assert `odrl:profile <profileIRI>`.
//! This module ships that profile (`ontologies/odrl-secprop-profile.ttl`) and makes
//! the leftOperands first-class — a small, standards-conformant addition, not ad-hoc.
//!
//! ## What this is — and is NOT
//!
//! This is the **bridge vocabulary**: it NAMES the requireable dimensions and the
//! leftOperand→dimension map ([`SECPROP_LEFT_OPERANDS`]). It does **not** perform the
//! admissibility reduction (that is the N3 ruleset, Phase 2, `sq-ufsi9`, over the
//! per-method annotation graph, Phase 3, `sq-bevd3`), and it asserts **NO** security
//! property of any method. Whether a sparq method actually *has* a property is
//! recorded — with its epistemic basis — by the annotation graph; **sparq's whole ZK
//! estate is research-grade and NOT externally audited** (bead `sq-qhy4`), and
//! `sparq-mpc` is semi-honest-only. This module only lets a policy *name the bar*; it
//! makes no privacy/soundness claim.
//!
//! ## The ODRL half of the ZK↔ODRL constraint-discharge envelope (sq-yh427)
//!
//! [`discharge_requirements`] projects a parsed [`crate::Policy`] to the
//! `secx:requires…` constraints it carries — *what a presented proof would have to
//! establish*, over which dimension, at which level, and on which deontic rule. That
//! is the **ODRL-side interface** the ZK↔ODRL envelope plugs into: a host reads it to
//! know which properties to demand of a presentation before it can decide.
//!
//! It returns a [`DischargeExpr`] **tree per rule**, not a flat list, because the
//! `odrl:` combinator is load-bearing: under `odrl:or` a presentation establishing
//! *either* branch suffices, and under `odrl:xone` establishing more than one is a
//! failure. A flat list would read as a conjunction and would overstate the
//! requirement. Branches whose leftOperand is not a `secx:` one are kept — whole,
//! `(leftOperand, operator, rightOperand)` triple intact — as [`DischargeExpr::Other`]
//! rather than pruned: dropping them from an `or` would likewise turn an alternative
//! into a mandate, and keeping only their leftOperand would leave two different
//! alternatives over one leftOperand indistinguishable and undecidable.
//!
//! The **wire half** of that envelope — a VC 2.0 Verifiable Presentation carrying a
//! Data-Integrity proof under a `sparql-zk`-style cryptosuite — is **deliberately NOT
//! implemented here**: no such cryptosuite exists in this workspace (`sparq-vc`
//! implements `eddsa-rdfc-2022` only), its identifier and claim shape are still under
//! cross-agent design with the Solid-server sibling, and sparq's ZK estate has **no**
//! external accredited-cryptographer sign-off (`sq-qhy4`). Extraction makes **no**
//! security claim and performs **no** verification.
//!
//! Nor does this module *discharge* anything. It does not order levels (the
//! `secx:atLeast` partial order + the admissibility reduction live in
//! `sparq-trust`'s `admissibility` module, `sq-ufsi9`) and it does not change the
//! stateless [`crate::evaluate`] path: a `secx:` constraint the request supplies no
//! evidence for remains fail-closed unsatisfied, exactly as before.
//!
//! ## Desugaring (design §4.5)
//!
//! Each `secx:requiresX` leftOperand is convenience **sugar** for the single generic
//! primitive "a constraint over dimension `X`": the one `secx:overDimension` fact per
//! leftOperand is what the admissibility rule reads, so a working group standardises
//! only the generic `hasProperty`/`overDimension`/`atLeast` machinery — one rule, not
//! one per leftOperand.
//!
//! ## Opt-in by construction
//!
//! This whole module (and the `.ttl` it pins) is behind the **default-OFF
//! `secprop-leftoperands`** cargo feature, so it adds **nothing** to the lean default
//! build — it is plain `const &str` data plus tests, no new dependency, strictly
//! additive, and it does **not** change the stateless `evaluate` path.
//!
//! The canonical machine-readable form is `ontologies/odrl-secprop-profile.ttl`; the
//! drift tests below keep that profile and these constants from diverging (the
//! `crates/sparq-trust/src/secprop.rs` discipline).
//!
//! [OPUS-4.8] sq-uor3g (epic sq-0dksu, Phase 4; design record
//! `research/security-properties-ontology-design.md` §4.3.1 + §9). 🤖 SPARQ agent —
//! security-properties ontology. Flag for re-review when Fable returns.

use crate::model::{
    Constraint, ConstraintNode, LogicalConstraint, LogicalOperator, Operator, Policy, Rule, Value,
};

/// The `secx:` namespace base — the vendored ZKP-SPARQL `sec-prop:` extension
/// namespace these leftOperands and dimensions live under (a real `w3id.org`
/// permanent identifier). NOT a placeholder.
///
/// Re-exported from the `sparq-secprop-vocab` leaf that owns the vocabulary
/// (sq-3705), so it is the SAME string `sparq_trust::secprop::SEC_PROP_NS` and
/// `sparq_zk::secprop::SEC_PROP_NS` name — not a fourth copy.
pub use sparq_secprop_vocab::SEC_PROP_NS as SECX_NS;

/// The published sparq **security-property ODRL profile** IRI. A policy that uses any
/// `secx:requires…` leftOperand MUST assert `odrl:profile <PROFILE_IRI>` so a
/// conforming ODRL processor admits the custom leftOperands (design §4.3.1). The
/// profile document is `ontologies/odrl-secprop-profile.ttl`.
pub const PROFILE_IRI: &str = "https://sparq.dev/ns/odrl-secprop-profile#";

/// The `secx:overDimension` property IRI — maps a `secx:requires…` leftOperand to the
/// `secx:` security-property dimension a constraint over it ranges over (the single
/// primitive the admissibility rule reads).
pub const OVER_DIMENSION: &str = "https://w3id.org/zkp-sparql/sec-prop#overDimension";

// ── the requireable leftOperands ─────────────────────────────────────────────

/// `secx:requiresUnlinkabilityScope` — over the `UnlinkabilityScope` dimension.
pub const REQUIRES_UNLINKABILITY_SCOPE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresUnlinkabilityScope";
/// `secx:requiresUnlinkabilityStrength` — over the `UnlinkabilityStrength` dimension.
pub const REQUIRES_UNLINKABILITY_STRENGTH: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresUnlinkabilityStrength";
/// `secx:requiresPostQuantumForgery` — over the `PostQuantumForgery` dimension.
pub const REQUIRES_POST_QUANTUM_FORGERY: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresPostQuantumForgery";
/// `secx:requiresPostQuantumSnooping` — over the `PostQuantumSnooping` dimension.
pub const REQUIRES_POST_QUANTUM_SNOOPING: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresPostQuantumSnooping";
/// `secx:requiresZeroKnowledge` — over the `ZeroKnowledgeType` dimension.
pub const REQUIRES_ZERO_KNOWLEDGE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresZeroKnowledge";
/// `secx:requiresSoundness` — over the `Soundness` dimension.
pub const REQUIRES_SOUNDNESS: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresSoundness";
/// `secx:requiresCompleteness` — over the `Completeness` dimension.
pub const REQUIRES_COMPLETENESS: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresCompleteness";
/// `secx:requiresHiding` — over the `Hiding` dimension.
pub const REQUIRES_HIDING: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresHiding";
/// `secx:requiresBinding` — over the `Binding` dimension.
pub const REQUIRES_BINDING: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresBinding";
/// `secx:requiresAnonymity` — over the `Anonymity` dimension.
pub const REQUIRES_ANONYMITY: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresAnonymity";
/// `secx:requiresSelectiveDisclosure` — over the `SelectiveDisclosure` dimension.
pub const REQUIRES_SELECTIVE_DISCLOSURE: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresSelectiveDisclosure";
/// `secx:requiresSingleUse` — over the `SingleUse` dimension.
pub const REQUIRES_SINGLE_USE: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresSingleUse";
/// `secx:requiresSetup` — over the `Setup` dimension.
pub const REQUIRES_SETUP: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresSetup";
/// `secx:requiresInteractivity` — over the `Interactivity` dimension.
pub const REQUIRES_INTERACTIVITY: &str =
    "https://w3id.org/zkp-sparql/sec-prop#requiresInteractivity";
/// `secx:requiresAssurance` — over the `AssuranceLevel` dimension. The conservative
/// gate: `requiresAssurance gteq secx:Proven` mechanically removes every unaudited
/// sparq method from the admissible set (`sq-qhy4` enters the data flow).
pub const REQUIRES_ASSURANCE: &str = "https://w3id.org/zkp-sparql/sec-prop#requiresAssurance";

// ── the dimension IRIs (the `secx:overDimension` targets) ────────────────────
//
// [OPUS-5] sq-3705: these are NOT declared here — every one is an alias of the
// canonical constant in the ZERO-dependency `sparq-secprop-vocab` leaf, which owns
// the `secx:` vocabulary, its `secprop-ext.ttl`, and the single drift test pinning
// the two. They used to be byte-identical copies of `sparq-trust::secprop`'s set,
// kept honest by an `include_str!` of `../../sparq-trust/…` (a compile-time read; this
// crate could not take a `sparq-trust` edge without dragging sparq-zk + sparq-canon
// + sparq-shacl + sparq-reason into its lean graph). The leaf has NO dependencies,
// so the edge costs this crate nothing and the copies are gone.
//
// The `DIM_*` NAMES are kept as the profile-facing spelling — a policy reader thinks
// in "the dimension this leftOperand ranges over" — but they now resolve to one
// string, so `DIM_SOUNDNESS` and `sparq_trust::secprop::SECX_SOUNDNESS` cannot drift.

/// `secx:UnlinkabilityScope` dimension.
pub use sparq_secprop_vocab::SECX_UNLINKABILITY_SCOPE as DIM_UNLINKABILITY_SCOPE;
/// `secx:UnlinkabilityStrength` dimension.
pub use sparq_secprop_vocab::SECX_UNLINKABILITY_STRENGTH as DIM_UNLINKABILITY_STRENGTH;
/// `secx:PostQuantumForgery` dimension (a vendored `sec-prop:` property).
pub use sparq_secprop_vocab::SEC_PROP_POST_QUANTUM_FORGERY as DIM_POST_QUANTUM_FORGERY;
/// `secx:PostQuantumSnooping` dimension (a vendored `sec-prop:` property).
pub use sparq_secprop_vocab::SEC_PROP_POST_QUANTUM_SNOOPING as DIM_POST_QUANTUM_SNOOPING;
/// `secx:ZeroKnowledgeType` dimension.
pub use sparq_secprop_vocab::SECX_ZERO_KNOWLEDGE_TYPE as DIM_ZERO_KNOWLEDGE_TYPE;
/// `secx:Soundness` dimension.
pub use sparq_secprop_vocab::SECX_SOUNDNESS as DIM_SOUNDNESS;
/// `secx:Completeness` dimension.
pub use sparq_secprop_vocab::SECX_COMPLETENESS as DIM_COMPLETENESS;
/// `secx:Hiding` dimension.
pub use sparq_secprop_vocab::SECX_HIDING as DIM_HIDING;
/// `secx:Binding` dimension.
pub use sparq_secprop_vocab::SECX_BINDING as DIM_BINDING;
/// `secx:Anonymity` dimension.
pub use sparq_secprop_vocab::SECX_ANONYMITY as DIM_ANONYMITY;
/// `secx:SelectiveDisclosure` dimension.
pub use sparq_secprop_vocab::SECX_SELECTIVE_DISCLOSURE as DIM_SELECTIVE_DISCLOSURE;
/// `secx:SingleUse` dimension.
pub use sparq_secprop_vocab::SECX_SINGLE_USE as DIM_SINGLE_USE;
/// `secx:Setup` dimension.
pub use sparq_secprop_vocab::SECX_SETUP as DIM_SETUP;
/// `secx:Interactivity` dimension.
pub use sparq_secprop_vocab::SECX_INTERACTIVITY as DIM_INTERACTIVITY;
/// `secx:AssuranceLevel` dimension (the epistemic-basis axis — design §4.2.2).
pub use sparq_secprop_vocab::SECX_ASSURANCE_LEVEL as DIM_ASSURANCE_LEVEL;

/// The complete `(leftOperand, dimension)` map of the sparq security-property
/// profile — one entry per requireable dimension, in declaration order. The **single
/// source of truth** the recogniser/lookup and the TTL drift tests iterate.
///
/// Each pair is the `secx:requiresX secx:overDimension secx:Dim` fact the profile
/// declares: a constraint over the leftOperand `.0` ranges over the security-property
/// dimension `.1`.
pub const SECPROP_LEFT_OPERANDS: &[(&str, &str)] = &[
    (REQUIRES_UNLINKABILITY_SCOPE, DIM_UNLINKABILITY_SCOPE),
    (REQUIRES_UNLINKABILITY_STRENGTH, DIM_UNLINKABILITY_STRENGTH),
    (REQUIRES_POST_QUANTUM_FORGERY, DIM_POST_QUANTUM_FORGERY),
    (REQUIRES_POST_QUANTUM_SNOOPING, DIM_POST_QUANTUM_SNOOPING),
    (REQUIRES_ZERO_KNOWLEDGE, DIM_ZERO_KNOWLEDGE_TYPE),
    (REQUIRES_SOUNDNESS, DIM_SOUNDNESS),
    (REQUIRES_COMPLETENESS, DIM_COMPLETENESS),
    (REQUIRES_HIDING, DIM_HIDING),
    (REQUIRES_BINDING, DIM_BINDING),
    (REQUIRES_ANONYMITY, DIM_ANONYMITY),
    (REQUIRES_SELECTIVE_DISCLOSURE, DIM_SELECTIVE_DISCLOSURE),
    (REQUIRES_SINGLE_USE, DIM_SINGLE_USE),
    (REQUIRES_SETUP, DIM_SETUP),
    (REQUIRES_INTERACTIVITY, DIM_INTERACTIVITY),
    (REQUIRES_ASSURANCE, DIM_ASSURANCE_LEVEL),
];

/// True iff `left_operand` is a `secx:requires…` leftOperand this profile declares
/// (i.e. a [`crate::Constraint::left`] the security-property bridge recognises).
///
/// A policy carrying a recognised leftOperand SHOULD assert `odrl:profile
/// <PROFILE_IRI>`; the recogniser lets a host check that without re-parsing the
/// profile TTL.
pub fn is_secprop_left_operand(left_operand: &str) -> bool {
    SECPROP_LEFT_OPERANDS
        .iter()
        .any(|(lo, _)| *lo == left_operand)
}

/// The `secx:` security-property **dimension** IRI a `secx:requires…` leftOperand
/// ranges over (its `secx:overDimension`), or `None` if `left_operand` is not a
/// leftOperand of this profile.
///
/// This is the one fact the admissibility reduction (Phase 2) reads: a constraint
/// over `left_operand` is discharged by a method whose asserted level for
/// `over_dimension(left_operand)` meets the constraint's right-operand level.
pub fn over_dimension(left_operand: &str) -> Option<&'static str> {
    SECPROP_LEFT_OPERANDS
        .iter()
        .find(|(lo, _)| *lo == left_operand)
        .map(|(_, dim)| *dim)
}

/// The deontic force of the rule a [`DischargeObligation`] was extracted from — which
/// way discharging it moves the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Deontic {
    /// The obligation sits on an `odrl:permission`: the permission grants only while
    /// the rule's whole constraint expression holds, so establishing this property is
    /// (part of) what earns access.
    Permission,
    /// The obligation sits on an `odrl:prohibition`: a prohibition **fires** (and
    /// deny-overrides the decision — see [`crate::evaluate`]) when its whole
    /// constraint expression holds. Establishing this property therefore moves toward
    /// *withholding* access; a host must not treat it as something to satisfy. Note it
    /// takes the enclosing [`DischargeExpr`] holding, not this leaf alone, to make a
    /// multi-constraint prohibition fire.
    Prohibition,
}

/// One security-property **proof-discharge obligation**: a single `secx:requires…`
/// constraint found on a policy rule, resolved to the dimension it ranges over.
///
/// This is a faithful re-projection of already-parsed [`crate::Constraint`] data — it
/// asserts nothing about any proof, method, or cryptosuite (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DischargeObligation {
    /// The [`crate::Rule::id`] of the rule carrying the constraint (for justification).
    pub rule: String,
    /// Whether that rule is a permission or a prohibition — see [`Deontic`].
    pub deontic: Deontic,
    /// The `secx:requires…` [`crate::Constraint::left`] IRI.
    pub left_operand: String,
    /// The `secx:` dimension the leftOperand ranges over — its
    /// [`over_dimension`] (`secx:overDimension`).
    pub dimension: &'static str,
    /// The constraint's [`crate::Operator`] (`odrl:gteq` for the usual "at least this
    /// level" preference).
    pub operator: Operator,
    /// The constraint's right operand — the required level.
    pub required: Value,
}

/// What one policy rule requires of a presentation, with the `odrl:` combinator
/// structure that decides *which combinations* of the leaves actually matter kept
/// intact.
///
/// The combinator is load-bearing and must not be flattened away: under [`Any`](Self::Any)
/// a presentation establishing **either** branch suffices, and under
/// [`ExactlyOne`](Self::ExactlyOne) establishing more than one is a *failure*. A host
/// can therefore read the alternatives straight off this tree without going back to
/// the original [`crate::Policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DischargeExpr {
    /// A leaf `secx:requires…` constraint — a property a proof can establish.
    Atomic(DischargeObligation),
    /// A leaf constraint whose leftOperand is **not** a `secx:` one (core ODRL, or an
    /// unrelated custom profile): decided from the ordinary request context, never by
    /// a proof.
    ///
    /// It carries the **whole** [`crate::Constraint`] — the full
    /// `(leftOperand, operator, rightOperand)` triple — not just the leftOperand: two
    /// branches over the same leftOperand (`recipient eq Bob` vs `recipient eq Carol`,
    /// or `eq` vs `neq`) are *different* alternatives, and a host that saw only the
    /// leftOperand could neither tell them apart nor decide which of them holds. That
    /// matters most under [`ExactlyOne`](Self::ExactlyOne), where the count of holding
    /// branches — not merely which dimensions appear — is the verdict.
    ///
    /// These are kept rather than pruned: deleting a non-proof operand from an
    /// [`Any`](Self::Any) would silently turn an alternative into a mandate.
    Other(Constraint),
    /// `odrl:and` — every operand must hold. Also the implicit combinator over a
    /// rule's own top-level [`crate::Rule::constraints`] plus its
    /// [`crate::Rule::logical_constraints`], which ODRL conjoins.
    All(Vec<DischargeExpr>),
    /// `odrl:or` — at least one operand must hold.
    Any(Vec<DischargeExpr>),
    /// `odrl:xone` — **exactly** one operand must hold; establishing a second one
    /// breaks the rule rather than reinforcing it.
    ExactlyOne(Vec<DischargeExpr>),
}

impl DischargeExpr {
    /// A flat inventory of every [`DischargeObligation`] leaf in this tree, in
    /// document order.
    ///
    /// **NON-DECISIONAL.** This is "which security properties does this rule mention
    /// at all", useful for capability negotiation or logging. It is emphatically *not*
    /// "which properties a presentation must establish": the flattening discards the
    /// [`Any`](Self::Any)/[`ExactlyOne`](Self::ExactlyOne) structure, so reading it as
    /// a conjunction overstates the requirement. Walk the tree for that.
    pub fn obligations(&self) -> Vec<&DischargeObligation> {
        let mut out = Vec::new();
        self.collect_obligations(&mut out);
        out
    }

    fn collect_obligations<'a>(&'a self, out: &mut Vec<&'a DischargeObligation>) {
        match self {
            DischargeExpr::Atomic(o) => out.push(o),
            DischargeExpr::Other(_) => {}
            DischargeExpr::All(ops) | DischargeExpr::Any(ops) | DischargeExpr::ExactlyOne(ops) => {
                for op in ops {
                    op.collect_obligations(out);
                }
            }
        }
    }

    /// Whether this subtree mentions any `secx:` property at all.
    fn has_obligation(&self) -> bool {
        match self {
            DischargeExpr::Atomic(_) => true,
            DischargeExpr::Other(_) => false,
            DischargeExpr::All(ops) | DischargeExpr::Any(ops) | DischargeExpr::ExactlyOne(ops) => {
                ops.iter().any(DischargeExpr::has_obligation)
            }
        }
    }
}

/// One policy rule's security-property discharge requirement — see [`DischargeExpr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDischarge {
    /// The [`crate::Rule::id`] the requirement came from (for justification). Every
    /// [`DischargeExpr::Atomic`] leaf below repeats it, so a flattened inventory stays
    /// self-describing.
    pub rule: String,
    /// Whether that rule is a permission or a prohibition — see [`Deontic`].
    pub deontic: Deontic,
    /// The requirement tree, combinator structure intact.
    pub requirement: DischargeExpr,
}

/// What a presentation would have to establish for each rule of a policy that asks for
/// any security property, in rule order (permissions before prohibitions).
///
/// This is the ODRL half of the ZK↔ODRL constraint-discharge envelope (`sq-yh427`): a
/// host reads it to answer *"before I can decide this request, what would a presented
/// proof have to establish?"*. Each entry keeps its `odrl:and`/`or`/`xone` structure,
/// so the valid **alternatives** are readable without consulting the original
/// [`crate::Policy`] — see [`DischargeExpr`]. Rules mentioning no `secx:` property are
/// omitted, so an empty result means no presentation is needed at all.
///
/// **Scope, stated honestly.** Extraction is pure projection: it verifies nothing,
/// orders no levels, and changes no evaluation semantics. It also **excludes duty
/// constraints** — the single-node base case checks a [`crate::Duty`] by *action*
/// discharge and never evaluates the duty's own constraints (see [`crate::Duty`]), so
/// reporting them as obligations the evaluator will check would misdescribe it.
///
/// ```
/// # // cargo: sparq-policy with --features secprop-leftoperands
/// use sparq_policy::secprop::{discharge_requirements, Deontic, DischargeExpr, REQUIRES_ASSURANCE};
/// use sparq_policy::{Action, Constraint, Operator, Policy, Rule, Value, ODRL_NS};
///
/// let policy = Policy {
///     iri: None,
///     permissions: vec![Rule {
///         id: "urn:rule:1".into(),
///         action: Action(format!("{}read", ODRL_NS)),
///         target: None,
///         assignee: None,
///         assigner: None,
///         constraints: vec![Constraint {
///             left: REQUIRES_ASSURANCE.into(),
///             operator: Operator::Gteq,
///             right: Value::Iri("https://w3id.org/zkp-sparql/sec-prop#Proven".into()),
///         }],
///         logical_constraints: vec![],
///         duties: vec![],
///     }],
///     prohibitions: vec![],
///     conflict: None,
///     ..Policy::default()
/// };
///
/// let requirements = discharge_requirements(&policy);
/// assert_eq!(requirements.len(), 1);
/// assert_eq!(requirements[0].deontic, Deontic::Permission);
/// // A rule's own constraints are conjoined: every one of them must hold.
/// assert!(matches!(requirements[0].requirement, DischargeExpr::All(_)));
/// let leaves = requirements[0].requirement.obligations();
/// assert_eq!(
///     leaves[0].dimension,
///     "https://w3id.org/zkp-sparql/sec-prop#AssuranceLevel"
/// );
/// ```
pub fn discharge_requirements(policy: &Policy) -> Vec<RuleDischarge> {
    let mut out = Vec::new();
    for (rules, deontic) in [
        (&policy.permissions, Deontic::Permission),
        (&policy.prohibitions, Deontic::Prohibition),
    ] {
        for rule in rules {
            let requirement = rule_expr(rule, deontic);
            if requirement.has_obligation() {
                out.push(RuleDischarge {
                    rule: rule.id.clone(),
                    deontic,
                    requirement,
                });
            }
        }
    }
    out
}

/// A rule's own constraints and its compound constraints, conjoined — ODRL requires
/// all of them of a rule.
fn rule_expr(rule: &Rule, deontic: Deontic) -> DischargeExpr {
    let mut operands: Vec<DischargeExpr> = rule
        .constraints
        .iter()
        .map(|c| leaf_expr(&rule.id, deontic, c))
        .collect();
    operands.extend(
        rule.logical_constraints
            .iter()
            .map(|lc| logical_expr(&rule.id, deontic, lc)),
    );
    DischargeExpr::All(operands)
}

/// Mirror an `odrl:LogicalConstraint` tree into a [`DischargeExpr`], preserving the
/// combinator: it is exactly what tells a host whether the operands are all required
/// (`and`), alternatives (`or`), or mutually exclusive (`xone`).
fn logical_expr(rule_id: &str, deontic: Deontic, lc: &LogicalConstraint) -> DischargeExpr {
    let operands = lc
        .operands
        .iter()
        .map(|operand| match operand {
            ConstraintNode::Atomic(c) => leaf_expr(rule_id, deontic, c),
            ConstraintNode::Compound(inner) => logical_expr(rule_id, deontic, inner),
        })
        .collect();
    match lc.operator {
        LogicalOperator::And => DischargeExpr::All(operands),
        LogicalOperator::Or => DischargeExpr::Any(operands),
        LogicalOperator::Xone => DischargeExpr::ExactlyOne(operands),
    }
}

/// A single constraint as a leaf: an obligation iff its leftOperand is one this
/// profile declares, otherwise an opaque non-proof branch.
fn leaf_expr(rule_id: &str, deontic: Deontic, c: &Constraint) -> DischargeExpr {
    let Some(dimension) = over_dimension(&c.left) else {
        // A core ODRL (or unrelated custom) leftOperand — not a proof obligation, but
        // still a branch of the requirement, so it must not be dropped. The whole
        // triple is carried: the operator and right operand are what let a host decide
        // the branch (and tell two branches over one leftOperand apart).
        return DischargeExpr::Other(c.clone());
    };
    DischargeExpr::Atomic(DischargeObligation {
        rule: rule_id.to_owned(),
        deontic,
        left_operand: c.left.clone(),
        dimension,
        operator: c.operator,
        required: c.right.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine-readable `odrl-secprop-profile.ttl` is the canonical form; keep it
    /// pinned to these constants so the two cannot drift (the
    /// `crates/sparq-trust/src/secprop.rs` discipline).
    const TTL: &str = include_str!("../ontologies/odrl-secprop-profile.ttl");

    /// Every leftOperand and every dimension IRI is in the `secx:` namespace.
    #[test]
    fn all_iris_share_the_secx_namespace() {
        for (lo, dim) in SECPROP_LEFT_OPERANDS {
            assert!(
                lo.starts_with(SECX_NS),
                "leftOperand `{}` is not in the secx: namespace",
                lo
            );
            assert!(
                dim.starts_with(SECX_NS),
                "dimension `{}` is not in the secx: namespace",
                dim
            );
        }
        assert!(OVER_DIMENSION.starts_with(SECX_NS));
    }

    /// No duplicate leftOperand, and no two leftOperands map to the same dimension
    /// (one leftOperand per requireable dimension — a copy-paste guard).
    #[test]
    fn no_duplicate_left_operand_or_dimension() {
        let mut seen_lo = std::collections::HashSet::new();
        let mut seen_dim = std::collections::HashSet::new();
        for (lo, dim) in SECPROP_LEFT_OPERANDS {
            assert!(seen_lo.insert(*lo), "duplicate leftOperand: {}", lo);
            assert!(
                seen_dim.insert(*dim),
                "two leftOperands map to dimension {}",
                dim
            );
        }
    }

    /// The recogniser + lookup agree with the registry (and reject an unrelated IRI).
    #[test]
    fn recogniser_and_lookup_agree_with_registry() {
        for (lo, dim) in SECPROP_LEFT_OPERANDS {
            assert!(is_secprop_left_operand(lo), "recogniser missed {}", lo);
            assert_eq!(over_dimension(lo), Some(*dim), "wrong dimension for {}", lo);
        }
        // A core ODRL leftOperand and a nonsense IRI are NOT secprop leftOperands.
        assert!(!is_secprop_left_operand(crate::ODRL_PURPOSE));
        assert!(!is_secprop_left_operand(
            "https://example.org/notAProfileTerm"
        ));
        assert_eq!(over_dimension(crate::ODRL_PURPOSE), None);
    }

    // ── discharge-obligation extraction (sq-yh427, the ODRL half) ────────────

    /// A permission rule carrying `constraints`, for the extraction tests.
    fn rule_with(id: &str, constraints: Vec<Constraint>) -> Rule {
        Rule {
            id: id.into(),
            action: crate::Action(format!("{}read", crate::ODRL_NS)),
            target: None,
            assignee: None,
            assigner: None,
            constraints,
            logical_constraints: vec![],
            duties: vec![],
        }
    }

    fn secprop_constraint(left: &str, level: &str) -> Constraint {
        Constraint {
            left: left.into(),
            operator: Operator::Gteq,
            right: Value::Iri(format!("{}{}", SECX_NS, level)),
        }
    }

    /// A rule whose only constraint set is `constraints` yields one entry; wrap the
    /// combinator-shape assertions.
    fn single_requirement(policy: &Policy) -> DischargeExpr {
        let mut reqs = discharge_requirements(policy);
        assert_eq!(reqs.len(), 1, "expected exactly one rule requirement");
        reqs.remove(0).requirement
    }

    /// A one-rule policy whose single compound constraint is `lc`.
    fn policy_with_logical(lc: LogicalConstraint) -> Policy {
        let mut rule = rule_with("urn:rule:1", vec![]);
        rule.logical_constraints = vec![lc];
        Policy {
            iri: None,
            permissions: vec![rule],
            prohibitions: vec![],
            conflict: None,
            ..Policy::default()
        }
    }

    /// A `secx:requires…` constraint is extracted with its dimension, operator and
    /// required level; a core ODRL constraint alongside it becomes an opaque
    /// [`DischargeExpr::Other`] branch (not a property a proof discharges, but still
    /// part of the requirement).
    #[test]
    fn extracts_secprop_constraints_and_ignores_core_left_operands() {
        let policy = Policy {
            iri: None,
            permissions: vec![rule_with(
                "urn:rule:1",
                vec![
                    secprop_constraint(REQUIRES_ASSURANCE, "Proven"),
                    Constraint {
                        left: crate::ODRL_PURPOSE.into(),
                        operator: Operator::Eq,
                        right: Value::Iri("https://w3id.org/dpv#ResearchAndDevelopment".into()),
                    },
                ],
            )],
            prohibitions: vec![],
            conflict: None,
            ..Policy::default()
        };

        let requirement = single_requirement(&policy);
        // A rule conjoins its own constraints, and the core leftOperand survives as a
        // non-proof branch.
        let DischargeExpr::All(ref operands) = requirement else {
            panic!("a rule's constraints are conjoined, got {:?}", requirement);
        };
        assert_eq!(operands.len(), 2);
        assert_eq!(
            operands[1],
            DischargeExpr::Other(Constraint {
                left: crate::ODRL_PURPOSE.into(),
                operator: Operator::Eq,
                right: Value::Iri("https://w3id.org/dpv#ResearchAndDevelopment".into()),
            }),
            "the non-proof branch keeps its whole (left, operator, right) triple",
        );

        let obligations = requirement.obligations();
        assert_eq!(obligations.len(), 1, "only the secx: constraint is an obligation");
        let o = obligations[0];
        assert_eq!(o.rule, "urn:rule:1");
        assert_eq!(o.deontic, Deontic::Permission);
        assert_eq!(o.left_operand, REQUIRES_ASSURANCE);
        assert_eq!(o.dimension, DIM_ASSURANCE_LEVEL);
        assert_eq!(o.operator, Operator::Gteq);
        assert_eq!(
            o.required,
            Value::Iri(format!("{}Proven", SECX_NS)),
            "the required level is carried verbatim",
        );
    }

    /// A policy with no `secx:` constraint yields no requirements — the host needs no
    /// presentation at all (and an empty policy likewise). A rule carrying only core
    /// ODRL constraints is omitted rather than reported as an all-`Other` tree.
    #[test]
    fn no_secprop_constraints_yields_no_obligations() {
        assert!(discharge_requirements(&Policy::default()).is_empty());
        let policy = Policy {
            iri: None,
            permissions: vec![
                rule_with("urn:rule:1", vec![]),
                rule_with(
                    "urn:rule:2",
                    vec![Constraint {
                        left: crate::ODRL_PURPOSE.into(),
                        operator: Operator::Eq,
                        right: Value::Iri("https://w3id.org/dpv#ResearchAndDevelopment".into()),
                    }],
                ),
            ],
            prohibitions: vec![],
            conflict: None,
            ..Policy::default()
        };
        assert!(discharge_requirements(&policy).is_empty());
    }

    /// LOAD-BEARING: an obligation on a PROHIBITION is reported as such. Establishing
    /// it moves the prohibition toward firing (deny-overrides), so a host must never
    /// read it as "a property to satisfy".
    #[test]
    fn prohibition_obligations_carry_the_prohibition_deontic() {
        let policy = Policy {
            iri: None,
            permissions: vec![rule_with(
                "urn:perm:1",
                vec![secprop_constraint(REQUIRES_ZERO_KNOWLEDGE, "PerfectZK")],
            )],
            prohibitions: vec![rule_with(
                "urn:proh:1",
                vec![secprop_constraint(REQUIRES_SINGLE_USE, "Nullifier")],
            )],
            conflict: None,
            ..Policy::default()
        };

        let requirements = discharge_requirements(&policy);
        assert_eq!(requirements.len(), 2);
        // Permissions are reported before prohibitions.
        assert_eq!(requirements[0].deontic, Deontic::Permission);
        assert_eq!(requirements[0].rule, "urn:perm:1");
        assert_eq!(requirements[1].deontic, Deontic::Prohibition);
        assert_eq!(requirements[1].rule, "urn:proh:1");
        // The leaves repeat the rule's deontic, so a flattened inventory stays honest.
        let proh = requirements[1].requirement.obligations();
        assert_eq!(proh.len(), 1);
        assert_eq!(proh[0].deontic, Deontic::Prohibition);
        assert_eq!(proh[0].dimension, DIM_SINGLE_USE);
    }

    /// LOAD-BEARING: `and`, `or` and `xone` over the SAME two leaves produce
    /// DISTINGUISHABLE structures, so a host can read the valid evidence alternatives
    /// off the result without consulting the original `Policy`. Flattening them to one
    /// list would make an `or` alternative look like a mandate and would lose `xone`'s
    /// exactly-one semantics entirely.
    #[test]
    fn and_or_and_xone_produce_distinguishable_requirements() {
        let leaves = || {
            vec![
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_HIDING, "PerfectHiding")),
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_SOUNDNESS, "Statistical")),
            ]
        };
        let expected_leaves = vec![
            DischargeExpr::Atomic(DischargeObligation {
                rule: "urn:rule:1".into(),
                deontic: Deontic::Permission,
                left_operand: REQUIRES_HIDING.into(),
                dimension: DIM_HIDING,
                operator: Operator::Gteq,
                required: Value::Iri(format!("{}PerfectHiding", SECX_NS)),
            }),
            DischargeExpr::Atomic(DischargeObligation {
                rule: "urn:rule:1".into(),
                deontic: Deontic::Permission,
                left_operand: REQUIRES_SOUNDNESS.into(),
                dimension: DIM_SOUNDNESS,
                operator: Operator::Gteq,
                required: Value::Iri(format!("{}Statistical", SECX_NS)),
            }),
        ];

        let of = |op: crate::LogicalOperator| {
            single_requirement(&policy_with_logical(LogicalConstraint {
                id: "urn:lc:1".into(),
                operator: op,
                operands: leaves(),
            }))
        };
        let and = of(crate::LogicalOperator::And);
        let or = of(crate::LogicalOperator::Or);
        let xone = of(crate::LogicalOperator::Xone);

        // The rule's own (empty) constraint list conjoins with its one compound.
        assert_eq!(
            and,
            DischargeExpr::All(vec![DischargeExpr::All(expected_leaves.clone())]),
        );
        assert_eq!(
            or,
            DischargeExpr::All(vec![DischargeExpr::Any(expected_leaves.clone())]),
        );
        assert_eq!(
            xone,
            DischargeExpr::All(vec![DischargeExpr::ExactlyOne(expected_leaves)]),
        );
        assert_ne!(and, or, "an `or` of alternatives is not an `and` of mandates");
        assert_ne!(or, xone, "exactly-one is not at-least-one");

        // …while the NON-decisional flat inventory is identical for all three — which
        // is exactly why it must not be read as the requirement.
        let dims = |e: &DischargeExpr| -> Vec<&'static str> {
            e.obligations().iter().map(|o| o.dimension).collect()
        };
        assert_eq!(dims(&and), vec![DIM_HIDING, DIM_SOUNDNESS]);
        assert_eq!(dims(&and), dims(&or));
        assert_eq!(dims(&and), dims(&xone));
    }

    /// A non-`secx:` operand of an `or` is KEPT as [`DischargeExpr::Other`]: pruning it
    /// would leave `Any([Atomic])`, telling the host the proof is the only way through
    /// when the recipient branch also satisfies the rule.
    #[test]
    fn non_secprop_alternatives_are_kept_not_pruned() {
        let requirement = single_requirement(&policy_with_logical(LogicalConstraint {
            id: "urn:lc:1".into(),
            operator: crate::LogicalOperator::Or,
            operands: vec![
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_SOUNDNESS, "Statistical")),
                ConstraintNode::Atomic(Constraint {
                    left: crate::ODRL_RECIPIENT.into(),
                    operator: Operator::Eq,
                    right: Value::Iri("https://bob.example/profile#me".into()),
                }),
            ],
        }));

        let DischargeExpr::All(outer) = &requirement else {
            panic!("expected the rule conjunction, got {:?}", requirement);
        };
        let DischargeExpr::Any(alternatives) = &outer[0] else {
            panic!("expected an `or`, got {:?}", outer[0]);
        };
        assert_eq!(alternatives.len(), 2, "the non-proof alternative survives");
        assert_eq!(
            alternatives[1],
            DischargeExpr::Other(Constraint {
                left: crate::ODRL_RECIPIENT.into(),
                operator: Operator::Eq,
                right: Value::Iri("https://bob.example/profile#me".into()),
            }),
            "the alternative survives WHOLE — `recipient` alone would not say which \
             recipient satisfies it",
        );
    }

    /// An `odrl:recipient` constraint operand, for the alternative-discrimination
    /// tests below.
    fn recipient(operator: Operator, who: &str) -> ConstraintNode {
        ConstraintNode::Atomic(Constraint {
            left: crate::ODRL_RECIPIENT.into(),
            operator,
            right: Value::Iri(who.into()),
        })
    }

    const BOB: &str = "https://bob.example/profile#me";
    const CAROL: &str = "https://carol.example/profile#me";

    /// A test-local **host** decision procedure over the returned tree ALONE — it
    /// never consults the original [`Policy`]. `established` is the set of `secx:`
    /// leftOperands a presentation proves; `context` is the ordinary request context
    /// the non-proof branches are decided from.
    ///
    /// This is exactly the reading [`discharge_requirements`] promises a host can
    /// perform without going back to the policy, so it is the invariant under test:
    /// if a leaf did not carry enough of its constraint, this function could not be
    /// written.
    fn holds(expr: &DischargeExpr, context: &[(&str, Value)], established: &[&str]) -> bool {
        match expr {
            DischargeExpr::Atomic(o) => established.contains(&o.left_operand.as_str()),
            DischargeExpr::Other(c) => context
                .iter()
                .find(|(left, _)| *left == c.left)
                .is_some_and(|(_, supplied)| match c.operator {
                    Operator::Eq => *supplied == c.right,
                    Operator::Neq => *supplied != c.right,
                    other => unimplemented!("the test host decides eq/neq only, got {:?}", other),
                }),
            DischargeExpr::All(ops) => ops.iter().all(|o| holds(o, context, established)),
            DischargeExpr::Any(ops) => ops.iter().any(|o| holds(o, context, established)),
            DischargeExpr::ExactlyOne(ops) => {
                ops.iter().filter(|o| holds(o, context, established)).count() == 1
            }
        }
    }

    /// LOAD-BEARING: two non-`secx:` branches over the SAME leftOperand differing only
    /// in OPERATOR are different alternatives, and the returned tree must both
    /// distinguish and decide them. Under `xone` the *count* of holding branches is the
    /// verdict, so a leaf carrying only the leftOperand would make the two collapse into
    /// one indistinguishable node and mis-evaluate the rule.
    #[test]
    fn xone_over_branches_sharing_a_left_operand_is_decidable_from_the_tree_alone() {
        let requirement = single_requirement(&policy_with_logical(LogicalConstraint {
            id: "urn:lc:1".into(),
            operator: crate::LogicalOperator::Xone,
            operands: vec![
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_SOUNDNESS, "Statistical")),
                recipient(Operator::Eq, BOB),
                recipient(Operator::Neq, BOB),
            ],
        }));

        let DischargeExpr::All(outer) = &requirement else {
            panic!("expected the rule conjunction, got {:?}", requirement);
        };
        let DischargeExpr::ExactlyOne(alternatives) = &outer[0] else {
            panic!("expected an `xone`, got {:?}", outer[0]);
        };
        assert_ne!(
            alternatives[1], alternatives[2],
            "`recipient eq Bob` and `recipient neq Bob` are opposite alternatives and \
             must not collapse into the same node",
        );

        // Decided from the tree alone — the `Policy` is never consulted again.
        let to_bob = [(crate::ODRL_RECIPIENT, Value::Iri(BOB.into()))];
        let to_carol = [(crate::ODRL_RECIPIENT, Value::Iri(CAROL.into()))];
        assert!(
            holds(&requirement, &to_bob, &[]),
            "exactly one branch (`eq Bob`) holds for a request to Bob",
        );
        assert!(
            holds(&requirement, &to_carol, &[]),
            "exactly one branch (`neq Bob`) holds for a request to Carol",
        );
        // …and `xone` really is exactly-one: a presentation establishing the `secx:`
        // branch as well makes TWO branches hold, which BREAKS the rule.
        assert!(
            !holds(&requirement, &to_bob, &[REQUIRES_SOUNDNESS]),
            "two holding branches must fail an `xone`",
        );
    }

    /// LOAD-BEARING: two non-`secx:` branches over the same leftOperand differing only
    /// in RIGHT OPERAND are likewise distinct alternatives — a host that saw only
    /// `odrl:recipient` twice could not tell whether Bob or Carol satisfies the `or`,
    /// and would have to re-read the `Policy` the API says it need not.
    #[test]
    fn or_alternatives_differing_only_in_right_operand_stay_distinguishable() {
        let requirement = single_requirement(&policy_with_logical(LogicalConstraint {
            id: "urn:lc:1".into(),
            operator: crate::LogicalOperator::Or,
            operands: vec![
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_HIDING, "PerfectHiding")),
                recipient(Operator::Eq, BOB),
                recipient(Operator::Eq, CAROL),
            ],
        }));

        let DischargeExpr::All(outer) = &requirement else {
            panic!("expected the rule conjunction, got {:?}", requirement);
        };
        let DischargeExpr::Any(alternatives) = &outer[0] else {
            panic!("expected an `or`, got {:?}", outer[0]);
        };
        assert_ne!(
            alternatives[1], alternatives[2],
            "the Bob and Carol alternatives must be distinguishable",
        );

        let to_bob = [(crate::ODRL_RECIPIENT, Value::Iri(BOB.into()))];
        let to_dave = [(
            crate::ODRL_RECIPIENT,
            Value::Iri("https://dave.example/profile#me".into()),
        )];
        assert!(
            holds(&requirement, &to_bob, &[]),
            "the Bob alternative discharges the `or` with no presentation at all",
        );
        assert!(
            !holds(&requirement, &to_dave, &[]),
            "neither recipient alternative holds for Dave, and no proof was presented",
        );
        assert!(
            holds(&requirement, &to_dave, &[REQUIRES_HIDING]),
            "…but the `secx:` alternative still discharges it for Dave",
        );
    }

    /// Compounds nested in compounds are mirrored to the same depth, so an `or` of
    /// `and`s reads back as an `or` of `and`s.
    #[test]
    fn collects_obligations_nested_in_logical_constraints() {
        let inner = LogicalConstraint {
            id: "urn:lc:inner".into(),
            operator: crate::LogicalOperator::And,
            operands: vec![
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_HIDING, "PerfectHiding")),
                ConstraintNode::Atomic(Constraint {
                    left: crate::ODRL_RECIPIENT.into(),
                    operator: Operator::Eq,
                    right: Value::Iri("https://bob.example/profile#me".into()),
                }),
            ],
        };
        let outer = LogicalConstraint {
            id: "urn:lc:outer".into(),
            operator: crate::LogicalOperator::Or,
            operands: vec![
                ConstraintNode::Compound(inner),
                ConstraintNode::Atomic(secprop_constraint(REQUIRES_SOUNDNESS, "Statistical")),
            ],
        };

        let requirement = single_requirement(&policy_with_logical(outer));
        let DischargeExpr::All(top) = &requirement else {
            panic!("expected the rule conjunction, got {:?}", requirement);
        };
        let DischargeExpr::Any(alternatives) = &top[0] else {
            panic!("expected an `or`, got {:?}", top[0]);
        };
        // Alternative 1 is the nested `and` — BOTH its operands, proof and non-proof.
        let DischargeExpr::All(conjuncts) = &alternatives[0] else {
            panic!("expected the nested `and`, got {:?}", alternatives[0]);
        };
        assert_eq!(conjuncts.len(), 2);
        assert!(matches!(conjuncts[0], DischargeExpr::Atomic(_)));
        assert!(matches!(conjuncts[1], DischargeExpr::Other(_)));
        // Alternative 2 is a bare obligation — establishing it alone suffices.
        assert!(matches!(alternatives[1], DischargeExpr::Atomic(_)));

        let dims: Vec<&str> = requirement
            .obligations()
            .iter()
            .map(|o| o.dimension)
            .collect();
        assert_eq!(
            dims,
            vec![DIM_HIDING, DIM_SOUNDNESS],
            "both nested secx: leaves are in the inventory, the core recipient leaf is not",
        );
    }

    /// Duty constraints are deliberately NOT reported: the base-case evaluator checks a
    /// duty by ACTION discharge and never evaluates the duty's own constraints, so
    /// reporting them would misdescribe what the evaluator will check.
    #[test]
    fn duty_constraints_are_not_reported_as_obligations() {
        let mut rule = rule_with("urn:rule:1", vec![]);
        rule.duties = vec![crate::Duty {
            id: "urn:duty:1".into(),
            action: crate::Action(format!("{}anonymize", crate::ODRL_NS)),
            constraints: vec![secprop_constraint(REQUIRES_ANONYMITY, "Anonymous")],
        }];
        let policy = Policy {
            iri: None,
            permissions: vec![rule],
            prohibitions: vec![],
            conflict: None,
            ..Policy::default()
        };
        assert!(
            discharge_requirements(&policy).is_empty(),
            "a duty constraint is not an obligation the evaluator ever checks",
        );
    }

    /// Every Rust leftOperand is declared in the profile TTL as a `secx:LocalName`
    /// subject with its `secx:overDimension` target — the published profile and these
    /// constants cannot diverge.
    #[test]
    fn ttl_declares_every_left_operand_with_its_dimension() {
        for (lo, dim) in SECPROP_LEFT_OPERANDS {
            let lo_local = lo
                .strip_prefix(SECX_NS)
                .expect("leftOperand is in the secx: namespace");
            let dim_local = dim
                .strip_prefix(SECX_NS)
                .expect("dimension is in the secx: namespace");
            // `secx:requiresFoo a owl:NamedIndividual, odrl:LeftOperand, skos:Concept`
            let decl = format!(
                "secx:{} a owl:NamedIndividual, odrl:LeftOperand, skos:Concept",
                lo_local
            );
            assert!(
                TTL.contains(&decl),
                "odrl-secprop-profile.ttl is missing the leftOperand declaration `{}` \
                 (the profile drifted from the Rust constants, sq-uor3g)",
                decl
            );
            // `secx:overDimension secx:Dim`
            let over = format!("secx:overDimension secx:{}", dim_local);
            assert!(
                TTL.contains(&over),
                "odrl-secprop-profile.ttl is missing the overDimension fact `{}` for \
                 leftOperand secx:{} (sq-uor3g)",
                over,
                lo_local
            );
        }
    }

    /// The inverse: every `secx:requires…` leftOperand DECLARED in the TTL is backed
    /// by a Rust constant — no published leftOperand without a constant.
    #[test]
    fn every_ttl_left_operand_has_a_rust_constant() {
        let declared: Vec<&str> = TTL
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let rest = t.strip_prefix("secx:requires")?;
                let name = rest.split_whitespace().next()?;
                // A leftOperand declaration line: `secx:requiresFoo a owl:NamedIndividual …`
                if rest[name.len()..].trim_start().starts_with("a ") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "no secx:requires… leftOperand declarations found in the profile TTL — \
             parser/format drift",
        );
        let constant_locals: std::collections::HashSet<&str> = SECPROP_LEFT_OPERANDS
            .iter()
            .filter_map(|(lo, _)| lo.strip_prefix(&format!("{}requires", SECX_NS)))
            .collect();
        for name in declared {
            assert!(
                constant_locals.contains(name),
                "odrl-secprop-profile.ttl declares `secx:requires{}` but no Rust constant \
                 names it — add the constant or remove the term (sq-uor3g)",
                name
            );
        }
    }

    /// The profile resource + the `odrl:profile` assertion contract are documented in
    /// the TTL — the load-bearing ODRL nuance (a custom leftOperand MUST be declared
    /// in a published profile, design §4.3.1).
    #[test]
    fn profile_resource_and_contract_are_documented() {
        assert!(
            TTL.contains("a odrl:Profile"),
            "the profile TTL must declare the odrl:Profile resource",
        );
        assert!(
            TTL.contains(PROFILE_IRI),
            "the profile TTL must use the published PROFILE_IRI",
        );
        assert!(
            TTL.contains("odrl:profile"),
            "the profile TTL must document the `odrl:profile <…>` assertion contract",
        );
    }

    /// The HONESTY invariant: the profile asserts NO security property and references
    /// the open external-audit gate (sq-qhy4) — this bridge names the bar, it does not
    /// claim any method meets it.
    #[test]
    fn honesty_caveat_is_present() {
        assert!(
            TTL.contains("sq-qhy4"),
            "the profile TTL must reference the open external-audit gate (sq-qhy4)",
        );
        assert!(
            TTL.contains("NO security claim") || TTL.contains("NO security property"),
            "the profile TTL must state it asserts no security property of any method",
        );
    }

    // ── dimension-IRI drift guard (sq-mgxz8, re-homed by sq-3705) ────────────
    //
    // Three sites used to declare `secx:` dimension IRIs independently, with no
    // crate-dependency edge between them:
    //   (1) THIS file — the `DIM_*` constants in `SECPROP_LEFT_OPERANDS`
    //   (2) `sparq-trust/src/secprop.rs` — the full `SECX_*` vocabulary
    //   (3) `sparq-zk/ontologies/secprop-methods.ttl` — the per-method annotation
    //       graph (`secx:property secx:Foo` triples)
    // so a typo in one crate's IRI would not be caught against the others. The gap
    // was filled with `include_str!` reads ACROSS package boundaries, because this
    // lean crate could not take a `sparq-trust` edge without dragging sparq-zk +
    // sparq-canon + sparq-shacl + sparq-reason onto its graph.
    //
    // sq-3705 removed the need for both. (1) and (2) are now literally the same
    // constants — this file's `DIM_*` are `pub use` aliases of the ZERO-dependency
    // `sparq-secprop-vocab` leaf's, which is also where the one TTL↔constant drift
    // test lives — so the guard below is a REAL crate edge over a registry, not a
    // text scan of a sibling package's file. (3) moved to where the annotation graph
    // actually lives: `sparq_zk::secprop`'s `methods_ttl_dims_are_canonical_vocabulary`
    // (it takes the same leaf edge), so this crate reads no other package's files.
    //
    // The `VENDORED_SEC_PROP_DIMS` exemption list is gone with them: since #3441 the
    // three vendored dimensions (`PostQuantumForgery`, `PostQuantumSnooping`,
    // `SignatureTypeLeakage`) are declared as `sec-prop:SecurityProperty` subjects in
    // `secprop-ext.ttl` and carry constants in `ALL_SECPROP_IRIS`, so they satisfy
    // the check directly — retiring the list is the follow-up that test named.

    /// Dimension-IRI drift guard (sq-mgxz8): every dimension in
    /// `SECPROP_LEFT_OPERANDS` is a term of the canonical `secx:` vocabulary, whose
    /// registry `sparq-secprop-vocab` pins to `secprop-ext.ttl`. A leftOperand added
    /// with a typo'd or renamed dimension fails here.
    #[test]
    fn policy_dim_iris_are_canonical_vocabulary_terms() {
        for (left_operand, dim) in SECPROP_LEFT_OPERANDS {
            assert!(
                dim.strip_prefix(SECX_NS).is_some(),
                "dimension `{}` (of `{}`) is not in the secx: namespace",
                dim,
                left_operand,
            );
            assert!(
                sparq_secprop_vocab::ALL_SECPROP_IRIS.contains(dim),
                "dimension `{}` (of `{}`) is not a term of the canonical secx: \
                 vocabulary (`sparq_secprop_vocab::ALL_SECPROP_IRIS`, pinned to \
                 secprop-ext.ttl) — IRI drift between the policy profile and the \
                 vocabulary (sq-mgxz8); check for a rename or a missing declaration",
                dim,
                left_operand,
            );
        }
    }
}
