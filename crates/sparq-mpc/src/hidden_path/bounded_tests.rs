// sq-py8h.3 — DIFFERENTIAL tests for the HIDDEN-intermediate BOUNDED
// property-path operator ({1,k} / {0,k} / p? + alternation, OR-fold length-dedup).
//! Differential acceptance suite for [`eval_bounded_path_hidden`].
//!
//! THE acceptance criterion (bead sq-py8h.3): the secure HIDDEN bounded path
//! (`{1,k}`, `{0,k}`, `p?`, alternation) must equal the **plaintext** bounded-path
//! evaluation over the same edge set; each connected endpoint pair returned exactly
//! once; the realized length is not separately opened or encoded in the result; the
//! reflexive diagonal is correct for
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
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
/// node IRIs to nonzero `Fp` keys (shared node namespace across predicates), and
/// DECLARE the public node domain as every subject/object term of those triples —
/// which is exactly the active graph's node domain here, so the zero-length arm of a
/// `{0,k}` form matches the oracle's diagonal (SPARQL 1.1 §18.4).
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
    let domain: Vec<Term> = triples
        .iter()
        .flat_map(|(_, s, o)| [iri_term(s), iri_term(o)])
        .collect();
    PredicatedEdges::new(edges)
        .with_node_domain(domain)
        .expect("the declared domain covers every edge endpoint by construction")
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

/// Run the SECURE hidden bounded path and return its disclosed `(a,b)` pair set.
///
/// An endpoint must be BOUND — an unbound one would mean a real defect in the
/// output stage, not "no result" — but NOT necessarily an IRI. The EDGE regime is
/// IRI-endpointed; the zero-length domain is not, so a `{0,k}` form over a graph
/// with a literal object returns that literal as an endpoint, which a
/// `PartialResult` row (`Vec<Option<Term>>`) carries directly. An IRI renders as
/// its bare string, matching `plaintext_bounded`'s keys; any other term renders
/// through `Display`.
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
            let render = |slot: &Option<Term>, which: &str| match slot {
                Some(Term::NamedNode(n)) => n.as_str().to_string(),
                Some(other) => other.to_string(),
                None => panic!("endpoint {which} must be BOUND, got an unbound slot"),
            };
            (render(&r[0], "a"), render(&r[1], "b"))
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
        let err = eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), 8)
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

/// An empty edge set with `min >= 1` yields nothing; with `min == 0` and an empty
/// DECLARED node domain the diagonal is empty too, so the result is empty (no panic,
/// fail-soft).
#[test]
fn bounded_empty_edges_is_empty() {
    let edges = predicated_edges(&[]);
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
    match eval_bounded_path_hidden_slots_through(&mut rec, &backend, &edges, &form, PayloadWidths::derived(2), 8) {
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
        match eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), 8) {
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
        match eval_bounded_path_hidden_slots_through(&mut src, &backend, &edges, &form, PayloadWidths::derived(2), 2) {
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
        PayloadWidths::derived(2),
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
            eval_bounded_path_hidden_slots_through(&mut prss, &backend, &edges, &form, PayloadWidths::derived(2), bound);
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
    eval_bounded_path_hidden_slots_through(&mut rec, &backend, &edges, &form, PayloadWidths::derived(2), 5).unwrap();
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
            eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), bound)
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

