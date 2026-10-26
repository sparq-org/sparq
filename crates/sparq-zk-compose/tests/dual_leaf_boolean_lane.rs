// sq-5xdlk — CROSS-VECTORS for the DUAL-LEAF `xsd:boolean` value lane.
//! The circuit half of the boolean-lane pair (host half: `sq-hh7a4`,
//! `sparq_zk::dual_leaf_boolean::encode_boolean`).
//!
//! # What is being wired (and what is NOT)
//!
//! There is **NO boolean Noir relation**. `filter_value_dl_int` already takes
//! `datatype_const` as a PUBLIC input and compares over `u64`, and the boolean
//! value hooks are `{0 = false, 1 = true}` — inside that domain. So the boolean
//! lane is pure WIRING: the host and the verifier pick the boolean lane
//! constant instead of `datatype_const(xsd:integer)`, and
//! everything else — the member, its `main` layout, its compiled artifact, its
//! gate count — is byte-for-byte the integer lane's. (No new member crate ⇒ no bb
//! gate re-measure, per the epic-owner design call sq-1s2.1.)
//!
//! # The invariant these vectors pin
//!
//! LANE SEPARATION IS THE PUBLIC `datatype_const`, AND ONLY THAT. Because it is
//! folded into `value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)`, the
//! honest witness for a committed `"1"^^xsd:integer` leaf recomputes a DIFFERENT
//! leaf under the boolean constant and so fails the member's
//! `assert_eq(leaf, operand_enc)` — and symmetrically. That is a BINDING argument
//! under Poseidon2 preimage resistance (a prover free to search preimages is out
//! of its scope), NOT an audited soundness claim.
//!
//! # Honest scope
//!
//! These vectors are the host/serialisation cross-checks, which run everywhere.
//! The genuinely IN-CIRCUIT claims — "the honest boolean witness is provable",
//! "a lying verdict is unprovable", "an integer leaf is unprovable on the boolean
//! lane" — need real witness solving, so they live in the `nargo`-gated tests at
//! the bottom and SKIP CLEANLY when the toolchain is absent (the established
//! `e2e.rs` pattern). Nothing here is a soundness or privacy claim: the boolean
//! lane inherits the value lane's INV-VL downgrade (value↔lexical agreement is
//! trusted-issuer honesty, not machine-enforced; #769 accepted) and the whole ZK
//! estate is NOT externally audited (gap CR-G8 / sq-qhy4).
#![cfg(feature = "dual-leaf")]

use oxrdf::{Literal, NamedNode};
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::dual_leaf::{
    datatype_const, DualLeafComponents, DualLeafError, LANG_NONE, XSD_INTEGER,
};
use sparq_zk::dual_leaf_boolean::{encode_boolean, XSD_BOOLEAN};
use sparq_zk::encode::TYPE_CODE_LITERAL;
use sparq_zk::field::{field_from_hash_bytes, field_from_hex_str, field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk_compose::build::boolean_verdict;
use sparq_zk_compose::legacy_v1::{
    boolean_datatype_const, build_filter_value_dl_boolean, filter_value_dl_boolean_prover_toml,
    filter_value_dl_prover_toml, BuildValueDlV1Error,
};
use sparq_zk_compose::dispatch::{resolve_circuit, DispatchError};
use sparq_zk_compose::manifest::{
    CircuitId, FieldHex, FilterOp, ProofInputs,
};

fn typed(lexical: &str, datatype: &str) -> Literal {
    Literal::new_typed_literal(lexical, NamedNode::new(datatype).unwrap())
}

fn bool_lit(v: bool) -> Literal {
    typed(if v { "true" } else { "false" }, XSD_BOOLEAN)
}

/// The string-canonical `h_s` a string-lane leaf carries in its lexical slot.
fn lexical_of(lit: &Literal) -> Fr {
    field_from_hash_bytes(blake3::hash(lit.to_string().as_bytes()).as_bytes())
}

fn fr(hex: &FieldHex) -> Fr {
    field_from_hex_str(&hex.0).expect("field hex emitted by this crate must parse")
}

/// The member's OWN leaf recompute, transcribed from
/// `zk/compose/compose_core/src/filter_value.nr::filter_value_dl_int`:
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

/// The shared value-lane public inputs, borrowed. The boolean lane REUSES the
/// integer member's variant — that reuse is itself part of the contract under
/// test, so [`unpack`] panics on anything else.
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
        ProofInputs::FilterValueDl { id, operand_enc, op, bound, datatype_const, expected } => {
            Public {
                id,
                operand_enc,
                op: *op,
                bound: *bound,
                datatype_const,
                expected: *expected,
            }
        }
        other => panic!("boolean lane must build the shared FilterValueDl inputs, got {other:?}"),
    }
}

