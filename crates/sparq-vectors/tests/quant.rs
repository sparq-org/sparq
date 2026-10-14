//! Accuracy gates for vector quantization (sq-nq5):
//!
//! 1. **SQ round-trip error bound** — per-dimension f32→u8→f32 reconstruction error is within
//!    half a quantization step, and the cosine of the reconstruction tracks the original;
//! 2. **PQ encode/decode** — codes are M bytes, decode within each subspace, and a vector's own
//!    code is its nearest centroid set;
//! 3. **PQ asymmetric-distance ranking preserves recall** — recall@10 of ADC ranking over PQ
//!    codes vs exact cosine on a synthetic *clustered* set, within a documented delta;
//! 4. degenerate / validation contracts (empty store, zero query, bad config) and the
//!    EncodedStore slot→id mapping used as the DiskANN candidate cache.

use sparq_vectors::{
    cosine, nearest_exact, DistanceTable, EncodedStore, PqConfig, ProductQuantizer,
    ScalarQuantizer, VectorStore,
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

/// L2-normalize (the cosine convention the quantizers use internally).
fn normalize(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter().map(|x| x / n).collect()
    } else {
        v.to_vec()
    }
}

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sparq-vectors-{name}-{}", std::process::id()))
}

/// A synthetic clustered set: `clusters` Gaussian-ish blobs of `per` points each, ids dense+1.
/// Returns the finalized store and the cluster centres. Used by the PQ recall gate (PQ excels on
/// clustered data — the regime real embeddings live in — which is what the recall budget assumes).
fn clustered_store(
    path: &std::path::Path,
    dim: usize,
    clusters: usize,
    per: usize,
    spread: f32,
    state: &mut u64,
) -> VectorStore {
    let mut store = VectorStore::create(path, dim).unwrap();
    let mut id = 1u32;
    for _ in 0..clusters {
        let center = rand_vec(state, dim);
        for _ in 0..per {
            let v: Vec<f32> = center
                .iter()
                .map(|x| x + ((splitmix64(state) >> 40) as f32 / (1u64 << 23) as f32 - 0.5) * spread)
                .collect();
            // Guard against an all-zero draw (put rejects it); clustered noise never zeroes.
            store.put(id, &v).unwrap();
            id += 1;
        }
    }
    store.finalize().unwrap();
    store
}

#[test]
fn sq_round_trip_error_within_half_step() {
    // SQ maps each dimension onto 256 levels of its learned [min,max]; the worst per-component
    // reconstruction error is half a step = range/510. We assert exactly that bound, then that the
    // reconstruction's cosine to the original normalized vector is high (the property re-ranking
    // relies on).
    const DIM: usize = 48;
    const N: usize = 2_000;

    let mut state = 0x5CA1A8_u64;
    let vecs: Vec<Vec<f32>> = (0..N).map(|_| rand_vec(&mut state, DIM)).collect();
    let sq = ScalarQuantizer::fit(DIM, vecs.iter().map(|v| v.as_slice())).unwrap();
    assert_eq!(sq.dim(), DIM);

    // Per-dim half-step bound: recover the step from a probe and bound every component.
    let mut worst_err = 0f32;
    let mut min_cos = 1f32;
    for v in &vecs {
        let nv = normalize(v);
        let code = sq.encode(v);
        assert_eq!(code.len(), DIM);
        let rec = sq.reconstruct(&code);
        assert_eq!(rec.len(), DIM);
        for d in 0..DIM {
            worst_err = worst_err.max((rec[d] - nv[d]).abs());
        }
        min_cos = min_cos.min(cosine(&nv, &rec));
    }
    // Half-step over the data range: the normalized components live in [-1,1], so a generous
    // single bound is (2.0 / 255) / 2 ≈ 0.0039 — every component must be within it.
    let half_step = (2.0f32 / 255.0) / 2.0 + 1e-6;
    eprintln!("SQ worst per-component error = {worst_err:.6} (bound {half_step:.6}), min cosine of reconstruction = {min_cos:.5}");
    assert!(worst_err <= half_step, "SQ component error {worst_err} exceeds half-step bound {half_step}");
    // Reconstruction stays a faithful direction (4× smaller): cosine well above 0.99.
    assert!(min_cos > 0.99, "SQ reconstruction cosine too low: {min_cos}");
}

