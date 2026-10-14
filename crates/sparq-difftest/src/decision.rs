//! Admission-decision equivalence comparator with a **fail-closed asymmetry** check.
//!
//! Every trust-graph / WAC / ODRL differential test so far has hand-rolled its own allow/deny
//! set comparison in-crate. This module is the shared, engine-independent comparator: a
//! decision outcome is expressed as two plain sets — an *allow* set and a *deny* set of
//! decision tuples (canonically `(principal, mode, target)`, but the comparator is generic
//! over any `Ord` tuple type, so callers keep their own identifier types and this crate
//! stays free of any dependency on `sparq-trust` / `sparq-solid`).
//!
//! Two things are computed:
//!
//! * [`decision_diff`] — the full structural diff: allows/denies added or dropped by the
//!   [`Candidate`] relative to the [`Baseline`], and whether the two outcomes are the *same*
//!   decision.
//! * [`fail_closed_asymmetry`] — the **privilege-escalation direction** only: every tuple side
//!   the candidate **allows** that the baseline did **not** allow (the baseline either denied
//!   it or omitted it entirely — under a fail-closed policy, absence is a deny, so both are
//!   escalations). By construction this is exactly `candidate.allow \ baseline.allow`, so it
//!   has zero false negatives on that direction: a tuple escapes the report iff the baseline
//!   already allowed it. Distinct [`Baseline`] / [`Candidate`] argument types make an
//!   accidental positional swap a compile-time error.
//!
//! # Honesty
//!
//! This is a **structural set-equivalence + fail-closed-asymmetry check**, NOT a cryptographic
//! soundness proof and not a semantic model of any policy language. It compares two already
//! *evaluated* outcomes; it cannot tell which side is *correct*, and it proves nothing about
//! the policy engines that produced them (production ZK/security soundness claims remain
//! gated on the external audit, `sq-qhy4`).

use std::collections::BTreeSet;

/// One evaluated admission-decision outcome: the set of admitted (`allow`) and refused
/// (`deny`) decision tuples.
///
/// `T` is the caller's decision-tuple type — canonically `(principal, mode, target)` — and
/// only needs `Ord` (for set membership). A tuple in *neither* set is an **omission**; under a
/// fail-closed reading an omission is a deny, which is why [`fail_closed_asymmetry`] treats
/// deny and absence identically. The comparator does not police `allow ∩ deny` overlaps (a
/// contradictory outcome is the producer's bug); overlapping tuples are compared per-set,
/// structurally.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecisionOutcome<T: Ord> {
    /// Tuples this decision admits.
    pub allow: BTreeSet<T>,
    /// Tuples this decision explicitly refuses.
    pub deny: BTreeSet<T>,
}

impl<T: Ord> DecisionOutcome<T> {
    /// Build an outcome from any two tuple iterators (duplicates collapse — these are sets).
    pub fn new<A, D>(allow: A, deny: D) -> Self
    where
        A: IntoIterator<Item = T>,
        D: IntoIterator<Item = T>,
    {
        Self {
            allow: allow.into_iter().collect(),
            deny: deny.into_iter().collect(),
        }
    }
}

/// The trusted reference outcome in a directional decision comparison.
///
/// This is intentionally a distinct type from [`Candidate`]: callers must name which outcome
/// defines the no-escalation boundary, so swapping the two arguments cannot compile.
#[derive(Debug, Clone, Copy)]
pub struct Baseline<'a, T: Ord>(&'a DecisionOutcome<T>);

impl<'a, T: Ord> Baseline<'a, T> {
    /// Mark `outcome` as the trusted baseline for a directional comparison.
    pub const fn new(outcome: &'a DecisionOutcome<T>) -> Self {
        Self(outcome)
    }
}

/// The outcome being checked against a trusted [`Baseline`].
///
/// This is intentionally a distinct type from [`Baseline`]: callers must name the outcome
/// whose newly admitted tuples would be privilege escalations.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a, T: Ord>(&'a DecisionOutcome<T>);

impl<'a, T: Ord> Candidate<'a, T> {
    /// Mark `outcome` as the candidate in a directional comparison.
    pub const fn new(outcome: &'a DecisionOutcome<T>) -> Self {
        Self(outcome)
    }
}

/// The structural diff between two decision outcomes (candidate relative to baseline).
///
/// `same` is `true` iff **all four** diff sets are empty — i.e. the allow sets are equal *and*
/// the deny sets are equal. The escalation direction (`added_allow`) is exactly what
/// [`fail_closed_asymmetry`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionDiff<T: Ord> {
    /// `true` iff the two outcomes are identical (all diff sets empty).
    pub same: bool,
    /// Tuples B allows that A did not allow — the privilege-escalation direction.
    pub added_allow: BTreeSet<T>,
    /// Tuples A allowed that B does not allow (the fail-closed-*safe* direction: B got stricter).
    pub dropped_allow: BTreeSet<T>,
    /// Tuples B explicitly denies that A did not explicitly deny.
    pub added_deny: BTreeSet<T>,
    /// Tuples A explicitly denied that B does not explicitly deny.
    pub dropped_deny: BTreeSet<T>,
}

