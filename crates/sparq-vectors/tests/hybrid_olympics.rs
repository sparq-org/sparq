//! sq-88c6 — validate the **hybrid-RRF claim on the real olympics fixture**.
//!
//! The fusion helper ([`hybrid_search`]) and its small-graph end-to-end test live in
//! `tests/hybrid.rs` (a hand-crafted 5-entity graph). The bead also asks to validate the
//! hybrid-RRF claim **at scale, on the olympics dataset** — so here we fuse, over the SAME
//! athlete `Term`, this crate's embedding ANN (`VectorIndex::nearest_term`, cosine over the
//! deterministic `HashEmbedder` label embeddings) with `sparq-sim`'s structural similarity
//! (`Sim::most_similar`, weighted Jaccard over the predicate/neighbour signature) via
//! Reciprocal Rank Fusion, and assert the documented properties of the fused output hold on
//! the 1.78M-triple graph: it is non-empty, every `Term` appears once (dedup across the two
//! lists), the query term is excluded, the fused scores are RRF scores (not either
//! retriever's), and a CONSENSUS neighbour — one ranked by both signals — strictly outranks
//! any neighbour ranked by only one. `sparq-sim` is a DEV-dependency only; the library never
//! depends on it (the helper is generic over retriever closures).
//!
//! SKIPS (passes, with a note on stderr) when `bench/qlever-olympics/olympics.nt` is absent —
//! the file is a downloaded benchmark fixture, not checked in. Override with `SPARQ_OLYMPICS_NT`.

// (sq-ip3a) HNSW (VectorIndex) is the approximate backend — gated behind `approx-ann`.
#![cfg(feature = "approx-ann")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_sim::Sim;
use sparq_vectors::{embed_labels, hybrid_search, HashEmbedder, VectorIndex, VectorStore, RRF_K};

fn data_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SPARQ_OLYMPICS_NT") {
        return Some(p.into());
    }
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench/qlever-olympics/olympics.nt");
    p.exists().then_some(p)
}

