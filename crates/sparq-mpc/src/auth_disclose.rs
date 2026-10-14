// [OPUS-4.8] sq-6fv7 (sq-ka8m residual; design Hole 4): route the THREE disclose
// path opens — the square-protocol `a²`, the masked open `c = sum + r`, and the
// boolean `verdict` — through the §2.5 batched IT-MAC check, the IT-MAC-hardened
// twin of `compare::disclose_threshold_verdict`.
//! Honest-majority **tamper-evident-with-abort** threshold disclosure over an
//! EXISTING secret-shared sum — the IT-MAC upgrade of the semi-honest
//! [`crate::compare::disclose_threshold_verdict`] (Hole 4, the federation £100k path).
//!
//! **⚠ This module is EXPERIMENTAL and is NOT malicious-secure.** Its public entry
//! point is [`experimental_tamper_evident_disclose_threshold_verdict`] — renamed from
//! `malicious_disclose_threshold_verdict` in review round 2, because the old name
//! claimed a tier the code demonstrably misses (see "Integrity coverage is NOT total"
//! below, and that function's own docs for why a rename, rather than a revert, is the
//! containment that is actually available). The prose below describes the IT-MAC
//! construction as designed; read the residual section before treating any of it as a
//! guarantee against an actively-deviating party. `[SONNET-4.6]`
//!
//! ## What this closes (sq-ka8m residual)
//!
//! sq-ka8m's [`crate::auth_compare`] made the *cleartext fresh-operand* comparison
//! tamper-evident: it bit-shares the operands as authenticated values and
//! MAC-checks the verdict. But the FEDERATION path —
//! [`crate::compare::disclose_threshold_verdict`], which runs over an EXISTING
//! degree-`t` sharing `[sum]` (e.g. the cumulative aggregate from
//! [`crate::backend::MpcBackend::run_secure`]) — uses a *masked-open*
//! bit-decomposition (Damgård et al. TCC'06), not cleartext bit-sharing. That path
//! has **three distinct opens**, every one of them semi-honest (no integrity check)
//! on `origin/main`:
//!
//! 1. the **square-protocol `c = a²`** opened once per mask bit
//!    ([`crate::compare`] `square_protocol_random_bit`) — a forged open share /
//!    wrong re-sharing flips a mask bit;
//! 2. the **masked open `c = sum + r`** ([`crate::compare`] `secure_bit_decompose`)
//!    — the single load-bearing disclosure whose bits drive the whole comparison;
//! 3. the **boolean `verdict`** ([`crate::compare::open_verdict`]).
//!
//! Each is exactly the Hole-1/Hole-2 deviation the IT-MAC defends against: a
//! deviating party re-shares a consistent degree-`t` codeword of the *wrong* value,
//! undetectable by Reed–Solomon even at over-provisioned `n`. This module carries an
//! IT-MAC through all three and MAC-checks each opened value **before it is acted
//! on**, so a tamper aborts fail-closed at the minimal `n = 2t+1` (soundness from the
//! secret `α`, not RS redundancy — design §2.5).
//!
//! ## The construction
//!
//! - **Authenticate the existing sum.** `[[sum]] = ([sum], [α·sum])` via
//!   [`crate::shamir::MacSession::authenticate_existing`] — one BGW mult-then-reduce
//!   over `[α]` and `[sum]`, never reconstructing the sum.
//! - **Authenticated square-protocol mask bits.** Each mask bit `[[b]]` is produced
//!   from a fresh authenticated `[[a]]`: `[[a²]] = auth_mul([[a]],[[a]])`,
//!   **MAC-check `[[a²]]`**, open `c = a²`, then `[[b]] = (d⁻¹·[[a]] + 1)·2⁻¹` as
//!   FREE authenticated affine maps — so the bit stays MAC-carried into the
//!   subtraction circuit. `c = a²` is independent of the sign of `a`, so the open
//!   reveals nothing about the bit; the MAC-check catches a tampered open.
//! - **Authenticated masked open.** `[[r]] = Σ_k [[b_k]]·2^k` (free),
//!   `[[sum+r]] = auth_add([[sum]],[[r]])`, **MAC-check `[[sum+r]]`**, then open
//!   `c = sum + r`. The mask `r` is jointly generated (no party knows it), so `c`
//!   statistically hides `sum` exactly as the semi-honest path's open does.
//! - **Authenticated bitwise subtraction + comparison.** `[[a_k]] = c_k ⊖ [[r_k]]`
//!   over the PUBLIC bits of `c` and the AUTHENTICATED mask bits, with an
//!   authenticated ripple borrow (every AND an [`crate::shamir::MacSession::auth_mul`]);
//!   then the MSB-first comparator against the public threshold over `[[a_k]]`.
//! - **MAC-check the verdict, then open.** The verdict is the third and final open;
//!   the MAC-check over it (and the chain values it depends on) fires on any tamper
//!   in the subtraction or comparison gates.
//!
//! ## Security tier (stated precisely, no over-claim)
//!
//! **Honest-majority; tamper-EVIDENT-with-abort on the opened values, NOT full
//! malicious-with-abort** (design §3 aspires to the latter; see "Integrity coverage is
//! NOT total" below for the demonstrated gap). AXIS-2 local detect-and-abort (NOT an agreed
//! unanimous or identifiable abort), AXIS-3 `HonestMajority`; AXIS-1 is **not** `Malicious` today. The exact sum is NEVER opened
//! — only `c = a²` (sign-independent), the statistically-masked `c = sum + r`, and the
//! 1-bit verdict, and the latter two only after their MAC-checks pass — so the
//! CONFIDENTIALITY statement is unaffected by the integrity gap. `[SONNET-4.6]`
//!
//! ## The range-proof zero-tests are MAC-checked too (sq-m4zi / sq-e7ma)
//!
//! The in-protocol **range proof** (`sum ∈ [0, 2^`[`crate::compare::DECOMP_VALUE_BITS`]`)`,
//! sq-nx0s) — clause (1) recompose `sum == Σ b_k·2^k` and clause (2) high-part
//! `Σ_{k≥value_bits} b_k·2^k == 0`, each a secret zero-test of a `v·r` masked product
//! — was, on `sq-6fv7`, still reused VERBATIM from the SEMI-HONEST
//! `crate::compare::verify_sum_in_range`: a tampered open of either `v·r` product
//! (a wrong re-sharing producing a consistent codeword of the wrong value) could flip
//! "was it zero?" and let an out-of-range / field-wrapping sum masquerade as in-range,
//! defeating the fail-closed guard. `auth_verify_sum_in_range` closes that residual:
//! the range proof now runs over the AUTHENTICATED sum and bits, the recompose-diff and
//! high-part are FREE authenticated linear combinations, and each zero-test multiplies
//! by a fresh nonzero AUTHENTICATED mask via [`crate::shamir::MacSession::auth_mul`] and
//! **MAC-checks the product `[[v·r]]` before its open is read** — so a tampered zero-test
//! open aborts with [`MpcError::MacCheckFailed`] like the other three.
//!
//! ## [OPUS-4.8] sq-km34.6 — the production path is now the AUTHENTICATED RABBIT chain
//!
//! Everything above describes the *masked-open* construction, which capped this path's
//! sum at `2^`[`crate::compare::DECOMP_VALUE_BITS`]` = 2^20` — a 40-bit gap behind the
//! semi-honest [`crate::compare::disclose_threshold_verdict`], which sq-bgsn had already
//! lifted to the full `2^60`.
//! [`experimental_tamper_evident_disclose_threshold_verdict`] now routes through
//! `auth_bit_decompose_rabbit` instead: the Rabbit (eprint 2021/119) wrap
//! recovery, with **every** product and degree-reduce in its solved-bits / LTBits /
//! ripple-add / ripple-sub chain a [`crate::shamir::MacSession::auth_mul`], its single
//! masked open MAC-checked before `c` is read, its range proof
//! (`auth_verify_value_in_range_rabbit`) MAC-checked, and the boolean verdict
//! MAC-checked before it is opened. The two masked-open helpers are retained as
//! `#[cfg(test)]` regression references, exactly as `crate::compare` retains its own.
//!
//! The lift closes a real gap in the MAGNITUDE dimension: this path now supports the
//! SAME `2^60` as the semi-honest one. It does NOT lift the integrity tier — see
//! "Integrity coverage is NOT total" below before treating it as malicious-secure.
//! `[SONNET-4.6]`
//!
//! ## Residual (honest scope statement)
//!
//! - **Hiding.** The Rabbit open `c = (sum + r) mod p` is (near-)uniform with statistical
//!   distance `≤ 2^{-61}` from uniform — the field-size floor, INDEPENDENT of the sum's
//!   magnitude. (The masked-open path's `2^{-40}` gap, coupled to its 20-bit cap, applied
//!   to the now test-only helpers.) The exact sum is never opened.
//! - **Integrity coverage is NOT total — and the gap is LOAD-BEARING here (review round
//!   1).** `auth_mul` adopts a value tamper on its SECOND operand, and `mac_check` can
//!   only cover values that are opened anyway — so "a tamper in ANY gate aborts" is NOT
//!   claimed. This is not confined to an unrelated intermediate:
//!   `auth_secret_is_zero` places its supposedly-nonzero mask in exactly that adopted
//!   slot, so forcing the mask to a sharing of `0` makes a NONZERO `recompose_diff`
//!   yield an authenticated zero product and the range proof accepts a corrupted
//!   decomposition. The final verdict MAC-check cannot recover — it verifies a MAC that
//!   was adopted onto the wrong value upstream — so the pipeline returns a WRONG VERDICT
//!   rather than aborting. Pinned end to end by
//!   `a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`. **Until
//!   [`crate::shamir::MacSession`] gains multiplication-gate verification that binds BOTH
//!   operands (Chida-et-al. style), treat this path as a research/semi-honest surface:
//!   do NOT deploy it as the integrity tier against a genuinely malicious party.** See
//!   "What the MAC-check does NOT cover" on `auth_bit_decompose_rabbit`. `[SONNET-4.6]`
//! - **Containment applied (review round 2).** The entry point was renamed
//!   `malicious_disclose_threshold_verdict` →
//!   [`experimental_tamper_evident_disclose_threshold_verdict`], so the API no longer
//!   advertises the tier it misses; a doc caveat on a `malicious_*`-named drop-in is
//!   not containment. Two things this deliberately did NOT do, and why. **(a) It did not revert the sq-km34.6 Rabbit
//!   lift.** `auth_secret_is_zero` is byte-identical to `origin/main`, where the
//!   masked-open `auth_verify_sum_in_range` fed it the same adopted second-operand mask
//!   TWICE — the pre-lift production path was exploitable in exactly the same way, at a
//!   40-bit-narrower magnitude. Reverting would delete the witness, not the break.
//!   **(b) It did not "fix" the composition in this module.** Any in-module repair is
//!   local to one `auth_mul` call site while the adoption is a property of the
//!   primitive, present at every gate on the chain; the honest fix is the `MacSession`
//!   change named above, and a partial patch here would read as closure. `[SONNET-4.6]`
//! - The dishonest-majority SPDZ regime (route (b) / sq-j5ok) is NOT entered. `[OPUS-4.8]`

use crate::authenticated::{auth_add, auth_add_constant, auth_scale, auth_sub, AuthenticatedShare};
#[cfg(test)]
use crate::compare::{DECOMP_MASK_BITS, DECOMP_VALUE_BITS, DECOMP_VALUE_MAX_EXCLUSIVE};
use crate::compare::{
    check_party_count, RABBIT_MASK_BITS, RABBIT_VALUE_BITS, RABBIT_VALUE_MAX_EXCLUSIVE,
};
use crate::field::Fp;
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::shamir::{MacSession, ShamirBackend, Share};

