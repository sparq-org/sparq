//! The T24d experiment: CPU vs GPU for sparq's hot-path shapes, with the
//! host→device transfer tax charged honestly.
//!
//! For each kernel × column size (1M / 10M / 100M elements) it measures, BEST-OF-N
//! and INTERLEAVED (each round runs every variant once, so thermal drift and cache
//! state hit all variants equally):
//!
//!   cpu1         — single-thread scalar Rust (one sparq scan thread today)
//!   cpuN         — rayon all-cores
//!   gpu resident — kernel dispatch + tiny readback; column(s) already in VRAM
//!                  (the amortised case a resident column cache would buy)
//!   gpu e2e      — re-upload ALL inputs + dispatch + readback (the per-query
//!                  streaming case; on this M1 "upload" is a unified-memory
//!                  memcpy — a discrete PCIe card is strictly worse)
//!
//! Checksums are asserted equal across all variants every round.
//!
//! Run: `cargo run -p sparq-gpu --release --example gpu_bench`

use rayon::prelude::*;
use sparq_gpu::{cpu, Gpu};
use std::time::Instant;

/// Deterministic xorshift64* (same stream family as the tests / original spike).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn u32(&mut self) -> u32 {
        (self.next() >> 32) as u32
    }
}

const PAR_CHUNK: usize = 64 * 1024;
/// A kernel call only fails if the device stalls past `sparq_gpu::POLL_TIMEOUT`;
/// the panic then unwinds safely (a stalled `Gpu` leaks rather than waits on drop).
const GPU_OK: &str = "GPU kernel stalled past POLL_TIMEOUT";

/// A named benchmark leg returning a checksum (asserted equal across legs).
type Variant<'a> = (&'a str, Box<dyn FnMut() -> u64 + 'a>);

/// Interleaved best-of-N: round-robin the variants, keep per-variant minima,
/// assert every variant agrees on the checksum every round.
fn interleaved(iters: usize, variants: &mut [Variant]) -> Vec<f64> {
    let mut best = vec![f64::MAX; variants.len()];
    for _ in 0..iters {
        let mut checks = Vec::with_capacity(variants.len());
        for (vi, (_, f)) in variants.iter_mut().enumerate() {
            let t = Instant::now();
            checks.push(f());
            best[vi] = best[vi].min(t.elapsed().as_secs_f64() * 1e3);
        }
        for (vi, &c) in checks.iter().enumerate() {
            assert_eq!(
                c, checks[0],
                "checksum mismatch: {} vs {}",
                variants[vi].0, variants[0].0
            );
        }
    }
    best
}

fn header(title: &str) {
    println!("\n### {title}\n");
    println!(
        "| elems | cpu1 ms | cpuN ms | gpu resident ms | gpu e2e ms | cpuN/resident | cpuN/e2e |"
    );
    println!("|--:|--:|--:|--:|--:|--:|--:|");
}

fn row(n: usize, b: &[f64]) {
    println!(
        "| {}M | {:.2} | {:.2} | {:.2} | {:.2} | {:.2}x | {:.2}x |",
        n / 1_000_000,
        b[0],
        b[1],
        b[2],
        b[3],
        b[1] / b[2],
        b[1] / b[3],
    );
}

fn iters_for(n: usize) -> usize {
    if n >= 100_000_000 {
        3
    } else {
        5
    }
}

