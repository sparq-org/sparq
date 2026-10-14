//! End-to-end: sign a graph loaded into the REAL sparq store, then verify through
//! the offline did:key resolver, and confirm tamper-evidence over the store path.
//! Exercises [`sparq_vc::sign_graph`] / [`sparq_vc::verify_graph`] against a graph
//! materialized by `sparq_core::Graph::load_str` (not a hand-built triple slice), so
//! the canonicalize → hash → Ed25519 path runs over real store IDs.

use sparq_core::Graph;
use sparq_vc::{did::DidKeyResolver, sign_graph, verify_graph, ProofConfig, SigningKey, VcError};

const NT: &str = r#"<http://ex/alice> <http://schema.org/name> "Alice" .
<http://ex/alice> <http://schema.org/knows> _:b0 .
_:b0 <http://schema.org/name> "Bob" .
"#;

fn vm_for(key: &SigningKey) -> String {
    let did = key.did_key();
    format!("{did}#{}", did.strip_prefix("did:key:").unwrap())
}

#[test]
fn sign_and_verify_a_loaded_graph() {
    let g = Graph::load_str(NT, "ntriples").expect("load N-Triples");
    let key = SigningKey::from_seed(&[42u8; 32]);
    let cfg = ProofConfig::new(vm_for(&key)).with_created("2026-06-22T12:00:00Z");

    let proof = sign_graph(&g, &key, &cfg).expect("sign graph");
    assert_eq!(proof.cryptosuite, "eddsa-rdfc-2022");
    assert!(proof.proof_value.starts_with('z'));

    let verified = verify_graph(&g, &proof, &DidKeyResolver).expect("verify graph");
    assert_eq!(verified.verification_method, cfg.verification_method);
}

#[test]
fn reordered_reload_verifies_under_the_same_proof() {
    // Same triples, different source order + blank-node label: RDFC-1.0 canonical
    // form is identical, so the proof from the first graph verifies the second.
    let key = SigningKey::from_seed(&[43u8; 32]);
    let cfg = ProofConfig::new(vm_for(&key));

    let g1 = Graph::load_str(NT, "ntriples").unwrap();
    let proof = sign_graph(&g1, &key, &cfg).unwrap();

    let reordered = r#"_:other <http://schema.org/name> "Bob" .
<http://ex/alice> <http://schema.org/knows> _:other .
<http://ex/alice> <http://schema.org/name> "Alice" .
"#;
    let g2 = Graph::load_str(reordered, "ntriples").unwrap();
    assert!(verify_graph(&g2, &proof, &DidKeyResolver).is_ok());
}

#[test]
fn tampered_graph_fails_closed() {
    let key = SigningKey::from_seed(&[44u8; 32]);
    let cfg = ProofConfig::new(vm_for(&key));

    let g = Graph::load_str(NT, "ntriples").unwrap();
    let proof = sign_graph(&g, &key, &cfg).unwrap();

    // A different graph (one literal changed) must not verify under the proof.
    let tampered_nt = NT.replace("\"Bob\"", "\"Mallory\"");
    let tampered = Graph::load_str(&tampered_nt, "ntriples").unwrap();
    assert!(matches!(
        verify_graph(&tampered, &proof, &DidKeyResolver),
        Err(VcError::SignatureInvalid)
    ));
}