/// **IT-MAC-hardened threshold disclosure over an existing secret-shared sum** — the
/// tamper-evident twin of [`crate::compare::disclose_threshold_verdict`]. Returns a
/// [`PartialResult`] carrying ONLY the boolean `sum > public_threshold`, never the
/// exact sum, with every opened value MAC-checked before it is acted on.
///
/// # ⚠ EXPERIMENTAL — NOT malicious-secure; not an integrity tier
///
/// [SONNET-4.6] Review round 1 demonstrated an end-to-end break:
/// [`crate::shamir::MacSession::auth_mul`] ADOPTS a value tamper on its second operand,
/// and the range proof's zero-test mask sits in exactly that slot, so a deviating party
/// can neutralise the range proof and make this function return a WRONG verdict instead
/// of aborting (witness:
/// `a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`). The MAC
/// layer here is a real but PARTIAL hardening over the semi-honest twin — it catches a
/// tampered OPEN, not an arbitrary malicious party. Confidentiality (only the verdict
/// bit is disclosed) is unaffected. See the module docs' "Integrity coverage is NOT
/// total" for the `MacSession`-level fix this needs.
///
/// **Review round 2 — why this function is no longer named `malicious_*`.** A caveat in
/// the docs is not containment: an API called `malicious_disclose_threshold_verdict` is
/// read as *the* integrity tier, and a drop-in with that name returning a wrong verdict
/// under its own named adversarial setting is a misrepresentation no doc comment fixes.
/// The defect is NOT in the sq-km34.6 Rabbit lift and reverting it would not remove the
/// break — `auth_secret_is_zero` is unchanged from the pre-lift `origin/main`, where
/// the masked-open `auth_verify_sum_in_range` fed it the same adopted second-operand
/// mask TWICE, so the previous production path was exploitable identically and at a
/// 40-bit-narrower magnitude. Nor can the composition be repaired inside this module:
/// [`crate::shamir::MacSession::mac_check`] can only cover values that are opened, so
/// binding both multiplication operands is a `MacSession` change (Chida-et-al. gate
/// verification). The containment that IS available today is the one applied here —
/// the name and the tier statement no longer claim a guarantee the code does not
/// deliver. Renaming is cheap and total: `sparq-mpc` is `publish = false` and nothing
/// in the workspace depends on it, so no caller is silently retargeted. `[SONNET-4.6]`
///
/// `sum_shares` is the degree-`t` sharing of the cumulative aggregate;
/// `public_threshold` is the public bar (e.g. £100k). [OPUS-4.8] sq-km34.6:
/// `public_threshold` must be `< 2^`[`RABBIT_VALUE_BITS`]` = 2^60` (fail-closed up
/// front) — the FULL width, at parity with the semi-honest path, because this now
/// routes through the AUTHENTICATED Rabbit decomposition rather than the 20-bit-capped
/// masked-open one. The SUM is range-checked in-protocol after its bit-decomposition
/// by the MAC-checked range proof (`auth_verify_value_in_range_rabbit`, whose zero-test
/// `v·r` open is MAC-checked too — sq-m4zi/sq-e7ma). On a tampered open the function
/// ABORTS with [`MpcError::MacCheckFailed`] (or the fail-closed non-boolean-verdict
/// guard) rather than returning a wrong verdict — but see the module docs' "Integrity
/// coverage is NOT total" residual for exactly which deviations that covers, and which
/// it demonstrably does not.
///
/// Honest-majority, **tamper-evident-with-abort on the opened values** (design §3 aims
/// at malicious-with-abort; the delivered tier is weaker). `[OPUS-4.8]` `[SONNET-4.6]`
pub fn experimental_tamper_evident_disclose_threshold_verdict(
    backend: &ShamirBackend,
    sum_shares: &[Share],
    public_threshold: u64,
) -> Result<PartialResult, MpcError> {
    let verdict =
        experimental_tamper_evident_threshold_over_sum(backend, sum_shares, public_threshold)?;
    Ok(PartialResult {
        holder: HolderId::new("federation"),
        vars: vec![oxrdf::Variable::new_unchecked("over_threshold")],
        rows: vec![vec![Some(oxrdf::Term::Literal(
            oxrdf::Literal::new_typed_literal(verdict.to_string(), oxrdf::vocab::xsd::BOOLEAN),
        ))]],
    })
}

/// The boolean core of [`experimental_tamper_evident_disclose_threshold_verdict`]: run
/// the full MAC-checked decompose + range-proof + compare chain over the existing
/// `[sum]` and return `sum > public_threshold`. Separated so tests can assert the boolean
/// directly (the `PartialResult` wrapper is the only difference from the public fn).
fn experimental_tamper_evident_threshold_over_sum(
    backend: &ShamirBackend,
    sum_shares: &[Share],
    public_threshold: u64,
) -> Result<bool, MpcError> {
    check_party_count(backend.parties(), backend.threshold())?;
    // Fail closed on the public threshold BEFORE any protocol work. [OPUS-4.8]
    // sq-km34.6 — the bound is now the FULL `2^RABBIT_VALUE_BITS = 2^60`, at parity
    // with the semi-honest `compare::disclose_threshold_verdict`: this routes through
    // the AUTHENTICATED Rabbit decomposition, which recovers the sum exactly through
    // the modular wrap and so carries no value/mask slack (the masked-open path's
    // `2^DECOMP_VALUE_BITS = 2^20` cap is gone).
    if public_threshold >= RABBIT_VALUE_MAX_EXCLUSIVE {
        return Err(MpcError::Protocol(format!(
            "experimental_tamper_evident_disclose_threshold_verdict: public_threshold = \
             {public_threshold} is out of \
             range (must be < 2^{RABBIT_VALUE_BITS} = {RABBIT_VALUE_MAX_EXCLUSIVE}; the \
             authenticated in-MPC Rabbit bit-decomposition recovers the secret sum through the \
             modular wrap, so with p = 2^61-1 the supported magnitude is the full \
             cleartext-operand width)"
        )));
    }

    let mut dealer = backend.dealer();
    let mut session = dealer.new_mac_session();
    let key = session.mac_key();

    // 1. Authenticate the EXISTING sum sharing (no reconstruction).
    let auth_sum = session.authenticate_existing(sum_shares)?;

    // 2. [OPUS-4.8] sq-km34.6 — AUTHENTICATED Rabbit bit-decomposition: every product
    //    and every degree-reduce in the wrap-recovery chain (solved bits, LTBits,
    //    bit-add, bit-sub) is a `MacSession::auth_mul`, and the single masked open
    //    `c = (sum + r) mod p` is MAC-checked before `c` is read. The recovered
    //    `auth_sum_bits[k]` is `[[a_k]]` (the k-th bit of the sum), MAC-carried.
    let auth_sum_bits = auth_bit_decompose_rabbit(&mut session, backend, &auth_sum, &key)?;

    // 3. IN-PROTOCOL range proof of the secret-shared sum, MAC-CHECKED end to end
    //    (sq-m4zi/sq-e7ma): the zero-test `v·r` open is routed through the §2.5 IT-MAC
    //    check over the AUTHENTICATED sum and bits, so a tampered zero-test open —
    //    which would otherwise let an out-of-range sum masquerade as in-range,
    //    bypassing the fail-closed guard — aborts with `MpcError::MacCheckFailed`
    //    rather than letting the verdict be derived from a corrupted decomposition.
    //    [SONNET-4.6] But NOT a tampered zero-test MASK: it sits in `auth_mul`'s adopted
    //    second operand slot, so zeroing it defeats this whole step and the pipeline
    //    returns a wrong verdict. See the module docs' "Integrity coverage is NOT total".
    auth_verify_value_in_range_rabbit(&mut session, backend, &auth_sum, &auth_sum_bits)?;

    // 4. Authenticated MSB-first comparison of the recovered sum bits against the
    //    PUBLIC threshold; the verdict is MAC-carried.
    let verdict =
        auth_greater_than_public_bits(&mut session, &auth_sum_bits, public_threshold, &key)?;

    // 5. MAC-CHECK the verdict BEFORE opening it (the third and final open).
    session.mac_check(std::slice::from_ref(&verdict))?;

    // 6. Open ONLY the (MAC-verified) verdict bit.
    open_auth_verdict(backend, &verdict)
}

/// Authenticated masked-open bit-decomposition of `[[sum]]` into `L =
/// `[`DECOMP_MASK_BITS`] AUTHENTICATED bits (LSB-first), the IT-MAC twin of
/// [`crate::compare`] `secure_bit_decompose`. Returns `(sum_bits, r_bits)` — both as
/// authenticated sharings.
///
/// The masked open `c = sum + r` is the SECOND of the three named opens; it is
/// MAC-checked before `c` is read. The mask bits come from the AUTHENTICATED square
/// protocol (whose `a²` opens are MAC-checked individually). The bitwise subtraction
/// `c ⊖ [[r]]` runs over the public `c` bits and the authenticated `[[r_k]]`, each
/// AND an [`MacSession::auth_mul`], so the recovered sum bits stay MAC-carried.
///
/// [OPUS-4.8] sq-km34.6 — now **TEST-ONLY**: superseded on the production
/// [`experimental_tamper_evident_disclose_threshold_verdict`] path by the authenticated Rabbit
/// [`auth_bit_decompose_rabbit`], which recovers the sum exactly through the modular
/// wrap (no statistical value/mask slack) and so supports the full
/// [`RABBIT_VALUE_BITS`] = 60 magnitude rather than this path's
/// [`DECOMP_VALUE_BITS`] = 20. Retained for the masked-open regression tests and as
/// the authenticated (tamper-evident) twin of the (likewise test-only) semi-honest
/// `crate::compare::secure_bit_decompose`.
#[cfg(test)]
fn auth_masked_bit_decompose(
    session: &mut MacSession,
    backend: &ShamirBackend,
    auth_sum: &AuthenticatedShare,
    key: &crate::authenticated::MacKey,
) -> Result<(Vec<AuthenticatedShare>, Vec<AuthenticatedShare>), MpcError> {
    // 1. Authenticated solved-bits: each [[r_k]] a square-protocol bit (MAC-checked
    //    a²), and [[r]] = Σ [[r_k]]·2^k as a FREE authenticated linear combination.
    let mut r_bits: Vec<AuthenticatedShare> = Vec::with_capacity(DECOMP_MASK_BITS);
    let mut auth_r = session.auth_const_sharing(Fp::zero());
    for k in 0..DECOMP_MASK_BITS {
        let bit = auth_square_protocol_random_bit(session, backend, key)?;
        auth_r = auth_add(&auth_r, &auth_scale(&bit, Fp::new(1u64 << k)))?;
        r_bits.push(bit);
    }

    // 2. Masked open c = sum + r, MAC-CHECKED before the public c is read.
    let auth_c = auth_add(auth_sum, &auth_r)?;
    session.mac_check(std::slice::from_ref(&auth_c))?;
    let c = backend.reconstruct(auth_c.value_shares())?.value();

    // 3. Authenticated bitwise subtraction [[a_k]] = c_k ⊖ [[r_k]] with an
    //    authenticated ripple borrow. a_k = c_k XOR r_k XOR borrow;
    //    borrow_out = (¬c_k ∧ r_k) ∨ (borrow ∧ ¬(c_k XOR r_k)).
    let mut out: Vec<AuthenticatedShare> = Vec::with_capacity(DECOMP_MASK_BITS);
    let mut borrow = session.auth_const_sharing(Fp::zero());
    #[allow(clippy::needless_range_loop)]
    for k in 0..DECOMP_MASK_BITS {
        let c_k = (c >> k) & 1;
        let r_k = &r_bits[k];
        // x = c_k XOR r_k (public XOR authenticated, FREE affine): c_k=1 → 1−[[r_k]].
        let x = if c_k == 1 {
            auth_add_constant(&auth_scale(r_k, Fp::one().neg()), Fp::one(), key)?
        } else {
            r_k.clone()
        };
        // a_k = x XOR borrow = x + borrow − 2·(x∧borrow) (one auth_mul).
        let x_and_borrow = session.auth_mul(&x, &borrow)?;
        let a_k = auth_sub(
            &auth_add(&x, &borrow)?,
            &auth_scale(&x_and_borrow, Fp::new(2)),
        )?;
        // borrow_out = (¬c_k ∧ r_k) ∨ (borrow ∧ ¬x).
        //   ¬c_k ∧ r_k : c_k public → c_k=1 ⇒ [[0]]; c_k=0 ⇒ [[r_k]].
        let nc_and_r = if c_k == 1 {
            session.auth_const_sharing(Fp::zero())
        } else {
            r_k.clone()
        };
        //   ¬x = 1 − x (free); borrow ∧ ¬x : one auth_mul; OR : one auth_mul.
        let not_x = auth_add_constant(&auth_scale(&x, Fp::one().neg()), Fp::one(), key)?;
        let borrow_and_notx = session.auth_mul(&borrow, &not_x)?;
        borrow = auth_or(session, &nc_and_r, &borrow_and_notx)?;
        out.push(a_k);
    }
    Ok((out, r_bits))
}

