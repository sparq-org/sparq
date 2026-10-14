// sq-nuok: adversarial-share negative suite + a 'no fake crypto'
// stub gate for sparq-mpc. Written while Fable unavailable — re-review on return.
//! Adversarial / robustness negative suite for `sparq-mpc` (sq-nuok).
//!
//! The honest-path differential suite (in `shamir.rs` / `join.rs`) already pins
//! that the REAL parts compute the right answer. This module is its adversarial
//! complement, and it is deliberately *empirically honest*: it asserts only what
//! the code ACTUALLY guarantees, and where a guarantee is MISSING it PINS the
//! absence (so nobody mistakes the semi-honest scaffold for malicious security)
//! and the gap is filed as a bead.
//!
//! Four parts, mapping to the sq-nuok deliverable:
//!
//! 1. **Tampered-share behaviour** ([`tamper`]). Honest-majority Shamir is
//!    SEMI-HONEST: it has NO tamper detection (that is guarantee (D), malicious
//!    security — explicitly deferred, see `shamir.rs` "Security model"). These
//!    tests document the *actual* behaviour: a corrupted share silently changes
//!    the reconstructed value (it does NOT panic, and it does NOT silently
//!    return the TRUE secret — there is no notion of "correct" for it to claim).
//!    FINDING (bead **sq-uu0u**): no integrity check even when redundant shares
//!    are present — a single tampered share flips the result undetected. This is
//!    correct for the stated model but is pinned here so the absence is explicit.
//!
//! 2. **Threshold boundary** ([`threshold`]). The information-theoretic hiding
//!    property at EXACTLY the boundary: `t` shares cannot reconstruct (a Protocol
//!    error, AND the `t` shares are consistent with every candidate secret), and
//!    `t+1` shares do reconstruct — both directions, both sides of the boundary.
//!
//! 3. **Field-arithmetic fuzz** ([`field_fuzz`]). Random add / sub / mul / neg /
//!    inv over many seeds, cross-checked against a `u128`-modulo reference. With
//!    `p = 2^61 - 1` every product of two field elements is `< 2^122 < 2^128`, so
//!    `(a as u128 * b as u128) % (P as u128)` is an EXACT bignum-free reference
//!    for the prime (the crate is deliberately `num-bigint`-free; a `u128`
//!    reference is sound here, not a compromise — see `field.rs` "Choice of p").
//!
//! 4. **'No fake crypto' table** ([`no_fake_crypto`]). Modelled on
//!    `sparq-zk-compose/tests/forge_gates.rs`: a gate-indexed map enumerating
//!    every deferred / `NotYetImplemented` entry point and asserting each STAYS a
//!    typed error that names its gate — never `Ok` with a fabricated/plaintext
//!    value. This is what stops the scaffold being mistaken for finished crypto.

use crate::backend::{
    AbortKind, AdversaryModel, BackendInfo, CorruptionThreshold, MaliciousSecurity, MpcBackend,
    OperatorClass, OutputGuarantee, PublicVerifiability, SecurityDescriptor, TrustModel,
};
use crate::field::{Fp, P};
use crate::holder::Holder;
use crate::join::{DisclosedKeyJoin, GlobalJoin, HiddenValueJoin, JoinPlan};
use crate::partial::{HolderId, MpcError, PartialResult};
// The proof/attestation boundary types live in `crate::proof`.
use crate::proof::{Attestation, AttestationShare, CollaborativeProof, Proof, ProofStatement};
use crate::shamir::{
    self, add_shares, mul_shares_raw, reconstruct_degree, scale, ShamirBackend, Share,
};
use oxrdf::{Term, Variable};

fn fp(v: u64) -> Fp {
    Fp::new(v)
}

// =====================================================================
// PART 1 — Tampered / corrupted shares (the semi-honest reality).
// =====================================================================
mod tamper {
    use super::*;

    /// Corrupt one share's `y` value by adding a nonzero delta (a malicious /
    /// faulty party feeding a wrong share). `delta != 0` guarantees the value
    /// actually changes in the field.
    fn corrupt_one(shares: &mut [Share], idx: usize, delta: u64) {
        // (sq-qmth) stable-1.96 clippy `manual_is_multiple_of`.
        let nonzero = if delta.is_multiple_of(P) { 1 } else { delta };
        shares[idx].y = shares[idx].y.add(fp(nonzero));
    }

    /// At EXACTLY `t+1` shares (no redundancy), tampering one share changes the
    /// reconstructed secret. There is no possible detection here — `t+1` points
    /// determine SOME degree-`t` polynomial, just the wrong one. We pin that the
    /// scheme: (a) does NOT panic, (b) does NOT silently return the TRUE secret
    /// (no magic self-correction), i.e. the corruption propagates as designed.
    #[test]
    fn tampered_share_at_minimum_set_changes_secret_silently() {
        let backend = ShamirBackend::new_seeded(5, 0xABCD).unwrap();
        let t = backend.threshold(); // (5-1)/2 = 2
        let secret = fp(123_456);
        let shares = backend.dealer().share(secret);

        // Honest reconstruction from t+1 shares is correct.
        let honest = backend.reconstruct(&shares[..t + 1]).unwrap();
        assert_eq!(
            honest, secret,
            "sanity: honest t+1 reconstruction is correct"
        );

        // Tamper one of those t+1 shares.
        let mut tampered = shares[..t + 1].to_vec();
        corrupt_one(&mut tampered, 0, 9_999);

        // It still RETURNS a value (no panic), but a DIFFERENT one — and crucially
        // NOT the true secret. The scheme has no integrity check at this set size,
        // and that is the documented semi-honest behaviour.
        let got = backend
            .reconstruct(&tampered)
            .expect("Lagrange interpolation never errors on t+1 distinct points");
        assert_ne!(
            got, secret,
            "FINDING (sq-nuok pin → bead sq-uu0u): tampered share silently changes the reconstructed \
             secret — no tamper detection (semi-honest model; malicious security \
             deferred, guarantee (D))"
        );
    }

    /// **POSITIVE detection/correction test (sq-m34i, was the `…_is_still_
    /// undetected` pin).** With MAXIMAL redundancy (all `n` shares present), the
    /// consistency-checked / robust reconstruction now EXPLOITS the redundancy
    /// via Reed–Solomon (Berlekamp–Welch): it corrects up to
    /// `e = ⌊(n−t−1)/2⌋` tampered shares and returns the TRUE secret, and aborts
    /// with `MpcError::Tampered` when the tampering exceeds that budget. This was
    /// the gap pinned by bead sq-uu0u and is now CLOSED at the Shamir layer.
    #[test]
    fn tampered_share_with_full_redundancy_is_corrected_or_aborts() {
        // n = 7, t = 3 → e_max = ⌊(7−3−1)/2⌋ = 1: one error is CORRECTABLE.
        let backend = ShamirBackend::new_seeded(7, 0x5EED).unwrap();
        let t = backend.threshold();
        assert_eq!(t, 3);
        let secret = fp(2_024);
        let shares = backend.dealer().share(secret); // n = 7 shares

        assert_eq!(
            backend.reconstruct(&shares).unwrap(),
            secret,
            "honest full set ok (no behaviour change on clean input)"
        );

        // ONE tampered share (within budget e_max=1) → robustly CORRECTED to the
        // true secret. The redundancy that v1 wasted is now used.
        let mut one_bad = shares.clone();
        corrupt_one(&mut one_bad, 3, 1); // flip a single share by +1
        assert_eq!(
            backend.reconstruct(&one_bad).unwrap(),
            secret,
            "sq-m34i: a single tampered share is RS-corrected back to the true secret \
             (Berlekamp–Welch, n≥t+2e+1)"
        );

        // TWO tampered shares (> e_max=1) → cannot be corrected → detect-and-abort.
        let mut two_bad = shares.clone();
        corrupt_one(&mut two_bad, 3, 1);
        corrupt_one(&mut two_bad, 5, 2);
        let err = backend.reconstruct(&two_bad).unwrap_err();
        assert!(
            matches!(err, MpcError::Tampered { .. }),
            "sq-m34i: tampering beyond the correctable budget must ABORT with \
             MpcError::Tampered, never silently corrupt; got {err:?}"
        );
    }

    /// **POSITIVE detection test (sq-m34i).** At the minimal redundancy where no
    /// correction is possible but detection IS (`e_max = 0`, i.e. `n = t+2`), a
    /// single tampered share is DETECTED and the reconstruction aborts. This is
    /// the pure detect-and-abort regime (no correction budget).
    #[test]
    fn tampered_share_with_minimal_redundancy_is_detected_and_aborts() {
        // n = 3, t = 1 → e_max = ⌊(3−1−1)/2⌋ = 0: a single error is DETECTABLE
        // (abort) but not correctable.
        let backend = ShamirBackend::new_seeded(3, 0xD00D).unwrap();
        assert_eq!(backend.threshold(), 1);
        let secret = fp(777);
        let mut shares = backend.dealer().share(secret); // n = 3 > t+1 = 2

        assert_eq!(backend.reconstruct(&shares).unwrap(), secret, "honest ok");

        corrupt_one(&mut shares, 1, 9); // single tampered share
        let err = backend.reconstruct(&shares).unwrap_err();
        match err {
            MpcError::Tampered { cheaters, .. } => {
                // Detection is SOUND (we aborted). Attribution is best-effort:
                // with e_max=0 the honest set is unknown, so the reported off-
                // curve point(s) — relative to an arbitrary reference subset —
                // need not be the true cheater. We assert only that SOME suspect
                // is named (the message is actionable), per the documented
                // best-effort attribution contract.
                assert!(
                    !cheaters.is_empty(),
                    "sq-m34i: detect-and-abort should name at least one suspect point; got {cheaters:?}"
                );
            }
            other => panic!("sq-m34i: a tampered share with redundancy must abort, got {other:?}"),
        }
    }

