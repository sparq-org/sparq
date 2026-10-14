//! **Rank/score fusion** for hybrid retrieval: combine the text-vector ranking from
//! this crate with any other ranked signal — typically `sparq-sim`'s structural
//! similarity — without this crate depending on the other.
//!
//! Both helpers take plain ranked `(item, score)` lists (best first), so any producer
//! works: `VectorIndex::nearest_term` cosine,
//! `sparq_sim::Sim::most_similar` weighted Jaccard, a lexical/BM25 list, …
//!
//! Which to use (industry conventions distilled in
//! `research/genai-text-embedding-practices.md`):
//!
//! - [`fuse_rrf`] — **Reciprocal Rank Fusion**, `Σ 1/(k + rank)`, the standard
//!   `k = `[`RRF_K`]` = 60`. Uses only the *ranks*, so it needs no score
//!   normalization — the right default when the signals' scales differ (cosine in
//!   `[-1, 1]` vs Jaccard in `[0, 1]`).
//! - [`fuse_scores`] — **relative score fusion**: min-max normalizes each list to
//!   `[0, 1]`, then blends `alpha·a + (1 − alpha)·b`. Preserves score *magnitudes*
//!   (a runaway top hit stays a runaway), at the cost of an `alpha` to tune.
//!
//! For the common case — fuse this crate's ANN with another `Term` retriever — reach
//! for [`hybrid_search`], which drives N retriever closures off one query `Term` and
//! fuses their ranked `Term` lists by [`fuse_rrf`], deduplicating by `Term`:
//!
//! (sq-ip3a) `ignore`d, not `no_run`: it references the HNSW `VectorIndex`, which is
//! gated behind the opt-in `approx-ann` feature, so this illustration must not be compiled in the
//! default doctest build. The executed RRF example below carries no ANN dependency.
//!
//! ```ignore
//! # use sparq_core::Graph;
//! # use sparq_vectors::{VectorStore, VectorIndex, hybrid_search, RRF_K};
//! # use oxrdf::Term;
//! # fn demo(graph: &Graph, store: &VectorStore, index: &VectorIndex,
//! #         sim: &sparq_sim::Sim, query: &Term) {
//! // Fuse the embedding ANN with sparq-sim's structural similarity, BY `Term`. The ANN's
//! // f32 cosine scores are widened to f64 so both lists share the fused `(Term, f64)`
//! // shape — RRF ignores the scores anyway, only the ranks matter.
//! let fused = hybrid_search(query, 10, RRF_K, &mut [
//!     &mut |t: &Term| index.nearest_term(t, graph, store, 50)   // ANN (cosine)
//!         .into_iter().map(|(t, s)| (t, s as f64)).collect(),
//!     &mut |t: &Term| sim.most_similar(t, 50),                  // semantic (Jaccard)
//! ]);
//! // `fused` is one ranked `Vec<(Term, f64)>`, each `Term` appearing once. A term ranked
//! // high in BOTH retrievers outranks one ranked high in only one — consensus wins.
//! # let _ = fused;
//! # }
//! ```
//!
//! ```
//! use sparq_vectors::{fuse_rrf, fuse_scores, RRF_K};
//!
//! let text: Vec<(&str, f64)> = vec![("a", 0.92), ("b", 0.85), ("c", 0.20)];
//! let structural: Vec<(&str, f64)> = vec![("b", 0.61), ("d", 0.55), ("a", 0.10)];
//!
//! let hybrid = fuse_rrf(&[&text, &structural], RRF_K, 3);
//! assert_eq!(hybrid[0].0, "b"); // ranked 2nd + 1st — consensus beats either alone
//!
//! let blended = fuse_scores(&text, &structural, 0.7, 3); // 70% text, 30% structural
//! assert_eq!(blended[0].0, "b"); // strong in both signals
//! assert_eq!(fuse_scores(&text, &structural, 1.0, 3)[0].0, "a"); // text only
//! ```

use rustc_hash::FxHashMap;
use std::hash::Hash;

/// The standard RRF rank constant (`k = 60`), empirically robust across datasets and
/// the default in Azure AI Search, Elasticsearch, MariaDB, … Lower `k` trusts each
/// list's top hit more; higher `k` rewards consensus across lists.
pub const RRF_K: f64 = 60.0;