/// [OPUS-4.8] sq-m4zi/sq-e7ma — **MAC-checked in-protocol range proof of the
/// secret-shared sum**, the IT-MAC twin of the semi-honest
/// `crate::compare::verify_sum_in_range` (sq-nx0s; that masked-open reference is
/// now test-only after the sq-bgsn Rabbit lift on the semi-honest production path).
/// Given the AUTHENTICATED sum
/// `[[sum]]` and the `L = `[`DECOMP_MASK_BITS`] AUTHENTICATED bits `[[b_0..b_{L-1}]]`
/// that [`auth_masked_bit_decompose`] recovered, it PROVES — without reconstructing the
/// sum — that `sum ∈ [0, 2^`[`DECOMP_VALUE_BITS`]`)` via the SAME two secret zero-tests
/// the semi-honest proof runs, but now each zero-test's `v·r` open is MAC-checked:
///
/// 1. **recompose clause** (no field wrap / fits `L` bits): `sum − Σ_{k<L} b_k·2^k == 0`;
/// 2. **magnitude clause** (below the supported width): the high part
///    `Σ_{k≥`[`DECOMP_VALUE_BITS`]`} b_k·2^k == 0`.
///
/// Both `[[recompose_diff]]` and `[[high_part]]` are FREE authenticated linear
/// combinations of `[[sum]]` and the `[[b_k]]` (no `auth_mul`); each is then fed to the
/// MAC-checked zero-test [`auth_secret_is_zero`]. On violation it returns a fail-closed
/// [`MpcError::Protocol`] (an out-of-range / wrapping sum), and on a TAMPERED zero-test
/// open [`MpcError::MacCheckFailed`] — so a deviating party can no longer flip
/// "was it zero?" to smuggle an out-of-range sum past the guard. Honest-majority,
/// tamper-evident-with-abort (see the module security tier). `[OPUS-4.8]`
///
/// [OPUS-4.8] sq-km34.6 — now **TEST-ONLY**: the production path range-proves the
/// AUTHENTICATED Rabbit decomposition via [`auth_verify_value_in_range_rabbit`] (full
/// field width). Retained as the masked-open regression reference.
#[cfg(test)]
fn auth_verify_sum_in_range(
    session: &mut MacSession,
    backend: &ShamirBackend,
    auth_sum: &AuthenticatedShare,
    auth_sum_bits: &[AuthenticatedShare],
) -> Result<(), MpcError> {
    // Recompose [[Σ b_k·2^k]] over ALL L bits and SEPARATELY the high part (bits ≥
    // DECOMP_VALUE_BITS). Both are FREE authenticated linear combinations of the bit
    // sharings — no auth_mul — so the recomposition stays MAC-carried.
    let mut recomposed = session.auth_const_sharing(Fp::zero());
    let mut high_part = session.auth_const_sharing(Fp::zero());
    for (k, bit) in auth_sum_bits.iter().enumerate() {
        let weighted = auth_scale(bit, Fp::new(1u64 << k));
        recomposed = auth_add(&recomposed, &weighted)?;
        if k >= DECOMP_VALUE_BITS {
            high_part = auth_add(&high_part, &weighted)?;
        }
    }

    // Clause (1): no field wrap / fits L bits ⇔ [[sum − Σ b_k 2^k]] == 0.
    let recompose_diff = auth_sub(auth_sum, &recomposed)?;
    if !auth_secret_is_zero(session, backend, &recompose_diff)? {
        return Err(MpcError::Protocol(format!(
            "experimental_tamper_evident_disclose_threshold_verdict: in-protocol range \
             proof FAILED — the \
             secret-shared sum does not equal the bit-composition of its recovered \
             {DECOMP_MASK_BITS} bits (the masked open `sum + r` wrapped the field modulus \
             p = 2^61-1, or the sum has content above bit {DECOMP_MASK_BITS}). The verdict would \
             be derived from a corrupted decomposition, so it is REJECTED fail-closed rather than \
             returned wrong. The sum must be < 2^{DECOMP_VALUE_BITS} = {DECOMP_VALUE_MAX_EXCLUSIVE}."
        )));
    }

    // Clause (2): below the supported magnitude ⇔ all bits ≥ DECOMP_VALUE_BITS are
    // zero ⇔ [[high_part]] == 0.
    if !auth_secret_is_zero(session, backend, &high_part)? {
        return Err(MpcError::Protocol(format!(
            "experimental_tamper_evident_disclose_threshold_verdict: in-protocol range \
             proof FAILED — the \
             secret-shared sum is >= 2^{DECOMP_VALUE_BITS} = {DECOMP_VALUE_MAX_EXCLUSIVE} (a bit \
             at or above position {DECOMP_VALUE_BITS} is set). That exceeds the statistically-safe \
             magnitude the in-MPC masked bit-decomposition supports, so the verdict is REJECTED \
             fail-closed rather than returned wrong."
        )));
    }
    Ok(())
}

// =============================================================================
// [OPUS-4.8] sq-km34.6 — the AUTHENTICATED Rabbit chain (design §3 + Hole 4).
//
// The IT-MAC twin of `compare`'s sq-bgsn Rabbit path. The masked-open
// decomposition above caps the authenticated (tamper-evident) path's value at
// 2^DECOMP_VALUE_BITS = 2^20
// because its mask must be κ = 40 bits WIDER than the value AND `value + r` must not
// wrap p = 2^61−1. Rabbit removes the slack by RECOVERING the value exactly through
// the modular wrap (`x = c − r + w·p`, `w = 1{c < r}`), lifting the authenticated
// (tamper-evident) path to
// the full 2^RABBIT_VALUE_BITS = 2^60 — parity with the semi-honest path.
//
// The lift is where the malicious threat surface GROWS: the wrap recovery is a much
// DEEPER arithmetic circuit than the masked-open subtraction (a 61-bit LTBits, a
// 62-bit ripple-carry add, a 62-bit ripple-borrow sub — roughly 4x the gates), and
// every one of those gates is a Hole-1/Hole-2 tamper surface. So EVERY product and
// EVERY degree-reduce in the chain is a `MacSession::auth_mul` (the §2.4
// mult-then-reduce with the sound independent-reduce MAC carry), all linear ops carry
// the MAC for free, the single masked open is MAC-checked before `c` is read, and the
// boolean verdict is MAC-checked before it is opened.
// =============================================================================

/// `[[a ⊕ b]]` (logical XOR) of two AUTHENTICATED 0/1 sharings: `a + b − 2·a·b`. One
/// [`MacSession::auth_mul`] (the `a·b` term); the rest is FREE authenticated linear
/// ops. The IT-MAC twin of `crate::compare`'s `secret_xor`, used by the
/// authenticated Rabbit bit-add / bit-sub circuits. `[OPUS-4.8]`
fn auth_xor(
    session: &mut MacSession,
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
) -> Result<AuthenticatedShare, MpcError> {
    let ab = session.auth_mul(a, b)?;
    auth_sub(&auth_add(a, b)?, &auth_scale(&ab, Fp::new(2)))
}

/// A fresh full-field random mask `[[r]]` together with its `L = `[`RABBIT_MASK_BITS`]
/// AUTHENTICATED bits (LSB-first), `r ∈ [0, 2^L)` uniform and produced so NO party
/// knows it. The IT-MAC twin of `crate::compare`'s
/// `deal_full_field_solved_bits`: every bit is an [`auth_square_protocol_random_bit`]
/// (whose `a²` open is MAC-checked before it is read), and `[[r]] = Σ_k [[r_k]]·2^k`
/// is a FREE authenticated linear combination, so `[[r]]` and the `[[r_k]]` are
/// consistent — and MAC-consistent — by construction. `[OPUS-4.8]`
fn auth_deal_full_field_solved_bits(
    session: &mut MacSession,
    backend: &ShamirBackend,
    key: &crate::authenticated::MacKey,
) -> Result<(AuthenticatedShare, Vec<AuthenticatedShare>), MpcError> {
    let mut r_bits: Vec<AuthenticatedShare> = Vec::with_capacity(RABBIT_MASK_BITS);
    let mut auth_r = session.auth_const_sharing(Fp::zero());
    for k in 0..RABBIT_MASK_BITS {
        let bit = auth_square_protocol_random_bit(session, backend, key)?;
        auth_r = auth_add(&auth_r, &auth_scale(&bit, Fp::new(1u64 << k)))?;
        r_bits.push(bit);
    }
    Ok((auth_r, r_bits))
}

/// The Rabbit **`LTBits`** comparison of a PUBLIC integer `c` against the AUTHENTICATED
/// shared bits `[[r]]_B` (LSB-first), returning an authenticated sharing of the wrap
/// indicator `w = 1{c < r}`. The IT-MAC twin of `crate::compare`'s
/// `rabbit_lt_bits_public_less_than_shared`.
///
/// Same MSB-first "first differing bit decides" recurrence: scanning MSB→LSB, `c < r`
/// is decided by the first bit where `c_k = 0, r_k = 1` with all higher bits equal.
/// `c_k` is PUBLIC, so `(1 − c_k)·r_k` and the `(c_k == r_k)` map are FREE authenticated
/// affine ops; the only authenticated multiplications are the `eq`/`lt` chain steps.
/// Nothing is opened here — `[[w]]` stays secret and MAC-carried. `[OPUS-4.8]`
fn auth_rabbit_lt_bits_public_less_than_shared(
    session: &mut MacSession,
    c: u64,
    r_bits: &[AuthenticatedShare],
    key: &crate::authenticated::MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    let mut lt = session.auth_const_sharing(Fp::zero());
    let mut eq = session.auth_const_sharing(Fp::one());
    for k in (0..r_bits.len()).rev() {
        let c_k = (c >> k) & 1;
        let r_k = &r_bits[k];
        let eq_here = if c_k == 1 {
            // c_k=1 ⇒ the "c<r here" term (1−c_k)·r_k is the public constant 0, so
            // `lt` is unchanged — skip the wasted auth_mul. (c_k == r_k) == r_k.
            r_k.clone()
        } else {
            // c_k=0 ⇒ "c<r here" = eq · r_k IS a real authenticated multiplication.
            let term = session.auth_mul(&eq, r_k)?;
            lt = auth_add(&lt, &term)?;
            // (c_k == r_k) == 1 − r_k (free authenticated affine).
            auth_add_constant(&auth_scale(r_k, Fp::one().neg()), Fp::one(), key)?
        };
        eq = session.auth_mul(&eq, &eq_here)?;
    }
    Ok(lt)
}

/// Ripple-carry **bit ADD** of a PUBLIC `width`-bit integer `c` with `konst · [[w]]`
/// (`konst` public, `[[w]]` a single AUTHENTICATED bit), returning the `width`
/// LSB-first AUTHENTICATED result bits of `c + w·konst`. The IT-MAC twin of
/// `crate::compare`'s `rabbit_add_public_and_w_times_const`.
///
/// `konst_k · [[w]]` is a public bit times one authenticated sharing, so forming the
/// addend is FREE; the carry chain costs two authenticated multiplications per output
/// bit (the `x ∧ carry` term and the OR inside the majority). `[OPUS-4.8]`
fn auth_rabbit_add_public_and_w_times_const(
    session: &mut MacSession,
    c: u64,
    w: &AuthenticatedShare,
    konst: u64,
    width: usize,
    key: &crate::authenticated::MacKey,
) -> Result<Vec<AuthenticatedShare>, MpcError> {
    let zero = session.auth_const_sharing(Fp::zero());
    let mut out: Vec<AuthenticatedShare> = Vec::with_capacity(width);
    let mut carry = session.auth_const_sharing(Fp::zero()); // [[0]]
    for k in 0..width {
        let c_k = (c >> k) & 1;
        // addend_k = konst_k · [[w]] : konst_k public ⇒ FREE (the sharing or [[0]]).
        let addend_k = if (konst >> k) & 1 == 1 {
            w.clone()
        } else {
            zero.clone()
        };
        // x = c_k XOR addend_k (public XOR authenticated, FREE affine): c_k=1 ⇒ 1−addend_k.
        let x = if c_k == 1 {
            auth_add_constant(&auth_scale(&addend_k, Fp::one().neg()), Fp::one(), key)?
        } else {
            addend_k.clone()
        };
        // sum_k = x XOR carry = x + carry − 2·(x∧carry) (one auth_mul).
        let x_and_carry = session.auth_mul(&x, &carry)?;
        let sum_k = auth_sub(
            &auth_add(&x, &carry)?,
            &auth_scale(&x_and_carry, Fp::new(2)),
        )?;
        // carry_out = MAJ(c_k, addend_k, carry) = (c_k ∧ addend_k) ∨ (carry ∧ (c_k ⊕ addend_k)).
        // c_k public: c_k ∧ addend_k = c_k ? addend_k : [[0]]; the carry term is x ∧ carry.
        let ck_and_addend = if c_k == 1 {
            addend_k.clone()
        } else {
            zero.clone()
        };
        carry = auth_or(session, &ck_and_addend, &x_and_carry)?;
        out.push(sum_k);
    }
    Ok(out)
}

/// Ripple-borrow **bit SUB** `[[a]]_B = lhs_B ⊖ [[r]]_B` over two vectors of
/// AUTHENTICATED bits (LSB-first), returning the `width` LSB-first authenticated
/// difference bits. The IT-MAC twin of `crate::compare`'s
/// `rabbit_sub_shared_bits`: both operands are shared, so the result bit's XOR costs a
/// multiplication too — five authenticated multiplications per bit position, every one
/// of them MAC-carrying. Missing high positions read as the authenticated constant
/// `[[0]]`. `[OPUS-4.8]`
fn auth_rabbit_sub_shared_bits(
    session: &mut MacSession,
    lhs_bits: &[AuthenticatedShare],
    r_bits: &[AuthenticatedShare],
    width: usize,
    key: &crate::authenticated::MacKey,
) -> Result<Vec<AuthenticatedShare>, MpcError> {
    let zero = session.auth_const_sharing(Fp::zero());
    let mut out: Vec<AuthenticatedShare> = Vec::with_capacity(width);
    let mut borrow = session.auth_const_sharing(Fp::zero()); // [[0]]
    for k in 0..width {
        let l_k = lhs_bits.get(k).unwrap_or(&zero).clone();
        let r_k = r_bits.get(k).unwrap_or(&zero).clone();
        // x = l_k XOR r_k (both authenticated) — one auth_mul.
        let x = auth_xor(session, &l_k, &r_k)?;
        // a_k = x XOR borrow — one auth_mul.
        let a_k = auth_xor(session, &x, &borrow)?;
        // borrow_out = (¬l_k ∧ r_k) ∨ (¬x ∧ borrow). ¬l_k = 1 − l_k (free).
        let not_l = auth_add_constant(&auth_scale(&l_k, Fp::one().neg()), Fp::one(), key)?;
        let notl_and_r = session.auth_mul(&not_l, &r_k)?;
        let not_x = auth_add_constant(&auth_scale(&x, Fp::one().neg()), Fp::one(), key)?;
        let notx_and_borrow = session.auth_mul(&not_x, &borrow)?;
        borrow = auth_or(session, &notl_and_r, &notx_and_borrow)?;
        out.push(a_k);
    }
    Ok(out)
}

