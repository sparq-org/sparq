use envelope_core::envelope::{check_status, Credential, StatusEvidence};
// Public: accepted snapshot-set digest and explicit missing-status policy.
fn main(
    accepted_root: pub Field,
    allow_missing: pub bool,
    c: Credential<8>,
    e: [StatusEvidence<3, 2>; 8],
) {
    check_status(c, accepted_root, allow_missing, e);
}
