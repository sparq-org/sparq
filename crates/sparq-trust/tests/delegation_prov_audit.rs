//! End-to-end PROV-O delegation-audit tests (design §4.2/§4.3, `sq-pfae.6`).
//!
//! These exercise the **REAL** path — a genuinely signed human → AI-agent delegation
//! chain, the shipped [`invoke`] gate, the [`effective_against_current`] intersection, and
//! then [`audit_invocation`] over the surviving capability — so the audit is proved over an
//! authority that ACTUALLY passed the gate, not a hand-built [`EffectiveCapability`].
//!
//! The load-bearing safety invariant: the audit graph can never record a grant BROADER than
//! the capability the gate conferred. When the current delegator grant has been narrowed
//! (item N), `effective_against_current` intersects it down, and the audit renders only the
//! intersected `auth:*` actions — never the (now-stale) chain-time capability.
//!
//! sq-pfae.6: PROV-O delegation audit for human + AI agents (issue #940, #1190,
//! epic sq-pfae P5). 🤖 SPARQ agent — trust-graph delegation PROV-O audit.
//!
//! This whole suite is gated on the `delegation-prov` feature (the module is feature-gated),
//! so a feature-matrix leg must enable it — see `.github/workflows/feature-matrix.yml`.
#![cfg(feature = "delegation-prov")]

use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_trust::delegation::{
    effective_against_current, hop_message, invoke, Capability, DelegationChain, DelegationHop,
    Invocation,
};
use sparq_trust::delegation_prov::{
    audit_invocation, AuditConfig, PrincipalClassification, PrincipalKind,
};
use sparq_zk::field::{field_from_hash_bytes, Fr};
use sparq_zk::sig::{PublicKey, SecretKey};

// --- fixtures: a human principal delegating to an AI agent -------------------

const HUMAN: &str = "https://alice.ex/card#me"; // human root (trust-anchored)
const AI_AGENT: &str = "https://alice-bot.ex/card#me"; // AI agent (terminal delegate)
const DOCS: &str = "https://pod.ex/docs/";
const DOC_X: &str = "https://pod.ex/docs/x";

const NOW: i64 = 1_700_000_000;
const FAR_FUTURE: i64 = NOW + 365 * 86_400;
const CHALLENGE: [u8; 32] = [0x42u8; 32];

const PROV_NS: &str = "http://www.w3.org/ns/prov#";
const TRUST_NS: &str = "https://sparq.dev/ns/trust#";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn read() -> NamedNode {
    iri("https://sparq.dev/ns/auth#read")
}
fn write() -> NamedNode {
    iri("https://sparq.dev/ns/auth#write")
}
fn prov(local: &str) -> NamedNode {
    iri(&format!("{}{}", PROV_NS, local))
}
fn cap(actions: Vec<NamedNode>, target: &str) -> Capability {
    Capability {
        actions,
        target: iri(target),
    }
}
fn key(seed: u64) -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(seed);
    let pk = sk.public_key();
    (sk, pk)
}

fn signed_hop(
    delegator: &str,
    delegator_sk: &SecretKey,
    delegator_pk: &PublicKey,
    delegate: &str,
    delegate_pk: &PublicKey,
    capability: Capability,
    expires_at_unix_secs: i64,
) -> DelegationHop {
    let mut hop = DelegationHop {
        delegator: iri(delegator),
        delegator_key: *delegator_pk,
        delegate: iri(delegate),
        delegate_key: *delegate_pk,
        capability,
        expires_at_unix_secs,
        signature_hex: String::new(),
    };
    let msg = hop_message(&hop);
    hop.signature_hex = delegator_sk.sign_commitment(&msg);
    hop
}

/// A genuinely signed single-hop chain: the human grants read+write over /docs/ to its AI
/// agent (the AI attenuates is enforced by the gate; here the hop confers read+write so we
/// can later watch the *current grant* narrow it). Returns (chain, roots, ai_sk).
fn human_to_ai_chain() -> (DelegationChain, Vec<PublicKey>, SecretKey) {
    let (human_sk, human_pk) = key(0x2001);
    let (ai_sk, ai_pk) = key(0x2002);
    let hop = signed_hop(
        HUMAN,
        &human_sk,
        &human_pk,
        AI_AGENT,
        &ai_pk,
        cap(vec![read(), write()], DOCS),
        FAR_FUTURE,
    );
    (DelegationChain { hops: vec![hop] }, vec![human_pk], ai_sk)
}

fn invocation(ai_sk: &SecretKey) -> Invocation {
    let challenge_fr: Fr = field_from_hash_bytes(&CHALLENGE);
    Invocation {
        agent: iri(AI_AGENT),
        challenge: CHALLENGE,
        pop_hex: ai_sk.sign_holder_pop(&challenge_fr),
        now_unix_secs: NOW,
    }
}

fn classification() -> PrincipalClassification {
    PrincipalClassification::new()
        .with(iri(HUMAN), PrincipalKind::Human)
        .with(iri(AI_AGENT), PrincipalKind::AiAgent)
}

fn has(graph: &[Triple], s: &str, p: NamedNode, o: Term) -> bool {
    graph.contains(&Triple {
        subject: NamedOrBlankNode::NamedNode(iri(s)),
        predicate: p,
        object: o,
    })
}

// --- the real-path positive case --------------------------------------------

