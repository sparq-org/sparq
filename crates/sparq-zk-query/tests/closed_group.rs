mod closed_support;
use closed_support::*;
use sparq_zk_query::{
    interp::{Cell, EncodeTerm, Table},
    interp_closed_group::*,
    Error,
};

fn agg(kind: AggregateKind, distinct: bool) -> Aggregate {
    Aggregate {
        kind,
        column: (kind != AggregateKind::CountAll).then_some(1),
        distinct,
    }
}
fn spec(kind: AggregateKind, distinct: bool) -> GroupSpec {
    GroupSpec {
        input_width: 2,
        columns: vec![0],
        aggregates: vec![agg(kind, distinct)],
        having: None,
        capacity: 8,
    }
}
fn data() -> Table {
    canonical(vec![
        vec![iri("urn:a"), int(1)],
        vec![iri("urn:a"), int(1)],
        vec![iri("urn:a"), int(4)],
        vec![iri("urn:b"), int(2)],
    ])
}

#[test]
fn all_integer_aggregates_distinct_and_bag() {
    let input = data();
    for (kind, plain, distinct) in [
        (AggregateKind::CountAll, 3, 2),
        (AggregateKind::Count, 3, 2),
        (AggregateKind::Sum, 6, 5),
        (AggregateKind::Min, 1, 1),
        (AggregateKind::Max, 4, 4),
        (AggregateKind::Sample, 1, 1),
    ] {
        for (is_distinct, expected) in [(false, plain), (true, distinct)] {
            let out = group(
                &input,
                &errors(&input, 1),
                &spec(kind, is_distinct),
                &IntegerCodec::new(Encoder).unwrap(),
            )
            .unwrap();
            let a = out.rows.iter().find(|r| r[0] == iri("urn:a")).unwrap()[1];
            if kind == AggregateKind::Sample {
                let candidates = [int(1), int(4)];
                assert_eq!(a, *candidates.iter().min_by_key(|c| c.enc).unwrap());
            } else {
                assert_eq!(a, int(expected));
            }
        }
    }
    let out = group(
        &input,
        &errors(&input, 1),
        &spec(AggregateKind::Avg, true),
        &IntegerCodec::new(Encoder).unwrap(),
    )
    .unwrap();
    let a = out.rows.iter().find(|r| r[0] == iri("urn:a")).unwrap()[1];
    assert_eq!(a.key, key("2.5"));
    assert_eq!(a.dt, hash("http://www.w3.org/2001/XMLSchema#decimal"));
}

#[test]
fn count_star_distinct_observes_whole_mapping() {
    let input = canonical(vec![vec![iri("urn:a"), int(1)], vec![iri("urn:b"), int(1)]]);
    let mut s = spec(AggregateKind::CountAll, true);
    s.columns.clear();
    assert_eq!(
        group(
            &input,
            &errors(&input, 1),
            &s,
            &IntegerCodec::new(Encoder).unwrap()
        )
        .unwrap()
        .rows,
        vec![vec![int(2)]]
    );
    s.aggregates = vec![agg(AggregateKind::Count, true)];
    assert_eq!(
        group(
            &input,
            &errors(&input, 1),
            &s,
            &IntegerCodec::new(Encoder).unwrap()
        )
        .unwrap()
        .rows,
        vec![vec![int(1)]]
    );
}

#[test]
fn global_empty_group_avg_zero_and_explicit_group_empty() {
    let input = Table { rows: vec![] };
    for kind in [
        AggregateKind::CountAll,
        AggregateKind::Count,
        AggregateKind::Sum,
        AggregateKind::Avg,
        AggregateKind::Min,
        AggregateKind::Max,
        AggregateKind::Sample,
    ] {
        let mut s = spec(kind, false);
        s.columns.clear();
        let expected = if matches!(
            kind,
            AggregateKind::Min | AggregateKind::Max | AggregateKind::Sample
        ) {
            Cell::default()
        } else {
            int(0)
        };
        assert_eq!(
            group(&input, &[], &s, &IntegerCodec::new(Encoder).unwrap())
                .unwrap()
                .rows,
            vec![vec![expected]]
        );
        s.columns = vec![0];
        assert!(group(&input, &[], &s, &IntegerCodec::new(Encoder).unwrap())
            .unwrap()
            .rows
            .is_empty());
    }
}

