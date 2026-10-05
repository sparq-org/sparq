// AUTHORED-BY Claude Opus 5
//! The OPT-IN trust-graph admission seam (`trust-graph`, sq-hed3q) — a PURE LIBRARY function,
//! deliberately NOT wired into the LDP handler.
//!
//! The running LWS pod server decides access with the flat-`.acl` WAC authorizer ([`super::wac`]),
//! which has no notion of an issuer-signed credential: a requester either matches an `acl:agent`
//! rule or is denied. The trust-graph estate that DOES model "an issuer I trust has attested a
//! fact about this requester, and my `.acr` rule turns that fact into a grant" lives entirely in
//! the `sparq-trust` admission gate and the `sparq-solid` `PodStore` wiring
//! (`PodStore::admit_trust_credential_static`). This module is the missing adapter: it runs that
//! gate and re-expresses its output in THIS crate's own [`AccessMode`]/[`Decision`] vocabulary.
//!
//! ## What this module is, and what it is deliberately NOT
//!
//! It is ONE pure function — [`trust_admit_verdict`] — plus the additive-union helpers over the
//! [`TrustGrantSet`] it returns ([`TrustGrantSet::union_onto`] and the `Option`-taking
//! [`union_trust_grants`]). It performs no I/O, holds no state, and — the load-bearing scope
//! decision — **is not called from [`crate::ldp::handler`]**. The `authorize_read`/`authorize_mode`
//! call sites are untouched, so enabling the feature changes NO request's outcome. Wiring the seam
//! into the hot path is a separate, soundness-sensitive follow-up; keeping it out here makes this
//! increment testable and behaviour-free.
//!
//! With the feature OFF the module is not compiled at all and neither `sparq-solid` nor
//! `sparq-trust` enters the dependency graph, so the default server build is byte-identical to
//! today (strict additivity — the `sparq-solid` G6 property this mirrors).
//!
//! ## The binding contract
//!
//! A [`TrustGrantSet`] is issued for ONE (requester, resource) pair and remembers it
//! ([`TrustGrantSet::holder`] / [`TrustGrantSet::target`]). Because the type is `Clone` and a caller
//! may hold a verdict across requests, the public combination operations
//! ([`TrustGrantSet::union_onto`] and [`union_trust_grants`]) take the CURRENT request's
//! authenticated WebID and requested resource and RE-CHECK them against that stored pair. On any
//! mismatch — a different resource, a different authenticated agent, or no authenticated agent at
//! all — the WAC decision is returned UNCHANGED. The raw, unchecked union is private, so a grant
//! minted for Jesse/`resourceX` cannot be replayed onto the `Forbidden` decision for
//! Jesse/`resourceY` or for another requester.
//!
//! ## The additivity contract
//!
//! Once that binding check passes, a trust grant is **allow-only**. [`TrustGrantSet::union_onto`]
//! can only ADD modes:
//!
//! - [`Decision::Allow(m)`](Decision::Allow) ⇒ `Allow(m ∪ granted)` — a trust grant can never drop
//!   a WAC allow, and never narrows the `WAC-Allow` advertisement.
//! - [`Decision::Forbidden`] ⇒ `Allow(granted)` — the authenticated-but-unauthorized case is
//!   exactly what a credential-gated resource exists for. This is the ONLY route by which a WAC
//!   denial becomes an allow, and a [`TrustGrantSet`] is unforgeable from outside this module: it
//!   has no public constructor and [`trust_admit_verdict`] returns `Some` only for a grant the
//!   `sparq-trust` gate admitted under a CHECKED issuer signature.
//! - [`Decision::Unauthenticated`] ⇒ unchanged. A deliberate fail-closed NARROWING of the general
//!   contract: that variant means the request carried no verified WebID, so there is no
//!   authenticated agent for the credential's holder binding to bind against. Admitting there
//!   would let a caller name any `session.agent` it liked. The client must authenticate first.
//!
//! A verdict is refused outright (`None`) — never partially honoured — when the `.acr` rule
//! derives an `auth:deny*` for the same requester and resource: an allow-only union has no way to
//! express that denial, so honouring only its sibling allows would invert the controller's intent.
//!
//! ## Which half of the static/dynamic split this uses
//!
//! `sparq-trust` splits admission into a session-INDEPENDENT static class (issuer signature,
//! statement-type scope, reserved-predicate guard, `trust:scope`) and a per-request dynamic class
//! (holder binding, freshness). `sparq_solid::PodStore::admit_trust_credential_static` is the
//! materialise-time path: it decides the static class once and installs a CONDITIONAL grant whose
//! holder/freshness are re-checked per request out of a long-lived `<urn:sparq:auth>` view.
//!
//! This seam has no such view — it is consulted per request, at decision time — so it uses the
//! combined `sparq_trust::admit` (the snapshot half), which evaluates BOTH classes against the
//! live session. That is the correct half for a per-request decision: nothing is frozen, so a
//! stale or wrong-holder credential simply admits nothing on that request. The grant vocabulary
//! (`auth:read|write|append|control`, mapped through [`sparq_solid::Mode`]) is the SAME one the
//! `PodStore` path installs, so a later hot-path wiring can switch to the conditional-grant view
//! without changing this module's output type.
//!
//! ## Honest scope (load-bearing)
//!
//! RESEARCH prototype. This is the **clear path**: the credential is presented in the clear and
//! the holder binding authenticates the requester's WebID in the clear. It makes **no** privacy,
//! unlinkability, anonymity, or ZK-soundness claim, and the ZK estate whose commitment/signature
//! primitives it composes with is externally UNAUDITED (`sq-qhy4`) — the issuer key is
//! operator-asserted. Do not read an admitted grant as a security guarantee.

