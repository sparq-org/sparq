// Prime-field arithmetic for Shamir secret sharing (M3).
//! Arithmetic in the prime field `F_p` underpinning Shamir secret sharing.
//!
//! Architecture refs: §3.1 (secret-sharing families), §4.3 step 4 (the secure
//! computation runs over secret-shared values). Honest-majority Shamir requires
//! a field; this module is that field and nothing more — no networking, no
//! sharing logic (that is [`crate::shamir`]).
//!
//! ## Choice of `p`
//!
//! A 61-bit Mersenne prime `p = 2^61 - 1` (the largest Mersenne prime below
//! 2^63). Three reasons:
//! - **Products stay in `u128`.** Two field elements are `< 2^61`, so their
//!   product is `< 2^122`, comfortably inside `u128` — reduction is a couple of
//!   shifts-and-adds, no big-integer dependency. This keeps the crate
//!   dependency-free (no `num-bigint` / `ark-ff` reaching the build graph) which
//!   matters for the native-only / wasm-exclusion invariant.
//! - **Range covers the use case.** Salaries, counts, and 61-bit-truncated IRI
//!   fingerprints all fit under 2^61. The flatmate cumulative-salary aggregate
//!   (four × ~10^5) is six orders of magnitude clear of the modulus, so the
//!   secure sum never wraps and reconstructs to the true integer.
//! - **It is a real prime field**, so Lagrange interpolation (reconstruction)
//!   and the additive/multiplicative structure Shamir relies on are exact — this
//!   is NOT a toy modular-arithmetic stand-in.
//!
//! ## Honesty note
//!
//! This is textbook finite-field arithmetic (Shamir 1979; standard MPC field
//! ops). It is the *correct* substrate for honest-majority Shamir. It is not, by
//! itself, a security claim: security comes from the sharing scheme + threshold
//! ([`crate::shamir`]) under the honest-majority semi-honest model stated there.

/// The field modulus `p = 2^61 - 1` (a Mersenne prime).
pub const P: u64 = (1u64 << 61) - 1;

/// sq-mnv5 — the Tonelli–Shanks shortcut exponent `(p + 1) / 4`, valid
/// because `p ≡ 3 (mod 4)` (concretely `p = 2^61 − 1 ≡ 7 (mod 8)`). For any
/// quadratic residue `c`, `c^((p+1)/4)` is a square root of `c` — i.e. `d² = c`.
/// This is the public, in-circuit-cheap square-root the random-bit sub-protocol
/// ([`crate::compare`] square-protocol) needs to derive the sign bit of a
/// secret-shared random `a` from the OPENED public square `c = a²`. The exponent
/// is a **public constant**, and its only use ([`Fp::sqrt_residue`]) is applied to
/// a **public** opened value, so no secret-dependent timing is involved.
pub const SQRT_EXP: u64 = (P + 1) / 4;

/// An element of `F_p`, always kept canonical in `[0, P)`.
///
/// sq-u8a8: derives [`zeroize::Zeroize`] so a secret-bearing `Fp`
/// (a Shamir share value, the cleartext MAC key `α`, a masking coefficient) can
/// be scrubbed from memory by the containers that own it. `Fp` is `Copy`, so it
/// cannot itself implement `Drop`/`ZeroizeOnDrop`; the zeroize-on-drop happens in
/// the owning secret container (e.g. `MacSession`, `MacKey`). This is memory
/// HYGIENE — `Zeroize` adds a scrub method, it changes no arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, zeroize::Zeroize)]
pub struct Fp(u64);

// Inherent `add`/`sub`/`mul`/`neg` are deliberate: field arithmetic reads
// clearest as `a.add(b)` in the share/interpolation code, and `Fp` is an
// internal crate type (not a public numeric tower), so shadowing the std op
// trait names carries no confusion risk for callers.
#[allow(clippy::should_implement_trait)]
impl Fp {
    /// Embed a `u64` into the field (reducing mod `p`).
    #[inline]
    pub fn new(v: u64) -> Self {
        Fp(v % P)
    }

    /// The raw canonical representative in `[0, P)`.
    #[inline]
    pub fn value(self) -> u64 {
        self.0
    }

    /// Additive identity.
    #[inline]
    pub fn zero() -> Self {
        Fp(0)
    }

    /// Multiplicative identity.
    #[inline]
    pub fn one() -> Self {
        Fp(1)
    }

    /// Field addition.
    #[inline]
    pub fn add(self, other: Fp) -> Fp {
        // a, b < P < 2^61, so a + b < 2^62 — no u64 overflow before reduction.
        let s = self.0 + other.0;
        Fp(if s >= P { s - P } else { s })
    }

    /// Field subtraction.
    #[inline]
    pub fn sub(self, other: Fp) -> Fp {
        Fp(if self.0 >= other.0 {
            self.0 - other.0
        } else {
            // self - other + P, computed without underflow.
            self.0 + P - other.0
        })
    }

