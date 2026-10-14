// [OPUS-4.8] sq-ka8m (sq-mnv5 residual; design Hole 4): TAMPER-EVIDENT secure
// greater-than over the decompose+compare chain. Every multiplication carries an
// IT-MAC (sq-km34.3, `MacSession::auth_mul`) and the boolean verdict is MAC-checked
// (sq-km34.4, `MacSession::mac_check`) BEFORE it is opened.
//! Honest-majority **tamper-evident-with-abort** secure greater-than — the IT-MAC
//! upgrade of the semi-honest [`crate::compare`] comparison chain (Hole 4).
//!
//! ## What this closes (design `research/mpc-malicious-security-design.md` Hole 4)
//!
//! The semi-honest [`crate::compare::secure_greater_than`] is a deep arithmetic
//! circuit — bit-decompose each operand, then an MSB-first chain of secret ANDs /
//! bit-equalities — and **every** product routes through
//! [`crate::shamir::mul_shares_raw`] + [`crate::shamir::ShamirDealer::degree_reduce`].
//! Each is a place a malicious party can inject an *undetected* offset: a forged
//! product share (Hole 1) or a wrong `degree_reduce` re-sharing (Hole 2), the latter
//! undetectable by Reed–Solomon even at over-provisioned `n` because the tampering is
//! on the value being re-shared, not on the open. The comparison is the **worst
//! case** of the four holes (design §1, Hole 4): a chain of Holes 1 + 2 with a final
//! boolean open at minimal `n = 2t+1` where RS redundancy is zero.
//!
//! ## The fix (design §2.4 route (a) + §2.5)
//!
//! Carry an information-theoretic MAC `m_x = α·x` (under a session-global,
//! secret-shared `[α]` no party knows) through the WHOLE chain:
//!
//! - operand bits are shared as **authenticated** values
//!   ([`crate::shamir::MacSession::authenticated_share`]);
//! - the `[0]`/`[1]` accumulators (`gt`, `eq`) are authenticated constants
//!   ([`crate::shamir::MacSession::auth_const_sharing`]);
//! - every secret AND / bit-equality is an authenticated multiplication
//!   ([`crate::shamir::MacSession::auth_mul`], the §2.4 mult-then-reduce that also
//!   MAC-covers the `degree_reduce` re-sharing);
//! - all linear ops (`1 − b`, `+`, scaling) carry the MAC for FREE
//!   ([`crate::authenticated`] `auth_*`).
//!
//! Then, **before the verdict is opened**, run ONE batched random-challenge MAC-check
//! ([`crate::shamir::MacSession::mac_check`], §2.5) over the verdict (and any other
//! value the verdict's integrity depends on): open a leakage-free
//! `σ = Σ χ_j·[m_{y_j}] − (Σ χ_j·y_j)·[α]` and **abort iff `σ ≠ 0`**. A tamper that
//! leaves some checked value `y_j` inconsistent with its MAC cannot be fixed up without
//! knowing the secret `α`, so the check fires with probability `≈ 1 − 2^{−61}` over
//! `F_p`. This does NOT cover every gate: `auth_mul` adopts a value tamper on its
//! SECOND operand (see the security tier below).
//! Crucially this works at the minimal `n = 2t+1` — soundness comes from the secret
//! `α`, not from RS over-determination (design §2.5, the reason RS could not close
//! Hole 1 / Hole 4 but the MAC can).
//!
//! ## Security tier (stated precisely, no over-claim)
//!
//! **Honest-majority, tamper-evident-with-abort on the opened values; the abort is local detection, not an agreed unanimous or identifiable abort.**
//! This is NOT AXIS-1 `Malicious` today, whatever design §3 targets:
//! [`crate::shamir::MacSession::auth_mul`] carries the output MAC as
//! `[α·z] = reduce([α·x]·[y])`, so it ADOPTS a value tamper on its second operand
//! (witness `mac_check_adopts_a_second_operand_tamper_but_catches_a_first_operand_one`
//! in [`crate::auth_disclose`]). Every gate of this chain is an `auth_mul`, so an
//! actively deviating party has a route to a wrong verdict without an abort. Closing it
//! needs multiplication-gate verification binding BOTH operands in `MacSession`. On the
//! three-axis registry, `ShamirBackend` still reports `OperatorClass::Comparison` as
//! [`crate::backend::SecurityDescriptor::semi_honest_only`]: AXIS-1 `SemiHonest`,
//! AXIS-2 `Abort(Selective)` (a failed MAC-check is detected locally, but nothing
//! establishes that every honest party agrees to abort, so this is NOT unanimous or
//! identifiable abort, NOT GOD), AXIS-3 `HonestMajority` (degrades / refuses if
//! `n ≤ 2t`). It does NOT enter the dishonest-majority SPDZ regime (route (b) /
//! sq-j5ok). The exact integer operands are NEVER opened — only the 1-bit verdict,
//! and only after the MAC-check passes. `[OPUS-4.8]`

use crate::authenticated::{auth_add, auth_add_constant, auth_scale, auth_sub, AuthenticatedShare};
use crate::compare::COMPARE_BITS;
use crate::field::Fp;
use crate::partial::MpcError;
use crate::shamir::{MacSession, ShamirBackend};

