#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/fixture.rs"]
mod fixture_support;
use sparq_zk_query::{
    capacity::Bounds,
    credential::SuiteEncoder,
    interp::{evaluate, ResultValue},
    lower::lower,
    statement::{Layout, Statement},
    witness::prover_toml_suite_fixture as prover_toml_authenticated,
    Error,
};

const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 4,
    rows_per_table: 4,
};

#[test]
fn opening_profile_limits_are_selected_before_private_data() {
    use sparq_zk_query::capacity::OpeningProfile;
    for (length, compact) in [(65, true), (66, false), (137, false), (160, false)] {
        let credential =
            authenticated::sign(&format!("<urn:a> <urn:p> \"{}\" .", "m".repeat(length)), 0);
        assert_eq!(
            credential
                .cells_with_profile(0, OpeningProfile::Solid)
                .is_ok(),
            compact
        );
        assert!(credential
            .cells_with_profile(0, OpeningProfile::Extended)
            .is_ok());
        assert_eq!(
            credential
                .cells_with_profile(0, OpeningProfile::LongObjects)
                .is_ok(),
            length <= 137
        );
    }
    let credential = authenticated::sign(&format!("<urn:{}> <urn:p> <urn:o> .", "r".repeat(61)), 0);
    assert!(
        matches!(credential.cells_with_profile(0, OpeningProfile::Solid),
        Err(Error::Capacity(reason)) if reason == "opening profile lexical bytes")
    );
    assert!(credential
        .cells_with_profile(0, OpeningProfile::Extended)
        .is_ok());
    let credential = authenticated::sign(&format!("<urn:a> <urn:{}> <urn:o> .", "p".repeat(61)), 0);
    assert!(
        matches!(credential.cells_with_profile(0, OpeningProfile::Solid),
        Err(Error::Capacity(reason)) if reason == "opening profile lexical bytes")
    );
    assert!(credential
        .cells_with_profile(0, OpeningProfile::Extended)
        .is_ok());
}

#[test]
fn opening_profile_checks_the_authenticated_verification_method_length() {
    use sparq_zk_query::capacity::OpeningProfile;
    let source = "<urn:a> <urn:p> <urn:o> .";
    let mut credential = authenticated::sign(source, 0);
    let config = format!(
        "_:proof <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n_:proof <https://w3id.org/security#verificationMethod> <urn:{}> .",
        "v".repeat(29)
    );
    credential.credential = sparq_merkle_suite::poseidon::issue(
        source,
        &config,
        &sparq_zk::sig::SecretKey::from_seed(19),
        sparq_merkle_suite::Fr::from(101u64),
    )
    .unwrap();
    assert!(
        matches!(credential.cells_with_profile(0, OpeningProfile::Solid),
        Err(Error::Capacity(reason)) if reason=="opening profile metadata bytes")
    );
    assert!(credential
        .cells_with_profile(0, OpeningProfile::Extended)
        .is_ok());
}

#[test]
fn generated_witness_uses_the_selected_lexical_backing() {
    use sparq_zk_query::capacity::OpeningProfile;
    let data = vec![authenticated::sign(
        &format!("<urn:a> <urn:p> \"{}\" .", "z".repeat(137)),
        0,
    )];
    let compact = lower("ASK {}", B).unwrap();
    let wide = compact
        .clone()
        .with_opening_profile(OpeningProfile::Extended);
    let trace = evaluate(&wide, &[data[0].cells(0).unwrap()], &SuiteEncoder).unwrap();
    assert_eq!(
        prover_toml_authenticated(
            &compact,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root()
        )
        .unwrap_err(),
        Error::Capacity("opening profile lexical bytes".into())
    );
    assert!(prover_toml_authenticated(
        &wide,
        &data,
        &trace,
        [7; 32],
        [3; 32],
        authenticated::issuer_root()
    )
    .is_ok());
}

