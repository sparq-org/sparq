//! (sq-1wc1) Predicate-constrained (filtered) ANN gate. Compiled only with the opt-in
//! `filtered-ann` feature (`cargo test -p sparq-vectors --features filtered-ann`).
//!
//! Covers:
//!  1. **recall** of filtered traversal (`DiskAnnIndex::nearest_filtered` over a *broad* mask) vs.
//!     the exact-filtered ground truth (`nearest_exact_filtered`) on a 20k synthetic set;
//!  2. the **pre-filter** strategy is exact (a selective mask matches the ground truth exactly);
//!  3. **selectivity edge cases**: empty mask → no results, full mask → equals the unfiltered
//!     search, single-element mask → that one id (when reachable / in store);
//!  4. the **HNSW** filtered entry point (`VectorIndex::nearest_filtered`, exact pre-filter) agrees
//!     with the ground truth;
//!  5. `IdMask` / `FilterConfig` mechanics (construction, `prefer_prefilter` crossover).

#![cfg(feature = "filtered-ann")]

use sparq_vectors::{
    nearest_exact, nearest_exact_filtered, DiskAnnIndex, FilterConfig, IdMask, VectorStore,
};
// (sq-ip3a) The HNSW filtered entry point is the approximate backend — `approx-ann` only.
#[cfg(feature = "approx-ann")]
use sparq_vectors::VectorIndex;

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

/// Builds an N×DIM store with sparse ids `i*7+3`, returns (store, store_path, all_ids).
fn build_store(n: usize, dim: usize, path: &std::path::Path) -> (VectorStore, Vec<u32>) {
    let mut store = VectorStore::create(path, dim).unwrap();
    let mut state = 0xC0FFEE_u64;
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let id = (i as u32) * 7 + 3;
        store.put(id, &rand_vec(&mut state, dim)).unwrap();
        ids.push(id);
    }
    store.finalize().unwrap();
    (store, ids)
}

