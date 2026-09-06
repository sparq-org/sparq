//! [FABLE-5] sq-snopa.6 (issue #992 FR-4): e2e integration tests for the OPT-IN Solid WAC/ACP HTTP
//! authorization surface (`POST /authz/decide`, `POST /authz/wac-allow`, `POST /authz/query`).
//!
//! These tests run ONLY with the `solid-authz` cargo feature (the whole file is gated). They spin
//! the REAL axum server and assert, over an HTTP request, that the endpoints exercise the REAL
//! `sparq-solid` authoriser path (not a mock) and uphold the load-bearing invariants:
//!   * the OPT-IN posture — `/authz/*` is `404` unless the config flag is set;
//!   * a real allow — an authenticated session with a grant gets `allow:true` + the FR-5
//!     `governingAcl` / `aclLink` provenance;
//!   * FAIL-CLOSED (the soundness invariant) — an anonymous session over the SAME dataset is a
//!     `403` deny; an UNPARSEABLE dataset is a fail-closed `400` (NOT an empty-dataset allow); an
//!     unknown mode is a `403` deny (never a grant);
//!   * `/authz/wac-allow` builds the RFC-style permission advertisement, and a grant-less session
//!     advertises nothing (`user=""`);
//!   * `/authz/query` runs an ACCESS-CONTROLLED query — the SAME query returns the granted rows for
//!     the authorised session and ZERO rows for an anonymous one (fail-closed view);
//!   * read-auth gating (`--auth-token-read`) — `401` without the bearer token, `200`/`403` with it.
#![cfg(feature = "solid-authz")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// A minimal boot store — the `/authz/*` endpoints NEVER read it (they authorise over the dataset
/// supplied in the request body), but the server needs a graph to boot.
const BOOT: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:alice a ex:Person .
"#;

/// The pod dataset the tests POST: alice has Read on the root `.acl`, inherited by `notes/n1`.
const WAC_NQUADS: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;

/// Boots a server with the solid-authz flag set as requested, returns its base URL.
async fn spawn_with(authz_on: bool, config: ServerConfig) -> String {
    let graph = Graph::load_str(BOOT, "turtle").unwrap();
    let config = ServerConfig {
        solid_authz: authz_on,
        ..config
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn(authz_on: bool) -> String {
    spawn_with(authz_on, ServerConfig::default()).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// The `{dataset, session:{agent}, resource, mode}` decide body for `agent` (or anonymous when
/// `agent` is `None`).
fn decide_body(agent: Option<&str>, resource: &str, mode: &str) -> serde_json::Value {
    let mut session = serde_json::Map::new();
    if let Some(a) = agent {
        session.insert("agent".into(), a.into());
    }
    serde_json::json!({
        "dataset": WAC_NQUADS,
        "session": session,
        "resource": resource,
        "mode": mode,
        "view": "wac",
    })
}

// ---------------------------------------------------------------------------
// OPT-IN posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authz_404_when_flag_off() {
    let base = spawn(false).await;
    for path in ["/authz/decide", "/authz/wac-allow", "/authz/query"] {
        let resp = client()
            .post(format!("{base}{path}"))
            .json(&serde_json::json!({ "dataset": WAC_NQUADS }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404, "{path} must be 404 with the flag off");
    }
}

// ---------------------------------------------------------------------------
// /authz/decide — a real allow with FR-5 provenance, and fail-closed denies.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn decide_allows_authenticated_grant_with_provenance() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "read",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["allow"], serde_json::Value::Bool(true));
    assert_eq!(body["status"], "resolved");
    // FR-5 provenance is in the body (feeds the sq-snopa.7 Link header).
    assert_eq!(body["governingAcl"], "https://pod.ex/.acl");
    assert_eq!(body["aclLink"], r#"<https://pod.ex/.acl>; rel="acl""#);
    assert!(body["grantedModes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m == "read"));
}

/// FAIL-CLOSED: the SAME dataset + resource, but anonymous, is an authoritative `403` deny.
#[tokio::test]
async fn decide_denies_anonymous_fail_closed() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(None, "https://pod.ex/notes/n1", "read"))
        .send()
        .await
        .unwrap();
    // A definitive permission deny -> 403 (not a 503, not an allow).
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["allow"], serde_json::Value::Bool(false));
    assert_eq!(body["status"], "resolved");
}

/// FAIL-CLOSED: an UNPARSEABLE dataset is a `400`, NOT an empty-dataset allow. This is the
/// load-bearing soundness invariant — an error path in the HTTP layer must DENY.
#[tokio::test]
async fn decide_denies_on_unparseable_dataset() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&serde_json::json!({
            "dataset": "this is @@@ not n-quads",
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/notes/n1",
            "mode": "read",
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "an unparseable dataset must fail closed, not allow");
}

/// FAIL-CLOSED: an unknown mode is a `403` deny — the HTTP layer never grants an unrecognised mode.
#[tokio::test]
async fn decide_denies_unknown_mode_fail_closed() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "delete", // not a WAC mode
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["allow"], serde_json::Value::Bool(false));
}

