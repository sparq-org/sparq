//! The commitment scheme's hash layer -- MUST stay bit-compatible with
//! `crates/sparq-zk` (`src/encode.rs` + `src/commit.rs`):
//!
//! - `h2(a, b)`          = `Poseidon2::hash([a, b], 2)` -- term encoding layer
//!   (`Enc_t(term) = h2(type_code, h_s(value))`).
//! - `h3(s, p, o)`       = `Poseidon2::hash([s, p, o], 3)` -- triple leaf hash.
//! - `commit_fold`       = `Poseidon2::hash(leaves, count)` -- the flat
//!   per-graph commitment (plan S2.2 shape 1). The library's length-bearing
//!   IV (`count * 2^64`) gives domain separation per leaf count, and a
//!   runtime `count < N` leaves padding slots outside the absorbed message.
//!
//! Cost note (noir-optimisation S5): all of these lower to the
//! `poseidon2_permutation` blackbox -- h2/h3 are one permutation each
//! (~74 gates), `commit_fold::<N>` pays ceil(N/3)+1 permutations regardless
//! of the runtime `count` (ACIR loops unroll over the comptime bound).

use poseidon::poseidon2::Poseidon2;

/// Term-encoding layer hash (`Enc_t = h2(type_code, h_s)`).
pub fn h2(a: Field, b: Field) -> Field {
    Poseidon2::hash([a, b], 2)
}

/// Triple leaf hash (`leaf = h3(Enc(s), Enc(p), Enc(o))`).
pub fn h3(s: Field, p: Field, o: Field) -> Field {
    Poseidon2::hash([s, p, o], 3)
}

/// Four-input hash -- the arity the sq-kndw fully-hidden revocation relation
/// needs. Mirrors `sparq_zk::poseidon2::hash(&[a, b, c, d])`, which is what
/// `sig::status_ref_commitment` and `sig::accepted_status_leaf` fold
/// (`[DOMAIN, x, y, z]`). One permutation, same cost class as `h2`/`h3`.
// [OPUS-5] sq-kndw: 4-input arity for the ref-commitment / accepted-set leaf.
pub fn h4(a: Field, b: Field, c: Field, d: Field) -> Field {
    Poseidon2::hash([a, b, c, d], 4)
}

/// Five-input hash -- the arity the METHOD-BOUND presentation attestation
/// message needs. Mirrors `sparq_zk::poseidon2::hash(&[a, b, c, d, e])`, which
/// is what `sig::presentation_message_with_status_and_scheme` folds
/// (`[ZKSIG_M5, C(G), salt, status_ref, scheme_digest]`). One permutation, same
/// cost class as `h2`/`h3`/`h4`.
pub fn h5(a: Field, b: Field, c: Field, d: Field, e: Field) -> Field {
    Poseidon2::hash([a, b, c, d, e], 5)
}

/// Flat per-graph commitment fold over the first `count` leaves.
pub fn commit_fold<let N: u32>(leaves: [Field; N], count: u32) -> Field {
    Poseidon2::hash(leaves, count)
}

/// Term type codes -- mirror `sparq_zk::encode::TYPE_CODE_*`.
pub global TYPE_CODE_IRI: Field = 1;
pub global TYPE_CODE_LITERAL: Field = 2;
pub global TYPE_CODE_BLANK_NODE: Field = 3;
