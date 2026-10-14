// sq-1vt: cryptographically-secure masking RNG for the Shamir backend.
//! Randomness for secret-sharing / masking — the security-critical RNG seam.
//!
//! ## Why this module exists (sq-1vt, a real confidentiality weakness)
//!
//! Shamir secret sharing hides a secret behind a degree-`t` polynomial whose
//! *free coefficients are random*, and the circuit-PSI equality test hides the
//! key difference behind a *random nonzero mask*. The whole information-theoretic
//! hiding argument (any `<= t` shares are independent of the secret; the opened
//! product `m = d·r` leaks nothing about `d` beyond `d == 0`) assumes those
//! coefficients / masks are **unpredictable**. If they come from a deterministic,
//! seedable, non-cryptographic PRNG (e.g. SplitMix64), an adversary who learns or
//! guesses the seed/state can reconstruct every mask and every share — collapsing
//! confidentiality. **Masking randomness MUST come from a CSPRNG.**
//!
//! ## The seam: [`MpcRng`]
//!
//! The backend draws masking field elements through the [`MpcRng`] trait, not a
//! concrete RNG. There are exactly two implementors and they are NOT
//! interchangeable by accident:
//!
//! - [`SecureRng`] — the **production / default** masking RNG. An **owned**
//!   ChaCha20 CSPRNG (`crate::chacha::ChaCha20Csprng`, private to this crate)
//!   seeded from OS entropy (`getrandom::fill`). This is what the real
//!   protocol uses and the only RNG reachable from the default
//!   [`crate::shamir::ShamirBackend::new`] constructor. We own the cipher (rather
//!   than wrapping `rand_chacha::ChaCha20Rng`) specifically so its key schedule is
//!   zeroized on drop — see the secret-memory-hygiene note on [`SecureRng`]
//!   (sq-it50).
//! - `InsecureTestRng` (`#[cfg(any(test, feature = "insecure-test-rng"))]`,
//!   so not an intra-doc link in default builds) — a **deterministic, seedable
//!   SplitMix64** PRNG for
//!   reproducible tests/benchmarks ONLY. It is gated behind
//!   `#[cfg(any(test, feature = "insecure-test-rng"))]` so it cannot be
//!   constructed from a normal (non-test, default-feature) build. Its name
//!   carries the warning; there is no way to wire it into the real masking path
//!   without opting into the `insecure-test-rng` feature.
//!
//! ## Uniform field sampling (no modulo bias)
//!
//! Field elements must be **uniform over `F_p`** for the hiding argument to hold;
//! a skewed distribution leaks information about the secret. `Fp::new(raw_u64)`
//! reduces by `% P`, which is *biased* when `raw_u64` ranges over all of
//! `[0, 2^64)` (the residues `[0, 2^64 mod P)` are over-represented). With
//! `p = 2^61 - 1` we instead **rejection-sample**: take the low 61 bits of a
//! fresh `u64` (a value in `[0, 2^61)`) and reject the single value `2^61 - 1`
//! (`== P`, which is not a canonical residue), re-drawing on rejection. Rejection
//! probability is `1/2^61`, so this is effectively constant-time in expectation
//! and the result is *exactly* uniform over `{0, 1, …, P-1}`. See [`next_fp`].
//!
//! [`next_fp`]: MpcRng::next_fp

use crate::chacha::ChaCha20Csprng;
use crate::field::{Fp, P};

/// Source of randomness for the masking / secret-sharing path.
///
/// Implementors yield uniform field elements over `F_p`. The trait exists so the
/// security-critical [`SecureRng`] (CSPRNG) is the production default while a
/// deterministic `InsecureTestRng` (test-gated, so not an intra-doc link) stays
/// available for reproducible tests —
/// without the two being silently swappable in the real protocol (the insecure
/// impl is `cfg`-gated out of normal builds).
pub trait MpcRng {
    /// Fill `dest` with the next raw random bytes. Implementors back this with a
    /// CSPRNG (production) or a deterministic PRNG (test-only).
    fn next_u64(&mut self) -> u64;

    /// Draw one field element **uniformly** over `F_p` (rejection-sampled to
    /// avoid the modulo bias of `Fp::new(raw_u64)` — see the module note). This
    /// is the method the dealer uses for masking coefficients and the equality
    /// test uses for its nonzero mask.
    fn next_fp(&mut self) -> Fp {
        // Low 61 bits → value in [0, 2^61). Reject the single non-canonical
        // value 2^61-1 (== P). Everything in [0, P) is equally likely.
        const MASK_61: u64 = (1u64 << 61) - 1; // == P
        loop {
            let candidate = self.next_u64() & MASK_61;
            if candidate < P {
                // candidate is already canonical in [0, P); Fp::new is a no-op
                // reduce but we route through it to keep Fp's invariant explicit.
                return Fp::new(candidate);
            }
            // candidate == P (probability 1/2^61): re-draw. Loop is the exact,
            // bias-free path; we do not fall back to `% P`.
        }
    }