    /// **POSITIVE detection test (sq-7q9i, WI-2; was the `…_flips_secure_
    /// equality_verdict_undetected` pin).** The secure-equality primitive
    /// (`HiddenValueJoin::secure_equal`) opens `m = d·r` at degree `2t` and
    /// reports `m == 0` as the match bit. We reproduce that EXACT primitive from
    /// the public `shamir` ops. WI-2 routes the degree-`2t` open through the WI-1
    /// Reed–Solomon checker, so when redundancy is present at degree `2t`
    /// (`n > 2t+1`) tampering one product share is now DETECTED rather than
    /// silently flipping the verdict.
    ///
    /// Redundancy at degree `2t` needs `n > 2t+1`. The honest-majority
    /// constructor fixes `t = ⌊(n−1)/2⌋`, so EVEN `n` gives `2t+1 = n−1 < n`
    /// (exactly one redundant share at degree `2t`, `e_max = 0` ⇒ detect-only).
    /// We use `n = 4, t = 1` (`2t = 2`, opened with all 4 shares ⇒ `4 > 3`).
    #[test]
    fn tampered_share_in_secure_equality_open_is_detected_when_redundant() {
        // n = 4, t = 1 → equality opens at degree 2t = 2 with all n=4 shares:
        // 4 > 2t+1 = 3 ⇒ one redundant share ⇒ detect-and-abort (e_max = 0).
        let backend = ShamirBackend::new_seeded(4, 0xEEEE).unwrap();
        let mut dealer = backend.dealer();
        let t = dealer.threshold();
        assert_eq!(t, 1);

        // Two EQUAL keys → honest verdict is "match" (m == 0).
        let key = fp(42);
        let sa = dealer.share(key);
        let sb = dealer.share(key);
        let mask = dealer.draw_nonzero_fp();
        let r = dealer.share(mask);
        let d = shamir::sub_shares(&sa, &sb).unwrap();
        let m_shares = mul_shares_raw(&d, &r).unwrap();
        assert_eq!(
            m_shares.len(),
            4,
            "all n=4 product shares opened (redundant)"
        );

        // Honest open: equal keys ⇒ m == 0 ⇒ match (no behaviour change clean).
        let m_honest = reconstruct_degree(&m_shares, 2 * t).unwrap();
        assert_eq!(
            m_honest,
            Fp::zero(),
            "sanity: equal keys open to m = 0 (match) on the checked path"
        );

        // Tamper ONE product share: with n > 2t+1 the RS check now DETECTS the
        // off-curve share and ABORTS instead of silently flipping the verdict.
        let mut tampered = m_shares.clone();
        tampered[0].y = tampered[0].y.add(fp(1));
        let err = reconstruct_degree(&tampered, 2 * t).unwrap_err();
        assert!(
            matches!(err, MpcError::Tampered { .. }),
            "sq-7q9i: a tampered degree-2t product share with redundancy (n > 2t+1) \
             must be DETECTED (MpcError::Tampered), not silently flip the match bit; got {err:?}"
        );
    }

    /// **NON-detection BOUNDARY pin (sq-7q9i, WI-2).** At EXACTLY the honest-
    /// majority minimum `n = 2t+1`, the degree-`2t` product open has ZERO RS
    /// redundancy (it is the minimal `2t+1` points that pin a unique degree-`2t`
    /// polynomial — analogous to the degree-`t` `t+1`-share no-redundancy case).
    /// So tampering one product share is information-theoretically UNDETECTABLE:
    /// it simply selects a different (wrong) degree-`2t` polynomial whose value at
    /// 0 flips the match verdict, with NO inconsistency to detect. WI-2 must NOT
    /// fake a guarantee here — the open must return the (flipped, wrong) value,
    /// never a false-positive [`MpcError::Tampered`]. This mirrors WI-1's exact-
    /// `t+1` non-detection pin. A true fix at `n = 2t+1` needs a MAC (deferred
    /// WI-4 / bead sq-6d6g).
    #[test]
    fn tampered_share_in_secure_equality_open_is_undetectable_at_n_eq_2t_plus_1() {
        // n = 5, t = 2 → equality opens at degree 2t = 4 with all n=5 shares:
        // n = 2t+1 = 5 ⇒ NO redundancy at degree 2t ⇒ tampering is undetectable.
        let backend = ShamirBackend::new_seeded(5, 0xEEEE).unwrap();
        let mut dealer = backend.dealer();
        let t = dealer.threshold();
        assert_eq!(t, 2);
        assert_eq!(dealer.parties(), 2 * t + 1, "exact honest-majority minimum");

        // Two EQUAL keys → honest verdict is "match" (m == 0).
        let key = fp(42);
        let sa = dealer.share(key);
        let sb = dealer.share(key);
        let mask = dealer.draw_nonzero_fp();
        let r = dealer.share(mask);
        let d = shamir::sub_shares(&sa, &sb).unwrap();
        let m_shares = mul_shares_raw(&d, &r).unwrap();
        assert_eq!(
            m_shares.len(),
            2 * t + 1,
            "exactly 2t+1 product shares (no redundancy)"
        );

        let m_honest = reconstruct_degree(&m_shares, 2 * t).unwrap();
        assert_eq!(
            m_honest,
            Fp::zero(),
            "sanity: equal keys open to m = 0 (match)"
        );

        // Tamper ONE product share at n = 2t+1: NO redundancy ⇒ the open must NOT
        // report Tampered (no false positive) and the verdict silently flips. This
        // is the honest boundary — RS cannot help; a MAC is required (WI-4).
        let mut tampered = m_shares.clone();
        tampered[0].y = tampered[0].y.add(fp(1));
        match reconstruct_degree(&tampered, 2 * t) {
            Ok(m_tampered) => assert_ne!(
                m_tampered,
                Fp::zero(),
                "sq-7q9i boundary: at n = 2t+1 a tampered product share flips the verdict \
                 (different valid degree-2t poly), so m must be nonzero — but NO detection"
            ),
            Err(MpcError::Tampered { .. }) => panic!(
                "sq-7q9i boundary FALSE POSITIVE: degree-2t open at n = 2t+1 has no RS \
                 redundancy, so tampering is undetectable and must NOT report Tampered \
                 (a MAC is the deferred WI-4 fix, not RS)"
            ),
            Err(other) => panic!("unexpected error at the n=2t+1 boundary: {other:?}"),
        }
    }

    /// Mismatched evaluation points (a party submitting a share on the WRONG `x`)
    /// in a LINEAR op (`add_shares`) IS caught — this is a real structural guard
    /// (not crypto integrity): the point-set check rejects it as a Protocol error.
    /// We assert the guard so the boundary between "checked structure" and
    /// "unchecked value integrity" is explicit.
    #[test]
    fn mismatched_points_in_linear_op_are_a_protocol_error() {
        let backend = ShamirBackend::new_seeded(4, 1).unwrap();
        let a = backend.dealer().share(fp(10));
        let mut b = backend.dealer().share(fp(20));
        // Corrupt b's evaluation point so the point-sets disagree.
        b[1].x += 1000;
        let err = add_shares(&a, &b).unwrap_err();
        assert!(
            matches!(err, MpcError::Protocol(_)),
            "a point-set mismatch must be a typed Protocol error, got {err:?}"
        );
    }

    /// **POSITIVE detection/correction test (sq-m34i, was the `…_corrupts_secure_
    /// sum_silently` pin).** A tampered share inside the SECURE SUM output
    /// (`run_secure`) is now caught by the same consistency-checked / robust
    /// reconstruction: with the available redundancy it is RS-corrected back to
    /// the true total (within `e_max`), and aborts when the tampering exceeds the
    /// correctable budget. The aggregate path is no longer silently corruptible.
    #[test]
    fn tampered_share_in_secure_sum_is_corrected_or_aborts() {
        // n = 4, t = 1 → e_max = ⌊(4−1−1)/2⌋ = 1: one error is CORRECTABLE.
        let backend = ShamirBackend::new_seeded(4, 0x5151).unwrap();
        let mut dealer = backend.dealer();
        let inputs: Vec<Vec<Share>> = [10u64, 20, 30]
            .iter()
            .map(|&v| dealer.share(fp(v)))
            .collect();
        let summed = backend.run_secure(&inputs).unwrap();
        assert_eq!(
            backend.reconstruct(&summed[0]).unwrap(),
            fp(60),
            "honest sum"
        );

        // ONE corrupted share of the summed sharing → RS-corrected to the truth.
        let mut one_bad = summed.clone();
        one_bad[0][0].y = one_bad[0][0].y.add(fp(7));
        assert_eq!(
            backend.reconstruct(&one_bad[0]).unwrap(),
            fp(60),
            "sq-m34i: a single tampered share in the aggregate is RS-corrected to the true sum"
        );

        // TWO corrupted shares (> e_max=1) → detect-and-abort, never a wrong total.
        let mut two_bad = summed.clone();
        two_bad[0][0].y = two_bad[0][0].y.add(fp(7));
        two_bad[0][2].y = two_bad[0][2].y.add(fp(11));
        let err = backend.reconstruct(&two_bad[0]).unwrap_err();
        assert!(
            matches!(err, MpcError::Tampered { .. }),
            "sq-m34i: tampering the aggregate beyond the correctable budget must ABORT \
             (MpcError::Tampered), not silently change the total; got {err:?}"
        );
    }

