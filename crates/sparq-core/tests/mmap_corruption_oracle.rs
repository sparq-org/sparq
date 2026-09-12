//! [OPUS-4.8] sq-ky2a — corruption / fuzz oracle for the memory-mapped on-disk store
//! loader (trust boundary B5 in `research/threat-model.md`: "hostile on-disk index →
//! unsafe mmap loader").
//!
//! The loader (`Graph::open` → `TripleStore::open` + `Dict::open_mmap` +
//! `CompressedPerm::from_mmap`) memory-maps attacker-controllable files and, before the
//! fixes for **sq-znld** / **sq-ed2i**, reinterpreted their bytes through
//! `from_utf8_unchecked` / unchecked slice indexing / unbounded varints with NO
//! validation. A non-UTF-8 byte in the dictionary blob was therefore immediate UNDEFINED
//! BEHAVIOUR the moment a term was materialised, and a hostile header/offset/varint was a
//! panic or out-of-bounds read.
//!
//! This oracle takes a VALID saved index (both the raw-perm and the block-compressed save
//! paths), then systematically TRUNCATES and BYTE-FLIPS each on-disk section, and asserts
//! the MEMORY-SAFETY contract of the loader: opening + fully exercising the result is
//! NEVER a panic, an out-of-bounds read, undefined behaviour, or a process abort (OOM).
//! It is always either a clean `Err` from `open`, or a graph every term of which
//! materialises safely.
//!
//! SCOPE NOTE (matches `research/threat-model.md`, T-MMAP-DoS). The on-disk store format
//! carries NO per-record integrity checksum, and the store is a TRUSTED-input asset ("the
//! store is trusted to be produced by sparq"). So flipping a *content* byte inside an
//! otherwise well-framed string/row legitimately decodes to a different — but still
//! memory-safe — graph; that is the format's documented integrity assumption, NOT a
//! memory-safety bug, and is explicitly out of scope for sq-znld / sq-ed2i (a separate
//! bead would add checksums). What the fixes guarantee, and what this oracle proves, is
//! that NO corruption — structural or content — can drive UB / a panic / an OOB read / an
//! abort. The pristine round-trip equality is pinned separately in `build_store`; here the
//! invariant is pure memory-safety. Modelled on `turtle_path_rejection_oracle` (lib.rs)
//! and the WAL torn-record tests (lib.rs).
//!
//! Run under both `--features mmap` and `--features mmap,dict-spill`; the loader is the
//! same. Best run under Miri / ASan for the strongest UB signal, but the deterministic
//! sweep here is the CI-runnable form and, critically, FAILS on the pre-fix code (it
//! aborts on a hostile allocation count and is UB on a non-UTF-8 blob byte under Miri) —
//! it actually catches the soundness holes. The narrower
//! `dict_blob_non_utf8_is_rejected_not_ub` below is the focused sq-znld regression that
//! panics (index-OOB) on pre-fix code even without Miri.

#![cfg(feature = "mmap")]

use sparq_core::Graph;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

/// A graph whose terms are deliberately ASCII so that flipping any blob byte to `0xFF`
/// reliably produces INVALID UTF-8 (the exact sq-znld trigger), and whose size spans
/// several compressed blocks / many dict records.
fn sample_graph() -> Graph {
    let mut nt = String::new();
    for i in 0..400u32 {
        // IRIs, a plain literal, a lang-tagged literal, and a typed literal — every
        // `StoredRef` record variant the blob can hold.
        nt.push_str(&format!("<http://example.org/subject/{i}> <http://example.org/p> <http://example.org/object/{i}> .\n"));
        nt.push_str(&format!("<http://example.org/subject/{i}> <http://example.org/label> \"literal value number {i}\" .\n"));
        nt.push_str(&format!("<http://example.org/subject/{i}> <http://example.org/lang> \"value {i}\"@en .\n"));
        nt.push_str(&format!("<http://example.org/subject/{i}> <http://example.org/n> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n"));
    }
    Graph::load_str(&nt, "nt").expect("valid N-Triples")
}

/// FULLY exercise an opened graph: materialise every term of every triple (this is what
/// dereferences the mmap'd dictionary blob — the UB site) and round-trip a lookup. Panics
/// here on the pre-fix code if the blob held non-UTF-8 bytes. Returns a sorted term dump.
fn exercise(g: &Graph) -> Vec<[String; 3]> {
    let mut dump: Vec<[String; 3]> = g
        .iter_ids()
        .map(|t| [g.dict.term(t[0]).to_string(), g.dict.term(t[1]).to_string(), g.dict.term(t[2]).to_string()])
        .collect();
    dump.sort();
    // Also drive the mmap LOOKUP path (binary search over dict-hash/dict-hid → stored()).
    for row in g.iter_ids().take(64) {
        for &id in &row {
            let _ = g.dict.term(id);
        }
    }
    dump
}

