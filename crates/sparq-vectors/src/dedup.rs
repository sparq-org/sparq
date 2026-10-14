//! (#2251) **Recall-gated ANN over raw concept vectors**: `build_ann` / `knn` /
//! `dedup` — an HNSW-class index over caller-supplied `(id, vector)` pairs (no
//! [`VectorStore`](crate::VectorStore) required), plus **recall-gated type-level
//! deduplication**: near-duplicate merges are computed from approximate nearest-neighbour
//! results, but are **applied only after** the index's measured recall against a
//! caller-supplied **exact O(m²) ground truth** clears a pre-registered gate (default
//! `0.99`). Below the gate, [`dedup`] returns `Err` and **no merge is emitted** — the gate
//! fails closed.
//!
//! **Why this exists.** Exact all-pairs dedup is O(m²) and dies past a few hundred
//! thousand vectors; ANN scales but is approximate (recall `< 1.0`). The contract here is
//! the honest middle: the consumer builds one exact ground truth at a rung where O(m²) is
//! still tractable (e.g. 100k), and every ANN-driven merge pass first proves — on that
//! ground truth — that the index recalls at least the gated fraction of true neighbours.
//! The gate is **evidence on the measured dataset**, not a universal recall guarantee: a
//! different corpus, dimension, or policy needs its own ground truth. The first consumer
//! is the Kernel-of-Truth scale track (cross-source concept dedup at the 100k → millions
//! rungs; sparq issue #2251).
//!
//! Similarity is **cosine** throughout, exactly as the rest of this crate: vectors are
//! L2-normalised into the graph and searched with Euclidean distance (rank-equivalent on
//! unit vectors, `cos = 1 − d²/2`), so scores are directly comparable to
//! [`nearest_exact`](crate::nearest_exact) / [`VectorIndex`](crate::VectorIndex).
//!
//! `approx-ann` only — this module rides the same opt-in `instant-distance` HNSW backend
//! as [`VectorIndex`](crate::VectorIndex), so the default build carries none of it.

use crate::ann::{normalized, HnswConfig, NPoint};
use instant_distance::{Builder, HnswMap, Search};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;

/// An HNSW index over raw `(id, vector)` concept pairs, built by [`build_ann`].
///
/// Unlike [`VectorIndex`](crate::VectorIndex) (which indexes a finalized
/// [`VectorStore`](crate::VectorStore) keyed by dictionary term ids), this index is built
/// straight from an in-RAM matrix — the shape the concept-vector consumers have — with
/// `Id` as an opaque `u32` key. **APPROXIMATE**: recall `< 1.0`; the recall evidence path
/// is [`dedup`]'s gate against an exact ground truth.
// Debug is hand-written (below): instant-distance's HnswMap has no Debug impl to derive over.
pub struct ConceptAnnIndex {
    map: HnswMap<NPoint, Id>,
    /// L2-normalised points, index-aligned with `ids` (`Arc`-backed — shared with `map`).
    points: Vec<NPoint>,
    ids: Vec<Id>,
    /// id → position in `points`/`ids`, for query-by-id (recall measurement, dedup).
    pos: FxHashMap<Id, usize>,
    policy: HnswConfig,
    dim: usize,
}

impl std::fmt::Debug for ConceptAnnIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConceptAnnIndex")
            .field("len", &self.ids.len())
            .field("dim", &self.dim)
            .field("policy", &self.policy)
            .finish_non_exhaustive() // the HNSW graph itself has no Debug
    }
}

