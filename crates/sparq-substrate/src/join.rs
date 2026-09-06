//! The shared id-tuple join kernels — sorted **merge join**, radix-partitioned
//! **hash join**, index-nested-loop **bind join**, and **leapfrog trie-join**
//! (WCOJ) — over the [`Row`] / [`Key`] / [`Posting`] vocabulary.
//!
//! These are the SPARQL engine's join probe loops, **moved here** from
//! `sparq-engine::exec`, generalised to operate on plain `&[Row]` slices plus a
//! tiny [`JoinKeys`] descriptor (the column layout) rather than the engine's
//! private `Bindings` struct. The engine keeps its planner, `Bindings`,
//! `LocalVocab` interning and `ScanCmp` filter pushdown private and wraps these
//! kernels with a thin adapter (`research/shared-eval-substrate.md` §2.3, Phase 3
//! of epic sq-qonbz). A future reasoner can drive the *same* kernels with its own
//! adapter, so both consumers share one join body without either depending on the
//! other.
//!
//! # Zero-overhead intent (the load-bearing contract)
//!
//! There is NO `Box<dyn>` / `&dyn` / vtable anywhere between a join's probe loop
//! and its key projection or its cooperative-cancellation poll. The kernels are
//! monomorphic over the concrete `Id = u32` and the `SmallVec` row/key aliases,
//! and the cancellation hook is a generic [`Budget`] type parameter (not a trait
//! object), so the compiler emits one specialised, inlinable body per call site.
//! Every hot item carries `#[inline]` so cross-crate inlining (with the workspace
//! LTO profile) keeps the engine's join hot loops identical to the pre-move
//! codegen. This is verified by the engine join/scan/BGP micro-benches staying
//! within noise of the pre-move baseline and the W3C SPARQL conformance floor
//! staying bit-identical.
//!
//! The kernels make **no entailment claim** — they compute joins over id-tuples;
//! which triples *should* exist is entirely the caller's (engine / reasoner)
//! concern (`research/shared-eval-substrate.md` §6).

use crate::rows::{Id, Key, Posting, Row, NO_ID};
use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;
use smallvec::SmallVec;
use std::cmp::Ordering;

/// A cooperative-cancellation poll the join kernels call once per key-group /
/// probe-row / distinct-value so a long join can be bounded without the kernel
/// knowing *how* the caller bounds it.
///
/// This is a **generic type parameter** on each kernel, NOT a trait object: the
/// engine supplies a zero-sized type whose [`exhausted`](Budget::exhausted) reads
/// its thread-local `QueryBudget`, a reasoner can supply its own closure-budget,
/// and a caller that wants no bound uses [`NoBudget`] — each monomorphises to a
/// direct call (or, for `NoBudget`, to a constant `false` the optimiser deletes),
/// so the probe loop pays no vtable.
pub trait Budget {
    /// Whether the join has produced enough output to stop (a sticky row/byte/time
    /// cap, or an external cancellation). `rows` is the current output length, so a
    /// caller can price the working set. Returning `true` cleanly truncates the
    /// result at a key-group boundary.
    fn exhausted(&self, rows: usize) -> bool;
}

/// An unbounded [`Budget`]: never stops the join. Zero-sized; its `exhausted`
/// folds to a constant `false` the optimiser removes, so an unbounded kernel call
/// has byte-identical codegen to a hand-written unbounded loop.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBudget;

impl Budget for NoBudget {
    #[inline]
    fn exhausted(&self, _rows: usize) -> bool {
        false
    }
}

/// A pure (no thread-local) exhaustion snapshot for parallel workers, where the
/// installing thread's sticky flag is out of reach. Generic, so the engine's
/// flattened limits implement it without a vtable on the rayon fold.
pub trait BudgetSnapshot: Sync {
    /// Whether a worker holding `rows` accumulated rows should stop producing.
    fn hit(&self, rows: usize) -> bool;
}

/// A [`BudgetSnapshot`] that never trips — the unbounded parallel case.
impl BudgetSnapshot for NoBudget {
    #[inline]
    fn hit(&self, _rows: usize) -> bool {
        false
    }
}

/// The column layout for combining a left (build) row with a right (probe) row:
/// which columns form the equi-join key on each side, which already-bound columns
/// must additionally agree (a repeated non-key shared variable), and which right
/// columns are appended to the output.
///
/// This is the **descriptor** the design calls `JoinKeys`: it captures the
/// row→key projection and the combine layout as plain index data (built by the
/// caller from its own variable layout), so the kernel stays generic over *what*
/// the columns mean while being monomorphic over the concrete `Row`/`Key` types.
/// No part of it is a trait object.
#[derive(Clone, Debug, Default)]
pub struct JoinKeys {
    /// `(left_col, right_col)` index pairs whose ids form the equi-join key — the
    /// columns hashed / merged on. For a hash join these are split into the build
    /// and probe key projections; for a merge join this is the single sorted
    /// variable. Build and probe must agree on the order of these pairs so the key
    /// tuples line up.
    pub key_cols: Vec<(usize, usize)>,
    /// The right-side columns appended (in order) after the left row's columns to
    /// form the combined output row.
    pub right_only: Vec<usize>,
}

impl JoinKeys {
    /// The left-side key column indices, in `key_cols` order.
    ///
    /// **Single-column fast path ([OPUS-4.8] sq-4r8uy).** The overwhelmingly
    /// common join is on ONE variable (`key_cols.len() == 1`). For it, building the
    /// [`Key`] by iterating the heap `key_cols` `Vec` and running the general
    /// `SmallVec::from_iter` collect machinery costs measurably more than a direct
    /// one-element push — the descriptor's per-row projection was the whole measured
    /// `hash_probe` overhead over a hand-specialised single-column probe (the #1810
    /// canonical delta). This branch special-cases it to `single_key`, which
    /// pushes exactly the one projected id, and falls through to the general
    /// `iter().collect()` for multi-column keys (byte-identical to the old path).
    /// The result is IDENTICAL to the general path in every case — same length,
    /// same ids, same order — so the join semantics are unchanged; only the
    /// single-column key derivation is cheaper.
    #[inline]
    pub fn left_key(&self, row: &[Id]) -> Key {
        if let [(lc, _)] = self.key_cols.as_slice() {
            return single_key(row[*lc]);
        }
        self.key_cols.iter().map(|&(lc, _)| row[lc]).collect()
    }

    /// The right-side key column indices, in `key_cols` order — projected to the
    /// SAME key tuple shape as [`left_key`](JoinKeys::left_key) so build and probe
    /// keys are equal exactly when the join columns are equal.
    ///
    /// Carries the same single-column fast path as [`left_key`](JoinKeys::left_key)
    /// ([OPUS-4.8] sq-4r8uy) — for `key_cols.len() == 1` it pushes exactly the one
    /// projected right-side id via `single_key` rather than running the general
    /// `iter().collect()`. The single-column result is IDENTICAL to the general
    /// path, so a build key and a probe key still compare equal exactly when the
    /// join columns agree — the hash-join correctness invariant is preserved.
    #[inline]
    pub fn right_key(&self, row: &[Id]) -> Key {
        if let [(_, rc)] = self.key_cols.as_slice() {
            return single_key(row[*rc]);
        }
        self.key_cols.iter().map(|&(_, rc)| row[rc]).collect()
    }
}

/// Build a one-element [`Key`] holding exactly `id` — the single-column key fast
/// path ([OPUS-4.8] sq-4r8uy).
///
/// This is byte-identical to `[id].into_iter().collect::<Key>()` (a length-1
/// [`Key`], the id in slot 0, inline — the `[Id; 2]` inline width holds one id with
/// no heap spill) but skips the general `SmallVec::from_iter` collect path: it
/// constructs an empty `Key` and pushes the single id, exactly what a
/// pre-extraction engine hard-coded for a single-variable equi-join. Kept a free
/// function (not inlined literally at each call site) so the two projection methods
/// share one definition and the differential test can pin the derivation.
#[inline]
fn single_key(id: Id) -> Key {
    let mut k = Key::new();
    k.push(id);
    k
}

/// Equality predicate used by the raw hash-table probe.
///
/// [FABLE-5] #3644: `SmallVec`'s general slice equality may lower to an outlined
/// `bcmp` even for the dominant one-id join key. Keep that shape as a scalar
/// comparison and retain the exact prior equality operation for every other key
/// width.
#[inline(always)]
fn probe_key_eq(candidate: &Key, key: &Key) -> bool {
    match key.as_slice() {
        [id] => matches!(candidate.as_slice(), [candidate_id] if candidate_id == id),
        _ => candidate == key,
    }
}

/// Select the serial table or the radix partition for a precomputed key hash.
///
/// Converting the documented 64-partition shape to an array reference gives LLVM
/// a statically-known bound; the masked partition index therefore needs no slice
/// bounds check. The fallback preserves the former behavior for malformed or
/// non-standard table slices.
#[inline(always)]
fn probe_table(tables: &[JoinTable], hash: u64) -> &JoinTable {
    if let [table] = tables {
        return table;
    }
    let partition = (hash % JOIN_PARTS as u64) as usize;
    if let Ok(partitioned) = <&[JoinTable; JOIN_PARTS]>::try_from(tables) {
        return &partitioned[partition];
    }
    &tables[partition]
}

/// The partition/lookup hash for a join key — build and probe must agree on it.
#[inline]
pub fn key_hash(key: &Key) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    key.hash(&mut h);
    h.finish()
}

/// Number of radix partitions for the parallel hash-join build. 64 spreads well at
/// high thread counts while keeping the per-partition tag-scan cheap.
pub const JOIN_PARTS: usize = 64;

/// The hash table type for hash-join build-side storage: a [`hashbrown::HashMap`] keyed on
/// [`Key`] (the equi-join projection), mapping to the sorted posting list of matching
/// build-row indices, backed by [`FxBuildHasher`].
///
/// Using `hashbrown::HashMap` (rather than `std::collections::HashMap`) lets [`probe_emit`]
/// and [`probe_gather_indices`] call `raw_entry().from_hash` to skip the second internal
/// re-hash on the partitioned probe path — a single [`key_hash`] call derives the radix
/// partition **and** performs the table lookup. The `FxBuildHasher` keeps the stored hash
/// identical to the explicit [`key_hash`] computation so the precomputed hash hits the
/// correct bucket every time.
///
/// [SONNET-4.6] sq-7d3dj.19
pub type JoinTable = HashMap<Key, Posting, FxBuildHasher>;

