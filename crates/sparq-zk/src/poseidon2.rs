//! Poseidon2 over BN254, bit-compatible with Noir.
//!
//! Two layers, each a faithful port of the reference the circuits will use:
//!
//! 1. **Permutation** (`permutation`): ported from noir-lang/noir
//!    `acvm-repo/bn254_blackbox_solver/src/poseidon2.rs` — the solver behind
//!    Noir's `std::hash::poseidon2_permutation` blackbox (itself matching the
//!    Barretenberg crypto module). t = 4, R_F = 8, R_P = 56, x^5 S-box;
//!    constants vendored in `crate::poseidon2_constants`.
//! 2. **Sponge** (`hash`): ported from noir-lang/poseidon `src/poseidon2.nr`
//!    (`Poseidon2::hash`): rate 3, capacity initialised to
//!    `len * 2^64`, absorb in chunks of 3, squeeze one element.
//!
//! Compatibility is PROVEN, not assumed: `tests/poseidon2_noir_cross.rs`
//! checks these functions against vectors printed by `nargo` from a Noir
//! program calling `poseidon::poseidon2::Poseidon2::hash` (fixture under
//! `tests/fixtures/noir_poseidon2/`), plus vectors published in the
//! noir-lang/poseidon repository tests.

use crate::field::{field_from_hex, Fr};
use crate::poseidon2_constants::{INTERNAL_MATRIX_DIAGONAL_HEX, ROUND_CONSTANT_HEX};
use ark_ff::Field as _;
use std::sync::LazyLock;

const T: usize = 4;
const ROUNDS_F: usize = 8;
const ROUNDS_P: usize = 56;
const RATE: usize = 3;

static INTERNAL_MATRIX_DIAGONAL: LazyLock<[Fr; 4]> = LazyLock::new(|| {
    INTERNAL_MATRIX_DIAGONAL_HEX.map(field_from_hex)
});

static ROUND_CONSTANT: LazyLock<[[Fr; 4]; 64]> = LazyLock::new(|| {
    ROUND_CONSTANT_HEX.map(|round| round.map(field_from_hex))
});

#[inline]
fn single_box(x: Fr) -> Fr {
    let s = x.square();
    s.square() * x
}

#[inline]
fn s_box(state: &mut [Fr; T]) {
    for x in state.iter_mut() {
        *x = single_box(*x);
    }
}

#[inline]
fn add_round_constants(state: &mut [Fr; T], round: usize) {
    let rc = &ROUND_CONSTANT[round];
    for (s, c) in state.iter_mut().zip(rc.iter()) {
        *s += c;
    }
}

/// External-round 4x4 MDS multiplication — algorithm taken verbatim from the
/// Barretenberg/noir implementation (circulant `[5,7,1,3]` trick).
#[inline]
fn matrix_multiplication_4x4(input: &mut [Fr; T]) {
    let t0 = input[0] + input[1]; // A + B
    let t1 = input[2] + input[3]; // C + D
    let mut t2 = input[1] + input[1]; // 2B
    t2 += t1; // 2B + C + D
    let mut t3 = input[3] + input[3]; // 2D
    t3 += t0; // 2D + A + B
    let mut t4 = t1 + t1;
    t4 += t4;
    t4 += t3; // A + B + 4C + 6D
    let mut t5 = t0 + t0;
    t5 += t5;
    t5 += t2; // 4A + 6B + C + D
    let t6 = t3 + t5; // 5A + 7B + C + 3D
    let t7 = t2 + t4; // A + 3B + 5C + 7D
    input[0] = t6;
    input[1] = t5;
    input[2] = t7;
    input[3] = t4;
}

#[inline]
fn internal_m_multiplication(input: &mut [Fr; T]) {
    let mut sum = Fr::from(0u64);
    for i in input.iter() {
        sum += i;
    }
    for (i, x) in input.iter_mut().enumerate() {
        *x *= INTERNAL_MATRIX_DIAGONAL[i];
        *x += sum;
    }
}

/// The Poseidon2-BN254 permutation (t = 4), identical to Noir's
/// `poseidon2_permutation` blackbox.
pub fn permutation(inputs: &[Fr; T]) -> [Fr; T] {
    let mut state = *inputs;
    permute_in_place(&mut state);
    state
}

