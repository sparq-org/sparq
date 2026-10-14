// [OPUS-4.8] MpcBackend trait — abstracts the secret-sharing / MPC primitive.
//! The MPC primitive abstraction. The trait is primitive-agnostic; the first
//! concrete impl is [`crate::shamir::ShamirBackend`] (honest-majority, M3).
//!
//! Architecture refs: §3.1 (secret-sharing families & network model), §4.2
//! (trust & threat model), and decision point **§5.2 Q2** (resolved for v1 =
//! honest-majority; configurable long-term).
//!
//! ## The decision point this trait exists to defer (Q2)
//!
//! The whole MPC primitive choice hinges on the trust model, and the literature
//! splits cleanly (architecture §3.1, §4.2):
//!
//! - **Honest-majority** (e.g. replicated 3PC secret sharing): the performance
//!   sweet spot, and — critically — the ONLY regime in which malicious-secure
//!   query-evaluation correctness (Senate/ORQ) AND authenticated inputs (Dutta,
//!   the attestation pillar) are both demonstrated.
//! - **Dishonest-majority malicious** (SPDZ/MASCOT/Overdrive): the realistic
//!   cross-org model for a hostile-pod federation, but pays expensive
//!   input-independent preprocessing and has NO demonstrated query-eval
//!   correctness in the cited literature.
//!
//! Network model compounds it: secret-sharing wins on LAN, garbled/constant-
//! round wins on WAN (the federated setting). The architecture's honest verdict
//! (§5.3) is that the *viable* first target is honest-majority, LAN/datacenter,
//! small data — and that whether the four-flatmates use case (cooperating
//! holders vs an external landlord) even NEEDS dishonest-majority among holders
//! is itself unresolved.
//!
//! **Therefore this trait commits to NO primitive.** It defines the seam so an
//! honest-majority impl (M3, first) and a dishonest-majority impl (later, if
//! Q2 resolves that way) are swappable without touching the join or proof
//! layers. Convention #7 (modularity is the contribution) demands exactly this.
//!
//! ## Q2 RESOLVED for v1 + how the trust model stays CONFIGURABLE
//!
//! Jesse's decision: **honest-majority for v1, configurable long-term.** The
//! concrete v1 impl is [`crate::shamir::ShamirBackend`] (honest-majority Shamir
//! `t`-of-`n`, semi-honest). The trait keeps that choice swappable:
//!
//! - **Callers select a backend by [`TrustModel`], never by concrete type.** A
//!   federation inspects [`BackendInfo`] and refuses a backend whose guarantees
//!   don't match its threat model. The join/proof layers are written against the
//!   `MpcBackend` trait (e.g. [`crate::join::HiddenValueJoin`] takes *a backend*,
//!   not "Shamir"), so substituting a dishonest-majority impl touches NO caller.
//! - **The associated [`MpcBackend::Share`] type absorbs the scheme change.**
//!   Shamir's share is a per-party polynomial-point vector; a SPDZ-style
//!   dishonest-majority backend's share is an *authenticated* additive share
//!   (value + MAC tag) with a preprocessing (triples) phase. Both hide behind
//!   `type Share`, so the difference never leaks into the join/proof signatures.
//! - **What a dishonest-majority (SPDZ/MASCOT/Overdrive) backend would add, all
//!   BEHIND this trait:** (1) an input-independent preprocessing step producing
//!   Beaver triples + MACs (a private step, not a trait change); (2) `run_secure`
//!   consuming triples for multiplications and tracking MACs; (3)
//!   `reconstruct_disclosed` doing a MAC-check before opening (abort on cheat →
//!   guarantee (D), malicious security). It reports
//!   `TrustModel::DishonestMajority` + a non-`SemiHonestOnly`
//!   [`MaliciousSecurity`] via [`BackendInfo`]. Crucially the
//!   `share_private_input` / `run_secure` /
//!   `reconstruct_disclosed` SIGNATURES are unchanged, so
//!   [`crate::join::HiddenValueJoin`] and the future collaborative-proof layer
//!   compose onto it unmodified.
//!
//! No trust model is hardcoded into the join/protocol layer: it is a property of
//! the chosen `MpcBackend` value, surfaced via [`BackendInfo`].

use crate::holder::Holder;
use crate::partial::{MpcError, PartialResult};

// =============================================================================
// [OPUS-4.8] sq-mq8q — THREE-AXIS SECURITY DESCRIPTOR (Fable unavailable;
// re-review when Fable returns).
//
// Background: the original two enums ([`TrustModel`] + [`MaliciousSecurity`])
// FLATTENED what the literature treats as THREE genuinely-orthogonal axes
// (research/mpc-security-models-and-benchmarks.md §1.2). `TrustModel` was a
// *binary* take on the corruption-threshold axis (it lost the n/3-vs-n/2 split
// that separates perfect-GOD from statistical-GOD), and `MaliciousSecurity`
// BUNDLED the adversary axis (semi-honest vs active) WITH the output-guarantee
// axis (abort vs robust) AND soldered "HonestMajority" into the *variant names*
// — so dishonest-majority-malicious-abort (= the SPDZ regime) was UNNAMEABLE,
// and covert / identifiable-vs-unanimous abort had NO representation.
//
// This module replaces that flattening with three orthogonal axes
// ([`AdversaryModel`], [`OutputGuarantee`], [`CorruptionThreshold`]) plus a
// [`PublicVerifiability`] marker, composed into a [`SecurityDescriptor`].
// Cleve's impossibility (STOC'86 — fairness/GOD is impossible without an honest
// majority) is encoded as a TYPE-LEVEL INVARIANT: the stronger output
// guarantees are only constructible under an honest-majority threshold (see
// [`OutputGuarantee::fairness`] / [`OutputGuarantee::guaranteed_output`], which
// are the ONLY way to build those variants and which take the threshold as
// proof of honest majority). The old [`MaliciousSecurity`] enum and the
// [`BackendInfo::malicious_security`] field are KEPT as a back-compat
// PROJECTION (see [`SecurityDescriptor::malicious_security`]) so
// `ShamirBackend::info()` and every existing `BackendInfo` caller still compile
// unchanged. Guarantees are reported PER-OPERATOR ([`OperatorClass`] /
// [`MpcBackend::operator_security`]) because they genuinely differ — the
// degree-`t` aggregate is robust while the degree-`2t` equality open is
// semi-honest-only at `n = 2t+1`, and one backend-level bit would lie.
//
// Scope: this is a CONTROLLED enum refactor behind the unchanged `MpcBackend`
// trait, NOT a rewrite. The fail-closed selection registry that consumes these
// axes is a SEPARATE concern — tracked as bead sq-a6p1 (depends on this).
// =============================================================================

/// AXIS-1 — the **adversary model**: how a corrupt party is allowed to behave
/// (research §1.2). Orthogonal to the corruption *threshold* ([`CorruptionThreshold`],
/// AXIS-3) and to the *output guarantee* ([`OutputGuarantee`], AXIS-2). Ordered
/// weakest → strongest. `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdversaryModel {
    /// **Passive / honest-but-curious.** A corrupt party follows the protocol
    /// faithfully but tries to infer secrets from its view. The v1 regime among
    /// cooperating holders (Shamir semi-honest).
    SemiHonest,
    /// **Covert (ε-deterrence).** A corrupt party MAY deviate arbitrarily but is
    /// *caught with probability ε* (Aumann–Lindell'07); a PVC variant adds a
    /// publicly-verifiable cheating certificate (see [`PublicVerifiability`]).
    /// The genuine middle tier between semi-honest and malicious — absent from
    /// the old enums. ε is carried as an exact rational `deterrence_num /
    /// deterrence_den` (e.g. `1/2`), avoiding a float in a security descriptor.
    ///
    /// **Invariant (now enforced, not just documented).** ε must be a valid
    /// probability in `(0, 1]`: `deterrence_den > 0` and `deterrence_num <=
    /// deterrence_den`. The fields are PRIVATE and the variant carries a private
    /// witness token, so it is unbuildable except through [`AdversaryModel::covert`],
    /// which range-checks and returns `None` on a nonsensical ε. Read the ε back
    /// via [`AdversaryModel::deterrence`]. This mirrors the Cleve type-invariant on
    /// [`OutputGuarantee`]. `[OPUS-4.8]` (Copilot review #87).
    Covert {
        /// Numerator of the deterrence probability ε. Private — set only by the
        /// range-checking [`AdversaryModel::covert`] constructor.
        deterrence_num: u32,
        /// Denominator of the deterrence probability ε. Private; the constructor
        /// guarantees `> 0` and `>= deterrence_num`.
        deterrence_den: u32,
        /// Zero-sized witness that [`AdversaryModel::covert`] validated the ε
        /// invariant — its private type bars any struct-literal escape hatch.
        _checked: private::CovertEpsilonProof,
    },
    /// **Active / malicious.** A corrupt party may deviate from the protocol
    /// arbitrarily. The output guarantee under this adversary is determined by
    /// AXIS-2 ([`OutputGuarantee`]) and constrained by AXIS-3 (Cleve).
    Malicious,
}

impl AdversaryModel {
    /// Construct [`AdversaryModel::Covert`] with deterrence probability
    /// ε = `num / den`, range-checking the invariant: `den > 0` and `num <= den`,
    /// so ε is a valid probability in `(0, 1]`. Returns `None` for a nonsensical ε
    /// (zero denominator, or ε > 1) instead of silently building a descriptor that
    /// misrepresents the deterrence security parameter. This is the ONLY way to
    /// obtain a `Covert` value (its fields and witness token are private).
    /// `num == 0` (ε = 0, i.e. no deterrence at all) is rejected too: that is the
    /// semi-honest regime — use [`AdversaryModel::SemiHonest`]. `[OPUS-4.8]`
    pub fn covert(num: u32, den: u32) -> Option<Self> {
        if den == 0 || num == 0 || num > den {
            return None;
        }
        Some(AdversaryModel::Covert {
            deterrence_num: num,
            deterrence_den: den,
            _checked: private::CovertEpsilonProof(()),
        })
    }

    /// The deterrence probability ε = `(num, den)` if this is a covert adversary,
    /// else `None`. The returned pair always satisfies the constructor invariant
    /// (`den > 0`, `0 < num <= den`). `[OPUS-4.8]`
    pub fn deterrence(self) -> Option<(u32, u32)> {
        match self {
            AdversaryModel::Covert {
                deterrence_num,
                deterrence_den,
                ..
            } => Some((deterrence_num, deterrence_den)),
            _ => None,
        }
    }
}

/// The flavour of an `abort` output guarantee (research §1.1 rows 3/4): what the
/// honest parties learn on a detected cheat. Ordered weakest → strongest.
/// `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortKind {
    /// **Selective abort.** A cheat is detected and the protocol aborts, but the
    /// adversary may cause only *some* honest parties to abort.
    Selective,
    /// **Unanimous abort.** On a detected cheat, *all* honest parties agree to
    /// abort (no split-brain), but no cheater is attributed. The SPDZ-style
    /// detect-and-abort guarantee.
    Unanimous,
    /// **Identifiable abort (IA).** On abort, the honest parties additionally
    /// AGREE on (at least one) cheater — sound cheater attribution. NB: the
    /// current Shamir backend's `Tampered{cheaters}` set is *heuristic* on the
    /// abort path, NOT a sound IA, so the backend must not advertise this for
    /// the equality open (see [`OperatorClass`] reporting).
    Identifiable,
}

/// AXIS-2 — the **output guarantee** the protocol delivers against the chosen
/// adversary (research §1.2): does it abort on a cheat, or always deliver the
/// correct output? Orthogonal to AXIS-1 ([`AdversaryModel`]) and AXIS-3
/// ([`CorruptionThreshold`]).
///
/// ## Cleve's impossibility as a type-level invariant
/// Cleve (STOC'86): **fairness and guaranteed output are impossible without an
/// honest majority.** This type encodes that as a constructor invariant — the
/// [`OutputGuarantee::Fairness`] and [`OutputGuarantee::GuaranteedOutput`]
/// variants have a PRIVATE field, so they cannot be built directly; the ONLY way
/// to obtain them is via [`OutputGuarantee::fairness`] /
/// [`OutputGuarantee::guaranteed_output`], which take a [`CorruptionThreshold`]
/// and return `None` for [`CorruptionThreshold::DishonestMajority`]. A
/// dishonest-majority backend is therefore *unable to construct* an output
/// guarantee Cleve forbids — the impossibility is a compile-/construct-time
/// property, not a runtime check that can be forgotten. `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputGuarantee {
    /// **Abort (no guaranteed output).** On a detected cheat the protocol aborts
    /// (per [`AbortKind`]) rather than returning a wrong answer; a single cheater
    /// can still force the abort, so a correct output is NOT guaranteed. The ONLY
    /// guarantee available under a dishonest majority (Cleve).
    Abort(AbortKind),
    /// **Fairness.** Either all parties learn the output or none do — but, unlike
    /// [`OutputGuarantee::GuaranteedOutput`], a cheater may still deny everyone
    /// the output. Sits strictly between abort and GOD. Honest-majority-only
    /// (Cleve); construct via [`OutputGuarantee::fairness`]. The private unit
    /// field makes the variant unbuildable except through that constructor.
    Fairness(private::HonestMajorityProof),
    /// **Guaranteed output delivery (GOD).** The honest parties ALWAYS obtain the
    /// correct output, even when up to the threshold of parties actively deviate.
    /// Honest-majority-only (Cleve); construct via
    /// [`OutputGuarantee::guaranteed_output`]. The private unit field makes the
    /// variant unbuildable except through that constructor.
    GuaranteedOutput(private::HonestMajorityProof),
}

