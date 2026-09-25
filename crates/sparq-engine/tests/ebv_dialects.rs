// [GPT-6] REC 2013 §17.2.2 versus pinned 2026-09-12 WD §17.2.3.
use oxrdf::{Literal, Term};
use sparq_core::Graph;
use sparq_engine::{EbvSemantics, FunctionRegistry, PreparedQuery, QueryBudget};

const PREFIX: &str = "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> ";

#[test]
fn structural_rewrite_retains_all_announcements_and_conflict_refusal() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    let prepared = PreparedQuery::parse(&format!(
        "VERSION '1.2' VERSION '1.2-basic' {PREFIX} ASK {{FILTER(!\"z\"^^xsd:boolean)}}"
    )).unwrap();
    let rewritten = prepared.with_query(prepared.query().clone());
    assert_eq!(rewritten.versions(), ["1.2", "1.2-basic"]);
    assert!(!sparq_engine::ask_prepared(&graph, &rewritten).unwrap());
    assert!(sparq_engine::ask_prepared_with_budget(&graph, &rewritten, &budget(EbvSemantics::Rec2013)).is_err());
}

#[test]
fn update_refuses_unimplemented_dialects_before_mutation() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    let insert = "INSERT DATA { <urn:s> <urn:p> <urn:o> }";
    for label in ["1.2", "1.2-basic", "bogus"] {
        let text = format!("VERSION '{label}' {insert}");
        assert!(sparq_engine::update(&graph, &text).is_err());
        let mut copy = graph.fork();
        assert!(sparq_engine::update_in_place(&mut copy, &text).is_err());
        assert!(!sparq_engine::ask(&copy, "ASK {?s ?p ?o}").unwrap());
        #[cfg(feature = "params")]
        assert!(sparq_engine::PreparedUpdate::parse(&text).is_err());
        // Parser syntax compatibility remains separate from execution support.
        let (_, labels) = spargebra::SparqlParser::new().parse_update_with_versions(&text).unwrap();
        assert_eq!(labels, [label]);
    }
    let updated = sparq_engine::update(&graph, &format!("VERSION '1.1' {insert}")).unwrap();
    assert!(sparq_engine::ask(&updated, "ASK {?s ?p ?o}").unwrap());
    let mut copy = graph.fork();
    assert!(sparq_engine::update_in_place_with_budget(&mut copy, insert, &budget(EbvSemantics::Draft20260912)).is_err());
    assert!(!sparq_engine::ask(&copy, "ASK {?s ?p ?o}").unwrap());
}
fn budget(semantics: EbvSemantics) -> QueryBudget {
    QueryBudget {
        ebv_semantics: Some(semantics),
        ..Default::default()
    }
}
fn result(graph: &Graph, query: &str, semantics: EbvSemantics) -> Vec<Vec<Option<Term>>> {
    sparq_engine::query_with_budget(graph, &format!("{PREFIX}{query}"), &budget(semantics))
        .unwrap()
        .rows
}
fn boolean(value: bool) -> Option<Term> {
    Some(Literal::from(value).into())
}

