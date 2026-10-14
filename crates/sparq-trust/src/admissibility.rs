//! # `admissibility` — the N3 proof-admissibility ruleset over `sparq-reason`
//!
//! The §4.3 **ODRL → proof-admissibility reduction** of
//! `research/security-properties-ontology-design.md`, run as a genuinely
//! **runnable Notation3 ruleset** on `sparq-reason`'s N3 forward engine
//! ([`sparq_reason::reason_n3_terms`]) — *not* pseudocode. Given a method's
//! [`crate::secprop`] property annotations and a user's ODRL preference (a set
//! of `gteq` constraints over property dimensions), it computes which methods
//! are **admissible**: a method is admissible for a policy iff it **satisfies
//! every** constraint of that policy.
//!
//! ## The reduction (design §4.3.2)
//!
//! Two parts, exactly as the design record states:
//!
//! 1. A **level-order fact base** — each dimension's `⊐` ordering stated once as
//!    ground `secx:strongerThan` facts, **transitively closed** by one N3 rule,
//!    then lifted to a reflexive `secx:atLeast` partial order (the `LEVEL_ORDERS`
//!    and `CLOSURE_RULES` constants).
//! 2. A **constraint-discharge rule** — *method `M` satisfies constraint `C`
//!    iff `M`'s asserted level for `C`'s dimension is `atLeast` `C`'s required
//!    level* (the `DISCHARGE_RULE` constant). One rule for the whole `gteq`
//!    operator; the leftOperand→dimension mapping is one `secx:overDimension`
//!    fact per leftOperand.
//!
//! The reasoner is **monotone** — it only ever *derives* `secx:satisfies`
//! facts, never refutes. The **"satisfies every constraint"** universal
//! (default-deny) is therefore a **Rust side-condition over the materialised
//! `secx:satisfies` facts** ([`admissible`]), *not* in-reasoner negation —
//! exactly as the trust-graph admission gate keeps freshness / holder-binding
//! as Rust side-conditions (the `crate::admit` module), and for the same
//! soundness reason (N3 has no stable negation-as-failure here).
//!
//! ```text
//!   admissible(M, P)  :=  ∀ c ∈ constraints(P) :  (M secx:satisfies c)   # default-deny
//! ```
//!
//! ## What this is — and is NOT (read first)
//!
//! This reasons over **annotations**, not over cryptography. It decides
//! admissibility from the *recorded* (method, property)→level claims of the
//! [`crate::secprop`] vocabulary; it does **not** verify that any method
//! actually has the property it is annotated with. The honesty of the answer
//! is exactly the honesty of the annotation graph fed in — and every sparq ZK
//! property annotation is at most `secx:Claimed` with audit status
//! `secx:ExternalSignOffPending` while the external accredited-cryptographer
//! sign-off is **pending** (`sq-qhy4`); `sparq-mpc` is honest-majority
//! semi-honest only. So `requiresAssurance gteq secx:Proven` mechanically
//! removes **every** sparq ZK method from the admissible set today — and the
//! §4.3.3 worked example (the golden integration test) pins exactly that: an **empty** admissible
//! set under Alice's strict preference, **non-empty** under the relaxed one.
//! The value is a **principled refusal** ("no admissible proof") rather than
//! silently serving a non-conforming one.
//!
//! ## The MPC trust-model axes (issues #3275 / #6186)
//!
//! `LEVEL_ORDERS` also carries the two ordered MPC dimensions — `secx:AdversaryModel`
//! (`Malicious ⊐ Covert ⊐ SemiHonest`) and `secx:CorruptionThreshold`
//! (`DishonestMajority ⊐ HonestMajority ⊐ SuperHonestMajority`, ordered by how much
//! corruption is *tolerated*) — plus their `secx:requiresAdversaryModel` /
//! `secx:requiresCorruptionThreshold` leftOperand mappings. `secx:assumption` remains
//! human-facing PROVENANCE: this reasoner keys strictly on `(dimension, level)`, so an
//! `secx:assumption secx:SemiHonest` triple alone is never discharged — which is why
//! the per-protocol levels exist. With `sparq-mpc`'s annotation graph
//! (`sparq_mpc::secprop::annotations_n3()`, its default-OFF `secprop-annotations`
//! feature) as the `annotations` argument, `requiresAdversaryModel gteq
//! secx:Malicious` removes **every** `sparq-mpc` protocol, and `gteq
//! secx:SemiHonest` admits them. No new rule was needed: the existing discharge rule
//! handles both dimensions.
//!
//! ## Opt-in by construction
//!
//! Behind the **default-OFF `secprop-admissibility`** cargo feature (which
//! enables `secprop-vocab`). It is pure N3-text data + a thin driver over the
//! `sparq-reason` edge this crate already carries — **no new dependency**, and
//! the lean default build is byte-identical with it OFF.
//!
//! sq-ufsi9 (epic sq-0dksu; design §4.3) — Phase 2. 🤖 SPARQ agent —
//! security-properties ontology. Flag for re-review when Fable returns.

