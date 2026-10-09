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

/// Every named graph's contents, for comparing a store before and after an update.
fn snapshot(s: &PodStore) -> std::collections::BTreeMap<String, String> {
    s.graph
        .named
        .iter()
        .filter(|(n, _)| !n.to_string().contains("urn:sparq:"))
        .map(|(n, g)| {
            let mut rows: Vec<String> = g
                .store
                .scan(&[None, None, None])
                .rows
                .iter()
                .map(|r| format!("{r:?}"))
                .collect();
            rows.sort();
            (n.to_string(), rows.join("\n"))
        })
        .collect()
}

const OUT: &[&str] = &["<https://pod.ex/out>"];

/// `GRAPH ?var` write targets: (update, the graphs it changes, or `None` when refused).
/// Bob reads `out` and `pub` and writes `out` and `wonly`, so a target that can bind
/// `pub` must be refused, and one that binds only unreadable graphs binds nothing.
const VARIABLE_TARGETS: &[(&str, Option<&[&str]>)] = &[
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } WHERE { GRAPH ?g { } }", None),
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } USING NAMED <https://pod.ex/out> WHERE { GRAPH ?g { } }", Some(OUT)),
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } USING NAMED <https://pod.ex/private> USING NAMED <https://pod.ex/wonly> WHERE { GRAPH ?g { } }", Some(&[])),
    ("WITH <https://pod.ex/out> INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } WHERE { GRAPH ?g { } }", None),
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } WHERE { GRAPH ?g { } MINUS { GRAPH ?g { ?s ?p \"public\" } } }", Some(OUT)),
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } WHERE { GRAPH ?g { } FILTER NOT EXISTS { GRAPH ?g { ?s ?p \"out\" } } }", None),
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> ?n } } WHERE { { SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g } }", None),
    ("INSERT { GRAPH ?g { <urn:f> <urn:p> ?n } } WHERE { { SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g HAVING (?g = <https://pod.ex/out>) } }", Some(OUT)),
    ("DELETE { GRAPH ?g { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }", None),
    ("DELETE { GRAPH ?g { ?s ?p ?o } } USING NAMED <https://pod.ex/out> WHERE { GRAPH ?g { ?s ?p ?o } }", Some(OUT)),
    // A refused operation after a permitted one leaves the store untouched.
    ("INSERT DATA { GRAPH <https://pod.ex/out> { <urn:m> <urn:p> 1 } } ; INSERT { GRAPH ?g { <urn:f> <urn:p> 1 } } WHERE { GRAPH ?g { } }", None),
];

/// The graphs a `GRAPH ?var` template writes are the graphs authorized: a permitted update
/// changes exactly the graphs expected, all writable by bob, and a refused one changes
/// nothing.
#[test]
fn variable_write_targets_are_authorized_as_written() {
    for (upd, expected) in VARIABLE_TARGETS {
        let mut s = store();
        let before = snapshot(&s);
        let r = s.update_as(&bob(), upd);
        let after = snapshot(&s);
        assert_eq!(r.is_ok(), expected.is_some(), "{upd}: {r:?}");
        let mut changed: Vec<&str> = after
            .iter()
            .filter(|(g, rows)| before.get(*g) != Some(*rows))
            .map(|(g, _)| g.as_str())
            .chain(before.keys().filter(|g| !after.contains_key(*g)).map(String::as_str))
            .collect();
        changed.sort_unstable();
        assert_eq!(changed, expected.unwrap_or(&[]), "{upd}");
    }
}

/// A directory-backed copy of [`store`], and the directory.
fn durable_store(tag: &str) -> (PodStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("sparq_update_scope_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    Graph::load_dataset(&small_dataset(), "nquads")
        .expect("loads")
        .save(&dir)
        .expect("saves");
    (reopen(&dir), dir)
}

fn reopen(dir: &std::path::Path) -> PodStore {
    let mut s = PodStore::new(Graph::open(dir).expect("opens"));
    s.materialize_wac().expect("wac");
    s
}

/// A multi-operation update with a variable write target commits through the store's
/// durable path: it survives a reopen, and so do later writes. A refused one changes
/// neither memory nor disk.
#[test]
fn multi_operation_updates_stay_durable() {
    let (mut s, dir) = durable_store("ok");
    s.update_as(
        &bob(),
        "INSERT DATA { GRAPH <https://pod.ex/out> { <urn:m> <urn:p> \"multi\" } } ; \
         INSERT { GRAPH ?g { <urn:f> <urn:p> \"var\" } } USING NAMED <https://pod.ex/out> WHERE { GRAPH ?g { } }",
    )
    .expect("permitted");
    s.update_as(
        &bob(),
        "INSERT DATA { GRAPH <https://pod.ex/out> { <urn:l> <urn:p> \"later\" } }",
    )
    .expect("a later write");
    let live = out(&s);
    drop(s);
    let back = out(&reopen(&dir));
    for v in ["multi", "var", "later"] {
        assert!(live.contains(v) && back.contains(v), "{v} lost: {back}");
    }
    std::fs::remove_dir_all(&dir).ok();

    let (mut s, dir) = durable_store("refused");
    let before = out(&s);
    s.update_as(
        &bob(),
        "INSERT DATA { GRAPH <https://pod.ex/out> { <urn:m> <urn:p> \"multi\" } } ; \
         INSERT { GRAPH ?g { <urn:f> <urn:p> \"var\" } } WHERE { GRAPH ?g { } }",
    )
    .expect_err("pub is not writable");
    assert_eq!(out(&s), before, "memory changed");
    drop(s);
    assert_eq!(out(&reopen(&dir)), before, "disk changed");
    std::fs::remove_dir_all(&dir).ok();
}
