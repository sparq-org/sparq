// sq-py8h.3 — DIFFERENTIAL tests for the HIDDEN-intermediate BOUNDED
// property-path operator ({1,k} / {0,k} / p? + alternation, OR-fold length-dedup).
//! Differential acceptance suite for [`eval_bounded_path_hidden`].
//!
//! THE acceptance criterion (bead sq-py8h.3): the secure HIDDEN bounded path
//! (`{1,k}`, `{0,k}`, `p?`, alternation) must equal the **plaintext** bounded-path
//! evaluation over the same edge set; each connected endpoint pair returned exactly
//! once; the realized length is never opened; the reflexive diagonal is correct for
//! `{0,k}` / `p?`.
//!
//! The differential ORACLE is a pure plaintext bounded-path reachability (the union
//! of exactly-ℓ chains for ℓ in `min..=max`, plus the reflexive `(x,x)` diagonal when
//! `min == 0`, deduped to a set), computed in this file with NO MPC, so the equality
//! witnesses the crypto path against an independent computation. The secure path's
//! per-pair connected-bit and per-hop / per-length match bits are never opened; only
//! the `B` shuffled output tags are.

use super::*;
use crate::field::Fp;
use crate::partial::HolderId;
use crate::shamir::ShamirBackend;
use oxrdf::{NamedNode, Term};
use std::collections::{BTreeSet, HashMap};

fn iri_term(iri: &str) -> Term {
    Term::NamedNode(NamedNode::new(iri).unwrap())
}

