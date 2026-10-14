//! (sq-lrp9) HDT benchmark ORACLE — the self-asserting TSV-emitting
//! runner `bench/hdt/run.sh` calls (the sparq-hdt analogue of FTS's `bench_text`
//! and RSP's `rsp_oracle`; the crate is isolated — not a `sparq-cli` dependency —
//! so the runner is a crate EXAMPLE).
//!
//! Design: `research/capability-benchmark-program.md` §3.7. The suite compares
//! **LOAD-AND-DECODE-TO-NATIVE + triple-pattern resolution** on a FIXED `.hdt`
//! archive. That is the ONLY like-for-like axis vs hdt-cpp / hdt-java:
//! `sparq-hdt` decodes HDT into sparq's own `Dict`/`Graph` (HDT as an *ingest
//! format*) then queries its own permutation indexes, whereas hdt-cpp/java query
//! the compressed BitmapTriples in place (HDT as a *queryable store*) — so a
//! "query over HDT" head-to-head is NOT like-for-like and is an explicit NON-goal
//! (see README.md). We gate/compare LOAD+DECODE only.
//!
//! Two metric families, emitted as `<metric>\t<value>\t<unit>`:
//!
//!   * DETERMINISTIC gate (`*_count` / `*_triples` / `*_terms` / `*_predicates`,
//!     unit `count`) — asserted by `run.sh` against `bench/hdt/expected.tsv` (exit
//!     1 on any drift). Box-contention-immune, the REAL gate. Derived from the
//!     vendored real-world `snikmeta.hdt` (NOT produced by this code path; the
//!     `hdt` crate's own tests assert its 328 triples), plus a direct-vs-upstream
//!     decode-agreement check on the SAME bytes (the id-translation oracle).
//!
//!   * Advisory timing (`hdt_load_s` seconds; `hdt_vs_ntgz_load_s` a unitless
//!     RATIO — survives box contention per `bench/CATALOG.md`) — trend-only, NEVER
//!     asserted. Measured on a synthetic archive (default ~250k triples; override
//!     with `$HDT_BENCH_N`). The full ~1M perf sketch + the direct-vs-upstream A/B
//!     live in `examples/bench_load.rs` / `examples/bench_direct_vs_upstream.rs`.
//!
//! Usage: `cargo run --release -p sparq-hdt --example bench_oracle [archive.hdt]`
//!        (the archive also accepts `$HDT_ARCHIVE`; honours `$HDT_BENCH_N` for
//!        the synthetic advisory archive size).

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use sparq_core::Graph;

/// Default synthetic-archive size for the ADVISORY timing rows. Small enough to
/// keep the per-commit hook quick; the headline ~1M sketch is `bench_load`.
const DEFAULT_SYNTH_N: usize = 250_000;
/// Min-of-N for the advisory timings (ratios survive contention; min-of-N firms it).
const RUNS: usize = 3;

