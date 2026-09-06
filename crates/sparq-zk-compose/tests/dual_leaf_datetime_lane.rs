// [OPUS-5] sq-wz99x — CROSS-VECTORS for the DUAL-LEAF `xsd:dateTime` / `xsd:date`
//! value lane. The circuit half of the §13 dateTime/date pair (host half:
//! `sq-we9vs`, `sparq_zk::dual_leaf_datetime::{encode_datetime, encode_date}`),
//! mirroring the boolean-lane pair's vector shape.
//!
//! # What is being wired
//!
//! ONE new Noir relation, `filter_value_dl_datetime`, with EXACTLY the
//! `filter_value_dl_decimal` structure — the value handle is the SIGNED SCALED
//! EPOCH (milliseconds on the XSD proleptic-Gregorian `timeOnTimeline`, lane-fixed
//! `FS = 3`) instead of a scaled decimal magnitude, and the verdict comes from the
//! UNCHANGED `signed_scaled_verdict`. Because `datatype_const` is a PUBLIC input,
//! that ONE relation serves BOTH the `xsd:dateTime` and the `xsd:date` lane — the
//! host picks `blake3("<IRI>@epochscale=3")` per §13.3 and there is no second Noir
//! function and no second compiled member.
//!
//! # The invariant these vectors pin
//!
//! LANE SEPARATION IS THE PUBLIC `datatype_const`, AND ONLY THAT — and here that
//! is load-bearing in a way it was not for the boolean lane, because a date's hook
//! is the scaled epoch of its STARTING instant and is therefore NUMERICALLY EQUAL
//! to the dateTime hook of that same instant (`"1970-01-02Z"` and
//! `"1970-01-02T00:00:00Z"` both hook `86_400_000`). Two DISTINCT terms, ONE hook:
//! the only thing keeping them apart is the constant folded into
//! `value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)`. That is a BINDING
//! argument under Poseidon2 preimage resistance (a prover free to search preimages
//! is out of its scope), NOT an audited soundness claim.
//!
//! # Honest scope
//!
//! The host/serialisation cross-checks run everywhere. The genuinely IN-CIRCUIT
//! claims need real witness solving, so they live in the `nargo`-gated test at the
//! bottom and SKIP CLEANLY when the toolchain is absent (the `e2e.rs` pattern).
//! Nothing here is a soundness or privacy claim: the lane inherits the value
//! lane's INV-VL downgrade (value↔lexical agreement is trusted-issuer honesty, not
//! machine-enforced; #769 accepted), and the §13 rule set is itself an OPEN
//! external-audit obligation — the whole ZK estate is NOT externally audited
//! (gap CR-G8 / sq-qhy4).
#![cfg(feature = "dual-leaf")]

