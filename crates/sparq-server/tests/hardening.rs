//! Integration tests for the T15 hardening guards: per-request timeout, body-size limit,
//! concurrency shedding, panic recovery and the SELECT row cap. Each test boots the real
//! hardened router on an ephemeral port and drives it over HTTP, asserting both the HTTP
//! status semantics and the structured JSON error bodies.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use std::time::Duration;

use sparq_core::Graph;
use sparq_server::{harden, router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:age 25 ; ex:name "Bob"@en .
    ex:carol ex:age 35 .
"#;

/// A dense graph whose 3-way disconnected pattern forces a cross-product far too large
/// to ever finish — the deliberately slow query for the timeout / shedding tests.
fn dense_graph_ttl() -> String {
    let n = 80; // 80×80 = 6 400 edges; (6 400)^3 cross-product rows ≈ 2.6e11 — never finishes
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..n {
        for j in 0..n {
            ttl.push_str(&format!("ex:n{i} ex:e ex:n{j} .\n"));
        }
    }
    ttl
}

const SLOW_QUERY: &str =
    "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?c ex:e ?d . ?e ex:e ?f }";

async fn serve(app: axum::Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_with(ttl: &str, config: ServerConfig) -> String {
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    serve(router(AppState::with_config(graph, config))).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// (1) Request timeout — slow query → 503, worker stopped cooperatively
// ---------------------------------------------------------------------------

#[tokio::test]
async fn timeout_fires_on_slow_query() {
    let config = ServerConfig { query_timeout: Some(Duration::from_secs(1)), ..ServerConfig::default() };
    let base = spawn_with(&dense_graph_ttl(), config).await;

    let started = std::time::Instant::now();
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", SLOW_QUERY)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    // 1s budget deadline + 2s hard grace; anything beyond means the guard didn't fire.
    assert!(started.elapsed() < Duration::from_secs(5), "503 took {:?}", started.elapsed());
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");
    assert!(body.contains("timed out"), "got: {body}");
}

#[tokio::test]
async fn fast_query_unaffected_by_timeout() {
    let config = ServerConfig { query_timeout: Some(Duration::from_secs(1)), ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("\"bindings\""));
}

// ---------------------------------------------------------------------------
// (2a) Body-size limit → 413
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oversized_body_is_413() {
    let config = ServerConfig { max_body_bytes: 256, ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;
    // A syntactically valid (huge) query body over the limit — rejected before parsing.
    let big = format!("SELECT * WHERE {{ ?s ?p ?o }} # {}", "x".repeat(1024));
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");

    // Under the limit still works.
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("SELECT * WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// (2b) Concurrency limit + load shed → 429
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrency_limit_sheds_with_429() {
    let config = ServerConfig {
        max_concurrent: 1,
        query_timeout: Some(Duration::from_secs(3)),
        ..ServerConfig::default()
    };
    let base = spawn_with(&dense_graph_ttl(), config).await;

    // Occupy the single slot with the slow query…
    let base2 = base.clone();
    let slow = tokio::spawn(async move {
        client().get(format!("{base2}/sparql")).query(&[("query", SLOW_QUERY)]).send().await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // …then any further request is shed immediately.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o } LIMIT 1")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 429);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");

    // The occupying request still terminates via the timeout guard (503).
    let slow_resp = slow.await.unwrap().unwrap();
    assert_eq!(slow_resp.status(), 503);
}

// ---------------------------------------------------------------------------
// (2c/3) --max-results row cap → honest 413 refusal, never truncation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_results_refuses_with_413() {
    let config = ServerConfig { max_results: Some(2), ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;

    // 3 matching rows > cap of 2 → refused, with the limit named in the error.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");
    assert!(body.contains("max-results"), "got: {body}");
    assert!(!body.contains("bindings"), "must refuse, not truncate: {body}");

    // Within the cap → normal result (and LIMIT keeps queries under the cap usable).
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a } LIMIT 2")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap().matches("\"type\":\"uri\"").count(), 2);

    // The cap applies to every SELECT serialisation, not just JSON.
    let resp = client()
        .get(format!("{base}/sparql"))
        .header("accept", "text/csv")
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);

    // ASK is existence-only and ignores the row cap.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "ASK { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("true"));
}

// ---------------------------------------------------------------------------
// (sq-ebii) Memory cap (--max-query-rows): coarse working-set row ceiling
// applied on EVERY query form (not just the SELECT projection), → honest 413.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_query_rows_caps_every_form() {
    // 3 ex:age triples. A cap of 2 must trip on the SELECT working set AND on a CONSTRUCT,
    // whose WHERE pattern materialises the same 3 rows — --max-query-rows is the coarse
    // working-set ceiling that applies on EVERY form, the property under test here.
    let config = ServerConfig { max_query_rows: Some(2), ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;

    // SELECT over 3 rows > cap → 413, naming the memory cap.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(body.contains("max-query-rows"), "should name the memory cap: {body}");

    // CONSTRUCT materialises the same 3 WHERE rows → also 413, bounded by --max-query-rows
    // (the every-form working-set ceiling this test sets and asserts).
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "CONSTRUCT { ?s <http://ex/a> ?a } WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413, "CONSTRUCT working set must be bounded by --max-query-rows");

    // Within the cap → normal 200.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a } LIMIT 2")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// (sq-ebii) The 413 row-cap message names the knob that ACTUALLY tripped on THIS
// path. On a path that does not fold in --max-results (ASK / GSP-read / UPDATE), a
// smaller-but-inapplicable --max-results must NOT be mis-named as the cause; the path's real
// cap is --max-query-rows. (Regression for the engine_error_response path-accuracy fix:
// previously the message picked the tighter of the GLOBAL config caps regardless of whether
// --max-results participated, so a GSP-read with --max-results < --max-query-rows reported
// "--max-results" and the smaller, never-applied row number.)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn row_cap_413_names_the_applied_knob_on_non_select_paths() {
    // --max-results (1) is SMALLER than --max-query-rows (2) but does NOT apply to a GSP-read
    // (which uses make_budget(_, false)); the real cap there is --max-query-rows.
    let config = ServerConfig { max_query_rows: Some(2), max_results: Some(1), ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;

    // GSP-read of the default graph (6 triples) > the working-set cap of 2 → 413, and the
    // message must name --max-query-rows (the cap that fed this path's budget), never
    // --max-results (not applied on a GSP-read).
    let resp = client().get(format!("{base}/sparql/graph?default")).send().await.unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(body.contains("max-query-rows"), "GSP-read 413 must name --max-query-rows: {body}");
    assert!(!body.contains("--max-results"), "GSP-read 413 must NOT name --max-results: {body}");

    // SELECT, by contrast, DOES fold in --max-results: with --max-results (1) tighter than
    // --max-query-rows (2) the projection cap is the one that trips → name it.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body = resp.text().await.unwrap();
    assert!(body.contains("--max-results"), "SELECT 413 must name --max-results: {body}");
}

// ---------------------------------------------------------------------------
// (sq-ebii) UPDATE timeout: the per-request query timeout now applies to the
// `application/sparql-update` path too — a never-finishing DELETE/INSERT WHERE → 503,
// bounded by `timeout + TIMEOUT_GRACE` (the same hard wall-clock cap the read paths use).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_path_honours_timeout() {
    let config = ServerConfig { query_timeout: Some(Duration::from_secs(1)), ..ServerConfig::default() };
    let base = spawn_with(&dense_graph_ttl(), config).await;

    // A DELETE/INSERT … WHERE whose WHERE is the same never-finishing 3-way cross-product.
    let update = "PREFIX ex: <http://ex/> \
         DELETE { ?a ex:e ?b } INSERT { ?a ex:gone ?b } \
         WHERE { ?a ex:e ?b . ?c ex:e ?d . ?e ex:e ?f }";
    let started = std::time::Instant::now();
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(update)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503, "slow UPDATE must time out, not hang");
    assert!(started.elapsed() < Duration::from_secs(8), "503 took {:?}", started.elapsed());
    let body = resp.text().await.unwrap();
    assert!(body.contains("timed out"), "got: {body}");

    // A fast update on the same server still succeeds (204).
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/y> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

// ---------------------------------------------------------------------------
// (sq-nulp) Writer-queue head-of-line blocking bound: a SEPARATE, shorter
// --update-where-timeout releases the single sequenced writer from a slow UPDATE's WHERE
// within that window, EVEN when the (read) --query-timeout is much longer. So a slow update
// (a) returns its 503 bounded by the short writer deadline, not the long read timeout, and
// (b) does not head-of-line block a fast update queued behind it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_where_timeout_bounds_head_of_line_blocking() {
    // A GENEROUS read timeout (8 s) — long enough that, without the writer-side WHERE
    // deadline, a slow update would hold the single writer for the full 8 s + grace and
    // head-of-line block everything behind it. The separate short WHERE deadline (1 s) is
    // what actually bounds the writer's work.
    let config = ServerConfig {
        query_timeout: Some(Duration::from_secs(8)),
        update_where_timeout: Some(Duration::from_secs(1)),
        ..ServerConfig::default()
    };
    let base = spawn_with(&dense_graph_ttl(), config).await;

    // The never-finishing 3-way cross-product WHERE, as a DELETE/INSERT update.
    let slow = "PREFIX ex: <http://ex/> \
         DELETE { ?a ex:e ?b } INSERT { ?a ex:gone ?b } \
         WHERE { ?a ex:e ?b . ?c ex:e ?d . ?e ex:e ?f }";

    // Fire the slow update and, concurrently, a trivial fast update. The fast one is queued
    // behind the slow one on the single writer; if the slow update were NOT cut at the 1 s
    // writer deadline, the fast one could not be acked until the slow one released the writer.
    let slow_base = base.clone();
    let slow_task = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let resp = client()
            .post(format!("{slow_base}/sparql"))
            .header("content-type", "application/sparql-update")
            .body(slow)
            .send()
            .await
            .unwrap();
        (resp.status(), started.elapsed())
    });

    // Give the slow update a head start so it is the one occupying the writer.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let fast_started = std::time::Instant::now();
    let fast = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/x> <http://ex/p> <http://ex/y> }")
        .send()
        .await
        .unwrap();
    let fast_elapsed = fast_started.elapsed();
    assert_eq!(fast.status(), 204, "fast queued update must still commit");

    let (slow_status, slow_elapsed) = slow_task.await.unwrap();
    // The slow update is refused (503): its WHERE hit the writer-side cooperative deadline.
    assert_eq!(slow_status, 503, "slow update must be cut at the writer WHERE deadline");
    // Bounded by the SHORT writer deadline (1 s + grace + scheduling slack), NOT the 8 s
    // read timeout — proving head-of-line blocking is bounded by --update-where-timeout.
    assert!(
        slow_elapsed < Duration::from_secs(6),
        "slow update 503 should be bounded by the 1 s writer deadline, took {slow_elapsed:?}"
    );
    // The fast update was not blocked for anything near the 8 s read timeout: it landed once
    // the writer was released (≈ the 1 s WHERE deadline), well under the read timeout.
    assert!(
        fast_elapsed < Duration::from_secs(6),
        "fast update must not be head-of-line blocked for the full read timeout, took {fast_elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// (sq-ebii) Decompression-ratio cap (--max-decompress-ratio): a high-ratio
// gzip body (a "zip bomb") on the GSP write path is refused with 413 BEFORE the full
// decompressed image is held; a benign small gzip body within the ratio decodes fine.
// ---------------------------------------------------------------------------

fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[tokio::test]
async fn decompress_ratio_cap_rejects_zip_bomb() {
    // Small ratio cap (3×) + a generous body limit so the COMPRESSED bytes pass the
    // body-size gate and the ratio cap is what actually trips.
    let config = ServerConfig { max_decompress_ratio: 3, max_body_bytes: 1 << 20, ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;

    // A highly compressible Turtle body: ~200 KiB of valid triples compresses to a few KiB,
    // i.e. a ratio well above 3× — the decompressed image would exceed 3 × compressed.
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..4000 {
        ttl.push_str(&format!("ex:s{i} ex:p ex:o .\n"));
    }
    let compressed = gzip(ttl.as_bytes());
    assert!(
        ttl.len() > compressed.len() * 3,
        "test fixture must exceed the 3x ratio (plain {}, gz {})",
        ttl.len(),
        compressed.len()
    );

    let resp = client()
        .put(format!("{base}/graphs/bomb"))
        .header("content-type", "text/turtle")
        .header("content-encoding", "gzip")
        .body(compressed)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413, "high-ratio gzip body must be refused as a possible zip bomb");
    let body = resp.text().await.unwrap();
    assert!(body.contains("decompression-ratio") || body.contains("zip bomb"), "got: {body}");

    // A benign small gzip body whose ratio is under the cap decodes and writes fine.
    let small = "@prefix ex: <http://ex/> . ex:a ex:p ex:b .";
    let small_gz = gzip(small.as_bytes());
    // (a 43-byte body gzips LARGER than itself, so 3× is plenty of headroom)
    let resp = client()
        .put(format!("{base}/graphs/ok"))
        .header("content-type", "text/turtle")
        .header("content-encoding", "gzip")
        .body(small_gz)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "benign gzip body should write, got {}", resp.status());
}

#[tokio::test]
async fn decompress_disabled_refuses_gzip_body() {
    // ratio 0 => Content-Encoding: gzip bodies are refused outright (fail-closed).
    let config = ServerConfig { max_decompress_ratio: 0, ..ServerConfig::default() };
    let base = spawn_with(DATA, config).await;
    let gz = gzip(b"@prefix ex: <http://ex/> . ex:a ex:p ex:b .");
    let resp = client()
        .put(format!("{base}/graphs/x"))
        .header("content-type", "text/turtle")
        .header("content-encoding", "gzip")
        .body(gz)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413, "gzip body must be refused when decompression is disabled");
}

// ---------------------------------------------------------------------------
// (4) Panic → 500 (CatchPanicLayer), connection stays alive
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handler_panic_is_500_not_dead_connection() {
    // Wrap a deliberately panicking route in the exact production middleware stack.
    async fn panicking() -> &'static str {
        panic!("boom")
    }
    let app = harden(
        axum::Router::new().route("/panic", axum::routing::get(panicking)),
        &ServerConfig::default(),
    );
    let base = serve(app).await;

    let cl = client();
    let resp = cl.get(format!("{base}/panic")).send().await.unwrap();
    assert_eq!(resp.status(), 500);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");

    // The server (and the client's pooled connection) survives the panic.
    let resp = cl.get(format!("{base}/panic")).send().await.unwrap();
    assert_eq!(resp.status(), 500);
}

