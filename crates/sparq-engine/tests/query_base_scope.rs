//! #6479 — a query run re-entrantly on the same thread (here from an extension
//! function) must not leak its BASE IRI, or its lack of one, into the enclosing query.
//! The base is a thread-local installed by each query entry point; it is now restored
//! when the nested query returns, errors, or unwinds.

use oxrdf::{Literal, Term};
use sparq_core::Graph;
use sparq_engine::{FunctionRegistry, query, query_with_functions};

const NESTED: &str = "http://ex/nested";

fn graph() -> Graph {
    Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap()
}

/// A registry whose `ex:nested(q)` runs the SPARQL text `q` against `g` on the calling
/// thread and returns a fixed literal (or an expression error when the inner query fails).
fn nesting_registry(g: Graph) -> FunctionRegistry {
    let g = std::sync::Arc::new(g);
    let mut reg = FunctionRegistry::new();
    reg.register(NESTED, move |args: &[Term]| {
        let Some(Term::Literal(q)) = args.first() else {
            return Err("expected a query string".into());
        };
        query(&g, q.value()).map(|_| Term::Literal(Literal::new_simple_literal("ran")))
    });
    reg
}

/// Runs `outer` (which calls `ex:nested(...)` before resolving a relative IRI) and
/// returns the projected `?r` cell.
fn resolved(outer: &str) -> Option<String> {
    let g = graph();
    let reg = nesting_registry(graph());
    let r = query_with_functions(&g, outer, &reg).expect("outer query");
    assert_eq!(r.rows.len(), 1, "one row expected for: {outer}");
    r.rows[0][0].as_ref().map(|t| t.to_string())
}

fn outer_query(inner: &str) -> String {
    format!(
        r#"BASE <https://outer.example/>
           SELECT ?r WHERE {{
             BIND(<{NESTED}>("{inner}") AS ?n)
             BIND(IRI("resource") AS ?r)
           }}"#
    )
}

#[test]
fn nested_query_with_its_own_base_does_not_replace_the_outer_base() {
    let r = resolved(&outer_query(
        "BASE <https://inner.example/> SELECT * WHERE { ?s ?p ?o }",
    ));
    assert_eq!(r.as_deref(), Some("<https://outer.example/resource>"));
}

#[test]
fn nested_query_without_a_base_does_not_clear_the_outer_base() {
    let r = resolved(&outer_query("SELECT * WHERE { ?s ?p ?o }"));
    assert_eq!(r.as_deref(), Some("<https://outer.example/resource>"));
}

#[test]
fn nested_query_that_fails_after_installing_its_base_still_restores_the_outer_base() {
    // Parses (so the inner entry point installs its BASE) but is rejected as an
    // unsupported form by the SELECT/ASK entry point.
    let r = resolved(&outer_query(
        "BASE <https://inner.example/> CONSTRUCT WHERE { ?s ?p ?o }",
    ));
    assert_eq!(r.as_deref(), Some("<https://outer.example/resource>"));
}

#[test]
fn base_does_not_persist_into_a_later_top_level_query() {
    let g = graph();
    query(
        &g,
        "BASE <https://first.example/> SELECT * WHERE { ?s ?p ?o }",
    )
    .unwrap();
    // No BASE: a relative IRI() is a type error, so ?r is unbound.
    let r = query(&g, r#"SELECT ?r WHERE { BIND(IRI("resource") AS ?r) }"#).unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], None);
}
