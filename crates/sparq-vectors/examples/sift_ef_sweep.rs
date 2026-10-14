//! (sq-z2z18) SIFT1M ef_search sweep for sparq-vectors HNSW.
//!
//! Reads pre-extracted SIFT1M base vectors and query vectors from raw binary files.
//! For each ef level in [16, 32, 64, 128, 256]:
//!   - Builds a VectorStore + VectorIndex (cosine, normalised) with that ef_search
//!   - Queries all test vectors, scores recall@10 vs brute-force cosine ground truth
//!   - Records QPS
//!
//! This gives the recall–QPS Pareto frontier for sparq-vectors on SIFT1M(subset).
//!
//! METRIC: sparq-vectors VectorIndex normalises vectors and uses cosine similarity.
//! Both the database vectors and query vectors are L2-normalised before indexing.
//! Ground truth is brute-force cosine on the normalised corpus.
//! The hnswlib comparison row in the gap-record uses the same metric (cosine on normalised
//! SIFT1M vectors, measured via FAISS IP ground truth).
//!
//! CORPUS SIZE: instant-distance HNSW build at 1M×128d exceeds 30 minutes on this box
//! (documented as a build-time gap). The Pareto sweep uses N_BASE=100_000 vectors
//! (the first 100k from SIFT1M) to complete the ef sweep in reasonable time.
//! The recall-QPS shape is representative; absolute QPS scales with corpus size.
//!
//! Input binary format (written by extract_sift.py):
//!   bytes 0..4   n:   u32 LE  — number of vectors
//!   bytes 4..8   dim: u32 LE  — dimension
//!   bytes 8..    data: [n * dim] f32 LE
//!
//! Usage:
//!   cargo run --release -p sparq-vectors --example sift_ef_sweep --features approx-ann -- \
//!       /tmp/sift-sweep/base.bin /tmp/sift-sweep/query.bin [n_base] [k] [n_reps]

use std::io::Read;
use std::time::Instant;

use sparq_vectors::{cosine, HnswConfig, VectorIndex, VectorStore};

const K: usize = 10;
const DEFAULT_REPS: usize = 3;
/// Number of base vectors to index (first N from SIFT1M).
const DEFAULT_N_BASE: usize = 100_000;
/// Number of query vectors to use.
const MAX_QUERIES: usize = 10_000;

fn read_raw_f32(path: &str, max_n: usize) -> (usize, usize, Vec<f32>) {
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let mut hdr = [0u8; 8];
    f.read_exact(&mut hdr).unwrap();
    let n_total = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
    let dim = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
    let n = n_total.min(max_n);
    let mut buf = vec![0u8; n * dim * 4];
    f.read_exact(&mut buf).unwrap();
    let floats: Vec<f32> = buf
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    (n, dim, floats)
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-9 {
        vec![0.0; v.len()]
    } else {
        v.iter().map(|x| x / norm).collect()
    }
}

/// Exact top-k by cosine similarity against normalised base vectors.
fn exact_topk(query_norm: &[f32], base_norm: &[Vec<f32>], k: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = base_norm
        .iter()
        .enumerate()
        .map(|(i, v)| (i, cosine(query_norm, v)))
        .collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(k);
    scored.into_iter().map(|(i, _)| i).collect()
}

fn p99(mut us: Vec<f64>) -> f64 {
    us.sort_by(|a, b| a.total_cmp(b));
    let idx = ((0.99 * us.len() as f64) as usize).min(us.len().saturating_sub(1));
    us[idx]
}