/// The coverage half of the bound contract, before any draw, for the bounded form.
///
/// Same defect as the exactly-`k` entry: the builder enumerated every length's
/// every tuple — `Σ_ℓ |E|^ℓ` secure equalities — and only then found that `B` was
/// too small to hold the answer, a refusal decidable from the disclosed endpoint
/// terms alone.
///
/// The projection has to be EXACT, not a sum of its arms: the reflexive diagonal,
/// the length-1 edge pairs and the length-`>=2` product OVERLAP, and a sum would
/// refuse bounds the operator should accept. So this pins the count for each shape
/// and then runs a bound equal to it.
#[test]
fn a_bound_below_the_bounded_candidate_count_is_refused_before_any_draw() {
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x7D07).unwrap();
    // a->b, b->c: subjects {a,b}, objects {b,c}, nodes {a,b,c}.
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ]);

    // {1,1}: only the edge pairs, (a,b) and (b,c) — 2.
    // {1,2}: the 2x2 product (a,b),(a,c),(b,b),(b,c) — 4 — which already CONTAINS
    //        both edge pairs, so a sum of the arms would say 6.
    // {0,2}: that product plus the reflexive (a,a),(b,b),(c,c) — but (b,b) is in
    //        BOTH, so the union is 6 and a sum would say 7.
    for (form, expected) in [
        (HiddenBoundedPath::range(P, 1, 1), 2usize),
        (HiddenBoundedPath::range(P, 1, 2), 4),
        (HiddenBoundedPath::range(P, 0, 2), 6),
    ] {
        let count = projected_bounded_candidate_count(&edges, &form).unwrap();
        assert_eq!(
            count, expected,
            "the projection must be the union, not the sum of its arms"
        );

        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), 1)
            .expect_err("a bound below the candidate count must be refused");
        match &err {
            MpcError::Protocol(m) => {
                assert!(m.contains("does not cover the candidate set"), "{}", m);
                // B and the PUBLIC bound only — never the candidate count.
                assert!(
                    m.contains("B=1") && m.contains("public candidate bound"),
                    "{}",
                    m
                );
                assert!(!m.contains(&format!("count {}", count)), "{}", m);
            }
            other => panic!("expected a Protocol refusal, got {:?}", other),
        }
        assert_eq!(counted.draws(), 0, "refused before any draw");

        // A bound equal to the projection runs, which is what makes it exact rather
        // than a bound that quietly refuses legal work.
        eval_bounded_path_hidden(&backend, &edges, &form, count)
            .expect("B = the exact candidate count must run");
    }
}
/// `public_bounded_candidate_pair_bound` is a function of the edge COUNT, the
/// DECLARED node-domain size and the form's public `min`/`max`, and of nothing in
/// the edges.
#[test]
fn the_public_bounded_candidate_pair_bound_is_a_function_of_the_public_sizes() {
    // {1,1}: the length-1 arm only; the domain size is irrelevant for min >= 1.
    assert_eq!(
        public_bounded_candidate_pair_bound(5, 9, &HiddenBoundedPath::range(P, 1, 1)).unwrap(),
        5
    );
    // {1,2}: length 1 plus any length >= 2.
    assert_eq!(
        public_bounded_candidate_pair_bound(5, 9, &HiddenBoundedPath::range(P, 1, 2)).unwrap(),
        30
    );
    // {0,2}: the same plus one diagonal pair per DECLARED node. The domain is an
    // independent public input, not a function of |E| — a graph whose other
    // predicates name more nodes has a larger domain for the same edge count.
    assert_eq!(
        public_bounded_candidate_pair_bound(5, 9, &HiddenBoundedPath::range(P, 0, 2)).unwrap(),
        39
    );
    assert_eq!(
        public_bounded_candidate_pair_bound(5, 40, &HiddenBoundedPath::range(P, 0, 2)).unwrap(),
        70
    );
    // {2,3}: neither the diagonal nor the length-1 arm applies.
    assert_eq!(
        public_bounded_candidate_pair_bound(5, 9, &HiddenBoundedPath::range(P, 2, 3)).unwrap(),
        25
    );
    // No edges and no declared domain: no candidates.
    assert_eq!(
        public_bounded_candidate_pair_bound(0, 0, &HiddenBoundedPath::range(P, 0, 2)).unwrap(),
        0
    );
    // No edges but a declared domain: the diagonal is still emitted for every
    // declared node, so the bound has to cover it.
    assert_eq!(
        public_bounded_candidate_pair_bound(0, 4, &HiddenBoundedPath::range(P, 0, 2)).unwrap(),
        4
    );
    // Overflow is a refusal, not a wrap.
    assert!(
        public_bounded_candidate_pair_bound(usize::MAX, 0, &HiddenBoundedPath::range(P, 0, 2))
            .is_err()
    );
    assert!(public_bounded_candidate_pair_bound(
        usize::MAX,
        usize::MAX,
        &HiddenBoundedPath::range(P, 0, 1)
    )
    .is_err());

    // Same size, different contents, same bound — the property that makes a
    // decision taken on it independent of the edge data.
    let wide = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/c", "http://ex/d"),
    ]);
    let narrow = predicated_edges(&[
        (P, "http://ex/s", "http://ex/o"),
        (P, "http://ex/s", "http://ex/o"),
    ]);
    let form = HiddenBoundedPath::range(P, 1, 2);
    assert_eq!(
        public_bounded_candidate_pair_bound(wide.len(), wide.declared_domain_len(), &form).unwrap(),
        public_bounded_candidate_pair_bound(narrow.len(), narrow.declared_domain_len(), &form)
            .unwrap()
    );
    // Their exact unions differ, which is why the public bound is what decides.
    assert_ne!(
        projected_bounded_candidate_count(&wide, &form).unwrap(),
        projected_bounded_candidate_count(&narrow, &form).unwrap()
    );
}

