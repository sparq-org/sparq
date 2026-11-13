//! Behavioral guards independent of the fetched W3C manifests.
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_shacl::{eval_node_expression_result, view::GraphView, Scope};

const PREFIXES: &str = r#"
@prefix ex: <urn:ex:> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .
@prefix sparql: <http://www.w3.org/ns/sparql#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
"#;
fn graph(text: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{text}"), "turtle").unwrap()
}
fn iri(local: &str) -> Term {
    NamedNode::new(format!("urn:ex:{local}")).unwrap().into()
}
fn run(data: &Graph, shapes: &Graph, scope: &Scope) -> Result<Vec<Term>, String> {
    let expr = GraphView::new(shapes)
        .object(&iri("Decl"), "urn:ex:expr")
        .unwrap();
    eval_node_expression_result(data, shapes, &expr, &iri("focus"), scope)
}
fn eval(text: &str) -> Result<Vec<Term>, String> {
    run(
        &graph(""),
        &graph(&format!("ex:Decl ex:expr {text} .")),
        &Scope::default(),
    )
}
fn ints(values: &[i64]) -> Vec<Term> {
    values.iter().map(|v| Literal::from(*v).into()).collect()
}
fn strings(values: &[&str]) -> Vec<Term> {
    values
        .iter()
        .map(|v| Literal::new_simple_literal(*v).into())
        .collect()
}

#[test]
fn failures_are_distinct_from_empty_and_propagate_through_named_operators() {
    assert_eq!(eval("[]").unwrap(), vec![]);
    for expr in [
        "[shnex:count [shnex:instancesOf 42]]",
        "[shnex:if [shnex:instancesOf 42]; shnex:then 1; shnex:else 2]",
        "[shnex:intersection ([] [shnex:instancesOf 42])]",
        "[shnex:concat (1 [shnex:instancesOf 42])]",
        "[shnex:nodes (1 2); shnex:orderBy [shnex:instancesOf 42]]",
    ] {
        assert!(eval(expr).is_err(), "{expr}");
    }
}

#[test]
fn path_values_require_one_start_but_legacy_path_accepts_many() {
    let data = graph("ex:a ex:p 7 . ex:b ex:p 8 .");
    for (syntax, expected) in [
        ("[shnex:pathValues ex:p; shnex:focusNode (ex:a ex:b)]", None),
        ("[shnex:pathValues ex:p; shnex:focusNode []]", Some(vec![])),
        (
            "[shnex:pathValues ex:p; shnex:focusNode ex:b]",
            Some(ints(&[8])),
        ),
        ("[sh:path ex:p; sh:nodes (ex:a ex:b)]", Some(ints(&[7, 8]))),
    ] {
        let result = run(
            &data,
            &graph(&format!("ex:Decl ex:expr {syntax} .")),
            &Scope::default(),
        );
        match expected {
            Some(expected) => assert_eq!(result.unwrap(), expected),
            None => assert!(result.is_err()),
        }
    }
}

#[test]
fn dynamic_instance_classes_union_and_reject_non_iris() {
    let data = graph("ex:a a ex:A, ex:B . ex:b a ex:Sub . ex:Sub rdfs:subClassOf ex:B . ex:focus ex:class ex:A, ex:B .");
    let shapes = graph("ex:Decl ex:expr [shnex:instancesOf [shnex:pathValues ex:class]] .");
    let mut got = run(&data, &shapes, &Scope::default()).unwrap();
    got.sort_by_key(ToString::to_string);
    assert_eq!(got, vec![iri("a"), iri("b")]);
    assert!(eval("[shnex:instancesOf (ex:A 3)]").is_err());
}

