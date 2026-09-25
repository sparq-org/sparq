// [GPT-6] Nested GRAPH keeps the active dataset catalog across public query surfaces.
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{QueryBudget, ask, count, query, query_json, query_with_budget};

fn graph() -> Graph {
    let mut graph = Graph::load_dataset(
        concat!(
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g2> .\n",
            "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g2> .\n",
        ),
        "nquads",
    )
    .unwrap();
    graph
        .ensure_named(&Term::NamedNode(NamedNode::new("http://ex/empty").unwrap()))
        .unwrap();
    graph
}

fn select(graph: &Graph, body: &str) -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<_> = query(graph, &format!("PREFIX ex: <http://ex/> {body}"))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| term.map(|t| t.to_string()))
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

fn iri(value: &str) -> Option<String> {
    Some(format!("<http://ex/{value}>"))
}

#[test]
fn nested_constant_graph_keeps_root_catalog_on_select_ask_count_and_json() {
    let graph = graph();
    let select = "PREFIX ex: <http://ex/> SELECT ?s { GRAPH ex:g1 { GRAPH ex:g2 { ?s ex:p ?o } } }";
    assert_eq!(query(&graph, select).unwrap().rows.len(), 2);
    assert_eq!(count(&graph, select).unwrap(), 2);
    let json: serde_json::Value =
        serde_json::from_str(&query_json(&graph, select).unwrap()).unwrap();
    assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 2);
    assert!(
        ask(
            &graph,
            "PREFIX ex: <http://ex/> ASK { GRAPH ex:g1 { GRAPH ex:g2 { ?s ex:p ?o } } }"
        )
        .unwrap()
    );
}

#[test]
fn nested_graph_variables_retain_empty_graphs_and_compatible_name_bindings() {
    let graph = graph();
    assert_eq!(
        select(&graph, "SELECT ?g { GRAPH ?g { GRAPH ?g {} } }"),
        vec![vec![iri("empty")], vec![iri("g1")], vec![iri("g2")],]
    );
    assert_eq!(
        select(&graph, "SELECT ?g ?h { GRAPH ?g { GRAPH ?h {} } }").len(),
        9
    );
    // Same output mapping from different outer active graph choices is a bag duplicate.
    assert_eq!(
        select(
            &graph,
            "SELECT ?s { GRAPH ?g { GRAPH ex:g1 { ?s ex:p ?o } } }"
        ),
        vec![vec![iri("a")], vec![iri("a")], vec![iri("a")],]
    );
    assert!(select(&graph, "SELECT ?g { GRAPH ?g { VALUES ?g { ex:absent } } }").is_empty());
}

#[test]
fn from_named_and_absent_graph_boundaries_apply_at_every_depth() {
    let graph = graph();
    assert_eq!(
        select(
            &graph,
            "SELECT ?g ?h FROM NAMED ex:g1 FROM NAMED ex:empty { GRAPH ?g { GRAPH ?h {} } }"
        )
        .len(),
        4
    );
    assert!(
        select(
            &graph,
            "SELECT ?s FROM NAMED ex:g1 { GRAPH ex:g1 { GRAPH ex:g2 { ?s ex:p ?o } } }"
        )
        .is_empty()
    );
    assert!(
        select(
            &graph,
            "SELECT ?s { GRAPH ex:absent { GRAPH ex:g2 { ?s ex:p ?o } } }"
        )
        .is_empty()
    );
    assert_eq!(
        select(
            &graph,
            "SELECT ?s { GRAPH ex:empty { GRAPH ex:g2 { ?s ex:p ?o } } }"
        ),
        vec![vec![iri("a")], vec![iri("c")]]
    );
}

#[test]
fn nested_graph_evaluation_obeys_row_budget_and_expression_reentry() {
    let graph = graph();
    let budget = QueryBudget {
        max_rows: Some(4),
        ..QueryBudget::unlimited()
    };
    assert!(
        query_with_budget(
            &graph,
            "PREFIX ex: <http://ex/> SELECT ?g ?h { GRAPH ?g { GRAPH ?h {} } }",
            &budget
        )
        .is_err()
    );
    assert!(ask(&graph,
        "PREFIX ex: <http://ex/> ASK { GRAPH ex:g1 { FILTER EXISTS { GRAPH ex:g2 { ?s ex:p ?o } } } }").unwrap());
    // A new query must not inherit another dataset's borrowed catalog.
    let empty = Graph::load_str("", "ntriples").unwrap();
    assert!(
        !ask(
            &empty,
            "PREFIX ex: <http://ex/> ASK { GRAPH ex:g1 { GRAPH ex:g2 { ?s ex:p ?o } } }"
        )
        .unwrap()
    );
}
