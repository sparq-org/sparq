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
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::dual_leaf::{DualLeafError, LANG_NONE, XSD_DECIMAL, XSD_DOUBLE, XSD_INTEGER};
use sparq_zk::dual_leaf_boolean::XSD_BOOLEAN;
use sparq_zk::dual_leaf_v2;
use sparq_zk::encode::TYPE_CODE_LITERAL;
use sparq_zk::field::{field_from_hex_str, field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk_compose::build::{boolean_verdict, build_filter_value_dl_bool};
use sparq_zk_compose::dispatch::{resolve_circuit, DispatchError};
use sparq_zk_compose::manifest::{
    boolean_datatype_const, boolean_lane_const_v2, CircuitId, FieldHex, FilterOp, ProofInputs,
};
use sparq_zk_compose::toml::{filter_value_dl_bool_prover_toml, filter_value_dl_prover_toml};

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
        assert_eq!(p.id.package(), "filter_value_dl_bool");

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
        let degenerate = dual_leaf_v2::degenerate_value_component(datatype, None);
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
/// `(method × circuit)` legality matrix: the dual-leaf methods admit it, and a
/// `string-canonical` graph — which committed no value handle at all — refuses it.
/// The matrix is COARSE (per lane, not per scheme version); which VERSION's lane a
/// proof is on is carried by the public `datatype_const`, which the separation
/// tests above pin.
#[test]
fn the_v2_boolean_member_inherits_the_value_lane_dispatch_legality() {
    let id = CircuitId::FilterValueDlBool;
    assert_eq!(
        resolve_circuit(CommitmentMethod::DualLeafV2, &id),
        Ok(id.clone())
    );
    assert_eq!(
        resolve_circuit(CommitmentMethod::DualLeafV1, &id),
        Ok(id.clone())
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
    let challenge = FieldHex("0x01".to_string());

    let dedicated = filter_value_dl_bool_prover_toml(
        &challenge,
        p.operand_enc,
        p.op.code(),
        false,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    );
    let shared = filter_value_dl_prover_toml(
        &challenge,
        p.operand_enc,
        p.op.code(),
        p.bound,
        p.datatype_const,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    );
    assert_eq!(dedicated, shared);

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

    // The rendered constant is the V2 boolean one — a V1-lane render differs.
    let v1_lane = filter_value_dl_prover_toml(
        &challenge,
        p.operand_enc,
        p.op.code(),
        p.bound,
        &boolean_datatype_const(),
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    );
    assert_ne!(dedicated, v1_lane);
}

/// `bound = true` renders the hook `1` (the other half of the `{0, 1}` mapping).
#[test]
fn the_v2_prover_toml_renders_the_true_bound_hook() {
    let built = build_filter_value_dl_bool(&bool_lit(true), FilterOp::Eq, true).unwrap();
    let p = unpack(&built.inputs);
    let toml = filter_value_dl_bool_prover_toml(
        &FieldHex("0x01".to_string()),
        p.operand_enc,
        p.op.code(),
        true,
        p.expected,
        &built.value_hook,
        &built.lexical_component,
    );
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

fn toolchain_available() -> bool {
    std::process::Command::new("nargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

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
    let toml = filter_value_dl_prover_toml(
        &FieldHex("0x01".to_string()),
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
    if !toolchain_available() {
        eprintln!("nargo absent; skipping in-circuit V2 boolean-lane vectors");
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
            &FieldHex("0x01".to_string()),
            p.operand_enc,
            p.op.code(),
            false,
            p.expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        filter_value_dl_prover_toml(
            &FieldHex("0x01".to_string()),
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
/// committed for it satisfies the binding constraint, so the integer member
/// ACCEPTS that assignment. The dedicated member refuses both, in-circuit.
#[test]
fn in_circuit_the_dedicated_member_refuses_a_handle_outside_the_bit_domain() {
    if !toolchain_available() {
        eprintln!("nargo absent; skipping in-circuit V2 bit-domain vectors");
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
    // The dedicated member refuses the witnessed handle...
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
    if !toolchain_available() {
        eprintln!("nargo absent; skipping in-circuit V2 opaque-datatype vectors");
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
