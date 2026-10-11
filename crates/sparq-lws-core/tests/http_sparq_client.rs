// AUTHORED-BY Claude Opus 4.8
//! Unit/integration tests for the live [`HttpSparqClient`] against an **in-process mock SPARQL HTTP
//! endpoint** — no running SPARQ needed.
//!
//! The mock is a tiny axum server that classifies each request the way `sparq-server`'s `/sparql`
//! route does (POST `application/sparql-query` → results-JSON/CONSTRUCT; POST
//! `application/sparql-update` → 204) and returns canned `application/sparql-results+json` / Turtle
//! bodies driven by the SPARQL the client sent. This exercises the full HTTP plumbing (request
//! shaping, status classification, bounded body read, JSON parsing) deterministically.
//!
//! The mock keeps a small in-memory "store" so the atomic `create_child` round-trip (the guarded
//! update + the follow-up create-marker confirm the client issues) is meaningfully exercised,
//! including the container-EXISTS-guard rejection path.
//!
//! A LIVE integration test against a real SPARQ is at the bottom, `#[ignore]`'d + env-gated
//! (`PSS_LIVE_SPARQ_URL`) — running it needs a SPARQ instance (a `needs:user` item).
//!
//! The whole file is gated on the opt-in `http-sparq` feature (sq-gg0qq.3 — the remote HTTP
//! backend is no longer compiled by default); it is a no-op test binary when the feature is off.
//! CI runs it in the `opt-in sparq-lws-core (http-sparq)` feature-matrix leg.
#![cfg(feature = "http-sparq")]

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use sparq_lws_core::store::{
    BodyObject, DeleteOutcome, HttpSparqClient, ResourceMeta, SparqClient, SparqError,
};

// ---------------------------------------------------------------------------
// The in-process mock SPARQL endpoint.
// ---------------------------------------------------------------------------

/// How the mock should behave, for the per-test variations (errors, timeouts, malformed bodies).
#[derive(Clone, Default)]
struct MockMode {
    /// Force every response to this status (with a body) — for the 5xx / 4xx tests.
    force_status: Option<StatusCode>,
    /// When `force_status` is set, make its body large (> the client's read bound is impractical in
    /// a test, so this just proves a non-empty error body does not flip the retryable classification).
    force_status_with_body: bool,
    /// Return a malformed (non-JSON) body for SELECT/ASK — for the malformed-response test.
    malformed: bool,
    /// Return a `SELECT ?child` body whose single row is MISSING the `child` binding — for the
    /// list_children malformed-row fatal test.
    children_row_missing_binding: bool,
    /// Sleep this long BEFORE sending any response (headers + body) — for the whole-request timeout.
    delay: Option<Duration>,
}

/// The mock's tiny store: resource IRI → its index record; container IRI → child IRIs; resource IRI
/// → its inserted body triples (raw N-Triples-ish lines) for the insert→construct round-trip.
#[derive(Default)]
struct MockStore {
    /// resource graph IRI → (contentType, blobKey, etag).
    meta: HashMap<String, (String, String, String)>,
    /// container graph IRI → child IRIs (ldp:contains).
    children: HashMap<String, Vec<String>>,
    /// resource graph IRI → its body triples, stored as already-rendered N-Triples lines.
    body: HashMap<String, Vec<String>>,
    /// child graph IRI → its per-operation create-marker nonces (the race-resistant create confirm).
    markers: HashMap<String, Vec<String>>,
    /// container IRI → its per-operation DELETE-marker nonces (the race-resistant empty-delete confirm,
    /// stored in a separate scratch graph keyed by the container IRI as the marker subject).
    delete_markers: HashMap<String, Vec<String>>,
    /// The last SPARQL string the mock received (so a test can assert on the query text/escaping).
    last_sparql: Option<String>,
    /// Total protocol requests received (so a test can pin how many round-trips an op cost).
    requests: usize,
}

#[derive(Clone)]
struct MockState {
    mode: Arc<Mutex<MockMode>>,
    store: Arc<Mutex<MockStore>>,
}

/// Spawn the mock server on an ephemeral port; returns the `/sparql` URL + the shared state handle.
async fn spawn_mock() -> (String, MockState) {
    let state = MockState {
        mode: Arc::new(Mutex::new(MockMode::default())),
        store: Arc::new(Mutex::new(MockStore::default())),
    };
    let app = Router::new()
        .route("/sparql", post(handle_sparql))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/sparql"), state)
}

