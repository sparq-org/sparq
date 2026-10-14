//! (sq-1wc1) **Predicate-constrained (filtered) ANN** — the RDF-native vector
//! differentiator.
//!
//! A SPARQL basic graph pattern constrains *which* graph nodes are eligible neighbours
//! (e.g. `?node a :Car ; :seats ?s` carves out a candidate id-set), and the nearest-neighbour
//! search must return only neighbours **within that set**. The caller (engine / `vec:` rewrite)
//! computes the exact set of permitted dictionary ids from the BGP through the permutation
//! indexes; this module's job is to *accept that set as a mask and honour it* during search.
//!
//! # The mask
//!
//! [`IdMask`] is a set of permitted dictionary [`Id`]s — the "visit mask" of allowed neighbours.
//! It is deliberately a plain id set (backed by [`rustc_hash::FxHashSet`], already a crate dep),
//! so building one is just `BGP-result-ids.collect()`; no new dependency enters the tree and the
//! feature stays lean. Membership is the only operation search needs.
//!
//! # Two strategies (selectivity decides)
//!
//! Both strategies return **exactly** the same neighbours an exact full scan restricted to the
//! mask would — but cost differently as the mask's selectivity changes:
//!
//! - **Pre-filter** ([`nearest_exact_filtered`]): scan only the masked ids and rank them by
//!   cosine. Cost is O(|mask|·dim). When the mask is **very selective** (few eligible nodes) this
//!   is both *exact* and *cheapest* — touching the whole ANN graph to find a handful of accepted
//!   nodes would be wasteful, and a filtered graph traversal can even fail to reach an isolated
//!   accepted node through unaccepted hops.
//!
//! - **Filtered traversal** ([`DiskAnnIndex::nearest_filtered`], ACORN / NaviX-style): walk the
//!   Vamana proximity graph **predicate-agnostically** — expand through *every* neighbour for
//!   connectivity — but **only accept** (collect into the result) nodes that are in the mask
//!   (*predicate-aware acceptance*). This keeps the graph's short search paths while filtering the
//!   output, and is the right choice when the mask is broad (a large fraction of the store passes),
//!   where pre-filtering would scan almost everything.
//!
//! The crossover is governed by a **selectivity threshold** (see [`FilterConfig`]): below it the
//! mask is "selective" and we pre-filter; above it we traverse. The default
//! ([`FilterConfig::default`]) pre-filters when the mask admits **≤ 1%** of the store *or* ≤ 256
//! nodes (whichever is larger), an honestly conservative line — pre-filter is exact, so erring
//! toward it never hurts recall, only (mildly) throughput on a mid-selective mask. Callers that
//! know their workload can move the line with [`FilterConfig`].
//!
//! Filtered traversal over an approximate graph is itself approximate: a target accepted node can
//! sit behind a region the beam prunes. [`DiskAnnIndex::nearest_filtered`] therefore widens its
//! own search beam (it must visit more nodes to *accept* `k`), and the
//! [`tests`](../tests/filtered.rs) measure recall against the exact-filtered ground truth.
//!
//! # Why the on-disk Vamana index and not the HNSW one
//!
//! Filtered traversal needs the graph's **adjacency** to walk it. [`DiskAnnIndex`] owns its
//! on-disk Vamana adjacency, so filtered traversal lives there. [`crate::ann::VectorIndex`] wraps
//! `instant-distance`, whose adjacency is **not exposed**, so it cannot do predicate-agnostic
//! traversal — for it (and as a universal fallback) [`nearest_exact_filtered`] is the filtered
//! path. This mirrors the crate's existing split: `VectorIndex` is the in-RAM convenience, the
//! on-disk graph is where the index internals are ours to extend.

#[cfg(doc)]
use crate::diskann::DiskAnnIndex;
use crate::store::VectorStore;
use rustc_hash::FxHashSet;
use sparq_core::dict::Id;

/// A set of permitted dictionary [`Id`]s — the **visit mask** a filtered ANN search restricts its
/// accepted neighbours to. Built from the id-set a SPARQL BGP selects (e.g. all `?node` with
/// `?node a :Car`); see the [module docs](self).
///
/// Backed by an [`FxHashSet`] so membership is O(1) and construction is a `collect`. An **empty**
/// mask admits nothing — every filtered search returns no results (the honest answer when the BGP
/// matched no nodes), distinct from "no mask" (an unfiltered search, which is the plain
/// [`nearest`](DiskAnnIndex::nearest) / [`nearest_exact`](crate::ann::nearest_exact) path).
#[derive(Clone, Debug, Default)]
pub struct IdMask {
    ids: FxHashSet<Id>,
}

