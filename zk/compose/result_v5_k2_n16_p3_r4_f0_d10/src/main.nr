// sq-j506 (#7103): the METHOD-BOUND successful-result member, contract version 5.
// Identical in PUBLIC layout and private witness to the `result_v1_*` f0 member of
// the same shape; it compiles `result_m5::result_check_m5`, which authenticates
// each credential over the method-bound ZKSIG_M5 message instead of the
// method-UNBOUND ZKSIG_C3 one. A NEW member: every shipped `result_v1_*` /
// `result_v2_*` / `result_v3_*` member stays byte-identical, and the v1 four are
// pinned to an immutable verification-key baseline.
// PUBLIC: challenge, query/result layout, issuer slots, accepted status-policy root.
// PRIVATE: graph roots/contents/sizes/salts, selected rows, signatures and status references.
// Declaration order is the Rust public-input reconstruction contract.
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
    result_check_m5::<u8, K, F, STATUS_DEPTH, 0, 5>(version, accepted_root, issuer_keys, pattern_count, pattern_vars, pattern_constants, projection, result_count, results, filter_count, [], [], [], counts, salts, enc, status_lists, status_versions, status_indices, status_roots, status_siblings, policy_indices, policy_siblings, signatures, selected_graphs, selected_leaves, values, selected_types, selected_hashes, [[]; R]);
}
