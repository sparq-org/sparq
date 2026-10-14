//! Default-OFF (pure-index, `engine` feature OFF) feature-state test.
//! (sq-bif.16)
//!
//! `sparq-text`'s defaults are `["engine", "parallel", "engine-builtins"]`, so the
//! whole crate is normally exercised with `sparq-engine`/`spargebra` in the graph.
//! The documented "Pure index, no engine (lighter dep graph)" config
//! (`default-features = false`) — the index build/query primitives that must work
//! WITHOUT the engine integration — was, before this file, only ever compiled with
//! `engine` ON: every other integration test is either un-gated (so it runs in both
//! states incidentally) or `#![cfg(feature = "engine")]`. Nothing asserted the
//! engine-OFF state as a DELIBERATE, load-bearing contract.
//!
//! This whole file is `#![cfg(not(feature = "engine"))]`, so it exists ONLY when the
//! engine is compiled out. It therefore:
//!
//!  1. compiles ONLY against the pure-index public surface (`TextIndex` + the
//!     `tokenize` module) — if any of those primitives ever grew an `engine`-gated
//!     dependency, this file would fail to BUILD in the engine-OFF state, which is
//!     exactly the regression the bead is about;
//!  2. asserts the index build/query answers against an explicit, hand-written
//!     GOLDEN expectation that is the documented contract — NOT recomputed from the
//!     library — so the pure-index answers are pinned independently of feature state.
//!     The engine-ON tests (`tests/index.rs`, `tests/oracle.rs`, ...) assert the same
//!     documented semantics, so any divergence between the two compilation states is
//!     caught from one side or the other (result-equivalence across feature states).
//!
//! Under `--workspace` feature unification (when another crate turns `sparq-text`'s
//! `engine` on) this file compiles to ZERO tests rather than to a now-false assertion
//! — the gate makes it inert, not unsound.

#![cfg(not(feature = "engine"))]

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_text::tokenize::Analyzer;
use sparq_text::{Hit, TextIndex};

/// A graph whose `ex:comment` objects are the given N-Triples literal terms
/// (one fresh subject each) — mirrors `tests/index.rs::graph_of`.
fn graph_of(literals: &[&str]) -> Graph {
    let nt: String = literals
        .iter()
        .enumerate()
        .map(|(i, l)| format!("<http://ex/s{i}> <http://ex/comment> {l} .\n"))
        .collect();
    Graph::load_str(&nt, "ntriples").unwrap()
}

