//! V2 credentials and typed builders shared by the flat value-FILTER live tests.
use super::*;
use sparq_zk::commit::{CommitmentMethod, MethodUse};
use sparq_zk::dual_leaf_v2;
use sparq_zk::field::field_from_hash_bytes;
use sparq_zk_compose::build::{
    build_filter_value_dl_bool, build_filter_value_dl_datetime, build_filter_value_dl_decimal,
    build_filter_value_dl_f64, build_filter_value_dl_int,
};
use sparq_zk_compose::driver::ProofArtifacts;
use sparq_zk_compose::value_lane::ValueLaneProof;

pub(super) fn literal(lex: &str, datatype: &str) -> Literal {
    Literal::new_typed_literal(
        lex,
        iri(&format!("http://www.w3.org/2001/XMLSchema#{datatype}")),
    )
}

pub(super) fn built(value: &Literal, bound: &Literal) -> (ProofInputs, ValueLaneProof) {
    match value.datatype().as_str().rsplit('#').next().unwrap() {
        "integer" => {
            let b = build_filter_value_dl_int(value, FilterOp::Ge, bound).unwrap();
            (b.inputs.clone(), b.lane_proof().unwrap())
        }
        "decimal" => {
            let b = build_filter_value_dl_decimal(value, FilterOp::Ge, bound).unwrap();
            (b.inputs.clone(), b.lane_proof().unwrap())
        }
        "double" => {
            let b = build_filter_value_dl_f64(value, FilterOp::Ge, bound).unwrap();
            (b.inputs.clone(), b.lane_proof().unwrap())
        }
        "dateTime" => {
            let b = build_filter_value_dl_datetime(value, FilterOp::Ge, bound).unwrap();
            (b.inputs.clone(), b.lane_proof().unwrap())
        }
        "boolean" => {
            let b =
                build_filter_value_dl_bool(value, FilterOp::Ge, bound.value() == "true").unwrap();
            (b.inputs.clone(), b.lane_proof().unwrap())
        }
        _ => unreachable!(),
    }
}

