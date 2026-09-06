//! [SONNET-4.6] (sq-5xgxe, gh #3218) The shipped `sparq-mcp` binary, end to end: spawn
//! the real process, feed it a line-delimited JSON-RPC session on stdin, and read the
//! responses off stdout. This exercises the actual startup path (argument parsing →
//! dataset load → `serve_stdio`), not a re-implementation of it.
//!
//! The two facts it pins are the ones the binary exists for: a DATA_FILE is loaded into
//! the served graph, and `--allow-update` — and only `--allow-update` — turns the
//! mutating `update` tool on.
#![cfg(feature = "stdio")]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

const DATA: &str = "<http://ex/a> <http://ex/p> <http://ex/b> .\n";

/// Write `DATA` to a test-unique file and return its path (the caller removes it).
fn data_file(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sparq-mcp-bin-{}-{}.nt", tag, std::process::id()));
    std::fs::write(&path, DATA).expect("write test data");
    path
}

/// Run the binary with `args`, feed `lines` on stdin, and return the parsed responses.
fn run(args: &[&str], lines: &[&str]) -> (Vec<Value>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sparq-mcp"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sparq-mcp");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for line in lines {
            writeln!(stdin, "{}", line).expect("write request");
        }
    }
    // Dropping stdin closes the pipe, so the serve loop sees EOF and exits.
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for sparq-mcp");
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let responses = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad response {:?}: {}", l, e)))
        .collect();
    (responses, output.status.success())
}

fn tool_names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("tool name").to_string())
        .collect()
}

/// Extract the payload of a `tools/call` text result.
fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text")
}

#[test]
fn serves_the_loaded_dataset_read_only_by_default() {
    let path = data_file("readonly");
    let (responses, ok) = run(
        &[path.to_str().unwrap()],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"update","arguments":{"sparql":"INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/y> }"}}}"#,
        ],
    );
    std::fs::remove_file(&path).ok();

    assert!(ok, "the binary exited non-zero");
    assert_eq!(responses.len(), 4, "one response per request: {:?}", responses);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "sparq-mcp");

    // The DATA_FILE really was loaded into the served graph.
    let stats: Value = serde_json::from_str(tool_text(&responses[2])).expect("stats json");
    assert_eq!(stats["triples"], 1, "the data file was not loaded");

    // Read-only: `update` is neither advertised nor callable.
    let names = tool_names(&responses[1]);
    assert!(names.contains(&"query".to_string()), "{:?}", names);
    assert!(
        !names.contains(&"update".to_string()),
        "update advertised without --allow-update: {:?}",
        names
    );
    assert_eq!(
        responses[3]["error"]["code"], -32601,
        "update must be refused without --allow-update: {:?}",
        responses[3]
    );
}

#[test]
fn allow_update_flag_enables_the_mutating_tool() {
    let path = data_file("allowupdate");
    let (responses, ok) = run(
        &["--allow-update", path.to_str().unwrap()],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"update","arguments":{"sparql":"INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/y> }"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#,
        ],
    );
    std::fs::remove_file(&path).ok();

    assert!(ok, "the binary exited non-zero");
    let names = tool_names(&responses[0]);
    assert!(
        names.contains(&"update".to_string()),
        "--allow-update must advertise update: {:?}",
        names
    );
    assert_ne!(
        responses[1]["result"]["isError"], true,
        "update failed: {:?}",
        responses[1]
    );
    // The write really landed: the loaded triple plus the inserted one.
    let stats: Value = serde_json::from_str(tool_text(&responses[2])).expect("stats json");
    assert_eq!(stats["triples"], 2, "the update did not mutate the dataset");
}

#[test]
fn without_a_data_file_it_serves_an_empty_graph() {
    let (responses, ok) = run(
        &[],
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"stats","arguments":{}}}"#],
    );
    assert!(ok, "the binary exited non-zero");
    let stats: Value = serde_json::from_str(tool_text(&responses[0])).expect("stats json");
    assert_eq!(stats["triples"], 0);
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_sparq-mcp"))
        .arg("--help")
        .output()
        .expect("run sparq-mcp --help");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf-8");
    assert!(text.contains("--allow-update"), "{}", text);
}

#[test]
fn a_bad_argument_refuses_to_start() {
    for args in [vec!["--not-a-flag"], vec!["/no/such/file.ttl"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_sparq-mcp"))
            .args(&args)
            .output()
            .expect("run sparq-mcp");
        assert!(
            !output.status.success(),
            "{:?} must not start a server",
            args
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("sparq-mcp:"),
            "{:?} must explain itself on stderr",
            args
        );
    }
}
