//! Typed, checked value-lane proving inputs (opt-in, `dual-leaf` feature).
//!
//! A value-lane proof is described here by WHICH lane it is on — a
//! [`ValueLaneProof`] variant — and the lane's public inputs and private witness
//! as typed fields. Nothing in it names a circuit: the member is DERIVED from the
//! lane by [`ValueLane::member`], the one variant→member table this crate has.
//! The verifier's sub-proof id derivation uses the same table through
//! [`derive_value_lane_member`], and so does the host-side variant↔id check in
//! `crate::dispatch::check_lane_member_pairing_for_use`, so a variant and a
//! member cannot be paired differently in two places.
//!
//! [`ValueLaneProof::checked_prover_toml`] is the validation-then-render step the
//! checked prove entry `crate::driver::CircuitProver::prove_value_lane` runs
//! before any subprocess: it resolves the `(method, member, use)` dispatch,
//! refuses any non-canonical `FieldHex` it would write (the challenge and the
//! private witnesses included), applies the lane↔member allow-list, requires the
//! method version and the lane constant version to agree, and only then renders
//! the `Prover.toml` from those validated fields. No caller-supplied TOML enters
//! that path.
//!
//! Honest scope: this is a host-side boundary on what this crate builds, renders
//! and proves. It asserts no soundness or privacy property; the value lanes carry
//! the dual-leaf INV-VL downgrade (#769 accepted, CR-G8 / sq-qhy4) and the estate
//! is NOT externally audited.

use crate::dispatch::{self, DispatchError};
use crate::manifest::{CircuitId, FieldHex, FilterOp, ProofInputs};
use sparq_zk::commit::{CommitmentMethod, MethodUse};

/// A value lane, independent of any member id — the key of the variant→member
/// table [`ValueLane::member`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueLane {
    /// `xsd:integer` (and the V1 `xsd:boolean` lane, which shares its member).
    Integer,
    /// `xsd:double`.
    Double,
    /// `xsd:decimal` at a fixed scale.
    Decimal,
    /// `xsd:dateTime` and `xsd:date` (one member, separated by the constant).
    DateTime,
    /// The V2 bit-domain `xsd:boolean` lane.
    Boolean,
}

impl ValueLane {
    /// Every value lane.
    pub const ALL: [ValueLane; 5] = [
        ValueLane::Integer,
        ValueLane::Double,
        ValueLane::Decimal,
        ValueLane::DateTime,
        ValueLane::Boolean,
    ];

    /// The ONE member this lane's proofs run on. This is the single definition
    /// of the variant→member relation: the checked prove entry, the host
    /// variant↔id check and the verifier's id derivation all read it.
    pub fn member(self) -> CircuitId {
        match self {
            ValueLane::Integer => CircuitId::FilterValueDl,
            ValueLane::Double => CircuitId::FilterValueDlF64,
            ValueLane::Decimal => CircuitId::FilterValueDlDecimal,
            ValueLane::DateTime => CircuitId::FilterValueDlDateTime,
            ValueLane::Boolean => CircuitId::FilterValueDlBool,
        }
    }

    /// The lane a value-lane `ProofInputs` VARIANT is on, or `None` for every
    /// other variant. Reads the variant only, never the declared `id`.
    pub fn of_inputs(inputs: &ProofInputs) -> Option<ValueLane> {
        match inputs {
            ProofInputs::FilterValueDl { .. } => Some(ValueLane::Integer),
            ProofInputs::FilterValueDlF64 { .. } => Some(ValueLane::Double),
            ProofInputs::FilterValueDlDecimal { .. } => Some(ValueLane::Decimal),
            ProofInputs::FilterValueDlDateTime { .. } => Some(ValueLane::DateTime),
            ProofInputs::FilterValueDlBool { .. } => Some(ValueLane::Boolean),
            _ => None,
        }
    }

    /// The `ProofInputs` variant name of this lane, for diagnostics.
    pub fn variant_name(self) -> &'static str {
        match self {
            ValueLane::Integer => "FilterValueDl",
            ValueLane::Double => "FilterValueDlF64",
            ValueLane::Decimal => "FilterValueDlDecimal",
            ValueLane::DateTime => "FilterValueDlDateTime",
            ValueLane::Boolean => "FilterValueDlBool",
        }
    }
}

