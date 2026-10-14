//! (sq-v02y, design §3.3) Vector / ANN benchmark runner — the G1 TSV-emitting
//! runner for `bench/vector/`, the ANN analogue of `sparq-cli bench` / `bench_shacl` /
//! `bench_text`. It is the SAME measurement the crate's gate tests (`tests/recall.rs`,
//! `tests/diskann.rs`, `tests/quant.rs`) already assert — recall@10 vs the `nearest_exact`
//! brute-force ground truth — promoted into a TRACKED METRIC the bench dashboard + gate
//! can ratchet across commits.
//!
//! The cross-cutting trick (design §4 / gap G4): ANN recall is *larger-is-better*, but the
//! `mode:"auto"` perf-gate ratchet is *smaller-is-better*. So we emit recall as a **DEFICIT**
//! `(1 - recall)` scaled to integer milli-units — `recall_deficit_milli = round((1-recall)*1000)`
//! — exactly like `scripts/bench-adapters/vector_lib_adapter.py`. A recall regression then
//! shows as the deficit GROWING, which the integer mode:auto ratchet catches with ZERO
//! `perf-gate.py` change.
//!
//! ```sh
//! # No args reproduces the PINNED per-commit corpus that bench/vector/gen.sh + expected.tsv
//! # are derived from, so a casual `cargo run` reproduces the gated deficits.
//! cargo run -p sparq-vectors --release --example bench_vectors             # N=50000 seed=0 iters=3
//! cargo run -p sparq-vectors --release --example bench_vectors -- [N] [seed] [iters]
//! ```
//!
//! Output: one TSV line per workload (sorted by name), the same `name\tcount\tus` 3-column
//! contract the ci-bench hook consumes:
//!
//! ```text
//! hnsw_recall_at10      <recall_deficit_milli>  <query_us>    # HNSW (in-RAM)
//! diskann_recall_at10   <recall_deficit_milli>  <query_us>    # Vamana (on-disk .spqg)
//! pq_recall_at10        <recall_deficit_milli>  <query_us>    # PQ + re-rank (DiskANN loop)
//! build_s               <0>                     <build_us>    # advisory HNSW build time
//! ```
//!
//! - `count` for the recall workloads = the integer recall deficit (the DETERMINISTIC gate
//!   column `bench/vector/run.sh` asserts == `expected.tsv`, exit 1 on drift; ALSO the
//!   `vectors_*_recall_at10` mode:auto ratchet in `bench/perf-baseline.json`). Deterministic
//!   at the pinned `(N, seed)` corpus because the PRNG, the index params, and the query set
//!   are all fixed — recall is a pure function of them.
//! - `count` for `build_s` = a sentinel `0` (it carries no deterministic gate — build time is
//!   advisory wall-clock; the row exists only to forward the advisory `vectors_build_s` timing).
//! - `us` = the best-of-`iters` time (ADVISORY; this dev box is non-canonical). For the recall
//!   rows it is microseconds-per-query (`vectors_*_query_us`); for `build_s` it is the HNSW
//!   index build time in microseconds (advisory `vectors_build_s`).
//!
//! `bench/vector/run.sh` parses this, asserts the `count` columns against
//! `bench/vector/expected.tsv` (exit 1 on drift), and forwards the same 3-column contract.
//! Correctness lives in `expected.tsv`, NOT in the binary — exactly like LUBM / SHACL / FTS.

use std::time::Instant;

use sparq_vectors::{
    cosine, nearest_exact, DiskAnnIndex, DistanceTable, PqConfig, ProductQuantizer, VectorIndex,
    VectorStore,
};

/// The DEFAULT corpus RNG seed — pinned to match `bench/vector/gen.sh` (seed 0) so a no-arg
/// `cargo run --example bench_vectors` reproduces EXACTLY the committed `bench/vector/expected.tsv`
/// deficits. Identical to the seed the crate's gate tests use (`0xC0FFEE` derived below) so the
/// bench corpus IS the test corpus.
const CORPUS_SEED: u64 = 0;

/// The DEFAULT corpus size — pinned to match `bench/vector/gen.sh` (N=50000), the same 50k×32
/// set `tests/recall.rs` / `tests/diskann.rs` gate on. Sub-second build at release opt-level; the
/// big ANN datasets (SIFT1M / GloVe-100-angular) are the gather-tier `ann-benchmarks` harness.
const CORPUS_N: usize = 50_000;

/// Vector dimensionality (matches the crate gate tests).
const DIM: usize = 32;
/// Recall is measured @K (matches the crate gate tests + the `recall_at10` metric names).
const K: usize = 10;
/// Query-set size (matches the crate gate tests; the deficit is the mean over these).
const QUERIES: usize = 100;