/// Private witness module: the unit token that gates construction of the
/// Cleve-restricted [`OutputGuarantee`] variants. Because the type is private to
/// this module, code OUTSIDE `backend` cannot mint a `Fairness`/`GuaranteedOutput`
/// directly — it must go through the honest-majority-checking constructors. This
/// is what makes Cleve a *type-level* invariant rather than a runtime assert.
/// `[OPUS-4.8]`
mod private {
    /// Zero-sized proof-of-honest-majority. Only the honest-majority constructors
    /// in [`super::OutputGuarantee`] can mint one, so its presence in a
    /// `Fairness`/`GuaranteedOutput` value is evidence the Cleve precondition was
    /// checked. `[OPUS-4.8]`
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HonestMajorityProof(pub(super) ());

    /// Zero-sized proof that the covert ε invariant (`den > 0`, `num <= den`) was
    /// range-checked. Only [`super::AdversaryModel::covert`] can mint one, so its
    /// presence in a `Covert` value is evidence ε is a valid probability in
    /// `(0, 1]`. `[OPUS-4.8]`
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CovertEpsilonProof(pub(super) ());
}

impl OutputGuarantee {
    /// Construct [`OutputGuarantee::Fairness`] IFF the threshold gives an honest
    /// majority (`HonestMajority` or `SuperHonestMajority`). Returns `None` under
    /// [`CorruptionThreshold::DishonestMajority`] — Cleve forbids fairness there.
    /// This is the ONLY way to obtain the `Fairness` variant. `[OPUS-4.8]`
    pub fn fairness(threshold: CorruptionThreshold) -> Option<Self> {
        threshold
            .is_honest_majority()
            .then_some(OutputGuarantee::Fairness(private::HonestMajorityProof(())))
    }

    /// Construct [`OutputGuarantee::GuaranteedOutput`] (GOD) IFF the threshold
    /// gives an honest majority. Returns `None` under
    /// [`CorruptionThreshold::DishonestMajority`] — Cleve forbids GOD there. This
    /// is the ONLY way to obtain the `GuaranteedOutput` variant. `[OPUS-4.8]`
    pub fn guaranteed_output(threshold: CorruptionThreshold) -> Option<Self> {
        threshold
            .is_honest_majority()
            .then_some(OutputGuarantee::GuaranteedOutput(
                private::HonestMajorityProof(()),
            ))
    }

    /// `true` iff this guarantee is some kind of abort (no guaranteed output).
    pub fn is_abort(self) -> bool {
        matches!(self, OutputGuarantee::Abort(_))
    }

    /// `true` iff this guarantee is fairness or stronger (guaranteed output) —
    /// i.e. it is constructible only under an honest majority. `[OPUS-4.8]`
    pub fn requires_honest_majority(self) -> bool {
        matches!(
            self,
            OutputGuarantee::Fairness(_) | OutputGuarantee::GuaranteedOutput(_)
        )
    }
}

/// AXIS-3 — the **corruption threshold**: how many of `n` parties may be corrupt,
/// carried as the concrete `t` so the same descriptor is expressible in BOTH
/// majority regimes (research §1.2). This is the axis the old binary
/// [`TrustModel`] collapsed; the three variants restore the `n/3`-vs-`n/2` split
/// that separates *perfect* GOD (BGW, point-to-point) from *statistical* GOD
/// (requires broadcast). `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptionThreshold {
    /// **Dishonest majority** (`t < n`): up to `n − 1` parties may be corrupt.
    /// The realistic cross-org model; only `Abort` output guarantees are
    /// reachable (Cleve). Carries the tolerated corruption count `t`.
    DishonestMajority {
        /// Number of corrupt parties tolerated (`t < n`).
        t: usize,
    },
    /// **Honest majority** (`n > 2t`): a strict majority of parties is honest.
    /// Admits statistical GOD (with a broadcast channel). Carries `t`.
    HonestMajority {
        /// Number of corrupt parties tolerated (`n > 2t`).
        t: usize,
    },
    /// **Super-honest majority** (`n > 3t`): more than two-thirds honest. Admits
    /// PERFECT, error-free GOD over point-to-point channels (BGW). Carries `t`.
    SuperHonestMajority {
        /// Number of corrupt parties tolerated (`n > 3t`).
        t: usize,
    },
}

impl CorruptionThreshold {
    /// The tolerated corruption count `t`, regardless of regime.
    pub fn threshold(self) -> usize {
        match self {
            CorruptionThreshold::DishonestMajority { t }
            | CorruptionThreshold::HonestMajority { t }
            | CorruptionThreshold::SuperHonestMajority { t } => t,
        }
    }

    /// `true` iff this threshold guarantees an honest majority (`HonestMajority`
    /// or `SuperHonestMajority`). The Cleve precondition for fairness/GOD.
    pub fn is_honest_majority(self) -> bool {
        !matches!(self, CorruptionThreshold::DishonestMajority { .. })
    }

    /// Derive the honest-majority threshold for a backend with `n` parties at the
    /// usual `t = ⌊(n−1)/2⌋` (the Shamir honest-majority constructor's choice),
    /// classifying it into the strongest applicable regime: `n > 3t` →
    /// super-honest, else `n > 2t` → honest (always true for `t = ⌊(n−1)/2⌋`,
    /// `n >= 2`). `[OPUS-4.8]`
    pub fn from_n_t(n: usize, t: usize) -> Self {
        if n > 3 * t {
            CorruptionThreshold::SuperHonestMajority { t }
        } else if n > 2 * t {
            CorruptionThreshold::HonestMajority { t }
        } else {
            CorruptionThreshold::DishonestMajority { t }
        }
    }
}

/// Marker for **public verifiability** (research §1.3): whether a cheat / the
/// computation is verifiable by an external party who did not run it — PVC's
/// publicly-verifiable cheating certificate, or a publicly-verifiable
/// collaborative-zk proof. Orthogonal to the three axes. `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PublicVerifiability(pub bool);

/// The full **three-axis security descriptor** for a backend (or a single
/// operator within it): adversary × output-guarantee × corruption-threshold,
/// plus the [`PublicVerifiability`] marker. This is the configurable security
/// descriptor that replaces the entangled [`TrustModel`] + [`MaliciousSecurity`]
/// pair; it composes them losslessly and projects BACK to the old enums for
/// back-compat (see [`SecurityDescriptor::malicious_security`] /
/// [`SecurityDescriptor::trust_model`]). `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityDescriptor {
    /// AXIS-1: how a corrupt party may behave.
    pub adversary: AdversaryModel,
    /// AXIS-2: what output guarantee holds against that adversary. Construction
    /// of the stronger variants is gated on `threshold` being honest-majority
    /// (Cleve), so any value here is internally consistent with `threshold`.
    pub output_guarantee: OutputGuarantee,
    /// AXIS-3: the corruption threshold (carrying `t`).
    pub threshold: CorruptionThreshold,
    /// Whether a cheat / the computation is publicly verifiable.
    pub public_verifiability: PublicVerifiability,
}

impl SecurityDescriptor {
    /// The v1 semi-honest, detect-and-abort-on-redundancy honest-majority
    /// descriptor for a `(n, t)` Shamir backend whose degree-`t` reconstruction
    /// carries RS redundancy. `robust_cheaters` is the RS correction budget
    /// `e = ⌊(n−t−1)/2⌋` (`0` ⇒ detect-and-abort only). Helper used by
    /// [`crate::shamir::ShamirBackend`] to build its per-operator descriptors
    /// without duplicating the Cleve plumbing. `[OPUS-4.8]`
    pub fn shamir_degree_recon(n: usize, t: usize, robust_cheaters: usize) -> Self {
        let threshold = CorruptionThreshold::from_n_t(n, t);
        // FAIL-CLOSED under a dishonest majority (`n <= 2t`). The RS-redundancy
        // detect-and-abort / Berlekamp–Welch correction this constructor encodes
        // is an *honest-majority-specific* claim: the bound `e = ⌊(n−t−1)/2⌋` and
        // the "any tampering is detected" guarantee both assume a strict honest
        // majority on the reconstruction. With `n <= 2t` there is not enough
        // honest redundancy to back that claim, so emitting an `Abort(Unanimous)`
        // here would over-claim — and would project (via `malicious_security`) to
        // `HonestMajorityAbort` while `trust_model()` reports `DishonestMajority`,
        // an internally contradictory `BackendInfo`. The legacy `MaliciousSecurity`
        // enum cannot represent dishonest-majority active security without
        // over-claiming, so this honest-majority constructor degrades to the
        // semi-honest-only baseline there. A genuine dishonest-majority active
        // backend (SPDZ/MASCOT: MAC-checked abort) is a *different* construction
        // and would build its descriptor directly, not via this helper. `[OPUS-4.8]`
        if !threshold.is_honest_majority() {
            return SecurityDescriptor::semi_honest_only(n, t);
        }
        // The honest-majority Shamir backend is semi-honest among cooperating
        // holders; the RS-checked reconstruction adds a detect/correct step. We
        // surface it as semi-honest adversary with the *output guarantee* the RS
        // redundancy delivers (the active-security hardening is the guarantee
        // axis, not a claim that the parties are malicious).
        let output_guarantee = if robust_cheaters >= 1 {
            // Robust correction up to a budget ⇒ guaranteed output (GOD) under
            // honest majority. Cleve-gated constructor; honest majority holds for
            // every valid Shamir `(n, t = ⌊(n−1)/2⌋)`, so this is `Some`.
            OutputGuarantee::guaranteed_output(threshold)
                .unwrap_or(OutputGuarantee::Abort(AbortKind::Unanimous))
        } else {
            // Redundancy lets us detect-and-abort (unanimous), but not correct.
            OutputGuarantee::Abort(AbortKind::Unanimous)
        };
        SecurityDescriptor {
            adversary: AdversaryModel::SemiHonest,
            output_guarantee,
            threshold,
            public_verifiability: PublicVerifiability(false),
        }
    }

    /// A no-redundancy descriptor: semi-honest-only, with no detection. Used for
    /// the degree-`2t` equality open at `n = 2t+1` (zero RS redundancy at degree
    /// `2t`) and for honest stub backends. `[OPUS-4.8]`
    pub fn semi_honest_only(n: usize, t: usize) -> Self {
        SecurityDescriptor {
            adversary: AdversaryModel::SemiHonest,
            output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
            threshold: CorruptionThreshold::from_n_t(n, t),
            public_verifiability: PublicVerifiability(false),
        }
    }

    /// **Back-compat PROJECTION → [`TrustModel`].** Collapses AXIS-3 to the old
    /// binary trust model so existing callers of `BackendInfo::trust_model`
    /// keep working: honest-/super-honest-majority project to
    /// [`TrustModel::HonestMajority`], dishonest-majority to
    /// [`TrustModel::DishonestMajority`]. `[OPUS-4.8]`
    pub fn trust_model(&self) -> TrustModel {
        if self.threshold.is_honest_majority() {
            TrustModel::HonestMajority
        } else {
            TrustModel::DishonestMajority
        }
    }

    /// **Back-compat PROJECTION → [`MaliciousSecurity`].** Maps the three-axis
    /// descriptor onto the old bundled enum so `ShamirBackend::info()` and every
    /// existing `BackendInfo` caller still type-checks against the same
    /// [`MaliciousSecurity`] values:
    ///
    /// - no active-security guarantee (semi-honest adversary AND a no-detection
    ///   selective abort) → [`MaliciousSecurity::SemiHonestOnly`];
    /// - guaranteed output / robust correction up to a budget →
    ///   [`MaliciousSecurity::HonestMajorityRobust`] (the budget is recovered
    ///   from `robust_cheaters`, threaded through this projection);
    /// - any other detect-and-abort → [`MaliciousSecurity::HonestMajorityAbort`].
    ///
    /// `robust_cheaters` is the RS correction budget the *operator* delivers
    /// (`>= 1` for the robust case); it cannot be recovered from the abstract
    /// `OutputGuarantee` alone (which only says "GOD", not "GOD up to e"), so it
    /// is supplied by the backend that built the descriptor. This keeps the
    /// projection faithful to the old enum's `max_cheaters` field. `[OPUS-4.8]`
    pub fn malicious_security(&self, robust_cheaters: usize) -> MaliciousSecurity {
        match self.output_guarantee {
            // Selective abort under a semi-honest adversary = the honest
            // "we claim nothing" baseline (no redundancy / stub).
            OutputGuarantee::Abort(AbortKind::Selective)
                if self.adversary == AdversaryModel::SemiHonest =>
            {
                MaliciousSecurity::SemiHonestOnly
            }
            OutputGuarantee::GuaranteedOutput(_) if robust_cheaters >= 1 => {
                MaliciousSecurity::HonestMajorityRobust {
                    max_cheaters: robust_cheaters,
                }
            }
            // Everything else that detects (unanimous/identifiable abort, or a
            // GOD claim with a zero correction budget — which is really
            // detect-and-abort) is the abort guarantee.
            _ => MaliciousSecurity::HonestMajorityAbort,
        }
    }
}

