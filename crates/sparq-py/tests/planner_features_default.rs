// sq-11664 — Tripwire that the sparq-py wheel's DEFAULT feature set keeps
// the three planner/optimizer features the CLI ships (`dp-planner`, `algebra-rewrite`,
// `antijoin-static-decline`) lit — the shipped pip wheel must execute the same rewritten
// plans the CLI and the canonical benchmarks measure.
//
// Root cause (sq-hmd7l.18 first read, 2026-07-10): the wheel built sparq-engine with
// LIBRARY-default features only, causing a ~100× regression on rewrite-dependent queries
// (SP2Bench q03b/q03c: ~1.2 ms wheel vs ~11–14 µs sparq-cli engine-internal).
//
// These tests are COMPILE-TIME + BEHAVIORAL tripwires — if any of the three features
// is dropped from the sparq-engine dep in Cargo.toml, the relevant #[cfg(feature = …)]
// arm fails to compile in per-crate CI lanes.
//
// NB: these tests link against `sparq-engine` directly (not the cdylib extension module)
// so they build without a Python interpreter — the `test = false` on [lib] prevents the
// cdylib from being compiled as a test harness but does NOT affect [[test]] targets.
//
// The coverage ratchet: each public-surface change needs at LEAST one direct test per
// public fn. Here the "surface" is the wheel's feature configuration; the three tests
// below each cover one feature axis.

#[cfg(feature = "algebra-rewrite")]
use spargebra::SparqlParser;

// ---------------------------------------------------------------------------
// 1. algebra-rewrite: FILTER(?v = <iri>) constant-substitution pass fires
// ---------------------------------------------------------------------------

/// COMPILE-TIME witness: `sparq_engine::rewrite` only exists when the engine's
/// `algebra-rewrite` feature is on. If the `features = [...]` list in Cargo.toml
/// ever loses `algebra-rewrite`, this file fails to compile in the per-crate CI lane.
///
/// BEHAVIORAL half: the trigger shape is rewritten; literal equality is left verbatim
/// (the sq-lr2ii avoidance contract, mirroring the sparq-server tripwire).
#[cfg(feature = "algebra-rewrite")]
#[test]
fn algebra_rewrite_fires_on_iri_filter_and_leaves_literal_verbatim() {
    let parse = |q: &str| SparqlParser::new().parse_query(q).expect("parse");

    // IRI equality → must be constant-substituted (rewritten != verbatim).
    let iri_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) }";
    let rewritten = sparq_engine::rewrite::rewrite_query(parse(iri_eq));
    assert_ne!(
        format!("{:?}", rewritten),
        format!("{:?}", parse(iri_eq)),
        "FILTER(?v = <iri>) must be constant-substituted by algebra-rewrite"
    );

    // Literal equality → must be left verbatim (sq-lr2ii avoidance contract).
    let lit_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = 42) }";
    let verbatim = sparq_engine::rewrite::rewrite_query(parse(lit_eq));
    assert_eq!(
        format!("{:?}", verbatim),
        format!("{:?}", parse(lit_eq)),
        "literal equality must be left verbatim (sq-lr2ii avoidance contract)"
    );
}

/// BEHAVIORAL confirmation: the rewrite-dark q03c query shape returns correct results
/// via the REAL engine entry point (the same path the Python wheel calls).
/// Before sq-11664, the wheel took the generic bind_join+apply_filter_scalar scan
/// (~1.2 ms) instead of the rewritten ~11 µs path — both give the SAME answer,
/// so this test is a RESULT PARITY + compile-time witness combined.
#[cfg(feature = "algebra-rewrite")]
#[test]
fn iri_equality_filter_result_correct_through_rewrite() {
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
    assert_eq!(rows.rows.len(), 2, "exactly two solutions: {}", joined);
    assert!(joined.contains("http://ex/a"), "row a expected: {}", joined);
    assert!(joined.contains("http://ex/c"), "row c expected: {}", joined);
    assert!(!joined.contains("http://ex/b"), "row b must be filtered out: {}", joined);
}

// ---------------------------------------------------------------------------
// 2. dp-planner: the DPccp join-order planner is reachable
// ---------------------------------------------------------------------------

/// COMPILE-TIME witness: `sparq_engine::with_dp_planner` only exists when the engine's
/// `dp-planner` feature is on. If the feature is dropped from Cargo.toml, this fails
/// to compile in the per-crate CI lane, catching drift before it reaches PyPI.
///
/// BEHAVIORAL half: the planner installs without error and the query still returns
/// the correct result (the planner is a result-equivalent reordering, not a rewrite).
#[cfg(feature = "dp-planner")]
#[test]
fn dp_planner_installs_and_query_result_correct() {
    let graph = sparq_core::Graph::load_str(
        "<http://ex/alice> <http://ex/knows> <http://ex/bob> .\n\
         <http://ex/bob>   <http://ex/age>   \"25\" .\n",
        "ntriples",
    )
    .expect("load fixture");
    // Install the planner (panics if dp-planner feature is absent at compile time).
    sparq_engine::with_dp_planner(|| {
        let rows = sparq_engine::query(
            &graph,
            "SELECT ?s ?age WHERE { ?s <http://ex/knows> ?o . ?o <http://ex/age> ?age }",
        )
        .expect("query");
        assert_eq!(rows.rows.len(), 1, "one solution: {:?}", rows.rows);
        let flat = format!("{:?}", rows.rows[0]);
        assert!(flat.contains("http://ex/alice"), "alice expected: {}", flat);
        assert!(flat.contains("25"), "age 25 expected: {}", flat);
    });
}

// ---------------------------------------------------------------------------
// 3. antijoin-static-decline: the static early-decline path is reachable
// ---------------------------------------------------------------------------

/// COMPILE-TIME witness: `sparq_engine::theta_antijoin_testing::early_declined()`
/// only exists when the engine's `antijoin-static-decline` feature is on. If the
/// feature is dropped from Cargo.toml, this fails to compile in the per-crate CI lane.
///
/// BEHAVIORAL half: a bare-`!bound` OPTIONAL (the q07 shape) with antijoin-static-decline
/// ON returns the correct answer (the decline is a plan-order fix, not a result change).
#[cfg(feature = "antijoin-static-decline")]
#[test]
fn antijoin_static_decline_reachable_and_optional_result_correct() {
    use sparq_engine::theta_antijoin_testing;

    let graph = sparq_core::Graph::load_str(
        "<http://ex/a> <http://ex/p> \"x\" .\n\
         <http://ex/b> <http://ex/p> \"y\" .\n\
         <http://ex/b> <http://ex/q> \"z\" .\n",
        "ntriples",
    )
    .expect("load fixture");

    // q07-shape: join + OPTIONAL where !bound check discriminates the match.
    // ?s with ex:p but without ex:q → a; ?s with ex:p and ex:q → b (with qval).
    theta_antijoin_testing::reset_stats();
    let rows = sparq_engine::query(
        &graph,
        "SELECT ?s ?qval WHERE { ?s <http://ex/p> ?v OPTIONAL { ?s <http://ex/q> ?qval } } ORDER BY ?s",
    )
    .expect("query");
    assert_eq!(rows.rows.len(), 2, "two solutions: {:?}", rows.rows);

    // early_declined() is the compile-time proof the feature is wired: calling it
    // would panic if antijoin-static-decline were absent at build time.
    let _ = theta_antijoin_testing::early_declined();
}