    /// **END-TO-END reconstruction hardening at the production API boundary
    /// (sq-uu0u).** The tests above exercise the RS checker via `reconstruct` /
    /// `reconstruct_degree`. This one drives the WHOLE public [`MpcBackend`]
    /// pipeline a federation actually calls — `run_secure` then
    /// `reconstruct_disclosed` (the disclosed-output trait method, NOT the helper)
    /// — and proves the RS hardening is wired all the way THROUGH that surface: a
    /// tampered share in the result sharing is (a) RS-corrected back to the true
    /// disclosed integer within the budget, and (b) detect-and-aborts with
    /// [`MpcError::Tampered`] beyond it — never a silently wrong disclosed answer.
    /// Honest runs still disclose the correct total (no behaviour change clean).
    #[test]
    fn tampered_share_at_reconstruct_disclosed_boundary_is_corrected_or_aborts() {
        // n = 4, t = 1 → e_max = ⌊(4−1−1)/2⌋ = 1: one error is CORRECTABLE, two abort.
        let backend = ShamirBackend::new_seeded(4, 0xD15C).unwrap();
        let mut dealer = backend.dealer();
        let inputs: Vec<Vec<Share>> = [30_000u64, 45_000, 28_000]
            .iter()
            .map(|&v| dealer.share(fp(v)))
            .collect();
        let plaintext_sum = 30_000u64 + 45_000 + 28_000;
        let summed = backend.run_secure(&inputs).unwrap();

        // Helper: pull the single disclosed integer out of the disclosed partial.
        let disclosed_int = |p: &PartialResult| -> u64 {
            assert_eq!(p.rows.len(), 1, "disclosed partial is exactly one row");
            match &p.rows[0][0] {
                Some(Term::Literal(l)) => l.value().parse::<u64>().unwrap(),
                other => panic!("expected integer literal, got {other:?}"),
            }
        };

        // HONEST: the disclosed-output method returns the true total.
        let honest = backend.reconstruct_disclosed(&summed).unwrap();
        assert_eq!(
            disclosed_int(&honest),
            plaintext_sum,
            "sanity: honest disclosed output is the true sum"
        );

        // ONE tampered result share (within e_max=1) → disclosed output is
        // RS-CORRECTED back to the true total at the public boundary.
        let mut one_bad = summed.clone();
        one_bad[0][1].y = one_bad[0][1].y.add(fp(13));
        let corrected = backend.reconstruct_disclosed(&one_bad).unwrap();
        assert_eq!(
            disclosed_int(&corrected),
            plaintext_sum,
            "sq-uu0u: a single tampered result share is RS-corrected through \
             reconstruct_disclosed — the relying party sees the TRUE total"
        );

        // TWO tampered result shares (> e_max=1) → reconstruct_disclosed ABORTS;
        // the relying party gets a typed error, never a fabricated disclosed value.
        let mut two_bad = summed;
        two_bad[0][1].y = two_bad[0][1].y.add(fp(13));
        two_bad[0][3].y = two_bad[0][3].y.add(fp(17));
        let err = backend.reconstruct_disclosed(&two_bad).unwrap_err();
        assert!(
            matches!(err, MpcError::Tampered { .. }),
            "sq-uu0u: tampering beyond the correctable budget must ABORT at the \
             reconstruct_disclosed boundary (MpcError::Tampered), never disclose a \
             silently-wrong total; got {err:?}"
        );
    }
}

// =====================================================================
// PART 1b — RS robust/checked reconstruction PROPERTY tests (sq-m34i).
// Random secret + random tamper patterns across the (n, t, e) regimes,
// plus a differential against honest Lagrange on untampered inputs.
// =====================================================================
mod robust_props {
    use super::*;
    use crate::robust::reconstruct_robust;
    use crate::shamir::reconstruct_at_zero;

