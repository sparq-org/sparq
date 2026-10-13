//! Throughput measurement behind `#[ignore]` — the source of the README numbers.
//! Run with:
//!
//! ```text
//! cargo test -p sparq-vectors --release --test throughput -- --ignored --nocapture
//! ```
//!
//! Prints store build/finalize/open, HNSW build, and exact vs HNSW query throughput
//! on the same 50k×32 random workload as the recall gate (tests/recall.rs).
//!
//! [OPUS-4.8] (sq-k5qq) Strictly-additive machine-readable emit: set `SPARQ_VECTORS_JSON`
//! to a path and the measurement run ALSO writes the same advisory timings as a stable,
//! DEPENDENCY-FREE JSON document there — mirroring the `format!`-JSON convention of
//! `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json`. This test runs under
//! `cargo test` with NO CLI args, so the path comes from an env var, not a `--json` flag.
//! STDERR above is byte-for-byte unchanged whether or not the env var is set; the timings
//! are ADVISORY + NON-CANONICAL (this dev box) and nothing is committed. (No serde dep is
//! added to the harness; serde_json is a TEST-only dev-dep used by `json_round_trips`.)

// [OPUS-4.8] (sq-ip3a) HNSW (VectorIndex) is the approximate backend — gated behind `approx-ann`.
#![cfg(feature = "approx-ann")]

use sparq_vectors::{nearest_exact, VectorIndex, VectorStore};
use std::time::Instant;

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-k5qq) SPARQ_VECTORS_JSON=<path> machine-readable results emit.
// Dependency-free `format!`-built JSON, mirroring mpc_net_bench::cell_json. The
// run prints its usual stderr table unconditionally; the JSON file is written
// ONLY when the env var is set, so behaviour is byte-identical when it is absent.
// ---------------------------------------------------------------------------

/// The env var that, when set to a path, makes the measurement run ALSO write its
/// timings there as JSON. Shared with `tests/diskann.rs`.
const JSON_ENV: &str = "SPARQ_VECTORS_JSON";

/// Minimal JSON string escaper (the dependency-free emit). Labels are static ASCII;
/// anything else still yields valid `\uXXXX`.
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialise `(harness, n, dim, k, queries)` plus the captured `metrics` rows to stable,
/// dependency-free JSON. Every metric is ADVISORY + NON-CANONICAL (stated in `note`).
fn results_json(
    harness: &str,
    n: usize,
    dim: usize,
    k: usize,
    queries: usize,
    metrics: &[(&str, f64)],
) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"harness\": {},\n", json_str(harness)));
    s.push_str(&format!("  \"n\": {n},\n  \"dim\": {dim},\n  \"k\": {k},\n  \"queries\": {queries},\n"));
    s.push_str(
        "  \"note\": \"`n`/`dim`/`k`/`queries` are the fixed deterministic workload \
         dimensions; every timing/throughput metric below is best-effort, MEASURED on the \
         running host — ADVISORY, NON-CANONICAL (this dev box) — do not bake into committed \
         files\",\n",
    );
    s.push_str("  \"metrics\": {\n");
    for (i, (key, val)) in metrics.iter().enumerate() {
        let comma = if i + 1 < metrics.len() { "," } else { "" };
        s.push_str(&format!("    {}: {val:.6}{comma}\n", json_str(key)));
    }
    s.push_str("  }\n");
    s.push_str("}\n");
    s
}

