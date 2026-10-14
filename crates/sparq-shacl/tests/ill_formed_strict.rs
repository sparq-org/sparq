//! (sq-11a) Ill-formed shapes graphs are reported as a FAILURE by the
//! strict channel — the W3C test-suite `sht:Failure` outcome — instead of only
//! being skipped leniently.
//!
//! Two-sided pinning:
//!   * every known ill-formed construct makes `validate_strict` return
//!     `Err(ShaclFailure)` carrying an [`sparq_shacl::IllFormedConstruct`] whose
//!     `predicate` names the offending SHACL predicate, while the lenient
//!     `validate` still produces a report (unchanged policy);
//!   * well-formed edge cases (a negative `sh:minCount`, `sh:closed false`, an
//!     EMPTY `sh:in ()` list, the SHACL-1.2 disjunctive `sh:datatype` list, …)
//!     must NOT be flagged — the false-positive guard.
//!
//! The W3C core suite contains no ill-formed-shapes `sht:Failure` entries, so
//! these fixtures are hand-crafted (the suite's Failure entries all live in
//! `sparql/pre-binding/`, pinned by `w3c_sparql_shacl12.rs`).

use sparq_core::Graph;
use sparq_shacl::{ShaclFailure, ShapesModel, ValidationReport};

const PREFIXES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;

const DATA: &str = "ex:n ex:p 1 . ex:n a ex:T .";

fn shapes_graph(shapes: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{shapes}"), "turtle").unwrap()
}

fn run_strict(shapes: &str) -> Result<ValidationReport, ShaclFailure> {
    let data = Graph::load_str(&format!("{PREFIXES}{DATA}"), "turtle").unwrap();
    sparq_shacl::validate_strict(&data, &shapes_graph(shapes))
}

fn run_lenient(shapes: &str) -> ValidationReport {
    let data = Graph::load_str(&format!("{PREFIXES}{DATA}"), "turtle").unwrap();
    sparq_shacl::validate(&data, &shapes_graph(shapes))
}

/// Asserts the shapes graph is rejected by the strict channel with an
/// [`sparq_shacl::IllFormedConstruct`] on the expected SHACL predicate — and
/// that the lenient `validate` still produces a report for the same graph.
fn assert_ill_formed(shapes: &str, expect_predicate_local: &str) {
    let err = match run_strict(shapes) {
        Err(e) => e,
        Ok(_) => panic!(
            "expected sht:Failure (strict rejection) for sh:{}, got a report:\n{}",
            expect_predicate_local, shapes
        ),
    };
    let expected = format!("http://www.w3.org/ns/shacl#{}", expect_predicate_local);
    assert!(
        err.ill_formed.iter().any(|i| i.predicate == expected),
        "expected an ill-formed record on {}, got {:?}",
        expected,
        err.ill_formed
    );
    assert!(
        err.pre_binding.is_empty(),
        "these fixtures are not pre-binding violations"
    );
    // The lenient channel is UNCHANGED: same graph, still an infallible report.
    let _report = run_lenient(shapes);
}

fn assert_well_formed(shapes: &str) {
    if let Err(e) = run_strict(shapes) {
        panic!("well-formed shapes graph wrongly rejected: {}\n{}", e, shapes);
    }
}

// ---- ill-formed constructs → sht:Failure (strict) ----

#[test]
fn ill_formed_path_expression_fails() {
    // A literal is never a path.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path "not-a-path" ; sh:minCount 1 ] ."#,
        "path",
    );
}

#[test]
fn multiple_path_values_fail() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:path ex:q ; sh:minCount 1 ] ."#,
        "path",
    );
}

#[test]
fn non_integer_min_count_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:minCount "three" ] ."#,
        "minCount",
    );
}

#[test]
fn iri_valued_max_length_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:maxLength ex:big ."#,
        "maxLength",
    );
}

#[test]
fn literal_datatype_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:datatype "xsd:integer" ."#,
        "datatype",
    );
}

#[test]
fn literal_member_in_node_kind_list_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:nodeKind ( sh:IRI "sh:Literal" ) ."#,
        "nodeKind",
    );
}

#[test]
fn literal_class_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:class "ex:T" ."#,
        "class",
    );
}

#[test]
fn non_literal_pattern_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:pattern ex:regex ."#,
        "pattern",
    );
}

#[test]
fn non_list_language_in_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:languageIn "en" ."#,
        "languageIn",
    );
}

