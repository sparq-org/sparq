//! W3C vc-di-eddsa `eddsa-rdfc-2022` published test vector, verified through the
//! public API. [OPUS-5.5] zkp-14.2.
//!
//! Testdata origin: W3C Recommendation "Data Integrity EdDSA Cryptosuites v1.0"
//! (15 May 2025), Appendix B.1 (`eddsa-rdfc-2022` representation) —
//! <https://www.w3.org/TR/vc-di-eddsa/#test-vectors>. The canonical document is
//! Example 9, its SHA-256 Example 10, and the `proofValue` Example 16. The vectors
//! are non-normative examples of the normative suite algorithm. Only the PUBLIC
//! test key (the `did:key` below) is used; no private key is needed or present.
//!
//! The eight triples are the RDF form of the example credential, supplied directly:
//! this crate does not perform the JSON-LD-to-RDF transformation. A passing verify
//! shows the hashing/signature path interoperates on this vector; it is not issuer
//! authorization, credential-status checking, or full protocol validation.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sha2::{Digest as _, Sha256};
use sparq_vc::{
    did::{did_key_for, DidKeyResolver},
    verify, DataIntegrityProof, ProofConfig, SigningKey, VcError, CRYPTOSUITE, PROOF_TYPE,
};

const VC: &str = "urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33";
const SUBJECT: &str = "did:example:abcdefgh";
const VM: &str = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2\
                  #z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";
const CREATED: &str = "2023-02-24T23:36:38Z";
const PROOF_VALUE: &str =
    "z2YwC8z3ap7yx1nZYCg4L3j3ApHsF8kgPdSb5xoS1VR7vPG3F561B52hYnQF9iseabecm3ijx4K1FBTQsCZahKZme";

/// Example 9: canonical N-Quads of the unsecured credential.
const EXPECTED_DOC_NQUADS: &str = concat!(
    "<did:example:abcdefgh> <https://www.w3.org/ns/credentials/examples#alumniOf> \"The School of Examples\" .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/ns/credentials/examples#AlumniCredential> .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://schema.org/description> \"A minimum viable example of an Alumni Credential.\" .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://schema.org/name> \"Alumni Credential\" .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#credentialSubject> <did:example:abcdefgh> .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#issuer> <https://vc.example/issuers/5678> .\n",
    "<urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33> <https://www.w3.org/2018/credentials#validFrom> \"2023-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n",
);

/// Example 10: SHA-256 of the canonical document.
const EXPECTED_DOC_SHA256: &str =
    "517744132ae165a5349155bef0bb0cf2258fff99dfe1dbd914b938d775a36017";

fn iri(s: &str) -> NamedNode {
    NamedNode::new_unchecked(s)
}

fn triple(s: &str, p: &str, o: Term) -> Triple {
    Triple::new(NamedOrBlankNode::NamedNode(iri(s)), iri(p), o)
}

fn document() -> Vec<Triple> {
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    vec![
        triple(
            SUBJECT,
            "https://www.w3.org/ns/credentials/examples#alumniOf",
            Term::Literal(Literal::new_simple_literal("The School of Examples")),
        ),
        triple(
            VC,
            rdf_type,
            Term::NamedNode(iri(
                "https://www.w3.org/2018/credentials#VerifiableCredential",
            )),
        ),
        triple(
            VC,
            rdf_type,
            Term::NamedNode(iri(
                "https://www.w3.org/ns/credentials/examples#AlumniCredential",
            )),
        ),
        triple(
            VC,
            "https://schema.org/description",
            Term::Literal(Literal::new_simple_literal(
                "A minimum viable example of an Alumni Credential.",
            )),
        ),
        triple(
            VC,
            "https://schema.org/name",
            Term::Literal(Literal::new_simple_literal("Alumni Credential")),
        ),
        triple(
            VC,
            "https://www.w3.org/2018/credentials#credentialSubject",
            Term::NamedNode(iri(SUBJECT)),
        ),
        triple(
            VC,
            "https://www.w3.org/2018/credentials#issuer",
            Term::NamedNode(iri("https://vc.example/issuers/5678")),
        ),
        triple(
            VC,
            "https://www.w3.org/2018/credentials#validFrom",
            Term::Literal(Literal::new_typed_literal(
                "2023-01-01T00:00:00Z",
                iri("http://www.w3.org/2001/XMLSchema#dateTime"),
            )),
        ),
    ]
}

