#![cfg(feature = "paths")]

// sq-lsp7k.3.3 acceptance and mutation-witnessed VIA-pattern coverage.

use sparq_core::Graph;
use sparq_engine::query_paths;

fn fixture() -> Graph {
    Graph::load_str(
        r#"@prefix ex: <http://ex/> .
            ex:a ex:knows ex:b .
            ex:b ex:memberOf ex:c .
            ex:c ex:knows ex:d .
            ex:d ex:memberOf ex:e .
            ex:a ex:p ex:c .
            ex:c ex:p ex:e ."#,
        "turtle",
    )
    .unwrap()
}

#[test]
fn composed_graph_pattern_materializes_the_expected_paths() {
    let result = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:a END ?e = ex:e \
         VIA { ?from ex:knows/ex:memberOf ?to }",
    )
    .unwrap();

    assert_eq!(
        result.rows.len(),
        2,
        "the fixture contains one two-edge path"
    );
    assert_eq!(
        result.rows[0][4].as_ref().unwrap().to_string(),
        "<http://ex/c>"
    );
    assert_eq!(
        result.rows[1][4].as_ref().unwrap().to_string(),
        "<http://ex/e>"
    );
    let edge_markers = result
        .rows
        .iter()
        .map(|row| row[5].as_ref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(edge_markers[0], edge_markers[1]);
    assert!(edge_markers[0].is_literal());
}

#[test]
fn single_predicate_pattern_is_byte_identical_to_predicate_via() {
    let predicate = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:a END ?e = ex:e VIA ex:p",
    )
    .unwrap();
    let pattern = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:a END ?e = ex:e \
         VIA { ?from ex:p ?to }",
    )
    .unwrap();

    assert_eq!(predicate.vars, pattern.vars);
    assert_eq!(predicate.rows.len(), pattern.rows.len());
    for (predicate_row, pattern_row) in predicate.rows.iter().zip(&pattern.rows) {
        assert_eq!(&predicate_row[..5], &pattern_row[..5]);
    }
}

#[test]
fn pattern_missing_reserved_to_binding_is_a_loud_error() {
    let error = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s END ?e \
         VIA { ?from ex:p ex:c }",
    )
    .unwrap_err();
    assert!(error.contains("must bind ?to"), "unexpected error: {error}");
}
