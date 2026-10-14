//! Relying-party PIN functions for the dual-leaf value lanes (opt-in,
//! `dual-leaf` feature).
//!
//! # ⚠️ THE VALUE-LANE VERIFIER GATES ARE CLOSED
//! Nothing in this module opens one. `verifier::prefilter_manifest_structure`
//! still refuses every value-lane sub-proof outright at stage 0
//! (`CheckError::ValueLaneSubProofNotAccepted`), the stage-2 binding-edge
//! operand match still admits only the `filter_int` family, and
//! `filter_edge_true` is untouched. A pin computed here decides nothing, matches
//! no obligation edge and discharges no query FILTER. What this module is, is
//! the INPUT those gates will need: the question "given an authenticated
//! commitment method and the typed FILTER the query spells, what exactly would a
//! value-lane sub-proof's public inputs have to be?", answered in one place so
//! the answer is written down once.
//!
//! # What a pin actually fixes
//! A value-lane member's `datatype_const` is a public input no member CONSTRAINS
//! — the leaf binding is satisfied whatever constant is presented, so the lane,
//! and for the decimal lane the SCALE, are the relying party's to pin. The
//! `bound` (or `b_bits`, or the sign-split magnitude) is likewise a public input
//! whose relation to the query's constant nothing in-circuit states. So a pin is
//! five equalities, every one of them against a value this module DERIVES from
//! the query text and the authenticated method:
//!
//! 1. the member is the one the constant's datatype routes to;
//! 2. the presented `datatype_const` is the canonical spelling of that lane's
//!    constant (for the decimal lane, of `decimal_lane_const(fd)` at an admitted
//!    scale);
//! 3. the presented `op` code equals the query comparison's, after the
//!    `const op ?var` flip the extraction already applied;
//! 4. the presented bound equals the query constant under the lane's own
//!    encoder — exactly, with no rounding and no numeric promotion;
//! 5. the presented verdict is `true`.
//!
//! Each of the five fails CLOSED with its own [`PinError`], so a refusal names
//! which equality did not hold rather than collapsing to one opaque rejection.
//!
//! # Only an authenticated `DualLeafV2` proceeds
//! A value lane exists only under the dual-leaf leaf shape, and only V2's lane
//! constants are the framed, separately-derived ones. So the method axis is
//! decided first and narrowly: an UNBOUND attestation gives
//! [`PinError::MethodNotBound`] (no method was attested, and a pin cannot be
//! computed against an assumed one), `StringCanonicalV1` gives
//! [`PinError::MethodHasNoValueLane`], `DualLeafV1` gives
//! [`PinError::LegacyMethodValueLane`], and the `value-only` research dial is
//! refused as having no value lane to pin on a verify path.
//!
//! # Honest scope
//! This is host-side arithmetic over the query text and the public encoders. It
//! asserts no soundness or privacy property. The value lanes carry the dual-leaf
//! INV-VL downgrade (value↔lexical agreement rests on trusted-issuer honesty;
//! #769 accepted at research grade, CR-G8), and the estate is NOT externally
//! audited (sq-qhy4).

use crate::manifest::{CircuitId, FieldHex, FilterOp, ProofInputs, ProofManifest};
use crate::method_auth::{AuthenticatedMethod, AuthenticatedMethods};
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::dual_leaf_v2;
use sparq_zk::field::{field_to_be_bytes_32, field_to_hex, Fr};
use sparq_zk::verify::{FilterCmp, TypedQueryFilter};

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";

/// The decimal fraction-digit counts a presented lane constant may name.
///
/// The lower bound is 1 and the upper bound is 19, which is the largest scale
/// whose `10^fd` still fits a `u64` — beyond it the host could not even
/// represent the rescaling factor, so admitting it would mean admitting a scale
/// no exact rescale can be computed at. A presented constant outside the range
/// is [`PinError::ScaleNotAdmitted`], never silently clamped.
pub const ADMITTED_FRACTION_DIGITS: std::ops::RangeInclusive<u32> = 1..=19;

/// Which value lane a FILTER constant routes to, and what the member's public
/// inputs would then have to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedLane {
    /// The ONE member this lane's proofs may run on.
    pub member: CircuitId,
    /// The lane constant the member's `datatype_const` must carry.
    pub lane_const: ExpectedLaneConst,
    /// The comparison the member's `op` must encode, variable-on-the-left.
    pub op: FilterOp,
    /// The bound the member must carry, in that lane's own encoding.
    pub bound: ExpectedBound,
}

/// The `datatype_const` a lane admits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedLaneConst {
    /// Exactly this field element.
    Fixed(Fr),
    /// `decimal_lane_const(fd)` for SOME `fd` in [`ADMITTED_FRACTION_DIGITS`].
    /// The decimal lane's constant folds the scale, and the scale the COMMITMENT
    /// was made at is not the query constant's — it is whatever the presented
    /// constant names — so the admitted set is parameterised rather than fixed,
    /// and the bound is then rescaled exactly to it.
    DecimalAtAdmittedScale,
}

/// The bound a lane's member must carry, in that lane's own encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedBound {
    /// The integer and boolean lanes carry a plain `u64` `bound`.
    U64(u64),
    /// The double lane carries the constant's CANONICAL IEEE-754 bit pattern as
    /// `b_bits` (the canonicalisation that folds `-0.0` and NaN payloads).
    Bits(u64),
    /// The dateTime and date lanes carry a sign-split magnitude on the lane's
    /// fixed scaled-epoch timeline.
    Signed { neg: bool, magnitude: u64 },
    /// The decimal lane carries a sign-split magnitude at the scale the
    /// PRESENTED lane constant names; this is the query constant's own
    /// magnitude and scale, to be rescaled exactly to it.
    Decimal { neg: bool, magnitude: u64, fraction_digits: u32 },
}

