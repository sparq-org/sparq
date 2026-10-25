//! Fail-closed `(commitment-method × circuit)` dispatch matrix (sq-cfmv).
//!
//! OPT-IN, behind the `dual-leaf` cargo feature (OFF by default).
//!
//! The circuit a proof uses MUST match HOW the graph it proves over was
//! committed: a value-bearing FILTER member that recomputes a `value_component`
//! is UNPROVABLE against a `string-canonical` leaf (no value handle exists), and
//! routing an identity operator at the many-to-one `value_component` of a
//! value-lane member is a soundness break (reject-list (v),
//! `research/zk-field-native-encoding.md` §8). The selection must therefore be
//! MACHINE-ENFORCED and FAIL-CLOSED, not advisory.
//!
//! This module is the single source of truth for the legal
//! `(CommitmentMethod, CircuitId)` pairs (the §3.1 legal matrix of
//! `research/zk-configurable-commitment-design.md`). [`resolve_circuit`] is the
//! resolver the verifier consults: it REJECTS any pair outside the matrix, an
//! identity operator routed at a value-lane member, and an unknown / unsupported
//! method — it NEVER silently mis-dispatches or defaults.
//!
//! It is also the single source of truth for the finer LANE↔MEMBER decision
//! (§3.1.1 of the same record): WHICH `datatype_const` may ride WHICH value-lane
//! member. `(method, circuit)` legality is about the leaf SHAPE and so admits a
//! V1 and a V2 graph's value lane alike, while `datatype_const` is a PUBLIC INPUT
//! no shipped member pins — so [`check_lane_const_for_member`] carries an
//! AFFIRMATIVE allow-list over DECODED field elements, and
//! [`check_lane_member_pairing_for_use`] applies it to built `ProofInputs`. ONE
//! table, consulted by the value-lane builders, every value-lane `Prover.toml`
//! renderer (the ordinary ones and the `crate::legacy_v1` ones), the typed
//! checked prove entry `driver::CircuitProver::prove_value_lane` and stage 0 of
//! the verifier's `prefilter_manifest_structure`.
//!
//! The checked prove entry takes typed `crate::value_lane::ValueLaneProof`
//! inputs, derives the member from the lane (the same
//! `crate::value_lane::ValueLane::member` the verifier's id derivation uses), and
//! renders the `Prover.toml` itself from validated fields — it accepts no TOML
//! from its caller. The raw `CircuitProver::{prove, prove_in, gen_witness,
//! gen_witness_tagged}` take a `CircuitId` and a `&str` of TOML and are
//! deliberately UNGATED building blocks: re-proving and witness-solving are
//! legitimate uses and the in-circuit test vectors need to hand a member a
//! pairing this boundary refuses.
//!
//! # Integration status (honest scope)
//!
//! [`resolve_circuit`] IS now consulted verifier-side. The per-sub-proof
//! verification-key selection in `crate::verifier` resolves each sub-proof's
//! member through it, against the AUTHENTICATED commitment method of the
//! commitments that sub-proof is bound to (its own `commitments` for a scan or
//! path, the VALIDATED endpoint commitments of the join edges that cover a join,
//! the source edge's scan for a FILTER), so the key is selected from the resolved
//! member rather than from the declared `CircuitId` alone. The five AUXILIARY bb gates — hidden and fully-hidden
//! revocation, hidden issuer, holder PoK, holder set — are deliberately NOT
//! routed through it: their member is fixed by the relying party's own policy or
//! registry, the manifest declares no id for them, and they read revocation and
//! key/holder-set roots rather than a committed graph's leaf shape.
//!
//! V2 scan/join dispatch uses dedicated component-witnessed members, legal
//! only under `DualLeafV2`. Opaque scan/join ids are legal under string-canonical
//! and legacy `DualLeafV1`, and refused under V2. Neither family is method-independent.
//! This leaves stage 0's value-lane refusals and the value-FILTER gates closed.
//!
//! The LANE↔MEMBER check is different in exactly this respect: it needs no method
//! input, because it decides from the `ProofInputs` alone. It therefore DOES run
//! verifier-side today, as stage 0 of `prefilter_manifest_structure`. That is a
//! structural refusal, not a value-lane gate: the value-lane binding edges and
//! `filter_edge_true` stay closed (sq-j506), which is also why the same stage
//! refuses a value-lane sub-proof no query edge binds.
//!
//! # Honesty (load-bearing)
//!
//! The whole ZK estate is remediated + internally re-audited but **NOT externally
//! audited** (sq-qhy4, P0). The `DualLeafV1` method carries the documented INV-VL
//! downgrade — value↔lexical agreement on the value-FILTER lane is
//! trusted-issuer-honesty, not machine-enforced (#769 accepted; gap CR-G8). This
//! resolver enforces the STRUCTURAL legality (no value member against a method
//! with no value handle; no identity op at a value member); it asserts NO
//! cryptographic soundness or privacy property.

use crate::manifest::{CircuitId, ProofInputs};
use sparq_zk::commit::{CommitmentMethod, MethodNotSelectable, MethodUse};

