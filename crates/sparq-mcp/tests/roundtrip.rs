//! [OPUS-4.8] (sq-0z43i, gh #909) Real MCP round-trip over an in-memory store:
//! initialize → tools/list → tools/call. Exercises the actual dispatch core
//! (`McpServer::handle_message`) — no mock bypasses the engine; `query` runs through
//! the real `sparq-engine` query path and `introspect`/`stats`/`classes`/`prefixes`
//! through the real `sparq-introspect` schema miner. The load-bearing invariants asserted here:
//!   - read-only by DEFAULT: `update` is absent from `tools/list` and a `tools/call`
//!     for it is refused (fail-closed mutation surface);
//!   - with update enabled, `update` is advertised AND actually mutates the graph,
//!     and a subsequent `query` observes the change (the write path is real);
//!   - a bad SPARQL string surfaces as an MCP tool error (`isError`), not a panic.

use serde_json::Value;
use sparq_core::Graph;
use sparq_mcp::{McpServer, ServerConfig};

const TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
ex:alice rdf:type ex:Person ; ex:name "Alice" ; ex:knows ex:bob .
ex:bob   rdf:type ex:Person ; ex:name "Bob" .
"#;

// [GPT-5.6] sq-kx5b0: three distinct schema.org IRIs and one FOAF IRI give the
// `prefixes` tool a mutation-witnessed ordering/count fixture.
const PREFIXES_TTL: &str = r#"@prefix schema: <https://schema.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
schema:alice schema:knows schema:bob ; foaf:name "Alice" .
"#;

// [GPT-5.6] sq-cekgj: two classes with unequal populations and predicate profiles give
// the `classes` tool mutation-witnessed ordering and count assertions.
const CLASSES_TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
ex:a1 rdf:type ex:A ; ex:p "one" ; ex:q ex:o .
ex:a2 rdf:type ex:A ; ex:p "two" .
ex:a3 rdf:type ex:A .
ex:b1 rdf:type ex:B ; ex:p "three" .
"#;

fn graph() -> Graph {
    Graph::load_str(TTL, "turtle").expect("load turtle")
}

fn prefixes_graph() -> Graph {
    Graph::load_str(PREFIXES_TTL, "turtle").expect("load prefixes turtle")
}

fn classes_graph() -> Graph {
    Graph::load_str(CLASSES_TTL, "turtle").expect("load classes turtle")
}

/// Parse a server response line into JSON.
fn parse(resp: &str) -> Value {
    serde_json::from_str(resp).expect("response is valid JSON")
}

/// Convenience: send a request, assert it produced a (non-notification) response.
fn call(server: &mut McpServer, raw: &str) -> Value {
    parse(&server.handle_message(raw).expect("request gets a response"))
}

#[test]
fn initialize_handshake_reports_server_info() {
    let mut server = McpServer::new(graph());
    let resp = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    );
    assert_eq!(resp["id"], 1);
    let result = &resp["result"];
    assert!(result["protocolVersion"].is_string());
    assert_eq!(result["serverInfo"]["name"], "sparq-mcp");
    assert!(result["capabilities"]["tools"].is_object());
}

// [SONNET-4.6] sq-bvnqm: MCP version negotiation — a supported client-proposed
// revision is accepted verbatim; anything else is answered with the server's
// latest supported revision (per the MCP versioning rules), never a fixed echo.
#[test]
fn initialize_accepts_a_supported_proposed_protocol_version() {
    let mut server = McpServer::new(graph());
    let resp = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
}

#[test]
fn initialize_answers_an_unsupported_proposed_version_with_the_latest() {
    let mut server = McpServer::new(graph());
    let resp = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
    );
    assert_eq!(resp["result"]["protocolVersion"], sparq_mcp::PROTOCOL_VERSION);
}

#[test]
fn initialize_defaults_to_the_latest_when_no_version_is_proposed() {
    let mut server = McpServer::new(graph());
    let resp = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    );
    assert_eq!(resp["result"]["protocolVersion"], sparq_mcp::PROTOCOL_VERSION);
}

#[test]
fn latest_protocol_version_heads_the_supported_list() {
    assert_eq!(
        sparq_mcp::SUPPORTED_PROTOCOL_VERSIONS.first().copied(),
        Some(sparq_mcp::PROTOCOL_VERSION),
    );
    assert_eq!(sparq_mcp::PROTOCOL_VERSION, "2025-11-25");
}

#[test]
fn notifications_get_no_response() {
    let mut server = McpServer::new(graph());
    let out = server.handle_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert!(out.is_none(), "a notification must not produce a response");
}

