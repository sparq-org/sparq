//! [SONNET-4.6] (#3517) How much of an HDT load is CRC verification?
//!
//! The direct decoder (`decode::graph_from_reader`) verifies every section's CRC —
//! CRC8 over each section's small header, CRC32-C (Castagnoli) over each section's
//! body. A proposed `load_unchecked` entry point would skip that. Turning integrity
//! checking off is a footgun, so it is only worth building if verification is a
//! *meaningful* share of load wall time, and only for the CRCs sparq can actually
//! reach. This example measures both instead of guessing; the verdict it produced is
//! recorded in `src/decode.rs` (module docs) and `UPSTREAM.md`.
//!
//! What it measures, and why each number is trustworthy:
//!
//!  1. **Load wall time**, from memory (no page-cache/syscall variance), plus the
//!     decoder's own `StageTimings` split so the CRC-carrying stages are visible.
//!  2. **The CRC32-C pass**, using the exact `crc` construct both `decode.rs` and the
//!     upstream `hdt` reader use, over the whole archive. This is sound as a total:
//!     the CRC32 bodies cover essentially every archive byte exactly once (the four
//!     PFC dictionary blobs and their offset sequences, the two triple bitmaps, the
//!     two triple sequences); only the control-info/header text and the CRC bytes
//!     themselves sit outside one. CRC8 covers a handful of header bytes per section
//!     and is timed too, so nothing is hand-waved.
//!  3. **The section byte split**, re-read with the same upstream readers the decoder
//!     uses. This bounds what a skip could reach: sparq computes the CRCs for the two
//!     triple bitmaps itself, but the dictionary and the triple sequences are read by
//!     upstream `DictSectPFC::read` / `Sequence::read`, which verify unconditionally.
//!  4. **The same CRC32-C with slice-by-16 tables** — identical algorithm, identical
//!     checksum, verification fully intact — as the alternative to skipping.
//!
//! Usage — generate the archive with the load benchmark first, or pass any `.hdt`:
//! ```sh
//! cargo run --release -p sparq-hdt --example bench_load        # writes bench-data/bench.hdt
//! cargo run --release -p sparq-hdt --example bench_crc_share
//! cargo run --release -p sparq-hdt --example bench_crc_share -- /path/to/other.hdt
//! ```
//!
//! Timings are wall-clock on the running host: ADVISORY and NON-CANONICAL. Nothing
//! here is committed or asserted against.