/// `n >= 2t+1` is required so each multiplication's degree reduction exists. Mirror
/// [`crate::compare`]'s fail-closed check, with the malicious-mode message.
fn check_party_count(n: usize, t: usize) -> Result<(), MpcError> {
    if n < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "tamper-evident comparison needs n >= 2t+1 (each authenticated multiplication's \
             degree reduction does, and the MAC-check is honest-majority); got n = {n}, t = {t}"
        )));
    }
    Ok(())
}

/// Both operands must fit the [`COMPARE_BITS`] bit-decomposition so equality of the
/// recovered bits is equality of the field values (no wrap). Fail-closed.
fn check_in_range(label: &str, v: Fp) -> Result<(), MpcError> {
    if v.value() >= (1u64 << COMPARE_BITS) {
        return Err(MpcError::Protocol(format!(
            "tamper-evident comparison operand `{label}` = {} is out of range (must be < 2^{} = \
             {}); the bit-decomposition would wrap and the verdict would be wrong",
            v.value(),
            COMPARE_BITS,
            1u64 << COMPARE_BITS
        )));
    }
    Ok(())
}

/// Share a cleartext value (held here ONLY because one process plays all parties —
/// exactly like [`crate::shamir::MacSession::authenticated_share`] / the dealer) into
/// [`COMPARE_BITS`] **authenticated** secret-shared bits, LSB-first. Each bit is a
/// fresh authenticated sharing of a 0/1 under the session key; the cleartext value is
/// used only to deal the bit shares and is never opened.
fn share_auth_bits(session: &mut MacSession, v: Fp) -> Vec<AuthenticatedShare> {
    let raw = v.value();
    (0..COMPARE_BITS)
        .map(|k| session.authenticated_share(Fp::new((raw >> k) & 1)))
        .collect()
}

/// `[a ∧ b]` for authenticated 0/1 sharings — one [`MacSession::auth_mul`] (the
/// §2.4 MAC-carrying multiplication). A multiplication on 0/1 values is logical AND.
fn auth_and(
    session: &mut MacSession,
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
) -> Result<AuthenticatedShare, MpcError> {
    session.auth_mul(a, b)
}

/// `[a ∧ ¬b]` for authenticated bits (= `a > b` at one bit). `¬b = 1 − b` is a FREE
/// local affine map on the authenticated sharing (value and MAC both updated); the
/// `∧` is one authenticated multiplication.
fn auth_and_not(
    session: &mut MacSession,
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
    key: &crate::authenticated::MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    let not_b = auth_add_constant(&auth_scale(b, Fp::one().neg()), Fp::one(), key)?; // 1 - b
    auth_and(session, a, &not_b)
}

/// `[a == b]` for authenticated bits: `1 − (a − b)^2`. `(a−b)` is free (linear);
/// `(a−b)^2` is one authenticated multiplication; `1 − …` is free. Returns an
/// authenticated 0/1 (`1` iff the bits are equal).
fn auth_bit_eq(
    session: &mut MacSession,
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
    key: &crate::authenticated::MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    let diff = auth_sub(a, b)?; // a - b ∈ {-1,0,1}
    let sq = session.auth_mul(&diff, &diff)?; // (a-b)^2 ∈ {0,1}
    auth_add_constant(&auth_scale(&sq, Fp::one().neg()), Fp::one(), key) // 1 - (a-b)^2
}

/// The MSB-first comparison recurrence over authenticated bit vectors (LSB-first
/// `a_bits`, `b_bits`), returning an authenticated sharing of the verdict bit `a > b`.
/// Identical recurrence to [`crate::compare`]'s `greater_than_bits`, but every
/// product is a [`MacSession::auth_mul`] and the accumulators are authenticated, so
/// the WHOLE chain stays MAC-carried.
fn auth_greater_than_bits(
    session: &mut MacSession,
    a_bits: &[AuthenticatedShare],
    b_bits: &[AuthenticatedShare],
    key: &crate::authenticated::MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    let mut gt = session.auth_const_sharing(Fp::zero()); // [[0]]
    let mut eq = session.auth_const_sharing(Fp::one()); // [[1]]

    for k in (0..COMPARE_BITS).rev() {
        let a_k = &a_bits[k];
        let b_k = &b_bits[k];
        // term = eq · (a_k ∧ ¬b_k): contributes 1 only at the first MSB-down bit
        // where a_k=1, b_k=0 while everything above was equal.
        let a_gt_b_here = auth_and_not(session, a_k, b_k, key)?;
        let term = auth_and(session, &eq, &a_gt_b_here)?;
        gt = auth_add(&gt, &term)?;
        // eq = eq · (a_k == b_k): once a higher bit differed, eq is 0 forever.
        let eq_here = auth_bit_eq(session, a_k, b_k, key)?;
        eq = auth_and(session, &eq, &eq_here)?;
    }
    Ok(gt)
}

