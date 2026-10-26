//! CROSS-VECTORS for the `DualLeafV2` value lanes and the dedicated BIT-DOMAIN
//! `xsd:boolean` member (`filter_value_dl_bool`). Host half:
//! `sparq_zk::dual_leaf_v2`.
//!
//! # What V2 changes, and what these vectors pin
//!
//! A V2 lane constant is derived in BLAKE3's KEY-DERIVATION mode over the
//! length-framed `(datatype IRI, lane params)` pair, while a V2 string-lane leaf
//! keeps folding the PLAIN datatype hash. So the claim under test here is a
//! PROPERTY under BLAKE3's mode domain separation and — after the 248-bit field
//! reduction — its collision resistance: for every datatype BYTE STRING (valid
//! IRI or not), a lane constant differs from the plain fold, and two lanes with
//! different framed parts differ from each other. The host module tests that
//! property directly over a corpus of byte strings; what THIS file pins is the
//! consequence for the compose wiring: the constant the compose crate puts on the
//! wire is the one the committer folded into the leaf, and a leaf committed on one
//! lane does not rebind on another.
//!
//! # The dedicated member
//!
//! The V2 boolean lane targets its OWN member rather than sharing the integer one.
//! That member constrains BOTH the witnessed `VALUE_HOOK` and the public `bound`
//! to `{0, 1}` in-circuit, so a handle of `2` — which the shared integer member's
//! `u64` range decomposition admits — is refused by the RELATION, not only by the
//! host encoder. The in-circuit half of that claim needs real witness solving, so
//! it lives in the `nargo`-gated tests at the bottom, which SKIP CLEANLY when the
//! toolchain is absent (the established `e2e.rs` pattern).
//!
//! # Honest scope
//!
//! Nothing here is a soundness or privacy claim. The V2 value lanes inherit the
//! dual-leaf method's documented INV-VL downgrade (value↔lexical agreement on the
//! value-FILTER lane rests on TRUSTED-ISSUER HONESTY, not machine enforcement;
//! #769 accepted at research grade) and the whole ZK estate is NOT externally
//! audited (gap CR-G8 / sq-qhy4).
#![cfg(feature = "dual-leaf")]

use oxrdf::{Literal, NamedNode};
use sparq_zk::commit::{CommitmentMethod, MethodUse};
use sparq_zk::dual_leaf::{DualLeafError, LANG_NONE, XSD_DECIMAL, XSD_DOUBLE, XSD_INTEGER};
use sparq_zk::dual_leaf_boolean::XSD_BOOLEAN;
use sparq_zk::dual_leaf_v2;
use sparq_zk::encode::TYPE_CODE_LITERAL;
use sparq_zk::field::{field_from_hash_bytes, field_from_hex_str, field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk_compose::build::{boolean_verdict, build_filter_value_dl_bool};
use sparq_zk_compose::legacy_v1::{self, boolean_datatype_const, build_filter_value_dl_boolean};
use sparq_zk_compose::value_lane::{
    BooleanLane, HookWitness, IntegerLane, LaneWitness, ValueLane, ValueLaneProof,
};
use sparq_zk_compose::driver::{CircuitProver, DriverError};
use sparq_zk_compose::dispatch::{
    check_lane_const_for_member, check_lane_member_pairing, check_lane_member_pairing_for_use,
    resolve_circuit, resolve_circuit_for_use, DispatchError,
};
use sparq_zk_compose::manifest::{
    boolean_lane_const_v2, CircuitId, FieldHex, FilterOp, ProofInputs,
    ProofManifest,
};
use sparq_zk_compose::verifier::{
    prefilter_manifest_structure, CheckError, KeySet, RevocationPolicy,
};
use sparq_zk_compose::toml::{
    filter_value_dl_bool_prover_toml, filter_value_dl_datetime_prover_toml,
    filter_value_dl_decimal_prover_toml, filter_value_dl_f64_prover_toml,
    filter_value_dl_prover_toml,
};

fn typed(lexical: &str, datatype: &str) -> Literal {
    Literal::new_typed_literal(lexical, NamedNode::new(datatype).unwrap())
}

/// A typed literal over a datatype string that is NOT run through RFC 3987
/// validation; the V2 framing argument is stated over arbitrary byte strings.
fn typed_unchecked(lexical: &str, datatype: &str) -> Literal {
    Literal::new_typed_literal(lexical, NamedNode::new_unchecked(datatype))
}

fn bool_lit(v: bool) -> Literal {
    typed(if v { "true" } else { "false" }, XSD_BOOLEAN)
}

fn fr(hex: &FieldHex) -> Fr {
    field_from_hex_str(&hex.0).expect("field hex emitted by this crate must parse")
}

fn hexf(f: &Fr) -> FieldHex {
    FieldHex(field_to_hex(f))
}

/// A canonical challenge: every renderer and the checked prove entry refuse a
/// non-canonical `FieldHex`, the challenge included.
fn challenge_one() -> FieldHex {
    hexf(&Fr::from(1u64))
}

/// The member's OWN leaf recompute, transcribed from
/// `zk/compose/compose_core/src/filter_value_bool.nr::filter_value_dl_bool`
/// (identical to the integer member's):
/// `leaf = h3(h3(VALUE_HOOK, datatype_const, LANG_NONE), lexical_component,
/// TYPE_CODE_LITERAL)`. Taking the two PRIVATE witnesses and the PUBLIC
/// `datatype_const` and rebuilding the leaf is exactly what the in-circuit
/// `assert_eq(leaf, operand_enc)` does, so equality here is the host mirror of
/// "the member's binding constraint is satisfied".
fn member_rebinds_leaf(value_hook: Fr, datatype_const: Fr, lexical_component: Fr) -> Fr {
    let value_component = poseidon2::hash(&[value_hook, datatype_const, Fr::from(LANG_NONE)]);
    poseidon2::hash(&[
        value_component,
        lexical_component,
        Fr::from(TYPE_CODE_LITERAL),
    ])
}

/// The dedicated member's public inputs, borrowed. The V2 boolean lane must build
/// its OWN variant — that is part of the contract under test, so [`unpack`] panics
/// on anything else (notably on the V1 lane's shared `FilterValueDl`).
struct Public<'a> {
    id: &'a CircuitId,
    operand_enc: &'a FieldHex,
    op: FilterOp,
    bound: u64,
    datatype_const: &'a FieldHex,
    expected: bool,
}

fn unpack(inputs: &ProofInputs) -> Public<'_> {
    match inputs {
        ProofInputs::FilterValueDlBool {
            id,
            operand_enc,
            op,
            bound,
            datatype_const,
            expected,
        } => Public {
            id,
            operand_enc,
            op: *op,
            bound: *bound,
            datatype_const,
            expected: *expected,
        },
        other => panic!("the V2 boolean lane must build FilterValueDlBool inputs, got {other:?}"),
    }
}

// =========================================================================
// 1. The lane constant — the compose-side pick MUST equal the host encoder's
// =========================================================================

/// DRIFT GUARD. `manifest::boolean_lane_const_v2()` is what the compose crate puts
/// on the wire; `dual_leaf_v2::encode_boolean` is what the COMMITTER folded into
/// the leaf. If those diverge, every V2 boolean proof becomes silently unprovable
/// with no compile-time signal — so pin them to each other AND to the host's named
/// derivation.
#[test]
fn the_v2_lane_constant_equals_the_host_encoders_datatype_const() {
    let committed = dual_leaf_v2::encode_boolean(&bool_lit(true))
        .unwrap()
        .datatype_const;
    assert_eq!(fr(&boolean_lane_const_v2()), committed);
    assert_eq!(fr(&boolean_lane_const_v2()), dual_leaf_v2::boolean_lane_const());
    assert_eq!(
        fr(&boolean_lane_const_v2()),
        dual_leaf_v2::lane_const(XSD_BOOLEAN, dual_leaf_v2::LANE_PARAMS_BOOLEAN),
    );
}

/// The V2 boolean constant is not the V1 boolean constant, not the V2 string-lane
/// (plain) fold of the same IRI, and not another V2 lane's constant. These
/// inequalities are what the compose-side separation rests on; they hold under
/// BLAKE3's mode domain separation and, after the 248-bit reduction, its collision
/// resistance — not unconditionally.
#[test]
fn the_v2_lane_constant_is_separated_from_v1_and_from_the_other_lanes() {
    let v2 = fr(&boolean_lane_const_v2());
    assert_ne!(v2, fr(&boolean_datatype_const()), "V1 boolean sibling");
    assert_ne!(
        v2,
        dual_leaf_v2::degenerate_datatype_const(XSD_BOOLEAN),
        "the plain fold a V2 string-lane leaf carries"
    );
    for other in [
        dual_leaf_v2::integer_lane_const(),
        dual_leaf_v2::double_lane_const(),
        dual_leaf_v2::decimal_lane_const(0),
        dual_leaf_v2::decimal_lane_const(3),
        dual_leaf_v2::datetime_lane_const(),
        dual_leaf_v2::date_lane_const(),
    ] {
        assert_ne!(v2, other);
    }
}

// =========================================================================
// 2. encode_boolean -> member accepts (the binding constraint is satisfied)
// =========================================================================

/// CROSS-VECTOR: for both boolean terms, the leaf `dual_leaf_v2::encode_boolean`
/// committed is EXACTLY the leaf the member rebuilds from the witnesses + the
/// public `datatype_const` the builder emitted. This is the in-circuit
/// `assert_eq(leaf, operand_enc)` evaluated host-side.
#[test]
fn v2_boolean_witnesses_rebind_to_the_committed_operand_enc() {
    for value in [false, true] {
        let lit = bool_lit(value);
        let committed = dual_leaf_v2::encode_boolean(&lit).unwrap();
        let built = build_filter_value_dl_bool(&lit, FilterOp::Eq, true).unwrap();
        let p = unpack(&built.inputs);

        // The V2 lane targets its OWN compiled member.
        assert_eq!(p.id, &CircuitId::FilterValueDlBool);
        assert_eq!(p.id.package().unwrap(), "filter_value_dl_bool");

        // The public anchor is the committed dual leaf.
        assert_eq!(fr(p.operand_enc), committed.leaf());
        // The private witnesses are the committer's own components.
        assert_eq!(fr(&built.value_hook), committed.value_hook);
        assert_eq!(fr(&built.lexical_component), committed.lexical_component);
        // The handle is in the bit domain the member constrains: 1 for true, 0 for
        // false.
        assert_eq!(fr(&built.value_hook), Fr::from(u64::from(value)));

        // The member's binding constraint holds.
        assert_eq!(
            member_rebinds_leaf(
                fr(&built.value_hook),
                fr(p.datatype_const),
                fr(&built.lexical_component)
            ),
            fr(p.operand_enc),
        );
    }
}

// =========================================================================
// 3. LANE SEPARATION across the V2 lanes, and against the V2 string lane
// =========================================================================

/// The load-bearing separation, both directions: an honest witness for one V2
/// lane's committed leaf does NOT satisfy another lane's member call, because the
/// public `datatype_const` it must fold in is different. `"1"^^xsd:integer` and
/// `"true"^^xsd:boolean` are the pair that matters — they share the value hook
/// `1`, so ONLY the lane constant separates them.
#[test]
fn a_v2_leaf_on_one_lane_does_not_rebind_on_another() {
    let bool_c = dual_leaf_v2::encode_boolean(&bool_lit(true)).unwrap();
    let int_c = dual_leaf_v2::encode_integer(&typed("1", XSD_INTEGER)).unwrap();

    // Same VALUE_HOOK — the case this test exists for.
    assert_eq!(bool_c.value_hook, int_c.value_hook);
    assert_eq!(bool_c.value_hook, Fr::from(1u64));
    assert_ne!(bool_c.datatype_const, int_c.datatype_const);
    assert_ne!(bool_c.value_component(), int_c.value_component());
    assert_ne!(bool_c.leaf(), int_c.leaf());

    // (a) an integer leaf presented to a BOOLEAN member call.
    assert_ne!(
        member_rebinds_leaf(
            int_c.value_hook,
            fr(&boolean_lane_const_v2()),
            int_c.lexical_component
        ),
        int_c.leaf(),
    );
    // (b) a boolean leaf presented to an INTEGER member call: symmetric.
    assert_ne!(
        member_rebinds_leaf(
            bool_c.value_hook,
            dual_leaf_v2::integer_lane_const(),
            bool_c.lexical_component
        ),
        bool_c.leaf(),
    );

    // Sanity that (a)/(b) are NOT vacuous: each witness set DOES rebind under its
    // OWN lane constant. If the recompute were broken these would fail too.
    assert_eq!(
        member_rebinds_leaf(
            int_c.value_hook,
            int_c.datatype_const,
            int_c.lexical_component
        ),
        int_c.leaf(),
    );
    assert_eq!(
        member_rebinds_leaf(
            bool_c.value_hook,
            bool_c.datatype_const,
            bool_c.lexical_component
        ),
        bool_c.leaf(),
    );
}

