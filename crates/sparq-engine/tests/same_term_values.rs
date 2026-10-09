//! `sameTerm` over computed operands: arithmetic and boolean results compare as the
//! literal a BIND would produce, and an unbound operand is a type error, not `false`.
use sparq_core::Graph;

fn count(graph: &Graph, query: &str) -> usize {
    sparq_engine::query(graph, query).unwrap().rows.len()
}

#[test]
fn computed_operands_compare_as_their_literals_and_unbound_is_an_error() {
    let data = "<http://ex/s> <http://ex/p> 1 .";
    let graph = Graph::load_str(data, "turtle").unwrap();
    let pre = "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#>";
    for (expression, rows) in [
        ("sameTerm(1 + 0, 1)", 1),
        ("!sameTerm(1 + 0, 1)", 0),
        ("sameTerm(?n + 0, ?n)", 1),
        ("sameTerm(?n + 0, 1.0)", 0),
        ("sameTerm(1 = 1, true)", 1),
        ("sameTerm(?n * 2, 2)", 1),
        ("sameTerm(1 + 0, \"1\"^^xsd:decimal)", 0),
        ("sameTerm(?unbound, 1)", 0),
        ("!sameTerm(?unbound, 1)", 0),
        ("!sameTerm(1 + 0, ?unbound)", 0),
        ("!sameTerm(1 / 0, 1)", 0),
    ] {
        let filtered = format!("{pre} SELECT ?s {{ ?s <http://ex/p> ?n FILTER({expression}) }}");
        assert_eq!(count(&graph, &filtered), rows, "{filtered}");
        let empty = format!(
            "{pre} SELECT * {{ FILTER({}) }}",
            expression.replace("?n", "1")
        );
        assert_eq!(count(&graph, &empty), rows, "{empty}");
    }
    let bound =
        format!("{pre} SELECT ?v {{ ?s <http://ex/p> ?n BIND(sameTerm(?unbound, ?n) AS ?v) }}");
    let result = sparq_engine::query(&graph, &bound).unwrap().rows;
    assert_eq!(result.len(), 1);
    assert!(
        result[0][0].is_none(),
        "an error operand leaves the BIND unbound"
    );
}
