// sq-py8h.2 — DIFFERENTIAL tests for the HIDDEN-intermediate
// exactly-`k` bounded property-path chain.
//! Differential acceptance suite for [`eval_exact_k_chain_hidden`].
//!
//! THE acceptance criterion (bead sq-py8h.2): the secure HIDDEN-intermediate
//! exactly-`k` chain must equal the **plaintext** bounded-path join over the union
//! edge set, while keys are NEVER reconstructed, intermediate node-sets stay
//! secret-shared, the per-pair match bit is never opened, and the output is
//! padded+shuffled to the public bound `B`.
//!
//! Unlike the disclosed-key suite ([`crate::bounded_path::tests`]), this path is
//! the CRYPTOGRAPHIC core: every hop's match is a secret-shared
//! [`crate::compare::secure_equal_to_bit`] bit, the `k` hops are chained by AND
//! (`mul_shares_raw` + `degree_reduce`), the per-edge-tuple chains are OR-folded,
//! and the final connected-bit drives [`crate::oblivious_join::oblivious_set_output`]
//! as a [`crate::oblivious_join::MatchBit::SecretShared`] selector. The differential
//! ORACLE is a pure plaintext exactly-`k` reachability over the same edge set,
//! computed in this test file (no MPC), so the equality witnesses correctness of
//! the crypto path against an independent computation.

use super::*;
use crate::field::Fp;
use crate::partial::HolderId;
use crate::shamir::ShamirBackend;
use oxrdf::{NamedNode, Term};
use std::collections::BTreeSet;

fn nn(iri: &str) -> NamedNode {
    NamedNode::new(iri).unwrap()
}

fn iri_term(iri: &str) -> Term {
    Term::NamedNode(nn(iri))
}

/// A tiny in-test node interner: map distinct node IRIs to distinct nonzero `Fp`
/// keys (the holder's private-key encoding stand-in — exactly the injective
/// encoding contract `HiddenValueJoin` documents). Returns `(term, fp)` pairs.
struct Interner {
    by_iri: std::collections::HashMap<String, Fp>,
    next: u64,
}
impl Interner {
    fn new() -> Self {
        Interner {
            by_iri: std::collections::HashMap::new(),
            next: 1, // nonzero keys (0 is a fine field element but keep it simple)
        }
    }
    fn key(&mut self, iri: &str) -> Fp {
        if let Some(k) = self.by_iri.get(iri) {
            return *k;
        }
        let k = Fp::new(self.next);
        self.next += 1;
        self.by_iri.insert(iri.to_string(), k);
        k
    }
}

/// Build a [`HiddenEdges`] from a list of `(subject_iri, object_iri)` edges,
/// interning IRIs to nonzero `Fp` keys. Returns the edges plus the interner.
fn hidden_edges(pairs: &[(&str, &str)]) -> (HiddenEdges, Interner) {
    let mut interner = Interner::new();
    let mut edges: Vec<HiddenEdge> = Vec::new();
    for (s, o) in pairs {
        edges.push(HiddenEdge {
            subject: HiddenNode {
                key: interner.key(s),
                term: iri_term(s),
            },
            object: HiddenNode {
                key: interner.key(o),
                term: iri_term(o),
            },
        });
    }
    (HiddenEdges::new(edges), interner)
}

/// PLAINTEXT differential oracle: the set of `(a,b)` IRI pairs connected by an
/// EXACTLY-`k`-hop chain over the edge set, computed with NO MPC. This is the
/// independent reference the secure path must match.
fn plaintext_exact_k(pairs: &[(&str, &str)], k: usize) -> BTreeSet<(String, String)> {
    if k == 0 {
        let mut nodes: BTreeSet<String> = BTreeSet::new();
        for (s, o) in pairs {
            nodes.insert((*s).to_string());
            nodes.insert((*o).to_string());
        }
        return nodes.into_iter().map(|n| (n.clone(), n)).collect();
    }
    // length-1 = the edges themselves.
    let mut current: BTreeSet<(String, String)> = pairs
        .iter()
        .map(|(s, o)| (s.to_string(), o.to_string()))
        .collect();
    for _ in 1..k {
        let mut next: BTreeSet<(String, String)> = BTreeSet::new();
        for (a, mid) in &current {
            for (s, o) in pairs {
                if *s == mid.as_str() {
                    next.insert((a.clone(), o.to_string()));
                }
            }
        }
        current = next;
    }
    current
}

/// Run the SECURE hidden-intermediate exactly-`k` chain and return its disclosed
/// `(a,b)` IRI pair set (dummies filtered). `B` is the public padded bound.
fn secure_exact_k(
    backend: &ShamirBackend,
    edges: &HiddenEdges,
    k: usize,
    bound: usize,
) -> BTreeSet<(String, String)> {
    let result = eval_exact_k_chain_hidden(backend, edges, k, bound).unwrap();
    assert_eq!(result.holder, HolderId::new("federation"));
    assert_eq!(
        result.vars.len(),
        2,
        "endpoint result must be 2-column (?a,?b)"
    );
    result
        .rows
        .iter()
        .map(|r| {
            let a = match &r[0] {
                Some(Term::NamedNode(n)) => n.as_str().to_string(),
                other => panic!("endpoint a must be a bound IRI, got {other:?}"),
            };
            let b = match &r[1] {
                Some(Term::NamedNode(n)) => n.as_str().to_string(),
                other => panic!("endpoint b must be a bound IRI, got {other:?}"),
            };
            (a, b)
        })
        .collect()
}

// =====================================================================
// THE acceptance test: secure exactly-k == plaintext exactly-k
// =====================================================================

/// `k = 2`: a small graph with TWO 2-hop chains and a dead-end branch. The secure
/// hidden-intermediate result must equal the plaintext exactly-2 reachability.
#[test]
fn differential_exact_2_equals_plaintext() {
    // a->b->c, a->b->d (so a reaches c and d at len-2), plus x->y with no
    // continuation (x reaches nothing at len-2).
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
        ("http://ex/b", "http://ex/d"),
        ("http://ex/x", "http://ex/y"),
    ];
    let (edges, _interner) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 0xC0FFEE).unwrap();

    let want = plaintext_exact_k(&pairs, 2);
    let got = secure_exact_k(&backend, &edges, 2, 64);
    assert_eq!(got, want, "secure exactly-2 != plaintext exactly-2");
    // Concretely a->c and a->d.
    assert_eq!(got.len(), 2, "{got:?}");
}

