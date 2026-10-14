//! (sq-7d3dj.30.13) Tripwire that the server's DEFAULT feature set keeps the
//! engine's pre-execution algebra rewrite pass (#1735) lit — the shipped `sparq-server`
//! binary must execute the same rewritten plans the CLI and the canonical benchmarks
//! measure.
//!
//! Gated on this crate's own `algebra-rewrite` feature (default ON), so:
//!   - every default lane (`cargo test -p sparq-server`, the feature-matrix legs, the
//!     coverage lanes) COMPILES AND RUNS it — dropping `algebra-rewrite` from `default`
//!     makes the default lanes silently skip it, but the compile-time witness below
//!     also stops existing anywhere the feature is off, so a per-crate default run
//!     that still expects it fails loudly at review time via the feature-matrix name
//!     contract (`sparq-server` legs never pass `--no-default-features`);
//!   - the deliberate opt-out builds (`--no-default-features` pure serialiser,
//!     `--no-default-features --features server,jsonld` rewrite-dark binary) compile
//!     it out, as intended.
//!
//! NB: under a whole-`--workspace` build, feature unification could light
//! `sparq-engine/algebra-rewrite` via a sibling crate (e.g. sparq-cli); the per-crate
//! CI lanes (`cargo test -p sparq-server --features …`) are where this tripwire is
//! authoritative.
#![cfg(feature = "algebra-rewrite")]

use spargebra::SparqlParser;

/// COMPILE-TIME witness: `sparq_engine::rewrite` only exists when the engine's
/// `algebra-rewrite` feature is on — if the `algebra-rewrite = ["sparq-engine/algebra-rewrite"]`
/// forwarding ever breaks, this file fails to compile in the per-crate lanes.
/// Behavioral half: the trigger shape is rewritten; a literal equality is left verbatim
/// (the sq-lr2ii avoidance contract).
#[test]
fn algebra_rewrite_forwarded_and_fires_on_iri_equality() {
    let parse = |q: &str| SparqlParser::new().parse_query(q).expect("parse");
    let iri_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) }";
    let rewritten = sparq_engine::rewrite::rewrite_query(parse(iri_eq));
    assert_ne!(
        format!("{:?}", rewritten),
        format!("{:?}", parse(iri_eq)),
        "FILTER(?v = <iri>) should be constant-substituted by the rewrite pass"
    );
    let lit_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = 1) }";
    let verbatim = sparq_engine::rewrite::rewrite_query(parse(lit_eq));
    assert_eq!(
        format!("{:?}", verbatim),
        format!("{:?}", parse(lit_eq)),
        "literal equality must be left verbatim (sq-lr2ii avoidance contract)"
    );
}

/// REAL evaluation path: the server's query handlers funnel string queries through
/// `sparq_engine` on a `sparq_core::Graph` — run the q03-shape trigger query through
/// that same engine entry point and assert the rewritten plan is result-correct.
#[test]
fn iri_equality_filter_rows_correct_through_rewrite() {
    let graph = sparq_core::Graph::load_str(
        "<http://ex/a> <http://ex/p> <http://ex/target> .\n\
         <http://ex/b> <http://ex/p> <http://ex/other> .\n\
         <http://ex/c> <http://ex/p> <http://ex/target> .\n",
        "ntriples",
    )
    .expect("load fixture");
    let rows = sparq_engine::query(
        &graph,
        "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) } ORDER BY ?s",
    )
    .expect("query");
    let flat: Vec<String> = rows.rows.iter().map(|r| format!("{:?}", r)).collect();
    let joined = flat.join("\n");
    assert_eq!(rows.rows.len(), 2, "exactly two solutions: {joined}");
    assert!(joined.contains("http://ex/a"), "row a expected: {joined}");
    assert!(joined.contains("http://ex/c"), "row c expected: {joined}");
    assert!(!joined.contains("http://ex/b"), "row b must be filtered out: {joined}");
}
