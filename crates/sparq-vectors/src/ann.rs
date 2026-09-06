//! Nearest-neighbour search over a [`VectorStore`]: an exact brute-force baseline and
//! an HNSW index (`instant-distance`, pure Rust).
//!
//! Similarity is **cosine** throughout. The HNSW index stores L2-normalized copies of
//! the vectors and searches with Euclidean distance, which is rank-equivalent to cosine
//! on unit vectors (`cos = 1 − d²/2`); reported scores are converted back to cosine, so
//! exact and approximate results are directly comparable.
//!
//! **Persistence**: the HNSW index is rebuilt from the mmap'd store on open rather than
//! persisted. `instant-distance`'s serde persistence would add serde + bincode and a
//! second versioned artifact; at the scales this v1 targets the rebuild is a one-off
//! cost per process (tens of seconds for 50k×32 on an M1, rayon-parallel — see the README
//! throughput table). Out-of-core persistent ANN (DiskANN-style) is the recorded
//! follow-up for 10M+ stores.
//!
//! [OPUS-4.8] (sq-ip3a) **The HNSW index (`VectorIndex`) is gated behind the opt-in
//! `approx-ann` feature** — it is the only thing here that pulls the third-party
//! `instant-distance` crate, so with `approx-ann` OFF the default build carries the exact
//! brute-force searchers ([`nearest_exact`], [`nearest_term_exact`]) and NO heavy ANN
//! dependency. Approximate search is APPROXIMATE: its recall is `< 1.0` (measured against
//! [`nearest_exact`], the ground truth) — only the exact path is answer-exact.
//!
//! [SONNET-4.6] (sq-jo6ty) **Per-query `ef_search` (`nearest_with_ef`)**: `instant-distance`
//! encodes `ef_search` into `HnswMap` at build time and does not expose a per-search
//! override. `VectorIndex` therefore caches the L2-normalised `NPoint` vectors and builds
//! a secondary map on first use of each new `ef_search` value (`ef_construction` and `seed`
//! are unchanged — though see `nearest_with_ef` on why that does NOT pin the resulting graph
//! topology bit-for-bit). The secondary map is stored in an
//! `RwLock<HashMap<usize, Arc<HnswMap<…>>>>` so sweeps amortise the rebuild cost: 100 queries
//! at `ef=16`, then 100 at `ef=32`, pay one extra build per ef level, not one per query.
//! `nearest_with_ef(q, k, ef)` where `ef == build_ef_search` is free (uses the primary map).
//!
//! [SONNET-4.6] (sq-ey95c) The secondary path holds NO lock while it searches: a cache hit
//! takes a *read* guard, clones the `Arc<HnswMap<…>>` handle out, drops the guard, and searches
//! the map unlocked — so any number of threads can query the same (or different) ef level in
//! parallel. The lazy build likewise runs outside the lock; the write guard is taken only for
//! the `HashMap` insert.

use crate::store::VectorStore;
#[cfg(feature = "approx-ann")]
use instant_distance::{Builder, HnswMap, Point, Search};
#[cfg(feature = "approx-ann")]
use std::collections::HashMap;
#[cfg(feature = "approx-ann")]
use std::sync::{Arc, RwLock};
use oxrdf::Term;
use sparq_core::dict::Id;
use sparq_core::Graph;

/// Cosine similarity of two equal-length vectors, in `[-1, 1]` (0 if either is zero).
/// Eight independent accumulator lanes so the compiler auto-vectorizes the loop.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "cosine over mismatched dims");
    const LANES: usize = 8;
    let mut dot = [0f32; LANES];
    let mut na = [0f32; LANES];
    let mut nb = [0f32; LANES];
    let chunks = a.len() / LANES;
    for c in 0..chunks {
        for l in 0..LANES {
            let (x, y) = (a[c * LANES + l], b[c * LANES + l]);
            dot[l] += x * y;
            na[l] += x * x;
            nb[l] += y * y;
        }
    }
    for i in chunks * LANES..a.len() {
        dot[0] += a[i] * b[i];
        na[0] += a[i] * a[i];
        nb[0] += b[i] * b[i];
    }
    let (dot, na, nb) =
        (dot.iter().sum::<f32>(), na.iter().sum::<f32>(), nb.iter().sum::<f32>());
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// **Exact** top-`k` by cosine similarity: a full scan of the store. The ground-truth
/// baseline the HNSW recall gate measures against, and a fine default below ~10⁵
/// vectors. Ties break on ascending id, so results are deterministic. An all-zero
/// `query` has no direction (cosine is undefined), so it returns no results — stored
/// vectors are never zero ([`VectorStore::put`] rejects them), and `VectorIndex`
/// treats a zero query the same way, so exact and HNSW agree on the degenerate case.
pub fn nearest_exact(store: &VectorStore, query: &[f32], k: usize) -> Vec<(Id, f32)> {
    if query.iter().all(|&v| v == 0.0) {
        return Vec::new();
    }
    let mut scored: Vec<(Id, f32)> =
        store.iter().map(|(id, v)| (id, cosine(query, v))).collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}

/// Exact cosine top-`k` with each hit's opaque metadata tag attached.
///
/// Ranking is delegated to [`nearest_exact`] and is therefore identical, including its ascending-id
/// tie-break. Metadata only decorates the completed ranking and never filters or reorders it. IDs
/// written with [`VectorStore::put`] return `None`; IDs written with
/// [`VectorStore::put_with_meta`] return the tag byte-for-byte as a `String`.
#[cfg(feature = "metadata-sidecar")]
pub fn nearest_exact_with_meta(
    store: &VectorStore,
    query: &[f32],
    k: usize,
) -> Vec<(Id, f32, Option<String>)> {
    nearest_exact(store, query, k)
        .into_iter()
        .map(|(id, score)| (id, score, store.meta(id).map(str::to_owned)))
        .collect()
}

