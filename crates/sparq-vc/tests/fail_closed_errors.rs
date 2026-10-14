//! Public verification and `did:web` resolution reject malformed inputs with the
//! documented error variants. These are authentication/error-behavior checks, not
//! broader cryptographic-soundness claims.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_vc::{
    did::{did_key_for, DidKeyResolver},
    sign, verify, DataIntegrityProof, ProofConfig, SigningKey, VcError, VerifiedProof,
};

fn signed_fixture() -> (Vec<Triple>, DataIntegrityProof) {
    let key = SigningKey::from_seed(&[23_u8; 32]);
    let config = ProofConfig::new(did_key_for(&key.verifying_key()));
    let triples = vec![Triple::new(
        NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("https://example.test/subject")),
        NamedNode::new_unchecked("https://example.test/predicate"),
        Term::Literal(Literal::new_simple_literal("object")),
    )];
    let proof = sign(&triples, &key, &config).expect("fixture signs");
    (triples, proof)
}

fn assert_bad_proof_value(result: Result<VerifiedProof, VcError>, expected_message: &str) {
    match result {
        Err(VcError::BadProofValue(message)) => assert_eq!(message, expected_message),
        other => panic!("expected BadProofValue({expected_message:?}), got {other:?}"),
    }
}

#[test]
fn verification_rejects_proof_value_without_multibase_z_prefix() {
    let (triples, mut proof) = signed_fixture();
    proof.proof_value = "not-multibase".to_string();

    assert_bad_proof_value(
        verify(&triples, &proof, &DidKeyResolver),
        "not multibase-z: not-multibase",
    );
}

#[test]
fn verification_rejects_invalid_base58_proof_value() {
    let (triples, mut proof) = signed_fixture();
    proof.proof_value = "z0".to_string();

    let result = verify(&triples, &proof, &DidKeyResolver);
    match result {
        Err(VcError::BadProofValue(message)) => assert!(
            message.starts_with("base58: "),
            "unexpected BadProofValue detail: {message}"
        ),
        other => panic!("expected a base58 BadProofValue, got {other:?}"),
    }
}

#[test]
fn verification_rejects_non_64_byte_proof_values() {
    let (triples, proof) = signed_fixture();

    for length in [0_usize, 63, 65] {
        let mut malformed = proof.clone();
        malformed.proof_value = format!("z{}", bs58::encode(vec![7_u8; length]).into_string());
        let expected = format!("expected 64-byte signature: {}", malformed.proof_value);

        assert_bad_proof_value(verify(&triples, &malformed, &DidKeyResolver), &expected);
    }
}

#[cfg(feature = "did-web")]
mod did_web {
    use sparq_vc::did::{DidDocumentFetcher, DidError, DidResolver, DidWebResolver};

    const DID: &str = "did:web:example.test";

    struct StubFetcher(Result<&'static str, &'static str>);

    impl DidDocumentFetcher for StubFetcher {
        fn fetch(&self, url: &str) -> Result<String, String> {
            assert_eq!(url, "https://example.test/.well-known/did.json");
            self.0.map(str::to_owned).map_err(str::to_owned)
        }
    }

    #[test]
    fn resolver_preserves_fetch_failures() {
        let resolver = DidWebResolver::new(StubFetcher(Err("transport unavailable")));

        assert_eq!(
            resolver.resolve_str(DID).unwrap_err(),
            DidError::FetchFailed("transport unavailable".to_string())
        );
    }

    #[test]
    fn resolver_rejects_malformed_and_keyless_documents() {
        let malformed = DidWebResolver::new(StubFetcher(Ok("not JSON")));
        match malformed.resolve_str(DID) {
            Err(DidError::NoIssuerKey(message)) => assert!(
                message.starts_with(DID),
                "malformed document error omitted DID: {message}"
            ),
            other => panic!("expected NoIssuerKey for malformed JSON, got {other:?}"),
        }

        let keyless = DidWebResolver::new(StubFetcher(Ok(r#"{"verificationMethod": []}"#)));
        assert_eq!(
            keyless.resolve_str(DID).unwrap_err(),
            DidError::NoIssuerKey(DID.to_string())
        );
    }

    #[test]
    fn resolver_rejects_unsupported_did_methods_before_fetching() {
        let resolver = DidWebResolver::new(StubFetcher(Ok("{}")));

        assert_eq!(
            resolver.resolve_str("did:key:z6MkUnsupported").unwrap_err(),
            DidError::UnsupportedMethod("key".to_string())
        );
    }
}