use std::collections::BTreeSet;

use oxrdf::{NamedNode, NamedOrBlankNode, Term};
use sparq_solid::Mode as SolidMode;
use sparq_trust::admit::admit;
use sparq_trust::wire::derive_grants;

use super::mode::AccessMode;
use super::wac::Decision;

/// The presented credential the gate admits (the claim graph + the issuer signature over its
/// RDFC-1.0 commitment + issuance instant + revocation flag).
pub use sparq_trust::admit::PresentedCredential;
/// The per-request requester context the holder binding and freshness check bind against.
pub use sparq_trust::admit::Session as TrustSession;
/// One parsed, Control-gated trust rule (trusted source + issuer key + statement-type shape +
/// `trust:scope` + freshness window). Build these with `sparq_trust::parse_policy`, which requires
/// a `ControlGate` witness that the policy came from the Control-gated `.acr` channel.
pub use sparq_trust::policy::TrustRule;

/// The `auth:` grant namespace, taken from `sparq-solid` rather than re-spelled here so the two
/// cannot drift.
const AUTH_NS: &str = sparq_solid::AUTH_NS;

/// A derived `auth:*` predicate, split by polarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthPredicate {
    /// `auth:read|write|append|control` — an allow this seam can union.
    Grant(SolidMode),
    /// `auth:denyRead|denyWrite|denyAppend|denyControl` — a denial an allow-only union cannot
    /// express.
    Deny(SolidMode),
}

/// Classify a derived grant predicate IRI. `None` for any IRI outside the `auth:` mode vocabulary
/// (a rule may derive other `auth:`-namespaced triples; they grant nothing here).
fn classify_auth_predicate(iri: &str) -> Option<AuthPredicate> {
    let local = iri.strip_prefix(AUTH_NS)?;
    Some(match local {
        "read" => AuthPredicate::Grant(SolidMode::Read),
        "write" => AuthPredicate::Grant(SolidMode::Write),
        "append" => AuthPredicate::Grant(SolidMode::Append),
        "control" => AuthPredicate::Grant(SolidMode::Control),
        "denyRead" => AuthPredicate::Deny(SolidMode::Read),
        "denyWrite" => AuthPredicate::Deny(SolidMode::Write),
        "denyAppend" => AuthPredicate::Deny(SolidMode::Append),
        "denyControl" => AuthPredicate::Deny(SolidMode::Control),
        _ => return None,
    })
}

