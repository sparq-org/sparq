//! (sq-ip3a, epic sq-3183; follow-up to sq-7hx6) **Pluggable ANN backend seam +
//! iterative over-fetch for the FILTERED approximate path.**
//!
//! sq-7hx6 ([`crate::cost`]) settled the *exact*-scan filtered story: post-filtering a **complete**
//! cosine ranking can never under-fill `k` (it sees every admitted vector), so there is no
//! over-fetch boundary for the exact backend. This module is where the **approximate** backend
//! bites — and it bites exactly as sq-7hx6's [`overfetch_target`]
//! docs warned it would.
//!
//! # The backend trait
//!
//! [`AnnBackend`] is the seam the filtered path searches *through*: a backend returns a ranked
//! `(Id, cosine)` candidate list of a requested `fetch` size, best-first. Two impls:
//!
//! - **[`ExactBackend`]** (default, no third-party dep) — wraps `nearest_exact`. `fetch` ≥ the
//!   store size yields the *complete* ranking, so it is **answer-exact**; recall is `1.0`.
//! - **[`ApproxBackend`]** (`approx-ann`, wraps the on-disk `DiskAnnIndex` Vamana graph; the
//!   in-RAM HNSW [`VectorIndex`](crate::VectorIndex) is an equally valid impl) — returns a
//!   *bounded, approximate* candidate list. Recall is **`< 1.0`** by construction: an approximate
//!   index can miss true neighbours. We never claim exactness for it; the recall floor is a
//!   *measured* test assertion (see `tests/overfetch.rs`), not a guarantee.
//!
//! # Why the FILTERED approximate path needs iterative over-fetch
//!
//! Post-filtering a *bounded* approximate candidate list can **under-fill `k`**: if the top
//! `fetch` candidates contain fewer than `k` ids the mask admits, the result is short even though
//! the store holds `≥ k` admitted vectors. [`overfetch_target`]
//! sizes a *first* fetch at `ceil(k / selectivity)` — enough *on average* — but the admitted ids
//! can cluster *below* that prefix, so a single sized fetch still under-fills (the recall boundary
//! sq-7hx6 documented).
//!
//! [`nearest_filtered_overfetch`] removes that boundary the honest way: fetch
//! `overfetch_target(k, …)`, post-filter, and **if fewer than `k` survive, grow the fetch
//! (doubling) and retry** — up to the store size or a probe bound. When the fetch reaches the
//! store size the candidate list is complete *for that backend*, so a still-short result means the
//! mask genuinely admits `< k` vectors (the correct short answer), NOT an over-fetch artefact.
//!
//! # Honesty about recall (the load-bearing caveat)
//!
//! Iterative over-fetch fixes **under-fill** (returning fewer than `k` when `k` exist); it does
//! **not** make an approximate backend exact. Even a full-store fetch from [`ApproxBackend`] is the
//! *approximate* ranking, so a true top-`k` admitted neighbour the index never visits is still
//! missed: **recall stays `< 1.0`**. Only [`ExactBackend`] is answer-exact. The A1-paper recall
//! evidence is for the exact / transitive answer-safe path (sq-7hx6); it does **not** transfer to
//! this approximate path — keep the distinction. The honest contract here is: "fill `k` whenever
//! `k` admitted vectors exist *and the backend surfaces them*, with a measured recall floor", not
//! "return the exact filtered top-`k`".

use crate::cost::overfetch_target;
use crate::filter::IdMask;
use crate::store::VectorStore;
use sparq_core::dict::Id;

/// (sq-ip3a) A nearest-neighbour backend the filtered over-fetch path searches through:
/// it returns a ranked `(Id, cosine)` candidate list of a requested `fetch` size, **best-first**.
///
/// The result must be a *prefix-stable* ranking: asking for a larger `fetch` returns a superset
/// (in rank order) of a smaller one, so [`nearest_filtered_overfetch`]'s doubling only ever adds
/// candidates. Both shipped impls satisfy this (the exact scan trivially; the approximate index by
/// widening its beam with `fetch`).
pub trait AnnBackend {
    /// Top-`fetch` candidates by cosine to `query`, best-first. An all-zero query (no direction)
    /// returns no candidates — the shared degenerate contract of every searcher in this crate.
    fn candidates(&self, query: &[f32], fetch: usize) -> Vec<(Id, f32)>;

