// sq-py8h.5 — tests for the hidden bounded property-path planner guard
// + cost-model wiring: (a) statically-large unroll rejected, (b) hidden UNBOUNDED
// path REFUSED via NoBackendSatisfies (not approximated), (c) operator cost emitted
// through CommCounter and read by the matrix runner.
//! Acceptance suite for [`super`] (sq-py8h.5).

use super::*;
use crate::bench::{cell, run_matrix, QueryClass, BOUNDED_PATH_K};
use crate::partial::MpcError;
use crate::shamir::ShamirBackend;

// (b) FAIL-CLOSED: a hidden UNBOUNDED path is refused via NoBackendSatisfies, never
// approximated by a default k.
#[test]
fn unbounded_plus_is_refused_via_no_backend_satisfies() {
    let req = HiddenPathRequest::unbounded("p", 1); // (p)+
    let err = plan_hidden_bounded_path(&req, 4).unwrap_err();
    match err {
        MpcError::NoBackendSatisfies { requirement, .. } => {
            assert!(
                requirement.contains("OPEN upper bound"),
                "refusal must name the open/unbounded bound: {requirement}"
            );
        }
        other => panic!("expected NoBackendSatisfies for a hidden unbounded path, got {other:?}"),
    }
}

#[test]
fn unbounded_star_is_refused_too() {
    let req = HiddenPathRequest::unbounded("p", 0); // (p)*
    assert!(matches!(
        plan_hidden_bounded_path(&req, 8),
        Err(MpcError::NoBackendSatisfies { .. })
    ));
}

#[test]
fn open_bound_is_never_silently_approximated_to_a_default_k() {
    // The whole point of (b): an open bound must NEVER produce a runnable plan. If
    // it did, the operator would compute a wrong answer the verifier could not
    // recompute. So the result is Err, not Ok(BoundedPathPlan).
    let req = HiddenPathRequest {
        alternatives: vec!["p".into()],
        min: 2,
        max: PathUpperBound::Open,
    };
    assert!(plan_hidden_bounded_path(&req, 100).is_err());
}

// (a) STATICALLY-LARGE unroll is rejected with a controlled Protocol error.
#[test]
fn statically_large_unroll_is_rejected() {
    // |E| = 64 edges, k = 12 → Σ_ℓ 64^ℓ ≫ MAX_CHAIN_TUPLES (2^18). 64^4 alone is
    // 2^24, so even a 4-way bound blows past the cap.
    let req = HiddenPathRequest::range("p", 1, 12);
    let err = plan_hidden_bounded_path(&req, 64).unwrap_err();
    match err {
        MpcError::Protocol(m) => assert!(
            m.contains("MAX_CHAIN_TUPLES"),
            "rejection must cite the cap: {m}"
        ),
        other => panic!("expected Protocol over-cap rejection, got {other:?}"),
    }
}

// Hop counts are capped before the projection: an upper bound past MAX_HOP_COUNT is
// refused with a controlled Protocol error rather than running `project_bounded`,
// whose own loop count grows with the bound. Finishing the test is the no-hang
// witness; nothing here asserts a timing.
#[test]
fn upper_bound_past_the_hop_cap_is_rejected() {
    for max in [MAX_HOP_COUNT + 1, 262_144, usize::MAX] {
        let req = HiddenPathRequest::range("p", 1, max);
        let err = plan_hidden_bounded_path(&req, 4).unwrap_err();
        match err {
            MpcError::Protocol(m) => assert!(
                m.contains("MAX_HOP_COUNT"),
                "max={}: rejection must cite the hop cap: {}",
                max,
                m
            ),
            other => panic!("max={}: expected Protocol refusal, got {:?}", max, other),
        }
    }
    // At the cap the hop guard passes; |E| = 1 keeps the projection inside the
    // tuple cap, so the plan is admitted.
    assert!(plan_hidden_bounded_path(&HiddenPathRequest::range("p", 1, MAX_HOP_COUNT), 1).is_ok());
}

