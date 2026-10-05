//! The seam's shared error/phase types and the **still-deferred** phase stubs (design record
//! §4.3 / §8).
//!
//! Phase 1 (sq-2q1x) landed the *shape* of the untrusted-planner → MPC-routing seam — real
//! input/output types wired to the actual upstream crates — with **none of the privacy-bearing
//! logic**. Phase 2 (sq-fix4) makes the **source-selection adapter** real (it lives in
//! [`crate::selection`]). Phase 3 (sq-i1wh2) makes the **disclosed/hidden routing pass** real
//! (it lives in [`crate::routing`]). Phase 4 (sq-pwr.2) makes the **leakage-envelope assembly +
//! dual ratification** real (it lives in [`crate::envelope`]). Phase 5 (sq-1fo4) lands only its
//! *canonicalisation* half in [`crate::binding`]; its **soundness** half stays a deferred stub: a
//! function with its real signature returning `Err(`[`SeamError::Deferred`]`)` naming the phase
//! and the gates it waits on. The stub **compiles and is callable**, performs NO MPC, reveals
//! NOTHING, and never panics (no `todo!()` / `unimplemented!()`): a caller gets an honest typed
//! "deferred" error, not a crash and not a fabricated result.
//!
//! The phases (from the design record's phased plan):
//! * [`select_private_sources`](crate::select_private_sources) — Phase 2 (**implemented**, sq-fix4):
//!   wraps [`sparq_fedplan::select_sources`] and prunes by the per-source
//!   [`crate::SourcePrivacyDescriptor`] (participation / authorisation). See [`crate::selection`].
//! * [`route_operators`](crate::route_operators) — Phase 3 (**implemented**, sq-i1wh2): the
//!   policy-parameterised disclosed/hidden partition, emitting the
//!   [`sparq_mpc::pipeline::OperatorRouting`] vector the pipeline consumes. See [`crate::routing`].
//! * [`assemble_leakage_envelope`](crate::assemble_leakage_envelope) +
//!   [`ratify_envelope`](crate::ratify_envelope) — Phase 4 (**implemented**, sq-pwr.2): collect
//!   what the chosen routing reveals into a declared [`LeakageEnvelope`], then the holder
//!   fail-closed + verifier dual ratification. See [`crate::envelope`].
//! * [`commit_plan`](crate::commit_plan) + [`revalidate_plan`](crate::revalidate_plan) — Phase 5
//!   (**canonicalisation half only**, sq-1fo4): a deterministic, domain-separated commitment to
//!   the produced plan and a fail-closed re-validation of a recomputed plan against it. The
//!   soundness half, [`bind_plan_to_proof`](crate::bind_plan_to_proof), is **deferred**. See
//!   [`crate::binding`].
//!
//! NONE of these makes a soundness/privacy claim; the privacy-bearing posture is research-grade
//! and audit-gated (sq-9hrn / sq-qhy4). See the crate `README.md` and
//! `research/mpc-untrusted-planner-routing-design.md`.
//!
//! [OPUS-4.8] sq-2q1x / sq-fix4 / sq-i1wh2 / sq-pwr.2. `[OPUS-5]` sq-1fo4.

use std::collections::BTreeSet;

use sparq_mpc::backend::OperatorClass;
use sparq_mpc::pipeline::OperatorRouting;

/// Which seam phase a [`SeamError`] came from. Naming the phase in the error value (not just a
/// doc-comment) keeps the failure auditable at the call site — the same honesty discipline as
/// `sparq-mpc`'s `MpcError::NotYetImplemented`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamPhase {
    /// Phase 2 — privacy/authorisation-aware source selection
    /// ([`select_private_sources`](crate::select_private_sources)).
    SourceSelection,
    /// Phase 3 — the disclosed/hidden routing partition
    /// ([`route_operators`](crate::route_operators)).
    Routing,
    /// Phase 4 — the leakage-envelope assembly + dual ratification
    /// ([`assemble_leakage_envelope`](crate::assemble_leakage_envelope)).
    LeakageEnvelope,
    /// Phase 6 — the result-aware source-combination prune
    /// ([`prune_source_combinations`](crate::prune_source_combinations)).
    SourceCombination,
    /// Phase 5 — the untrusted-plan binding + soundness re-validation
    /// ([`commit_plan`](crate::commit_plan) / [`revalidate_plan`](crate::revalidate_plan) /
    /// the still-deferred [`bind_plan_to_proof`](crate::bind_plan_to_proof)). `[OPUS-5]` sq-1fo4.
    PlanBinding,
}

