//! [OPUS-4.8] (sq-0z43i, gh #909) The stdio transport round-trip — only compiled
//! with the `stdio` feature. Drives `sparq_mcp::serve` over an in-memory
//! reader/writer pair (a real line-delimited JSON-RPC session: two requests and a
//! notification on stdin), then asserts the writer holds exactly the two responses,
//! one JSON object per line, with the notification correctly producing no line. This
//! is the REAL transport loop (not a mock), so the feature-ON state is exercised
//! end-to-end.
#![cfg(feature = "stdio")]

use serde_json::Value;
use sparq_core::Graph;
use sparq_mcp::McpServer;

#[test]
fn serve_handles_a_line_delimited_session() {
    let graph = Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap();
    let mut server = McpServer::new(graph);

    // Two requests bracketing a notification (which must yield no output line).
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#,
        "\n",
    );

    let reader = std::io::BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sparq_mcp::serve(&mut server, reader, &mut out).expect("serve loop");

    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "two responses, the notification produced none: {:?}", lines);

    let init: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(init["id"], 1);
    assert!(init["result"]["serverInfo"]["name"].is_string());

    let stats: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(stats["id"], 2);
    let payload: Value =
        serde_json::from_str(stats["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["triples"], 1);
}

// [OPUS-5] gh #2497: a JSON-RPC batch survives the line-delimited transport intact — a
// batch array arrives on ONE line and its array of responses goes back out on ONE line,
// so nothing about batch receipt depends on re-framing the stream.
#[test]
fn serve_carries_a_batch_on_a_single_line_each_way() {
    let graph = Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap();
    let mut server = McpServer::new(graph);

    let input = concat!(
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"},"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"ping"}]"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
        "\n",
    );

    let reader = std::io::BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sparq_mcp::serve(&mut server, reader, &mut out).expect("serve loop");

    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "the whole batch is one output line, then the single request's line: {:?}",
        lines
    );

    let batch: Value = serde_json::from_str(lines[0]).unwrap();
    let batch = batch.as_array().expect("the batch line holds an array");
    assert_eq!(batch.len(), 2, "the notification element contributes nothing");
    assert_eq!(batch[0]["id"], 1);
    assert_eq!(batch[1]["id"], 2);

    let single: Value = serde_json::from_str(lines[1]).unwrap();
    assert!(!single.is_array(), "a single request still gets a bare object");
    assert_eq!(single["id"], 3);
}

// [GPT-5.6] sq-y1ljq: malformed tools/call names are protocol errors, not tool results.
#[test]
fn serve_rejects_missing_or_non_string_tool_names() {
    let graph = Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap();
    let mut server = McpServer::new(graph);

    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":null}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":123}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#,
        "\n",
    );

    let reader = std::io::BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sparq_mcp::serve(&mut server, reader, &mut out).expect("serve loop");

    let text = String::from_utf8(out).unwrap();
    let responses: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 4);

    for (index, response) in responses[..3].iter().enumerate() {
        assert_eq!(response["id"], index + 1);
        assert_eq!(response["error"]["code"], -32602);
        assert!(response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("tools/call requires a string `name`")));
        assert!(response.get("result").is_none());
    }

    let stats = &responses[3];
    assert_eq!(stats["id"], 4);
    assert!(stats.get("error").is_none());
    let payload: Value =
        serde_json::from_str(stats["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["triples"], 1);
}