/// Like [`nearest_exact`] but query-by-term: resolves `term` through the graph's
/// dictionary, looks its vector up in the store, excludes the term itself from the
/// results and maps neighbor ids back to [`Term`]s. Empty if the term is absent from
/// the dictionary or has no vector.
///
/// [OPUS-4.8] (sq-32i5) This does NOT verify `store` matches `graph` — a store keyed by a
/// stale graph generation silently mis-resolves `term`. Use
/// [`nearest_term_exact_checked`] (or call [`VectorStore::check_graph`] once) to make a
/// mismatch a hard error.
pub fn nearest_term_exact(
    store: &VectorStore,
    graph: &Graph,
    term: &Term,
    k: usize,
) -> Vec<(Term, f32)> {
    let Some(id) = graph.id_of(term) else { return Vec::new() };
    let Some(query) = store.get(id) else { return Vec::new() };
    nearest_exact(store, query, k + 1)
        .into_iter()
        .filter(|&(n, _)| n != id)
        .take(k)
        .map(|(n, s)| (graph.dict.term(n), s))
        .collect()
}

/// [SONNET-4.6] (sq-tb9p0) **Answer-exact top-`k` with the deterministic boundary tie-break**
/// (spec assertion VG-TIE-1 of `site/specs/sparql-vector-genai.typ`): when candidates tie on
/// score at the k boundary (the k-th best and the next-best scores are equal), result-set
/// *membership* is decided by the candidates' **canonical N-Triples serialisation**, admitted in
/// ascending Unicode-codepoint order, until `k` results are reached. The rule is defined over
/// terms whose key is **fixed by the RDF term alone**: the spec's mainline embeddable domain —
/// IRIs (`<iri>`) and literals (the quoted, escaped lexical form plus the
/// `^^<datatype>`/`@lang` suffix) — plus the ground triple terms the RDF 1.2 estate embeds
/// (their `<<( … )>>` form is likewise pinned by the grammar). For those terms two answer-exact
/// implementations return the SAME top-k set on the same store — unlike [`nearest_exact`]'s
/// ascending-id tie-break, which is deterministic within one build but not reproducible across
/// dictionaries. UTF-8 byte order IS codepoint order, so the comparison is a plain byte compare
/// of the serialised terms.
///
/// **Blank-node candidates are rejected, not ranked (`Err`)**: a blank-node label is
/// document-local per N-Triples — two equivalent stores can carry different labels for
/// corresponding nodes — so a term containing a blank node (a blank node itself, or a non-ground
/// triple term) has NO cross-implementation tie-break key, and admitting one would silently void
/// the reproducibility claim above. The guard is fail-closed
/// and applies exactly where a candidate's *term* (rather than its score alone) is load-bearing:
/// every admitted candidate and every member of the boundary tie group. A candidate strictly
/// below the boundary (and outside the tie group) is rejected by score comparison alone, so its
/// term is never materialised — the guard costs O(k + tie group), never a store scan.
///
/// The N-Triples keys are computed only for the boundary tie group (candidates whose score
/// equals the k-th best), never for the whole store. Candidates strictly above the boundary are
/// admitted unconditionally (after the domain check); ties strictly inside the top-k need no
/// rule (the solution multiset is unordered). `exclude` drops one id from the candidate pool
/// before ranking — the seed-self-exclusion of a query-by-node search (`None` for a
/// query-by-vector search). `k = 0` and an all-zero `query` yield no results, exactly as
/// [`nearest_exact`].
///
/// Like [`nearest_term_exact`], this does NOT verify `store` matches `graph`; run
/// [`VectorStore::check_graph`] first when the store carries a fingerprint.
pub fn nearest_exact_tiebreak(
    store: &VectorStore,
    graph: &Graph,
    query: &[f32],
    k: usize,
    exclude: Option<Id>,
) -> Result<Vec<(Id, f32)>, String> {
    if k == 0 || query.iter().all(|&v| v == 0.0) {
        return Ok(Vec::new());
    }
    let mut scored: Vec<(Id, f32)> = store
        .iter()
        .filter(|&(id, _)| Some(id) != exclude)
        .map(|(id, v)| (id, cosine(query, v)))
        .collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    select_top_k_tiebreak(graph, scored, k)
}

/// [SONNET-4.6] The VG-TIE-1 embeddable-domain guard: resolves `id` and returns its [`Term`] iff
/// its N-Triples tie-break key is fixed by the term alone — an IRI, a literal (the spec's
/// mainline embeddable domain), or a **ground** triple term (the RDF 1.2 estate sparq
/// additionally embeds; its `<<( … )>>` serialisation is likewise pinned by the grammar) — else
/// the fail-closed error [`nearest_exact_tiebreak`] documents. A blank node's N-Triples label is
/// scoped to a single document, so any term containing one (at any nesting depth) has no
/// canonical cross-implementation tie-break key.
fn tiebreak_domain_term(graph: &Graph, id: Id) -> Result<Term, String> {
    fn is_ground(term: &Term) -> bool {
        match term {
            Term::NamedNode(_) | Term::Literal(_) => true,
            Term::BlankNode(_) => false,
            Term::Triple(t) => {
                !matches!(t.subject, oxrdf::NamedOrBlankNode::BlankNode(_))
                    && is_ground(&t.object)
            }
        }
    }
    let term = graph.dict.term(id);
    if is_ground(&term) {
        Ok(term)
    } else {
        Err(format!(
            "vec: the answer-exact boundary tie-break (VG-TIE-1) is defined only over terms \
             whose N-Triples key is fixed by the term alone (IRIs, literals, and ground triple \
             terms); candidate {} contains a blank node, whose document-local label has no \
             tie-break key that is stable across implementations; refusing to rank it",
            term
        ))
    }
}