    /// Deterministic SplitMix64 (reproducible; same shape as `field_fuzz::Lcg`).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A canonical field element in [0, P) via rejection (no modulo bias).
        fn next_fp(&mut self) -> u64 {
            loop {
                let c = self.next() & ((1u64 << 61) - 1);
                if c < P {
                    return c;
                }
            }
        }
        fn next_in(&mut self, hi: usize) -> usize {
            (self.next() % hi as u64) as usize
        }
    }

    /// Choose `k` distinct indices in `[0, n)`.
    fn distinct_indices(rng: &mut Lcg, n: usize, k: usize) -> Vec<usize> {
        let mut chosen: Vec<usize> = Vec::with_capacity(k);
        while chosen.len() < k {
            let i = rng.next_in(n);
            if !chosen.contains(&i) {
                chosen.push(i);
            }
        }
        chosen
    }

    /// Tamper exactly the chosen indices by a nonzero field delta.
    fn tamper(rng: &mut Lcg, shares: &mut [Share], idxs: &[usize]) {
        for &i in idxs {
            let mut d = rng.next_fp();
            if d == 0 {
                d = 1;
            }
            shares[i].y = shares[i].y.add(fp(d));
        }
    }

    /// PROPERTY: with `n ≥ t + 2e + 1` and up to `e` random tampered shares, the
    /// robust reconstruction returns the TRUE secret (Berlekamp–Welch corrects).
    #[test]
    fn robust_corrects_up_to_e_errors_returns_truth() {
        let mut rng = Lcg(0x00C0_DE15_F00D);
        // Use n with comfortable redundancy so e_max >= 1 for several t.
        for &n in &[4usize, 5, 7, 9] {
            let backend = ShamirBackend::new_seeded(n, 0x5A5A + n as u64).unwrap();
            let t = backend.threshold();
            let e_max = (n - t - 1) / 2;
            if e_max == 0 {
                continue; // no correction budget at this (n, t)
            }
            for _ in 0..200 {
                let secret = fp(rng.next_fp());
                let clean = backend.dealer().share(secret);
                // Tamper a random number of shares in 0..=e_max.
                let num_err = rng.next_in(e_max + 1); // 0..=e_max
                let mut shares = clean.clone();
                let idxs = distinct_indices(&mut rng, n, num_err);
                tamper(&mut rng, &mut shares, &idxs);
                let got = reconstruct_robust(&shares, t).unwrap_or_else(|e| {
                    panic!("n={n} t={t} e_max={e_max} num_err={num_err}: must correct, got {e:?}")
                });
                assert_eq!(
                    got, secret,
                    "n={n} t={t} e_max={e_max} num_err={num_err}: robust must return the truth"
                );
            }
        }
    }

    /// PROPERTY: when the number of tampered shares is in `(e_max, n−t−1]` — i.e.
    /// MORE than correctable but redundancy still present — the reconstruction
    /// ABORTS with `MpcError::Tampered` rather than returning a wrong secret.
    /// (`n−t−1` is the count of redundant shares; beyond it the codeword can be
    /// underdetermined, so we cap at the detectable band.)
    #[test]
    fn robust_aborts_when_errors_exceed_budget() {
        let mut rng = Lcg(0xDEAD_BEEF_F00D);
        for &n in &[3usize, 4, 5, 7, 9] {
            let backend = ShamirBackend::new_seeded(n, 0x1234 + n as u64).unwrap();
            let t = backend.threshold();
            let e_max = (n - t - 1) / 2;
            // # excess shares over the threshold; the detectable-but-not-
            // correctable band is (e_max, redundancy].
            let redundancy = n - t - 1;
            if redundancy <= e_max {
                continue; // no such band (e.g. nothing beyond correctable to test)
            }
            for _ in 0..200 {
                let secret = fp(rng.next_fp());
                let clean = backend.dealer().share(secret);
                // num_err strictly above e_max, up to `redundancy`.
                let span = redundancy - e_max; // >= 1
                let num_err = e_max + 1 + rng.next_in(span); // (e_max, redundancy]
                let mut shares = clean.clone();
                let idxs = distinct_indices(&mut rng, n, num_err);
                tamper(&mut rng, &mut shares, &idxs);
                match reconstruct_robust(&shares, t) {
                    Err(MpcError::Tampered { .. }) => { /* detected — correct */ }
                    Ok(v) => panic!(
                        "n={n} t={t} e_max={e_max} num_err={num_err}: must ABORT, \
                         but returned {v:?} (true secret was {secret:?})"
                    ),
                    Err(other) => {
                        panic!("n={n} num_err={num_err}: must be Tampered, got {other:?}")
                    }
                }
            }
        }
    }

    /// PROPERTY (documented non-detection): at EXACTLY `t+1` shares (no
    /// redundancy) a single tampered share is information-theoretically
    /// undetectable — robust reconstruction must NOT return `Tampered` and must
    /// simply interpolate (to a different, wrong secret). No false positives.
    #[test]
    fn exact_threshold_set_never_reports_tampered() {
        let mut rng = Lcg(0xFEED_FACE);
        for &n in &[2usize, 3, 5, 7, 9] {
            let backend = ShamirBackend::new_seeded(n, 0xABCD + n as u64).unwrap();
            let t = backend.threshold();
            for _ in 0..200 {
                let secret = fp(rng.next_fp());
                let clean = backend.dealer().share(secret);
                // Take exactly t+1 shares, then tamper one of them.
                let mut min_set = clean[..t + 1].to_vec();
                let i = rng.next_in(t + 1);
                let mut d = rng.next_fp();
                if d == 0 {
                    d = 1;
                }
                min_set[i].y = min_set[i].y.add(fp(d));
                match reconstruct_robust(&min_set, t) {
                    Ok(v) => assert_ne!(
                        v, secret,
                        "n={n} t={t}: t+1-share tamper must change the secret (no magic correction)"
                    ),
                    Err(MpcError::Tampered { .. }) => panic!(
                        "n={n} t={t}: FALSE POSITIVE — no redundancy means tampering is \
                         undetectable; must not report Tampered"
                    ),
                    Err(other) => panic!("n={n}: unexpected error {other:?}"),
                }
            }
        }
    }

    /// DIFFERENTIAL: on UNTAMPERED inputs the robust path returns EXACTLY the
    /// same value as the honest primitive `reconstruct_at_zero` (plain Lagrange)
    /// — no regression for the common, honest case, at every redundancy level.
    #[test]
    fn robust_matches_honest_lagrange_on_clean_inputs() {
        let mut rng = Lcg(0x000F_F1CE);
        for &n in &[2usize, 3, 4, 5, 7, 9] {
            let backend = ShamirBackend::new_seeded(n, 0x7E57 + n as u64).unwrap();
            let t = backend.threshold();
            for _ in 0..300 {
                let secret = fp(rng.next_fp());
                let shares = backend.dealer().share(secret);
                // Compare on every subset size from t+1 up to n.
                for take in (t + 1)..=n {
                    let subset = &shares[..take];
                    let honest = reconstruct_at_zero(subset, t).unwrap();
                    let robust = reconstruct_robust(subset, t).unwrap();
                    assert_eq!(
                        robust, honest,
                        "n={n} take={take}: robust must equal honest Lagrange on clean input"
                    );
                    assert_eq!(robust, secret, "and equal the true secret");
                }
            }
        }
    }

    // =================================================================
    // sq-7q9i (WI-2): degree-2t open property tests. The
    // equality/mult path opens m = d·r at degree 2t; these pin the
    // n>2t+1 (detect/correct) vs n=2t+1 (no detection) boundary at the
    // ACTUAL product degree, mirroring the degree-t props above.
    // =================================================================

    /// Build a degree-`degree` Shamir-style sharing of `secret` at points
    /// `x = 1..=n` directly (the production dealer only deals degree `t`). Free
    /// coefficients are drawn from the deterministic test RNG so the resulting
    /// codeword genuinely has the requested degree.
    ///
    /// sq-7q9i (WI-2): the LEADING (highest-index) coefficient is
    /// forced non-zero by rejection sampling. If it were left as a raw RNG draw
    /// it could be 0 with probability ~1/P, silently dropping the polynomial
    /// below the requested `degree`; that would hand the degree-`2t` boundary /
    /// correction-budget tests MORE RS redundancy than intended and weaken them.
    /// Resampling stays deterministic (it only consumes extra draws from the
    /// seeded test RNG). `degree >= 1` at every call site (always `2t`, `t >= 1`).
    fn share_degree(rng: &mut Lcg, secret: Fp, degree: usize, n: usize) -> Vec<Share> {
        let mut coeffs = Vec::with_capacity(degree + 1);
        coeffs.push(secret);
        // Free (middle) coefficients: zero is harmless for the degree.
        for _ in 0..degree.saturating_sub(1) {
            coeffs.push(fp(rng.next_fp()));
        }
        // Leading coefficient must be non-zero so the codeword is genuinely
        // degree `degree` (only relevant when `degree >= 1`).
        if degree >= 1 {
            let mut lead = rng.next_fp();
            while lead == 0 {
                lead = rng.next_fp();
            }
            coeffs.push(fp(lead));
        }
        (1..=n as u64)
            .map(|x| {
                // Horner eval of the degree-`degree` polynomial at x.
                let xf = fp(x);
                let mut acc = Fp::zero();
                for &c in coeffs.iter().rev() {
                    acc = acc.mul(xf).add(c);
                }
                Share { x, y: acc }
            })
            .collect()
    }

    /// Reproduce the EXACT secure-equality open (`m = d·r` at degree `2t`) for
    /// honest-majority party counts and assert the WI-2 boundary as a PROPERTY:
    /// whenever `n > 2t+1` (redundancy at degree `2t`) a single tampered product
    /// share is DETECTED (abort) — never a silently flipped verdict. Even `n`
    /// gives `2t+1 = n−1 < n` (one redundant share), which is the regime the
    /// honest-majority constructor actually reaches.
    #[test]
    fn equality_open_detects_tamper_whenever_redundant() {
        let mut rng = Lcg(0x2E0F_7A11);
        for &n in &[4usize, 6, 8] {
            let backend = ShamirBackend::new_seeded(n, 0x9E5 + n as u64).unwrap();
            let t = backend.threshold();
            // Honest-majority even-n invariant: redundancy at degree 2t is n - (2t+1).
            assert!(n > 2 * t + 1, "n={n} t={t}: expected degree-2t redundancy");
            for _ in 0..150 {
                let mut dealer = backend.dealer();
                // EQUAL keys ⇒ honest m = 0 (match). (Unequal keys would open to a
                // random nonzero m; either way one tampered share is off-curve.)
                let key = fp(rng.next_fp());
                let sa = dealer.share(key);
                let sb = dealer.share(key);
                let mask = dealer.draw_nonzero_fp();
                let r = dealer.share(mask);
                let d = shamir::sub_shares(&sa, &sb).unwrap();
                let m_shares = mul_shares_raw(&d, &r).unwrap();
                // Honest open is the match (no behaviour change on clean input).
                assert_eq!(
                    reconstruct_degree(&m_shares, 2 * t).unwrap(),
                    Fp::zero(),
                    "n={n}: honest equal-key open must be m = 0"
                );
                // Tamper one product share ⇒ detected (redundancy present).
                let mut tampered = m_shares.clone();
                let i = rng.next_in(n);
                let mut delta = rng.next_fp();
                if delta == 0 {
                    delta = 1;
                }
                tampered[i].y = tampered[i].y.add(fp(delta));
                match reconstruct_degree(&tampered, 2 * t) {
                    Err(MpcError::Tampered { .. }) => { /* detected — correct */ }
                    Ok(v) => panic!(
                        "n={n} t={t}: tampered degree-2t product share with redundancy must \
                         ABORT, got {v:?} (would silently flip the match verdict)"
                    ),
                    Err(other) => panic!("n={n}: must be Tampered, got {other:?}"),
                }
            }
        }
    }

    /// PROPERTY: the degree-`2t` open CORRECTS up to `e` tampered shares when the
    /// codeword carries enough redundancy (`n ≥ 2t + 2e + 1`). The honest-
    /// majority constructor (`t = ⌊(n−1)/2⌋`) never provisions this headroom for a
    /// degree-`2t` product, so we build the degree-`2t` sharing DIRECTLY at a
    /// chosen `n` to exercise the checker's correction arm at the product degree
    /// (the equality primitive itself only ever DETECTS, never corrects — that is
    /// the honest envelope, documented on `reconstruct_degree`).
    #[test]
    fn degree_2t_open_corrects_up_to_budget_when_overprovisioned() {
        let mut rng = Lcg(0x4C0F_FEE2);
        // (t, n) with n >= 2t + 3 so e_max = floor((n - 2t - 1)/2) >= 1 at deg 2t.
        for &(t, n) in &[(1usize, 5usize), (1, 7), (2, 7), (2, 9), (3, 11)] {
            let degree = 2 * t;
            let e_max = (n - degree - 1) / 2;
            assert!(e_max >= 1, "t={t} n={n}: expected a correction budget");
            for _ in 0..150 {
                let secret = fp(rng.next_fp());
                let clean = share_degree(&mut rng, secret, degree, n);
                // Honest open recovers the secret.
                assert_eq!(
                    reconstruct_degree(&clean, degree).unwrap(),
                    secret,
                    "t={t} n={n}: honest degree-2t open must recover the secret"
                );
                // Tamper up to e_max shares ⇒ corrected back to the truth.
                let num_err = rng.next_in(e_max + 1); // 0..=e_max
                let mut shares = clean.clone();
                let idxs = distinct_indices(&mut rng, n, num_err);
                tamper(&mut rng, &mut shares, &idxs);
                let got = reconstruct_degree(&shares, degree).unwrap_or_else(|e| {
                    panic!("t={t} n={n} e_max={e_max} num_err={num_err}: must correct, got {e:?}")
                });
                assert_eq!(
                    got, secret,
                    "t={t} n={n} num_err={num_err}: degree-2t open must correct to the truth"
                );
            }
        }
    }

    /// PROPERTY (documented non-detection at the boundary): at EXACTLY
    /// `n = 2t + 1` the degree-`2t` open has NO redundancy, so a single tampered
    /// product share is undetectable — the open must NOT report `Tampered` (no
    /// false positive) and simply interpolates to a different (wrong) value. This
    /// is the degree-`2t` analogue of `exact_threshold_set_never_reports_tampered`
    /// and pins the honest WI-2 boundary across the honest-majority odd-`n` counts
    /// (where `2t+1 = n`). A MAC is the deferred WI-4 fix here, not RS.
    #[test]
    fn degree_2t_open_never_reports_tampered_at_n_eq_2t_plus_1() {
        let mut rng = Lcg(0x2717_B0DE);
        for &n in &[3usize, 5, 7, 9] {
            let backend = ShamirBackend::new_seeded(n, 0xB0 + n as u64).unwrap();
            let t = backend.threshold();
            assert_eq!(
                n,
                2 * t + 1,
                "odd honest-majority n: degree-2t has no redundancy"
            );
            let degree = 2 * t;
            for _ in 0..150 {
                let secret = fp(rng.next_fp());
                // Exactly n = 2t+1 points of a degree-2t sharing (no redundancy).
                let mut shares = share_degree(&mut rng, secret, degree, n);
                let i = rng.next_in(n);
                let mut delta = rng.next_fp();
                if delta == 0 {
                    delta = 1;
                }
                shares[i].y = shares[i].y.add(fp(delta));
                match reconstruct_degree(&shares, degree) {
                    Ok(v) => assert_ne!(
                        v, secret,
                        "n={n} t={t}: degree-2t tamper at n=2t+1 must change the value \
                         (no magic correction)"
                    ),
                    Err(MpcError::Tampered { .. }) => panic!(
                        "n={n} t={t}: FALSE POSITIVE — degree-2t open at n=2t+1 has no \
                         redundancy; tampering is undetectable, must not report Tampered"
                    ),
                    Err(other) => panic!("n={n}: unexpected error {other:?}"),
                }
            }
        }
    }

    /// PROPERTY (bead **sq-ji5f**, decision: the honest envelope is DETECT-only).
    /// The honest-majority constructor fixes `t = ⌊(n−1)/2⌋`, so the equality /
    /// mult open at degree `2t` has correction budget
    /// `e_max = ⌊(n − (2t+1))/2⌋`. We sweep EVERY honest-majority party count the
    /// constructor will build and assert `e_max == 0` for all of them — odd `n`
    /// (`n = 2t+1`, zero redundancy → no detection) AND even `n` (`n = 2t+2`,
    /// exactly one redundant share → detect-and-abort). So the production
    /// equality primitive can at best DETECT (even `n`) and NEVER auto-CORRECTS a
    /// cheater: a flipped verdict is never silently repaired into the honest one.
    /// Robust correction (`e_max ≥ 1`) at degree `2t` would need `n ≥ 2t+3`, which
    /// the constructor never provisions for the equality open (see the
    /// `…_corrects_…_when_overprovisioned` prop, which builds the degree-`2t`
    /// sharing DIRECTLY at `n ≥ 2t+3` — no honest-majority `secure_equal` path
    /// reaches it).
    #[test]
    fn honest_majority_equality_open_has_zero_correction_budget() {
        for n in 2usize..=24 {
            let backend = ShamirBackend::new_seeded(n, 0xC0DE + n as u64).unwrap();
            let t = backend.threshold();
            assert_eq!(t, (n - 1) / 2, "honest-majority t = ⌊(n−1)/2⌋");
            // One multiplication needs the 2t+1 <= n headroom.
            assert!(n > 2 * t, "n={n} t={t}: equality open needs n >= 2t+1");
            let degree = 2 * t;
            // Redundant shares at the product degree, and the RS correction budget.
            let redundancy = n - (degree + 1);
            let e_max = redundancy / 2;
            assert_eq!(
                e_max, 0,
                "sq-ji5f: honest-majority n={n} (t={t}) must give ZERO correction \
                 budget at degree 2t (got e_max={e_max}); the equality open DETECTS \
                 at most, never auto-corrects"
            );
            // Cross-check the two reachable regimes the constructor produces.
            if n % 2 == 1 {
                assert_eq!(
                    redundancy, 0,
                    "odd honest-majority n={n}: n = 2t+1, zero redundancy at degree 2t \
                     (tampering undetectable — MAC is the deferred WI-4 fix, sq-6d6g)"
                );
            } else {
                assert_eq!(
                    redundancy, 1,
                    "even honest-majority n={n}: n = 2t+2, exactly one redundant share \
                     at degree 2t (detect-and-abort only, e_max=0)"
                );
            }
        }
    }

    /// PROPERTY (bead **sq-ji5f**): the FUNCTIONAL counterpart of the budget
    /// arithmetic above — on every even honest-majority `n` (the only regime with
    /// redundancy), a tampered degree-`2t` product share is DETECT-and-ABORT and
    /// is NEVER silently corrected back to the honest verdict. The reconstruction
    /// must therefore return `Err(Tampered)`, not `Ok(honest_value)`: with
    /// `e_max = 0` the RS checker has no correction budget, so "correct the
    /// cheater away" is provably unreachable on the honest-majority equality path.
    #[test]
    fn honest_majority_equality_open_never_corrects_tamper() {
        let mut rng = Lcg(0x5111_CE5A);
        for &n in &[4usize, 6, 8, 10] {
            let backend = ShamirBackend::new_seeded(n, 0xA17 + n as u64).unwrap();
            let t = backend.threshold();
            assert_eq!(
                n - (2 * t + 1),
                1,
                "even honest-majority n: one redundant share"
            );
            for _ in 0..120 {
                let mut dealer = backend.dealer();
                // EQUAL keys ⇒ honest m = 0 (a "match"); tampering tries to flip it.
                let key = fp(rng.next_fp());
                let sa = dealer.share(key);
                let sb = dealer.share(key);
                let mask = dealer.draw_nonzero_fp();
                let r = dealer.share(mask);
                let d = shamir::sub_shares(&sa, &sb).unwrap();
                let m_shares = mul_shares_raw(&d, &r).unwrap();
                let mut tampered = m_shares.clone();
                let i = rng.next_in(n);
                let mut delta = rng.next_fp();
                if delta == 0 {
                    delta = 1;
                }
                tampered[i].y = tampered[i].y.add(fp(delta));
                match reconstruct_degree(&tampered, 2 * t) {
                    Err(MpcError::Tampered { .. }) => { /* detect-and-abort — the honest envelope */
                    }
                    Ok(v) => panic!(
                        "sq-ji5f: n={n} t={t} has e_max=0, so the equality open must \
                         DETECT-and-ABORT a tampered share, never silently correct it \
                         (got Ok({v:?}))"
                    ),
                    Err(other) => panic!("n={n}: must be Tampered (detect-only), got {other:?}"),
                }
            }
        }
    }
}

