// sq-py8h.4 — tests for hidden-key endpoint DISTINCT.
use super::*;
use crate::shamir::ShamirBackend;
use std::collections::BTreeSet;

fn iri(s: &str) -> Term {
    Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!("http://ex/{s}")))
}

fn pair(a_key: u64, b_key: u64, a: &str, b: &str) -> SecretEndpointPair {
    SecretEndpointPair {
        a_key: Fp::new(a_key),
        b_key: Fp::new(b_key),
        a_term: iri(a),
        b_term: iri(b),
    }
}

/// The clear-text DISTINCT oracle: the set of distinct `(a_term, b_term)` pairs
/// keyed by the SECRET `(a_key, b_key)` (the dedup is over the secret keys, not the
/// terms — though in a well-formed input each secret key maps to one term).
fn clear_distinct(pairs: &[SecretEndpointPair]) -> BTreeSet<(u64, u64)> {
    pairs
        .iter()
        .map(|p| (p.a_key.value(), p.b_key.value()))
        .collect()
}

/// The DISTINCT result's surviving disclosed term pairs as a sorted set of strings.
fn result_set(part: &PartialResult) -> BTreeSet<(String, String)> {
    part.rows
        .iter()
        .map(|r| {
            let a = format!("{:?}", r[0]);
            let b = format!("{:?}", r[1]);
            (a, b)
        })
        .collect()
}

