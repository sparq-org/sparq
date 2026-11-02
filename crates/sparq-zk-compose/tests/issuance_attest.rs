//! Method authentication round-trips typed issuance for every recognised method.

use sparq_zk::commit::CommitmentMethod;
use sparq_zk::sig::{self, MethodCommitment, SecretKey};
use sparq_zk::Fr;
use sparq_zk_compose::method_auth::{candidate_messages, classify, CommitmentMessageInputs};
use sparq_zk_compose::AuthenticatedMethod;

#[test]
fn attest_authenticates_as_the_bound_method_for_bearer_and_holder() {
    let issuer = SecretKey::from_seed(81);
    let holder = sig::holder_key_digest(&SecretKey::from_seed(82).public_key()).unwrap();
    let salt = Fr::from(83u64);
    let status_ref = Fr::from(84u64);
    for &method in CommitmentMethod::recognised() {
        let c = MethodCommitment { method, commitment: Fr::from(85u64) };
        for holder_digest in [None, Some(&holder)] {
            let signature = sig::signature_from_hex(
                &issuer.attest(&c, &salt, &status_ref, holder_digest),
            ).unwrap();
            let inputs = CommitmentMessageInputs {
                commitment: &c.commitment, salt: &salt, status_ref: &status_ref, holder_digest,
            };
            assert_eq!(classify(&candidate_messages(&inputs), |message|
                sig::verify(&issuer.public_key(), message, &signature)),
                Ok(AuthenticatedMethod::Bound(method)), "{method:?}");
        }
    }
}
