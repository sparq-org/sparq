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
