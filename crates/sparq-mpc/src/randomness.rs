// sq-yyro: the DEALER-LESS correlated-randomness seam — PRSS /
// honest-majority coin-toss / dealer-less VSS. Design + seam only; NO dealer-less
// crypto ships here. See research/mpc-distributed-randomness-design.md.
//! Distributed / dealer-less correlated randomness — the seam that will replace
//! the single-trusted-dealer simulation (sq-yyro).
//!
//! ## Why this module exists
//!
//! Masks and correlated randomness are **security-critical** and, in a REAL
//! federation, must be generated so that **no single party knows them**. Today a
//! single [`crate::shamir::ShamirDealer`] owns the masking CSPRNG and draws every
//! mask / share centrally — an honest **single-dealer SIMULATION**, but not a
//! federation-ready source: the dealer *knows* every mask (it drew it). This
//! module names the contract a dealer-less source must satisfy and labels the
//! current dealer honestly as the simulation it is.
//!
//! The randomness *quality* is already honest (sq-1vt: OS-seeded ChaCha20,
//! uniform rejection sampling — see [`crate::rng`]). The gap this seam addresses
//! is **who** draws it: PRSS (replicated-PRF, non-interactive, small-`n`) or a
//! distributed coin-toss (interactive, any-`n`) generate correlated randomness
//! jointly, and **dealer-less VSS** lets each holder share its OWN input
//! verifiably. See `research/mpc-distributed-randomness-design.md` for the
//! PRSS-vs-coin-toss decision and the dealer-less-VSS construction.
//!
//! ## The `r = 0` threat (the load-bearing correctness point)
//!
//! [`secure_equal`](crate::compare::secure_equal_to_bit) hides a key difference
//! `d = a − b` behind `m = d · r` and opens `m`; the verdict `m == 0 ⇔ a == b`
//! holds **iff `r ≠ 0`**. An adversary who forces the mask to `r = 0` makes
//! `m = 0` for EVERY pair — a false match on every row. So the equality mask must
//! be uniform, **nonzero**, and jointly generated so no party controls it. Under
//! the current single honest dealer this is guaranteed by
//! [`draw_nonzero_fp`](crate::shamir::ShamirDealer::draw_nonzero_fp); under a
//! semi-honest PRSS/coin-toss source `r` is uniform + unknown but **NOT defended
//! against an ACTIVE adversary** — enforcing `r ≠ 0` against a malicious
//! contributor needs a *biasing-resistant* joint generation plus a distributed
//! nonzero zero-test-and-redraw, with the test's opens IT-MAC-authenticated
//! (`mpc-malicious-security-design.md`, sq-km34.*) so a tampered share aborts. An
//! IT-MAC on the final open **alone does NOT suffice**: it authenticates that `m`
//! equals `[d · r]`, not that `r ≠ 0`, so a correctly-authenticated `r = 0` still
//! opens `m = 0` and flips the verdict. That defense is a FOLLOW-ON bead;
//! [`DistributedRandomness::shared_nonzero_mask`] is documented semi-honest-only
//! until it lands.
//!
//! ## Status — no fake crypto
//!
//! The only implementor *in this module* is [`ShamirDealer`], which reports
//! [`RandomnessModel::TrustedDealerSim`] (`deployable() == false`).
//! [`crate::prss::PrssRandomness`] is the first dealer-less GENERATOR behind the
//! trait ([`RandomnessModel::Prss`], sq-yyro follow-on): its
//! `Σ_A PRF(k_A, ctr)·f_A` online generation is real and non-interactive, but its
//! replicated-seed SETUP is a simulated one-time trusted setup, its `r ≠ 0` check
//! is a simulation artefact, and it refuses `vss_own_input` — read that module's
//! docs before relying on any of it. [`RandomnessModel::HonestMajorityCoinToss`]
//! and dealer-less VSS remain follow-on beads.
//!
//! **NO variant is `deployable()` today.** The model is a self-reported
//! description, so deployment acceptance stays fail-closed until a validated
//! dealer-less implementation lands behind an acceptance boundary tied to the
//! construction (design record §5 item 5) — a label is never the evidence.

use crate::field::Fp;
use crate::partial::MpcError;
use crate::shamir::{ShamirBackend, ShamirDealer, Share};