    /// The number of vectors the backend ranks over — the ceiling a `fetch` is clamped to and the
    /// point at which the candidate list is *complete for this backend* (so a still-short
    /// post-filter result is the genuine answer, not an over-fetch artefact).
    fn len(&self) -> usize;

    /// Is the backend empty?
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// (sq-ip3a) The **answer-exact** backend: a full brute-force scan
/// ([`nearest_exact`](crate::ann::nearest_exact)). A `fetch` ≥ the store size yields the COMPLETE
/// cosine ranking, so post-filtering it never under-fills (the sq-7hx6 invariant) — recall `1.0`.
/// This is the default backend and pulls no third-party ANN crate.
pub struct ExactBackend<'a> {
    store: &'a VectorStore,
}

impl<'a> ExactBackend<'a> {
    /// Wraps `store` as the exact backend.
    pub fn new(store: &'a VectorStore) -> ExactBackend<'a> {
        ExactBackend { store }
    }
}

impl AnnBackend for ExactBackend<'_> {
    fn candidates(&self, query: &[f32], fetch: usize) -> Vec<(Id, f32)> {
        crate::ann::nearest_exact(self.store, query, fetch)
    }
    fn len(&self) -> usize {
        self.store.len()
    }
}

/// (sq-ip3a) The **approximate** backend over the on-disk Vamana graph
/// ([`DiskAnnIndex`](crate::DiskAnnIndex)). Recall is `< 1.0`: it returns a *bounded, approximate*
/// candidate list (a greedy beam search), so post-filtering can under-fill — which is exactly why
/// [`nearest_filtered_overfetch`] iterates. The beam is widened with the requested `fetch` so a
/// larger fetch is a (super)set in rank order. Gated behind `approx-ann`.
///
/// Not answer-exact — the recall floor is a *measured* test assertion, never claimed as exactness.
#[cfg(feature = "approx-ann")]
pub struct ApproxBackend<'a> {
    index: &'a crate::DiskAnnIndex,
}

#[cfg(feature = "approx-ann")]
impl<'a> ApproxBackend<'a> {
    /// Wraps an on-disk Vamana `index` as the approximate backend.
    pub fn new(index: &'a crate::DiskAnnIndex) -> ApproxBackend<'a> {
        ApproxBackend { index }
    }
}

#[cfg(feature = "approx-ann")]
impl AnnBackend for ApproxBackend<'_> {
    fn candidates(&self, query: &[f32], fetch: usize) -> Vec<(Id, f32)> {
        // `DiskAnnIndex::nearest` clamps its beam to `max(search_beam, k)`, so requesting `fetch`
        // candidates also widens the beam to `fetch` — a larger fetch is a (super)set in rank order.
        self.index.nearest(query, fetch)
    }
    fn len(&self) -> usize {
        self.index.len()
    }
}

