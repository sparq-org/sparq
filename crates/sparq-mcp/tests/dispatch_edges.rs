//! [GPT-5.6] (sq-bif.35) JSON-RPC dispatch edge contracts for the always-compiled
//! MCP server core.

use serde_json::Value;
use sparq_core::Graph;
use sparq_mcp::{McpServer, ServerConfig};

const TTL: &str = r#"@prefix ex: <http://ex/> .
ex:alice ex:name "Alice" .
"#;

fn graph() -> Graph {
    Graph::load_str(TTL, "turtle").expect("load turtle")
}

fn parse(response: &str) -> Value {
    serde_json::from_str(response).expect("response is valid JSON")
}

fn call(server: &mut McpServer, raw: &str) -> Value {
    parse(&server.handle_message(raw).expect("request gets a response"))
}

#[test]
fn unknown_tool_name_returns_exact_method_not_found_error() {
    let mut server = McpServer::new(graph());
    let response = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nonesuch","arguments":{}}}"#,
    );

    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], "unknown tool: nonesuch");
}

#[test]
fn ping_returns_empty_object_and_echoes_request_id() {
    let mut server = McpServer::new(graph());
    let response = call(&mut server, r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);

    assert_eq!(response["id"], 7);
    assert_eq!(response["result"], serde_json::json!({}));
}

#[test]
fn unknown_method_notification_gets_no_response() {
    let mut server = McpServer::new(graph());
    let response = server.handle_message(r#"{"jsonrpc":"2.0","method":"nonesuch"}"#);

    assert!(
        response.is_none(),
        "an erroneous notification must remain silent"
    );
}

#[test]
fn malformed_json_error_has_null_id() {
    let mut server = McpServer::new(graph());
    let response = parse(
        &server
            .handle_message("{ not json")
            .expect("parse error gets a response"),
    );

    assert!(
        response["id"].is_null(),
        "an uncorrelatable parse error must use a null id"
    );
}

// [OPUS-5] gh #2497: JSON-RPC 2.0 §6 batch receipt — the one thing the MCP 2025-03-26
// revision requires that the others do not, and therefore the precondition for
// 2025-03-26 appearing in SUPPORTED_PROTOCOL_VERSIONS.

#[test]
fn batch_returns_one_response_per_request_in_order() {
    let mut server = McpServer::new(graph());
    let response = call(
        &mut server,
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},
            {"jsonrpc":"2.0","id":"two","method":"tools/list"}]"#,
    );

    let batch = response.as_array().expect("a batch is answered with an array");
    assert_eq!(batch.len(), 2, "one response per non-notification element");
    assert_eq!(batch[0]["id"], 1);
    assert_eq!(batch[0]["result"], serde_json::json!({}));
    assert_eq!(batch[1]["id"], "two");
    assert!(batch[1]["result"]["tools"].is_array());
}

#[test]
fn batch_omits_notification_entries() {
    let mut server = McpServer::new(graph());
    let response = call(
        &mut server,
        r#"[{"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":4,"method":"ping"},
            {"jsonrpc":"2.0","method":"nonesuch"}]"#,
    );

    let batch = response.as_array().expect("a batch is answered with an array");
    assert_eq!(
        batch.len(),
        1,
        "notifications get no entry, not even an erroneous one"
    );
    assert_eq!(batch[0]["id"], 4);
}

#[test]
fn all_notification_batch_gets_no_response() {
    let mut server = McpServer::new(graph());
    let response = server.handle_message(
        r#"[{"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","method":"ping"}]"#,
    );

    assert!(
        response.is_none(),
        "a batch of nothing but notifications must stay silent, like a single notification"
    );
}

#[test]
fn empty_batch_is_a_single_invalid_request_error() {
    let mut server = McpServer::new(graph());
    let response = call(&mut server, "[]");

    assert!(
        !response.is_array(),
        "the empty-batch error is a single object, not an array"
    );
    assert!(response["id"].is_null());
    assert_eq!(response["error"]["code"], -32600);
    assert_eq!(
        response["error"]["message"], "invalid JSON-RPC request: empty batch",
        "the empty array is rejected as a batch, not as an accidental parse failure"
    );
}

#[test]
fn malformed_batch_element_does_not_void_the_rest_of_the_batch() {
    let mut server = McpServer::new(graph());
    let response = call(
        &mut server,
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},
            "not a request object",
            {"jsonrpc":"2.0","id":3,"method":"ping"}]"#,
    );

    let batch = response.as_array().expect("a batch is answered with an array");
    assert_eq!(batch.len(), 3);
    assert_eq!(batch[0]["id"], 1);
    assert!(batch[0]["error"].is_null(), "the valid elements still run");
    assert!(
        batch[1]["id"].is_null(),
        "an uncorrelatable element error uses a null id"
    );
    assert_eq!(batch[1]["error"]["code"], -32600);
    assert_eq!(batch[2]["id"], 3);
    assert!(batch[2]["error"].is_null());
}

#[test]
fn batch_dispatch_sees_state_written_by_an_earlier_element() {
    // Elements are dispatched in order against the SAME server, so a batch is not a
    // set of independent calls: element 2 must observe element 1's write.
    let config = ServerConfig { allow_update: true, ..ServerConfig::default() };
    let mut server = McpServer::with_config(graph(), config);
    let response = call(
        &mut server,
        r#"[{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update","arguments":{"sparql":"INSERT DATA { <http://ex/bob> <http://ex/name> \"Bob\" }"}}},
            {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"query","arguments":{"sparql":"SELECT ?n WHERE { <http://ex/bob> <http://ex/name> ?n }"}}}]"#,
    );

    let batch = response.as_array().expect("a batch is answered with an array");
    assert_eq!(batch.len(), 2);
    let rows = batch[1]["result"]["content"][0]["text"]
        .as_str()
        .expect("query tool returns text content");
    assert!(
        rows.contains("Bob"),
        "the query element must see the insert from the earlier element, got: {}",
        rows
    );
}

#[test]
fn the_batch_only_revision_is_supported_and_negotiated_verbatim() {
    assert!(
        sparq_mcp::SUPPORTED_PROTOCOL_VERSIONS.contains(&"2025-03-26"),
        "batch receipt is implemented, so the batch-requiring revision is claimed"
    );

    let mut server = McpServer::new(graph());
    let response = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
    );
    assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
}

#[test]
fn initialized_request_returns_null_result() {
    let mut server = McpServer::new(graph());
    let response = call(
        &mut server,
        r#"{"jsonrpc":"2.0","id":9,"method":"initialized"}"#,
    );

    assert!(response["result"].is_null());
}