/// The lexical values of `hits`, in the order returned (best-first for ranked
/// queries) — mirrors `tests/index.rs::values`.
fn values(graph: &Graph, hits: &[Hit]) -> Vec<String> {
    hits.iter()
        .map(|h| match graph.dict.term(h.id) {
            Term::Literal(l) => l.value().to_string(),
            other => panic!("hit {} is not a literal: {}", h.id, other),
        })
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// The cheap index (`build`) answers AND / OR / prefix / BM25-ordering queries
/// with the engine compiled out, exactly as the documented contract specifies.
/// Golden values are written by hand — a regression in the pure-index path (or a
/// divergence from the engine-ON behaviour the un-gated tests pin) fails here.
#[test]
fn pure_index_build_and_query() {
    let g = graph_of(&[
        r#""The quick brown fox""#,
        r#""the lazy dog""#,
        r#""quick dogs and lazy foxes""#, // no stemming: "fox" != "foxes"
    ]);
    let idx = TextIndex::build(&g);
    assert_eq!(idx.len(), 3, "three distinct string literals are documents");

    // AND: every token must be present; case-folded.
    assert_eq!(values(&g, &idx.search("quick fox")), ["The quick brown fox"]);
    assert!(idx.search("quick cat").is_empty(), "AND with an absent token is empty");
    // Token order / duplication does not matter under AND.
    assert_eq!(idx.search("fox quick"), idx.search("quick quick fox"));

    // OR: at least one token; unknown tokens ignored under OR but kill AND.
    assert_eq!(
        sorted(values(&g, &idx.search_any("brown lazy"))),
        ["The quick brown fox", "quick dogs and lazy foxes", "the lazy dog"]
    );
    assert_eq!(values(&g, &idx.search_any("brown zzz")), ["The quick brown fox"]);
    assert!(idx.search("brown zzz").is_empty());

    // Prefix (`*`-suffix) under AND.
    assert_eq!(
        sorted(values(&g, &idx.search("quick fox*"))),
        ["The quick brown fox", "quick dogs and lazy foxes"]
    );
}

/// BM25 ordering (tf up-ranks, length normalisation down-ranks long docs) and the
/// `Hit.score` shape hold with the engine OFF.
#[test]
fn pure_index_bm25_ranking() {
    let g = graph_of(&[
        r#""fox""#,                                                              // short, tf 1
        r#""fox fox fox""#,                                                      // higher tf
        r#""fox and a very long sentence about many other animals entirely""#,   // long doc
        r#""unrelated""#,
    ]);
    let idx = TextIndex::build(&g);
    let hits = idx.search("fox");
    let ranked = values(&g, &hits);
    assert_eq!(ranked.len(), 3, "the unrelated doc never matches");
    assert_eq!(ranked[0], "fox fox fox", "highest tf ranks first");
    assert_eq!(
        ranked[2], "fox and a very long sentence about many other animals entirely",
        "the long doc ranks last (length normalisation)"
    );
    let scores: Vec<f32> = hits.iter().map(|h| h.score).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "scores descend");
    assert!(scores.iter().all(|s| *s > 0.0), "matching scores are positive");
}

/// Only plain / `xsd:string` / language-tagged literals are indexed (IRIs and
/// typed literals are skipped) — the analyzer + dictionary scan in the pure path.
#[test]
fn pure_index_only_string_literals() {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/p> "plain text" .
        <http://ex/a> <http://ex/p> "tagged text"@en .
        <http://ex/a> <http://ex/p> "42"^^<http://www.w3.org/2001/XMLSchema#integer> .
        <http://ex/a> <http://ex/text> <http://ex/not-text> .
        "#,
        "ntriples",
    )
    .unwrap();
    let idx = TextIndex::build(&g);
    assert_eq!(idx.len(), 2, "exactly the two string literals");
    assert!(idx.search("42").is_empty(), "typed literals are not indexed");
    assert_eq!(sorted(values(&g, &idx.search("text"))), ["plain text", "tagged text"]);
    // The named-analyzer accessor is reachable in the pure path.
    assert_eq!(idx.analyzer(), Analyzer::Unicode);
}

/// The opt-in positional index (`build_with_positions`) + `phrase` / `phrase_near`
/// adjacency / proximity primitives work with the engine OFF, and the
/// positionless-index guard (`has_positions`) is observable.
#[test]
fn pure_index_positions_phrase_and_near() {
    let g = graph_of(&[
        r#""the quick brown fox""#,
        r#""quick the brown fox""#,    // tokens present, NOT adjacent-in-order
        r#""a quick brown fox""#,
    ]);

    let cheap = TextIndex::build(&g);
    assert!(!cheap.has_positions(), "the default index stores no positions");

    let pidx = TextIndex::build_with_positions(&g);
    assert!(pidx.has_positions());

    // Phrase: adjacent, in order. Returns ascending dict ids.
    let phrase_hits: Vec<String> = pidx
        .phrase("quick brown fox")
        .iter()
        .map(|id| match g.dict.term(*id) {
            Term::Literal(l) => l.value().to_string(),
            other => panic!("phrase hit is not a literal: {}", other),
        })
        .collect();
    assert_eq!(
        sorted(phrase_hits),
        ["a quick brown fox", "the quick brown fox"],
        "only the docs where the three tokens are adjacent & in order"
    );

    // Proximity: slop 0 == exact adjacency; slop 1 lets the one non-adjacent doc in.
    assert_eq!(pidx.phrase_near("quick fox", 0).len(), 0, "quick..fox are never adjacent");
    let near = pidx.phrase_near("quick fox", 2);
    assert_eq!(near.len(), 3, "all three have quick before fox within the gap budget");
    // Tightest cluster ("a quick brown fox": gap 1) scores highest; score 1/(1+gap).
    let near_vals = values(&g, &near);
    assert!(
        near_vals[0] == "a quick brown fox" || near_vals[0] == "the quick brown fox",
        "a slop-1 cluster outranks the slop-2 reorder, got {:?}",
        near_vals
    );
    assert!(near.windows(2).all(|w| w[0].score >= w[1].score), "near scores descend");
}

