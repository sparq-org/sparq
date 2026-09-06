//! [SONNET-4.6] (sq-7d3dj.26) **Mid-stream truncation-safety for the pull-streaming SELECT
//! body** — Wave D child D3 of `research/wave-d-pull-streaming-response-body.md` §6.
//!
//! Once the first byte of a `200 OK` streamed body is on the wire the status is COMMITTED: a
//! mid-stream budget abort can only truncate the body, never retract it into a 413/503. The
//! load-bearing invariant is therefore:
//!
//! > A client MUST NOT be able to mistake a truncated stream for a complete result.
//!
//! These tests force a REAL mid-stream abort — a `--max-results` row cap that the engine's
//! streaming single-pattern scan only trips after it has already flushed several 64 KiB chunks
//! to the socket — and assert both enforcement mechanisms:
//!
//! 1. the received body is NOT valid `sparql-results+json` (the document-closing `]}}` is
//!    never written), so any conformant parser errors rather than silently accepting a short
//!    result — the floor guarantee, correct-by-construction; and
//! 2. the truncation is reported out of band: an `X-Sparq-Truncated: max-rows` trailer for a
//!    client that negotiated `TE: trailers`, and an aborted chunked framing (no terminating
//!    zero-length chunk) for one that did not.
//!
//! The FORBIDDEN outcome — a well-formed, correctly-terminated SHORT `200` — is asserted
//! against directly. This matters because the engine's scan path DOES append `]}}` when its
//! cooperative budget check breaks the loop, and only its caller then converts the sticky flag
//! into an error: without the server-side hold-back the wire would carry exactly that.
//!
//! `StreamingJsonBody`'s own state machine (including the panicking-worker case, which needs a
//! dead channel rather than a live engine) is unit-tested in `src/http.rs`. 🤖 SPARQ agent.

#![cfg(feature = "server")]

use http_body::Body as _;
use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;
use tower::ServiceExt;

/// Rows kept before the cap trips. The engine checks its cooperative budget every 1024 scanned
/// rows, so the abort lands at row 4096 — far past the first 64 KiB flush, which is what makes
/// this a genuine MID-STREAM truncation rather than a pre-first-byte refusal.
const MAX_RESULTS: usize = 4000;

/// Enough triples that the serialised result runs to many chunks before the cap trips.
const TRIPLES: usize = 20_000;

const QUERY: &str = "SELECT * WHERE { ?s ?p ?o }";