/// The class of SPARQL/MPC operator a [`SecurityDescriptor`] is reported FOR.
/// Guarantees differ PER-OPERATOR for the same backend `(n, t)` — the degree-`t`
/// linear aggregate carries RS redundancy (robust) while the degree-`2t`
/// equality/join open has NO redundancy at `n = 2t+1` (semi-honest-only) — so a
/// single backend-level bit would lie (research §1.2). A backend reports its
/// guarantee keyed by this class via [`MpcBackend::operator_security`].
/// `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorClass {
    /// Linear / cumulative aggregate (SUM/COUNT): free local share-addition,
    /// reconstructed at degree `t`. Carries RS redundancy for every valid
    /// honest-majority `(n, t)`, so it is the robust path.
    LinearAggregate,
    /// Equality / hidden-value join: opens a degree-`2t` product. At `n = 2t+1`
    /// (the honest-majority odd-`n` case) degree `2t` has ZERO RS redundancy, so
    /// this open is semi-honest-only there even though the aggregate is robust.
    EqualityJoin,
    /// Comparison (`<`, `≤`, `>`): bit-decomposition; not yet realized in the
    /// crate (disclosed operands are recomputed by the verifier outside the
    /// crypto). Reported for completeness so a federation sees the gap.
    Comparison,
}

/// The trust regime an [`MpcBackend`] implementation provides. This is the
/// Q2 axis made explicit in the type system so a federation can refuse a
/// backend whose guarantees do not match its threat model (§4.2).
///
/// **Back-compat note (sq-mq8q).** This binary enum is now a *projection* of the
/// richer [`CorruptionThreshold`] axis (which carries `t` and distinguishes
/// honest- from super-honest-majority). It is retained so existing
/// `BackendInfo::trust_model` callers keep compiling; new code should prefer
/// `BackendInfo::security.threshold`. See [`SecurityDescriptor::trust_model`] for
/// the projection. `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustModel {
    /// Security holds only if a strict majority of compute parties are honest.
    /// The viable first target (M3). Sub-variants (semi-honest vs malicious)
    /// are an impl detail surfaced via [`BackendInfo`].
    HonestMajority,
    /// Security holds against up to n-1 corrupt parties. The realistic
    /// cross-org model; deferred pending the Q2 decision and far heavier.
    DishonestMajority,
}

/// The malicious-security guarantee a backend delivers against *actively*
/// deviating parties — guarantee (D) of §4.2, orthogonal to [`TrustModel`]'s
/// majority axis. A `bool` is too coarse: WI-1/WI-2 gave the honest-majority
/// Shamir backend two *distinct* active-security levels depending on how much
/// reconstruction redundancy a given `(n, t)` configuration carries, and a
/// federation inspecting [`BackendInfo`] needs to tell them apart. The variants
/// are ordered from weakest to strongest. `[OPUS-4.8]`
///
/// **Back-compat note (sq-mq8q).** This enum bundled AXIS-1 (semi-honest vs
/// active) with AXIS-2 (abort vs robust) and soldered "HonestMajority" into the
/// variant names. It is now a *projection* of the orthogonal three-axis
/// [`SecurityDescriptor`] (see [`SecurityDescriptor::malicious_security`]),
/// retained so `ShamirBackend::info()` and every existing caller keep compiling.
/// New code should prefer `BackendInfo::security` / [`MpcBackend::operator_security`].
/// `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaliciousSecurity {
    /// **No active-security guarantee — semi-honest only.** Security holds only
    /// against semi-honest / honest-but-curious parties; an actively-deviating
    /// party can silently corrupt the output with no detection. The honest
    /// baseline for a not-yet-built / stub backend, and the guarantee for any
    /// reconstruction at exactly `degree + 1` shares (no RS redundancy — e.g. the
    /// degree-`2t` equality open at `n = 2t + 1`). NB: with the honest-majority
    /// `t = ⌊(n−1)/2⌋` the degree-`t` cumulative-aggregate path always has
    /// redundancy, so it never reports `SemiHonestOnly`. Named `SemiHonestOnly`
    /// (not `None`) so it cannot be confused with [`Option::None`] nor collide
    /// under a `use MaliciousSecurity::*` glob import. `[OPUS-4.8]`
    SemiHonestOnly,
    /// **Detect-and-abort (no guaranteed output).** Any tampering by an
    /// actively-deviating party is *detected* and the protocol aborts with a
    /// typed error rather than returning a wrong answer — but a single cheater
    /// can still force an abort, so a correct output is NOT guaranteed. This is
    /// the guarantee when there is at least one redundant share to cross-check
    /// (`n > t + 1`) but not enough to *correct* a cheater (`max_cheaters == 0`).
    HonestMajorityAbort,
    /// **Robust: guaranteed-correct output up to a cheater budget.** The protocol
    /// returns the *true* output even when up to `max_cheaters` parties actively
    /// deviate (Reed–Solomon / Berlekamp–Welch correction), and detect-and-aborts
    /// beyond that budget. `max_cheaters >= 1` always (a `0` budget is
    /// [`MaliciousSecurity::HonestMajorityAbort`], not this variant).
    HonestMajorityRobust {
        /// The maximum number of actively-cheating parties tolerated while still
        /// producing the guaranteed-correct output. For the honest-majority
        /// Shamir backend at degree `t` this is `e = ⌊(n − t − 1)/2⌋` (the RS
        /// correction bound); always `>= 1` for this variant.
        max_cheaters: usize,
    },
}

impl MaliciousSecurity {
    /// `true` iff the backend provides *some* active-security guarantee (detects
    /// or corrects an actively-deviating party), i.e. anything other than
    /// [`MaliciousSecurity::SemiHonestOnly`]. The backwards-compatible projection
    /// of the former `malicious_secure: bool` field, for callers that only need
    /// the coarse "is this hardened at all?" bit.
    pub fn is_malicious_secure(self) -> bool {
        self != MaliciousSecurity::SemiHonestOnly
    }
}

/// Declared offline preprocessing cost, independent of measured online communication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreprocessingCost {
    /// No offline preprocessing is required; both estimates are explicitly zero.
    None,
    /// A linear item budget with an optional per-party byte cost per item.
    PerOperation {
        /// Items consumed per modeled multiplication.
        items_per_multiplication: u64,
        /// Additional items consumed per modeled ordering comparison.
        items_per_comparison: u64,
        /// Declared bytes per item per party; `None` means unmodeled, never zero.
        bytes_per_item_per_party: Option<u64>,
    },
}

impl PreprocessingCost {
    /// Estimate offline items; return `None` if the declared budget overflows.
    pub fn estimate_items(self, multiplications: u64, comparisons: u64) -> Option<u64> {
        match self {
            Self::None => Some(0),
            Self::PerOperation {
                items_per_multiplication,
                items_per_comparison,
                ..
            } => multiplications
                .checked_mul(items_per_multiplication)?
                .checked_add(comparisons.checked_mul(items_per_comparison)?),
        }
    }

    /// Estimate offline bytes per party; preserve unknown costs and overflow as `None`.
    pub fn estimate_bytes_per_party(self, multiplications: u64, comparisons: u64) -> Option<u64> {
        match self {
            Self::None => Some(0),
            Self::PerOperation {
                bytes_per_item_per_party,
                ..
            } => self
                .estimate_items(multiplications, comparisons)?
                .checked_mul(bytes_per_item_per_party?),
        }
    }
}

/// Static description of a concrete backend's guarantees — what a federation
/// inspects to decide whether a backend is acceptable BEFORE running anything.
///
/// **sq-mq8q.** The source of truth is now the three-axis [`security`] descriptor
/// (adversary × output-guarantee × corruption-threshold + public verifiability).
/// The `trust_model` and `malicious_security` fields are KEPT as a back-compat
/// projection of it (every existing caller of those two fields keeps compiling),
/// but a `BackendInfo` is best built via [`BackendInfo::new`] so the projection
/// stays internally consistent, and new code should read `security` /
/// [`MpcBackend::operator_security`]. `[OPUS-4.8]`
///
/// [`security`]: BackendInfo::security
#[derive(Debug, Clone)]
pub struct BackendInfo {
    /// Human-readable backend name (e.g. an eventual `"replicated-3pc"`).
    pub name: &'static str,
    /// The backend-level three-axis security descriptor (sq-mq8q). It describes
    /// the backend's PRIMARY reconstruction path (for the Shamir backend, the
    /// degree-`t` linear-aggregate open). It is NOT a single bit that can
    /// summarise every operator: a backend whose guarantees differ per operator
    /// (the Shamir degree-`2t` equality open is semi-honest-only at `n = 2t+1`
    /// even though the degree-`t` aggregate is robust) reports the per-operator
    /// detail via [`MpcBackend::operator_security`] — query that, do not assume
    /// this field covers every operator. `[OPUS-4.8]`
    pub security: SecurityDescriptor,
    /// The trust regime (the Q2 axis). **Back-compat projection** of
    /// `security.threshold` — see [`SecurityDescriptor::trust_model`].
    pub trust_model: TrustModel,
    /// The malicious-security guarantee against actively-deviating parties
    /// (§4.2's guarantee (D)), orthogonal to `trust_model`. **Back-compat
    /// projection** of `security` — see [`SecurityDescriptor::malicious_security`].
    /// Replaces the former coarse `malicious_secure: bool` so the *kind* of active
    /// security (detect-and-abort vs robust-up-to-`max_cheaters`) is legible to a
    /// federation. Use [`BackendInfo::malicious_secure`] for the coarse bool.
    /// `[OPUS-4.8]`
    pub malicious_security: MaliciousSecurity,
    /// Declared offline cost; this is a budget model, not a measurement.
    pub requires_preprocessing: PreprocessingCost,
}

impl BackendInfo {
    /// Build a `BackendInfo` from the three-axis [`SecurityDescriptor`], deriving
    /// the back-compat `trust_model` / `malicious_security` projection fields so
    /// they can never drift from `security`. `robust_cheaters` is the RS
    /// correction budget the backend's reconstruction delivers (`0` ⇒
    /// detect-and-abort only); it is threaded into the
    /// [`MaliciousSecurity::HonestMajorityRobust`] projection so the old enum's
    /// `max_cheaters` stays faithful. `[OPUS-4.8]`
    /// Defaults to no preprocessing; backends needing it must set `requires_preprocessing`.
    pub fn new(name: &'static str, security: SecurityDescriptor, robust_cheaters: usize) -> Self {
        BackendInfo {
            name,
            trust_model: security.trust_model(),
            malicious_security: security.malicious_security(robust_cheaters),
            requires_preprocessing: PreprocessingCost::None,
            security,
        }
    }

    /// Backwards-compatible coarse bool: `true` iff this backend provides *any*
    /// active-security guarantee (i.e. `malicious_security != None`). Preserves
    /// the question the old `malicious_secure: bool` field answered for callers
    /// that don't need the finer variant.
    pub fn malicious_secure(&self) -> bool {
        self.malicious_security.is_malicious_secure()
    }
}

/// Abstracts the secret-sharing / MPC primitive over which the federated SPARQL
/// operator pipeline runs (architecture §4.3 step 4).
///
/// The interface is deliberately minimal and primitive-agnostic at M0: it
/// captures the obligations every secret-sharing MPC must meet — share private
/// inputs, run the secure computation, reconstruct only the disclosed output —
/// WITHOUT naming a scheme. Associated type [`Self::Share`] stands in for the
/// scheme's share representation so honest- and dishonest-majority impls can
/// carry entirely different share types behind the same trait.
///
/// ## Q2 DECISION (resolved for v1)
/// Honest- vs dishonest-majority (and, on a WAN, secret-sharing vs garbled-
/// circuit) reshapes `Share`, the round structure, and the preprocessing model.
/// **v1 resolves Q2 to honest-majority** (Jesse's decision: honest-majority now,
/// configurable long-term) — the four *cooperating* flatmates prove an aggregate
/// to an external landlord; among themselves they are honest-but-curious, the
/// regime Shamir serves. Dishonest-majority remains future research behind this
/// same trait (see the module-level "how the trust model stays CONFIGURABLE").
///
/// ## Implementation status
/// First concrete implementor: [`crate::shamir::ShamirBackend`] (M3, honest-
/// majority Shamir `t`-of-`n`, semi-honest). It implements all three methods
/// below for real (over an in-process multi-party simulation). A dishonest-
/// majority impl slots in behind the same trait unchanged.
pub trait MpcBackend {
    /// The scheme-specific representation of a secret share. Opaque to the rest
    /// of the crate; only this trait's impl manipulates it.
    type Share;

    /// Static guarantees of this backend (the three-axis [`SecurityDescriptor`]
    /// plus its back-compat `trust_model` / `malicious_security` projection). The
    /// `security` field describes the backend's PRIMARY reconstruction path; use
    /// [`MpcBackend::operator_security`] for the per-operator detail where they
    /// differ.
    fn info(&self) -> BackendInfo;

    /// Per-operator security guarantee (sq-mq8q). Guarantees differ by
    /// [`OperatorClass`] for the same backend `(n, t)` — e.g. the Shamir
    /// degree-`t` linear aggregate is robust while the degree-`2t` equality open
    /// has no RS redundancy at `n = 2t+1` — so a single backend-level bit would
    /// lie. The default returns the backend-level `info().security`; backends
    /// whose operators genuinely differ (the Shamir backend) override this to
    /// report each class precisely. `[OPUS-4.8]`
    fn operator_security(&self, _operator: OperatorClass) -> SecurityDescriptor {
        self.info().security
    }