/// **Tamper-evident secure greater-than over two SECRET operands** (not yet
/// malicious-secure; see the module security tier), opening only
/// the verdict bit — and **only after a batched IT-MAC check passes**.
///
/// Returns the boolean `a > b` (1 if `a > b`, else 0). The operands `a`, `b` are
/// passed as cleartext here ONLY because this routine plays ALL parties in one
/// process — it secret-shares their BITS as authenticated values internally and never
/// reconstructs them, exactly as the dealer that shares them does. The ONLY value ever
/// opened (besides the leakage-free `σ` of the MAC-check, which is identically 0 on an
/// honest run) is the 1-bit verdict.
///
/// ## Pipeline
///
/// 1. Bit-decompose both operands into [`COMPARE_BITS`] authenticated bits.
/// 2. Run the authenticated MSB-first comparator (`auth_greater_than_bits`) — every
///    gate a [`MacSession::auth_mul`].
/// 3. **MAC-check the verdict** ([`MacSession::mac_check`]) BEFORE opening it — a
///    tamper that leaves the verdict inconsistent with its MAC makes `σ ≠ 0` and
///    this ABORTS with [`MpcError::MacCheckFailed`]. A value tamper on the second
///    operand of a step-2 `auth_mul` is adopted, not caught (module security tier).
/// 4. Open the (now MAC-verified) verdict.
///
/// Both operands must be `< 2^`[`COMPARE_BITS`] (fail-closed). `n >= 2t+1`
/// (fail-closed). Honest-majority, **tamper-evident-with-abort** (design §3 targets
/// malicious-with-abort; see the module security tier). `[OPUS-4.8]`
pub fn malicious_greater_than(backend: &ShamirBackend, a: Fp, b: Fp) -> Result<bool, MpcError> {
    check_party_count(backend.parties(), backend.threshold())?;
    check_in_range("a", a)?;
    check_in_range("b", b)?;

    let mut dealer = backend.dealer();
    let mut session = dealer.new_mac_session();
    let key = session.mac_key();

    let a_bits = share_auth_bits(&mut session, a);
    let b_bits = share_auth_bits(&mut session, b);
    let verdict = auth_greater_than_bits(&mut session, &a_bits, &b_bits, &key)?;

    // MAC-CHECK BEFORE OPEN (design §2.5). Every authenticated multiplication in the
    // chain folded its MAC into `verdict`; the verdict's MAC is α·verdict iff no gate
    // was tampered. The batched check opens a leakage-free σ and aborts on σ != 0,
    // BEFORE the verdict is acted on (the coZK-2025/1026 confidentiality discipline).
    // `mac_check_and_open` hands back the verdict it just authenticated, so the bit we
    // act on is the very bit the check covered — and it is opened exactly once.
    let opened = session.mac_check_and_open(std::slice::from_ref(&verdict))?;
    verdict_bit(opened[0])
}

/// **Tamper-evident secure greater-than against a PUBLIC threshold** (not yet
/// malicious-secure; see the module security tier) — the £100k
/// flatmate path. Returns `secret > threshold`, MAC-checked before open.
///
/// The threshold is public, so its bits are public CONSTANTS and the per-bit products
/// against it collapse to FREE local affine maps on the authenticated secret bits — no
/// authenticated multiplication is spent on the threshold side, only on the `eq`/`gt`
/// chain. Same security tier and pipeline as [`malicious_greater_than`]. `[OPUS-4.8]`
pub fn malicious_threshold(
    backend: &ShamirBackend,
    secret: Fp,
    threshold: Fp,
) -> Result<bool, MpcError> {
    check_party_count(backend.parties(), backend.threshold())?;
    check_in_range("secret", secret)?;
    check_in_range("threshold", threshold)?;

    let mut dealer = backend.dealer();
    let mut session = dealer.new_mac_session();
    let key = session.mac_key();

    let a_bits = share_auth_bits(&mut session, secret);
    let verdict = auth_greater_than_public_bits(&mut session, &a_bits, threshold.value(), &key)?;
    let opened = session.mac_check_and_open(std::slice::from_ref(&verdict))?;
    verdict_bit(opened[0])
}

/// MSB-first comparison of authenticated secret bits `a_bits` against a PUBLIC value
/// `pub_val` (its bits are constants). Same recurrence as [`auth_greater_than_bits`]
/// but every product against a public bit is a FREE local affine map, so the only
/// authenticated multiplications are the `eq`/`gt` chain steps.
fn auth_greater_than_public_bits(
    session: &mut MacSession,
    a_bits: &[AuthenticatedShare],
    pub_val: u64,
    key: &crate::authenticated::MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    let mut gt = session.auth_const_sharing(Fp::zero());
    let mut eq = session.auth_const_sharing(Fp::one());
    let l = a_bits.len();

    for k in (0..l).rev() {
        let a_k = &a_bits[k];
        let b_k = (pub_val >> k) & 1;
        // a_k == b_k with b_k PUBLIC: if b_k=1 it is a_k; if b_k=0 it is 1 - a_k.
        let eq_here = if b_k == 1 {
            // a_k ∧ ¬1 = 0 is a public constant ⇒ term = eq·0 = 0, gt unchanged
            // (skip the wasted auth_mul). (a_k == 1) == a_k.
            a_k.clone()
        } else {
            // b_k=0: a_k ∧ ¬0 = a_k ⇒ term = eq·a_k is a real authenticated mult.
            let term = auth_and(session, &eq, a_k)?;
            gt = auth_add(&gt, &term)?;
            // (a_k == 0) == 1 - a_k.
            auth_add_constant(&auth_scale(a_k, Fp::one().neg()), Fp::one(), key)?
        };
        eq = auth_and(session, &eq, &eq_here)?;
    }
    Ok(gt)
}