/// A tiny in-test node interner: distinct node IRIs → distinct nonzero `Fp` keys
/// (the holder's private-key encoding stand-in). The interner is SHARED across all
/// predicates so the same node has the same key everywhere — exactly the federated
/// injective-encoding contract.
struct Interner {
    by_iri: HashMap<String, Fp>,
    next: u64,
}
impl Interner {
    fn new() -> Self {
        Interner {
            by_iri: HashMap::new(),
            next: 1,
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
    fn node(&mut self, iri: &str) -> HiddenNode {
        HiddenNode {
            key: self.key(iri),
            term: iri_term(iri),
        }
    }
}

/// Build a [`PredicatedEdges`] from `(predicate, subject, object)` triples, interning
/// node IRIs to nonzero `Fp` keys (shared node namespace across predicates).
fn predicated_edges(triples: &[(&str, &str, &str)]) -> PredicatedEdges {
    let mut interner = Interner::new();
    let mut edges: Vec<PredicatedEdge> = Vec::new();
    for (p, s, o) in triples {
        edges.push(PredicatedEdge {
            predicate: (*p).to_string(),
            subject: interner.node(s),
            object: interner.node(o),
        });
    }
    PredicatedEdges::new(edges)
}

/// PLAINTEXT differential oracle: the SET of `(a,b)` IRI pairs connected by the
/// bounded path `(alternatives){min,max}` over the labelled edge set, with NO MPC.
/// A hop of length-ℓ chain may use ANY of `alternatives` at each position; the
/// reflexive `(x,x)` diagonal is added when `min == 0` over the node domain.
fn plaintext_bounded(
    triples: &[(&str, &str, &str)],
    alternatives: &[&str],
    min: usize,
    max: usize,
) -> BTreeSet<(String, String)> {
    let mut out: BTreeSet<(String, String)> = BTreeSet::new();

    // Reflexive length-0 diagonal.
    if min == 0 {
        let mut nodes: BTreeSet<String> = BTreeSet::new();
        for (_, s, o) in triples {
            nodes.insert((*s).to_string());
            nodes.insert((*o).to_string());
        }
        for n in nodes {
            out.insert((n.clone(), n));
        }
    }

    // Edges restricted to the alternation predicate set.
    let alt: BTreeSet<&str> = alternatives.iter().copied().collect();
    let edges: Vec<(String, String)> = triples
        .iter()
        .filter(|(p, _, _)| alt.contains(p))
        .map(|(_, s, o)| (s.to_string(), o.to_string()))
        .collect();

    // Union of exactly-ℓ reachability for ℓ in max(min,1)..=max.
    let lo = min.max(1);
    if lo <= max && !edges.is_empty() {
        // length-1 = the (restricted) edges themselves.
        let mut current: BTreeSet<(String, String)> = edges.iter().cloned().collect();
        for length in 1..=max {
            if length >= lo {
                out.extend(current.iter().cloned());
            }
            if length == max {
                break;
            }
            // Extend by one hop.
            let mut next: BTreeSet<(String, String)> = BTreeSet::new();
            for (a, mid) in &current {
                for (s, o) in &edges {
                    if s == mid {
                        next.insert((a.clone(), o.clone()));
                    }
                }
            }
            current = next;
        }
    }
    out
}

/// Run the SECURE hidden bounded path and return its disclosed `(a,b)` IRI pair set.
fn secure_bounded(
    backend: &ShamirBackend,
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
    bound: usize,
) -> BTreeSet<(String, String)> {
    let result = eval_bounded_path_hidden(backend, edges, form, bound).unwrap();
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

const P: &str = "http://ex/p";
const Q: &str = "http://ex/q";

// =====================================================================
// {1,k} bounded `+` — union of length-1..k chains, deduped
// =====================================================================

/// `(p){1,3}`: chain a->b->c->d. Connected pairs at lengths 1,2,3 unioned & deduped:
/// (a,b),(b,c),(c,d) [len1]; (a,c),(b,d) [len2]; (a,d) [len3].
#[test]
fn differential_one_to_k_equals_plaintext() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (P, "http://ex/c", "http://ex/d"),
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0xB0).unwrap();
    let form = HiddenBoundedPath::range(P, 1, 3);
    let want = plaintext_bounded(&triples, &[P], 1, 3);
    let got = secure_bounded(&backend, &edges, &form, 64);
    assert_eq!(got, want, "secure (p){{1,3}} != plaintext");
    assert_eq!(got.len(), 6, "{got:?}");
}

/// OR-fold length-dedup: a pair reachable at TWO different lengths appears ONCE.
/// In a 2-cycle a<->b, `(p){1,2}` reaches (a,b) at len-1 AND (a,b) at... no — but
/// (a,a) at len-2 and (a,b) at len-1; build a graph where (a,c) is reachable by both
/// a len-2 (a->b->c) and a len-3 (a->x->y->c) so {2,3} would double-count without the
/// OR-fold.
#[test]
fn differential_or_fold_collapses_multi_length_pair() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"), // a->b->c  (len 2)
        (P, "http://ex/a", "http://ex/x"),
        (P, "http://ex/x", "http://ex/y"),
        (P, "http://ex/y", "http://ex/c"), // a->x->y->c (len 3)
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0xC1).unwrap();
    let form = HiddenBoundedPath::range(P, 2, 3);
    let want = plaintext_bounded(&triples, &[P], 2, 3);
    let got = secure_bounded(&backend, &edges, &form, 64);
    assert_eq!(got, want);
    // (a,c) reachable by both len-2 and len-3 → present exactly once.
    let ac = ("http://ex/a".to_string(), "http://ex/c".to_string());
    assert!(got.contains(&ac));
    assert_eq!(
        got.iter().filter(|p| **p == ac).count(),
        1,
        "(a,c) must appear exactly once despite two connecting lengths"
    );
}

// =====================================================================
// {0,k} bounded `*` + p? — reflexive diagonal correct
// =====================================================================

/// `(p){0,2}`: the {1,2} union PLUS the reflexive (x,x) for every node.
#[test]
fn differential_zero_to_k_includes_reflexive_diagonal() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0xD2).unwrap();
    let form = HiddenBoundedPath::range(P, 0, 2);
    let want = plaintext_bounded(&triples, &[P], 0, 2);
    let got = secure_bounded(&backend, &edges, &form, 64);
    assert_eq!(got, want, "secure (p){{0,2}} != plaintext");
    // Reflexive pairs for a,b,c present.
    for n in ["http://ex/a", "http://ex/b", "http://ex/c"] {
        assert!(
            got.contains(&(n.to_string(), n.to_string())),
            "reflexive ({n},{n}) missing"
        );
    }
    // And the real chains: (a,b),(b,c) [len1], (a,c) [len2].
    assert!(got.contains(&("http://ex/a".into(), "http://ex/c".into())));
}