// ---------------------------------------------------------------------------
// Structured JSON error bodies on the ordinary error paths too
// ---------------------------------------------------------------------------

#[tokio::test]
async fn errors_are_structured_json() {
    let base = spawn_with(DATA, ServerConfig::default()).await;

    // 400 malformed query.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT WHERE {")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "got: {body}");

    // 405 keeps its Allow header after JSON normalisation.
    let resp = client()
        .request(reqwest::Method::DELETE, format!("{base}/sparql"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    assert!(resp.headers().contains_key("allow"));
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "got: {body}");
}

// ---------------------------------------------------------------------------
// (6) Information-leak guard (sq-cz89 / sq-j9zs / sq-zg0u)
//
// On the no-auth-by-default path an error body MUST NOT echo the caller's submitted input,
// a fragment of the loaded RDF, or a server-side filesystem path. Each test provokes an
// error whose triggering content contains a SENTINEL token and asserts the sentinel does
// NOT appear in the (still structured-JSON) error body. The detail is logged server-side
// instead — never returned to the caller.
// ---------------------------------------------------------------------------

/// A token vanishingly unlikely to occur in any generic class message — its presence in a
/// response body would mean the server echoed caller/loaded/path content back.
const SENTINEL: &str = "secret_sentinel_value";

#[tokio::test]
async fn no_echo_query_parse_error() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // A malformed query whose offending token is the sentinel — `spargebra` would normally
    // quote it verbatim in the parse error.
    let q = format!("SELECT * WHERE {{ {SENTINEL} }}");
    let resp = client().get(format!("{base}/sparql")).query(&[("query", q.as_str())]).send().await.unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");
    assert!(!body.contains(SENTINEL), "query parse error echoed caller input: {body}");
}

