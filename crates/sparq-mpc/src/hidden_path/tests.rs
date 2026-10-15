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
    // The REVEALED slot ORDER moved in #6117 step 6 (it was
    // ["dummy", "dummy", "<http://ex/a> <http://ex/c>", "dummy", "dummy"]): the
    // output stage now draws its sharings and its shuffle from the SAME source as
    // the chain stage instead of minting a second dealer, so the shuffle samples a
    // different point of one continuous keystream. The order is the shuffle's
    // secret permutation and carries no result meaning — the canonicalised
    // multiset below is byte-identical to the pre-routing capture.
    let (slots, cost) = eval_exact_k_chain_hidden_slots(&backend, &edges, 2, 5).unwrap();
    assert_eq!(
        slot_strings(&slots),
        [
            "dummy",
            "dummy",
            "dummy",
            "<http://ex/a> <http://ex/c>",
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
/// stage's 36 `vss_own_input` draws and its `oblivious_shuffle`: if
/// `eval_exact_k_chain_hidden_slots_through` went back to minting its own dealer
/// for the output stage the log would stop at 1920 entries and both the length and
/// the suffix assertion would fail.
///
/// The 1920-entry PREFIX is the pre-routing whole-log pin
/// (`0x1013_9382_581b_a7e3`, captured on the pre-routing code), asserted here
/// unchanged: the routing appended an output-stage SUFFIX and moved nothing in the
/// chain part. The full-log fingerprint is the new pin.
#[test]
fn seeded_exact_k_stage_transcript_is_pinned() {
    use crate::randomness::test_sources::{fingerprint, RecordingSource};
    let backend = ShamirBackend::new_seeded(3, 0x9A7B).unwrap();
    let (edges, _) = hidden_edges(&PIN_EDGES);
    let mut rec = RecordingSource::new(backend.dealer());
    eval_exact_k_chain_hidden_slots_through(&mut rec, &backend, &edges, 2, 5).unwrap();
    assert_eq!(rec.log.len(), 1957);
    // Prefix unchanged from the pre-routing pin; suffix is the new output stage.
    assert_eq!(fingerprint(&rec.log[..1920]), 0x1013_9382_581b_a7e3);
    assert_eq!(fingerprint(&rec.log), 0xbea3_3cb0_9754_016e);
    let suffix: Vec<&str> = rec.log[1920..].iter().map(|(m, _)| *m).collect();
    assert_eq!(
        suffix,
        [["vss_own_input"; 36].as_slice(), &["oblivious_shuffle"]].concat()
    );
}
