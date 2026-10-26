//! The value-lane obligation model, not yet accepting.
//!
//! This diagnostic predicate binds typed query conjuncts to authenticated scans,
//! active rows and exact slots. No accepting path calls it. The value-lane gate
//! stays CLOSED, including when every obligation discharges here. These are
//! structural checks, not proof verification; private handle domains and the
//! comparison relation remain circuit constraints. Research-grade, not externally
//! audited; the documented INV-VL issuer-honesty assumption still applies.

use std::collections::{BTreeMap, BTreeSet};

use crate::manifest::{ProofInputs, ProofManifest};
use crate::method_auth::{AuthenticatedMethod, AuthenticatedMethods};
use crate::value_pin::{check_value_lane_pin, expected_value_lane, PinError};
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::verify::{fragment_pattern_consts, fragment_patterns, fragment_typed_filters, variable_slots};

/// One typed FILTER conjunct at one active source-scan row and variable slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValueObligation {
    /// Query conjunct index, in `fragment_typed_filters` order.
    pub filter: usize,
    /// Source scan index in `sub_proofs`.
    pub source_proof: usize,
    /// Active source row index.
    pub row: usize,
    /// Subject, predicate or object slot index.
    pub slot: usize,
    /// Query variable name without its question mark.
    pub var: String,
}

/// Diagnostic assignment of each obligation to exactly one value sub-proof.
pub type DischargeMap = BTreeMap<ValueObligation, usize>;

/// A named structural refusal; none of these changes a verifier verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObligationError {
    /// No proof covers this active row and slot.
    Undischarged { obligation: ValueObligation },
    /// Multiple proofs claim the same obligation.
    DischargedTwice { obligation: ValueObligation },
    /// A value proof covers no query obligation.
    OrphanValueProof { proof: usize },
    /// An edge or supplied obligation does not identify the query-derived target.
    EdgeMismatch,
    /// The member or lane disagrees with the query's typed literal.
    LaneMismatch { proof: usize },
    /// The constant, comparison, verdict or operand encoding is wrong.
    PinMismatch { proof: usize, reason: String },
    /// An edge names padding or a nonexistent row.
    RowOutOfRange,
    /// An edge names a slot the query variable does not occupy.
    SlotMismatch,
    /// The source scan authenticates no usable value lane.
    MethodHasNoValueLane { source_proof: usize },
    /// The query contains an operation outside the typed comparison fragment.
    OpNotModelled,
    /// V1 value members are never admitted by a legacy string-lane policy.
    LegacyV1ValueMember { proof: usize },
    /// Existing scan authentication, member or pattern validation failed.
    InvalidScan { reason: String },
}

impl std::fmt::Display for ObligationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "value obligation refused: {self:?}")
    }
}
impl std::error::Error for ObligationError {}

fn invalid_scan(error: crate::verifier::CheckError) -> ObligationError {
    ObligationError::InvalidScan { reason: error.to_string() }
}