/// A mode the agent lacks (Write) is a `403` deny, not a 503 — an authoritative outcome.
#[tokio::test]
async fn decide_denies_ungranted_mode_403_not_503() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "write",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

// ---------------------------------------------------------------------------
// /authz/wac-allow — the RFC-style permission advertisement.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wac_allow_advertises_granted_modes_and_public() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/wac-allow"))
        .json(&serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/notes/n1",
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    // alice holds read; there is no public grant here, so public is empty.
    assert_eq!(body["wacAllow"], r#"user="read",public="""#);
}

/// FAIL-CLOSED: a grant-less (anonymous) session advertises NOTHING (`user=""`).
#[tokio::test]
async fn wac_allow_grantless_advertises_nothing() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/wac-allow"))
        .json(&serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": {},
            "resource": "https://pod.ex/notes/n1",
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["wacAllow"], r#"user="",public="""#);
}

// ---------------------------------------------------------------------------
// /authz/query — access-controlled query (two sessions, different results).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn query_is_access_controlled_per_session() {
    let base = spawn(true).await;
    let query = "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }";

    // The authorised session sees the row.
    let resp = client()
        .post(format!("{base}/authz/query"))
        .json(&serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "query": query,
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let rows = body["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the authorised session sees the granted row");

    // FAIL-CLOSED: the SAME query as anonymous returns ZERO rows (empty view).
    let resp = client()
        .post(format!("{base}/authz/query"))
        .json(&serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": {},
            "query": query,
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let rows = body["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 0, "an anonymous session sees ZERO rows (fail-closed view)");
}

// ---------------------------------------------------------------------------
// sq-snopa.7 — Link: rel="acl" response header (FR-5).
// The decide and wac-allow endpoints must emit the RFC 8288 Link header value from
// WacDecision::acl_link_header() when a governing ACL was discovered, and MUST NOT emit it
// when none was found (fail-closed — nothing to advertise).
// ---------------------------------------------------------------------------

/// decide emits `Link: <acl-iri>; rel="acl"` when a governing ACL is known.
/// MUTATION SPOT-CHECK: change the expected value below to `<https://pod.ex/.OTHER>; rel="acl"`
/// and this test goes red — confirming it exercises the real header, not a vacuous assertion.
#[tokio::test]
async fn decide_emits_link_header_when_governing_acl_known() {
    // [SONNET-4.6] sq-snopa.7: assert the RFC-8288 Link header is present and correct.
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "read",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let link = resp
        .headers()
        .get("link")
        .expect("decide must emit a Link header when a governing ACL is known")
        .to_str()
        .unwrap();
    assert_eq!(
        link,
        r#"<https://pod.ex/.acl>; rel="acl""#,
        "Link header must be the RFC-8288 link-value for the governing ACL"
    );
}

/// decide does NOT emit a Link header when no governing ACL exists (resource outside the pod).
/// Fail-closed: if there is no ACL document to advertise, nothing is emitted.
#[tokio::test]
async fn decide_omits_link_header_when_no_governing_acl() {
    // [SONNET-4.6] sq-snopa.7: resource with no governing ACL → no Link header.
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://other.ex/resource", // not in the WAC dataset — no governing ACL
            "read",
        ))
        .send()
        .await
        .unwrap();
    // The status is a deny (no grant) — but we are testing the HEADER, not the body.
    assert!(
        resp.headers().get("link").is_none(),
        "decide must NOT emit a Link header when no governing ACL was discovered"
    );
}

/// wac-allow emits `Link: <acl-iri>; rel="acl"` when a governing ACL is known.
#[tokio::test]
async fn wac_allow_emits_link_header_when_governing_acl_known() {
    // [SONNET-4.6] sq-snopa.7: wac-allow must also carry the Link header.
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/wac-allow"))
        .json(&serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/notes/n1",
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let link = resp
        .headers()
        .get("link")
        .expect("wac-allow must emit a Link header when a governing ACL is known")
        .to_str()
        .unwrap();
    assert_eq!(
        link,
        r#"<https://pod.ex/.acl>; rel="acl""#,
        "Link header must be the RFC-8288 link-value for the governing ACL"
    );
}

/// wac-allow does NOT emit a Link header when no governing ACL exists.
#[tokio::test]
async fn wac_allow_omits_link_header_when_no_governing_acl() {
    // [SONNET-4.6] sq-snopa.7: resource outside the dataset → no governing ACL → no Link header.
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/authz/wac-allow"))
        .json(&serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://other.ex/resource", // not in the WAC dataset
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().get("link").is_none(),
        "wac-allow must NOT emit a Link header when no governing ACL was discovered"
    );
}

// ---------------------------------------------------------------------------
// read-auth gating.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn decide_is_read_auth_gated() {
    let config = ServerConfig {
        auth_token: Some("sekret".into()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let base = spawn_with(true, config).await;

    // Without the bearer token -> 401.
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "read",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // With it -> 200 (the decision then runs).
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .header("Authorization", "Bearer sekret")
        .json(&decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "read",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-snopa.8 — the STATEFUL lane (`"source":"server"`).
//
// Everything above authorises over a dataset supplied in the request BODY. These tests boot a
// server whose OWN loaded store IS the pod and assert that `"source":"server"` decides over it:
// the same allow/deny contract, no `"dataset"` in the body, and — the load-bearing new
// invariant — that an `.acl` WRITE is picked up, because the cached authorization view is keyed
// by the ring generation and every commit publishes a new one.
// ---------------------------------------------------------------------------

/// Boots a server whose OWN loaded store is `WAC_NQUADS` (rather than the throwaway `BOOT`
/// graph), with `solid_authz` on — the pod the stateful lane authorises over.
async fn spawn_pod_server() -> String {
    let graph = Graph::load_dataset(WAC_NQUADS, "nquads").unwrap();
    let config = ServerConfig {
        solid_authz: true,
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

/// A `"source":"server"` decide body — note the deliberate ABSENCE of `"dataset"`.
fn server_decide_body(agent: Option<&str>, resource: &str, mode: &str) -> serde_json::Value {
    let mut session = serde_json::Map::new();
    if let Some(a) = agent {
        session.insert("agent".into(), a.into());
    }
    serde_json::json!({
        "source": "server",
        "session": session,
        "resource": resource,
        "mode": mode,
        "view": "wac",
    })
}

/// The stateful lane decides over the SERVER'S OWN store: a granted agent is allowed (with the
/// FR-5 provenance), and an anonymous session over the same store is a fail-closed 403.
#[tokio::test]
async fn stateful_decide_reads_the_servers_own_store() {
    let base = spawn_pod_server().await;

    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&server_decide_body(
            Some("https://alice.ex/card#me"),
            "https://pod.ex/notes/n1",
            "read",
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "alice's grant lives in the server's own store");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["allow"], serde_json::Value::Bool(true));
    assert_eq!(body["governingAcl"], "https://pod.ex/.acl");

    // FAIL-CLOSED: anonymous over the SAME server store is an authoritative deny.
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&server_decide_body(None, "https://pod.ex/notes/n1", "read"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["allow"], serde_json::Value::Bool(false));
}

/// THE HEADLINE GUARD (sq-snopa.8's "re-materialise on ACL change"): bob is denied, an `.acl`
/// WRITE grants him Read, and the very next stateful decision ALLOWS him — proving the cached
/// per-generation view was invalidated by the commit rather than serving a stale grant set.
///
/// MUTATION SPOT-CHECK: delete the `state.current()` generation comparison in
/// `build_server_view` (i.e. always return the cached entry) and the post-write assertion below
/// goes red — the second decision would still see bob's pre-write deny.
#[tokio::test]
async fn stateful_view_rematerialises_after_an_acl_write() {
    let base = spawn_pod_server().await;
    let bob = "https://bob.ex/card#me";
    let decide = |body: serde_json::Value| {
        let base = base.clone();
        async move {
            client()
                .post(format!("{base}/authz/decide"))
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };

    // 1. Bob has no grant yet — a definitive deny, which also WARMS the generation-0 cache.
    let resp = decide(server_decide_body(Some(bob), "https://pod.ex/notes/n1", "read")).await;
    assert_eq!(resp.status(), 403, "bob starts with no grant");

    // 2. Write a Read authorization for bob into the pod's `.acl` graph.
    let update = r#"
        INSERT DATA {
          GRAPH <https://pod.ex/.acl> {
            <https://pod.ex/.acl#bob>
              <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> ;
              <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> ;
              <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> ;
              <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> .
          }
        }
    "#;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(update)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204, "the ACL write must commit");

    // 3. The next stateful decision must see the NEW grant (a new generation => a re-materialise).
    let resp = decide(server_decide_body(Some(bob), "https://pod.ex/notes/n1", "read")).await;
    assert_eq!(
        resp.status(),
        200,
        "the ACL write must re-materialise the cached authorization view"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["allow"], serde_json::Value::Bool(true));

    // 4. And the write did not widen anything else: an anonymous session is still denied.
    let resp = decide(server_decide_body(None, "https://pod.ex/notes/n1", "read")).await;
    assert_eq!(resp.status(), 403, "the re-materialised view stays fail-closed");
}

/// `/authz/query` over the server's own store is access-controlled per session, exactly as the
/// stateless lane is over a body dataset.
#[tokio::test]
async fn stateful_query_is_access_controlled_over_the_servers_own_store() {
    let base = spawn_pod_server().await;
    let query = "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }";

    let resp = client()
        .post(format!("{base}/authz/query"))
        .json(&serde_json::json!({
            "source": "server",
            "session": { "agent": "https://alice.ex/card#me" },
            "query": query,
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let rows = body["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the authorised session sees the granted row");

    // FAIL-CLOSED: anonymous sees ZERO rows — never the whole loaded store.
    let resp = client()
        .post(format!("{base}/authz/query"))
        .json(&serde_json::json!({
            "source": "server",
            "session": {},
            "query": query,
            "view": "wac",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let rows = body["results"]["bindings"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        0,
        "an anonymous session must NOT see the server's loaded store"
    );
}

/// `/authz/wac-allow` advertises over the server's own store, and a grant-less session still
/// advertises nothing.
#[tokio::test]
async fn stateful_wac_allow_advertises_over_the_servers_own_store() {
    let base = spawn_pod_server().await;
    let body = |session: serde_json::Value| {
        serde_json::json!({
            "source": "server",
            "session": session,
            "resource": "https://pod.ex/notes/n1",
            "view": "wac",
        })
    };

    let resp = client()
        .post(format!("{base}/authz/wac-allow"))
        .json(&body(serde_json::json!({ "agent": "https://alice.ex/card#me" })))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let out: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(out["wacAllow"], r#"user="read",public="""#);

    let resp = client()
        .post(format!("{base}/authz/wac-allow"))
        .json(&body(serde_json::json!({})))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let out: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(out["wacAllow"], r#"user="",public="""#);
}

/// FAIL-CLOSED: a body naming BOTH pods (`"source":"server"` AND a `"dataset"`) is ambiguous
/// about which one it means, so it is refused rather than silently resolved to either.
#[tokio::test]
async fn stateful_source_rejects_an_ambiguous_body_dataset() {
    let base = spawn_pod_server().await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&serde_json::json!({
            "source": "server",
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/notes/n1",
            "mode": "read",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "an ambiguous body must be refused");
}

/// FAIL-CLOSED: an unknown `"source"` keyword is a 400 on the wire, never an inferred default.
#[tokio::test]
async fn stateful_unknown_source_keyword_is_refused() {
    let base = spawn_pod_server().await;
    let resp = client()
        .post(format!("{base}/authz/decide"))
        .json(&serde_json::json!({
            "source": "elsewhere",
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/notes/n1",
            "mode": "read",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}
