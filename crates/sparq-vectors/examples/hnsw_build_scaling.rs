//! [SONNET-4.6] (sq-pm6i2, follow-up sq-ose80.2) **BUILD-TIME-ONLY** HNSW scaling harness.
//!
//! `research/gap-vector-ann-simd-2026-07.md` §7.3 carries a 1M×128 build-time
//! **extrapolation** (from a measured 200k curve), not a measured 1M point: the full 1M run
//! that would have produced one was abandoned under disk pressure, and the *recall* half of
//! that harness — the brute-force `nearest_exact` oracle, which is O(n_query × n_base) — is
//! what makes a 1M run expensive in the first place. This harness drops the oracle entirely
//! and measures **only** what §7.3 needs: the wall-clock cost of
//! [`VectorIndex::build_with`] at a given `(n, ef_construction)`.
//!
//! It measures **nothing else**. There is no recall column, no QPS column, and no query loop
//! — so it makes NO accuracy claim of any kind. The recall side of the same knob is already
//! gated by `tests/recall.rs::build_time_presets_preserve_the_recall_floor`, and the
//! canonical recall+QPS 1M run stays its own bead (`sq-hmd7l.26`). Use `sift_ef_sweep` when
//! you want recall/QPS; use this when you want build time and nothing else.
//!
//! ## Honesty
//!
//! Wall-clock build time is **NON-CANONICAL** unless this ran on the dedicated quiet bench
//! box under the quiet-box protocol — a shared/work box's numbers are indicative only, and
//! the *ranking* of the `ef_construction` levels transfers where the absolute seconds do
//! not. The emitted footer records the corpus, the dimension, and the thread count so a
//! result can never be read without its provenance. Do NOT copy a number out of this
//! harness into markdown (AGENTS.md: no hard-coded perf numbers).
//!
//! ## Corpus
//!
//! SIFT1M is **not redistributable in-repo**, so the base vectors are supplied by the
//! operator as a raw f32 binary — the same format `sift_ef_sweep` reads. Note the converter
//! that produces it from the TEXMEX `.fvecs` distribution is **not committed** (the `.fvecs`
//! parser in `scripts/bench-adapters/vector_lib_adapter.py::read_vecs` is the closest thing
//! in-tree); the layout is:
//!
//! ```text
//! bytes 0..4   n:   u32 LE  — number of vectors
//! bytes 4..8   dim: u32 LE  — dimension
//! bytes 8..    data: [n * dim] f32 LE
//! ```
//!
//! Vectors are streamed from the file straight into the `.spqv` store one at a time, so a
//! 1M×128 corpus never lands in RAM as a second copy. `--smoke` swaps the file for a tiny
//! deterministic synthetic corpus purely to prove the harness runs on any box; a synthetic
//! run is labelled as such in the footer and is NOT a SIFT measurement.
//!
//! ## Usage
//!
//! ```sh
//! # self-test — no dataset needed, seconds
//! cargo run --release -p sparq-vectors --example hnsw_build_scaling --features approx-ann -- --smoke
//!
//! # the sq-ose80.2 measurement: 1M SIFT base vectors, the three shipped presets
//! cargo run --release -p sparq-vectors --example hnsw_build_scaling --features approx-ann -- \
//!     /data/ann/sift/base.bin 1000000 40,100,200
//!
//! # the scaling curve in one invocation (N list), on a scratch disk with room for the store
//! SPARQ_VECTORS_TMP=/data/scratch cargo run --release -p sparq-vectors \
//!     --example hnsw_build_scaling --features approx-ann -- /data/ann/sift/base.bin 200000,1000000
//! ```
//!
//! `$SPARQ_VECTORS_TMP` overrides where the temporary `.spqv` store is written (the store is
//! `n × dim × 4` bytes — ~512 MB at 1M×128 — and §7.3's run died on a full `/tmp`). The store
//! is removed after each `n`.
//!
//! Output is TSV on stdout, one row per `(n, ef_construction)`, flushed as it is produced so
//! a long run reports incrementally:
//!
//! ```text
//! n  dim  ef_construction  preset  store_s  build_s  vec_per_s
//! ```
//!
//! The corpus is loaded **once per `n`** and reused across the `ef_construction` levels, so
//! `store_s` is that one-time load repeated on each of the `n`'s rows — only `build_s` varies
//! with `ef_construction`. Each level's graph is dropped before the next one is built, so peak
//! RSS is one graph rather than the whole sweep's worth.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use sparq_vectors::{HnswConfig, VectorIndex, VectorStore};

