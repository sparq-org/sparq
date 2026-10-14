//! END-TO-END test that the sparq-mpc per-protocol annotation graph is discharged by
//! the **SHARED** N3 admissibility reasoner — the whole point of issues #3275 and
//! #6186.
//!
//! Before this, a protocol's trust model was recorded (at best) as an
//! `secx:assumption secx:SemiHonest` triple, and `sparq_trust::admissibility` keys
//! strictly on `(dimension, level)` through `secx:overDimension` + `secx:atLeast` —
//! so no ODRL preference could exclude a semi-honest protocol. The load-bearing
//! invariants pinned here, all computed by `sparq_trust::admissible` over this
//! crate's `secprop::annotations_n3()` (no crate-local exclusion predicate):
//!
//! 1. `requiresAdversaryModel gteq secx:Malicious` **excludes every** sparq-mpc
//!    protocol;
//! 2. `requiresCorruptionThreshold gteq secx:DishonestMajority` excludes every one
//!    of them too;
//! 3. the **relaxed** preference (`gteq secx:SemiHonest` / `gteq
//!    secx:HonestMajority`) **admits** them — so the gate is demonstrably not
//!    vacuously empty;
//! 4. the strict preference **admits** the illustrative malicious /
//!    dishonest-majority subject — so the exclusions in (1)/(2) are the ORDER
//!    talking, not a missing annotation.
//!
//! Runs only with BOTH `secprop-annotations` (this crate's graph) and the
//! `sparq-trust/secprop-admissibility` reasoner the dev-dependency pins on.
#![cfg(feature = "secprop-annotations")]

use sparq_mpc::secprop::{
    annotations_n3, ANNOTATED_PROTOCOLS, ILLUSTRATIVE_SPDZ_DISHONEST_MAJORITY,
};
use sparq_trust::admissible;

/// A preference requiring MALICIOUS security. `mpc:cAdv` / `mpc:cThr` are the
/// constraint IRIs; `secx:requiresAdversaryModel` /
/// `secx:requiresCorruptionThreshold` are the leftOperands the shared ruleset maps
/// to the two dimensions.
const STRICT_POLICY: &str = "\
<https://sparq.dev/ns/mpc#cAdv> odrl:leftOperand secx:requiresAdversaryModel ; odrl:operator odrl:gteq ; odrl:rightOperand secx:Malicious .\n\
<https://sparq.dev/ns/mpc#cThr> odrl:leftOperand secx:requiresCorruptionThreshold ; odrl:operator odrl:gteq ; odrl:rightOperand secx:DishonestMajority .\n";

const C_ADV: &str = "https://sparq.dev/ns/mpc#cAdv";
const C_THR: &str = "https://sparq.dev/ns/mpc#cThr";

/// The RELAXED preference: exactly the posture sparq-mpc actually provides.
const RELAXED_POLICY: &str = "\
<https://sparq.dev/ns/mpc#rAdv> odrl:leftOperand secx:requiresAdversaryModel ; odrl:operator odrl:gteq ; odrl:rightOperand secx:SemiHonest .\n\
<https://sparq.dev/ns/mpc#rThr> odrl:leftOperand secx:requiresCorruptionThreshold ; odrl:operator odrl:gteq ; odrl:rightOperand secx:HonestMajority .\n";

const R_ADV: &str = "https://sparq.dev/ns/mpc#rAdv";
const R_THR: &str = "https://sparq.dev/ns/mpc#rThr";

/// INVARIANT 1 + 2: a preference requiring malicious security excludes EVERY
/// sparq-mpc protocol, and so does one requiring a dishonest-majority threshold.
/// This is the exclusion #3275 asked for, computed by the shared reasoner.
#[test]
fn requiring_malicious_security_excludes_every_mpc_protocol() {
    for &m in ANNOTATED_PROTOCOLS {
        let a = admissible(m, &[C_ADV], STRICT_POLICY, annotations_n3()).expect("reasoning failed");
        assert!(
            !a.admissible,
            "{} must be INADMISSIBLE under `requiresAdversaryModel gteq Malicious` \
             (it is semi-honest only); got admissible",
            m
        );
        assert_eq!(a.unsatisfied, vec![C_ADV.to_string()]);

        let a = admissible(m, &[C_THR], STRICT_POLICY, annotations_n3()).expect("reasoning failed");
        assert!(
            !a.admissible,
            "{} must be INADMISSIBLE under `requiresCorruptionThreshold gteq \
             DishonestMajority` (it is honest-majority only); got admissible",
            m
        );
        assert_eq!(a.unsatisfied, vec![C_THR.to_string()]);
    }
}

/// INVARIANT 3: the relaxed preference ADMITS every protocol — the gate is not
/// vacuously empty, and the annotations really are reaching the discharge rule.
#[test]
fn the_relaxed_preference_admits_every_mpc_protocol() {
    for &m in ANNOTATED_PROTOCOLS {
        let a = admissible(m, &[R_ADV, R_THR], RELAXED_POLICY, annotations_n3())
            .expect("reasoning failed");
        assert!(
            a.admissible,
            "{} MUST be admissible under `gteq SemiHonest` / `gteq HonestMajority` — \
             exactly the posture it is annotated with; unsatisfied={:?}",
            m, a.unsatisfied
        );
    }
}

/// INVARIANT 4: the STRICT preference admits the illustrative malicious /
/// dishonest-majority subject. The positive control — without it, the exclusions
/// above would also pass if the annotation graph were empty or the dimensions
/// unregistered.
#[test]
fn the_strict_preference_admits_the_illustrative_malicious_subject() {
    let a = admissible(
        ILLUSTRATIVE_SPDZ_DISHONEST_MAJORITY,
        &[C_ADV, C_THR],
        STRICT_POLICY,
        annotations_n3(),
    )
    .expect("reasoning failed");
    assert!(
        a.admissible,
        "the illustrative Malicious / DishonestMajority subject MUST satisfy the \
         strict preference — otherwise the exclusions above prove nothing about the \
         ORDER; unsatisfied={:?}",
        a.unsatisfied
    );
}

/// A sparq-mpc protocol is excluded by the strict preference on BOTH constraints at
/// once (default-deny over the whole preference), with both named in the honest
/// "why not admissible" detail.
#[test]
fn both_strict_constraints_are_reported_unsatisfied() {
    let a = admissible(
        ANNOTATED_PROTOCOLS[0],
        &[C_ADV, C_THR],
        STRICT_POLICY,
        annotations_n3(),
    )
    .expect("reasoning failed");
    assert!(!a.admissible);
    let mut got = a.unsatisfied.clone();
    got.sort();
    let mut want = vec![C_ADV.to_string(), C_THR.to_string()];
    want.sort();
    assert_eq!(
        got, want,
        "both the adversary-model and the corruption-threshold constraint must be \
         reported unsatisfied"
    );
}