use oxrdf::{Literal, NamedNode};
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::dual_leaf::{DualLeafComponents, DualLeafError, LANG_NONE};
use sparq_zk::dual_leaf_datetime::{encode_date, encode_datetime, XSD_DATE, XSD_DATE_TIME};
use sparq_zk::encode::TYPE_CODE_LITERAL;
use sparq_zk::field::{field_from_hex_str, field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk_compose::build::{
    build_filter_value_dl_date, build_filter_value_dl_datetime, signed_epoch_verdict,
    BuiltFilterValueDlDateTime,
};
use sparq_zk_compose::dispatch::{resolve_circuit, DispatchError};
use sparq_zk_compose::manifest::{
    date_datatype_const, datetime_datatype_const, CircuitId, FieldHex, FilterOp, ProofInputs,
};
use sparq_zk_compose::toml::filter_value_dl_datetime_prover_toml;

const ALL_OPS: [FilterOp; 6] = [
    FilterOp::Lt,
    FilterOp::Le,
    FilterOp::Gt,
    FilterOp::Ge,
    FilterOp::Eq,
    FilterOp::Ne,
];

fn dt_lit(lexical: &str) -> Literal {
    Literal::new_typed_literal(lexical, NamedNode::new(XSD_DATE_TIME).unwrap())
}

fn date_lit(lexical: &str) -> Literal {
    Literal::new_typed_literal(lexical, NamedNode::new(XSD_DATE).unwrap())
}

fn fr(hex: &FieldHex) -> Fr {
    field_from_hex_str(&hex.0).expect("field hex emitted by this crate must parse")
}

/// The scaled-epoch MAGNITUDE the builder put on the wire, back as a `u64` — the
/// private witness is a small non-negative integer by construction (the sign lives
/// in `value_neg`), so the high 24 bytes must be zero.
fn magnitude(hex: &FieldHex) -> u64 {
    let b = sparq_zk::field::field_to_be_bytes_32(&fr(hex));
    assert!(
        b[..24].iter().all(|x| *x == 0),
        "the private magnitude must be a u64, not a wrapped field element"
    );
    u64::from_be_bytes(b[24..32].try_into().unwrap())
}

/// The member's OWN leaf recompute, transcribed from
/// `zk/compose/compose_core/src/filter_value.nr::filter_value_dl_datetime`:
/// it re-folds the sign into the handle by FIELD NEGATION, then
/// `leaf = h3(h3(signed_hook, datatype_const, LANG_NONE), lexical_component,
/// TYPE_CODE_LITERAL)`. Taking the three PRIVATE witnesses and the PUBLIC
/// `datatype_const` and rebuilding the leaf is exactly what the in-circuit
/// `assert_eq(leaf, operand_enc)` does, so equality here is the host mirror of
/// "the member's binding constraint is satisfied".
fn member_rebinds_leaf(
    value_neg: bool,
    value_mag: Fr,
    datatype_const: Fr,
    lexical_component: Fr,
) -> Fr {
    let signed = if value_neg { -value_mag } else { value_mag };
    let value_component = poseidon2::hash(&[signed, datatype_const, Fr::from(LANG_NONE)]);
    poseidon2::hash(&[
        value_component,
        lexical_component,
        Fr::from(TYPE_CODE_LITERAL),
    ])
}

struct Public<'a> {
    id: &'a CircuitId,
    operand_enc: &'a FieldHex,
    op: FilterOp,
    bound_neg: bool,
    bound_scaled_epoch: u64,
    datatype_const: &'a FieldHex,
    expected: bool,
}

fn unpack(inputs: &ProofInputs) -> Public<'_> {
    match inputs {
        ProofInputs::FilterValueDlDateTime {
            id,
            operand_enc,
            op,
            bound_neg,
            bound_scaled_epoch,
            datatype_const,
            expected,
        } => Public {
            id,
            operand_enc,
            op: *op,
            bound_neg: *bound_neg,
            bound_scaled_epoch: *bound_scaled_epoch,
            datatype_const,
            expected: *expected,
        },
        other => panic!("the dateTime/date lane must build FilterValueDlDateTime, got {other:?}"),
    }
}

// =========================================================================
// 1. The lane constants — the compose-side pick MUST equal the host encoder's
// =========================================================================

/// DRIFT GUARD. `manifest::{datetime,date}_datatype_const()` is what the compose
/// crate puts on the wire; the `dual_leaf_datetime` encoders are what the
/// COMMITTER folded into the leaf. If those ever diverge, every dateTime/date
/// proof becomes silently unprovable with no compile-time signal.
#[test]
fn lane_constants_equal_the_host_encoders_datatype_const() {
    assert_eq!(
        fr(&datetime_datatype_const()),
        sparq_zk::dual_leaf_datetime::datetime_datatype_const(),
        "the compose-side dateTime constant must be the encoder's"
    );
    assert_eq!(
        fr(&date_datatype_const()),
        sparq_zk::dual_leaf_datetime::date_datatype_const(),
        "the compose-side date constant must be the encoder's"
    );
    // ...and the two lanes are genuinely distinct constants (§13.3).
    assert_ne!(
        datetime_datatype_const().0,
        date_datatype_const().0,
        "xsd:date must not share the xsd:dateTime lane constant"
    );
    // Each is also what the encoder actually folded into a committed leaf.
    assert_eq!(
        encode_datetime(&dt_lit("2001-09-09T01:46:40Z"))
            .unwrap()
            .datatype_const,
        fr(&datetime_datatype_const())
    );
    assert_eq!(
        encode_date(&date_lit("2001-09-09Z")).unwrap().datatype_const,
        fr(&date_datatype_const())
    );
}

// =========================================================================
// 2. The binding — the builder's witnesses rebind to the committed operand_enc
// =========================================================================

