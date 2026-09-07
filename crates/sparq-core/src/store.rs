//! Triple store: the six sorted permutation indexes over dictionary-encoded
//! triples (Hexastore / RDF-3X / QLever design).
//!
//! Storing all six orderings (SPO SOP PSO POS OSP OPS) means every triple
//! pattern is answered by a single contiguous range (binary search on the
//! bound prefix), and the scan output is sorted by the remaining positions —
//! which is exactly what merge joins need. M1 holds each permutation as a
//! sorted `Vec<[Id; 3]>`; later milestones replace these with block-compressed,
//! optionally memory-mapped columns.

use crate::dict::Id;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// The six permutations. Each names the order of (subject, predicate, object)
/// columns as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Spo,
    Sop,
    Pso,
    Pos,
    Osp,
    Ops,
}

/// The permutations actually built and searched. The full six give every triple
/// pattern a sorted scan in the order any merge join wants. The `compact-index` set
/// {SPO, POS, OSP} still answers EVERY triple pattern from one index (SPO→S*/SP*,
/// POS→P*/PO*, OSP→O*/OS*) at half the memory, at the cost of some merge joins (and
/// some lazy-count fast paths) falling back to hashing / sorting.
// Compact set on wasm ALWAYS (memory-bound target), or on native opt-in via the
// `compact-index` feature (for testing). Keyed on `target_arch` — NOT just a feature —
// so the wasm choice does not leak to the native build via Cargo feature unification.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];

impl Perm {
    pub const ALL: [Perm; 6] = [Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];

    /// The column indices (into a canonical s,p,o triple) in this permutation's
    /// sort order. e.g. POS -> 1,2,0.
    #[inline]
    pub fn order(self) -> [usize; 3] {
        match self {
            Perm::Spo => [0, 1, 2],
            Perm::Sop => [0, 2, 1],
            Perm::Pso => [1, 0, 2],
            Perm::Pos => [1, 2, 0],
            Perm::Osp => [2, 0, 1],
            Perm::Ops => [2, 1, 0],
        }
    }
}

/// A triple pattern over ids: `None` is a variable (wildcard), `Some(id)` is
/// bound.
pub type Pattern = [Option<Id>; 3];

use rustc_hash::{FxHashMap, FxHashSet};

/// Per-predicate statistics for cardinality estimation (a characteristic-set-lite
/// summary): how many triples use the predicate, and how many *distinct* subjects
/// and objects it relates. Lets the planner estimate join result sizes.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PredStat {
    pub count: usize,
    pub ndv_subj: usize,
    pub ndv_obj: usize,
}

/// A permutation index's storage: either an in-memory `Vec` (built / loaded) or, with
/// the `mmap` feature, a memory-mapped on-disk file — so a dataset larger than RAM can
/// be queried, the OS paging in only the working set (out-of-core).
enum PermData {
    Owned(Vec<[Id; 3]>),
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap),
    /// Block-compressed (~4-6 B/triple vs 12). The memory-bound storage mode for the
    /// browser: scans decode only the blocks the key-range touches. See [`compress`].
    Compressed(crate::compress::CompressedPerm),
}

impl Default for PermData {
    fn default() -> Self {
        PermData::Owned(Vec::new())
    }
}

impl PermData {
    /// Borrows the rows as a contiguous slice. Valid only for the raw (Owned/Mapped)
    /// modes — the compressed mode has no flat layout, so callers that may hold a
    /// compressed perm must go through [`rows_in`](Self::rows_in) instead.
    #[inline]
    fn as_slice(&self) -> &[[Id; 3]] {
        match self {
            PermData::Owned(v) => v,
            #[cfg(feature = "mmap")]
            PermData::Mapped(m) => {
                let bytes: &[u8] = m;
                let n = bytes.len() / std::mem::size_of::<[Id; 3]>();
                // SAFETY: the file is a whole number of little-endian [u32;3] triples and
                // an mmap is page-aligned (>= the 4-byte alignment of `u32`).
                unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<[Id; 3]>(), n) }
            }
            PermData::Compressed(_) => unreachable!("as_slice on a compressed permutation"),
        }
    }

    /// The rows matching the inclusive key range `[lo, hi]`, sorted. Raw modes binary-
    /// search and BORROW a sub-slice (no allocation); the compressed mode decodes only
    /// the spanning blocks and returns an OWNED `Vec`. Either way the operators above
    /// receive a `&[[Id;3]]` (via the `Cow`), so their algorithms are unchanged.
    #[inline]
    fn rows_in(&self, lo: [Id; 3], hi: [Id; 3]) -> std::borrow::Cow<'_, [[Id; 3]]> {
        match self {
            PermData::Compressed(c) => std::borrow::Cow::Owned(c.range(lo, hi)),
            _ => {
                let rows = self.as_slice();
                let s = lower_bound(rows, &lo);
                let e = upper_bound(rows, &hi);
                std::borrow::Cow::Borrowed(&rows[s..e])
            }
        }
    }

    /// Cheap count of rows in `[lo, hi]` (for the planner) — no full materialization.
    #[inline]
    fn count_in(&self, lo: [Id; 3], hi: [Id; 3]) -> usize {
        match self {
            PermData::Compressed(c) => c.count_range(lo, hi),
            _ => {
                let rows = self.as_slice();
                upper_bound(rows, &hi) - lower_bound(rows, &lo)
            }
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            PermData::Compressed(c) => c.len(),
            _ => self.as_slice().len(),
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            PermData::Owned(v) => v.capacity() * std::mem::size_of::<[Id; 3]>(),
            #[cfg(feature = "mmap")]
            PermData::Mapped(_) => 0, // resident pages are charged to the OS page cache, not the heap
            PermData::Compressed(c) => c.heap_bytes(),
        }
    }
}

/// Pending updates layered over the immutable base indexes (the T17 delta-overlay):
/// triples INSERTED since the last compaction, and base triples DELETED since then.
/// Consulted at scan time — the base stays immutable (and mmap-able), and an update
/// batch costs O(batch) instead of the O(n) full rebuild. Invariants kept by
/// [`TripleStore::apply_delta`]: `added` is canonical-SPO sorted + deduplicated and
/// DISJOINT from both the base and `deleted`; `deleted` only ever holds base triples.
///
/// `Clone` because [`TripleStore::fork`] carries the overlay into the forked store
/// BY VALUE (O(overlay), bounded by the compaction policy) while the base indexes are
/// shared structurally.
#[derive(Default, Clone)]
struct Overlay {
    added: Vec<[Id; 3]>,
    deleted: FxHashSet<[Id; 3]>,
    /// CACHED perm-sorted projections of `added`, indexed by `perm as usize`
    /// (sq-7d3dj.16). See [`Overlay::added_sorted`].
    added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
}

impl Overlay {
    /// All of `added` projected into `perm` column order and SORTED in it — computed
    /// ONCE per permutation and reused until `added` changes (sq-7d3dj.16).
    ///
    /// Built LAZILY on the first scan that needs this permutation rather than eagerly
    /// for all six in [`TripleStore::apply_delta`]: a write batch then stays O(batch)
    /// (it only drops the caches, see [`Overlay::invalidate_added`]) instead of paying
    /// O(6·k log k) per call, and a store only ever materialises the projections its
    /// query mix actually scans — so the memory cost is bounded by the permutations in
    /// use, not a flat 6×. SPO needs no projection or sort at all: `added` is already
    /// canonical-SPO sorted, so that permutation ALIASES it and costs nothing.
    ///
    /// `OnceLock` (not `RefCell`) because scans take `&self` and `TripleStore` must stay
    /// `Sync`; a race just recomputes the same value and discards the loser.
    fn added_sorted(&self, perm: Perm) -> &[[Id; 3]] {
        let order = perm.order();
        if order == [0, 1, 2] {
            return &self.added; // SPO: `added` is already the projection, already sorted
        }
        self.added_by_perm[perm as usize].get_or_init(|| {
            let mut rows: Vec<[Id; 3]> =
                self.added.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
            rows.sort_unstable();
            rows
        })
    }

    /// Drops every cached projection — called whenever `added` is about to change, so a
    /// cache can never outlive the `added` it was derived from.
    fn invalidate_added(&mut self) {
        for slot in &mut self.added_by_perm {
            slot.take();
        }
    }

    /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
    /// `perm` column order, SORTED in that order. A BORROWED sub-slice of the cached
    /// perm-sorted projection located by two binary searches — O(log k + m) on k
    /// insertions and m matches, and allocation-free.
    fn added_rows(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> &[[Id; 3]] {
        let rows = self.added_sorted(perm);
        let start = rows.partition_point(|r| *r < lo);
        let end = rows.partition_point(|r| *r <= hi);
        &rows[start..end]
    }

    /// Merges the (perm-sorted) base rows with the overlay for one scan: base rows whose
    /// canonical triple is deleted are dropped, and the matching `added` rows are merge-
    /// interleaved — so the output keeps the permutation's sort order, preserving the
    /// guarantees downstream merge joins rely on. `added` is disjoint from the base, so
    /// no duplicate handling is needed.
    fn merge(&self, base: &[[Id; 3]], perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
        let add = self.added_rows(perm, lo, hi);
        let order = perm.order();
        let mut out = Vec::with_capacity(base.len() + add.len());
        let mut ai = 0;
        let check_deleted = !self.deleted.is_empty();
        for &row in base {
            if check_deleted {
                let mut spo = [0; 3];
                spo[order[0]] = row[0];
                spo[order[1]] = row[1];
                spo[order[2]] = row[2];
                if self.deleted.contains(&spo) {
                    continue;
                }
            }
            while ai < add.len() && add[ai] < row {
                out.push(add[ai]);
                ai += 1;
            }
            out.push(row);
        }
        out.extend_from_slice(&add[ai..]);
        out
    }

    /// How many overlay triples fall in the `[lo, hi]` range of `perm` — the exact
    /// correction to a base range count. The `added` side rides the cached perm-sorted
    /// projection (O(log k), two binary searches); the `deleted` side is an unordered
    /// hash set and stays O(|deleted|).
    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
        let order = perm.order();
        let add = self.added_rows(perm, lo, hi).len();
        let del = self
            .deleted
            .iter()
            .filter(|t| {
                let r = [t[order[0]], t[order[1]], t[order[2]]];
                r >= lo && r <= hi
            })
            .count();
        (add, del)
    }

    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.deleted.is_empty()
    }

    fn heap_bytes(&self) -> usize {
        // The cached perm-sorted projections are part of the overlay's footprint; SPO
        // aliases `added` and so never occupies a slot.
        let cached: usize = self
            .added_by_perm
            .iter()
            .filter_map(|slot| slot.get())
            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
            .sum();
        self.added.capacity() * std::mem::size_of::<[Id; 3]>()
            + self.deleted.capacity() * 13
            + cached
    }
}

pub struct TripleStore {
    // Each permutation in its column order, sorted (so binary search on a bound prefix
    // is a plain lexicographic comparison of the leading columns) — owned or mmap'd.
    //
    // Behind an `Arc` so [`fork`](Self::fork) can SHARE the immutable base indexes
    // across snapshot generations (the structural fork): every store is born
    // shareable, a fork is an Arc bump. The only post-build mutation,
    // [`decompress_to_ram`](Self::decompress_to_ram), goes through `Arc::get_mut`
    // (it runs on freshly opened, never-yet-shared stores). Cost when unused: one
    // extra pointer indirection per scan/estimate CALL (not per row) — measured in
    // the flat-read benchmark as within noise.
    perms: std::sync::Arc<[PermData; 6]>,
    // Per-predicate stats keyed by predicate id (for the cost-based planner).
    // Arc-shared across forks like the permutations (read-only after build).
    pred_stats: std::sync::Arc<FxHashMap<Id, PredStat>>,
    // The delta-overlay of pending updates, `None` when there are none — so the scan
    // hot path pays exactly one (perfectly predicted) branch when no update happened.
    // NOTE: `pred_stats` is not overlay-adjusted (planner estimates only); `estimate`
    // and `len` are exact.
    overlay: Option<Box<Overlay>>,
}

