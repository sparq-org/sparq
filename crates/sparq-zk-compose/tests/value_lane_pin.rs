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
/// can actually hold. The whole-graph builder now routes all six value lanes, so
/// all six pass; every other datatype is still refused.
#[test]
fn the_routing_check_admits_the_six_value_lanes() {
    for dt in [
        XSD_INTEGER,
        XSD_DECIMAL,
        "http://www.w3.org/2001/XMLSchema#double",
        "http://www.w3.org/2001/XMLSchema#boolean",
        "http://www.w3.org/2001/XMLSchema#dateTime",
        "http://www.w3.org/2001/XMLSchema#date",
    ] {
        assert_eq!(check_lane_routed(dt), Ok(()), "{dt} is routed");
    }
    for dt in [
        "http://www.w3.org/2001/XMLSchema#string",
        "http://www.w3.org/2001/XMLSchema#float",
        "http://ex/opaque",
    ] {
        assert!(matches!(
            check_lane_routed(dt),
            Err(PinError::LaneNotRouted { .. })
        ));
    }
}

// ============ the V2 value-lane BUILD helpers, against the pin ==============
//
// `sparq_zk_compose::build` builds a value-lane sub-proof's public inputs from
// TYPED operands, one entry per lane over one shared assembly. The claim these
// tests make is the composition one: what a builder emits is EXACTLY what
// `value_pin` requires of that lane for the matching typed FILTER, and changing
// any ONE pinned field of it is refused with that field's own `PinError`.
//
// They do not open a gate. `the_gates_stay_closed_for_a_helper_built_manifest`
// below is the control: a manifest assembled from a builder's output is still
// refused with `ValueLaneSubProofNotAccepted`.

const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";

fn hexf(f: &Fr) -> FieldHex {
    FieldHex(field_to_hex(f))
}

fn filter_on(op: FilterCmp, constant: Literal) -> TypedQueryFilter {
    TypedQueryFilter { variable: "o".into(), op, constant }
}

/// Replace ONE public field of a value-lane `ProofInputs`, through its own
/// serialisation, so the mutation touches nothing else.
fn mutated(inputs: &ProofInputs, key: &str, value: serde_json::Value) -> ProofInputs {
    let mut v = serde_json::to_value(inputs).expect("inputs serialize");
    let obj = v.as_object_mut().expect("a value-lane ProofInputs is a JSON object");
    assert!(obj.contains_key(key), "{} is a field of this variant", key);
    obj.insert(key.to_string(), value);
    serde_json::from_value(v).expect("the mutated inputs deserialize")
}

/// The same public inputs presented on a DIFFERENT value-lane member — the
/// member-mismatch mutation. Lane constant, op, bound and verdict are carried
/// over unchanged, so the only thing that differs is the member.
fn on_another_member(inputs: &ProofInputs) -> ProofInputs {
    match inputs {
        ProofInputs::FilterValueDl { operand_enc, op, bound, datatype_const, expected, .. }
        | ProofInputs::FilterValueDlBool {
            operand_enc,
            op,
            bound,
            datatype_const,
            expected,
            ..
        } => ProofInputs::FilterValueDlF64 {
            id: CircuitId::FilterValueDlF64,
            operand_enc: operand_enc.clone(),
            op: *op,
            b_bits: *bound,
            datatype_const: datatype_const.clone(),
            expected: *expected,
        },
        ProofInputs::FilterValueDlF64 {
            operand_enc,
            op,
            b_bits,
            datatype_const,
            expected,
            ..
        } => ProofInputs::FilterValueDl {
            id: CircuitId::FilterValueDl,
            operand_enc: operand_enc.clone(),
            op: *op,
            bound: *b_bits,
            datatype_const: datatype_const.clone(),
            expected: *expected,
        },
        ProofInputs::FilterValueDlDecimal {
            operand_enc,
            op,
            bound_neg,
            bound_scaled,
            datatype_const,
            expected,
            ..
        } => ProofInputs::FilterValueDlDateTime {
            id: CircuitId::FilterValueDlDateTime,
            operand_enc: operand_enc.clone(),
            op: *op,
            bound_neg: *bound_neg,
            bound_scaled_epoch: *bound_scaled,
            datatype_const: datatype_const.clone(),
            expected: *expected,
        },
        ProofInputs::FilterValueDlDateTime {
            operand_enc,
            op,
            bound_neg,
            bound_scaled_epoch,
            datatype_const,
            expected,
            ..
        } => ProofInputs::FilterValueDlDecimal {
            id: CircuitId::FilterValueDlDecimal,
            operand_enc: operand_enc.clone(),
            op: *op,
            bound_neg: *bound_neg,
            bound_scaled: *bound_scaled_epoch,
            datatype_const: datatype_const.clone(),
            expected: *expected,
        },
        other => panic!("not a value-lane sub-proof: {other:?}"),
    }
}

