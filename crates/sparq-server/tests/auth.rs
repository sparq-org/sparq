//! sq-zcby (PSS gh-46) — HTTP integration tests for the Bearer-token gate on the
//! write surface, plus the optional read gate.
//!
//! Each test spins the real axum server on an ephemeral port and drives it over HTTP with
//! `reqwest`, asserting:
//!   * back-compat — with NO token configured, writes still work;
//!   * a configured write-token gates the UPDATE path (no header / wrong token => 401 with
//!     `WWW-Authenticate: Bearer`; correct `Bearer` token => 204 and the mutation applies,
//!     verified by a follow-up read);
//!   * the Graph-Store-Protocol write methods (PUT a graph) are gated the same way;
//!   * reads stay OPEN with only a write-token, and are GATED (401) once `--auth-token-read`
//!     is on;
//!   * classification keys on "does this mutate", not the route — an UPDATE smuggled through
//!     the query path (POST `application/sparql-query` body that is an update) is gated too.
//!
//! Mirrors QLever's `-a <token>` write gate.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const TOKEN: &str = "s3cr3t-write-token";

/// Boots the server over an empty graph with `config`; returns its base URL.
async fn spawn_with(config: ServerConfig) -> String {
    let graph = Graph::load_str("", "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A config with the write-token set; `read_gate` toggles `--auth-token-read`.
fn token_config(read_gate: bool) -> ServerConfig {
    ServerConfig {
        auth_token: Some(TOKEN.to_string()),
        auth_token_read: read_gate,
        ..ServerConfig::default()
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

const INSERT: &str = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";
const SELECT: &str = "SELECT ?s WHERE { ?s ?p ?o }";

/// Counts solutions of `SELECT` via the JSON result (no auth header — only valid when reads
/// are open).
async fn count_rows_open(cl: &reqwest::Client, base: &str) -> usize {
    let body: serde_json::Value = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["results"]["bindings"].as_array().unwrap().len()
}

/// Counts solutions with a Bearer token (for the read-gated case).
async fn count_rows_authed(cl: &reqwest::Client, base: &str, token: &str) -> usize {
    let body: serde_json::Value = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .header("authorization", format!("Bearer {token}"))
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["results"]["bindings"].as_array().unwrap().len()
}

// ---------------------------------------------------------------------------
// Back-compat: no token configured
// ---------------------------------------------------------------------------

/// With NO token configured, an UPDATE still works (today's behaviour preserved).
#[tokio::test]
async fn update_with_no_token_configured_still_works() {
    let base = spawn_with(ServerConfig::default()).await;
    let cl = client();
    let r = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(INSERT)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "no-token UPDATE must be back-compat 204");
    assert_eq!(
        count_rows_open(&cl, &base).await,
        1,
        "the mutation must have applied"
    );
}

// ---------------------------------------------------------------------------
// Write gate: the UPDATE path
// ---------------------------------------------------------------------------

/// UPDATE with a token configured but NO Authorization header => 401 + WWW-Authenticate.
#[tokio::test]
async fn update_without_header_is_401() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    let r = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(INSERT)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(
        r.headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer"),
        "401 must carry WWW-Authenticate: Bearer"
    );
    // The mutation must NOT have applied (reads are open in write-only mode).
    assert_eq!(
        count_rows_open(&cl, &base).await,
        0,
        "a rejected UPDATE must not mutate"
    );
}

/// UPDATE with the WRONG token => 401 (indistinguishable from the missing-token 401).
#[tokio::test]
async fn update_with_wrong_token_is_401() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    let r = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("authorization", "Bearer not-the-token")
        .body(INSERT)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(
        r.headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer")
    );
    assert_eq!(count_rows_open(&cl, &base).await, 0);
}

/// UPDATE with the CORRECT Bearer token => 204 and the mutation actually applies (verified
/// by a follow-up read). Scheme casing is tolerated.
#[tokio::test]
async fn update_with_correct_token_applies() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    let r = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("authorization", format!("bearer {TOKEN}")) // lowercase scheme on purpose
        .body(INSERT)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "correct token => 204");
    assert_eq!(
        count_rows_open(&cl, &base).await,
        1,
        "the mutation must have applied"
    );
}

// ---------------------------------------------------------------------------
// Write gate: a Graph-Store-Protocol write method (PUT a graph)
// ---------------------------------------------------------------------------