use crate::secprop::SEC_PROP_NS;
use std::collections::BTreeSet;

/// `secx:strongerThan` — the per-dimension strict order over levels (the `⊐`
/// relation), stated once per adjacent pair and transitively closed by
/// [`CLOSURE_RULES`].
pub const SECX_STRONGER_THAN: &str = "https://w3id.org/zkp-sparql/sec-prop#strongerThan";
/// `secx:atLeast` — the reflexive partial order a discharge rule reads (a level
/// is `atLeast` itself and everything below it).
pub const SECX_AT_LEAST: &str = "https://w3id.org/zkp-sparql/sec-prop#atLeast";
/// `secx:overDimension` — the leftOperand→dimension mapping (one fact per
/// ODRL leftOperand; design §4.3.2).
pub const SECX_OVER_DIMENSION: &str = "https://w3id.org/zkp-sparql/sec-prop#overDimension";
/// `secx:satisfies` — the discharge-rule conclusion: *this method satisfies
/// this constraint*.
pub const SECX_SATISFIES: &str = "https://w3id.org/zkp-sparql/sec-prop#satisfies";

/// The N3 prefix preamble every ruleset / policy / annotation document shares.
pub const PREFIXES: &str = "\
@prefix secx: <https://w3id.org/zkp-sparql/sec-prop#> .\n\
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .\n\
@prefix zk:   <https://sparq.dev/ns/zk#> .\n";

/// The **level-order fact base** (design §4.3.2): each property dimension's `⊐`
/// ordering as adjacent `secx:strongerThan` ground facts, **plus** the
/// leftOperand→dimension `secx:overDimension` map. Orders match the
/// `secx:strongerThan` chains of the design's vocabulary skeleton (§5b) and the
/// [`crate::secprop`] level constants. Transitively closed by [`CLOSURE_RULES`].
pub const LEVEL_ORDERS: &str = "\
# === level orders (each dimension's ⊐ chain, adjacent pairs; closed by a rule) ===\n\
secx:CrossPresentation secx:strongerThan secx:PerPresentation .\n\
secx:PerPresentation   secx:strongerThan secx:Linkable .\n\
secx:EverlastingUnlinkable secx:strongerThan secx:ComputationalUnlinkable .\n\
secx:PerfectZK     secx:strongerThan secx:StatisticalZK .\n\
secx:StatisticalZK secx:strongerThan secx:ComputationalZK .\n\
secx:ComputationalZK secx:strongerThan secx:NotZK .\n\
secx:KnowledgeSound secx:strongerThan secx:Sound .\n\
secx:Sound          secx:strongerThan secx:Unsound .\n\
secx:PQForgeryResistant secx:strongerThan secx:PQForgeable .\n\
secx:PQHiding secx:strongerThan secx:PQRevealable .\n\
secx:Transparent           secx:strongerThan secx:UniversalTrustedSetup .\n\
secx:UniversalTrustedSetup secx:strongerThan secx:PerCircuitTrustedSetup .\n\
secx:SingleUseEnforced secx:strongerThan secx:Replayable .\n\
secx:SelectivelyDisclosable secx:strongerThan secx:AllOrNothing .\n\
secx:MechanisedProof secx:strongerThan secx:ManualAudit .\n\
secx:ManualAudit     secx:strongerThan secx:Unaudited .\n\
secx:Proven  secx:strongerThan secx:Claimed .\n\
secx:Claimed secx:strongerThan secx:Conjectured .\n\
secx:ExternallyAudited  secx:strongerThan secx:InternallyReviewed .\n\
secx:InternallyReviewed secx:strongerThan secx:Unreviewed .\n\
# the two MPC trust-model axes (issue #6186). Before them, an MPC protocol's\n\
# honest-majority / semi-honest posture was recorded only as `secx:assumption`,\n\
# which this reasoner keys nothing on, so no ODRL preference could exclude a\n\
# semi-honest protocol. CorruptionThreshold is ordered by how much corruption is\n\
# TOLERATED, so dishonest-majority is the STRONGEST level.\n\
secx:Malicious secx:strongerThan secx:Covert .\n\
secx:Covert    secx:strongerThan secx:SemiHonest .\n\
secx:DishonestMajority secx:strongerThan secx:HonestMajority .\n\
secx:HonestMajority    secx:strongerThan secx:SuperHonestMajority .\n\
\n\
# === leftOperand → dimension (one fact per ODRL leftOperand; design §4.3.2/§5b) ===\n\
secx:requiresUnlinkabilityScope secx:overDimension secx:UnlinkabilityScope .\n\
secx:requiresPostQuantumForgery secx:overDimension secx:PostQuantumForgery .\n\
secx:requiresZeroKnowledge       secx:overDimension secx:ZeroKnowledgeType .\n\
secx:requiresSoundness           secx:overDimension secx:Soundness .\n\
secx:requiresSelectiveDisclosure secx:overDimension secx:SelectiveDisclosure .\n\
secx:requiresSingleUse           secx:overDimension secx:SingleUse .\n\
secx:requiresAssurance           secx:overDimension secx:AssuranceLevel .\n\
secx:requiresAdversaryModel      secx:overDimension secx:AdversaryModel .\n\
secx:requiresCorruptionThreshold secx:overDimension secx:CorruptionThreshold .\n";

