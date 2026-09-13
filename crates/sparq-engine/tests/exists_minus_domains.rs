// [GPT-6] Published SPARQL 1.1 §18.6 substitution and §18.5 MINUS domains.
use sparq_core::Graph;
use sparq_engine::{QueryBudget, query, query_with_budget};

const DATA: &str = "@prefix ex:<http://example.org/> .
ex:a ex:score 10 . ex:b ex:score 20 . ex:c ex:score 30 .";

fn rows(body: &str) -> Vec<Vec<String>> {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let mut rows: Vec<_> = query(&graph, &format!("PREFIX ex:<http://example.org/> {body}"))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| term.unwrap().to_string())
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

fn iris(values: &[&str]) -> Vec<Vec<String>> {
    values
        .iter()
        .map(|v| vec![format!("<http://example.org/{v}>")])
        .collect()
}

#[test]
fn captured_subject_is_removed_from_both_minus_domains() {
    // After replacing ?s with its bound IRI, ?m and ?z are disjoint.
    let body = "{?s ex:score ?m MINUS {?s ex:score ?z}}";
    assert_eq!(
        rows(&format!(
            "SELECT ?s {{?s ex:score ?n FILTER EXISTS {body}}}"
        )),
        iris(&["a", "b", "c"])
    );
    assert!(
        rows(&format!(
            "SELECT ?s {{?s ex:score ?n FILTER NOT EXISTS {body}}}"
        ))
        .is_empty()
    );
}

#[test]
fn uncaptured_shared_variable_still_removes_matches() {
    assert!(
        rows("SELECT ?s {?s ex:score ?n FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?m}}}")
            .is_empty()
    );
}

#[test]
fn capture_only_in_minus_right_side_is_applied_before_subtraction() {
    assert_eq!(
        rows(
            "SELECT ?s {VALUES ?s {ex:a ex:missing} FILTER EXISTS {ex:a ex:score ?m MINUS {?s ex:score ?m}}}"
        ),
        iris(&["missing"])
    );
}

#[test]
fn unbound_outer_cell_keeps_its_variable_in_the_domains() {
    assert_eq!(
        rows(
            "SELECT ?tag {VALUES (?s ?tag) {(ex:a \"bound\") (UNDEF \"unbound\")} FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?z}}}"
        ),
        vec![vec!["\"bound\"".to_string()]]
    );
}

#[test]
fn captured_literal_uses_term_identity_and_not_numeric_equality() {
    assert_eq!(
        rows(
            "SELECT ?n {VALUES ?n {10 \"010\"^^<http://www.w3.org/2001/XMLSchema#integer>} FILTER EXISTS {ex:a ex:score ?n MINUS {?s ex:score ?n}}}"
        ),
        vec![vec![
            "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string()
        ]]
    );
}

#[test]
fn filters_unions_and_nested_minus_use_substituted_domains() {
    assert_eq!(
        rows(
            "SELECT ?s {?s ex:score ?n FILTER EXISTS {{?s ex:score ?m} UNION {?s ex:score ?m FILTER(false)} MINUS {?s ex:score ?z FILTER(?z > ?n)}}}"
        ),
        iris(&["a", "b", "c"])
    );
    assert_eq!(
        rows(
            "SELECT ?s {?s ex:score ?n FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?z MINUS {?s ex:score ?z}}}}"
        ),
        iris(&["a", "b", "c"])
    );
}

#[test]
fn intermediate_budget_exhaustion_cannot_turn_into_false_exists() {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let budget = QueryBudget {
        max_rows: Some(2),
        ..QueryBudget::default()
    };
    let text = "PREFIX ex:<http://example.org/> SELECT ?s {VALUES ?s {ex:a} FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?z}}}";
    assert!(query_with_budget(&graph, text, &budget).is_err());
}

#[test]
fn duplicate_outer_rows_and_zero_column_inner_solutions_are_preserved() {
    assert_eq!(
        rows(
            "SELECT ?s {VALUES ?s {ex:a ex:a ex:b} FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?z}}}"
        ),
        iris(&["a", "a", "b"])
    );
    assert_eq!(
        rows(
            "SELECT ?s {VALUES ?s {ex:a ex:missing} FILTER EXISTS {?s ex:score 10 MINUS {?s ex:score 10}}}"
        ),
        iris(&["a"])
    );
}

#[test]
fn local_bound_and_unbound_union_columns_keep_their_domains() {
    assert_eq!(
        rows(
            "SELECT ?s {VALUES ?s {ex:a} FILTER EXISTS { { ?s ex:score ?m FILTER(BOUND(?m)) } UNION {ex:b ex:score ?z} MINUS {?s ex:score ?m} }}"
        ),
        iris(&["a"])
    );
}

#[test]
fn undefined_capture_shapes_retain_native_practical_behavior() {
    // These are native compatibility controls, not Recommendation expectations.
    let graph = Graph::load_str("_:a <http://example.org/score> 10 .", "turtle").unwrap();
    let text = "PREFIX ex:<http://example.org/> SELECT ?s {?s ex:score ?n FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?z}}}";
    assert!(query(&graph, text).unwrap().rows.is_empty());
    assert!(rows("SELECT ?s {VALUES ?s {ex:a} FILTER EXISTS {?s ex:score ?m FILTER(BOUND(?s)) MINUS {?s ex:score ?z}}}").is_empty());
    assert!(rows("SELECT ?s {VALUES ?s {ex:a} FILTER EXISTS { {SELECT ?s ?m {?s ex:score ?m}} MINUS {?s ex:score ?z}}}").is_empty());
}

#[test]
fn active_named_graph_uses_exact_term_bindings_in_its_dictionary() {
    let graph = Graph::load_dataset("@prefix ex:<http://example.org/> . ex:outside ex:score 99 . ex:g { ex:a ex:score 10 . ex:b ex:score 20 . }", "trig").unwrap();
    let result = query(&graph, "PREFIX ex:<http://example.org/> SELECT ?g ?s {GRAPH ?g {?s ex:score ?n FILTER EXISTS {?s ex:score ?m MINUS {?s ex:score ?z}}}} ORDER BY ?s").unwrap();
    assert_eq!(result.rows.len(), 2);
    for (row, subject) in result.rows.iter().zip(["a", "b"]) {
        assert_eq!(
            row[0].as_ref().unwrap().to_string(),
            "<http://example.org/g>"
        );
        assert_eq!(
            row[1].as_ref().unwrap().to_string(),
            format!("<http://example.org/{subject}>")
        );
    }
}