/// One lane's builder output, the FILTER it answers, and the two mutations whose
/// refusal is lane-specific.
struct LaneCase {
    lane: &'static str,
    inputs: ProofInputs,
    filter: TypedQueryFilter,
    /// The JSON key of this variant's bound field.
    bound_key: &'static str,
    /// A bound that is NOT the FILTER constant's encoding.
    other_bound: serde_json::Value,
    /// A lane constant that is not this lane's.
    other_lane_const: FieldHex,
    /// The refusal `other_lane_const` must produce: a lane whose expected
    /// constant is FIXED gives `LaneConstMismatch`; the decimal lane, whose
    /// admitted constant is `decimal_lane_const(fd)` for some admitted `fd`,
    /// gives `ScaleNotAdmitted` because a non-decimal constant names no scale.
    lane_const_error: PinError,
}

/// Every lane, built by its own entry in `sparq_zk_compose::build`.
fn lane_cases() -> Vec<LaneCase> {
    use sparq_zk_compose::build::{
        build_filter_value_dl_bool, build_filter_value_dl_date,
        build_filter_value_dl_datetime, build_filter_value_dl_decimal,
        build_filter_value_dl_f64, build_filter_value_dl_int,
    };
    vec![
        LaneCase {
            lane: "xsd:integer",
            inputs: build_filter_value_dl_int(
                &typed("18", XSD_INTEGER),
                FilterOp::Ge,
                &typed("18", XSD_INTEGER),
            )
            .expect("the integer builder accepts canonical operands")
            .inputs,
            filter: filter_on(FilterCmp::Ge, typed("18", XSD_INTEGER)),
            bound_key: "bound",
            other_bound: serde_json::json!(19),
            other_lane_const: hexf(&dual_leaf_v2::double_lane_const()),
            lane_const_error: PinError::LaneConstMismatch,
        },
        LaneCase {
            lane: "xsd:double",
            inputs: build_filter_value_dl_f64(
                &typed("2.5E0", XSD_DOUBLE),
                FilterOp::Gt,
                &typed("1.0E0", XSD_DOUBLE),
            )
            .expect("the double builder accepts canonical operands")
            .inputs,
            filter: filter_on(FilterCmp::Gt, typed("1.0E0", XSD_DOUBLE)),
            bound_key: "b_bits",
            other_bound: serde_json::json!(0),
            other_lane_const: hexf(&dual_leaf_v2::integer_lane_const()),
            lane_const_error: PinError::LaneConstMismatch,
        },
        LaneCase {
            lane: "xsd:decimal",
            // The committed literal is at scale 2 and the FILTER constant at
            // scale 1, so the bound is carried rescaled EXACTLY to scale 2.
            inputs: build_filter_value_dl_decimal(
                &typed("2.50", XSD_DECIMAL),
                FilterOp::Ge,
                &typed("1.5", XSD_DECIMAL),
            )
            .expect("the decimal builder accepts canonical operands")
            .inputs,
            filter: filter_on(FilterCmp::Ge, typed("1.5", XSD_DECIMAL)),
            bound_key: "bound_scaled",
            other_bound: serde_json::json!(151),
            other_lane_const: hexf(&dual_leaf_v2::integer_lane_const()),
            lane_const_error: PinError::ScaleNotAdmitted,
        },
        LaneCase {
            lane: "xsd:boolean",
            inputs: build_filter_value_dl_bool(
                &typed("true", XSD_BOOLEAN),
                FilterOp::Eq,
                true,
            )
            .expect("the boolean builder accepts a canonical lexical")
            .inputs,
            filter: filter_on(FilterCmp::Eq, typed("true", XSD_BOOLEAN)),
            bound_key: "bound",
            other_bound: serde_json::json!(0),
            other_lane_const: hexf(&dual_leaf_v2::integer_lane_const()),
            lane_const_error: PinError::LaneConstMismatch,
        },
        LaneCase {
            lane: "xsd:dateTime",
            inputs: build_filter_value_dl_datetime(
                &typed("2024-01-02T03:04:05Z", XSD_DATE_TIME),
                FilterOp::Ge,
                &typed("2020-01-01T00:00:00Z", XSD_DATE_TIME),
            )
            .expect("the dateTime builder accepts canonical operands")
            .inputs,
            filter: filter_on(FilterCmp::Ge, typed("2020-01-01T00:00:00Z", XSD_DATE_TIME)),
            bound_key: "bound_scaled_epoch",
            other_bound: serde_json::json!(1),
            // The date lane SHARES this member, so its constant is the one thing
            // that would read a date proof as a dateTime one.
            other_lane_const: hexf(&dual_leaf_v2::date_lane_const()),
            lane_const_error: PinError::LaneConstMismatch,
        },
        LaneCase {
            lane: "xsd:date",
            inputs: build_filter_value_dl_date(
                &typed("2024-01-02Z", XSD_DATE),
                FilterOp::Ge,
                &typed("2020-01-01Z", XSD_DATE),
            )
            .expect("the date builder accepts canonical operands")
            .inputs,
            filter: filter_on(FilterCmp::Ge, typed("2020-01-01Z", XSD_DATE)),
            bound_key: "bound_scaled_epoch",
            other_bound: serde_json::json!(1),
            other_lane_const: hexf(&dual_leaf_v2::datetime_lane_const()),
            lane_const_error: PinError::LaneConstMismatch,
        },
    ]
}