/// Sorted **merge join** of two relations already sorted on a single shared key
/// column (`lk` on the left, `rk` on the right), with optional `extra_shared`
/// `(left_col, right_col)` pairs that must additionally agree (a second shared
/// variable that is not the sorted key). Appends one combined row — the full left
/// row plus the `right_only` columns — per matching pair into `out`.
///
/// The inputs are plain row slices; the caller owns the output buffer and the
/// `out_vars` layout. `budget` is polled once per key group so a capped query
/// truncates cleanly at a group boundary (identical to the engine's pre-move
/// per-group check).
// The column-layout arguments are the price of operating on plain row slices instead of the
// engine's private `Bindings` struct — the whole point of the move (zero-overhead, shareable).
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn merge_join<B: Budget>(
    left: &[Row],
    lk: usize,
    right: &[Row],
    rk: usize,
    extra_shared: &[(usize, usize)],
    right_only: &[usize],
    budget: &B,
    out: &mut Vec<Row>,
) {
    let (l, r) = (left, right);
    let (mut i, mut j) = (0, 0);
    while i < l.len() && j < r.len() {
        // Coarse budget check once per key group.
        if budget.exhausted(out.len()) {
            break;
        }
        let (lv, rv) = (l[i][lk], r[j][rk]);
        match lv.cmp(&rv) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                let mut i2 = i;
                while i2 < l.len() && l[i2][lk] == lv {
                    i2 += 1;
                }
                let mut j2 = j;
                while j2 < r.len() && r[j2][rk] == rv {
                    j2 += 1;
                }
                for lrow in l.iter().take(i2).skip(i) {
                    for rrow in r.iter().take(j2).skip(j) {
                        if extra_shared.iter().all(|&(lc, rc)| lrow[lc] == rrow[rc]) {
                            let mut row = lrow.clone();
                            for &rc in right_only {
                                row.push(rrow[rc]);
                            }
                            out.push(row);
                        }
                    }
                }
                i = i2;
                j = j2;
            }
        }
    }
}

/// Builds the serial hash table for a hash join: maps each build-side key to the
/// ascending list of build-row indices sharing it. Returns a [`JoinTable`]
/// (`hashbrown::HashMap<Key, Posting, FxBuildHasher>`) so that the probe path can
/// call `raw_entry().from_hash` with the precomputed [`key_hash`] value.
#[inline]
pub fn build_table(build: &[Row], keys: &JoinKeys) -> JoinTable {
    let mut t = JoinTable::default();
    for (ri, row) in build.iter().enumerate() {
        t.entry(keys.left_key(row)).or_default().push(ri);
    }
    t
}

/// Builds the radix-partitioned hash tables for a parallel hash join: `JOIN_PARTS`
/// private maps, each over the build rows whose key-hash falls in that partition.
/// Within a partition rows are scanned in ascending index, so each posting list
/// stays in ascending build-row order — exactly the serial build — and the probe
/// output is byte-identical. Requires the `parallel` feature (the inner rayon
/// import is the caller's; this returns the per-partition maps). Returns
/// `Vec<`[`JoinTable`]`>` so the probe path can use `raw_entry().from_hash`.
#[inline]
pub fn build_partitioned(build: &[Row], keys: &JoinKeys, parts: &[u8]) -> Vec<JoinTable> {
    (0..JOIN_PARTS)
        .map(|p| {
            let mut t = JoinTable::default();
            for (ri, row) in build.iter().enumerate() {
                if parts[ri] as usize == p {
                    t.entry(keys.left_key(row)).or_default().push(ri);
                }
            }
            t
        })
        .collect()
}

/// Emits, for one probe row, every combined output row (its build-side matches
/// from `tables`, each extended with the probe-only columns). Shared by the serial
/// and parallel probe paths so they are byte-identical. `tables` is either one
/// serial [`JoinTable`] or `JOIN_PARTS` radix partitions; `probe_only` are the probe
/// columns appended after the build row.
///
/// **Single-hash optimisation ([SONNET-4.6] sq-7d3dj.19):** the join key is hashed
/// ONCE via [`key_hash`]; the hash bits select the radix partition and
/// `raw_entry().from_hash` performs the table lookup without a second internal
/// re-hash. `out.reserve(matches.len())` allocates for all matches in one call —
/// the **batch-emission contract** the sq-pntvh M4 morsel pipeline inherits:
/// reserve then materialise once per key group, not per match.
#[inline]
pub fn probe_emit(
    prow: &Row,
    keys: &JoinKeys,
    build: &[Row],
    tables: &[JoinTable],
    probe_only: &[usize],
    out: &mut Vec<Row>,
) {
    let key: Key = keys.right_key(prow);
    // [SONNET-4.6] sq-7d3dj.19: hash ONCE — partition selection and raw_entry lookup
    // share this value; no second internal re-hash on the partitioned probe path.
    let h = key_hash(&key);
    let table = probe_table(tables, h);
    let Some((_, matches)) = table.raw_entry().from_hash(h, |k| probe_key_eq(k, &key)) else {
        return;
    };
    // Reserve exact match count — one allocation for the entire key group.
    // Batch-emission contract (sq-pntvh M4): materialise after reserve, not per match.
    // [SONNET-4.6] sq-7d3dj.19
    out.reserve(matches.len());
    for &bi in matches {
        let mut combined = build[bi].clone();
        for &pi in probe_only {
            combined.push(prow[pi]);
        }
        out.push(combined);
    }
}

/// Collects, for one probe row, the build-side match indices into `build_indices`
/// without materialising any output rows. This is the **M4 batch-emission contract**
/// (sq-pntvh.7): a morsel pipeline calls this to gather `(build_idx, probe_row)`
/// index runs, then materialises the combined output once per chunk — no per-match
/// `Row` clone. The scalar [`probe_emit`] is the row-materialising wrapper over
/// this primitive.
///
/// The join key is hashed ONCE via [`key_hash`]; the hash selects the radix
/// partition and drives `raw_entry().from_hash` — the partitioned probe path pays
/// exactly one hash operation, identical to the serial path.
///
/// [SONNET-4.6] sq-7d3dj.19
#[inline]
pub fn probe_gather_indices(
    prow: &Row,
    keys: &JoinKeys,
    tables: &[JoinTable],
    build_indices: &mut Vec<usize>,
) {
    let key: Key = keys.right_key(prow);
    let h = key_hash(&key);
    let table = probe_table(tables, h);
    if let Some((_, matches)) = table.raw_entry().from_hash(h, |k| probe_key_eq(k, &key)) {
        build_indices.extend(matches.iter().copied());
    }
}

/// Serial **hash join** probe: for every probe row, emit its build-side matches.
/// `budget` is polled once per probe row (the engine's pre-move per-row check).
/// The build table(s) and the layout are the caller's; this is the probe hot loop.
/// Delegates per-row emission to [`probe_emit`] (single-hash + batch-reserve).
#[inline]
pub fn hash_probe_serial<B: Budget>(
    probe: &[Row],
    keys: &JoinKeys,
    build: &[Row],
    tables: &[JoinTable],
    probe_only: &[usize],
    budget: &B,
    out: &mut Vec<Row>,
) {
    for prow in probe {
        // Coarse budget check once per probe row.
        if budget.exhausted(out.len()) {
            break;
        }
        probe_emit(prow, keys, build, tables, probe_only, out);
    }
}

/// Index-nested-loop **bind join** combine step: given the groups of result rows
/// keyed by the join value and, for one distinct value, the projected new-variable
/// tuples of the matching scanned triples, append one combined row per
/// (result-row, match) pair. The scan + filter pushdown stay with the caller (the
/// engine), which owns the store and the `ScanCmp`; this is the pure id-tuple
/// combine that produced the per-value output rows.
#[inline]
pub fn bind_combine(result_rows: &[Row], ris: &[usize], new_vals: &[Id], out: &mut Vec<Row>) {
    for &ri in ris {
        bind_emit(&result_rows[ri], new_vals, out);
    }
}

/// Appends one combined row per row in a contiguous bind-join group.
///
/// Pass the group's row slice and the new-variable values from one scanned match.
/// Rows are appended in slice order, including duplicates; existing output is kept.
/// Like [`bind_combine`], this only combines id tuples. The caller owns scanning,
/// filtering and budget checks. No index vector is needed for the group.
/// [GPT-6-ASTRA]
#[inline]
pub fn bind_combine_rows(result_rows: &[Row], new_vals: &[Id], out: &mut Vec<Row>) {
    for row in result_rows {
        bind_emit(row, new_vals, out);
    }
}

// [GPT-6-ASTRA] Indexed and contiguous groups share the same row construction.
#[inline]
fn bind_emit(row: &Row, new_vals: &[Id], out: &mut Vec<Row>) {
    let mut combined = row.clone();
    combined.extend(new_vals.iter().copied());
    out.push(combined);
}

// ---- Leapfrog Triejoin (WCOJ) ------------------------------------------------
//
// LFTJ (Veldhuizen 2014) evaluates a BGP one *variable* at a time in a fixed
// global order. At each variable it intersects, via the "leapfrog" galloping
// search, the sorted value streams of every pattern mentioning that variable,
// then recurses. Each pattern is a [`Trie`] of its variable columns. The total
// work is bounded by the AGM fractional-edge-cover bound. The trie *contents* are
// built by the caller (the engine projects them from a permutation index); this
// module owns the navigation (the `TrieIter` cursor, the `Leapfrog` intersection,
// and `lftj_recurse`), which is the WCOJ hot loop.

/// A pattern's relation projected onto its variables (in global order) as sorted,
/// deduplicated tuples — the trie LFTJ navigates level by level. The caller builds
/// and fills `tuples`.
#[derive(Clone, Debug, Default)]
pub struct Trie {
    /// The sorted, deduplicated variable tuples (each in global variable order).
    pub tuples: Vec<Vec<Id>>,
}

/// One open level of a [`TrieIter`]: `hi` bounds the current key's subtree (rows
/// sharing all already-fixed columns), `cur` is the cursor within it.
struct Frame {
    hi: usize,
    cur: usize,
}

/// A cursor over a [`Trie`], using Veldhuizen's open-on-entry semantics: it starts
/// *above* the root, and `open()` descends one column, resetting the cursor to the
/// start of that subtree. This reset is what makes non-contiguous variable
/// participation correct — re-entering a level re-opens (rewinds) the iterator.
pub struct TrieIter<'a> {
    trie: &'a Trie,
    frames: Vec<Frame>,
}

impl<'a> TrieIter<'a> {
    /// A fresh cursor positioned above the root of `trie`.
    #[inline]
    pub fn new(trie: &'a Trie) -> Self {
        TrieIter { trie, frames: Vec::new() }
    }
    /// The column currently being iterated (valid once at least one `open`).
    #[inline]
    fn col(&self) -> usize {
        self.frames.len() - 1
    }
    /// Whether the current level's cursor has run past its subtree.
    #[inline]
    fn at_end(&self) -> bool {
        let f = self.frames.last().unwrap();
        f.cur >= f.hi
    }
    /// The id at the cursor in the current column.
    #[inline]
    fn key(&self) -> Id {
        let col = self.col();
        let f = self.frames.last().unwrap();
        self.trie.tuples[f.cur][col]
    }
    /// End (exclusive) of the run of rows in `[start, hi)` whose `col` equals the
    /// value at `start`. The slice is sorted, so this is a binary search — O(log n)
    /// rather than a linear scan of the (possibly large) run.
    #[inline]
    fn run_end(&self, col: usize, start: usize, hi: usize, val: Id) -> usize {
        start + self.trie.tuples[start..hi].partition_point(|row| row[col] <= val)
    }
    /// Advances to the next distinct value in the current column.
    #[inline]
    fn next(&mut self) {
        let col = self.col();
        let (cur, hi) = {
            let f = self.frames.last().unwrap();
            (f.cur, f.hi)
        };
        let val = self.trie.tuples[cur][col];
        self.frames.last_mut().unwrap().cur = self.run_end(col, cur, hi, val);
    }
    /// Galloping seek: first value `>= x` in the current column.
    #[inline]
    fn seek(&mut self, x: Id) {
        let col = self.col();
        let f = self.frames.last_mut().unwrap();
        let (mut a, mut b) = (f.cur, f.hi);
        while a < b {
            let m = a + (b - a) / 2;
            if self.trie.tuples[m][col] < x {
                a = m + 1;
            } else {
                b = m;
            }
        }
        f.cur = a;
    }
    /// Descends one column: into the subtree of the parent's current key (or, at
    /// the root, the whole relation), with the cursor reset to its start.
    #[inline]
    fn open(&mut self) {
        match self.frames.last() {
            None => {
                self.frames.push(Frame { hi: self.trie.tuples.len(), cur: 0 });
            }
            Some(&Frame { cur: plo, hi: phi }) => {
                let pcol = self.frames.len() - 1;
                let val = self.trie.tuples[plo][pcol];
                let end = self.run_end(pcol, plo, phi, val);
                self.frames.push(Frame { hi: end, cur: plo });
            }
        }
    }
    /// Ascends one column.
    #[inline]
    fn up(&mut self) {
        self.frames.pop();
    }
}

