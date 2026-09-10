//! [OPUS-4.8] (sq-gr8mb / survey §A3) Exact-bitmap semi-join reducer on dense `u32` ids.
//!
//! A binary BGP join materialises its left side, then scans the next pattern and
//! merge/hash-joins on the connecting variable. A scan row survives that join **iff**
//! its join-key id appears in the left side's join column — every other scanned row is
//! pure waste. This module builds a membership set ([`KeyFilter`]) over the left side's
//! distinct join-key ids and lets [`crate::exec::scan_to_bindings`] DROP a row whose
//! join key is absent **before** projection, so the wasted rows never enter the join.
//!
//! Why a bitmap fits *this* engine. sparq's dictionary assigns DENSE `u32` ids in
//! `[1, dict::INLINE_BASE)` (see `sparq_core::dict`), so a flat bitmap over a key range
//! is a perfect-hash, ZERO-false-positive, branch-free membership test — strictly
//! cheaper and more accurate than the Bloom filters the wider join literature settled
//! for (CIDR'26 "Not Yannakakis"). When the keys are dense the bitmap is chosen; when
//! they are SPARSE+HUGE (a few keys scattered across a wide id span — e.g. inline
//! integers high in the id space, or local-vocab ids) a flat bitmap would waste memory,
//! so the reducer falls back to a hash set. Both are EXACT (no false positives): the
//! semi-join only removes rows the join would have dropped anyway, so the produced
//! result is unchanged — only fewer rows are scanned.
//!
//! **Correctness contract (load-bearing).** This is an OPT-IN performance feature
//! (`semijoin-bitmap`, OFF by default). With the feature OFF none of this code compiles
//! and the executor's existing path is byte-identical. With it ON the RESULT is
//! identical to OFF (the prefilter is membership-exact and only drops rows that would
//! fail the join), proven by the feature-on-vs-off differential
//! (`tests/semijoin_differential.rs`).

use sparq_core::dict::Id;

/// How many distinct keys, per id span covered, below which a flat bitmap is judged too
/// sparse and the hash-set fallback is used instead. The bitmap costs `span / 8` bytes;
/// the hash set costs roughly a machine word per key. Choosing the bitmap only when the
/// keys are dense enough keeps the membership structure small. (Density heuristic only —
/// it never affects RESULTS, just which exact structure backs the membership test.)
const MIN_KEY_DENSITY_INV: u64 = 64;

/// The largest id span for which a flat bitmap is ever allocated, regardless of density.
/// Bounds the worst-case bitmap to `MAX_BITMAP_SPAN / 8` bytes so a pathological key
/// range can never blow up memory; above it the hash set is used. (Also never affects
/// RESULTS.)
const MAX_BITMAP_SPAN: u64 = 1 << 28; // 256 Mi ids => 32 MiB bitmap ceiling.

/// An EXACT membership test over a set of join-key ids — the semi-join reducer's
/// "reachable id" structure. Backed by a flat bitmap when the keys are dense (the common
/// case for dictionary ids) and a hash set when they are sparse+huge. Either way
/// [`KeyFilter::contains`] has NO false positives, so filtering a scan by it removes
/// only rows that would fail the downstream join.
pub(crate) enum KeyFilter {
    /// Dense path: one bit per id in `[base, base + bits.len()*64)`. `contains(id)` is a
    /// word index + bit test. Ids outside the range are absent by construction (the
    /// range spans exactly the observed min..=max key).
    Bitmap { base: Id, bits: Vec<u64> },
    /// Sparse path: an exact hash set of the keys.
    Set(rustc_hash::FxHashSet<Id>),
}