/// The **order-closure rules** (design §4.3.2): transitively close
/// `secx:strongerThan`, then lift it to the **reflexive** `secx:atLeast`
/// partial order.
///
/// The design states reflexivity as `{ ?x secx:atLeast ?x }`; written as a bare
/// formula with an unbounded variable that is **not range-restricted** (unsafe —
/// it would assert `atLeast` over *every* term in the document). The
/// **range-restricted** equivalent fires reflexivity only for levels that
/// actually appear in an order, which is exactly the set a discharge rule can
/// reference (a held or required level is always on some `strongerThan` chain):
/// `{ ?a secx:strongerThan ?b } => { ?a secx:atLeast ?a . ?b secx:atLeast ?b }`.
pub const CLOSURE_RULES: &str = "\
# transitive closure of the strict order:\n\
{ ?a secx:strongerThan ?b . ?b secx:strongerThan ?c } => { ?a secx:strongerThan ?c } .\n\
# strict ⇒ atLeast:\n\
{ ?a secx:strongerThan ?b } => { ?a secx:atLeast ?b } .\n\
# reflexive (range-restricted to levels that appear in an order — every held /\n\
# required level is on a strongerThan chain, so this is complete for discharge):\n\
{ ?a secx:strongerThan ?b } => { ?a secx:atLeast ?a . ?b secx:atLeast ?b } .\n";

/// The **constraint-discharge rule** (design §4.3.2): a method `M` satisfies a
/// `gteq` constraint `C` iff `M`'s asserted level for `C`'s dimension is
/// `secx:atLeast` `C`'s required level.
///
/// The design writes the method's annotation as the blank-node property list
/// `?m secx:hasProperty [ secx:property ?dim ; secx:level ?have ]`. Inside an N3
/// **rule premise** a `[ … ]` blank is a *constant* blank label (this engine
/// treats `Term::Blank` as ground), so it would **not** join the three atoms
/// through one node. The desugared, explicit-variable form below joins
/// correctly (`?a` is a rule variable that binds the method's assertion node):
pub const DISCHARGE_RULE: &str = "\
{ ?c odrl:leftOperand ?lo ; odrl:operator odrl:gteq ; odrl:rightOperand ?required .\n\
  ?lo secx:overDimension ?dim .\n\
  ?m secx:hasProperty ?a . ?a secx:property ?dim . ?a secx:level ?have .\n\
  ?have secx:atLeast ?required .\n\
} => { ?m secx:satisfies ?c } .\n";