/// Why a `(method, circuit)` pair was rejected. Every variant is a FAIL-CLOSED
/// refusal — the resolver never falls through to a default circuit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// The id has no compiled V2 member.
    UncompiledCircuit(crate::manifest::CircuitIdError),
    /// The circuit family is not legal for the commitment method: e.g. a
    /// value-bearing `FilterValueDl` member against a `string-canonical` graph
    /// (which committed no value handle), or a blake3-token FILTER against a
    /// `value-only` graph (which committed no lexical handle). Carries the method
    /// `zk:scheme` IRI + the circuit package name for diagnostics.
    IllegalPair { method_iri: String, circuit: String },
    /// An IDENTITY-sensitive operator was routed at a value-lane member
    /// (reject-list (v)): the value handle is many-to-one on the term for
    /// double/decimal, so an identity op (scan-row equality, join, DISTINCT,
    /// sameTerm) MUST read the full leaf / `lexical_component`, NEVER the
    /// `value_component`. Structurally refused here, not left as prose.
    IdentityOpAtValueLane { circuit: String },
    /// The commitment method is unknown / unsupported in this build — e.g. an
    /// `zk:scheme` IRI that `CommitmentMethod::from_scheme_iri` did not recognize
    /// (the `value-only` IRI when its feature is OFF, or a bogus IRI). Rejected,
    /// never defaulted (the closed-enum, fail-closed discipline).
    UnknownMethod { method_iri: String },
    /// A NEW proof was being routed under a method that is not selectable for new
    /// issuance (a superseded version or a research-only dial). Re-verifying an
    /// ALREADY-ISSUED proof is a different use and passes
    /// [`MethodUse::LegacyAlreadyIssued`], which this resolver accepts — so this
    /// variant is only ever raised on the ISSUANCE side.
    MethodNotSelectableForIssuance { method_iri: String, circuit: String },
    /// A `ProofInputs` paired a KNOWN lane constant with a member that is not
    /// that lane's — e.g. the `DualLeafV2` `xsd:boolean` lane constant presented
    /// on the shared integer member [`CircuitId::FilterValueDl`], whose relation
    /// range-decomposes its handle to the whole `u64` domain instead of
    /// constraining it to `{0, 1}`. Raised by
    /// `check_lane_member_pairing_for_use`, which decides by DECODED field
    /// equality against the lane table, so a re-spelled constant cannot slip
    /// past.
    LaneMemberMismatch { circuit: String, lane: String },
    /// A value-lane `datatype_const` decoded to a field element that is NOT in
    /// the lane table of ANY member (an unknown, random or degenerate-fold
    /// constant). The allow-list is AFFIRMATIVE: a constant no shipped lane
    /// derives is refused on every member rather than carried as an unexamined
    /// public input.
    UnknownLaneConst { circuit: String },
    /// A value-lane `datatype_const` that IS this member's lane constant, but a
    /// `DualLeafV1` one, presented for NEW issuance. V1 is superseded, so its
    /// constants are accepted only under `MethodUse::LegacyAlreadyIssued`.
    LegacyLaneConstForIssuance { circuit: String, lane: String },
    /// A `ProofInputs` whose VARIANT and declared `id` disagree: a value-lane
    /// variant carrying another member's id, or — the case that mattered — a
    /// NON-value-lane variant (e.g. `FilterInt`) declaring a value-lane member
    /// id. The second shape used to skip the pairing check entirely, because the
    /// check reads the `datatype_const` off the variant and a `FilterInt` has
    /// none, while the declared id is what later picks the circuit. Refused here
    /// with the same rule the verifier's `derive_circuit_id` applies: the variant
    /// and the id must name the SAME member.
    ValueLaneVariantIdMismatch { circuit: String, variant: String },
    /// A value-lane `FieldHex` whose TEXT is not the canonical rendering of the
    /// field element it decodes to — an uppercase or `0x`-less spelling, a short
    /// (leading-zeros-dropped) one, a 64-nibble value at or above the field
    /// modulus, all of which `sparq_zk::field::field_from_hex_str` otherwise
    /// accepts (it pads and reduces), or text that is not hex at all. Raised for
    /// the public inputs of a value-lane `ProofInputs`, and — by the renderers
    /// and the checked prove entry — for every `FieldHex` that is written into a
    /// `Prover.toml` (the challenge and the private witnesses included). Carries
    /// the offending field's name.
    NonCanonicalFieldHex { circuit: String, field: String },
    /// A `crate::legacy_v1` entry point was handed a lane constant that is not a
    /// `DualLeafV1` constant. The legacy names render and build V1 lanes only;
    /// the ordinary names are the V2 ones.
    NotALegacyV1LaneConst { circuit: String, lane: String },
    /// The checked prove entry was handed a dual-leaf METHOD whose version does
    /// not match the version of the lane constant it carries — a `DualLeafV2`
    /// method with a V1 constant, or the reverse.
    MethodLaneVersionMismatch { method_iri: String, circuit: String, lane: String },
    /// Typed value-lane inputs could not be assembled from a `ProofInputs` and a
    /// witness: the witness has the other shape (a `(value_hook,
    /// lexical_component)` pair for a signed lane, or the reverse), or a
    /// bit-domain boolean bound is outside `{0, 1}`.
    MalformedValueLaneInputs { circuit: String, reason: String },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::UncompiledCircuit(e) => write!(f, "{e}"),
            DispatchError::IllegalPair { method_iri, circuit } => write!(
                f,
                "illegal (commitment-method, circuit) pair: circuit {} is not legal \
                 for a graph committed under {} (fail-closed dispatch matrix, sq-cfmv)",
                circuit, method_iri
            ),
            DispatchError::IdentityOpAtValueLane { circuit } => write!(
                f,
                "identity operator routed at value-lane member {}: the value handle \
                 is many-to-one on the term, so identity ops MUST read the lexical \
                 component (reject-list (v), fail-closed)",
                circuit
            ),
            DispatchError::MethodNotSelectableForIssuance { method_iri, circuit } => write!(
                f,
                "commitment method {} is not selectable for new issuance, so circuit {} \
                 cannot be routed for a NEW proof under it; re-verifying an already-issued \
                 proof passes MethodUse::LegacyAlreadyIssued (fail-closed dispatch matrix)",
                method_iri, circuit
            ),
            DispatchError::LaneMemberMismatch { circuit, lane } => write!(
                f,
                "lane/member mismatch: the {} lane constant was presented on circuit {}, \
                 which is not that lane's member (fail-closed dispatch matrix)",
                lane, circuit
            ),
            DispatchError::UnknownLaneConst { circuit } => write!(
                f,
                "unknown value-lane datatype_const presented on circuit {}: no shipped \
                 lane derives it, so it is refused rather than carried (affirmative \
                 allow-list, fail-closed dispatch matrix)",
                circuit
            ),
            DispatchError::LegacyLaneConstForIssuance { circuit, lane } => write!(
                f,
                "the {} lane constant is a dual-leaf v1 constant, so circuit {} cannot \
                 be routed for a NEW proof under it; re-proving an already-issued graph \
                 passes MethodUse::LegacyAlreadyIssued (fail-closed dispatch matrix)",
                lane, circuit
            ),
            DispatchError::ValueLaneVariantIdMismatch { circuit, variant } => write!(
                f,
                "value-lane variant/id mismatch: ProofInputs::{} declares circuit {}, \
                 but a variant and its id must name the same member (fail-closed \
                 dispatch matrix)",
                variant, circuit
            ),
            DispatchError::NonCanonicalFieldHex { circuit, field } => write!(
                f,
                "non-canonical field hex in value-lane input {} of circuit {}: the text \
                 is not the canonical 0x-prefixed 64-nibble lowercase rendering of the \
                 field element it decodes to (fail-closed dispatch matrix)",
                field, circuit
            ),
            DispatchError::NotALegacyV1LaneConst { circuit, lane } => write!(
                f,
                "the {} lane constant is not a dual-leaf v1 constant, so the legacy_v1 \
                 entry for circuit {} refuses it; the ordinary entry points build and \
                 render the v2 lanes",
                lane, circuit
            ),
            DispatchError::MethodLaneVersionMismatch { method_iri, circuit, lane } => write!(
                f,
                "commitment method {} does not derive the {} lane constant presented \
                 on circuit {}: the method version and the lane constant version must \
                 agree (fail-closed dispatch matrix)",
                method_iri, lane, circuit
            ),
            DispatchError::MalformedValueLaneInputs { circuit, reason } => write!(
                f,
                "malformed value-lane inputs for circuit {}: {}",
                circuit, reason
            ),
            DispatchError::UnknownMethod { method_iri } => write!(
                f,
                "unknown / unsupported commitment method {}: rejected, never defaulted \
                 (fail-closed dispatch matrix, sq-cfmv)",
                method_iri
            ),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Whether a `CircuitId` is a VALUE-LANE member — one that binds the operand via
