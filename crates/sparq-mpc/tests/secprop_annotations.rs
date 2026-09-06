//! Phase 6 (sq-dz10l, epic sq-0dksu) — the sparq-mpc per-protocol security-properties
//! annotation graph, asserted against the four over-claim guards and against the
//! crate's own [`SecurityDescriptor`].
//!
//! Design record: `research/security-properties-ontology-design.md` §5c — annotate the
//! MPC protocols with the `secx:HonestMajority` / `secx:SemiHonest` assumptions so a
//! preference requiring malicious-security / dishonest-majority mechanically excludes
//! them, rather than the exclusion living only in README prose.
//!
//! [OPUS-5] 🤖 SPARQ agent — security-properties ontology.

#![cfg(feature = "secprop-annotations")]

use sparq_mpc::backend::{AdversaryModel, OperatorClass};
use sparq_mpc::secprop::{
    assumption_completeness_violations, assurance_overclaim_violations, completeness_violations,
    descriptor_drift_violations, excluded_by_requiring_dishonest_majority,
    excluded_by_requiring_malicious_security, parse_annotations, Assurance, MethodAnnotations,
    PropertyAssertion, ViolationKind, ANNOTATED_METHODS, MPC_AUTH_COMPARISON_V1,
    MPC_COMPARISON_V1, MPC_EQUALITY_JOIN_V1, MPC_LINEAR_AGGREGATE_V1,
    MPC_TAMPER_EVIDENT_DISCLOSE_V1, OPERATOR_CLASS_METHODS,
};
use sparq_mpc::shamir::ShamirBackend;
use sparq_secprop_vocab::{
    ALL_SECPROP_IRIS, SECX_CLAIMED, SECX_EXTERNAL_SIGN_OFF_PENDING, SECX_HONEST_MAJORITY,
    SECX_NOT_ZK, SECX_SEMI_HONEST,
};
use std::collections::{BTreeMap, BTreeSet};

/// A representative honest-majority backend: `n = 5`, so `t = 2` and the degree-`2t`
/// equality open sits at the minimal `n = 2t+1` (zero RS redundancy — the Hole 1 case).
fn backend() -> ShamirBackend {
    ShamirBackend::new(5).expect("n = 5 is a valid honest-majority party count")
}

#[test]
fn methods_ttl_parses_and_annotates_every_declared_protocol() {
    let annotations = parse_annotations();
    assert_eq!(
        annotations.len(),
        ANNOTATED_METHODS.len(),
        "every declared protocol (and nothing else) must carry an annotation block; got {:?}",
        annotations.keys().collect::<Vec<_>>()
    );
    for m in ANNOTATED_METHODS {
        let block = annotations
            .get(*m)
            .unwrap_or_else(|| panic!("{m} has no annotation block"));
        assert!(
            !block.assertions.is_empty(),
            "{m} has an empty annotation block"
        );
    }
}

#[test]
fn shipped_graph_satisfies_all_four_guards() {
    let annotations = parse_annotations();
    let backend = backend();

    let mut violations = Vec::new();
    violations.extend(assurance_overclaim_violations(&annotations));
    violations.extend(assumption_completeness_violations(&annotations));
    violations.extend(completeness_violations(&annotations));
    violations.extend(descriptor_drift_violations(&annotations, &backend));

    assert!(
        violations.is_empty(),
        "shipped annotation graph violates an over-claim guard: {violations:#?}"
    );
}

/// Guard 1, stated explicitly: while `sq-qhy4` (external accredited-cryptographer
/// sign-off) is open, only settled NEGATIVE facts may be `secx:Proven`. Every positive
/// property is `secx:Claimed` + `secx:ExternalSignOffPending`.
#[test]
fn only_the_negative_not_zk_rows_are_proven() {
    for (method, block) in parse_annotations() {
        for a in &block.assertions {
            if a.assurance == Assurance::Proven {
                assert_eq!(
                    a.level.as_deref(),
                    Some(SECX_NOT_ZK),
                    "{method}: only the settled-negative secx:NotZK row may be secx:Proven, \
                     but {} (level {:?}) is",
                    a.property,
                    a.level
                );
            } else {
                assert_eq!(
                    a.assurance,
                    Assurance::Claimed,
                    "{method}: positive property {} must be secx:Claimed",
                    a.property
                );
                assert_eq!(
                    a.audit_status.as_deref(),
                    Some(SECX_EXTERNAL_SIGN_OFF_PENDING),
                    "{method}: positive property {} must carry secx:ExternalSignOffPending \
                     while sq-qhy4 is open",
                    a.property
                );
            }
        }
    }
}