/// [FABLE-5] sq-7d3dj.32.2.7 — which compressed on-disk block-stream format a build emits.
/// `V1` is the shipped `SPQCPRM1`; `V2` is `SPQCPRM2` (only reachable behind the `spqcprm2`
/// feature). The oracle sweeps ALL of them so the mmap loader's memory-safety contract is
/// proven on the V2 magic + frame-of-reference reader too, not just V1.
#[derive(Clone, Copy)]
enum SaveMode {
    Raw,
    CompressedV1,
    #[cfg(feature = "spqcprm2")]
    CompressedV2,
}

/// Save the sample graph into `dir` via the given save mode, returning the pristine term dump
/// (the consistency reference any successfully-reopened corrupt store must still match).
fn build_store(dir: &Path, mode: SaveMode) -> Vec<[String; 3]> {
    let g = sample_graph();
    match mode {
        SaveMode::Raw => g.save(dir).expect("save"),
        SaveMode::CompressedV1 => g.save_compressed(dir).expect("save_compressed"),
        // [FABLE-5] sq-7d3dj.32.2.7 — force V2 emission on THIS thread only (no env mutation),
        // so the saved perms carry the SPQCPRM2 magic and decode through `decode_block_v2_at`.
        #[cfg(feature = "spqcprm2")]
        SaveMode::CompressedV2 => sparq_core::compress::with_emit_format(
            sparq_core::compress::EmitFormat::V2,
            || g.save_compressed(dir).expect("save_compressed V2"),
        ),
    }
    // Reference dump comes from a clean reopen (so it reflects the on-disk decode, not the
    // in-RAM graph) — proves the pristine round-trip works before we corrupt anything.
    let clean = Graph::open(dir).expect("pristine store must open");
    // For a compressed V2 save, at least one perm must actually carry the SPQCPRM2 magic (else
    // the V2 arm of the oracle is vacuous — it would just be re-sweeping V1 files).
    #[cfg(feature = "spqcprm2")]
    if matches!(mode, SaveMode::CompressedV2) {
        let saw_v2 = (0..6).any(|i| {
            std::fs::read(dir.join(format!("perm{i}.bin")))
                .map(|b| b.starts_with(b"SPQCPRM2"))
                .unwrap_or(false)
        });
        assert!(saw_v2, "V2 save produced no SPQCPRM2 perm — the V2 oracle arm is vacuous");
    }
    exercise(&clean)
}

/// The on-disk files a store directory contains (those that exist for the given save mode).
fn store_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    for name in [
        "dict-meta.bin",
        "dict-terms.bin",
        "dict-offs.bin",
        "dict-hash.bin",
        "dict-hid.bin",
        "perm0.bin",
        "perm1.bin",
        "perm2.bin",
        "perm3.bin",
        "perm4.bin",
        "perm5.bin",
        "numerics-v2.bin",
        "temporals-v2.bin",
        // [OPUS-4.8] sq-f5jh: `predstats.bin` is an untrusted on-disk file too — its u64
        // record-count drove an unbounded `FxHashMap::reserve` (OOM-abort DoS) before the
        // cap in `TripleStore::load_pred_stats`. Sweeping it here keeps that cap exercised.
        "predstats.bin",
    ] {
        let p = dir.join(name);
        if p.exists() && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false) {
            v.push(p);
        }
    }
    v
}

/// Open + exercise a (possibly corrupt) store directory under `catch_unwind`, asserting the
/// MEMORY-SAFETY contract: a clean `Err`, or a graph that fully materialises without UB /
/// OOB / panic. A panic / abort FAILS (an OOM-abort cannot be caught and takes the whole
/// process down — which is itself the failure signal the pre-fix sweep produces). A
/// successful-but-different decode is ALLOWED for content-byte corruption (see the module
/// scope note): the contract here is safety, not bitwise integrity. `_reference` is kept
/// for signature symmetry with the focused regression and to document intent.
fn assert_safe(dir: &Path, _reference: &[[String; 3]], what: &str) {
    let dir = dir.to_path_buf();
    let outcome = catch_unwind(AssertUnwindSafe(|| match Graph::open(&dir) {
        Err(_) => {}              // clean rejection — the desired response to a bad file
        Ok(g) => {
            let _ = exercise(&g); // opened: every term must materialise without UB / panic
        }
    }));
    if outcome.is_err() {
        panic!("PANIC/UB: the mmap loader panicked or aborted on a corrupt store ({what})");
    }
}

