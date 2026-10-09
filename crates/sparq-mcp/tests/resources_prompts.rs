//! [SONNET-4.6] sq-sjey1 (gh #3220) The MCP **resources** + **prompts** surfaces, end to
//! end through the real `McpServer::handle_message` dispatch — no mock bypasses it.
//!
//! The load-bearing invariants asserted here:
//!   - `initialize` declares `resources` and `prompts`, with every sub-capability flag
//!     FALSE (this server implements neither `subscribe` nor `listChanged`);
//!   - `resources/list` names the dataset, the default graph, and every NAMED graph, and
//!     `resources/read` returns THAT graph's triples and no sibling's;
//!   - a URI the server does not serve is `RESOURCE_NOT_FOUND` (-32002);
//!   - `prompts/get` REFUSES an IRI argument that would break out of the SPARQL `IRIREF`
//!     it is interpolated into, and an unknown prompt name, with `INVALID_PARAMS`;
//!   - the surfaces are read-only: nothing here mutates the graph.

use serde_json::Value;
use sparq_core::Graph;
use sparq_mcp::McpServer;

/// A dataset with a default graph and two named graphs, so "read the RIGHT graph" is
/// actually witnessed (a single-graph fixture could not distinguish them).
const TRIG: &str = r#"@prefix ex: <http://ex/> .
ex:alice a ex:Person ; ex:name "Alice" .
<http://ex/g1> { ex:bob a ex:Person ; ex:name "Bob" . }
<http://ex/g2> { ex:carol a ex:Person ; ex:name "Carol" . }
"#;

fn server() -> McpServer {
    McpServer::new(Graph::load_dataset(TRIG, "trig").expect("load trig"))
}

fn call(server: &mut McpServer, raw: &str) -> Value {
    let resp = server.handle_message(raw).expect("request gets a response");
    serde_json::from_str(&resp).expect("response is valid JSON")
}

fn request(id: u32, method: &str, params: Value) -> String {
    serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": method, "params": params,
    }))
    .expect("serialize request")
}

#[test]
fn initialize_declares_resources_and_prompts_without_overclaiming() {
    let mut s = server();
    let resp = call(&mut s, &request(1, "initialize", serde_json::json!({})));
    let caps = &resp["result"]["capabilities"];
    assert!(
        caps.get("tools").is_some(),
        "tools capability lost: {}",
        caps
    );
    assert_eq!(caps["resources"]["subscribe"].as_bool(), Some(false));
    assert_eq!(caps["resources"]["listChanged"].as_bool(), Some(false));
    assert_eq!(caps["prompts"]["listChanged"].as_bool(), Some(false));
}

#[test]
fn resources_list_names_the_dataset_default_graph_and_every_named_graph() {
    let mut s = server();
    let resp = call(&mut s, &request(2, "resources/list", serde_json::json!({})));
    let uris: Vec<&str> = resp["result"]["resources"]
        .as_array()
        .expect("resources array")
        .iter()
        .map(|r| r["uri"].as_str().expect("uri is a string"))
        .collect();
    assert_eq!(
        uris,
        vec![
            "urn:sparq:dataset",
            "urn:sparq:graph:default",
            "http://ex/g1",
            "http://ex/g2",
        ]
    );
}

#[test]
fn resources_read_returns_exactly_the_addressed_graph() {
    let mut s = server();

    let default_graph = call(
        &mut s,
        &request(
            3,
            "resources/read",
            serde_json::json!({"uri": "urn:sparq:graph:default"}),
        ),
    );
    let text = default_graph["result"]["contents"][0]["text"]
        .as_str()
        .expect("contents text");
    assert!(
        text.contains("Alice"),
        "default graph triple missing: {}",
        text
    );
    assert!(!text.contains("Bob"), "named-graph triple leaked: {}", text);
    assert_eq!(
        default_graph["result"]["contents"][0]["mimeType"].as_str(),
        Some("application/n-triples")
    );

    let named = call(
        &mut s,
        &request(
            4,
            "resources/read",
            serde_json::json!({"uri": "http://ex/g1"}),
        ),
    );
    let text = named["result"]["contents"][0]["text"]
        .as_str()
        .expect("contents text");
    assert!(text.contains("Bob"), "named graph triple missing: {}", text);
    assert!(
        !text.contains("Carol"),
        "sibling named graph leaked: {}",
        text
    );
    assert!(!text.contains("Alice"), "default graph leaked: {}", text);
}

#[test]
fn resources_read_of_the_dataset_returns_the_void_descriptor() {
    let mut s = server();
    let resp = call(
        &mut s,
        &request(
            5,
            "resources/read",
            serde_json::json!({"uri": "urn:sparq:dataset"}),
        ),
    );
    let text = resp["result"]["contents"][0]["text"]
        .as_str()
        .expect("contents text");
    assert!(
        text.contains("rdfs.org/ns/void#"),
        "not a VoID descriptor: {}",
        text
    );
}

