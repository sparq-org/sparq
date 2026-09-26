// [GPT-6] Graph-form causes are captured before their own budget frame exits.
use oxrdf::Triple;
use sparq_core::Graph;
use sparq_engine::{
    BudgetExceeded, EvaluationCapacity, PreparedQuery, QueryBudget, QueryFailure,
    construct_prepared_with_budget, construct_prepared_with_budget_detailed,
    describe_prepared_with_budget, describe_prepared_with_budget_detailed,
};
use std::sync::{Arc, atomic::AtomicBool};

fn run(graph: &Graph, construct: bool, query: &str, budget: &QueryBudget) -> Result<Vec<Triple>, QueryFailure> {
    let prepared = PreparedQuery::parse(query).unwrap();
    if construct {
        construct_prepared_with_budget_detailed(graph, &prepared, budget)
    } else {
        describe_prepared_with_budget_detailed(graph, &prepared, budget)
    }
}

fn query(construct: bool, body: &str) -> String {
    if construct {
        format!("CONSTRUCT {{ ?s <http://ex/p> <http://ex/o> }} WHERE {{ {body} }}")
    } else {
        format!("DESCRIBE ?s WHERE {{ {body} }}")
    }
}

fn graph() -> Graph {
    Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/o> .\n<http://ex/b> <http://ex/p> <http://ex/o> .", "ntriples").unwrap()
}

#[test]
fn graph_forms_retain_resource_causes_legacy_strings_and_success_after_failure() {
    let graph = graph();
    for construct in [true, false] {
        let query = query(construct, "VALUES ?s { <http://ex/a> <http://ex/b> }");
        for (budget, cause) in [
            (QueryBudget { max_rows: Some(1), ..QueryBudget::unlimited() }, BudgetExceeded::Rows),
            (QueryBudget { max_bytes: Some(0), ..QueryBudget::unlimited() }, BudgetExceeded::Bytes),
            (QueryBudget::cancelled_by(Arc::new(AtomicBool::new(true))), BudgetExceeded::Cancelled),
        ] {
            let failure = run(&graph, construct, &query, &budget).unwrap_err();
            assert_eq!(failure, QueryFailure::Budget(cause));
            let prepared = PreparedQuery::parse(&query).unwrap();
            let legacy = if construct {
                construct_prepared_with_budget(&graph, &prepared, &budget)
            } else {
                describe_prepared_with_budget(&graph, &prepared, &budget)
            };
            assert_eq!(legacy.unwrap_err(), failure.to_string());
            assert_eq!(run(&graph, construct, &query, &QueryBudget::unlimited()).unwrap().len(), 2);
        }
    }
}

#[test]
fn graph_forms_do_not_lose_numeric_or_temporal_capacity_on_frame_exit() {
    let graph = graph();
    for construct in [true, false] {
        for (expression, budget, cause) in [
            ("9223372036854775807 + 1", QueryBudget { strict_numeric_capacity: true, ..QueryBudget::unlimited() }, EvaluationCapacity::NumericRepresentation),
            ("STRDT(\"0000-01-01T00:00:00Z\", <http://www.w3.org/2001/XMLSchema#dateTime>)", QueryBudget { temporal_year_range: Some((1, 1_000_000_000)), ..QueryBudget::unlimited() }, EvaluationCapacity::TemporalYear),
        ] {
            let text = query(construct, &format!("BIND(<http://ex/a> AS ?s) BIND({expression} AS ?value) FILTER(BOUND(?value))"));
            assert_eq!(run(&graph, construct, &text, &budget).unwrap_err(), QueryFailure::Capacity(cause));
            let success = query(construct, "BIND(<http://ex/a> AS ?s)");
            assert_eq!(run(&graph, construct, &success, &budget).unwrap().len(), 1);
        }
    }
}

#[test]
fn ordinary_errors_and_wrong_forms_cannot_manufacture_graph_capacity() {
    let graph = graph();
    let budget = QueryBudget { strict_numeric_capacity: true, ..QueryBudget::unlimited() };
    for construct in [true, false] {
        // An expression error leaves ?s unbound; the observable graph is empty.
        assert!(run(&graph, construct, &query(construct, "BIND(1 / 0 AS ?s)"), &budget).unwrap().is_empty());
        assert!(matches!(run(&graph, construct, "ASK {}", &budget), Err(QueryFailure::Evaluation(_))));
        assert_eq!(run(&graph, construct, &query(construct, "BIND(<http://ex/a> AS ?s)"), &budget).unwrap().len(), 1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn graph_deadlines_remain_distinct_from_capacity() {
    for construct in [true, false] {
        let budget = QueryBudget { deadline: Some(std::time::Instant::now()), ..QueryBudget::unlimited() };
        assert_eq!(run(&graph(), construct, &query(construct, "BIND(<http://ex/a> AS ?s)"), &budget).unwrap_err(), QueryFailure::Budget(BudgetExceeded::Deadline));
    }
}

#[test]
fn nested_graph_failure_does_not_poison_a_parent_graph_query() {
    for child_construct in [true, false] {
        let mut registry = sparq_engine::FunctionRegistry::new();
        registry.register("urn:nested-graph", move |_| {
            let text = query(child_construct, "BIND(<http://ex/a> AS ?s) FILTER((9223372036854775807 + 1) > 0)");
            let budget = QueryBudget { strict_numeric_capacity: true, ..QueryBudget::unlimited() };
            assert_eq!(run(&graph(), child_construct, &text, &budget).unwrap_err(), QueryFailure::Capacity(EvaluationCapacity::NumericRepresentation));
            Ok(oxrdf::NamedNode::new("http://ex/a").unwrap().into())
        });
        for outer_construct in [true, false] {
            let text = query(outer_construct, "BIND(<urn:nested-graph>() AS ?s)");
            let budget = QueryBudget { max_rows: Some(1), strict_numeric_capacity: true, ..QueryBudget::unlimited() };
            let result = sparq_engine::with_functions(&registry, || run(&graph(), outer_construct, &text, &budget));
            assert_eq!(result.unwrap().len(), 1);
        }
    }
}
