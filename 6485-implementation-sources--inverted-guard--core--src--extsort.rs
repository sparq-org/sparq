//! External-memory (out-of-core) index BUILD: sort/merge the triples on disk so a
//! dataset whose permutation indexes exceed RAM can be CONSTRUCTED with bounded memory
//! (only one chunk of triples + the dictionary live in RAM at once). Complements the
//! memory-mapped query path (`Graph::open`). Native only (`mmap` feature).

use crate::dict::Id;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

const TRIPLE_BYTES: usize = std::mem::size_of::<[Id; 3]>(); // 12

/// Reinterprets a triple slice as raw little-endian bytes for writing.
fn as_bytes(t: &[[Id; 3]]) -> &[u8] {
    // SAFETY: [u32;3] is plain-old-data; we only read it as bytes.
    unsafe { std::slice::from_raw_parts(t.as_ptr().cast::<u8>(), std::mem::size_of_val(t)) }
}

/// Advise the kernel to RECLAIM the `[offset, offset+len)` page range of a read-only,
/// front-to-back-consumed mmap (drop-behind), keeping merge-input residency bounded.
/// Linux: `MADV_DONTNEED` discards the resident file pages (re-faulted from the clean
/// file if ever touched again — which we never do). macOS: `MADV_DONTNEED` does NOT
/// reduce RSS for file-backed maps, so use `MADV_FREE_REUSABLE`, the Darwin idiom that
/// actually returns the pages. Both are advisory and unmapped-range-safe; failures are
/// ignored. The map is read-only and the range is already fully consumed, so no data
/// can be lost — the `unchecked_advise` contract is satisfied.
// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns
#[cfg(unix)]
fn drop_behind(map: &memmap2::Mmap, offset: usize, len: usize) {
    if len == 0 {
        return;
    }
    #[cfg(target_vendor = "apple")]
    let advice = memmap2::UncheckedAdvice::FreeReusable;
    #[cfg(not(target_vendor = "apple"))]
    let advice = memmap2::UncheckedAdvice::DontNeed;
    // SAFETY: read-only mapping, range already consumed (never read again); DontNeed/
    // FreeReusable on clean file pages only drops the resident copy.
    unsafe {
        map.unchecked_advise_range(advice, offset, len).ok();
    }
}

#[cfg(not(unix))]
fn drop_behind(_map: &memmap2::Mmap, _offset: usize, _len: usize) {}

/// Sorts a chunk (by its stored column order) and spills it to a fresh run file under
/// `tmp`, clearing the buffer. The caller pushes triples already in the desired key
/// order, so a plain lexicographic sort suffices.
pub fn spill_run(buf: &mut Vec<[Id; 3]>, runs: &mut Vec<PathBuf>, tmp: &Path) -> io::Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    // Parallel sort when available: a run is up to `chunk` (tens of millions of) triples,
    // and this sort previously sat SERIALLY inside the ingest spill stage and each
    // sibling-perm external sort. `[u32;3]` is total-ordered and equal triples are
    // byte-identical, so the run file is byte-identical to the serial `sort_unstable`.
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        buf.par_sort_unstable();
    }
    #[cfg(not(feature = "parallel"))]
    buf.sort_unstable();
    let path = tmp.join(format!("run{}.bin", runs.len()));
    std::fs::write(&path, as_bytes(buf))?;
    runs.push(path);
    buf.clear();
    Ok(())
}