    /// Secret-share a holder's *private* contribution to the computation —
    /// the per-source intermediate values that must NEVER leave the source in
    /// the clear (architecture §4.3 step 4: "intermediate per-source values
    /// never leave a source"). Contrast [`Holder::evaluate_local`], which
    /// discloses; this path hides.
    ///
    /// Implemented by [`crate::shamir::ShamirBackend`] (M3): extracts the
    /// holder's single private integer and Shamir-shares it across the parties.
    fn share_private_input(&self, holder: &Holder) -> Result<Vec<Self::Share>, MpcError>;

    /// [OPUS-4.8] sq-dwb5 — **batched / vector** secret-sharing of a holder's
    /// private contribution: the holder surfaces a `Vec` of private integers (a
    /// per-row hidden-value / salary vector / per-graph commitment column) via a
    /// caller-supplied fragment, each row is Shamir-shared, and the result is
    /// *positionally* row-bound — output index `i` is the sharing of the holder's
    /// `i`-th private row. This generalises [`Self::share_private_input`] (which is
    /// the `k = 1` special case) to MORE than one secret value per source, so the
    /// secure aggregate / hidden-value join can range over many rows per holder.
    ///
    /// **Row-binding contract.** The binding is POSITIONAL: every holder evaluates
    /// the SAME `fragment` projecting one integer column, ordered DETERMINISTICALLY.
    /// Nothing about the column is disclosed — the values ARE the secret inputs. The
    /// implementation imposes the deterministic order by **sorting the holder's rows
    /// by their (secret) value LOCALLY, before any sharing**, so two holders' batches
    /// line up by position regardless of local row order; the values themselves never
    /// leave the holder — only their shares do (see
    /// [`crate::shamir::ShamirBackend::share_private_inputs`]). Downstream (per-row
    /// secure aggregate, hidden join) correlate across holders by output index. For a
    /// KEYED binding (element `i` ↔ a row id that IS disclosed), the caller pairs the
    /// returned batch with the holder's disclosed key column out-of-band and uses that
    /// public key — not the secret value — to define the cross-holder order.
    ///
    /// Returns one trait-`Share` PER ROW (so a single-scalar contribution is the
    /// `k = 1` case). Each `Self::Share` is the per-party sharing of one private
    /// value; index `i` is row `i` under the row-binding above.
    ///
    /// Default impl: the single-scalar fallback so existing backends/stubs keep
    /// compiling — it returns a length-1 batch via [`Self::share_private_input`]
    /// (the `share_private_input` result is itself one trait-`Share`).
    /// [`crate::shamir::ShamirBackend`] overrides it with the real vector path.
    fn share_private_inputs(
        &self,
        holder: &Holder,
        _fragment: &str,
    ) -> Result<Vec<Self::Share>, MpcError> {
        self.share_private_input(holder)
    }

    /// Run the secure computation over secret-shared inputs from all holders
    /// (e.g. the cumulative-salary aggregate whose per-source addends stay
    /// private). Returns the shares of the result.
    ///
    /// Implemented by [`crate::shamir::ShamirBackend`] (M3) for the cumulative
    /// sum — a pure linear function, so it is the zero-round local addition of
    /// the sharings (the honest-majority Shamir sweet spot).
    fn run_secure(&self, shares: &[Self::Share]) -> Result<Vec<Self::Share>, MpcError>;

    /// Reconstruct ONLY the disclosed output from result shares (the minimal
    /// answer — e.g. the cumulative integer, from which the verifier recomputes
    /// `cumulative > £100k` OUTSIDE the crypto), under the
    /// no-proof-of-revealed-properties discipline (§2 convention #4).
    ///
    /// Implemented by [`crate::shamir::ShamirBackend`] (M3) via Lagrange
    /// interpolation of the result sharing at `x = 0`.
    fn reconstruct_disclosed(
        &self,
        result_shares: &[Self::Share],
    ) -> Result<PartialResult, MpcError>;
}

// =============================================================================
// [OPUS-4.8] sq-a6p1 — SECURITY-MODEL SELECTION / NEGOTIATION API + a FAIL-CLOSED
// backend registry (Fable unavailable; re-review when Fable returns).
//
// Background: sq-mq8q (above) gave every backend a three-axis
// [`SecurityDescriptor`] and per-operator reporting, but a caller could only
// inspect a chosen backend's [`BackendInfo`] *post-hoc*. There was no way for a
// federation to state its security requirement UP FRONT and have a backend
// MATCHED against it — and, crucially, no way for an over-strong request (e.g.
// dishonest-majority-malicious over the SPARQL pipeline, which NO shipped backend
// can honour — research §1.3 honesty-anchor (e)) to be *truthfully refused*
// instead of silently downgraded onto the one semi-honest Shamir backend.
//
// This module adds exactly that, the "configurable long-term" deliverable from
// research §1.3(b): a [`SecurityRequirement`] (the federation's stated floor on
// each axis + the attribution / public-verifiability flags), its
// [`SecurityRequirement::satisfies`] predicate against a [`BackendInfo`], and a
// [`BackendRegistry`] whose [`BackendRegistry::select`] returns the STRONGEST
// registered backend meeting the requirement or, FAILING CLOSED, the typed
// [`MpcError::NoBackendSatisfies`] — mirroring the crate's
// [`MpcError::NotYetImplemented`] honesty discipline (no fake crypto; here, no
// fake security level). It composes with the existing descriptor + per-operator
// reporting: [`BackendRegistry::select_for_operator`] matches against
// [`MpcBackend::operator_security`] so the degree-`2t` equality open is judged on
// ITS guarantee, not the backend's headline degree-`t` aggregate bit.
//
// Scope (sq-a6p1): stays in `backend.rs` + the one new `MpcError` variant. The
// registry is GENERIC over a concrete backend type `B: MpcBackend` rather than
// `&dyn MpcBackend` (as the research sketch wrote it) because [`MpcBackend`] has
// an associated `Share` type and is therefore NOT object-safe — a `dyn` form
// would force every backend in one registry to share one `Share`, which is
// exactly the cross-scheme heterogeneity a federation does NOT have at v1 (it
// runs ONE scheme family). Generic-over-`B` is the honest, compiling shape.
// =============================================================================

/// Rank of an [`AdversaryModel`] on its weakest→strongest axis, for the
/// "satisfies a *minimum*" comparison. A backend secure against a STRONGER
/// adversary also satisfies a requirement for a weaker one (a malicious-secure
/// backend trivially meets a semi-honest floor). Covert sits strictly between
/// semi-honest and malicious regardless of its ε (the ε refines *how strong*
/// covert is, but every covert tier still ranks below full malicious security).
/// `[OPUS-4.8]`
fn adversary_rank(a: AdversaryModel) -> u8 {
    match a {
        AdversaryModel::SemiHonest => 0,
        AdversaryModel::Covert { .. } => 1,
        AdversaryModel::Malicious => 2,
    }
}

/// Rank of an [`AbortKind`] on its weakest→strongest axis (selective < unanimous
/// < identifiable). `[OPUS-4.8]`
fn abort_rank(k: AbortKind) -> u8 {
    match k {
        AbortKind::Selective => 0,
        AbortKind::Unanimous => 1,
        AbortKind::Identifiable => 2,
    }
}

/// Total rank of an [`OutputGuarantee`] on its weakest→strongest axis, flattening
/// the abort sub-tiers below fairness below guaranteed-output:
/// `Abort(Selective) < Abort(Unanimous) < Abort(Identifiable) < Fairness < GuaranteedOutput`.
/// Used so a requirement's `min_output_guarantee` is met by any backend guarantee
/// that ranks at least as high. `[OPUS-4.8]`
fn output_guarantee_rank(g: OutputGuarantee) -> u8 {
    match g {
        OutputGuarantee::Abort(k) => abort_rank(k), // 0..=2
        OutputGuarantee::Fairness(_) => 3,
        OutputGuarantee::GuaranteedOutput(_) => 4,
    }
}

/// Rank of a [`CorruptionThreshold`] by **how adverse a corruption setting the
/// regime survives** — the more corruption tolerated, the stronger the backend.
/// Dishonest-majority (up to `n−1` corrupt) is the strongest regime to operate
/// under, super-honest-majority (needs `> 2/3` honest) the weakest:
/// `SuperHonestMajority < HonestMajority < DishonestMajority`.
///
/// This ranks the *regime*, deliberately ignoring the concrete `t`: a requirement
/// is "the backend must remain secure under AT LEAST this adverse a corruption
/// setting", and the regime is what determines reachable guarantees (Cleve). A
/// backend that works under a dishonest majority therefore satisfies a
/// requirement that only needs an honest majority. `[OPUS-4.8]`
fn threshold_rank(c: CorruptionThreshold) -> u8 {
    match c {
        CorruptionThreshold::SuperHonestMajority { .. } => 0,
        CorruptionThreshold::HonestMajority { .. } => 1,
        CorruptionThreshold::DishonestMajority { .. } => 2,
    }
}

/// `true` iff `desc` advertises a SOUND cheater-attribution guarantee — i.e. its
/// output guarantee is an [`AbortKind::Identifiable`] abort (identifiable abort:
/// the honest parties AGREE on a cheater). A robust GOD backend corrects cheaters
/// rather than attributing them, and a heuristic `Tampered{cheaters}` set is
/// explicitly NOT sound IA (see [`AbortKind`] docs), so neither is treated as
/// attribution here. `[OPUS-4.8]`
fn provides_cheater_attribution(desc: &SecurityDescriptor) -> bool {
    matches!(
        desc.output_guarantee,
        OutputGuarantee::Abort(AbortKind::Identifiable)
    )
}

/// FAIL-CLOSED adversary-axis check: does a backend whose adversary model is
/// `desc` meet a requirement floor of `req`? Strength is the coarse
/// weakest→strongest rank ([`adversary_rank`]) AND — when BOTH are covert —
/// the backend's deterrence ε must be at least the required ε.
///
/// The coarse rank alone is unsound for the covert tier: every `Covert{..}` ranks
/// 1 regardless of its ε, so a requirement of covert(ε=1/2) would otherwise be
/// satisfied by a much weaker covert(ε=1/100) backend — silently accepting a
/// lower deterrence than asked for (Copilot review #92). When both sides are
/// covert we additionally require `desc_ε >= req_ε`, compared as exact rationals
/// via cross-multiplication (`desc_num·req_den >= req_num·desc_den`, both
/// denominators `> 0` by the [`AdversaryModel::covert`] invariant) so no float
/// enters a security decision. A strictly stronger adversary model (malicious)
/// outranks any covert floor and needs no ε comparison; a covert backend never
/// satisfies a malicious floor (rank 1 < 2). The products are widened to `u64`
/// so the `u32 × u32` cross-multiply cannot overflow. `[OPUS-4.8]`
fn adversary_satisfies(desc: AdversaryModel, req: AdversaryModel) -> bool {
    if adversary_rank(desc) > adversary_rank(req) {
        // Strictly stronger adversary model dominates (e.g. malicious >= covert).
        return true;
    }
    if adversary_rank(desc) < adversary_rank(req) {
        return false;
    }
    // Equal rank. For the covert tier the rank is ε-blind, so compare ε exactly.
    match (desc.deterrence(), req.deterrence()) {
        (Some((d_num, d_den)), Some((r_num, r_den))) => {
            // desc_ε >= req_ε  ⇔  d_num/d_den >= r_num/r_den
            //                  ⇔  d_num·r_den >= r_num·d_den   (denominators > 0)
            u64::from(d_num) * u64::from(r_den) >= u64::from(r_num) * u64::from(d_den)
        }
        // Same non-covert rank (both semi-honest or both malicious): rank suffices.
        _ => true,
    }
}

/// FAIL-CLOSED corruption-axis check: does a backend at corruption threshold
/// `desc` meet a requirement of `req`? The backend must survive AT LEAST as
/// adverse a *regime* ([`threshold_rank`]) AND tolerate AT LEAST as many corrupt
/// parties (`desc.t >= req.t`).
///
/// The coarse regime rank alone is unsound: it deliberately ignores the concrete
/// `t`, so a requirement of `HonestMajority{t=3}` (stay secure with up to 3
/// corrupt parties) would otherwise be satisfied by a backend at
/// `HonestMajority{t=1}` that a *second* corrupt party already breaks — silently
/// accepting a weaker concrete threshold than asked for (Copilot review #92).
/// Requiring `desc.t >= req.t` closes that: a backend must tolerate at least the
/// requested corruption count in addition to surviving at least the requested
/// regime. (A higher-rank regime with a lower `t` is therefore NOT accepted for a
/// higher-`t` floor — correctly, since it tolerates fewer corruptions.)
/// `[OPUS-4.8]`
fn corruption_satisfies(desc: CorruptionThreshold, req: CorruptionThreshold) -> bool {
    threshold_rank(desc) >= threshold_rank(req) && desc.threshold() >= req.threshold()
}

