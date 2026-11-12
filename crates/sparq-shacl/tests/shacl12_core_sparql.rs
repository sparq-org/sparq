//! Behavioural guards for the SHACL 1.2 Core and SPARQL rules at the suite pin.
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_shacl::{validate, validate_strict, validate_strict_shacl10};

const PREFIXES: &str = r#"
@prefix ex: <urn:ex:> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;
fn graph(text: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}\n{text}"), "turtle").unwrap()
}
fn iri(local: &str) -> Term {
    NamedNode::new(format!("urn:ex:{local}")).unwrap().into()
}
fn constraint(body: &str) -> Graph {
    graph(&format!(
        r#"ex:S a sh:NodeShape; sh:targetNode ex:a; sh:sparql [sh:select """{body}"""] ."#
    ))
}

#[test]
fn root_class_lists_are_disjunctive_and_require_iris() {
    let g = graph(
        r#"
ex:S sh:targetNode ex:A, ex:Child, ex:B, ex:Other, "A";
    sh:rootClass (ex:A ex:B) .
ex:Child rdfs:subClassOf ex:Middle . ex:Middle rdfs:subClassOf ex:A .
"#,
    );
    let r = validate_strict(&g, &g).unwrap();
    let values: Vec<_> = r.results.iter().map(|r| r.value.clone().unwrap()).collect();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&iri("Other")));
    assert!(values.contains(&Literal::new_simple_literal("A").into()));
}

#[test]
fn uniqueness_reports_once_without_a_value_and_ignores_empty_keys() {
    let g = graph(
        r#"
ex:S sh:targetNode ex:a, ex:b, ex:c, ex:d, ex:e; sh:uniqueValuesFor (ex:key ex:scope) .
ex:a ex:key "same"; ex:scope ex:one . ex:b ex:key "same"; ex:scope ex:one .
ex:c ex:key "same"; ex:scope ex:one . ex:d ex:key "same"; ex:scope ex:two .
"#,
    );
    let r = validate(&g, &g);
    assert_eq!(r.results.len(), 3);
    for f in ["a", "b", "c"] {
        assert_eq!(
            r.results.iter().filter(|r| r.focus_node == iri(f)).count(),
            1
        );
    }
    assert!(r.results.iter().all(|r| r.value.is_none()));
}

#[test]
fn values_union_is_deduplicated_and_default_is_only_for_an_empty_union() {
    let data = graph("ex:a ex:p 1, 2 .");
    for expression in ["2", "[sh:select \"SELECT (2 AS ?value) WHERE {}\"]"] {
        let shapes = graph(&format!(
            r#"
ex:S sh:targetNode ex:a, ex:b; sh:path ex:p; sh:values {expression};
    sh:defaultValue 99; sh:minCount 1; sh:maxCount 1; sh:in (1 2) .
"#
        ));
        let r = validate_strict(&data, &shapes).unwrap();
        assert_eq!(r.results.len(), 1, "{expression}");
        assert_eq!(r.results[0].focus_node, iri("a"));
        assert!(r.results[0]
            .source_component
            .ends_with("MaxCountConstraintComponent"));
    }
    let defaults =
        graph("ex:S sh:targetNode ex:a, ex:b; sh:path ex:p; sh:defaultValue 99; sh:in (1 2) .");
    let r = validate_strict(&data, &defaults).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].focus_node, iri("b"));
    assert_eq!(r.results[0].value, Some(Literal::from(99).into()));
}

#[test]
fn triple_node_kind_and_each_nonconforming_reifier() {
    let g = graph(
        r#"
ex:a ex:p 7 .
ex:r1 rdf:reifies <<(ex:a ex:p 7)>> .
ex:r2 rdf:reifies <<(ex:a ex:p 7)>> .
ex:good rdf:reifies <<(ex:a ex:p 7)>>; ex:source ex:doc .
ex:PS sh:targetNode ex:a; sh:path ex:p; sh:reifierShape [sh:property [sh:path ex:source; sh:minCount 1]] .
ex:TS sh:targetNode ex:r1; sh:path rdf:reifies; sh:nodeKind sh:TripleTerm .
"#,
    );
    let r = validate_strict(&g, &g).unwrap();
    assert_eq!(r.results.len(), 2);
    assert!(r
        .results
        .iter()
        .all(|r| r.source_shape == iri("PS") && r.value == Some(Literal::from(7).into())));
}

