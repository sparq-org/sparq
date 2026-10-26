//! Genuine V2 component witnesses and method/member dispatch controls.
use super::*;
use serde::Serialize;
use sparq_zk::field::field_from_hash_bytes;

pub(super) struct Graph {
    commitment: FieldHex,
    method_commitment: sparq_zk::sig::MethodCommitment,
    salt: Fr,
    enc: [FieldHex; 3],
    identities: Vec<[FieldHex; 3]>,
}

impl Graph {
    fn new(v2: bool, predicate: &str, salt: u64) -> Self {
        let salt = Fr::from(salt);
        let terms = ["http://ex/s", predicate, "http://ex/o"].map(|s| Term::NamedNode(iri(s)));
        let triple = Triple::new(iri("http://ex/s"), iri(predicate), iri("http://ex/o"));
        let method_commitment = if v2 {
            sparq_zk::dual_leaf_v2::commit_triples(&[triple], salt).unwrap().method_commitment()
        } else {
            commit_triples(&[triple], salt).unwrap().method_commitment()
        };
        let enc = terms.map(|t| FieldHex::from_field(&if v2 {
            sparq_zk::dual_leaf_v2::encode_term(&t, &salt).unwrap().unwrap()
        } else {
            sparq_zk::encode::encode_term(&t, &salt).unwrap()
        }));
        let mut identities = vec![zero_row(); 16];
        identities[0] = ["http://ex/s", predicate, "http://ex/o"].map(|s|
            FieldHex::from_field(&field_from_hash_bytes(blake3::hash(s.as_bytes()).as_bytes())));
        Self { commitment: FieldHex::from_field(&method_commitment.commitment),
            method_commitment, salt, enc, identities }
    }

    fn scan(&self, v2: bool) -> ProofInputs {
        ProofInputs::Scan {
            id: if v2 { CircuitId::scan_v2_dl(1, 16, 4).unwrap() }
                else { CircuitId::Scan { k: 1, n: 16, r: 4 } },
            commitments: vec![self.commitment.clone()],
            pattern_is_const: [false, true, false],
            pattern_const_enc: [fh0(), self.enc[1].clone(), fh0()],
            rows: vec![self.enc.clone()], row_count: 1, attribution: vec![true],
        }
    }

    fn components(&self, out: &mut String, suffix: &str, scan: bool) {
        let mut tc = vec![zero_row(); 16];
        let mut vs = tc.clone();
        let mut lc = tc.clone();
        tc[0] = std::array::from_fn(|_| FieldHex("0x1".into()));
        vs[0] = std::array::from_fn(|_| FieldHex::from_field(&sparq_zk::dual_leaf_v2::value_none()));
        lc[0] = std::array::from_fn(|_| FieldHex("0x1".into()));
        let zeros = vec![zero_row(); 16];
        let flags = vec![[false; 3]; 16];
        for (key, rows) in [("type_codes", &tc), ("value_slot0", &vs),
            ("datatype_consts", &zeros), ("lang_consts", &lc), ("identities", &self.identities)] {
            if scan { put(out, &format!("{key}{suffix}"), &[rows]); }
            else { put(out, &format!("{key}{suffix}"), rows); }
        }
        for key in ["has_value", "has_lang"] {
            if scan { put(out, &format!("{key}{suffix}"), &[&flags]); }
            else { put(out, &format!("{key}{suffix}"), &flags); }
        }
    }
}

fn fh0() -> FieldHex { FieldHex("0x0".into()) }
fn zero_row() -> [FieldHex; 3] { std::array::from_fn(|_| fh0()) }
fn put(out: &mut String, key: &str, value: &impl Serialize) {
    out.push_str(&format!("{key} = {}\n", serde_json::to_string(value).unwrap()));
}

pub(super) fn scan_toml(g: &Graph, inputs: &ProofInputs, components: bool) -> String {
    let ProofInputs::Scan { commitments, pattern_is_const, pattern_const_enc, rows,
        row_count, attribution, .. } = inputs else { unreachable!() };
    let mut out = String::new();
    put(&mut out, "challenge", &FieldHex("0x2a".into()));
    put(&mut out, "commitments", commitments);
    put(&mut out, "pattern_is_const", pattern_is_const);
    put(&mut out, "pattern_const_enc", pattern_const_enc);
    let mut padded = rows.clone(); padded.resize(4, zero_row());
    put(&mut out, "rows", &padded);
    put(&mut out, "row_count", row_count);
    put(&mut out, "attribution", attribution);
    put(&mut out, "counts", &[1u32]);
    if components { g.components(&mut out, "", true); }
    else {
        let mut enc = vec![zero_row(); 16]; enc[0] = g.enc.clone();
        put(&mut out, "enc", &[enc]);
    }
    out
}