// As in v2_members::scan_toml, V2 scans need raw component witnesses: the opaque
// build_scan/prover_toml_for path deliberately cannot render V2 components.
pub(super) fn fixture(
    values: &[Literal],
    bound: &Literal,
) -> (ProofManifest, String, Vec<ValueLaneProof>) {
    use super::v2_members::put;
    let salt = Fr::from(7u64);
    let triples: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            Triple::new(
                iri(&format!("http://ex/s{i}")),
                iri("http://ex/p"),
                value.clone(),
            )
        })
        .collect();
    let graph = dual_leaf_v2::commit_triples(&triples, salt).unwrap();
    let zero = || std::array::from_fn(|_| FieldHex::from_field(&Fr::from(0u64)));
    let mut rows = Vec::new();
    let mut tc = vec![zero(); 16];
    let mut vs = tc.clone();
    let mut dt = tc.clone();
    let mut lc = tc.clone();
    let mut ids = tc.clone();
    let mut hv = vec![[false; 3]; 16];
    let hl = hv.clone();
    let mut proofs = Vec::new();
    let mut sub_proofs = Vec::new();
    for (r, triple) in graph.canonical.triples.iter().enumerate() {
        let Term::Literal(value) = &triple.object else {
            unreachable!()
        };
        let components = match value.datatype().as_str().rsplit('#').next().unwrap() {
            "integer" => dual_leaf_v2::encode_integer(value),
            "decimal" => dual_leaf_v2::encode_decimal(value),
            "double" => dual_leaf_v2::encode_double(value),
            "dateTime" => dual_leaf_v2::encode_datetime(value),
            "boolean" => dual_leaf_v2::encode_boolean(value),
            _ => unreachable!(),
        }
        .unwrap();
        let terms = [
            Term::from(triple.subject.clone()),
            Term::from(triple.predicate.clone()),
            triple.object.clone(),
        ];
        rows.push(terms.map(|t| {
            FieldHex::from_field(&dual_leaf_v2::encode_term(&t, &salt).unwrap().unwrap())
        }));
        tc[r] = [
            sparq_zk::encode::TYPE_CODE_IRI,
            sparq_zk::encode::TYPE_CODE_IRI,
            sparq_zk::encode::TYPE_CODE_LITERAL,
        ]
        .map(|n| FieldHex::from_field(&Fr::from(n)));
        hv[r][2] = true;
        vs[r] = [
            dual_leaf_v2::value_none(),
            dual_leaf_v2::value_none(),
            components.value_hook,
        ]
        .map(|f| FieldHex::from_field(&f));
        dt[r][2] = FieldHex::from_field(&components.datatype_const);
        lc[r] = std::array::from_fn(|_| {
            FieldHex::from_field(&Fr::from(sparq_zk::dual_leaf::LANG_NONE))
        });
        let NamedOrBlankNode::NamedNode(s) = &triple.subject else {
            unreachable!()
        };
        ids[r] = [
            field_from_hash_bytes(blake3::hash(s.as_str().as_bytes()).as_bytes()),
            field_from_hash_bytes(blake3::hash(triple.predicate.as_str().as_bytes()).as_bytes()),
            components.lexical_component,
        ]
        .map(|f| FieldHex::from_field(&f));
        let (inputs, proof) = built(value, bound);
        sub_proofs.push(SubProof {
            inputs,
            proof_hex: String::new(),
        });
        proofs.push(proof);
    }
    let (mut m, _) = super::v2_members::fixture(true, false);
    m.query = format!("SELECT ?s WHERE {{ ?s <http://ex/p> ?o FILTER(?o >= {bound}) }}");
    let scan = ProofInputs::Scan {
        id: CircuitId::scan_v2_dl(1, 16, 4).unwrap(),
        commitments: vec![FieldHex::from_field(&graph.commitment)],
        pattern_is_const: [false, true, false],
        // Match build_scan's exact variable-slot filler. The structural pattern
        // check compares FieldHex strings, so a padded field zero is different.
        pattern_const_enc: [FieldHex("0x0".into()), rows[0][1].clone(), FieldHex("0x0".into())],
        rows: rows.clone(),
        row_count: rows.len() as u32,
        attribution: vec![true],
    };
    m.sub_proofs = vec![SubProof {
        inputs: scan,
        proof_hex: String::new(),
    }];
    m.sub_proofs.extend(sub_proofs);
    m.binding_edges = (0..rows.len())
        .map(|row| BindingEdge {
            from_proof: 0,
            from_row: row,
            from_slot: 2,
            to_proof: row + 1,
            filter: Some(0),
        })
        .collect();
    let mut att = attest_with_salt(graph.commitment, salt, &test_issuer_sk(1));
    let status = sparq_zk::sig::status_ref_digest(
        &sparq_zk::sig::status_list_id_to_field(FIXTURE_STATUS_LIST),
        FIXTURE_STATUS_INDEX,
        FIXTURE_STATUS_VERSION,
    );
    att.signature = test_issuer_sk(1).sign_presentation_with_status_and_scheme(
        &graph.commitment,
        &salt,
        &status,
        &sparq_zk::sig::scheme_iri_to_field(CommitmentMethod::DualLeafV2.scheme_iri()),
    );
    m.commitment_attestations = vec![att];
    let ProofInputs::Scan {
        commitments,
        pattern_is_const,
        pattern_const_enc,
        row_count,
        attribution,
        ..
    } = &m.sub_proofs[0].inputs
    else {
        unreachable!()
    };
    let mut toml = String::new();
    put(&mut toml, "challenge", &FieldHex("0x2a".into()));
    put(&mut toml, "commitments", commitments);
    put(&mut toml, "pattern_is_const", pattern_is_const);
    put(&mut toml, "pattern_const_enc", pattern_const_enc);
    rows.resize(4, zero());
    put(&mut toml, "rows", &rows);
    put(&mut toml, "row_count", row_count);
    put(&mut toml, "attribution", attribution);
    put(&mut toml, "counts", &[*row_count]);
    for (name, data) in [
        ("type_codes", tc),
        ("value_slot0", vs),
        ("datatype_consts", dt),
        ("lang_consts", lc),
        ("identities", ids),
    ] {
        put(&mut toml, name, &[data]);
    }
    put(&mut toml, "has_value", &[hv]);
    put(&mut toml, "has_lang", &[hl]);
    (m, toml, proofs)
}