// =====================================================================
// PART 2 — Threshold boundary, both sides (information-theoretic hiding).
// =====================================================================
mod threshold {
    use super::*;

    /// `t` shares (one BELOW the `t+1` reconstruction threshold) cannot
    /// reconstruct: it is a typed Protocol error, NOT a silent wrong/honest-
    /// looking value. This is the "fails closed below threshold" direction.
    #[test]
    fn exactly_threshold_shares_cannot_reconstruct() {
        // n = 7 → t = 3. Exactly t = 3 shares must be refused (need t+1 = 4).
        let backend = ShamirBackend::new_seeded(7, 11).unwrap();
        let t = backend.threshold();
        assert_eq!(t, 3);
        let shares = backend.dealer().share(fp(98_765));

        let err = backend.reconstruct(&shares[..t]).unwrap_err();
        match err {
            MpcError::Protocol(m) => {
                assert!(
                    m.contains("reconstruction needs"),
                    "should name the threshold: {m}"
                )
            }
            other => panic!("t shares must be a Protocol error, got {other:?}"),
        }
    }

    /// `t+1` shares (exactly AT the reconstruction threshold) DO reconstruct the
    /// true secret — the "succeeds at threshold" direction. Tested at several `n`.
    #[test]
    fn exactly_threshold_plus_one_shares_reconstruct() {
        for (n, seed) in [(3usize, 1u64), (5, 2), (7, 3), (9, 4)] {
            let backend = ShamirBackend::new_seeded(n, seed).unwrap();
            let t = backend.threshold();
            let secret = fp(31_337 + n as u64);
            let shares = backend.dealer().share(secret);
            // Take EXACTLY t+1 shares (the minimum that must succeed).
            let got = backend
                .reconstruct(&shares[..t + 1])
                .unwrap_or_else(|e| panic!("n={n}: t+1 shares must reconstruct, got {e:?}"));
            assert_eq!(got, secret, "n={n}: t+1 shares reconstruct the true secret");
        }
    }

    /// INFORMATION-THEORETIC HIDING at the boundary: a set of EXACTLY `t` shares
    /// is consistent with EVERY candidate secret. We prove it constructively —
    /// for two DIFFERENT target secrets we exhibit a `(t+1)`-th share that, with
    /// the same `t` shares, interpolates to each target. Hence the `t` shares
    /// alone determine nothing about the secret (the core privacy guarantee).
    #[test]
    fn t_shares_are_consistent_with_every_secret() {
        // n = 5 → t = 2. Take 2 shares; show they fit BOTH secret 1000 and 2000.
        let backend = ShamirBackend::new_seeded(5, 7).unwrap();
        let t = backend.threshold();
        assert_eq!(t, 2);
        let actual = fp(1000);
        let shares = backend.dealer().share(actual);
        let two = &shares[..t]; // exactly t shares

        for &target in &[fp(1000), fp(2000), fp(0), fp(P - 1)] {
            let forged = forge_completion(two, target);
            let mut combined = two.to_vec();
            combined.push(forged);
            // With the forged (t+1)-th share, the SAME t shares reconstruct to the
            // chosen target — so the t shares carry zero information about which.
            assert_eq!(
                reconstruct_degree(&combined, t).unwrap(),
                target,
                "t shares must be consistent with target {target:?} (hiding)"
            );
        }
    }

