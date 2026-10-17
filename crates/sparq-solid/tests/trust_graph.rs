//! Trust-graph PoC end-to-end through the REAL `PodStore` (the `trust-graph` feature).
//!
//! Proves: a trusted-government-issued VC `<Jesse> age 25` + a Control-gated trust
//! policy + the `.acr` ABAC rule ⇒ `<Jesse>` gains a real `auth:read` grant on
//! `<resourceX>` that `PodStore::accessible` / `query_as` honour — and the negative
//! cases (untrusted issuer, age 16) do NOT. Plus the strict-additivity check: with the
//! feature compiled in but NO admission call, the store behaves EXACTLY as WAC/ACP.
//!
//! Compiled only under `--features trust-graph`.
//!
//! sq-pfae PoC (issue #940). 🤖 SPARQ agent — trust-graph authorisation PoC.
#![cfg(feature = "trust-graph")]

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};
use sparq_trust::admit::{PresentedCredential, Session as TrustSession};
use sparq_trust::policy::{parse_policy, ControlGate, TrustRule};
use sparq_trust::vocab;
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::sig::{public_key_to_hex, PublicKey, SecretKey};

const JESSE: &str = "https://jesse.ex/card#me";
const GOV_ISSUER: &str = "https://gov.example/issuer";
const SCHEMA_AGE: &str = "http://schema.org/age";
const RESOURCE_X: &str = "https://pod.ex/resourceX";
const SALT: [u8; 32] = [9u8; 32];
const NOW: i64 = 1_700_000_000;

const ACR_ABAC_RULE: &str = r#"@prefix schema: <http://schema.org/> .
@prefix math:   <http://www.w3.org/2000/10/swap/math#> .
@prefix auth:   <https://sparq.dev/ns/auth#> .
{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .
"#;

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

/// A minimal pod: one private resource `resourceX` (a document graph), and an `.acl`
/// granting Alice (NOT Jesse) Read — so the baseline auth view is well-formed but
/// gives Jesse nothing without the trust admission.
fn pod_nquads() -> String {
    format!(
        "<{RESOURCE_X}#it> <https://ex.dev/ns#title> \"x\" <{RESOURCE_X}> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#a> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n"
    )
}

fn age_credential_graph(value: &str) -> Vec<Triple> {
    vec![Triple::new(
        NamedOrBlankNode::NamedNode(iri(JESSE)),
        iri(SCHEMA_AGE),
        Term::Literal(Literal::new_typed_literal(
            value,
            iri("http://www.w3.org/2001/XMLSchema#integer"),
        )),
    )]
}

fn gov_key() -> (SecretKey, PublicKey) {
    let sk = SecretKey::from_seed(0xABCDEF);
    let pk = sk.public_key();
    (sk, pk)
}

fn signed_cred(sk: &SecretKey, value: &str) -> PresentedCredential {
    let graph = age_credential_graph(value);
    let salt = salt_from_bytes(&SALT);
    let commitment = commit_triples(&graph, salt).unwrap();
    PresentedCredential {
        graph,
        issuer_signature_hex: sk.sign_commitment(&commitment.commitment),
        salt: SALT,
        issued_at_unix_secs: NOW - 86_400,
        revoked: false,
    }
}

fn gov_age_rules(issuer: &str, key: &PublicKey) -> Vec<TrustRule> {
    let rule = iri("https://pod.ex/.acr#trustrule");
    let policy = vec![
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::RDF_TYPE),
            Term::NamedNode(iri(vocab::TRUST_RULE)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::SOURCE),
            Term::NamedNode(iri(issuer)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::ISSUER_KEY),
            Term::Literal(Literal::new_simple_literal(public_key_to_hex(key))),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::FOR_PREDICATE),
            Term::NamedNode(iri(SCHEMA_AGE)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule.clone()),
            iri(vocab::SCOPE),
            Term::NamedNode(iri(RESOURCE_X)),
        ),
        Triple::new(
            NamedOrBlankNode::NamedNode(rule),
            iri(vocab::FRESH_WITHIN),
            Term::Literal(Literal::new_simple_literal("P30D")),
        ),
    ];
    parse_policy(&policy, ControlGate::assert_control_gated()).unwrap()
}

fn jesse_can_read_resource_x(store: &mut PodStore) -> bool {
    let jesse = Session {
        agent: Some(JESSE),
        client: None,
        issuer: None,
        now: None,
    };
    store
        .accessible(&jesse, Mode::Read)
        .iter()
        .any(|n| n.as_str() == RESOURCE_X)
}