impl SeamPhase {
    /// A short label for the phase (for the error `Display`).
    pub fn label(self) -> &'static str {
        match self {
            SeamPhase::SourceSelection => "privacy-aware source selection (Phase 2)",
            SeamPhase::Routing => "disclosed/hidden routing partition (Phase 3)",
            SeamPhase::LeakageEnvelope => "leakage-envelope assembly + ratification (Phase 4)",
            SeamPhase::SourceCombination => "result-aware source-combination prune (Phase 6)",
            SeamPhase::PlanBinding => "untrusted-plan binding + soundness re-validation (Phase 5)",
        }
    }
}

/// The seam's error type. [`SeamError::Deferred`] is the honest typed channel a *deferred* phase
/// stub returns (it carries the [`SeamPhase`] and the `gated_on` issues/audits, so a caller learns
/// WHY the phase is unavailable rather than hitting a panic or a fabricated result). It is
/// returned today by [`bind_plan_to_proof`](crate::bind_plan_to_proof) — Phase 5's soundness half,
/// which stays gated on the collaborative proof plus the sq-qhy4 / sq-9hrn audits.
/// [`SeamError::DescriptorMismatch`] is the *real* error variant
/// — the implemented source-selection adapter (Phase 2), the routing pass (Phase 3), and the
/// envelope assembly (Phase 4) all return it on an ambiguous / inconsistent privacy declaration or
/// a routing/operator mismatch. (The enum is `#[non_exhaustive]` so later phases can add variants
/// without a breaking change.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SeamError {
    /// A deferred seam phase was invoked. `phase` names which one; `gated_on` names the
    /// milestone(s) / audit(s) that must land first. This is NOT a fabricated answer and NOT
    /// a crash — it is an honest "not yet built" the caller can match on.
    Deferred {
        /// Which deferred phase was invoked.
        phase: SeamPhase,
        /// The gate(s) the phase waits on (issues / audits), as a human-readable string.
        gated_on: &'static str,
    },
    /// A privacy descriptor set was inconsistent — e.g. two
    /// [`crate::SourcePrivacyDescriptor`]s declared for the same source id (an ambiguous
    /// authorisation policy). The adapter **refuses** (fail-closed) rather than guessing which
    /// one binds. Returned by [`select_private_sources`](crate::select_private_sources).
    DescriptorMismatch {
        /// Which phase raised it ([`SeamPhase::SourceSelection`] for Phase 2,
        /// [`SeamPhase::Routing`] for Phase 3, [`SeamPhase::LeakageEnvelope`] for Phase 4,
        /// [`SeamPhase::SourceCombination`] for Phase 6).
        phase: SeamPhase,
        /// The offending source id.
        source_id: String,
        /// A short human-readable explanation of the inconsistency.
        detail: &'static str,
    },
}

impl std::fmt::Display for SeamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeamError::Deferred { phase, gated_on } => write!(
                f,
                "sparq-fedplan-mpc seam phase deferred: {} (gated on {})",
                phase.label(),
                gated_on
            ),
            SeamError::DescriptorMismatch {
                phase,
                source_id,
                detail,
            } => write!(
                f,
                "sparq-fedplan-mpc {} privacy-descriptor mismatch for source {}: {}",
                phase.label(),
                source_id,
                detail
            ),
        }
    }
}

impl std::error::Error for SeamError {}

/// The output of the **Phase-3 routing pass** ([`route_operators`](crate::route_operators)): the
/// per-operator disclosed-vs-hidden routing the `sparq-mpc` pipeline consumes. It wraps the real
/// [`sparq_mpc::pipeline::OperatorRouting`] vector so the output is the genuine seam shape — the
/// structure the pipeline today receives hand-written. The routing decision is computed by the
/// pass (default-deny, policy-parameterised); see [`crate::routing`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PrivateRouting {
    /// The per-operator routing the `sparq-mpc` pipeline accepts, in input-operator order.
    pub routing: Vec<OperatorRouting>,
}