    /// Field negation.
    #[inline]
    pub fn neg(self) -> Fp {
        if self.0 == 0 {
            Fp(0)
        } else {
            Fp(P - self.0)
        }
    }

    /// Field multiplication. Reduces the `u128` product mod the Mersenne prime.
    #[inline]
    pub fn mul(self, other: Fp) -> Fp {
        let prod = (self.0 as u128) * (other.0 as u128);
        Fp(reduce_mersenne(prod))
    }

    /// Multiplicative inverse via Fermat's little theorem: `a^(p-2) mod p`.
    /// Panics if `self` is zero (zero has no inverse) — callers (Lagrange
    /// interpolation) only ever invert non-zero differences of distinct points.
    ///
    /// sq-7ltf: this routes through [`Fp::pow_ct`] (a fixed-iteration,
    /// branchless square-and-multiply), so the inverse is **constant-time in the
    /// inverted value `self`** by construction — there is no data-dependent
    /// multiply branch on the base. The exponent (`P − 2`) is a public constant,
    /// so its bit pattern is public regardless; routing through `pow_ct` removes
    /// the *value*-dependent timing that a future secret-value inversion would
    /// otherwise expose. This closes the latent `Fp::pow` hazard (CR-G5) for the
    /// inversion path without changing any arithmetic result. The panic on zero
    /// is a public-domain contract (zero is rejected before any frame crosses the
    /// wire; see `transport.rs`), not a secret-dependent branch.
    pub fn inv(self) -> Fp {
        assert!(self.0 != 0, "Fp::inv of zero");
        self.pow_ct(P - 2)
    }

    /// **Variable-time** modular exponentiation by square-and-multiply.
    ///
    /// sq-7ltf: this is the classic data-dependent square-and-multiply
    /// — `if exp & 1 == 1 { acc = acc.mul(base) }` branches on the **exponent's
    /// bits**, so its timing leaks the exponent's bit pattern / Hamming weight.
    ///
    /// # Public-exponent contract
    ///
    /// This function is **non-constant-time in the exponent** and MUST only be
    /// called with a **PUBLIC** exponent. The side-channel analysis (CR-G5) found
    /// every present caller satisfies this: the only direct caller is the robust
    /// Berlekamp–Welch decoder, which raises a public evaluation point to the
    /// **public** error-correction parameter `e` (`robust.rs`). Inversion does
    /// **not** use this path — it uses [`Fp::pow_ct`] so it is constant-time in
    /// the base by construction. If a **secret** exponent is ever needed, use
    /// [`Fp::pow_ct`] instead; never widen this function's callers to secret
    /// exponents.
    ///
    /// The `_vartime` suffix is the load-bearing marker: it makes the
    /// non-constant-time property visible at every call site.
    pub fn pow_vartime(self, mut exp: u64) -> Fp {
        let mut base = self;
        let mut acc = Fp::one();
        while exp > 0 {
            if exp & 1 == 1 {
                acc = acc.mul(base);
            }
            base = base.mul(base);
            exp >>= 1;
        }
        acc
    }

    /// Fixed-iteration, branchless modular exponentiation.
    ///
    /// sq-7ltf: a square-and-multiply that runs a **fixed 64
    /// iterations** (one per `u64` exponent bit) and selects the conditional
    /// multiply with [`subtle::ConditionallySelectable`] instead of an `if`. There is no
    /// data-dependent branch on the base value, so the routine is **constant-time
    /// in the base `self`** — closing the latent `Fp::pow` hazard for any caller
    /// that exponentiates a secret-bearing field element. It is **not** asserted
    /// constant-time in the *exponent*: the `exp.bit(i)` extraction reads the
    /// exponent bits, and the iteration count is fixed at 64 regardless, but the
    /// loop body does not vary with the secret base. (`subtle` is a manifest dep,
    /// adopted in sq-u8a8 as a defensive constant-time primitive.)
    ///
    /// Used by [`Fp::inv`] (public exponent `P − 2`); also available should a
    /// future secret-value exponentiation be required.
    pub fn pow_ct(self, exp: u64) -> Fp {
        use subtle::{ConditionallySelectable, ConstantTimeEq};
        let mut base = self;
        let mut acc = Fp::one();
        // Fixed 64 iterations — one per bit of the u64 exponent. No early exit,
        // so the trip count is independent of the base and of the exponent's
        // magnitude.
        for i in 0..u64::BITS {
            // bit = 1 iff the i-th exponent bit is set; constant-time extraction.
            let bit = ((exp >> i) & 1).ct_eq(&1);
            // Branchless: acc' = bit ? acc·base : acc.
            let multiplied = acc.mul(base);
            acc = Fp::conditional_select(&acc, &multiplied, bit);
            base = base.mul(base);
        }
        acc
    }

