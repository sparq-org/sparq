//! (sq-bif.30) Integration coverage for empty SHACL inputs, Turtle
//! loader failures, and strict rejection of unsound SPARQL pre-binding.

use sparq_shacl::{
    count_focus_nodes, graph_from_triples, load_turtle_with_base, validate, validate_strict,
    ShapesModel,
};

fn empty_graph() -> sparq_core::Graph {
    graph_from_triples(std::iter::empty::<oxrdf::Triple>())
}

// (sq-bif.30) Zero targeted shapes produce a vacuously conforming,
// completely empty report.
#[test]
fn empty_shapes_conform_without_results_or_diagnostics() {
    let data = empty_graph();
    let shapes = empty_graph();

    let report = validate(&data, &shapes);

    assert!(report.conforms);
    assert!(report.results.is_empty());
    assert!(report.diagnostics.is_empty());
}

// (sq-bif.30) A declared target class selects nothing when the data
// graph itself is empty.
#[test]
fn target_class_has_zero_focus_nodes_in_empty_data() {
    let data = empty_graph();
    let shapes = load_turtle_with_base(
        r#"
            @prefix sh: <http://www.w3.org/ns/shacl#> .
            @prefix ex: <http://example.org/> .
            ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person .
        "#,
        "http://base.example/",
    )
    .expect("target-class fixture must parse");
    let model = ShapesModel::parse(&shapes);

    assert_eq!(count_focus_nodes(&data, &model), 0);
}

// (sq-bif.30) Both document parsing and base-IRI validation surface
// errors through the public Turtle loader.
#[test]
fn turtle_loader_rejects_malformed_input_and_invalid_base() {
    assert!(load_turtle_with_base("this is not turtle @@@", "http://base.example/").is_err());
    assert!(load_turtle_with_base("", "not a valid base").is_err());
}

// (sq-bif.30) Mirror the crate's existing MINUS fixture: lenient
// validation skips the unsound constraint, while strict validation rejects it.
#[test]
fn strict_validation_reports_unsound_pre_binding() {
    let fixture = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetNode ex:x ;
          sh:sparql [ a sh:SPARQLConstraint ;
            sh:select "SELECT $this WHERE { $this ?p ?o . MINUS { $this ?p \"v\" } }" ] .
    "#;
    let graph = load_turtle_with_base(fixture, "http://base.example/")
        .expect("pre-binding fixture must parse");

    assert!(
        validate(&graph, &graph).conforms,
        "lenient validation must skip the unsound constraint"
    );

    let err = validate_strict(&graph, &graph)
        .expect_err("strict validation must reject a MINUS pre-binding");
    assert!(!err.pre_binding.is_empty());
    assert!(err.to_string().starts_with("SHACL failure:"));
}
