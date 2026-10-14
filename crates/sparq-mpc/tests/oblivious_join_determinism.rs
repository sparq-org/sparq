// sq-bif.15 — glue-level DETERMINISM + CARDINALITY tests for the
// oblivious set-returning hidden-join output path (`crate::oblivious_join`).
// TEST-COVERAGE bead: no production logic changed. These exercise the REAL
// public path (`oblivious_set_output` / `oblivious_join_output` /
// `oblivious_set_output_hidden_keys`) over the seedable simulation RNG and pin
// two glue invariants the in-crate tests do not assert together:
//
//   1. DETERMINISM — the same (n, seed) over the same candidate set yields a
//      BIT-IDENTICAL `Vec<OutputSlot>` (including the oblivious-shuffled order).
//   2. CARDINALITY — no phantom or lost rows: the revealed slot count is exactly
//      the public bound `B` for ANY true match count (L1), and the real
//      (non-dummy) row multiset equals the plaintext match set, regardless of
//      which permutation the shuffle drew.
//
// This file needs the deterministic, seedable masking RNG
// (`ShamirBackend::new_seeded`), which is gated behind `insecure-test-rng`, so
// the WHOLE file is feature-gated. In the default feature state it compiles to
// nothing (an empty crate) and the gate is honest: a production build cannot
// construct a predictable masking RNG. 🤖 SPARQ agent.
//
// HONESTY: sparq-mpc is honest-majority, SEMI-HONEST only (sq-qhy4 external
// sign-off pending). Determinism HERE is a property of the seeded TEST RNG, NOT
// a security property — the real protocol draws OS-seeded ChaCha20 and is NOT
// reproducible. Nothing here claims soundness, malicious-security, or audit.
#![cfg(feature = "insecure-test-rng")]

use std::collections::HashSet;

use oxrdf::{Literal, Term, Variable};
use sparq_mpc::{
    oblivious_join_output, oblivious_set_output, oblivious_set_output_hidden_keys, Candidate, Fp,
    HolderId, MatchBit, OutputSlot, ShamirBackend,
};

fn lit(s: &str) -> Option<Term> {
    Some(Term::Literal(Literal::new_simple_literal(s)))
}

/// Candidates in the disclosed-key (public match-bit) regime — the SOUND-today
/// path the determinism/cardinality glue tests drive.
fn public_candidates(rows: &[(bool, Vec<Option<Term>>)]) -> Vec<Candidate> {
    rows.iter()
        .map(|(m, p)| Candidate {
            payload: p.clone(),
            matched: MatchBit::Public(*m),
        })
        .collect()
}

/// The real (non-dummy) rows of an output, as an order-independent sorted
/// multiset of debug strings — so a check is invariant to the shuffle's order.
fn real_multiset(slots: &[OutputSlot]) -> Vec<String> {
    let mut m: Vec<String> = slots
        .iter()
        .filter_map(|s| match s {
            OutputSlot::Row(r) => Some(format!("{r:?}")),
            OutputSlot::Dummy => None,
        })
        .collect();
    m.sort();
    m
}

/// The Row/Dummy classification pattern (the transcript-visible shape) — used to
/// confirm the shuffle genuinely moves match positions across seeds, so the
/// determinism assertion is NOT vacuously true.
fn classification(slots: &[OutputSlot]) -> Vec<bool> {
    slots
        .iter()
        .map(|s| matches!(s, OutputSlot::Row(_)))
        .collect()
}

// =============================================================================
// 1. Determinism — fixed (n, seed) over fixed inputs -> bit-identical output.
// =============================================================================

#[test]
fn same_seed_yields_bit_identical_output() {
    // Distinct matched payloads at fixed input positions; the shuffle reorders
    // them, but a SECOND run at the same seed must reproduce the EXACT slot
    // vector (same Row/Dummy positions AND same payloads in each Row).
    let cands = public_candidates(&[
        (true, vec![lit("Alice"), lit("Leeds")]),
        (false, vec![lit("Bob"), lit("York")]),
        (true, vec![lit("Carol"), lit("Hull")]),
        (false, vec![lit("Dan"), lit("Ely")]),
        (true, vec![lit("Eve"), lit("Derby")]),
    ]);

    let run = || {
        let backend = ShamirBackend::new_seeded(3, 0x00DE_7E45_u64).unwrap();
        oblivious_set_output(&backend, &cands, 2, 8).unwrap().0
    };
    let a = run();
    let b = run();
    assert_eq!(
        a, b,
        "same (n, seed) must produce a bit-identical OutputSlot vector \
         (including the oblivious-shuffled order)"
    );
}

