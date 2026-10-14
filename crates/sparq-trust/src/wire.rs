//! # `wire.rs` — admitted facts → N3 merge → derived auth grants
//!
//! Takes the [`AdmittedFact`]s the gate produced and the controller-authored ABAC
//! rule (carried in the `.acr` Control-gated channel), feeds the admitted age fact
//! into the **shipped** `sparq-reason` N3 reasoner ahead of the materialiser, and
//! reads off the derived `auth:*` grants — the §3.1 worked example, concretely:
//!
//! ```n3
//! # admitted fact (issuer-tagged, trust:admitted):
//! <Jesse> schema:age 25 .
//! # the .acr ABAC rule (controller-authored, trusted root):
//! { ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <resourceX> } .
//! # ⇒ derived:  <Jesse> auth:read <resourceX> .
//! ```
//!
//! The derived grants are returned as plain `auth:*` triples. The
//! `sparq-solid`-side `trust-graph` feature installs them into `<urn:sparq:auth>`
//! **on top of** the unchanged WAC/ACP view (the ODRL-bridge precedent), so
//! everything downstream — the session-scoped `∪ allow ∖ ∪ deny` enforcement, the
//! query rewrite — is the existing shipped code, untouched.
//!
//! This is the "the existing N3 reasoner merges the admitted age fact with the
//! `.acl`/`.acr` rule" step (§6.0 pipeline step 4): the same full `reason_n3` the
//! materialiser uses, which supports `math:greaterThan`.
//!
//! sq-pfae PoC (issue #940). 🤖 SPARQ agent — trust-graph authorisation PoC.

use crate::admit::{AdmittedFact, StaticAdmittedFact};
use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_core::dict::Dict;
use sparq_reason::reason_n3;
use std::fmt::Write as _;

/// The `auth:` view namespace (`auth:read|write|append|control` — the SAME predicates
/// the shipped WAC/ACP rules emit and `sparq_solid::AuthIndex` reads).
pub const AUTH_NS: &str = "https://sparq.dev/ns/auth#";

/// A derived `auth:*` grant carrying the **DYNAMIC** conditions it still owes — the
/// output of [`derive_conditional_grants`], the materialise-time half of the `sq-xc4y`
/// split. The `sparq-solid` `trust-graph` layer installs each as a **conditional grant**
/// (`auth:ConditionalGrant`) whose `auth:agent` is re-checked against `Session.agent`
/// and whose `auth:notAfter` (`= not_after_unix_secs` rendered as `xsd:dateTime`) is
/// re-checked against `Session.now` PER REQUEST — the shipped sq-0q7n path. The grant is
/// therefore NOT a frozen unconditional allow: a stale or wrong-holder credential is
/// denied at query time without a re-materialise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalGrant {
    /// The derived `auth:*` grant triple (e.g. `<Jesse> auth:read <resourceX>`). Its
    /// subject is always the bound [`holder`](Self::holder) (the credential subject the
    /// `.acr` rule fired on).
    pub grant: Triple,
    /// The bound holder (credential subject). Becomes the conditional grant's
    /// `auth:agent` head — re-checked against `Session.agent` per request (the deferred
    /// DYNAMIC holder binding; never frozen). A different requester never matches.
    pub holder: NamedNode,
    /// The freshness deadline (Unix seconds): `issued_at + freshWithin`. Becomes the
    /// conditional grant's `auth:notAfter` (`xsd:dateTime`) — re-checked against
    /// `Session.now` per request (the deferred DYNAMIC freshness check; never frozen).
    pub not_after_unix_secs: i64,
}