/// Inclusive offline preprocessing ceilings for a caller-declared workload.
///
/// Estimates use [`BackendInfo::requires_preprocessing`], not measured spending.
/// A set ceiling rejects unknown estimates (including arithmetic overflow).
/// An unset ceiling imposes no restriction on that quantity; both unset impose
/// no cost restriction. Callers must update the workload when the plan changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreprocessingBudget {
    /// Modeled multiplications, including those used by comparisons.
    pub multiplications: u64,
    /// Modeled ordering comparisons; their item costs are additional.
    pub comparisons: u64,
    /// Maximum offline items, or `None` for no item ceiling.
    pub max_items: Option<u64>,
    /// Maximum offline bytes per compute party, or `None` for no byte ceiling.
    pub max_bytes_per_party: Option<u64>,
}

impl PreprocessingBudget {
    fn refusal(&self, cost: PreprocessingCost) -> Option<String> {
        let items = cost.estimate_items(self.multiplications, self.comparisons);
        let bytes = cost.estimate_bytes_per_party(self.multiplications, self.comparisons);
        let items_fit = self
            .max_items
            .is_none_or(|max| items.is_some_and(|n| n <= max));
        let bytes_fit = self
            .max_bytes_per_party
            .is_none_or(|max| bytes.is_some_and(|n| n <= max));
        if items_fit && bytes_fit {
            None
        } else {
            Some(format!(
                "items={items:?}, bytes-per-party={bytes:?} (None means unknown or overflow)"
            ))
        }
    }
}

/// A federation's **security requirement**, stated UP FRONT, that a backend must
/// meet before it is allowed to run any part of the SPARQL pipeline (research
/// §1.3(b)). The security fields are *floors* on the three-axis
/// [`SecurityDescriptor`]; [`SecurityRequirement::satisfies`] checks a concrete
/// [`BackendInfo`] against all of them and the optional preprocessing ceiling.
///
/// The whole point is **fail-closed negotiation**: a federation can REQUEST any
/// requirement — including one no shipped backend can honour (dishonest-majority
/// malicious over the SPARQL operator pipeline, which has zero published
/// instances — research §1.3(e)) — and [`BackendRegistry::select`] will
/// truthfully refuse it with [`MpcError::NoBackendSatisfies`] rather than
/// silently serve a weaker backend. `[OPUS-4.8]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityRequirement {
    /// Minimum adversary model the backend must withstand (AXIS-1). A backend
    /// whose [`AdversaryModel`] ranks at least this high satisfies it (malicious
    /// ≥ covert ≥ semi-honest).
    pub min_adversary: AdversaryModel,
    /// Minimum output guarantee the backend must deliver (AXIS-2), ordered
    /// `Abort(Selective) < Abort(Unanimous) < Abort(Identifiable) < Fairness <
    /// GuaranteedOutput`. Construct the honest-majority-only variants via
    /// [`OutputGuarantee::fairness`] / [`OutputGuarantee::guaranteed_output`].
    pub min_output_guarantee: OutputGuarantee,
    /// The MOST adverse corruption setting the backend must remain secure under
    /// (AXIS-3). A backend whose regime tolerates at least this much corruption
    /// satisfies it (`DishonestMajority` ≥ `HonestMajority` ≥
    /// `SuperHonestMajority`). The concrete `t` is informational here — the
    /// *regime* is what gates reachable guarantees (Cleve). Named `max_corruption`
    /// to read as "I need security up to this much corruption".
    pub max_corruption: CorruptionThreshold,
    /// Require SOUND cheater attribution (identifiable abort), not the heuristic
    /// best-effort `Tampered{cheaters}` set. Only an
    /// [`OutputGuarantee::Abort`]`(`[`AbortKind::Identifiable`]`)` descriptor
    /// satisfies this.
    pub require_cheater_attribution: bool,
    /// Require [`PublicVerifiability`] — a cheat / the computation must be
    /// verifiable by an external party who did not run it (PVC certificate or a
    /// publicly-verifiable collaborative-zk proof).
    pub require_public_verifiability: bool,
    /// Optional offline cost ceilings for a stated workload. `None` preserves
    /// security-only selection; constructors default to `None`.
    pub max_preprocessing: Option<PreprocessingBudget>,
}

impl SecurityRequirement {
    /// The v1 honest-majority, semi-honest floor — the requirement the shipped
    /// Shamir backend (cooperating holders, LAN) is built to meet: a semi-honest
    /// adversary, a detect-and-abort-or-better output guarantee on the redundant
    /// reconstruction (so `Abort(Unanimous)` as the floor), honest-majority
    /// corruption, and NO attribution / public-verifiability demand. A federation
    /// that accepts the v1 envelope can `select` with this and get the Shamir
    /// backend; tightening any field is how it asks for more. `[OPUS-4.8]`
    pub fn v1_honest_majority_semi_honest() -> Self {
        SecurityRequirement {
            min_adversary: AdversaryModel::SemiHonest,
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Unanimous),
            max_corruption: CorruptionThreshold::HonestMajority { t: 1 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        }
    }

    /// The hardest realistic ask: **dishonest-majority, malicious**, identifiable
    /// abort. This is the requirement the research honesty-anchor (§1.3(e)) says
    /// has ZERO published instances for SPARQL/graph query evaluation — a
    /// federation MAY state it, and [`BackendRegistry::select`] MUST refuse it
    /// fail-closed for the shipped backend set. Provided as a named constructor so
    /// the "this is the request we truthfully refuse" intent is explicit at call
    /// sites and in tests. (GOD is Cleve-impossible under a dishonest majority, so
    /// the strongest floor reachable in that regime is identifiable abort; flooring
    /// any weaker would let a semi-honest backend through.) `[OPUS-4.8]`
    pub fn dishonest_majority_malicious() -> Self {
        SecurityRequirement {
            min_adversary: AdversaryModel::Malicious,
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Identifiable),
            max_corruption: CorruptionThreshold::DishonestMajority { t: 1 },
            require_cheater_attribution: true,
            require_public_verifiability: false,
            max_preprocessing: None,
        }
    }

    /// Check a backend's [`SecurityDescriptor`] against every axis of this
    /// requirement. `true` iff the descriptor meets the floor on ALL of:
    /// adversary, output guarantee, corruption regime, plus the attribution and
    /// public-verifiability flags. This is the per-descriptor core shared by
    /// [`SecurityRequirement::satisfies`] (backend-level) and the per-operator
    /// path. This checks security only: a descriptor contains no preprocessing
    /// cost. Use [`SecurityRequirement::satisfies`] or the registry for budgets.
    pub fn satisfied_by(&self, desc: &SecurityDescriptor) -> bool {
        // AXIS-1 honours the covert deterrence ε (not just the coarse tier rank)
        // and AXIS-3 honours the concrete corruption count `t` (not just the
        // regime rank), so neither axis can silently accept a weaker guarantee
        // than requested — see [`adversary_satisfies`] / [`corruption_satisfies`]
        // (Copilot review #92). `[OPUS-4.8]`
        adversary_satisfies(desc.adversary, self.min_adversary)
            && output_guarantee_rank(desc.output_guarantee)
                >= output_guarantee_rank(self.min_output_guarantee)
            && corruption_satisfies(desc.threshold, self.max_corruption)
            && (!self.require_cheater_attribution || provides_cheater_attribution(desc))
            && (!self.require_public_verifiability || desc.public_verifiability.0)
    }

    /// Check a backend's [`BackendInfo`] (its PRIMARY/headline reconstruction
    /// path) against this requirement. Equivalent to
    /// [`SecurityRequirement::satisfied_by`] on `info.security`, plus the optional
    /// preprocessing budget. For an operator whose guarantee differs from the
    /// headline (the Shamir degree-`2t` equality
    /// open), match against [`MpcBackend::operator_security`] instead — see
    /// [`BackendRegistry::select_for_operator`]. `[OPUS-4.8]`
    pub fn satisfies(&self, info: &BackendInfo) -> bool {
        self.satisfied_by(&info.security)
            && self
                .max_preprocessing
                .is_none_or(|budget| budget.refusal(info.requires_preprocessing).is_none())
    }

    /// A compact human-readable rendering of the requirement, for the
    /// [`MpcError::NoBackendSatisfies`] message on a fail-closed refusal.
    /// `[OPUS-4.8]`
    fn describe(&self) -> String {
        let adversary = match self.min_adversary {
            AdversaryModel::SemiHonest => "semi-honest".to_string(),
            AdversaryModel::Covert { .. } => match self.min_adversary.deterrence() {
                Some((num, den)) => format!("covert(ε={num}/{den})"),
                None => "covert".to_string(),
            },
            AdversaryModel::Malicious => "malicious".to_string(),
        };
        let guarantee = match self.min_output_guarantee {
            OutputGuarantee::Abort(AbortKind::Selective) => "abort(selective)",
            OutputGuarantee::Abort(AbortKind::Unanimous) => "abort(unanimous)",
            OutputGuarantee::Abort(AbortKind::Identifiable) => "abort(identifiable)",
            OutputGuarantee::Fairness(_) => "fairness",
            OutputGuarantee::GuaranteedOutput(_) => "guaranteed-output",
        };
        // Include the concrete corruption count `t`: now that `satisfied_by`
        // ENFORCES `desc.t >= req.t` (Copilot review #92), two requirements that
        // differ only in `t` are genuinely different floors, so the refusal
        // message must distinguish them or it is ambiguous to debug. `[OPUS-4.8]`
        let regime = match self.max_corruption {
            CorruptionThreshold::DishonestMajority { t } => format!("dishonest-majority(t={t})"),
            CorruptionThreshold::HonestMajority { t } => format!("honest-majority(t={t})"),
            CorruptionThreshold::SuperHonestMajority { t } => {
                format!("super-honest-majority(t={t})")
            }
        };
        format!(
            "adversary>={adversary}, output>={guarantee}, corruption<={regime}, \
             attribution={}, public-verifiability={}",
            self.require_cheater_attribution, self.require_public_verifiability
        )
    }
}

/// A **fail-closed registry** of MPC backends, all of one scheme family
/// `B: MpcBackend` (research §1.3(b)). A federation registers the backends it is
/// willing to run, then states a [`SecurityRequirement`];
/// [`BackendRegistry::select`] returns the STRONGEST registered backend that
/// meets it, or refuses with [`MpcError::NoBackendSatisfies`] or
/// [`MpcError::PreprocessingBudgetRefused`] — it NEVER returns
/// a backend that fails the requirement.
///
/// Generic over the concrete backend type `B` (not `dyn MpcBackend`): the trait
/// carries an associated `Share` type and so is not object-safe, and at v1 a
/// federation runs ONE scheme family anyway (the crate ships exactly one,
/// [`crate::shamir::ShamirBackend`]). When a second family lands, a federation
/// uses a second registry per family — selection is a per-family decision because
/// the `Share` type, round model, and preprocessing differ (module-level docs).
/// `[OPUS-4.8]`
#[derive(Debug)]
pub struct BackendRegistry<B: MpcBackend> {
    backends: Vec<B>,
}

impl<B: MpcBackend> Default for BackendRegistry<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: MpcBackend> BackendRegistry<B> {
    /// An empty registry.
    pub fn new() -> Self {
        BackendRegistry {
            backends: Vec::new(),
        }
    }

    /// Register a backend the federation is willing to run. Returns `&mut self`
    /// for chaining. `[OPUS-4.8]`
    pub fn register(&mut self, backend: B) -> &mut Self {
        self.backends.push(backend);
        self
    }

    /// Number of registered backends.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// `true` iff no backend is registered.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Select the strongest backend meeting the security floor and preprocessing budget.
    ///
    /// Matches each backend's headline [`BackendInfo`] against `req`; among
    /// those that pass, returns the one whose descriptor ranks highest. Without
    /// a budget, selection and registration-order tie breaking are unchanged.
    /// An unsatisfiable request is refused, never silently downgraded.
    ///
    /// # Errors
    /// Returns [`MpcError::NoBackendSatisfies`] if no backend meets the security
    /// floor. Returns [`MpcError::PreprocessingBudgetRefused`] if security-qualified
    /// backends all exceed a set ceiling or have unknown/overflowing estimates.
    pub fn select(&self, req: &SecurityRequirement) -> Result<&B, MpcError> {
        self.select_by(req, |b| b.info().security)
    }

    /// Like [`BackendRegistry::select`] but matches against the per-operator
    /// guarantee for `operator` ([`MpcBackend::operator_security`]) instead of the
    /// headline [`BackendInfo`]. This is the correct path when a federation cares
    /// about a specific operator whose guarantee differs from the backend's
    /// primary one — e.g. the Shamir degree-`2t` [`OperatorClass::EqualityJoin`]
    /// open is semi-honest-only at `n = 2t+1` even though the degree-`t`
    /// [`OperatorClass::LinearAggregate`] is robust, so a robustness requirement
    /// that the aggregate meets may be (correctly) refused for the equality join.
    /// Same fail-closed contract. `[OPUS-4.8]`
    /// The backend's declared cost is evaluated for the budget's full workload;
    /// the operator changes only the security descriptor used for selection.
    ///
    /// # Errors
    /// Returns the same typed security or budget refusals as [`Self::select`].
    pub fn select_for_operator(
        &self,
        req: &SecurityRequirement,
        operator: OperatorClass,
    ) -> Result<&B, MpcError> {
        self.select_by(req, |b| b.operator_security(operator))
    }