#[test]
fn determinism_is_per_seed_not_per_input_position() {
    // Non-vacuity guard for the determinism test above: DIFFERENT seeds must
    // produce DIFFERENT shuffle orders for at least some seeds. If the output
    // were not actually shuffled (match positions fixed by input position), the
    // classification pattern would be identical for every seed and this would
    // fail — proving the determinism test is pinning a real reordering, not a
    // no-op identity.
    let cands = public_candidates(&[
        (true, vec![lit("p0")]),
        (false, vec![lit("p1")]),
        (true, vec![lit("p2")]),
        (false, vec![lit("p3")]),
        (true, vec![lit("p4")]),
    ]);
    let mut patterns: HashSet<Vec<bool>> = HashSet::new();
    for seed in 0..32u64 {
        let backend = ShamirBackend::new_seeded(3, 0x9000 + seed).unwrap();
        let (slots, _) = oblivious_set_output(&backend, &cands, 1, 5).unwrap();
        patterns.insert(classification(&slots));
        // Cardinality holds on every run regardless of order (see below).
        assert_eq!(real_multiset(&slots).len(), 3);
    }
    assert!(
        patterns.len() > 1,
        "the output is not actually shuffled — match positions never moved \
         across 32 seeds, so the determinism test would be vacuous"
    );
}

#[test]
fn hidden_key_path_is_deterministic_under_a_fixed_seed() {
    // The hidden-key all-pairs entry point (secret-shared match bits, never
    // opened) must ALSO be reproducible under a fixed seed — same keys, same
    // (n, seed) -> identical revealed slots.
    let left = [Fp::new(1), Fp::new(2), Fp::new(3)];
    let right = [Fp::new(3), Fp::new(5), Fp::new(1)];
    let run = || {
        let backend = ShamirBackend::new_seeded(3, 0xC0FFEE).unwrap();
        oblivious_set_output_hidden_keys(&backend, &left, &right, 9)
            .unwrap()
            .0
    };
    assert_eq!(
        run(),
        run(),
        "hidden-key oblivious output must be reproducible under a fixed seed"
    );
}

// =============================================================================
// 2. Cardinality — no phantom / lost rows at the glue level.
// =============================================================================

#[test]
fn revealed_slot_count_is_the_bound_for_any_true_match_count() {
    // L1 / phantom-protection: the number of REVEALED slots is exactly the
    // public bound `B`, independent of the true match count. Two candidate sets
    // with DIFFERENT true match counts must reveal the SAME number of slots —
    // the parties learn `B`, never the true cardinality.
    let backend = ShamirBackend::new_seeded(3, 0x4242).unwrap();
    let one_match = public_candidates(&[
        (true, vec![lit("a")]),
        (false, vec![lit("b")]),
        (false, vec![lit("c")]),
    ]);
    let three_match = public_candidates(&[
        (true, vec![lit("a")]),
        (true, vec![lit("b")]),
        (true, vec![lit("c")]),
    ]);
    let (s1, _) = oblivious_set_output(&backend, &one_match, 1, 6).unwrap();
    let (s3, _) = oblivious_set_output(&backend, &three_match, 1, 6).unwrap();
    assert_eq!(s1.len(), 6, "revealed slot count must equal the bound B");
    assert_eq!(s3.len(), 6, "revealed slot count must equal the bound B");
    assert_eq!(
        s1.len(),
        s3.len(),
        "revealed slot count must not leak the true match count"
    );
    // Only the recipient who filters dummies sees the true counts differ.
    assert_eq!(
        real_multiset(&s1).len(),
        1,
        "exactly one real match survives"
    );
    assert_eq!(
        real_multiset(&s3).len(),
        3,
        "exactly three real matches survive"
    );
}

#[test]
fn real_rows_are_exactly_the_matched_payloads_no_phantom_no_lost() {
    // The real (non-dummy) multiset must equal EXACTLY the matched candidates'
    // payloads — no phantom row (a non-match leaking through) and no lost row (a
    // true match dropped) — and must be INVARIANT across shuffle permutations.
    let cands = public_candidates(&[
        (true, vec![lit("A"), lit("1")]),
        (false, vec![lit("B"), lit("2")]),
        (true, vec![lit("C"), lit("3")]),
        (false, vec![lit("D"), lit("4")]),
    ]);
    let mut expected = vec![
        format!("{:?}", vec![lit("A"), lit("1")]),
        format!("{:?}", vec![lit("C"), lit("3")]),
    ];
    expected.sort();
    for seed in 0..16u64 {
        let backend = ShamirBackend::new_seeded(5, seed).unwrap();
        let (slots, cost) = oblivious_set_output(&backend, &cands, 2, 4).unwrap();
        assert_eq!(slots.len(), 4, "seed {seed}: exactly B=4 slots");
        assert_eq!(
            real_multiset(&slots),
            expected,
            "seed {seed}: real multiset must be exactly the matched payloads \
             (no phantom, no lost row)"
        );
        // The modelled cost reflects the bound, not the true match count.
        assert_eq!(cost.bound, 4);
        assert_eq!(cost.select_mults, 4);
        assert_eq!(cost.opens, 4);
    }
}

