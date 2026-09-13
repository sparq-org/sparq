//! [GPT-6] Actual native BBS+ proof controls; no mock signature verifier.
use super::*;
use ark_std::rand::{rngs::StdRng, SeedableRng};

fn fixture(two: bool) -> (StdRng, Request, Vec<Credential>) {
    let mut rng = StdRng::seed_from_u64(91);
    let issuer = Issuer::generate(&mut rng, "urn:issuer:name").unwrap();
    let reference = StatusReference {
        list: "urn:status:names".into(),
        epoch: 7,
        index: 9,
    };
    let credential = issue_rdf(
        &mut rng,
        &issuer,
        "<urn:alice> <urn:name> \"Alice\" .\n<urn:private> <urn:p> \"unreleased-secret\" .",
        &reference,
    )
    .unwrap();
    let mut request = Request {
        query: "SELECT DISTINCT ?name WHERE { <urn:alice> <urn:name> ?name }".into(),
        rows: vec![BTreeMap::from([("name".into(), "\"Alice\"".into())])],
        roles: vec![RolePolicy {
            issuer: issuer.public(),
            status: AcceptedStatus {
                reference,
                bits: vec![0; 2],
            },
        }],
        nonce: [7; 32],
    };
    let mut credentials = vec![credential];
    if two {
        let issuer = Issuer::generate(&mut rng, "urn:issuer:age").unwrap();
        let reference = StatusReference {
            list: "urn:status:ages".into(),
            epoch: 8,
            index: 3,
        };
        credentials.push(
            issue_rdf(
                &mut rng,
                &issuer,
                "<urn:alice> <urn:age> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                &reference,
            )
            .unwrap(),
        );
        request.query = "SELECT DISTINCT ?name ?age WHERE { <urn:alice> <urn:name> ?name . <urn:alice> <urn:age> ?age }".into();
        request.rows[0].insert(
            "age".into(),
            "\"021\"^^<http://www.w3.org/2001/XMLSchema#integer>".into(),
        );
        request.roles.push(RolePolicy {
            issuer: issuer.public(),
            status: AcceptedStatus {
                reference,
                bits: vec![0],
            },
        });
    }
    (rng, request, credentials)
}

fn check(rng: &mut StdRng, request: &Request, proof: &Presentation) -> Result<()> {
    verify_public_bgp(rng, request, proof, &mut ConsumedNonces::default())
}

#[test]
fn independent_key_import_is_bounded_and_rejects_infinity() {
    let (mut rng, mut request, credentials) = fixture(false);
    let bytes = request.roles[0].issuer.to_bytes().unwrap();
    request.roles[0].issuer = TrustedIssuer::from_bytes("urn:issuer:name", &bytes).unwrap();
    let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    assert_eq!(check(&mut rng, &request, &proof), Ok(()));
    for bad in [
        vec![],
        [bytes.as_slice(), &[0]].concat(),
        vec![0xff; PUBLIC_KEY_BYTES],
    ] {
        assert!(matches!(
            TrustedIssuer::from_bytes("urn:issuer:name", &bad),
            Err(Error::Decoding)
        ));
    }
    let zero = PublicKeyG2::<Bls12_381>(Default::default());
    let mut zero_bytes = Vec::new();
    zero.serialize_compressed(&mut zero_bytes).unwrap();
    assert!(matches!(
        TrustedIssuer::from_bytes("urn:issuer:name", &zero_bytes),
        Err(Error::Decoding)
    ));
}

#[test]
fn actual_single_and_two_issuer_support_then_replay() {
    for two in [false, true] {
        let (mut rng, request, credentials) = fixture(two);
        let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
        let mut consumed = ConsumedNonces::default();
        assert_eq!(
            verify_public_bgp(&mut rng, &request, &proof, &mut consumed),
            Ok(())
        );
        assert_eq!(
            verify_public_bgp(&mut rng, &request, &proof, &mut consumed),
            Err(Error::Replay)
        );
    }
}