/// `k = 3`: a chain a->b->c->d->e; exactly-3 connects a->d and b->e.
#[test]
fn differential_exact_3_equals_plaintext() {
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
        ("http://ex/c", "http://ex/d"),
        ("http://ex/d", "http://ex/e"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 7).unwrap();
    let want = plaintext_exact_k(&pairs, 3);
    let got = secure_exact_k(&backend, &edges, 3, 128);
    assert_eq!(got, want);
    assert_eq!(got.len(), 2, "{got:?}"); // (a,d) and (b,e)
}

/// `k = 1`: the chain degenerates to the edge relation itself (one hop). Pins the
/// base case of the recurrence.
#[test]
fn differential_exact_1_equals_edges() {
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let want = plaintext_exact_k(&pairs, 1);
    let got = secure_exact_k(&backend, &edges, 1, 32);
    assert_eq!(got, want);
    assert_eq!(got.len(), 2, "{got:?}");
}

/// A graph where NO exactly-k chain exists must yield the empty result (all
/// dummies) — pins that the connected-bit is `0` everywhere and nothing leaks
/// through as a false match.
#[test]
fn differential_no_chain_yields_empty() {
    // Two disjoint single edges: no 2-hop chain exists.
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/c", "http://ex/d"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 99).unwrap();
    let got = secure_exact_k(&backend, &edges, 2, 32);
    assert!(got.is_empty(), "expected no 2-hop pairs, got {got:?}");
}

/// A self-loop and a cycle: a->b->a (a 2-cycle) gives a->a and b->b at len-2.
/// Pins that cycles are handled like the plaintext oracle (no special-casing).
#[test]
fn differential_cycle_equals_plaintext() {
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/a"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 55).unwrap();
    let want = plaintext_exact_k(&pairs, 2);
    let got = secure_exact_k(&backend, &edges, 2, 32);
    assert_eq!(got, want);
    // len-2: a->b->a = (a,a); b->a->b = (b,b).
    assert_eq!(got.len(), 2, "{got:?}");
}

// =====================================================================
// L1 obliviousness: the revealed output is padded+shuffled to B
// =====================================================================

/// L1: two graphs with DIFFERENT true match counts but the same `B` reveal the
/// SAME number of output slots (the padded bound), not the true cardinality.
#[test]
fn revealed_slot_count_is_bound_not_true_cardinality() {
    let many = [
        ("http://ex/a", "http://ex/m"),
        ("http://ex/m", "http://ex/c"),
        ("http://ex/m", "http://ex/d"),
        ("http://ex/m", "http://ex/e"),
    ];
    let few = [
        ("http://ex/a", "http://ex/m"),
        ("http://ex/m", "http://ex/c"),
    ];
    let (e_many, _i1) = hidden_edges(&many);
    let (e_few, _i2) = hidden_edges(&few);
    let backend = ShamirBackend::new_seeded(3, 3).unwrap();
    let (slots_many, cost_many) =
        eval_exact_k_chain_hidden_slots(&backend, &e_many, 2, 50).unwrap();
    let (slots_few, cost_few) = eval_exact_k_chain_hidden_slots(&backend, &e_few, 2, 50).unwrap();
    assert_eq!(slots_many.len(), slots_few.len(), "slot count must equal B");
    assert_eq!(cost_many.bound, 50);
    assert_eq!(cost_few.bound, 50);
}

// =====================================================================
// Fail-closed contracts
// =====================================================================

/// `k = 0` is rejected: the exactly-`k` chain operator is for `k >= 1` (the
/// reflexive length-0 arm is the sibling `{0,k}` increment, sq-py8h.3).
#[test]
fn k_zero_is_rejected() {
    let (edges, _i) = hidden_edges(&[("http://ex/a", "http://ex/b")]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let err = eval_exact_k_chain_hidden(&backend, &edges, 0, 8).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(m) if m.contains("k")));
}

/// A bound `B` below the candidate count fails closed (never truncates, which
/// could drop a true match).
#[test]
fn bound_below_candidate_count_fails_closed() {
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let err = eval_exact_k_chain_hidden(&backend, &edges, 2, 1).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(_)));
}

/// An over-large `k` that would explode the edge-tuple enumeration is rejected
/// fail-closed BEFORE doing any crypto (a public, statically-computable guard).
#[test]
fn oversized_k_is_rejected() {
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
        ("http://ex/c", "http://ex/a"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let err = eval_exact_k_chain_hidden(&backend, &edges, 64, 1 << 30).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(m) if m.contains("chain")));
}

/// Hop counts are capped before evaluation: a `k` past [`MAX_HOP_COUNT`] — up to
/// `usize::MAX`, where the per-hop counting loop and the `k`-wide odometer would
/// otherwise run — is refused by the hop guard with NO draw made, so the refusal
/// does not wait on the enumeration being sized. Finishing the test at all is the
/// no-hang witness; nothing here asserts a timing.
#[test]
fn huge_hop_count_is_refused_before_any_draw() {
    use crate::randomness::test_sources::DrawCountingSource;
    let (edges, _i) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ]);
    // A single self-loop edge too: with |E| = 1 the projected tuple count stays 1
    // for ANY k, so MAX_CHAIN_TUPLES never fires and only the hop cap refuses.
    let (one_edge, _j) = hidden_edges(&[("http://ex/a", "http://ex/a")]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();

    for edge_set in [&edges, &one_edge] {
        for k in [MAX_HOP_COUNT + 1, 262_144, usize::MAX] {
            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            let err =
                eval_exact_k_chain_hidden_slots_through(&mut counted, &backend, edge_set, k, 8)
                    .expect_err("a hop count past the cap must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_HOP_COUNT")),
                "k={}: expected the hop-count refusal, got {:?}",
                k,
                err
            );
            assert_eq!(counted.draws(), 0, "k={}: refused before any draw", k);
        }
    }
    // The cap itself is still a legal `k` as far as the hop guard goes (|E| = 1
    // keeps the tuple count at 1, so nothing else refuses it).
    assert!(eval_exact_k_chain_hidden(&backend, &one_edge, MAX_HOP_COUNT, 8).is_ok());
}

