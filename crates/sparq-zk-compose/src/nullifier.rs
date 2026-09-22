// [OPUS-4.8] sq-rsd3v.1: host-side wiring for the single-use NULLIFIER — witness +
// Prover.toml for the `nullifier` circuit member, plus the verifier-side
// double-spend seen-set gate. Re-review when Fable returns.
//! Host-side witness + `Prover.toml` machinery for the in-circuit **single-use
//! nullifier** (`sq-rsd3v.1`, design `research/zk-inference-and-credentials.md`
//! §6.3), plus the verifier-side per-epoch double-spend gate. The in-circuit gadget
//! is `sparq_zk_compose_core::nullifier::nullifier`; the composed possession+nf
//! relation it participates in is exercised in-circuit by `compose_core`'s
//! `tests.nr` (`nullifier_member_accepts_bound_holder`). This module is its Rust
//! mirror, the analogue of [`crate::holder`] for the holder-PoK member.
//!
//! The composed relation proves, in zero knowledge, knowledge of a holder secret
//! `hsk` that (1) is DL-bound to an issuer-attested credential (holder possession,
//! `holder_pok`) AND (2) hashes to the PUBLIC nullifier
//! `nf = Poseidon2([ZKSIG_NF, hsk, epoch])` ([`sparq_zk::sig::nullifier`]). The
//! verifier then enforces DOUBLE-SPEND by recording each accepted `(epoch, nf)` in a
//! single-use seen-set, fail-closed on collision (§6.3(d)).
//!
//! # The DEPLOYABLE member is deferred (honest scope)
//! This ships the PRIMITIVE: the gadget, the tag, the host witness/`Prover.toml`
//! renderer, and the verifier seen-set gate. The compiled, PROVABLE bin member
//! (a `zk/compose/nullifier` package with its own measured VK + gate-count
//! baseline) and its `CircuitId` / `ProofManifest` verify-pipeline wiring are the
//! larger `sq-wvne` follow-up — which, per the gate-count discipline, also re-opens
//! the soundness audit and needs the nargo/bb toolchain to baseline. The
//! [`nullifier_prover_toml`] renderer below fixes that member's intended
//! public/private input order now, so the follow-up is a wiring step, not a
//! redesign.
//!
//! # Single source of truth (SECURITY-critical)
//! [`NullifierWitness::nf`] is [`sparq_zk::sig::nullifier`] over the SAME base-field
//! `hsk` the [`HolderPokWitness`] carries, and the in-circuit
//! `sparq_zk_compose_core::nullifier::nullifier` recomputes the identical
//! `Poseidon2([ZKSIG_NF, hsk, epoch])`. So the host `nf`, the circuit `nf`, and the
//! value the seen-set records are bit-identical by construction (pinned by the
//! `nullifier_cross_vector` Noir test).
//!
//! # Scope + honesty (NOT-yet-sound, `sq-qhy4`; §6.3)
//! - **Granularity is per-holder-per-EPOCH, NOT per-presentation.** `nf` binds
//!   `hsk` + `epoch` but NOT the credential/commitment: one holder key reused across
//!   distinct credentials in one epoch COLLIDES on `nf` (a rate-limit, usable as a
//!   feature). A per-presentation single-use token folds the commitment in — a
//!   SEPARATE, larger obligation, deliberately not built here.
//! - **Soundness rides TWO NOT-yet-sound legs:** the holder-PoK DL-binding of `hsk`
//!   (itself NOT-yet-sound, `sq-qhy4`) AND Poseidon2 collision resistance over
//!   `(ZKSIG_NF, hsk, epoch)`. This module asserts NO soundness property as
//!   achieved; the whole ZK estate is internally re-audited but NOT externally
//!   audited (`sq-qhy4`).
//! - This is the PRIMITIVE only (gadget + tag + seen-set gate). Folding it into the
//!   full `ProofManifest` verify pipeline (an unlinkable presentation) is the larger
//!   `sq-wvne` obligation and is deliberately out of scope here.

