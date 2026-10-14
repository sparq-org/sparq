//! Property tests for functional EdDSA-RDFC-2022 round trips and tamper rejection.
//! These are spec-conformance checks, not a cryptographic-soundness claim.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use proptest::prelude::*;
use sparq_core::Graph;
use sparq_vc::{
    did::{did_key_for, DidKeyResolver},
    sign, sign_graph, verify, verify_graph, ProofConfig, SigningKey, VcError,
};

fn triples_for(values: &[(u16, u16)]) -> Vec<Triple> {
    values
        .iter()
        .enumerate()
        .map(|(index, (predicate, value))| {
            Triple::new(
                NamedOrBlankNode::NamedNode(NamedNode::new_unchecked(format!(
                    "https://example.test/subject/{index}"
                ))),
                NamedNode::new_unchecked(format!("https://example.test/predicate/{predicate}")),
                Term::Literal(Literal::new_simple_literal(value.to_string())),
            )
        })
        .collect()
}

fn graph_for(triples: &[Triple]) -> Graph {
    let mut graph = Graph::new();
    for triple in triples {
        graph
            .insert_triple(
                triple.subject.clone(),
                triple.predicate.clone(),
                triple.object.clone(),
            )
            .expect("generated triple is valid");
    }
    graph
}

fn proof_with_flipped_byte(proof: &sparq_vc::DataIntegrityProof) -> sparq_vc::DataIntegrityProof {
    let mut tampered = proof.clone();
    let encoded = tampered
        .proof_value
        .strip_prefix('z')
        .expect("proof value uses base58btc");
    let mut signature = bs58::decode(encoded)
        .into_vec()
        .expect("generated proof value decodes");
    signature[0] ^= 1;
    tampered.proof_value = format!("z{}", bs58::encode(signature).into_string());
    tampered
}

fn assert_signature_invalid(result: Result<sparq_vc::VerifiedProof, VcError>) {
    assert!(matches!(result, Err(VcError::SignatureInvalid)));
}

proptest! {
    #[test]
    fn sign_verify_round_trips_are_deterministic_and_reject_tampering(
        seed in any::<[u8; 32]>(),
        other_seed in any::<[u8; 32]>(),
        values in prop::collection::vec((any::<u16>(), any::<u16>()), 1..12),
    ) {
        let key = SigningKey::from_seed(&seed);
        let config = ProofConfig::new(did_key_for(&key.verifying_key()));
        let triples = triples_for(&values);
        let graph = graph_for(&triples);

        let proof = sign(&triples, &key, &config).expect("generated triples sign");
        let repeated = sign(&triples, &key, &config).expect("same triples sign again");
        prop_assert_eq!(&proof, &repeated);
        prop_assert!(verify(&triples, &proof, &DidKeyResolver).is_ok());

        let graph_proof = sign_graph(&graph, &key, &config).expect("generated graph signs");
        let repeated_graph_proof =
            sign_graph(&graph, &key, &config).expect("same graph signs again");
        prop_assert_eq!(&graph_proof, &repeated_graph_proof);
        prop_assert!(verify_graph(&graph, &graph_proof, &DidKeyResolver).is_ok());

        let mut added = triples.clone();
        added.push(Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new_unchecked(
                "https://example.test/tampered",
            )),
            NamedNode::new_unchecked("https://example.test/predicate/tampered"),
            Term::Literal(Literal::new_simple_literal("added")),
        ));
        assert_signature_invalid(verify_graph(
            &graph_for(&added),
            &graph_proof,
            &DidKeyResolver,
        ));

        let removed = &triples[1..];
        assert_signature_invalid(verify_graph(
            &graph_for(removed),
            &graph_proof,
            &DidKeyResolver,
        ));

        let mut mutated = triples.clone();
        mutated[0].object = Term::Literal(Literal::new_simple_literal("mutated"));
        assert_signature_invalid(verify_graph(
            &graph_for(&mutated),
            &graph_proof,
            &DidKeyResolver,
        ));

        let flipped = proof_with_flipped_byte(&graph_proof);
        assert_signature_invalid(verify_graph(&graph, &flipped, &DidKeyResolver));

        let mut wrong_key_proof = graph_proof.clone();
        let mut distinct_seed = other_seed;
        if distinct_seed == seed {
            distinct_seed[0] ^= 1;
        }
        let other_key = SigningKey::from_seed(&distinct_seed);
        wrong_key_proof.config.verification_method = did_key_for(&other_key.verifying_key());
        assert_signature_invalid(verify_graph(&graph, &wrong_key_proof, &DidKeyResolver));
    }
}