#[test]
fn suite_credentials_feed_the_same_exact_interpreter_and_public_layout() {
    for case in fixture_support::n0_cases() {
        let bounds = Bounds {
            credentials: case.sources.len() as u32,
            ..B
        };
        let data = case
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| authenticated::sign(s, i as u64))
            .collect::<Vec<_>>();
        let cells = data
            .iter()
            .enumerate()
            .map(|(i, c)| c.cells(i as u32).unwrap())
            .collect::<Vec<_>>();
        let plan = lower(case.query, bounds).unwrap();
        let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
        if case.name != "slice" {
            fixture_support::assert_engine_with_encoder(
                case.query,
                &case.sources.join("\n"),
                &trace.result,
                &SuiteEncoder,
            );
        }
        let input = prover_toml_authenticated(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root(),
        )
        .unwrap();
        assert!(input.contains("signature_mode = 1\n"));
        assert!(input.contains("credential_proof_0 = "));
        assert!(input.contains("credential_0 = "));
        assert!(!input.contains("credentials = "));
        assert_eq!(
            input,
            prover_toml_authenticated(
                &plan,
                &data,
                &trace,
                [7; 32],
                [3; 32],
                authenticated::issuer_root()
            )
            .unwrap()
        );
        let statement = Statement {
            request_digest: [7; 32],
            input_commitment: sparq_zk_query::credential::dataset_commitment(&data, [3; 32])
                .unwrap(),
            signature_mode: 1,
            result: trace.result,
        };
        let words = Layout::new(&plan)
            .reconstruct_authenticated(&plan, &statement, authenticated::issuer_root())
            .unwrap();
        let mut wrong = authenticated::issuer_root();
        wrong[31] ^= 1;
        assert_ne!(
            words.last(),
            Layout::new(&plan)
                .reconstruct_authenticated(&plan, &statement, wrong)
                .unwrap()
                .last()
        );
    }
}

#[test]
fn suite_signature_opening_and_named_admission_guards_are_observable() {
    let valid = authenticated::fixture().remove(0);
    valid.cells(0).unwrap();
    for part in 0..5 {
        let mut changed = valid.clone();
        match part {
            0 => {
                changed.credential.document = changed.credential.document.replace("urn:a", "urn:z")
            }
            1 => changed.credential.salt += sparq_merkle_suite::Fr::from(1u64),
            2 => changed.credential.root += sparq_merkle_suite::Fr::from(1u64),
            3 => changed.credential.count += 1,
            4 => changed.issuer = sparq_zk::sig::SecretKey::from_seed(20).public_key(),
            _ => unreachable!(),
        }
        assert!(changed.cells(0).is_err());
    }
    let mut changed = valid.clone();
    changed.issuer_siblings.pop();
    assert_eq!(
        changed.issuer_root().unwrap_err(),
        Error::Invalid("issuer membership shape".into())
    );
    let named = authenticated::sign("<urn:a> <urn:p> <urn:o> <urn:g> .", 0);
    assert_eq!(
        named.cells(0).unwrap_err(),
        Error::Unsupported("credential named graph".into())
    );
    let long = authenticated::sign(
        &format!(
            "<urn:{}> <urn:p> <urn:o> .",
            "a".repeat(sparq_zk_query::credential::RESOURCE_CAPACITY)
        ),
        0,
    );
    assert_eq!(
        long.cells(0).unwrap_err(),
        Error::Capacity("opened lexical bytes".into())
    );
    let floating = authenticated::sign(
        "<urn:a> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#double> .",
        0,
    );
    let cells = vec![floating.cells(0).unwrap()];
    assert_eq!(
        evaluate(
            &lower("ASK {?s <urn:p> ?x}", B).unwrap(),
            &cells,
            &SuiteEncoder
        )
        .unwrap()
        .result,
        ResultValue::Ask(true)
    );
    assert!(
        matches!(evaluate(&lower("ASK {?s <urn:p> ?x FILTER(?x = 1)}", B).unwrap(), &cells, &SuiteEncoder),
        Err(Error::Unsupported(reason)) if reason == "floating-point comparison")
    );
    let plan=lower("ASK {?s <urn:p> ?x FILTER(?x >= \"2024-01-01Z\"^^<http://www.w3.org/2001/XMLSchema#date>)}",B).unwrap();
    let zoned = authenticated::sign(
        "<urn:a> <urn:p> \"2024-01-01Z\"^^<http://www.w3.org/2001/XMLSchema#date> .",
        0,
    );
    assert_eq!(
        evaluate(&plan, &[zoned.cells(0).unwrap()], &SuiteEncoder).unwrap_err(),
        Error::Capacity("literal comparison range".into())
    );
}

