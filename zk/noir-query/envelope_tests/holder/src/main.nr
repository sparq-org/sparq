use envelope_core::envelope::{check_holder, Credential, Point, Signature};
// Public: both exact request-digest limbs. Holder key and signature are private.
fn main(request_digest: pub [u128; 2], c: Credential<8>, pk: Point, sig: Signature) {
    check_holder(c, request_digest, pk, sig);
}