/// Every hookable V2 lane, each against every OTHER lane's constant: an honest
/// witness rebinds under its own lane constant and under no other. Covers the
/// parameterised lanes (`decimal_lane_const(fd)`, the epoch-scaled dateTime/date
/// constants) as well as the unparameterised ones.
#[test]
fn every_v2_value_lane_rebinds_under_its_own_constant_and_no_other() {
    let lanes: Vec<(&str, _)> = vec![
        (
            "integer",
            dual_leaf_v2::encode_integer(&typed("7", XSD_INTEGER)).unwrap(),
        ),
        (
            "double",
            dual_leaf_v2::encode_double(&typed("7.0E0", XSD_DOUBLE)).unwrap(),
        ),
        (
            "decimal",
            dual_leaf_v2::encode_decimal(&typed("7.25", XSD_DECIMAL)).unwrap(),
        ),
        ("boolean", dual_leaf_v2::encode_boolean(&bool_lit(true)).unwrap()),
        (
            "dateTime",
            dual_leaf_v2::encode_datetime(&typed(
                "2024-01-01T00:00:00Z",
                "http://www.w3.org/2001/XMLSchema#dateTime",
            ))
            .unwrap(),
        ),
        (
            "date",
            dual_leaf_v2::encode_date(&typed(
                "2024-01-01Z",
                "http://www.w3.org/2001/XMLSchema#date",
            ))
            .unwrap(),
        ),
    ];
    for (name, c) in &lanes {
        assert_eq!(
            member_rebinds_leaf(c.value_hook, c.datatype_const, c.lexical_component),
            c.leaf(),
            "{} must rebind under its own lane constant",
            name
        );
        for (other_name, other) in &lanes {
            if other.datatype_const == c.datatype_const {
                continue;
            }
            assert_ne!(
                member_rebinds_leaf(c.value_hook, other.datatype_const, c.lexical_component),
                c.leaf(),
                "{} must not rebind under the {} lane constant",
                name,
                other_name
            );
        }
    }
}

/// A V2 STRING-LANE leaf (the unrouted path: any non-hookable datatype, including
/// datatype strings that are not valid IRIs) carries the PLAIN datatype fold in
/// its degenerate value component, so it does not rebind under any V2 lane
/// constant. Iterated over opaque datatype byte strings built WITHOUT IRI
/// validation.
#[test]
fn a_v2_string_lane_literal_is_separated_from_every_value_lane() {
    let opaque = [
        "http://example.org/opaque",
        "http://www.w3.org/2001/XMLSchema#boolean@=bit",
        "http://www.w3.org/2001/XMLSchema#boolean%20bit",
        "http://example.org/dt x",
        "",
        "\0bit",
        dual_leaf_v2::LANE_CONTEXT,
    ];
    let lane_constants = [
        dual_leaf_v2::integer_lane_const(),
        dual_leaf_v2::double_lane_const(),
        dual_leaf_v2::decimal_lane_const(2),
        dual_leaf_v2::boolean_lane_const(),
        dual_leaf_v2::datetime_lane_const(),
        dual_leaf_v2::date_lane_const(),
    ];
    for datatype in opaque {
        assert!(
            !dual_leaf_v2::is_hookable_datatype(datatype),
            "{} must take the string lane",
            datatype
        );
        // The string lane commits the DEGENERATE value component over the PLAIN
        // fold; a value-lane member recomputes `h3(hook, lane_const, LANG_NONE)`.
        // Those differ, so the string-lane leaf is not a value-lane leaf — for
        // either boolean handle, and for every lane constant.
        let lit = typed_unchecked("true", datatype);
        let string_lane_leaf = dual_leaf_v2::encode_literal(&lit).unwrap();
        let degenerate = dual_leaf_v2::degenerate_value_component(datatype, None).unwrap();
        let plain = dual_leaf_v2::degenerate_datatype_const(datatype);
        for lane in lane_constants {
            assert_ne!(plain, lane, "{}", datatype);
            for hook in [Fr::from(0u64), Fr::from(1u64)] {
                let value_component = poseidon2::hash(&[hook, lane, Fr::from(LANG_NONE)]);
                assert_ne!(value_component, degenerate, "{}", datatype);
            }
        }
        // The leaf-level consequence, in-circuit, is covered by the `nargo`-gated
        // opaque-datatype vectors below; here pin that the string-lane leaf is
        // well-defined and does NOT equal the V2 boolean lane's leaf for the same
        // lexical.
        assert_ne!(
            string_lane_leaf,
            dual_leaf_v2::encode_boolean(&bool_lit(true)).unwrap().leaf(),
            "{}",
            datatype
        );
    }
}

// =========================================================================
// 4. The verdict table — the builder discloses the HONEST verdict
// =========================================================================

/// The full `(value, op, bound)` truth table written out LONGHAND — deliberately
/// not a re-derivation of `boolean_verdict`'s own formula, so a mutation to that
/// function (or to the member's `bit_verdict`) goes RED here instead of being
/// mirrored.
const VERDICTS: [(bool, FilterOp, bool, bool); 24] = [
    // false <op> false
    (false, FilterOp::Lt, false, false),
    (false, FilterOp::Le, false, true),
    (false, FilterOp::Gt, false, false),
    (false, FilterOp::Ge, false, true),
    (false, FilterOp::Eq, false, true),
    (false, FilterOp::Ne, false, false),
    // false <op> true   — the degenerate ordering: false < true
    (false, FilterOp::Lt, true, true),
    (false, FilterOp::Le, true, true),
    (false, FilterOp::Gt, true, false),
    (false, FilterOp::Ge, true, false),
    (false, FilterOp::Eq, true, false),
    (false, FilterOp::Ne, true, true),
    // true <op> false
    (true, FilterOp::Lt, false, false),
    (true, FilterOp::Le, false, false),
    (true, FilterOp::Gt, false, true),
    (true, FilterOp::Ge, false, true),
    (true, FilterOp::Eq, false, false),
    (true, FilterOp::Ne, false, true),
    // true <op> true
    (true, FilterOp::Lt, true, false),
    (true, FilterOp::Le, true, true),
    (true, FilterOp::Gt, true, false),
    (true, FilterOp::Ge, true, true),
    (true, FilterOp::Eq, true, true),
    (true, FilterOp::Ne, true, false),
];

/// The builder discloses the HONEST verdict for every op × term × bound, and
/// carries the V2 lane constant and the `{0, 1}` bound hook.
#[test]
fn the_v2_builder_discloses_the_honest_verdict_for_every_op() {
    for (value, op, bound, expected) in VERDICTS {
        let built = build_filter_value_dl_bool(&bool_lit(value), op, bound).unwrap();
        let p = unpack(&built.inputs);
        assert_eq!(p.op, op);
        assert_eq!(p.bound, u64::from(bound));
        assert_eq!(fr(p.datatype_const), dual_leaf_v2::boolean_lane_const());
        assert_eq!(p.expected, expected, "{value:?} {op:?} {bound:?}");
        assert_eq!(boolean_verdict(value, op, bound), expected);
    }
}

// =========================================================================
// 5. Fail-closed inputs + fail-closed dispatch
// =========================================================================

/// The §6 co-binding is not bypassed by the compose-side V2 builder: the
/// non-canonical XSD-legal spellings and every non-boolean datatype are refused
/// with the encoder's own error, never a silently desynced leaf.
#[test]
fn non_canonical_and_non_boolean_literals_are_fail_closed_at_the_v2_builder() {
    for lex in ["1", "0", "True", " true"] {
        assert!(matches!(
            build_filter_value_dl_bool(&typed(lex, XSD_BOOLEAN), FilterOp::Eq, true),
            Err(DualLeafError::NonCanonicalValue(_)),
        ));
    }
    assert!(matches!(
        build_filter_value_dl_bool(&typed("1", XSD_INTEGER), FilterOp::Eq, true),
        Err(DualLeafError::NotValueLane(_)),
    ));
    assert!(matches!(
        build_filter_value_dl_bool(&Literal::new_simple_literal("true"), FilterOp::Eq, true),
        Err(DualLeafError::NotValueLane(_)),
    ));
}

/// The dedicated member sits on the value lane of the fail-closed
/// `(method × circuit)` legality matrix: V2 admits it, and a
/// `string-canonical` graph — which committed no value handle at all — refuses it.
/// V1 value members have an explicit refusal independently of method policy.
#[test]
fn the_v2_boolean_member_inherits_the_value_lane_dispatch_legality() {
    let id = CircuitId::FilterValueDlBool;
    assert_eq!(
        resolve_circuit(CommitmentMethod::DualLeafV2, &id),
        Ok(id.clone())
    );
    assert_eq!(
        resolve_circuit(CommitmentMethod::DualLeafV1, &id),
        Err(DispatchError::LegacyV1ValueMember { circuit: id.package().unwrap() })
    );
    assert!(matches!(
        resolve_circuit(CommitmentMethod::StringCanonicalV1, &id),
        Err(DispatchError::IllegalPair { .. }),
    ));
}

/// V2 is the dual-leaf method selectable for new issuance; V1 stays recognised and
/// verifiable but is not selectable.
#[test]
fn v2_is_the_selectable_dual_leaf_method() {
    assert!(CommitmentMethod::DualLeafV2.is_production_selectable());
    assert!(!CommitmentMethod::DualLeafV1.is_production_selectable());
    assert_eq!(
        CommitmentMethod::from_scheme_iri(CommitmentMethod::DualLeafV1.scheme_iri()),
        Some(CommitmentMethod::DualLeafV1),
        "V1 must stay recognised so existing credentials still verify"
    );
}

// =========================================================================
// 6. Prover.toml — the integer member's layout, with the V2 constant PINNED
// =========================================================================