// =========================================================================
// 1. The lane constant — the compose-side pick MUST equal the host encoder's
// =========================================================================

/// DRIFT GUARD. `legacy_v1::boolean_datatype_const()` is what the compose crate
/// puts on the wire; `encode_boolean` is what the COMMITTER folded into the leaf.
/// If those ever diverge, every boolean proof becomes silently unprovable with no
/// compile-time signal — so pin them to each other AND to the BLAKE3
/// key derivation over the `xsd:boolean` IRI.
#[test]
fn lane_constant_equals_the_host_encoders_datatype_const() {
    let committed = encode_boolean(&bool_lit(true)).unwrap().datatype_const;
    assert_eq!(fr(&boolean_datatype_const()), committed);
    assert_eq!(
        fr(&boolean_datatype_const()),
        sparq_zk::dual_leaf_boolean::boolean_datatype_const()
    );
    // ...and it is NOT the integer lane's constant. This inequality is what the
    // whole separation argument rests on.
    assert_ne!(fr(&boolean_datatype_const()), datatype_const(XSD_INTEGER));
}

// =========================================================================
// 2. encode_boolean -> member accepts (the binding constraint is satisfied)
// =========================================================================

/// CROSS-VECTOR: for both boolean terms, the leaf `encode_boolean` committed is
/// EXACTLY the leaf the member rebuilds from the witnesses + public
/// `datatype_const` the builder emitted. This is the in-circuit
/// `assert_eq(leaf, operand_enc)` evaluated host-side.
#[test]
fn encode_boolean_witnesses_rebind_to_the_committed_operand_enc() {
    for value in [false, true] {
        let lit = bool_lit(value);
        let committed = encode_boolean(&lit).unwrap();
        let built = build_filter_value_dl_boolean(&lit, FilterOp::Eq, true).unwrap();
        let p = unpack(&built.inputs);
        let (id, operand_enc, dt) = (p.id, p.operand_enc, p.datatype_const);

        // The lane targets the EXISTING integer member — no new circuit exists.
        assert_eq!(id, &CircuitId::FilterValueDl);
        assert_eq!(id.package().unwrap(), "filter_value_dl_int");

        // The public anchor is the committed dual leaf.
        assert_eq!(fr(operand_enc), committed.leaf());
        // The private witnesses are the committer's own components.
        assert_eq!(fr(&built.value_hook), committed.value_hook);
        assert_eq!(fr(&built.lexical_component), committed.lexical_component);
        // §3.3: VALUE_HOOK is 1 for true, 0 for false.
        assert_eq!(fr(&built.value_hook), Fr::from(u64::from(value)));

        // The member's binding constraint holds.
        assert_eq!(
            member_rebinds_leaf(fr(&built.value_hook), fr(dt), fr(&built.lexical_component)),
            fr(operand_enc),
        );
    }
}

// =========================================================================
// 3. int <-> boolean datatype_const SEPARATION
// =========================================================================