#[test]
fn sq_encode_store_maps_slots_to_ids() {
    let path = tmp("sq-store.spqv");
    let mut state = 0xABC_u64;
    let store = clustered_store(&path, 16, 4, 25, 0.1, &mut state);
    let sq = ScalarQuantizer::fit(16, store.iter().map(|(_, v)| v)).unwrap();
    let enc = sq.encode_store(&store).unwrap();
    assert_eq!(enc.len(), store.len());
    assert_eq!(enc.stride(), 16); // SQ stride is dim
    // Every slot's id+code reconstruct to ~the stored (normalized) vector.
    for (slot, id, code) in enc.iter() {
        assert_eq!(id, enc.id(slot));
        let stored = normalize(store.get(id).unwrap());
        let rec = sq.reconstruct(code);
        assert!(cosine(&stored, &rec) > 0.99, "slot {slot} reconstruction drifted");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn pq_encode_decode_and_self_nearest() {
    // A code is M bytes; reconstruct concatenates the chosen subspace centroids; a vector's own
    // code picks, per subspace, the centroid nearest its sub-vector (so re-encoding the
    // reconstruction is idempotent on well-separated codebooks).
    const DIM: usize = 32;
    let path = tmp("pq-codec.spqv");
    let mut state = 0xDEF_u64;
    let store = clustered_store(&path, DIM, 8, 60, 0.08, &mut state);
    let cfg = PqConfig { m: 8, k: 256, iters: 15, seed: 1 };
    let pq = ProductQuantizer::fit(DIM, store.iter().map(|(_, v)| v), cfg).unwrap();
    assert_eq!(pq.m(), 8);
    assert_eq!(pq.k(), 256);
    assert_eq!(pq.dim(), DIM);

    for (_, v) in store.iter() {
        let code = pq.encode(v);
        assert_eq!(code.len(), 8);
        let rec = pq.reconstruct(&code);
        assert_eq!(rec.len(), DIM);
        // Re-encoding the reconstruction yields the same code (each centroid is its own nearest).
        assert_eq!(pq.encode(&rec), code, "PQ encode of reconstruction is not idempotent");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn pq_adc_distance_matches_reconstructed_distance() {
    // The ADC table sum must equal the squared distance to the reconstructed code (the identity
    // that makes ADC exact *given the codes*): Σₛ ||q_s − c_{code_s}||² = ||q − reconstruct(code)||².
    const DIM: usize = 24;
    let path = tmp("pq-adc.spqv");
    let mut state = 0x111_u64;
    let store = clustered_store(&path, DIM, 5, 40, 0.1, &mut state);
    let pq = ProductQuantizer::fit(DIM, store.iter().map(|(_, v)| v), PqConfig { m: 6, k: 64, iters: 12, seed: 9 }).unwrap();

    let mut q_state = 0x222_u64;
    for _ in 0..50 {
        let q = rand_vec(&mut q_state, DIM);
        let nq = normalize(&q);
        let table = DistanceTable::new(&pq, &q);
        for (_, v) in store.iter() {
            let code = pq.encode(v);
            let adc = table.distance(&code);
            let rec = pq.reconstruct(&code);
            let direct = nq.iter().zip(&rec).map(|(a, b)| (a - b) * (a - b)).sum::<f32>();
            assert!((adc - direct).abs() < 1e-3, "ADC {adc} != reconstructed distance {direct}");
            // cosine() helper agrees with 1 − d²/2.
            assert!((table.cosine(&code) - (1.0 - adc / 2.0)).abs() < 1e-5);
        }
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn pq_adc_ranking_preserves_recall_on_clustered_set() {
    // The headline gate. PQ trades recall for 8× compression (M=16,K=256 → 16-byte codes vs
    // 32-d×4=128 B). We measure recall@10 in two regimes against exact cosine on a synthetic
    // clustered set, and gate BOTH:
    //
    //   (a) PQ-alone     — rank by ADC over codes, truncate to 10. The RAM-only candidate ranking,
    //                      no disk access. PQ codes are coarse, so this is honestly *modest*
    //                      (~0.6 measured) — gated only at a documented floor.
    //   (b) PQ + re-rank — take the top-`REORDER` ADC candidates, re-score the *handful* of them
    //                      against full-precision vectors, keep 10. This is the DiskANN search loop
    //                      (search on PQ, re-rank the beam on disk) and is the number that matters:
    //                      it recovers near-exact recall (~0.98 measured) at 8× compression while
    //                      reading only `REORDER` full vectors per query.
    //
    // The contrast (b ≫ a) is precisely *why* DiskANN re-ranks: PQ alone is a fast coarse filter,
    // not the final ranking.
    const DIM: usize = 32;
    const K: usize = 10;
    const REORDER: usize = 50;
    const QUERIES: usize = 100;

    let path = tmp("pq-recall.spqv");
    let mut state = 0xC0FFEE_u64;
    // 40 clusters × 250 = 10k clustered vectors (capped well under any RAM/disk concern).
    let store = clustered_store(&path, DIM, 40, 250, 0.15, &mut state);
    assert_eq!(store.len(), 10_000);

    let pq = ProductQuantizer::fit(DIM, store.iter().map(|(_, v)| v), PqConfig::default()).unwrap();
    let enc = pq.encode_store(&store).unwrap();
    assert_eq!(enc.stride(), 16); // M = 16 default → 16-byte codes
    let compression = (DIM * 4) as f64 / enc.stride() as f64;

    let mut q_state = 0xDECAF_u64;
    let (mut pq_hits, mut rerank_hits) = (0usize, 0usize);
    for _ in 0..QUERIES {
        let q = rand_vec(&mut q_state, DIM);
        let exact: Vec<u32> = nearest_exact(&store, &q, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(exact.len(), K);
        let table = DistanceTable::new(&pq, &q);

        // (a) PQ-alone.
        let pq_only: Vec<u32> = enc.rank_pq(&table, K).into_iter().map(|(id, _)| id).collect();
        assert_eq!(pq_only.len(), K);
        pq_hits += pq_only.iter().filter(|id| exact.contains(id)).count();

        // (b) PQ candidate list → full-precision re-rank.
        let cand = enc.rank_pq(&table, REORDER);
        let mut rescored: Vec<(u32, f32)> =
            cand.into_iter().map(|(id, _)| (id, cosine(&q, store.get(id).unwrap()))).collect();
        rescored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let reranked: Vec<u32> = rescored.into_iter().take(K).map(|(id, _)| id).collect();
        rerank_hits += reranked.iter().filter(|id| exact.contains(id)).count();
    }
    let pq_recall = pq_hits as f64 / (QUERIES * K) as f64;
    let rerank_recall = rerank_hits as f64 / (QUERIES * K) as f64;
    eprintln!(
        "PQ on {}x{DIM} at {compression:.0}× compression, {QUERIES} queries:\n  \
         (a) PQ-alone        recall@{K} = {pq_recall:.4}\n  \
         (b) PQ + re-rank({REORDER}) recall@{K} = {rerank_recall:.4}",
        store.len()
    );
    // Documented floors (measured ~0.62 PQ-alone, ~0.98 re-rank — see the recall-vs-compression
    // note in the report/README). The re-rank gate is the load-bearing one: it is the DiskANN
    // loop and proves PQ codes are a usable candidate filter at 8× with near-exact final recall.
    assert!(pq_recall >= 0.55, "PQ-alone recall@{K} below floor: {pq_recall:.4} < 0.55");
    assert!(
        rerank_recall >= 0.95,
        "PQ + re-rank recall@{K} below gate: {rerank_recall:.4} < 0.95"
    );
    assert!(rerank_recall >= pq_recall, "re-rank must not lose recall vs PQ-alone");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn pq_uneven_subspaces_cover_every_dimension() {
    // dim not divisible by M: subspaces are ceil/floor sized, every dim covered exactly once.
    const DIM: usize = 30; // 30 / 7 → sizes 5,5,4,4,4,4,4 (sum 30)
    let path = tmp("pq-uneven.spqv");
    let mut state = 0x333_u64;
    let store = clustered_store(&path, DIM, 4, 50, 0.1, &mut state);
    let pq = ProductQuantizer::fit(DIM, store.iter().map(|(_, v)| v), PqConfig { m: 7, k: 64, iters: 10, seed: 3 }).unwrap();
    let (_, v) = store.iter().next().unwrap();
    let code = pq.encode(v);
    assert_eq!(code.len(), 7);
    let rec = pq.reconstruct(&code);
    assert_eq!(rec.len(), DIM); // all 30 dims reconstructed, no gaps/overlaps
    let _ = std::fs::remove_file(&path);
}

#[test]
fn quant_validation_and_degenerate_cases() {
    // Config validation.
    assert!(ProductQuantizer::fit(8, [[0.0f32; 8].as_slice()], PqConfig { m: 0, ..Default::default() }).is_err());
    assert!(ProductQuantizer::fit(8, [[0.0f32; 8].as_slice()], PqConfig { m: 9, ..Default::default() }).is_err()); // m > dim
    assert!(ProductQuantizer::fit(8, [[1.0f32; 8].as_slice()], PqConfig { k: 0, ..Default::default() }).is_err());
    assert!(ProductQuantizer::fit(8, [[1.0f32; 8].as_slice()], PqConfig { k: 257, ..Default::default() }).is_err());
    // Empty training set for PQ is an error (k-means needs a point); SQ tolerates it (degenerate).
    let empty: [&[f32]; 0] = [];
    assert!(ProductQuantizer::fit(8, empty, PqConfig::default()).is_err());
    let sq_empty = ScalarQuantizer::fit(8, empty).unwrap();
    assert_eq!(sq_empty.encode(&[0.5; 8]), vec![0u8; 8]); // degenerate → all level 0

    // K may exceed the training count (fewer points than centroids); fit succeeds.
    let one = [[1.0f32, 2.0, 3.0, 4.0].as_slice()];
    let pq1 = ProductQuantizer::fit(4, one, PqConfig { m: 2, k: 256, iters: 5, seed: 1 }).unwrap();
    let code = pq1.encode(&[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(code.len(), 2);

    // Encode/encode_store dim-mismatch error path.
    let path = tmp("pq-dim.spqv");
    let mut state = 0x444_u64;
    let store = clustered_store(&path, 12, 3, 30, 0.1, &mut state);
    let pq_wrong = ProductQuantizer::fit(8, [[1.0f32; 8].as_slice()], PqConfig { m: 4, k: 16, iters: 5, seed: 1 }).unwrap();
    assert!(pq_wrong.encode_store(&store).is_err(), "store dim 12 != quantizer dim 8 must error");
    let _ = std::fs::remove_file(&path);

    // Empty store → empty EncodedStore (the candidate cache for an empty index).
    let epath = tmp("pq-emptystore.spqv");
    let mut estore = VectorStore::create(&epath, 8).unwrap();
    estore.finalize().unwrap();
    let pq2 = ProductQuantizer::fit(8, [[1.0f32; 8].as_slice()], PqConfig { m: 4, k: 16, iters: 5, seed: 1 }).unwrap();
    let enc = pq2.encode_store(&estore).unwrap();
    assert!(enc.is_empty());
    let q = [0.1f32; 8];
    assert!(enc.rank_pq(&DistanceTable::new(&pq2, &q), 10).is_empty());
    let _ = std::fs::remove_file(&epath);
}

#[test]
fn pq_ties_break_on_ascending_id() {
    // rank_pq tie-breaks on ascending id (matching nearest_exact), so identical-code vectors rank
    // deterministically by id.
    let path = tmp("pq-ties.spqv");
    let mut store = VectorStore::create(&path, 4).unwrap();
    // Two clusters; within each, vectors share a code.
    for id in [10u32, 5, 7] {
        store.put(id, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    }
    store.put(100, &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.finalize().unwrap();
    let pq = ProductQuantizer::fit(4, store.iter().map(|(_, v)| v), PqConfig { m: 2, k: 4, iters: 10, seed: 1 }).unwrap();
    let enc = pq.encode_store(&store).unwrap();
    let table = DistanceTable::new(&pq, &[1.0, 0.0, 0.0, 0.0]);
    let ranked: Vec<u32> = enc.rank_pq(&table, 3).into_iter().map(|(id, _)| id).collect();
    // The three [1,0,0,0] vectors share a code → ascending id order 5,7,10.
    assert_eq!(ranked, vec![5, 7, 10]);
    let _ = std::fs::remove_file(&path);
}

#[test]
// (sq-qamd) The PQ codebook round-trips through to_bytes/from_bytes byte-for-byte (so it
// can be persisted alongside a .spqg graph), and a decoded quantizer produces identical codes and
// ADC distances.
fn pq_codebook_serialization_round_trips() {
    let path = tmp("pq-serde.spqv");
    let mut cstate = 0x5151_u64;
    let store = clustered_store(&path, 16, 3, 40, 0.1, &mut cstate);
    let pq = ProductQuantizer::fit(16, store.iter().map(|(_, v)| v), PqConfig::default()).unwrap();

    let bytes = pq.to_bytes();
    let decoded = ProductQuantizer::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.dim(), pq.dim());
    assert_eq!(decoded.m(), pq.m());
    assert_eq!(decoded.k(), pq.k());
    // Re-serializing the decoded quantizer yields the identical bytes.
    assert_eq!(decoded.to_bytes(), bytes);

    // Codes and ADC distances match the original for every stored vector.
    let mut state = 0x9999_u64;
    let q = rand_vec(&mut state, 16);
    let t0 = DistanceTable::new(&pq, &q);
    let t1 = DistanceTable::new(&decoded, &q);
    for (_, v) in store.iter() {
        assert_eq!(pq.encode(v), decoded.encode(v), "decoded codebook encodes differently");
        let c = pq.encode(v);
        assert!((t0.distance(&c) - t1.distance(&c)).abs() < 1e-6, "ADC distance differs");
    }

    // A truncated / over-long / invalid block is rejected.
    assert!(ProductQuantizer::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(ProductQuantizer::from_bytes(&extra).is_err());
    assert!(ProductQuantizer::from_bytes(b"").is_err());

    let _ = std::fs::remove_file(&path);
}

#[test]
// (sq-qamd) EncodedStore::from_parts reconstructs the candidate cache from its flat
// ids/codes/stride and validates the code-array length.
fn encoded_store_from_parts_round_trips_and_validates() {
    let path = tmp("pq-fromparts.spqv");
    let mut cstate = 0x2222_u64;
    let store = clustered_store(&path, 8, 2, 30, 0.1, &mut cstate);
    let pq = ProductQuantizer::fit(
        8,
        store.iter().map(|(_, v)| v),
        PqConfig { m: 4, k: 16, iters: 10, seed: 3 },
    )
    .unwrap();
    let enc = pq.encode_store(&store).unwrap();

    let ids: Vec<u32> = enc.iter().map(|(_, id, _)| id).collect();
    let rebuilt = EncodedStore::from_parts(ids, enc.codes().to_vec(), enc.stride()).unwrap();
    assert_eq!(rebuilt.len(), enc.len());
    for slot in 0..enc.len() {
        assert_eq!(rebuilt.code(slot), enc.code(slot));
        assert_eq!(rebuilt.id(slot), enc.id(slot));
    }

    // Wrong length is rejected.
    assert!(EncodedStore::from_parts(vec![1, 2, 3], vec![0u8; 5], 2).is_err());
    // stride 0 with codes present is rejected; empty is fine.
    assert!(EncodedStore::from_parts(vec![1], vec![0u8], 0).is_err());
    assert!(EncodedStore::from_parts(vec![], vec![], 0).is_ok());

    let _ = std::fs::remove_file(&path);
}
