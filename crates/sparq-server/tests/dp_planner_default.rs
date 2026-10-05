//! [OPUS-5] (sq-7d3dj.30.15) Tripwire that the server's DEFAULT feature set keeps the
//! engine's DPccp join-order planner (sq-7d3dj.30.5 / #1732) lit — the shipped
//! `sparq-server` binary must plan joins the same way the CLI and the canonical
//! benchmarks do.
//!
//! Root cause this guards: sq-7d3dj.30.5 enabled `dp-planner` on `sparq-cli`'s
//! sparq-engine dep ONLY. The server never got it, so an HTTP-measured query planned
//! its joins with greedy GOO while the CLI-measured build planned the same query with
//! the cost-optimal bushy DP — two shipped surfaces disagreeing on join order, and the
//! HTTP numbers not comparable to the canonical CLI ones.
//!
//! Gated on this crate's own `dp-planner` feature (default ON), exactly like
//! `tests/algebra_rewrite_default.rs`, so:
//!   - every default lane (`cargo test -p sparq-server`, the feature-matrix legs, the
//!     coverage lanes) COMPILES AND RUNS it;
//!   - the deliberate opt-out builds (`--no-default-features` pure serialiser,
//!     `--no-default-features --features server,jsonld` greedy-GOO binary) compile it
//!     out, as intended — the `feature-matrix.yml` `server-no-default-features` lane
//!     runs `cargo test -p sparq-server --no-default-features`, so an UNGATED test here
//!     would red a legitimate configuration.
//!
//! Honest limitation, shared with `tests/algebra_rewrite_default.rs`: because the gate is
//! this crate's own feature, dropping `dp-planner` from `default` makes the default lanes
//! compile the file to ZERO tests rather than fail. The load-bearing half is therefore the
//! COMPILE-TIME witness — a broken `dp-planner = ["sparq-engine/dp-planner"]` forwarding
//! (feature present in `default`, engine feature not reached) fails the build outright.
//!
//! NB: under a whole-`--workspace` build, feature unification could light
//! `sparq-engine/dp-planner` via a sibling crate (e.g. sparq-cli); the per-crate CI
//! lanes (`cargo test -p sparq-server --features …`) are where this tripwire is
//! authoritative.
#![cfg(feature = "dp-planner")]

/// COMPILE-TIME witness: `sparq_engine::without_dp_planner` (and the `dp` module it
/// lives in) only exists when the engine's `dp-planner` feature is on — if the
/// `dp-planner = ["sparq-engine/dp-planner"]` forwarding, or its place in `default`,
/// ever breaks, this file fails to compile in the per-crate lanes.
///
/// BEHAVIORAL half: the planner is DEFAULT-ON once compiled (the engine's
/// `Install::Default` state), so the query below is DP-planned with no
/// `with_dp_planner` call on the request path — the exact situation an HTTP handler is
/// in. `without_dp_planner` is the inverse scope, and the two must agree on the answer
/// because the planner only reorders joins.
#[test]
fn dp_planner_forwarded_and_default_on_without_explicit_install() {
    // A 3-pattern connected BGP — the smallest shape `dp::plan()` accepts (it requires
    // n >= 3; for n <= 2 greedy GOO is already Cout-optimal, see sq-7d3dj.30.5).
    let graph = fixture();
    let q = "SELECT ?s ?name WHERE { \
             ?s <http://ex/knows> ?o . \
             ?o <http://ex/worksAt> ?org . \
             ?org <http://ex/name> ?name } ORDER BY ?name";

    // No `with_dp_planner` wrapper: this is the request path as the server runs it.
    let dp_planned = rows(&graph, q);

    // Same query with the DP explicitly disabled → greedy GOO. Result-equivalence is
    // the planner's core invariant (asserted exhaustively by sparq-engine's
    // `tests/query_features/dp_planner_differential.rs`); asserting it here proves the
    // engine linked into THIS crate is the DP-capable one and still answers correctly.
    let greedy = sparq_engine::without_dp_planner(|| rows(&graph, q));

    assert_eq!(
        dp_planned, greedy,
        "the DPccp planner only reorders joins — it must never change the answer"
    );
    assert_eq!(dp_planned.len(), 2, "expected two solutions: {dp_planned:?}");
    let joined = dp_planned.join("\n");
    assert!(joined.contains("Acme"), "org name Acme expected: {joined}");
    assert!(joined.contains("Umbrella"), "org name Umbrella expected: {joined}");
}

/// The tri-state install is scoped, not sticky: after a `without_dp_planner` scope ends
/// the server's next request is back on the compiled-in default. Without this, one
/// opt-out call site could silently leave a tokio worker thread greedy-planning for the
/// rest of the process's life — the drift this whole bead is about, in miniature.
#[test]
fn opt_out_scope_does_not_leak_into_later_queries() {
    let graph = fixture();
    let q = "SELECT ?s ?name WHERE { \
             ?s <http://ex/knows> ?o . \
             ?o <http://ex/worksAt> ?org . \
             ?org <http://ex/name> ?name } ORDER BY ?name";

    let before = rows(&graph, q);
    let _ = sparq_engine::without_dp_planner(|| rows(&graph, q));
    let after = rows(&graph, q);

    assert_eq!(before, after, "the opt-out scope must restore the previous install state");
    assert_eq!(after.len(), 2, "expected two solutions: {after:?}");
}

fn fixture() -> sparq_core::Graph {
    sparq_core::Graph::load_str(
        "<http://ex/alice> <http://ex/knows>    <http://ex/bob> .\n\
         <http://ex/carol> <http://ex/knows>    <http://ex/dave> .\n\
         <http://ex/bob>   <http://ex/worksAt>  <http://ex/org1> .\n\
         <http://ex/dave>  <http://ex/worksAt>  <http://ex/org2> .\n\
         <http://ex/org1>  <http://ex/name>     \"Acme\" .\n\
         <http://ex/org2>  <http://ex/name>     \"Umbrella\" .\n",
        "ntriples",
    )
    .expect("load fixture")
}

fn rows(graph: &sparq_core::Graph, q: &str) -> Vec<String> {
    sparq_engine::query(graph, q)
        .expect("query")
        .rows
        .iter()
        .map(|r| format!("{r:?}"))
        .collect()
}
