#![cfg(feature = "paths")]

// sq-lsp7k.3 acceptance and mutation-witnessed START/END graph-pattern coverage.

use oxrdf::{NamedNode, Term, Variable};
use sparq_core::Graph;
use sparq_engine::{enumerate_paths, query_paths, Endpoint, PathMode, PathSpec, Via};

/// Two hub-rooted two-hop routes into `ex:z`, plus `ex:c` — a one-hop route into
/// `ex:z` whose start is deliberately NOT a hub, so an ignored START restriction
/// shows up as an extra path.
fn fixture() -> Graph {
    Graph::load_str(
        r#"@prefix ex: <http://ex/> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            ex:a ex:p ex:m . ex:m ex:p ex:z .
            ex:b ex:p ex:n . ex:n ex:p ex:z .
            ex:c ex:p ex:z .
            ex:a rdf:type ex:Hub .
            ex:b rdf:type ex:Hub .
            ex:z rdf:type ex:Sink ."#,
        "turtle",
    )
    .unwrap()
}

fn column(result: &sparq_engine::QueryResult, name: &str) -> Vec<String> {
    let index = result
        .vars
        .iter()
        .position(|var| var.as_str() == name)
        .expect("column is projected");
    result
        .rows
        .iter()
        .map(|row| row[index].as_ref().expect("PATHS binds every column").to_string())
        .collect()
}

fn distinct(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

#[test]
fn start_graph_pattern_restricts_the_enumerated_start_set() {
    let result = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = { ?s a ex:Hub } END ?e = ex:z \
         VIA ex:p",
    )
    .unwrap();

    // `ex:c` also reaches `ex:z`, but is not a hub — its absence witnesses the restriction.
    assert_eq!(
        distinct(column(&result, "s")),
        ["<http://ex/a>", "<http://ex/b>"]
    );
    assert_eq!(result.rows.len(), 4, "two two-hop paths, one row per hop");
    assert_eq!(
        distinct(column(&result, "node")),
        ["<http://ex/m>", "<http://ex/n>", "<http://ex/z>"]
    );
}

#[test]
fn end_graph_pattern_restricts_the_enumerated_end_set() {
    let result = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:a END ?e = { ?e a ex:Sink } \
         VIA ex:p",
    )
    .unwrap();

    // `ex:m` is reachable from `ex:a` but is not a sink, so the only path is a -> m -> z.
    assert_eq!(distinct(column(&result, "e")), ["<http://ex/z>"]);
    assert_eq!(result.rows.len(), 2, "one two-hop path, one row per hop");
    assert_eq!(
        column(&result, "node"),
        ["<http://ex/m>", "<http://ex/z>"]
    );
}

#[test]
fn endpoint_pattern_must_bind_the_declared_endpoint_variable() {
    let error = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = { ?other a ex:Hub } END ?e = ex:z \
         VIA ex:p",
    )
    .unwrap_err();
    assert!(error.contains("must bind ?s"), "unexpected error: {}", error);
}

#[test]
fn single_solution_endpoint_pattern_matches_the_fixed_iri_form() {
    let fixed = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:a END ?e = ex:z VIA ex:p",
    )
    .unwrap();
    let pattern = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = { VALUES ?s { ex:a } } END ?e = ex:z \
         VIA ex:p",
    )
    .unwrap();

    assert_eq!(fixed.vars, pattern.vars);
    assert_eq!(fixed.rows, pattern.rows);
    assert_eq!(fixed.rows.len(), 2);
}

#[test]
fn programmatic_endpoint_pattern_enumerates_both_hub_paths() {
    let paths = enumerate_paths(
        &fixture(),
        &PathSpec {
            mode: PathMode::Shortest,
            cyclic: false,
            start: Some(Endpoint::Pattern {
                source: "?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Hub>"
                    .to_owned(),
                variable: Variable::new_unchecked("s"),
            }),
            end: Some(Endpoint::Node(Term::NamedNode(
                NamedNode::new("http://ex/z").unwrap(),
            ))),
            via: Via::Predicate(NamedNode::new("http://ex/p").unwrap()),
            max_length: None,
        },
    )
    .unwrap();

    assert_eq!(paths.len(), 2);
    assert!(paths.iter().all(|path| path.nodes.len() == 3));
    assert_eq!(
        distinct(
            paths
                .iter()
                .map(|path| path.nodes[0].to_string())
                .collect()
        ),
        ["<http://ex/a>", "<http://ex/b>"]
    );
}

#[test]
fn endpoint_pattern_that_selects_nothing_yields_no_paths() {
    let result = query_paths(
        &fixture(),
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = { ?s a ex:Missing } END ?e = ex:z \
         VIA ex:p",
    )
    .unwrap();
    assert!(result.rows.is_empty());
}