#[test]
fn extrema_support_strings_dates_and_exact_large_numbers() {
    assert_eq!(eval("[shnex:min (\"z\" \"a\")] ").unwrap(), strings(&["a"]));
    assert_eq!(
        eval("[shnex:max (9007199254740992 9007199254740993)]").unwrap(),
        ints(&[9007199254740993])
    );
    let result = eval("[shnex:min (\"2024-12-31\"^^xsd:date \"2024-01-01\"^^xsd:date)]").unwrap();
    assert_eq!(
        result,
        vec![Literal::new_typed_literal(
            "2024-01-01",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#date").unwrap()
        )
        .into()]
    );
}

#[test]
fn ordering_uses_term_kind_and_first_key_preserving_duplicates() {
    assert_eq!(
        eval("[shnex:nodes (\"aaa\" ex:z ex:z); shnex:orderBy [shnex:var \"focusNode\"]]").unwrap(),
        vec![
            iri("z"),
            iri("z"),
            Literal::new_simple_literal("aaa").into()
        ]
    );
    let data = graph("ex:a ex:keys 9, 1 . ex:b ex:keys 4 .");
    let shapes = graph(
        "ex:Decl ex:expr [shnex:nodes (ex:a ex:b); shnex:orderBy [shnex:pathValues ex:keys]] .",
    );
    // Determine the first path value as the path's order is implementation-defined.
    let first = GraphView::new(&data).objects(&iri("a"), "urn:ex:keys")[0].clone();
    let expected = if first == Literal::from(9).into() {
        vec![iri("b"), iri("a")]
    } else {
        vec![iri("a"), iri("b")]
    };
    assert_eq!(run(&data, &shapes, &Scope::default()).unwrap(), expected);
}

#[test]
fn if_branches_are_optional_and_unselected_failures_are_ignored() {
    assert_eq!(
        eval("[shnex:if true; shnex:else [shnex:instancesOf 42]]").unwrap(),
        vec![]
    );
    assert_eq!(
        eval("[shnex:if false; shnex:then [shnex:instancesOf 42]; shnex:else 7]").unwrap(),
        ints(&[7])
    );
}

#[test]
fn triple_term_constants_keep_the_complete_term() {
    let term = Term::Triple(Box::new(oxrdf::Triple::new(
        NamedNode::new("urn:s").unwrap(),
        NamedNode::new("urn:p").unwrap(),
        Literal::from(4),
    )));
    assert_eq!(
        eval_node_expression_result(
            &graph(""),
            &graph(""),
            &term,
            &iri("focus"),
            &Scope::default()
        )
        .unwrap(),
        vec![term]
    );
}

const FUNCTIONS: &str = r#"
ex:identity a sh:NamedParameterExpressionFunction; rdfs:subClassOf sh:NamedParameterExpression;
    sh:parameter [sh:path ex:key; sh:keyParameter true]; sh:bodyExpression [shnex:arg ex:key] .
ex:echo a sh:ListParameterExpressionFunction; rdfs:subClassOf sh:ListParameterExpression;
    sh:bodyExpression [shnex:arg 0] .
ex:constant a sh:ListParameterExpressionFunction; rdfs:subClassOf sh:ListParameterExpression;
    sh:bodyExpression 42 .
"#;
#[test]
fn named_arguments_are_lazy_expressions_with_an_empty_scope() {
    let mut scope = Scope::default();
    scope.insert("outer".into(), ints(&[99]));
    for (expr, expected) in [
        ("[ex:key (3 2 3)]", ints(&[3, 2, 3])),
        ("[ex:key [shnex:var \"outer\"]]", vec![]),
        ("[ex:key [shnex:var \"focusNode\"]]", vec![iri("focus")]),
    ] {
        let shapes = graph(&format!("{FUNCTIONS} ex:Decl ex:expr {expr} ."));
        assert_eq!(run(&graph(""), &shapes, &scope).unwrap(), expected);
    }
}