/// Why a pin could not be computed, or did not hold. Every variant is a
/// fail-closed refusal; none of them is a warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinError {
    /// The issuer signature attested NO commitment method, so there is no
    /// authenticated leaf shape to pin against. A pin is never computed against
    /// an assumed method.
    MethodNotBound,
    /// The authenticated method has no value lane at all
    /// (`StringCanonicalV1` commits a lexical token only).
    MethodHasNoValueLane { method_iri: String },
    /// The authenticated method is the superseded dual-leaf version, whose lane
    /// constants are not the framed V2 derivations this module pins.
    LegacyMethodValueLane,
    /// Two commitments a scan draws from authenticated DIFFERENT methods, so
    /// there is no single leaf shape the scan's columns were committed under.
    MixedScanMethods,
    /// The FILTER constant's datatype is on no value lane.
    UnsupportedConstantDatatype { datatype: String },
    /// The constant's datatype IS on a value lane, but its lexical form is not
    /// one the lane's encoder accepts — a non-canonical spelling, a negative
    /// integer on the non-negative integer lane, an un-timezoned instant.
    ConstantNotEncodable { lexical: String },
    /// The lane the constant routes to is not one the whole-graph builder routes
    /// to, so a committed graph cannot hold that lane's leaves today and a pin
    /// on it would be vacuous.
    LaneNotRouted { datatype: String },
    /// The sub-proof is on a different member than the constant's lane.
    MemberMismatch { expected: String, presented: String },
    /// The presented `datatype_const` is not the lane's constant — or is not
    /// its canonical spelling.
    LaneConstMismatch,
    /// The presented decimal lane constant names no admitted scale, or the query
    /// constant cannot be rescaled to that scale EXACTLY (it would need
    /// rounding, or the rescaled magnitude overflows a `u64`).
    ScaleNotAdmitted,
    /// The presented `op` code is not the query comparison's.
    OpMismatch { expected: u32, presented: u32 },
    /// The presented bound is not the query constant under the lane's encoder.
    BoundMismatch,
    /// The presented verdict is not `true`. A value-lane sub-proof can only
    /// discharge a FILTER it reports as SATISFIED.
    VerdictNotTrue,
    /// The sub-proof is not a value-lane variant at all.
    NotAValueLaneSubProof,
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinError::MethodNotBound => write!(
                f,
                "the issuer signature attests no commitment method, so there is no authenticated leaf shape to pin a value lane against"
            ),
            PinError::MethodHasNoValueLane { method_iri } => write!(
                f,
                "the authenticated commitment method {} has no value lane",
                method_iri
            ),
            PinError::LegacyMethodValueLane => write!(
                f,
                "the authenticated commitment method is the superseded dual-leaf version, whose lane constants are not the ones pinned here"
            ),
            PinError::MixedScanMethods => write!(
                f,
                "a scan's committed graphs authenticated different commitment methods, so there is no single leaf shape to pin against"
            ),
            PinError::UnsupportedConstantDatatype { datatype } => write!(
                f,
                "the FILTER constant's datatype {} is on no value lane",
                datatype
            ),
            PinError::ConstantNotEncodable { lexical } => write!(
                f,
                "the FILTER constant {} is not encodable on its value lane (non-canonical lexical form, or outside the lane's domain)",
                lexical
            ),
            PinError::LaneNotRouted { datatype } => write!(
                f,
                "the value lane for {} is not routed by the whole-graph commitment builder, so a pin on it would be vacuous",
                datatype
            ),
            PinError::MemberMismatch { expected, presented } => write!(
                f,
                "the sub-proof runs on member {} but the FILTER constant's lane is member {}",
                presented, expected
            ),
            PinError::LaneConstMismatch => write!(
                f,
                "the sub-proof's datatype_const is not the canonical spelling of the FILTER constant's lane constant"
            ),
            PinError::ScaleNotAdmitted => write!(
                f,
                "the sub-proof's decimal lane constant names no admitted scale, or the FILTER constant cannot be rescaled to it exactly"
            ),
            PinError::OpMismatch { expected, presented } => write!(
                f,
                "the sub-proof's op code is {} but the query FILTER's is {}",
                presented, expected
            ),
            PinError::BoundMismatch => write!(
                f,
                "the sub-proof's bound is not the FILTER constant under its lane's encoder"
            ),
            PinError::VerdictNotTrue => write!(
                f,
                "the sub-proof's disclosed verdict is not true, so it reports the FILTER as unsatisfied"
            ),
            PinError::NotAValueLaneSubProof => {
                write!(f, "the sub-proof is not a value-lane member")
            }
        }
    }
}

impl std::error::Error for PinError {}

/// The one commitment method EVERY commitment a verified scan draws from
/// authenticated.
///
/// A scan's `k` columns come from `k` committed graphs, and the member that runs
/// against them expects ONE leaf shape. Two graphs under different methods have
/// no common shape, so a mixed scan is [`PinError::MixedScanMethods`] rather
/// than silently pinned against one of them.
///
/// A commitment absent from `methods` counts as
/// [`AuthenticatedMethod::Unbound`], which is what the map's contract says —
/// so a manifest with no authenticated methods at all yields `Unbound` and the
/// caller's method check refuses it. A manifest with NO scan commitments also
/// yields `Unbound`: there is no attested method, which is exactly the
/// fail-closed answer.
pub fn scan_method(
    manifest: &ProofManifest,
    methods: &AuthenticatedMethods,
) -> Result<AuthenticatedMethod, PinError> {
    let mut agreed: Option<AuthenticatedMethod> = None;
    for sp in &manifest.sub_proofs {
        let ProofInputs::Scan { commitments, .. } = &sp.inputs else {
            continue;
        };
        for c in commitments {
            let authenticated = c
                .to_field()
                .and_then(|f| methods.get(&field_to_hex(&f)).copied())
                .unwrap_or(AuthenticatedMethod::Unbound);
            match agreed {
                Some(prev) if prev != authenticated => return Err(PinError::MixedScanMethods),
                _ => agreed = Some(authenticated),
            }
        }
    }
    Ok(agreed.unwrap_or(AuthenticatedMethod::Unbound))
}