/// `p?` = `{0,1}`: reflexive diagonal plus the one-hop edges.
#[test]
fn differential_optional_equals_zero_or_one() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/c", "http://ex/d"),
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0xE3).unwrap();
    let form = HiddenBoundedPath::optional(P);
    let want = plaintext_bounded(&triples, &[P], 0, 1);
    let got = secure_bounded(&backend, &edges, &form, 32);
    assert_eq!(got, want, "secure p? != plaintext {{0,1}}");
    // 4 reflexive (a,b,c,d) + 2 edges (a,b),(c,d) = 6.
    assert_eq!(got.len(), 6, "{got:?}");
}

/// A node reachable from itself by a longer chain (a 2-cycle a<->b: (a,a) at len-2)
/// AND by the reflexive arm must appear exactly once in `{0,2}`.
#[test]
fn differential_reflexive_dedups_with_cycle() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/a"),
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0xF4).unwrap();
    let form = HiddenBoundedPath::range(P, 0, 2);
    let want = plaintext_bounded(&triples, &[P], 0, 2);
    let got = secure_bounded(&backend, &edges, &form, 32);
    assert_eq!(got, want);
    let aa = ("http://ex/a".to_string(), "http://ex/a".to_string());
    assert!(got.contains(&aa));
    assert_eq!(
        got.iter().filter(|p| **p == aa).count(),
        1,
        "(a,a) reached by reflexive AND by the len-2 cycle must appear once"
    );
}

// =====================================================================
// alternation (p|q){m,k} — union-of-fixed-chains over branches
// =====================================================================

/// `(p|q){1,2}`: a mixed graph where some hops are `p` and some are `q`. The secure
/// result must equal the plaintext bounded path over the UNION of p and q edges.
#[test]
fn differential_alternation_equals_plaintext() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (Q, "http://ex/b", "http://ex/c"), // a -p-> b -q-> c  (len-2 mixed)
        (Q, "http://ex/a", "http://ex/d"),
        (P, "http://ex/d", "http://ex/e"), // a -q-> d -p-> e  (len-2 mixed)
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0x1A).unwrap();
    let form = HiddenBoundedPath::alternation([P, Q], 1, 2);
    let want = plaintext_bounded(&triples, &[P, Q], 1, 2);
    let got = secure_bounded(&backend, &edges, &form, 64);
    assert_eq!(got, want, "secure (p|q){{1,2}} != plaintext");
    // The mixed len-2 chains must connect (a,c) and (a,e).
    assert!(got.contains(&("http://ex/a".into(), "http://ex/c".into())));
    assert!(got.contains(&("http://ex/a".into(), "http://ex/e".into())));
}

/// Alternation must NOT cross into a predicate outside the branch set: a `q`-only
/// edge is invisible to `(p){1,2}`.
#[test]
fn differential_alternation_excludes_other_predicates() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (Q, "http://ex/b", "http://ex/c"), // q hop — NOT in (p){...}
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0x2B).unwrap();
    let form = HiddenBoundedPath::range(P, 1, 2);
    let want = plaintext_bounded(&triples, &[P], 1, 2);
    let got = secure_bounded(&backend, &edges, &form, 32);
    assert_eq!(got, want);
    // Only the single p-edge (a,b) — no (a,c) via the q hop.
    assert!(got.contains(&("http://ex/a".into(), "http://ex/b".into())));
    assert!(!got.contains(&("http://ex/a".into(), "http://ex/c".into())));
    assert_eq!(got.len(), 1, "{got:?}");
}

// =====================================================================
// Cross-check: bounded {k,k} == the exactly-k operator (sq-py8h.2)
// =====================================================================

/// `(p){2,2}` over the union machinery must agree with the dedicated exactly-2 oracle.
#[test]
fn differential_exact_k_via_bounded_matches_plaintext() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (P, "http://ex/b", "http://ex/d"),
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 0x3C).unwrap();
    let form = HiddenBoundedPath::exact(P, 2);
    let want = plaintext_bounded(&triples, &[P], 2, 2);
    let got = secure_bounded(&backend, &edges, &form, 64);
    assert_eq!(got, want);
    // a->b->c and a->b->d.
    assert_eq!(got.len(), 2, "{got:?}");
}

// =====================================================================
// L1 obliviousness + determinism
// =====================================================================