/// What ONE operator of the plan reveals — the unit the [`LeakageEnvelope`] is built from
/// (assembled by [`assemble_leakage_envelope`](crate::assemble_leakage_envelope)).
///
/// Honest by construction: it records the operator's label and [`OperatorClass`] (the query shape
/// always leaks), whether it was routed disclosed (`disclosed` — recomputed in the clear), and —
/// for a disclosed operator — the predicate ids of the operands it reveals in the clear
/// (`disclosed_operands`). A hidden operator discloses **no** operand (it runs under MPC), so its
/// `disclosed_operands` is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperatorDisclosure {
    /// The operator's human label (verbatim from the [`OperatorRouting`] / `QueryOperator`).
    pub operator: String,
    /// The [`OperatorClass`] this operator runs as. Part of the *operator structure* that always
    /// leaks — disclosed regardless of the route.
    pub class: OperatorClass,
    /// Whether this operator was routed disclosed (recomputed in the clear). `false` ⇒ hidden
    /// (under MPC), disclosing no operand.
    pub disclosed: bool,
    /// For a **disclosed** operator, the predicate ids of the operands revealed in the clear, in
    /// deterministic ascending order (deduplicated). Empty for a hidden operator. The honest
    /// enumeration of what the disclose route exposes.
    pub disclosed_operands: Vec<String>,
}

/// The **leakage envelope** — the declared, explicit statement of what the Phase-3 routing+disclosure
/// plan reveals (design record §2.1(3): the "declared bounded leak"). Assembled by
/// [`assemble_leakage_envelope`](crate::assemble_leakage_envelope) from the Phase-3
/// [`PrivateRouting`], the operators it was derived from, and the Phase-2 source selection. It is a
/// **declaration**, not a cryptographic enforcement: it enumerates — honestly over-counting, never
/// under-counting — the operator structure, the disclose/hide partition, the disclosed operands,
/// and which sources participate. It carries **no** secret material; the predicate ids and labels
/// in it are the (public) plan metadata the holders and verifier ratify before any execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LeakageEnvelope {
    /// Per-operator disclosure, in input-operator order. The full operator structure (count,
    /// per-operator class + label) and the disclose/hide partition are read off here.
    pub operators: Vec<OperatorDisclosure>,
    /// The distinct source ids that contribute to the plan after the Phase-2 prune, ascending.
    /// *That a source participates at all* is itself disclosed by the plan, so it is declared here.
    pub participating_sources: Vec<String>,
}

impl LeakageEnvelope {
    /// The distinct predicate ids disclosed in the clear across the **whole** plan, ascending.
    /// This is the honest "how much is revealed in the clear" set — the metric the verifier's
    /// acceptance budget bounds in [`ratify_envelope`](crate::ratify_envelope). The operator
    /// structure (which always leaks) is *not* counted here; this is the *operand* disclosure the
    /// plan chose.
    pub fn distinct_disclosed_operands(&self) -> Vec<String> {
        self.operators
            .iter()
            .filter(|op| op.disclosed)
            .flat_map(|op| op.disclosed_operands.iter().cloned())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect()
    }

    /// The indices of the operators routed disclosed (recomputed in the clear), ascending — the
    /// disclose half of the partition, surfaced for an auditor.
    pub fn disclosed_operator_indices(&self) -> Vec<usize> {
        self.operators
            .iter()
            .enumerate()
            .filter(|(_, op)| op.disclosed)
            .map(|(i, _)| i)
            .collect()
    }
}

/// The outcome of the dual-ratification gate ([`ratify_envelope`](crate::ratify_envelope)). It
/// enumerates, honestly, whether the plan was cleared for execution and — when not — *which*
/// ratification failed and *why*, so an over-disclosing or over-leaking plan is reported, never
/// silently dropped. `#[non_exhaustive]` so a later phase can add an outcome without a breaking
/// change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RatificationOutcome {
    /// Both ratifications passed — every holder accepted and the verifier's acceptance policy
    /// cleared the envelope. The plan is cleared for execution.
    Ratified,
    /// A **holder** fail-closed-rejected the plan (constraint C-B): it discloses a predicate the
    /// holder marked private (or the holder presented an ambiguous duplicate policy). The plan is
    /// NOT honoured. The data owner's veto is primary.
    HolderRejected {
        /// The holder (source id) that rejected the plan.
        source_id: String,
        /// The predicate the plan tried to disclose that this holder marks private (empty for an
        /// ambiguous-duplicate-policy rejection).
        predicate: String,
        /// A short human-readable explanation.
        detail: &'static str,
    },
    /// The **verifier** rejected an over-leaking envelope: it discloses more operands than the
    /// verifier's declared acceptance budget.
    VerifierRejected {
        /// The count of distinct disclosed operands the envelope declares.
        disclosed: usize,
        /// The verifier's declared budget that was exceeded.
        budget: usize,
        /// A short human-readable explanation.
        detail: &'static str,
    },
}

