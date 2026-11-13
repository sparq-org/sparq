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
    witness::prover_toml_authenticated,
    Error,
};

const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 4,
    rows_per_table: 4,
};

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
        changed.cells(0).unwrap_err(),
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
    assert_eq!(
        floating.cells(0).unwrap_err(),
        Error::Unsupported("N0 credential literal datatype".into())
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
    assert_eq!(rows[0][0], cell[2]);
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
