// sq-it50: a minimal ChaCha20 CSPRNG that sparq fully OWNS, so its
// key schedule can be scrubbed on drop.
//! Owned ChaCha20 keystream generator with a scrubbable key schedule.
//!
//! ## Why this module exists (sq-it50, follow-up to sq-19ej)
//!
//! [`crate::rng::SecureRng`] needs a ChaCha20 CSPRNG whose **entire secret state
//! is zeroized on drop**. The ecosystem `rand_chacha::ChaCha20Rng` cannot do this
//! from our side: it exposes no `zeroize` feature, no `Drop`/`Zeroize` impl, and
//! no `&mut` accessor to its private, expanded key words (the key schedule,
//! recoverable via `get_seed()`). So the residual secret — the expanded ChaCha key
//! — lingered on the stack after drop. That was the documented, NOT-scrubbed
//! residual on `SecureRng` (sq-19ej).
//!
//! This module closes that residual by owning a tiny, self-contained ChaCha20
//! keystream generator. Because *we* hold the 16-word block state, we can
//! `#[derive(ZeroizeOnDrop)]` on it: every secret word (the key, the block
//! counter, and the cached keystream block) is overwritten with zeros when the
//! generator is dropped. There is no longer any sparq-reachable copy of the key
//! material that survives drop.
//!
//! ## What this is (and is NOT)
//!
//! - It IS the standard ChaCha20 block function (RFC 8439 §2.3): a 256-bit key, a
//!   64-bit block counter, and a 64-bit nonce arranged as the canonical 4×4 state
//!   of 32-bit words `["expa","nd 3","2-by","te k", key×8, counter_lo, counter_hi,
//!   nonce_lo, nonce_hi]`, run through 20 rounds (10 column + 10 diagonal
//!   double-rounds). Correctness is pinned to the RFC 8439 §2.3.2 and §2.4.2
//!   known-answer vectors in the tests.
//! - The nonce is fixed at **zero**: this is a *deterministic CSPRNG keyed by a
//!   fresh OS seed*, not a stream cipher reused across messages. Each
//!   [`crate::rng::SecureRng`] mints a brand-new generator from fresh OS entropy
//!   and never re-keys, so a fixed nonce is correct (the (key, nonce) pair is used
//!   exactly once, for one ever-incrementing counter sequence). It is NOT to be
//!   used to encrypt with a caller-chosen or reused key.
//! - The 64-bit block counter is monotone and wide enough that a single session
//!   cannot exhaust it (2^64 blocks = 2^70 bytes); we still guard the wrap in
//!   debug to surface misuse.
//!
//! ## Security note
//!
//! ChaCha20 is a vetted CSPRNG construction; the security of the masks/shares
//! rests on the OS seed being unpredictable (`getrandom`) and the key schedule not
//! leaking — which is exactly what the scrub-on-drop here protects. The quarter
//! round and block function are constant-time (no secret-dependent branch or
//! index); throughput is not a concern for the masking path (a handful of field
//! elements per dealing session).

use zeroize::{Zeroize, ZeroizeOnDrop};

/// The four ChaCha constant words: the ASCII of "expand 32-byte k", little-endian.
const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// ChaCha20 keystream generator owning its full block state so the key schedule
/// can be zeroized on drop.
///
/// `state` is the canonical 16-word ChaCha block (constants, 8 key words, 2
/// counter words, 2 nonce words). `keystream` caches the current 64-byte output
/// block; `pos` is the byte cursor into it. `ZeroizeOnDrop` wipes ALL of these —
/// the key words, the counter, and any buffered keystream bytes — on drop. There
/// is no `Clone` (duplicating CSPRNG state would reuse the mask stream) and no
/// accessor that returns the key.
// `Zeroize` gives the in-place `zeroize()` used by the `ZeroizeOnDrop` glue (and
// lets the test assert the scrub on a live value without touching dropped memory).
#[derive(Zeroize, ZeroizeOnDrop)]
pub(crate) struct ChaCha20Csprng {
    /// Canonical 4×4 ChaCha state: `[constants×4, key×8, counter_lo, counter_hi,
    /// nonce_lo, nonce_hi]`. The key words and counter are secret.
    state: [u32; 16],
    /// The most recently generated 64-byte keystream block (secret: it is the raw
    /// mask material). Wiped on drop with everything else.
    keystream: [u8; 64],
    /// Byte cursor into `keystream`; `64` means "no buffered bytes, refill next".
    /// Not secret, but zeroizing a `usize` is free and keeps the derive simple.
    pos: usize,
}