/// The mock `/sparql` handler — classifies query vs update + answers from the tiny store.
async fn handle_sparql(
    State(state): State<MockState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mode = state.mode.lock().unwrap().clone();
    if let Some(delay) = mode.delay {
        tokio::time::sleep(delay).await;
    }
    if let Some(status) = mode.force_status {
        // A non-empty error body (the realistic case) must NOT flip the retryable classification —
        // the client classifies on status from headers first.
        let body = if mode.force_status_with_body {
            "x".repeat(64 * 1024) // a chunky-but-bounded error body
        } else {
            r#"{"error":"forced"}"#.to_string()
        };
        return (status, body).into_response();
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let sparql = String::from_utf8_lossy(&body).to_string();
    {
        let mut store = state.store.lock().unwrap();
        store.last_sparql = Some(sparql.clone());
        store.requests += 1;
    }

    if ct.starts_with("application/sparql-update") {
        apply_update(&state, &sparql);
        return StatusCode::NO_CONTENT.into_response();
    }
    // Otherwise a query (application/sparql-query). Answer ASK / SELECT / CONSTRUCT.
    if mode.malformed {
        return results_json("this is not json{");
    }
    if mode.children_row_missing_binding && sparql.starts_with("SELECT ?child") {
        // A SELECT-children result whose row is missing the `child` binding (a malformed backend).
        return results_json(
            r#"{"head":{"vars":["child"]},"results":{"bindings":[{"other":{"type":"uri","value":"http://x"}}]}}"#,
        );
    }
    answer_query(&state, &sparql)
}

/// Apply a (mock) SPARQL update by recognising the client's fixed-shape statements. This is NOT a
/// full SPARQL engine — it pattern-matches the specific updates [`sparq_lws_core::store::sparql`]
/// emits — but it DOES faithfully model SPARQL 1.1 Update SEQUENCING for the delete path: a
/// `;`-joined request is split into its constituent operations and each is applied IN ORDER, with the
/// later operation's guard evaluated against the store AS LEFT BY the earlier one. That faithfulness
/// is what lets a regression test catch the round's HIGH — a two-statement `DELETE …WHERE… ; INSERT
/// …WHERE…` form would have its INSERT guard re-evaluated after the DELETE emptied the container, so
/// the marker would NOT be written and a real delete would mis-report NotFound. The fix (a SINGLE
/// `DELETE { } INSERT { } WHERE { }` modify) has no `;`, so its WHERE is evaluated once and the marker
/// is written. See `apply_delete_container_modify`.
fn apply_update(state: &MockState, sparql: &str) {
    let mut store = state.store.lock().unwrap();
    // delete_container_if_empty: a SINGLE `DELETE { container-graph ; parent-edge } INSERT { marker }
    // WHERE { exists+empty guard }` modify, recognised by the `deleteMarker` predicate (distinguishing
    // it from create_child's `createMarker`). Model SPARQL 1.1 sequencing FAITHFULLY: split on the
    // top-level `;` into operations and apply each in order, re-deriving each op's guard from the
    // CURRENT (post-previous-op) store. The fix is ONE op (no `;`), so its single WHERE matches the
    // pre-delete state ⇒ the marker is written. The pre-fix two-op form would NOT write the marker.
    if sparql.contains("deleteMarker") && sparql.starts_with("DELETE {") {
        for op in split_top_level_semicolons(sparql) {
            apply_delete_container_modify(&mut store, op.trim());
        }
        return;
    }
    // create_child: `DELETE { GRAPH <child> {…stale record…} } INSERT { GRAPH <child> {…record…}
    //                GRAPH <container> { <record> <ldp:contains> <child> } }
    //                WHERE { GRAPH <container> { <record> <contentType> ?anyCt } OPTIONAL … }`
    if sparql.contains("ldp#contains")
        && sparql.starts_with("DELETE {")
        && sparql.contains("INSERT {")
        && sparql.contains("WHERE {")
    {
        // child graph = first GRAPH IRI (in the DELETE block); container = first GRAPH IRI in WHERE.
        let child = first_graph_iri(sparql).unwrap_or_default();
        let where_pos = sparql.find("WHERE {").unwrap();
        let container = first_iri_after(sparql, where_pos).unwrap_or_default();
        // The guard: only create if the container has an index record (contentType) in the store.
        if store.meta.contains_key(&container) {
            // Replace any stale child record (the DELETE-before-INSERT semantics), then add the edge
            // (idempotent) + record THIS operation's marker nonce. The INSERT literals are, in order:
            // contentType, blobKey, etag, createMarker-nonce.
            let literals_found = extract_literals(sparql);
            let g = |i: usize| literals_found.get(i).cloned().unwrap_or_default();
            store.meta.insert(child.clone(), (g(0), g(1), g(2)));
            let nonce = g(3);
            // Markers are APPEND-ONLY (no DELETE of markers), so a same-child create never removes an
            // earlier op's nonce — each op confirms its own.
            store.markers.entry(child.clone()).or_default().push(nonce);
            let kids = store.children.entry(container).or_default();
            if !kids.contains(&child) {
                kids.push(child);
            }
        }
        return;
    }
    // insert_body: `INSERT DATA { GRAPH <r> { <s> <p> <o> . … } }` (no leading DROP/DELETE) — store
    // the body triple lines for the construct round-trip.
    if sparql.starts_with("INSERT DATA") && !sparql.contains("contentType") {
        if let Some(resource) = first_graph_iri(sparql) {
            // Capture each `<s> <p> <o> .` triple as a raw line (the IRIs/literals as rendered).
            let inner = inside_graph_block(sparql).unwrap_or_default();
            for line in inner.split(" . ") {
                let t = line.trim().trim_end_matches('.').trim();
                if !t.is_empty() {
                    store
                        .body
                        .entry(resource.clone())
                        .or_default()
                        .push(t.to_string());
                }
            }
        }
        return;
    }
    // put_meta: three `DELETE WHERE { GRAPH <r> { <record> <pred> ?old } }` then
    // `INSERT DATA { GRAPH <r> { <record> <contentType> "ct" ; … } }` — replaces ONLY the reserved
    // record triples, leaving children + RDF intact. The mock store keys metadata by resource, so it
    // upserts the record without touching `children` (the bug roborev flagged would be a `.remove`).
    if sparql.starts_with("DELETE WHERE") && sparql.contains("INSERT DATA") {
        // The resource graph IRI is the first GRAPH IRI (same across all clauses).
        if let Some(resource) = first_graph_iri(sparql) {
            let (ct, bk, et) = parse_record_literals(sparql);
            store.meta.insert(resource, (ct, bk, et));
        }
        return;
    }
    // delete_resource: `DROP SILENT GRAPH <r>` — removes the whole resource (record + children + RDF
    // + markers).
    if sparql.starts_with("DROP SILENT GRAPH") {
        if let Some(resource) = extract_iris(sparql).first().cloned() {
            store.meta.remove(&resource);
            store.children.remove(&resource);
            store.body.remove(&resource);
            store.markers.remove(&resource);
        }
        return;
    }
    // remove_child: `DELETE WHERE { GRAPH <container> { <record> <ldp:contains> <child> } }`
    if sparql.starts_with("DELETE WHERE") && sparql.contains("ldp#contains") {
        let iris = extract_iris(sparql);
        // The graph IRI is first; the child IRI is the last.
        if let (Some(container), Some(child)) = (iris.first().cloned(), iris.last().cloned()) {
            if let Some(v) = store.children.get_mut(&container) {
                v.retain(|c| c != &child);
            }
        }
    }
}

/// Answer a query (ASK exists / SELECT meta / SELECT children / CONSTRUCT) from the store.
fn answer_query(state: &MockState, sparql: &str) -> Response {
    let store = state.store.lock().unwrap();
    if sparql.starts_with("ASK") {
        let graph = first_graph_iri(sparql).unwrap_or_default();
        let boolean = if sparql.contains("deleteMarker") {
            // ask_delete_marker: ASK { GRAPH <scratch> { <container> <deleteMarker> "nonce" } } — true
            // iff that exact nonce was recorded for the container. The graph here is the SCRATCH graph;
            // the container is the SUBJECT (the first <...> IRI INSIDE the graph block), and the nonce
            // is the literal. (`first_graph_iri` returns the scratch graph, so re-extract the subject.)
            let iris = extract_iris(sparql);
            // tokens: [scratchGraph, container, deleteMarkerPredicate] — the container is index 1.
            let container = iris.get(1).cloned().unwrap_or_default();
            let nonce = extract_literals(sparql)
                .first()
                .cloned()
                .unwrap_or_default();
            store
                .delete_markers
                .get(&container)
                .is_some_and(|ns| ns.contains(&nonce))
        } else if sparql.contains("createMarker") {
            // ask_create_marker: ASK { GRAPH <child> { <record> <createMarker> "nonce" } } — true iff
            // that exact nonce was recorded for the child (the race-resistant create confirm).
            let nonce = extract_literals(sparql)
                .first()
                .cloned()
                .unwrap_or_default();
            store
                .markers
                .get(&graph)
                .is_some_and(|ns| ns.contains(&nonce))
        } else if sparql.contains("ldp#contains") {
            // ask_contains_edge: ASK { GRAPH <container> { <record> <ldp:contains> <child> } } —
            // true iff that exact containment edge is present. The child is the last <...> IRI.
            let child = extract_iris(sparql).last().cloned().unwrap_or_default();
            store
                .children
                .get(&graph)
                .is_some_and(|kids| kids.contains(&child))
        } else {
            // ask_exists: ASK { GRAPH <r> { <record> <contentType> ?ct } } — true iff `r` is indexed.
            store.meta.contains_key(&graph)
        };
        return results_json(&format!(r#"{{"head":{{}},"boolean":{boolean}}}"#));
    }
    if sparql.starts_with("SELECT ?g ?ct ?bk ?etag") {
        // select_read_plan: `VALUES ?g { <target> <acl…> } GRAPH ?g { record… }` — one row per
        // VALUES graph that has an index record.
        let values = sparql
            .find("VALUES ?g {")
            .map(|p| p + "VALUES ?g {".len())
            .and_then(|start| {
                sparql[start..]
                    .find('}')
                    .map(|end| sparql[start..start + end].to_string())
            })
            .unwrap_or_default();
        let rows: Vec<String> = extract_iris(&values)
            .into_iter()
            .filter_map(|g| {
                store.meta.get(&g).map(|(ct, bk, et)| {
                    format!(
                        r#"{{"g":{{"type":"uri","value":{g}}},"ct":{{"type":"literal","value":{ct}}},"bk":{{"type":"literal","value":{bk}}},"etag":{{"type":"literal","value":{et}}}}}"#,
                        g = json_str(&g),
                        ct = json_str(ct),
                        bk = json_str(bk),
                        et = json_str(et),
                    )
                })
            })
            .collect();
        return results_json(&format!(
            r#"{{"head":{{"vars":["g","ct","bk","etag"]}},"results":{{"bindings":[{}]}}}}"#,
            rows.join(",")
        ));
    }
    if sparql.starts_with("SELECT ?ct") {
        // select_meta
        let resource = first_graph_iri(sparql).unwrap_or_default();
        return match store.meta.get(&resource) {
            Some((ct, bk, et)) => results_json(&select_meta_json(ct, bk, et)),
            None => {
                results_json(r#"{"head":{"vars":["ct","bk","etag"]},"results":{"bindings":[]}}"#)
            }
        };
    }
    if sparql.starts_with("SELECT ?child") {
        // select_children
        let container = first_graph_iri(sparql).unwrap_or_default();
        let mut kids = store.children.get(&container).cloned().unwrap_or_default();
        // A `LIMIT n` suffix (the bounded listing) cuts the rows, as a real endpoint does.
        if let Some(n) = sparql
            .rsplit_once(" LIMIT ")
            .and_then(|(_, n)| n.trim().parse::<usize>().ok())
        {
            kids.truncate(n);
        }
        return results_json(&select_children_json(&kids));
    }
    if sparql.starts_with("CONSTRUCT") {
        // CONSTRUCT — return the resource's ACTUAL inserted body triples (a real round-trip), as
        // N-Triples. Empty if nothing was inserted.
        let resource = first_graph_iri(sparql).unwrap_or_default();
        let mut body = String::new();
        if let Some(lines) = store.body.get(&resource) {
            for t in lines {
                body.push_str(t);
                body.push_str(" .\n");
            }
        }
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/n-triples; charset=utf-8")],
            body,
        )
            .into_response();
    }
    results_json(r#"{"head":{},"boolean":false}"#)
}

/// Split a SPARQL update into its top-level `;`-separated operations, IGNORING `;` that appear inside
/// `{ }` group graph patterns, `< >` IRIs, or `" "` literals. This is what lets the mock model SPARQL
/// 1.1 sequencing: each returned operation is applied IN ORDER against the store as left by the prior
/// one. The fix's SINGLE `DELETE { } INSERT { } WHERE { }` modify has no top-level `;`, so this yields
/// ONE op; the pre-fix `DELETE …WHERE… ; INSERT …WHERE…` form yields TWO.
fn split_top_level_semicolons(sparql: &str) -> Vec<String> {
    let mut ops = Vec::new();
    let mut depth = 0i32;
    let mut in_iri = false;
    let mut in_lit = false;
    let mut start = 0usize;
    let bytes = sparql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '"' if !in_iri => in_lit = !in_lit,
            '<' if !in_lit => in_iri = true,
            '>' if in_iri => in_iri = false,
            '{' if !in_iri && !in_lit => depth += 1,
            '}' if !in_iri && !in_lit => depth -= 1,
            ';' if depth == 0 && !in_iri && !in_lit => {
                ops.push(sparql[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    ops.push(sparql[start..].to_string());
    ops
}

/// Apply ONE operation of the empty-container delete modify against the CURRENT store, modelling
/// SPARQL 1.1 `DELETE/INSERT … WHERE` semantics faithfully: derive the guard (the WHERE's
/// exists+empty condition) from the store AS IT IS NOW, and IFF it matches, apply EVERY DELETE and
/// INSERT clause present in THIS op together (one shared solution). Crucially, the guard is read from
/// the live store, so when a two-statement `;`-joined form is fed in, the second op's INSERT guard is
/// re-evaluated AFTER the first op's DELETE emptied the container — it fails, so the marker is NOT
/// written (the round's HIGH). The single-op fix evaluates its guard once (container still present) and
/// both the DELETE and the INSERT run, so the marker IS written.
fn apply_delete_container_modify(store: &mut MockStore, op: &str) {
    // The container graph is the FIRST GRAPH IRI in the WHERE clause (the guard targets the container's
    // own graph in BOTH the single-modify and the pre-fix two-statement forms — so deriving it from
    // WHERE, not the op's first GRAPH, is robust to the INSERT-only op whose first GRAPH is the marker
    // scratch graph). Fall back to the op's first GRAPH if there is no WHERE (defensive).
    let container = op
        .find("WHERE {")
        .and_then(|w| first_iri_after(op, w))
        .or_else(|| first_graph_iri(op))
        .unwrap_or_default();
    // The guard, evaluated against the CURRENT store: the container record EXISTS and has no member.
    let exists = store.meta.contains_key(&container);
    let empty = store
        .children
        .get(&container)
        .is_none_or(|kids| kids.is_empty()); // `is_none_or` (stable 1.82) — MSRV is now 1.88.
    if !(exists && empty) {
        // Guard fails ⇒ NOTHING in this op runs (neither DELETE nor INSERT) — the safety invariant.
        return;
    }
    // The op's DELETE clause (the text before INSERT/WHERE) may name the container graph and the
    // parent edge; the INSERT clause (if present in THIS op) writes the marker. Apply whatever this op
    // contains, all under the single guard match above.
    let has_delete = op.trim_start().starts_with("DELETE {");
    let has_insert = op.contains("INSERT {");
    if has_delete {
        // Empty the container's graph (record + body) and, if the DELETE template carries a parent
        // edge (`GRAPH <parent> { <record> ldp:contains <container> }`), detach that edge too. The
        // parent graph is the SECOND distinct GRAPH IRI in the DELETE template (before WHERE).
        let where_pos = op.find("WHERE {").unwrap_or(op.len());
        let delete_template = &op[..where_pos];
        let graph_iris = all_graph_iris(delete_template);
        store.meta.remove(&container);
        store.children.remove(&container);
        store.body.remove(&container);
        for parent in graph_iris.iter().filter(|g| *g != &container) {
            if let Some(v) = store.children.get_mut(parent) {
                v.retain(|c| c != &container);
            }
        }
    }
    if has_insert {
        let nonce = extract_literals(op).first().cloned().unwrap_or_default();
        store
            .delete_markers
            .entry(container)
            .or_default()
            .push(nonce);
    }
}

/// Every `GRAPH <...>` IRI in `s`, in order (deduped not required — callers filter). Used to find the
/// parent-edge graph folded into the delete modify's DELETE template.
fn all_graph_iris(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = s[from..].find("GRAPH ") {
        let at = from + rel;
        if let Some(iri) = first_iri_after(s, at) {
            out.push(iri);
        }
        from = at + "GRAPH ".len();
    }
    out
}

/// Extract the text inside the FIRST `GRAPH <...> { ... }` block — the triples body. Returns the
/// content between the matching braces (single-level; the client's INSERT DATA bodies are flat).
fn inside_graph_block(sparql: &str) -> Option<String> {
    let g = sparql.find("GRAPH ")?;
    let open = sparql[g..].find('{')? + g;
    // Find the matching close brace for this block.
    let mut depth = 0i32;
    for (i, ch) in sparql[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(sparql[open + 1..open + i].trim().to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn results_json(body: &str) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/sparql-results+json")],
        body.to_string(),
    )
        .into_response()
}

fn select_meta_json(ct: &str, bk: &str, et: &str) -> String {
    format!(
        r#"{{"head":{{"vars":["ct","bk","etag"]}},"results":{{"bindings":[
            {{"ct":{{"type":"literal","value":{ct}}},
             "bk":{{"type":"literal","value":{bk}}},
             "etag":{{"type":"literal","value":{et}}}}}
        ]}}}}"#,
        ct = json_str(ct),
        bk = json_str(bk),
        et = json_str(et),
    )
}

fn select_children_json(kids: &[String]) -> String {
    let bindings: Vec<String> = kids
        .iter()
        .map(|k| format!(r#"{{"child":{{"type":"uri","value":{}}}}}"#, json_str(k)))
        .collect();
    format!(
        r#"{{"head":{{"vars":["child"]}},"results":{{"bindings":[{}]}}}}"#,
        bindings.join(",")
    )
}

/// JSON-encode a string value (so canned bodies are valid JSON even with quotes inside).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

// ----- tiny SPARQL-text extraction helpers for the mock (NOT a parser) -----

/// All `<...>` IRI tokens in order (the inner text, unbracketed).
fn extract_iris(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(end) = s[i + 1..].find('>') {
                out.push(s[i + 1..i + 1 + end].to_string());
                i = i + 1 + end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The IRI of the first `GRAPH <...>` occurrence.
fn first_graph_iri(s: &str) -> Option<String> {
    let pos = s.find("GRAPH ")?;
    first_iri_after(s, pos)
}

/// The first `<...>` IRI at or after byte offset `from`.
fn first_iri_after(s: &str, from: usize) -> Option<String> {
    let rest = &s[from..];
    let lt = rest.find('<')?;
    let gt = rest[lt + 1..].find('>')?;
    Some(rest[lt + 1..lt + 1 + gt].to_string())
}

/// Best-effort: the three string literals of an index record, in INSERT order (ct, bk, etag).
/// Returns their UNESCAPED lexical values.
fn parse_record_literals(s: &str) -> (String, String, String) {
    let literals_found = extract_literals(s);
    let g = |i: usize| literals_found.get(i).cloned().unwrap_or_default();
    (g(0), g(1), g(2))
}

/// All `"..."` string literals (unescaped), in order.
fn extract_literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            let mut val = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    // Unescape the SPARQL string escapes the builder emits.
                    let next = chars[i + 1];
                    val.push(match next {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        c => c,
                    });
                    i += 2;
                    continue;
                }
                val.push(chars[i]);
                i += 1;
            }
            out.push(val);
            i += 1; // skip closing quote
            continue;
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Tests against the mock.
// ---------------------------------------------------------------------------

const CONTAINER: &str = "https://pod.example/alice/";
const CHILD: &str = "https://pod.example/alice/note1";
const RES: &str = "https://pod.example/alice/data";

fn meta() -> ResourceMeta {
    ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "blob-key-1".into(),
        etag: "\"etag-1\"".into(),
        last_modified: None,
    }
}

#[tokio::test]
async fn exists_is_false_then_true() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    assert!(!c.exists(RES).await.unwrap());
    c.put_meta(RES, meta()).await.unwrap();
    assert!(c.exists(RES).await.unwrap());
}

#[tokio::test]
async fn put_then_get_meta_round_trips() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(RES, meta()).await.unwrap();
    let got = c.get_meta(RES).await.unwrap();
    assert_eq!(got, meta());
}

#[tokio::test]
async fn get_meta_of_absent_is_not_found() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    let err = c.get_meta(RES).await.unwrap_err();
    assert!(matches!(err, SparqError::NotFound));
}

#[tokio::test]
async fn read_plan_is_one_protocol_request_carrying_target_and_all_candidates() {
    // read-2 (§3.1): the live client answers the ENTIRE per-read metadata chain — target meta +
    // every ACL candidate's presence/etag — in ONE combined SELECT (one protocol request), and the
    // rows map back onto the caller's candidate order.
    let (url, state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    // Present: the target and the ROOT acl. Absent: the two nearer candidates.
    c.put_meta(RES, meta()).await.unwrap();
    let root_acl_meta = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "blob-acl".into(),
        etag: "\"etag-acl\"".into(),
        last_modified: None,
    };
    c.put_meta("https://pod.example/.acl", root_acl_meta)
        .await
        .unwrap();

    let candidates = vec![
        format!("{RES}.acl"),
        "https://pod.example/alice/.acl".to_string(),
        "https://pod.example/.acl".to_string(),
    ];
    let before = state.store.lock().unwrap().requests;
    let plan = c.read_plan(RES, &candidates).await.unwrap();
    let after = state.store.lock().unwrap().requests;
    assert_eq!(after - before, 1, "read_plan = ONE protocol request");
    // The one request was the combined SELECT with a single VALUES block over all four graphs.
    let sent = state.store.lock().unwrap().last_sparql.clone().unwrap();
    assert!(sent.starts_with("SELECT ?g ?ct ?bk ?etag"), "{sent}");
    assert_eq!(sent.matches("VALUES").count(), 1, "{sent}");

    assert_eq!(
        plan.target,
        Some(meta()),
        "the target row is the raw target's meta"
    );
    assert_eq!(
        plan.acls,
        vec![
            (format!("{RES}.acl"), None),
            ("https://pod.example/alice/.acl".to_string(), None),
            (
                "https://pod.example/.acl".to_string(),
                Some("\"etag-acl\"".to_string())
            ),
        ],
        "one entry per candidate, in order: absent ⇒ None, present ⇒ its etag"
    );
}

#[tokio::test]
async fn read_plan_of_a_fully_absent_chain_is_all_none() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    let candidates = vec![format!("{RES}.acl"), "https://pod.example/.acl".to_string()];
    let plan = c.read_plan(RES, &candidates).await.unwrap();
    assert_eq!(plan.target, None, "absent target ⇒ None (the caller's 404)");
    assert!(plan.acls.iter().all(|(_, etag)| etag.is_none()));
}

