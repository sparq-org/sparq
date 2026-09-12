// [GPT-6] Manual SPARQL 1.1 oracles for optimizer scope boundaries.
use sparq_core::Graph;
use sparq_engine::{query, sip_testing};

fn run(graph: &Graph, text: &str, enabled: bool) -> Vec<Vec<String>> {
    let previous = sip_testing::set_enabled(enabled);
    let result = query(graph, &format!("PREFIX ex:<http://ex/> {text}"));
    sip_testing::set_enabled(previous);
    let mut rows: Vec<_> = result
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

#[test]
fn scope_binders_filters_and_modifiers_keep_their_bottom_up_meaning() {
    let graph = Graph::load_str(
        "@prefix ex:<http://ex/> . ex:a ex:p 1; ex:q 3 . ex:b ex:p 2 . ex:z ex:marker true .",
        "turtle",
    )
    .unwrap();
    let integer = |x| format!("\"{x}\"^^<http://www.w3.org/2001/XMLSchema#integer>");
    let ab = vec![
        vec!["<http://ex/a>".into(), "UNBOUND".into()],
        vec!["<http://ex/b>".into(), "UNBOUND".into()],
    ];
    let cases = [
        (
            "filter sibling-only reference",
            "SELECT ?s {VALUES ?x {ex:z} {?x ex:marker true . {?s ex:p ?m FILTER(?x=ex:z)}}}",
            vec![],
        ),
        (
            "BOUND sibling-only reference",
            "SELECT ?s {VALUES ?x {ex:z} {?x ex:marker true . {?s ex:p ?m FILTER(BOUND(?x))}}}",
            vec![],
        ),
        (
            "BIND sibling-only reference",
            "SELECT ?s ?copy {VALUES ?x {ex:z} {?x ex:marker true . {?s ex:p ?m BIND(?x AS ?copy)}}}",
            ab.clone(),
        ),
        (
            "OPTIONAL sibling-only condition",
            "SELECT ?s ?o {VALUES ?x {ex:z} {?x ex:marker true . {?s ex:p ?m OPTIONAL {?s ex:q ?o FILTER(?x=ex:z)}}}}",
            ab,
        ),
        (
            "subquery hidden variable",
            "SELECT ?m {VALUES ?x {ex:z} {?x ex:marker true . {SELECT ?m WHERE {?x ex:p ?m}}}}",
            vec![vec![integer(1)], vec![integer(2)]],
        ),
        (
            "subquery ORDER LIMIT",
            "SELECT ?m {VALUES ?x {ex:z} {?x ex:marker true . {SELECT ?m WHERE {?x ex:p ?m} ORDER BY ?m LIMIT 1}}}",
            vec![vec![integer(1)]],
        ),
        (
            "subquery DISTINCT",
            "SELECT ?m {VALUES ?x {ex:z} {?x ex:marker true . {SELECT DISTINCT ?m WHERE {?x ex:p ?m}}}}",
            vec![vec![integer(1)], vec![integer(2)]],
        ),
        (
            "subquery aggregate",
            "SELECT ?n {VALUES ?x {ex:z} {?x ex:marker true . {SELECT (COUNT(*) AS ?n) WHERE {?x ex:p ?m}}}}",
            vec![vec![integer(2)]],
        ),
        (
            "VALUES binder",
            "SELECT ?y {VALUES ?x {ex:z} {?x ex:marker true . VALUES ?y {ex:a ex:a ex:b}}}",
            vec![
                vec!["<http://ex/a>".into()],
                vec!["<http://ex/a>".into()],
                vec!["<http://ex/b>".into()],
            ],
        ),
    ];
    let mut failures = Vec::new();
    for (name, text, expected) in cases {
        for enabled in [false, true] {
            let actual = run(&graph, text, enabled);
            if actual != expected {
                failures.push(format!(
                    "{name}: SIP={enabled}, expected {expected:?}, actual {actual:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn positive_bgp_and_nonnullable_path_substitution_still_fires() {
    let graph = Graph::load_str(
        "@prefix ex:<http://ex/> . ex:a ex:p ex:b . ex:b ex:p ex:c .",
        "turtle",
    )
    .unwrap();
    for pattern in ["?s ex:p ?o", "?s ex:p+ ?o"] {
        let text = format!("SELECT ?s ?o {{VALUES ?s {{ex:a ex:a}} {pattern}}}");
        let expected = run(&graph, &text, false);
        sip_testing::reset_stats();
        assert_eq!(run(&graph, &text, true), expected);
        let (fired, _, bindings) = sip_testing::stats();
        assert!(
            fired && bindings > 0,
            "positive optimization must still execute: {pattern}"
        );
    }
}
