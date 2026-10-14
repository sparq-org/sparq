//! SPARQL 1.1 SERVICE (federated query) — end-to-end tests over a real loopback
//! HTTP endpoint.
//!
//! These exercise the PRODUCTION path (the ureq `HttpTransport`): a tiny in-process
//! `TcpListener` thread serves a canned SPARQL-Results-JSON body on `127.0.0.1`, the
//! engine runs `SERVICE <http://127.0.0.1:PORT/> { … }`, and we assert the remote
//! solutions are joined into the surrounding query. SILENT + malformed-body handling
//! are covered too. No public network is touched.
//!
//! Built only when the `service` feature is on:
//!   cargo test -p sparq-engine --features service --test service_federation

#![cfg(feature = "service")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use sparq_engine::{query, with_service_egress_allow};
use sparq_core::Graph;

/// The loopback test endpoints below resolve to `127.0.0.1`, which the default-deny
/// SSRF egress filter (bead sq-2v6f) refuses. The tests are legitimately
/// federating to a known-safe local server, so they opt the loopback host back in via
/// the allowlist — exercising the end-to-end allowlist path against the real ureq
/// transport, not just the policy classifier.
fn query_local(g: &Graph, q: &str) -> Result<sparq_engine::QueryResult, String> {
    with_service_egress_allow(["127.0.0.1".to_string()], || query(g, q))
}

/// Spawn a one-shot HTTP/1.1 server that replies to the first request with
/// `status` + `body` (Content-Type sparql-results+json). Returns the bound URL.
/// The thread serves exactly `n` requests then exits, so the OS port is freed.
fn serve(status: &'static str, body: String, n: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        tx.send(()).ok();
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else { return };
            // Drain the request (headers + form body) so the client's write completes.
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 {status}\r\n\
                 Content-Type: application/sparql-results+json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    rx.recv().unwrap(); // wait until the listener thread is live
    format!("http://{addr}/")
}

fn local_graph() -> Graph {
    // Two people locally; the remote endpoint supplies their names.
    Graph::load_str(
        "@prefix ex: <http://ex/> .\n\
         ex:alice a ex:Person .\n\
         ex:bob   a ex:Person .\n",
        "turtle",
    )
    .unwrap()
}

#[test]
fn service_returns_and_joins_remote_solutions() {
    // Remote endpoint binds ?s (matching local subjects) and ?name.
    let body = r#"{
        "head": { "vars": ["s", "name"] },
        "results": { "bindings": [
            { "s": {"type":"uri","value":"http://ex/alice"},
              "name": {"type":"literal","value":"Alice"} },
            { "s": {"type":"uri","value":"http://ex/bob"},
              "name": {"type":"literal","value":"Bob"} }
        ] }
    }"#
    .to_string();
    let url = serve("200 OK", body, 1);
    let g = local_graph();

    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res = query_local(&g, &q).expect("federated query");

    // Two local Persons joined with two remote names => two rows.
    assert_eq!(res.rows.len(), 2, "expected the remote names joined onto both local persons");
    let names: Vec<String> = res
        .rows
        .iter()
        .filter_map(|r| {
            let i = res.vars.iter().position(|v| v.as_str() == "name")?;
            r[i].as_ref().map(|t| t.to_string())
        })
        .collect();
    assert!(names.iter().any(|n| n.contains("Alice")), "got {names:?}");
    assert!(names.iter().any(|n| n.contains("Bob")), "got {names:?}");
}

#[test]
fn service_join_restricts_to_overlapping_subject() {
    // Remote binds a name only for alice; the join must drop bob.
    let body = r#"{
        "head": { "vars": ["s", "name"] },
        "results": { "bindings": [
            { "s": {"type":"uri","value":"http://ex/alice"},
              "name": {"type":"literal","value":"Alice"} }
        ] }
    }"#
    .to_string();
    let url = serve("200 OK", body, 1);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res = query_local(&g, &q).expect("federated query");
    assert_eq!(res.rows.len(), 1, "only alice has a remote name");
}