#[test]
fn per_credential_limits_are_checked_before_the_result_and_bind_the_statement() {
    let bounds = Bounds {
        credentials: 4,
        quads_per_credential: 64,
        ..B
    };
    let plan = lower("ASK {}", bounds)
        .unwrap()
        .with_credential_quad_bounds(&[1, 8, 16, 64])
        .unwrap();
    let credentials = authenticated::fixture();
    let cells = vec![credentials[0].cells(0).unwrap()];
    assert_eq!(
        evaluate(&plan, &cells, &SuiteEncoder).unwrap_err(),
        Error::Capacity("per-credential quad count".into())
    );
    let statement = Statement {
        request_digest: [7; 32],
        input_commitment: [0; 32],
        signature_mode: 1,
        result: ResultValue::Ask(true),
    };
    let other = plan
        .clone()
        .with_credential_quad_bounds(&[2, 8, 16, 64])
        .unwrap();
    assert_eq!(plan.hash(), other.hash());
    let layout = Layout::new(&plan);
    assert_ne!(
        layout
            .reconstruct_authenticated(&plan, &statement, authenticated::issuer_root())
            .unwrap(),
        layout
            .reconstruct_authenticated(&other, &statement, authenticated::issuer_root())
            .unwrap()
    );
}

#[test]
fn suite_value_keys_preserve_false_ebv_and_refuse_pending_lexical_ebv() {
    use sparq_zk_query::interp::EncodeTerm;
    let integer = oxrdf::Literal::new_typed_literal(
        "0",
        oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    );
    assert_eq!(
        sparq_zk_query::statement::hex(SuiteEncoder.encode(&integer.into()).unwrap().key),
        "0x0001800000000000000000000000000000000000000000000000000000000000"
    );
    for literal in ["0", "false", "\"\""] {
        let plan = lower(&format!("ASK {{VALUES ?x {{{literal}}} FILTER(?x)}}"), B).unwrap();
        assert_eq!(
            evaluate(&plan, &[], &SuiteEncoder).unwrap().result,
            ResultValue::Ask(false)
        );
    }
    let plan = lower("ASK {VALUES ?x {\"word\"@en} FILTER(?x)}", B).unwrap();
    assert_eq!(
        evaluate(&plan, &[], &SuiteEncoder).unwrap_err(),
        Error::Unsupported("language-tagged effective boolean value".into())
    );
}

#[test]
fn credentials_cannot_supply_the_method_issuer_assertion() {
    let credential = authenticated::sign(
        "<urn:s> <https://w3id.org/sparq/vcq#verifiedIssuer> <urn:issuer> .",
        0,
    );
    assert_eq!(
        credential.cells(0).unwrap_err(),
        Error::Excluded("credential verifiedIssuer assertion".into())
    );
}