#[test]
fn blank_focus_identity_survives_optional_exists_union_and_paths() {
    let data = graph("ex:root ex:focus _:bad, _:good . _:good ex:p ex:value .");
    for body in [
        "OPTIONAL { $this <urn:ex:p> ?v } FILTER(!BOUND(?v))",
        "OPTIONAL { $this <urn:ex:p> ?v FILTER EXISTS { FILTER(isBlank($this)) } } FILTER(!BOUND(?v))",
        "FILTER NOT EXISTS { $this <urn:ex:p> ?v }",
        "FILTER EXISTS { OPTIONAL { $this <urn:ex:p> ?v } FILTER(!BOUND(?v)) }",
        "{ FILTER(false) } UNION { OPTIONAL { $this <urn:ex:p> ?v } FILTER(!BOUND(?v)) }",
        "OPTIONAL { $this <urn:ex:p>/<urn:ex:q>? ?v } FILTER(!BOUND(?v))",
    ] {
        let shapes = graph(&format!(
            r#"ex:S sh:targetObjectsOf ex:focus; sh:sparql [sh:select """
SELECT $this WHERE {{ {body} FILTER(isBlank($this)) }}"""] ."#
        ));
        let r = validate_strict(&data, &shapes).unwrap();
        assert_eq!(r.results.len(), 1, "{body}");
        let expected = sparq_shacl::view::GraphView::new(&data)
            .objects(&iri("root"), "urn:ex:focus")
            .into_iter()
            .find(|n| {
                sparq_shacl::view::GraphView::new(&data)
                    .objects(n, "urn:ex:p")
                    .is_empty()
            })
            .unwrap();
        assert_eq!(r.results[0].focus_node, expected, "{body}");
        assert_eq!(r.results[0].value, Some(expected), "{body}");
    }
    let data = graph(
        r#"
ex:root ex:focus <<( _:s ex:p ex:bad )>>, <<( _:s ex:p ex:good )>> .
ex:good ex:about <<( _:s ex:p ex:good )>>; ex:present true .
"#,
    );
    let shapes = graph(
        r#"ex:S sh:targetObjectsOf ex:focus; sh:sparql [sh:select """
SELECT $this WHERE { OPTIONAL { ?record <urn:ex:about> $this; <urn:ex:present> ?v } FILTER(!BOUND(?v)) }
"""] ."#,
    );
    let r = validate_strict(&data, &shapes).unwrap();
    let expected = sparq_shacl::view::GraphView::new(&data)
        .objects(&iri("root"), "urn:ex:focus")
        .into_iter()
        .find(|n| matches!(n, Term::Triple(t) if t.object == iri("bad")))
        .unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].focus_node, expected);
    assert_eq!(r.results[0].value, Some(expected));
}

#[test]
fn nested_subquery_limits_apply_per_focus_and_hidden_bindings_are_allowed() {
    let data = graph("ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:q 3 .");
    for projection in ["$this ?v", "?v"] {
        let shapes = graph(&format!(
            r#"ex:S sh:targetNode ex:a, ex:b, ex:c; sh:sparql [sh:select """
SELECT $this WHERE {{ OPTIONAL {{ {{ SELECT {projection} WHERE {{ $this <urn:ex:p> ?v }} LIMIT 1 }} }} FILTER(!BOUND(?v)) }}"""] ."#
        ));
        let r = validate_strict(&data, &shapes).unwrap();
        assert_eq!(r.results.len(), 1);
        assert_eq!(r.results[0].focus_node, iri("c"));
    }
}

#[test]
fn admission_rules_are_version_specific_and_apply_inside_exists() {
    let data = graph("");
    let shapes = constraint("SELECT $this WHERE { VALUES ?local { 1 } FILTER(?local = 1) }");
    assert_eq!(validate_strict(&data, &shapes).unwrap().results.len(), 1);
    assert!(validate_strict_shacl10(&data, &shapes).is_err());
    let shapes = constraint("SELECT $this WHERE { {SELECT ?v WHERE {$this <urn:ex:p> ?v}} }");
    assert!(validate_strict(&data, &shapes).is_ok());
    assert!(validate_strict_shacl10(&data, &shapes).is_err());
    for body in [
        "FILTER EXISTS { VALUES ?this { <urn:ex:a> } }",
        "FILTER NOT EXISTS { ?s ?p ?o MINUS { ?s ?p 1 } }",
        "FILTER EXISTS { BIND(1 AS $this) }",
        "FILTER EXISTS { SERVICE <urn:remote> { ?s ?p ?o } }",
    ] {
        assert!(
            validate_strict(
                &data,
                &constraint(&format!("SELECT $this WHERE {{ {body} }}"))
            )
            .is_err(),
            "{body}"
        );
    }
    for version in ["", "VERSION '1.2'"] {
        let shapes = graph(&format!(
            r#"
ex:prefixes sh:declare [sh:prefix "ex"; sh:namespace "urn:ex:"] .
ex:C a sh:ConstraintComponent; sh:parameter [sh:path ex:flag];
 sh:validator [sh:prefixes ex:prefixes; sh:select """{version} SELECT $this WHERE {{
 $this ex:p ?v MINUS {{ $this ex:p 1 }}
 }}"""] .
ex:S sh:targetNode ex:a; ex:flag true .
"#
        ));
        let failure = validate_strict(&data, &shapes).unwrap_err();
        assert_eq!(failure.pre_binding.len(), 1, "{version}");
    }
}

#[test]
fn declared_failure_is_distinct_from_a_report_and_false_is_a_result() {
    let data = graph("");
    for boolean in ["true", "1"] {
        let shapes = constraint(&format!("SELECT $this (\"{boolean}\"^^<http://www.w3.org/2001/XMLSchema#boolean> AS ?failure) WHERE {{}}"));
        let failure = validate_strict(&data, &shapes).unwrap_err();
        assert_eq!(failure.pre_binding.len(), 1);
        let r = validate(&data, &shapes);
        assert!(r.results.is_empty());
        assert_eq!(r.diagnostics.len(), 1);
    }
    let shapes = constraint("SELECT $this (false AS ?failure) WHERE {}");
    assert_eq!(validate_strict(&data, &shapes).unwrap().results.len(), 1);
    // The runtime failure context must not leak into a later validation.
    assert!(validate_strict(&data, &graph("")).is_ok());
}

#[test]
fn query_message_takes_priority_and_preserves_language_and_substitution() {
    let shapes = graph(
        r#"ex:S sh:targetNode ex:a; sh:message "shape"; sh:sparql [
sh:message "constraint"@en; sh:select """SELECT $this ?message ?v WHERE { BIND(7 AS ?v) BIND("row {?v}"@fr AS ?message) }"""] ."#,
    );
    let r = validate_strict(&graph(""), &shapes).unwrap();
    assert_eq!(
        r.results[0].effective_messages(),
        vec![Literal::new_language_tagged_literal("row 7", "fr")
            .unwrap()
            .into()]
    );
}

#[test]
fn prefix_defaults_version_iris_and_import_cycles() {
    let data = graph("ex:a ex:p 1 .");
    for prefixes in [
        "",
        "sh:prefixes ex:version;",
        "sh:prefixes ex:importVersion;",
    ] {
        let shapes = graph(&format!(
            r#"
ex:ontology a owl:Ontology; owl:versionIRI ex:version; sh:declare [sh:prefix "ex"; sh:namespace "urn:ex:"] .
ex:imports owl:versionIRI ex:importVersion; owl:imports ex:version, ex:imports .
ex:S sh:targetNode ex:a; sh:sparql [{prefixes} sh:select "SELECT $this WHERE {{ $this ex:p 1 }}"] .
"#
        ));
        assert_eq!(
            validate_strict(&data, &shapes).unwrap().results.len(),
            1,
            "{prefixes}"
        );
    }
}

#[test]
fn shapes_graph_and_current_shape_are_prebound() {
    let shapes = graph(
        r#"ex:S sh:targetNode ex:a; sh:sparql [sh:select """
SELECT $this WHERE { GRAPH $shapesGraph { $currentShape <http://www.w3.org/ns/shacl#targetNode> $this } }"""] ."#,
    );
    let r = validate_strict(&graph(""), &shapes).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].focus_node, iri("a"));
    let shapes = constraint("SELECT $this WHERE { FILTER(BOUND($currentShape) && BOUND($shapesGraph) && $currentShape = <urn:ex:S>) }");
    assert_eq!(
        validate_strict(&graph(""), &shapes).unwrap().results.len(),
        1
    );
}

#[test]
fn custom_functions_evaluate_bodies_and_enforce_single_output() {
    let g = graph(
        r#"
ex:count a sh:ListParameterExpressionFunction;
    sh:bodyExpression [shnex:count [shnex:instancesOf [shnex:arg 0]]] .
ex:one a sh:ListParameterExpressionFunction; sh:bodyExpression [sh:select "SELECT ?v WHERE {?v a $arg0}"] .
ex:A rdfs:subClassOf ex:Root . ex:a a ex:A, ex:Root . ex:b a ex:Root . ex:c a ex:Single .
ex:S sh:targetNode ex:a; sh:sparql [sh:select """
SELECT $this ?value WHERE {
 BIND(<urn:ex:count>(<urn:ex:Root>) AS ?count)
 BIND(<urn:ex:one>(<urn:ex:Single>) AS ?one)
 BIND(<urn:ex:one>(<urn:ex:Root>) AS ?many)
 BIND(<urn:ex:one>(<urn:ex:Missing>) AS ?none)
 BIND(CONCAT(STR(?count), ' ', STR(?one), ' ', STR(BOUND(?many)), ' ', STR(BOUND(?none))) AS ?value)
}"""] .
"#,
    );
    let r = validate_strict(&g, &g).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(
        r.results[0].value,
        Some(Literal::new_simple_literal("2 urn:ex:c false false").into())
    );
}

#[test]
fn deactivation_and_default_expressions_use_the_original_focus() {
    let data = graph("ex:a ex:p 2 . ex:b ex:default 3 .");
    let shapes = graph(
        r#"
ex:S sh:targetNode ex:a, ex:b; sh:path ex:p; sh:minInclusive 4;
    sh:defaultValue [sh:select "SELECT ?v WHERE {$this <urn:ex:default> ?v}"];
    sh:deactivated [sh:select "SELECT (EXISTS {$this <urn:ex:p> 2} AS ?off) WHERE {}"] .
"#,
    );
    let r = validate_strict(&data, &shapes).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].focus_node, iri("b"));
    assert_eq!(r.results[0].value, Some(Literal::from(3).into()));
}