#[test]
fn list_arguments_see_outer_scope_but_empty_arguments_prevent_constant_bodies() {
    let mut scope = Scope::default();
    scope.insert("outer".into(), ints(&[19]));
    for (expr, expected) in [
        ("[ex:echo [shnex:var \"outer\"]]", ints(&[19])),
        ("[ex:echo ()]", vec![]),
        ("[ex:constant [shnex:var \"missing\"]]", vec![]),
        ("[ex:constant 8]", ints(&[42])),
    ] {
        assert_eq!(
            run(
                &graph(""),
                &graph(&format!("{FUNCTIONS} ex:Decl ex:expr {expr} .")),
                &scope
            )
            .unwrap(),
            expected
        );
    }
    for args in ["([] (1 2))", "((1 2) [])", "([] [shnex:instancesOf 42])"] {
        let shapes = graph(&format!(
            "{FUNCTIONS} ex:Decl ex:expr [ex:constant {args}] ."
        ));
        assert!(run(&graph(""), &shapes, &scope).is_err());
    }
}

#[test]
fn list_body_must_be_scalar_and_nested_functions_share_one_evaluator() {
    let shapes = graph(&format!(
        r#"{FUNCTIONS}
        ex:many a sh:ListParameterExpressionFunction; sh:bodyExpression (1 2) .
        ex:Decl ex:expr [ex:many ()] ."#
    ));
    assert!(run(&graph(""), &shapes, &Scope::default()).is_err());
    let shapes = graph(&format!(
        r#"{FUNCTIONS} ex:Decl ex:expr [ex:echo [ex:key [sparql:plus (8 9)]]] ."#
    ));
    assert_eq!(
        run(&graph(""), &shapes, &Scope::default()).unwrap(),
        ints(&[17])
    );
}

#[test]
fn sparql_errors_precede_empty_arguments_and_functions_handle_unbound() {
    for args in ["([] [shnex:instancesOf 42])", "([shnex:instancesOf 42] [])"] {
        assert!(eval(&format!("[sparql:plus {args}]")).is_err());
    }
    assert_eq!(eval("[sparql:plus ([] 4)]").unwrap(), vec![]);
    assert_eq!(
        eval("[sparql:bound []]").unwrap(),
        vec![Literal::from(false).into()]
    );
    assert_eq!(
        eval("[sparql:coalesce ((1 2) [sparql:abs \"bad\"] 7)]").unwrap(),
        ints(&[7])
    );
    assert_eq!(
        eval("[sparql:if (false [shnex:instancesOf 42] 8)]").unwrap(),
        ints(&[8])
    );
    assert!(eval("[sparql:if (true [] 8)]").is_err());
    assert!(eval("[sparql:coalesce ([] [])]").is_err());
}

#[test]
fn sparql_logical_error_truth_tables_are_symmetric() {
    for (name, decisive, expected) in [
        ("logical-and", "false", false),
        ("logical-or", "true", true),
    ] {
        for args in [
            format!("({decisive} [shnex:instancesOf 42])"),
            format!("([shnex:instancesOf 42] {decisive})"),
        ] {
            assert_eq!(
                eval(&format!("[sparql:{name} {args}]")).unwrap(),
                vec![Literal::from(expected).into()]
            );
        }
    }
    assert!(eval("[sparql:logical-and (true [])]").is_err());
    assert!(eval("[sparql:logical-or (false [])]").is_err());
}

#[test]
fn same_value_uses_value_equality_and_same_term_uses_rdf_term_equality() {
    assert_eq!(
        eval("[sparql:sameValue (1 1.0)]").unwrap(),
        vec![Literal::from(true).into()]
    );
    assert_eq!(
        eval("[sparql:sameTerm (1 1.0)]").unwrap(),
        vec![Literal::from(false).into()]
    );
}