/// The two branches of the bounded-form bound contract, at the entry point.
#[test]
fn both_branches_of_the_bounded_bound_contract() {
    use crate::randomness::test_sources::DrawCountingSource;

    let backend = ShamirBackend::new_seeded(5, 0x7D09).unwrap();
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ]);
    let form = HiddenBoundedPath::range(P, 0, 2);
    let public =
        public_bounded_candidate_pair_bound(edges.len(), edges.declared_domain_len(), &form)
            .unwrap();
    // |E| = 2 edges, a declared domain of 3 nodes: 3 diagonal + 2 at length 1 + 4
    // at length 2.
    assert_eq!(public, 9);
    let exact = projected_bounded_candidate_count(&edges, &form).unwrap();
    assert_eq!(exact, 6, "the arms overlap, so the union is below the sum");

    // Branch 1: B at the public bound is accepted on the public sizes alone.
    eval_bounded_path_hidden(&backend, &edges, &form, public).expect("B = the public bound runs");

    // Branch 2: a B strictly below the public bound but covering the real union is
    // still accepted — no legal work is refused.
    eval_bounded_path_hidden(&backend, &edges, &form, exact)
        .expect("B below the public bound runs when the exact union fits");

    // Branch 2, refusing: below both, refused before any draw, public text only.
    let mut dealer = backend.dealer();
    let mut counted = DrawCountingSource::new(&mut dealer);
    let err =
        eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), exact - 1)
            .expect_err("B below the exact union must be refused");
    assert_eq!(counted.draws(), 0, "refused before any draw");
    match &err {
        MpcError::Protocol(m) => {
            assert!(
                m.contains("B=5") && m.contains("public candidate bound 9"),
                "{}",
                m
            );
            assert!(!m.contains("count 6"), "the union is not rendered: {}", m);
        }
        other => panic!("expected a Protocol refusal, got {:?}", other),
    }
}

// =====================================================================
// ZERO-LENGTH NODE DOMAIN — the whole active graph, not the edge
// relation's own endpoints (SPARQL 1.1 §18.4 ZeroLengthPath)
// =====================================================================

/// Build a [`PredicatedEdges`] whose EDGES come from `edge_triples` only, while the
/// DECLARED public node domain spans `all_triples` — the shape where the active
/// graph has terms the path's own edges never name.
fn predicated_edges_with_wider_domain(
    edge_triples: &[(&str, &str, &str)],
    all_triples: &[(&str, &str, &str)],
) -> PredicatedEdges {
    let mut interner = Interner::new();
    let mut edges: Vec<PredicatedEdge> = Vec::new();
    for (p, s, o) in edge_triples {
        edges.push(PredicatedEdge {
            predicate: (*p).to_string(),
            subject: interner.node(s),
            object: interner.node(o),
        });
    }
    let domain: Vec<Term> = all_triples
        .iter()
        .flat_map(|(_, s, o)| [iri_term(s), iri_term(o)])
        .collect();
    PredicatedEdges::new(edges)
        .with_node_domain(domain)
        .expect("the active graph's domain covers every edge endpoint")
}