/// Builds a [`ConceptAnnIndex`] over `vectors` under `policy` (the HNSW build policy —
/// [`HnswConfig`]; [`Default`] matches [`VectorIndex`](crate::VectorIndex)).
///
/// Fail-closed input validation, mirroring [`VectorStore::put`](crate::VectorStore::put):
/// an empty input, a zero-length or inconsistent dimension, a non-finite component, an
/// all-zero vector (no direction under cosine) or a duplicate id is an `Err`, never a
/// silently-degraded index. Deterministic for a fixed `policy.seed`.
pub fn build_ann(
    vectors: &[(Id, Vec<f32>)],
    policy: HnswConfig,
) -> Result<ConceptAnnIndex, String> {
    let dim = validate(vectors).map_err(|e| format!("build_ann: {e}"))?;
    let mut points = Vec::with_capacity(vectors.len());
    let mut ids = Vec::with_capacity(vectors.len());
    let mut pos = FxHashMap::default();
    for (id, v) in vectors {
        // Validated above: never zero, so `normalized` cannot return None.
        let n = normalized(v).expect("validate rejects all-zero vectors");
        pos.insert(*id, points.len());
        points.push(NPoint(n));
        ids.push(*id);
    }
    // Cheap clones: NPoint is Arc-backed, so this shares the float data with `points`.
    let map = Builder::default()
        .ef_search(policy.ef_search)
        .ef_construction(policy.ef_construction)
        .seed(policy.seed)
        .build(points.clone(), ids.clone());
    Ok(ConceptAnnIndex {
        map,
        points,
        ids,
        pos,
        policy,
        dim,
    })
}

/// The shared fail-closed input validation ([`build_ann`], [`exact_ground_truth`]):
/// `Err` on an empty input, a zero/inconsistent dimension, a non-finite component, an
/// all-zero vector, or a duplicate id. Returns the common dimension.
fn validate(vectors: &[(Id, Vec<f32>)]) -> Result<usize, String> {
    let Some((_, first)) = vectors.first() else {
        return Err("empty input (no vectors)".into());
    };
    let dim = first.len();
    if dim == 0 {
        return Err("zero-dimensional vectors".into());
    }
    let mut seen = FxHashSet::default();
    for (id, v) in vectors {
        if v.len() != dim {
            return Err(format!("vector for id {id} has dim {} != {dim}", v.len()));
        }
        if v.iter().any(|x| !x.is_finite()) {
            return Err(format!("vector for id {id} has a non-finite component"));
        }
        if v.iter().all(|&x| x == 0.0) {
            return Err(format!(
                "vector for id {id} is all-zero (no direction under cosine)"
            ));
        }
        if !seen.insert(*id) {
            return Err(format!("duplicate id {id}"));
        }
    }
    Ok(dim)
}

/// Approximate top-`k` ids by cosine similarity to `query`, best first (free-function
/// form of [`ConceptAnnIndex::knn`], the issue-#2251 surface).
///
/// **APPROXIMATE** — recall `< 1.0`. `k` is clamped by the build policy's `ef_search`.
/// An all-zero `query` returns no results (same contract as
/// [`nearest_exact`](crate::nearest_exact)); a dimension mismatch panics (a programming
/// error, consistent with [`VectorIndex::nearest`](crate::VectorIndex::nearest)).
pub fn knn(index: &ConceptAnnIndex, query: &[f32], k: usize) -> Vec<(Id, f32)> {
    index.knn(query, k)
}

impl ConceptAnnIndex {
    /// Approximate top-`k` ids by cosine similarity to `query`, best first. See [`knn`].
    pub fn knn(&self, query: &[f32], k: usize) -> Vec<(Id, f32)> {
        assert_eq!(
            query.len(),
            self.dim,
            "query dim {} != index dim {}",
            query.len(),
            self.dim
        );
        let Some(q) = normalized(query) else {
            return Vec::new();
        };
        self.search_point(&NPoint(q), k)
    }

    /// Approximate top-`k` neighbours of the **indexed** vector `id` (the id itself
    /// excluded). Empty if `id` is not in the index.
    pub fn knn_of(&self, id: Id, k: usize) -> Vec<(Id, f32)> {
        let Some(&p) = self.pos.get(&id) else {
            return Vec::new();
        };
        let mut hits = self.search_point(&self.points[p].clone(), k + 1);
        hits.retain(|&(n, _)| n != id);
        hits.truncate(k);
        hits
    }