fn main() {
    // ---- 1. DETERMINISTIC gate: the vendored real-world snikmeta.hdt ---------------------
    // A real archive (NOT produced by our writer) so the gate pins the decode of a
    // hdt-cpp/java-shaped FourSectDict+BitmapTriples layout, not just a round-trip.
    // sq-45zbg — the deterministic suite keeps the fixture default,
    // while same-box gathers may supply a scale-representative rdf2hdt archive.
    let hdt_path = std::env::args_os()
        .nth(1)
        .or_else(|| std::env::var_os("HDT_ARCHIVE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/snikmeta.hdt")
        });
    if !hdt_path.exists() {
        eprintln!("[hdt] ERROR: fixture {} absent", hdt_path.display());
        std::process::exit(1);
    }

    let g = sparq_hdt::load(&hdt_path).expect("loading benchmark HDT archive");

    // (a) load-and-decode-to-native: stored triple count + distinct dictionary terms.
    let triples = g.store.len();
    let terms = g.dict.len();

    // (b) triple-pattern RESOLUTION on the decoded graph (the design's §3.7(a) "+
    //     triple-pattern resolution on the same archive"): both prove the decoded
    //     graph is queryable through sparq's own index, deterministically.
    //   * distinct predicates: a full SPO scan grouped by predicate.
    //   * rdf:type triples: a single bound-predicate triple-pattern resolution.
    let distinct_predicates = distinct_predicate_count(&g);
    let rdf_type_triples =
        bound_predicate_count(&g, "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>");

    // (c) the id-translation ORACLE: the default direct decoder (H1–H4) and the
    //     upstream-backed path (`Hdt::read` + per-id `id_to_string`) must decode the
    //     SAME bytes to byte-identical graphs. Emitted as a gate metric (1 == agree)
    //     so a translation regression that still round-trips through one path fails
    //     the suite. (The exhaustive form lives in `tests/roundtrip.rs`.)
    let bytes = std::fs::read(&hdt_path).expect("reading benchmark HDT archive bytes");
    let upstream = sparq_hdt::load_reader_via_upstream(std::io::Cursor::new(bytes.clone()))
        .expect("upstream-oracle load");
    let direct_eq_upstream = (triple_set(&g) == triple_set(&upstream)
        && g.store.len() == upstream.store.len()
        && g.dict.len() == upstream.dict.len()) as u64;

    // DETERMINISTIC rows (unit `count`) — the HARD gate in run.sh.
    let mut out = String::new();
    let _ = writeln!(out, "snikmeta_triples\t{triples}\tcount");
    let _ = writeln!(out, "snikmeta_terms\t{terms}\tcount");
    let _ = writeln!(
        out,
        "snikmeta_distinct_predicates\t{distinct_predicates}\tcount"
    );
    let _ = writeln!(out, "snikmeta_rdf_type_triples\t{rdf_type_triples}\tcount");
    let _ = writeln!(
        out,
        "snikmeta_direct_eq_upstream\t{direct_eq_upstream}\tcount"
    );

    // (d) sq-hmd7l.33 — TIMED decode of the SAME vendored archive, min-of-RUNS:
    //     the sparq cell of the same-box HDT decode comparison (scripts/bench/
    //     hdt-same-box.sh → `snikmeta_decode_s` → the ingest's normalizeHdt). Timing row
    //     (unit `s`), NEVER asserted by bench/hdt/run.sh (which gates only `count` rows);
    //     work-box values are contended/advisory — canonical values come from the
    //     dedicated quiet-box gather.
    let mut best_decode_s = f64::MAX;
    for _ in 0..RUNS {
        let t = Instant::now();
        let timed = sparq_hdt::load(&hdt_path).expect("timed benchmark archive decode");
        best_decode_s = best_decode_s.min(t.elapsed().as_secs_f64());
        std::hint::black_box(&timed);
    }
    let _ = writeln!(out, "snikmeta_decode_s\t{best_decode_s:.6}\ts");

    // ---- 2. ADVISORY timing: synthetic archive load + the ntgz RATIO ---------------------
    // A unitless RATIO (hdt_vs_ntgz_load_s) survives box contention (CATALOG); the
    // absolute hdt_load_s is trend-only. Both are NEVER asserted in run.sh.
    let n: usize = std::env::var("HDT_BENCH_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SYNTH_N);
    if let Some((hdt_s, ntgz_s, synth_triples)) = synthetic_timing(n) {
        let ratio = if hdt_s > 0.0 { ntgz_s / hdt_s } else { 0.0 };
        let _ = writeln!(out, "hdt_load_s\t{hdt_s:.6}\ts");
        let _ = writeln!(out, "hdt_vs_ntgz_load_s\t{ratio:.4}\tratio");
        eprintln!(
            "[hdt] advisory (CONTENDED — orchestrator re-measures on a quiet box): \
             synthetic {synth_triples} triples, hdt_load_s={hdt_s:.4} ntgz_load_s={ntgz_s:.4} \
             ratio={ratio:.2}x"
        );
    } else {
        eprintln!("[hdt] note: synthetic advisory timing skipped (HDT writer unavailable)");
    }

    print!("{out}");
}

/// A full SPO scan grouped by predicate -> number of DISTINCT predicates.
fn distinct_predicate_count(g: &Graph) -> usize {
    use std::collections::BTreeSet;
    let scan = g.store.scan(&[None, None, None]);
    let mut preds: BTreeSet<u32> = BTreeSet::new();
    for r in scan.rows.iter() {
        let [_, pid, _] = scan.to_spo(r);
        preds.insert(pid);
    }
    preds.len()
}

/// A single bound-predicate triple-pattern resolution: count of triples whose
/// predicate term is `pred_str` (returns 0 if that predicate is absent).
fn bound_predicate_count(g: &Graph, pred_str: &str) -> usize {
    let scan = g.store.scan(&[None, None, None]);
    scan.rows
        .iter()
        .filter(|r| {
            let [_, pid, _] = scan.to_spo(r);
            g.dict.term(pid).to_string() == pred_str
        })
        .count()
}