    /// Draw a **nonzero** field element uniformly over `F_p \ {0}`. The equality
    /// test's mask must be nonzero (masking by zero would open `m = 0` even for
    /// unequal keys → a false match). Re-draws on the `1/p` chance of zero.
    fn next_nonzero_fp(&mut self) -> Fp {
        loop {
            let v = self.next_fp();
            if v != Fp::zero() {
                return v;
            }
        }
    }
}

/// **Production / default masking RNG.** A ChaCha20 CSPRNG seeded from OS
/// entropy. This is the only RNG the default [`crate::shamir::ShamirBackend::new`]
/// constructor uses — the real protocol's masks and Shamir coefficients are
/// unpredictable (resolves sq-1vt for the masking path).
///
/// Not `Clone`: a CSPRNG state must not be duplicated (cloning would reuse the
/// same mask stream across two "dealers"). When the backend needs a fresh dealer
/// it *re-seeds* a brand-new `SecureRng` from the OS, never copies state.
///
/// # Secret-memory hygiene (sq-u8a8 / sq-19ej / sq-it50, from the side-channel analysis CR-G5)
/// The CSPRNG seed/key/keystream state is secret (it predicts every
/// mask and Shamir coefficient). `SecureRng` redacts its [`Debug`], forbids
/// [`Clone`], and zeroizes on drop **the entire secret state, including the key
/// schedule**:
///
/// - **The sparq-held seed buffer (scrubbed).** [`from_os`](SecureRng::from_os)
///   pulls the 32-byte ChaCha seed into a [`zeroize::Zeroizing`] buffer that this
///   crate owns, feeds a *copy* to the owned ChaCha constructor, and the buffer is
///   wiped when the constructor returns — so the only sparq-controlled secret byte
///   buffer never lingers.
/// - **The key schedule (scrubbed — sq-it50).** The inner CSPRNG is now an
///   **owned** `crate::chacha::ChaCha20Csprng` (NOT `rand_chacha`), whose full
///   16-word block state (the expanded ChaCha key, the block counter, and the
///   cached 64-byte keystream block) is `#[derive(ZeroizeOnDrop)]`. Dropping
///   `SecureRng` drops the inner cipher, which overwrites every secret word with
///   zeros. There is no longer any sparq-reachable copy of the key material that
///   survives drop, and no `get_seed()`-style accessor that re-exposes it.
///
/// ## Why we own the cipher (sq-it50)
/// Ecosystem `rand_chacha::ChaCha20Rng` could not be scrubbed in place from this
/// crate — verified at the latest published `0.10.0` (Feb 2026) as well as the
/// pinned `0.9`, it exposes **no `zeroize` feature**, implements no `Drop`/`Zeroize`,
/// and gives no `&mut` accessor to the private key words; its key schedule
/// (recoverable via `get_seed()`) lingered on the stack after drop. Rather than
/// fake a scrub we could not perform, sq-19ej documented that as an irreducible
/// residual and beaded the fix; sq-it50 closes it by replacing the dependency with
/// a minimal, fully-owned ChaCha20 keystream generator we *can* zeroize. The
/// generator is correctness-pinned to the RFC 8439 known-answer vectors (see
/// `crate::chacha`); RNG *behaviour* is unchanged for the protocol (a CSPRNG
/// keyed by fresh OS entropy, drawn as uniform field elements).
pub struct SecureRng {
    inner: ChaCha20Csprng,
}

impl SecureRng {
    /// Build a fresh CSPRNG seeded from operating-system entropy
    /// (`getrandom`). Panics only if the OS entropy source fails irrecoverably,
    /// which is the correct behaviour for a security-critical seed: we must NOT
    /// silently continue with predictable randomness.
    ///
    /// sq-19ej/sq-it50: the 32-byte seed is pulled into a [`Zeroizing`]
    /// buffer owned by this crate and wiped when this function returns, so the only
    /// sparq-held copy of the *seed* secret is scrubbed. The copy handed to the
    /// owned `crate::chacha::ChaCha20Csprng` is absorbed into its block state —
    /// which is itself `ZeroizeOnDrop`, so the expanded key schedule is scrubbed on
    /// drop too (sq-it50 closes the residual sq-19ej had to leave open under
    /// `rand_chacha`).
    ///
    /// [`Zeroizing`]: zeroize::Zeroizing
    pub fn from_os() -> Self {
        // Pull the ChaCha seed into a sparq-owned buffer we CAN scrub. `Zeroizing`
        // wipes `seed` on scope exit (incl. on an unwind), so the sparq-held seed
        // secret never lingers on the stack.
        let mut seed = zeroize::Zeroizing::new([0u8; 32]);
        // sq-8xug: draw the seed straight from the OS CSPRNG via
        // `getrandom::fill`. `rand_core` 0.10 removed `OsRng`, so the prior
        // `rand_chacha::rand_core::OsRng::try_fill_bytes` path no longer exists;
        // `getrandom` is the underlying OS-entropy source that `OsRng` wrapped.
        // It surfaces a hard OS-entropy failure as an error instead of silently
        // degrading to a weak/known seed (which would reintroduce sq-1vt).
        // Panicking on that failure is intended for a security seed.
        getrandom::fill(seed.as_mut())
            .expect("OS entropy source failed while seeding the masking CSPRNG (sq-1vt)");
        // `from_seed` takes the seed by value (a copy); `seed` itself is still
        // scrubbed by `Zeroizing` at end of scope. The inner cipher's key schedule
        // is scrubbed on drop by its own `ZeroizeOnDrop` (sq-it50).
        SecureRng {
            inner: ChaCha20Csprng::from_seed(*seed),
        }
    }
}

