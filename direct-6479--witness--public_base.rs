// [GPT-6 Astra] Fixed public-API witness for issue #6479; no production instrumentation.
// Rust guideline compliant 2026-02-21
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{query, query_with_functions, FunctionRegistry, QueryResult};
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};

const OUTER: &str = "BASE <https://outer.example/> SELECT (IRI(<urn:base-probe>()) AS ?iri) WHERE {}";
const INNER_BASE: &str = "BASE <https://inner.example/> SELECT (IRI(\"child\") AS ?iri) WHERE {}";
const INNER_UNSUPPORTED: &str = "BASE <https://inner.example/> CONSTRUCT { ?s ?p ?o } WHERE {}";

#[derive(Clone, Copy, Debug)]
enum Case { NoNestedCall, InnerBase, InnerNoBase, InnerParseError, InnerPostInstallError }

fn expected_iri(result: &QueryResult, iri: &str) {
    assert_eq!(result.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["iri"]);
    assert_eq!(result.rows, vec![vec![Some(Term::NamedNode(NamedNode::new(iri).unwrap()))]]);
}

fn main() {
    for case in [Case::NoNestedCall, Case::InnerBase, Case::InnerNoBase,
        Case::InnerParseError, Case::InnerPostInstallError] {
        let graph = Graph::new();
        let nested = Graph::new();
        let thread = std::thread::current().id();
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = Arc::clone(&calls);
        let mut registry = FunctionRegistry::new();
        registry.register("urn:base-probe", move |args| {
            assert!(args.is_empty());
            assert_eq!(std::thread::current().id(), thread, "callback must run on the outer thread");
            callback_calls.fetch_add(1, Ordering::Relaxed);
            match case {
                Case::NoNestedCall => println!("case={case:?} child=none"),
                Case::InnerBase => {
                    let child = query(&nested, INNER_BASE).expect("child BASE query");
                    expected_iri(&child, "https://inner.example/child");
                    println!("case={case:?} child={child:?}");
                }
                Case::InnerNoBase => {
                    let child = query(&nested, "ASK {}").expect("child no-BASE query");
                    assert!(child.vars.is_empty());
                    assert_eq!(child.rows, vec![Vec::<Option<Term>>::new()]);
                    println!("case={case:?} child={child:?}");
                }
                Case::InnerParseError => {
                    let child = query(&nested, "not a SPARQL query");
                    assert!(child.is_err(), "parse-error control must reject its input");
                    println!("case={case:?} child={child:?}");
                }
                Case::InnerPostInstallError => {
                    let child = query(&nested, INNER_UNSUPPORTED);
                    assert_eq!(child.as_ref().err().map(String::as_str),
                        Some("only SELECT and ASK queries are supported"));
                    println!("case={case:?} child={child:?}");
                }
            }
            Ok(Term::Literal(Literal::new_simple_literal("resource")))
        });
        let outer = query_with_functions(&graph, OUTER, &registry);
        assert_eq!(calls.load(Ordering::Relaxed), 1, "callback must execute once");
        if matches!(case, Case::NoNestedCall | Case::InnerParseError) {
            expected_iri(outer.as_ref().expect("outer control query"), "https://outer.example/resource");
        }
        let restored = outer.as_ref().is_ok_and(|r| r.rows == vec![vec![Some(Term::NamedNode(
            NamedNode::new("https://outer.example/resource").unwrap()))]]);
        println!("case={case:?} callback_count=1 same_thread=true restored={restored} outer={outer:?}");
        let followup = query(&graph,
            "BASE <https://followup.example/> SELECT (IRI(\"resource\") AS ?iri) WHERE {}")
            .expect("independent followup BASE query");
        expected_iri(&followup, "https://followup.example/resource");
        let no_base = query(&graph, "SELECT (IRI(\"resource\") AS ?iri) WHERE {}")
            .expect("independent no-BASE query");
        assert_eq!(no_base.rows, vec![vec![None]]);
        println!("case={case:?} followup={followup:?} no_base_followup={no_base:?}");
    }
    println!("completed_fixed_cases=5");
}