    /// Shared selection core: pick the registered backend whose descriptor (as
    /// extracted by `descriptor_of`) satisfies `req` and ranks strongest, or fail
    /// closed. Strength is the lexicographic
    /// `(output_guarantee, adversary, corruption)` rank — output guarantee first
    /// because it is the property a federation feels most directly (does it get a
    /// guaranteed answer?). `[OPUS-4.8]`
    fn select_by(
        &self,
        req: &SecurityRequirement,
        descriptor_of: impl Fn(&B) -> SecurityDescriptor,
    ) -> Result<&B, MpcError> {
        let mut best: Option<(&B, (u8, u8, u8))> = None;
        let mut rejected = Vec::new();
        for b in &self.backends {
            let desc = descriptor_of(b);
            if !req.satisfied_by(&desc) {
                continue;
            }
            if let Some(budget) = req.max_preprocessing {
                let info = b.info();
                if let Some(reason) = budget.refusal(info.requires_preprocessing) {
                    rejected.push(format!("{}: {reason}", info.name));
                    continue;
                }
            }
            let rank = (
                output_guarantee_rank(desc.output_guarantee),
                adversary_rank(desc.adversary),
                threshold_rank(desc.threshold),
            );
            match &best {
                Some((_, best_rank)) if *best_rank >= rank => {}
                _ => best = Some((b, rank)),
            }
        }
        best.map(|(b, _)| b).ok_or_else(|| {
            if let Some(budget) = req.max_preprocessing.filter(|_| !rejected.is_empty()) {
                MpcError::PreprocessingBudgetRefused {
                    requirement: req.describe(),
                    budget,
                    rejected,
                }
            } else {
                MpcError::NoBackendSatisfies {
                    requirement: req.describe(),
                    considered: self.backends.len(),
                }
            }
        })
    }
}

// =============================================================================
// [OPUS-4.8] sq-mq8q — three-axis security descriptor tests: the axes construct
// correctly; the Cleve impossibility is a type-level invariant (a
// DishonestMajority + GuaranteedOutput is UNREPRESENTABLE — the constructor
// refuses it); the back-compat projection maps old↔new faithfully. Per-operator
// reporting for the real Shamir backend is exercised in `adversarial_tests.rs`
// where the `(n, t)` fixtures live. Fable unavailable — flag for re-review.
// =============================================================================
#[cfg(test)]
mod axis_tests {
    use super::*;

    // --- AXIS-1: AdversaryModel constructs across all three tiers. ----------
    #[test]
    fn adversary_model_three_tiers_construct() {
        assert_eq!(AdversaryModel::SemiHonest, AdversaryModel::SemiHonest);
        // Covert carries an exact rational deterrence ε = 1/2, via the checked
        // constructor (the only way to build the variant).
        let covert = AdversaryModel::covert(1, 2).expect("ε = 1/2 is a valid probability");
        assert_ne!(covert, AdversaryModel::SemiHonest);
        assert_ne!(covert, AdversaryModel::Malicious);
        assert_ne!(AdversaryModel::Malicious, AdversaryModel::SemiHonest);
        assert_eq!(covert.deterrence(), Some((1, 2)));
        assert_eq!(AdversaryModel::SemiHonest.deterrence(), None);
        assert_eq!(AdversaryModel::Malicious.deterrence(), None);
    }

    // The covert ε invariant (`den > 0`, `0 < num <= den`) is ENFORCED by the
    // constructor, not merely documented — a nonsensical ε returns `None` rather
    // than building a descriptor that misrepresents the deterrence parameter.
    // `[OPUS-4.8]` (Copilot review #87).
    #[test]
    fn covert_constructor_enforces_epsilon_invariant() {
        // Valid: ε in (0, 1].
        assert!(AdversaryModel::covert(1, 2).is_some());
        assert!(AdversaryModel::covert(1, 1).is_some()); // ε = 1 (always caught)
        assert!(AdversaryModel::covert(3, 4).is_some());
        // Invalid: zero denominator (division by zero / undefined ε).
        assert_eq!(AdversaryModel::covert(1, 0), None);
        // Invalid: ε > 1 (num > den) — not a probability.
        assert_eq!(AdversaryModel::covert(3, 2), None);
        // Invalid: ε = 0 (num == 0) — that is the semi-honest regime, not covert.
        assert_eq!(AdversaryModel::covert(0, 2), None);
        assert_eq!(AdversaryModel::covert(0, 0), None);
    }

    // --- AXIS-3: CorruptionThreshold carries t and classifies the regime. ---
    #[test]
    fn corruption_threshold_carries_t_and_classifies_regime() {
        // n > 3t → super-honest (perfect GOD, BGW point-to-point).
        assert_eq!(
            CorruptionThreshold::from_n_t(4, 1),
            CorruptionThreshold::SuperHonestMajority { t: 1 }
        );
        // 2t < n <= 3t → honest (statistical GOD, needs broadcast).
        assert_eq!(
            CorruptionThreshold::from_n_t(5, 2),
            CorruptionThreshold::HonestMajority { t: 2 }
        );
        // n <= 2t → dishonest majority.
        assert_eq!(
            CorruptionThreshold::from_n_t(3, 2),
            CorruptionThreshold::DishonestMajority { t: 2 }
        );
        // t is carried (and recoverable) in every regime.
        assert_eq!(
            CorruptionThreshold::DishonestMajority { t: 7 }.threshold(),
            7
        );
        assert_eq!(CorruptionThreshold::HonestMajority { t: 3 }.threshold(), 3);
        assert_eq!(
            CorruptionThreshold::SuperHonestMajority { t: 2 }.threshold(),
            2
        );
        // The honest-majority classifier.
        assert!(CorruptionThreshold::HonestMajority { t: 1 }.is_honest_majority());
        assert!(CorruptionThreshold::SuperHonestMajority { t: 1 }.is_honest_majority());
        assert!(!CorruptionThreshold::DishonestMajority { t: 1 }.is_honest_majority());
    }

    // --- AXIS-2 + CLEVE: the impossibility is a type-level invariant. -------
    // Fairness/GuaranteedOutput are ONLY constructible under an honest majority;
    // a DishonestMajority + GuaranteedOutput is unrepresentable (the constructor
    // returns None — and the variant's private field makes it the only path).
    #[test]
    fn cleve_invariant_dishonest_majority_cannot_construct_fairness_or_god() {
        let dishonest = CorruptionThreshold::DishonestMajority { t: 2 };
        assert_eq!(
            OutputGuarantee::fairness(dishonest),
            None,
            "Cleve: fairness is impossible without an honest majority"
        );
        assert_eq!(
            OutputGuarantee::guaranteed_output(dishonest),
            None,
            "Cleve: guaranteed output (GOD) is impossible without an honest majority"
        );
        // Abort, by contrast, IS available under a dishonest majority.
        let abort = OutputGuarantee::Abort(AbortKind::Unanimous);
        assert!(abort.is_abort());
        assert!(!abort.requires_honest_majority());
    }

    #[test]
    fn cleve_invariant_honest_majority_can_construct_fairness_and_god() {
        for threshold in [
            CorruptionThreshold::HonestMajority { t: 1 },
            CorruptionThreshold::SuperHonestMajority { t: 1 },
        ] {
            let fair =
                OutputGuarantee::fairness(threshold).expect("honest majority admits fairness");
            let god =
                OutputGuarantee::guaranteed_output(threshold).expect("honest majority admits GOD");
            assert!(fair.requires_honest_majority());
            assert!(god.requires_honest_majority());
            assert!(!fair.is_abort());
            assert!(!god.is_abort());
            assert_ne!(fair, god);
        }
    }

    // The private witness field means the ONLY way to obtain a Fairness/GOD value
    // is the Cleve-checking constructor — there is no struct-literal escape
    // hatch from outside `backend`. (This test lives inside the module so it can
    // even *name* the private token; external code cannot, which is the point.)
    #[test]
    fn cleve_god_value_implies_an_honest_majority_was_proven() {
        let god = OutputGuarantee::guaranteed_output(CorruptionThreshold::HonestMajority { t: 1 })
            .unwrap();
        // The value exists, so a HonestMajorityProof token was minted — which only
        // the honest-majority constructor can do. Round-trip the discriminant.
        assert!(matches!(god, OutputGuarantee::GuaranteedOutput(_)));
    }

    // --- Back-compat PROJECTION: new descriptor → old enums, faithfully. ----
    #[test]
    fn projection_trust_model_collapses_axis3_to_binary() {
        let mk = |threshold| SecurityDescriptor {
            adversary: AdversaryModel::SemiHonest,
            output_guarantee: OutputGuarantee::Abort(AbortKind::Unanimous),
            threshold,
            public_verifiability: PublicVerifiability(false),
        };
        assert_eq!(
            mk(CorruptionThreshold::SuperHonestMajority { t: 1 }).trust_model(),
            TrustModel::HonestMajority
        );
        assert_eq!(
            mk(CorruptionThreshold::HonestMajority { t: 1 }).trust_model(),
            TrustModel::HonestMajority
        );
        assert_eq!(
            mk(CorruptionThreshold::DishonestMajority { t: 1 }).trust_model(),
            TrustModel::DishonestMajority
        );
    }

    #[test]
    fn projection_malicious_security_maps_each_case() {
        // Semi-honest + no-detection selective abort → SemiHonestOnly.
        let semi = SecurityDescriptor::semi_honest_only(3, 1);
        assert_eq!(
            semi.malicious_security(0),
            MaliciousSecurity::SemiHonestOnly
        );
        // Detect-and-abort (redundancy, e == 0) → HonestMajorityAbort.
        let abort = SecurityDescriptor::shamir_degree_recon(3, 1, 0);
        assert_eq!(
            abort.malicious_security(0),
            MaliciousSecurity::HonestMajorityAbort
        );
        // Robust (GOD, e >= 1) → HonestMajorityRobust{ max_cheaters: e }, with the
        // budget threaded through the projection (it cannot be recovered from the
        // abstract OutputGuarantee alone).
        let robust = SecurityDescriptor::shamir_degree_recon(4, 1, 1);
        assert_eq!(
            robust.malicious_security(1),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 1 }
        );
        let robust2 = SecurityDescriptor::shamir_degree_recon(9, 4, 2);
        assert_eq!(
            robust2.malicious_security(2),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 2 }
        );
    }

    // BackendInfo::new keeps the projection fields internally consistent with the
    // three-axis descriptor (they can never drift).
    #[test]
    fn backend_info_new_derives_consistent_projection() {
        let info = BackendInfo::new("x", SecurityDescriptor::shamir_degree_recon(4, 1, 1), 1);
        assert_eq!(info.trust_model, TrustModel::HonestMajority);
        assert_eq!(
            info.malicious_security,
            MaliciousSecurity::HonestMajorityRobust { max_cheaters: 1 }
        );
        assert!(info.malicious_secure());
        // And the new descriptor is preserved verbatim.
        assert_eq!(info.security.adversary, AdversaryModel::SemiHonest);
        assert!(info.security.output_guarantee.requires_honest_majority());
    }

    // FAIL-CLOSED: `shamir_degree_recon` with a dishonest-majority `(n, t)`
    // (`n <= 2t`) must NOT emit an honest-majority active-security claim. The
    // RS-redundancy detect-and-abort it would otherwise encode is honest-majority
    // specific; under a dishonest majority it degrades to the semi-honest-only
    // baseline, so the projection stays internally consistent (no
    // `trust_model = DishonestMajority` + `malicious_security = HonestMajority…`).
    // `[OPUS-4.8]` sq-mq8q (Copilot review #87).
    #[test]
    fn shamir_degree_recon_fails_closed_under_dishonest_majority() {
        // n = 2, t = 1 → DishonestMajority (n <= 2t). Even with robust_cheaters = 0
        // (the detect-and-abort case) this must not claim honest-majority security.
        let desc = SecurityDescriptor::shamir_degree_recon(2, 1, 0);
        assert_eq!(
            desc.threshold,
            CorruptionThreshold::DishonestMajority { t: 1 }
        );
        assert_eq!(desc.trust_model(), TrustModel::DishonestMajority);
        assert_eq!(desc.adversary, AdversaryModel::SemiHonest);
        // Degrades to the no-detection selective-abort baseline...
        assert_eq!(
            desc.output_guarantee,
            OutputGuarantee::Abort(AbortKind::Selective)
        );
        // ...so the back-compat projection is SemiHonestOnly, NOT HonestMajorityAbort.
        // The two projections are now mutually consistent.
        assert_eq!(
            desc.malicious_security(0),
            MaliciousSecurity::SemiHonestOnly
        );
        assert!(!desc.malicious_security(0).is_malicious_secure());

        // Even a non-zero correction budget cannot resurrect an honest-majority
        // claim under a dishonest majority (Cleve already blocks GOD; the abort
        // path is blocked here).
        let desc_budget = SecurityDescriptor::shamir_degree_recon(3, 2, 1);
        assert_eq!(
            desc_budget.threshold,
            CorruptionThreshold::DishonestMajority { t: 2 }
        );
        assert_eq!(
            desc_budget.malicious_security(1),
            MaliciousSecurity::SemiHonestOnly
        );
    }
}