/// The load-bearing separation, both directions: an honest witness for one lane's
/// committed leaf does NOT satisfy the other lane's member call, because the
/// public `datatype_const` it must fold in is different. `"1"^^xsd:integer` and
/// `"true"^^xsd:boolean` are the adversarially-chosen pair — they share the value
/// hook `1`, so ONLY `datatype_const` can separate them.
#[test]
fn an_integer_leaf_cannot_satisfy_a_boolean_member_call_and_vice_versa() {
    let bool_c = encode_boolean(&bool_lit(true)).unwrap();
    let int_lit = typed("1", XSD_INTEGER);
    let int_c = sparq_zk::dual_leaf::encode_literal(&int_lit).unwrap();

    // Same VALUE_HOOK — the hazard this test exists for.
    assert_eq!(bool_c.value_hook, int_c.value_hook);
    assert_eq!(bool_c.value_hook, Fr::from(1u64));
    // Different datatype constants => different value components => different leaves.
    assert_ne!(bool_c.datatype_const, int_c.datatype_const);
    assert_ne!(bool_c.value_component(), int_c.value_component());
    assert_ne!(bool_c.leaf(), int_c.leaf());

    // (a) integer leaf presented to a BOOLEAN member call: the prover's honest
    //     witnesses rebind to something other than the committed integer leaf, so
    //     `assert_eq(leaf, operand_enc)` fails.
    assert_ne!(
        member_rebinds_leaf(
            int_c.value_hook,
            fr(&boolean_datatype_const()),
            int_c.lexical_component,
        ),
        int_c.leaf(),
    );
    // (b) boolean leaf presented to an INTEGER member call: symmetric.
    assert_ne!(
        member_rebinds_leaf(
            bool_c.value_hook,
            datatype_const(XSD_INTEGER),
            bool_c.lexical_component,
        ),
        bool_c.leaf(),
    );

    // Sanity that (a)/(b) are NOT vacuous: each witness set DOES rebind under its
    // OWN lane constant. If the recompute were broken these would fail too.
    assert_eq!(
        member_rebinds_leaf(int_c.value_hook, int_c.datatype_const, int_c.lexical_component),
        int_c.leaf(),
    );
    assert_eq!(
        member_rebinds_leaf(bool_c.value_hook, bool_c.datatype_const, bool_c.lexical_component),
        bool_c.leaf(),
    );
}

/// The `"false"`/`"0"` half of the same pair — a committed `"0"^^xsd:integer`
/// cannot answer a boolean member call either.
#[test]
fn the_zero_hook_pair_is_separated_too() {
    let bool_c = encode_boolean(&bool_lit(false)).unwrap();
    let int_c = sparq_zk::dual_leaf::encode_literal(&typed("0", XSD_INTEGER)).unwrap();
    assert_eq!(bool_c.value_hook, int_c.value_hook);
    assert_ne!(bool_c.leaf(), int_c.leaf());
    assert_ne!(
        member_rebinds_leaf(
            int_c.value_hook,
            fr(&boolean_datatype_const()),
            int_c.lexical_component,
        ),
        int_c.leaf(),
    );
}

/// A whole-graph `DualLeafV1` commitment puts `xsd:boolean` on the string lane,
/// whose leaf folds `VALUE_NONE` under the bare `blake3(xsd:boolean IRI)`. The
/// boolean member call must not be able to open that leaf with any hook: the
/// lane's `datatype_const` has to differ from the constant the string lane folds.
#[test]
fn a_string_lane_boolean_leaf_cannot_satisfy_a_boolean_member_call() {
    let lit = bool_lit(true);
    let string_lane_leaf = sparq_zk::dual_leaf::encode_literal_dual(&lit).unwrap();
    let lexical = encode_boolean(&lit).unwrap().lexical_component;
    assert_ne!(
        member_rebinds_leaf(
            Fr::from(sparq_zk::dual_leaf::VALUE_NONE),
            fr(&boolean_datatype_const()),
            lexical,
        ),
        string_lane_leaf,
    );
}

/// A literal of an opaque datatype also takes the string lane and folds
/// `blake3(datatype IRI)`. Datatype IRIs shaped like a lane tag must not reach the
/// boolean lane constant, so such a leaf cannot be opened as a boolean either.
#[test]
fn an_opaque_datatype_leaf_cannot_satisfy_a_boolean_member_call() {
    for iri in [
        format!("{XSD_BOOLEAN}@domain=bit"),
        format!("{XSD_BOOLEAN}%20domain=bit"),
        format!("{XSD_BOOLEAN}domain=bit"),
        // Not an IRI, but a datatype string can reach the fold unvalidated.
        format!("{XSD_BOOLEAN} domain=bit"),
    ] {
        let lit = Literal::new_typed_literal("1", NamedNode::new_unchecked(iri.as_str()));
        let string_lane_leaf = sparq_zk::dual_leaf::encode_literal_dual(&lit).unwrap();
        assert_ne!(fr(&boolean_datatype_const()), datatype_const(&iri), "{iri}");
        assert_ne!(
            member_rebinds_leaf(
                Fr::from(sparq_zk::dual_leaf::VALUE_NONE),
                fr(&boolean_datatype_const()),
                lexical_of(&lit),
            ),
            string_lane_leaf,
            "{iri}"
        );
    }
}