    /// sq-u8a8: constant-time equality on the canonical representative,
    /// via [`subtle::ConstantTimeEq`]. Because every `Fp` is kept canonical in
    /// `[0, P)`, comparing the `u64` values in constant time is equivalent to
    /// `self == other` but does not leak (through timing) whether the two values
    /// differ in their low or high bits.
    ///
    /// This is a **defensive PRIMITIVE**, not a replacement for any existing
    /// comparison. The side-channel analysis (CR-G5) found every equality on the
    /// MPC protocol paths is over PUBLIC / opened values — the masked product
    /// `m == 0` (which is exactly the disclosed match/zero bit), the verdict bit,
    /// and public threshold bits — so NO live `==` is secret-dependent and none is
    /// changed. `ct_eq` exists so a FUTURE secret-vs-secret field comparison can be
    /// constant-time by construction. The returned [`subtle::Choice`] is `1` iff
    /// `self == other`, identical to the boolean `==`.
    pub fn ct_eq(self, other: Fp) -> subtle::Choice {
        use subtle::ConstantTimeEq;
        self.0.ct_eq(&other.0)
    }

    /// sq-mnv5 — a square root of `self` for the `p ≡ 3 (mod 4)`
    /// shortcut: `self^((p+1)/4)`. If `self` is a quadratic residue this returns a
    /// `d` with `d² = self` (one of the two roots `±d`); if `self` is a
    /// NON-residue the identity does not hold, so the caller MUST verify
    /// `d.mul(d) == self` and treat a mismatch as "not a residue" (the random-bit
    /// sub-protocol only ever calls this on `c = a²`, which is a residue by
    /// construction, and re-checks anyway — fail-closed).
    ///
    /// # Public-value contract
    ///
    /// This routes through [`Fp::pow_vartime`] with the **public** exponent
    /// [`SQRT_EXP`]. It MUST only be called on a **public / opened** `self` (as the
    /// square-protocol does: it is applied to the opened `c = a²`, never to a
    /// secret share), so the variable-time exponentiation leaks nothing secret —
    /// the base and the exponent are both public.
    pub fn sqrt_residue(self) -> Fp {
        self.pow_vartime(SQRT_EXP)
    }
}

/// sq-7ltf: constant-time conditional select on the canonical
/// representative. Because every `Fp` is kept canonical in `[0, P)`, selecting
/// between two canonical `u64` values yields a canonical `Fp`; the underlying
/// `u64::conditional_select` (from `subtle`) is branchless. This is the
/// primitive [`Fp::pow_ct`] uses to make the conditional multiply branchless.
impl subtle::ConditionallySelectable for Fp {
    #[inline]
    fn conditional_select(a: &Fp, b: &Fp, choice: subtle::Choice) -> Fp {
        Fp(u64::conditional_select(&a.0, &b.0, choice))
    }
}