/// L1: two graphs with DIFFERENT true bounded-match counts but the same `B` reveal the
/// SAME number of output slots (the padded bound), not the true cardinality.
#[test]
fn bounded_revealed_slot_count_is_bound_not_cardinality() {
    let many = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (P, "http://ex/b", "http://ex/d"),
        (P, "http://ex/b", "http://ex/e"),
    ];
    let few = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ];
    let e_many = predicated_edges(&many);
    let e_few = predicated_edges(&few);
    let backend = ShamirBackend::new_seeded(3, 5).unwrap();
    let form = HiddenBoundedPath::range(P, 1, 2);
    let (slots_many, cost_many) =
        eval_bounded_path_hidden_slots(&backend, &e_many, &form, 60).unwrap();
    let (slots_few, cost_few) =
        eval_bounded_path_hidden_slots(&backend, &e_few, &form, 60).unwrap();
    assert_eq!(slots_many.len(), slots_few.len(), "slot count must equal B");
    assert_eq!(cost_many.bound, 60);
    assert_eq!(cost_few.bound, 60);
}

/// The disclosed `(a,b)` set is invariant to the shuffle's permutation seed.
#[test]
fn bounded_result_is_shuffle_seed_invariant() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (P, "http://ex/b", "http://ex/d"),
    ];
    let edges = predicated_edges(&triples);
    let form = HiddenBoundedPath::range(P, 0, 2);
    let want = plaintext_bounded(&triples, &[P], 0, 2);
    for seed in 0..6u64 {
        let backend = ShamirBackend::new_seeded(5, 2000 + seed).unwrap();
        let got = secure_bounded(&backend, &edges, &form, 64);
        assert_eq!(got, want, "seed {seed}: result changed");
    }
}

// =====================================================================
// Fail-closed contracts
// =====================================================================

/// `min > max` is rejected.
#[test]
fn bounded_min_gt_max_is_rejected() {
    let edges = predicated_edges(&[(P, "http://ex/a", "http://ex/b")]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let form = HiddenBoundedPath::range(P, 3, 1);
    let err = eval_bounded_path_hidden(&backend, &edges, &form, 8).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(m) if m.contains("min > max")));
}

/// An empty alternation is rejected.
#[test]
fn bounded_empty_alternation_is_rejected() {
    let edges = predicated_edges(&[(P, "http://ex/a", "http://ex/b")]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let form = HiddenBoundedPath {
        alternatives: Vec::new(),
        min: 1,
        max: 2,
    };
    let err = eval_bounded_path_hidden(&backend, &edges, &form, 8).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(m) if m.contains("no predicates")));
}

/// An over-large unroll is rejected fail-closed BEFORE any crypto runs. An upper
/// bound past [`MAX_HOP_COUNT`] is refused by the hop guard (which precedes the
/// tuple accounting, since that accounting's own cost grows with the bound); an
/// upper bound WITHIN the hop cap whose `Σ_ℓ (a^ℓ·|E|^ℓ)` is still too large is
/// refused by [`MAX_CHAIN_TUPLES`].
#[test]
fn bounded_oversized_unroll_is_rejected() {
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (P, "http://ex/c", "http://ex/a"),
    ]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();

    // Upper bound past the hop cap → the hop guard.
    let form = HiddenBoundedPath::range(P, 1, 64);
    let err = eval_bounded_path_hidden(&backend, &edges, &form, 1 << 30).unwrap_err();
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_HOP_COUNT")),
        "expected the hop-count refusal, got {:?}",
        err
    );

    // Upper bound AT the hop cap, so the hop guard passes; Σ_ℓ 3^ℓ over ℓ = 1..=18
    // is far past MAX_CHAIN_TUPLES → the tuple guard.
    let form = HiddenBoundedPath::range(P, 1, MAX_HOP_COUNT);
    let err = eval_bounded_path_hidden(&backend, &edges, &form, 8).unwrap_err();
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_CHAIN_TUPLES")),
        "expected the tuple-count refusal, got {:?}",
        err
    );
}

/// Hop counts are capped before evaluation on the bounded form too: an upper bound
/// past [`MAX_HOP_COUNT`] — up to `usize::MAX`, where `check_bounded_params`'
/// `Σ_ℓ` accumulation is quadratic in the bound — is refused with NO draw made.
/// Finishing the test at all is the no-hang witness; nothing here asserts a timing.
#[test]
fn huge_upper_bound_is_refused_before_any_draw() {
    use crate::randomness::test_sources::DrawCountingSource;
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    for max in [MAX_HOP_COUNT + 1, 262_144, usize::MAX] {
        let form = HiddenBoundedPath::range(P, 1, max);
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, 8)
            .expect_err("an upper bound past the cap must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_HOP_COUNT")),
            "max={}: expected the hop-count refusal, got {:?}",
            max,
            err
        );
        assert_eq!(counted.draws(), 0, "max={}: refused before any draw", max);
    }
}