/// [SONNET-4.6] The VG-TIE-1 boundary-membership rule applied to an already
/// score-descending-sorted **complete** candidate ranking: candidates strictly above the k
/// boundary are admitted unconditionally; a score tie straddling the boundary is admitted in
/// ascending N-Triples codepoint order until `k` results are reached. Shared by
/// [`nearest_exact_tiebreak`] (the whole store as the pool) and the filtered
/// `nearest_filtered_costed_tiebreak` (`filtered-ann` only; the mask-admitted pool) so both
/// apply the identical rule — the ranking must already have any seed exclusion applied, so
/// the boundary is computed over the true candidate pool.
///
/// Enforces the key-stability domain fail-closed (see [`nearest_exact_tiebreak`]): every
/// admitted candidate and every boundary-tie-group member must be an IRI, a literal, or a
/// ground triple term, else `Err` — a term containing a blank node has no cross-implementation
/// tie-break key, so ranking one would silently break the reproducibility contract. Candidates
/// whose membership is decided by score alone (strictly below the boundary and outside the tie
/// group) are not materialised or checked.
pub(crate) fn select_top_k_tiebreak(
    graph: &Graph,
    mut scored: Vec<(Id, f32)>,
    k: usize,
) -> Result<Vec<(Id, f32)>, String> {
    use std::cmp::Ordering;
    if k == 0 {
        return Ok(Vec::new());
    }
    if scored.len() <= k {
        for &(id, _) in &scored {
            tiebreak_domain_term(graph, id)?;
        }
        return Ok(scored);
    }
    let boundary = scored[k - 1].1;
    if scored[k].1.total_cmp(&boundary) != Ordering::Equal {
        // No tie straddles the k boundary: membership is already determined by score — but
        // every admitted candidate still surfaces in the answer, so the domain guard applies.
        scored.truncate(k);
        for &(id, _) in &scored {
            tiebreak_domain_term(graph, id)?;
        }
        return Ok(scored);
    }
    // The tie group is the contiguous run of boundary-scored candidates [lo, hi) in the
    // score-descending order; `partition_point`'s predicates are monotone over that order.
    let lo = scored.partition_point(|&(_, s)| s.total_cmp(&boundary) == Ordering::Greater);
    let hi = scored.partition_point(|&(_, s)| s.total_cmp(&boundary) != Ordering::Less);
    for &(id, _) in &scored[..lo] {
        tiebreak_domain_term(graph, id)?;
    }
    let mut tied = Vec::with_capacity(hi - lo);
    for &(id, s) in &scored[lo..hi] {
        // Every tie-group member's KEY enters the ordering (it competes for admission even if
        // it loses), so the whole group — not just the winners — must be in-domain.
        tied.push((id, s, tiebreak_domain_term(graph, id)?.to_string()));
    }
    tied.sort_unstable_by(|a, b| a.2.cmp(&b.2));
    let mut out = scored[..lo].to_vec();
    out.extend(tied.into_iter().take(k - lo).map(|(id, s, _)| (id, s)));
    Ok(out)
}

/// [OPUS-4.8] (sq-32i5) [`nearest_term_exact`] with the staleness check: returns `Err` if
/// `store` was built against a different graph generation than `graph` (which would otherwise
/// return silently-wrong neighbours), else `Ok` with the neighbours.
pub fn nearest_term_exact_checked(
    store: &VectorStore,
    graph: &Graph,
    term: &Term,
    k: usize,
) -> Result<Vec<(Term, f32)>, String> {
    store.check_graph(graph)?;
    Ok(nearest_term_exact(store, graph, term, k))
}

/// HNSW construction/search parameters (passed through to `instant-distance`).
/// [OPUS-4.8] (sq-ip3a) `approx-ann` only.
///
/// [OPUS-4.8] (sq-ose80) **`ef_construction` is the dominant BUILD-time knob.** The
/// `instant-distance` graph build is already `rayon`-parallel (per-layer
/// `into_par_iter`); its cost is dominated by the greedy distance search each insert runs,
/// and that search's beam width IS `ef_construction`. Roughly halving `ef_construction`
/// roughly halves build time. Lowering it also lowers the graph quality (fewer
/// back-links), so recall drops slightly — but on the measured corpora the drop stays well
/// inside the recall floor. Use [`fast_build`](Self::fast_build) when build latency
/// matters more than the last fraction of recall, [`high_recall`](Self::high_recall) for
/// the opposite. Both are **opt-in** presets; the [`Default`] is unchanged, so existing
/// callers keep exactly the same graph and recall.
#[cfg(feature = "approx-ann")]
#[derive(Clone, Copy, Debug)]
pub struct HnswConfig {
    /// Beam width during search (the recall knob; must be ≥ the `k` you will query).
    pub ef_search: usize,
    /// Beam width during construction — the dominant build-time knob (see the type docs).
    pub ef_construction: usize,
    /// Level-assignment RNG seed — fixed by default so builds are reproducible.
    pub seed: u64,
}

#[cfg(feature = "approx-ann")]
impl Default for HnswConfig {
    fn default() -> Self {
        HnswConfig { ef_search: 100, ef_construction: 100, seed: 0x5350_5156_0001 }
    }
}

#[cfg(feature = "approx-ann")]
impl HnswConfig {
    /// [OPUS-4.8] (sq-ose80) A **faster-build** preset: same `ef_search` and `seed` as the
    /// [`Default`], but a lower `ef_construction` (40 vs 100) so the graph build runs
    /// markedly faster at scale.
    ///
    /// **Why it exists.** The `instant-distance` build is already `rayon`-parallel, so the
    /// build-time gap vs a C++ HNSW is not a missing-parallelism bug — it is the per-insert
    /// greedy distance search, whose beam width is `ef_construction`. A narrower construction
    /// beam does less work per insert. On a 200k×128d cosine SIFT slice on the aarch64 work
    /// box (**NON-CANONICAL** timings — the ranking, not the absolute seconds, is what
    /// transfers) `ef_construction = 40` built in roughly a third of the `ef_construction =
    /// 100` time and still measured recall@10 = 0.9944 (vs 0.9990 at the default) — comfortably
    /// above the 0.95 floor the [`VectorIndex`] recall gate asserts. Prefer this when you rebuild
    /// the index often (e.g. per store generation) and a fraction of a percent of recall is an
    /// acceptable trade.
    ///
    /// This is a **pure config** preset: it adds no dependency, changes no default, and keeps
    /// the build deterministic for a fixed seed (the same seed yields the same graph). The
    /// `nearest` / `nearest_with_ef` query path and its monotone-recall contract are unchanged.
    pub fn fast_build() -> Self {
        HnswConfig { ef_construction: 40, ..HnswConfig::default() }
    }

