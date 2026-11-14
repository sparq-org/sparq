use envelope_core::envelope::{check_policy, CredentialOpening, PublicPolicy};
use envelope_support::open as open_suite_credential;
// Public scope is the stored request digest. All instants use exact epoch milliseconds.
// Roots, depths, flags and lists are verifier-derived public policy values.
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
    proofs: [CredentialOpening<64, 160>; 4],
    holders: [envelope_core::envelope::HolderEvidence; 4],
    slots: [u32; 4],
) {
    let _scope = request_digest;
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
    check_policy::<4, 4>(policy);
    let cs = [
        open_suite_credential(proofs[0], 0),
        open_suite_credential(proofs[1], 1),
        open_suite_credential(proofs[2], 2),
        open_suite_credential(proofs[3], 3),
    ];
    envelope_core::envelope::check_holders(cs, policy, request_digest, slots, holders);
}