/// The host mirror of the in-circuit `assert_eq(leaf, operand_enc)`, over the
/// four corners of the signed domain (post-epoch, pre-epoch, the epoch itself,
/// sub-second precision) and both lanes. The builder splits the encoder's SIGNED
/// hook into the member's `(value_neg, value_hook_scaled)` shape, so this also
/// pins that the split is the exact inverse of the encoder's field negation.
#[test]
fn builder_witnesses_rebind_to_the_committed_operand_enc() {
    let cases: Vec<(BuiltFilterValueDlDateTime, Fr)> = vec![
        (
            build_filter_value_dl_datetime(
                &dt_lit("2001-09-09T01:46:40Z"),
                FilterOp::Ge,
                &dt_lit("1970-01-01T00:00:00Z"),
            )
            .unwrap(),
            fr(&datetime_datatype_const()),
        ),
        (
            // PRE-epoch: the sign is folded by field negation.
            build_filter_value_dl_datetime(
                &dt_lit("1969-12-31T23:59:59.5Z"),
                FilterOp::Lt,
                &dt_lit("1970-01-01T00:00:00Z"),
            )
            .unwrap(),
            fr(&datetime_datatype_const()),
        ),
        (
            // The epoch itself — magnitude 0, and never `-0`.
            build_filter_value_dl_datetime(
                &dt_lit("1970-01-01T00:00:00Z"),
                FilterOp::Eq,
                &dt_lit("1970-01-01T00:00:00Z"),
            )
            .unwrap(),
            fr(&datetime_datatype_const()),
        ),
        (
            build_filter_value_dl_date(&date_lit("1969-12-31Z"), FilterOp::Lt, &date_lit("1970-01-01Z"))
                .unwrap(),
            fr(&date_datatype_const()),
        ),
    ];
    for (built, dt_const) in cases {
        let p = unpack(&built.inputs);
        assert_eq!(*p.id, CircuitId::FilterValueDlDateTime);
        assert_eq!(fr(p.datatype_const), dt_const);
        assert_eq!(
            member_rebinds_leaf(
                built.value_neg,
                fr(&built.value_hook_scaled),
                dt_const,
                fr(&built.lexical_component),
            ),
            fr(p.operand_enc),
            "the builder's private witnesses must rebind to the committed leaf"
        );
    }
}

/// The epoch instant must be witnessed NON-negative — the member fails closed on
/// a negative sign over a zero magnitude ("negative zero is not canonical"), so an
/// honest builder must never emit that pair.
#[test]
fn the_epoch_is_never_witnessed_as_negative_zero() {
    let built = build_filter_value_dl_datetime(
        &dt_lit("1970-01-01T00:00:00Z"),
        FilterOp::Eq,
        &dt_lit("1970-01-01T00:00:00Z"),
    )
    .unwrap();
    assert!(!built.value_neg, "a zero magnitude must report non-negative");
    assert_eq!(fr(&built.value_hook_scaled), Fr::from(0u64));
    let p = unpack(&built.inputs);
    assert!(!p.bound_neg && p.bound_scaled_epoch == 0);
}

// =========================================================================
// 3. THE lane separation — one hook, two terms, two leaves
// =========================================================================

/// THE §13.3 INVARIANT. A date's value handle is the scaled epoch of its STARTING
/// instant, so `"1970-01-02Z"` and `"1970-01-02T00:00:00Z"` carry the SAME hook.
/// They are still distinct terms; only the public lane constant separates them, so
/// the two leaves differ and neither honest witness rebinds under the other's
/// constant. (Host mirror; the in-circuit half is the toolchain-gated test below.)
#[test]
fn a_date_and_its_starting_instant_share_a_hook_but_never_a_leaf() {
    let date = encode_date(&date_lit("1970-01-02Z")).unwrap();
    let instant = encode_datetime(&dt_lit("1970-01-02T00:00:00Z")).unwrap();

    assert_eq!(
        date.value_hook, instant.value_hook,
        "the two lanes DO collide on the raw hook — that is why the constant matters"
    );
    assert_eq!(date.value_hook, Fr::from(86_400_000u64));
    assert_ne!(date.value_component(), instant.value_component());
    assert_ne!(date.leaf(), instant.leaf());

    // Cross-lane rebinding fails in BOTH directions: the ONLY thing changed is the
    // public constant the same honest witnesses are presented against.
    let mag = Fr::from(86_400_000u64);
    assert_eq!(
        member_rebinds_leaf(false, mag, date.datatype_const, date.lexical_component),
        date.leaf(),
        "control: the date witness rebinds on its OWN lane"
    );
    assert_ne!(
        member_rebinds_leaf(false, mag, instant.datatype_const, date.lexical_component),
        date.leaf(),
        "a date leaf must not rebind under the dateTime constant"
    );
    assert_ne!(
        member_rebinds_leaf(false, mag, date.datatype_const, instant.lexical_component),
        instant.leaf(),
        "a dateTime leaf must not rebind under the date constant"
    );
}