/// [OPUS-4.8] sq-km34.6 — **AUTHENTICATED Rabbit-style FULL-FIELD in-MPC
/// bit-decomposition** of an EXISTING authenticated sharing `[[x]]` into its
/// [`RABBIT_VALUE_BITS`] AUTHENTICATED bits (LSB-first), WITHOUT ever reconstructing
/// `x`. The IT-MAC twin of `crate::compare`'s
/// `secure_bit_decompose_rabbit` (sq-bgsn, eprint 2021/119), and the lift that brings
/// the IT-MAC disclose path to the semi-honest path's full `2^60` magnitude.
///
/// ## Protocol (and where the MAC sits at each step)
///
/// 1. Deal a fresh full-field solved-bits mask `([[r]], [[r]]_B)`,
///    `r ∈ [0, 2^`[`RABBIT_MASK_BITS`]`)` uniform, no party knows it
///    ([`auth_deal_full_field_solved_bits`]). Every mask bit comes from the
///    AUTHENTICATED square protocol, whose `a²` open is MAC-checked before it is read.
/// 2. **Open `c = (x + r) mod p`** — the ONLY opening in the decomposition, and it is
///    **MAC-checked before `c` is read** ([`MacSession::mac_check`], §2.5). `x` itself
///    is never opened; `c` is (near-)uniform over `[0, p)` (see "Hiding").
/// 3. **Exact wrap recovery.** Over the integers `x = c − r + w·p` with
///    `w = 1{c < r}` = [`auth_rabbit_lt_bits_public_less_than_shared`]`(c, [[r]]_B)`
///    (proof: in the wrap case `c = x + r − p` and `c < r ⇔ x < p`, always true; in
///    the no-wrap case `c = x + r ≥ r`). Then `[[t]]_B = c_B + w·p`
///    ([`auth_rabbit_add_public_and_w_times_const`]) and
///    `[[x]]_B = [[t]]_B ⊖ [[r]]_B` ([`auth_rabbit_sub_shared_bits`]). The arithmetic
///    is exact (`x < p < 2^61`, `c + w·p < 2p < 2^62`), so a `W = L + 1` working width
///    overflows nothing; the returned [`RABBIT_VALUE_BITS`] low bits are `x`'s bits.
///
/// ## MAC coverage of the chain (the sq-km34.6 obligation — stated precisely)
///
/// Steps 1 and 3 are a DEEP arithmetic circuit — every gate a place a malicious party
/// can inject an undetected offset by forging a product share (Hole 1) or re-sharing a
/// consistent degree-`t` codeword of the WRONG value inside `degree_reduce` (Hole 2),
/// neither detectable by Reed–Solomon at the minimal `n = 2t+1`. **Every** product in
/// the LTBits recurrence, the carry chain, the borrow chain and the XORs routes through
/// [`MacSession::auth_mul`], whose §2.4 independent-reduce carry keeps the output MAC
/// equal to `α·(true product)` when the VALUE reduce is tampered; the linear ops
/// (`1 − b`, `+`, public scaling) carry the MAC for FREE, so there is no
/// *unauthenticated* seam between the gates.
///
/// What that buys, concretely, is what the tests check: the single open is MAC-checked
/// before `c` is read; and a tamper on the wrap indicator `[[w]]`, or on ANY recovered
/// bit `[[b_k]]`, ABORTS the pipeline fail-closed — the latter because the caller's
/// range proof folds every `[[b_k]]` linearly into a value it then feeds to `auth_mul`
/// in the FIRST operand slot (see [`auth_verify_value_in_range_rabbit`]). That
/// abort-on-a-tampered-bit property rests ENTIRELY on the range proof, which the next
/// section shows a malicious party can switch off — so read the two together.
///
/// ## What the MAC-check does NOT cover (honest residual — this is NOT "any tamper aborts")
///
/// [`MacSession::auth_mul`] carries the MAC as `[α·z] = reduce([α·x]·[y])` — the FIRST
/// operand's MAC times the SECOND operand's VALUE. So a value tamper on the **second**
/// operand is *adopted*: the gate recomputes a MAC consistent with the tampered value,
/// and the batched check on that product passes although the product is WRONG. This is
/// a property of the sq-km34.3/.4 primitives (it applies equally to
/// [`crate::auth_compare`] and to the masked-open path in this module), and it cannot
/// be closed by checking more values here: [`MacSession::mac_check`] **opens** every
/// value it checks, so it can only ever cover values that are public anyway — never
/// secret intermediates. Closing it needs the Chida-et-al. verification of ALL
/// multiplication gate outputs, with `α` revealed only after the circuit is fixed so
/// that the check is linear and opens nothing — a change to `MacSession`, not to this
/// module. The residual is pinned by the witness test
/// `mac_check_adopts_a_second_operand_tamper_but_catches_a_first_operand_one`, which
/// goes red if the primitive is ever fixed. The design record's §2.5 "closes all four
/// holes" phrasing is therefore **stronger than what the code delivers today**.
///
/// **This is EXPLOITABLE on this path, not merely theoretical** [SONNET-4.6]:
/// [`auth_verify_value_in_range_rabbit`]'s zero-test hands its mask to `auth_mul` in
/// precisely the adopted second slot, so a zeroed mask neutralises the range proof — the
/// one check that catches a corrupted `[[b_k]]` — and the pipeline then returns the
/// adversary's verdict instead of aborting
/// (`a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`). The
/// "What that buys, concretely" paragraph above therefore holds ONLY against an
/// adversary that leaves the zero-test mask alone.
///
/// ## Hiding (stated honestly — `2^{-61}`, not perfect)
///
/// `r` is a sum of `L = 61` exactly-uniform bits, so it is uniform over `[0, 2^61)`,
/// ONE element wider than `p = 2^61 − 1`. After `mod p` the value `0` is hit by both
/// `r = 0` and `r = p`, giving `c`'s distribution statistical distance `≤ 1/p ≈ 2^{-61}`
/// from uniform — a cryptographic-strength gap, INDEPENDENT of the value's magnitude
/// (unlike the masked-open path's `2^{-40}` that was coupled to the 20-bit cap).
///
/// ## Magnitude bound (caller precondition)
///
/// Correct only while `x < 2^`[`RABBIT_VALUE_BITS`] so the recovered low bits capture
/// the whole value. `x` is a SHARING, so this cannot be checked here without disclosing
/// it; the caller proves it in-protocol via [`auth_verify_value_in_range_rabbit`].
/// Honest-majority, tamper-evident-with-abort (see the module security tier). `[OPUS-4.8]`
fn auth_bit_decompose_rabbit(
    session: &mut MacSession,
    backend: &ShamirBackend,
    auth_x: &AuthenticatedShare,
    key: &crate::authenticated::MacKey,
) -> Result<Vec<AuthenticatedShare>, MpcError> {
    // 1. Fresh AUTHENTICATED full-field solved bits (no party knows r; each bit's
    //    square-protocol `a²` open is MAC-checked inside).
    let (auth_r, auth_r_bits) = auth_deal_full_field_solved_bits(session, backend, key)?;

    // 2. Open c = (x + r) mod p — the ONLY opening — MAC-CHECKED BEFORE `c` is read,
    //    so the whole wrap-recovery circuit never runs on a corrupted `c`.
    let auth_c = auth_add(auth_x, &auth_r)?;
    session.mac_check(std::slice::from_ref(&auth_c))?;
    let c = backend.reconstruct(auth_c.value_shares())?.value();

    // 3a. Wrap indicator [[w]] = 1{c < r} via the authenticated public-vs-shared LTBits.
    let w = auth_rabbit_lt_bits_public_less_than_shared(session, c, &auth_r_bits, key)?;

    // 3b. [[t]]_B = c + w·p (public c, public p, authenticated single bit w); width
    //     L+1 holds c + w·p < 2p < 2^62 with no overflow.
    let width = RABBIT_MASK_BITS + 1;
    let t_bits =
        auth_rabbit_add_public_and_w_times_const(session, c, &w, crate::field::P, width, key)?;

    // 3c. [[x]]_B = [[t]]_B ⊖ [[r]]_B (both authenticated). The low RABBIT_VALUE_BITS
    //     bits are x's bits.
    let mut x_bits = auth_rabbit_sub_shared_bits(session, &t_bits, &auth_r_bits, width, key)?;
    x_bits.truncate(RABBIT_VALUE_BITS);
    Ok(x_bits)
}

/// [OPUS-4.8] sq-km34.6 — **MAC-checked in-protocol range proof for the AUTHENTICATED
/// Rabbit path**: PROVES `value ∈ [0, 2^`[`RABBIT_VALUE_BITS`]`)` from the recovered
/// authenticated bits WITHOUT reconstructing the value. The IT-MAC twin
/// of `crate::compare`'s `verify_value_in_range_rabbit`.
///
/// `[[Σ_k b_k·2^k]]` is a FREE authenticated linear combination of the recovered bits,
/// so the recomposition stays MAC-carried; the single clause `value − Σ b_k·2^k == 0`
/// is fed to the MAC-checked zero-test [`auth_secret_is_zero`]. Because the Rabbit
/// decomposition returns EXACTLY [`RABBIT_VALUE_BITS`] bits, a faithful recomposition
/// already lies in `[0, 2^RABBIT_VALUE_BITS)` — so this one zero-test IS the range
/// check (no separate high-part clause is needed, unlike the wider masked-open
/// decomposition's [`auth_verify_sum_in_range`]).
///
/// On violation it returns a fail-closed [`MpcError::Protocol`] (abort) rather than
/// feeding a truncated decomposition to the comparator; on a TAMPERED zero-test open it
/// returns [`MpcError::MacCheckFailed`]. Only the uniform-nonzero `v·r` mask product is
/// opened — the value is never reconstructed.
///
/// **⚠ This guard is NOT sound against a malicious party** [SONNET-4.6]: it is only as
/// strong as [`auth_secret_is_zero`], whose mask occupies [`MacSession::auth_mul`]'s
/// ADOPTED second operand slot. Forcing that mask to a sharing of `0` makes the
/// zero-test answer "yes, zero" for ANY `recompose_diff`, so an out-of-range sum — or an
/// arbitrarily corrupted decomposition — walks past this check with the MAC-check
/// passing. Witness:
/// `a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`. `[OPUS-4.8]`
fn auth_verify_value_in_range_rabbit(
    session: &mut MacSession,
    backend: &ShamirBackend,
    auth_value: &AuthenticatedShare,
    auth_value_bits: &[AuthenticatedShare],
) -> Result<(), MpcError> {
    // Recompose from the RABBIT_VALUE_BITS recovered bits — a FREE authenticated
    // linear combination (no auth_mul), so the recomposition stays MAC-carried.
    let mut recomposed = session.auth_const_sharing(Fp::zero());
    for (k, bit) in auth_value_bits.iter().enumerate() {
        recomposed = auth_add(&recomposed, &auth_scale(bit, Fp::new(1u64 << k)))?;
    }
    // No field wrap / fits the recovered window ⇔ [[value − Σ b_k 2^k]] == 0. Because
    // the decomposition returns exactly RABBIT_VALUE_BITS bits, a faithful
    // recomposition is itself < 2^RABBIT_VALUE_BITS — so this single MAC-checked
    // zero-test is EXACTLY value ∈ [0, 2^RABBIT_VALUE_BITS).
    let recompose_diff = auth_sub(auth_value, &recomposed)?;
    if !auth_secret_is_zero(session, backend, &recompose_diff)? {
        return Err(MpcError::Protocol(format!(
            "experimental_tamper_evident_disclose_threshold_verdict (Rabbit): in-protocol \
             range proof FAILED — the \
             secret-shared sum does not equal the bit-composition of its recovered \
             {RABBIT_VALUE_BITS} bits (the sum has content above bit {RABBIT_VALUE_BITS}, i.e. \
             sum >= 2^{RABBIT_VALUE_BITS} = {RABBIT_VALUE_MAX_EXCLUSIVE}). The verdict would be \
             derived from a truncated decomposition, so it is REJECTED fail-closed rather than \
             returned wrong."
        )));
    }
    Ok(())
}

