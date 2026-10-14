//! `DualLeafV1` value-lane builders, renderers and lane constants, by explicit
//! name (opt-in, `dual-leaf` feature).
//!
//! `DualLeafV1` is superseded by `DualLeafV2` for new issuance but stays
//! recognised and verifiable, so an ALREADY-ISSUED V1 graph still needs its
//! value-lane proofs built and rendered. Everything that produces a V1 lane
//! constant lives here, so a call site asks for V1 by naming this module. The
//! ordinary names in [`crate::build`], [`crate::toml`] and [`crate::manifest`]
//! build and render the V2 lanes only, and refuse a V1 constant.
//!
//! Each entry here admits V1 constants ONLY: a V2 constant is
//! [`DispatchError::NotALegacyV1LaneConst`]. Every `FieldHex` a renderer writes
//! must be canonical, as for the ordinary renderers. Using these names is the
//! caller's statement that the graph was issued under V1; nothing here reads
//! issuance history. The checked prove entry
//! (`crate::driver::CircuitProver::prove_value_lane`) accepts a V1 lane only with
//! `CommitmentMethod::DualLeafV1` and `MethodUse::LegacyAlreadyIssued`.
//!
//! DOCUMENTED RISK: the V1 lanes carry the INV-VL downgrade (#769 accepted,
//! CR-G8 / sq-qhy4) and V1 does not meet the
//! `research/zk-field-native-encoding.md` §14.1 reserved-tag requirement. NOT
//! externally audited; no soundness or privacy claim.

use crate::build::{
    assemble_dl_datetime, boolean_verdict, hexf, BuiltFilterValueDlDateTime,
};
use crate::dispatch::DispatchError;
use crate::manifest::{CircuitId, FieldHex, FilterOp, ProofInputs};
use crate::toml::{render_hook_checked, render_signed_checked, RenderVersion};
use sparq_zk::field::{field_to_hex, Fr};

/// The PUBLIC `datatype_const` of the `DualLeafV1` `xsd:boolean` value lane
/// (sq-5xdlk) — BLAKE3 key derivation over the `xsd:boolean` IRI, exactly the
/// constant `sparq_zk::dual_leaf_boolean::encode_boolean` folds into the
/// committed leaf's `value_component`.
///
/// The V1 boolean lane has NO member of its own: it rides
/// [`CircuitId::FilterValueDl`] (`filter_value_dl_int`), whose `u64` comparison
/// domain covers the hooks `{0 = false, 1 = true}`, and is separated from the
/// integer lane by this constant alone (it is folded into `value_component`).
/// That is a binding argument resting on Poseidon2 preimage resistance, not an
/// audited soundness claim. The V2 boolean lane has a dedicated bit-domain
/// member ([`crate::manifest::boolean_lane_const_v2`]).
pub fn boolean_datatype_const() -> FieldHex {
    FieldHex(field_to_hex(&sparq_zk::dual_leaf_boolean::boolean_datatype_const()))
}

/// The PUBLIC `datatype_const` of the `DualLeafV1` `xsd:dateTime` value lane
/// (sq-wz99x) — `blake3_field("<xsd:dateTime IRI>@epochscale=3")`, the constant
/// `sparq_zk::dual_leaf_datetime::encode_datetime` folds into the committed leaf.
/// It carries the lane-fixed sub-second scale `FS` (the B4 bind).
pub fn datetime_datatype_const() -> FieldHex {
    FieldHex(field_to_hex(&sparq_zk::dual_leaf_datetime::datetime_datatype_const()))
}

/// The PUBLIC `datatype_const` of the `DualLeafV1` `xsd:date` value lane
/// (sq-wz99x) — `blake3_field("<xsd:date IRI>@epochscale=3")`. It shares
/// [`CircuitId::FilterValueDlDateTime`] with the dateTime lane: a date's hook is
/// numerically equal to the dateTime hook of its starting instant, so this
/// constant is the only thing keeping the two terms apart.
pub fn date_datatype_const() -> FieldHex {
    FieldHex(field_to_hex(&sparq_zk::dual_leaf_datetime::date_datatype_const()))
}

/// Why a `DualLeafV1` value-lane builder refused.
#[derive(Debug, PartialEq, Eq)]
pub enum BuildValueDlV1Error {
    /// The host encoder refused the literal (the §6 fail-closed co-binding).
    Encode(sparq_zk::dual_leaf::DualLeafError),
    /// The assembled public inputs did not pass the LANE↔MEMBER pairing check —
    /// the builder re-checks its own output rather than trusting its assembly.
    Pairing(DispatchError),
}

