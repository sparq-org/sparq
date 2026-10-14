//! sq-0ut5x — self-relative build-time/query-time/recall Pareto sweep for sparq's
//! internal HNSW, DiskANN, and product-quantized indexes.
//!
//! Every emitted frontier row first clears the same recall@10 floor used by `bench/vector/run.sh`,
//! measured against [`nearest_exact`]. Timings are advisory unless gathered on a canonical quiet
//! box; the recall gate is always active.
//!
//! ```sh
//! cargo run -p sparq-vectors --release --example pareto_bench --features approx-ann -- --smoke
//! cargo run -p sparq-vectors --release --example pareto_bench --features approx-ann
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sparq_vectors::{
    cosine, nearest_exact, DiskAnnIndex, DistanceTable, HnswConfig, PqConfig, ProductQuantizer,
    VamanaConfig, VectorIndex, VectorStore,
};

const DIM: usize = 32;
const K: usize = 10;
const SEED: u64 = 0;

#[derive(Clone, Debug)]
struct Row {
    kind: &'static str,
    params: String,
    recall: f64,
    build_s: f64,
    query_us: f64,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn rand_vec(state: &mut u64) -> Vec<f32> {
    (0..DIM)
        .map(|_| ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 1.0)
        .collect()
}

fn build_uniform_store(path: &Path, n: usize) -> VectorStore {
    let mut store = VectorStore::create(path, DIM).expect("create uniform corpus");
    let mut state = 0xC0FFEE_u64 ^ SEED;
    for i in 0..n {
        store
            .put((i as u32) * 7 + 3, &rand_vec(&mut state))
            .expect("write vector");
    }
    store.finalize().expect("finalize uniform corpus");
    store
}

fn build_clustered_store(path: &Path, clusters: usize, per: usize) -> VectorStore {
    let mut store = VectorStore::create(path, DIM).expect("create clustered corpus");
    let mut state = 0xC0FFEE_u64 ^ SEED;
    let mut id = 1u32;
    for _ in 0..clusters {
        let center = rand_vec(&mut state);
        for _ in 0..per {
            let point: Vec<f32> = center
                .iter()
                .map(|x| {
                    x + ((splitmix64(&mut state) >> 40) as f32 / (1u64 << 23) as f32 - 0.5) * 0.15
                })
                .collect();
            store.put(id, &point).expect("write clustered vector");
            id += 1;
        }
    }
    store.finalize().expect("finalize clustered corpus");
    store
}

fn queries(count: usize) -> Vec<Vec<f32>> {
    let mut state = 0xDECAF_u64 ^ SEED;
    (0..count).map(|_| rand_vec(&mut state)).collect()
}

fn truth(store: &VectorStore, queries: &[Vec<f32>]) -> Vec<Vec<u32>> {
    queries
        .iter()
        .map(|q| {
            nearest_exact(store, q, K)
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        })
        .collect()
}

fn measure<F>(queries: &[Vec<f32>], exact: &[Vec<u32>], mut search: F) -> (f64, f64)
where
    F: FnMut(&[f32]) -> Vec<u32>,
{
    let start = Instant::now();
    let mut hits = 0usize;
    for (query, expected) in queries.iter().zip(exact) {
        hits += search(query)
            .iter()
            .filter(|id| expected.contains(id))
            .count();
    }
    let query_us = start.elapsed().as_secs_f64() * 1e6 / queries.len() as f64;
    (hits as f64 / (queries.len() * K) as f64, query_us)
}

fn floor(kind: &str) -> f64 {
    match kind {
        "diskann" => 0.90,
        "hnsw" | "pq" => 0.95,
        _ => unreachable!("unknown index kind"),
    }
}

/// Enforce recall before computing a frontier. This ordering is the harness's central invariant.
fn gated_frontier(rows: Vec<Row>) -> Result<Vec<Row>, String> {
    for row in &rows {
        let required = floor(row.kind);
        if row.recall + f64::EPSILON < required {
            return Err(format!(
                "{} ({}) recall@{K} {:.4} is below floor {required:.2}; no envelope emitted",
                row.kind, row.params, row.recall
            ));
        }
    }

    let mut frontier = Vec::new();
    for candidate in &rows {
        let dominated = rows.iter().any(|other| {
            other.kind == candidate.kind
                && (other.build_s <= candidate.build_s)
                && (other.query_us <= candidate.query_us)
                && (other.recall >= candidate.recall)
                && ((other.build_s < candidate.build_s)
                    || (other.query_us < candidate.query_us)
                    || (other.recall > candidate.recall))
        });
        if !dominated {
            frontier.push(candidate.clone());
        }
    }
    frontier.sort_by(|a, b| a.kind.cmp(b.kind).then(a.build_s.total_cmp(&b.build_s)));
    Ok(frontier)
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sparq-vector-pareto-{name}-{}", std::process::id()))
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn emit(rows: &[Row], smoke: bool, corpus_n: usize, query_count: usize) {
    println!("{{");
    println!("  \"schema\": \"sparq.vector-pareto.v1\",");
    println!("  \"canonical\": false,");
    println!("  \"smoke\": {smoke},");
    println!("  \"corpus\": {{\"seed\": {SEED}, \"n\": {corpus_n}, \"dim\": {DIM}, \"queries\": {query_count}}},");
    println!("  \"frontier\": [");
    for (i, row) in rows.iter().enumerate() {
        let comma = if i + 1 == rows.len() { "" } else { "," };
        println!(
            "    {{\"kind\":\"{}\",\"params\":\"{}\",\"recall_at10\":{:.6},\"recall_floor\":{:.2},\"build_s\":{:.6},\"query_us\":{:.3}}}{comma}",
            row.kind,
            json_escape(&row.params),
            row.recall,
            floor(row.kind),
            row.build_s,
            row.query_us
        );
    }
    println!("  ]");
    println!("}}");
}

fn main() {
    let smoke = std::env::args().skip(1).any(|arg| arg == "--smoke");
    let (n, query_count) = if smoke { (1_000, 12) } else { (50_000, 100) };
    let store_path = tmp("uniform.spqv");
    let store = build_uniform_store(&store_path, n);
    let qs = queries(query_count);
    let exact = truth(&store, &qs);
    let mut rows = Vec::new();

    let hnsw_builds: &[usize] = if smoke { &[40] } else { &[40, 100, 200] };
    for &ef_construction in hnsw_builds {
        let cfg = HnswConfig {
            ef_construction,
            ..HnswConfig::default()
        };
        let start = Instant::now();
        let index = VectorIndex::build_with(&store, cfg);
        let build_s = start.elapsed().as_secs_f64();
        let (recall, query_us) = measure(&qs, &exact, |q| {
            index.nearest(q, K).into_iter().map(|(id, _)| id).collect()
        });
        rows.push(Row {
            kind: "hnsw",
            params: format!("ef_construction={ef_construction}"),
            recall,
            build_s,
            query_us,
        });
    }

    let disk_builds: &[(usize, usize)] = if smoke {
        &[(16, 50)]
    } else {
        &[(16, 50), (32, 100)]
    };
    for &(degree, build_beam) in disk_builds {
        let graph_path = tmp(&format!("diskann-{degree}.spqg"));
        let cfg = VamanaConfig {
            degree,
            build_beam,
            search_beam: 100,
            ..VamanaConfig::default()
        };
        let start = Instant::now();
        let index = DiskAnnIndex::build_with(&store, &graph_path, cfg).expect("build DiskANN");
        let build_s = start.elapsed().as_secs_f64();
        let (recall, query_us) = measure(&qs, &exact, |q| {
            index.nearest(q, K).into_iter().map(|(id, _)| id).collect()
        });
        rows.push(Row {
            kind: "diskann",
            params: format!("degree={degree},build_beam={build_beam}"),
            recall,
            build_s,
            query_us,
        });
        drop(index);
        let _ = std::fs::remove_file(graph_path);
    }

    // PQ uses the same deterministic clustered regime as bench_vectors/tests/quant.rs.
    let pq_path = tmp("pq.spqv");
    let pq_store = build_clustered_store(&pq_path, if smoke { 16 } else { 40 }, 250);
    let pq_exact = truth(&pq_store, &qs);
    let pq_builds: &[(usize, usize)] = if smoke {
        &[(16, 50)]
    } else {
        &[(8, 40), (16, 50)]
    };
    for &(m, reorder) in pq_builds {
        let cfg = PqConfig {
            m,
            ..PqConfig::default()
        };
        let start = Instant::now();
        let pq = ProductQuantizer::fit(DIM, pq_store.iter().map(|(_, v)| v), cfg).expect("fit PQ");
        let encoded = pq.encode_store(&pq_store).expect("encode PQ store");
        let build_s = start.elapsed().as_secs_f64();
        let (recall, query_us) = measure(&qs, &pq_exact, |q| {
            let table = DistanceTable::new(&pq, q);
            let mut rescored: Vec<(u32, f32)> = encoded
                .rank_pq(&table, reorder)
                .into_iter()
                .map(|(id, _)| (id, cosine(q, pq_store.get(id).expect("candidate exists"))))
                .collect();
            rescored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            rescored.into_iter().take(K).map(|(id, _)| id).collect()
        });
        rows.push(Row {
            kind: "pq",
            params: format!("m={m},reorder={reorder}"),
            recall,
            build_s,
            query_us,
        });
    }

    let frontier = gated_frontier(rows).unwrap_or_else(|message| {
        eprintln!("[vector-pareto] ERROR: {message}");
        std::process::exit(1);
    });
    let kinds: BTreeMap<_, _> = frontier.iter().map(|row| (row.kind, ())).collect();
    for kind in ["hnsw", "diskann", "pq"] {
        assert!(kinds.contains_key(kind), "frontier lost index kind {kind}");
    }
    emit(&frontier, smoke, n, query_count);

    drop(pq_store);
    drop(store);
    let _ = std::fs::remove_file(pq_path);
    let _ = std::fs::remove_file(store_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &'static str, recall: f64, build_s: f64, query_us: f64) -> Row {
        Row {
            kind,
            params: "test".into(),
            recall,
            build_s,
            query_us,
        }
    }

    #[test]
    fn recall_gate_rejects_a_row_before_envelope_construction() {
        let error = gated_frontier(vec![row("diskann", 0.899, 1.0, 1.0)]).unwrap_err();
        assert!(error.contains("below floor"));
    }

    #[test]
    fn pareto_filter_removes_only_strictly_dominated_rows() {
        let frontier = gated_frontier(vec![
            row("hnsw", 0.96, 1.0, 2.0),
            row("hnsw", 0.96, 2.0, 3.0),
            row("hnsw", 0.98, 3.0, 1.0),
        ])
        .unwrap();
        assert_eq!(frontier.len(), 2);
        assert!(!frontier.iter().any(|candidate| candidate.build_s == 2.0));
    }
}