/// Bridge the `sparq-solid` auth-view mode onto this crate's WAC [`AccessMode`].
///
/// Exhaustive on purpose: it is the compile-time contract between the two vocabularies, so a new
/// mode on either side breaks the build here rather than being silently dropped from a decision.
fn access_mode(mode: SolidMode) -> AccessMode {
    match mode {
        SolidMode::Read => AccessMode::Read,
        SolidMode::Write => AccessMode::Write,
        SolidMode::Append => AccessMode::Append,
        SolidMode::Control => AccessMode::Control,
    }
}

/// The ADDITIVE grant a verified credential yields for one (requester, resource) pair.
///
/// Constructible ONLY by [`trust_admit_verdict`], and only when the `sparq-trust` gate admitted at
/// least one fact under a checked issuer signature AND the Control-gated `.acr` rule derived at
/// least one `auth:*` allow for that exact requester and resource. That privacy — no public
/// constructor, no public field — is what makes the "a WAC denial can only be lifted by a verified
/// grant" invariant hold by construction rather than by convention.
///
/// The pair it was issued for is carried along in [`holder`](Self::holder)/[`target`](Self::target)
/// and re-checked by [`union_onto`](Self::union_onto), so a clone of this value cannot be replayed
/// onto a different requester's or a different resource's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustGrantSet {
    modes: BTreeSet<AccessMode>,
    holder: NamedNode,
    target: NamedNode,
    issuers: Vec<NamedNode>,
}

impl TrustGrantSet {
    /// The modes granted — always non-empty.
    pub fn modes(&self) -> &BTreeSet<AccessMode> {
        &self.modes
    }

    /// Whether `mode` is among the granted modes.
    pub fn grants(&self, mode: AccessMode) -> bool {
        self.modes.contains(&mode)
    }

    /// The bound holder: the credential subject, which equalled the authenticated requester's
    /// WebID (`Session.agent`) at admission time.
    pub fn holder(&self) -> &NamedNode {
        &self.holder
    }

    /// The resource the grant is scoped to. A grant derived for any other resource is dropped.
    pub fn target(&self) -> &NamedNode {
        &self.target
    }

    /// The trusted sources (`trust:source`) whose attested facts fed the derivation — the audit
    /// trail for why the grant exists. Deduplicated, in first-admitted order.
    pub fn issuers(&self) -> &[NamedNode] {
        &self.issuers
    }

    /// UNION this grant onto a WAC [`Decision`] for the request identified by `agent` and `target`
    /// — the only way a grant reaches a decision.
    ///
    /// `agent` is the CURRENT request's authenticated WebID (`None` for an anonymous request) and
    /// `target` the CURRENT requested resource. Per the module's binding contract, the union
    /// happens ONLY when both equal this grant's [`holder`](Self::holder) and
    /// [`target`](Self::target); on any mismatch `wac` is returned UNCHANGED, so a grant issued for
    /// one (requester, resource) pair cannot lift another pair's denial.
    ///
    /// When the binding holds the union is allow-only, per the module's additivity contract: an
    /// existing `Allow` gains modes and never loses any; a [`Decision::Forbidden`] becomes an
    /// `Allow` of exactly the granted modes; a [`Decision::Unauthenticated`] is returned unchanged
    /// (no authenticated WebID exists for the holder binding to have bound against).
    pub fn union_onto(
        &self,
        wac: Decision,
        agent: Option<&NamedNode>,
        target: &NamedNode,
    ) -> Decision {
        if agent != Some(&self.holder) || target != &self.target {
            return wac;
        }
        self.union_onto_bound(wac)
    }