/// Reduce a 128-bit product modulo the Mersenne prime `p = 2^61 - 1` using the
/// identity `2^61 ≡ 1 (mod p)`: fold the high bits into the low bits. Two folds
/// then a conditional subtraction suffice for any `u128` product of two
/// sub-`2^61` operands.
#[inline]
fn reduce_mersenne(mut x: u128) -> u64 {
    const MASK: u128 = (1u128 << 61) - 1;
    // First fold: low 61 bits + (high bits × 2^61 ≡ 1, i.e. just added back).
    x = (x & MASK) + (x >> 61);
    // Second fold handles a carry the first fold may have pushed past 2^61.
    x = (x & MASK) + (x >> 61);
    let mut r = x as u64;
    if r >= P {
        r -= P;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sub_roundtrip() {
        let a = Fp::new(12345);
        let b = Fp::new(67890);
        assert_eq!(a.add(b).sub(b), a);
        assert_eq!(a.sub(b).add(b), a);
    }

    #[test]
    fn add_wraps_at_modulus() {
        let a = Fp::new(P - 1);
        assert_eq!(a.add(Fp::one()), Fp::zero());
    }

    #[test]
    fn neg_is_additive_inverse() {
        let a = Fp::new(999_999);
        assert_eq!(a.add(a.neg()), Fp::zero());
    }

    #[test]
    fn mul_reduces_near_modulus() {
        // Large operands near the modulus exercise the u128 reduction path.
        let a = Fp::new(P - 3);
        let b = Fp::new(P - 5);
        // (P-3)(P-5) = P^2 - 8P + 15 ≡ 15 (mod P).
        assert_eq!(a.mul(b), Fp::new(15));
    }

    #[test]
    fn inv_is_multiplicative_inverse() {
        for v in [1u64, 2, 3, 7, 12345, P - 1] {
            let a = Fp::new(v);
            assert_eq!(a.mul(a.inv()), Fp::one(), "inv of {v}");
        }
    }

    // sq-u8a8: the constant-time equality primitive returns the SAME
    // boolean as the plain `==` on a spread of representative inputs (equal,
    // unequal, boundary values). This is the "CT-eq agrees with the prior `==`"
    // gate from the bead — proving the hygiene primitive does not change any
    // comparison outcome.
    #[test]
    fn ct_eq_agrees_with_value_eq() {
        use subtle::ConstantTimeEq;
        let samples = [0u64, 1, 2, 7, 12345, P / 2, P - 2, P - 1];
        for &a in &samples {
            for &b in &samples {
                let fa = Fp::new(a);
                let fb = Fp::new(b);
                let ct: bool = fa.ct_eq(fb).into();
                assert_eq!(ct, fa == fb, "ct_eq disagrees with == for {a} vs {b}");
                // Also exercise the underlying u64 CT primitive directly.
                let raw: bool = fa.value().ct_eq(&fb.value()).into();
                assert_eq!(raw, fa == fb);
            }
        }
    }

    // sq-u8a8: a compile-and-behaviour check that `Fp` implements
    // `Zeroize` and that scrubbing leaves the additive identity (so a secret-
    // bearing `Fp` owned by a container can be wiped).
    #[test]
    fn fp_zeroizes_to_zero() {
        use zeroize::Zeroize;
        let mut x = Fp::new(0x1234_5678_9abc);
        x.zeroize();
        assert_eq!(x, Fp::zero(), "zeroized Fp must be the additive identity");
    }

    // sq-7ltf: the constant-time (branchless, fixed-iteration)
    // exponentiation `pow_ct` must compute the SAME field element as the
    // variable-time square-and-multiply `pow_vartime` for a spread of bases and
    // exponents (including the inversion exponent `P-2`, zero, one, and large
    // exponents that exercise the high bits). This proves the CT rewrite changes
    // no arithmetic — the latent-gap fix is behaviour-preserving.
    #[test]
    fn pow_ct_agrees_with_pow_vartime() {
        let bases = [0u64, 1, 2, 3, 7, 12345, P / 2, P - 2, P - 1];
        let exps = [0u64, 1, 2, 3, 8, 63, 64, 1000, P - 2, P - 1, u64::MAX];
        for &b in &bases {
            for &e in &exps {
                let fb = Fp::new(b);
                assert_eq!(
                    fb.pow_ct(e),
                    fb.pow_vartime(e),
                    "pow_ct vs pow_vartime disagree for base={b} exp={e}"
                );
            }
        }
    }

    // sq-7ltf: `inv` (which now routes through `pow_ct`) still produces
    // a true multiplicative inverse for boundary and random-ish values.
    #[test]
    fn inv_via_pow_ct_is_multiplicative_inverse() {
        for v in [1u64, 2, 3, 7, 12345, P / 2, P - 2, P - 1] {
            let a = Fp::new(v);
            assert_eq!(a.mul(a.inv()), Fp::one(), "inv (via pow_ct) of {v}");
        }
    }

    // sq-mnv5: the `p ≡ 3 (mod 4)` square-root shortcut. For every
    // quadratic RESIDUE c = a², `c.sqrt_residue()` must square back to c (d² == c),
    // and the precondition p ≡ 3 (mod 4) — which the shortcut relies on — holds for
    // p = 2^61 − 1. The random-bit sub-protocol only ever calls this on c = a².
    #[test]
    fn sqrt_residue_squares_back_for_residues() {
        // Precondition the shortcut rests on.
        assert_eq!(P % 4, 3, "the (p+1)/4 sqrt shortcut needs p ≡ 3 (mod 4)");
        for a in [1u64, 2, 3, 7, 12345, 1 << 30, P / 3, P / 2, P - 1] {
            let fa = Fp::new(a);
            let c = fa.mul(fa); // a residue by construction
            let d = c.sqrt_residue();
            assert_eq!(
                d.mul(d),
                c,
                "sqrt_residue(a²) must square back to a² for a={a}"
            );
            // The recovered root is ±a (the two square roots of a²).
            assert!(
                d == fa || d == fa.neg(),
                "sqrt_residue(a²) must be ±a for a={a} (got {})",
                d.value()
            );
        }
        // Zero is its own (trivial) root.
        assert_eq!(Fp::zero().sqrt_residue(), Fp::zero());
    }

    #[test]
    fn reduce_matches_reference_modulo() {
        // Cross-check the Mersenne fold against a plain u128 % P for a spread of
        // products — the fold must be exact, not approximate.
        for &(x, y) in &[(P - 1, P - 1), (1234567u64, 7654321u64), (P / 2, P / 2 + 1)] {
            let prod = (x as u128) * (y as u128);
            assert_eq!(reduce_mersenne(prod) as u128, prod % (P as u128));
        }
    }
}
