//! [SONNET-4.6] sq-ysv3u (issue #2935) — the OPT-IN `acp-vc` trust-the-issuer backend:
//! verify a presented credential's W3C Data Integrity proof with `sparq-vc`, then let the
//! resulting holding satisfy an ACP `acp:vc` matcher end-to-end.
//!
//! The whole file is feature-gated; the `acp:vc` MATCHER semantics (which are unconditional
//! and fail-closed) are covered by `tests/acp_vc.rs` in both feature states.
#![cfg(feature = "acp-vc")]

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_core::Graph;
use sparq_solid::{
    AccessProvenance, Mode, PodStore, Session, VcRequirement, VerifiedCredentials,
};
use sparq_vc::did::DidKeyResolver;
use sparq_vc::{sign, ProofConfig, SigningKey};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const DOC: &str = "https://pod.ex/clinic/d0.ttl";
const OVER_18: &str = "https://issuer.ex/req#OverEighteen";
const AGE_CREDENTIAL: &str = "https://issuer.ex/vocab#AgeCredential";
const CREDENTIAL_SUBJECT: &str = "https://www.w3.org/2018/credentials#credentialSubject";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const AGE_BAND: &str = "https://issuer.ex/vocab#ageBand";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).expect("valid IRI")
}

fn triple(s: &str, p: &str, o: Term) -> Triple {
    Triple::new(NamedOrBlankNode::NamedNode(iri(s)), iri(p), o)
}

/// A credential asserting that `subject` is an `AgeCredential` subject in the `18+` band.
fn credential(subject: &str) -> Vec<Triple> {
    vec![
        triple("https://issuer.ex/creds/1", CREDENTIAL_SUBJECT, Term::NamedNode(iri(subject))),
        triple(subject, RDF_TYPE, Term::NamedNode(iri(AGE_CREDENTIAL))),
        triple(subject, AGE_BAND, Term::Literal(Literal::new_simple_literal("18+"))),
    ]
}

/// A signing key and the `did:key` verification method its proofs carry.
fn issuer_key() -> (SigningKey, String, String) {
    let key = SigningKey::from_seed(&[7u8; 32]);
    let did = key.did_key();
    let vm = format!("{}#{}", did, did.strip_prefix("did:key:").expect("did:key"));
    (key, did, vm)
}

/// The `/clinic/` pod, credential-gated on `OVER_18`.
fn pod() -> String {
    let g = "https://pod.ex/clinic/.acr";
    let mut s = format!("<{DOC}#it> <https://ex.dev/ns#k> \"v\" <{DOC}> .\n");
    s.push_str(&format!("<{g}> <{ACP}memberAccessControl> <{g}#c> <{g}> .\n"));
    s.push_str(&format!("<{g}#c> <{ACP}apply> <{g}#pol> <{g}> .\n"));
    s.push_str(&format!("<{g}#pol> <{ACP}allow> <{ACL}Read> <{g}> .\n"));
    s.push_str(&format!("<{g}#pol> <{ACP}allOf> <{g}#m> <{g}> .\n"));
    s.push_str(&format!("<{g}#m> <{ACP}vc> <{OVER_18}> <{g}> .\n"));
    s
}

fn can_read(s: &PodStore, agent: &str) -> bool {
    let session = Session { agent: Some(agent), client: None, issuer: None, now: None };
    s.accessible(&session, Mode::Read).iter().any(|g| g.as_str() == DOC)
}

/// End-to-end: sign a credential for alice, verify it through the backend, and watch the
/// `acp:vc` policy start granting her — and only her.
#[test]
fn a_verified_credential_grants_its_subject() {
    let (key, did, vm) = issuer_key();
    let cred = credential(ALICE);
    let proof = sign(&cred, &key, &ProofConfig::new(vm)).expect("signs");

    let req = VcRequirement::new(OVER_18, &did, AGE_CREDENTIAL)
        .with_claim(AGE_BAND, Term::Literal(Literal::new_simple_literal("18+")));
    let mut creds = VerifiedCredentials::new();
    let recorded = creds
        .admit_data_integrity(&cred, &proof, &DidKeyResolver, std::slice::from_ref(&req))
        .expect("the proof verifies");
    assert_eq!(recorded, vec![OVER_18.to_owned()]);

    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("loads"));
    store
        .materialize_acp_with_credentials(&AccessProvenance::new(), &creds)
        .expect("materializes");
    assert!(can_read(&store, ALICE));
    assert!(!can_read(&store, BOB));
}

/// A TAMPERED credential does not verify, so no holding is recorded and the policy keeps
/// denying. This is the property the whole backend exists to provide.
#[test]
fn a_tampered_credential_is_refused_and_grants_nothing() {
    let (key, did, vm) = issuer_key();
    let honest = credential(BOB);
    let proof = sign(&honest, &key, &ProofConfig::new(vm)).expect("signs");

    // Bob re-points the signed credential at himself with a better claim.
    let mut tampered = honest.clone();
    tampered[2] = triple(BOB, AGE_BAND, Term::Literal(Literal::new_simple_literal("21+")));

    let req = VcRequirement::new(OVER_18, &did, AGE_CREDENTIAL);
    let mut creds = VerifiedCredentials::new();
    let err = creds
        .admit_data_integrity(&tampered, &proof, &DidKeyResolver, &[req])
        .expect_err("a modified credential must not verify");
    assert!(format!("{}", err).contains("proof did not verify"), "got: {}", err);
    assert!(creds.is_empty(), "a refused credential must record no holding");

    let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("loads"));
    store
        .materialize_acp_with_credentials(&AccessProvenance::new(), &creds)
        .expect("materializes");
    assert!(!can_read(&store, BOB));
}

