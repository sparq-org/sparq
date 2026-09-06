//! Unit + adversarial tests (noir-testing conventions: accept path plus one
//! `should_fail_with` per distinct rejection clause, message-matched so each
//! test pins WHICH clause caught the lie -- noir-circuit-patterns
//! "Adversarial-prover testing").
//!
//! Cross-vector chain: the constants below are the `nargo execute` vectors
//! from `crates/sparq-zk/tests/poseidon2_noir_cross.rs` (fixture
//! `tests/fixtures/noir_poseidon2/`). Asserting them here proves
//! `compose_core` hashes through the exact function the Rust commitment
//! pipeline ports -- the bit-compatibility requirement for commitment
//! recomputation in-circuit.

use crate::derivation::{derivation_check, RULE_LEAF, RULE_RDFS9};
use crate::filter_float::filter_f64_check;
use crate::filter_int::{filter_int_check, OP_EQ, OP_GE, OP_GT, OP_LE, OP_LT, OP_NE};
use crate::filter_signed::{filter_decimal_check, filter_signed_int_check, signed_int_encoding};
use crate::n3::{
    BUILTIN_GREATER_THAN, BUILTIN_NONE, n3_derivation_check, rule_leaf, SLOT_CONST, SLOT_VAR,
};
use crate::hashes::{commit_fold, h2, h3, TYPE_CODE_LITERAL};
use crate::join::{JOIN_DOMAIN, join_eq_check, join_value_commitment};
use crate::path::path_reach_check;
use crate::revoke::{revoke_unset_check, revoke_unset_check_committed, SIG_DOMAIN_STATUS_IDX_COMMIT};
use crate::sameas::{canon_of, sameas_entails_row};
use crate::scan::{PatternSpec, scan_check};

// ---------------------------------------------------------------------------
// Cross-vectors (sparq-zk fixture outputs, nargo 1.0.0-beta.21).
// ---------------------------------------------------------------------------

#[test]
fn commit_fold_matches_sparq_zk_cross_vectors() {
    // Poseidon2::hash([1], 1) ... ([1,2,3,4], 4) -- the h2/h3 layers and small
    // folds.
    assert_eq(
        commit_fold([1], 1),
        0x168758332d5b3e2d13be8048c8011b454590e06c44bce7f702f09103eef5a373,
    );
    assert_eq(h2(1, 2), 0x038682aa1cb5ae4e0a3f13da432a95c77c5c111f6f030faf9cad641ce1ed7383);
    assert_eq(h3(1, 2, 3), 0x23864adb160dddf590f1d3303683ebcb914f828e2635f6e85a32f0a1aecd3dd8);
    assert_eq(
        commit_fold([1, 2, 3, 4], 4),
        0x130bf204a32cac1f0ace56c78b731aa3809f06df2731ebcf6b3464a15788b1b9,
    );
    assert_eq(
        commit_fold([5, 6, 7, 8, 9, 10, 11], 7),
        0x0ceb49bb97a406f119fcceb666e68b0acb48c286d9e011a4fa9427cd49989db5,
    );
    // The 40-leaf commitment-scale fold.
    let mut leaves: [Field; 40] = [0; 40];
    for i in 0..40 {
        leaves[i] = (i as Field) + 1;
    }
    assert_eq(
        commit_fold(leaves, 40),
        0x046964eee810ddedae9085f126317d43ad22b719fedfba0a78a1de86795dffee,
    );
}

#[test]
fn commit_fold_padding_slots_do_not_enter_the_commitment() {
    // The (k, n) bucket property: a graph of 4 triples committed inside an
    // N = 8 circuit bucket must produce the SAME commitment the Rust
    // pipeline computed over exactly 4 leaves -- and garbage in padding slots
    // must not matter.
    let exact = commit_fold([1, 2, 3, 4], 4);
    assert_eq(commit_fold([1, 2, 3, 4, 0, 0, 0, 0], 4), exact);
    assert_eq(commit_fold([1, 2, 3, 4, 99, 77, 55, 33], 4), exact);
    // ...and the count is domain-separating (length-bearing IV).
    assert(commit_fold([1, 2, 3, 4, 0, 0, 0, 0], 5) != exact);
}

// ---------------------------------------------------------------------------
// scan_check -- accept + adversarial. Tiny (K=2, N=4, R=2) monomorphisation.
// ---------------------------------------------------------------------------

// Test universe: term encodings are arbitrary nonzero field elements; what
// matters to the relation is equality structure, not provenance.
global S1: Field = 101;
global P: Field = 7;
global O1: Field = 201;
global S2: Field = 102;
global O2: Field = 202;
global P_OTHER: Field = 8;

fn test_graphs() -> ([[[Field; 3]; 4]; 2], [u32; 2], [Field; 2]) {
    // [OPUS-4.8] sq-7p9c: the fixture's per-graph witnesses are laid out in
    // CANONICAL ascending-commitment order, the same canonicalisation a real host
    // builder performs (scan.nr step 1b / plan S2.5): `scan_check` now requires the
    // PUBLIC `commitments` array STRICTLY INCREASING (`commitments[0] < ...`, the
    // distinct-graph guard added by #95 that closes the duplicate-inclusion / COUNT
    // forgery class). `commit_fold` is hash-like, so a graph's commitment value is
    // unrelated to its content "size" -- the single-triple graph { (S2,P,O2) }
    // happens to commit to the SMALLER field element than the two-triple graph
    // { (S1,P,O1), (S1,P_OTHER,O2) }, so the single-triple graph is graph 0 and the
    // two-triple graph is graph 1. Graphs are an unordered set keyed by commitment,
    // so reordering them (with the matching `counts` and disclosed `rows`) does NOT
    // change the proved statement -- only the now-canonical index assignment, which
    // every per-test attribution array below tracks. The trailing assert pins the
    // ordering invariant so this fixture fails LOUDLY (not the circuit's strict-order
    // assert silently) if an encoding edit ever flips the natural order again.
    //
    // Graph 0: { (S2, P, O2) }                    -- 1 active slot.
    // Graph 1: { (S1, P, O1), (S1, P_OTHER, O2) } -- 2 active slots.
    let enc: [[[Field; 3]; 4]; 2] = [
        [[S2, P, O2], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
        [[S1, P, O1], [S1, P_OTHER, O2], [0, 0, 0], [0, 0, 0]],
    ];
    let counts: [u32; 2] = [1, 2];
    let mut commitments: [Field; 2] = [0, 0];
    for g in 0..2 {
        let mut leaves: [Field; 4] = [0; 4];
        for i in 0..4 {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
        }
        commitments[g] = commit_fold(leaves, counts[g]);
    }
    // [OPUS-4.8] Canonical-order invariant: the public commitments this fixture
    // hands to `scan_check` MUST already be strictly increasing (the circuit's
    // step-1b contract). If a future encoding change flips the natural order, this
    // asserts HERE -- a clear fixture bug -- rather than tripping the circuit's
    // "per-graph commitments not strictly increasing" guard inside an unrelated test.
    assert(
        commitments[0].lt(commitments[1]),
        "test_graphs fixture not in canonical ascending-commitment order",
    );
    (enc, counts, commitments)
}

fn p_pattern() -> PatternSpec {
    // ?s <P> ?o -- predicate constant, subject/object variables.
    PatternSpec { is_const: [false, true, false], const_enc: [0, P, 0] }
}

// Pattern ?s P ?o matches in BOTH graph 0 and graph 1 (test_graphs): the true
// attribution for the accept paths is therefore [true, true].
global ATTR_BOTH: [bool; 2] = [true, true];

#[test]
fn scan_accepts_complete_match_set() {
    let (enc, counts, commitments) = test_graphs();
    // Pattern ?s P ?o matches (S2,P,O2) in graph 0 and (S1,P,O1) in graph 1
    // (canonical ascending-commitment order, see test_graphs). Disclosed rows are
    // listed in that same graph order: graph-0 row first, then graph-1.
    let rows: [[Field; 3]; 2] = [[S2, P, O2], [S1, P, O1]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 2, ATTR_BOTH, counts, enc);
}

#[test]
fn scan_accepts_constant_only_pattern_with_padding_rows() {
    let (enc, counts, commitments) = test_graphs();
    // Fully-constant pattern (an ASK-shaped membership): one matching row,
    // second row slot padded. (S2,P,O2) lives only in graph 0 (canonical
    // ascending-commitment order, see test_graphs), so the bound attribution is
    // [true, false].
    let pattern = PatternSpec { is_const: [true, true, true], const_enc: [S2, P, O2] };
    let rows: [[Field; 3]; 2] = [[S2, P, O2], [0, 0, 0]];
    scan_check::<2, 4, 2>(commitments, pattern, rows, 1, [true, false], counts, enc);
}

// [OPUS-4.8] audit #8: the per-graph attribution is constrained to the true
// matched-graph set. Lying in EITHER direction is rejected.
#[test(should_fail_with = "attribution does not match the proved source graphs")]
fn scan_rejects_attribution_understated() {
    // The [[0],[0]] collapse-two-graphs lie at the circuit level: the pattern
    // matches in graph 1 too (it holds (S1,P,O1)), but the prover claims
    // attribution[1] = false to hide the cross-graph source. The true attribution
    // is [true, true]; [true, false] understates the genuinely-matching graph 1.
    let (enc, counts, commitments) = test_graphs();
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [S2, P, O2]];
    scan_check::<2, 4, 2>(
        commitments,
        p_pattern(),
        rows,
        2,
        [true, false],
        counts,
        enc,
    );
}

#[test(should_fail_with = "attribution does not match the proved source graphs")]
fn scan_rejects_attribution_overstated() {
    // Claiming a graph contributes when it does not is also rejected (exact
    // matched set, not a superset). (S2,P,O2) lives only in graph 0, so the true
    // attribution is [true, false]; [true, true] overstates the non-matching
    // graph 1.
    let (enc, counts, commitments) = test_graphs();
    let pattern = PatternSpec { is_const: [true, true, true], const_enc: [S2, P, O2] };
    let rows: [[Field; 3]; 2] = [[S2, P, O2], [0, 0, 0]];
    scan_check::<2, 4, 2>(commitments, pattern, rows, 1, [true, true], counts, enc);
}

#[test(should_fail_with = "graph commitment mismatch")]
fn scan_rejects_tampered_commitment() {
    let (enc, counts, mut commitments) = test_graphs();
    commitments[0] += 1;
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [S2, P, O2]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 2, ATTR_BOTH, counts, enc);
}

#[test(should_fail_with = "graph commitment mismatch")]
fn scan_rejects_tampered_graph_content() {
    let (mut enc, counts, commitments) = test_graphs();
    // Suppress a committed triple by overwriting an active slot ("license:
    // suspended" hiding, plan S1.1) -- the fold no longer reproduces C. Here we
    // overwrite graph 1's first active slot (S1,P,O1); the step-1 recompute of
    // graph 1's commitment then mismatches BEFORE any row/attribution check.
    enc[1][0] = [S1, P_OTHER, O1];
    let rows: [[Field; 3]; 2] = [[S2, P, O2], [0, 0, 0]];
    scan_check::<2, 4, 2>(
        commitments,
        p_pattern(),
        rows,
        1,
        [true, false],
        counts,
        enc,
    );
}

#[test(should_fail_with = "matching triple missing from disclosed rows")]
fn scan_rejects_suppressed_row() {
    let (enc, counts, commitments) = test_graphs();
    // Graph 0's match (S2,P,O2) is suppressed (only (S1,P,O1) disclosed):
    // completeness sweep must catch the undisclosed matching slot.
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [0, 0, 0]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 1, ATTR_BOTH, counts, enc);
}

#[test(should_fail_with = "disclosed row not in any committed graph")]
fn scan_rejects_fabricated_row() {
    let (enc, counts, commitments) = test_graphs();
    // A row that matches the pattern but is in NO committed graph.
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [999, P, 998]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 2, ATTR_BOTH, counts, enc);
}

#[test(should_fail_with = "disclosed row does not match the pattern")]
fn scan_rejects_off_pattern_row() {
    let (enc, counts, commitments) = test_graphs();
    // A committed triple that does NOT match the pattern's constant slot.
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [S1, P_OTHER, O2]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 2, ATTR_BOTH, counts, enc);
}

#[test(should_fail_with = "matching triple missing from disclosed rows")]
fn scan_rejects_inactive_row_padding_as_disclosure() {
    let (enc, counts, commitments) = test_graphs();
    // The matching triples are in the row array but row_count hides one:
    // padding rows must NOT satisfy completeness.
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [S2, P, O2]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 1, ATTR_BOTH, counts, enc);
}

#[test(should_fail_with = "graph size exceeds circuit bucket")]
fn scan_rejects_oversized_count() {
    let (enc, _, commitments) = test_graphs();
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [S2, P, O2]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 2, ATTR_BOTH, [5, 1], enc);
}

#[test(should_fail_with = "row_count exceeds circuit bucket")]
fn scan_rejects_oversized_row_count() {
    let (enc, counts, commitments) = test_graphs();
    let rows: [[Field; 3]; 2] = [[S1, P, O1], [S2, P, O2]];
    scan_check::<2, 4, 2>(commitments, p_pattern(), rows, 3, ATTR_BOTH, counts, enc);
}

// ---------------------------------------------------------------------------
// filter_int -- accept + adversarial.
// ---------------------------------------------------------------------------

/// Test-side replica of the operand binding (digits -> Enc). The
/// NON-circular anchor for this construction is the Rust e2e
/// (crates/sparq-zk-compose), which computes `operand_enc` via
/// `sparq_zk::encode` and proves against these circuits.
fn enc_int_literal<let D: u32>(digits: [u8; D]) -> Field {
    let suffix: [u8; 44] = "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut token: [u8; D + 46] = [0; D + 46];
    token[0] = 34;
    for i in 0..D {
        token[i + 1] = digits[i];
    }
    token[D + 1] = 34;
    for i in 0..44 {
        token[D + 2 + i] = suffix[i];
    }
    let digest: [u8; 32] = std::hash::blake3(token);
    let mut hs: Field = 0;
    for i in 1..32 {
        hs = hs * 256 + (digest[i] as Field);
    }
    h2(TYPE_CODE_LITERAL, hs)
}

#[test]
fn filter_int_all_ops_verdicts() {
    let digits: [u8; 2] = [50, 53]; // "25"
    let enc = enc_int_literal(digits);
    // 25 vs 18: lt false, le false, gt true, ge true, eq false, ne true.
    filter_int_check::<2>(enc, OP_LT, 18, false, digits);
    filter_int_check::<2>(enc, OP_LE, 18, false, digits);
    filter_int_check::<2>(enc, OP_GT, 18, true, digits);
    filter_int_check::<2>(enc, OP_GE, 18, true, digits);
    filter_int_check::<2>(enc, OP_EQ, 18, false, digits);
    filter_int_check::<2>(enc, OP_NE, 18, true, digits);
    // Boundary: 25 vs 25.
    filter_int_check::<2>(enc, OP_GE, 25, true, digits);
    filter_int_check::<2>(enc, OP_GT, 25, false, digits);
    filter_int_check::<2>(enc, OP_EQ, 25, true, digits);
}

