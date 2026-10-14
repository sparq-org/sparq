//! (sq-fh4z / gh-47) HTTP integration tests for the SERVER's named-graph
//! query + update surface — the exact PSS (Solid pod-server) set that was previously
//! UNCOVERED at the server level.
//!
//! ## Why this exists (the gap it closes)
//!
//! The engine has supported a full RDF dataset (a default graph + named graphs, with
//! `GRAPH`/`FROM`/`FROM NAMED`, cross-graph joins and per-graph updates) since the
//! SPARQL conformance work (rounds 3–4; see `sparq_engine::exec` and the dataset-view
//! tests). The SERVER threads that through unchanged — a `GRAPH` pattern reaches the
//! engine, a `GRAPH`-scoped update commits through the sequenced writer, and the
//! Graph-Store-Protocol write side already lowers each named-graph write to a
//! `GRAPH <iri> { … }` update (`src/http.rs`). But until this file there was **no
//! server-level HTTP coverage of named-graph QUERYING** — `tests/protocol.rs` only ever
//! queried the default graph, and `tests/updates.rs` asserted the cache-epoch SCOPE of a
//! `GRAPH` update, never that a named-graph SELECT over HTTP returns the right rows.
//!
//! The stale `src/exec.rs` doc-comment + the `tests/protocol.rs` note used to claim "the
//! engine has no named-graph support (a `GRAPH` pattern makes the engine error at
//! execution)". That was FALSE and is corrected alongside this file.
//!
//! ## What it asserts (the PSS hot paths)
//!
//! Each test spins the real axum server on an ephemeral port, seeds a Solid-pod-shaped
//! dataset (a container graph that `ldp:contains` child resources, each child resource in
//! its OWN named graph carrying an `ex:size`), drives it over real HTTP with `reqwest`,
//! and asserts the **exact** SPARQL-results+json shape — not a `contains` substring but
//! the parsed `results.bindings[].{type,value,datatype}` (and `boolean` for ASK) — the
//! byte-level contract a real client (and the W3C results spec) depends on. The five
//! prioritised shapes, each a named PSS operation:
//!
//!   1. **variable-second-`GRAPH` cross-graph join** — `getChildren` existence/listing:
//!      a container graph lists children, then a `GRAPH ?child { … }` joins each child to
//!      its own per-resource graph.
//!   2. **`COALESCE(SUM(?size))` + `COUNT(DISTINCT ?g)` with `FILTER(STRSTARTS(STR(?g),…))`**
//!      — the pod *usage* aggregate (total bytes + resource count under a path prefix),
//!      including the empty-match path where `COALESCE` supplies the `0` default.
//!   3. **`DELETE`/`INSERT … WHERE` with `OPTIONAL`** — `setAclPointer` / `putContainer`:
//!      idempotently replace a single-valued pointer whether or not it already exists.
//!   4. **`VALUES` membership** — a bind-join over an explicit set of named graphs.
//!   5. **`ORDER BY` / `OFFSET` / `LIMIT`** — a paged, deterministically-ordered listing.
//!
//! These are deterministic (`ORDER BY` pins row order where multi-row) so the JSON is a
//! stable oracle; the field-level assertions are what a differential check against QLever
//! would compare (term kind / lexical value / datatype IRI), here pinned to the spec's
//! canonical projection.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Boots the real server over an EMPTY graph on an ephemeral port; returns its base URL.
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

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Applies a SPARQL Update over HTTP, asserting the `204 No Content` ack.
async fn update(cl: &reqwest::Client, base: &str, u: &str) {
    let r = cl
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(u.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "update was not acked 204: {u}");
}

/// Runs a SELECT/ASK over HTTP, asserts 200 + the JSON results media type, and returns the
/// parsed SPARQL-results+json document.
async fn query_json(cl: &reqwest::Client, base: &str, q: &str) -> serde_json::Value {
    let resp = cl
        .get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "query did not return 200: {q}");
    assert_eq!(
        resp.headers()["content-type"],
        "application/sparql-results+json",
        "query did not return the JSON results media type: {q}"
    );
    resp.json().await.unwrap()
}