fn main() {
    let Some(gpu) = Gpu::new() else {
        eprintln!("no GPU adapter available — nothing to measure");
        return;
    };
    println!(
        "adapter: {} | backend: {:?} | type: {:?} | max storage binding: {} MiB | host cores: {}",
        gpu.info.name,
        gpu.info.backend,
        gpu.info.device_type,
        gpu.max_storage_bytes / (1024 * 1024),
        rayon::current_num_threads(),
    );
    println!("(ratios > 1.0x mean the GPU wins; 'e2e' charges the full input re-upload each call)");

    let sizes = [1_000_000usize, 10_000_000, 100_000_000];

    // ---------------- kernel 1a: FILTER count over a u32 column ----------------
    header("FILTER u32 (lo <= v < hi, ~12.5% selectivity) — compute-light");
    for &n in &sizes {
        if (n * 4) as u64 > gpu.max_storage_bytes {
            println!(
                "| {}M | skipped — column exceeds max storage binding |",
                n / 1_000_000
            );
            continue;
        }
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let col: Vec<u32> = (0..n).map(|_| rng.u32()).collect();
        let (lo, hi) = (0u32, u32::MAX / 8);
        let resident = gpu.upload_u32(&col);
        let _ = gpu.filter_count_u32(&resident, lo, hi).expect(GPU_OK); // warm pipeline/caches

        let mut variants: Vec<Variant> = vec![
            ("cpu1", Box::new(|| cpu::filter_count_u32(&col, lo, hi))),
            (
                "cpuN",
                Box::new(|| {
                    col.par_chunks(PAR_CHUNK)
                        .map(|c| cpu::filter_count_u32(c, lo, hi))
                        .sum()
                }),
            ),
            (
                "gpu resident",
                Box::new(|| gpu.filter_count_u32(&resident, lo, hi).expect(GPU_OK)),
            ),
            (
                "gpu e2e",
                Box::new(|| {
                    gpu.write_u32(&resident, &col);
                    gpu.filter_count_u32(&resident, lo, hi).expect(GPU_OK)
                }),
            ),
        ];
        row(n, &interleaved(iters_for(n), &mut variants));
    }

    // ------------- kernel 1b: FILTER count over an f64 column (numeric cache) -------------
    header("FILTER f64 (v > t, ~12.5% selectivity; exact bit-order compare — WGSL has no f64) — compute-light");
    for &n in &sizes {
        if (n * 8) as u64 > gpu.max_storage_bytes {
            println!(
                "| {}M | skipped — column exceeds max storage binding |",
                n / 1_000_000
            );
            continue;
        }
        let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
        let col: Vec<f64> = (0..n).map(|_| rng.u32() as f64 / 1e3).collect();
        let t = u32::MAX as f64 / 1e3 * 0.875;
        let resident = gpu.upload_f64(&col);
        let _ = gpu.filter_count_f64_gt(&resident, t).expect(GPU_OK);

        let mut variants: Vec<Variant> = vec![
            ("cpu1", Box::new(|| cpu::filter_count_f64_gt(&col, t))),
            (
                "cpuN",
                Box::new(|| {
                    col.par_chunks(PAR_CHUNK)
                        .map(|c| cpu::filter_count_f64_gt(c, t))
                        .sum()
                }),
            ),
            (
                "gpu resident",
                Box::new(|| gpu.filter_count_f64_gt(&resident, t).expect(GPU_OK)),
            ),
            (
                "gpu e2e",
                Box::new(|| {
                    gpu.write_f64(&resident, &col);
                    gpu.filter_count_f64_gt(&resident, t).expect(GPU_OK)
                }),
            ),
        ];
        row(n, &interleaved(iters_for(n), &mut variants));
    }

    // ---------------- kernel 2: hash-join probe ----------------
    header("HASH-JOIN probe (build = n/4 ids, table load 0.5, ~50% hit rate) — compute-medium");
    for &n in &sizes {
        let n_build = n / 4;
        let cap = (n_build * 2).next_power_of_two();
        let table_bytes = (cap * 8) as u64;
        if (n * 4) as u64 > gpu.max_storage_bytes || table_bytes > gpu.max_storage_bytes {
            println!(
                "| {}M | skipped — inputs exceed max storage binding |",
                n / 1_000_000
            );
            continue;
        }
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        let build_keys: Vec<u32> = (0..n_build)
            .map(|_| rng.u32() % (2 * n_build as u32))
            .collect();
        let payloads: Vec<u32> = (0..n_build).map(|_| rng.u32() & 0xFFFF).collect();
        let probe: Vec<u32> = (0..n).map(|_| rng.u32() % (4 * n_build as u32)).collect();
        let slots = cpu::build_hash_table(&build_keys, &payloads);
        let table = gpu.upload_table(&slots);
        let probe_col = gpu.upload_u32(&probe);
        let _ = gpu.hash_probe(&table, &probe_col).expect(GPU_OK);

        let mut variants: Vec<Variant> = vec![
            (
                "cpu1",
                Box::new(|| {
                    let (m, s) = cpu::hash_probe(&slots, &probe);
                    m.wrapping_add(s)
                }),
            ),
            (
                "cpuN",
                Box::new(|| {
                    let (m, s) = probe
                        .par_chunks(PAR_CHUNK)
                        .map(|c| cpu::hash_probe(&slots, c))
                        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
                    m.wrapping_add(s)
                }),
            ),
            (
                "gpu resident",
                Box::new(|| {
                    let (m, s) = gpu.hash_probe(&table, &probe_col).expect(GPU_OK);
                    m.wrapping_add(s)
                }),
            ),
            (
                "gpu e2e",
                Box::new(|| {
                    gpu.write_table(&table, &slots);
                    gpu.write_u32(&probe_col, &probe);
                    let (m, s) = gpu.hash_probe(&table, &probe_col).expect(GPU_OK);
                    m.wrapping_add(s)
                }),
            ),
        ];
        row(n, &interleaved(iters_for(n), &mut variants));
    }

    // ---------------- kernel 3: COUNT+SUM GROUP BY ----------------
    header("GROUP BY COUNT+SUM (256 groups, u64 sums via atomic carry) — compute-dense for a scan");
    for &n in &sizes {
        if (n * 4) as u64 > gpu.max_storage_bytes {
            println!(
                "| {}M | skipped — columns exceed max storage binding |",
                n / 1_000_000
            );
            continue;
        }
        const G: u32 = 256;
        const KEYS_IN_RANGE: &str = "keys are generated `% G`";
        let mut rng = Rng(0x0F0F_F0F0_1337_4242);
        let keys: Vec<u32> = (0..n).map(|_| rng.u32() % G).collect();
        let vals: Vec<u32> = (0..n).map(|_| rng.u32()).collect();
        let keys_col = gpu.upload_u32(&keys);
        let vals_col = gpu.upload_u32(&vals);
        let _ = gpu
            .group_aggregate(&keys_col, &vals_col, G)
            .expect(KEYS_IN_RANGE);

        let fold = |rows: Vec<(u64, u64)>| -> u64 {
            rows.iter().fold(0u64, |acc, (c, s)| {
                acc.wrapping_mul(31).wrapping_add(c.wrapping_add(*s))
            })
        };
        let merge = |mut a: Vec<(u64, u64)>, b: Vec<(u64, u64)>| {
            for (x, y) in a.iter_mut().zip(b) {
                x.0 += y.0;
                x.1 += y.1;
            }
            a
        };

        let mut variants: Vec<Variant> = vec![
            (
                "cpu1",
                Box::new(|| fold(cpu::group_aggregate(&keys, &vals, G).expect(KEYS_IN_RANGE))),
            ),
            (
                "cpuN",
                Box::new(|| {
                    let rows = keys
                        .par_chunks(PAR_CHUNK)
                        .zip(vals.par_chunks(PAR_CHUNK))
                        .map(|(k, v)| cpu::group_aggregate(k, v, G).expect(KEYS_IN_RANGE))
                        .reduce(|| vec![(0, 0); G as usize], merge);
                    fold(rows)
                }),
            ),
            (
                "gpu resident",
                Box::new(|| {
                    fold(
                        gpu.group_aggregate(&keys_col, &vals_col, G)
                            .expect(KEYS_IN_RANGE),
                    )
                }),
            ),
            (
                "gpu e2e",
                Box::new(|| {
                    gpu.write_u32(&keys_col, &keys);
                    gpu.write_u32(&vals_col, &vals);
                    fold(
                        gpu.group_aggregate(&keys_col, &vals_col, G)
                            .expect(KEYS_IN_RANGE),
                    )
                }),
            ),
        ];
        row(n, &interleaved(iters_for(n), &mut variants));
    }

    println!("\ndone — all variants checksum-verified every round.");
}