// =====================================================================
// Determinism of the disclosed result across shuffle seeds
// =====================================================================

/// The disclosed `(a,b)` multiset is invariant to the shuffle's permutation: the
/// SAME pair set comes out regardless of the backend seed.
#[test]
fn result_is_shuffle_seed_invariant() {
    let pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
        ("http://ex/b", "http://ex/d"),
    ];
    let (edges, _i) = hidden_edges(&pairs);
    let want = plaintext_exact_k(&pairs, 2);
    for seed in 0..8u64 {
        let backend = ShamirBackend::new_seeded(5, 1000 + seed).unwrap();
        let got = secure_exact_k(&backend, &edges, 2, 64);
        assert_eq!(got, want, "seed {seed}: result changed");
    }
}

// ---- sq-yyro / issue #6117: the chain stage draws through the seam ----

/// The disclosed `(a, b)` IRI pairs among the revealed slots (dummies dropped).
fn slot_pairs(slots: &[crate::oblivious_join::OutputSlot]) -> BTreeSet<(String, String)> {
    use crate::oblivious_join::OutputSlot;
    slots
        .iter()
        .filter_map(|s| match s {
            OutputSlot::Row(p) => match (&p[0], &p[1]) {
                (Some(Term::NamedNode(a)), Some(Term::NamedNode(b))) => {
                    Some((a.as_str().to_string(), b.as_str().to_string()))
                }
                other => panic!("endpoints must be bound IRIs, got {other:?}"),
            },
            OutputSlot::Dummy => None,
        })
        .collect()
}

/// A source whose `(n, t)` differs from the backend's is refused by the up-front
/// check in `eval_exact_k_chain_hidden_slots_through`, BEFORE the chain stage
/// draws anything.
///
/// The shape matters: `k = 2` over the two-hop chain at `B = 4` is a shape whose
/// OWN stage draws (the hop equalities' operand bits, then the AND fold's
/// re-sharings — the sites `malformed_draws_in_the_chain_are_refused` aims at).
/// A draw-free shape would be refused by the output stage's own
/// `require_source_matches` even with the up-front check deleted, which is why
/// the assertions below are on the draw count and not only on the error.
#[test]
fn exact_k_through_refuses_a_source_that_does_not_match_the_backend() {
    use crate::randomness::test_sources::{
        DrawCountingSource, MisreportedThresholdSource, RecordingSource,
    };
    let backend = ShamirBackend::new_seeded(3, 0x9A7C).unwrap();
    let (edges, _) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ]);

    // Wrong `n`: a dealer from a 5-party backend, recorded so the test can see
    // whether anything was drawn before the refusal.
    let mut rec = RecordingSource::new(ShamirBackend::new_seeded(5, 0x9A7C).unwrap().dealer());
    match eval_exact_k_chain_hidden_slots_through(&mut rec, &backend, &edges, 2, 4) {
        Err(MpcError::Protocol(m)) => assert!(m.contains("but the backend is"), "{m}"),
        other => panic!("a wrong-n source must be refused, got {other:?}"),
    }
    assert!(
        rec.log.is_empty(),
        "drew {} times before refusing",
        rec.log.len()
    );

    // Misreported `t`: every sharing such a source returns is canonically SHAPED
    // for the backend's `n`, so no downstream shape check can catch it — only the
    // source-match check can. Counted through a wrapper, since `RecordingSource`
    // wraps a real dealer and so cannot misreport a threshold.
    for t in [0, 2] {
        let mut lying = MisreportedThresholdSource {
            inner: backend.dealer(),
            t,
        };
        let mut counted = DrawCountingSource::new(&mut lying);
        match eval_exact_k_chain_hidden_slots_through(&mut counted, &backend, &edges, 2, 4) {
            Err(MpcError::Protocol(m)) => assert!(m.contains("but the backend is"), "t={t}: {m}"),
            other => panic!("t={t}: a misreported threshold must be refused, got {other:?}"),
        }
        assert_eq!(counted.draws(), 0, "t={t}: drew before refusing");
    }
}

/// A malformed draw anywhere in the chain is refused by the check at its
/// consumption site: the first own-input draw is a hop equality's operand bit
/// (shape-checked in `compare`), and the first draw after the two operands'
/// `2 · COMPARE_BITS` bits is a degree-reduction re-sharing.
#[test]
fn malformed_draws_in_the_chain_are_refused() {
    use crate::compare::COMPARE_BITS;
    use crate::randomness::test_sources::{
        MalformedSource, SeamMethod, ALL_MALFORMATIONS, SHAPE_REFUSAL,
    };
    let backend = ShamirBackend::new_seeded(3, 0x9A7D).unwrap();
    let (edges, _) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ]);
    for (skip, expected) in [
        (0, SHAPE_REFUSAL),
        (2 * COMPARE_BITS, "re-sharing that is not the canonical"),
    ] {
        for how in ALL_MALFORMATIONS {
            let mut src =
                MalformedSource::new(backend.dealer(), SeamMethod::VssOwnInput, skip, how);
            match eval_exact_k_chain_hidden_slots_through(&mut src, &backend, &edges, 2, 4) {
                Err(MpcError::Protocol(m)) => assert!(m.contains(expected), "{skip}, {how:?}: {m}"),
                other => panic!("{skip}, {how:?}: must fail closed, got {other:?}"),
            }
            assert!(src.fired());
        }
    }
}