#[test]
fn filter_int_single_digit_zero_is_canonical() {
    let digits: [u8; 1] = [48]; // "0"
    let enc = enc_int_literal(digits);
    filter_int_check::<1>(enc, OP_LT, 1, true, digits);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn filter_int_rejects_lying_verdict() {
    let digits: [u8; 2] = [49, 55]; // "17"
    let enc = enc_int_literal(digits);
    // 17 > 18 claimed true -- the per-row pruning lie a prover would need to
    // drop a result row (or keep a failing one).
    filter_int_check::<2>(enc, OP_GT, 18, true, digits);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn filter_int_rejects_swapped_operand() {
    // Digits for 99, anchor encoding for 17: the prover cannot satisfy the
    // predicate with a different committed value.
    let enc17 = enc_int_literal([49, 55]);
    filter_int_check::<2>(enc17, OP_GT, 18, true, [57, 57]);
}

#[test(should_fail_with = "not a decimal digit")]
fn filter_int_rejects_non_digit_bytes() {
    let digits: [u8; 2] = [49, 65]; // "1A"
    let enc = enc_int_literal(digits);
    filter_int_check::<2>(enc, OP_GT, 0, true, digits);
}

#[test(should_fail_with = "non-canonical leading zero")]
fn filter_int_rejects_leading_zero() {
    let digits: [u8; 2] = [48, 53]; // "05"
    let enc = enc_int_literal(digits);
    filter_int_check::<2>(enc, OP_LT, 18, true, digits);
}

#[test(should_fail_with = "unknown comparison operator")]
fn filter_int_rejects_unknown_op() {
    let digits: [u8; 1] = [53];
    let enc = enc_int_literal(digits);
    filter_int_check::<1>(enc, 6, 1, true, digits);
}

// ---------------------------------------------------------------------------
// filter_f64 -- IEEE semantics via sparq_ieee754.
// ---------------------------------------------------------------------------

global F64_ONE_POINT_FIVE: u64 = 0x3FF8000000000000;
global F64_TWO_POINT_FIVE: u64 = 0x4004000000000000;
global F64_NAN: u64 = 0x7FF8000000000000;
global F64_POS_ZERO: u64 = 0x0000000000000000;
global F64_NEG_ZERO: u64 = 0x8000000000000000;

#[test]
fn filter_f64_ordered_comparisons() {
    filter_f64_check(F64_ONE_POINT_FIVE, F64_TWO_POINT_FIVE, OP_LT, true);
    filter_f64_check(F64_ONE_POINT_FIVE, F64_TWO_POINT_FIVE, OP_GE, false);
    filter_f64_check(F64_TWO_POINT_FIVE, F64_ONE_POINT_FIVE, OP_GT, true);
    filter_f64_check(F64_ONE_POINT_FIVE, F64_ONE_POINT_FIVE, OP_LE, true);
    filter_f64_check(F64_ONE_POINT_FIVE, F64_ONE_POINT_FIVE, OP_EQ, true);
}

#[test]
fn filter_f64_ieee_edge_cases() {
    // NaN is unordered against everything, including itself; ne(NaN) = true.
    filter_f64_check(F64_NAN, F64_ONE_POINT_FIVE, OP_LT, false);
    filter_f64_check(F64_NAN, F64_ONE_POINT_FIVE, OP_GE, false);
    filter_f64_check(F64_NAN, F64_NAN, OP_EQ, false);
    filter_f64_check(F64_NAN, F64_NAN, OP_NE, true);
    // -0 == +0 despite distinct bit patterns.
    filter_f64_check(F64_NEG_ZERO, F64_POS_ZERO, OP_EQ, true);
    filter_f64_check(F64_NEG_ZERO, F64_POS_ZERO, OP_LT, false);
}

#[test(should_fail_with = "float filter verdict mismatch")]
fn filter_f64_rejects_lying_verdict() {
    filter_f64_check(F64_TWO_POINT_FIVE, F64_ONE_POINT_FIVE, OP_LT, true);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-1q9h: filter_signed_int + filter_decimal -- accept + adversarial.
// ---------------------------------------------------------------------------

/// Test-side replica of the SIGNED-integer operand binding (sign + digits ->
/// Enc). The lexical form is '[-]?<digits>'; the token folds in the xsd:integer
/// datatype. The NON-circular anchor is the Rust e2e (the crate computes
/// `operand_enc` via `sparq_zk::encode` and proves against this circuit).
fn enc_signed_int_literal<let MD: u32>(neg: bool, mag_digits: [u8; MD]) -> Field {
    let suffix: [u8; 44] = "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    // Two-branch token layout, mirroring the circuit's `assert_literal_binding`
    // (the negative token carries a leading '-', so its length differs by one).
    if neg {
        let mut t: [u8; MD + 47] = [0; MD + 47]; // 1('"') + 1('-') + MD + 1('"') + 44
        t[0] = 34;
        t[1] = 45;
        for i in 0..MD {
            t[i + 2] = mag_digits[i];
        }
        t[MD + 2] = 34;
        for i in 0..44 {
            t[MD + 3 + i] = suffix[i];
        }
        let digest: [u8; 32] = std::hash::blake3(t);
        let mut hs: Field = 0;
        for i in 1..32 {
            hs = hs * 256 + (digest[i] as Field);
        }
        h2(TYPE_CODE_LITERAL, hs)
    } else {
        let mut t: [u8; MD + 46] = [0; MD + 46]; // 1 + MD + 1 + 44
        t[0] = 34;
        for i in 0..MD {
            t[i + 1] = mag_digits[i];
        }
        t[MD + 1] = 34;
        for i in 0..44 {
            t[MD + 2 + i] = suffix[i];
        }
        let digest: [u8; 32] = std::hash::blake3(t);
        let mut hs: Field = 0;
        for i in 1..32 {
            hs = hs * 256 + (digest[i] as Field);
        }
        h2(TYPE_CODE_LITERAL, hs)
    }
}

#[test]
fn filter_signed_int_negative_vs_positive_bound() {
    // value = -42, bound = +1: -42 < 1 so lt true, gt false, eq false, ne true.
    let digits: [u8; 2] = [52, 50]; // "42"
    let enc = enc_signed_int_literal(true, digits);
    filter_signed_int_check::<2>(enc, OP_LT, false, 1, true, true, digits);
    filter_signed_int_check::<2>(enc, OP_GT, false, 1, false, true, digits);
    filter_signed_int_check::<2>(enc, OP_EQ, false, 1, false, true, digits);
    filter_signed_int_check::<2>(enc, OP_NE, false, 1, true, true, digits);
    filter_signed_int_check::<2>(enc, OP_LE, false, 1, true, true, digits);
    filter_signed_int_check::<2>(enc, OP_GE, false, 1, false, true, digits);
}

#[test]
fn filter_signed_int_both_negative_order() {
    // value = -42, bound = -100: among negatives larger magnitude is smaller,
    // so -42 > -100. gt true, lt false, ge true.
    let digits: [u8; 2] = [52, 50]; // "42"
    let enc = enc_signed_int_literal(true, digits);
    filter_signed_int_check::<2>(enc, OP_GT, true, 100, true, true, digits);
    filter_signed_int_check::<2>(enc, OP_LT, true, 100, false, true, digits);
    filter_signed_int_check::<2>(enc, OP_GE, true, 100, true, true, digits);
    // boundary: -42 vs -42 eq.
    filter_signed_int_check::<2>(enc, OP_EQ, true, 42, true, true, digits);
    filter_signed_int_check::<2>(enc, OP_GE, true, 42, true, true, digits);
    filter_signed_int_check::<2>(enc, OP_GT, true, 42, false, true, digits);
}

#[test]
fn filter_signed_int_non_negative_matches_filter_int() {
    // value = +25, bound = +18, gt true -- same semantics as filter_int's path.
    let digits: [u8; 2] = [50, 53]; // "25"
    let enc = enc_signed_int_literal(false, digits);
    filter_signed_int_check::<2>(enc, OP_GT, false, 18, true, false, digits);
    filter_signed_int_check::<2>(enc, OP_LT, false, 18, false, false, digits);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn filter_signed_int_rejects_lying_verdict() {
    // -17 < -18 is FALSE (among negatives -17 > -18); claiming true is the lie.
    let digits: [u8; 2] = [49, 55]; // "17"
    let enc = enc_signed_int_literal(true, digits);
    filter_signed_int_check::<2>(enc, OP_LT, true, 18, true, true, digits);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn filter_signed_int_rejects_sign_swap() {
    // Anchor encoding is for -42; prover tries to pass it off as +42.
    let digits: [u8; 2] = [52, 50]; // "42"
    let enc_neg = enc_signed_int_literal(true, digits);
    filter_signed_int_check::<2>(enc_neg, OP_GT, false, 0, true, false, digits);
}

#[test(should_fail_with = "negative zero is not canonical")]
fn filter_signed_int_rejects_negative_zero() {
    let digits: [u8; 1] = [48]; // "0"
    let enc = enc_signed_int_literal(true, digits);
    filter_signed_int_check::<1>(enc, OP_EQ, false, 0, true, true, digits);
}

#[test(should_fail_with = "non-canonical leading zero")]
fn filter_signed_int_rejects_leading_zero() {
    let digits: [u8; 2] = [48, 53]; // "05"
    let enc = enc_signed_int_literal(true, digits);
    filter_signed_int_check::<2>(enc, OP_LT, false, 0, true, true, digits);
}

/// Test-side replica of the DECIMAL operand binding (sign + int + '.' + frac).
fn enc_decimal_literal<let ID: u32, let FD: u32>(
    neg: bool,
    int_digits: [u8; ID],
    frac_digits: [u8; FD],
) -> Field {
    let suffix: [u8; 44] = "^^<http://www.w3.org/2001/XMLSchema#decimal>".as_bytes();
    if neg {
        // token = '"' + '-' + int + '.' + frac + '"' + suffix
        let mut t: [u8; ID + FD + 48] = [0; ID + FD + 48]; // 1+1+ID+1+FD+1+44
        t[0] = 34;
        t[1] = 45;
        for i in 0..ID {
            t[i + 2] = int_digits[i];
        }
        t[ID + 2] = 46; // '.'
        for i in 0..FD {
            t[ID + 3 + i] = frac_digits[i];
        }
        t[ID + FD + 3] = 34;
        for i in 0..44 {
            t[ID + FD + 4 + i] = suffix[i];
        }
        let digest: [u8; 32] = std::hash::blake3(t);
        let mut hs: Field = 0;
        for i in 1..32 {
            hs = hs * 256 + (digest[i] as Field);
        }
        h2(TYPE_CODE_LITERAL, hs)
    } else {
        // token = '"' + int + '.' + frac + '"' + suffix
        let mut t: [u8; ID + FD + 47] = [0; ID + FD + 47]; // 1+ID+1+FD+1+44
        t[0] = 34;
        for i in 0..ID {
            t[i + 1] = int_digits[i];
        }
        t[ID + 1] = 46; // '.'
        for i in 0..FD {
            t[ID + 2 + i] = frac_digits[i];
        }
        t[ID + FD + 2] = 34;
        for i in 0..44 {
            t[ID + FD + 3 + i] = suffix[i];
        }
        let digest: [u8; 32] = std::hash::blake3(t);
        let mut hs: Field = 0;
        for i in 1..32 {
            hs = hs * 256 + (digest[i] as Field);
        }
        h2(TYPE_CODE_LITERAL, hs)
    }
}

#[test]
fn filter_decimal_positive_fraction_compare() {
    // value = 123.45 (scaled 12345 at FD=2). bound = 123.40 (scaled 12340).
    // 123.45 > 123.40: gt true, lt false, eq false.
    let int_d: [u8; 3] = [49, 50, 51]; // "123"
    let frac_d: [u8; 2] = [52, 53]; // "45"
    let enc = enc_decimal_literal(false, int_d, frac_d);
    filter_decimal_check::<3, 2>(enc, OP_GT, false, 12340, true, false, int_d, frac_d);
    filter_decimal_check::<3, 2>(enc, OP_LT, false, 12340, false, false, int_d, frac_d);
    filter_decimal_check::<3, 2>(enc, OP_EQ, false, 12345, true, false, int_d, frac_d);
    filter_decimal_check::<3, 2>(enc, OP_GE, false, 12345, true, false, int_d, frac_d);
}

#[test]
fn filter_decimal_negative_fraction_order() {
    // value = -0.50 (scaled 50, neg). bound = -0.10 (scaled 10, neg).
    // among negatives larger magnitude is smaller, so -0.50 < -0.10: lt true.
    let int_d: [u8; 1] = [48]; // "0"
    let frac_d: [u8; 2] = [53, 48]; // "50"
    let enc = enc_decimal_literal(true, int_d, frac_d);
    filter_decimal_check::<1, 2>(enc, OP_LT, true, 10, true, true, int_d, frac_d);
    filter_decimal_check::<1, 2>(enc, OP_GT, true, 10, false, true, int_d, frac_d);
    // -0.50 vs +0.10: negative < non-negative.
    filter_decimal_check::<1, 2>(enc, OP_LT, false, 10, true, true, int_d, frac_d);
}

#[test]
fn filter_decimal_trailing_zero_is_canonical() {
    // "3.0" is a canonical xsd:decimal (trailing fraction zero allowed).
    let int_d: [u8; 1] = [51]; // "3"
    let frac_d: [u8; 1] = [48]; // "0"
    let enc = enc_decimal_literal(false, int_d, frac_d);
    // 3.0 == 3.0 (scaled 30 at FD=1).
    filter_decimal_check::<1, 1>(enc, OP_EQ, false, 30, true, false, int_d, frac_d);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn filter_decimal_rejects_lying_verdict() {
    // 1.50 < 1.40 is FALSE; claiming true is the per-row pruning lie.
    let int_d: [u8; 1] = [49]; // "1"
    let frac_d: [u8; 2] = [53, 48]; // "50"
    let enc = enc_decimal_literal(false, int_d, frac_d);
    filter_decimal_check::<1, 2>(enc, OP_LT, false, 140, true, false, int_d, frac_d);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn filter_decimal_rejects_swapped_fraction() {
    // Anchor encoding is for 1.50; prover witnesses 1.59 digits.
    let enc_150 = enc_decimal_literal(false, [49], [53, 48]); // "1.50"
    filter_decimal_check::<1, 2>(enc_150, OP_GT, false, 0, true, false, [49], [53, 57]);
}

#[test(should_fail_with = "negative zero is not canonical")]
fn filter_decimal_rejects_negative_zero() {
    // "-0.00" has zero magnitude -- not canonical.
    let int_d: [u8; 1] = [48]; // "0"
    let frac_d: [u8; 2] = [48, 48]; // "00"
    let enc = enc_decimal_literal(true, int_d, frac_d);
    filter_decimal_check::<1, 2>(enc, OP_EQ, false, 0, true, true, int_d, frac_d);
}

// ---------------------------------------------------------------------------
// revoke -- hidden-index status-list inclusion + bit-unset (sq-3e5 / sq-h2v).
// ---------------------------------------------------------------------------
//
// A depth-2 tree over 4 leaves built BY HAND with the same `h2` the Rust root
// builder uses. Leaves are the status bits (0 = active, 1 = revoked):
//   leaf0=0  leaf1=1  leaf2=0  leaf3=0
//   n01 = h2(leaf0, leaf1)   n23 = h2(leaf2, leaf3)
//   root = h2(n01, n23)
// The proof for an active index reveals neither the index nor the bits; the
// witness here just exercises the relation in-circuit.

// Authentication path for index 0 (an UNSET leaf): bottom-up siblings are
// [leaf1, n23]; directions are LSB-first bits of 0 = [0,0] (left, left).
#[test]
fn revoke_unset_index0_active_accepts() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0); // h2(leaf2=0, leaf3=0)
    let root = h2(h2(leaf0, leaf1), n23);
    revoke_unset_check::<2>(root, 0, leaf0, [leaf1, n23]);
}

// Index 2 (also UNSET): directions = bits of 2 = [0,1]; siblings = [leaf3, n01].
#[test]
fn revoke_unset_index2_active_accepts() {
    let n01 = h2(0, 1); // h2(leaf0=0, leaf1=1)
    let leaf2: Field = 0;
    let leaf3: Field = 0;
    let root = h2(n01, h2(leaf2, leaf3));
    revoke_unset_check::<2>(root, 2, leaf2, [leaf3, n01]);
}

// A REVOKED leaf (bit==1, index 1) must be REJECTED: the bit-unset assertion
// fails. The holder of a revoked credential cannot produce a proof.
#[test(should_fail_with = "credential is revoked: status bit is set")]
fn revoke_set_bit_rejected() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0);
    let root = h2(h2(leaf0, leaf1), n23);
    // index 1, bit 1 (revoked): path siblings bottom-up = [leaf0, n23],
    // directions = bits of 1 = [1,0].
    revoke_unset_check::<2>(root, 1, leaf1, [leaf0, n23]);
}

// A leaf whose authentication path does NOT recompute the committed root is
// rejected (a forged inclusion / wrong sibling).
#[test(should_fail_with = "merkle inclusion does not recompute the committed root")]
fn revoke_wrong_path_rejected() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0);
    let root = h2(h2(leaf0, leaf1), n23);
    // Correct directions for index 0, but a WRONG sibling at level 0 (99 not
    // leaf1) so the fold misses the root.
    revoke_unset_check::<2>(root, 0, leaf0, [99, n23]);
}

// The index IS the path direction (directions are decomposed FROM `index`
// in-circuit, not a separate witness). So claiming index 2 (directions [0,1])
// while supplying index-0's authentication path ([leaf1, n23]) folds along the
// WRONG side at level 1 and cannot recompute the root -- rejected. A prover
// therefore cannot reuse another (unset) leaf's path under a different claimed
// index: changing the index changes the fold directions, breaking inclusion.
#[test(should_fail_with = "merkle inclusion does not recompute the committed root")]
fn revoke_index_path_mismatch_rejected() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0);
    let root = h2(h2(leaf0, leaf1), n23);
    revoke_unset_check::<2>(root, 2, leaf0, [leaf1, n23]);
}

// `index` is range-bound to `2^D` by the in-circuit bit-decomposition: an index
// >= 2^D has a nonzero bit beyond position D, which `to_le_bits::<D>` constrains
// to zero, so the witness is unsatisfiable. (D=2 here => valid indices 0..=3;
// index 4 = 0b100 overflows.) This pins the in-range guarantee the tree needs.
#[test(should_fail)]
fn revoke_out_of_range_index_rejected() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0);
    let root = h2(h2(leaf0, leaf1), n23);
    revoke_unset_check::<2>(root, 4, leaf0, [leaf1, n23]);
}