// =========================================================================
// 4. The verdict table (EQ/NE + the degenerate false < true orderings)
// =========================================================================

/// The full `(value, op, bound)` truth table written out LONGHAND — deliberately
/// not a re-derivation of `boolean_verdict`'s own formula, so a mutation to that
/// function goes RED here instead of being mirrored.
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

#[test]
fn boolean_verdict_matches_the_xsd_boolean_order() {
    for (value, op, bound, expected) in VERDICTS {
        assert_eq!(
            boolean_verdict(value, op, bound),
            expected,
            "{value:?} {op:?} {bound:?}"
        );
    }
}

/// The verdict the member is asked to prove is the one the integer relation
/// computes over the hooks `{0, 1}` — i.e. the boolean lane rides the integer
/// comparison unchanged. Transcribed from `filter_value.nr::integer_verdict`.
#[test]
fn boolean_verdict_is_the_integer_relation_over_the_hooks() {
    fn integer_verdict(value: u64, op: FilterOp, bound: u64) -> bool {
        let lt = value < bound;
        let eq = value == bound;
        match op {
            FilterOp::Lt => lt,
            FilterOp::Le => lt || eq,
            FilterOp::Gt => !lt && !eq,
            FilterOp::Ge => !lt,
            FilterOp::Eq => eq,
            FilterOp::Ne => !eq,
        }
    }
    for (value, op, bound, expected) in VERDICTS {
        assert_eq!(
            integer_verdict(u64::from(value), op, u64::from(bound)),
            expected
        );
    }
}

/// The builder discloses the HONEST verdict for every op × term × bound, and
/// carries the boolean `datatype_const` and the `{0,1}` bound hook.
#[test]
fn builder_discloses_the_honest_verdict_for_every_op() {
    for (value, op, bound, expected) in VERDICTS {
        let built = build_filter_value_dl_boolean(&bool_lit(value), op, bound).unwrap();
        let p = unpack(&built.inputs);
        let (got_op, got_bound, dt, got_expected) =
            (p.op, p.bound, p.datatype_const, p.expected);
        assert_eq!(got_op, op);
        assert_eq!(got_bound, u64::from(bound));
        assert_eq!(fr(dt), sparq_zk::dual_leaf_boolean::boolean_datatype_const());
        assert_eq!(got_expected, expected, "{value:?} {op:?} {bound:?}");
    }
}

// =========================================================================
// 5. Fail-closed inputs + fail-closed dispatch
// =========================================================================

/// The §6 co-binding is not bypassed by the compose-side builder: the
/// non-canonical XSD-legal spellings and every non-boolean datatype are refused
/// with the encoder's own error, never a silently desynced leaf.
#[test]
fn non_canonical_and_non_boolean_literals_are_fail_closed_at_the_builder() {
    for lex in ["1", "0", "True", " true"] {
        assert!(matches!(
            build_filter_value_dl_boolean(&typed(lex, XSD_BOOLEAN), FilterOp::Eq, true),
            Err(BuildValueDlV1Error::Encode(DualLeafError::NonCanonicalValue(_))),
        ));
    }
    assert!(matches!(
        build_filter_value_dl_boolean(&typed("1", XSD_INTEGER), FilterOp::Eq, true),
        Err(BuildValueDlV1Error::Encode(DualLeafError::NotValueLane(_))),
    ));
    assert!(matches!(
        build_filter_value_dl_boolean(&Literal::new_simple_literal("true"), FilterOp::Eq, true),
        Err(BuildValueDlV1Error::Encode(DualLeafError::NotValueLane(_))),
    ));
}