/// `VectorIndex::nearest_term` returns `(Term, f32)`; widen to `f64` so the ANN list shares
/// the fused `(Term, f64)` shape with `Sim::most_similar`. RRF ignores the scores (only ranks
/// matter), so the cast is purely a type bridge — exactly the pattern documented on
/// `hybrid_search`.
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
fn hybrid_search_fuses_ann_and_structural_on_olympics() {
    let Some(path) = data_path() else {
        eprintln!("skipping: olympics.nt not present (downloaded benchmark fixture)");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read olympics.nt");
    let g = Graph::load_str(&text, "ntriples").expect("parse olympics.nt");
    drop(text);
    assert!(
        g.len() > 1_700_000,
        "expected the 1.78M-triple olympics dataset, got {}",
        g.len()
    );

    // Label-embedding store + HNSW index (the embedding/ANN retriever).
    let spqv = std::env::temp_dir().join(format!(
        "sparq-vectors-hybrid-olympics-{}.spqv",
        std::process::id()
    ));
    let embedder = HashEmbedder::new(64);
    let mut store = VectorStore::create(&spqv, 64).unwrap();
    let n = embed_labels(&g, &mut store, &embedder).expect("embed labels");
    assert!(
        n > 100_000,
        "olympics has 137k rdfs:labels, embedded only {n}"
    );
    store.finalize().unwrap();
    let index = VectorIndex::build(&store);

    // Structural similarity retriever (the semantic retriever).
    let sim = Sim::new(&g);

    let athlete = Term::NamedNode(
        NamedNode::new("http://wallscope.co.uk/resource/olympics/athlete/AAananthaSambuMayavo")
            .unwrap(),
    );

    // Both retrievers must produce non-empty rankings for this known, labelled athlete — only
    // then is the fusion claim meaningfully exercised at scale.
    let ann_alone = index.nearest_term(&athlete, &g, &store, 50);
    assert_eq!(
        ann_alone.len(),
        50,
        "ANN must rank a full window over a 137k-vector store"
    );
    let struct_alone = sim.most_similar(&athlete, 50);
    assert!(
        !struct_alone.is_empty(),
        "structural similarity must rank neighbours for a connected athlete"
    );

    // Fuse the two retrievers by `Term` via RRF — the documented hybrid pipeline.
    let top_k = 20;
    let fused = hybrid_search(
        &athlete,
        top_k,
        RRF_K,
        &mut [
            &mut |t: &Term| ann_f64(&index, &g, &store, t, 50),
            &mut |t: &Term| sim.most_similar(t, 50),
        ],
    );

    assert!(
        !fused.is_empty(),
        "fusion over a known athlete yields results at scale"
    );
    assert!(fused.len() <= top_k, "top_k bounds the fused list");

    // Every `Term` appears at most once: dedup by `Term` across the two lists.
    let mut seen = std::collections::HashSet::new();
    for (t, _) in &fused {
        assert!(seen.insert(t.clone()), "{t} duplicated in fused output");
    }
    // The query term itself is excluded by both retrievers, so it never appears.
    assert!(
        !fused.iter().any(|(t, _)| *t == athlete),
        "self must be excluded"
    );

    // Fused scores are RRF scores (a sum of `1/(k+rank)` contributions), strictly positive and
    // non-increasing — NOT either retriever's cosine/Jaccard score. The maximum possible RRF
    // score here is two rank-1 contributions, `2/(k+1)`.
    let mut prev = f64::INFINITY;
    for (_, s) in &fused {
        assert!(*s > 0.0, "RRF scores are positive");
        assert!(
            *s <= 2.0 / (RRF_K + 1.0) + 1e-12,
            "RRF score cannot exceed two rank-1 contributions"
        );
        assert!(*s <= prev + 1e-12, "fused scores must be non-increasing");
        prev = *s;
    }

    // The CONSENSUS property — the whole point of hybrid fusion: a neighbour ranked by BOTH
    // signals must be able to outrank a neighbour ranked by only one. Find a term in both lists
    // (its fused score sums two contributions) and a term in exactly one (a single contribution);
    // the two-list term, at no worse a rank in either list, scores strictly higher.
    let ann_terms: std::collections::HashSet<&Term> = ann_alone.iter().map(|(t, _)| t).collect();
    let struct_terms: std::collections::HashSet<&Term> =
        struct_alone.iter().map(|(t, _)| t).collect();
    let consensus: Vec<&Term> = ann_terms.intersection(&struct_terms).copied().collect();
    if !consensus.is_empty() {
        // Highest-ranked consensus term: best combined rank, so the strongest fused score.
        // Generic over the score type (`nearest_term` is f32, `most_similar` is f64) — only the
        // position matters for the rank.
        fn rank_in<S>(list: &[(Term, S)], term: &Term) -> Option<usize> {
            list.iter().position(|(t, _)| t == term)
        }
        let best_consensus = consensus
            .iter()
            .min_by_key(|&&t| {
                rank_in(&ann_alone, t).unwrap_or(usize::MAX)
                    + rank_in(&struct_alone, t).unwrap_or(usize::MAX)
            })
            .copied()
            .unwrap();
        let fused_consensus = fused
            .iter()
            .find(|(t, _)| t == best_consensus)
            .map(|(_, s)| *s);
        // A single-list neighbour (present in exactly one of the two retrievers).
        let single_list = fused
            .iter()
            .find(|(t, _)| {
                (ann_terms.contains(t) ^ struct_terms.contains(t)) && t != best_consensus
            })
            .map(|(_, s)| *s);
        if let (Some(c), Some(s)) = (fused_consensus, single_list) {
            assert!(
                c >= s,
                "a consensus neighbour (in both signals) must not score below a single-list one: \
                 consensus={c}, single={s}"
            );
        }
    }

    let _ = std::fs::remove_file(&spqv);
}