/// Leapfrog intersection of the participating iterators at one level.
struct Leapfrog {
    /// Participant iterator indices kept in cyclic ascending-key order.
    ///
    /// `SmallVec<[usize; 8]>` inline for k ≤ 8 (all practical WCO arities) so the
    /// struct lives entirely on the stack — no per-recursion-level heap allocation.
    /// For k=3 (the dominant triangle/clique case), `search_k3` loads all three
    /// entries as stack-local copies at entry so the hot loop never reads this field.
    ///
    /// [SONNET-4.6] sq-7d3dj.20
    order: SmallVec<[usize; 8]>,
    p: usize,
    ended: bool,
    key: Id,
}

impl Leapfrog {
    fn init(iters: &mut [TrieIter], parts: &[usize]) -> Self {
        let mut lf = Leapfrog {
            order: SmallVec::from_slice(parts),
            p: 0,
            ended: false,
            key: 0,
        };
        if parts.iter().any(|&i| iters[i].at_end()) {
            lf.ended = true;
            return lf;
        }
        lf.order.sort_by_key(|&i| iters[i].key());
        lf.search(iters);
        lf
    }

    /// Advance the leapfrog state to the next common value across all participants.
    ///
    /// Three micro-optimisations ([SONNET-4.6] sq-7d3dj.20):
    ///
    /// (a) **Branchless wrap**: `p` only advances by 1 per iteration, so
    ///     `(p + 1) % k` becomes `p += 1; if p == k { p = 0 }` — compiles to a
    ///     `cmov` rather than an integer divide.
    ///
    /// (b) **Hoisted indirection**: `order[p]` and `order[prev]` are resolved once
    ///     per iteration into `cur_idx` / `prev_idx` locals, eliminating repeated
    ///     SmallVec + slice pointer chases inside the hot loop.
    ///
    /// (c) **Monomorphized k=3 dispatch**: routes to `search_k3` when
    ///     `order.len() == 3` (the triangle / 3-clique case dominating WCO workloads),
    ///     which additionally copies all three entries as stack-local variables so the
    ///     hot loop never touches the `order` field at all.
    #[inline]
    fn search(&mut self, iters: &mut [TrieIter]) {
        let k = self.order.len();
        // (c) Route to monomorphized k=3 path for the dominant WCO arity.
        if k == 3 {
            return self.search_k3(iters);
        }
        loop {
            // (a) Branchless predecessor: p only decrements by 1 cyclically.
            let prev = if self.p == 0 { k - 1 } else { self.p - 1 };
            // (b) Hoist: resolve order[..] into locals once per iteration.
            let prev_idx = self.order[prev];
            let cur_idx = self.order[self.p];
            let max = iters[prev_idx].key();
            let min = iters[cur_idx].key();
            if min == max {
                self.key = min;
                return;
            }
            iters[cur_idx].seek(max);
            if iters[cur_idx].at_end() {
                self.ended = true;
                return;
            }
            // (a) Branchless wrap: advance p one step mod k.
            self.p += 1;
            if self.p == k {
                self.p = 0;
            }
        }
    }

    /// Monomorphized k=3 fast path for triangle / 3-clique queries.
    ///
    /// Loads `order[0..3]` into a stack-local array `o` at entry so the hot loop
    /// reads register-resident copies rather than the `SmallVec` field.  Cyclic
    /// advance is a compare-and-reset against the fixed modulus 3, eliminating both
    /// `%` ops present in the generic path.
    ///
    /// Only called from [`search`](Leapfrog::search) when `order.len() == 3`.
    /// [SONNET-4.6] sq-7d3dj.20
    #[inline]
    fn search_k3(&mut self, iters: &mut [TrieIter]) {
        debug_assert_eq!(self.order.len(), 3);
        // Load all three order entries as stack-locals — hot loop never touches
        // self.order again. [SONNET-4.6] sq-7d3dj.20
        let o = [self.order[0], self.order[1], self.order[2]];
        loop {
            // Predecessor in {0, 1, 2}: 0 → 2, else p − 1.
            let prev = if self.p == 0 { 2usize } else { self.p - 1 };
            let ci = o[self.p]; // current iterator index
            let max = iters[o[prev]].key();
            let min = iters[ci].key();
            if min == max {
                self.key = min;
                return;
            }
            iters[ci].seek(max);
            if iters[ci].at_end() {
                self.ended = true;
                return;
            }
            // Branchless cyclic advance for the fixed modulus 3: no % op.
            self.p += 1;
            if self.p == 3 {
                self.p = 0;
            }
        }
    }

    #[inline]
    fn next(&mut self, iters: &mut [TrieIter]) {
        let k = self.order.len();
        // (b) Hoist: resolve order[p] once. [SONNET-4.6] sq-7d3dj.20
        let cur_idx = self.order[self.p];
        iters[cur_idx].next();
        if iters[cur_idx].at_end() {
            self.ended = true;
            return;
        }
        // (a) Branchless wrap: advance p one step mod k. [SONNET-4.6] sq-7d3dj.20
        self.p += 1;
        if self.p == k {
            self.p = 0;
        }
        self.search(iters);
    }
}

/// The Leapfrog-Triejoin recursion: at each global `level`, intersect the
/// participating tries' value streams and recurse, appending one output [`Row`]
/// (a copy of `current`) per full match. `budget` is polled once per leapfrog key
/// (sticky, so it also unwinds the enclosing recursion levels) — the engine's
/// pre-move per-key check, threaded as a generic type so the recursion carries no
/// vtable.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn lftj_recurse<B: Budget>(
    iters: &mut [TrieIter],
    parts_at_level: &[Vec<usize>],
    level: usize,
    n_levels: usize,
    current: &mut [Id],
    budget: &B,
    out: &mut Vec<Row>,
) {
    if level == n_levels {
        out.push(Row::from_slice(current));
        return;
    }
    let parts = &parts_at_level[level];
    // Open-on-entry: descend each relevant iterator into this level (rewinding it).
    for &i in parts {
        iters[i].open();
    }
    let mut lf = Leapfrog::init(iters, parts);
    while !lf.ended {
        // Coarse budget check once per leapfrog key (sticky, so it also unwinds
        // the enclosing recursion levels).
        if budget.exhausted(out.len()) {
            break;
        }
        current[level] = lf.key;
        lftj_recurse(iters, parts_at_level, level + 1, n_levels, current, budget, out);
        lf.next(iters);
    }
    for &i in parts {
        iters[i].up();
    }
}

/// SPARQL solution compatibility on the shared columns: an unbound (`NO_ID`) value
/// never conflicts; two bound values must be equal. Used by the OPTIONAL / UNION /
/// VALUES-UNDEF fallback nested loop the engine drives.
#[inline]
pub fn compatible(lrow: &[Id], rrow: &[Id], shared: &[(usize, usize)]) -> bool {
    shared.iter().all(|&(lc, rc)| {
        let (a, b) = (lrow[lc], rrow[rc]);
        a == NO_ID || b == NO_ID || a == b
    })
}

/// Combines two compatible rows: left's row extended with the right-only columns,
/// filling any shared column that was unbound on the left from the right side.
#[inline]
pub fn merge_rows(lrow: &[Id], rrow: &[Id], shared: &[(usize, usize)], right_only: &[usize]) -> Row {
    let mut row = Row::from_slice(lrow);
    for &(lc, rc) in shared {
        if row[lc] == NO_ID {
            row[lc] = rrow[rc];
        }
    }
    for &rc in right_only {
        row.push(rrow[rc]);
    }
    row
}

/// Whether any row leaves any of the given columns unbound (`NO_ID`).
#[inline]
pub fn any_unbound(rows: &[Row], cols: &[usize]) -> bool {
    rows.iter().any(|r| cols.iter().any(|&c| r[c] == NO_ID))
}

/// Delta-aware build-side table for the semi-naive Δ⋈full join shape.
///
/// This module adds a persistent, extendable build-side table alongside the static
/// kernels so the OWL-RL semi-naive fixpoint can share the substrate join without
/// paying per-round O(|full|) rebuild cost. See `research/substrate-remaining-design.md`
/// §3 (decision R1) and §4 (the seam shape) for the design rationale and the
/// byte-identity guarantee.
///
/// [SONNET-4.6] sq-qonbz.1
pub mod delta {
    use super::{Budget, JoinKeys};
    use crate::rows::{Key, Posting, Row};
    use rustc_hash::FxHashMap;

    /// A persistent, extendable build-side hash table for the semi-naive Δ⋈full join.
    ///
    /// ## Layout
    ///
    /// A `Vec<Row>` arena plus an `FxHashMap<Key, Posting>` where each posting list
    /// holds **arena offsets in insertion order** (ascending). Match enumeration by
    /// `probe_emit` yields results in that order so that, given an identical insertion
    /// sequence, the consumer's emission sequence is byte-identical — the determinism
    /// guarantee required by the OWL-RL ratchet (decision R1, §4.2 of the design record).
    ///
    /// ## Zero-overhead contract
    ///
    /// No `Box<dyn>` / `&dyn` anywhere on any path. The emit closure and the budget are
    /// generic type parameters — the compiler emits one monomorphised body per call site.
    /// `probe_emit` carries `#[inline]` so cross-crate LTO keeps the probe hot loop
    /// identical to a hand-written equivalent. `scripts/check-no-dyn-dispatch.py` covers
    /// the parent `join.rs` file (which contains this module) structurally.
    ///
    /// ## Feature requirement
    ///
    /// Available only with the `join` Cargo feature (which implies `rows`).
    ///
    /// [SONNET-4.6] sq-qonbz.1
    pub struct DeltaTable {
        /// Row arena — build-side rows stored in insertion order.
        arena: Vec<Row>,
        /// Key to list of arena offsets (ascending = insertion order).
        map: FxHashMap<Key, Posting>,
    }

    impl DeltaTable {
        /// Construct an empty `DeltaTable`.
        ///
        /// Equivalent to `DeltaTable::default()`.
        #[inline]
        pub fn new() -> Self {
            DeltaTable { arena: Vec::new(), map: FxHashMap::default() }
        }

