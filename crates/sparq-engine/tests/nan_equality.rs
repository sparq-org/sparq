// op:numeric-equal(NaN, NaN) is false even when both operands are the same RDF term,
// and ordinary execution must agree with strict numeric capacity (the proved evaluator's mode).
use sparq_core::Graph;
use sparq_engine::{query_with_budget, QueryBudget};

fn budget(strict: bool) -> QueryBudget {
    QueryBudget {
        strict_numeric_capacity: strict,
        ..QueryBudget::unlimited()
    }
}

fn cells(graph: &Graph, query: &str, strict: bool) -> Vec<Vec<Option<String>>> {
    query_with_budget(graph, query, &budget(strict))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| term.map(|term| term.to_string()))
                .collect()
        })
        .collect()
}

const TRUE: &str = "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>";
const FALSE: &str = "\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>";

#[test]
fn stored_nan_is_not_equal_to_itself_in_either_mode() {
    for datatype in ["double", "float"] {
        let data = format!(
            "<urn:s> <urn:p> \"NaN\"^^<http://www.w3.org/2001/XMLSchema#{datatype}> .\n\
             <urn:s> <urn:q> \"NaN\"^^<http://www.w3.org/2001/XMLSchema#{datatype}> ."
        );
        let graph = Graph::load_str(&data, "ntriples").unwrap();
        for strict in [false, true] {
            for (projection, expected) in [
                ("?n = ?n", FALSE),
                ("?n != ?n", TRUE),
                ("?n = ?m", FALSE),
                ("?n != ?m", TRUE),
                ("sameTerm(?n, ?m)", TRUE),
            ] {
                let query = format!(
                    "SELECT ({projection} AS ?v) WHERE {{ ?s <urn:p> ?n . ?s <urn:q> ?m }}"
                );
                assert_eq!(
                    cells(&graph, &query, strict),
                    vec![vec![Some(expected.to_string())]],
                    "{datatype} strict={strict}: {query}"
                );
            }
            for (filter, rows) in [("?n = ?m", 0), ("?n != ?m", 1), ("?n = ?n", 0)] {
                let query =
                    format!("SELECT ?s WHERE {{ ?s <urn:p> ?n . ?s <urn:q> ?m FILTER({filter}) }}");
                assert_eq!(
                    cells(&graph, &query, strict).len(),
                    rows,
                    "{datatype} strict={strict}: {query}"
                );
            }
        }
    }
}