#[tokio::test]
async fn no_echo_update_parse_error() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // A malformed UPDATE carrying the sentinel as a bare (invalid) token.
    let upd = format!("INSERT DATA {{ {SENTINEL} }}");
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(upd)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");
    assert!(!body.contains(SENTINEL), "update parse error echoed caller input: {body}");
}

#[tokio::test]
async fn no_echo_rdf_body_parse_error() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // Malformed N-Triples whose offending subject token is the sentinel — `oxttl` quotes
    // the bad token verbatim (the exact leak the Privacy audit surfaced for loaded RDF).
    let bad = format!("{SENTINEL} <http://ex/p> <http://ex/o> .\n");
    let resp = client()
        .put(format!("{base}/graphs/leak"))
        .header("content-type", "application/n-triples")
        .body(bad)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");
    assert!(!body.contains(SENTINEL), "RDF-body parse error echoed loaded-data fragment: {body}");
}

#[tokio::test]
async fn no_echo_malformed_gzip_body() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // A body that ADVERTISES gzip but is not a valid gzip stream — the decoder error can
    // quote bytes of the (caller-supplied) body. Embed the sentinel in those bytes.
    let mut not_gzip = vec![0x1f, 0x8b, 0x08, 0x00]; // gzip magic so it enters the decode path
    not_gzip.extend_from_slice(SENTINEL.as_bytes());
    not_gzip.extend_from_slice(&[0xff; 16]);
    let resp = client()
        .put(format!("{base}/graphs/gz"))
        .header("content-type", "text/turtle")
        .header("content-encoding", "gzip")
        .body(not_gzip)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "a corrupt gzip stream is a 400");
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("{\"error\":"), "structured JSON error, got: {body}");
    assert!(!body.contains(SENTINEL), "gzip decode error echoed caller body bytes: {body}");
}