/// LSD radix sort of `[Id; 3]` rows into ascending lexicographic order — column 0
/// major, then 1, then 2 — the exact ordering a comparison `sort_unstable()`
/// produces on the same rows (an `[Id; 3]` compares lexicographically, and with
/// `Id = u32` the row packs into a 96-bit key whose numeric order IS that
/// lexicographic order). This is the O(n) index-build sort that replaces the
/// branchy comparison quicksort over the packed permutation tuples — the single
/// largest ingest self-time bucket (`research/engine-performance-review.md` §1.1,
/// sq-7d3dj.17). [OPUS-4.8]
///
/// Output equivalence is exact and gated: for any input, the multiset is preserved
/// and the result is fully sorted, so it is BYTE-IDENTICAL to `sort_unstable()`
/// (equal rows are indistinguishable, so stability is irrelevant). See the
/// `radix_sort_equiv_comparison_sort` differential-fuzz test.
///
/// Twelve least-significant-digit passes over the 12 key bytes (least-significant
/// first): passes 0..4 = column 2, 4..8 = column 1, 8..12 = column 0. A pass whose
/// digit is constant across every row (e.g. the high bytes of a small dictionary)
/// is a no-op and skipped; the double-buffer invariant keeps the current partial
/// result in `v` whether or not a pass runs, so the final result is always in `v`.
fn radix_sort_rows(v: &mut Vec<[Id; 3]>) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Scratch back-buffer; `v` and `scratch` are swapped after each executed pass so
    // the sorted-so-far data always lives in `v` (a skipped pass leaves it there too).
    let mut scratch: Vec<[Id; 3]> = vec![[0; 3]; n];
    for pass in 0..12usize {
        // Byte `pass` of the packed key, LSB first. col2 holds bytes 0..4, col1 4..8,
        // col0 8..12 — so the most-significant byte (pass 11) is column 0's top byte.
        let col = 2 - pass / 4;
        let shift = ((pass % 4) * 8) as u32;
        let digit = |row: &[Id; 3]| ((row[col] >> shift) & 0xff) as usize;

        // Histogram of this pass's digit.
        let mut count = [0usize; 256];
        for row in v.iter() {
            count[digit(row)] += 1;
        }
        // If every row shares one digit value this pass is a stable no-op — skip the
        // scatter (and the buffer swap), leaving the correct partial result in `v`.
        if count[digit(&v[0])] == n {
            continue;
        }
        // Prefix-sum the histogram into per-digit start offsets.
        let mut sum = 0usize;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        // Stable scatter into the back-buffer, then make it the live buffer.
        for row in v.iter() {
            let d = digit(row);
            scratch[count[d]] = *row;
            count[d] += 1;
        }
        std::mem::swap(v, &mut scratch);
    }
}

/// Stable LSD radix sort of `v` by COLUMN 0 ONLY — rows that tie on column 0 keep their
/// incoming relative order. Four passes over the four bytes of the leading column (a pass
/// whose digit is constant across every row is skipped, so a small dictionary costs one
/// or two passes, not four).
///
/// This is the primitive the DERIVED permutation build rests on
/// ([`TripleStore::derive_perm`], sq-dzfzq): re-sorting an already-sorted run by its NEW
/// leading column alone is enough to reach full lexicographic order, because stability
/// preserves the source order — which is exactly the remaining two columns, ascending —
/// inside every tie group.
fn radix_sort_rows_by_col0(v: &mut Vec<[Id; 3]>) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Allocated on the first pass that actually scatters; a fully constant leading column
    // (e.g. a single-subject graph) therefore allocates nothing at all.
    let mut scratch: Vec<[Id; 3]> = Vec::new();
    for pass in 0..4usize {
        let shift = (pass * 8) as u32;
        let digit = |row: &[Id; 3]| ((row[0] >> shift) & 0xff) as usize;

        let mut count = [0usize; 256];
        for row in v.iter() {
            count[digit(row)] += 1;
        }
        // Constant digit this pass -> a stable no-op; leave the partial result in `v`.
        if count[digit(&v[0])] == n {
            continue;
        }
        if scratch.len() != n {
            scratch = vec![[0; 3]; n];
        }
        let mut sum = 0usize;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        for row in v.iter() {
            let d = digit(row);
            scratch[count[d]] = *row;
            count[d] += 1;
        }
        std::mem::swap(v, &mut scratch);
    }
}

/// The DERIVATION PLAN (sq-dzfzq): waves of `(source, destination)` permutation pairs.
/// SPO is built once by the full 12-digit [`radix_sort_rows`] + the ONE dedup; every other
/// [`BUILT`] permutation is then DERIVED from an already-materialised one by a column
/// re-map plus a stable sort on its new leading column alone ([`radix_sort_rows_by_col0`]).
///
/// Pairs within a wave are independent (their sources are all already materialised) and so
/// run concurrently; the waves themselves are ordered. Depth is 2 derivations, so the
/// critical path is `full-sort + 2 single-column sorts` rather than six full sorts.
///
/// Every pair must satisfy the derivability invariant asserted in
/// [`TripleStore::derive_perm`]: deleting the destination's leading column from the
/// SOURCE's column order must leave the destination's other two columns, in order.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
const DERIVE_PLAN: &[&[(Perm, Perm)]] = &[
    // Wave 1 — both straight off the deduped SPO run.
    &[(Perm::Spo, Perm::Pso), (Perm::Spo, Perm::Osp)],
    // Wave 2 — off wave 1.
    &[(Perm::Pso, Perm::Ops), (Perm::Osp, Perm::Pos), (Perm::Osp, Perm::Sop)],
];
/// The `compact-index` / wasm plan: only {SPO, POS, OSP} are [`BUILT`], and POS derives
/// from OSP (deleting P from `[O,S,P]` leaves `[O,S]` = POS's trailing columns).
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
const DERIVE_PLAN: &[&[(Perm, Perm)]] = &[&[(Perm::Spo, Perm::Osp)], &[(Perm::Osp, Perm::Pos)]];