/// Run the N3 merge: the admitted facts + the controller-authored ABAC `rule_n3`
/// (from the `.acr`) → the derived `auth:*` grant triples.
///
/// `rule_n3` is the trusted ABAC rule(s) carried in the Control-gated `.acr` channel,
/// e.g.:
///
/// ```n3
/// @prefix schema: <http://schema.org/> .
/// @prefix math:   <http://www.w3.org/2000/10/swap/math#> .
/// @prefix auth:   <https://sparq.dev/ns/auth#> .
/// { ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
/// ```
///
/// Returns only the `auth:*`-predicated closure triples (the derived grants), as
/// oxrdf [`Triple`]s ready to install into `<urn:sparq:auth>`. NEVER panics on a
/// reasoner error — a malformed rule yields an `Err` the caller fails closed on.
///
/// Honest boundary: the merge admits ONLY directly-attested admitted facts of a
/// trusted type — there is **no entailment laundering** of derived facts through the
/// trust boundary (§3.3 G3). The ABAC rule itself comes from the trusted `.acr`
/// channel, so it is not an admission surface.
pub fn derive_grants(admitted: &[AdmittedFact], rule_n3: &str) -> Result<Vec<Triple>, String> {
    if admitted.is_empty() {
        return Ok(Vec::new()); // default-deny: nothing admitted ⇒ nothing derived
    }
    // Render the admitted facts as N3 ground facts, prepend them to the rule, and run
    // the full `reason_n3` (the same evaluator the materialiser uses — supports
    // math:greaterThan). The facts are the issuer-attested claims, NOT pod content:
    // they reached here only by passing the admission gate.
    let mut src = String::new();
    for f in admitted {
        let _ = writeln!(
            src,
            "{} {} {} .",
            f.triple.subject, f.triple.predicate, f.triple.object
        );
    }
    src.push('\n');
    src.push_str(rule_n3);

    let mut dict = Dict::new();
    let closure = reason_n3(&mut dict, &src)?;

    // Keep only the derived `auth:*` grant triples (the derivation-stratum output).
    let mut grants: Vec<Triple> = Vec::new();
    for t in &closure {
        let p = dict.term(t[1]);
        let Term::NamedNode(pred) = &p else { continue };
        if !pred.as_str().starts_with(AUTH_NS) {
            continue;
        }
        let s = dict.term(t[0]);
        let o = dict.term(t[2]);
        if let Some(triple) = to_triple(s, pred.clone(), o) {
            if !grants.contains(&triple) {
                grants.push(triple);
            }
        }
    }
    Ok(grants)
}

/// Run the N3 merge over **statically-admitted** facts and return each derived
/// `auth:*` grant tagged with the DYNAMIC conditions it still owes (the bound holder +
/// freshness deadline) — the materialise-time half of the `sq-xc4y` static/dynamic
/// split. Use this (not [`derive_grants`]) when feeding the **session-independent**
/// materialise-once view: the returned [`ConditionalGrant`]s carry the per-request
/// holder/freshness so the caller installs a CONDITIONAL grant re-checked at query time,
/// rather than an unconditional allow that would freeze a per-request decision.
///
/// The merge runs **per static fact** so each derived grant keeps its holder/expiry
/// association: a fact only fires `.acr` rules whose head subject is the fact's holder
/// (the credential subject), so the grant's `auth:agent` and its `not_after` stay
/// consistent. A grant whose derived subject is NOT the bound holder is dropped
/// (fail-closed: the credential cannot grant on behalf of a third party — §3.4).
///
/// `rule_n3` is the trusted ABAC rule(s) from the Control-gated `.acr` channel (see
/// [`derive_grants`]). NEVER panics — a malformed rule yields an `Err` the caller fails
/// closed on; an empty input derives nothing (default-deny).
pub fn derive_conditional_grants(
    admitted: &[StaticAdmittedFact],
    rule_n3: &str,
) -> Result<Vec<ConditionalGrant>, String> {
    let mut out: Vec<ConditionalGrant> = Vec::new();
    for f in admitted {
        // The plain AdmittedFact the N3 merge consumes (issuer-tagged).
        let af = AdmittedFact {
            triple: f.triple.clone(),
            issuer: f.issuer.clone(),
        };
        for grant in derive_grants(std::slice::from_ref(&af), rule_n3)? {
            // Fail-closed holder consistency: the derived grant's subject MUST be the
            // bound holder (the credential subject). A rule that derived a grant for a
            // different subject must not ride this credential's holder binding.
            let subj_is_holder = matches!(
                &grant.subject,
                NamedOrBlankNode::NamedNode(n) if n.as_str() == f.holder.as_str()
            );
            if !subj_is_holder {
                continue;
            }
            let cg = ConditionalGrant {
                grant,
                holder: f.holder.clone(),
                not_after_unix_secs: f.not_after_unix_secs,
            };
            if !out.contains(&cg) {
                out.push(cg);
            }
        }
    }
    Ok(out)
}