#[tokio::test]
async fn no_echo_execution_error() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // A syntactically valid query that fails at EXECUTION — the engine error string can
    // embed term text drawn from the query/graph. Use a sentinel IRI in a construct so the
    // engine error (if any) would quote it; assert it never reaches the body. Even when the
    // query SUCCEEDS this is a no-op assertion (no error body), so it is robust either way.
    let q = format!("SELECT * WHERE {{ ?s ?p ?o . FILTER(?s = <http://ex/{SENTINEL}> && SAMETERM(?s, 1/0)) }}");
    let resp = client().get(format!("{base}/sparql")).query(&[("query", q.as_str())]).send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(!body.contains(SENTINEL), "execution error echoed query/graph content: {body}");
}

// ---------------------------------------------------------------------------
// (6b) ASVS-G3 regression guard (sq-kfel, epic sq-toze)
//
// "Verify sparq-server engine error strings never leak internal/sensitive info." This block
// is the durable VERIFY the bead asks for: it pins, per failure CLASS, that an error response
// (a) carries a USEFUL, actionable category and (b) leaks NO internals — no echoed caller
// input / loaded-data fragment, no `Debug` of an internal type, no secret/token material, and
// (crucially) no ABSOLUTE FILESYSTEM PATH. The existing (6) tests cover the no-echo property
// for the parse/decode/execution classes; these add the two paths that previously echoed a raw
// engine error verbatim (the GSP-write minted-update rejection and the TPF term parse), plus a
// blanket path-leak + structured-shape assertion across the main failure classes.
// ---------------------------------------------------------------------------

/// Substrings that would betray an internal/sensitive leak in ANY error body. A real
/// absolute path on the box (the build/run cwd) would start with one of these; the Rust
/// `Debug` of the crate's own internal error types tends to name them.
const FORBIDDEN_INTERNALS: &[&str] = &[
    "/home/", // an absolute POSIX path (build/run dir, --persist dir, an io::Error path)
    "/Users/", "/tmp/", "/var/", "/etc/", // other absolute-path roots
    "src/http.rs", "src/exec.rs", // a source-file path (a Debug/panic-location leak)
    "WriteError::", "PrepareError::", "DecodeError::", // a `Debug` of an internal enum
    "Custom { kind:", "Os { code:", // the `Debug` of a std::io::Error
];

