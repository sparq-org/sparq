//! (sq-7hx6, epic sq-3183) **Filtered-ANN pre-filter vs post-filter cost model** —
//! deciding when materialising a transitive [`IdMask`] and pre-filtering
//! the search is worth it, versus running the search unfiltered and dropping the disallowed hits
//! afterwards.
//!
//! This is the cost concern recorded as sq-ic0n ("when transitive masking pays"); sq-7hx6 IS that
//! cost model, so sq-ic0n is **subsumed** by this module.
//!
//! # The two strategies (and why they return the same answer)
//!
//! For a `vec:` neighbour variable constrained by a sub-BGP, the *answer* is unambiguous: the
//! top-`k` stored vectors **whose id is admitted by the constraint**, ranked by cosine. There are
//! two ways to compute it, and — this is the load-bearing invariant — **they return byte-identical
//! results** (same ids, same order; cosine ties break on ascending id in both):
//!
//! - **Pre-filter** ([`crate::filter::nearest_exact_filtered`]): scan only the masked ids and rank
//!   them. Work ≈ `|mask| · dim` distance computations. This is what the `vec:` rewrite has always
//!   done once a mask is derived.
//!
//! - **Post-filter** ([`postfilter_exact`]): run the *unfiltered* exact scan over the whole store
//!   (already a full sort by cosine), then walk that ranking dropping ids not in the mask until `k`
//!   admitted hits are collected. Work ≈ `store_len · dim` distance computations (the full scan)
//!   regardless of selectivity.
//!
//! Because the unfiltered scan is a *complete* ranking of the store, post-filtering it can never
//! under-fill `k` unless the mask itself admits fewer than `k` stored-and-embedded vectors — in
//! which case **both** strategies return the same short list (the pre-filter scan of the mask is
//! also short). So there is no over-fetch recall boundary for the exact-scan backend: post-filter
//! over a full ranking is answer-complete by construction. (The over-fetch *boundary* only bites an
//! **approximate** index that returns a bounded candidate list; see [`overfetch_target`] and the
//! honest caveat there — that path is a documented follow-up, not the exact backend used here.)
//!
//! # The cost the mask itself carries
//!
//! Both strategies still need to know *which* ids are admitted, but they pay for it differently.
//! Pre-filter **must** materialise the full mask (it scans it). Post-filter only needs *membership*
//! — and crucially, deriving the mask through the engine (a full SELECT over the sub-BGP, in the
//! `vec:` rewrite under the `vec-predicate` feature) costs roughly the sub-BGP's cardinality `c`
//! regardless. So the model's job is
//! NOT "mask vs no mask" (we derive the mask either way to stay answer-safe); it is **"having a mask,
//! is it cheaper to scan the mask (pre-filter) or scan the whole store and drop non-members
//! (post-filter)?"** — i.e. is `|mask| · dim` (plus the mask's hash-set build) below `store_len · dim`
//! by enough to be worth the worse cache behaviour of gathering scattered masked rows?
//!
//! # The heuristic (NOT optimal — estimates only)
//!
//! Let `m = |mask|`, `n = store_len`. Pre-filter touches `m` rows, post-filter touches `n`. Pre-filter
//! gathers *scattered* rows (random-access into the mmap) whereas post-filter streams the store
//! sequentially, so a masked row is modelled as `scatter_penalty ×` (default `2.0`) the cost of a
//! sequential row. **Pre-filter is chosen when**
//!
//! ```text
//!     m · scatter_penalty  ≤  n
//! ```
//!
//! i.e. when the mask is selective enough that even at the scatter penalty, scanning it beats a full
//! sequential scan. With the default penalty that is `m ≤ n / 2` — pre-filter for a mask admitting up
//! to half the store, post-filter for a broader one. `k` and `dim` cancel out of the *exact-scan*
//! comparison (both strategies do `dim`-width distance work per touched row and both then take `k`),
//! so they do not enter this crossover; they are surfaced in [`CostEstimate`] for callers wiring an
//! approximate backend (where `k`/over-fetch *do* matter) and to keep the estimate honest about what
//! it did and did not model.
//!
//! This is a **heuristic over a cost *estimate***, not a proven optimum: `scatter_penalty` is a
//! single modelled constant, real cache/SIMD behaviour is hardware-dependent, and the sub-BGP
//! cardinality is the *materialised* mask size (exact here, but an estimate in a planner that has not
//! evaluated it). The one thing it is NOT allowed to get wrong is the **answer**: whichever branch it
//! picks, the returned top-`k` is identical (asserted in the tests), so a mis-estimate costs only
//! throughput, never correctness.

