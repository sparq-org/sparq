// [GPT-6] Published SPARQL 1.1 MINUS domains and existential NPS matching.
use sparq_core::Graph;
use sparq_engine::query;

fn rows(graph: &Graph, body: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<_> = query(graph, &format!("PREFIX ex:<http://ex/> {body}"))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| term.map_or("UNBOUND".into(), |term| term.to_string()))
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn values_join_cannot_remove_minus_shared_variable_domains() {
    let graph = Graph::load_str(
        "@prefix ex:<http://ex/> . ex:a ex:p 1 . ex:b ex:p 2 .",
        "turtle",
    )
    .unwrap();
    let right = "{?x ex:p ?m MINUS {?x ex:p ?z}}";
    assert!(rows(&graph, &format!("SELECT ?x {right}")).is_empty());
    assert!(rows(&graph, &format!("SELECT ?x {{VALUES ?x {{ex:a}} {right}}}")).is_empty());
}

fn selective_graph() -> Graph {
    Graph::load_str(
        "@prefix ex:<http://ex/> . ex:a ex:p 1; ex:q 3 . ex:b ex:p 2 . ex:z ex:marker true .",
        "turtle",
    )
    .unwrap()
}

#[test]
fn minus_join_preserves_selectivity_duplicates_and_disjoint_domains() {
    let graph = selective_graph();
    assert_eq!(
        rows(
            &graph,
            "SELECT ?x {VALUES ?x {ex:a ex:a ex:b} {?x ex:p ?m MINUS {?x ex:q ?z}}}"
        ),
        vec![vec!["<http://ex/b>".to_owned()]]
    );
    assert_eq!(
        rows(
            &graph,
            "SELECT ?x {VALUES ?x {ex:a ex:a ex:b} {?x ex:p ?m MINUS {?y ex:q ?z}}}"
        ),
        vec![
            vec!["<http://ex/a>".to_owned()],
            vec!["<http://ex/a>".to_owned()],
            vec!["<http://ex/b>".to_owned()]
        ]
    );
}

#[test]
fn optional_minus_preserves_unmatched_outer_rows() {
    let graph = selective_graph();
    assert_eq!(
        rows(
            &graph,
            "SELECT ?x ?m {VALUES ?x {ex:a ex:a ex:b} OPTIONAL {?x ex:p ?m MINUS {?x ex:q ?z}}}"
        ),
        vec![
            vec!["<http://ex/a>".to_owned(), "UNBOUND".to_owned()],
            vec!["<http://ex/a>".to_owned(), "UNBOUND".to_owned()],
            vec![
                "<http://ex/b>".to_owned(),
                "\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_owned()
            ]
        ]
    );
}

#[test]
fn sibling_binding_must_not_filter_a_minus_right_only_variable() {
    let graph = selective_graph();
    // ?x is bound by a positive sibling, but is not a column in MINUS's left
    // operand. The anti-join must inspect every ?x value on its right.
    assert_eq!(
        rows(
            &graph,
            "SELECT ?s {VALUES ?x {ex:z} {?x ex:marker true . {?s ex:p ?m MINUS {?s ex:q ?x}}}}"
        ),
        vec![vec!["<http://ex/b>".to_owned()]]
    );
}

#[test]
fn negated_property_set_is_existential_over_edge_predicates() {
    let graph = Graph::load_str(
        "@prefix ex:<http://ex/> . ex:a ex:p ex:b; ex:q ex:b . ex:b ex:r ex:a; ex:s ex:a .",
        "turtle",
    )
    .unwrap();
    // The predicate is existential in §18.4, not a projected BGP variable.
    assert_eq!(
        rows(&graph, "SELECT ?o {ex:a !ex:excluded ?o}"),
        vec![vec!["<http://ex/b>".to_owned()]]
    );
    assert_eq!(
        rows(&graph, "SELECT * {ex:a !ex:excluded ex:b}"),
        vec![Vec::<String>::new()]
    );
    assert_eq!(
        rows(&graph, "SELECT ?o {ex:a ^(!ex:excluded) ?o}"),
        vec![vec!["<http://ex/b>".to_owned()]]
    );
    // Forward and reverse alternatives are a multiset union of these sets.
    assert_eq!(
        rows(&graph, "SELECT ?o {ex:a !(ex:excluded|^ex:excluded) ?o}"),
        vec![vec!["<http://ex/b>".to_owned()]; 2]
    );
}
