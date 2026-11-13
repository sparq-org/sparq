#![cfg(feature = "sparql-rl")]

use oxrdf::{Literal, Term};
use sparq_core::Graph;
use sparq_shacl::sparql_rl::{
    check_well_formed, infer, infer_with_limits, stratify, Limits, RuleErrorKind,
};

const BASE: &str = "http://example.org/rules.srl";
fn graph(text: &str) -> Graph {
    Graph::load_str(text, "turtle").unwrap()
}
fn infer_graph(data: &Graph, source: &str) -> Graph {
    sparq_shacl::graph_from_triples(infer(data, source, BASE).unwrap())
}
fn objects(data: &Graph, predicate: &str) -> Vec<Term> {
    let mut terms: Vec<_> = sparq_shacl::view::GraphView::new(data)
        .triples(None, Some(predicate), None)
        .into_iter()
        .map(|[_, _, o]| o)
        .collect();
    terms.sort_by_key(ToString::to_string);
    terms
}

#[test]
fn data_seeds_are_visible_to_rules_but_absent_from_base_only_patterns() {
    let data = graph("<urn:a> <urn:p> 1 .");
    let source="DATA { <urn:a> <urn:p> 1 . <urn:b> <urn:p> 2 } RULE {?s <urn:q> ?o} WHERE {?s <urn:p> ?o} RULE {?s <urn:r> ?o} WHERE DATA {?s <urn:p> ?o}";
    let out = infer_graph(&data, source);
    assert_eq!(out.len(), 4);
    assert_eq!(objects(&out, "urn:p"), vec![Literal::from(2).into()]);
    assert_eq!(
        objects(&out, "urn:q"),
        vec![Literal::from(1).into(), Literal::from(2).into()]
    );
    assert_eq!(objects(&out, "urn:r"), vec![Literal::from(1).into()]);
    assert_eq!(data.len(), 1);
}

#[test]
fn negation_uses_the_requested_graph_and_correlates_inner_filters() {
    let data = graph("<urn:a> <urn:p> 1. <urn:b> <urn:p> 2.");
    let source="DATA { <urn:a> <urn:blocked> true } RULE {?s <urn:ok> ?v} WHERE {?s <urn:p> ?v NOT {?x <urn:blocked> true FILTER(?x=?s)}} RULE {?s <urn:baseok> ?v} WHERE {?s <urn:p> ?v NOT DATA {?s <urn:blocked> true}}";
    let out = infer_graph(&data, source);
    assert_eq!(objects(&out, "urn:ok"), vec![Literal::from(2).into()]);
    assert_eq!(
        objects(&out, "urn:baseok"),
        vec![Literal::from(1).into(), Literal::from(2).into()]
    );
}

#[test]
fn set_errors_drop_solutions_and_filters_remain_sequential() {
    let data = graph("<urn:a> <urn:p> 0, 2 .");
    let source="RULE {?s <urn:q> ?y} WHERE {?s <urn:p> ?x SET(?y := 8/?x) FILTER(?y<5)} RULE {?s <urn:r> ?y} WHERE {?s <urn:p> ?x FILTER(?x>0) SET(?y := ?x+1)} RULE {<urn:a> <urn:bad> true} WHERE {SET(?x := 1/0)}";
    let out = infer_graph(&data, source);
    assert_eq!(objects(&out, "urn:q").len(), 1);
    assert!(objects(&out, "urn:bad").is_empty());
    assert_eq!(objects(&out, "urn:r"), vec![Literal::from(3).into()]);
    assert!(sparq_engine::ask(&out, "ASK {?s <urn:q> ?v FILTER(?v=4)}").unwrap());
}

#[test]
fn group_projects_only_keys_and_preserves_each_outer_solution() {
    let data = graph("<urn:a> <urn:p> 1,2; <urn:q> 10,20 .");
    let source="RULE {?s <urn:outer> ?o; <urn:count> ?n} WHERE {?s <urn:p> ?o GROUP BY ?s {?s <urn:q> ?v} [?n := COUNT(*)]} RULE {<urn:a> <urn:empty> ?n} WHERE {GROUP {?s <urn:absent> ?v} [?n := COUNT(*)]} RULE {<urn:a> <urn:noGroup> ?n} WHERE {?s <urn:p> ?o GROUP BY ?s {?s <urn:absent> ?v} [?n := COUNT(*)]}";
    let out = infer_graph(&data, source);
    assert_eq!(
        objects(&out, "urn:outer"),
        vec![Literal::from(1).into(), Literal::from(2).into()]
    );
    assert_eq!(objects(&out, "urn:count"), vec![Literal::from(2).into()]);
    assert_eq!(objects(&out, "urn:empty"), vec![Literal::from(0).into()]);
    assert!(objects(&out, "urn:noGroup").is_empty());
}