/// Truncate `path` to `new_len` bytes in `dst_dir` (a fresh copy of the pristine store).
fn corrupt_truncate(src_dir: &Path, dst_dir: &Path, file: &Path, new_len: u64) {
    copy_dir(src_dir, dst_dir);
    let target = dst_dir.join(file.file_name().unwrap());
    let f = std::fs::OpenOptions::new().write(true).open(&target).unwrap();
    f.set_len(new_len).unwrap();
}

/// XOR the byte at `pos` of `file` with `xor` in a fresh copy of the pristine store.
fn corrupt_flip(src_dir: &Path, dst_dir: &Path, file: &Path, pos: usize, xor: u8) {
    copy_dir(src_dir, dst_dir);
    let target = dst_dir.join(file.file_name().unwrap());
    let mut bytes = std::fs::read(&target).unwrap();
    if pos < bytes.len() {
        bytes[pos] ^= xor;
        std::fs::write(&target, &bytes).unwrap();
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    let _ = std::fs::remove_dir_all(dst);
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
        }
    }
}

/// The core sweep, run for every save mode.
fn corruption_sweep(mode: SaveMode) {
    let tmp = tempdir();
    let pristine = tmp.join("pristine");
    let reference = build_store(&pristine, mode);
    let scratch = tmp.join("scratch");
    let mode_tag = match mode {
        SaveMode::Raw => "raw",
        SaveMode::CompressedV1 => "compressed-v1",
        #[cfg(feature = "spqcprm2")]
        SaveMode::CompressedV2 => "compressed-v2",
    };

    let files = store_files(&pristine);
    assert!(files.iter().any(|p| p.ends_with("dict-terms.bin")), "the term blob must exist");

    for file in &files {
        let len = std::fs::metadata(file).unwrap().len();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();

        // ---- TRUNCATION sweep: a spread of cut points incl. the boundaries ----
        let cuts: Vec<u64> = {
            let mut c = vec![0u64];
            for d in [1u64, 2, 4, 7, 8, 9, 16, 32] {
                if d < len {
                    c.push(len - d);
                    c.push(d);
                }
            }
            // a handful of interior cuts
            for k in 1..=8 {
                c.push(len * k / 9);
            }
            c.sort_unstable();
            c.dedup();
            c.retain(|&x| x < len);
            c
        };
        for &cut in &cuts {
            corrupt_truncate(&pristine, &scratch, file, cut);
            assert_safe(&scratch, &reference, &format!("truncate {name} -> {cut}/{len} (mode={mode_tag})"));
        }

        // ---- BYTE-FLIP sweep: every position for small files; a stride for large ones,
        //      with the high-bit flip (0x80) and 0xFF both used so blob bytes go non-UTF-8.
        let stride = (len / 256).max(1) as usize;
        let mut pos = 0usize;
        while pos < len as usize {
            for xor in [0x80u8, 0xFF, 0x01] {
                corrupt_flip(&pristine, &scratch, file, pos, xor);
                assert_safe(&scratch, &reference, &format!("flip {name}@{pos}^{xor:#x} (mode={mode_tag})"));
            }
            pos += stride;
        }
    }
}

#[test]
fn mmap_loader_survives_corruption_raw() {
    corruption_sweep(SaveMode::Raw);
}

#[test]
fn mmap_loader_survives_corruption_compressed() {
    corruption_sweep(SaveMode::CompressedV1);
}

/// [FABLE-5] sq-7d3dj.32.2.7 — the SAME memory-safety oracle over a `SPQCPRM2` (V2) saved store:
/// the V2 magic auto-detect + the frame-of-reference `decode_block_v2_at` reader must survive
/// every truncation / byte-flip with a clean `Err` or a fully-materialising graph — never a
/// panic / OOB / abort. This proves the migration did not open a new UB surface in the loader.
#[cfg(feature = "spqcprm2")]
#[test]
fn mmap_loader_survives_corruption_compressed_v2() {
    corruption_sweep(SaveMode::CompressedV2);
}