#[test]
fn literal_comparand_for_less_than_fails() {
    // The sh:lessThan comparand must be a predicate IRI / path, never a literal.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:lessThan "ex:q" ] ."#,
        "lessThan",
    );
}

#[test]
fn non_list_sh_in_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:in ex:notAList ."#,
        "in",
    );
}

#[test]
fn malformed_sh_in_list_tail_fails() {
    // _:l has rdf:first but no rdf:rest: the lenient list truncates, the strict
    // well-formedness check rejects.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:in _:l .
           _:l rdf:first ex:a ."#,
        "in",
    );
}

#[test]
fn non_list_sh_or_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:or ex:notAList ."#,
        "or",
    );
}

#[test]
fn literal_shape_in_sh_and_list_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:and ( "not-a-shape" ) ."#,
        "and",
    );
}

#[test]
fn literal_sh_node_shape_ref_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:node "ex:Other" ."#,
        "node",
    );
}

#[test]
fn literal_sh_property_shape_ref_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:property "ex:P" ."#,
        "property",
    );
}

#[test]
fn literal_target_class_fails() {
    assert_ill_formed(r#"ex:S a sh:NodeShape ; sh:targetClass "ex:T" ."#, "targetClass");
}

#[test]
fn literal_target_subjects_of_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetSubjectsOf "ex:p" ."#,
        "targetSubjectsOf",
    );
}

#[test]
fn non_boolean_unique_lang_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:uniqueLang "yes" ] ."#,
        "uniqueLang",
    );
}

#[test]
fn non_boolean_closed_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:closed "shut" ."#,
        "closed",
    );
}

#[test]
fn malformed_ignored_properties_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:closed true ;
             sh:ignoredProperties ex:notAList ."#,
        "ignoredProperties",
    );
}

#[test]
fn non_integer_qualified_min_count_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ;
               sh:qualifiedValueShape [ sh:datatype xsd:integer ] ;
               sh:qualifiedMinCount "one" ] ."#,
        "qualifiedMinCount",
    );
}

#[test]
fn literal_member_shape_ref_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:memberShape "ex:M" ."#,
        "memberShape",
    );
}

#[test]
fn literal_some_value_shape_ref_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:someValue "ex:M" ."#,
        "someValue",
    );
}

#[test]
fn literal_reifier_shape_ref_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:reifierShape "ex:M" ."#,
        "reifierShape",
    );
}

#[test]
fn literal_qualified_value_shape_ref_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:qualifiedValueShape "ex:M" ;
                           sh:qualifiedMinCount 1 ] ."#,
        "qualifiedValueShape",
    );
}

#[test]
fn non_list_sh_xone_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:xone "not-a-list" ."#,
        "xone",
    );
}

#[test]
fn literal_target_objects_of_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetObjectsOf "ex:p" ."#,
        "targetObjectsOf",
    );
}

#[test]
fn literal_unique_values_for_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:uniqueValuesFor "ex:key" ] ."#,
        "uniqueValuesFor",
    );
}

#[test]
fn non_literal_min_exclusive_comparand_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:minExclusive ex:zero ."#,
        "minExclusive",
    );
}

#[test]
fn non_literal_flags_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:pattern "^a" ; sh:flags ex:i ."#,
        "flags",
    );
}

#[test]
fn iri_member_in_language_in_list_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:languageIn ( "en" ex:fr ) ."#,
        "languageIn",
    );
}

#[test]
fn non_integer_qualified_max_count_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ;
               sh:qualifiedValueShape [ sh:datatype xsd:integer ] ;
               sh:qualifiedMaxCount ex:five ] ."#,
        "qualifiedMaxCount",
    );
}

#[test]
fn ill_formed_equals_comparand_path_fails() {
    // A cyclic blank-node path expression is unparsable.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:equals _:c ] .
           _:c sh:inversePath _:c ."#,
        "equals",
    );
}

#[test]
fn sparql_constraint_without_select_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:sparql [ sh:message "no query here" ] ."#,
        "select",
    );
}

// ---- parser-dependent rejections (sq-ehq4g): a PRESENT query that does not
// ---- parse. Fail-closed relative to THIS engine's SPARQL parser (README).

#[test]
fn unparsable_sparql_constraint_select_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:sparql [ sh:select "SELECT ??? garbage {" ] ."#,
        "select",
    );
}

#[test]
fn non_select_sparql_constraint_query_fails() {
    // A parsable but non-SELECT query (ASK) — sh:select requires a SELECT.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:sparql [ sh:select "ASK { ?s ?p ?o }" ] ."#,
        "select",
    );
}