#[tokio::test]
async fn read_plan_rejects_an_injection_iri_fail_closed() {
    // An IRIREF-invalid candidate must reject the WHOLE plan build (fail-closed at the injection-
    // safe builder) — no request reaches the endpoint.
    let (url, state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    let before = state.store.lock().unwrap().requests;
    let err = c
        .read_plan(RES, &["https://pod/x> } DROP GRAPH <y".to_string()])
        .await
        .unwrap_err();
    assert!(matches!(err, SparqError::Backend(_)));
    assert_eq!(
        state.store.lock().unwrap().requests,
        before,
        "no request is issued for a rejected build"
    );
}

#[tokio::test]
async fn insert_body_then_construct_round_trips() {
    // A REAL round-trip: write triples through the client (insert_body → INSERT DATA), then read them
    // back via CONSTRUCT. This exercises both the insert_body_data builder and construct_resource end
    // to end (no canned body — the mock returns exactly what was inserted).
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    let triples = vec![
        (
            format!("{RES}#me"),
            "http://xmlns.com/foaf/0.1/name".to_string(),
            BodyObject::PlainLiteral("Alice".into()),
        ),
        (
            format!("{RES}#me"),
            "http://xmlns.com/foaf/0.1/knows".to_string(),
            BodyObject::Iri(format!("{RES}#bob")),
        ),
    ];
    c.insert_body(RES, &triples).await.unwrap();

    let body = c.construct_resource_ntriples(RES).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        text.contains("<https://pod.example/alice/data#me>"),
        "got: {text}"
    );
    assert!(text.contains("\"Alice\""), "got: {text}");
    assert!(
        text.contains("<https://pod.example/alice/data#bob>"),
        "got: {text}"
    );
}