/// A canonical, comparable triple set (N-Triples term rendering) for the
/// direct-vs-upstream decode-agreement oracle.
fn triple_set(g: &Graph) -> std::collections::BTreeSet<[String; 3]> {
    let scan = g.store.scan(&[None, None, None]);
    scan.rows
        .iter()
        .map(|r| {
            let [s, p, o] = scan.to_spo(r);
            [
                g.dict.term(s).to_string(),
                g.dict.term(p).to_string(),
                g.dict.term(o).to_string(),
            ]
        })
        .collect()
}

/// Builds a deterministic synthetic graph (same shape as `bench_load.rs`:
/// clustered subjects, a small predicate set, a mix of shared-section IRI objects
/// and literal kinds) as BOTH a `.hdt` archive and a gzipped N-Triples file, then
/// returns `(best_hdt_load_s, best_ntgz_load_s, triples)` — min-of-`RUNS`.
///
/// The `.hdt` is built via the `hdt` crate's experimental N-Triples writer
/// (`read_nt`, the dev-dependency `sophia` feature) — the same build path the
/// round-trip tests + `bench_load` use. Returns `None` if that writer is
/// unavailable, so the advisory rows are skipped rather than failing the suite.
fn synthetic_timing(n: usize) -> Option<(f64, f64, usize)> {
    let nt = generate_nt(n);

    // Scratch in a UNIQUE auto-removed temp dir (disk discipline; CATALOG).
    let dir = std::env::temp_dir().join(format!("sparq-hdt-bench-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let nt_path = dir.join("synth.nt");
    let gz_path = dir.join("synth.nt.gz");
    let hdt_path = dir.join("synth.hdt");
    std::fs::write(&nt_path, &nt).ok()?;

    // gzip the N-Triples (the .nt.gz competitor ingest path).
    let gz_file = std::fs::File::create(&gz_path).ok()?;
    let mut enc = flate2::write::GzEncoder::new(
        std::io::BufWriter::new(gz_file),
        flate2::Compression::default(),
    );
    enc.write_all(nt.as_bytes()).ok()?;
    enc.finish().ok()?.flush().ok()?;

    // N-Triples -> HDT archive (FourSectDict PFC + BitmapTriples).
    let hdt = hdt::Hdt::read_nt(&nt_path).ok()?;
    {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&hdt_path).ok()?);
        hdt.write(&mut w).ok()?;
    }

    let mut best_hdt = f64::MAX;
    let mut triples = 0usize;
    for _ in 0..RUNS {
        let t = Instant::now();
        let g = sparq_hdt::load(&hdt_path).ok()?;
        best_hdt = best_hdt.min(t.elapsed().as_secs_f64());
        triples = g.store.len();
    }

    let mut best_ntgz = f64::MAX;
    for _ in 0..RUNS {
        let t = Instant::now();
        let f = std::fs::File::open(&gz_path).ok()?;
        let g = Graph::load_reader(flate2::read::GzDecoder::new(f), "ntriples").ok()?;
        best_ntgz = best_ntgz.min(t.elapsed().as_secs_f64());
        // Both ingest paths must agree on the stored triple count (a cheap sanity
        // cross-check; the HARD correctness gate is the snikmeta block above).
        debug_assert_eq!(g.store.len(), triples);
    }

    // Clean the scratch dir (disk discipline; bench data is regenerable).
    let _ = std::fs::remove_dir_all(&dir);
    Some((best_hdt, best_ntgz, triples))
}

/// Deterministic synthetic N-Triples (SplitMix-free xorshift, fixed seed): clustered
/// subjects, a small predicate set, shared-section IRI objects + a literal zoo —
/// mirroring `bench_load.rs` so the dictionary has realistic term reuse across PFC
/// blocks.
fn generate_nt(n: usize) -> String {
    let mut nt = String::with_capacity(n * 100);
    let mut st = 0x243f_6a88u64;
    let mut rng = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        st
    };
    for _ in 0..n {
        let s = rng() % 100_000;
        let p = rng() % 50;
        let _ = write!(
            nt,
            "<http://bench.example/e{s}> <http://bench.example/vocab/p{p}> "
        );
        match rng() % 5 {
            0 => {
                let _ = write!(nt, "<http://bench.example/e{}>", rng() % 100_000);
            }
            1 => {
                let _ = write!(nt, "\"label {}\"", rng() % 1_000_000);
            }
            2 => {
                let _ = write!(nt, "\"etikett {}\"@de", rng() % 1_000_000);
            }
            3 => {
                let _ = write!(
                    nt,
                    "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                    rng() % 100_000
                );
            }
            _ => {
                let _ = write!(
                    nt,
                    "\"{}.{:02}\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                    rng() % 1000,
                    rng() % 100
                );
            }
        }
        nt.push_str(" .\n");
    }
    nt
}