/// The complete admissibility N3 ruleset: [`LEVEL_ORDERS`], then
/// [`CLOSURE_RULES`], then [`DISCHARGE_RULE`] (without the `@prefix` preamble —
/// prepend [`PREFIXES`]). This is the ruleset the design §4.3.2 specifies, ready
/// to feed to [`sparq_reason::reason_n3_terms`] alongside the policy and
/// annotation data.
#[must_use]
pub fn ruleset() -> String {
    format!("{LEVEL_ORDERS}\n{CLOSURE_RULES}\n{DISCHARGE_RULE}")
}

/// The outcome of an admissibility check for one method against one policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admissibility {
    /// `true` iff the method satisfies **every** constraint of the policy
    /// (default-deny: a policy with an unsatisfiable constraint is *not*
    /// admissible; an **empty** constraint set is vacuously admissible — the
    /// caller is responsible for not handing an empty policy where one is
    /// required, exactly as the trust gate treats a Control-gate-less policy).
    pub admissible: bool,
    /// The policy constraint IRIs the method **fails** (does not `secx:satisfies`),
    /// sorted — the honest "why not admissible" detail. Empty iff [`Self::admissible`].
    pub unsatisfied: Vec<String>,
}

/// Run the admissibility reduction for one `method` IRI against a `policy`
/// (a set of `constraint_iris`, each an `odrl:Constraint` carrying
/// `odrl:leftOperand`/`operator odrl:gteq`/`rightOperand`), over the supplied
/// `annotations` (the method's [`crate::secprop`] `secx:hasProperty` graph) and
/// `policy_n3` (the constraints' triples). Both are N3/Turtle fragments **without**
/// the `@prefix` preamble (it is prepended).
///
/// The reduction is the design §4.3 N3 ruleset run on [`sparq_reason::reason_n3_terms`];
/// admissibility itself is the **Rust default-deny universal** over the
/// materialised `secx:satisfies` facts (N3 is monotone — see the module docs).
///
/// `method` and each constraint IRI are full IRIs (no prefix); they are matched
/// against the reasoner's term IRIs. Returns an [`Admissibility`] or the
/// reasoner's parse/closure error.
///
/// **Operator scope (fail-closed):** only the `odrl:gteq` operator is reduced
/// (the design §4.3.2 discharge rule). A constraint with any *other* operator
/// (`eq`/`lt`/…) never produces a `secx:satisfies` fact and is therefore counted
/// **unsatisfied** — the method is denied, never accidentally admitted. A `eq`
/// over a *top* level is expressible as `gteq` that top (only the top satisfies
/// `gteq`-top); richer operators are a later phase (`sq-uor3g` ODRL profile).
///
/// # Errors
/// Propagates any `sparq-reason` parse or closure error from the combined document.
pub fn admissible(
    method: &str,
    constraint_iris: &[&str],
    policy_n3: &str,
    annotations: &str,
) -> Result<Admissibility, String> {
    let doc = format!("{PREFIXES}{policy_n3}\n{annotations}\n{}", ruleset());
    let closure = sparq_reason::reason_n3_terms(&doc, None)?;

    // The materialised (method secx:satisfies constraint) IRI pairs.
    let satisfies = format!("{SEC_PROP_NS}satisfies");
    let mut satisfied: BTreeSet<&str> = BTreeSet::new();
    for [s, p, o] in &closure.facts {
        if iri(p) == Some(satisfies.as_str()) && iri(s) == Some(method) {
            if let Some(c) = iri(o) {
                satisfied.insert(c);
            }
        }
    }

    // Default-deny: the method must satisfy EVERY constraint of the policy.
    let unsatisfied: Vec<String> = constraint_iris
        .iter()
        .filter(|c| !satisfied.contains(*c))
        .map(|c| (*c).to_string())
        .collect();

    Ok(Admissibility { admissible: unsatisfied.is_empty(), unsatisfied })
}

/// The IRI string of a `sparq-reason` `n3::Term`, if it is one. The N3 closure
/// is at the term level (`reason_n3_terms`), where IRIs are `Term::Iri`; non-IRI
/// terms (literals, blanks, formulae) are not subjects of a `secx:satisfies`
/// fact, so `None` correctly skips them.
fn iri(t: &sparq_reason::n3::Term) -> Option<&str> {
    match t {
        sparq_reason::n3::Term::Iri(s) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
