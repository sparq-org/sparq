//! Scalar CPU reference implementations of the GPU kernels.
//!
//! These are the *correctness oracles* for the tests and the single-thread
//! baselines for `examples/gpu_bench.rs` (the rayon-parallel baselines live in
//! the example — the library carries no thread-pool dependency). They are
//! deliberately the same tight scalar loops a sparq scan thread runs today.

use crate::{hash32, GroupKeyOutOfRange, EMPTY_KEY};

/// Counts elements with `lo <= v < hi`.
pub fn filter_count_u32(col: &[u32], lo: u32, hi: u32) -> u64 {
    let mut c = 0u64;
    for &v in col {
        if v >= lo && v < hi {
            c += 1;
        }
    }
    c
}

/// Counts elements with `v > t` (IEEE: NaN compares false on either side).
pub fn filter_count_f64_gt(col: &[f64], t: f64) -> u64 {
    let mut c = 0u64;
    for &v in col {
        if v > t {
            c += 1;
        }
    }
    c
}

/// Builds the open-addressing (linear-probing) hash table both the CPU and GPU
/// probes walk: capacity = next power of two >= 2×keys (load factor <= 0.5),
/// `EMPTY_KEY` marks free slots, duplicates occupy separate slots (1:N join
/// semantics). Keys must not equal `EMPTY_KEY`.
pub fn build_hash_table(keys: &[u32], payloads: &[u32]) -> Vec<[u32; 2]> {
    assert_eq!(keys.len(), payloads.len());
    let cap = (keys.len() * 2).next_power_of_two().max(2);
    let mask = (cap - 1) as u32;
    let mut slots = vec![[EMPTY_KEY, 0u32]; cap];
    for (&k, &p) in keys.iter().zip(payloads) {
        assert_ne!(
            k, EMPTY_KEY,
            "EMPTY_KEY is reserved as the empty-slot sentinel"
        );
        let mut slot = (hash32(k) & mask) as usize;
        while slots[slot][0] != EMPTY_KEY {
            slot = (slot + 1) & mask as usize;
        }
        slots[slot] = [k, p];
    }
    slots
}

/// Probes every id against the table; returns (matches, sum of matched
/// payloads) — identical walk order and semantics to the WGSL kernel.
pub fn hash_probe(slots: &[[u32; 2]], probe: &[u32]) -> (u64, u64) {
    let mask = (slots.len() - 1) as u32;
    let mut matches = 0u64;
    let mut sum = 0u64;
    for &k in probe {
        let mut slot = (hash32(k) & mask) as usize;
        // Bounded at mask + 1 steps, mirroring the kernel (GitHub #4603).
        for _ in 0..=mask {
            let [key, payload] = slots[slot];
            if key == EMPTY_KEY {
                break;
            }
            if key == k {
                matches += 1;
                sum += u64::from(payload);
            }
            slot = (slot + 1) & mask as usize;
        }
    }
    (matches, sum)
}

/// COUNT + SUM GROUP BY. Returns `(count, sum)` per group, or
/// [`GroupKeyOutOfRange`] if any `keys[i] >= groups` — the same contract as
/// [`crate::Gpu::group_aggregate`] (GitHub #4603).
pub fn group_aggregate(
    keys: &[u32],
    vals: &[u32],
    groups: u32,
) -> Result<Vec<(u64, u64)>, GroupKeyOutOfRange> {
    assert_eq!(keys.len(), vals.len());
    let mut out = vec![(0u64, 0u64); groups as usize];
    for (&k, &v) in keys.iter().zip(vals) {
        let e = out.get_mut(k as usize).ok_or(GroupKeyOutOfRange)?;
        e.0 += 1;
        e.1 += u64::from(v);
    }
    Ok(out)
}