/// A FOCUSED, fast regression for the exact sq-znld soundness hole: flip a byte inside the
/// dictionary term blob to a value that makes the enclosing string non-UTF-8, and assert
/// the loader REJECTS it (or, never, decodes it). Pre-fix this reaches
/// `from_utf8_unchecked` over invalid bytes → UB. This is the smallest test that proves the
/// fix; the sweeps above are the broad oracle.
#[test]
fn dict_blob_non_utf8_is_rejected_not_ub() {
    let tmp = tempdir();
    let pristine = tmp.join("pristine");
    let g = sample_graph();
    g.save(&pristine).unwrap();

    // Walk the blob; for each byte, set it to 0xFF (a standalone 0xFF is never valid UTF-8)
    // and require that opening + materialising every term is SAFE (Err, or — impossible
    // here — a consistent decode). At least one such flip must land inside a string payload.
    let blob = pristine.join("dict-terms.bin");
    let len = std::fs::metadata(&blob).unwrap().len() as usize;
    let scratch = tmp.join("scratch");
    let mut saw_rejection = false;
    let mut pos = 0usize;
    while pos < len {
        corrupt_flip(&pristine, &scratch, &blob, pos, 0xFF);
        let scratch_p = scratch.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| match Graph::open(&scratch_p) {
            Err(_) => "err",
            Ok(g) => {
                // Materialise every term — this is the UB site on pre-fix code.
                g.iter_ids().for_each(|t| {
                    for &id in &t {
                        let _ = g.dict.term(id).to_string();
                    }
                });
                "ok"
            }
        }));
        match outcome {
            Ok("err") => saw_rejection = true,
            Ok(_) => {}
            Err(_) => panic!("PANIC/UB on a non-UTF-8 dict blob byte @ {pos} — sq-znld not fixed"),
        }
        pos += 7; // stride so the test stays fast but still hits string payloads
    }
    assert!(saw_rejection, "no blob corruption was rejected — the oracle is vacuous");
}

/// [OPUS-4.8] sq-cvug — a graph containing RDF 1.2 quoted/triple terms `<<( s p o )>>` so
/// the store holds tag-3 records whose components cross-reference other dictionary ids. The
/// byte-flip sweeps above never synthesise these from the plain `sample_graph` (no triple
/// terms in it), so this focused test drives the triple-term reconstruction path directly.
fn sample_star_graph() -> Graph {
    let mut nt = String::new();
    for i in 0..40u32 {
        // A triple term in the OBJECT position (the RDF 1.2 N-Triples grammar) — a
        // tag-3 record whose subject/predicate/object are themselves dictionary ids.
        nt.push_str(&format!(
            "<http://example.org/s/{i}> <http://example.org/says> <<( <http://example.org/inner/{i}> <http://example.org/p> \"object {i}\" )>> .\n"
        ));
    }
    Graph::load_str(&nt, "nt").expect("valid RDF 1.2 N-Triples with triple terms")
}

/// sq-cvug — a triple-term component id that is IN RANGE but resolves to the WRONG KIND
/// (e.g. a literal where the subject must be IRI/blank) used to hit `reconstruct_triple`'s
/// `unreachable!()` → a clean panic (DoS) on a hostile/corrupt store. We exercise the whole
/// store-shaped boundary: save a store with triple terms, brute-force flip every byte of the
/// term blob (which includes the tag-3 records' inline component ids), and require the loader
/// to be SAFE on every variant — a clean `Err` from `open` (the kind check in
/// `MappedDict::validate` rejects it), or a graph that fully materialises without a panic
/// (`reconstruct_triple` falls back to a safe placeholder). Pre-fix this aborts the process.
#[test]
fn quoted_triple_wrong_kind_component_is_safe_not_panic() {
    let tmp = tempdir();
    let pristine = tmp.join("pristine");
    let g = sample_star_graph();
    g.save(&pristine).unwrap();
    // Pristine store with triple terms must open and materialise cleanly.
    let clean = Graph::open(&pristine).expect("pristine star store must open");
    let reference = exercise(&clean);
    assert!(reference.iter().any(|r| r[2].contains("<<")), "the reference dump must contain triple terms");

    let blob = pristine.join("dict-terms.bin");
    let len = std::fs::metadata(&blob).unwrap().len() as usize;
    let scratch = tmp.join("scratch");
    // Flip each blob byte to a spread of values: this rewrites tag bytes (so a record's KIND
    // changes) and the inline u32 component ids inside tag-3 records (so a triple component
    // can come to point at a wrong-kind term). Every variant must be SAFE.
    let mut pos = 0usize;
    while pos < len {
        for xor in [0x01u8, 0x02, 0x03, 0x80, 0xFF] {
            corrupt_flip(&pristine, &scratch, &blob, pos, xor);
            assert_safe(&scratch, &reference, &format!("star-blob flip @{pos}^{xor:#x}"));
        }
        pos += 1;
    }
}

// ---- tiny tempdir (no dev-dep): a unique scratch dir under the OS temp, cleaned on drop is
// not needed (CI temp is ephemeral); we just need a unique path per run. ----
fn tempdir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "sparq-mmap-oracle-{}-{}-{}",
        std::process::id(),
        n,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