/// Derive every obligation from query text and authenticated, validated scans.
///
/// Constant membership remains authoritative: `pattern_scans` cannot narrow the
/// set. Duplicate pattern occurrences at the same variable slot are deduplicated.
///
/// # Errors
/// Refuses unsupported query operations, unauthenticated or incompatible scans,
/// missing patterns and malformed declared pattern-to-scan assignments.
pub fn derive(
    manifest: &ProofManifest,
    methods: &AuthenticatedMethods,
) -> Result<Vec<ValueObligation>, ObligationError> {
    let filters = fragment_typed_filters(&manifest.query).map_err(|_| ObligationError::OpNotModelled)?;
    let patterns = fragment_patterns(&manifest.query).map_err(|_| ObligationError::OpNotModelled)?;
    let consts = fragment_pattern_consts(&patterns);
    let vars = variable_slots(&patterns);
    crate::verifier::check_pattern_scans(manifest, methods).map_err(invalid_scan)?;
    let mut matched = BTreeSet::new();
    let mut matched_filters = BTreeSet::new();
    let mut obligations = BTreeSet::new();
    for (source_proof, sp) in manifest.sub_proofs.iter().enumerate() {
        let ProofInputs::Scan { rows, row_count, .. } = &sp.inputs else { continue };
        let method = crate::verifier::obligation_scan_method(manifest, source_proof, methods)
            .map_err(invalid_scan)?;
        let answers: BTreeSet<usize> = consts.iter().enumerate()
            .filter(|(_, c)| crate::verifier::scan_matches_pattern(
                &sp.inputs, c, Some(AuthenticatedMethod::Bound(method)),
            ))
            .map(|(p, _)| p).collect();
        matched.extend(answers.iter().copied());
        for (filter, f) in filters.iter().enumerate() {
            for (_, _, slot) in vars.iter().filter(|(v, p, _)| v == &f.variable && answers.contains(p)) {
                matched_filters.insert(filter);
                for row in 0..(*row_count as usize).min(rows.len()) {
                    obligations.insert(ValueObligation {
                        filter, source_proof, row, slot: *slot, var: f.variable.clone(),
                    });
                }
            }
        }
    }
    if matched.len() != patterns.len() || matched_filters.len() != filters.len() {
        return Err(ObligationError::EdgeMismatch);
    }
    Ok(obligations.into_iter().collect())
}

/// Check exact value-proof coverage without accepting a manifest.
///
/// `methods` must come from the issuer authentication stage. Each edge must
/// explicitly name its query conjunct (`BindingEdge::filter`), source, row and
/// slot; the proof's `operand_enc` must equal that scan cell. Pins use the typed
/// literal and the source's own authenticated method. General NOT never becomes
/// an `expected=false` obligation; only `!=` and `!(=)` extract as `Ne`.
///
/// # Errors
/// Refuses omitted or invented obligations, wrong edges, lanes, pins or methods,
/// padding rows, duplicates and orphan value proofs. This does not run bb or
/// inspect private witnesses; a passing diagnostic is not an accepted proof.
pub fn discharge(
    manifest: &ProofManifest,
    methods: &AuthenticatedMethods,
    obligations: &[ValueObligation],
) -> Result<DischargeMap, ObligationError> {
    let derived = derive(manifest, methods)?;
    if obligations != derived {
        return Err(ObligationError::EdgeMismatch);
    }
    let filters = fragment_typed_filters(&manifest.query).map_err(|_| ObligationError::OpNotModelled)?;
    let mut out = DischargeMap::new();
    for (proof, sp) in manifest.sub_proofs.iter().enumerate() {
        let Some((member, _)) = crate::dispatch::value_lane_fields(&sp.inputs) else { continue };
        crate::dispatch::refuse_legacy_value_member(&sp.inputs).map_err(|e| match e {
            crate::dispatch::DispatchError::LegacyV1ValueMember { .. } => ObligationError::LegacyV1ValueMember { proof },
            _ => ObligationError::LaneMismatch { proof },
        })?;
        crate::dispatch::check_lane_member_pairing(&sp.inputs)
            .map_err(|_| ObligationError::LaneMismatch { proof })?;
        let mut used = false;
        for edge in manifest.binding_edges.iter().filter(|e| e.to_proof == proof) {
            let Some(source) = manifest.sub_proofs.get(edge.from_proof) else {
                return Err(ObligationError::EdgeMismatch);
            };
            let ProofInputs::Scan { rows, row_count, .. } = &source.inputs else {
                return Err(ObligationError::EdgeMismatch);
            };
            if edge.from_row >= (*row_count as usize).min(rows.len()) {
                return Err(ObligationError::RowOutOfRange);
            }
            let Some(filter) = edge.filter else { return Err(ObligationError::EdgeMismatch) };
            let Some(o) = obligations.iter().find(|o| {
                o.filter == filter && o.source_proof == edge.from_proof
                    && o.row == edge.from_row && o.slot == edge.from_slot
            }) else {
                if obligations.iter().any(|o| o.filter == filter && o.source_proof == edge.from_proof) {
                    return Err(ObligationError::SlotMismatch);
                }
                return Err(ObligationError::EdgeMismatch);
            };
            let method = crate::verifier::obligation_scan_method(manifest, o.source_proof, methods)
                .map_err(invalid_scan)?;
            if method == CommitmentMethod::DualLeafV1 {
                return Err(ObligationError::LegacyV1ValueMember { proof });
            }
            if method != CommitmentMethod::DualLeafV2 {
                return Err(ObligationError::MethodHasNoValueLane { source_proof: o.source_proof });
            }
            let f = &filters[o.filter];
            let method = AuthenticatedMethod::Bound(method);
            let expected = expected_value_lane(method, f.op, &f.constant)
                .map_err(|e| pin_error(proof, e))?;
            if member != &expected.member {
                return Err(ObligationError::LaneMismatch { proof });
            }
            check_value_lane_pin(method, f, &sp.inputs).map_err(|e| pin_error(proof, e))?;
            let operand = match &sp.inputs {
                ProofInputs::FilterValueDl { operand_enc, .. }
                | ProofInputs::FilterValueDlF64 { operand_enc, .. }
                | ProofInputs::FilterValueDlDecimal { operand_enc, .. }
                | ProofInputs::FilterValueDlDateTime { operand_enc, .. }
                | ProofInputs::FilterValueDlBool { operand_enc, .. } => operand_enc,
                _ => return Err(ObligationError::LaneMismatch { proof }),
            };
            // V2 scan rows and value members' operand_enc both carry the full
            // dual-leaf term encoding, not a bare value hook. Reuse stage 2's
            // scan-cell lookup; this diagnostic adds no accepting edge arm.
            if crate::verifier::scan_binding_cell(&source.inputs, o.row, o.slot) != Some(operand) {
                return Err(ObligationError::PinMismatch { proof, reason: "scan cell != operand_enc".into() });
            }
            if out.insert(o.clone(), proof).is_some() {
                return Err(ObligationError::DischargedTwice { obligation: o.clone() });
            }
            used = true;
        }
        if !used {
            return Err(ObligationError::OrphanValueProof { proof });
        }
    }
    for o in obligations {
        if !out.contains_key(o) {
            return Err(ObligationError::Undischarged { obligation: o.clone() });
        }
    }
    Ok(out)
}