// --- exact-shape helpers over the parsed results document ---------------------------

/// The `results.bindings` array of a SELECT result document.
fn bindings(doc: &serde_json::Value) -> &Vec<serde_json::Value> {
    doc["results"]["bindings"]
        .as_array()
        .unwrap_or_else(|| panic!("no results.bindings: {doc}"))
}

/// Asserts a binding cell is a `uri` term with exactly `value`.
fn assert_uri(cell: &serde_json::Value, value: &str) {
    assert_eq!(cell["type"], "uri", "cell is not a uri term: {cell}");
    assert_eq!(cell["value"], value, "uri value mismatch: {cell}");
    assert!(
        cell.get("datatype").is_none(),
        "a uri term must not carry a datatype: {cell}"
    );
}

/// Asserts a binding cell is a typed `literal` with exactly `value` + `datatype`.
fn assert_typed_literal(cell: &serde_json::Value, value: &str, datatype: &str) {
    assert_eq!(
        cell["type"], "literal",
        "cell is not a literal term: {cell}"
    );
    assert_eq!(
        cell["value"], value,
        "literal lexical value mismatch: {cell}"
    );
    assert_eq!(
        cell["datatype"], datatype,
        "literal datatype mismatch: {cell}"
    );
}

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// A Solid-pod-shaped dataset: a container graph `<…/alice/>` that `ldp:contains` three
/// child resources, each child in its OWN per-resource named graph carrying an `ex:size`,
/// plus an unrelated resource under a different prefix (`…/bob/`) for the FILTER tests.
/// Seven triples total, all in NAMED graphs (3 `ldp:contains` + 4 `ex:size`).
const SEED: &str = r#"INSERT DATA {
  GRAPH <http://pod/alice/> {
    <http://pod/alice/> <http://www.w3.org/ns/ldp#contains>
        <http://pod/alice/a.ttl> , <http://pod/alice/b.ttl> , <http://pod/alice/c.ttl> .
  }
  GRAPH <http://pod/alice/a.ttl> { <http://pod/alice/a.ttl> <http://ex/size> 100 }
  GRAPH <http://pod/alice/b.ttl> { <http://pod/alice/b.ttl> <http://ex/size> 250 }
  GRAPH <http://pod/alice/c.ttl> { <http://pod/alice/c.ttl> <http://ex/size> 30  }
  GRAPH <http://pod/bob/d.ttl>   { <http://pod/bob/d.ttl>   <http://ex/size> 999 }
}"#;

/// Seeds a fresh server with `SEED` and returns `(base_url, client)`.
async fn seeded() -> (String, reqwest::Client) {
    let base = spawn_empty().await;
    let cl = client();
    update(&cl, &base, SEED).await;
    (base, cl)
}

// ---------------------------------------------------------------------------
// (1) variable-second-GRAPH cross-graph join — getChildren existence + listing
// ---------------------------------------------------------------------------

/// The `getChildren` listing hot path: read the container's `ldp:contains` set from one
/// named graph, then join each child to its OWN per-resource named graph (a VARIABLE in
/// the second `GRAPH`) to pull its size. Ordered so the rows are a deterministic oracle.
#[tokio::test]
async fn cross_graph_join_variable_second_graph_lists_children() {
    let (base, cl) = seeded().await;
    let doc = query_json(
        &cl,
        &base,
        r#"PREFIX ldp: <http://www.w3.org/ns/ldp#>
           SELECT ?child ?sz WHERE {
             GRAPH <http://pod/alice/> { <http://pod/alice/> ldp:contains ?child }
             GRAPH ?child            { ?child <http://ex/size> ?sz }
           } ORDER BY ?child"#,
    )
    .await;

    // Exactly the three children of `<…/alice/>`, each joined to its own graph's size, in
    // ascending IRI order — never bob/d.ttl (not contained by alice).
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        3,
        "cross-graph join must list all three children: {doc}"
    );
    assert_uri(&rows[0]["child"], "http://pod/alice/a.ttl");
    assert_typed_literal(&rows[0]["sz"], "100", XSD_INTEGER);
    assert_uri(&rows[1]["child"], "http://pod/alice/b.ttl");
    assert_typed_literal(&rows[1]["sz"], "250", XSD_INTEGER);
    assert_uri(&rows[2]["child"], "http://pod/alice/c.ttl");
    assert_typed_literal(&rows[2]["sz"], "30", XSD_INTEGER);
}

