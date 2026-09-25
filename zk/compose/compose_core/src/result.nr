// [GPT-6] Additive successful-result relation; research-stage, not externally audited.
//! Authenticated support for released positive BGP solutions, with private witnesses.
//!
//! This relation asserts existence of successful witnesses, never completeness.
//! Credential roots, salts, sizes, selected source slots, status references, and
//! hidden term encodings stay private. Issuer key slots and the fixed capacities
//! are public. A slot may repeat a credential; no credential-count claim follows.
//! Selected blank nodes are rejected in-circuit. Canonical string encodings retain
//! lexical identity; numeric predicates open that same encoding in-circuit.
//!
//! Work includes K Schnorr checks and, for the F=2 member, integer token
//! hashing. The F=0 member eliminates hidden predicate circuitry; relative
//! component costs depend on the admitted integer capacity.

use crate::hashes::{commit_fold, h2, h3, h4, TYPE_CODE_LITERAL};
use crate::issuer::{schnorr_verify, Point};
use crate::revoke::revoke_unset_check;
use crate::filter_int::filter_int_check;
use crate::result_integer::full_u64_check;

pub global N: u32 = 16;
pub global P: u32 = 3;
pub global R: u32 = 4;
pub global V: u32 = 6;
pub global STATUS_DEPTH: u32 = 10;
pub global POLICY_DEPTH: u32 = 4;
// Exact existing sig.rs domain tags: clear-index status signature, kept private.
global STATUS_REF: Field = 0x5a4b5349475f5352;
global COMMITMENT_STATUS: Field = 0x5a4b5349475f4333;
global ACCEPTED_STATUS: Field = 0x5a4b5349475f414c;

/// Opens one bounded integer without disclosing its digit count.
pub fn small_integer_check(operand: Field, op: u32, bound: u64, value: u8) {
    assert(value <= 99, "integer exceeds successful-result lane");
    if value < 10 {
        filter_int_check::<1>(operand, op, bound, true, [48 + value]);
    } else {
        filter_int_check::<2>(operand, op, bound, true, [48 + value / 10, 48 + value % 10]);
    }
}

// [GPT-6] Static numeric dispatch keeps v1 values u8 throughout the shared
// relation; expanded members retain their u64 witness ABI and pre-cast guard.
pub trait ResultInteger {
    fn check(self, operand: Field, op: u32, bound: u64, bits: u32);
}

impl ResultInteger for u8 {
    fn check(self, operand: Field, op: u32, bound: u64, bits: u32) {
        assert(bits == 8, "u8 requires tiny integer capacity");
        small_integer_check(operand, op, bound, self);
    }
}

impl ResultInteger for u64 {
    fn check(self, operand: Field, op: u32, bound: u64, bits: u32) {
        if bits == 8 {
            assert(self <= 99, "integer exceeds successful-result lane");
            small_integer_check(operand, op, bound, self as u8);
        } else {
            assert(bits == 64, "u64 requires an admitted integer capacity");
            full_u64_check(operand, op, bound, self);
        }
    }
}

/// Checks the selected result witnesses under the fixed-capacity public statement.
pub fn result_check<T, let K: u32, let F: u32, let SD: u32, let INTEGER_BITS: u32, let CONTRACT_VERSION: u32>(
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
    filter_vars: [u32; F],
    filter_ops: [u32; F],
    filter_bounds: [u64; F],
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
    selected_graphs: [[u32; P]; R],
    selected_leaves: [[u32; P]; R],
    values: [[Field; V]; R],
    selected_types: [[[u32; 3]; P]; R],
    selected_hashes: [[[Field; 3]; P]; R],
    filter_values: [[T; F]; R],
) where T: ResultInteger {
    // [GPT-6] Every published member fixes these capacities at compile time.
    std::static_assert((SD == 10) | (SD == 17) | (SD == 20), "unsupported status depth");
    std::static_assert((INTEGER_BITS == 0) | (INTEGER_BITS == 8) | (INTEGER_BITS == 64), "unsupported integer capacity");
    std::static_assert((F == 0) == (INTEGER_BITS == 0), "numeric capacity must match filter member");
    assert_eq(version, CONTRACT_VERSION as Field, "unsupported successful-result contract");
    assert((pattern_count > 0) & (pattern_count <= P), "pattern capacity");
    assert((result_count > 0) & (result_count <= R), "result capacity");
    assert(filter_count <= F, "filter capacity");
    for p in 0..P {
        for s in 0..3 {
            assert(pattern_vars[p][s] <= V, "variable capacity");
        }
    }
    for f in 0..F {
        assert((filter_vars[f] > 0) & (filter_vars[f] <= V), "filter variable capacity");
    }
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
    for r in 0..R {
        let row_active = r < result_count;
        for v in 0..V {
            assert(!row_active | !projection[v] | (values[r][v] == results[r][v]), "projected binding mismatch");
            assert((row_active & projection[v]) | (results[r][v] == 0), "noncanonical public result padding");
        }
        for p in 0..P {
            let active = row_active & (p < pattern_count);
            let g = selected_graphs[r][p];
            let i = selected_leaves[r][p];
            assert(g < K, "selected credential out of range");
            assert(i < N, "selected leaf out of range");
            assert(!active | (i < counts[g]), "selected padding leaf");
            for s in 0..3 {
                let term = enc[g][i][s];
                let ty = selected_types[r][p][s];
                assert(!active | ((ty == 1) | (ty == 2)), "selected blank or unsupported term");
                if s < 2 {
                    assert(!active | (ty == 1), "subject and predicate must be IRIs");
                }
                assert(!active | (term == h2(ty as Field, selected_hashes[r][p][s])), "selected term type opening mismatch");
                let variable = pattern_vars[p][s];
                if variable == 0 {
                    assert(!active | (term == pattern_constants[p][s]), "pattern constant mismatch");
                } else {
                    // One shared row binding enforces every repeated occurrence,
                    // including repeats within one pattern and cross-credential joins.
                    assert(!active | (term == values[r][variable - 1]), "joined variable mismatch");
                }
            }
        }
        for f in 0..F {
            if row_active & (f < filter_count) {
                filter_values[r][f].check(values[r][filter_vars[f] - 1], filter_ops[f], filter_bounds[f], INTEGER_BITS);
            }
        }
    }
}

#[test]
fn accepts_both_private_digit_lengths() {
    let a = std::hash::blake3("\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes());
    let b = std::hash::blake3("\"42\"^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes());
    let mut x: Field = 0;
    let mut y: Field = 0;
    for i in 1..32 { x = x * 256 + a[i] as Field; y = y * 256 + b[i] as Field; }
    small_integer_check(h2(TYPE_CODE_LITERAL, x), 2, 7, 8);
    small_integer_check(h2(TYPE_CODE_LITERAL, y), 2, 7, 42);
}

#[test(should_fail_with = "integer exceeds successful-result lane")]
fn rejects_out_of_range_integer() { small_integer_check(0, 2, 7, 100); }

#[test(should_fail_with = "operand encoding mismatch")]
fn rejects_unbound_integer() { small_integer_check(0, 2, 7, 42); }
