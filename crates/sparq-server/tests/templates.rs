//! sq-lsp7k.10: e2e integration tests for the OPT-IN named-parameterized-template
//! REST surface (`/templates`).
//!
//! These tests run ONLY with the `templates` cargo feature (the whole file is gated). They
//! spin the real axum server and assert, over HTTP:
//!   * the OPT-IN posture — every `/templates` path is `404` unless the config flag is set;
//!   * CRUD — `PUT` stores (201) / replaces (204), `GET` round-trips the definition,
//!     `GET /templates` lists, `DELETE` removes (204/404);
//!   * fail-closed registration — an unparseable text / undeclared parameter is a `400`;
//!   * invocation — typed parameters produce the same rows as the hand-written constant
//!     query (SPARQL-JSON for SELECT, N-Triples for CONSTRUCT);
//!   * FAIL-CLOSED invocation — an unknown / missing / mistyped parameter is a `400`;
//!   * the GATED-UPDATE POSTURE — an UPDATE template invocation goes through the same
//!     write gate + sequenced-writer path as a `/sparql` update: 401 without the write
//!     token, 204 + visible mutation with it, and a hostile bound literal stays DATA
//!     (the #901 injection posture end-to-end over HTTP);
//!   * template writes (`PUT`/`DELETE`) are write-gated too;
//!   * persistence — `templates_file` survives a restart, and a corrupt file is a
//!     fail-closed startup error.

#![cfg(feature = "templates")]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:name "Alice" .
    ex:bob   ex:knows ex:carol ; ex:name "Bob" .
"#;