#[test]
fn signed_dataset_derives_each_issuer_from_its_authorized_identity_method_and_key() {
    use oxrdf::{NamedNode, Term};
    use sparq_zk_query::{
        credential::{assemble_issuer_dataset, Credential, VERIFIED_ISSUER},
        envelope_policy::TrustMember,
        interp::EncodeTerm,
    };
    let sign = |document: &str, method: Option<&str>, seed: u64| {
        let key = sparq_zk::sig::SecretKey::from_seed(seed);
        let mut configuration = "<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n".to_string();
        if let Some(method) = method {
            configuration.push_str(&format!(
                "<urn:proof> <https://w3id.org/security#verificationMethod> <{method}> .\n"
            ));
        }
        Credential {
            credential: sparq_merkle_suite::poseidon::issue(
                document,
                &configuration,
                &key,
                sparq_merkle_suite::Fr::from(101u64),
            )
            .unwrap(),
            issuer: key.public_key(),
            issuer_index: 0,
            issuer_siblings: vec![sparq_merkle_suite::Fr::from(0u64); 4],
        }
    };
    let document = |issuer: &str| {
        format!("<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:credential> <https://www.w3.org/2018/credentials#issuer> <{issuer}> .\n<urn:subject> <urn:fact> <urn:value> .\n")
    };
    let trust: Vec<_> = (1..=2)
        .map(|i| TrustMember {
            issuer: NamedNode::new_unchecked(format!("urn:issuer:{i}")),
            verification_method: NamedNode::new_unchecked(format!("urn:method:{i}")),
            key: sparq_zk::sig::SecretKey::from_seed(i).public_key(),
        })
        .collect();
    let credentials = [
        sign(&document("urn:issuer:1"), Some("urn:method:1"), 1),
        sign(&document("urn:issuer:2"), Some("urn:method:2"), 2),
    ];
    for credential in &credentials {
        let (set, index, path) = credential.authorization_path(&trust).unwrap();
        let leaf = sparq_zk_query::envelope_policy::trust_member_digest(
            &credential.trust_member().unwrap(),
        )
        .unwrap();
        let mut node = sparq_zk::field::field_from_hex_str_strict(&format!(
            "0x{}",
            leaf.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))
        .unwrap();
        for (level, sibling) in path.iter().enumerate() {
            let sibling = sparq_zk::field::field_from_hex_str_strict(&format!(
                "0x{}",
                sibling
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ))
            .unwrap();
            node = if (index >> level) & 1 == 0 {
                sparq_zk::poseidon2::hash(&[node, sibling])
            } else {
                sparq_zk::poseidon2::hash(&[sibling, node])
            };
        }
        assert_eq!(sparq_zk::field::field_to_be_bytes_32(&node), set.root);
        assert_eq!(path.len(), set.depth as usize);
    }
    let dataset = assemble_issuer_dataset(&credentials, &trust, [7; 32]).unwrap();
    assert_eq!(dataset.credential_counts(), [3, 3]);
    let predicate = SuiteEncoder
        .encode(&Term::NamedNode(NamedNode::new_unchecked(VERIFIED_ISSUER)))
        .unwrap();
    for (i, member) in trust.iter().enumerate() {
        let graph = sparq_zk_query::credential::credential_graph([7; 32], i as u32).unwrap();
        let issuer = SuiteEncoder
            .encode(&Term::NamedNode(member.issuer.clone()))
            .unwrap();
        assert!(dataset
            .quads()
            .contains(&[graph, predicate, issuer, Default::default()]));
        assert_eq!(dataset.quads().iter().filter(|q| q[3] == graph).count(), 3);
    }
    let mut reordered = trust.clone();
    reordered.reverse();
    reordered.push(trust[0].clone());
    assert_eq!(
        dataset,
        assemble_issuer_dataset(&credentials, &reordered, [7; 32]).unwrap()
    );
    let authorization = Error::Invalid("issuer authorization set".into());
    for changed in [
        sign(&document("urn:issuer:2"), Some("urn:method:1"), 1),
        sign(&document("urn:issuer:1"), Some("urn:method:2"), 1),
        sign(&document("urn:issuer:1"), Some("urn:method:1"), 2),
    ] {
        assert_eq!(
            assemble_issuer_dataset(&[changed], &trust, [7; 32]).unwrap_err(),
            authorization
        );
    }
    let missing_method = sign(&document("urn:issuer:1"), None, 1);
    assert_eq!(
        assemble_issuer_dataset(&[missing_method], &trust, [7; 32]).unwrap_err(),
        Error::Invalid("credential verification method".into())
    );
    let rootless = sign(
        "<urn:subject> <urn:fact> <urn:value> .",
        Some("urn:method:1"),
        1,
    );
    let two_roots = sign(&format!("{}<urn:other> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .", document("urn:issuer:1")), Some("urn:method:1"), 1);
    for changed in [rootless, two_roots] {
        assert_eq!(
            assemble_issuer_dataset(&[changed], &trust, [7; 32]).unwrap_err(),
            Error::Invalid("credential root cardinality".into())
        );
    }
    let ambiguous_issuer = sign(
        &format!(
            "{}<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer:2> .",
            document("urn:issuer:1")
        ),
        Some("urn:method:1"),
        1,
    );
    let literal_issuer = sign(
        &document("urn:issuer:1").replace("<urn:issuer:1>", "\"urn:issuer:1\""),
        Some("urn:method:1"),
        1,
    );
    for changed in [ambiguous_issuer, literal_issuer] {
        assert_eq!(
            assemble_issuer_dataset(&[changed], &trust, [7; 32]).unwrap_err(),
            Error::Invalid("credential issuer cardinality or kind".into())
        );
    }
    let mut altered = credentials[0].clone();
    altered.credential.document = altered
        .credential
        .document
        .replace("<urn:value>", "<urn:tampered>");
    assert!(assemble_issuer_dataset(&[altered], &trust, [7; 32]).is_err());
    assert_eq!(
        assemble_issuer_dataset(&credentials, &[], [7; 32]).unwrap_err(),
        Error::Invalid("empty policy set".into())
    );
}
#[test]
fn presentation_graph_names_use_the_shared_suite_constructor() {
    let first = sparq_zk_query::credential::credential_graph([7; 32], 0).unwrap();
    let other_slot = sparq_zk_query::credential::credential_graph([7; 32], 1).unwrap();
    let other_request = sparq_zk_query::credential::credential_graph([8; 32], 0).unwrap();
    assert_eq!(first.kind, 2);
    assert_eq!(first.scope, 1);
    assert_ne!(first.enc, other_slot.enc);
    assert_ne!(first.enc, other_request.enc);
    assert!(sparq_zk_query::credential::credential_graph([7; 32], u32::MAX).is_err());
}

