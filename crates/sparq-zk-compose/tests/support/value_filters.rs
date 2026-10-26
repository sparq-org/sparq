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
        pattern_const_enc: [zero()[0].clone(), rows[0][1].clone(), zero()[0].clone()],
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
