//! Dictionary-id-keyed importance scores — the adapter between a *ranker* in this crate
//! and a consumer keyed by sparq-core dictionary [`Id`]s.
//! [OPUS-5] sq-lsp7k.9.4.
//!
//! Every algorithm here runs over a [`NodeGraph`], whose results are indexed by **dense
//! node index** (`0..n`). A consumer that already speaks dictionary ids — notably
//! `sparq_text::CompletionIndex::complete`, which takes an
//! `Option<&FxHashMap<Id, f64>>` of injected importance scores — cannot use that vector
//! directly: it must be re-keyed through [`NodeGraph::dict_id`]. That re-keying is the
//! whole of this module.
//!
//! # Scale: use a [`ScoreBudget`]
//!
//! The score map is the caller's *resident* structure — it is built once per graph
//! generation and then consulted on every interactive request — so its size, not the
//! ranker's, is what matters. A [`ScoreBudget::All`] map holds one entry per graph node,
//! which on a very large graph is a substantial resident allocation. Prefer
//! [`ScoreBudget::TopK`]: entities missing from the map score `0.0` in
//! `CompletionIndex::complete`, so a top-k map ranks the entities that can actually win a
//! completion slot and leaves the long tail to the index's own deterministic
//! key-then-id ordering. The ranker still runs over the whole graph either way — only the
//! retained map is truncated, so the scores kept are the true global ranks.
//!
//! # What is *not* scored
//!
//! [`NodeGraph`] is predicate-erased: a term that appears **only** as a predicate is not a
//! node and therefore gets no score, even though `CompletionIndex` will happily complete
//! it. Such a term ranks below every scored entity rather than being hidden. Literal
//! objects are likewise unscored under the default [`NodeFilter::EntitiesOnly`]
//! projection, so a `rdfs:label` completion is ranked by the score of its *subject*, which
//! is the entity the caller ultimately selects.
//!
//! [`NodeFilter::EntitiesOnly`]: crate::NodeFilter::EntitiesOnly

use rustc_hash::FxHashMap;
use sparq_core::dict::Id;
use sparq_core::Graph;

use crate::graph::NodeGraph;
use crate::pagerank::{pagerank, PageRankConfig};

/// How many entities a score map retains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScoreBudget {
    /// Retain every node. The map holds one entry per node in the [`NodeGraph`].
    #[default]
    All,
    /// Retain only the `k` highest-scoring nodes, ties broken by ascending node index
    /// (which is canonical ascending-term order — see [`NodeGraph::build_with`]), so the
    /// retained set is deterministic. `TopK(0)` yields an empty map.
    TopK(usize),
}

/// Re-keys a node-indexed score vector by sparq-core dictionary [`Id`].
///
/// `ranks[i]` is taken to be the score of node `i` in `g`. The result is exactly the
/// `scores` argument `sparq_text::CompletionIndex::complete` expects.
///
/// Non-finite scores (`NaN`, `±inf`) are dropped: they cannot participate in a total
/// ordering, and a `NaN` reaching the consumer's comparator would make the ranking depend
/// on candidate arrival order. A ranker in this crate never produces one, so this is a
/// defence against a hand-supplied vector, not an expected path.
///
/// # Panics
///
/// Panics when `ranks.len() != g.len()` — a mismatched vector would silently mis-attribute
/// every score to the wrong entity.
pub fn scores_by_dict_id(g: &NodeGraph, ranks: &[f64], budget: ScoreBudget) -> FxHashMap<Id, f64> {
    assert_eq!(
        ranks.len(),
        g.len(),
        "score vector length {} does not match the node graph's {} nodes",
        ranks.len(),
        g.len()
    );

    match budget {
        // Streams straight into the map — no `O(n)` intermediate.
        ScoreBudget::All => collect_scores(g, ranks.iter().copied().enumerate()),
        ScoreBudget::TopK(0) => FxHashMap::default(),
        ScoreBudget::TopK(k) => {
            // Mirrors `centrality::top_k`: sort by score descending, then node index
            // ascending. The `O(n)` index vector is transient — it is dropped before the
            // caller's (bounded) map becomes resident.
            //
            // Non-finite scores are dropped BEFORE the truncation, not after: `total_cmp`
            // sorts `NaN` above `+inf` above every finite score, so a post-truncation
            // filter would let them occupy — and then vacate — the k retained slots,
            // silently shrinking the map below `min(k, finite_count)`.
            let mut ranked: Vec<(usize, f64)> = ranks
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, score)| score.is_finite())
                .collect();
            ranked.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            ranked.truncate(k);
            collect_scores(g, ranked.into_iter())
        }
    }
}

/// Drops non-finite scores and re-keys the rest from node index to dictionary [`Id`].
fn collect_scores(g: &NodeGraph, scored: impl Iterator<Item = (usize, f64)>) -> FxHashMap<Id, f64> {
    scored
        .filter(|(_, score)| score.is_finite())
        .map(|(i, score)| (g.dict_id(i), score))
        .collect()
}

/// Builds the entity-graph projection of `graph`, runs [`pagerank`], and returns the
/// resulting scores keyed by dictionary [`Id`] — the once-per-graph-generation call a
/// completion or ranking service makes.
///
/// Equivalent to `scores_by_dict_id(&ng, &pagerank(&ng, cfg), budget)` over
/// `NodeGraph::build(graph)`; the intermediate projection and rank vector are dropped, so
/// only the score map stays resident. Callers that need the projection for other
/// algorithms should build it once themselves and call [`scores_by_dict_id`] instead of
/// paying for a second projection here.
///
/// The result is deterministic for a given graph: the projection is in canonical
/// ascending-term order and PageRank uses no RNG.
pub fn pagerank_scores_by_dict_id(
    graph: &Graph,
    cfg: PageRankConfig,
    budget: ScoreBudget,
) -> FxHashMap<Id, f64> {
    let ng = NodeGraph::build(graph);
    let ranks = pagerank(&ng, cfg);
    scores_by_dict_id(&ng, &ranks, budget)
}