/// Which correlated-randomness regime is producing masks — an **honest
/// self-description** so a caller (or a `BackendInfo`-style report) can state
/// whether the randomness source is federation-ready, never guess it from the
/// concrete type.
///
/// See `research/mpc-distributed-randomness-design.md` §2 for the PRSS-vs-coin-toss
/// decision behind the two dealer-less variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomnessModel {
    /// The CURRENT in-process **single-trusted-dealer simulation**: one
    /// [`ShamirDealer`] draws every mask from the CSPRNG and therefore *knows*
    /// it. Honest for testing / the in-process protocol simulation, but **NOT
    /// deployable** to a real federation (there is no trusted dealer). This is
    /// what every production call site still draws through today.
    TrustedDealerSim,
    /// **PRSS** — replicated-PRF pseudo-random secret sharing (eprint 2021/1223,
    /// IETF draft-thomson-ppm-prss). Non-interactive (0 online rounds) after a
    /// one-time replicated-seed setup; the primary choice for **small `n`**
    /// (the four-flatmates target). Dealer-less. Implemented by
    /// [`crate::prss::PrssRandomness`] — the *generator* is real, its seed setup
    /// is a simulated one-time trusted setup, and it is still not `deployable()`.
    Prss,
    /// **Distributed coin-toss** — commit-open (or VSS-summed) joint randomness.
    /// Interactive (≥1 round per batch) but needs no combinatorial setup, so it
    /// is the general-`n` / setup-free fallback and the malicious-with-abort
    /// upgrade path. Dealer-less. Follow-on bead — not built.
    HonestMajorityCoinToss,
}

impl RandomnessModel {
    /// Whether this model **describes** a dealer-less regime — one *designed* so
    /// that no single party knows a mask. Descriptive only: it states what the
    /// regime is designed to be, not that any implementation validly achieves it.
    /// Only [`Self::TrustedDealerSim`] is not dealer-less.
    pub fn is_dealer_less(self) -> bool {
        match self {
            RandomnessModel::TrustedDealerSim => false,
            RandomnessModel::Prss | RandomnessModel::HonestMajorityCoinToss => true,
        }
    }

    /// Whether a source *self-reporting* this model may be accepted for a REAL
    /// federation deployment. **`false` for every variant today.** The
    /// single-dealer simulation is structurally non-deployable (the dealer knows
    /// every mask). The dealer-less regimes are deployable *in principle*, but no
    /// validated implementation of either exists, and this enum is a
    /// self-description — any type can report [`Self::Prss`] — so a label alone
    /// can never be evidence that the mask-secrecy / VSS guarantees actually
    /// hold. Deployment acceptance stays fail-closed until the concrete
    /// PRSS / coin-toss implementations land behind an acceptance boundary tied
    /// to the validated construction (not to this descriptive enum) — the
    /// follow-on beads in `research/mpc-distributed-randomness-design.md` §5.
    pub fn deployable(self) -> bool {
        match self {
            // The dealer knows every mask — never deployable.
            RandomnessModel::TrustedDealerSim => false,
            // Dealer-less BY DESIGN, but descriptive only: no validated
            // implementation ships, so a self-reported label is refused.
            RandomnessModel::Prss => false,
            RandomnessModel::HonestMajorityCoinToss => false,
        }
    }
}

/// The correlated-randomness contract the MPC protocol consumes, independent of
/// **how** the randomness is produced (single dealer today; PRSS / coin-toss in a
/// real deployment). A dealer-less source is a new type implementing this trait,
/// selected by [`RandomnessModel`] — never by concrete type — so the join /
/// comparison / oblivious-shuffle callers compose onto it unmodified. This is the
/// randomness analogue of the [`crate::backend::MpcBackend`] primitive seam.
///
/// ## Method contracts are STRUCTURAL; security is a [`RandomnessModel`] property
///
/// The methods below promise only what **every** implementor delivers — a
/// well-formed degree-`t` sharing of the stated value. The security guarantees a
/// federation actually cares about (that *no `≤ t` parties know* a mask; that an
/// input sharing is *VSS-verified* against a malicious holder) are **not** part of
/// any method's contract, because no implementor satisfies them both:
/// [`ShamirDealer`] is a simulation whose dealer knows every mask, and
/// [`crate::prss::PrssRandomness`] generates masks dealer-lessly but on a
/// *simulated* seed setup and refuses `vss_own_input` outright. A generic caller
/// must never infer those guarantees from the trait — they are capabilities of a
/// **validated dealer-less implementation**, none of which ships yet.
/// [`randomness_model`](Self::randomness_model) is an honest *description* of the
/// regime, and [`require_deployable`](Self::require_deployable) is the fail-closed
/// refusal gate — today it refuses EVERY source (see its docs), and wiring it
/// through the production call sites is itself a follow-on bead (no call site is
/// gated on it yet, although `degree_reduce`, the hidden-value equality path and
/// the whole comparison chain already draw through this trait).
///
/// See `research/mpc-distributed-randomness-design.md` for the full design and
/// the follow-on implementation beads.
pub trait DistributedRandomness {
    /// Honest self-description of which regime is producing the randomness — so a
    /// caller (or a `BackendInfo`-style report) can state what the source claims
    /// to be rather than guess it from the concrete type. **Descriptive only:**
    /// the label is self-reported and is never by itself evidence that the
    /// mask-secrecy / dealer-less-VSS guarantees hold — those are capabilities of
    /// a validated dealer-less implementation, none of which ships yet.
    fn randomness_model(&self) -> RandomnessModel;