    /// The raw allow-only union, PRIVATE so the binding check in [`union_onto`](Self::union_onto)
    /// is the only public route into a decision.
    fn union_onto_bound(&self, wac: Decision) -> Decision {
        match wac {
            Decision::Allow(mut modes) => {
                modes.extend(self.modes.iter().copied());
                Decision::Allow(modes)
            }
            Decision::Forbidden => Decision::Allow(self.modes.clone()),
            Decision::Unauthenticated => Decision::Unauthenticated,
        }
    }
}

/// Apply an OPTIONAL trust verdict to a WAC decision for the request identified by `agent` (the
/// current authenticated WebID, `None` when anonymous) and `target` (the current requested
/// resource).
///
/// `None` (nothing admitted) leaves the decision exactly as WAC decided it, as does a grant whose
/// stored holder/target do not match this request — see [`TrustGrantSet::union_onto`].
pub fn union_trust_grants(
    wac: Decision,
    grants: Option<&TrustGrantSet>,
    agent: Option<&NamedNode>,
    target: &NamedNode,
) -> Decision {
    match grants {
        Some(g) => g.union_onto(wac, agent, target),
        None => wac,
    }
}

/// Run the trust-graph admission gate for one request and return the ADDITIVE grant it yields, or
/// `None` when it yields nothing.
///
/// `credential` is the presented, issuer-signed claim graph; `rules` is the parsed **Control-gated**
/// trust policy (build it with `sparq_trust::parse_policy`); `session` is the per-request requester
/// context (authenticated WebID + `now`); `target` is the requested resource; `abac_rule_n3` is the
/// controller-authored ABAC rule carried in the Control-gated `.acr` channel, e.g.
///
/// ```n3
/// { ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
/// ```
///
/// # Fail-closed behaviour
///
/// Returns `None` — never an error, and never a partial grant — for every failure mode:
/// a malformed or forged issuer signature, an uncanonicalisable claim graph, a credential whose
/// subject is not the authenticated requester, one issued outside the rule's freshness window, a
/// revoked one, one outside the rule's `trust:scope`, an unparseable `.acr` rule, a rule that
/// derives nothing, a grant derived for a different requester or a different resource, or a rule
/// that derives an `auth:deny*` for this requester and resource (which an allow-only union cannot
/// honour, so the whole verdict is refused). Never panics on adversarial input.
///
/// # Honest scope
///
/// RESEARCH prototype, clear path. No privacy, unlinkability, anonymity, or ZK-soundness claim;
/// the ZK estate the underlying commitment/signature primitives come from is externally unaudited
/// (`sq-qhy4`). See the module docs.
pub fn trust_admit_verdict(
    credential: &PresentedCredential,
    rules: &[TrustRule],
    session: &TrustSession,
    target: &NamedNode,
    abac_rule_n3: &str,
) -> Option<TrustGrantSet> {
    // The combined (static + dynamic) gate: checked issuer signature over the RDFC-1.0 commitment,
    // `trust:scope` containment, freshness, revocation, the reserved-predicate guard, the
    // statement-type shape, and the holder binding. Anything that fails admits nothing.
    let admitted = admit(credential, rules, session, target);
    if admitted.is_empty() {
        return None;
    }

    // Merge the admitted facts with the Control-gated `.acr` rule. A malformed rule is an Err the
    // caller must fail closed on — here that is simply "no grant".
    let derived = derive_grants(&admitted, abac_rule_n3).ok()?;

    let mut modes: BTreeSet<AccessMode> = BTreeSet::new();
    for triple in &derived {
        // The grant must name the authenticated requester as its subject. `admit` already binds
        // each admitted FACT to the holder, but the `.acr` rule head is controller-authored and
        // could name a third party; such a grant is not this requester's to use.
        let NamedOrBlankNode::NamedNode(subject) = &triple.subject else {
            continue;
        };
        if subject != &session.agent {
            continue;
        }
        // ...and the resource actually being requested. A grant scoped elsewhere is dropped.
        let Term::NamedNode(object) = &triple.object else {
            continue;
        };
        if object != target {
            continue;
        }
        match classify_auth_predicate(triple.predicate.as_str()) {
            Some(AuthPredicate::Grant(mode)) => {
                modes.insert(access_mode(mode));
            }
            // An allow-only union cannot express a denial, so honouring the sibling allows would
            // invert the controller's intent. Refuse the whole verdict.
            Some(AuthPredicate::Deny(_)) => return None,
            None => continue,
        }
    }

    if modes.is_empty() {
        return None;
    }

    let mut issuers: Vec<NamedNode> = Vec::new();
    for fact in &admitted {
        if !issuers.contains(&fact.issuer) {
            issuers.push(fact.issuer.clone());
        }
    }

    Some(TrustGrantSet {
        modes,
        holder: session.agent.clone(),
        target: target.clone(),
        issuers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    const JESSE: &str = "https://jesse.ex/card#me";
    const MALLORY: &str = "https://mallory.ex/card#me";
    const RESOURCE_X: &str = "https://pod.ex/resourceX";
    const RESOURCE_Y: &str = "https://pod.ex/resourceY";

    /// A grant set built directly — only reachable from inside the module, which is precisely the
    /// property the union tests are exercising. Bound to Jesse/`resourceX`.
    fn grant_set(modes: &[AccessMode]) -> TrustGrantSet {
        TrustGrantSet {
            modes: modes.iter().copied().collect(),
            holder: iri(JESSE),
            target: iri(RESOURCE_X),
            issuers: vec![iri("https://gov.example/issuer")],
        }
    }

    /// The request the [`grant_set`] above was issued for.
    fn bound_request() -> (NamedNode, NamedNode) {
        (iri(JESSE), iri(RESOURCE_X))
    }

    #[test]
    fn auth_predicates_classify_by_polarity() {
        assert_eq!(
            classify_auth_predicate("https://sparq.dev/ns/auth#read"),
            Some(AuthPredicate::Grant(SolidMode::Read))
        );
        assert_eq!(
            classify_auth_predicate("https://sparq.dev/ns/auth#control"),
            Some(AuthPredicate::Grant(SolidMode::Control))
        );
        assert_eq!(
            classify_auth_predicate("https://sparq.dev/ns/auth#denyRead"),
            Some(AuthPredicate::Deny(SolidMode::Read))
        );
        // Outside the mode vocabulary, and outside the namespace entirely.
        assert_eq!(
            classify_auth_predicate("https://sparq.dev/ns/auth#agent"),
            None
        );
        assert_eq!(
            classify_auth_predicate("http://www.w3.org/ns/auth/acl#Read"),
            None
        );
    }

    #[test]
    fn solid_modes_bridge_onto_wac_access_modes() {
        assert_eq!(access_mode(SolidMode::Read), AccessMode::Read);
        assert_eq!(access_mode(SolidMode::Write), AccessMode::Write);
        assert_eq!(access_mode(SolidMode::Append), AccessMode::Append);
        assert_eq!(access_mode(SolidMode::Control), AccessMode::Control);
    }

    #[test]
    fn union_onto_allow_adds_modes_and_drops_none() {
        let (agent, target) = bound_request();
        let existing: BTreeSet<AccessMode> = [AccessMode::Read, AccessMode::Append].into();
        let after = grant_set(&[AccessMode::Write]).union_onto(
            Decision::Allow(existing),
            Some(&agent),
            &target,
        );
        assert_eq!(
            after,
            Decision::Allow([AccessMode::Read, AccessMode::Append, AccessMode::Write].into())
        );
    }

    #[test]
    fn union_onto_forbidden_allows_exactly_the_granted_modes() {
        let (agent, target) = bound_request();
        let after =
            grant_set(&[AccessMode::Read]).union_onto(Decision::Forbidden, Some(&agent), &target);
        assert_eq!(after, Decision::Allow([AccessMode::Read].into()));
    }

    #[test]
    fn union_onto_never_lifts_unauthenticated() {
        // No authenticated WebID exists for the holder binding to have bound against, so even a
        // Control grant must leave the 401 in place — even if a caller passes the bound holder.
        let (agent, target) = bound_request();
        let after = grant_set(&[AccessMode::Control]).union_onto(
            Decision::Unauthenticated,
            Some(&agent),
            &target,
        );
        assert_eq!(after, Decision::Unauthenticated);
    }

    #[test]
    fn union_onto_refuses_a_target_other_than_the_one_the_grant_was_issued_for() {
        // The SAME grant object, replayed onto another resource's denial: the binding must hold it
        // off, leaving both the 403 and an unrelated allow exactly as WAC decided them.
        let grant = grant_set(&[AccessMode::Read, AccessMode::Control]);
        let (agent, _) = bound_request();
        let other = iri(RESOURCE_Y);
        assert_eq!(
            grant.union_onto(Decision::Forbidden, Some(&agent), &other),
            Decision::Forbidden
        );
        let allow = Decision::Allow([AccessMode::Append].into());
        assert_eq!(
            grant.union_onto(allow.clone(), Some(&agent), &other),
            allow,
            "an unrelated resource must not gain the grant's modes"
        );
    }

    #[test]
    fn union_onto_refuses_an_agent_other_than_the_bound_holder() {
        let grant = grant_set(&[AccessMode::Read]);
        let (_, target) = bound_request();
        // Another authenticated requester...
        assert_eq!(
            grant.union_onto(Decision::Forbidden, Some(&iri(MALLORY)), &target),
            Decision::Forbidden
        );
        // ...and no authenticated requester at all.
        assert_eq!(
            grant.union_onto(Decision::Forbidden, None, &target),
            Decision::Forbidden
        );
    }

    #[test]
    fn union_trust_grants_without_a_verdict_is_the_identity() {
        let (agent, target) = bound_request();
        assert_eq!(
            union_trust_grants(Decision::Forbidden, None, Some(&agent), &target),
            Decision::Forbidden
        );
        let allow = Decision::Allow([AccessMode::Read].into());
        assert_eq!(
            union_trust_grants(allow.clone(), None, Some(&agent), &target),
            allow
        );
    }

    #[test]
    fn union_trust_grants_enforces_the_same_binding_as_union_onto() {
        let grant = grant_set(&[AccessMode::Read]);
        let (agent, target) = bound_request();
        // Bound: the grant applies.
        assert_eq!(
            union_trust_grants(Decision::Forbidden, Some(&grant), Some(&agent), &target),
            Decision::Allow([AccessMode::Read].into())
        );
        // Wrong resource / wrong agent: it does not.
        assert_eq!(
            union_trust_grants(
                Decision::Forbidden,
                Some(&grant),
                Some(&agent),
                &iri(RESOURCE_Y)
            ),
            Decision::Forbidden
        );
        assert_eq!(
            union_trust_grants(
                Decision::Forbidden,
                Some(&grant),
                Some(&iri(MALLORY)),
                &target
            ),
            Decision::Forbidden
        );
    }

    #[test]
    fn grant_set_accessors_report_the_admitted_shape() {
        let gs = grant_set(&[AccessMode::Read]);
        assert!(gs.grants(AccessMode::Read));
        assert!(!gs.grants(AccessMode::Write));
        assert_eq!(gs.modes(), &BTreeSet::from([AccessMode::Read]));
        assert_eq!(gs.holder().as_str(), JESSE);
        assert_eq!(gs.target().as_str(), RESOURCE_X);
        assert_eq!(gs.issuers(), &[iri("https://gov.example/issuer")]);
    }
}