#[test]
fn filtered_traversal_recall_vs_exact_on_broad_mask() {
    // A broad mask (~half the store passes) forces the FILTERED-TRAVERSAL strategy (above the
    // pre-filter crossover) and measures its recall against the exact-filtered ground truth.
    const N: usize = 20_000;
    const DIM: usize = 32;
    const K: usize = 10;
    const QUERIES: usize = 100;

    let store_path = tmp("filtered-recall.spqv");
    let graph_path = tmp("filtered-recall.spqg");
    let (store, ids) = build_store(N, DIM, &store_path);
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();

    // ~50% mask: keep every other id. Well above the 1% pre-filter line, so traversal runs.
    let mask: IdMask = ids.iter().copied().step_by(2).collect();
    assert!(
        !FilterConfig::default().prefer_prefilter(mask.len(), N),
        "a ~50% mask must take the traversal path, not pre-filter"
    );

    let mut q_state = 0xDECAF_u64;
    let mut hits = 0usize;
    let mut total = 0usize;
    for _ in 0..QUERIES {
        let q = rand_vec(&mut q_state, DIM);
        let truth: Vec<u32> = nearest_exact_filtered(&store, &q, &mask, K)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let got: Vec<u32> = idx
            .nearest_filtered(&q, &mask, &store, K)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        // Every returned id must be in the mask (the hard guarantee).
        for id in &got {
            assert!(
                mask.contains(*id),
                "filtered search returned an unmasked id {id}"
            );
        }
        total += truth.len();
        hits += got.iter().filter(|id| truth.contains(id)).count();
    }
    let recall = hits as f64 / total as f64;
    eprintln!("filtered-traversal recall@{K} over {QUERIES} queries (~50% mask) on {N}x{DIM}: {recall:.4}");
    assert!(
        recall >= 0.90,
        "filtered recall gate failed: {recall:.4} < 0.90"
    );

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn selective_mask_prefilter_is_exact() {
    // A selective mask (≤ 1% / floor) takes the exact PRE-FILTER path; it must match the
    // ground truth EXACTLY (pre-filter is exact, not approximate).
    const N: usize = 20_000;
    const DIM: usize = 32;
    const K: usize = 10;

    let store_path = tmp("filtered-selective.spqv");
    let graph_path = tmp("filtered-selective.spqg");
    let (store, ids) = build_store(N, DIM, &store_path);
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();

    // 50 ids — under the 256 floor → pre-filter.
    let mask: IdMask = ids.iter().copied().take(50).collect();
    assert!(FilterConfig::default().prefer_prefilter(mask.len(), N));

    let mut q_state = 0x1234_u64;
    for _ in 0..50 {
        let q = rand_vec(&mut q_state, DIM);
        let truth = nearest_exact_filtered(&store, &q, &mask, K);
        let got = idx.nearest_filtered(&q, &mask, &store, K);
        assert_eq!(
            got, truth,
            "selective mask must be served exactly (pre-filter)"
        );
    }

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[test]
fn edge_cases_empty_full_single() {
    const N: usize = 2_000;
    const DIM: usize = 16;
    const K: usize = 10;

    let store_path = tmp("filtered-edge.spqv");
    let graph_path = tmp("filtered-edge.spqg");
    let (store, ids) = build_store(N, DIM, &store_path);
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();
    #[cfg(feature = "approx-ann")]
    let vidx = VectorIndex::build(&store);

    let mut q_state = 0xBEEF_u64;
    let q = rand_vec(&mut q_state, DIM);

    // (a) EMPTY mask → no results, from every filtered path.
    let empty = IdMask::new();
    assert!(empty.is_empty());
    assert!(idx.nearest_filtered(&q, &empty, &store, K).is_empty());
    #[cfg(feature = "approx-ann")]
    assert!(vidx.nearest_filtered(&q, &empty, &store, K).is_empty());
    assert!(nearest_exact_filtered(&store, &q, &empty, K).is_empty());

    // (b) FULL mask (every id) → equals the UNFILTERED exact search (filter is a no-op).
    let full: IdMask = ids.iter().copied().collect();
    assert!(
        !FilterConfig::default().prefer_prefilter(full.len(), N),
        "full mask is broad → traversal"
    );
    let unfiltered: Vec<u32> = nearest_exact(&store, &q, K)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let truth_full: Vec<u32> = nearest_exact_filtered(&store, &q, &full, K)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        truth_full, unfiltered,
        "a full mask must not change the exact result"
    );
    // Traversal over a full mask: every returned id is valid and it recovers the top hit.
    let got_full = idx.nearest_filtered(&q, &full, &store, K);
    assert_eq!(got_full.len(), K);
    assert!(got_full.iter().all(|(id, _)| full.contains(*id)));
    assert_eq!(
        got_full[0].0, unfiltered[0],
        "full-mask traversal must find the nearest"
    );

    // (c) SINGLE-element mask → exactly that id (it is in the store), score = cosine to it.
    let only = ids[123];
    let single = IdMask::from_ids([only]);
    assert_eq!(single.len(), 1);
    let got_single = idx.nearest_filtered(&q, &single, &store, K);
    assert_eq!(got_single.len(), 1);
    assert_eq!(got_single[0].0, only);
    // ...and the score matches a direct cosine.
    let direct = sparq_vectors::cosine(&q, store.get(only).unwrap());
    assert!(
        (got_single[0].1 - direct).abs() < 1e-4,
        "score {} != cosine {direct}",
        got_single[0].1
    );

    // (d) a mask id ABSENT from the store is silently skipped.
    let absent = IdMask::from_ids([only, 999_999]);
    let got_absent = nearest_exact_filtered(&store, &q, &absent, K);
    assert_eq!(got_absent.len(), 1);
    assert_eq!(got_absent[0].0, only);

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

#[cfg(feature = "approx-ann")]
#[test]
fn hnsw_filtered_agrees_with_ground_truth() {
    // The in-RAM HNSW filtered entry point uses the exact pre-filter (instant-distance adjacency is
    // not exposed), so it must match the ground truth exactly on any mask.
    const N: usize = 5_000;
    const DIM: usize = 16;
    const K: usize = 10;

    let store_path = tmp("filtered-hnsw.spqv");
    let (store, ids) = build_store(N, DIM, &store_path);
    let vidx = VectorIndex::build(&store);

    let mask: IdMask = ids.iter().copied().step_by(3).collect();
    let mut q_state = 0xFEED_u64;
    for _ in 0..30 {
        let q = rand_vec(&mut q_state, DIM);
        let truth = nearest_exact_filtered(&store, &q, &mask, K);
        let got = vidx.nearest_filtered(&q, &mask, &store, K);
        assert_eq!(
            got, truth,
            "HNSW filtered (pre-filter) must equal the ground truth"
        );
    }

    let _ = std::fs::remove_file(&store_path);
}

#[test]
fn idmask_and_config_mechanics() {
    // IdMask construction paths and FilterConfig crossover arithmetic.
    let mut m = IdMask::new();
    assert!(m.is_empty());
    m.insert(5).insert(7).insert(5); // dup collapses
    assert_eq!(m.len(), 2);
    assert!(m.contains(5) && m.contains(7) && !m.contains(6));

    let from_iter: IdMask = [1u32, 2, 3, 3].into_iter().collect();
    assert_eq!(from_iter.len(), 3);
    let mut got: Vec<u32> = from_iter.iter().collect();
    got.sort_unstable();
    assert_eq!(got, vec![1, 2, 3]);

    let cfg = FilterConfig::default();
    // Over a 1000-vector store: 1% = 10, but the 256 floor dominates → pre-filter up to 256.
    assert!(cfg.prefer_prefilter(256, 1000));
    assert!(!cfg.prefer_prefilter(257, 1000));
    // Over a 1_000_000-vector store: 1% = 10_000 dominates the floor.
    assert!(cfg.prefer_prefilter(10_000, 1_000_000));
    assert!(!cfg.prefer_prefilter(10_001, 1_000_000));
    // fraction 0 + floor 0 → only an empty mask "pre-filters" (degenerate); 1.0 → always.
    let never = FilterConfig {
        prefilter_fraction: 0.0,
        prefilter_floor: 0,
        traversal_beam_factor: 4,
    };
    assert!(!never.prefer_prefilter(1, 100));
    let always = FilterConfig {
        prefilter_fraction: 1.0,
        prefilter_floor: 0,
        traversal_beam_factor: 4,
    };
    assert!(always.prefer_prefilter(100, 100));
}