/// THE reproducer, hidden regime. The federation's active graph holds `a p b` and
/// `c q d`; only the `p` edge enters the path's edge relation. `p?` is a
/// zero-length arm, so §18.4 binds `(x,x)` for every term of the active graph —
/// `c` and `d` included, although no edge in the relation names them.
///
/// Drawing the reflexive diagonal from the edge relation's own endpoints yielded
/// {aa, ab, bb} and missed (c,c) and (d,d).
#[test]
fn differential_zero_length_diagonal_spans_the_whole_active_graph() {
    let all = [
        (P, "http://ex/a", "http://ex/b"),
        (Q, "http://ex/c", "http://ex/d"),
    ];
    let edges = predicated_edges_with_wider_domain(&all[..1], &all);
    let backend = ShamirBackend::new_seeded(5, 0x2D01).unwrap();
    let form = HiddenBoundedPath::optional(P);

    let got = secure_bounded(&backend, &edges, &form, 8);
    let want = plaintext_bounded(&all, &[P], 0, 1);
    assert_eq!(
        got, want,
        "p? must bind (x,x) for every term of the active graph, not just the \
         endpoints of the path's own edges"
    );
    for n in ["http://ex/a", "http://ex/b", "http://ex/c", "http://ex/d"] {
        assert!(
            got.contains(&(n.to_string(), n.to_string())),
            "reflexive ({n},{n}) missing: {got:?}"
        );
    }
    assert_eq!(got.len(), 5, "4 diagonal pairs + the one p edge: {got:?}");
}

/// The same with a `{0,2}` arm, and the bound sized from the public inputs.
#[test]
fn differential_zero_to_k_diagonal_spans_the_whole_active_graph() {
    let all = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (Q, "http://ex/x", "http://ex/y"),
    ];
    let edges = predicated_edges_with_wider_domain(&all[..2], &all);
    let backend = ShamirBackend::new_seeded(5, 0x2D02).unwrap();
    let form = HiddenBoundedPath::range(P, 0, 2);

    let public =
        public_bounded_candidate_pair_bound(edges.len(), edges.declared_domain_len(), &form)
            .unwrap();
    let got = secure_bounded(&backend, &edges, &form, public);
    assert_eq!(got, plaintext_bounded(&all, &[P], 0, 2));
    // 5 diagonal (a,b,c,x,y) + (a,b),(b,c) + (a,c) = 8.
    assert_eq!(got.len(), 8, "{got:?}");
}

/// A zero-length arm on an edge relation with NO declared node domain is refused
/// BEFORE any sharing is drawn, not answered from the edge endpoints. The refusal
/// is value-free: no term, no count.
#[test]
fn hidden_zero_length_arm_without_a_declared_domain_is_refused_pre_crypto() {
    use crate::randomness::test_sources::DrawCountingSource;

    let edges = PredicatedEdges::new(vec![PredicatedEdge {
        predicate: P.to_string(),
        subject: HiddenNode {
            key: Fp::new(1),
            term: iri_term("http://ex/a"),
        },
        object: HiddenNode {
            key: Fp::new(2),
            term: iri_term("http://ex/b"),
        },
    }]);
    let backend = ShamirBackend::new_seeded(5, 0x2D03).unwrap();

    for form in [
        HiddenBoundedPath::optional(P),
        HiddenBoundedPath::range(P, 0, 3),
    ] {
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), 64)
            .expect_err("a min == 0 form without a declared domain must be refused");
        assert_eq!(counted.draws(), 0, "refused before any draw");
        match &err {
            MpcError::Protocol(m) => {
                assert!(
                    m.contains("PUBLIC node domain") && m.contains("with_node_domain"),
                    "{}",
                    m
                );
                for value in ["ex/a", "ex/b"] {
                    assert!(!m.contains(value), "the message names a term: {}", m);
                }
            }
            other => panic!("expected a Protocol refusal, got {:?}", other),
        }
    }

    // min >= 1 never reads the domain, so it runs on the same relation.
    let plus = HiddenBoundedPath::range(P, 1, 1);
    let got = secure_bounded(&backend, &edges, &plus, 8);
    assert_eq!(
        got,
        BTreeSet::from([("http://ex/a".to_string(), "http://ex/b".to_string())])
    );
}