#[test]
fn all_match_and_no_match_cardinality_boundaries() {
    // Boundary cardinalities: B == match count (all real, zero dummies) and
    // zero matches (all dummies) — the cardinality must be exact at both ends.
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();

    let all = public_candidates(&[(true, vec![lit("x")]), (true, vec![lit("y")])]);
    let (slots, _) = oblivious_set_output(&backend, &all, 1, 2).unwrap();
    assert_eq!(real_multiset(&slots).len(), 2, "all matched -> 2 real rows");
    assert!(
        slots.iter().all(|s| matches!(s, OutputSlot::Row(_))),
        "B == match count -> no dummies"
    );

    let none = public_candidates(&[(false, vec![lit("x")]), (false, vec![lit("y")])]);
    let (slots, _) = oblivious_set_output(&backend, &none, 1, 4).unwrap();
    assert_eq!(real_multiset(&slots).len(), 0, "no matches -> 0 real rows");
    assert!(
        slots.iter().all(|s| matches!(s, OutputSlot::Dummy)),
        "no matches -> all dummies"
    );
}

#[test]
fn hidden_key_cardinality_matches_plaintext_all_pairs() {
    // The hidden-key all-pairs join's revealed real multiset must equal the
    // plaintext equi-join (no phantom / lost match), while the revealed slot
    // count stays the bound B (not the true match count). left[0]=1==right[2]=1
    // -> "0-2"; left[2]=3==right[0]=3 -> "2-0": exactly 2 matches over 9 pairs.
    let backend = ShamirBackend::new_seeded(3, 7).unwrap();
    let left = [Fp::new(1), Fp::new(2), Fp::new(3)];
    let right = [Fp::new(3), Fp::new(5), Fp::new(1)];
    let mut expected: Vec<String> = ["0-2", "2-0"]
        .into_iter()
        .map(|s| {
            format!(
                "{:?}",
                vec![Some(Term::Literal(Literal::new_simple_literal(s)))]
            )
        })
        .collect();
    expected.sort();

    let (slots, cost) = oblivious_set_output_hidden_keys(&backend, &left, &right, 9).unwrap();
    assert_eq!(
        slots.len(),
        9,
        "revealed slot count is the bound (9 pairs), not the 2 true matches"
    );
    assert_eq!(cost.bound, 9);
    assert_eq!(
        real_multiset(&slots),
        expected,
        "hidden-key real multiset must equal the plaintext all-pairs equi-join"
    );
}

#[test]
fn partial_result_wrapper_filters_dummies_to_the_matched_rows() {
    // The end-to-end `oblivious_join_output` wrapper must surface ONLY the real
    // matched rows (dummies filtered), federation-attributed, with the right
    // arity — the disclosed cardinality the recipient sees.
    let backend = ShamirBackend::new_seeded(3, 5).unwrap();
    let cands = public_candidates(&[
        (true, vec![lit("Bob"), lit("Leeds")]),
        (false, vec![lit("x"), lit("y")]),
        (true, vec![lit("Carol"), lit("York")]),
    ]);
    let out_vars = vec![
        Variable::new_unchecked("name"),
        Variable::new_unchecked("city"),
    ];
    let (result, cost) = oblivious_join_output(&backend, &cands, out_vars.clone(), 5).unwrap();
    assert_eq!(result.holder, HolderId::new("federation"));
    assert_eq!(result.vars, out_vars);
    assert_eq!(
        result.rows.len(),
        2,
        "only the 2 matched rows survive (dummies filtered) — no phantom, no lost"
    );
    assert_eq!(cost.bound, 5);
    let got: Vec<String> = {
        let mut v: Vec<String> = result.rows.iter().map(|r| format!("{r:?}")).collect();
        v.sort();
        v
    };
    let mut exp = vec![
        format!("{:?}", vec![lit("Bob"), lit("Leeds")]),
        format!("{:?}", vec![lit("Carol"), lit("York")]),
    ];
    exp.sort();
    assert_eq!(got, exp, "disclosed multiset wrong");
}

// =============================================================================
// 3. Fail-closed cardinality contract — never silently truncate a candidate.
// =============================================================================

#[test]
fn bound_below_candidate_count_fails_closed_never_drops_a_match() {
    // A bound smaller than the candidate count would force a truncation that
    // could drop a true match (a LOST row); the path must FAIL CLOSED instead.
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let cands = public_candidates(&[(true, vec![lit("a")]), (true, vec![lit("b")])]);
    let err = oblivious_set_output(&backend, &cands, 1, 1).unwrap_err();
    assert!(
        err.to_string().contains("truncate"),
        "B < candidate count must fail closed citing truncation, got: {err}"
    );
}
