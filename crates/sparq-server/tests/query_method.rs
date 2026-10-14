//! Integration tests for the HTTP `QUERY` method on the SPARQL Protocol endpoint
//! (sq-b3df9, epic sq-my8wd, w3c/sparql-protocol#40).
//!
//! The `QUERY` verb is a query-ONLY input path that feeds the SAME downstream query
//! execution + Accept-based content negotiation as GET/POST. These tests drive the real
//! axum server over real HTTP using exactly the bytes an Oxigraph client would send, so
//! the two interoperate:
//!   * the raw SPARQL string IS the body under `Content-Type: application/sparql-query`;
//!   * OR the `query=` parameter of an `application/x-www-form-urlencoded` body;
//!   * `default-graph-uri` / `named-graph-uri` come from the URL query string;
//!   * Accept negotiates SRJ / SRX / CSV / TSV (SELECT/ASK) and RDF (CONSTRUCT/DESCRIBE),
//!     reusing the SAME negotiation as GET/POST. A PRESENT-but-unsatisfiable Accept (naming no
//!     supported result format and no wildcard) is `406 Not Acceptable`, matching Oxigraph
//!     (sq-406acc); an absent / empty / `*/*` Accept still defaults to JSON;
//!   * an `update=` form field is a 400 and `application/sparql-update` is a 415 (query-only).
//!
//! Gate the whole suite on the `server` feature (it spins the real axum server
//! and uses the `server`-gated `router` / `AppState` API). 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:age 25 ; ex:name "Bob"@en .
    ex:carol ex:age 35 .
"#;

const NQ_DATASET: &str = r#"
    <http://ex/d> <http://ex/p> <http://ex/in_default> .
    <http://ex/a> <http://ex/p> <http://ex/in_g1> <http://ex/g1> .
    <http://ex/b> <http://ex/p> <http://ex/in_g2> <http://ex/g2> .
"#;

/// Boots the server on a random local port and returns its base URL.
async fn spawn() -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    spawn_with(graph).await
}

/// Boots a server over [`NQ_DATASET`] (a default graph + named graphs g1, g2).
async fn spawn_dataset() -> String {
    let graph = Graph::load_dataset(NQ_DATASET, "nquads").unwrap();
    spawn_with(graph).await
}

async fn spawn_with(graph: Graph) -> String {
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// The custom HTTP method token an Oxigraph client sends (oxhttp passes "QUERY" through;
/// reqwest carries it the same way via `Method::from_bytes`).
fn query_method() -> reqwest::Method {
    reqwest::Method::from_bytes(b"QUERY").unwrap()
}

fn json_row_count(body: &str) -> usize {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    v["results"]["bindings"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// (1) Content-Type: application/sparql-query — raw query IS the body
// ---------------------------------------------------------------------------

/// The canonical Oxigraph-interop form: `QUERY /sparql` with the raw SPARQL string as the
/// `application/sparql-query` body, negotiating SPARQL-results JSON. Same downstream
/// execution as POST, so the same three `ex:age` subjects come back.
#[tokio::test]
async fn query_method_direct_body_json() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", "application/sparql-results+json")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
    let body = resp.text().await.unwrap();
    assert_eq!(json_row_count(&body), 3, "three subjects have ex:age: {body}");
}

/// A `Content-Type` with parameters (`; charset=utf-8`) still matches — the server strips
/// everything after `;` and lowercases, exactly like Oxigraph.
#[tokio::test]
async fn query_method_content_type_with_charset_param() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query; charset=utf-8")
        .body("ASK { <http://ex/alice> <http://ex/knows> <http://ex/bob> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("true"));
}

// ---------------------------------------------------------------------------
// (2) Content-Type: application/x-www-form-urlencoded — query= in the body
// ---------------------------------------------------------------------------

/// The url-encoded form carries the query as the `query=` body parameter (Oxigraph merges
/// body+URL `query` parameters; here it is in the body).
#[tokio::test]
async fn query_method_urlencoded_form() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .form(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(json_row_count(&resp.text().await.unwrap()), 3);
}

// ---------------------------------------------------------------------------
// Accept negotiation — SRJ / SRX / CSV / TSV (SELECT/ASK) and RDF (CONSTRUCT)
// ---------------------------------------------------------------------------

async fn select_with_accept(base: &str, accept: &str) -> reqwest::Response {
    client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", accept)
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn query_method_accept_xml() {
    let base = spawn().await;
    let resp = select_with_accept(&base, "application/sparql-results+xml").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+xml"
    );
    assert!(resp.text().await.unwrap().contains("<sparql"));
}

#[tokio::test]
async fn query_method_accept_csv() {
    let base = spawn().await;
    let resp = select_with_accept(&base, "text/csv").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/csv; charset=utf-8");
}

#[tokio::test]
async fn query_method_accept_tsv() {
    let base = spawn().await;
    let resp = select_with_accept(&base, "text/tab-separated-values").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "text/tab-separated-values; charset=utf-8"
    );
}

/// No Accept header → SPARQL-results JSON for solutions (the protocol default), NOT a 406.
#[tokio::test]
async fn query_method_no_accept_defaults_to_json() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
}

/// CONSTRUCT under QUERY negotiates an RDF format on Accept (Turtle here). sparq's graph
/// serialiser emits an N-Triples body (a syntactic subset of Turtle) under the negotiated
/// `text/turtle` Content-Type — identical to the GET/POST path
/// (`protocol::construct_negotiates_turtle`), so the QUERY input path reuses it unchanged.
#[tokio::test]
async fn query_method_construct_rdf() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", "text/turtle")
        .body("PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    // The body is N-Triples (a Turtle subset), so it loads as Turtle: alice/bob/carol's ages.
    let body = resp.text().await.unwrap();
    assert_eq!(Graph::load_str(&body, "turtle").unwrap().len(), 3);
    assert!(body.contains("<http://ex/years>"), "got {body}");
}