/// The member a value-lane `ProofInputs` runs on, derived from its VARIANT, when
/// its declared `id` is that member; `None` for a non-value-lane variant or a
/// variant/id disagreement. The verifier's sub-proof id derivation calls this for
/// every value-lane variant.
pub fn derive_value_lane_member(inputs: &ProofInputs) -> Option<CircuitId> {
    let member = ValueLane::of_inputs(inputs)?.member();
    (inputs.circuit_id() == &member).then_some(member)
}

/// The private witness of the integer, double and boolean members: the value
/// handle and the off-circuit lexical hash, both field elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookWitness {
    /// PRIVATE: the `VALUE_HOOK` as a field.
    pub value_hook: FieldHex,
    /// PRIVATE: the off-circuit blake3 lexical hash.
    pub lexical_component: FieldHex,
}

/// The private witness of the sign-folded decimal and dateTime members: the
/// handle's sign and magnitude, and the off-circuit lexical hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedWitness {
    /// PRIVATE: the handle's sign (`true` = negative / pre-epoch).
    pub value_neg: bool,
    /// PRIVATE: the handle's magnitude as a field.
    pub value_hook_scaled: FieldHex,
    /// PRIVATE: the off-circuit blake3 lexical hash.
    pub lexical_component: FieldHex,
}

/// Either witness shape, for [`ValueLaneProof::from_inputs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneWitness {
    /// The integer / double / boolean shape.
    Hook(HookWitness),
    /// The decimal / dateTime shape.
    Signed(SignedWitness),
}

/// The integer lane (`filter_value_dl_int`). The V1 `xsd:boolean` lane rides this
/// member too, with its own constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegerLane {
    pub operand_enc: FieldHex,
    pub op: FilterOp,
    pub bound: u64,
    pub datatype_const: FieldHex,
    pub expected: bool,
    pub witness: HookWitness,
}

/// The double lane (`filter_value_dl_f64`); `b_bits` is the bound's IEEE bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoubleLane {
    pub operand_enc: FieldHex,
    pub op: FilterOp,
    pub b_bits: u64,
    pub datatype_const: FieldHex,
    pub expected: bool,
    pub witness: HookWitness,
}

/// The decimal lane (`filter_value_dl_decimal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalLane {
    pub operand_enc: FieldHex,
    pub op: FilterOp,
    pub bound_neg: bool,
    pub bound_scaled: u64,
    pub datatype_const: FieldHex,
    pub expected: bool,
    pub witness: SignedWitness,
}

/// The dateTime / date lane (`filter_value_dl_datetime`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeLane {
    pub operand_enc: FieldHex,
    pub op: FilterOp,
    pub bound_neg: bool,
    pub bound_scaled_epoch: u64,
    pub datatype_const: FieldHex,
    pub expected: bool,
    pub witness: SignedWitness,
}

/// The V2 bit-domain boolean lane (`filter_value_dl_bool`). It carries no
/// `datatype_const` field: the member accepts exactly one constant
/// ([`crate::manifest::boolean_lane_const_v2`]), so it is pinned, and the bound
/// is a `bool`, so a bound outside `{0, 1}` cannot be expressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BooleanLane {
    pub operand_enc: FieldHex,
    pub op: FilterOp,
    pub bound: bool,
    pub expected: bool,
    pub witness: HookWitness,
}

/// A value-lane proof request: which lane, and that lane's typed inputs. The
/// member is derived from the variant ([`ValueLaneProof::circuit_id`]), so a
/// lane and a mismatched member cannot be expressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueLaneProof {
    Integer(IntegerLane),
    Double(DoubleLane),
    Decimal(DecimalLane),
    DateTime(DateTimeLane),
    Boolean(BooleanLane),
}

impl ValueLaneProof {
    /// The lane this proof is on.
    pub fn lane(&self) -> ValueLane {
        match self {
            ValueLaneProof::Integer(_) => ValueLane::Integer,
            ValueLaneProof::Double(_) => ValueLane::Double,
            ValueLaneProof::Decimal(_) => ValueLane::Decimal,
            ValueLaneProof::DateTime(_) => ValueLane::DateTime,
            ValueLaneProof::Boolean(_) => ValueLane::Boolean,
        }
    }

    /// The member this proof runs on — [`ValueLane::member`] of [`Self::lane`].
    pub fn circuit_id(&self) -> CircuitId {
        self.lane().member()
    }

    /// The lane constant this proof carries (the pinned V2 constant for the
    /// boolean lane).
    pub fn datatype_const(&self) -> FieldHex {
        match self {
            ValueLaneProof::Integer(l) => l.datatype_const.clone(),
            ValueLaneProof::Double(l) => l.datatype_const.clone(),
            ValueLaneProof::Decimal(l) => l.datatype_const.clone(),
            ValueLaneProof::DateTime(l) => l.datatype_const.clone(),
            ValueLaneProof::Boolean(_) => crate::manifest::boolean_lane_const_v2(),
        }
    }