/// A credential signed by a key the requirement does not name verifies cryptographically but
/// satisfies nothing — the requirement is anchored on ITS issuer, not on any valid signer.
#[test]
fn a_credential_from_an_unaccepted_issuer_satisfies_nothing() {
    let (key, _did, vm) = issuer_key();
    let cred = credential(ALICE);
    let proof = sign(&cred, &key, &ProofConfig::new(vm)).expect("signs");

    let other = SigningKey::from_seed(&[9u8; 32]).did_key();
    let req = VcRequirement::new(OVER_18, other, AGE_CREDENTIAL);
    let mut creds = VerifiedCredentials::new();
    let recorded = creds
        .admit_data_integrity(&cred, &proof, &DidKeyResolver, &[req])
        .expect("the proof itself is valid");
    assert!(recorded.is_empty(), "a different issuer must not satisfy the requirement");
    assert!(creds.is_empty());
}

/// Every claim a requirement states must be present: a valid credential missing one claim
/// (or carrying a different value) satisfies nothing.
#[test]
fn an_unmet_claim_or_type_satisfies_nothing() {
    let (key, did, vm) = issuer_key();
    let cred = credential(ALICE);
    let proof = sign(&cred, &key, &ProofConfig::new(vm)).expect("signs");
    let mut creds = VerifiedCredentials::new();

    // wrong claim VALUE
    let wrong_value = VcRequirement::new(OVER_18, &did, AGE_CREDENTIAL)
        .with_claim(AGE_BAND, Term::Literal(Literal::new_simple_literal("21+")));
    assert!(creds
        .admit_data_integrity(&cred, &proof, &DidKeyResolver, &[wrong_value])
        .expect("valid proof")
        .is_empty());

    // wrong credential TYPE
    let wrong_type = VcRequirement::new(OVER_18, &did, "https://issuer.ex/vocab#OtherCredential");
    assert!(creds
        .admit_data_integrity(&cred, &proof, &DidKeyResolver, &[wrong_type])
        .expect("valid proof")
        .is_empty());

    // the matching requirement still works, so the checks above are not vacuous
    let ok = VcRequirement::new(OVER_18, &did, AGE_CREDENTIAL)
        .with_claim(AGE_BAND, Term::Literal(Literal::new_simple_literal("18+")));
    assert_eq!(
        creds.admit_data_integrity(&cred, &proof, &DidKeyResolver, &[ok]).expect("valid proof"),
        vec![OVER_18.to_owned()]
    );
}

/// A SIGNED credential naming two distinct `credentialSubject`s is refused in EITHER
/// presentation order, and records no holding for either candidate.
///
/// This is the order-dependence property: the `eddsa-rdfc-2022` proof binds to the RDFC-1.0
/// CANONICAL form, so one signed graph verifies whichever order its triples arrive in. A
/// backend that took the first `credentialSubject` it saw would hand the SAME valid
/// credential to alice under one order and to bob under the other — the assertion below that
/// both orders fail on *ambiguity* (not on the proof) is what pins that down.
#[test]
fn a_two_subject_credential_is_refused_in_either_order() {
    let (key, did, vm) = issuer_key();
    let mut cred = credential(ALICE);
    cred.push(triple("https://issuer.ex/creds/1", CREDENTIAL_SUBJECT, Term::NamedNode(iri(BOB))));
    cred.push(triple(BOB, RDF_TYPE, Term::NamedNode(iri(AGE_CREDENTIAL))));
    cred.push(triple(BOB, AGE_BAND, Term::Literal(Literal::new_simple_literal("18+"))));
    let proof = sign(&cred, &key, &ProofConfig::new(vm)).expect("signs");

    // the same signed graph, with bob's `credentialSubject` moved to the front
    let mut bob_first = cred.clone();
    bob_first.swap(0, 3);

    for order in [cred, bob_first] {
        let req = VcRequirement::new(OVER_18, &did, AGE_CREDENTIAL)
            .with_claim(AGE_BAND, Term::Literal(Literal::new_simple_literal("18+")));
        let mut creds = VerifiedCredentials::new();
        let err = creds
            .admit_data_integrity(&order, &proof, &DidKeyResolver, &[req])
            .expect_err("two subjects must be refused, not resolved by slice order");
        assert!(format!("{}", err).contains("ambiguous"), "got: {}", err);
        assert!(creds.is_empty(), "an ambiguous credential must record no holding");

        let mut store = PodStore::new(Graph::load_dataset(&pod(), "nquads").expect("loads"));
        store
            .materialize_acp_with_credentials(&AccessProvenance::new(), &creds)
            .expect("materializes");
        assert!(!can_read(&store, ALICE));
        assert!(!can_read(&store, BOB));
    }
}

/// A credential whose subject is in the reserved `urn:sparq:` principal space is refused
/// before it can be recorded — it could otherwise impersonate a minted pair/triple principal.
#[test]
fn a_reserved_space_subject_is_refused() {
    let (key, did, vm) = issuer_key();
    let cred = credential("urn:sparq:pair?agent=x&client=y");
    let proof = sign(&cred, &key, &ProofConfig::new(vm)).expect("signs");
    let req = VcRequirement::new(OVER_18, &did, AGE_CREDENTIAL);
    let mut creds = VerifiedCredentials::new();
    let err = creds
        .admit_data_integrity(&cred, &proof, &DidKeyResolver, &[req])
        .expect_err("a reserved-space subject must be refused");
    assert!(format!("{}", err).contains("reserved"), "got: {}", err);
    assert!(creds.is_empty());
}