#[tokio::test]
async fn insert_body_of_no_triples_is_a_noop() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    // An empty triple set issues no request and is Ok.
    c.insert_body(RES, &[]).await.unwrap();
    let body = c.construct_resource_ntriples(RES).await.unwrap();
    assert!(body.is_empty(), "no triples inserted ⇒ empty construct");
}

#[tokio::test]
async fn create_child_is_atomic_and_lists() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    // The container must be indexed first (the EXISTS guard).
    c.put_meta(CONTAINER, meta()).await.unwrap();
    c.create_child(CONTAINER, CHILD, meta()).await.unwrap();

    assert!(c.exists(CHILD).await.unwrap());
    let kids = c.list_children(CONTAINER).await.unwrap();
    assert_eq!(kids, vec![CHILD.to_string()]);
}

/// Review finding: a bounded listing over HTTP buffered and parsed the whole result before
/// checking its bound, and could not tell a listing cut by its LIMIT from a complete one. It
/// asks for one row past the bound and refuses when it gets it, and reads the response only up
/// to what such a listing could take.
#[tokio::test]
async fn a_listing_read_within_a_bound_is_refused_not_cut() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(CONTAINER, meta()).await.unwrap();
    let kids: Vec<String> = (0..5).map(|i| format!("{CONTAINER}k{i}")).collect();
    for kid in &kids {
        c.create_child(CONTAINER, kid, meta()).await.unwrap();
    }
    let all: usize = kids.iter().map(String::len).sum();
    let mut got = c
        .list_children_within(CONTAINER, all)
        .await
        .unwrap()
        .unwrap();
    got.sort();
    assert_eq!(got, kids);
    // Room in bytes for three members: the fourth row shows the listing is longer.
    let three = 3 * (CONTAINER.len() + 1);
    assert_eq!(
        c.list_children_within(CONTAINER, three).await.unwrap(),
        None
    );
    // One member far longer than the bound: the response is not read past what fits.
    let long = format!("{CONTAINER}{}", "x".repeat(64 * 1024));
    c.create_child(CONTAINER, &long, meta()).await.unwrap();
    assert_eq!(
        c.list_children_within(CONTAINER, 4 * 1024).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn create_child_in_missing_container_is_not_found() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    // The container was never indexed: the EXISTS guard rejects ⇒ nothing inserted ⇒ the follow-up
    // existence-confirm fails ⇒ NotFound. Fail-closed (no fabricated success).
    let err = c.create_child(CONTAINER, CHILD, meta()).await.unwrap_err();
    assert!(matches!(err, SparqError::NotFound));
    assert!(!c.exists(CHILD).await.unwrap());
    assert!(c.list_children(CONTAINER).await.unwrap().is_empty());
}