#[test]
fn full_capacity_vector_authenticates_multiple_rows_with_public_predicates() {
    let mut rng = StdRng::seed_from_u64(190);
    let issuer = Issuer::generate(&mut rng, "urn:issuer:full").unwrap();
    let status = StatusReference {
        list: "urn:status:full".into(),
        epoch: 1,
        index: 0,
    };
    let document = (0..TRIPLE_SLOTS)
        .rev()
        .map(|i| format!("<urn:s:{i:02}> <urn:p> \"value-{i:02}\" .\n"))
        .collect::<String>();
    let credential = issue_rdf(&mut rng, &issuer, &document, &status).unwrap();
    let request = Request {
        query: "SELECT DISTINCT ?s ?p ?o WHERE { ?s ?p ?o }".into(),
        rows: [14, 15]
            .into_iter()
            .map(|i| {
                BTreeMap::from([
                    ("s".into(), format!("<urn:s:{i:02}>")),
                    ("p".into(), "<urn:p>".into()),
                    ("o".into(), format!("\"value-{i:02}\"")),
                ])
            })
            .collect(),
        roles: vec![RolePolicy {
            issuer: issuer.public(),
            status: AcceptedStatus {
                reference: status,
                bits: vec![0],
            },
        }],
        nonce: [8; 32],
    };
    let proof = prove_public_bgp(&mut rng, &request, &[credential]).unwrap();
    assert_eq!(
        proof.support.rows,
        vec![
            vec![Slot {
                role: 0,
                triple: 14
            }],
            vec![Slot {
                role: 0,
                triple: 15
            }]
        ]
    );
    assert_eq!(check(&mut rng, &request, &proof), Ok(()));
    let mut changed = request.clone();
    changed.rows.swap(0, 1);
    assert_eq!(check(&mut rng, &changed, &proof), Err(Error::Verification));
}

#[test]
fn exact_query_terms_result_and_issuer_are_verifier_owned() {
    let (mut rng, request, credentials) = fixture(true);
    let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    let mut variants = Vec::new();
    for (old, new) in [
        ("urn:alice", "urn:mallory"),
        ("urn:name", "urn:names"),
        ("SELECT", " SELECT"),
    ] {
        let mut other = request.clone();
        other.query = other.query.replace(old, new);
        variants.push(other);
    }
    for value in [
        "\"21\"^^<http://www.w3.org/2001/XMLSchema#integer>",
        "\"021\"",
        "\"021\"^^<urn:custom:type>",
        "<urn:021>",
    ] {
        let mut other = request.clone();
        other.rows[0].insert("age".into(), value.into());
        variants.push(other);
    }
    let mut other = request.clone();
    other.nonce[0] ^= 1;
    variants.push(other);
    let mut other = request.clone();
    other.roles[0].issuer = Issuer::generate(&mut rng, "urn:issuer:name")
        .unwrap()
        .public();
    variants.push(other);
    let mut other = request.clone();
    other.roles[0].issuer.id = "urn:issuer:renamed".into();
    variants.push(other);
    for other in variants {
        assert_eq!(check(&mut rng, &other, &proof), Err(Error::Verification));
    }
    assert_eq!(check(&mut rng, &request, &proof), Ok(()));
}

#[test]
fn status_reference_snapshot_and_revocation_are_independently_checked() {
    let (mut rng, request, credentials) = fixture(false);
    let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    let mut other = request.clone();
    other.roles[0].status.bits[1] = 2;
    assert_eq!(check(&mut rng, &other, &proof), Err(Error::Status));
    for field in 0..4 {
        let mut other = request.clone();
        match field {
            0 => other.roles[0].status.reference.list = "urn:status:substitute".into(),
            1 => other.roles[0].status.reference.epoch += 1,
            2 => other.roles[0].status.reference.index -= 1,
            _ => other.roles[0].status.bits[0] = 1, // Different accepted snapshot, target still active.
        }
        assert_eq!(check(&mut rng, &other, &proof), Err(Error::Verification));
    }
    let mut other = request.clone();
    other.roles[0].status.reference.index = 16;
    assert_eq!(check(&mut rng, &other, &proof), Err(Error::Status));
    let mut other = request.clone();
    other.roles[0].status.bits.clear();
    assert_eq!(check(&mut rng, &other, &proof), Err(Error::Capacity));
    assert_eq!(check(&mut rng, &request, &proof), Ok(()));
}

