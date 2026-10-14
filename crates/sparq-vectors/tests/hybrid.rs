//! sq-88c6 — `hybrid_search` end to end: fuse this crate's embedding ANN
//! (`VectorIndex::nearest_term`, cosine) with `sparq-sim`'s structural similarity
//! (`Sim::most_similar`, weighted Jaccard) BY `Term`, via Reciprocal Rank Fusion.
//!
//! `sparq-sim` is a DEV-dependency only — the `hybrid_search` helper is generic over
//! retriever closures, so the library never depends on it.

// (sq-ip3a) HNSW (VectorIndex) is the approximate backend — gated behind `approx-ann`.
#![cfg(feature = "approx-ann")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_sim::Sim;
use sparq_vectors::{embed_labels, hybrid_search, HashEmbedder, VectorIndex, VectorStore, RRF_K};

// A tiny social graph. `:alice` and `:twin` are structurally identical (same predicates,
// same neighbors) so `most_similar(alice)` ranks `:twin` first. Labels are crafted so the
// label-embedding ANN ALSO ranks `:twin` near `:alice` (shared n-grams "Alice"), giving a
// consensus winner, while `:half` is structurally close but lexically distant, and
// `:alias` is lexically close ("Alice Smith") but structurally distant.
const TTL: &str = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:   <http://example.org/> .

ex:alice rdfs:label "Alice Jones" ; ex:knows ex:bob, ex:eve ; ex:worksAt ex:acme .
ex:twin  rdfs:label "Alice Janes" ; ex:knows ex:bob, ex:eve ; ex:worksAt ex:acme .
ex:half  rdfs:label "Zeno Quark"  ; ex:knows ex:bob ; ex:worksAt ex:other .
ex:alias rdfs:label "Alice Smith" ; ex:plays ex:chess .
ex:far   rdfs:label "Far Away"    ; ex:plays ex:go .
"#;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(format!("http://example.org/{s}")).unwrap())
}

/// `VectorIndex::nearest_term` returns `(Term, f32)`; widen the score to `f64` so the ANN
/// list shares the fused `(Term, f64)` shape with `Sim::most_similar`. RRF ignores the
/// scores (only ranks matter), so the cast is purely a type bridge.
fn ann_f64(
    index: &VectorIndex,
    g: &Graph,
    store: &VectorStore,
    t: &Term,
    k: usize,
) -> Vec<(Term, f64)> {
    index
        .nearest_term(t, g, store, k)
        .into_iter()
        .map(|(t, s)| (t, s as f64))
        .collect()
}

#[test]
fn hybrid_search_fuses_ann_and_structural_by_term() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let dim = 64;
    let path =
        std::env::temp_dir().join(format!("sparq-vectors-hybrid-{}.spqv", std::process::id()));

    // Label-embedding store + HNSW index (the embedding/ANN retriever).
    let embedder = HashEmbedder::new(dim);
    let mut store = VectorStore::create(&path, dim).unwrap();
    let n = embed_labels(&g, &mut store, &embedder).unwrap();
    assert_eq!(n, 5, "all five entities are labeled");
    store.finalize().unwrap();
    let index = VectorIndex::build(&store);

    // Structural similarity retriever (the semantic retriever).
    let sim = Sim::new(&g);

    let query = iri("alice");

    // Sanity: each retriever alone ranks `:twin` first (consensus candidate).
    let ann_alone = index.nearest_term(&query, &g, &store, 10);
    assert_eq!(ann_alone[0].0, iri("twin"), "ANN: closest label n-grams");
    let struct_alone = sim.most_similar(&query, 10);
    assert_eq!(
        struct_alone[0].0,
        iri("twin"),
        "structural: identical signature"
    );

    // Fuse the two retrievers by `Term` via RRF.
    let fused = hybrid_search(
        &query,
        10,
        RRF_K,
        &mut [
            // `nearest_term` scores are f32; cast to f64 so both retrievers share the
            // fused `(Term, f64)` shape. RRF ignores the scores — only ranks matter.
            &mut |t: &Term| ann_f64(&index, &g, &store, t, 10),
            &mut |t: &Term| sim.most_similar(t, 10),
        ],
    );

    assert!(!fused.is_empty(), "fusion over a known term yields results");
    // Consensus winner: `:twin` is rank 1 in BOTH retrievers → top of the fused list.
    assert_eq!(
        fused[0].0,
        iri("twin"),
        "high in BOTH retrievers wins the fusion"
    );
    // Every `Term` appears at most once (dedup by Term across the two lists).
    let mut seen = std::collections::HashSet::new();
    for (t, _) in &fused {
        assert!(seen.insert(t.clone()), "{t} duplicated in fused output");
    }
    // The query term itself is excluded by both retrievers, so never appears.
    assert!(
        !fused.iter().any(|(t, _)| *t == query),
        "self must be excluded"
    );

    // Fused scores are RRF scores: `:twin` (rank 1 in both) = 2/(k+1).
    assert!(
        (fused[0].1 - 2.0 / (RRF_K + 1.0)).abs() < 1e-12,
        "consensus rank-1 score must be 2/(k+1), got {}",
        fused[0].1
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn hybrid_search_unknown_query_yields_empty() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let dim = 32;
    let path = std::env::temp_dir().join(format!(
        "sparq-vectors-hybrid-unknown-{}.spqv",
        std::process::id()
    ));
    let mut store = VectorStore::create(&path, dim).unwrap();
    embed_labels(&g, &mut store, &HashEmbedder::new(dim)).unwrap();
    store.finalize().unwrap();
    let index = VectorIndex::build(&store);
    let sim = Sim::new(&g);

    // A term absent from the graph: both retrievers return empty, so fusion is empty.
    let unknown = iri("nobody");
    let fused = hybrid_search(
        &unknown,
        10,
        RRF_K,
        &mut [
            &mut |t: &Term| ann_f64(&index, &g, &store, t, 10),
            &mut |t: &Term| sim.most_similar(t, 10),
        ],
    );
    assert!(
        fused.is_empty(),
        "unknown query fuses to nothing, not an error"
    );

    let _ = std::fs::remove_file(&path);
}