#[test]
fn select_binds_scope_and_custom_argument_indexes() {
    let mut scope = Scope::default();
    scope.insert("offset".into(), ints(&[10]));
    let shapes = graph("ex:Decl ex:expr [sh:select \"SELECT ((?offset + 3) AS ?v) WHERE {}\"] .");
    assert_eq!(run(&graph(""), &shapes, &scope).unwrap(), ints(&[13]));
    let shapes = graph(&format!(
        r#"{FUNCTIONS}
      ex:select a sh:ListParameterExpressionFunction;
          sh:bodyExpression [sh:select "SELECT ((?arg0 + ?arg1) AS ?v) WHERE {{}}"] .
      ex:Decl ex:expr [ex:select (7 6)] ."#
    ));
    assert_eq!(
        run(&graph(""), &shapes, &Scope::default()).unwrap(),
        ints(&[13])
    );
    for name in ["this", "bad variable"] {
        let bad = [(name.into(), ints(&[1]))].into_iter().collect();
        assert!(
            run(&graph(""), &shapes, &bad).is_ok(),
            "custom function replaces the outer scope"
        );
        let shapes = graph("ex:Decl ex:expr [sh:sparqlExpr \"1\"] .");
        assert!(run(&graph(""), &shapes, &bad).is_err());
    }
}

#[test]
fn select_preserves_blank_and_triple_term_bindings() {
    let data = graph("_:node ex:p 5 .");
    let focus = GraphView::new(&data).subjects_of("urn:ex:p").remove(0);
    let shapes = graph("ex:Decl ex:expr [sh:select \"SELECT ?this WHERE {}\"] .");
    let expr = GraphView::new(&shapes)
        .object(&iri("Decl"), "urn:ex:expr")
        .unwrap();
    assert_eq!(
        eval_node_expression_result(&data, &shapes, &expr, &focus, &Scope::default()).unwrap(),
        vec![focus]
    );
    let triple = Term::Triple(Box::new(oxrdf::Triple::new(
        NamedNode::new("urn:s").unwrap(),
        NamedNode::new("urn:p").unwrap(),
        Literal::from(4),
    )));
    assert_eq!(
        eval_node_expression_result(&data, &shapes, &expr, &triple, &Scope::default()).unwrap(),
        vec![triple]
    );
}

#[test]
fn conformance_uses_the_supplied_shapes_graph_and_preserves_failures() {
    let data = graph("ex:focus ex:p 5 . ex:Valid a sh:NodeShape; sh:class ex:Absent .");
    let shapes = graph("ex:Valid a sh:NodeShape; sh:property [sh:path ex:p; sh:minCount 1] . ex:Decl ex:expr [shnex:conformsToShape ([shnex:var \"focusNode\"] ex:Valid)] .");
    assert_eq!(
        run(&data, &shapes, &Scope::default()).unwrap(),
        vec![Literal::from(true).into()]
    );
    let bad_shapes = graph("ex:Bad a sh:NodeShape; sh:sparql [sh:select \"SELECT ?this ?failure WHERE { BIND(true AS ?failure) }\"] . ex:Decl ex:expr [shnex:conformsToShape ([shnex:var \"focusNode\"] ex:Bad)] .");
    assert!(run(&data, &bad_shapes, &Scope::default()).is_err());
}

#[test]
fn validation_custom_sparql_functions_use_the_complete_shared_body_evaluator() {
    let data = graph("ex:focus ex:p 1 .");
    let shapes = graph(
        r#"
      ex:double a sh:ListParameterExpressionFunction;
          sh:bodyExpression [sparql:multiply ([shnex:arg 0] 2)] .
      ex:outer a sh:ListParameterExpressionFunction;
          sh:bodyExpression [ex:double [shnex:arg 0]] .
      ex:S sh:targetNode ex:focus; sh:sparql [sh:select "SELECT $this WHERE { FILTER(<urn:ex:outer>(21) != 42) }"] .
    "#,
    );
    assert!(
        sparq_shacl::validate_strict(&data, &shapes)
            .unwrap()
            .conforms
    );
}

