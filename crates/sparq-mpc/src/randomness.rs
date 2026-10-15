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
use crate::oblivious::{SecretColumn, ShuffleCost};
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
/// gated on it yet, although `degree_reduce`, the hidden-value equality path, the
/// whole comparison chain, the hidden DISTINCT / property-path stages, the
/// private-input sharing, the batched join's key sharing and the oblivious output
/// stage (its selector / tag / payload-limb sharings and its shuffle) already draw
/// through this trait).
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
    /// ([`crate::join::HiddenValueJoin`]), the [`crate::compare`] chain, the
    /// [`crate::hidden_distinct`] / [`crate::hidden_path`] stages, the
    /// [`crate::shamir::ShamirBackend`] private-input sharing and
    /// [`crate::join::BatchedHiddenInput::shared_keys`] and the oblivious output
    /// stage ([`crate::oblivious_join`]'s selector / tag / payload-limb sharings
    /// and its shuffle, via [`oblivious_shuffle`](Self::oblivious_shuffle)) draw
    /// through the seam; the authenticated (IT-MAC) twins, `transport`, `bench`
    /// and `draw_bit` still call the dealer directly.
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

    /// **Oblivious shuffle** of a secret-shared column: return a re-ordering of
    /// `col`'s entries under a permutation the source chooses, together with the
    /// MODELLED cost ([`ShuffleCost`]). This is the randomness-side half of
    /// [`crate::oblivious::shuffle`]: the permutation is correlated randomness
    /// like any mask, so WHO draws it belongs behind this seam.
    ///
    /// **Structural contract only.** Every implementor returns a column of the
    /// same entry count whose entries have the same widths, and whose entries are
    /// the input's entries re-ordered — nothing else is promised here. In
    /// particular:
    ///
    /// - *Who knows the permutation* is NOT part of this contract. The
    ///   [`RandomnessModel::TrustedDealerSim`] implementor samples the permutation
    ///   centrally and therefore *knows* it; a dealer-less implementor is
    ///   designed so no `≤ t` parties do. A caller that needs the permutation
    ///   hidden MUST gate on [`require_deployable`](Self::require_deployable) — a
    ///   necessary, not sufficient, check.
    /// - *Nothing is verified.* No implementor proves that what it returned is a
    ///   permutation of the input at all, so a deviating party can corrupt or
    ///   reorder entries undetected. Semi-honest only; the consumer-side
    ///   `checked_oblivious_shuffle` adds a SHAPE check and nothing more.
    ///
    /// ## An implementor MAY require degree-`t` entries — the caller's obligation
    ///
    /// A shuffle that only moves whole entries never reads a share, so it is
    /// **degree-agnostic**: [`ShamirDealer`]'s switch-network implementation
    /// relabels share-vectors and carries any degree through untouched. A shuffle
    /// that *recombines* — converting each element between sharings, as a
    /// dealer-less source must — interpolates from `t+1` points, which is only
    /// faithful at degree `<= t`; handed a degree-`2t` element it would return a
    /// well-formed degree-`t` sharing of a DIFFERENT secret, which no later
    /// reconstruction could detect.
    ///
    /// So an implementor is entitled to require that every element of every entry
    /// be a degree-`<= t` sharing on the canonical points, and to refuse with
    /// [`MpcError::Protocol`] otherwise — [`crate::prss::PrssRandomness`] does
    /// exactly that. A caller holding degree-`2t` elements (the oblivious output
    /// stage's `selector · real_tag` products) must **degree-reduce them first**
    /// (`crate::shamir::degree_reduce_through`) before shuffling through a
    /// recombining source. Which sources recombine is not something a generic
    /// caller can inspect, so the safe discipline is: shuffle degree-`t` columns.
    ///
    /// ## The returned shares may be FRESH — do not rely on share identity
    ///
    /// What this method preserves is the **reconstructed values** and their
    /// per-entry binding, not the share vectors themselves. A switch-network
    /// implementor hands back the very same `Share`s in a new order, but a
    /// recombining one returns **fresh sharings** of the permuted values — new
    /// masking polynomials, so every `y` differs even though the entry
    /// reconstructs to the same tuple. A caller must therefore never match an
    /// output entry to an input entry by share equality, cache a share across the
    /// call, or assume the output is a sub-multiset of the input's share vectors.
    /// (Fresh sharings are a feature, not a wart: re-randomising is what keeps a
    /// resharing shuffle's output unlinkable to its input.)
    ///
    /// ## Ordered delivery is a TRANSPORT obligation, not enforced here
    ///
    /// This crate is an in-process simulation, so "sending" a resharing is a
    /// function call and the loop that produces them is invisible. A real
    /// deployment runs the same construction over private channels, and there the
    /// per-round resharing batches MUST be emitted in **output-position order**:
    /// exposing a source index or a per-element counter alongside each batch, or
    /// emitting them in input order so the timing tracks it, hands the adversary
    /// the very permutation the shuffle exists to hide. Nothing in this crate
    /// enforces that today — there is no wire format for it yet — so it is an
    /// obligation on the network / transport layer when that lands, recorded here
    /// so it is not discovered later.
    fn oblivious_shuffle(
        &mut self,
        col: SecretColumn,
    ) -> Result<(SecretColumn, ShuffleCost), MpcError>;
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

    fn oblivious_shuffle(
        &mut self,
        col: SecretColumn,
    ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
        // Simulation artefact: the dealer samples AND knows the permutation (it
        // drew it from its own masking RNG), then routes it through the
        // Waksman network with CLEARTEXT control bits. A dealer-less source
        // derives the permutation from randomness no `<= t` parties hold.
        crate::oblivious::shuffle(self, col)
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

/// [`DistributedRandomness::oblivious_shuffle`] followed by a **SHAPE** check on
/// the returned column: the same number of entries as the input, and the same
/// multiset of entry widths.
///
/// The consumer of a shuffled column splits every entry back into fixed-width
/// slices by position ([`crate::oblivious_join`] splits `[tag | limb₁ | … ]`),
/// so an entry that lost or gained a share, or a column that lost or gained an
/// entry, would silently mis-decode a row. The source is caller-supplied, so its
/// output shape is checked here rather than trusted and a mismatch is refused
/// with [`MpcError::Protocol`].
///
/// **Shape only — this is not a correctness check.** It does NOT verify that the
/// output is a permutation of the input, that any entry's shares still lie on the
/// polynomial they came from, or that the permutation is uniform or hidden. The
/// width multiset is compared order-independently precisely because a shuffle is
/// *expected* to re-order entries; a source that returned the input unchanged, or
/// returned entries built from unrelated shares of the right widths, passes.
///
/// It deliberately compares WIDTHS and not shares: a recombining source returns
/// **fresh sharings** of the permuted values (see
/// [`DistributedRandomness::oblivious_shuffle`]), so every `y` legitimately
/// differs from the input's. Only the reconstructed values and their per-entry
/// binding are preserved across the call, and neither is checked here.
pub(crate) fn checked_oblivious_shuffle(
    source: &mut dyn DistributedRandomness,
    col: SecretColumn,
) -> Result<(SecretColumn, ShuffleCost), MpcError> {
    let mut before: Vec<usize> = col.iter().map(|e| e.len()).collect();
    let (shuffled, cost) = source.oblivious_shuffle(col)?;
    let mut after: Vec<usize> = shuffled.iter().map(|e| e.len()).collect();
    if before.len() != after.len() {
        return Err(MpcError::Protocol(format!(
            "the randomness source returned a shuffled column of {} entries, but it was \
             handed {}",
            after.len(),
            before.len()
        )));
    }
    before.sort_unstable();
    after.sort_unstable();
    if before != after {
        return Err(MpcError::Protocol(format!(
            "the randomness source returned a shuffled column whose entry widths {:?} are \
             not a re-ordering of the widths {:?} it was handed",
            after, before
        )));
    }
    Ok((shuffled, cost))
}

/// Test-only [`DistributedRandomness`] sources shared by the seam tests of every
/// module that draws through the seam: a source that corrupts exactly one
/// targeted draw, one that misreports its threshold, one that records the full
/// draw transcript, and one that merely counts the draws made through it.
#[cfg(test)]
pub(crate) mod test_sources {
    use super::*;

    /// A seam method that draws a sharing.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum SeamMethod {
        SharedMask,
        SharedNonzeroMask,
        VssOwnInput,
        ObliviousShuffle,
    }

    impl SeamMethod {
        fn name(self) -> &'static str {
            match self {
                SeamMethod::SharedMask => "shared_mask",
                SeamMethod::SharedNonzeroMask => "shared_nonzero_mask",
                SeamMethod::VssOwnInput => "vss_own_input",
                SeamMethod::ObliviousShuffle => "oblivious_shuffle",
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
        /// Passed through untouched: [`Malformation`] corrupts a single
        /// `Vec<Share>` sharing, which is the wrong shape for a shuffled column —
        /// [`MalformedShuffleSource`] is the column-shaped twin.
        fn oblivious_shuffle(
            &mut self,
            col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            self.inner.oblivious_shuffle(col)
        }
    }

    /// How [`MalformedShuffleSource`] corrupts the SHAPE of the shuffled column.
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum ShuffleMalformation {
        /// Drop an entry — the column comes back one row short.
        DropEntry,
        /// Append an entry — the column comes back one row long.
        AddEntry,
        /// Truncate one entry — that row comes back one share narrow.
        NarrowFirst,
        /// Extend one entry — that row comes back one share wide.
        WidenFirst,
    }

    /// Every [`ShuffleMalformation`], for tests that sweep them all.
    pub(crate) const ALL_SHUFFLE_MALFORMATIONS: [ShuffleMalformation; 4] = [
        ShuffleMalformation::DropEntry,
        ShuffleMalformation::AddEntry,
        ShuffleMalformation::NarrowFirst,
        ShuffleMalformation::WidenFirst,
    ];

    /// The substring of [`checked_oblivious_shuffle`]'s two refusals, for
    /// asserting that a malformed shuffle was refused by the shape check itself
    /// and not by something downstream.
    pub(crate) const SHUFFLE_SHAPE_REFUSAL: &str = "returned a shuffled column";

    /// Delegates to a real dealer but corrupts the SHAPE of the shuffled column
    /// it returns. Every other draw is passed through untouched.
    pub(crate) struct MalformedShuffleSource {
        inner: ShamirDealer,
        how: ShuffleMalformation,
        fired: bool,
    }

    impl MalformedShuffleSource {
        pub(crate) fn new(inner: ShamirDealer, how: ShuffleMalformation) -> Self {
            MalformedShuffleSource {
                inner,
                how,
                fired: false,
            }
        }

        /// Whether the shuffle happened (and its column was corrupted).
        pub(crate) fn fired(&self) -> bool {
            self.fired
        }
    }

    impl DistributedRandomness for MalformedShuffleSource {
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
            self.inner.shared_mask()
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.inner.shared_nonzero_mask()
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            self.inner.vss_own_input(secret)
        }
        fn oblivious_shuffle(
            &mut self,
            col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            let (mut shuffled, cost) = self.inner.oblivious_shuffle(col)?;
            self.fired = true;
            match self.how {
                ShuffleMalformation::DropEntry => {
                    shuffled.pop();
                }
                ShuffleMalformation::AddEntry => {
                    let width = shuffled.last().map(|e| e.len()).unwrap_or(0);
                    shuffled.push(vec![
                        Share {
                            x: 1,
                            y: Fp::zero()
                        };
                        width
                    ]);
                }
                ShuffleMalformation::NarrowFirst => {
                    if let Some(entry) = shuffled.first_mut() {
                        entry.pop();
                    }
                }
                ShuffleMalformation::WidenFirst => {
                    if let Some(entry) = shuffled.first_mut() {
                        entry.push(Share {
                            x: 1,
                            y: Fp::zero(),
                        });
                    }
                }
            }
            Ok((shuffled, cost))
        }
    }

    /// A dealer-backed source that keeps the shuffled column's SHAPE intact but
    /// rewrites VALUES in it, so the corruption survives the shape check and has to
    /// be caught by whatever the consumer checks about the opened values.
    ///
    /// Unlike [`MalformedShuffleSource`], which only breaks the shape, this one
    /// returns a column that is still exactly `entries × width` canonical sharings.
    /// Re-sharing a value through the inner dealer keeps every element a consistent
    /// degree-`t` sharing, so a robust open accepts it and the only thing that can
    /// notice is a check on the value itself.
    pub(crate) struct TamperingShuffleSource {
        inner: ShamirDealer,
        how: ShuffleTampering,
        fired: bool,
    }

    /// What [`TamperingShuffleSource`] rewrites in the shuffled column.
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum ShuffleTampering {
        /// Re-share `value` over every element of every entry PAST the first
        /// `keep_prefix` elements, as a consistent degree-`t` sharing. With
        /// `keep_prefix = 1` this keeps each entry's first element (an output
        /// stage's tag) and rewrites all of its payload limbs.
        ResharePast { keep_prefix: usize, value: u64 },
    }

    impl TamperingShuffleSource {
        pub(crate) fn new(inner: ShamirDealer, how: ShuffleTampering) -> Self {
            TamperingShuffleSource {
                inner,
                how,
                fired: false,
            }
        }

        /// Whether the shuffle happened (and its column was tampered with).
        pub(crate) fn fired(&self) -> bool {
            self.fired
        }
    }

    impl DistributedRandomness for TamperingShuffleSource {
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
            self.inner.shared_mask()
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.inner.shared_nonzero_mask()
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            self.inner.vss_own_input(secret)
        }
        fn oblivious_shuffle(
            &mut self,
            col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            let n = DistributedRandomness::parties(&self.inner);
            let (mut shuffled, cost) = self.inner.oblivious_shuffle(col)?;
            self.fired = true;
            match self.how {
                ShuffleTampering::ResharePast { keep_prefix, value } => {
                    let keep = keep_prefix * n;
                    for entry in shuffled.iter_mut() {
                        if entry.len() <= keep {
                            continue;
                        }
                        let elements = (entry.len() - keep) / n;
                        let mut rewritten = entry[..keep].to_vec();
                        for _ in 0..elements {
                            rewritten.extend(self.inner.vss_own_input(Fp::new(value))?);
                        }
                        // Keep the width byte-exact even if it was not a multiple
                        // of `n` (the shape check would catch that on its own).
                        rewritten.truncate(entry.len());
                        while rewritten.len() < entry.len() {
                            rewritten.push(Share {
                                x: rewritten.len() as u64 % n as u64 + 1,
                                y: Fp::zero(),
                            });
                        }
                        *entry = rewritten;
                    }
                }
            }
            Ok((shuffled, cost))
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
        fn oblivious_shuffle(
            &mut self,
            col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            self.inner.oblivious_shuffle(col)
        }
    }

    /// Counts the draws made through it and delegates EVERYTHING else — including
    /// the reported `(n, t)` — to the wrapped source.
    ///
    /// [`RecordingSource`] can only wrap a real [`ShamirDealer`], which always
    /// reports the backend's own `(n, t)`; this wrapper takes any
    /// [`DistributedRandomness`], so a test can wrap a
    /// [`MisreportedThresholdSource`] and still assert that an up-front source
    /// check refused it before a single draw.
    pub(crate) struct DrawCountingSource<'a> {
        inner: &'a mut dyn DistributedRandomness,
        draws: usize,
    }

    impl<'a> DrawCountingSource<'a> {
        pub(crate) fn new(inner: &'a mut dyn DistributedRandomness) -> Self {
            DrawCountingSource { inner, draws: 0 }
        }

        /// How many draws (of any seam method) have gone through this wrapper.
        pub(crate) fn draws(&self) -> usize {
            self.draws
        }
    }

    impl DistributedRandomness for DrawCountingSource<'_> {
        fn randomness_model(&self) -> RandomnessModel {
            self.inner.randomness_model()
        }
        fn parties(&self) -> usize {
            self.inner.parties()
        }
        fn threshold(&self) -> usize {
            self.inner.threshold()
        }
        fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.draws += 1;
            self.inner.shared_mask()
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.draws += 1;
            self.inner.shared_nonzero_mask()
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            self.draws += 1;
            self.inner.vss_own_input(secret)
        }
        fn oblivious_shuffle(
            &mut self,
            col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            self.draws += 1;
            self.inner.oblivious_shuffle(col)
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
        /// Records the shuffled column's shares, entry by entry in the order the
        /// shuffle returned them, flattened into one entry of the transcript — so
        /// the digest moves if the permutation, any entry's shares, or the entry
        /// order changes.
        fn oblivious_shuffle(
            &mut self,
            col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            let (shuffled, cost) = self.inner.oblivious_shuffle(col)?;
            self.log.push((
                SeamMethod::ObliviousShuffle.name(),
                shuffled.iter().flatten().map(|s| s.y.value()).collect(),
            ));
            Ok((shuffled, cost))
        }
    }

    /// Order-sensitive 64-bit FNV-1a digest of a transcript: for each entry, the
    /// method name's bytes, a `0` separator byte, then each `y` as 8
    /// little-endian bytes. Any change to which method was called, what it
    /// returned, or the ORDER of the draws changes the digest.
    /// The transcript PREFIX that runs before the first oblivious shuffle, for a
    /// prefix-equality pin.
    ///
    /// The shuffle is the one seam call whose RECORDED payload is its own output,
    /// so a change to what the shuffle hands back moves the whole-log fingerprint
    /// while leaving every earlier entry untouched. Pinning this prefix separately
    /// states that: the draws an operator makes before it shuffles are byte-for-byte
    /// what they were, and only the shuffle's output and the draws after it moved.
    /// Returns the whole log when it contains no shuffle.
    pub(crate) fn prefix_before_first_shuffle(log: &[Entry]) -> &[Entry] {
        let cut = log
            .iter()
            .position(|e| e.0 == SeamMethod::ObliviousShuffle.name())
            .unwrap_or(log.len());
        &log[..cut]
    }

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
        fn oblivious_shuffle(
            &mut self,
            _col: SecretColumn,
        ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
            Err(MpcError::not_yet(
                "PRSS oblivious_shuffle",
                "sq-yyro follow-on",
            ))
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
    fn draw_counting_source_counts_every_draw_and_reports_the_inner_n_and_t() {
        use test_sources::{DrawCountingSource, MisreportedThresholdSource};
        let mut lying = MisreportedThresholdSource {
            inner: seeded_dealer(),
            t: 0,
        };
        let mut counted = DrawCountingSource::new(&mut lying);
        // The wrapper is transparent about (n, t): it passes the INNER source's
        // misreported threshold through, which is the whole reason it exists.
        assert_eq!(DistributedRandomness::parties(&counted), 5);
        assert_eq!(DistributedRandomness::threshold(&counted), 0);
        assert_eq!(counted.draws(), 0);
        // Every seam method counts, once per call.
        counted.shared_mask().unwrap();
        assert_eq!(counted.draws(), 1);
        counted.shared_nonzero_mask().unwrap();
        counted.vss_own_input(Fp::new(7)).unwrap();
        assert_eq!(counted.draws(), 3);
        let col = vec![counted.vss_own_input(Fp::new(1)).unwrap()];
        assert_eq!(counted.draws(), 4);
        counted.oblivious_shuffle(col).unwrap();
        assert_eq!(counted.draws(), 5);
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
    fn require_source_matches_refuses_any_n_or_t_mismatch() {
        use test_sources::MisreportedThresholdSource;
        let backend = ShamirBackend::new_seeded(5, 0xC0FFEE).unwrap();
        assert!(require_source_matches(&backend.dealer(), &backend).is_ok());
        // A dealer-less source with the same (n, t) is accepted: the check is about
        // composition, not about the regime.
        let prss = crate::prss::PrssRandomness::with_simulated_setup(5).unwrap();
        assert!(require_source_matches(&prss, &backend).is_ok());

        let other_n = ShamirBackend::new_seeded(3, 1).unwrap().dealer();
        let mut refused: Vec<Box<dyn DistributedRandomness>> = vec![Box::new(other_n)];
        for t in [0, 1, 3, usize::MAX] {
            refused.push(Box::new(MisreportedThresholdSource {
                inner: backend.dealer(),
                t,
            }));
        }
        for source in &refused {
            match require_source_matches(source.as_ref(), &backend) {
                Err(MpcError::Protocol(m)) => assert!(
                    m.contains("but the backend is (n, t) = (5, 2)"),
                    "n = {}, t = {}: {m}",
                    source.parties(),
                    source.threshold()
                ),
                other => panic!(
                    "n = {}, t = {}: must be refused, got {other:?}",
                    source.parties(),
                    source.threshold()
                ),
            }
        }
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