/// THE COMPOSITION TEST. Every lane's builder output satisfies the FULL pin for
/// the typed FILTER it was built against.
#[test]
fn every_lanes_builder_output_satisfies_the_pin() {
    let cases = lane_cases();
    assert_eq!(cases.len(), 6, "all six value lanes are covered");
    for c in &cases {
        assert_eq!(
            check_value_lane_pin(V2, &c.filter, &c.inputs),
            Ok(()),
            "{} must pin",
            c.lane
        );
    }
}

/// ... and changing any ONE pinned field of it is refused, with that field's own
/// `PinError`. Mutating a field is what makes the test above non-vacuous.
#[test]
fn every_mismatched_field_of_a_builder_output_is_refused() {
    for c in &lane_cases() {
        // 1. the member
        assert!(
            matches!(
                check_value_lane_pin(V2, &c.filter, &on_another_member(&c.inputs)),
                Err(PinError::MemberMismatch { .. })
            ),
            "{}: another member must be refused",
            c.lane
        );
        // 2. the lane constant
        assert_eq!(
            check_value_lane_pin(
                V2,
                &c.filter,
                &mutated(
                    &c.inputs,
                    "datatype_const",
                    serde_json::json!(c.other_lane_const.0)
                )
            ),
            Err(c.lane_const_error.clone()),
            "{}: another lane constant must be refused",
            c.lane
        );
        // 3. the op
        let other_op = if c.filter.op == FilterCmp::Eq { "ne" } else { "lt" };
        assert!(
            matches!(
                check_value_lane_pin(
                    V2,
                    &c.filter,
                    &mutated(&c.inputs, "op", serde_json::json!(other_op))
                ),
                Err(PinError::OpMismatch { .. })
            ),
            "{}: another op must be refused",
            c.lane
        );
        // 4. the bound
        assert_eq!(
            check_value_lane_pin(
                V2,
                &c.filter,
                &mutated(&c.inputs, c.bound_key, c.other_bound.clone())
            ),
            Err(PinError::BoundMismatch),
            "{}: another bound must be refused",
            c.lane
        );
        // 5. the verdict
        assert_eq!(
            check_value_lane_pin(
                V2,
                &c.filter,
                &mutated(&c.inputs, "expected", serde_json::json!(false))
            ),
            Err(PinError::VerdictNotTrue),
            "{}: a false verdict must be refused",
            c.lane
        );
    }
}