/// The dedicated renderer is the shared value-lane renderer with the V2 lane
/// constant PINNED and the bound mapped to its hook, so its bytes must equal the
/// shared renderer's when handed those values — the layout cannot drift from the
/// member's `main`.
#[test]
fn the_v2_prover_toml_pins_the_lane_constant_and_keeps_the_member_layout() {
    let built = build_filter_value_dl_bool(&bool_lit(true), FilterOp::Ge, false).unwrap();
    let p = unpack(&built.inputs);
    let challenge = challenge_one();

    let dedicated = filter_value_dl_bool_prover_toml(
        &challenge,
        p.operand_enc,
        p.op.code(),
        false,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the pinned V2 boolean constant rides its own member");
    // The layout is the integer member's, byte for byte.
    assert_eq!(
        dedicated,
        raw_value_lane_toml(
            &challenge,
            p.operand_enc,
            p.op.code(),
            p.bound,
            p.datatype_const,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        )
    );
    // And the SHARED (integer-member) renderer REFUSES those same values, because
    // the constant is the V2 boolean lane's: the refusal happens before any text
    // is rendered, so no Prover.toml for the crossed pairing exists.
    let crossed = filter_value_dl_prover_toml(
        &challenge,
        p.operand_enc,
        p.op.code(),
        p.bound,
        p.datatype_const,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    );
    assert!(matches!(crossed, Err(DispatchError::LaneMemberMismatch { .. })), "{:?}", crossed);

    let lines: Vec<&str> = dedicated.lines().collect();
    assert!(lines[0].starts_with("challenge = "));
    assert!(lines[1].starts_with("operand_enc = "));
    assert_eq!(lines[2], "op = \"3\"");
    assert_eq!(lines[3], "bound = \"0\"");
    assert_eq!(
        lines[4],
        format!("datatype_const = \"{}\"", boolean_lane_const_v2().0)
    );
    assert_eq!(lines[5], "expected = true");
    assert!(lines[6].starts_with("value_hook = "));
    assert!(lines[7].starts_with("lexical_component = "));

    // The rendered constant is the V2 boolean one — a V1-lane render differs, and
    // is available only under the explicit legacy NAME.
    let v1_lane = legacy_v1::filter_value_dl_prover_toml(
        &challenge,
        p.operand_enc,
        p.op.code(),
        p.bound,
        &boolean_datatype_const(),
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the V1 boolean constant rides the shared integer member, legacy name");
    assert_ne!(dedicated, v1_lane);
    assert!(matches!(
        filter_value_dl_prover_toml(
            &challenge,
            p.operand_enc,
            p.op.code(),
            p.bound,
            &boolean_datatype_const(),
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));
}

/// `bound = true` renders the hook `1` (the other half of the `{0, 1}` mapping).
#[test]
fn the_v2_prover_toml_renders_the_true_bound_hook() {
    let built = build_filter_value_dl_bool(&bool_lit(true), FilterOp::Eq, true).unwrap();
    let p = unpack(&built.inputs);
    let toml = filter_value_dl_bool_prover_toml(
        &challenge_one(),
        p.operand_enc,
        p.op.code(),
        true,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the pinned V2 boolean constant rides its own member");
    assert!(toml.contains("bound = \"1\"\n"));
}

/// The dedicated member gets its OWN wire tag, so a V1 boolean proof (shared
/// `filter_value_dl` tag) and a V2 one are never confused on the wire.
#[test]
fn the_v2_boolean_inputs_round_trip_on_their_own_wire_tag() {
    let built = build_filter_value_dl_bool(&bool_lit(false), FilterOp::Ne, true).unwrap();
    let json = serde_json::to_string(&built.inputs).unwrap();
    assert!(json.contains("\"circuit\":\"filter_value_dl_bool\""));
    assert!(json.contains(&boolean_lane_const_v2().0));
    let back: ProofInputs = serde_json::from_str(&json).unwrap();
    assert_eq!(back, built.inputs);
    assert_eq!(back.circuit_id(), &CircuitId::FilterValueDlBool);
}

// =========================================================================
// 7. IN-CIRCUIT vectors — real witness solving. Skip cleanly with no toolchain.
// =========================================================================

/// The value-lane `Prover.toml` layout, rendered LOCALLY.
///
/// The library renderers now refuse a `(member, datatype_const)` pairing the lane
/// table does not allow, which is the host boundary this suite also tests. The
/// IN-CIRCUIT vectors below must be able to hand a member a pairing that boundary
/// refuses — that is how they measure what the RELATION does rather than what the
/// host permits — so they render the bytes here instead. The field order is the
/// member `main`'s, and
/// `the_v2_prover_toml_pins_the_lane_constant_and_keeps_the_member_layout` pins
/// these bytes against the library renderer for an ALLOWED pairing.
#[allow(clippy::too_many_arguments)]
fn raw_value_lane_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> String {
    format!(
        "challenge = \"{}\"\noperand_enc = \"{}\"\nop = \"{}\"\nbound = \"{}\"\n\
         datatype_const = \"{}\"\nexpected = {}\nvalue_hook = \"{}\"\n\
         lexical_component = \"{}\"\n",
        challenge.0,
        operand_enc.0,
        op,
        bound,
        datatype_const.0,
        expected,
        value_hook.0,
        lexical_component.0,
    )
}

mod support;

/// Solve the given member's witness for the given public/private assignment,
/// using the shared value-lane renderer (whose layout the dedicated member
/// reuses). `Ok` ⇔ the relation is SATISFIABLE ⇔ the member ACCEPTS.
#[allow(clippy::too_many_arguments)]
fn member_accepts(
    id: &CircuitId,
    tag: &str,
    operand_enc: &FieldHex,
    op: FilterOp,
    bound: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> bool {
    let toml = raw_value_lane_toml(
        &challenge_one(),
        operand_enc,
        op.code(),
        bound,
        datatype_const,
        expected,
        value_hook,
        lexical_component,
    );
    sparq_zk_compose::driver::CircuitProver::from_crate_root()
        .gen_witness_tagged(id, &toml, tag)
        .is_ok()
}

/// The honest V2 boolean witness is provable on the dedicated member, and a LYING
/// disclosed verdict is not.
#[test]
fn in_circuit_the_v2_boolean_member_accepts_honest_and_rejects_a_lying_verdict() {
    if !support::require_toolchain("in_circuit_the_v2_boolean_member_accepts_honest_and_rejects_a_lying_verdict", &["nargo"]) {
        return;
    }
    let built = build_filter_value_dl_bool(&bool_lit(true), FilterOp::Gt, false).unwrap();
    let p = unpack(&built.inputs);
    assert!(p.expected, "true > false is the honest verdict");

    assert!(
        member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_ok",
            p.operand_enc,
            p.op,
            p.bound,
            p.datatype_const,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        "the honest V2 boolean witness must be provable on filter_value_dl_bool"
    );
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_lie",
            p.operand_enc,
            p.op,
            p.bound,
            p.datatype_const,
            !p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        "a flipped disclosed verdict must be unprovable"
    );
    // The dedicated renderer's bytes are what the honest case was solved from.
    assert_eq!(
        filter_value_dl_bool_prover_toml(
            &challenge_one(),
            p.operand_enc,
            p.op.code(),
            false,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        )
        .expect("the pinned V2 boolean constant rides its own member"),
        raw_value_lane_toml(
            &challenge_one(),
            p.operand_enc,
            p.op.code(),
            p.bound,
            p.datatype_const,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
    );
}

/// THE BIT DOMAIN — the reason this member exists. A handle (and a bound) of `2`
/// is inside the shared integer member's `u64` range decomposition, and a leaf
/// committed for it satisfies that member's binding constraint, so the integer
/// member ACCEPTS the assignment. The dedicated member REFUSES it.
///
/// Note what this vector does and does not attribute: on the dedicated member the
/// assignment is refused, but TWO of its constraints independently refuse it —
/// `assert_bit(value_hook)`, and the leaf binding (the member rebinds the leaf from
/// the canonicalised BIT, not from the raw handle, so a leaf committed for `2`
/// cannot match). The bit constraint is isolated by
/// [`in_circuit_the_hook_bit_constraint_is_what_refuses_an_otherwise_binding_witness`]
/// below. The `bound` half here IS isolated: the handle is honest and the leaf
/// binds, so only `assert_bit(bound)` can refuse.
#[test]
fn in_circuit_the_dedicated_member_refuses_a_handle_outside_the_bit_domain() {
    if !support::require_toolchain("in_circuit_the_dedicated_member_refuses_a_handle_outside_the_bit_domain", &["nargo"]) {
        return;
    }
    let dt = boolean_lane_const_v2();
    let lexical = hexf(&Fr::from(0xdefu64));
    // The leaf an (impossible) committer of the handle 2 would have produced, so
    // the binding constraint is satisfied and ONLY the bit constraint can refuse.
    let enc_two = hexf(&member_rebinds_leaf(Fr::from(2u64), fr(&dt), fr(&lexical)));
    let hook_two = hexf(&Fr::from(2u64));

    // Control: the SHARED integer member accepts the very same assignment (2 > 1).
    assert!(
        member_accepts(
            &CircuitId::FilterValueDl,
            "dlv2_two_int",
            &enc_two,
            FilterOp::Gt,
            1,
            &dt,
            true,
            &hook_two,
            &lexical,
        ),
        "control: the integer member's u64 range admits the handle 2"
    );
    // The dedicated member refuses that assignment (by either of its two
    // constraints — see the doc comment; the isolating vector is below).
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_two_bool",
            &enc_two,
            FilterOp::Gt,
            1,
            &dt,
            true,
            &hook_two,
            &lexical,
        ),
        "a handle outside {{0, 1}} must be unprovable on filter_value_dl_bool"
    );
    // ...and the public bound, independently (an honest handle, bound 2).
    let built = build_filter_value_dl_bool(&bool_lit(true), FilterOp::Eq, true).unwrap();
    let p = unpack(&built.inputs);
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_bound_two",
            p.operand_enc,
            FilterOp::Eq,
            2,
            p.datatype_const,
            false,
            &built.value_hook,
            &built.lexical_component,
        ),
        "a public bound outside {{0, 1}} must be unprovable"
    );
}

/// IN-CIRCUIT lane separation, over several datatype byte strings built WITHOUT
/// IRI validation.
/// The honest V2 boolean witnesses are presented under each opaque datatype's
/// PLAIN fold — a V2 string-lane constant — and the member refuses every one,
/// while the control under the real lane constant is accepted.
#[test]
fn in_circuit_the_v2_boolean_witness_does_not_open_under_an_opaque_datatype_constant() {
    if !support::require_toolchain("in_circuit_the_v2_boolean_witness_does_not_open_under_an_opaque_datatype_constant", &["nargo"]) {
        return;
    }
    let built = build_filter_value_dl_bool(&bool_lit(true), FilterOp::Eq, true).unwrap();
    let p = unpack(&built.inputs);
    assert!(
        member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_lane_ok",
            p.operand_enc,
            p.op,
            p.bound,
            p.datatype_const,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        "control: the honest witness IS provable under its own lane constant"
    );

    let opaque = [
        ("plain_bool", XSD_BOOLEAN),
        ("suffix_at", "http://www.w3.org/2001/XMLSchema#boolean@=bit"),
        ("suffix_pct", "http://www.w3.org/2001/XMLSchema#boolean%20bit"),
        ("space", "http://example.org/dt x"),
        ("empty", ""),
        ("nul", "\0bit"),
        ("context", dual_leaf_v2::LANE_CONTEXT),
    ];
    for (tag, datatype) in opaque {
        // The V2 string lane's constant for this datatype: the PLAIN fold, which
        // the key-derivation-mode lane constant is separated from.
        let plain = hexf(&dual_leaf_v2::degenerate_datatype_const(datatype));
        assert_ne!(plain.0, p.datatype_const.0, "{}", datatype);
        assert!(
            !member_accepts(
                &CircuitId::FilterValueDlBool,
                &format!("dlv2_opaque_{}", tag),
                p.operand_enc,
                p.op,
                p.bound,
                &plain,
                p.expected,
                &built.value_hook,
                &built.lexical_component,
            ),
            "the honest witness must be unprovable under the {} string-lane constant",
            datatype
        );
    }
}

/// ISOLATES `assert_bit(value_hook)`. The member rebinds its leaf from the
/// CANONICALISED bit (`value_hook == 1`), so a witness whose committed leaf was
/// built for the bit `0` SATISFIES the binding constraint even when the witnessed
/// handle is `2` — and the disclosed verdict is the honest one for `false`. Every
/// constraint except the hook's bit constraint is therefore met, so this vector is
/// refused by `assert_bit(value_hook)` and by nothing else.
///
/// Deleting that one assert in `filter_value_bool.nr` makes this test pass, which
/// is what makes it a guard rather than a restatement.
#[test]
fn in_circuit_the_hook_bit_constraint_is_what_refuses_an_otherwise_binding_witness() {
    if !support::require_toolchain("in_circuit_the_hook_bit_constraint_is_what_refuses_an_otherwise_binding_witness", &["nargo"]) {
        return;
    }
    // An honest committed `false` leaf and its honest public inputs.
    let built = build_filter_value_dl_bool(&bool_lit(false), FilterOp::Eq, false).unwrap();
    let p = unpack(&built.inputs);
    assert!(p.expected, "false == false is the honest verdict");
    assert_eq!(fr(&built.value_hook), Fr::from(0u64));

    // Control: the honest witness is provable.
    assert!(
        member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_bit_iso_ok",
            p.operand_enc,
            p.op,
            p.bound,
            p.datatype_const,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        "control: the honest false witness must be provable"
    );

    // The ONLY change is the witnessed handle, from `0` to `2`. The member reads
    // `value = (2 == 1) = false`, rebinds the leaf from the bit `0` — which still
    // equals `operand_enc` — and computes the same verdict. The host mirror of that
    // rebinding, asserted so the vector cannot silently stop being isolating:
    let hook_two = hexf(&Fr::from(2u64));
    assert_eq!(
        member_rebinds_leaf(
            Fr::from(0u64),
            fr(p.datatype_const),
            fr(&built.lexical_component)
        ),
        fr(p.operand_enc),
        "the leaf the member rebinds for a handle of 2 must still be operand_enc, \
         otherwise this vector is not isolating the bit constraint"
    );
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_bit_iso_two",
            p.operand_enc,
            p.op,
            p.bound,
            p.datatype_const,
            p.expected,
            &hook_two,
            &built.lexical_component,
        ),
        "a witnessed handle of 2 must be refused even though the leaf binding and \
         the verdict both hold"
    );
}

/// What actually keeps a V2 STRING-LANE leaf off the value lanes: the handle, not
/// the lane constant. `datatype_const` is an UNPINNED public input, so this vector
/// presents a real V2 string-lane leaf together with the constant it was committed
/// under (the PLAIN datatype fold) and the handle a degenerate `value_component`
/// carries (`value_none`) — the most favourable presentation available to a prover.
/// Both members refuse it, because `value_none` lies outside the reachable handle
/// band: the integer member's `u64` range decomposition rejects it, and the
/// dedicated boolean member's `{0, 1}` constraint rejects it.
#[test]
fn in_circuit_a_v2_string_lane_leaf_is_refused_on_the_value_lane_members() {
    if !support::require_toolchain("in_circuit_a_v2_string_lane_leaf_is_refused_on_the_value_lane_members", &["nargo"]) {
        return;
    }
    // A real V2 string-lane literal: an opaque datatype, so `encode_literal` takes
    // the degenerate path.
    let datatype = "http://example.org/opaque";
    assert!(!dual_leaf_v2::is_hookable_datatype(datatype));
    let lit = typed_unchecked("some value", datatype);
    let committed_leaf = dual_leaf_v2::encode_literal(&lit).unwrap();

    // Reconstruct that leaf's parts: the degenerate value component (which folds
    // `value_none` and the PLAIN datatype fold) and the string-canonical lexical
    // hash. Equality with the committed leaf is what makes this the REAL leaf
    // rather than a leaf-shaped construction.
    let lexical_component =
        field_from_hash_bytes(blake3::hash(lit.to_string().as_bytes()).as_bytes());
    let degenerate = dual_leaf_v2::degenerate_value_component(datatype, None).unwrap();
    assert_eq!(
        poseidon2::hash(&[
            degenerate,
            lexical_component,
            Fr::from(TYPE_CODE_LITERAL)
        ]),
        committed_leaf,
        "the reconstructed parts must be the committed string-lane leaf"
    );
    // The handle that component carries is out of band, which is the whole point.
    assert!(!dual_leaf_v2::is_in_handle_band(&dual_leaf_v2::value_none()));

    let enc = hexf(&committed_leaf);
    let hook = hexf(&dual_leaf_v2::value_none());
    let plain = hexf(&dual_leaf_v2::degenerate_datatype_const(datatype));
    let lexical = hexf(&lexical_component);

    // The shared integer member: the leaf binding is the member's own recompute
    // over the range-decomposed handle, so the `u64` range constraint is what
    // refuses this.
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDl,
            "dlv2_strlane_int",
            &enc,
            FilterOp::Eq,
            0,
            &plain,
            true,
            &hook,
            &lexical,
        ),
        "a string-lane leaf must be unprovable on the integer value-lane member"
    );
    // The dedicated boolean member: the `{0, 1}` constraint refuses it.
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_strlane_bool",
            &enc,
            FilterOp::Eq,
            0,
            &plain,
            true,
            &hook,
            &lexical,
        ),
        "a string-lane leaf must be unprovable on the bit-domain boolean member"
    );
}

