use envelope_core::envelope::{check_issuer, Credential, Membership, Signature};
// Public: request-derived authorized key-set digest. All selected issuer data is private.
fn main(trusted_root: pub Field, c: Credential<4>, sig: Signature, member: Membership<2>) {
    let _ = check_issuer(c, trusted_root, sig, member);
}
