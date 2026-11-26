#[path = "support/authenticated.rs"]
mod authenticated;
#[path = "support/fixture.rs"]
mod fixture_support;

use fixture_support::{fixture, FixtureEncoder};
use sparq_zk_query::{
    capacity::Bounds,
    interp::{evaluate, ResultValue},
    lower::lower,
    witness::prover_toml,
    Error,
};

const B: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 4,
    rows_per_table: 4,
};

fn hint(input: &str, name: &str) -> Vec<u32> {
    let prefix = format!("{name} = [");
    input
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap()
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect()
}

#[test]
fn scan_hints_preserve_physical_padding_and_first_matching_sources() {
    use fixture_support::{iri, quad};
    use sparq_zk_query::interp::CredentialCells;
    let plan = lower("SELECT ?s WHERE {?s <urn:p> <urn:o>}", B).unwrap();
    let b = quad(&iri("urn:b"), &iri("urn:p"), &iri("urn:o"));
    let mut later = b;
    later[0].key[31] = 42;
    let a = quad(&iri("urn:a"), &iri("urn:p"), &iri("urn:o"));
    let data = vec![
        CredentialCells {
            quads: vec![b, later],
        },
        CredentialCells { quads: vec![a] },
    ];
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
    let b_index = trace.tables[0]
        .rows
        .iter()
        .position(|r| r[0] == b[0])
        .unwrap();
    let a_index = trace.tables[0]
        .rows
        .iter()
        .position(|r| r[0] == a[0])
        .unwrap();
    let mut targets = vec![0; 8];
    targets[0] = b_index as u32;
    targets[1] = b_index as u32;
    targets[4] = a_index as u32;
    let mut sources = vec![0; 4];
    sources[a_index] = 4;
    assert_eq!(hint(&input, "target_0"), targets);
    assert_eq!(hint(&input, "source_0"), sources);
}

#[test]
fn scan_hints_check_constants_and_repeated_variables() {
    use fixture_support::{iri, quad};
    use sparq_zk_query::interp::CredentialCells;
    let plan = lower("ASK {?x <urn:p> ?x}", B).unwrap();
    let data = vec![CredentialCells {
        quads: vec![
            quad(&iri("urn:a"), &iri("urn:q"), &iri("urn:a")),
            quad(&iri("urn:a"), &iri("urn:p"), &iri("urn:b")),
            quad(&iri("urn:a"), &iri("urn:p"), &iri("urn:a")),
        ],
    }];
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32], [3; 32]).unwrap();
    assert_eq!(hint(&input, "target_0"), vec![0; 8]);
    assert_eq!(hint(&input, "source_0"), vec![2, 0, 0, 0]);
}

#[test]
fn scan_hints_cover_heterogeneous_authenticated_slots() {
    use sparq_zk_query::{credential::SuiteEncoder, witness::prover_toml_suite_fixture};
    let plan = lower(
        "ASK {?s <urn:p> ?o}",
        Bounds {
            quads_per_credential: 8,
            ..B
        },
    )
    .unwrap()
    .with_credential_quad_bounds(&[4, 8])
    .unwrap();
    let data = vec![
        authenticated::sign("<urn:a> <urn:q> <urn:b> .\n", 0),
        authenticated::fixture().remove(0),
    ];
    let cells = data
        .iter()
        .enumerate()
        .map(|(i, c)| c.cells(i as u32).unwrap())
        .collect::<Vec<_>>();
    let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
    let input = prover_toml_suite_fixture(
        &plan,
        &data,
        &trace,
        [7; 32],
        [3; 32],
        authenticated::issuer_root(),
    )
    .unwrap();
    assert_eq!(hint(&input, "target_0").len(), 12);
    let mut sources = hint(&input, "source_0");
    sources.truncate(trace.tables[0].rows.len());
    sources.sort_unstable();
    assert_eq!(sources, vec![4, 5]);
}

#[test]
fn scan_hints_include_canonical_default_issuer_assertions() {
    use sparq_zk_query::{
        credential::{assemble_issuer_dataset, SuiteEncoder},
        interp::evaluate_dataset,
        witness::{prover_toml_enveloped, EnvelopeInput, EnvelopeWitness},
    };
    let plan = lower(
        &format!(
            "ASK {{?g <{}> <urn:issuer>}}",
            sparq_zk_query::credential::VERIFIED_ISSUER
        ),
        Bounds {
            credentials: 1,
            quads_per_credential: 8,
            rows_per_table: 2,
        },
    )
    .unwrap();
    let data = authenticated::canonical_fixture();
    let policy = authenticated::canonical_policy();
    let dataset = assemble_issuer_dataset(&data, &policy.trust_members, [7; 32]).unwrap();
    let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
    let input = prover_toml_enveloped(
        &plan,
        &data,
        &trace,
        [7; 32],
        [3; 32],
        EnvelopeInput {
            policy: &policy,
            evidence: &EnvelopeWitness::default(),
            revealed: None,
        },
    )
    .unwrap();
    assert_eq!(hint(&input, "target_0"), vec![0; 9]);
    assert_eq!(hint(&input, "source_0"), vec![8, 0]);
}