// [OPUS-4.8] sq-ayv: the committed variant binds a hiding index commitment to the
// SAME private index proven unset. ACCEPT: index 0 active, with the CORRECT
// commitment h3(DOMAIN, index, blinding) supplied as the public input.
#[test]
fn revoke_committed_active_with_correct_commitment_accepts() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0);
    let root = h2(h2(leaf0, leaf1), n23);
    let index: Field = 0;
    let blinding: Field = 0xb1; // the holder's hiding randomness
    let index_commitment = h3(SIG_DOMAIN_STATUS_IDX_COMMIT, index, blinding);
    revoke_unset_check_committed::<2>(root, index_commitment, index, leaf0, blinding, [leaf1, n23]);
}

// REJECT: the public index commitment does NOT bind the proven-unset index (it
// was computed for a DIFFERENT index 2). A holder cannot sign over a commitment
// to its revoked index and prove bit-unset for some other index -- the in-circuit
// recompute from the proof's own `index` (0) differs from the supplied commitment.
#[test(should_fail_with = "index commitment does not bind the proven-unset index")]
fn revoke_committed_wrong_index_commitment_rejected() {
    let leaf0: Field = 0;
    let leaf1: Field = 1;
    let n23 = h2(0, 0);
    let root = h2(h2(leaf0, leaf1), n23);
    let blinding: Field = 0xb1;
    // Commitment for index 2, but the proof proves index 0 active.
    let wrong_commitment = h3(SIG_DOMAIN_STATUS_IDX_COMMIT, 2, blinding);
    revoke_unset_check_committed::<2>(root, wrong_commitment, 0, leaf0, blinding, [leaf1, n23]);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-z9l: in-circuit Schnorr-over-Baby-JubJub + hidden-key set
// membership. Vectors generated by `sparq_zk_compose::issuer` from a REAL
// signature (sig.rs Schnorr-over-Baby-JubJub) over commitment_message(0xc0ffee):
// a key set K = {seed100, seed101, seed102, seed103} committed as a depth-2
// Poseidon2 Merkle tree; the signer is seed102 at index 2; the membership path
// is [leaf(seed103), h2(leaf100, leaf101)]. The out-of-set vectors are a REAL
// signature by seed200 (a key NOT in K).
// ---------------------------------------------------------------------------

use crate::issuer::{
    assert_in_prime_order_subgroup, assert_on_curve, hidden_issuer_attestation, key_set_membership,
    Point, reduction_range_bind, schnorr_verify,
};

// In-set signer (seed102, index 2 in K).
global IV_M: Field = 0x18bca25b3980838a520d97b3d360b9d88fa497367d04d73db872ad86f05b9bab;
global IV_PK_X: Field = 0x2f55331c1d80c2398a6ca962b853ac337350e2ff4c17f1842024337ed07190c2;
global IV_PK_Y: Field = 0x10427ec74fc160b890be66dbc372aa61108183ddd27765186f476ff50044ddbe;
global IV_R_X: Field = 0x240647481be38e09b1e68097f680a049cd9000fd25da4ab17ecee8e1b0901ef9;
global IV_R_Y: Field = 0x0d0dc60835d519ccb1ea434908c96f409ce46a9df1ee570775439c7be7166b1a;
global IV_S: Field = 0x0371b558bbb53d69bb2cd844e813d01aff66e444a9aa8b3bbce206213276c7d1;
global IV_E: Field = 0x028528f1d34f843c12e5a7394de73ea7dd1e15d2c841114d52ed3d23a86c9760;
global IV_E_K: Field = 0x0000000000000000000000000000000000000000000000000000000000000003;
global IV_ROOT: Field = 0x104b3a86e296bb7a523bf0b8319dc05f2714a1ad5e4e805a65d7f78b40690b87;
global IV_SIB0: Field = 0x1e04339eeca97bcd93a8a4d853e159d48e7639c64e98dea5bd85fe52ae1bc75c;
global IV_SIB1: Field = 0x0476a552751e04b2d3eaf2726e9192f83da0ec60b186b1b0a85def79400d12e9;
global IV_IDX: Field = 2;

// Out-of-set signer (seed200, NOT in K).
global OV_PK_X: Field = 0x200dfe61440979d4e8b0827802a9ea8940fe2189c385b838f8305684ce5a56e8;
global OV_PK_Y: Field = 0x032de7d8b3f3f23488bd0819b6949e13456dc9a9d315cd7bfa7b87b7666ea421;
global OV_R_X: Field = 0x07fafb2d1553cc777b04eb9ed32b51d360a2ea02c2663a05b2e95c5d2b866600;
global OV_R_Y: Field = 0x24b9c4a554267ee5b4d5631d816eb1b022a101daccb32228d307cc75dbe1d1aa;
global OV_S: Field = 0x060040d301f61065d8bfcce98a1847a28ee8a57978be00f63b3c65af39e85673;
global OV_E: Field = 0x00216946ce445d7b02ac66fcd1c5f8a0f93d3950929ba2abb2e0bd4e8cacba00;
global OV_E_K: Field = 0x0000000000000000000000000000000000000000000000000000000000000003;

fn in_set_pk() -> Point {
    Point { x: IV_PK_X, y: IV_PK_Y }
}
fn in_set_r() -> Point {
    Point { x: IV_R_X, y: IV_R_Y }
}

// (1) ACCEPT: a valid signature by an IN-SET issuer verifies end-to-end, and the
// issuer key is NOT among the public inputs (m, key_set_root only). The full
// relation: Schnorr verify + key-set membership.
#[test]
fn hidden_issuer_in_set_signature_verifies() {
    hidden_issuer_attestation::<2>(
        IV_M,
        IV_ROOT,
        in_set_pk(),
        in_set_r(),
        IV_S,
        IV_E,
        IV_E_K,
        IV_IDX,
        [IV_SIB0, IV_SIB1],
    );
}

// (1b) The Schnorr gadget alone verifies the real signature (isolates the
// curve-arithmetic + challenge-binding from the membership fold).
#[test]
fn hidden_issuer_schnorr_equation_holds() {
    schnorr_verify(in_set_pk(), in_set_r(), IV_S, IV_E, IV_E_K, IV_M);
}

// (1c) The membership gadget alone: the in-set key folds to the public root.
#[test]
fn hidden_issuer_membership_holds() {
    key_set_membership::<2>(in_set_pk(), IV_ROOT, IV_IDX, [IV_SIB0, IV_SIB1]);
}

// (2) REJECT out-of-set key: a REAL signature by an out-of-set issuer (seed200)
// is a valid signature (the schnorr gadget would accept it) but the key is NOT a
// member of K, so the membership fold cannot recompute the public root.
#[test(should_fail_with = "issuer key is not a member of the committed key set")]
fn hidden_issuer_out_of_set_key_rejected() {
    // The out-of-set key's own valid signature, but folded against K's root with
    // the in-set member's path (the only path the prover could try to present).
    hidden_issuer_attestation::<2>(
        IV_M,
        IV_ROOT,
        Point { x: OV_PK_X, y: OV_PK_Y },
        Point { x: OV_R_X, y: OV_R_Y },
        OV_S,
        OV_E,
        OV_E_K,
        IV_IDX,
        [IV_SIB0, IV_SIB1],
    );
}

// (2b) The out-of-set signature is itself VALID (Schnorr equation holds) -- so
// the rejection in (2) is purely the membership gate, not a bad signature. This
// is the analogue of sig.rs's "a real sig by an untrusted key" forge.
#[test]
fn hidden_issuer_out_of_set_signature_is_itself_valid() {
    schnorr_verify(
        Point { x: OV_PK_X, y: OV_PK_Y },
        Point { x: OV_R_X, y: OV_R_Y },
        OV_S,
        OV_E,
        OV_E_K,
        IV_M,
    );
}

// (3) REJECT forged/invalid signature: tampering the signature scalar s breaks
// the verification equation s*G == R + e*pk.
#[test(should_fail_with = "schnorr verification equation failed")]
fn hidden_issuer_tampered_s_rejected() {
    schnorr_verify(in_set_pk(), in_set_r(), IV_S + 1, IV_E, IV_E_K, IV_M);
}

// (3b) REJECT a forged challenge: substituting a different e (not e_base mod L)
// breaks the reduction binding e_base == e + e_k*L.
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn hidden_issuer_forged_challenge_rejected() {
    schnorr_verify(in_set_pk(), in_set_r(), IV_S, IV_E + 1, IV_E_K, IV_M);
}

// (3c) REJECT a wrong message: a signature over IV_M must not verify against a
// different message (the challenge recomputed over m' diverges, so the bound e
// no longer matches the equation). This is the truncated-commitment / message-
// swap shape from sig.rs::wrong_message_rejected.
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn hidden_issuer_wrong_message_rejected() {
    schnorr_verify(in_set_pk(), in_set_r(), IV_S, IV_E, IV_E_K, IV_M + 1);
}

// (4) REJECT identity issuer key (universal-forgery guard, sig.rs codex #3).
#[test(should_fail_with = "identity issuer key rejected")]
fn hidden_issuer_identity_key_rejected() {
    // The neutral point (0, 1). s*G == R is satisfiable for any (R = s*G), so the
    // gadget must reject it before the equation can be gamed.
    schnorr_verify(Point { x: 0, y: 1 }, in_set_r(), IV_S, IV_E, IV_E_K, IV_M);
}

// (5) REJECT off-curve key: an off-curve pk (a forger's twist / arbitrary point,
// NOT on the curve) is rejected by the on-curve guard before the equation. (A
// low-order/torsion point IS on the curve; that is a SEPARATE guard -- see (6).)
#[test(should_fail_with = "point is not on Baby-JubJub")]
fn hidden_issuer_off_curve_key_rejected() {
    // (1, 1) is not on Baby-JubJub (1 + 1 != 1 + d).
    schnorr_verify(Point { x: 1, y: 1 }, in_set_r(), IV_S, IV_E, IV_E_K, IV_M);
}

// [OPUS-4.8] sq-l15mi (audit H-1): the TORSION / small-subgroup key forgery guard.
// Baby-JubJub has cofactor 8, so `(0, -1)` (= (0, q_base - 1)) is an order-2 point
// that is ON the curve and is NOT the identity. Before this fix `schnorr_verify`
// accepted it (on-curve + non-identity), and a prover with NO secret key could forge
// a satisfying `(R, s)` for such a torsion key. The prime-order-subgroup guard
// `[L]*pk == O` now rejects it.

// (6a) NON-VACUITY witness: the order-2 torsion point `(0, -1)` PASSES the two
// pre-existing guards (on-curve, and not the identity `(0, 1)`), so ONLY the new
// prime-order-subgroup guard can catch it. If this ever fails, the torsion vector
// below is no longer a valid "slips past the old guards" witness.
#[test]
fn torsion_key_passes_on_curve_and_non_identity_guards() {
    let torsion = Point { x: 0, y: -1 };
    assert_on_curve(torsion); // on the curve (0 + 1 == 1 + 0)
    assert(!((torsion.x == 0) & (torsion.y == 1)), "torsion (0,-1) is not the identity");
}

// (6b) The new guard ACCEPTS a genuine prime-order (order-L) key -- non-vacuity in
// the accept direction: it does not reject legitimate issuer keys (`in_set_pk` is a
// real `sk*G`, so it lies in the order-L subgroup and `[L]*pk == O`).
#[test]
fn prime_order_guard_accepts_valid_issuer_key() {
    assert_in_prime_order_subgroup(in_set_pk());
    assert_in_prime_order_subgroup(in_set_r()); // honest R = k*G is in-subgroup too
}

// (6c) The new guard in ISOLATION REJECTS the order-2 torsion point `(0, -1)`.
#[test(should_fail_with = "not in prime-order subgroup")]
fn prime_order_guard_rejects_order2_torsion() {
    assert_in_prime_order_subgroup(Point { x: 0, y: -1 });
}

// (6d) FORGE REJECTION end-to-end: `schnorr_verify` rejects the torsion issuer key
// at the prime-order-subgroup guard. Message-matched (noir-testing convention) so
// this pins the NEW clause: before the fix this input flowed past to the equation
// and failed with "schnorr verification equation failed" (a DIFFERENT message), so
// the message match makes this test non-vacuous w.r.t. the fix.
#[test(should_fail_with = "not in prime-order subgroup")]
fn hidden_issuer_torsion_key_rejected() {
    schnorr_verify(Point { x: 0, y: -1 }, in_set_r(), IV_S, IV_E, IV_E_K, IV_M);
}

// [OPUS-4.8] sq-ru0yx (internal re-audit finding M-1): the challenge-reduction
// NO-WRAP regression suite. The bare reduction identity `e + e_k*L == e_base` plus
// `e < L` and `e_k < 8` does NOT uniquely pin `e = e_base mod L`: for `e_k == 7`
// the FIELD identity also accepts the wrap alternative `e' in [q_base-7L, L)`. The
// fix (`reduction_range_bind` step 4) forbids that window. These exercise the
// range-bind in isolation with explicit `(e, e_k)` (a genuine Poseidon2 preimage
// with tiny `e_base`, the only `e_base` for which the wrap window is reachable, is
// infeasible to grind; so the regression pins the constraint directly).

// The reduction constants this test wires (mirrors `issuer.nr`):
//   L     = BJJ_L                  (scalar-field order, ~2^251)
//   TOP   = q_base - 7*L           (REDUCTION_TOP_BOUND, the e_k==7 no-wrap bound)
global RW_L_MINUS_1: Field =
    0x060c89ce5c263405370a08b6d0302b0bab3eedb83920ee0a677297dc392126f0; // L - 1
global RW_TOP: Field =
    0x060c89ce5c263405370a08b6d0302b0b797b683ee9d2ee486fbfce8e6017ef6a; // q_base - 7*L
global RW_TOP_MINUS_1: Field =
    0x060c89ce5c263405370a08b6d0302b0b797b683ee9d2ee486fbfce8e6017ef69; // (q_base - 7*L) - 1

// (M-1a) ACCEPT: for `e_k < 7` the full `[0, L)` range is honest (bound == L), so
// the largest canonical scalar `e = L - 1` passes the range bind.
#[test]
fn reduction_no_wrap_accepts_max_e_for_low_quotient() {
    reduction_range_bind(RW_L_MINUS_1, 6);
}

// (M-1b) ACCEPT: for `e_k == 7` the honest window is `[0, q_base - 7L)`, so
// `e = (q_base - 7L) - 1` (the largest honest value) passes -- the fix does NOT
// reject any honest witness.
#[test]
fn reduction_no_wrap_accepts_top_boundary_honest_e() {
    reduction_range_bind(RW_TOP_MINUS_1, 7);
}

// (M-1c) REJECT (the forge this fix closes): for `e_k == 7`, the FIRST value of the
// previously-admitted wrap window `e = q_base - 7L` is now rejected by the no-wrap
// upper bound. Without the fix this `e` satisfied `e < L` and (for the matching
// small `e_base`) the bare identity, forging a second valid challenge scalar.
#[test(should_fail_with = "call to assert_max_bit_size")]
fn reduction_no_wrap_rejects_wrap_window_e_at_top_quotient() {
    reduction_range_bind(RW_TOP, 7);
}

// (M-1d) REJECT: the very top of the old wrap window, `e = L - 1`, with `e_k == 7`
// is also excluded by the no-wrap bound (it is `>= q_base - 7L`).
#[test(should_fail_with = "call to assert_max_bit_size")]
fn reduction_no_wrap_rejects_max_e_at_top_quotient() {
    reduction_range_bind(RW_L_MINUS_1, 7);
}

// [OPUS-4.8] sq-ayv: pin the index-commitment cross-vector so the Rust
// `status_index_commitment` and this in-circuit `h3(DOMAIN, index, blinding)`
// stay bit-identical (the cross-binding rests on them computing the same value).
#[test]
fn revoke_idx_commit_cross_vector() {
    assert_eq(
        h3(SIG_DOMAIN_STATUS_IDX_COMMIT, 0, 0xb1),
        0x204fe7eb53d68b4f2c7b981619ed4fbd5f3b99879e0f387761bf489a21456118,
    );
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-xqfg (HolderPoP T5): in-circuit holder Proof-of-Possession (B2).
// Vectors generated by `sparq_zk_compose::holder` (via
// `sparq_zk::sig::in_circuit_holder_witness`) from a REAL holder key:
//   - HSK_* / HPK_* / HDIG: the holder secret + public key + holder_key_digest for
//     the seed-102 key (the SAME key as the IV_PK_* hidden-issuer vectors above, so
//     HPK_X/HPK_Y == IV_PK_X/IV_PK_Y -- cross-confirming the host witness).
//   - HPK2_* / HDIG2: a DIFFERENT holder key (seed 200, the OV_PK_* key), used to
//     show the digest is key-specific (the binding A cannot reuse with its own key).
// The digest is `holder_key_digest(hpk) = Poseidon2([ZKSIG_HK, hpk.x, hpk.y])` --
// bit-identical to the host helper and the issuer-attested value the T6 verifier
// gate cross-checks.
// ---------------------------------------------------------------------------

use crate::holder::{holder_key_digest as holder_digest, holder_pok};

// seed-102 holder key (== IV_PK_X / IV_PK_Y).
global HSK: Field = 0x04c49ec34f100efeb528ac3d436a6e1a2cb6b0c85fab6a485462c74c12a82d15;
global HPK_X: Field = 0x2f55331c1d80c2398a6ca962b853ac337350e2ff4c17f1842024337ed07190c2;
global HPK_Y: Field = 0x10427ec74fc160b890be66dbc372aa61108183ddd27765186f476ff50044ddbe;
global HDIG: Field = 0x19e7f4238df100483b1786ae2bd0e8e1c06fc0660ee1016430e95bf824d6e12b;

// seed-200 holder key (== OV_PK_X / OV_PK_Y), a DIFFERENT key.
global HSK2: Field = 0x0222b007e0222f517ed0c2039ede22d3b0566e6e513acb300be1e091d236dc0a;
global HPK2_X: Field = 0x200dfe61440979d4e8b0827802a9ea8940fe2189c385b838f8305684ce5a56e8;
global HPK2_Y: Field = 0x032de7d8b3f3f23488bd0819b6949e13456dc9a9d315cd7bfa7b87b7666ea421;
global HDIG2: Field = 0x1fcb2bb6901f29951dcdc1d75f7d3bee6eca922a34fcd58c88bdc194dba5a559;

fn holder_hpk() -> Point {
    Point { x: HPK_X, y: HPK_Y }
}

// (H0) CROSS-VECTOR: the in-circuit holder_key_digest is bit-identical to the host
// `sparq_zk::sig::holder_key_digest` (the SECURITY-critical T6 cross-check anchor).
#[test]
fn holder_key_digest_cross_vector() {
    assert_eq(holder_digest(holder_hpk()), HDIG);
    assert_eq(holder_digest(Point { x: HPK2_X, y: HPK2_Y }), HDIG2);
}

// (H1) ACCEPT: knowledge of hsk with hpk = hsk*G AND digest match verifies. The
// holder secret and key are PRIVATE; only `holder_pk_digest` is public.
#[test]
fn holder_pok_accepts_valid_possession() {
    holder_pok(HDIG, HSK, holder_hpk());
}

// (H2) REJECT wrong secret: a key not derived from the proven hpk breaks the
// `hpk == hsk*G` relation (HSK2*G is HPK2, not HPK). A holder who does NOT hold
// the bound secret cannot satisfy it.
#[test(should_fail_with = "holder key mismatch")]
fn holder_pok_rejects_wrong_secret() {
    holder_pok(HDIG, HSK2, holder_hpk());
}

// (H3) REJECT key substitution: presenting a DIFFERENT (self-consistent) key pair
// against the issuer-attested digest HDIG fails the digest binding. This is the
// "malicious holder A substitutes its OWN key" forge: hpk2 = hsk2*G holds, but
// holder_key_digest(hpk2) != HDIG. (Design S4.1: A cannot launder B's credential
// under A's own key.)
#[test(should_fail_with = "holder digest mismatch")]
fn holder_pok_rejects_key_substitution() {
    holder_pok(HDIG, HSK2, Point { x: HPK2_X, y: HPK2_Y });
}

// (H4) REJECT digest mismatch: a tampered public digest (not the one bound to the
// real key) is rejected -- the in-circuit digest of the real key cannot equal it.
#[test(should_fail_with = "holder digest mismatch")]
fn holder_pok_rejects_tampered_digest() {
    holder_pok(HDIG + 1, HSK, holder_hpk());
}

// (H5) REJECT identity holder key (universal-degenerate guard, mirrors
// schnorr_verify's identity-key rejection). The neutral (0,1) is hsk*G only for
// hsk == 0 mod L and carries no possession secret.
#[test(should_fail_with = "identity holder key rejected")]
fn holder_pok_rejects_identity_key() {
    // digest of the neutral point, so only the identity guard (not the digest
    // binding) catches the lie.
    let id = Point { x: 0, y: 1 };
    holder_pok(holder_digest(id), 0, id);
}

// (H6) REJECT off-curve holder key (twist / small-subgroup forgery guard).
#[test(should_fail_with = "point is not on Baby-JubJub")]
fn holder_pok_rejects_off_curve_key() {
    // (1, 1) is not on Baby-JubJub (1 + 1 != 1 + d).
    let off = Point { x: 1, y: 1 };
    holder_pok(holder_digest(off), HSK, off);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): in-circuit hidden-holder
// SET membership. Vectors generated by `sparq_zk_compose::holder` from the REAL
// holder set K = {seed100, seed101, seed102, seed103} committed as a depth-2
// Poseidon2 Merkle tree whose leaf is `holder_key_digest(hpk)` (the canonical
// attested holder identity, NOT the issuer `h2(x,y)` shape). The in-set holder is
// seed102 at index 2 (its digest is HDIG, the SAME seed-102 holder key the
// holder_pok vectors above use); the membership path is
// [leaf(seed103), h2(leaf100, leaf101)]. The out-of-set holder is seed200 (the
// HPK2_* / HDIG2 key, NOT in K). NOT-yet-sound (sq-qhy4); no soundness property is
// asserted as achieved.
// ---------------------------------------------------------------------------

use crate::holder::{hidden_holder_set, holder_set_leaf, holder_set_membership};

// holder set K = {seed100, seed101, seed102, seed103} as a depth-2 tree. The
// seed-102 holder (HDIG) is at index 2; its path is [leaf(seed103), node(0,1)].
global HS_ROOT: Field = 0x2764f6597829fa6d121c559b5aee2d9bbf3212490f75198b74025d52e7d97bc9;
global HS_SIB0: Field = 0x1326436cfe5c0e122b2113d1a76160914a63572996e452ff2b4f455574d5e999;
global HS_SIB1: Field = 0x3023a9d3b938915778d33941416030795369f7f6fc2e03b2467546d0d4f5c03c;
global HS_IDX: Field = 2;

// (HS0) The holder-set leaf is the holder-key DIGEST (== HDIG), bit-identical to
// the host `holder_set_root`'s leaf -- distinct from the issuer key-set leaf
// `key_leaf = h2(x, y)` over the same key (the ZKSIG_HK domain tag separates them).
#[test]
fn holder_set_leaf_is_holder_digest() {
    assert_eq(holder_set_leaf(holder_hpk()), HDIG);
}

// (HS1) ACCEPT: the in-set holder's digest folds to the public root along the
// claimed path (the membership gadget alone, isolating it from the key-pair check).
#[test]
fn holder_set_membership_in_set_holds() {
    holder_set_membership::<2>(holder_hpk(), HS_ROOT, HS_IDX, [HS_SIB0, HS_SIB1]);
}

// (HS2) ACCEPT: the FULL hidden-holder-set relation -- possession of hsk
// (hpk = hsk*G) AND membership of holder_key_digest(hpk) in the committed set --
// verifies end-to-end. The holder key, secret, index, and path are ALL private;
// only `holder_set_root` is public (the hidden-holder upgrade over holder_pok,
// which makes the digest public).
#[test]
fn hidden_holder_set_in_set_verifies() {
    hidden_holder_set::<2>(HS_ROOT, HSK, holder_hpk(), HS_IDX, [HS_SIB0, HS_SIB1]);
}

// (HS3) REJECT out-of-set holder: a DIFFERENT (self-consistent) holder key pair
// (seed200, HDIG2) -- the prover knows hsk2 with hpk2 = hsk2*G -- is rejected
// because holder_key_digest(hpk2) is NOT a member of K, so the membership fold
// cannot recompute the public root. This is the holder analogue of the
// hidden_issuer out-of-set-key forge: a real, possessed key that is simply not in
// the set.
#[test(should_fail_with = "holder is not a member of the committed holder set")]
fn hidden_holder_set_out_of_set_holder_rejected() {
    // The out-of-set holder's own valid (hsk2, hpk2), folded against K's root with
    // the in-set member's path (the only path the prover could try to present).
    hidden_holder_set::<2>(
        HS_ROOT,
        HSK2,
        Point { x: HPK2_X, y: HPK2_Y },
        HS_IDX,
        [HS_SIB0, HS_SIB1],
    );
}

// (HS4) REJECT wrong secret: a key not derived from the proven hpk breaks
// `hpk == hsk*G` (HSK2*G is HPK2, not HPK) -- a "holder" who does NOT possess the
// in-set secret cannot satisfy the relation even with the correct path.
#[test(should_fail_with = "holder key mismatch")]
fn hidden_holder_set_wrong_secret_rejected() {
    hidden_holder_set::<2>(HS_ROOT, HSK2, holder_hpk(), HS_IDX, [HS_SIB0, HS_SIB1]);
}

// (HS5) REJECT wrong path: the correct in-set holder with a TAMPERED sibling cannot
// recompute the public root (the prover cannot forge an authentication path).
#[test(should_fail_with = "holder is not a member of the committed holder set")]
fn hidden_holder_set_wrong_path_rejected() {
    hidden_holder_set::<2>(HS_ROOT, HSK, holder_hpk(), HS_IDX, [HS_SIB0 + 1, HS_SIB1]);
}

// (HS6) REJECT index/path mismatch: presenting the in-set holder's path under a
// DIFFERENT claimed index folds the leaf along the wrong directions, so it cannot
// reach the root (a prover cannot reuse another slot's path -- the index bits are
// the fold directions, bound in-circuit).
#[test(should_fail_with = "holder is not a member of the committed holder set")]
fn hidden_holder_set_index_path_mismatch_rejected() {
    // index 0 (dirs [0,0]) with seed-102's path (which is for index 2, dirs [0,1]).
    hidden_holder_set::<2>(HS_ROOT, HSK, holder_hpk(), 0, [HS_SIB0, HS_SIB1]);
}

// (HS7) REJECT identity holder key (universal-degenerate guard, mirrors holder_pok
// / schnorr_verify). The neutral (0,1) is hsk*G only for hsk == 0 mod L and carries
// no possession secret -- rejected before the membership fold can be gamed.
#[test(should_fail_with = "identity holder key rejected")]
fn hidden_holder_set_identity_key_rejected() {
    let id = Point { x: 0, y: 1 };
    hidden_holder_set::<2>(HS_ROOT, 0, id, HS_IDX, [HS_SIB0, HS_SIB1]);
}

// (HS8) REJECT off-curve holder key (twist / small-subgroup forgery guard).
#[test(should_fail_with = "point is not on Baby-JubJub")]
fn hidden_holder_set_off_curve_key_rejected() {
    let off = Point { x: 1, y: 1 };
    hidden_holder_set::<2>(HS_ROOT, HSK, off, HS_IDX, [HS_SIB0, HS_SIB1]);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-bwwl / sq-8mwz: join_eq -- hidden cross-credential JOIN.
// accept + each adversarial reject + the public-input-hiding pin. Tiny (N_A=4,
// N_B=4) monomorphisations; the equality structure is what matters.
// ---------------------------------------------------------------------------
//
// Universe: two credentials sharing the join key JK (a global IRI encoding) at
// graph-A object slot 2 and graph-B subject slot 0 (the `?p` join column).
//   Graph A: { (S1, P, JK) }                  -- ?p in slot 2 (object)
//   Graph B: { (JK, P_OTHER, O2) }            -- ?p in slot 0 (subject)
// So a true join is row_a[2] == row_b[0] == JK, slot_a = 2, slot_b = 0.
global JK: Field = 0xabc123; // the (hidden) shared join-key encoding
global JB: Field = 0xb11d; // the per-presentation blinder

// Build a single-triple graph's commitment over an N=4 bucket (the same
// commit_fold the join relation re-derives from witnessed contents).
fn commit_one(triple: [Field; 3]) -> Field {
    let leaves: [Field; 4] = [h3(triple[0], triple[1], triple[2]), 0, 0, 0];
    commit_fold(leaves, 1)
}

// (J0) Cross-vector: the in-circuit join-value commitment matches the host
// `sparq_zk::sig::join_value_commitment` (single source of truth). The host
// half is pinned in `crates/sparq-zk/src/sig.rs::join_value_commitment_is_*`;
// JOIN_DOMAIN here == SIG_DOMAIN_JOIN there ("ZKSIG_JN" = 0x5a4b5349475f4a4e).
#[test]
fn join_value_commitment_matches_host() {
    assert_eq(JOIN_DOMAIN, 0x5a4b5349475f4a4e);
    // h3(JOIN_DOMAIN, value, blinding) -- the exact preimage the host computes.
    assert_eq(join_value_commitment(JK, JB), h3(JOIN_DOMAIN, JK, JB));
}

// (J1) ACCEPT: two credentials genuinely share JK at the bound slots; the
// equality proves and the hiding commitment binds the join value.
#[test]
fn join_eq_genuine_join_accepts() {
    let row_a = [101, 7, JK]; // (S1, P, JK)
    let row_b = [JK, 8, 202]; // (JK, P_OTHER, O2)
    let enc_a = [row_a, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_a = commit_one(row_a);
    let commit_b = commit_one(row_b);
    let jc = join_value_commitment(JK, JB);
    join_eq_check::<4, 4>(
        commit_a,
        commit_b,
        jc,
        2,
        0,
        enc_a,
        1,
        enc_b,
        1,
        row_a,
        row_b,
        JB,
    );
}

// (J2 / A1) REJECT forged equality: the two slots hold DIFFERENT terms. A prover
// claiming a join of unequal terms has no satisfying witness -- the in-circuit
// equality fails. (Graph B's subject is a DIFFERENT key JK2 != JK.)
#[test(should_fail_with = "join slots are not equal")]
fn join_eq_unequal_slots_rejected() {
    let jk2: Field = 0xdead; // a DIFFERENT join key
    let row_a = [101, 7, JK];
    let row_b = [jk2, 8, 202];
    let enc_a = [row_a, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_a = commit_one(row_a);
    let commit_b = commit_one(row_b);
    // Prover claims equality (and commits to JK), but row_b[0] = jk2 != JK.
    let jc = join_value_commitment(JK, JB);
    join_eq_check::<4, 4>(
        commit_a,
        commit_b,
        jc,
        2,
        0,
        enc_a,
        1,
        enc_b,
        1,
        row_a,
        row_b,
        JB,
    );
}

// (J3 / A2) REJECT row-swap / forged graph: the prover supplies graph-A contents
// that do NOT recommit to the PUBLIC commit_a (it tries to pass a fabricated
// graph the scan proof never attested). The commitment recompute (step 1)
// catches it before any equality is even reached.
#[test(should_fail_with = "graph-A commitment mismatch")]
fn join_eq_forged_graph_a_rejected() {
    let row_a = [101, 7, JK];
    let row_b = [JK, 8, 202];
    let enc_a = [row_a, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_b = commit_one(row_b);
    let jc = join_value_commitment(JK, JB);
    // commit_a is for a DIFFERENT graph (a row the prover does not actually
    // hold) -- it cannot match the re-commit of enc_a.
    let bogus_commit_a = commit_one([999, 999, 999]);
    join_eq_check::<4, 4>(
        bogus_commit_a,
        commit_b,
        jc,
        2,
        0,
        enc_a,
        1,
        enc_b,
        1,
        row_a,
        row_b,
        JB,
    );
}

// (J4 / A2) REJECT join over a row NOT present in the (correctly committed)
// graph: enc_a/commit_a are consistent, but row_a is a triple the graph does not
// contain (it shares JK in slot 2, so the equality WOULD pass, but the
// present-in-graph check fails -- the join must attach to a real credential row).
#[test(should_fail_with = "join row A not present in committed graph A")]
fn join_eq_row_not_in_graph_rejected() {
    let real_a = [101, 7, JK]; // the row actually in graph A
    let row_b = [JK, 8, 202];
    let enc_a = [real_a, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_a = commit_one(real_a);
    let commit_b = commit_one(row_b);
    let jc = join_value_commitment(JK, JB);
    // A fabricated row_a that shares JK in slot 2 but is NOT a row of graph A.
    let phantom_a = [555, 555, JK];
    join_eq_check::<4, 4>(
        commit_a,
        commit_b,
        jc,
        2,
        0,
        enc_a,
        1,
        enc_b,
        1,
        phantom_a,
        row_b,
        JB,
    );
}

// (J5) REJECT a join commitment that does NOT bind the proven join value (the
// prover proves equality on JK but commits to a DIFFERENT value -- equivocation
// guard, design S4.3).
#[test(should_fail_with = "join commitment does not bind the proven join value")]
fn join_eq_wrong_commitment_rejected() {
    let row_a = [101, 7, JK];
    let row_b = [JK, 8, 202];
    let enc_a = [row_a, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_a = commit_one(row_a);
    let commit_b = commit_one(row_b);
    // Commitment to a DIFFERENT value than the join value JK that was proven.
    let wrong_jc = join_value_commitment(0xfeed, JB);
    join_eq_check::<4, 4>(
        commit_a,
        commit_b,
        wrong_jc,
        2,
        0,
        enc_a,
        1,
        enc_b,
        1,
        row_a,
        row_b,
        JB,
    );
}

// (J6) REJECT an out-of-range join slot (must be subject/predicate/object).
#[test(should_fail_with = "join slot out of range (must be subject/predicate/object)")]
fn join_eq_out_of_range_slot_rejected() {
    let row_a = [101, 7, JK];
    let row_b = [JK, 8, 202];
    let enc_a = [row_a, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_a = commit_one(row_a);
    let commit_b = commit_one(row_b);
    let jc = join_value_commitment(JK, JB);
    join_eq_check::<4, 4>(
        commit_a,
        commit_b,
        jc,
        3,
        0,
        enc_a,
        1,
        enc_b,
        1,
        row_a,
        row_b,
        JB,
    );
}

// (J7) PUBLIC-INPUT HIDING PIN (the headline privacy property). The relation's
// PUBLIC interface is exactly [challenge, commit_a, commit_b, join_commitment,
// slot_a, slot_b] (see `join_eq_na16_nb16/src/main.nr`). This test pins that the
// joined term encoding JK is ABSENT from every one of those public artefacts:
//   - commit_a / commit_b are commitments over the WHOLE graph (JK is one of
//     many absorbed leaves -- not recoverable without the full preimage);
//   - join_commitment is HIDING (Poseidon2 over JK *and a secret blinder*), so
//     it differs under a different blinder and is not a bare Enc(JK);
//   - slot_a / slot_b are positions, not values.
// A verifier reading the public inputs never sees JK. (The accept path J1 proves
// the same witnesses verify; here we assert the join VALUE does not equal any
// public field, and that the hiding commitment is blinder-dependent.)
#[test]
fn join_eq_value_absent_from_public_inputs() {
    let row_a = [101, 7, JK];
    let row_b = [JK, 8, 202];
    let commit_a = commit_one(row_a);
    let commit_b = commit_one(row_b);
    let jc = join_value_commitment(JK, JB);
    let slot_a: Field = 2;
    let slot_b: Field = 0;
    // The hidden join value is not literally any public input.
    assert(JK != commit_a);
    assert(JK != commit_b);
    assert(JK != jc);
    assert(JK != slot_a);
    assert(JK != slot_b);
    // The public join artefact is HIDING: the SAME join value under a DIFFERENT
    // blinder yields a different commitment, so the verifier cannot dictionary
    // -attack JK by recomputing a bare encoding (design S1.4 R4).
    let jc_other_blinder = join_value_commitment(JK, JB + 1);
    assert(jc != jc_other_blinder);
    // And the commitment is NOT the bare encoding of the value (which a verifier
    // *could* recompute from a guessed JK) -- the blinder is load-bearing.
    assert(jc != JK);
}

// (J8) ACCEPT a join where the shared key sits at DIFFERENT slots in each
// credential and one graph has MORE than one triple (the join row is selected
// from among several present rows). Exercises select_slot at slot 1 and the
// multi-row present-in-graph sweep.
#[test]
fn join_eq_mixed_slots_multi_row_accepts() {
    // Graph A holds two triples; the join key JK is in slot 1 (predicate-position
    // join, e.g. a reified statement) of the SECOND triple.
    let a0 = [11, 22, 33];
    let a1 = [44, JK, 66];
    let enc_a = [a0, a1, [0, 0, 0], [0, 0, 0]];
    // Graph B holds JK in slot 2 (object).
    let row_b = [77, 88, JK];
    let enc_b = [row_b, [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let commit_a = commit_fold([h3(a0[0], a0[1], a0[2]), h3(a1[0], a1[1], a1[2]), 0, 0], 2);
    let commit_b = commit_one(row_b);
    let jc = join_value_commitment(JK, JB);
    // slot_a = 1 (predicate of a1), slot_b = 2 (object of row_b); join on a1.
    join_eq_check::<4, 4>(
        commit_a,
        commit_b,
        jc,
        1,
        2,
        enc_a,
        2,
        enc_b,
        1,
        a1,
        row_b,
        JB,
    );
}

// ---------------------------------------------------------------------------
// path_reach_check -- accept + adversarial (sq-3kd2g.2 / #1591, record sec 4).
// Tiny (D=3, K=1, N=8) monomorphisation for the single-graph cases; a (D=2,
// K=2, N=4) one for cross-graph chaining, attribution and the strict-ordering
// guard. The padding-forgery negatives are NORMATIVE (record sec 4 req 4).
// ---------------------------------------------------------------------------

// [FABLE-5] Path-test universe: node term encodings (arbitrary nonzero field
// elements; the relation only sees equality structure). The single committed
// graph holds a P-chain NA -> NB -> NC -> ND, a P_OTHER distractor edge
// NB -> ND (so a P-proof cannot shortcut through it), and a P back-edge
// NB -> NA (so a chain may legally revisit nodes -- record sec 4 req 6).
global NA: Field = 401;
global NB: Field = 402;
global NC: Field = 403;
global ND: Field = 404;
global GARBAGE: Field = 0xdead;

fn path_graph() -> ([[[Field; 3]; 8]; 1], [u32; 1], [Field; 1]) {
    let enc: [[[Field; 3]; 8]; 1] = [
        [
            [NA, P, NB],
            [NB, P, NC],
            [NC, P, ND],
            [NB, P_OTHER, ND],
            [NB, P, NA],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
        ],
    ];
    let counts: [u32; 1] = [5];
    let mut leaves: [Field; 8] = [0; 8];
    for i in 0..8 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, counts[0])];
    (enc, counts, commitments)
}

global PATH_ATTR: [bool; 1] = [true];

#[test]
fn path_accepts_single_step() {
    let (enc, counts, commitments) = path_graph();
    // NA -P-> NB, one active step; padding nodes unconstrained garbage.
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NB, false, 3, PATH_ATTR, 1, [NB, GARBAGE, GARBAGE], counts, enc,
    );
}

#[test]
fn path_accepts_full_depth_chain() {
    let (enc, counts, commitments) = path_graph();
    // NA -P-> NB -P-> NC -P-> ND: all D steps active.
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 3, [NB, NC, ND], counts, enc,
    );
}

#[test]
fn path_accepts_shorter_chain_with_garbage_padding() {
    let (enc, counts, commitments) = path_graph();
    // l = 2 < D = 3: the inactive third row carries garbage and must be inert
    // on the ACCEPT side too (an honest shorter chain proves fine).
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NC, false, 3, PATH_ATTR, 2, [NB, NC, GARBAGE], counts, enc,
    );
}

// Record sec 4 req 6: cycles are harmless for membership -- the chain may revisit
// nodes (NA -P-> NB -P-> NA -P-> NB uses the committed back-edge).
#[test]
fn path_accepts_chain_revisiting_nodes() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NB, false, 3, PATH_ATTR, 3, [NB, NA, NB], counts, enc,
    );
}

// Record sec 4 req 5 accept side: zero-length path (`p*`) with the endpoint term
// occurring as a NODE (subject and object) of the committed union.
#[test]
fn path_accepts_zero_length_when_allowed() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NC, NC, true, 3, PATH_ATTR, 0, [GARBAGE, GARBAGE, GARBAGE], counts, enc,
    );
}

// The chain cannot jump: (NA, P, NC) is not a committed triple.
#[test(should_fail_with = "chain step not in any committed graph")]
fn path_rejects_uncommitted_step() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NC, false, 3, PATH_ATTR, 1, [NC, GARBAGE, GARBAGE], counts, enc,
    );
}