#[test]
fn valid_weaker_same_nonce_proof_cannot_omit_a_required_disclosure() {
    let (mut rng, request, credentials) = fixture(false);
    let honest = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    let (mut disclosed, context) = relation(&request, &honest.support).unwrap();
    let removed = honest.support.rows[0][0].triple + 2;
    disclosed[0].remove(&removed);
    let mut prover = Statements::new();
    let mut verifier = Statements::new();
    let mut witnesses = Witnesses::new();
    for ((role, credential), public) in request.roles.iter().zip(&credentials).zip(&disclosed) {
        prover.add(PoKBBSSignatureG1Prover::new_statement_from_params(
            parameters(),
            public.clone(),
        ));
        verifier.add(PoKBBSSignatureG1Verifier::new_statement_from_params(
            parameters(),
            role.issuer.key.clone(),
            public.clone(),
        ));
        witnesses.add(PoKBBSSignatureG1::new_as_witness(
            credential.signature.clone(),
            credential
                .messages
                .iter()
                .enumerate()
                .filter(|(i, _)| !public.contains_key(i))
                .map(|(i, m)| (i, *m))
                .collect(),
        ));
    }
    let weak_prover = ProofSpec::new(prover, MetaStatements::new(), vec![], Some(context.clone()));
    let weak_verifier = ProofSpec::new(verifier, MetaStatements::new(), vec![], Some(context));
    let (weak, _) = Proof::new::<StdRng, Blake2b512>(
        &mut rng,
        weak_prover,
        witnesses,
        Some(request.nonce.to_vec()),
        Default::default(),
    )
    .unwrap();
    // It is a genuine valid proof under its deliberately weaker specification.
    assert!(weak
        .clone()
        .verify::<StdRng, Blake2b512>(
            &mut rng,
            weak_verifier,
            Some(request.nonce.to_vec()),
            Default::default()
        )
        .is_ok());
    let malicious = Presentation {
        support: honest.support,
        proof: encode(weak).unwrap(),
    };
    assert_eq!(
        check(&mut rng, &request, &malicious),
        Err(Error::Verification)
    );
}

#[test]
fn valid_wrong_preimage_proof_with_expected_context_is_rejected() {
    let mut rng = StdRng::seed_from_u64(192);
    let issuer = Issuer::generate(&mut rng, "urn:issuer:preimage").unwrap();
    let status = StatusReference {
        list: "urn:status:expected".into(),
        epoch: 9,
        index: 2,
    };
    let query = "SELECT DISTINCT ?value WHERE { <urn:alice> <urn:value> ?value }";
    let expected = "\"021\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let request = Request {
        query: query.into(),
        rows: vec![BTreeMap::from([("value".into(), expected.into())])],
        roles: vec![RolePolicy {
            issuer: issuer.public(),
            status: AcceptedStatus {
                reference: status.clone(),
                bits: vec![0],
            },
        }],
        nonce: [29; 32],
    };
    let support = Support {
        rows: vec![vec![Slot { role: 0, triple: 0 }]],
    };
    let (_, context) = relation(&request, &support).unwrap();
    for variant in 0..7 {
        let mut reference = status.clone();
        let (subject, predicate, object) = match variant {
            0 => ("urn:mallory", "urn:value", expected),
            1 => ("urn:alice", "urn:other", expected),
            2 => (
                "urn:alice",
                "urn:value",
                "\"21\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            ),
            3 => ("urn:alice", "urn:value", "\"021\""),
            _ => ("urn:alice", "urn:value", expected),
        };
        match variant {
            4 => reference.list = "urn:status:wrong".into(),
            5 => reference.epoch -= 1,
            6 => reference.index += 1,
            _ => {}
        }
        let credential = issue_rdf(
            &mut rng,
            &issuer,
            &format!("<{subject}> <{predicate}> {object} ."),
            &reference,
        )
        .unwrap();
        let actual_public = BTreeMap::from([
            (0, credential.messages[0]),
            (1, credential.messages[1]),
            (2, credential.messages[2]),
        ]);
        let mut prover = Statements::new();
        prover.add(PoKBBSSignatureG1Prover::new_statement_from_params(
            parameters(),
            actual_public.clone(),
        ));
        let mut verifier = Statements::new();
        verifier.add(PoKBBSSignatureG1Verifier::new_statement_from_params(
            parameters(),
            issuer.public().key,
            actual_public.clone(),
        ));
        let mut witnesses = Witnesses::new();
        witnesses.add(PoKBBSSignatureG1::new_as_witness(
            credential.signature,
            credential
                .messages
                .iter()
                .enumerate()
                .filter(|(i, _)| !actual_public.contains_key(i))
                .map(|(i, m)| (i, *m))
                .collect(),
        ));
        let (proof, _) = Proof::new::<StdRng, Blake2b512>(
            &mut rng,
            ProofSpec::new(prover, MetaStatements::new(), vec![], Some(context.clone())),
            witnesses,
            Some(request.nonce.to_vec()),
            Default::default(),
        )
        .unwrap();
        // Same expected query/result/context/nonce, genuinely signed different
        // preimage or status. A context-only check cannot distinguish this.
        assert!(proof
            .clone()
            .verify::<StdRng, Blake2b512>(
                &mut rng,
                ProofSpec::new(
                    verifier,
                    MetaStatements::new(),
                    vec![],
                    Some(context.clone())
                ),
                Some(request.nonce.to_vec()),
                Default::default()
            )
            .is_ok());
        let presentation = Presentation {
            support: support.clone(),
            proof: encode(proof).unwrap(),
        };
        assert_eq!(
            check(&mut rng, &request, &presentation),
            Err(Error::Verification),
            "variant {variant}"
        );
    }
}