/// THE CONTROL. A manifest whose value-lane sub-proof was assembled by one of
/// these builders — so its pin holds by construction — is STILL refused by the
/// verifier's pre-filter. Building correct inputs is not enforcing them.
#[test]
fn the_gates_stay_closed_for_a_helper_built_manifest() {
    for c in &lane_cases() {
        let manifest = manifest_with(vec![c.inputs.clone()], PIN_PASSING_QUERY);
        match prefilter_manifest_structure(
            &manifest,
            &KeySet::empty(),
            &RevocationPolicy::accept_version(1),
        ) {
            Err(CheckError::ValueLaneSubProofNotAccepted { proof }) => assert_eq!(proof, 0),
            other => panic!(
                "the value-lane gate must stay CLOSED for the {} builder; got {other:?}",
                c.lane
            ),
        }
    }
}

/// The typed path to a `Prover.toml`: a builder's output goes through
/// `ValueLaneProof::from_inputs`, which re-checks the public fields rather than
/// trusting them, and the entry refuses a public field edited after the build.
#[test]
fn a_builder_output_assembles_typed_inputs_and_a_tampered_one_does_not() {
    use sparq_zk_compose::build::build_filter_value_dl_int;
    let built = build_filter_value_dl_int(
        &typed("18", XSD_INTEGER),
        FilterOp::Ge,
        &typed("18", XSD_INTEGER),
    )
    .expect("canonical operands");
    let proof = built.lane_proof().expect("the build assembles typed inputs");
    assert_eq!(proof.circuit_id(), CircuitId::FilterValueDl);
    assert_eq!(proof.proof_inputs(), built.inputs);
    // The rendered TOML comes from the validated typed fields, not caller text.
    let toml = proof
        .checked_prover_toml(
            CommitmentMethod::DualLeafV2,
            sparq_zk::commit::MethodUse::NewIssuance,
            &FieldHex(field_to_hex(&Fr::from(1u64))),
        )
        .expect("a V2 integer lane renders under NewIssuance");
    assert!(toml.contains("bound"), "the member's bound is rendered");

    // A member swapped after the build is refused by the typed assembly.
    let tampered = sparq_zk_compose::build::BuiltFilterValueDlNumeric {
        inputs: on_another_member(&built.inputs),
        witness: built.witness.clone(),
    };
    assert!(
        tampered.lane_proof().is_ok(),
        "the f64 variant is itself a value-lane variant"
    );
    // ... but its lane constant no longer rides that member.
    assert!(tampered
        .lane_proof()
        .expect("assembles")
        .checked_prover_toml(
            CommitmentMethod::DualLeafV2,
            sparq_zk::commit::MethodUse::NewIssuance,
            &FieldHex(field_to_hex(&Fr::from(1u64))),
        )
        .is_err());
}

// --------------------------------------------- the builders' own refusals