/// A bound `B` below the candidate count fails closed (never truncates a true match).
#[test]
fn bounded_bound_below_candidate_count_fails_closed() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ];
    let edges = predicated_edges(&triples);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let form = HiddenBoundedPath::range(P, 1, 2);
    // {1,2} yields (a,b),(b,c),(a,c) = 3 pairs; B=1 must fail closed.
    let err = eval_bounded_path_hidden(&backend, &edges, &form, 1).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(_)));
}

/// An empty edge set with `min >= 1` yields nothing; with `min == 0` and no edges the
/// node domain is empty too, so the result is empty (no panic, fail-soft).
#[test]
fn bounded_empty_edges_is_empty() {
    let edges = PredicatedEdges::new(Vec::new());
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let got1 = secure_bounded(&backend, &edges, &HiddenBoundedPath::range(P, 1, 3), 8);
    assert!(got1.is_empty());
    let got0 = secure_bounded(&backend, &edges, &HiddenBoundedPath::range(P, 0, 3), 8);
    assert!(got0.is_empty());
}

// ---- sq-yyro / issue #6117: the union stage draws through the seam ----

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
/// check in `eval_bounded_path_hidden_slots_through`, BEFORE the union stage
/// draws anything.
///
/// The shape matters: `{1,2}` over the two-hop chain at `B = 8` is a shape whose
/// OWN stage draws (every hop equality shares a holder's own input — the reason
/// `prss_fails_closed_on_every_bounded_shape` uses it). `p?` over a single edge
/// draws nothing in its union stage, so with the up-front check deleted the
/// output stage's own `require_source_matches` would still raise the same error
/// — hence the assertions below are on the draw count, not only on the error.
#[test]
fn bounded_through_refuses_a_source_that_does_not_match_the_backend() {
    use crate::randomness::test_sources::{
        DrawCountingSource, MisreportedThresholdSource, RecordingSource,
    };
    let backend = ShamirBackend::new_seeded(3, 0xB0C).unwrap();
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ]);
    let form = HiddenBoundedPath::range(P, 1, 2);

    // Wrong `n`: a dealer from a 5-party backend, recorded so the test can see
    // whether anything was drawn before the refusal.
    let mut rec = RecordingSource::new(ShamirBackend::new_seeded(5, 0xB0C).unwrap().dealer());
    match eval_bounded_path_hidden_slots_through(&mut rec, &backend, &edges, &form, 8) {
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
        match eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, 8) {
            Err(MpcError::Protocol(m)) => assert!(m.contains("but the backend is"), "t={t}: {m}"),
            other => panic!("t={t}: a misreported threshold must be refused, got {other:?}"),
        }
        assert_eq!(counted.draws(), 0, "t={t}: drew before refusing");
    }
}

/// A malformed draw in the union's OR-fold is refused by the degree-reduction
/// re-sharing check. `{0,1}` over a self-loop `a → a` folds the length-1 chain
/// into the reflexive `(a, a)`: one OR, i.e. one degree reduction, so the very
/// first own-input draw is that reduction's first re-sharing.
#[test]
fn malformed_draws_in_the_or_fold_are_refused() {
    use crate::randomness::test_sources::{MalformedSource, SeamMethod, ALL_MALFORMATIONS};
    let backend = ShamirBackend::new_seeded(3, 0xB0D).unwrap();
    let edges = predicated_edges(&[(P, "http://ex/a", "http://ex/a")]);
    let form = HiddenBoundedPath::optional(P);
    for how in ALL_MALFORMATIONS {
        let mut src = MalformedSource::new(backend.dealer(), SeamMethod::VssOwnInput, 0, how);
        match eval_bounded_path_hidden_slots_through(&mut src, &backend, &edges, &form, 2) {
            Err(MpcError::Protocol(m)) => {
                assert!(
                    m.contains("re-sharing that is not the canonical"),
                    "{how:?}: {m}"
                )
            }
            other => panic!("{how:?}: must fail closed, got {other:?}"),
        }
        assert!(src.fired());
    }
}