/// The sign is part of the handle: a prover witnessing the same magnitude with a
/// flipped sign rebinds to a DIFFERENT leaf, so a committed instant cannot be
/// moved across the epoch.
#[test]
fn a_sign_flip_unbinds_the_leaf() {
    let c = encode_datetime(&dt_lit("1970-01-01T00:00:00.5Z")).unwrap();
    let mag = Fr::from(500u64);
    assert_eq!(
        member_rebinds_leaf(false, mag, c.datatype_const, c.lexical_component),
        c.leaf()
    );
    assert_ne!(
        member_rebinds_leaf(true, mag, c.datatype_const, c.lexical_component),
        c.leaf(),
        "flipping the sign of the committed instant must break the binding"
    );
}

// =========================================================================
// 4. The verdict oracle — the XSD timeOnTimeline order
// =========================================================================

/// `signed_epoch_verdict` is the host mirror of the member's UNCHANGED
/// `signed_scaled_verdict`. Pin it against a straightforward `i128` timeline
/// comparison over the same operands — including the PRE-epoch corner, where the
/// LARGER magnitude is the EARLIER instant (the arm a naive magnitude compare
/// gets backwards).
#[test]
fn signed_epoch_verdict_is_the_signed_timeline_order() {
    let operands: [(bool, u64); 7] = [
        (false, 0),
        (false, 1),
        (false, 86_400_000),
        (false, 1_000_000_000_000),
        (true, 1),
        (true, 500),
        (true, 86_400_000),
    ];
    let as_i128 = |(neg, mag): (bool, u64)| {
        let m = i128::from(mag);
        if neg {
            -m
        } else {
            m
        }
    };
    for v in operands {
        for b in operands {
            let (vi, bi) = (as_i128(v), as_i128(b));
            for op in ALL_OPS {
                let want = match op {
                    FilterOp::Lt => vi < bi,
                    FilterOp::Le => vi <= bi,
                    FilterOp::Gt => vi > bi,
                    FilterOp::Ge => vi >= bi,
                    FilterOp::Eq => vi == bi,
                    FilterOp::Ne => vi != bi,
                };
                assert_eq!(
                    signed_epoch_verdict(v.0, v.1, b.0, b.1, op),
                    want,
                    "verdict mismatch for {v:?} {op:?} {b:?}"
                );
            }
        }
    }
}

/// Lexicals of DIFFERING sub-second precision land in ONE scaled domain under ONE
/// constant, so a single signed comparison orders them (the member-fixed-`FS`
/// point, §13.1) — the property the whole scaled-epoch design exists for.
#[test]
fn cross_precision_operands_are_ordered_by_the_one_member() {
    let built = build_filter_value_dl_datetime(
        &dt_lit("2020-06-01T12:00:00Z"),
        FilterOp::Lt,
        &dt_lit("2020-06-01T12:00:00.5Z"),
    )
    .unwrap();
    let p = unpack(&built.inputs);
    assert!(p.expected, "T12:00:00Z is 500 ms BEFORE T12:00:00.5Z");
    assert!(!built.value_neg && !p.bound_neg);
    assert_eq!(
        p.bound_scaled_epoch,
        magnitude(&built.value_hook_scaled) + 500,
        "the two precisions differ by exactly 500 ms on the shared timeline"
    );
}

/// The builder DISCLOSES the honest verdict rather than trusting the caller, for
/// every operator and across the epoch — so an honest host cannot accidentally
/// publish a verdict the member will refuse to prove.
#[test]
fn builder_discloses_the_honest_verdict_for_every_op() {
    for (value, bound) in [
        ("2020-01-01T00:00:00Z", "2019-01-01T00:00:00Z"),
        ("2019-01-01T00:00:00Z", "2020-01-01T00:00:00Z"),
        ("2020-01-01T00:00:00Z", "2020-01-01T00:00:00Z"),
        ("1969-01-01T00:00:00Z", "1970-01-01T00:00:00Z"),
        ("1969-01-01T00:00:00Z", "1968-01-01T00:00:00Z"),
    ] {
        for op in ALL_OPS {
            let built =
                build_filter_value_dl_datetime(&dt_lit(value), op, &dt_lit(bound)).unwrap();
            let p = unpack(&built.inputs);
            let want = signed_epoch_verdict(
                built.value_neg,
                magnitude(&built.value_hook_scaled),
                p.bound_neg,
                p.bound_scaled_epoch,
                op,
            );
            assert_eq!(p.expected, want, "{value} {op:?} {bound}");
        }
    }
}