pub(super) fn prove(
    m: &mut ProofManifest,
    scan: &str,
    proofs: &[ValueLaneProof],
    tag: &str,
) -> Vec<ProofArtifacts> {
    let p = CircuitProver::from_crate_root();
    let art = p
        .prove_in(
            m.sub_proofs[0].inputs.circuit_id(),
            scan,
            &scratch(&format!("{tag}_scan")),
            tag,
        )
        .unwrap();
    m.sub_proofs[0].proof_hex = encode_artifacts(&art);
    proofs
        .iter()
        .enumerate()
        .map(|(i, proof)| {
            let art = p
                .prove_value_lane(
                    CommitmentMethod::DualLeafV2,
                    MethodUse::NewIssuance,
                    &FieldHex::from_field(&Fr::from(42u64)),
                    proof,
                    &scratch(&format!("{tag}_value{i}")),
                    tag,
                )
                .unwrap();
            m.sub_proofs[i + 1].proof_hex = encode_artifacts(&art);
            art
        })
        .collect()
}

pub(super) fn honest(datatype: &str, value: &str, bound: &str, tag: &str) {
    let (mut m, scan, proofs) = fixture(&[literal(value, datatype)], &literal(bound, datatype));
    let mut artifacts = prove(&mut m, &scan, &proofs, tag);
    super::v2_members::check(&m, tag)
        .expect("honest V2 value FILTER verifies through verify_manifest");
    let art = &mut artifacts[0];
    let mid = art.proof.len() / 2;
    art.proof[mid] ^= 0xff;
    m.sub_proofs[1].proof_hex = encode_artifacts(art);
    assert!(
        matches!(
            super::v2_members::check(&m, &format!("{tag}_bad")),
            Err(CheckError::ProofRejected { proof: 1 })
        ),
        "every value member reaches bb verification"
    );
}

// These use the live fixture unchanged, including its signed V2 commitment,
// typed builders, query and explicit pattern_scans. Proof bytes are placeholders:
// passing the prefilter establishes structure only, never proof verification.
fn structure(m: &ProofManifest) -> Result<(), CheckError> {
    prefilter_manifest_structure(m, &trusted_k(&test_issuer_sk(1)), &fresh_policy())
        .map(|_| ())
}

#[test]
fn live_positive_fixtures_pass_structure() {
    for (datatype, value, bound) in [
        ("integer", "25", "18"),
        ("decimal", "2.50", "1.5"),
        ("dateTime", "2024-01-02T03:04:05Z", "2024-01-01T00:00:00Z"),
        ("double", "2.5E0", "1.5E0"),
        ("boolean", "true", "true"),
    ] {
        let (mut m, _, _) = fixture(&[literal(value, datatype)], &literal(bound, datatype));
        // Nonempty placeholders also ensure this test never asks a proof driver
        // to create artifacts merely to reach the structural check.
        for sp in &mut m.sub_proofs {
            sp.proof_hex = "00".into();
        }
        structure(&m).unwrap_or_else(|e| panic!("live {datatype} fixture: {e:?}"));
    }
}