#[test]
fn unbound_error_and_type_error_semantics() {
    let input = canonical(vec![
        vec![iri("urn:a"), Cell::default()],
        vec![iri("urn:a"), int(3)],
    ]);
    for kind in [
        AggregateKind::Count,
        AggregateKind::Sum,
        AggregateKind::Avg,
        AggregateKind::Min,
        AggregateKind::Max,
        AggregateKind::Sample,
    ] {
        let mut e = errors(&input, 1);
        let plain = group(
            &input,
            &e,
            &spec(kind, false),
            &IntegerCodec::new(Encoder).unwrap(),
        )
        .unwrap();
        assert_eq!(
            plain.rows[0][1].bound,
            matches!(kind, AggregateKind::Count | AggregateKind::Sample)
        );
        e[input.rows.iter().position(|r| !r[1].bound).unwrap()][0] = true;
        let explicit = group(
            &input,
            &e,
            &spec(kind, false),
            &IntegerCodec::new(Encoder).unwrap(),
        )
        .unwrap();
        assert_eq!(
            explicit.rows[0][1].bound,
            matches!(kind, AggregateKind::Count | AggregateKind::Sample)
        );
    }
    let input = canonical(vec![
        vec![iri("urn:a"), iri("urn:x")],
        vec![iri("urn:a"), int(3)],
    ]);
    for kind in [AggregateKind::Sum, AggregateKind::Avg] {
        assert!(
            !group(
                &input,
                &errors(&input, 1),
                &spec(kind, false),
                &IntegerCodec::new(Encoder).unwrap()
            )
            .unwrap()
            .rows[0][1]
                .bound
        );
    }
}