#[test]
fn age_25_grants_real_podstore_read_end_to_end() {
    let (sk, pk) = gov_key();
    let mut store = PodStore::new(Graph::load_dataset(&pod_nquads(), "nquads").unwrap());
    store.materialize_wac().unwrap();
    // Baseline: Jesse cannot read resourceX (only Alice is granted by the .acl).
    assert!(
        !jesse_can_read_resource_x(&mut store),
        "baseline: no Jesse read"
    );

    let cred = signed_cred(&sk, "25");
    let rules = gov_age_rules(GOV_ISSUER, &pk);
    let session = TrustSession {
        agent: iri(JESSE),
        now_unix_secs: NOW,
    };
    let outcome = store
        .admit_trust_credential_with_rule(&cred, &rules, &session, &iri(RESOURCE_X), ACR_ABAC_RULE)
        .unwrap();
    assert_eq!(outcome.admitted.len(), 1, "the age fact is admitted");
    assert_eq!(
        outcome.installed_grants.len(),
        1,
        "one read grant derived + installed"
    );
    // The REAL PodStore now grants Jesse read on resourceX.
    assert!(
        jesse_can_read_resource_x(&mut store),
        "age 25 from the trusted gov issuer ⇒ <Jesse> auth:read <resourceX> through the \
         real PodStore (admission + N3 merge + materialise)"
    );
    // And a query as Jesse now sees the resource content.
    // sq-gq28y: explicit GRAPH ?g (empty-default spec flip — identical row count
    // for this single-triple probe as the old union-always bare pattern).
    let q = "SELECT ?t WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?t } }";
    let jesse = Session {
        agent: Some(JESSE),
        client: None,
        issuer: None,
        now: None,
    };
    assert_eq!(store.query_as(&jesse, Mode::Read, q).unwrap().rows.len(), 1);
}

#[test]
fn untrusted_issuer_grants_nothing_through_podstore() {
    let (_gov_sk, gov_pk) = gov_key();
    let forge_sk = SecretKey::from_seed(0x111222);
    let mut store = PodStore::new(Graph::load_dataset(&pod_nquads(), "nquads").unwrap());
    store.materialize_wac().unwrap();
    // Credential signed by the forge key; policy trusts the GOV key → no verify → no admit.
    let cred = signed_cred(&forge_sk, "25");
    let rules = gov_age_rules(GOV_ISSUER, &gov_pk);
    let session = TrustSession {
        agent: iri(JESSE),
        now_unix_secs: NOW,
    };
    let outcome = store
        .admit_trust_credential_with_rule(&cred, &rules, &session, &iri(RESOURCE_X), ACR_ABAC_RULE)
        .unwrap();
    assert!(
        outcome.admitted.is_empty(),
        "untrusted issuer ⇒ nothing admitted"
    );
    assert!(
        !jesse_can_read_resource_x(&mut store),
        "untrusted issuer ⇒ no real access"
    );
}

#[test]
fn age_16_admitted_but_no_grant_through_podstore() {
    let (sk, pk) = gov_key();
    let mut store = PodStore::new(Graph::load_dataset(&pod_nquads(), "nquads").unwrap());
    store.materialize_wac().unwrap();
    let cred = signed_cred(&sk, "16");
    let rules = gov_age_rules(GOV_ISSUER, &pk);
    let session = TrustSession {
        agent: iri(JESSE),
        now_unix_secs: NOW,
    };
    let outcome = store
        .admit_trust_credential_with_rule(&cred, &rules, &session, &iri(RESOURCE_X), ACR_ABAC_RULE)
        .unwrap();
    assert_eq!(
        outcome.admitted.len(),
        1,
        "age 16 is admitted (a trusted age statement)"
    );
    assert!(
        outcome.installed_grants.is_empty(),
        "but the >18 rule denies → no grant"
    );
    assert!(
        !jesse_can_read_resource_x(&mut store),
        "age 16 ⇒ no real access"
    );
}

// --- the static/dynamic admission split (sq-xc4y) ---------------------------

/// `now` as an `xsd:dateTime` lexical, for a per-request `Session.now`.
fn at(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    // Hinnant civil_from_days, inline for the test (matches trust_wire's formatter).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let (yy, mm) = if m <= 2 { (y + 1, m) } else { (y, m) };
    format!("{yy:04}-{mm:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn read_at(store: &mut PodStore, agent: &str, now: &str) -> bool {
    let s = Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: Some(now),
    };
    store
        .accessible(&s, Mode::Read)
        .iter()
        .any(|n| n.as_str() == RESOURCE_X)
}