/// What a value-lane sub-proof's public inputs would have to be, for this
/// authenticated method, this comparison and this FILTER constant.
///
/// The method axis is decided first (see the module docs): only
/// [`AuthenticatedMethod::Bound`]`(`[`CommitmentMethod::DualLeafV2`]`)` proceeds.
/// Then the constant's datatype selects the lane, and the lane's PUBLIC V2
/// encoder produces the bound — so the bound this returns is the same field the
/// prover would have had to commit, computed by the same code, rather than a
/// second arithmetic path that could disagree with it.
pub fn expected_value_lane(
    method: AuthenticatedMethod,
    op: FilterCmp,
    constant: &oxrdf::Literal,
) -> Result<ExpectedLane, PinError> {
    match method {
        AuthenticatedMethod::Unbound => return Err(PinError::MethodNotBound),
        AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2) => {}
        AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1) => {
            return Err(PinError::LegacyMethodValueLane)
        }
        AuthenticatedMethod::Bound(m) => {
            // `StringCanonicalV1` commits a lexical token only; the `value-only`
            // research dial drops term identity and is never admitted on a
            // verify path. Neither has a V2 value lane to pin.
            return Err(PinError::MethodHasNoValueLane {
                method_iri: m.scheme_iri().to_string(),
            });
        }
    }

    let op = filter_op_of(op);
    let datatype = constant.datatype().as_str().to_string();
    let lexical = constant.to_string();
    let not_encodable = || PinError::ConstantNotEncodable { lexical: lexical.clone() };

    match datatype.as_str() {
        XSD_INTEGER => {
            let c = dual_leaf_v2::encode_integer(constant).map_err(|_| not_encodable())?;
            Ok(ExpectedLane {
                member: CircuitId::FilterValueDl,
                lane_const: ExpectedLaneConst::Fixed(dual_leaf_v2::integer_lane_const()),
                op,
                // The integer lane's handle IS the value, so the member's
                // `bound` is that same `u64`. A negative or non-canonically
                // spelled integer never reaches here: the encoder refuses it.
                bound: ExpectedBound::U64(u64_of_field(&c.value_hook).ok_or_else(not_encodable)?),
            })
        }
        XSD_DOUBLE => {
            let c = dual_leaf_v2::encode_double(constant).map_err(|_| not_encodable())?;
            Ok(ExpectedLane {
                member: CircuitId::FilterValueDlF64,
                lane_const: ExpectedLaneConst::Fixed(dual_leaf_v2::double_lane_const()),
                op,
                bound: ExpectedBound::Bits(
                    u64_of_field(&c.value_hook).ok_or_else(not_encodable)?,
                ),
            })
        }
        XSD_DECIMAL => {
            let c = dual_leaf_v2::encode_decimal(constant).map_err(|_| not_encodable())?;
            let (neg, magnitude) = split_signed_hook(&c.value_hook).ok_or_else(not_encodable)?;
            // The encoder folded the constant's OWN fraction-digit count into
            // its lane constant; recover it by naming the scale whose constant
            // that is. (The scale is public via the constant either way — that
            // is already true of every decimal value-lane proof.)
            let fraction_digits =
                fraction_digits_of(&c.datatype_const).ok_or(PinError::ScaleNotAdmitted)?;
            Ok(ExpectedLane {
                member: CircuitId::FilterValueDlDecimal,
                lane_const: ExpectedLaneConst::DecimalAtAdmittedScale,
                op,
                bound: ExpectedBound::Decimal { neg, magnitude, fraction_digits },
            })
        }
        XSD_BOOLEAN => {
            let c = dual_leaf_v2::encode_boolean(constant).map_err(|_| not_encodable())?;
            Ok(ExpectedLane {
                member: CircuitId::FilterValueDlBool,
                lane_const: ExpectedLaneConst::Fixed(dual_leaf_v2::boolean_lane_const()),
                op,
                bound: ExpectedBound::U64(u64_of_field(&c.value_hook).ok_or_else(not_encodable)?),
            })
        }
        XSD_DATE_TIME => {
            let c = dual_leaf_v2::encode_datetime(constant).map_err(|_| not_encodable())?;
            let (neg, magnitude) = split_signed_hook(&c.value_hook).ok_or_else(not_encodable)?;
            Ok(ExpectedLane {
                member: CircuitId::FilterValueDlDateTime,
                lane_const: ExpectedLaneConst::Fixed(dual_leaf_v2::datetime_lane_const()),
                op,
                bound: ExpectedBound::Signed { neg, magnitude },
            })
        }
        XSD_DATE => {
            let c = dual_leaf_v2::encode_date(constant).map_err(|_| not_encodable())?;
            let (neg, magnitude) = split_signed_hook(&c.value_hook).ok_or_else(not_encodable)?;
            Ok(ExpectedLane {
                // The dateTime and date lanes share ONE member and are
                // separated by the lane constant alone, so pinning that
                // constant is the only thing that keeps a date proof from being
                // read as a dateTime one.
                member: CircuitId::FilterValueDlDateTime,
                lane_const: ExpectedLaneConst::Fixed(dual_leaf_v2::date_lane_const()),
                op,
                bound: ExpectedBound::Signed { neg, magnitude },
            })
        }
        _ => Err(PinError::UnsupportedConstantDatatype { datatype }),
    }
}

/// Whether the value lane a `datatype` routes to is one the WHOLE-GRAPH
/// commitment builder routes to.
///
/// It reads [`dual_leaf_v2::is_hookable_datatype`], the one definition of that
/// routing, so the answer cannot drift from what the builder actually does.
/// `xsd:boolean`, `xsd:dateTime` and `xsd:date` are the documented UNROUTED
/// seam: their standalone encoders exist and feed the value-lane FILTER members,
/// but a committed graph's literals on those datatypes still take the string
/// lane. Pinning a FILTER to a lane no committed leaf is on would be vacuous, so
/// those are refused here with [`PinError::LaneNotRouted`] rather than pinned.
pub fn check_lane_routed(datatype: &str) -> Result<(), PinError> {
    if dual_leaf_v2::is_hookable_datatype(datatype) {
        Ok(())
    } else {
        Err(PinError::LaneNotRouted { datatype: datatype.to_string() })
    }
}