/// The numeric builders are fail-closed on both operands: a non-canonical or
/// wrong-datatyped lexical is the encoder's error, never a repaired input.
#[test]
fn the_numeric_builders_refuse_a_non_canonical_or_wrong_datatype_operand() {
    use sparq_zk_compose::build::{
        build_filter_value_dl_decimal, build_filter_value_dl_f64, build_filter_value_dl_int,
        BuildValueDlV2Error,
    };
    // A negative integer is outside the non-negative integer lane.
    assert!(matches!(
        build_filter_value_dl_int(
            &typed("-1", XSD_INTEGER),
            FilterOp::Ge,
            &typed("0", XSD_INTEGER)
        ),
        Err(BuildValueDlV2Error::Encode(_))
    ));
    // A non-canonical integer spelling.
    assert!(matches!(
        build_filter_value_dl_int(
            &typed("018", XSD_INTEGER),
            FilterOp::Ge,
            &typed("18", XSD_INTEGER)
        ),
        Err(BuildValueDlV2Error::Encode(_))
    ));
    // The BOUND is checked too, by the same encoder.
    assert!(matches!(
        build_filter_value_dl_int(
            &typed("18", XSD_INTEGER),
            FilterOp::Ge,
            &typed("18", XSD_DECIMAL)
        ),
        Err(BuildValueDlV2Error::Encode(_))
    ));
    // A Rust-parser double extension the canonical predicate rejects.
    assert!(matches!(
        build_filter_value_dl_f64(
            &typed("2.5e0", XSD_DOUBLE),
            FilterOp::Gt,
            &typed("1.0E0", XSD_DOUBLE)
        ),
        Err(BuildValueDlV2Error::Encode(_))
    ));
    // A decimal with no fraction digits is not the canonical lexical form.
    assert!(matches!(
        build_filter_value_dl_decimal(
            &typed("2", XSD_DECIMAL),
            FilterOp::Ge,
            &typed("1.5", XSD_DECIMAL)
        ),
        Err(BuildValueDlV2Error::Encode(_))
    ));
}

/// The decimal lane carries the bound at the COMMITTED literal's scale, rescaled
/// EXACTLY. A bound that would need rounding to reach that scale is refused —
/// a rounded bound is a different bound, so the proof would answer a different
/// question than the query asked.
#[test]
fn the_decimal_builder_refuses_a_bound_that_cannot_be_rescaled_exactly() {
    use sparq_zk_compose::build::{build_filter_value_dl_decimal, BuildValueDlV2Error};
    // Scaling UP is exact: 1.5 at the literal's scale 2 is 150.
    let up = build_filter_value_dl_decimal(
        &typed("2.50", XSD_DECIMAL),
        FilterOp::Ge,
        &typed("1.5", XSD_DECIMAL),
    )
    .expect("scaling up is exact");
    match &up.inputs {
        ProofInputs::FilterValueDlDecimal { bound_scaled, datatype_const, .. } => {
            assert_eq!(*bound_scaled, 150);
            assert_eq!(datatype_const, &hexf(&dual_leaf_v2::decimal_lane_const(2)));
        }
        other => panic!("the decimal variant, got {other:?}"),
    }
    // Scaling DOWN is exact only when it discards nothing: 1.50 -> scale 1 is 15.
    let down = build_filter_value_dl_decimal(
        &typed("2.5", XSD_DECIMAL),
        FilterOp::Ge,
        &typed("1.50", XSD_DECIMAL),
    )
    .expect("scaling 1.50 down to scale 1 discards nothing");
    match &down.inputs {
        ProofInputs::FilterValueDlDecimal { bound_scaled, .. } => assert_eq!(*bound_scaled, 15),
        other => panic!("the decimal variant, got {other:?}"),
    }
    // But 1.55 cannot reach scale 1 without rounding.
    assert!(matches!(
        build_filter_value_dl_decimal(
            &typed("2.5", XSD_DECIMAL),
            FilterOp::Ge,
            &typed("1.55", XSD_DECIMAL)
        ),
        Err(BuildValueDlV2Error::DecimalScaleNotRepresentable { .. })
    ));
}

