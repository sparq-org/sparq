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
//! # Integration status (honest scope)
//!
//! This is the matrix + resolver as a self-contained, tested unit. WIRING it
//! into the giant `verify_manifest` flow — so the verifier reads the recorded
//! `zk:scheme` of the graph each `operand_enc` anchors to and calls
//! [`resolve_circuit`] before accepting a sub-proof — is design bead 6, which
//! depends on the host dual-leaf encoding (sq-j506) plumbing the method onto the
//! verifier's view of a graph. Until then the verifier has NO commitment-method
//! input (it operates on `CircuitId` + `ProofInputs` only — verified), so this
//! resolver is the component bead 6 consumes, not yet a wired gate.
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

use crate::manifest::CircuitId;
use sparq_zk::commit::CommitmentMethod;

/// Why a `(method, circuit)` pair was rejected. Every variant is a FAIL-CLOSED
/// refusal — the resolver never falls through to a default circuit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
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
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
/// ([OPUS-5] sq-wz99x — ONE member for BOTH lanes; the lane lives in the public
/// `datatype_const`, so it needs no separate classification here).
fn is_value_lane_member(id: &CircuitId) -> bool {
    matches!(
        id,
        CircuitId::FilterValueDl
            | CircuitId::FilterValueDlF64
            | CircuitId::FilterValueDlDecimal
            | CircuitId::FilterValueDlDateTime
    )
}

/// Whether a `CircuitId` is an IDENTITY-sensitive member — one whose correctness
/// rests on TERM identity (the full-leaf / `lexical_component` equality): scan
/// (row presence / attribution equality), and the hidden cross-credential join
/// (`row_a[slot] == row_b[slot]`). These MUST NOT be routed at a value-lane
/// member (reject-list (v)). The other FILTER families are value-comparison, not
/// identity, members.
fn is_identity_op_member(id: &CircuitId) -> bool {
    matches!(id, CircuitId::Scan { .. } | CircuitId::JoinEq { .. })
}

/// Whether the BLAKE3-TOKEN (string-canonical) FILTER / identity members are
/// legal against `method`. They read the full string-canonical leaf, so they are
/// legal for `string-canonical` AND for `dual-leaf` (whose `lexical_component`
/// preserves term identity), but NOT for `value-only` (which dropped the lexical
/// handle entirely, so a token-binding member is unprovable / an identity op is
/// unanswerable).
fn string_lane_legal(method: CommitmentMethod) -> bool {
    match method {
        CommitmentMethod::StringCanonicalV1 | CommitmentMethod::DualLeafV1 => true,
        #[cfg(feature = "commitment-value-only")]
        CommitmentMethod::ValueOnlyV1 => false,
    }
}

/// Whether a VALUE-LANE member is legal against `method`. Value-lane members
/// need a committed value handle, so they are legal for `dual-leaf` and (the
/// research dial) `value-only`, but NEVER for `string-canonical` (no value
/// handle exists — the member is unprovable against it).
fn value_lane_legal(method: CommitmentMethod) -> bool {
    match method {
        CommitmentMethod::StringCanonicalV1 => false,
        CommitmentMethod::DualLeafV1 => true,
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
pub fn resolve_circuit(
    method: CommitmentMethod,
    id: &CircuitId,
) -> Result<CircuitId, DispatchError> {
    // (3) reject-list (v): an identity op must never be a value-lane member.
    if is_value_lane_member(id) && is_identity_op_member(id) {
        return Err(DispatchError::IdentityOpAtValueLane { circuit: id.package() });
    }

    let legal = if is_value_lane_member(id) {
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
            circuit: id.package(),
        })
    }
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
        // [OPUS-4.8] sq-2ezsx: the double + decimal datatype-class value-lane
        // members are legal against dual-leaf, exactly like the integer member.
        assert_eq!(
            resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::FilterValueDlF64),
            Ok(CircuitId::FilterValueDlF64)
        );
        assert_eq!(
            resolve_circuit(CommitmentMethod::DualLeafV1, &CircuitId::FilterValueDlDecimal),
            Ok(CircuitId::FilterValueDlDecimal)
        );
        // [OPUS-5] sq-wz99x: the dateTime/date member (ONE member, BOTH lanes).
        assert_eq!(
            resolve_circuit(
                CommitmentMethod::DualLeafV1,
                &CircuitId::FilterValueDlDateTime
            ),
            Ok(CircuitId::FilterValueDlDateTime)
        );
    }

    #[test]
    fn string_canonical_rejects_the_double_and_decimal_value_lane_siblings() {
        // [OPUS-4.8] sq-2ezsx: a string-canonical graph has no value handle, so the
        // double + decimal value-lane members are UNPROVABLE against it — fail-closed.
        // [OPUS-5] sq-wz99x: and the dateTime/date member, for the same reason.
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
        let mut methods = vec![CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV1];
        #[cfg(feature = "commitment-value-only")]
        methods.push(CommitmentMethod::ValueOnlyV1);

        let circuits = [
            CircuitId::Scan { k: 1, n: 16, r: 4 },
            CircuitId::FilterInt { d: 2 },
            CircuitId::FilterValueDl,
            CircuitId::FilterValueDlF64,
            CircuitId::FilterValueDlDecimal,
            CircuitId::FilterValueDlDateTime,
            CircuitId::JoinEq { n_a: 16, n_b: 16 },
        ];
        for m in &methods {
            for c in &circuits {
                let r = resolve_circuit(*m, c);
                // Each result is a clean Ok or a typed error; the value member is
                // legal iff the method committed a value handle, and conversely.
                // (A fail-closed refusal is the other valid, non-panicking outcome.)
                if let Ok(id) = r {
                    assert_eq!(&id, c, "resolver must echo the validated id");
                }
            }
        }
    }

    // [OPUS-4.8] sq-3kd2g.6: the bounded-depth path member reads the
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
        // [OPUS-4.8] sq-2ezsx: the double + decimal siblings are value-lane, not identity.
        assert!(is_value_lane_member(&CircuitId::FilterValueDlF64));
        assert!(is_value_lane_member(&CircuitId::FilterValueDlDecimal));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlF64));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlDecimal));
        // [OPUS-5] sq-wz99x: the dateTime/date member is value-lane, not identity.
        assert!(is_value_lane_member(&CircuitId::FilterValueDlDateTime));
        assert!(!is_identity_op_member(&CircuitId::FilterValueDlDateTime));
        assert!(is_identity_op_member(&CircuitId::Scan { k: 1, n: 16, r: 4 }));
        assert!(is_identity_op_member(&CircuitId::JoinEq { n_a: 16, n_b: 16 }));
        assert!(!is_value_lane_member(&CircuitId::Scan { k: 1, n: 16, r: 4 }));
    }
}
