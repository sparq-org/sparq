// Research query program; the selected library entry point defines its credential contract.
use core::table::{TermCell, Table};
use core::{ops_core, ops_core_statement};

fn main(
    request_digest: pub [u8; 32],
    input_commitment: pub [u8; 32],
    signature_mode: pub u8,
    bounds: pub [u32; 3],
    parameters: pub [TermCell; 1],
    indices: pub [u32; 1],
    result: pub bool,
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
    credential_quad_bounds: pub [u32; 1],
    revealed_count: pub u32,
    revealed_messages: pub [Field; 1],
    revealed_keys: pub [Field; 1],
    statement_binding: pub Field,
    credential_0: [core::ops_core_profile::QuadOpening<64, 65>; 4],
    credential_proof_0: core::ops_core_suite::BoundedCredentialProof<4>,
    counts: [u32; 1],
    credential_count: u32,
    presentation_salt: [u8; 32],
    holder_evidence: [core::envelope::HolderEvidence; 1],
    envelope_0: core::ops_core_envelope::CredentialEvidence<4>,
    t0: Table<2, 1>,
    t1: Table<2, 1>,
) {
    ops_core::check_parameters(parameters, 0);
    ops_core::check_indices(indices, 0);
    let policy=core::envelope::PublicPolicy {issuer_root,issuer_depth,holder_required,holder_issuers_present,holder_issuer_count,holder_issuers,status_required,allow_missing_status,snapshot_root,snapshot_depth,validity_required,now,freshness_required,earliest_created};
    core::envelope::check_policy::<4,4>(policy);
    let holder_used_start=[false;1];
    let credential_flags = core::ops_core_suite::credential_mode_flags::<1>(credential_count, bounds[0], signature_mode, revealed_count, revealed_messages, revealed_keys);
    let dataset_start = core::ops_core_suite::empty_dataset::<5>();
    let credential_index_0: u32 = 0;
    let (opened_0, issuer_0, used_0)=core::ops_core_envelope::open_slot::<4,64,65,32,1>(credential_0,credential_proof_0,counts[0],credential_flags[0],credential_quad_bounds[0],credential_index_0,request_digest,signature_mode,revealed_messages[0],revealed_keys[0],policy,envelope_0,holder_evidence);
    let holder_used_0=core::ops_core_envelope::merge_coverage(holder_used_start,used_0);
    let facts_0=core::ops_core_suite::append_credential(dataset_start,opened_0,0);
    let dataset_0=core::ops_core_suite::append_verified_issuer(facts_0,opened_0,issuer_0,credential_flags[0],4);
    let _dataset = dataset_0;
    core::envelope::check_holder_signatures(request_digest,holder_used_0,holder_evidence);
    core::ops_core_statement::check_suite_dataset([opened_0.message()], credential_count, presentation_salt, input_commitment);
    ops_core::unit(t0);
    ops_core::check_bound(t0, bounds[2]);
    ops_core::project(t0, [1], t1);
    ops_core::check_bound(t1, bounds[2]);
    ops_core::ask(t1, result);
    let envelope = ops_core_statement::envelope_words(issuer_root, issuer_depth, holder_required, holder_issuers_present, holder_issuer_count, holder_issuers, status_required, allow_missing_status, snapshot_root, snapshot_depth, validity_required, now, freshness_required, earliest_created);
    ops_core_statement::bind_ask_authenticated_modes(envelope, credential_quad_bounds, request_digest, input_commitment, signature_mode, bounds, parameters, indices, result, revealed_count, revealed_messages, revealed_keys, statement_binding);
}