#[test]
fn method_dataset_keeps_named_copies_and_a_deduplicated_default_merge() {
    use sparq_zk_query::{
        credential::{assemble_dataset, SuiteEncoder, VERIFIED_ISSUER},
        interp::{evaluate_dataset, EncodeTerm, ResultValue},
    };
    let cell = fixture_support::quad(
        &fixture_support::iri("urn:s"),
        &fixture_support::iri("urn:p"),
        &fixture_support::integer(1),
    );
    let credentials = vec![sparq_zk_query::interp::CredentialCells { quads: vec![cell] }; 2];
    let issuers = [
        SuiteEncoder
            .encode(&fixture_support::iri("urn:issuer:a"))
            .unwrap(),
        SuiteEncoder
            .encode(&fixture_support::iri("urn:issuer:b"))
            .unwrap(),
    ];
    let dataset = assemble_dataset(&credentials, &issuers, [7; 32]).unwrap();
    assert_eq!(dataset.credential_counts(), [1, 1]);
    assert_eq!(dataset.quads().len(), 4);
    let named = dataset
        .quads()
        .iter()
        .filter(|q| q[3].bound)
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 2);
    assert_ne!(named[0][3].enc, named[1][3].enc);
    let defaults = dataset
        .quads()
        .iter()
        .filter(|q| !q[3].bound)
        .collect::<Vec<_>>();
    assert_eq!(defaults.len(), 2);
    let predicate = SuiteEncoder
        .encode(&fixture_support::iri(VERIFIED_ISSUER))
        .unwrap();
    for (i, q) in defaults.iter().enumerate() {
        assert_eq!(q[0], named[i][3]);
        assert_eq!(q[1], predicate);
        assert_eq!(q[2], issuers[i]);
    }
    let bounds = Bounds {
        credentials: 2,
        quads_per_credential: 4,
        rows_per_table: 4,
    };
    let plan = lower("SELECT ?x WHERE {?s <urn:p> ?x}", bounds).unwrap();
    let ResultValue::Select { rows, .. } = evaluate_dataset(&plan, &dataset, &SuiteEncoder)
        .unwrap()
        .result
    else {
        panic!()
    };
    assert_eq!(rows.len(), 1);
    let mut released = cell[2];
    released.key_class = 0;
    released.key = [0; 32];
    assert_eq!(rows[0][0], released);
    let plan = lower(
        &format!("SELECT ?issuer WHERE {{?g <{VERIFIED_ISSUER}> ?issuer}}"),
        bounds,
    )
    .unwrap();
    let ResultValue::Select { rows, .. } = evaluate_dataset(&plan, &dataset, &SuiteEncoder)
        .unwrap()
        .result
    else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    let actual = rows
        .iter()
        .map(|r| r[0].enc)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, issuers.iter().map(|c| c.enc).collect());
    let other = assemble_dataset(&credentials, &issuers, [8; 32]).unwrap();
    assert_ne!(dataset.quads()[0][3].enc, other.quads()[0][3].enc);
    assert_eq!(other.credential_counts(), dataset.credential_counts());
    assert!(assemble_dataset(&credentials, &issuers[..1], [7; 32]).is_err());
    let mut invalid = issuers;
    invalid[0] = sparq_zk_query::interp::Cell::default();
    assert!(assemble_dataset(&credentials, &invalid, [7; 32]).is_err());
}

