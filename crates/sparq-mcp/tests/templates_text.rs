//! [FABLE-5] sq-lsp7k.10: MCP integration tests for the opt-in `templates`
//! (`template_list` / `template_invoke`) and `text` (`text_search`) tool surfaces —
//! the bead's acceptance test: a real MCP round-trip invoking a named parameterized
//! template AND a full-text search, through the actual dispatch core
//! (`McpServer::handle_message`, no mocks). Load-bearing invariants:
//!   - the template tools are advertised only when templates are registered;
//!   - `template_invoke` binds TYPED arguments and returns the same rows as the
//!     hand-written constant query (result equivalence);
//!   - fail-closed invocation: unknown template / unknown / missing / mistyped
//!     parameters are tool errors, never guesses;
//!   - the GATED-UPDATE POSTURE: an UPDATE template is refused on a read-only server
//!     (the `allow_update` gate), applies when update is enabled, and a hostile bound
//!     literal stays DATA (the #901 injection posture through the MCP surface);
//!   - `text_search` BM25-ranks literal matches, supports and/any + prefix tokens,
//!     and observes literals added by a later update (the reconcile path).

#![cfg(all(feature = "templates", feature = "text"))]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_engine::templates::{ParamType, Template};
use sparq_mcp::{McpServer, ServerConfig};

const TTL: &str = r#"@prefix ex: <http://ex/> .
ex:alice ex:name "Alice" ; ex:bio "the quick brown fox jumps" ; ex:knows ex:bob .
ex:bob   ex:name "Bob" ; ex:bio "a lazy dog sleeps" .
"#;

fn graph() -> Graph {
    Graph::load_str(TTL, "turtle").expect("load turtle")
}

fn friends_template() -> Template {
    Template::new(
        "http://ex/tpl/friends",
        "SELECT ?f WHERE { ?who <http://ex/knows> ?f }",
        [("who".to_string(), ParamType::Iri)].into_iter().collect(),
        Some("who does ?who know".to_string()),
    )
    .unwrap()
}

fn note_update_template() -> Template {
    Template::new(
        "http://ex/tpl/add-note",
        "INSERT { <http://ex/m> <http://ex/note> ?note } WHERE { }",
        [("note".to_string(), ParamType::String)]
            .into_iter()
            .collect(),
        None,
    )
    .unwrap()
}

fn server_with(templates: Vec<Template>, allow_update: bool) -> McpServer {
    let config = ServerConfig {
        allow_update,
        templates,
        ..ServerConfig::default()
    };
    McpServer::with_config(graph(), config)
}

fn call(server: &mut McpServer, raw: &str) -> Value {
    serde_json::from_str(&server.handle_message(raw).expect("response")).expect("valid JSON")
}

fn tool_call(server: &mut McpServer, name: &str, args: Value) -> Value {
    let req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args }
    });
    call(server, &req.to_string())
}

/// The text payload of a CallToolResult + its isError flag.
fn tool_text(resp: &Value) -> (String, bool) {
    let result = &resp["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        result["isError"].as_bool().unwrap_or(false),
    )
}

fn advertised_names(server: &mut McpServer) -> Vec<String> {
    let resp = call(server, r#"{"jsonrpc":"2.0","id":9,"method":"tools/list"}"#);
    resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Advertisement.
// ---------------------------------------------------------------------------

#[test]
fn template_tools_advertised_only_when_registered() {
    let mut bare = server_with(Vec::new(), false);
    let names = advertised_names(&mut bare);
    assert!(!names.contains(&"template_list".to_string()));
    assert!(!names.contains(&"template_invoke".to_string()));
    // text_search is self-contained: advertised by the feature alone.
    assert!(names.contains(&"text_search".to_string()));

    let mut with = server_with(vec![friends_template()], false);
    let names = advertised_names(&mut with);
    assert!(names.contains(&"template_list".to_string()));
    assert!(names.contains(&"template_invoke".to_string()));
}

// ---------------------------------------------------------------------------
// template_list + template_invoke (query).
// ---------------------------------------------------------------------------

#[test]
fn template_list_grounds_invocation() {
    let mut server = server_with(vec![friends_template(), note_update_template()], false);
    let (text, is_err) = tool_text(&tool_call(&mut server, "template_list", json!({})));
    assert!(!is_err);
    let defs: Value = serde_json::from_str(&text).unwrap();
    let arr = defs.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let friends = arr
        .iter()
        .find(|d| d["name"] == "http://ex/tpl/friends")
        .unwrap();
    assert_eq!(friends["kind"], "query");
    assert_eq!(friends["parameters"], json!({"who": "iri"}));
}

#[test]
fn template_invoke_binds_typed_params_result_equivalent() {
    let mut server = server_with(vec![friends_template()], false);
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "template_invoke",
        json!({"name": "http://ex/tpl/friends", "parameters": {"who": "http://ex/alice"}}),
    ));
    assert!(!is_err, "{text}");
    let results: Value = serde_json::from_str(&text).unwrap();
    let bindings = results["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["f"]["value"], "http://ex/bob");
    // Result equivalence with the raw query tool's hand-written constant query.
    let (direct, _) = tool_text(&tool_call(
        &mut server,
        "query",
        json!({"sparql": "SELECT ?f WHERE { <http://ex/alice> <http://ex/knows> ?f }"}),
    ));
    let direct: Value = serde_json::from_str(&direct).unwrap();
    assert_eq!(
        results["results"]["bindings"],
        direct["results"]["bindings"]
    );
}

#[test]
fn template_invoke_fail_closed_arguments() {
    let mut server = server_with(vec![friends_template()], false);
    // Unknown template.
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "template_invoke",
        json!({"name": "nope", "parameters": {}}),
    ));
    assert!(is_err && text.contains("no such template"), "{text}");
    // Unknown parameter.
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "template_invoke",
        json!({"name": "http://ex/tpl/friends",
               "parameters": {"who": "http://ex/alice", "typo": 1}}),
    ));
    assert!(is_err && text.contains("unknown parameter"), "{text}");
    // Missing parameter.
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "template_invoke",
        json!({"name": "http://ex/tpl/friends"}),
    ));
    assert!(
        is_err && text.contains("missing required parameter"),
        "{text}"
    );
    // Mistyped parameter (a number is not an IRI string — no coercion).
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "template_invoke",
        json!({"name": "http://ex/tpl/friends", "parameters": {"who": 5}}),
    ));
    assert!(is_err && text.contains("expects a JSON string"), "{text}");
}