/// The signed decimal lane: a negative committed literal and a negative bound
/// are carried sign-split, and the verdict follows the signed order.
#[test]
fn the_decimal_builder_carries_a_negative_operand_sign_split() {
    use sparq_zk_compose::build::build_filter_value_dl_decimal;
    let built = build_filter_value_dl_decimal(
        &typed("-2.50", XSD_DECIMAL),
        FilterOp::Lt,
        &typed("-1.50", XSD_DECIMAL),
    )
    .expect("canonical negative operands");
    match &built.inputs {
        ProofInputs::FilterValueDlDecimal { bound_neg, bound_scaled, expected, .. } => {
            assert!(*bound_neg);
            assert_eq!(*bound_scaled, 150);
            assert!(*expected, "-2.50 < -1.50");
        }
        other => panic!("the decimal variant, got {other:?}"),
    }
    assert_eq!(
        check_value_lane_pin(V2, &filter_on(FilterCmp::Lt, typed("-1.50", XSD_DECIMAL)), &built.inputs),
        Ok(())
    );
}

/// The verdict is COMPUTED by the builder, not taken from the caller — and it is
/// the member's own verdict relation. These are the two host mirrors, exercised
/// directly over the op set.
#[test]
fn the_host_verdict_mirrors_decide_every_operator() {
    use sparq_zk_compose::build::{double_bits_verdict, integer_verdict};
    for (op, want) in [
        (FilterOp::Lt, false),
        (FilterOp::Le, true),
        (FilterOp::Gt, false),
        (FilterOp::Ge, true),
        (FilterOp::Eq, true),
        (FilterOp::Ne, false),
    ] {
        assert_eq!(integer_verdict(18, op, 18), want, "18 {:?} 18", op);
    }
    assert!(integer_verdict(17, FilterOp::Lt, 18));
    assert!(!integer_verdict(19, FilterOp::Le, 18));

    let bits = |v: f64| v.to_bits();
    for (op, want) in [
        (FilterOp::Lt, false),
        (FilterOp::Le, false),
        (FilterOp::Gt, true),
        (FilterOp::Ge, true),
        (FilterOp::Eq, false),
        (FilterOp::Ne, true),
    ] {
        assert_eq!(
            double_bits_verdict(bits(2.5), op, bits(1.0)),
            want,
            "2.5 {:?} 1.0",
            op
        );
    }
    // IEEE semantics, which is what the member's `f64_verdict` decides: -0.0
    // equals +0.0, and a NaN is unordered so only NE holds of it.
    assert!(double_bits_verdict(bits(-0.0), FilterOp::Eq, bits(0.0)));
    assert!(double_bits_verdict(bits(f64::NAN), FilterOp::Ne, bits(1.0)));
    for op in [FilterOp::Lt, FilterOp::Le, FilterOp::Gt, FilterOp::Ge, FilterOp::Eq] {
        assert!(
            !double_bits_verdict(bits(f64::NAN), op, bits(1.0)),
            "NaN is unordered under {:?}",
            op
        );
    }
}

/// Each refusal of the numeric builders renders a distinct message, so a caller
/// learns which operand or which scale was the problem.
#[test]
fn the_numeric_builder_errors_render_distinctly() {
    use sparq_zk_compose::build::BuildValueDlV2Error;
    let msgs = [
        BuildValueDlV2Error::HandleOutsideLaneDomain { lexical: "\"x\"".into() }.to_string(),
        BuildValueDlV2Error::DecimalScaleNotRepresentable {
            value: "\"2.5\"".into(),
            bound: "\"1.55\"".into(),
        }
        .to_string(),
    ];
    for m in &msgs {
        assert!(!m.is_empty());
    }
    assert_eq!(
        msgs.iter().collect::<std::collections::HashSet<_>>().len(),
        msgs.len(),
        "each refusal says something different"
    );
}

