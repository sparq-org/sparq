//! Typed issuance uses the existing method-bound message families.

use sparq_zk::commit::{commit_triples, CommitmentMethod};
use sparq_zk::sig::{self, MethodCommitment, SecretKey};
use sparq_zk::Fr;

#[test]
fn attest_round_trips_every_recognised_method_and_holder_shape() {
    let issuer = SecretKey::from_seed(71);
    let holder = sig::holder_key_digest(&SecretKey::from_seed(72).public_key()).unwrap();
    let salt = Fr::from(73u64);
    let status_ref = Fr::from(74u64);
    for &method in CommitmentMethod::recognised() {
        let c = MethodCommitment { method, commitment: Fr::from(75u64) };
        let scheme = sig::scheme_iri_to_field(method.scheme_iri());
        for holder_digest in [None, Some(&holder)] {
            let signature = sig::signature_from_hex(
                &issuer.attest(&c, &salt, &status_ref, holder_digest),
            ).unwrap();
            let message = match holder_digest {
                None => sig::presentation_message_with_status_and_scheme(
                    &c.commitment, &salt, &status_ref, &scheme,
                ),
                Some(h) => sig::commitment_message_with_holder_and_scheme(
                    &c.commitment, &salt, &status_ref, h, &scheme,
                ),
            };
            assert!(sig::verify(&issuer.public_key(), &message, &signature), "{method:?}");
            for legacy in [
                sig::commitment_message(&c.commitment),
                sig::commitment_message_with_salt(&c.commitment, &salt),
                sig::commitment_message_with_status(&c.commitment, &salt, &status_ref),
                sig::commitment_message_with_holder(&c.commitment, &salt, &status_ref, &holder),
            ] {
                assert!(!sig::verify(&issuer.public_key(), &legacy, &signature), "{method:?}");
            }
        }
    }
}

#[test]
fn string_canonical_builders_expose_their_method_commitment() {
    let salt = Fr::from(76u64);
    let triples = commit_triples(&[], salt).unwrap();
    let graph = sparq_zk::commit::commit_graph_content(&sparq_core::Graph::new(), salt).unwrap();
    for built in [triples, graph] {
        assert_eq!(built.method_commitment(), MethodCommitment {
            method: CommitmentMethod::StringCanonicalV1, commitment: built.commitment,
        });
    }
}

#[cfg(feature = "dual-leaf")]
#[test]
fn dual_leaf_builders_record_the_version_for_issuance() {
    use sparq_zk::{dual_leaf, dual_leaf_v2};
    use sparq_zk::commit::MethodUse;
    let salt = Fr::from(77u64);
    let graph = sparq_core::Graph::new();
    for (method, built) in [
        (CommitmentMethod::DualLeafV1,
            dual_leaf::commit_triples_dual(&[], salt, MethodUse::LegacyAlreadyIssued).unwrap()),
        (CommitmentMethod::DualLeafV1,
            dual_leaf::commit_graph_dual(&graph, salt, MethodUse::LegacyAlreadyIssued).unwrap()),
        (CommitmentMethod::DualLeafV2, dual_leaf_v2::commit_triples(&[], salt).unwrap()),
        (CommitmentMethod::DualLeafV2, dual_leaf_v2::commit_graph(&graph, salt).unwrap()),
    ] {
        assert_eq!(built.method_commitment(), MethodCommitment { method, commitment: built.commitment });
    }
}
