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

#[test]
fn stored_nan_orders_false_not_error_in_either_mode() {
    let data = "<urn:s> <urn:p> \"NaN\"^^<http://www.w3.org/2001/XMLSchema#double> .\n\
                <urn:t> <urn:p> \"NaN\"^^<http://www.w3.org/2001/XMLSchema#float> .";
    let graph = Graph::load_str(data, "ntriples").unwrap();
    for strict in [false, true] {
        for op in ["<", "<=", ">", ">="] {
            let projected = format!("SELECT (?n {op} 0 AS ?v) WHERE {{ ?s <urn:p> ?n }}");
            assert_eq!(
                cells(&graph, &projected, strict),
                vec![vec![Some(FALSE.to_string())]; 2],
                "strict={strict}: {projected}"
            );
            let negated = format!("SELECT ?s WHERE {{ ?s <urn:p> ?n FILTER(!(?n {op} 0)) }}");
            assert_eq!(
                cells(&graph, &negated, strict).len(),
                2,
                "strict={strict}: {negated}"
            );
        }
    }
}

/// Ordinary execution and strict numeric capacity (the proved evaluator's mode) agree on
/// every comparison and arithmetic operator over the IEEE special values, signed zero and
/// mixed numeric tiers, as stored operands and against constants.
#[test]
fn special_numerics_agree_between_normal_and_strict_execution() {
    let xsd = "http://www.w3.org/2001/XMLSchema#";
    let values = [
        ("NaN", "double"),
        ("NaN", "float"),
        ("INF", "double"),
        ("-INF", "float"),
        ("0", "double"),
        ("-0", "double"),
        ("-0", "float"),
        ("0", "integer"),
        ("1.5", "decimal"),
        ("2", "integer"),
    ];
    let mut data = String::new();
    for (i, (lexical, datatype)) in values.iter().enumerate() {
        data.push_str(&format!(
            "<urn:s{i}> <urn:p> \"{lexical}\"^^<{xsd}{datatype}> .\n"
        ));
    }
    let graph = Graph::load_str(&data, "ntriples").unwrap();
    let constants = [
        "0",
        "-0.0e0",
        "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#double>",
    ];
    let mut expressions = Vec::new();
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        expressions.push(format!("?a {op} ?b"));
        for c in constants {
            expressions.push(format!("?a {op} {c}"));
        }
    }
    for op in ["+", "-", "*", "/"] {
        expressions.push(format!("?a {op} ?b"));
        expressions.push(format!("(?a {op} ?b) = (?a {op} ?b)"));
        expressions.push(format!("(?a {op} ?b) < 1"));
    }
    expressions.extend(["sameTerm(?a, ?b)", "-?a", "ABS(?a)", "?a IN (?b)"].map(String::from));
    for expression in &expressions {
        for query in [
            format!("SELECT ?x ?y ({expression} AS ?v) WHERE {{ ?x <urn:p> ?a . ?y <urn:p> ?b }} ORDER BY ?x ?y"),
            format!("SELECT ?x ?y WHERE {{ ?x <urn:p> ?a . ?y <urn:p> ?b FILTER({expression}) }} ORDER BY ?x ?y"),
            format!("SELECT ?x ?y WHERE {{ ?x <urn:p> ?a . ?y <urn:p> ?b FILTER(!({expression})) }} ORDER BY ?x ?y"),
        ] {
            assert_eq!(cells(&graph, &query, false), cells(&graph, &query, true), "{query}");
        }
    }
}