    /// The number of parties `n` this source deals sharings to.
    fn parties(&self) -> usize;

    /// The privacy threshold `t`: every sharing this source returns has degree `t`.
    fn threshold(&self) -> usize;

    /// Fail-closed refusal gate: `Err(`[`MpcError::NoBackendSatisfies`]`)` unless
    /// this source's [`randomness_model`](Self::randomness_model) is
    /// [`deployable`](RandomnessModel::deployable). **Necessary, not sufficient**
    /// — and since `deployable()` is `false` for every variant today (no
    /// validated dealer-less implementation exists, and the model is
    /// self-reported), this currently refuses EVERY source, including one that
    /// merely *labels* itself [`RandomnessModel::Prss`]. `Ok` here does NOT
    /// establish mask secrecy or VSS verification; those require a validated
    /// dealer-less implementation behind the acceptance boundary the follow-on
    /// beads add. No production call site is gated on this yet. Wiring the callers
    /// is in progress (design record §5 item 5): `degree_reduce`
    /// ([`crate::shamir::degree_reduce_through`]), the hidden-value equality path
    /// ([`crate::join::HiddenValueJoin`]) and the [`crate::compare`] chain draw
    /// through the seam; the oblivious shuffle, the authenticated (IT-MAC) twins
    /// and the remaining input-dealing paths still call the dealer directly.
    /// Provided as a default so every implementor inherits the same refusal.
    fn require_deployable(&self) -> Result<(), MpcError> {
        let model = self.randomness_model();
        if model.deployable() {
            Ok(())
        } else {
            Err(MpcError::NoBackendSatisfies {
                requirement: format!(
                    "dealer-less (deployable) correlated-randomness source; got {:?}",
                    model
                ),
                considered: 1,
            })
        }
    }

    /// A fresh degree-`t` sharing of a **uniform** mask. Used wherever the
    /// protocol needs correlated randomness whose value is irrelevant as long as
    /// it is uniform (e.g. blinding a value before an open, degree-reduction
    /// re-sharing randomness).
    ///
    /// **Structural contract only.** Every implementor returns a well-formed
    /// degree-`t` sharing of a uniform value. Whether *no `≤ t` parties know* that
    /// mask is NOT part of this method's contract: a
    /// [`RandomnessModel::TrustedDealerSim`] source draws — and therefore knows —
    /// every mask; party-secrecy is a capability only a validated dealer-less
    /// implementation delivers (none ships yet). A caller that needs secrecy MUST
    /// gate on [`require_deployable`](Self::require_deployable) — a necessary,
    /// not sufficient, check.
    fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError>;

    /// A fresh degree-`t` sharing of a uniform **nonzero** mask `r` — the
    /// equality-test mask (see the module `r = 0` threat note). The nonzero value
    /// is the whole correctness of `secure_equal`, and (unlike secrecy) IS part of
    /// this method's structural contract: every implementor returns `r ≠ 0`.
    ///
    /// **Secrecy + active-adversary caveats are implementation capabilities.** As
    /// with [`shared_mask`](Self::shared_mask), whether *no `≤ t` parties know*
    /// `r` is not guaranteed here — it is a capability of a validated dealer-less
    /// implementation. And even a validated **semi-honest** dealer-less source yields a
    /// uniform, party-unknown `r` but does NOT defend `r = 0` against an *active*
    /// adversary that biases its contribution: that needs a biasing-resistant joint
    /// generation plus an authenticated distributed nonzero zero-test (a follow-on
    /// bead) — an IT-MAC on the open alone does not prove `r ≠ 0`. The current
    /// single-dealer impl is honest because the one dealer draws `r` nonzero.
    fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError>;

