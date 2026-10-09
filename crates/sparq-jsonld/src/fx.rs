//! A fast keyed hash for internal maps keyed by IRIs, blank-node labels and terms. The keys
//! can come from untrusted documents, so each process draws a random key (from std's
//! `RandomState`) and every block goes through a folded 64×64→128-bit multiply with it:
//! unlike FxHash, collisions cannot be precomputed without the key. On
//! wasm32-unknown-unknown, which has no entropy source, the maps are ordered instead.
//! Kept in-crate so `sparq-jsonld` stays dependency-free.

// The hasher is unused where the maps are ordered.
#![cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]

use std::collections::hash_map::RandomState;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;

/// The process-wide random key: two odd multipliers.
fn key() -> [u64; 2] {
    static KEY: OnceLock<[u64; 2]> = OnceLock::new();
    *KEY.get_or_init(|| {
        let r = RandomState::new();
        [r.hash_one(0u64) | 1, r.hash_one(1u64) | 1]
    })
}

#[inline]
fn folded_mul(a: u64, b: u64) -> u64 {
    let p = u128::from(a).wrapping_mul(u128::from(b));
    (p as u64) ^ ((p >> 64) as u64)
}

/// Keyed hash state.
pub(crate) struct FxHasher {
    state: u64,
    k: u64,
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            let w = u64::from_le_bytes(c.try_into().expect("8-byte chunk"));
            self.state = folded_mul(self.state ^ w, self.k);
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            let w = u64::from_le_bytes(buf) ^ ((rest.len() as u64) << 59);
            self.state = folded_mul(self.state ^ w, self.k);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.state = folded_mul(self.state ^ u64::from(i), self.k);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.state = folded_mul(self.state ^ i as u64, self.k);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }
}

/// Builds [`FxHasher`]s with the process key.
#[derive(Clone, Copy)]
pub(crate) struct FxBuild([u64; 2]);

impl Default for FxBuild {
    #[inline]
    fn default() -> Self {
        FxBuild(key())
    }
}

impl BuildHasher for FxBuild {
    type Hasher = FxHasher;

    #[inline]
    fn build_hasher(&self) -> FxHasher {
        FxHasher {
            state: self.0[1],
            k: self.0[0],
        }
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) type FxMap<K, V> = HashMap<K, V, FxBuild>;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) type FxSet<K> = HashSet<K, FxBuild>;

// `RandomState` has no entropy source on wasm32-unknown-unknown, so a key there would be
// predictable; ordered maps keep the worst case at O(log n) per lookup instead.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) type FxMap<K, V> = std::collections::BTreeMap<K, V>;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) type FxSet<K> = std::collections::BTreeSet<K>;

#[cfg(test)]
mod tests {
    use super::*;

    /// FxHash (`(rotl5(h) ^ w) * K` per block), seeded with `seed`.
    fn fxhash(seed: u64, bytes: &[u8]) -> u64 {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        bytes.chunks_exact(8).fold(seed, |h, c| {
            (h.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(K)
        })
    }

    // Flipping the top bit of the first block flips only the top bit of FxHash's state
    // (K is odd), which the next block can cancel at bit 4: a collision for every seed.
    // The keyed hash does not share it.
    #[test]
    fn fxhash_seed_independent_collision_does_not_carry_over() {
        let a = [0u64.to_le_bytes(), 1u64.to_le_bytes()].concat();
        let c = [(1u64 << 63).to_le_bytes(), (1u64 ^ 16).to_le_bytes()].concat();
        for seed in [0, 7, u64::MAX] {
            assert_eq!(fxhash(seed, &a), fxhash(seed, &c));
        }
        let b = FxBuild::default();
        assert_ne!(b.hash_one(&a[..]), b.hash_one(&c[..]));
    }
}