// Predicate soundness: NB -> ND exists ONLY under P_OTHER; a P-path proof
// cannot use it (the membership probe carries pred_enc structurally).
#[test(should_fail_with = "chain step not in any committed graph")]
fn path_rejects_wrong_predicate_edge() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NB, ND, false, 3, PATH_ATTR, 1, [ND, GARBAGE, GARBAGE], counts, enc,
    );
}

// Endpoint forgery: an honest single step NA -> NB cannot claim destination ND.
#[test(should_fail_with = "chain does not reach the destination")]
fn path_rejects_wrong_destination() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 1, [NB, GARBAGE, GARBAGE], counts, enc,
    );
}

// NORMATIVE padding forge-negative (record sec 4 req 4, direction 1): a SHORTER
// real chain cannot forge a LONGER claim -- the prover holds only NA -P-> NB
// (l = 1) and crafts the INACTIVE second row to "reach" ND; the pass-through
// padding never advances the endpoint, so the destination check trips.
#[test(should_fail_with = "chain does not reach the destination")]
fn path_rejects_padding_row_extending_the_chain() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 1, [NB, ND, GARBAGE], counts, enc,
    );
}

// NORMATIVE padding forge-negative (record sec 4 req 4, direction 2): declaring
// the crafted row ACTIVE instead (l = 2) walks it into the membership check --
// (NB, P, ND) is not a committed triple.
#[test(should_fail_with = "chain step not in any committed graph")]
fn path_rejects_activating_a_fabricated_step() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 2, [NB, ND, GARBAGE], counts, enc,
    );
}