#[test]
fn unparsable_target_node_select_expression_fails() {
    // A SHACL-1.2 SPARQL-based node-expression target whose query is unparsable.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode [ sh:select "not sparql at all" ] ."#,
        "select",
    );
}

#[test]
fn unparsable_values_sparql_expr_fails() {
    // A SHACL-1.2 `sh:values [ sh:sparqlExpr … ]` whose expression is unparsable.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:values [ sh:sparqlExpr "((" ] ] ."#,
        "sparqlExpr",
    );
}

// ---- remaining non-construct-local syntax rules (sq-ehq4g) ----

#[test]
fn property_shape_missing_path_fails() {
    // The value of sh:property must be a property shape, i.e. carry an sh:path.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:minCount 1 ] ."#,
        "path",
    );
}

#[test]
fn typed_property_shape_missing_path_fails() {
    // An explicitly-typed sh:PropertyShape root with no sh:path.
    assert_ill_formed(r#"ex:P a sh:PropertyShape ; sh:targetNode ex:n ."#, "path");
}

#[test]
fn qualified_value_shape_without_counts_fails() {
    // sh:qualifiedValueShape with NEITHER sh:qualifiedMinCount nor
    // sh:qualifiedMaxCount: both qualified-cardinality components are missing a
    // mandatory parameter (SHACL §4.7.5–6 partial-parameter ill-formedness).
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ;
               sh:qualifiedValueShape [ sh:nodeKind sh:IRI ] ] ."#,
        "qualifiedValueShape",
    );
}

#[test]
fn unknown_node_kind_fails() {
    // Not one of the six sh:* node kinds (SHACL §4.6.1).
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:nodeKind ex:SomeKind ."#,
        "nodeKind",
    );
}

#[test]
fn unknown_node_kind_in_list_fails() {
    // sh:Iri (wrong case) is in the sh: namespace but is NOT a node kind.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:nodeKind ( sh:IRI sh:Iri ) ."#,
        "nodeKind",
    );
}

// ---- remaining syntax rules deferred from sq-ehq4g (sq-c1v3e) ----

#[test]
fn literal_severity_fails() {
    // Syntax rule severity-nodeKind: the value of sh:severity is an IRI.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:severity "high" ."#,
        "severity",
    );
}

#[test]
fn literal_sparql_constraint_severity_fails() {
    // The SHACL-1.2 constraint-level sh:severity (on the sh:SPARQLConstraint
    // node) obeys the same rule.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:sparql [ sh:select "SELECT $this WHERE { $this ?p ?o }" ;
                         sh:severity "warn" ] ."#,
        "severity",
    );
}

#[test]
fn string_typed_min_count_fails() {
    // "3" is integer-LEXICAL but xsd:string-typed; the syntax rules type the
    // counts/lengths as xsd:integer. (Bare Turtle 3 types as xsd:integer.)
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:minCount "3" ] ."#,
        "minCount",
    );
}

#[test]
fn string_typed_max_list_length_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:maxListLength "2" ] ."#,
        "maxListLength",
    );
}

#[test]
fn string_typed_qualified_min_count_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ;
               sh:qualifiedValueShape [ sh:nodeKind sh:IRI ] ;
               sh:qualifiedMinCount "1" ] ."#,
        "qualifiedMinCount",
    );
}

#[test]
fn entailment_declaration_fails() {
    // SHACL §3.4: an unsupported entailment regime MUST be a failure — and this
    // processor supports none, so any sh:entailment declaration is one.
    assert_ill_formed(
        r#"ex:sg sh:entailment <http://www.w3.org/ns/entailment/RDFS> .
           ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:nodeKind sh:IRI ."#,
        "entailment",
    );
}

#[test]
fn qualified_min_count_without_value_shape_fails() {
    // The symmetric partial-parameter case of sq-ehq4g: a qualified count
    // without sh:qualifiedValueShape misses that mandatory parameter.
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:qualifiedMinCount 1 ] ."#,
        "qualifiedMinCount",
    );
}

#[test]
fn qualified_max_count_without_value_shape_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:qualifiedMaxCount 2 ] ."#,
        "qualifiedMaxCount",
    );
}

// ---- shacl-af: structural node expressions that do not build (sq-c1v3e) ----

