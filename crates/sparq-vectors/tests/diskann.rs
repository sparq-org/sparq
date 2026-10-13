//! [OPUS-4.8] Accuracy + persistence gate for the on-disk Vamana index ([`DiskAnnIndex`]):
//!
//! 1. **recall@10 vs exact brute force** ≥ 0.90 on the same 50k synthetic set the HNSW gate
//!    (`tests/recall.rs`) uses — a persisted graph must find the true neighbours;
//! 2. **restart survival** — build → drop the handle → `open` a fresh one (no rebuild) and get
//!    byte-identical neighbours, the acceptance criterion of sq-7zc;
//! 3. **parity with the in-RAM searchers** on tiny separable clusters, and the degenerate
//!    all-zero-query contract shared with `nearest_exact` / `VectorIndex`.
//!
//! [OPUS-4.8] (sq-k5qq) Strictly-additive machine-readable emit: set `SPARQ_VECTORS_JSON`
//! to a path and the recall gate ALSO writes its deterministic recall + advisory build/query
//! timings there as a stable, DEPENDENCY-FREE JSON document (mirroring
//! `crates/sparq-mpc/examples/mpc_net_bench.rs::cell_json`). These tests run under `cargo test`
//! with NO CLI args, so the path comes from an env var, not a `--json` flag. STDERR is
//! byte-for-byte unchanged whether or not the env var is set; the timings are ADVISORY +
//! NON-CANONICAL (this dev box) and nothing is committed. serde_json is a TEST-only dev-dep.

use sparq_vectors::{nearest_exact, DiskAnnIndex, PqConfig, VamanaConfig, VectorStore};
// [OPUS-4.8] (sq-ip3a) HNSW is the approximate backend — `approx-ann` only (the disk-vs-HNSW
// parity test below is gated to match).
#[cfg(feature = "approx-ann")]
use sparq_vectors::VectorIndex;
use std::time::Instant;

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-k5qq) SPARQ_VECTORS_JSON=<path> machine-readable results emit.
// Dependency-free `format!`-built JSON, the same idiom as tests/throughput.rs.
// ---------------------------------------------------------------------------

/// The env var that, when set to a path, makes a measurement test ALSO write its
/// metrics there as JSON. Shared with `tests/throughput.rs`.
const JSON_ENV: &str = "SPARQ_VECTORS_JSON";

/// Minimal JSON string escaper (the dependency-free emit).
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    sparq_core::json::escape_into(&mut out, raw);
    out.push('"');
    out
}

/// Serialise the workload dimensions + captured `metrics` to stable, dependency-free JSON.
/// `recall` is a DETERMINISTIC float at the pinned `(N, seed)` workload; the timings are
/// ADVISORY + NON-CANONICAL (stated in `note`).
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
        "  \"note\": \"`recall` is DETERMINISTIC at the pinned (N, seed) workload; the \
         build/query timings are best-effort, MEASURED on the running host — ADVISORY, \
         NON-CANONICAL (this dev box) — do not bake into committed files\",\n",
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

/// Uniform in [-1, 1).
fn rand_vec(state: &mut u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|_| ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 1.0)
        .collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sparq-vectors-{name}-{}", std::process::id()))
}

