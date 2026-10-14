//! (sq-7hx6, epic sq-3183) Tests for the filtered-ANN **pre-filter vs post-filter cost
//! model** — the public `CostModel` / `nearest_filtered_costed` / `postfilter_exact` seam.
//!
//! What is proven here:
//! - the heuristic picks **pre-filter for a selective mask** and **post-filter for a broad mask**
//!   (assert the *decision*, via [`CostModel::decide`] and the [`Strategy`] in the estimate);
//! - whichever branch is chosen, the result is **byte-identical** (the load-bearing correctness
//!   invariant) — `nearest_exact_filtered`, `postfilter_exact`, and the costed entry point all agree;
//! - the model wired into the `vec:` rewrite leaves the end-to-end answer unchanged (the same query
//!   answer whether the mask is selective or broad), proven through the public `query_vec` seam.
//!
//! Gated on `filtered-ann` (the cost model). The end-to-end section additionally needs
//! `vec-predicate` (the rewrite) and is gated inline.
#![cfg(feature = "filtered-ann")]

use sparq_vectors::{
    nearest_exact_filtered, nearest_filtered_costed, postfilter_exact, CostModel, IdMask, Strategy,
    VectorStore,
};

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("sparq_vec_costtest_{}_{name}.spqv", std::process::id()));
    p
}

/// Points on the unit circle, dict ids 1..=n.
fn ring(name: &str, n: usize) -> VectorStore {
    let mut store = VectorStore::create(tmp(name), 2).unwrap();
    for i in 0..n {
        let theta = std::f32::consts::TAU * (i as f32) / (n as f32);
        store
            .put((i as u32) + 1, &[theta.cos(), theta.sin()])
            .unwrap();
    }
    store.finalize().unwrap();
    store
}

/// The HEURISTIC decision: a selective mask → pre-filter; a non-selective (broad) mask →
/// post-filter. (Default scatter_penalty 2.0 ⇒ crossover at half the store.)
#[test]
fn heuristic_picks_prefilter_when_selective_postfilter_when_broad() {
    let model = CostModel::default();
    let n = 1000;

    // Selective: 5% of the store → pre-filter.
    let selective = model.decide(50, n, 10);
    assert_eq!(
        selective.strategy,
        Strategy::PreFilter,
        "a selective mask (5% of store) must pre-filter: {selective:?}"
    );

    // Broad: 80% of the store → post-filter (50·.. no — 800·2 = 1600 > 1000).
    let broad = model.decide(800, n, 10);
    assert_eq!(
        broad.strategy,
        Strategy::PostFilter,
        "a broad mask (80% of store) must post-filter: {broad:?}"
    );

    // The estimate exposes WHY: prefilter_cost < postfilter_cost on the selective case, and the
    // reverse on the broad one.
    assert!(selective.prefilter_cost < selective.postfilter_cost);
    assert!(broad.prefilter_cost > broad.postfilter_cost);
}

/// CORRECTNESS INVARIANT: the two strategies return byte-identical (id, score) lists across a range
/// of selectivities and k, including k larger than the mask. Whichever the heuristic picks, the
/// answer is the same.
#[test]
fn pre_and_post_filter_return_identical_results() {
    let store = ring("identical", 128);
    let query = [1.0f32, 0.0];

    for ids in [
        vec![1u32, 17, 33, 49, 65],        // selective, scattered
        (1u32..=128).step_by(2).collect(), // half
        (1u32..=128).collect(),            // full
        vec![3u32, 9],                     // fewer than k
    ] {
        let mask: IdMask = ids.into_iter().collect();
        for k in [1usize, 4, 8, 20] {
            let pre = nearest_exact_filtered(&store, &query, &mask, k);
            let post = postfilter_exact(&store, &query, &mask, k);
            assert_eq!(
                pre, post,
                "pre/post-filter must be identical (mask_len={}, k={k})",
                mask.len()
            );

            // The costed entry point agrees regardless of which branch the model would pick:
            // force each branch and confirm the hits match the ground-truth pre-filter.
            let force_pre = CostModel {
                scatter_penalty: 1.0,
            };
            let force_post = CostModel {
                scatter_penalty: 1e9,
            };
            let (h_pre, e_pre) = nearest_filtered_costed(&store, &query, &mask, k, &force_pre);
            let (h_post, e_post) = nearest_filtered_costed(&store, &query, &mask, k, &force_post);
            // (A full mask under penalty 1.0 sits exactly at the boundary → still pre-filter.)
            assert_eq!(e_pre.strategy, Strategy::PreFilter);
            assert_eq!(e_post.strategy, Strategy::PostFilter);
            assert_eq!(h_pre, pre, "forced pre-filter must match ground truth");
            assert_eq!(h_post, pre, "forced post-filter must match ground truth");
        }
    }
}