impl From<sparq_zk::dual_leaf::DualLeafError> for BuildValueDlV1Error {
    fn from(e: sparq_zk::dual_leaf::DualLeafError) -> Self {
        BuildValueDlV1Error::Encode(e)
    }
}

impl std::fmt::Display for BuildValueDlV1Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildValueDlV1Error::Encode(e) => write!(f, "{}", e),
            BuildValueDlV1Error::Pairing(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for BuildValueDlV1Error {}

/// The V1 builders' self-check: the assembled inputs pass the pairing check for
/// an already-issued graph AND carry a V1 constant.
fn v1_self_check(inputs: &ProofInputs) -> Result<(), BuildValueDlV1Error> {
    crate::dispatch::check_lane_member_pairing_for_use(
        inputs,
        sparq_zk::commit::MethodUse::LegacyAlreadyIssued,
    )
    .map_err(BuildValueDlV1Error::Pairing)?;
    let (id, datatype_const) = crate::dispatch::value_lane_fields(inputs)
        .expect("a V1 builder assembles a value-lane variant");
    crate::dispatch::check_legacy_v1_lane_const(id, datatype_const)
        .map_err(BuildValueDlV1Error::Pairing)
}

/// A built `DualLeafV1` `xsd:boolean` value-lane FILTER (sq-5xdlk): the public
/// [`ProofInputs`] for the shared integer member plus its two PRIVATE field
/// witnesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltFilterValueDlBoolean {
    /// Public inputs for the shared [`CircuitId::FilterValueDl`] member, carrying
    /// the V1 boolean `datatype_const` and `bound ∈ {0, 1}`.
    pub inputs: ProofInputs,
    /// PRIVATE: the boolean `VALUE_HOOK` (`0` = false, `1` = true) as a field.
    pub value_hook: FieldHex,
    /// PRIVATE: the OFF-circuit blake3 lexical hash, carried as a free witness.
    pub lexical_component: FieldHex,
}

impl BuiltFilterValueDlBoolean {
    /// The typed inputs of the checked prove entry: the INTEGER lane carrying the
    /// V1 boolean constant. Proving them takes `CommitmentMethod::DualLeafV1`
    /// with `MethodUse::LegacyAlreadyIssued`.
    pub fn lane_proof(&self) -> Result<crate::value_lane::ValueLaneProof, DispatchError> {
        crate::value_lane::ValueLaneProof::from_inputs(
            &self.inputs,
            crate::value_lane::LaneWitness::Hook(crate::value_lane::HookWitness {
                value_hook: self.value_hook.clone(),
                lexical_component: self.lexical_component.clone(),
            }),
        )
    }
}

/// Build a `DualLeafV1` `xsd:boolean` value-lane FILTER over the committed
/// literal `literal` under operator `op` against the constant `bound` — for an
/// ALREADY-ISSUED V1 graph. It targets the shared [`CircuitId::FilterValueDl`]
/// with [`boolean_datatype_const`]. A new boolean-lane proof uses
/// [`crate::build::build_filter_value_dl_bool`] (the V2 bit-domain member).
///
/// The disclosed verdict is COMPUTED ([`boolean_verdict`]). Returns the encoder's
/// error unchanged when `literal` is not a canonical `xsd:boolean`
/// (`"true"`/`"false"`).
pub fn build_filter_value_dl_boolean(
    literal: &oxrdf::Literal,
    op: FilterOp,
    bound: bool,
) -> Result<BuiltFilterValueDlBoolean, BuildValueDlV1Error> {
    let components = sparq_zk::dual_leaf_boolean::encode_boolean(literal)?;
    // The encoder maps ONLY the canonical lexicals, so the hook is exactly {0, 1}.
    let value = components.value_hook != Fr::from(0u64);
    let inputs = ProofInputs::FilterValueDl {
        id: CircuitId::FilterValueDl,
        operand_enc: hexf(&components.leaf()),
        op,
        bound: u64::from(bound),
        datatype_const: boolean_datatype_const(),
        expected: boolean_verdict(value, op, bound),
    };
    v1_self_check(&inputs)?;
    Ok(BuiltFilterValueDlBoolean {
        inputs,
        value_hook: hexf(&components.value_hook),
        lexical_component: hexf(&components.lexical_component),
    })
}

/// Build a `DualLeafV1` `xsd:dateTime` value-lane FILTER — for an ALREADY-ISSUED
/// V1 graph. Both operands go through `sparq_zk::dual_leaf_datetime::
/// encode_datetime`; see [`crate::build::build_filter_value_dl_datetime`] (the V2
/// form) for the shared semantics.
pub fn build_filter_value_dl_datetime(
    literal: &oxrdf::Literal,
    op: FilterOp,
    bound: &oxrdf::Literal,
) -> Result<BuiltFilterValueDlDateTime, BuildValueDlV1Error> {
    let value = sparq_zk::dual_leaf_datetime::encode_datetime(literal)?;
    let bound_components = sparq_zk::dual_leaf_datetime::encode_datetime(bound)?;
    let built = assemble_dl_datetime(literal, bound, op, value, bound_components)?;
    v1_self_check(&built.inputs)?;
    Ok(built)
}