impl KeyFilter {
    /// Builds a membership filter over the DISTINCT values in `keys` (the left side's
    /// join column). Returns `None` when there is nothing to gain — an empty key set
    /// (the join is already empty; the executor handles that separately) — so the caller
    /// can skip installing a prefilter. Chooses the bitmap when the keys are dense within
    /// their min..=max span and the span is bounded, else an exact hash set.
    pub(crate) fn build(keys: impl Iterator<Item = Id>) -> Option<KeyFilter> {
        // First pass: distinct keys + min/max, via an exact set. (The set is reused as
        // the sparse fallback, so the sparse path pays for it only once.)
        let mut set: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
        let mut min = Id::MAX;
        let mut max = Id::MIN;
        for k in keys {
            if set.insert(k) {
                if k < min {
                    min = k;
                }
                if k > max {
                    max = k;
                }
            }
        }
        if set.is_empty() {
            return None;
        }
        // The id range the bitmap would have to span (inclusive). `max >= min` holds
        // because the set is non-empty.
        let span = (max as u64) - (min as u64) + 1;
        // Dense ENOUGH and bounded => flat bitmap; else exact hash set. `set.len()` is the
        // distinct-key count; `span / set.len()` is the average gap between keys.
        let dense = span <= MAX_BITMAP_SPAN && span <= (set.len() as u64) * MIN_KEY_DENSITY_INV;
        if dense {
            let words = (span / 64 + 1) as usize;
            let mut bits = vec![0u64; words];
            for &k in &set {
                let off = (k - min) as usize;
                bits[off / 64] |= 1u64 << (off % 64);
            }
            Some(KeyFilter::Bitmap { base: min, bits })
        } else {
            Some(KeyFilter::Set(set))
        }
    }

    /// EXACT membership: `true` iff `id` is one of the keys the filter was built from. No
    /// false positives in either backing representation.
    #[inline]
    pub(crate) fn contains(&self, id: Id) -> bool {
        match self {
            KeyFilter::Bitmap { base, bits } => {
                // Below the range or above it => not a member (the bitmap spans exactly
                // the observed min..=max). The unsigned subtraction underflow-guard is
                // the `id < *base` check.
                if id < *base {
                    return false;
                }
                let off = (id - *base) as usize;
                let word = off / 64;
                match bits.get(word) {
                    Some(w) => (w >> (off % 64)) & 1 == 1,
                    None => false,
                }
            }
            KeyFilter::Set(s) => s.contains(&id),
        }
    }
}

/// The kind of backing store a [`KeyFilter`] chose, for tests / introspection only.
#[cfg(test)]
impl KeyFilter {
    pub(crate) fn is_bitmap(&self) -> bool {
        matches!(self, KeyFilter::Bitmap { .. })
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-5zf8i / survey §A4) Yannakakis bottom-up full-semijoin PREPASS.
//
// The A3 reducer above filters one scan against the *already-materialised* join
// result during the binary-join loop (a top-down, per-step prefilter). The
// Yannakakis prepass is the complementary, classical move: BEFORE the main join,
// reduce every relation against its neighbours in the acyclic join tree so that no
// "dangling" tuple (one that joins with nothing downstream) is ever materialised by
// the join. A full reducer (the bottom-up then top-down sweep) leaves every surviving
// tuple participating in the final answer, so the subsequent join runs in
// O(input + output) — no intermediate blow-up.
//
// This file owns only the PURE, store-free topology of that prepass: which relations
// are adjacent (share a join variable), and a join tree / reverse-topological visiting
// order over them. The row-level reduction (which reuses [`KeyFilter`] for the exact
// membership test) lives next to the materialised `Bindings` in `exec.rs`. Splitting it
// this way keeps the tree logic unit-testable without a `Graph`.
//
// CORRECTNESS NOTE (load-bearing). A semijoin is a pure FILTER: `R ⋉ S` keeps exactly
// the rows of `R` that have a join partner in `S`, so it can only ever DROP rows that
// the final join would itself drop — it never adds or alters a row. The prepass is
// therefore answer-preserving for ANY adjacency choice; the acyclic *join tree*
// (running-intersection) is what makes it a FULL reducer (removes ALL dangling tuples),
// which is the performance guarantee, not a correctness precondition. The executor only
// runs the prepass on the acyclic branch (cyclic BGPs keep the existing LFTJ).

/// A node of the join tree built over the relations of an acyclic BGP — a relation index
/// and (when not the root) its parent. The tree is rooted and the edges connect relations
/// that share at least one join variable, with the running-intersection property when the
/// BGP is α-acyclic.
///
/// Yannakakis-ONLY: gated to the `yannakakis` feature (the only consumer is the prepass in
/// `exec.rs`). Under the sibling `semijoin-bitmap` feature alone this type does not exist,
/// so the opt-in stays lean and `-D dead_code` is satisfied.
#[cfg(feature = "yannakakis")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TreeNode {
    /// Index of the relation (BGP pattern) this node stands for.
    pub(crate) rel: usize,
    /// Index INTO THE NODE LIST of this node's parent, or `usize::MAX` for the root.
    pub(crate) parent: usize,
}