#[test]
fn alternation_arity_blowup_is_rejected() {
    // A 6-way alternation at k = 8 over |E| = 6: Σ_ℓ (6·6)^ℓ overshoots the cap.
    let req = HiddenPathRequest::range("p", 1, 8);
    let req = HiddenPathRequest {
        alternatives: vec![
            "p1".into(),
            "p2".into(),
            "p3".into(),
            "p4".into(),
            "p5".into(),
            "p6".into(),
        ],
        ..req
    };
    assert!(matches!(
        plan_hidden_bounded_path(&req, 6),
        Err(MpcError::Protocol(_))
    ));
}

#[test]
fn min_greater_than_max_is_rejected() {
    let req = HiddenPathRequest::range("p", 5, 2);
    assert!(matches!(
        plan_hidden_bounded_path(&req, 4),
        Err(MpcError::Protocol(_))
    ));
}

#[test]
fn empty_alternation_is_rejected() {
    let req = HiddenPathRequest {
        alternatives: vec![],
        min: 1,
        max: PathUpperBound::Finite(2),
    };
    assert!(matches!(
        plan_hidden_bounded_path(&req, 4),
        Err(MpcError::Protocol(_))
    ));
}

// An admissible small bound is PLANNED, with the right projected enumeration size.
#[test]
fn small_bound_is_planned_with_projected_sizes() {
    // |E| = 3 edges, (p){1,3}: Σ_ℓ 3^ℓ = 3 + 9 + 27 = 39 tuples.
    // secure equalities = Σ_ℓ 3^ℓ·(ℓ−1) = 3·0 + 9·1 + 27·2 = 63.
    let req = HiddenPathRequest::range("p", 1, 3);
    let plan = plan_hidden_bounded_path(&req, 3).unwrap();
    assert_eq!(plan.projected_tuples, 39);
    assert_eq!(plan.secure_equalities, 63);
    assert_eq!(plan.edge_count, 3);
    assert!(!plan.warn, "39 tuples is far below the warn threshold");
    assert_eq!(plan.form, HiddenBoundedPath::range("p", 1, 3));
}

#[test]
fn admissible_but_heavy_path_sets_the_warn_flag() {
    // Pick a bound whose projection is admissible (≤ cap) but above the warn
    // threshold (cap/16 = 2^14 = 16_384). |E| = 26, (p){1,4}:
    // 26 + 26² + 26³ + 26⁴ = 26 + 676 + 17_576 + 456_976 — that overshoots the cap.
    // Use |E| = 14, k = 4: 14 + 196 + 2_744 + 38_416 = 41_350 — also over cap.
    // |E| = 11, k = 4: 11 + 121 + 1_331 + 14_641 = 16_104 < cap, > warn? warn=16_384,
    // 16_104 < 16_384 → not warned. |E| = 12, k = 4: 12+144+1728+20736 = 22_620:
    // > warn (16_384) and < cap (262_144) → admissible AND warned.
    let req = HiddenPathRequest::range("p", 1, 4);
    let plan = plan_hidden_bounded_path(&req, 12).unwrap();
    assert!(
        plan.projected_tuples <= MAX_CHAIN_TUPLES,
        "must be admissible"
    );
    assert!(
        plan.warn,
        "a heavy-but-admissible projection ({} tuples) should warn",
        plan.projected_tuples
    );
}

// (c) COST WIRING: the plan emits a non-trivial CommCounter; the matrix cell uses it.
#[test]
fn comm_counter_counts_equalities_reductions_and_output() {
    // (p){1,3} over |E| = 3, B = 4 slots.
    let req = HiddenPathRequest::range("p", 1, 3);
    let plan = plan_hidden_bounded_path(&req, 3).unwrap();
    let c = plan.comm_counter(5, 4).unwrap();
    // The k×-join secure equalities (63) each contribute one mult + one open; the
    // reduction chain folds another `secure_equalities` mults; the B=4 output adds
    // 4 select mults + 4 opens + the shuffle's switch mults. So multiplications and
    // opens must BOTH exceed the bare equality count — the cost is genuinely wired.
    assert!(
        c.multiplications >= plan.secure_equalities * 2 + 4,
        "mults must include equalities + reductions + B select mults"
    );
    assert!(
        c.bytes_per_party().unwrap() > 0,
        "a wired operator pays non-zero modelled bytes"
    );
    assert!(
        c.total_rounds().unwrap() > 0,
        "a wired operator has communication rounds"
    );
}