    /// Number of indexed vectors.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// `true` when the index holds no vectors ([`build_ann`] rejects an empty input, so
    /// this is `false` for any successfully-built index).
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The vector dimension.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The ids in the index, in insertion order.
    pub fn ids(&self) -> &[Id] {
        &self.ids
    }

    fn search_point(&self, q: &NPoint, k: usize) -> Vec<(Id, f32)> {
        let mut search = Search::default();
        self.map
            .search(q, &mut search)
            .take(k)
            .map(|item| (*item.value, 1.0 - item.distance * item.distance / 2.0))
            .collect()
    }
}

/// An **exact** k-nearest-neighbour ground truth: for each query id, its true top-`k`
/// neighbour ids by cosine (self excluded), computed by a full O(m²) scan — the oracle
/// [`dedup`] measures ANN recall against.
///
/// Build it in-process with [`exact_ground_truth`] (fine up to ~10⁵ vectors), or
/// construct one from an externally-computed oracle (e.g. a one-off O(m²) pass at the
/// 100k rung) with [`GroundTruth::new`]. Neighbour lists **must not contain the query id
/// itself** and may be shorter than `k` (when fewer than `k` other vectors exist).
#[derive(Clone, Debug)]
pub struct GroundTruth {
    k: usize,
    neighbors: Vec<(Id, Vec<Id>)>,
}

impl GroundTruth {
    /// Wraps an externally-computed exact ground truth: `neighbors` maps each query id to
    /// its true top-`k` neighbour ids (best first or any order — recall is set-based),
    /// self excluded. `Err` on an empty ground truth, `k == 0`, a list longer than `k`,
    /// a self-referential list, or a duplicate query id — a malformed oracle must not
    /// silently weaken the gate.
    pub fn new(k: usize, neighbors: Vec<(Id, Vec<Id>)>) -> Result<GroundTruth, String> {
        if k == 0 {
            return Err("GroundTruth: k must be >= 1".into());
        }
        if neighbors.is_empty() {
            return Err("GroundTruth: empty (no query ids — nothing to gate recall on)".into());
        }
        let mut seen = FxHashSet::default();
        for (id, list) in &neighbors {
            if !seen.insert(*id) {
                return Err(format!("GroundTruth: duplicate query id {id}"));
            }
            if list.len() > k {
                return Err(format!(
                    "GroundTruth: id {id} has {} neighbours > k = {k}",
                    list.len()
                ));
            }
            if list.contains(id) {
                return Err(format!("GroundTruth: id {id} lists itself as a neighbour"));
            }
        }
        Ok(GroundTruth { k, neighbors })
    }

    /// The `k` this ground truth was computed at.
    pub fn k(&self) -> usize {
        self.k
    }

    /// The `(query id, exact neighbour ids)` pairs.
    pub fn neighbors(&self) -> &[(Id, Vec<Id>)] {
        &self.neighbors
    }
}

/// Builds the **exact O(m²)** top-`k` ground truth over `vectors` by full pairwise cosine
/// scan (ties break on ascending id, matching [`nearest_exact`](crate::nearest_exact), so
/// the oracle is deterministic). Tractable up to roughly 10⁵ vectors — build it **once**
/// at that rung and reuse it to gate every later ANN pass. Same fail-closed input
/// validation as [`build_ann`].
pub fn exact_ground_truth(vectors: &[(Id, Vec<f32>)], k: usize) -> Result<GroundTruth, String> {
    if k == 0 {
        return Err("exact_ground_truth: k must be >= 1".into());
    }
    // Same fail-closed validation as build_ann (dims, finiteness, zero vectors, duplicate
    // ids) so the oracle can never be built over input the index itself would reject —
    // WITHOUT paying an HNSW build for it.
    validate(vectors).map_err(|e| format!("exact_ground_truth: {e}"))?;
    let neighbors = vectors
        .iter()
        .map(|(id, v)| {
            let mut scored: Vec<(Id, f32)> = vectors
                .iter()
                .filter(|(other, _)| other != id)
                .map(|(other, w)| (*other, crate::ann::cosine(v, w)))
                .collect();
            scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            scored.truncate(k);
            (*id, scored.into_iter().map(|(n, _)| n).collect())
        })
        .collect();
    GroundTruth::new(k, neighbors)
}