/// the `value_component` (the numeric handle), available ONLY against a method
/// that committed a value handle (`dual-leaf` / `value-only`). The dual-leaf
/// FILTER members for the integer (sq-xojl), double, and decimal datatype classes
/// (sq-2ezsx) are all value-lane members, as is the dateTime/date member
/// (sq-wz99x — ONE member for BOTH lanes; the lane lives in the public
/// `datatype_const`, so it needs no separate classification here) and the
/// bit-domain `xsd:boolean` member.
fn is_value_lane_member(id: &CircuitId) -> bool {
    matches!(
        id,
        CircuitId::FilterValueDl
            | CircuitId::FilterValueDlF64
            | CircuitId::FilterValueDlDecimal
            | CircuitId::FilterValueDlDateTime
            | CircuitId::FilterValueDlBool
    )
}

/// Whether a `CircuitId` is an IDENTITY-sensitive member — one whose correctness
/// rests on TERM identity (the full-leaf / `lexical_component` equality): scan
/// (row presence / attribution equality), and the hidden cross-credential join
/// (`row_a[slot] == row_b[slot]`). These MUST NOT be routed at a value-lane
/// member (reject-list (v)). The other FILTER families are value-comparison, not
/// identity, members.
fn is_identity_op_member(id: &CircuitId) -> bool {
    matches!(id, CircuitId::Scan { .. } | CircuitId::JoinEq { .. }
        | CircuitId::ScanV2Dl { .. } | CircuitId::JoinEqV2Dl { .. })
}

/// Whether a token FILTER or method-independent auxiliary member may use
/// `method`. Scan/join identity members have their own explicit pairing below.
/// A value-only commitment supplies no lexical handle.
fn string_lane_legal(method: CommitmentMethod) -> bool {
    match method {
        CommitmentMethod::StringCanonicalV1
        | CommitmentMethod::DualLeafV1
        | CommitmentMethod::DualLeafV2 => true,
        #[cfg(feature = "commitment-value-only")]
        CommitmentMethod::ValueOnlyV1 => false,
    }
}

/// Whether a VALUE-LANE member is legal against `method`. Value-lane members
/// need a committed value handle, so they are legal for BOTH `dual-leaf` versions
/// and (the research dial) `value-only`, but NEVER for `string-canonical` (no value
/// handle exists — the member is unprovable against it).
fn value_lane_legal(method: CommitmentMethod) -> bool {
    match method {
        CommitmentMethod::StringCanonicalV1 => false,
        // BOTH dual-leaf versions commit a value handle, so the value lane is
        // STRUCTURALLY legal against either. Legality here is about the leaf
        // SHAPE, not about which version a new issuance may select — that is
        // `CommitmentMethod::is_production_selectable`, a separate axis. A V1
        // graph stays verifiable, so refusing its value lane here would make
        // already-issued credentials unprovable.
        CommitmentMethod::DualLeafV1 | CommitmentMethod::DualLeafV2 => true,
        #[cfg(feature = "commitment-value-only")]
        CommitmentMethod::ValueOnlyV1 => true,
    }
}

/// The fail-closed `(method, circuit)` resolver — the heart of sq-cfmv.
///
/// Returns the `CircuitId` to dispatch (echoed on success — the resolver does not
/// rewrite the id, only validates legality), or a [`DispatchError`] that REJECTS:
///
/// 1. a VALUE-LANE member against a method with no value handle
///    (`string-canonical`) — [`DispatchError::IllegalPair`];
/// 2. a STRING-LANE member (scan / blake3 FILTER / join) against `value-only`
///    (no lexical handle) — [`DispatchError::IllegalPair`];
/// 3. an IDENTITY operator (scan / join) that is also a value-lane member — the
///    reject-list (v) hazard — [`DispatchError::IdentityOpAtValueLane`]. (Today no
///    member is both, so this is a forward guard for the decimal/double value
///    members and any future value-keyed DISTINCT/sameTerm.)
///
/// It NEVER returns a circuit for an illegal pair and NEVER defaults — an
/// unsupported combination is an error, not a silent mis-dispatch.
///
/// This entry is the VERIFICATION / legacy view: it asks only whether the leaf
/// shape admits the member, so an already-issued graph under a superseded method
/// keeps resolving. A path building a NEW proof calls
/// [`resolve_circuit_for_use`] with [`MethodUse::NewIssuance`] instead, which
/// adds the method-selectability refusal.
pub fn resolve_circuit(
    method: CommitmentMethod,
    id: &CircuitId,
) -> Result<CircuitId, DispatchError> {
    id.validate().map_err(DispatchError::UncompiledCircuit)?;
    // (3) reject-list (v): an identity op must never be a value-lane member.
    if is_value_lane_member(id) && is_identity_op_member(id) {
        return Err(DispatchError::IdentityOpAtValueLane { circuit: id.label() });
    }

    let legal = if matches!(id, CircuitId::ScanV2Dl { .. } | CircuitId::JoinEqV2Dl { .. }) {
        method == CommitmentMethod::DualLeafV2
    } else if matches!(id, CircuitId::Scan { .. } | CircuitId::JoinEq { .. }) {
        // Legacy opaque members remain available for already-issued V1 graphs.
        match method {
            CommitmentMethod::StringCanonicalV1 | CommitmentMethod::DualLeafV1 => true,
            CommitmentMethod::DualLeafV2 => true,
            #[cfg(feature = "commitment-value-only")]
            CommitmentMethod::ValueOnlyV1 => false,
        }
    } else if is_value_lane_member(id) {
        // (1) value-lane: legal only where a value handle was committed.
        value_lane_legal(method)
    } else {
        // (2) string-lane (scan / blake3 FILTER / join / revoke / issuer /
        // holder): legal where the string-canonical leaf is present.
        string_lane_legal(method)
    };

    if legal {
        Ok(id.clone())
    } else {
        Err(DispatchError::IllegalPair {
            method_iri: method.scheme_iri().to_string(),
            circuit: id.label(),
        })
    }
}

/// [`resolve_circuit`] with the method-selectability axis made explicit.
///
/// `MethodUse::LegacyAlreadyIssued` is exactly [`resolve_circuit`].
/// `MethodUse::NewIssuance` additionally refuses a method that is not selectable
/// for new issuance with [`DispatchError::MethodNotSelectableForIssuance`] — so a
/// path that routes a FRESH proof cannot pick up a superseded method by accident,
/// while re-verification keeps working.
pub fn resolve_circuit_for_use(
    method: CommitmentMethod,
    id: &CircuitId,
    use_: MethodUse,
) -> Result<CircuitId, DispatchError> {
    if let Err(e) = method.checked_for(use_) {
        return Err(not_selectable(&e, id));
    }
    resolve_circuit(method, id)
}