/// K-way merges sorted run files, deduplicating consecutive equal triples, calling `sink`
/// for each surviving row IN SORTED ORDER. The runs are memory-mapped (paged on demand)
/// and merged through a min-heap with drop-behind, so peak RAM is the heap (one triple per
/// run) — independent of the dataset size. [OPUS-4.8] sq-vkz7: extracted from `kway_merge`
/// so the raw writer and the streaming COMPRESSED writer share one merge body.
fn kway_merge_to<S: FnMut([Id; 3]) -> io::Result<()>>(runs: &[PathBuf], mut sink: S) -> io::Result<()> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let maps: Vec<memmap2::Mmap> = runs
        .iter()
        .map(|p| {
            let f = std::fs::File::open(p)?;
            // SAFETY: the run files are written by us and not mutated during the merge.
            let m = unsafe { memmap2::Mmap::map(&f) }?;
            // Each run is consumed front-to-back exactly once: tell the kernel so it
            // reads ahead and DROPS BEHIND instead of letting every merged byte sit
            // resident — at 100M+ triples the merge inputs are GBs of file pages that
            // otherwise dominate peak RSS (advisory; failures are ignorable).
            #[cfg(unix)]
            m.advise(memmap2::Advice::Sequential).ok();
            Ok(m)
        })
        .collect::<io::Result<_>>()?;
    let slices: Vec<&[[Id; 3]]> = maps
        .iter()
        .map(|m| {
            let n = m.len() / TRIPLE_BYTES;
            // SAFETY: each run is a whole number of page-aligned [u32;3] triples.
            unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<[Id; 3]>(), n) }
        })
        .collect();

    let mut heads = vec![0usize; runs.len()];
    let mut heap: BinaryHeap<Reverse<([Id; 3], usize)>> = BinaryHeap::new();
    for (i, s) in slices.iter().enumerate() {
        if !s.is_empty() {
            heap.push(Reverse((s[0], i)));
        }
    }

    // DROP-BEHIND: each run is consumed front-to-back exactly once, so pages behind the
    // merge cursor are dead — release them in 32 MiB strides (`MADV_DONTNEED`; advisory)
    // instead of letting the whole merge input (GBs at 100M+ triples) sit resident.
    // `Advice::Sequential` alone does NOT shrink residency on macOS, measured: the
    // sibling-merge phase peaked at 5.7 GiB on a 100M-triple build without this.
    const DROP_STRIDE: usize = 32 << 20;
    let mut dropped = vec![0usize; runs.len()];

    let mut last: Option<[Id; 3]> = None;
    while let Some(Reverse((t, i))) = heap.pop() {
        if last != Some(t) {
            sink(t)?;
            last = Some(t);
        }
        heads[i] += 1;
        if heads[i] < slices[i].len() {
            heap.push(Reverse((slices[i][heads[i]], i)));
            #[cfg(unix)]
            {
                let consumed = heads[i] * TRIPLE_BYTES;
                if consumed - dropped[i] >= 2 * DROP_STRIDE {
                    // Keep one stride of headroom behind the cursor; offsets stay
                    // page-aligned because DROP_STRIDE is a multiple of the page size.
                    let upto = consumed - DROP_STRIDE;
                    drop_behind(&maps[i], dropped[i], upto - dropped[i]);
                    dropped[i] = upto;
                }
            }
        }
    }
    #[cfg(not(unix))]
    let _ = &mut dropped;
    Ok(())
}

/// K-way merges sorted run files into a RAW little-endian `[u32;3]` permutation at `out`
/// (deduplicating consecutive equal triples). See `kway_merge_to` for the shared merge
/// machinery.
pub fn kway_merge(runs: &[PathBuf], out: &Path) -> io::Result<()> {
    let mut w = BufWriter::new(std::fs::File::create(out)?);
    kway_merge_to(runs, |t| {
        for &id in &t {
            w.write_all(&id.to_le_bytes())?;
        }
        Ok(())
    })?;
    w.flush()
}

/// [OPUS-4.8] sq-vkz7 — k-way merges sorted run files into a BLOCK-COMPRESSED `SPQCPRM1`
/// permutation at `out`, streaming each deduplicated row straight into the FoR+varint
/// block encoder. This lets the external-memory build emit compressed perms in ONE pass —
/// no raw write followed by a separate `open` + `decode_all` + re-encode recompress pass
/// over the (84+ GB at 1B-scale) index. The file is byte-identical to running the raw
/// `kway_merge` then `recompress` on its output.
pub fn kway_merge_compressed(runs: &[PathBuf], out: &Path) -> io::Result<()> {
    let mut w = crate::compress::CompressedPermWriter::create(out)?;
    kway_merge_to(runs, |t| w.push(t))?;
    w.finish(out)
}

/// External-sorts the triples produced by `iter` into key order `order` (a column
/// permutation), writing the sorted result to `out` using at most `chunk` triples of
/// RAM at a time. The runs are written under `tmp`.
pub fn external_sort(
    iter: impl Iterator<Item = [Id; 3]>,
    order: [usize; 3],
    out: &Path,
    tmp: &Path,
    chunk: usize,
) -> io::Result<()> {
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut buf: Vec<[Id; 3]> = Vec::with_capacity(chunk);
    for t in iter {
        // Store each triple already in `order` so the run sort + merge compare directly.
        buf.push([t[order[0]], t[order[1]], t[order[2]]]);
        if buf.len() >= chunk {
            spill_run(&mut buf, &mut runs, tmp)?;
        }
    }
    spill_run(&mut buf, &mut runs, tmp)?;
    kway_merge(&runs, out)?;
    for r in &runs {
        std::fs::remove_file(r).ok();
    }
    Ok(())
}

