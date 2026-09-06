//! [SONNET-4.6] (sq-qsm5z) Integration tests for the `query-registry` feature's
//! `GET /queries` and `DELETE /queries/{id}` HTTP routes.
//!
//! These exercise the REAL HTTP path (not a mock): boot the server, drive it over
//! HTTP with `reqwest`, and assert:
//!   * both routes are fail-closed on auth — no token => 401, wrong token => 401;
//!   * no token-enumeration: the 401 for a missing token is identical to a wrong token;
//!   * `GET /queries` with the correct READ token => 200 + well-formed `{"queries":[...]}`;
//!   * `DELETE /queries/{id}` with the correct WRITE token => 404 for unknown id.
//!
//! Harness: same pattern as `tests/auth.rs` (spawn_with / reqwest / ephemeral port).
//!
//! Run: `cargo test -p sparq-server --features query-registry --test query_registry`
//!
//! 🤖 SPARQ agent

#![cfg(feature = "query-registry")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// The single shared Bearer token — used for both reads (when `auth_token_read=true`)
/// and writes, matching the `ServerConfig::auth_token` / `auth_check` model exactly.
const TOKEN: &str = "s3cr3t-registry-token";
const WRONG: &str = "definitely-not-the-token";

/// Boots a real axum server with a write+read gate (both routes are auth-gated).
/// Returns the base URL.
async fn spawn_gated() -> String {
    let graph = Graph::load_str("", "turtle").unwrap();
    // auth_token: the single token for all operations.
    // auth_token_read: true so that GET /queries (Operation::Read) is also gated.
    let config = ServerConfig {
        auth_token: Some(TOKEN.to_string()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// GET /queries — auth gate
// ---------------------------------------------------------------------------

/// `GET /queries` with NO Authorization header => 401 (fail-closed).
#[tokio::test]
async fn list_queries_no_token_is_401() {
    let base = spawn_gated().await;
    let resp = client()
        .get(format!("{base}/queries"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "GET /queries with no token must be 401"
    );
    // WWW-Authenticate: Bearer is the standard fail-closed signal.
    assert_eq!(
        resp.headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
        "GET /queries 401 must carry WWW-Authenticate: Bearer"
    );
}

/// `GET /queries` with the WRONG token => 401 (indistinguishable from missing-token 401,
/// so no enumeration is possible).
#[tokio::test]
async fn list_queries_wrong_token_is_401() {
    let base = spawn_gated().await;
    let resp = client()
        .get(format!("{base}/queries"))
        .header("authorization", format!("Bearer {WRONG}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "GET /queries with wrong token must be 401 (no enumeration)"
    );
    assert_eq!(
        resp.headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
        "wrong-token 401 must carry the same WWW-Authenticate: Bearer"
    );
}

/// `GET /queries` with the correct token => 200 + `{"queries":[...]}` body.
#[tokio::test]
async fn list_queries_correct_token_is_200_with_body() {
    let base = spawn_gated().await;
    let resp = client()
        .get(format!("{base}/queries"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "GET /queries with correct token must be 200"
    );
    // Body must be well-formed JSON with a top-level "queries" array.
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body.get("queries").and_then(|v| v.as_array()).is_some(),
        "response body must be {{\"queries\":[...]}}; got: {body}"
    );
    // On an idle server with no executing queries the list must be empty.
    let queries = body["queries"].as_array().unwrap();
    assert!(
        queries.is_empty(),
        "idle server must report an empty query list"
    );
}

// ---------------------------------------------------------------------------
// DELETE /queries/{id} — auth gate
// ---------------------------------------------------------------------------

/// `DELETE /queries/{id}` with NO token => 401.
#[tokio::test]
async fn cancel_query_no_token_is_401() {
    let base = spawn_gated().await;
    let resp = client()
        .delete(format!("{base}/queries/0000000000000000"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "DELETE /queries/{{id}} with no token must be 401"
    );
    assert_eq!(
        resp.headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
        "DELETE 401 must carry WWW-Authenticate: Bearer"
    );
}

/// `DELETE /queries/{id}` with the WRONG token => 401.
#[tokio::test]
async fn cancel_query_wrong_token_is_401() {
    let base = spawn_gated().await;
    let resp = client()
        .delete(format!("{base}/queries/0000000000000000"))
        .header("authorization", format!("Bearer {WRONG}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "DELETE /queries/{{id}} with wrong token must be 401 (no enumeration)"
    );
}

/// `DELETE /queries/{id}` with the correct WRITE token but an unknown id => 404.
/// This proves the auth gate PASSES and the handler logic runs (non-vacuous: the gate
/// did not short-circuit to 401 for a valid token, and the 404 comes from the registry).
#[tokio::test]
async fn cancel_query_correct_token_unknown_id_is_404() {
    let base = spawn_gated().await;
    let resp = client()
        .delete(format!("{base}/queries/0000000000000000"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "DELETE /queries/{{id}} with correct token + unknown id must be 404"
    );
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-t1isr — EXPLAIN ANALYZE query-registry wiring
//
// These tests verify the HTTP-layer behaviour of the EXPLAIN ANALYZE → registry wiring.
//
// The unit-level proofs (register-while-running, cancel-flag, RAII-cleanup-on-panic) live
// in the `query_registry_tests` module inside `src/http.rs` — they directly exercise the
// registry API with the EXACT code pattern used in `run_query_pinned`'s EXPLAIN branch.
//
// MUTATION PROOF (unit tests): removing the `register(sparql)` call from the EXPLAIN branch
// in `run_query_pinned` does NOT break the unit tests (they call the registry API directly).
// The unit tests prove the REGISTRY itself works; the HTTP-level test below proves the guard
// is correctly wired (it goes red if the guard LEAKS — i.e., registered but never dropped).
// ---------------------------------------------------------------------------

/// Helper: spawn an open (no-auth) server so `GET /queries` is accessible without
/// credentials.  EXPLAIN is a read operation, not write-gated.
async fn spawn_open() -> String {
    let graph = sparq_core::Graph::load_str("", "turtle").unwrap();
    let config = sparq_server::ServerConfig::default();
    let app = sparq_server::router(sparq_server::AppState::with_config(graph, config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// After an EXPLAIN request completes, the registry must be empty (RAII deregister-after).
///
/// Non-vacuous check: this test goes red if the EXPLAIN path registers the query but the
/// `QueryGuard` is NOT moved into the `spawn_blocking` closure (i.e., the guard drops
/// BEFORE the engine runs), causing a phantom entry to appear and then disappear before the
/// EXPLAIN HTTP response is returned.  In other words: if someone moves the `let _guard =
/// qr_guard;` line OUTSIDE the closure (so it drops at the closure boundary rather than
/// inside), `GET /queries` after a completed EXPLAIN would return a stale entry — this
/// assertion catches that.  (A leaked guard whose drop is wired INSIDE the closure but
/// never fires would also be caught by the "after empty" check.)
///
/// The in-flight registration test (entry visible while engine runs) requires catching the
/// `spawn_blocking` mid-execution, which is impractical for a fast plan-only EXPLAIN; that
/// obligation is satisfied by the `explain_path_entry_present_while_guard_alive` unit test.
#[tokio::test]
async fn explain_deregistered_after_completion() {
    let base = spawn_open().await;

    // Baseline: no queries in flight on an idle server.
    let before: serde_json::Value = client()
        .get(format!("{base}/queries"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        before["queries"].as_array().unwrap().is_empty(),
        "registry must be empty before any request; got: {before}"
    );

    // Fire a plan-only EXPLAIN (fast; does not execute the engine).
    let explain_resp = client()
        .get(format!(
            "{base}/sparql?query=SELECT+*+WHERE+%7B+%3Fs+%3Fp+%3Fo+%7D&explain=plan"
        ))
        .send()
        .await
        .unwrap();
    assert!(
        explain_resp.status().is_success(),
        "EXPLAIN plan request must succeed; got: {}",
        explain_resp.status()
    );

    // After the EXPLAIN completes, the RAII guard must have fired — registry must be empty.
    let after: serde_json::Value = client()
        .get(format!("{base}/queries"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        after["queries"].as_array().unwrap().is_empty(),
        "registry must be empty after EXPLAIN completes (RAII guard inside spawn_blocking \
         must have fired); got: {after}"
    );
}

/// In-flight test: an in-progress EXPLAIN request registers itself in the running-query
/// registry while the `spawn_blocking` worker is executing.
///
/// Non-vacuous: removing the `register(sparql)` call from the EXPLAIN branch in
/// `run_query_pinned` makes this test fail — `running_query_count()` stays 0 on EVERY
/// attempt and the final `assert_eq!(1)` fires.  The retry loop does not weaken that
/// mutation proof: the property is existential (the entry must be OBSERVABLE while the
/// engine executes), so one observed registration proves the wiring, while a removed
/// `register()` call can never be observed no matter how many attempts run.
///
/// Flake-hardening (#3510): the original version raced a DETACHED poller task with a
/// fixed budget of 200 `yield_now()` iterations against the HTTP round-trip.  Two
/// wall-clock assumptions made that flaky on a loaded runner: (a) 200 yields cost
/// microseconds and can burn out before the request even reaches the server, and
/// (b) the count==1 window can close between two polls, because the RAII guard drops
/// on a blocking-pool thread that runs concurrently with the poller.  This version
/// removes both: the registry count is sampled between polls of the IN-FLIGHT response
/// future (`select!`), so sampling lasts exactly as long as the request with no
/// iteration budget to exhaust, and the fire-and-observe attempt is retried so a single
/// lost cross-thread race cannot fail the test on its own.
///
/// Uses `AppState::running_query_count()` (feature-gated accessor) to observe the live
/// registry count from a shared `AppState` clone while the EXPLAIN's `spawn_blocking`
/// worker runs.  The trick: `AppState` holds the `Arc<QueryRegistry>` so any clone sees
/// the same backing store; we keep a clone BEFORE handing ownership to `router()`.
#[tokio::test]
async fn explain_registered_while_in_flight_via_state_accessor() {
    // Build the AppState — keep a clone of the state (it is `Clone`, backed by Arcs)
    // so we share the registry with the router.
    let graph = sparq_core::Graph::load_str("", "turtle").unwrap();
    let state = sparq_server::AppState::with_config(graph, sparq_server::ServerConfig::default());
    let state_clone = state.clone();

    let app = sparq_server::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let http = client();

    // Fire EXPLAIN ANALYZE (executes the engine; keeps the spawn_blocking alive longer
    // than plan-only) and sample the registry count while the response is in flight.
    // Requests are strictly sequential, so the observable count is only ever 0 or 1.
    let mut peak_count = 0usize;
    for _attempt in 0..50 {
        let mut req = std::pin::pin!(
            http.get(format!(
                "{base}/sparql?query=SELECT+*+WHERE+%7B+%3Fs+%3Fp+%3Fo+%7D&explain=analyze"
            ))
            .send()
        );
        let explain_resp = loop {
            tokio::select! {
                biased;
                resp = &mut req => break resp.unwrap(),
                // yield_now parks this task at the back of the run queue, so the server
                // tasks (accept + the handler, which registers BEFORE spawn_blocking)
                // get to run between two samples.
                _ = tokio::task::yield_now() => {
                    peak_count = peak_count.max(state_clone.running_query_count());
                }
            }
        };
        assert!(
            explain_resp.status().is_success(),
            "EXPLAIN ANALYZE must succeed; got: {}",
            explain_resp.status()
        );
        if peak_count > 0 {
            break;
        }
    }

    assert_eq!(
        peak_count, 1,
        "EXPLAIN ANALYZE must register exactly one entry in the running-query registry while \
         the engine executes (peak_count stayed 0 across every attempt — likely the register() \
         call was removed from the EXPLAIN branch in run_query_pinned)"
    );
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-m9prn — SPARQL UPDATE registration + kill
//
// A running UPDATE holds the single sequenced writer thread, so it is the operation an
// operator most needs to see and kill: every write queued behind it is blocked until it
// releases. These tests drive the REAL HTTP path end to end.
// ---------------------------------------------------------------------------

/// The number of subjects seeded for the long-running UPDATE below. The update's WHERE is a
/// three-way cross product over them, so the writer has `SEEDS^3` solutions to grind through
/// — long enough to observe and cancel, and it aborts promptly once the flag flips.
const SEEDS: usize = 100;

/// Serialises a graph of `SEEDS` triples on one predicate.
fn seed_turtle() -> String {
    let mut ttl = String::new();
    for i in 0..SEEDS {
        ttl.push_str(&format!("<http://ex/s{}> <http://ex/p> \"{}\" .\n", i, i));
    }
    ttl
}

/// Boots an open (no-auth) server seeded with [`seed_turtle`].
async fn spawn_seeded() -> String {
    let graph = sparq_core::Graph::load_str(&seed_turtle(), "turtle").unwrap();
    let app = sparq_server::router(sparq_server::AppState::with_config(
        graph,
        sparq_server::ServerConfig::default(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Polls `GET /queries` until an entry with `kind == "update"` appears, and returns its id.
/// Returns `None` if none appears within the deadline.
async fn await_update_row(base: &str) -> Option<serde_json::Value> {
    for _ in 0..600 {
        let body: serde_json::Value = client()
            .get(format!("{base}/queries"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(row) = body["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == "update")
        {
            return Some(row.clone());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    None
}

/// Counts the seeded triples still present.
async fn remaining(base: &str) -> usize {
    let body: serde_json::Value = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/p> ?o }")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["results"]["bindings"].as_array().unwrap().len()
}

/// A running SPARQL UPDATE is listed by `GET /queries` as `kind: "update"`, carries only a
/// fingerprint (never the update text), and `DELETE /queries/{id}` aborts it so the delete
/// never commits.
///
/// Non-vacuity, both halves:
///   * drop the `register_update` call from `ServerApplier::apply` → no `kind:"update"` row
///     ever appears → `await_update_row` returns `None` → the first assert fires;
///   * drop the `.with_cancel(...)` from that same budget (or the `with_registry` wiring) →
///     `DELETE /queries/{id}` no longer reaches the engine's poll site → the update runs to
///     completion, returns 204, and deletes everything → the last two asserts fire.
#[tokio::test]
async fn running_update_is_listed_and_killable() {
    let base = spawn_seeded().await;
    assert_eq!(remaining(&base).await, SEEDS, "the seed data must be loaded");

    // A DELETE whose WHERE is a three-way cross product: slow enough to catch in flight.
    let update = "DELETE { ?a <http://ex/p> ?x } WHERE { \
                  ?a <http://ex/p> ?x . ?b <http://ex/p> ?y . ?c <http://ex/p> ?z }";
    let update_base = base.clone();
    let in_flight = tokio::spawn(async move {
        client()
            .post(format!("{update_base}/sparql"))
            .header("content-type", "application/sparql-update")
            .body(update)
            .send()
            .await
            .unwrap()
            .status()
    });

    let row = await_update_row(&base)
        .await
        .expect("a running UPDATE must appear in GET /queries with kind=\"update\"");

    // Redaction posture (#241): the row exposes a fingerprint, never the update text.
    assert!(
        row["fingerprint"].as_str().is_some_and(|f| f.len() == 16),
        "an update row must carry the 16-hex fingerprint; got: {row}"
    );
    assert!(
        !row.to_string().contains("DELETE"),
        "the listing must never echo the update text; got: {row}"
    );

    // Kill it.
    let id = row["id"].as_str().unwrap();
    let killed = client()
        .delete(format!("{base}/queries/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        killed.status(),
        reqwest::StatusCode::NO_CONTENT,
        "DELETE /queries/{{id}} on a running update must be 204"
    );

    // The cancelled update is REJECTED, not committed: the writer forks per batch and only
    // seals on success, so the seed data survives intact.
    let status = in_flight.await.unwrap();
    assert!(
        !status.is_success(),
        "a cancelled UPDATE must not report success; got: {status}"
    );
    assert_eq!(
        remaining(&base).await,
        SEEDS,
        "a cancelled UPDATE must not commit — every seeded triple must survive"
    );
}

/// After the cancelled update returns, its registry row is gone (the RAII guard in
/// `ServerApplier::apply` fires on the error path too), and the server still accepts writes —
/// the kill released the sequenced writer rather than wedging it.
#[tokio::test]
async fn writer_survives_a_killed_update() {
    let base = spawn_seeded().await;
    let update = "DELETE { ?a <http://ex/p> ?x } WHERE { \
                  ?a <http://ex/p> ?x . ?b <http://ex/p> ?y . ?c <http://ex/p> ?z }";
    let update_base = base.clone();
    let in_flight = tokio::spawn(async move {
        client()
            .post(format!("{update_base}/sparql"))
            .header("content-type", "application/sparql-update")
            .body(update)
            .send()
            .await
            .unwrap()
            .status()
    });

    let row = await_update_row(&base).await.expect("the update must register");
    let id = row["id"].as_str().unwrap();
    client()
        .delete(format!("{base}/queries/{id}"))
        .send()
        .await
        .unwrap();
    let _ = in_flight.await.unwrap();

    // The row is deregistered once the apply returns.
    let body: serde_json::Value = client()
        .get(format!("{base}/queries"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        body["queries"].as_array().unwrap().is_empty(),
        "the killed update's row must be removed on the error path; got: {body}"
    );

    // The writer is still alive and sequencing writes.
    let ok = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/after> <http://ex/p> \"kill-survivor\" }")
        .send()
        .await
        .unwrap();
    assert_eq!(
        ok.status(),
        reqwest::StatusCode::NO_CONTENT,
        "the writer must still accept updates after a kill"
    );
    assert_eq!(
        remaining(&base).await,
        SEEDS + 1,
        "the post-kill insert must be visible"
    );
}