/// THE VERDICT IS COMPUTED, NOT ASSUMED. Each numeric builder discloses the
/// verdict its member's own relation decides — including `false` — so a host
/// cannot disclose a verdict the member would refuse to prove. A pin requires
/// `true`, so these FALSE cases are the ones that would be invisible to the pin
/// tests above: they are what pins the computation itself.
#[test]
fn the_numeric_builders_disclose_the_computed_verdict_including_false() {
    use sparq_zk_compose::build::{
        build_filter_value_dl_decimal, build_filter_value_dl_f64, build_filter_value_dl_int,
    };
    let verdict = |inputs: &ProofInputs| match inputs {
        ProofInputs::FilterValueDl { expected, .. }
        | ProofInputs::FilterValueDlF64 { expected, .. }
        | ProofInputs::FilterValueDlDecimal { expected, .. } => *expected,
        other => panic!("a numeric value-lane variant, got {other:?}"),
    };
    // xsd:integer, over every operator at 17 vs 18.
    for (op, want) in [
        (FilterOp::Lt, true),
        (FilterOp::Le, true),
        (FilterOp::Gt, false),
        (FilterOp::Ge, false),
        (FilterOp::Eq, false),
        (FilterOp::Ne, true),
    ] {
        let built =
            build_filter_value_dl_int(&typed("17", XSD_INTEGER), op, &typed("18", XSD_INTEGER))
                .expect("canonical integer operands");
        assert_eq!(verdict(&built.inputs), want, "17 {:?} 18", op);
    }
    // xsd:double, over every operator at 1.0 vs 2.5.
    for (op, want) in [
        (FilterOp::Lt, true),
        (FilterOp::Le, true),
        (FilterOp::Gt, false),
        (FilterOp::Ge, false),
        (FilterOp::Eq, false),
        (FilterOp::Ne, true),
    ] {
        let built =
            build_filter_value_dl_f64(&typed("1.0E0", XSD_DOUBLE), op, &typed("2.5E0", XSD_DOUBLE))
                .expect("canonical double operands");
        assert_eq!(verdict(&built.inputs), want, "1.0 {:?} 2.5", op);
    }
    // xsd:decimal, over every operator at -2.50 vs 1.5 (so the signed order and
    // the exact rescale both participate).
    for (op, want) in [
        (FilterOp::Lt, true),
        (FilterOp::Le, true),
        (FilterOp::Gt, false),
        (FilterOp::Ge, false),
        (FilterOp::Eq, false),
        (FilterOp::Ne, true),
    ] {
        let built = build_filter_value_dl_decimal(
            &typed("-2.50", XSD_DECIMAL),
            op,
            &typed("1.5", XSD_DECIMAL),
        )
        .expect("canonical decimal operands");
        assert_eq!(verdict(&built.inputs), want, "-2.50 {:?} 1.5", op);
    }
}

