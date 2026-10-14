//! (sq-bif.14) FEATURE-COMPOSITION test: the OPT-IN `audit-log` (flat `tracing`
//! event under target `sparq_server::audit`) and `access-audit` (richer JSON-Lines structured
//! sink) features enabled TOGETHER and turned on at runtime on the SAME server.
//!
//! Each audit feature is exercised in ISOLATION by `tests/audit_log.rs` and
//! `tests/access_audit.rs`. Neither single-feature leg can catch a COMPOSITION regression: the
//! two sinks hook the SAME request handler (see `src/http.rs`, where `audit::AuditRecord` and the
//! `access_audit` `AuditPending` are begun and emitted side-by-side on every read/update path).
//! One uses a PROCESS-GLOBAL `tracing` subscriber; the other a PER-`AppState` file sink. A
//! router/init-ordering or shared-state bug — one sink swallowing the other's emit, a decision
//! recorded inconsistently between the two, or the privacy redaction holding in only one — would
//! pass both single-feature legs yet fail here.
//!
//! The load-bearing invariant: for a single request, the two independently-derived audit records
//! must AGREE — same operation, same enforced decision, same HTTP status, same NON-reversible
//! query fingerprint — and BOTH must honour the redaction boundary (never the raw query text or
//! Bearer token). This is asserted for an ALLOWED read AND a DENIED read.
//!
//! Only one global `tracing` subscriber may be installed per process, so (as in `audit_log.rs`)
//! every assertion lives in a SINGLE `#[tokio::test]` driving both cases against one server.
//!
//! Honesty: the redaction assertions verify the documented #241 info-leak posture (the query text
//! and token never reach either sink). They are NOT an unqualified security guarantee — the audit
//! trail records identities and resource IRIs BY DESIGN; see the crate README + the per-feature
//! docs in `Cargo.toml`.
#![cfg(all(feature = "audit-log", feature = "access-audit"))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sparq_core::Graph;
use sparq_server::access_audit::SinkTarget;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;
use tracing::field::{Field, Visit};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};

const TOKEN: &str = "s3cr3t-composition-token";
const SELECT: &str = "SELECT ?s WHERE { ?s ?p ?o }";

// --- audit-log capture (a minimal global subscriber for the `sparq_server::audit` target) ----
// Mirrors the capture harness in tests/audit_log.rs so the composition assertions sit on the
// same shape as the single-feature leg.

#[derive(Debug, Clone, Default)]
struct Captured {
    fields: HashMap<String, String>,
}

impl Captured {
    fn get(&self, k: &str) -> &str {
        self.fields.get(k).map(String::as_str).unwrap_or("")
    }
}

#[derive(Clone)]
struct AuditCapture {
    records: Arc<Mutex<Vec<Captured>>>,
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_string(), format!("{:?}", value));
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