#[test]
fn ordering_mixed_literals_is_consistent_across_permutations() {
    for values in [
        "2 10 \"11\"",
        "2 \"11\" 10",
        "10 2 \"11\"",
        "10 \"11\" 2",
        "\"11\" 2 10",
        "\"11\" 10 2",
    ] {
        let expected: Vec<Term> = ints(&[2, 10]).into_iter().chain(strings(&["11"])).collect();
        assert_eq!(
            eval(&format!(
                "[shnex:nodes ({values}); shnex:orderBy [shnex:var \"focusNode\"]]"
            ))
            .unwrap(),
            expected
        );
    }
    assert_eq!(
        eval("[shnex:nodes (2 10 \"3\"); shnex:orderBy [shnex:var \"focusNode\"]]").unwrap(),
        ints(&[2, 10])
            .into_iter()
            .chain(strings(&["3"]))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        eval("[shnex:min (\"10\" \"2\")]").unwrap(),
        strings(&["10"])
    );
    assert_eq!(eval("[shnex:max (\"10\" \"2\")]").unwrap(), strings(&["2"]));
    assert_eq!(eval("[shnex:max (10 \"11\" 2)]").unwrap(), strings(&["11"]));
    assert_eq!(eval("[shnex:min (10 \"11\" 2)]").unwrap(), ints(&[2]));
}

#[test]
fn numeric_order_refines_float_ties_and_keeps_large_lexicals_exact() {
    let actual = eval("[shnex:nodes (9007199254740993 9007199254740992e0 9007199254740992); shnex:orderBy [shnex:var \"focusNode\"]]").unwrap();
    assert_eq!(actual.last(), ints(&[9007199254740993]).first());
    let got = eval("[shnex:min (100000000000000000000000000000000000000000000000001 100000000000000000000000000000000000000000000000000)]").unwrap();
    assert!(
        matches!(&got[0], Term::Literal(l) if l.value()=="100000000000000000000000000000000000000000000000000")
    );
}

#[test]
fn sum_preserves_exact_digits_and_distinguishes_capacity_from_failure() {
    assert_eq!(
        eval("[shnex:sum (9007199254740992 1)]").unwrap(),
        ints(&[9007199254740993])
    );
    assert_eq!(
        eval("[shnex:sum (340282366920938463463374607431768211456000 1)]").unwrap(),
        vec![]
    );
    assert!(eval("[shnex:sum [shnex:instancesOf 42]]").is_err());
}