/// Under PRSS every shape fails closed for any `B > 0`, never panics. `k = 2` needs
/// a hop equality and `k = 1` over parallel edges needs an OR-fold, both of which
/// share a holder's own input — which PRSS refuses. Since #6117 step 6 routed the
/// output stage through the same source, so does `k = 1` over distinct endpoint
/// pairs, whose chain stage draws nothing: the output stage shares the real-row tag
/// and the payload limbs for each of its `B` slots, so that shape is draw-free only
/// at `B = 0`. (Before the routing that case ran at any `B`, because the output
/// stage minted its own dealer and never touched the caller's source.)
#[test]
fn prss_fails_closed_on_every_exact_k_shape() {
    use crate::prss::PrssRandomness;
    let backend = ShamirBackend::new_seeded(3, 0x9A7E).unwrap();
    let mut prss = PrssRandomness::with_simulated_setup(3).unwrap();
    let (edges, _) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ]);
    let (parallel, _) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/a", "http://ex/b"),
    ]);
    // The formerly-draw-free shape still evaluates CORRECTLY through the seam on a
    // dealer source — it is the SOURCE that refuses below, not the shape.
    let (slots, _) =
        eval_exact_k_chain_hidden_slots_through(&mut backend.dealer(), &backend, &edges, 1, 3)
            .unwrap();
    assert_eq!(slots.len(), 3);
    assert_eq!(
        slot_pairs(&slots),
        plaintext_exact_k(
            &[
                ("http://ex/a", "http://ex/b"),
                ("http://ex/b", "http://ex/c"),
            ],
            1
        )
    );

    for (edges, k) in [(&edges, 1), (&edges, 2), (&parallel, 1)] {
        let outcome = eval_exact_k_chain_hidden_slots_through(&mut prss, &backend, edges, k, 4);
        assert!(
            matches!(outcome, Err(MpcError::NotYetImplemented { .. })),
            "k = {k}: PRSS must fail closed, got {outcome:?}"
        );
    }
}

/// The per-pair connected-bit sharings of a candidate list, in candidate order.
fn candidate_bits(candidates: &[crate::oblivious_join::Candidate]) -> Vec<(Vec<String>, Vec<u64>)> {
    candidates
        .iter()
        .map(|c| {
            let MatchBit::SecretShared(bit) = &c.matched else {
                panic!("connected-bit must be secret-shared");
            };
            (
                c.payload
                    .iter()
                    .map(|t| t.as_ref().unwrap().to_string())
                    .collect(),
                bit.iter().map(|s| s.y.value()).collect(),
            )
        })
        .collect()
}

/// Render revealed slots as `"<a> <b>"` / `"dummy"`, in revealed order.
fn slot_strings(slots: &[crate::oblivious_join::OutputSlot]) -> Vec<String> {
    use crate::oblivious_join::OutputSlot;
    slots
        .iter()
        .map(|s| match s {
            OutputSlot::Row(p) => p
                .iter()
                .map(|t| t.as_ref().unwrap().to_string())
                .collect::<Vec<_>>()
                .join(" "),
            OutputSlot::Dummy => "dummy".to_string(),
        })
        .collect()
}

const PIN_EDGES: [(&str, &str); 2] = [
    ("http://ex/a", "http://ex/b"),
    ("http://ex/b", "http://ex/c"),
];

/// Captured on the PRE-ROUTING code by logging every dealer draw and re-checked
/// byte-identical after the routing: the connected-bit sharings pin the values,
/// the trailing `draw_fp()` WITNESS pins the draw count, and the revealed slot
/// order pins the public output.
#[test]
fn seeded_exact_k_stage_draws_exactly_what_the_dealer_drew_before_the_seam() {
    let backend = ShamirBackend::new_seeded(3, 0x9A7B).unwrap();
    let (edges, _) = hidden_edges(&PIN_EDGES);
    let mut dealer = backend.dealer();
    let candidates = build_candidates(&backend, &mut dealer, &edges, 2).unwrap();
    let pair = |a: &str, b: &str| vec![format!("<http://ex/{a}>"), format!("<http://ex/{b}>")];
    assert_eq!(
        candidate_bits(&candidates),
        [
            (
                pair("a", "b"),
                vec![200120304358927318, 400240608717854636, 600360913076781954]
            ),
            (
                pair("a", "c"),
                vec![1085933241605771511, 2171866483211543021, 951956715603620580]
            ),
            (
                pair("b", "b"),
                vec![1627966387087110708, 950089764960527465, 272213142833944222]
            ),
            (
                pair("b", "c"),
                vec![501855066999073978, 1003710133998147956, 1505565200997221934]
            ),
        ]
    );
    assert_eq!(dealer.draw_fp().value(), 641842556892271371);
    // The REVEALED slot ORDER has moved twice: first in #6117 step 6 (the output
    // stage draws its sharings and its shuffle from the SAME source as the chain
    // stage instead of minting a second dealer), then again when the output stage's
    // per-element degree reduction added its re-sharings, so the shuffle samples a
    // later point of one continuous keystream. The order is the shuffle's secret
    // permutation and carries no result meaning — the canonicalised multiset below
    // is byte-identical to the pre-routing capture.
    let (slots, cost) = eval_exact_k_chain_hidden_slots(&backend, &edges, 2, 5).unwrap();
    assert_eq!(
        slot_strings(&slots),
        [
            "<http://ex/a> <http://ex/c>",
            "dummy",
            "dummy",
            "dummy",
            "dummy"
        ]
    );
    let mut sorted = slot_strings(&slots);
    sorted.sort();
    assert_eq!(
        sorted,
        [
            "<http://ex/a> <http://ex/c>",
            "dummy",
            "dummy",
            "dummy",
            "dummy"
        ],
        "revealed multiset must be unchanged"
    );
    assert_eq!(
        (
            cost.bound,
            cost.select_mults,
            cost.opens,
            cost.shuffle.items,
            cost.shuffle.switches,
            cost.shuffle.depth_rounds
        ),
        (5, 5, 5, 5, 56, 5)
    );
}

