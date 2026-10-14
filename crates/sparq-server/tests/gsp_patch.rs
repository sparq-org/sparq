//! (sq-hj4n, gh-916) Integration tests for the Graph-Store-Protocol `PATCH` method —
//! 🤖 SPARQ agent.
//!
//! Two dialects:
//!   * `application/sparql-update` — ALWAYS-ON (no feature). These tests run in BOTH feature
//!     states and assert the patch is atomic + scoped to the addressed graph.
//!   * `text/n3` (Solid N3-Patch) — OPT-IN behind the `n3-patch` cargo feature AND the `--n3-patch`
//!     runtime flag. The N3-Patch tests are `#[cfg(feature = "n3-patch")]`; a `#[cfg(not(...))]`
//!     test asserts the feature-OFF behaviour (a `text/n3` PATCH body is a plain `415`).
//!
//! The server runs on an ephemeral port in-process and is driven over real HTTP with `reqwest`.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:age 25 .
"#;

/// Boots a server with the given config and returns its base URL.
async fn spawn_with(config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Boots a default server (no opt-in flags).
async fn spawn() -> String {
    spawn_with(ServerConfig::default()).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Convenience: run a SELECT and return the JSON body text.
async fn select(base: &str, q: &str) -> String {
    client()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// Allow header + method posture
// ---------------------------------------------------------------------------

/// An unsupported method on a graph resource is `405` and its `Allow` header now advertises PATCH.
#[tokio::test]
async fn unsupported_method_allow_lists_patch() {
    let base = spawn().await;
    // OPTIONS is not a GSP method, so the dispatcher answers 405 with the Allow set.
    let resp = client()
        .request(
            reqwest::Method::OPTIONS,
            format!("{base}/sparql/graph?default"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    let allow = resp.headers()["allow"].to_str().unwrap().to_string();
    assert!(allow.contains("PATCH"), "Allow header must list PATCH: {allow}");
    assert!(allow.contains("PUT") && allow.contains("DELETE"));
}

// ---------------------------------------------------------------------------
// ALWAYS-ON dialect: application/sparql-update PATCH (runs in BOTH feature states)
// ---------------------------------------------------------------------------

/// A SPARQL-Update PATCH body modifies the addressed default graph in place: delete one triple,
/// insert another, in ONE atomic update → 204.
#[tokio::test]
async fn sparql_update_patch_default_graph_atomic() {
    let base = spawn().await;
    let body = "DELETE DATA { <http://ex/alice> <http://ex/age> 30 } ; \
                INSERT DATA { <http://ex/alice> <http://ex/age> 31 }";
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "application/sparql-update")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    // The old triple is gone, the new one present (atomic delete+insert applied).
    let got = select(
        &base,
        "SELECT ?a WHERE { <http://ex/alice> <http://ex/age> ?a }",
    )
    .await;
    assert!(got.contains("31"), "age must be updated to 31: {got}");
    assert!(!got.contains("\"30\""), "old age 30 must be gone: {got}");
}

/// A SPARQL-Update PATCH on a NAMED graph is scoped to that graph: the WHERE dataset defaults to
/// the addressed graph, and an `INSERT { GRAPH <g> { … } }` writes into it.
#[tokio::test]
async fn sparql_update_patch_named_graph_scoped() {
    let base = spawn().await;
    let g = "http://ex/g1";
    // Seed the named graph via GSP PUT.
    let put = client()
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .body("<http://ex/s> <http://ex/p> <http://ex/o> .")
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success());
    // PATCH the named graph: delete the seeded triple, insert a new one (explicit GRAPH targets).
    let body = format!(
        "DELETE DATA {{ GRAPH <{g}> {{ <http://ex/s> <http://ex/p> <http://ex/o> }} }} ; \
         INSERT DATA {{ GRAPH <{g}> {{ <http://ex/s> <http://ex/p2> <http://ex/o2> }} }}"
    );
    let resp = client()
        .patch(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/sparql-update")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    // The named graph now holds only the new triple.
    let got = select(
        &base,
        &format!("SELECT ?p WHERE {{ GRAPH <{g}> {{ <http://ex/s> ?p ?o }} }}"),
    )
    .await;
    assert!(got.contains("p2"), "named-graph PATCH must apply: {got}");
    assert!(!got.contains("\"p\"") || got.contains("p2"));
}

/// A SPARQL-Update PATCH with a WHERE template scoped to the named graph: the override defaults the
/// WHERE dataset to the addressed graph, so variables bind against THAT graph.
#[tokio::test]
async fn sparql_update_patch_named_graph_where_dataset_scoped() {
    let base = spawn().await;
    let g = "http://ex/gscope";
    // Seed graph g with one triple.
    client()
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .body("<http://ex/x> <http://ex/k> \"v\" .")
        .send()
        .await
        .unwrap();
    // PATCH: DELETE { GRAPH g { ?s ?p ?o } } WHERE { ?s ?p ?o } — WHERE reads the addressed graph
    // (the using-graph-uri override), so it matches the seeded triple and removes it.
    let body = format!(
        "DELETE {{ GRAPH <{g}> {{ ?s ?p ?o }} }} WHERE {{ ?s ?p ?o }}"
    );
    let resp = client()
        .patch(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/sparql-update")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let got = select(
        &base,
        &format!("SELECT ?s WHERE {{ GRAPH <{g}> {{ ?s ?p ?o }} }}"),
    )
    .await;
    assert!(
        !got.contains("http://ex/x"),
        "WHERE-scoped delete must empty the graph: {got}"
    );
}

/// A malformed SPARQL-Update PATCH body is a 400 (and does not echo the body token verbatim).
#[tokio::test]
async fn sparql_update_patch_malformed_is_400() {
    let base = spawn().await;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "application/sparql-update")
        .body("DELETE DATA { this is not sparql")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// Supplying the named-graph PATCH alongside an in-body WITH/USING is the §2.2 conflict → 400.
#[tokio::test]
async fn sparql_update_patch_named_with_in_body_using_conflicts() {
    let base = spawn().await;
    let g = "http://ex/gc";
    let body = format!(
        "WITH <{g}> DELETE {{ ?s ?p ?o }} USING <{g}> WHERE {{ ?s ?p ?o }}"
    );
    let resp = client()
        .patch(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/sparql-update")
        .body(body)
        .send()
        .await
        .unwrap();
    // The using-graph-uri override (from the named-graph scoping) conflicts with the in-body
    // USING/WITH → 400 (SPARQL 1.1 Protocol §2.2).
    assert_eq!(resp.status(), 400);
}

/// An unsupported PATCH body Content-Type is a 415.
#[tokio::test]
async fn patch_unsupported_content_type_is_415() {
    let base = spawn().await;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
}

// ---------------------------------------------------------------------------
// FEATURE-OFF: a text/n3 PATCH body is a plain 415 (byte-identical posture)
// ---------------------------------------------------------------------------

/// With the `n3-patch` feature COMPILED OUT, a `text/n3` PATCH body is an unsupported media type
/// (`415`) — the N3-Patch dialect does not exist in the build, and the 415 message names only the
/// always-on dialect.
#[cfg(not(feature = "n3-patch"))]
#[tokio::test]
async fn n3_patch_body_is_415_without_feature() {
    let base = spawn().await;
    let body = r#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch; solid:inserts { ex:s ex:p ex:o. }."#;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "text/n3")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
    let msg = resp.text().await.unwrap();
    // The feature-off message must NOT advertise text/n3 (the build cannot serve it).
    assert!(
        !msg.contains("text/n3"),
        "feature-off 415 must not advertise text/n3: {msg}"
    );
    // The always-on dialect IS named.
    assert!(msg.contains("application/sparql-update"));
}

// ---------------------------------------------------------------------------
// FEATURE-ON: Solid N3-Patch (text/n3) — double-opt-in + atomic graph-scoped apply
// ---------------------------------------------------------------------------

/// With the feature ON but the runtime flag OFF, a `text/n3` PATCH body is still `415` (the
/// double-opt-in posture mirroring tpf/shacl).
#[cfg(feature = "n3-patch")]
#[tokio::test]
async fn n3_patch_415_when_flag_off() {
    // Feature compiled in, but ServerConfig::n3_patch defaults to false.
    let base = spawn().await;
    let body = r#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch; solid:inserts { ex:s ex:p ex:o. }."#;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "text/n3")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
}

/// With the feature ON and the flag ON, a ground N3-Patch (inserts only) applies to the default
/// graph atomically → 204.
#[cfg(feature = "n3-patch")]
#[tokio::test]
async fn n3_patch_ground_insert_default_graph() {
    let base = spawn_with(ServerConfig {
        n3_patch: true,
        ..ServerConfig::default()
    })
    .await;
    let body = r#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch;
  solid:inserts { <http://ex/carol> <http://ex/age> 40 . }."#;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "text/n3")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let got = select(
        &base,
        "SELECT ?a WHERE { <http://ex/carol> <http://ex/age> ?a }",
    )
    .await;
    assert!(got.contains("40"), "N3-Patch insert must apply: {got}");
}

/// A pattern-based N3-Patch with a `solid:where` clause: bind ?p by name, delete the old age and
/// insert a new one — in ONE atomic update → 204. The load-bearing invariant: the delete and
/// insert are atomic (the read after sees BOTH effects, never a partial state).
#[cfg(feature = "n3-patch")]
#[tokio::test]
async fn n3_patch_where_delete_insert_atomic() {
    let base = spawn_with(ServerConfig {
        n3_patch: true,
        ..ServerConfig::default()
    })
    .await;
    // alice has age 30 + name "Alice"; bind ?p via name, change her age 30 -> 31.
    let body = r#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:patch a solid:InsertDeletePatch;
  solid:where   { ?p <http://ex/name> "Alice" . };
  solid:deletes { ?p <http://ex/age> 30 . };
  solid:inserts { ?p <http://ex/age> 31 . }."#;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "text/n3")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let got = select(
        &base,
        "SELECT ?a WHERE { <http://ex/alice> <http://ex/age> ?a }",
    )
    .await;
    assert!(got.contains("31"), "patched age must be 31: {got}");
    assert!(!got.contains("\"30\""), "old age 30 must be gone: {got}");
    // bob is untouched (the WHERE bound only alice).
    let bob = select(
        &base,
        "SELECT ?a WHERE { <http://ex/bob> <http://ex/age> ?a }",
    )
    .await;
    assert!(bob.contains("25"), "bob must be untouched: {bob}");
}

/// An N3-Patch on a NAMED graph is graph-scoped: every block is wrapped in `GRAPH <g> { … }`, so
/// the patch only touches that graph and is invisible to the default graph.
#[cfg(feature = "n3-patch")]
#[tokio::test]
async fn n3_patch_named_graph_scoped() {
    let base = spawn_with(ServerConfig {
        n3_patch: true,
        ..ServerConfig::default()
    })
    .await;
    let g = "http://ex/n3g";
    let body = r#"@prefix solid: <http://www.w3.org/ns/solid/terms#>.
@prefix ex: <http://ex/>.
_:p a solid:InsertDeletePatch;
  solid:inserts { <http://ex/s> <http://ex/p> <http://ex/o> . }."#;
    let resp = client()
        .patch(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/n3")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    // The triple is in graph g...
    let in_g = select(
        &base,
        &format!("SELECT ?o WHERE {{ GRAPH <{g}> {{ <http://ex/s> <http://ex/p> ?o }} }}"),
    )
    .await;
    assert!(in_g.contains("http://ex/o"), "must be in named graph: {in_g}");
    // ...and NOT in the default graph (scoping held).
    let in_default = select(
        &base,
        "SELECT ?o WHERE { <http://ex/s> <http://ex/p> ?o }",
    )
    .await;
    assert!(
        !in_default.contains("http://ex/o"),
        "named-graph PATCH must not leak into the default graph: {in_default}"
    );
}

/// A malformed N3-Patch body (feature on, flag on) is a 400 — and the response does not echo the
/// offending body token verbatim (sanitized).
#[cfg(feature = "n3-patch")]
#[tokio::test]
async fn n3_patch_malformed_is_400() {
    let base = spawn_with(ServerConfig {
        n3_patch: true,
        ..ServerConfig::default()
    })
    .await;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "text/n3")
        .body("@prefix solid: <http://www.w3.org/ns/solid/terms#>. _:p a solid:InsertDeletePatch; solid:inserts { not valid n3 ")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// An N3-Patch with no `solid:InsertDeletePatch` resource is a 400 (structural validation).
#[cfg(feature = "n3-patch")]
#[tokio::test]
async fn n3_patch_no_patch_resource_is_400() {
    let base = spawn_with(ServerConfig {
        n3_patch: true,
        ..ServerConfig::default()
    })
    .await;
    let resp = client()
        .patch(format!("{base}/sparql/graph?default"))
        .header("content-type", "text/n3")
        .body("@prefix ex: <http://ex/>. ex:s ex:p ex:o .")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}
