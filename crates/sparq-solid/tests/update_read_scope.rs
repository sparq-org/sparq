//! Every pattern a SPARQL Update evaluates sees only what the updating session may read.
//!
//! Bob's grants: `out` read+write, `pub` read only, `wonly` write only, `private` nothing.
//! The store's default graph also holds a triple. For every update form that evaluates a
//! pattern (`INSERT … WHERE`, `DELETE … WHERE`, `DELETE WHERE`, `DELETE/INSERT … WHERE`,
//! with `USING`, `USING NAMED` and `WITH`, and with `OPTIONAL`, `MINUS`, `EXISTS`,
//! `NOT EXISTS`, `BIND`, a subquery and an aggregate around the read), an update whose
//! condition needs a graph Bob cannot read is refused or matches nothing, and the store
//! keeps no trace of the unreadable data. Conditions over readable graphs still work.

use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_solid::{Mode, PodStore, Session};

const BOB: &str = "https://bob.ex/card#me";
const SECRET: &str = "s3cret";

fn acl(graph: &str, modes: &[&str]) -> String {
    let mut nq = format!(
        "<{graph}.acl#b> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <{graph}.acl> .\n\
         <{graph}.acl#b> <http://www.w3.org/ns/auth/acl#accessTo> <{graph}> <{graph}.acl> .\n\
         <{graph}.acl#b> <http://www.w3.org/ns/auth/acl#agent> <{BOB}> <{graph}.acl> .\n"
    );
    for m in modes {
        nq += &format!(
            "<{graph}.acl#b> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#{m}> <{graph}.acl> .\n"
        );
    }
    nq
}

fn small_dataset() -> String {
    let mut nq = format!(
        "<urn:s> <urn:p> \"{SECRET}\" <https://pod.ex/private> .\n\
         <urn:w> <urn:p> \"{SECRET}\" <https://pod.ex/wonly> .\n\
         <urn:d> <urn:p> \"{SECRET}\" .\n\
         <urn:t> <urn:p> \"public\" <https://pod.ex/pub> .\n\
         <urn:o> <urn:p> \"out\" <https://pod.ex/out> .\n"
    );
    nq += &acl("https://pod.ex/out", &["Read", "Write"]);
    nq += &acl("https://pod.ex/pub", &["Read"]);
    nq += &acl("https://pod.ex/wonly", &["Write"]);
    nq
}

fn store() -> PodStore {
    let mut s = PodStore::new(Graph::load_dataset(&small_dataset(), "nquads").expect("loads"));
    s.materialize_wac().expect("wac");
    s
}

fn bob() -> Session<'static> {
    Session {
        agent: Some(BOB),
        client: None,
        issuer: None,
        now: None,
    }
}

/// Everything bob can read back from `out`.
fn out(s: &PodStore) -> String {
    s.query_json_as(
        &bob(),
        Mode::Read,
        "SELECT ?s ?o WHERE { GRAPH <https://pod.ex/out> { ?s ?p ?o } }",
    )
    .expect("query")
}

fn wonly_len(s: &PodStore) -> usize {
    let term = oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked("https://pod.ex/wonly"));
    s.graph
        .named
        .iter()
        .find(|(n, _)| *n == term)
        .map(|(_, g)| g.store.scan(&[None, None, None]).rows.len())
        .unwrap_or(0)
}

/// Updates that must be refused outright, leaving `out` and `wonly` exactly as they were.
const REFUSED: &[&str] = &[
    // The copy-out shape: INSERT … WHERE over an unreadable graph.
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { GRAPH <https://pod.ex/private> { ?s ?p ?o } }",
    // DELETE … WHERE conditioned on an unreadable graph.
    "DELETE { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { GRAPH <https://pod.ex/out> { ?s ?p ?o } GRAPH <https://pod.ex/private> { ?x ?y ?z } }",
    // DELETE WHERE on a graph bob may write but not read.
    "DELETE WHERE { GRAPH <https://pod.ex/wonly> { ?s ?p ?o } }",
    // DELETE/INSERT … WHERE with the read behind OPTIONAL, MINUS, EXISTS and NOT EXISTS.
    "DELETE { GRAPH <https://pod.ex/out> { ?s ?p ?o } } INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?v } } WHERE { GRAPH <https://pod.ex/out> { ?s ?p ?o } OPTIONAL { GRAPH <https://pod.ex/private> { ?a ?b ?v } } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> 1 } } WHERE { GRAPH <https://pod.ex/out> { ?s ?p ?o } MINUS { GRAPH <https://pod.ex/private> { ?s ?p ?o } } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> 1 } } WHERE { FILTER EXISTS { GRAPH <https://pod.ex/private> { ?s ?p \"s3cret\" } } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> 1 } } WHERE { FILTER NOT EXISTS { GRAPH <https://pod.ex/private> { ?s ?p \"nope\" } } }",
    // BIND(EXISTS …), a subquery and an aggregate.
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> ?b } } WHERE { BIND(EXISTS { GRAPH <https://pod.ex/private> { ?s ?p ?o } } AS ?b) }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> ?o } } WHERE { { SELECT ?o WHERE { GRAPH <https://pod.ex/private> { ?s ?p ?o } } } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> ?n } } WHERE { { SELECT (COUNT(*) AS ?n) WHERE { GRAPH <https://pod.ex/private> { ?s ?p ?o } } } }",
    // USING and WITH make the unreadable graph the default graph.
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } USING <https://pod.ex/private> WHERE { ?s ?p ?o }",
    "WITH <https://pod.ex/private> INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { ?s ?p ?o }",
    // A default-graph pattern with no USING reads the store's default graph.
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { ?s ?p ?o }",
];

/// Updates that are permitted but can only match readable graphs, so nothing unreadable
/// reaches `out`.
const CONFINED: &[&str] = &[
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }",
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } USING NAMED <https://pod.ex/private> WHERE { GRAPH ?g { ?s ?p ?o } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> \"leak\" } } WHERE { FILTER EXISTS { GRAPH ?g { ?s ?p \"s3cret\" } } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> \"leak\" } } WHERE { FILTER NOT EXISTS { GRAPH ?g { ?s ?p \"s3cret\" } } FILTER(false) }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> ?n } } WHERE { { SELECT (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p \"s3cret\" } } } }",
];

