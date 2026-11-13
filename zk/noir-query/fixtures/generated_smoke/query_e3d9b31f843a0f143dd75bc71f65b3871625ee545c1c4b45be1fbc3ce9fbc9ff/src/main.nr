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
    credential_quad_bounds: pub [u32; 1],
    statement_binding: pub Field,
    credential_0: [[core::suite_entry::TermOpening<32>; 4]; 4],
    credential_proof_0: core::ops_core_suite::CredentialProof<4>,
    counts: [u32; 1],
    credential_count: u32,
    presentation_salt: [u8; 32],
    t0: Table<2, 1>,
    t1: Table<2, 1>,
) {
    ops_core::check_parameters(parameters, 0);
    ops_core::check_indices(indices, 0);
    let credential_flags = core::ops_core_suite::credential_flags::<1>(credential_count, bounds[0], signature_mode);
    let dataset_start = core::ops_core_suite::empty_dataset::<4>();
    let opened_0 = core::ops_core_suite::open_one(credential_0, credential_proof_0, counts[0], credential_flags[0], credential_quad_bounds[0], 0, issuer_root);
    let dataset_0 = core::ops_core_suite::append_credential(dataset_start, opened_0, 0);
    let _dataset = dataset_0;
    core::ops_core_statement::check_suite_dataset([opened_0.message], credential_count, presentation_salt, input_commitment);
    ops_core::unit(t0);
    ops_core::check_bound(t0, bounds[2]);
    ops_core::project(t0, [1], t1);
    ops_core::check_bound(t1, bounds[2]);
    ops_core::ask(t1, result);
    ops_core_statement::bind_ask_authenticated(issuer_root, credential_quad_bounds, request_digest, input_commitment, signature_mode, bounds, parameters, indices, result, statement_binding);
}