use crate::ann::{nearest_exact, select_top_k_tiebreak};
use crate::filter::{nearest_exact_filtered, IdMask};
use crate::store::VectorStore;
use sparq_core::dict::Id;
use sparq_core::Graph;

/// Tuning for the [pre-filter ↔ post-filter](self) decision. Separate from
/// [`FilterConfig`](crate::filter::FilterConfig) (which tunes the *pre-filter ↔ filtered-traversal*
/// choice on the on-disk index): this governs the **`vec:` rewrite's** choice between scanning the
/// derived mask and scanning the whole store then dropping non-members.
#[derive(Clone, Copy, Debug)]
pub struct CostModel {
    /// How much more expensive a *scattered* masked-row access (random-access gather into the mmap)
    /// is than a *sequential* full-scan row. `1.0` = no penalty (decide purely on row counts `m` vs
    /// `n`); the default `2.0` reflects that gathering scattered rows defeats sequential prefetch and
    /// is the honestly conservative line — it errs toward post-filter (the strategy whose cost does
    /// not depend on a possibly-mis-estimated mask size) on a mid-selective mask. Must be ≥ 1.0.
    pub scatter_penalty: f32,
}

impl Default for CostModel {
    fn default() -> Self {
        // non-canonical constant: a single modelled scatter penalty, not a measured fit.
        // It only shifts the crossover (m ≤ n / scatter_penalty); it never affects the answer.
        CostModel {
            scatter_penalty: 2.0,
        }
    }
}

/// Which strategy the [`CostModel`] picked, with the estimate it was based on — returned so callers
/// (and tests) can assert the *decision*, not just the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Scan only the masked ids ([`nearest_exact_filtered`]). Chosen for a **selective** mask.
    PreFilter,
    /// Scan the whole store then drop non-masked hits ([`postfilter_exact`]). Chosen for a **broad**
    /// mask, where the mask scan would touch almost the whole store anyway (at the scatter penalty).
    PostFilter,
}

/// The cost estimate behind a [`Strategy`] choice — the modelled work of each branch, so the
/// decision is inspectable and testable. The units are *modelled row-touches* (not wall-clock); only
/// their ratio matters.
#[derive(Clone, Copy, Debug)]
pub struct CostEstimate {
    /// `|mask|` — masked rows the pre-filter would touch.
    pub mask_len: usize,
    /// `store_len` — rows the post-filter (full scan) would touch.
    pub store_len: usize,
    /// `k` — requested neighbours (does not enter the exact-scan crossover; recorded for an
    /// approximate backend where over-fetch makes it matter).
    pub k: usize,
    /// Modelled pre-filter cost: `mask_len · scatter_penalty`.
    pub prefilter_cost: f32,
    /// Modelled post-filter cost: `store_len` (one sequential touch per row).
    pub postfilter_cost: f32,
    /// The chosen strategy.
    pub strategy: Strategy,
}

impl CostModel {
    /// Estimate both branches' cost for a mask of `mask_len` over a `store_len`-vector store and
    /// pick the cheaper. `k` does not change the exact-scan crossover (see the [module docs](self))
    /// but is carried into the [`CostEstimate`].
    ///
    /// Pre-filter wins iff `mask_len · scatter_penalty ≤ store_len`.
    pub fn decide(&self, mask_len: usize, store_len: usize, k: usize) -> CostEstimate {
        let prefilter_cost = mask_len as f32 * self.scatter_penalty;
        let postfilter_cost = store_len as f32;
        let strategy = if prefilter_cost <= postfilter_cost {
            Strategy::PreFilter
        } else {
            Strategy::PostFilter
        };
        CostEstimate {
            mask_len,
            store_len,
            k,
            prefilter_cost,
            postfilter_cost,
            strategy,
        }
    }
}