#[test]
fn invalid_lexical_ebv_has_explicit_operator_goldens() {
    for typed in [
        "\"z\"^^xsd:boolean",
        "\" true \"^^xsd:boolean",
        "\"1.5\"^^xsd:integer",
        "\" 1 \"^^xsd:decimal",
        "\"1200\"^^xsd:byte",
    ] {
        for compressed in [false, true] {
            let graph = Graph::load_str(&format!("@prefix xsd:<http://www.w3.org/2001/XMLSchema#>. <http://ex/s> <http://ex/p> {typed}."), "turtle").unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            for source in [
                format!("VALUES ?v {{{typed}}}"),
                "?s ?p ?v".into(),
                format!("BIND({typed} AS ?v)"),
            ] {
                let query = format!(
                    "SELECT (!?v AS ?not) (!!?v AS ?twice) (IF(?v,1,2) AS ?if) (COALESCE(!?v,7) AS ?coalesce) (?v && false AS ?and) (?v || true AS ?or) (?v && true AS ?and_error) (?v || false AS ?or_error) {{{source}}}"
                );
                assert_eq!(
                    result(&graph, &query, EbvSemantics::Rec2013),
                    vec![vec![
                        boolean(true),
                        boolean(false),
                        Some(Literal::from(2).into()),
                        boolean(true),
                        boolean(false),
                        boolean(true),
                        boolean(false),
                        boolean(false)
                    ]],
                    "{query}"
                );
                assert_eq!(
                    result(&graph, &query, EbvSemantics::Draft20260912),
                    vec![vec![
                        None,
                        None,
                        None,
                        Some(Literal::from(7).into()),
                        boolean(false),
                        boolean(true),
                        None,
                        None
                    ]],
                    "{query}"
                );
                for semantics in [EbvSemantics::Rec2013, EbvSemantics::Draft20260912] {
                    assert!(!sparq_engine::ask_with_budget(
                        &graph,
                        &format!("{PREFIX}ASK {{{source} FILTER(?v)}}"),
                        &budget(semantics)
                    )
                    .unwrap());
                    assert_eq!(
                        sparq_engine::ask_with_budget(
                            &graph,
                            &format!("{PREFIX}ASK {{{source} FILTER(!?v)}}"),
                            &budget(semantics)
                        )
                        .unwrap(),
                        semantics == EbvSemantics::Rec2013
                    );
                }
            }
            let query = format!("SELECT (!!{typed} AS ?v) {{}}");
            assert_eq!(
                result(&graph, &query, EbvSemantics::Rec2013),
                vec![vec![boolean(false)]]
            );
            assert_eq!(
                result(&graph, &query, EbvSemantics::Draft20260912),
                vec![vec![None]]
            );
        }
    }
}

#[test]
fn valid_values_and_ordinary_errors_keep_their_goldens() {
    let graph = Graph::load_str("", "nt").unwrap();
    for semantics in [EbvSemantics::Rec2013, EbvSemantics::Draft20260912] {
        assert_eq!(
            result(
                &graph,
                "SELECT (!!\"0\"^^xsd:integer AS ?a) (!!\"NaN\"^^xsd:double AS ?b) (!!\"true\"^^xsd:boolean AS ?c) (!!xsd:boolean(\" true \") AS ?d) (!!?unbound AS ?e) (COALESCE(1/0,7) AS ?f) {}",
                semantics
            ),
            vec![vec![
                boolean(false),
                boolean(false),
                boolean(true),
                boolean(true),
                None,
                Some(Literal::from(7).into())
            ]]
        );
        let tiny = format!("0.{}1", "0".repeat(324));
        assert_eq!(
            result(
                &graph,
                &format!("SELECT (!!\"{tiny}\"^^xsd:decimal AS ?v) {{}}"),
                semantics
            ),
            vec![vec![boolean(true)]]
        );
    }
}

#[test]
fn nested_patterns_graphs_and_interpreted_aggregate_arguments_inherit() {
    let graph = Graph::load_dataset(
        "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .",
        "nquads",
    )
    .unwrap();
    for (semantics, expected) in [
        (EbvSemantics::Rec2013, true),
        (EbvSemantics::Draft20260912, false),
    ] {
        for query in [
            "SELECT (EXISTS {FILTER(!\"z\"^^xsd:boolean)} AS ?v) {}",
            "SELECT (EXISTS {FILTER(EXISTS {FILTER(!\"z\"^^xsd:boolean)})} AS ?v) {}",
            "SELECT ?v {GRAPH <http://ex/g> {BIND(COALESCE(!\"z\"^^xsd:boolean,false) AS ?v)}}",
            "SELECT ?v {GRAPH ?g {BIND(COALESCE(!\"z\"^^xsd:boolean,false) AS ?v)}}",
            "SELECT (MAX(COALESCE(!?x,false)) AS ?v) {VALUES ?x {\"z\"^^xsd:boolean}}",
            "SELECT ?v {{SELECT (COALESCE(!?x,false) AS ?v) {VALUES ?x {\"z\"^^xsd:boolean}}}}",
        ] {
            assert_eq!(
                result(&graph, query, semantics),
                vec![vec![boolean(expected)]],
                "{query}"
            );
        }
    }
}