/// **The §5c headline, stated FAIL-CLOSED.** A preference requiring malicious security
/// must mechanically exclude every protocol whose guarantee is not established against
/// an actively deviating party — which today is the ENTIRE crate.
///
/// That includes [`MPC_AUTH_COMPARISON_V1`], the IT-MAC comparison, even though it is
/// the crate's strongest construction. `MacSession::auth_mul` carries the output MAC as
/// `[α·z] = reduce([α·x]·[y])`, so a value tamper on its SECOND operand is *adopted* —
/// the gate recomputes a MAC consistent with the tampered value and the batched
/// `mac_check` passes on a wrong product (pinned by the witness
/// `mac_check_adopts_a_second_operand_tamper_but_catches_a_first_operand_one` in
/// `crate::auth_disclose`). That is a property of the primitive, not of one call site,
/// and every gate of `malicious_greater_than` is an `auth_mul` — so an active deviation
/// has a known route to a wrong verdict WITHOUT an abort. Admitting it under a
/// malicious-security preference would hand a caller exactly the protocol its preference
/// exists to refuse; `secx:Claimed` + `secx:ExternalSignOffPending` records how much we
/// know about a claim and cannot repair a wrong adversary-model classification.
///
/// **Do not relax this test to re-admit the comparison on the strength of its
/// `auth_`/MAC surface.** It may be admitted only once (a) the second-operand adoption
/// is closed in `MacSession` (multiplication-gate verification — `mac_check` only ever
/// covers values it OPENS, so more output checks cannot close it), and (b) an
/// END-TO-END adversarial witness on the real `malicious_greater_than` pipeline shows an
/// ABORT for the relevant tamper classes — a test that is RED before the fix and green
/// after. Metadata-only admission is what this test exists to prevent.
#[test]
fn requiring_malicious_security_excludes_the_whole_crate_including_the_it_mac_comparison() {
    let annotations = parse_annotations();
    let excluded: BTreeSet<String> = excluded_by_requiring_malicious_security(&annotations)
        .into_iter()
        .collect();

    for m in [
        MPC_LINEAR_AGGREGATE_V1,
        MPC_EQUALITY_JOIN_V1,
        MPC_COMPARISON_V1,
        MPC_TAMPER_EVIDENT_DISCLOSE_V1,
    ] {
        assert!(
            excluded.contains(m),
            "{m} rests on secx:SemiHonest, so requiring malicious security must exclude it"
        );
    }

    assert!(
        excluded.contains(MPC_AUTH_COMPARISON_V1),
        "{MPC_AUTH_COMPARISON_V1} inherits auth_mul's ADOPTED second-operand tamper on \
         every gate of its chain, so a preference requiring malicious security must \
         exclude it too — admitting it would over-claim the adversary model"
    );

    let all: BTreeSet<String> = ANNOTATED_METHODS.iter().map(|m| (*m).to_owned()).collect();
    assert_eq!(
        excluded, all,
        "no sparq-mpc protocol is established against an actively deviating party today, \
         so requiring malicious security must exclude all of them"
    );
}

/// The `auth_disclose` surface carries an IT-MAC and was once named
/// `malicious_disclose_…`, but it does NOT deliver malicious security: `auth_mul` adopts
/// a value tamper on its second operand, and the range proof's zero-test mask sits in
/// that slot, so a deviating party can flip the verdict instead of forcing an abort.
/// Reading its `auth_` surface as malicious-secure is exactly the over-claim this graph
/// prevents, so it must stay in the semi-honest set.
#[test]
fn the_experimental_tamper_evident_disclose_is_annotated_semi_honest_despite_its_mac_layer() {
    let annotations = parse_annotations();
    let block = &annotations[MPC_TAMPER_EVIDENT_DISCLOSE_V1];
    assert!(
        block.rests_on(SECX_SEMI_HONEST),
        "the experimental tamper-evident disclosure must rest on secx:SemiHonest — its MAC \
         coverage is partial and a tamper can flip the verdict rather than abort"
    );
}

/// Nothing in this crate is dishonest-majority secure, so requiring that regime must
/// exclude the ENTIRE annotated set — including the malicious-secure comparison, which
/// is honest-majority-only and does not enter the SPDZ regime.
#[test]
fn requiring_dishonest_majority_excludes_the_entire_crate() {
    let annotations = parse_annotations();
    let excluded: BTreeSet<String> = excluded_by_requiring_dishonest_majority(&annotations)
        .into_iter()
        .collect();
    let all: BTreeSet<String> = ANNOTATED_METHODS.iter().map(|m| (*m).to_owned()).collect();
    assert_eq!(
        excluded, all,
        "every sparq-mpc protocol rests on secx:HonestMajority, so requiring \
         dishonest-majority security must exclude all of them"
    );
}