impl RatificationOutcome {
    /// Whether the plan was cleared for execution (both ratifications passed).
    pub fn is_ratified(&self) -> bool {
        matches!(self, RatificationOutcome::Ratified)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Phase 2 (`select_private_sources`) is IMPLEMENTED — its tests live in `crate::selection`.
    // Phase 3 (`route_operators`) is IMPLEMENTED — its tests live in `crate::routing`.
    // Phase 4 (`assemble_leakage_envelope` + `ratify_envelope`) is IMPLEMENTED — its tests live in
    // `crate::envelope`. The tests below pin the shared seam types this module defines.

    fn disclosure(op: &str, disclosed: bool, operands: &[&str]) -> OperatorDisclosure {
        OperatorDisclosure {
            operator: op.to_string(),
            class: OperatorClass::EqualityJoin,
            disclosed,
            disclosed_operands: operands.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn envelope_distinct_disclosed_operands_dedups_and_ignores_hidden() {
        // A disclosed op revealing p1+p2, another disclosed revealing p2 (dup), and a HIDDEN op
        // naming p3 (which must NOT count — a hidden op discloses nothing).
        let env = LeakageEnvelope {
            operators: vec![
                disclosure("d1", true, &["http://ex/p1", "http://ex/p2"]),
                disclosure("d2", true, &["http://ex/p2"]),
                disclosure("h1", false, &["http://ex/p3"]),
            ],
            participating_sources: vec!["http://a/".to_string()],
        };
        // p1 + p2 distinct; p2 deduped; p3 excluded (hidden).
        assert_eq!(
            env.distinct_disclosed_operands(),
            vec!["http://ex/p1".to_string(), "http://ex/p2".to_string()]
        );
        // The disclose half of the partition: operators 0 and 1.
        assert_eq!(env.disclosed_operator_indices(), vec![0, 1]);
    }

    #[test]
    fn ratification_outcome_is_ratified_predicate() {
        assert!(RatificationOutcome::Ratified.is_ratified());
        assert!(!RatificationOutcome::HolderRejected {
            source_id: "http://a/".to_string(),
            predicate: "http://ex/p".to_string(),
            detail: "x",
        }
        .is_ratified());
        assert!(!RatificationOutcome::VerifierRejected {
            disclosed: 3,
            budget: 1,
            detail: "x",
        }
        .is_ratified());
    }

    #[test]
    fn deferred_error_still_displays_its_phase_and_gate() {
        // The Deferred variant is retained for a future gated phase (Phase 5); its Display still
        // renders the phase + gate honestly.
        let err = SeamError::Deferred {
            phase: SeamPhase::LeakageEnvelope,
            gated_on: "sq-qhy4 / sq-9hrn",
        };
        let shown = format!("{}", err);
        assert!(shown.contains("deferred"));
        assert!(shown.contains("sq-qhy4"));
    }

    #[test]
    fn phase_labels_are_distinct() {
        let labels = [
            SeamPhase::SourceSelection.label(),
            SeamPhase::Routing.label(),
            SeamPhase::LeakageEnvelope.label(),
            SeamPhase::SourceCombination.label(),
        ];
        // All seam phases carry a distinct human label.
        assert_eq!(
            labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
    }

    #[test]
    fn descriptor_mismatch_displays_the_offending_source() {
        // The new Phase-2 error variant renders the source id + phase in its Display.
        let err = SeamError::DescriptorMismatch {
            phase: SeamPhase::SourceSelection,
            source_id: "http://dup/".to_string(),
            detail: "duplicate descriptor",
        };
        let shown = format!("{}", err);
        assert!(shown.contains("http://dup/"));
        assert!(shown.contains("source selection"));
    }
}