#[test]
fn declarations_options_and_prepared_metadata_agree_or_reject() {
    // The query-only selector does not reinterpret existing UPDATE declarations.
    assert!(spargebra::SparqlParser::new()
        .parse_update("VERSION '1.1' VERSION '1.2' INSERT DATA {}")
        .is_ok());
    let graph = Graph::load_str("", "nt").unwrap();
    let body = "SELECT (!!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean> AS ?v) {}";
    assert_eq!(
        sparq_engine::query(&graph, body).unwrap().rows,
        vec![vec![boolean(false)]]
    );
    for (label, semantics) in [
        ("1.1", EbvSemantics::Rec2013),
        ("1.2", EbvSemantics::Draft20260912),
        ("1.2-basic", EbvSemantics::Draft20260912),
    ] {
        let prepared = PreparedQuery::parse(&format!(
            "# VERSION \\\"ignored\\\"\nVERSION '{label}' {body}"
        ))
        .unwrap();
        assert_eq!(prepared.versions(), [label]);
        let expected = if semantics == EbvSemantics::Rec2013 {
            boolean(false)
        } else {
            None
        };
        assert_eq!(
            sparq_engine::query_prepared(&graph, &prepared)
                .unwrap()
                .rows,
            vec![vec![expected.clone()]]
        );
        assert_eq!(
            sparq_engine::query_prepared_with_budget(&graph, &prepared, &budget(semantics))
                .unwrap()
                .rows,
            vec![vec![expected]]
        );
        let opposite = if semantics == EbvSemantics::Rec2013 {
            EbvSemantics::Draft20260912
        } else {
            EbvSemantics::Rec2013
        };
        assert!(
            sparq_engine::query_prepared_with_budget(&graph, &prepared, &budget(opposite))
                .unwrap_err()
                .contains("contradicts")
        );
    }
    let compatible = PreparedQuery::parse(&format!(
        "VERSION '1.2' VERSION '1.2-basic' VERSION '1.2' {body}"
    ))
    .unwrap();
    assert_eq!(compatible.versions(), ["1.2", "1.2-basic", "1.2"]);
    assert_eq!(
        sparq_engine::query_prepared(&graph, &compatible)
            .unwrap()
            .rows,
        vec![vec![None]]
    );
    assert!(sparq_engine::query_prepared_with_budget(
        &graph,
        &compatible,
        &budget(EbvSemantics::Rec2013)
    )
    .is_err());
    assert!(PreparedQuery::parse(&format!("VERSION 'unknown' {body}")).is_err());
    assert!(PreparedQuery::parse(&format!("VERSION 'unknown' VERSION '1.2' {body}")).is_err());
    assert!(spargebra::SparqlParser::new()
        .parse_query(&format!("VERSION '1.1' VERSION '1.2' {body}"))
        .is_ok());
    assert!(PreparedQuery::parse(&format!("VERSION '1.1' VERSION '1.2' {body}")).is_err());
    assert_eq!(
        PreparedQuery::parse("SELECT (\"VERSION '1.2'\" AS ?v) {}")
            .unwrap()
            .versions(),
        &[] as &[String]
    );
}