#[tokio::test]
async fn repeated_same_child_create_each_confirms_its_own_marker() {
    // Regression for the round-5 finding: markers are APPEND-ONLY, so a second create on the same
    // child does NOT remove the first op's marker. Two sequential creates (each with its own unique
    // nonce) both succeed — neither false-NotFounds because the other cleared its marker.
    let (url, state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(CONTAINER, meta()).await.unwrap();
    c.create_child(CONTAINER, CHILD, meta()).await.unwrap();
    // A second create on the SAME child must also succeed (idempotent membership, fresh marker).
    c.create_child(CONTAINER, CHILD, meta()).await.unwrap();
    // Both operations' markers are retained (append-only) — proof neither cleared the other's.
    let markers = state
        .store
        .lock()
        .unwrap()
        .markers
        .get(CHILD)
        .cloned()
        .unwrap_or_default();
    assert_eq!(markers.len(), 2, "both markers retained: {markers:?}");
    // Membership stays a single edge (idempotent).
    assert_eq!(
        c.list_children(CONTAINER).await.unwrap(),
        vec![CHILD.to_string()]
    );
}

#[tokio::test]
async fn create_child_with_missing_container_but_preexisting_child_still_not_found() {
    // Regression for the roborev finding: confirming via the CONTAINMENT EDGE (not `exists(child)`).
    // The child already exists in its own right, but its container was never indexed — so the guarded
    // insert adds NO edge and create_child must STILL report NotFound (never a false success).
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(CHILD, meta()).await.unwrap(); // the child exists on its own
    assert!(c.exists(CHILD).await.unwrap());

    let err = c.create_child(CONTAINER, CHILD, meta()).await.unwrap_err();
    assert!(
        matches!(err, SparqError::NotFound),
        "missing container must yield NotFound even when the child already exists"
    );
    // And no spurious containment edge was recorded.
    assert!(c.list_children(CONTAINER).await.unwrap().is_empty());
}

#[tokio::test]
async fn put_meta_rewrite_preserves_container_children() {
    // Regression for the roborev finding: a metadata re-write on a container must NOT erase its
    // `ldp:contains` children (the earlier `DROP SILENT GRAPH` would have).
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(CONTAINER, meta()).await.unwrap();
    c.create_child(CONTAINER, CHILD, meta()).await.unwrap();
    assert_eq!(c.list_children(CONTAINER).await.unwrap().len(), 1);

    // Re-write the container's metadata (e.g. a new ETag) — children must survive.
    let updated = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "blob-key-2".into(),
        etag: "\"etag-2\"".into(),
        last_modified: None,
    };
    c.put_meta(CONTAINER, updated.clone()).await.unwrap();
    assert_eq!(
        c.list_children(CONTAINER).await.unwrap(),
        vec![CHILD.to_string()],
        "put_meta must not erase containment edges"
    );
    assert_eq!(c.get_meta(CONTAINER).await.unwrap(), updated);
}