/// Corpus size for `--smoke`: large enough to exercise a real multi-layer graph build, small
/// enough to finish in seconds on any box.
const SMOKE_N: usize = 2_000;
/// Dimensionality for `--smoke` (matches the crate's gate-test corpora).
const SMOKE_DIM: usize = 32;
/// The default `n` sweep — the single 1M point sq-ose80.2 asks for.
const DEFAULT_N: usize = 1_000_000;
/// The default `ef_construction` sweep — the three shipped presets (`fast_build`, the
/// `Default`, `high_recall`), i.e. exactly the three levels §7.3 tabulates at 200k.
const DEFAULT_EFC: [usize; 3] = [40, 100, 200];

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The shipped preset for an `ef_construction` level, plus its name. Using the preset
/// constructors (rather than a hand-built config) means the measured path IS the path a
/// caller who opts into `HnswConfig::fast_build()` gets.
fn config_for(efc: usize) -> (HnswConfig, &'static str) {
    match efc {
        40 => (HnswConfig::fast_build(), "fast_build"),
        100 => (HnswConfig::default(), "default"),
        200 => (HnswConfig::high_recall(), "high_recall"),
        _ => (HnswConfig { ef_construction: efc, ..HnswConfig::default() }, "custom"),
    }
}

/// Where the temporary `.spqv` store is written. `$SPARQ_VECTORS_TMP` exists because the
/// store is ~512 MB at 1M×128 and the §7.3 run was killed by a full `/tmp`.
fn store_dir() -> PathBuf {
    std::env::var_os("SPARQ_VECTORS_TMP").map_or_else(std::env::temp_dir, PathBuf::from)
}

/// Stream up to `n_max` vectors from a raw f32 binary into a fresh `.spqv` store.
///
/// Returns the store, the number of vectors actually written, and the dimension. Vectors are
/// read and written one at a time — a 1M×128 corpus is never materialised in RAM.
/// Zero-norm vectors are skipped (the store rejects them) and counted on stderr.
fn store_from_file(path: &str, n_max: usize, store_path: &Path) -> (VectorStore, usize, usize) {
    let f = File::open(path).unwrap_or_else(|e| panic!("open {}: {}", path, e));
    let mut r = BufReader::with_capacity(1 << 20, f);

    let mut hdr = [0u8; 8];
    r.read_exact(&mut hdr).unwrap_or_else(|e| panic!("read header of {}: {}", path, e));
    let n_total = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
    let dim = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
    assert!(dim > 0, "{} declares dim=0", path);
    let n = n_total.min(n_max);
    eprintln!(
        "[build-scaling] {}: n_total={} dim={} — streaming first n={}",
        path, n_total, dim, n
    );

    let mut store = VectorStore::create(store_path, dim)
        .unwrap_or_else(|e| panic!("create store at {}: {}", store_path.display(), e));
    let mut raw = vec![0u8; dim * 4];
    let mut vec = vec![0f32; dim];
    let mut written = 0usize;
    let mut skipped = 0usize;
    for i in 0..n {
        r.read_exact(&mut raw).unwrap_or_else(|e| panic!("read vector {} of {}: {}", i, path, e));
        for (dst, src) in vec.iter_mut().zip(raw.chunks_exact(4)) {
            *dst = f32::from_le_bytes(src.try_into().unwrap());
        }
        if vec.iter().all(|x| *x == 0.0) {
            skipped += 1;
            continue;
        }
        store.put(i as u32, &vec).unwrap_or_else(|e| panic!("put vector {}: {}", i, e));
        written += 1;
    }
    if skipped > 0 {
        eprintln!(
            "[build-scaling] skipped {} zero-norm vector(s) (the store rejects them)",
            skipped
        );
    }
    store.finalize().unwrap_or_else(|e| panic!("finalize store: {}", e));
    (store, written, dim)
}

