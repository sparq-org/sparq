//! (sq-gos8, epic sq-toze) Integration tests for the OPT-IN RICHER STRUCTURED
//! access-audit sink (ASVS V7 / ISO 27001 A.8.15 / CDMC CD-2 logging).
//!
//! These spin the real axum server with a file-backed structured access-audit sink configured
//! at runtime, drive requests through the REAL enforcement seam, then read back the JSON-Lines
//! audit file and assert the structured record fields for BOTH an ALLOWED and a DENIED request:
//!   * an allowed read records `action=query`, `decision=allow`, the actor (a token
//!     fingerprint, never the raw token), the resource, a NON-empty query fingerprint,
//!     `status=200`, the policy basis and a duration;
//!   * a denied read (read-gated surface, no token) records `decision=deny`, `status=401`,
//!     `actor=anonymous`, the enforced policy basis — and NEVER the raw query text or token.
//!
//! The privacy boundary is asserted directly: the query CONTENT never appears in the audit
//! file (only its fingerprint), while the resource IRI / actor identity DO (by design).
#![cfg(feature = "access-audit")]

use sparq_core::Graph;
use sparq_server::access_audit::{fingerprint, SinkTarget};
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const TOKEN: &str = "s3cr3t-access-audit-token";
const SELECT: &str = "SELECT ?s WHERE { ?s ?p ?o }";