fn join_toml(a: &Graph, b: &Graph, inputs: &ProofInputs) -> String {
    let ProofInputs::JoinEq { commit_a, commit_b, join_commitment, slot_a, slot_b, .. } = inputs
        else { unreachable!() };
    let mut out = String::new();
    put(&mut out, "challenge", &FieldHex("0x2a".into()));
    put(&mut out, "commit_a", commit_a); put(&mut out, "commit_b", commit_b);
    put(&mut out, "join_commitment", join_commitment);
    put(&mut out, "slot_a", slot_a); put(&mut out, "slot_b", slot_b);
    put(&mut out, "counts_a", &1u32); put(&mut out, "counts_b", &1u32);
    a.components(&mut out, "_a", false); b.components(&mut out, "_b", false);
    put(&mut out, "row_a", &a.enc); put(&mut out, "row_b", &b.enc);
    put(&mut out, "blinding", &FieldHex("0x2a".into()));
    out
}

pub(super) fn fixture(v2: bool, join: bool) -> (ProofManifest, Vec<Graph>) {
    let mut graphs = vec![Graph::new(v2, "http://ex/p", 7)];
    if join { graphs.push(Graph::new(v2, "http://ex/q", 8)); }
    let mut m = sample_manifest();
    m.query = if join { "SELECT ?s WHERE { ?s <http://ex/p> ?o . ?s <http://ex/q> ?v }" }
        else { "SELECT ?s WHERE { ?s <http://ex/p> ?o }" }.into();
    m.sub_proofs = graphs.iter().map(|g| SubProof { inputs: g.scan(v2), proof_hex: String::new() }).collect();
    m.binding_edges.clear(); m.commitment_attestations.clear();
    m.attributions = vec![vec![0]; graphs.len()];
    m.pattern_scans = (0..graphs.len()).map(|i| vec![i]).collect();
    if join {
        m.join_obligations = vec![("s".into(), 0, 1)];
        m.join_edges = vec![sparq_zk_compose::manifest::JoinEdge {
            scan_a: 0, graph_a: 0, scan_b: 1, graph_b: 0, join_proof: 2,
        }];
        m.sub_proofs.push(SubProof { inputs: ProofInputs::JoinEq {
            id: if v2 { CircuitId::join_eq_v2_dl(16, 16).unwrap() }
                else { CircuitId::JoinEq { n_a: 16, n_b: 16 } },
            commit_a: graphs[0].commitment.clone(), commit_b: graphs[1].commitment.clone(),
            join_commitment: FieldHex::from_field(&sparq_zk::sig::join_value_commitment(
                &graphs[0].enc[0].to_field().unwrap(), &Fr::from(42u64))),
            slot_a: 0, slot_b: 0,
        }, proof_hex: String::new() });
    }
    for g in &graphs {
        let mut att = attest_with_salt(g.commitment.to_field().unwrap(), g.salt, &test_issuer_sk(1));
        let status = sparq_zk::sig::status_ref_digest(
            &sparq_zk::sig::status_list_id_to_field(FIXTURE_STATUS_LIST), FIXTURE_STATUS_INDEX, FIXTURE_STATUS_VERSION);
        att.signature = test_issuer_sk(1).attest(&g.method_commitment, &g.salt, &status, None);
        m.commitment_attestations.push(att);
    }
    (m, graphs)
}

pub(super) fn check(m: &ProofManifest, tag: &str) -> Result<(), CheckError> {
    verify_manifest(m, &CircuitProver::from_crate_root(), &scratch(tag),
        &trusted_k(&test_issuer_sk(1)), &fresh_policy(), &HolderRegistry::empty(),
        &HolderBindingPolicy::allow_bearer(), &EntailmentPolicy::simple_only(),
        &nonce_for("0x2a"), &InMemorySeenNonces::new())
}

#[test]
fn v2_scan_pairings_are_checked_through_verify_manifest() {
    for v2 in [false, true] {
        let (mut m, _) = fixture(v2, false);
        assert!(matches!(check(&m, "v2_scan_control"), Err(CheckError::MissingProof { proof: 0 })));
        if let ProofInputs::Scan { id, .. } = &mut m.sub_proofs[0].inputs {
            *id = if v2 { CircuitId::Scan { k: 1, n: 16, r: 4 } }
                else { CircuitId::scan_v2_dl(1, 16, 4).unwrap() };
        }
        assert!(matches!(check(&m, "v2_scan_pair"), Err(CheckError::MemberNotLegalForMethod { .. })),
            "scan method/member mismatch must fail before proof bytes");
    }
}

#[test]
fn v2_join_pairings_are_checked_through_verify_manifest() {
    for v2 in [false, true] {
        let (mut m, _) = fixture(v2, true);
        assert!(matches!(check(&m, "v2_join_control"), Err(CheckError::MissingProof { proof: 0 })));
        if let ProofInputs::JoinEq { id, .. } = &mut m.sub_proofs[2].inputs {
            *id = if v2 { CircuitId::JoinEq { n_a: 16, n_b: 16 } }
                else { CircuitId::join_eq_v2_dl(16, 16).unwrap() };
        }
        assert!(matches!(check(&m, "v2_join_pair"), Err(CheckError::MemberNotLegalForMethod { .. })),
            "join method/member mismatch must fail before proof bytes");
    }
}

