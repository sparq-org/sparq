#[path = "support/fixture.rs"]
mod fixture_support;
use fixture_support::*;
use sparq_zk_query::{
    capacity::Bounds,
    interp::{evaluate, Cell, CredentialCells, EncodeTerm, ResultValue},
    lower::lower,
    Error,
};

const BOUNDS: Bounds = Bounds {
    credentials: 2,
    quads_per_credential: 16,
    rows_per_table: 16,
};

#[test]
fn shared_interpreter_enforces_each_verifier_selected_table_bucket() {
    let bounds = Bounds {
        credentials: 1,
        quads_per_credential: 4,
        rows_per_table: 4,
    };
    let plan =
        sparq_zk_query::credential_graphs::lower("SELECT ?x WHERE {VALUES ?x {1 2 3}}", bounds)
            .unwrap();
    assert!(evaluate(&plan, &[], &FixtureEncoder).is_ok());
    let capacities = vec![2; plan.nodes().len()];
    let refined = sparq_zk_query::plan_closed::with_row_bounds(plan, bounds, &capacities).unwrap();
    assert_eq!(
        evaluate(&refined, &[], &FixtureEncoder).unwrap_err(),
        Error::Capacity("per-table operator output".into())
    );
}

#[test]
fn medication_input_capacity_covers_rejected_rows_before_the_result_limit() {
    let query = "SELECT ?medication WHERE {?medication <urn:strength> ?strength FILTER(?strength >= 31)} LIMIT 1";
    let input = |count| {
        (0..count).map(|i| format!(
        "<urn:medication:{i}> <urn:strength> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n"
    )).collect::<String>()
    };
    let bounds = Bounds {
        credentials: 1,
        quads_per_credential: 64,
        rows_per_table: 32,
    };
    let plan = lower(query, bounds).unwrap();
    let (data, _) = dataset(&input(32));
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    assert!(trace.tables.iter().any(|t| t.rows.len() == 32));
    assert!(matches!(&trace.result, ResultValue::Select {rows,..} if rows.len()==1));
    assert_engine(query, &input(32), &trace.result);
    let narrow = lower(
        query,
        Bounds {
            rows_per_table: 4,
            ..bounds
        },
    )
    .unwrap();
    assert_ne!(plan.hash(), narrow.hash());
    assert_eq!(
        evaluate(&narrow, &data, &FixtureEncoder).unwrap_err(),
        Error::Capacity("operator output".into())
    );
    let (overflow, _) = dataset(&input(33));
    assert_eq!(
        evaluate(&plan, &overflow, &FixtureEncoder).unwrap_err(),
        Error::Capacity("operator output".into())
    );
}

#[test]
fn bag_projection_distinct_join_values_and_false_ask() {
    let data = fixture();
    let run = |q| {
        evaluate(&lower(q, BOUNDS).unwrap(), &data, &FixtureEncoder)
            .unwrap()
            .result
    };
    match run("SELECT ?p WHERE {?s ?p ?o}") {
        ResultValue::Select { rows, .. } => assert_eq!(rows.len(), 2),
        _ => panic!(),
    }
    match run("SELECT DISTINCT ?p WHERE {?s ?p ?o}") {
        ResultValue::Select { rows, .. } => assert_eq!(rows.len(), 1),
        _ => panic!(),
    }
    assert_eq!(
        run("ASK {?s <urn:p> ?x FILTER(?x > 10)}"),
        ResultValue::Ask(false)
    );
    assert_eq!(
        run("ASK {?s <urn:p> ?x VALUES ?x {1 1 UNDEF}}"),
        ResultValue::Ask(true)
    );
    match run("SELECT ?x WHERE {?s <urn:p> ?x VALUES ?x {1 1 UNDEF}}") {
        ResultValue::Select { rows, .. } => assert_eq!(rows.len(), 4),
        _ => panic!(),
    }
    assert_eq!(
        run("ASK {FILTER(?missing || true)}"),
        ResultValue::Ask(true)
    );
    assert_eq!(
        run("ASK {FILTER(?missing && false)}"),
        ResultValue::Ask(false)
    );
    assert_eq!(
        run("ASK {FILTER(2 IN (1,?missing,2))}"),
        ResultValue::Ask(true)
    );
    assert_eq!(
        run("ASK {FILTER(2 IN (1,?missing))}"),
        ResultValue::Ask(false)
    );
}