impl IdMask {
    /// An empty mask (admits no id). A filtered search against it returns no results.
    pub fn new() -> IdMask {
        IdMask {
            ids: FxHashSet::default(),
        }
    }

    /// Builds a mask from the permitted ids (duplicates collapse). This is the BGP → mask seam:
    /// `IdMask::from_ids(bgp_solution_ids)`.
    pub fn from_ids<I: IntoIterator<Item = Id>>(ids: I) -> IdMask {
        IdMask {
            ids: ids.into_iter().collect(),
        }
    }

    /// Adds a permitted id. Returns `self` for chaining.
    pub fn insert(&mut self, id: Id) -> &mut IdMask {
        self.ids.insert(id);
        self
    }

    /// Is `id` permitted?
    pub fn contains(&self, id: Id) -> bool {
        self.ids.contains(&id)
    }

    /// Number of permitted ids.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Does the mask admit nothing?
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The permitted ids (unordered).
    pub fn iter(&self) -> impl Iterator<Item = Id> + '_ {
        self.ids.iter().copied()
    }
}

impl FromIterator<Id> for IdMask {
    fn from_iter<I: IntoIterator<Item = Id>>(iter: I) -> IdMask {
        IdMask::from_ids(iter)
    }
}

/// Tuning for the [filtered ANN](self) strategy choice and beam.
///
/// The two knobs pick the pre-filter ↔ filtered-traversal crossover; the third widens the beam so
/// filtered traversal can still *accept* `k` despite skipping unmasked nodes.
#[derive(Clone, Copy, Debug)]
pub struct FilterConfig {
    /// Pre-filter (exact scan over the mask) when `mask.len()` is at most this **fraction** of the
    /// store. Below the crossover the mask is "selective" and scanning just the masked ids beats
    /// (and is exact vs.) traversing the whole graph. `0.0` forces traversal except for the
    /// absolute floor; `1.0` always pre-filters.
    pub prefilter_fraction: f32,
    /// …or when `mask.len()` is at most this **absolute** count — a floor so a tiny mask over a
    /// huge store always pre-filters even if the fraction would not trip.
    pub prefilter_floor: usize,
    /// Filtered traversal multiplies the search beam by this factor (≥ 1): because traversal only
    /// *accepts* masked nodes, it must visit proportionally more nodes to collect `k`. Higher =
    /// better filtered recall, more work.
    pub traversal_beam_factor: usize,
}

impl Default for FilterConfig {
    fn default() -> Self {
        // Conservative line: pre-filter (exact) up to 1% of the store or 256 nodes, whichever is
        // larger. Pre-filter never costs recall, only mild throughput on a mid-selective mask, so
        // erring toward it is the honest default. Traversal beam ×4 so accepting k stays robust
        // under a moderately selective mask. non-canonical: see tests/filtered.rs for
        // the measured recall these defaults clear.
        FilterConfig {
            prefilter_fraction: 0.01,
            prefilter_floor: 256,
            traversal_beam_factor: 4,
        }
    }
}

impl FilterConfig {
    /// Should a mask of `mask_len` permitted ids over a `store_len`-vector store be served by the
    /// **pre-filter** (exact scan) strategy rather than filtered traversal?
    pub fn prefer_prefilter(&self, mask_len: usize, store_len: usize) -> bool {
        let frac_cut = (store_len as f32 * self.prefilter_fraction) as usize;
        mask_len <= frac_cut.max(self.prefilter_floor)
    }
}

/// **Exact filtered top-`k`** — the ground-truth filtered search and the *pre-filter* strategy:
/// rank only the masked ids by cosine and return the best `k`. The result is exactly what an
/// unfiltered exact scan would yield after discarding every non-masked id, so it is the recall
/// reference the approximate filtered traversal is measured against.
///
/// An empty mask, or a query with no direction (all-zero), returns no results — the same
/// degenerate-case contract as [`nearest_exact`](crate::ann::nearest_exact). Ids absent from the
/// store are silently skipped (a BGP can select an unembedded node).
pub fn nearest_exact_filtered(
    store: &VectorStore,
    query: &[f32],
    mask: &IdMask,
    k: usize,
) -> Vec<(Id, f32)> {
    if mask.is_empty() || query.iter().all(|&v| v == 0.0) {
        return Vec::new();
    }
    let mut scored: Vec<(Id, f32)> = mask
        .iter()
        .filter_map(|id| store.get(id).map(|v| (id, crate::ann::cosine(query, v))))
        .collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}
