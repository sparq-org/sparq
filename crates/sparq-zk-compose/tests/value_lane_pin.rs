//! The value-lane PIN functions against real manifests — and the proof that the
//! value-lane verifier GATES STAY CLOSED.
//!
//! `sparq_zk_compose::value_pin` answers "what would this value-lane sub-proof's
//! public inputs have to be, for this authenticated method and this typed query
//! FILTER?". It answers nothing else. The load-bearing test in this file is
//! [`a_manifest_whose_pin_passes_is_still_refused`]: a manifest whose value-lane
//! sub-proof satisfies every pinned equality is STILL refused by the verifier's
//! pre-filter with `ValueLaneSubProofNotAccepted`. Computing a pin is not
//! enforcing one, and nothing in this slice changes which manifests verify.
//!
//! Honest scope: research-grade, NOT externally audited (sq-qhy4); the value
//! lanes carry the dual-leaf INV-VL downgrade (#769 accepted, CR-G8).
#![cfg(feature = "dual-leaf")]

use oxrdf::{Literal, NamedNode};
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::dual_leaf_v2;
use sparq_zk::field::{field_to_hex, Fr};
use sparq_zk::verify::{fragment_typed_filters, FilterCmp, TypedQueryFilter};
use sparq_zk_compose::manifest::{CircuitId, FieldHex, FilterOp, ProofInputs, ProofManifest};
use sparq_zk_compose::value_pin::{
    check_lane_routed, check_value_lane_pin, expected_value_lane, ExpectedBound,
    ExpectedLaneConst, PinError,
};
use sparq_zk_compose::verifier::{
    prefilter_manifest_structure, CheckError, KeySet, RevocationPolicy,
};
use sparq_zk_compose::AuthenticatedMethod;

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";

const V2: AuthenticatedMethod = AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2);

fn typed(value: &str, datatype: &str) -> Literal {
    Literal::new_typed_literal(value, NamedNode::new_unchecked(datatype))
}

/// A minimal manifest carrying exactly `inputs` as its sub-proofs, over a query
/// with ONE integer FILTER — enough to reach the pre-filter's stage 0.
fn manifest_with(inputs: Vec<ProofInputs>, query: &str) -> ProofManifest {
    let sub_proofs: Vec<String> = inputs
        .iter()
        .map(|i| {
            format!(
                "{{\"inputs\":{},\"proof_hex\":\"\"}}",
                serde_json::to_string(i).expect("inputs serialize")
            )
        })
        .collect();
    let json = format!(
        "{{\"type\":\"urn:sparq:zk:ProofManifest\",\"query\":{},\"attributions\":[],\"entailment_regime\":\"simple\",\"binding\":{{\"mode\":\"challenge\",\"challenge\":\"0x2a\"}},\"sub_proofs\":[{}]}}",
        serde_json::to_string(query).expect("the query serializes"),
        sub_proofs.join(",")
    );
    ProofManifest::from_json(&json).expect("the minimal manifest parses")
}

/// The integer-lane sub-proof that satisfies every pinned equality for
/// `FILTER(?o >= 18)`.
fn pin_passing_integer_lane() -> ProofInputs {
    ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc: FieldHex(field_to_hex(&Fr::from(1u64))),
        op: FilterOp::Ge,
        bound: 18,
        datatype_const: FieldHex(field_to_hex(&dual_leaf_v2::integer_lane_const())),
        expected: true,
    }
}

const PIN_PASSING_QUERY: &str = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= \"18\"^^<http://www.w3.org/2001/XMLSchema#integer>) }";

// ------------------------------------------------- THE GATES ARE STILL CLOSED

/// THE LOAD-BEARING TEST. A manifest whose value-lane sub-proof satisfies the
/// FULL pin — right member, right lane constant, right op, right bound, verdict
/// `true` — is STILL refused by the verifier's structural pre-filter, with the
/// unconditional stage-0 `ValueLaneSubProofNotAccepted`.
///
/// That refusal is FIRST, before the pre-filter reads a sub-proof for anything
/// else, so it cannot be reached past by making the pin hold. If this test ever
/// goes green on an `Ok(())`, the gate has been opened — which is a separate
/// change with its own obligations, not a side effect of being able to compute a
/// pin.
#[test]
fn a_manifest_whose_pin_passes_is_still_refused() {
    let inputs = pin_passing_integer_lane();
    // First: the pin really does hold, so the refusal below is not just the pin
    // failing by accident.
    let filters = fragment_typed_filters(PIN_PASSING_QUERY).expect("the query's filters extract");
    assert_eq!(filters.len(), 1);
    assert_eq!(
        check_value_lane_pin(V2, &filters[0], &inputs),
        Ok(()),
        "the fixture must be pin-PASSING for this test to mean anything"
    );

    // And the verifier refuses it anyway.
    let manifest = manifest_with(vec![inputs], PIN_PASSING_QUERY);
    match prefilter_manifest_structure(
        &manifest,
        &KeySet::empty(),
        &RevocationPolicy::accept_version(1),
    ) {
        Err(CheckError::ValueLaneSubProofNotAccepted { proof }) => assert_eq!(proof, 0),
        other => panic!(
            "the value-lane gate must stay CLOSED; expected ValueLaneSubProofNotAccepted, got {other:?}"
        ),
    }
}

/// The stage-0 refusal does not depend on the policy either: the strictest
/// method policy and the most permissive one both refuse a value-lane sub-proof.
#[test]
fn no_method_policy_opens_the_value_lane_gate() {
    use sparq_zk_compose::MethodPolicy;
    let manifest = manifest_with(vec![pin_passing_integer_lane()], PIN_PASSING_QUERY);
    for policy in [
        MethodPolicy::current(),
        MethodPolicy::current().require_method_bound(),
        MethodPolicy::current().admit_legacy_dual_leaf_v1(),
    ] {
        let ks = KeySet::empty().with_method_policy(policy);
        assert!(
            matches!(
                prefilter_manifest_structure(&manifest, &ks, &RevocationPolicy::accept_version(1)),
                Err(CheckError::ValueLaneSubProofNotAccepted { .. })
            ),
            "no policy may open the value-lane gate"
        );
    }
}

