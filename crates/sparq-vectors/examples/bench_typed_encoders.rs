//! sq-0wo9e.2 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md` §6.B)
//! P1 typed-literal encoder ABLATION harness — **encoder-level, honest, deterministic**.
//!
//! # Why this, and NOT a filtered link-prediction (FB15k-237 / WN18RR) MRR run
//!
//! The epic's P6 eval harness (filtered link-prediction / ablation matrix) is **bead sq-0wo9e.7,
//! still OPEN** — it did NOT land with P0 (sq-0wo9e.1 added only the closure + negative-sampling
//! primitives). Critically, `sparq-vectors` has **no KGE trainer** (embeddings are out-of-process;
//! see `crate::structure` docs), so there is no in-tree path that trains entity/relation
//! embeddings, consumes the typed-literal encoders, and reports filtered MRR / Hits@k on
//! FB15k-237 or WN18RR. Reporting such numbers from this crate today would mean **fabricating
//! them** — which the empirical-honesty mandate forbids. So this harness measures the property the
//! P1 encoders **actually deliver and that is testable now**: order-preserving numeric retrieval
//! quality, P1 ON vs OFF, **including a long-tail (rare-value) slice** — the design's "where typed
//! structure should help most".
//!
//! # The measured task (deterministic, self-contained)
//!
//! A synthetic numeric-literal-rich attribute: `N` entities each carry one numeric value drawn
//! from a **heavy-tailed** distribution (a dense body + a sparse long tail of rare large values).
//! "Retrieval" = for a query entity, rank all others by encoded distance and ask whether the
//! true value-nearest entities are returned. We report **Recall@k of the value-nearest neighbours**
//! and **Spearman rank-correlation** between encoded-distance order and true value-distance order:
//!
//! - **P1 ON**  — the order-preserving [`NumericEncoder`] thermometer block (L2).
//! - **P1 OFF** — the baseline the design ablates against: the literal value is **dropped** and the
//!   entity carries a hashed pseudo-embedding (what a plain relational KGE does with a literal).
//!
//! plus a **long-tail slice**: the same metrics restricted to query entities whose value lies in
//! the rare upper tail (the bottom-frequency decile of the value histogram).
//!
//! All numbers are **MEASURED ON THIS WORK BOX, NON-CANONICAL** (AGENTS.md *No hard-coded
//! performance numbers*): deterministic at a fixed `(N, seed)` because the PRNG, the encoder, and
//! the query set are all fixed, but they are an encoder-quality sanity check, **not** a published
//! KGE benchmark and **must not** be baked into docs/tests as canonical perf.
//!
//! ```sh
//! cargo run -p sparq-vectors --features structure --release --example bench_typed_encoders
//! cargo run -p sparq-vectors --features structure --release --example bench_typed_encoders -- [N] [seed] [k]
//! ```

use sparq_vectors::encode::NumericEncoder;

/// SplitMix64 — the same deterministic PRNG step the rest of the crate uses, so the corpus is
/// reproducible at a fixed seed across runs and platforms.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform `f64` in `[0, 1)` from the stream.
fn unit(state: &mut u64) -> f64 {
    (splitmix64(state) >> 11) as f64 / (1u64 << 53) as f64
}