#[test]
fn canonical_status_witness_admits_bounded_paths_and_refuses_mismatched_lengths() {
    use sparq_zk_query::{
        credential::assemble_issuer_dataset,
        interp::evaluate_dataset,
        witness::{prover_toml_enveloped, EnvelopeInput},
    };
    let plan = lower(
        "ASK {}",
        Bounds {
            credentials: 1,
            quads_per_credential: 16,
            rows_per_table: 2,
        },
    )
    .unwrap()
    .with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures {
        status: true,
        ..Default::default()
    });
    for depth in [0, 1, 32] {
        let (data, policy, evidence) = authenticated::canonical_status_fixture(depth);
        let dataset = assemble_issuer_dataset(&data, &policy.trust_members, [7; 32]).unwrap();
        let trace = evaluate_dataset(&plan, &dataset, &FixtureEncoder).unwrap();
        let render = |evidence: &_| {
            prover_toml_enveloped(
                &plan,
                &data,
                &trace,
                [7; 32],
                [3; 32],
                EnvelopeInput {
                    policy: &policy,
                    evidence,
                    revealed: None,
                },
            )
        };
        render(&evidence).unwrap();
        let mut altered = evidence.clone();
        altered.credentials[0].statuses[0].path.push([0; 32]);
        assert_eq!(
            render(&altered).unwrap_err(),
            Error::Capacity("status bit tree depth".into())
        );
    }
}

#[test]
fn authenticated_read_witnesses_cover_consumer_inputs_and_public_values() {
    use sparq_zk_query::{
        credential::SuiteEncoder, plan::Operator,
        witness::prover_toml_suite_fixture as prover_toml_authenticated,
    };
    let data = authenticated::fixture();
    let cells = vec![data[0].cells(0).unwrap()];
    for query in [
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x > 1)}",
        "SELECT ?x WHERE {VALUES ?x {7 UNDEF} FILTER(?x > 0)}",
        "ASK {FILTER(?absent > 0)}",
    ] {
        let plan = lower(query, B).unwrap();
        let trace = evaluate(&plan, &cells, &SuiteEncoder).unwrap();
        let render = |trace: &_| {
            prover_toml_authenticated(
                &plan,
                &data,
                trace,
                [9; 32],
                [3; 32],
                authenticated::issuer_root(),
            )
        };
        let text = render(&trace).unwrap();
        assert_eq!(text, render(&trace).unwrap());
        let (filter, input) = plan
            .nodes()
            .iter()
            .enumerate()
            .find_map(|(i, n)| {
                if let Operator::Filter(input, _) = n.operator {
                    Some((i, input as usize))
                } else {
                    None
                }
            })
            .unwrap();
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("reads_{filter} =")));
        if query.contains("absent") {
            assert!(line.is_none());
            continue;
        }
        let line = line.unwrap();
        let literals = trace.tables[input]
            .rows
            .iter()
            .flatten()
            .filter(|c| {
                c.bound
                    && c.kind == 3
                    && !plan.parameters().iter().any(|p| {
                        if let sparq_zk_query::plan::Parameter::Term(t) = p {
                            use sparq_zk_query::interp::EncodeTerm;
                            SuiteEncoder.encode(t).unwrap() == **c
                        } else {
                            false
                        }
                    })
            })
            .count();
        assert_eq!(line.matches("bound = true").count(), literals);
        assert_eq!(
            line.matches("bound = false").count(),
            plan.nodes()[input].output.capacity as usize - literals
        );
        let mut forged = trace.clone();
        let literal = forged.tables[input]
            .rows
            .iter_mut()
            .flatten()
            .find(|c| c.bound && c.kind == 3)
            .unwrap();
        literal.enc[31] ^= 1;
        assert_eq!(
            render(&forged).unwrap_err(),
            Error::Invalid("expression read opening".into())
        );
    }
}