#[test]
fn diskann_recall_at_10_vs_brute_force_on_50k() {
    const N: usize = 50_000;
    const DIM: usize = 32;
    const K: usize = 10;
    const QUERIES: usize = 100;

    let store_path = tmp("diskann-recall.spqv");
    let graph_path = tmp("diskann-recall.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 0xC0FFEE_u64;
    for i in 0..N {
        // Sparse, non-contiguous ids to exercise the id mapping (same as the HNSW gate).
        let id = (i as u32) * 7 + 3;
        store.put(id, &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();

    let t = Instant::now();
    let index = DiskAnnIndex::build(&store, &graph_path).unwrap();
    let build = t.elapsed();
    assert_eq!(index.len(), N);
    assert_eq!(index.dim(), DIM);

    let mut q_state = 0xDECAF_u64;
    let mut hits = 0usize;
    let t = Instant::now();
    for _ in 0..QUERIES {
        let q = rand_vec(&mut q_state, DIM);
        let exact: Vec<u32> =
            nearest_exact(&store, &q, K).into_iter().map(|(id, _)| id).collect();
        let approx: Vec<u32> = index.nearest(&q, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(exact.len(), K);
        assert_eq!(approx.len(), K);
        hits += approx.iter().filter(|id| exact.contains(id)).count();
    }
    let query = t.elapsed();
    let recall = hits as f64 / (QUERIES * K) as f64;
    eprintln!("DiskANN recall@{K} over {QUERIES} queries on {N}x{DIM}: {recall:.4}");
    assert!(recall >= 0.90, "recall@{K} gate failed: {recall:.4} < 0.90");

    // [OPUS-4.8] (sq-k5qq) Strictly-additive: writes only when SPARQ_VECTORS_JSON is set;
    // the stderr line above is unchanged either way. `recall` is the deterministic gate value.
    let doc = results_json(
        "sparq-vectors diskann recall",
        N,
        DIM,
        K,
        QUERIES,
        &[
            ("recall", recall),
            ("build_ms", build.as_secs_f64() * 1e3),
            ("query_ms_per_query", query.as_secs_f64() * 1e3 / QUERIES as f64),
        ],
    );
    maybe_write_json(&doc);

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-k5qq) The dependency-free emit must round-trip through a REAL
// serde_json parse — this runs in BOTH feature states (no approx-ann gate).
// ---------------------------------------------------------------------------
#[test]
fn json_round_trips() {
    assert_eq!(json_str("recall"), "\"recall\"");
    assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");

    let doc = results_json(
        "sparq-vectors diskann recall",
        50_000,
        32,
        10,
        100,
        &[("recall", 0.97), ("build_ms", 1234.5)],
    );
    let v: serde_json::Value = serde_json::from_str(&doc).expect("emit must be valid JSON");
    assert_eq!(v["harness"], "sparq-vectors diskann recall");
    assert_eq!(v["n"], 50_000);
    assert_eq!(v["dim"], 32);
    assert_eq!(v["k"], 10);
    assert_eq!(v["queries"], 100);
    assert!(v["note"].as_str().unwrap().contains("NON-CANONICAL"));
    assert!(v["metrics"]["recall"].is_number());
    assert!(v["metrics"]["build_ms"].is_number());

    let empty = results_json("h", 1, 1, 1, 1, &[]);
    let v2: serde_json::Value = serde_json::from_str(&empty).expect("empty metrics is valid JSON");
    assert!(v2["metrics"].as_object().unwrap().is_empty());
}

#[test]
fn reopen_without_rebuild_returns_identical_neighbours() {
    // The sq-7zc acceptance criterion: a versioned on-disk artifact opened WITHOUT a rebuild,
    // with recall parity to the built index. We build once, capture results, drop the handle
    // (simulating process exit), then `open` a fresh handle off the file alone and assert the
    // neighbours+scores match exactly — proving the graph round-tripped through disk intact.
    const N: usize = 5_000;
    const DIM: usize = 24;
    const K: usize = 10;

    let store_path = tmp("diskann-reopen.spqv");
    let graph_path = tmp("diskann-reopen.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 0x1234_5678_u64;
    for i in 0..N {
        store.put((i as u32) * 3 + 1, &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();

    let mut q_state = 0xFACE_u64;
    let queries: Vec<Vec<f32>> = (0..50).map(|_| rand_vec(&mut q_state, DIM)).collect();

    // Build, query, then drop the built handle entirely.
    let built_results: Vec<Vec<(u32, f32)>> = {
        let built = DiskAnnIndex::build_with(&store, &graph_path, VamanaConfig::default()).unwrap();
        queries.iter().map(|q| built.nearest(q, K)).collect()
    };

    // Reopen from the file alone — NO rebuild — and re-query.
    let reopened = DiskAnnIndex::open(&graph_path).unwrap();
    assert_eq!(reopened.len(), N);
    assert_eq!(reopened.dim(), DIM);
    for (q, expected) in queries.iter().zip(&built_results) {
        let got = reopened.nearest(q, K);
        assert_eq!(got.len(), expected.len());
        for (g, e) in got.iter().zip(expected) {
            assert_eq!(g.0, e.0, "reopened neighbour id differs from the built index");
            assert!(
                (g.1 - e.1).abs() < 1e-6,
                "reopened cosine {} differs from built {}",
                g.1,
                e.1
            );
        }
    }

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[cfg(feature = "approx-ann")]
#[test]
fn diskann_parity_with_hnsw_on_separable_clusters() {
    // Three well-separated clusters; the on-disk index, the in-RAM HNSW, and exact brute force
    // must all return the query's own cluster, with cosine scores in agreement.
    const DIM: usize = 8;
    let store_path = tmp("diskann-tiny.spqv");
    let graph_path = tmp("diskann-tiny.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 7_u64;
    let mut centers = Vec::new();
    for c in 0..3 {
        let center = rand_vec(&mut state, DIM);
        for i in 0..20 {
            let v: Vec<f32> = center
                .iter()
                .map(|x| {
                    x + ((splitmix64(&mut state) >> 40) as f32 / (1u64 << 23) as f32 - 0.5) * 0.05
                })
                .collect();
            store.put((c * 100 + i) as u32 + 1, &v).unwrap();
        }
        centers.push(center);
    }
    store.finalize().unwrap();

    // Small graphs: degree must stay ≤ build_beam, and both ≤ the cluster sizes are fine.
    let cfg = VamanaConfig { degree: 16, build_beam: 32, search_beam: 32, ..Default::default() };
    let disk = DiskAnnIndex::build_with(&store, &graph_path, cfg).unwrap();
    let hnsw = VectorIndex::build(&store);

    for (c, center) in centers.iter().enumerate() {
        let exact = nearest_exact(&store, center, 5);
        let approx_disk = disk.nearest(center, 5);
        let approx_hnsw = hnsw.nearest(center, 5);
        for &(id, sim) in exact.iter().chain(approx_disk.iter()).chain(approx_hnsw.iter()) {
            assert_eq!((id - 1) / 100, c as u32, "neighbor {id} from wrong cluster");
            assert!(sim > 0.99, "cluster member should be near-identical, got {sim}");
        }
        // The on-disk index returns the same id set as the in-RAM HNSW on separable data.
        let disk_ids: std::collections::HashSet<u32> =
            approx_disk.iter().map(|&(id, _)| id).collect();
        let hnsw_ids: std::collections::HashSet<u32> =
            approx_hnsw.iter().map(|&(id, _)| id).collect();
        assert_eq!(disk_ids, hnsw_ids, "on-disk and HNSW disagree on separable cluster {c}");
    }

    // Degenerate: an all-zero query has no direction → empty, matching the other searchers.
    let zero = vec![0.0f32; DIM];
    assert!(disk.nearest(&zero, 5).is_empty());

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn empty_and_singleton_graphs_roundtrip() {
    let store_path = tmp("diskann-empty.spqv");
    let graph_path = tmp("diskann-empty.spqg");

    // Empty store → empty graph; opens, searches return nothing.
    let mut store = VectorStore::create(&store_path, 8).unwrap();
    store.finalize().unwrap();
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();
    assert!(idx.is_empty());
    assert!(idx.nearest(&[1.0; 8], 5).is_empty());
    let reopened = DiskAnnIndex::open(&graph_path).unwrap();
    assert!(reopened.is_empty());

    // Singleton store → the one vector is its own nearest neighbour.
    let store_path1 = tmp("diskann-one.spqv");
    let graph_path1 = tmp("diskann-one.spqg");
    let mut one = VectorStore::create(&store_path1, 4).unwrap();
    one.put(99, &[1.0, 2.0, 3.0, 4.0]).unwrap();
    one.finalize().unwrap();
    let idx1 = DiskAnnIndex::build(&one, &graph_path1).unwrap();
    let nn = idx1.nearest(&[1.0, 2.0, 3.0, 4.0], 5);
    assert_eq!(nn.len(), 1);
    assert_eq!(nn[0].0, 99);
    assert!((nn[0].1 - 1.0).abs() < 1e-5, "self-similarity should be ~1, got {}", nn[0].1);

    for p in [&store_path, &graph_path, &store_path1, &graph_path1] {
        let _ = std::fs::remove_file(p);
    }
}

// [OPUS-4.8] (sq-qamd) PQ candidate cache wired into the DiskANN search path: build with a PQ
// section, confirm it persists + reloads, the codes drive the beam (RAM-only ranking) and the
// final beam is re-ranked off the mmap so the reported cosine is EXACT, and recall stays high.

#[test]
fn pq_index_persists_and_reloads_the_candidate_cache() {
    const N: usize = 2_000;
    const DIM: usize = 32;
    let store_path = tmp("diskann-pq-rt.spqv");
    let graph_path = tmp("diskann-pq-rt.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 0xA11CE_u64;
    for i in 0..N {
        store.put((i as u32) * 5 + 1, &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();

    let built = DiskAnnIndex::build_with_pq(
        &store,
        &graph_path,
        VamanaConfig::default(),
        PqConfig::default(),
    )
    .unwrap();
    assert!(built.has_pq_cache(), "build_with_pq must produce a PQ candidate cache");
    assert_eq!(built.len(), N);

    // Reopen from the file alone — the PQ section must reload (no rebuild).
    let reopened = DiskAnnIndex::open(&graph_path).unwrap();
    assert!(reopened.has_pq_cache(), "reopened index must carry the persisted PQ cache");
    assert_eq!(reopened.len(), N);
    assert_eq!(reopened.dim(), DIM);

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn pq_returned_cosines_are_exact_after_rerank() {
    // The re-rank reads full-precision vectors off the mmap, so a returned (id, cosine) must equal
    // the EXACT cosine of that id to the query — the PQ approximation guides the beam but never
    // leaks into the reported score.
    const N: usize = 2_000;
    const DIM: usize = 32;
    const K: usize = 10;
    let store_path = tmp("diskann-pq-exact.spqv");
    let graph_path = tmp("diskann-pq-exact.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 0xBEEF_u64;
    for i in 0..N {
        store.put((i as u32) + 1, &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();

    let idx = DiskAnnIndex::build_with_pq(
        &store,
        &graph_path,
        VamanaConfig::default(),
        PqConfig::default(),
    )
    .unwrap();

    let mut q_state = 0xF00D_u64;
    for _ in 0..30 {
        let q = rand_vec(&mut q_state, DIM);
        let approx = idx.nearest(&q, K);
        // Returned scores must be sorted best-first and equal the exact cosine for each id.
        let exact_all = nearest_exact(&store, &q, N);
        for w in approx.windows(2) {
            assert!(w[0].1 >= w[1].1 - 1e-6, "results not sorted best-first");
        }
        for &(id, cos) in &approx {
            let exact = exact_all
                .iter()
                .find(|&&(eid, _)| eid == id)
                .map(|&(_, c)| c)
                .expect("returned id must exist in the store");
            assert!(
                (cos - exact).abs() < 1e-5,
                "re-ranked cosine {cos} for id {id} != exact {exact}"
            );
        }
    }

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn pq_recall_at_10_stays_high() {
    // PQ-guided search is approximate, but with the re-rank it should still recover most of the
    // true top-K. A gentle gate (≥ 0.80) over a moderate set — well clear of chance.
    const N: usize = 10_000;
    const DIM: usize = 32;
    const K: usize = 10;
    const QUERIES: usize = 100;
    let store_path = tmp("diskann-pq-recall.spqv");
    let graph_path = tmp("diskann-pq-recall.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 0xC0FFEE_u64;
    for i in 0..N {
        store.put((i as u32) * 7 + 3, &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();

    // A wider beam compensates for the lossy codes; M=16 gives 2 dims/subspace at DIM=32.
    let cfg = VamanaConfig { search_beam: 200, ..Default::default() };
    let idx = DiskAnnIndex::build_with_pq(&store, &graph_path, cfg, PqConfig::default()).unwrap();
    assert!(idx.has_pq_cache());

    let mut q_state = 0xDECAF_u64;
    let mut hits = 0usize;
    for _ in 0..QUERIES {
        let q = rand_vec(&mut q_state, DIM);
        let exact: Vec<u32> = nearest_exact(&store, &q, K).into_iter().map(|(id, _)| id).collect();
        let approx: Vec<u32> = idx.nearest(&q, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(approx.len(), K);
        hits += approx.iter().filter(|id| exact.contains(id)).count();
    }
    let recall = hits as f64 / (QUERIES * K) as f64;
    eprintln!("DiskANN+PQ recall@{K} over {QUERIES} queries on {N}x{DIM}: {recall:.4}");
    assert!(recall >= 0.80, "PQ recall@{K} gate failed: {recall:.4} < 0.80");

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn plain_index_has_no_pq_cache() {
    let store_path = tmp("diskann-nopq.spqv");
    let graph_path = tmp("diskann-nopq.spqg");
    let mut store = VectorStore::create(&store_path, 8).unwrap();
    let mut state = 1_u64;
    for i in 0..100 {
        store.put(i + 1, &rand_vec(&mut state, 8)).unwrap();
    }
    store.finalize().unwrap();
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();
    assert!(!idx.has_pq_cache(), "plain build must not carry a PQ cache");
    let reopened = DiskAnnIndex::open(&graph_path).unwrap();
    assert!(!reopened.has_pq_cache());
    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn open_rejects_corrupt_and_wrong_magic() {
    let path = tmp("diskann-bad.spqg");
    // Wrong magic.
    std::fs::write(&path, b"NOPExxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").unwrap();
    assert!(DiskAnnIndex::open(&path).is_err());
    // Truncated header.
    std::fs::write(&path, b"SPQG").unwrap();
    assert!(DiskAnnIndex::open(&path).is_err());
    let _ = std::fs::remove_file(&path);
}

// [FABLE-5] (sq-98c) `open_from_bytes` — the filesystem-less / wasm read path (memmap2 is
// target-gated out of wasm32 builds; on wasm the graph arrives as fetched/embedded bytes).
// The owned backing must be RESULT-IDENTICAL to the memory map: same neighbours, same scores,
// same metadata. This also exercises the f32-alignment `debug_assert` in `node_vector` over
// the owned (`AlignedBytes`) backing — a plain `Vec<u8>` base would trip it.
#[test]
fn open_from_bytes_returns_identical_neighbours_to_mmap_open() {
    const DIM: usize = 8;
    let store_path = tmp("diskann-bytes.spqv");
    let graph_path = tmp("diskann-bytes.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 11_u64;
    for id in 1..=60_u32 {
        let v = rand_vec(&mut state, DIM);
        store.put(id, &v).unwrap();
    }
    store.finalize().unwrap();
    let cfg = VamanaConfig { degree: 8, build_beam: 16, search_beam: 24, ..Default::default() };
    let mapped = DiskAnnIndex::build_with(&store, &graph_path, cfg).unwrap();

    let bytes = std::fs::read(&graph_path).unwrap();
    let owned = DiskAnnIndex::open_from_bytes(bytes).unwrap();
    assert_eq!(owned.len(), mapped.len());
    assert_eq!(owned.dim(), mapped.dim());
    assert_eq!(owned.fingerprint(), mapped.fingerprint());
    assert!(!owned.has_pq_cache());
    // NOTE: `build_with` seeds `search_beam` from the config while `open`/`open_from_bytes`
    // use the default; compare two opened handles so the traversals are identical.
    let reopened = DiskAnnIndex::open(&graph_path).unwrap();
    let mut state = 99_u64;
    for _ in 0..10 {
        let q = rand_vec(&mut state, DIM);
        assert_eq!(owned.nearest(&q, 5), reopened.nearest(&q, 5));
    }
    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

// [FABLE-5] (sq-98c) The PQ candidate cache (trailing `.spqg` section) must also parse + serve
// identically from owned bytes — the section offsets are relative, not map-page-dependent.
#[test]
fn open_from_bytes_serves_the_pq_index_identically() {
    const DIM: usize = 8;
    let store_path = tmp("diskann-bytes-pq.spqv");
    let graph_path = tmp("diskann-bytes-pq.spqg");
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = 13_u64;
    for id in 1..=60_u32 {
        let v = rand_vec(&mut state, DIM);
        store.put(id, &v).unwrap();
    }
    store.finalize().unwrap();
    let cfg = VamanaConfig { degree: 8, build_beam: 16, search_beam: 24, ..Default::default() };
    // M (subspaces) must divide into the small test dimension; the default M=16 needs dim ≥ 16.
    let pq_cfg = PqConfig { m: 4, ..Default::default() };
    DiskAnnIndex::build_with_pq(&store, &graph_path, cfg, pq_cfg).unwrap();

    let reopened = DiskAnnIndex::open(&graph_path).unwrap();
    let owned = DiskAnnIndex::open_from_bytes(std::fs::read(&graph_path).unwrap()).unwrap();
    assert!(owned.has_pq_cache(), "the PQ section must survive the owned-bytes open");
    let mut state = 101_u64;
    for _ in 0..10 {
        let q = rand_vec(&mut state, DIM);
        assert_eq!(owned.nearest(&q, 5), reopened.nearest(&q, 5));
    }
    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

// [FABLE-5] (sq-98c) Validation is identical to `open`: bad magic / truncation reject up front.
#[test]
fn open_from_bytes_rejects_corrupt_and_wrong_magic() {
    assert!(DiskAnnIndex::open_from_bytes(b"NOPExxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_vec()).is_err());
    assert!(DiskAnnIndex::open_from_bytes(b"SPQG".to_vec()).is_err());
    assert!(DiskAnnIndex::open_from_bytes(Vec::new()).is_err());
}