    /// The public inputs as the manifest carries them, with the DERIVED member as
    /// `id`.
    pub fn proof_inputs(&self) -> ProofInputs {
        let id = self.circuit_id();
        match self {
            ValueLaneProof::Integer(l) => ProofInputs::FilterValueDl {
                id,
                operand_enc: l.operand_enc.clone(),
                op: l.op,
                bound: l.bound,
                datatype_const: l.datatype_const.clone(),
                expected: l.expected,
            },
            ValueLaneProof::Double(l) => ProofInputs::FilterValueDlF64 {
                id,
                operand_enc: l.operand_enc.clone(),
                op: l.op,
                b_bits: l.b_bits,
                datatype_const: l.datatype_const.clone(),
                expected: l.expected,
            },
            ValueLaneProof::Decimal(l) => ProofInputs::FilterValueDlDecimal {
                id,
                operand_enc: l.operand_enc.clone(),
                op: l.op,
                bound_neg: l.bound_neg,
                bound_scaled: l.bound_scaled,
                datatype_const: l.datatype_const.clone(),
                expected: l.expected,
            },
            ValueLaneProof::DateTime(l) => ProofInputs::FilterValueDlDateTime {
                id,
                operand_enc: l.operand_enc.clone(),
                op: l.op,
                bound_neg: l.bound_neg,
                bound_scaled_epoch: l.bound_scaled_epoch,
                datatype_const: l.datatype_const.clone(),
                expected: l.expected,
            },
            ValueLaneProof::Boolean(l) => ProofInputs::FilterValueDlBool {
                id,
                operand_enc: l.operand_enc.clone(),
                op: l.op,
                bound: u64::from(l.bound),
                datatype_const: crate::manifest::boolean_lane_const_v2(),
                expected: l.expected,
            },
        }
    }

    /// Assemble typed inputs from a built `ProofInputs` and its witness.
    ///
    /// Refuses ([`DispatchError`]) rather than repairs: a non-value-lane variant
    /// or a variant whose declared `id` is not its lane's member
    /// ([`DispatchError::ValueLaneVariantIdMismatch`], decided by
    /// [`derive_value_lane_member`]); a witness of the other shape, or a
    /// bit-domain boolean bound outside `{0, 1}`
    /// ([`DispatchError::MalformedValueLaneInputs`]); and, for the boolean lane,
    /// any constant other than the pinned V2 one (the lane allow-list's own
    /// refusal).
    pub fn from_inputs(inputs: &ProofInputs, witness: LaneWitness) -> Result<Self, DispatchError> {
        let declared = inputs.circuit_id();
        if derive_value_lane_member(inputs).is_none() {
            let variant = ValueLane::of_inputs(inputs)
                .map(ValueLane::variant_name)
                .unwrap_or_else(|| dispatch::non_value_lane_variant_name(inputs));
            return Err(DispatchError::ValueLaneVariantIdMismatch {
                circuit: declared.label(),
                variant: variant.to_string(),
            });
        }
        let malformed = |reason: &str| DispatchError::MalformedValueLaneInputs {
            circuit: declared.label(),
            reason: reason.to_string(),
        };
        let wrong_shape = "the witness shape is not this lane's";
        Ok(match (inputs, witness) {
            (
                ProofInputs::FilterValueDl {
                    operand_enc,
                    op,
                    bound,
                    datatype_const,
                    expected,
                    ..
                },
                LaneWitness::Hook(witness),
            ) => ValueLaneProof::Integer(IntegerLane {
                operand_enc: operand_enc.clone(),
                op: *op,
                bound: *bound,
                datatype_const: datatype_const.clone(),
                expected: *expected,
                witness,
            }),
            (
                ProofInputs::FilterValueDlF64 {
                    operand_enc,
                    op,
                    b_bits,
                    datatype_const,
                    expected,
                    ..
                },
                LaneWitness::Hook(witness),
            ) => ValueLaneProof::Double(DoubleLane {
                operand_enc: operand_enc.clone(),
                op: *op,
                b_bits: *b_bits,
                datatype_const: datatype_const.clone(),
                expected: *expected,
                witness,
            }),
            (
                ProofInputs::FilterValueDlDecimal {
                    operand_enc,
                    op,
                    bound_neg,
                    bound_scaled,
                    datatype_const,
                    expected,
                    ..
                },
                LaneWitness::Signed(witness),
            ) => ValueLaneProof::Decimal(DecimalLane {
                operand_enc: operand_enc.clone(),
                op: *op,
                bound_neg: *bound_neg,
                bound_scaled: *bound_scaled,
                datatype_const: datatype_const.clone(),
                expected: *expected,
                witness,
            }),
            (
                ProofInputs::FilterValueDlDateTime {
                    operand_enc,
                    op,
                    bound_neg,
                    bound_scaled_epoch,
                    datatype_const,
                    expected,
                    ..
                },
                LaneWitness::Signed(witness),
            ) => ValueLaneProof::DateTime(DateTimeLane {
                operand_enc: operand_enc.clone(),
                op: *op,
                bound_neg: *bound_neg,
                bound_scaled_epoch: *bound_scaled_epoch,
                datatype_const: datatype_const.clone(),
                expected: *expected,
                witness,
            }),
            (
                ProofInputs::FilterValueDlBool {
                    id,
                    operand_enc,
                    op,
                    bound,
                    datatype_const,
                    expected,
                },
                LaneWitness::Hook(witness),
            ) => {
                let bound = match bound {
                    0 => false,
                    1 => true,
                    _ => return Err(malformed("the boolean bound is outside {0, 1}")),
                };
                dispatch::check_lane_const_for_member(id, datatype_const, MethodUse::NewIssuance)?;
                ValueLaneProof::Boolean(BooleanLane {
                    operand_enc: operand_enc.clone(),
                    op: *op,
                    bound,
                    expected: *expected,
                    witness,
                })
            }
            _ => return Err(malformed(wrong_shape)),
        })
    }

