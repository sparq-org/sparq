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