#[cfg(feature = "shacl-af")]
#[test]
fn expression_constraints_bind_value_and_keep_the_original_focus() {
    let g = graph(
        r#"
ex:a ex:p "yes", "no"; ex:allowed "yes" .
ex:S sh:targetNode ex:a; sh:path ex:p; sh:expression [shnex:exists [shnex:intersection (
    [shnex:var "value"] [shnex:pathValues ex:allowed]
)]] .
"#,
    );
    let r = validate_strict(&g, &g).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(
        r.results[0].value,
        Some(Literal::new_simple_literal("no").into())
    );
}

#[test]
fn component_messages_preserve_language_and_override_shape_messages() {
    for (validator, expected) in [
        ("sh:ask \"ASK { FILTER(false) }\"", "component"),
        ("sh:select \"SELECT $this ?value WHERE { BIND(9 AS ?value) }\"; sh:message \"validator\"@en", "validator"),
        ("sh:select \"SELECT $this ?message WHERE { BIND('row'@en AS ?message) }\"; sh:message \"validator\"@en", "row"),
    ] {
        let shapes = graph(&format!(r#"
ex:C a sh:ConstraintComponent; sh:parameter [sh:path ex:flag];
 sh:message "component"@en; sh:validator [{validator}] .
ex:S sh:targetNode ex:a; sh:message "shape"; ex:flag true .
"#));
        let r = validate_strict(&graph(""), &shapes).unwrap();
        assert_eq!(r.results.len(), 1);
        assert_eq!(r.results[0].effective_messages(), vec![Literal::new_language_tagged_literal(expected, "en").unwrap().into()]);
    }
}

#[test]
fn custom_function_blank_missing_and_recursive_arguments_are_errors() {
    let g = graph(
        r#"
ex:identity a sh:ListParameterExpressionFunction; sh:parameter [sh:path shnex:arg0];
 sh:bodyExpression [sh:sparqlExpr "$arg0"] .
ex:recursive a sh:ListParameterExpressionFunction; sh:bodyExpression [sh:sparqlExpr "<urn:ex:recursive>()"] .
ex:a ex:p _:value .
ex:S sh:targetNode ex:a; sh:sparql [sh:select """
SELECT $this ?value WHERE {
 $this <urn:ex:p> ?blank
 BIND(<urn:ex:identity>(?blank) AS ?bad)
 BIND(<urn:ex:identity>() AS ?missing)
 BIND(<urn:ex:recursive>() AS ?recursive)
 BIND(<urn:ex:identity>(<urn:ex:ok>) AS ?ok)
 BIND(CONCAT(STR(BOUND(?bad)), ' ', STR(BOUND(?missing)), ' ', STR(BOUND(?recursive)), ' ', STR(?ok)) AS ?value)
}"""] .
"#,
    );
    let r = validate_strict(&g, &g).unwrap();
    assert_eq!(r.results.len(), 1);
    assert_eq!(
        r.results[0].value,
        Some(Literal::new_simple_literal("false false false urn:ex:ok").into())
    );
}
