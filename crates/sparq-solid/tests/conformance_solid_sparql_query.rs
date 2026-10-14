//! sq-gq28y (issue #1546): run the VENDORED upstream **query-semantics
//! conformance suite** of the *Access-Controlled SPARQL Query over a Solid Pod* Editor's
//! Draft (`jeswr/solid-sparql-query`) against the real `PodStore` read path, and assert
//! sparq passes EVERY case.
//!
//! The suite (manifest + fixture) is a pinned copy of the upstream `test-suite/query-semantics`
//! directory — see `tests/conformance/solid-sparql-query/PROVENANCE.md` for the upstream repo +
//! commit. This is the "contribute spec tests and pass all of them" standing directive from
//! issue #1546: the same manifest is portable to any other implementation of the spec.
//!
//! It asserts the SPEC-CONFORMANT default behaviour (empty standing default graph + per-query
//! union opt-in), so it is gated OFF under the `legacy-union-default-graph` escape hatch (which
//! deliberately reinstates the non-conformant union-always default).
#![cfg(not(feature = "legacy-union-default-graph"))]

use oxrdf::Term;
use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};
use std::collections::BTreeSet;

const SUITE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance/solid-sparql-query");

fn load_manifest() -> serde_json::Value {
    let path = format!("{SUITE_DIR}/manifest.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {path}: {e}"))
}

fn build_store(fixture_file: &str) -> PodStore {
    let path = format!("{SUITE_DIR}/{fixture_file}");
    let nq = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let mut s = PodStore::new(Graph::load_dataset(&nq, "nquads").expect("fixture loads"));
    s.materialize_wac().expect("materialize WAC");
    s
}

/// Resolve a manifest requester name to a `Session`. `null` (JSON) => the anonymous/public
/// requester (`Session::default()`); a WebID string => an authenticated agent.
fn session_for<'a>(requesters: &'a serde_json::Value, name: &str) -> Session<'a> {
    match requesters.get(name) {
        Some(serde_json::Value::String(webid)) => {
            Session { agent: Some(webid.as_str()), client: None, issuer: None, now: None }
        }
        Some(serde_json::Value::Null) => Session::default(),
        other => panic!("unknown requester {name:?} in manifest: {other:?}"),
    }
}

/// The IRI strings bound to `var` across all solutions of a single-answer projection.
fn iri_bindings(result: &sparq_engine::QueryResult, var: &str) -> BTreeSet<String> {
    let col = result
        .vars
        .iter()
        .position(|v| v.as_str() == var)
        .unwrap_or_else(|| panic!("query does not project ?{var}; vars = {:?}", result.vars));
    result
        .rows
        .iter()
        .filter_map(|row| row.get(col).and_then(|t| t.as_ref()))
        .filter_map(|t| match t {
            Term::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

/// The lexical value of the single literal bound to `var` in the single solution row.
fn scalar_literal(result: &sparq_engine::QueryResult, var: &str) -> String {
    let col = result
        .vars
        .iter()
        .position(|v| v.as_str() == var)
        .unwrap_or_else(|| panic!("query does not project ?{var}"));
    let row = result.rows.first().unwrap_or_else(|| panic!("no solution row for ?{var}"));
    match row.get(col).and_then(|t| t.as_ref()) {
        Some(Term::Literal(l)) => l.value().to_owned(),
        other => panic!("expected a literal for ?{var}, got {other:?}"),
    }
}

fn str_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default()
}

/// The whole suite runs as ONE test so a failure names the exact case; the pass count is
/// checked against the manifest so a silently-dropped case cannot pass vacuously.
#[test]
fn solid_sparql_query_query_semantics_conformance() {
    let manifest = load_manifest();
    assert_eq!(
        manifest["conformanceClass"].as_str(),
        Some("query-semantics"),
        "unexpected conformance class",
    );
    let fixture = manifest["fixture"].as_str().expect("manifest.fixture");
    let requesters = &manifest["requesters"];
    let cases = manifest["cases"].as_array().expect("manifest.cases is an array");
    assert!(!cases.is_empty(), "the conformance suite has no cases");

    let store = build_store(fixture);
    let mut passed = 0usize;

    for case in cases {
        let id = case["id"].as_str().expect("case.id");
        let requester = case["requester"].as_str().expect("case.requester");
        let query = case["query"].as_str().expect("case.query");
        let session = session_for(requesters, requester);
        let expect = &case["expect"];
        let kind = expect["type"].as_str().expect("expect.type");

        match kind {
            "rowCount" => {
                let want = expect["value"].as_u64().expect("expect.value") as usize;
                let got = store
                    .query_as(&session, Mode::Read, query)
                    .unwrap_or_else(|e| panic!("[{id}] query errored (must be absent-not-error): {e}"))
                    .rows
                    .len();
                assert_eq!(got, want, "[{id}] rowCount");
            }
            "ask" => {
                let want = expect["value"].as_bool().expect("expect.value");
                let got = store
                    .ask_as(&session, Mode::Read, query)
                    .unwrap_or_else(|e| panic!("[{id}] ASK errored: {e}"));
                assert_eq!(got, want, "[{id}] ask");
            }
            "count" => {
                let var = expect["var"].as_str().unwrap_or("n");
                let want = expect["value"].as_u64().expect("expect.value").to_string();
                let res = store
                    .query_as(&session, Mode::Read, query)
                    .unwrap_or_else(|e| panic!("[{id}] count query errored: {e}"));
                assert_eq!(scalar_literal(&res, var), want, "[{id}] count(?{var})");
            }
            "graphBindings" => {
                let var = expect["var"].as_str().unwrap_or("g");
                let includes = str_list(expect, "includes");
                let excludes = str_list(expect, "excludes");
                let res = store
                    .query_as(&session, Mode::Read, query)
                    .unwrap_or_else(|e| panic!("[{id}] graph query errored: {e}"));
                let bound = iri_bindings(&res, var);
                for want in &includes {
                    assert!(bound.contains(want), "[{id}] ?{var} must include {want}; got {bound:?}");
                }
                for forbidden in &excludes {
                    assert!(!bound.contains(forbidden), "[{id}] ?{var} must NOT include {forbidden}; got {bound:?}");
                }
            }
            other => panic!("[{id}] unknown expect.type {other:?}"),
        }
        passed += 1;
    }

    assert_eq!(passed, cases.len(), "every case in the suite must run and pass");
    // Guard against silent shrinkage of the vendored suite.
    assert!(passed >= 15, "the query-semantics suite unexpectedly shrank to {passed} cases");
}