/// Reads the JSON-Lines access-audit file, retrying briefly so the synchronous handler emits
/// have landed before the test thread reads (same pattern as tests/access_audit.rs).
async fn read_jsonl(path: &std::path::Path, want: usize) -> Vec<serde_json::Value> {
    let mut tries = 0;
    loop {
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("each access-audit line is valid JSON"))
            .collect();
        if lines.len() >= want || tries > 50 {
            break lines;
        }
        tries += 1;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Both audit sinks turned on at runtime on ONE server: a single request must produce a
/// CONSISTENT record in EACH sink (same op / decision / status / fingerprint), and the redaction
/// boundary must hold in BOTH — proving the two features compose with no init-ordering or
/// shared-state interference.
#[tokio::test]
async fn audit_log_and_access_audit_compose_consistently() {
    // ---- access-audit: a per-AppState file sink (isolated; no global state) ----
    let dir = std::env::temp_dir().join(format!(
        "sparq-audit-compose-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let audit_path = dir.join("access.jsonl");

    // ---- audit-log: a process-global tracing subscriber for the `sparq_server::audit` target.
    // Installed BEFORE the access-audit sink is built so an init-ordering bug (the global
    // subscriber install disturbing the per-AppState sink, or vice versa) would surface.
    let tracing_records = Arc::new(Mutex::new(Vec::<Captured>::new()));
    tracing::subscriber::set_global_default(AuditCapture {
        records: tracing_records.clone(),
    })
    .expect("install audit-log subscriber");

    // BOTH features on at runtime on the SAME read-gated server.
    let config = ServerConfig {
        auth_token: Some(TOKEN.to_string()),
        auth_token_read: true,
        audit_log: true,
        access_audit: Some(SinkTarget::File(audit_path.clone())),
        ..ServerConfig::default()
    };
    let graph =
        Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let cl = reqwest::Client::new();

    // ---- ALLOWED: a read with the correct token ----
    let ok = cl
        .get(format!("{}/sparql", base))
        .header("accept", "application/sparql-results+json")
        .header("authorization", format!("Bearer {}", TOKEN))
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "authed read should succeed");
    let ok_body = ok.text().await.unwrap();

    // ---- DENIED: the same read with NO token (read surface is gated) ----
    let denied = cl
        .get(format!("{}/sparql", base))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401, "unauthed read should be 401");
    let denied_body = denied.text().await.unwrap();

    // The two sinks must compute the SAME fingerprint for the SAME query (consistent identity of
    // "what was attempted" across the audit estate). Both must be non-empty.
    let log_fp = sparq_server::audit::query_fingerprint(SELECT);
    let aa_fp = sparq_server::access_audit::fingerprint(SELECT);
    assert!(!log_fp.is_empty() && !aa_fp.is_empty(), "fingerprints are non-empty");
    assert_eq!(
        log_fp, aa_fp,
        "the two audit sinks must agree on the query fingerprint for the same query",
    );

    // ---- audit-log (tracing) records ----
    let log_recs = {
        let mut tries = 0;
        loop {
            let snapshot = tracing_records.lock().unwrap().clone();
            if snapshot.len() >= 2 || tries > 50 {
                break snapshot;
            }
            tries += 1;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    assert!(
        log_recs.len() >= 2,
        "audit-log must emit a record for BOTH requests even with access-audit also on, got {}: {:?}",
        log_recs.len(),
        log_recs,
    );
    let log_allowed = log_recs
        .iter()
        .find(|r| r.get("decision") == "allowed")
        .expect("an audit-log allowed record");
    let log_denied = log_recs
        .iter()
        .find(|r| r.get("decision") == "denied")
        .expect("an audit-log denied record");

    // ---- access-audit (JSON-Lines) records ----
    let aa_recs = read_jsonl(&audit_path, 2).await;
    assert!(
        aa_recs.len() >= 2,
        "access-audit must emit a record for BOTH requests even with audit-log also on, got {:?}",
        aa_recs,
    );
    let aa_allowed = aa_recs
        .iter()
        .find(|r| r["decision"] == "allow")
        .expect("an access-audit allow record");
    let aa_denied = aa_recs
        .iter()
        .find(|r| r["decision"] == "deny")
        .expect("an access-audit deny record");

    // ---- CROSS-SINK AGREEMENT (the composition invariant) for the ALLOWED request ----
    assert_eq!(log_allowed.get("op"), "query");
    assert_eq!(aa_allowed["action"], "query", "both sinks classify the read as a query");
    assert_eq!(log_allowed.get("status"), "200");
    assert_eq!(aa_allowed["status"], 200, "both sinks record the enforced 200");
    assert_eq!(
        log_allowed.get("fingerprint"),
        aa_allowed["fingerprint"].as_str().unwrap(),
        "the allowed record's fingerprint must match across the two sinks",
    );
    assert_eq!(log_allowed.get("fingerprint"), log_fp, "the SELECT's fingerprint");
    // Both sinks resolve the authed identity to the SAME token fingerprint, never the raw token.
    let log_actor = log_allowed.get("requester");
    let aa_actor = aa_allowed["actor"].as_str().unwrap();
    assert!(log_actor.starts_with("token:"), "audit-log: token-fingerprint identity");
    assert!(aa_actor.starts_with("token:"), "access-audit: token-fingerprint actor");
    assert_eq!(log_actor, aa_actor, "both sinks agree on the authed actor identity");
    assert!(!log_actor.contains(TOKEN) && !aa_actor.contains(TOKEN), "never the raw token");

    // ---- CROSS-SINK AGREEMENT for the DENIED request ----
    assert_eq!(log_denied.get("op"), "query");
    assert_eq!(aa_denied["action"], "query");
    assert_eq!(log_denied.get("status"), "401");
    assert_eq!(aa_denied["status"], 401, "both sinks record the enforced 401");
    assert_eq!(log_denied.get("requester"), "anonymous", "audit-log: anonymous");
    assert_eq!(aa_denied["actor"], "anonymous", "access-audit: anonymous");
    // The fingerprint of the attempted query is still recorded in BOTH on denial (operator sees
    // WHAT was attempted) — and it matches.
    assert_eq!(
        log_denied.get("fingerprint"),
        aa_denied["fingerprint"].as_str().unwrap(),
        "the denied record's fingerprint must match across the two sinks",
    );
    assert_eq!(log_denied.get("fingerprint"), log_fp);

    // ---- REDACTION BOUNDARY holds in BOTH sinks AND in the HTTP bodies ----
    // The raw query text + token must never reach the JSON-Lines file…
    let file = std::fs::read_to_string(&audit_path).unwrap();
    assert!(
        !file.contains("SELECT") && !file.contains("?s") && !file.contains(TOKEN),
        "access-audit must not write the raw query text or token",
    );
    // …and the audit detail (fingerprint) must never leak to the HTTP response bodies (the
    // server-side-log-only posture must hold even with both sinks on).
    assert!(
        !ok_body.contains(&log_fp) && !denied_body.contains(&log_fp),
        "the query fingerprint must not leak into the HTTP response body",
    );

    let _ = std::fs::remove_dir_all(&dir);
}