    /// [OPUS-4.8] (sq-ose80) A **higher-recall** preset: same `ef_search` and `seed` as the
    /// [`Default`], but a wider `ef_construction` (200 vs 100) for a denser graph. This is the
    /// opposite trade to [`fast_build`](Self::fast_build): a wider construction beam links more
    /// back-neighbours, so recall rises, at a proportionally longer build. Prefer it for a
    /// build-once, query-forever index where the extra build time amortises. (Also a pure config
    /// preset — no dependency, no default change, deterministic for a fixed seed.)
    pub fn high_recall() -> Self {
        HnswConfig { ef_construction: 200, ..HnswConfig::default() }
    }
}

/// A normalized point in the HNSW graph. Euclidean distance over unit vectors is
/// rank-equivalent to cosine; see the module docs.
///
/// [FABLE-5] (sq-jk7w7) The vector data is behind an `Arc<[f32]>` so that `Clone` — which
/// `instant_distance::Point` requires, and which [`VectorIndex::build_with`] relies on to
/// retain the point set for lazy per-`ef_search` secondary maps — is a refcount bump, not a
/// deep copy. Before this, `build_with` deep-cloned the whole normalised point set into the
/// primary map (~512 MB extra peak at 1M×128d, doubling build peak memory; `instant-distance`
/// clones each point once more internally while shuffling, so the transient peak was 3×). The
/// distance kernel still sees a plain `&[f32]` (one indirection, same as `Vec`), so query and
/// build speed and determinism are unchanged.
#[cfg(feature = "approx-ann")]
#[derive(Clone)]
// [FABLE-5] (#2251) `pub(crate)`: the recall-gated concept-dedup module (`dedup.rs`, same
// `approx-ann` feature) builds its HNSW over the identical Arc-backed normalised point type so
// exact/HNSW/concept scores all stay directly comparable — not exported outside the crate.
pub(crate) struct NPoint(pub(crate) Arc<[f32]>);

#[cfg(feature = "approx-ann")]
impl Point for NPoint {
    fn distance(&self, other: &Self) -> f32 {
        // [OPUS-4.8] (sq-lfo84) The HNSW build + search call this millions of times — it is the
        // dominant cost of both. Dispatch to the explicit NEON/AVX2 kernel (scalar fallback where
        // the vector ISA is absent), which measurably cuts build time and lifts QPS vs the previous
        // scalar-only loop. `instant-distance` wants the true Euclidean distance, so we `sqrt` the
        // squared kernel (sqrt is monotone, so `sqrt` itself introduces no reorder vs the squared
        // value). The SIMD kernel uses FMA (one rounding) vs the scalar `d*d`+`+=` (two roundings),
        // so a distance differs by <=1 ULP: rankings are stable up to exact near-ties, which the
        // HNSW recall FLOOR gate (tests/recall.rs, recall@10 >= 0.95) absorbs — NOT a bit-identity
        // claim. The reported cosine score is derived from `item.distance` exactly as before.
        crate::simd::l2_sq_dist(&self.0, &other.0).sqrt()
    }
}

/// L2-normalizes `v`; `None` for an all-zero vector (no direction). Stored vectors are
/// never zero ([`VectorStore::put`] rejects them), so `None` only arises for queries.
/// [FABLE-5] (sq-jk7w7) Returns the shared-ownership `Arc<[f32]>` form [`NPoint`] wraps.
#[cfg(feature = "approx-ann")]
pub(crate) fn normalized(v: &[f32]) -> Option<Arc<[f32]>> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (norm > 0.0).then(|| v.iter().map(|x| x / norm).collect())
}

/// An in-RAM HNSW index over a [`VectorStore`], for approximate top-`k` at scales
/// where [`nearest_exact`]'s full scan is too slow. Build once per store generation
/// (rebuilt on open — see the module docs on persistence).
///
/// [OPUS-4.8] (sq-ip3a) **APPROXIMATE** — recall `< 1.0`, gated behind the opt-in
/// `approx-ann` feature (the only thing that pulls `instant-distance`). For answer-exact
/// search use [`nearest_exact`]; this trades recall for speed at scale.
///
/// [SONNET-4.6] (sq-jo6ty) Exposes `nearest_with_ef` for per-query `ef_search` sweeps
/// (e.g. to build a recall–QPS Pareto curve at multiple ef levels without rebuilding the
/// whole store). The primary map (built at `HnswConfig::ef_search`) is used for
/// `nearest` and for `nearest_with_ef` when `ef == build_ef_search`. A secondary per-ef
/// map is built lazily and cached in `ef_cache` on first use of each new ef value.
#[cfg(feature = "approx-ann")]
pub struct VectorIndex {
    /// Primary HNSW map — built with the `HnswConfig::ef_search` value.
    map: HnswMap<NPoint, Id>,
    /// The `ef_search` the primary map was built with (needed for the cache short-circuit).
    ef_search_default: usize,
    /// The `ef_construction` / `seed` used to build the primary map (reused for secondary maps
    /// so that all ef levels share the same graph topology and seed).
    ef_construction: usize,
    seed: u64,
    /// L2-normalised points and their dict ids — retained so that secondary maps for different
    /// `ef_search` values can be built from the same normalised vectors (no store re-scan).
    /// [FABLE-5] (sq-jk7w7) Each [`NPoint`] is `Arc`-backed, so this retention (and every
    /// secondary-map build) SHARES the primary map's vector allocations — the per-index cost is
    /// one pointer-sized `Vec` per map, not a second copy of the float data.
    points: Vec<NPoint>,
    values: Vec<Id>,
    /// Lazily-built secondary maps keyed by `ef_search`. Populated on the first
    /// `nearest_with_ef` call at a new ef level; thereafter the cached map is reused.
    ///
    /// [SONNET-4.6] (sq-ey95c) An `RwLock` over `Arc`-handled maps rather than a
    /// `Mutex<HashMap<_, HnswMap<…>>>`: the `Arc` lets a hit clone the handle out from under a
    /// *read* guard and search with NO lock held, so concurrent queries at the same ef level run
    /// in parallel instead of serialising on the cache. A once-built map is never mutated, so
    /// sharing it by `Arc` is sound.
    ef_cache: RwLock<HashMap<usize, Arc<HnswMap<NPoint, Id>>>>,
    dim: usize,
}