/// [OPUS-4.8] sq-m4zi/sq-e7ma — **MAC-checked secret zero-test**: returns `true` iff the
/// AUTHENTICATED `[[v]]` reconstructs to `0`, WITHOUT reconstructing `v`, the
/// IT-MAC twin of [`crate::compare`] `secret_is_zero` (sq-nx0s). Draw a
/// fresh NONZERO authenticated mask `[[r]]`, form `[[m]] = [[v]]·[[r]]` via
/// [`MacSession::auth_mul`] (which carries the sound MAC `[α·m]` — design §2.4), then
/// **MAC-check `[[m]]` BEFORE its value open is read** ([`MacSession::mac_check`], §2.5),
/// and test `m == 0`.
///
/// Soundness of the disclosure (unchanged from the semi-honest twin): if `v == 0` then
/// `m == 0` regardless of `r`; if `v != 0` then `v·r` is a UNIFORM NONZERO field element
/// (nonzero times uniform nonzero), so the open of `m` reveals ONLY the single bit
/// "was `v` zero?" — nothing about `v`'s magnitude. The nonzero mask is load-bearing: a
/// zero mask would open `m = 0` even for `v != 0` (a false "in-range"). The MAC-check is
/// the new integrity layer: a tampered open of `m` (a wrong re-sharing producing a
/// consistent degree-`t` codeword of a DIFFERENT value, undetectable by Reed–Solomon at
/// `n = 2t+1`) makes `σ ≠ 0` and aborts with [`MpcError::MacCheckFailed`].
///
/// **⚠ A deviating party CAN still flip the zero-test result** [SONNET-4.6]: the mask
/// `[[r]]` is passed as [`MacSession::auth_mul`]'s SECOND operand, whose value tamper the
/// primitive ADOPTS (it recomputes `[α·m]` from the tampered value). Substituting a
/// consistent sharing of `0` for `[[r]]` therefore yields `m = 0` with a MATCHING MAC for
/// any `v`, and this function wrongly reports `v == 0` — the "nonzero mask is
/// load-bearing" note above is a statement about the HONEST mask only. Swapping the
/// operand order does not fix it (it merely moves the adopted slot onto `v`), and
/// zeroing BOTH operands defeats a both-orders check as well; the fix is
/// multiplication-gate verification in [`MacSession`] that binds both operands. Witness:
/// `a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`.
/// Honest-majority, tamper-evident-with-abort (NOT malicious-with-abort). `[OPUS-4.8]`
fn auth_secret_is_zero(
    session: &mut MacSession,
    backend: &ShamirBackend,
    v: &AuthenticatedShare,
) -> Result<bool, MpcError> {
    // Fresh NONZERO authenticated mask [[r]]. Nonzero is load-bearing: m = v·0 = 0
    // would falsely report v == 0. `authenticated_share` mints [[r]] = ([r], [α·r]).
    let mask_value = session.draw_nonzero_fp();
    let auth_mask = session.authenticated_share(mask_value);
    // [[m]] = [[v]]·[[r]] — a MAC-carrying multiplication (degree-t value product with
    // the SOUND independent-reduce MAC, design §2.4): a tampered value reduce makes
    // σ ≠ 0 at the check below.
    let auth_m = session.auth_mul(v, &auth_mask)?;
    // MAC-CHECK [[m]] BEFORE its value is opened — the §2.5 batched check is the sole
    // detector at the minimal n = 2t+1 (soundness ≈ 1 − 2^-61 from the secret α).
    session.mac_check(std::slice::from_ref(&auth_m))?;
    // Open the (MAC-verified) product and test m == 0 ⇔ v == 0. v itself is never
    // reconstructed; m is a uniform nonzero when v != 0 (leaks only "was it zero?").
    let m = backend.reconstruct(auth_m.value_shares())?;
    Ok(m == Fp::zero())
}

/// `[[a ∨ b]]` for authenticated 0/1 sharings: `a + b − a·b`. One [`MacSession::auth_mul`]
/// (the `a·b` term); the rest is FREE authenticated linear ops.
fn auth_or(
    session: &mut MacSession,
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
) -> Result<AuthenticatedShare, MpcError> {
    let ab = session.auth_mul(a, b)?;
    auth_sub(&auth_add(a, b)?, &ab)
}

/// A single AUTHENTICATED uniform random bit `[[b]] ∈ {0,1}` from the square protocol,
/// the IT-MAC twin of [`crate::compare`] `square_protocol_random_bit`. The
/// `c = a²` open (the FIRST of the three named opens) is **MAC-checked before it is
/// read**: a tampered open of `a²` makes `σ ≠ 0` and aborts, rather than silently
/// flipping the mask bit.
///
/// `[[a]]` is a fresh nonzero authenticated value; `[[a²]] = auth_mul([[a]],[[a]])`;
/// MAC-check `[[a²]]`; open `c = a²` (sign-independent, leaks nothing about the bit);
/// then `[[b]] = (d⁻¹·[[a]] + 1)·2⁻¹` as FREE authenticated affine maps so the bit
/// stays MAC-carried. `[OPUS-4.8]`
fn auth_square_protocol_random_bit(
    session: &mut MacSession,
    backend: &ShamirBackend,
    key: &crate::authenticated::MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    for _attempt in 0..64 {
        // 1. Fresh nonzero authenticated [[a]] — no party knows a; never opened.
        let a_value = session.draw_nonzero_fp();
        let auth_a = session.authenticated_share(a_value);
        // 2. [[a²]] via one auth_mul, then MAC-CHECK before opening c = a².
        let auth_a_sq = session.auth_mul(&auth_a, &auth_a)?;
        session.mac_check(std::slice::from_ref(&auth_a_sq))?;
        let c = backend.reconstruct(auth_a_sq.value_shares())?;
        // 3. c == 0 (a was 0; Pr 1/p) ⇒ retry with fresh [[a]].
        if c == Fp::zero() {
            continue;
        }
        // 4. Public square root d of c, re-checked fail-closed.
        let d = c.sqrt_residue();
        if d.mul(d) != c {
            return Err(MpcError::Protocol(format!(
                "authenticated square-protocol random bit: c = {} is not a quadratic residue \
                 (sqrt d = {}, d² = {} ≠ c) — the MAC-checked open of a² is inconsistent; \
                 refusing to emit a bit",
                c.value(),
                d.value(),
                d.mul(d).value()
            )));
        }
        // 5. [[s]] = d⁻¹·[[a]] ∈ {+1,−1}; [[b]] = (s+1)·2⁻¹ (free authenticated affine).
        let d_inv = d.inv();
        let s = auth_scale(&auth_a, d_inv);
        let two_inv = Fp::new(2).inv();
        let b = auth_scale(&auth_add_constant(&s, Fp::one(), key)?, two_inv);
        return Ok(b);
    }
    Err(MpcError::Protocol(
        "authenticated square-protocol random bit: drew a == 0 on every attempt (astronomically \
         improbable) — refusing to emit a bit"
            .into(),
    ))
}

/// MSB-first comparison of AUTHENTICATED secret bits `a_bits` (LSB-first) against a
/// PUBLIC value `pub_val`, returning the authenticated verdict `[[a > pub_val]]`. The
/// public bits are constants, so per-bit products against them collapse to FREE
/// authenticated affine maps; only the `eq`/`gt` chain spends authenticated
/// multiplications. Mirrors [`crate::auth_compare`] `auth_greater_than_public_bits`.
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
        let eq_here = if b_k == 1 {
            // a_k ∧ ¬1 = 0 ⇒ gt unchanged; (a_k == 1) == a_k.
            a_k.clone()
        } else {
            // b_k=0: a_k ∧ ¬0 = a_k ⇒ term = eq·a_k (one auth_mul).
            let term = session.auth_mul(&eq, a_k)?;
            gt = auth_add(&gt, &term)?;
            // (a_k == 0) == 1 − a_k.
            auth_add_constant(&auth_scale(a_k, Fp::one().neg()), Fp::one(), key)?
        };
        eq = session.auth_mul(&eq, &eq_here)?;
    }
    Ok(gt)
}

/// Open ONLY the verdict bit of an authenticated comparison result — the third named
/// open. The value sharing `[verdict]` is reconstructed (the MAC was consumed by the
/// check, never opened); a non-boolean reconstruction is refused rather than coerced.
/// This is the ONLY value this path opens besides the sign-independent `a²` and the
/// statistically-masked `sum + r`. `[OPUS-4.8]`
fn open_auth_verdict(
    backend: &ShamirBackend,
    verdict: &AuthenticatedShare,
) -> Result<bool, MpcError> {
    let bit = backend.reconstruct(verdict.value_shares())?;
    if bit == Fp::zero() {
        Ok(false)
    } else if bit == Fp::one() {
        Ok(true)
    } else {
        Err(MpcError::Protocol(format!(
            "experimental_tamper_evident_disclose_threshold_verdict: verdict reconstructed \
             to a non-boolean field \
             element {} (expected 0 or 1) — refusing to coerce",
            bit.value()
        )))
    }
}

#[cfg(test)]
mod tests {
    //! sq-6fv7 + sq-m4zi/sq-e7ma acceptance suite. The load-bearing ones:
    //! - `differential_*`: across many (sum, threshold) pairs incl. edges, the
    //!   MAC-checked verdict equals the plaintext `sum > threshold`.
    //! - `matches_semi_honest_disclose`: the authenticated (tamper-evident) path agrees
    //!   with the
    //!   semi-honest [`crate::compare::disclose_threshold_verdict`] verdict.
    //! - `tamper_in_*_is_caught`: a tampered open on each of the three named opens
    //!   (`a²`, `c=sum+r`, `verdict`) AND on the range-proof zero-test product
    //!   (`tamper_in_the_zero_test_open_is_caught`, sq-m4zi) aborts fail-closed.
    //! - `auth_secret_is_zero_matches_plaintext` / `auth_range_proof_*`: the
    //!   MAC-checked zero-test and range proof accept the in-range / zero cases and
    //!   reject the out-of-range / nonzero ones, agreeing with the plaintext.
    //! - `*_fails_closed`: out-of-range threshold / sum and n<2t+1 are descriptive
    //!   errors, never a silent wrong verdict.
    use super::*;
    use crate::compare::disclose_threshold_verdict;
    use crate::shamir::ShamirBackend;

    /// Deal a degree-`t` sharing of a cleartext sum, exactly as the federation
    /// aggregate would hand `disclose_threshold_verdict` its `[sum]`.
    fn share_sum(backend: &ShamirBackend, sum: u64) -> Vec<Share> {
        backend.dealer().share(Fp::new(sum))
    }

    /// `authenticate_existing` lifts a plain `[v]` into `[[v]] = ([v],[α·v])`: the
    /// value sharing is reused verbatim and the MAC sharing reconstructs to `α·v`
    /// (the relation the §2.5 check verifies). The cleartext `α` is read ONLY via the
    /// test-only accessor — production has no such path.
    #[test]
    fn authenticate_existing_carries_the_alpha_v_mac() {
        for &v in &[0u64, 1, 42, 100_000, DECOMP_VALUE_MAX_EXCLUSIVE - 1] {
            let backend = ShamirBackend::new_seeded(5, v.wrapping_add(3)).unwrap();
            let plain = share_sum(&backend, v);
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let auth = session.authenticate_existing(&plain).unwrap();
            // Value sharing is the caller's, unchanged.
            assert_eq!(
                backend.reconstruct(auth.value_shares()).unwrap(),
                Fp::new(v)
            );
            // MAC reconstructs to α·v (read α only via the test accessor).
            let alpha = session.alpha_for_test();
            assert_eq!(
                backend.reconstruct(auth.mac_shares()).unwrap(),
                alpha.mul(Fp::new(v)),
                "authenticate_existing MAC must equal α·v"
            );
            // And the honest authenticated value passes its MAC-check.
            session.mac_check(std::slice::from_ref(&auth)).unwrap();
        }
    }

    /// THE differential: the MAC-checked verdict equals the plaintext `sum > thr`
    /// across a spread of in-range sums and thresholds incl. edges, several
    /// honest-majority party counts.
    #[test]
    fn differential_malicious_disclose() {
        let cases: &[(u64, u64)] = &[
            (0, 0),
            (1, 0),
            (0, 1),
            (100_000, 100_000),
            (100_001, 100_000),
            (99_999, 100_000),
            (DECOMP_VALUE_MAX_EXCLUSIVE - 1, 0),
            (
                DECOMP_VALUE_MAX_EXCLUSIVE - 1,
                DECOMP_VALUE_MAX_EXCLUSIVE - 2,
            ),
            (42, 1000),
            (1000, 42),
        ];
        for n in [3usize, 5, 7] {
            for (idx, &(sum, thr)) in cases.iter().enumerate() {
                let backend = ShamirBackend::new_seeded(
                    n,
                    (idx as u64).wrapping_mul(131).wrapping_add(n as u64),
                )
                .unwrap();
                let shares = share_sum(&backend, sum);
                let got =
                    experimental_tamper_evident_threshold_over_sum(&backend, &shares, thr)
                        .unwrap();
                assert_eq!(
                    got,
                    sum > thr,
                    "n={n}: authenticated disclose verdict for ({sum} > {thr}) must match plaintext"
                );
            }
        }
    }

    /// The authenticated (tamper-evident) path agrees with the semi-honest
    /// `disclose_threshold_verdict` on
    /// the disclosed boolean (same verdict, just MAC-checked opens).
    #[test]
    fn matches_semi_honest_disclose() {
        for n in [3usize, 5] {
            for &(sum, thr) in &[
                (150_000u64, 100_000u64),
                (50_000, 100_000),
                (100_000, 100_000),
            ] {
                let backend = ShamirBackend::new_seeded(n, sum.wrapping_add(thr)).unwrap();
                let shares = share_sum(&backend, sum);
                let mal =
                    experimental_tamper_evident_threshold_over_sum(&backend, &shares, thr)
                        .unwrap();
                let semi = disclose_threshold_verdict(&backend, &shares, thr).unwrap();
                let semi_bool = semi.rows[0][0]
                    .as_ref()
                    .map(|t| t.to_string().contains("true"))
                    .unwrap_or(false);
                assert_eq!(
                    mal, semi_bool,
                    "n={n}: authenticated must agree with semi-honest"
                );
                assert_eq!(mal, sum > thr);
            }
        }
    }