#[test]
fn suite_identity_is_shared_by_sources_parameters_and_released_terms() {
    use oxrdf::{Literal, NamedNode, Term};
    use sparq_zk_query::{answer::Answer, interp::EncodeTerm};
    let terms: Vec<Term> = vec![
        NamedNode::new_unchecked("urn:identity").into(),
        Literal::new_simple_literal("one").into(),
        Literal::new_typed_literal(
            "1",
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )
        .into(),
        Literal::new_typed_literal(
            "01",
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        )
        .into(),
        Literal::new_language_tagged_literal("one", "en")
            .unwrap()
            .into(),
    ];
    let mut identities = std::collections::BTreeSet::new();
    for term in terms {
        let encoded = SuiteEncoder.encode(&term).unwrap();
        assert_eq!(
            encoded.enc,
            sparq_merkle_suite::poseidon::field_bytes(
                sparq_merkle_suite::poseidon::term_identity(&term).unwrap()
            )
        );
        assert!(identities.insert(encoded.enc));
        let source = format!("<urn:s> <urn:p> {term} .");
        let credential = authenticated::sign(&source, 0);
        let cells = vec![credential.cells(0).unwrap()];
        assert_eq!(cells[0].quads[0][2], encoded);
        let plan = lower("SELECT ?x WHERE {?s <urn:p> ?x}", B).unwrap();
        let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
        let answer = Answer::release(
            &trace.result,
            &plan,
            std::slice::from_ref(&term),
            &SuiteEncoder,
        )
        .unwrap();
        assert_eq!(
            answer,
            Answer::Select {
                variables: vec!["x".into()],
                rows: vec![vec![Some(term)]]
            }
        );
        assert_eq!(answer.encode(&SuiteEncoder).unwrap(), trace.result);
        let mut statement = Statement {
            request_digest: [7; 32],
            input_commitment: [3; 32],
            signature_mode: 0,
            result: trace.result,
        };
        assert!(Layout::new(&plan)
            .reconstruct(&plan, &statement, &SuiteEncoder)
            .is_ok());
        let mut class_statement = statement.clone();
        if let ResultValue::Select { rows, .. } = &mut class_statement.result {
            rows[0][0].key_class = 1;
        }
        assert_eq!(
            Layout::new(&plan)
                .reconstruct(&plan, &class_statement, &SuiteEncoder)
                .unwrap_err(),
            Error::Invalid("released value metadata".into())
        );
        if let ResultValue::Select { rows, .. } = &mut statement.result {
            assert_eq!(rows[0][0].key_class, 0);
            assert_eq!(rows[0][0].key, [0; 32]);
            rows[0][0].key = encoded.enc;
        }
        assert_eq!(
            Layout::new(&plan)
                .reconstruct(&plan, &statement, &SuiteEncoder)
                .unwrap_err(),
            Error::Invalid("released value metadata".into())
        );
    }
}

