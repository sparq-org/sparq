//! sq-toze.36 (cert gap GX-13) — end-to-end test of the in-binary container
//! HEALTHCHECK probe. The Dockerfile's `HEALTHCHECK` runs `sparq-server --health-probe`,
//! which opens a TCP connection to the loopback `/health` and exits 0/non-zero. This boots
//! the REAL server on an ephemeral port and drives `health_probe::run_probe` against it
//! (healthy), then against a closed port (unhealthy) — the two outcomes the container
//! runtime maps to healthy/unhealthy.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::health_probe::run_probe;
use sparq_server::{router, AppState};
use tokio::net::TcpListener;

/// Boot the server on a random loopback port; return its `host:port` (no scheme — that is
/// exactly what `run_probe` takes, since it dials a raw `TcpStream`).
async fn spawn() -> String {
    let graph = Graph::load_str("", "turtle").unwrap();
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr.to_string()
}

#[tokio::test]
async fn probe_passes_against_a_live_server() {
    let addr = spawn().await;
    // The live server answers GET /health with 200 "ok" -> healthy -> Ok(()).
    let res = run_probe(&addr).await;
    assert!(
        res.is_ok(),
        "probe should pass against a live server: {res:?}"
    );
}

#[tokio::test]
async fn probe_fails_when_nothing_is_listening() {
    // Bind a port, capture it, then drop the listener so the port is (almost certainly)
    // free again — a connect to it refuses, which the probe maps to Err (unhealthy).
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    let res = run_probe(&addr).await;
    assert!(
        res.is_err(),
        "probe should fail when no server is listening: {res:?}"
    );
}

#[tokio::test]
async fn probe_fails_against_a_non_http_listener() {
    // A raw TCP listener that accepts the connection but never speaks HTTP: the probe's
    // bounded read closes (peer-side) or times out, and the empty/garbage response is
    // classified unhealthy. Proves the probe does not hang or falsely report healthy.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        // Accept one connection and immediately drop it (closes the socket -> EOF read).
        if let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    let res = run_probe(&addr).await;
    assert!(
        res.is_err(),
        "probe should fail against a non-HTTP listener: {res:?}"
    );
}