/// A declared domain that omits an endpoint of an edge in the relation is refused
/// at declaration time — a zero-length arm over it would under-answer exactly as
/// the old edge-derived diagonal did. The message names counts only.
#[test]
fn hidden_declared_domain_omitting_an_edge_endpoint_is_refused() {
    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ];
    let base = predicated_edges(&triples);
    let err = base
        .clone()
        .with_node_domain(vec![iri_term("http://ex/a"), iri_term("http://ex/b")])
        .unwrap_err();
    match &err {
        MpcError::Protocol(m) => {
            assert!(
                m.contains("the declared node domain does not cover the edge endpoints"),
                "{}",
                m
            );
            assert!(!m.contains("ex/c"), "the message names a term: {}", m);
            // COUNT-free as well as value-free: a message naming how many endpoints
            // were uncovered is a statistic over the edge relation, and a caller
            // could probe one candidate term at a time and read it as a membership
            // answer.
            assert!(
                !m.chars().any(|c| c.is_ascii_digit()),
                "the message carries a count: {}",
                m
            );
        }
        other => panic!("expected a Protocol refusal, got {:?}", other),
    }

    // A WIDER domain is accepted, and it is what widens the diagonal.
    let wider = base
        .with_node_domain(vec![
            iri_term("http://ex/a"),
            iri_term("http://ex/b"),
            iri_term("http://ex/c"),
            iri_term("http://ex/elsewhere"),
        ])
        .expect("a superset domain is accepted");
    let backend = ShamirBackend::new_seeded(5, 0x2D04).unwrap();
    let got = secure_bounded(&backend, &wider, &HiddenBoundedPath::optional(P), 16);
    assert!(got.contains(&(
        "http://ex/elsewhere".to_string(),
        "http://ex/elsewhere".to_string()
    )));
    assert_eq!(got.len(), 6, "4 diagonal + (a,b),(b,c): {got:?}");
}

/// The declared domain is a set of `Term`s, not IRIs: a LITERAL in the active
/// graph's object position is in the zero-length domain (SPARQL 1.1 §18.4), and
/// the diagonal must carry it. The edges stay IRI-endpointed — only the DOMAIN
/// widens — and a `PartialResult` row is `Vec<Option<Term>>`, so the literal
/// endpoint comes back out of the oblivious output unchanged.
#[test]
fn hidden_zero_length_diagonal_carries_a_literal_domain_term() {
    let mut interner = Interner::new();
    let edges = vec![PredicatedEdge {
        predicate: P.to_string(),
        subject: interner.node("http://ex/a"),
        object: interner.node("http://ex/b"),
    }];
    let lit = Term::Literal(oxrdf::Literal::new_simple_literal("L"));
    let typed = Term::Literal(oxrdf::Literal::new_language_tagged_literal("L", "en").unwrap());
    let edges = PredicatedEdges::new(edges)
        .with_node_domain(vec![
            iri_term("http://ex/a"),
            iri_term("http://ex/b"),
            iri_term("http://ex/c"),
            lit.clone(),
            typed.clone(),
        ])
        .expect("a Term domain covering the edge endpoints is accepted");

    let backend = ShamirBackend::new_seeded(5, 0x2D05).unwrap();
    let got = secure_bounded(&backend, &edges, &HiddenBoundedPath::optional(P), 16);

    for term in [&lit, &typed] {
        let rendered = term.to_string();
        assert!(
            got.contains(&(rendered.clone(), rendered.clone())),
            "the literal identity pair for {rendered} must be present: {got:?}"
        );
    }
    // 3 IRIs + 2 distinct literals = 5 diagonal pairs, + the (a,b) edge = 6. The
    // plain and the language-tagged "L" are distinct TERMS and do not collapse.
    assert_eq!(got.len(), 6, "{got:?}");
}