#[cfg(feature = "approx-ann")]
impl VectorIndex {
    /// Builds the index over every vector in the store with default parameters
    /// (rayon-parallel inside `instant-distance`).
    pub fn build(store: &VectorStore) -> VectorIndex {
        VectorIndex::build_with(store, HnswConfig::default())
    }

    /// Builds the index with the given [`HnswConfig`].
    ///
    /// [FABLE-5] (sq-jk7w7) Peak memory is ONE copy of the L2-normalised point set plus
    /// pointer-sized bookkeeping: the internal `NPoint` is `Arc`-backed, so the `points.clone()` handed to
    /// the primary map shares the float allocations with the retained `self.points` (used for
    /// lazy per-`ef_search` secondary maps) instead of deep-copying them. Previously that clone
    /// duplicated the whole point set (~512 MB extra at 1M×128d), doubling build peak memory.
    pub fn build_with(store: &VectorStore, cfg: HnswConfig) -> VectorIndex {
        let mut points = Vec::with_capacity(store.len());
        let mut values = Vec::with_capacity(store.len());
        for (id, v) in store.iter() {
            let n = normalized(v).expect("stores never hold zero vectors (put rejects them)");
            points.push(NPoint(n));
            values.push(id);
        }
        // [FABLE-5] (sq-jk7w7) Cheap: clones Vecs of `Arc` handles + ids, NOT the vector data.
        let map = Builder::default()
            .ef_search(cfg.ef_search)
            .ef_construction(cfg.ef_construction)
            .seed(cfg.seed)
            .build(points.clone(), values.clone());
        VectorIndex {
            map,
            ef_search_default: cfg.ef_search,
            ef_construction: cfg.ef_construction,
            seed: cfg.seed,
            points,
            values,
            ef_cache: RwLock::new(HashMap::new()),
            dim: store.dim(),
        }
    }

    /// Approximate top-`k` ids by cosine similarity to `query`, best first.
    /// Uses the build-time `ef_search`. An all-zero `query` returns no
    /// results (same contract as [`nearest_exact`]).
    pub fn nearest(&self, query: &[f32], k: usize) -> Vec<(Id, f32)> {
        assert_eq!(query.len(), self.dim, "query dim {} != store dim {}", query.len(), self.dim);
        let Some(q) = normalized(query) else { return Vec::new() };
        let q = NPoint(q);
        let mut search = Search::default();
        self.map
            .search(&q, &mut search)
            .take(k)
            .map(|item| (*item.value, 1.0 - item.distance * item.distance / 2.0))
            .collect()
    }

    /// Approximate top-`k` ids with a **per-query** `ef_search` beam width.
    ///
    /// [SONNET-4.6] (sq-jo6ty) This is the recall–QPS sweep entry point: the caller can
    /// vary `ef_search` across queries to trace the recall–throughput Pareto frontier
    /// without rebuilding the index. The `ef_search` value controls the candidate beam at
    /// query time — a larger beam visits more neighbours and improves recall at the cost of
    /// throughput; a smaller beam is faster but may miss true top-`k` candidates.
    ///
    /// **Monotone-recall property**: for any fixed query and `k`, recall@k vs the exact
    /// oracle is non-decreasing as `ef_search` increases (sweeping upward can never lower
    /// recall). This follows from the HNSW algorithm: a wider beam subsumes the candidate
    /// set explored by any narrower beam over the same graph.
    ///
    /// When `ef_search == build_ef_search` (the value passed to [`HnswConfig`] at build
    /// time), the primary map is used directly — zero extra overhead. For any other value,
    /// a secondary map is built once and cached; subsequent calls at the same ef level
    /// reuse the cached map. All secondary maps are built from the same `ef_construction`,
    /// `seed`, and normalised point set as the primary map.
    ///
    /// [SONNET-4.6] (sq-ey95c) **Not bit-reproducible across builds.** Measured on this crate's
    /// 5 000 × 32 test corpus, two `build_with` calls with an identical store and identical
    /// `HnswConfig` (`seed` included) disagree on a small fraction of queries at low `ef`:
    /// `instant-distance` builds the graph rayon-parallel, and the seed does not pin the
    /// resulting topology. Results are therefore stable *within* one `VectorIndex` (each ef
    /// level is built and cached exactly once), but two indices over the same data may differ.
    /// Compare against the same index, not a rebuilt one.
    ///
    /// [SONNET-4.6] (sq-ey95c) **Concurrency**: the secondary path holds no lock while it
    /// searches, so any number of threads may query the same ef level in parallel; only the
    /// cache lookup/insert is synchronised. The method takes `&self` and is safe to call from
    /// many threads at once.
    ///
    /// **APPROXIMATE** — recall `< 1.0` at any finite `ef_search`; use [`nearest_exact`]
    /// for answer-exact results.
    pub fn nearest_with_ef(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(Id, f32)> {
        assert_eq!(query.len(), self.dim, "query dim {} != store dim {}", query.len(), self.dim);
        let Some(q) = normalized(query) else { return Vec::new() };
        let q = NPoint(q);
        let mut search = Search::default();
        // [SONNET-4.6] (sq-jo6ty) Short-circuit: the build-time ef → primary map (zero overhead).
        if ef_search == self.ef_search_default {
            return self
                .map
                .search(&q, &mut search)
                .take(k)
                .map(|item| (*item.value, 1.0 - item.distance * item.distance / 2.0))
                .collect();
        }
        // [SONNET-4.6] (sq-ey95c) Non-default ef: resolve the secondary map to an owned `Arc`
        // handle, then search with NO lock held. Both guards below are scoped so the (expensive)
        // build and the (hot) search happen outside them — concurrent queries at the same ef
        // level share a read guard and then run fully in parallel.
        let map = self.secondary_map(ef_search);
        map.search(&q, &mut search)
            .take(k)
            .map(|item| (*item.value, 1.0 - item.distance * item.distance / 2.0))
            .collect()
    }

    /// [SONNET-4.6] (sq-ey95c) Returns the cached secondary map for a non-default `ef_search`,
    /// building it on first use. Neither the lookup nor the insert keeps a guard alive past the
    /// statement that takes it, so the caller searches the returned handle lock-free.
    ///
    /// A lost build race is possible — two threads may build the same ef level concurrently and
    /// one result is discarded. That is deliberate: building under the write guard would block
    /// every reader (including readers at *other*, already-cached ef levels) for the whole build.
    /// The race costs duplicated work, never a differing answer: `or_insert` keeps the FIRST map
    /// to land and the loser searches that same map, so for the life of the index exactly ONE
    /// map is ever observed per ef level. (This matters because `instant-distance`'s build is
    /// not bit-reproducible even at a fixed `seed` — see the caveat on [`Self::nearest_with_ef`]
    /// — so returning the loser's own map instead could change results between runs.)
    fn secondary_map(&self, ef_search: usize) -> Arc<HnswMap<NPoint, Id>> {
        // Fast path: a shared read guard, released before the (unlocked) search.
        if let Some(map) = self
            .ef_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&ef_search)
        {
            return Arc::clone(map);
        }
        // Miss: build OUTSIDE the lock so concurrent queries at other ef levels are unaffected.
        let built = Arc::new(
            Builder::default()
                .ef_search(ef_search)
                .ef_construction(self.ef_construction)
                .seed(self.seed)
                .build(self.points.clone(), self.values.clone()),
        );
        // The write guard covers the insert only. `or_insert` keeps the first map to land, so a
        // racing thread's map is dropped here rather than replacing a handle already in use.
        Arc::clone(
            self.ef_cache
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .entry(ef_search)
                .or_insert(built),
        )
    }