#[test]
fn uncompiled_v2_shapes_are_refused_through_verify_manifest() {
    for join in [false, true] {
        let (mut m, _) = fixture(true, join);
        assert!(matches!(check(&m, "v2_shape_control"), Err(CheckError::MissingProof { .. })));
        assert!(m.to_json().is_ok(), "compiled V2 manifest serializes");
        let index = if join { 2 } else { 0 };
        match &mut m.sub_proofs[index].inputs {
            ProofInputs::Scan { id, .. } => *id = CircuitId::ScanV2Dl { k: 2, n: 16, r: 4 },
            ProofInputs::JoinEq { id, .. } => *id = CircuitId::JoinEqV2Dl { n_a: 8, n_b: 16 },
            _ => unreachable!(),
        }
        assert!(m.to_json().unwrap_err().to_string().contains("UncompiledV2Member"));
        #[cfg(feature = "extended-fragment")]
        {
            let fragment = sparq_zk_compose::manifest::FragmentManifest::new(m.clone(), vec![]);
            assert!(fragment.to_json().unwrap_err().to_string().contains("UncompiledV2Member"));
        }
        assert!(matches!(check(&m, "v2_shape"), Err(CheckError::CircuitIdMismatch { .. })),
            "uncompiled V2 declaration must fail derivation");
    }
}

#[test]
fn v2_opaque_renderer_requires_component_witnesses() {
    use sparq_zk_compose::toml::ProverTomlError;
    let challenge = FieldHex("0x2a".into());
    for join in [false, true] {
        let (m, _) = fixture(true, join);
        let index = if join { 2 } else { 0 };
        assert_eq!(prover_toml_for(&m.sub_proofs[index].inputs, &challenge,
            &[], &[], &[], None, None), Err(ProverTomlError::V2ComponentsRequired));
    }
    let (control, graphs) = fixture(false, false);
    assert!(prover_toml_for(&control.sub_proofs[0].inputs, &challenge,
        &[1], &[vec![graphs[0].enc.clone()]], &[], None, None).is_ok());
}

pub(super) fn prove_all(m: &mut ProofManifest, graphs: &[Graph], join: bool, tag: &str) {
    let p = CircuitProver::from_crate_root();
    for (i, g) in graphs.iter().enumerate() {
        let inputs = &m.sub_proofs[i].inputs;
        let art = p.prove_in(inputs.circuit_id(), &scan_toml(g, inputs, true),
            &scratch(&format!("{tag}_scan_{i}")), &format!("{tag}_scan_{i}")).unwrap();
        m.sub_proofs[i].proof_hex = encode_artifacts(&art);
    }
    if join {
        let inputs = &m.sub_proofs[2].inputs;
        let art = p.prove_in(inputs.circuit_id(), &join_toml(&graphs[0], &graphs[1], inputs),
            &scratch(&format!("{tag}_join")), &format!("{tag}_join")).unwrap();
        m.sub_proofs[2].proof_hex = encode_artifacts(&art);
    }
}

pub(super) fn cross_vk(scan: bool) {
    let (m, graphs) = fixture(true, !scan);
    let p = CircuitProver::from_crate_root();
    let index = if scan { 0 } else { 2 };
    let inputs = &m.sub_proofs[index].inputs;
    let toml = if scan { scan_toml(&graphs[0], inputs, true) }
        else { join_toml(&graphs[0], &graphs[1], inputs) };
    let tag = if scan { "v2_cross_scan" } else { "v2_cross_join" };
    let art = p.prove_in(inputs.circuit_id(), &toml, &scratch(tag), tag).unwrap();
    let honest_vk = p.canonical_vk(inputs.circuit_id(), &scratch(&format!("{tag}_honest_vk"))).unwrap();
    assert!(p.verify_with(&art.proof, &art.public_inputs, &honest_vk,
        &scratch(&format!("{tag}_control"))).unwrap(), "honest V2 VK verifies");
    let opaque = if scan { CircuitId::Scan { k: 1, n: 16, r: 4 } }
        else { CircuitId::JoinEq { n_a: 16, n_b: 16 } };
    let wrong_vk = p.canonical_vk(&opaque, &scratch(&format!("{tag}_wrong_vk"))).unwrap();
    // Direct driver call deliberately bypasses method/member structural dispatch.
    assert!(!p.verify_with(&art.proof, &art.public_inputs, &wrong_vk,
        &scratch(&format!("{tag}_wrong"))).unwrap(), "bb must refuse the cross-member VK");
}