/// A tiny deterministic synthetic corpus for `--smoke`. NOT a SIFT measurement — the footer
/// labels any run that uses it `corpus=synthetic-splitmix64`.
fn store_synthetic(n: usize, dim: usize, store_path: &Path) -> VectorStore {
    let mut store = VectorStore::create(store_path, dim)
        .unwrap_or_else(|e| panic!("create store at {}: {}", store_path.display(), e));
    let mut state = 0xC0FF_EE00_u64;
    for i in 0..n {
        // Offset away from zero so no vector is degenerate (the store rejects zero vectors).
        let v: Vec<f32> = (0..dim)
            .map(|_| ((splitmix64(&mut state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 0.5)
            .collect();
        store.put(i as u32, &v).unwrap_or_else(|e| panic!("put vector {}: {}", i, e));
    }
    store.finalize().unwrap_or_else(|e| panic!("finalize store: {}", e));
    store
}

fn parse_list(arg: Option<String>, default: &[usize]) -> Vec<usize> {
    match arg {
        None => default.to_vec(),
        Some(s) => {
            let parsed: Vec<usize> = s
                .split(',')
                .map(|p| {
                    p.trim()
                        .parse()
                        .unwrap_or_else(|_| panic!("not a number in list '{}': '{}'", s, p))
                })
                .collect();
            assert!(!parsed.is_empty(), "empty list '{}'", s);
            parsed
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().map(String::as_str);
    let smoke = first == Some("--smoke");
    if !smoke && !matches!(first, Some(a) if !a.starts_with('-')) {
        // No corpus, or a flag we do not recognise (`--help` included).
        eprintln!("usage: hnsw_build_scaling <base.bin|--smoke> [n[,n...]] [ef_construction[,...]]");
        eprintln!("  base.bin  raw f32 corpus — u32 LE n, u32 LE dim, then n*dim f32 LE");
        eprintln!("  --smoke   tiny synthetic corpus (self-test; NOT a SIFT measurement)");
        eprintln!("  $SPARQ_VECTORS_TMP  directory for the temporary .spqv store");
        std::process::exit(2);
    }
    let base_path = if smoke { String::new() } else { args[0].clone() };
    let n_list = parse_list(args.get(1).cloned(), if smoke { &[SMOKE_N] } else { &[DEFAULT_N] });
    let efc_list = parse_list(args.get(2).cloned(), &DEFAULT_EFC);

    let dir = store_dir();
    let threads = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);
    let mut out = std::io::stdout();
    println!("n\tdim\tef_construction\tpreset\tstore_s\tbuild_s\tvec_per_s");
    let _ = out.flush();

    for &n in &n_list {
        let store_path = dir.join(format!("hnsw-build-scaling-{}-{}.spqv", n, std::process::id()));
        let _ = std::fs::remove_file(&store_path);

        eprintln!("[build-scaling] n={}: writing store to {}", n, store_path.display());
        let t_store = Instant::now();
        let (store, n_actual, dim) = if smoke {
            (store_synthetic(n, SMOKE_DIM, &store_path), n, SMOKE_DIM)
        } else {
            store_from_file(&base_path, n, &store_path)
        };
        let store_s = t_store.elapsed().as_secs_f64();
        eprintln!(
            "[build-scaling] store ready: {} vectors (requested {}), dim={}, in {:.2}s",
            n_actual, n, dim, store_s
        );

        for &efc in &efc_list {
            let (cfg, preset) = config_for(efc);
            eprintln!("[build-scaling] n={} efc={} ({}): building HNSW...", n_actual, efc, preset);
            let t_build = Instant::now();
            let index = VectorIndex::build_with(&store, cfg);
            let build_s = t_build.elapsed().as_secs_f64();
            // Drop the graph before the next level so one level's index never overlaps the
            // next level's build (peak RSS is ~one graph, not the whole sweep's worth).
            drop(std::hint::black_box(index));

            let vec_per_s = if build_s > 0.0 { n_actual as f64 / build_s } else { 0.0 };
            println!(
                "{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.0}",
                n_actual, dim, efc, preset, store_s, build_s, vec_per_s
            );
            let _ = out.flush();
            eprintln!(
                "[build-scaling] n={} efc={}: build_s={:.2} vec_per_s={:.0}",
                n_actual, efc, build_s, vec_per_s
            );
        }

        drop(store);
        let _ = std::fs::remove_file(&store_path);
    }

    let corpus = if smoke { "synthetic-splitmix64".to_string() } else { base_path };
    println!(
        "# corpus={} threads={} metric=build_time_only recall=NOT_MEASURED \
         note=NON-CANONICAL unless run on the dedicated quiet bench box",
        corpus, threads
    );
    let _ = out.flush();
}