fn main() {
    let base_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/sift-sweep/base.bin".to_string());
    let query_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/sift-sweep/query.bin".to_string());
    let n_base: usize = std::env::args()
        .nth(3)
        .and_then(|a| a.parse().ok())
        .unwrap_or(DEFAULT_N_BASE);
    let k: usize = std::env::args()
        .nth(4)
        .and_then(|a| a.parse().ok())
        .unwrap_or(K);
    let n_reps: usize = std::env::args()
        .nth(5)
        .and_then(|a| a.parse().ok())
        .unwrap_or(DEFAULT_REPS);

    eprintln!("[sift-sweep] reading base vectors from {base_path} (max n={n_base})");
    let (n_base_actual, dim, base_raw) = read_raw_f32(&base_path, n_base);
    eprintln!("[sift-sweep] base: n={n_base_actual} dim={dim}");

    eprintln!("[sift-sweep] reading query vectors from {query_path}");
    let (n_query_raw, qdim, query_raw) = read_raw_f32(&query_path, MAX_QUERIES);
    assert_eq!(dim, qdim, "query dim {qdim} != base dim {dim}");
    eprintln!("[sift-sweep] queries: {n_query_raw} (dim={dim})");

    // ---- Normalise vectors (cosine metric) -----------------------------------------------
    eprintln!("[sift-sweep] normalising {n_base_actual} base + {n_query_raw} query vectors...");
    let base_norm: Vec<Vec<f32>> = (0..n_base_actual)
        .map(|i| normalize(&base_raw[i * dim..(i + 1) * dim]))
        .collect();
    let query_norm: Vec<Vec<f32>> = (0..n_query_raw)
        .map(|i| normalize(&query_raw[i * dim..(i + 1) * dim]))
        .collect();
    eprintln!("[sift-sweep] normalisation done");

    // ---- Compute exact cosine ground truth -----------------------------------------------
    eprintln!("[sift-sweep] computing exact cosine ground truth for {n_query_raw} queries × {n_base_actual} base...");
    let t_gt = Instant::now();
    let ground_truth: Vec<Vec<usize>> = query_norm
        .iter()
        .map(|q| exact_topk(q, &base_norm, k))
        .collect();
    let gt_s = t_gt.elapsed().as_secs_f64();
    eprintln!("[sift-sweep] ground truth done in {gt_s:.1}s");

    // ---- ef sweep: build fresh index at each ef, measure recall + QPS -------------------
    let ef_values: &[usize] = &[16, 32, 64, 128, 256];
    println!("ef\trecall@{k}\tdeficit_milli\tmean_us\tp99_us\tqps\tbuild_s");

    for &ef in ef_values {
        eprintln!("[sift-sweep] ef={ef}: building VectorStore...");

        // Build VectorStore with normalised vectors
        let store_path = std::env::temp_dir()
            .join(format!("sift-ef{ef}-{}.spqv", std::process::id()));
        {
            let mut store = VectorStore::create(&store_path, dim)
                .unwrap_or_else(|e| panic!("create store ef={ef}: {e}"));
            for (i, v) in base_norm.iter().enumerate() {
                store.put(i as u32, v).unwrap_or_else(|e| panic!("put {i}: {e}"));
            }
            store.finalize().unwrap_or_else(|e| panic!("finalize ef={ef}: {e}"));
        }
        let store = VectorStore::open(&store_path)
            .unwrap_or_else(|e| panic!("open store ef={ef}: {e}"));

        // Build HNSW index with ef_search = ef
        let cfg = HnswConfig {
            ef_search: ef,
            ef_construction: 200,
            seed: 0x5350_5156_0001,
        };
        eprintln!("[sift-sweep] ef={ef}: building HNSW index (ef_construction=200)...");
        let t_build = Instant::now();
        let index = VectorIndex::build_with(&store, cfg);
        let build_s = t_build.elapsed().as_secs_f64();
        eprintln!("[sift-sweep] ef={ef}: build done in {build_s:.1}s");

        // Query sweep
        let mut all_latencies: Vec<f64> = Vec::with_capacity(n_reps * n_query_raw);
        let mut last_labels: Vec<Vec<u32>> = Vec::new();

        for rep in 0..n_reps {
            let t0 = Instant::now();
            let mut rep_labels: Vec<Vec<u32>> = Vec::with_capacity(n_query_raw);
            for q in &query_norm {
                let approx = index.nearest(q, k);
                rep_labels.push(approx.into_iter().map(|(id, _)| id).collect());
            }
            let elapsed_us = t0.elapsed().as_secs_f64() * 1e6;
            let per_query_us = elapsed_us / n_query_raw as f64;
            for _ in 0..n_query_raw {
                all_latencies.push(per_query_us);
            }
            if rep == n_reps - 1 {
                last_labels = rep_labels;
            }
        }

        // Score recall vs ground truth
        let mut hits = 0usize;
        for (qi, labels) in last_labels.iter().enumerate() {
            let gt_set: std::collections::HashSet<usize> =
                ground_truth[qi].iter().copied().collect();
            for &id in labels {
                if gt_set.contains(&(id as usize)) {
                    hits += 1;
                }
            }
        }

        let recall = hits as f64 / (n_query_raw * k) as f64;
        let deficit_milli = ((1.0 - recall) * 1000.0).round() as i64;
        let mean_us = all_latencies.iter().sum::<f64>() / all_latencies.len() as f64;
        let p99_us = p99(all_latencies);
        let qps = if mean_us > 0.0 { 1_000_000.0 / mean_us } else { 0.0 };

        eprintln!(
            "[sift-sweep] ef={ef}: recall@{k}={recall:.4} deficit={deficit_milli} \
             mean_us={mean_us:.1} p99_us={p99_us:.1} qps={qps:.1} build_s={build_s:.1}"
        );
        println!("{ef}\t{recall:.4}\t{deficit_milli}\t{mean_us:.1}\t{p99_us:.1}\t{qps:.1}\t{build_s:.1}");

        // Cleanup store
        drop(index);
        drop(store);
        let _ = std::fs::remove_file(&store_path);
    }

    println!(
        "# n_base={n_base_actual} dim={dim} k={k} n_query={n_query_raw} n_reps={n_reps} \
         metric=cosine_on_normalised_vectors"
    );
}
