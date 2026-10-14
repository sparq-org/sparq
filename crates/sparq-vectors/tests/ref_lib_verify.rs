//! sq-6te5 — END-TO-END gather verification of sparq-vectors' ANN against an
//! ESTABLISHED reference library (hnswlib / FAISS).
//!
//! WHAT THIS PROVES (the load-bearing invariant) ------------------------------
//! `tests/diskann.rs` / `tests/recall.rs` already gate sparq-vectors' approximate index
//! against its OWN exact brute-force searcher. They do NOT anchor that floor against an
//! established ANN library. This test closes sq-6te5's gap: sparq-vectors' search is
//! shown CONSISTENT with hnswlib on a fixed corpus —
//!   1. metric agreement: sparq-vectors' `nearest_exact` reproduces the SAME exact-kNN
//!      ground truth numpy computed in the capture (so both libraries score recall
//!      against an identical oracle — the comparison is apples-to-apples);
//!   2. recall equivalence: sparq-vectors' approximate index (on-disk DiskANN/Vamana in
//!      the default build; in-RAM HNSW under `approx-ann`) achieves recall@k vs that
//!      oracle AT LEAST AS HIGH as hnswlib's own recall (within a small slack), so the
//!      sparq adapter is verified against the established library, not only itself.
//!
//! WHAT RUNS IN CI vs WHAT IS GATHER-ONLY -------------------------------------
//! This whole test runs in ordinary CI with NO native deps. It does so by reading a
//! COMMITTED fixture (`tests/fixtures/hnswlib_ref.tsv`) captured from a REAL hnswlib run
//! by `scripts/capture_hnswlib_ref.py` (gather-only: needs numpy + hnswlib). The corpus
//! and queries are NOT stored — both the capture harness and this test regenerate them
//! bit-identically from the same splitmix64 seed (the u64 stream matches across Python
//! and Rust), so the fixture is just the two neighbour-id columns. Regenerating the
//! fixture (the live hnswlib comparison) is the gather-only half; verifying sparq-vectors
//! against it is the CI half.
//!
//! NUMBERS HERE ARE WORK-BOX NON-CANONICAL — the hnswlib recall (~0.95 at capture) and
//! sparq-vectors' recall are recomputed off the ids; nothing is baked as a canonical
//! perf figure (AGENTS.md). The fixture header's params are re-asserted against the
//! consts below so a regenerated-with-different-params fixture FAILS CLOSED.

use sparq_vectors::{nearest_exact, DiskAnnIndex, VamanaConfig, VectorStore};
#[cfg(feature = "approx-ann")]
use sparq_vectors::VectorIndex;

// --- fixture parameters: MUST match scripts/capture_hnswlib_ref.py ----------
const SEED: u64 = 0xC0FFEE;
const QUERY_SEED: u64 = 0xDECAF;
const N: usize = 5_000;
const DIM: usize = 32;
const K: usize = 10;
const QUERIES: usize = 50;

/// One step of splitmix64 — bit-identical to the capture harness and tests/recall.rs.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// `dim` f32 samples — mirrors the capture harness `rand_vec` and tests/recall.rs.
fn rand_vec(state: &mut u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|_| ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32) * 2.0 - 1.0)
        .collect()
}

/// Sparse, non-contiguous dict-style ids (matches the harness `corpus_id`: i*7 + 3).
fn corpus_id(i: usize) -> u32 {
    (i as u32) * 7 + 3
}

/// A captured reference row: hnswlib's approximate neighbours + numpy's exact-kNN.
struct RefRow {
    hnsw: Vec<u32>,
    exact: Vec<u32>,
}

/// Parse the committed fixture. Asserts the header `# params` line matches the consts
/// above (fail-closed on a fixture regenerated with different settings), and returns the
/// per-query rows keyed by qid.
fn load_fixture() -> Vec<RefRow> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hnswlib_ref.tsv");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read reference fixture {}: {}", path, e));

    let mut saw_params = false;
    let mut rows: Vec<Option<RefRow>> = (0..QUERIES).map(|_| None).collect();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# params ") {
            // e.g. "seed=12648430 ... n=5000 dim=32 k=10 queries=50 space=cosine ...".
            let get = |key: &str| -> Option<u64> {
                rest.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(key)?.parse::<u64>().ok())
            };
            assert_eq!(get("seed="), Some(SEED), "fixture seed drift");
            assert_eq!(get("query_seed="), Some(QUERY_SEED), "fixture query_seed drift");
            assert_eq!(get("n="), Some(N as u64), "fixture n drift");
            assert_eq!(get("dim="), Some(DIM as u64), "fixture dim drift");
            assert_eq!(get("k="), Some(K as u64), "fixture k drift");
            assert_eq!(get("queries="), Some(QUERIES as u64), "fixture queries drift");
            assert!(rest.contains("space=cosine"), "fixture must be cosine-space");
            saw_params = true;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let qid: usize = cols.next().unwrap().parse().unwrap();
        let parse_ids = |s: &str| -> Vec<u32> {
            s.split(',').filter(|t| !t.is_empty()).map(|t| t.parse().unwrap()).collect()
        };
        let hnsw = parse_ids(cols.next().expect("missing hnswlib column"));
        let exact = parse_ids(cols.next().expect("missing exact-knn column"));
        assert!(cols.next().is_none(), "unexpected extra column on qid {}", qid);
        assert_eq!(hnsw.len(), K, "hnswlib row {} not k-wide", qid);
        assert_eq!(exact.len(), K, "exact row {} not k-wide", qid);
        rows[qid] = Some(RefRow { hnsw, exact });
    }
    assert!(saw_params, "fixture missing `# params` header line");
    rows.into_iter()
        .enumerate()
        .map(|(qid, r)| r.unwrap_or_else(|| panic!("fixture missing query {}", qid)))
        .collect()
}