#[cfg(feature = "shacl-af")]
#[test]
fn unbuildable_expression_constraint_fails() {
    // A blank-node (structural) sh:expression with an unregistered operator IRI
    // was dropped silently; strict now rejects it (fail-closed relative to this
    // engine's node-expression coverage).
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:expression [ ex:noSuchOperator ( ex:a ex:b ) ] ."#,
        "expression",
    );
}

#[cfg(feature = "shacl-af")]
#[test]
fn unbuildable_node_by_expression_fails() {
    assert_ill_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:nodeByExpression [ ex:noSuchOperator ( ex:a ) ] ."#,
        "nodeByExpression",
    );
}

#[cfg(feature = "shacl-af")]
#[test]
fn buildable_expression_constraint_is_not_flagged() {
    // A supported structural expression (a path expression) must not be flagged.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:expression [ sh:path ex:p ] ."#,
    );
}

// ---- well-formed edge cases must NOT be flagged (false-positive guard) ----

#[test]
fn explicitly_typed_integer_count_is_not_flagged() {
    // The canonical typed spelling is exactly what bare Turtle integers carry.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:minCount "1"^^xsd:integer ;
                           sh:maxLength "5"^^xsd:integer ] ."#,
    );
}

#[test]
fn iri_severity_is_not_flagged() {
    // Both a sh:* severity and a custom severity IRI are well-formed.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:severity sh:Warning ;
             sh:property [ sh:path ex:p ; sh:severity ex:MySeverity ] ."#,
    );
}

#[test]
fn well_formed_shapes_graphs_pass_strict() {
    // A representative well-formed graph exercising the instrumented parses.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:targetClass ex:T ;
             sh:targetSubjectsOf ex:p ; sh:closed true ; sh:ignoredProperties ( ex:q ) ;
             sh:in ( 1 2 ) ; sh:or ( [ sh:datatype xsd:integer ] [ sh:datatype xsd:string ] ) ;
             sh:node ex:Other ;
             sh:property [ sh:path ex:p ; sh:minCount 1 ; sh:maxCount 5 ;
                           sh:pattern "^a" ; sh:flags "i" ; sh:uniqueLang true ;
                           sh:lessThan ex:q ;
                           sh:qualifiedValueShape [ sh:datatype xsd:integer ] ;
                           sh:qualifiedMinCount 1 ] .
           ex:Other a sh:NodeShape ; sh:datatype xsd:integer ."#,
    );
}

#[test]
fn negative_min_count_is_not_ill_formed() {
    // A negative count is a WELL-formed xsd:integer (trivially satisfiable);
    // it stays a silent skip, not a failure.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:minCount -1 ] ."#,
    );
}

#[test]
fn closed_false_is_not_ill_formed() {
    assert_well_formed(r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:closed false ."#);
}

#[test]
fn empty_sh_in_list_is_not_ill_formed() {
    // `sh:in ()` is the well-formed EMPTY SHACL list (rdf:nil).
    assert_well_formed(r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:in () ."#);
}

#[test]
fn disjunctive_datatype_list_is_not_ill_formed() {
    // The SHACL-1.2 disjunctive list form.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:datatype ( xsd:integer xsd:string ) ; sh:nodeKind ( sh:IRI sh:Literal ) ."#,
    );
}

#[test]
fn sequence_path_is_not_ill_formed() {
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ( ex:p ex:q ) ; sh:minCount 0 ] ."#,
    );
}

#[test]
fn well_formed_sparql_queries_are_not_flagged() {
    // (sq-ehq4g) Parsable SELECT queries in all three parser-dependent sites:
    // an sh:sparql constraint, a node-expression target and sh:values.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:sparql [ sh:select "SELECT $this ?value WHERE { $this <http://example.org/p> ?value }" ] .
           ex:T2 a sh:NodeShape ;
             sh:targetNode [ sh:select "SELECT ?n WHERE { ?n a <http://example.org/T> }" ] ;
             sh:property [ sh:path ex:p ;
               sh:values [ sh:sparqlExpr "STRLEN(STR($this))" ] ] ."#,
    );
}

#[test]
fn node_shape_without_path_is_not_flagged() {
    // (sq-ehq4g) Only sh:property values / typed sh:PropertyShapes need a path;
    // node shapes referenced via sh:node / sh:targetWhere never do.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:node ex:Other ;
             sh:targetWhere [ sh:nodeKind sh:IRI ] .
           ex:Other a sh:NodeShape ; sh:nodeKind sh:IRI ."#,
    );
}