#[test]
fn merge_deduplicates_triples_and_scopes_blank_nodes() {
    let mut data = fixture();
    data.push(data[0].clone());
    let p = lower("SELECT ?s WHERE {?s <urn:p> ?x}", BOUNDS).unwrap();
    match evaluate(&p, &data, &FixtureEncoder).unwrap().result {
        ResultValue::Select { rows, .. } => assert_eq!(rows.len(), 2),
        _ => panic!(),
    }
    let mut blank = FixtureEncoder
        .encode(&oxrdf::BlankNode::new("b").unwrap().into())
        .unwrap();
    blank.scope = 1;
    data[0].quads = vec![[
        blank,
        FixtureEncoder.encode(&iri("urn:p")).unwrap(),
        FixtureEncoder.encode(&integer(1)).unwrap(),
        Cell::default(),
    ]];
    blank.scope = 2;
    data[1].quads = vec![[
        blank,
        FixtureEncoder.encode(&iri("urn:q")).unwrap(),
        FixtureEncoder.encode(&integer(1)).unwrap(),
        Cell::default(),
    ]];
    let p = lower("ASK {?s <urn:p> ?x . ?s <urn:q> ?y}", BOUNDS).unwrap();
    assert_eq!(
        evaluate(&p, &data, &FixtureEncoder).unwrap().result,
        ResultValue::Ask(false)
    );
    data[1].quads[0][0].scope = 1;
    assert!(matches!(
        evaluate(&p, &data, &FixtureEncoder),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn intermediate_overflow_refuses_even_before_limit_or_false_ask() {
    let p = lower(
        "SELECT ?x WHERE {VALUES ?x {1 2} VALUES ?y {3 4}} LIMIT 1",
        Bounds {
            rows_per_table: 2,
            ..BOUNDS
        },
    )
    .unwrap();
    assert_eq!(
        evaluate(&p, &[], &FixtureEncoder).unwrap_err(),
        Error::Capacity("join output".into())
    );
    let p = lower(
        "ASK {VALUES ?x {1 2} VALUES ?y {3 4} FILTER(?x > 10)}",
        Bounds {
            rows_per_table: 2,
            ..BOUNDS
        },
    )
    .unwrap();
    assert!(matches!(
        evaluate(&p, &[], &FixtureEncoder),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn malformed_cells_and_graphs_are_refused() {
    let p = lower("ASK {?s ?p ?o}", BOUNDS).unwrap();
    let mut data = fixture();
    data[0].quads[0][3].enc[31] = 1;
    assert!(matches!(
        evaluate(&p, &data, &FixtureEncoder),
        Err(Error::Invalid(_))
    ));
    let mut data = fixture();
    data[0].quads[0][1].kind = 3;
    assert!(matches!(
        evaluate(&p, &data, &FixtureEncoder),
        Err(Error::Invalid(_))
    ));
    let mut data = fixture();
    data[0].quads[0][3] = FixtureEncoder.encode(&iri("urn:g")).unwrap();
    assert_eq!(
        evaluate(&p, &data, &FixtureEncoder).unwrap_err(),
        Error::Unsupported("credential named graph".into())
    );
}

#[test]
fn cross_credential_join_and_decimal_range_preserve_exact_rows() {
    let mut data = fixture();
    data.push(CredentialCells {
        quads: vec![quad(&iri("urn:a"), &iri("urn:q"), &iri("urn:group"))],
    });
    let p = lower("SELECT ?s WHERE {?s <urn:p> ?x . ?s <urn:q> ?y}", BOUNDS).unwrap();
    match evaluate(&p, &data, &FixtureEncoder).unwrap().result {
        ResultValue::Select { rows, .. } => assert_eq!(
            rows,
            vec![vec![FixtureEncoder.encode(&iri("urn:a")).unwrap()]]
        ),
        _ => panic!(),
    }
    let decimal = |s: &str| {
        oxrdf::Literal::new_typed_literal(
            s,
            oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#decimal").unwrap(),
        )
        .into()
    };
    data[0].quads = vec![
        quad(&iri("urn:a"), &iri("urn:p"), &decimal("1.25")),
        quad(&iri("urn:b"), &iri("urn:p"), &decimal("2.5")),
    ];
    let p = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1.25 && ?x < 2.5)}",
        BOUNDS,
    )
    .unwrap();
    match evaluate(&p, &data, &FixtureEncoder).unwrap().result {
        ResultValue::Select { rows, .. } => assert_eq!(
            rows,
            vec![vec![FixtureEncoder.encode(&iri("urn:a")).unwrap()]]
        ),
        _ => panic!(),
    }
}

#[test]
fn released_blank_nodes_have_result_local_labels() {
    let mut blank = FixtureEncoder
        .encode(&oxrdf::BlankNode::new("b").unwrap().into())
        .unwrap();
    let original = blank.enc;
    let mut data = vec![];
    for scope in [1, 2] {
        blank.scope = scope;
        data.push(CredentialCells {
            quads: vec![[
                blank,
                FixtureEncoder.encode(&iri("urn:p")).unwrap(),
                FixtureEncoder.encode(&integer(1)).unwrap(),
                Cell::default(),
            ]],
        });
    }
    let p = lower("SELECT ?s WHERE {?s <urn:p> ?x}", BOUNDS).unwrap();
    let trace = evaluate(&p, &data, &FixtureEncoder).unwrap();
    match trace.result {
        ResultValue::Select { rows, .. } => {
            assert_eq!(rows.len(), 2);
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(row[0].scope, 0);
                assert_ne!(row[0].enc, original);
                let mut label = [0; 32];
                label[31] = (i + 1) as u8;
                assert_eq!(row[0].enc, label);
            }
        }
        _ => panic!(),
    }
    assert!(trace.tables[0].rows.iter().all(|r| r[0].scope > 0));
}

#[test]
fn seeded_differential_against_engine() {
    let seeds = std::env::var("SPARQ_NOIR_QUERY_SEED")
        .map(|s| vec![s.parse().expect("integer seed")])
        .unwrap_or_else(|_| vec![0, 1, 7, 41, 893, 1201]);
    for seed in seeds {
        differential(seed);
    }
}

fn differential(seed: u64) {
    let (rdf, data, _) = fixture_support::generated_dataset(seed);
    for q in fixture_support::DIFFERENTIAL_QUERIES {
        let result = evaluate(&lower(q, BOUNDS).unwrap(), &data, &FixtureEncoder)
            .unwrap()
            .result;
        fixture_support::assert_engine(q, &rdf, &result);
    }
}

#[test]
fn mixed_string_equality_uses_recommendation_dispatch_and_records_engine_extension() {
    for (query, expected, engine_expected) in fixture_support::mixed_string_queries() {
        let plan = lower(&query, BOUNDS).unwrap();
        let result = evaluate(&plan, &[], &FixtureEncoder).unwrap().result;
        assert_eq!(result, ResultValue::Ask(expected), "{query}");
        fixture_support::assert_engine(&query, "", &ResultValue::Ask(engine_expected));
    }
}

#[test]
fn malformed_numeric_and_boolean_ebv_is_false_without_a_sparql_error() {
    for datatype in [
        "integer",
        "decimal",
        "boolean",
        "double",
        "float",
        "unsignedByte",
    ] {
        let query =
            format!("ASK {{FILTER(!\"bad\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>)}}");
        let plan = lower(&query, BOUNDS).unwrap();
        let result = evaluate(&plan, &[], &FixtureEncoder).unwrap().result;
        assert_eq!(result, ResultValue::Ask(true));
        fixture_support::assert_engine(&query, "", &result);
    }
}

#[test]
fn zero_bytes_in_strings_preserve_ebv_and_identity_without_implying_lexical_order() {
    for (query, expected) in [
        (r#"ASK { FILTER("\u0000") }"#, true),
        (r#"ASK { FILTER(!"\u0000") }"#, false),
        (r#"ASK { FILTER("" = "\u0000") }"#, false),
        (r#"ASK { FILTER("a" = "a\u0000") }"#, false),
    ] {
        let expected = ResultValue::Ask(expected);
        fixture_support::assert_engine(query, "", &expected);
        assert_eq!(
            evaluate(&lower(query, BOUNDS).unwrap(), &[], &FixtureEncoder)
                .unwrap()
                .result,
            expected
        );
    }
    let query = r#"ASK { FILTER("a" < "a\u0000") }"#;
    fixture_support::assert_engine(query, "", &ResultValue::Ask(true));
    assert_eq!(
        evaluate(&lower(query, BOUNDS).unwrap(), &[], &FixtureEncoder).unwrap_err(),
        sparq_zk_query::Error::Unsupported("string comparison capacity".into())
    );
}

#[test]
fn plain_string_identity_ignores_unused_key_classes() {
    use sparq_zk_query::credential::SuiteEncoder;
    let query = "ASK {?s <urn:p> ?x FILTER(?x NOT IN (\"different\"))}";
    let plan = lower(query, BOUNDS).unwrap();
    let term = oxrdf::Literal::new_simple_literal("value").into();
    let original = quad(&iri("urn:s"), &iri("urn:p"), &term);
    for class in [0, 1, 2, 3, 4, 5, 6, 7, 8, 127] {
        let mut cells = original;
        cells[2] = SuiteEncoder.encode(&term).unwrap();
        cells[2].key_class = class;
        cells[2].key = [0; 32];
        let data = [CredentialCells { quads: vec![cells] }];
        assert_eq!(
            evaluate(&plan, &data, &SuiteEncoder).unwrap().result,
            ResultValue::Ask(true)
        );
    }
}

#[test]
fn plain_string_equality_class_is_derived_from_its_datatype() {
    use sparq_zk_query::credential::SuiteEncoder;
    let plan = lower("ASK {?s <urn:p> ?x FILTER(?x = 1)}", BOUNDS).unwrap();
    let term = oxrdf::Literal::new_simple_literal("value").into();
    let mut cells = quad(&iri("urn:s"), &iri("urn:p"), &term);
    cells[2] = SuiteEncoder.encode(&term).unwrap();
    let numeric = SuiteEncoder
        .encode(&oxrdf::Literal::from(1).into())
        .unwrap();
    cells[2].key_class = numeric.key_class;
    cells[2].key = numeric.key;
    assert_eq!(
        evaluate(
            &plan,
            &[CredentialCells { quads: vec![cells] }],
            &SuiteEncoder
        )
        .unwrap()
        .result,
        ResultValue::Ask(false)
    );
}