/// One `PodStore` update entry point, called as bob.
type Entry = fn(&mut PodStore, &str, &QueryBudget) -> Result<(), String>;

#[test]
fn unreadable_conditions_are_refused_on_every_update_entry_point() {
    let budget = QueryBudget::unlimited();
    for upd in REFUSED {
        let entry_points: [(&str, Entry); 4] = [
            ("update_as", |s, u, _| s.update_as(&bob(), u)),
            ("update_as_acp", |s, u, _| s.update_as_acp(&bob(), u)),
            ("update_as_with_budget", |s, u, b| {
                s.update_as_with_budget(&bob(), u, b)
            }),
            ("update_as_acp_with_budget", |s, u, b| {
                s.update_as_acp_with_budget(&bob(), u, b)
            }),
        ];
        for (name, run) in entry_points {
            let mut s = store();
            let (before_out, before_wonly) = (out(&s), wonly_len(&s));
            let r = run(&mut s, upd, &budget);
            assert!(r.is_err(), "{name} must refuse: {upd}");
            assert_eq!(out(&s), before_out, "{name} changed out: {upd}");
            assert_eq!(wonly_len(&s), before_wonly, "{name} changed wonly: {upd}");
        }
    }
}

#[test]
fn variable_graphs_range_over_the_read_view_only() {
    for upd in CONFINED {
        let mut s = store();
        s.update_as(&bob(), upd)
            .unwrap_or_else(|e| panic!("{upd}: {e}"));
        let after = out(&s);
        assert!(
            !after.contains(SECRET) && !after.contains("leak"),
            "{upd} leaked: {after}"
        );
    }
    // The first one did copy what bob can read.
    let mut s = store();
    s.update_as(&bob(), CONFINED[0]).expect("copy");
    assert!(out(&s).contains("public"));
}

#[test]
fn readable_conditions_still_work() {
    let mut s = store();
    s.update_as(
        &bob(),
        "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { GRAPH <https://pod.ex/pub> { ?s ?p ?o } }",
    )
    .expect("copy from a readable graph");
    assert!(out(&s).contains("public"));
    s.update_as(
        &bob(),
        "DELETE WHERE { GRAPH <https://pod.ex/out> { <urn:o> ?p ?o } }",
    )
    .expect("delete by a readable condition");
    assert!(!out(&s).contains("\"out\""));
    s.update_as(
        &bob(),
        "INSERT DATA { GRAPH <https://pod.ex/wonly> { <urn:x> <urn:p> 1 } }",
    )
    .expect("a blind write needs no read");
    s.update_as(
        &bob(),
        "INSERT { GRAPH <https://pod.ex/out> { <urn:k> <urn:p> 2 } } WHERE { }",
    )
    .expect("an empty condition reads nothing");
}

/// Updates whose conditions range over every graph the session may read.
const VARIABLE: &[&str] = &[
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> 1 } } WHERE { FILTER EXISTS { GRAPH ?g { ?s ?p ?o } } }",
    "INSERT { GRAPH <https://pod.ex/out> { ?s ?p ?o } } USING NAMED <https://pod.ex/private> USING NAMED <https://pod.ex/pub> WHERE { GRAPH ?g { ?s ?p ?o } }",
    "WITH <https://pod.ex/out> INSERT { <urn:f> <urn:p> ?o } WHERE { GRAPH ?g { ?s ?p ?o } }",
    "INSERT { GRAPH <https://pod.ex/out> { <urn:f> <urn:p> ?n } } WHERE { { SELECT (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } } } }",
    // A variable write target: the authorization check evaluates the condition as well.
    "INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } WHERE { GRAPH ?g { ?s ?p ?o } FILTER(?g = <https://pod.ex/out>) }",
];

/// An unreadable graph is never evaluated: a large unreadable graph cannot push a
/// condition over budget, so the outcome is the same whatever it holds.
#[test]
fn unreadable_graphs_are_never_evaluated() {
    let budget = QueryBudget { max_rows: Some(50), ..QueryBudget::unlimited() };
    let mut big = String::new();
    for i in 0..1000 {
        big += &format!("<urn:s{i}> <urn:p> \"{SECRET}\" <https://pod.ex/private> .\n");
    }
    for upd in VARIABLE {
        let mut small = store();
        let mut large = PodStore::new(
            Graph::load_dataset(&(big.clone() + &small_dataset()), "nquads").expect("loads"),
        );
        large.materialize_wac().expect("wac");
        small
            .update_as_with_budget(&bob(), upd, &budget)
            .unwrap_or_else(|e| panic!("{upd}: {e}"));
        large
            .update_as_with_budget(&bob(), upd, &budget)
            .unwrap_or_else(|e| panic!("hidden data changed the outcome of {upd}: {e}"));
        assert_eq!(out(&small), out(&large), "{upd}");
        assert!(!out(&large).contains(SECRET), "{upd} leaked");
    }
}