/// The full pin: every equality in the module docs, against one value-lane
/// sub-proof and one typed query FILTER.
///
/// Returns `Ok(())` only when the sub-proof's member, lane constant, op, bound
/// and verdict are ALL exactly what `filter` under `method` requires. It decides
/// nothing about the manifest: the caller still has to establish that THIS
/// sub-proof is the one answering THAT filter, which is the obligation-edge work
/// the closed gates will do.
pub fn check_value_lane_pin(
    method: AuthenticatedMethod,
    filter: &TypedQueryFilter,
    inputs: &ProofInputs,
) -> Result<(), PinError> {
    let expected = expected_value_lane(method, filter.op, &filter.constant)?;
    check_lane_routed(filter.constant.datatype().as_str())?;

    let presented = presented_lane(inputs).ok_or(PinError::NotAValueLaneSubProof)?;

    if presented.member != expected.member {
        return Err(PinError::MemberMismatch {
            expected: expected.member.package(),
            presented: presented.member.package(),
        });
    }

    // The lane constant is compared on the DECODED field element AND the text
    // must be its canonical spelling: `field_from_hex_str` accepts an uppercase,
    // `0x`-less, short or at-or-above-modulus spelling, so a text comparison
    // alone would let a re-spelled constant through.
    let decoded = canonical_field(presented.datatype_const).ok_or(PinError::LaneConstMismatch)?;
    let presented_fd = match expected.lane_const {
        ExpectedLaneConst::Fixed(c) => {
            if decoded != c {
                return Err(PinError::LaneConstMismatch);
            }
            None
        }
        ExpectedLaneConst::DecimalAtAdmittedScale => {
            Some(fraction_digits_of_decoded(&decoded).ok_or(PinError::ScaleNotAdmitted)?)
        }
    };

    if presented.op.code() != expected.op.code() {
        return Err(PinError::OpMismatch {
            expected: expected.op.code(),
            presented: presented.op.code(),
        });
    }

    match (&expected.bound, &presented.bound) {
        (ExpectedBound::U64(want), PresentedBound::U64(got))
        | (ExpectedBound::Bits(want), PresentedBound::U64(got)) => {
            if want != got {
                return Err(PinError::BoundMismatch);
            }
        }
        (ExpectedBound::Signed { neg, magnitude }, PresentedBound::Signed { neg: n, magnitude: m }) => {
            if neg != n || magnitude != m {
                return Err(PinError::BoundMismatch);
            }
        }
        (
            ExpectedBound::Decimal { neg, magnitude, fraction_digits },
            PresentedBound::Signed { neg: n, magnitude: m },
        ) => {
            let fd = presented_fd.ok_or(PinError::ScaleNotAdmitted)?;
            let want = rescale_exact(*magnitude, *fraction_digits, fd)
                .ok_or(PinError::ScaleNotAdmitted)?;
            if neg != n {
                return Err(PinError::BoundMismatch);
            }
            if want != *m {
                return Err(PinError::BoundMismatch);
            }
        }
        _ => return Err(PinError::BoundMismatch),
    }

    if !presented.expected {
        return Err(PinError::VerdictNotTrue);
    }
    Ok(())
}

/// Rescale `magnitude` from `from` fraction digits to `to`, EXACTLY.
///
/// `None` when the rescaling would not be exact: scaling UP must not overflow a
/// `u64`, and scaling DOWN must not discard a non-zero digit. There is no
/// rounding mode, because a rounded bound is a different bound — the query asked
/// about one number and the proof would be about another.
fn rescale_exact(magnitude: u64, from: u32, to: u32) -> Option<u64> {
    if to >= from {
        let factor = 10u64.checked_pow(to - from)?;
        magnitude.checked_mul(factor)
    } else {
        let factor = 10u64.checked_pow(from - to)?;
        if !magnitude.is_multiple_of(factor) {
            return None;
        }
        Some(magnitude / factor)
    }
}

/// The admitted decimal scale a lane constant names, or `None`.
fn fraction_digits_of(datatype_const: &Fr) -> Option<u32> {
    fraction_digits_of_decoded(datatype_const)
}

fn fraction_digits_of_decoded(decoded: &Fr) -> Option<u32> {
    ADMITTED_FRACTION_DIGITS.clone().find(|fd| dual_leaf_v2::decimal_lane_const(*fd) == *decoded)
}

/// Split a signed value handle back into `(negative, magnitude)`.
///
/// The signed lanes fold the sign by FIELD NEGATION, so the handle is either a
/// `u64` (non-negative) or the negation of one. `None` for a field element that
/// is neither — which no lane encoder produces, and which no member could range-
/// check as a handle either.
fn split_signed_hook(f: &Fr) -> Option<(bool, u64)> {
    if let Some(v) = u64_of_field(f) {
        return Some((false, v));
    }
    u64_of_field(&-*f).map(|v| (true, v))
}

/// The `u64` a field element represents, or `None` if it is not one.
fn u64_of_field(f: &Fr) -> Option<u64> {
    let be = field_to_be_bytes_32(f);
    if be[..24].iter().any(|b| *b != 0) {
        return None;
    }
    Some(u64::from_be_bytes(be[24..32].try_into().ok()?))
}

/// Decode a `FieldHex` and accept it only if its text is the CANONICAL rendering
/// of the decoded element. Mirrors `dispatch::canonical_field`, which is
/// crate-private to the dispatch module.
fn canonical_field(h: &FieldHex) -> Option<Fr> {
    let f = h.to_field()?;
    (field_to_hex(&f) == h.0).then_some(f)
}

/// Map a query comparison to its manifest `FilterOp`. The two enums are 1:1 by
/// construction (`FilterCmp::code` and `FilterOp::code` agree), and this is the
/// one place the correspondence is written.
fn filter_op_of(op: FilterCmp) -> FilterOp {
    match op {
        FilterCmp::Lt => FilterOp::Lt,
        FilterCmp::Le => FilterOp::Le,
        FilterCmp::Gt => FilterOp::Gt,
        FilterCmp::Ge => FilterOp::Ge,
        FilterCmp::Eq => FilterOp::Eq,
        FilterCmp::Ne => FilterOp::Ne,
    }
}

/// The bound a value-lane sub-proof actually presents.
enum PresentedBound {
    U64(u64),
    Signed { neg: bool, magnitude: u64 },
}

/// The public inputs of a value-lane sub-proof, read off the VARIANT (never the
/// declared id).
struct PresentedLane<'a> {
    member: CircuitId,
    datatype_const: &'a FieldHex,
    op: FilterOp,
    bound: PresentedBound,
    expected: bool,
}

fn presented_lane(inputs: &ProofInputs) -> Option<PresentedLane<'_>> {
    match inputs {
        ProofInputs::FilterValueDl { op, bound, datatype_const, expected, .. } => {
            Some(PresentedLane {
                member: CircuitId::FilterValueDl,
                datatype_const,
                op: *op,
                bound: PresentedBound::U64(*bound),
                expected: *expected,
            })
        }
        ProofInputs::FilterValueDlF64 { op, b_bits, datatype_const, expected, .. } => {
            Some(PresentedLane {
                member: CircuitId::FilterValueDlF64,
                datatype_const,
                op: *op,
                bound: PresentedBound::U64(*b_bits),
                expected: *expected,
            })
        }
        ProofInputs::FilterValueDlDecimal {
            op,
            bound_neg,
            bound_scaled,
            datatype_const,
            expected,
            ..
        } => Some(PresentedLane {
            member: CircuitId::FilterValueDlDecimal,
            datatype_const,
            op: *op,
            bound: PresentedBound::Signed { neg: *bound_neg, magnitude: *bound_scaled },
            expected: *expected,
        }),
        ProofInputs::FilterValueDlDateTime {
            op,
            bound_neg,
            bound_scaled_epoch,
            datatype_const,
            expected,
            ..
        } => Some(PresentedLane {
            member: CircuitId::FilterValueDlDateTime,
            datatype_const,
            op: *op,
            bound: PresentedBound::Signed { neg: *bound_neg, magnitude: *bound_scaled_epoch },
            expected: *expected,
        }),
        ProofInputs::FilterValueDlBool { op, bound, datatype_const, expected, .. } => {
            Some(PresentedLane {
                member: CircuitId::FilterValueDlBool,
                datatype_const,
                op: *op,
                bound: PresentedBound::U64(*bound),
                expected: *expected,
            })
        }
        _ => None,
    }
}