/// THE LOAD-BEARING sq-xc4y SOUNDNESS TEST. Static admission installs a CONDITIONAL
/// grant whose holder + freshness are re-checked PER REQUEST — so the per-request
/// decision is NOT frozen into the materialise-once view:
/// 1. as Jesse, before the deadline → GRANTED;
/// 2. as Jesse, AFTER the freshness deadline → DENIED (the freshness lapsed at query
///    time, without any re-materialise);
/// 3. as a DIFFERENT requester (Mallory), before the deadline → DENIED (the holder
///    binding is re-checked against Session.agent, not frozen as Jesse's allow).
#[test]
fn static_admission_defers_holder_and_freshness_to_query_time() {
    let (sk, pk) = gov_key();
    let mut store = PodStore::new(Graph::load_dataset(&pod_nquads(), "nquads").unwrap());
    store.materialize_wac().unwrap();

    // Credential issued at NOW-1day, P30D window ⇒ deadline = NOW - 1day + 30days.
    let cred = signed_cred(&sk, "25");
    let rules = gov_age_rules(GOV_ISSUER, &pk);
    let outcome = store
        .admit_trust_credential_static(&cred, &rules, &iri(RESOURCE_X), ACR_ABAC_RULE)
        .unwrap();
    assert_eq!(
        outcome.installed_count, 1,
        "one conditional grant installed for the age fact"
    );
    let deadline = cred.issued_at_unix_secs + 30 * 86_400;

    // (1) Jesse, well before the deadline → GRANTED.
    let before = at(NOW);
    assert!(
        read_at(&mut store, JESSE, &before),
        "static admission ⇒ Jesse is granted read while the credential is fresh"
    );

    // (2) Jesse, AFTER the freshness deadline → DENIED, without a re-materialise. This
    // is the proof the freshness is NOT frozen into the view.
    let after = at(deadline + 86_400);
    assert!(
        !read_at(&mut store, JESSE, &after),
        "a request past trust:freshWithin is denied at QUERY TIME — the freshness was \
         deferred to the per-request conditional check, not frozen into the view (sq-xc4y)"
    );

    // (3) Mallory (a different requester), before the deadline → DENIED. The holder
    // binding is re-checked against Session.agent; the grant is pinned to Jesse.
    let mallory = "https://mallory.ex/card#me";
    assert!(
        !read_at(&mut store, mallory, &before),
        "a different requester (Mallory) is denied — the holder binding is re-checked \
         per request against Session.agent, never frozen as a bare allow (sq-xc4y)"
    );

    // (4) And the fresh-Jesse request still works (idempotent, repeatable check).
    assert!(
        read_at(&mut store, JESSE, &before),
        "still granted for fresh Jesse"
    );
}

/// STRICT ADDITIVITY (G6): with the `trust-graph` feature compiled in but NO admission
/// call made, the store's auth view is IDENTICAL to a plain WAC materialise — the
/// feature is additive, never a behaviour change to the WAC/ACP path. (The byte-level
/// feature-OFF parity is asserted by the always-compiled WAC/ACP test suites, which
/// run unchanged in both feature states; this test pins the in-feature no-op.)
#[test]
fn strict_additivity_no_admission_is_unchanged_wac() {
    let mut a = PodStore::new(Graph::load_dataset(&pod_nquads(), "nquads").unwrap());
    a.materialize_wac().unwrap();
    let alice = Session {
        agent: Some("https://alice.ex/card#me"),
        client: None,
        issuer: None,
        now: None,
    };
    // Alice's legit Read is intact; Jesse has nothing — exactly WAC.
    assert!(a
        .accessible(&alice, Mode::Read)
        .iter()
        .any(|n| n.as_str() == RESOURCE_X));
    assert!(!jesse_can_read_resource_x(&mut a));
    // No admission call was made → the auth view is the plain WAC view.
}

/// A resource whose access control document cannot be used gets no trust grant either:
/// the admitted credential installs its grant, and the resource stays withheld.
#[test]
fn trust_grants_do_not_apply_to_a_withheld_resource() {
    let acp = "http://www.w3.org/ns/solid/acp#";
    let acl = "http://www.w3.org/ns/auth/acl#";
    let acr = format!("{RESOURCE_X}.acr");
    let pod = format!(
        "<{RESOURCE_X}#it> <https://ex.dev/ns#title> \"x\" <{RESOURCE_X}> .\n\
         <{acr}> <{acp}accessControl> <{acr}#c> <{acr}> .\n\
         <{acr}#c> <{acp}apply> <{acr}#p> <{acr}> .\n\
         <{acr}#p> <{acp}allow> <{acl}Read> <{acr}> .\n\
         <{acr}#p> <{acp}anyOf> <{acr}#m> <{acr}> .\n\
         <{acr}#m> <{acp}agent> <https://alice.ex/card#me> <{acr}> .\n\
         <https://elsewhere.ex/x#n> <https://ex.dev/ns#t> \"t\" <{acr}> .\n"
    );
    let (sk, pk) = gov_key();
    let mut store = PodStore::new(Graph::load_dataset(&pod, "nquads").unwrap());
    store.materialize_acp().unwrap();
    let cred = signed_cred(&sk, "25");
    let rules = gov_age_rules(GOV_ISSUER, &pk);
    let outcome = store
        .admit_trust_credential_static(&cred, &rules, &iri(RESOURCE_X), ACR_ABAC_RULE)
        .unwrap();
    assert_eq!(outcome.installed_count, 1, "the grant is installed");
    assert!(!read_at(&mut store, JESSE, &at(NOW)), "the withheld resource stays closed");
}