#[test]
fn nested_public_api_defaults_and_explicit_rules_do_not_leak() {
    let graph = Graph::load_str("", "nt").unwrap();
    let mut registry = FunctionRegistry::new();
    registry.register("http://ex/nested", |_| {
        let graph = Graph::load_str("", "nt").unwrap();
        let query = "ASK {FILTER(!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>)}";
        assert!(sparq_engine::ask(&graph, query).unwrap());
        assert!(!sparq_engine::ask_with_budget(
            &graph,
            query,
            &budget(EbvSemantics::Draft20260912)
        )
        .unwrap());
        Ok(Literal::from(true).into())
    });
    for semantics in [EbvSemantics::Rec2013, EbvSemantics::Draft20260912] {
        let query = format!(
            "{PREFIX}SELECT (<http://ex/nested>() AS ?nested) (!!\"z\"^^xsd:boolean AS ?v) {{}}"
        );
        let rows = sparq_engine::query_with_functions_and_budget(
            &graph,
            &query,
            &registry,
            &budget(semantics),
        )
        .unwrap()
        .rows;
        assert_eq!(
            rows,
            vec![vec![
                boolean(true),
                if semantics == EbvSemantics::Rec2013 {
                    boolean(false)
                } else {
                    None
                }
            ]]
        );
    }
}

#[test]
fn entry_points_refuse_conflicts_before_emitting_results() {
    let graph = Graph::load_str("", "nt").unwrap();
    let pin = budget(EbvSemantics::Rec2013);
    let select =
        "VERSION '1.2' SELECT (!!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean> AS ?v) {}";
    let prepared = PreparedQuery::parse(select).unwrap();
    assert!(sparq_engine::query_json_with_budget(&graph, select, &pin).is_err());
    assert!(sparq_engine::query_json_chunks_with_budget(&graph, select, &pin).is_err());
    assert!(sparq_engine::count_prepared_with_budget(&graph, &prepared, &pin).is_err());
    let mut emitted = false;
    assert!(
        sparq_engine::query_json_stream_prepared_with_budget(&graph, &prepared, &pin, |_| {
            emitted = true;
            std::ops::ControlFlow::Continue(())
        })
        .is_err()
    );
    assert!(!emitted);
    assert!(sparq_engine::ask_with_budget(&graph, "VERSION '1.2' ASK {}", &pin).is_err());
    assert!(sparq_engine::construct_with_budget(
        &graph,
        "VERSION '1.2' CONSTRUCT {<http://ex/s> <http://ex/p> <http://ex/o>} WHERE {}",
        &pin
    )
    .is_err());
    assert!(sparq_engine::describe_with_budget(
        &graph,
        "VERSION '1.2' DESCRIBE <http://ex/s>",
        &pin
    )
    .is_err());
    assert!(sparq_engine::explain_analyze_with_budget(&graph, select, &pin).is_err());
    #[cfg(feature = "explain-json")]
    assert!(sparq_engine::explain_plan_analyze_with_budget(&graph, select, &pin).is_err());
    #[cfg(feature = "params")]
    {
        let bound = PreparedQuery::parse("VERSION '1.2' SELECT (!!?value AS ?v) {}")
            .unwrap()
            .bind(
                "value",
                Literal::new_typed_literal("z", oxrdf::vocab::xsd::BOOLEAN).into(),
            )
            .unwrap();
        assert_eq!(bound.versions(), ["1.2"]);
        assert!(sparq_engine::query_prepared_with_budget(&graph, &bound, &pin).is_err());
    }
}

