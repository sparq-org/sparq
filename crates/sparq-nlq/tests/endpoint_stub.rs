//! `EndpointLlm` over a loopback stub — the PRODUCT-flavor NL-tool integration suite
//! (`sq-2m6zm.6`). Proves the REAL request/response/parse path AND the full
//! `Nlq::ask` round-trip work against an OpenAI-compatible chat-completions endpoint,
//! WITHOUT a live network call: a `TcpListener` on `127.0.0.1:0` returns a canned
//! chat-completion JSON body, exactly as Ollama / llama.cpp / vLLM / OpenAI would.
//!
//!   cargo test -p sparq-nlq --features nlq-endpoint --test endpoint_stub
//!
#![cfg(feature = "nlq-endpoint")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use sparq_core::Graph;
use sparq_nlq::endpoint::{EndpointConfig, EndpointLlm};
use sparq_nlq::{Llm, Nlq, TurnOutcome};

/// What the loopback stub recorded about one served request.
#[derive(Clone, Default)]
struct Captured {
    /// Raw request bodies it received (one per served request).
    bodies: Vec<String>,
    /// `Authorization` header values it saw (`None` if the header was absent).
    auth: Vec<Option<String>>,
    /// Request-target paths (the part after the method, before HTTP/1.1).
    paths: Vec<String>,
}

/// A loopback OpenAI-compatible chat-completions stub. Serves `n` requests, replying to
/// each with `response_body` wrapped in a 200, then exits (freeing the port). Records
/// each request body / auth header / path so the test can assert the wire contract.
fn serve_stub(response_body: String, n: usize) -> (String, Arc<Mutex<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Captured::default()));
    let cap = Arc::clone(&captured);
    let (ready_tx, ready_rx) = mpsc::channel();
    thread::spawn(move || {
        ready_tx.send(()).ok();
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            // Read until headers + exactly Content-Length body bytes are in hand (a
            // short read mid-body would drop the JSON we assert on).
            loop {
                if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..hdr_end]).to_string();
                    let clen = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if buf.len() >= hdr_end + 4 + clen {
                        break;
                    }
                }
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    Err(_) => break,
                }
            }
            let hdr_end = buf
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .unwrap_or(buf.len());
            let head = String::from_utf8_lossy(&buf[..hdr_end]).to_string();
            let body = String::from_utf8_lossy(&buf[(hdr_end + 4).min(buf.len())..]).to_string();
            let path = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let auth = head.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("authorization")
                    .then(|| v.trim().to_string())
            });
            {
                let mut c = cap.lock().unwrap();
                c.bodies.push(body);
                c.auth.push(auth);
                c.paths.push(path);
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(resp.as_bytes()).ok();
            stream.flush().ok();
        }
    });
    ready_rx.recv().expect("stub thread started");
    (format!("http://{addr}/v1"), captured)
}

/// A canned OpenAI chat-completions success body whose assistant message is `content`.
fn chat_completion(content: &str) -> String {
    serde_json::json!({
        "id": "chatcmpl-stub",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }]
    })
    .to_string()
}

fn tiny_graph() -> Graph {
    let ttl = r#"
        @prefix ex: <http://example.org/> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        ex:alice a ex:Person ; rdfs:label "Alice" .
        ex:bob a ex:Person ; rdfs:label "Bob" .
        ex:acme a ex:Company ; rdfs:label "Acme" .
    "#;
    Graph::load_str(ttl, "turtle").expect("tiny graph parses")
}

const COUNT_PEOPLE: &str = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
     PREFIX ex: <http://example.org/>\n\
     SELECT (COUNT(?p) AS ?n) WHERE { ?p rdf:type ex:Person }";

/// The REAL request/parse path: a single `complete` POSTs OpenAI-shaped JSON to
/// `{base}/chat/completions`, sends the bearer header, and parses the assistant content.
#[test]
fn complete_speaks_openai_chat_completions() {
    let (base, cap) = serve_stub(chat_completion("hello from the stub"), 1);
    let cfg = EndpointConfig::new(base, "test-model").with_api_key("sk-test-123");
    let llm = EndpointLlm::new(cfg).expect("client builds");
    assert_eq!(llm.model(), "test-model");

    let out = llm.complete("ping").expect("stub replies 200");
    assert_eq!(out, "hello from the stub");

    let c = cap.lock().unwrap();
    assert_eq!(c.paths[0], "/v1/chat/completions", "OpenAI-style path");
    assert_eq!(
        c.auth[0].as_deref(),
        Some("Bearer sk-test-123"),
        "bearer auth sent"
    );
    let body: serde_json::Value = serde_json::from_str(&c.bodies[0]).expect("request body is JSON");
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "ping");
}

/// No key configured → no `Authorization` header (local endpoints need none).
#[test]
fn no_key_omits_auth_header() {
    let (base, cap) = serve_stub(chat_completion("ok"), 1);
    let llm = EndpointLlm::new(EndpointConfig::new(base, "local-model")).unwrap();
    llm.complete("ping").unwrap();
    assert_eq!(cap.lock().unwrap().auth[0], None, "no bearer header");
}

/// A trailing slash on the base URL is tolerated (no doubled `//chat`).
#[test]
fn base_url_trailing_slash_tolerated() {
    let (base, cap) = serve_stub(chat_completion("ok"), 1);
    let llm = EndpointLlm::new(EndpointConfig::new(format!("{base}/"), "m")).unwrap();
    llm.complete("ping").unwrap();
    assert_eq!(cap.lock().unwrap().paths[0], "/v1/chat/completions");
}

