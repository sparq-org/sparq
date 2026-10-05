// [OPUS-5] sq-j3b9 (follow-up to sq-8jv7, gap CR-G5) — mark for crypto re-review.
//! Scalar-independent Baby-JubJub scalar multiplication for the SECRET-scalar
//! paths of [`crate::sig`] (issuer/holder key derivation and Schnorr signing).
//!
//! # The gap this NARROWS — not closes (sq-j3b9, deferred from sq-8jv7)
//! `sq-8jv7` made the one secret-dependent branch sparq *owns* on the sign path
//! branchless (`derive_nonce`'s degenerate-`k` guard) and then documented an
//! irreducible residual: the scalar multiplication itself was `arkworks`'
//! generic `ark_ec::PrimeGroup::mul_bigint`, whose textbook double-and-add
//!
//! ```text
//! for b in BitIteratorBE::without_leading_zeros(scalar) {
//!     res.double_in_place();
//!     if b { res += self; }        // <-- branch on a SECRET scalar bit
//! }
//! ```
//!
//! leaks the secret scalar twice over: `without_leading_zeros` makes the loop
//! *trip count* a function of the scalar's bit length, and the `if b` makes the
//! per-iteration work a function of each individual scalar bit. That is the
//! textbook square-and-multiply side channel: the most *structurally* explicit
//! secret dependence on the signing path, in that the secret's bits map directly
//! onto which instructions execute. (That it is the strongest signal in practice
//! is an unmeasured expectation, not a measured result — see the honesty note.)
//!
//! [`mul_ct`] replaces it with a fixed-shape double-and-ALWAYS-add ladder:
//!
//! * the loop always runs exactly `PrimeField::MODULUS_BIT_SIZE` iterations,
//!   independent of the scalar's value (no leading-zero short-circuit), and
//! * each iteration performs the SAME operations regardless of the bit — one
//!   doubling, one group addition, and a branchless arithmetic select of the
//!   addend between the base point and the curve identity.
//!
//! The select is pure field arithmetic, not a branch and not a table index:
//! for a bit `b ∈ {0, 1}` lifted into the base field, the addend is
//! `(b·x, 1 + b·(y − 1))`, which is `(x, y)` when `b = 1` and the twisted-Edwards
//! affine identity `(0, 1)` when `b = 0`. Adding the identity is *correct*, not
//! merely harmless, because Baby-JubJub's addition law is complete (`a` square,
//! `d` non-square) and `ark-ec` implements the unified `E^e` addition — the same
//! property `mul_bigint` already relies on for its own accumulator.
//!
//! # HONESTY — what this does and does NOT claim [OPUS-5]
//! **This is NOT a claim that signing is constant-time.** It removes the
//! secret-scalar-dependent *control flow and loop shape* from the code sparq
//! owns. Two residuals remain, and neither is closed here:
//!
//! 1. **`arkworks` field arithmetic is still not asserted constant-time.** Every
//!    operation above bottoms out in `ark-ff` `Fp` add/mul, whose Montgomery
//!    reduction ends in a conditional subtraction, and which sparq does not
//!    assert (and cannot assert without replacing the field implementation) is
//!    constant-time. Closing *that* is the curve/dep swap this bead scoped as
//!    the ceiling; sq-j3b9 deliberately takes the part that is achievable
//!    without a dependency change rather than claiming the whole.
//! 2. **The scalar arithmetic `s = k + e·sk` in [`crate::sig::sign_deterministic`]
//!    is untouched** — it is a fixed, data-independent sequence of two scalar-field
//!    operations (no loop, no branch), so it carries no square-and-multiply
//!    structure, but it inherits residual (1) exactly as before.
//!
//! No instrumented `dudect` / `ctgrind` measurement has been run; this is a
//! source-level shape argument, and a clean reading is not a timing-channel
//! proof. The instrumented pass folds into the external cryptographer audit
//! (CR-G1 / `sq-qhy4`), under which this whole crate remains **research-grade
//! and NOT externally audited**. See
//! `compliance/cryptoreview/side-channel-analysis.md`.
//!
//! # Why the *verify* path deliberately does not use this
//! [`crate::sig::verify`] operates entirely on PUBLIC data (the commitment, the
//! signature, the disclosed public key) — there is no secret on that path, so a
//! scalar-independent ladder would buy no security and would cost an extra
//! group addition per scalar bit on every verification. Verification keeps the
//! `arkworks` variable-time multiplication.

use crate::field::Fr;
use ark_ec::{twisted_edwards::Affine, AffineRepr, CurveGroup};
use ark_ed_on_bn254::{EdwardsConfig, EdwardsProjective, Fr as JjScalar};
use ark_ff::{BigInteger, PrimeField, Zero};