#[test]
fn support_slots_cannot_change_or_omit_required_preimages() {
    let (mut rng, request, credentials) = fixture(true);
    let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    let mut other = proof.clone();
    other.support.rows[0].pop();
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Support));
    let mut other = proof.clone();
    other.support.rows[0][0].triple = TRIPLE_SLOTS;
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Support));
    let mut other = proof.clone();
    other.support.rows[0][0].role = MAX_ROLES;
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Support));
    let mut other = proof.clone();
    other.support.rows[0][0].triple += 1;
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Verification));
    let mut other = proof.clone();
    other.support.rows[0].swap(0, 1);
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Verification));
}

#[test]
fn bounded_bbs_only_wire_rejects_malformed_and_trailing_bytes() {
    let (mut rng, request, credentials) = fixture(false);
    let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    for bytes in [
        vec![],
        vec![0; 9],
        proof.proof[..proof.proof.len() - 1].to_vec(),
        [proof.proof.as_slice(), &[0]].concat(),
    ] {
        assert_eq!(
            check(
                &mut rng,
                &request,
                &Presentation {
                    support: proof.support.clone(),
                    proof: bytes
                }
            ),
            Err(Error::Decoding)
        );
    }
    let mut other = proof.clone();
    other.proof[8] = 255;
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Decoding));
    let mut other = proof.clone();
    other.proof[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(check(&mut rng, &request, &other), Err(Error::Decoding));
}

#[test]
fn hidden_variables_and_non_bgp_algebra_fail_closed() {
    let (_, mut request, _) = fixture(false);
    for query in [
        "SELECT ?name WHERE { <urn:alice> <urn:name> ?name }",
        "SELECT DISTINCT ?name WHERE { ?s <urn:name> ?name }",
        "SELECT DISTINCT ?name WHERE { <urn:alice> <urn:name> ?name FILTER(?name = \"Alice\") }",
        "SELECT DISTINCT ?name WHERE { <urn:alice> <urn:name> ?name } LIMIT 1",
        "SELECT DISTINCT ?name FROM <urn:g> WHERE { <urn:alice> <urn:name> ?name }",
        "SELECT DISTINCT ?name WHERE { _:b <urn:name> ?name }",
        "SELECT DISTINCT ?name WHERE { <urn:alice> <urn:p>/<urn:name> ?name }",
        "ASK { <urn:alice> <urn:name> ?name }",
    ] {
        request.query = query.into();
        assert_eq!(
            required_lines(&request),
            Err(Error::QueryProfile),
            "{query}"
        );
    }
}