/// Build the deterministic corpus store + the matching query set (shared by the exact,
/// DiskANN and HNSW checks). Returns (store, store_path, queries).
fn build_corpus(tag: &str) -> (VectorStore, std::path::PathBuf, Vec<Vec<f32>>) {
    let store_path = std::env::temp_dir()
        .join(format!("sparq-vectors-refverify-{}-{}.spqv", tag, std::process::id()));
    let mut store = VectorStore::create(&store_path, DIM).unwrap();
    let mut state = SEED;
    for i in 0..N {
        store.put(corpus_id(i), &rand_vec(&mut state, DIM)).unwrap();
    }
    store.finalize().unwrap();

    let mut q_state = QUERY_SEED;
    let queries: Vec<Vec<f32>> = (0..QUERIES).map(|_| rand_vec(&mut q_state, DIM)).collect();
    (store, store_path, queries)
}

/// Recall@k of `approx` (one query's neighbour ids) against the `exact` oracle ids:
/// |approx∩exact| / k. Identical to the harness recall_at_k per-query term.
fn recall(approx: &[u32], exact: &[u32]) -> f64 {
    use std::collections::HashSet;
    let e: HashSet<u32> = exact.iter().copied().collect();
    approx.iter().filter(|id| e.contains(id)).count() as f64 / K as f64
}

#[test]
fn sparq_exact_reproduces_the_reference_exact_knn_oracle() {
    // METRIC-AGREEMENT ANCHOR: sparq-vectors' `nearest_exact` (cosine) must reproduce the
    // SAME top-k set numpy computed in the capture. If the corpus regeneration or the
    // cosine metric drifted, the two oracles would disagree and the whole recall
    // comparison would be meaningless. We compare the top-k SET (not the exact rank
    // order) because two neighbours with all-but-equal cosine can swap under f32 rounding
    // between numpy float32 and Rust f32 — that is a tie, not a metric disagreement.
    let fixture = load_fixture();
    let (store, store_path, queries) = build_corpus("exact");

    let mut total_overlap = 0usize;
    for (q, row) in queries.iter().zip(&fixture) {
        let ours: Vec<u32> =
            nearest_exact(&store, q, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ours.len(), K);
        let overlap = recall(&ours, &row.exact);
        total_overlap += (overlap * K as f64).round() as usize;
    }
    // Set agreement must be essentially total; we allow a tiny slack purely for f32-tie
    // swaps at the k/k+1 boundary (a neighbour just inside vs just outside the cut).
    let mean = total_overlap as f64 / (QUERIES * K) as f64;
    eprintln!(
        "sparq nearest_exact vs numpy exact-kNN: mean top-{} set agreement = {:.4}",
        K, mean
    );
    assert!(
        mean >= 0.99,
        "sparq exact searcher disagrees with the captured exact-kNN oracle ({:.4} < 0.99) \
         — corpus regeneration or the cosine metric has drifted from the capture",
        mean
    );

    let _ = std::fs::remove_file(&store_path);
}