fn pin_error(proof: usize, error: PinError) -> ObligationError {
    match error {
        PinError::MemberMismatch { .. } | PinError::LaneConstMismatch | PinError::NotAValueLaneSubProof =>
            ObligationError::LaneMismatch { proof },
        _ => ObligationError::PinMismatch { proof, reason: error.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BindingEdge, CircuitId, FieldHex, FilterOp, SubProof};
    use crate::value_pin::{ExpectedBound, ExpectedLaneConst};
    use sparq_zk::dual_leaf_v2;
    use sparq_zk::field::{field_to_hex, Fr};

    const V2: AuthenticatedMethod = AuthenticatedMethod::Bound(CommitmentMethod::DualLeafV2);
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

    fn fh(n: u64) -> FieldHex { FieldHex(field_to_hex(&Fr::from(n))) }
    fn sub(inputs: ProofInputs) -> SubProof { SubProof { inputs, proof_hex: String::new() } }
    fn query(expr: &str) -> String {
        format!("PREFIX xsd: <{XSD}> SELECT * WHERE {{ ?s ?p ?o FILTER({expr}) }}")
    }
    fn scan(commitment: u64, count: u32, method: AuthenticatedMethod) -> ProofInputs {
        ProofInputs::Scan {
            id: if method == V2 { CircuitId::ScanV2Dl { k: 1, n: 16, r: 4 } }
                else { CircuitId::Scan { k: 1, n: 16, r: 4 } },
            commitments: vec![fh(commitment)],
            pattern_is_const: [false; 3],
            pattern_const_enc: std::array::from_fn(|_| FieldHex("0x0".into())),
            rows: (0..4).map(|r| [fh(10 + r), fh(20 + r), fh(30 + r)]).collect(),
            row_count: count, attribution: vec![true],
        }
    }
    fn proof(query: &str, filter: usize, operand_enc: FieldHex) -> ProofInputs {
        let fs = fragment_typed_filters(query).unwrap();
        let f = &fs[filter];
        let e = expected_value_lane(V2, f.op, &f.constant).unwrap();
        let dt = match e.lane_const {
            ExpectedLaneConst::Fixed(c) => FieldHex(field_to_hex(&c)),
            ExpectedLaneConst::DecimalAtAdmittedScale => FieldHex(field_to_hex(&dual_leaf_v2::decimal_lane_const(2))),
        };
        match e.bound {
            ExpectedBound::U64(bound) if e.member == CircuitId::FilterValueDlBool =>
                ProofInputs::FilterValueDlBool { id: e.member, operand_enc, op: e.op, bound, datatype_const: dt, expected: true },
            ExpectedBound::U64(bound) =>
                ProofInputs::FilterValueDl { id: e.member, operand_enc, op: e.op, bound, datatype_const: dt, expected: true },
            ExpectedBound::Bits(b_bits) =>
                ProofInputs::FilterValueDlF64 { id: e.member, operand_enc, op: e.op, b_bits, datatype_const: dt, expected: true },
            ExpectedBound::Signed { neg, magnitude } =>
                ProofInputs::FilterValueDlDateTime { id: e.member, operand_enc, op: e.op, bound_neg: neg, bound_scaled_epoch: magnitude, datatype_const: dt, expected: true },
            ExpectedBound::Decimal { neg, magnitude, fraction_digits } =>
                ProofInputs::FilterValueDlDecimal { id: e.member, operand_enc, op: e.op, bound_neg: neg,
                    bound_scaled: crate::value_pin::rescale_exact(magnitude, fraction_digits, 2).unwrap(), datatype_const: dt, expected: true },
        }
    }
    fn fixture(expr: &str, count: u32) -> (ProofManifest, AuthenticatedMethods) {
        let query = query(expr);
        let mut m: ProofManifest = serde_json::from_value(serde_json::json!({
            "type": "urn:sparq:zk:ProofManifest", "query": query,
            "attributions": [], "entailment_regime": "simple",
            "binding": {"mode": "challenge", "challenge": fh(42)}, "sub_proofs": []
        })).unwrap();
        m.sub_proofs.push(sub(scan(1, count, V2)));
        let mut methods = AuthenticatedMethods::new();
        methods.insert(fh(1).0, V2);
        complete(&mut m, &methods);
        (m, methods)
    }
    fn complete(m: &mut ProofManifest, methods: &AuthenticatedMethods) {
        for o in derive(m, methods).unwrap() {
            let ProofInputs::Scan { rows, .. } = &m.sub_proofs[o.source_proof].inputs else { unreachable!() };
            let inputs = proof(&m.query, o.filter, rows[o.row][o.slot].clone());
            let to_proof = m.sub_proofs.len();
            m.sub_proofs.push(sub(inputs));
            m.binding_edges.push(BindingEdge {
                from_proof: o.source_proof, from_row: o.row, from_slot: o.slot,
                to_proof, filter: Some(o.filter),
            });
        }
    }
    fn run(m: &ProofManifest, methods: &AuthenticatedMethods) -> Result<DischargeMap, ObligationError> {
        discharge(m, methods, &derive(m, methods)?)
    }
    fn control(m: &ProofManifest, methods: &AuthenticatedMethods) -> DischargeMap {
        run(m, methods).expect("honest control must discharge every obligation")
    }
    fn change_input(m: &mut ProofManifest, proof: usize, key: &str, value: serde_json::Value) {
        let mut v = serde_json::to_value(&m.sub_proofs[proof].inputs).unwrap();
        v[key] = value;
        m.sub_proofs[proof].inputs = serde_json::from_value(v).unwrap();
    }

    #[test]
    fn active_rows_are_distinct_obligations() {
        for count in [1, 3] {
            let (m, methods) = fixture("?o >= 18", count);
            let map = control(&m, &methods);
            assert_eq!(map.len(), count as usize);
            for (row, o) in map.keys().enumerate() { assert_eq!((o.filter, o.source_proof, o.row, o.slot), (0, 0, row, 2)); }
        }
    }

    #[test]
    fn empty_scans_have_no_active_row_obligations() {
        let (mut m, methods) = fixture("?o >= 18", 0);
        assert!(control(&m, &methods).is_empty());
        m.sub_proofs.push(sub(proof(&m.query, 0, fh(30))));
        assert_eq!(run(&m, &methods), Err(ObligationError::OrphanValueProof { proof: 1 }));
    }

    #[test]
    fn missing_row_is_undischarged() {
        let (mut m, methods) = fixture("?o >= 18", 3);
        control(&m, &methods);
        m.sub_proofs.pop(); m.binding_edges.pop();
        assert!(matches!(run(&m, &methods), Err(ObligationError::Undischarged { obligation }) if obligation.row == 2));
    }

    #[test]
    fn duplicate_proof_is_discharged_twice() {
        let (mut m, methods) = fixture("?o >= 18", 1);
        control(&m, &methods);
        m.sub_proofs.push(m.sub_proofs[1].clone());
        let mut edge = m.binding_edges[0].clone(); edge.to_proof = 2; m.binding_edges.push(edge);
        assert!(matches!(run(&m, &methods), Err(ObligationError::DischargedTwice { .. })));
    }

    #[test]
    fn orphan_proof_is_refused() {
        let (mut m, methods) = fixture("?o >= 18", 1);
        control(&m, &methods);
        m.sub_proofs.push(m.sub_proofs[1].clone());
        assert_eq!(run(&m, &methods), Err(ObligationError::OrphanValueProof { proof: 2 }));
    }

    #[test]
    fn edges_require_exact_scan_row_slot_and_conjunct() {
        let (m, methods) = fixture("?o >= 18", 3);
        control(&m, &methods);
        let mut bad = m.clone(); bad.binding_edges[0].from_proof = 1;
        assert_eq!(run(&bad, &methods), Err(ObligationError::EdgeMismatch));
        let mut bad = m.clone(); bad.binding_edges[0].from_row = 1;
        assert!(matches!(run(&bad, &methods), Err(ObligationError::PinMismatch { .. })));
        let mut bad = m.clone(); bad.binding_edges[0].from_slot = 0;
        assert_eq!(run(&bad, &methods), Err(ObligationError::SlotMismatch));
        let mut bad = m.clone(); bad.binding_edges[0].filter = None;
        assert_eq!(run(&bad, &methods), Err(ObligationError::EdgeMismatch));
        let mut bad = m.clone(); bad.binding_edges[0].from_row = 3;
        assert_eq!(run(&bad, &methods), Err(ObligationError::RowOutOfRange));
        let mut bad = m.clone(); bad.binding_edges[0].from_row = 99;
        assert_eq!(run(&bad, &methods), Err(ObligationError::RowOutOfRange));
    }

    #[test]
    fn every_slot_is_a_distinct_obligation_even_with_equal_cells() {
        let (mut m, methods) = fixture("?o >= 18", 1);
        m.query = "SELECT * WHERE { ?s ?p ?o . ?o ?p ?s FILTER(?o >= 18) }".into();
        m.sub_proofs.truncate(1); m.binding_edges.clear();
        if let ProofInputs::Scan { rows, .. } = &mut m.sub_proofs[0].inputs { rows[0][0] = rows[0][2].clone(); }
        complete(&mut m, &methods);
        assert_eq!(control(&m, &methods).len(), 2);
        m.sub_proofs.pop(); m.binding_edges.pop();
        assert!(matches!(run(&m, &methods), Err(ObligationError::Undischarged { obligation }) if obligation.slot == 2));
    }

    #[test]
    fn a_filter_zero_proof_cannot_answer_filter_one() {
        let (mut m, methods) = fixture("?o >= 18 && ?o >= 99", 1);
        control(&m, &methods);
        m.sub_proofs[2] = m.sub_proofs[1].clone();
        assert!(matches!(run(&m, &methods), Err(ObligationError::PinMismatch { proof: 2, .. })));
    }

    #[test]
    fn a_substituted_comparison_or_operand_cannot_discharge() {
        let (m, methods) = fixture("?o >= 18", 1);
        control(&m, &methods);
        let mut bad = m.clone();
        change_input(&mut bad, 1, "op", serde_json::to_value(FilterOp::Gt).unwrap());
        assert!(matches!(run(&bad, &methods), Err(ObligationError::PinMismatch { proof: 1, .. })));
        let mut bad = m.clone();
        change_input(&mut bad, 1, "operand_enc", serde_json::json!(fh(99)));
        assert!(matches!(run(&bad, &methods), Err(ObligationError::PinMismatch { proof: 1, .. })));
    }

    #[test]
    fn scan_authentication_and_pattern_assignments_cannot_be_bypassed() {
        let (m, methods) = fixture("?o >= 18", 1);
        control(&m, &methods);
        assert!(matches!(run(&m, &AuthenticatedMethods::new()), Err(ObligationError::InvalidScan { .. })));
        let mut bad = m.clone();
        bad.pattern_scans = vec![vec![1]];
        assert!(matches!(run(&bad, &methods), Err(ObligationError::InvalidScan { .. })));
    }

    #[test]
    fn source_method_is_per_scan_and_unbound_has_no_value_lane() {
        let (mut m, mut methods) = fixture("?o >= 18", 1);
        control(&m, &methods);
        // An unrelated string scan must not poison the V2 scan's method.
        m.query = "SELECT * WHERE { ?s ?p ?o . ?a <http://ex/unrelated> ?b FILTER(?o >= 18) }".into();
        let mut extra = scan(2, 1, AuthenticatedMethod::Unbound);
        if let ProofInputs::Scan { pattern_is_const, pattern_const_enc, .. } = &mut extra {
            pattern_is_const[1] = true;
            pattern_const_enc[1] = FieldHex(field_to_hex(&sparq_zk::encode::encode_term(
                &oxrdf::NamedNode::new("http://ex/unrelated").unwrap().into(), &Fr::from(0u64),
            ).unwrap()));
        }
        m.sub_proofs.push(sub(extra));
        methods.insert(fh(2).0, AuthenticatedMethod::Unbound);
        // The all-variable filtered pattern also matches the extra scan only if
        // constancy agrees; the extra scan has a constant predicate, so it cannot.
        assert_eq!(control(&m, &methods).len(), 1);
        let (base, _) = fixture("?o >= 18", 1);
        for auth in [AuthenticatedMethod::Unbound, AuthenticatedMethod::Bound(CommitmentMethod::StringCanonicalV1)] {
            let mut bad = base.clone(); bad.sub_proofs[0] = sub(scan(1, 1, auth));
            let map = [(fh(1).0, auth)].into_iter().collect();
            assert_eq!(run(&bad, &map), Err(ObligationError::MethodHasNoValueLane { source_proof: 0 }));
        }
    }

    #[test]
    fn v1_value_members_are_refused_under_every_policy() {
        let (mut m, methods) = fixture("?o >= 18", 1);
        control(&m, &methods);
        change_input(&mut m, 1, "datatype_const", serde_json::json!(FieldHex(field_to_hex(&sparq_zk::dual_leaf::datatype_const(sparq_zk::dual_leaf::XSD_INTEGER)))));
        assert_eq!(run(&m, &methods), Err(ObligationError::LegacyV1ValueMember { proof: 1 }));
        for policy in [crate::MethodPolicy::current(), crate::MethodPolicy::current().admit_legacy_dual_leaf_v1()] {
            let keys = crate::verifier::KeySet::empty().with_method_policy(policy);
            assert!(matches!(crate::verifier::prefilter_manifest_structure(&m, &keys, &crate::verifier::RevocationPolicy::accept_version(1)),
                Err(crate::verifier::CheckError::LegacyV1ValueMember { proof: 1 })));
        }
    }

    #[test]
    fn two_variables_in_two_joined_scans_do_not_cross_pair() {
        let (mut m, mut methods) = fixture("?o >= 18", 1);
        m.query = "SELECT * WHERE { ?s <http://ex/age> ?o . ?s <http://ex/pay> ?v FILTER(?o >= 18 && ?v >= 99) }".into();
        m.sub_proofs.clear(); m.binding_edges.clear();
        for (n, predicate) in [(1, "http://ex/age"), (2, "http://ex/pay")] {
            let mut inputs = scan(n, 1, V2);
            if let ProofInputs::Scan { pattern_is_const, pattern_const_enc, .. } = &mut inputs {
                pattern_is_const[1] = true;
                pattern_const_enc[1] = FieldHex(field_to_hex(&dual_leaf_v2::encode_term(
                    &oxrdf::NamedNode::new(predicate).unwrap().into(), &Fr::from(0u64),
                ).unwrap().unwrap()));
            }
            m.sub_proofs.push(sub(inputs)); methods.insert(fh(n).0, V2);
        }
        complete(&mut m, &methods);
        let map = control(&m, &methods);
        assert_eq!(map.len(), 2);
        assert!(map.keys().all(|o| o.filter == o.source_proof));
        m.binding_edges[0].from_proof = 1;
        assert_eq!(run(&m, &methods), Err(ObligationError::EdgeMismatch));
    }

    #[test]
    fn negative_integer_and_negative_zero_decimal_are_refused() {
        for (honest, bad) in [("?o >= 1", "?o >= \"-1\"^^xsd:integer"),
            ("?o >= \"0.00\"^^xsd:decimal", "?o >= \"-0.00\"^^xsd:decimal")] {
            let (mut m, methods) = fixture(honest, 1); control(&m, &methods);
            m.query = query(bad);
            assert!(matches!(run(&m, &methods), Err(ObligationError::PinMismatch { .. })));
        }
    }

    #[test]
    fn signed_lane_is_temporal_only() {
        let (mut m, methods) = fixture("?o >= \"1960-01-01T00:00:00Z\"^^xsd:dateTime", 1);
        control(&m, &methods);
        m.query = query("?o >= 18");
        assert_eq!(run(&m, &methods), Err(ObligationError::LaneMismatch { proof: 1 }));
        let (m, methods) = fixture("?o >= \"1960-01-01Z\"^^xsd:date", 1);
        control(&m, &methods);
    }

    #[test]
    fn decimals_rescale_exactly_without_matching_query_fraction_digits() {
        let (mut m, methods) = fixture("?o >= \"1.5000\"^^xsd:decimal", 1);
        control(&m, &methods);
        m.query = query("?o >= \"1.5001\"^^xsd:decimal");
        assert!(matches!(run(&m, &methods), Err(ObligationError::PinMismatch { .. })));
    }

    #[test]
    fn doubles_use_canonical_nan_and_zero_and_ieee_comparisons() {
        for literal in ["NaN", "0.0E0", "-0.0E0"] {
            let (mut m, methods) = fixture(&format!("?o != \"{literal}\"^^xsd:double"), 1);
            control(&m, &methods);
            let ProofInputs::FilterValueDlF64 { b_bits, .. } = &m.sub_proofs[1].inputs else { unreachable!() };
            let bits = *b_bits;
            if literal == "NaN" {
                assert_eq!(bits, sparq_zk::dual_leaf::F64_CANONICAL_NAN);
                assert!(f64::from_bits(bits).is_nan());
                assert!(!crate::build::double_bits_verdict(bits, FilterOp::Eq, bits));
                assert!(crate::build::double_bits_verdict(bits, FilterOp::Ne, bits));
                assert!(!crate::build::double_bits_verdict(bits, FilterOp::Le, bits));
            } else {
                assert_eq!(bits, 0);
                assert!(crate::build::double_bits_verdict(0, FilterOp::Eq, (-0.0f64).to_bits()));
            }
            change_input(&mut m, 1, "b_bits", serde_json::json!(bits ^ 1));
            assert!(matches!(run(&m, &methods), Err(ObligationError::PinMismatch { .. })));
        }
    }

    #[test]
    fn boolean_uses_the_member_constraining_both_operands() {
        let (mut m, methods) = fixture("?o = true", 1); control(&m, &methods);
        assert_eq!(m.sub_proofs[1].inputs.circuit_id(), &CircuitId::FilterValueDlBool);
        // Structural check of the actual relation; no bb proof is claimed here.
        let relation = include_str!("../../../zk/compose/compose_core/src/filter_value_bool.nr");
        assert!(relation.contains("assert_bit(value_hook);"));
        assert!(relation.contains("assert_bit(bound_field);"));
        change_input(&mut m, 1, "bound", serde_json::json!(2));
        assert!(matches!(run(&m, &methods), Err(ObligationError::PinMismatch { .. })));
        let (mut m, methods) = fixture("?o = true", 1);
        let ProofInputs::FilterValueDlBool { operand_enc, op, bound, datatype_const, expected, .. } = m.sub_proofs[1].inputs.clone() else { unreachable!() };
        m.sub_proofs[1].inputs = ProofInputs::FilterValueDl { id: CircuitId::FilterValueDl, operand_enc, op, bound, datatype_const, expected };
        assert!(run(&m, &methods).is_err(), "the unconstrained integer member must not answer a boolean FILTER");
    }

    #[test]
    fn only_not_equal_maps_to_ne_and_false_never_discharges() {
        for expr in ["?o != 18", "!(?o = 18)"] {
            let (mut m, methods) = fixture(expr, 1); control(&m, &methods);
            assert_eq!(fragment_typed_filters(&m.query).unwrap()[0].op, sparq_zk::verify::FilterCmp::Ne);
            change_input(&mut m, 1, "expected", serde_json::json!(false));
            assert!(matches!(run(&m, &methods), Err(ObligationError::PinMismatch { .. })));
        }
        let (m, methods) = fixture("?o >= 18", 1); control(&m, &methods);
        let mut bad = m.clone();
        bad.query = query("!(?o > 18)");
        change_input(&mut bad, 1, "op", serde_json::to_value(FilterOp::Gt).unwrap());
        change_input(&mut bad, 1, "expected", serde_json::json!(false));
        assert_eq!(run(&bad, &methods), Err(ObligationError::OpNotModelled),
            "NOT cannot be discharged by a false verdict");
        for expr in ["!(?o > 18)", "!(?o >= 18 && ?o < 99)", "!!(?o = 18)"] {
            let mut bad = m.clone(); bad.query = query(expr);
            assert_eq!(run(&bad, &methods), Err(ObligationError::OpNotModelled));
        }
    }

    #[test]
    fn omitted_or_invented_obligations_are_refused() {
        let (m, methods) = fixture("?o >= 18", 3); control(&m, &methods);
        let mut os = derive(&m, &methods).unwrap(); os.pop();
        assert_eq!(discharge(&m, &methods, &os), Err(ObligationError::EdgeMismatch));
        let mut os = derive(&m, &methods).unwrap(); os[0].var = "other".into();
        assert_eq!(discharge(&m, &methods, &os), Err(ObligationError::EdgeMismatch));
    }

    #[test]
    fn fully_discharged_manifest_is_still_refused_by_verify_manifest() {
        use crate::verifier::*;
        let (m, methods) = fixture("?o >= 18", 3);
        assert_eq!(control(&m, &methods).len(), 3);
        let result = verify_manifest(&m, &crate::driver::CircuitProver::new(std::env::temp_dir()),
            &std::env::temp_dir(), &KeySet::empty(), &RevocationPolicy::accept_version(1),
            &HolderRegistry::empty(), &HolderBindingPolicy::allow_bearer(), &EntailmentPolicy::simple_only(),
            &VerifierNonce::from_field(Fr::from(42u64)), &InMemorySeenNonces::new());
        assert!(matches!(result, Err(CheckError::ValueLaneSubProofNotAccepted { proof: 1 })),
            "fully discharged diagnostics must leave the gate CLOSED: {result:?}");
    }
}