#[test]
fn live_wrong_row_reaches_binding_check() {
    let (mut m, _, _) = fixture(
        &[literal("25", "integer"), literal("26", "integer")],
        &literal("18", "integer"),
    );
    structure(&m).expect("live wrong-row control");
    m.binding_edges[0].from_row = 1;
    assert!(matches!(structure(&m), Err(CheckError::BindingInconsistent { edge: 0 })));
}

#[test]
fn live_false_verdict_reaches_pin_check() {
    let value = literal("25", "integer");
    let (mut m, _, _) = fixture(std::slice::from_ref(&value), &literal("18", "integer"));
    structure(&m).expect("live false-verdict control");
    let (inputs, _) = built(&value, &literal("99", "integer"));
    assert!(matches!(inputs, ProofInputs::FilterValueDl { expected: false, .. }));
    m.sub_proofs[1].inputs = inputs;
    m.query = "SELECT ?s WHERE { ?s <http://ex/p> ?o FILTER(?o >= 99) }".into();
    assert!(matches!(structure(&m), Err(CheckError::ValueObligation(
        sparq_zk_compose::value_obligation::ObligationError::PinMismatch { proof: 1, ref reason }
    )) if reason == &sparq_zk_compose::value_pin::PinError::VerdictNotTrue.to_string()));
}

#[test]
fn live_different_constant_reaches_pin_check() {
    let value = literal("25", "integer");
    let (mut m, _, _) = fixture(std::slice::from_ref(&value), &literal("18", "integer"));
    structure(&m).expect("live different-constant control");
    let (inputs, _) = built(&value, &literal("17", "integer"));
    assert!(matches!(inputs, ProofInputs::FilterValueDl { expected: true, .. }));
    m.sub_proofs[1].inputs = inputs;
    assert!(matches!(structure(&m), Err(CheckError::ValueObligation(
        sparq_zk_compose::value_obligation::ObligationError::PinMismatch { proof: 1, ref reason }
    )) if reason == &sparq_zk_compose::value_pin::PinError::BoundMismatch.to_string()));
}

#[test]
fn live_crypto_setup_passes_structure() {
    let (m, _, _) = fixture(&[literal("25", "integer")], &literal("18", "integer"));
    structure(&m).expect("live crypto control");
    assert!(matches!(super::v2_members::check(&m, "value_crypto_structure"),
        Err(CheckError::MissingProof { proof: 0 })),
        "full verification must reach the proof stage without artifacts");
}

#[test]
fn mixed_string_integer_and_live_decimal_pass_structure() {
    let bound = literal("1.5", "decimal");
    let (mut m, _, _) = fixture(&[literal("2.50", "decimal")], &bound);
    // Keep the V2 scan/value pair at 0/1 and append the existing genuine
    // string-canonical scan/filter pair at 2/3. Separate variables avoid joins.
    let integer = sample_manifest();
    m.sub_proofs.extend(integer.sub_proofs);
    m.commitment_attestations.extend(integer.commitment_attestations);
    m.query = format!("SELECT ?s ?x WHERE {{ ?x <http://ex/age> ?age . \
        ?s <http://ex/p> ?o FILTER(?age >= 18 && ?o >= {bound}) }}");
    m.pattern_scans = vec![vec![2], vec![0]];
    m.attributions = vec![vec![0], vec![0]];
    m.binding_edges[0].filter = Some(1);
    m.binding_edges.push(BindingEdge {
        from_proof: 2, from_row: 0, from_slot: 2, to_proof: 3, filter: None,
    });
    structure(&m).expect("mixed integer-string/decimal-value control");

    // Retain both scans and the integer filter, but omit the decimal conjunct's
    // proof and edge. Remap indices after removing sub-proof 1.
    m.sub_proofs.remove(1);
    m.binding_edges.remove(0);
    m.binding_edges[0].from_proof = 1;
    m.binding_edges[0].to_proof = 2;
    m.pattern_scans[0] = vec![1];
    assert!(matches!(structure(&m), Err(CheckError::UnboundFilter { .. })),
        "the integer string edge cannot cover the decimal conjunct");
}
