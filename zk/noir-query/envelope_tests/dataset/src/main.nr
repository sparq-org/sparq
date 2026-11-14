use envelope_core::envelope::{check_policy, CredentialOpening, PublicPolicy};
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
    for i in 0..4 {
        let credential_index = i;
        let (c, view) =
            envelope_support::open_and_convert(proofs[i], credential_index, request_digest);
        let issuer = envelope_core::envelope::required_property(
            c,
            c.credential_subject(),
            envelope_core::term::iri_issuer(),
        );
        let _dataset = envelope_core::envelope::credential_dataset::<64, 129, 8>(c, issuer, view);
    }
}