/// [OPUS-4.8] sq-vkz7 — like [`external_sort`] but writes `out` BLOCK-COMPRESSED
/// (`SPQCPRM1`) via [`kway_merge_compressed`], so a sibling permutation is produced in its
/// compressed on-disk form directly from the sort tail (no recompress second pass). The
/// run-spill phase is identical; only the merge tail differs.
pub fn external_sort_compressed(
    iter: impl Iterator<Item = [Id; 3]>,
    order: [usize; 3],
    out: &Path,
    tmp: &Path,
    chunk: usize,
) -> io::Result<()> {
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut buf: Vec<[Id; 3]> = Vec::with_capacity(chunk);
    for t in iter {
        buf.push([t[order[0]], t[order[1]], t[order[2]]]);
        if buf.len() >= chunk {
            spill_run(&mut buf, &mut runs, tmp)?;
        }
    }
    spill_run(&mut buf, &mut runs, tmp)?;
    kway_merge_compressed(&runs, out)?;
    for r in &runs {
        std::fs::remove_file(r).ok();
    }
    Ok(())
}

/// Memory-maps a permutation file as a `[[Id;3]]` slice (for re-sorting into other
/// orders during the build).
pub fn map_perm(path: &Path) -> io::Result<(memmap2::Mmap, usize)> {
    let f = std::fs::File::open(path)?;
    // SAFETY: read-only mapping of a file we own for the call's duration.
    let m = unsafe { memmap2::Mmap::map(&f)? };
    // The sibling re-sorts stream this front-to-back (one pass each): prefer read-ahead
    // + drop-behind over keeping the whole permutation resident (advisory).
    #[cfg(unix)]
    m.advise(memmap2::Advice::Sequential).ok();
    let n = m.len() / TRIPLE_BYTES;
    Ok((m, n))
}

#[cfg(test)]
mod tests {
    // [OPUS-4.8] sq-j67o — behavioural tests for the out-of-core index build
    // (spill -> k-way merge -> raw/compressed write). These assert OBSERVABLE
    // behaviour: that the on-disk permutation is sorted in the requested column
    // order, that consecutive-equal triples are deduplicated, that the input is
    // physically split across multiple runs (so the MERGE path — not a single
    // in-RAM sort — is exercised), that the run files are cleaned up afterwards,
    // and that the compressed merge tail emits the canonical block archive. Before
    // this, extsort.rs was only touched transitively by the build pipeline and
    // sat at the crate's lowest line coverage.
    use super::*;

    /// A unique scratch directory under the system temp dir; removed on drop so a
    /// failing assertion does not leak run files. Follows the crate's existing
    /// `temp_dir().join(pid)` test convention (no extra dev-dependency).
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir()
                .join(format!("sparq_extsort_{tag}_{}_{n}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Reads a RAW little-endian `[u32;3]` permutation file back into a `Vec`.
    /// Reads the bytes (no mmap / no `unsafe`) and decodes each 12-byte triple.
    fn read_raw_perm(path: &Path) -> Vec<[Id; 3]> {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            bytes.len() % TRIPLE_BYTES,
            0,
            "perm file is not a whole number of triples"
        );
        bytes
            .chunks_exact(TRIPLE_BYTES)
            .map(|c| {
                [
                    Id::from_le_bytes(c[0..4].try_into().unwrap()),
                    Id::from_le_bytes(c[4..8].try_into().unwrap()),
                    Id::from_le_bytes(c[8..12].try_into().unwrap()),
                ]
            })
            .collect()
    }

    #[test]
    fn spill_run_on_empty_buffer_is_a_noop() {
        let tmp = Scratch::new("spill_empty");
        let mut buf: Vec<[Id; 3]> = Vec::new();
        let mut runs: Vec<PathBuf> = Vec::new();
        spill_run(&mut buf, &mut runs, tmp.path()).unwrap();
        // No run file is created for an empty buffer.
        assert!(runs.is_empty());
    }

    #[test]
    fn spill_run_sorts_and_writes_one_run_then_clears_buffer() {
        let tmp = Scratch::new("spill_sort");
        let mut buf: Vec<[Id; 3]> = vec![[3, 0, 0], [1, 0, 0], [2, 0, 0]];
        let mut runs: Vec<PathBuf> = Vec::new();
        spill_run(&mut buf, &mut runs, tmp.path()).unwrap();
        // Buffer is drained, exactly one run file exists, and it is sorted.
        assert!(buf.is_empty());
        assert_eq!(runs.len(), 1);
        assert!(runs[0].exists());
        assert_eq!(
            read_raw_perm(&runs[0]),
            vec![[1, 0, 0], [2, 0, 0], [3, 0, 0]]
        );
    }

    #[test]
    fn external_sort_reorders_into_requested_column_order_and_dedups() {
        let tmp = Scratch::new("ext_reorder");
        let out = tmp.path().join("perm.bin");
        // Triples in SPO terms. Includes an exact duplicate ([5,1,9] twice) and
        // rows that only sort correctly under the POS order [1,2,0].
        let input = [
            [5u32, 1, 9],
            [1, 3, 2],
            [5, 1, 9], // dup of row 0
            [4, 1, 0],
            [2, 3, 1],
        ];
        // Sort by predicate, then object, then subject: order = [1,2,0].
        let order = [1usize, 2, 0];
        // chunk = 2 forces 3 runs => the k-way MERGE path (not a single sort) runs.
        external_sort(input.iter().copied(), order, &out, tmp.path(), 2).unwrap();

        let got = read_raw_perm(&out);

        // Expected: each triple permuted into `order`, deduplicated, fully sorted.
        let mut expected: Vec<[Id; 3]> = input
            .iter()
            .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
            .collect();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(got, expected);
        // Sanity: the duplicate collapsed (5 inputs, 1 dup => 4 rows).
        assert_eq!(got.len(), 4);

        // Run files were cleaned up — only `out` remains in tmp.
        let leftover_runs: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("run"))
            .collect();
        assert!(leftover_runs.is_empty(), "run files were not removed");
    }