/// Incremental `apply_delta` stays exactly equal to a from-scratch rebuild, and the
/// rebuild-on-boot / `reconcile` contract holds — with the engine OFF. This pins the
/// pure-index durability surface (`apply_delta`, `reconcile`, `needs_rebuild`,
/// `indexed_dict_len`, `is_consistent_with`) in the engine-OFF state.
#[test]
fn pure_index_delta_and_reconcile_contract() {
    let mut graph = Graph::load_str("", "ntriples").unwrap();
    let seed = [triple("s0", r#"the quick brown fox"#), triple("s1", "lazy dog")];
    graph.apply_delta(&seed, &[]).unwrap();

    let mut index = TextIndex::build(&graph);
    assert_eq!(index.len(), 2);
    assert!(index.is_consistent_with(&graph), "fresh build is consistent");
    assert!(!index.needs_rebuild(&graph));
    let after_build_gen = index.indexed_dict_len();

    // Insert a new string literal and a non-text (typed) literal in one batch.
    let inserts = [
        triple("s2", "quick data graph"),
        typed_triple("s3", "7", "http://www.w3.org/2001/XMLSchema#integer"),
    ];
    graph.apply_delta(&inserts, &[]).unwrap();

    // Before reconcile, the warm index is stale (dict grew).
    assert!(!index.is_consistent_with(&graph), "dict grew, index is stale");
    assert!(index.indexed_dict_len() == after_build_gen, "stale gen unchanged pre-reconcile");

    // apply_delta and reconcile must each reach the same state as a fresh rebuild.
    let mut via_delta = TextIndex::build(&{
        let mut g = Graph::load_str("", "ntriples").unwrap();
        g.apply_delta(&seed, &[]).unwrap();
        g
    });
    // rebuild `via_delta`'s graph to the post-insert graph incrementally:
    via_delta.apply_delta(&graph, &inserts, &[]);
    index.reconcile(&graph);

    let rebuilt = TextIndex::build(&graph);
    assert_eq!(index, rebuilt, "reconcile == rebuild");
    assert_eq!(via_delta, rebuilt, "apply_delta == rebuild");
    assert!(index.is_consistent_with(&graph), "reconciled index is consistent");
    assert!(!index.needs_rebuild(&graph), "an append-only dict never needs a full rebuild");

    // The newly inserted text is now searchable; the typed literal is not indexed.
    assert_eq!(values(&graph, &index.search("data graph")), ["quick data graph"]);
    assert_eq!(index.len(), 3, "two seed + one inserted string literal (typed skipped)");
}

fn triple(subj: &str, lit: &str) -> [Term; 3] {
    [
        Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{subj}"))),
        Term::NamedNode(NamedNode::new_unchecked("http://ex/comment")),
        Term::Literal(Literal::new_simple_literal(lit)),
    ]
}

fn typed_triple(subj: &str, value: &str, dt: &str) -> [Term; 3] {
    [
        Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{subj}"))),
        Term::NamedNode(NamedNode::new_unchecked("http://ex/comment")),
        Term::Literal(Literal::new_typed_literal(value, NamedNode::new_unchecked(dt))),
    ]
}