/// The published proof: `assertionMethod` purpose (the `ProofConfig` default),
/// no domain, no challenge.
fn published_proof() -> DataIntegrityProof {
    DataIntegrityProof {
        proof_type: PROOF_TYPE.to_string(),
        cryptosuite: CRYPTOSUITE.to_string(),
        config: ProofConfig::new(VM).with_created(CREATED),
        proof_value: PROOF_VALUE.to_string(),
    }
}

fn assert_signature_invalid(triples: &[Triple], proof: &DataIntegrityProof, what: &str) {
    match verify(triples, proof, &DidKeyResolver) {
        Err(VcError::SignatureInvalid) => {}
        other => panic!("{what}: expected SignatureInvalid, got {other:?}"),
    }
}

#[test]
fn canonical_document_matches_published_hash() {
    let nquads = sparq_canon::canonicalize_triples(&document())
        .expect("vector document canonicalizes")
        .to_nquads();
    assert_eq!(nquads, EXPECTED_DOC_NQUADS);
    assert_eq!(
        hex::encode(Sha256::digest(nquads.as_bytes())),
        EXPECTED_DOC_SHA256
    );
}

#[test]
fn published_proof_value_verifies() {
    let proof = published_proof();
    let verified = verify(&document(), &proof, &DidKeyResolver)
        .expect("W3C published eddsa-rdfc-2022 proofValue verifies");
    assert_eq!(verified.verification_method, VM);
    assert_eq!(proof.config.proof_purpose, "assertionMethod");
    assert_eq!(proof.config.domain, None);
    assert_eq!(proof.config.challenge, None);
}

#[test]
fn tampered_content_is_rejected() {
    let mut tampered = document();
    tampered[4] = triple(
        VC,
        "https://schema.org/name",
        Term::Literal(Literal::new_simple_literal("Alumni Credential (edited)")),
    );
    assert_signature_invalid(&tampered, &published_proof(), "edited name");

    let mut dropped = document();
    dropped.pop();
    assert_signature_invalid(&dropped, &published_proof(), "dropped validFrom");
}

#[test]
fn tampered_proof_fields_are_rejected() {
    let doc = document();

    let mut created = published_proof();
    created.config.created = Some("2023-02-24T23:36:39Z".to_string());
    assert_signature_invalid(&doc, &created, "created");

    let mut no_created = published_proof();
    no_created.config.created = None;
    assert_signature_invalid(&doc, &no_created, "created removed");

    let mut purpose = published_proof();
    purpose.config.proof_purpose = "authentication".to_string();
    assert_signature_invalid(&doc, &purpose, "proofPurpose");

    // A different, resolvable Ed25519 did:key (public half only).
    let other_key = SigningKey::from_seed(&[9; 32]).verifying_key();
    let mut method = published_proof();
    method.config.verification_method = did_key_for(&other_key);
    assert_signature_invalid(&doc, &method, "verificationMethod");

    let mut domain = published_proof();
    domain.config.domain = Some("vc.example".to_string());
    assert_signature_invalid(&doc, &domain, "domain added");

    let mut challenge = published_proof();
    challenge.config.challenge = Some("nonce".to_string());
    assert_signature_invalid(&doc, &challenge, "challenge added");

    // Last base58 digit bumped: still a 64-byte value, but not the signature.
    let mut value = published_proof();
    value.proof_value = PROOF_VALUE.replace("KZme", "KZmf");
    assert_signature_invalid(&doc, &value, "proofValue");

    let mut suite = published_proof();
    suite.cryptosuite = "eddsa-jcs-2022".to_string();
    assert!(matches!(
        verify(&doc, &suite, &DidKeyResolver),
        Err(VcError::UnsupportedProof(_))
    ));

    let mut proof_type = published_proof();
    proof_type.proof_type = "Ed25519Signature2020".to_string();
    assert!(matches!(
        verify(&doc, &proof_type, &DidKeyResolver),
        Err(VcError::UnsupportedProof(_))
    ));
}