        /// Build the table from `rows` using the left-side key projection from `keys`.
        ///
        /// Shares the same hashing and keying discipline as the static `build_table`
        /// kernel so there is exactly one keying convention in the crate.
        #[inline]
        pub fn build(rows: &[Row], keys: &JoinKeys) -> Self {
            let mut t = DeltaTable {
                arena: Vec::with_capacity(rows.len()),
                map: FxHashMap::default(),
            };
            for row in rows {
                let offset = t.arena.len();
                t.arena.push(row.clone());
                t.map.entry(keys.left_key(row)).or_default().push(offset);
            }
            t
        }

        /// Append a round's Δ rows to the arena and index.
        ///
        /// Per-round cost is O(|Δ|): existing rows are never rehashed or copied, so the
        /// asymptotic cost matches the hand-rolled `FxHashMap` adjacency the OWL-RL
        /// closure uses today.
        #[inline]
        pub fn extend(&mut self, delta_rows: &[Row], keys: &JoinKeys) {
            self.arena.reserve(delta_rows.len());
            for row in delta_rows {
                let offset = self.arena.len();
                self.arena.push(row.clone());
                self.map.entry(keys.left_key(row)).or_default().push(offset);
            }
        }

        /// Full reconstruction from `rows`, discarding all prior content.
        ///
        /// Provided for the consumer's union-find merge-epoch policy: when a union merges
        /// representatives and the consumer must re-canonicalise its rows, it calls
        /// `rebuild` at the same program points where the hand-rolled code rebuilt its
        /// adjacency map. The seam is policy-free — it neither observes the `UnionFind`
        /// nor decides when to rebuild.
        #[inline]
        pub fn rebuild(&mut self, rows: &[Row], keys: &JoinKeys) {
            self.arena.clear();
            self.map.clear();
            self.arena.reserve(rows.len());
            for row in rows {
                let offset = self.arena.len();
                self.arena.push(row.clone());
                self.map.entry(keys.left_key(row)).or_default().push(offset);
            }
        }

        /// Probe the build side with each row in `delta`, invoking `emit` for every match.
        ///
        /// For each probe row in `delta` this looks up its right-side key (via `keys`)
        /// in the build index and calls `emit(build_row, probe_row)` for each build-side
        /// match, in **insertion order** (ascending arena offset).
        ///
        /// **Determinism contract**: given an identical insertion sequence (`build` +
        /// `extend` calls), the emission sequence is identical. This is load-bearing for
        /// the OWL-RL byte-identity ratchet (§4.2 of the design record).
        ///
        /// `budget` is polled once per probe row (sticky: if exhausted, the remainder of
        /// `delta` is skipped). `emit` and `budget` are generic type parameters — no
        /// `Box<dyn>`, no vtable.
        ///
        /// **Budget count is per-pass, by design.** The value passed to `Budget::exhausted`
        /// is the number of rows emitted by *this* `probe_emit` call, not a cumulative total
        /// across successive `build`/`extend`/`probe_emit` rounds. A caller that wants one
        /// bound spanning a whole Δ fixpoint owns that cumulative count in its own `Budget`
        /// state — this is the reasoner's closure-level budget, deliberately a reasoner (not
        /// a substrate) concern per `research/shared-eval-substrate.md` §5. The seam only
        /// exposes the cooperative poll; it does not track fixpoint-global progress. A budget
        /// whose exhaustion depends solely on the passed count (e.g. a bare max-rows cap)
        /// will therefore reset each pass — such a caller must feed its own running total.
        #[inline]
        pub fn probe_emit<B: Budget, F: FnMut(&Row, &Row)>(
            &self,
            delta: &[Row],
            keys: &JoinKeys,
            budget: &B,
            emit: &mut F,
        ) {
            let mut emitted = 0usize;
            for prow in delta {
                if budget.exhausted(emitted) {
                    break;
                }
                let key: Key = keys.right_key(prow);
                if let Some(offsets) = self.map.get(&key) {
                    for &offset in offsets {
                        emit(&self.arena[offset], prow);
                        emitted += 1;
                    }
                }
            }
        }

        /// The number of rows currently in the build-side arena.
        #[inline]
        pub fn len(&self) -> usize {
            self.arena.len()
        }

        /// Whether the build-side arena is empty.
        #[inline]
        pub fn is_empty(&self) -> bool {
            self.arena.is_empty()
        }
    }

    impl Default for DeltaTable {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use super::super::{Budget, NoBudget, JoinKeys, build_table, probe_emit as static_probe_emit};
        use crate::rows::Row;

        fn row(xs: &[u32]) -> Row {
            Row::from_slice(xs)
        }