#[test]
fn read_only_server_lists_read_tools_only() {
    let mut server = McpServer::new(graph());
    let resp = call(&mut server, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"query"));
    assert!(names.contains(&"construct"));
    assert!(names.contains(&"introspect"));
    assert!(names.contains(&"shapes"));
    assert!(names.contains(&"stats"));
    assert!(names.contains(&"classes"));
    assert!(names.contains(&"prefixes"));
    assert!(names.contains(&"void"));
    // [GPT-5.6] sq-lsp7k.22: the validator dependency and tool stay opt-in.
    #[cfg(not(feature = "shacl"))]
    assert!(!names.contains(&"validate"));
    // [FABLE-5] sq-lsp7k.1.6: form derivation shares the same opt-in gate — absent
    // from the default (feature-off) tool surface.
    #[cfg(not(feature = "shacl"))]
    assert!(!names.contains(&"describe_form"));
    assert!(!names.contains(&"update"), "update must NOT be advertised by default");
    // The NL tools are feature-gated AND backend-gated: never advertised in the
    // default build (the `nlq` feature is off here).
    assert!(!names.contains(&"ask"), "ask must NOT be advertised in the default build");
    assert!(
        !names.contains(&"nl_query"),
        "nl_query must NOT be advertised in the default build"
    );
    // Each tool ships a proper inputSchema object.
    for tool in resp["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[test]
fn shapes_tool_returns_data_grounded_constraints_for_a_class() {
    // [OPUS-4.8] sq-zak4f: the structured shape tool, exercised through the REAL MCP
    // dispatch (no mock bypasses the introspection miner).
    let mut server = McpServer::new(graph());
    let req = r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{
        "name":"shapes",
        "arguments":{"class":"http://ex/Person"}
    }}"#;
    let resp = call(&mut server, req);
    let result = &resp["result"];
    assert_eq!(result["isError"], false, "{resp}");
    let shape: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(shape["class"], "http://ex/Person");
    assert_eq!(shape["instances"], 2, "alice + bob");

    let preds: Vec<&str> = shape["predicates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["predicate"].as_str().unwrap())
        .collect();
    // name (on both) and knows (on alice only) are reported; rdf:type is NOT (carried by
    // the class targeting itself).
    assert!(preds.contains(&"http://ex/name"), "{preds:?}");
    assert!(preds.contains(&"http://ex/knows"), "{preds:?}");
    assert!(
        !preds.contains(&"http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        "rdf:type must not appear as a property: {preds:?}"
    );

    // name is on both instances exactly once ⇒ min_count 1, max_count 1.
    let name = shape["predicates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["predicate"] == "http://ex/name")
        .unwrap();
    assert_eq!(name["min_count"], 1);
    assert_eq!(name["max_count"], 1);
    assert_eq!(name["coverage"], 1.0);
}

#[test]
fn shapes_tool_unknown_class_is_a_tool_error_not_a_protocol_error() {
    let mut server = McpServer::new(graph());
    let req = r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{
        "name":"shapes",
        "arguments":{"class":"http://ex/Nope"}
    }}"#;
    let resp = call(&mut server, req);
    // A missing class is a TOOL error (isError true, the client reads + corrects), not a
    // JSON-RPC protocol error.
    assert!(resp["result"]["isError"].as_bool().unwrap(), "{resp}");
    assert!(resp["error"].is_null());
    let msg = resp["result"]["content"][0]["text"].as_str().unwrap();
    // It names the classes that DO exist so the client can self-correct.
    assert!(msg.contains("http://ex/Person"), "{msg}");
}

#[test]
fn query_tool_runs_real_select() {
    let mut server = McpServer::new(graph());
    let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"query",
        "arguments":{"sparql":"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n } ORDER BY ?n"}
    }}"#;
    let resp = call(&mut server, req);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    // The text payload is SPARQL 1.1 JSON results from the REAL engine.
    let results: Value = serde_json::from_str(text).unwrap();
    let bindings = results["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 2, "Alice + Bob");
    assert_eq!(bindings[0]["n"]["value"], "Alice");
    assert_eq!(bindings[1]["n"]["value"], "Bob");
}

#[test]
fn introspect_and_stats_reflect_the_graph() {
    let mut server = McpServer::new(graph());
    let stats = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#,
    );
    let text = stats["result"]["content"][0]["text"].as_str().unwrap();
    let s: Value = serde_json::from_str(text).unwrap();
    // alice: type+name+knows (3), bob: type+name (2) = 5 triples.
    assert_eq!(s["triples"], 5);
    assert_eq!(s["typed_entities"], 2);

    // introspect text summary is grounded, non-empty schema text.
    let ix = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"introspect","arguments":{"format":"text","budget_chars":2000}}}"#,
    );
    let summary = ix["result"]["content"][0]["text"].as_str().unwrap();
    assert!(summary.contains("Schema summary"), "got: {}", summary);
}

#[test]
fn prefixes_tool_lists_namespaces_by_descending_term_count() {
    let mut server = McpServer::new(prefixes_graph());
    let response = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"prefixes","arguments":{}}}"#,
    );
    let result = &response["result"];
    assert_eq!(result["isError"], false, "{response}");
    let payload: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(payload["distinct_prefixes"], 2);
    let prefixes = payload["prefixes"].as_array().unwrap();
    assert_eq!(prefixes.len(), 2);
    assert_eq!(prefixes[0]["prefix"], "schema");
    assert_eq!(prefixes[0]["namespace"], "https://schema.org/");
    assert_eq!(prefixes[0]["term_count"], 3);
    assert_eq!(prefixes[1]["prefix"], "foaf");
    assert_eq!(prefixes[1]["namespace"], "http://xmlns.com/foaf/0.1/");
    assert_eq!(prefixes[1]["term_count"], 1);
}