fn to_triple(s: Term, p: NamedNode, o: Term) -> Option<Triple> {
    let subject = match s {
        Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
        _ => return None,
    };
    Some(Triple::new(subject, p, o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{Literal, NamedNode};

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn admitted_age(value: &str) -> Vec<AdmittedFact> {
        vec![AdmittedFact {
            triple: Triple::new(
                oxrdf::NamedOrBlankNode::NamedNode(iri("https://jesse.ex/card#me")),
                iri("http://schema.org/age"),
                Term::Literal(Literal::new_typed_literal(
                    value,
                    iri("http://www.w3.org/2001/XMLSchema#integer"),
                )),
            ),
            issuer: iri("https://gov.example/issuer"),
        }]
    }

    const RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
"#;

    #[test]
    fn age_over_18_derives_the_read_grant() {
        let grants = derive_grants(&admitted_age("25"), RULE).unwrap();
        assert_eq!(
            grants.len(),
            1,
            "age 25 > 18 derives exactly one read grant"
        );
        let g = &grants[0];
        assert_eq!(g.predicate.as_str(), "https://sparq.dev/ns/auth#read");
        assert!(
            matches!(&g.subject, oxrdf::NamedOrBlankNode::NamedNode(n) if n.as_str() == "https://jesse.ex/card#me")
        );
    }

    #[test]
    fn age_16_admitted_but_rule_denies() {
        // age 16 is ADMITTED (it's a trusted age statement) but the > 18 rule does not
        // fire — so NO grant is derived. The negative case proves the merge is the
        // decision point, not admission.
        let grants = derive_grants(&admitted_age("16"), RULE).unwrap();
        assert!(
            grants.is_empty(),
            "age 16 admitted but rule denies: no grant"
        );
    }

    #[test]
    fn nothing_admitted_derives_nothing() {
        let grants = derive_grants(&[], RULE).unwrap();
        assert!(
            grants.is_empty(),
            "default-deny: empty admission ⇒ empty derivation"
        );
    }

    fn static_age(value: &str, not_after: i64) -> Vec<StaticAdmittedFact> {
        vec![StaticAdmittedFact {
            triple: Triple::new(
                oxrdf::NamedOrBlankNode::NamedNode(iri("https://jesse.ex/card#me")),
                iri("http://schema.org/age"),
                Term::Literal(Literal::new_typed_literal(
                    value,
                    iri("http://www.w3.org/2001/XMLSchema#integer"),
                )),
            ),
            issuer: iri("https://gov.example/issuer"),
            holder: iri("https://jesse.ex/card#me"),
            not_after_unix_secs: not_after,
        }]
    }

    #[test]
    fn conditional_grant_carries_holder_and_expiry() {
        // age 25 > 18 → one conditional read grant, tagged with the holder + deadline.
        let cgs = derive_conditional_grants(&static_age("25", 1_700_086_400), RULE).unwrap();
        assert_eq!(cgs.len(), 1, "age 25 > 18 derives one conditional grant");
        let cg = &cgs[0];
        assert_eq!(
            cg.grant.predicate.as_str(),
            "https://sparq.dev/ns/auth#read"
        );
        assert_eq!(cg.holder.as_str(), "https://jesse.ex/card#me");
        assert_eq!(
            cg.not_after_unix_secs, 1_700_086_400,
            "the freshness deadline rides the grant as a DYNAMIC condition"
        );
    }

    #[test]
    fn conditional_grant_age_16_denies() {
        // age 16 is statically admitted but the > 18 rule does not fire → no grant.
        let cgs = derive_conditional_grants(&static_age("16", 1_700_086_400), RULE).unwrap();
        assert!(cgs.is_empty(), "age 16: rule denies ⇒ no conditional grant");
    }

    #[test]
    fn conditional_grant_drops_third_party_subject() {
        // A rule that grants a DIFFERENT subject than the holder must not ride this
        // credential's holder binding (fail-closed §3.4).
        const THIRD_PARTY_RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { <https://mallory.ex/card#me> auth:read <https://pod.ex/resourceX> } .
"#;
        let cgs =
            derive_conditional_grants(&static_age("25", 1_700_086_400), THIRD_PARTY_RULE).unwrap();
        assert!(
            cgs.is_empty(),
            "a grant whose subject is not the bound holder is dropped"
        );
    }
}