use sparq_zk::field::{field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk::sig::{nullifier as sig_nullifier, SecretKey};

use crate::holder::{holder_pok_witness, HolderPokWitness};
use crate::verifier::{SeenNonces, VerifierNonce};

/// The complete witness for one single-use nullifier proof: the holder PoK witness
/// ([`HolderPokWitness`] — `hsk`, `hpk` coords, and the issuer-attested
/// `holder_pk_digest`) plus the epoch and the derived nullifier `nf`. Mirrors the
/// inputs the `nullifier` member proves over: `hsk`, `hpk_x`, `hpk_y` are PRIVATE;
/// `holder_pk_digest`, `epoch`, `nf` are PUBLIC.
#[derive(Debug, Clone)]
pub struct NullifierWitness {
    /// The holder PoK witness (`hsk`, `hpk_x`, `hpk_y`, `holder_pk_digest`). `hsk`
    /// and `hpk` are PRIVATE in-circuit; the nullifier binds the SAME `hsk`.
    pub pok: HolderPokWitness,
    /// The verifier-published rate-limit window (a field element). PUBLIC.
    pub epoch: Fr,
    /// The nullifier `nf = Poseidon2([ZKSIG_NF, hsk, epoch])`
    /// ([`sparq_zk::sig::nullifier`]) — PUBLIC, and the value the double-spend
    /// seen-set records.
    pub nf: Fr,
}

/// Build the [`NullifierWitness`] for a holder key pair `hsk` and an `epoch`. The
/// nullifier is computed from the SAME base-field `hsk` embedding the holder-PoK
/// witness carries (single source of truth with the in-circuit gadget). Returns
/// `None` if the derived holder public key is the identity (no affine coordinates —
/// which the in-circuit `holder_pok` rejects anyway, fail-closed).
pub fn nullifier_witness(hsk: &SecretKey, epoch: &Fr) -> Option<NullifierWitness> {
    let pok = holder_pok_witness(hsk)?;
    // nf over the SAME base-field hsk the holder-PoK witness (and circuit) uses.
    let nf = sig_nullifier(&pok.hsk, epoch);
    Some(NullifierWitness { pok, epoch: *epoch, nf })
}

/// Render the `Prover.toml` body for the (sq-wvne) `nullifier` member. Order fixes
/// the member's intended `main` signature:
/// PUBLIC: challenge, holder_pk_digest, epoch, nf;
/// PRIVATE: hsk, hpk_x, hpk_y.
///
/// `challenge` is the verifier's fresh nonce (the public-input field-0 convention
/// the whole circuit family shares). The proof discloses NEITHER the holder secret
/// NOR the key (`hsk`, `hpk_x`, `hpk_y` are private); the verifier learns only the
/// per-holder-per-epoch `nf`.
pub fn nullifier_prover_toml(challenge: &Fr, witness: &NullifierWitness) -> String {
    let w = &witness.pok;
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", field_to_hex(challenge)));
    s.push_str(&format!(
        "holder_pk_digest = \"{}\"\n",
        field_to_hex(&w.holder_pk_digest)
    ));
    s.push_str(&format!("epoch = \"{}\"\n", field_to_hex(&witness.epoch)));
    s.push_str(&format!("nf = \"{}\"\n", field_to_hex(&witness.nf)));
    s.push_str(&format!("hsk = \"{}\"\n", field_to_hex(&w.hsk)));
    s.push_str(&format!("hpk_x = \"{}\"\n", field_to_hex(&w.hpk_x)));
    s.push_str(&format!("hpk_y = \"{}\"\n", field_to_hex(&w.hpk_y)));
    s
}

// --- verifier-side double-spend seen-set (§6.3(d)) -------------------------
//
// Double-spend enforcement is a verifier-side single-use seen-set of accepted
// `(epoch, nf)` pairs, fail-closed on collision. Rather than duplicate the
// audit-#4 durable-store machinery (flock/fsync append-only file), this REUSES the
// existing [`SeenNonces`] store via a domain-separated canonical key, so the
// nullifier gate inherits the SAME restart-surviving / atomic guarantees (and the
// same test-only vs. durable impl choice — [`crate::InMemorySeenNonces`] is
// NON-DURABLE, [`crate::FileSeenNonces`] is the production, restart-surviving one).

/// Domain tag for the double-spend STORE key (host-side bookkeeping only — NOT an
/// in-circuit value): `"ZKNFSEEN"`. Distinct from the nullifier hash tag
/// ([`sparq_zk::sig::SIG_DOMAIN_NULLIFIER`]) so the recorded key differs from `nf`
/// itself and cannot collide with a raw [`VerifierNonce`] a relying party might
/// record in a SHARED store (a dedicated store per purpose is still recommended).
const NF_SEEN_DOMAIN: u64 = 0x5a4b_4e46_5345_454e; // "ZKNFSEEN"

/// The canonical single-use STORE key for an `(epoch, nf)` pair:
/// `Poseidon2([ZKNFSEEN, epoch, nf])`, wrapped as a [`VerifierNonce`] so it plugs
/// into any [`SeenNonces`] store. `nf` already binds `epoch` (it is
/// `Poseidon2([ZKSIG_NF, hsk, epoch])`), so keying on the pair is belt-and-suspenders
/// that ALSO lets a verifier scope/prune the seen-set per epoch (§6.3(d)).
pub fn nullifier_seen_key(epoch: &Fr, nf: &Fr) -> VerifierNonce {
    let key = poseidon2::hash(&[Fr::from(NF_SEEN_DOMAIN), *epoch, *nf]);
    VerifierNonce::from_field(key)
}

/// The verifier-side DOUBLE-SPEND gate: extension of any [`SeenNonces`] store with a
/// per-epoch nullifier single-use check. A relying party calls
/// [`Self::record_fresh_nullifier`] for each accepted proof's `(epoch, nf)`; the
/// FIRST presentation of an `(epoch, nf)` returns `true` (fresh, accept), a REPLAY
/// of the same `(epoch, nf)` returns `false` (already spent — the verifier rejects,
/// fail-closed).
///
/// Blanket-implemented over every [`SeenNonces`], so a durable
/// [`crate::FileSeenNonces`] (restart-surviving) or the test-only
/// [`crate::InMemorySeenNonces`] both gain the gate. Durability is NOT optional in
/// production for the same reason it is not for audit-#4 nonces: an in-memory-only
/// seen-set forgets spent nullifiers on restart, re-admitting a replayed proof.
///
/// # Ordering (load-bearing)
/// A relying party MUST record `(epoch, nf)` ONLY AFTER the nullifier proof has
/// VERIFIED (once the sq-wvne member is wired), so `nf` is proof-bound to `epoch`
/// (`nf == Poseidon2([ZKSIG_NF, hsk, epoch])`, which makes `nf` determine `epoch`
/// for a fixed holder). Recording an unverified `(epoch, nf)` would let an attacker
/// poison the seen-set with an arbitrary pair. This gate is host bookkeeping over
/// already-verified public inputs; it does not itself verify the proof.
pub trait SeenNullifiers {
    /// Record `(epoch, nf)` as spent and return `true` iff it was FRESH (first use).
    /// Returns `false` on a replay (already spent) — the verifier then rejects the
    /// presentation. Atomic check-and-insert (inherited from the backing
    /// [`SeenNonces::record_fresh`]) so concurrent verifiers cannot both accept the
    /// same nullifier as fresh.
    fn record_fresh_nullifier(&self, epoch: &Fr, nf: &Fr) -> bool;
}

impl<T: SeenNonces + ?Sized> SeenNullifiers for T {
    fn record_fresh_nullifier(&self, epoch: &Fr, nf: &Fr) -> bool {
        self.record_fresh(&nullifier_seen_key(epoch, nf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemorySeenNonces;
    use sparq_zk::sig::holder_key_digest;

    // The witness's nf is EXACTLY sparq_zk::sig::nullifier over the holder-PoK
    // witness's base-field hsk, and the pok carries the real holder_key_digest — the
    // single-source-of-truth the in-circuit equality + the seen-set both rest on.
    #[test]
    fn nullifier_witness_nf_matches_host_primitive() {
        let hsk = SecretKey::from_seed(102);
        let epoch = Fr::from(7u64);
        let w = nullifier_witness(&hsk, &epoch).expect("non-identity holder key has a witness");
        assert_eq!(
            w.nf,
            sig_nullifier(&w.pok.hsk, &epoch),
            "witness nf is nullifier(hsk_base, epoch)"
        );
        assert_eq!(
            w.pok.holder_pk_digest,
            holder_key_digest(&hsk.public_key()).unwrap(),
            "witness carries the real holder_key_digest (possession binding)"
        );
        assert_eq!(w.epoch, epoch);
    }

    // Granularity §6.3(c): the SAME holder key in the SAME epoch gives the SAME nf
    // (collides — per-holder-per-epoch), regardless of which credential; DIFFERENT
    // epochs give DIFFERENT nf; DIFFERENT holders give DIFFERENT nf.
    #[test]
    fn nullifier_granularity_is_per_holder_per_epoch() {
        let epoch7 = Fr::from(7u64);
        let epoch8 = Fr::from(8u64);
        let a7 = nullifier_witness(&SecretKey::from_seed(102), &epoch7).unwrap().nf;
        let a7_again = nullifier_witness(&SecretKey::from_seed(102), &epoch7).unwrap().nf;
        let a8 = nullifier_witness(&SecretKey::from_seed(102), &epoch8).unwrap().nf;
        let b7 = nullifier_witness(&SecretKey::from_seed(200), &epoch7).unwrap().nf;
        assert_eq!(a7, a7_again, "same holder + same epoch => same nf (rate-limit)");
        assert_ne!(a7, a8, "same holder, different epoch => different nf");
        assert_ne!(a7, b7, "different holder, same epoch => different nf");
    }

    // FORGE-AND-VERIFY REGRESSION (§6.3(d)): the double-spend gate accepts a
    // nullifier's FIRST presentation in an epoch and REJECTS a replay of the same nf
    // in the same epoch, fail-closed. The same nf under a LATER epoch is a different
    // (epoch, nf) key and is admitted (a fresh rate-limit window).
    #[test]
    fn double_spend_replay_same_epoch_fails_closed() {
        let store = InMemorySeenNonces::new();
        let epoch = Fr::from(7u64);
        let w = nullifier_witness(&SecretKey::from_seed(102), &epoch).unwrap();

        // First presentation: fresh, accepted.
        assert!(
            store.record_fresh_nullifier(&w.epoch, &w.nf),
            "first presentation of (epoch, nf) is fresh"
        );
        // REPLAY of the SAME nf in the SAME epoch: rejected (already spent).
        assert!(
            !store.record_fresh_nullifier(&w.epoch, &w.nf),
            "replay of the same nf in the same epoch must fail-closed"
        );
        // A distinct holder in the same epoch is still admitted (independent nf).
        let w2 = nullifier_witness(&SecretKey::from_seed(200), &epoch).unwrap();
        assert!(
            store.record_fresh_nullifier(&w2.epoch, &w2.nf),
            "a different holder's nullifier is independent"
        );
        // A NEW epoch opens a fresh window for the original holder.
        let next = Fr::from(8u64);
        let w_next = nullifier_witness(&SecretKey::from_seed(102), &next).unwrap();
        assert!(
            store.record_fresh_nullifier(&w_next.epoch, &w_next.nf),
            "the same holder in a new epoch is admitted (fresh window)"
        );
    }

    // The seen-key is a deterministic, domain-separated function of (epoch, nf), and
    // distinct pairs give distinct store keys (so no two genuine presentations alias).
    #[test]
    fn seen_key_is_deterministic_and_pair_distinguishing() {
        let e = Fr::from(7u64);
        let nf1 = sig_nullifier(&Fr::from(11u64), &e);
        let nf2 = sig_nullifier(&Fr::from(12u64), &e);
        assert_eq!(
            nullifier_seen_key(&e, &nf1).as_field_hex(),
            nullifier_seen_key(&e, &nf1).as_field_hex(),
            "seen-key is deterministic"
        );
        assert_ne!(
            nullifier_seen_key(&e, &nf1).as_field_hex(),
            nullifier_seen_key(&e, &nf2).as_field_hex(),
            "distinct nf => distinct store key"
        );
        assert_ne!(
            nullifier_seen_key(&Fr::from(7u64), &nf1).as_field_hex(),
            nullifier_seen_key(&Fr::from(8u64), &nf1).as_field_hex(),
            "distinct epoch => distinct store key"
        );
    }

    // The Prover.toml renders all seven fields in main's declaration order
    // (PUBLIC challenge, holder_pk_digest, epoch, nf; PRIVATE hsk, hpk_x, hpk_y).
    #[test]
    fn nullifier_prover_toml_renders_all_fields_in_order() {
        let w = nullifier_witness(&SecretKey::from_seed(303), &Fr::from(5u64)).unwrap();
        let toml = nullifier_prover_toml(&Fr::from(0x2au64), &w);
        for field in ["challenge", "holder_pk_digest", "epoch", "nf", "hsk", "hpk_x", "hpk_y"] {
            assert!(toml.contains(field), "toml must render {field}");
        }
        let challenge_pos = toml.find("challenge").unwrap();
        let digest_pos = toml.find("holder_pk_digest").unwrap();
        let epoch_pos = toml.find("epoch").unwrap();
        let nf_pos = toml.find("nf =").unwrap();
        let hsk_pos = toml.find("hsk").unwrap();
        assert!(challenge_pos < digest_pos && digest_pos < epoch_pos);
        assert!(epoch_pos < nf_pos && nf_pos < hsk_pos);
    }

    // CROSS-VECTOR PIN (SECURITY-critical): the seed-102 holder, epoch=7 nullifier
    // matches the exact hex the Noir `nullifier_cross_vector` test asserts. If the
    // host nullifier or the hsk embedding ever drifts from the in-circuit gadget,
    // this pin (and its Noir twin) fails — the in-circuit `nf == public nf` equality
    // and the seen-set both rest on them agreeing bit-for-bit.
    #[test]
    fn nullifier_matches_noir_cross_vector() {
        let w = nullifier_witness(&SecretKey::from_seed(102), &Fr::from(7u64)).unwrap();
        // hsk pinned identical to holder.rs / tests.nr (seed-102 HSK).
        assert_eq!(
            field_to_hex(&w.pok.hsk),
            "0x04c49ec34f100efeb528ac3d436a6e1a2cb6b0c85fab6a485462c74c12a82d15"
        );
        assert_eq!(
            field_to_hex(&w.nf),
            "0x27113b53c9dd70eaf8705b017290442911e46676758fd901c1446286940c7d7e"
        );
    }
}