#[test]
fn comm_counter_is_deterministic() {
    let req = HiddenPathRequest::range("p", 1, 3);
    let plan = plan_hidden_bounded_path(&req, 3).unwrap();
    let a = plan.comm_counter(5, 4).unwrap();
    let b = plan.comm_counter(5, 4).unwrap();
    assert_eq!(a.multiplications, b.multiplications);
    assert_eq!(a.total_bytes().unwrap(), b.total_bytes().unwrap());
}

#[test]
fn matrix_runner_emits_a_bounded_path_cell() {
    // The matrix sweeps the bounded-path class; the cell carries a real cost cell.
    let r = run_matrix(&[3, 5], 2, 4).unwrap();
    let bp: Vec<_> = r
        .cells
        .iter()
        .filter(|c| c.query_class == QueryClass::HiddenBoundedPath)
        .collect();
    assert!(
        !bp.is_empty(),
        "the matrix must include the bounded-path class"
    );
    for c in bp {
        assert!(
            c.multiplications() > 0,
            "the bounded-path cell must carry a counted (non-zero) cost"
        );
    }
}

#[test]
fn matrix_bounded_path_cost_grows_with_edge_count() {
    // More secret edges → a strictly larger k×-join enumeration → more mults.
    let small = cell(QueryClass::HiddenBoundedPath, 5, 3).unwrap();
    let large = cell(QueryClass::HiddenBoundedPath, 5, 5).unwrap();
    assert!(
        large.multiplications() > small.multiplications(),
        "bounded-path cost must grow with |E|: {} vs {}",
        large.multiplications(),
        small.multiplications()
    );
}

// The cost CELL co-gates correctness via the real operator (design §5.4).
#[test]
fn bounded_path_correctness_gate_passes() {
    use crate::bench::check_bounded_path;
    let backend = ShamirBackend::new(5).unwrap();
    // A length-4 chain: (p){1,k} connects (i,j) with 1<=j-i<=k.
    let pairs = check_bounded_path(&backend, 4).unwrap();
    // k = BOUNDED_PATH_K; oracle = Σ_i min(i+k,4) - i over i in 0..=4.
    let k = BOUNDED_PATH_K;
    let expected: usize = (0..=4usize).map(|i| (i + k).min(4) - i).sum();
    assert_eq!(pairs, expected);
}

/// The cost model's own output bound is capped. `comm_counter` iterates `B` times
/// and builds a real Waksman network over `B` slots, so an unchecked `B` would do
/// work and allocate in proportion to a caller-chosen number — and it would be
/// modelling a run the output stage itself refuses.
#[test]
fn comm_counter_refuses_an_over_cap_bound() {
    use crate::oblivious_join::MAX_OUTPUT_BOUND;
    let req = HiddenPathRequest::range("p", 1, 3);
    let plan = plan_hidden_bounded_path(&req, 3).unwrap();
    // `MAX_OUTPUT_BOUND + 1` first on purpose: with the cap removed, that case
    // fails in a moment, whereas `usize::MAX` would instead build a `usize::MAX`
    // wide modelled network and never finish. A mutant should go red, not hang.
    for bound in [MAX_OUTPUT_BOUND + 1, usize::MAX] {
        match plan.comm_counter(5, bound) {
            Err(MpcError::Protocol(m)) => {
                assert!(m.contains("MAX_OUTPUT_BOUND"), "B={}: {}", bound, m)
            }
            other => panic!(
                "B={}: an over-cap bound must be a Protocol refusal, got {:?}",
                bound,
                other.map(|c| c.multiplications)
            ),
        }
    }
    // A bound inside the cap is still modelled.
    assert!(plan.comm_counter(5, 4).is_ok());
}
