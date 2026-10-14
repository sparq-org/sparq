//! SERVICE federation egress allowlist — server integration. (sq-4w18)
//!
//! Runs only with `cargo test -p sparq-server --features service`. Boots the real axum
//! server in-process and drives `/sparql` with a `SERVICE <iri>` clause, asserting the
//! server's default-DENY-all posture and the allowlist opt-in:
//!
//!   * with NO allowlist (the default), a SERVICE to a loopback endpoint is REFUSED
//!     before any socket is opened (the fake remote serves 0 requests and the query
//!     still errors) — the SSRF guard;
//!   * with the loopback host on the allowlist, the SAME query reaches the endpoint and
//!     joins the remote solution;
//!   * `SERVICE SILENT` under the deny posture yields the join identity (the query
//!     succeeds with the surrounding bindings intact, no network call);
//!   * sq-a7jw4 — a PORT-SCOPED entry (`127.0.0.1:<ephemeral>`) permits exactly
//!     that endpoint, and REJECTS the same loopback host on a different port (403, no dial).
#![cfg(feature = "service")]

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::sync::mpsc;
use std::thread;

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig, ServiceAllowlist};
use tokio::net::TcpListener;

const DATA: &str = "@prefix ex: <http://ex/> . ex:alice a ex:Person . ex:bob a ex:Person .";

/// A one-shot loopback HTTP endpoint returning a canned SPARQL-Results-JSON body to the
/// first `n` requests, then exiting.
///
/// sq-4w18: returns `(url, accepts)` where `accepts` is a channel that fires
/// once for every connection the listener actually `accept()`s. A deny-posture test can
/// therefore PROVE the egress filter refused before any socket was opened — by asserting
/// nothing arrives on `accepts` within a window — rather than relying on a request count
/// (a count of 0 alone does not distinguish "no socket opened" from "dialled but the OS
/// returned a fast `ECONNREFUSED`"). With `n == 0` the listener accepts nothing.
fn serve(body: String, n: usize) -> (String, mpsc::Receiver<()>) {
    let listener = StdListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (accept_tx, accept_rx) = mpsc::channel();
    thread::spawn(move || {
        ready_tx.send(()).ok();
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else { return };
            accept_tx.send(()).ok(); // a connection was accepted
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/sparql-results+json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    ready_rx.recv().unwrap();
    (format!("http://{addr}/"), accept_rx)
}

async fn spawn_with(config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn remote_body() -> String {
    r#"{ "head": { "vars": ["s", "name"] },
        "results": { "bindings": [
            { "s": {"type":"uri","value":"http://ex/alice"}, "name": {"type":"literal","value":"Alice"} },
            { "s": {"type":"uri","value":"http://ex/bob"},   "name": {"type":"literal","value":"Bob"} }
        ] } }"#
        .to_string()
}

#[tokio::test]
async fn empty_allowlist_refuses_service_before_any_network_call() {
    // sq-4w18: the fake remote stays LIVE and ready to accept one connection
    // (n == 1). Under the strict deny posture the engine refuses the host before DNS
    // resolution, so the server must never dial it — we PROVE that by asserting nothing
    // is ever `accept()`ed on the live listener (see the `accepts` channel below). A
    // bare request count of 0 would not distinguish "no socket opened" from "dialled but
    // got a fast ECONNREFUSED on a closed port".
    let (remote, accepts) = serve(remote_body(), 1);
    let host = remote.trim_start_matches("http://").trim_end_matches('/');
    // Default config + a SHORT query timeout: if the strict pre-DNS refusal ever
    // regresses and the server actually dials the (live) endpoint, the request must not
    // hang on the 30s default and slow CI — the timeout bounds the failure.
    // empty allowlist => deny ALL SERVICE
    let config = ServerConfig { query_timeout: Some(std::time::Duration::from_secs(2)), ..ServerConfig::default() };
    let base = spawn_with(config).await;
    let q = format!(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <http://{host}/> {{ ?s ex:name ?name }} }}"
    );
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    // (sq-iu0c) A non-SILENT SERVICE blocked by the egress allowlist is a POLICY
    // refusal, not a server fault — the server maps the engine's egress-refusal marker to an
    // honest 403 (Forbidden), distinct from the 500 a genuine execution error gives. The
    // egress/allowlist detail names the refused host, so the server SANITIZES it (see
    // `forbidden_egress`/`sanitized_error` in http.rs): the body carries only the stable
    // generic policy-refusal class, and the host detail goes to the server log. The 403 plus
    // the "no socket dialled" assertion below are the load-bearing proof of the strict
    // pre-DNS refusal.
    assert_eq!(
        resp.status(),
        403,
        "blocked SERVICE must be a 403 policy refusal, got {}",
        resp.status(),
    );
    let body = resp.text().await.unwrap().to_lowercase();
    assert!(
        body.contains("service") && body.contains("refus") && body.contains("egress allowlist"),
        "expected the sanitized egress-refusal class in the body, got: {body}"
    );
    // The refused host must NOT leak into the client body (info-leak / SSRF-probe oracle).
    assert!(
        !body.contains(&host.to_lowercase()),
        "the refused host must not reach the client body, got: {body}"
    );
    // The query has already returned; give any in-flight dial a brief window, then assert
    // the live listener accepted NOTHING — i.e. the refusal happened before any socket
    // was opened to the endpoint.
    assert!(
        accepts.recv_timeout(std::time::Duration::from_millis(250)).is_err(),
        "strict deny posture must refuse BEFORE dialling — the listener must accept no connection",
    );
}

#[tokio::test]
async fn allowlisted_loopback_host_reaches_the_endpoint() {
    let (remote, _accepts) = serve(remote_body(), 1);
    let host = remote.trim_start_matches("http://").trim_end_matches('/');
    let bare_host = host.split(':').next().unwrap().to_string(); // "127.0.0.1"
    // Allowlist the loopback host => SERVICE to it is permitted.
    let mut config = ServerConfig::default();
    let mut allow = ServiceAllowlist::default();
    allow.add(&bare_host).unwrap();
    config.service_allow = allow;
    let base = spawn_with(config).await;
    let q = format!(
        "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <http://{host}/> {{ ?s ex:name ?name }} }}"
    );
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "allowlisted SERVICE must succeed");
    let body = resp.text().await.unwrap();
    assert!(body.contains("Alice"), "expected the remote name joined in: {body}");
    assert!(body.contains("Bob"), "expected the remote name joined in: {body}");
}

#[tokio::test]
async fn port_scoped_allowlist_permits_exact_endpoint() {
    // sq-a7jw4: the in-process loopback SERVICE federation use case — permit
    // EXACTLY `127.0.0.1:<ephemeral>` (the mock endpoint's bound port), and it is reached.
    let (remote, _accepts) = serve(remote_body(), 1);
    let host = remote.trim_start_matches("http://").trim_end_matches('/'); // 127.0.0.1:<port>
    // Allowlist the FULL host:port (port-scoped) — exactly the mock endpoint, nothing wider.
    let mut config = ServerConfig::default();
    let mut allow = ServiceAllowlist::default();
    allow.add(host).unwrap(); // e.g. "127.0.0.1:54321"
    config.service_allow = allow;
    let base = spawn_with(config).await;
    let q = format!(
        "PREFIX ex: <http://ex/> SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <http://{host}/> {{ ?s ex:name ?name }} }}"
    );
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "exact port-scoped SERVICE must succeed");
    let body = resp.text().await.unwrap();
    assert!(body.contains("Alice") && body.contains("Bob"), "expected the remote joined in: {body}");
}

