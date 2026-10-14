//! sq-o7o0 (ASVS V14.5.3) — first-party CORS origin allowlist, integration.
//!
//! Boots the real hardened router and asserts the END-TO-END CORS contract:
//!   * DEFAULT (empty allowlist) emits NO CORS headers — the historical, safe posture for
//!     a data API (a cross-origin browser read is blocked by the Same-Origin Policy);
//!   * a configured first-party origin is REFLECTED into `Access-Control-Allow-Origin`
//!     (never `*`, never `Access-Control-Allow-Credentials`) with `Vary: Origin`;
//!   * an UN-listed origin gets NO CORS header (so the browser still blocks it);
//!   * the `OPTIONS` preflight is answered (`204` + methods/headers/max-age) for an
//!     allowlisted origin only;
//!   * allowlisting an origin does NOT relax the existing auth gate.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState, CorsAllowlist, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob .
"#;

const APP_ORIGIN: &str = "https://app.example.org";
const EVIL_ORIGIN: &str = "https://evil.example.org";

async fn serve(app: axum::Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_with(config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    serve(router(AppState::with_config(graph, config))).await
}

fn cors_config(origins: &[&str]) -> ServerConfig {
    let mut cors = CorsAllowlist::default();
    for o in origins {
        cors.add(o).unwrap();
    }
    ServerConfig { cors_allow: cors, ..ServerConfig::default() }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

const ACAO: &str = "access-control-allow-origin";
const ACAC: &str = "access-control-allow-credentials";

// ---------------------------------------------------------------------------
// (1) DEFAULT: no allowlist => no CORS headers (back-compat / safe default).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_emits_no_cors_headers_even_with_origin() {
    let base = spawn_with(ServerConfig::default()).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
        .header("origin", APP_ORIGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // The whole point of the safe default: a browser cross-origin read is blocked because
    // there is NO Access-Control-Allow-Origin header.
    assert!(resp.headers().get(ACAO).is_none(), "default must emit no ACAO header");
    assert!(resp.headers().get(ACAC).is_none());
}

#[tokio::test]
async fn default_preflight_options_has_no_cors_headers() {
    // With no allowlist the CORS layer is not installed; an OPTIONS just 405s (the
    // /sparql route allows GET/HEAD/POST) and carries no CORS headers.
    let base = spawn_with(ServerConfig::default()).await;
    let resp = client()
        .request(reqwest::Method::OPTIONS, format!("{base}/sparql"))
        .header("origin", APP_ORIGIN)
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();
    assert!(resp.headers().get(ACAO).is_none());
}

// ---------------------------------------------------------------------------
// (2) Allowlisted origin is reflected (never *), with Vary: Origin, no credentials.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn allowlisted_origin_is_reflected_on_actual_request() {
    let base = spawn_with(cors_config(&[APP_ORIGIN])).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
        .header("origin", APP_ORIGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let acao = resp.headers().get(ACAO).expect("ACAO present for allowlisted origin");
    // Exact reflection — never the wildcard `*`.
    assert_eq!(acao.to_str().unwrap(), APP_ORIGIN);
    assert_ne!(acao.to_str().unwrap(), "*");
    // Never opt into credentialed CORS.
    assert!(resp.headers().get(ACAC).is_none());
    // Vary: Origin so a shared cache keys on the origin.
    let vary = resp
        .headers()
        .get_all("vary")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.eq_ignore_ascii_case("origin"));
    assert!(vary, "Vary: Origin must be present");
}

// ---------------------------------------------------------------------------
// (3) Un-listed origin gets NO CORS header (browser blocks the read).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unlisted_origin_gets_no_cors_header() {
    let base = spawn_with(cors_config(&[APP_ORIGIN])).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
        .header("origin", EVIL_ORIGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200); // the request itself succeeds...
    // ...but the browser cannot read it: no ACAO header at all.
    assert!(resp.headers().get(ACAO).is_none(), "un-listed origin must get no ACAO");
}

#[tokio::test]
async fn no_origin_header_means_no_cors_header() {
    let base = spawn_with(cors_config(&[APP_ORIGIN])).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers().get(ACAO).is_none());
}

// ---------------------------------------------------------------------------
// (4) Preflight OPTIONS — answered for an allowlisted origin only.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn preflight_answered_for_allowlisted_origin() {
    let base = spawn_with(cors_config(&[APP_ORIGIN])).await;
    let resp = client()
        .request(reqwest::Method::OPTIONS, format!("{base}/sparql"))
        .header("origin", APP_ORIGIN)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    assert_eq!(resp.headers().get(ACAO).unwrap().to_str().unwrap(), APP_ORIGIN);
    let methods = resp
        .headers()
        .get("access-control-allow-methods")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(methods.contains("POST"), "preflight advertises POST: {methods}");
    assert!(methods.contains("GET"), "preflight advertises GET: {methods}");
    // The requested headers are echoed.
    let allow_headers = resp
        .headers()
        .get("access-control-allow-headers")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(allow_headers.to_ascii_lowercase().contains("content-type"));
    assert!(resp.headers().get("access-control-max-age").is_some());
    assert!(resp.headers().get(ACAC).is_none());
}

#[tokio::test]
async fn preflight_for_unlisted_origin_has_no_allow_origin() {
    let base = spawn_with(cors_config(&[APP_ORIGIN])).await;
    let resp = client()
        .request(reqwest::Method::OPTIONS, format!("{base}/sparql"))
        .header("origin", EVIL_ORIGIN)
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();
    // A 204 with NO Access-Control-Allow-Origin makes the browser fail the preflight.
    assert!(resp.headers().get(ACAO).is_none());
}

#[tokio::test]
async fn second_origin_in_list_also_allowed() {
    let other = "http://localhost:5173";
    let base = spawn_with(cors_config(&[APP_ORIGIN, other])).await;
    for origin in [APP_ORIGIN, other] {
        let resp = client()
            .get(format!("{base}/sparql"))
            .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
            .header("origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.headers().get(ACAO).unwrap().to_str().unwrap(), origin);
    }
}

// ---------------------------------------------------------------------------
// (5) CORS does NOT relax auth — an allowlisted browser still needs the token.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cors_does_not_bypass_write_auth() {
    let mut config = cors_config(&[APP_ORIGIN]);
    config.auth_token = Some("s3cret".to_string());
    let base = spawn_with(config).await;

    // An UPDATE from the allowlisted origin WITHOUT the token is still refused 401 —
    // CORS is a browser-read gate, not an authorization bypass.
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("origin", APP_ORIGIN)
        .body("INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "CORS must not relax the write-auth gate");
    // The CORS header is still stamped on the 401 (it is a response like any other),
    // so a browser can read the 401 — but the write did not happen.
    assert_eq!(resp.headers().get(ACAO).unwrap().to_str().unwrap(), APP_ORIGIN);
}