    /// Approximate top-`k` neighbors of `term`: resolves it through the graph's
    /// dictionary, looks its vector up in `store`, excludes the term itself and maps
    /// neighbor ids back to [`Term`]s. Empty if the term is absent or unembedded.
    pub fn nearest_term(
        &self,
        term: &Term,
        graph: &Graph,
        store: &VectorStore,
        k: usize,
    ) -> Vec<(Term, f32)> {
        let Some(id) = graph.id_of(term) else { return Vec::new() };
        let Some(query) = store.get(id) else { return Vec::new() };
        self.nearest(query, k + 1)
            .into_iter()
            .filter(|&(n, _)| n != id)
            .take(k)
            .map(|(n, s)| (graph.dict.term(n), s))
            .collect()
    }

    /// [OPUS-4.8] (sq-1wc1) **Predicate-constrained (filtered) top-`k`** over the HNSW index:
    /// returns only neighbours whose id the `mask` permits (the candidate id-set a SPARQL BGP
    /// selects). `instant-distance`'s adjacency is **not exposed**, so — unlike
    /// [`DiskAnnIndex::nearest_filtered`](crate::DiskAnnIndex::nearest_filtered), which can do
    /// predicate-agnostic graph traversal — the in-RAM HNSW cannot be walked with predicate-aware
    /// acceptance. This therefore uses the **exact pre-filter** strategy (scan only the masked ids):
    /// exact, and the right choice for a selective mask; for a broad mask over a large store prefer
    /// the on-disk index's filtered traversal. Empty mask / all-zero query → no results.
    #[cfg(feature = "filtered-ann")]
    pub fn nearest_filtered(
        &self,
        query: &[f32],
        mask: &crate::IdMask,
        store: &VectorStore,
        k: usize,
    ) -> Vec<(Id, f32)> {
        assert_eq!(query.len(), self.dim, "query dim {} != store dim {}", query.len(), self.dim);
        crate::filter::nearest_exact_filtered(store, query, mask, k)
    }

    /// [OPUS-4.8] (sq-32i5) [`nearest_term`](Self::nearest_term) with the staleness check: returns
    /// `Err` if `store` was built against a different graph generation than `graph` (which would
    /// otherwise return silently-wrong neighbours), else `Ok` with the neighbours. The HNSW index
    /// is rebuilt from `store` on open, so checking the store covers the index too.
    pub fn nearest_term_checked(
        &self,
        term: &Term,
        graph: &Graph,
        store: &VectorStore,
        k: usize,
    ) -> Result<Vec<(Term, f32)>, String> {
        store.check_graph(graph)?;
        Ok(self.nearest_term(term, graph, store, k))
    }
}

#[cfg(test)]
mod tests {
    // [OPUS-4.8] Default-feature unit gate for the `cosine` primitive that the exact searcher, the
    // filtered pre-filter and the quantizers all rank with. The integration tests use it transitively
    // for ranking but never pin its documented numeric contract directly — these do, so a regression
    // in the zero-vector guard (which would emit a NaN instead of 0.0 and silently break every
    // ranking) or in the lane-tail handling is caught.
    use super::cosine;