/// THE guard for the output stage's payload select on the two-hop chain
/// `a → b → c`: only the pair `(a, c)` is connected, so every other endpoint-pair
/// candidate is rejected — and no rejected pair's endpoint row may appear among the
/// opened payloads of the `B` slots. The opened set must be exactly `{(a, c)}` plus
/// the dummy encoding (two unbound cells).
///
/// Asserted by comparing the candidates' OWN payload rows against what each slot
/// actually opened to, through `exact_k_chain_candidates_and_opened_rows` — the
/// same candidate construction and the same output stage the public path runs.
#[test]
fn rejected_two_hop_pairs_payloads_open_to_the_dummy_encoding() {
    let render = |row: &[Option<Term>]| format!("{row:?}");
    for seed in 0..8u64 {
        let backend = ShamirBackend::new_seeded(3, 0x9A80 + seed).unwrap();
        let (edges, _) = hidden_edges(&PIN_EDGES);
        let (candidates, slots) =
            exact_k_chain_candidates_and_opened_rows(&mut backend.dealer(), &backend, &edges, 2, 6)
                .unwrap();
        assert_eq!(slots.len(), 6, "seed {seed}: exactly B slots");
        let connected = vec![Some(iri_term("http://ex/a")), Some(iri_term("http://ex/c"))];
        let pad: Vec<Option<Term>> = vec![None, None];
        // Every candidate whose endpoint row is not the connected pair is rejected;
        // its row must not be among the opened payloads.
        let opened: Vec<String> = slots.iter().map(|(_, r)| render(r)).collect();
        let mut rejected_seen = 0usize;
        for c in &candidates {
            if c.payload != connected {
                rejected_seen += 1;
                assert!(
                    !opened.contains(&render(&c.payload)),
                    "seed {seed}: a rejected pair's endpoint row was opened: {}",
                    render(&c.payload)
                );
            }
        }
        assert!(
            rejected_seen > 0,
            "seed {seed}: fixture must contain rejected pairs"
        );
        // The opened set is exactly the connected pair plus the dummy encoding.
        let distinct: BTreeSet<String> = opened.iter().cloned().collect();
        assert_eq!(
            distinct,
            BTreeSet::from([render(&connected), render(&pad)]),
            "seed {seed}: opened payload set"
        );
        for (is_real, row) in &slots {
            if *is_real {
                assert_eq!(row, &connected, "seed {seed}");
            } else {
                assert_eq!(row, &pad, "seed {seed}: dummy slot payload");
            }
        }
    }
}

/// The COMPLETE draw sequence of the exactly-2 chain stage (entry count and an
/// order-sensitive fingerprint over every method and returned `y`).
///
/// **This is the #6117-step-6 guard.** The transcript now ends in the output
/// stage's 141 `vss_own_input` draws and its `oblivious_shuffle`: if
/// `eval_exact_k_chain_hidden_slots_through` went back to minting its own dealer
/// for the output stage the log would stop at 1920 entries and both the length and
/// the suffix assertion would fail.
///
/// The 1920-entry PREFIX is the pre-routing whole-log pin
/// (`0x1013_9382_581b_a7e3`, captured on the pre-routing code), asserted here
/// unchanged: the routing appended an output-stage SUFFIX and moved nothing in the
/// chain part, and the output stage's per-element degree reduction only grew that
/// suffix. The full-log fingerprint is the new pin.
#[test]
fn seeded_exact_k_stage_transcript_is_pinned() {
    use crate::randomness::test_sources::{
        fingerprint, prefix_before_first_shuffle, RecordingSource,
    };
    let backend = ShamirBackend::new_seeded(3, 0x9A7B).unwrap();
    let (edges, _) = hidden_edges(&PIN_EDGES);
    let mut rec = RecordingSource::new(backend.dealer());
    eval_exact_k_chain_hidden_slots_through(&mut rec, &backend, &edges, 2, 5).unwrap();
    assert_eq!(rec.log.len(), 2062);
    // Prefix unchanged from the pre-routing pin; the whole move is in the output
    // suffix, which grew by the per-element degree reductions.
    assert_eq!(fingerprint(&rec.log[..1920]), 0x1013_9382_581b_a7e3);
    // The draws BEFORE the shuffle are byte-for-byte unchanged: this prefix
    // fingerprint is the value the pre-re-randomisation code produced, so only
    // the shuffle's own output and the draws after it moved.
    assert_eq!(
        fingerprint(prefix_before_first_shuffle(&rec.log)),
        0x57c8_25d3_c07f_3fe9
    );
    assert_eq!(fingerprint(&rec.log), 0x33fc_792d_1e0b_566a);
    let suffix: Vec<&str> = rec.log[1920..].iter().map(|(m, _)| *m).collect();
    assert_eq!(
        suffix,
        [["vss_own_input"; 141].as_slice(), &["oblivious_shuffle"]].concat()
    );
}

/// The two caps are pinned by VALUE, not only by their relation.
///
/// `MAX_HOP_COUNT` is DEFINED as `MAX_CHAIN_TUPLES.ilog2()`, so asserting that
/// relation would be a tautology that passes however either moves. What is
/// load-bearing is the numbers themselves: a reviewer reading a cost argument
/// about "18 hops" and "2^18 tuples" should find that claim pinned somewhere, and
/// changing either constant should make a test say so rather than quietly
/// rescaling every bound in the module.
#[test]
fn the_hop_and_tuple_caps_are_pinned_by_value() {
    use crate::hidden_path::{MAX_CHAIN_TUPLES, MAX_HOP_COUNT};
    assert_eq!(MAX_HOP_COUNT, 18, "the documented hop cap");
    assert_eq!(MAX_CHAIN_TUPLES, 1u64 << 18, "the documented tuple cap");
    // And the relation the definition expresses, so a future hand-written value
    // cannot drift from it.
    assert_eq!(MAX_HOP_COUNT, MAX_CHAIN_TUPLES.ilog2() as usize);
}