// `p+` (allow_zero = false) requires at least one step.
#[test(should_fail_with = "zero-length path not allowed for this operator")]
fn path_rejects_zero_length_when_disallowed() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NC, NC, false, 3, PATH_ATTR, 0, [GARBAGE, GARBAGE, GARBAGE], counts, enc,
    );
}

// Record sec 4 req 5: bare src == dst equality is NOT enough for a zero-length
// path -- the term must occur in the committed union.
#[test(should_fail_with = "zero-length endpoint not a node of any committed graph")]
fn path_rejects_zero_length_endpoint_not_committed() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, GARBAGE, GARBAGE, true, 3, [true], 0,
        [GARBAGE, GARBAGE, GARBAGE], counts, enc,
    );
}

// SPARQL's zero-length term universe is the graph's NODES (subjects/objects):
// P occurs only in predicate position, so it is not a zero-length endpoint.
#[test(should_fail_with = "zero-length endpoint not a node of any committed graph")]
fn path_rejects_zero_length_predicate_only_occurrence() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, P, P, true, 3, [true], 0, [GARBAGE, GARBAGE, GARBAGE], counts, enc,
    );
}

// A zero-length claim between DIFFERENT endpoints fails the endpoint check
// (the never-advanced running endpoint is still src).
#[test(should_fail_with = "chain does not reach the destination")]
fn path_rejects_zero_length_unequal_endpoints() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NB, true, 3, PATH_ATTR, 0, [GARBAGE, GARBAGE, GARBAGE], counts, enc,
    );
}

#[test(should_fail_with = "path length exceeds circuit depth bound")]
fn path_rejects_length_over_the_bound() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 4, [NB, NC, ND], counts, enc,
    );
}

// Mechanism (a) belt-and-braces: the public depth_bound input must equal the
// member's compile-time D (the VK identity already discloses it; a manifest
// cannot surface a different number).
#[test(should_fail_with = "depth bound does not match the circuit member")]
fn path_rejects_depth_bound_mismatch() {
    let (enc, counts, commitments) = path_graph();
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NB, false, 4, PATH_ATTR, 1, [NB, GARBAGE, GARBAGE], counts, enc,
    );
}

#[test(should_fail_with = "graph commitment mismatch")]
fn path_rejects_tampered_commitment() {
    let (enc, counts, mut commitments) = path_graph();
    commitments[0] += 1;
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, NB, false, 3, PATH_ATTR, 1, [NB, GARBAGE, GARBAGE], counts, enc,
    );
}

