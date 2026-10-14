//! (sq-0bxp) Integration tests for the OPT-IN per-query access audit log
//! (CDMC CD-2 / ISO 27001 A.8.15 / EU CRA logging).
//!
//! These spin the real axum server with the audit log turned on at runtime and capture the
//! `tracing` events emitted under the dedicated `sparq_server::audit` target via a custom
//! global subscriber, then assert the structured record fields for BOTH an ALLOWED and a
//! DENIED request:
//!   * an allowed read records `op=query`, `decision=allowed`, a non-empty query fingerprint,
//!     `status=200`, the requester identity, and a duration;
//!   * a denied read (read-gated surface, no/invalid token) records `decision=denied`,
//!     `status=401`, `requester=anonymous`, and a reason — and NEVER the raw query text or the
//!     Bearer token.
//!
//! The audit log is a SERVER-SIDE log: the test asserts the records surface to the operator's
//! tracing sink, never to the HTTP response body.
//!
//! Only one global `tracing` subscriber may be installed per process, so all assertions live
//! in a SINGLE `#[tokio::test]` that drives every case against one server + one subscriber.
#![cfg(feature = "audit-log")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;
use tracing::field::{Field, Visit};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};

const TOKEN: &str = "s3cr3t-audit-token";
const SELECT: &str = "SELECT ?s WHERE { ?s ?p ?o }";

/// One captured audit event: its string/number fields flattened into a map.
#[derive(Debug, Clone, Default)]
struct Captured {
    fields: HashMap<String, String>,
}

impl Captured {
    fn get(&self, k: &str) -> &str {
        self.fields.get(k).map(String::as_str).unwrap_or("")
    }
}

/// A minimal global subscriber that records ONLY events on the `sparq_server::audit` target,
/// flattening their fields into the shared `Vec`. Everything else is ignored (disabled).
#[derive(Clone)]
struct AuditCapture {
    records: Arc<Mutex<Vec<Captured>>>,
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

impl Subscriber for AuditCapture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "sparq_server::audit"
    }
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.target() == "sparq_server::audit" {
            Interest::always()
        } else {
            Interest::never()
        }
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != "sparq_server::audit" {
            return;
        }
        let mut fields = HashMap::new();
        event.record(&mut FieldVisitor(&mut fields));
        self.records.lock().unwrap().push(Captured { fields });
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn audit_log_emits_records_for_allowed_and_denied_requests() {
    let records = Arc::new(Mutex::new(Vec::<Captured>::new()));
    let capture = AuditCapture {
        records: records.clone(),
    };
    // Global (process-wide) so events from the server's tokio worker / blocking threads are
    // captured, not just this test thread.
    tracing::subscriber::set_global_default(capture).expect("install audit subscriber");

    // Read-gated surface (token gates reads AND writes) with the audit log ON.
    let config = ServerConfig {
        auth_token: Some(TOKEN.to_string()),
        auth_token_read: true,
        audit_log: true,
        ..ServerConfig::default()
    };
    let graph = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let cl = reqwest::Client::new();

    // ---- ALLOWED: a read with the correct token ----
    let ok = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .header("authorization", format!("Bearer {TOKEN}"))
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "authed read should succeed");
    let ok_body = ok.text().await.unwrap();

    // ---- DENIED: the same read with NO token (read surface is gated) ----
    let denied = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401, "unauthed read should be 401");
    let denied_body = denied.text().await.unwrap();

    // The audit detail must NEVER appear in the HTTP responses (server-side log only).
    let fp = sparq_server::audit::query_fingerprint(SELECT);
    assert!(
        !ok_body.contains(&fp) && !denied_body.contains(&fp),
        "fingerprint must not leak to the body"
    );
    assert!(
        !denied_body.contains("query"),
        "denied body carries only a generic message"
    );

    // Give the async emits a beat to land (they are synchronous in the handler, but the
    // response can race the test thread reading the shared Vec).
    let recs = {
        let mut tries = 0;
        loop {
            let snapshot = records.lock().unwrap().clone();
            if snapshot.len() >= 2 || tries > 50 {
                break snapshot;
            }
            tries += 1;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    assert!(
        recs.len() >= 2,
        "expected >= 2 audit records, got {}: {recs:?}",
        recs.len()
    );

    // Find the allowed + denied records.
    let allowed = recs
        .iter()
        .find(|r| r.get("decision") == "allowed")
        .expect("an allowed record");
    let denied = recs
        .iter()
        .find(|r| r.get("decision") == "denied")
        .expect("a denied record");

    // ---- ALLOWED record fields ----
    assert_eq!(allowed.get("op"), "query");
    assert_eq!(allowed.get("status"), "200");
    assert_eq!(allowed.get("fingerprint"), fp, "fingerprint of the SELECT");
    assert!(
        allowed.get("requester").starts_with("token:"),
        "authed => token fingerprint identity"
    );
    assert!(
        !allowed.get("requester").contains(TOKEN),
        "raw token must never be logged"
    );
    assert!(
        allowed.fields.contains_key("duration_us"),
        "duration recorded"
    );

    // ---- DENIED record fields ----
    assert_eq!(denied.get("op"), "query");
    assert_eq!(denied.get("status"), "401");
    assert_eq!(
        denied.get("requester"),
        "anonymous",
        "no token presented => anonymous"
    );
    assert!(
        denied.get("reason").contains("auth"),
        "denial carries a reason: {:?}",
        denied.get("reason")
    );
    // The fingerprint is still recorded for the denied request (so an operator sees WHAT was
    // attempted), but never the raw query text.
    assert_eq!(denied.get("fingerprint"), fp);
}