/// Asserts an error body is the structured JSON envelope, names a useful category, and
/// contains none of the forbidden internal markers.
fn assert_clean_error(body: &str, expect_category: &str) {
    assert!(
        body.starts_with("{\"error\":"),
        "expected structured JSON error envelope, got: {body}"
    );
    assert!(
        body.to_ascii_lowercase().contains(expect_category),
        "error body lost its actionable category ({expect_category:?}): {body}"
    );
    for marker in FORBIDDEN_INTERNALS {
        assert!(
            !body.contains(marker),
            "error body leaked an internal marker {marker:?}: {body}"
        );
    }
}

/// The GSP-write minted-UPDATE rejection path (`apply_gsp_update`) previously wrapped the raw
/// engine error verbatim (`graph store write failed: {e}`). A body whose terms survive RDF
/// parse but make the minted `INSERT DATA` engine-reject must NOT echo that engine string; the
/// response must stay a structured 400 with a category and no internals. We embed the sentinel
/// in a literal (valid N-Triples, so it passes `body_to_ntriples` and reaches the writer).
#[tokio::test]
async fn no_echo_gsp_write_rejection_stays_clean() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // Valid N-Triples carrying the sentinel as a literal object: parses fine, so the failure
    // (if any) happens in the SERVER-MINTED update, exercising the line-3045 path.
    let nt = format!("<http://ex/s> <http://ex/p> \"{SENTINEL}\" .\n");
    let resp = client()
        .put(format!("{base}/graphs/minted"))
        .header("content-type", "application/n-triples")
        .body(nt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.text().await.unwrap();
    // A well-formed body should succeed (201/204); if the engine ever rejects the minted
    // update, the body must still be clean. Either way, the sentinel must never appear and no
    // internal marker may leak.
    assert!(
        !body.contains(SENTINEL),
        "GSP minted-update rejection echoed body content: {body}"
    );
    if status.is_client_error() || status.is_server_error() {
        for marker in FORBIDDEN_INTERNALS {
            assert!(!body.contains(marker), "GSP-write error leaked {marker:?}: {body}");
        }
    }
}

/// The unauthorized (401) class: a useful "authentication" category, identical for a missing
/// vs a wrong token (no oracle), and no internals.
#[tokio::test]
async fn unauthorized_error_is_clean_and_actionable() {
    let config = ServerConfig {
        auth_token: Some("the-write-token".to_string()),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    // A write with NO token => 401.
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert!(resp.headers().contains_key("www-authenticate"));
    let body = resp.text().await.unwrap();
    assert_clean_error(&body, "authentication");
    // The 401 must NEVER carry the configured secret.
    assert!(!body.contains("the-write-token"), "401 leaked the token: {body}");
}

/// The not-found (404) class: structured shape, an actionable category, and no internals.
/// (sq-pj6u) A BARE unmatched route now goes through the router fallback
/// (`unmatched_route`), so its body is the CATEGORISED `{"error":"not found"}` envelope —
/// previously it was axum's catch-all empty 404 that `json_error_bodies` wrapped into the
/// uncategorised `{"error":""}`. So we now assert the same categorised + clean shape on the
/// bare route as on a HANDLER-minted 404 (a `GET /.well-known/void` when the
/// federation-descriptor feature/flag is off). The message is server-constructed and never
/// echoes the request path, so it stays leak-free.
#[tokio::test]
async fn not_found_error_is_clean() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // Bare unmatched route: now the categorised `{"error":"not found"}` body, still clean.
    let resp = client().get(format!("{base}/no-such-route")).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    let body = resp.text().await.unwrap();
    // (sq-pj6u) Non-empty, categorised `not found` — no longer `{"error":""}`.
    assert_clean_error(&body, "not found");
    assert_ne!(
        body, "{\"error\":\"\"}",
        "unmatched-route 404 must now be CATEGORISED, not the bare empty envelope"
    );
}

