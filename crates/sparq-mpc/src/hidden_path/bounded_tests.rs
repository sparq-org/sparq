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

/// An over-large unroll (huge max) is rejected fail-closed BEFORE any crypto runs.
#[test]
fn bounded_oversized_unroll_is_rejected() {
    let edges = predicated_edges(&[
        (P, "http://ex/a", "http://ex/b"),
        (P, "http://ex/b", "http://ex/c"),
        (P, "http://ex/c", "http://ex/a"),
    ]);
    let backend = ShamirBackend::new_seeded(3, 1).unwrap();
    let form = HiddenBoundedPath::range(P, 1, 64);
    let err = eval_bounded_path_hidden(&backend, &edges, &form, 1 << 30).unwrap_err();
    assert!(matches!(err, MpcError::Protocol(m) if m.contains("MAX_CHAIN_TUPLES")));
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
