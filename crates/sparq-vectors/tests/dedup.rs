//! (#2251) The recall-gated concept-ANN dedup surface: `build_ann` / `knn` /
//! `dedup`. The load-bearing tests are the two halves of the gate contract — (1) on a
//! corpus where the exact O(m²) ground truth IS built, the HNSW index measures recall
//! `>= 0.99` and dedup proceeds; (2) when the measured recall is below the gate, `dedup`
//! returns `Err` and emits NO merge (fail-closed). Plus the planted-near-duplicate merge
//! semantics and the fail-closed input validation.

// The concept-ANN surface rides the opt-in HNSW backend (the only third-party ANN dep).
#![cfg(feature = "approx-ann")]

use sparq_vectors::{
    build_ann, dedup, exact_ground_truth, knn, DedupPolicy, GroundTruth, HnswConfig,
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

/// A deterministic random concept matrix with sparse, non-contiguous ids.
fn corpus(n: usize, dim: usize, seed: u64) -> Vec<(u32, Vec<f32>)> {
    let mut state = seed;
    (0..n)
        .map(|i| ((i as u32) * 7 + 3, rand_vec(&mut state, dim)))
        .collect()
}

/// The issue-#2251 acceptance test: build the exact O(m²) ground truth over the corpus,
/// build the ANN index, and assert `dedup` measures recall >= 0.99 against that oracle
/// (so the merges it then emits are gate-backed).
#[test]
fn ann_recall_gate_passes_vs_exact_ground_truth() {
    const N: usize = 3_000;
    const DIM: usize = 32;
    const K: usize = 10;

    let vectors = corpus(N, DIM, 0xC0FFEE);
    let truth = exact_ground_truth(&vectors, K).unwrap();
    assert_eq!(truth.k(), K);
    assert_eq!(truth.neighbors().len(), N);

    let index = build_ann(&vectors, HnswConfig::high_recall()).unwrap();
    assert_eq!(index.len(), N);
    assert_eq!(index.dim(), DIM);

    // Random uniform vectors: no planted near-duplicates, so a passing gate must emit
    // zero merges at a tight threshold — proving the gate and the merge phase are
    // independent (recall evidence never manufactures merges).
    let policy = DedupPolicy {
        recall_gate: 0.99,
        merge_threshold: 0.999,
        k: K,
    };
    let report = dedup(&index, &policy, &truth).unwrap();
    eprintln!("concept-ANN recall@{K} on {N}x{DIM}: {:.4}", report.recall);
    assert!(
        report.recall >= 0.99,
        "recall gate: {:.4} < 0.99",
        report.recall
    );
    assert!(
        report.merges.is_empty(),
        "no near-duplicates were planted: {:?}",
        report.merges
    );
    assert!(report.groups.is_empty());
}

/// Fail-closed half of the gate: a ground truth the index measurably under-recalls
/// (here: a deliberately WRONG oracle — each id's "exact" neighbours are far-away ids)
/// must make `dedup` return `Err` mentioning the gate, and emit no merges at all.
#[test]
fn dedup_refuses_merges_below_the_recall_gate() {
    const N: usize = 500;
    const DIM: usize = 16;
    const K: usize = 5;

    let vectors = corpus(N, DIM, 0xDECAF);
    let index = build_ann(&vectors, HnswConfig::default()).unwrap();

    // A wrong oracle: claim each id's true neighbours are the K ids "opposite" it in the
    // corpus order. Random 16-d vectors: the chance these are the actual top-5 is nil,
    // so measured recall lands far below 0.99.
    let ids: Vec<u32> = vectors.iter().map(|(id, _)| *id).collect();
    let wrong = GroundTruth::new(
        K,
        ids.iter()
            .enumerate()
            .map(|(i, &id)| {
                let far: Vec<u32> = (1..=K).map(|j| ids[(i + N / 2 + j) % N]).collect();
                (id, far)
            })
            .collect(),
    )
    .unwrap();

    let err = dedup(&index, &DedupPolicy::default(), &wrong).unwrap_err();
    assert!(
        err.contains("below the pre-registered gate"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains("refusing to apply merges"),
        "unexpected error: {err}"
    );
}

/// Merge semantics: planted near-duplicate clusters (and only they) merge, transitively,
/// with the smallest id as the canonical representative.
#[test]
fn dedup_merges_planted_near_duplicates_and_nothing_else() {
    const DIM: usize = 32;
    const K: usize = 5;
    let mut state = 7_u64;

    // 200 well-separated random concepts (random 32-d directions: pairwise cosine far
    // below any merge threshold)…
    let mut vectors: Vec<(u32, Vec<f32>)> = (0..200u32)
        .map(|i| (i * 10, rand_vec(&mut state, DIM)))
        .collect();
    // …plus two planted duplicate clusters: tiny perturbations of a base concept, the
    // cross-source near-duplicate shape (cosine > 0.9999).
    let plant = |base_of: u32, dups: &[u32], vectors: &mut Vec<(u32, Vec<f32>)>| {
        let base = vectors
            .iter()
            .find(|(id, _)| *id == base_of)
            .unwrap()
            .1
            .clone();
        for &dup_id in dups {
            let v: Vec<f32> = base.iter().map(|x| x + x * 1e-4).collect();
            vectors.push((dup_id, v));
        }
    };
    plant(50, &[2001, 2002], &mut vectors); // group {50, 2001, 2002} — canonical 50
    plant(1200, &[2003], &mut vectors); // group {1200, 2003}     — canonical 1200

    let truth = exact_ground_truth(&vectors, K).unwrap();
    let index = build_ann(&vectors, HnswConfig::high_recall()).unwrap();
    let policy = DedupPolicy {
        recall_gate: 0.99,
        merge_threshold: 0.999,
        k: K,
    };
    let report = dedup(&index, &policy, &truth).unwrap();

    assert_eq!(
        report.groups,
        vec![vec![50, 2001, 2002], vec![1200, 2003]],
        "exactly the planted clusters merge, smallest id first"
    );
    assert_eq!(report.merges, vec![(2001, 50), (2002, 50), (2003, 1200)]);
}

/// `knn` (free function == method) returns cosine-ranked neighbours; a zero query has no
/// direction and returns nothing — the same contract as the rest of the crate.
#[test]
fn knn_returns_cosine_ranked_neighbours_and_zero_query_is_empty() {
    const DIM: usize = 8;
    let mut state = 11_u64;
    // Three well-separated clusters of 20, ids partitioned by hundreds.
    let mut vectors: Vec<(u32, Vec<f32>)> = Vec::new();
    let mut centers = Vec::new();
    for c in 0..3u32 {
        let center = rand_vec(&mut state, DIM);
        for i in 0..20u32 {
            let v: Vec<f32> = center
                .iter()
                .map(|x| {
                    x + ((splitmix64(&mut state) >> 40) as f32 / (1u64 << 23) as f32 - 0.5) * 0.05
                })
                .collect();
            vectors.push((c * 100 + i + 1, v));
        }
        centers.push(center);
    }
    let index = build_ann(&vectors, HnswConfig::default()).unwrap();

    for (c, center) in centers.iter().enumerate() {
        let hits = knn(&index, center, 5);
        assert_eq!(hits, index.knn(center, 5), "free fn and method agree");
        assert_eq!(hits.len(), 5);
        for &(id, sim) in &hits {
            assert_eq!(
                (id - 1) / 100,
                c as u32,
                "neighbour {id} from the wrong cluster"
            );
            assert!(
                sim > 0.99,
                "cluster member should be near-identical, got {sim}"
            );
        }
        // Best-first ordering.
        for w in hits.windows(2) {
            assert!(w[0].1 >= w[1].1, "scores must be non-increasing: {hits:?}");
        }
    }

    assert!(
        knn(&index, &[0.0; DIM], 5).is_empty(),
        "zero query has no direction"
    );
}

/// Fail-closed input validation on `build_ann` / `exact_ground_truth` / `GroundTruth::new`
/// / `dedup` policy checks: every malformed input is an `Err`, never a degraded result.
#[test]
fn fail_closed_validation() {
    let ok = vec![(1u32, vec![1.0f32, 0.0]), (2, vec![0.0, 1.0])];
    let cfg = HnswConfig::default();

    // build_ann / exact_ground_truth input validation.
    assert!(build_ann(&[], cfg).unwrap_err().contains("empty input"));
    let dup = vec![(1u32, vec![1.0f32, 0.0]), (1, vec![0.0, 1.0])];
    assert!(build_ann(&dup, cfg).unwrap_err().contains("duplicate id"));
    let mixed = vec![(1u32, vec![1.0f32, 0.0]), (2, vec![1.0])];
    assert!(build_ann(&mixed, cfg).unwrap_err().contains("dim"));
    let zero = vec![(1u32, vec![0.0f32, 0.0])];
    assert!(build_ann(&zero, cfg).unwrap_err().contains("all-zero"));
    let nan = vec![(1u32, vec![f32::NAN, 1.0])];
    assert!(build_ann(&nan, cfg).unwrap_err().contains("non-finite"));
    assert!(exact_ground_truth(&dup, 1)
        .unwrap_err()
        .contains("duplicate id"));
    assert!(exact_ground_truth(&ok, 0)
        .unwrap_err()
        .contains("k must be"));

    // GroundTruth::new oracle validation.
    assert!(GroundTruth::new(0, vec![(1, vec![])])
        .unwrap_err()
        .contains("k must be"));
    assert!(GroundTruth::new(1, vec![]).unwrap_err().contains("empty"));
    assert!(GroundTruth::new(1, vec![(1, vec![1])])
        .unwrap_err()
        .contains("itself"));
    assert!(GroundTruth::new(1, vec![(1, vec![2, 3])])
        .unwrap_err()
        .contains("> k"));
    assert!(GroundTruth::new(1, vec![(1, vec![2]), (1, vec![2])])
        .unwrap_err()
        .contains("duplicate query id"));

    // dedup misconfiguration checks (all pre-gate, all Err).
    let index = build_ann(&ok, cfg).unwrap();
    let truth = exact_ground_truth(&ok, 1).unwrap();
    let bad_gate = DedupPolicy {
        recall_gate: 1.5,
        ..DedupPolicy::default()
    };
    assert!(dedup(&index, &bad_gate, &truth)
        .unwrap_err()
        .contains("recall_gate"));
    let bad_thresh = DedupPolicy {
        merge_threshold: f32::NAN,
        ..DedupPolicy::default()
    };
    assert!(dedup(&index, &bad_thresh, &truth)
        .unwrap_err()
        .contains("merge_threshold"));
    let bad_k = DedupPolicy {
        k: 0,
        ..DedupPolicy::default()
    };
    assert!(dedup(&index, &bad_k, &truth)
        .unwrap_err()
        .contains("policy.k"));
    // k beyond the index's ef_search beam: rejected, not silently truncated.
    let tiny_beam = build_ann(
        &ok,
        HnswConfig {
            ef_search: 3,
            ..cfg
        },
    )
    .unwrap();
    let wide_k = DedupPolicy {
        k: 10,
        ..DedupPolicy::default()
    };
    assert!(dedup(&tiny_beam, &wide_k, &truth)
        .unwrap_err()
        .contains("ef_search"));
    // A ground-truth query id the index does not hold: oracle/index mismatch is an Err.
    let foreign = GroundTruth::new(1, vec![(99, vec![1])]).unwrap();
    assert!(dedup(&index, &DedupPolicy::default(), &foreign)
        .unwrap_err()
        .contains("not in the index"));
}