/// Both exactly-`k` entries — the public slot path and the test-only instrumented
/// one — refuse an over-cap output bound before ANY draw, on a NON-EMPTY edge set.
/// `build_candidates` draws per hop equality and the output stage materialises `B`
/// slots, so the refusal has to precede both.
#[test]
fn an_over_cap_bound_is_refused_before_any_draw() {
    use crate::oblivious_join::MAX_OUTPUT_BOUND;
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x7C01).unwrap();
    let (edges, _interner) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
        ("http://ex/c", "http://ex/d"),
    ]);

    for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = eval_exact_k_chain_hidden_slots_through(&mut counted, &backend, &edges, 2, bound)
            .expect_err("an over-cap bound must be refused");
        match &err {
            MpcError::Protocol(m) => assert!(m.contains("MAX_OUTPUT_BOUND"), "B={}: {}", bound, m),
            other => panic!("B={}: expected a Protocol refusal, got {:?}", bound, other),
        }
        assert_eq!(counted.draws(), 0, "slots_through B={bound}");

        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err =
            exact_k_chain_candidates_and_opened_rows(&mut counted, &backend, &edges, 2, bound)
                .expect_err("the instrumented entry must refuse it too");
        match &err {
            MpcError::Protocol(m) => assert!(m.contains("MAX_OUTPUT_BOUND"), "B={}: {}", bound, m),
            other => panic!("B={}: expected a Protocol refusal, got {:?}", bound, other),
        }
        assert_eq!(counted.draws(), 0, "candidates_and_opened_rows B={bound}");
    }

    // A legal bound over the same edges still runs.
    let legal = eval_exact_k_chain_hidden(&backend, &edges, 2, 16);
    assert!(legal.is_ok(), "a legal bound must still run: {:?}", legal);
}

/// The OTHER half of the bound contract — `B` covering the candidate count — is
/// also settled before any draw.
///
/// This is the half that used to be left to the output stage: with `|E| = 2` and
/// `k = 18` the builder drew one secure equality per hop of every one of the
/// `2^18` tuples, about 2.1 billion draws over roughly seventeen minutes, and only
/// THEN discovered that `B = 1` could not hold the answer. The count is decidable
/// from the disclosed endpoint terms in `O(|E| log |E|)`, so it is now decided
/// first. A `k = 18` fixture is used deliberately: with the guard in place the test
/// finishes instantly, and without it the enumeration is the cost being avoided.
#[test]
fn a_bound_below_the_candidate_count_is_refused_before_any_draw() {
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x7C07).unwrap();
    // Two edges over three nodes: 2 distinct subjects x 2 distinct objects = 4
    // endpoint pairs for any k >= 2, so B = 1 cannot cover it.
    let (edges, _interner) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ]);
    assert_eq!(
        projected_candidate_count(&edges, 18).unwrap(),
        4,
        "the projection must agree with the builder's grouping"
    );

    for k in [2usize, 18] {
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = eval_exact_k_chain_hidden_slots_through(&mut counted, &backend, &edges, k, 1)
            .expect_err("a bound below the candidate count must be refused");
        match &err {
            MpcError::Protocol(m) => {
                assert!(
                    m.contains("does not cover the candidate set"),
                    "k={}: {}",
                    k,
                    m
                );
                // The message names B and the PUBLIC bound, and no candidate count.
                assert!(
                    m.contains("B=1") && m.contains("public candidate bound 4"),
                    "k={}: {}",
                    k,
                    m
                );
                assert!(!m.contains("count 4"), "k={}: {}", k, m);
            }
            other => panic!("k={}: expected a Protocol refusal, got {:?}", k, other),
        }
        assert_eq!(counted.draws(), 0, "k={}: refused before any draw", k);
    }

    // k = 1 groups by the edges' own (subject, object) pairs, so the count is 2 and
    // B = 1 is still short — the projection must track `k`, not assume the product.
    assert_eq!(projected_candidate_count(&edges, 1).unwrap(), 2);
    let mut dealer = backend.dealer();
    let mut counted = DrawCountingSource::new(&mut dealer);
    assert!(
        eval_exact_k_chain_hidden_slots_through(&mut counted, &backend, &edges, 1, 1).is_err(),
        "B=1 below the k=1 count must be refused"
    );
    assert_eq!(counted.draws(), 0);

    // And the projection is EXACT, not merely an upper bound: a bound equal to it
    // runs, for both the k = 1 and the k >= 2 grouping.
    eval_exact_k_chain_hidden(&backend, &edges, 1, 2).expect("B = exact count runs at k=1");
    eval_exact_k_chain_hidden(&backend, &edges, 2, 4).expect("B = exact count runs at k=2");

    // The test-only instrumentation entry carries the SAME bound contract, so a
    // short `B` cannot reach the builder through it either.
    let mut dealer = backend.dealer();
    let mut counted = DrawCountingSource::new(&mut dealer);
    assert!(
        exact_k_chain_candidates_and_opened_rows(&mut counted, &backend, &edges, 2, 1).is_err(),
        "the instrumentation entry must refuse a short B too"
    );
    assert_eq!(counted.draws(), 0);
}

/// `public_candidate_pair_bound` is the value a caller provisions `B` against: it
/// is a function of the edge COUNT and `k`, and of nothing in the edges.
#[test]
fn the_public_candidate_pair_bound_is_a_function_of_the_public_sizes() {
    // k = 1 keys at most one pair per edge.
    assert_eq!(public_candidate_pair_bound(7, 1).unwrap(), 7);
    // k >= 2 is the product of the two endpoint positions, capped by |E|^k.
    assert_eq!(public_candidate_pair_bound(7, 2).unwrap(), 49);
    assert_eq!(public_candidate_pair_bound(7, 9).unwrap(), 49);
    // |E|^k is the tighter bound while it is below |E|^2 — it never is for k >= 2,
    // but k = 2 over two edges shows the two agreeing.
    assert_eq!(public_candidate_pair_bound(2, 2).unwrap(), 4);
    // Degenerate inputs produce no candidates at all.
    assert_eq!(public_candidate_pair_bound(0, 5).unwrap(), 0);
    assert_eq!(public_candidate_pair_bound(5, 0).unwrap(), 0);
    // `|E|^2` overflowing `usize` is a refusal, not a wrap.
    assert!(public_candidate_pair_bound(usize::MAX, 2).is_err());

    // Two edge sets of the SAME size but different contents give the same bound —
    // that is the property that makes a decision taken on it data-independent.
    let (a, _ia) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/c", "http://ex/d"),
    ]);
    let (b, _ib) = hidden_edges(&[
        ("http://ex/x", "http://ex/x"),
        ("http://ex/x", "http://ex/x"),
    ]);
    assert_eq!(
        public_candidate_pair_bound(a.len(), 3).unwrap(),
        public_candidate_pair_bound(b.len(), 3).unwrap()
    );
    // Their EXACT counts differ (4 pairs vs 1), which is exactly why the public
    // bound is what the decision is taken on.
    assert_ne!(
        projected_candidate_count(&a, 3).unwrap(),
        projected_candidate_count(&b, 3).unwrap()
    );
}