/// The dedup policy: the recall gate, the neighbour budget, and the merge threshold.
/// Freeze one policy per scale track and report metrics under it at each rung (the
/// issue-#2251 protocol) — changing the policy between rungs invalidates the comparison.
#[derive(Clone, Copy, Debug)]
pub struct DedupPolicy {
    /// The **pre-registered recall gate**: [`dedup`] refuses to emit merges when the
    /// index's measured recall against the exact ground truth is below this. Default
    /// `0.99` (the issue-#2251 pre-registration).
    pub recall_gate: f64,
    /// Cosine similarity at or above which two concepts are merge candidates. Default
    /// `0.995` — a plain default, **not** a canonical value: the right threshold is a
    /// property of the consumer's vectoriser and must be frozen per scale track.
    pub merge_threshold: f32,
    /// Neighbours examined per vector when collecting merge candidates. Default `10`.
    /// A pair further apart than the `k`-th neighbour is never examined, so `k` bounds
    /// the transitive-merge fan-in per vector.
    pub k: usize,
}

impl Default for DedupPolicy {
    fn default() -> Self {
        DedupPolicy {
            recall_gate: 0.99,
            merge_threshold: 0.995,
            k: 10,
        }
    }
}

/// The outcome of a gate-passing [`dedup`] run.
#[derive(Clone, Debug)]
pub struct DedupReport {
    /// The measured ANN recall against the exact ground truth (`>=` the gate, or [`dedup`]
    /// would have returned `Err` instead of this report).
    pub recall: f64,
    /// The merged ids: each `(duplicate, canonical)` maps a merged-away id to its group's
    /// retained representative (the **smallest id** in the group — deterministic).
    /// Sorted by duplicate id; canonical ids never appear on the left.
    pub merges: Vec<(Id, Id)>,
    /// The merge groups (transitive closure of above-threshold pairs, union-find), each
    /// sorted ascending with the canonical id first; only groups of `>= 2` ids appear.
    /// Sorted by canonical id.
    pub groups: Vec<Vec<Id>>,
}