impl TripleStore {
    /// Builds the [`BUILT`] permutation indexes from canonical s,p,o triples (all
    /// six by default; just SPO/POS/OSP under `compact-index`). SPO is sorted (in
    /// parallel) and deduplicated first; the rest are independent and built concurrently.
    pub fn from_triples(triples: Vec<[Id; 3]>) -> Self {
        let perms = Self::build_raw_perms(triples);
        let pred_stats = Self::compute_pred_stats(&perms);
        TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None }
    }

    /// Like [`from_triples`](Self::from_triples) but stores each permutation
    /// BLOCK-COMPRESSED (~4-6 B/triple vs 12) — the memory-bound storage mode for the
    /// browser, where holding 2.5x more triples in the same RAM matters more than the
    /// per-scan decode cost. Cardinality stats are computed from the raw perms *before*
    /// encoding (so neither the build nor the planner ever decodes a whole index).
    ///
    /// [FABLE-5] sq-559dp — encodes through `CompressedPerm::encode_emit`, the SAME one-place
    /// emit-format gate the on-disk save path uses, so `SPARQ_STORE_PROFILE=compressed` can opt
    /// into the `SPQCPRM2` frame-of-reference block stream (`spqcprm2` feature +
    /// `SPARQ_EMIT_FORMAT=v2` / `with_emit_format`) instead of being pinned to V1. The default
    /// build has the gate compiled out, so the in-RAM stream stays byte-identical to V1.
    pub fn from_triples_compressed(triples: Vec<[Id; 3]>) -> Self {
        let raw = Self::build_raw_perms(triples);
        let pred_stats = Self::compute_pred_stats(&raw);
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        for (i, pd) in raw.into_iter().enumerate() {
            if let PermData::Owned(v) = pd {
                if !v.is_empty() {
                    perms[i] = PermData::Compressed(crate::compress::CompressedPerm::encode_emit(&v));
                }
            }
        }
        TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None }
    }

    /// Builds the [`BUILT`] raw permutation indexes from canonical [s,p,o] triples (all six
    /// by default; just SPO/POS/OSP under `compact-index`). Two `cfg`-selected bodies — the
    /// PARALLEL build runs each [`DERIVE_PLAN`] wave concurrently, the NO-THREADS build runs
    /// the same plan in order — kept as separate `fn` definitions so the wasm codegen carries
    /// no rayon shape at all.
    ///
    /// [SONNET-4.6 sq-dzfzq] DERIVED build. One full 12-digit [`radix_sort_rows`] + ONE dedup
    /// produces SPO; every other permutation is then [derived](Self::derive_perm) from an
    /// already-sorted run by a column re-map plus a stable sort on its NEW LEADING COLUMN
    /// alone — at most four byte passes over one column, usually fewer (a predicate column
    /// that fits in a byte costs exactly one), and no dedup pass. Total sort work drops from
    /// six full sorts (~72 byte passes over the whole row set, plus six dedups) to one full
    /// sort plus five single-column sorts (~25 passes, one dedup). Since large-graph ingest is
    /// memory-bandwidth bound, that traffic reduction is the point; the plan's 2-derivation
    /// depth keeps the critical path short.
    ///
    /// This SUPERSEDES the sq-7d3dj.31 concurrent-N build, which sorted and deduped all six
    /// permutations independently and in parallel. Concurrent-N bought wall-clock by spending
    /// 6x the sort work at 6x the memory traffic — a good trade only while cores sit idle and
    /// bandwidth is not the binding constraint.
    ///
    /// WHICH BUILD WINS, AND WHY IT IS NOT SETTLED. The two builds trade the same quantity in
    /// opposite directions — derived does strictly less work on a critical path of ONE full
    /// sort plus 2 derivations, concurrent-N does six sorts' worth but on N independent tasks —
    /// so the outcome is governed by THREADS vs PERMUTATIONS:
    ///
    /// * With six permutations, derived is expected to win, by a margin that NARROWS as threads
    ///   are added, because concurrent-N is the arm that has parallelism left to spend.
    /// * With three permutations (`compact-index`) and no threads — the wasm configuration —
    ///   the trade does not exist and derived wins outright.
    /// * With three permutations AND enough threads to give every permutation its own core,
    ///   concurrent-N runs all three in parallel while the derived chain SPO->OSP->POS is
    ///   strictly serial, so derived LOSES. This is a knowing, documented regression in a
    ///   configuration that ships nowhere — `compact-index` is wasm-only in production
    ///   (`BUILT` keys it on `target_arch`, and the wasm build has no threads) and is
    ///   native-opt-in "for testing"; the one native consumer is the `bench/memtier` research
    ///   spike. It is NOT worth a second build path here; see the follow-up issue.
    ///
    /// The above is the STRUCTURAL argument, and the direction of the six-permutation trade —
    /// including whether concurrent-N eventually catches up at a high enough thread count — is
    /// UNVERIFIED on the canonical setup. The bead's canonical gate — ingest wall + query
    /// latency on WatDiv/synthetic-social at two scales, on the dedicated bench box — is still
    /// OUTSTANDING and must run before this is treated as a settled multi-core win. To generate
    /// current numbers for your own machine (non-canonical, do not commit them), run the
    /// in-tree A/B against the replaced body, `measure_derived_vs_radix_all_build`, sweeping
    /// `RAYON_NUM_THREADS` and the feature axis as documented on that test.
    ///
    /// Correctness: a column permutation is a BIJECTION on rows, so deduplicating SPO alone
    /// deduplicates every derived permutation, and "the deduped triple set, permuted, fully
    /// sorted" is unique — so each perm stays BYTE-IDENTICAL to the reference
    /// `sort_unstable`+dedup+permute. Gated by the `from_triples_perms_match_reference_sort`
    /// and `derived_perms_match_radix_all_build` differential tests.
    ///
    /// That byte-identity is also why this is an INGEST-ONLY change: scan/lookup/estimate, the
    /// delta-overlay and save/open all read the same rows, in the same order, out of Vecs of
    /// the same length AND capacity (the derived Vecs are exact-sized by construction — see
    /// `build_raw_perms_no_capacity_slack`). There is no first-touch cost and no query-latency
    /// dimension to trade, because no permutation is materialised any later than before. The
    /// LAZY half of sq-dzfzq — deferring a rarely-scanned permutation to its first use, which
    /// WOULD move cost into the query path — is deliberately NOT taken here; it is a separate,
    /// higher-risk change and is left to a follow-up.
    #[cfg(feature = "parallel")]
    fn build_raw_perms(mut triples: Vec<[Id; 3]>) -> [PermData; 6] {
        radix_sort_rows(&mut triples);
        triples.dedup();
        // [SONNET-4.6 sq-7d3dj.32.1] Eliminate dedup capacity slack: after dedup the Vec retains
        // its pre-dedup allocation. shrink_to_fit realigns len == capacity so heap_bytes() (which
        // counts capacity()) returns zero slack. The DERIVED perms are exact-sized by
        // construction (an ExactSize map-collect, then a same-length radix double-buffer).
        triples.shrink_to_fit();

        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        perms[Perm::Spo as usize] = PermData::Owned(triples);
        for wave in DERIVE_PLAN {
            // Sources are materialised by an earlier wave, so the pairs inside a wave are
            // independent and derive concurrently.
            let built: Vec<(Perm, Vec<[Id; 3]>)> = wave
                .par_iter()
                .map(|&(src, dst)| (dst, Self::derive_perm(perms[src as usize].as_slice(), src, dst)))
                .collect();
            for (p, v) in built {
                perms[p as usize] = PermData::Owned(v);
            }
        }
        perms
    }

    /// Derives the `dst` permutation from the ALREADY-SORTED `src` permutation's rows: re-map
    /// the columns into `dst`'s layout, then stable-sort by `dst`'s leading column ALONE
    /// ([`radix_sort_rows_by_col0`]) — at most four byte passes over one column instead of the
    /// twelve a from-scratch [`radix_sort_rows`] costs, and no dedup pass (a column permutation
    /// is a bijection on rows, so the deduped SPO multiset stays deduped).
    ///
    /// WHY ONE COLUMN SUFFICES. `src` is in full lexicographic order, so inside any group of
    /// rows sharing a value of `dst`'s leading column the rows are still in `src` order. A
    /// STABLE sort leaves that intra-group order untouched. So the result is fully `dst`-sorted
    /// exactly when `src`'s order, restricted to such a group, already agrees with `dst`'s
    /// remaining two columns — which is precisely the invariant asserted below: deleting
    /// `dst`'s leading column from `src`'s column order leaves `dst`'s trailing columns, in
    /// order. Every pair in [`DERIVE_PLAN`] satisfies it, and
    /// `derive_plan_pairs_are_derivable` pins that for the plan as shipped.
    ///
    /// Output is BYTE-IDENTICAL to sorting the mapped rows from scratch (both are "the deduped
    /// triple set, permuted, fully sorted" — and a fully sorted deduped set is unique), which is
    /// what keeps scan/lookup/estimate, the delta-overlay and save/open untouched. Gated by
    /// `from_triples_perms_match_reference_sort` and `derived_perms_match_radix_all_build`.
    fn derive_perm(src_rows: &[[Id; 3]], src: Perm, dst: Perm) -> Vec<[Id; 3]> {
        let (so, dor) = (src.order(), dst.order());
        debug_assert!(
            so.iter().copied().filter(|&c| c != dor[0]).eq(dor[1..].iter().copied()),
            "{:?} is not derivable from {:?}: deleting the leading column does not leave the rest in order",
            dst,
            src
        );
        // Column j of a dst row is column `pick[j]` of a src row.
        let pick: [usize; 3] =
            std::array::from_fn(|j| so.iter().position(|&c| c == dor[j]).expect("a permutation covers every column"));
        // Pre-sized exactly from the known row count (the mapped iterator is ExactSize, so
        // `collect` reserves `src_rows.len()` up front — no grow tail, no capacity slack).
        let mut v: Vec<[Id; 3]> =
            src_rows.iter().map(|r| [r[pick[0]], r[pick[1]], r[pick[2]]]).collect();
        radix_sort_rows_by_col0(&mut v);
        v
    }

    /// No-threads build of the permutation indexes (wasm / no-rayon path). Deduplicates via
    /// the SPO ordering first (radix — no parallel sort to beat it here), then walks
    /// [`DERIVE_PLAN`] in order, reusing the deduped array for the SPO slot. [SONNET-4.6
    /// sq-7d3dj.32.1] shrink_to_fit after dedup so the SPO slot carries zero capacity slack
    /// (the derived perms are exact by construction). [SONNET-4.6 sq-dzfzq] each derived perm
    /// now costs one single-column sort instead of a full 12-digit one — on wasm, where there
    /// are no threads to hide the work behind, this is the whole saving.
    /// The wasm bundle byte count changes from the historical value — declared in
    /// bench/feature-off-declarations/ and bench/perf-baseline.json feature_off_exact.
    #[cfg(not(feature = "parallel"))]
    fn build_raw_perms(mut triples: Vec<[Id; 3]>) -> [PermData; 6] {
        radix_sort_rows(&mut triples);
        triples.dedup();
        // [SONNET-4.6 sq-7d3dj.32.1] Release the pre-dedup capacity so the SPO slot is
        // exact-sized.  heap_bytes() counts capacity(); without this call it would count
        // the full pre-dedup allocation even after duplicates are removed.
        triples.shrink_to_fit();

        // Place each permutation at its canonical slot; the rest stay empty. Every non-SPO
        // BUILT permutation is DERIVED from an already-materialised one (sq-dzfzq) — with no
        // threads there is nothing to overlap, so the waves just run in order and the saving is
        // pure work: one full 12-digit sort plus one single-column sort per derived perm,
        // instead of a full sort (and a dedup) each.
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        perms[Perm::Spo as usize] = PermData::Owned(triples);
        for &(src, dst) in DERIVE_PLAN.iter().copied().flatten() {
            perms[dst as usize] = PermData::Owned(Self::derive_perm(perms[src as usize].as_slice(), src, dst));
        }
        perms
    }

    /// Persists the permutation indexes to `dir` (one raw little-endian `[u32;3]` file
    /// per permutation) so they can be memory-mapped later via [`open`](Self::open) —
    /// the on-disk side of out-of-core querying.
    #[cfg(feature = "mmap")]
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        self.save_with(dir, false)
    }

    /// Like [`save`](Self::save) but writes each permutation BLOCK-COMPRESSED (the
    /// delta+varint format of [`crate::compress`], ~3-5x smaller on disk). The files are
    /// auto-detected by [`open`](Self::open) via [`crate::compress::FILE_MAGIC`], so old
    /// raw directories keep working and the two formats can be mixed.
    #[cfg(feature = "mmap")]
    pub fn save_compressed(&self, dir: &std::path::Path) -> std::io::Result<()> {
        self.save_with(dir, true)
    }

    #[cfg(feature = "mmap")]
    fn save_with(&self, dir: &std::path::Path, compressed: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for (i, p) in self.perms.iter().enumerate() {
            // Raw modes borrow zero-copy; a compressed perm is decoded back to raw rows so
            // `save` is total (e.g. a `load_str_compressed` graph can still be persisted).
            let rows: std::borrow::Cow<[[Id; 3]]> = match p {
                PermData::Compressed(c) => std::borrow::Cow::Owned(c.decode_all()),
                _ => std::borrow::Cow::Borrowed(p.as_slice()),
            };
            // A pending delta-overlay is FOLDED into every BUILT permutation on save, so
            // the persisted base always reflects the full current state (unbuilt perms
            // stay empty). The in-memory overlay is untouched (`save` takes `&self`).
            let rows: std::borrow::Cow<[[Id; 3]]> = match &self.overlay {
                Some(ov) if BUILT.contains(&Perm::ALL[i]) => {
                    std::borrow::Cow::Owned(ov.merge(&rows, Perm::ALL[i], [Id::MIN; 3], [Id::MAX; 3]))
                }
                _ => rows,
            };
            let path = dir.join(format!("perm{i}.bin"));
            if compressed && !rows.is_empty() {
                // Unbuilt (empty) permutations stay raw-empty so `open` skips them by size.
                // [FABLE-5] sq-7d3dj.32.2.7: `encode_emit` honours the emit-format config gate —
                // `SPQCPRM1` by default, `SPQCPRM2` only when a `spqcprm2` build has opted in.
                let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
                crate::compress::CompressedPerm::encode_emit(&rows).write_to(&mut w)?;
                std::io::Write::flush(&mut w)?;
            } else {
                // SAFETY: reinterpret the contiguous [u32;3] rows as bytes for writing.
                let bytes = unsafe { std::slice::from_raw_parts(rows.as_ptr().cast::<u8>(), std::mem::size_of_val(rows.as_ref())) };
                std::fs::write(path, bytes)?;
            }
        }
        self.save_pred_stats(dir)
    }

    /// Persists per-predicate statistics in ascending predicate-ID order.
    ///
    /// This lets `open` avoid re-scanning the POS/PSO indexes
    /// (a ~2-permutation read — the dominant out-of-core open cost + resident RSS once the
    /// dict is mmap'd). Small: a handful of fields per distinct predicate.
    ///
    /// # Errors
    /// Returns an error if creating, writing, or flushing the statistics file fails.
    #[cfg(feature = "mmap")]
    pub fn save_pred_stats(&self, dir: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(dir.join("predstats.bin"))?);
        w.write_all(&(self.pred_stats.len() as u64).to_le_bytes())?;
        // [GPT-6] Hash-map iteration can change after loading/reserving identical stats.
        // Canonicalize the shared writer so external builds and re-saves agree bytewise.
        let mut entries: Vec<_> = self.pred_stats.iter().collect();
        entries.sort_unstable_by_key(|&(&p, _)| p);
        for (&p, s) in entries {
            w.write_all(&p.to_le_bytes())?;
            w.write_all(&(s.count as u64).to_le_bytes())?;
            w.write_all(&(s.ndv_subj as u64).to_le_bytes())?;
            w.write_all(&(s.ndv_obj as u64).to_le_bytes())?;
        }
        w.flush()
    }

    /// Loads persisted per-predicate stats (written by [`save_pred_stats`]); `None` if the
    /// file is absent (an older saved dir) so the caller falls back to recomputing.
    #[cfg(feature = "mmap")]
    fn load_pred_stats(dir: &std::path::Path) -> Option<FxHashMap<Id, PredStat>> {
        use std::io::Read;
        let mut r = std::io::BufReader::new(std::fs::File::open(dir.join("predstats.bin")).ok()?);
        fn rd8(r: &mut impl Read) -> Option<u64> {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).ok()?;
            Some(u64::from_le_bytes(b))
        }
        // The predicate id is written as a little-endian `Id` (u32, 4 bytes) by
        // `save_pred_stats` — this loader used to read 8 bytes for it, mis-framing every
        // record, so the load ALWAYS failed and `open` silently fell back to recomputing
        // the stats, paging in the whole POS+PSO indexes (~24 B/triple of resident memory
        // and most of the out-of-core open time). Measured in research/memory-tiering.md.
        fn rd_id(r: &mut impl Read) -> Option<Id> {
            let mut b = [0u8; std::mem::size_of::<Id>()];
            r.read_exact(&mut b).ok()?;
            Some(Id::from_le_bytes(b))
        }
        // [OPUS-4.8] sq-f5jh: `predstats.bin` is an UNTRUSTED on-disk file (trust boundary
        // B5). `n` is a u64 count read straight from it, and `reserve(n)` was unbounded — a
        // single flipped count byte could ask `FxHashMap` to pre-allocate billions of slots
        // (~17 B each) and ABORT the process (uncatchable OOM DoS; under llvm-cov's added
        // memory pressure this is the residual rc=101 / coverage-undercount trigger). Each
        // record on disk is `size_of::<Id>() + 24` bytes (id + three u64s), so the file
        // length is a hard upper bound on the real record count: clamp the reservation to it
        // (the per-record `read_exact`s below still error cleanly via `?`/`None` if the file
        // actually ends early). We never reserve for more records than can possibly fit.
        let n = rd8(&mut r)? as usize;
        const PREDSTAT_REC_BYTES: usize = std::mem::size_of::<Id>() + 24; // id + count + ndv_subj + ndv_obj
        let file_len = std::fs::metadata(dir.join("predstats.bin")).ok()?.len() as usize;
        let max_records = file_len.saturating_sub(8) / PREDSTAT_REC_BYTES; // 8-byte header
        let mut stats = FxHashMap::default();
        stats.reserve(n.min(max_records));
        for _ in 0..n {
            let p = rd_id(&mut r)?;
            let count = rd8(&mut r)? as usize;
            let ndv_subj = rd8(&mut r)? as usize;
            let ndv_obj = rd8(&mut r)? as usize;
            stats.insert(p, PredStat { count, ndv_subj, ndv_obj });
        }
        Some(stats)
    }

    /// Opens a store whose permutations are MEMORY-MAPPED from `dir` (written by
    /// [`save`](Self::save)). The 6 (or 3, compact) index files stay on disk; the OS
    /// pages in only the ranges a query touches, so datasets larger than RAM are
    /// queryable. Per-predicate stats are recomputed from the mapped POS/PSO indexes.
    #[cfg(feature = "mmap")]
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self> {
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        for (i, slot) in perms.iter_mut().enumerate() {
            let path = dir.join(format!("perm{i}.bin"));
            let file = std::fs::File::open(&path)?;
            if file.metadata()?.len() == 0 {
                continue; // an empty (unbuilt, e.g. compact-index) permutation
            }
            // SAFETY: the file is owned by this store for its lifetime and is not mutated.
            let map = unsafe { memmap2::Mmap::map(&file)? };
            // FORMAT AUTO-DETECTION: a block-compressed file (written by `save_compressed`)
            // starts with FILE_MAGIC (`SPQCPRM1`) or, for a `spqcprm2`-emitting build,
            // FILE_MAGIC_V2 (`SPQCPRM2`); anything else is the original raw [u32;3] format.
            // [FABLE-5] sq-7d3dj.32.2.7: both magics route to `from_mmap`, which re-checks the
            // magic and picks the V1/V2 decode reader — a V1 file decodes byte-identically
            // forever. Compressed perms are served lazily — block-wise decode off the mapped file.
            *slot = if map.len() >= 8
                && (map[..8] == crate::compress::FILE_MAGIC || map[..8] == crate::compress::FILE_MAGIC_V2)
            {
                PermData::Compressed(crate::compress::CompressedPerm::from_mmap(map)?)
            } else {
                PermData::Mapped(map)
            };
        }
        // Use the persisted stats if present (no POS/PSO re-scan — keeps open fast and the
        // resident set small); else recompute (backward compatible with older saved dirs).
        let pred_stats = Self::load_pred_stats(dir).unwrap_or_else(|| Self::compute_pred_stats(&perms));
        Ok(TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None })
    }

    /// Per-predicate stats: count + distinct objects from POS (always built), and
    /// distinct subjects from PSO when it is built (the full six-permutation index),
    /// else approximated by the count (under `compact-index`, where PSO is absent —
    /// the planner then treats subjects as non-selective, which is safe for ordering).
    fn compute_pred_stats(perms: &[PermData; 6]) -> FxHashMap<Id, PredStat> {
        // Full-range `rows_in`: raw modes borrow the whole slice (zero-copy, as before);
        // a compressed perm (an opened compressed dir missing predstats.bin) is decoded.
        let pos = perms[Perm::Pos as usize].rows_in([Id::MIN; 3], [Id::MAX; 3]); // [P, O, S]
        let pso = perms[Perm::Pso as usize].rows_in([Id::MIN; 3], [Id::MAX; 3]); // [P, S, O], empty under compact-index
        let mut stats: FxHashMap<Id, PredStat> = FxHashMap::default();
        // POS: count + distinct O per P. ndv_subj defaults to count (refined below).
        let mut i = 0;
        while i < pos.len() {
            let p = pos[i][0];
            let (mut count, mut ndv_o, mut last_o) = (0usize, 0usize, None);
            while i < pos.len() && pos[i][0] == p {
                count += 1;
                if last_o != Some(pos[i][1]) {
                    ndv_o += 1;
                    last_o = Some(pos[i][1]);
                }
                i += 1;
            }
            stats.insert(p, PredStat { count, ndv_subj: count, ndv_obj: ndv_o });
        }
        // PSO (when built): exact distinct S per P.
        let mut i = 0;
        while i < pso.len() {
            let p = pso[i][0];
            let (mut ndv_s, mut last_s) = (0usize, None);
            while i < pso.len() && pso[i][0] == p {
                if last_s != Some(pso[i][1]) {
                    ndv_s += 1;
                    last_s = Some(pso[i][1]);
                }
                i += 1;
            }
            stats.entry(p).or_default().ndv_subj = ndv_s;
        }
        stats
    }

    /// Stats for a predicate id (for the cost-based planner), if present.
    pub fn pred_stat(&self, predicate: Id) -> Option<PredStat> {
        self.pred_stats.get(&predicate).copied()
    }

    pub fn len(&self) -> usize {
        let base = self.perms[0].len();
        match &self.overlay {
            Some(ov) => base + ov.added.len() - ov.deleted.len(),
            None => base,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether a delta-overlay of pending updates exists (i.e. updates were applied
    /// since the base was built / last compacted).
    pub fn has_overlay(&self) -> bool {
        self.overlay.is_some()
    }

    /// [OPUS-4.8] (sq-5lf) Strong-reference count of the `Arc`-shared base permutation
    /// indexes — i.e. how many stores currently SHARE this exact base storage. 1 for a
    /// freshly built / just-compacted store; bumps by one for each live
    /// [`fork`](Self::fork) / [`Graph::snapshot`](crate::Graph::snapshot) of it. Used to
    /// PROVE structural sharing in tests (a cheap snapshot bumps this count rather than
    /// duplicating the index memory). Two stores share a base iff this is > 1 and they
    /// were derived from the same lineage.
    pub fn base_strong_count(&self) -> usize {
        std::sync::Arc::strong_count(&self.perms)
    }

    /// Whether the store (base merged with any overlay) contains the canonical triple.
    pub fn contains(&self, t: [Id; 3]) -> bool {
        match &self.overlay {
            Some(ov) => {
                !ov.deleted.contains(&t)
                    && (ov.added.binary_search(&t).is_ok() || self.base_contains(t))
            }
            None => self.base_contains(t),
        }
    }

    /// Whether the immutable BASE (ignoring the overlay) contains the canonical triple —
    /// one binary search of the SPO permutation (always built, in every index set).
    #[inline]
    fn base_contains(&self, t: [Id; 3]) -> bool {
        self.perms[Perm::Spo as usize].count_in(t, t) > 0
    }

    /// Applies an update batch as a DELTA-OVERLAY: `deletes` first, then `inserts`
    /// (SPARQL's DELETE/INSERT order), each O(log n + batch · overlay) — instead of the
    /// O(n) rebuild. Set semantics: re-inserting a present triple and deleting an absent
    /// one are no-ops; a delete of a pending insertion simply retracts it. When the
    /// overlay nets out to nothing it is dropped entirely, so an untouched (or fully
    /// reverted) store scans with zero overhead.
    pub fn apply_delta(&mut self, inserts: &[[Id; 3]], deletes: &[[Id; 3]]) {
        if inserts.is_empty() && deletes.is_empty() {
            return;
        }
        let mut ov = self.overlay.take().unwrap_or_default();
        // `added` is about to change, so every cached perm-sorted projection of it is
        // stale from here on. Dropping them up front (O(1) per permutation) keeps the
        // write path O(batch) — the projections are rebuilt lazily by the next scan
        // that needs them, and only for the permutations it actually scans.
        ov.invalidate_added();
        for t in deletes {
            if let Ok(i) = ov.added.binary_search(t) {
                ov.added.remove(i); // retract a pending insertion
            } else if self.base_contains(*t) {
                ov.deleted.insert(*t);
            }
        }
        for t in inserts {
            if ov.deleted.remove(t) {
                continue; // re-insert of a deleted base triple: just undelete
            }
            if self.base_contains(*t) {
                continue; // already present in the base
            }
            if let Err(i) = ov.added.binary_search(t) {
                ov.added.insert(i, *t);
            }
        }
        self.overlay = if ov.is_empty() { None } else { Some(ov) };
    }

    /// Decodes every block-compressed permutation into its raw in-RAM form, so later
    /// scans are pure binary-search slice borrows (zero decode cost) — the LOAD-TIME
    /// DECOMPRESSION mode for an opened compressed directory: pay one full decode up
    /// front, query at exactly raw-store speed. Raw/mapped permutations are untouched.
    ///
    /// Runs on freshly built/opened stores (load-time), which are never yet forked;
    /// on a structurally SHARED store (post-[`fork`](Self::fork)) it is a no-op —
    /// decompression is an optimisation, never a correctness requirement.
    pub fn decompress_to_ram(&mut self) {
        let Some(perms) = std::sync::Arc::get_mut(&mut self.perms) else {
            return; // shared with a fork: leave the (immutable) base untouched
        };
        for slot in perms {
            if let PermData::Compressed(c) = slot {
                *slot = PermData::Owned(c.decode_all());
            }
        }
    }

    /// A structural FORK of this store: the immutable base permutation indexes and
    /// planner stats are SHARED (Arc bumps, O(1)); the pending delta-overlay is
    /// carried by value (O(overlay), bounded by the compaction policy). The fork and
    /// the original then evolve independently through [`apply_delta`](Self::apply_delta)
    /// — neither ever mutates the shared base, so existing readers are unaffected.
    pub fn fork(&self) -> TripleStore {
        TripleStore {
            perms: std::sync::Arc::clone(&self.perms),
            pred_stats: std::sync::Arc::clone(&self.pred_stats),
            overlay: self.overlay.clone(),
        }
    }

    /// Number of pending overlay entries (insertions + deletions) — the input to a
    /// compaction threshold policy (a fork costs O(this); folding it costs O(n)).
    pub fn overlay_len(&self) -> usize {
        self.overlay.as_ref().map_or(0, |ov| ov.added.len() + ov.deleted.len())
    }

    /// Heap footprint of the permutation indexes in bytes (for benchmarking). Memory-
    /// mapped permutations contribute 0 — their resident pages are OS page cache.
    pub fn heap_bytes(&self) -> usize {
        self.perms.iter().map(PermData::heap_bytes).sum::<usize>()
            + self.overlay.as_ref().map_or(0, |ov| ov.heap_bytes())
    }

    /// Chooses the permutation whose sort order places all bound pattern
    /// positions as a contiguous prefix, so the matches form one range. Returns
    /// the permutation and the number of leading bound columns.
    fn choose(pattern: &Pattern) -> (Perm, usize) {
        // Prefer an order where every bound position precedes every unbound one.
        let bound = |i: usize| pattern[i].is_some();
        for &perm in BUILT {
            let order = perm.order();
            // count leading bound columns
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            // valid if all bound positions are within the leading prefix
            let total_bound = (0..3).filter(|&i| bound(i)).count();
            if lead == total_bound {
                return (perm, lead);
            }
        }
        (Perm::Spo, 0)
    }

    /// Like [`choose`], but among the permutations whose sort order places every
    /// bound position as a prefix, prefers one whose first *unbound* column is
    /// `sort_col` (a position 0..3 into a canonical triple). This makes the scan
    /// output sorted by that column, enabling a merge join on it.
    fn choose_sorted(pattern: &Pattern, sort_col: usize) -> (Perm, usize) {
        let bound = |i: usize| pattern[i].is_some();
        let total_bound = (0..3).filter(|&i| bound(i)).count();
        // Prefer: bound positions form the leading prefix AND column `sort_col`
        // is the first column after the prefix.
        for &perm in BUILT {
            let order = perm.order();
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            if lead == total_bound && lead < 3 && order[lead] == sort_col {
                return (perm, lead);
            }
        }
        Self::choose(pattern)
    }

    /// Returns the contiguous slice of rows (in `perm` order) matching the bound
    /// prefix of the pattern, together with the chosen permutation.
    pub fn scan(&self, pattern: &Pattern) -> Scan<'_> {
        let (perm, lead) = Self::choose(pattern);
        self.scan_with(pattern, perm, lead)
    }

    /// Scans choosing a permutation whose output is sorted by canonical column
    /// `sort_col` (when possible), for merge joins.
    pub fn scan_sorted(&self, pattern: &Pattern, sort_col: usize) -> Scan<'_> {
        let (perm, lead) = Self::choose_sorted(pattern, sort_col);
        self.scan_with(pattern, perm, lead)
    }

    /// [OPUS-4.8] (sq-7d3dj.30.4) Scans a SPECIFIC permutation `perm`, for callers that
    /// need a particular SECONDARY column order rather than just a primary sort column
    /// (e.g. the DISTINCT loose skip-scan wants the layout `[..bound.., P, J, ..]` so each
    /// `P`-block is `J`-sorted). Returns `None` when `perm` is not built (e.g. the compact
    /// index) or when the pattern's bound positions do not form a leading prefix of `perm`
    /// (so a contiguous range scan is impossible). The returned rows are identical to what
    /// `scan`/`scan_sorted` would yield had they chosen `perm` — only the choice differs.
    pub fn scan_perm(&self, pattern: &Pattern, perm: Perm) -> Option<Scan<'_>> {
        if !BUILT.contains(&perm) {
            return None;
        }
        let order = perm.order();
        let bound = |i: usize| pattern[i].is_some();
        let total_bound = (0..3).filter(|&i| bound(i)).count();
        let mut lead = 0;
        while lead < 3 && bound(order[lead]) {
            lead += 1;
        }
        // Every bound position must be within the leading prefix, else this permutation
        // cannot answer the pattern with one contiguous range.
        if lead != total_bound {
            return None;
        }
        Some(self.scan_with(pattern, perm, lead))
    }

    /// The inclusive [lo, hi] key bounds for a pattern's bound prefix in `perm` order.
    #[inline]
    fn bounds(pattern: &Pattern, perm: Perm, lead: usize) -> ([Id; 3], [Id; 3]) {
        let order = perm.order();
        let mut lo = [Id::MIN; 3];
        let mut hi = [Id::MAX; 3];
        for k in 0..lead {
            let v = pattern[order[k]].unwrap();
            lo[k] = v;
            hi[k] = v;
        }
        (lo, hi)
    }

    fn scan_with(&self, pattern: &Pattern, perm: Perm, lead: usize) -> Scan<'_> {
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].rows_in(lo, hi);
        // The single overlay branch on the scan hot path: with no pending updates the
        // base range is returned untouched (borrowed, zero copies); with an overlay the
        // deleted triples are filtered out and the inserted ones merge-interleaved, so
        // the rows keep the permutation's sort order (merge joins stay valid).
        //
        // ZERO-COPY FAST PATH (sq-7d3dj.3) [OPUS-4.8]: even WITH an overlay, most ranges a small
        // overlay does not touch. `count_correction` tells us exactly how many
        // `added`/`deleted` triples fall in this range; when it is `(0, 0)` the
        // overlay contributes nothing here — no `added` row projects into `[lo, hi]` (so
        // nothing is interleaved) and no in-range base row is deleted (so nothing is
        // dropped) — hence `merge` would reproduce `base` verbatim, rows AND sort order.
        // We therefore return the BORROWED base slice directly, restoring allocation-free
        // scans for every untouched range (the read-mostly mutated-server common case)
        // instead of paying the owned merge path — which copies the whole base range into a
        // fresh `Vec` and merge-interleaves the (separately, already perm-sorted) in-range
        // `added` rows. It never re-sorts the range; the cost is the copy plus the interleave.
        let rows = match &self.overlay {
            None => base,
            Some(ov) if ov.count_correction(perm, lo, hi) == (0, 0) => base,
            Some(ov) => std::borrow::Cow::Owned(ov.merge(&base, perm, lo, hi)),
        };
        Scan { rows, perm }
    }

    /// Estimated number of matches for a pattern (the range length) — the cardinality
    /// estimate used by the greedy planner. Cheap for every storage mode: raw modes
    /// subtract binary-search bounds; the compressed mode counts via the block directory
    /// decoding at most two boundary blocks (never the whole range).
    pub fn estimate(&self, pattern: &Pattern) -> usize {
        let (perm, lead) = Self::choose(pattern);
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].count_in(lo, hi);
        match &self.overlay {
            None => base,
            Some(ov) => {
                let (add, del) = ov.count_correction(perm, lo, hi);
                base + add - del
            }
        }
    }
}