fn not_selectable(e: &MethodNotSelectable, id: &CircuitId) -> DispatchError {
    DispatchError::MethodNotSelectableForIssuance {
        method_iri: e.scheme_iri().to_string(),
        circuit: id.label(),
    }
}

/// The lane table: every `datatype_const` a SHIPPED value lane derives, with the
/// member it may ride and the commitment-method version that derives it.
///
/// Built once per process. The decimal lanes are enumerated over the fraction-digit
/// counts the host encoder can produce: `sparq_zk::dual_leaf::canonical_decimal_scaled`
/// requires a non-empty fraction (`fd >= 1`) and computes `10^fd` with
/// `checked_pow` on `u64`, so `fd > 19` cannot be encoded at all.
#[cfg(feature = "dual-leaf")]
const DECIMAL_SCALE_MAX: u32 = 19;

/// One row of the lane table.
#[cfg(feature = "dual-leaf")]
struct LaneRow {
    /// The lane's `datatype_const` as a DECODED field element — what the pairing
    /// check compares against, so hex spelling never enters the decision.
    constant: sparq_zk::field::Fr,
    /// Neutral diagnostic label, e.g. `"dual-leaf v2 xsd:boolean"`.
    lane: String,
    /// The ONE member this lane's constant may ride.
    member: CircuitId,
    /// `true` for a `DualLeafV1` constant — accepted only under
    /// [`MethodUse::LegacyAlreadyIssued`].
    legacy_v1: bool,
}

/// The affirmative allow-list, as rows of [`LaneRow`].
///
/// AFFIRMATIVE and BIDIRECTIONAL: a constant is legal on a member iff a row
/// pairs them. A constant with no row at all is [`DispatchError::UnknownLaneConst`]
/// on every member, and a constant whose row names another member is
/// [`DispatchError::LaneMemberMismatch`] — so the dedicated bit-domain boolean
/// member accepts the V2 boolean constant and NOTHING else, and the shared integer
/// member does not accept it.
#[cfg(feature = "dual-leaf")]
fn lane_table() -> &'static [LaneRow] {
    use sparq_zk::{dual_leaf, dual_leaf_boolean, dual_leaf_datetime, dual_leaf_v2};
    static TABLE: std::sync::OnceLock<Vec<LaneRow>> = std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| {
            let mut rows = vec![
                // --- DualLeafV1 (superseded: legacy use only) ---------------
                LaneRow {
                    constant: dual_leaf::datatype_const(dual_leaf::XSD_INTEGER),
                    lane: "dual-leaf v1 xsd:integer".to_string(),
                    member: CircuitId::FilterValueDl,
                    legacy_v1: true,
                },
                // The V1 xsd:boolean lane SHARES the integer member (it has no
                // bit-domain relation of its own) — which is exactly why its
                // constant may ride that member and the V2 one may not.
                LaneRow {
                    constant: dual_leaf_boolean::boolean_datatype_const(),
                    lane: "dual-leaf v1 xsd:boolean".to_string(),
                    member: CircuitId::FilterValueDl,
                    legacy_v1: true,
                },
                LaneRow {
                    constant: dual_leaf::datatype_const(dual_leaf::XSD_DOUBLE),
                    lane: "dual-leaf v1 xsd:double".to_string(),
                    member: CircuitId::FilterValueDlF64,
                    legacy_v1: true,
                },
                LaneRow {
                    constant: dual_leaf_datetime::datetime_datatype_const(),
                    lane: "dual-leaf v1 xsd:dateTime".to_string(),
                    member: CircuitId::FilterValueDlDateTime,
                    legacy_v1: true,
                },
                LaneRow {
                    constant: dual_leaf_datetime::date_datatype_const(),
                    lane: "dual-leaf v1 xsd:date".to_string(),
                    member: CircuitId::FilterValueDlDateTime,
                    legacy_v1: true,
                },
                // --- DualLeafV2 (the production-selectable version) ---------
                LaneRow {
                    constant: dual_leaf_v2::integer_lane_const(),
                    lane: "dual-leaf v2 xsd:integer".to_string(),
                    member: CircuitId::FilterValueDl,
                    legacy_v1: false,
                },
                LaneRow {
                    constant: dual_leaf_v2::double_lane_const(),
                    lane: "dual-leaf v2 xsd:double".to_string(),
                    member: CircuitId::FilterValueDlF64,
                    legacy_v1: false,
                },
                LaneRow {
                    constant: dual_leaf_v2::boolean_lane_const(),
                    lane: "dual-leaf v2 xsd:boolean".to_string(),
                    member: CircuitId::FilterValueDlBool,
                    legacy_v1: false,
                },
                LaneRow {
                    constant: dual_leaf_v2::datetime_lane_const(),
                    lane: "dual-leaf v2 xsd:dateTime".to_string(),
                    member: CircuitId::FilterValueDlDateTime,
                    legacy_v1: false,
                },
                LaneRow {
                    constant: dual_leaf_v2::date_lane_const(),
                    lane: "dual-leaf v2 xsd:date".to_string(),
                    member: CircuitId::FilterValueDlDateTime,
                    legacy_v1: false,
                },
            ];
            // The decimal lanes carry the SCALE in the constant, and the member
            // id does not carry `fd` (`CircuitId::FilterValueDlDecimal` is a unit
            // variant), so every encodable scale is a row. The scale a relying
            // party expects must still be PINNED verifier-side (sq-j506); this
            // table only decides lane-vs-member.
            for fd in 1..=DECIMAL_SCALE_MAX {
                rows.push(LaneRow {
                    constant: dual_leaf::decimal_datatype_const(fd),
                    lane: format!("dual-leaf v1 xsd:decimal@scale={}", fd),
                    member: CircuitId::FilterValueDlDecimal,
                    legacy_v1: true,
                });
                rows.push(LaneRow {
                    constant: dual_leaf_v2::decimal_lane_const(fd),
                    lane: format!("dual-leaf v2 xsd:decimal@scale={}", fd),
                    member: CircuitId::FilterValueDlDecimal,
                    legacy_v1: false,
                });
            }
            rows
        })
        .as_slice()
}

/// The `(circuit, datatype_const)` fields of a VALUE-LANE `ProofInputs`, or
/// `None` for every other variant.
///
/// One place decides what "a value-lane input" is for the pairing check, so a new
/// value-lane variant that is not added here does not silently skip it: the
/// `ProofInputs` match below is exhaustive over the value-lane variants.
#[cfg(feature = "dual-leaf")]
pub fn value_lane_fields(inputs: &ProofInputs) -> Option<(&CircuitId, &crate::manifest::FieldHex)> {
    match inputs {
        ProofInputs::FilterValueDl { id, datatype_const, .. }
        | ProofInputs::FilterValueDlF64 { id, datatype_const, .. }
        | ProofInputs::FilterValueDlDecimal { id, datatype_const, .. }
        | ProofInputs::FilterValueDlDateTime { id, datatype_const, .. }
        | ProofInputs::FilterValueDlBool { id, datatype_const, .. } => Some((id, datatype_const)),
        _ => None,
    }
}