#[test]
fn qualified_value_shape_with_one_count_is_not_flagged() {
    // (sq-ehq4g) Either count parameter alone is well-formed.
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ;
               sh:qualifiedValueShape [ sh:nodeKind sh:IRI ] ;
               sh:qualifiedMaxCount 3 ] ."#,
    );
}

#[test]
fn all_six_node_kinds_are_not_flagged() {
    assert_well_formed(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:nodeKind ( sh:BlankNode sh:IRI sh:Literal sh:BlankNodeOrIRI
                           sh:BlankNodeOrLiteral sh:IRIOrLiteral ) ."#,
    );
}

#[test]
fn plain_rdfs_class_root_is_not_flagged() {
    // A shapes graph that also carries vocabulary triples (a plain rdfs:Class
    // with no SHACL constructs) must not produce failures.
    assert_well_formed(
        r#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
           ex:T a rdfs:Class ; rdfs:label "a class" .
           ex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:nodeKind sh:IRI ."#,
    );
}

// ---- the recording surface itself ----

#[test]
fn shapes_model_ill_formed_accessor_reports_records() {
    // DIRECT unit test for the new public accessor: node + predicate + message.
    let g = shapes_graph(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
             sh:property [ sh:path ex:p ; sh:minCount "three" ] ."#,
    );
    let model = ShapesModel::parse(&g);
    let recs = model.ill_formed();
    assert_eq!(recs.len(), 1, "exactly one ill-formed construct: {:?}", recs);
    assert_eq!(recs[0].predicate, "http://www.w3.org/ns/shacl#minCount");
    assert!(recs[0].message.contains("integer"), "message: {}", recs[0].message);
    // The carrying node is the property shape (a blank node).
    assert!(matches!(recs[0].node, oxrdf::Term::BlankNode(_)));
    // A well-formed graph records nothing.
    let ok = ShapesModel::parse(&shapes_graph(
        r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:nodeKind sh:IRI ."#,
    ));
    assert!(ok.ill_formed().is_empty());
}

#[test]
fn shacl_failure_display_mentions_ill_formed() {
    let err = run_strict(r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:closed "shut" ."#)
        .expect_err("must reject");
    let text = err.to_string();
    assert!(
        text.contains("1 ill-formed shapes-graph construct(s)"),
        "display: {}",
        text
    );
    assert!(text.contains("closed"), "display names the predicate: {}", text);
}

#[test]
fn lenient_validate_still_skips_unparsable_sparql_select() {
    // (sq-ehq4g) The parser-dependent rejection keeps the lenient invariant:
    // the SAME unparsable sh:select is a strict failure and a lenient skip.
    let shapes = r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
                      sh:sparql [ sh:select "SELECT ??? garbage {" ] ."#;
    assert!(run_strict(shapes).is_err());
    let report = run_lenient(shapes);
    assert!(report.conforms, "lenient skip unchanged: {}", report.to_text());
}

#[test]
fn lenient_report_surfaces_ill_formed_as_diagnostics() {
    // (sq-c1v3e) The lenient path surfaces each ill-formed construct as a
    // report diagnostic — visibility without failing: the skip is unchanged
    // (the report still conforms) and `source_component` carries the offending
    // SHACL predicate IRI, mirroring IllFormedConstruct::predicate.
    let shapes = r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
                      sh:property [ sh:path ex:missing ; sh:minCount "three" ] ."#;
    let report = run_lenient(shapes);
    assert!(report.conforms, "skip unchanged: {}", report.to_text());
    assert_eq!(report.diagnostics.len(), 1, "{}", report.to_text());
    let d = &report.diagnostics[0];
    assert_eq!(d.source_component, "http://www.w3.org/ns/shacl#minCount");
    assert!(d.message.contains("integer"), "message: {}", d.message);
    // A well-formed graph produces no diagnostics.
    let clean = run_lenient(r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ; sh:nodeKind sh:IRI ."#);
    assert!(clean.diagnostics.is_empty());
}

#[test]
fn lenient_validate_still_skips_ill_formed_constructs() {
    // The load-bearing invariant pair: the SAME graph is a strict failure and a
    // lenient skip — and the skip really skips (the unparsable sh:minCount
    // imposes no constraint, so the report conforms).
    let shapes = r#"ex:S a sh:NodeShape ; sh:targetNode ex:n ;
                      sh:property [ sh:path ex:missing ; sh:minCount "three" ] ."#;
    assert!(run_strict(shapes).is_err());
    let report = run_lenient(shapes);
    assert!(report.conforms, "lenient skip unchanged: {}", report.to_text());
}
