//! [SONNET-4.6] sq-mc06h — Tripwire that `sparq-mcp`'s DEFAULT feature set keeps the two
//! engine planner/optimizer features lit: `algebra-rewrite` (the pre-execution algebra
//! rewrite pass, #1735 / sq-7d3dj.30.1) and `dp-planner` (the DPccp bushy join-order
//! planner, sq-7d3dj.30.5 / #1732).
//!
//! Why this crate: sq-mc06h asks the per-surface question for each engine-embedding USER
//! surface, and `sparq-mcp` is NATIVE with no bundle-size floor, so it answers it the same
//! way sparq-cli / sparq-server / sparq-py already did — an agent's `query` / `construct`
//! tool call must execute the same plans the CLI and the canonical benchmarks measure,
//! rather than a rewrite-dark, greedy-GOO variant of the same engine. (`sparq-wasm` answers
//! it the other way: it keeps `sparq-engine` at `default-features = false` and forwards
//! neither feature, because that surface IS under a bundle-size gate — `wasm_bundle_bytes`
//! in `scripts/perf-gate.py`. Both it and the gui/src-tauri surface are outside this
//! crate's scope and are not touched here.)
//!
//! Guard structure — two layers, because they fail on different mutations:
//!
//! 1. `default_feature_set_keeps_both_planner_features_lit` reads THIS crate's `Cargo.toml`
//!    and asserts the `[features] default` list still names both. It is deliberately a
//!    MANIFEST assertion rather than the `cfg!(feature = …)` spelling the equivalent
//!    `sparq-server` tripwires use: a `cfg!`/`#[cfg]` guard can only observe how the test
//!    binary was compiled, so it either silently vanishes when the feature is dropped from
//!    `default` (the exact mutation it exists to catch) or falsely reds a deliberate
//!    `--no-default-features` build. Reading the manifest catches the drop and stays
//!    correct in every feature state, so it needs no `#[cfg]` gate.
//! 2. The per-feature tests below are `#[cfg]`-gated on THIS crate's feature, and each
//!    touches an engine item that only exists when the forwarding actually reaches
//!    `sparq-engine` — so a broken `algebra-rewrite = ["sparq-engine/algebra-rewrite"]` (or
//!    the `dp-planner` equivalent) fails to COMPILE instead of quietly passing.
//!
//! NB: under a whole-`--workspace` build, cargo feature unification could light the engine
//! features via a sibling crate (sparq-cli, sparq-server). The per-crate lanes
//! (`cargo test -p sparq-mcp …`) are where layer 2 is authoritative; layer 1 reads this
//! crate's own manifest and is authoritative everywhere.

/// Layer 1: the DEFAULT set itself. Removing `"algebra-rewrite"` or `"dp-planner"` from
/// `default` in `crates/sparq-mcp/Cargo.toml` makes this fail — the mutation that layer 2
/// alone cannot see, because a `#[cfg]`-gated test simply stops existing.
#[test]
fn default_feature_set_keeps_both_planner_features_lit() {
    let default_features = declared_default_features();
    for feature in ["algebra-rewrite", "dp-planner"] {
        assert!(
            default_features.iter().any(|f| f.as_str() == feature),
            "sparq-mcp's `[features] default` must keep `{}` (sq-mc06h): the MCP tool path \
             is a native surface with no bundle-size floor, so it must execute the same \
             plans the CLI and the canonical benchmarks measure — declared default set was \
             {:?}",
            feature,
            default_features
        );
    }
}

/// The feature names in `[features] default` of this crate's own manifest.
///
/// Hand-scanned rather than parsed with a `toml` dev-dependency: the one line it needs is
/// unambiguous (a `default = [...]` inside `[features]`), and the crate carries no toml
/// dependency today. Panics rather than returning empty on a shape it does not understand,
/// so a manifest reformat surfaces as a loud failure instead of a silently vacuous test.
fn declared_default_features() -> Vec<String> {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read sparq-mcp Cargo.toml");

    let mut in_features = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if !in_features || !line.starts_with("default") {
            continue;
        }
        let (_, list) = line.split_once('=').expect("`default` line has an `=`");
        let list = list.trim();
        let inner = list
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or_else(|| panic!("`default` must be a single-line array, got: {}", list));
        return inner
            .split(',')
            .map(|item| item.trim().trim_matches('"').to_string())
            .filter(|item| !item.is_empty())
            .collect();
    }
    panic!("no `default = [...]` line found in the [features] table of sparq-mcp's Cargo.toml");
}

// ---------------------------------------------------------------------------
// Layer 2a — `algebra-rewrite`
// ---------------------------------------------------------------------------

/// COMPILE-TIME witness: `sparq_engine::rewrite` only exists when the engine's
/// `algebra-rewrite` feature is on, so a broken forwarding fails the per-crate build.
///
/// BEHAVIORAL half: the trigger shape IS rewritten and a literal equality is left verbatim
/// (the sq-lr2ii avoidance contract) — i.e. the engine linked into `sparq-mcp` is one that
/// actually performs the substitution, not merely one that exposes the module.
#[cfg(feature = "algebra-rewrite")]
#[test]
fn algebra_rewrite_forwarded_and_fires_on_iri_equality() {
    use spargebra::SparqlParser;

    let parse = |q: &str| SparqlParser::new().parse_query(q).expect("parse");

    let iri_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) }";
    let rewritten = sparq_engine::rewrite::rewrite_query(parse(iri_eq));
    assert_ne!(
        format!("{:?}", rewritten),
        format!("{:?}", parse(iri_eq)),
        "FILTER(?v = <iri>) must be constant-substituted by the algebra-rewrite pass"
    );

    let lit_eq = "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = 42) }";
    let verbatim = sparq_engine::rewrite::rewrite_query(parse(lit_eq));
    assert_eq!(
        format!("{:?}", verbatim),
        format!("{:?}", parse(lit_eq)),
        "literal equality must be left verbatim (the sq-lr2ii avoidance contract)"
    );
}