/// The two branches of the exactly-`k` bound contract, at the entry point.
///
/// A `B` at or above the public bound is accepted whatever the edges are; a `B`
/// below it is accepted when the real count fits and refused when it does not —
/// after the data-dependent projection, but before any build and before any draw.
/// The second branch is the one whose outcome discloses whether the distinct
/// endpoint-pair count is at most `B`, and it exists so that a caller choosing a
/// tight `B` is not refused work the data allows. Neither branch hides that count
/// from a successful run; see the module docs for who learns what.
#[test]
fn both_branches_of_the_exact_k_bound_contract() {
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x7C09).unwrap();
    // One edge pair, a self-loop and a chain: 2 distinct subjects x 2 distinct
    // objects = 4 pairs at k >= 2, while the public bound is |E|^2 = 4 as well, so
    // a second fixture is used below for the strictly-tighter case.
    let (edges, _interner) = hidden_edges(&[
        ("http://ex/a", "http://ex/b"),
        ("http://ex/b", "http://ex/c"),
    ]);
    let public = public_candidate_pair_bound(edges.len(), 2).unwrap();
    assert_eq!(public, 4);

    // Branch 1: B at the public bound, and above it — accepted.
    eval_exact_k_chain_hidden(&backend, &edges, 2, public).expect("B = the public bound runs");
    eval_exact_k_chain_hidden(&backend, &edges, 2, public + 3)
        .expect("B above the public bound runs");

    // Branch 2: a strictly tighter B than the public bound, with a count that
    // FITS — must be accepted, not refused. Three parallel edges share one subject
    // and one object, so k >= 2 yields a single pair while the public bound is 9.
    let (narrow, _i2) = hidden_edges(&[
        ("http://ex/s", "http://ex/o"),
        ("http://ex/s", "http://ex/o"),
        ("http://ex/s", "http://ex/o"),
    ]);
    let narrow_public = public_candidate_pair_bound(narrow.len(), 2).unwrap();
    assert_eq!(narrow_public, 9);
    assert_eq!(projected_candidate_count(&narrow, 2).unwrap(), 1);
    eval_exact_k_chain_hidden(&backend, &narrow, 2, 1)
        .expect("a B far below the public bound runs when the real count fits");

    // Branch 2, the other way: the count does not fit, so it is refused before any
    // draw, with a message carrying only B and the public bound.
    let mut dealer = backend.dealer();
    let mut counted = DrawCountingSource::new(&mut dealer);
    let err = eval_exact_k_chain_hidden_slots_through(&mut counted, &backend, &edges, 2, 3)
        .expect_err("B below both the public bound and the count must be refused");
    assert_eq!(counted.draws(), 0, "refused before any draw");
    match &err {
        MpcError::Protocol(m) => {
            assert!(
                m.contains("B=3") && m.contains("public candidate bound 4"),
                "{}",
                m
            );
            assert!(
                !m.contains("count 4"),
                "the count itself is not rendered: {}",
                m
            );
        }
        other => panic!("expected a Protocol refusal, got {:?}", other),
    }
}

/// Build `n` predicated edges `(s_i -> o_i)`, all under one predicate.
fn predicated_chain(n: usize, predicate: &str) -> PredicatedEdges {
    let mut interner = Interner::new();
    let mut edges = Vec::with_capacity(n);
    for i in 0..n {
        let s = format!("http://ex/s{}", i);
        let o = format!("http://ex/o{}", i);
        edges.push(PredicatedEdge {
            predicate: predicate.to_string(),
            subject: HiddenNode {
                key: interner.key(&s),
                term: iri_term(&s),
            },
            object: HiddenNode {
                key: interner.key(&o),
                term: iri_term(&o),
            },
        });
    }
    // Each edge contributes two fresh nodes, so the active graph's node domain is
    // exactly `2n` terms. Declared explicitly, as a `{0,k}` form requires.
    let domain: Vec<Term> = (0..n)
        .flat_map(|i| {
            [
                iri_term(&format!("http://ex/s{}", i)),
                iri_term(&format!("http://ex/o{}", i)),
            ]
        })
        .collect();
    PredicatedEdges::new(edges)
        .with_node_domain(domain)
        .expect("the declared domain covers every edge endpoint by construction")
}