        /// `JoinKeys` that joins on column 0 of both sides, no additional probe columns.
        fn keys_col0() -> JoinKeys {
            JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] }
        }

        // --- new() / Default ---

        #[test]
        fn new_is_empty() {
            let t = DeltaTable::new();
            assert!(t.is_empty());
            assert_eq!(t.len(), 0);
        }

        #[test]
        fn default_is_empty() {
            let t: DeltaTable = Default::default();
            assert!(t.is_empty());
            assert_eq!(t.len(), 0);
        }

        // --- build() ---

        #[test]
        fn build_indexes_rows_by_key() {
            let rows = vec![row(&[1, 10]), row(&[2, 20]), row(&[1, 11])];
            let keys = keys_col0();
            let t = DeltaTable::build(&rows, &keys);
            assert_eq!(t.len(), 3);
            assert!(!t.is_empty());

            // Probe key=1: expect build rows [1,10] then [1,11] in insertion order.
            let probe = vec![row(&[1, 99])];
            let mut hits: Vec<Row> = Vec::new();
            t.probe_emit(&probe, &keys, &NoBudget, &mut |b, _p| {
                hits.push(b.clone());
            });
            assert_eq!(hits, vec![row(&[1, 10]), row(&[1, 11])]);
        }

        #[test]
        fn build_empty_rows_is_empty() {
            let t = DeltaTable::build(&[], &keys_col0());
            assert!(t.is_empty());
            assert_eq!(t.len(), 0);
        }

        // --- extend() ---

        #[test]
        fn extend_appends_delta_rows_in_order() {
            let base = vec![row(&[1, 10])];
            let keys = keys_col0();
            let mut t = DeltaTable::build(&base, &keys);
            let delta = vec![row(&[1, 11]), row(&[2, 20])];
            t.extend(&delta, &keys);
            assert_eq!(t.len(), 3);

            // Probe key=1: base row then delta row, insertion order.
            let probe = vec![row(&[1, 0])];
            let mut hits: Vec<Row> = Vec::new();
            t.probe_emit(&probe, &keys, &NoBudget, &mut |b, _p| {
                hits.push(b.clone());
            });
            assert_eq!(hits, vec![row(&[1, 10]), row(&[1, 11])]);
        }

        #[test]
        fn extend_empty_delta_is_noop() {
            let base = vec![row(&[1, 10])];
            let keys = keys_col0();
            let mut t = DeltaTable::build(&base, &keys);
            t.extend(&[], &keys);
            assert_eq!(t.len(), 1);
        }

        // --- rebuild() ---

        #[test]
        fn rebuild_replaces_entire_table() {
            let initial = vec![row(&[1, 10]), row(&[2, 20])];
            let keys = keys_col0();
            let mut t = DeltaTable::build(&initial, &keys);

            let new_rows = vec![row(&[3, 30]), row(&[3, 31])];
            t.rebuild(&new_rows, &keys);
            assert_eq!(t.len(), 2);

            // Old key=1 entry must be gone.
            let mut hits_old: Vec<Row> = Vec::new();
            t.probe_emit(&[row(&[1, 0])], &keys, &NoBudget, &mut |b, _| {
                hits_old.push(b.clone());
            });
            assert!(hits_old.is_empty(), "rebuild must clear old entries");

            // New key=3 entries present in insertion order.
            let mut hits_new: Vec<Row> = Vec::new();
            t.probe_emit(&[row(&[3, 0])], &keys, &NoBudget, &mut |b, _| {
                hits_new.push(b.clone());
            });
            assert_eq!(hits_new, vec![row(&[3, 30]), row(&[3, 31])]);
        }

        #[test]
        fn rebuild_to_empty_clears_all() {
            let rows = vec![row(&[1, 10])];
            let keys = keys_col0();
            let mut t = DeltaTable::build(&rows, &keys);
            t.rebuild(&[], &keys);
            assert!(t.is_empty());
            let mut count = 0usize;
            t.probe_emit(&[row(&[1, 0])], &keys, &NoBudget, &mut |_, _| count += 1);
            assert_eq!(count, 0);
        }

        // --- probe_emit() ---

        #[test]
        fn probe_emit_empty_delta_emits_nothing() {
            let rows = vec![row(&[1, 10])];
            let keys = keys_col0();
            let t = DeltaTable::build(&rows, &keys);
            let mut count = 0usize;
            t.probe_emit(&[], &keys, &NoBudget, &mut |_, _| count += 1);
            assert_eq!(count, 0);
        }

        #[test]
        fn probe_emit_empty_table_emits_nothing() {
            let keys = keys_col0();
            let t = DeltaTable::new();
            let probe = vec![row(&[1, 99])];
            let mut count = 0usize;
            t.probe_emit(&probe, &keys, &NoBudget, &mut |_, _| count += 1);
            assert_eq!(count, 0);
        }

        #[test]
        fn probe_emit_no_matching_key_emits_nothing() {
            let rows = vec![row(&[1, 10]), row(&[2, 20])];
            let keys = keys_col0();
            let t = DeltaTable::build(&rows, &keys);
            let probe = vec![row(&[99, 0])];
            let mut count = 0usize;
            t.probe_emit(&probe, &keys, &NoBudget, &mut |_, _| count += 1);
            assert_eq!(count, 0);
        }

        #[test]
        fn probe_emit_passes_probe_row_to_closure() {
            let rows = vec![row(&[1, 10])];
            let keys = keys_col0();
            let t = DeltaTable::build(&rows, &keys);
            let probe = vec![row(&[1, 77])];
            let mut probe_rows_seen: Vec<Row> = Vec::new();
            t.probe_emit(&probe, &keys, &NoBudget, &mut |_b, p| {
                probe_rows_seen.push(p.clone());
            });
            assert_eq!(probe_rows_seen, vec![row(&[1, 77])]);
        }

        // --- Determinism contract (load-bearing for byte-identity) ---

        #[test]
        fn probe_emit_insertion_order_is_deterministic_across_build_extend_rebuild() {
            // Build incrementally (build + two extends), then rebuild with the same
            // concatenated rows. Both paths must yield matches in the same insertion order.
            let keys = keys_col0();
            let r1 = vec![row(&[5, 1])];
            let r2 = vec![row(&[5, 2])];
            let r3 = vec![row(&[5, 3])];

            // Incremental path.
            let mut t_incr = DeltaTable::build(&r1, &keys);
            t_incr.extend(&r2, &keys);
            t_incr.extend(&r3, &keys);

            // Batch path (build over the concatenated rows).
            let mut all: Vec<Row> = Vec::new();
            all.extend_from_slice(&r1);
            all.extend_from_slice(&r2);
            all.extend_from_slice(&r3);
            let t_batch = DeltaTable::build(&all, &keys);

            // Rebuild path.
            let mut t_rebuild = DeltaTable::new();
            t_rebuild.rebuild(&all, &keys);

            let probe = vec![row(&[5, 0])];
            let collect = |t: &DeltaTable| -> Vec<Row> {
                let mut seq = Vec::new();
                t.probe_emit(&probe, &keys, &NoBudget, &mut |b, _p| seq.push(b.clone()));
                seq
            };

            let seq_incr = collect(&t_incr);
            let seq_batch = collect(&t_batch);
            let seq_rebuild = collect(&t_rebuild);

            let expected = vec![row(&[5, 1]), row(&[5, 2]), row(&[5, 3])];
            assert_eq!(seq_incr, expected, "incremental build+extend must yield insertion order");
            assert_eq!(seq_batch, expected, "batch build must yield insertion order");
            assert_eq!(seq_rebuild, expected, "rebuild must yield insertion order");
        }

        // --- Equivalence vs. static build_table + probe_emit ---

        #[test]
        fn build_extend_equivalent_to_static_build_over_concatenated_rows() {
            // DeltaTable::build(base) + extend(delta) must yield the same probe matches
            // as the static build_table over (base ++ delta) — the primary correctness claim.
            let base = vec![row(&[7, 1]), row(&[8, 2])];
            let delta = vec![row(&[7, 3]), row(&[9, 4])];
            let keys = keys_col0();

            // Delta path.
            let mut dt = DeltaTable::build(&base, &keys);
            dt.extend(&delta, &keys);

            // Static path over the concatenated rows.
            let mut all: Vec<Row> = base.clone();
            all.extend_from_slice(&delta);
            let static_tables = vec![build_table(&all, &keys)];

            let probe = vec![row(&[7, 0]), row(&[8, 0]), row(&[9, 0]), row(&[99, 0])];

            // Collect delta-table build rows.
            let mut dt_hits: Vec<Row> = Vec::new();
            dt.probe_emit(&probe, &keys, &NoBudget, &mut |b, _p| dt_hits.push(b.clone()));

            // Collect static build rows (probe_only=[] so output IS the build row).
            let mut st_hits: Vec<Row> = Vec::new();
            for prow in &probe {
                static_probe_emit(prow, &keys, &all, &static_tables, &[], &mut st_hits);
            }

            assert_eq!(dt_hits, st_hits, "DeltaTable must yield the same matches as static build_table");
        }

        // --- Budget truncation ---

        #[test]
        fn probe_emit_budget_truncates_after_first_match() {
            struct Cap(usize);
            impl Budget for Cap {
                fn exhausted(&self, rows: usize) -> bool {
                    rows >= self.0
                }
            }
            // Three probe rows each match one build row; cap at 1 match.
            let rows = vec![row(&[1, 10]), row(&[2, 20]), row(&[3, 30])];
            let keys = keys_col0();
            let t = DeltaTable::build(&rows, &keys);
            let probe = vec![row(&[1, 0]), row(&[2, 0]), row(&[3, 0])];
            let mut count = 0usize;
            t.probe_emit(&probe, &keys, &Cap(1), &mut |_, _| count += 1);
            assert_eq!(count, 1, "Cap(1) must stop after the first emitted match");
        }

        #[test]
        fn probe_emit_budget_cooperative_cancel_with_control() {
            // [HAIKU-4.5] sq-qonbz.6
            // Test Budget-driven cooperative cancellation mid-enumeration, spanning
            // multiple budget checks (one per probe row). Asserts truncation vs. a
            // NoBudget control run to verify deterministic emission order.
            struct Cap(usize);
            impl Budget for Cap {
                fn exhausted(&self, rows: usize) -> bool {
                    rows >= self.0
                }
            }

            let keys = keys_col0();

            // Build table with distinct keys so each probe row matches exactly one build row.
            // This allows us to control the emission count and span multiple budget checks.
            let build = vec![
                row(&[1, 100]),
                row(&[2, 200]),
                row(&[3, 300]),
                row(&[4, 400]),
                row(&[5, 500]),
            ];
            let t = DeltaTable::build(&build, &keys);

            // Probe with 5 rows (5 budget checks).
            let probe = vec![
                row(&[1, 0]),
                row(&[2, 0]),
                row(&[3, 0]),
                row(&[4, 0]),
                row(&[5, 0]),
            ];

            // === Full run (NoBudget control) ===
            // Should emit all 5 matches (one per probe row).
            let mut full_output: Vec<Row> = Vec::new();
            t.probe_emit(&probe, &keys, &NoBudget, &mut |b, _p| {
                full_output.push(b.clone());
            });
            assert_eq!(full_output.len(), 5, "full NoBudget run must emit all 5 matches");
            let expected_full = vec![
                row(&[1, 100]),
                row(&[2, 200]),
                row(&[3, 300]),
                row(&[4, 400]),
                row(&[5, 500]),
            ];
            assert_eq!(
                full_output, expected_full,
                "full run must preserve insertion order and deterministic multiset"
            );

            // === Budgeted run (Cap at 2 emits) ===
            // Budget is checked once per probe row. Probe row 0 emits 1 (emitted=1), probe
            // row 1 emits 1 (emitted=2), probe row 2: budget.exhausted(2)? Yes (2 >= 2).
            // Break. So we get exactly 2 emits, stopping before the third probe row.
            let mut budgeted_output: Vec<Row> = Vec::new();
            t.probe_emit(&probe, &keys, &Cap(2), &mut |b, _p| {
                budgeted_output.push(b.clone());
            });
            assert_eq!(
                budgeted_output.len(),
                2,
                "Cap(2) must truncate after 2 emits: budget.exhausted fires at the START of \
                 the third probe row (third budget check; emitted=2 >= 2), so that row is \
                 never processed"
            );
            let expected_budgeted = vec![row(&[1, 100]), row(&[2, 200])];
            assert_eq!(
                budgeted_output, expected_budgeted,
                "budgeted run must emit deterministic prefix and stop mid-enumeration"
            );

            // Verify truncation: budgeted_output is a proper prefix of full_output.
            assert_eq!(&full_output[..2], &budgeted_output[..], "budgeted output must be a prefix of full");
            assert!(
                budgeted_output.len() < full_output.len(),
                "Budget must have truncated before completion"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::smallvec;

    fn row(xs: &[Id]) -> Row {
        Row::from_slice(xs)
    }

    #[test]
    fn merge_join_single_key_matches_by_value() {
        // left (?a ?b) sorted on ?a (col 0); right (?a ?c) sorted on ?a (col 0).
        // key_cols = [(0,0)]; right_only = [1] (right's ?c).
        let left = vec![row(&[1, 10]), row(&[2, 20]), row(&[2, 21])];
        let right = vec![row(&[2, 200]), row(&[3, 300])];
        let mut out = Vec::new();
        merge_join(&left, 0, &right, 0, &[], &[1], &NoBudget, &mut out);
        // ?a=2 matches both left rows (b=20,21) with right c=200.
        assert_eq!(out.len(), 2);
        assert!(out.contains(&row(&[2, 20, 200])));
        assert!(out.contains(&row(&[2, 21, 200])));
    }

    #[test]
    fn merge_join_respects_extra_shared() {
        // A second shared variable (left col 1 vs right col 1) must also agree.
        let left = vec![row(&[1, 5]), row(&[1, 9])];
        let right = vec![row(&[1, 5, 50]), row(&[1, 7, 70])];
        let mut out = Vec::new();
        // key on col 0; extra shared (1,1); right_only = [2].
        merge_join(&left, 0, &right, 0, &[(1, 1)], &[2], &NoBudget, &mut out);
        assert_eq!(out, vec![row(&[1, 5, 50])]);
    }

    #[test]
    fn hash_join_serial_combines_build_and_probe() {
        // build (?a ?b), probe (?a ?c); key (0,0); probe_only = [1].
        let build = vec![row(&[1, 10]), row(&[2, 20]), row(&[1, 11])];
        let probe = vec![row(&[1, 100]), row(&[3, 300])];
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let tables = vec![build_table(&build, &keys)];
        let mut out = Vec::new();
        hash_probe_serial(&probe, &keys, &build, &tables, &[1], &NoBudget, &mut out);
        // ?a=1 matches build rows [1,10] and [1,11] (ascending index order), with c=100.
        assert_eq!(out, vec![row(&[1, 10, 100]), row(&[1, 11, 100])]);
    }

    #[test]
    fn budget_truncates_at_group_boundary() {
        struct Cap(usize);
        impl Budget for Cap {
            fn exhausted(&self, rows: usize) -> bool {
                rows >= self.0
            }
        }
        let left = vec![row(&[1, 1]), row(&[2, 2]), row(&[3, 3])];
        let right = vec![row(&[1, 1]), row(&[2, 2]), row(&[3, 3])];
        let mut out = Vec::new();
        // Cap at 1 row: the second key group's pre-check trips and stops.
        merge_join(&left, 0, &right, 0, &[], &[], &Cap(1), &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn lftj_two_relations_triangle_edge() {
        // Two relations over levels [0,1]: R(a,b) and S(a,b). Both share both
        // levels, so LFTJ yields the intersection of the tuple sets.
        let r = Trie { tuples: vec![vec![1, 2], vec![1, 3], vec![2, 4]] };
        let s = Trie { tuples: vec![vec![1, 2], vec![2, 4], vec![5, 6]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s)];
        // both relations participate at both levels.
        let parts_at_level = vec![vec![0, 1], vec![0, 1]];
        let mut current = vec![NO_ID, NO_ID];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 2, &mut current, &NoBudget, &mut out);
        // Intersection: {(1,2),(2,4)}.
        assert_eq!(out.len(), 2);
        assert!(out.contains(&row(&[1, 2])));
        assert!(out.contains(&row(&[2, 4])));
    }

    #[test]
    fn compatible_treats_unbound_as_wildcard() {
        // shared (0,0): left bound to 1, right unbound -> compatible.
        let l = [1u32, 9];
        let r = [NO_ID, 9];
        assert!(compatible(&l, &r, &[(0, 0)]));
        // both bound but unequal -> incompatible.
        let r2 = [2u32, 9];
        assert!(!compatible(&l, &r2, &[(0, 0)]));
    }

    #[test]
    fn merge_rows_fills_unbound_left_from_right() {
        // left col 0 unbound; shared (0,0) fills it from right; right_only = [1].
        let l = [NO_ID, 7];
        let r = [42u32, 99];
        let out = merge_rows(&l, &r, &[(0, 0)], &[1]);
        let expected: Row = smallvec![42u32, 7u32, 99u32];
        assert_eq!(out, expected);
    }

    #[test]
    fn bind_combine_appends_new_vals_per_result_row() {
        let result = vec![row(&[1, 10]), row(&[2, 20]), row(&[3, 30])];
        let mut out = Vec::new();
        // distinct value matched result rows 0 and 2; the scanned triple's new vars are [99].
        bind_combine(&result, &[0, 2], &[99], &mut out);
        assert_eq!(out, vec![row(&[1, 10, 99]), row(&[3, 30, 99])]);
    }

    // [GPT-6-ASTRA] Exercise empty, interior and full slices, repeated rows, wide
    // rows and projections, and append-to-existing-output semantics.
    #[test]
    fn bind_combine_rows_matches_indexed_groups_and_tuple_concatenation() {
        let result = vec![row(&[1, 10]), row(&[2, 20, 30, 40]), row(&[2, 20, 30, 40]), row(&[3])];
        for start in 0..=result.len() {
            for end in start..=result.len() {
                for new_vals in [&[][..], &[99][..], &[7, 8, 9][..]] {
                    let prefix = vec![row(&[42, 43])];
                    let mut sliced = prefix.clone();
                    bind_combine_rows(&result[start..end], new_vals, &mut sliced);
                    let mut indexed = prefix.clone();
                    let indices: Vec<usize> = (start..end).collect();
                    bind_combine(&result, &indices, new_vals, &mut indexed);
                    let expected: Vec<Row> = prefix.into_iter().chain(
                        result[start..end].iter().map(|r| r.iter().chain(new_vals).copied().collect())
                    ).collect();
                    assert_eq!(sliced, expected, "slice {start}..{end}, projection {new_vals:?}");
                    assert_eq!(indexed, expected, "indexed {start}..{end}, projection {new_vals:?}");
                }
            }
        }
    }

    // [OPUS-4.8] sq-qcnn.12 — DIRECT unit tests over the join kernels' descriptor projection,
    // the radix-partitioned hash build/probe path, the budget-truncation branches, the
    // merge-join advance arms, and the solution-compatibility helpers. Each asserts EXACT
    // output rows / counts so a mutation of the branch or index logic goes red.

    #[test]
    fn no_budget_never_exhausts_or_hits() {
        // The unbounded Budget / BudgetSnapshot both fold to a constant `false`.
        assert!(!Budget::exhausted(&NoBudget, 0));
        assert!(!Budget::exhausted(&NoBudget, 1_000_000));
        assert!(!BudgetSnapshot::hit(&NoBudget, 0));
        assert!(!BudgetSnapshot::hit(&NoBudget, 1_000_000));
    }

    #[test]
    fn key_hash_agrees_on_equal_keys() {
        // The only correctness requirement: equal keys must hash identically so build
        // and probe land in the same partition.  Hash collisions between distinct keys
        // are permitted — the FxHashMap in each bucket handles them correctly.
        let k1: Key = smallvec![1u32, 2u32];
        let k2: Key = smallvec![1u32, 2u32];
        assert_eq!(key_hash(&k1), key_hash(&k2)); // build & probe MUST agree
    }

    #[test]
    fn probe_key_eq_is_identical_to_key_equality_for_all_key_widths() {
        // [FABLE-5] #3644: deterministic randomized differential covering the scalar
        // one-id branch and the unchanged general fallback, including spilled keys.
        let mut state = 0x9e37_79b9_u32;
        let mut keys = Vec::new();
        for len in 0..=4 {
            for _ in 0..64 {
                let mut key = Key::new();
                for _ in 0..len {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    key.push(state % 11);
                }
                keys.push(key);
            }
        }
        for left in &keys {
            for right in &keys {
                assert_eq!(probe_key_eq(left, right), left == right);
            }
        }

        // Mutation witness: a planted scalar comparator that ignores the candidate
        // value disagrees with the reference, so the differential above is non-vacuous.
        let left = single_key(3);
        let right = single_key(7);
        let deliberately_wrong = left.len() == 1 && right.len() == 1;
        assert_ne!(deliberately_wrong, left == right);
    }

    #[test]
    fn probe_table_preserves_serial_partitioned_and_fallback_selection() {
        let serial = vec![JoinTable::default()];
        assert!(std::ptr::eq(probe_table(&serial, 17), &serial[0]));

        let partitioned: Vec<JoinTable> = (0..JOIN_PARTS).map(|_| JoinTable::default()).collect();
        for hash in 0..256_u64 {
            let expected = (hash % JOIN_PARTS as u64) as usize;
            assert!(std::ptr::eq(probe_table(&partitioned, hash), &partitioned[expected]));
        }

        // Witness the decline path: a non-standard slice retains the prior indexed
        // selection rather than entering the fixed-array optimization.
        let fallback: Vec<JoinTable> = (0..JOIN_PARTS + 1).map(|_| JoinTable::default()).collect();
        assert!(std::ptr::eq(probe_table(&fallback, 5), &fallback[5]));
    }

    #[test]
    fn join_keys_project_left_and_right_to_the_same_shape() {
        // key_cols = [(0,1),(2,3)]: left picks cols 0,2 ; right picks cols 1,3.
        let keys = JoinKeys { key_cols: vec![(0, 1), (2, 3)], right_only: vec![4] };
        let lrow = [10u32, 20, 30, 40];
        let rrow = [5u32, 10, 7, 30, 99];
        let lk = keys.left_key(&lrow);
        let rk = keys.right_key(&rrow);
        assert_eq!(lk.as_slice(), &[10u32, 30u32]);
        assert_eq!(rk.as_slice(), &[10u32, 30u32]);
        assert_eq!(lk, rk); // equal exactly when the join columns agree
    }

    #[test]
    fn merge_join_greater_arm_advances_the_right_cursor() {
        // left's first key (3) EXCEEDS right's first key (1), so the Ordering::Greater arm
        // must advance the right cursor before the keys line up at 3.
        let left = vec![row(&[3, 30])];
        let right = vec![row(&[1, 10]), row(&[3, 300])];
        let mut out = Vec::new();
        merge_join(&left, 0, &right, 0, &[], &[1], &NoBudget, &mut out);
        assert_eq!(out, vec![row(&[3, 30, 300])]);
    }

    #[test]
    fn merge_join_empty_inputs_produce_no_output() {
        let mut out = Vec::new();
        merge_join::<NoBudget>(&[], 0, &[], 0, &[], &[], &NoBudget, &mut out);
        assert!(out.is_empty());
        let left = vec![row(&[1, 10])];
        merge_join::<NoBudget>(&left, 0, &[], 0, &[], &[], &NoBudget, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn build_partitioned_covers_all_rows_across_the_radix_maps() {
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let build = vec![row(&[1, 10]), row(&[2, 20]), row(&[3, 30])];
        // The engine computes each row's partition as key_hash(left_key) % JOIN_PARTS.
        let parts: Vec<u8> = build
            .iter()
            .map(|r| (key_hash(&keys.left_key(r)) % JOIN_PARTS as u64) as u8)
            .collect();
        let tables = build_partitioned(&build, &keys, &parts);
        assert_eq!(tables.len(), JOIN_PARTS);
        // Every build row lands in exactly one partition — the union covers all 3.
        let total: usize = tables.iter().map(|t| t.values().map(|v| v.len()).sum::<usize>()).sum();
        assert_eq!(total, 3);
        // The row for key=1 is findable in its own partition.
        let part1 = (key_hash(&keys.left_key(&build[0])) % JOIN_PARTS as u64) as usize;
        let k1: Key = keys.left_key(&build[0]);
        assert_eq!(tables[part1].get(&k1), Some(&smallvec![0usize] as &Posting));
    }

    #[test]
    fn probe_emit_multi_partition_finds_the_matching_build_row() {
        // A partitioned table (len != 1) exercises the radix branch of probe_emit.
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let build = vec![row(&[1, 10]), row(&[2, 20])];
        let parts: Vec<u8> = build
            .iter()
            .map(|r| (key_hash(&keys.left_key(r)) % JOIN_PARTS as u64) as u8)
            .collect();
        let tables = build_partitioned(&build, &keys, &parts);
        let probe_row = row(&[1, 0]);
        let mut out = Vec::new();
        probe_emit(&probe_row, &keys, &build, &tables, &[], &mut out);
        assert_eq!(out, vec![row(&[1, 10])]);
        // A probe with no match emits nothing.
        let mut out2 = Vec::new();
        probe_emit(&row(&[99, 0]), &keys, &build, &tables, &[], &mut out2);
        assert!(out2.is_empty());
    }

    #[test]
    fn hash_probe_serial_budget_truncates_after_the_cap() {
        struct Cap(usize);
        impl Budget for Cap {
            fn exhausted(&self, rows: usize) -> bool {
                rows >= self.0
            }
        }
        let build = vec![row(&[1, 10]), row(&[2, 20]), row(&[3, 30])];
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let tables = vec![build_table(&build, &keys)];
        let probe = vec![row(&[1, 0]), row(&[2, 0]), row(&[3, 0])];
        let mut out = Vec::new();
        hash_probe_serial(&probe, &keys, &build, &tables, &[], &Cap(1), &mut out);
        assert_eq!(out.len(), 1); // the second probe row's pre-check trips and stops
    }

    #[test]
    fn lftj_empty_trie_yields_no_output() {
        let r = Trie { tuples: vec![] };
        let s = Trie { tuples: vec![vec![1, 2]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s)];
        let parts_at_level = vec![vec![0, 1], vec![0, 1]];
        let mut current = vec![NO_ID, NO_ID];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 2, &mut current, &NoBudget, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn lftj_budget_truncates_after_the_first_match() {
        struct Cap(usize);
        impl Budget for Cap {
            fn exhausted(&self, rows: usize) -> bool {
                rows >= self.0
            }
        }
        // Two identical relations over levels [0,1]; their intersection is all four tuples,
        // but the sticky Cap(1) budget truncates after the first full match.
        let r = Trie { tuples: vec![vec![1, 2], vec![1, 3], vec![2, 4], vec![3, 5]] };
        let s = Trie { tuples: vec![vec![1, 2], vec![1, 3], vec![2, 4], vec![3, 5]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s)];
        let parts_at_level = vec![vec![0, 1], vec![0, 1]];
        let mut current = vec![NO_ID, NO_ID];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 2, &mut current, &Cap(1), &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn merge_rows_keeps_a_bound_left_and_appends_right_only() {
        // The left's shared column is already bound (not NO_ID), so merge_rows must KEEP it
        // and ignore the right's value for that column — only right_only cols are appended.
        let l = [10u32, 7u32];
        let r = [10u32, 99u32, 42u32];
        let out = merge_rows(&l, &r, &[(0, 0)], &[2]);
        let expected: Row = smallvec![10u32, 7u32, 42u32];
        assert_eq!(out, expected);
    }

    #[test]
    fn any_unbound_detects_an_unbound_column() {
        let rows = vec![row(&[1, NO_ID, 3])];
        assert!(any_unbound(&rows, &[1])); // col 1 is unbound
        assert!(!any_unbound(&rows, &[0])); // col 0 is bound
        assert!(any_unbound(&rows, &[0, 1])); // any unbound in the set trips it
        let full = vec![row(&[1, 2, 3])];
        assert!(!any_unbound(&full, &[0, 1, 2])); // all bound
    }

    // [SONNET-4.6] sq-7d3dj.20 — DIRECT tests for the monomorphized k=3 path.

    #[test]
    fn lftj_k3_three_way_intersection() {
        // 3-way set intersection: R(x) ∧ S(x) ∧ T(x).
        // All three tries participate at level 0 (k=3), exercising the monomorphized
        // search_k3 path directly. Results must match the manual intersection.
        // [SONNET-4.6] sq-7d3dj.20
        let r = Trie { tuples: vec![vec![1], vec![2], vec![3], vec![5], vec![7]] };
        let s = Trie { tuples: vec![vec![2], vec![3], vec![5], vec![6], vec![8]] };
        let t = Trie { tuples: vec![vec![1], vec![3], vec![5], vec![7], vec![9]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s), TrieIter::new(&t)];
        let parts_at_level = vec![vec![0usize, 1, 2]];
        let mut current = vec![NO_ID; 1];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 1, &mut current, &NoBudget, &mut out);
        // Intersection: {3, 5} — values present in all three tries.
        assert_eq!(out.len(), 2, "k=3 intersection must find exactly 2 common values");
        assert!(out.contains(&row(&[3])));
        assert!(out.contains(&row(&[5])));
    }

    #[test]
    fn lftj_k3_intersection_empty_when_no_common_value() {
        // k=3 path: empty intersection. [SONNET-4.6] sq-7d3dj.20
        let r = Trie { tuples: vec![vec![1], vec![2]] };
        let s = Trie { tuples: vec![vec![3], vec![4]] };
        let t = Trie { tuples: vec![vec![5], vec![6]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s), TrieIter::new(&t)];
        let parts_at_level = vec![vec![0usize, 1, 2]];
        let mut current = vec![NO_ID; 1];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 1, &mut current, &NoBudget, &mut out);
        assert!(out.is_empty(), "k=3 path must produce empty output when no common value exists");
    }

    #[test]
    fn lftj_k3_intersection_result_equivalent_to_k2_pair_intersection() {
        // Invariant: 3-way intersection equals the pairwise intersection computed
        // sequentially. Verifies the k=3 path is semantically equivalent to the
        // generic path. [SONNET-4.6] sq-7d3dj.20
        let vals_r: Vec<u32> = (0..20).filter(|x| x % 2 == 0).collect(); // even
        let vals_s: Vec<u32> = (0..20).filter(|x| x % 3 == 0).collect(); // multiples of 3
        let vals_t: Vec<u32> = (0..20).filter(|x| x % 4 == 0).collect(); // multiples of 4

        // Expected: values divisible by lcm(2,3,4)=12 in 0..20 → {0, 12}
        let expected: Vec<Row> = vals_r
            .iter()
            .filter(|&&v| vals_s.contains(&v) && vals_t.contains(&v))
            .map(|&v| row(&[v]))
            .collect();

        let r = Trie { tuples: vals_r.iter().map(|&v| vec![v]).collect() };
        let s = Trie { tuples: vals_s.iter().map(|&v| vec![v]).collect() };
        let t = Trie { tuples: vals_t.iter().map(|&v| vec![v]).collect() };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s), TrieIter::new(&t)];
        let parts_at_level = vec![vec![0usize, 1, 2]];
        let mut current = vec![NO_ID; 1];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 1, &mut current, &NoBudget, &mut out);
        let mut out_sorted = out.clone();
        out_sorted.sort();
        let mut exp_sorted = expected.clone();
        exp_sorted.sort();
        assert_eq!(out_sorted, exp_sorted, "k=3 result must match sequential pairwise intersection");
    }

    // [SONNET-4.6] sq-qcnn.40 — targeted tests killing the surviving mutants from
    // nightly run 28776460517. Each asserts an EXACT value or outcome so a mutation
    // of the key/hash/trie logic goes red.

    /// `key_hash` must produce distinct values for distinct keys so the hash join
    /// partitions build and probe rows correctly into separate buckets.
    /// Kills: `126:5 replace key_hash -> u64 with 0` (constant hash collapses all
    /// partitions to one, making this assertion fail since equal hashes for distinct keys
    /// are not guaranteed by the function's contract but a constant-0 trivially equals 0).
    #[test]
    fn key_hash_distinct_keys_produce_distinct_hashes() {
        use smallvec::smallvec;
        let k1: Key = smallvec![1u32];
        let k2: Key = smallvec![2u32];
        let k3: Key = smallvec![100u32, 200u32];
        let k4: Key = smallvec![100u32, 201u32];
        // With a constant-0 hash, ALL of these would be equal — so assert they differ.
        assert_ne!(key_hash(&k1), key_hash(&k2), "key [1] and [2] must hash differently");
        assert_ne!(key_hash(&k3), key_hash(&k4), "key [100,200] and [100,201] must hash differently");
    }

    /// When a trie has multiple child rows for the same parent key, `TrieIter::open`
    /// must expose ALL of them at the child level. The parent-column index in `open`
    /// is `frames.len() - 1`; mutating to `/ 1` (= `frames.len()`, one-off) reads the
    /// wrong column for the grouping key, shrinking the visible child range.
    /// Kills: `401:46 replace - with / in TrieIter<'a>::open`.
    #[test]
    fn lftj_multiple_children_per_parent_key_all_emitted() {
        // Trie with a=1 having three children b=1,2,3. The leapfrog intersection of
        // two identical tries must emit all three (a=1,b=1), (a=1,b=2), (a=1,b=3).
        // Under the mutation, open() uses column 1 (the b value) as the parent grouping
        // key, which confines each iterator to only the first child row → 1 row output.
        let r = Trie { tuples: vec![vec![1, 1], vec![1, 2], vec![1, 3]] };
        let s = Trie { tuples: vec![vec![1, 1], vec![1, 2], vec![1, 3]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s)];
        let parts_at_level = vec![vec![0usize, 1], vec![0usize, 1]];
        let mut current = vec![NO_ID; 2];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 2, &mut current, &NoBudget, &mut out);
        assert_eq!(out.len(), 3, "must emit all 3 children; open() must use column 0 not 1");
        assert!(out.contains(&row(&[1, 1])));
        assert!(out.contains(&row(&[1, 2])));
        assert!(out.contains(&row(&[1, 3])));
    }

    /// The generic Leapfrog::search path (k >= 4) must advance `p` FORWARD (`+= 1`),
    /// not backward. With k=4 participants starting at distinct values the leapfrog
    /// must seek iterators in order; `p -= 1` from p=0 underflows (debug panic) or
    /// cycles backward through the wrong iterator.
    /// Kills: `489:20 replace += with -= in Leapfrog::search`.
    ///
    /// Also exercises the k=2 case where the two iterators start at different values
    /// (so the `p += 1` branch is reached inside search, unlike the existing tests
    /// where both iterators always start at the same key and `search` returns early).
    #[test]
    fn lftj_k2_different_start_keys_reaches_search_advance() {
        // R starts at 3, S starts at 5; intersection is {7}.
        // `search` must seek R from 3 to 7 (via 5, then wrap) and S from 5 to 7,
        // going through the `p += 1` branch at least once.
        let r = Trie { tuples: vec![vec![3], vec![7]] };
        let s = Trie { tuples: vec![vec![5], vec![7]] };
        let mut iters = vec![TrieIter::new(&r), TrieIter::new(&s)];
        let parts_at_level = vec![vec![0usize, 1]];
        let mut current = vec![NO_ID; 1];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 1, &mut current, &NoBudget, &mut out);
        assert_eq!(out, vec![row(&[7])]);
    }

    /// k=4 leapfrog with all four participants starting at distinct values forces
    /// multiple iterations through the `p += 1` branch of `search` (the generic path,
    /// since k != 3 bypasses `search_k3`).
    #[test]
    fn lftj_k4_intersection_via_generic_search_path() {
        // Four single-column tries; only value 10 is common to all.
        let a = Trie { tuples: vec![vec![1], vec![5], vec![10]] };
        let b = Trie { tuples: vec![vec![3], vec![7], vec![10]] };
        let c = Trie { tuples: vec![vec![4], vec![8], vec![10]] };
        let d = Trie { tuples: vec![vec![2], vec![6], vec![10]] };
        let mut iters = vec![
            TrieIter::new(&a),
            TrieIter::new(&b),
            TrieIter::new(&c),
            TrieIter::new(&d),
        ];
        let parts_at_level = vec![vec![0usize, 1, 2, 3]];
        let mut current = vec![NO_ID; 1];
        let mut out = Vec::new();
        lftj_recurse(&mut iters, &parts_at_level, 0, 1, &mut current, &NoBudget, &mut out);
        assert_eq!(out, vec![row(&[10])], "k=4 intersection must find the single common value");
    }

    // [SONNET-4.6] sq-7d3dj.19 — DIRECT tests for `probe_gather_indices` (M4 batch-emission
    // contract) and the single-hash optimisation in `probe_emit`.

    #[test]
    fn probe_gather_indices_serial_table_finds_matching_build_indices() {
        // Serial table (len == 1): probe_gather_indices must return the correct build-row indices.
        // [SONNET-4.6] sq-7d3dj.19
        let build = vec![row(&[1, 10]), row(&[2, 20]), row(&[1, 11])];
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let tables = vec![build_table(&build, &keys)];

        // key=1 matches build rows 0 and 2 (ascending index order from build_table).
        let probe_row = row(&[1, 99]);
        let mut indices = Vec::new();
        probe_gather_indices(&probe_row, &keys, &tables, &mut indices);
        assert_eq!(indices, vec![0usize, 2], "key=1 must yield build indices [0, 2] in insertion order");

        // key=2 matches build row 1 only.
        let mut indices2 = Vec::new();
        probe_gather_indices(&row(&[2, 0]), &keys, &tables, &mut indices2);
        assert_eq!(indices2, vec![1usize]);

        // key=99 has no match.
        let mut indices3 = Vec::new();
        probe_gather_indices(&row(&[99, 0]), &keys, &tables, &mut indices3);
        assert!(indices3.is_empty(), "unmatched key must yield no indices");
    }

    #[test]
    fn probe_gather_indices_partitioned_table_finds_matching_build_indices() {
        // Partitioned table (len == JOIN_PARTS): exercises the radix branch and raw_entry lookup.
        // [SONNET-4.6] sq-7d3dj.19
        let build = vec![row(&[10, 100]), row(&[20, 200]), row(&[10, 101])];
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let parts: Vec<u8> = build
            .iter()
            .map(|r| (key_hash(&keys.left_key(r)) % JOIN_PARTS as u64) as u8)
            .collect();
        let tables = build_partitioned(&build, &keys, &parts);
        assert_eq!(tables.len(), JOIN_PARTS);

        let mut indices = Vec::new();
        probe_gather_indices(&row(&[10, 0]), &keys, &tables, &mut indices);
        assert_eq!(indices, vec![0usize, 2], "key=10 must yield build indices [0, 2] from partitioned table");
    }

    #[test]
    fn probe_gather_indices_result_equivalent_to_probe_emit_indices() {
        // Load-bearing invariant: probe_gather_indices must collect exactly the build indices
        // that probe_emit materialises. Verifies the M4 batch-emission contract: the morsel
        // pipeline can gather indices via probe_gather_indices and materialise the same rows.
        // [SONNET-4.6] sq-7d3dj.19
        let build = vec![
            row(&[3, 30]),
            row(&[1, 10]),
            row(&[2, 20]),
            row(&[1, 11]),
            row(&[3, 31]),
        ];
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![1] };
        let tables = vec![build_table(&build, &keys)];

        for probe_val in [1u32, 2, 3, 99] {
            let prow = row(&[probe_val, 77]);

            // Collect build indices via probe_gather_indices.
            let mut indices = Vec::new();
            probe_gather_indices(&prow, &keys, &tables, &mut indices);

            // Materialise the combined rows manually from the gathered indices.
            let mut manual_out: Vec<Row> = Vec::with_capacity(indices.len());
            for &bi in &indices {
                let mut combined = build[bi].clone();
                combined.push(prow[1]); // probe_only = [1]
                manual_out.push(combined);
            }

            // Collect rows via probe_emit.
            let mut emit_out = Vec::new();
            probe_emit(&prow, &keys, &build, &tables, &[1], &mut emit_out);

            assert_eq!(
                manual_out, emit_out,
                "probe_gather_indices + manual materialise must equal probe_emit for key={}",
                probe_val
            );
        }
    }

    #[test]
    fn probe_emit_reserve_is_exact_for_high_fanout() {
        // Confirms the batch-reservation: for a key with N build matches, out.capacity()
        // increases by exactly N after probe_emit (no over-allocation on a hot start).
        // This is the deterministic alloc-count proof for the optimization. [SONNET-4.6] sq-7d3dj.19
        let mut build = Vec::new();
        for i in 0..20u32 {
            build.push(row(&[7, i])); // 20 build rows all share key=7
        }
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let tables = vec![build_table(&build, &keys)];

        let mut out: Vec<Row> = Vec::new();
        let cap_before = out.capacity();
        probe_emit(&row(&[7, 99]), &keys, &build, &tables, &[], &mut out);
        let cap_after = out.capacity();
        assert_eq!(out.len(), 20, "all 20 build rows must be emitted");
        // capacity must cover all 20 rows (reserve(20) was called); no under-allocation.
        assert!(cap_after >= 20, "reserve(20) must have been called: cap_after={}", cap_after);
        // and the grow should have been exactly from 0 → ≥20 (no wasted double-grow on cold start)
        assert_eq!(cap_before, 0, "output was empty before probe_emit");
    }

    // [OPUS-4.8] sq-4r8uy — the SINGLE-COLUMN JoinKeys key fast path. `left_key`/`right_key`
    // special-case `key_cols.len() == 1` (the dominant join arity) to a direct one-element
    // `Key::push` (`single_key`) instead of the general `iter().collect()`. The HARD invariant
    // is byte-identity with the general path — same length, same ids, same order — so a
    // single-column join produces the identical key on every row and the join semantics do not
    // change. These tests pin that identity so a mutation of the fast-path key derivation goes
    // red, and a differential test runs a whole single-column join old-path-vs-fast-path.

    /// The single-column fast path must produce a `Key` BYTE-IDENTICAL to the general
    /// `iter().map().collect()` path — same length (1), the same projected id in slot 0,
    /// and inline (no heap spill). This is the load-bearing derivation the fast path
    /// replaces; the differential join test below relies on it. MUTATION GUARD: if
    /// `single_key` pushed the wrong id (e.g. `id.wrapping_add(1)`, or read the wrong
    /// column), or produced a length ≠ 1, this goes red.
    #[test]
    fn single_column_key_is_byte_identical_to_the_general_collect() {
        // A general reference collect over the SAME descriptor, forced through the
        // multi-column code (bypassing the fast path) by projecting explicitly.
        let reference = |cols: &[(usize, usize)], row: &[Id], left: bool| -> Key {
            cols.iter()
                .map(|&(lc, rc)| if left { row[lc] } else { row[rc] })
                .collect()
        };
        // Distinct left/right key columns so a swapped-side bug is caught, and a
        // deliberately-unbound (NO_ID) key value so the null-key case is covered.
        for &lc in &[0usize, 1, 2] {
            for &rc in &[0usize, 1, 2] {
                let keys = JoinKeys { key_cols: vec![(lc, rc)], right_only: vec![] };
                let r = row(&[10, NO_ID, 30]);

                let fast_l = keys.left_key(&r);
                let fast_r = keys.right_key(&r);
                let ref_l = reference(&keys.key_cols, &r, true);
                let ref_r = reference(&keys.key_cols, &r, false);

                assert_eq!(fast_l.len(), 1, "single-column left key must have length 1");
                assert_eq!(fast_r.len(), 1, "single-column right key must have length 1");
                assert_eq!(fast_l, ref_l, "left_key fast path must equal the general collect (lc={}, rc={})", lc, rc);
                assert_eq!(fast_r, ref_r, "right_key fast path must equal the general collect (lc={}, rc={})", lc, rc);
                assert_eq!(fast_l.as_slice(), &[r[lc]], "left key must be exactly [row[lc]]");
                assert_eq!(fast_r.as_slice(), &[r[rc]], "right key must be exactly [row[rc]]");
                assert!(!fast_l.spilled(), "a 1-element Key must stay inline (no heap spill)");
            }
        }
    }

    /// Multi-column keys are UNCHANGED — they must not take the single-column fast path.
    /// A 2-column and a 3-column key still project all columns in `key_cols` order.
    #[test]
    fn multi_column_key_is_unaffected_by_the_fast_path() {
        let keys2 = JoinKeys { key_cols: vec![(0, 1), (2, 3)], right_only: vec![] };
        let lrow = [10u32, 20, 30, 40];
        let rrow = [5u32, 10, 7, 30];
        assert_eq!(keys2.left_key(&lrow).as_slice(), &[10u32, 30u32]);
        assert_eq!(keys2.right_key(&rrow).as_slice(), &[10u32, 30u32]);

        // 3-column key spills past the [Id; 2] inline width — the general path handles it.
        let keys3 = JoinKeys { key_cols: vec![(0, 0), (1, 1), (2, 2)], right_only: vec![] };
        let r = [1u32, 2, 3];
        let k = keys3.left_key(&r);
        assert_eq!(k.as_slice(), &[1u32, 2, 3]);
        assert!(k.spilled(), "a 3-column Key spills, exercising the general (non-fast) path");
    }

    /// **Differential test (the HARD invariant).** For a whole single-column hash join,
    /// the fast-path `left_key`/`right_key` must produce a result BYTE-IDENTICAL to a
    /// reference that forces the general multi-column collect projection — across empty
    /// inputs, a no-match probe, many-duplicate keys, and null/unbound (NO_ID) keys.
    /// The two runs differ ONLY in how the single-column key is derived, so any deviation
    /// is the fast path breaking join semantics. MUTATION GUARD: reverting `single_key`
    /// to push a wrong id makes the two result sets diverge and this goes red.
    #[test]
    fn single_column_join_is_result_identical_fast_path_vs_general_path() {
        // A reference JoinKeys whose key projection is FORCED down the general collect by
        // building the key explicitly here (never calling the fast-path methods). Same
        // build_table / probe_emit as the real path otherwise, so the ONLY difference is
        // the single-column key derivation.
        fn reference_join(
            build: &[Row],
            probe: &[Row],
            key_col: (usize, usize),
            probe_only: &[usize],
        ) -> Vec<Row> {
            // Build a table keyed on the GENERAL collect (a 1-element collect, but via the
            // multi-column iterator, exactly the old pre-fast-path behaviour), so the ONLY
            // difference from `fast_join` is the single-column key derivation.
            let general_left = |r: &[Id]| -> Key { [key_col].iter().map(|&(lc, _)| r[lc]).collect() };
            let general_right = |r: &[Id]| -> Key { [key_col].iter().map(|&(_, rc)| r[rc]).collect() };
            let mut table: std::collections::HashMap<Vec<Id>, Vec<usize>> = std::collections::HashMap::new();
            for (bi, br) in build.iter().enumerate() {
                table.entry(general_left(br).to_vec()).or_default().push(bi);
            }
            let mut out = Vec::new();
            for pr in probe {
                let key = general_right(pr).to_vec();
                if let Some(matches) = table.get(&key) {
                    let mut sorted = matches.clone();
                    sorted.sort_unstable();
                    for &bi in &sorted {
                        let mut combined = build[bi].clone();
                        for &pi in probe_only {
                            combined.push(pr[pi]);
                        }
                        out.push(combined);
                    }
                }
            }
            out
        }

        // The real fast-path join via the substrate kernels (build_table posts in ascending
        // build-index order; the reference sorts its posting to match that order).
        fn fast_join(build: &[Row], probe: &[Row], key_col: (usize, usize), probe_only: &[usize]) -> Vec<Row> {
            let keys = JoinKeys { key_cols: vec![key_col], right_only: vec![] };
            let tables = vec![build_table(build, &keys)];
            let mut out = Vec::new();
            hash_probe_serial(probe, &keys, build, &tables, probe_only, &NoBudget, &mut out);
            out
        }

        // Fixture set: (name, build, probe, key_col, probe_only).
        type JoinCase = (&'static str, Vec<Row>, Vec<Row>, (usize, usize), Vec<usize>);
        let empty: Vec<Row> = vec![];
        let cases: Vec<JoinCase> = vec![
            ("empty-both", empty.clone(), empty.clone(), (0, 0), vec![1]),
            ("empty-build", empty.clone(), vec![row(&[1, 10])], (0, 0), vec![1]),
            ("empty-probe", vec![row(&[1, 10])], empty.clone(), (0, 0), vec![1]),
            (
                "no-match",
                vec![row(&[1, 10]), row(&[2, 20])],
                vec![row(&[9, 90]), row(&[8, 80])],
                (0, 0),
                vec![1],
            ),
            (
                "many-duplicate-keys",
                vec![row(&[7, 1]), row(&[7, 2]), row(&[7, 3]), row(&[7, 4]), row(&[3, 9])],
                vec![row(&[7, 100]), row(&[3, 200]), row(&[7, 101])],
                (0, 0),
                vec![1],
            ),
            (
                "null-unbound-keys",
                vec![row(&[NO_ID, 10]), row(&[1, 11]), row(&[NO_ID, 12])],
                vec![row(&[NO_ID, 99]), row(&[1, 88])],
                (0, 0),
                vec![1],
            ),
            (
                "distinct-left-right-key-cols",
                vec![row(&[5, 1]), row(&[6, 2]), row(&[5, 3])],
                vec![row(&[0, 5]), row(&[0, 6]), row(&[0, 7])],
                (0, 1),
                vec![0],
            ),
        ];

        for (name, build, probe, key_col, probe_only) in &cases {
            let fast = fast_join(build, probe, *key_col, probe_only);
            let reference = reference_join(build, probe, *key_col, probe_only);
            assert_eq!(
                fast, reference,
                "single-column fast path must be result-identical to the general path for `{}`",
                name
            );
        }
    }

    /// The fast path and the general path must yield the same DeltaTable probe results too
    /// (the delta join also routes its key projection through `JoinKeys::{left,right}_key`).
    #[test]
    fn single_column_delta_table_matches_static_build_under_the_fast_path() {
        let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![] };
        let build = vec![row(&[1, 10]), row(&[1, 11]), row(&[2, 20])];
        let probe = vec![row(&[1, 0]), row(&[2, 0]), row(&[9, 0])];

        // Static path (build_table + probe_emit) uses the fast single-column key.
        let tables = vec![build_table(&build, &keys)];
        let mut static_out = Vec::new();
        for pr in &probe {
            probe_emit(pr, &keys, &build, &tables, &[], &mut static_out);
        }

        // Delta path routes its key projection through the same fast `left_key`/`right_key`.
        let dt = delta::DeltaTable::build(&build, &keys);
        let mut delta_out: Vec<Row> = Vec::new();
        dt.probe_emit(&probe, &keys, &NoBudget, &mut |b, _p| delta_out.push(b.clone()));

        assert_eq!(static_out, delta_out, "delta and static single-column joins must agree under the fast path");
    }
}