/// L2 distance over `f32` slices.
fn l2(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

/// Build a heavy-tailed value corpus: a dense body in `[0, 100)` plus a sparse long tail that can
/// reach `~10^4` (a small fraction of entities). Returns the per-entity values.
fn corpus(n: usize, seed: u64) -> Vec<f64> {
    let mut st = seed ^ 0xA5A5_5A5A_DEAD_BEEF;
    (0..n)
        .map(|_| {
            let u = unit(&mut st);
            if u < 0.85 {
                // Dense body.
                unit(&mut st) * 100.0
            } else {
                // Long tail: rare large values, power-law-ish.
                let t = unit(&mut st);
                100.0 + t.powf(4.0) * 9900.0
            }
        })
        .collect()
}

/// The P1-OFF baseline embedding for an entity: a deterministic hashed pseudo-vector that carries
/// **no value information** (what a plain relational KGE does with a dropped literal). Same width
/// as the P1 block so the comparison is like-for-like on dimensionality.
fn baseline_vec(entity: usize, dim: usize) -> Vec<f32> {
    let mut st = (entity as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234_5678;
    (0..dim).map(|_| (unit(&mut st) * 2.0 - 1.0) as f32).collect()
}

/// Recall@k of the true value-nearest neighbours, averaged over the query set, plus the Spearman
/// rank-correlation between encoded-distance order and true value-distance order.
struct Quality {
    recall_at_k: f64,
    spearman: f64,
}

fn evaluate(
    values: &[f64],
    embed: impl Fn(usize) -> Vec<f32>,
    queries: &[usize],
    k: usize,
) -> Quality {
    let n = values.len();
    let codes: Vec<Vec<f32>> = (0..n).map(&embed).collect();

    let mut recall_sum = 0.0;
    let mut spearman_sum = 0.0;
    for &q in queries {
        // True value-distance order over all other entities.
        let mut by_value: Vec<usize> = (0..n).filter(|&i| i != q).collect();
        by_value.sort_by(|&a, &b| {
            (values[a] - values[q])
                .abs()
                .partial_cmp(&(values[b] - values[q]).abs())
                .unwrap()
        });
        // Encoded-distance order.
        let mut by_code: Vec<usize> = (0..n).filter(|&i| i != q).collect();
        by_code.sort_by(|&a, &b| {
            l2(&codes[a], &codes[q]).partial_cmp(&l2(&codes[b], &codes[q])).unwrap()
        });

        // Recall@k: fraction of the true top-k present in the encoded top-k.
        use std::collections::HashSet;
        let truth: HashSet<usize> = by_value.iter().take(k).copied().collect();
        let got: HashSet<usize> = by_code.iter().take(k).copied().collect();
        recall_sum += truth.intersection(&got).count() as f64 / k as f64;

        // Spearman over the FULL ranking of all `m = n-1` other entities: the rank of each entity
        // in the true value-distance order vs the encoded-distance order. Using the full set (not a
        // cap) keeps the `1 - 6·Σd² / (m·(m²−1))` formula exact and the result inside `[-1, 1]`.
        let m = by_value.len();
        let mut rank_by_value = vec![0usize; n];
        for (r, &e) in by_value.iter().enumerate() {
            rank_by_value[e] = r;
        }
        let mut rank_by_code = vec![0usize; n];
        for (r, &e) in by_code.iter().enumerate() {
            rank_by_code[e] = r;
        }
        let mut d2 = 0.0f64;
        for &e in &by_value {
            let d = rank_by_value[e] as f64 - rank_by_code[e] as f64;
            d2 += d * d;
        }
        let mf = m as f64;
        spearman_sum += 1.0 - 6.0 * d2 / (mf * (mf * mf - 1.0));
    }
    let q = queries.len() as f64;
    Quality { recall_at_k: recall_sum / q, spearman: spearman_sum / q }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let k: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let dim = 32usize;

    let values = corpus(n, seed);
    let enc = NumericEncoder::fit(values.clone(), dim);

    // Long-tail query slice: entities whose value is in the upper (rare) tail — the top-frequency
    // decile by value (the sparse end of the heavy-tailed histogram).
    let mut sorted = values.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let tail_threshold = sorted[(n as f64 * 0.9) as usize];
    let all_queries: Vec<usize> = (0..n).collect();
    let tail_queries: Vec<usize> = (0..n).filter(|&i| values[i] >= tail_threshold).collect();

    let on = evaluate(&values, |e| enc.encode(values[e]), &all_queries, k);
    let off = evaluate(&values, |e| baseline_vec(e, dim), &all_queries, k);
    let on_tail = evaluate(&values, |e| enc.encode(values[e]), &tail_queries, k);
    let off_tail = evaluate(&values, |e| baseline_vec(e, dim), &tail_queries, k);

    // NON-CANONICAL, work-box, encoder-quality sanity check — NOT a KGE benchmark.
    println!("# P1 typed-encoder ablation (NON-CANONICAL, work-box). N={} seed={} k={} dim={}", n, seed, k, dim);
    println!("# tail slice = top value-decile (>= {:.1}), {} of {} entities", tail_threshold, tail_queries.len(), n);
    println!("slice\tarm\trecall_at_k\tspearman");
    println!("overall\tP1_ON\t{:.4}\t{:.4}", on.recall_at_k, on.spearman);
    println!("overall\tP1_OFF\t{:.4}\t{:.4}", off.recall_at_k, off.spearman);
    println!("long_tail\tP1_ON\t{:.4}\t{:.4}", on_tail.recall_at_k, on_tail.spearman);
    println!("long_tail\tP1_OFF\t{:.4}\t{:.4}", off_tail.recall_at_k, off_tail.spearman);
}