// [OPUS-4.8] sq-goay: unit tests for the CPU reference itself.
//
// `tests/correctness.rs` only exercises these functions transitively, *and only
// when a GPU adapter exists* — on CI (no device) every differential test takes
// the skip path, so the oracle ships with ~0% real coverage (the audit's "4.1%
// — only CPU fallback runs" gap). These tests run on every `cargo test`,
// device or not, and pin the reference to an INDEPENDENT brute-force oracle so
// "bit-identical CPU fallback" is a guarantee, not an aspiration.
#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift64* (same stream the differential tests use).
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

    // ---- filter_count_u32 -------------------------------------------------

    #[test]
    fn filter_u32_brute_force_random() {
        let mut rng = Rng(0xABCD_1234_5678_9F01);
        let col: Vec<u32> = (0..5_000).map(|_| rng.u32()).collect();
        for (lo, hi) in [
            (0u32, u32::MAX),
            (0, u32::MAX / 8),
            (5, 5),  // empty range
            (10, 5), // inverted range => empty
            (u32::MAX / 2, u32::MAX / 2 + 1_000_000),
            (u32::MAX, u32::MAX), // empty at the top edge
        ] {
            // Independent oracle: filter().count(), not the same loop shape.
            let want = col.iter().filter(|&&v| v >= lo && v < hi).count() as u64;
            assert_eq!(filter_count_u32(&col, lo, hi), want, "lo={lo} hi={hi}");
        }
    }

    #[test]
    fn filter_u32_boundary_is_half_open() {
        // `lo <= v < hi`: lo included, hi excluded.
        let col = [4u32, 5, 6, 7];
        assert_eq!(filter_count_u32(&col, 5, 7), 2, "{{5,6}} match, 7 excluded");
        assert_eq!(filter_count_u32(&[], 0, 10), 0);
    }

    // ---- filter_count_f64_gt ---------------------------------------------

    #[test]
    fn filter_f64_gt_brute_force_random() {
        let mut rng = Rng(0x0F1E_2D3C_4B5A_6978);
        let col: Vec<f64> = (0..5_000)
            .map(|_| (rng.u32() as f64 - (u32::MAX / 2) as f64) / 1e3)
            .collect();
        for t in [
            0.0,
            -0.0,
            1.5,
            -123_456.789,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let want = col.iter().filter(|&&v| v > t).count() as u64;
            assert_eq!(filter_count_f64_gt(&col, t), want, "t={t}");
        }
    }

    #[test]
    fn filter_f64_gt_ieee_edges() {
        // Every awkward IEEE-754 citizen, asserted against the language's own
        // `>` so the reference cannot drift from IEEE semantics.
        let col = [
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            5e-324,
            -5e-324,
            f64::MAX,
            f64::MIN,
        ];
        for t in [
            f64::NEG_INFINITY,
            -0.0,
            0.0,
            f64::INFINITY,
            f64::NAN,
            5e-324,
        ] {
            let want = col.iter().filter(|&&v| v > t).count() as u64;
            assert_eq!(filter_count_f64_gt(&col, t), want, "t={t}");
        }
        // v > NaN is false for every v, including NaN itself.
        assert_eq!(filter_count_f64_gt(&col, f64::NAN), 0);
        // -0.0 and 0.0 are equal under `>`, so the count must agree.
        assert_eq!(
            filter_count_f64_gt(&col, 0.0),
            filter_count_f64_gt(&col, -0.0),
            "-0.0 and 0.0 compare equal"
        );
    }

    // ---- build_hash_table / hash_probe -----------------------------------

    #[test]
    fn build_hash_table_invariants() {
        let mut rng = Rng(0x1357_9BDF_2468_ACE0);
        let n = 1_000usize;
        let keys: Vec<u32> = (0..n).map(|_| rng.u32() % (2 * n as u32)).collect();
        let payloads: Vec<u32> = (0..n).map(|_| rng.u32()).collect();
        let slots = build_hash_table(&keys, &payloads);

        // capacity = next pow2 >= 2n, load factor <= 0.5.
        assert!(slots.len().is_power_of_two());
        assert!(slots.len() >= 2 * n, "load factor must stay <= 0.5");

        // Exactly n occupied slots (duplicates each take a separate slot), and
        // every (key,payload) pair from the input is present somewhere.
        let occupied = slots.iter().filter(|s| s[0] != EMPTY_KEY).count();
        assert_eq!(occupied, n, "1:N join semantics — no de-dup on insert");
        for (&k, &p) in keys.iter().zip(&payloads) {
            assert!(
                slots.contains(&[k, p]),
                "inserted pair ({k},{p}) missing from the table"
            );
        }
    }

    #[test]
    #[should_panic(expected = "EMPTY_KEY is reserved")]
    fn build_hash_table_rejects_sentinel_key() {
        build_hash_table(&[EMPTY_KEY], &[0]);
    }

    #[test]
    fn hash_probe_brute_force_random() {
        let mut rng = Rng(0x2468_ACE0_1357_9BDF);
        let n_build = 800usize;
        let build_keys: Vec<u32> = (0..n_build)
            .map(|_| rng.u32() % (2 * n_build as u32))
            .collect();
        let payloads: Vec<u32> = (0..n_build).map(|_| rng.u32()).collect();
        let probe: Vec<u32> = (0..3_000)
            .map(|_| {
                let k = rng.u32() % (4 * n_build as u32);
                if k == EMPTY_KEY {
                    0
                } else {
                    k
                }
            })
            .collect();

        let slots = build_hash_table(&build_keys, &payloads);
        let (matches, sum) = hash_probe(&slots, &probe);

        // Independent O(n*m) oracle: for each probe key, scan the raw build
        // arrays (NOT the hash table) and tally matches + summed payloads.
        let mut want_m = 0u64;
        let mut want_s = 0u64;
        for &k in &probe {
            for (&bk, &bp) in build_keys.iter().zip(&payloads) {
                if bk == k {
                    want_m += 1;
                    want_s += u64::from(bp);
                }
            }
        }
        assert_eq!(matches, want_m, "match count");
        assert_eq!(sum, want_s, "payload sum");
    }

    #[test]
    fn hash_probe_handles_collisions_and_duplicates() {
        // Two keys that the table can only separate by linear probing, plus a
        // duplicate key (two payloads under the same key => fanout 2).
        let keys = [7u32, 7, 42];
        let payloads = [10u32, 100, 1];
        let slots = build_hash_table(&keys, &payloads);
        // Probing 7 hits both duplicate rows; 42 hits one; 999 misses.
        let (m, s) = hash_probe(&slots, &[7, 42, 999]);
        assert_eq!(m, 3, "7 matches twice, 42 once, 999 zero");
        assert_eq!(s, 10 + 100 + 1);
    }

    /// GitHub #4603: a table with no EMPTY_KEY slot must not spin forever; the
    /// walk is bounded at one pass over the table.
    #[test]
    fn hash_probe_terminates_on_a_full_table() {
        let slots = [[1u32, 10], [2, 20], [3, 30], [4, 40]];
        assert_eq!(hash_probe(&slots, &[3, 9]), (1, 30));
    }

    // ---- group_aggregate -------------------------------------------------

    /// GitHub #4603: an out-of-range key is a typed error, not a panic.
    #[test]
    fn group_aggregate_rejects_out_of_range_key() {
        assert_eq!(
            group_aggregate(&[0, 3], &[1, 1], 3),
            Err(GroupKeyOutOfRange)
        );
        assert_eq!(group_aggregate(&[600], &[1], 3), Err(GroupKeyOutOfRange));
    }

    #[test]
    fn group_aggregate_brute_force_random() {
        let mut rng = Rng(0x9999_7777_5555_3333);
        let groups = 37u32;
        let keys: Vec<u32> = (0..6_000).map(|_| rng.u32() % groups).collect();
        let vals: Vec<u32> = (0..6_000).map(|_| rng.u32()).collect();
        let got = group_aggregate(&keys, &vals, groups).unwrap();

        // Independent oracle, per group.
        for g in 0..groups {
            let want_c = keys.iter().filter(|&&k| k == g).count() as u64;
            let want_s: u64 = keys
                .iter()
                .zip(&vals)
                .filter(|(&k, _)| k == g)
                .map(|(_, &v)| u64::from(v))
                .sum();
            assert_eq!(got[g as usize], (want_c, want_s), "group {g}");
        }
        // Conservation: every row lands in exactly one group.
        assert_eq!(got.iter().map(|(c, _)| c).sum::<u64>(), keys.len() as u64);
    }

    #[test]
    fn group_aggregate_sum_exceeds_u32() {
        // Single group, large values: the u64 accumulator must not wrap where a
        // u32 would (this is exactly what the GPU's lo/hi carry emulation must
        // also get right, so the reference has to be unambiguous).
        let n = 100usize;
        let keys = vec![0u32; n];
        let vals = vec![u32::MAX; n];
        let got = group_aggregate(&keys, &vals, 1).unwrap();
        assert_eq!(got[0], (n as u64, n as u64 * u64::from(u32::MAX)));
        assert!(got[0].1 > u64::from(u32::MAX), "sum overflows u32");
    }
}