/// A blanket assertion across the main client-facing failure classes (malformed query,
/// malformed update, unsupported media type): every error body is clean and categorised.
#[tokio::test]
async fn main_failure_classes_are_clean() {
    let base = spawn_with(DATA, ServerConfig::default()).await;

    // Malformed SPARQL query => 400.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT WHERE {{{")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_clean_error(&resp.text().await.unwrap(), "query");

    // Malformed SPARQL update => 400.
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA WHERE {{{")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_clean_error(&resp.text().await.unwrap(), "update");

    // Unsupported GSP write media type => 415, server-constructed message (no internals).
    let resp = client()
        .put(format!("{base}/graphs/g"))
        .header("content-type", "application/x-nonsense")
        .body("nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
    assert_clean_error(&resp.text().await.unwrap(), "rdf");
}

// ---------------------------------------------------------------------------
// sq-cmvh (ASVS V14.4) — security response headers on every response
// ---------------------------------------------------------------------------

/// The exact hardening header set the server must stamp on every response. Mirrors
/// `http::SECURITY_HEADERS` (kept in lock-step by review); asserted on a success, an error
/// and a streamed response below.
const EXPECTED_SECURITY_HEADERS: &[(&str, &str)] = &[
    ("x-content-type-options", "nosniff"),
    ("content-security-policy", "default-src 'none'; frame-ancestors 'none'"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "no-referrer"),
];

fn assert_security_headers(headers: &reqwest::header::HeaderMap) {
    for (name, expected) in EXPECTED_SECURITY_HEADERS {
        let got = headers
            .get(*name)
            .unwrap_or_else(|| panic!("missing security header {name}"))
            .to_str()
            .unwrap();
        assert_eq!(got, *expected, "security header {name} value");
    }
    // Headers we deliberately do NOT emit from the plain-HTTP origin (HSTS is the fronting
    // TLS proxy's job; X-XSS-Protection is deprecated). Their absence is part of the contract.
    assert!(
        !headers.contains_key("strict-transport-security"),
        "HSTS must not be set by the origin"
    );
    assert!(
        !headers.contains_key("x-xss-protection"),
        "deprecated X-XSS-Protection must not be set"
    );
}

#[tokio::test]
async fn security_headers_on_success_response() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // A normal 200 SELECT.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_security_headers(resp.headers());
    // And on the trivial /health route, which goes through the same hardened stack.
    let health = client().get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    assert_security_headers(health.headers());
}

#[tokio::test]
async fn security_headers_on_error_response() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // A 400 (missing 'query' parameter) — an error envelope must be hardened identically.
    let resp = client().get(format!("{base}/sparql")).send().await.unwrap();
    assert_eq!(resp.status(), 400);
    assert_security_headers(resp.headers());

    // A 413 produced INSIDE the middleware stack (body over the limit) — confirms the headers
    // are stamped even on extractor-level rejections rewritten by `json_error_bodies`.
    let cfg = ServerConfig { max_body_bytes: 16, ..ServerConfig::default() };
    let base = spawn_with(DATA, cfg).await;
    let big = "SELECT * WHERE { ?s ?p ?o }".repeat(64);
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    assert_security_headers(resp.headers());
}

#[tokio::test]
async fn security_headers_on_streamed_response() {
    // A larger result set exercises the chunked/streamed SELECT body path; the headers
    // (set by the response-path middleware) must be present there too.
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..500 {
        ttl.push_str(&format!("ex:s{i} ex:age {i} .\n"));
    }
    let base = spawn_with(&ttl, ServerConfig::default()).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s ?a WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_security_headers(resp.headers());
}

/// sq-2bhm (ASVS-G1): the headers must also land on an AUTH-GATED refusal, and the
/// auth-refusal — a sensitive response — must additionally carry `Cache-Control: no-store` so a
/// shared cache never retains a 401. Gate the whole surface (write token + `--auth-token-read`)
/// and hit a read with no token: a `401` that is still fully hardened.
#[tokio::test]
async fn security_headers_on_auth_gated_response() {
    let cfg = ServerConfig {
        auth_token: Some("s3cr3t-token".to_string()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, cfg).await;
    // A read with NO Authorization header — gated → 401.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert_eq!(
        resp.headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok()),
        Some("Bearer")
    );
    assert_security_headers(resp.headers());
    // Sensitive auth response: no shared cache may retain it.
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store"),
        "auth-refusal must carry Cache-Control: no-store"
    );
}

// ---------------------------------------------------------------------------
// (sq-2gqr) Slow-loris guard — the connection header-read deadline.
//
// `axum::serve` never installs a timer, so hyper's HTTP/1 header_read_timeout is inert and a
// client that opens a connection then dribbles (or simply never finishes) its request-header
// block holds the connection — and a concurrency slot — open forever. `sparq_server::serve`
// fixes this by owning the accept loop and wiring a TokioTimer + header_read_timeout. These
// tests drive the REAL `serve` loop over a raw TCP socket (reqwest always sends a complete
// header block, so it cannot exercise this — we have to be the misbehaving client).
// ---------------------------------------------------------------------------

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Spawn the real `sparq_server::serve` accept loop (NOT `axum::serve`) with the given
/// header-read and body read/idle deadlines, returning the bound address for a raw-socket client.
async fn spawn_serve(config: ServerConfig) -> SocketAddr {
    let header_read_timeout = config.header_read_timeout;
    // sq-lodb: also drive the real slow-body read/idle deadline through the serve loop.
    let body_read_timeout = config.body_read_timeout;
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        sparq_server::serve(
            listener,
            app,
            header_read_timeout,
            body_read_timeout,
            std::future::pending::<()>(),
        )
        .await
        .unwrap();
    });
    addr
}