// --- cross-graph fixture (D=2, K=2, N=4) ----------------------------------
//
// Graph A holds the chain edges { (NA, P, NB), (NB, P, NC) }; graph B holds
// one unrelated P_OTHER triple. The fixture canonicalises the pair into
// ascending-commitment order (the relation's step-1b contract, exactly like
// `test_graphs`) and returns graph A's canonical index so the attribution
// tests can name the truly-contributing graph regardless of hash order.
fn path_cross_graphs() -> ([[[Field; 3]; 4]; 2], [u32; 2], [Field; 2], u32) {
    let ga: [[Field; 3]; 4] = [[NA, P, NB], [NB, P, NC], [0, 0, 0], [0, 0, 0]];
    let gb: [[Field; 3]; 4] = [[S2, P_OTHER, O2], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let mut la: [Field; 4] = [0; 4];
    let mut lb: [Field; 4] = [0; 4];
    for i in 0..4 {
        la[i] = h3(ga[i][0], ga[i][1], ga[i][2]);
        lb[i] = h3(gb[i][0], gb[i][1], gb[i][2]);
    }
    let ca = commit_fold(la, 2);
    let cb = commit_fold(lb, 1);
    if ca.lt(cb) {
        ([ga, gb], [2, 1], [ca, cb], 0)
    } else {
        ([gb, ga], [1, 2], [cb, ca], 1)
    }
}

// Accept: the whole chain lives in graph A; graph B contributes nothing --
// and the true attribution says exactly that. The inactive-row corner is
// covered by the D=2 bucket being full here (both steps active).
#[test]
fn path_accepts_with_exact_cross_graph_attribution() {
    let (enc, counts, commitments, idx_a) = path_cross_graphs();
    let mut attr: [bool; 2] = [false, false];
    attr[idx_a] = true;
    path_reach_check::<2, 2, 4>(commitments, P, NA, NC, false, 2, attr, 2, [NB, NC], counts, enc);
}

// Overstating: claiming the non-contributing graph B as a source is rejected
// (the attested set is exact w.r.t. the witnessed chain, both directions).
#[test(should_fail_with = "attribution does not match the witnessed chain's source graphs")]
fn path_rejects_attribution_overstated() {
    let (enc, counts, commitments, _idx_a) = path_cross_graphs();
    path_reach_check::<2, 2, 4>(
        commitments, P, NA, NC, false, 2, [true, true], 2, [NB, NC], counts, enc,
    );
}

// Understating: hiding the contributing graph A is rejected too.
#[test(should_fail_with = "attribution does not match the witnessed chain's source graphs")]
fn path_rejects_attribution_understated() {
    let (enc, counts, commitments, idx_a) = path_cross_graphs();
    let mut attr: [bool; 2] = [false, false];
    attr[1 - idx_a] = true;
    path_reach_check::<2, 2, 4>(commitments, P, NA, NC, false, 2, attr, 2, [NB, NC], counts, enc);
}

// A chain may SPAN graphs: put the two edges in two separate committed graphs
// (canonicalised ascending, like every fixture) -- both source bits are true.
#[test]
fn path_accepts_chain_spanning_two_graphs() {
    let g1: [[Field; 3]; 4] = [[NA, P, NB], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let g2: [[Field; 3]; 4] = [[NB, P, NC], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let mut l1: [Field; 4] = [0; 4];
    let mut l2: [Field; 4] = [0; 4];
    for i in 0..4 {
        l1[i] = h3(g1[i][0], g1[i][1], g1[i][2]);
        l2[i] = h3(g2[i][0], g2[i][1], g2[i][2]);
    }
    let c1 = commit_fold(l1, 1);
    let c2 = commit_fold(l2, 1);
    let (enc, commitments) = if c1.lt(c2) {
        ([g1, g2], [c1, c2])
    } else {
        ([g2, g1], [c2, c1])
    };
    path_reach_check::<2, 2, 4>(
        commitments, P, NA, NC, false, 2, ATTR_BOTH, 2, [NB, NC], [1, 1], enc,
    );
}

// Family guard 1b carries over: duplicate (non-increasing) commitments are
// rejected before any chain logic.
#[test(should_fail_with = "per-graph commitments not strictly increasing")]
fn path_rejects_duplicate_commitments() {
    let (enc, counts, commitments) = path_graph();
    let enc2: [[[Field; 3]; 8]; 2] = [enc[0], enc[0]];
    let counts2: [u32; 2] = [counts[0], counts[0]];
    let commitments2: [Field; 2] = [commitments[0], commitments[0]];
    path_reach_check::<3, 2, 8>(
        commitments2, P, NA, NB, false, 3, ATTR_BOTH, 1, [NB, GARBAGE, GARBAGE], counts2, enc2,
    );
}

// [FABLE-5] SLOT-padding forgery (the scan relation's other padding surface,
// orthogonal to the CHAIN-padding negatives above): a fabricated row parked in
// an INACTIVE slot (index >= counts[g]). The commitment recompute still passes
// (commit_fold only folds the first counts[g] leaves), so the membership probe
// is the only thing standing between this row and a forged chain step -- it
// must not see slot indices beyond the committed count.
#[test(should_fail_with = "chain step not in any committed graph")]
fn path_rejects_fabricated_row_in_inactive_slot() {
    let (mut enc, counts, commitments) = path_graph();
    // Slot 5 is beyond counts[0] = 5: plant the missing shortcut (NA, P, ND).
    enc[0][5] = [NA, P, ND];
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 1, [ND, GARBAGE, GARBAGE], counts, enc,
    );
}

// Same slot-padding surface for the zero-length occurrence witness (record
// sec 4 req 5): an endpoint term occurring ONLY in an inactive slot is NOT a
// node of the committed union.
#[test(should_fail_with = "zero-length endpoint not a node of any committed graph")]
fn path_rejects_zero_length_occurrence_in_inactive_slot() {
    let (mut enc, counts, commitments) = path_graph();
    enc[0][5] = [GARBAGE, P, GARBAGE];
    path_reach_check::<3, 1, 8>(
        commitments, P, GARBAGE, GARBAGE, true, 3, PATH_ATTR, 0,
        [GARBAGE, GARBAGE, GARBAGE], counts, enc,
    );
}

// ---------------------------------------------------------------------------
// [FABLE-5] sq-apwxw: Mutation-pinning tests for the reviewer-identified
// surviving mutants (escalated review comment, 2026-07-06).
// ---------------------------------------------------------------------------

// --- Item 1: padding-row attribution gate (`active &` in contrib) -----------
//
// Fixture (D=2, K=2, N=4): graph A holds the single chain edge (NA, P, NB);
// graph B holds (NB, P, ND) -- the chain's endpoint NB appears as the SUBJECT of
// graph B's committed triple. The prover witnesses: src=NA, dst=NB, path_len=1,
// nodes=[NB, ND]. Step 0 is ACTIVE (NA-P->NB, attributed to A). Step 1 is
// INACTIVE: nodes[1]=ND, but `cur` stays at NB (pass-through) and the inactive
// step must NOT attribute graph B even though graph B contains (NB, P, ND) and
// `present_g` would be true for that step.
//
// Without `active &` in `contrib[g] |= active & present_g`, graph B would be
// spuriously attributed at the inactive step (provenance forgery). The accept
// test pins that the correct circuit accepts [A only]; the reject test pins that
// claiming [A,B] is rejected -- both mutations kill the surviving `contrib` mutant.
fn path_graph_k2_attr_inactive() -> ([[[Field; 3]; 4]; 2], [u32; 2], [Field; 2], u32) {
    // Graph A: the chain edge only. Graph B: (NB, P, ND) -- NB is the chain
    // endpoint; an inactive padding row nodes[1]=ND walks into this triple's
    // "present" check but the step is inactive and must not attribute.
    let ga: [[Field; 3]; 4] = [[NA, P, NB], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let gb: [[Field; 3]; 4] = [[NB, P, ND], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
    let mut la: [Field; 4] = [0; 4];
    let mut lb: [Field; 4] = [0; 4];
    for i in 0..4 {
        la[i] = h3(ga[i][0], ga[i][1], ga[i][2]);
        lb[i] = h3(gb[i][0], gb[i][1], gb[i][2]);
    }
    let ca = commit_fold(la, 1);
    let cb = commit_fold(lb, 1);
    // Canonicalise into ascending-commitment order (step-1b contract).
    if ca.lt(cb) {
        ([ga, gb], [1, 1], [ca, cb], 0)
    } else {
        ([gb, ga], [1, 1], [cb, ca], 1)
    }
}

// ACCEPT: chain NA-P->NB lives in graph A only; graph B's committed triple
// (NB,P,ND) is not visited by any ACTIVE step, so the true attribution is
// [A only]. The real circuit accepts; the mutant (missing `active &`) would
// compute [A,B] and fail the attribution assertion here.
#[test]
fn path_accepts_inactive_row_does_not_attribute_graph_b() {
    let (enc, counts, commitments, idx_a) = path_graph_k2_attr_inactive();
    let mut attr: [bool; 2] = [false, false];
    attr[idx_a] = true;
    // nodes[1] = ND (inactive): cur stays NB; graph B's (NB,P,ND) is not attributed.
    path_reach_check::<2, 2, 4>(commitments, P, NA, NB, false, 2, attr, 1, [NB, ND], counts, enc);
}

// REJECT: claiming that graph B (which contributes only via the INACTIVE step)
// is also a source -- the correct circuit's exact attribution check catches the
// overstatement. (The mutant would accept [A,B] and pass this through, revealing it.)
#[test(should_fail_with = "attribution does not match the witnessed chain's source graphs")]
fn path_rejects_inactive_row_attribution_overstatement() {
    let (enc, counts, commitments, _idx_a) = path_graph_k2_attr_inactive();
    // Claim both graphs contributed, but only graph A's chain edge was active.
    path_reach_check::<2, 2, 4>(
        commitments, P, NA, NB, false, 2, [true, true], 1, [NB, ND], counts, enc,
    );
}

// --- Item 2: padding-advance direction -- forged dst in the LAST padding row --
//
// Sibling to `path_rejects_padding_row_extending_the_chain`: the forged destination
// ND is planted in the LAST (index 2) padding row instead of index 1. The pass-
// through guard applies to ALL inactive rows regardless of position; the endpoint
// check must still fail. This shape is exactly the "strongest witness" the escalated
// reviewer verified manually on the PR head -- now pinned as a permanent test.
#[test(should_fail_with = "chain does not reach the destination")]
fn path_rejects_padding_row_forged_dst_in_last_slot() {
    let (enc, counts, commitments) = path_graph();
    // l=1 (only NA-P->NB active); nodes=[NB, GARBAGE, ND]: the forged destination
    // ND is in the last (index 2) padding row. Inactive steps cannot advance cur.
    path_reach_check::<3, 1, 8>(
        commitments, P, NA, ND, false, 3, PATH_ATTR, 1, [NB, GARBAGE, ND], counts, enc,
    );
}

// ---------------------------------------------------------------------------
// [FABLE-5] sq-g91d: in-circuit RDFS single-step entailment (entail_step_check).
// Proves a derived triple's UNDISCLOSED antecedents are members of the committed
// graph (the privacy upgrade for the Rdfs/Owl regimes over the host disclosed-
// base re-check `bind_entailment`). Coverage (noir-testing convention, message-
// matched): accepts EN0/EN1 (single graph, both rules) + EN9 (K=2 split
// antecedents, exact cross-graph attribution); should_fail per rejection clause --
// rule shape (EN2), membership via BOTH the uncommitted-triple (EN3) and the
// inactive-padding-slot (EN8) paths, commitment recompute (EN4), unknown rule
// (EN5), attribution understated (EN6) and overstated cross-graph (EN12), bucket
// bound (EN7), and the K>=2 strict commitment ordering via duplicate (EN10) and
// descending (EN11) pairs. The committed graph's commitment is computed IN the
// test from the same witnessed leaves, so the vectors are self-consistent with
// the `commit_fold`/`h3` cross-vectors asserted at the top of this module.
// NOT-yet-sound (sq-qhy4); no soundness property is asserted as achieved.
// ---------------------------------------------------------------------------

use crate::entail::{entail_step_check, RULE_RDFS7_SUBPROPERTY, RULE_RDFS9_SUBCLASS_TYPE};

// Public schema-vocabulary encodings (stand-ins; the real verifier pins these to
// the true rdf:type / rdfs:subClassOf / rdfs:subPropertyOf IRI encodings).
global E_TYPE: Field = 1001;
global E_SUBCLASS: Field = 1002;
global E_SUBPROP: Field = 1003;

// rdfs9 individuals/classes: x=(10) type c1=(20) subClassOf c2=(30).
global E_X: Field = 10;
global E_C1: Field = 20;
global E_C2: Field = 30;

// A committed graph (N=8, count=2) holding EXACTLY the two rdfs9 antecedents
// (x type c1) and (c1 subClassOf c2), padded with zero rows. The commitment is
// recomputed from these leaves, so `entail_step_check`'s step-1 recompute matches.
fn rdfs9_graph() -> ([[[Field; 3]; 8]; 1], [u32; 1], [Field; 1]) {
    let mut enc: [[[Field; 3]; 8]; 1] = [[[0; 3]; 8]; 1];
    enc[0][0] = [E_X, E_TYPE, E_C1]; //      (x type c1)
    enc[0][1] = [E_C1, E_SUBCLASS, E_C2]; // (c1 subClassOf c2)
    let counts: [u32; 1] = [2];
    let mut leaves: [Field; 8] = [0; 8];
    for i in 0..8 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, 2)];
    (enc, counts, commitments)
}

// (EN0) ACCEPT rdfs9: derived (x type c2) is proved from the two COMMITTED,
// UNDISCLOSED antecedents. Only `derived` (and the public schema encodings) is
// disclosed; the antecedents are private witnesses.
#[test]
fn entail_rdfs9_accepts_committed_antecedents() {
    let (enc, counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2], // derived (x type c2)
        [true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN1) ACCEPT rdfs7: (p1 subPropertyOf p2),(x p1 y) |- (x p2 y), antecedents
// committed + undisclosed. A distinct committed graph so the rule branch is
// exercised end to end.
#[test]
fn entail_rdfs7_accepts_committed_antecedents() {
    // p1=40, p2=50, subj=11, obj=12.
    let mut enc: [[[Field; 3]; 8]; 1] = [[[0; 3]; 8]; 1];
    enc[0][0] = [40, E_SUBPROP, 50]; // (p1 subPropertyOf p2)
    enc[0][1] = [11, 40, 12]; //        (x p1 y)
    let counts: [u32; 1] = [2];
    let mut leaves: [Field; 8] = [0; 8];
    for i in 0..8 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, 2)];
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS7_SUBPROPERTY,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [11, 50, 12], // derived (x p2 y)
        [true],
        [[40, E_SUBPROP, 50], [11, 40, 12]],
        counts,
        enc,
    );
}

// (EN2) REJECT malformed rule instance: the derived object does not match the
// rule's consequent (derived (x type 999) instead of (x type c2)), so the
// well-formedness disjunct fails though both antecedents ARE committed (isolating
// the shape clause from the membership clause).
#[test(should_fail_with = "derivation step is not a valid instance of its rule")]
fn entail_rejects_malformed_consequent() {
    let (enc, counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, 999], // wrong consequent object
        [true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN3) REJECT ungrounded antecedent: an antecedent that is NOT a member of the
// committed graph. The graph commits only (x type c1) + (c1 subClassOf c2); the
// prover swaps a0 for (x type c9) (well-formed shape with c1:=c9, c2 stays), so
// the shape holds but (x type c9) is not committed -- the membership clause fires.
#[test(should_fail_with = "antecedent not a member of any committed graph")]
fn entail_rejects_ungrounded_antecedent() {
    // Commit (x type c9) is ABSENT: graph holds (x type c1),(c1 subClassOf c2).
    // Present antecedents (x type C9),(C9 subClassOf c2) |- (x type c2): well-formed
    // (shared class C9), but neither antecedent under C9 is committed.
    let (enc, counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true],
        [[E_X, E_TYPE, 900], [900, E_SUBCLASS, E_C2]], // uncommitted shared class 900
        counts,
        enc,
    );
}

// (EN4) REJECT commitment mismatch: a wrong public commitment cannot be
// recomputed from the witnessed leaves (the step-1 bind that ties the antecedent
// membership to the SCAN commitments).
#[test(should_fail_with = "graph commitment mismatch")]
fn entail_rejects_commitment_mismatch() {
    let (enc, counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        [commitments[0] + 1], // tampered commitment
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN5) REJECT unknown rule: a `rule` selector outside {0, 1}.
#[test(should_fail_with = "unknown entailment rule")]
fn entail_rejects_unknown_rule() {
    let (enc, counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        commitments,
        5, // not a registered rule
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN6) REJECT attribution understated: both antecedents live in graph 0, so the
// true attribution is [true]; declaring [false] hides the contributing graph.
#[test(should_fail_with = "attribution does not match the proved source graphs")]
fn entail_rejects_attribution_understated() {
    let (enc, counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [false], // understated
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN7) REJECT graph size exceeds bucket: a count above the compiled slot bound N.
#[test(should_fail_with = "graph size exceeds circuit bucket")]
fn entail_rejects_count_over_bucket() {
    let (enc, _counts, commitments) = rdfs9_graph();
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        [9], // 9 > N=8
        enc,
    );
}

// (EN8) REJECT antecedent hidden in an INACTIVE padding slot: the graph COMMITS
// only (x type c1) -- count = 1, and `commit_fold`'s runtime count keeps slot 1
// outside the absorbed message (pinned by
// `commit_fold_padding_slots_do_not_enter_the_commitment`), so the step-1
// recompute still matches. The second rdfs9 antecedent (c1 subClassOf c2) is
// planted byte-for-byte in padding slot 1 (index 1 >= count): the rule instance
// is well formed and the bytes ARE in `enc`, but `slot_active = i < counts[g]`
// must exclude the slot from the membership sweep -- otherwise a prover could
// smuggle an UNCOMMITTED antecedent through the padding region.
#[test(should_fail_with = "antecedent not a member of any committed graph")]
fn entail_rejects_antecedent_in_inactive_padding_slot() {
    let mut enc: [[[Field; 3]; 8]; 1] = [[[0; 3]; 8]; 1];
    enc[0][0] = [E_X, E_TYPE, E_C1]; //      (x type c1)          -- committed
    enc[0][1] = [E_C1, E_SUBCLASS, E_C2]; // (c1 subClassOf c2)   -- PADDING, not committed
    let counts: [u32; 1] = [1];
    let mut leaves: [Field; 8] = [0; 8];
    for i in 0..8 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, 1)];
    entail_step_check::<1, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// K=2 fixture: the two rdfs9 antecedents SPLIT across two committed graphs, one
// active slot each. Commitments are recomputed from the witnessed leaves and the
// graphs are laid out in CANONICAL ascending-commitment order (the circuit's
// step-1b contract); the trailing assert fails LOUDLY here -- a clear fixture
// bug -- if an encoding change ever flips the natural order (same convention as
// `test_graphs`).
//
// Graph 0: { (c1 subClassOf c2) }  -- holds antecedent a1.
// Graph 1: { (x type c1) }         -- holds antecedent a0.
// (`commit_fold` is hash-like, so which graph commits to the smaller field
// element is content-arbitrary; the subClassOf graph happens to be graph 0.)
fn entail_k2_graphs() -> ([[[Field; 3]; 8]; 2], [u32; 2], [Field; 2]) {
    let mut enc: [[[Field; 3]; 8]; 2] = [[[0; 3]; 8]; 2];
    enc[0][0] = [E_C1, E_SUBCLASS, E_C2];
    enc[1][0] = [E_X, E_TYPE, E_C1];
    let counts: [u32; 2] = [1, 1];
    let mut commitments: [Field; 2] = [0, 0];
    for g in 0..2 {
        let mut leaves: [Field; 8] = [0; 8];
        for i in 0..8 {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
        }
        commitments[g] = commit_fold(leaves, counts[g]);
    }
    assert(
        commitments[0].lt(commitments[1]),
        "entail_k2_graphs fixture not in canonical ascending-commitment order",
    );
    (enc, counts, commitments)
}

// (EN9) ACCEPT K=2 split antecedents: a1 is a member of graph 0 only, a0 of
// graph 1 only, so BOTH the cross-graph membership sweep and the per-graph
// attribution accumulation are exercised -- the exact attribution is
// [true, true], each graph contributing exactly one antecedent.
#[test]
fn entail_k2_accepts_split_antecedents_with_exact_attribution() {
    let (enc, counts, commitments) = entail_k2_graphs();
    entail_step_check::<2, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true, true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN10) REJECT duplicate commitments: the SAME committed graph presented twice
// (the duplicate-inclusion / COUNT forgery shape the guard exists for). Both
// slots recompute fine in step 1, but equality is non-strict, so the step-1b
// ordering guard fires.
#[test(should_fail_with = "per-graph commitments not strictly increasing")]
fn entail_k2_rejects_duplicate_commitment() {
    let (enc1, counts1, commitments1) = rdfs9_graph();
    let enc: [[[Field; 3]; 8]; 2] = [enc1[0], enc1[0]];
    let counts: [u32; 2] = [counts1[0], counts1[0]];
    let commitments: [Field; 2] = [commitments1[0], commitments1[0]];
    entail_step_check::<2, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true, true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// (EN11) REJECT descending commitment order: the K=2 split fixture with both
// graphs swapped CONSISTENTLY (enc/counts/commitments together), so each
// commitment still recomputes in step 1 -- only the strictly-DECREASING pair
// violates the canonical-order contract, isolating the ordering clause.
#[test(should_fail_with = "per-graph commitments not strictly increasing")]
fn entail_k2_rejects_descending_commitment_order() {
    let (enc, counts, commitments) = entail_k2_graphs();
    let enc_swapped: [[[Field; 3]; 8]; 2] = [enc[1], enc[0]];
    let counts_swapped: [u32; 2] = [counts[1], counts[0]];
    let commitments_swapped: [Field; 2] = [commitments[1], commitments[0]];
    entail_step_check::<2, 8>(
        commitments_swapped,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true, true],
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts_swapped,
        enc_swapped,
    );
}

// (EN12) REJECT overstated cross-graph attribution: BOTH antecedents live in
// one graph; the other committed graph holds only an unrelated triple and
// contributes nothing to the derivation. Claiming [true, true] overstates the
// non-contributing graph -- the exact-set discipline (scan step 4) in the
// cross-graph direction EN6's single-graph understatement cannot reach. The
// [true, true] claim is wrong whichever index the contributing graph canonically
// lands at, so this does not depend on the fixture's commitment order.
#[test(should_fail_with = "attribution does not match the proved source graphs")]
fn entail_k2_rejects_attribution_overstating_noncontributing_graph() {
    let mut enc: [[[Field; 3]; 8]; 2] = [[[0; 3]; 8]; 2];
    // One graph holds BOTH rdfs9 antecedents; the other an unrelated triple
    // (canonical ascending-commitment order puts the unrelated graph at 0).
    enc[0][0] = [11, 40, 12]; // unrelated: contributes NO antecedent
    enc[1][0] = [E_X, E_TYPE, E_C1];
    enc[1][1] = [E_C1, E_SUBCLASS, E_C2];
    let counts: [u32; 2] = [1, 2];
    let mut commitments: [Field; 2] = [0, 0];
    for g in 0..2 {
        let mut leaves: [Field; 8] = [0; 8];
        for i in 0..8 {
            leaves[i] = h3(enc[g][i][0], enc[g][i][1], enc[g][i][2]);
        }
        commitments[g] = commit_fold(leaves, counts[g]);
    }
    assert(
        commitments[0].lt(commitments[1]),
        "entail EN12 fixture not in canonical ascending-commitment order",
    );
    entail_step_check::<2, 8>(
        commitments,
        RULE_RDFS9_SUBCLASS_TYPE,
        E_TYPE, E_SUBCLASS, E_SUBPROP,
        [E_X, E_TYPE, E_C2],
        [true, true], // overstates the unrelated graph
        [[E_X, E_TYPE, E_C1], [E_C1, E_SUBCLASS, E_C2]],
        counts,
        enc,
    );
}

// ---------------------------------------------------------------------------
// derivation_check -- accept + forge-and-verify (sq-rsd3v.2, #2614). Tiny
// (K=1, N=4, M=4) monomorphisation. Schema encodings are arbitrary distinct
// nonzero constants (the relation only cares about equality structure).
// ---------------------------------------------------------------------------

global D_TYPE: Field = 31; // rdf:type
global D_SUBCLASS: Field = 32; // rdfs:subClassOf
global D_SUBPROP: Field = 33; // rdfs:subPropertyOf
global D_DOMAIN: Field = 34; // rdfs:domain
global D_RANGE: Field = 35; // rdfs:range
global D_X: Field = 101; // an instance
global D_C1: Field = 201; // a class
global D_C2: Field = 202; // a superclass
global D_GHOST: Field = 777; // a fabricated class NOT in the committed graph

fn d_schema() -> [Field; 5] {
    [D_TYPE, D_SUBCLASS, D_SUBPROP, D_DOMAIN, D_RANGE]
}

// The committed base graph: { (X type C1), (C1 subClassOf C2) } in an N=4
// bucket. Returns (enc, counts, commitments). The rdfs9 firing over these two
// facts entails (X type C2) -- WITHOUT disclosing the two base facts.
fn d_graph() -> ([[[Field; 3]; 4]; 1], [u32; 1], [Field; 1]) {
    let enc: [[[Field; 3]; 4]; 1] = [[
        [D_X, D_TYPE, D_C1],
        [D_C1, D_SUBCLASS, D_C2],
        [0, 0, 0],
        [0, 0, 0],
    ]];
    let counts: [u32; 1] = [2];
    let mut leaves: [Field; 4] = [0; 4];
    for i in 0..4 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, 2)];
    (enc, counts, commitments)
}