/// **Recall-gated type-level dedup** (the issue-#2251 surface): verifies the ANN `index`'s
/// recall against the exact `ground_truth` **before** computing or emitting any merge.
///
/// Two phases, strictly ordered:
///
/// 1. **Gate.** For every `(query id, exact neighbours)` pair in `ground_truth`, the
///    index's top-`ground_truth.k()` (self excluded) is compared set-wise against the
///    exact list; recall = matched / expected over the whole oracle. If recall is below
///    `policy.recall_gate`, this returns `Err` and **no merge is computed** — an
///    under-recalling index must not silently under-merge (a missed true near-duplicate
///    is a silent correctness loss, not a graceful degradation).
/// 2. **Merge.** Only after the gate passes: every indexed vector's `policy.k` nearest
///    neighbours at cosine `>= policy.merge_threshold` become merge edges; groups are the
///    union-find transitive closure; each group keeps its smallest id as canonical.
///
/// Errors (fail-closed, never a silent skip): a gate failure; a `ground_truth` query id
/// absent from the index (an oracle/index mismatch would make the recall measurement
/// meaningless); a `policy.k` or `ground_truth.k() + 1` exceeding the index's build
/// `ef_search` (the beam could not surface enough candidates, silently deflating recall);
/// a non-finite or out-of-range gate/threshold.
///
/// The measured recall is evidence **for this index over this ground truth's corpus** —
/// re-gate after any re-vectorisation, and report per-rung metrics under one frozen
/// `policy`.
pub fn dedup(
    index: &ConceptAnnIndex,
    policy: &DedupPolicy,
    ground_truth: &GroundTruth,
) -> Result<DedupReport, String> {
    if !policy.recall_gate.is_finite() || !(0.0..=1.0).contains(&policy.recall_gate) {
        return Err(format!(
            "dedup: recall_gate {} outside [0, 1]",
            policy.recall_gate
        ));
    }
    if !policy.merge_threshold.is_finite() || !(-1.0..=1.0).contains(&policy.merge_threshold) {
        return Err(format!(
            "dedup: merge_threshold {} outside [-1, 1]",
            policy.merge_threshold
        ));
    }
    if policy.k == 0 {
        return Err("dedup: policy.k must be >= 1".into());
    }
    // The HNSW beam returns at most ef_search candidates; asking for more silently
    // truncates, deflating recall / merge fan-in. Reject the misconfiguration instead.
    let ef = index.policy.ef_search;
    let need = policy.k.max(ground_truth.k()) + 1; // +1: the self-hit is discarded
    if need > ef {
        return Err(format!(
            "dedup: needs top-{need} per query (max of policy.k = {}, ground_truth.k = {}, + 1 \
             for the self-hit) but the index was built with ef_search = {ef}; rebuild with a \
             larger ef_search",
            policy.k,
            ground_truth.k()
        ));
    }

    // Phase 1 — the recall gate. Set-based recall@k over the whole oracle.
    let (mut matched, mut expected) = (0usize, 0usize);
    for (id, exact) in ground_truth.neighbors() {
        if !index.pos.contains_key(id) {
            return Err(format!(
                "dedup: ground-truth query id {id} is not in the index (oracle/index mismatch)"
            ));
        }
        let ann = index.knn_of(*id, ground_truth.k());
        let ann_ids: FxHashSet<Id> = ann.into_iter().map(|(n, _)| n).collect();
        matched += exact.iter().filter(|n| ann_ids.contains(n)).count();
        expected += exact.len();
    }
    if expected == 0 {
        return Err("dedup: ground truth has no neighbour entries (recall is unmeasurable)".into());
    }
    let recall = matched as f64 / expected as f64;
    if recall < policy.recall_gate {
        return Err(format!(
            "dedup: measured ANN recall {recall:.4} is below the pre-registered gate {:.4} \
             ({matched}/{expected} exact neighbours recalled) — refusing to apply merges; \
             rebuild the index with a wider policy (ef_search/ef_construction) and re-gate",
            policy.recall_gate
        ));
    }

    // Phase 2 — merge edges from the gated index, then union-find.
    let n = index.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path halving
            x = parent[x];
        }
        x
    }
    for (i, id) in index.ids.iter().enumerate() {
        for (neighbor, score) in index.knn_of(*id, policy.k) {
            if score >= policy.merge_threshold {
                let (a, b) = (
                    find(&mut parent, i),
                    find(&mut parent, index.pos[&neighbor]),
                );
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut by_root: FxHashMap<usize, Vec<Id>> = FxHashMap::default();
    for i in 0..n {
        let root = find(&mut parent, i);
        by_root.entry(root).or_default().push(index.ids[i]);
    }
    let mut groups: Vec<Vec<Id>> = by_root
        .into_values()
        .filter(|g| g.len() >= 2)
        .map(|mut g| {
            g.sort_unstable();
            g
        })
        .collect();
    groups.sort_unstable_by_key(|g| g[0]);
    let mut merges: Vec<(Id, Id)> = groups
        .iter()
        .flat_map(|g| g[1..].iter().map(|&dup| (dup, g[0])))
        .collect();
    merges.sort_unstable();
    Ok(DedupReport {
        recall,
        merges,
        groups,
    })
}
