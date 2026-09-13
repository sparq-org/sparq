// [GPT-6] SPARQL 1.1 substitution and property-path multiset regressions.
use sparq_core::Graph;
use sparq_engine::{query, query_with_budget, QueryBudget};

fn graph() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://example.org/> .\n\
         ex:a ex:score 10; ex:knows ex:b, ex:c .\n\
         ex:b ex:score 20; ex:knows ex:c .\n\
         ex:c ex:score 30 .",
        "turtle",
    )
    .unwrap()
}

fn rows(body: &str) -> Vec<Vec<String>> {
    let result = query(
        &graph(),
        &format!("PREFIX ex: <http://example.org/> {body}"),
    )
    .unwrap();
    let mut rows: Vec<_> = result
        .rows
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|t| t.map_or("UNBOUND".into(), |t| t.to_string()))
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

fn iris(names: &[&str]) -> Vec<Vec<String>> {
    names
        .iter()
        .map(|s| vec![format!("<http://example.org/{s}>")])
        .collect()
}

#[test]
fn exists_filter_reads_bound_outer_variable_without_an_inner_column() {
    let pattern = "{ ?s ex:knows ?o . ?o ex:score ?m FILTER(?m > ?n + 10) }";
    assert_eq!(
        rows(&format!(
            "SELECT ?s WHERE {{ ?s ex:score ?n FILTER EXISTS {pattern} }}"
        )),
        iris(&["a"])
    );
    assert_eq!(
        rows(&format!(
            "SELECT ?s WHERE {{ ?s ex:score ?n FILTER NOT EXISTS {pattern} }}"
        )),
        iris(&["b", "c"])
    );
}

#[test]
fn exists_filter_only_pattern_observes_outer_binding() {
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n FILTER EXISTS { FILTER(?n = 10) } }"),
        iris(&["a"])
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n FILTER NOT EXISTS { FILTER(?n = 10) } }"),
        iris(&["b", "c"])
    );
}

#[test]
fn alternative_paths_retain_duplicate_solutions() {
    assert_eq!(
        rows("SELECT ?o WHERE { ex:a (ex:knows|ex:knows) ?o }"),
        iris(&["b", "b", "c", "c"])
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s (ex:knows|ex:knows) ex:c }"),
        iris(&["a", "a", "b", "b"])
    );
    assert_eq!(
        rows("SELECT * WHERE { ex:a (ex:knows|ex:knows) ex:b }").len(),
        2
    );
    assert_eq!(
        rows("SELECT DISTINCT ?o WHERE { ex:a (ex:knows|ex:knows) ?o }"),
        iris(&["b", "c"])
    );
}

#[test]
fn nested_alternative_and_sequence_multiply_path_counts() {
    assert_eq!(
        rows("SELECT ?o WHERE { ex:a ((ex:knows|ex:knows)/(ex:knows|ex:knows)) ?o }"),
        iris(&["c", "c", "c", "c"])
    );
    // Quantifiers preserve endpoint reachability, not route multiplicity.
    assert_eq!(
        rows("SELECT ?o WHERE { ex:a (ex:knows|ex:knows)+ ?o }"),
        iris(&["b", "c"])
    );
}

#[test]
fn path_multiplicity_respects_query_row_budget() {
    let query_text = "PREFIX ex: <http://example.org/> SELECT ?o WHERE {\n\
        ex:a ((ex:knows|ex:knows)|(ex:knows|ex:knows)) ?o }";
    let budget = QueryBudget {
        max_rows: Some(4),
        ..QueryBudget::default()
    };
    assert!(query_with_budget(&graph(), query_text, &budget).is_err());
}

#[test]
fn exists_keeps_term_identity_unbound_cells_and_nested_outer_values() {
    assert_eq!(
        rows("SELECT ?n WHERE { VALUES ?n { 10 UNDEF } FILTER EXISTS { FILTER(BOUND(?n)) } }"),
        vec![vec![
            "\"10\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string()
        ]]
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n FILTER EXISTS { FILTER EXISTS { FILTER(?n = 10) } } }"),
        iris(&["a"])
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n BIND(?n + 1 AS ?computed) FILTER EXISTS { FILTER(?computed = 11) } }"),
        iris(&["a"])
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n BIND(BNODE() AS ?blank) FILTER EXISTS { FILTER(isBlank(?blank) && sameTerm(?blank, ?blank)) } }"),
        iris(&["a", "b", "c"])
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n BIND(EXISTS { FILTER(?n = 10) } AS ?yes) FILTER(?yes) }"),
        iris(&["a"])
    );
}

#[test]
fn alternative_bags_reach_aggregates_and_both_unbound_endpoints() {
    assert_eq!(
        rows("SELECT ?s ?o WHERE { ?s (ex:knows|ex:knows) ?o }").len(),
        6
    );
    assert_eq!(
        rows("SELECT (COUNT(*) AS ?count) WHERE { ex:a (ex:knows|ex:knows) ?o }"),
        vec![vec![
            "\"4\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string()
        ]]
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ^(ex:knows|ex:knows) ex:a }"),
        iris(&["b", "b", "c", "c"])
    );
}

#[test]
fn nested_exists_matches_inherited_terms_even_without_parent_columns() {
    assert_eq!(
        rows(
            "SELECT ?s WHERE { ?s ex:score ?n FILTER EXISTS { FILTER EXISTS { ?s ex:score 10 } } }"
        ),
        iris(&["a"])
    );
    assert_eq!(
        rows("SELECT ?s WHERE { ?s ex:score ?n FILTER EXISTS { FILTER NOT EXISTS { ?s ex:score 10 } } }"),
        iris(&["b", "c"])
    );
}