    #[test]
    fn external_sort_on_empty_input_writes_empty_perm() {
        let tmp = Scratch::new("ext_empty");
        let out = tmp.path().join("empty.bin");
        external_sort(std::iter::empty(), [0, 1, 2], &out, tmp.path(), 8).unwrap();
        assert_eq!(read_raw_perm(&out), Vec::<[Id; 3]>::new());
    }

    #[test]
    fn compressed_external_sort_matches_canonical_block_encoding() {
        let tmp = Scratch::new("ext_cmp");
        // A larger, repetitive input so the FoR+varint block encoder is exercised
        // across more than one block and dedup actually fires.
        let mut input: Vec<[Id; 3]> = Vec::new();
        for s in 0..40u32 {
            for p in 0..3u32 {
                input.push([s, p, s * 7 + p]);
                input.push([s, p, s * 7 + p]); // every triple duplicated
            }
        }
        let order = [0usize, 1, 2];

        let raw_out = tmp.path().join("raw.bin");
        let cmp_out = tmp.path().join("cmp.bin");
        external_sort(input.iter().copied(), order, &raw_out, tmp.path(), 16).unwrap();
        external_sort_compressed(input.iter().copied(), order, &cmp_out, tmp.path(), 16).unwrap();

        // The raw tail is the ground truth: fully sorted + deduplicated.
        let raw = read_raw_perm(&raw_out);
        let mut expected = input.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(raw, expected);
        // Every triple was duplicated, so the result is exactly half the input size.
        assert_eq!(raw.len(), input.len() / 2);

        // The COMPRESSED tail must be byte-identical to encoding those same sorted,
        // deduplicated rows through the canonical in-RAM `CompressedPerm` encoder —
        // i.e. the streaming merge-write produced exactly the standard archive, so a
        // later `from_mmap` would decode it back to `expected`. (Asserting the bytes
        // avoids re-mapping the file here.)
        let cmp_bytes = std::fs::read(&cmp_out).unwrap();
        let mut canonical = Vec::new();
        crate::compress::CompressedPerm::encode(&expected)
            .write_to(&mut canonical)
            .unwrap();
        assert_eq!(
            cmp_bytes, canonical,
            "compressed tail must equal the canonical block encoding"
        );
    }

    #[test]
    fn kway_merge_combines_multiple_presorted_runs() {
        let tmp = Scratch::new("kway");
        let mut runs: Vec<PathBuf> = Vec::new();
        // Two runs whose sorted order interleaves, with a cross-run duplicate.
        let mut a: Vec<[Id; 3]> = vec![[1, 0, 0], [4, 0, 0], [7, 0, 0]];
        let mut b: Vec<[Id; 3]> = vec![[2, 0, 0], [4, 0, 0], [9, 0, 0]];
        spill_run(&mut a, &mut runs, tmp.path()).unwrap();
        spill_run(&mut b, &mut runs, tmp.path()).unwrap();
        assert_eq!(runs.len(), 2);

        let out = tmp.path().join("merged.bin");
        kway_merge(&runs, &out).unwrap();
        // Interleaved, with the cross-run [4,0,0] duplicate collapsed to one.
        assert_eq!(
            read_raw_perm(&out),
            vec![[1, 0, 0], [2, 0, 0], [4, 0, 0], [7, 0, 0], [9, 0, 0]]
        );
    }

    #[test]
    fn map_perm_reports_the_triple_count() {
        let tmp = Scratch::new("map_count");
        let out = tmp.path().join("count.bin");
        let input = [[1u32, 2, 3], [4, 5, 6], [7, 8, 9]];
        external_sort(input.iter().copied(), [0, 1, 2], &out, tmp.path(), 8).unwrap();
        let (_map, n) = map_perm(&out).unwrap();
        assert_eq!(n, 3);
    }
}