/// Compute the full structural [`DecisionDiff`] of `candidate` relative to `baseline`.
///
/// Pure set arithmetic: `added_allow = candidate.allow \ baseline.allow`,
/// `dropped_allow = baseline.allow \ candidate.allow`, and likewise for the deny sets. `same`
/// iff every diff set is empty. The distinct argument types prevent a positional swap from
/// silently reversing every directional field.
pub fn decision_diff<T: Ord + Clone>(
    baseline: Baseline<'_, T>,
    candidate: Candidate<'_, T>,
) -> DecisionDiff<T> {
    let baseline = baseline.0;
    let candidate = candidate.0;
    let added_allow: BTreeSet<T> = candidate
        .allow
        .difference(&baseline.allow)
        .cloned()
        .collect();
    let dropped_allow: BTreeSet<T> = baseline
        .allow
        .difference(&candidate.allow)
        .cloned()
        .collect();
    let added_deny: BTreeSet<T> = candidate.deny.difference(&baseline.deny).cloned().collect();
    let dropped_deny: BTreeSet<T> = baseline.deny.difference(&candidate.deny).cloned().collect();
    let same = added_allow.is_empty()
        && dropped_allow.is_empty()
        && added_deny.is_empty()
        && dropped_deny.is_empty();
    DecisionDiff {
        same,
        added_allow,
        dropped_allow,
        added_deny,
        dropped_deny,
    }
}

/// Report every tuple `candidate` **allows** that `baseline` did **not** allow — the
/// privilege-escalation (fail-closed-violation) direction.
///
/// A tuple lands in the result whether the baseline explicitly *denied* it or merely *omitted*
/// it: under a fail-closed policy both mean "not admitted", so a candidate allow is an
/// escalation either way. Equivalently this is `candidate.allow \ baseline.allow`, so the check
/// has **zero false negatives** in this direction: the only tuples excluded are those the
/// baseline itself already allowed. An empty result means the candidate admits nothing beyond
/// the baseline (the candidate may still be *stricter* — dropped allows are the safe direction
/// and are reported by [`decision_diff`], not here).
///
/// The direction is part of the argument types. A swapped call does not compile:
///
/// ```compile_fail
/// use sparq_difftest::{
///     fail_closed_asymmetry, Baseline, Candidate, DecisionOutcome,
/// };
///
/// let baseline = DecisionOutcome::new([1_u8], []);
/// let candidate = DecisionOutcome::new([1_u8, 2], []);
/// fail_closed_asymmetry(Candidate::new(&candidate), Baseline::new(&baseline));
/// ```
pub fn fail_closed_asymmetry<T: Ord + Clone>(
    baseline: Baseline<'_, T>,
    candidate: Candidate<'_, T>,
) -> BTreeSet<T> {
    candidate
        .0
        .allow
        .difference(&baseline.0.allow)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Tuple = (&'static str, &'static str, &'static str);

    fn t(p: &'static str, m: &'static str, tgt: &'static str) -> Tuple {
        (p, m, tgt)
    }

    // Direct unit test: DecisionOutcome::new collects both iterators into sets (dups collapse).
    #[test]
    fn outcome_new_collects_and_dedups() {
        let o = DecisionOutcome::new(
            [t("alice", "Read", "/a"), t("alice", "Read", "/a")],
            [t("bob", "Write", "/a")],
        );
        assert_eq!(o.allow.len(), 1);
        assert!(o.allow.contains(&t("alice", "Read", "/a")));
        assert!(o.deny.contains(&t("bob", "Write", "/a")));
    }

    // Direct unit test: decision_diff classifies each of the four directions independently.
    #[test]
    fn diff_classifies_all_four_directions() {
        let a = DecisionOutcome::new(
            [t("alice", "Read", "/a"), t("carol", "Read", "/c")],
            [t("bob", "Write", "/a"), t("dave", "Write", "/d")],
        );
        let b = DecisionOutcome::new(
            [t("alice", "Read", "/a"), t("eve", "Read", "/e")],
            [t("bob", "Write", "/a"), t("frank", "Write", "/f")],
        );
        let d = decision_diff(Baseline::new(&a), Candidate::new(&b));
        assert!(!d.same);
        assert_eq!(d.added_allow, BTreeSet::from([t("eve", "Read", "/e")]));
        assert_eq!(d.dropped_allow, BTreeSet::from([t("carol", "Read", "/c")]));
        assert_eq!(d.added_deny, BTreeSet::from([t("frank", "Write", "/f")]));
        assert_eq!(d.dropped_deny, BTreeSet::from([t("dave", "Write", "/d")]));
    }

    // Direct unit test: fail_closed_asymmetry flags a deny→allow flip AND an absence→allow
    // appearance, and excludes tuples A already allowed.
    #[test]
    fn asymmetry_flags_deny_flip_and_absence_flip_only() {
        let a = DecisionOutcome::new(
            [t("alice", "Read", "/a")],
            [t("bob", "Write", "/a")], // explicitly denied by A
        );
        let b = DecisionOutcome::new(
            [
                t("alice", "Read", "/a"),   // A already allowed: NOT an escalation
                t("bob", "Write", "/a"),    // A denied → B allows: escalation
                t("mallory", "Read", "/s"), // absent on A → B allows: escalation
            ],
            [],
        );
        let esc = fail_closed_asymmetry(Baseline::new(&a), Candidate::new(&b));
        assert_eq!(
            esc,
            BTreeSet::from([t("bob", "Write", "/a"), t("mallory", "Read", "/s")])
        );
    }
}