#[test]
fn resources_read_of_an_unserved_uri_is_resource_not_found() {
    let mut s = server();
    let resp = call(
        &mut s,
        &request(
            6,
            "resources/read",
            serde_json::json!({"uri": "http://ex/nope"}),
        ),
    );
    assert_eq!(resp["error"]["code"].as_i64(), Some(-32002), "{}", resp);
    assert!(resp.get("result").is_none(), "{}", resp);
}

#[test]
fn resources_read_without_a_uri_is_invalid_params() {
    let mut s = server();
    let resp = call(&mut s, &request(7, "resources/read", serde_json::json!({})));
    assert_eq!(resp["error"]["code"].as_i64(), Some(-32602), "{}", resp);
}

#[test]
fn prompts_list_advertises_the_catalog_with_its_arguments() {
    let mut s = server();
    let resp = call(&mut s, &request(8, "prompts/list", serde_json::json!({})));
    let prompts = resp["result"]["prompts"].as_array().expect("prompts array");
    let names: Vec<&str> = prompts
        .iter()
        .map(|p| p["name"].as_str().expect("name is a string"))
        .collect();
    assert_eq!(
        names,
        vec![
            "explore-dataset",
            "count-by-class",
            "class-overview",
            "predicate-usage"
        ]
    );
    let overview = prompts
        .iter()
        .find(|p| p["name"] == "class-overview")
        .expect("class-overview advertised");
    assert_eq!(overview["arguments"][0]["name"].as_str(), Some("class"));
    assert_eq!(overview["arguments"][0]["required"].as_bool(), Some(true));
}

#[test]
fn prompts_get_renders_a_user_message_carrying_the_argument() {
    let mut s = server();
    let resp = call(
        &mut s,
        &request(
            9,
            "prompts/get",
            serde_json::json!({
                "name": "class-overview",
                "arguments": {"class": "http://ex/Person"},
            }),
        ),
    );
    let message = &resp["result"]["messages"][0];
    assert_eq!(message["role"].as_str(), Some("user"));
    assert_eq!(message["content"]["type"].as_str(), Some("text"));
    let text = message["content"]["text"].as_str().expect("text");
    assert!(text.contains("<http://ex/Person>"), "{}", text);
}

#[test]
fn prompts_get_renders_an_argument_free_prompt() {
    let mut s = server();
    let resp = call(
        &mut s,
        &request(
            10,
            "prompts/get",
            serde_json::json!({"name": "explore-dataset"}),
        ),
    );
    let text = resp["result"]["messages"][0]["content"]["text"]
        .as_str()
        .expect("text");
    assert!(text.contains("SELECT"), "{}", text);
}

/// THE guard, through the real dispatch: an argument crafted to close the SPARQL
/// `IRIREF` it is interpolated into must be REFUSED, not rendered.
#[test]
fn prompts_get_refuses_an_iri_argument_that_escapes_the_iriref() {
    let mut s = server();
    let hostile = "http://ex/Person> . ?x ?y ?z . <http://ex/Other";
    let resp = call(
        &mut s,
        &request(
            11,
            "prompts/get",
            serde_json::json!({"name": "class-overview", "arguments": {"class": hostile}}),
        ),
    );
    assert_eq!(resp["error"]["code"].as_i64(), Some(-32602), "{}", resp);
    assert!(
        resp.get("result").is_none(),
        "a hostile IRI must not render a prompt: {}",
        resp
    );
}

#[test]
fn prompts_get_of_an_unknown_prompt_is_invalid_params() {
    let mut s = server();
    let resp = call(
        &mut s,
        &request(
            12,
            "prompts/get",
            serde_json::json!({"name": "no-such-prompt"}),
        ),
    );
    assert_eq!(resp["error"]["code"].as_i64(), Some(-32602), "{}", resp);
}

/// Both surfaces are READ-ONLY: exercising them on a default (read-only) server leaves
/// the graph exactly as it was.
#[test]
fn the_surfaces_never_mutate_the_graph() {
    let mut s = server();
    let before = s.graph().len();
    call(
        &mut s,
        &request(13, "resources/list", serde_json::json!({})),
    );
    call(
        &mut s,
        &request(
            14,
            "resources/read",
            serde_json::json!({"uri": "urn:sparq:graph:default"}),
        ),
    );
    call(&mut s, &request(15, "prompts/list", serde_json::json!({})));
    call(
        &mut s,
        &request(
            16,
            "prompts/get",
            serde_json::json!({"name": "count-by-class"}),
        ),
    );
    assert_eq!(s.graph().len(), before);
}
