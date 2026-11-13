use envelope_core::envelope::{RequestPolicy, stand_in_request_digest};
// Public: supplied digest, checked against every local request-fixture field.
fn main(
    request_digest: pub [u128; 2],
    query: Field,
    domain: Field,
    challenge: Field,
    policy: RequestPolicy,
) {
    assert(
        request_digest == stand_in_request_digest(query, domain, challenge, policy),
        "request binding",
    );
}
