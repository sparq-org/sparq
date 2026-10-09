// [OPUS-5.5] beadzkp-15.1: version-four public-pattern result relation.
// Research-grade, NOT externally audited; no gate or runtime saving is claimed
// before the pending EC2 measurement.
//! Successful-result support where pattern zero is fully determined by public data.
//!
//! The verifier derives `public_triples` from the query and released rows alone:
//! pattern zero is the first original-order pattern whose every slot is a
//! constant or a projected variable. It checks IRI/literal slot kinds on those
//! public terms itself, so this relation compares the privately selected leaf
//! directly to the public row triple, omitting typed openings and slot muxes.
//! Every credential signature, hidden root, salt, status reference, policy path,
//! and leaf membership check is retained. Remaining patterns use the generic check.
//! Hidden numeric filters are unsupported, so filter_count must be zero.
//!
//! `result::result_check` source is intentionally left byte-for-byte unchanged.
//! Unchanged source is not by itself a guarantee of identical compiled keys;
//! version-one key equivalence is tested separately against pinned baselines. The helpers
//! below therefore mirror its authentication and generic-pattern checks; any
//! divergence from `result_check` other than pattern zero is a bug.

use crate::hashes::{commit_fold, h2, h3, h4};
use crate::issuer::{schnorr_verify, Point};
use crate::revoke::revoke_unset_check;
use crate::result::{N, P, R, V, POLICY_DEPTH};

/// Exact public contract version for this relation; distinct from versions one to three.
pub global PUBLIC_PATTERN_VERSION: u32 = 4;
/// Only the depth-10 status tree is published for this first specialization.
pub global PUBLIC_PATTERN_STATUS_DEPTH: u32 = 10;
// Same sig.rs domain tags as `result.nr`; changing either copy alone breaks proofs.
global STATUS_REF: Field = 0x5a4b5349475f5352;
global COMMITMENT_STATUS: Field = 0x5a4b5349475f4333;
global ACCEPTED_STATUS: Field = 0x5a4b5349475f414c;

/// Authenticates every credential slot: root, signature, policy leaf, and live status bit.
pub fn authenticate_credentials<let K: u32, let SD: u32>(
    accepted_root: Field,
    issuer_keys: [[Field; 2]; K],
    counts: [u32; K],
    salts: [Field; K],
    enc: [[[Field; 3]; N]; K],
    status_lists: [Field; K],
    status_versions: [u64; K],
    status_indices: [Field; K],
    status_roots: [Field; K],
    status_siblings: [[Field; SD]; K],
    policy_indices: [Field; K],
    policy_siblings: [[Field; POLICY_DEPTH]; K],
    signatures: [[Field; 5]; K],
) {
    for g in 0..K {
        assert((counts[g] > 0) & (counts[g] <= N), "graph capacity");
        let mut leaves = [0; N];
        for i in 0..N {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
            for s in 0..3 {
                assert((i < counts[g]) | (enc[g][i][s] == 0), "noncanonical graph padding");
            }
        }
        let root = commit_fold(leaves, counts[g]);
        // Independently bind each credential's own list, version, and index.
        let reference = h4(STATUS_REF, status_lists[g], status_indices[g], status_versions[g] as Field);
        let message = h4(COMMITMENT_STATUS, root, salts[g], reference);
        let sig = signatures[g];
        schnorr_verify(
            Point { x: issuer_keys[g][0], y: issuer_keys[g][1] },
            Point { x: sig[0], y: sig[1] }, sig[2], sig[3], sig[4], message,
        );
        // The policy leaf atomically binds list identity, version, and live-bit root.
        let mut node = h4(ACCEPTED_STATUS, status_lists[g], status_versions[g] as Field, status_roots[g]);
        let directions: [bool; POLICY_DEPTH] = policy_indices[g].to_le_bits();
        for d in 0..POLICY_DEPTH {
            let sib = policy_siblings[g][d];
            node = if directions[d] { h2(sib, node) } else { h2(node, sib) };
        }
        assert_eq(node, accepted_root, "credential status outside verifier policy");
        revoke_unset_check::<SD>(status_roots[g], status_indices[g], 0, status_siblings[g]);
    }
}

/// Checks one selected leaf against a generic pattern through typed openings.
pub fn generic_pattern_check<let K: u32>(
    active: bool,
    counts: [u32; K],
    enc: [[[Field; 3]; N]; K],
    g: u32,
    i: u32,
    types: [u32; 3],
    hashes: [Field; 3],
    vars: [u32; 3],
    constants: [Field; 3],
    values: [Field; V],
) {
    assert(g < K, "selected credential out of range");
    assert(i < N, "selected leaf out of range");
    assert(!active | (i < counts[g]), "selected padding leaf");
    for s in 0..3 {
        let term = enc[g][i][s];
        let ty = types[s];
        assert(!active | ((ty == 1) | (ty == 2)), "selected blank or unsupported term");
        if s < 2 {
            assert(!active | (ty == 1), "subject and predicate must be IRIs");
        }
        assert(!active | (term == h2(ty as Field, hashes[s])), "selected term type opening mismatch");
        let variable = vars[s];
        if variable == 0 {
            assert(!active | (term == constants[s]), "pattern constant mismatch");
        } else {
            // One shared row binding enforces every repeated occurrence,
            // including repeats within one pattern and cross-credential joins.
            assert(!active | (term == values[variable - 1]), "joined variable mismatch");
        }
    }
}