/// An explicitly EMPTY declared domain is a declaration, so a `min == 0` form
/// RUNS and returns an empty diagonal — the right answer for an empty active
/// graph. An UNDECLARED domain is refused. Both would otherwise look alike.
#[test]
fn hidden_an_empty_declared_domain_runs_while_an_undeclared_one_is_refused() {
    let backend = ShamirBackend::new_seeded(5, 0x2D06).unwrap();
    let form = HiddenBoundedPath::range(P, 0, 2);

    let empty = PredicatedEdges::new(Vec::new())
        .with_node_domain(Vec::new())
        .expect("an empty domain over an empty edge relation is consistent");
    assert_eq!(empty.declared_domain_len(), 0);
    assert!(secure_bounded(&backend, &empty, &form, 8).is_empty());

    let undeclared = PredicatedEdges::new(Vec::new());
    assert_eq!(undeclared.declared_domain_len(), 0);
    assert!(
        eval_bounded_path_hidden(&backend, &undeclared, &form, 8).is_err(),
        "an undeclared domain must be refused, not answered as empty"
    );
}

// =====================================================================
// ENGINE-BACKED differential for the declared node domain
// =====================================================================

/// Build the hidden relation and its DECLARED domain from one turtle document,
/// the way a federation member would: the edges are the document's triples for
/// `edge_preds` only, and the domain is every distinct subject/object term of the
/// WHOLE document — read back out of the real engine, so it is literally the term
/// set the clear-text oracle's zero-length match ranges over.
///
/// This gives the hidden regime the same engine-backed oracle the disclosed-key
/// suite uses, instead of only the in-file plaintext one: the declared domain is
/// not this test's guess at `graph_nodes`, it IS `graph_nodes`.
fn hidden_edges_and_engine_domain(doc: &str, edge_preds: &[&str]) -> PredicatedEdges {
    hidden_edges_and_engine_domain_from(&load_fixture(doc), edge_preds)
}

/// Parse one fixture document into a graph. Separate from the helpers below so a
/// test can share ONE load between the edges, the declared domain and the engine
/// oracle — which a fixture carrying a BLANK NODE must do, since a blank node's
/// identity is per-load and two loads of the same document do not agree on it.
fn load_fixture(doc: &str) -> sparq_core::Graph {
    let full = format!("@prefix ex: <http://ex/> .\n{doc}");
    sparq_core::Graph::load_str(&full, "turtle").expect("the fixture parses")
}

/// [`hidden_edges_and_engine_domain`] over an already-loaded graph.
fn hidden_edges_and_engine_domain_from(
    graph: &sparq_core::Graph,
    edge_preds: &[&str],
) -> PredicatedEdges {
    use sparq_engine::query;

    let mut interner = Interner::new();
    let mut edges: Vec<PredicatedEdge> = Vec::new();
    for p in edge_preds {
        let rows = query(graph, &format!("SELECT ?s ?o WHERE {{ ?s <{p}> ?o }}"))
            .unwrap()
            .rows;
        for row in rows {
            // The EDGE regime is IRI-only, so a fixture must not hand a literal
            // edge endpoint to `edge_preds`.
            let iri = |slot: &Option<Term>| match slot {
                Some(Term::NamedNode(n)) => n.as_str().to_string(),
                other => panic!("edge endpoints must be IRIs, got {other:?}"),
            };
            edges.push(PredicatedEdge {
                predicate: (*p).to_string(),
                subject: interner.node(&iri(&row[0])),
                object: interner.node(&iri(&row[1])),
            });
        }
    }

    let mut domain: BTreeMap<String, Term> = BTreeMap::new();
    for row in query(graph, "SELECT ?s ?o WHERE { ?s ?p ?o }")
        .unwrap()
        .rows
    {
        for slot in row.iter().flatten() {
            domain
                .entry(format!("{:?}", slot))
                .or_insert_with(|| slot.clone());
        }
    }
    PredicatedEdges::new(edges)
        .with_node_domain(domain.into_values())
        .expect("graph_nodes covers every edge endpoint")
}