    /// Every `FieldHex` this proof writes into its `Prover.toml`, named.
    fn field_hexes<'a>(&'a self, challenge: &'a FieldHex) -> Vec<(&'static str, &'a FieldHex)> {
        let mut v = vec![("challenge", challenge)];
        match self {
            ValueLaneProof::Integer(IntegerLane {
                operand_enc,
                datatype_const,
                witness,
                ..
            })
            | ValueLaneProof::Double(DoubleLane {
                operand_enc,
                datatype_const,
                witness,
                ..
            }) => {
                v.push(("operand_enc", operand_enc));
                v.push(("datatype_const", datatype_const));
                v.push(("value_hook", &witness.value_hook));
                v.push(("lexical_component", &witness.lexical_component));
            }
            ValueLaneProof::Decimal(DecimalLane {
                operand_enc,
                datatype_const,
                witness,
                ..
            })
            | ValueLaneProof::DateTime(DateTimeLane {
                operand_enc,
                datatype_const,
                witness,
                ..
            }) => {
                v.push(("operand_enc", operand_enc));
                v.push(("datatype_const", datatype_const));
                v.push(("value_hook_scaled", &witness.value_hook_scaled));
                v.push(("lexical_component", &witness.lexical_component));
            }
            ValueLaneProof::Boolean(BooleanLane {
                operand_enc,
                witness,
                ..
            }) => {
                v.push(("operand_enc", operand_enc));
                v.push(("value_hook", &witness.value_hook));
                v.push(("lexical_component", &witness.lexical_component));
            }
        }
        v
    }

    /// Validate this proof for `(method, use_)` and render its `Prover.toml`.
    ///
    /// In order, refusing with the first [`DispatchError`] that applies:
    ///
    /// 1. the `(method, derived member, use_)` dispatch
    ///    (`crate::dispatch::resolve_circuit_for_use`);
    /// 2. every `FieldHex` to be written — `challenge`, `operand_enc`,
    ///    `datatype_const` and the private witness fields — must be the canonical
    ///    `0x` + 64-lowercase-nibble rendering of its element
    ///    ([`DispatchError::NonCanonicalFieldHex`], the same decoder the dispatch
    ///    checks use);
    /// 3. the lane↔member allow-list under `use_`
    ///    (`crate::dispatch::check_lane_member_pairing_for_use`);
    /// 4. a dual-leaf `method` and the lane constant must be the same version
    ///    ([`DispatchError::MethodLaneVersionMismatch`]).
    ///
    /// Only then is the text rendered, from these validated fields, in the member
    /// `main`'s declaration order. Because every interpolated string has passed
    /// step 2, the rendered file cannot carry a key or a table its fields did not
    /// produce.
    pub fn checked_prover_toml(
        &self,
        method: CommitmentMethod,
        use_: MethodUse,
        challenge: &FieldHex,
    ) -> Result<String, DispatchError> {
        let member = dispatch::resolve_circuit_for_use(method, &self.circuit_id(), use_)?;
        dispatch::require_canonical(&member, &self.field_hexes(challenge))?;
        dispatch::check_lane_member_pairing_for_use(&self.proof_inputs(), use_)?;
        dispatch::check_method_lane_version(method, &member, &self.datatype_const())?;
        Ok(self.render(challenge))
    }

    /// The `Prover.toml` text, through the same layout functions the public
    /// renderers use. Callers validate first; see [`Self::checked_prover_toml`].
    fn render(&self, challenge: &FieldHex) -> String {
        use crate::toml::{render_value_dl_hook, render_value_dl_signed};
        match self {
            ValueLaneProof::Integer(l) => render_value_dl_hook(
                challenge,
                &l.operand_enc,
                l.op.code(),
                "bound",
                l.bound,
                &l.datatype_const,
                l.expected,
                &l.witness.value_hook,
                &l.witness.lexical_component,
            ),
            ValueLaneProof::Double(l) => render_value_dl_hook(
                challenge,
                &l.operand_enc,
                l.op.code(),
                "b_bits",
                l.b_bits,
                &l.datatype_const,
                l.expected,
                &l.witness.value_hook,
                &l.witness.lexical_component,
            ),
            ValueLaneProof::Boolean(l) => render_value_dl_hook(
                challenge,
                &l.operand_enc,
                l.op.code(),
                "bound",
                u64::from(l.bound),
                &crate::manifest::boolean_lane_const_v2(),
                l.expected,
                &l.witness.value_hook,
                &l.witness.lexical_component,
            ),
            ValueLaneProof::Decimal(l) => render_value_dl_signed(
                challenge,
                &l.operand_enc,
                l.op.code(),
                l.bound_neg,
                "bound_scaled",
                l.bound_scaled,
                &l.datatype_const,
                l.expected,
                &l.witness,
            ),
            ValueLaneProof::DateTime(l) => render_value_dl_signed(
                challenge,
                &l.operand_enc,
                l.op.code(),
                l.bound_neg,
                "bound_scaled_epoch",
                l.bound_scaled_epoch,
                &l.datatype_const,
                l.expected,
                &l.witness,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is a bijection onto the five value-lane members, and
    /// `of_inputs` inverts `proof_inputs` for every lane — so the derived member
    /// and the variant a typed proof emits cannot drift apart.
    #[test]
    fn the_variant_member_table_is_one_to_one_and_round_trips() {
        let members: std::collections::HashSet<String> = ValueLane::ALL
            .iter()
            .map(|l| l.member().label())
            .collect();
        assert_eq!(members.len(), ValueLane::ALL.len());
        let h = || FieldHex::from_field(&sparq_zk::field::Fr::from(1u64));
        let hook = || HookWitness {
            value_hook: h(),
            lexical_component: h(),
        };
        let signed = || SignedWitness {
            value_neg: false,
            value_hook_scaled: h(),
            lexical_component: h(),
        };
        let proofs = [
            ValueLaneProof::Integer(IntegerLane {
                operand_enc: h(),
                op: FilterOp::Eq,
                bound: 1,
                datatype_const: h(),
                expected: true,
                witness: hook(),
            }),
            ValueLaneProof::Double(DoubleLane {
                operand_enc: h(),
                op: FilterOp::Eq,
                b_bits: 1,
                datatype_const: h(),
                expected: true,
                witness: hook(),
            }),
            ValueLaneProof::Decimal(DecimalLane {
                operand_enc: h(),
                op: FilterOp::Eq,
                bound_neg: false,
                bound_scaled: 1,
                datatype_const: h(),
                expected: true,
                witness: signed(),
            }),
            ValueLaneProof::DateTime(DateTimeLane {
                operand_enc: h(),
                op: FilterOp::Eq,
                bound_neg: false,
                bound_scaled_epoch: 1,
                datatype_const: h(),
                expected: true,
                witness: signed(),
            }),
            ValueLaneProof::Boolean(BooleanLane {
                operand_enc: h(),
                op: FilterOp::Eq,
                bound: true,
                expected: true,
                witness: hook(),
            }),
        ];
        for p in proofs {
            let inputs = p.proof_inputs();
            assert_eq!(ValueLane::of_inputs(&inputs), Some(p.lane()));
            assert_eq!(derive_value_lane_member(&inputs), Some(p.circuit_id()));
            assert_eq!(inputs.circuit_id(), &p.circuit_id());
        }
    }
}