/// The boolean lane inherits the value lane's fail-closed `(method × circuit)`
/// refusal for V1, while a `string-canonical` graph has no value handle.
#[test]
fn boolean_lane_inherits_the_value_lane_dispatch_legality() {
    let built = build_filter_value_dl_boolean(&bool_lit(true), FilterOp::Eq, true).unwrap();
    let id = unpack(&built.inputs).id.clone();
    assert!(matches!(resolve_circuit(CommitmentMethod::DualLeafV1, &id),
        Err(DispatchError::LegacyV1ValueMember { .. })));
    assert_eq!(resolve_circuit(CommitmentMethod::DualLeafV2, &id), Ok(id.clone()));
    assert!(matches!(
        resolve_circuit(CommitmentMethod::StringCanonicalV1, &id),
        Err(DispatchError::IllegalPair { .. }),
    ));
}

// =========================================================================
// 6. Prover.toml — same member, so the SAME layout; only the constant differs
// =========================================================================

/// The boolean renderer is the integer renderer with the lane constant PINNED and
/// the bound mapped to its hook, so its bytes must equal the integer renderer's
/// when handed those values — the layout cannot drift from the shared `main`.
#[test]
fn boolean_prover_toml_pins_the_lane_constant_and_keeps_the_member_layout() {
    let built = build_filter_value_dl_boolean(&bool_lit(true), FilterOp::Ge, false).unwrap();
    let p = unpack(&built.inputs);
    let (operand_enc, op, bound, dt, expected) =
        (p.operand_enc, p.op, p.bound, p.datatype_const, p.expected);
    let challenge = FieldHex(field_to_hex(&Fr::from(1u64)));

    let boolean = filter_value_dl_boolean_prover_toml(
        &challenge,
        operand_enc,
        op.code(),
        false,
        expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the V1 boolean constant rides the shared member under the legacy use");
    let shared = filter_value_dl_prover_toml(
        &challenge,
        operand_enc,
        op.code(),
        bound,
        dt,
        expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the same values through the shared renderer");
    assert_eq!(boolean, shared);
    // And the LOCAL renderer the in-circuit vectors use emits those same bytes, so
    // routing them around the host boundary cannot silently change the witness.
    assert_eq!(
        boolean,
        raw_value_lane_toml(
            &challenge,
            operand_enc,
            op.code(),
            bound,
            dt,
            expected,
            &built.value_hook,
            &built.lexical_component,
        )
    );
    // The V1 constant is reachable only through the legacy names: the ORDINARY
    // renderer of the same member refuses it.
    assert!(matches!(
        sparq_zk_compose::toml::filter_value_dl_prover_toml(
            &challenge,
            operand_enc,
            op.code(),
            bound,
            dt,
            expected,
            &built.value_hook,
            &built.lexical_component,
        ),
        Err(DispatchError::LegacyLaneConstForIssuance { .. })
    ));

    let lines: Vec<&str> = boolean.lines().collect();
    assert!(lines[0].starts_with("challenge = "));
    assert!(lines[1].starts_with("operand_enc = "));
    assert_eq!(lines[2], "op = \"3\"");
    assert_eq!(lines[3], "bound = \"0\"");
    assert_eq!(
        lines[4],
        format!("datatype_const = \"{}\"", boolean_datatype_const().0)
    );
    assert_eq!(lines[5], "expected = true");
    assert!(lines[6].starts_with("value_hook = "));
    assert!(lines[7].starts_with("lexical_component = "));

    // The rendered constant is the BOOLEAN one — an integer-lane render differs.
    let integer_lane = filter_value_dl_prover_toml(
        &challenge,
        operand_enc,
        op.code(),
        bound,
        &FieldHex(field_to_hex(&datatype_const(XSD_INTEGER))),
        expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the V1 integer constant also rides the shared member, legacy use");
    assert_ne!(boolean, integer_lane);
}

/// `bound = true` renders the hook `1` (the other half of the `{0,1}` mapping).
#[test]
fn boolean_prover_toml_renders_the_true_bound_hook() {
    let built = build_filter_value_dl_boolean(&bool_lit(true), FilterOp::Eq, true).unwrap();
    let p = unpack(&built.inputs);
    let (operand_enc, op, expected) = (p.operand_enc, p.op, p.expected);
    let toml = filter_value_dl_boolean_prover_toml(
        &FieldHex(field_to_hex(&Fr::from(1u64))),
        operand_enc,
        op.code(),
        true,
        expected,
        &built.value_hook,
        &built.lexical_component,
    )
    .expect("the V1 boolean constant rides the shared member under the legacy use");
    assert!(toml.contains("bound = \"1\"\n"));
}

/// The manifest inputs serde round-trip on the SHARED `filter_value_dl` tag (the
/// boolean lane introduces no new wire tag — that is the point of the reuse).
#[test]
fn boolean_lane_inputs_round_trip_on_the_shared_wire_tag() {
    let built = build_filter_value_dl_boolean(&bool_lit(false), FilterOp::Ne, true).unwrap();
    let json = serde_json::to_string(&built.inputs).unwrap();
    assert!(json.contains("\"circuit\":\"filter_value_dl\""));
    assert!(json.contains(&boolean_datatype_const().0));
    let back: ProofInputs = serde_json::from_str(&json).unwrap();
    assert_eq!(back, built.inputs);
}

// =========================================================================
// 7. IN-CIRCUIT vectors — real witness solving. LIVE: they need `nargo`.
// =========================================================================

mod support;

/// The value-lane `Prover.toml` layout, rendered LOCALLY.
///
/// The library renderer now refuses a `(member, datatype_const)` pairing the lane
/// table does not allow. The IN-CIRCUIT vectors below must be able to hand the
/// member a pairing that boundary refuses — claim (c) below is exactly such a
/// vector — so they render the bytes here. The field order is the member `main`'s,
/// and `boolean_prover_toml_pins_the_lane_constant_and_keeps_the_member_layout`
/// pins the library renderer's own bytes for an ALLOWED pairing.
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

/// What nargo prints when a witness violates a constraint: an `assert` with a
/// message, one without, or an unsatisfiable range/bit constraint.
const CONSTRAINT_REFUSALS: [&str; 3] = [
    "Assertion failed",
    "Failed constraint",
    "Cannot satisfy constraint",
];

/// Solve `filter_value_dl_int`'s witness for the given public/private assignment.
/// `Ok` ⇔ the relation is SATISFIABLE ⇔ the member ACCEPTS.
/// A nargo constraint refusal is `false`; any other failure panics.
#[allow(clippy::too_many_arguments)]
fn member_accepts(
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
        &FieldHex(field_to_hex(&Fr::from(1u64))),
        operand_enc,
        op.code(),
        bound,
        datatype_const,
        expected,
        value_hook,
        lexical_component,
    );
    match sparq_zk_compose::driver::CircuitProver::from_crate_root().gen_witness_tagged(
        &CircuitId::FilterValueDl,
        &toml,
        tag,
    ) {
        Ok(_) => true,
        // A refusal counts only when nargo rejected the witness on a constraint; a
        // spawn, I/O or compile failure leaves no witness either, and must not be
        // read as the member refusing.
        Err(sparq_zk_compose::driver::DriverError::Tool { tool, stderr })
            if tool.starts_with("nargo")
                && CONSTRAINT_REFUSALS.iter().any(|m| stderr.contains(m)) =>
        {
            false
        }
        Err(other) => panic!("{tag}: infrastructure failure, not a constraint refusal: {other}"),
    }
}

/// The three in-circuit claims, in one toolchain-gated pass:
///   (a) the honest boolean witness IS provable on the shared member;
///   (b) a LYING disclosed verdict is NOT (the `filter verdict mismatch` assert);
///   (c) an `xsd:integer` leaf is NOT provable under the boolean `datatype_const`
///       (the `dual-leaf operand encoding mismatch` assert) — the separation.
#[test]
fn in_circuit_boolean_lane_accepts_honest_and_rejects_lies() {
    if !support::require_toolchain("in_circuit_boolean_lane_accepts_honest_and_rejects_lies", &["nargo"]) {
        return;
    }
    let built = build_filter_value_dl_boolean(&bool_lit(true), FilterOp::Gt, false).unwrap();
    let p = unpack(&built.inputs);
    let (operand_enc, op, bound, dt, expected) =
        (p.operand_enc, p.op, p.bound, p.datatype_const, p.expected);
    assert!(expected, "true > false is the honest verdict");

    // (a) honest.
    assert!(
        member_accepts(
            "sq5xdlk_ok", operand_enc, op, bound, dt, expected,
            &built.value_hook, &built.lexical_component,
        ),
        "the honest boolean witness must be provable on filter_value_dl_int"
    );

    // (b) lying verdict — the ONLY change is `expected`.
    assert!(
        !member_accepts(
            "sq5xdlk_lie", operand_enc, op, bound, dt, !expected,
            &built.value_hook, &built.lexical_component,
        ),
        "a flipped disclosed verdict must be unprovable"
    );

    // (c) an integer leaf on the boolean lane — the ONLY change is the datatype
    //     lane the same honest witnesses are presented against.
    let int_c: DualLeafComponents =
        sparq_zk::dual_leaf::encode_literal(&typed("1", XSD_INTEGER)).unwrap();
    let int_enc = FieldHex(field_to_hex(&int_c.leaf()));
    let int_hook = FieldHex(field_to_hex(&int_c.value_hook));
    let int_lex = FieldHex(field_to_hex(&int_c.lexical_component));
    assert!(
        member_accepts(
            "sq5xdlk_int_ok", &int_enc, op, bound,
            &FieldHex(field_to_hex(&datatype_const(XSD_INTEGER))), expected,
            &int_hook, &int_lex,
        ),
        "control: the integer leaf IS provable on its own lane"
    );
    assert!(
        !member_accepts(
            "sq5xdlk_int_on_bool", &int_enc, op, bound, dt, expected,
            &int_hook, &int_lex,
        ),
        "an xsd:integer leaf must be unprovable under the boolean datatype_const"
    );
    // ...and symmetrically, the boolean leaf on the integer lane.
    assert!(
        !member_accepts(
            "sq5xdlk_bool_on_int", operand_enc, op, bound,
            &FieldHex(field_to_hex(&datatype_const(XSD_INTEGER))), expected,
            &built.value_hook, &built.lexical_component,
        ),
        "an xsd:boolean leaf must be unprovable under the integer datatype_const"
    );
    support::complete_required();
}

/// In-circuit: the string-lane leaf a whole-graph commitment gives `"true"^^xsd:boolean`
/// is unprovable on the boolean lane with the `VALUE_NONE` hook, for a verdict that
/// would hold if that hook were compared as an integer.
#[test]
fn in_circuit_string_lane_boolean_leaf_is_rejected() {
    if !support::require_toolchain("in_circuit_string_lane_boolean_leaf_is_rejected", &["nargo"]) {
        return;
    }
    let lit = bool_lit(true);
    let leaf = FieldHex(field_to_hex(&sparq_zk::dual_leaf::encode_literal_dual(&lit).unwrap()));
    let lexical = FieldHex(field_to_hex(&encode_boolean(&lit).unwrap().lexical_component));
    let hook = FieldHex(field_to_hex(&Fr::from(sparq_zk::dual_leaf::VALUE_NONE)));
    // `VALUE_NONE > 1` holds over u64, so only the binding can reject this call.
    assert!(
        !member_accepts(
            "bool_string_lane", &leaf, FilterOp::Gt, 1,
            &boolean_datatype_const(), true, &hook, &lexical,
        ),
        "a string-lane boolean leaf must be unprovable on the boolean lane"
    );
    support::complete_required();
}

/// In-circuit: an opaque-datatype literal whose datatype IRI is shaped like a lane
/// tag is unprovable on the boolean lane with the `VALUE_NONE` hook.
#[test]
fn in_circuit_opaque_datatype_leaf_is_rejected() {
    if !support::require_toolchain("in_circuit_opaque_datatype_leaf_is_rejected", &["nargo"]) {
        return;
    }
    let iri = format!("{XSD_BOOLEAN}@domain=bit");
    let lit = Literal::new_typed_literal("1", NamedNode::new(iri.as_str()).unwrap());
    let leaf = FieldHex(field_to_hex(&sparq_zk::dual_leaf::encode_literal_dual(&lit).unwrap()));
    let lexical = FieldHex(field_to_hex(&lexical_of(&lit)));
    let hook = FieldHex(field_to_hex(&Fr::from(sparq_zk::dual_leaf::VALUE_NONE)));
    assert!(
        !member_accepts(
            "bool_opaque_datatype", &leaf, FilterOp::Gt, 1,
            &boolean_datatype_const(), true, &hook, &lexical,
        ),
        "an opaque-datatype leaf must be unprovable on the boolean lane"
    );
    support::complete_required();
}