/// `getChildren` existence: an ASK over the same cross-graph shape is `true` when at least
/// one contained child has a sized graph, and `false` for a container with no children.
#[tokio::test]
async fn cross_graph_existence_ask_true_and_false() {
    let (base, cl) = seeded().await;

    let t = query_json(
        &cl,
        &base,
        r#"PREFIX ldp: <http://www.w3.org/ns/ldp#>
           ASK { GRAPH <http://pod/alice/> { <http://pod/alice/> ldp:contains ?c }
                 GRAPH ?c { ?c <http://ex/size> ?sz } }"#,
    )
    .await;
    assert_eq!(
        t["boolean"], true,
        "container with children must ASK true: {t}"
    );

    let f = query_json(
        &cl,
        &base,
        r#"PREFIX ldp: <http://www.w3.org/ns/ldp#>
           ASK { GRAPH <http://pod/empty/> { <http://pod/empty/> ldp:contains ?c }
                 GRAPH ?c { ?c <http://ex/size> ?sz } }"#,
    )
    .await;
    assert_eq!(f["boolean"], false, "empty container must ASK false: {f}");
}

// ---------------------------------------------------------------------------
// (2) usage aggregate — COALESCE(SUM(?size)) + COUNT(DISTINCT ?g) + STRSTARTS filter
// ---------------------------------------------------------------------------

/// Pod *usage*: sum the sizes and count the distinct resource graphs whose graph IRI starts
/// with a path prefix — the `getStorageUsage` aggregate. `COALESCE` guards the empty case.
#[tokio::test]
async fn usage_aggregate_sum_and_distinct_count_under_prefix() {
    let (base, cl) = seeded().await;
    let doc = query_json(
        &cl,
        &base,
        r#"SELECT (COALESCE(SUM(?sz), 0) AS ?total) (COUNT(DISTINCT ?g) AS ?n) WHERE {
             GRAPH ?g { ?s <http://ex/size> ?sz }
             FILTER(STRSTARTS(STR(?g), "http://pod/alice/"))
           }"#,
    )
    .await;

    // Only the three alice/* resource graphs match the prefix: 100 + 250 + 30 = 380, 3 graphs.
    // bob/d.ttl (999) is filtered out, so SUM must NOT include it.
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        1,
        "an aggregate with no GROUP BY yields one row: {doc}"
    );
    assert_typed_literal(&rows[0]["total"], "380", XSD_INTEGER);
    assert_typed_literal(&rows[0]["n"], "3", XSD_INTEGER);
}

/// The empty-match path: with a prefix that matches NO graph, `SUM` is unbound and
/// `COALESCE(SUM(...), 0)` must supply the explicit `0` default (a sized literal, not an
/// absent cell) — the property that makes the usage query safe for an empty pod.
#[tokio::test]
async fn usage_aggregate_coalesce_defaults_to_zero_on_empty_match() {
    let (base, cl) = seeded().await;
    let doc = query_json(
        &cl,
        &base,
        r#"SELECT (COALESCE(SUM(?sz), 0) AS ?total) WHERE {
             GRAPH ?g { ?s <http://ex/size> ?sz }
             FILTER(STRSTARTS(STR(?g), "http://pod/nobody/"))
           }"#,
    )
    .await;
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        1,
        "COALESCE over an empty match still yields one row: {doc}"
    );
    assert_typed_literal(&rows[0]["total"], "0", XSD_INTEGER);
}

// ---------------------------------------------------------------------------
// (3) DELETE/INSERT … WHERE with OPTIONAL — setAclPointer / putContainer
// ---------------------------------------------------------------------------

/// `setAclPointer`: idempotently replace a single-valued pointer inside a named graph with
/// a `DELETE { … } INSERT { … } WHERE { OPTIONAL { … } }`. The OPTIONAL makes it work whether
/// or not a prior pointer exists, and the DELETE clears any stale one (no duplicate left).
#[tokio::test]
async fn delete_insert_where_optional_replaces_acl_pointer() {
    let (base, cl) = seeded().await;
    let acl = "http://www.w3.org/ns/auth/acl#accessControl";

    // CASE A — no prior pointer: the OPTIONAL binds nothing, the INSERT still sets it.
    update(
        &cl,
        &base,
        r#"PREFIX acl: <http://www.w3.org/ns/auth/acl#>
           DELETE { GRAPH <http://pod/alice/> { <http://pod/alice/a.ttl> acl:accessControl ?old } }
           INSERT { GRAPH <http://pod/alice/> { <http://pod/alice/a.ttl> acl:accessControl <http://pod/alice/v1.acl> } }
           WHERE  { OPTIONAL { GRAPH <http://pod/alice/> { <http://pod/alice/a.ttl> acl:accessControl ?old } } }"#,
    )
    .await;
    let doc = query_json(
        &cl,
        &base,
        &format!("SELECT ?p WHERE {{ GRAPH <http://pod/alice/> {{ <http://pod/alice/a.ttl> <{acl}> ?p }} }}"),
    )
    .await;
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        1,
        "case A must leave exactly one pointer: {doc}"
    );
    assert_uri(&rows[0]["p"], "http://pod/alice/v1.acl");

    // CASE B — a pointer already exists: the DELETE clears the old, the INSERT sets the new,
    // and there is STILL exactly one value (no duplicate accumulates).
    update(
        &cl,
        &base,
        r#"PREFIX acl: <http://www.w3.org/ns/auth/acl#>
           DELETE { GRAPH <http://pod/alice/> { <http://pod/alice/a.ttl> acl:accessControl ?old } }
           INSERT { GRAPH <http://pod/alice/> { <http://pod/alice/a.ttl> acl:accessControl <http://pod/alice/v2.acl> } }
           WHERE  { OPTIONAL { GRAPH <http://pod/alice/> { <http://pod/alice/a.ttl> acl:accessControl ?old } } }"#,
    )
    .await;
    let doc = query_json(
        &cl,
        &base,
        &format!("SELECT ?p WHERE {{ GRAPH <http://pod/alice/> {{ <http://pod/alice/a.ttl> <{acl}> ?p }} }}"),
    )
    .await;
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        1,
        "case B must REPLACE, not accumulate — exactly one pointer: {doc}"
    );
    assert_uri(&rows[0]["p"], "http://pod/alice/v2.acl");

    // The replacement is scoped to alice's graph only: bob's graph never gains an acl pointer.
    let doc = query_json(
        &cl,
        &base,
        "SELECT ?p WHERE { GRAPH <http://pod/bob/d.ttl> { ?s ?p ?o } FILTER(?p = <http://www.w3.org/ns/auth/acl#accessControl>) }",
    )
    .await;
    assert_eq!(
        bindings(&doc).len(),
        0,
        "the acl write must not bleed into bob's graph: {doc}"
    );
}

// ---------------------------------------------------------------------------
// (4) VALUES membership — bind-join over an explicit named-graph set
// ---------------------------------------------------------------------------

/// `VALUES` membership: pin the graph variable to an explicit set of named-graph IRIs and
/// pull each graph's size — a bind-join over a known resource list (e.g. a batch read).
#[tokio::test]
async fn values_membership_over_named_graphs() {
    let (base, cl) = seeded().await;
    let doc = query_json(
        &cl,
        &base,
        r#"SELECT ?g ?sz WHERE {
             VALUES ?g { <http://pod/alice/a.ttl> <http://pod/bob/d.ttl> <http://pod/alice/absent.ttl> }
             GRAPH ?g { ?s <http://ex/size> ?sz }
           } ORDER BY ?g"#,
    )
    .await;

    // Only the two graphs that EXIST contribute a row; the absent one drops out (inner join),
    // ordered ascending by IRI: alice/a.ttl (100) then bob/d.ttl (999).
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        2,
        "VALUES membership must yield only the existing graphs: {doc}"
    );
    assert_uri(&rows[0]["g"], "http://pod/alice/a.ttl");
    assert_typed_literal(&rows[0]["sz"], "100", XSD_INTEGER);
    assert_uri(&rows[1]["g"], "http://pod/bob/d.ttl");
    assert_typed_literal(&rows[1]["sz"], "999", XSD_INTEGER);
}

// ---------------------------------------------------------------------------
// (5) ORDER BY / OFFSET / LIMIT — paged, deterministically ordered listing
// ---------------------------------------------------------------------------

/// A paged listing across ALL resource graphs, ordered by descending size with an OFFSET +
/// LIMIT window — the deterministic slice a paged client requests. Across the four sized
/// graphs (999, 250, 100, 30) descending, `OFFSET 1 LIMIT 2` is the middle pair.
#[tokio::test]
async fn order_by_offset_limit_pages_named_graph_rows() {
    let (base, cl) = seeded().await;
    let doc = query_json(
        &cl,
        &base,
        r#"SELECT ?g ?sz WHERE { GRAPH ?g { ?s <http://ex/size> ?sz } }
           ORDER BY DESC(?sz) OFFSET 1 LIMIT 2"#,
    )
    .await;

    // Full descending order is 999, 250, 100, 30; OFFSET 1 LIMIT 2 -> [250, 100].
    let rows = bindings(&doc);
    assert_eq!(
        rows.len(),
        2,
        "OFFSET 1 LIMIT 2 over four rows must yield two: {doc}"
    );
    assert_uri(&rows[0]["g"], "http://pod/alice/b.ttl");
    assert_typed_literal(&rows[0]["sz"], "250", XSD_INTEGER);
    assert_uri(&rows[1]["g"], "http://pod/alice/a.ttl");
    assert_typed_literal(&rows[1]["sz"], "100", XSD_INTEGER);
}

// ---------------------------------------------------------------------------
// Default-graph isolation — named-graph data does NOT bleed into the default graph
// ---------------------------------------------------------------------------

/// The dataset boundary the whole PSS model depends on: every `SEED` triple lives in a NAMED
/// graph, so a default-graph query (`{ ?s ?p ?o }`, no `GRAPH`) must see NOTHING — the named
/// graphs are isolated from the default graph over the real HTTP query path. A `GRAPH ?g`
/// wildcard, by contrast, sees every one of the seven seeded triples.
#[tokio::test]
async fn named_graph_data_is_isolated_from_default_graph() {
    let (base, cl) = seeded().await;

    let def = query_json(&cl, &base, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }").await;
    assert_eq!(
        bindings(&def).len(),
        0,
        "named-graph-only data must NOT be visible in the default graph: {def}"
    );

    // The union over all named graphs DOES see it: 3 ldp:contains + 4 ex:size = 7 triples.
    let all = query_json(
        &cl,
        &base,
        "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } }",
    )
    .await;
    assert_typed_literal(&bindings(&all)[0]["n"], "7", XSD_INTEGER);
}