#[test]
fn literal_subject_or_predicate_and_injected_terms_are_not_rdf_support() {
    let (_, mut request, _) = fixture(false);
    for (query, row) in [
        (
            "SELECT DISTINCT ?s WHERE { ?s <urn:p> <urn:o> }",
            BTreeMap::from([("s".into(), "\"literal\"".into())]),
        ),
        (
            "SELECT DISTINCT ?p WHERE { <urn:s> ?p <urn:o> }",
            BTreeMap::from([("p".into(), "\"literal\"".into())]),
        ),
    ] {
        request.query = query.into();
        request.rows = vec![row];
        assert_eq!(required_lines(&request), Err(Error::RdfProfile));
    }
    for value in [
        "_:b",
        "\"v\" . <urn:s> <urn:p> <urn:o>",
        "\"v\" <urn:g>",
        "<relative>",
        "\"v\"@en--bad",
        "\"v\"@en--ltr",
    ] {
        assert!(
            matches!(mapped_term(value), Err(Error::RdfProfile)),
            "{value}"
        );
    }
}

#[test]
fn canonical_graph_set_and_lexical_identity_are_not_value_normalization() {
    // The API admits canonical term spellings; xsd:string uses the simple
    // literal spelling. This is an RDF-profile rejection, not a crypto failure.
    assert_eq!(
        mapped_term("\"021\"^^<http://www.w3.org/2001/XMLSchema#string>"),
        Err(Error::RdfProfile)
    );
    let a = "<urn:s> <urn:p> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
    let lines = canonical_lines(a).unwrap();
    assert!(lines[0].contains("\"021\""));
    assert_eq!(lines, canonical_lines(&format!("{a}\n{a}")).unwrap());
    assert_ne!(lines, canonical_lines(&a.replace("021", "21")).unwrap());
    assert_ne!(
        canonical_lines("<urn:s> <urn:p> \"x\"@en .").unwrap(),
        canonical_lines("<urn:s> <urn:p> \"x\"@fr .").unwrap()
    );
    for text in [
        "_:b <urn:p> <urn:o> .",
        "<urn:s> <urn:p> _:b .",
        "<urn:s> <urn:p> <urn:o> <urn:g> .",
        "<urn:s> <urn:p> <<( <urn:a> <urn:b> <urn:c> )>> .",
    ] {
        assert_eq!(canonical_lines(text), Err(Error::RdfProfile));
    }
}

#[test]
fn capacity_and_distinct_rows_are_not_silently_truncated() {
    let text = (0..=TRIPLE_SLOTS)
        .map(|i| format!("<urn:s:{i}> <urn:p> <urn:o> .\n"))
        .collect::<String>();
    assert_eq!(canonical_lines(&text), Err(Error::Capacity));
    let first16 = text
        .lines()
        .take(TRIPLE_SLOTS)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(canonical_lines(&first16).unwrap().len(), TRIPLE_SLOTS);
    let (_, mut request, _) = fixture(false);
    request.rows.push(request.rows[0].clone());
    assert_eq!(required_lines(&request), Err(Error::Support));
    request.rows = vec![request.rows[0].clone(); MAX_ROWS + 1];
    assert_eq!(required_lines(&request), Err(Error::Capacity));
    assert_eq!(
        mapped_term(&format!("\"{}\"", "x".repeat(MAX_TERM_BYTES))),
        Err(Error::Capacity)
    );
}

#[test]
fn failed_verification_does_not_consume_challenge_and_debug_redacts_credentials() {
    let (mut rng, request, credentials) = fixture(false);
    let proof = prove_public_bgp(&mut rng, &request, &credentials).unwrap();
    let mut invalid = proof.clone();
    invalid.proof.push(0);
    let mut consumed = ConsumedNonces::default();
    assert_eq!(
        verify_public_bgp(&mut rng, &request, &invalid, &mut consumed),
        Err(Error::Decoding)
    );
    assert_eq!(
        verify_public_bgp(&mut rng, &request, &proof, &mut consumed),
        Ok(())
    );
    assert_eq!(format!("{:?}", credentials[0]), "Credential([REDACTED])");
}