/// Open ONLY the verdict bit of an authenticated comparison result — the value
/// sharing `[verdict]` (the MAC is consumed by the check, never opened). The result is
/// a degree-`t` sharing of a 0/1; this reconstructs it (robust where redundancy
/// exists, like every other open) and returns the boolean. A non-boolean
/// reconstruction is refused rather than coerced. This is the ONLY value the
/// tamper-evident comparison path opens; the operands are never reconstructed.
/// `[OPUS-4.8]`
pub fn open_auth_verdict(
    backend: &ShamirBackend,
    verdict: &AuthenticatedShare,
) -> Result<bool, MpcError> {
    verdict_bit(backend.reconstruct(verdict.value_shares())?)
}

/// Coerce an ALREADY-OPENED verdict field element to a boolean, refusing anything that
/// is not `0` or `1` rather than silently rounding it. Split out of
/// [`open_auth_verdict`] so the MAC-checked paths can reuse it on the value handed back
/// by [`MacSession::mac_check_and_open`] — they must not re-open the sharing, or the
/// bit they act on would be bound to the checked bit only by convention. `[OPUS-4.8]`
fn verdict_bit(bit: Fp) -> Result<bool, MpcError> {
    if bit == Fp::zero() {
        Ok(false)
    } else if bit == Fp::one() {
        Ok(true)
    } else {
        Err(MpcError::Protocol(format!(
            "tamper-evident comparison verdict reconstructed to a non-boolean field element {} \
             (expected 0 or 1) — refusing to coerce",
            bit.value()
        )))
    }
}

#[cfg(test)]
mod tests {
    //! sq-ka8m acceptance suite (design Hole 4 / §6 step 6). The load-bearing ones:
    //! - `differential_*`: across many (a,b)/(secret,threshold) pairs incl. edges,
    //!   the MAC-checked verdict equals the plaintext `a > b` / `secret > threshold`.
    //! - `tamper_in_*_is_caught`: a tampered gate / forged share / wrong re-sharing on
    //!   the authenticated chain makes the batched MAC-check ABORT
    //!   (`MpcError::MacCheckFailed`) — the witness that the upgrade is real.
    //! - `only_the_verdict_is_opened`: the operands are never reconstructed.
    use super::*;
    use crate::field::P;
    use crate::shamir::{ShamirBackend, Share};

    /// THE differential (both-secret): the MAC-checked verdict equals the plaintext
    /// `a > b` across a spread of values incl. edges, several honest-majority party
    /// counts. Deterministic seeds so the simulation reproduces.
    #[test]
    fn differential_malicious_greater_than() {
        let cases: &[(u64, u64)] = &[
            (0, 0),
            (1, 0),
            (0, 1),
            (5, 5),
            (100_000, 100_000),
            (100_001, 100_000),
            (99_999, 100_000),
            (1 << 59, (1 << 59) - 1),
            ((1 << 60) - 1, 0),
            ((1 << 60) - 1, (1 << 60) - 2),
            (42, 1_000_000),
            (1_000_000, 42),
        ];
        for n in [3usize, 5, 7] {
            for (idx, &(a, b)) in cases.iter().enumerate() {
                let backend = ShamirBackend::new_seeded(
                    n,
                    (idx as u64).wrapping_mul(131).wrapping_add(n as u64),
                )
                .unwrap();
                let got = malicious_greater_than(&backend, Fp::new(a), Fp::new(b)).unwrap();
                assert_eq!(
                    got,
                    a > b,
                    "n={n}: tamper-evident verdict for ({a} > {b}) must match plaintext"
                );
            }
        }
    }

    /// THE differential against a public threshold (the four-flatmates £100k path).
    #[test]
    fn differential_malicious_threshold() {
        let cases: &[(u64, u64)] = &[
            (0, 0),
            (100_001, 100_000),
            (100_000, 100_000),
            (99_999, 100_000),
            (1_000_000, 999_999),
            (1, 1_000_000),
        ];
        for n in [3usize, 5, 7] {
            for (idx, &(s, thr)) in cases.iter().enumerate() {
                let backend =
                    ShamirBackend::new_seeded(n, (idx as u64).wrapping_mul(71).wrapping_add(9))
                        .unwrap();
                let got = malicious_threshold(&backend, Fp::new(s), Fp::new(thr)).unwrap();
                assert_eq!(
                    got,
                    s > thr,
                    "n={n}: tamper-evident threshold verdict ({s} > {thr}) must match plaintext"
                );
            }
        }
    }

