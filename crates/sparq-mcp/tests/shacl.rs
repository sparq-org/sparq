//! sq-lsp7k.22: JSON-RPC round trips for the opt-in SHACL validation tool.

#![cfg(feature = "shacl")]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_mcp::McpServer;

const DATA_MISSING_NAME: &str = r#"@prefix ex: <http://example.com/> .
ex:alice a ex:Person .
"#;

const DATA_WITH_NAME: &str = r#"@prefix ex: <http://example.com/> .
ex:alice a ex:Person ; ex:name "Alice" .
"#;

const PERSON_SHAPE: &str = r#"@prefix ex: <http://example.com/> .
@prefix sh: <http://www.w3.org/ns/shacl#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:minCount 1 ;
        sh:message "A person requires a name" ;
    ] .
"#;

fn server(data: &str) -> McpServer {
    McpServer::new(Graph::load_str(data, "turtle").expect("data fixture parses"))
}

fn message(server: &mut McpServer, request: Value) -> Value {
    serde_json::from_str(
        &server
            .handle_message(&request.to_string())
            .expect("request produces a response"),
    )
    .expect("response is JSON")
}

fn validate(server: &mut McpServer, shapes: Value) -> Value {
    message(
        server,
        json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": { "name": "validate", "arguments": { "shapes": shapes } }
        }),
    )
}

fn tool_payload(response: &Value) -> (Value, bool) {
    let result = &response["result"];
    let text = result["content"][0]["text"]
        .as_str()
        .expect("tool result has text");
    (
        serde_json::from_str(text).expect("successful validate result is JSON"),
        result["isError"].as_bool().unwrap_or(false),
    )
}

#[test]
fn validate_is_advertised_with_feature() {
    let response = message(
        &mut server(DATA_MISSING_NAME),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    let validate = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == "validate")
        .expect("validate is advertised when the feature is on");
    assert_eq!(validate["inputSchema"]["required"], json!(["shapes"]));
}

#[test]
fn missing_required_property_returns_violation_without_mutating_data() {
    let mut server = server(DATA_MISSING_NAME);
    let before = server.graph().len();
    let (payload, is_error) = tool_payload(&validate(&mut server, json!(PERSON_SHAPE)));

    assert!(
        !is_error,
        "validation failure is a result, not a tool error"
    );
    assert_eq!(payload["conforms"], false);
    let results = payload["results"].as_array().expect("results array");
    assert!(
        !results.is_empty(),
        "minCount must produce a result: {payload}"
    );
    assert!(results.iter().any(|result| {
        result["focusNode"]
            .as_str()
            .is_some_and(|node| node.contains("http://example.com/alice"))
            && result["path"] == "<http://example.com/name>"
            && result["severity"] == "http://www.w3.org/ns/shacl#Violation"
            && result["message"] == "A person requires a name"
    }));
    assert_eq!(server.graph().len(), before, "validate must be read-only");
}

#[test]
fn satisfied_shape_returns_conforms_with_no_results() {
    let mut server = server(DATA_WITH_NAME);
    let (payload, is_error) = tool_payload(&validate(&mut server, json!(PERSON_SHAPE)));

    assert!(!is_error);
    assert_eq!(payload["conforms"], true);
    assert_eq!(payload["results"], json!([]));
}

#[test]
fn malformed_shapes_are_a_tool_error_not_a_protocol_error() {
    let response = validate(
        &mut server(DATA_MISSING_NAME),
        json!("@prefix sh: [ broken"),
    );

    assert!(
        response.get("error").is_none(),
        "must not be a protocol error: {response}"
    );
    assert_eq!(response["result"]["isError"], true);
    assert!(response["result"]["content"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("invalid SHACL shapes graph")));
}