/// The decimal and dateTime members cannot even be ASKED the question above: their
/// ABI takes the handle as `(value_neg: bool, value_hook_scaled: u64)`, and the set
/// of field elements such a pair can denote — `mag` or its field negation, for
/// `mag: u64` — is EXACTLY the reachable handle band. So an out-of-band handle is
/// not expressible through those members' public interface, and `value_none` is
/// out of band. Host-side statement; no toolchain needed.
#[test]
fn the_sign_folded_members_cannot_express_an_out_of_band_handle() {
    let sentinel = dual_leaf_v2::value_none();
    assert!(!dual_leaf_v2::is_in_handle_band(&sentinel));
    // Every `(neg, u64)` pair lands inside the band, by construction...
    for mag in [0u64, 1, 2, u64::MAX, u64::MAX / 2] {
        for neg in [false, true] {
            let folded = if neg { -Fr::from(mag) } else { Fr::from(mag) };
            assert!(
                dual_leaf_v2::is_in_handle_band(&folded),
                "({}, {}) must be in band",
                neg,
                mag
            );
            // ...so none of them is the sentinel.
            assert_ne!(folded, sentinel);
        }
    }
}

// =========================================================================
// 8. The issuance boundary and the lane↔member pairing
// =========================================================================

/// Fresh V1 issuance and legacy V1 value members have separate named refusals.
/// V2 remains the positive control for each value member.
#[test]
fn fresh_and_legacy_v1_value_routing_have_named_refusals() {
    for id in [CircuitId::FilterValueDl, CircuitId::FilterValueDlBool] {
        let err = resolve_circuit_for_use(
            CommitmentMethod::DualLeafV1,
            &id,
            MethodUse::NewIssuance,
        )
        .expect_err("a fresh proof under a superseded method must be refused");
        assert!(matches!(
            err,
            DispatchError::MethodNotSelectableForIssuance { .. }
        ));
        assert!(err.to_string().contains(CommitmentMethod::DualLeafV1.scheme_iri()));
        // Legacy use cannot admit a V1 value member.
        assert_eq!(
            resolve_circuit_for_use(
                CommitmentMethod::DualLeafV1,
                &id,
                MethodUse::LegacyAlreadyIssued
            ),
            Err(DispatchError::LegacyV1ValueMember { circuit: id.package().unwrap() })
        );
        // And V2 is selectable for new issuance.
        assert_eq!(
            resolve_circuit_for_use(CommitmentMethod::DualLeafV2, &id, MethodUse::NewIssuance),
            Ok(id.clone())
        );
    }
}

/// A V1 boolean filter is built only by its LEGACY name, and the checked prove
/// entry refuses every method/use combination, including an already-issued V1
/// graph. The ordinary V2 builder remains the honest control.
#[test]
fn a_v1_boolean_filter_has_a_legacy_builder_but_no_checked_prove() {
    let lit = typed("true", XSD_BOOLEAN);
    let built = build_filter_value_dl_boolean(&lit, FilterOp::Eq, true)
        .expect("the legacy name builds a V1 boolean proof");
    assert_eq!(built.inputs.circuit_id(), &CircuitId::FilterValueDl);
    // The ORDINARY boolean builder is the V2 one: its own member, its own constant.
    let v2 = build_filter_value_dl_bool(&lit, FilterOp::Eq, true).unwrap();
    assert_eq!(v2.inputs.circuit_id(), &CircuitId::FilterValueDlBool);

    let proof = built.lane_proof().expect("the V1 build assembles typed inputs");
    assert_eq!(proof.lane(), ValueLane::Integer);
    let c = challenge_one();
    let at = |m, u| proof.checked_prover_toml(m, u, &c);
    assert!(matches!(
        at(CommitmentMethod::DualLeafV1, MethodUse::LegacyAlreadyIssued),
        Err(DispatchError::LegacyV1ValueMember { .. })
    ));
    assert!(matches!(
        at(CommitmentMethod::DualLeafV1, MethodUse::NewIssuance),
        Err(DispatchError::MethodNotSelectableForIssuance { .. })
    ));
    assert!(matches!(
        at(CommitmentMethod::DualLeafV2, MethodUse::NewIssuance),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));
    assert!(matches!(
        at(CommitmentMethod::DualLeafV2, MethodUse::LegacyAlreadyIssued),
        Err(DispatchError::MethodLaneVersionMismatch { .. })
    ));
    assert!(matches!(
        at(CommitmentMethod::StringCanonicalV1, MethodUse::LegacyAlreadyIssued),
        Err(DispatchError::IllegalPair { .. })
    ));
    // A V2 lane under the V1 method is refused at dispatch before version checks.
    let v2_proof = v2.lane_proof().unwrap();
    assert!(v2_proof
        .checked_prover_toml(CommitmentMethod::DualLeafV2, MethodUse::NewIssuance, &c)
        .is_ok());
    assert!(matches!(
        v2_proof.checked_prover_toml(
            CommitmentMethod::DualLeafV1,
            MethodUse::LegacyAlreadyIssued,
            &c
        ),
        Err(DispatchError::LegacyV1ValueMember { .. })
    ));
}

/// A V2 boolean lane constant may ONLY ride the bit-constrained member. On the
/// shared integer member — whose relation range-decomposes the handle over the
/// whole `u64` domain — the pairing is refused, by the host boundary and by the
/// verifier's structural pre-filter. `datatype_const` is a public input the member
/// does not pin, which is exactly why the refusal lives here.
#[test]
fn the_v2_boolean_lane_constant_is_refused_on_the_shared_integer_member() {
    let built = build_filter_value_dl_bool(&typed("true", XSD_BOOLEAN), FilterOp::Eq, true)
        .expect("the V2 boolean builder accepts a canonical literal");
    let ProofInputs::FilterValueDlBool { operand_enc, op, bound, datatype_const, expected, .. } =
        built.inputs.clone()
    else {
        panic!("the V2 boolean builder emits its own variant");
    };
    assert_eq!(datatype_const, boolean_lane_const_v2());
    // The honest pairing passes.
    assert_eq!(check_lane_member_pairing(&built.inputs), Ok(()));

    // The same lane constant re-presented on the integer member is refused.
    let crossed = ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc,
        op,
        bound,
        datatype_const,
        expected,
    };
    let err = check_lane_member_pairing(&crossed).expect_err("the crossed pairing must refuse");
    assert!(matches!(err, DispatchError::LaneMemberMismatch { .. }));
    assert!(err.to_string().contains("filter_value_dl_int"));

    // The integer lane's own constant on the integer member stays legal, so the
    // check refuses the pairing and not the member.
    let integer_lane = ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc: FieldHex(field_to_hex(&Fr::from(1u64))),
        op,
        bound,
        datatype_const: FieldHex(field_to_hex(&dual_leaf_v2::integer_lane_const())),
        expected,
    };
    assert_eq!(check_lane_member_pairing(&integer_lane), Ok(()));
}

/// The verifier's structural pre-filter carries the SAME pairing refusal, before
/// it reads the sub-proofs for anything else — so a manifest presenting the V2
/// boolean lane constant on the shared integer member is rejected by a relying
/// party, not only by this crate's builders.
#[test]
fn the_verifier_prefilter_rejects_the_crossed_lane_member_pairing() {
    let built = build_filter_value_dl_bool(&typed("true", XSD_BOOLEAN), FilterOp::Eq, true)
        .expect("the V2 boolean builder accepts a canonical literal");
    let ProofInputs::FilterValueDlBool { operand_enc, op, bound, datatype_const, expected, .. } =
        built.inputs
    else {
        panic!("the V2 boolean builder emits its own variant");
    };
    let crossed = ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc,
        op,
        bound,
        datatype_const,
        expected,
    };
    let manifest = manifest_with(vec![crossed]);
    match prefilter_manifest_structure(&manifest, &KeySet::empty(), &RevocationPolicy::accept_version(1)) {
        Err(CheckError::LaneMemberMismatch { proof, reason }) => {
            assert_eq!(proof, 0);
            assert!(reason.contains("filter_value_dl_int"), "{}", reason);
        }
        other => panic!("expected a lane/member refusal, got {:?}", other),
    }
}

/// LIVE prove+verify on the dedicated bit-domain member: `nargo execute` +
/// `bb prove` + `bb verify`, through the TYPED checked prove entry
/// (`prove_value_lane`, which renders its own `Prover.toml`), for an honest TRUE
/// and an honest FALSE case — plus the refusal of a handle of `2` at witness
/// generation.
///
/// Why all three in one test: a proof that verifies says the relation is
/// satisfiable for the honest assignment, and the `2` case says the bit
/// constraint is what narrows that relation; neither alone distinguishes the
/// dedicated member from the shared integer one. The tampered-proof case is the
/// standard control that `verify` is actually deciding something.
#[test]
fn full_prove_verify_filter_value_dl_bool() {
    if !support::require_toolchain("full_prove_verify_filter_value_dl_bool", &["nargo", "bb"]) {
        return;
    }
    let prover = CircuitProver::from_crate_root();
    for (value, op, bound, tag) in [
        (true, FilterOp::Gt, false, "dlv2_live_true"),
        (false, FilterOp::Eq, false, "dlv2_live_false"),
    ] {
        let built = build_filter_value_dl_bool(&bool_lit(value), op, bound)
            .expect("the V2 boolean builder accepts a canonical literal");
        let p = unpack(&built.inputs);
        let proof = built.lane_proof().expect("the V2 build assembles typed inputs");
        // The checked entry renders exactly the public renderer's bytes.
        assert_eq!(
            proof
                .checked_prover_toml(
                    CommitmentMethod::DualLeafV2,
                    MethodUse::NewIssuance,
                    &challenge_one()
                )
                .unwrap(),
            filter_value_dl_bool_prover_toml(
                &challenge_one(),
                p.operand_enc,
                p.op.code(),
                bound,
                p.expected,
                &built.value_hook,
                &built.lexical_component,
            )
            .expect("the pinned V2 boolean constant rides its own member"),
        );
        let out = unique_out_dir(tag);
        let art = prover
            .prove_value_lane(
                CommitmentMethod::DualLeafV2,
                MethodUse::NewIssuance,
                &challenge_one(),
                &proof,
                &out,
                tag,
            )
            .expect("the honest V2 boolean assignment must prove");
        assert!(!art.proof.is_empty(), "bb must emit a proof");
        assert!(
            prover
                .verify(&art, &out.join("verify"))
                .expect("bb verify runs"),
            "the honest {} proof must verify",
            value
        );
        // Control: a flipped proof byte must be rejected, so the assertion above
        // is not vacuously true.
        let mut bad = art.clone();
        let mid = bad.proof.len() / 2;
        bad.proof[mid] ^= 0xff;
        assert!(
            !prover
                .verify(&bad, &out.join("verify_bad"))
                .expect("bb verify runs"),
            "a tampered proof must be rejected"
        );
    }

    // A handle of 2 — admissible in the shared integer member's `u64` domain — is
    // refused on this member at witness generation. The leaf is the one an
    // (impossible) committer of the handle 2 would have produced, so the binding
    // constraint is satisfied and the bit constraint is what refuses.
    let dt = boolean_lane_const_v2();
    let lexical = hexf(&Fr::from(0xdefu64));
    let enc_two = hexf(&member_rebinds_leaf(Fr::from(2u64), fr(&dt), fr(&lexical)));
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDlBool,
            "dlv2_live_two",
            &enc_two,
            FilterOp::Gt,
            1,
            &dt,
            true,
            &hexf(&Fr::from(2u64)),
            &lexical,
        ),
        "a handle of 2 must be refused on the bit-domain member"
    );
}

fn manifest_with(inputs: Vec<ProofInputs>) -> ProofManifest {
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
        "{{\"type\":\"urn:sparq:zk:ProofManifest\",\"query\":\"SELECT ?s WHERE {{ ?s <http://ex/p> ?o }}\",\"attributions\":[],\"entailment_regime\":\"simple\",\"binding\":{{\"mode\":\"challenge\",\"challenge\":\"0x2a\"}},\"sub_proofs\":[{}]}}",
        sub_proofs.join(",")
    );
    ProofManifest::from_json(&json).expect("the minimal manifest parses")
}

/// A unique scratch path per call, so two runs of this suite (or a run alongside
/// another) never collide on a fixed name and never see a leftover directory.
fn unique_out_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "sparq_zk_{}_{}_{}_{}",
        label,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ))
}

