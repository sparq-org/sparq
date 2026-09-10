// [GPT-6 ASTRA] Fixed synthetic RDF-term and UPDATE-template conformance witness.
// Rust guideline compliant 2026-02-21
use sparq_core::Graph;
use std::collections::BTreeSet;

const TEMPLATE: &str =
    "INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }";

fn dataset(graph: &Graph) -> Vec<String> {
    let scan = graph.store.scan(&[None, None, None]);
    let mut lines: Vec<String> = scan
        .rows
        .iter()
        .map(|row| {
            let ids = scan.to_spo(row);
            format!(
                "{} {} {} .",
                graph.dict.term(ids[0]),
                graph.dict.term(ids[1]),
                graph.dict.term(ids[2])
            )
        })
        .collect();
    lines.sort();
    lines
}

fn observe(case: &str, terms: &[&str], inplace: bool) {
    let input = terms
        .iter()
        .map(|term| format!("<http://ex/s2> <http://ex/p1> {term} ."))
        .collect::<Vec<_>>()
        .join(" ");
    let insert = format!("INSERT DATA {{ {input} }}");
    let mut graph = Graph::new();
    if inplace {
        sparq_engine::update_in_place(&mut graph, &insert).expect("in-place data insert");
    } else {
        graph = sparq_engine::update(&graph, &insert).expect("rebuild data insert");
    }
    let before = dataset(&graph);
    let result = sparq_engine::query(
        &graph,
        "SELECT ?s ?o WHERE { ?s <http://ex/p1> ?o }",
    )
    .expect("public WHERE query");
    let mut rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row| row.iter().map(|t| t.as_ref().expect("bound term").to_string()).collect())
        .collect();
    rows.sort();
    let mut expected: Vec<String> = terms.iter().map(|term| (*term).to_string()).collect();
    expected.sort();
    let lexical_terms: Vec<String> = rows.iter().map(|row| row[1].clone()).collect();
    assert_eq!(lexical_terms, expected, "exact RDF lexical identities");
    assert_eq!(before.len(), terms.len());
    assert_eq!(rows.len(), terms.len());
    if inplace {
        sparq_engine::update_in_place(&mut graph, TEMPLATE).expect("in-place template insert");
    } else {
        graph = sparq_engine::update(&graph, TEMPLATE).expect("rebuild template insert");
    }
    let after = dataset(&graph);
    let output = sparq_engine::query(
        &graph,
        "SELECT ?b WHERE { <http://ex/s2> <http://ex/p0> ?b }",
    )
    .expect("public inserted-term query");
    let mut blank_nodes = BTreeSet::new();
    for row in &output.rows {
        let term = row[0].as_ref().expect("bound inserted term");
        assert!(term.is_blank_node(), "template must create blank nodes");
        blank_nodes.insert(term.to_string());
    }
    assert_eq!(output.rows.len(), terms.len());
    assert_eq!(blank_nodes.len(), terms.len(), "fresh blank node per solution");
    assert_eq!(after.len(), 2 * terms.len());
    let path = if inplace { "in-place" } else { "rebuild" };
    let blanks: Vec<_> = blank_nodes.into_iter().collect();
    println!(
        "{{\"case\":{case:?},\"path\":{path:?},\"lexical_terms\":{lexical_terms:?},\"where_rows\":{rows:?},\"blank_nodes\":{blanks:?},\"before\":{before:?},\"after\":{after:?}}"
    );
}

fn main() {
    let eight = "\"8\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let padded = "\"008\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    let nine = "\"9\"^^<http://www.w3.org/2001/XMLSchema#integer>";
    for inplace in [false, true] {
        observe("lexical-pair", &[eight, padded], inplace);
        observe("canonical-eight", &[eight], inplace);
        observe("different-values", &[eight, nine], inplace);
    }
}