#[cfg(feature = "yannakakis")]
impl TreeNode {
    /// `true` for the (single) root node, which has no parent.
    #[inline]
    pub(crate) fn is_root(&self) -> bool {
        self.parent == usize::MAX
    }
}

/// Builds a rooted join tree (really a forest, joined under a synthetic root order) over
/// `rel_vars` — one entry per relation, each the set of JOIN-variable ids that relation
/// binds (an id per distinct variable; constants are not included). Two relations are
/// adjacent when their variable sets intersect. The returned nodes are in a PRE-ORDER
/// (each parent precedes its children), so:
///   * iterating the slice in REVERSE visits children before parents — the bottom-up
///     (leaf → root) semijoin order, and
///   * iterating it FORWARD visits parents before children — the top-down order.
///
/// The construction is a greedy BFS over the adjacency graph: it seeds a component at its
/// lowest-index relation and attaches each newly reached relation to the already-placed
/// neighbour it shares the most variables with. A disconnected BGP (independent
/// components, i.e. a cross product) yields several roots; each component is still fully
/// reduced within itself, and the cross product across components is handled by the join.
///
/// This is intentionally simple and robust rather than a textbook GYO tree: because the
/// semijoin sweep is answer-preserving for any spanning structure (see the module note),
/// a maximum-overlap spanning forest is sufficient and, for α-acyclic BGPs, recovers a
/// running-intersection tree.
///
/// Yannakakis-ONLY: gated to the `yannakakis` feature; its sole caller is the prepass in
/// `exec.rs`. Absent under `semijoin-bitmap` alone (keeps the opt-in lean, satisfies
/// `-D dead_code`).
#[cfg(feature = "yannakakis")]
pub(crate) fn build_join_tree(rel_vars: &[Vec<u32>]) -> Vec<TreeNode> {
    let n = rel_vars.len();
    let mut placed = vec![false; n];
    let mut nodes: Vec<TreeNode> = Vec::with_capacity(n);

    // Number of shared variables between two relations' (small, sorted-or-not) var sets.
    let overlap = |a: usize, b: usize| -> usize {
        rel_vars[a].iter().filter(|v| rel_vars[b].contains(v)).count()
    };

    while placed.iter().any(|p| !p) {
        // Seed a new component at the lowest unplaced relation (a fresh root).
        let seed = (0..n).find(|&i| !placed[i]).unwrap();
        placed[seed] = true;
        let root_node = nodes.len();
        nodes.push(TreeNode { rel: seed, parent: usize::MAX });

        // Grow the component: repeatedly attach the unplaced relation that shares the most
        // variables with SOME already-placed node of this component, as that node's child.
        loop {
            let mut best: Option<(usize, usize, usize)> = None; // (overlap, child_rel, parent_node)
            // The component's already-placed nodes are `nodes[root_node..]`; this snapshot is
            // stable for the inner search (nodes only grows AFTER a child is chosen, below).
            let placed_end = nodes.len();
            for (node_idx, node) in nodes.iter().enumerate().take(placed_end).skip(root_node) {
                let prel = node.rel;
                for (cand, &is_placed) in placed.iter().enumerate() {
                    if is_placed {
                        continue;
                    }
                    let ov = overlap(prel, cand);
                    if ov == 0 {
                        continue;
                    }
                    // Prefer the largest overlap; break ties by smallest candidate index so
                    // the tree is deterministic (the result never depends on it, but a
                    // stable shape keeps tests and EXPLAIN reproducible).
                    let better = match best {
                        None => true,
                        Some((bov, bcand, _)) => ov > bov || (ov == bov && cand < bcand),
                    };
                    if better {
                        best = Some((ov, cand, node_idx));
                    }
                }
            }
            match best {
                Some((_, cand, parent_node)) => {
                    placed[cand] = true;
                    nodes.push(TreeNode { rel: cand, parent: parent_node });
                }
                None => break, // component exhausted
            }
        }
    }
    nodes
}