/// An integer-lane typed proof with canonical placeholder fields and the given
/// constant — enough for every check that precedes the toolchain.
fn integer_lane(datatype_const: FieldHex) -> ValueLaneProof {
    ValueLaneProof::Integer(IntegerLane {
        operand_enc: hexf(&Fr::from(7u64)),
        op: FilterOp::Eq,
        bound: 7,
        datatype_const,
        expected: true,
        witness: HookWitness {
            value_hook: hexf(&Fr::from(7u64)),
            lexical_component: hexf(&Fr::from(8u64)),
        },
    })
}

/// The checked prove entry refuses a FRESH proof under the superseded method
/// BEFORE it spawns `nargo`/`bb` — so the boundary holds on the proving path, not
/// only in the builder. No toolchain needed: the refusal happens first, which is
/// the point.
#[test]
fn the_checked_prove_entry_refuses_a_fresh_v1_proof_before_spawning() {
    let prover = CircuitProver::from_crate_root();
    let out = unique_out_dir("v1_issuance_boundary");
    let v1 = integer_lane(hexf(&sparq_zk::dual_leaf::datatype_const(XSD_INTEGER)));
    let err = prover
        .prove_value_lane(
            CommitmentMethod::DualLeafV1,
            MethodUse::NewIssuance,
            &challenge_one(),
            &v1,
            &out,
            "boundary",
        )
        .expect_err("a fresh V1 proof must be refused");
    assert!(matches!(err, DriverError::Dispatch(_)), "{:?}", err);
    assert!(err.to_string().contains(CommitmentMethod::DualLeafV1.scheme_iri()));
    // The refusal came before any subprocess, so nothing was written.
    assert!(!out.exists(), "the refusal must precede any scratch directory");
}

/// The checked entry checks the INPUTS' lane constant as well as the method enum:
/// a V2 method paired with a V1-constant input is refused for new issuance, and
/// — because the entry takes the method — also for a legacy re-proof, where the
/// method version and the constant version must agree. Still before any
/// subprocess.
#[test]
fn the_checked_prove_entry_refuses_a_v2_method_carrying_a_v1_lane_constant() {
    let prover = CircuitProver::from_crate_root();
    let out = unique_out_dir("v2_method_v1_const");
    let proof = integer_lane(boolean_datatype_const()); // the V1 boolean lane constant
    for (use_, check) in [
        (MethodUse::NewIssuance, "legacy-constant"),
        (MethodUse::LegacyAlreadyIssued, "version-agreement"),
    ] {
        let err = prover
            .prove_value_lane(CommitmentMethod::DualLeafV2, use_, &challenge_one(), &proof, &out, "v1const")
            .expect_err("a V1 lane constant must not be provable under the V2 method");
        match (use_, err) {
            (
                MethodUse::NewIssuance,
                DriverError::Dispatch(DispatchError::LegacyLaneConstForIssuance { .. }),
            )
            | (
                MethodUse::LegacyAlreadyIssued,
                DriverError::Dispatch(DispatchError::MethodLaneVersionMismatch { .. }),
            ) => {}
            (_, other) => panic!("expected the {} refusal, got {:?}", check, other),
        }
    }
    assert!(!out.exists(), "the refusal must precede any scratch directory");
}

/// Non-canonical texts for a value-lane `FieldHex`: texts that would add TOML
/// structure the fields did not produce (a second top-level key, a nested
/// table, a trailing newline), plus the spellings the permissive parser accepts
/// (uppercase, short, `0x`-less).
fn non_canonical_spellings() -> Vec<(&'static str, String)> {
    let v1 = boolean_datatype_const().0;
    // An element whose canonical text has hex LETTERS, so its uppercase
    // spelling actually differs from it.
    let canonical = hexf(&Fr::from(0xabc7u64)).0;
    assert_ne!(canonical, canonical.to_uppercase().replace("0X", "0x"));
    vec![
        ("second key", format!("{}\"\ndatatype_const = \"{}", canonical, v1)),
        ("nested table", format!("{}\"\n[unused]\ndatatype_const = \"{}", canonical, v1)),
        ("trailing newline", format!("{}\n", canonical)),
        ("uppercase", format!("0x{}", canonical[2..].to_uppercase())),
        ("short", "0x7".to_string()),
        ("no prefix", canonical[2..].to_string()),
    ]
}

/// CALLER TEXT CANNOT REACH `nargo`. The checked entry has no TOML parameter, so
/// the only caller text that enters its `Prover.toml` arrives through `FieldHex`
/// fields — and each of those, in every position the file is rendered from
/// (`challenge`, `operand_enc`, `datatype_const`, `value_hook`,
/// `lexical_component`), is refused unless it is canonical hex, naming the field,
/// before any subprocess. The public renderers refuse the same texts.
#[test]
fn caller_text_cannot_reach_the_rendered_prover_toml() {
    let prover = CircuitProver::from_crate_root();
    let out = unique_out_dir("non_canonical_field_text");
    let built = build_filter_value_dl_bool(&typed("true", XSD_BOOLEAN), FilterOp::Eq, true).unwrap();
    let honest = built.lane_proof().unwrap();
    let ValueLaneProof::Boolean(base) = honest.clone() else {
        panic!("the V2 boolean build is a boolean lane")
    };
    let int_base = integer_lane(hexf(&dual_leaf_v2::integer_lane_const()));
    for (label, text) in non_canonical_spellings() {
        let h = FieldHex(text);
        let mut cases: Vec<(&str, FieldHex, ValueLaneProof)> = Vec::new();
        cases.push(("challenge", h.clone(), honest.clone()));
        let mut b = base.clone();
        b.operand_enc = h.clone();
        cases.push(("operand_enc", challenge_one(), ValueLaneProof::Boolean(b)));
        let mut b = base.clone();
        b.witness.value_hook = h.clone();
        cases.push(("value_hook", challenge_one(), ValueLaneProof::Boolean(b)));
        let mut b = base.clone();
        b.witness.lexical_component = h.clone();
        cases.push(("lexical_component", challenge_one(), ValueLaneProof::Boolean(b)));
        if let ValueLaneProof::Integer(mut i) = int_base.clone() {
            i.datatype_const = h.clone();
            cases.push(("datatype_const", challenge_one(), ValueLaneProof::Integer(i)));
        }
        for (field, challenge, proof) in cases {
            let err = prover
                .prove_value_lane(
                    CommitmentMethod::DualLeafV2,
                    MethodUse::NewIssuance,
                    &challenge,
                    &proof,
                    &out,
                    "non_canonical",
                )
                .expect_err("non-canonical field text must be refused");
            match err {
                DriverError::Dispatch(DispatchError::NonCanonicalFieldHex { field: f, .. }) => {
                    assert_eq!(f, field, "{} in {}", label, field)
                }
                other => panic!("{} in {}: expected a non-canonical refusal, got {:?}", label, field, other),
            }
        }
        // The public renderer refuses the same text in the same positions.
        let p = unpack(&built.inputs);
        let r = |c: &FieldHex, e: &FieldHex, vh: &FieldHex, lc: &FieldHex| {
            filter_value_dl_bool_prover_toml(c, e, p.op.code(), true, p.expected, vh, lc)
        };
        for (field, res) in [
            ("challenge", r(&h, p.operand_enc, &built.value_hook, &built.lexical_component)),
            ("operand_enc", r(&challenge_one(), &h, &built.value_hook, &built.lexical_component)),
            ("value_hook", r(&challenge_one(), p.operand_enc, &h, &built.lexical_component)),
            ("lexical_component", r(&challenge_one(), p.operand_enc, &built.value_hook, &h)),
        ] {
            assert!(
                matches!(&res, Err(DispatchError::NonCanonicalFieldHex { field: f, .. }) if f == field),
                "{} in {}: {:?}",
                label,
                field,
                res
            );
        }
    }
    assert!(!out.exists(), "every refusal must precede any scratch directory");
}