// =========================================================================
// 5. Fail-closed — the §13.4 domain, on BOTH operands
// =========================================================================

/// Every lexical outside the hookable domain is REJECTED at the builder, for the
/// HIDDEN operand AND for the FILTER's own constant. The bound matters just as
/// much: comparing against a bare or offset instant would be exactly the
/// order-INDETERMINATE comparison §13.2 exists to refuse.
#[test]
fn non_canonical_lexicals_are_fail_closed_on_both_operands() {
    let ok = dt_lit("2020-01-01T00:00:00Z");
    for bad in [
        "2020-01-01T12:00:00",       // bare — order-indeterminate (§13.2(1))
        "2020-01-01T12:00:00+01:00", // non-Z offset (§13.2(2))
        "2020-01-01T12:00:00+00:00", // spells Z but is NON-canonical
        "2020-01-01T24:00:00Z",      // two lexicals for one value
        "2016-12-31T23:59:60Z",      // leap second — not an XSD lexical
        "2020-01-01T12:00:00.1234Z", // more than FS digits — would need rounding
        "2020-01-01T12:00:00.500Z",  // trailing zero — non-canonical
        "2023-02-29T00:00:00Z",      // not a real calendar day
        "999999999999-01-01T00:00:00Z", // scaled epoch overflows u64
    ] {
        assert!(
            matches!(
                build_filter_value_dl_datetime(&dt_lit(bad), FilterOp::Lt, &ok),
                Err(DualLeafError::NonCanonicalValue(_))
            ),
            "hidden operand {bad:?} must be fail-closed"
        );
        assert!(
            matches!(
                build_filter_value_dl_datetime(&ok, FilterOp::Lt, &dt_lit(bad)),
                Err(DualLeafError::NonCanonicalValue(_))
            ),
            "FILTER bound {bad:?} must be fail-closed"
        );
    }
    // Bare dates too — same §13.2 reason.
    assert!(matches!(
        build_filter_value_dl_date(&date_lit("2020-01-01"), FilterOp::Lt, &date_lit("2021-01-01Z")),
        Err(DualLeafError::NonCanonicalValue(_))
    ));
}

/// A cross-LANE misuse is refused by the encoder's datatype check before any
/// comparison is built, so this API cannot express a `date`-vs-`dateTime`
/// comparison at all — the host-side face of the in-circuit lane separation.
#[test]
fn cross_lane_operands_are_structurally_inexpressible() {
    assert!(matches!(
        build_filter_value_dl_datetime(
            &date_lit("2020-01-01Z"),
            FilterOp::Lt,
            &dt_lit("2020-01-01T00:00:00Z")
        ),
        Err(DualLeafError::NotValueLane(_))
    ));
    assert!(matches!(
        build_filter_value_dl_datetime(
            &dt_lit("2020-01-01T00:00:00Z"),
            FilterOp::Lt,
            &date_lit("2020-01-01Z")
        ),
        Err(DualLeafError::NotValueLane(_))
    ));
    assert!(matches!(
        build_filter_value_dl_date(
            &date_lit("2020-01-01Z"),
            FilterOp::Lt,
            &dt_lit("2020-01-01T00:00:00Z")
        ),
        Err(DualLeafError::NotValueLane(_))
    ));
}

// =========================================================================
// 6. Dispatch + wire format
// =========================================================================

/// The member is a VALUE-LANE member: legal only against a method that committed
/// a value handle, fail-closed against `string-canonical` (which has none).
#[test]
fn datetime_lane_inherits_the_value_lane_dispatch_legality() {
    assert_eq!(
        resolve_circuit(
            CommitmentMethod::DualLeafV1,
            &CircuitId::FilterValueDlDateTime
        ),
        Ok(CircuitId::FilterValueDlDateTime)
    );
    assert!(matches!(
        resolve_circuit(
            CommitmentMethod::StringCanonicalV1,
            &CircuitId::FilterValueDlDateTime
        ),
        Err(DispatchError::IllegalPair { .. })
    ));
}