/// Build a `DualLeafV1` `xsd:date` value-lane FILTER — for an ALREADY-ISSUED V1
/// graph. Both operands go through `sparq_zk::dual_leaf_datetime::encode_date`;
/// see [`crate::build::build_filter_value_dl_date`] (the V2 form).
pub fn build_filter_value_dl_date(
    literal: &oxrdf::Literal,
    op: FilterOp,
    bound: &oxrdf::Literal,
) -> Result<BuiltFilterValueDlDateTime, BuildValueDlV1Error> {
    let value = sparq_zk::dual_leaf_datetime::encode_date(literal)?;
    let bound_components = sparq_zk::dual_leaf_datetime::encode_date(bound)?;
    let built = assemble_dl_datetime(literal, bound, op, value, bound_components)?;
    v1_self_check(&built.inputs)?;
    Ok(built)
}

/// Render the `Prover.toml` body for a `DualLeafV1` value-lane FILTER on the
/// integer member — the V1 integer lane or the V1 boolean lane (both ride
/// `filter_value_dl_int`). Layout as [`crate::toml::filter_value_dl_prover_toml`];
/// admits V1 constants only, and every `FieldHex` must be canonical.
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> Result<String, DispatchError> {
    render_hook_checked(
        CircuitId::FilterValueDl,
        "bound",
        RenderVersion::LegacyV1,
        challenge,
        operand_enc,
        op,
        bound,
        datatype_const,
        expected,
        value_hook,
        lexical_component,
    )
}

/// Render the `Prover.toml` body for a `DualLeafV1` `xsd:boolean` value-lane
/// FILTER: [`filter_value_dl_prover_toml`] with `datatype_const` PINNED to
/// [`boolean_datatype_const`] and `bound` taken as a `bool` mapped to `{0, 1}`.
/// The emitted bytes are the integer lane's layout — it is the same member.
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_boolean_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound: bool,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> Result<String, DispatchError> {
    filter_value_dl_prover_toml(
        challenge,
        operand_enc,
        op,
        u64::from(bound),
        &boolean_datatype_const(),
        expected,
        value_hook,
        lexical_component,
    )
}

/// Render the `Prover.toml` body for a `DualLeafV1` `xsd:double` value-lane
/// FILTER. Layout as [`crate::toml::filter_value_dl_f64_prover_toml`]; V1
/// constants only.
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_f64_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    b_bits: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> Result<String, DispatchError> {
    render_hook_checked(
        CircuitId::FilterValueDlF64,
        "b_bits",
        RenderVersion::LegacyV1,
        challenge,
        operand_enc,
        op,
        b_bits,
        datatype_const,
        expected,
        value_hook,
        lexical_component,
    )
}

/// Render the `Prover.toml` body for a `DualLeafV1` `xsd:decimal` value-lane
/// FILTER. Layout as [`crate::toml::filter_value_dl_decimal_prover_toml`]; V1
/// constants only.
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_decimal_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound_scaled: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: &FieldHex,
    lexical_component: &FieldHex,
) -> Result<String, DispatchError> {
    render_signed_checked(
        CircuitId::FilterValueDlDecimal,
        "bound_scaled",
        RenderVersion::LegacyV1,
        challenge,
        operand_enc,
        op,
        bound_neg,
        bound_scaled,
        datatype_const,
        expected,
        value_neg,
        value_hook_scaled,
        lexical_component,
    )
}

/// Render the `Prover.toml` body for a `DualLeafV1` `xsd:dateTime` / `xsd:date`
/// value-lane FILTER. Layout as
/// [`crate::toml::filter_value_dl_datetime_prover_toml`]; V1 constants only
/// ([`datetime_datatype_const`] or [`date_datatype_const`]).
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_datetime_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound_scaled_epoch: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: &FieldHex,
    lexical_component: &FieldHex,
) -> Result<String, DispatchError> {
    render_signed_checked(
        CircuitId::FilterValueDlDateTime,
        "bound_scaled_epoch",
        RenderVersion::LegacyV1,
        challenge,
        operand_enc,
        op,
        bound_neg,
        bound_scaled_epoch,
        datatype_const,
        expected,
        value_neg,
        value_hook_scaled,
        lexical_component,
    )
}