fn big_graph() -> Graph {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..TRIPLES {
        // ~175 bytes of SPARQL-results JSON per row, so 4096 rows is ~11 chunks.
        ttl.push_str(&format!(
            "ex:s{} ex:p \"value-{}-padding-padding-padding\" .\n",
            i, i
        ));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

fn capped_config() -> ServerConfig {
    ServerConfig { max_results: Some(MAX_RESULTS), ..ServerConfig::default() }
}

/// A drained streamed response body.
struct Drained {
    bytes: Vec<u8>,
    trailers: Option<axum::http::HeaderMap>,
    /// The body failed a frame — over a real socket this aborts the chunked stream without its
    /// terminating zero-length chunk, so the client sees a transport error.
    failed: bool,
}

async fn drain(body: axum::body::Body) -> Drained {
    let mut body = Box::pin(body);
    let mut out = Drained { bytes: Vec::new(), trailers: None, failed: false };
    loop {
        let frame = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
        match frame {
            None => return out,
            Some(Err(_)) => {
                out.failed = true;
                return out;
            }
            Some(Ok(frame)) => match frame.into_data() {
                Ok(data) => out.bytes.extend_from_slice(&data),
                Err(frame) => out.trailers = frame.into_trailers().ok(),
            },
        }
    }
}

/// Runs `QUERY` against the capped server through the REAL router, with or without a
/// `TE: trailers` negotiation, and drains the response.
async fn truncated_response(te_trailers: bool) -> (axum::http::response::Parts, Drained) {
    let app = router(AppState::with_config(big_graph(), capped_config()));
    let mut request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/sparql")
        .header(axum::http::header::CONTENT_TYPE, "application/sparql-query")
        .header(axum::http::header::ACCEPT, "application/sparql-results+json");
    if te_trailers {
        request = request.header(axum::http::header::TE, "trailers");
    }
    let response = app
        .oneshot(request.body(axum::body::Body::from(QUERY)).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (parts, drain(body).await)
}

/// THE LOAD-BEARING TEST. A forced mid-stream row-cap abort yields a body that fails JSON
/// parsing at the closing brace AND carries the truncation trailer — never a clean short 200.
#[tokio::test]
async fn a_mid_stream_row_cap_abort_truncates_and_says_so() {
    let (parts, out) = truncated_response(true).await;

    // The status was committed before the abort was detected — it cannot become a 413.
    assert_eq!(parts.status, axum::http::StatusCode::OK);
    // Streamed, so the length is unknown up front.
    assert!(
        parts.headers.get(axum::http::header::CONTENT_LENGTH).is_none(),
        "a streamed body must not declare a Content-Length"
    );
    // RFC 9110 §6.6.1: the trailer fields that may follow are announced.
    let announced = parts
        .headers
        .get(axum::http::header::TRAILER)
        .expect("a trailers-capable client must be told which trailers to expect")
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(announced.contains("x-sparq-truncated"), "announced: {}", announced);

    // Mechanism 1 — the document is NEVER closed on a truncation.
    assert!(!out.bytes.is_empty(), "the truncation must land mid-stream, not pre-first-byte");
    assert!(
        out.bytes.len() > 64 * 1024,
        "expected a genuinely multi-chunk stream before the cap, got {} bytes",
        out.bytes.len()
    );
    assert!(
        !out.bytes.ends_with(b"]}}"),
        "FORBIDDEN: a truncated stream must not close the JSON document"
    );
    serde_json::from_slice::<serde_json::Value>(&out.bytes)
        .expect_err("FORBIDDEN: a truncated stream must not parse as a complete result");

    // Mechanism 2 — the reason is reported, and completeness is never claimed.
    let trailers = out.trailers.expect("a truncated stream must carry a trailer");
    assert_eq!(
        trailers.get("x-sparq-truncated").map(|v| v.to_str().unwrap()),
        Some("max-rows")
    );
    assert!(
        trailers.get("x-sparq-complete").is_none(),
        "a truncated stream must never claim completeness"
    );
}

/// The same abort for a client that did NOT negotiate trailers: hyper would drop a trailers
/// frame, so the body fails the frame instead (aborting the chunked framing). The document is
/// still never closed.
#[tokio::test]
async fn a_client_without_te_trailers_gets_an_aborted_framing() {
    let (parts, out) = truncated_response(false).await;

    assert_eq!(parts.status, axum::http::StatusCode::OK);
    assert!(
        parts.headers.get(axum::http::header::TRAILER).is_none(),
        "trailers must not be announced to a client that cannot read them"
    );
    assert!(out.failed, "the aborted framing IS the truncation signal here");
    assert!(out.trailers.is_none());
    assert!(!out.bytes.ends_with(b"]}}"));
    serde_json::from_slice::<serde_json::Value>(&out.bytes)
        .expect_err("FORBIDDEN: a truncated stream must not parse as a complete result");
}

/// Control (anti-vacuity): the SAME query with no cap streams to completion — the closing
/// `]}}` IS written, the body parses, and the trailer asserts completeness. Without this the
/// tests above would pass on a server that simply never returned a usable body.
#[tokio::test]
async fn an_uncapped_stream_completes_and_claims_completeness() {
    let app = router(AppState::with_config(big_graph(), ServerConfig::default()));
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/sparql")
        .header(axum::http::header::CONTENT_TYPE, "application/sparql-query")
        .header(axum::http::header::ACCEPT, "application/sparql-results+json")
        .header(axum::http::header::TE, "trailers")
        .body(axum::body::Body::from(QUERY))
        .unwrap();
    let (parts, out) = {
        let response = app.oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        (parts, drain(body).await)
    };

    assert_eq!(parts.status, axum::http::StatusCode::OK);
    assert!(!out.failed);
    assert!(out.bytes.ends_with(b"]}}"), "a complete stream must close the document");
    let parsed: serde_json::Value = serde_json::from_slice(&out.bytes).expect("valid JSON");
    assert_eq!(
        parsed["results"]["bindings"].as_array().map(Vec::len),
        Some(TRIPLES),
        "the complete stream must carry every row"
    );
    let trailers = out.trailers.expect("a complete stream carries the completeness trailer");
    assert_eq!(
        trailers.get("x-sparq-complete").map(|v| v.to_str().unwrap()),
        Some("true")
    );
    assert!(trailers.get("x-sparq-truncated").is_none());
}

/// The floor guarantee over a REAL socket, for a stock HTTP client: a mid-stream truncation
/// can never hand back a parseable complete result. Either the transport errors (the chunked
/// stream ended without its terminating zero-length chunk) or the bytes that did arrive are
/// unparseable — never a clean short 200.
#[tokio::test]
async fn over_a_real_socket_a_truncated_stream_is_never_a_usable_result() {
    let app = router(AppState::with_config(big_graph(), capped_config()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let response = reqwest::Client::new()
        .post(format!("http://{}/sparql", addr))
        .header("content-type", "application/sparql-query")
        .header("accept", "application/sparql-results+json")
        .body(QUERY)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response.content_length().is_none(), "streamed: no Content-Length");

    match response.bytes().await {
        // The chunked stream aborted — the client sees a transport error. Correct.
        Err(_) => {}
        // Some bytes arrived; they must not form a complete result.
        Ok(body) => {
            assert!(
                !body.ends_with(b"]}}"),
                "FORBIDDEN: a truncated stream closed the JSON document"
            );
            serde_json::from_slice::<serde_json::Value>(&body)
                .expect_err("FORBIDDEN: a truncated stream parsed as a complete result");
        }
    }
}