impl ChaCha20Csprng {
    /// Build a generator from a 32-byte seed (the ChaCha key), nonce = 0, counter
    /// = 0. The seed is consumed by value; callers pass a copy they themselves
    /// scrub (see [`crate::rng::SecureRng::from_os`]).
    pub(crate) fn from_seed(seed: [u8; 32]) -> Self {
        let mut state = [0u32; 16];
        state[0..4].copy_from_slice(&CONSTANTS);
        for (i, chunk) in seed.chunks_exact(4).enumerate() {
            state[4 + i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        // state[12..14] = block counter (starts at 0); state[14..16] = nonce (0).
        // Already zero-initialised.
        Self {
            state,
            keystream: [0u8; 64],
            // Force a keystream refill on the first byte request.
            pos: 64,
        }
    }

    /// One ChaCha quarter round on four state words (RFC 8439 §2.1).
    #[inline(always)]
    fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(16);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(12);
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(8);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(7);
    }

    /// Run the 20-round ChaCha block function on the current `state` (using the
    /// current counter) and write the 64-byte keystream block into `keystream`.
    /// Does NOT advance the counter (caller does, after consuming the block).
    fn fill_block(&mut self) {
        let mut working = self.state;
        // 10 double-rounds = 20 rounds: 4 column quarter-rounds + 4 diagonal.
        for _ in 0..10 {
            // Column rounds.
            Self::quarter_round(&mut working, 0, 4, 8, 12);
            Self::quarter_round(&mut working, 1, 5, 9, 13);
            Self::quarter_round(&mut working, 2, 6, 10, 14);
            Self::quarter_round(&mut working, 3, 7, 11, 15);
            // Diagonal rounds.
            Self::quarter_round(&mut working, 0, 5, 10, 15);
            Self::quarter_round(&mut working, 1, 6, 11, 12);
            Self::quarter_round(&mut working, 2, 7, 8, 13);
            Self::quarter_round(&mut working, 3, 4, 9, 14);
        }
        // Add the original input block (feed-forward) and serialise little-endian.
        for (i, w) in working.iter_mut().enumerate() {
            *w = w.wrapping_add(self.state[i]);
            self.keystream[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        // Scrub the working state: it equals (output − input) per word, which
        // together with the emitted keystream would reveal the input key words.
        working.zeroize();
    }

    /// Advance the 64-bit block counter (words 12 = low, 13 = high) by one block.
    #[inline]
    fn increment_counter(&mut self) {
        let (lo, carry) = self.state[12].overflowing_add(1);
        self.state[12] = lo;
        if carry {
            // 2^32 blocks consumed; bump the high word. A full 2^64-block wrap is
            // unreachable in a real masking session (2^70 bytes); guard misuse.
            self.state[13] = self.state[13].wrapping_add(1);
            debug_assert!(
                self.state[13] != 0 || self.state[12] != 0,
                "ChaCha20 block counter wrapped 2^64 blocks — CSPRNG misuse"
            );
        }
    }

    /// Produce the next 8 keystream bytes as a `u64` (little-endian), refilling and
    /// advancing the block as needed. This is the only output path [`crate::rng`]
    /// uses (it draws `u64`s and rejection-samples field elements from them).
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut out = [0u8; 8];
        for byte in &mut out {
            if self.pos >= 64 {
                self.fill_block();
                self.increment_counter();
                self.pos = 0;
            }
            *byte = self.keystream[self.pos];
            self.pos += 1;
        }
        u64::from_le_bytes(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8439 §2.3.2 known-answer: a single ChaCha20 block from the test key,
    /// nonce, and counter must match the published keystream. We construct the
    /// canonical state directly (this test exercises the block function against the
    /// RFC's exact nonce/counter, which differ from our CSPRNG's fixed-zero nonce).
    #[test]
    fn rfc8439_block_function_vector() {
        // Key 00,01,02 ... 1f.
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let mut g = ChaCha20Csprng::from_seed(key);
        // Override counter/nonce to the RFC §2.3.2 vector:
        // counter = 1 (word 12), nonce = 00:00:00:09 00:00:00:4a 00:00:00:00.
        g.state[12] = 1;
        g.state[13] = 0x0900_0000; // nonce word 0 = 09 00 00 00 LE
        g.state[14] = 0x4a00_0000; // nonce word 1 = 00 00 00 4a ... see below
        g.state[15] = 0x0000_0000;
        // RFC nonce bytes: 00 00 00 09 00 00 00 4a 00 00 00 00 -> words (LE):
        g.state[13] = u32::from_le_bytes([0x00, 0x00, 0x00, 0x09]);
        g.state[14] = u32::from_le_bytes([0x00, 0x00, 0x00, 0x4a]);
        g.state[15] = u32::from_le_bytes([0x00, 0x00, 0x00, 0x00]);
        g.fill_block();
        // Expected keystream block from RFC 8439 §2.3.2.
        let expected: [u8; 64] = [
            0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15, 0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20,
            0x71, 0xc4, 0xc7, 0xd1, 0xf4, 0xc7, 0x33, 0xc0, 0x68, 0x03, 0x04, 0x22, 0xaa, 0x9a,
            0xc3, 0xd4, 0x6c, 0x4e, 0xd2, 0x82, 0x64, 0x46, 0x07, 0x9f, 0xaa, 0x09, 0x14, 0xc2,
            0xd7, 0x05, 0xd9, 0x8b, 0x02, 0xa2, 0xb5, 0x12, 0x9c, 0xd1, 0xde, 0x16, 0x4e, 0xb9,
            0xcb, 0xd0, 0x83, 0xe8, 0xa2, 0x50, 0x3c, 0x4e,
        ];
        assert_eq!(
            g.keystream, expected,
            "ChaCha20 block function must match RFC 8439 §2.3.2"
        );
    }

    /// RFC 8439 §2.4.2 keystream: the first 64 keystream bytes for the §2.4.2 key
    /// and nonce at counter=1. Validates the block-function + serialisation path
    /// end to end against a second independent published vector.
    #[test]
    fn rfc8439_keystream_vector_2_4_2() {
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let mut g = ChaCha20Csprng::from_seed(key);
        // §2.4.2: counter = 1, nonce = 00:00:00:00 00:00:00:4a 00:00:00:00.
        g.state[12] = 1;
        g.state[13] = u32::from_le_bytes([0x00, 0x00, 0x00, 0x00]);
        g.state[14] = u32::from_le_bytes([0x00, 0x00, 0x00, 0x4a]);
        g.state[15] = u32::from_le_bytes([0x00, 0x00, 0x00, 0x00]);
        g.fill_block();
        // First keystream block from RFC 8439 §2.4.2 ("Sunscreen") example.
        let expected_first_block: [u8; 64] = [
            0x22, 0x4f, 0x51, 0xf3, 0x40, 0x1b, 0xd9, 0xe1, 0x2f, 0xde, 0x27, 0x6f, 0xb8, 0x63,
            0x1d, 0xed, 0x8c, 0x13, 0x1f, 0x82, 0x3d, 0x2c, 0x06, 0xe2, 0x7e, 0x4f, 0xca, 0xec,
            0x9e, 0xf3, 0xcf, 0x78, 0x8a, 0x3b, 0x0a, 0xa3, 0x72, 0x60, 0x0a, 0x92, 0xb5, 0x79,
            0x74, 0xcd, 0xed, 0x2b, 0x93, 0x34, 0x79, 0x4c, 0xba, 0x40, 0xc6, 0x3e, 0x34, 0xcd,
            0xea, 0x21, 0x2c, 0x4c, 0xf0, 0x7d, 0x41, 0xb7,
        ];
        assert_eq!(
            g.keystream, expected_first_block,
            "ChaCha20 keystream must match RFC 8439 §2.4.2 first block"
        );
    }

    /// Two generators from the same seed yield identical streams (deterministic in
    /// the seed), and from different seeds yield different streams (keyed). This is
    /// the basic CSPRNG sanity for the masking path.
    #[test]
    fn deterministic_in_seed_and_keyed() {
        let seed_a: [u8; 32] = core::array::from_fn(|i| i as u8);
        let seed_b: [u8; 32] = core::array::from_fn(|i| (i as u8) ^ 0xff);
        let mut a1 = ChaCha20Csprng::from_seed(seed_a);
        let mut a2 = ChaCha20Csprng::from_seed(seed_a);
        let mut b = ChaCha20Csprng::from_seed(seed_b);
        let sa1: Vec<u64> = (0..32).map(|_| a1.next_u64()).collect();
        let sa2: Vec<u64> = (0..32).map(|_| a2.next_u64()).collect();
        let sb: Vec<u64> = (0..32).map(|_| b.next_u64()).collect();
        assert_eq!(sa1, sa2, "same seed must reproduce the same stream");
        assert_ne!(sa1, sb, "different seeds must produce different streams");
    }

    /// The block counter advances across the 64-byte boundary: bytes 0..8 of block
    /// 0 then bytes 0..8 of block 1 must come from distinct ChaCha blocks (i.e. the
    /// 9th u64 is not a repeat of the 1st). Guards an off-by-one in `pos`/counter.
    #[test]
    fn counter_advances_across_block_boundary() {
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let mut g = ChaCha20Csprng::from_seed(key);
        // 64 bytes = 8 u64s consume exactly one block; the 9th forces a refill.
        let first_block: Vec<u64> = (0..8).map(|_| g.next_u64()).collect();
        let ninth = g.next_u64();
        assert!(
            !first_block.contains(&ninth),
            "9th u64 must come from the next ChaCha block, not repeat block 0"
        );
        // Counter low word must have advanced to 2 (block 0 and block 1 consumed).
        assert_eq!(g.state[12], 2, "block counter must have advanced by two");
    }

    /// sq-it50 — the core of this bead: the key schedule (and the
    /// counter + cached keystream) is scrubbed. `zeroize()` is exactly what the
    /// derived `Drop`/`ZeroizeOnDrop` runs, so asserting it wipes the state on a
    /// live value proves drop scrubs the key — WITHOUT the UB of reading dropped
    /// memory. (Under `rand_chacha` this was impossible: no `zeroize`/`&mut`-key
    /// access, the residual sq-19ej had to document as NOT-scrubbed.)
    #[test]
    fn key_schedule_is_zeroized_on_drop() {
        let key: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_add(1));
        let mut g = ChaCha20Csprng::from_seed(key);
        // Draw some output so the keystream block + counter hold secret material.
        for _ in 0..10 {
            let _ = g.next_u64();
        }
        // The key words (state[4..12]) must be non-zero before the scrub (the seed
        // bytes 1..=32 are all non-zero, so every key word is non-zero).
        assert!(
            g.state[4..12].iter().any(|&w| w != 0),
            "precondition: key schedule should hold the seed before scrub"
        );
        // This is the exact operation `Drop` performs.
        g.zeroize();
        assert!(
            g.state.iter().all(|&w| w == 0),
            "key schedule + counter + nonce must be zero after scrub"
        );
        assert!(
            g.keystream.iter().all(|&b| b == 0),
            "cached keystream block must be zero after scrub"
        );
        assert_eq!(g.pos, 0, "byte cursor must be zero after scrub");
    }
}