/// A GSP write (PUT a named graph) is gated the same way: no token => 401; correct token =>
/// the graph is written (a follow-up read sees it).
#[tokio::test]
async fn gsp_put_is_gated() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    let g = "http://ex/g/team";
    let ttl = "<http://ex/a> <http://ex/p> <http://ex/b> .";

    // No token => 401.
    let unauth = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .body(ttl)
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401, "GSP PUT must be gated");
    assert_eq!(
        unauth
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer")
    );

    // Correct token => 201 Created.
    let authed = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(ttl)
        .send()
        .await
        .unwrap();
    assert_eq!(
        authed.status(),
        201,
        "authed GSP PUT into an absent graph => 201"
    );

    // Read it back (reads are open in write-only mode) — the write applied.
    let got = cl
        .get(format!("{base}/sparql/graph?graph={g}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        got.contains("<http://ex/a>"),
        "the GSP PUT must have applied: {got:?}"
    );
}

// ---------------------------------------------------------------------------
// Reads: open in write-only mode, gated under --auth-token-read
// ---------------------------------------------------------------------------

/// With only a write-token, reads stay OPEN (no Authorization needed).
#[tokio::test]
async fn reads_open_when_only_write_token_set() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    // Seed via an authed UPDATE, then read with NO header.
    let seed = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(INSERT)
        .send()
        .await
        .unwrap();
    assert_eq!(seed.status(), 204);
    // GET /sparql with no token must succeed.
    let r = cl
        .get(format!("{base}/sparql"))
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "reads must be open with only a write-token"
    );
    assert_eq!(count_rows_open(&cl, &base).await, 1);
    // A GSP read with no token must succeed too.
    let gr = cl
        .get(format!("{base}/sparql/graph?default"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        gr.status(),
        200,
        "GSP reads must be open with only a write-token"
    );
}

/// With --auth-token-read on, reads are GATED: no token => 401, correct token => 200.
#[tokio::test]
async fn reads_gated_when_read_gate_on() {
    let base = spawn_with(token_config(true)).await;
    let cl = client();
    // Seed (authed write).
    cl.post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(INSERT)
        .send()
        .await
        .unwrap();

    // GET /sparql with NO header => 401.
    let unauth = cl
        .get(format!("{base}/sparql"))
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        unauth.status(),
        401,
        "reads must be gated with --auth-token-read"
    );
    assert_eq!(
        unauth
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer")
    );

    // A GSP read with no header => 401 too.
    let gsp_unauth = cl
        .get(format!("{base}/sparql/graph?default"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        gsp_unauth.status(),
        401,
        "GSP reads must be gated with --auth-token-read"
    );

    // With the token => 200 and the data is there.
    assert_eq!(count_rows_authed(&cl, &base, TOKEN).await, 1);
}