// ACCEPT: the honest DAG -- two anchored leaves feeding one rdfs9 firing whose
// conclusion is the disclosed derived row (X type C2). The base facts are
// HIDDEN (private `enc`/`prem`/`concl`); only `root` is public.
#[test]
fn derivation_accepts_grounded_rdfs9() {
    let (enc, counts, commitments) = d_graph();
    let rule: [u32; 4] = [RULE_LEAF, RULE_LEAF, RULE_RDFS9, RULE_LEAF];
    let concl: [[Field; 3]; 4] = [
        [D_X, D_TYPE, D_C1], // leaf, anchored to the graph
        [D_C1, D_SUBCLASS, D_C2], // leaf, anchored to the graph
        [D_X, D_TYPE, D_C2], // rdfs9 firing (the derived row)
        [0, 0, 0], // inactive padding
    ];
    let prem: [[[Field; 3]; 2]; 4] = [
        [[0, 0, 0], [0, 0, 0]],
        [[0, 0, 0], [0, 0, 0]],
        [[D_X, D_TYPE, D_C1], [D_C1, D_SUBCLASS, D_C2]], // chain to nodes 0,1
        [[0, 0, 0], [0, 0, 0]],
    ];
    derivation_check::<1, 4, 4>(
        commitments, d_schema(), [D_X, D_TYPE, D_C2], counts, enc, 3, rule, prem, concl,
    );
}

// FORGE-AND-VERIFY (the core soundness regression): an rdfs9 firing whose
// antecedents are about a FABRICATED class C_GHOST that is neither in the
// committed graph nor derived by any earlier node. The rule SHAPE is satisfied
// (so shape alone cannot catch it), but the premises are ungrounded -- there is
// no anchored leaf and no earlier conclusion for them -- so the proof MUST fail
// closed. This is the "ungrounded/forged antecedent must fail-closed" property.
#[test(should_fail_with = "derivation premise not grounded")]
fn derivation_rejects_ungrounded_antecedent() {
    let (enc, counts, commitments) = d_graph();
    // node0: a genuine anchored leaf; node1: the forged rdfs9 over C_GHOST.
    let rule: [u32; 4] = [RULE_LEAF, RULE_RDFS9, RULE_LEAF, RULE_LEAF];
    let concl: [[Field; 3]; 4] = [
        [D_C1, D_SUBCLASS, D_C2], // leaf, anchored (real)
        [D_X, D_TYPE, D_C2], // forged rdfs9 conclusion (still claims X type C2)
        [0, 0, 0],
        [0, 0, 0],
    ];
    let prem: [[[Field; 3]; 2]; 4] = [
        [[0, 0, 0], [0, 0, 0]],
        // shape-valid rdfs9 over a GHOST class: (X type GHOST),(GHOST sCO C2).
        // Neither premise is an earlier conclusion nor a committed fact.
        [[D_X, D_TYPE, D_GHOST], [D_GHOST, D_SUBCLASS, D_C2]],
        [[0, 0, 0], [0, 0, 0]],
        [[0, 0, 0], [0, 0, 0]],
    ];
    derivation_check::<1, 4, 4>(
        commitments, d_schema(), [D_X, D_TYPE, D_C2], counts, enc, 2, rule, prem, concl,
    );
}

// FORGE variant: a LEAF claiming a base fact that is NOT in the committed graph
// (a fabricated antecedent smuggled in as an "asserted" leaf) must fail the
// membership anchor -- the privacy upgrade's soundness anchor (design step 3).
#[test(should_fail_with = "derivation leaf not present in committed graph")]
fn derivation_rejects_leaf_absent_from_graph() {
    let (enc, counts, commitments) = d_graph();
    let rule: [u32; 4] = [RULE_LEAF, RULE_LEAF, RULE_RDFS9, RULE_LEAF];
    let concl: [[Field; 3]; 4] = [
        [D_X, D_TYPE, D_GHOST], // leaf claiming a fact NOT in the graph
        [D_GHOST, D_SUBCLASS, D_C2], // leaf claiming a fact NOT in the graph
        [D_X, D_TYPE, D_C2],
        [0, 0, 0],
    ];
    let prem: [[[Field; 3]; 2]; 4] = [
        [[0, 0, 0], [0, 0, 0]],
        [[0, 0, 0], [0, 0, 0]],
        [[D_X, D_TYPE, D_GHOST], [D_GHOST, D_SUBCLASS, D_C2]],
        [[0, 0, 0], [0, 0, 0]],
    ];
    derivation_check::<1, 4, 4>(
        commitments, d_schema(), [D_X, D_TYPE, D_C2], counts, enc, 3, rule, prem, concl,
    );
}

// REJECT: the disclosed derived row must bind to the DAG's terminal conclusion.
// A prover who proves a valid derivation of (X type C2) cannot present it as a
// proof of a DIFFERENT row.
#[test(should_fail_with = "derivation root not bound to the disclosed derived row")]
fn derivation_rejects_root_mismatch() {
    let (enc, counts, commitments) = d_graph();
    let rule: [u32; 4] = [RULE_LEAF, RULE_LEAF, RULE_RDFS9, RULE_LEAF];
    let concl: [[Field; 3]; 4] = [
        [D_X, D_TYPE, D_C1],
        [D_C1, D_SUBCLASS, D_C2],
        [D_X, D_TYPE, D_C2],
        [0, 0, 0],
    ];
    let prem: [[[Field; 3]; 2]; 4] = [
        [[0, 0, 0], [0, 0, 0]],
        [[0, 0, 0], [0, 0, 0]],
        [[D_X, D_TYPE, D_C1], [D_C1, D_SUBCLASS, D_C2]],
        [[0, 0, 0], [0, 0, 0]],
    ];
    // The proof derives (X type C2) but claims the row (X type C1).
    derivation_check::<1, 4, 4>(
        commitments, d_schema(), [D_X, D_TYPE, D_C1], counts, enc, 3, rule, prem, concl,
    );
}

// ---------------------------------------------------------------------------
// sameas -- union-find canonicalisation + representative-membership
// (sq-rsd3v.6, #3265). Tiny (K=1, N=8, U=4) monomorphisation. Term encodings
// are arbitrary distinct nonzero constants (the relation only cares about
// equality structure), exactly as the derivation_check tests above.
// ---------------------------------------------------------------------------

global SA_SAMEAS: Field = 41; // owl:sameAs (the PUBLIC vocabulary encoding)
global SA_P: Field = 51; // <worksAt>
global SA_O: Field = 61; // <acme>
global SA_A: Field = 101; // <alice>
global SA_B: Field = 102; // <alice-2>, committed sameAs <alice>
global SA_C: Field = 103; // <alice-3>, committed sameAs <alice-2>
global SA_GHOST: Field = 999; // a term with NO committed sameAs edge

// The committed base graph, in an N=8 bucket:
//   { (A P O), (A sameAs B), (B sameAs C) }
// so A ~ B ~ C, and (C P O) is entailed under OWL-RL eq-rep-s WITHOUT
// disclosing (A P O) or either equality.
fn sa_graph() -> ([[[Field; 3]; 8]; 1], [u32; 1], [Field; 1]) {
    let enc: [[[Field; 3]; 8]; 1] = [[
        [SA_A, SA_P, SA_O],
        [SA_A, SA_SAMEAS, SA_B],
        [SA_B, SA_SAMEAS, SA_C],
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
    ]];
    let counts: [u32; 1] = [3];
    let mut leaves: [Field; 8] = [0; 8];
    for i in 0..8 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, 3)];
    (enc, counts, commitments)
}

// The HONEST spanning forest of the committed sameAs edges, rooted at A:
//   entry 0: A -> A (root)
//   entry 1: B -> A, parent 0, justified by (A sameAs B)
//   entry 2: C -> A, parent 1, justified by (B sameAs C)
// Entry 3 is inactive padding (entry_count = 3).
fn sa_table() -> ([Field; 4], [Field; 4], [u32; 4], u32) {
    ([SA_A, SA_B, SA_C, 0], [SA_A, SA_A, SA_A, 0], [0, 0, 1, 3], 3)
}

// canon_of reads the representative for a table member and is the IDENTITY on
// a term with no entry (OWL-RL eq-ref: every term is its own class). Padding
// entries are invisible.
#[test]
fn canon_of_maps_members_and_fixes_strangers() {
    let (mem, rep, _parent, n) = sa_table();
    assert(canon_of(mem, rep, n, SA_A) == SA_A);
    assert(canon_of(mem, rep, n, SA_B) == SA_A);
    assert(canon_of(mem, rep, n, SA_C) == SA_A);
    // Not a member: its own representative.
    assert(canon_of(mem, rep, n, SA_P) == SA_P);
    assert(canon_of(mem, rep, n, SA_GHOST) == SA_GHOST);
    // Idempotence (module-doc L3).
    assert(canon_of(mem, rep, n, canon_of(mem, rep, n, SA_C)) == SA_A);
}

// ACCEPT: (C P O) is entailed from the hidden base triple (A P O) via the
// two-edge equality chain A ~ B ~ C. Only `row` is public; the base triple,
// its slot, and the whole equality chain stay private.
#[test]
fn sameas_accepts_row_rewritten_across_the_equality_chain() {
    let (enc, counts, commitments) = sa_graph();
    let (mem, rep, parent, n) = sa_table();
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_C, SA_P, SA_O], counts, enc, n, mem, rep, parent, 0, 0,
    );
}

// FORGE (the core soundness regression -- the OVER-MERGE): a prover adds a
// table entry merging a fabricated term into A's class WITHOUT any committed
// `A owl:sameAs GHOST` triple, then claims A's facts for it. Constraints 3a/3b
// are all satisfied (the parent is earlier, the representative agrees), so
// shape alone cannot catch it -- only the committed-edge anchor can.
#[test(should_fail_with = "canon merge not justified by a committed owl:sameAs triple")]
fn sameas_rejects_unjustified_merge() {
    let (enc, counts, commitments) = sa_graph();
    let mem: [Field; 4] = [SA_A, SA_B, SA_C, SA_GHOST];
    let rep: [Field; 4] = [SA_A, SA_A, SA_A, SA_A];
    let parent: [u32; 4] = [0, 0, 1, 0];
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_GHOST, SA_P, SA_O], counts, enc, 4, mem, rep, parent, 0, 0,
    );
}

// FORGE: the representative must be constant along a tree edge. A non-root
// that keeps its own representative would let a prover split a class (and, in
// the general table, launder a merge through an intermediate).
#[test(should_fail_with = "canon entry disagrees with its parent's representative")]
fn sameas_rejects_representative_break() {
    let (enc, counts, commitments) = sa_graph();
    let mem: [Field; 4] = [SA_A, SA_B, SA_C, 0];
    // Entry 1 is a non-root but claims itself as representative.
    let rep: [Field; 4] = [SA_A, SA_B, SA_A, 0];
    let parent: [u32; 4] = [0, 0, 1, 3];
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_C, SA_P, SA_O], counts, enc, 3, mem, rep, parent, 0, 0,
    );
}

// FORGE: a forward-referencing parent. Without the strict-backwards rule a
// prover could build a CYCLE of mutually-justifying entries with no root, and
// the induction that makes the forest well-founded would have no base case.
#[test(should_fail_with = "canon parent is not an earlier entry")]
fn sameas_rejects_forward_parent() {
    let (enc, counts, commitments) = sa_graph();
    let mem: [Field; 4] = [SA_A, SA_B, SA_C, 0];
    let rep: [Field; 4] = [SA_A, SA_A, SA_A, 0];
    // Entry 1 points FORWARD at entry 2.
    let parent: [u32; 4] = [0, 2, 1, 3];
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_C, SA_P, SA_O], counts, enc, 3, mem, rep, parent, 0, 0,
    );
}

// FORGE: duplicate members would make `canon_of` multi-valued (the prover picks
// whichever representative suits the row), breaking idempotence -- L3.
#[test(should_fail_with = "duplicate member in canon table")]
fn sameas_rejects_duplicate_member() {
    let (enc, counts, commitments) = sa_graph();
    // Entry 2 re-enters B (already at index 1). Its edge is separately
    // justified by the committed (A sameAs B), so ONLY uniqueness catches it.
    let mem: [Field; 4] = [SA_A, SA_B, SA_B, 0];
    let rep: [Field; 4] = [SA_A, SA_A, SA_A, 0];
    let parent: [u32; 4] = [0, 0, 0, 3];
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_B, SA_P, SA_O], counts, enc, 3, mem, rep, parent, 0, 0,
    );
}

// FORGE: an HONEST table cannot license a row outside the proven class. GHOST
// has no entry, so canon(GHOST) = GHOST != A = canon(A) -- the representative-
// membership step is what refuses it.
#[test(should_fail_with = "disclosed row is not the committed triple up to owl:sameAs")]
fn sameas_rejects_row_outside_the_class() {
    let (enc, counts, commitments) = sa_graph();
    let (mem, rep, parent, n) = sa_table();
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_GHOST, SA_P, SA_O], counts, enc, n, mem, rep, parent, 0, 0,
    );
}

// FORGE: the base triple must be an ACTIVE slot. Slot 5 is padding, so a
// fabricated base fact smuggled into the padding region has no witness.
#[test(should_fail_with = "witnessed base triple is not an active committed slot")]
fn sameas_rejects_base_triple_in_padding() {
    let (enc, counts, commitments) = sa_graph();
    let (mem, rep, parent, n) = sa_table();
    sameas_entails_row::<1, 8, 4>(
        commitments, SA_SAMEAS, [SA_C, SA_P, SA_O], counts, enc, n, mem, rep, parent, 0, 5,
    );
}

// ---------------------------------------------------------------------------
// n3_derivation_check -- witnessed-rule-shape N3 derivation over a
// DATASET-SUPPLIED rule graph, plus the math: builtin gadget whitelist
// (sq-rsd3v.3, #2946). Tiny (K=1, N=4, R=2, P=2, C=1, M=4, V=3, MD=2)
// monomorphisation. IRI encodings are arbitrary distinct nonzero constants (the
// relation only cares about equality structure), exactly as the
// derivation_check / sameas tests above; LITERAL operand encodings are DERIVED
// through `signed_int_encoding` because the builtin gadgets bind them to their
// canonical xsd:integer token.
// ---------------------------------------------------------------------------