/// Under PRSS every shape fails closed for any `B > 0`, never panics. `{1,2}` needs
/// a hop equality (a holder's own input shared), which PRSS refuses. Since #6117
/// step 6 routed the output stage through the same source, so does `p?` over a
/// single predicate with no self-loop and no repeated endpoint pair, whose union
/// stage draws nothing: the output stage shares the real-row tag and the payload
/// limbs for each of its `B` slots, so that shape is draw-free only at `B = 0`.
/// (Before the routing that case ran at any `B`, because the output stage minted
/// its own dealer and never touched the caller's source.)
#[test]
fn prss_fails_closed_on_every_bounded_shape() {
    use crate::prss::PrssRandomness;
    let backend = ShamirBackend::new_seeded(3, 0xB0E).unwrap();
    let mut prss = PrssRandomness::with_simulated_setup(3).unwrap();
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ]);
    // The formerly-draw-free shape still evaluates CORRECTLY through the seam on a
    // dealer source — it is the SOURCE that refuses below, not the shape.
    let optional = HiddenBoundedPath::optional(P);
    let (slots, _) = eval_bounded_path_hidden_slots_through(
        &mut backend.dealer(),
        &backend,
        &edges,
        &optional,
        6,
    )
    .unwrap();
    assert_eq!(slots.len(), 6);
    assert_eq!(
        slot_pairs(&slots),
        secure_bounded(&backend, &edges, &optional, 6)
    );

    for (form, bound) in [(optional, 6), (HiddenBoundedPath::range(P, 1, 2), 8)] {
        let outcome =
            eval_bounded_path_hidden_slots_through(&mut prss, &backend, &edges, &form, bound);
        assert!(
            matches!(outcome, Err(MpcError::NotYetImplemented { .. })),
            "{form:?}: PRSS must fail closed, got {outcome:?}"
        );
    }
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

/// A 2-cycle `a ⇄ b` under `{0,2}`: the reflexive diagonal, both length-1 edges
/// and all four length-2 tuples, four of which OR-fold into an existing pair.
fn pin_edges() -> PredicatedEdges {
    predicated_edges(&[
        ("p", "http://ex/a", "http://ex/b"),
        ("p", "http://ex/b", "http://ex/a"),
    ])
}