/// `operand_enc` is the COMMITTED literal's term encoding — the leaf a V2
/// commitment holds for it and a scan proof discloses for the operand column —
/// not the FILTER constant's. Asserted against `dual_leaf_v2::encode_term`, the
/// builder the commitment itself runs.
#[test]
fn the_numeric_builders_anchor_operand_enc_to_the_committed_literal() {
    use sparq_zk_compose::build::{
        build_filter_value_dl_decimal, build_filter_value_dl_f64, build_filter_value_dl_int,
    };
    let salt = Fr::from(0u64);
    let cases = [
        (
            "xsd:integer",
            typed("18", XSD_INTEGER),
            build_filter_value_dl_int(
                &typed("18", XSD_INTEGER),
                FilterOp::Gt,
                &typed("3", XSD_INTEGER),
            )
            .unwrap()
            .inputs,
        ),
        (
            "xsd:double",
            typed("2.5E0", XSD_DOUBLE),
            build_filter_value_dl_f64(
                &typed("2.5E0", XSD_DOUBLE),
                FilterOp::Gt,
                &typed("1.0E0", XSD_DOUBLE),
            )
            .unwrap()
            .inputs,
        ),
        (
            "xsd:decimal",
            typed("2.50", XSD_DECIMAL),
            build_filter_value_dl_decimal(
                &typed("2.50", XSD_DECIMAL),
                FilterOp::Ge,
                &typed("1.5", XSD_DECIMAL),
            )
            .unwrap()
            .inputs,
        ),
    ];
    for (lane, literal, inputs) in cases {
        let operand_enc = match &inputs {
            ProofInputs::FilterValueDl { operand_enc, .. }
            | ProofInputs::FilterValueDlF64 { operand_enc, .. }
            | ProofInputs::FilterValueDlDecimal { operand_enc, .. } => operand_enc.clone(),
            other => panic!("a numeric value-lane variant, got {other:?}"),
        };
        let committed = dual_leaf_v2::encode_term(&oxrdf::Term::Literal(literal), &salt)
            .expect("a routed lane's canonical literal encodes")
            .expect("a literal is not a triple term");
        assert_eq!(
            operand_enc,
            FieldHex(field_to_hex(&committed)),
            "{} must anchor to the committed literal's term encoding",
            lane
        );
    }
}

/// The PRIVATE witness a numeric builder emits REBINDS to the public
/// `operand_enc`: `h3(h3(hook, datatype_const, LANG_NONE), lexical_component,
/// TYPE_CODE_LITERAL)` is exactly the presented operand encoding. That is the
/// leaf binding the member asserts, so a witness drawn from the wrong operand
/// would be unprovable — this is the host-side statement of it.
#[test]
fn the_numeric_builders_witness_rebinds_to_the_presented_operand_enc() {
    use sparq_zk::dual_leaf::LANG_NONE;
    use sparq_zk::encode::TYPE_CODE_LITERAL;
    use sparq_zk::poseidon2;
    use sparq_zk_compose::build::{
        build_filter_value_dl_decimal, build_filter_value_dl_f64, build_filter_value_dl_int,
    };
    use sparq_zk_compose::value_lane::LaneWitness;

    let builds = [
        build_filter_value_dl_int(
            &typed("18", XSD_INTEGER),
            FilterOp::Gt,
            &typed("3", XSD_INTEGER),
        )
        .unwrap(),
        build_filter_value_dl_f64(
            &typed("2.5E0", XSD_DOUBLE),
            FilterOp::Gt,
            &typed("1.0E0", XSD_DOUBLE),
        )
        .unwrap(),
        build_filter_value_dl_decimal(
            &typed("-2.50", XSD_DECIMAL),
            FilterOp::Lt,
            &typed("1.5", XSD_DECIMAL),
        )
        .unwrap(),
    ];
    for built in &builds {
        let (operand_enc, datatype_const) = match &built.inputs {
            ProofInputs::FilterValueDl { operand_enc, datatype_const, .. }
            | ProofInputs::FilterValueDlF64 { operand_enc, datatype_const, .. }
            | ProofInputs::FilterValueDlDecimal { operand_enc, datatype_const, .. } => {
                (operand_enc, datatype_const)
            }
            other => panic!("a numeric value-lane variant, got {other:?}"),
        };
        let field = |h: &FieldHex| h.to_field().expect("this crate emits canonical field hex");
        let (hook, lexical) = match &built.witness {
            LaneWitness::Hook(w) => (field(&w.value_hook), field(&w.lexical_component)),
            LaneWitness::Signed(w) => {
                let mag = field(&w.value_hook_scaled);
                (if w.value_neg { -mag } else { mag }, field(&w.lexical_component))
            }
        };
        let value_component =
            poseidon2::hash(&[hook, field(datatype_const), Fr::from(LANG_NONE)]);
        assert_eq!(
            poseidon2::hash(&[value_component, lexical, Fr::from(TYPE_CODE_LITERAL)]),
            field(operand_enc),
            "the witness must rebind to the presented operand_enc"
        );
    }
}