/// The `Prover.toml` field ORDER is the member `main`'s declaration order — the
/// audit-#1 discipline. A reorder here silently desyncs the proof from the
/// verifier's public-input reconstruction, so pin it.
#[test]
fn prover_toml_matches_the_member_main_declaration_order() {
    let built = build_filter_value_dl_datetime(
        &dt_lit("1969-12-31T23:59:59.5Z"),
        FilterOp::Lt,
        &dt_lit("1970-01-01T00:00:00Z"),
    )
    .unwrap();
    let p = unpack(&built.inputs);
    let toml = filter_value_dl_datetime_prover_toml(
        &FieldHex("0x01".to_string()),
        p.operand_enc,
        p.op.code(),
        p.bound_neg,
        p.bound_scaled_epoch,
        p.datatype_const,
        p.expected,
        built.value_neg,
        &built.value_hook_scaled,
        &built.lexical_component,
    );
    let lines: Vec<&str> = toml.lines().collect();
    assert!(lines[0].starts_with("challenge = "));
    assert!(lines[1].starts_with("operand_enc = "));
    assert!(lines[2].starts_with("op = "));
    assert!(lines[3].starts_with("bound_neg = "));
    assert!(lines[4].starts_with("bound_scaled_epoch = "));
    assert!(lines[5].starts_with("datatype_const = "));
    assert!(lines[6].starts_with("expected = "));
    assert!(lines[7] == "value_neg = true", "pre-epoch value is negative");
    assert!(lines[8].starts_with("value_hook_scaled = "));
    assert!(lines[9].starts_with("lexical_component = "));
    assert_eq!(lines.len(), 10, "no stray field");
}