/// Every `secx:` term the graph uses must be a canonical vocabulary IRI from the
/// `sparq-secprop-vocab` leaf — no locally-invented dimension, level or assumption.
#[test]
fn every_secx_term_used_is_canonical_vocabulary() {
    for (method, block) in parse_annotations() {
        for a in &block.assertions {
            for iri in std::iter::once(&a.property)
                .chain(a.level.iter())
                .chain(a.audit_status.iter())
                .chain(a.assumptions.iter())
            {
                assert!(
                    ALL_SECPROP_IRIS.contains(&iri.as_str()),
                    "{method}: {iri} is not a canonical secx: term from sparq-secprop-vocab"
                );
            }
        }
    }
}

/// Guard 4 is only worth having if it goes RED on a wrong answer. Hand-build an
/// annotation map in which the equality join OMITS `secx:SemiHonest` while the code
/// still reports [`AdversaryModel::SemiHonest`] for that operator, and assert the guard
/// reports the drift. Without this, a guard that always returned `vec![]` would pass
/// every other test in this file.
#[test]
fn descriptor_drift_guard_goes_red_when_the_turtle_contradicts_the_code() {
    let backend = backend();

    // Sanity: the code really does report semi-honest for this operator, so omitting
    // the assumption below is genuinely a contradiction and not a vacuous setup.
    assert_eq!(
        backend
            .operator_descriptor(OperatorClass::EqualityJoin)
            .adversary,
        AdversaryModel::SemiHonest,
        "precondition: the degree-2t equality open is semi-honest-only at n = 2t+1"
    );

    let mut assumptions = BTreeSet::new();
    assumptions.insert(SECX_HONEST_MAJORITY.to_owned());
    // NOTE: secx:SemiHonest deliberately omitted — the over-claim we want caught.
    let drifted = MethodAnnotations {
        method: MPC_EQUALITY_JOIN_V1.to_owned(),
        assertions: vec![PropertyAssertion {
            property: "https://w3id.org/zkp-sparql/sec-prop#Soundness".to_owned(),
            level: Some("https://w3id.org/zkp-sparql/sec-prop#Sound".to_owned()),
            assurance: Assurance::Claimed,
            audit_status: Some(SECX_EXTERNAL_SIGN_OFF_PENDING.to_owned()),
            assumptions,
        }],
    };
    let mut annotations = BTreeMap::new();
    annotations.insert(MPC_EQUALITY_JOIN_V1.to_owned(), drifted);

    let violations = descriptor_drift_violations(&annotations, &backend);
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::DescriptorDrift
                && v.method == MPC_EQUALITY_JOIN_V1),
        "omitting secx:SemiHonest while the code reports AdversaryModel::SemiHonest must be \
         reported as descriptor drift; got {violations:#?}"
    );
}

/// The companion mutation check for guard 2: a claim with no stated assumption must be
/// reported, since an unstated adversary model is the over-claim §5c exists to prevent.
#[test]
fn assumption_guard_goes_red_on_a_claim_that_states_no_assumption() {
    let bare = MethodAnnotations {
        method: MPC_COMPARISON_V1.to_owned(),
        assertions: vec![PropertyAssertion {
            property: "https://w3id.org/zkp-sparql/sec-prop#Soundness".to_owned(),
            level: Some("https://w3id.org/zkp-sparql/sec-prop#Sound".to_owned()),
            assurance: Assurance::Claimed,
            audit_status: Some(SECX_EXTERNAL_SIGN_OFF_PENDING.to_owned()),
            assumptions: BTreeSet::new(),
        }],
    };
    let mut annotations = BTreeMap::new();
    annotations.insert(MPC_COMPARISON_V1.to_owned(), bare);

    let violations = assumption_completeness_violations(&annotations);
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::UnstatedAssumption),
        "an assertion with no secx:assumption must be reported; got {violations:#?}"
    );
}

/// Each [`OperatorClass`] the backend reports a descriptor for is annotated, so the
/// drift guard actually covers the operator surface rather than a subset of it.
#[test]
fn every_operator_class_is_annotated() {
    let annotations = parse_annotations();
    for (operator, iri) in OPERATOR_CLASS_METHODS {
        assert!(
            annotations.contains_key(*iri),
            "{operator:?} maps to {iri}, which has no annotation block"
        );
    }
}

/// The assurance axis is uniform on the positive rows: every positive claim in this
/// crate is `Claimed`, never stronger, while the estate is unaudited.
#[test]
fn positive_claims_are_uniformly_claimed() {
    let annotations = parse_annotations();
    let positives = annotations
        .values()
        .flat_map(|m| &m.assertions)
        .filter(|a| a.level.as_deref() != Some(SECX_NOT_ZK));
    for a in positives {
        assert_eq!(a.assurance, Assurance::Claimed);
        assert!(ALL_SECPROP_IRIS.contains(&SECX_CLAIMED));
    }
}
