use envelope_core::envelope::{
    check_holder, check_policy, Credential, holder_applies, Point, PublicPolicy, required_property,
    Signature, vocabulary,
};
// Public: verifier-derived policy words in the authenticated layout order.
// Credential evidence and selected members remain private.
fn main(
    request_digest: pub [u8; 32],
    issuer_root: pub Field,
    issuer_depth: pub u32,
    holder_required: pub bool,
    holder_issuers_present: pub bool,
    holder_issuer_count: pub u32,
    holder_issuers: pub [Field; 4],
    status_required: pub bool,
    allow_missing_status: pub bool,
    snapshot_root: pub Field,
    snapshot_depth: pub u32,
    validity_required: pub bool,
    now: pub u64,
    freshness_required: pub bool,
    earliest_created: pub u64,
    c: Credential<8>,
    pk: Point,
    sig: Signature,
) {
    let policy = PublicPolicy {
        issuer_root,
        issuer_depth,
        holder_required,
        holder_issuers_present,
        holder_issuer_count,
        holder_issuers,
        status_required,
        allow_missing_status,
        snapshot_root,
        snapshot_depth,
        validity_required,
        now,
        freshness_required,
        earliest_created,
    };
    check_policy::<2, 2>(policy);
    if holder_required {
        let (_, issuer) = required_property(c, c.root, vocabulary().issuer);
        if holder_applies(issuer, policy) {
            check_holder(c, request_digest, pk, sig);
        }
    }
}