#[test]
fn distinct_collapses_duplicate_secret_pairs() {
    let backend = ShamirBackend::new_seeded(3, 99).unwrap();
    // (1,2) appears 3×, (3,4) appears 2×, (5,6) once → 3 distinct.
    let pairs = vec![
        pair(1, 2, "a", "b"),
        pair(3, 4, "c", "d"),
        pair(1, 2, "a", "b"),
        pair(5, 6, "e", "f"),
        pair(1, 2, "a", "b"),
        pair(3, 4, "c", "d"),
    ];
    let bound = pairs.len();
    let out = distinct_hidden_pairs(&backend, &pairs, bound).unwrap();

    // The result is exactly the 3 distinct pairs (each surviving once).
    assert_eq!(
        out.rows.len(),
        3,
        "duplicates must collapse to distinct set"
    );
    let got = result_set(&out);
    let expected: BTreeSet<(String, String)> = [
        (
            format!("{:?}", Some(iri("a"))),
            format!("{:?}", Some(iri("b"))),
        ),
        (
            format!("{:?}", Some(iri("c"))),
            format!("{:?}", Some(iri("d"))),
        ),
        (
            format!("{:?}", Some(iri("e"))),
            format!("{:?}", Some(iri("f"))),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(got, expected);
}

#[test]
fn differential_against_cleartext_distinct_many_shapes() {
    // Drive a range of multiplicities / orderings and compare cardinalities to the
    // clear-text DISTINCT oracle.
    let shapes: Vec<Vec<(u64, u64)>> = vec![
        vec![(1, 1)],
        vec![(1, 1), (1, 1)],
        vec![(1, 2), (2, 1)], // (1,2) != (2,1): order matters in the composite key
        vec![(1, 2), (2, 1), (1, 2), (2, 1), (3, 3)],
        vec![
            (7, 7),
            (7, 8),
            (8, 7),
            (7, 7),
            (8, 7),
            (9, 9),
            (9, 9),
            (9, 9),
        ],
        vec![(5, 5), (4, 4), (3, 3), (2, 2), (1, 1)], // reverse-sorted, all distinct
        vec![(2, 2), (2, 2), (2, 2), (2, 2)],         // all identical → 1
    ];
    for (s, shape) in shapes.iter().enumerate() {
        let backend = ShamirBackend::new_seeded(3, 1000 + s as u64).unwrap();
        let pairs: Vec<SecretEndpointPair> = shape
            .iter()
            .map(|&(a, b)| pair(a, b, &format!("a{a}"), &format!("b{b}")))
            .collect();
        let bound = pairs.len();
        let out = distinct_hidden_pairs(&backend, &pairs, bound).unwrap();
        let expected = clear_distinct(&pairs);
        assert_eq!(
            out.rows.len(),
            expected.len(),
            "shape {s} {shape:?}: distinct count must match clear-text oracle"
        );
        // Every surviving term pair must correspond to a distinct secret key (no
        // spurious or dropped pair). Map terms back via the (well-formed) key→term.
        let got = result_set(&out);
        let want: BTreeSet<(String, String)> = expected
            .iter()
            .map(|&(a, b)| {
                (
                    format!("{:?}", Some(iri(&format!("a{a}")))),
                    format!("{:?}", Some(iri(&format!("b{b}")))),
                )
            })
            .collect();
        assert_eq!(
            got, want,
            "shape {s}: surviving pairs must match the oracle"
        );
    }
}

#[test]
fn padding_reveals_exactly_bound_slots() {
    let backend = ShamirBackend::new_seeded(3, 7).unwrap();
    let pairs = vec![
        pair(1, 2, "a", "b"),
        pair(1, 2, "a", "b"),
        pair(3, 4, "c", "d"),
    ];
    let bound = 8; // padded well above |pairs|
    let (slots, cost) = distinct_hidden_pairs_slots(&backend, &pairs, bound).unwrap();
    assert_eq!(
        slots.len(),
        bound,
        "exactly B slots are revealed (L1 bound)"
    );
    let real = slots
        .iter()
        .filter(|s| matches!(s, OutputSlot::Row(_)))
        .count();
    assert_eq!(real, 2, "2 distinct pairs survive; the rest are dummies");
    assert_eq!(cost.rows, 3);
    assert_eq!(cost.output_bound, bound);
    // The sort over 3 items pays >0 compare-exchanges; the adjacent scan is N−1.
    assert!(cost.sort_compare_exchanges >= 1);
    assert_eq!(cost.adjacent_equalities, 2);
}

#[test]
fn empty_and_singleton() {
    let backend = ShamirBackend::new_seeded(3, 3).unwrap();
    // Empty input → empty distinct set, B may be 0.
    let out = distinct_hidden_pairs(&backend, &[], 0).unwrap();
    assert!(out.rows.is_empty());
    // Singleton → itself.
    let one = vec![pair(42, 43, "x", "y")];
    let out = distinct_hidden_pairs(&backend, &one, 1).unwrap();
    assert_eq!(out.rows.len(), 1);
}

#[test]
fn bound_below_row_count_fails_closed() {
    let backend = ShamirBackend::new_seeded(3, 5).unwrap();
    let pairs = vec![pair(1, 1, "a", "a"), pair(2, 2, "b", "b")];
    let err = distinct_hidden_pairs(&backend, &pairs, 1).unwrap_err();
    match err {
        MpcError::Protocol(m) => assert!(m.contains("truncate"), "got: {m}"),
        other => panic!("expected Protocol truncate error, got {other:?}"),
    }
}

#[test]
fn out_of_range_key_fails_closed() {
    let backend = ShamirBackend::new_seeded(3, 5).unwrap();
    let pairs = vec![SecretEndpointPair {
        a_key: Fp::new(COMPARE_MAX_EXCLUSIVE), // exactly the exclusive bound → out of range
        b_key: Fp::new(1),
        a_term: iri("a"),
        b_term: iri("b"),
    }];
    let err = distinct_hidden_pairs(&backend, &pairs, 1).unwrap_err();
    match err {
        MpcError::Protocol(m) => assert!(m.contains("COMPARE_BITS"), "got: {m}"),
        other => panic!("expected Protocol range error, got {other:?}"),
    }
}

#[test]
fn too_few_parties_fails_closed() {
    // n = 3, t = 2 → 3 < 2t+1 = 5: the degree reduction has no headroom. The public
    // constructors can never build this (they pin t = ⌊(n−1)/2⌋), so use the
    // test-only unchecked-threshold escape hatch to hit the fail-closed guard.
    let backend = ShamirBackend::with_unchecked_threshold(3, 2);
    let pairs = vec![pair(1, 1, "a", "a")];
    let err = distinct_hidden_pairs(&backend, &pairs, 1).unwrap_err();
    match err {
        MpcError::Protocol(m) => assert!(m.contains("2t+1"), "got: {m}"),
        other => panic!("expected Protocol party-count error, got {other:?}"),
    }
}

#[test]
fn too_many_rows_fails_closed() {
    let backend = ShamirBackend::new_seeded(3, 5).unwrap();
    let pairs: Vec<SecretEndpointPair> = (0..(MAX_DISTINCT_ROWS + 1) as u64)
        .map(|i| pair(i % 100, i % 50, "a", "b"))
        .collect();
    let err = distinct_hidden_pairs(&backend, &pairs, pairs.len()).unwrap_err();
    match err {
        MpcError::Protocol(m) => assert!(m.contains("MAX_DISTINCT_ROWS"), "got: {m}"),
        other => panic!("expected Protocol cap error, got {other:?}"),
    }
}

/// The sort's compare-exchange ACCESS PATTERN is a function of N only (the
/// obliviousness substrate): two inputs of the same length pay the same gate count
/// regardless of their secret key values / ordering.
#[test]
fn sort_access_pattern_is_data_independent() {
    let backend = ShamirBackend::new_seeded(3, 11).unwrap();
    let sorted = vec![
        pair(1, 1, "a", "a"),
        pair(2, 2, "b", "b"),
        pair(3, 3, "c", "c"),
    ];
    let reversed = vec![
        pair(3, 3, "c", "c"),
        pair(2, 2, "b", "b"),
        pair(1, 1, "a", "a"),
    ];
    let dups = vec![
        pair(7, 7, "z", "z"),
        pair(7, 7, "z", "z"),
        pair(7, 7, "z", "z"),
    ];
    let (_s1, c1) = distinct_hidden_pairs_slots(&backend, &sorted, 3).unwrap();
    let (_s2, c2) = distinct_hidden_pairs_slots(&backend, &reversed, 3).unwrap();
    let (_s3, c3) = distinct_hidden_pairs_slots(&backend, &dups, 3).unwrap();
    assert_eq!(c1.sort_compare_exchanges, c2.sort_compare_exchanges);
    assert_eq!(c1.sort_compare_exchanges, c3.sort_compare_exchanges);
    assert_eq!(c1.adjacent_equalities, c2.adjacent_equalities);
}

/// The conditional-swap actually sorts: after the sort the keep-bits collapse
/// duplicates that were NOT adjacent in the input (the sort must bring equal keys
/// together for the adjacent scan to see them).
#[test]
fn non_adjacent_duplicates_collapse() {
    let backend = ShamirBackend::new_seeded(3, 21).unwrap();
    // (1,1) and (1,1) are separated by other pairs; only a correct sort makes them
    // adjacent so the adjacent-equality scan dedups them.
    let pairs = vec![
        pair(1, 1, "a", "a"),
        pair(9, 9, "i", "i"),
        pair(5, 5, "e", "e"),
        pair(1, 1, "a", "a"),
        pair(9, 9, "i", "i"),
    ];
    let out = distinct_hidden_pairs(&backend, &pairs, pairs.len()).unwrap();
    assert_eq!(out.rows.len(), 3, "(1,1),(5,5),(9,9) distinct");
}

// ---- sq-yyro / issue #6117: the sort-and-scan stage draws through the seam ----

/// Every key sharing (`a_key` then `b_key`, row by row — the first `2N` own-input
/// draws) is shape-checked where it is consumed. Each case corrupts exactly ONE
/// key sharing and must be refused by the shape check itself.
#[test]
fn malformed_key_sharings_are_refused() {
    use crate::randomness::test_sources::{
        MalformedSource, SeamMethod, ALL_MALFORMATIONS, SHAPE_REFUSAL,
    };
    let backend = ShamirBackend::new_seeded(3, 0xD15C8).unwrap();
    let pairs = vec![
        pair(5, 6, "e", "f"),
        pair(1, 2, "a", "b"),
        pair(5, 6, "e", "f"),
    ];
    for skip in 0..2 * pairs.len() {
        for how in ALL_MALFORMATIONS {
            let mut src =
                MalformedSource::new(backend.dealer(), SeamMethod::VssOwnInput, skip, how);
            match distinct_hidden_pairs_slots_through(&mut src, &backend, &pairs, 4) {
                Err(MpcError::Protocol(m)) => {
                    assert!(
                        m.contains(SHAPE_REFUSAL),
                        "key sharing {skip}, {how:?}: {m}"
                    )
                }
                other => panic!("key sharing {skip}, {how:?}: must fail closed, got {other:?}"),
            }
            assert!(src.fired(), "key sharing {skip} was never drawn");
        }
    }
}

/// A source whose `(n, t)` differs from the backend's is refused by the up-front
/// check in `distinct_hidden_pairs_slots_through`, BEFORE the sort-and-scan
/// stage draws anything.
///
/// The shape matters: three pairs (one a duplicate) at `B = 4` is a shape whose
/// OWN stage draws — each row's two keys are shared as the holder's own input
/// before any comparison, the sites `malformed_key_sharings_are_refused` aims
/// at. With the up-front check deleted, the output stage's own
/// `require_source_matches` would still raise the same error, so the assertions
/// below are on the draw count and not only on the error.
#[test]
fn distinct_through_refuses_a_source_that_does_not_match_the_backend() {
    use crate::randomness::test_sources::{
        DrawCountingSource, MisreportedThresholdSource, RecordingSource,
    };
    let backend = ShamirBackend::new_seeded(3, 0xD15C9).unwrap();
    let pairs = vec![
        pair(1, 2, "a", "b"),
        pair(5, 6, "e", "f"),
        pair(1, 2, "a", "b"),
    ];

    // Wrong `n`: a dealer from a 5-party backend, recorded so the test can see
    // whether anything was drawn before the refusal.
    let mut rec = RecordingSource::new(ShamirBackend::new_seeded(5, 0xD15C9).unwrap().dealer());
    match distinct_hidden_pairs_slots_through(&mut rec, &backend, &pairs, 4) {
        Err(MpcError::Protocol(m)) => assert!(m.contains("but the backend is"), "{m}"),
        other => panic!("a wrong-n source must be refused, got {other:?}"),
    }
    assert!(
        rec.log.is_empty(),
        "drew {} times before refusing",
        rec.log.len()
    );

    // Misreported `t`: canonically SHAPED sharings, so only the source-match
    // check can catch it. Counted through a wrapper, since `RecordingSource`
    // wraps a real dealer and so cannot misreport a threshold.
    for t in [0, 2] {
        let mut lying = MisreportedThresholdSource {
            inner: backend.dealer(),
            t,
        };
        let mut counted = DrawCountingSource::new(&mut lying);
        match distinct_hidden_pairs_slots_through(&mut counted, &backend, &pairs, 4) {
            Err(MpcError::Protocol(m)) => assert!(m.contains("but the backend is"), "t={t}: {m}"),
            other => panic!("t={t}: a misreported threshold must be refused, got {other:?}"),
        }
        assert_eq!(counted.draws(), 0, "t={t}: drew before refusing");
    }
}

/// Under PRSS every shape fails closed for any `B > 0`, never panics. A row needs
/// its keys shared as the holder's own input, which PRSS refuses; and since #6117
/// step 6 routed the output stage through the same source, so does the EMPTY
/// DISTINCT — the output stage shares the real-row tag and the payload limbs for
/// each of its `B` pure-dummy slots, so an empty input is draw-free only at
/// `B = 0`. (Before the routing the empty case ran at any `B`, because the output
/// stage minted its own dealer and never touched the caller's source.)
///
/// First, on a DEALER source, the empty DISTINCT still returns its `B` dummy slots:
/// the PRSS refusal below belongs to PRSS, not to a shape this operator cannot run.
#[test]
fn prss_fails_closed_on_every_distinct_shape() {
    use crate::prss::PrssRandomness;
    let backend = ShamirBackend::new_seeded(3, 0xD15CA).unwrap();
    let (slots, cost) =
        distinct_hidden_pairs_slots_through(&mut backend.dealer(), &backend, &[], 2).unwrap();
    assert_eq!(slots, vec![OutputSlot::Dummy, OutputSlot::Dummy]);
    assert_eq!(cost.rows, 0);
    let mut prss = PrssRandomness::with_simulated_setup(3).unwrap();
    for pairs in [
        vec![],
        vec![pair(1, 2, "a", "b")],
        vec![pair(1, 2, "a", "b"), pair(1, 2, "a", "b")],
    ] {
        let outcome = distinct_hidden_pairs_slots_through(&mut prss, &backend, &pairs, 2);
        assert!(
            matches!(outcome, Err(MpcError::NotYetImplemented { .. })),
            "PRSS must fail closed over {} rows, got {outcome:?}",
            pairs.len()
        );
    }
}

/// Render revealed slots as `"<a> <b>"` / `"dummy"`, in revealed order.
fn slot_strings(slots: &[OutputSlot]) -> Vec<String> {
    slots
        .iter()
        .map(|s| match s {
            OutputSlot::Row(p) => p
                .iter()
                .map(|t| t.as_ref().expect("bound endpoint").to_string())
                .collect::<Vec<_>>()
                .join(" "),
            OutputSlot::Dummy => "dummy".to_string(),
        })
        .collect()
}

/// Every number below was captured on the PRE-ROUTING code (the stage calling
/// the dealer's inherent `share` / `degree_reduce` and the `&mut ShamirDealer`
/// comparison entry points) by logging every dealer draw, and re-checked
/// byte-identical after the routing. The keep-bit sharings pin the values, the
/// trailing `draw_fp()` WITNESS pins how many draws the stage consumed, and the
/// revealed slot order pins the public output end-to-end.
#[test]
fn seeded_distinct_stage_draws_exactly_what_the_dealer_drew_before_the_seam() {
    let backend = ShamirBackend::new_seeded(3, 0xD15C7).unwrap();
    let pairs = vec![
        pair(5, 6, "e", "f"),
        pair(1, 2, "a", "b"),
        pair(5, 6, "e", "f"),
    ];
    let mut dealer = backend.dealer();
    let (candidates, ce, adj) = build_distinct_candidates(&backend, &mut dealer, &pairs).unwrap();
    assert_eq!((ce, adj), (3, 2));
    let keeps: Vec<(Vec<String>, Vec<u64>)> = candidates
        .iter()
        .map(|c| {
            let MatchBit::SecretShared(bit) = &c.matched else {
                panic!("keep-bit must be secret-shared");
            };
            (
                c.payload
                    .iter()
                    .map(|t| t.as_ref().unwrap().to_string())
                    .collect(),
                bit.iter().map(|s| s.y.value()).collect(),
            )
        })
        .collect();
    let ab = vec!["<http://ex/a>".to_string(), "<http://ex/b>".to_string()];
    let ef = vec!["<http://ex/e>".to_string(), "<http://ex/f>".to_string()];
    assert_eq!(
        keeps,
        [
            (ab, vec![1, 1, 1]),
            (
                ef.clone(),
                vec![
                    2136424062827090610,
                    1967005116440487268,
                    1797586170053883926
                ]
            ),
            (
                ef,
                vec![977037174203969327, 1954074348407938654, 625268513398214030]
            ),
        ]
    );
    assert_eq!(dealer.draw_fp().value(), 888758199960741934);

    // The REVEALED slot ORDER has moved twice: first in #6117 step 6 (the output
    // stage draws its sharings and its shuffle from the SAME source as the
    // sort-and-scan stage instead of minting a second dealer), then again when the
    // output stage's per-element degree reduction added its re-sharings, so the
    // shuffle samples a later point of one continuous keystream. The order is the
    // shuffle's secret permutation and carries no result meaning — the
    // canonicalised multiset below is what the recipient sees, and it is
    // byte-identical to the pre-routing capture.
    let expected_slots = [
        "<http://ex/a> <http://ex/b>",
        "<http://ex/e> <http://ex/f>",
        "dummy",
        "dummy",
    ];
    let canonical = [
        "<http://ex/a> <http://ex/b>",
        "<http://ex/e> <http://ex/f>",
        "dummy",
        "dummy",
    ];
    let (slots, cost) = distinct_hidden_pairs_slots(&backend, &pairs, 4).unwrap();
    assert_eq!(slot_strings(&slots), expected_slots);
    let mut sorted = slot_strings(&slots);
    sorted.sort();
    assert_eq!(sorted, canonical, "revealed multiset must be unchanged");
    assert_eq!(
        (cost.sort_compare_exchanges, cost.adjacent_equalities),
        (3, 2)
    );
    // `distinct_hidden_pairs_oblivious` draws both stages from ONE dealer too, so
    // it reveals exactly the same slots in the same order, and its output-stage
    // `ObliviousOutputCost` is unchanged from the pre-routing capture.
    let (slots, out_cost) = distinct_hidden_pairs_oblivious(&backend, &pairs, 4).unwrap();
    assert_eq!(slot_strings(&slots), expected_slots);
    assert_eq!(
        (
            out_cost.bound,
            out_cost.select_mults,
            out_cost.opens,
            out_cost.shuffle.items,
            out_cost.shuffle.switches,
            out_cost.shuffle.depth_rounds
        ),
        (4, 4, 4, 4, 35, 3)
    );
    // The payload-limb select is the one NEW cost: each slot's endpoint-pair
    // payload packs six limbs (a length prefix plus five data limbs), so the limb
    // select adds 6 multiplications per slot on top of the one tag select.
    assert_eq!(out_cost.payload_select_mults, 24);
    // One degree reduction per SELECTED element: the tag plus the six limbs, for
    // each of the four slots.
    assert_eq!(out_cost.degree_reductions, 28);
}

/// The COMPLETE draw sequence, not just its length: every seam call the stage
/// makes, in order, with every `y` it returned, folded into an order-sensitive
/// fingerprint. A reordering, a swapped method or a changed value fails here even
/// when the witness draw count and the keep bits happen to agree.
///
/// **This is the #6117-step-6 guard.** The transcript now ends in the output
/// stage's 113 `vss_own_input` draws and its `oblivious_shuffle`: if
/// `distinct_hidden_pairs_slots_through` went back to minting its own dealer for
/// the output stage the log would stop at 8418 entries and both the length and the
/// suffix assertion would fail.
///
/// The 8418-entry PREFIX is the pre-routing whole-log pin
/// (`0x5455_9ace_20c7_0f97`, captured on the pre-routing code), asserted here
/// unchanged: the routing appended an output-stage SUFFIX and moved nothing in the
/// sort-and-scan part, and the output stage's per-element degree reduction only
/// grew that suffix. The full-log fingerprint is the new pin.
#[test]
fn seeded_distinct_stage_transcript_is_pinned() {
    use crate::randomness::test_sources::{
        fingerprint, prefix_before_first_shuffle, RecordingSource,
    };
    let backend = ShamirBackend::new_seeded(3, 0xD15C7).unwrap();
    let pairs = vec![
        pair(5, 6, "e", "f"),
        pair(1, 2, "a", "b"),
        pair(5, 6, "e", "f"),
    ];
    let mut rec = RecordingSource::new(backend.dealer());
    distinct_hidden_pairs_slots_through(&mut rec, &backend, &pairs, 4).unwrap();
    assert_eq!(rec.log.len(), 8532);
    // Prefix unchanged from the pre-routing pin; the whole move is in the output
    // suffix, which grew by the per-element degree reductions.
    assert_eq!(fingerprint(&rec.log[..8418]), 0x5455_9ace_20c7_0f97);
    // The draws BEFORE the shuffle are byte-for-byte unchanged: this prefix
    // fingerprint is the value the pre-re-randomisation code produced, so only
    // the shuffle's own output and the draws after it moved.
    assert_eq!(
        fingerprint(prefix_before_first_shuffle(&rec.log)),
        0x367d_69a7_76c8_bf27
    );
    assert_eq!(fingerprint(&rec.log), 0x4ec5_f6ca_6493_9b7a);
    let suffix: Vec<&str> = rec.log[8418..].iter().map(|(m, _)| *m).collect();
    assert_eq!(
        suffix,
        [["vss_own_input"; 113].as_slice(), &["oblivious_shuffle"]].concat()
    );
}

/// The DISTINCT stage refuses an over-cap output bound before ANY draw, on a
/// NON-EMPTY input: the sort-and-scan draws per comparison and the output stage
/// materialises `B` slots, so the refusal has to precede both.
#[test]
fn an_over_cap_bound_is_refused_before_any_draw() {
    use crate::oblivious_join::MAX_OUTPUT_BOUND;
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x9D01).unwrap();
    let pairs = vec![
        pair(1, 2, "a", "b"),
        pair(1, 2, "a", "b"),
        pair(3, 4, "c", "d"),
    ];
    for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = distinct_hidden_pairs_slots_through(&mut counted, &backend, &pairs, bound)
            .expect_err("an over-cap bound must be refused");
        match &err {
            MpcError::Protocol(m) => {
                assert!(m.contains("MAX_OUTPUT_BOUND"), "B={bound}: {m}")
            }
            other => panic!("B={bound}: expected a Protocol refusal, got {other:?}"),
        }
        assert_eq!(
            counted.draws(),
            0,
            "B={bound}: the bound must be refused before any draw"
        );
    }
    // A legal bound over the same input still runs.
    assert!(distinct_hidden_pairs(&backend, &pairs, 4).is_ok());
}