/// The variant name of a NON-value-lane `ProofInputs`, for the mismatch message.
#[cfg(feature = "dual-leaf")]
pub(crate) fn non_value_lane_variant_name(inputs: &ProofInputs) -> &'static str {
    match inputs {
        ProofInputs::Scan { .. } => "Scan",
        ProofInputs::FilterInt { .. } => "FilterInt",
        ProofInputs::FilterF64 { .. } => "FilterF64",
        ProofInputs::FilterSignedInt { .. } => "FilterSignedInt",
        ProofInputs::FilterDecimal { .. } => "FilterDecimal",
        ProofInputs::JoinEq { .. } => "JoinEq",
        #[cfg(feature = "extended-fragment")]
        ProofInputs::PathReach { .. } => "PathReach",
        _ => "<value-lane>",
    }
}

/// Every `FieldHex` PUBLIC INPUT of a value-lane `ProofInputs`, with its field
/// name — the set the canonicality check runs over.
#[cfg(feature = "dual-leaf")]
fn value_lane_field_hexes(
    inputs: &ProofInputs,
) -> Option<(&CircuitId, Vec<(&'static str, &crate::manifest::FieldHex)>)> {
    match inputs {
        ProofInputs::FilterValueDl { id, operand_enc, datatype_const, .. }
        | ProofInputs::FilterValueDlF64 { id, operand_enc, datatype_const, .. }
        | ProofInputs::FilterValueDlDecimal { id, operand_enc, datatype_const, .. }
        | ProofInputs::FilterValueDlDateTime { id, operand_enc, datatype_const, .. }
        | ProofInputs::FilterValueDlBool { id, operand_enc, datatype_const, .. } => Some((
            id,
            vec![("operand_enc", operand_enc), ("datatype_const", datatype_const)],
        )),
        _ => None,
    }
}

/// The LANE↔MEMBER pairing check for ONE member and ONE `datatype_const` — the
/// single function every host path and the verifier's pre-filter share.
///
/// The decision is made on the DECODED field element, against the crate-private
/// lane table `lane_table` (whose rows are tabulated in
/// `research/zk-configurable-commitment-design.md` §3.1.1):
///
/// 1. the text must be CANONICAL — exactly `sparq_zk::field::field_to_hex` of the
///    element it decodes to — else [`DispatchError::NonCanonicalFieldHex`]. This
///    is load-bearing: `sparq_zk::field::field_from_hex_str` accepts an
///    uppercase, `0x`-less, short (left-padded) or at-or-above-modulus (reduced)
///    spelling, so a TEXT comparison would let a re-spelled constant through;
/// 2. the decoded element must have a row in the table, else
///    [`DispatchError::UnknownLaneConst`];
/// 3. that row's member must be THIS member, else
///    [`DispatchError::LaneMemberMismatch`];
/// 4. a `DualLeafV1` row under [`MethodUse::NewIssuance`] is
///    [`DispatchError::LegacyLaneConstForIssuance`].
///
/// A NON-value-lane member is accepted unchanged: it takes no `datatype_const`,
/// so there is no pairing to decide.
///
/// PRECONDITION, stated plainly: this is a HOST-side boundary on what this crate
/// builds, renders and proves, plus a structural pre-filter refusal verifier-side.
/// The verifier now does AUTHENTICATE the commitment method from the issuer
/// signature ([`crate::method_auth`]) and can compute the member, lane, scale and
/// bound a value-lane sub-proof would need ([`crate::value_pin`]) — but the
/// value-lane verifier GATES are still CLOSED: `verifier.rs` refuses every
/// value-lane sub-proof unconditionally at stage 0, and its stage-2 binding edges
/// and `filter_edge_true` are unchanged. Neither a check here nor a computed pin
/// makes a value-lane proof verifier-enforced.
#[cfg(feature = "dual-leaf")]
pub fn check_lane_const_for_member(
    circuit: &CircuitId,
    datatype_const: &crate::manifest::FieldHex,
    use_: MethodUse,
) -> Result<(), DispatchError> {
    if !is_value_lane_member(circuit) {
        return Ok(());
    }
    let row = lane_row(circuit, datatype_const)?;
    if row.legacy_v1 && use_ == MethodUse::NewIssuance {
        return Err(DispatchError::LegacyLaneConstForIssuance {
            circuit: circuit.label(),
            lane: row.lane.clone(),
        });
    }
    Ok(())
}

/// The lane-table row of `datatype_const` on the value-lane member `circuit`:
/// the text must be canonical, the decoded element must have a row, and that
/// row's member must be `circuit`. The shared first half of every lane decision.
#[cfg(feature = "dual-leaf")]
fn lane_row(
    circuit: &CircuitId,
    datatype_const: &crate::manifest::FieldHex,
) -> Result<&'static LaneRow, DispatchError> {
    let decoded = canonical_field(datatype_const).ok_or_else(|| {
        DispatchError::NonCanonicalFieldHex {
            circuit: circuit.label(),
            field: "datatype_const".to_string(),
        }
    })?;
    let Some(row) = lane_table().iter().find(|r| r.constant == decoded) else {
        return Err(DispatchError::UnknownLaneConst { circuit: circuit.label() });
    };
    if &row.member != circuit {
        return Err(DispatchError::LaneMemberMismatch {
            circuit: circuit.label(),
            lane: row.lane.clone(),
        });
    }
    Ok(row)
}

/// The `crate::legacy_v1` form of [`check_lane_const_for_member`]: the constant
/// must ride `circuit` AND be a `DualLeafV1` constant, else
/// [`DispatchError::NotALegacyV1LaneConst`]. The legacy names exist so V1 output
/// is asked for by name; they render nothing else.
#[cfg(feature = "dual-leaf")]
pub(crate) fn check_legacy_v1_lane_const(
    circuit: &CircuitId,
    datatype_const: &crate::manifest::FieldHex,
) -> Result<(), DispatchError> {
    let row = lane_row(circuit, datatype_const)?;
    if !row.legacy_v1 {
        return Err(DispatchError::NotALegacyV1LaneConst {
            circuit: circuit.label(),
            lane: row.lane.clone(),
        });
    }
    Ok(())
}