/// **Reciprocal Rank Fusion** of any number of ranked lists (each best first):
/// `score(item) = Σ_lists 1 / (k + rank)`, ranks 1-based. Input scores are ignored —
/// only the order matters — so differently-scaled signals fuse without normalization.
/// Returns the top `top_k` fused items, best first; ties break by first appearance
/// (earlier list, then earlier rank), so the result is deterministic.
///
/// Use [`RRF_K`] for `k` unless you have a reason not to.
pub fn fuse_rrf<T: Clone + Eq + Hash>(
    lists: &[&[(T, f64)]],
    k: f64,
    top_k: usize,
) -> Vec<(T, f64)> {
    let weighted: Vec<(&[(T, f64)], f64)> = lists.iter().map(|&l| (l, 1.0)).collect();
    fuse_rrf_weighted(&weighted, k, top_k)
}

/// **Weighted Reciprocal Rank Fusion** (Elasticsearch-style): like
/// [`fuse_rrf`] but each list carries a non-negative weight —
/// `score(item) = Σ_lists weight_l / (k + rank_l)`, ranks 1-based. Use it to
/// down-weight a noisier signal without dropping it (e.g. text 1.0,
/// structural 0.5); weight 0 mutes a list ENTIRELY — its items do
/// not appear unless another weighted list also ranks them (review 1874: a
/// muted list previously injected its items at score 0.0, so they could surface
/// as standalone results when `top_k` was large or every list was zero-weighted).
/// All-1.0 weights reduce to plain [`fuse_rrf`]. Ties break by first appearance
/// across `(list, rank)` order, deterministically.
///
/// # Panics
/// If `k ≤ 0`, or any weight is negative or non-finite.
pub fn fuse_rrf_weighted<T: Clone + Eq + Hash>(
    lists: &[(&[(T, f64)], f64)],
    k: f64,
    top_k: usize,
) -> Vec<(T, f64)> {
    assert!(k > 0.0, "RRF k must be positive");
    assert!(
        lists.iter().all(|&(_, w)| w.is_finite() && w >= 0.0),
        "RRF list weights must be finite and non-negative"
    );
    // item -> (fused score, first-seen order for deterministic ties)
    let mut acc: FxHashMap<T, (f64, usize)> = FxHashMap::default();
    let mut order = 0usize;
    for (list, weight) in lists {
        // Skip muted (weight 0) lists entirely so they inject no standalone items —
        // a 0.0 contribution would still register the item in `acc` and let it surface in the
        // top-k. A muted list contributes nothing. See review 1874.
        if *weight == 0.0 {
            continue;
        }
        for (rank0, (item, _)) in list.iter().enumerate() {
            let contribution = weight / (k + (rank0 + 1) as f64);
            let entry = acc.entry(item.clone()).or_insert_with(|| {
                order += 1;
                (0.0, order)
            });
            entry.0 += contribution;
        }
    }
    top_n(acc, top_k)
}

/// sq-88c6 — a borrowed retriever for [`hybrid_search`]: maps a query of type
/// `Q` to a ranked `Vec<(T, f64)>` (best first). `&mut dyn` so heterogeneous retrievers
/// (a `VectorIndex::nearest_term` closure, a `Sim::most_similar` closure, …) share one
/// slice. Construct as `&mut |q: &Q| { … }`.
pub type Retriever<'r, Q, T> = &'r mut dyn FnMut(&Q) -> Vec<(T, f64)>;