/// One `(value-lane sub-proof, typed query FILTER)` pin outcome.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueLanePin {
    /// Index into `manifest.sub_proofs`.
    pub(crate) proof: usize,
    /// Index into the query's typed FILTER list.
    pub(crate) filter: usize,
    /// Whether the pin holds, or why it does not.
    pub(crate) outcome: Result<(), PinError>,
}

/// Pin every value-lane sub-proof of `manifest` against every typed FILTER its
/// query imposes, and report each outcome.
///
/// # ⚠️ NOT A VERIFIER
/// This is a test and diagnostic seam. It accepts nothing and refuses nothing:
/// it returns a list of outcomes and makes no decision about the manifest. In
/// particular it does NOT establish that a sub-proof whose pin holds is the one
/// answering that FILTER — there is no obligation edge from a value-lane
/// sub-proof to a query FILTER in this verifier, and the gate that would create
/// one is CLOSED. A manifest every one of whose pins holds is still refused by
/// `verifier::prefilter_manifest_structure` with
/// `CheckError::ValueLaneSubProofNotAccepted`, and a test asserts exactly that.
///
/// It runs the issuer-attestation stage alone to learn the authenticated methods
/// (the full pre-filter would refuse the manifest at stage 0 before reaching
/// it), then one `scan_method` call, then one pin per pair.
#[cfg(test)]
pub(crate) fn value_lane_pin_report(
    manifest: &ProofManifest,
    key_set: &crate::verifier::KeySet,
) -> Result<Vec<ValueLanePin>, crate::verifier::CheckError> {
    let methods = crate::verifier::authenticated_methods_for(manifest, key_set)?;
    let filters = sparq_zk::verify::fragment_typed_filters(&manifest.query)
        .map_err(crate::verifier::CheckError::Sparqzk)?;
    let method = scan_method(manifest, &methods);
    let mut out = Vec::new();
    for (pi, sp) in manifest.sub_proofs.iter().enumerate() {
        if presented_lane(&sp.inputs).is_none() {
            continue;
        }
        for (fi, filter) in filters.iter().enumerate() {
            let outcome = match &method {
                Ok(m) => check_value_lane_pin(*m, filter, &sp.inputs),
                Err(e) => Err(e.clone()),
            };
            out.push(ValueLanePin { proof: pi, filter: fi, outcome });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verifier::KeySet;

    const V2: AuthenticatedMethod = AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2);

    fn lit(value: &str, datatype: &str) -> oxrdf::Literal {
        oxrdf::Literal::new_typed_literal(value, oxrdf::NamedNode::new_unchecked(datatype))
    }

    fn filter(op: FilterCmp, value: &str, datatype: &str) -> TypedQueryFilter {
        TypedQueryFilter { variable: "v".into(), op, constant: lit(value, datatype) }
    }

    fn fh(f: &Fr) -> FieldHex {
        FieldHex(field_to_hex(f))
    }

    // ---------------- the method axis ----------------

    #[test]
    fn only_an_authenticated_dual_leaf_v2_proceeds() {
        let c = lit("1", XSD_INTEGER);
        assert_eq!(
            expected_value_lane(AuthenticatedMethod::Unbound, FilterCmp::Ge, &c),
            Err(PinError::MethodNotBound)
        );
        assert_eq!(
            expected_value_lane(
                AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV1),
                FilterCmp::Ge,
                &c
            ),
            Err(PinError::LegacyMethodValueLane)
        );
        assert_eq!(
            expected_value_lane(
                AuthenticatedMethod::Bound(CommitmentMethod::StringCanonicalV1),
                FilterCmp::Ge,
                &c
            ),
            Err(PinError::MethodHasNoValueLane {
                method_iri: CommitmentMethod::StringCanonicalV1.scheme_iri().to_string()
            })
        );
        assert!(expected_value_lane(V2, FilterCmp::Ge, &c).is_ok());
    }

    #[cfg(feature = "commitment-value-only")]
    #[test]
    fn the_value_only_dial_has_no_value_lane_to_pin() {
        assert_eq!(
            expected_value_lane(
                AuthenticatedMethod::Bound(CommitmentMethod::ValueOnlyV1),
                FilterCmp::Ge,
                &lit("1", XSD_INTEGER)
            ),
            Err(PinError::MethodHasNoValueLane {
                method_iri: CommitmentMethod::ValueOnlyV1.scheme_iri().to_string()
            })
        );
    }

    // ---------------- the lane table ----------------

    /// Every row of the lane table: the datatype selects the member and the lane
    /// constant, and the bound is the constant under that lane's V2 encoder.
    #[test]
    fn the_lane_table_routes_every_datatype_to_its_member_and_constant() {
        let rows: Vec<(&str, &str, CircuitId, Fr)> = vec![
            ("7", XSD_INTEGER, CircuitId::FilterValueDl, dual_leaf_v2::integer_lane_const()),
            ("1.5E0", XSD_DOUBLE, CircuitId::FilterValueDlF64, dual_leaf_v2::double_lane_const()),
            (
                "true",
                XSD_BOOLEAN,
                CircuitId::FilterValueDlBool,
                dual_leaf_v2::boolean_lane_const(),
            ),
            (
                "2020-01-01T00:00:00Z",
                XSD_DATE_TIME,
                CircuitId::FilterValueDlDateTime,
                dual_leaf_v2::datetime_lane_const(),
            ),
            (
                "2020-01-01Z",
                XSD_DATE,
                CircuitId::FilterValueDlDateTime,
                dual_leaf_v2::date_lane_const(),
            ),
        ];
        for (value, datatype, member, lane_const) in rows {
            let e = expected_value_lane(V2, FilterCmp::Ge, &lit(value, datatype))
                .unwrap_or_else(|err| panic!("{value}^^{datatype} must route: {err}"));
            assert_eq!(e.member, member, "{datatype}");
            assert_eq!(e.lane_const, ExpectedLaneConst::Fixed(lane_const), "{datatype}");
            assert_eq!(e.op, FilterOp::Ge);
        }
        // The decimal lane's constant is scale-parameterised, not fixed.
        let d = expected_value_lane(V2, FilterCmp::Lt, &lit("1.50", XSD_DECIMAL)).unwrap();
        assert_eq!(d.member, CircuitId::FilterValueDlDecimal);
        assert_eq!(d.lane_const, ExpectedLaneConst::DecimalAtAdmittedScale);
        assert_eq!(
            d.bound,
            ExpectedBound::Decimal { neg: false, magnitude: 150, fraction_digits: 2 }
        );
        // dateTime and date share ONE member, separated by the constant alone.
        let t = expected_value_lane(V2, FilterCmp::Ge, &lit("2020-01-01T00:00:00Z", XSD_DATE_TIME))
            .unwrap();
        let dd = expected_value_lane(V2, FilterCmp::Ge, &lit("2020-01-01Z", XSD_DATE)).unwrap();
        assert_eq!(t.member, dd.member);
        assert_ne!(t.lane_const, dd.lane_const);
    }

    #[test]
    fn the_integer_and_double_bounds_are_the_lanes_own_encodings() {
        let i = expected_value_lane(V2, FilterCmp::Ge, &lit("18", XSD_INTEGER)).unwrap();
        assert_eq!(i.bound, ExpectedBound::U64(18));
        let d = expected_value_lane(V2, FilterCmp::Ge, &lit("1.5E0", XSD_DOUBLE)).unwrap();
        assert_eq!(
            d.bound,
            ExpectedBound::Bits(sparq_zk::dual_leaf::canonical_f64_bits(1.5f64.to_bits()))
        );
        let b = expected_value_lane(V2, FilterCmp::Eq, &lit("false", XSD_BOOLEAN)).unwrap();
        assert_eq!(b.bound, ExpectedBound::U64(0));
        let b = expected_value_lane(V2, FilterCmp::Eq, &lit("true", XSD_BOOLEAN)).unwrap();
        assert_eq!(b.bound, ExpectedBound::U64(1));
    }

    #[test]
    fn an_unsupported_datatype_and_a_non_encodable_constant_fail_closed() {
        assert_eq!(
            expected_value_lane(V2, FilterCmp::Ge, &lit("x", "http://ex/opaque")),
            Err(PinError::UnsupportedConstantDatatype { datatype: "http://ex/opaque".into() })
        );
        for (value, datatype) in [
            ("018", XSD_INTEGER),           // non-canonical leading zero
            ("-1", XSD_INTEGER),            // the integer lane is non-negative
            ("1", XSD_BOOLEAN),             // XSD-legal but non-canonical boolean
            ("1.5", XSD_DOUBLE),            // non-canonical double (canonical is 1.5E0)
            ("2020-01-01T00:00:00", XSD_DATE_TIME), // un-timezoned
        ] {
            assert!(
                matches!(
                    expected_value_lane(V2, FilterCmp::Ge, &lit(value, datatype)),
                    Err(PinError::ConstantNotEncodable { .. })
                ),
                "{value}^^{datatype} must fail closed"
            );
        }
    }

    /// Numeric promotion is NOT supported: an integer-datatype FILTER constant
    /// against a decimal lane does not route to the decimal member, so the
    /// member equality refuses it rather than promoting.
    #[test]
    fn numeric_promotion_is_not_supported() {
        let i = expected_value_lane(V2, FilterCmp::Ge, &lit("2", XSD_INTEGER)).unwrap();
        assert_eq!(i.member, CircuitId::FilterValueDl);
        let inputs = ProofInputs::FilterValueDlDecimal {
            id: CircuitId::FilterValueDlDecimal,
            operand_enc: FieldHex("0x1".into()),
            op: FilterOp::Ge,
            bound_neg: false,
            bound_scaled: 200,
            datatype_const: fh(&dual_leaf_v2::decimal_lane_const(2)),
            expected: true,
        };
        assert!(matches!(
            check_value_lane_pin(V2, &filter(FilterCmp::Ge, "2", XSD_INTEGER), &inputs),
            Err(PinError::MemberMismatch { .. })
        ));
    }

    // ---------------- routing ----------------

    /// `check_lane_routed` reads the whole-graph builder's own routing, so the
    /// unrouted seam — boolean, dateTime, date — is refused TODAY.
    #[test]
    fn check_lane_routed_refuses_the_unrouted_seam() {
        for dt in [XSD_INTEGER, XSD_DECIMAL, XSD_DOUBLE] {
            assert_eq!(check_lane_routed(dt), Ok(()), "{dt} is routed");
        }
        for dt in [XSD_BOOLEAN, XSD_DATE_TIME, XSD_DATE, "http://ex/opaque"] {
            assert_eq!(
                check_lane_routed(dt),
                Err(PinError::LaneNotRouted { datatype: dt.to_string() }),
                "{dt} is not routed"
            );
        }
    }

    // ---------------- the full pin ----------------

    fn int_inputs(op: FilterOp, bound: u64, lane: Fr, expected: bool) -> ProofInputs {
        ProofInputs::FilterValueDl {
            id: CircuitId::FilterValueDl,
            operand_enc: FieldHex("0x1".into()),
            op,
            bound,
            datatype_const: fh(&lane),
            expected,
        }
    }

    #[test]
    fn a_matching_integer_lane_sub_proof_pins() {
        let inputs = int_inputs(FilterOp::Ge, 18, dual_leaf_v2::integer_lane_const(), true);
        assert_eq!(
            check_value_lane_pin(V2, &filter(FilterCmp::Ge, "18", XSD_INTEGER), &inputs),
            Ok(())
        );
    }

    /// Each of the five equalities, broken one at a time, must give its OWN
    /// refusal — the pin is not one opaque reject.
    #[test]
    fn each_pinned_equality_has_its_own_refusal() {
        let f = filter(FilterCmp::Ge, "18", XSD_INTEGER);
        let lane = dual_leaf_v2::integer_lane_const();

        // (1) member
        let wrong_member = ProofInputs::FilterValueDlBool {
            id: CircuitId::FilterValueDlBool,
            operand_enc: FieldHex("0x1".into()),
            op: FilterOp::Ge,
            bound: 18,
            datatype_const: fh(&dual_leaf_v2::boolean_lane_const()),
            expected: true,
        };
        assert!(matches!(
            check_value_lane_pin(V2, &f, &wrong_member),
            Err(PinError::MemberMismatch { .. })
        ));

        // (2) lane constant — another lane's constant on the right member.
        assert_eq!(
            check_value_lane_pin(
                V2,
                &f,
                &int_inputs(FilterOp::Ge, 18, dual_leaf_v2::boolean_lane_const(), true)
            ),
            Err(PinError::LaneConstMismatch)
        );
        // ... and a NON-CANONICAL spelling of the RIGHT constant.
        let respelled = ProofInputs::FilterValueDl {
            id: CircuitId::FilterValueDl,
            operand_enc: FieldHex("0x1".into()),
            op: FilterOp::Ge,
            bound: 18,
            datatype_const: FieldHex(field_to_hex(&lane).to_uppercase()),
            expected: true,
        };
        assert_eq!(
            check_value_lane_pin(V2, &f, &respelled),
            Err(PinError::LaneConstMismatch)
        );

        // (3) op
        assert_eq!(
            check_value_lane_pin(V2, &f, &int_inputs(FilterOp::Gt, 18, lane, true)),
            Err(PinError::OpMismatch {
                expected: FilterOp::Ge.code(),
                presented: FilterOp::Gt.code()
            })
        );

        // (4) bound
        assert_eq!(
            check_value_lane_pin(V2, &f, &int_inputs(FilterOp::Ge, 17, lane, true)),
            Err(PinError::BoundMismatch)
        );

        // (5) verdict
        assert_eq!(
            check_value_lane_pin(V2, &f, &int_inputs(FilterOp::Ge, 18, lane, false)),
            Err(PinError::VerdictNotTrue)
        );

        // And a non-value-lane sub-proof is not pinnable at all.
        let scan = ProofInputs::FilterInt {
            id: CircuitId::FilterInt { d: 2 },
            operand_enc: FieldHex("0x1".into()),
            op: FilterOp::Ge,
            bound: 18,
            expected: true,
        };
        assert_eq!(
            check_value_lane_pin(V2, &f, &scan),
            Err(PinError::NotAValueLaneSubProof)
        );
    }

    #[test]
    fn the_signed_datetime_bound_is_pinned_sign_and_magnitude() {
        // The pin is computed for the dateTime lane even though
        // `check_lane_routed` refuses it, so exercise the bound comparison
        // through `expected_value_lane` directly.
        let e = expected_value_lane(V2, FilterCmp::Lt, &lit("1969-12-31T23:59:59Z", XSD_DATE_TIME))
            .unwrap();
        let ExpectedBound::Signed { neg, magnitude } = e.bound else {
            panic!("the dateTime lane carries a signed bound");
        };
        assert!(neg, "a pre-epoch instant is negative");
        assert_eq!(magnitude, 1000, "one second before the epoch, in milliseconds");
    }

    // ---------------- the decimal scale ----------------

    /// The presented lane constant names the scale, and the query constant is
    /// rescaled to it EXACTLY in both directions.
    #[test]
    fn the_decimal_bound_is_rescaled_exactly_to_the_presented_scale() {
        let f = filter(FilterCmp::Ge, "1.50", XSD_DECIMAL); // magnitude 150 at fd 2
        let dec = |fd: u32, scaled: u64| ProofInputs::FilterValueDlDecimal {
            id: CircuitId::FilterValueDlDecimal,
            operand_enc: FieldHex("0x1".into()),
            op: FilterOp::Ge,
            bound_neg: false,
            bound_scaled: scaled,
            datatype_const: fh(&dual_leaf_v2::decimal_lane_const(fd)),
            expected: true,
        };
        // Same scale.
        assert_eq!(check_value_lane_pin(V2, &f, &dec(2, 150)), Ok(()));
        // Scaling UP: fd 4 needs 15000.
        assert_eq!(check_value_lane_pin(V2, &f, &dec(4, 15_000)), Ok(()));
        assert_eq!(
            check_value_lane_pin(V2, &f, &dec(4, 150)),
            Err(PinError::BoundMismatch)
        );
        // Scaling DOWN: fd 1 needs 15, exactly (150 / 10 is exact).
        assert_eq!(check_value_lane_pin(V2, &f, &dec(1, 15)), Ok(()));
        // An INEXACT scale-down is refused, not rounded.
        let odd = filter(FilterCmp::Ge, "1.55", XSD_DECIMAL); // 155 at fd 2
        assert_eq!(
            check_value_lane_pin(V2, &odd, &dec(1, 16)),
            Err(PinError::ScaleNotAdmitted)
        );
        assert_eq!(
            check_value_lane_pin(V2, &odd, &dec(1, 15)),
            Err(PinError::ScaleNotAdmitted)
        );
        // A scale the admitted range does not contain.
        let unknown = ProofInputs::FilterValueDlDecimal {
            id: CircuitId::FilterValueDlDecimal,
            operand_enc: FieldHex("0x1".into()),
            op: FilterOp::Ge,
            bound_neg: false,
            bound_scaled: 150,
            datatype_const: fh(&dual_leaf_v2::lane_const(XSD_DECIMAL, "scale=99")),
            expected: true,
        };
        assert_eq!(
            check_value_lane_pin(V2, &f, &unknown),
            Err(PinError::ScaleNotAdmitted)
        );
        // The SIGN is pinned too.
        let negf = filter(FilterCmp::Ge, "-1.50", XSD_DECIMAL);
        assert_eq!(
            check_value_lane_pin(V2, &negf, &dec(2, 150)),
            Err(PinError::BoundMismatch)
        );
    }

    #[test]
    fn rescale_exact_is_exact_in_both_directions_and_refuses_overflow() {
        assert_eq!(rescale_exact(150, 2, 2), Some(150));
        assert_eq!(rescale_exact(150, 2, 4), Some(15_000));
        assert_eq!(rescale_exact(150, 2, 1), Some(15));
        assert_eq!(rescale_exact(155, 2, 1), None, "an inexact scale-down is refused");
        assert_eq!(rescale_exact(1, 0, 19), Some(10u64.pow(19)));
        assert_eq!(rescale_exact(u64::MAX, 0, 1), None, "an overflowing scale-up is refused");
        assert_eq!(rescale_exact(1, 0, 20), None, "a factor beyond u64 is refused");
    }

    #[test]
    fn the_admitted_scale_range_is_one_to_nineteen() {
        assert_eq!(*ADMITTED_FRACTION_DIGITS.start(), 1);
        assert_eq!(*ADMITTED_FRACTION_DIGITS.end(), 19);
        for fd in ADMITTED_FRACTION_DIGITS.clone() {
            assert_eq!(
                fraction_digits_of(&dual_leaf_v2::decimal_lane_const(fd)),
                Some(fd)
            );
        }
        assert_eq!(fraction_digits_of(&dual_leaf_v2::decimal_lane_const(0)), None);
        assert_eq!(fraction_digits_of(&dual_leaf_v2::decimal_lane_const(20)), None);
    }

    // ---------------- helpers ----------------

    /// The REPORT pairs each value-lane sub-proof with each typed FILTER and
    /// records the outcome. With no attestation at all, every commitment is
    /// unbound, so every pin is `MethodNotBound` — the fail-closed answer.
    #[test]
    fn the_pin_report_pairs_every_sub_proof_with_every_filter() {
        let inputs = int_inputs(FilterOp::Ge, 18, dual_leaf_v2::integer_lane_const(), true);
        let query = "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= \"18\"^^<http://www.w3.org/2001/XMLSchema#integer> && ?o < \"99\"^^<http://www.w3.org/2001/XMLSchema#integer>) }";
        let json = format!(
            "{{\"type\":\"urn:sparq:zk:ProofManifest\",\"query\":{},\"attributions\":[],\"entailment_regime\":\"simple\",\"binding\":{{\"mode\":\"challenge\",\"challenge\":\"0x2a\"}},\"sub_proofs\":[{{\"inputs\":{},\"proof_hex\":\"\"}}]}}",
            serde_json::to_string(query).unwrap(),
            serde_json::to_string(&inputs).unwrap()
        );
        let m = ProofManifest::from_json(&json).expect("the minimal manifest parses");
        let report = value_lane_pin_report(&m, &KeySet::empty()).expect("the report runs");
        // One value-lane sub-proof x two FILTER conjuncts.
        assert_eq!(report.len(), 2);
        for pin in &report {
            assert_eq!(pin.proof, 0);
            assert_eq!(
                pin.outcome,
                Err(PinError::MethodNotBound),
                "with no attestation, no method is authenticated"
            );
        }
        assert_eq!(report[0].filter, 0);
        assert_eq!(report[1].filter, 1);
    }

    #[test]
    fn split_signed_hook_round_trips_both_signs() {
        for (neg, magnitude) in [(false, 0u64), (false, 7), (true, 7), (true, u64::MAX)] {
            let f = if neg { -Fr::from(magnitude) } else { Fr::from(magnitude) };
            let (n, m) = split_signed_hook(&f).expect("a lane handle splits");
            // `-0` and `0` are the same element, so the sign of zero is not
            // recoverable — and no encoder produces a negative zero.
            if magnitude != 0 {
                assert_eq!(n, neg);
            }
            assert_eq!(m, magnitude);
        }
    }

    #[test]
    fn u64_of_field_refuses_a_non_u64_element() {
        assert_eq!(u64_of_field(&Fr::from(u64::MAX)), Some(u64::MAX));
        assert_eq!(u64_of_field(&(Fr::from(u64::MAX) + Fr::from(1u64))), None);
    }

    /// `scan_method` requires EVERY referenced commitment to agree, and treats
    /// an unlisted commitment as unbound.
    #[test]
    fn scan_method_requires_all_commitments_to_agree() {
        use crate::manifest::SubProof;
        let a = Fr::from(10u64);
        let b = Fr::from(20u64);
        let scan = |cs: Vec<Fr>| ProofInputs::Scan {
            id: CircuitId::Scan { k: cs.len() as u32, n: 1, r: 1 },
            commitments: cs.iter().map(fh).collect(),
            pattern_is_const: [false; 3],
            pattern_const_enc: [
                FieldHex("0x0".into()),
                FieldHex("0x0".into()),
                FieldHex("0x0".into()),
            ],
            rows: vec![],
            row_count: 0,
            attribution: vec![false; cs.len()],
        };
        let m = |inputs: ProofInputs| ProofManifest {
            r#type: "urn:sparq:zk:ProofManifest".to_string(),
            query: String::new(),
            issuers: vec![],
            key_set: vec![],
            commitment_attestations: vec![],
            attributions: vec![],
            pattern_scans: vec![],
            join_obligations: vec![],
            entailment_regime: crate::manifest::EntailmentRegime::Simple,
            derivation_steps: vec![],
            binding: crate::manifest::BindingMode::Challenge {
                challenge: FieldHex("0x2a".into()),
            },
            revocation: None,
            status_snapshots: vec![],
            sub_proofs: vec![SubProof { inputs, proof_hex: String::new() }],
            binding_edges: vec![],
            join_edges: vec![],
            hidden_revocation: None,
            hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
            fully_hidden_revocation: None,
        };
        let mut methods = AuthenticatedMethods::new();
        methods.insert(field_to_hex(&a), V2);
        methods.insert(field_to_hex(&b), V2);
        assert_eq!(scan_method(&m(scan(vec![a, b])), &methods), Ok(V2));

        // One commitment under a DIFFERENT method.
        let mut mixed = AuthenticatedMethods::new();
        mixed.insert(field_to_hex(&a), V2);
        mixed.insert(
            field_to_hex(&b),
            AuthenticatedMethod::Bound(CommitmentMethod::StringCanonicalV1),
        );
        assert_eq!(
            scan_method(&m(scan(vec![a, b])), &mixed),
            Err(PinError::MixedScanMethods)
        );

        // An UNLISTED commitment is unbound, so a listed + unlisted pair is mixed.
        let mut partial = AuthenticatedMethods::new();
        partial.insert(field_to_hex(&a), V2);
        assert_eq!(
            scan_method(&m(scan(vec![a, b])), &partial),
            Err(PinError::MixedScanMethods)
        );
        // And a manifest with no scan commitments authenticates no method.
        assert_eq!(
            scan_method(&m(scan(vec![])), &AuthenticatedMethods::new()),
            Ok(AuthenticatedMethod::Unbound)
        );
    }
}
