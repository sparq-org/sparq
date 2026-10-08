//! [FABLE-5] sq-lsp7k.1.6: JSON-RPC round trips for the opt-in shape-aware
//! `describe_form` tool (feature `shacl`).
//!
//! The load-bearing invariant: the tool returns the SAME JSON string
//! `sparq_forms::derive_form` produces for the same inputs — verbatim, no key
//! reshaping — so every renderer/agent consumes the one canonical
//! `FormDescription` contract. The expectation is computed INDEPENDENTLY in the
//! test through the real derivation path (not a captured golden), so a mutated
//! tool output fails the string comparison.

#![cfg(feature = "shacl")]

use oxrdf::{BlankNode, NamedNode, Term};
use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_forms::{derive_form, FormOptions, Mode};
use sparq_mcp::McpServer;

// Fixtures use NAMED property shapes (no anonymous blank nodes) so two
// independent parses of the same text yield term-identical graphs and the
// verbatim string comparison is deterministic.
const DATA: &str = r#"@prefix ex: <http://example.com/> .
ex:alice a ex:Person ; ex:name "Alice" .
"#;

const PERSON_SHAPE: &str = r#"@prefix ex: <http://example.com/> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property ex:NameShape .

ex:NameShape sh:path ex:name ;
    sh:datatype xsd:string ;
    sh:minCount 1 ;
    sh:name "Name" .
"#;

const ALICE: &str = "http://example.com/alice";

fn server() -> McpServer {
    McpServer::new(Graph::load_str(DATA, "turtle").expect("data fixture parses"))
}

fn message(server: &mut McpServer, request: Value) -> Value {
    serde_json::from_str(
        &server
            .handle_message(&request.to_string())
            .expect("request produces a response"),
    )
    .expect("response is JSON")
}

fn describe(server: &mut McpServer, arguments: Value) -> Value {
    message(
        server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": { "name": "describe_form", "arguments": arguments }
        }),
    )
}

fn tool_text(response: &Value) -> (String, bool) {
    let result = &response["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .expect("tool result has text")
            .to_string(),
        result["isError"].as_bool().unwrap_or(false),
    )
}

/// What the tool must emit: `derive_form` over the same parsed graphs, pretty-printed.
fn expected_json(focus: &Term, opts: &FormOptions) -> String {
    let data = Graph::load_str(DATA, "turtle").expect("data fixture parses");
    let shapes = Graph::load_str(PERSON_SHAPE, "turtle").expect("shapes fixture parses");
    serde_json::to_string_pretty(&derive_form(&data, &shapes, focus, opts))
        .expect("FormDescription serializes")
}

#[test]
fn describe_form_is_advertised_with_feature() {
    let response = message(
        &mut server(),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    let tool = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == "describe_form")
        .expect("describe_form is advertised when the feature is on");
    assert_eq!(tool["inputSchema"]["required"], json!(["focus", "shapes"]));
}

#[test]
fn returns_derive_form_json_verbatim() {
    let mut server = server();
    let before = server.graph().len();
    let (text, is_error) = tool_text(&describe(
        &mut server,
        json!({ "focus": ALICE, "shapes": PERSON_SHAPE }),
    ));
    assert!(
        !is_error,
        "well-formed inputs must not be a tool error: {}",
        text
    );

    let focus = Term::NamedNode(NamedNode::new(ALICE).expect("focus IRI"));
    assert_eq!(
        text,
        expected_json(&focus, &FormOptions::default()),
        "describe_form must return derive_form's FormDescription JSON verbatim"
    );

    // Non-vacuity: the payload really is the derived edit form for THIS focus —
    // mutate any of these expected values and the test goes red.
    let payload: Value = serde_json::from_str(&text).expect("payload is JSON");
    assert_eq!(payload["focus"]["value"], ALICE);
    assert_eq!(payload["mode"], "edit");
    assert_eq!(payload["shape"]["value"], "http://example.com/PersonShape");
    let fields: Vec<&Value> = payload["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .flat_map(|group| group["fields"].as_array().expect("fields array"))
        .collect();
    let name = fields
        .iter()
        .find(|field| field["path"] == "<http://example.com/name>")
        .expect("the declared ex:name field is present");
    assert_eq!(name["label"], "Name");
    assert_eq!(name["required"], true);
    assert_eq!(name["values"][0]["term"]["value"], "Alice");

    assert_eq!(
        server.graph().len(),
        before,
        "describe_form must be read-only"
    );
}

#[test]
fn honours_mode_and_explicit_shape_arguments() {
    let (text, is_error) = tool_text(&describe(
        &mut server(),
        json!({
            "focus": ALICE,
            "shapes": PERSON_SHAPE,
            "mode": "view",
            "shape": "http://example.com/PersonShape"
        }),
    ));
    assert!(!is_error, "{}", text);

    let focus = Term::NamedNode(NamedNode::new(ALICE).expect("focus IRI"));
    let shape =
        Term::NamedNode(NamedNode::new("http://example.com/PersonShape").expect("shape IRI"));
    let opts = FormOptions {
        mode: Mode::View,
        shape: Some(shape),
        ..FormOptions::default()
    };
    assert_eq!(text, expected_json(&focus, &opts));

    let payload: Value = serde_json::from_str(&text).expect("payload is JSON");
    assert_eq!(payload["mode"], "view");
    let editable = payload["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .flat_map(|group| group["fields"].as_array().expect("fields array"))
        .any(|field| field["editable"] == true);
    assert!(!editable, "view mode derives no editable field");
}

#[test]
fn blank_node_focus_uses_the_underscore_colon_convention() {
    let (text, is_error) = tool_text(&describe(
        &mut server(),
        json!({ "focus": "_:absent", "shapes": PERSON_SHAPE }),
    ));
    assert!(!is_error, "{}", text);
    // The data graph has no blank nodes, so the term is constructed from the
    // label alone on both sides — verbatim equality stays deterministic.
    let focus = Term::BlankNode(BlankNode::new("absent").expect("bnode label"));
    assert_eq!(text, expected_json(&focus, &FormOptions::default()));
}

#[test]
fn bad_focus_shapes_or_mode_are_tool_errors_not_protocol_errors() {
    let cases = [
        (
            json!({ "focus": "not an iri", "shapes": PERSON_SHAPE }),
            "invalid focus IRI",
        ),
        (
            json!({ "focus": ALICE, "shapes": "@prefix sh: [ broken" }),
            "invalid SHACL shapes graph",
        ),
        (
            json!({ "focus": ALICE, "shapes": PERSON_SHAPE, "mode": "banana" }),
            "unknown mode",
        ),
        (
            json!({ "focus": ALICE, "shapes": PERSON_SHAPE, "shape": "not an iri" }),
            "invalid shape IRI",
        ),
        (
            json!({ "shapes": PERSON_SHAPE }),
            "missing required string argument",
        ),
    ];
    for (arguments, needle) in cases {
        let response = describe(&mut server(), arguments.clone());
        assert!(
            response.get("error").is_none(),
            "must not be a protocol error for {}: {}",
            arguments,
            response
        );
        let (text, is_error) = tool_text(&response);
        assert!(is_error, "must be a tool error for {}: {}", arguments, text);
        assert!(
            text.contains(needle),
            "error for {} must mention `{}`: {}",
            arguments,
            needle,
            text
        );
    }
}