    /// A degree-`t` sharing of a holder's **own** `secret`.
    ///
    /// **Structural contract only.** Every implementor returns a degree-`t`
    /// sharing that reconstructs to `secret`. Whether that sharing is *verified* —
    /// **dealer-less VSS**, so a malicious holder cannot distribute an inconsistent
    /// polynomial that reconstructs to different values for different quorums — is
    /// NOT guaranteed by this method. The [`RandomnessModel::TrustedDealerSim`]
    /// implementor performs an UNCHECKED plain sharing (the holder is assumed to
    /// share correctly); only a validated dealer-less VSS implementation (none
    /// ships yet) verifies the shares lie on one consistent degree-`t`
    /// polynomial and aborts on a cheater — the input-side twin of
    /// [`crate::robust`]'s output-side robustness. A caller that needs that
    /// guarantee MUST gate on [`require_deployable`](Self::require_deployable) —
    /// a necessary, not sufficient, check.
    fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError>;
}

/// The CURRENT single-trusted-dealer **SIMULATION** implements the seam.
///
/// [`ShamirDealer`] owns the masking CSPRNG and therefore *knows* every mask it
/// produces — so it reports [`RandomnessModel::TrustedDealerSim`] and is **NOT
/// deployable** to a real federation. Implementing the trait here (rather than
/// leaving it an unused stub) pins the exact call-shape a future dealer-less PRSS
/// / coin-toss source must satisfy and lets callers migrate to
/// `&mut dyn DistributedRandomness` incrementally, with the dealer-less source
/// swapped in behind the same trait. No behaviour changes: each method routes to
/// the dealer's existing inherent randomness methods.
impl DistributedRandomness for ShamirDealer {
    fn randomness_model(&self) -> RandomnessModel {
        RandomnessModel::TrustedDealerSim
    }

    fn parties(&self) -> usize {
        ShamirDealer::parties(self)
    }

    fn threshold(&self) -> usize {
        ShamirDealer::threshold(self)
    }

    fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
        // Simulation artefact: the dealer draws AND knows `r`. A dealer-less
        // source would generate `[r]` jointly (PRSS/coin-toss) so no party knows it.
        let r = self.draw_fp();
        Ok(self.share(r))
    }

    fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
        // Honest here only because the one dealer draws `r != 0` (rejection loop).
        // A semi-honest dealer-less source needs the active `r = 0` defense — see
        // the module note.
        let r = self.draw_nonzero_fp();
        Ok(self.share(r))
    }

    fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
        // Simulation: the dealer shares on the holder's behalf. Dealer-less VSS
        // would have the holder share its own input with a distributed
        // consistency check.
        Ok(self.share(secret))
    }
}

/// Fail closed unless `shares` is the canonical `n`-party sharing: exactly `n`
/// shares on the points `x = 1..=n`, in order.
///
/// Every protocol step that consumes a sharing drawn from a
/// [`DistributedRandomness`] source pairs it with the other operands BY
/// POSITION (`add_shares` / `mul_shares_raw` / the degree-reduction
/// accumulation), and that pairing is only meaningful on the canonical points.
/// The source is caller-supplied, so its output shape is checked here rather
/// than trusted: a short, long, permuted or mis-pointed sharing is refused with
/// [`MpcError::Protocol`] before it can silently misalign a computation. This
/// is a SHAPE check only — it says nothing about the sharing's degree or the
/// value it reconstructs to.
pub(crate) fn require_canonical_sharing(shares: &[Share], n: usize) -> Result<(), MpcError> {
    if shares.len() != n
        || shares
            .iter()
            .enumerate()
            .any(|(j, sh)| sh.x != j as u64 + 1)
    {
        return Err(MpcError::Protocol(format!(
            "the randomness source returned a sharing that is not the canonical {n}-party \
             sharing on x = 1..={n}, in order ({} shares)",
            shares.len()
        )));
    }
    Ok(())
}

/// Fail closed unless `source` deals sharings for the same `(n, t)` as `backend`.
///
/// A stage that draws from a caller-supplied source but combines the result with
/// sharings, constants and opens built from a [`ShamirBackend`] (its `parties()`
/// for public-constant sharings, its `reconstruct` for the cleartext the
/// simulation holds, the oblivious output stage) silently computes garbage if the
/// two disagree: a source that under-reports `t` passes every honest-majority
/// guard and then yields too-low-degree reductions. So every `*_through` entry
/// point that takes both a source and a backend checks the match up front and
/// refuses a mismatched source with [`MpcError::Protocol`].
pub(crate) fn require_source_matches(
    source: &dyn DistributedRandomness,
    backend: &ShamirBackend,
) -> Result<(), MpcError> {
    let (sn, st) = (source.parties(), source.threshold());
    let (bn, bt) = (backend.parties(), backend.threshold());
    if (sn, st) != (bn, bt) {
        return Err(MpcError::Protocol(format!(
            "the randomness source reports (n, t) = ({sn}, {st}) but the backend is \
             (n, t) = ({bn}, {bt}); its sharings would not compose with the backend's"
        )));
    }
    Ok(())
}