#[test]
fn real_invoke_then_audit_records_on_behalf_of_and_the_conferred_grant() {
    let (chain, roots, ai_sk) = human_to_ai_chain();
    let inv = invocation(&ai_sk);

    // REAL gate: invoke verifies the genuine signature + binds the live PoP.
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("the gate confers the capability");
    assert_eq!(eff.delegate.as_str(), AI_AGENT);

    let cfg = AuditConfig {
        principals: classification(),
        at_time_unix_secs: Some(NOW),
        ..Default::default()
    };
    let audit = audit_invocation(&chain, &eff, &cfg);
    let g = audit.graph();

    // on-behalf-of: the AI agent acted on behalf of the human.
    assert!(
        has(
            g,
            AI_AGENT,
            prov("actedOnBehalfOf"),
            Term::NamedNode(iri(HUMAN))
        ),
        "AI agent prov:actedOnBehalfOf the human"
    );
    // the §4.2 attested attribute: the delegate is an AI agent, the root is human.
    assert!(
        has(
            g,
            AI_AGENT,
            iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::NamedNode(iri(&format!("{}AiAgent", TRUST_NS)))
        ),
        "terminal delegate typed trust:AiAgent"
    );
    assert!(
        has(
            g,
            HUMAN,
            iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            Term::NamedNode(iri(&format!("{}HumanPrincipal", TRUST_NS)))
        ),
        "human root typed trust:HumanPrincipal"
    );
    // the grant rendered as auth:* RDF for the agent, over the CAPABILITY's target (the
    // granted scope /docs/, which covers the requested /docs/x). read+write conferred here.
    assert_eq!(
        eff.capability.target.as_str(),
        DOCS,
        "the conferred scope is /docs/"
    );
    assert!(
        has(g, AI_AGENT, read(), Term::NamedNode(iri(DOCS))),
        "read grant rendered for the AI agent over the granted scope"
    );
    assert!(
        has(g, AI_AGENT, write(), Term::NamedNode(iri(DOCS))),
        "write grant rendered (the chain confers it here)"
    );
    // the activity is associated with the AI agent and timestamped.
    assert!(g
        .iter()
        .any(|t| t.predicate.as_str() == format!("{}atTime", PROV_NS)));
}

// --- the load-bearing safety invariant: audit never exceeds the gate --------

#[test]
fn audit_of_intersected_grant_never_exceeds_the_current_delegator_grant() {
    // The chain confers read+write, but the CURRENT delegator grant has been narrowed to
    // read-only (item N: the human's own grant was revoked down). The intersection must
    // drop write, and the audit must render read ONLY — never the stale chain-time write.
    let (chain, roots, ai_sk) = human_to_ai_chain();
    let inv = invocation(&ai_sk);
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("gate confers");
    assert_eq!(
        eff.capability.actions,
        vec![read(), write()],
        "chain confers both"
    );

    // The current delegator grant: read only over /docs/.
    let current = cap(vec![read()], DOCS);
    let narrowed =
        effective_against_current(&eff, &current).expect("non-empty intersection (read survives)");
    assert_eq!(
        narrowed.capability.actions,
        vec![read()],
        "write dropped by the current-grant intersection"
    );

    let audit = audit_invocation(&chain, &narrowed, &AuditConfig::default());
    let g = audit.graph();
    // The grant renders over the intersected target /docs/ (read only).
    assert!(
        has(g, AI_AGENT, read(), Term::NamedNode(iri(DOCS))),
        "the surviving read is audited"
    );
    assert!(
        !g.iter().any(|t| t.predicate.as_str() == write().as_str()),
        "SAFETY: the revoked write is NOT in the audit — the audit never exceeds the gate"
    );
}

#[test]
fn audit_is_not_an_authority_source_feeding_it_back_grants_nothing() {
    // The audit graph contains `<ai> auth:read <docX>` triples. But re-admitting that graph
    // through the trust gate must grant nothing: there is no admission rule that trusts a
    // prov:actedOnBehalfOf or a rendered auth:* triple — they carry no issuer signature.
    // We assert the structural fact the design relies on: the audit's grant triples are
    // plain `auth:*` assertions with NO `trust:admitted` marker and NO issuer attribution,
    // so they cannot re-enter the admission stratum (which admits only signed facts).
    let (chain, roots, ai_sk) = human_to_ai_chain();
    let inv = invocation(&ai_sk);
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("gate confers");
    let audit = audit_invocation(&chain, &eff, &AuditConfig::default());
    let g = audit.graph();
    let admitted_marker = format!("{}admitted", TRUST_NS);
    assert!(
        !g.iter().any(|t| t.predicate.as_str() == admitted_marker),
        "the audit never marks anything trust:admitted — it is a record, not an admission"
    );
}

#[test]
fn deterministic_node_iris_for_replayability() {
    // Two audits of the same invocation mint the SAME activity/grant IRIs (deterministic),
    // so an auditor can correlate records across runs.
    let (chain, roots, ai_sk) = human_to_ai_chain();
    let inv = invocation(&ai_sk);
    let eff = invoke(&chain, &roots, &inv, &iri(DOC_X)).expect("gate confers");
    let a1 = audit_invocation(&chain, &eff, &AuditConfig::default());
    let a2 = audit_invocation(&chain, &eff, &AuditConfig::default());
    assert_eq!(
        a1.activity(),
        a2.activity(),
        "activity IRI is deterministic"
    );
    assert_eq!(
        a1.grant_entity(),
        a2.grant_entity(),
        "grant-entity IRI is deterministic"
    );
}