// =============================================================================
// [OPUS-4.8] sq-a6p1 — SELECTION / NEGOTIATION + FAIL-CLOSED registry tests.
// The matrix the bead pins: a semi-honest backend is REFUSED for a malicious
// requirement; an honest-majority backend is REFUSED for a dishonest-majority
// requirement; a satisfiable request RETURNS the backend; NoBackendSatisfies on
// impossible requests. Fail-closed is the load-bearing property. Fable
// unavailable — flag for re-review.
// =============================================================================
#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::holder::Holder;

    /// A configurable stub backend that reports a caller-supplied
    /// [`SecurityDescriptor`] from `info()` (and the same from `operator_security`
    /// for every operator). Its crypto methods are honest `NotYetImplemented`
    /// stubs — selection NEVER runs crypto, it only inspects descriptors, so the
    /// stub is sufficient to exercise the whole matrix without standing up real
    /// Shamir parties. `[OPUS-4.8]`
    #[derive(Debug)]
    struct DescBackend {
        name: &'static str,
        desc: SecurityDescriptor,
        robust_cheaters: usize,
        cost: PreprocessingCost,
    }

    impl DescBackend {
        fn new(name: &'static str, desc: SecurityDescriptor) -> Self {
            DescBackend {
                name,
                desc,
                robust_cheaters: 0,
                cost: PreprocessingCost::None,
            }
        }
    }

    impl MpcBackend for DescBackend {
        type Share = ();
        fn info(&self) -> BackendInfo {
            let mut info = BackendInfo::new(self.name, self.desc, self.robust_cheaters);
            info.requires_preprocessing = self.cost;
            info
        }
        fn share_private_input(&self, _h: &Holder) -> Result<Vec<()>, MpcError> {
            Err(MpcError::not_yet("secret-share private input", "stub"))
        }
        fn run_secure(&self, _s: &[()]) -> Result<Vec<()>, MpcError> {
            Err(MpcError::not_yet("run secure computation", "stub"))
        }
        fn reconstruct_disclosed(&self, _s: &[()]) -> Result<PartialResult, MpcError> {
            Err(MpcError::not_yet("reconstruct disclosed output", "stub"))
        }
    }

    // --- Descriptor fixtures spanning the axes. -----------------------------

    /// Honest-majority, semi-honest, detect-and-abort (unanimous) — the shipped
    /// v1 Shamir aggregate's headline guarantee.
    fn honest_semi_abort() -> SecurityDescriptor {
        SecurityDescriptor {
            adversary: AdversaryModel::SemiHonest,
            output_guarantee: OutputGuarantee::Abort(AbortKind::Unanimous),
            threshold: CorruptionThreshold::HonestMajority { t: 1 },
            public_verifiability: PublicVerifiability(false),
        }
    }

    /// Honest-majority, MALICIOUS, robust guaranteed-output — a hypothetical
    /// hardened honest-majority backend (Goyal-Liu-Song "free" malicious compiler).
    fn honest_malicious_god() -> SecurityDescriptor {
        SecurityDescriptor {
            adversary: AdversaryModel::Malicious,
            output_guarantee: OutputGuarantee::guaranteed_output(
                CorruptionThreshold::HonestMajority { t: 1 },
            )
            .expect("honest majority admits GOD"),
            threshold: CorruptionThreshold::HonestMajority { t: 1 },
            public_verifiability: PublicVerifiability(false),
        }
    }

    /// Dishonest-majority, malicious, identifiable abort + public verifiability —
    /// a hypothetical SPDZ-with-PVC backend (NOT shipped; used to prove that when
    /// such a backend IS registered the demanding request is satisfied, i.e. the
    /// registry is not refusing by accident).
    fn dishonest_malicious_ia_pvc() -> SecurityDescriptor {
        SecurityDescriptor {
            adversary: AdversaryModel::Malicious,
            output_guarantee: OutputGuarantee::Abort(AbortKind::Identifiable),
            threshold: CorruptionThreshold::DishonestMajority { t: 2 },
            public_verifiability: PublicVerifiability(true),
        }
    }

    fn budget() -> PreprocessingBudget {
        // 3 * 2 + 2 * 5 = 16 items; 16 * 8 = 128 bytes per party.
        PreprocessingBudget {
            multiplications: 3,
            comparisons: 2,
            max_items: Some(16),
            max_bytes_per_party: Some(128),
        }
    }

    fn cost(bytes: Option<u64>) -> PreprocessingCost {
        PreprocessingCost::PerOperation {
            items_per_multiplication: 2,
            items_per_comparison: 5,
            bytes_per_item_per_party: bytes,
        }
    }

    fn budget_registry(cost: PreprocessingCost) -> BackendRegistry<DescBackend> {
        let mut reg = BackendRegistry::new();
        let mut backend = DescBackend::new("candidate", honest_semi_abort());
        backend.cost = cost;
        reg.register(backend);
        reg
    }

    fn budget_requirement(budget: PreprocessingBudget) -> SecurityRequirement {
        SecurityRequirement {
            max_preprocessing: Some(budget),
            ..SecurityRequirement::v1_honest_majority_semi_honest()
        }
    }

    fn assert_selected(reg: &BackendRegistry<DescBackend>, req: &SecurityRequirement, name: &str) {
        assert_eq!(reg.select(req).unwrap().info().name, name);
        assert_eq!(
            reg.select_for_operator(req, OperatorClass::Comparison)
                .unwrap()
                .info()
                .name,
            name
        );
    }

    fn assert_budget_refused(reg: &BackendRegistry<DescBackend>, req: &SecurityRequirement) {
        for result in [
            reg.select(req),
            reg.select_for_operator(req, OperatorClass::Comparison),
        ] {
            let err = result.expect_err("cost must fail closed before selecting a backend");
            assert!(err.to_string().contains("preprocessing budget"));
            match err {
                MpcError::PreprocessingBudgetRefused {
                    requirement,
                    budget,
                    rejected,
                } => {
                    assert_eq!(requirement, req.describe());
                    assert_eq!(Some(budget), req.max_preprocessing);
                    assert_eq!(rejected.len(), 1);
                    assert!(rejected[0].starts_with("candidate: items="));
                    assert!(rejected[0].contains("bytes-per-party="));
                }
                other => panic!("expected a typed budget refusal, got {other:?}"),
            }
        }
        assert!(!req.satisfies(&reg.backends[0].info()));
        // Cost does not change whether the security-only descriptor meets the floor.
        assert!(req.satisfied_by(&reg.backends[0].desc));
    }

    #[test]
    fn preprocessing_no_budget_preserves_strength_and_tie_order() {
        let mut reg = budget_registry(cost(Some(u64::MAX)));
        for name in ["first-strong", "second-strong"] {
            let mut backend = DescBackend::new(name, honest_malicious_god());
            backend.cost = cost(None);
            reg.register(backend);
        }
        let req = SecurityRequirement::v1_honest_majority_semi_honest();
        assert_eq!(req.max_preprocessing, None);
        assert_eq!(
            SecurityRequirement::dishonest_majority_malicious().max_preprocessing,
            None
        );
        assert_selected(&reg, &req, "first-strong");
        assert!(req.satisfies(&reg.backends[1].info()));
    }

    #[test]
    fn preprocessing_budget_satisfied_at_inclusive_ceilings() {
        let reg = budget_registry(cost(Some(8)));
        let req = budget_requirement(budget());
        assert_selected(&reg, &req, "candidate");
        assert!(req.satisfies(&reg.backends[0].info()));
    }

    #[test]
    fn preprocessing_item_ceiling_exceeded_is_typed_refusal() {
        let reg = budget_registry(cost(Some(8)));
        let req = budget_requirement(PreprocessingBudget {
            max_items: Some(15),
            ..budget()
        });
        assert_budget_refused(&reg, &req);
    }

    #[test]
    fn preprocessing_byte_ceiling_exceeded_is_typed_refusal() {
        let reg = budget_registry(cost(Some(8)));
        let req = budget_requirement(PreprocessingBudget {
            max_bytes_per_party: Some(127),
            ..budget()
        });
        assert_budget_refused(&reg, &req);
    }

    #[test]
    fn preprocessing_unknown_bytes_with_ceiling_is_typed_refusal() {
        let reg = budget_registry(cost(None));
        assert_budget_refused(&reg, &budget_requirement(budget()));
        // Even a zero workload cannot turn an unmodeled byte cost into known zero.
        let zero = PreprocessingBudget {
            multiplications: 0,
            comparisons: 0,
            ..budget()
        };
        assert_budget_refused(&reg, &budget_requirement(zero));
    }

    #[test]
    fn preprocessing_unknown_bytes_without_byte_ceiling_is_selectable() {
        let reg = budget_registry(cost(None));
        let req = budget_requirement(PreprocessingBudget {
            max_bytes_per_party: None,
            ..budget()
        });
        assert_selected(&reg, &req, "candidate");
        assert!(req.satisfies(&reg.backends[0].info()));
    }

    #[test]
    fn preprocessing_only_fitting_backend_is_selected() {
        let mut reg = budget_registry(cost(Some(9)));
        let mut fitting = DescBackend::new("fits", honest_semi_abort());
        fitting.cost = cost(Some(8));
        reg.register(fitting);
        let mut strongest = DescBackend::new("strong-but-unknown", honest_malicious_god());
        strongest.cost = cost(None);
        reg.register(strongest);
        assert_selected(&reg, &budget_requirement(budget()), "fits");
    }

    #[test]
    fn preprocessing_overflow_is_typed_refusal() {
        for (multiplications, comparisons, bytes) in [
            (u64::MAX, 0, Some(1)),     // multiplication estimate overflows
            (0, u64::MAX, Some(1)),     // comparison estimate overflows
            (u64::MAX / 2, 1, Some(1)), // sum overflows
            (3, 2, Some(u64::MAX)),     // only byte estimate overflows
        ] {
            let reg = budget_registry(cost(bytes));
            let workload = PreprocessingBudget {
                multiplications,
                comparisons,
                max_items: Some(u64::MAX),
                max_bytes_per_party: Some(u64::MAX),
            };
            assert_budget_refused(&reg, &budget_requirement(workload));
            // A byte-only ceiling must also refuse an overflowing item estimate.
            assert_budget_refused(
                &reg,
                &budget_requirement(PreprocessingBudget {
                    max_items: None,
                    ..workload
                }),
            );
        }
    }

    #[test]
    fn preprocessing_unset_ceilings_and_no_preprocessing_are_selectable() {
        let reg = budget_registry(cost(None));
        let unrestricted = PreprocessingBudget {
            multiplications: u64::MAX,
            comparisons: u64::MAX,
            max_items: None,
            max_bytes_per_party: None,
        };
        assert_selected(&reg, &budget_requirement(unrestricted), "candidate");
        let reg = budget_registry(PreprocessingCost::None);
        assert_selected(
            &reg,
            &budget_requirement(PreprocessingBudget {
                max_items: Some(0),
                max_bytes_per_party: Some(0),
                ..unrestricted
            }),
            "candidate",
        );
    }

    #[test]
    fn preprocessing_budget_does_not_replace_security_refusal() {
        let mut reg = budget_registry(PreprocessingCost::None);
        let req = SecurityRequirement {
            max_preprocessing: Some(budget()),
            ..SecurityRequirement::dishonest_majority_malicious()
        };
        for empty in [false, true] {
            if empty {
                reg.backends.clear();
            }
            for result in [
                reg.select(&req),
                reg.select_for_operator(&req, OperatorClass::Comparison),
            ] {
                assert!(
                    matches!(result, Err(MpcError::NoBackendSatisfies { considered, .. })
                    if considered == reg.len())
                );
            }
        }
    }

    // === satisfies() per-axis matrix ========================================

    #[test]
    fn satisfies_semi_honest_backend_refused_for_malicious_requirement() {
        let info = BackendInfo::new("semi", honest_semi_abort(), 0);
        let req = SecurityRequirement {
            min_adversary: AdversaryModel::Malicious,
            ..SecurityRequirement::v1_honest_majority_semi_honest()
        };
        assert!(
            !req.satisfies(&info),
            "a semi-honest backend must NOT satisfy a malicious-adversary requirement"
        );
        // A malicious backend DOES satisfy it (sanity: the floor is real, not vacuous).
        let mal = BackendInfo::new("mal", honest_malicious_god(), 1);
        assert!(req.satisfies(&mal));
    }

    #[test]
    fn satisfies_honest_majority_backend_refused_for_dishonest_majority_requirement() {
        // A backend that only holds under an HONEST majority cannot satisfy a
        // requirement that demands security under a DISHONEST majority.
        let honest = BackendInfo::new("honest", honest_malicious_god(), 1);
        let req = SecurityRequirement {
            min_adversary: AdversaryModel::SemiHonest,
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
            max_corruption: CorruptionThreshold::DishonestMajority { t: 1 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        };
        assert!(
            !req.satisfies(&honest),
            "an honest-majority backend must NOT satisfy a dishonest-majority requirement"
        );
        // A genuine dishonest-majority backend satisfies it.
        let dishonest = BackendInfo::new("dis", dishonest_malicious_ia_pvc(), 0);
        assert!(req.satisfies(&dishonest));
    }

    #[test]
    fn satisfies_output_guarantee_floor_is_ordered() {
        // Floor = unanimous abort. Selective < unanimous (refused); unanimous,
        // identifiable, fairness all >= unanimous (accepted).
        let req = SecurityRequirement {
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Unanimous),
            ..SecurityRequirement::v1_honest_majority_semi_honest()
        };
        let mk = |g| {
            BackendInfo::new(
                "x",
                SecurityDescriptor {
                    output_guarantee: g,
                    ..honest_semi_abort()
                },
                0,
            )
        };
        assert!(!req.satisfies(&mk(OutputGuarantee::Abort(AbortKind::Selective))));
        assert!(req.satisfies(&mk(OutputGuarantee::Abort(AbortKind::Unanimous))));
        assert!(req.satisfies(&mk(OutputGuarantee::Abort(AbortKind::Identifiable))));
        let fairness =
            OutputGuarantee::fairness(CorruptionThreshold::HonestMajority { t: 1 }).unwrap();
        assert!(req.satisfies(&mk(fairness)));
    }

    #[test]
    fn satisfies_attribution_and_public_verifiability_flags() {
        let base = SecurityRequirement {
            min_adversary: AdversaryModel::SemiHonest,
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
            max_corruption: CorruptionThreshold::HonestMajority { t: 1 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        };
        // Without the flags, the plain semi-honest backend passes.
        let plain = BackendInfo::new("plain", honest_semi_abort(), 0);
        assert!(base.satisfies(&plain));

        // require_cheater_attribution: only an identifiable-abort descriptor passes.
        let need_attr = SecurityRequirement {
            require_cheater_attribution: true,
            ..base
        };
        assert!(
            !need_attr.satisfies(&plain),
            "unanimous abort is not sound IA"
        );
        let ia = BackendInfo::new(
            "ia",
            SecurityDescriptor {
                output_guarantee: OutputGuarantee::Abort(AbortKind::Identifiable),
                ..honest_semi_abort()
            },
            0,
        );
        assert!(need_attr.satisfies(&ia));

        // require_public_verifiability: only a PV descriptor passes.
        let need_pv = SecurityRequirement {
            require_public_verifiability: true,
            ..base
        };
        assert!(!need_pv.satisfies(&plain));
        let pv = BackendInfo::new(
            "pv",
            SecurityDescriptor {
                public_verifiability: PublicVerifiability(true),
                ..honest_semi_abort()
            },
            0,
        );
        assert!(need_pv.satisfies(&pv));
    }

    // === registry.select() fail-closed contract =============================

    #[test]
    fn select_returns_backend_for_satisfiable_request() {
        let mut reg: BackendRegistry<DescBackend> = BackendRegistry::new();
        reg.register(DescBackend::new("shamir-like", honest_semi_abort()));
        let chosen = reg
            .select(&SecurityRequirement::v1_honest_majority_semi_honest())
            .expect("the v1 floor is satisfiable by the registered backend");
        assert_eq!(chosen.info().name, "shamir-like");
    }

    #[test]
    fn select_fails_closed_on_dishonest_majority_malicious_over_shipped_set() {
        // The shipped set = one honest-majority semi-honest backend. The hardest
        // realistic ask (dishonest-majority malicious) MUST be refused, never
        // downgraded onto the semi-honest backend. THIS is the load-bearing
        // property of the whole deliverable.
        let mut reg: BackendRegistry<DescBackend> = BackendRegistry::new();
        reg.register(DescBackend::new("shamir-like", honest_semi_abort()));
        let req = SecurityRequirement::dishonest_majority_malicious();
        let err = reg
            .select(&req)
            .expect_err("a dishonest-majority-malicious request must be refused fail-closed");
        match err {
            MpcError::NoBackendSatisfies {
                considered,
                requirement,
            } => {
                assert_eq!(
                    considered, 1,
                    "the one registered backend was inspected and rejected"
                );
                assert!(
                    requirement.contains("malicious") && requirement.contains("dishonest-majority"),
                    "the refusal names the unmet requirement: {requirement}"
                );
            }
            other => panic!("expected NoBackendSatisfies, got {other:?}"),
        }
    }

    #[test]
    fn select_fails_closed_on_empty_registry() {
        let reg: BackendRegistry<DescBackend> = BackendRegistry::new();
        let err = reg
            .select(&SecurityRequirement::v1_honest_majority_semi_honest())
            .expect_err("an empty registry satisfies nothing");
        assert!(matches!(
            err,
            MpcError::NoBackendSatisfies { considered: 0, .. }
        ));
    }

    #[test]
    fn select_satisfied_once_a_qualifying_backend_is_registered() {
        // The SAME demanding request that fails closed over the shipped set
        // succeeds the moment a backend that genuinely meets it is registered —
        // proving the refusal above is HONEST (capability gap), not a bug.
        let mut reg: BackendRegistry<DescBackend> = BackendRegistry::new();
        reg.register(DescBackend::new("shamir-like", honest_semi_abort()));
        reg.register(DescBackend::new("spdz-pvc", dishonest_malicious_ia_pvc()));
        let chosen = reg
            .select(&SecurityRequirement::dishonest_majority_malicious())
            .expect("the dishonest-majority PVC backend satisfies the demanding request");
        assert_eq!(chosen.info().name, "spdz-pvc");
    }

    #[test]
    fn select_returns_strongest_among_satisfiers() {
        // Two backends both clear a minimal floor; select returns the STRONGER
        // (higher output-guarantee rank), so asking for the minimum still yields
        // the best available.
        let mut reg: BackendRegistry<DescBackend> = BackendRegistry::new();
        reg.register(DescBackend::new("weak", honest_semi_abort())); // unanimous abort
        reg.register(DescBackend {
            name: "strong",
            desc: honest_malicious_god(),
            robust_cheaters: 1,
            cost: PreprocessingCost::None,
        }); // GOD
        let req = SecurityRequirement {
            min_adversary: AdversaryModel::SemiHonest,
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
            max_corruption: CorruptionThreshold::HonestMajority { t: 1 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        };
        let chosen = reg.select(&req).expect("both satisfy the minimal floor");
        assert_eq!(
            chosen.info().name,
            "strong",
            "select returns the strongest satisfier"
        );
    }

    // === per-operator selection (composes with operator_security) ===========

    #[test]
    fn select_for_operator_judges_each_operator_on_its_own_guarantee() {
        // Real Shamir backend at n = 3, t = 1. The degree-`t` LinearAggregate has
        // one redundant share (RS budget e = ⌊(3−1−1)/2⌋ = 0) → detect-and-abort
        // (`Abort(Unanimous)`); the degree-`2t` EqualityJoin open has ZERO
        // redundancy at n = 2t+1 → semi-honest-only (`Abort(Selective)`). A
        // requirement demanding BETTER than selective abort is met for the
        // aggregate but REFUSED for the equality join — the registry judges each
        // operator on its OWN descriptor via operator_security, not the headline
        // bit. (For robust GOD on the aggregate you need n >= 4, e >= 1.)
        let mut reg: BackendRegistry<crate::shamir::ShamirBackend> = BackendRegistry::new();
        reg.register(crate::shamir::ShamirBackend::new(3).expect("n=3 backend"));

        // Sanity: pin the exact per-operator guarantees this test depends on.
        let agg = reg.backends[0].operator_security(OperatorClass::LinearAggregate);
        let eqj = reg.backends[0].operator_security(OperatorClass::EqualityJoin);
        assert_eq!(
            agg.output_guarantee,
            OutputGuarantee::Abort(AbortKind::Unanimous),
            "n=3 aggregate: one redundant share → detect-and-abort"
        );
        assert_eq!(
            eqj.output_guarantee,
            OutputGuarantee::Abort(AbortKind::Selective),
            "n=3 degree-2t equality open: no redundancy → semi-honest-only"
        );

        let req = SecurityRequirement {
            min_adversary: AdversaryModel::SemiHonest,
            // Demand BETTER than selective abort (i.e. some real detection).
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Unanimous),
            max_corruption: CorruptionThreshold::HonestMajority { t: 1 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        };

        // Aggregate: satisfied (unanimous-abort meets the unanimous-abort floor).
        assert!(
            reg.select_for_operator(&req, OperatorClass::LinearAggregate)
                .is_ok(),
            "the degree-t aggregate meets the detect-or-better floor"
        );
        // Equality join: refused fail-closed (selective abort < unanimous floor).
        let err = reg
            .select_for_operator(&req, OperatorClass::EqualityJoin)
            .expect_err("the degree-2t equality open is semi-honest-only at n=2t+1");
        assert!(matches!(err, MpcError::NoBackendSatisfies { .. }));
    }

    #[test]
    fn requirement_describe_is_human_readable() {
        let s = SecurityRequirement::dishonest_majority_malicious().describe();
        assert!(s.contains("malicious"));
        assert!(s.contains("dishonest-majority"));
        assert!(s.contains("attribution=true"));
    }

    // === FAIL-CLOSED on the covert-ε and concrete-`t` sub-axes (Copilot #92) ===

    /// A requirement for covert(ε=1/2) must NOT be satisfied by a weaker
    /// covert(ε=1/100) backend — the coarse tier rank is ε-blind, so the ε must
    /// be compared exactly. A stronger covert ε, an equal ε, and a malicious
    /// backend all satisfy it; a weaker covert ε and a semi-honest backend do not.
    /// `[OPUS-4.8]`
    #[test]
    fn satisfies_covert_epsilon_is_not_silently_downgraded() {
        let req = SecurityRequirement {
            min_adversary: AdversaryModel::covert(1, 2).expect("ε=1/2 valid"),
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
            max_corruption: CorruptionThreshold::HonestMajority { t: 1 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        };
        let mk = |adv| {
            BackendInfo::new(
                "x",
                SecurityDescriptor {
                    adversary: adv,
                    ..honest_semi_abort()
                },
                0,
            )
        };
        // Weaker deterrence ε = 1/100 < 1/2 → REFUSED (the bug this fix closes).
        assert!(
            !req.satisfies(&mk(AdversaryModel::covert(1, 100).unwrap())),
            "covert(ε=1/100) must NOT satisfy a covert(ε=1/2) floor"
        );
        // Equal ε = 1/2 (and an equivalent 2/4) → accepted.
        assert!(req.satisfies(&mk(AdversaryModel::covert(1, 2).unwrap())));
        assert!(req.satisfies(&mk(AdversaryModel::covert(2, 4).unwrap())));
        // Stronger ε = 3/4 > 1/2 → accepted.
        assert!(req.satisfies(&mk(AdversaryModel::covert(3, 4).unwrap())));
        // Strictly stronger adversary model (malicious) → accepted (rank dominates).
        assert!(req.satisfies(&mk(AdversaryModel::Malicious)));
        // Weaker adversary model (semi-honest) → refused (rank 0 < 1).
        assert!(!req.satisfies(&mk(AdversaryModel::SemiHonest)));
    }

    /// A requirement to tolerate up to `t=3` corruptions must NOT be satisfied by
    /// a backend that tolerates only `t=1`, even in the same (or a stronger)
    /// regime — the coarse regime rank ignores the concrete `t`. `[OPUS-4.8]`
    #[test]
    fn satisfies_concrete_threshold_t_is_not_silently_downgraded() {
        let req = SecurityRequirement {
            min_adversary: AdversaryModel::SemiHonest,
            min_output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
            max_corruption: CorruptionThreshold::HonestMajority { t: 3 },
            require_cheater_attribution: false,
            require_public_verifiability: false,
            max_preprocessing: None,
        };
        let mk = |threshold| {
            BackendInfo::new(
                "x",
                SecurityDescriptor {
                    threshold,
                    ..honest_semi_abort()
                },
                0,
            )
        };
        // Same regime but fewer tolerated corruptions (t=1 < 3) → REFUSED.
        assert!(
            !req.satisfies(&mk(CorruptionThreshold::HonestMajority { t: 1 })),
            "honest-majority(t=1) must NOT satisfy an honest-majority(t=3) floor"
        );
        // Same regime, t=3 → accepted.
        assert!(req.satisfies(&mk(CorruptionThreshold::HonestMajority { t: 3 })));
        // Stronger regime AND enough t (dishonest-majority tolerates more corruption,
        // t=3 >= 3) → accepted.
        assert!(req.satisfies(&mk(CorruptionThreshold::DishonestMajority { t: 3 })));
        // Stronger regime but too few corruptions (dishonest-majority{t=1}) → REFUSED:
        // a higher regime rank does not excuse a lower concrete threshold.
        assert!(
            !req.satisfies(&mk(CorruptionThreshold::DishonestMajority { t: 1 })),
            "a stronger regime with t=1 still fails a t=3 floor"
        );
    }

    /// The fail-closed refusal message distinguishes requirements that differ
    /// ONLY in the concrete `t` — otherwise the `NoBackendSatisfies` string would
    /// be ambiguous to debug now that `t` is enforced (Copilot #92). `[OPUS-4.8]`
    #[test]
    fn describe_distinguishes_requirements_by_concrete_t() {
        let r1 = SecurityRequirement {
            max_corruption: CorruptionThreshold::HonestMajority { t: 1 },
            ..SecurityRequirement::v1_honest_majority_semi_honest()
        };
        let r3 = SecurityRequirement {
            max_corruption: CorruptionThreshold::HonestMajority { t: 3 },
            ..SecurityRequirement::v1_honest_majority_semi_honest()
        };
        assert_ne!(
            r1.describe(),
            r3.describe(),
            "requirements differing only in t must render differently"
        );
        assert!(r3.describe().contains("honest-majority(t=3)"));
    }
}