#[test]
fn scope_tracks_hidden_names_fresh_targets_and_aggregate_arguments() {
    for source in [
        "RULE {?s <urn:q> ?x} WHERE {?s <urn:p> ?o NOT {?s <urn:r> ?x}}",
        "RULE {} WHERE {?s <urn:p> ?o SET(?o := 1)}",
        "RULE {} WHERE {NOT {?s <urn:p> ?o} ?s <urn:r> 1}",
        "RULE {} WHERE {?s <urn:p> ?o GROUP BY ?s {?s <urn:q> ?o} []}",
        "RULE {} WHERE {GROUP {?s <urn:p> ?o} [?x := SUM(?missing)]}",
        "RULE {} WHERE {GROUP {?s <urn:p> ?o} [?x := COUNT(*), ?x := SUM(?o)]}",
        "RULE {} WHERE {GROUP BY ?s {?s <urn:p> ?o} [] GROUP BY ?s {?s <urn:q> ?o} []}",
    ] {
        assert_eq!(
            check_well_formed(source, BASE).unwrap_err().kind,
            RuleErrorKind::WellFormedness,
            "{source}"
        );
    }
    check_well_formed(
        "RULE {?s <urn:q> ?n} WHERE {GROUP BY ?s {?s <urn:p> ?o} [?n := COUNT(*)]}",
        BASE,
    )
    .unwrap();
}

#[test]
fn stratification_rejects_closed_cycles_but_not_distinct_constants() {
    let closed = "RULE {?s <urn:p> 1} WHERE {?s <urn:q> 1 NOT {?s <urn:p> 1}}";
    assert_eq!(
        stratify(closed, BASE).unwrap_err().kind,
        RuleErrorKind::Stratification
    );
    let run_once = "RULE {?s <urn:p> ?x} WHERE {?s <urn:p> ?o SET(?x := ?o+1)}";
    assert_eq!(
        stratify(run_once, BASE).unwrap_err().kind,
        RuleErrorKind::Stratification
    );
    let triple_term = "RULE {?s <urn:p> <<(?s <urn:p> ?o)>>} WHERE {?s <urn:p> ?o}";
    assert_eq!(
        stratify(triple_term, BASE).unwrap_err().kind,
        RuleErrorKind::Stratification
    );
    assert!(stratify(
        "RULE {<urn:a> <urn:p> 1} WHERE {NOT {<urn:b> <urn:p> 1}}",
        BASE
    )
    .is_ok());
    assert!(stratify(
        "RULE {<urn:a> <urn:p> <urn:b>} WHERE {NOT {?s <urn:p> ?s}}",
        BASE
    )
    .is_ok());
    assert!(stratify(
        "RULE {?s <urn:p> ?x} WHERE DATA {?s <urn:p> ?o SET(?x := ?o+1)}",
        BASE
    )
    .is_ok());
}

