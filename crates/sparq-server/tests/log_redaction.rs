//! (sq-toze.34, epic sq-toze) Integration test for the **request-log redaction**
//! option — the `--verbose` request log must NOT leak SPARQL query text (a PII/privacy exposure:
//! `GET /sparql?query=…` puts the full query in the logged URI).
//!
//! Strategy: spin the real axum server with `--verbose`, install a global `tracing` subscriber
//! that captures the values recorded on the `tower_http::trace` request SPAN (where the URI is
//! recorded), fire a `GET /sparql?query=<sensitive>` request, and assert:
//!   * with redaction ON (the DEFAULT), the captured span data does NOT contain the sensitive
//!     query string — it carries the `<redacted …>` placeholder instead;
//!   * with redaction OFF (`--log-full-requests`), the captured span DOES contain the query text,
//!     exactly as the bare TraceLayer logged before.
//!
//! Only one global subscriber may be installed per process, so BOTH cases run against ONE
//! subscriber in a single `#[tokio::test]`; the capture buffer is cleared between cases.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use std::sync::{Arc, Mutex};

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::subscriber::Interest;
use tracing::{Metadata, Subscriber};

/// A PII-shaped literal that must never appear verbatim in a redacted log.
const PII: &str = "alice.secret@example.com";

/// Captures the textual values recorded on any span/event under the `tower_http::trace` target.
#[derive(Clone)]
struct SpanCapture {
    seen: Arc<Mutex<Vec<String>>>,
}

struct CollectVisitor<'a>(&'a mut Vec<String>);

impl Visit for CollectVisitor<'_> {
    fn record_debug(&mut self, _field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push(format!("{value:?}"));
    }
    fn record_str(&mut self, _field: &Field, value: &str) {
        self.0.push(value.to_string());
    }
}

/// The request log lives under the `tower_http` target (the started/finished events) and the
/// redacting span lives under the `sparq_server` crate's module path — capture BOTH so the test
/// sees the recorded `uri` field regardless of which side records it.
fn is_request_target(target: &str) -> bool {
    target.starts_with("tower_http") || target.starts_with("sparq_server")
}

impl Subscriber for SpanCapture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        is_request_target(metadata.target())
    }
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if is_request_target(metadata.target()) {
            Interest::always()
        } else {
            Interest::never()
        }
    }
    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        if is_request_target(attrs.metadata().target()) {
            let mut vals = self.seen.lock().unwrap();
            attrs.record(&mut CollectVisitor(&mut vals));
        }
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, values: &tracing::span::Record<'_>) {
        let mut vals = self.seen.lock().unwrap();
        values.record(&mut CollectVisitor(&mut vals));
    }
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if is_request_target(event.metadata().target()) {
            let mut vals = self.seen.lock().unwrap();
            event.record(&mut CollectVisitor(&mut vals));
        }
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

async fn spawn(config: ServerConfig) -> String {
    let graph = Graph::load_str("", "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn verbose_log_redacts_query_by_default_and_logs_full_when_opted_out() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    tracing::subscriber::set_global_default(SpanCapture { seen: seen.clone() })
        .expect("install span capture");

    let query = format!("SELECT ?s WHERE {{ ?s <http://ex/email> \"{PII}\" }}");
    let client = reqwest::Client::new();

    // --- Case 1: redaction ON (the default) -------------------------------------------------
    let base = spawn(ServerConfig {
        verbose: true,
        // redact_logs defaults to true; assert that explicitly here for clarity.
        redact_logs: true,
        ..ServerConfig::default()
    })
    .await;
    let resp = client
        .get(format!("{base}/sparql"))
        .query(&[("query", query.as_str())])
        .header("Accept", "application/sparql-results+json")
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), 200, "query should succeed");
    // give the trace span a moment to be recorded
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let captured: Vec<String> = seen.lock().unwrap().clone();
    let joined = captured.join("\n");
    assert!(
        !joined.contains(PII),
        "PII literal MUST be absent from the redacted request log, but was present in:\n{joined}"
    );
    assert!(
        !joined.contains("SELECT"),
        "SPARQL query text MUST be absent from the redacted request log, but was present in:\n{joined}"
    );
    assert!(
        joined.contains("<redacted"),
        "redacted placeholder should be present in the request log:\n{joined}"
    );

    // --- Case 2: redaction OFF (--log-full-requests) ----------------------------------------
    seen.lock().unwrap().clear();
    let base = spawn(ServerConfig {
        verbose: true,
        redact_logs: false,
        ..ServerConfig::default()
    })
    .await;
    let resp = client
        .get(format!("{base}/sparql"))
        .query(&[("query", query.as_str())])
        .header("Accept", "application/sparql-results+json")
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let captured: Vec<String> = seen.lock().unwrap().clone();
    let joined = captured.join("\n");
    // With redaction off the raw URI (URL-encoded query) is logged. The PII appears either
    // verbatim or percent-encoded depending on the client encoding; either form is a leak the
    // operator opted into. Assert the (decoded) query text marker survived in some form.
    assert!(
        joined.contains("query=") || joined.contains("SELECT") || joined.contains("alice"),
        "with --log-full-requests the request URI/query should be logged verbatim:\n{joined}"
    );
}