/// What the checked entry DOES render is always the flat form: one `key = value`
/// per line, each key once, no table header, every quoted value either canonical
/// hex or a decimal integer, in the member's declaration order. Checked for every
/// lane, so a renderer that grew a stray key, a table or an unquoted string would
/// show up here.
#[test]
fn the_checked_prover_toml_is_always_the_flat_form() {
    let h = |n: u64| hexf(&Fr::from(n));
    let hook = || HookWitness { value_hook: h(3), lexical_component: h(4) };
    let signed = || sparq_zk_compose::value_lane::SignedWitness {
        value_neg: true,
        value_hook_scaled: h(3),
        lexical_component: h(4),
    };
    use sparq_zk_compose::value_lane::{DateTimeLane, DecimalLane, DoubleLane};
    let cases: Vec<(ValueLaneProof, Vec<&str>)> = vec![
        (
            integer_lane(hexf(&dual_leaf_v2::integer_lane_const())),
            vec!["challenge", "operand_enc", "op", "bound", "datatype_const", "expected", "value_hook", "lexical_component"],
        ),
        (
            ValueLaneProof::Double(DoubleLane {
                operand_enc: h(2),
                op: FilterOp::Lt,
                b_bits: 0x4008_0000_0000_0000,
                datatype_const: hexf(&dual_leaf_v2::double_lane_const()),
                expected: false,
                witness: hook(),
            }),
            vec!["challenge", "operand_enc", "op", "b_bits", "datatype_const", "expected", "value_hook", "lexical_component"],
        ),
        (
            ValueLaneProof::Decimal(DecimalLane {
                operand_enc: h(2),
                op: FilterOp::Ge,
                bound_neg: false,
                bound_scaled: 725,
                datatype_const: hexf(&dual_leaf_v2::decimal_lane_const(2)),
                expected: true,
                witness: signed(),
            }),
            vec!["challenge", "operand_enc", "op", "bound_neg", "bound_scaled", "datatype_const", "expected", "value_neg", "value_hook_scaled", "lexical_component"],
        ),
        (
            ValueLaneProof::DateTime(DateTimeLane {
                operand_enc: h(2),
                op: FilterOp::Ne,
                bound_neg: true,
                bound_scaled_epoch: 86_400_000,
                datatype_const: hexf(&dual_leaf_v2::date_lane_const()),
                expected: true,
                witness: signed(),
            }),
            vec!["challenge", "operand_enc", "op", "bound_neg", "bound_scaled_epoch", "datatype_const", "expected", "value_neg", "value_hook_scaled", "lexical_component"],
        ),
        (
            ValueLaneProof::Boolean(BooleanLane {
                operand_enc: h(2),
                op: FilterOp::Eq,
                bound: true,
                expected: true,
                witness: hook(),
            }),
            vec!["challenge", "operand_enc", "op", "bound", "datatype_const", "expected", "value_hook", "lexical_component"],
        ),
    ];
    for (proof, keys) in cases {
        let toml = proof
            .checked_prover_toml(CommitmentMethod::DualLeafV2, MethodUse::NewIssuance, &challenge_one())
            .unwrap_or_else(|e| panic!("{:?}: {}", proof.lane(), e));
        let lines: Vec<&str> = toml.lines().collect();
        assert_eq!(lines.len(), keys.len(), "{:?}", proof.lane());
        for (line, key) in lines.iter().zip(&keys) {
            let (k, v) = line.split_once(" = ").expect("key = value");
            assert_eq!(k, *key, "{:?}", proof.lane());
            let flat_value = v == "true"
                || v == "false"
                || (v.len() == 68
                    && v.starts_with("\"0x")
                    && v.ends_with('"')
                    && v[3..67].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                || (v.len() >= 3
                    && v.starts_with('"')
                    && v.ends_with('"')
                    && v[1..v.len() - 1].bytes().all(|b| b.is_ascii_digit()));
            assert!(flat_value, "{:?}: `{}` is not a flat value", proof.lane(), line);
        }
        assert!(toml.ends_with('\n') && !toml.contains('['), "{:?}", proof.lane());
    }
}

/// A crossed variant/id is not expressible through the typed inputs — a
/// `ValueLaneProof` has no id field, and its member is derived — and assembling
/// typed inputs from a crossed `ProofInputs` is refused by the SAME derivation
/// the verifier uses. A non-value-lane variant declaring a value-lane member
/// (`FilterInt { id: FilterValueDl }`) and a value-lane variant declaring another
/// member are both refused.
#[test]
fn a_crossed_variant_and_member_is_refused_by_the_shared_derivation() {
    let witness = || {
        LaneWitness::Hook(HookWitness {
            value_hook: hexf(&Fr::from(1u64)),
            lexical_component: hexf(&Fr::from(2u64)),
        })
    };
    let filter_int_crossed = ProofInputs::FilterInt {
        id: CircuitId::FilterValueDl,
        operand_enc: hexf(&Fr::from(1u64)),
        op: FilterOp::Eq,
        bound: 1,
        expected: true,
    };
    let crossed = ProofInputs::FilterValueDlBool {
        id: CircuitId::FilterValueDl,
        operand_enc: hexf(&Fr::from(1u64)),
        op: FilterOp::Eq,
        bound: 1,
        datatype_const: boolean_lane_const_v2(),
        expected: true,
    };
    for inputs in [&filter_int_crossed, &crossed] {
        assert!(matches!(
            ValueLaneProof::from_inputs(inputs, witness()),
            Err(DispatchError::ValueLaneVariantIdMismatch { .. })
        ));
        for use_ in [MethodUse::NewIssuance, MethodUse::LegacyAlreadyIssued] {
            assert!(matches!(
                check_lane_member_pairing_for_use(inputs, use_),
                Err(DispatchError::ValueLaneVariantIdMismatch { .. })
            ));
        }
    }
    // The honest variant assembles, and its derived member is its own.
    let mut honest = crossed.clone();
    if let ProofInputs::FilterValueDlBool { id, .. } = &mut honest {
        *id = CircuitId::FilterValueDlBool;
    }
    let typed_ok = ValueLaneProof::from_inputs(&honest, witness()).unwrap();
    assert_eq!(typed_ok.circuit_id(), CircuitId::FilterValueDlBool);
    assert_eq!(typed_ok.proof_inputs(), honest);
    // A witness of the other shape, and a boolean bound outside {0, 1}, are refused.
    let signed = LaneWitness::Signed(sparq_zk_compose::value_lane::SignedWitness {
        value_neg: false,
        value_hook_scaled: hexf(&Fr::from(1u64)),
        lexical_component: hexf(&Fr::from(2u64)),
    });
    assert!(matches!(
        ValueLaneProof::from_inputs(&honest, signed),
        Err(DispatchError::MalformedValueLaneInputs { .. })
    ));
    let mut two = honest.clone();
    if let ProofInputs::FilterValueDlBool { bound, .. } = &mut two {
        *bound = 2;
    }
    assert!(matches!(
        ValueLaneProof::from_inputs(&two, witness()),
        Err(DispatchError::MalformedValueLaneInputs { .. })
    ));
    // A NON-value-lane variant declaring its OWN member is still accepted by the
    // pairing check — the rule is agreement, not a ban on non-value-lane inputs.
    let honest_int = ProofInputs::FilterInt {
        id: CircuitId::FilterInt { d: 2 },
        operand_enc: hexf(&Fr::from(1u64)),
        op: FilterOp::Eq,
        bound: 1,
        expected: true,
    };
    assert_eq!(
        check_lane_member_pairing_for_use(&honest_int, MethodUse::NewIssuance),
        Ok(())
    );
}

/// An uppercase or short `operand_enc` — both of which the permissive parser
/// decodes to the intended element — is refused by the `ProofInputs`-level check,
/// by every ordinary renderer and by the checked prove entry.
#[test]
fn an_uppercase_or_short_operand_enc_is_refused_everywhere() {
    let canonical = hexf(&Fr::from(0xabc7u64));
    for text in [format!("0x{}", canonical.0[2..].to_uppercase()), "0xabc7".to_string()] {
        let bad = FieldHex(text.clone());
        assert_ne!(bad, canonical, "the spelling must differ from the canonical text");
        assert_eq!(field_from_hex_str(&text), Some(Fr::from(0xabc7u64)), "{}", text);
        let mut proof = integer_lane(hexf(&dual_leaf_v2::integer_lane_const()));
        if let ValueLaneProof::Integer(i) = &mut proof {
            i.operand_enc = bad.clone();
        }
        let named = |r: Result<String, DispatchError>| {
            matches!(r, Err(DispatchError::NonCanonicalFieldHex { field, .. }) if field == "operand_enc")
        };
        assert!(named(proof.checked_prover_toml(
            CommitmentMethod::DualLeafV2,
            MethodUse::NewIssuance,
            &challenge_one()
        )));
        assert!(matches!(
            check_lane_member_pairing(&proof.proof_inputs()),
            Err(DispatchError::NonCanonicalFieldHex { .. })
        ));
        let (c, h) = (challenge_one(), hexf(&Fr::from(3u64)));
        let f = |x: Fr| hexf(&x);
        assert!(named(filter_value_dl_prover_toml(
            &c, &bad, 4, 7, &f(dual_leaf_v2::integer_lane_const()), true, &h, &h
        )));
        assert!(named(filter_value_dl_bool_prover_toml(&c, &bad, 4, true, true, &h, &h)));
        assert!(named(filter_value_dl_f64_prover_toml(
            &c, &bad, 4, 7, &f(dual_leaf_v2::double_lane_const()), true, &h, &h
        )));
        assert!(named(filter_value_dl_decimal_prover_toml(
            &c, &bad, 4, false, 7, &f(dual_leaf_v2::decimal_lane_const(2)), true, false, &h, &h
        )));
        assert!(named(filter_value_dl_datetime_prover_toml(
            &c, &bad, 4, false, 7, &f(dual_leaf_v2::datetime_lane_const()), true, false, &h, &h
        )));
        assert!(named(legacy_v1::filter_value_dl_prover_toml(
            &c, &bad, 4, 7, &f(sparq_zk::dual_leaf::datatype_const(XSD_INTEGER)), true, &h, &h
        )));
    }
}

/// The checked entry takes no TOML, so a `Prover.toml` outside the renderers'
/// flat form cannot be HANDED to it; the remaining route for such text is a
/// field value. Each spelling below is valid TOML (or close to it) that a parser
/// could read a different `datatype_const` from; placed in any `FieldHex` field
/// the checked entry renders from, every one is refused as non-canonical before
/// any subprocess — and the honest proof renders exactly the flat bytes the
/// in-circuit vectors are solved from.
#[test]
fn every_prover_toml_spelling_outside_the_flat_form_is_refused_as_field_text() {
    let prover = CircuitProver::from_crate_root();
    let out = unique_out_dir("flat_form");
    let built = build_filter_value_dl_bool(&typed("true", XSD_BOOLEAN), FilterOp::Eq, true)
        .expect("the V2 boolean builder accepts a canonical literal");
    let p = unpack(&built.inputs);
    let honest = raw_value_lane_toml(
        &challenge_one(),
        p.operand_enc,
        p.op.code(),
        p.bound,
        p.datatype_const,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    );
    let proof = built.lane_proof().unwrap();
    assert_eq!(
        proof
            .checked_prover_toml(CommitmentMethod::DualLeafV2, MethodUse::NewIssuance, &challenge_one())
            .unwrap(),
        honest,
        "the checked entry renders the flat form"
    );
    let v2 = &p.datatype_const.0;
    let v1 = &boolean_datatype_const().0;
    let line = format!("datatype_const = \"{}\"", v2);
    let without = honest.replace(&format!("{}\n", line), "");
    assert_ne!(without, honest, "the honest rendering carries the line");
    let cases: Vec<(&str, String)> = vec![
        ("comment line", format!("# note\n{}", honest)),
        ("blank line", format!("\n{}", honest)),
        ("CRLF", honest.replace('\n', "\r\n")),
        ("byte-order mark", format!("\u{feff}{}", honest)),
        ("quoted key", format!("{}\"datatype_const\" = \"{}\"\n", without, v2)),
        ("dotted key", format!("{}a.datatype_const = \"{}\"\n", without, v2)),
        ("literal string", format!("{}datatype_const = '{}'\n", without, v2)),
        ("escape in string", format!("{}datatype_const = \"{}\\u0030\"\n", without, v2)),
        ("multi-line string", format!("{}datatype_const = \"\"\"{}\"\"\"\n", without, v2)),
        ("inline table", format!("{}t = {{ datatype_const = \"{}\" }}\n{}\n", without, v1, line)),
        ("array of tables after", format!("{}[[t]]\ndatatype_const = \"{}\"\n", honest, v1)),
        ("extra spaces", format!("{}datatype_const  =  \"{}\"\n", without, v2)),
        ("trailing comment", format!("{}{} # x\n", without, line)),
        ("duplicate key", format!("{}{}\n", honest, line)),
        ("no key", without.clone()),
    ];
    let ValueLaneProof::Boolean(base) = proof else { panic!("boolean lane") };
    for (label, toml) in cases {
        let text = FieldHex(toml);
        let mut in_operand = base.clone();
        in_operand.operand_enc = text.clone();
        let mut in_lexical = base.clone();
        in_lexical.witness.lexical_component = text.clone();
        for (field, challenge, proof) in [
            ("challenge", text.clone(), ValueLaneProof::Boolean(base.clone())),
            ("operand_enc", challenge_one(), ValueLaneProof::Boolean(in_operand)),
            ("lexical_component", challenge_one(), ValueLaneProof::Boolean(in_lexical)),
        ] {
            let err = prover
                .prove_value_lane(
                    CommitmentMethod::DualLeafV2,
                    MethodUse::NewIssuance,
                    &challenge,
                    &proof,
                    &out,
                    "flatform",
                )
                .expect_err(label);
            assert!(
                matches!(&err, DriverError::Dispatch(DispatchError::NonCanonicalFieldHex { field: f, .. }) if f == field),
                "{} in {}: {:?}",
                label,
                field,
                err
            );
        }
    }
    assert!(!out.exists(), "every refusal must precede any scratch directory");
}

// =========================================================================
// 9. The lane↔member ALLOW-LIST, over every (constant, member) cell
// =========================================================================

/// Every lane constant this build derives, with the ONE member it may ride and
/// whether it is a `DualLeafV1` constant (legacy use only).
fn every_lane_constant() -> Vec<(&'static str, FieldHex, CircuitId, bool)> {
    let f = |x: Fr| FieldHex(field_to_hex(&x));
    let mut rows: Vec<(&'static str, FieldHex, CircuitId, bool)> = vec![
        (
            "v1 integer",
            f(sparq_zk::dual_leaf::datatype_const(XSD_INTEGER)),
            CircuitId::FilterValueDl,
            true,
        ),
        ("v1 boolean", boolean_datatype_const(), CircuitId::FilterValueDl, true),
        (
            "v1 double",
            f(sparq_zk::dual_leaf::datatype_const(XSD_DOUBLE)),
            CircuitId::FilterValueDlF64,
            true,
        ),
        (
            "v1 dateTime",
            f(sparq_zk::dual_leaf_datetime::datetime_datatype_const()),
            CircuitId::FilterValueDlDateTime,
            true,
        ),
        (
            "v1 date",
            f(sparq_zk::dual_leaf_datetime::date_datatype_const()),
            CircuitId::FilterValueDlDateTime,
            true,
        ),
        (
            "v2 integer",
            f(dual_leaf_v2::integer_lane_const()),
            CircuitId::FilterValueDl,
            false,
        ),
        (
            "v2 double",
            f(dual_leaf_v2::double_lane_const()),
            CircuitId::FilterValueDlF64,
            false,
        ),
        ("v2 boolean", boolean_lane_const_v2(), CircuitId::FilterValueDlBool, false),
        (
            "v2 dateTime",
            f(dual_leaf_v2::datetime_lane_const()),
            CircuitId::FilterValueDlDateTime,
            false,
        ),
        ("v2 date", f(dual_leaf_v2::date_lane_const()), CircuitId::FilterValueDlDateTime, false),
    ];
    // The decimal lanes carry the SCALE in the constant and the member id does
    // not, so EVERY encodable scale is its own constant and all of them must ride
    // the decimal member. Covering the whole range (not one sample scale) is what
    // makes a narrowing of the encodable range visible here.
    for fd in 1..=ENCODABLE_DECIMAL_SCALES {
        rows.push((
            "v1 decimal",
            f(sparq_zk::dual_leaf::decimal_datatype_const(fd)),
            CircuitId::FilterValueDlDecimal,
            true,
        ));
        rows.push((
            "v2 decimal",
            f(dual_leaf_v2::decimal_lane_const(fd)),
            CircuitId::FilterValueDlDecimal,
            false,
        ));
    }
    rows
}

/// The number of decimal scales the host encoder can produce, and therefore the
/// number of decimal lane constants the allow-list must hold per version. The
/// encoder needs a non-empty fraction and computes `10^fd` in a `u64`, so the
/// range is `1..=19`; `check_every_encodable_decimal_scale_rides_the_decimal_member`
/// asserts BOTH ends of it against the real encoder.
const ENCODABLE_DECIMAL_SCALES: u32 = 19;

fn every_value_lane_member() -> [CircuitId; 5] {
    [
        CircuitId::FilterValueDl,
        CircuitId::FilterValueDlF64,
        CircuitId::FilterValueDlDecimal,
        CircuitId::FilterValueDlDateTime,
        CircuitId::FilterValueDlBool,
    ]
}

/// The allow-list is AFFIRMATIVE and BIDIRECTIONAL: for EVERY (constant, member)
/// cell, the pairing is accepted iff the table pairs them — and a `DualLeafV1`
/// constant additionally only under the legacy use. Every other cell is a typed
/// refusal, never an accept.
#[test]
fn every_lane_constant_member_cell_is_decided_by_the_allow_list() {
    for (label, constant, owner, legacy_v1) in every_lane_constant() {
        for member in every_value_lane_member() {
            let legacy = check_lane_const_for_member(
                &member,
                &constant,
                MethodUse::LegacyAlreadyIssued,
            );
            if member == owner {
                assert_eq!(legacy, Ok(()), "{} must ride {}", label, member.package().unwrap());
            } else {
                assert!(
                    matches!(legacy, Err(DispatchError::LaneMemberMismatch { .. })),
                    "{} on {} must be a lane/member refusal, got {:?}",
                    label,
                    member.package().unwrap(),
                    legacy
                );
            }
            let fresh =
                check_lane_const_for_member(&member, &constant, MethodUse::NewIssuance);
            match (member == owner, legacy_v1) {
                (true, false) => assert_eq!(fresh, Ok(()), "{} is V2 and selectable", label),
                (true, true) => assert!(
                    matches!(fresh, Err(DispatchError::LegacyLaneConstForIssuance { .. })),
                    "{} is a V1 constant and must be legacy-only, got {:?}",
                    label,
                    fresh
                ),
                (false, _) => assert!(fresh.is_err()),
            }
        }
    }
}

/// The three renderers that TAKE a `datatype_const` — `f64`, `decimal` and
/// `datetime` — each refuse a constant that is not their member's, and render no
/// text when they do.
///
/// Their own lane constant is accepted, so the test separates "refuses the wrong
/// constant" from "refuses everything". The `bool` renderer is absent because it
/// takes no constant: it pins its own.
#[test]
fn every_renderer_that_takes_a_constant_refuses_another_lane() {
    let f = |x: Fr| FieldHex(field_to_hex(&x));
    let c = challenge_one();
    let hook = FieldHex(field_to_hex(&Fr::from(5u64)));
    let lex = FieldHex(field_to_hex(&Fr::from(9u64)));
    let enc = FieldHex(field_to_hex(&Fr::from(5u64)));
    let unknown = f(sparq_zk::dual_leaf::datatype_const("http://ex/opaque"));

    // --- f64 (M14) ---
    let f64_render = |dt: &FieldHex| {
        filter_value_dl_f64_prover_toml(
            &c, &enc, FilterOp::Eq.code(), 5, dt, true, &hook, &lex,
        )
    };
    assert!(f64_render(&f(dual_leaf_v2::double_lane_const())).is_ok());
    for (label, wrong) in [
        ("the integer lane", f(dual_leaf_v2::integer_lane_const())),
        ("the decimal lane", f(dual_leaf_v2::decimal_lane_const(2))),
        ("the boolean lane", boolean_lane_const_v2()),
        ("the dateTime lane", f(dual_leaf_v2::datetime_lane_const())),
    ] {
        assert!(
            matches!(
                f64_render(&wrong),
                Err(DispatchError::LaneMemberMismatch { .. })
            ),
            "the f64 renderer must refuse {}",
            label
        );
    }
    assert!(matches!(
        f64_render(&unknown),
        Err(DispatchError::UnknownLaneConst { .. })
    ));
    assert!(matches!(
        f64_render(
            &f(sparq_zk::dual_leaf::datatype_const(XSD_DOUBLE))),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));

    // --- decimal (M15) ---
    let dec_render = |dt: &FieldHex| {
        filter_value_dl_decimal_prover_toml(
            &c,
            &enc,
            FilterOp::Eq.code(),
            false,
            725,
            dt,
            true,
            false,
            &hook,
            &lex
        )
    };
    assert!(dec_render(&f(dual_leaf_v2::decimal_lane_const(2))).is_ok());
    for (label, wrong) in [
        ("the integer lane", f(dual_leaf_v2::integer_lane_const())),
        ("the double lane", f(dual_leaf_v2::double_lane_const())),
        ("the boolean lane", boolean_lane_const_v2()),
        ("the date lane", f(dual_leaf_v2::date_lane_const())),
    ] {
        assert!(
            matches!(
                dec_render(&wrong),
                Err(DispatchError::LaneMemberMismatch { .. })
            ),
            "the decimal renderer must refuse {}",
            label
        );
    }
    assert!(matches!(
        dec_render(&unknown),
        Err(DispatchError::UnknownLaneConst { .. })
    ));
    assert!(matches!(
        dec_render(
            &f(sparq_zk::dual_leaf::decimal_datatype_const(2))),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));

    // --- datetime (M16) ---
    let dt_render = |dt: &FieldHex| {
        filter_value_dl_datetime_prover_toml(
            &c,
            &enc,
            FilterOp::Eq.code(),
            false,
            86_400_000,
            dt,
            true,
            false,
            &hook,
            &lex
        )
    };
    assert!(dt_render(&f(dual_leaf_v2::datetime_lane_const())).is_ok());
    assert!(dt_render(&f(dual_leaf_v2::date_lane_const())).is_ok());
    for (label, wrong) in [
        ("the integer lane", f(dual_leaf_v2::integer_lane_const())),
        ("the double lane", f(dual_leaf_v2::double_lane_const())),
        ("the decimal lane", f(dual_leaf_v2::decimal_lane_const(3))),
        ("the boolean lane", boolean_lane_const_v2()),
    ] {
        assert!(
            matches!(
                dt_render(&wrong),
                Err(DispatchError::LaneMemberMismatch { .. })
            ),
            "the datetime renderer must refuse {}",
            label
        );
    }
    assert!(matches!(
        dt_render(&unknown),
        Err(DispatchError::UnknownLaneConst { .. })
    ));
    assert!(matches!(
        dt_render(
            &f(sparq_zk::dual_leaf_datetime::datetime_datatype_const())),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));
}

/// Every decimal scale the HOST ENCODER can produce has a row on the decimal
/// member — both versions — and a scale it cannot produce has none.
///
/// The range is pinned against the real encoder rather than against the table's
/// own bound: `0.<19 digits>` encodes and yields `decimal_lane_const(19)`, while
/// `0.<20 digits>` is refused by the encoder (its `10^fd` overflows a `u64`), so
/// `decimal_lane_const(20)` is a constant no lane derives and is refused on every
/// member. A narrowing of the allow-list's scale range therefore shows up as an
/// honest encoder output the table no longer admits.
#[test]
fn check_every_encodable_decimal_scale_rides_the_decimal_member() {
    let f = |x: Fr| FieldHex(field_to_hex(&x));
    for fd in 1..=ENCODABLE_DECIMAL_SCALES {
        let lexical = format!("0.{}1", "0".repeat((fd - 1) as usize));
        let encoded = dual_leaf_v2::encode_decimal(&typed(&lexical, XSD_DECIMAL))
            .unwrap_or_else(|e| panic!("the encoder must accept scale {}: {:?}", fd, e));
        assert_eq!(
            encoded.datatype_const,
            dual_leaf_v2::decimal_lane_const(fd),
            "the encoder's constant at scale {} must be the lane constant",
            fd
        );
        // Both versions' constant at this scale rides the decimal member, V1 only
        // for the legacy use.
        for (constant, legacy_v1) in [
            (f(dual_leaf_v2::decimal_lane_const(fd)), false),
            (f(sparq_zk::dual_leaf::decimal_datatype_const(fd)), true),
        ] {
            assert_eq!(
                check_lane_const_for_member(
                    &CircuitId::FilterValueDlDecimal,
                    &constant,
                    MethodUse::LegacyAlreadyIssued
                ),
                Ok(()),
                "scale {} must ride the decimal member (legacy_v1={})",
                fd,
                legacy_v1
            );
            let fresh = check_lane_const_for_member(
                &CircuitId::FilterValueDlDecimal,
                &constant,
                MethodUse::NewIssuance,
            );
            if legacy_v1 {
                assert!(
                    matches!(fresh, Err(DispatchError::LegacyLaneConstForIssuance { .. })),
                    "the V1 constant at scale {} must be legacy-only, got {:?}",
                    fd,
                    fresh
                );
            } else {
                assert_eq!(fresh, Ok(()), "the V2 constant at scale {} is selectable", fd);
            }
            // And on another member it is a lane/member refusal, not an unknown
            // constant — the row exists, it just is not that member's.
            assert!(matches!(
                check_lane_const_for_member(
                    &CircuitId::FilterValueDl,
                    &constant,
                    MethodUse::LegacyAlreadyIssued
                ),
                Err(DispatchError::LaneMemberMismatch { .. })
            ));
        }
    }
    // One scale past the encodable range: the encoder refuses the literal, and the
    // constant is in no lane's row.
    let past = ENCODABLE_DECIMAL_SCALES + 1;
    let lexical = format!("0.{}1", "0".repeat((past - 1) as usize));
    assert!(
        dual_leaf_v2::encode_decimal(&typed(&lexical, XSD_DECIMAL)).is_err(),
        "scale {} must be beyond the encoder",
        past
    );
    for constant in [
        f(dual_leaf_v2::decimal_lane_const(past)),
        f(sparq_zk::dual_leaf::decimal_datatype_const(past)),
    ] {
        for member in every_value_lane_member() {
            assert!(
                matches!(
                    check_lane_const_for_member(
                        &member,
                        &constant,
                        MethodUse::LegacyAlreadyIssued
                    ),
                    Err(DispatchError::UnknownLaneConst { .. })
                ),
                "an unencodable scale must be unknown on {}",
                member.package().unwrap()
            );
        }
    }
}

/// The dedicated bit-domain member accepts the V2 boolean constant and NOTHING
/// else — the integer, decimal and temporal constants are refused on it even
/// though a `0`/`1` handle is inside their own domains.
#[test]
fn the_bit_domain_member_accepts_only_the_v2_boolean_constant() {
    for (label, constant, _, _) in every_lane_constant() {
        let r = check_lane_const_for_member(
            &CircuitId::FilterValueDlBool,
            &constant,
            MethodUse::LegacyAlreadyIssued,
        );
        if label == "v2 boolean" {
            assert_eq!(r, Ok(()));
        } else {
            assert!(
                matches!(r, Err(DispatchError::LaneMemberMismatch { .. })),
                "{} must not ride the bit-domain member, got {:?}",
                label,
                r
            );
        }
    }
}

/// A constant NO shipped lane derives is refused on EVERY member, under both
/// uses. Covered: the plain fold of an opaque datatype and of `xsd:boolean`
/// (V1's boolean constant is a `derive_key` one, so the plain fold is not a lane
/// constant), the V2 no-value tag, `0`, `1`, a language constant, and an
/// arbitrary field element.
#[test]
fn an_unknown_constant_is_refused_on_every_member() {
    let f = |x: Fr| FieldHex(field_to_hex(&x));
    let unknown = [
        ("opaque datatype fold", f(sparq_zk::dual_leaf::datatype_const("http://ex/opaque"))),
        ("plain xsd:boolean fold", f(sparq_zk::dual_leaf::datatype_const(XSD_BOOLEAN))),
        ("v2 value_none", f(dual_leaf_v2::value_none())),
        ("zero", f(Fr::from(0u64))),
        ("one", f(Fr::from(1u64))),
        ("v2 lang const", f(dual_leaf_v2::lang_const("en"))),
        ("arbitrary element", f(field_from_hash_bytes(blake3::hash(b"not a lane constant").as_bytes()))),
        (
            "v1 decimal at an unencodable scale",
            f(sparq_zk::dual_leaf::decimal_datatype_const(20)),
        ),
    ];
    for (label, constant) in unknown {
        for member in every_value_lane_member() {
            for use_ in [MethodUse::LegacyAlreadyIssued, MethodUse::NewIssuance] {
                let r = check_lane_const_for_member(&member, &constant, use_);
                assert!(
                    matches!(r, Err(DispatchError::UnknownLaneConst { .. })),
                    "{} on {} must be an unknown-constant refusal, got {:?}",
                    label,
                    member.package().unwrap(),
                    r
                );
            }
        }
    }
}

/// The decision is made on the DECODED field element, so a RE-SPELLED constant
/// does not slip past a text comparison. `field_from_hex_str` accepts an
/// uppercase, `0x`-less, short (left-padded) and at-or-above-modulus (reduced)
/// spelling, and all four decode to the V2 integer lane constant's element — yet
/// every one is refused as non-canonical rather than accepted.
#[test]
fn a_re_spelled_lane_constant_is_refused_as_non_canonical() {
    let canonical = field_to_hex(&dual_leaf_v2::integer_lane_const());
    let body = canonical.strip_prefix("0x").expect("canonical hex is 0x-prefixed");

    let uppercase = format!("0x{}", body.to_uppercase());
    let no_prefix = body.to_string();
    // Short: drop the leading zero nibbles the canonical form pads with. Use a
    // small element whose canonical form HAS leading zeros.
    let small = Fr::from(7u64);
    let short = "0x7".to_string();
    // At or above the modulus: the canonical text of `1` plus the modulus, which
    // `field_from_hex_str` reduces back to `1`.
    let modulus_plus_one = {
        // p = 2^254 + 0x0f0000000000000000000000000000000000000000000000000000000000000... —
        // rather than hard-code it, re-derive: -1 in the field is p-1, so p = (p-1) + 1.
        let p_minus_1 = Fr::from(0u64) - Fr::from(1u64);
        let bytes = field_to_hex(&p_minus_1);
        // Increment the last nibble of (p-1) to spell p itself, which reduces to 0.
        let (head, last) = bytes.split_at(bytes.len() - 1);
        let digit = u8::from_str_radix(last, 16).expect("hex nibble");
        format!("{}{:x}", head, digit + 1)
    };

    for (label, text, expect_decode) in [
        ("uppercase", uppercase, Some(dual_leaf_v2::integer_lane_const())),
        ("no 0x prefix", no_prefix, Some(dual_leaf_v2::integer_lane_const())),
        ("short", short, Some(small)),
        ("the modulus itself", modulus_plus_one, Some(Fr::from(0u64))),
    ] {
        let h = FieldHex(text.clone());
        // The permissive parser DOES accept it (which is the hazard).
        assert_eq!(
            field_from_hex_str(&text),
            expect_decode,
            "{}: the permissive parser decodes this spelling",
            label
        );
        let r = check_lane_const_for_member(
            &CircuitId::FilterValueDl,
            &h,
            MethodUse::LegacyAlreadyIssued,
        );
        assert!(
            matches!(r, Err(DispatchError::NonCanonicalFieldHex { .. })),
            "{}: must be refused as non-canonical, got {:?}",
            label,
            r
        );
    }

    // And the canonical spelling of that same element is accepted, so the refusal
    // is about the TEXT and not about the constant.
    assert_eq!(
        check_lane_const_for_member(
            &CircuitId::FilterValueDl,
            &FieldHex(canonical),
            MethodUse::LegacyAlreadyIssued
        ),
        Ok(())
    );
}

/// The canonicality check runs over EVERY `FieldHex` public input of a value-lane
/// variant, not only `datatype_const`: a re-spelled `operand_enc` is refused too,
/// and the refusal names the field.
#[test]
fn a_non_canonical_operand_enc_is_refused_and_names_the_field() {
    let inputs = ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc: FieldHex("0xAB".to_string()),
        op: FilterOp::Eq,
        bound: 1,
        datatype_const: FieldHex(field_to_hex(&dual_leaf_v2::integer_lane_const())),
        expected: true,
    };
    match check_lane_member_pairing(&inputs) {
        Err(DispatchError::NonCanonicalFieldHex { field, circuit }) => {
            assert_eq!(field, "operand_enc");
            assert_eq!(circuit, "filter_value_dl_int");
        }
        other => panic!("expected a non-canonical-field refusal, got {:?}", other),
    }
}