#[tokio::test]
async fn delete_removes_the_resource() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(RES, meta()).await.unwrap();
    assert!(c.exists(RES).await.unwrap());
    c.delete_meta(RES).await.unwrap();
    assert!(!c.exists(RES).await.unwrap());
}

#[tokio::test]
async fn delete_meta_is_idempotent_on_absent() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    // DROP SILENT on an absent graph is a no-op success.
    c.delete_meta(RES).await.unwrap();
}

#[tokio::test]
async fn delete_meta_if_empty_over_http_reports_all_three_outcomes() {
    // The live client's atomic empty-container delete (a single `DELETE { } INSERT { } WHERE { }`
    // modify + a race-resistant delete-marker confirm) must map to NotFound / NotEmpty / Deleted
    // correctly via the now-sequencing-faithful mock — and crucially leave a NON-EMPTY container
    // untouched (the safety invariant). Because the mock now models SPARQL 1.1 sequencing, a Deleted
    // here is REAL evidence the marker was written by a single-WHERE evaluation against the pre-delete
    // state (the round's HIGH: the old two-statement form would have produced NotFound here).
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);

    // Absent ⇒ NotFound (no marker written ⇒ confirm ASK false ⇒ exists ASK false).
    assert_eq!(
        c.delete_meta_if_empty(CONTAINER, None).await.unwrap(),
        DeleteOutcome::NotFound
    );

    // Populated ⇒ NotEmpty, and the container + child both survive (nothing deleted).
    c.put_meta(CONTAINER, meta()).await.unwrap();
    c.create_child(CONTAINER, CHILD, meta()).await.unwrap();
    assert_eq!(
        c.delete_meta_if_empty(CONTAINER, None).await.unwrap(),
        DeleteOutcome::NotEmpty
    );
    assert!(
        c.exists(CONTAINER).await.unwrap(),
        "a NotEmpty result must not delete the container"
    );
    assert_eq!(
        c.list_children(CONTAINER).await.unwrap(),
        vec![CHILD.to_string()],
        "a NotEmpty result must leave the membership intact"
    );

    // Empty it, then Deleted ⇒ the container's record is gone.
    c.remove_child(CONTAINER, CHILD).await.unwrap();
    assert_eq!(
        c.delete_meta_if_empty(CONTAINER, None).await.unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!c.exists(CONTAINER).await.unwrap());
}