/// **Post-filter exact top-`k`**: rank the WHOLE store unfiltered, then keep only ids admitted by
/// `mask`, best-first, up to `k`. The alternative to [`nearest_exact_filtered`] when the mask is too
/// broad for pre-filtering to pay.
///
/// # Identical result to the pre-filter (the load-bearing invariant)
///
/// [`nearest_exact`] returns a *complete* cosine ranking of every stored vector (ties on ascending
/// id). Dropping the non-masked ids from that ranking and taking the first `k` yields **exactly** the
/// same `(id, score)` list, in the same order, as [`nearest_exact_filtered`] (which ranks the masked
/// ids directly with the identical comparator). So this is answer-safe with **no over-fetch boundary
/// for the exact backend**: a full ranking cannot under-fill `k` unless the mask admits fewer than
/// `k` stored vectors, in which case the pre-filter scan is equally short and the two agree on the
/// short list. (An *approximate* index would return a bounded candidate list and could under-fill —
/// see [`overfetch_target`].)
///
/// An empty mask, or an all-zero query, returns no results — the same degenerate-case contract as
/// [`nearest_exact_filtered`].
pub fn postfilter_exact(
    store: &VectorStore,
    query: &[f32],
    mask: &IdMask,
    k: usize,
) -> Vec<(Id, f32)> {
    if mask.is_empty() || k == 0 || query.iter().all(|&v| v == 0.0) {
        return Vec::new();
    }
    // Full unfiltered ranking (best-first, deterministic ties). `k = store.len()` so it is the
    // COMPLETE ranking — post-filtering a complete ranking is answer-complete (no over-fetch needed).
    nearest_exact(store, query, store.len())
        .into_iter()
        .filter(|&(id, _)| mask.contains(id))
        .take(k)
        .collect()
}

/// **Cost-model-driven filtered top-`k`**: pick pre-filter or post-filter by [`CostModel::decide`],
/// run the chosen branch, and return both the hits and the [`CostEstimate`] (so the decision is
/// observable). Both branches return the **identical** answer (see [`postfilter_exact`]); the model
/// only trades throughput.
///
/// This is the seam the `vec:` rewrite uses when both `vec-predicate` and `filtered-ann` are on: it
/// already derived the `mask` (it must, to stay answer-safe), and this decides how to *use* it.
pub fn nearest_filtered_costed(
    store: &VectorStore,
    query: &[f32],
    mask: &IdMask,
    k: usize,
    model: &CostModel,
) -> (Vec<(Id, f32)>, CostEstimate) {
    let est = model.decide(mask.len(), store.len(), k);
    let hits = match est.strategy {
        Strategy::PreFilter => nearest_exact_filtered(store, query, mask, k),
        Strategy::PostFilter => postfilter_exact(store, query, mask, k),
    };
    (hits, est)
}

/// **Cost-model-driven filtered top-`k` with the VG-TIE-1 boundary tie-break** —
/// [`nearest_filtered_costed`] upgraded to the answer-exact mode contract of
/// `site/specs/sparql-vector-genai.typ`: result-set *membership* at a score tie on the k boundary
/// is decided by ascending Unicode-codepoint order of the tied candidates' canonical N-Triples
/// serialisations (the same rule as [`nearest_exact_tiebreak`](crate::ann::nearest_exact_tiebreak)),
/// computed over the **mask-admitted candidate pool** with `exclude` (the seed of a query-by-node
/// search; `None` for a query-by-vector search) dropped **before** the boundary is determined.
/// This keeps VG-FILT-2 exact in answer-exact mode: the filtered top-k equals post-filtering the
/// unfiltered tie-broken retrieval by the same mask, because both sides apply the identical
/// membership rule to the identical admitted pool.
///
/// Both cost-model branches rank the **complete** admitted pool — pre-filter scans the mask,
/// post-filter scans the whole store and drops non-members; the two complete rankings are
/// byte-identical (see [`postfilter_exact`]) — so the strategy choice is never observable in the
/// answer, exactly as for [`nearest_filtered_costed`]; the tie-break then reselects only the
/// boundary tie group. Degenerate cases (`k = 0`, an empty mask, an all-zero query) return no
/// results, the same contract as [`nearest_filtered_costed`].
///
/// Fail-closed like [`nearest_exact_tiebreak`](crate::ann::nearest_exact_tiebreak): a candidate
/// term containing a blank node (whose document-local label has no cross-implementation
/// tie-break key) is an `Err` wherever its term — rather than its score alone — decides
/// membership: admitted into the top-k, or a member of the boundary tie group.
pub fn nearest_filtered_costed_tiebreak(
    store: &VectorStore,
    graph: &Graph,
    query: &[f32],
    mask: &IdMask,
    k: usize,
    exclude: Option<Id>,
    model: &CostModel,
) -> Result<(Vec<(Id, f32)>, CostEstimate), String> {
    let est = model.decide(mask.len(), store.len(), k);
    if k == 0 {
        return Ok((Vec::new(), est));
    }
    // The COMPLETE admitted ranking (`k = mask.len()` cannot truncate: at most |mask| ids are
    // admitted), so the boundary tie group is fully present whichever branch ranked it.
    let mut scored = match est.strategy {
        Strategy::PreFilter => nearest_exact_filtered(store, query, mask, mask.len()),
        Strategy::PostFilter => postfilter_exact(store, query, mask, mask.len()),
    };
    // Seed exclusion BEFORE the boundary is determined, so the boundary is computed over the
    // true candidate pool — matching the unfiltered `nearest_exact_tiebreak` contract.
    if let Some(seed) = exclude {
        scored.retain(|&(id, _)| id != seed);
    }
    Ok((select_top_k_tiebreak(graph, scored, k)?, est))
}