global N3_WORKS_AT: Field = 71;
global N3_LOCATED_IN: Field = 72;
global N3_BASED_IN: Field = 73;
global N3_SALARY: Field = 74;
global N3_HIGH_EARNER: Field = 75;
global N3_YES: Field = 76;
global N3_X: Field = 111; // a well-paid person based in NL
global N3_Y: Field = 112; // a low-paid person (the comparison's negative case)
global N3_ACME: Field = 113;
global N3_NL: Field = 114;
global N3_GHOST: Field = 777; // fabricated: in neither the graph nor the rules

// Canonical MD=2 padding digits for an operand slot the gadget does not read.
// "10" rather than "00" because a multi-digit magnitude may not lead with '0'
// (the canonicality asserts inside `signed_int_encoding` run unmasked).
fn n3_pad() -> [u8; 2] {
    [49, 48]
}

// The committed base graph:
//   { (X worksAt ACME), (ACME locatedIn NL), (X salary "50"), (Y salary "10") }
fn n3_graph(l50: Field, l10: Field) -> ([[[Field; 3]; 4]; 1], [u32; 1], [Field; 1]) {
    let enc: [[[Field; 3]; 4]; 1] = [[
        [N3_X, N3_WORKS_AT, N3_ACME],
        [N3_ACME, N3_LOCATED_IN, N3_NL],
        [N3_X, N3_SALARY, l50],
        [N3_Y, N3_SALARY, l10],
    ]];
    let counts: [u32; 1] = [4];
    let mut leaves: [Field; 4] = [0; 4];
    for i in 0..4 {
        leaves[i] = h3(enc[0][i][0], enc[0][i][1], enc[0][i][2]);
    }
    let commitments: [Field; 1] = [commit_fold(leaves, 4)];
    (enc, counts, commitments)
}

// The DATASET-SUPPLIED rule graph (the thing an N3 dataset ships and its author
// signs), as witnessed shapes:
//   rule 0: { ?x worksAt ?c . ?c locatedIn ?r }        => { ?x basedIn ?r }
//   rule 1: { ?x salary ?s . ?s math:greaterThan 20 }  => { ?x highEarner yes }
// Variable ids: ?x = 0, ?c/?s = 1, ?r = 2.
//
// Rule 1's comparison atom shows the slot-1 repurposing: a builtin's predicate
// is carried by the TAG, so slot 1 holds no pattern (it is only an operand for
// binary arithmetic). Slot 0 is `?s`, slot 2 the constant bound.
fn n3_rules(
    l20: Field,
) -> ([u32; 2], [u32; 2], [[u32; 2]; 2], [[[u32; 3]; 2]; 2], [[[Field; 3]; 2]; 2], [[[u32; 3]; 2]; 2], [[[u32; 3]; 1]; 2], [[[Field; 3]; 1]; 2], [[[u32; 3]; 1]; 2]) {
    let prem_len: [u32; 2] = [2, 2];
    let concl_len: [u32; 2] = [1, 1];
    let prem_builtin: [[u32; 2]; 2] =
        [[BUILTIN_NONE, BUILTIN_NONE], [BUILTIN_NONE, BUILTIN_GREATER_THAN]];
    let prem_kind: [[[u32; 3]; 2]; 2] = [
        [[SLOT_VAR, SLOT_CONST, SLOT_VAR], [SLOT_VAR, SLOT_CONST, SLOT_VAR]],
        [[SLOT_VAR, SLOT_CONST, SLOT_VAR], [SLOT_VAR, SLOT_CONST, SLOT_CONST]],
    ];
    let prem_konst: [[[Field; 3]; 2]; 2] = [
        [[0, N3_WORKS_AT, 0], [0, N3_LOCATED_IN, 0]],
        [[0, N3_SALARY, 0], [0, 0, l20]],
    ];
    let prem_var: [[[u32; 3]; 2]; 2] = [[[0, 0, 1], [1, 0, 2]], [[0, 0, 1], [1, 0, 0]]];
    let concl_kind: [[[u32; 3]; 1]; 2] =
        [[[SLOT_VAR, SLOT_CONST, SLOT_VAR]], [[SLOT_VAR, SLOT_CONST, SLOT_CONST]]];
    let concl_konst: [[[Field; 3]; 1]; 2] =
        [[[0, N3_BASED_IN, 0]], [[0, N3_HIGH_EARNER, N3_YES]]];
    let concl_var: [[[u32; 3]; 1]; 2] = [[[0, 0, 2]], [[0, 0, 0]]];
    (
        prem_len, concl_len, prem_builtin, prem_kind, prem_konst, prem_var, concl_kind,
        concl_konst, concl_var,
    )
}

// The rule author as ISSUER: the root the rule graph is committed under. The
// prover must reproduce every committed shape bit-for-bit to match it.
fn n3_rules_root(
    prem_len: [u32; 2],
    concl_len: [u32; 2],
    prem_builtin: [[u32; 2]; 2],
    prem_kind: [[[u32; 3]; 2]; 2],
    prem_konst: [[[Field; 3]; 2]; 2],
    prem_var: [[[u32; 3]; 2]; 2],
    concl_kind: [[[u32; 3]; 1]; 2],
    concl_konst: [[[Field; 3]; 1]; 2],
    concl_var: [[[u32; 3]; 1]; 2],
) -> Field {
    let mut leaves: [Field; 2] = [0; 2];
    for r in 0..2 {
        leaves[r] = rule_leaf(
            prem_len[r],
            concl_len[r],
            prem_builtin[r],
            prem_kind[r],
            prem_konst[r],
            prem_var[r],
            concl_kind[r],
            concl_konst[r],
            concl_var[r],
        );
    }
    commit_fold(leaves, 2)
}

// Operand witnesses for nodes whose premise atoms are all JOIN atoms: the
// gadget is disabled, so only the (unmasked) canonicality checks run.
fn n3_pad_ops() -> ([[[bool; 3]; 2]; 4], [[[[u8; 2]; 3]; 2]; 4]) {
    ([[[false; 3]; 2]; 4], [[[n3_pad(); 3]; 2]; 4])
}

// ACCEPT: the honest DAG for the pure-join rule 0 -- two anchored leaves feeding
// one firing whose conclusion is the disclosed derived row (X basedIn NL). The
// base facts AND the rule text are HIDDEN (all private); only the commitments,
// the rule-set root and `root` are public.
#[test]
fn n3_accepts_a_committed_join_rule() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]); // "50"
    let (l10, _, _) = signed_int_encoding(false, [49, 48]); // "10"
    let (l20, _, _) = signed_int_encoding(false, [50, 48]); // "20"
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    let node_is_leaf: [bool; 4] = [true, true, false, false];
    let node_rule: [u32; 4] = [0, 0, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [0, 0, 0], [N3_X, N3_ACME, N3_NL], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME], // leaf, anchored to the graph
        [N3_ACME, N3_LOCATED_IN, N3_NL], // leaf, anchored to the graph
        [N3_X, N3_BASED_IN, N3_NL], // rule 0 firing (the derived row)
        [0, 0, 0], // inactive padding
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_NL],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        3,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// ACCEPT: the whitelisted `math:greaterThan` gadget. Rule 1 fires over the
// hidden (X salary "50") with the committed bound "20"; the operand encodings
// are bound to their canonical xsd:integer tokens and the verdict is
// `signed_verdict` reused verbatim.
#[test]
fn n3_accepts_a_whitelisted_math_comparison() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits_pad) = n3_pad_ops();
    let mut op_digits = op_digits_pad;
    // Node 1, premise atom 1 (the comparison): operand slot 0 is ?s = "50",
    // slot 2 the committed bound "20". Slot 1 is unread for a comparison.
    op_digits[1][1][0] = [53, 48];
    op_digits[1][1][2] = [50, 48];

    let node_is_leaf: [bool; 4] = [true, false, false, false];
    let node_rule: [u32; 4] = [0, 1, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] = [[0, 0, 0], [N3_X, l50, 0], [0, 0, 0], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_SALARY, l50], // leaf, anchored to the graph
        [N3_X, N3_HIGH_EARNER, N3_YES], // rule 1 firing (the derived row)
        [0, 0, 0],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_HIGH_EARNER, N3_YES],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        2,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE-AND-VERIFY (the headline soundness regression for THIS bead): the
// prover INVENTS a rule. Everything else is honest -- the graph is real, the
// leaves are anchored, the firing is a perfectly well-formed instance of the
// rule it cites, and the rule is even SAFE -- but the shape it cites is not the
// shape the rule author committed, so only the rule-set root can catch it. This
// is what "the proof says WHOSE rules" buys (design sec 4.1 move 1).
#[test(should_fail_with = "rule-set commitment mismatch")]
fn n3_rejects_an_uncommitted_rule_shape() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    // The root the rule AUTHOR published, over the honest shapes.
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    // The prover rewrites rule 0's conclusion to `{ ?x basedIn GHOST }` so a
    // single honest firing "derives" a row about a place nobody asserted.
    let mut ck2 = ck;
    let mut cko2 = cko;
    ck2[0][0] = [SLOT_VAR, SLOT_CONST, SLOT_CONST];
    cko2[0][0] = [0, N3_BASED_IN, N3_GHOST];

    let node_is_leaf: [bool; 4] = [true, true, false, false];
    let node_rule: [u32; 4] = [0, 0, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [0, 0, 0], [N3_X, N3_ACME, N3_NL], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME],
        [N3_ACME, N3_LOCATED_IN, N3_NL],
        [N3_X, N3_BASED_IN, N3_GHOST],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_GHOST],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck2,
        cko2,
        cv,
        3,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE: an UNSAFE rule -- one whose conclusion carries a variable the premise
// never binds -- must be REJECTED, not approximated (design sec 4.3). Note the
// rule set here is COMMITTED honestly (the root is recomputed over the unsafe
// shape), i.e. the dataset really does ship this rule; it is still refused,
// because the substitution is a PRIVATE witness, so an unbound conclusion
// variable lets the prover CHOOSE the derived row (here: GHOST). This is the
// safety condition doing real soundness work, not style.
#[test(should_fail_with = "rule is unsafe: a conclusion variable is not bound in the premise")]
fn n3_rejects_unsafe_rule_with_unbound_conclusion_variable() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl_honest, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let mut pl = pl_honest;
    // Rule 0 loses its second premise atom, so ?r (var 2) is no longer bound --
    // but the conclusion `{ ?x basedIn ?r }` still mentions it.
    pl[0] = 1;
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    let node_is_leaf: [bool; 4] = [true, false, false, false];
    let node_rule: [u32; 4] = [0, 0, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    // ?r is freely chosen as GHOST -- exactly the forgery safety forbids.
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [N3_X, N3_ACME, N3_GHOST], [0, 0, 0], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME],
        [N3_X, N3_BASED_IN, N3_GHOST],
        [0, 0, 0],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_GHOST],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        2,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE: an UNGROUNDED antecedent. The rule is committed and honest and the
// firing satisfies its shape, but the substitution names a company nobody
// asserted, so neither premise is an earlier conclusion nor an anchored leaf.
#[test(should_fail_with = "n3 premise not grounded")]
fn n3_rejects_ungrounded_premise() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    let node_is_leaf: [bool; 4] = [true, false, false, false];
    let node_rule: [u32; 4] = [0, 0, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [N3_X, N3_GHOST, N3_NL], [0, 0, 0], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME], // a genuine anchored leaf
        [N3_X, N3_BASED_IN, N3_NL], // fired over (X worksAt GHOST),(GHOST locatedIn NL)
        [0, 0, 0],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_NL],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        2,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE: a LEAF claiming a base fact that is not in the committed graph -- a
// fabricated antecedent smuggled in as an "asserted" fact. This is the
// hidden-antecedent soundness anchor: because leaves are PRIVATE, membership in
// C(G) is the only thing standing behind them.
#[test(should_fail_with = "n3 derivation leaf not present in committed graph")]
fn n3_rejects_leaf_absent_from_graph() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    let node_is_leaf: [bool; 4] = [true, true, false, false];
    let node_rule: [u32; 4] = [0, 0, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [0, 0, 0], [N3_X, N3_ACME, N3_GHOST], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME],
        [N3_ACME, N3_LOCATED_IN, N3_GHOST], // NOT a committed triple
        [N3_X, N3_BASED_IN, N3_GHOST],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_GHOST],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        3,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE: the whitelisted comparison gadget must actually HOLD. Y's committed
// salary is "10", which is NOT greater than the rule's bound "20", so rule 1
// cannot fire for Y even though every operand binding is honest.
#[test(should_fail_with = "math: comparison builtin does not hold")]
fn n3_rejects_a_false_math_comparison() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits_pad) = n3_pad_ops();
    let mut op_digits = op_digits_pad;
    op_digits[1][1][0] = [49, 48]; // "10" -- Y's real, committed salary
    op_digits[1][1][2] = [50, 48]; // "20" -- the rule's committed bound

    let node_is_leaf: [bool; 4] = [true, false, false, false];
    let node_rule: [u32; 4] = [0, 1, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] = [[0, 0, 0], [N3_Y, l10, 0], [0, 0, 0], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_Y, N3_SALARY, l10],
        [N3_Y, N3_HIGH_EARNER, N3_YES],
        [0, 0, 0],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_Y, N3_HIGH_EARNER, N3_YES],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        2,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE: lying about a builtin OPERAND's value. Y's salary really is "10", but
// the prover witnesses the digits "50" to make the comparison come out true.
// The operand's term ENCODING is what the graph committed, so the canonical
// token rebuild catches the substitution.
#[test(should_fail_with = "builtin operand encoding mismatch")]
fn n3_rejects_a_relabelled_builtin_operand() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits_pad) = n3_pad_ops();
    let mut op_digits = op_digits_pad;
    op_digits[1][1][0] = [53, 48]; // claims "50" while ?s is bound to "10"
    op_digits[1][1][2] = [50, 48];

    let node_is_leaf: [bool; 4] = [true, false, false, false];
    let node_rule: [u32; 4] = [0, 1, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] = [[0, 0, 0], [N3_Y, l10, 0], [0, 0, 0], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_Y, N3_SALARY, l10],
        [N3_Y, N3_HIGH_EARNER, N3_YES],
        [0, 0, 0],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_Y, N3_HIGH_EARNER, N3_YES],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        2,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// FORGE: citing a rule index outside the committed set (here `rule_count = 2`,
// so index 5 names no rule at all). Fails closed rather than defaulting to some
// rule.
#[test(should_fail_with = "node cites a rule outside the committed rule set")]
fn n3_rejects_a_rule_index_outside_the_committed_set() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    let node_is_leaf: [bool; 4] = [true, false, false, false];
    let node_rule: [u32; 4] = [0, 5, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [N3_X, N3_ACME, N3_NL], [0, 0, 0], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME],
        [N3_X, N3_BASED_IN, N3_NL],
        [0, 0, 0],
        [0, 0, 0],
    ];
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_NL],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        2,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}

// REJECT: the disclosed derived row must bind to the DAG's terminal conclusion.
// A prover who proves a valid derivation of (X basedIn NL) cannot present it as
// a proof of a DIFFERENT row.
#[test(should_fail_with = "n3 derivation root not bound to the disclosed derived row")]
fn n3_rejects_root_mismatch() {
    let (l50, _, _) = signed_int_encoding(false, [53, 48]);
    let (l10, _, _) = signed_int_encoding(false, [49, 48]);
    let (l20, _, _) = signed_int_encoding(false, [50, 48]);
    let (enc, counts, commitments) = n3_graph(l50, l10);
    let (pl, cl, pb, pk, pko, pv, ck, cko, cv) = n3_rules(l20);
    let rules_root = n3_rules_root(pl, cl, pb, pk, pko, pv, ck, cko, cv);
    let (op_neg, op_digits) = n3_pad_ops();

    let node_is_leaf: [bool; 4] = [true, true, false, false];
    let node_rule: [u32; 4] = [0, 0, 0, 0];
    let node_concl: [u32; 4] = [0, 0, 0, 0];
    let node_subst: [[Field; 3]; 4] =
        [[0, 0, 0], [0, 0, 0], [N3_X, N3_ACME, N3_NL], [0, 0, 0]];
    let node_out: [[Field; 3]; 4] = [
        [N3_X, N3_WORKS_AT, N3_ACME],
        [N3_ACME, N3_LOCATED_IN, N3_NL],
        [N3_X, N3_BASED_IN, N3_NL],
        [0, 0, 0],
    ];
    // The proof derives (X basedIn NL) but claims (X basedIn ACME).
    n3_derivation_check::<1, 4, 2, 2, 1, 4, 3, 2>(
        commitments,
        rules_root,
        [N3_X, N3_BASED_IN, N3_ACME],
        counts,
        enc,
        2,
        pl,
        cl,
        pb,
        pk,
        pko,
        pv,
        ck,
        cko,
        cv,
        3,
        node_is_leaf,
        node_rule,
        node_concl,
        node_subst,
        node_out,
        op_neg,
        op_digits,
    );
}