/// EVERY value-lane sub-proof is refused by the structural pre-filter — the
/// refusal inspects no edge. It is sound to refuse them wholesale because
/// `filter_edge_true` discharges a query FILTER only through a `filter_int` edge,
/// so such a sub-proof would be reconstructed and verified at stage 3 while
/// discharging nothing. Such a manifest used to be accepted.
#[test]
fn the_prefilter_rejects_every_value_lane_sub_proof() {
    let built = build_filter_value_dl_bool(&typed("true", XSD_BOOLEAN), FilterOp::Eq, true)
        .expect("the V2 boolean builder accepts a canonical literal");
    // The pairing itself is honest — this refusal is about the missing binding.
    assert_eq!(check_lane_member_pairing(&built.inputs), Ok(()));
    let manifest = manifest_with(vec![built.inputs]);
    match prefilter_manifest_structure(
        &manifest,
        &KeySet::empty(),
        &RevocationPolicy::accept_version(1),
    ) {
        Err(CheckError::ValueLaneSubProofNotAccepted { proof }) => assert_eq!(proof, 0),
        other => panic!("expected the value-lane sub-proof refusal, got {:?}", other),
    }
}

/// `check_lane_member_pairing_for_use` is the `ProofInputs`-level form of the
/// decision, and the use axis is live in it: the V1 boolean lane's inputs are
/// accepted for re-proving an already-issued graph and refused for a fresh one,
/// while the V2 boolean lane's inputs are accepted either way.
#[test]
fn the_inputs_level_pairing_check_is_use_aware() {
    let v1 = ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc: FieldHex(field_to_hex(&Fr::from(1u64))),
        op: FilterOp::Eq,
        bound: 1,
        datatype_const: boolean_datatype_const(),
        expected: true,
    };
    assert_eq!(
        check_lane_member_pairing_for_use(&v1, MethodUse::LegacyAlreadyIssued),
        Ok(())
    );
    assert!(matches!(
        check_lane_member_pairing_for_use(&v1, MethodUse::NewIssuance),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));

    let v2 = build_filter_value_dl_bool(&typed("true", XSD_BOOLEAN), FilterOp::Eq, true)
        .expect("the V2 boolean builder accepts a canonical literal")
        .inputs;
    for use_ in [MethodUse::LegacyAlreadyIssued, MethodUse::NewIssuance] {
        assert_eq!(check_lane_member_pairing_for_use(&v2, use_), Ok(()));
    }

    // A non-value-lane input carries no datatype_const, so there is no pairing to
    // decide and it is accepted under both uses.
    let scan_free = ProofInputs::FilterInt {
        id: CircuitId::FilterInt { d: 2 },
        operand_enc: FieldHex(field_to_hex(&Fr::from(5u64))),
        op: FilterOp::Lt,
        bound: 50,
        expected: true,
    };
    assert_eq!(
        check_lane_member_pairing_for_use(&scan_free, MethodUse::NewIssuance),
        Ok(())
    );
}