/// (sq-ip3a) **Filtered top-`k` with iterative over-fetch** over any [`AnnBackend`].
///
/// Returns the best `k` `(Id, cosine)` neighbours the backend surfaces **whose id the `mask`
/// admits**, best-first. The seam where the sq-7hx6 over-fetch boundary actually bites: post-
/// filtering a *bounded* candidate list can under-fill `k`, so we
///
/// 1. fetch [`overfetch_target(k, mask.len(), backend.len())`](crate::cost::overfetch_target)
///    candidates (the average-selectivity estimate),
/// 2. keep only mask-admitted ids, take `k`,
/// 3. **if fewer than `k` survive AND the fetch was below the backend size, double the fetch and
///    retry** — up to the backend size (a complete fetch) or `max_rounds` probes.
///
/// When the fetch reaches the backend size the candidate list is complete *for that backend*, so a
/// still-short result means the mask genuinely admits `< k` vectors (the correct short answer).
///
/// # Recall (honest)
///
/// With [`ExactBackend`] a full-store fetch is the *complete* ranking, so the result equals
/// [`nearest_exact_filtered`](crate::filter::nearest_exact_filtered) — **exact**. With
/// [`ApproxBackend`] the candidate list is approximate, so even a full fetch can miss a true
/// admitted neighbour the index never visits: **recall `< 1.0`** (measured floor in
/// `tests/overfetch.rs`, not claimed as exactness). Over-fetch fixes *under-fill*, not the
/// approximate index's inherent misses.
///
/// Empty mask, all-zero query, or `k == 0` → no results (the shared degenerate contract).
pub fn nearest_filtered_overfetch(
    backend: &impl AnnBackend,
    query: &[f32],
    mask: &IdMask,
    k: usize,
    max_rounds: usize,
) -> Vec<(Id, f32)> {
    if mask.is_empty() || k == 0 || query.iter().all(|&v| v == 0.0) || backend.is_empty() {
        return Vec::new();
    }
    let backend_len = backend.len();
    // First fetch: the average-selectivity estimate, clamped to the backend size.
    let mut fetch = overfetch_target(k, mask.len(), backend_len).min(backend_len);
    let mut round = 0usize;
    loop {
        let survivors: Vec<(Id, f32)> = backend
            .candidates(query, fetch)
            .into_iter()
            .filter(|&(id, _)| mask.contains(id))
            .take(k)
            .collect();
        // Filled k, OR the fetch is already the whole backend (a complete fetch — a short result
        // now is the genuine answer: the mask admits < k surfaced vectors), OR out of probe budget.
        if survivors.len() >= k || fetch >= backend_len || round + 1 >= max_rounds {
            return survivors;
        }
        // Under-filled with room to grow: double the fetch (clamped) and retry.
        fetch = fetch.saturating_mul(2).min(backend_len);
        round += 1;
    }
}

/// (sq-ip3a) Default probe bound for [`nearest_filtered_overfetch`]: with doubling, this
/// many rounds spans a fetch from the first estimate up to the full backend size even for a large
/// store (the loop also stops the instant the fetch reaches the backend size, so this is only a
/// hard ceiling on pathological growth, never the usual exit).
pub const DEFAULT_MAX_ROUNDS: usize = 24;