/// The engine's answer for `?a <path> ?b` over the same document, keyed exactly as
/// [`secure_bounded`] keys its rows (an IRI as its bare string, anything else
/// through `Display`).
fn engine_pairs(doc: &str, path: &str) -> BTreeSet<(String, String)> {
    engine_pairs_from(&load_fixture(doc), path)
}

/// [`engine_pairs`] over an already-loaded graph.
fn engine_pairs_from(graph: &sparq_core::Graph, path: &str) -> BTreeSet<(String, String)> {
    use sparq_engine::query;

    let q = format!("PREFIX ex: <http://ex/> SELECT ?a ?b WHERE {{ ?a {path} ?b }}");
    let render = |slot: &Option<Term>| match slot {
        Some(Term::NamedNode(n)) => n.as_str().to_string(),
        Some(other) => other.to_string(),
        None => panic!("an endpoint column must be BOUND"),
    };
    query(graph, &q)
        .unwrap()
        .rows
        .iter()
        .map(|r| (render(&r[0]), render(&r[1])))
        .collect()
}

/// The hidden `p?` against the REAL ENGINE, on a graph whose domain carries two
/// literals under a predicate the path does not traverse. Same shape as the
/// disclosed-key reproducer, now through the secure operator.
#[test]
fn differential_hidden_optional_equals_the_engine_with_a_literal_domain() {
    let doc = "ex:a ex:p ex:b . ex:c ex:q \"L\" . ex:d ex:q \"L\"@en .";
    let edges = hidden_edges_and_engine_domain(doc, &[P]);
    let backend = ShamirBackend::new_seeded(5, 0x2D07).unwrap();

    let got = secure_bounded(&backend, &edges, &HiddenBoundedPath::optional(P), 16);
    assert_eq!(
        got,
        engine_pairs(doc, "ex:p?"),
        "the hidden p? must equal the engine's over the same active graph"
    );
    for lit in [
        Term::Literal(oxrdf::Literal::new_simple_literal("L")),
        Term::Literal(oxrdf::Literal::new_language_tagged_literal("L", "en").unwrap()),
    ] {
        let key = lit.to_string();
        assert!(
            got.contains(&(key.clone(), key.clone())),
            "the literal identity pair {key} must be present: {got:?}"
        );
    }
    // 4 IRIs (a,b,c,d) + 2 distinct literals = 6 diagonal pairs, + (a,b) = 7.
    assert_eq!(got.len(), 7, "{got:?}");
}

/// The hidden `{0,2}` against the REAL ENGINE, over a two-hop chain plus an
/// undisclosed predicate contributing further domain terms.
#[test]
fn differential_hidden_zero_to_two_equals_the_engine() {
    let doc = "ex:a ex:p ex:b . ex:b ex:p ex:c . ex:x ex:q ex:y . ex:z ex:q 7 .";
    let edges = hidden_edges_and_engine_domain(doc, &[P]);
    let backend = ShamirBackend::new_seeded(5, 0x2D08).unwrap();

    let form = HiddenBoundedPath::range(P, 0, 2);
    let public =
        public_bounded_candidate_pair_bound(edges.len(), edges.declared_domain_len(), &form)
            .unwrap();
    let got = secure_bounded(&backend, &edges, &form, public);
    assert_eq!(got, engine_pairs(doc, "(ex:p)? | ex:p/ex:p"));
    // 7 domain terms (a,b,c,x,y,z,7) diagonal + (a,b),(b,c) + (a,c) = 10.
    assert_eq!(got.len(), 10, "{got:?}");
}