    /// Given `t` shares on a degree-`t` polynomial, produce a `(t+1)`-th share at
    /// a fresh `x` so the `t+1` points interpolate to `target` at `x = 0`. This is
    /// the constructive witness of information-theoretic hiding (the constant term
    /// is free until the `(t+1)`-th point is fixed). Test-only.
    fn forge_completion(shares: &[Share], target: Fp) -> Share {
        // Pick a fresh evaluation point distinct from the existing ones.
        let x_new = (1..=10_000u64)
            .find(|x| shares.iter().all(|s| s.x != *x))
            .expect("a fresh point exists");
        let mut xs: Vec<u64> = shares.iter().map(|s| s.x).collect();
        xs.push(x_new);
        // Lagrange weight at 0 for the i-th point of the (t+1)-point set.
        let weight = |i: usize| -> Fp {
            let xi = fp(xs[i]);
            let mut num = Fp::one();
            let mut den = Fp::one();
            for (j, &xj) in xs.iter().enumerate() {
                if i == j {
                    continue;
                }
                let xj = fp(xj);
                num = num.mul(xj.neg());
                den = den.mul(xi.sub(xj));
            }
            num.mul(den.inv())
        };
        // target = Σ_{i<t} y_i·L_i + y_new·L_new  ⇒  solve y_new.
        let last = shares.len(); // index of x_new in xs
        let mut acc = Fp::zero();
        for (i, s) in shares.iter().enumerate() {
            acc = acc.add(s.y.mul(weight(i)));
        }
        let l_new = weight(last);
        let y_new = target.sub(acc).mul(l_new.inv());
        Share { x: x_new, y: y_new }
    }

    /// Boundary symmetry across many `n`: `t` fails (typed error), `t+1` succeeds.
    /// One table covering both sides at once so the boundary is unambiguous.
    #[test]
    fn boundary_table_t_fails_t_plus_one_succeeds() {
        for (n, seed) in [(2usize, 10u64), (3, 11), (4, 12), (6, 13), (8, 14)] {
            let backend = ShamirBackend::new_seeded(n, seed).unwrap();
            let t = backend.threshold();
            let secret = fp(7 * n as u64 + 1);
            let shares = backend.dealer().share(secret);

            // BELOW threshold (t shares): refused. (For n=2, t=1 ⇒ 1 share fails.)
            assert!(
                backend.reconstruct(&shares[..t]).is_err(),
                "n={n}, t={t}: exactly t shares must NOT reconstruct"
            );
            // AT threshold (t+1 shares): succeeds with the true secret.
            assert_eq!(
                backend.reconstruct(&shares[..t + 1]).unwrap(),
                secret,
                "n={n}, t={t}: t+1 shares must reconstruct"
            );
        }
    }
}

// =====================================================================
// PART 3 — Field-arithmetic fuzz vs a u128-modulo reference.
// =====================================================================
mod field_fuzz {
    use super::*;

    /// Deterministic SplitMix64 (local to the fuzz; reproducible across runs).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A canonical field element in [0, P) via rejection (no modulo bias).
        fn next_fp(&mut self) -> u64 {
            loop {
                let c = self.next() & ((1u64 << 61) - 1);
                if c < P {
                    return c;
                }
            }
        }
    }

    const PP: u128 = P as u128;

    /// add / sub / neg / mul over many random pairs and seeds, each checked
    /// against the EXACT `u128`-modulo reference. With p = 2^61-1 the product of
    /// two field elements is < 2^122 < 2^128, so `(a*b) % P` in u128 is exact —
    /// a sound bignum-free reference for this prime (field.rs "Choice of p").
    #[test]
    fn add_sub_neg_mul_match_u128_reference_over_many_seeds() {
        for seed in 0..64u64 {
            let mut rng = Lcg(seed.wrapping_mul(0x100_0001).wrapping_add(1));
            for _ in 0..2_000 {
                let a = rng.next_fp();
                let b = rng.next_fp();
                let (fa, fb) = (fp(a), fp(b));

                assert_eq!(fa.add(fb).value() as u128, (a as u128 + b as u128) % PP);
                assert_eq!(
                    fa.sub(fb).value() as u128,
                    ((a as u128 + PP) - b as u128) % PP
                );
                assert_eq!(fa.neg().value() as u128, (PP - a as u128) % PP);
                assert_eq!(fa.mul(fb).value() as u128, (a as u128 * b as u128) % PP);
            }
        }
    }

    /// Multiplicative inverse: `a · a⁻¹ == 1` AND `a⁻¹` equals the reference
    /// `a^(p-2) mod p` computed independently in u128 (so we are not just trusting
    /// `Fp::pow` to validate `Fp::inv`). Over many random NONZERO elements.
    #[test]
    fn inverse_matches_independent_u128_modpow_reference() {
        // Independent reference modular exponentiation in u128 (square-and-multiply).
        fn modpow_u128(mut base: u128, mut exp: u128, m: u128) -> u128 {
            let mut acc = 1u128;
            base %= m;
            while exp > 0 {
                if exp & 1 == 1 {
                    acc = acc * base % m;
                }
                base = base * base % m;
                exp >>= 1;
            }
            acc
        }
        for seed in 0..32u64 {
            let mut rng = Lcg(seed.wrapping_mul(2_654_435_761).wrapping_add(1));
            for _ in 0..1_000 {
                let mut a = rng.next_fp();
                if a == 0 {
                    a = 1; // inv(0) is undefined / panics — skip the single zero.
                }
                let fa = fp(a);
                let inv = fa.inv();
                // a · a⁻¹ = 1.
                assert_eq!(fa.mul(inv), Fp::one(), "a={a}: a·inv(a) must be 1");
                // a⁻¹ = a^(p-2) mod p, computed independently in u128.
                let reference = modpow_u128(a as u128, (P - 2) as u128, PP);
                assert_eq!(
                    inv.value() as u128,
                    reference,
                    "a={a}: inv mismatch vs u128 modpow"
                );
            }
        }
    }

    /// Field LAWS under fuzz: associativity/commutativity of + and ·, the
    /// distributive law, and additive/multiplicative identities — over random
    /// triples. A reduction bug (e.g. a bad Mersenne fold) breaks at least one.
    #[test]
    fn field_axioms_hold_under_fuzz() {
        let mut rng = Lcg(0xF1E1D);
        for _ in 0..50_000 {
            let a = fp(rng.next_fp());
            let b = fp(rng.next_fp());
            let c = fp(rng.next_fp());

            assert_eq!(a.add(b), b.add(a), "+ commutes");
            assert_eq!(a.mul(b), b.mul(a), "· commutes");
            assert_eq!(a.add(b).add(c), a.add(b.add(c)), "+ associates");
            assert_eq!(a.mul(b).mul(c), a.mul(b.mul(c)), "· associates");
            assert_eq!(
                a.mul(b.add(c)),
                a.mul(b).add(a.mul(c)),
                "· distributes over +"
            );
            assert_eq!(a.add(Fp::zero()), a, "0 is additive identity");
            assert_eq!(a.mul(Fp::one()), a, "1 is multiplicative identity");
            assert_eq!(a.add(a.neg()), Fp::zero(), "neg is additive inverse");
        }
    }

    /// Edge operands around the modulus: 0, 1, P-1, P-2, and small values — the
    /// reduction's conditional-subtraction branches. Checked vs u128 reference.
    #[test]
    fn boundary_operands_match_reference() {
        let edges = [0u64, 1, 2, 3, P - 1, P - 2, P - 3, 1 << 30, 1 << 60];
        for &a in &edges {
            for &b in &edges {
                let (fa, fb) = (fp(a), fp(b));
                assert_eq!(
                    fa.add(fb).value() as u128,
                    (a as u128 + b as u128) % PP,
                    "add {a}+{b}"
                );
                assert_eq!(
                    fa.mul(fb).value() as u128,
                    (a as u128 * b as u128) % PP,
                    "mul {a}*{b}"
                );
                assert_eq!(
                    fa.sub(fb).value() as u128,
                    ((a as u128 + PP) - b as u128) % PP,
                    "sub {a}-{b}"
                );
            }
        }
    }

    /// `scale` (public share op) is exactly field multiplication applied per
    /// share — fuzz it against the reference so the share-arithmetic helper, not
    /// just `Fp`, is covered.
    #[test]
    fn scale_share_op_matches_field_mul() {
        let backend = ShamirBackend::new_seeded(4, 0x5CA1E).unwrap();
        let mut rng = Lcg(0xC0FFEE);
        for _ in 0..500 {
            let secret = rng.next_fp();
            let factor = rng.next_fp();
            let shares = backend.dealer().share(fp(secret));
            let scaled = scale(&shares, fp(factor));
            let got = backend.reconstruct(&scaled).unwrap();
            let expect = (secret as u128 * factor as u128) % PP;
            assert_eq!(got.value() as u128, expect, "scale({secret})·{factor}");
        }
    }
}

// =====================================================================
// PART 4 — 'No fake crypto' table: every deferred entry point STAYS typed.
// Modelled on sparq-zk-compose/tests/forge_gates.rs (one gate per row).
// =====================================================================
mod no_fake_crypto {
    use super::*;