/// An error envelope from the endpoint surfaces as a descriptive `Err`, not a panic.
#[test]
fn endpoint_error_envelope_surfaces() {
    let err_body = serde_json::json!({ "error": { "message": "model not found" } }).to_string();
    let (base, _cap) = serve_stub_status(err_body, "404 Not Found", 1);
    let llm = EndpointLlm::new(EndpointConfig::new(base, "missing")).unwrap();
    let err = llm.complete("ping").unwrap_err();
    assert!(err.contains("model not found"), "got: {err}");
    assert!(err.contains("404"), "got: {err}");
}

/// A 200 with no assistant content is an error (never a silent empty answer).
#[test]
fn empty_content_is_an_error() {
    let body =
        serde_json::json!({ "choices": [{ "message": { "role": "assistant" } }] }).to_string();
    let (base, _cap) = serve_stub(body, 1);
    let llm = EndpointLlm::new(EndpointConfig::new(base, "m")).unwrap();
    let err = llm.complete("ping").unwrap_err();
    assert!(err.contains("no assistant message content"), "got: {err}");
}

/// The PRODUCT round-trip: the full `Nlq::ask` loop driven by `EndpointLlm`. The stub
/// returns a fenced SPARQL block; the loop extracts → parses → executes it against the
/// real engine and returns the answer + executed query (the soundness contract: the
/// caller can verify the SPARQL that produced the result).
#[test]
fn nlq_ask_round_trip_over_endpoint() {
    let g = tiny_graph();
    let (base, cap) = serve_stub(
        chat_completion(&format!("```sparql\n{COUNT_PEOPLE}\n```")),
        1,
    );
    let llm = EndpointLlm::new(EndpointConfig::new(base, "cheap-model")).unwrap();
    let nlq = Nlq::new(&g, Box::new(llm));

    let answer = nlq
        .ask("How many people are there?")
        .expect("loop succeeds");
    assert_eq!(answer.repairs, 0);
    assert_eq!(answer.result.len(), 1);
    // Soundness: the executed SPARQL is returned for verification.
    assert!(answer.sparql.contains("COUNT(?p)"));
    assert_eq!(answer.transcript[0].outcome, TurnOutcome::Ok { rows: 1 });
    let n = answer.result.rows[0][0].as_ref().expect("bound count");
    assert!(n.to_string().contains('2'), "two people, got {n}");

    // The endpoint actually received the grounding prompt (real call, not a mock seam).
    let body: serde_json::Value =
        serde_json::from_str(&cap.lock().unwrap().bodies[0]).expect("JSON body");
    let sent = body["messages"][0]["content"].as_str().unwrap();
    assert!(sent.contains("Question: How many people are there?"));
}

/// `from_env` requires base URL + model and is the only env path; missing either errors.
#[test]
fn config_from_env_requires_url_and_model() {
    // Serialize every env-mutating test against ONE process-wide lock: `set_var` /
    // `remove_var` are not thread-safe, and cargo runs this binary's tests on a thread
    // pool. No other test in this binary reads these vars, so this lock is sufficient.
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();
    use sparq_nlq::endpoint::{ENV_API_KEY, ENV_BASE_URL, ENV_MODEL};
    // Use canned absent state.
    std::env::remove_var(ENV_BASE_URL);
    std::env::remove_var(ENV_MODEL);
    std::env::remove_var(ENV_API_KEY);
    let err = EndpointConfig::from_env().unwrap_err();
    assert!(err.contains(ENV_BASE_URL), "got: {err}");

    std::env::set_var(ENV_BASE_URL, "http://localhost:11434/v1");
    let err = EndpointConfig::from_env().unwrap_err();
    assert!(err.contains(ENV_MODEL), "got: {err}");

    std::env::set_var(ENV_MODEL, "llama3.1");
    let cfg = EndpointConfig::from_env().expect("url + model present");
    assert_eq!(cfg.base_url, "http://localhost:11434/v1");
    assert_eq!(cfg.model, "llama3.1");
    assert!(cfg.api_key.is_none(), "key optional");

    // Clean up so we do not leak env into other tests in this binary.
    std::env::remove_var(ENV_BASE_URL);
    std::env::remove_var(ENV_MODEL);
}

/// Like [`serve_stub`] but with a caller-chosen HTTP status line (for the error path).
fn serve_stub_status(
    response_body: String,
    status_line: &str,
    n: usize,
) -> (String, Arc<Mutex<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Captured::default()));
    let status = status_line.to_string();
    let (ready_tx, ready_rx) = mpsc::channel();
    thread::spawn(move || {
        ready_tx.send(()).ok();
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut tmp = [0u8; 8192];
            let mut buf = Vec::new();
            loop {
                if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..hdr_end]).to_string();
                    let clen = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if buf.len() >= hdr_end + 4 + clen {
                        break;
                    }
                }
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    Err(_) => break,
                }
            }
            let resp = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                response_body.len(),
                response_body
            );
            stream.write_all(resp.as_bytes()).ok();
            stream.flush().ok();
        }
    });
    ready_rx.recv().expect("stub thread started");
    (format!("http://{addr}/v1"), captured)
}