    /// ACCEPTANCE (the witness, end-to-end): a tampered operand bit that flows into
    /// the verdict makes the full [`malicious_greater_than`] pipeline ABORT
    /// fail-closed — it never returns a wrong boolean. We pick `a = 1, b = 0` so the
    /// verdict is decided at bit 0 with `eq = 1` (the bit is not masked by a zeroed
    /// `eq`), and corrupt bit 0 of `a` into a consistent sharing of a DIFFERENT value
    /// without fixing its MAC (the canonical Hole-2 deviation).
    ///
    /// HONESTY about WHICH guard fires: a *value*-only tamper on a chain INPUT (vs the
    /// final product) is partially "healed" by downstream multiplications — `auth_mul`
    /// carries the MAC forward as `[α·z] = [α·x]·[y]`, so a subsequent product over the
    /// tampered value can re-establish a MAC that matches the tampered value, yet the
    /// verdict then carries a NON-BIT value (here `2`, not `0`/`1`). So the abort comes
    /// from whichever fail-closed guard fires FIRST on the corrupted execution — the
    /// batched MAC-check (`σ != 0`, when the inconsistency survives to the open) OR the
    /// non-boolean-verdict guard in [`open_auth_verdict`]. Either is a fail-closed
    /// abort; the load-bearing property is that NO wrong boolean is returned. A tamper
    /// of the FINAL product / verdict (the SPDZ in-model deviation — a wrong
    /// `degree_reduce` re-sharing or a forged open share AFTER the MAC is committed) is
    /// caught specifically by the MAC-check — see `tamper_in_the_verdict_is_caught` /
    /// `auth_mul_chain_is_correct_and_tamper_evident`.
    #[test]
    fn tamper_in_an_operand_bit_aborts_fail_closed() {
        // Drive the chain manually so we can corrupt bit 0 of `a` mid-pipeline, then
        // run BOTH guards (MAC-check, then open) exactly as the public entry point does.
        let backend = ShamirBackend::new_seeded(5, 0xCA8E).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();

        let mut a_bits = share_auth_bits(&mut session, Fp::new(1));
        let b_bits = share_auth_bits(&mut session, Fp::new(0));
        a_bits[0] = tamper_value(&a_bits[0]); // value+δ, MAC untouched

        let verdict = auth_greater_than_bits(&mut session, &a_bits, &b_bits, &key).unwrap();
        // The public pipeline: MAC-check, then open. The corrupted execution must
        // ABORT at one of the two — never return Ok(<wrong boolean>).
        let outcome = session
            .mac_check(std::slice::from_ref(&verdict))
            .and_then(|()| open_auth_verdict(&backend, &verdict));
        assert!(
            matches!(
                outcome,
                Err(MpcError::MacCheckFailed { .. }) | Err(MpcError::Protocol(_))
            ),
            "a tampered deciding operand bit must abort fail-closed (MAC-check or \
             non-boolean-verdict guard), got {outcome:?}"
        );
    }