// --- the deterministic splitmix64 PRNG the crate gate tests use (so the bench corpus is the
//     SAME corpus the tests gate on; recall is reproducible byte-for-byte at a fixed seed). ----
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Uniform in [-1, 1) (identical to the crate gate tests).
fn rand_vec(state: &mut u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|_| ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 1.0)
        .collect()
}

/// (1 - recall) scaled to integer milli-units — the smaller-is-better metric the
/// mode:"auto" ratchet wants (design §4 / gap G4). round() so e.g. recall 0.971 -> 29.
fn recall_deficit_milli(recall: f64) -> i64 {
    ((1.0 - recall) * 1000.0).round() as i64
}

/// Build the pinned uniform corpus (the HNSW / DiskANN substrate): N×DIM, sparse non-contiguous
/// ids (id = i*7+3) to exercise the id→slot mapping, exactly like `tests/recall.rs`.
fn build_uniform_store(path: &std::path::Path, n: usize, seed_word: u64) -> VectorStore {
    let mut store = VectorStore::create(path, DIM).unwrap();
    let mut state = seed_word;
    for i in 0..n {
        let id = (i as u32) * 7 + 3;
        store.put(id, &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();
    store
}

/// A synthetic CLUSTERED store (PQ excels on clustered data — the regime real embeddings live in,
/// matching `tests/quant.rs`'s recall gate). `clusters` blobs of `per` points, dense ids+1.
fn build_clustered_store(
    path: &std::path::Path,
    clusters: usize,
    per: usize,
    spread: f32,
    state: &mut u64,
) -> VectorStore {
    let mut store = VectorStore::create(path, DIM).unwrap();
    let mut id = 1u32;
    for _ in 0..clusters {
        let center = rand_vec(state, DIM);
        for _ in 0..per {
            let v: Vec<f32> = center
                .iter()
                .map(|x| x + ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32 - 0.5) * spread)
                .collect();
            store.put(id, &v).unwrap();
            id += 1;
        }
    }
    store.finalize().unwrap();
    store
}

fn tmp(name: &str, seed: u64) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sparq-vectors-bench-{name}-{seed}-{}", std::process::id()))
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(CORPUS_N);
    let seed: u64 = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(CORPUS_SEED);
    let iters: usize = std::env::args().nth(3).and_then(|a| a.parse().ok()).unwrap_or(3);
    let iters = iters.max(1);

    // The PRNG seed words: derived from the run seed so a non-default seed yields a different
    // (still deterministic) corpus. The default (seed=0) reproduces the crate gate-test words.
    let corpus_word = 0xC0FFEE_u64 ^ seed;
    let query_word = 0xDECAF_u64 ^ seed;

    let mut rows: Vec<(String, i64, f64)> = Vec::new();

    // ---- 1. HNSW recall@10 (in-RAM) — the tests/recall.rs gate, as a tracked deficit --------
    let store_path = tmp("hnsw.spqv", seed);
    let store = build_uniform_store(&store_path, n, corpus_word);

    // Build EXACTLY `iters` times (the best-of-`iters` is the reported build time) — do NOT build
    // an extra warm-up index outside the loop, or build runs iters+1× and skews the build metric.
    // The last-built index is retained for the recall measurement below.
    let mut build_us = f64::INFINITY;
    let mut index = {
        let t = Instant::now();
        let idx = VectorIndex::build(&store);
        build_us = build_us.min(t.elapsed().as_secs_f64() * 1e6);
        idx
    };
    for _ in 1..iters {
        let t = Instant::now();
        index = VectorIndex::build(&store);
        build_us = build_us.min(t.elapsed().as_secs_f64() * 1e6);
    }

    // The query set: fixed at `query_word`, independent of corpus generation (so the deficit
    // shifts only when ANN SEMANTICS change, never as a corpus-RNG artefact).
    let queries: Vec<Vec<f32>> = {
        let mut q_state = query_word;
        (0..QUERIES).map(|_| rand_vec(&mut q_state, DIM)).collect()
    };
    // Exact ground truth, computed once (it is the same every iter).
    let exact: Vec<Vec<u32>> = queries
        .iter()
        .map(|q| nearest_exact(&store, q, K).into_iter().map(|(id, _)| id).collect())
        .collect();

    {
        let (mut hits, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut h = 0usize;
            for (qi, q) in queries.iter().enumerate() {
                let approx: Vec<u32> = index.nearest(q, K).into_iter().map(|(id, _)| id).collect();
                h += approx.iter().filter(|id| exact[qi].contains(id)).count();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            hits = h;
        }
        let recall = hits as f64 / (QUERIES * K) as f64;
        eprintln!("[vector] HNSW recall@{K} = {recall:.4} (deficit {})", recall_deficit_milli(recall));
        rows.push(("hnsw_recall_at10".into(), recall_deficit_milli(recall), us));
    }

    // ---- 2. DiskANN (Vamana, on-disk .spqg) recall@10 — the tests/diskann.rs gate -----------
    {
        let graph_path = tmp("diskann.spqg", seed);
        let disk = DiskAnnIndex::build(&store, &graph_path).unwrap();
        let (mut hits, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut h = 0usize;
            for (qi, q) in queries.iter().enumerate() {
                let approx: Vec<u32> = disk.nearest(q, K).into_iter().map(|(id, _)| id).collect();
                h += approx.iter().filter(|id| exact[qi].contains(id)).count();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
            hits = h;
        }
        let recall = hits as f64 / (QUERIES * K) as f64;
        eprintln!("[vector] DiskANN recall@{K} = {recall:.4} (deficit {})", recall_deficit_milli(recall));
        rows.push(("diskann_recall_at10".into(), recall_deficit_milli(recall), us));
        // Drop the index (which may hold the on-disk .spqg open) BEFORE unlinking it, so the
        // remove succeeds on platforms with mandatory file locks (e.g. Windows). No-op on Linux.
        drop(disk);
        let _ = std::fs::remove_file(&graph_path);
    }

    // ---- 3. PQ + re-rank recall@10 (the DiskANN search loop) — the tests/quant.rs gate -------
    // PQ is gated on a CLUSTERED corpus (its design regime); the (N, seed) are scaled to keep the
    // per-commit run sub-second while staying the same shape the quant.rs recall gate uses.
    {
        const REORDER: usize = 50;
        let pq_path = tmp("pq.spqv", seed);
        let mut pq_state = corpus_word;
        // 40 clusters × 250 = 10k clustered vectors at spread 0.15 — the SAME clustered regime
        // `tests/quant.rs` gates PQ+re-rank at ≥0.95 on (PQ's design regime; the re-rank loop is
        // the DiskANN search loop). Held FIXED (independent of `n`, which scales only the HNSW /
        // DiskANN substrate) so the PQ deficit reproduces the documented near-exact re-rank recall.
        let pq_store = build_clustered_store(&pq_path, 40, 250, 0.15, &mut pq_state);
        let pq = ProductQuantizer::fit(DIM, pq_store.iter().map(|(_, v)| v), PqConfig::default())
            .unwrap();
        let enc = pq.encode_store(&pq_store).unwrap();

        let pq_queries: Vec<Vec<f32>> = {
            let mut q_state = query_word;
            (0..QUERIES).map(|_| rand_vec(&mut q_state, DIM)).collect()
        };
        let pq_exact: Vec<Vec<u32>> = pq_queries
            .iter()
            .map(|q| nearest_exact(&pq_store, q, K).into_iter().map(|(id, _)| id).collect())
            .collect();

        let (mut hits, mut us) = (0usize, f64::INFINITY);
        for _ in 0..iters {
            let t = Instant::now();
            let mut h = 0usize;
            for (qi, q) in pq_queries.iter().enumerate() {
                let table = DistanceTable::new(&pq, q);
                let cand = enc.rank_pq(&table, REORDER);
                let mut rescored: Vec<(u32, f32)> = cand
                    .into_iter()
                    .map(|(id, _)| (id, cosine(q, pq_store.get(id).unwrap())))
                    .collect();
                rescored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let reranked: Vec<u32> = rescored.into_iter().take(K).map(|(id, _)| id).collect();
                h += reranked.iter().filter(|id| pq_exact[qi].contains(id)).count();
            }
            us = us.min(t.elapsed().as_secs_f64() * 1e6 / pq_queries.len() as f64);
            hits = h;
        }
        let recall = hits as f64 / (QUERIES * K) as f64;
        eprintln!("[vector] PQ+re-rank recall@{K} = {recall:.4} (deficit {})", recall_deficit_milli(recall));
        rows.push(("pq_recall_at10".into(), recall_deficit_milli(recall), us));
        // Drop the encoded set + PQ + the file-backed store (which may hold `pq_path` open)
        // BEFORE unlinking, so the remove succeeds under mandatory file locks. No-op on Linux.
        drop(enc);
        drop(pq);
        drop(pq_store);
        let _ = std::fs::remove_file(&pq_path);
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, deficit, us) in &rows {
        println!("{name}\t{deficit}\t{us:.1}");
    }
    // The advisory HNSW build-time row (count is a sentinel 0 — build time is wall-clock, NOT a
    // deterministic gate; the row only forwards the advisory `vectors_build_s` timing).
    println!("build_s\t0\t{build_us:.1}");

    // Drop the index + the file-backed store (which may hold `store_path` open) BEFORE unlinking,
    // so the remove succeeds under mandatory file locks (e.g. Windows). No-op on Linux.
    drop(index);
    drop(store);
    let _ = std::fs::remove_file(&store_path);
}