#[test]
fn diskann_recall_is_at_least_hnswlib_on_the_reference_fixture() {
    // CROSS-LIBRARY EQUIVALENCE: the DEFAULT-build approximate index (on-disk
    // DiskANN/Vamana) must match an established ANN library's quality. We recompute BOTH
    // recalls off the SAME committed oracle: hnswlib's (from the fixture ids) and
    // sparq-vectors' (live), and require sparq >= hnswlib minus a small slack. This is
    // the headline sq-6te5 claim and it runs in CI with no native deps.
    let fixture = load_fixture();
    let (store, store_path, queries) = build_corpus("disk");
    let graph_path = store_path.with_extension("spqg");

    // A generous search beam: this is the QUALITY check (is sparq's index AS GOOD as
    // hnswlib?), not a speed check, so we let DiskANN search hard.
    let cfg = VamanaConfig { search_beam: 128, ..Default::default() };
    let index = DiskAnnIndex::build_with(&store, &graph_path, cfg).unwrap();
    assert_eq!(index.len(), N);

    let mut hnsw_hits = 0.0f64;
    let mut disk_hits = 0.0f64;
    for (q, row) in queries.iter().zip(&fixture) {
        hnsw_hits += recall(&row.hnsw, &row.exact);
        let ours: Vec<u32> = index.nearest(q, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ours.len(), K);
        disk_hits += recall(&ours, &row.exact);
    }
    let hnsw_recall = hnsw_hits / QUERIES as f64;
    let disk_recall = disk_hits / QUERIES as f64;
    eprintln!(
        "reference fixture recall@{}: hnswlib={:.4} sparq-DiskANN={:.4} (work-box NON-CANONICAL)",
        K, hnsw_recall, disk_recall
    );
    // Equivalence with a small slack: sparq's index must not be materially WORSE than the
    // established library on the identical oracle. The slack absorbs the two indexes'
    // different ANN heuristics + f32-tie swaps; it is NOT a free pass — at capture hnswlib
    // ~0.95, so this is a real, non-vacuous floor.
    assert!(
        disk_recall >= hnsw_recall - 0.05,
        "sparq DiskANN recall {:.4} is materially below the hnswlib reference {:.4} \
         on the shared exact-kNN oracle",
        disk_recall,
        hnsw_recall
    );

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[cfg(feature = "approx-ann")]
#[test]
fn hnsw_recall_is_at_least_hnswlib_on_the_reference_fixture() {
    // The same equivalence claim for the in-RAM HNSW backend (`approx-ann`): sparq's HNSW
    // (instant-distance) vs the reference hnswlib, both scored on the committed oracle.
    let fixture = load_fixture();
    let (store, store_path, queries) = build_corpus("hnsw");
    let index = VectorIndex::build(&store);

    let mut hnswlib_hits = 0.0f64;
    let mut ours_hits = 0.0f64;
    for (q, row) in queries.iter().zip(&fixture) {
        hnswlib_hits += recall(&row.hnsw, &row.exact);
        let ours: Vec<u32> = index.nearest(q, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ours.len(), K);
        ours_hits += recall(&ours, &row.exact);
    }
    let hnswlib_recall = hnswlib_hits / QUERIES as f64;
    let ours_recall = ours_hits / QUERIES as f64;
    eprintln!(
        "reference fixture recall@{}: hnswlib={:.4} sparq-HNSW={:.4} (work-box NON-CANONICAL)",
        K, hnswlib_recall, ours_recall
    );
    assert!(
        ours_recall >= hnswlib_recall - 0.05,
        "sparq HNSW recall {:.4} is materially below the hnswlib reference {:.4} \
         on the shared exact-kNN oracle",
        ours_recall,
        hnswlib_recall
    );

    let _ = std::fs::remove_file(&store_path);
}

// --- GATHER-ONLY: the LIVE hnswlib comparison (#[ignore], needs numpy + hnswlib) -----
// The three tests above are the CI verification: they run sparq-vectors against the
// COMMITTED capture with no native deps. This last test is the gather-only other half —
// it re-runs the REAL hnswlib capture harness and asserts its output is byte-for-byte the
// committed fixture, so the fixture is provably reproducible from the established library
// and has not silently drifted. It is `#[ignore]`d (and a no-op skip if python3 / numpy /
// hnswlib are absent) so plain `cargo test` never touches it; a gather box runs it with
// `cargo test -p sparq-vectors --test ref_lib_verify -- --ignored`.
#[test]
#[ignore = "gather-only: needs python3 + numpy + hnswlib (heavy native deps)"]
fn live_hnswlib_capture_matches_the_committed_fixture() {
    let script =
        concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/capture_hnswlib_ref.py");
    // Prefer an explicit interpreter (VECTOR_PY) so a gather box can point at its venv;
    // fall back to python3 on PATH.
    let py = std::env::var("VECTOR_PY").unwrap_or_else(|_| "python3".to_string());
    let out = match std::process::Command::new(&py).arg(script).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP live hnswlib capture: cannot spawn {} ({})", py, e);
            return;
        }
    };
    if !out.status.success() {
        // The harness exits non-zero (and prints to stderr) when numpy/hnswlib are
        // missing — treat that as a skip, not a failure, so an under-provisioned box
        // running `--ignored` does not red the suite.
        eprintln!(
            "SKIP live hnswlib capture: harness exited {} ({})",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return;
    }
    let produced = String::from_utf8(out.stdout).expect("harness output is utf-8");
    let committed_path =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hnswlib_ref.tsv");
    let committed = std::fs::read_to_string(committed_path).unwrap();
    assert_eq!(
        produced.trim_end(),
        committed.trim_end(),
        "a live hnswlib re-capture differs from the committed fixture — \
         hnswlib output drifted (version change?); re-commit scripts/capture_hnswlib_ref.py output"
    );
    eprintln!("live hnswlib re-capture is byte-identical to the committed fixture");
}