/// Reads the JSON-Lines audit file and parses each line into a JSON value, retrying briefly so
/// the synchronous handler emits have landed before the test thread reads the file.
async fn read_records(path: &std::path::Path, want: usize) -> Vec<serde_json::Value> {
    let mut tries = 0;
    loop {
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("each audit line is valid JSON"))
            .collect();
        if lines.len() >= want || tries > 50 {
            break lines;
        }
        tries += 1;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn structured_sink_records_allow_and_deny_through_the_real_path() {
    // A unique temp audit file (no global state — unlike the tracing audit-log, the structured
    // sink is per-AppState, so this test is fully isolated).
    let dir = std::env::temp_dir().join(format!("sparq-access-audit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let audit_path = dir.join("access.jsonl");

    // Read-gated surface (token gates reads AND writes) with the structured sink ON.
    let config = ServerConfig {
        auth_token: Some(TOKEN.to_string()),
        auth_token_read: true,
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

    // ---- DENIED: the same read with NO token (read surface is gated) ----
    let denied = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401, "unauthed read should be 401");

    let recs = read_records(&audit_path, 2).await;
    assert!(recs.len() >= 2, "expected >= 2 audit records, got {recs:?}");

    let allowed = recs
        .iter()
        .find(|r| r["decision"] == "allow")
        .expect("an allow record");
    let denied = recs
        .iter()
        .find(|r| r["decision"] == "deny")
        .expect("a deny record");

    // ---- ALLOWED record: full structured fields through the real decision path ----
    assert_eq!(allowed["action"], "query");
    assert_eq!(allowed["status"], 200);
    assert_eq!(allowed["resource"], "/sparql");
    assert_eq!(allowed["resource_kind"], "dataset");
    assert_eq!(allowed["fingerprint"], fingerprint(SELECT));
    assert_eq!(allowed["policy_basis"], "bearer-auth: allowed");
    let actor = allowed["actor"].as_str().unwrap();
    assert!(actor.starts_with("token:"), "authed => token fingerprint actor");
    assert!(!actor.contains(TOKEN), "raw token must never be logged");
    assert!(allowed["duration_us"].is_u64(), "duration recorded");
    assert!(allowed["ts"].as_str().unwrap().ends_with('Z'), "RFC-3339 UTC ts");

    // ---- DENIED record: the ENFORCED decision + basis ----
    assert_eq!(denied["action"], "query");
    assert_eq!(denied["status"], 401);
    assert_eq!(denied["actor"], "anonymous", "no token => anonymous");
    assert_eq!(denied["policy_basis"], "bearer-auth: missing or invalid token");
    // The fingerprint is still recorded (operator sees WHAT was attempted) — never the raw text.
    assert_eq!(denied["fingerprint"], fingerprint(SELECT));

    // ---- PRIVACY BOUNDARY: query CONTENT never written; resource + actor ARE (by design) ----
    let file = std::fs::read_to_string(&audit_path).unwrap();
    assert!(
        !file.contains("SELECT") && !file.contains("?s") && !file.contains(TOKEN),
        "query content + raw token must never be written to the audit trail"
    );
    assert!(file.contains("/sparql"), "resource IS recorded (the audit trail)");
    assert!(file.contains("token:"), "actor identity IS recorded (the audit trail)");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn graph_store_write_records_named_graph_resource() {
    let dir = std::env::temp_dir().join(format!("sparq-aa-gsp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let audit_path = dir.join("access.jsonl");
    let config = ServerConfig {
        access_audit: Some(SinkTarget::File(audit_path.clone())),
        ..ServerConfig::default()
    };
    let graph = Graph::load_str("", "ntriples").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let cl = reqwest::Client::new();

    // A GSP PUT to a named graph (indirect identification) — a write the audit records with the
    // named-graph IRI as the resource.
    let g = "http://example.org/g1";
    let put = cl
        .put(format!("{base}/sparql/graph"))
        .query(&[("graph", g)])
        .header("content-type", "application/n-triples")
        .body("<http://ex/a> <http://ex/b> <http://ex/c> .")
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "GSP PUT should succeed: {}", put.status());

    let recs = read_records(&audit_path, 1).await;
    let rec = recs
        .iter()
        .find(|r| r["action"] == "graph_write")
        .expect("a graph_write record");
    assert_eq!(rec["resource"], g, "the named-graph IRI is the resource");
    assert_eq!(rec["resource_kind"], "named_graph");
    assert_eq!(rec["decision"], "allow");
    assert_eq!(rec["fingerprint"], "-", "a GSP body has no query to fingerprint");

    let _ = std::fs::remove_dir_all(&dir);
}

/// sq-ljfz (sq-gos8 follow-up): when the operator configures a TRUSTED forwarded
/// -identity header, a fronting auth layer's resolved WebID is recorded as the audit actor
/// (`Actor::WebId`) — the real authenticated subject — instead of the coarse Bearer-token
/// fingerprint. This is the bead's enrichment, driven end-to-end through the real seam.
#[tokio::test]
async fn trusted_forwarded_webid_header_becomes_the_audit_actor() {
    let dir = std::env::temp_dir().join(format!("sparq-aa-webid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let audit_path = dir.join("access.jsonl");

    // Trust the front's `X-WebID` header (the deployment names sparq-solid / a Solid-WAC proxy).
    let config = ServerConfig {
        access_audit: Some(SinkTarget::File(audit_path.clone())),
        audit_webid_header: Some("X-WebID".to_string()),
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
    let base = format!("http://{addr}");
    let cl = reqwest::Client::new();

    let webid = "https://alice.example/profile#me";
    // Header casing is deliberately different from the configured name to exercise the
    // case-insensitive HTTP header match.
    let ok = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .header("x-webid", webid)
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "unauthed open read should succeed");

    let recs = read_records(&audit_path, 1).await;
    let rec = recs.iter().find(|r| r["decision"] == "allow").expect("an allow record");
    assert_eq!(
        rec["actor"],
        format!("webid:{webid}"),
        "the trusted front's WebID is the audit actor",
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// sq-ljfz (audit-integrity / no-spoof): with NO trusted header configured (the
/// default), a client-supplied identity header is IGNORED — the actor stays the local Bearer
/// gate (here `anonymous`). This is the guarantee that a direct client cannot spoof an
/// arbitrary WebID into the audit trail; the WebID is honoured ONLY behind a trusted front the
/// operator opted into.
#[tokio::test]
async fn forwarded_identity_header_is_ignored_unless_a_trusted_header_is_configured() {
    let dir = std::env::temp_dir().join(format!("sparq-aa-nospoof-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let audit_path = dir.join("access.jsonl");

    // NO audit_webid_header set => trust nothing forwarded.
    let config = ServerConfig {
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
    let base = format!("http://{addr}");
    let cl = reqwest::Client::new();

    let spoofed = "https://attacker.example/#me";
    let ok = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        // A common forwarded-identity header name — but the operator trusts none, so it is inert.
        .header("X-WebID", spoofed)
        .query(&[("query", SELECT)])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    let recs = read_records(&audit_path, 1).await;
    let rec = recs.iter().find(|r| r["decision"] == "allow").expect("an allow record");
    assert_eq!(rec["actor"], "anonymous", "a spoofed header must NOT become the actor");
    let file = std::fs::read_to_string(&audit_path).unwrap();
    assert!(
        !file.contains(spoofed),
        "an untrusted client-supplied WebID must never reach the audit trail"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