use hdt::containers::{Bitmap, ControlInfo, Sequence};
use hdt::four_sect_dict::FourSectDict;
use hdt::header::Header;
use std::io::{BufRead, Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

const RUNS: usize = 7;

fn main() {
    let path: PathBuf = match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("bench-data")
            .join("bench.hdt"),
    };
    if !path.exists() {
        eprintln!(
            "no archive at {}\nrun `cargo run --release -p sparq-hdt --example bench_load` \
             first (it generates bench-data/bench.hdt), or pass a .hdt path",
            path.display()
        );
        std::process::exit(2);
    }

    let bytes = std::fs::read(&path).expect("read archive");
    let total = bytes.len();
    println!("archive: {} ({total} bytes)", path.display());

    // --- 1. load wall time + the decoder's own stage split -------------------------
    // From memory, not from the file: this isolates decode CPU from page-cache and
    // syscall variance, which is the quantity the CRC share is a share OF.
    let mut triples = 0;
    let mut stages = sparq_hdt::StageTimings::default();
    let load = best_of(|| {
        let mut t = sparq_hdt::StageTimings::default();
        let g = sparq_hdt::graph_from_reader_timed(Cursor::new(&bytes), &mut t).expect("load");
        triples = g.store.len();
        stages = t;
        g
    });
    println!(
        "\nload (direct decoder): {load:.4}s  ({triples} triples, {:.0} triples/s)",
        triples as f64 / load
    );
    println!("  stage dict      {:.4}s", stages.dict.as_secs_f64());
    println!("  stage scan_read {:.4}s", stages.scan_read.as_secs_f64());
    println!("  stage scan_walk {:.4}s", stages.scan_walk.as_secs_f64());
    println!("  stage build     {:.4}s", stages.build.as_secs_f64());

    // --- 2. the CRC work itself ----------------------------------------------------
    // Table construction is inside the timer because the decoder pays it per section.
    let crc32 = best_of(|| crc32c_table1(&bytes));
    println!(
        "\nCRC32-C over all {total} bytes: {crc32:.4}s  ({:.1} MB/s)",
        total as f64 / crc32 / 1e6
    );
    // CRC8 covers only each section's small header, so a CRC8 over 1 KiB is an
    // over-estimate of every section header in the archive combined.
    let crc8 = best_of(|| {
        let c = crc::Crc::<u8>::new(&crc::CRC_8_SMBUS);
        let mut d = c.digest();
        d.update(&bytes[..total.min(1024)]);
        d.finalize()
    });
    println!("CRC8 over 1 KiB (>= every section header combined): {crc8:.6}s");
    println!(
        "\n=> CRC verification is ~{:.1}% of direct-decode load wall time (upper bound: the \
         CRC32\n   pass covers the FULL archive, the decoder's CRC32 bodies do not)",
        (crc32 + crc8) / load * 100.0
    );

    // --- 3. what a skip could actually reach ---------------------------------------
    let split = section_bytes(&bytes);
    println!("\nsection byte split (which reader computes each CRC):");
    println!(
        "  dictionary       upstream FourSectDict::read  {:>10} bytes  {:>5.1}%",
        split.dict,
        pct(split.dict, total)
    );
    println!(
        "  triple bitmaps   sparq read_bitmap_words      {:>10} bytes  {:>5.1}%",
        split.bitmaps,
        pct(split.bitmaps, total)
    );
    println!(
        "  triple sequences upstream Sequence::read      {:>10} bytes  {:>5.1}%",
        split.sequences,
        pct(split.sequences, total)
    );
    // Timed directly, not extrapolated: the CRC pass over exactly the bitmap bytes.
    let crc32_bitmaps = best_of(|| crc32c_table1(&bytes[..split.bitmaps.min(total)]));
    println!(
        "=> the only CRCs sparq can skip without forking upstream are the bitmaps':\n   \
         ~{crc32_bitmaps:.4}s = ~{:.2}% of load",
        crc32_bitmaps / load * 100.0
    );

    // --- 4. the integrity-preserving alternative -----------------------------------
    let crc32_t16 = best_of(|| crc32c_table16(&bytes));
    // The comparison is only honest if the two agree on this exact archive.
    assert_eq!(crc32c_table1(&bytes), crc32c_table16(&bytes));
    println!(
        "\nsame CRC32-C, slice-by-16 tables (same checksum, verification intact): \
         {crc32_t16:.4}s\n   ({:.1} MB/s, {:.1}x the byte-at-a-time default) — i.e. most of the \
         CRC cost is the\n   table width, not the checking",
        total as f64 / crc32_t16 / 1e6,
        crc32 / crc32_t16
    );
}

/// The CRC32-C construct `decode.rs` and the upstream `hdt` reader both use: the `crc`
/// crate's default byte-at-a-time `Table<1>` over `CRC_32_ISCSI`.
fn crc32c_table1(data: &[u8]) -> u32 {
    let c = crc::Crc::<u32>::new(&crc::CRC_32_ISCSI);
    let mut d = c.digest();
    d.update(data);
    d.finalize()
}

/// The same CRC32-C via slice-by-16 tables: same algorithm, same checksum, wider table.
fn crc32c_table16(data: &[u8]) -> u32 {
    let c = crc::Crc::<u32, crc::Table<16>>::new(&crc::CRC_32_ISCSI);
    let mut d = c.digest();
    d.update(data);
    d.finalize()
}