#[test]
fn table_identity_keeps_kind_scope_and_excludes_value_metadata() {
    use sparq_zk_query::interp::Cell;
    let mut a = Cell {
        bound: true,
        kind: 1,
        enc: [7; 32],
        ..Cell::default()
    };
    let mut b = a;
    b.kind = 2;
    assert!(!a.same_term(b));
    a.kind = 2;
    a.scope = 1;
    b.scope = 2;
    assert!(!a.same_term(b));
    b.scope = 1;
    b.key_class = 1;
    b.key = [9; 32];
    assert!(a.same_term(b));
}

#[test]
fn private_value_read_bounds_do_not_restrict_unrelated_source_identity() {
    for (literal, error) in [
        (
            format!(
                "\"{}1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "0".repeat(32)
            ),
            Error::Capacity("read scalar lexical bytes".into()),
        ),
        (
            "\"1\"^^<http://www.w3.org/2001/XMLSchema#long>".into(),
            Error::Unsupported("N0 credential value datatype".into()),
        ),
    ] {
        let data = vec![authenticated::sign(
            &format!("<urn:s> <urn:p> {literal} ."),
            0,
        )];
        let cells = vec![data[0].cells(0).unwrap()];
        let identity = lower("ASK {?s <urn:p> ?x}", B).unwrap();
        let trace = evaluate(&identity, &cells, &SuiteEncoder).unwrap();
        assert_eq!(trace.result, ResultValue::Ask(true));
        assert!(prover_toml_authenticated(
            &identity,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            authenticated::issuer_root()
        )
        .is_ok());
        let value = lower("ASK {?s <urn:p> ?x FILTER(?x >= 1)}", B).unwrap();
        let trace = evaluate(&value, &cells, &SuiteEncoder).unwrap();
        assert_eq!(
            prover_toml_authenticated(
                &value,
                &data,
                &trace,
                [7; 32],
                [3; 32],
                authenticated::issuer_root()
            )
            .unwrap_err(),
            error
        );
    }
}

#[test]
fn canonical_witness_paths_bind_the_signed_member_and_public_depth() {
    use sparq_zk_query::{
        credential::assemble_issuer_dataset, envelope_policy, interp::evaluate_dataset,
        statement::EnvelopeInputs, witness::prover_toml_authenticated_with_envelope,
    };
    let policy = authenticated::canonical_policy();
    let public: EnvelopeInputs = envelope_policy::derive(&policy).unwrap().into();
    let mut credential = authenticated::canonical_fixture().remove(0);
    let (set, index, path) = credential
        .authorization_path(&policy.trust_members)
        .unwrap();
    credential.issuer_index = index as u64;
    credential.issuer_siblings = path
        .into_iter()
        .map(|word| {
            sparq_zk::field::field_from_hex_str_strict(&sparq_zk_query::statement::hex(word))
                .unwrap()
        })
        .collect();
    assert_eq!(
        credential
            .canonical_issuer_root(public.issuer_depth)
            .unwrap(),
        set.root
    );
    let mut wrong = credential.clone();
    wrong.issuer_index = 1;
    assert_eq!(
        wrong.canonical_issuer_root(0).unwrap_err(),
        Error::Invalid("issuer membership shape".into())
    );
    assert_eq!(
        credential.canonical_issuer_root(1).unwrap_err(),
        Error::Invalid("issuer membership shape".into())
    );
    let plan = lower(
        "ASK {}",
        Bounds {
            quads_per_credential: 8,
            ..B
        },
    )
    .unwrap();
    let data = [credential];
    let dataset = assemble_issuer_dataset(&data, &policy.trust_members, [7; 32]).unwrap();
    let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
    let render = |public: &EnvelopeInputs| {
        prover_toml_authenticated_with_envelope(&plan, &data, &trace, [7; 32], [3; 32], public)
    };
    let witness = render(&public).unwrap();
    assert!(witness.contains("issuer_depth = 0\n"));
    assert_eq!(witness, render(&public).unwrap());
    let mut wrong = public.clone();
    wrong.issuer_root = sparq_zk_query::statement::number(1);
    assert_eq!(
        render(&wrong).unwrap_err(),
        Error::Invalid("issuer membership root".into())
    );
}