// ---------------------------------------------------------------------------
// The gated-update posture through the template layer.
// ---------------------------------------------------------------------------

#[test]
fn update_template_respects_allow_update_gate() {
    // Read-only server: the UPDATE template is listed but its invocation is refused.
    let mut ro = server_with(vec![note_update_template()], false);
    let (text, is_err) = tool_text(&tool_call(
        &mut ro,
        "template_invoke",
        json!({"name": "http://ex/tpl/add-note", "parameters": {"note": "x"}}),
    ));
    assert!(is_err && text.contains("read-only"), "{text}");
    // Nothing mutated.
    let (ask, _) = tool_text(&tool_call(
        &mut ro,
        "query",
        json!({"sparql": "ASK { <http://ex/m> ?p ?o }"}),
    ));
    assert!(ask.contains("false"), "{ask}");

    // Update-enabled server: the same invocation applies atomically, and a hostile
    // bound literal stays DATA (no injected DROP ALL).
    let mut rw = server_with(vec![note_update_template()], true);
    let hostile =
        r#"x" } ; DROP ALL ; INSERT DATA { <http://ex/evil> <http://ex/p> <http://ex/o> } # "#;
    let (text, is_err) = tool_text(&tool_call(
        &mut rw,
        "template_invoke",
        json!({"name": "http://ex/tpl/add-note", "parameters": {"note": hostile}}),
    ));
    assert!(!is_err, "{text}");
    let (ask, _) = tool_text(&tool_call(
        &mut rw,
        "query",
        json!({"sparql": "ASK { <http://ex/m> <http://ex/note> ?n }"}),
    ));
    assert!(ask.contains("true"), "{ask}");
    // alice survived (DROP ALL did NOT run) and no evil triple appeared.
    let (alice, _) = tool_text(&tool_call(
        &mut rw,
        "query",
        json!({"sparql": "ASK { <http://ex/alice> <http://ex/name> ?n }"}),
    ));
    assert!(alice.contains("true"), "{alice}");
    let (evil, _) = tool_text(&tool_call(
        &mut rw,
        "query",
        json!({"sparql": "ASK { <http://ex/evil> ?p ?o }"}),
    ));
    assert!(evil.contains("false"), "{evil}");
}

// ---------------------------------------------------------------------------
// text_search.
// ---------------------------------------------------------------------------

#[test]
fn text_search_ranks_and_filters() {
    let mut server = server_with(Vec::new(), false);
    // AND semantics: both tokens must appear (only alice's bio has quick+fox).
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "quick fox"}),
    ));
    assert!(!is_err, "{text}");
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["total_matches"], 1);
    assert!(out["hits"][0]["literal"]
        .as_str()
        .unwrap()
        .contains("quick brown fox"));
    assert!(out["hits"][0]["score"].as_f64().unwrap() > 0.0);
    // ANY semantics: fox OR dog matches both bios.
    let (text, _) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "fox dog", "mode": "any"}),
    ));
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["total_matches"], 2);
    // Prefix token (autocomplete-style discovery).
    let (text, _) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "qui*"}),
    ));
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["total_matches"], 1);
    // limit caps returned hits but reports the honest total.
    let (text, _) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "fox dog", "mode": "any", "limit": 1}),
    ));
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["total_matches"], 2);
    assert_eq!(out["returned"], 1);
    // Unknown mode is a fail-closed tool error.
    let (text, is_err) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "fox", "mode": "phrase"}),
    ));
    assert!(is_err && text.contains("unknown mode"), "{text}");
    // Missing query is a tool error.
    let (_, is_err) = tool_text(&tool_call(&mut server, "text_search", json!({})));
    assert!(is_err);
}

#[test]
fn text_search_observes_updates_via_reconcile() {
    let mut server = server_with(Vec::new(), true);
    // Build the index (first search), then mutate, then search again: the new literal
    // must be found (the O(new terms) reconcile path, not a stale index).
    let (text, _) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "zebra"}),
    ));
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["total_matches"], 0);
    let (_, is_err) = tool_text(&tool_call(
        &mut server,
        "update",
        json!({"sparql": "INSERT DATA { <http://ex/z> <http://ex/bio> \"a striped zebra grazes\" }"}),
    ));
    assert!(!is_err);
    let (text, _) = tool_text(&tool_call(
        &mut server,
        "text_search",
        json!({"query": "zebra"}),
    ));
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["total_matches"], 1);
    assert!(out["hits"][0]["literal"]
        .as_str()
        .unwrap()
        .contains("zebra"));
}