// ====== the whole-graph builder ROUTES the boolean, dateTime and date lanes ===

/// The V2 whole-graph builder routes all six value lanes, so a graph committed
/// under V2 carries a real value hook for an `xsd:boolean` / `xsd:dateTime` /
/// `xsd:date` literal — and the leaf it commits is EXACTLY the `operand_enc` that
/// lane's value-lane FILTER builder presents.
///
/// That equality is the whole point of routing: before it, those literals took
/// the degenerate string lane, so a value-lane FILTER's `operand_enc` matched no
/// committed leaf of the graph and the proof could not be anchored to it.
#[test]
fn a_v2_committed_graph_carries_the_three_newly_routed_lanes_value_hooks() {
    use oxrdf::{NamedOrBlankNode, Term, Triple};
    use sparq_zk::encode::salt_from_bytes;
    use sparq_zk_compose::build::{build_filter_value_dl_date, build_filter_value_dl_datetime};

    let subject = || NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap());
    let flag = typed("true", XSD_BOOLEAN);
    let at = typed(
        "2024-01-02T03:04:05Z",
        "http://www.w3.org/2001/XMLSchema#dateTime",
    );
    let on = typed("2024-01-02Z", "http://www.w3.org/2001/XMLSchema#date");
    let g = vec![
        Triple::new(
            subject(),
            NamedNode::new("http://ex/flag").unwrap(),
            Term::Literal(flag.clone()),
        ),
        Triple::new(
            subject(),
            NamedNode::new("http://ex/at").unwrap(),
            Term::Literal(at.clone()),
        ),
        Triple::new(
            subject(),
            NamedNode::new("http://ex/on").unwrap(),
            Term::Literal(on.clone()),
        ),
    ];
    let salt = salt_from_bytes(&[3u8; 32]);
    let committed = dual_leaf_v2::commit_triples(&g, salt)
        .expect("every literal is on a routed lane and canonically spelled");
    assert_eq!(committed.leaves.len(), 3);

    // Each lane's FILTER builder presents, as `operand_enc`, exactly the TERM
    // encoding the committed graph holds for that object — which is what a scan
    // proof discloses for the operand column.
    let built_flag = build_filter_value_dl_bool(&flag, FilterOp::Eq, true).unwrap();
    let built_at = build_filter_value_dl_datetime(&at, FilterOp::Ge, &at).unwrap();
    let built_on = build_filter_value_dl_date(&on, FilterOp::Ge, &on).unwrap();
    for (lane, literal, inputs) in [
        ("xsd:boolean", &flag, &built_flag.inputs),
        ("xsd:dateTime", &at, &built_at.inputs),
        ("xsd:date", &on, &built_on.inputs),
    ] {
        let operand_enc = match inputs {
            ProofInputs::FilterValueDlBool { operand_enc, .. }
            | ProofInputs::FilterValueDlDateTime { operand_enc, .. } => operand_enc,
            other => panic!("a value-lane variant, got {other:?}"),
        };
        let committed_term = dual_leaf_v2::encode_term(&Term::Literal(literal.clone()), &salt)
            .expect("a routed lane's canonical literal encodes")
            .expect("a literal is not a triple term");
        assert_eq!(
            committed_term,
            fr(operand_enc),
            "{} must be committed as its lane encoder's leaf",
            lane
        );
        // And that term encoding is the leaf the lane ENCODER builds, not a
        // degenerate string-lane one.
        assert_ne!(
            committed_term,
            sparq_zk::poseidon2::hash(&[
                dual_leaf_v2::degenerate_value_component(literal.datatype().as_str(), None)
                    .unwrap(),
                field_from_hash_bytes(blake3::hash(literal.to_string().as_bytes()).as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ]),
            "{} must no longer take the degenerate lane",
            lane
        );
    }

    // And a literal on one of those lanes whose lexical the lane encoder rejects
    // fails the WHOLE commitment — the §6 fail-closed rule, which routing extends
    // to these three lanes.
    let non_canonical = vec![Triple::new(
        subject(),
        NamedNode::new("http://ex/flag").unwrap(),
        Term::Literal(typed("1", XSD_BOOLEAN)),
    )];
    assert!(
        dual_leaf_v2::commit_triples(&non_canonical, salt).is_err(),
        "a non-canonical boolean must not be committed on the string lane instead"
    );
}

// ====== LIVE: the numeric build helpers on their members =====================

/// The PRIVATE witness of a numeric build, for the raw hook-layout renderer.
fn hook_witness_of(
    built: &sparq_zk_compose::build::BuiltFilterValueDlNumeric,
) -> (&FieldHex, &FieldHex) {
    match &built.witness {
        LaneWitness::Hook(w) => (&w.value_hook, &w.lexical_component),
        other => panic!("the hook witness shape, got {other:?}"),
    }
}

/// LIVE, one helper per lane family: the HOOK-shaped `xsd:integer` lane and the
/// SIGN-FOLDED `xsd:decimal` lane, each proved through the checked typed path
/// (`prove_value_lane`, which renders its own `Prover.toml` from the validated
/// typed fields) and verified with `bb`. A flipped proof byte is the control that
/// `verify` decides something. The lying-verdict check is the next test.
#[test]
fn full_prove_verify_the_numeric_build_helpers() {
    use sparq_zk_compose::build::{build_filter_value_dl_decimal, build_filter_value_dl_int};
    if !support::require_toolchain("full_prove_verify_the_numeric_build_helpers", &["nargo", "bb"]) {
        return;
    }
    let prover = CircuitProver::from_crate_root();
    let integer = build_filter_value_dl_int(
        &typed("18", XSD_INTEGER),
        FilterOp::Ge,
        &typed("18", XSD_INTEGER),
    )
    .expect("canonical integer operands");
    let decimal = build_filter_value_dl_decimal(
        &typed("2.50", XSD_DECIMAL),
        FilterOp::Ge,
        &typed("1.5", XSD_DECIMAL),
    )
    .expect("canonical decimal operands");
    for (tag, built) in [("dlv2_num_int", &integer), ("dlv2_num_dec", &decimal)] {
        let proof = built.lane_proof().expect("the build assembles typed inputs");
        let out = unique_out_dir(tag);
        let art = prover
            .prove_value_lane(
                CommitmentMethod::DualLeafV2,
                MethodUse::NewIssuance,
                &challenge_one(),
                &proof,
                &out,
                tag,
            )
            .unwrap_or_else(|e| panic!("the honest {} assignment must prove: {:?}", tag, e));
        assert!(!art.proof.is_empty(), "bb must emit a proof");
        assert!(
            prover.verify(&art, &out.join("verify")).expect("bb verify runs"),
            "the honest {} proof must verify",
            tag
        );
        let mut bad = art.clone();
        let mid = bad.proof.len() / 2;
        bad.proof[mid] ^= 0xff;
        assert!(
            !prover
                .verify(&bad, &out.join("verify_bad"))
                .expect("bb verify runs"),
            "a tampered {} proof must be rejected",
            tag
        );
    }
}

/// LIVE: a LYING disclosed verdict on the integer lane built by the helper does
/// not solve a witness — so the verdict the helper computes is the one the member
/// enforces, not a free public input.
#[test]
fn in_circuit_a_lying_verdict_on_the_integer_build_helper_is_refused() {
    use sparq_zk_compose::build::build_filter_value_dl_int;
    if !support::require_toolchain("in_circuit_a_lying_verdict_on_the_integer_build_helper_is_refused", &["nargo"]) {
        return;
    }
    let built = build_filter_value_dl_int(
        &typed("18", XSD_INTEGER),
        FilterOp::Ge,
        &typed("18", XSD_INTEGER),
    )
    .expect("canonical integer operands");
    let ProofInputs::FilterValueDl {
        operand_enc,
        op,
        bound,
        datatype_const,
        expected,
        ..
    } = &built.inputs
    else {
        panic!("the integer builder must build FilterValueDl inputs");
    };
    assert!(*expected, "18 >= 18 is the honest verdict");
    let (value_hook, lexical_component) = hook_witness_of(&built);
    assert!(
        member_accepts(
            &CircuitId::FilterValueDl,
            "dlv2_num_int_ok",
            operand_enc,
            *op,
            *bound,
            datatype_const,
            *expected,
            value_hook,
            lexical_component,
        ),
        "the helper's honest integer witness must be provable"
    );
    assert!(
        !member_accepts(
            &CircuitId::FilterValueDl,
            "dlv2_num_int_lie",
            operand_enc,
            *op,
            *bound,
            datatype_const,
            !*expected,
            value_hook,
            lexical_component,
        ),
        "a flipped disclosed verdict must not solve"
    );
}