#[tokio::test]
async fn port_scoped_allowlist_rejects_same_host_other_port() {
    // sq-a7jw4: the security property — a `127.0.0.1:<portA>` entry must REJECT
    // the SAME loopback host on a DIFFERENT port. We allowlist one ephemeral port but the
    // SERVICE IRI targets a *second*, live loopback endpoint on another port; the server must
    // refuse it (403) BEFORE dialling, proven by the live listener accepting nothing.
    let allowed_listener = StdListener::bind("127.0.0.1:0").expect("bind loopback");
    let allowed_addr = allowed_listener.local_addr().unwrap(); // the permitted host:port
    drop(allowed_listener); // we only need its host:port string for the allowlist entry

    // A SECOND live endpoint on a DIFFERENT port — the off-allowlist target.
    let (other_remote, other_accepts) = serve(remote_body(), 1);
    let other_host = other_remote.trim_start_matches("http://").trim_end_matches('/');
    assert_ne!(
        allowed_addr.port(),
        other_host.rsplit(':').next().unwrap().parse::<u16>().unwrap(),
        "the two endpoints must be on different ports for this test to be meaningful",
    );

    let mut config = ServerConfig {
        query_timeout: Some(std::time::Duration::from_secs(2)),
        ..ServerConfig::default()
    };
    let mut allow = ServiceAllowlist::default();
    allow.add(&allowed_addr.to_string()).unwrap(); // port-scoped: ONLY 127.0.0.1:<portA>
    config.service_allow = allow;
    let base = spawn_with(config).await;
    let q = format!(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE {{ ?s a ex:Person . SERVICE <http://{other_host}/> {{ ?s ex:name ?name }} }}"
    );
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "same host on a non-allowlisted port must be a 403 policy refusal, got {}",
        resp.status(),
    );
    // And the off-allowlist endpoint must never have been dialled.
    assert!(
        other_accepts.recv_timeout(std::time::Duration::from_millis(250)).is_err(),
        "a port off the scoped allowlist must be refused BEFORE any socket is opened",
    );
}

#[tokio::test]
async fn silent_service_under_deny_yields_identity() {
    // sq-4w18: live listener (n == 1) so a regression that dials would hit a
    // real accept, plus a SHORT query timeout so such a regression cannot hang CI on the
    // 30s default. (SILENT swallows the refusal either way, but the timeout bounds it.)
    let (remote, _accepts) = serve(remote_body(), 1);
    let host = remote.trim_start_matches("http://").trim_end_matches('/');
    // empty allowlist => deny ALL SERVICE
    let config = ServerConfig { query_timeout: Some(std::time::Duration::from_secs(2)), ..ServerConfig::default() };
    let base = spawn_with(config).await;
    let q = format!(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE {{ ?s a ex:Person . SERVICE SILENT <http://{host}/> {{ ?s ex:name ?name }} }}"
    );
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    // SILENT swallows the egress refusal -> identity -> both local persons survive, 200.
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("alice") && body.contains("bob"), "SILENT must keep local bindings: {body}");
}
