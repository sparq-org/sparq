// [OPUS-4.8] sq-rsd3v.1: the SINGLE-USE NULLIFIER primitive -- the one
// genuinely-absent ZK gadget in the estate. Re-review when Fable returns.
//! In-circuit **single-use nullifier** (design
//! `research/zk-inference-and-credentials.md` §6.3): compute
//! `nf = Poseidon2([ZKSIG_NF, hsk, epoch])` IN-CIRCUIT from the WITNESSED holder
//! secret `hsk` and assert it equals the PUBLIC `nf`. This is NEW in-circuit
//! soundness surface -- NOT the shipped audit-#4 nonce binding, which is purely
//! verifier-side (`let _ = challenge;` + a public-input byte-compare). Here the
//! nullifier is a computed CONSTRAINT over a private witness.
//!
//! # Why this is bound to holder possession (soundness dep, §6.3(b))
//! `nullifier` alone (the raw hash) would let a prover witness ANY `hsk`; the value
//! is only meaningful when the SAME `hsk` is DL-bound to a credential. So the
//! composed relation puts `holder_pok` (`hpk = hsk*G`,
//! `holder_key_digest(hpk) == holder_pk_digest`) TOGETHER with the nf equality over
//! the SAME private `hsk` (exercised in `tests.nr`'s
//! `nullifier_member_accepts_bound_holder`; the deployable member is the sq-wvne
//! follow-up). Unforgeability then rides BOTH the holder-PoK DL-binding (itself
//! NOT-yet-sound, `sq-qhy4`) AND Poseidon2 domain-separated collision resistance
//! over `(ZKSIG_NF, hsk, epoch)`.
//!
//! # Granularity -- per-holder-per-epoch, NOT per-presentation (§6.3(c))
//! `nf` binds `hsk` and `epoch` but NOT the credential/commitment: one holder key
//! reused across DISTINCT credentials in the SAME epoch COLLIDES on `nf`. That is a
//! per-holder-per-epoch RATE-LIMIT, not a per-presentation single-use token (which
//! would additionally fold the commitment -- a separate, larger obligation).
//!
//! # Bit-compatibility (SECURITY-critical)
//! `SIG_DOMAIN_NULLIFIER` MUST equal `sparq_zk::sig::SIG_DOMAIN_NULLIFIER`
//! ("ZKSIG_NF" = 0x5a4b5349475f4e46) and the hash MUST match the host
//! `sparq_zk::sig::nullifier`, so the in-circuit `nf` is bit-identical to the
//! host value the verifier's double-spend seen-set records. Pinned by the
//! `nullifier_cross_vector` test in `tests.nr` (generated from a REAL host
//! nullifier).

use poseidon::poseidon2::Poseidon2;

// Domain separator for the nullifier -- MUST equal
// `sparq_zk::sig::SIG_DOMAIN_NULLIFIER` ("ZKSIG_NF" = 0x5a4b5349475f4e46), distinct
// from `holder::SIG_DOMAIN_HOLDER_KEY` ("ZKSIG_HK") so a nullifier can never be
// confused with the holder-key digest computed over the same holder.
pub global SIG_DOMAIN_NULLIFIER: Field = 0x5a4b5349475f4e46;

/// The single-use nullifier, bit-identical to `sparq_zk::sig::nullifier`:
/// `Poseidon2([ZKSIG_NF, hsk, epoch], 3)`. `hsk` is the holder secret embedded in
/// the base field (the SAME private witness `holder_pok` proves possession of);
/// `epoch` is the verifier-published rate-limit window.
pub fn nullifier(hsk: Field, epoch: Field) -> Field {
    Poseidon2::hash([SIG_DOMAIN_NULLIFIER, hsk, epoch], 3)
}
