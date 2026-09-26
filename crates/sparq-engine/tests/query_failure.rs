// [GPT-6] Typed causes come from emitters, not messages or prior query state.
use sparq_core::Graph;
use sparq_engine::{
    BudgetExceeded, EvaluationCapacity, PreparedQuery, QueryBudget, QueryFailure,
    query_prepared_with_budget, query_prepared_with_budget_detailed,
};
use std::sync::{Arc, atomic::AtomicBool};

fn empty() -> Graph {
    Graph::load_str("", "n-triples").unwrap()
}

#[test]
fn resources_cancellation_and_domain_capacity_keep_distinct_causes() {
    let graph = empty();
    let prepared = PreparedQuery::parse("SELECT ?v { VALUES ?v {1 2} }").unwrap();
    for (budget, cause) in [
        (
            QueryBudget {
                max_rows: Some(1),
                ..QueryBudget::unlimited()
            },
            BudgetExceeded::Rows,
        ),
        (
            QueryBudget {
                max_bytes: Some(0),
                ..QueryBudget::unlimited()
            },
            BudgetExceeded::Bytes,
        ),
        (
            QueryBudget::cancelled_by(Arc::new(AtomicBool::new(true))),
            BudgetExceeded::Cancelled,
        ),
    ] {
        let error = query_prepared_with_budget_detailed(&graph, &prepared, &budget).unwrap_err();
        assert_eq!(error, QueryFailure::Budget(cause));
        assert_eq!(
            query_prepared_with_budget(&graph, &prepared, &budget).unwrap_err(),
            error.to_string()
        );
        assert_eq!(
            query_prepared_with_budget_detailed(&graph, &prepared, &QueryBudget::unlimited())
                .unwrap()
                .rows
                .len(),
            2
        );
    }
    let numeric = PreparedQuery::parse("SELECT (9223372036854775807 + 1 AS ?v) {}").unwrap();
    assert_eq!(
        query_prepared_with_budget_detailed(
            &graph,
            &numeric,
            &QueryBudget {
                strict_numeric_capacity: true,
                ..QueryBudget::unlimited()
            }
        )
        .unwrap_err(),
        QueryFailure::Capacity(EvaluationCapacity::NumericRepresentation)
    );
    let temporal = PreparedQuery::parse("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> ASK { BIND(STRDT(\"0000-01-01T00:00:00Z\", xsd:dateTime) AS ?v) }").unwrap();
    assert_eq!(
        query_prepared_with_budget_detailed(
            &graph,
            &temporal,
            &QueryBudget {
                temporal_year_range: Some((1, 1_000_000_000)),
                ..QueryBudget::unlimited()
            }
        )
        .unwrap_err(),
        QueryFailure::Capacity(EvaluationCapacity::TemporalYear)
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn deadline_is_not_a_capacity_cause() {
    let prepared = PreparedQuery::parse("ASK {}").unwrap();
    let budget = QueryBudget {
        deadline: Some(std::time::Instant::now()),
        ..QueryBudget::unlimited()
    };
    assert_eq!(
        query_prepared_with_budget_detailed(&empty(), &prepared, &budget).unwrap_err(),
        QueryFailure::Budget(BudgetExceeded::Deadline)
    );
}

#[test]
fn ordinary_expression_errors_and_whole_query_failures_are_not_capacity() {
    let graph = empty();
    let budget = QueryBudget {
        strict_numeric_capacity: true,
        ..QueryBudget::unlimited()
    };
    let expression = PreparedQuery::parse("SELECT (1/0 AS ?v) {}").unwrap();
    let result = query_prepared_with_budget_detailed(&graph, &expression, &budget).unwrap();
    assert_eq!(result.rows, vec![vec![None]]);
    let unsupported = PreparedQuery::parse("CONSTRUCT {} WHERE {}").unwrap();
    assert!(matches!(
        query_prepared_with_budget_detailed(&graph, &unsupported, &budget),
        Err(QueryFailure::Evaluation(_))
    ));
    let prepared = PreparedQuery::parse("ASK {}").unwrap();
    assert_eq!(
        query_prepared_with_budget_detailed(&graph, &prepared, &budget)
            .unwrap()
            .rows,
        vec![vec![]]
    );
}

#[test]
fn reentrant_function_failure_does_not_poison_the_outer_query() {
    let mut registry = sparq_engine::FunctionRegistry::new();
    registry.register("urn:nested", |_| {
        let child = PreparedQuery::parse("SELECT (9223372036854775807 + 1 AS ?v) {}").unwrap();
        assert_eq!(
            query_prepared_with_budget_detailed(
                &empty(),
                &child,
                &QueryBudget {
                    strict_numeric_capacity: true,
                    ..QueryBudget::unlimited()
                }
            )
            .unwrap_err(),
            QueryFailure::Capacity(EvaluationCapacity::NumericRepresentation)
        );
        Ok(oxrdf::Term::Literal(oxrdf::Literal::from(true)))
    });
    let outer = PreparedQuery::parse("SELECT (<urn:nested>() AS ?v) {}").unwrap();
    let budget = QueryBudget {
        max_rows: Some(1),
        strict_numeric_capacity: true,
        ..QueryBudget::unlimited()
    };
    let result = sparq_engine::with_functions(&registry, || {
        query_prepared_with_budget_detailed(&empty(), &outer, &budget)
    })
    .unwrap();
    assert_eq!(
        result.rows,
        vec![vec![Some(oxrdf::Term::Literal(oxrdf::Literal::from(true)))]]
    );
}