/// sq-406acc: an Accept naming only unsupported solution media types (and no
/// wildcard) is now `406 Not Acceptable`, matching Oxigraph (w3c/sparql-protocol#40) instead of
/// the previous silent JSON fallback. The QUERY input path reuses the SAME negotiation as
/// GET/POST, so this stricter behaviour is consistent across all three query input paths.
#[tokio::test]
async fn query_method_unsupported_accept_is_406() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", "image/png")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 406, "present-but-unsatisfiable Accept → 406");
}

/// sq-406acc: the load-bearing converse — an ABSENT Accept (and `*/*`) still returns
/// the SPARQL-results JSON default via QUERY, NOT a 406. The W3C SPARQL Protocol permits a
/// default representation when Accept is absent, so the strictness change must not regress this.
#[tokio::test]
async fn query_method_absent_or_wildcard_accept_defaults_to_json() {
    let base = spawn().await;
    // No Accept header at all → JSON default, 200.
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "absent Accept → JSON default, not 406");
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
    // `Accept: */*` → JSON default, 200 (a wildcard is satisfiable).
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", "*/*")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "*/* Accept → JSON default, not 406");
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
}

// ---------------------------------------------------------------------------
// Dataset graph params — from the URL query string (NOT the body)
// ---------------------------------------------------------------------------

/// `default-graph-uri=g1` (URL query string) re-scopes the active default graph to g1, even
/// when the query is sent as the raw `application/sparql-query` body — the graph params come
/// from the URL, never the body (Oxigraph's `url_query_parameters`).
#[tokio::test]
async fn query_method_default_graph_uri_from_url() {
    let base = spawn_dataset().await;
    let resp = client()
        .request(
            query_method(),
            format!("{base}/sparql?default-graph-uri=http://ex/g1"),
        )
        .header("content-type", "application/sparql-query")
        .header("accept", "application/sparql-results+json")
        .body("SELECT ?s ?p ?o WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(json_row_count(&body), 1);
    assert!(body.contains("http://ex/in_g1"), "expected g1's triple: {body}");
    assert!(!body.contains("in_default"), "store default graph must drop out: {body}");
}

/// The dataset params are read from the URL query string even for the url-encoded FORM body
/// (Oxigraph reads graph params via `url_query_parameters`, not the body, on QUERY).
#[tokio::test]
async fn query_method_named_graph_uri_from_url_with_form_body() {
    let base = spawn_dataset().await;
    let resp = client()
        .request(
            query_method(),
            format!("{base}/sparql?named-graph-uri=http://ex/g1"),
        )
        .header("accept", "application/sparql-results+json")
        .form(&[("query", "SELECT ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(json_row_count(&body), 1, "only g1 is a named graph: {body}");
    assert!(body.contains("http://ex/in_g1"), "got {body}");
}

// ---------------------------------------------------------------------------
// Query-only rejections — update= (400) and application/sparql-update (415)
// ---------------------------------------------------------------------------

/// An `update=` form field under QUERY → 400 (Oxigraph: "SPARQL updates are not compatible
/// with the QUERY HTTP method"). The data must NOT change.
#[tokio::test]
async fn query_method_form_update_is_400() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .form(&[(
            "update",
            "INSERT DATA { <http://ex/x> <http://ex/y> <http://ex/z> }",
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// `Content-Type: application/sparql-update` under QUERY → 415 (the `&& !is_query` guard
/// fails and it falls through to unsupported-media-type, NOT a 400/403). Under POST the same
/// body is a valid update — see `protocol::sparql_update_insert_then_query`, unchanged.
#[tokio::test]
async fn query_method_sparql_update_content_type_is_415() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/x> <http://ex/y> <http://ex/z> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
}

// ---------------------------------------------------------------------------
// Other status semantics — body / Content-Type edge cases
// ---------------------------------------------------------------------------

/// Empty body with `application/sparql-query` → empty query string → parse failure → 400.
#[tokio::test]
async fn query_method_empty_body_is_400() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// A malformed query string → 400.
#[tokio::test]
async fn query_method_malformed_query_is_400() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("SELECT WHERE {")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// A wrong/unknown Content-Type (`text/plain`) → 415 (only `application/sparql-query` and
/// `application/x-www-form-urlencoded` are accepted, matching Oxigraph).
#[tokio::test]
async fn query_method_wrong_content_type_is_415() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "text/plain")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
}

/// A url-encoded form body with neither `query` nor `update` → 400.
#[tokio::test]
async fn query_method_form_missing_query_is_400() {
    let base = spawn().await;
    let resp = client()
        .request(query_method(), format!("{base}/sparql"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("foo=bar")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ---------------------------------------------------------------------------
// No regression to GET/POST — QUERY is additive
// ---------------------------------------------------------------------------

/// A still-unsupported method (DELETE) on `/sparql` stays a 405 — adding QUERY did not open
/// the door to arbitrary verbs.
#[tokio::test]
async fn query_method_does_not_open_arbitrary_verbs() {
    let base = spawn().await;
    let resp = client()
        .request(reqwest::Method::DELETE, format!("{base}/sparql"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
}

/// POST query still works after the QUERY arm was added (regression guard).
#[tokio::test]
async fn post_query_still_works_alongside_query_method() {
    let base = spawn().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", "application/sparql-results+json")
        .body("SELECT ?s WHERE { ?s <http://ex/age> ?a }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(json_row_count(&resp.text().await.unwrap()), 3);
}