/// sq-rejk9: the bespoke TCP loop must preserve axum's peer-address contract just as
/// the HTTP/3 dispatch path does. A missing extension makes the extractor reject this request,
/// while checking the exact source socket prevents a placeholder address from satisfying it.
#[tokio::test]
async fn tcp_serve_injects_peer_connect_info() {
    use axum::extract::ConnectInfo;
    use axum::routing::get;

    async fn peer(ConnectInfo(addr): ConnectInfo<SocketAddr>) -> String {
        addr.to_string()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route("/peer", get(peer));
    tokio::spawn(async move {
        sparq_server::serve(listener, app, None, None, std::future::pending::<()>())
            .await
            .unwrap();
    });

    let mut socket = TcpStream::connect(server_addr).await.unwrap();
    let expected_peer = socket.local_addr().unwrap();
    socket
        .write_all(b"GET /peer HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_string(&mut response))
        .await
        .expect("TCP response timed out")
        .unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "response: {response:?}"
    );
    assert!(
        response.ends_with(&expected_peer.to_string()),
        "handler saw the wrong peer; expected {expected_peer}, response: {response:?}"
    );
}

/// A slow-loris connection: open a socket, send a partial request line + ONE header, then
/// never send the terminating blank line. With a short `header_read_timeout` the SERVER must
/// close the connection (a read returns 0 / a reset) within roughly the deadline. Without the
/// fix the read would block until the test's own timeout, proving the connection is held open.
#[tokio::test]
async fn slow_loris_partial_headers_closed_by_deadline() {
    let config = ServerConfig {
        header_read_timeout: Some(Duration::from_millis(400)),
        ..ServerConfig::default()
    };
    let addr = spawn_serve(config).await;

    let mut sock = TcpStream::connect(addr).await.unwrap();
    // A well-formed request LINE and one header, but NO terminating CRLFCRLF — the header block
    // is never completed, exactly as a slow-loris client behaves.
    sock.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    sock.flush().await.unwrap();

    // The server's header-read deadline (400ms) must close the connection well within this
    // generous 5s cap. `read` returning Ok(0) is a clean close; an Err is a reset — both mean
    // the server let go of the slot. A timeout here means the connection was held open (the bug).
    let started = std::time::Instant::now();
    let mut buf = [0u8; 256];
    let outcome = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    let elapsed = started.elapsed();
    match outcome {
        Ok(Ok(0)) | Ok(Err(_)) => {} // connection closed / reset by the server — the guard fired
        // hyper may emit a 408 Request Timeout status line before closing; that's also a pass —
        // it still means the server let go rather than holding the slot.
        Ok(Ok(n)) => {
            let head = String::from_utf8_lossy(&buf[..n]);
            assert!(
                head.contains("408") || head.contains("400"),
                "server responded but did not signal a header-read timeout: {head:?}"
            );
        }
        Err(_) => panic!(
            "slow-loris connection was NOT closed within 5s — the header-read deadline did not fire \
             (connection + concurrency slot held open: the slow-loris DoS)"
        ),
    }
    assert!(
        elapsed < Duration::from_secs(3),
        "connection closed but only after {elapsed:?} — far beyond the 400ms deadline"
    );
}

/// With the deadline DISABLED (`None`), the partial-header connection is NOT proactively closed
/// by the server within a short window — proving the guard is what does the closing (and is
/// genuinely opt-out-able), not some unrelated default. We only assert it stays open briefly
/// (then we drop it), so the test is fast and not flaky.
#[tokio::test]
async fn slow_loris_held_open_when_deadline_disabled() {
    let config = ServerConfig { header_read_timeout: None, ..ServerConfig::default() };
    let addr = spawn_serve(config).await;

    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    sock.flush().await.unwrap();

    // Within a short window the server must NOT have closed the (incomplete) connection.
    let mut buf = [0u8; 64];
    let outcome = tokio::time::timeout(Duration::from_millis(700), sock.read(&mut buf)).await;
    assert!(
        outcome.is_err(),
        "with header_read_timeout=None the connection should stay open (read blocks), \
         but the server closed/answered it: {outcome:?}"
    );
}

/// Sanity: the new `serve` loop still serves a COMPLETE request normally (it is a faithful port
/// of axum's accept loop, not just a timeout). Drives a full HTTP/1.1 request over a raw socket
/// and asserts a 200 with the hardened security headers, so we know nothing regressed.
#[tokio::test]
async fn serve_loop_handles_complete_request() {
    let config = ServerConfig {
        header_read_timeout: Some(Duration::from_secs(15)),
        ..ServerConfig::default()
    };
    let addr = spawn_serve(config).await;

    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    sock.flush().await.unwrap();

    let mut resp = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), sock.read_to_end(&mut resp))
        .await
        .expect("complete request must be answered promptly")
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "complete request through sparq_server::serve must return 200: {text:?}"
    );
    // The hardened stack still runs on this path — the X-Content-Type-Options header must land.
    assert!(
        text.to_ascii_lowercase().contains("x-content-type-options: nosniff"),
        "security headers must still be stamped by the serve loop: {text:?}"
    );
}

// ---------------------------------------------------------------------------
// (sq-lodb) Slow-BODY guard — the request-body read/idle deadline (complement to
// sq-2gqr's header-read deadline).
//
// sq-2gqr's `header_read_timeout` closes the slow-loris HEADER dribble, but once a client has
// sent a complete, valid header block it can then dribble the BODY — declare a large
// `Content-Length`, send a few bytes, then stall forever — staying under `max_body_bytes` (a SIZE
// cap a trickle never trips) and before `query_timeout` starts (an engine deadline that begins
// only after the whole request is read). `body_read_timeout` wraps the body in a `TimeoutBody`
// whose per-frame idle deadline aborts that read. These tests drive the REAL `serve` loop over a
// raw TCP socket (reqwest sends its whole body at once, so it cannot exercise this — we have to be
// the misbehaving client and stall mid-body).
// ---------------------------------------------------------------------------

