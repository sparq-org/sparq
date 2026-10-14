//! Acceptance tests for the admission-decision equivalence comparator (bead `sq-yxkp7`).
//!
//! Structural set-equivalence + fail-closed-asymmetry checks only — NOT a cryptographic
//! soundness proof of any policy engine.

use std::collections::BTreeSet;

use sparq_difftest::{decision_diff, fail_closed_asymmetry, Baseline, Candidate, DecisionOutcome};

type Tuple = (&'static str, &'static str, &'static str);

fn t(principal: &'static str, mode: &'static str, target: &'static str) -> Tuple {
    (principal, mode, target)
}

/// Acceptance (1): identical decisions ⇒ empty `DecisionDiff` (`same == true`, all four diff
/// sets empty) and no fail-closed asymmetry.
#[test]
fn identical_decisions_yield_empty_diff() {
    let a = DecisionOutcome::new(
        [t("alice", "Read", "/doc"), t("carol", "Write", "/doc")],
        [t("bob", "Write", "/doc")],
    );
    let b = a.clone();

    let d = decision_diff(Baseline::new(&a), Candidate::new(&b));
    assert!(d.same);
    assert!(d.added_allow.is_empty());
    assert!(d.dropped_allow.is_empty());
    assert!(d.added_deny.is_empty());
    assert!(d.dropped_deny.is_empty());
    assert!(fail_closed_asymmetry(Baseline::new(&a), Candidate::new(&b)).is_empty());
}

/// Acceptance (2): an added allow on side B — whether A explicitly denied the tuple or merely
/// omitted it — makes `fail_closed_asymmetry` return exactly the escalating tuples.
#[test]
fn added_allow_on_b_is_flagged_as_escalation() {
    let a = DecisionOutcome::new(
        [t("alice", "Read", "/doc")],
        [t("bob", "Write", "/doc")], // A explicitly denies bob
    );
    let b = DecisionOutcome::new(
        [
            t("alice", "Read", "/doc"),    // unchanged: not an escalation
            t("bob", "Write", "/doc"),     // A-deny → B-allow: escalation
            t("mallory", "Read", "/priv"), // A-absent → B-allow: escalation (fail-closed: absence = deny)
        ],
        [],
    );

    let esc = fail_closed_asymmetry(Baseline::new(&a), Candidate::new(&b));
    assert_eq!(
        esc,
        BTreeSet::from([t("bob", "Write", "/doc"), t("mallory", "Read", "/priv")])
    );
    // The full diff agrees: the escalation direction IS added_allow, and same == false.
    let d = decision_diff(Baseline::new(&a), Candidate::new(&b));
    assert!(!d.same);
    assert_eq!(d.added_allow, esc);
}

/// Acceptance (2) control — NON-VACUITY witness: when side B only *drops* an allow (the
/// fail-closed-SAFE direction, B stricter than A), the asymmetry is empty even though the
/// decisions differ. This proves the escalation test above isn't passing merely because the
/// helper flags any difference.
#[test]
fn dropped_allow_only_yields_empty_asymmetry() {
    let a = DecisionOutcome::new(
        [t("alice", "Read", "/doc"), t("carol", "Write", "/doc")],
        [t("bob", "Write", "/doc")],
    );
    let b = DecisionOutcome::new(
        [t("alice", "Read", "/doc")], // carol's allow dropped: stricter, not an escalation
        [t("bob", "Write", "/doc")],
    );

    assert!(fail_closed_asymmetry(Baseline::new(&a), Candidate::new(&b)).is_empty());
    let d = decision_diff(Baseline::new(&a), Candidate::new(&b));
    assert!(!d.same); // the decisions DO differ…
    assert_eq!(
        d.dropped_allow,
        BTreeSet::from([t("carol", "Write", "/doc")])
    );
    assert!(d.added_allow.is_empty()); // …but not in the escalation direction
}

/// Acceptance (3): added/dropped denies are classified correctly (and a pure deny-side change
/// is not an escalation).
#[test]
fn deny_changes_classified_correctly() {
    let a = DecisionOutcome::new(
        [t("alice", "Read", "/doc")],
        [t("bob", "Write", "/doc"), t("dave", "Append", "/doc")],
    );
    let b = DecisionOutcome::new(
        [t("alice", "Read", "/doc")],
        [t("bob", "Write", "/doc"), t("erin", "Control", "/doc")],
    );

    let d = decision_diff(Baseline::new(&a), Candidate::new(&b));
    assert!(!d.same);
    assert_eq!(d.added_deny, BTreeSet::from([t("erin", "Control", "/doc")]));
    assert_eq!(
        d.dropped_deny,
        BTreeSet::from([t("dave", "Append", "/doc")])
    );
    assert!(d.added_allow.is_empty());
    assert!(d.dropped_allow.is_empty());
    assert!(fail_closed_asymmetry(Baseline::new(&a), Candidate::new(&b)).is_empty());
}