/// sq-88c6 — **Hybrid retrieval over one query item, fused by [`fuse_rrf`].**
///
/// Drives every `retriever` off the same `query` (typically a [`Term`](oxrdf::Term)),
/// then fuses the resulting ranked lists by item with Reciprocal Rank Fusion, returning
/// the fused top `top_k` (best first). Each retriever is `FnMut(&Q) -> Vec<(T, f64)>`
/// (best first), so any `Term` source plugs in WITHOUT this crate depending on it — pass
/// closures over the producers you already have:
///
/// - this crate's ANN: `&mut |t| index.nearest_term(t, graph, store, n)` (cosine);
/// - structural similarity: `&mut |t| sim.most_similar(t, n)` (`sparq-sim`, weighted
///   Jaccard);
/// - a lexical/BM25 list, a SPARQL-derived candidate set, …
///
/// Fusion is RRF, so the retrievers' score *scales never need reconciling* (cosine in
/// `[-1, 1]` vs Jaccard in `[0, 1]`) — only their ranks matter. The returned scores are
/// fused RRF scores, NOT either retriever's. An item is deduplicated to a single entry
/// whose score sums the `1/(k + rank)` contributions of every list that ranks it; an item
/// in only one list still appears (with that one contribution). A retriever returning an
/// empty list contributes nothing, so a partly-unembedded query degrades gracefully.
///
/// Use [`RRF_K`] for `k` unless you have a reason not to. To down-weight a noisier
/// retriever, collect its list and call [`fuse_rrf_weighted`] directly. `top_k == 0`,
/// no retrievers, or all-empty results yield an empty `Vec`.
///
/// # Panics
/// If `k ≤ 0` (propagated from [`fuse_rrf`]).
pub fn hybrid_search<Q, T: Clone + Eq + Hash>(
    query: &Q,
    top_k: usize,
    k: f64,
    retrievers: &mut [Retriever<'_, Q, T>],
) -> Vec<(T, f64)> {
    // Run each retriever once on the shared query, keeping its rank order.
    let lists: Vec<Vec<(T, f64)>> = retrievers.iter_mut().map(|r| r(query)).collect();
    let refs: Vec<&[(T, f64)]> = lists.iter().map(|l| l.as_slice()).collect();
    fuse_rrf(&refs, k, top_k)
}

/// **Relative score fusion** of two ranked lists: min-max normalizes each list's
/// scores to `[0, 1]` independently (a constant list normalizes to all-1.0), then
/// scores the union as `alpha·norm_a + (1 − alpha)·norm_b`, items missing from a list
/// contributing 0 for it. Returns the top `top_k`, best first; ties break by first
/// appearance (list `a` order, then list `b`), deterministically.
///
/// `alpha` is the text-vs-other balance: `1.0` = only `a`, `0.0` = only `b`, `0.5` =
/// equal. Because both lists are normalized first, `alpha` compares like with like
/// even when the raw scales differ (cosine vs Jaccard). Prefer [`fuse_rrf`] when you
/// don't want to tune anything.
pub fn fuse_scores<T: Clone + Eq + Hash>(
    a: &[(T, f64)],
    b: &[(T, f64)],
    alpha: f64,
    top_k: usize,
) -> Vec<(T, f64)> {
    assert!((0.0..=1.0).contains(&alpha), "alpha must be in [0, 1]");
    let mut acc: FxHashMap<T, (f64, usize)> = FxHashMap::default();
    let mut order = 0usize;
    for (weight, list) in [(alpha, a), (1.0 - alpha, b)] {
        for (item, norm) in normalized(list) {
            let entry = acc.entry(item.clone()).or_insert_with(|| {
                order += 1;
                (0.0, order)
            });
            entry.0 += weight * norm;
        }
    }
    top_n(acc, top_k)
}

/// Min-max normalization of one list's scores to `[0, 1]`; all-1.0 when the scores
/// are constant (rank information is all there is).
fn normalized<T: Clone>(list: &[(T, f64)]) -> Vec<(T, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, s) in list {
        lo = lo.min(*s);
        hi = hi.max(*s);
    }
    list.iter()
        .map(|(t, s)| {
            let n = if hi > lo { (s - lo) / (hi - lo) } else { 1.0 };
            (t.clone(), n)
        })
        .collect()
}