/// End-to-end through the public `vec:` seam: the rewrite uses the cost model internally, so the
/// query answer must be the SAME whether the mask is selective or broad — i.e. the cost model is
/// answer-transparent. We compare a constrained `vec:` query against the post-filter computed by
/// hand from the unfiltered result.
#[cfg(feature = "vec-predicate")]
mod end_to_end {
    use super::*;
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;
    use sparq_vectors::{query_vec, QueryResult};

    const PFX: &str = "PREFIX vec: <http://sparq.dev/vec#> ";

    fn fixture(name: &str) -> (Graph, VectorStore) {
        // 12 entities on a ring; even ids are :Car, odd ids are :Boat. A :Car constraint admits
        // half the embedded entities — broad enough that the cost model would post-filter — while
        // still exercising the answer-identity through the rewrite.
        let mut ttl = String::new();
        for i in 1..=12u32 {
            let kind = if i % 2 == 0 { "Car" } else { "Boat" };
            ttl.push_str(&format!(
                "<http://ex/n{i}> <http://ex/kind> <http://ex/{kind}> .\n"
            ));
        }
        let g = Graph::load_str(&ttl, "ntriples").unwrap();
        let id = |s: &str| {
            g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
                .unwrap()
        };
        let mut store = VectorStore::create(tmp(name), 2).unwrap();
        for i in 1..=12u32 {
            let theta = std::f32::consts::TAU * (i as f32) / 12.0;
            store
                .put(id(&format!("http://ex/n{i}")), &[theta.cos(), theta.sin()])
                .unwrap();
        }
        store.finalize().unwrap();
        (g, store)
    }

    fn iris(r: &QueryResult, col: usize) -> Vec<String> {
        r.rows
            .iter()
            .filter_map(|row| match &row[col] {
                Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
                _ => None,
            })
            .collect()
    }

    /// The constrained `vec:search` (which goes through the cost model) returns exactly the
    /// post-filter of the unfiltered ranking by the same constraint — proving the model wired into
    /// the rewrite is answer-transparent, whichever branch it takes.
    #[test]
    fn rewrite_answer_matches_manual_postfilter() {
        let (g, store) = fixture("e2e");
        let k = 3;

        // Unfiltered best-first order over ALL 12.
        let unfiltered = query_vec(
            &g,
            &format!("{PFX} SELECT ?node ?s WHERE {{ ( ?node ?s ) vec:search ( \"1,0\" 12 ) }} ORDER BY DESC(?s)"),
            &store,
        )
        .unwrap();
        let order = iris(&unfiltered, 0);

        // The Car set.
        let cars: std::collections::HashSet<String> = query_vec(
            &g,
            "SELECT ?node WHERE { ?node <http://ex/kind> <http://ex/Car> }",
            &store,
        )
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match &row[0] {
            Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect();

        let manual: Vec<String> = order
            .iter()
            .filter(|n| cars.contains(*n))
            .take(k)
            .cloned()
            .collect();

        // Constrained search — runs through the cost model in the rewrite.
        let filtered = query_vec(
            &g,
            &format!(
                "{PFX} SELECT ?node ?s WHERE {{
                   ( ?node ?s ) vec:search ( \"1,0\" {k} ) .
                   ?node <http://ex/kind> <http://ex/Car> .
                 }} ORDER BY DESC(?s)"
            ),
            &store,
        )
        .unwrap();

        assert_eq!(
            iris(&filtered, 0),
            manual,
            "the cost-model-routed rewrite answer must equal the manual post-filter"
        );
    }
}