/// [`permutation`] applied to `state` itself, so a caller's guarded state is the only
/// copy the rounds work on.
fn permute_in_place(state: &mut [Fr; T]) {
    // 1st linear layer.
    matrix_multiplication_4x4(state);

    // First half of the external rounds.
    let rf_first = ROUNDS_F / 2;
    for r in 0..rf_first {
        add_round_constants(state, r);
        s_box(state);
        matrix_multiplication_4x4(state);
    }

    // Internal rounds (S-box on lane 0 only).
    let p_end = rf_first + ROUNDS_P;
    for r in rf_first..p_end {
        state[0] += ROUND_CONSTANT[r][0];
        state[0] = single_box(state[0]);
        internal_m_multiplication(state);
    }

    // Remaining external rounds.
    for r in p_end..(ROUNDS_F + ROUNDS_P) {
        add_round_constants(state, r);
        s_box(state);
        matrix_multiplication_4x4(state);
    }
}

/// Variable-length Poseidon2 hash, identical to noir-lang/poseidon's
/// `Poseidon2::hash(input, input.len())`: sponge with rate 3 and IV
/// `len * 2^64` in the capacity lane.
pub fn hash(inputs: &[Fr]) -> Fr {
    sponge(inputs, &mut [Fr::from(0u64); T])
}

/// [`hash`] for secret inputs: the permutation runs in place on a sponge state that is
/// zeroized when it is dropped. Compiler and field-arithmetic temporaries are not scrubbed.
pub(crate) fn hash_secret(inputs: &[Fr]) -> Fr {
    sponge(inputs, &mut zeroize::Zeroizing::new([Fr::from(0u64); T]))
}

fn sponge(inputs: &[Fr], state: &mut [Fr; T]) -> Fr {
    let two_pow_64 = Fr::from(1u128 << 64);
    let iv = Fr::from(inputs.len() as u64) * two_pow_64;
    state[RATE] = iv;

    let mut chunks = inputs.chunks_exact(RATE);
    for chunk in chunks.by_ref() {
        for (i, x) in chunk.iter().enumerate() {
            state[i] += x;
        }
        permute_in_place(state);
    }
    let rem = chunks.remainder();
    // A final permutation runs iff the input is empty or has a partial
    // trailing chunk (mirrors `(in_len == 0) | (in_len % RATE != 0)`).
    if !rem.is_empty() || inputs.is_empty() {
        for (i, x) in rem.iter().enumerate() {
            state[i] += x;
        }
        permute_in_place(state);
    }
    state[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::field_from_hex_str;

    /// Zero-input permutation vector from noir-lang/noir
    /// `bn254_blackbox_solver/src/poseidon2.rs` (`smoke_test`).
    #[test]
    fn permutation_zero_vector_matches_noir_smoke_test() {
        let out = permutation(&[Fr::from(0u64); 4]);
        let expected = [
            "0x18dfb8dc9b82229cff974efefc8df78b1ce96d9d844236b496785c698bc6732e",
            "0x095c230d1d37a246e8d2d5a63b165fe0fade040d442f61e25f0590e5fb76f839",
            "0x0bb9545846e1afa4fa3c97414a60a20fc4949f537a68cceca34c5ce71e28aa59",
            "0x18a4f34c9c6f99335ff7638b82aeed9018026618358873c982bbdde265b2ed6d",
        ]
        .map(|h| field_from_hex_str(h).unwrap());
        assert_eq!(out, expected);
    }

    #[test]
    fn hash_is_deterministic_and_length_separated() {
        let a = hash(&[Fr::from(1u64), Fr::from(2u64)]);
        let b = hash(&[Fr::from(1u64), Fr::from(2u64)]);
        let c = hash(&[Fr::from(1u64), Fr::from(2u64), Fr::from(0u64)]);
        assert_eq!(a, b);
        assert_ne!(a, c, "length must enter the IV");
    }

    // [OPUS-4.8] sq-qcnn.25: the empty-input branch. An empty input still runs the
    // final permutation (`(in_len == 0) | (in_len % RATE != 0)`), so the digest is
    // the permuted state[0], NOT the pre-permutation zero. Deterministic + distinct
    // from a single explicit zero element (which carries a different length IV).
    #[test]
    fn hash_empty_input_runs_final_permutation() {
        let e = hash(&[]);
        assert_eq!(e, hash(&[]), "empty-input hash is deterministic");
        assert_ne!(e, Fr::from(0u64), "empty input runs the final permutation, not identity");
        assert_ne!(e, hash(&[Fr::from(0u64)]), "length enters the IV (0 vs 1 element)");
    }
}