/// Checks one selected leaf against a public row triple, with canonical padding.
pub fn public_pattern_check<let K: u32>(
    row_active: bool,
    counts: [u32; K],
    enc: [[[Field; 3]; N]; K],
    g: u32,
    i: u32,
    triple: [Field; 3],
) {
    assert(g < K, "selected credential out of range");
    assert(i < N, "selected leaf out of range");
    assert(!row_active | (i < counts[g]), "selected padding leaf");
    for s in 0..3 {
        assert(!row_active | (enc[g][i][s] == triple[s]), "public triple mismatch");
        assert(row_active | (triple[s] == 0), "noncanonical public triple padding");
    }
}

/// Checks selected result support with a public first pattern.
pub fn result_public_check<let K: u32>(
    version: Field,
    accepted_root: Field,
    issuer_keys: [[Field; 2]; K],
    pattern_count: u32,
    pattern_vars: [[u32; 3]; P],
    pattern_constants: [[Field; 3]; P],
    projection: [bool; V],
    result_count: u32,
    results: [[Field; V]; R],
    filter_count: u32,
    public_triples: [[Field; 3]; R],
    counts: [u32; K],
    salts: [Field; K],
    enc: [[[Field; 3]; N]; K],
    status_lists: [Field; K],
    status_versions: [u64; K],
    status_indices: [Field; K],
    status_roots: [Field; K],
    status_siblings: [[Field; PUBLIC_PATTERN_STATUS_DEPTH]; K],
    policy_indices: [Field; K],
    policy_siblings: [[Field; POLICY_DEPTH]; K],
    signatures: [[Field; 5]; K],
    selected_graphs: [[u32; P]; R],
    selected_leaves: [[u32; P]; R],
    values: [[Field; V]; R],
    selected_types: [[[u32; 3]; P]; R],
    selected_hashes: [[[Field; 3]; P]; R],
) {
    assert_eq(version, PUBLIC_PATTERN_VERSION as Field, "unsupported successful-result contract");
    assert((pattern_count > 0) & (pattern_count <= P), "pattern capacity");
    assert((result_count > 0) & (result_count <= R), "result capacity");
    assert_eq(filter_count, 0, "hidden filters unsupported by public-pattern contract");
    for p in 0..P {
        for s in 0..3 {
            assert(pattern_vars[p][s] <= V, "variable capacity");
        }
    }
    authenticate_credentials::<K, PUBLIC_PATTERN_STATUS_DEPTH>(accepted_root, issuer_keys, counts, salts, enc, status_lists, status_versions, status_indices, status_roots, status_siblings, policy_indices, policy_siblings, signatures);
    for r in 0..R {
        let row_active = r < result_count;
        for v in 0..V {
            assert(!row_active | !projection[v] | (values[r][v] == results[r][v]), "projected binding mismatch");
            assert((row_active & projection[v]) | (results[r][v] == 0), "noncanonical public result padding");
        }
        // Pattern zero: membership in an authenticated credential of the exact public triple.
        public_pattern_check::<K>(row_active, counts, enc, selected_graphs[r][0], selected_leaves[r][0], public_triples[r]);
        for p in 1..P {
            let active = row_active & (p < pattern_count);
            generic_pattern_check::<K>(active, counts, enc, selected_graphs[r][p], selected_leaves[r][p], selected_types[r][p], selected_hashes[r][p], pattern_vars[p], pattern_constants[p], values[r]);
        }
    }
}

#[test]
fn accepts_supported_public_leaf() {
    let mut enc = [[[0; 3]; N]; 1];
    enc[0][0] = [11, 12, 13];
    enc[0][1] = [21, 22, 23];
    public_pattern_check::<1>(true, [2], enc, 0, 1, [21, 22, 23]);
}

#[test(should_fail_with = "public triple mismatch")]
fn rejects_missing_public_leaf() {
    let mut enc = [[[0; 3]; N]; 1];
    enc[0][0] = [11, 12, 13];
    public_pattern_check::<1>(true, [1], enc, 0, 0, [11, 12, 14]);
}

#[test(should_fail_with = "public triple mismatch")]
fn rejects_changed_public_binding() {
    let mut enc = [[[0; 3]; N]; 2];
    enc[0][0] = [11, 12, 13];
    enc[1][0] = [31, 12, 13];
    // A leaf from the other credential cannot satisfy a substituted subject.
    public_pattern_check::<2>(true, [1, 1], enc, 1, 0, [11, 12, 13]);
}

#[test(should_fail_with = "selected padding leaf")]
fn rejects_padding_leaf_for_zero_triple() {
    let enc = [[[0; 3]; N]; 1];
    public_pattern_check::<1>(true, [1], enc, 0, 3, [0, 0, 0]);
}

#[test(should_fail_with = "selected blank or unsupported term")]
fn remaining_generic_pattern_still_rejects_blank_openings() {
    let enc = [[[0; 3]; N]; 1];
    generic_pattern_check::<1>(true, [1], enc, 0, 0, [3, 1, 1], [0, 0, 0], [1, 0, 0], [0, 0, 0], [0; V]);
}

#[test(should_fail_with = "noncanonical public triple padding")]
fn rejects_nonzero_inactive_table_row() {
    let enc = [[[0; 3]; N]; 1];
    public_pattern_check::<1>(false, [1], enc, 0, 0, [1, 0, 0]);
}
