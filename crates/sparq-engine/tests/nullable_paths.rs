// [GPT-6] Published SPARQL 1.1 property-path endpoint and multiplicity rules.
use sparq_core::Graph;
use sparq_engine::{QueryBudget, query, query_with_budget};

fn rows(graph: &Graph, body: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<_> = query(graph, &format!("PREFIX ex:<http://example.org/> {body}"))
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

fn repeated(name: &str, count: usize) -> Vec<Vec<String>> {
    vec![vec![format!("<http://example.org/{name}>")]; count]
}

#[test]
fn absent_constant_alternatives_preserve_each_zero_length_solution() {
    for graph in [
        Graph::new(),
        Graph::load_str(
            "<http://example.org/a> <http://example.org/p> <http://example.org/b> .",
            "ntriples",
        )
        .unwrap(),
    ] {
        for path in ["(ex:p*|ex:p*)", "^(ex:p?|ex:p*)", "((ex:p*)+|ex:p*)"] {
            assert_eq!(
                rows(&graph, &format!("SELECT ?o {{ex:missing {path} ?o}}")),
                repeated("missing", 2),
                "{path}"
            );
            assert_eq!(
                rows(&graph, &format!("SELECT ?s {{?s {path} ex:missing}}")),
                repeated("missing", 2),
                "{path}"
            );
            assert_eq!(
                rows(
                    &graph,
                    &format!("SELECT * {{ex:missing {path} ex:missing}}")
                ),
                vec![Vec::<String>::new(); 2],
                "{path}"
            );
            assert!(
                rows(&graph, &format!("SELECT * {{ex:missing {path} ex:other}}")).is_empty(),
                "{path}"
            );
        }
    }
}

#[test]
fn sequence_midpoints_remain_variables_even_when_their_hint_is_absent() {
    let graph = Graph::new();
    // Section 18.4: sequence is a join with a fresh variable midpoint.
    // The far var-var p* ranges nodes(G), which is empty here.
    for path in [
        "(ex:p*/ex:p*)",
        "((ex:p*/ex:p*)|ex:q)",
        "^((ex:p*/ex:p*)|ex:q)",
    ] {
        assert!(
            rows(&graph, &format!("SELECT ?o {{ex:missing {path} ?o}}")).is_empty(),
            "{path}"
        );
        assert!(
            rows(&graph, &format!("SELECT ?s {{?s {path} ex:missing}}")).is_empty(),
            "{path}"
        );
        assert_eq!(
            rows(
                &graph,
                &format!("SELECT * {{ex:missing {path} ex:missing}}")
            ),
            vec![Vec::<String>::new()],
            "{path}"
        );
    }
}

#[test]
fn quantifiers_collapse_duplicate_routes_and_keep_nested_constant_seeds() {
    let graph = Graph::new();
    for path in [
        "(ex:p*)+",
        "(ex:p*)*",
        "(ex:p?)?",
        "(ex:p*|ex:p*)+",
        "^(ex:p*)+",
    ] {
        assert_eq!(
            rows(&graph, &format!("SELECT ?o {{ex:missing {path} ?o}}")),
            repeated("missing", 1),
            "{path}"
        );
        assert_eq!(
            rows(&graph, &format!("SELECT ?s {{?s {path} ex:missing}}")),
            repeated("missing", 1),
            "{path}"
        );
        assert!(
            rows(&graph, &format!("SELECT ?s ?o {{?s {path} ?o}}")).is_empty(),
            "{path}"
        );
    }
    assert!(rows(&graph, "SELECT ?o {ex:missing (ex:p*/ex:p*)+ ?o}").is_empty());
}

#[test]
fn variable_node_domain_excludes_predicate_only_and_values_terms() {
    let graph = Graph::load_str(
        "<http://example.org/a> <http://example.org/p> <http://example.org/b> .",
        "ntriples",
    )
    .unwrap();
    assert_eq!(
        rows(&graph, "SELECT ?o {ex:p (ex:p*|ex:p*) ?o}"),
        repeated("p", 2)
    );
    for path in ["ex:p*", "ex:p?", "(ex:p*)+", "(ex:p*|ex:p*)"] {
        assert!(
            rows(
                &graph,
                &format!("SELECT ?s {{VALUES ?s {{ex:p ex:missing}} ?s {path} ?o}}")
            )
            .is_empty(),
            "{path}"
        );
        assert!(
            rows(
                &graph,
                &format!("SELECT ?o {{VALUES ?o {{ex:p ex:missing}} ?s {path} ?o}}")
            )
            .is_empty(),
            "{path}"
        );
        assert_eq!(
            rows(
                &graph,
                &format!("SELECT ?s ?o {{VALUES ?s {{ex:missing}} OPTIONAL {{?s {path} ?o}}}}")
            ),
            vec![vec![
                "<http://example.org/missing>".to_owned(),
                "UNBOUND".to_owned()
            ]],
            "{path}"
        );
    }
    assert_eq!(
        rows(&graph, "SELECT ?s {?s (ex:p*|ex:p*) ?s}"),
        [repeated("a", 2), repeated("b", 2)].concat()
    );
}

#[test]
fn empty_default_view_does_not_import_the_backing_graphs_node_domain() {
    let graph = Graph::load_str(
        "<http://example.org/a> <http://example.org/p> <http://example.org/b> .",
        "ntriples",
    )
    .unwrap();
    assert_eq!(
        rows(
            &graph,
            "SELECT ?o FROM NAMED ex:unused {ex:a (ex:p*|ex:p*) ?o}"
        ),
        repeated("a", 2)
    );
    assert!(
        rows(
            &graph,
            "SELECT ?s FROM NAMED ex:unused {?s (ex:p*|ex:p*) ?s}"
        )
        .is_empty()
    );
}

#[test]
fn constant_seed_expansion_is_subject_to_the_row_budget() {
    let text =
        "PREFIX ex:<http://example.org/> SELECT ?o { ex:missing ((ex:p*|ex:p*)|(ex:p*|ex:p*)) ?o }";
    let err = query_with_budget(
        &Graph::new(),
        text,
        &QueryBudget {
            max_rows: Some(3),
            ..QueryBudget::default()
        },
    )
    .unwrap_err();
    assert!(err.contains("budget"), "{err}");
}
