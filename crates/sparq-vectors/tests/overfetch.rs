//! (sq-ip3a, follow-up to sq-7hx6) **Approximate filtered backend + iterative
//! over-fetch** gate. Compiled only with `filtered-ann` (the over-fetch path) AND `approx-ann`
//! (the approximate backend it exercises):
//!
//! ```text
//! cargo test -p sparq-vectors --features filtered-ann,approx-ann --test overfetch
//! ```
//!
//! Covers, on a known synthetic set:
//!  1. **recall** of the APPROXIMATE backend ([`ApproxBackend`] over the on-disk Vamana graph)
//!     with iterative over-fetch under a selective filter, measured against the exact-brute-force
//!     filtered ground truth ([`nearest_exact_filtered`]) — asserted `≥ 0.90@10` (a measured FLOOR,
//!     NOT exactness: an approximate index has recall `< 1.0`);
//!  2. **iterative over-fetch fills k** under a selective filter where a single sized fetch would
//!     under-fill (the recall boundary sq-7hx6 documented);
//!  3. the **exact** backend over the same path is answer-exact (recall `1.0`) — the contrast that
//!     keeps the approximate claim honest.

#![cfg(all(feature = "filtered-ann", feature = "approx-ann"))]

use sparq_vectors::{
    nearest_exact_filtered, nearest_filtered_overfetch_default, AnnBackend, ApproxBackend,
    DiskAnnIndex, ExactBackend, IdMask, VectorStore,
};

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
    std::env::temp_dir().join(format!("sparq-vectors-overfetch-{name}-{}", std::process::id()))
}

/// N×DIM store with sparse ids `i*7+3`; returns (store, all_ids).
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

/// (1) APPROXIMATE backend recall floor under a selective filter, WITH iterative over-fetch.
///
/// A selective mask (~10%) is the regime where post-filtering a bounded candidate list under-fills
/// without over-fetch. We measure recall of the over-fetch result (approximate backend) against the
/// exact-brute-force filtered ground truth and assert it clears a documented FLOOR — NOT exactness.
#[test]
fn approx_backend_overfetch_recall_floor_selective_mask() {
    const N: usize = 20_000;
    const DIM: usize = 32;
    const K: usize = 10;
    const QUERIES: usize = 100;
    const RECALL_FLOOR: f64 = 0.90; // measured floor for the approximate backend — recall < 1.0.

    let store_path = tmp("recall.spqv");
    let graph_path = tmp("recall.spqg");
    let (store, ids) = build_store(N, DIM, &store_path);
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();
    let approx = ApproxBackend::new(&idx);

    // ~10% selective mask: every 10th id. Selective enough that a bounded approximate fetch can
    // under-fill — exactly what iterative over-fetch exists to fix.
    let mask: IdMask = ids.iter().copied().step_by(10).collect();

    let mut q_state = 0xDECAF_u64;
    let mut hits = 0usize;
    let mut total = 0usize;
    let mut filled = 0usize;
    for _ in 0..QUERIES {
        let q = rand_vec(&mut q_state, DIM);
        let truth: Vec<u32> = nearest_exact_filtered(&store, &q, &mask, K)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let got = nearest_filtered_overfetch_default(&approx, &q, &mask, K);
        // HARD guarantee: every returned id is admitted by the mask (never an over-fetch leak).
        for &(id, _) in &got {
            assert!(mask.contains(id), "over-fetch returned an unmasked id {id}");
        }
        if got.len() == K {
            filled += 1;
        }
        let got_ids: Vec<u32> = got.into_iter().map(|(id, _)| id).collect();
        total += truth.len();
        hits += got_ids.iter().filter(|id| truth.contains(id)).count();
    }
    let recall = hits as f64 / total as f64;
    eprintln!(
        "approx-backend over-fetch recall@{K} over {QUERIES} queries (~10% mask) on {N}x{DIM}: \
         {recall:.4} (floor {RECALL_FLOOR}); filled-k {filled}/{QUERIES}"
    );
    // The mask admits ~2000 vectors ≫ K, so over-fetch should fill k on essentially every query.
    assert!(
        filled >= QUERIES * 95 / 100,
        "iterative over-fetch must fill k on ≥95% of queries, filled {filled}/{QUERIES}"
    );
    // APPROXIMATE: assert a FLOOR, not exactness.
    assert!(
        recall >= RECALL_FLOOR,
        "approximate over-fetch recall {recall:.4} < floor {RECALL_FLOOR}"
    );
    assert!(
        recall < 1.0 + 1e-9,
        "recall is a fraction; sanity guard on the measurement"
    );

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

/// (2) Iterative over-fetch FILLS k under a selective filter — contrasted with the EXACT backend
/// over the same mask, which is answer-exact. Confirms the approximate path returns k admitted
/// neighbours (filling), and that the exact path is the recall=1.0 reference.
#[test]
fn overfetch_fills_k_and_exact_is_reference() {
    const N: usize = 10_000;
    const DIM: usize = 16;
    const K: usize = 10;

    let store_path = tmp("fill.spqv");
    let graph_path = tmp("fill.spqg");
    let (store, ids) = build_store(N, DIM, &store_path);
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();
    let approx = ApproxBackend::new(&idx);
    let exact = ExactBackend::new(&store);

    // Selective ~5% mask.
    let mask: IdMask = ids.iter().copied().step_by(20).collect();
    assert!(mask.len() > K, "mask must admit ≥ k for a fill test");

    let mut q_state = 0xBEEF_u64;
    for _ in 0..50 {
        let q = rand_vec(&mut q_state, DIM);

        // Exact backend over the over-fetch path == the brute-force filtered ground truth (exact).
        let exact_got = nearest_filtered_overfetch_default(&exact, &q, &mask, K);
        let truth = nearest_exact_filtered(&store, &q, &mask, K);
        assert_eq!(exact_got, truth, "exact backend over-fetch must equal the ground truth");
        assert_eq!(exact_got.len(), K, "exact backend must fill k (mask admits ≫ k)");

        // Approximate backend fills k as well (over-fetch grows the fetch until k survive).
        let approx_got = nearest_filtered_overfetch_default(&approx, &q, &mask, K);
        assert_eq!(
            approx_got.len(),
            K,
            "iterative over-fetch must fill k from the approximate backend too"
        );
        for &(id, _) in &approx_got {
            assert!(mask.contains(id), "approx over-fetch leaked an unmasked id {id}");
        }
    }

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}

/// (3) Backend trait sanity: the approximate backend reports the store length and an empty backend
/// over an empty mask / zero query returns nothing (the shared degenerate contract).
#[test]
fn backend_len_and_degenerate() {
    const N: usize = 1_000;
    const DIM: usize = 8;
    let store_path = tmp("len.spqv");
    let graph_path = tmp("len.spqg");
    let (store, ids) = build_store(N, DIM, &store_path);
    let idx = DiskAnnIndex::build(&store, &graph_path).unwrap();
    let approx = ApproxBackend::new(&idx);
    assert_eq!(approx.len(), N);
    assert!(!approx.is_empty());

    let full: IdMask = ids.iter().copied().collect();
    let q = rand_vec(&mut 1u64, DIM);
    let zero = [0.0f32; DIM];
    assert!(nearest_filtered_overfetch_default(&approx, &q, &IdMask::new(), 5).is_empty());
    assert!(nearest_filtered_overfetch_default(&approx, &zero, &full, 5).is_empty());
    assert!(nearest_filtered_overfetch_default(&approx, &q, &full, 0).is_empty());

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&graph_path);
}