#[test]
fn operational_limits_return_errors_without_changing_the_base() {
    let data = graph("<urn:a> <urn:p> <urn:b>. <urn:b> <urn:p> <urn:c>. <urn:c> <urn:p> <urn:d>.");
    let source="RULE {?s <urn:q> ?o} WHERE {?s <urn:p> ?o} RULE {?s <urn:q> ?o} WHERE {?s <urn:q> ?x. ?x <urn:p> ?o}";
    for limits in [
        Limits {
            max_rules: 0,
            ..Limits::default()
        },
        Limits {
            max_dependency_checks: 0,
            ..Limits::default()
        },
        Limits {
            max_iterations: 1,
            ..Limits::default()
        },
        Limits {
            max_queries: 1,
            ..Limits::default()
        },
        Limits {
            max_rows: 1,
            ..Limits::default()
        },
        Limits {
            max_inferred_triples: 1,
            ..Limits::default()
        },
        Limits {
            max_inferred_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_construct_triples: 1,
            ..Limits::default()
        },
        Limits {
            max_construct_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_query_bytes: 0,
            ..Limits::default()
        },
        Limits {
            timeout: std::time::Duration::ZERO,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            infer_with_limits(&data, source, BASE, &limits)
                .unwrap_err()
                .kind,
            RuleErrorKind::Limit,
            "{limits:?}"
        );
        assert_eq!(data.len(), 3);
        assert!(objects(&data, "urn:q").is_empty());
    }
    assert_eq!(
        infer_with_limits(
            &data,
            "DATA {<urn:x> <urn:p> 1,2}",
            BASE,
            &Limits {
                max_inferred_triples: 1,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .kind,
        RuleErrorKind::Limit
    );
}

#[test]
fn imports_are_refused_even_without_rules_and_now_is_frozen() {
    assert_eq!(
        infer(&graph(""), "IMPORTS <missing.srl>", BASE)
            .unwrap_err()
            .kind,
        RuleErrorKind::Imports
    );
    let out=infer_graph(&graph(""),"RULE {<urn:a> <urn:p> ?n} WHERE {SET(?n := NOW())} RULE {<urn:b> <urn:p> ?n} WHERE {SET(?n := NOW())}");
    let times = objects(&out, "urn:p");
    assert_eq!(times.len(), 2);
    assert_eq!(times[0], times[1]);
}

#[test]
fn blank_bindings_and_head_identity_survive_multiple_queries() {
    let data = graph("_:a <urn:p> _:b. _:b <urn:p> _:c.");
    let out=infer_graph(&data,"RULE {?s <urn:q> ?o} WHERE {?s <urn:p> ?x. ?x <urn:p> ?o FILTER(isBlank(?s))} RULE {[] <urn:value> ?o; <urn:again> ?o} WHERE {?s <urn:q> ?o}");
    assert_eq!(out.len(), 3);
    let view = sparq_shacl::view::GraphView::new(&out);
    let values = view.triples(None, Some("urn:value"), None);
    let again = view.triples(None, Some("urn:again"), None);
    assert_eq!(values[0][0], again[0][0]);
    assert_eq!(values[0][2], again[0][2]);
}

#[test]
fn base_and_prefix_scope_are_retained_for_runtime_iri_construction() {
    assert_eq!(
        check_well_formed("IMPORTS <missing.srl>", BASE)
            .unwrap_err()
            .kind,
        RuleErrorKind::Imports
    );
    assert_eq!(
        stratify("IMPORTS <missing.srl>", BASE).unwrap_err().kind,
        RuleErrorKind::Imports
    );
    let out = infer_graph(&graph(""), "BASE <http://one.example/> PREFIX : <v/> RULE {:s :p ?iri} WHERE {SET(?iri := IRI('child'))} BASE <http://two.example/> RULE {<urn:s> <urn:p> ?iri} WHERE {SET(?iri := IRI('child'))}");
    assert_eq!(
        objects(&out, "http://one.example/v/p"),
        vec![oxrdf::NamedNode::new("http://one.example/child")
            .unwrap()
            .into()]
    );
    assert_eq!(
        objects(&out, "urn:p"),
        vec![oxrdf::NamedNode::new("http://two.example/child")
            .unwrap()
            .into()]
    );
}

#[test]
fn group_errors_empty_late_keys_and_repeated_keys_have_defined_results() {
    let data = graph("<urn:a> <urn:p> 1,2; <urn:text> 'bad'.");
    let out=infer_graph(&data,"RULE {<urn:a> <urn:badSum> ?n} WHERE {GROUP {?s <urn:text> ?v} [?n := SUM(?v)]} RULE {<urn:a> <urn:lateKey> ?n} WHERE {GROUP BY ?k {?s <urn:absent> ?v SET(?k := ?s)} [?n := COUNT(*)]} RULE {?s <urn:count> ?n} WHERE {GROUP BY ?s ?s {?s <urn:p> ?v} [?n := COUNT(*)]}");
    assert!(objects(&out, "urn:badSum").is_empty());
    assert!(objects(&out, "urn:lateKey").is_empty());
    assert_eq!(objects(&out, "urn:count"), vec![Literal::from(2).into()]);
}

#[test]
fn dependency_matching_respects_rdf_subject_and_predicate_domains() {
    for source in [
        "RULE {?s <urn:p> 1} WHERE {?s <urn:q> 1 NOT {?x <urn:p> ?x}}",
        "RULE {<urn:s> ?p 1} WHERE {<urn:s> <urn:prop> ?p NOT {<urn:s> ?x ?x}}",
    ] {
        assert_eq!(stratify(source, BASE).unwrap(), vec![0]);
    }
    assert_eq!(
        stratify(
            "RULE {?s <urn:p> ?s} WHERE {?s <urn:q> 1 NOT {?x <urn:p> ?x}}",
            BASE
        )
        .unwrap_err()
        .kind,
        RuleErrorKind::Stratification
    );
}