#[test]
fn numeric_extrema_and_sort_refine_values_beyond_the_float_range() {
    let low = format!("1{}", "0".repeat(400));
    let high = format!("1{}1", "0".repeat(399));
    let integer = |value: &str| -> Term {
        Literal::new_typed_literal(
            value,
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        )
        .into()
    };
    let double = |value: &str| -> Term {
        Literal::new_typed_literal(
            value,
            NamedNode::new("http://www.w3.org/2001/XMLSchema#double").unwrap(),
        )
        .into()
    };
    let values =
        format!(r#"{high} "INF"^^xsd:double {low} "-INF"^^xsd:double "NaN"^^xsd:double -{low}"#);
    let expected = vec![
        double("NaN"),
        double("-INF"),
        integer(&format!("-{low}")),
        integer(&low),
        integer(&high),
        double("INF"),
    ];
    assert_eq!(
        eval(&format!(
            "[shnex:nodes ({values}); shnex:orderBy [shnex:var \"focusNode\"]]"
        ))
        .unwrap(),
        expected
    );
    assert_eq!(
        eval(&format!("[shnex:min ({high} {low})]")).unwrap(),
        vec![integer(&low)]
    );
    assert_eq!(
        eval(&format!("[shnex:max ({low} {high})]")).unwrap(),
        vec![integer(&high)]
    );
    assert_eq!(
        eval(&format!("[shnex:min ({low} \"INF\"^^xsd:double)]")).unwrap(),
        vec![integer(&low)]
    );
    assert_eq!(
        eval(&format!("[shnex:max (-{low} \"-INF\"^^xsd:double)]")).unwrap(),
        vec![integer(&format!("-{low}"))]
    );
}

#[test]
fn temporal_order_uses_exact_values_and_a_total_timezone_order() {
    let actual = eval(r#"[shnex:nodes ("2000-01-01T00:30:00"^^xsd:dateTime "2000-01-01T00:00:00Z"^^xsd:dateTime "2000-01-01T01:00:00+01:00"^^xsd:dateTime); shnex:orderBy [shnex:var "focusNode"]]"#).unwrap();
    assert!(matches!(actual.last(), Some(Term::Literal(l)) if l.value()=="2000-01-01T00:30:00"));
    let actual = eval(r#"[shnex:min ("2000-01-01T00:00:00.0000000000000002Z"^^xsd:dateTime "2000-01-01T00:00:00.0000000000000001Z"^^xsd:dateTime)]"#).unwrap();
    assert!(
        matches!(&actual[0], Term::Literal(l) if l.value()=="2000-01-01T00:00:00.0000000000000001Z")
    );
}

#[test]
fn review_list_identity_preserves_evaluated_nodes() {
    let data = graph("ex:focus ex:value [] .");
    let shapes = graph(&format!(
        "{FUNCTIONS} ex:Decl ex:expr [ex:echo [shnex:pathValues ex:value]] ."
    ));
    let blank = GraphView::new(&data)
        .object(&iri("focus"), "urn:ex:value")
        .unwrap();
    assert_eq!(run(&data, &shapes, &Scope::default()).unwrap(), vec![blank]);
    let shapes = graph(&format!(
        "{FUNCTIONS} ex:Decl ex:expr [ex:echo [shnex:var \"input\"]]; ex:template [shnex:count (1 2)] ."
    ));
    let template = GraphView::new(&shapes)
        .object(&iri("Decl"), "urn:ex:template")
        .unwrap();
    for value in [
        template,
        NamedNode::new("http://www.w3.org/ns/shacl#this")
            .unwrap()
            .into(),
    ] {
        let mut scope = Scope::default();
        scope.insert("input".into(), vec![value.clone()]);
        assert_eq!(run(&data, &shapes, &scope).unwrap(), vec![value]);
    }
}

#[test]
fn review_sparql_callback_arguments_are_evaluated_values() {
    let shapes = graph(&format!(
        r#"{FUNCTIONS}
        ex:S sh:targetNode ex:focus; sh:sparql [sh:select
          "SELECT $this WHERE {{ BIND(<urn:ex:echo>(<http://www.w3.org/ns/shacl#this>) AS ?value) FILTER(!BOUND(?value) || ?value != <http://www.w3.org/ns/shacl#this>) }}"] .
    "#
    ));
    assert!(
        sparq_shacl::validate_strict(&graph(""), &shapes)
            .unwrap()
            .conforms
    );
}

#[test]
fn review_sparql_callback_blank_arguments_remain_errors() {
    let shapes = graph(&format!(
        r#"{FUNCTIONS}
        ex:S sh:targetNode ex:focus; sh:sparql [sh:select
          "SELECT $this WHERE {{ $this <urn:ex:value> ?blank BIND(<urn:ex:echo>(7) AS ?normal) BIND(<urn:ex:echo>(?blank) AS ?value) FILTER(?normal = 7 && !BOUND(?value)) }}"] .
    "#
    ));
    let report = sparq_shacl::validate_strict(&graph("ex:focus ex:value [] ."), &shapes).unwrap();
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].focus_node, iri("focus"));
}

#[test]
#[cfg(feature = "shacl-af")]
fn review_custom_body_shapes_work_in_constraints_and_rules() {
    let function = r#"
        ex:check a sh:ListParameterExpressionFunction; rdfs:subClassOf sh:ListParameterExpression;
          sh:bodyExpression [shnex:matchAll [sh:minInclusive 3]; shnex:nodes [shnex:arg 0]] .
    "#;
    let shapes = graph(&format!(
        r#"{function}
        ex:S sh:targetNode ex:good; sh:expression [ex:check 5] .
        ex:T sh:targetNode ex:bad; sh:expression [ex:check 2] .
    "#
    ));
    let report = sparq_shacl::validate_strict(&graph(""), &shapes).unwrap();
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].focus_node, iri("bad"));
    let shapes = graph(&format!(
        r#"{function}
        ex:S sh:targetNode ex:focus; sh:rule [a sh:TripleRule; sh:subject sh:this;
          sh:predicate ex:result; sh:object [ex:check 5]] .
    "#
    ));
    assert_eq!(
        sparq_shacl::apply_rules(&graph(""), &shapes).triples,
        vec![oxrdf::Triple::new(
            NamedNode::new("urn:ex:focus").unwrap(),
            NamedNode::new("urn:ex:result").unwrap(),
            Literal::from(true)
        )]
    );
}