#[tokio::test]
async fn delete_modify_writes_the_marker_but_a_two_statement_form_does_not() {
    // NON-VACUOUS MUTATION-CHECK for the round's HIGH, driving the sequencing-faithful mock's
    // `apply_update` directly (no HTTP round-trip needed — the mock IS the system under test for the
    // sequencing semantics that the High exploited).
    // (1) The REAL builder output (a SINGLE `DELETE { } INSERT { } WHERE { }` modify) writes the delete
    //     marker — its WHERE is evaluated once, pre-delete, so the marker INSERT shares the guard match.
    // (2) The PRE-FIX shape (two `;`-joined statements: `DELETE …WHERE… ; INSERT …WHERE…`) does NOT
    //     write the marker — the mock applies the DELETE first (emptying the container), then
    //     re-evaluates the INSERT's WHERE against the now-empty store, where it fails. This is exactly
    //     the bug; were the mock NOT sequencing-faithful, both forms would (wrongly) write the marker,
    //     so this assertion is what makes the new mock faithfulness load-bearing.

    // --- (1) the real single-modify writes the marker ---
    let state = MockState {
        mode: Arc::new(Mutex::new(MockMode::default())),
        store: Arc::new(Mutex::new(MockStore::default())),
    };
    state.store.lock().unwrap().meta.insert(
        CONTAINER.to_string(),
        (
            "text/turtle".into(),
            "blob-key-1".into(),
            "\"etag-1\"".into(),
        ),
    );
    let real =
        sparq_lws_core::store::sparql::update_delete_container_if_empty(CONTAINER, None, "op-real")
            .unwrap();
    // Shape sanity: the real builder MUST emit a single modify (no top-level `;`), else this test is
    // not exercising the fix.
    assert!(
        !real.contains(';'),
        "the real builder must emit a single modify (no `;`): {real}"
    );
    apply_update(&state, &real);
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .delete_markers
            .get(CONTAINER)
            .is_some_and(|v| v.contains(&"op-real".to_string())),
        "the single-modify form must write the delete marker (one WHERE, evaluated pre-delete)"
    );

    // --- (2) the PRE-FIX two-statement form does NOT write its marker (the regression the High was) ---
    let state2 = MockState {
        mode: Arc::new(Mutex::new(MockMode::default())),
        store: Arc::new(Mutex::new(MockStore::default())),
    };
    state2.store.lock().unwrap().meta.insert(
        CONTAINER.to_string(),
        (
            "text/turtle".into(),
            "blob-key-1".into(),
            "\"etag-1\"".into(),
        ),
    );
    let two_statement = two_statement_delete(CONTAINER, "op-buggy");
    assert!(
        two_statement.matches("WHERE {").count() == 2 && two_statement.contains(" ; "),
        "the buggy fixture must be the two-statement `;`-joined form: {two_statement}"
    );
    apply_update(&state2, &two_statement);
    assert!(
        state2
            .store
            .lock()
            .unwrap()
            .delete_markers
            .get(CONTAINER)
            .is_none_or(|v| !v.contains(&"op-buggy".to_string())),
        "the two-statement form must NOT write the marker (DELETE empties the graph before the \
         INSERT's WHERE re-evaluates) — the mutation-check proving the test is non-vacuous"
    );
    // ...and the container's record IS gone (the first statement's DELETE ran), so a client using this
    // buggy shape would see no marker + no record ⇒ wrongly report NotFound for a real delete.
    assert!(
        !state2.store.lock().unwrap().meta.contains_key(CONTAINER),
        "the two-statement form's DELETE still empties the container — so the missing marker mis-maps \
         a genuine delete to NotFound (exactly the High)"
    );
}

/// The PRE-FIX BUGGY shape, as a fixture for the mutation-check above: TWO `;`-joined operations —
/// `DELETE { graph } WHERE { exists+empty guard } ; INSERT { marker } WHERE { exists+empty guard }`.
/// Mirrors the predicates/graph IRIs the real builder uses so the mock recognises it, but as two
/// statements (the shape this round replaced). Built by hand here ONLY to prove the test would catch a
/// regression to it — the production builder never emits this.
fn two_statement_delete(container: &str, nonce: &str) -> String {
    use sparq_lws_core::store::sparql::{
        g_delete_markers, iri, p_content_type, p_delete_marker, s_record, LDP_CONTAINS,
    };
    // All known-valid IRIs (constants + the test container), so `iri(...).unwrap()` is the public
    // equivalent of the crate-private `iri_const` wrapper.
    let g = iri(container).unwrap();
    let rec = iri(&s_record()).unwrap();
    let pct = iri(&p_content_type()).unwrap();
    let contains = iri(LDP_CONTAINS).unwrap();
    let mg = iri(&g_delete_markers()).unwrap();
    let pmark = iri(&p_delete_marker()).unwrap();
    format!(
        "DELETE {{ GRAPH {g} {{ ?s ?p ?o }} }} WHERE {{ \
            GRAPH {g} {{ ?s ?p ?o }} \
            GRAPH {g} {{ {rec} {pct} ?anyCt }} \
            FILTER NOT EXISTS {{ GRAPH {g} {{ {rec} {contains} ?anyChild }} }} \
         }} ; \
         INSERT {{ GRAPH {mg} {{ {g} {pmark} \"{nonce}\" }} }} WHERE {{ \
            GRAPH {g} {{ {rec} {pct} ?anyCt2 }} \
            FILTER NOT EXISTS {{ GRAPH {g} {{ {rec} {contains} ?anyChild2 }} }} \
         }}",
    )
}

