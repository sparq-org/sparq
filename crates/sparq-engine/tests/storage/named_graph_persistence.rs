//! (sq-3ui0, gh-45) End-to-end persistence test for NAMED graphs: a dataset
//! with several named graphs is queried with `GRAPH ?g { ... }`, then SAVED and REOPENED
//! (memory-mapped, the out-of-core path), and the SAME query must return byte-identical
//! rows. Before the fix, `Graph::open` dropped `named` entirely, so every `GRAPH`-scoped
//! query returned ZERO rows after a reopen — total data loss for the PSS shape (every
//! resource / `.acl` stored as a named graph whose IRI == the resource IRI).
//!
//! Pinned at the engine layer (not just core's structural round-trip) so the guarantee is
//! the one PSS actually relies on: a quad-level query over a reopened store.

#![cfg(feature = "parallel")] // engine default; mmap comes from sparq-core dev-dep

use sparq_core::Graph;
use sparq_engine::{query, QueryResult};

/// PSS-shaped dataset: a default graph plus named graphs whose IRI == the resource IRI,
/// including a `.acl` graph and a graph carrying a language-tagged literal.
const DATA: &str = "\
<http://ex/d1> <http://ex/p> <http://ex/d2> .
<http://res/a> <http://ex/p> <http://ex/b> <http://res/a> .
<http://res/a> <http://ex/label> \"caf\\u00e9\"@fr <http://res/a> .
<http://res/b> <http://ex/p> <http://ex/c> <http://res/b> .
<http://res/b/.acl> <http://acl#mode> <http://acl#Read> <http://res/b/.acl> .
";

/// Sort a result's rows into a stable string form for set-equality comparison.
fn rows_sorted(r: &QueryResult) -> Vec<Vec<String>> {
    let mut v: Vec<Vec<String>> = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string()).unwrap_or_default()).collect())
        .collect();
    v.sort();
    v
}

#[test]
fn graph_query_survives_save_open_roundtrip() {
    let g = Graph::load_dataset(DATA, "nquads").unwrap();
    assert_eq!(g.named.len(), 3, "fixture should hold 3 named graphs");

    // Quad-level queries to pin down: enumerate (?g, ?s, ?p, ?o), and a graph-scoped one.
    let q_all = "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }";
    let q_one = "SELECT ?s ?p ?o WHERE { GRAPH <http://res/b/.acl> { ?s ?p ?o } }";
    let q_default = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";

    let before_all = rows_sorted(&query(&g, q_all).unwrap());
    let before_one = rows_sorted(&query(&g, q_one).unwrap());
    let before_default = rows_sorted(&query(&g, q_default).unwrap());
    assert_eq!(before_all.len(), 4, "4 named-graph triples expected");
    assert_eq!(before_one.len(), 1, "the .acl graph has one triple");
    assert_eq!(before_default.len(), 1, "default graph has one triple");

    let dir = std::env::temp_dir().join(format!("sparq_ng_persist_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    g.save(&dir).unwrap();
    let g2 = Graph::open(&dir).unwrap();

    // The SAME queries over the reopened (mmap) store must return the SAME rows.
    assert_eq!(rows_sorted(&query(&g2, q_all).unwrap()), before_all, "GRAPH ?g rows changed after reopen");
    assert_eq!(rows_sorted(&query(&g2, q_one).unwrap()), before_one, "graph-scoped rows changed after reopen");
    assert_eq!(rows_sorted(&query(&g2, q_default).unwrap()), before_default, "default-graph rows changed after reopen");

    std::fs::remove_dir_all(&dir).ok();
}