#[test]
#[cfg(feature = "window-functions")]
fn custom_aggregate_parser_and_explain_keep_the_announcement() {
    let graph = Graph::load_str("", "nt").unwrap();
    let mut registry = sparq_engine::CustomAggregateRegistry::new();
    registry.register("http://ex/first", |values| Ok(values[0].clone()));
    for (label, expected) in [("1.1", boolean(false)), ("1.2", None)] {
        let query = format!(
            "VERSION '{label}' SELECT (<http://ex/first>(!!?v) AS ?result) {{VALUES ?v {{\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>}}}}"
        );
        assert_eq!(
            sparq_engine::query_with_aggregates(&graph, &query, &registry)
                .unwrap()
                .rows,
            vec![vec![expected]]
        );
        let selected = format!(
            "VERSION '{label}' SELECT ?v {{VALUES ?v {{\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>}} FILTER(!?v)}}"
        );
        let trace = sparq_engine::explain_analyze(&graph, &selected).unwrap();
        assert!(trace.contains(if label == "1.1" {
            "Total: 1 result row(s)"
        } else {
            "Total: 0 result row(s)"
        }));
        let windowed = format!(
            "VERSION '{label}' SELECT ?v (ROW_NUMBER() OVER (ORDER BY ?v) AS ?rank) {{VALUES ?v {{\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>}} FILTER(!?v)}}"
        );
        assert_eq!(
            sparq_engine::query_over(&graph, &windowed)
                .unwrap()
                .rows
                .len(),
            usize::from(label == "1.1")
        );
        assert_eq!(
            sparq_engine::query_over_with_budget(&graph, &windowed, &budget(EbvSemantics::Rec2013))
                .is_ok(),
            label == "1.1"
        );
    }
    let query = "VERSION '1.2' SELECT (<http://ex/first>(!!?v) AS ?result) {VALUES ?v {true}}";
    assert!(sparq_engine::query_with_aggregates_and_budget(
        &graph,
        query,
        &registry,
        &budget(EbvSemantics::Rec2013)
    )
    .is_err());
}

#[cfg(feature = "result-cache")]
#[test]
fn result_cache_keys_the_resolved_rule_in_both_orders() {
    let graph = Graph::load_str("", "nt").unwrap();
    let body = "SELECT (!!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean> AS ?v) {}";
    let query = PreparedQuery::parse(body).unwrap();
    for order in [
        [EbvSemantics::Rec2013, EbvSemantics::Draft20260912],
        [EbvSemantics::Draft20260912, EbvSemantics::Rec2013],
    ] {
        let cache = sparq_engine::ResultCache::new(4);
        for _ in 0..2 {
            for rule in order {
                let result = cache
                    .get_or_eval(&graph, query.query(), 0, &budget(rule))
                    .unwrap();
                assert_eq!(
                    result.rows,
                    vec![vec![if rule == EbvSemantics::Rec2013 {
                        boolean(false)
                    } else {
                        None
                    }]]
                );
            }
        }
        assert_eq!(
            (
                cache.stats().hits,
                cache.stats().misses,
                cache.stats().entries
            ),
            (2, 2, 2)
        );
        let announced = PreparedQuery::parse(&format!("VERSION '1.2' {body}")).unwrap();
        assert_eq!(
            cache
                .get_or_eval_prepared(&graph, &announced, 0, &QueryBudget::default())
                .unwrap()
                .rows,
            vec![vec![None]]
        );
        let before = cache.stats().hits;
        assert!(cache
            .get_or_eval_prepared(&graph, &announced, 0, &budget(EbvSemantics::Rec2013))
            .is_err());
        assert_eq!(
            cache.stats().hits,
            before,
            "conflict must reject before cache lookup"
        );
    }
}

#[cfg(feature = "parallel")]
#[test]
fn rayon_filter_and_bind_use_the_captured_rule() {
    let data=(0..50_001).map(|i|format!("<http://ex/s{i}> <http://ex/p> \"z\"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n")).collect::<String>();
    let graph = Graph::load_str(&data, "nt").unwrap();
    for semantics in [EbvSemantics::Rec2013, EbvSemantics::Draft20260912] {
        let expected = if semantics == EbvSemantics::Rec2013 {
            50_001
        } else {
            0
        };
        assert_eq!(
            result(&graph, "SELECT ?s {?s ?p ?x FILTER(!?x)}", semantics).len(),
            expected
        );
        let rows = result(&graph, "SELECT ?v {?s ?p ?x BIND(!!?x AS ?v)}", semantics);
        assert_eq!(rows.len(), 50_001);
        assert!(rows.iter().all(|row| row
            == &vec![if semantics == EbvSemantics::Rec2013 {
                boolean(false)
            } else {
                None
            }]));
    }
}
