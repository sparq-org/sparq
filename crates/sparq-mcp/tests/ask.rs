//! [OPUS-4.8] sq-jxjgr: the opt-in server-side NL `ask` tool, exercised through the REAL
//! MCP dispatch (`McpServer::handle_message`). This suite ONLY compiles/runs with the
//! `nlq` feature (`#![cfg(feature = "nlq")]`), so it covers the feature-ON state.
//!
//! NO live network call: the only path reachable end-to-end through the public MCP server
//! WITHOUT a configured backend is the **clean degrade** path — the load-bearing
//! fail-closed invariant. The REAL NL→SPARQL loop (with a scripted in-process backend) is
//! covered by the `nlq::tests` unit suite in src/nlq.rs, which can inject a stub `Llm`.
//! Here we assert that with the feature ON but NO backend configured:
//!   - the `ask` tool is NOT advertised in `tools/list` (degrade by absence); and
//!   - a direct `tools/call` for `ask` returns a clear "not configured" TOOL error
//!     (`isError: true`) — never a fabricated answer, never a panic / protocol error.
#![cfg(feature = "nlq")]

use serde_json::Value;
use sparq_core::Graph;
use sparq_mcp::McpServer;

const TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
ex:alice rdf:type ex:Person ; ex:name "Alice" .
ex:bob   rdf:type ex:Person ; ex:name "Bob" .
"#;

fn graph() -> Graph {
    Graph::load_str(TTL, "turtle").expect("load turtle")
}

fn call(server: &mut McpServer, raw: &str) -> Value {
    serde_json::from_str(&server.handle_message(raw).expect("request gets a response"))
        .expect("response is valid JSON")
}

/// Clear every backend-selecting env var so the test is hermetic, run `f`, then restore.
fn with_no_backend(f: impl FnOnce()) {
    let vars = [
        "SPARQ_NLQ_ENDPOINT_URL",
        "SPARQ_NLQ_ENDPOINT_MODEL",
        "SPARQ_NLQ_ENDPOINT_KEY",
        "ANTHROPIC_API_KEY",
    ];
    let saved: Vec<(&str, Option<String>)> =
        vars.iter().map(|v| (*v, std::env::var(v).ok())).collect();
    for v in vars {
        std::env::remove_var(v);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    for (v, val) in saved {
        match val {
            Some(val) => std::env::set_var(v, val),
            None => std::env::remove_var(v),
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

#[test]
fn ask_is_not_advertised_when_no_backend_is_configured() {
    with_no_backend(|| {
        let mut server = McpServer::new(graph());
        let resp = call(
            &mut server,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // The structured tools are always present; `ask` is hidden until a backend is set.
        assert!(names.contains(&"shapes"), "{names:?}");
        assert!(
            !names.contains(&"ask"),
            "ask must NOT be advertised with no backend configured: {names:?}"
        );
    });
}

#[test]
fn ask_call_with_no_backend_returns_a_clean_not_configured_error_not_an_answer() {
    with_no_backend(|| {
        let mut server = McpServer::new(graph());
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"ask",
            "arguments":{"question":"how many people are there?"}
        }}"#;
        let resp = call(&mut server, req);
        // Fail-closed: a TOOL error the client can read, NOT a JSON-RPC protocol error and
        // NOT a (fabricated) answer.
        assert!(resp["error"].is_null(), "{resp}");
        assert!(resp["result"]["isError"].as_bool().unwrap(), "{resp}");
        let msg = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(msg.contains("not configured"), "{msg}");
        // It names both backends so the operator can fix it.
        assert!(msg.contains("SPARQ_NLQ_ENDPOINT_URL"), "{msg}");
        assert!(msg.contains("ANTHROPIC_API_KEY"), "{msg}");
        // CRUCIAL: never a fabricated answer (no executed SPARQL, no result rows).
        assert!(
            !msg.contains("\"sparql\""),
            "an unconfigured ask must NEVER return a fabricated answer: {msg}"
        );
    });
}