/// Both lanes ride the SAME wire tag (one member), and the recorded
/// `datatype_const` is what tells them apart on the wire.
#[test]
fn both_lanes_round_trip_on_the_shared_wire_tag() {
    for (built, want_const) in [
        (
            build_filter_value_dl_datetime(
                &dt_lit("2020-01-01T00:00:00Z"),
                FilterOp::Ge,
                &dt_lit("2019-01-01T00:00:00Z"),
            )
            .unwrap(),
            datetime_datatype_const(),
        ),
        (
            build_filter_value_dl_date(
                &date_lit("2020-01-01Z"),
                FilterOp::Ge,
                &date_lit("2019-01-01Z"),
            )
            .unwrap(),
            date_datatype_const(),
        ),
    ] {
        let json = serde_json::to_string(&built.inputs).unwrap();
        assert!(json.contains("\"circuit\":\"filter_value_dl_datetime\""));
        assert!(json.contains(&want_const.0));
        let back: ProofInputs = serde_json::from_str(&json).unwrap();
        assert_eq!(back, built.inputs);
    }
    assert_eq!(
        CircuitId::FilterValueDlDateTime.package(),
        "filter_value_dl_datetime"
    );
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

/// Solve `filter_value_dl_datetime`'s witness for the given public/private
/// assignment. `Ok` ⇔ the relation is SATISFIABLE ⇔ the member ACCEPTS.
#[allow(clippy::too_many_arguments)]
fn member_accepts(
    tag: &str,
    operand_enc: &FieldHex,
    op: FilterOp,
    bound_neg: bool,
    bound_scaled_epoch: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: &FieldHex,
    lexical_component: &FieldHex,
) -> bool {
    let toml = filter_value_dl_datetime_prover_toml(
        &FieldHex("0x01".to_string()),
        operand_enc,
        op.code(),
        bound_neg,
        bound_scaled_epoch,
        datatype_const,
        expected,
        value_neg,
        value_hook_scaled,
        lexical_component,
    );
    sparq_zk_compose::driver::CircuitProver::from_crate_root()
        .gen_witness_tagged(&CircuitId::FilterValueDlDateTime, &toml, tag)
        .is_ok()
}

fn accepts(tag: &str, built: &BuiltFilterValueDlDateTime) -> bool {
    let p = unpack(&built.inputs);
    member_accepts(
        tag,
        p.operand_enc,
        p.op,
        p.bound_neg,
        p.bound_scaled_epoch,
        p.datatype_const,
        p.expected,
        built.value_neg,
        &built.value_hook_scaled,
        &built.lexical_component,
    )
}

/// The in-circuit claims, in one toolchain-gated pass:
///   (a) the honest dateTime witness IS provable, post- AND pre-epoch;
///   (b) a LYING disclosed verdict is NOT (`filter verdict mismatch`);
///   (c) a SIGN FLIP over the same magnitude is NOT (`dual-leaf operand encoding
///       mismatch`) — the committed instant cannot be moved across the epoch;
///   (d) a `xsd:date` leaf is NOT provable under the dateTime `datatype_const`,
///       and symmetrically — even though the two hooks are NUMERICALLY EQUAL.
///       This is THE §13.3 lane-separation claim, in-circuit.
#[test]
fn in_circuit_datetime_lane_accepts_honest_and_rejects_lies() {
    if !toolchain_available() {
        eprintln!("nargo absent; skipping sq-wz99x in-circuit dateTime-lane vectors");
        return;
    }

    // (a) honest, post-epoch.
    let post = build_filter_value_dl_datetime(
        &dt_lit("2001-09-09T01:46:40Z"),
        FilterOp::Ge,
        &dt_lit("1970-01-02T00:00:00Z"),
    )
    .unwrap();
    assert!(unpack(&post.inputs).expected);
    assert!(
        accepts("sqwz99x_post", &post),
        "the honest post-epoch witness must be provable"
    );

    // (a') honest, PRE-epoch — the sign-folded half of the domain.
    let pre = build_filter_value_dl_datetime(
        &dt_lit("1969-12-31T23:59:59.5Z"),
        FilterOp::Lt,
        &dt_lit("1970-01-01T00:00:00Z"),
    )
    .unwrap();
    assert!(pre.value_neg && unpack(&pre.inputs).expected);
    assert!(
        accepts("sqwz99x_pre", &pre),
        "the honest pre-epoch witness must be provable"
    );

    // (b) lying verdict — the ONLY change is `expected`.
    let p = unpack(&post.inputs);
    assert!(
        !member_accepts(
            "sqwz99x_lie",
            p.operand_enc,
            p.op,
            p.bound_neg,
            p.bound_scaled_epoch,
            p.datatype_const,
            !p.expected,
            post.value_neg,
            &post.value_hook_scaled,
            &post.lexical_component,
        ),
        "a flipped disclosed verdict must be unprovable"
    );

    // (c) sign flip — the ONLY change is `value_neg`.
    assert!(
        !member_accepts(
            "sqwz99x_signflip",
            p.operand_enc,
            p.op,
            p.bound_neg,
            p.bound_scaled_epoch,
            p.datatype_const,
            p.expected,
            !post.value_neg,
            &post.value_hook_scaled,
            &post.lexical_component,
        ),
        "flipping the witnessed sign must break the leaf binding"
    );

    // (d) THE lane separation, in-circuit. `"1970-01-02Z"` and
    //     `"1970-01-02T00:00:00Z"` have the SAME hook, so only the public constant
    //     can separate them.
    let date = build_filter_value_dl_date(
        &date_lit("1970-01-02Z"),
        FilterOp::Eq,
        &date_lit("1970-01-02Z"),
    )
    .unwrap();
    let dp = unpack(&date.inputs);
    let instant: DualLeafComponents = encode_datetime(&dt_lit("1970-01-02T00:00:00Z")).unwrap();
    assert_eq!(
        fr(&date.value_hook_scaled),
        instant.value_hook,
        "precondition: the two lanes share the raw hook"
    );
    assert!(
        accepts("sqwz99x_date_ok", &date),
        "control: the date witness IS provable on its OWN lane"
    );
    assert!(
        !member_accepts(
            "sqwz99x_date_on_dt",
            dp.operand_enc,
            dp.op,
            dp.bound_neg,
            dp.bound_scaled_epoch,
            &datetime_datatype_const(),
            dp.expected,
            date.value_neg,
            &date.value_hook_scaled,
            &date.lexical_component,
        ),
        "an xsd:date leaf must be unprovable under the xsd:dateTime datatype_const"
    );
    assert!(
        !member_accepts(
            "sqwz99x_dt_on_date",
            &FieldHex(field_to_hex(&instant.leaf())),
            dp.op,
            dp.bound_neg,
            dp.bound_scaled_epoch,
            &date_datatype_const(),
            dp.expected,
            false,
            &FieldHex(field_to_hex(&instant.value_hook)),
            &FieldHex(field_to_hex(&instant.lexical_component)),
        ),
        "an xsd:dateTime leaf must be unprovable under the xsd:date datatype_const"
    );
}