/// The checked prove entry's version agreement: a `DualLeafV1` method needs a V1
/// lane constant and a `DualLeafV2` method a V2 one, else
/// [`DispatchError::MethodLaneVersionMismatch`]. Other methods carry no
/// dual-leaf version and are decided by the dispatch matrix alone. In
/// particular `ValueOnlyV1` (only under the `commitment-value-only` research
/// feature, and never selectable for new issuance) is admitted on the value-lane
/// members by the matrix, so under `MethodUse::LegacyAlreadyIssued` it accepts
/// a V1 or a V2 lane constant: no value-only leaf encoding exists to say which
/// version it would carry.
#[cfg(feature = "dual-leaf")]
pub(crate) fn check_method_lane_version(
    method: CommitmentMethod,
    circuit: &CircuitId,
    datatype_const: &crate::manifest::FieldHex,
) -> Result<(), DispatchError> {
    let row = lane_row(circuit, datatype_const)?;
    let agrees = match method {
        CommitmentMethod::DualLeafV1 => row.legacy_v1,
        CommitmentMethod::DualLeafV2 => !row.legacy_v1,
        _ => true,
    };
    if agrees {
        Ok(())
    } else {
        Err(DispatchError::MethodLaneVersionMismatch {
            method_iri: method.scheme_iri().to_string(),
            circuit: circuit.label(),
            lane: row.lane.clone(),
        })
    }
}

/// Decode a `FieldHex` and accept it ONLY if its text is the canonical rendering
/// of the decoded element (`0x` + 64 lowercase nibbles, no reduction applied).
/// `None` for a malformed, re-spelled or at-or-above-modulus text. The same
/// decoder serves the lane decision, the `ProofInputs`-level check, the
/// renderers and the checked prove entry.
#[cfg(feature = "dual-leaf")]
pub(crate) fn canonical_field(h: &crate::manifest::FieldHex) -> Option<sparq_zk::field::Fr> {
    let f = h.to_field()?;
    (sparq_zk::field::field_to_hex(&f) == h.0).then_some(f)
}