    /// Assert an `MpcError` is the deferred-stub channel AND names a gate. Every
    /// deferred path must carry its gate in the VALUE (not just a doc-comment) so
    /// the deferral is auditable at the call site (see `partial.rs` MpcError doc).
    fn assert_deferred(err: &MpcError, must_mention: &[&str]) {
        match err {
            MpcError::NotYetImplemented { what, gated_on } => {
                let hay = format!("{what} || {gated_on}");
                for needle in must_mention {
                    assert!(
                        hay.contains(needle),
                        "deferred error must name {needle:?}; got what={what:?} gated_on={gated_on:?}"
                    );
                }
            }
            other => panic!(
                "expected NotYetImplemented (the no-fake-crypto channel), got {other:?} \
                 — a deferred path returned a non-typed-stub result"
            ),
        }
    }

    // --- helpers reused across rows ---------------------------------------

    fn holder_with_salary() -> Holder {
        Holder::from_rdf(
            "h",
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             ex:h ex:salary \"5\"^^xsd:integer .",
            "turtle",
        )
        .unwrap()
    }

    /// A do-nothing backend whose three trait methods are HONEST stubs — exactly
    /// the shape a future dishonest-majority backend (or any not-yet-built
    /// backend) presents before its crypto lands. Mirrors `proof.rs`'s StubBackend
    /// but lives here so the table is self-contained.
    struct StubBackend;
    impl MpcBackend for StubBackend {
        type Share = ();
        fn info(&self) -> BackendInfo {
            // sq-mq8q — a dishonest-majority, semi-honest-only stub
            // built from the three-axis descriptor. `BackendInfo::new` derives the
            // back-compat `trust_model = DishonestMajority` / `malicious_security =
            // SemiHonestOnly` projection. Under a dishonest majority the ONLY
            // representable output guarantee is `Abort` — Cleve forbids
            // fairness/GOD — which the type system enforces (the
            // `OutputGuarantee::guaranteed_output` constructor would return `None`
            // for this threshold).
            BackendInfo::new(
                "stub-deferred",
                SecurityDescriptor {
                    adversary: AdversaryModel::SemiHonest,
                    output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
                    threshold: CorruptionThreshold::DishonestMajority { t: 2 },
                    public_verifiability: PublicVerifiability(false),
                },
                0,
            )
        }
        fn share_private_input(&self, _h: &Holder) -> Result<Vec<()>, MpcError> {
            Err(MpcError::not_yet("secret-share private input", "M3 + Q2"))
        }
        fn run_secure(&self, _s: &[()]) -> Result<Vec<()>, MpcError> {
            Err(MpcError::not_yet("run secure computation", "M3 + Q2"))
        }
        fn reconstruct_disclosed(&self, _s: &[()]) -> Result<PartialResult, MpcError> {
            Err(MpcError::not_yet("reconstruct disclosed output", "M3 + Q2"))
        }
    }

    // === ROW 1: disclosed-key join routes the HIDDEN-VALUE regime away. =====
    // The crypto-free DisclosedKeyJoin must NOT fake a private-key join: a plan
    // with key_disclosed == false stays a typed NotYetImplemented naming M3/Q2/Q3.
    #[test]
    fn row1_disclosed_join_hidden_path_is_typed_stub() {
        let h = Holder::from_rdf(
            "a",
            "@prefix ex: <http://ex/> . ex:p1 ex:knows ex:x1 .",
            "turtle",
        )
        .unwrap();
        let pa = h
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let plan = JoinPlan {
            join_var: Variable::new_unchecked("x"),
            key_disclosed: false,
        };
        let err = DisclosedKeyJoin.join(&[pa], &plan).unwrap_err();
        assert_deferred(&err, &["hidden-value", "M3", "Q2", "Q3"]);
    }

    // === ROW 2: collaborative-proof `prove` is deferred (Q1 + ZK foundation). ==
    #[test]
    fn row2_collaborative_prove_is_typed_stub() {
        struct StubProof;
        impl CollaborativeProof<StubBackend> for StubProof {
            fn prove(
                &self,
                _b: &StubBackend,
                _s: &ProofStatement,
                _j: &[JoinPlan],
            ) -> Result<Proof, MpcError> {
                Err(MpcError::not_yet(
                    "collaborative proof of correct + attested-source result",
                    "ZK foundation #3/#4/#5/#6/#8/#9/#12 + Q1 distributed-attestation spike (M4)",
                ))
            }
            fn verify(&self, _s: &ProofStatement, _p: &Proof) -> Result<bool, MpcError> {
                Err(MpcError::not_yet(
                    "verify collaborative proof",
                    "ZK foundation + M4",
                ))
            }
        }
        let stmt = ProofStatement {
            query: "SELECT ?x WHERE { ?x ?p ?o }".into(),
            disclosed_key_set: vec!["did:example:k#1".into()],
            challenge: vec![],
            disclosed_result: PartialResult {
                holder: HolderId::new("fed"),
                vars: vec![Variable::new_unchecked("x")],
                rows: vec![],
            },
        };
        let err = StubProof.prove(&StubBackend, &stmt, &[]).unwrap_err();
        // Must cite the issuer-signature gate (#3) AND the Q1 research spike — a
        // proof that "just works" without these would be fake crypto.
        assert_deferred(&err, &["#3", "Q1"]);
    }

    // === ROW 3: collaborative-proof `verify` is deferred too. ================
    #[test]
    fn row3_collaborative_verify_is_typed_stub() {
        struct StubProof;
        impl CollaborativeProof<StubBackend> for StubProof {
            fn prove(
                &self,
                _b: &StubBackend,
                _s: &ProofStatement,
                _j: &[JoinPlan],
            ) -> Result<Proof, MpcError> {
                Err(MpcError::not_yet("prove", "M4"))
            }
            fn verify(&self, _s: &ProofStatement, _p: &Proof) -> Result<bool, MpcError> {
                Err(MpcError::not_yet(
                    "verify collaborative proof",
                    "ZK foundation + M4",
                ))
            }
        }
        let stmt = ProofStatement {
            query: "SELECT * WHERE { ?s ?p ?o }".into(),
            disclosed_key_set: vec![],
            challenge: vec![],
            disclosed_result: PartialResult {
                holder: HolderId::new("fed"),
                vars: vec![],
                rows: vec![],
            },
        };
        let proof = Proof::test_stub();
        let err = StubProof.verify(&stmt, &proof).unwrap_err();
        // CRITICAL: verify must NOT return Ok(true) — a stub that "accepts" any
        // proof is the worst possible fake crypto. It must be the typed stub.
        assert!(
            !matches!(StubProof.verify(&stmt, &proof), Ok(true)),
            "FAKE-CRYPTO GUARD: deferred verify must never accept (Ok(true))"
        );
        assert_deferred(&err, &["verify", "M4"]);
    }

    // === ROW 4: distributed attestation (the federated #3) is deferred. ======
    #[test]
    fn row4_attestation_is_typed_stub() {
        struct StubAttest;
        impl Attestation for StubAttest {
            fn attest_source(
                &self,
                _h: &HolderId,
                _k: &[String],
            ) -> Result<AttestationShare, MpcError> {
                Err(MpcError::not_yet(
                    "distributed issuer-signature attestation over secret-shared witness",
                    "ZK foundation #3/#8/#9 + Q1",
                ))
            }
        }
        let err = StubAttest
            .attest_source(&HolderId::new("alice"), &[])
            .unwrap_err();
        assert_deferred(&err, &["attestation", "#3", "Q1"]);
    }

    // === ROW 5: a not-yet-built backend's share/run/reconstruct stay typed. ==
    // Documents the trust-model seam: a backend whose crypto is not implemented
    // (e.g. the future dishonest-majority one) must fail closed on ALL three
    // obligations — never return a plaintext / fabricated share or result.
    #[test]
    fn row5_deferred_backend_methods_are_typed_stubs() {
        let b = StubBackend;
        let h = holder_with_salary();
        assert_deferred(
            &b.share_private_input(&h).unwrap_err(),
            &["secret-share", "Q2"],
        );
        assert_deferred(
            &b.run_secure(&[]).unwrap_err(),
            &["secure computation", "Q2"],
        );
        assert_deferred(
            &b.reconstruct_disclosed(&[]).unwrap_err(),
            &["reconstruct", "Q2"],
        );
        // And its self-declared guarantees are honest: NO active-security claim.
        // Assert the precise enum variant (not just the coarse bool) so a stub
        // cannot smuggle in a detect/robust claim it does not deliver.
        assert_eq!(
            b.info().malicious_security,
            MaliciousSecurity::SemiHonestOnly,
            "stub backend must not claim any malicious-security guarantee"
        );
        assert!(
            !b.info().malicious_secure(),
            "coarse accessor must agree: stub is not malicious-secure"
        );
    }