/// Best-of-`RUNS` wall time of `f`, in seconds. Best-of (not mean) for the same reason
/// `bench_load` uses it: it is the least noise-contaminated estimate of the work done.
fn best_of<T>(mut f: impl FnMut() -> T) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..RUNS {
        let t = Instant::now();
        let out = f();
        let dt = t.elapsed().as_secs_f64();
        // Keep the result alive across the timer so the work cannot be elided.
        std::hint::black_box(out);
        best = best.min(dt);
    }
    best
}

fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 / whole as f64 * 100.0
}

struct SectionBytes {
    dict: usize,
    bitmaps: usize,
    sequences: usize,
}

/// Re-reads the archive with the same upstream readers the decoder uses, recording how
/// many bytes each section consumed.
fn section_bytes(bytes: &[u8]) -> SectionBytes {
    let mut r = Counting {
        inner: Cursor::new(bytes),
        n: 0,
    };
    ControlInfo::read(&mut r).expect("global control info");
    Header::read(&mut r).expect("header");
    let after_header = r.n;
    FourSectDict::read(&mut r).expect("dictionary");
    let after_dict = r.n;
    ControlInfo::read(&mut r).expect("triples control info");
    let after_triples_ci = r.n;
    Bitmap::read(&mut r).expect("bitmap_y");
    Bitmap::read(&mut r).expect("bitmap_z");
    let after_bitmaps = r.n;
    Sequence::read(&mut r).expect("sequence_y");
    Sequence::read(&mut r).expect("sequence_z");
    SectionBytes {
        dict: after_dict - after_header,
        bitmaps: after_bitmaps - after_triples_ci,
        sequences: r.n - after_bitmaps,
    }
}

/// A `BufRead` that counts the bytes handed out, so the archive's section layout can be
/// measured by re-reading it with the same upstream readers the decoder uses.
struct Counting<R> {
    inner: R,
    n: usize,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.n += n;
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Counting<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
        self.n += amt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The section split is the number the "what can a skip reach" verdict rests on, so
    /// it is pinned against the checked-in fixture: every section must be non-empty and
    /// the three must fit inside the archive.
    #[test]
    fn section_split_covers_the_fixture() {
        let bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("snikmeta.hdt"),
        )
        .expect("fixture");
        let s = section_bytes(&bytes);
        assert!(s.dict > 0 && s.bitmaps > 0 && s.sequences > 0);
        assert!(s.dict + s.bitmaps + s.sequences <= bytes.len());
        // The dictionary dominates a real archive — the reason a sparq-side CRC skip,
        // which can only reach the bitmaps, is bounded to noise.
        assert!(s.dict > s.bitmaps);
    }

    /// `Counting` must report exactly the bytes consumed, whichever `Read`/`BufRead`
    /// path the upstream reader takes; the split is meaningless otherwise.
    #[test]
    fn counting_reader_counts_both_paths() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut r = Counting {
            inner: Cursor::new(&data[..]),
            n: 0,
        };
        let mut buf = [0u8; 3];
        r.read_exact(&mut buf).unwrap();
        assert_eq!(r.n, 3);
        let got = r.fill_buf().unwrap().len();
        assert_eq!(r.n, 3, "fill_buf alone must not advance the count");
        r.consume(got);
        assert_eq!(r.n, 8);
    }

    /// Both CRC32-C implementations must agree — the slice-by-16 alternative is only a
    /// candidate because it is the SAME checksum, not a weaker one.
    #[test]
    fn table1_and_table16_agree() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(crc32c_table1(&data), crc32c_table16(&data));
        // Against the published CRC-32C check vector, so a bug shared by both
        // implementations cannot make this test pass vacuously.
        assert_eq!(crc32c_table1(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn pct_is_a_percentage_and_zero_safe() {
        assert!((pct(1, 4) - 25.0).abs() < 1e-9);
        assert_eq!(pct(0, 0), 0.0);
    }
}
