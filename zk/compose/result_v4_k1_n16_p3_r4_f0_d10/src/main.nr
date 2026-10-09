// [OPUS-5.5] beadzkp-15.1: version-four public-pattern successful-result member.
// Research-grade, NOT externally audited; gate count pending EC2 measurement.
// PUBLIC: challenge, query/result layout, issuer slots, policy root, public triple table.
// PRIVATE: graph roots/contents/sizes/salts, selected rows, signatures and status references.
// Declaration order is the Rust public-input reconstruction contract.
use sparq_zk_compose_core::result::{N, P, R, V, POLICY_DEPTH};
use sparq_zk_compose_core::result_public::result_public_check;
global K: u32 = 1;
global STATUS_DEPTH: u32 = 10;

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
    public_triples: pub [[Field; 3]; R],
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
    result_public_check::<K>(version, accepted_root, issuer_keys, pattern_count, pattern_vars, pattern_constants, projection, result_count, results, filter_count, public_triples, counts, salts, enc, status_lists, status_versions, status_indices, status_roots, status_siblings, policy_indices, policy_siblings, signatures, selected_graphs, selected_leaves, values, selected_types, selected_hashes);
}