/// Boots a server holding `DATA`, returning its base URL.
async fn spawn_with(templates_on: bool, config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        templates: templates_on,
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

async fn spawn(templates_on: bool) -> String {
    spawn_with(templates_on, ServerConfig::default()).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// The friends-of template definition used across tests.
fn friends_def() -> Value {
    json!({
        "text": "SELECT ?f WHERE { ?who <http://ex/knows> ?f }",
        "parameters": { "who": "iri" },
        "description": "who does ?who know"
    })
}

async fn put_template(base: &str, name: &str, def: &Value) -> reqwest::Response {
    client()
        .put(format!("{base}/templates/{name}"))
        .json(def)
        .send()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// OPT-IN posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn templates_404_when_flag_off() {
    let base = spawn(false).await;
    for (method, path) in [
        ("GET", "/templates"),
        ("GET", "/templates/t"),
        ("PUT", "/templates/t"),
        ("POST", "/templates/t"),
        ("DELETE", "/templates/t"),
    ] {
        let req = match method {
            "GET" => client().get(format!("{base}{path}")),
            "PUT" => client().put(format!("{base}{path}")).json(&friends_def()),
            "POST" => client().post(format!("{base}{path}")).json(&json!({})),
            _ => client().delete(format!("{base}{path}")),
        };
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 404, "{method} {path} must be 404 when off");
        assert_eq!(resp.json::<Value>().await.unwrap()["error"], "not found");
    }
}

// ---------------------------------------------------------------------------
// CRUD.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn put_get_list_delete_round_trip() {
    let base = spawn(true).await;
    // Empty list initially.
    let list: Value = client()
        .get(format!("{base}/templates"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list, json!([]));
    // Create → 201; replace → 204.
    assert_eq!(put_template(&base, "friends", &friends_def()).await.status(), 201);
    assert_eq!(put_template(&base, "friends", &friends_def()).await.status(), 204);
    // GET round-trips the definition (name from the path, kind derived).
    let def: Value = client()
        .get(format!("{base}/templates/friends"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(def["name"], "friends");
    assert_eq!(def["kind"], "query");
    assert_eq!(def["parameters"], json!({"who": "iri"}));
    // List shows it.
    let list: Value = client()
        .get(format!("{base}/templates"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    // DELETE → 204, then 404 (both the delete and the get).
    assert_eq!(
        client()
            .delete(format!("{base}/templates/friends"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        client()
            .delete(format!("{base}/templates/friends"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client()
            .get(format!("{base}/templates/friends"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn put_rejects_invalid_definitions() {
    let base = spawn(true).await;
    // Unparseable SPARQL.
    let resp = put_template(&base, "bad", &json!({"text": "NOT SPARQL"})).await;
    assert_eq!(resp.status(), 400);
    // A declared parameter that is not a free placeholder.
    let resp = put_template(
        &base,
        "bad",
        &json!({"text": "ASK { ?s ?p ?o }", "parameters": {"nope": "iri"}}),
    )
    .await;
    assert_eq!(resp.status(), 400);
    // A body name that contradicts the path.
    let resp = put_template(&base, "bad", &json!({"name": "other", "text": "ASK { ?s ?p ?o }"}))
        .await;
    assert_eq!(resp.status(), 400);
    // Not JSON at all.
    let resp = client()
        .put(format!("{base}/templates/bad"))
        .body("nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    // IRI-identified template names (wildcard capture carries `/`).
    let resp = put_template(&base, "http://ex/tpl/friends", &friends_def()).await;
    assert_eq!(resp.status(), 201);
    let def: Value = client()
        .get(format!("{base}/templates/http://ex/tpl/friends"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(def["name"], "http://ex/tpl/friends");
}

// ---------------------------------------------------------------------------
// Invocation — typed binding, result equivalence, fail-closed arguments.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoke_select_binds_typed_params() {
    let base = spawn(true).await;
    put_template(&base, "friends", &friends_def()).await;
    let resp = client()
        .post(format!("{base}/templates/friends"))
        .json(&json!({"who": "http://ex/alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/sparql-results+json"));
    let body: Value = resp.json().await.unwrap();
    let bindings = body["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["f"]["value"], "http://ex/bob");
    // 404 for an unknown template.
    let resp = client()
        .post(format!("{base}/templates/nope"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn invoke_construct_returns_ntriples() {
    let base = spawn(true).await;
    put_template(
        &base,
        "describe-knows",
        &json!({
            "text": "CONSTRUCT { ?who <http://ex/knows> ?f } WHERE { ?who <http://ex/knows> ?f }",
            "parameters": { "who": "iri" }
        }),
    )
    .await;
    let resp = client()
        .post(format!("{base}/templates/describe-knows"))
        .json(&json!({"who": "http://ex/bob"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/n-triples"));
    let body = resp.text().await.unwrap();
    assert!(body.contains("<http://ex/bob> <http://ex/knows> <http://ex/carol>"), "{body}");
}

#[tokio::test]
async fn invoke_fail_closed_on_bad_arguments() {
    let base = spawn(true).await;
    put_template(&base, "friends", &friends_def()).await;
    // Unknown parameter name.
    let resp = client()
        .post(format!("{base}/templates/friends"))
        .json(&json!({"who": "http://ex/alice", "typo": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert!(resp.text().await.unwrap().contains("unknown parameter"));
    // Missing required parameter (empty body counts as no arguments).
    let resp = client()
        .post(format!("{base}/templates/friends"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    // Wrong JSON shape for the declared type (a number is not an IRI string).
    let resp = client()
        .post(format!("{base}/templates/friends"))
        .json(&json!({"who": 5}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    // Not-JSON body.
    let resp = client()
        .post(format!("{base}/templates/friends"))
        .body("nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ---------------------------------------------------------------------------
// The gated-update posture.
// ---------------------------------------------------------------------------

/// The rename UPDATE template (a "smart update" in GraphDB terms).
fn rename_def() -> Value {
    json!({
        "text": "DELETE { ?s <http://ex/name> ?old } INSERT { ?s <http://ex/name> ?new } \
                 WHERE { ?s <http://ex/name> ?old . FILTER(?s = ?who) }",
        "parameters": { "who": "iri", "new": "string" }
    })
}

#[tokio::test]
async fn invoke_update_applies_through_writer_path() {
    let base = spawn(true).await;
    put_template(&base, "rename", &rename_def()).await;
    put_template(&base, "friends", &friends_def()).await;
    put_template(
        &base,
        "name-of",
        &json!({
            "text": "SELECT ?n WHERE { ?who <http://ex/name> ?n }",
            "parameters": { "who": "iri" }
        }),
    )
    .await;
    // A hostile bound literal: under concatenation this would break out and DROP ALL.
    let hostile = r#"x" } ; DROP ALL ; INSERT DATA { <http://ex/evil> <http://ex/p> <http://ex/o> } # "#;
    let resp = client()
        .post(format!("{base}/templates/rename"))
        .json(&json!({"who": "http://ex/alice", "new": hostile}))
        .send()
        .await
        .unwrap();
    // Same status contract as a /sparql update: 204 on success.
    assert_eq!(resp.status(), 204);
    // The mutation is visible and the hostile text is DATA (alice's name is now the
    // verbatim string; bob is untouched, nothing was dropped).
    let names: Value = client()
        .post(format!("{base}/templates/name-of"))
        .json(&json!({"who": "http://ex/alice"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bindings = names["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["n"]["value"], hostile);
    let bob: Value = client()
        .post(format!("{base}/templates/name-of"))
        .json(&json!({"who": "http://ex/bob"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bob["results"]["bindings"][0]["n"]["value"], "Bob");
}

#[tokio::test]
async fn update_template_and_store_writes_are_write_gated() {
    let config = ServerConfig {
        auth_token: Some("secret".to_string()),
        ..ServerConfig::default()
    };
    let base = spawn_with(true, config).await;
    // Storing a template is a write: 401 without the token, 201 with it.
    let resp = put_template(&base, "rename", &rename_def()).await;
    assert_eq!(resp.status(), 401);
    let resp = client()
        .put(format!("{base}/templates/rename"))
        .bearer_auth("secret")
        .json(&rename_def())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let resp = client()
        .put(format!("{base}/templates/friends"))
        .bearer_auth("secret")
        .json(&friends_def())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    // Invoking the UPDATE template without the write token is refused (the same gate as
    // a /sparql update) — and nothing mutates.
    let resp = client()
        .post(format!("{base}/templates/rename"))
        .json(&json!({"who": "http://ex/alice", "new": "X"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    // A QUERY template invocation stays a read (no read token is configured).
    let resp = client()
        .post(format!("{base}/templates/friends"))
        .json(&json!({"who": "http://ex/alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // With the token the update applies.
    let resp = client()
        .post(format!("{base}/templates/rename"))
        .bearer_auth("secret")
        .json(&json!({"who": "http://ex/alice", "new": "X"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    // DELETE of a template is a write too.
    let resp = client()
        .delete(format!("{base}/templates/rename"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ---------------------------------------------------------------------------
// Persistence.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn templates_file_survives_restart_and_fails_closed() {
    let dir = std::env::temp_dir().join(format!("sparq-tpl-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("templates.json");
    let config = ServerConfig {
        templates: true,
        templates_file: Some(file.clone()),
        ..ServerConfig::default()
    };
    let base = spawn_with(true, config.clone()).await;
    assert_eq!(put_template(&base, "friends", &friends_def()).await.status(), 201);
    assert!(file.exists(), "PUT must persist the store");
    // A fresh server over the same file serves the stored template.
    let base2 = spawn_with(true, config.clone()).await;
    let resp = client()
        .post(format!("{base2}/templates/friends"))
        .json(&json!({"who": "http://ex/alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // DELETE persists the removal.
    client()
        .delete(format!("{base2}/templates/friends"))
        .send()
        .await
        .unwrap();
    let store_now = std::fs::read_to_string(&file).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&store_now).unwrap(), json!([]));
    // A corrupt file is a fail-closed STARTUP error, never a silently-empty store.
    std::fs::write(&file, "not json").unwrap();
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let err = match AppState::try_with_config(graph, config) {
        Err(e) => e,
        Ok(_) => panic!("a corrupt templates file must be a startup error"),
    };
    assert!(err.contains("not valid JSON"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