/// The bounded entry's PUBLIC bound can exceed `MAX_OUTPUT_BOUND` for inputs
/// `check_bounded_params` ACCEPTS — the public bound sums the arms, while `B` is
/// capped. For those shapes no admissible `B` reaches the public bound, so the
/// caller has only the data-dependent branch. Two representative shapes, on a
/// fixture whose declared node domain is exactly `2|E|`.
#[test]
fn the_public_bounded_bound_can_exceed_every_admissible_bound() {
    use crate::oblivious_join::MAX_OUTPUT_BOUND;

    let backend = ShamirBackend::new_seeded(5, 0x51A3).unwrap();

    // `{0,2}` over 511 edges: |V| = 2|E| diagonal + |E| at length 1 + |E|^2 at
    // length 2.
    let two = HiddenBoundedPath::range("http://ex/p", 0, 2);
    let edges = predicated_chain(511, "http://ex/p");
    check_bounded_params(&backend, &edges, &two).expect("{0,2} over 511 edges is accepted");
    let public =
        public_bounded_candidate_pair_bound(edges.len(), edges.declared_domain_len(), &two)
            .unwrap();
    assert_eq!(public, 2 * 511 + 511 + 511 * 511);
    assert!(
        public > MAX_OUTPUT_BOUND,
        "the public bound {} must be out of B's range {}",
        public,
        MAX_OUTPUT_BOUND
    );

    // `{0,1}`: the |V| = 2|E| diagonal plus length 1 is already `3·|E|` on this
    // fixture, so the public bound leaves `B`'s range at `|E| = ceil((B+1)/3)`.
    let one = HiddenBoundedPath::range("http://ex/p", 0, 1);
    let over = MAX_OUTPUT_BOUND / 3 + 1;
    let wide = predicated_chain(over, "http://ex/p");
    check_bounded_params(&backend, &wide, &one).expect("{0,1} over the fixture is accepted");
    let public_one =
        public_bounded_candidate_pair_bound(wide.len(), wide.declared_domain_len(), &one).unwrap();
    assert_eq!(public_one, 3 * 87_382);
    assert!(
        public_one > MAX_OUTPUT_BOUND,
        "the public bound {} must be out of B's range {}",
        public_one,
        MAX_OUTPUT_BOUND
    );
    // One edge fewer still fits, so on this `|V| = 2|E|` fixture `over` is where the
    // reach is actually lost.
    assert!(
        public_bounded_candidate_pair_bound(over - 1, 2 * (over - 1), &one).unwrap()
            <= MAX_OUTPUT_BOUND
    );
}

/// A DRAW-COUNT regression on the fold count, and nothing wider.
///
/// What it inspects is one projection of a whole call: the sequence of
/// `DistributedRandomness` calls, each reduced to its seam method and its width.
/// That projection is coarse on purpose and is NOT an operation or communication
/// schedule — sharing an input bit and resharing a multiplication both surface as
/// `vss_own_input`, so two runs with equal draw sequences can still differ in which
/// operation each draw served and in what a transport would carry.
///
/// Within that projection: fixtures agreeing on the public sizes, the distinct
/// endpoint-pair count `D`, the payload width and the (empty) result are equal
/// whatever their nodes are, while a fixture with a smaller `D` is longer, because
/// every repeated pair costs one extra secure OR. So the projection tracks `T − D`,
/// hence `D`.
///
/// It does not separate the fold POSITIONS, and the equality on `folds_late` below
/// says so. That is a statement about this projection only: the positions are
/// disclosed elsewhere — the grouping reads every tuple's endpoint pair in the
/// clear, and a distributed implementation that supplies every evaluating party with
/// those ordered plaintext terms, or with this construction's data-dependent
/// accumulator wiring, lets every such party determine the positions. This crate
/// implements a single-process simulation and does not establish a per-party input
/// or wiring-distribution boundary, as the module docs set out.
#[test]
fn the_draw_method_count_projection_tracks_the_fold_count() {
    use crate::randomness::test_sources::RecordingSource;

    // The method/count projection of one whole call's randomness calls: each
    // draw's seam method and its width. The drawn VALUES are deliberately excluded
    // — they are sharings of the secrets, so they differ between any two distinct
    // inputs and would say nothing about the call sequence. The seam method does
    // not identify the OPERATION either (see the doc comment), which is why this is
    // a draw-count regression and not a schedule comparison.
    fn projection(pairs: &[(&str, &str)]) -> Vec<(&'static str, usize)> {
        let backend = ShamirBackend::new_seeded(5, 0x5C4E).unwrap();
        let (edges, _interner) = hidden_edges(pairs);
        let mut recorder = RecordingSource::new(backend.dealer());
        // |E| = 3, k = 2: T = 9 tuples, and B = 9 is the public bound, so the bound
        // decision takes the public branch and no exact count is computed.
        let (slots, _cost) =
            eval_exact_k_chain_hidden_slots_through(&mut recorder, &backend, &edges, 2, 9)
                .expect("the fixtures are legal");
        // Every fixture below has an empty result, so the transcripts being compared
        // belong to calls that disclose the same answer.
        assert!(
            partial_result_from_slots(slots, chain_out_vars())
                .rows
                .is_empty(),
            "fixture {:?} must have an empty result for the comparison to be about the \
             projection",
            pairs
        );
        recorder.log.iter().map(|(m, v)| (*m, v.len())).collect()
    }

    // Subjects (a, a, d) x objects (b, c, b): D = 4 distinct pairs, so 5 of the 9
    // tuples fold — at positions 3, 4, 5, 6 and 9.
    let folds_early = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/a", "http://ex/c"),
        ("http://ex/d", "http://ex/b"),
    ];
    // The same shape over four different nodes, every IRI the same length so the
    // payload width is unchanged.
    let folds_early_relabelled = [
        ("http://ex/p", "http://ex/q"),
        ("http://ex/p", "http://ex/r"),
        ("http://ex/s", "http://ex/q"),
    ];
    // Same |E|, same D = 4, same width, same empty result — but the repeats fall at
    // positions 3, 6, 7, 8 and 9 instead.
    let folds_late = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/d", "http://ex/c"),
        ("http://ex/a", "http://ex/b"),
    ];
    // Same |E|, same width, same empty result, but objects (b, b, b) leave only
    // D = 2 distinct pairs, so 7 tuples fold instead of 5.
    let fewer_pairs = [
        ("http://ex/a", "http://ex/b"),
        ("http://ex/a", "http://ex/b"),
        ("http://ex/d", "http://ex/b"),
    ];

    let early = projection(&folds_early);
    let relabelled = projection(&folds_early_relabelled);
    let late = projection(&folds_late);
    let fewer = projection(&fewer_pairs);

    assert_eq!(
        early, relabelled,
        "fixtures agreeing on the public sizes, D, the width and the result must have the same \
         draw projection"
    );
    assert_eq!(
        early, late,
        "this projection does not separate the fold positions — if this ever differs, it \
         carries more than the docs say and they must be widened"
    );
    assert_ne!(
        early, fewer,
        "a different D must change the draw projection: that is the count the module docs \
         attribute to it"
    );
    assert!(
        fewer.len() > early.len(),
        "fewer distinct pairs means more folds, so more draws: {} vs {}",
        fewer.len(),
        early.len()
    );
}
