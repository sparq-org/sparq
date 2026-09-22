//! Integration tests for the SPARQL 1.1 Protocol + Graph Store HTTP Protocol (read side).
//!
//! Each test spins the actual axum server on an ephemeral port in-process and drives it
//! over real HTTP with `reqwest`, asserting the protocol's request forms, exact result
//! media types, payload shapes, ASK booleans and HTTP status semantics. This is structured
//! so the official W3C SPARQL Protocol test suite could be pointed at the running endpoint
//! (see the crate README for how to run conformance).
//!
//! [OPUS-4.8] (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
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

/// Boots the server on a random local port and returns its base URL.
async fn spawn() -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
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

// ---------------------------------------------------------------------------
// Query operation — request forms
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_query_json_default() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
    let body = resp.text().await.unwrap();
    assert!(body.contains("\"head\""));
    assert!(body.contains("\"bindings\""));
    // three subjects have ex:age
    assert_eq!(body.matches("\"type\":\"uri\"").count(), 3);
}

#[tokio::test]
async fn post_direct_sparql_query() {
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
    let body = resp.text().await.unwrap();
    assert!(body.contains("\"bindings\""));
}

#[tokio::test]
async fn post_urlencoded_query() {
    let base = spawn().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .form(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
}

// ---------------------------------------------------------------------------
// Result format content negotiation
// ---------------------------------------------------------------------------

async fn select_with_accept(base: &str, accept: &str) -> reqwest::Response {
    client()
        .get(format!("{base}/sparql"))
        .header("accept", accept)
        .query(&[("query", "SELECT ?s ?a WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn negotiate_xml() {
    let base = spawn().await;
    let resp = select_with_accept(&base, "application/sparql-results+xml").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+xml"
    );
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("<?xml"));
    assert!(body.contains("xmlns=\"http://www.w3.org/2005/sparql-results#\""));
    assert!(body.contains("<variable name=\"s\"/>"));
}

#[tokio::test]
async fn negotiate_csv() {
    let base = spawn().await;
    let resp = select_with_accept(&base, "text/csv").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/csv; charset=utf-8");
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("s,a\r\n"));
}

#[tokio::test]
async fn negotiate_tsv() {
    let base = spawn().await;
    let resp = select_with_accept(&base, "text/tab-separated-values").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "text/tab-separated-values; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("?s\t?a\n"));
    // sq-u79ee: SPARQL Results TSV abbreviates xsd:integer to its bare Turtle token.
    assert!(body.lines().any(|l| l.ends_with("\t30")), "integer not abbreviated: {body}");
    assert!(!body.contains("\"30\"^^"), "integer should not be quoted+typed: {body}");
}

// ---------------------------------------------------------------------------
// ASK
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ask_json_true_and_false() {
    let base = spawn().await;
    let t = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "ASK { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(t.status(), 200);
    assert_eq!(
        t.headers()["content-type"],
        "application/sparql-results+json"
    );
    assert_eq!(t.text().await.unwrap(), "{\"head\":{},\"boolean\":true}");

    let f = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "ASK { ?s <http://ex/nope> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(f.text().await.unwrap(), "{\"head\":{},\"boolean\":false}");
}

#[tokio::test]
async fn ask_xml() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+xml")
        .query(&[("query", "ASK { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+xml"
    );
    assert!(resp.text().await.unwrap().contains("<boolean>true</boolean>"));
}

// ---------------------------------------------------------------------------
// CONSTRUCT / DESCRIBE (T16)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn construct_ntriples_default() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[(
            "query",
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/n-triples; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    assert_eq!(body.lines().count(), 3);
    assert!(body.contains(
        "<http://ex/alice> <http://ex/years> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
    ));
}

#[tokio::test]
async fn construct_negotiates_turtle() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("accept", "text/turtle")
        .query(&[(
            "query",
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    // The body is N-Triples — a syntactic subset of Turtle, so it loads as Turtle.
    let body = resp.text().await.unwrap();
    assert_eq!(Graph::load_str(&body, "turtle").unwrap().len(), 3);
}

#[tokio::test]
async fn construct_via_post_and_empty_result() {
    let base = spawn().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("CONSTRUCT { ?s <http://ex/x> ?o } WHERE { ?s <http://ex/nope> ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), ""); // the empty graph
}

#[tokio::test]
async fn describe_returns_cbd() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "DESCRIBE <http://ex/alice>")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/n-triples; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    // alice's three outbound triples, nothing else.
    assert_eq!(body.lines().count(), 3);
    assert!(body.lines().all(|l| l.starts_with("<http://ex/alice>")));
}

#[tokio::test]
async fn head_construct_mirrors_get() {
    let base = spawn().await;
    let resp = client()
        .head(format!("{base}/sparql"))
        .query(&[("query", "DESCRIBE <http://ex/alice>")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/n-triples; charset=utf-8"
    );
    assert_eq!(resp.text().await.unwrap(), "");
}

// ---------------------------------------------------------------------------
// Streamed SELECT bodies (T16) — byte-identical to the engine's single string
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streamed_select_json_is_byte_identical() {
    let base = spawn().await;
    let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let advertised: usize = resp.headers()["content-length"].to_str().unwrap().parse().unwrap();
    let body = resp.text().await.unwrap();
    assert_eq!(body.len(), advertised, "Content-Length must match the streamed body");
    // Exactly the engine's single-string serialisation.
    let expect = sparq_engine::query_json(&Graph::load_str(DATA, "turtle").unwrap(), q).unwrap();
    assert_eq!(body, expect);
}

#[tokio::test]
async fn streamed_select_large_multichunk_is_byte_identical() {
    // >64 KiB of JSON forces a genuinely multi-chunk stream over real HTTP.
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..3000 {
        ttl.push_str(&format!(
            "ex:subject{i} ex:somePredicate \"value-{i}-padding-padding\" .\n"
        ));
    }
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    let expect = sparq_engine::query_json(&graph, "SELECT * WHERE { ?s ?p ?o }").unwrap();
    assert!(expect.len() > 64 * 1024);

    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let resp = client()
        .get(format!("http://{addr}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // [OPUS-4.8] (sq-7d3dj.34.2) A genuinely multi-chunk SELECT-JSON result now STREAMS: the
    // body is written under chunked transfer-encoding as the engine serialises it (so the
    // first byte is flushed before the whole result is built — TTFB), which means there is no
    // `Content-Length` (the total is not known until the last row). The buffered small-result
    // path still carries a `Content-Length` — see `streamed_select_json_is_byte_identical`.
    assert!(
        resp.headers().get("content-length").is_none(),
        "a streamed multi-chunk result must not advertise Content-Length (chunked transfer-encoding)"
    );
    let body = resp.text().await.unwrap();
    // The load-bearing invariant: the streamed body is byte-identical to the engine's single
    // materialised JSON string (same solution order, same escaping).
    assert_eq!(body, expect);
}

// ---------------------------------------------------------------------------
// Streamed CONSTRUCT / DESCRIBE bodies ([FABLE-5] sq-0kq6k)
//
// Same contract as the streamed SELECT above: a small result keeps its buffered
// `Content-Length` wire shape; a result too big for one chunk streams under chunked
// transfer-encoding; either way the bytes equal the buffered serialiser's output exactly.
// ---------------------------------------------------------------------------

/// A graph whose `?s ?p ?o` CONSTRUCT renders to well over the 64 KiB chunk threshold, so the
/// response is a genuinely multi-chunk stream over real HTTP.
fn big_graph_ttl() -> String {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..3000 {
        ttl.push_str(&format!(
            "ex:subject{i} ex:somePredicate \"value-{i}-padding-padding-padding\" .\n"
        ));
    }
    ttl
}

/// Boots a server over `ttl` and returns its base URL (the shared `spawn` uses `DATA`).
async fn spawn_over(ttl: &str) -> String {
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// The buffered rendering the streamed body must equal, computed independently of the server:
/// run the same CONSTRUCT on the engine, serialise with the public buffered writer.
fn expected_graph_body(ttl: &str, query: &str, fmt: &str) -> String {
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    let triples = sparq_engine::construct_or_describe(&graph, query).unwrap();
    match fmt {
        "turtle" => sparq_server::graph::triples_to_turtle(&triples),
        _ => sparq_server::graph::triples_to_ntriples(&triples),
    }
}

/// A CONSTRUCT small enough to fit one chunk keeps the pre-streaming wire shape: a
/// `Content-Length`, no chunked transfer-encoding, and the buffered serialiser's exact bytes.
#[tokio::test]
async fn small_construct_stays_buffered_with_content_length() {
    let base = spawn().await;
    let q = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let advertised: usize = resp.headers()["content-length"].to_str().unwrap().parse().unwrap();
    let body = resp.text().await.unwrap();
    assert_eq!(body.len(), advertised, "Content-Length must match the buffered body");
    assert_eq!(body, expected_graph_body(DATA, q, "ntriples"));
}

/// The load-bearing test: a CONSTRUCT too large for one chunk STREAMS (no `Content-Length`)
/// and the streamed bytes are identical to the buffered serialiser's output.
#[tokio::test]
async fn streamed_construct_large_multichunk_is_byte_identical() {
    let ttl = big_graph_ttl();
    let q = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
    let expect = expected_graph_body(&ttl, q, "ntriples");
    assert!(expect.len() > 64 * 1024, "the fixture must exceed one chunk");

    let base = spawn_over(&ttl).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "application/n-triples; charset=utf-8");
    assert!(
        resp.headers().get("content-length").is_none(),
        "a streamed multi-chunk graph result must not advertise Content-Length"
    );
    assert_eq!(resp.text().await.unwrap(), expect);
}

/// The same invariant through the prefix-compacting Turtle writer — the format whose
/// serialiser carries subject/predicate grouping state across chunk boundaries, so it is the
/// one that could actually differ if the chunking leaked into the rendering.
#[tokio::test]
async fn streamed_construct_turtle_is_byte_identical() {
    let ttl = big_graph_ttl();
    let q = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
    let expect = expected_graph_body(&ttl, q, "turtle");
    assert!(expect.len() > 64 * 1024, "the fixture must exceed one chunk");

    let base = spawn_over(&ttl).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("accept", "text/turtle")
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    let body = resp.text().await.unwrap();
    assert_eq!(body, expect);
    // …and it is still a Turtle document that re-parses to the same triple count.
    assert_eq!(Graph::load_str(&body, "turtle").unwrap().len(), 3000);
}

/// HEAD keeps the buffered path so it can still advertise the `Content-Length` the GET body
/// would have had — the documented "HEAD mirrors GET" contract survives the streaming change
/// even for a result that a GET would stream.
#[tokio::test]
async fn head_large_construct_still_advertises_content_length() {
    let ttl = big_graph_ttl();
    let q = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
    let expect = expected_graph_body(&ttl, q, "ntriples");

    let base = spawn_over(&ttl).await;
    let resp = client()
        .head(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let advertised: usize = resp.headers()["content-length"].to_str().unwrap().parse().unwrap();
    assert_eq!(advertised, expect.len(), "HEAD must advertise the GET body length");
    assert_eq!(resp.text().await.unwrap(), "", "HEAD carries no body");
}

/// A CONSTRUCT that matches nothing still answers `200` with an empty body and a zero
/// `Content-Length` — the empty document must not look like "the worker produced no stream".
#[tokio::test]
async fn empty_construct_is_200_with_empty_body() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "CONSTRUCT { ?s ?p ?o } WHERE { ?s <http://ex/nope> ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-length"], "0");
    assert_eq!(resp.text().await.unwrap(), "");
}

/// A budget refusal on a streamed-eligible CONSTRUCT is still a clean `413`, never a truncated
/// `200`: the engine materialises the whole graph before any byte is rendered, so the status is
/// always decided pre-first-byte.
#[tokio::test]
async fn large_construct_row_cap_is_a_clean_413() {
    use sparq_server::ServerConfig;
    let ttl = big_graph_ttl();
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    let config = ServerConfig { max_results: Some(10), ..ServerConfig::default() };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let resp = client()
        .get(format!("http://{addr}/sparql"))
        .query(&[("query", "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413, "a row-cap refusal must be a clean 413, not a truncated 200");
}

// ---------------------------------------------------------------------------
// HTTP semantics: 400 / 405 / 501 / HEAD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_query_is_400() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT WHERE {")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn missing_query_param_is_400() {
    let base = spawn().await;
    let resp = client().get(format!("{base}/sparql")).send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn unsupported_method_is_405_with_allow() {
    let base = spawn().await;
    let resp = client()
        .request(reqwest::Method::DELETE, format!("{base}/sparql"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    let allow = resp.headers()["allow"].to_str().unwrap();
    assert!(allow.contains("GET"));
    assert!(allow.contains("POST"));
}

#[tokio::test]
async fn sparql_update_insert_then_query() {
    let base = spawn().await;
    let cl = client();
    // INSERT DATA via the SPARQL 1.1 Protocol update operation -> 204 No Content.
    let resp = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/newS> <http://ex/newP> <http://ex/newO> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // The inserted triple is now visible to a query against the swapped-in graph.
    let body = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", "SELECT ?o WHERE { <http://ex/newS> <http://ex/newP> ?o }")])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("http://ex/newO"), "inserted object should be queryable: {body}");

    // A malformed update is a 400.
    let bad = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { not valid sparql")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn head_query_has_no_body_but_content_type() {
    let base = spawn().await;
    let resp = client()
        .head(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json"
    );
    assert_eq!(resp.text().await.unwrap(), "");
}

// ---------------------------------------------------------------------------
// Graph Store HTTP Protocol — READ side
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gsp_get_default_graph_indirect() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql/graph?default"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("application/n-triples"));
    let body = resp.text().await.unwrap();
    // dump must contain the triples in N-Triples syntax
    assert!(body.contains("<http://ex/alice> <http://ex/age>"));
    assert!(body.lines().all(|l| l.is_empty() || l.ends_with(" .")));
}

#[tokio::test]
async fn gsp_get_direct_graph_turtle_accept() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/graphs/mygraph"))
        .header("accept", "text/turtle")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/turtle"));
}

// ---------------------------------------------------------------------------
// Graph Store HTTP Protocol — WRITE side (sq-gxsj) [OPUS-4.8]
//
// PUT/POST/DELETE on graph resources, routed through the sequenced writer (the same
// path the application/sparql-update operation uses). Round-trip via GSP read.
// ---------------------------------------------------------------------------

/// Boots a server over an EMPTY default graph so write-side assertions start clean.
async fn spawn_empty() -> String {
    let graph = Graph::load_str("", "turtle").unwrap();
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// PUT a named graph, then GET it back: the round-trip must reproduce exactly the body.
#[tokio::test]
async fn gsp_put_then_get_roundtrip_named() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/g1";
    let body = "<http://ex/s> <http://ex/p> <http://ex/o> .\n";

    // First PUT into an absent graph → 201 Created.
    let put = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/n-triples")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 201, "first PUT into an absent graph must be 201 Created");

    // GET it back via the indirect form.
    let get = cl
        .get(format!("{base}/sparql/graph?graph={g}"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);
    let got = get.text().await.unwrap();
    assert!(got.contains("<http://ex/s> <http://ex/p> <http://ex/o> ."), "round-trip lost the triple: {got:?}");

    // The triple must NOT bleed into the default graph.
    let def = cl.get(format!("{base}/sparql/graph?default")).send().await.unwrap();
    assert_eq!(def.text().await.unwrap().trim(), "", "named-graph PUT leaked into the default graph");
}

/// A second PUT REPLACES the graph contents (not a merge) and returns 204 (graph existed).
#[tokio::test]
async fn gsp_put_replaces_existing_204() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/g2";

    assert_eq!(
        cl.put(format!("{base}/sparql/graph?graph={g}"))
            .header("content-type", "text/turtle")
            .body("<http://ex/a> <http://ex/p> <http://ex/1> .")
            .send().await.unwrap().status(),
        201
    );
    // Replace with a different triple.
    let second = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .body("<http://ex/b> <http://ex/p> <http://ex/2> .")
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 204, "PUT replacing an existing graph must be 204");

    let got = cl.get(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap().text().await.unwrap();
    assert!(got.contains("<http://ex/b>"), "replaced content missing: {got:?}");
    assert!(!got.contains("<http://ex/a>"), "PUT must REPLACE, not merge: {got:?}");
}

/// POST MERGES (additive) into a graph; two POSTs accumulate.
#[tokio::test]
async fn gsp_post_merges_additive() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/g3";

    // First POST into an absent graph creates it → 201.
    let p1 = cl
        .post(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/n-triples")
        .body("<http://ex/s1> <http://ex/p> <http://ex/o1> .")
        .send().await.unwrap();
    assert_eq!(p1.status(), 201);

    // Second POST adds to it → 204 (graph now exists).
    let p2 = cl
        .post(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/n-triples")
        .body("<http://ex/s2> <http://ex/p> <http://ex/o2> .")
        .send().await.unwrap();
    assert_eq!(p2.status(), 204);

    let got = cl.get(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap().text().await.unwrap();
    assert!(got.contains("<http://ex/s1>") && got.contains("<http://ex/s2>"), "POST must accumulate both triples: {got:?}");
}

/// DELETE drops a graph; a follow-up GET serialises it as empty, and a second DELETE 404s.
#[tokio::test]
async fn gsp_delete_then_get_and_404() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/g4";

    cl.put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/n-triples")
        .body("<http://ex/s> <http://ex/p> <http://ex/o> .")
        .send().await.unwrap();

    let del = cl.delete(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap();
    assert_eq!(del.status(), 204, "DELETE of an existing graph must be 204");

    // GET the now-dropped graph → 200, empty body (GSP read serves the empty graph).
    let get = cl.get(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap();
    assert_eq!(get.status(), 200);
    assert_eq!(get.text().await.unwrap().trim(), "");

    // Second DELETE of the now-absent graph → 404.
    let del2 = cl.delete(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap();
    assert_eq!(del2.status(), 404, "DELETE of an absent graph must be 404");
}

/// The default graph is addressable for writes via `?default`; DELETE empties it (204).
#[tokio::test]
async fn gsp_default_graph_write_and_delete() {
    let base = spawn().await; // pre-populated default graph (DATA)
    let cl = client();

    // POST merges into the default graph.
    let post = cl
        .post(format!("{base}/sparql/graph?default"))
        .header("content-type", "application/n-triples")
        .body("<http://ex/extra> <http://ex/p> <http://ex/v> .")
        .send().await.unwrap();
    assert!(post.status() == 204 || post.status() == 201, "default-graph POST status: {}", post.status());
    let got = cl.get(format!("{base}/sparql/graph?default")).send().await.unwrap().text().await.unwrap();
    assert!(got.contains("<http://ex/extra>"), "default-graph POST not visible: {got:?}");

    // DELETE (CLEAR DEFAULT) empties it → 204; the default graph always exists.
    let del = cl.delete(format!("{base}/sparql/graph?default")).send().await.unwrap();
    assert_eq!(del.status(), 204);
    assert_eq!(cl.get(format!("{base}/sparql/graph?default")).send().await.unwrap().text().await.unwrap().trim(), "");
}

/// Content negotiation by Content-Type: a Turtle body parses; an N-Quads body folds its
/// graph names into the addressed graph (the URL names the graph, the body carries triples).
#[tokio::test]
async fn gsp_write_content_negotiation() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/g5";

    // Turtle with a prefix.
    let put = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .body("@prefix ex: <http://ex/> . ex:s ex:p ex:o .")
        .send().await.unwrap();
    assert_eq!(put.status(), 201);
    let got = cl.get(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap().text().await.unwrap();
    assert!(got.contains("<http://ex/s> <http://ex/p> <http://ex/o> ."), "turtle body not parsed: {got:?}");
}

/// A malformed RDF body is a 400 (the body is parsed/validated before any write).
#[tokio::test]
async fn gsp_malformed_body_is_400() {
    let base = spawn_empty().await;
    let resp = client()
        .put(format!("{base}/sparql/graph?graph=http://ex/bad"))
        .header("content-type", "text/turtle")
        .body("this is not turtle <<< ]")
        .send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

/// An unsupported write body Content-Type is a 415.
#[tokio::test]
async fn gsp_unsupported_media_type_is_415() {
    let base = spawn_empty().await;
    let resp = client()
        .put(format!("{base}/sparql/graph?graph=http://ex/x"))
        .header("content-type", "application/json")
        .body("{}")
        .send().await.unwrap();
    assert_eq!(resp.status(), 415);
}

/// Direct identification (request URI is the graph) round-trips, addressable also via the
/// indirect `?graph=<reconstructed-iri>` form.
#[tokio::test]
async fn gsp_put_direct_then_get_direct() {
    let base = spawn_empty().await;
    let cl = client();

    let put = cl
        .put(format!("{base}/graphs/team/alpha"))
        .header("content-type", "application/n-triples")
        .body("<http://ex/s> <http://ex/p> <http://ex/o> .")
        .send().await.unwrap();
    assert_eq!(put.status(), 201);

    let got = cl.get(format!("{base}/graphs/team/alpha")).send().await.unwrap().text().await.unwrap();
    assert!(got.contains("<http://ex/s> <http://ex/p> <http://ex/o> ."), "direct round-trip lost the triple: {got:?}");
}

/// A selector-less POST to the GSP endpoint creates a fresh, server-named graph (GSP §5.5):
/// 201, and the new graph is queryable (its triples show up across all named graphs).
#[tokio::test]
async fn gsp_post_no_selector_creates_fresh_graph() {
    let base = spawn_empty().await;
    let cl = client();
    let resp = cl
        .post(format!("{base}/sparql/graph"))
        .header("content-type", "application/n-triples")
        .body("<http://ex/fresh> <http://ex/p> <http://ex/o> .")
        .send().await.unwrap();
    assert_eq!(resp.status(), 201, "selector-less POST must create a fresh graph (201)");

    // The triple lives in SOME named graph now — visible via a GRAPH ?g query.
    let q = cl
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?g WHERE { GRAPH ?g { <http://ex/fresh> ?p ?o } }")])
        .send().await.unwrap().text().await.unwrap();
    assert!(q.contains("\"bindings\"") && q.contains("http://ex/fresh") || q.matches("urn:sparq:gsp").count() >= 1,
        "fresh-graph triple not queryable: {q}");
}

/// An unsupported method (not GET/HEAD/PUT/POST/DELETE/PATCH) on a graph resource is a 405 with an
/// Allow header listing the supported GSP verbs — now INCLUDING PATCH ([OPUS-4.8] sq-hj4n, gh-916:
/// PATCH is a real GSP method, not a 405). The full PATCH behaviour is exercised in
/// `tests/gsp_patch.rs`.
#[tokio::test]
async fn gsp_unsupported_method_is_405_with_allow() {
    let base = spawn().await;
    let resp = client()
        .request(reqwest::Method::OPTIONS, format!("{base}/sparql/graph?default"))
        .send().await.unwrap();
    assert_eq!(resp.status(), 405);
    let allow = resp.headers()["allow"].to_str().unwrap();
    assert!(
        allow.contains("PUT")
            && allow.contains("POST")
            && allow.contains("DELETE")
            && allow.contains("GET")
            && allow.contains("PATCH"),
        "Allow must list all GSP verbs incl PATCH: {allow}"
    );
}

/// [OPUS-4.8] (sq-hj4n, gh-916) A PATCH on a graph resource is NO LONGER a 405 — it is a real GSP
/// method. A PATCH with no recognised body Content-Type is a 415 (the PATCH route was reached and
/// classified the body), proving PATCH is handled rather than method-not-allowed.
#[tokio::test]
async fn gsp_patch_is_handled_not_405() {
    let base = spawn().await;
    let resp = client()
        .request(reqwest::Method::PATCH, format!("{base}/sparql/graph?default"))
        .body("anything")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415, "PATCH is handled (415 unsupported body), not 405");
}

#[tokio::test]
async fn gsp_indirect_requires_graph_selector() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql/graph"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-z33x — SPARQL 1.1 Protocol §2.1.4 / §2.2 dataset-override params
// (`default-graph-uri` / `named-graph-uri`; update `using-graph-uri` / `using-named-graph-uri`)
// ---------------------------------------------------------------------------
//
// [OPUS-4.8] The engine DOES support named graphs — `GRAPH` / `FROM` / `FROM NAMED` and
// cross-graph joins all execute (see `tests/named_graphs.rs`, sq-fh4z). On top of that, the
// *protocol-level* dataset override — the `default-graph-uri` / `named-graph-uri` request
// parameters (and the update `using-graph-uri` / `using-named-graph-uri` forms) — now
// re-scopes the active dataset of the query/update per §2.1.4 / §2.2 (sq-z33x), as the
// tests below assert.

/// An RDF dataset with a default graph plus two named graphs, so the protocol dataset override
/// has something real to re-scope the active dataset to.
const NQ_DATASET: &str = r#"
    <http://ex/d> <http://ex/p> <http://ex/in_default> .
    <http://ex/a> <http://ex/p> <http://ex/in_g1> <http://ex/g1> .
    <http://ex/b> <http://ex/p> <http://ex/in_g2> <http://ex/g2> .
"#;

/// Boots a server over [`NQ_DATASET`] (a default graph + named graphs g1, g2).
async fn spawn_dataset() -> String {
    let graph = Graph::load_dataset(NQ_DATASET, "nquads").unwrap();
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Counts SELECT `?s ?p ?o` solution rows in a SPARQL-JSON body.
fn json_row_count(body: &str) -> usize {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    v["results"]["bindings"].as_array().map(|a| a.len()).unwrap_or(0)
}

/// `default-graph-uri=g1` re-scopes the active default graph to ONLY g1's triple — the store's
/// own default graph drops out (one row, the g1 triple; not the default-graph one).
#[tokio::test]
async fn default_graph_uri_rescopes_active_dataset() {
    let base = spawn_dataset().await;
    let cl = client();

    // Without the override: the store's default graph (1 triple).
    let plain = cl
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(plain.status(), 200);
    assert_eq!(json_row_count(&plain.text().await.unwrap()), 1);

    // With `default-graph-uri=g1`: the active default graph is g1 (still 1 triple, but it is g1's).
    let resp = cl
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
            ("default-graph-uri", "http://ex/g1"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(json_row_count(&body), 1);
    assert!(body.contains("http://ex/in_g1"), "expected g1's triple, got {body}");
    assert!(!body.contains("in_default"), "store default graph must drop out: {body}");
}

/// Two `default-graph-uri` values merge into the active default graph (g1 ∪ g2 = 2 triples).
#[tokio::test]
async fn multiple_default_graph_uri_merge() {
    let base = spawn_dataset().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
            ("default-graph-uri", "http://ex/g1"),
            ("default-graph-uri", "http://ex/g2"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(json_row_count(&resp.text().await.unwrap()), 2);
}

/// `named-graph-uri=g1` makes g1 the only named graph; the active default graph is empty, so a
/// `GRAPH ?g` query sees exactly g1's one triple and a default-graph BGP sees nothing.
#[tokio::test]
async fn named_graph_uri_rescopes_named_graphs() {
    let base = spawn_dataset().await;
    let cl = client();

    let in_graph = cl
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "SELECT ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }"),
            ("named-graph-uri", "http://ex/g1"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(in_graph.status(), 200);
    let body = in_graph.text().await.unwrap();
    assert_eq!(json_row_count(&body), 1);
    assert!(body.contains("http://ex/in_g1"), "got {body}");

    // The active default graph is empty under a named-only override.
    let in_default = cl
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"),
            ("named-graph-uri", "http://ex/g1"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(json_row_count(&in_default.text().await.unwrap()), 0);
}

/// Per §2.1.4 the protocol dataset REPLACES an in-query `FROM` clause: a query that says
/// `FROM <g2>` but is sent with `default-graph-uri=g1` runs against g1, not g2.
#[tokio::test]
async fn protocol_override_replaces_in_query_from() {
    let base = spawn_dataset().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "SELECT ?s ?p ?o FROM <http://ex/g2> WHERE { ?s ?p ?o }"),
            ("default-graph-uri", "http://ex/g1"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("http://ex/in_g1"), "override (g1) must win over in-query FROM g2: {body}");
    assert!(!body.contains("in_g2"), "in-query FROM g2 must be replaced: {body}");
}

/// The override also applies on the url-encoded POST form (body-carried params).
#[tokio::test]
async fn default_graph_uri_via_post_form() {
    let base = spawn_dataset().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("query=SELECT+%3Fs+%3Fp+%3Fo+WHERE+%7B+%3Fs+%3Fp+%3Fo+%7D&default-graph-uri=http://ex/g1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("http://ex/in_g1"), "POST-form override must re-scope: {body}");
}

/// A `default-graph-uri` that is not a valid absolute IRI is a 400 (caller-input validation).
#[tokio::test]
async fn bad_default_graph_uri_is_400() {
    let base = spawn_dataset().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "SELECT ?s WHERE { ?s ?p ?o }"),
            ("default-graph-uri", "not a valid iri"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// `using-graph-uri` re-scopes the WHERE clause of an UPDATE: an `INSERT … WHERE` that reads
/// from g1 (via the override) copies g1's triple into the default graph.
#[tokio::test]
async fn using_graph_uri_rescopes_update_where() {
    let base = spawn_dataset().await;
    let cl = client();

    // INSERT a marker into the default graph for every triple visible in the override's default
    // graph (g1). Without `using-graph-uri` this WHERE would read the store's default graph.
    let upd = cl
        .post(format!("{base}/sparql?using-graph-uri=http://ex/g1"))
        .header("content-type", "application/sparql-update")
        .body("INSERT { <http://ex/marker> <http://ex/saw> ?o } WHERE { ?s <http://ex/p> ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(upd.status(), 204, "update must succeed");

    // The marker must reference g1's object, proving the WHERE read g1, not the store default.
    let q = cl
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?o WHERE { <http://ex/marker> <http://ex/saw> ?o }")])
        .send()
        .await
        .unwrap();
    let body = q.text().await.unwrap();
    assert!(body.contains("http://ex/in_g1"), "USING g1 must re-scope the WHERE: {body}");
    assert!(!body.contains("in_default"), "WHERE must NOT read the store default graph: {body}");
}

/// Per §2.2 it is an error to supply `using-graph-uri` alongside an in-update `USING` clause: 400.
#[tokio::test]
async fn using_graph_uri_conflict_with_in_update_using_is_400() {
    let base = spawn_dataset().await;
    let resp = client()
        .post(format!("{base}/sparql?using-graph-uri=http://ex/g1"))
        .header("content-type", "application/sparql-update")
        .body(
            "INSERT { <http://ex/m> <http://ex/p> ?o } USING <http://ex/g2> \
             WHERE { ?s <http://ex/p> ?o }",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "USING + using-graph-uri must be a protocol error");
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-rt6v — RDF/XML body parsing + RDF/XML & prefix-Turtle serialisation
// ---------------------------------------------------------------------------

/// A simple, self-contained RDF/XML document with one triple (uses a registered prefix).
const RDFXML_BODY: &str = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:foaf="http://xmlns.com/foaf/0.1/">
  <rdf:Description rdf:about="http://ex/alice">
    <foaf:name>Alice</foaf:name>
  </rdf:Description>
</rdf:RDF>"#;

/// PUT an `application/rdf+xml` body, then GET the graph back as RDF/XML: the round-trip must
/// preserve the triple (parse RDF/XML in → serialise RDF/XML out, both via oxrdfxml).
#[tokio::test]
async fn gsp_put_rdfxml_then_get_rdfxml_roundtrip() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/rdfxml-g";

    let put = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "application/rdf+xml")
        .body(RDFXML_BODY)
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 201, "first PUT of an RDF/XML body into an absent graph must be 201");

    // GET it back, asking for RDF/XML.
    let get = cl
        .get(format!("{base}/sparql/graph?graph={g}"))
        .header("accept", "application/rdf+xml")
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);
    assert_eq!(get.headers()["content-type"], "application/rdf+xml; charset=utf-8");
    let body = get.text().await.unwrap();
    assert!(body.contains("<?xml"), "RDF/XML response must be a real XML document: {body}");
    assert!(body.contains("rdf:RDF"), "RDF/XML response must have an rdf:RDF root: {body}");

    // The recovered RDF/XML must re-parse to exactly the inserted triple.
    let triples = sparq_server::graph::parse_rdfxml(body.as_bytes(), None).unwrap();
    assert_eq!(triples.len(), 1, "RDF/XML round-trip must preserve the single triple: {body}");
    assert_eq!(triples[0].subject.to_string(), "<http://ex/alice>");
    assert_eq!(triples[0].predicate.as_str(), "http://xmlns.com/foaf/0.1/name");

    // And it must also be readable as N-Triples (default Accept) — same one triple.
    let nt = cl.get(format!("{base}/sparql/graph?graph={g}")).send().await.unwrap().text().await.unwrap();
    assert!(nt.contains("<http://ex/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" ."), "N-Triples view lost the triple: {nt}");
}

/// A malformed `application/rdf+xml` body is a 400 (parsed/validated before any write).
#[tokio::test]
async fn gsp_malformed_rdfxml_body_is_400() {
    let base = spawn_empty().await;
    let resp = client()
        .put(format!("{base}/sparql/graph?graph=http://ex/bad-xml"))
        .header("content-type", "application/rdf+xml")
        .body("<rdf:RDF><this is not well-formed xml")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "malformed RDF/XML must be a 400");
}

/// CONSTRUCT with `Accept: application/rdf+xml` returns a well-formed RDF/XML document that
/// re-parses to the constructed triples.
#[tokio::test]
async fn construct_negotiates_rdfxml() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("accept", "application/rdf+xml")
        .query(&[(
            "query",
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "application/rdf+xml; charset=utf-8");
    let body = resp.text().await.unwrap();
    assert!(body.contains("<?xml") && body.contains("rdf:RDF"), "not RDF/XML: {body}");
    // Three subjects have ex:age in DATA → three constructed triples.
    let triples = sparq_server::graph::parse_rdfxml(body.as_bytes(), None).unwrap();
    assert_eq!(triples.len(), 3, "RDF/XML CONSTRUCT result must carry all three triples: {body}");
}

/// CONSTRUCT with `Accept: text/turtle` returns PREFIX-COMPACTING Turtle (a real `@prefix`
/// header + compact `prefix:local` IRIs), not N-Triples-as-Turtle.
#[tokio::test]
async fn construct_turtle_is_prefix_compacting() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("accept", "text/turtle")
        .query(&[(
            "query",
            // rdf:type into an owl: class exercises two registered prefixes.
            "PREFIX ex: <http://ex/> PREFIX owl: <http://www.w3.org/2002/07/owl#> \
             CONSTRUCT { ?s a owl:Thing } WHERE { ?s ex:age ?a }",
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    let body = resp.text().await.unwrap();
    // A genuine Turtle document declares prefixes and uses the compact form.
    assert!(body.contains("@prefix owl:"), "Turtle must declare the owl prefix: {body}");
    assert!(body.contains("owl:Thing"), "Turtle must compact the owl: IRI: {body}");
    // And it must still load as Turtle to the same triple count.
    assert_eq!(Graph::load_str(&body, "turtle").unwrap().len(), 3);
}

/// GSP read negotiates RDF/XML when the client asks for it (via the q-value-aware Accept).
#[tokio::test]
async fn gsp_read_negotiates_rdfxml() {
    let base = spawn().await; // pre-populated default graph (DATA)
    let resp = client()
        .get(format!("{base}/sparql/graph?default"))
        .header("accept", "application/rdf+xml;q=0.9, application/n-triples;q=0.5")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "application/rdf+xml; charset=utf-8");
    let body = resp.text().await.unwrap();
    assert!(body.contains("rdf:RDF"), "GSP RDF/XML read must be RDF/XML: {body}");
    // The default graph (DATA): alice(knows/age/name)=3, bob(age/name)=2, carol(age)=1 → 6.
    let triples = sparq_server::graph::parse_rdfxml(body.as_bytes(), None).unwrap();
    assert_eq!(triples.len(), 6, "GSP RDF/XML read must carry every default-graph triple: {body}");
}

/// A Turtle body PUT round-trips through an RDF/XML GET (cross-format): write Turtle, read
/// RDF/XML — content negotiation is independent on each side.
#[tokio::test]
async fn gsp_put_turtle_get_rdfxml_cross_format() {
    let base = spawn_empty().await;
    let cl = client();
    let g = "http://ex/cross";

    let put = cl
        .put(format!("{base}/sparql/graph?graph={g}"))
        .header("content-type", "text/turtle")
        .body("@prefix ex: <http://ex/> . ex:s ex:p ex:o .")
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 201);

    let get = cl
        .get(format!("{base}/sparql/graph?graph={g}"))
        .header("accept", "application/rdf+xml")
        .send()
        .await
        .unwrap();
    assert_eq!(get.headers()["content-type"], "application/rdf+xml; charset=utf-8");
    let body = get.text().await.unwrap();
    let triples = sparq_server::graph::parse_rdfxml(body.as_bytes(), None).unwrap();
    assert_eq!(triples.len(), 1, "cross-format round-trip lost the triple: {body}");
    assert_eq!(triples[0].object.to_string(), "<http://ex/o>");
}