/// A range of rows in a permutation's column order. Borrowed from the raw index, or
/// owned when decoded from a compressed permutation — uniformly a `&[[Id;3]]` to callers.
pub struct Scan<'a> {
    pub rows: std::borrow::Cow<'a, [[Id; 3]]>,
    pub perm: Perm,
}

impl<'a> Scan<'a> {
    /// Maps a stored row back to a canonical s,p,o triple.
    #[inline]
    pub fn to_spo(&self, row: &[Id; 3]) -> [Id; 3] {
        let order = self.perm.order();
        let mut out = [0; 3];
        out[order[0]] = row[0];
        out[order[1]] = row[1];
        out[order[2]] = row[2];
        out
    }
}

/// First index where `rows[i] >= key` comparing only the leading columns that
/// are constrained (MIN acts as -inf in unconstrained columns of `key`).
fn lower_bound(rows: &[[Id; 3]], key: &[Id; 3]) -> usize {
    rows.partition_point(|row| row < key)
}

/// First index where `rows[i] > key` (MAX acts as +inf).
fn upper_bound(rows: &[[Id; 3]], key: &[Id; 3]) -> usize {
    rows.partition_point(|row| row <= key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MANDATORY differential-fuzz equivalence gate (sq-7d3dj.17): the O(n) LSD
    /// `radix_sort_rows` used to build every permutation index MUST produce output
    /// BYTE-IDENTICAL to the comparison `sort_unstable()` it replaces, for every input.
    /// The permutations back index range lookups and merge-join order, so any divergence
    /// is a silent correctness bug — this test is the gate that forbids it.
    ///
    /// Covers the degenerate shapes the ingest path can hand the sorter: empty, single,
    /// two (ordered + reversed), all-identical rows, all-same-subject, heavy duplicates,
    /// the `Id::MAX` boundary (top key bytes = `0xFF`, exercising the no-op-pass skip on a
    /// non-constant digit), small-range ids (high bytes constant → passes skipped), and
    /// full 32-bit-range ids (every one of the 12 passes active).
    #[test]
    fn radix_sort_equiv_comparison_sort() {
        // Assert radix output == comparison-sort output on one row set, non-vacuously
        // (the reference is the exact sort the radix path replaces).
        fn check(rows: &[[Id; 3]]) {
            let mut reference = rows.to_vec();
            reference.sort_unstable();
            let mut radixed = rows.to_vec();
            radix_sort_rows(&mut radixed);
            assert_eq!(radixed, reference, "radix diverged from sort_unstable for {rows:?}");
        }

        // Fixed degenerate + boundary shapes.
        check(&[]);
        check(&[[7, 7, 7]]);
        check(&[[1, 2, 3], [1, 2, 3]]); // duplicate pair
        check(&[[2, 0, 0], [1, 0, 0]]); // reversed pair (col-0 tiebreak)
        check(&[[9; 3]; 64]); // all identical
        check(&[[5, 1, 9], [5, 3, 2], [5, 2, 2], [5, 2, 1]]); // all-same-subject
        check(&[
            [Id::MAX, Id::MAX, Id::MAX],
            [0, 0, 0],
            [Id::MAX, 0, Id::MAX],
            [0, Id::MAX, 0],
            [Id::MAX, Id::MAX, 0],
            [1, Id::MAX, Id::MAX],
        ]); // max-id boundary, high bytes 0xFF but non-constant

        // Randomized sets across several id-range regimes (some keep high key bytes
        // constant → passes skipped; the full-range regime activates all 12 passes).
        let mut st = 0x9E3779B9u32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        for regime in 0..4 {
            for _ in 0..40 {
                let n = (rng() % 3000) as usize;
                let mut rows: Vec<[Id; 3]> = Vec::with_capacity(n);
                for _ in 0..n {
                    // Three raw draws, then shape per regime: tiny (byte 0 only), two low
                    // bytes, ragged per-column widths, or the full 32-bit range.
                    let (a, b, c) = (rng(), rng(), rng());
                    let cell = |raw: u32, bits: u32| match regime {
                        0 => 1 + raw % 16,
                        1 => 1 + raw % 60_000,
                        2 => raw & ((1u32 << bits) - 1).max(1),
                        _ => raw,
                    };
                    rows.push([cell(a, 20), cell(b, 12), cell(c, 28)]);
                }
                // Salt in exact duplicates + a boundary row so ties + max-id are exercised.
                if !rows.is_empty() {
                    rows.push(rows[rows.len() / 2]);
                    rows.push([Id::MAX, Id::MAX, Id::MAX]);
                    rows.push([0, 0, 0]);
                }
                check(&rows);
            }
        }
    }

    /// End-to-end build equivalence: each BUILT permutation of a radix-built store must
    /// equal the reference the comparison sort would produce — deduped SPO triples,
    /// column-permuted, `sort_unstable()`ed. Proves the wiring in `build_raw_perms`, not
    /// just the sort primitive, and is non-vacuous (the reference is computed independently
    /// of the store). Runs across both index sets (`compact-index` builds three perms).
    #[test]
    fn from_triples_perms_match_reference_sort() {
        let mut st = 0xC0FFEEu32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        for _ in 0..8 {
            let n = 500 + (rng() % 4000) as usize;
            let mut triples: Vec<[Id; 3]> = Vec::with_capacity(n);
            for _ in 0..n {
                triples.push([1 + rng() % 300, 1 + rng() % 20, 1 + rng() % 1500]);
            }
            // Reference deduped SPO set (independent of the store under test).
            let mut deduped = triples.clone();
            deduped.sort_unstable();
            deduped.dedup();

            let store = TripleStore::from_triples(triples);
            for &perm in BUILT {
                let order = perm.order();
                let mut want: Vec<[Id; 3]> = deduped
                    .iter()
                    .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
                    .collect();
                want.sort_unstable();
                let got = store.perms[perm as usize].as_slice();
                assert_eq!(got, &want[..], "permutation {perm:?} differs from the comparison-sort reference");
            }
        }
    }

    /// [SONNET-4.6 sq-dzfzq] The reference build the DERIVED build replaced: every BUILT
    /// permutation mapped from the raw input and sorted+deduped INDEPENDENTLY with the full
    /// 12-digit radix (the sq-7d3dj.31 concurrent-N body). Kept test-only as the differential
    /// oracle for `derived_perms_match_radix_all_build` and as the "A" arm of
    /// `measure_derived_vs_radix_all_build`.
    fn build_raw_perms_radix_all(triples: &[[Id; 3]]) -> Vec<(Perm, Vec<[Id; 3]>)> {
        let build = |p: Perm| {
            let order = p.order();
            let mut v: Vec<[Id; 3]> =
                triples.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
            radix_sort_rows(&mut v);
            v.dedup();
            v.shrink_to_fit();
            (p, v)
        };
        // Mirrors the replaced body's OWN concurrency, so the A/B compares schedules and not
        // merely work: rayon when `parallel` is on, sequential otherwise.
        #[cfg(feature = "parallel")]
        {
            BUILT.par_iter().map(|&p| build(p)).collect()
        }
        #[cfg(not(feature = "parallel"))]
        {
            BUILT.iter().map(|&p| build(p)).collect()
        }
    }

    /// [SONNET-4.6 sq-dzfzq] The derivability invariant `derive_perm` rests on, checked for the
    /// plan AS SHIPPED: for every `(src, dst)` pair, deleting `dst`'s LEADING column from
    /// `src`'s column order must leave `dst`'s two trailing columns in that order — that is
    /// exactly the condition under which a stable sort of the re-mapped rows by `dst`'s leading
    /// column alone lands in full `dst` lexicographic order.
    ///
    /// Also pins the two properties that make the plan a valid *schedule*: every source is
    /// already materialised when its wave runs (SPO, or a destination of an earlier wave), and
    /// the plan produces each non-SPO BUILT permutation exactly once and nothing else.
    /// Non-vacuous: swap either column of a pair and the first assertion fails.
    #[test]
    fn derive_plan_pairs_are_derivable() {
        let mut have: Vec<Perm> = vec![Perm::Spo];
        for wave in DERIVE_PLAN {
            for &(src, dst) in wave.iter() {
                assert!(have.contains(&src), "{src:?} is not materialised before it is used to derive {dst:?}");
                let (so, dor) = (src.order(), dst.order());
                let rest: Vec<usize> = so.iter().copied().filter(|&c| c != dor[0]).collect();
                assert_eq!(rest, dor[1..], "{dst:?} is NOT derivable from {src:?} by a stable sort on its leading column");
            }
            // Destinations only become available to the NEXT wave.
            for &(_, dst) in wave.iter() {
                assert!(!have.contains(&dst), "{dst:?} is derived twice");
                have.push(dst);
            }
        }
        have.sort_unstable_by_key(|p| *p as usize);
        let mut want: Vec<Perm> = BUILT.to_vec();
        want.sort_unstable_by_key(|p| *p as usize);
        assert_eq!(have, want, "the derivation plan must produce exactly the BUILT set");
    }

    /// [SONNET-4.6 sq-dzfzq] RESULT-EQUIVALENCE of the derived build: every BUILT permutation of
    /// a derived-built store must be BYTE-IDENTICAL to the independent full-radix sort+dedup of
    /// the same input (`build_raw_perms_radix_all` — the body the derived build replaced).
    /// Complements `from_triples_perms_match_reference_sort` (which compares against
    /// `sort_unstable`): this one pins the specific *replacement* claim.
    ///
    /// The corpora deliberately span the shapes the single-column stable sort could get wrong:
    /// a constant leading column (no radix pass runs at all), a leading column that spans more
    /// than one byte (multiple passes, so stability across passes matters), heavy duplicates,
    /// and the id-space boundaries.
    #[test]
    fn derived_perms_match_radix_all_build() {
        let mut st = 0x5EEDBEEFu32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        // (subject span, predicate span, object span) — 1 means a CONSTANT column.
        let shapes: [(u32, u32, u32); 6] = [
            (1, 1, 1),          // every column constant: no radix pass executes anywhere
            (1, 7, 100_000),    // constant subject, wide multi-byte object
            (300, 1, 300),      // constant predicate
            (100_000, 5, 3),    // wide subject, tiny object -> heavy duplicates
            (40, 40, 40),       // all sub-byte: exactly one pass per derivation
            (3_000_000, 900, 5_000_000), // all multi-byte
        ];
        for (si, pi, oi) in shapes {
            for n in [0usize, 1, 2, 997, 6_000] {
                let mut triples: Vec<[Id; 3]> = (0..n)
                    .map(|_| [1 + rng() % si, 1 + rng() % pi, 1 + rng() % oi])
                    .collect();
                if n > 2 {
                    // Exact duplicates and both id-space boundaries.
                    triples.push(triples[n / 2]);
                    triples.push([Id::MAX, Id::MAX, Id::MAX]);
                    triples.push([0, 0, 0]);
                    triples.push([Id::MAX, 0, Id::MAX]);
                }
                let want = build_raw_perms_radix_all(&triples);
                let store = TripleStore::from_triples(triples);
                for (perm, want_rows) in want {
                    assert_eq!(
                        store.perms[perm as usize].as_slice(),
                        &want_rows[..],
                        "derived {perm:?} differs from the independent full-radix build (shape {si}/{pi}/{oi}, n={n})"
                    );
                }
            }
        }
    }

    /// [SONNET-4.6 sq-dzfzq] NON-CANONICAL work-box A/B for the derived permutation build —
    /// the measure-first half of the bead. `#[ignore]`d: it times, so it must never run in the
    /// normal test lane, and its numbers belong to whatever box ran it. Run explicitly:
    ///
    /// ```text
    /// cargo test -p sparq-core --release measure_derived_vs_radix_all_build -- --ignored --nocapture
    /// ```
    ///
    /// The interesting axis is THREADS vs PERMUTATIONS (see `build_raw_perms`), so sweep it:
    /// vary `RAYON_NUM_THREADS`, and add `--no-default-features` (no threads) and/or
    /// `--features compact-index` (three permutations) to move the other end.
    ///
    /// Arm A is `build_raw_perms_radix_all` — the sq-7d3dj.31 body INCLUDING its own rayon
    /// fan-out, so this is a schedule-level comparison, not just a work-count one. Arm B is the
    /// shipped [`TripleStore::build_raw_perms`]. The arms are interleaved and repeated so slow
    /// drift cancels, and the input clone arm B consumes is taken OUTSIDE its timer (arm A
    /// clones per permutation inside its own). No number is asserted — the gate is
    /// `derived_perms_match_radix_all_build`; this only reports.
    #[test]
    #[ignore = "timing measurement; non-canonical, run explicitly"]
    fn measure_derived_vs_radix_all_build() {
        use std::time::{Duration, Instant};

        // Two scales, on a shape with realistic RDF skew (few predicates, many objects).
        for n in [200_000usize, 2_000_000] {
            let mut st = 0xA5A5_1234u32;
            let mut rng = || {
                st ^= st << 13;
                st ^= st >> 17;
                st ^= st << 5;
                st
            };
            let triples: Vec<[Id; 3]> = (0..n)
                .map(|_| [1 + rng() % (n as u32 / 8).max(2), 1 + rng() % 60, 1 + rng() % (n as u32 / 2).max(2)])
                .collect();

            let (mut a, mut b) = (Duration::ZERO, Duration::ZERO);
            for _ in 0..3 {
                let t = Instant::now();
                let ra = build_raw_perms_radix_all(&triples);
                a += t.elapsed();

                let input = triples.clone();
                let t = Instant::now();
                let got = TripleStore::build_raw_perms(input);
                b += t.elapsed();

                // Keep both arms alive across the timing loop AND re-check equivalence here, so
                // the measurement can never report a speedup for a wrong answer.
                for (perm, want) in &ra {
                    assert_eq!(got[*perm as usize].as_slice(), &want[..], "{perm:?} differs between the two arms");
                }
            }
            println!(
                "sq-dzfzq build A/B (NON-canonical work-box) n={}: radix-all {:?} vs derived {:?} -> {:.2}x",
                n,
                a,
                b,
                a.as_secs_f64() / b.as_secs_f64()
            );
        }
    }

    /// [OPUS-4.8] (sq-7d3dj.30.4) `scan_perm` returns the rows for a SPECIFIC built
    /// permutation (with the pattern's bound positions as a leading prefix), and `None`
    /// when the permutation is not built or the bound positions are not a prefix.
    #[test]
    fn scan_perm_selects_named_permutation() {
        let triples: Vec<[Id; 3]> = vec![
            [10, 1, 100],
            [10, 2, 100],
            [11, 1, 100],
            [12, 1, 101],
        ];
        let store = TripleStore::from_triples(triples.clone());

        // Fully-unbound: a built predicate-first permutation must be selectable, and its rows
        // sorted in that permutation's column order. Which predicate-first perm is BUILT
        // differs by index set: the default 6-perm build has PSO; the 3-perm `compact-index`
        // build ({SPO, POS, OSP}) has POS instead — where `scan_perm(Perm::Pso)` correctly
        // returns `None` because PSO is not built (the documented contract). [OPUS-4.8]
        #[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
        {
            let scan = store.scan_perm(&[None, None, None], Perm::Pso).expect("PSO is built");
            assert_eq!(scan.perm, Perm::Pso);
            let spo: Vec<[Id; 3]> = scan.rows.iter().map(|r| scan.to_spo(r)).collect();
            let mut want = triples.clone();
            want.sort_by_key(|t| (t[1], t[0], t[2])); // predicate, subject, object
            assert_eq!(spo, want, "PSO scan not sorted by (predicate, subject, object)");
        }
        #[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
        {
            // PSO is NOT built under compact-index → `scan_perm` declines it (the contract).
            assert!(
                store.scan_perm(&[None, None, None], Perm::Pso).is_none(),
                "PSO is not built under compact-index; scan_perm must return None"
            );
            // POS IS built there and serves the fully-unbound pattern, sorted (predicate, object, subject).
            let scan = store.scan_perm(&[None, None, None], Perm::Pos).expect("POS is built");
            assert_eq!(scan.perm, Perm::Pos);
            let spo: Vec<[Id; 3]> = scan.rows.iter().map(|r| scan.to_spo(r)).collect();
            let mut want = triples.clone();
            want.sort_by_key(|t| (t[1], t[2], t[0])); // predicate, object, subject
            assert_eq!(spo, want, "POS scan not sorted by (predicate, object, subject)");
        }

        // Object bound: OSP places the object as the leading prefix → Some. (OSP is built in
        // both the default and the compact index set.)
        let obj = store.scan_perm(&[None, None, Some(100)], Perm::Osp).expect("OSP built");
        assert!(obj.rows.iter().all(|r| obj.to_spo(r)[2] == 100), "OSP range must be object=100");
        assert_eq!(obj.rows.len(), 3);

        // Object bound but SPO does NOT put the bound object in the leading prefix → None.
        assert!(
            store.scan_perm(&[None, None, Some(100)], Perm::Spo).is_none(),
            "SPO cannot serve an object-only bound pattern as a prefix range"
        );
    }

    /// [OPUS-4.8 sq-7d3dj.31] Direct gate on the concurrent-N build's INDEPENDENT per-perm
    /// dedup: with the parallel build, SPO is no longer deduped-first-then-mapped — every perm
    /// dedups its own sorted copy. On a heavily-duplicated input (the same handful of triples
    /// repeated thousands of times) each BUILT perm must still hold EXACTLY the distinct set,
    /// sorted, with no residual duplicate row — proving independent dedup equals the SPO-first
    /// dedup it replaced. Non-vacuous: the expected length is the true distinct count.
    #[test]
    fn from_triples_independent_dedup_removes_all_duplicates() {
        let base: [[Id; 3]; 5] = [[3, 1, 9], [1, 2, 2], [1, 2, 8], [7, 4, 4], [3, 1, 5]];
        let mut triples: Vec<[Id; 3]> = Vec::new();
        // Repeat the whole set 4000× (20 000 rows, only 5 distinct) plus one lone triple.
        for _ in 0..4000 {
            triples.extend_from_slice(&base);
        }
        triples.push([9, 9, 9]);
        let distinct = 6; // the 5 in `base` plus [9,9,9]

        let store = TripleStore::from_triples(triples);
        for &perm in BUILT {
            let rows = store.perms[perm as usize].as_slice();
            assert_eq!(rows.len(), distinct, "permutation {perm:?} still holds duplicate rows");
            // Fully sorted in this perm's column order, and strictly increasing (no dup).
            assert!(rows.windows(2).all(|w| w[0] < w[1]), "permutation {perm:?} not strictly sorted/deduped");
        }
    }

    /// The compressed store must answer EVERY triple pattern with the exact same rows
    /// (and the same `estimate`) as the raw store — across all bound/unbound shapes and
    /// many key ranges, including misses and the boundaries of the id space.
    #[test]
    fn compressed_scans_match_raw() {
        // A few-predicate, clustered-subject, sparse-object graph spanning many blocks.
        let mut triples: Vec<[Id; 3]> = Vec::new();
        let mut st = 0x12345677u32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        for _ in 0..40_000 {
            triples.push([1 + rng() % 800, 1 + rng() % 12, 1 + rng() % 5000]);
        }
        let raw = TripleStore::from_triples(triples.clone());
        let cmp = TripleStore::from_triples_compressed(triples);
        assert_eq!(raw.len(), cmp.len());

        let dump = |s: &Scan| {
            let mut v: Vec<[Id; 3]> = s.rows.iter().map(|r| s.to_spo(r)).collect();
            v.sort_unstable();
            v
        };
        // Probe values: present ids, absent ids, and the id-space boundaries.
        let svals = [None, Some(1), Some(400), Some(801), Some(Id::MAX)];
        let pvals = [None, Some(1), Some(6), Some(13)];
        let ovals = [None, Some(1), Some(2500), Some(5001)];
        for &s in &svals {
            for &p in &pvals {
                for &o in &ovals {
                    let pat: Pattern = [s, p, o];
                    let rs = raw.scan(&pat);
                    let cs = cmp.scan(&pat);
                    assert_eq!(dump(&rs), dump(&cs), "rows differ for pattern {pat:?}");
                    // estimate() must equal the true match count for both modes.
                    assert_eq!(cmp.estimate(&pat), dump(&rs).len(), "estimate wrong for {pat:?}");
                }
            }
        }
        // Per-predicate stats must be identical (computed from raw before encoding).
        for p in 1..=13 {
            assert_eq!(raw.pred_stat(p), cmp.pred_stat(p), "pred_stat differs for {p}");
        }
    }

    /// [FABLE-5] sq-559dp — the IN-RAM compressed profile honours the SPQCPRM2 emit gate.
    /// `from_triples_compressed` now encodes through `CompressedPerm::encode_emit` (the same
    /// one-place gate `save_compressed` uses), so `SPARQ_STORE_PROFILE=compressed` can opt into
    /// the frame-of-reference col2 reset instead of being pinned to V1. Asserts three things:
    ///
    /// 1. DEFAULT (no override) still builds `V1` perms — the byte-identity invariant; and
    /// 2. under `with_emit_format(V2)` every non-empty perm is `V2` — the gate actually reaches
    ///    the in-RAM path; and
    /// 3. on a corpus whose objects cluster inside a block (large absolute ids, small in-block
    ///    spread — the `reset_d1` shape the FoR encoding targets) the V2 store's resident bytes
    ///    are STRICTLY lower, i.e. the acceptance "lower comp_store_bpt" direction holds. This
    ///    is a SHAPE assertion on a designed corpus, not a canonical benchmark number: V2 is not
    ///    universally smaller (a far-from-frame reset costs zigzag bytes), which is exactly why
    ///    the emit gate stays opt-in.
    ///
    /// Both formats must also answer every pattern exactly as the raw store does.
    #[cfg(feature = "spqcprm2")]
    #[test]
    fn in_ram_compressed_honours_v2_emit_gate() {
        use crate::compress::{with_emit_format, EmitFormat};

        // Clustered objects: 3-byte absolute ids (>= 2^21) with a small in-block spread, each
        // subject carrying several predicates so most rows hit the `reset_d1` (col2 absolute in
        // V1 / frame-delta in V2) branch of the block encoder.
        let mut triples: Vec<[Id; 3]> = Vec::new();
        for s in 1..600u32 {
            for p in 1..9u32 {
                triples.push([s, p, 4_000_000 + s * 8 + p]);
            }
        }
        let raw = TripleStore::from_triples(triples.clone());
        let v1 = TripleStore::from_triples_compressed(triples.clone());
        let v2 = with_emit_format(EmitFormat::V2, || TripleStore::from_triples_compressed(triples));

        let mut checked = 0;
        for i in 0..6 {
            if let (PermData::Compressed(a), PermData::Compressed(b)) = (&v1.perms[i], &v2.perms[i]) {
                assert_eq!(a.format(), crate::compress::Format::V1, "default in-RAM emit must stay V1 (perm {})", i);
                assert_eq!(b.format(), crate::compress::Format::V2, "emit gate did not reach the in-RAM path (perm {})", i);
                checked += 1;
            }
        }
        assert!(checked > 0, "no compressed permutations were built");

        // Same answers in both formats, for every bound/unbound shape.
        let dump = |s: &Scan| {
            let mut v: Vec<[Id; 3]> = s.rows.iter().map(|r| s.to_spo(r)).collect();
            v.sort_unstable();
            v
        };
        for &s in &[None, Some(1), Some(300), Some(601)] {
            for &p in &[None, Some(1), Some(8), Some(9)] {
                for &o in &[None, Some(4_000_009), Some(4_002_400), Some(7)] {
                    let pat: Pattern = [s, p, o];
                    let want = dump(&raw.scan(&pat));
                    assert_eq!(dump(&v1.scan(&pat)), want, "V1 rows differ for {:?}", pat);
                    assert_eq!(dump(&v2.scan(&pat)), want, "V2 rows differ for {:?}", pat);
                    assert_eq!(v2.estimate(&pat), want.len(), "V2 estimate wrong for {:?}", pat);
                }
            }
        }

        let bytes = |s: &TripleStore| s.perms.iter().map(PermData::heap_bytes).sum::<usize>();
        assert!(
            bytes(&v2) < bytes(&v1),
            "V2 in-RAM store must be smaller on a clustered-object corpus: v2={} v1={}",
            bytes(&v2),
            bytes(&v1)
        );
    }
    /// The COMPRESSED on-disk format must round-trip exactly: `save_compressed` → `open`
    /// must answer every triple pattern with the same rows, in the same scan order, with
    /// the same `estimate`, as the raw `save` → `open` of the same store — whether served
    /// lazily (block-wise off the mapped file) or after `decompress_to_ram`. Also checks
    /// FORMAT AUTO-DETECTION (magic header present iff compressed; both dirs open through
    /// the same `open`), the old-format compat (the raw dir keeps working untouched), and
    /// that the compressed files are genuinely smaller.
    #[cfg(feature = "mmap")]
    #[test]
    fn compressed_save_open_roundtrip() {
        let mut triples: Vec<[Id; 3]> = Vec::new();
        let mut st = 0xBEEF5EEDu32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        for _ in 0..60_000 {
            triples.push([1 + rng() % 1500, 1 + rng() % 17, 1 + rng() % 9000]);
        }
        let store = TripleStore::from_triples(triples);

        let base = std::env::temp_dir().join(format!("sparq_cperm_{}", std::process::id()));
        let raw_dir = base.join("raw");
        let cmp_dir = base.join("cmp");
        store.save(&raw_dir).unwrap();
        store.save_compressed(&cmp_dir).unwrap();

        // Auto-detection bytes: compressed BUILT perms carry FILE_MAGIC and are smaller;
        // raw files never start with the magic.
        let mut raw_total = 0u64;
        let mut cmp_total = 0u64;
        for &perm in BUILT {
            let f = format!("perm{}.bin", perm as usize);
            let r = std::fs::read(raw_dir.join(&f)).unwrap();
            let c = std::fs::read(cmp_dir.join(&f)).unwrap();
            assert_ne!(&r[..8], &crate::compress::FILE_MAGIC, "raw {f} must not carry the magic");
            assert_eq!(&c[..8], &crate::compress::FILE_MAGIC, "compressed {f} must carry the magic");
            assert!(c.len() < r.len(), "{f}: compressed ({}) not smaller than raw ({})", c.len(), r.len());
            raw_total += r.len() as u64;
            cmp_total += c.len() as u64;
        }
        assert!(cmp_total * 2 < raw_total, "expected >2x overall perm compression, got {raw_total}/{cmp_total}");

        let raw = TripleStore::open(&raw_dir).unwrap(); // old format: still opens (compat)
        let lazy = TripleStore::open(&cmp_dir).unwrap(); // auto-detected compressed
        let mut eager = TripleStore::open(&cmp_dir).unwrap();
        eager.decompress_to_ram(); // load-time decompression mode
        assert!(matches!(lazy.perms[Perm::Spo as usize], PermData::Compressed(_)), "compressed file not auto-detected");
        assert!(matches!(raw.perms[Perm::Spo as usize], PermData::Mapped(_)), "raw file must stay mmap'd");
        assert!(matches!(eager.perms[Perm::Spo as usize], PermData::Owned(_)), "decompress_to_ram must own the rows");
        assert_eq!(raw.len(), lazy.len());
        assert_eq!(raw.len(), eager.len());

        // Every pattern shape (and the merge-join sorted variants) must yield identical
        // rows in identical order, and identical estimates, across the three stores.
        let svals = [None, Some(1), Some(700), Some(1501)];
        let pvals = [None, Some(1), Some(9), Some(18)];
        let ovals = [None, Some(1), Some(4500), Some(9001)];
        for &s in &svals {
            for &p in &pvals {
                for &o in &ovals {
                    let pat: Pattern = [s, p, o];
                    for sort_col in [None, Some(0), Some(1), Some(2)] {
                        fn scans<'a>(g: &'a TripleStore, pat: &Pattern, sort_col: Option<usize>) -> Scan<'a> {
                            match sort_col {
                                None => g.scan(pat),
                                Some(c) => g.scan_sorted(pat, c),
                            }
                        }
                        let (r, l, e) = (scans(&raw, &pat, sort_col), scans(&lazy, &pat, sort_col), scans(&eager, &pat, sort_col));
                        assert_eq!(r.perm, l.perm);
                        assert_eq!(r.rows, l.rows, "lazy rows differ for {pat:?} sort {sort_col:?}");
                        assert_eq!(r.rows, e.rows, "eager rows differ for {pat:?} sort {sort_col:?}");
                    }
                    assert_eq!(raw.estimate(&pat), lazy.estimate(&pat), "estimate differs for {pat:?}");
                }
            }
        }
        // Pred stats: persisted identically; and recomputable from compressed perms when
        // predstats.bin is missing (the fallback decodes, it must not panic or differ).
        std::fs::remove_file(cmp_dir.join("predstats.bin")).unwrap();
        let refallback = TripleStore::open(&cmp_dir).unwrap();
        for p in 1..=18 {
            assert_eq!(raw.pred_stat(p), lazy.pred_stat(p), "persisted pred_stat differs for {p}");
            assert_eq!(raw.pred_stat(p), refallback.pred_stat(p), "recomputed pred_stat differs for {p}");
        }
        std::fs::remove_dir_all(&base).ok();
    }

    /// `load_pred_stats` must actually LOAD the persisted file — not silently return
    /// `None` and let `open` fall back to recomputing (which pages in the whole POS+PSO
    /// indexes, defeating the point of persisting the stats). Regression test for the
    /// id-width mis-framing bug: the predicate id is written as a 4-byte `Id`, and the
    /// loader read 8 — so every load failed and `compressed_save_open_roundtrip` above
    /// still passed (recomputed == recomputed). Asserts the load is `Some` AND exact.
    #[cfg(feature = "mmap")]
    #[test]
    fn pred_stats_load_is_some_and_exact() {
        let mut triples: Vec<[Id; 3]> = Vec::new();
        let mut st = 0xACCE55u32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        for _ in 0..30_000 {
            triples.push([1 + rng() % 900, 1 + rng() % 23, 1 + rng() % 4000]);
        }
        let store = TripleStore::from_triples(triples);
        let dir = std::env::temp_dir().join(format!("sparq_predstats_{}", std::process::id()));
        store.save(&dir).unwrap();
        let loaded = TripleStore::load_pred_stats(&dir)
            .expect("persisted predstats.bin must load, not fall back to a POS+PSO re-scan");
        assert_eq!(loaded, *store.pred_stats, "loaded pred stats must equal the saved ones");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [GPT-6] Persisted bytes depend on statistics, not hash-map layout or reloads.
    #[cfg(feature = "mmap")]
    #[test]
    fn pred_stats_serialization_is_canonical_across_map_layouts() {
        // The same records had different iteration orders after external-build reload.
        let records: [(Id, PredStat); 4] = [
            (1, PredStat { count: 400, ndv_subj: 400, ndv_obj: 140 }),
            (2, PredStat { count: 400, ndv_subj: 400, ndv_obj: 400 }),
            (182, PredStat { count: 400, ndv_subj: 400, ndv_obj: 400 }),
            (424, PredStat { count: 400, ndv_subj: 400, ndv_obj: 90 }),
        ];
        // Independent format oracle: u64 record count, then u32 ID + three u64 values.
        let mut expected = 4u64.to_le_bytes().to_vec();
        for (id, stats) in &records {
            expected.extend_from_slice(&id.to_le_bytes());
            for value in [stats.count, stats.ndv_subj, stats.ndv_obj] {
                expected.extend_from_slice(&(value as u64).to_le_bytes());
            }
        }
        let dir = std::env::temp_dir().join(format!("sparq_predstats_order_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("predstats.bin");
        for capacity in [0, 4, 64, 1024] {
            for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
                let mut stats = FxHashMap::default();
                stats.reserve(capacity);
                for i in order {
                    stats.insert(records[i].0, records[i].1);
                }
                let mut store = TripleStore::from_triples(Vec::new());
                store.pred_stats = std::sync::Arc::new(stats);
                store.save_pred_stats(&dir).unwrap();
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    expected,
                    "capacity {capacity}, order {order:?}"
                );
                let loaded = TripleStore::load_pred_stats(&dir).unwrap();
                assert_eq!(loaded, *store.pred_stats);
                store.pred_stats = std::sync::Arc::new(loaded);
                store.save_pred_stats(&dir).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), expected, "reload must preserve exact bytes");
            }
        }
        // Existing files need not already be ordered: read compatibility is unchanged.
        let mut legacy = expected[..8].to_vec();
        for record in expected[8..].chunks_exact(28).rev() {
            legacy.extend_from_slice(record);
        }
        std::fs::write(&path, legacy).unwrap();
        let loaded = TripleStore::load_pred_stats(&dir).unwrap();
        assert_eq!(loaded, records.into_iter().collect::<FxHashMap<_, _>>());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The delta-overlay must be INVISIBLE in results: a store with an overlay must answer
    /// every triple pattern with exactly the rows of a store REBUILT from the merged triple
    /// set — and the rows must stay sorted in the scan's permutation order (the guarantee
    /// merge joins rely on). Also: `estimate` stays exact, `len`/`contains` agree, and an
    /// overlay that nets out to nothing is dropped (zero-overhead empty case).
    #[test]
    fn overlay_scans_match_rebuild() {
        let mut st = 0xC0FFEEu32;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            st
        };
        let mut triples: Vec<[Id; 3]> = Vec::new();
        for _ in 0..20_000 {
            triples.push([1 + rng() % 400, 1 + rng() % 9, 1 + rng() % 2000]);
        }
        let mut store = TripleStore::from_triples(triples.clone());

        // Deletes: a sample of base triples (plus an absent one — must be a no-op).
        // Inserts: fresh triples (plus a base duplicate — must be a no-op).
        triples.sort_unstable();
        triples.dedup();
        let deletes: Vec<[Id; 3]> = triples.iter().step_by(7).copied().chain([[9999, 9999, 9999]]).collect();
        let mut inserts: Vec<[Id; 3]> = (0..500).map(|_| [401 + rng() % 50, 10 + rng() % 3, 2001 + rng() % 100]).collect();
        inserts.push(triples[3]); // already in the base
        store.apply_delta(&inserts, &deletes);
        assert!(store.has_overlay());

        // Reference: the same set operations applied eagerly, then a full rebuild.
        let mut reference: Vec<[Id; 3]> = triples.clone();
        let del: std::collections::HashSet<[Id; 3]> = deletes.iter().copied().collect();
        reference.retain(|t| !del.contains(t));
        reference.extend(inserts.iter().copied().filter(|t| !del.contains(t)));
        reference.sort_unstable();
        reference.dedup();
        let rebuilt = TripleStore::from_triples(reference.clone());

        assert_eq!(store.len(), rebuilt.len(), "len must reflect the overlay");
        for t in &reference {
            assert!(store.contains(*t));
        }
        assert!(!store.contains([9999, 9999, 9999]));

        // Every pattern x sort-column, over every permutation the planner can pick, must
        // agree with the rebuild — rows AND sort order. Reused below to re-check the SAME
        // store after a SECOND delta, which is what pins cache invalidation (sq-7d3dj.16).
        let sweep = |store: &TripleStore, rebuilt: &TripleStore, label: &str| {
            let svals = [None, Some(1), Some(200), Some(420), Some(9999)];
            let pvals = [None, Some(1), Some(5), Some(11)];
            let ovals = [None, Some(2), Some(1500), Some(2050)];
            for &s in &svals {
                for &p in &pvals {
                    for &o in &ovals {
                        let pat: Pattern = [s, p, o];
                        for sort_col in [None, Some(0), Some(1), Some(2)] {
                            let (ov_scan, rb_scan) = match sort_col {
                                None => (store.scan(&pat), rebuilt.scan(&pat)),
                                Some(c) => (store.scan_sorted(&pat, c), rebuilt.scan_sorted(&pat, c)),
                            };
                            // Rows must be SORTED in the chosen permutation's order…
                            assert!(
                                ov_scan.rows.windows(2).all(|w| w[0] <= w[1]),
                                "unsorted overlay scan for {pat:?} ({label})"
                            );
                            // …and identical (same perm choice — `choose` is pattern-only) to the rebuild.
                            assert_eq!(ov_scan.perm, rb_scan.perm);
                            assert_eq!(
                                ov_scan.rows, rb_scan.rows,
                                "rows differ for {pat:?} sort {sort_col:?} ({label})"
                            );
                        }
                        assert_eq!(
                            store.estimate(&pat),
                            rebuilt.scan(&pat).rows.len(),
                            "estimate wrong for {pat:?} ({label})"
                        );
                    }
                }
            }
        };
        sweep(&store, &rebuilt, "first delta");

        // sq-7d3dj.16: the sweep above has now materialised a cached perm-sorted projection
        // of `added` for EVERY permutation it scanned. A SECOND delta must invalidate all of
        // them — it both GROWS `added` (fresh insertions) and SHRINKS it (a delete of a
        // pending insertion retracts it, the `added.remove` path), so a stale cache would
        // surface retracted rows and miss new ones. Re-running the identical sweep against a
        // fresh rebuild of the twice-updated set is what catches that.
        let more_inserts: Vec<[Id; 3]> = (0..300).map(|_| [451 + rng() % 50, 13 + rng() % 3, 2101 + rng() % 100]).collect();
        // Retract a sample of the FIRST batch's pending insertions, and delete more base triples.
        let pending: Vec<[Id; 3]> = inserts.iter().copied().filter(|t| triples.binary_search(t).is_err()).collect();
        let more_deletes: Vec<[Id; 3]> =
            pending.iter().step_by(3).copied().chain(triples.iter().skip(1).step_by(11).copied()).collect();
        store.apply_delta(&more_inserts, &more_deletes);

        let mut reference2: Vec<[Id; 3]> = reference.clone();
        let del2: std::collections::HashSet<[Id; 3]> = more_deletes.iter().copied().collect();
        reference2.retain(|t| !del2.contains(t));
        reference2.extend(more_inserts.iter().copied().filter(|t| !del2.contains(t)));
        reference2.sort_unstable();
        reference2.dedup();
        let rebuilt2 = TripleStore::from_triples(reference2);
        assert_eq!(store.len(), rebuilt2.len(), "len must reflect the second delta");
        sweep(&store, &rebuilt2, "second delta");

        // Restore the single-delta state the revert check below expects.
        let undo_ins: Vec<[Id; 3]> = more_deletes.iter().copied().filter(|t| reference.binary_search(t).is_ok()).collect();
        store.apply_delta(&undo_ins, &more_inserts);
        sweep(&store, &rebuilt, "reverted to first delta");

        // Reverting every EFFECTIVE change must drop the overlay entirely (no residual
        // overhead): re-insert the base triples that were deleted, delete the genuinely
        // new ones (the no-op delete/insert from above never entered the overlay).
        let eff_del: Vec<[Id; 3]> = deletes.iter().copied().filter(|t| triples.binary_search(t).is_ok()).collect();
        let eff_add: Vec<[Id; 3]> = inserts.iter().copied().filter(|t| triples.binary_search(t).is_err()).collect();
        store.apply_delta(&eff_del, &eff_add);
        assert!(!store.has_overlay(), "a fully reverted overlay must be dropped");
        let full = store.scan(&[None, None, None]);
        let mut orig: Vec<[Id; 3]> = triples.clone();
        orig.sort_unstable();
        assert_eq!(full.rows.as_ref(), &orig[..]);
    }

    /// The overlay zero-copy fast path (sq-7d3dj.3) must be PERF-ONLY: a scanned range the
    /// overlay does not touch (`count_correction == (0, 0)`) returns the BORROWED base
    /// slice, byte-identical (rows AND sort order) to the general merge path, while a range
    /// the overlay DOES touch still takes the owned merge path and reflects the correction.
    /// Proven directly — the `Cow` variant witnesses which path ran (a raw base range
    /// borrows; `merge` allocates an owned `Vec`), so the equality checks are non-vacuous.
    #[test]
    fn overlay_zero_copy_fast_path() {
        use std::borrow::Cow;

        // Base: subjects 1..=100, predicate 1, objects 1..=10 (1000 triples, several perms).
        let mut triples: Vec<[Id; 3]> = Vec::new();
        for s in 1..=100u32 {
            for o in 1..=10u32 {
                triples.push([s, 1, o]);
            }
        }
        // A no-overlay reference store: EVERY scan already borrows (the `None` branch).
        let base_store = TripleStore::from_triples(triples.clone());
        assert!(!base_store.has_overlay());

        // A small overlay touching ONLY subject 50: insert (50,1,11), delete (50,1,1).
        let mut store = TripleStore::from_triples(triples.clone());
        store.apply_delta(&[[50, 1, 11]], &[[50, 1, 1]]);
        assert!(store.has_overlay(), "the overlay must exist for a non-vacuous test");

        // A full rebuild of the corrected triple set (the general-path oracle).
        let mut reference: Vec<[Id; 3]> = triples.clone();
        reference.retain(|t| *t != [50, 1, 1]);
        reference.push([50, 1, 11]);
        let rebuilt = TripleStore::from_triples(reference);

        // (i) UNTOUCHED range (subject 7): fast path — rows are BORROWED and identical to
        // the no-overlay base store (which also borrows). This is the "store WITH an
        // overlay, scanned range clean" case the fast path exists for.
        for s in [1u32, 7, 49, 51, 100] {
            let pat: Pattern = [Some(s), None, None];
            let scan = store.scan(&pat);
            assert!(matches!(scan.rows, Cow::Borrowed(_)), "untouched subject {} must be zero-copy borrowed", s);
            assert_eq!(scan.rows, base_store.scan(&pat).rows, "fast-path rows must equal the base for subject {}", s);
            // A clean range is untouched by the overlay, so the rebuild agrees too.
            assert_eq!(scan.rows, rebuilt.scan(&pat).rows, "fast-path rows must equal the rebuild for subject {}", s);
        }

        // (ii) TOUCHED range (subject 50): general merge path — rows are OWNED, reflect the
        // correction (object 1 gone, 11 added), and match the rebuild in sort order.
        let touched: Pattern = [Some(50), None, None];
        let scan = store.scan(&touched);
        assert!(matches!(scan.rows, Cow::Owned(_)), "a range the overlay touches must take the merge path");
        assert_eq!(scan.rows, rebuilt.scan(&touched).rows, "merge-path rows must equal the rebuild");
        assert!(scan.rows.windows(2).all(|w| w[0] <= w[1]), "merge-path rows must stay sorted");

        // (iii) A full unbound scan intersects the overlay -> merge path, equals rebuild.
        let all: Pattern = [None, None, None];
        let scan = store.scan(&all);
        assert!(matches!(scan.rows, Cow::Owned(_)), "an overlay-intersecting full scan takes the merge path");
        assert_eq!(scan.rows, rebuilt.scan(&all).rows, "full merge-path scan must equal the rebuild");

        // (iv) An overlay that touches a range only via a DELETE (no insert) must still take
        // the merge path there (count_correction = (0, 1) != (0, 0)) and drop the row.
        let mut del_store = TripleStore::from_triples(triples.clone());
        del_store.apply_delta(&[], &[[7, 1, 5]]);
        let pat: Pattern = [Some(7), None, None];
        let scan = del_store.scan(&pat);
        assert!(matches!(scan.rows, Cow::Owned(_)), "a delete-only touched range must take the merge path");
        assert!(!scan.rows.contains(&[7, 1, 5]), "the deleted row must be absent");
        assert_eq!(scan.rows.len(), base_store.scan(&pat).rows.len() - 1, "exactly one row dropped");

        // (v) sq-7d3dj.16 — CACHE INVALIDATION, witnessed by the `Cow` variant. The cached
        // perm-sorted projection of `added` must never outlive the `added` it came from, in
        // BOTH directions: a range currently on the zero-copy path must LEAVE it when an
        // insertion lands there, and must RETURN to it when that insertion is retracted.
        //
        // The warm-up below scans P-bound and O-bound patterns as well as S-bound ones,
        // because SPO alone would NOT exercise a cache: `added` is already canonical-SPO
        // sorted, so that permutation aliases it and can never go stale. POS/PSO and
        // OSP/OPS are the permutations that really do materialise a projection, so those
        // are the ones a missing invalidation would serve stale rows from.
        let pat7: Pattern = [Some(7), None, None];
        let pat_p: Pattern = [None, Some(1), None];
        let pat_o: Pattern = [None, None, Some(11)];
        assert!(matches!(store.scan(&pat7).rows, Cow::Borrowed(_)), "subject 7 starts clean (cache warmed)");
        // Warm the non-SPO projections. Object 11 exists ONLY as the overlay's insertion,
        // so the O-bound scan is exactly one overlay row — a stale OSP cache is then visible
        // as a wrong row rather than as a needle in the base.
        let warm_p = store.scan(&pat_p).rows.len();
        // OSP rows are (o, s, p), so the single overlay insertion (50, 1, 11) reads (11, 50, 1).
        assert_eq!(store.scan(&pat_o).rows.as_ref(), [[11, 50, 1]], "OSP warm-up sees only the inserted object 11");

        // GROW: insert into the warmed ranges. Every projection must be rebuilt, so the scans
        // see the new row, subject 7 leaves the fast path, and all of it matches a rebuild.
        store.apply_delta(&[[7, 1, 11]], &[]);
        let mut grown: Vec<[Id; 3]> = triples.clone();
        grown.retain(|t| *t != [50, 1, 1]);
        grown.extend([[50, 1, 11], [7, 1, 11]]);
        let grown_rebuild = TripleStore::from_triples(grown);
        let scan = store.scan(&pat7);
        assert!(matches!(scan.rows, Cow::Owned(_)), "an insertion into the range must leave the fast path");
        assert!(scan.rows.contains(&[7, 1, 11]), "the freshly inserted row must be visible (stale cache would hide it)");
        assert_eq!(scan.rows, grown_rebuild.scan(&pat7).rows, "grown rows must equal the rebuild");
        assert!(scan.rows.windows(2).all(|w| w[0] <= w[1]), "grown rows must stay sorted");
        // The NON-SPO caches warmed above must have been invalidated too — these are the
        // assertions a missing `invalidate_added` fails, since SPO can never go stale.
        assert_eq!(
            store.scan(&pat_o).rows.as_ref(),
            [[11, 7, 1], [11, 50, 1]],
            "OSP must see BOTH object-11 rows (a stale cache keeps only the old one)"
        );
        assert_eq!(store.scan(&pat_p).rows.len(), warm_p + 1, "POS must count the new predicate-1 row");
        for pat in [pat_p, pat_o] {
            let scan = store.scan(&pat);
            assert_eq!(scan.rows, grown_rebuild.scan(&pat).rows, "grown cross-perm rows differ for {pat:?}");
            assert!(scan.rows.windows(2).all(|w| w[0] <= w[1]), "grown cross-perm rows must stay sorted");
        }

        // SHRINK: retract that pending insertion (the `added.remove` path). Every projection
        // must be rebuilt again, so the row disappears and subject 7 returns to zero-copy.
        store.apply_delta(&[], &[[7, 1, 11]]);
        let scan = store.scan(&pat7);
        assert!(matches!(scan.rows, Cow::Borrowed(_)), "retracting the insertion must restore the fast path");
        assert!(!scan.rows.contains(&[7, 1, 11]), "the retracted row must be gone (stale cache would keep it)");
        assert_eq!(scan.rows, base_store.scan(&pat7).rows, "retracted rows must equal the untouched base");
        assert_eq!(
            store.scan(&pat_o).rows.as_ref(),
            [[11, 50, 1]],
            "OSP must drop the retracted row (a stale cache keeps it)"
        );
        assert_eq!(store.scan(&pat_p).rows.len(), warm_p, "POS must be back to its pre-insert count");
        for pat in [pat7, pat_p, pat_o, [Some(50), None, None]] {
            assert_eq!(store.scan(&pat).rows, rebuilt.scan(&pat).rows, "reverted cross-perm rows differ for {pat:?}");
        }
    }

    /// [SONNET-4.6 sq-7d3dj.32.1] Each built raw-mode permutation Vec must carry zero
    /// capacity slack after construction.  heap_bytes() uses capacity(), so any excess
    /// inflates the reported B/triple; shrink_to_fit() (called in both build_raw_perms
    /// bodies) must leave len == capacity.  The input has heavy duplicates so the pre-dedup
    /// allocation is larger than the post-dedup len — without shrink_to_fit the test fails.
    #[test]
    fn build_raw_perms_no_capacity_slack() {
        // Build a Vec with heavy duplicates to maximise pre/post-dedup slack.
        // 4 unique triples repeated 100x each → capacity starts at 400, len after dedup = 4.
        let unique: Vec<[Id; 3]> = vec![
            [1, 1, 1],
            [2, 2, 2],
            [3, 3, 3],
            [4, 4, 4],
        ];
        let mut triples: Vec<[Id; 3]> = Vec::with_capacity(400);
        for _ in 0..100 {
            triples.extend_from_slice(&unique);
        }
        let store = TripleStore::from_triples(triples);
        for &perm in BUILT {
            if let PermData::Owned(ref v) = store.perms[perm as usize] {
                assert_eq!(
                    v.len(),
                    v.capacity(),
                    "permutation {perm:?}: capacity ({}) > len ({}) — dedup slack not eliminated",
                    v.capacity(),
                    v.len(),
                );
            }
        }
    }
}