/// RESULT PARITY on the real tool path: the rewritten `FILTER(?v = <iri>)` plan is what an
/// agent's `query` tool call now executes, and it must still answer correctly — the rewrite
/// is an optimisation, never a semantics change.
#[cfg(feature = "algebra-rewrite")]
#[test]
fn iri_equality_filter_answers_correctly_through_the_mcp_query_tool() {
    const FIXTURE: &str = "<http://ex/a> <http://ex/p> <http://ex/target> .\n\
                           <http://ex/b> <http://ex/p> <http://ex/other> .\n\
                           <http://ex/c> <http://ex/p> <http://ex/target> .\n";
    let reply = tool_call_query(
        FIXTURE,
        "SELECT ?s WHERE { ?s <http://ex/p> ?v . FILTER(?v = <http://ex/target>) } ORDER BY ?s",
    );
    assert!(reply.contains("http://ex/a"), "row a expected: {}", reply);
    assert!(reply.contains("http://ex/c"), "row c expected: {}", reply);
    assert!(
        !reply.contains("http://ex/b"),
        "row b must be filtered out: {}",
        reply
    );
}

// ---------------------------------------------------------------------------
// Layer 2b — `dp-planner`
// ---------------------------------------------------------------------------

/// A 3-pattern connected BGP — the smallest shape `dp::plan()` accepts (for n <= 2 greedy
/// GOO is already Cout-optimal, see sq-7d3dj.30.5) — over a two-chain fixture.
#[cfg(feature = "dp-planner")]
const JOIN_FIXTURE: &str = "<http://ex/alice> <http://ex/knows>   <http://ex/bob> .\n\
                            <http://ex/carol> <http://ex/knows>   <http://ex/dave> .\n\
                            <http://ex/bob>   <http://ex/worksAt> <http://ex/org1> .\n\
                            <http://ex/dave>  <http://ex/worksAt> <http://ex/org2> .\n\
                            <http://ex/org1>  <http://ex/name>    \"Acme\" .\n\
                            <http://ex/org2>  <http://ex/name>    \"Umbrella\" .\n";

#[cfg(feature = "dp-planner")]
const JOIN_QUERY: &str = "SELECT ?s ?name WHERE { \
                          ?s <http://ex/knows> ?o . \
                          ?o <http://ex/worksAt> ?org . \
                          ?org <http://ex/name> ?name } ORDER BY ?name";

/// COMPILE-TIME witness: `sparq_engine::without_dp_planner` (and the `dp` module it lives
/// in) only exists when the engine's `dp-planner` feature is on.
///
/// BEHAVIORAL half: once compiled the planner is DEFAULT-ON in the engine (`Install::
/// Default`), so the MCP `query` tool is DP-planned with no `with_dp_planner` call anywhere
/// on the dispatch path — the exact situation a `tools/call` is in. Running the same query
/// inside `without_dp_planner` yields the greedy-GOO plan, and the two MUST agree: the
/// planner reorders joins, it never changes the answer.
#[cfg(feature = "dp-planner")]
#[test]
fn dp_planner_forwarded_and_default_on_across_the_mcp_tool_path() {
    // No `with_dp_planner` wrapper: this is the tool path exactly as an agent drives it.
    let dp_planned = tool_call_query(JOIN_FIXTURE, JOIN_QUERY);
    let greedy = sparq_engine::without_dp_planner(|| tool_call_query(JOIN_FIXTURE, JOIN_QUERY));

    assert_eq!(
        dp_planned, greedy,
        "the DPccp planner only reorders joins — it must never change a tool's answer"
    );
    assert!(
        dp_planned.contains("Acme"),
        "org name Acme expected: {}",
        dp_planned
    );
    assert!(
        dp_planned.contains("Umbrella"),
        "org name Umbrella expected: {}",
        dp_planned
    );
}

/// The scoped opt-out is not sticky: after a `without_dp_planner` scope ends the next tool
/// call is back on the compiled-in default. Without this, one opt-out call site could leave
/// a long-lived MCP server greedy-planning for the rest of the process's life — the exact
/// silent-drift failure mode this bead is about.
#[cfg(feature = "dp-planner")]
#[test]
fn dp_planner_opt_out_scope_does_not_leak_into_later_tool_calls() {
    let before = tool_call_query(JOIN_FIXTURE, JOIN_QUERY);
    let _ = sparq_engine::without_dp_planner(|| tool_call_query(JOIN_FIXTURE, JOIN_QUERY));
    let after = tool_call_query(JOIN_FIXTURE, JOIN_QUERY);

    assert_eq!(
        before, after,
        "the opt-out scope must restore the previous install state"
    );
    assert!(after.contains("Acme"), "org name Acme expected: {}", after);
}

/// Run `sparql` against an N-Triples `fixture` through the REAL MCP `tools/call` dispatch
/// (the path an agent drives) and return the response text.
#[cfg(any(feature = "algebra-rewrite", feature = "dp-planner"))]
fn tool_call_query(fixture: &str, sparql: &str) -> String {
    let graph = sparq_core::Graph::load_str(fixture, "ntriples").expect("load fixture");
    let mut server = sparq_mcp::McpServer::new(graph);
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "query", "arguments": { "sparql": sparql } },
    })
    .to_string();
    let reply = server
        .handle_message(&request)
        .expect("tools/call must produce a response");
    assert!(
        !reply.contains("\"isError\":true"),
        "the query tool must succeed: {}",
        reply
    );
    reply
}