/// `point · scalar`, with an instruction and memory-access sequence that is
/// independent of `scalar` in the code sparq owns (see the module docs for the
/// precise, bounded claim — this is NOT an assertion that the operation is
/// constant-time end to end).
///
/// Value-identical to `point * scalar` for every scalar; the difference is
/// purely the shape of the computation. Use this wherever `scalar` is secret
/// (a signing nonce or a long-term secret key); use the plain `*` operator on
/// public data.
pub(crate) fn mul_ct(point: &EdwardsProjective, scalar: &JjScalar) -> EdwardsProjective {
    // The affine base coordinates, read once outside the ladder. `xy()` declines
    // only for the identity, whose affine twisted-Edwards representation is
    // exactly `(0, 1)` — so the fallback is the true value, not a sentinel.
    let base = point.into_affine();
    let (bx, by) = base.xy().unwrap_or((Fr::from(0u64), Fr::from(1u64)));
    let one = Fr::from(1u64);

    // A canonical little-endian byte view of the secret scalar. Held in
    // `Zeroizing` so this copy of the secret is scrubbed when the ladder ends
    // (sq-u8a8 / sq-19ej secret-memory hygiene), rather than left in the freed
    // allocation.
    let scalar_le = zeroize::Zeroizing::new(scalar.into_bigint().to_bytes_le());
    let bytes = scalar_le.as_slice();

    // FIXED trip count: the scalar field's modulus bit size, never the scalar's
    // own bit length. Every canonical scalar is `< MODULUS`, so bits at or above
    // this index are zero for every input and skipping them leaks nothing; what
    // matters is that the count does not VARY with the scalar.
    let bits = JjScalar::MODULUS_BIT_SIZE as usize;

    let mut acc = EdwardsProjective::zero();
    for i in (0..bits).rev() {
        // The bit index is a loop counter — public — so this indexing is not a
        // secret-dependent memory access.
        let bit = (bytes[i / 8] >> (i % 8)) & 1;
        // Branchless select: `b = 1` yields the base point, `b = 0` yields the
        // affine identity `(0, 1)`. Same operations either way.
        let b = Fr::from(u64::from(bit));
        let addend = Affine::<EdwardsConfig>::new_unchecked(b * bx, one + b * (by - one));
        // Doubling via the unified addition law (`ark-ec` implements the unified
        // extended-coordinates `E^e` addition, which is correct for `P + P` and
        // for the identity), so the ladder needs no doubling special case.
        let doubled = acc + acc;
        acc = doubled + addend.into_group();
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::PrimeGroup;
    use ark_ff::UniformRand;
    use ark_std::rand::SeedableRng;

    fn rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0x5a4b_5349_474a_3362)
    }

    /// The load-bearing obligation: the scalar-independent ladder computes
    /// EXACTLY the same group element as the `arkworks` variable-time
    /// multiplication it replaces, for every scalar. This is the result-
    /// equivalence differential — it goes red on any arithmetic drift (a wrong
    /// bit order, an off-by-one trip count, a mis-built addend), because the
    /// reference is the implementation the signing path used before sq-j3b9.
    #[test]
    fn ct_ladder_matches_arkworks_scalar_mul() {
        let g = EdwardsProjective::generator();
        // A second, non-generator base so the ladder is exercised on an
        // arbitrary point and not just on `G`.
        let h = g * JjScalar::from(7u64);

        let mut edge = vec![
            JjScalar::from(0u64),
            JjScalar::from(1u64),
            JjScalar::from(2u64),
            JjScalar::from(3u64),
            // `r - 1`: the maximal canonical scalar, so the TOP bit of the
            // fixed-width ladder is exercised (an under-sized trip count fails
            // here and nowhere else).
            -JjScalar::from(1u64),
            -JjScalar::from(2u64),
        ];
        let mut r = rng();
        for _ in 0..24 {
            edge.push(JjScalar::rand(&mut r));
        }

        for k in edge {
            for base in [g, h] {
                assert_eq!(
                    mul_ct(&base, &k),
                    base * k,
                    "constant-shape ladder must equal the arkworks reference for k = {k}"
                );
            }
        }
    }

    /// The ladder's trip count is a function of the MODULUS, never of the
    /// scalar: a one-bit scalar and a 251-bit scalar must both come out right.
    /// Pinning `MODULUS_BIT_SIZE` here also makes a future curve/dep swap that
    /// changed the scalar field fail loudly instead of silently truncating the
    /// top bits of every secret scalar.
    #[test]
    fn ladder_width_covers_every_canonical_scalar() {
        let bits = JjScalar::MODULUS_BIT_SIZE as usize;
        // Every canonical scalar is `< MODULUS`, so its little-endian byte
        // encoding has no set bit at or above `MODULUS_BIT_SIZE`. If that ever
        // stopped holding, the ladder would drop the high bits.
        let max = -JjScalar::from(1u64);
        let le = max.into_bigint().to_bytes_le();
        for i in bits..(le.len() * 8) {
            assert_eq!(
                (le[i / 8] >> (i % 8)) & 1,
                0,
                "bit {i} of the largest canonical scalar must be above the ladder width"
            );
        }
        // ...and the ladder is actually that wide: `2^(bits-1)` needs the top
        // iteration to survive.
        let mut top = JjScalar::from(1u64);
        for _ in 0..(bits - 1) {
            let doubled = top + top;
            top = doubled;
        }
        let g = EdwardsProjective::generator();
        assert_eq!(mul_ct(&g, &top), g * top, "top-bit scalar must round-trip");
    }

    /// The identity addend really is the twisted-Edwards identity: multiplying
    /// by zero yields the identity rather than a garbage point, which is the
    /// case a "select the base point unconditionally" bug would break.
    #[test]
    fn zero_scalar_yields_the_identity() {
        let g = EdwardsProjective::generator();
        assert!(mul_ct(&g, &JjScalar::from(0u64)).is_zero());
    }
}