#[test]
fn classes_tool_lists_profiles_by_descending_instance_count() {
    let mut server = McpServer::new(classes_graph());
    let response = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"classes","arguments":{}}}"#,
    );
    let result = &response["result"];
    assert_eq!(result["isError"], false, "{response}");
    let payload: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(payload["distinct_classes"], 2);
    let classes = payload["classes"].as_array().unwrap();
    assert_eq!(classes.len(), 2);
    assert_eq!(classes[0]["class"], "http://ex/A");
    assert_eq!(classes[0]["instances"], 3);
    // ClassProfile predicates include rdf:type alongside the two descriptive predicates.
    assert_eq!(classes[0]["predicate_count"], 3);
    assert_eq!(classes[1]["class"], "http://ex/B");
    assert_eq!(classes[1]["instances"], 1);
    assert_eq!(classes[1]["predicate_count"], 2);
}

#[test]
fn void_tool_emits_dataset_descriptor() {
    let mut server = McpServer::new(graph());
    let descriptor = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"void","arguments":{"dataset":"http://example.org/ds"}}}"#,
    );
    let result = &descriptor["result"];
    assert_eq!(result["isError"], false, "{descriptor}");
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains(
            "<http://example.org/ds> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://rdfs.org/ns/void#Dataset> ."
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "<http://example.org/ds> <http://rdfs.org/ns/void#triples> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
        ),
        "{text}"
    );
    assert!(
        text.contains("<http://rdfs.org/ns/void#class> <http://ex/Person>"),
        "{text}"
    );
}

#[test]
fn void_tool_defaults_dataset_iri_and_optionally_emits_characteristic_sets() {
    let mut server = McpServer::new(graph());
    let default_descriptor = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"void","arguments":{}}}"#,
    );
    let default_text = default_descriptor["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        default_text.contains(
            "<urn:sparq:dataset> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://rdfs.org/ns/void#Dataset> ."
        ),
        "{default_text}"
    );
    assert!(
        !default_text.contains("http://sparq.dev/ns/cs#characteristicSet"),
        "bare VoID must omit characteristic-set statistics: {default_text}"
    );

    let with_cs = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"void","arguments":{"characteristic_sets":true}}}"#,
    );
    let with_cs_text = with_cs["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        with_cs_text.contains("http://sparq.dev/ns/cs#characteristicSet"),
        "{with_cs_text}"
    );
}

#[test]
fn bad_sparql_is_a_tool_error_not_a_panic() {
    let mut server = McpServer::new(graph());
    let resp = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"query","arguments":{"sparql":"NOT SPARQL"}}}"#,
    );
    // A bad query is a TOOL error (isError true), not a JSON-RPC protocol error.
    assert!(resp["result"]["isError"].as_bool().unwrap());
    assert!(resp["error"].is_null());
}

#[test]
fn update_refused_when_read_only() {
    let mut server = McpServer::new(graph());
    let resp = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"update","arguments":{"sparql":"INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/y> }"}}}"#,
    );
    // Fail-closed: a disabled mutation surface is a protocol METHOD_NOT_FOUND-class error.
    assert!(resp["error"].is_object(), "update must be refused: {}", resp);
    assert_eq!(resp["error"]["code"], -32601);
    // And the graph is UNCHANGED — proven by the count below.
    let stats = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#,
    );
    let s: Value = serde_json::from_str(stats["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(s["triples"], 5, "read-only server must not have mutated the graph");
}

#[test]
fn update_applies_when_enabled_and_query_observes_it() {
    let config = ServerConfig { allow_update: true, ..ServerConfig::default() };
    let mut server = McpServer::with_config(graph(), config);

    // It is now advertised.
    let list = call(&mut server, r#"{"jsonrpc":"2.0","id":9,"method":"tools/list"}"#);
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"update"));

    // Apply a real INSERT.
    let upd = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"update","arguments":{"sparql":"INSERT DATA { <http://ex/carol> <http://ex/name> \"Carol\" }"}}}"#,
    );
    assert_eq!(upd["result"]["isError"], false, "{}", upd);

    // A subsequent query observes the mutation — the write path is REAL.
    let q = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"query","arguments":{"sparql":"SELECT ?n WHERE { <http://ex/carol> <http://ex/name> ?n }"}}}"#,
    );
    let text = q["result"]["content"][0]["text"].as_str().unwrap();
    let results: Value = serde_json::from_str(text).unwrap();
    assert_eq!(results["results"]["bindings"][0]["n"]["value"], "Carol");
}

#[test]
fn unknown_method_is_method_not_found() {
    let mut server = McpServer::new(graph());
    let resp = call(&mut server, r#"{"jsonrpc":"2.0","id":12,"method":"nope/nope"}"#);
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn malformed_json_is_invalid_request() {
    let mut server = McpServer::new(graph());
    let resp = parse(&server.handle_message("{ not json").unwrap());
    assert_eq!(resp["error"]["code"], -32600);
    assert!(resp["id"].is_null());
}