#[test]
fn service_silent_on_unreachable_endpoint_yields_no_failure() {
    // Port 1 is reserved/unused on loopback => connection refused immediately.
    let g = local_graph();
    let q = "PREFIX ex: <http://ex/>\n\
             SELECT ?s WHERE { ?s a ex:Person . \
             SERVICE SILENT <http://127.0.0.1:1/> { ?s ex:name ?name } }";
    let res = query(&g, q).expect("SILENT must not fail the query");
    // SILENT failure -> join identity -> the local persons survive unchanged.
    assert_eq!(res.rows.len(), 2, "SILENT keeps the surrounding bindings");
}

#[test]
fn service_non_silent_on_unreachable_endpoint_errors() {
    let g = local_graph();
    let q = "PREFIX ex: <http://ex/>\n\
             SELECT ?s WHERE { ?s a ex:Person . \
             SERVICE <http://127.0.0.1:1/> { ?s ex:name ?name } }";
    assert!(query(&g, q).is_err(), "non-SILENT SERVICE against a dead endpoint must error");
}

#[test]
fn service_malformed_response_errors_but_silent_swallows() {
    // A 200 with a non-JSON body.
    let url = serve("200 OK", "this is not sparql results json".to_string(), 1);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    assert!(query_local(&g, &q).is_err(), "malformed remote body must error (non-SILENT)");

    let url2 = serve("200 OK", "still not json".to_string(), 1);
    let q2 = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE SILENT <{url2}> {{ ?s ex:name ?name }} }}"
    );
    let res = query_local(&g, &q2).expect("SILENT swallows a malformed body");
    assert_eq!(res.rows.len(), 2, "SILENT malformed -> identity -> local persons survive");
}

#[test]
fn service_default_deny_refuses_loopback_endpoint() {
    // SSRF default-deny (bead sq-2v6f): with NO allowlist, a SERVICE
    // endpoint that resolves to loopback must be refused by the egress filter — even
    // though a real server is listening there. Non-SILENT => the query errors.
    let body = r#"{"head":{"vars":["name"]},"results":{"bindings":[]}}"#.to_string();
    // Serve 0 requests: the filter must reject before any socket is opened.
    let url = serve("200 OK", body, 0);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    // No `with_service_egress_allow` wrapper => default-deny is in force.
    let err = query(&g, &q).expect_err("loopback SERVICE must be refused by default-deny SSRF policy");
    assert!(
        err.to_lowercase().contains("egress")
            || err.to_lowercase().contains("private")
            || err.to_lowercase().contains("dns"),
        "expected an egress/SSRF refusal, got: {err}"
    );
    // sq-g2xs: the SSRF refusal originates in the ureq-3 resolver (as a
    // `ureq::Error::Io(PermissionDenied)`); the stable `SERVICE_EGRESS_REFUSED_MARKER`
    // the server `contains()`-classifies as a 403 (http.rs) MUST survive ureq-3's error
    // wrapping all the way out through `HttpTransport::fetch`. Guard that contract.
    assert!(
        err.contains(sparq_engine::SERVICE_EGRESS_REFUSED_MARKER),
        "the egress-refusal marker must survive ureq-3 error wrapping, got: {err}"
    );
}

#[test]
fn service_silent_default_deny_loopback_yields_identity() {
    // Same default-deny refusal, but under SILENT: the surrounding bindings survive.
    let body = r#"{"head":{"vars":["name"]},"results":{"bindings":[]}}"#.to_string();
    let url = serve("200 OK", body, 0);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE SILENT <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res = query(&g, &q).expect("SILENT swallows the egress refusal");
    assert_eq!(res.rows.len(), 2, "SILENT egress refusal -> identity -> local persons survive");
}

#[test]
fn service_http_error_status_handled() {
    let url = serve("500 Internal Server Error", "boom".to_string(), 1);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s WHERE {{ ?s a ex:Person . SERVICE SILENT <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res = query_local(&g, &q).expect("SILENT swallows a 5xx");
    assert_eq!(res.rows.len(), 2);
}