impl Default for SecureRng {
    fn default() -> Self {
        Self::from_os()
    }
}

impl std::fmt::Debug for SecureRng {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print CSPRNG internal state.
        f.debug_struct("SecureRng")
            .field("inner", &"ChaCha20Csprng(<redacted>)")
            .finish()
    }
}

impl MpcRng for SecureRng {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }
}

/// **INSECURE, test/benchmark-only** deterministic SplitMix64 PRNG.
///
/// Gated behind `#[cfg(any(test, feature = "insecure-test-rng"))]` so it cannot
/// be constructed from a normal default build — the real masking path is
/// physically unable to reach it without opting into the `insecure-test-rng`
/// feature. Used so the in-process multi-party SIMULATION and its differential /
/// stress tests are reproducible from a fixed seed. **No security is claimed**;
/// a deterministic PRNG makes shares predictable and would break confidentiality
/// in a real deployment (the very weakness sq-1vt fixes).
#[cfg(any(test, feature = "insecure-test-rng"))]
#[derive(Debug, Clone)]
pub struct InsecureTestRng {
    state: u64,
}

#[cfg(any(test, feature = "insecure-test-rng"))]
impl InsecureTestRng {
    /// New deterministic PRNG from a fixed seed (reproducible simulation). The
    /// name is the warning: this is NOT for the real masking path.
    pub fn new(seed: u64) -> Self {
        InsecureTestRng { state: seed }
    }
}

#[cfg(any(test, feature = "insecure-test-rng"))]
impl MpcRng for InsecureTestRng {
    fn next_u64(&mut self) -> u64 {
        // SplitMix64 (deterministic; simulation/test only).
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_rng_produces_canonical_field_elements() {
        let mut rng = SecureRng::from_os();
        for _ in 0..10_000 {
            let v = rng.next_fp();
            assert!(v.value() < P, "field element must be canonical in [0, P)");
        }
    }

    #[test]
    fn secure_rng_nonzero_is_never_zero() {
        let mut rng = SecureRng::from_os();
        for _ in 0..10_000 {
            assert_ne!(rng.next_nonzero_fp(), Fp::zero());
        }
    }

    #[test]
    fn secure_rng_two_instances_differ() {
        // Two freshly OS-seeded CSPRNGs must not produce the same stream
        // (probability of collision is cryptographically negligible). This is the
        // unpredictability witness: the masks are not a fixed sequence.
        let mut a = SecureRng::from_os();
        let mut b = SecureRng::from_os();
        let sa: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let sb: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(sa, sb, "two OS-seeded CSPRNGs produced identical streams");
    }

    #[test]
    fn insecure_test_rng_is_deterministic_and_canonical() {
        let mut a = InsecureTestRng::new(0xC0FFEE);
        let mut b = InsecureTestRng::new(0xC0FFEE);
        for _ in 0..1_000 {
            let va = a.next_fp();
            let vb = b.next_fp();
            assert_eq!(va, vb, "same seed must reproduce the same stream");
            assert!(va.value() < P);
        }
    }

    #[test]
    fn secure_rng_from_os_seed_path_still_unpredictable() {
        // sq-19ej: switching from `from_os_rng()` to a scrubbed-seed
        // `from_seed` constructor must NOT change the security property — two
        // freshly OS-seeded instances still produce independent streams. (RNG
        // behaviour is identical; the zeroize is drop-time only.)
        let mut a = SecureRng::from_os();
        let mut b = SecureRng::from_os();
        let sa: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let sb: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(sa, sb, "scrubbed-seed CSPRNGs produced identical streams");
    }

    #[test]
    fn secure_rng_drop_does_not_panic() {
        // sq-it50: dropping `SecureRng` drops the owned
        // `ChaCha20Csprng`, whose `ZeroizeOnDrop` scrubs the full key schedule +
        // keystream. The drop must run cleanly (the key-schedule scrub is now
        // complete, not best-effort). The actual zeroing is asserted in the
        // `chacha` module's `key_schedule_is_zeroized_on_drop` test.
        let rng = SecureRng::from_os();
        drop(rng);
    }

    #[test]
    fn rejection_sampling_covers_the_top_residue() {
        // Sanity: a known SplitMix64 stream still yields canonical elements; the
        // rejection loop never returns P or wraps incorrectly.
        let mut rng = InsecureTestRng::new(1);
        let mut saw_large = false;
        for _ in 0..100_000 {
            let v = rng.next_fp().value();
            assert!(v < P);
            if v > P / 2 {
                saw_large = true;
            }
        }
        assert!(
            saw_large,
            "sampler should reach the upper half of the field"
        );
    }
}