#[test]
fn capacity_precision_and_named_dependency_refusals() {
    let input = data();
    let codec = IntegerCodec::new(Encoder).unwrap();
    let mut s = spec(AggregateKind::CountAll, false);
    s.capacity = 1;
    assert!(matches!(
        group(&input, &errors(&input, 1), &s, &codec),
        Err(Error::Capacity(_))
    ));
    s = spec(AggregateKind::GroupConcat, false);
    assert_eq!(
        group(&input, &errors(&input, 1), &s, &codec).unwrap_err(),
        Error::Unsupported("GROUP_CONCAT lexical opening required".into())
    );
    for datatype in ["decimal", "double"] {
        let c = Encoder
            .encode(
                &oxrdf::Literal::new_typed_literal(
                    "1",
                    oxrdf::NamedNode::new(format!("http://www.w3.org/2001/XMLSchema#{datatype}"))
                        .unwrap(),
                )
                .into(),
            )
            .unwrap();
        let input = canonical(vec![vec![iri("urn:a"), c]]);
        assert!(matches!(
            group(
                &input,
                &errors(&input, 1),
                &spec(AggregateKind::Sum, false),
                &codec
            ),
            Err(Error::Unsupported(_))
        ));
    }
    let input = canonical(vec![
        vec![iri("urn:a"), int(0)],
        vec![iri("urn:a"), int(0)],
        vec![iri("urn:a"), int(1)],
    ]);
    assert!(matches!(
        group(
            &input,
            &errors(&input, 1),
            &spec(AggregateKind::Avg, false),
            &codec
        ),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn group_membership_and_having_read_full_input() {
    let input = data();
    let mut s = spec(AggregateKind::Sum, false);
    s.having = Some(Having {
        column: 1,
        comparison: HavingComparison::Greater,
        rhs: int(3),
    });
    assert_eq!(
        group(
            &input,
            &errors(&input, 1),
            &s,
            &IntegerCodec::new(Encoder).unwrap()
        )
        .unwrap()
        .rows,
        vec![vec![iri("urn:a"), int(6)]]
    );
    let full = group(
        &input,
        &errors(&input, 1),
        &spec(AggregateKind::CountAll, false),
        &IntegerCodec::new(Encoder).unwrap(),
    )
    .unwrap();
    let mut hidden = input.clone();
    hidden.rows.remove(0);
    assert_ne!(
        full,
        group(
            &hidden,
            &errors(&hidden, 1),
            &spec(AggregateKind::CountAll, false),
            &IntegerCodec::new(Encoder).unwrap()
        )
        .unwrap()
    );
}

#[test]
fn malformed_error_lanes_columns_and_codec_are_rejected() {
    let input = data();
    let codec = IntegerCodec::new(Encoder).unwrap();
    let mut s = spec(AggregateKind::Sum, false);
    assert!(group(&input, &[], &s, &codec).is_err());
    assert!(group(&input, &vec![vec![true]; input.rows.len()], &s, &codec).is_err());
    s.columns = vec![2];
    assert!(group(&input, &errors(&input, 1), &s, &codec).is_err());
    s = spec(AggregateKind::Sum, false);
    s.aggregates[0].column = None;
    assert!(group(&input, &errors(&input, 1), &s, &codec).is_err());
    struct Wrong;
    impl NumericCodec for Wrong {
        fn types(&self) -> NumericTypes {
            NumericTypes {
                integer_dt: hash("http://www.w3.org/2001/XMLSchema#integer"),
                decimal_dt: [0; 32],
            }
        }
        fn encode_numeric(&self, _: [u8; 32], _: bool) -> Result<Cell, Error> {
            Ok(int(97))
        }
    }
    assert!(matches!(
        group(
            &input,
            &errors(&input, 1),
            &spec(AggregateKind::CountAll, false),
            &Wrong
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn differential_group_aggregate_and_having_on_engine_input_tables() {
    for seed in [0i64, 1, 7, 41, 893, 1201] {
        let mut rdf = String::new();
        for i in 0..6 {
            rdf.push_str(&format!("<urn:s{i}> <urn:g> <urn:g{}> .\n<urn:s{i}> <urn:n> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", i % 2, ((seed + i * 7) % 5) - 2));
        }
        let pattern = "{?s <urn:g> ?g; <urn:n> ?x}";
        let input = engine(&rdf, &format!("SELECT ?g ?x WHERE {pattern}"), &["g", "x"]);
        let input = canonical(input.rows);
        for (name, kind) in [
            ("COUNT(*)", AggregateKind::CountAll),
            ("COUNT(?x)", AggregateKind::Count),
            ("SUM(?x)", AggregateKind::Sum),
            ("MIN(?x)", AggregateKind::Min),
            ("MAX(?x)", AggregateKind::Max),
            ("COUNT(DISTINCT ?x)", AggregateKind::Count),
            ("SUM(DISTINCT ?x)", AggregateKind::Sum),
        ] {
            let s = spec(kind, name.contains("DISTINCT"));
            let expected = engine(
                &rdf,
                &format!("SELECT ?g ({name} AS ?v) WHERE {pattern} GROUP BY ?g"),
                &["g", "v"],
            );
            assert_eq!(
                group(
                    &input,
                    &errors(&input, 1),
                    &s,
                    &IntegerCodec::new(Encoder).unwrap()
                )
                .unwrap(),
                canonical(expected.rows),
                "seed {seed}, {name}"
            );
        }
        let mut s = spec(AggregateKind::Sum, false);
        s.having = Some(Having {
            column: 1,
            comparison: HavingComparison::Greater,
            rhs: int(0),
        });
        let expected = engine(
            &rdf,
            &format!("SELECT ?g (SUM(?x) AS ?v) WHERE {pattern} GROUP BY ?g HAVING(SUM(?x)>0)"),
            &["g", "v"],
        );
        assert_eq!(
            group(
                &input,
                &errors(&input, 1),
                &s,
                &IntegerCodec::new(Encoder).unwrap()
            )
            .unwrap(),
            canonical(expected.rows)
        );
        let grouped = group(
            &input,
            &errors(&input, 1),
            &spec(AggregateKind::Sum, false),
            &IntegerCodec::new(Encoder).unwrap(),
        )
        .unwrap();
        let ordered = sparq_zk_query::interp_closed_order::order_by(
            &grouped,
            2,
            &[
                sparq_zk_query::interp_closed_order::OrderKey {
                    column: 1,
                    descending: false,
                },
                sparq_zk_query::interp_closed_order::OrderKey {
                    column: 0,
                    descending: false,
                },
            ],
            8,
        )
        .unwrap();
        let expected = engine(
            &rdf,
            &format!("SELECT ?g (SUM(?x) AS ?v) WHERE {pattern} GROUP BY ?g ORDER BY ?v ?g"),
            &["g", "v"],
        );
        assert_eq!(ordered, expected, "group/order seed {seed}");
    }
}

#[test]
fn differential_empty_distinct_average_sample_and_errors() {
    let codec = IntegerCodec::new(Encoder).unwrap();
    for (values, expression, kind) in [
        ("", "COUNT(*)", AggregateKind::CountAll),
        ("", "AVG(?x)", AggregateKind::Avg),
        (
            "1 \"01\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            "COUNT(DISTINCT ?x)",
            AggregateKind::Count,
        ),
        ("1 1.0", "COUNT(DISTINCT ?x)", AggregateKind::Count),
        ("1 4", "AVG(?x)", AggregateKind::Avg),
        ("1 3", "AVG(?x)", AggregateKind::Avg),
        ("-1 1", "AVG(?x)", AggregateKind::Avg),
        ("7 7", "SAMPLE(?x)", AggregateKind::Sample),
        ("1 1 UNDEF 2", "COUNT(DISTINCT ?x)", AggregateKind::Count),
        ("1 UNDEF 2", "SUM(?x)", AggregateKind::Sum),
    ] {
        let pattern = format!("{{ VALUES ?x {{{values}}} }}");
        let input = canonical(engine("", &format!("SELECT ?x WHERE {pattern}"), &["x"]).rows);
        let mut s = spec(kind, expression.contains("DISTINCT"));
        s.input_width = 1;
        s.columns.clear();
        if kind != AggregateKind::CountAll {
            s.aggregates[0].column = Some(0);
        }
        let expected = engine(
            "",
            &format!("SELECT ({expression} AS ?v) WHERE {pattern}"),
            &["v"],
        );
        let actual = group(&input, &errors(&input, 1), &s, &codec).unwrap();
        if values == "1 UNDEF 2" && kind == AggregateKind::Sum {
            // Recommendation discriminator: the engine currently skips UNDEF.
            assert_eq!(expected.rows, vec![vec![int(3)]]);
            assert_eq!(actual.rows, vec![vec![Cell::default()]]);
        } else {
            assert_eq!(actual, expected, "{values}: {expression}");
        }
    }
}

#[test]
fn recommendation_error_operands_record_engine_divergences() {
    let codec = IntegerCodec::new(Encoder).unwrap();
    let input = canonical(engine("", "SELECT ?x {VALUES ?x {1 UNDEF 3}}", &["x"]).rows);
    for (name, kind, engine_value) in [
        ("SUM", AggregateKind::Sum, int(4)),
        ("MIN", AggregateKind::Min, int(1)),
        ("MAX", AggregateKind::Max, int(3)),
        (
            "AVG",
            AggregateKind::Avg,
            codec.encode_numeric(key("2"), true).unwrap(),
        ),
    ] {
        let mut s = spec(kind, false);
        s.input_width = 1;
        s.columns.clear();
        s.aggregates[0].column = Some(0);
        let expected = engine(
            "",
            &format!("SELECT ({name}(?x) AS ?v) {{VALUES ?x {{1 UNDEF 3}}}}"),
            &["v"],
        );
        assert_eq!(expected.rows, vec![vec![engine_value]]);
        assert_eq!(
            group(&input, &errors(&input, 1), &s, &codec).unwrap().rows,
            vec![vec![Cell::default()]]
        );
    }
    let empty_values = canonical(vec![vec![Cell::default()]]);
    let mut s = spec(AggregateKind::Avg, false);
    s.input_width = 1;
    s.columns.clear();
    s.aggregates[0].column = Some(0);
    assert_eq!(
        group(&empty_values, &[vec![true]], &s, &codec)
            .unwrap()
            .rows,
        vec![vec![int(0)]]
    );
}

#[test]
fn full_integer_range_and_large_cancellation() {
    let n = "883423532389192164791648750371459257913741948437809479";
    let term = |lexical: &str| {
        Encoder
            .encode(
                &oxrdf::Literal::new_typed_literal(
                    lexical,
                    oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
                )
                .into(),
            )
            .unwrap()
    };
    let high = term(n);
    let low = term(&format!("-{n}"));
    let codec = IntegerCodec::new(Encoder).unwrap();
    let mut s = spec(AggregateKind::Sum, false);
    s.columns.clear();
    let input = canonical(vec![vec![iri("urn:a"), high], vec![iri("urn:a"), high]]);
    assert!(matches!(
        group(&input, &errors(&input, 1), &s, &codec),
        Err(Error::Capacity(_))
    ));
    let input = canonical(vec![
        vec![iri("urn:a"), high],
        vec![iri("urn:a"), high],
        vec![iri("urn:a"), low],
    ]);
    assert_eq!(
        group(&input, &errors(&input, 1), &s, &codec).unwrap().rows,
        vec![vec![high]]
    );
    let input = canonical(vec![vec![iri("urn:a"), low]]);
    assert_eq!(
        group(&input, &errors(&input, 1), &s, &codec).unwrap().rows,
        vec![vec![low]]
    );
}

#[test]
fn grouping_without_aggregates_and_empty_schema_validation() {
    let input = data();
    let codec = IntegerCodec::new(Encoder).unwrap();
    let s = GroupSpec {
        input_width: 2,
        columns: vec![0],
        aggregates: vec![],
        having: None,
        capacity: 2,
    };
    assert_eq!(
        group(&input, &errors(&input, 0), &s, &codec).unwrap(),
        canonical(vec![vec![iri("urn:a")], vec![iri("urn:b")]])
    );
    let mut s = s;
    s.columns.clear();
    let empty = Table { rows: vec![] };
    assert_eq!(group(&empty, &[], &s, &codec).unwrap().rows, vec![vec![]]);
    s.columns = vec![2];
    assert!(matches!(
        group(&empty, &[], &s, &codec),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn having_comparisons_errors_and_nonnumeric_refusal() {
    let input = data();
    let codec = IntegerCodec::new(Encoder).unwrap();
    for (comparison, count) in [
        (HavingComparison::Equal, 1),
        (HavingComparison::Less, 0),
        (HavingComparison::LessOrEqual, 1),
        (HavingComparison::Greater, 1),
        (HavingComparison::GreaterOrEqual, 2),
    ] {
        let mut s = spec(AggregateKind::Sum, false);
        s.having = Some(Having {
            column: 1,
            comparison,
            rhs: int(2),
        });
        assert_eq!(
            group(&input, &errors(&input, 1), &s, &codec)
                .unwrap()
                .rows
                .len(),
            count
        );
    }
    let mut s = spec(AggregateKind::Sum, false);
    s.having = Some(Having {
        column: 1,
        comparison: HavingComparison::Equal,
        rhs: Cell::default(),
    });
    assert!(group(&input, &errors(&input, 1), &s, &codec)
        .unwrap()
        .rows
        .is_empty());
    s.having.as_mut().unwrap().rhs = iri("urn:a");
    assert_eq!(
        group(&input, &errors(&input, 1), &s, &codec).unwrap_err(),
        Error::Unsupported("HAVING numeric comparison required".into())
    );
}