#[test]
fn supplied_trace_renders_all_tables_and_keeps_the_public_request() {
    let p = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x IN (1,2))} LIMIT 1 OFFSET 1",
        B,
    )
    .unwrap();
    let data = fixture();
    let trace = evaluate(&p, &data, &FixtureEncoder).unwrap();
    let a = prover_toml(&p, &data, &FixtureEncoder, &trace, [9; 32], [3; 32]).unwrap();
    assert_eq!(
        a,
        prover_toml(&p, &data, &FixtureEncoder, &trace, [9; 32], [3; 32]).unwrap()
    );
    assert!(a.contains("counts = [2, 0]\ncredential_count = 1\n"));
    assert!(a.contains("indices = [2, 1, 1]"));
    assert!(a.contains("request_digest = [9, 9, 9,"));
    for i in 0..p.nodes().len() {
        assert!(a
            .lines()
            .any(|l| l.starts_with(&format!("t{i} = {{ len = "))));
    }
    let mut wrong = trace.clone();
    wrong.tables.pop();
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("trace node count".into())
    );
    let mut wrong = trace.clone();
    wrong.tables[0].rows[0].pop();
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("witness row width".into())
    );
    let mut wrong = trace.clone();
    wrong.tables[0].rows.resize(5, vec![]);
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Capacity("witness table".into())
    );
    let mut wrong = trace.clone();
    wrong.result = ResultValue::Ask(true);
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("result form".into())
    );
    let mut wrong = trace.clone();
    if let ResultValue::Select { variables, .. } = &mut wrong.result {
        variables[0] = "other".into();
    }
    assert_eq!(
        prover_toml(&p, &data, &FixtureEncoder, &wrong, [9; 32], [3; 32]).unwrap_err(),
        Error::Invalid("result variables".into())
    );
    assert!(matches!(
        prover_toml(
            &p,
            &vec![data[0].clone(); 3],
            &FixtureEncoder,
            &trace,
            [9; 32],
            [3; 32],
        ),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn false_ask_is_an_explicit_result_with_an_empty_private_table() {
    let p = lower("ASK {?s <urn:p> ?x FILTER(?x > 8)}", B).unwrap();
    let data = fixture();
    let trace = evaluate(&p, &data, &FixtureEncoder).unwrap();
    let input = prover_toml(&p, &data, &FixtureEncoder, &trace, [1; 32], [3; 32]).unwrap();
    assert!(input.lines().any(|l| l == "result = false"));
    assert_eq!(trace.tables.last().unwrap().rows.len(), 0);
}

#[test]
fn unrequested_validity_does_not_open_unrelated_metadata() {
    use sparq_zk_query::{
        capacity::OpeningProfile,
        credential::{assemble_issuer_dataset, SuiteEncoder},
        interp::evaluate_dataset,
        plan::EnvelopeFeatures,
        witness::{prover_toml_enveloped, EnvelopeInput, EnvelopeWitness},
    };
    let plan = lower(
        "ASK {}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 2,
        },
    )
    .unwrap()
    .with_opening_profile(OpeningProfile::LongObjects);
    let policy = authenticated::canonical_policy();
    for property in [
        "<urn:credential> <https://www.w3.org/2018/credentials#validFrom> <urn:not-a-time> ."
            .to_string(),
        format!(
            "<urn:credential> <https://www.w3.org/2018/credentials#validUntil> \"{}\" .",
            "x".repeat(80)
        ),
    ] {
        let data = [authenticated::canonical_sign(&property, 0)];
        let dataset = assemble_issuer_dataset(&data, &policy.trust_members, [7; 32]).unwrap();
        let trace = evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
        let evidence = EnvelopeWitness::default();
        let input = prover_toml_enveloped(
            &plan,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            EnvelopeInput {
                policy: &policy,
                evidence: &evidence,
                revealed: None,
            },
        )
        .unwrap();
        let slot = input
            .lines()
            .find(|line| line.starts_with("envelope_0 = "))
            .expect("first envelope slot");
        let validity = &slot[slot.find("validity = {").expect("validity table")..];
        assert!(validity.contains("from_len = 0,"), "{validity}");
        assert!(validity.contains("until_len = 0 }"), "{validity}");
        let required = plan.clone().with_envelope_features(EnvelopeFeatures {
            validity: true,
            ..Default::default()
        });
        let mut required_policy = policy.clone();
        required_policy.validity_time = Some(0);
        let error = prover_toml_enveloped(
            &required,
            &data,
            &trace,
            [7; 32],
            [3; 32],
            EnvelopeInput {
                policy: &required_policy,
                evidence: &evidence,
                revealed: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(error, Error::Invalid(_) | Error::Capacity(_)),
            "{error}"
        );
    }
}
