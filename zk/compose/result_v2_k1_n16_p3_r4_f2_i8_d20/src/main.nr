// [GPT-6] Versioned successful-result relation. Research-stage, not externally audited.
// PUBLIC: challenge, query/result layout, issuer slots, accepted status-policy root.
// PRIVATE: graph roots/contents/sizes/salts, selected rows, signatures and status references.
// Declaration order is the Rust public-input reconstruction contract.
use sparq_zk_compose_core::result::{result_check, N, P, R, V, POLICY_DEPTH};
global K: u32 = 1;
// [GPT-6] Explicit public capacity; actual integer length remains private.
global STATUS_DEPTH: u32 = 20;
global F: u32 = 2;

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
    filter_vars: pub [u32; F],
    filter_ops: pub [u32; F],
    filter_bounds: pub [u64; F],
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
    filter_values: [[u64; F]; R],
) {
    let _ = challenge;
    result_check::<K, F, STATUS_DEPTH, 8, 2>(version, accepted_root, issuer_keys, pattern_count, pattern_vars, pattern_constants, projection, result_count, results, filter_count, filter_vars, filter_ops, filter_bounds, counts, salts, enc, status_lists, status_versions, status_indices, status_roots, status_siblings, policy_indices, policy_siblings, signatures, selected_graphs, selected_leaves, values, selected_types, selected_hashes, filter_values);
}