    #[test]
    fn cosine_known_values_unit_orthogonal_and_opposite() {
        // Identical direction → 1, orthogonal → 0, opposite → −1 (length cancels: cosine is
        // magnitude-invariant, so 3·a and a give the same score).
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[3.0, 0.0]) - 1.0).abs() < 1e-6, "cosine ignores magnitude");
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6, "orthogonal vectors are 0");
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6, "opposite vectors are -1");
        // 45° between (1,0) and (1,1): cos = 1/√2.
        assert!((cosine(&[1.0, 0.0], &[1.0, 1.0]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_is_zero_not_nan() {
        // A zero vector has no direction — the documented contract is a defined `0.0`, NEVER a NaN
        // (which `dot / (0 * …)` would otherwise produce and which would poison `total_cmp` sorting).
        let z = [0.0f32, 0.0, 0.0];
        let v = [1.0f32, 2.0, 3.0];
        assert_eq!(cosine(&z, &v), 0.0, "zero on the left is 0, not NaN");
        assert_eq!(cosine(&v, &z), 0.0, "zero on the right is 0, not NaN");
        assert_eq!(cosine(&z, &z), 0.0, "both zero is 0, not NaN");
        assert!(!cosine(&z, &v).is_nan());
    }

    #[test]
    fn cosine_handles_a_non_lane_multiple_length() {
        // dim 11 is not a multiple of the 8 SIMD lanes, so the scalar tail loop must run. A vector
        // against itself is still exactly 1.0 — proving the tail accumulators are summed in.
        let v: Vec<f32> = (1..=11).map(|i| i as f32).collect();
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5, "self-cosine over a tail length must be 1.0");
        // And a length below one full lane (dim 3) goes entirely through the tail loop.
        let w = [2.0f32, -1.0, 4.0];
        assert!((cosine(&w, &w) - 1.0).abs() < 1e-6);
    }

    #[test]
    #[should_panic(expected = "cosine over mismatched dims")]
    fn cosine_panics_on_mismatched_dims() {
        // A dim mismatch is a programming error, not a silently-truncated comparison.
        cosine(&[1.0, 0.0, 0.0], &[1.0, 0.0]);
    }

    // [SONNET-4.6] (sq-tb9p0) Direct unit gate for the VG-TIE-1 boundary tie-break: membership at
    // a score tie on the k boundary is decided by ascending N-Triples codepoint order of the
    // candidates' serialised terms, NOT by dictionary-id order (which is deterministic within one
    // build but not reproducible across implementations).
    mod tiebreak {
        use super::super::{nearest_exact_tiebreak, VectorStore};
        use oxrdf::{NamedNode, Term};
        use sparq_core::Graph;

        /// Three candidates tied at cosine 0.6 to the +x query (0.6/0.8-direction vectors, one
        /// scaled to prove magnitude-invariance) plus one exact match and one far vector. The
        /// tie group is chosen so **dictionary-id order and N-Triples key order genuinely
        /// disagree**: the dict inlines numeric literals into a high-id numeric-order range
        /// (`"2"` gets a LOWER id than `"10"`, and every IRI id is lower still), while the
        /// N-Triples codepoint order is `"10"^^…` < `"2"^^…` < `<http://ex/z-tied>`. An
        /// implementation that tie-broke by ascending id would admit `z-tied` first — the
        /// VG-TIE-1 rule admits `"10"^^xsd:integer` — so the tests discriminate the two.
        fn fixture(name: &str) -> (Graph, VectorStore) {
            let g = Graph::load_str(
                r#"
                <http://ex/top> <http://ex/num> "10"^^<http://www.w3.org/2001/XMLSchema#integer> .
                <http://ex/top> <http://ex/num> "2"^^<http://www.w3.org/2001/XMLSchema#integer> .
                <http://ex/z-tied> <http://ex/label> "z" .
                <http://ex/far> <http://ex/label> "far" .
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
            let path = std::env::temp_dir()
                .join(format!("sparq_vec_tiebreak_{}_{}.spqv", std::process::id(), name));
            let mut store = VectorStore::create(path, 2).unwrap();
            store.put(id("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0
            store.put(id("http://ex/z-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6
            store.put(num("10"), &[0.6, -0.8]).unwrap(); // cos 0.6
            store.put(num("2"), &[3.0, 4.0]).unwrap(); // cos 0.6 (scaled)
            store.put(id("http://ex/far"), &[-1.0, 0.0]).unwrap(); // cos -1.0
            (g, store)
        }

        const TEN: &str = "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>";
        const TWO: &str = "\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>";

        fn iris(g: &Graph, hits: &[(sparq_core::dict::Id, f32)]) -> Vec<String> {
            hits.iter().map(|&(id, _)| g.dict.term(id).to_string()).collect()
        }

        #[test]
        fn boundary_tie_admits_ascending_ntriples_key_order() {
            let (g, store) = fixture("boundary");
            // k=2: `top` (1.0) is strictly above; ONE slot remains for the three 0.6-tied
            // candidates → the smallest N-Triples key `"10"^^xsd:integer` wins (id order would
            // have picked `<http://ex/z-tied>` — the lowest id — instead).
            let hits = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 2, None).unwrap();
            assert_eq!(iris(&g, &hits), vec!["<http://ex/top>".to_string(), TEN.to_string()]);
            // k=3: two slots for the tie group → "10" then "2"; z-tied is still excluded.
            let hits = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 3, None).unwrap();
            assert_eq!(
                iris(&g, &hits),
                vec!["<http://ex/top>".to_string(), TEN.to_string(), TWO.to_string()]
            );
        }

        #[test]
        fn no_boundary_tie_matches_plain_top_k_and_degenerate_cases_are_empty() {
            let (g, store) = fixture("plain");
            // k=1: the 1.0 score is unique — no boundary tie, plain top-k.
            let hits = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 1, None).unwrap();
            assert_eq!(iris(&g, &hits), vec!["<http://ex/top>".to_string()]);
            // k ≥ store size returns everything (all candidates admitted, no boundary).
            assert_eq!(
                nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 99, None).unwrap().len(),
                5
            );
            // k=0 and the all-zero query yield no results (VG-DEG-2/3 alignment).
            assert!(nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 0, None).unwrap().is_empty());
            assert!(nearest_exact_tiebreak(&store, &g, &[0.0, 0.0], 3, None).unwrap().is_empty());
        }

        #[test]
        fn exclude_drops_the_seed_before_ranking() {
            let (g, store) = fixture("exclude");
            let top = g.id_of(&Term::NamedNode(NamedNode::new("http://ex/top").unwrap())).unwrap();
            // With the exact-match seed excluded, k=1 lands directly on the boundary tie group.
            let hits = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 1, Some(top)).unwrap();
            assert_eq!(iris(&g, &hits), vec![TEN.to_string()]);
            assert!(hits.iter().all(|&(id, _)| id != top), "the excluded seed must not appear");
        }

        // [SONNET-4.6] (#2445 review) The embeddable-domain guard: the VG-TIE-1 key is only
        // stable for IRIs and literals — a blank-node label is document-local, so a blank-node
        // candidate must be REJECTED (fail-closed `Err`), never ranked, wherever its term (not
        // its score alone) decides membership: in the boundary tie group, admitted strictly
        // above the boundary, or admitted because the pool is no larger than k.
        mod blank_node_domain {
            use super::super::super::{nearest_exact_tiebreak, VectorStore};
            use oxrdf::{BlankNode, NamedNode, Term};
            use sparq_core::Graph;

            /// `_:tied` shares the cosine-0.6 boundary tie with an IRI; `top` is an exact
            /// match. Blank-node labels are kept verbatim by the N-Triples loader, so the id
            /// resolves through `Term::BlankNode`.
            fn fixture(name: &str) -> (Graph, VectorStore) {
                let g = Graph::load_str(
                    r#"
                    <http://ex/top> <http://ex/p> "t" .
                    <http://ex/a-tied> <http://ex/p> "a" .
                    _:tied <http://ex/p> "b" .
                    "#,
                    "ntriples",
                )
                .unwrap();
                let iri =
                    |s: &str| g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap())).unwrap();
                let bnode =
                    g.id_of(&Term::BlankNode(BlankNode::new_unchecked("tied"))).unwrap();
                let path = std::env::temp_dir().join(format!(
                    "sparq_vec_tiebreak_bnode_{}_{}.spqv",
                    std::process::id(),
                    name
                ));
                let mut store = VectorStore::create(path, 2).unwrap();
                store.put(iri("http://ex/top"), &[1.0, 0.0]).unwrap(); // cos 1.0
                store.put(iri("http://ex/a-tied"), &[0.6, 0.8]).unwrap(); // cos 0.6
                store.put(bnode, &[0.6, -0.8]).unwrap(); // cos 0.6 — tied blank node
                (g, store)
            }

            #[test]
            fn blank_node_in_the_boundary_tie_group_is_a_fail_closed_error() {
                let (g, store) = fixture("in_tie");
                // k=2: one slot for the two-way 0.6 tie — the blank node's key would have to
                // enter the ordering, so the call must refuse, naming the term kind.
                let err = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 2, None).unwrap_err();
                assert!(err.contains("_:tied") || err.contains("blank"), "got: {}", err);
            }

            #[test]
            fn admitted_blank_node_is_a_fail_closed_error_even_without_a_tie() {
                let (g, store) = fixture("admitted");
                // k=3 admits everything (pool == k): the blank node would surface in the
                // answer, so the call must refuse rather than return a document-local label.
                let err = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 3, None).unwrap_err();
                assert!(err.contains("blank node"), "got: {}", err);
                // And with the blank node strictly above the boundary: make it the unique
                // best by querying its own direction.
                let err =
                    nearest_exact_tiebreak(&store, &g, &[0.6, -0.8], 1, None).unwrap_err();
                assert!(err.contains("blank node"), "got: {}", err);
            }

            #[test]
            fn blank_node_strictly_below_the_boundary_never_becomes_load_bearing() {
                let (g, store) = fixture("below");
                // k=1: the unique 1.0 winner is an IRI; the blank node sits strictly below the
                // boundary, is rejected by score alone, and must NOT trip the guard — the
                // documented O(k + tie group) scoping.
                let hits = nearest_exact_tiebreak(&store, &g, &[1.0, 0.0], 1, None).unwrap();
                assert_eq!(hits.len(), 1);
                assert_eq!(
                    g.dict.term(hits[0].0).to_string(),
                    "<http://ex/top>".to_string()
                );
            }
        }
    }

    // [FABLE-5] (sq-jk7w7) Peak-memory contract of `build_with`: the point set handed to the
    // HNSW map must SHARE allocations with the retained `points` (for lazy per-ef secondary
    // maps), never deep-copy them — the deep copy doubled build peak memory (~512 MB extra at
    // 1M×128d). `Arc::strong_count` observes the sharing directly, so reintroducing a deep
    // copy (fresh buffers for the map, or for a secondary map) flips these asserts red
    // (mutation-checked: a deep-copying build fails with strong_count 1 vs 2).
    #[cfg(feature = "approx-ann")]
    mod point_set_sharing {
        use super::super::{HnswConfig, VectorIndex, VectorStore};
        use std::sync::Arc;

        #[test]
        fn build_shares_point_allocations_with_the_map_instead_of_deep_copying() {
            let path = std::env::temp_dir()
                .join(format!("sparq-vectors-arcshare-{}.spqv", std::process::id()));
            let mut store = VectorStore::create(&path, 8).unwrap();
            for i in 0..64u32 {
                // Deterministic non-zero vectors; sparse ids exercise the id→slot index.
                let v: Vec<f32> = (0..8).map(|d| ((i + d + 1) as f32).sin() + 2.0).collect();
                store.put(i * 3 + 1, &v).unwrap();
            }
            store.finalize().unwrap();

            let index = VectorIndex::build_with(&store, HnswConfig::default());
            assert_eq!(index.points.len(), 64);
            for p in &index.points {
                assert_eq!(
                    Arc::strong_count(&p.0),
                    2,
                    "primary map must share each point's allocation with the retained set \
                     (strong_count 2 = retained + map); a deep copy leaves it at 1"
                );
            }

            // The lazily-built secondary map (non-default ef) must share too, not deep-copy.
            let q: Vec<f32> = (0..8).map(|d| ((d + 1) as f32).cos() + 2.0).collect();
            let default_ef = HnswConfig::default().ef_search;
            let via_default = index.nearest_with_ef(&q, 5, default_ef);
            let via_secondary = index.nearest_with_ef(&q, 5, default_ef + 7);
            for p in &index.points {
                assert_eq!(
                    Arc::strong_count(&p.0),
                    3,
                    "a secondary per-ef map must share the point allocations too"
                );
            }
            // Behavioural sanity: both ef levels answer over the same shared point set.
            assert_eq!(via_default.len(), 5);
            assert_eq!(via_secondary.len(), 5);

            let _ = std::fs::remove_file(&path);
        }
    }
}