/// A BLANK NODE of the LOADED graph yields its identity pair in the hidden
/// regime too, against the real engine. The blank node sits under a predicate the
/// path does not traverse, so it is a DOMAIN term only — the edges keep their own
/// `HiddenNode` keys — and its diagonal entry gets the same constant shared `1`
/// chain bit as any other, so nothing is compared on a document-scoped label.
///
/// One load feeds the edges, the declared domain and the oracle: a blank node's
/// identity is per-load, so two loads would differ on the label and the
/// comparison would be about parsing rather than about the operator.
#[test]
fn differential_hidden_blank_node_of_the_loaded_graph_yields_its_identity_pair() {
    let doc = "ex:a ex:p ex:b . ex:c ex:q [ ] .";
    let graph = load_fixture(doc);
    let edges = hidden_edges_and_engine_domain_from(&graph, &[P]);

    let backend = ShamirBackend::new_seeded(5, 0x2D09).unwrap();
    let got = secure_bounded(&backend, &edges, &HiddenBoundedPath::optional(P), 16);
    assert_eq!(
        got,
        engine_pairs_from(&graph, "ex:p?"),
        "the hidden p? must equal the engine's over the same loaded graph"
    );
    let blanks = got
        .iter()
        .filter(|(a, b)| a.starts_with("_:") && a == b)
        .count();
    assert_eq!(
        blanks, 1,
        "the loaded graph's blank node must contribute its identity pair: {got:?}"
    );
    // (a,a), (b,b), (c,c), the blank node's diagonal, and the (a,b) edge.
    assert_eq!(got.len(), 5, "{got:?}");
}

/// The output stage's PUBLIC-SHAPE caps are decided at the OPERATOR's entry, so
/// nothing is drawn when one of them refuses.
///
/// The term and limb caps used to be charged inside the output stage, which the
/// bounded operator reaches only after `build_bounded_candidates` has already dealt
/// the per-hop match bits — so "refused before any draw" was true of the stage and
/// not of the operator. `check_output_public_shape` runs the same closed forms at
/// the operator's entry; the compute-party cap is the one a caller can trip with a
/// legal form and a legal bound, so it is the one tested here. The counter is the
/// witness: a refusal that still dealt would show a nonzero draw count.
#[test]
fn an_over_cap_public_shape_refuses_the_operator_before_any_draw() {
    use crate::oblivious_join::MAX_OUTPUT_PARTIES;
    use crate::randomness::test_sources::DrawCountingSource;

    let triples = [
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
    ];
    let edges = predicated_edges(&triples);
    let form = HiddenBoundedPath::range(P, 1, 2);

    // More compute parties than the output stage's work model admits.
    let backend = ShamirBackend::new_seeded(MAX_OUTPUT_PARTIES + 2, 0x91).unwrap();
    let mut dealer = backend.dealer();
    let mut counted = DrawCountingSource::new(&mut dealer);
    let err = eval_bounded_path_hidden_slots_through(&mut counted, &backend, &edges, &form, PayloadWidths::derived(2), 8)
        .expect_err("a party count past the output cap must be refused");
    assert!(
        matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_PARTIES")),
        "the refusal must name the cap, got {:?}",
        err
    );
    assert_eq!(
        counted.draws(),
        0,
        "the operator must refuse on public numbers alone, having drawn nothing"
    );

    // And the same form at an admissible party count still runs, so the test is
    // pinning the cap and not a broken fixture.
    let ok_backend = ShamirBackend::new_seeded(3, 0x92).unwrap();
    let mut ok_dealer = ok_backend.dealer();
    eval_bounded_path_hidden_slots_through(&mut ok_dealer, &ok_backend, &edges, &form, PayloadWidths::derived(2), 8)
        .expect("an admissible party count still evaluates");
}