    // === ROW 6: the REAL Shamir backend is NOT a stub on its real methods, but
    // it MUST still fail closed on protocol-precondition violations (typed
    // Protocol error, never a fabricated answer). This is the "real path is real,
    // and its error paths are typed too" control for the table. ================
    #[test]
    fn row6_real_backend_fails_closed_on_preconditions_not_with_fake_data() {
        // n < 2 is refused (typed Protocol error), not silently coerced.
        assert!(matches!(ShamirBackend::new(1), Err(MpcError::Protocol(_))));

        // run_secure with no inputs is a typed Protocol error, not an Ok(empty).
        let b = ShamirBackend::new_seeded(3, 1).unwrap();
        let empty: Vec<Vec<Share>> = vec![];
        assert!(matches!(b.run_secure(&empty), Err(MpcError::Protocol(_))));

        // reconstruct below threshold is a typed error, not a fabricated value
        // (overlaps the threshold suite; restated here as the no-fake-crypto row
        // for the REAL backend's reconstruction path).
        let shares = b.dealer().share(fp(5));
        let t = b.threshold();
        assert!(matches!(
            b.reconstruct(&shares[..t]),
            Err(MpcError::Protocol(_))
        ));

        // And the honest path still really works (the table doesn't break it).
        assert_eq!(b.reconstruct(&shares).unwrap(), fp(5));
        // The real backend declares its honest guarantees: honest-majority, and
        // the PRECISE reconstruction-hardening level WI-1's RS-checked reconstruction
        // delivers for THIS `(n, t)` — no over-claim, no under-claim. At n=3,
        // t=1 there is one redundant share (n > t+1) but the correction budget
        // e = ⌊(n−t−1)/2⌋ = 0, so the honest guarantee is detect-and-abort.
        let info = b.info();
        assert_eq!(info.trust_model, TrustModel::HonestMajority);
        assert_eq!(
            info.malicious_security,
            MaliciousSecurity::HonestMajorityAbort,
            "n=3,t=1 has one redundant share → detect-and-abort, no robust correction"
        );
        // It must NOT claim ROBUST here (it cannot correct a cheater at n=3) —
        // the no-fake-crypto invariant: never claim a guarantee not provided.
        assert!(
            !matches!(
                info.malicious_security,
                MaliciousSecurity::HonestMajorityRobust { .. }
            ),
            "n=3 Shamir must NOT claim robust correction"
        );

        // Boundary table — the report tracks the real RS redundancy, nothing more.
        // HONESTY: with the honest-majority `t = ⌊(n−1)/2⌋`, EVERY valid `n >= 2`
        // already carries at least one redundant share at degree `t`, so the
        // degree-`t` reconstruction is NEVER `SemiHonestOnly` (that variant is
        // reachable only on the no-redundancy degree-`2t` equality open / a stub
        // backend).
        // n=2,t=0: one redundant share, e = ⌊(2−0−1)/2⌋ = 0 → detect-and-abort.
        assert_eq!(
            ShamirBackend::new(2).unwrap().malicious_security(),
            MaliciousSecurity::HonestMajorityAbort,
            "n=2,t=0: one redundant share, e=0 → detect-and-abort (never SemiHonestOnly at degree t)"
        );
        // n=4,t=1: e = ⌊(4−1−1)/2⌋ = 1 → robust correction of one cheater.
        assert_eq!(
            ShamirBackend::new(4).unwrap().malicious_security(),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 1 },
            "n=4,t=1: redundancy admits robust correction of 1 cheater"
        );
        // n=5,t=2: e = ⌊(5−2−1)/2⌋ = 1 → robust correction of one cheater.
        assert_eq!(
            ShamirBackend::new(5).unwrap().malicious_security(),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 1 },
            "n=5,t=2: redundancy admits robust correction of 1 cheater"
        );
        // n=9,t=4: e = ⌊(9−4−1)/2⌋ = 2 → robust correction of two cheaters.
        assert_eq!(
            ShamirBackend::new(9).unwrap().malicious_security(),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 2 },
            "n=9,t=4: redundancy admits robust correction of 2 cheaters"
        );
    }

    // === ROW 6b sq-mq8q: PER-OPERATOR security reporting. =========
    // The degree-`t` linear aggregate and the degree-`2t` equality open deliver
    // DIFFERENT guarantees for the SAME backend `(n, t)`. One backend-level bit
    // would lie: at n=3,t=1 the aggregate is detect-and-abort (one redundant
    // share) but the equality open is SEMI-HONEST-ONLY (degree 2t=2 has zero RS
    // redundancy at n=2t+1=3). `operator_security` must report each precisely.
    #[test]
    fn row6b_per_operator_security_differs_aggregate_vs_equality_open() {
        // n=3, t=1 — the canonical honest-majority odd-n case (n = 2t+1).
        let b = ShamirBackend::new(3).unwrap();

        // LINEAR AGGREGATE (degree t=1): one redundant share, e = ⌊(3−1−1)/2⌋ = 0
        // → detect-and-abort. Honest-majority threshold; abort output guarantee.
        let agg = b.operator_security(OperatorClass::LinearAggregate);
        assert_eq!(agg.adversary, AdversaryModel::SemiHonest);
        assert_eq!(agg.threshold, CorruptionThreshold::HonestMajority { t: 1 });
        assert_eq!(
            agg.output_guarantee,
            OutputGuarantee::Abort(AbortKind::Unanimous),
            "n=3,t=1 aggregate: detect-and-abort (e=0)"
        );

        // EQUALITY JOIN (degree 2t=2): n = 2t+1 = 3, so degree-2t has NO RS
        // redundancy → SEMI-HONEST-ONLY. This is the per-operator difference the
        // single backend-level bit would have hidden.
        let eq = b.operator_security(OperatorClass::EqualityJoin);
        assert_eq!(
            eq.malicious_security(0),
            MaliciousSecurity::SemiHonestOnly,
            "n=3,t=1 equality open at degree 2t has zero redundancy → semi-honest-only"
        );
        assert_eq!(
            eq.output_guarantee,
            OutputGuarantee::Abort(AbortKind::Selective),
            "no-redundancy open is selective-abort/no-detection (the SemiHonestOnly projection)"
        );

        // They genuinely DIFFER for the same backend — the load-bearing point.
        assert_ne!(
            agg.output_guarantee, eq.output_guarantee,
            "aggregate (detect-and-abort) and equality open (semi-honest-only) must NOT collapse to one bit"
        );

        // The backend-level `info().security` tracks the PRIMARY (aggregate) path,
        // and its back-compat projection is unchanged (== the old value).
        let info = b.info();
        assert_eq!(info.security.output_guarantee, agg.output_guarantee);
        assert_eq!(
            info.malicious_security,
            MaliciousSecurity::HonestMajorityAbort
        );

        // n=4, t=1 — degree 2t=2 now has redundancy (n=4 > 2t+1=3): the equality
        // open becomes detect-and-abort (e = ⌊(4−2−1)/2⌋ = 0), while the
        // aggregate at degree t=1 is ROBUST (e = ⌊(4−1−1)/2⌋ = 1). Still differ.
        let b4 = ShamirBackend::new(4).unwrap();
        assert_eq!(
            b4.operator_security(OperatorClass::LinearAggregate)
                .malicious_security(1),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 1 },
            "n=4 aggregate is robust (e=1)"
        );
        assert_eq!(
            b4.operator_security(OperatorClass::EqualityJoin)
                .malicious_security(0),
            MaliciousSecurity::HonestMajorityAbort,
            "n=4 equality open at degree 2t has one redundant share → detect-and-abort"
        );

        // Comparison is not realized in-crypto → honest no-guarantee baseline
        // (no over-claim), reported rather than silently omitted.
        assert_eq!(
            b.operator_security(OperatorClass::Comparison)
                .malicious_security(0),
            MaliciousSecurity::SemiHonestOnly
        );
    }

    // === ROW 6c sq-mq8q: the dishonest-majority stub cannot advertise
    // a Cleve-forbidden guarantee. The three-axis descriptor it returns has a
    // DishonestMajority threshold, under which fairness/GOD are unconstructible —
    // so its honest output guarantee can only be Abort. ======================
    #[test]
    fn row6c_dishonest_majority_stub_is_abort_only_cleve() {
        let b = StubBackend;
        let info = b.info();
        assert_eq!(info.trust_model, TrustModel::DishonestMajority);
        assert_eq!(
            info.security.threshold,
            CorruptionThreshold::DishonestMajority { t: 2 }
        );
        // Cleve: under this threshold the descriptor's guarantee is an abort, and
        // the type system would refuse to construct anything stronger.
        assert!(info.security.output_guarantee.is_abort());
        assert!(!info.security.output_guarantee.requires_honest_majority());
        assert_eq!(
            OutputGuarantee::guaranteed_output(info.security.threshold),
            None,
            "Cleve: a dishonest-majority backend cannot mint a GOD guarantee"
        );
    }

    // === ROW 7: HiddenValueJoin (the REAL M3 hidden join) is real — assert it
    // does NOT silently reconstruct/disclose the private KEY (only the payload).
    // A hidden join that leaked the key would be the join-layer's "fake privacy".
    #[test]
    fn row7_hidden_join_discloses_only_payload_not_the_private_key() {
        use crate::join::HiddenKeyedRows;
        let lit = |s: &str| Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)));
        let left = HiddenKeyedRows {
            holder: HolderId::new("L"),
            payload_vars: vec![Variable::new_unchecked("name")],
            rows: vec![(fp(100), vec![lit("Alice")]), (fp(200), vec![lit("Bob")])],
        };
        let right = HiddenKeyedRows {
            holder: HolderId::new("R"),
            payload_vars: vec![Variable::new_unchecked("city")],
            rows: vec![(fp(200), vec![lit("Leeds")])],
        };
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 0xBEEF).unwrap());
        let got = join.join(&left, &right).unwrap();
        // Output schema is payload-only: the private key var is NOT projected.
        assert_eq!(
            got.vars,
            vec![
                Variable::new_unchecked("name"),
                Variable::new_unchecked("city")
            ]
        );
        // And the disclosed rows carry NO field-encoded key (100/200) — only the
        // matched payload (Bob, Leeds).
        let rendered = format!("{:?}", got.rows);
        assert!(
            !rendered.contains("100") && !rendered.contains("200"),
            "key must not leak: {rendered}"
        );
        assert_eq!(got.rows.len(), 1, "exactly the matching (Bob,Leeds) row");
    }
}
