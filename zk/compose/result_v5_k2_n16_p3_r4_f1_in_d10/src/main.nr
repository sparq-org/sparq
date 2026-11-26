// Research grade and not externally audited. Method-bound successful-result IN member.
// Public layout authenticates the query/result, issuer slots and status policy as in f0.
// The added public inputs name a bound hidden variable and its typed encoded constant list.
// Private values retain the existing authenticated row bindings; declaration order is the ABI.
use sparq_zk_compose_core::result::{N, P, R, V, STATUS_DEPTH, POLICY_DEPTH};
use sparq_zk_compose_core::result_m5::result_check_m5;
global K: u32 = 2;
global F: u32 = 0;

fn main(
    challenge: pub Field, // fresh verifier challenge
    version: pub Field,
    accepted_root: pub Field,
    issuer_keys: pub [[Field; 2]; K],
    pattern_count: pub u32,
    pattern_vars: pub [[u32; 3]; P],
    pattern_constants: pub [[Field; 3]; P],
    projection: pub [bool; V],
    result_count: pub u32,
    results: pub [[Field; V]; R],
    filter_count: pub u32,
    in_filter_var: pub u32, // bound hidden variable ID from the verifier's query
    in_filter_count: pub u32, // active expected-query constants
    in_filter_constants: pub [Field; 16], // typed RDF encodings, zero-padded
    counts: [u32; K],
    salts: [Field; K],
    enc: [[[Field; 3]; N]; K],
    status_lists: [Field; K],
    status_versions: [u64; K],
    status_indices: [Field; K],
    status_roots: [Field; K],
    status_siblings: [[Field; STATUS_DEPTH]; K],
    policy_indices: [Field; K],
    policy_siblings: [[Field; POLICY_DEPTH]; K],
    signatures: [[Field; 5]; K],
    selected_graphs: [[u32; P]; R],
    selected_leaves: [[u32; P]; R],
    values: [[Field; V]; R],
    selected_types: [[[u32; 3]; P]; R],
    selected_hashes: [[[Field; 3]; P]; R],
) {
    let _ = challenge;
    result_in_core::membership_check::<P, V, R>(pattern_count, pattern_vars, projection, result_count, values, in_filter_var, in_filter_count, in_filter_constants);
    result_check_m5::<u8, K, F, STATUS_DEPTH, 0, 5>(version, accepted_root, issuer_keys, pattern_count, pattern_vars, pattern_constants, projection, result_count, results, filter_count, [], [], [], counts, salts, enc, status_lists, status_versions, status_indices, status_roots, status_siblings, policy_indices, policy_siblings, signatures, selected_graphs, selected_leaves, values, selected_types, selected_hashes, [[]; R]);
}