/// sq-b3df9 (w3c/sparql-protocol#40): the HTTP `QUERY` method enforces the SAME
/// `--auth-token-read` gate as GET/POST — it is a read on the dataset and must NOT bypass the
/// hot auth path. No header => 401 + `WWW-Authenticate: Bearer`; correct token => 200.
#[tokio::test]
async fn query_method_gated_when_read_gate_on() {
    let base = spawn_with(token_config(true)).await;
    let cl = client();
    let query_method = reqwest::Method::from_bytes(b"QUERY").unwrap();

    // QUERY with NO Authorization header => 401 (the read gate), NOT 200/400 — the auth gate
    // runs before the query handler ever sees the body.
    let unauth = cl
        .request(query_method.clone(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body(SELECT)
        .send()
        .await
        .unwrap();
    assert_eq!(
        unauth.status(),
        401,
        "QUERY must be gated with --auth-token-read"
    );
    assert_eq!(
        unauth
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer")
    );

    // With the Bearer token => 200 (the request reaches the shared query path).
    let authed = cl
        .request(query_method, format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("accept", "application/sparql-results+json")
        .body(SELECT)
        .send()
        .await
        .unwrap();
    assert_eq!(authed.status(), 200, "QUERY with the read token must pass");
}

// ---------------------------------------------------------------------------
// sq-9jrx: /metrics is a READ (it leaks the live triple count and the
// active-subscription count), so it is gated by --auth-token-read like any other
// GET. /health stays ungated for liveness probes.
// ---------------------------------------------------------------------------

/// With only a write-token, `/metrics` stays OPEN (no Authorization needed) — same
/// as any other read in write-only mode.
#[tokio::test]
async fn metrics_open_when_only_write_token_set() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    let r = cl.get(format!("{base}/metrics")).send().await.unwrap();
    assert_eq!(
        r.status(),
        200,
        "/metrics must be open with only a write-token"
    );
    assert!(r.text().await.unwrap().contains("sparq_graph_triples"));
}

/// With `--auth-token-read` on, `/metrics` is GATED: no token => 401 with
/// `WWW-Authenticate: Bearer` (and the gauges are NOT disclosed); correct token => 200.
#[tokio::test]
async fn metrics_gated_when_read_gate_on() {
    let base = spawn_with(token_config(true)).await;
    let cl = client();

    // No header => 401, and the body must NOT contain the leaked gauges.
    let unauth = cl.get(format!("{base}/metrics")).send().await.unwrap();
    assert_eq!(
        unauth.status(),
        401,
        "/metrics must be gated with --auth-token-read"
    );
    assert_eq!(
        unauth
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer"),
        "the /metrics 401 must carry WWW-Authenticate: Bearer"
    );
    assert!(
        !unauth.text().await.unwrap().contains("sparq_graph_triples"),
        "a gated /metrics must not disclose the triple-count gauge"
    );

    // Wrong token => still 401.
    let wrong = cl
        .get(format!("{base}/metrics"))
        .header("authorization", "Bearer not-the-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401, "wrong token on /metrics => 401");

    // Correct token => 200 and the exposition is served.
    let authed = cl
        .get(format!("{base}/metrics"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authed.status(), 200, "correct token => /metrics 200");
    assert!(authed.text().await.unwrap().contains("sparq_graph_triples"));
}

/// `/health` stays UNGATED for liveness probes even with `--auth-token-read` on.
#[tokio::test]
async fn health_open_even_when_read_gate_on() {
    let base = spawn_with(token_config(true)).await;
    let cl = client();
    let r = cl.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(
        r.status(),
        200,
        "/health must stay open for liveness even under --auth-token-read"
    );
    assert_eq!(r.text().await.unwrap(), "ok");
}

// ---------------------------------------------------------------------------
// Mutation classification (not route): UPDATE via the query path
// ---------------------------------------------------------------------------

/// An UPDATE smuggled through the QUERY path (POST with `application/sparql-query` whose body
/// is actually an UPDATE) must be gated as a write — classification keys on the payload, not
/// the Content-Type/route. With no token it would be a 400 (the query handler rejects a
/// non-query); with a token AND no header it must be a 401 BEFORE that, proving the gate
/// classified it as a write.
#[tokio::test]
async fn update_via_query_path_is_gated() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    let r = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query") // the *query* operation route
        .body(INSERT) // ...but the body is an UPDATE
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        401,
        "an UPDATE on the query path must be gated as a write"
    );
    assert_eq!(
        r.headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer")
    );

    // And a genuine READ on the query path stays open (write-only mode), proving the
    // classifier does not over-gate.
    let read = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body(SELECT)
        .send()
        .await
        .unwrap();
    assert_eq!(
        read.status(),
        200,
        "a genuine query must stay open in write-only mode"
    );
}

/// The url-encoded form path: an `update=` form parameter is a write UNCONDITIONALLY (it IS
/// an update by SPARQL-Protocol definition; see `update_form_with_select_body_is_gated_as_write`
/// for the bypass guard), and an update string smuggled through `query=` is gated too.
#[tokio::test]
async fn update_via_form_path_is_gated() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    // `update=<INSERT…>` with no token => 401 (gated as a write).
    let r = cl
        .post(format!("{base}/sparql"))
        .form(&[("update", INSERT)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        401,
        "an `update=` form must be gated as a write"
    );

    // A genuine `query=` form stays open.
    let read = cl
        .post(format!("{base}/sparql"))
        .form(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        read.status(),
        200,
        "a `query=` form read must stay open in write-only mode"
    );
}

/// sq-zcby (Copilot PR#71 SECURITY regression test): the auth-bypass fix.
///
/// An `update=` form field IS an update operation BY DEFINITION (SPARQL 1.1 Protocol §2.2),
/// so it MUST be gated as a write UNCONDITIONALLY — even when its value is a well-formed
/// read-only query (e.g. `update=SELECT…`). The old classifier ran `payload_mutates(u)` on
/// the `update=` value, which returned `false` for a SELECT-looking body and classified the
/// request as a READ, so the write gate was BYPASSED. After the fix this request must be 401
/// without a (correct) token, exactly like any other `update=` form.
#[tokio::test]
async fn update_form_with_select_body_is_gated_as_write() {
    let base = spawn_with(token_config(false)).await;
    let cl = client();
    // `update=<SELECT…>` (a SELECT-looking body in the update field) with NO token => 401.
    // This is the bypass: pre-fix it was classified as a read and allowed through.
    let r = cl
        .post(format!("{base}/sparql"))
        .form(&[("update", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        401,
        "an `update=` form with a SELECT-looking body must still be gated as a write (no bypass)"
    );
    assert_eq!(
        r.headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer"),
        "the bypass-fix 401 must carry WWW-Authenticate: Bearer"
    );

    // With the WRONG token it must still be 401 (the gate ran, not bypassed).
    let wrong = cl
        .post(format!("{base}/sparql"))
        .header("authorization", "Bearer not-the-token")
        .form(&[("update", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        wrong.status(),
        401,
        "wrong token on an `update=SELECT…` form must still be 401"
    );
}