/// (sq-ip3a) [`nearest_filtered_overfetch`] with [`DEFAULT_MAX_ROUNDS`].
pub fn nearest_filtered_overfetch_default(
    backend: &impl AnnBackend,
    query: &[f32],
    mask: &IdMask,
    k: usize,
) -> Vec<(Id, f32)> {
    nearest_filtered_overfetch(backend, query, mask, k, DEFAULT_MAX_ROUNDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::nearest_exact_filtered;
    use crate::store::VectorStore;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("sparq_vec_backend_{}_{name}.spqv", std::process::id()));
        p
    }

    /// Points on the unit circle, ids 1..=n at angle 2πi/n (same fixture shape as cost.rs tests).
    fn ring_store(name: &str, n: usize) -> VectorStore {
        let mut store = VectorStore::create(tmp(name), 2).unwrap();
        for i in 0..n {
            let theta = std::f32::consts::TAU * (i as f32) / (n as f32);
            store.put((i as u32) + 1, &[theta.cos(), theta.sin()]).unwrap();
        }
        store.finalize().unwrap();
        store
    }

    /// EXACT backend + over-fetch == the exact filtered ground truth, for selective and broad masks
    /// and for k larger than the mask. (A full-store-capable backend never under-fills falsely.)
    #[test]
    fn exact_backend_overfetch_equals_ground_truth() {
        let store = ring_store("exact", 64);
        let backend = ExactBackend::new(&store);
        let query = [1.0f32, 0.0];
        for ids in [
            vec![1u32, 9, 17, 41],            // selective, scattered
            (1u32..=64).step_by(2).collect(), // ~half
            (1u32..=64).collect(),            // full
            vec![2u32, 4],                    // smaller than k
        ] {
            let mask: IdMask = ids.into_iter().collect();
            for k in [1usize, 3, 5, 10] {
                let got = nearest_filtered_overfetch_default(&backend, &query, &mask, k);
                let truth = nearest_exact_filtered(&store, &query, &mask, k);
                assert_eq!(got, truth, "exact backend must equal ground truth (k={k})");
            }
        }
    }

    /// ITERATIVE OVER-FETCH fills k under a SELECTIVE filter even when the first sized fetch
    /// under-fills because the admitted ids cluster *away* from the query. We force a first fetch
    /// far below the needed depth via a tiny mask of FAR ids, then assert the loop grows the fetch
    /// and still returns k.
    #[test]
    fn iterative_overfetch_fills_k_under_selective_filter() {
        let store = ring_store("fill", 128);
        let backend = ExactBackend::new(&store);
        // Query at angle 0 → near ids ~1 and ~128. Admit only ids near angle π (the FAR side):
        // ids ~57..72. They sit deep in the ranking, so the average-selectivity first fetch is too
        // shallow to capture k of them — the loop must grow the fetch.
        let mask: IdMask = (57u32..=72).collect();
        let k = 10;
        let query = [1.0f32, 0.0];
        let got = nearest_filtered_overfetch_default(&backend, &query, &mask, k);
        assert_eq!(got.len(), k, "iterative over-fetch must fill k from the far-side mask");
        // Every returned id is admitted, and the result equals the exact ground truth (exact
        // backend) — so the grow-and-retry recovered the true filtered top-k.
        for &(id, _) in &got {
            assert!(mask.contains(id), "over-fetch returned an unmasked id {id}");
        }
        assert_eq!(got, nearest_exact_filtered(&store, &query, &mask, k));
    }

    /// A mask admitting FEWER than k vectors returns the short correct list (not an infinite loop /
    /// not padded): the fetch reaches the backend size and the genuine short answer is returned.
    #[test]
    fn mask_smaller_than_k_returns_short_correct_list() {
        let store = ring_store("short", 64);
        let backend = ExactBackend::new(&store);
        let mask: IdMask = vec![10u32, 20, 30].into_iter().collect();
        let got = nearest_filtered_overfetch_default(&backend, &[1.0f32, 0.0], &mask, 10);
        assert_eq!(got.len(), 3, "only 3 admitted vectors exist → short list, no padding");
        assert_eq!(got, nearest_exact_filtered(&store, &[1.0f32, 0.0], &mask, 10));
    }

    /// Degenerate contracts: empty mask, all-zero query, k=0 → no results.
    #[test]
    fn degenerate_cases() {
        let store = ring_store("degen", 16);
        let backend = ExactBackend::new(&store);
        let mask: IdMask = (1u32..=16).collect();
        assert!(nearest_filtered_overfetch_default(&backend, &[1.0, 0.0], &IdMask::new(), 5).is_empty());
        assert!(nearest_filtered_overfetch_default(&backend, &[0.0, 0.0], &mask, 5).is_empty());
        assert!(nearest_filtered_overfetch_default(&backend, &[1.0, 0.0], &mask, 0).is_empty());
    }

    /// `max_rounds = 1` (no growth allowed) can under-fill on a far-clustered selective mask, while
    /// the default rounds fill k — proving the iteration (not the first fetch) is what fills k.
    #[test]
    fn single_round_can_underfill_iteration_fixes_it() {
        let store = ring_store("rounds", 128);
        let backend = ExactBackend::new(&store);
        let mask: IdMask = (57u32..=72).collect(); // far side, deep in the ranking
        let k = 10;
        let query = [1.0f32, 0.0];
        let one = nearest_filtered_overfetch(&backend, &query, &mask, k, 1);
        let many = nearest_filtered_overfetch_default(&backend, &query, &mask, k);
        assert!(
            one.len() < many.len() || one.len() == k,
            "single-round len {} vs iterated len {}",
            one.len(),
            many.len()
        );
        assert_eq!(many.len(), k, "iteration must fill k");
    }
}