/// Refuse the first of `fields` whose text is not canonical, naming it. Every
/// value-lane `FieldHex` that is written into a `Prover.toml` passes through
/// here first, so the only text a renderer interpolates is `0x` followed by 64
/// lowercase hex nibbles — nothing that could close a TOML string.
#[cfg(feature = "dual-leaf")]
pub(crate) fn require_canonical(
    circuit: &CircuitId,
    fields: &[(&'static str, &crate::manifest::FieldHex)],
) -> Result<(), DispatchError> {
    for (name, h) in fields {
        if canonical_field(h).is_none() {
            return Err(DispatchError::NonCanonicalFieldHex {
                circuit: circuit.label(),
                field: (*name).to_string(),
            });
        }
    }
    Ok(())
}

/// The LANE↔MEMBER pairing check over built public inputs, in the VERIFICATION /
/// legacy view — [`check_lane_member_pairing_for_use`] with
/// [`MethodUse::LegacyAlreadyIssued`], so an already-issued `DualLeafV1`
/// value-lane proof keeps resolving.
pub fn check_lane_member_pairing(inputs: &ProofInputs) -> Result<(), DispatchError> {
    #[cfg(feature = "dual-leaf")]
    return check_lane_member_pairing_for_use(inputs, MethodUse::LegacyAlreadyIssued);
    #[cfg(not(feature = "dual-leaf"))]
    {
        let _ = inputs;
        Ok(())
    }
}

/// The LANE↔MEMBER pairing check over built public inputs, with the
/// method-selectability axis made explicit.
///
/// Three refusals, in this order:
///
/// 1. the VARIANT and the declared `id` must name the same member
///    ([`DispatchError::ValueLaneVariantIdMismatch`]);
/// 2. the two `FieldHex` public inputs of a value-lane variant — `operand_enc`
///    and `datatype_const` — must be canonically spelled
///    ([`DispatchError::NonCanonicalFieldHex`]);
/// 3. [`check_lane_const_for_member`] for the pairing itself.
///
/// At this level the canonicality rule reaches exactly those two fields; the
/// renderers and the checked prove entry extend it to every `FieldHex` they
/// write (challenge and private witnesses). A non-value-lane variant's
/// `FieldHex` inputs are NOT checked (the existing fixtures spell them short).
/// A NON-value-lane `ProofInputs` variant is accepted unchanged whenever its
/// declared id is NOT a value-lane member — whatever non-value-lane id that is.
/// Agreement between a non-value-lane variant and its own id (e.g. `FilterInt`
/// with a `Scan` id) is NOT decided here; that is the verifier's id derivation.
/// This check adds refusals on the value lane only, and does not re-state the
/// dispatch matrix.
///
/// `MethodUse::LegacyAlreadyIssued` is a CALLER ASSERTION that the graph was
/// already issued under that method. Nothing here proves issuance history — it is
/// not read from a signature, a registry entry or a timestamp — so it narrows what
/// a host path will build or route, and does not attest what happened. This
/// function takes no method, so it does not compare a method version with the
/// constant version; the typed checked prove entry, which does take the method,
/// adds that agreement check.
#[cfg(feature = "dual-leaf")]
pub fn check_lane_member_pairing_for_use(
    inputs: &ProofInputs,
    use_: MethodUse,
) -> Result<(), DispatchError> {
    // The VARIANT and the declared id must name the same member, BEFORE anything
    // reads one of them. Without this, `FilterInt { id: FilterValueDl, .. }` has no
    // `datatype_const` to read, so the whole check below was skipped while the
    // declared id still chose the circuit downstream.
    let declared = inputs.circuit_id();
    match crate::value_lane::ValueLane::of_inputs(inputs) {
        Some(lane) => {
            if crate::value_lane::derive_value_lane_member(inputs).is_none() {
                return Err(DispatchError::ValueLaneVariantIdMismatch {
                    circuit: declared.label(),
                    variant: lane.variant_name().to_string(),
                });
            }
        }
        None => {
            if is_value_lane_member(declared) {
                return Err(DispatchError::ValueLaneVariantIdMismatch {
                    circuit: declared.label(),
                    variant: non_value_lane_variant_name(inputs).to_string(),
                });
            }
            return Ok(());
        }
    }
    let Some((id, fields)) = value_lane_field_hexes(inputs) else {
        return Ok(());
    };
    require_canonical(id, &fields)?;
    let (id, datatype_const) =
        value_lane_fields(inputs).expect("a value-lane variant carries both fields");
    check_lane_const_for_member(id, datatype_const, use_)
}

/// Fail-closed resolution from a recorded `zk:scheme` IRI (rather than a parsed
/// [`CommitmentMethod`]): an unknown / unsupported IRI is REJECTED with
/// [`DispatchError::UnknownMethod`], never defaulted. This is the entry the
/// verifier uses once it reads a graph's recorded method IRI (design bead 6).
pub fn resolve_circuit_for_scheme(
    scheme_iri: &str,
    id: &CircuitId,
) -> Result<CircuitId, DispatchError> {
    match CommitmentMethod::from_scheme_iri(scheme_iri) {
        Some(method) => resolve_circuit(method, id),
        None => Err(DispatchError::UnknownMethod { method_iri: scheme_iri.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the LEGAL matrix (design §3.1) -----------------------------------

    #[test]
    fn dual_leaf_admits_the_value_lane_member() {
        // The #769 method: dual-leaf is the ONE method the value-FILTER member is
        // legal against AND that keeps term identity (via lexical_component).
        let r = resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::FilterValueDl);
        assert_eq!(r, Ok(CircuitId::FilterValueDl));
    }

    #[test]
    fn dual_leaf_admits_the_double_and_decimal_value_lane_siblings() {
        // sq-2ezsx: the double + decimal datatype-class value-lane
        // members are legal against dual-leaf, exactly like the integer member.
        assert_eq!(
            resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::FilterValueDlF64),
            Ok(CircuitId::FilterValueDlF64)
        );
        assert_eq!(
            resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::FilterValueDlDecimal),
            Ok(CircuitId::FilterValueDlDecimal)
        );
        // sq-wz99x: the dateTime/date member (ONE member, BOTH lanes).
        assert_eq!(
            resolve_circuit(
                CommitmentMethod::DualLeafV1,
                &CircuitId::FilterValueDlDateTime
            ),
            Ok(CircuitId::FilterValueDlDateTime)
        );
    }

    #[test]
    fn dual_leaf_v2_admits_every_value_lane_member_exactly_as_v1_does() {
        // The versioned method commits the same leaf SHAPE, so the legality matrix
        // is V1's — including the dedicated bit-domain boolean member, which only
        // V2's boolean lane targets.
        for id in [
            CircuitId::FilterValueDl,
            CircuitId::FilterValueDlF64,
            CircuitId::FilterValueDlDecimal,
            CircuitId::FilterValueDlDateTime,
            CircuitId::FilterValueDlBool,
        ] {
            assert_eq!(
                resolve_circuit(CommitmentMethod::DualLeafV2, &id),
                Ok(id.clone()),
                "{} must be legal against dual-leaf v2",
                id.label()
            );
            assert!(
                resolve_circuit(CommitmentMethod::DualLeafV1, &id).is_ok(),
                "{} must stay legal against dual-leaf v1 (already-issued graphs)",
                id.label()
            );
            // And neither version's value lane is legal against string-canonical.
            assert!(matches!(
                resolve_circuit(CommitmentMethod::StringCanonicalV1, &id),
                Err(DispatchError::IllegalPair { .. })
            ));
        }
        // Token FILTER legality is unchanged; V2 scans use the dedicated member.
        assert!(resolve_circuit(CommitmentMethod::DualLeafV2, &CircuitId::FilterInt { d: 2 }).is_ok());
        assert!(resolve_circuit(
            CommitmentMethod::DualLeafV2,
            &CircuitId::ScanV2Dl { k: 1, n: 16, r: 4 }
        )
        .is_ok());
        // V2's recorded scheme IRI routes through the IRI entry too.
        assert_eq!(
            resolve_circuit_for_scheme(
                CommitmentMethod::DualLeafV2.scheme_iri(),
                &CircuitId::FilterValueDlBool
            ),
            Ok(CircuitId::FilterValueDlBool)
        );
    }

    #[test]
    fn string_canonical_rejects_the_double_and_decimal_value_lane_siblings() {
        // sq-2ezsx: a string-canonical graph has no value handle, so the
        // double + decimal value-lane members are UNPROVABLE against it — fail-closed.
        // sq-wz99x: and the dateTime/date member, for the same reason.
        for id in [
            CircuitId::FilterValueDlF64,
            CircuitId::FilterValueDlDecimal,
            CircuitId::FilterValueDlDateTime,
        ] {
            let r = resolve_circuit(CommitmentMethod::StringCanonicalV1, &id);
            assert!(matches!(r, Err(DispatchError::IllegalPair { .. })));
        }
    }

    #[test]
    fn dual_leaf_also_admits_the_blake3_filter_and_scan_lanes() {
        // dual-leaf carries the lexical_component, so the blake3-token FILTER + the
        // identity ops (scan, join) stay legal (term FILTER / identity on the
        // lexical leaf).
        assert!(resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::FilterInt { d: 2 }).is_ok());
        assert!(resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::Scan { k: 1, n: 16, r: 4 }).is_ok());
        assert!(resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::JoinEq { n_a: 16, n_b: 16 }).is_ok());
    }

    #[test]
    fn string_canonical_admits_the_blake3_lane() {
        // The conservative default + back-compat anchor: every existing member is
        // legal.
        assert!(resolve_circuit(CommitmentMethod::StringCanonicalV1, &CircuitId::FilterInt { d: 2 }).is_ok());
        assert!(resolve_circuit(CommitmentMethod::StringCanonicalV1, &CircuitId::FilterF64 { d: 1 }).is_ok());
        assert!(resolve_circuit(CommitmentMethod::StringCanonicalV1, &CircuitId::Scan { k: 2, n: 64, r: 8 }).is_ok());
        assert!(resolve_circuit(CommitmentMethod::StringCanonicalV1, &CircuitId::JoinEq { n_a: 16, n_b: 64 }).is_ok());
    }

    // --- the FAIL-CLOSED refusals (the load-bearing property) -------------

    #[test]
    fn string_canonical_rejects_the_value_lane_member() {
        // A string-canonical graph has NO value handle, so the value-FILTER member
        // is UNPROVABLE against it — the resolver MUST reject the pair, never
        // silently dispatch it.
        let r = resolve_circuit(CommitmentMethod::StringCanonicalV1, &CircuitId::FilterValueDl);
        assert!(matches!(r, Err(DispatchError::IllegalPair { .. })));
        // The error names the offending method + circuit.
        if let Err(DispatchError::IllegalPair { method_iri, circuit }) = r {
            assert_eq!(circuit, "filter_value_dl_int");
            assert_eq!(method_iri, "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1");
        } else {
            panic!("expected IllegalPair");
        }
    }

    #[test]
    fn unknown_scheme_iri_is_rejected_not_defaulted() {
        // The closed-enum, fail-closed discipline: an unknown / bogus zk:scheme is
        // rejected, NEVER mapped to a default method.
        let r = resolve_circuit_for_scheme(
            "https://sparq.dev/ns/zk#poseidon2-bogus",
            &CircuitId::FilterInt { d: 2 },
        );
        assert!(matches!(r, Err(DispatchError::UnknownMethod { .. })));
        let r2 = resolve_circuit_for_scheme("", &CircuitId::Scan { k: 1, n: 16, r: 4 });
        assert!(matches!(r2, Err(DispatchError::UnknownMethod { .. })));
    }

    #[test]
    fn known_scheme_iri_round_trips_through_the_resolver() {
        // dual-leaf's recorded zk:scheme IRI resolves to the legal value-lane pair.
        let r = resolve_circuit_for_scheme(
            CommitmentMethod::DualLeafV1.scheme_iri(),
            &CircuitId::FilterValueDl,
        );
        assert_eq!(r, Ok(CircuitId::FilterValueDl));
        // string-canonical's IRI rejects the value-lane member (fail-closed).
        let r2 = resolve_circuit_for_scheme(
            CommitmentMethod::StringCanonicalV1.scheme_iri(),
            &CircuitId::FilterValueDl,
        );
        assert!(matches!(r2, Err(DispatchError::IllegalPair { .. })));
    }

    // --- value-only research dial (feature-gated) ------------------------

    #[cfg(feature = "commitment-value-only")]
    #[test]
    fn value_only_admits_the_value_lane_but_rejects_the_string_lane() {
        // value-only committed a value handle but discarded the lexical component, so
        // the value-FILTER member is legal but every string-lane / identity member
        // is rejected (no lexical leaf to bind / no term identity to read).
        assert!(resolve_circuit(CommitmentMethod::ValueOnlyV1, &CircuitId::FilterValueDl).is_ok());
        assert!(matches!(
            resolve_circuit(CommitmentMethod::ValueOnlyV1, &CircuitId::FilterInt { d: 2 }),
            Err(DispatchError::IllegalPair { .. })
        ));
        assert!(matches!(
            resolve_circuit(CommitmentMethod::ValueOnlyV1, &CircuitId::Scan { k: 1, n: 16, r: 4 }),
            Err(DispatchError::IllegalPair { .. })
        ));
    }

    #[cfg(not(feature = "commitment-value-only"))]
    #[test]
    fn value_only_iri_is_an_unknown_method_without_its_feature() {
        // With the value-only feature OFF, its IRI is unknown to
        // `from_scheme_iri`, so the resolver rejects it as an UNKNOWN method —
        // a build that did not opt into the research dial cannot dispatch it.
        let r = resolve_circuit_for_scheme(
            "https://sparq.dev/ns/zk#poseidon2-valuehook-v1",
            &CircuitId::FilterValueDl,
        );
        assert!(matches!(r, Err(DispatchError::UnknownMethod { .. })));
    }

    // --- exhaustive legality sweep over the methods in this build --------

    #[test]
    fn the_matrix_is_total_and_fail_closed_over_the_build_methods() {
        // Every (method, circuit) pair resolves to EITHER Ok OR a typed
        // DispatchError — the resolver is total and never panics / defaults.
        #[allow(unused_mut)]
        let mut methods = vec![
            CommitmentMethod::StringCanonicalV1,
            CommitmentMethod::DualLeafV1,
            CommitmentMethod::DualLeafV2,
        ];
        #[cfg(feature = "commitment-value-only")]
        methods.push(CommitmentMethod::ValueOnlyV1);

        let circuits = [
            CircuitId::Scan { k: 1, n: 16, r: 4 },
            CircuitId::FilterInt { d: 2 },
            CircuitId::FilterValueDl,
            CircuitId::FilterValueDlF64,
            CircuitId::FilterValueDlDecimal,
            CircuitId::FilterValueDlDateTime,
            CircuitId::FilterValueDlBool,
            CircuitId::JoinEq { n_a: 16, n_b: 16 },
            CircuitId::ScanV2Dl { k: 1, n: 16, r: 4 },
            CircuitId::JoinEqV2Dl { n_a: 16, n_b: 16 },
        ];
        for m in &methods {
            for c in &circuits {
                let r = resolve_circuit(*m, c);
                // Each result is a clean Ok or a typed error; the value member is
                // legal iff the method committed a value handle, and conversely.
                // (A fail-closed refusal is the other valid, non-panicking outcome.)
                let expected = match c {
                    CircuitId::ScanV2Dl { .. } | CircuitId::JoinEqV2Dl { .. } =>
                        *m == CommitmentMethod::DualLeafV2,
                    CircuitId::Scan { .. } | CircuitId::JoinEq { .. } =>
                        matches!(m, CommitmentMethod::StringCanonicalV1 | CommitmentMethod::DualLeafV1),
                    _ => value_lane_legal(*m),
                };
                let expected = if matches!(c, CircuitId::FilterInt { .. }) {
                    string_lane_legal(*m)
                } else { expected };
                assert_eq!(r.is_ok(), expected, "{m:?} × {c:?}");
                if let Ok(id) = r { assert_eq!(&id, c); }
            }
        }
    }

    // sq-3kd2g.6: the bounded-depth path member reads the
    // string-canonical leaf (committed term encodings), so it is a STRING-LANE
    // member — legal for `string-canonical` + `dual-leaf`, rejected for the
    // `value-only` research dial (no lexical handle). It is NEVER a value-lane
    // member (it binds no `value_component`), so it resolves via the else branch
    // with no dispatch-matrix change; this pins that classification.
    #[cfg(feature = "extended-fragment")]
    #[test]
    fn path_reach_resolves_as_a_string_lane_member() {
        let id = CircuitId::PathReach { d: 4, k: 2, n: 16 };
        assert!(!is_value_lane_member(&id));
        assert_eq!(
            resolve_circuit(CommitmentMethod::StringCanonicalV1, &id),
            Ok(id.clone())
        );
        assert_eq!(resolve_circuit(CommitmentMethod::DualLeafV1, &id), Ok(id.clone()));
        #[cfg(feature = "commitment-value-only")]
        assert!(matches!(
            resolve_circuit(CommitmentMethod::ValueOnlyV1, &id),
            Err(DispatchError::IllegalPair { .. })
        ));
    }

    #[test]
    fn identity_op_at_value_lane_guard_is_present() {
        // Forward guard for reject-list (v): if a member were ever BOTH a value-lane
        // member AND an identity op, the resolver rejects it. Today no member is
        // both (the value member is a comparison, not an identity op), so this
        // asserts the guard exists and the current members are correctly classified.
        assert!(is_value_lane_member(&CircuitId::FilterValueDl));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDl));
        // sq-2ezsx: the double + decimal siblings are value-lane, not identity.
        assert!(is_value_lane_member(&CircuitId::FilterValueDlF64));
        assert!(is_value_lane_member(&CircuitId::FilterValueDlDecimal));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlF64));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlDecimal));
        // sq-wz99x: the dateTime/date member is value-lane, not identity.
        assert!(is_value_lane_member(&CircuitId::FilterValueDlDateTime));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlDateTime));
        // The bit-domain boolean member is value-lane, not identity.
        assert!(is_value_lane_member(&CircuitId::FilterValueDlBool));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlBool));
        assert!(is_identity_op_member(&CircuitId::Scan { k: 1, n: 16, r: 4 }));
        assert!(is_identity_op_member(&CircuitId::JoinEq { n_a: 16, n_b: 16 }));
        assert!(!is_value_lane_member(&CircuitId::Scan { k: 1, n: 16, r: 4 }));
    }
}
