use envelope_core::envelope::{
    check_issuer, check_policy, Credential, Membership, PublicPolicy, Signature,
};
// Public: verifier-derived policy words in the authenticated layout order.
// Credential evidence and selected members remain private.
fn main(
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
    c: Credential<4>,
    sig: Signature,
    member: Membership<2>,
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
    let _ = check_issuer(c, issuer_root, issuer_depth, sig, member);
}