#[test]
fn review_conformance_resolves_untyped_dynamic_shapes() {
    let shapes = graph(
        r#"
        ex:Valid sh:nodeKind sh:IRI .
        ex:Invalid sh:nodeKind sh:Literal .
        ex:Custom <http://example.org/requireIRI> true .
        ex:C a sh:ConstraintComponent; sh:parameter [sh:path <http://example.org/requireIRI>];
          sh:validator [a sh:SPARQLAskValidator; sh:ask "ASK { FILTER(isIRI($this)) }"] .
        ex:Decl ex:expr [shnex:conformsToShape ([shnex:var "focusNode"] [shnex:var "shape"])] .
    "#,
    );
    for (shape, expected) in [("Valid", true), ("Invalid", false), ("Custom", true)] {
        let mut scope = Scope::default();
        scope.insert("shape".into(), vec![iri(shape)]);
        assert_eq!(
            run(&graph(""), &shapes, &scope).unwrap(),
            vec![Literal::from(expected).into()]
        );
    }
}

#[test]
fn review_conformance_requires_a_shape_iri() {
    let shapes = graph(
        r#"
        ex:Decl ex:shape [a sh:NodeShape; sh:nodeKind sh:IRI];
          ex:expr [shnex:conformsToShape ([shnex:var "focusNode"] [shnex:var "shape"])] .
    "#,
    );
    let shape = GraphView::new(&shapes)
        .object(&iri("Decl"), "urn:ex:shape")
        .unwrap();
    let mut scope = Scope::default();
    scope.insert("shape".into(), vec![shape]);
    assert!(run(&graph(""), &shapes, &scope).is_err());
}

#[test]
#[cfg(feature = "shacl-af")]
fn review_legacy_rule_select_calls_expression_functions() {
    let shapes = graph(&format!(
        r#"{FUNCTIONS}
        ex:S sh:targetNode ex:focus; sh:rule [a sh:TripleRule; sh:subject sh:this;
          sh:predicate ex:result; sh:object [sh:select "SELECT (<urn:ex:echo>(7) AS ?v) WHERE {{}}"]] .
    "#
    ));
    assert_eq!(
        sparq_shacl::apply_rules(&graph(""), &shapes).triples,
        vec![oxrdf::Triple::new(
            NamedNode::new("urn:ex:focus").unwrap(),
            NamedNode::new("urn:ex:result").unwrap(),
            Literal::from(7)
        )]
    );
}

#[test]
fn review_conformance_does_not_promote_prefix_metadata_to_a_shape() {
    let shapes = graph(
        r#"
        ex:Prefixes sh:declare [sh:prefix "ex"; sh:namespace "urn:ex:"] .
        ex:Decl ex:expr [shnex:conformsToShape ([shnex:var "focusNode"] ex:Prefixes)] .
    "#,
    );
    assert!(run(&graph(""), &shapes, &Scope::default()).is_err());
}