/// Write the JSON document iff `SPARQ_VECTORS_JSON` is set. Returns whether it wrote.
fn maybe_write_json(doc: &str) -> bool {
    match std::env::var(JSON_ENV) {
        Ok(path) if !path.is_empty() => {
            if let Err(e) = std::fs::write(&path, doc) {
                eprintln!("error writing {JSON_ENV} results to {path}: {e}");
            } else {
                eprintln!("wrote results JSON to {path}");
            }
            true
        }
        _ => false,
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn rand_vec(state: &mut u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|_| ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 1.0)
        .collect()
}

#[test]
#[ignore = "throughput measurement, not a correctness gate; run with --ignored --nocapture"]
fn throughput_50k() {
    const N: usize = 50_000;
    const DIM: usize = 32;
    const K: usize = 10;
    const QUERIES: usize = 200;

    let path = std::env::temp_dir()
        .join(format!("sparq-vectors-throughput-{}.spqv", std::process::id()));

    let t = Instant::now();
    let mut store = VectorStore::create(&path, DIM).unwrap();
    let mut state = 0xC0FFEE_u64;
    for i in 0..N {
        store.put((i as u32) * 7 + 3, &rand_vec(&mut state, DIM)).unwrap();
    }
    let put = t.elapsed();
    let t = Instant::now();
    store.finalize().unwrap();
    let finalize = t.elapsed();
    let t = Instant::now();
    let reopened = VectorStore::open(&path).unwrap();
    let open = t.elapsed();
    assert_eq!(reopened.len(), N);

    let t = Instant::now();
    let index = VectorIndex::build(&store);
    let build = t.elapsed();

    let mut q_state = 0xDECAF_u64;
    let queries: Vec<Vec<f32>> = (0..QUERIES).map(|_| rand_vec(&mut q_state, DIM)).collect();

    let t = Instant::now();
    let mut sink = 0usize;
    for q in &queries {
        sink += nearest_exact(&store, q, K).len();
    }
    let exact = t.elapsed();

    let t = Instant::now();
    for q in &queries {
        sink += index.nearest(q, K).len();
    }
    let hnsw = t.elapsed();
    assert_eq!(sink, 2 * QUERIES * K);

    let per = |d: std::time::Duration| d.as_secs_f64() / QUERIES as f64;
    eprintln!("--- sparq-vectors throughput, {N}x{DIM}, k={K} ---");
    eprintln!("put {N} vectors:        {put:?}");
    eprintln!("finalize (write+mmap):  {finalize:?}");
    eprintln!("open (mmap):            {open:?}");
    eprintln!("HNSW build:             {build:?}");
    eprintln!(
        "exact top-{K}:           {:.3} ms/query ({:.0} q/s)",
        per(exact) * 1e3,
        1.0 / per(exact)
    );
    eprintln!(
        "HNSW top-{K}:            {:.4} ms/query ({:.0} q/s)",
        per(hnsw) * 1e3,
        1.0 / per(hnsw)
    );

    // [OPUS-4.8] (sq-k5qq) Strictly-additive: only writes when SPARQ_VECTORS_JSON is set;
    // the stderr table above is unchanged either way. Same advisory numbers, machine-readable.
    let doc = results_json(
        "sparq-vectors throughput",
        N,
        DIM,
        K,
        QUERIES,
        &[
            ("put_ms", put.as_secs_f64() * 1e3),
            ("finalize_ms", finalize.as_secs_f64() * 1e3),
            ("open_ms", open.as_secs_f64() * 1e3),
            ("hnsw_build_ms", build.as_secs_f64() * 1e3),
            ("exact_ms_per_query", per(exact) * 1e3),
            ("hnsw_ms_per_query", per(hnsw) * 1e3),
        ],
    );
    maybe_write_json(&doc);

    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-k5qq) The dependency-free emit must round-trip through a REAL
// serde_json parse — this catches a malformed document (e.g. a missing comma).
// ---------------------------------------------------------------------------
#[test]
fn json_round_trips() {
    assert_eq!(json_str("put_ms"), "\"put_ms\"");
    assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");

    let doc = results_json(
        "sparq-vectors throughput",
        50_000,
        32,
        10,
        200,
        &[("put_ms", 12.5), ("hnsw_ms_per_query", 0.0042)],
    );
    let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
    assert_eq!(v["harness"], "sparq-vectors throughput");
    assert_eq!(v["n"], 50_000);
    assert_eq!(v["dim"], 32);
    assert_eq!(v["k"], 10);
    assert_eq!(v["queries"], 200);
    assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
    assert!(v["metrics"]["put_ms"].is_number());
    assert!(v["metrics"]["hnsw_ms_per_query"].is_number());

    // An empty metrics map must still be valid JSON (no trailing comma).
    let empty = results_json("h", 1, 1, 1, 1, &[]);
    let v2: serde_json::Value = serde_json::from_str(&empty).expect("empty metrics is valid JSON");
    assert!(v2["metrics"].as_object().unwrap().is_empty());
}