/// Captured on the PRE-ROUTING code by logging every dealer draw and re-checked
/// byte-identical after the routing: the connected-bit sharings pin the values,
/// the trailing `draw_fp()` WITNESS pins the draw count, and the revealed slot
/// order pins the public output.
#[test]
fn seeded_bounded_stage_draws_exactly_what_the_dealer_drew_before_the_seam() {
    let backend = ShamirBackend::new_seeded(3, 0xB0B).unwrap();
    let edges = pin_edges();
    let form = HiddenBoundedPath::range("p", 0, 2);
    let mut dealer = backend.dealer();
    let candidates = build_bounded_candidates(&backend, &mut dealer, &edges, &form).unwrap();
    let bits: Vec<(Vec<String>, Vec<u64>)> = candidates
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
        .collect();
    let pair = |a: &str, b: &str| vec![format!("<http://ex/{a}>"), format!("<http://ex/{b}>")];
    assert_eq!(
        bits,
        [
            (
                pair("a", "a"),
                vec![319826375870180777, 639652751740361553, 959479127610542329]
            ),
            (
                pair("a", "b"),
                vec![
                    2049686828521023477,
                    1793530647828353002,
                    1537374467135682527
                ]
            ),
            (
                pair("b", "a"),
                vec![1022014675660616802, 2044029351321233603, 760201017768156453]
            ),
            (
                pair("b", "b"),
                vec![
                    2039469137268213830,
                    1773095265322733708,
                    1506721393377253586
                ]
            ),
        ]
    );
    assert_eq!(dealer.draw_fp().value(), 1940071552581823960);
    // The REVEALED slot ORDER has moved twice: first in #6117 step 6 (the output
    // stage draws its sharings and its shuffle from the SAME source as the union
    // stage instead of minting a second dealer), then again when the output stage's
    // per-element degree reduction added its re-sharings, so the shuffle samples a
    // later point of one continuous keystream. The order is the shuffle's secret
    // permutation and carries no result meaning — the canonicalised multiset below
    // is byte-identical to the pre-routing capture.
    let (slots, cost) = eval_bounded_path_hidden_slots(&backend, &edges, &form, 5).unwrap();
    assert_eq!(
        slot_strings(&slots),
        [
            "<http://ex/b> <http://ex/b>",
            "dummy",
            "<http://ex/a> <http://ex/a>",
            "<http://ex/a> <http://ex/b>",
            "<http://ex/b> <http://ex/a>"
        ]
    );
    let mut sorted = slot_strings(&slots);
    sorted.sort();
    assert_eq!(
        sorted,
        [
            "<http://ex/a> <http://ex/a>",
            "<http://ex/a> <http://ex/b>",
            "<http://ex/b> <http://ex/a>",
            "<http://ex/b> <http://ex/b>",
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

/// The COMPLETE draw sequence of the `{0,2}` union stage (entry count and an
/// order-sensitive fingerprint over every method and returned `y`).
///
/// **This is the #6117-step-6 guard.** The transcript now ends in the output
/// stage's 141 `vss_own_input` draws and its `oblivious_shuffle`: if
/// `eval_bounded_path_hidden_slots_through` went back to minting its own dealer for
/// the output stage the log would stop at 1932 entries and both the length and the
/// suffix assertion would fail.
///
/// The 1932-entry PREFIX is the pre-routing whole-log pin
/// (`0x690e_cad1_cdf7_7eb0`, captured on the pre-routing code), asserted here
/// unchanged: the routing appended an output-stage SUFFIX and moved nothing in the
/// union part, and the output stage's per-element degree reduction only grew that
/// suffix. The full-log fingerprint is the new pin.
#[test]
fn seeded_bounded_stage_transcript_is_pinned() {
    use crate::randomness::test_sources::{
        fingerprint, prefix_before_first_shuffle, RecordingSource,
    };
    let backend = ShamirBackend::new_seeded(3, 0xB0B).unwrap();
    let edges = pin_edges();
    let form = HiddenBoundedPath::range("p", 0, 2);
    let mut rec = RecordingSource::new(backend.dealer());
    eval_bounded_path_hidden_slots_through(&mut rec, &backend, &edges, &form, 5).unwrap();
    assert_eq!(rec.log.len(), 2074);
    // Prefix unchanged from the pre-routing pin; the whole move is in the output
    // suffix, which grew by the per-element degree reductions.
    assert_eq!(fingerprint(&rec.log[..1932]), 0x690e_cad1_cdf7_7eb0);
    // The draws BEFORE the shuffle are byte-for-byte unchanged: this prefix
    // fingerprint is the value the pre-re-randomisation code produced, so only
    // the shuffle's own output and the draws after it moved.
    assert_eq!(
        fingerprint(prefix_before_first_shuffle(&rec.log)),
        0xa6c3_51d1_ad54_0fc2
    );
    assert_eq!(fingerprint(&rec.log), 0xd1f8_cf68_6b5c_ba26);
    let suffix: Vec<&str> = rec.log[1932..].iter().map(|(m, _)| *m).collect();
    assert_eq!(
        suffix,
        [["vss_own_input"; 141].as_slice(), &["oblivious_shuffle"]].concat()
    );
}

/// The bounded-form entry refuses an over-cap output bound before ANY draw, on a
/// NON-EMPTY edge set: `build_bounded_candidates` draws per hop equality and the
/// output stage materialises `B` slots.
#[test]
fn an_over_cap_output_bound_is_refused_before_any_draw() {
    use crate::oblivious_join::MAX_OUTPUT_BOUND;
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x7D01).unwrap();
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ]);
    let form = HiddenBoundedPath::range(P, 1, 2);
    for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err =
            eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, bound)
                .expect_err("an over-cap bound must be refused");
        match &err {
            MpcError::Protocol(m) => assert!(m.contains("MAX_OUTPUT_BOUND"), "B={}: {}", bound, m),
            other => panic!("B={}: expected a Protocol refusal, got {:?}", bound, other),
        }
        assert_eq!(
            counted.draws(),
            0,
            "B={}: the bound must be refused before any draw",
            bound
        );
    }
    assert!(eval_bounded_path_hidden(&backend, &edges, &form, 4).is_ok());
}