#[tokio::test]
async fn delete_meta_if_empty_over_http_detaches_the_parent_edge_atomically() {
    // FINDING 2 over HTTP: a single `delete_meta_if_empty(container, Some(parent))` both deletes the
    // empty sub-container AND detaches `<parent> ldp:contains <container>` in the ONE modify — no
    // separate remove_child. After Deleted, the parent no longer lists the sub-container.
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    let parent = "https://pod.example/alice/";
    let container = "https://pod.example/alice/sub/";
    c.put_meta(parent, meta()).await.unwrap();
    c.create_child(parent, container, meta()).await.unwrap();
    assert_eq!(
        c.list_children(parent).await.unwrap(),
        vec![container.to_string()],
        "the parent lists the empty sub-container before the delete"
    );

    assert_eq!(
        c.delete_meta_if_empty(container, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!c.exists(container).await.unwrap());
    assert!(
        c.list_children(parent).await.unwrap().is_empty(),
        "the parent edge is detached in the SAME atomic modify (no separate remove_child)"
    );
}

#[tokio::test]
async fn remove_child_detaches_membership() {
    let (url, _state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    c.put_meta(CONTAINER, meta()).await.unwrap();
    c.create_child(CONTAINER, CHILD, meta()).await.unwrap();
    assert_eq!(c.list_children(CONTAINER).await.unwrap().len(), 1);
    c.remove_child(CONTAINER, CHILD).await.unwrap();
    assert!(c.list_children(CONTAINER).await.unwrap().is_empty());
}

#[tokio::test]
async fn server_5xx_is_a_retryable_backend_error() {
    let (url, state) = spawn_mock().await;
    state.mode.lock().unwrap().force_status = Some(StatusCode::SERVICE_UNAVAILABLE);
    let c = HttpSparqClient::new(url);
    let err = c.exists(RES).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("retryable:"), "got: {msg}"),
        other => panic!("expected retryable Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn server_5xx_with_a_body_is_still_retryable() {
    // Regression: classification is from the STATUS (header) first, so a 5xx carrying a non-trivial
    // body is still retryable — the body read never flips it to a fatal `Body`/`Malformed`.
    let (url, state) = spawn_mock().await;
    {
        let mut m = state.mode.lock().unwrap();
        m.force_status = Some(StatusCode::INTERNAL_SERVER_ERROR);
        m.force_status_with_body = true;
    }
    let c = HttpSparqClient::new(url);
    let err = c.exists(RES).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("retryable:"), "got: {msg}"),
        other => panic!("expected retryable Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn list_children_row_missing_binding_is_fatal() {
    // Regression: a SELECT-children row missing the `child` binding is a malformed backend response,
    // surfaced as a fatal error — NOT silently dropped (which could shorten the list + wrongly let a
    // non-empty container be deleted).
    let (url, state) = spawn_mock().await;
    state.mode.lock().unwrap().children_row_missing_binding = true;
    let c = HttpSparqClient::new(url);
    let err = c.list_children(CONTAINER).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("fatal:"), "got: {msg}"),
        other => panic!("expected fatal Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn client_4xx_is_a_fatal_backend_error() {
    let (url, state) = spawn_mock().await;
    state.mode.lock().unwrap().force_status = Some(StatusCode::BAD_REQUEST);
    let c = HttpSparqClient::new(url);
    let err = c.exists(RES).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("fatal:"), "got: {msg}"),
        other => panic!("expected fatal Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_response_is_a_fatal_error() {
    let (url, state) = spawn_mock().await;
    state.mode.lock().unwrap().malformed = true;
    let c = HttpSparqClient::new(url);
    let err = c.exists(RES).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("fatal:"), "got: {msg}"),
        other => panic!("expected fatal Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn a_slow_endpoint_times_out_retryably() {
    let (url, state) = spawn_mock().await;
    state.mode.lock().unwrap().delay = Some(Duration::from_millis(500));
    let c = HttpSparqClient::with_timeout(url, Duration::from_millis(50));
    let err = c.exists(RES).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("retryable:"), "got: {msg}"),
        other => panic!("expected retryable timeout Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn an_injection_crafted_iri_is_rejected_fail_closed() {
    let (url, state) = spawn_mock().await;
    let c = HttpSparqClient::new(url);
    // A resource IRI crafted to break out of the IRI term and inject a DROP. It is an INVALID IRIREF
    // (contains `>`, spaces, `{`), so the client REJECTS it fail-closed (a fatal Backend error) — NO
    // request is ever sent, so the injection can never reach the endpoint. Rejecting (not escaping) is
    // what keeps the IRI->graph mapping injective (no aliasing).
    let attack = "https://pod.example/x> } ; DROP GRAPH <https://victim> ; ASK { GRAPH <https://pod.example/x";
    let err = c.exists(attack).await.unwrap_err();
    match err {
        SparqError::Backend(msg) => assert!(msg.starts_with("fatal:"), "got: {msg}"),
        other => panic!("expected fatal Backend (rejected IRI), got {other:?}"),
    }
    // No request reached the mock — the build failed before any HTTP call.
    assert!(
        state.store.lock().unwrap().last_sparql.is_none(),
        "a rejected IRI must never hit the wire"
    );
}

// Confirm the unused-`Infallible` import is intentional plumbing (axum handler return types use it
// transitively); keep this so clippy/-D warnings does not flag an unused import in some toolchains.
#[allow(dead_code)]
fn _assert_infallible_is_used() -> Result<(), Infallible> {
    Ok(())
}

// ---------------------------------------------------------------------------
// LIVE integration test (ignored): needs a real SPARQ.
// ---------------------------------------------------------------------------

/// End-to-end against a REAL SPARQ `/sparql` endpoint. `#[ignore]` + env-gated: provide
/// `PSS_LIVE_SPARQ_URL` (e.g. `http://localhost:8080/sparql`) and run with
/// `cargo test --test http_sparq_client -- --ignored`. Standing up a SPARQ instance is a
/// `needs:user` item, so this never runs in the standard `cargo test` gate.
#[tokio::test]
#[ignore = "needs a live SPARQ instance (set PSS_LIVE_SPARQ_URL); needs:user"]
async fn live_sparq_round_trip() {
    let url = match std::env::var("PSS_LIVE_SPARQ_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("PSS_LIVE_SPARQ_URL not set; skipping live SPARQ test");
            return;
        }
    };
    let c = HttpSparqClient::new(url);

    let parent = "https://live.example/";
    let container = "https://live.example/c/";
    let child = "https://live.example/c/item1";
    let m = ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: "live-blob-1".into(),
        etag: "\"live-etag-1\"".into(),
        last_modified: None,
    };

    // Clean slate.
    c.delete_meta(child).await.ok();
    c.delete_meta(container).await.ok();
    c.delete_meta(parent).await.ok();

    // exists false → put → exists true → get round-trips.
    assert!(!c.exists(container).await.unwrap());
    c.put_meta(container, m.clone()).await.unwrap();
    assert!(c.exists(container).await.unwrap());
    assert_eq!(c.get_meta(container).await.unwrap(), m);

    // Atomic create_child + listing.
    c.create_child(container, child, m.clone()).await.unwrap();
    assert!(c.exists(child).await.unwrap());
    let kids = c.list_children(container).await.unwrap();
    assert!(kids.contains(&child.to_string()), "kids: {kids:?}");

    // create_child into a missing container ⇒ NotFound.
    let err = c
        .create_child(
            "https://live.example/missing/",
            "https://live.example/missing/x",
            m.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, SparqError::NotFound));

    // Atomic empty-container delete (NO parent): while the child is present it is NotEmpty (nothing
    // deleted); after detaching the child it is Deleted; a second delete of the now-absent container is
    // NotFound. This asserts all THREE outcomes against the REAL single-modify endpoint — the marker is
    // reliably written on a genuine delete (the round's HIGH would have returned NotFound for Deleted).
    assert_eq!(
        c.delete_meta_if_empty(container, None).await.unwrap(),
        DeleteOutcome::NotEmpty
    );
    assert!(c.exists(container).await.unwrap());
    c.remove_child(container, child).await.unwrap();
    assert!(c.list_children(container).await.unwrap().is_empty());
    c.delete_meta(child).await.unwrap();
    assert_eq!(
        c.delete_meta_if_empty(container, None).await.unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!c.exists(container).await.unwrap());
    assert_eq!(
        c.delete_meta_if_empty(container, None).await.unwrap(),
        DeleteOutcome::NotFound
    );

    // FINDING 2 against the REAL endpoint: the parent-edge detach is folded into the ONE modify. Index
    // a parent that contains an empty sub-container, then a single `delete_meta_if_empty(sub, parent)`
    // deletes the sub AND detaches the parent edge — the parent must no longer list it.
    c.put_meta(parent, m.clone()).await.unwrap();
    c.create_child(parent, container, m.clone()).await.unwrap();
    assert!(
        c.list_children(parent)
            .await
            .unwrap()
            .contains(&container.to_string()),
        "parent lists the empty sub-container before the atomic delete"
    );
    assert_eq!(
        c.delete_meta_if_empty(container, Some(parent))
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!c.exists(container).await.unwrap());
    assert!(
        c.list_children(parent).await.unwrap().is_empty(),
        "the parent edge is detached in the SAME atomic modify (no separate remove_child)"
    );

    // Clean up the parent.
    c.delete_meta(parent).await.ok();
}
