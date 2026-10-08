#![cfg(feature = "paths")]

use oxrdf::Term;
use sparq_core::Graph;
use sparq_engine::{explain_paths, query, query_paths};

const PATHS: &str = "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:a END ?e = ex:d VIA ex:p";

fn diamond() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> . ex:a ex:p ex:b, ex:c . ex:b ex:p ex:d . ex:c ex:p ex:d .",
        "turtle",
    )
    .unwrap()
}

fn lexical(term: &Option<Term>) -> String {
    term.as_ref().unwrap().to_string()
}

#[test]
fn shortest_diamond_returns_exact_row_per_hop_bindings() {
    let result = query_paths(&diamond(), PATHS).unwrap();
    assert_eq!(
        result.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
        ["s", "e", "pathIndex", "hopIndex", "node", "edge"]
    );
    let rows = result
        .rows
        .iter()
        .map(|row| row.iter().map(lexical).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        vec![
            vec![
                "<http://ex/a>",
                "<http://ex/d>",
                "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "<http://ex/b>",
                "<http://ex/p>"
            ],
            vec![
                "<http://ex/a>",
                "<http://ex/d>",
                "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "<http://ex/d>",
                "<http://ex/p>"
            ],
            vec![
                "<http://ex/a>",
                "<http://ex/d>",
                "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "<http://ex/c>",
                "<http://ex/p>"
            ],
            vec![
                "<http://ex/a>",
                "<http://ex/d>",
                "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                "<http://ex/d>",
                "<http://ex/p>"
            ],
        ]
    );
}

#[test]
fn standard_surface_still_rejects_paths() {
    assert!(query(&diamond(), PATHS).is_err());
}

#[test]
fn malformed_paths_are_loud() {
    assert!(query_paths(&diamond(), "PATHS SHORTEST START ?s END ?e")
        .unwrap_err()
        .contains("VIA"));
    assert_eq!(
        query_paths(&diamond(), "PATHS ALL START ?s END ?e VIA <http://ex/p>").unwrap_err(),
        "PATHS ALL requires MAX LENGTH"
    );
}

#[test]
fn explain_contains_typed_paths_operator() {
    assert_eq!(
        explain_paths(&diamond(), PATHS).unwrap(),
        "Paths mode=shortest cyclic=false start=<http://ex/a> end=<http://ex/d> via=<http://ex/p> maxLength=none"
    );
}

/// #3994 — the START/END restriction is part of the rendered plan, so two plans that
/// differ only there no longer explain identically.
#[test]
fn explain_renders_endpoint_restrictions() {
    let g = diamond();
    assert_eq!(
        explain_paths(&g, "PATHS ALL START ?s END ?e VIA <http://ex/p> MAX LENGTH 2").unwrap(),
        "Paths mode=all cyclic=false start=none end=none via=<http://ex/p> maxLength=2"
    );
    let pattern = explain_paths(
        &g,
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = { ?s ex:p ex:b } END ?e = ex:d VIA ex:p",
    )
    .unwrap();
    assert!(pattern.contains(" end=<http://ex/d> "), "{pattern}");
    assert!(pattern.contains(" start=?s { ") && !pattern.contains("start=none"), "{pattern}");
    // The same pattern selecting its object instead must render differently.
    let object = explain_paths(
        &g,
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?x = { ?s ex:p ?x } END ?e = ex:d VIA ex:p",
    )
    .unwrap();
    let subject = explain_paths(
        &g,
        "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = { ?s ex:p ?x } END ?e = ex:d VIA ex:p",
    )
    .unwrap();
    assert!(object.contains(" start=?x { "), "{object}");
    assert!(subject.contains(" start=?s { "), "{subject}");
}