/// Sorts the accumulated `(score, first-seen)` map best-first (score desc, then
/// first-seen asc) and keeps `top_k`.
fn top_n<T>(acc: FxHashMap<T, (f64, usize)>, top_k: usize) -> Vec<(T, f64)> {
    let mut out: Vec<(T, f64, usize)> =
        acc.into_iter().map(|(t, (s, o))| (t, s, o)).collect();
    out.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.2.cmp(&y.2)));
    out.truncate(top_k);
    out.into_iter().map(|(t, s, _)| (t, s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_hand_computed() {
        let a: Vec<(&str, f64)> = vec![("x", 0.9), ("y", 0.8)];
        let b: Vec<(&str, f64)> = vec![("y", 0.3), ("z", 0.2)];
        let fused = fuse_rrf(&[&a, &b], 60.0, 10);
        // y: 1/62 + 1/61; x: 1/61; z: 1/62.
        assert_eq!(fused[0].0, "y");
        assert!((fused[0].1 - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
        assert_eq!(fused[1].0, "x");
        assert!((fused[1].1 - 1.0 / 61.0).abs() < 1e-12);
        assert_eq!(fused[2].0, "z");
        // Raw scores must not matter, only ranks.
        let a2: Vec<(&str, f64)> = vec![("x", 1000.0), ("y", -5.0)];
        let fused2 = fuse_rrf(&[&a2, &b], 60.0, 10);
        assert_eq!(
            fused.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            fused2.iter().map(|(t, _)| *t).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rrf_ties_break_by_first_appearance_and_top_k_truncates() {
        let a: Vec<(&str, f64)> = vec![("x", 1.0)];
        let b: Vec<(&str, f64)> = vec![("y", 1.0)];
        // x and y both score 1/61; x appeared first.
        let fused = fuse_rrf(&[&a, &b], 60.0, 10);
        assert_eq!(fused[0].0, "x");
        assert_eq!(fused[1].0, "y");
        assert_eq!(fuse_rrf(&[&a, &b], 60.0, 1).len(), 1);
    }

    #[test]
    fn scores_min_max_normalize_then_blend() {
        // Different scales: cosine-ish [0.2, 0.9] vs jaccard-ish [0.0, 0.1].
        let text: Vec<(&str, f64)> = vec![("x", 0.9), ("y", 0.2)];
        let strct: Vec<(&str, f64)> = vec![("y", 0.1), ("x", 0.0)];
        // norm: text x=1 y=0; struct y=1 x=0.
        let fused = fuse_scores(&text, &strct, 0.7, 10);
        assert_eq!(fused[0].0, "x");
        assert!((fused[0].1 - 0.7).abs() < 1e-12);
        assert_eq!(fused[1].0, "y");
        assert!((fused[1].1 - 0.3).abs() < 1e-12);
        // alpha = 0.5 ties; first appearance (text order) wins deterministically.
        let even = fuse_scores(&text, &strct, 0.5, 10);
        assert_eq!(even[0].0, "x");
    }

    #[test]
    fn scores_missing_items_and_constant_lists() {
        let a: Vec<(&str, f64)> = vec![("x", 0.5), ("y", 0.5)]; // constant → both 1.0
        let b: Vec<(&str, f64)> = vec![("z", 0.4)]; // singleton → 1.0
        let fused = fuse_scores(&a, &b, 0.6, 10);
        assert_eq!(fused.len(), 3);
        assert_eq!(fused[0].0, "x"); // 0.6, ties with y, x first
        assert!((fused[0].1 - 0.6).abs() < 1e-12);
        assert_eq!(fused[2].0, "z"); // 0.4 from list b only
        assert!((fused[2].1 - 0.4).abs() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "alpha must be in [0, 1]")]
    fn scores_rejects_bad_alpha() {
        let a: Vec<(&str, f64)> = vec![];
        let b: Vec<(&str, f64)> = vec![];
        fuse_scores(&a, &b, 1.5, 1);
    }

    #[test]
    fn weighted_rrf_hand_computed_and_reduces_to_plain() {
        let a: Vec<(&str, f64)> = vec![("x", 0.9), ("y", 0.8)];
        let b: Vec<(&str, f64)> = vec![("y", 0.3), ("z", 0.2)];
        // Unit weights ≡ plain RRF (scores and order).
        let plain = fuse_rrf(&[&a, &b], 60.0, 10);
        let unit = fuse_rrf_weighted(&[(&a, 1.0), (&b, 1.0)], 60.0, 10);
        assert_eq!(plain, unit);
        // Down-weight list b to 0.5: y = 1/61·0 + … hand-computed.
        let fused = fuse_rrf_weighted(&[(&a, 1.0), (&b, 0.5)], 60.0, 10);
        // x: 1/61; y: 1/62 + 0.5/61; z: 0.5/62.
        assert_eq!(fused[0].0, "y");
        assert!((fused[0].1 - (1.0 / 62.0 + 0.5 / 61.0)).abs() < 1e-12);
        assert_eq!(fused[1].0, "x");
        assert!((fused[1].1 - 1.0 / 61.0).abs() < 1e-12);
        assert_eq!(fused[2].0, "z");
        assert!((fused[2].1 - 0.5 / 62.0).abs() < 1e-12);
        // Weight strong enough to flip the consensus winner:
        // x = 3/61 ≈ 0.04918 must beat y = 3/62 + 0.01/61 ≈ 0.04855.
        let text_heavy = fuse_rrf_weighted(&[(&a, 3.0), (&b, 0.01)], 60.0, 10);
        assert_eq!(text_heavy[0].0, "x");
    }

    #[test]
    fn weighted_rrf_zero_weight_mutes_a_list() {
        let a: Vec<(&str, f64)> = vec![("x", 1.0)];
        let b: Vec<(&str, f64)> = vec![("y", 1.0), ("x", 0.5)];
        let fused = fuse_rrf_weighted(&[(&a, 0.0), (&b, 1.0)], 60.0, 10);
        // x still appears (rank 2 in b) but a contributes nothing.
        assert_eq!(fused[0].0, "y");
        assert!((fused[1].1 - 1.0 / 62.0).abs() < 1e-12);
    }

    // Regression for review 1874: an item that exists ONLY in a zero-weight (muted)
    // list must NOT surface as a standalone result. Previously a 0.0 contribution still inserted
    // it into the accumulator, so it could appear when top_k was large enough.
    #[test]
    fn weighted_rrf_zero_weight_injects_no_standalone_items() {
        let muted: Vec<(&str, f64)> = vec![("only_in_muted", 1.0), ("z", 1.0)];
        let live: Vec<(&str, f64)> = vec![("y", 1.0)];
        // Large top_k so nothing is dropped by truncation.
        let fused = fuse_rrf_weighted(&[(&muted, 0.0), (&live, 1.0)], 60.0, 100);
        assert_eq!(fused.len(), 1, "only the live list's item should appear");
        assert_eq!(fused[0].0, "y");
        assert!(fused.iter().all(|(t, _)| *t != "only_in_muted" && *t != "z"));
    }

    // When EVERY list is muted, the fused result is empty (not a pile of 0.0-scored
    // items). Review 1874.
    #[test]
    fn weighted_rrf_all_zero_weights_yields_empty() {
        let a: Vec<(&str, f64)> = vec![("x", 1.0), ("y", 1.0)];
        let b: Vec<(&str, f64)> = vec![("z", 1.0)];
        let fused = fuse_rrf_weighted(&[(&a, 0.0), (&b, 0.0)], 60.0, 100);
        assert!(fused.is_empty(), "all-muted fusion must be empty, got {fused:?}");
    }

    #[test]
    #[should_panic(expected = "weights must be finite and non-negative")]
    fn weighted_rrf_rejects_negative_weight() {
        let a: Vec<(&str, f64)> = vec![("x", 1.0)];
        fuse_rrf_weighted(&[(&a, -0.1)], 60.0, 1);
    }

    // sq-88c6 — hybrid_search: two retriever closures fused by RRF.

    #[test]
    fn hybrid_search_fuses_two_retrievers_consensus_wins() {
        // Stand-ins for the two retrievers (e.g. `index.nearest_term` and
        // `sim.most_similar`), each keyed on a query string and returning ranked items.
        // "b" is rank 2 in ANN and rank 1 in semantic → consensus; "a" is rank 1 in ANN
        // only; "d" is rank 2 in semantic only. Query type `Q = &str`, so each retriever
        // receives `&Q = &&str`.
        let ann = |_q: &&str| -> Vec<(&'static str, f64)> {
            vec![("a", 0.92), ("b", 0.85), ("c", 0.20)] // cosine scale
        };
        let semantic = |_q: &&str| -> Vec<(&'static str, f64)> {
            vec![("b", 0.61), ("d", 0.55), ("a", 0.10)] // Jaccard scale
        };
        let mut r0 = ann;
        let mut r1 = semantic;
        let fused =
            hybrid_search(&"query", 10, 60.0, &mut [&mut r0 as &mut dyn FnMut(&&str) -> _, &mut r1]);

        // Consensus (high in BOTH) beats high-in-only-one: b = rank2 ANN + rank1 semantic
        // = 1/62 + 1/61.
        assert_eq!(fused[0].0, "b");
        assert!((fused[0].1 - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
        // a: rank1 (ANN) + rank3 (semantic) = 1/61 + 1/63 — beats single-list items.
        assert_eq!(fused[1].0, "a");
        assert!((fused[1].1 - (1.0 / 61.0 + 1.0 / 63.0)).abs() < 1e-12);
        // Every item is deduplicated by key: a, b, c, d each appear once.
        assert_eq!(fused.len(), 4);
        let keys: Vec<&str> = fused.iter().map(|(t, _)| *t).collect();
        for want in ["a", "b", "c", "d"] {
            assert_eq!(keys.iter().filter(|&&t| t == want).count(), 1, "{want} deduped");
        }
        // RRF ignores raw scores: a term in ONE list still appears (d, from semantic only).
        assert!(keys.contains(&"d"));
        // top_k truncates the fused list.
        let truncated =
            hybrid_search(&"q", 2, 60.0, &mut [&mut r0 as &mut dyn FnMut(&&str) -> _, &mut r1]);
        assert_eq!(truncated.len(), 2);
    }

    #[test]
    fn hybrid_search_high_in_both_outranks_high_in_one() {
        // "shared" is rank 1 in retriever A and rank 1 in retriever B. "soloA" is strong
        // in A only; "soloB" strong in B only.
        let mut a = |_q: &i32| vec![("soloA", 9.9), ("shared", 0.0)];
        let mut b = |_q: &i32| vec![("soloB", 9.9), ("shared", 0.0)];
        let fused = hybrid_search(&0, 10, 60.0, &mut [&mut a as &mut dyn FnMut(&i32) -> _, &mut b]);
        // shared: 1/62 + 1/62 ≈ 0.03226 beats soloA/soloB at 1/61 ≈ 0.01639.
        assert_eq!(fused[0].0, "shared");
        assert!(fused[0].1 > fused[1].1, "consensus item must strictly outrank single-list items");
    }

    #[test]
    fn hybrid_search_edge_cases() {
        // (1) No retrievers → empty.
        let empty: Vec<(&str, f64)> = hybrid_search::<i32, &str>(&0, 10, 60.0, &mut []);
        assert!(empty.is_empty());

        // (2) One retriever empty, the other not: degrades to the non-empty list, in order.
        let mut none = |_q: &i32| Vec::<(&'static str, f64)>::new();
        let mut some = |_q: &i32| vec![("x", 1.0), ("y", 0.5)];
        let fused =
            hybrid_search(&0, 10, 60.0, &mut [&mut none as &mut dyn FnMut(&i32) -> _, &mut some]);
        assert_eq!(fused.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec!["x", "y"]);

        // (3) Both empty → empty.
        let mut none2 = |_q: &i32| Vec::<(&'static str, f64)>::new();
        let both_empty =
            hybrid_search(&0, 10, 60.0, &mut [&mut none as &mut dyn FnMut(&i32) -> _, &mut none2]);
        assert!(both_empty.is_empty());

        // (4) top_k == 0 → empty even with results.
        let zero = hybrid_search(&0, 0, 60.0, &mut [&mut some as &mut dyn FnMut(&i32) -> _]);
        assert!(zero.is_empty());
    }

    #[test]
    fn hybrid_search_term_in_one_list_only() {
        // "only_b" appears solely in retriever b; it must surface (one contribution), but
        // below any item ranked by both. "both" is rank 2 in a and rank 1 in b.
        let mut a = |_q: &i32| vec![("top_a", 1.0), ("both", 0.9)];
        let mut b = |_q: &i32| vec![("both", 0.8), ("only_b", 0.7)];
        let fused = hybrid_search(&0, 10, 60.0, &mut [&mut a as &mut dyn FnMut(&i32) -> _, &mut b]);
        // both: 1/62 + 1/61; top_a: 1/61; only_b: 1/62. Consensus first.
        assert_eq!(fused[0].0, "both");
        assert_eq!(fused[1].0, "top_a"); // single list, but rank 1 (1/61) > only_b's 1/62
        assert_eq!(fused[2].0, "only_b");
        assert!(
            (fused[2].1 - 1.0 / 62.0).abs() < 1e-12,
            "single-list item keeps its lone contribution"
        );
    }

    #[test]
    fn hybrid_search_identical_lists_double_every_contribution() {
        // Two identical retrievers: each item's fused score is exactly 2× its single
        // contribution, and the order is preserved.
        let list = || vec![("p", 1.0), ("q", 0.5), ("r", 0.1)];
        let mut a = |_q: &i32| list();
        let mut b = |_q: &i32| list();
        let fused = hybrid_search(&0, 10, 60.0, &mut [&mut a as &mut dyn FnMut(&i32) -> _, &mut b]);
        assert_eq!(fused.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec!["p", "q", "r"]);
        assert!((fused[0].1 - 2.0 / 61.0).abs() < 1e-12);
        assert!((fused[1].1 - 2.0 / 62.0).abs() < 1e-12);
        assert!((fused[2].1 - 2.0 / 63.0).abs() < 1e-12);
    }
}