/// Whether `id` is a DICTIONARY id (the dense `[1, INLINE_BASE)` range the bitmap path is
/// designed for) rather than an inline integer / local-vocab id high in the `u32` space.
/// Not used to gate correctness (the filter is exact for any id) — kept as a documented
/// predicate next to the density rationale so the "dense ids" claim is checkable.
#[inline]
#[cfg(test)]
pub(crate) fn is_dense_dict_id(id: Id) -> bool {
    id != sparq_core::dict::NO_ID && id < sparq_core::dict::INLINE_BASE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_keys_build_none() {
        assert!(KeyFilter::build(std::iter::empty()).is_none());
    }

    #[test]
    fn dense_keys_pick_bitmap_and_membership_is_exact() {
        // 0..1000 dense dictionary ids (id 0 is NO_ID, so start at 1).
        let keys: Vec<Id> = (1..1000).collect();
        let f = KeyFilter::build(keys.iter().copied()).unwrap();
        assert!(f.is_bitmap(), "a fully-dense range must use the bitmap");
        for k in 1..1000 {
            assert!(f.contains(k), "present key {} must be a member", k);
        }
        assert!(!f.contains(0), "id below the range is absent");
        assert!(!f.contains(1000), "id above the range is absent");
        assert!(!f.contains(Id::MAX), "far-away id is absent");
    }

    #[test]
    fn sparse_huge_keys_fall_back_to_set_and_stay_exact() {
        // A handful of keys scattered across the whole u32 span: a bitmap would need
        // ~512 MiB, so the sparse fallback must kick in.
        let keys = [1u32, 1 << 20, 1 << 28, (1u32 << 31) + 5, u32::MAX - 1];
        let f = KeyFilter::build(keys.iter().copied()).unwrap();
        assert!(!f.is_bitmap(), "a sparse+huge span must use the set fallback");
        for &k in &keys {
            assert!(f.contains(k), "present key {} must be a member", k);
        }
        assert!(!f.contains(2), "absent key is not a member");
        assert!(!f.contains((1 << 28) + 1), "absent key is not a member");
    }

    #[test]
    fn duplicate_keys_are_deduplicated() {
        let keys = [5u32, 5, 5, 7, 7, 9];
        let f = KeyFilter::build(keys.iter().copied()).unwrap();
        for k in [5, 7, 9] {
            assert!(f.contains(k));
        }
        assert!(!f.contains(6));
        assert!(!f.contains(8));
    }

    #[test]
    fn single_key_membership() {
        let f = KeyFilter::build(std::iter::once(42u32)).unwrap();
        assert!(f.contains(42));
        assert!(!f.contains(41));
        assert!(!f.contains(43));
    }

    #[test]
    fn dense_dict_id_predicate_matches_dict_partition() {
        use sparq_core::dict;
        assert!(is_dense_dict_id(1));
        assert!(is_dense_dict_id(dict::INLINE_BASE - 1));
        assert!(!is_dense_dict_id(dict::NO_ID));
        assert!(!is_dense_dict_id(dict::INLINE_BASE)); // first inline integer id
    }

    // ---- [OPUS-4.8] (sq-5zf8i / §A4) join-tree builder ---------------------
    // Yannakakis-ONLY: these exercise `build_join_tree`/`TreeNode`, both gated to the
    // `yannakakis` feature, so the tests must be too — otherwise the `semijoin-bitmap`-only
    // test build would reference items that don't exist.

    /// Every relation is placed exactly once, and every non-root parent points at an
    /// earlier (already-placed) node — the pre-order invariant the sweep relies on.
    #[cfg(feature = "yannakakis")]
    fn assert_tree_well_formed(rel_vars: &[Vec<u32>], nodes: &[TreeNode]) {
        assert_eq!(nodes.len(), rel_vars.len(), "every relation is a node exactly once");
        let mut seen = vec![false; rel_vars.len()];
        let mut roots = 0;
        for (i, nd) in nodes.iter().enumerate() {
            assert!(!seen[nd.rel], "relation {} placed twice", nd.rel);
            seen[nd.rel] = true;
            if nd.is_root() {
                roots += 1;
            } else {
                assert!(nd.parent < i, "parent {} must precede child node {}", nd.parent, i);
            }
        }
        assert!(seen.iter().all(|&s| s), "all relations placed");
        assert!(roots >= 1, "at least one root");
    }

    #[cfg(feature = "yannakakis")]
    #[test]
    fn chain_join_tree_is_a_single_spine() {
        // Chain: r0(?a,?b) r1(?b,?c) r2(?c,?d) — adjacency a-b, b-c, c-d.
        let rel_vars = vec![vec![0u32, 1], vec![1, 2], vec![2, 3]];
        let nodes = build_join_tree(&rel_vars);
        assert_tree_well_formed(&rel_vars, &nodes);
        // One connected component => exactly one root.
        assert_eq!(nodes.iter().filter(|n| n.is_root()).count(), 1);
    }

    #[cfg(feature = "yannakakis")]
    #[test]
    fn star_join_tree_roots_at_the_hub() {
        // Star: hub r0(?p,?a) and spokes r1(?p,?b) r2(?p,?c) r3(?p,?d), all share ?p (=0).
        let rel_vars = vec![vec![0u32, 1], vec![0, 2], vec![0, 3], vec![0, 4]];
        let nodes = build_join_tree(&rel_vars);
        assert_tree_well_formed(&rel_vars, &nodes);
        assert_eq!(nodes.iter().filter(|n| n.is_root()).count(), 1);
        // The root is the seed relation r0; the spokes all attach (directly or not) under it.
        assert!(nodes[0].is_root() && nodes[0].rel == 0);
    }

    #[cfg(feature = "yannakakis")]
    #[test]
    fn disconnected_bgp_yields_one_root_per_component() {
        // Two independent edges: {r0(?a,?b), r1(?b,?c)} and {r2(?x,?y)} — a cross product.
        let rel_vars = vec![vec![0u32, 1], vec![1, 2], vec![10, 11]];
        let nodes = build_join_tree(&rel_vars);
        assert_tree_well_formed(&rel_vars, &nodes);
        assert_eq!(nodes.iter().filter(|n| n.is_root()).count(), 2, "two components => two roots");
    }

    #[cfg(feature = "yannakakis")]
    #[test]
    fn single_relation_is_its_own_root() {
        let rel_vars = vec![vec![0u32, 1]];
        let nodes = build_join_tree(&rel_vars);
        assert_tree_well_formed(&rel_vars, &nodes);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].is_root());
    }

    #[cfg(feature = "yannakakis")]
    #[test]
    fn snowflake_join_tree_well_formed() {
        // Star on ?p plus a chain ?p->?city->?country:
        // r0(?p,?name) r1(?p,?age) r2(?p,?city) r3(?city,?country)
        let rel_vars = vec![vec![0u32, 1], vec![0, 2], vec![0, 3], vec![3, 4]];
        let nodes = build_join_tree(&rel_vars);
        assert_tree_well_formed(&rel_vars, &nodes);
        assert_eq!(nodes.iter().filter(|n| n.is_root()).count(), 1);
    }
}