// --------------------------------------------------------- the pin, positively

#[test]
fn the_pin_holds_only_for_the_filter_the_query_actually_spells() {
    let inputs = pin_passing_integer_lane();
    let matching = TypedQueryFilter {
        variable: "o".into(),
        op: FilterCmp::Ge,
        constant: typed("18", XSD_INTEGER),
    };
    assert_eq!(check_value_lane_pin(V2, &matching, &inputs), Ok(()));

    // A different bound.
    let other_bound = TypedQueryFilter { constant: typed("19", XSD_INTEGER), ..matching.clone() };
    assert_eq!(
        check_value_lane_pin(V2, &other_bound, &inputs),
        Err(PinError::BoundMismatch)
    );
    // A different comparison.
    let other_op = TypedQueryFilter { op: FilterCmp::Gt, ..matching.clone() };
    assert!(matches!(
        check_value_lane_pin(V2, &other_op, &inputs),
        Err(PinError::OpMismatch { .. })
    ));
    // A different datatype routes to a different member.
    let other_lane = TypedQueryFilter { constant: typed("1.8", XSD_DECIMAL), ..matching };
    assert!(matches!(
        check_value_lane_pin(V2, &other_lane, &inputs),
        Err(PinError::MemberMismatch { .. })
    ));
}

/// The pin is computed from the TYPED filter extraction, so the two surfaces
/// compose: the query text alone determines what the pin demands.
#[test]
fn the_query_text_alone_determines_what_the_pin_demands() {
    let q = "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o FILTER(\"1.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> <= ?o) }";
    let filters = fragment_typed_filters(q).expect("extracts");
    assert_eq!(filters.len(), 1);
    // The const-op-var form flipped, so the pin demands `>=`.
    let e = expected_value_lane(V2, filters[0].op, &filters[0].constant).expect("routes");
    assert_eq!(e.op, FilterOp::Ge);
    assert_eq!(e.member, CircuitId::FilterValueDlDecimal);
    assert_eq!(e.lane_const, ExpectedLaneConst::DecimalAtAdmittedScale);
    assert_eq!(
        e.bound,
        ExpectedBound::Decimal { neg: false, magnitude: 150, fraction_digits: 2 }
    );

    let inputs = ProofInputs::FilterValueDlDecimal {
        id: CircuitId::FilterValueDlDecimal,
        operand_enc: FieldHex(field_to_hex(&Fr::from(1u64))),
        op: FilterOp::Ge,
        bound_neg: false,
        bound_scaled: 1_500,
        datatype_const: FieldHex(field_to_hex(&dual_leaf_v2::decimal_lane_const(3))),
        expected: true,
    };
    assert_eq!(
        check_value_lane_pin(V2, &filters[0], &inputs),
        Ok(()),
        "1.50 at the presented scale 3 is 1500, exactly"
    );
    // The same magnitude under a scale the constant cannot reach with it.
    let wrong_scale = ProofInputs::FilterValueDlDecimal {
        id: CircuitId::FilterValueDlDecimal,
        operand_enc: FieldHex(field_to_hex(&Fr::from(1u64))),
        op: FilterOp::Ge,
        bound_neg: false,
        bound_scaled: 1_500,
        datatype_const: FieldHex(field_to_hex(&dual_leaf_v2::decimal_lane_const(4))),
        expected: true,
    };
    assert_eq!(
        check_value_lane_pin(V2, &filters[0], &wrong_scale),
        Err(PinError::BoundMismatch),
        "at the presented scale 4 the bound would have to be 15000"
    );
}

// ------------------------------------------------------- the method axis again

/// Pinning requires an AUTHENTICATED `DualLeafV2`. An unbound attestation gives
/// no leaf shape to pin against, and the other methods have no V2 value lane.
#[test]
fn pinning_requires_an_authenticated_dual_leaf_v2() {
    let f = TypedQueryFilter {
        variable: "o".into(),
        op: FilterCmp::Ge,
        constant: typed("18", XSD_INTEGER),
    };
    let inputs = pin_passing_integer_lane();
    assert_eq!(
        check_value_lane_pin(AuthenticatedMethod::Unbound, &f, &inputs),
        Err(PinError::MethodNotBound)
    );
    assert_eq!(
        check_value_lane_pin(
            AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1),
            &f,
            &inputs
        ),
        Err(PinError::LegacyMethodValueLane)
    );
    assert!(matches!(
        check_value_lane_pin(
            AuthenticatedMethod::Bound(CommitmentMethod::StringCanonicalV1),
            &f,
            &inputs
        ),
        Err(PinError::MethodHasNoValueLane { .. })
    ));
}

/// `check_lane_routed` is the public statement of which lanes a committed graph
/// can actually hold today — the unrouted seam is refused.
#[test]
fn the_unrouted_lanes_are_refused_by_the_routing_check() {
    assert_eq!(check_lane_routed(XSD_INTEGER), Ok(()));
    assert_eq!(check_lane_routed(XSD_DECIMAL), Ok(()));
    for dt in [
        "http://www.w3.org/2001/XMLSchema#boolean",
        "http://www.w3.org/2001/XMLSchema#dateTime",
        "http://www.w3.org/2001/XMLSchema#date",
    ] {
        assert!(matches!(
            check_lane_routed(dt),
            Err(PinError::LaneNotRouted { .. })
        ));
    }
}