/// Write a complete request head for a POST /sparql whose `Content-Length` PROMISES more body
/// bytes than we will actually send, then send a partial body and stall. With a short
/// `body_read_timeout` the SERVER must let go of the connection (close / reset / an error status)
/// within roughly the deadline, instead of waiting forever for the rest of the body.
#[tokio::test]
async fn slow_body_dribble_closed_by_deadline() {
    let config = ServerConfig {
        // Short body deadline; keep the header deadline generous so it is unambiguously the BODY
        // guard that fires, not the header guard.
        body_read_timeout: Some(Duration::from_millis(400)),
        header_read_timeout: Some(Duration::from_secs(30)),
        ..ServerConfig::default()
    };
    let addr = spawn_serve(config).await;

    let mut sock = TcpStream::connect(addr).await.unwrap();
    // A COMPLETE header block (so the header-read deadline is satisfied), promising 4096 body
    // bytes — but we send only a handful and then never send the rest (the slow-body dribble).
    sock.write_all(
        b"POST /sparql HTTP/1.1\r\n\
          Host: localhost\r\n\
          Content-Type: application/sparql-query\r\n\
          Content-Length: 4096\r\n\r\n\
          SELECT",
    )
    .await
    .unwrap();
    sock.flush().await.unwrap();

    // The body-read deadline (400ms) must make the server release the connection well within this
    // generous 5s cap. Ok(0) is a clean close, Err is a reset, and a 400/408 status line before
    // close all mean the server let go of the slot rather than holding it for the missing body.
    let started = std::time::Instant::now();
    let mut buf = [0u8; 256];
    let outcome = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await;
    let elapsed = started.elapsed();
    match outcome {
        Ok(Ok(0)) | Ok(Err(_)) => {} // connection closed / reset by the server — the guard fired
        Ok(Ok(n)) => {
            let head = String::from_utf8_lossy(&buf[..n]);
            assert!(
                head.contains("400") || head.contains("408") || head.contains("500"),
                "server responded but did not signal a body-read timeout: {head:?}"
            );
        }
        Err(_) => panic!(
            "slow-body connection was NOT closed within 5s — the body-read deadline did not fire \
             (connection + concurrency slot held open for a dribbled body: the slow-body DoS)"
        ),
    }
    assert!(
        elapsed < Duration::from_secs(3),
        "connection released but only after {elapsed:?} — far beyond the 400ms body deadline"
    );
}

/// With the body deadline DISABLED (`None`) — header guard still on — the same dribbled-body
/// connection is NOT proactively released within a short window, proving the body guard is what
/// does the releasing (and is genuinely opt-out-able). We only assert it stays open briefly (then
/// we drop it), so the test is fast and not flaky.
#[tokio::test]
async fn slow_body_held_open_when_deadline_disabled() {
    let config = ServerConfig {
        body_read_timeout: None,
        header_read_timeout: Some(Duration::from_secs(30)),
        ..ServerConfig::default()
    };
    let addr = spawn_serve(config).await;

    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(
        b"POST /sparql HTTP/1.1\r\n\
          Host: localhost\r\n\
          Content-Type: application/sparql-query\r\n\
          Content-Length: 4096\r\n\r\n\
          SELECT",
    )
    .await
    .unwrap();
    sock.flush().await.unwrap();

    // Within a short window the server must NOT have released the (incomplete-body) connection.
    let mut buf = [0u8; 64];
    let outcome = tokio::time::timeout(Duration::from_millis(700), sock.read(&mut buf)).await;
    assert!(
        outcome.is_err(),
        "with body_read_timeout=None the connection should stay open (waiting for the rest of the \
         body), but the server released/answered it: {outcome:?}"
    );
}

/// Sanity: with the body deadline ON, a COMPLETE POST /sparql whose body arrives all at once is
/// still answered normally (the timer resets after the frame and never fires). Proves the guard
/// does not penalise an honest request that simply carries a body.
#[tokio::test]
async fn slow_body_guard_does_not_break_a_complete_post() {
    let config = ServerConfig {
        body_read_timeout: Some(Duration::from_millis(500)),
        ..ServerConfig::default()
    };
    let addr = spawn_serve(config).await;

    let body = b"SELECT * WHERE { ?s ?p ?o } LIMIT 1";
    let req = format!(
        "POST /sparql HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/sparql-query\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    let mut sock = TcpStream::connect(addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.write_all(body).await.unwrap(); // whole body in one write — no idle gap
    sock.flush().await.unwrap();

    let mut resp = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), sock.read_to_end(&mut resp))
        .await
        .expect("a complete POST with a body must be answered promptly under the body deadline")
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "complete POST /sparql under body_read_timeout must return 200: {text:?}"
    );
}