    /// The public PartialResult surface carries ONLY the boolean.
    #[test]
    fn public_partial_result_carries_only_the_bool() {
        let backend = ShamirBackend::new_seeded(5, 7).unwrap();
        let shares = share_sum(&backend, 200_000);
        let pr =
            experimental_tamper_evident_disclose_threshold_verdict(&backend, &shares, 100_000)
                .unwrap();
        assert_eq!(pr.rows.len(), 1);
        assert_eq!(pr.rows[0].len(), 1);
        assert_eq!(pr.vars.len(), 1);
        assert!(pr.rows[0][0].as_ref().unwrap().to_string().contains("true"));
    }

    /// ACCEPTANCE (open 3): a tamper INTRODUCED at the verdict (a wrong re-sharing of
    /// the final comparison product would land here) is caught by the MAC-check.
    #[test]
    fn tamper_in_the_verdict_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0x5151).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let auth_sum = session
            .authenticate_existing(&backend.dealer().share(Fp::new(9)))
            .unwrap();
        let (bits, _r) =
            auth_masked_bit_decompose(&mut session, &backend, &auth_sum, &key).unwrap();
        let verdict = auth_greater_than_public_bits(&mut session, &bits, 4, &key).unwrap();
        let tampered = tamper_value(&verdict);
        assert!(
            matches!(
                session.mac_check(std::slice::from_ref(&tampered)),
                Err(MpcError::MacCheckFailed { .. })
            ),
            "a tampered verdict must make the MAC-check abort"
        );
    }

    /// ACCEPTANCE (open 1): a tamper on a square-protocol `[[a²]]` open is caught by
    /// the per-bit MAC-check — the bit is never emitted from a corrupted `a²` open.
    #[test]
    fn tamper_in_the_square_protocol_open_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0xA2A2).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let a = session.authenticated_share(Fp::new(7));
        let a_sq = session.auth_mul(&a, &a).unwrap();
        // Honest a² passes its MAC-check.
        session.mac_check(std::slice::from_ref(&a_sq)).unwrap();
        // A tampered a² open is caught (the same check the square protocol runs).
        let tampered = tamper_value(&a_sq);
        assert!(matches!(
            session.mac_check(std::slice::from_ref(&tampered)),
            Err(MpcError::MacCheckFailed { .. })
        ));
    }

    /// ACCEPTANCE (open 2): a tamper on the masked-open `[[sum+r]]` (a wrong re-share
    /// producing a consistent codeword of the wrong c) is caught by the MAC-check —
    /// so the bit-decomposition never runs on a corrupted c.
    #[test]
    fn tamper_in_the_masked_open_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0x5057).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let auth_sum = session
            .authenticate_existing(&backend.dealer().share(Fp::new(123)))
            .unwrap();
        // Build [[sum+r]] exactly as the decompose does, then tamper it before the open.
        let mut auth_r = session.auth_const_sharing(Fp::zero());
        for k in 0..DECOMP_MASK_BITS {
            let bit = auth_square_protocol_random_bit(&mut session, &backend, &key).unwrap();
            auth_r = auth_add(&auth_r, &auth_scale(&bit, Fp::new(1u64 << k))).unwrap();
        }
        let auth_c = auth_add(&auth_sum, &auth_r).unwrap();
        // Honest c passes.
        session.mac_check(std::slice::from_ref(&auth_c)).unwrap();
        // Tampered c is caught.
        let tampered = tamper_value(&auth_c);
        assert!(matches!(
            session.mac_check(std::slice::from_ref(&tampered)),
            Err(MpcError::MacCheckFailed { .. })
        ));
    }

    /// Out-of-range threshold/sum are descriptive errors, not silent wrong verdicts.
    /// [OPUS-4.8] sq-km34.6 — the bound is now the FULL `2^RABBIT_VALUE_BITS`, not the
    /// masked-open path's `2^DECOMP_VALUE_BITS`.
    #[test]
    fn fails_closed_on_bad_inputs() {
        let backend = ShamirBackend::new_seeded(5, 1).unwrap();
        let shares = share_sum(&backend, 100);
        // Threshold at/over 2^RABBIT_VALUE_BITS.
        assert!(matches!(
            experimental_tamper_evident_threshold_over_sum(
                &backend,
                &shares,
                RABBIT_VALUE_MAX_EXCLUSIVE
            ),
            Err(MpcError::Protocol(_))
        ));
        // An over-magnitude sum aborts via the in-protocol range proof.
        let big = share_sum(&backend, RABBIT_VALUE_MAX_EXCLUSIVE + 5);
        assert!(matches!(
            experimental_tamper_evident_threshold_over_sum(&backend, &big, 100_000),
            Err(MpcError::Protocol(_))
        ));
    }

    // ---- sq-m4zi/sq-e7ma: the MAC-checked range-proof zero-tests -----------------

    /// The MAC-checked zero-test [`auth_secret_is_zero`] agrees with the plaintext
    /// `v == 0` over a spread of values, several honest-majority party counts — and
    /// never reconstructs `v` (only the masked `m = v·r` is opened inside).
    #[test]
    fn auth_secret_is_zero_matches_plaintext() {
        for n in [3usize, 5, 7] {
            for &v in &[0u64, 1, 2, 42, 100_000, DECOMP_VALUE_MAX_EXCLUSIVE - 1] {
                let backend = ShamirBackend::new_seeded(n, v.wrapping_add(n as u64)).unwrap();
                let mut dealer = backend.dealer();
                let mut session = dealer.new_mac_session();
                let auth_v = session.authenticate_existing(&backend.dealer().share(Fp::new(v)));
                let auth_v = auth_v.unwrap();
                let is_zero = auth_secret_is_zero(&mut session, &backend, &auth_v).unwrap();
                assert_eq!(
                    is_zero,
                    v == 0,
                    "n={n}: MAC-checked zero-test of v={v} must match plaintext v==0"
                );
            }
        }
    }

    /// ACCEPTANCE (range-proof open, sq-m4zi): a tamper on the zero-test's `[[v·r]]`
    /// product open (the canonical Hole-2 consistent-codeword-of-the-wrong-value
    /// deviation that RS cannot detect at n=2t+1) is caught by the MAC-check — so a
    /// deviating party cannot flip "was it zero?" to smuggle an out-of-range sum past
    /// the fail-closed guard.
    #[test]
    fn tamper_in_the_zero_test_open_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0x2E20).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        // A nonzero v whose zero-test would HONESTLY report false; build [[v·r]]
        // exactly as the zero-test does, then tamper its value before the open.
        let auth_v = session
            .authenticate_existing(&backend.dealer().share(Fp::new(7)))
            .unwrap();
        let auth_mask = session.authenticated_share(Fp::new(3));
        let auth_m = session.auth_mul(&auth_v, &auth_mask).unwrap();
        // Honest m passes its MAC-check (the check the zero-test runs before the open).
        session.mac_check(std::slice::from_ref(&auth_m)).unwrap();
        // A tampered m open is caught — the zero-test can no longer be flipped.
        let tampered = tamper_value(&auth_m);
        assert!(
            matches!(
                session.mac_check(std::slice::from_ref(&tampered)),
                Err(MpcError::MacCheckFailed { .. })
            ),
            "a tampered zero-test product open must make the MAC-check abort"
        );
    }

    /// The MAC-checked range proof on the PRODUCTION path ACCEPTS in-range sums (incl.
    /// edges) and REJECTS out-of-range / field-wrapping sums fail-closed.
    /// [OPUS-4.8] sq-km34.6 — the supported width is now the FULL
    /// `2^RABBIT_VALUE_BITS`, so the accept set includes magnitudes the masked-open
    /// range proof rejected and the reject set starts at `2^60`, not `2^20`.
    #[test]
    fn auth_range_proof_accepts_in_range_and_rejects_out_of_range() {
        // In-range sums (incl. the top of the supported width, and magnitudes FAR
        // above the old masked-open cap) must be accepted by the full authenticated path.
        for &sum in &[
            0u64,
            1,
            100_000,
            DECOMP_VALUE_MAX_EXCLUSIVE,     // 2^20 — the old first-OOB, now in range
            1u64 << 40,                     // deep above the old cap
            RABBIT_VALUE_MAX_EXCLUSIVE - 1, // 2^60 - 1, the new max
        ] {
            let backend = ShamirBackend::new_seeded(5, sum.wrapping_add(11)).unwrap();
            let shares = share_sum(&backend, sum);
            assert!(
                experimental_tamper_evident_threshold_over_sum(&backend, &shares, 100_000).is_ok(),
                "in-range sum {sum} must pass the MAC-checked Rabbit range proof"
            );
        }
        // Out-of-range (>= 2^RABBIT_VALUE_BITS) sums must be REJECTED fail-closed by the
        // MAC-checked range proof (a descriptive Protocol error, never a silent wrong
        // verdict).
        for &sum in &[
            RABBIT_VALUE_MAX_EXCLUSIVE,
            RABBIT_VALUE_MAX_EXCLUSIVE + 1,
            RABBIT_VALUE_MAX_EXCLUSIVE + 12345,
        ] {
            let backend =
                ShamirBackend::new_seeded(5, sum.wrapping_mul(7).wrapping_add(3)).unwrap();
            let shares = share_sum(&backend, sum);
            assert!(
                matches!(
                    experimental_tamper_evident_threshold_over_sum(&backend, &shares, 100_000),
                    Err(MpcError::Protocol(_))
                ),
                "out-of-range sum {sum} must be rejected fail-closed by the range proof"
            );
        }
    }

    /// The superseded masked-open range proof [`auth_verify_sum_in_range`] (kept as the
    /// test-only reference after the sq-km34.6 Rabbit lift) still ACCEPTS in-range sums
    /// and REJECTS over-magnitude ones on its own `2^DECOMP_VALUE_BITS` width — the
    /// regression guard that the reference implementation has not rotted.
    #[test]
    fn masked_open_range_proof_reference_still_accepts_and_rejects() {
        for (sum, expect_ok) in [
            (0u64, true),
            (100_000u64, true),
            (DECOMP_VALUE_MAX_EXCLUSIVE - 1, true),
            (DECOMP_VALUE_MAX_EXCLUSIVE, false),
            (DECOMP_VALUE_MAX_EXCLUSIVE + 12345, false),
        ] {
            let backend = ShamirBackend::new_seeded(5, sum.wrapping_add(17)).unwrap();
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let key = session.mac_key();
            let auth_sum = session.authenticate_existing(&share_sum(&backend, sum)).unwrap();
            let (bits, _r) =
                auth_masked_bit_decompose(&mut session, &backend, &auth_sum, &key).unwrap();
            let got = auth_verify_sum_in_range(&mut session, &backend, &auth_sum, &bits);
            assert_eq!(
                got.is_ok(),
                expect_ok,
                "masked-open range proof on sum {sum}: expected ok={expect_ok}, got {got:?}"
            );
        }
    }

    // ---- sq-km34.6: the AUTHENTICATED Rabbit chain -------------------------------

    /// Reconstruct the plaintext integer from a vector of AUTHENTICATED bit sharings,
    /// asserting each reconstructs to a genuine 0/1. Test-only: production never opens
    /// these.
    fn recover_bits(backend: &ShamirBackend, bits: &[AuthenticatedShare]) -> u64 {
        let mut acc = 0u64;
        for (k, bit) in bits.iter().enumerate() {
            let b = backend.reconstruct(bit.value_shares()).unwrap();
            assert!(
                b == Fp::zero() || b == Fp::one(),
                "recovered bit[{k}] is not 0/1: {}",
                b.value()
            );
            if b == Fp::one() {
                acc |= 1u64 << k;
            }
        }
        acc
    }

    /// ACCEPTANCE (sq-km34.6, correctness): the AUTHENTICATED Rabbit decomposition
    /// recovers bits whose reconstruction equals the plaintext bits of the
    /// never-opened sum — across the FULL supported field width, including magnitudes
    /// the masked-open authenticated path (capped at `2^20`) could never decompose. Every
    /// product in that chain was a `MacSession::auth_mul`, and each recovered bit
    /// still passes its MAC-check (the value/MAC pair is consistent end to end).
    #[test]
    fn auth_rabbit_decompose_recovers_the_plaintext_bits() {
        let values: &[u64] = &[
            0,
            1,
            42,
            100_000,
            DECOMP_VALUE_MAX_EXCLUSIVE - 1,  // 2^20 - 1, the old max
            DECOMP_VALUE_MAX_EXCLUSIVE,      // 2^20, the old first-OOB
            1u64 << 30,
            1u64 << 45,
            1u64 << 59,                      // a high bit deep above the old cap
            RABBIT_VALUE_MAX_EXCLUSIVE - 1,  // 2^60 - 1, the new max
            (1u64 << 59) | (1u64 << 20) | 7, // mixed high+low bits
        ];
        for n in [3usize, 5] {
            for (idx, &v) in values.iter().enumerate() {
                let backend =
                    ShamirBackend::new_seeded(n, 90_000 + idx as u64 + n as u64).unwrap();
                let mut dealer = backend.dealer();
                let mut session = dealer.new_mac_session();
                let key = session.mac_key();
                let auth_v = session
                    .authenticate_existing(&share_sum(&backend, v))
                    .unwrap();
                let bits =
                    auth_bit_decompose_rabbit(&mut session, &backend, &auth_v, &key).unwrap();
                assert_eq!(bits.len(), RABBIT_VALUE_BITS, "n={n}: Rabbit bit count");
                assert_eq!(
                    recover_bits(&backend, &bits),
                    v,
                    "n={n}: authenticated Rabbit recovered the wrong value for {v}"
                );
                // The whole recovered vector is MAC-consistent — no gate left an
                // unauthenticated seam.
                session.mac_check(&bits).unwrap();
            }
        }
    }

    /// THE sq-km34.6 differential: the MAC-checked verdict equals the plaintext
    /// `sum > threshold` at magnitudes the masked-open authenticated path could NOT
    /// support (`>= 2^DECOMP_VALUE_BITS`), all the way to the new `2^60` ceiling —
    /// the headline lift, and the acceptance "secure verdict == plaintext".
    #[test]
    fn differential_malicious_disclose_rabbit_full_width() {
        let cases: &[(u64, u64)] = &[
            (DECOMP_VALUE_MAX_EXCLUSIVE, DECOMP_VALUE_MAX_EXCLUSIVE - 1), // just over the old cap
            (DECOMP_VALUE_MAX_EXCLUSIVE, DECOMP_VALUE_MAX_EXCLUSIVE),     // equal, over the old cap
            (1u64 << 30, 1_000_000),
            (1_000_000, 1u64 << 30),
            (1u64 << 45, (1u64 << 45) - 1),  // adjacent, high magnitude
            ((1u64 << 45) - 1, 1u64 << 45),  // adjacent, other side
            (1u64 << 59, (1u64 << 59) + 1),  // adjacent near the top
            (RABBIT_VALUE_MAX_EXCLUSIVE - 1, RABBIT_VALUE_MAX_EXCLUSIVE - 2), // max vs max-1
            (RABBIT_VALUE_MAX_EXCLUSIVE - 1, 0),
            (0, RABBIT_VALUE_MAX_EXCLUSIVE - 1),
        ];
        for n in [3usize, 5] {
            for (idx, &(sum, thr)) in cases.iter().enumerate() {
                let backend = ShamirBackend::new_seeded(
                    n,
                    (idx as u64).wrapping_mul(977).wrapping_add(n as u64),
                )
                .unwrap();
                let shares = share_sum(&backend, sum);
                let got =
                    experimental_tamper_evident_threshold_over_sum(&backend, &shares, thr)
                        .unwrap();
                assert_eq!(
                    got,
                    sum > thr,
                    "n={n}: authenticated Rabbit verdict for ({sum} > {thr}) must match plaintext"
                );
            }
        }
    }

    /// The authenticated (tamper-evident) Rabbit path agrees with the semi-honest Rabbit
    /// `disclose_threshold_verdict` at full-field magnitudes — same verdict, just
    /// MAC-checked opens. (The pre-sq-km34.6 authenticated path could not even be RUN on
    /// these inputs, so the two paths are now comparable over the same domain.)
    #[test]
    fn matches_semi_honest_disclose_at_full_width() {
        for &(sum, thr) in &[
            (1u64 << 40, 1u64 << 39),
            (1u64 << 39, 1u64 << 40),
            (RABBIT_VALUE_MAX_EXCLUSIVE - 1, 1u64 << 55),
        ] {
            let backend = ShamirBackend::new_seeded(5, sum.wrapping_add(thr)).unwrap();
            let shares = share_sum(&backend, sum);
            let mal =
                experimental_tamper_evident_threshold_over_sum(&backend, &shares, thr).unwrap();
            let semi = disclose_threshold_verdict(&backend, &shares, thr).unwrap();
            let semi_bool = semi.rows[0][0]
                .as_ref()
                .map(|t| t.to_string().contains("true"))
                .unwrap_or(false);
            assert_eq!(mal, semi_bool, "authenticated Rabbit must agree with semi-honest");
            assert_eq!(mal, sum > thr);
        }
    }

    /// ACCEPTANCE (sq-km34.6, "a tamper in ANY gate aborts"): a tamper on the Rabbit
    /// **masked open** `[[sum + r]]` — the ONE opening of the decomposition, and the
    /// canonical Hole-2 deviation (a consistent degree-`t` codeword of the WRONG `c`,
    /// which Reed–Solomon cannot detect at `n = 2t+1`) — is caught by the MAC-check
    /// that runs BEFORE `c` is read, so the whole wrap-recovery circuit never runs on
    /// a corrupted `c`.
    #[test]
    fn tamper_in_the_rabbit_masked_open_is_caught() {
        let backend = ShamirBackend::new_seeded(5, 0x2ABB).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let auth_sum = session
            .authenticate_existing(&share_sum(&backend, 1u64 << 40))
            .unwrap();
        let (auth_r, _r_bits) =
            auth_deal_full_field_solved_bits(&mut session, &backend, &key).unwrap();
        let auth_c = auth_add(&auth_sum, &auth_r).unwrap();
        // Honest c passes the check the decomposition runs before reading it.
        session.mac_check(std::slice::from_ref(&auth_c)).unwrap();
        // Tampered c is caught.
        let tampered = tamper_value(&auth_c);
        assert!(
            matches!(
                session.mac_check(std::slice::from_ref(&tampered)),
                Err(MpcError::MacCheckFailed { .. })
            ),
            "a tampered Rabbit masked open must make the MAC-check abort"
        );
    }

    /// Replay the PRODUCTION pipeline (`experimental_tamper_evident_threshold_over_sum`)
    /// with a tamper
    /// injected at `tamper` — a consistent degree-`t` codeword of a wrong value with
    /// the MAC untouched, the canonical Hole-2 deviation that Reed–Solomon cannot see
    /// at `n = 2t+1`. Returns whatever the pipeline returns, so the caller can assert
    /// it ABORTED rather than produced a verdict.
    fn rabbit_pipeline_with_tamper(
        seed: u64,
        sum: u64,
        threshold: u64,
        tamper: RabbitTamper,
    ) -> Result<bool, MpcError> {
        let backend = ShamirBackend::new_seeded(5, seed).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let auth_sum = session.authenticate_existing(&share_sum(&backend, sum))?;

        // --- auth_bit_decompose_rabbit, replayed so a tamper can be injected ---
        let (auth_r, r_bits) = auth_deal_full_field_solved_bits(&mut session, &backend, &key)?;
        let auth_c = auth_add(&auth_sum, &auth_r)?;
        session.mac_check(std::slice::from_ref(&auth_c))?;
        let c = backend.reconstruct(auth_c.value_shares())?.value();
        let mut w = auth_rabbit_lt_bits_public_less_than_shared(&mut session, c, &r_bits, &key)?;
        if tamper == RabbitTamper::WrapIndicator {
            w = tamper_value(&w);
        }
        let width = RABBIT_MASK_BITS + 1;
        let t_bits = auth_rabbit_add_public_and_w_times_const(
            &mut session,
            c,
            &w,
            crate::field::P,
            width,
            &key,
        )?;
        let mut bits = auth_rabbit_sub_shared_bits(&mut session, &t_bits, &r_bits, width, &key)?;
        bits.truncate(RABBIT_VALUE_BITS);
        match tamper {
            RabbitTamper::RecoveredBit(k) => bits[k] = tamper_value(&bits[k]),
            // `tamper_value` adds 1, so on an honestly-ZERO bit the corruption stays
            // BOOLEAN — the comparator downstream is well-formed and the verdict FLIPS,
            // rather than tripping the non-boolean-verdict guard.
            RabbitTamper::RaiseBitAndZeroTheZeroTestMask(k) => bits[k] = tamper_value(&bits[k]),
            _ => {}
        }

        // --- the production tail: range proof, comparator, MAC-check, open ---
        if let RabbitTamper::RaiseBitAndZeroTheZeroTestMask(_) = tamper {
            // The range proof replayed with the zero-test's MASK (the SECOND `auth_mul`
            // operand) forced to a consistent sharing of 0 — the composition the
            // production `auth_verify_value_in_range_rabbit` cannot defend against.
            if !zero_test_with_zeroed_mask(&mut session, &backend, &auth_sum, &bits)? {
                return Err(MpcError::Protocol("range proof rejected".into()));
            }
        } else {
            auth_verify_value_in_range_rabbit(&mut session, &backend, &auth_sum, &bits)?;
        }
        let verdict = auth_greater_than_public_bits(&mut session, &bits, threshold, &key)?;
        let opened = session.mac_check_and_open(std::slice::from_ref(&verdict))?;
        if opened[0] == Fp::zero() {
            Ok(false)
        } else if opened[0] == Fp::one() {
            Ok(true)
        } else {
            Err(MpcError::Protocol("non-boolean verdict".into()))
        }
    }

    /// Where [`rabbit_pipeline_with_tamper`] injects its deviation.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum RabbitTamper {
        /// No deviation — the honest control.
        None,
        /// The wrap indicator `[[w]] = 1{c < r}`, DEEP inside the chain: its output
        /// feeds every bit of `c + w·p` and hence every recovered sum bit.
        WrapIndicator,
        /// One recovered sum bit `[[b_k]]`, the output of the ripple-borrow chain.
        RecoveredBit(usize),
        /// [SONNET-4.6] The ADVERSARIAL COMPOSITION (review round 1): raise the
        /// honestly-zero recovered bit `k` to `1` (a still-BOOLEAN corruption, so the
        /// comparator downstream stays well-formed and the verdict actually FLIPS) AND
        /// force the range proof's zero-test MASK — which sits in `auth_mul`'s adopted
        /// SECOND operand slot — to a consistent sharing of `0`. The first tamper alone
        /// aborts ([`RabbitTamper::RecoveredBit`]); together they do not.
        RaiseBitAndZeroTheZeroTestMask(usize),
    }

    /// ACCEPTANCE (sq-km34.6): a tamper on the **wrap indicator** `[[w]] = 1{c < r}` —
    /// a gate the masked-open path does not even have, i.e. tamper surface NEWLY
    /// introduced by the Rabbit lift — aborts the pipeline fail-closed. `[[w]]` enters
    /// the ripple-carry add in the FIRST `auth_mul` operand slot, so its MAC defect
    /// propagates (rather than being adopted) into the recovered bits, where the range
    /// proof's zero-test catches it. The honest control run on the same inputs
    /// succeeds, so the abort is caused by the tamper and not by the harness.
    #[test]
    fn tamper_in_the_rabbit_wrap_indicator_aborts() {
        let (sum, thr) = (1u64 << 40, 100_000u64);
        // Control: the same pipeline with no deviation returns the correct verdict.
        assert_eq!(
            rabbit_pipeline_with_tamper(0x7ADD, sum, thr, RabbitTamper::None).unwrap(),
            sum > thr,
            "the honest control run must produce the plaintext verdict"
        );
        // Tampered: must ABORT, never return a verdict.
        let got = rabbit_pipeline_with_tamper(0x7ADD, sum, thr, RabbitTamper::WrapIndicator);
        assert!(
            got.is_err(),
            "a tampered Rabbit wrap indicator must abort fail-closed, got {got:?}"
        );
    }

    /// ACCEPTANCE (sq-km34.6): a tamper on ANY **recovered sum bit** — the output of
    /// the ripple-borrow chain, and the value the comparator reads — aborts the
    /// pipeline fail-closed at EVERY bit position tried.
    ///
    /// Why this is a structural guarantee and not luck: the in-protocol range proof
    /// forms `[[v]] = [[sum]] − Σ_k 2^k·[[b_k]]` as a FREE authenticated linear
    /// combination, so a defect in ANY `b_k` lands in `v` scaled by `2^k ≠ 0`; `v`
    /// then enters the zero-test's `auth_mul` in the **FIRST** operand slot, where a
    /// defect propagates into the product's MAC rather than being adopted by it. The
    /// range proof's MAC-check therefore fires before the comparator ever runs. This
    /// is why the range proof is load-bearing for INTEGRITY here, not only for the
    /// magnitude bound.
    ///
    /// [SONNET-4.6] **Scope of this guarantee:** it holds only while the range proof's
    /// own zero-test is intact. An adversary that ALSO zeroes that zero-test's mask
    /// switches the range proof off and this abort does not happen — see
    /// `a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`.
    #[test]
    fn tamper_in_any_recovered_rabbit_bit_aborts() {
        let sum = (1u64 << 45) | (1u64 << 3);
        let threshold = 1u64 << 44;
        // Control first: an honest run on these inputs returns the plaintext verdict.
        assert_eq!(
            rabbit_pipeline_with_tamper(0xB17, sum, threshold, RabbitTamper::None).unwrap(),
            sum > threshold
        );
        for k in [0usize, 3, 44, 45, 59] {
            let got =
                rabbit_pipeline_with_tamper(0xB17, sum, threshold, RabbitTamper::RecoveredBit(k));
            assert!(
                got.is_err(),
                "a tamper on recovered bit {k} must abort fail-closed, got {got:?}"
            );
        }
    }

    /// **RESIDUAL, pinned by a witness (honesty — see the module docs' "What the
    /// MAC-check does NOT cover").** [`MacSession::auth_mul`] carries the output MAC as
    /// `[α·z] = reduce([α·x]·[y])` — from the FIRST operand's MAC times the SECOND
    /// operand's VALUE. That asymmetry is what makes an in-reduce tamper on the first
    /// operand detectable, but it means a value tamper on the **second** operand is
    /// *adopted*: the gate recomputes a MAC consistent with the tampered value, so the
    /// batched check on the product passes even though the product is WRONG.
    ///
    /// This is a property of the sq-km34.3/.4 primitives, not of the Rabbit chain, and
    /// it applies equally to the already-landed [`crate::auth_compare`] and to the
    /// masked-open path in this module. It cannot be closed by checking more values
    /// here, because [`MacSession::mac_check`] **opens** every value it checks — so it
    /// can only ever cover values that are public anyway, never secret intermediates.
    ///
    /// This test is the WITNESS that keeps the limitation honest and tested: if a
    /// future `MacSession` closes the gap, this test goes red and the module docs (and
    /// the design record's §2.5 "closes all four holes" claim) must be revisited.
    #[test]
    fn mac_check_adopts_a_second_operand_tamper_but_catches_a_first_operand_one() {
        let backend = ShamirBackend::new_seeded(5, 0x5107).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let a = session.authenticated_share(Fp::new(3));
        let b = session.authenticated_share(Fp::new(5));
        // A consistent degree-t codeword of 6, with the MAC still committing to 5.
        let b_tampered = tamper_value(&b);

        // SECOND slot: the gate recomputes the MAC from the tampered value, so the
        // wrong product (3·6 = 18; honest answer 15) PASSES the check. The residual.
        let z_second = session.auth_mul(&a, &b_tampered).unwrap();
        assert_eq!(
            backend.reconstruct(z_second.value_shares()).unwrap(),
            Fp::new(18),
            "the second-slot tamper does change the product"
        );
        assert!(
            session.mac_check(std::slice::from_ref(&z_second)).is_ok(),
            "WITNESS: a second-operand tamper is currently ADOPTED by auth_mul's MAC \
             carry — if this now FAILS, the primitive was fixed and the honest residual \
             statements in this module's docs must be updated"
        );

        // FIRST slot: the same tamper is CAUGHT, because the output MAC is carried
        // from the tampered operand's (untampered) MAC and so no longer matches.
        let z_first = session.auth_mul(&b_tampered, &a).unwrap();
        assert_eq!(
            backend.reconstruct(z_first.value_shares()).unwrap(),
            Fp::new(18)
        );
        assert!(
            matches!(
                session.mac_check(std::slice::from_ref(&z_first)),
                Err(MpcError::MacCheckFailed { .. })
            ),
            "a first-operand tamper must be caught by the batched MAC-check"
        );
    }

    /// [SONNET-4.6] **THE EXPLOIT WITNESS (review round 1): the adopted second-operand
    /// tamper is NOT confined to an unrelated intermediate — it defeats the in-protocol
    /// range proof end to end, and the pipeline returns a WRONG verdict instead of
    /// aborting.**
    ///
    /// [`auth_secret_is_zero`] puts its supposedly-nonzero mask in
    /// [`MacSession::auth_mul`]'s SECOND operand slot — the slot whose value tamper the
    /// primitive adopts (pinned by
    /// [`mac_check_adopts_a_second_operand_tamper_but_catches_a_first_operand_one`]).
    /// Forcing that mask to a consistent sharing of `0` makes `m = v·0 = 0` for an
    /// arbitrarily nonzero `v`, with a MAC the gate recomputes to match, so the batched
    /// check passes and the zero-test answers "yes, zero". The range proof is the ONLY
    /// thing that catches a corrupted decomposition (see
    /// `tamper_in_any_recovered_rabbit_bit_aborts`), so with it neutralised a raised
    /// recovered bit sails through: this test raises bit 45 of a small sum and the
    /// pipeline reports `sum > 2^44` when the true sum is 8.
    ///
    /// This is why the module documents its tier as **NOT** "any tamper aborts": the
    /// final verdict MAC-check cannot recover, because it verifies a MAC that was
    /// adopted onto the wrong value upstream. Closing it needs multiplication-gate
    /// verification binding BOTH operands in [`MacSession`] (Chida-et-al. style), not a
    /// change in this module — when that lands, this test goes RED and the tier
    /// statements here, in `lib.rs`, and in `skills/mpc/SKILL.md` must be revisited.
    #[test]
    fn a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict() {
        let (sum, threshold) = (8u64, 1u64 << 44);
        // Control 1: the honest pipeline returns the plaintext verdict (8 > 2^44 = false).
        assert!(
            !rabbit_pipeline_with_tamper(0x2E80, sum, threshold, RabbitTamper::None).unwrap(),
            "the honest control must report the true verdict"
        );
        // Control 2: raising bit 45 ALONE aborts — the range proof does its job while
        // its zero-test mask is honest.
        assert!(
            rabbit_pipeline_with_tamper(0x2E80, sum, threshold, RabbitTamper::RecoveredBit(45))
                .is_err(),
            "with an honest zero-test mask, a raised recovered bit must abort"
        );
        // THE WITNESS: the same raised bit, composed with a zeroed zero-test mask, does
        // NOT abort — it returns the adversary's chosen verdict.
        let got = rabbit_pipeline_with_tamper(
            0x2E80,
            sum,
            threshold,
            RabbitTamper::RaiseBitAndZeroTheZeroTestMask(45),
        );
        assert_eq!(
            got,
            Ok(true),
            "WITNESS: zeroing the zero-test mask (auth_mul's ADOPTED second operand) \
             neutralises the range proof, so a raised recovered bit flips the verdict to \
             `true` for a true sum of {} — if this now ABORTS, `MacSession` gained \
             multiplication-gate verification that binds both operands and every \
             'tamper-evident, NOT malicious-with-abort' caveat in this crate must be \
             revisited",
            sum
        );
    }

    /// ACCEPTANCE (sq-km34.6, "the exact sum is never opened"). Two parts:
    ///
    /// 1. **Algebraic hiding of the one open.** The single masked open
    ///    `c = (sum + r) mod p`, with `r` uniform over `[0, 2^RABBIT_MASK_BITS)`, is
    ///    explainable by ANY other in-range sum under a legal alternative mask — so
    ///    observing `c` carries (near-)no information about which sum produced it.
    ///    Because the mask range `2^61` is one WIDER than `p = 2^61 − 1`, every
    ///    residue class has a legal representative, so the explaining mask always
    ///    exists. (The residual `2^{-61}` non-uniformity is the field-size floor,
    ///    documented on `auth_bit_decompose_rabbit`, not a tunable slack.)
    /// 2. **The disclosed surface is one bit.** Wildly different sums on the SAME side
    ///    of the threshold produce the SAME disclosed output — the surface cannot
    ///    distinguish them, so it carries the verdict and nothing else.
    #[test]
    fn the_rabbit_path_never_opens_the_exact_sum() {
        // (1) algebraic hiding of the single open.
        let p = crate::field::P as u128;
        let mask_hi = 1u128 << RABBIT_MASK_BITS;
        for &sum in &[0u64, 100_000, 1u64 << 40, RABBIT_VALUE_MAX_EXCLUSIVE - 1] {
            for &other in &[7u64, 1u64 << 55, RABBIT_VALUE_MAX_EXCLUSIVE - 1] {
                for &r in &[0u64, 1, 1u64 << 60, (1u64 << 61) - 1] {
                    let c = ((sum as u128 + r as u128) % p) as u64;
                    let r_prime = ((c as u128 + p - other as u128) % p) as u64;
                    assert!(
                        (r_prime as u128) < mask_hi,
                        "an explaining mask for sum={other} must be a legal draw"
                    );
                    assert_eq!(
                        ((other as u128 + r_prime as u128) % p) as u64,
                        c,
                        "the open c must be explainable by BOTH sums (hiding)"
                    );
                }
            }
        }
        // (2) the disclosed surface does not distinguish sums on the same side.
        let threshold = 1u64 << 40;
        for &(lo, hi) in &[
            ((1u64 << 40) + 1, RABBIT_VALUE_MAX_EXCLUSIVE - 1), // both ABOVE
            (0u64, 1u64 << 40),                                 // both AT-OR-BELOW
        ] {
            let b_lo = ShamirBackend::new_seeded(5, lo.wrapping_add(1)).unwrap();
            let b_hi = ShamirBackend::new_seeded(5, hi.wrapping_add(2)).unwrap();
            let v_lo = experimental_tamper_evident_disclose_threshold_verdict(
                &b_lo,
                &share_sum(&b_lo, lo),
                threshold,
            )
            .unwrap();
            let v_hi = experimental_tamper_evident_disclose_threshold_verdict(
                &b_hi,
                &share_sum(&b_hi, hi),
                threshold,
            )
            .unwrap();
            assert_eq!(
                v_lo.rows, v_hi.rows,
                "the disclosed surface must not distinguish {lo} from {hi} (same side of {threshold})"
            );
            assert_eq!(v_lo.vars, v_hi.vars);
        }
    }



    /// ACCEPTANCE (sq-km34.6, the MAC-check-BEFORE-open discipline, pinned so it
    /// cannot be silently deleted): the production `auth_bit_decompose_rabbit` spends
    /// EXACTLY `RABBIT_MASK_BITS + 1` batched `σ` opens — one per authenticated
    /// square-protocol mask bit (each MAC-checks its own `a²` before reading it), plus
    /// **one for the masked open `c = (sum + r) mod p`**, which is the check that must
    /// run before `c` is read.
    ///
    /// This is a mutation guard with teeth: the tamper tests replay the chain inline,
    /// so removing the `session.mac_check(&[auth_c])` line from the PRODUCTION function
    /// left every other test green. This one goes red, because the budget drops by one.
    /// It also states the amortisation honestly — the whole wrap-recovery circuit
    /// (LTBits + ripple-add + ripple-sub, several hundred authenticated
    /// multiplications) adds NO further `σ` opens, since nothing in it is opened.
    #[test]
    fn the_rabbit_decomposition_spends_exactly_one_sigma_open_per_open() {
        let backend = ShamirBackend::new_seeded(3, 5).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let auth_v = session
            .authenticate_existing(&share_sum(&backend, 12_345))
            .unwrap();
        let before = session.sigma_opens_for_test();
        let bits = auth_bit_decompose_rabbit(&mut session, &backend, &auth_v, &key).unwrap();
        let spent = session.sigma_opens_for_test() - before;
        assert_eq!(
            spent,
            RABBIT_MASK_BITS as u64 + 1,
            "the Rabbit decomposition must MAC-check every square-protocol a² ({RABBIT_MASK_BITS} \
             of them) AND the masked open c = sum + r (1) before reading them — a smaller budget \
             means a MAC-check-before-open was dropped"
        );
        // Sanity: the decomposition it produced is still the honest one.
        assert_eq!(recover_bits(&backend, &bits), 12_345);
    }

    // ---- tamper helper (test-only) ---------------------------------------------

    /// Corrupt the VALUE sharing into a CONSISTENT degree-`t` sharing of a DIFFERENT
    /// value (the canonical Hole-2 deviation: a valid codeword of the wrong value,
    /// which RS cannot detect — only the IT-MAC catches it), WITHOUT touching the MAC.
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

    /// [SONNET-4.6] Replay [`auth_verify_value_in_range_rabbit`]'s zero-test with the
    /// mask's VALUE sharing forced to a consistent sharing of `0` (its MAC `[α·r]`
    /// untouched — the canonical Hole-2 deviation) and return the zero-test's answer.
    ///
    /// The mask enters [`MacSession::auth_mul`] in the SECOND operand slot, whose value
    /// tamper the primitive ADOPTS: the gate recomputes `[α·m] = reduce([α·v]·[0]) = 0`
    /// to match the product `m = v·0 = 0`, so the batched check passes and the test
    /// reports "`v` is zero" for an arbitrarily nonzero `v`. Returns `Err` only if the
    /// MAC-check fires — which is exactly what the witness test asserts it does NOT.
    fn zero_test_with_zeroed_mask(
        session: &mut MacSession,
        backend: &ShamirBackend,
        auth_value: &AuthenticatedShare,
        auth_value_bits: &[AuthenticatedShare],
    ) -> Result<bool, MpcError> {
        let mut recomposed = session.auth_const_sharing(Fp::zero());
        for (k, bit) in auth_value_bits.iter().enumerate() {
            recomposed = auth_add(&recomposed, &auth_scale(bit, Fp::new(1u64 << k)))?;
        }
        let recompose_diff = auth_sub(auth_value, &recomposed)?;
        let mask_value = session.draw_nonzero_fp();
        let auth_mask = session.authenticated_share(mask_value);
        // Consistent degree-0 ⊆ t sharing of ZERO; the MAC still commits to `mask_value`.
        let zeroed: Vec<Share> = auth_mask
            .value_shares()
            .iter()
            .map(|s| Share {
                x: s.x,
                y: Fp::zero(),
            })
            .collect();
        let auth_mask =
            crate::authenticated::AuthenticatedShare::new(zeroed, auth_mask.mac_shares().to_vec())?;
        let auth_m = session.auth_mul(&recompose_diff, &auth_mask)?;
        session.mac_check(std::slice::from_ref(&auth_m))?;
        let m = backend.reconstruct(auth_m.value_shares())?;
        Ok(m == Fp::zero())
    }
}