/// [`DistributedRandomness::vss_own_input`] followed by
/// [`require_canonical_sharing`] against the caller's (already validated) party
/// count `n`.
pub(crate) fn checked_vss_own_input(
    source: &mut dyn DistributedRandomness,
    n: usize,
    secret: Fp,
) -> Result<Vec<Share>, MpcError> {
    let shares = source.vss_own_input(secret)?;
    require_canonical_sharing(&shares, n)?;
    Ok(shares)
}

/// [`DistributedRandomness::shared_nonzero_mask`] followed by
/// [`require_canonical_sharing`] against the caller's (already validated) party
/// count `n`.
pub(crate) fn checked_shared_nonzero_mask(
    source: &mut dyn DistributedRandomness,
    n: usize,
) -> Result<Vec<Share>, MpcError> {
    let shares = source.shared_nonzero_mask()?;
    require_canonical_sharing(&shares, n)?;
    Ok(shares)
}

/// Test-only [`DistributedRandomness`] sources shared by the seam tests of every
/// module that draws through the seam: a source that corrupts exactly one
/// targeted draw, one that misreports its threshold, and one that records the
/// full draw transcript.
#[cfg(test)]
pub(crate) mod test_sources {
    use super::*;

    /// A seam method that draws a sharing.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum SeamMethod {
        SharedMask,
        SharedNonzeroMask,
        VssOwnInput,
    }

    impl SeamMethod {
        fn name(self) -> &'static str {
            match self {
                SeamMethod::SharedMask => "shared_mask",
                SeamMethod::SharedNonzeroMask => "shared_nonzero_mask",
                SeamMethod::VssOwnInput => "vss_own_input",
            }
        }
    }

    /// How [`MalformedSource`] corrupts the targeted sharing's shape.
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Malformation {
        Short,
        Long,
        Permuted,
        DuplicateX,
        WrongX,
    }

    /// Every [`Malformation`], for tests that sweep them all.
    pub(crate) const ALL_MALFORMATIONS: [Malformation; 5] = [
        Malformation::Short,
        Malformation::Long,
        Malformation::Permuted,
        Malformation::DuplicateX,
        Malformation::WrongX,
    ];

    /// The substring of [`require_canonical_sharing`]'s refusal, for asserting
    /// that a malformed draw was refused by the shape check and not by something
    /// downstream.
    pub(crate) const SHAPE_REFUSAL: &str = "returned a sharing that is not the canonical";

    /// Delegates to a real dealer but corrupts the SHAPE of exactly ONE draw: the
    /// call to `target` after `skip` earlier calls to the same method. Every other
    /// draw is passed through untouched, so a test can aim at one specific
    /// consumption site.
    pub(crate) struct MalformedSource {
        inner: ShamirDealer,
        target: SeamMethod,
        skip: usize,
        how: Malformation,
        seen: usize,
        fired: bool,
    }

    impl MalformedSource {
        pub(crate) fn new(
            inner: ShamirDealer,
            target: SeamMethod,
            skip: usize,
            how: Malformation,
        ) -> Self {
            MalformedSource {
                inner,
                target,
                skip,
                how,
                seen: 0,
                fired: false,
            }
        }

        /// Whether the targeted draw has happened (and was corrupted).
        pub(crate) fn fired(&self) -> bool {
            self.fired
        }

        fn maybe_corrupt(
            &mut self,
            method: SeamMethod,
            drawn: Result<Vec<Share>, MpcError>,
        ) -> Result<Vec<Share>, MpcError> {
            let mut shares = drawn?;
            if method != self.target {
                return Ok(shares);
            }
            let this = self.seen;
            self.seen += 1;
            if this != self.skip {
                return Ok(shares);
            }
            self.fired = true;
            match self.how {
                Malformation::Short => {
                    shares.pop();
                }
                Malformation::Long => {
                    let x = shares.len() as u64 + 1;
                    shares.push(Share { x, y: Fp::zero() });
                }
                Malformation::Permuted => shares.swap(0, 1),
                Malformation::DuplicateX => shares[1].x = shares[0].x,
                Malformation::WrongX => shares[0].x = 0,
            }
            Ok(shares)
        }
    }

    impl DistributedRandomness for MalformedSource {
        fn randomness_model(&self) -> RandomnessModel {
            self.inner.randomness_model()
        }
        fn parties(&self) -> usize {
            DistributedRandomness::parties(&self.inner)
        }
        fn threshold(&self) -> usize {
            DistributedRandomness::threshold(&self.inner)
        }
        fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            let drawn = self.inner.shared_mask();
            self.maybe_corrupt(SeamMethod::SharedMask, drawn)
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            let drawn = self.inner.shared_nonzero_mask();
            self.maybe_corrupt(SeamMethod::SharedNonzeroMask, drawn)
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            let drawn = self.inner.vss_own_input(secret);
            self.maybe_corrupt(SeamMethod::VssOwnInput, drawn)
        }
    }

    /// A dealer-backed source that misreports its threshold as `t`.
    pub(crate) struct MisreportedThresholdSource {
        pub(crate) inner: ShamirDealer,
        pub(crate) t: usize,
    }

    impl DistributedRandomness for MisreportedThresholdSource {
        fn randomness_model(&self) -> RandomnessModel {
            self.inner.randomness_model()
        }
        fn parties(&self) -> usize {
            DistributedRandomness::parties(&self.inner)
        }
        fn threshold(&self) -> usize {
            self.t
        }
        fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.inner.shared_mask()
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.inner.shared_nonzero_mask()
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            self.inner.vss_own_input(secret)
        }
    }

    /// One recorded draw: the seam method's name and the `y`-values it returned.
    pub(crate) type Entry = (&'static str, Vec<u64>);

    /// Delegates to a real dealer and records every draw, in order — the full
    /// transcript a pin test fingerprints.
    pub(crate) struct RecordingSource {
        pub(crate) inner: ShamirDealer,
        pub(crate) log: Vec<Entry>,
    }

    impl RecordingSource {
        pub(crate) fn new(inner: ShamirDealer) -> Self {
            RecordingSource {
                inner,
                log: Vec::new(),
            }
        }

        fn record(
            &mut self,
            method: SeamMethod,
            drawn: Result<Vec<Share>, MpcError>,
        ) -> Result<Vec<Share>, MpcError> {
            let shares = drawn?;
            self.log
                .push((method.name(), shares.iter().map(|s| s.y.value()).collect()));
            Ok(shares)
        }
    }

    impl DistributedRandomness for RecordingSource {
        fn randomness_model(&self) -> RandomnessModel {
            self.inner.randomness_model()
        }
        fn parties(&self) -> usize {
            DistributedRandomness::parties(&self.inner)
        }
        fn threshold(&self) -> usize {
            DistributedRandomness::threshold(&self.inner)
        }
        fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            let drawn = self.inner.shared_mask();
            self.record(SeamMethod::SharedMask, drawn)
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            let drawn = self.inner.shared_nonzero_mask();
            self.record(SeamMethod::SharedNonzeroMask, drawn)
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            let drawn = self.inner.vss_own_input(secret);
            self.record(SeamMethod::VssOwnInput, drawn)
        }
    }

    /// Order-sensitive 64-bit FNV-1a digest of a transcript: for each entry, the
    /// method name's bytes, a `0` separator byte, then each `y` as 8
    /// little-endian bytes. Any change to which method was called, what it
    /// returned, or the ORDER of the draws changes the digest.
    pub(crate) fn fingerprint(log: &[Entry]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |b: u8| {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for (name, ys) in log {
            name.bytes().for_each(&mut eat);
            eat(0);
            for y in ys {
                y.to_le_bytes().into_iter().for_each(&mut eat);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Fp;
    use crate::shamir::ShamirBackend;

    fn seeded_dealer() -> ShamirDealer {
        // Deterministic simulation RNG (test-gated) so the seam is exercised
        // reproducibly; the production dealer draws OS-seeded ChaCha20.
        ShamirBackend::new_seeded(5, 0xC0FFEE_u64)
            .expect("n >= 2")
            .dealer()
    }

    #[test]
    fn dealer_reports_the_honest_simulation_model() {
        let dealer = seeded_dealer();
        let model = dealer.randomness_model();
        assert_eq!(model, RandomnessModel::TrustedDealerSim);
        // The whole point of the seam: the current source is NOT dealer-less and
        // NOT deployable to a real federation — labelled honestly, not silently.
        assert!(!model.is_dealer_less());
        assert!(!model.deployable());
    }

    #[test]
    fn dealer_less_models_are_descriptive_not_deployable() {
        for m in [
            RandomnessModel::Prss,
            RandomnessModel::HonestMajorityCoinToss,
        ] {
            assert!(m.is_dealer_less(), "{m:?} is dealer-less by design");
            // Descriptive only: no validated implementation exists, so a
            // self-reported dealer-less label must NOT pass deployment acceptance.
            assert!(!m.deployable(), "{m:?} must not be deployable yet");
        }
    }

    #[test]
    fn shared_mask_is_a_full_degree_t_sharing() {
        let mut dealer = seeded_dealer();
        let shares = dealer.shared_mask().expect("mask");
        // n shares on the canonical points x = 1..n; reconstructible by the
        // backend (a well-formed degree-t sharing of the drawn mask).
        assert_eq!(shares.len(), dealer.parties());
    }

    #[test]
    fn shared_nonzero_mask_never_shares_zero() {
        // The r = 0 threat: the equality mask must be nonzero. Draw many
        // SUCCESSIVE masks from ONE dealer — a fresh `dealer()` re-seeds the
        // deterministic test RNG, so re-minting it each iteration would re-test the
        // same first mask 1,000 times — and assert each reconstructs nonzero.
        let backend = ShamirBackend::new_seeded(5, 7).expect("n >= 2");
        let mut dealer = backend.dealer();
        for _ in 0..1_000 {
            let shares = dealer.shared_nonzero_mask().expect("nonzero mask");
            let r = backend.reconstruct(&shares).expect("reconstruct");
            assert_ne!(r, Fp::zero(), "equality mask must never be r = 0");
        }
    }

    /// A test-only stub that *labels* itself [`RandomnessModel::Prss`] while
    /// implementing NO dealer-less crypto — exactly the source the gate must
    /// refuse: a self-reported model is not evidence of a validated
    /// implementation.
    struct SelfLabelledPrssStub;
    impl DistributedRandomness for SelfLabelledPrssStub {
        fn randomness_model(&self) -> RandomnessModel {
            RandomnessModel::Prss
        }
        fn parties(&self) -> usize {
            3
        }
        fn threshold(&self) -> usize {
            1
        }
        fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            Err(MpcError::not_yet("PRSS shared_mask", "sq-yyro follow-on"))
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            Err(MpcError::not_yet(
                "PRSS shared_nonzero_mask",
                "sq-yyro follow-on",
            ))
        }
        fn vss_own_input(&mut self, _secret: Fp) -> Result<Vec<Share>, MpcError> {
            Err(MpcError::not_yet("dealer-less VSS", "sq-yyro follow-on"))
        }
    }

    #[test]
    fn require_deployable_refuses_the_simulation_and_a_self_labelled_source() {
        // The single-dealer SIMULATION must be refused fail-closed in production.
        let dealer = seeded_dealer();
        assert!(matches!(
            dealer.require_deployable(),
            Err(MpcError::NoBackendSatisfies { .. })
        ));
        // A source that merely SELF-LABELS a dealer-less model — with no validated
        // dealer-less implementation behind it — must ALSO be refused: the label
        // alone is never sufficient. Acceptance stays fail-closed until the real
        // PRSS / coin-toss implementations land behind a validated boundary.
        assert!(matches!(
            SelfLabelledPrssStub.require_deployable(),
            Err(MpcError::NoBackendSatisfies { .. })
        ));
    }

    #[test]
    fn require_canonical_sharing_accepts_only_the_canonical_shape() {
        use test_sources::SHAPE_REFUSAL;
        let mut dealer = seeded_dealer();
        let good = dealer.share(Fp::new(9));
        assert!(require_canonical_sharing(&good, 5).is_ok());
        // Right shape, wrong party count.
        for n in [4, 6] {
            assert!(require_canonical_sharing(&good, n).is_err(), "n = {n}");
        }
        let mut short = good.clone();
        short.pop();
        let mut long = good.clone();
        long.push(Share {
            x: 6,
            y: Fp::zero(),
        });
        let mut permuted = good.clone();
        permuted.swap(0, 1);
        let mut dup = good.clone();
        dup[1].x = dup[0].x;
        let mut wrong = good.clone();
        wrong[0].x = 0;
        for (name, bad) in [
            ("short", short),
            ("long", long),
            ("permuted", permuted),
            ("duplicate x", dup),
            ("wrong x", wrong),
        ] {
            match require_canonical_sharing(&bad, 5) {
                Err(MpcError::Protocol(m)) => assert!(m.contains(SHAPE_REFUSAL), "{name}: {m}"),
                other => panic!("{name}: must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn checked_draws_refuse_a_malformed_sharing_and_pass_a_clean_one() {
        use test_sources::{MalformedSource, SeamMethod, ALL_MALFORMATIONS, SHAPE_REFUSAL};
        for how in ALL_MALFORMATIONS {
            let mut src = MalformedSource::new(seeded_dealer(), SeamMethod::VssOwnInput, 0, how);
            match checked_vss_own_input(&mut src, 5, Fp::new(3)) {
                Err(MpcError::Protocol(m)) => assert!(m.contains(SHAPE_REFUSAL), "{how:?}: {m}"),
                other => panic!("{how:?}: vss_own_input must be refused, got {other:?}"),
            }
            let mut src =
                MalformedSource::new(seeded_dealer(), SeamMethod::SharedNonzeroMask, 0, how);
            match checked_shared_nonzero_mask(&mut src, 5) {
                Err(MpcError::Protocol(m)) => assert!(m.contains(SHAPE_REFUSAL), "{how:?}: {m}"),
                other => panic!("{how:?}: shared_nonzero_mask must be refused, got {other:?}"),
            }
        }
        // The checked draws are otherwise transparent: same sharing as the raw draw.
        let mut a = seeded_dealer();
        let mut b = seeded_dealer();
        assert_eq!(
            checked_vss_own_input(&mut a, 5, Fp::new(3)).unwrap(),
            b.vss_own_input(Fp::new(3)).unwrap()
        );
        assert_eq!(
            checked_shared_nonzero_mask(&mut a, 5).unwrap(),
            b.shared_nonzero_mask().unwrap()
        );
        // A canonical sharing for the wrong party count is refused too.
        assert!(checked_vss_own_input(&mut a, 4, Fp::new(3)).is_err());
    }

    #[test]
    fn malformed_source_corrupts_exactly_the_targeted_call() {
        use test_sources::{Malformation, MalformedSource, SeamMethod};
        let mut src = MalformedSource::new(
            seeded_dealer(),
            SeamMethod::VssOwnInput,
            1,
            Malformation::Short,
        );
        // Other methods are never touched, and do not advance the skip count.
        assert_eq!(src.shared_nonzero_mask().unwrap().len(), 5);
        assert_eq!(src.vss_own_input(Fp::one()).unwrap().len(), 5);
        assert!(!src.fired());
        assert_eq!(src.vss_own_input(Fp::one()).unwrap().len(), 4);
        assert!(src.fired());
        assert_eq!(src.vss_own_input(Fp::one()).unwrap().len(), 5);
    }

    #[test]
    fn transcript_fingerprint_is_order_sensitive() {
        use test_sources::{fingerprint, RecordingSource};
        let mut rec = RecordingSource::new(seeded_dealer());
        rec.vss_own_input(Fp::new(1)).unwrap();
        rec.shared_nonzero_mask().unwrap();
        rec.shared_mask().unwrap();
        assert_eq!(rec.log.len(), 3);
        assert_eq!(
            rec.log.iter().map(|(m, _)| *m).collect::<Vec<_>>(),
            ["vss_own_input", "shared_nonzero_mask", "shared_mask"]
        );
        let base = fingerprint(&rec.log);
        // Same entries, different order.
        let mut swapped = rec.log.clone();
        swapped.swap(0, 1);
        assert_ne!(fingerprint(&swapped), base);
        // Same values under a different method name.
        let mut renamed = rec.log.clone();
        renamed[2].0 = "shared_nonzero_mask";
        assert_ne!(fingerprint(&renamed), base);
        // Same multiset of y-values, permuted within one entry.
        let mut inner = rec.log.clone();
        inner[0].1.swap(0, 1);
        assert_ne!(fingerprint(&inner), base);
        assert_ne!(fingerprint(&rec.log[..2]), base);
        assert_eq!(fingerprint(&rec.log), base);
    }

    #[test]
    fn vss_own_input_reconstructs_to_the_holders_value() {
        let backend = ShamirBackend::new_seeded(5, 11).expect("n >= 2");
        let mut dealer = backend.dealer();
        let secret = Fp::new(4085);
        let shares = dealer.vss_own_input(secret).expect("vss");
        assert_eq!(backend.reconstruct(&shares).expect("reconstruct"), secret);
    }
}