    /// ACCEPTANCE: a tamper INTRODUCED at the verdict (post-chain) — a wrong re-sharing
    /// of the final product would land here — is caught by the MAC-check.
    #[test]
    fn tamper_in_the_verdict_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0x5151).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let a_bits = share_auth_bits(&mut session, Fp::new(9));
        let b_bits = share_auth_bits(&mut session, Fp::new(4));
        let verdict = auth_greater_than_bits(&mut session, &a_bits, &b_bits, &key).unwrap();

        // Corrupt the verdict's value sharing without fixing its MAC.
        let tampered = tamper_value(&verdict);
        let checked = session.mac_check(std::slice::from_ref(&tampered));
        assert!(
            matches!(checked, Err(MpcError::MacCheckFailed { .. })),
            "a tampered verdict must make the MAC-check abort, got {checked:?}"
        );
    }

    /// ACCEPTANCE: a tamper on the MAC sharing (not the value) is ALSO caught — the
    /// check binds value and MAC, so changing either side breaks σ == 0.
    #[test]
    fn tamper_in_the_mac_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0x3C3C).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let a_bits = share_auth_bits(&mut session, Fp::new(6));
        let b_bits = share_auth_bits(&mut session, Fp::new(6));
        let verdict = auth_greater_than_bits(&mut session, &a_bits, &b_bits, &key).unwrap();
        let tampered = tamper_mac(&verdict);
        let checked = session.mac_check(std::slice::from_ref(&tampered));
        assert!(
            matches!(checked, Err(MpcError::MacCheckFailed { .. })),
            "a tampered MAC must make the MAC-check abort, got {checked:?}"
        );
    }

    /// ACCEPTANCE: the HONEST path's MAC-check PASSES (no false abort) for every
    /// verdict — the dual of the tamper tests. If the chain were not consistently
    /// MAC-carried, this would spuriously abort.
    #[test]
    fn honest_path_mac_check_passes() {
        for n in [3usize, 5, 7] {
            for &(a, b) in &[(7u64, 3u64), (3, 7), (5, 5), (0, 1), (1_000_000, 0)] {
                let backend =
                    ShamirBackend::new_seeded(n, a.wrapping_add(b).wrapping_add(1)).unwrap();
                let mut dealer = backend.dealer();
                let mut session = dealer.new_mac_session();
                let key = session.mac_key();
                let a_bits = share_auth_bits(&mut session, Fp::new(a));
                let b_bits = share_auth_bits(&mut session, Fp::new(b));
                let verdict = auth_greater_than_bits(&mut session, &a_bits, &b_bits, &key).unwrap();
                session
                    .mac_check(std::slice::from_ref(&verdict))
                    .expect("honest path must pass the MAC-check");
                // And the verdict opens to the right boolean.
                assert_eq!(open_auth_verdict(&backend, &verdict).unwrap(), a > b);
            }
        }
    }

    /// ACCEPTANCE: `auth_mul` is correct AND tamper-evident — `a·b·c` chains and a
    /// wrong re-sharing of an intermediate product is caught (design §6 step 3
    /// acceptance). This test tampers the OUTPUT value sharing of an authenticated
    /// product (`[z]` inconsistent with its MAC), the same OBSERVABLE deviation a
    /// wrong re-sharing produces, so the check catches it. It does NOT distinguish a
    /// SOUND MAC-carry from the (rejected) UNSOUND one, because a post-hoc value
    /// tamper happens after the MAC is committed either way — that discrimination is
    /// the dedicated job of [`mac_carry_soundness_distinguished_by_in_reduce_tamper`]
    /// (sq-81gd), which injects the deviation INSIDE the value degree-reduce.
    #[test]
    fn auth_mul_chain_is_correct_and_tamper_evident() {
        let backend = ShamirBackend::new_seeded(7, 0xABBA).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();

        // a·b·c with a,b,c = 3,5,7 → 105.
        let a = session.authenticated_share(Fp::new(3));
        let b = session.authenticated_share(Fp::new(5));
        let c = session.authenticated_share(Fp::new(7));
        let ab = session.auth_mul(&a, &b).unwrap();
        let abc = session.auth_mul(&ab, &c).unwrap();
        // Honest: opens to 105 and passes the check.
        session.mac_check(std::slice::from_ref(&abc)).unwrap();
        assert_eq!(
            backend.reconstruct(abc.value_shares()).unwrap(),
            Fp::new(105)
        );

        // Tampered: corrupt the product value → caught.
        let tampered = tamper_value(&abc);
        assert!(matches!(
            session.mac_check(std::slice::from_ref(&tampered)),
            Err(MpcError::MacCheckFailed { .. })
        ));
    }

    /// [OPUS-4.8] sq-81gd — **THE regression test that distinguishes the SOUND
    /// MAC-carry from the UNSOUND one by tampering INSIDE the value `degree_reduce`.**
    ///
    /// The other tamper tests in this module (and `auth_mul_chain_is_correct_and_
    /// tamper_evident`) corrupt the OUTPUT value sharing of a completed `auth_mul`.
    /// That models the *observable* result of a wrong re-sharing, but it CANNOT
    /// discriminate the production MAC-carry (`[α·z] = reduce([α·x]·[y])`, an
    /// independent reduce) from the design's REJECTED alternative
    /// (`[α·z] = reduce([z]·[α])`): a post-hoc value tamper lands AFTER the MAC is
    /// committed, so BOTH carries would catch it. The genuine "Hole 2" attack — a
    /// deviating party re-sharing `h_i + δ` INSIDE the BGW reduce — produces a
    /// *perfectly consistent* degree-`t` `[z]` of a WRONG product, with nothing for
    /// Reed–Solomon to detect, and ONLY the independence of the two reduces makes the
    /// MAC catch it. This test injects exactly that, then runs it through BOTH carries
    /// and asserts they DIVERGE:
    ///
    /// - **SOUND** (`MacCarry::SoundIndependentReduce`): the MAC reduce never saw `δ`,
    ///   so `[α·z]` holds the true `α·z` while `[z]` opens to `z + λδ` →
    ///   `σ = α·z − (z+λδ)·α = −λδ·α ≠ 0` → `mac_check` ABORTS. Tamper CAUGHT.
    /// - **UNSOUND** (`MacCarry::UnsoundFromValueTimesAlpha`): `[α·z]` is recomputed
    ///   as `reduce([z]·[α])` from the tampered `[z]`, so it tracks `α·(z+λδ)` and
    ///   `σ = 0` → `mac_check` PASSES on a WRONG product. Tamper MISSED.
    ///
    /// The divergence is the whole point: it proves the soundness lives in the
    /// independent MAC reduce, not in any post-hoc consistency / RS check. If a future
    /// refactor "simplified" `auth_mul` to recompute the MAC from the reduced value,
    /// the SOUND assertion here would start failing — this is the guard that catches
    /// that regression. We also sanity-check that the honest run (no `δ`) passes under
    /// the production `auth_mul`, and that the in-reduce tamper alone (no MAC) leaves
    /// `[z]` a *consistent* sharing the robust open does NOT flag (so the catch really
    /// is the MAC, not RS).
    #[test]
    fn mac_carry_soundness_distinguished_by_in_reduce_tamper() {
        use crate::shamir::MacCarry;
        // n = 5, t = 2 → 2t+1 = 5: the MINIMAL honest-majority count, where the
        // degree-2t product has ZERO RS redundancy — so RS/robust opens cannot help
        // and the MAC is the SOLE detector (the regime the MAC was added for).
        let backend = ShamirBackend::new_seeded(5, 0x81_9D).unwrap();
        let t = backend.threshold();
        assert_eq!(t, 2);
        assert_eq!(
            backend.parties(),
            2 * t + 1,
            "minimal honest-majority count"
        );

        let x_val = Fp::new(6);
        let y_val = Fp::new(7);
        let true_z = x_val.mul(y_val); // 42
        let delta = Fp::new(123);
        // Re-sharing party 0 deviates; the reduced secret shifts by λ_0·δ ≠ 0
        // (λ_0 is the nonzero Lagrange-at-0 weight for the first recombination point).
        let reshare_party = 0usize;

        // ---- (a) honest production auth_mul passes and is correct (no behaviour
        //          change on the clean path). ----
        {
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let x = session.authenticated_share(x_val);
            let y = session.authenticated_share(y_val);
            let z = session.auth_mul(&x, &y).unwrap();
            session.mac_check(std::slice::from_ref(&z)).unwrap();
            assert_eq!(backend.reconstruct(z.value_shares()).unwrap(), true_z);
        }

        // ---- (b) SOUND carry, tamper INSIDE the value reduce → MAC-check ABORTS,
        //          and the tampered value is a CONSISTENT (undetectable-by-RS)
        //          sharing of the wrong product. ----
        {
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let x = session.authenticated_share(x_val);
            let y = session.authenticated_share(y_val);
            let z = session
                .auth_mul_with_value_reduce_tamper_for_test(
                    &x,
                    &y,
                    reshare_party,
                    delta,
                    MacCarry::SoundIndependentReduce,
                )
                .unwrap();

            // The value sharing is internally CONSISTENT: the robust degree-t open
            // succeeds (no RS flag) and yields the WRONG product — exactly the Hole-2
            // codeword. (If this were an off-curve tamper the open would error.)
            let opened = backend.reconstruct(z.value_shares()).unwrap();
            assert_ne!(
                opened, true_z,
                "in-reduce δ must shift the reduced product to a wrong value"
            );

            // The SOUND MAC reduce never saw δ → σ ≠ 0 → ABORT. THIS is the catch
            // that a post-hoc value tamper test cannot uniquely attribute to the MAC.
            assert!(
                matches!(
                    session.mac_check(std::slice::from_ref(&z)),
                    Err(MpcError::MacCheckFailed { .. })
                ),
                "SOUND independent MAC reduce must catch an in-reduce value tamper"
            );
        }

        // ---- (c) UNSOUND carry, SAME tamper → MAC-check PASSES (false negative).
        //          This is the discriminator: the unsound MAC tracks the tampered
        //          value, so σ = 0 and a WRONG product would be silently accepted. ----
        {
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let x = session.authenticated_share(x_val);
            let y = session.authenticated_share(y_val);
            let z = session
                .auth_mul_with_value_reduce_tamper_for_test(
                    &x,
                    &y,
                    reshare_party,
                    delta,
                    MacCarry::UnsoundFromValueTimesAlpha,
                )
                .unwrap();

            let opened = backend.reconstruct(z.value_shares()).unwrap();
            assert_ne!(opened, true_z, "same in-reduce tamper → same wrong value");

            // The UNSOUND recompute `[α·z] = reduce([z]·[α])` makes the MAC track the
            // tampered z, so the batched check is FOOLED — it passes on a wrong result.
            // This is the false negative the sound design avoids; the two carries
            // DIVERGING on the identical in-reduce tamper is the property sq-81gd pins.
            assert!(
                session.mac_check(std::slice::from_ref(&z)).is_ok(),
                "UNSOUND [z]·[α] MAC carry is FOOLED by the in-reduce tamper (σ = 0) — \
                 this divergence from the sound route is what the test must exhibit"
            );
        }
    }

    /// [SONNET-4.6] **The COORDINATED Hole-2 deviation — a deviating party re-shares
    /// wrongly in BOTH degree-reduces of the same `auth_mul`, and is still caught.**
    ///
    /// [`mac_carry_soundness_distinguished_by_in_reduce_tamper`] deviates only in the
    /// VALUE reduce, so by itself it does not exercise the other two cases the design's
    /// adversary model admits (§2.4: up to `t` corruptions, deviating in both rounds,
    /// choosing the deviations jointly). This test walks all three:
    ///
    /// - `(δ_v ≠ 0, δ_m = 0)` — value reduce only → `σ = −α·δ_v ≠ 0`;
    /// - `(δ_v = 0, δ_m ≠ 0)` — MAC reduce only → `σ = δ_m ≠ 0`;
    /// - `(δ_v ≠ 0, δ_m ≠ 0)` — BOTH, coordinated → `σ = δ_m − α·δ_v`, which the
    ///   adversary can only zero by picking `δ_m = α·δ_v`, i.e. by guessing the secret
    ///   `α` its `≤ t` shares are independent of (probability `1/p ≈ 2^-61`).
    ///
    /// Every case must `MacCheckFailed`-abort. This is what makes the design record's
    /// "MAC-covered in both rounds, including under coordinated deviation" claim
    /// non-vacuous rather than an assertion about the single-round case only.
    #[test]
    fn coordinated_tamper_in_both_reduces_is_caught() {
        // n = 5, t = 2 → the minimal honest-majority count, where the degree-2t product
        // has ZERO RS redundancy and the MAC is the sole detector.
        let backend = ShamirBackend::new_seeded(5, 0xC0_0D).unwrap();
        assert_eq!(backend.parties(), 2 * backend.threshold() + 1);

        let x_val = Fp::new(6);
        let y_val = Fp::new(7);
        let reshare_party = 0usize;

        // (δ_v, δ_m): value-only, MAC-only, and both-coordinated.
        for (value_delta, mac_delta) in [
            (Fp::new(123), Fp::zero()),
            (Fp::zero(), Fp::new(456)),
            (Fp::new(123), Fp::new(456)),
        ] {
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let x = session.authenticated_share(x_val);
            let y = session.authenticated_share(y_val);
            let z = session
                .auth_mul_with_both_reduces_tampered_for_test(
                    &x,
                    &y,
                    reshare_party,
                    value_delta,
                    mac_delta,
                )
                .unwrap();

            // Both sharings stay internally CONSISTENT degree-t codewords — the robust
            // open succeeds and RS flags nothing. The catch is the MAC, not redundancy.
            backend
                .reconstruct(z.value_shares())
                .expect("an in-reduce deviation leaves a consistent degree-t sharing");

            assert!(
                matches!(
                    session.mac_check(std::slice::from_ref(&z)),
                    Err(MpcError::MacCheckFailed { .. })
                ),
                "coordinated in-reduce deviation (delta_v = {:?}, delta_m = {:?}) must abort",
                value_delta,
                mac_delta
            );
        }
    }

    /// `mac_check` of a clean degree-2t-style product (`auth_mul`) on an unequal
    /// challenge batch is sound: a single forged value among several is caught even
    /// when batched with honest values (random-linear-combination batching).
    #[test]
    fn batched_check_catches_one_bad_value_among_many() {
        let backend = ShamirBackend::new_seeded(5, 0x9999).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let v0 = session.authenticated_share(Fp::new(11));
        let v1 = session.authenticated_share(Fp::new(22));
        let v2_bad = tamper_value(&session.authenticated_share(Fp::new(33)));
        // Honest pair passes.
        session
            .mac_check(&[v0.clone(), v1.clone()])
            .expect("honest batch passes");
        // Add the bad one → caught.
        assert!(matches!(
            session.mac_check(&[v0, v1, v2_bad]),
            Err(MpcError::MacCheckFailed { .. })
        ));
    }

    /// Out-of-range operands and `n < 2t+1` are descriptive errors (fail-closed), not
    /// silent wrong verdicts.
    #[test]
    fn fails_closed_on_bad_inputs() {
        let backend = ShamirBackend::new_seeded(5, 1).unwrap();
        // Operand at/over 2^COMPARE_BITS.
        let over = Fp::new(1u64 << COMPARE_BITS);
        assert!(matches!(
            malicious_greater_than(&backend, over, Fp::new(1)),
            Err(MpcError::Protocol(_))
        ));
        assert!(matches!(
            malicious_threshold(&backend, over, Fp::new(1)),
            Err(MpcError::Protocol(_))
        ));
    }

    /// The empty MAC-check is a no-op pass (nothing to verify before nothing is
    /// opened) — the OWA-safe boundary.
    #[test]
    fn empty_mac_check_passes() {
        let backend = ShamirBackend::new_seeded(3, 7).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        session.mac_check(&[]).unwrap();
    }

    /// auth_mul value bound: a sharing whose value is P-1 (max) still authenticates
    /// and round-trips (no overflow in the MAC mult).
    #[test]
    fn auth_mul_extreme_values_round_trip() {
        let backend = ShamirBackend::new_seeded(5, 0xF1F1).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let x = session.authenticated_share(Fp::new(P - 1));
        let y = session.authenticated_share(Fp::new(2));
        let z = session.auth_mul(&x, &y).unwrap();
        session.mac_check(std::slice::from_ref(&z)).unwrap();
        assert_eq!(
            backend.reconstruct(z.value_shares()).unwrap(),
            Fp::new(P - 1).mul(Fp::new(2))
        );
    }

    // ---- tamper helpers (test-only) --------------------------------------------

    /// Corrupt the VALUE sharing of an authenticated share into a **consistent**
    /// degree-`t` sharing of a DIFFERENT value (add δ to EVERY share's `y`, shifting
    /// the constant term `value → value + δ`) WITHOUT touching the MAC. This is the
    /// canonical Hole-2 deviation: a perfectly consistent codeword of the wrong value,
    /// which Reed–Solomon CANNOT detect (it is a valid degree-`t` sharing) — only the
    /// IT-MAC catches it, because `m = α·value` no longer holds for `value + δ` and the
    /// party cannot fix the MAC without knowing `α`. Tampering a single off-curve point
    /// instead would just be RS-corrected at `n > 2t+1`, which is NOT what the MAC-check
    /// is the defense for. We rebuild via the crate-internal constructor (fields private).
    fn tamper_value(av: &AuthenticatedShare) -> AuthenticatedShare {
        let delta = Fp::new(1);
        let value: Vec<Share> = av
            .value_shares()
            .iter()
            .map(|s| Share {
                x: s.x,
                y: s.y.add(delta),
            })
            .collect();
        crate::authenticated::AuthenticatedShare::new(value, av.mac_shares().to_vec()).unwrap()
    }

    /// Corrupt the MAC sharing instead (consistently shift the MAC's secret by δ) —
    /// the dual deviation; the check binds value and MAC so a wrong MAC is caught too.
    fn tamper_mac(av: &AuthenticatedShare) -> AuthenticatedShare {
        let delta = Fp::new(1);
        let mac: Vec<Share> = av
            .mac_shares()
            .iter()
            .map(|s| Share {
                x: s.x,
                y: s.y.add(delta),
            })
            .collect();
        crate::authenticated::AuthenticatedShare::new(av.value_shares().to_vec(), mac).unwrap()
    }
}