/// How many candidates an **approximate** index must over-fetch to expect `k` survivors
/// after post-filtering at selectivity `mask_len / store_len`. `ceil(k / selectivity)`, clamped to
/// the store size and to at least `k`.
///
/// This is **not used by the exact-scan backend** (which post-filters a *complete* ranking and so
/// never under-fills). It is provided — and documented as a HEURISTIC with a real recall boundary —
/// for a future approximate filtered `vec:` path: an approximate index returns a bounded candidate
/// list, so post-filtering it can under-fill `k` if too few survive. The honest handling is
/// **iterative over-fetch**: fetch `overfetch_target(k, …)`, and if fewer than `k` survive, re-fetch
/// a larger candidate list (doubling) until `k` survive or the store is exhausted — at which point
/// the mask genuinely admits fewer than `k` and the short list is correct. The recall boundary is
/// that a *single* over-fetch sized by the average selectivity can under-fill when the admitted ids
/// cluster *below* the fetched prefix; iterative over-fetch removes that boundary at the cost of
/// extra index probes.
///
/// (sq-ip3a) That iterative over-fetch path now exists:
/// [`nearest_filtered_overfetch`](crate::backend::nearest_filtered_overfetch) over the pluggable
/// [`AnnBackend`](crate::backend::AnnBackend) seam (exact default; approximate
/// [`ApproxBackend`](crate::backend::ApproxBackend) behind `approx-ann`). This function sizes its
/// FIRST fetch. Honesty unchanged: the approximate backend's recall is `< 1.0` — over-fetch fixes
/// under-fill, not the index's inherent misses.
pub fn overfetch_target(k: usize, mask_len: usize, store_len: usize) -> usize {
    if mask_len == 0 || store_len == 0 {
        return k;
    }
    let selectivity = mask_len as f64 / store_len as f64;
    let target = (k as f64 / selectivity).ceil() as usize;
    target.max(k).min(store_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::VectorStore;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("sparq_vec_cost_{}_{name}.spqv", std::process::id()));
        p
    }

    /// A small store of points on the unit circle, ids 1..=n, each at angle 2πi/n.
    fn ring_store(name: &str, n: usize) -> VectorStore {
        let mut store = VectorStore::create(tmp(name), 2).unwrap();
        for i in 0..n {
            let theta = std::f32::consts::TAU * (i as f32) / (n as f32);
            // id 0 is reserved-ish; use i+1 as the dict id.
            store
                .put((i as u32) + 1, &[theta.cos(), theta.sin()])
                .unwrap();
        }
        store.finalize().unwrap();
        store
    }

    /// The decision crossover: with the default scatter_penalty 2.0, pre-filter iff m ≤ n/2.
    #[test]
    fn decision_crossover_default_penalty() {
        let m = CostModel::default();
        // n = 100. m ≤ 50 → pre-filter; m > 50 → post-filter.
        assert_eq!(m.decide(10, 100, 5).strategy, Strategy::PreFilter);
        assert_eq!(m.decide(50, 100, 5).strategy, Strategy::PreFilter); // boundary: 50·2 = 100 ≤ 100
        assert_eq!(m.decide(51, 100, 5).strategy, Strategy::PostFilter);
        assert_eq!(m.decide(100, 100, 5).strategy, Strategy::PostFilter);
    }

    /// scatter_penalty 1.0 decides purely on row counts (m ≤ n → pre-filter).
    #[test]
    fn penalty_one_decides_on_row_counts() {
        let m = CostModel {
            scatter_penalty: 1.0,
        };
        assert_eq!(m.decide(99, 100, 5).strategy, Strategy::PreFilter);
        assert_eq!(m.decide(100, 100, 5).strategy, Strategy::PreFilter);
        // m can't exceed n in practice, but the comparison is still well-defined.
    }

    /// CORRECTNESS: pre-filter and post-filter return BYTE-IDENTICAL (id, score) lists for a
    /// selective mask, a broad mask, and k larger than the mask. This is the load-bearing invariant.
    #[test]
    fn prefilter_and_postfilter_identical() {
        let store = ring_store("identical", 64);
        let query = [1.0f32, 0.0];
        // A few masks of different selectivity, including ones smaller than k.
        for ids in [
            vec![1u32, 5, 9, 13, 33],           // selective, scattered
            (1u32..=64).step_by(2).collect(),   // ~half
            (1u32..=64).collect(),              // full
            vec![2u32, 4],                      // smaller than k=5
        ] {
            let mask: IdMask = ids.into_iter().collect();
            for k in [1usize, 3, 5, 10] {
                let pre = nearest_exact_filtered(&store, &query, &mask, k);
                let post = postfilter_exact(&store, &query, &mask, k);
                assert_eq!(
                    pre, post,
                    "pre/post-filter must be identical (mask_len={}, k={k})",
                    mask.len()
                );
            }
        }
    }

    /// The costed entry point returns the same hits regardless of which branch the model picks —
    /// force each branch via the penalty and check the hits match the direct pre-filter.
    #[test]
    fn costed_entry_point_branch_agnostic_result() {
        let store = ring_store("costed", 64);
        let query = [0.0f32, 1.0];
        let mask: IdMask = (1u32..=64).step_by(3).collect();
        let k = 4;

        // Force pre-filter (penalty 1.0, m ≤ n).
        let always_pre = CostModel {
            scatter_penalty: 1.0,
        };
        let (pre_hits, pre_est) = nearest_filtered_costed(&store, &query, &mask, k, &always_pre);
        assert_eq!(pre_est.strategy, Strategy::PreFilter);

        // Force post-filter (huge penalty makes any non-empty mask "broad").
        let always_post = CostModel {
            scatter_penalty: 1e6,
        };
        let (post_hits, post_est) = nearest_filtered_costed(&store, &query, &mask, k, &always_post);
        assert_eq!(post_est.strategy, Strategy::PostFilter);

        assert_eq!(
            pre_hits, post_hits,
            "the model's branch choice must never change the answer"
        );
        // And both equal the ground-truth pre-filter.
        assert_eq!(pre_hits, nearest_exact_filtered(&store, &query, &mask, k));
    }

    /// Degenerate cases match nearest_exact_filtered's contract.
    #[test]
    fn postfilter_degenerate_cases() {
        let store = ring_store("degenerate", 16);
        let mask: IdMask = (1u32..=16).collect();
        // empty mask
        assert!(postfilter_exact(&store, &[1.0, 0.0], &IdMask::new(), 5).is_empty());
        // zero query
        assert!(postfilter_exact(&store, &[0.0, 0.0], &mask, 5).is_empty());
        // k = 0
        assert!(postfilter_exact(&store, &[1.0, 0.0], &mask, 0).is_empty());
    }

    /// overfetch_target: ceil(k / selectivity), clamped. (Backend-agnostic arithmetic check.)
    #[test]
    fn overfetch_target_arithmetic() {
        // 10% selective, k=5 → fetch ~50.
        assert_eq!(overfetch_target(5, 100, 1000), 50);
        // full mask → fetch exactly k.
        assert_eq!(overfetch_target(5, 1000, 1000), 5);
        // never below k, never above store.
        assert_eq!(overfetch_target(5, 1, 10), 10); // 5/(1/10)=50, clamped to store_len 10
        assert_eq!(overfetch_target(5, 0, 1000), 5); // empty mask guard
    }

    // Direct unit gate for `nearest_filtered_costed_tiebreak`: VG-TIE-1 boundary
    // membership over the mask-ADMITTED pool (seed excluded before the boundary), identical from
    // both cost-model branches. The tie group mixes an IRI with two inlined numeric literals so
    // dictionary-id order and N-Triples codepoint order genuinely disagree (see the ann.rs
    // tiebreak fixture): id order would admit `<http://ex/z-tied>`, the VG-TIE-1 rule admits
    // `"10"^^xsd:integer`.
    mod filtered_tiebreak {
        use super::super::{nearest_filtered_costed_tiebreak, CostModel, IdMask, Strategy};
        use crate::store::VectorStore;
        use oxrdf::{NamedNode, Term};
        use sparq_core::Graph;

        fn fixture(name: &str) -> (Graph, VectorStore, IdMask) {
            let g = Graph::load_str(
                r#"
                <http://ex/top> <http://ex/num> "10"^^<http://www.w3.org/2001/XMLSchema#integer> .
                <http://ex/top> <http://ex/num> "2"^^<http://www.w3.org/2001/XMLSchema#integer> .
                <http://ex/z-tied> <http://ex/label> "z" .
                <http://ex/out> <http://ex/label> "out" .
                "#,
                "ntriples",
            )
            .unwrap();
            let id = |s: &str| g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap())).unwrap();
            let num = |n: &str| {
                g.id_of(&Term::Literal(oxrdf::Literal::new_typed_literal(
                    n,
                    oxrdf::vocab::xsd::INTEGER,
                )))
                .unwrap()
            };
            let path = std::env::temp_dir().join(format!(
                "sparq_vec_cost_tiebreak_{}_{name}.spqv",
                std::process::id()
            ));
            let mut store = VectorStore::create(path, 2).unwrap();
            store.put(id("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0, admitted
            store.put(id("http://ex/z-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6, admitted
            store.put(num("10"), &[0.6, -0.8]).unwrap(); // cos 0.6, admitted
            store.put(num("2"), &[3.0, 4.0]).unwrap(); // cos 0.6 (scaled), admitted
            store.put(id("http://ex/out"), &[0.9, 0.1]).unwrap(); // cos ~0.99 but NOT admitted
            let mask: IdMask =
                [id("http://ex/top"), id("http://ex/z-tied"), num("10"), num("2")]
                    .into_iter()
                    .collect();
            (g, store, mask)
        }

        fn terms(g: &Graph, hits: &[(sparq_core::dict::Id, f32)]) -> Vec<String> {
            hits.iter().map(|&(id, _)| g.dict.term(id).to_string()).collect()
        }

        const TEN: &str = "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>";

        /// k=2 over the admitted pool: `top` (1.0) plus ONE slot for the three-way 0.6 tie —
        /// the smallest N-Triples key `"10"^^xsd:integer` wins, from BOTH cost-model branches;
        /// the closer non-admitted `out` never appears (VG-FILT-3).
        #[test]
        fn boundary_tie_membership_by_ntriples_key_from_both_branches() {
            let (g, store, mask) = fixture("both_branches");
            let query = [1.0f32, 0.0];
            let always_pre = CostModel { scatter_penalty: 1.0 };
            let always_post = CostModel { scatter_penalty: 1e6 };
            let (pre, pre_est) =
                nearest_filtered_costed_tiebreak(&store, &g, &query, &mask, 2, None, &always_pre)
                    .unwrap();
            let (post, post_est) =
                nearest_filtered_costed_tiebreak(&store, &g, &query, &mask, 2, None, &always_post)
                    .unwrap();
            assert_eq!(pre_est.strategy, Strategy::PreFilter);
            assert_eq!(post_est.strategy, Strategy::PostFilter);
            assert_eq!(pre, post, "the branch choice must never change the answer");
            assert_eq!(
                terms(&g, &pre),
                vec!["<http://ex/top>".to_string(), TEN.to_string()],
                "boundary membership must follow the N-Triples key, not id order"
            );
        }

        /// The seed is dropped from the admitted pool BEFORE the boundary is determined: with
        /// `top` excluded, k=1 lands directly on the three-way tie → `"10"^^xsd:integer`.
        #[test]
        fn seed_excluded_before_the_boundary() {
            let (g, store, mask) = fixture("seed_excluded");
            let top = g.id_of(&Term::NamedNode(NamedNode::new("http://ex/top").unwrap())).unwrap();
            let (hits, _est) = nearest_filtered_costed_tiebreak(
                &store,
                &g,
                &[1.0, 0.0],
                &mask,
                1,
                Some(top),
                &CostModel::default(),
            )
            .unwrap();
            assert_eq!(terms(&g, &hits), vec![TEN.to_string()]);
        }

        /// (#2445 review) A blank node admitted by the mask and landing in the
        /// boundary tie group is a fail-closed error, not a ranked candidate — its
        /// document-local label has no cross-implementation tie-break key.
        #[test]
        fn masked_blank_node_in_the_tie_group_is_a_fail_closed_error() {
            use oxrdf::BlankNode;
            let g = Graph::load_str(
                r#"
                <http://ex/top> <http://ex/p> "t" .
                <http://ex/a-tied> <http://ex/p> "a" .
                _:tied <http://ex/p> "b" .
                "#,
                "ntriples",
            )
            .unwrap();
            let id = |s: &str| g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap())).unwrap();
            let bnode = g.id_of(&Term::BlankNode(BlankNode::new_unchecked("tied"))).unwrap();
            let path = std::env::temp_dir().join(format!(
                "sparq_vec_cost_tiebreak_bnode_{}.spqv",
                std::process::id()
            ));
            let mut store = VectorStore::create(path, 2).unwrap();
            store.put(id("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0
            store.put(id("http://ex/a-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6
            store.put(bnode, &[0.6, -0.8]).unwrap(); // cos 0.6 — tied blank node
            let mask: IdMask =
                [id("http://ex/top"), id("http://ex/a-tied"), bnode].into_iter().collect();
            // k=2: one slot for the two-way 0.6 tie the blank node sits in → Err.
            let err = nearest_filtered_costed_tiebreak(
                &store,
                &g,
                &[1.0, 0.0],
                &mask,
                2,
                None,
                &CostModel::default(),
            )
            .unwrap_err();
            assert!(err.contains("blank node"), "got: {}", err);
        }

        /// Degenerate cases keep the `nearest_filtered_costed` contract: k=0, an empty mask,
        /// and an all-zero query all yield no results.
        #[test]
        fn degenerate_cases_are_empty() {
            let (g, store, mask) = fixture("degenerate");
            let m = CostModel::default();
            let (hits, _) =
                nearest_filtered_costed_tiebreak(&store, &g, &[1.0, 0.0], &mask, 0, None, &m)
                    .unwrap();
            assert!(hits.is_empty(), "k=0 must yield no results");
            let (hits, _) = nearest_filtered_costed_tiebreak(
                &store,
                &g,
                &[1.0, 0.0],
                &IdMask::new(),
                3,
                None,
                &m,
            )
            .unwrap();
            assert!(hits.is_empty(), "an empty mask must yield no results");
            let (hits, _) =
                nearest_filtered_costed_tiebreak(&store, &g, &[0.0, 0.0], &mask, 3, None, &m)
                    .unwrap();
            assert!(hits.is_empty(), "an all-zero query must yield no results");
        }
    }

    /// Sanity: the post-filter never returns an id outside the mask, and respects best-first order.
    #[test]
    fn postfilter_respects_mask_and_order() {
        let store = ring_store("respect", 32);
        let query = [1.0f32, 0.0];
        let mask: IdMask = vec![3u32, 7, 11, 19].into_iter().collect();
        let hits = postfilter_exact(&store, &query, &mask, 10);
        for &(id, _) in &hits {
            assert!(mask.contains(id), "post-filter returned an unmasked id {id}");
        }
        // Scores are non-increasing (best-first).
        for w in hits.windows(2) {
            assert!(w[0].1 >= w[1].1, "post-filter not best-first: {hits:?}");
        }
        // Identical to ground truth, including the recompute path.
        assert_eq!(hits, nearest_exact_filtered(&store, &query, &mask, 10));
    }
}
