//! LWS query dataset assembly. Protocol semantics are shared with the Solid endpoint.

use std::collections::BTreeSet;
use std::time::Instant;

use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use oxrdf::{NamedNode, Term};

use super::{access, method_not_allowed, names_record, problem, Agent, LwsRequest, LwsState};
use crate::ldp::content::{classify, parse_to_triples};
use crate::sparql_protocol::{self as protocol, AuthorizedDataset};
use crate::store::Store;

pub(super) const PATH: &str = "/.lws/sparql";

pub(super) async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    if !matches!(req.method, Method::GET | Method::POST) {
        return method_not_allowed("GET, POST");
    }
    let timeout = state.cfg.sparql_timeout;
    let deadline = timeout.map(|duration| Instant::now() + duration);
    let run = execute(state, req, agent, deadline);
    let mut response = match timeout {
        Some(timeout) => tokio::time::timeout(timeout, run)
            .await
            .unwrap_or_else(|_| timed_out()),
        None => run.await,
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::VARY,
        header::HeaderValue::from_static("Accept, Origin, Authorization"),
    );
    response
}

fn timed_out() -> Response {
    problem(StatusCode::GATEWAY_TIMEOUT, Some("query deadline exceeded"))
}

async fn execute<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    deadline: Option<Instant>,
) -> Response {
    // Refuse every update binding, including an update placed in a query field. Solid retains
    // its existing protocol errors; LWS specifies 405 for update operations.
    let form = req
        .header(header::CONTENT_TYPE)
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    let update_param = |bytes: &[u8]| form_urlencoded::parse(bytes).any(|(key, _)| key == "update");
    if form.eq_ignore_ascii_case("application/sparql-update")
        || update_param(req.query.as_deref().unwrap_or_default().as_bytes())
        || (form.eq_ignore_ascii_case("application/x-www-form-urlencoded")
            && update_param(&req.body))
    {
        return method_not_allowed("GET, POST");
    }
    let request = if req.method == Method::GET {
        protocol::parse_parameters(req.query.as_deref().unwrap_or_default().as_bytes())
    } else {
        protocol::parse_post(req.query.as_deref(), &req.headers, &req.body)
    };
    let request = match request {
        Ok(request) => request,
        Err(error) => return protocol_error(error),
    };
    if request.is_update() {
        return method_not_allowed("GET, POST");
    }
    let (query, versions) = match protocol::parse_query(&request) {
        Ok(parsed) => parsed,
        Err(error) => return protocol_error(error),
    };
    // One read lease for the whole request, never one lock per candidate resource. All IRI
    // mutation locks and tracked writes share this barrier, including detached rollback work.
    let snapshot = state.store.generations().snapshot.read().await;
    let authorized = match assemble(state, agent).await {
        Ok(dataset) => dataset,
        Err(response) => return response,
    };
    let headers = req.headers.clone();
    let budget = sparq_engine::QueryBudget {
        deadline,
        ..sparq_engine::QueryBudget::unlimited()
    };
    // The lease follows evaluation even if the request times out or disconnects.
    match tokio::task::spawn_blocking(move || {
        let _snapshot = snapshot;
        protocol::evaluate(&headers, query, versions, &authorized, &budget)
    })
    .await
    {
        Ok(Ok(response)) => response,
        _ if deadline.is_some_and(|end| Instant::now() >= end) => timed_out(),
        Ok(Err(error)) => protocol_error(error),
        Err(_) => problem(StatusCode::INTERNAL_SERVER_ERROR, None),
    }
}

fn protocol_error(error: crate::error::ServerError) -> Response {
    // Diagnostics from storage or resource parsing are never sent to a query caller.
    let status = error.into_response().status();
    problem(status, None)
}

#[allow(clippy::result_large_err)]
async fn assemble<S: Store + 'static>(
    state: &LwsState<S>,
    agent: &Agent,
) -> Result<AuthorizedDataset, Response> {
    let root = state.cfg.storage();
    let services = state.cfg.absolute("/.lws");
    let well_known = state.cfg.absolute("/.well-known");
    let mut queue = vec![root.clone()];
    let mut seen = BTreeSet::new();
    let mut graph_names = Vec::new();
    let mut nquads = String::new();
    let mut parsed_bytes = 0usize;
    while let Some(uri) = queue.pop() {
        if !uri.starts_with(&root)
            || names_record(&uri)
            || uri == services
            || uri.starts_with(&format!("{services}/"))
            || uri == well_known
            || uri.starts_with(&format!("{well_known}/"))
            || !seen.insert(uri.clone())
        {
            continue;
        }
        // Enumeration is authoritative, independent of whether a container itself is readable:
        // a grant to a child must not require a separate grant to each ancestor.
        if uri.ends_with('/') {
            if let Ok(children) = state.store.list_children(&uri).await {
                queue.extend(children.into_iter().map(|child| child.as_str().to_owned()));
            }
        }
        if !state.visible(&uri)
            || uri.is_empty() && !access::allowed(state, access::Action::Read, &uri, agent)
                .await
                .unwrap_or(false)
        {
            continue;
        }
        let Ok(Some(meta)) = state.store.meta(&uri).await else {
            continue;
        };
        if graph_names.len() >= state.cfg.sparql_max_resources {
            return Err(problem(
                StatusCode::INSUFFICIENT_STORAGE,
                Some("query resource limit exceeded"),
            ));
        }
        let Ok(graph_name) = NamedNode::new(uri.clone()) else {
            continue;
        };
        if !uri.ends_with('/') {
            if let Ok(format) = classify(Some(&meta.content_type)) {
                // Bound the representation bytes before parsing.
                let Ok(resource) = state.store.read(&uri).await else {
                    continue;
                };
                if resource.body.len() > state.cfg.sparql_max_bytes.saturating_sub(parsed_bytes) {
                    return Err(problem(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Some("query RDF byte limit exceeded"),
                    ));
                }
                parsed_bytes += resource.body.len();
                if let Ok(triples) = parse_to_triples(format, &resource.body, &uri) {
                    for triple in protocol::scope_blank_nodes(triples, graph_names.len()) {
                        protocol::write_quad(&mut nquads, &triple, &graph_name);
                    }
                }
            }
        }
        // Empty RDF, malformed RDF, non-RDF data and containers all have a graph name.
        graph_names.push(uri);
    }
    let mut graph = sparq_core::Graph::load_dataset(&nquads, "nquads")
        .map_err(|_| problem(StatusCode::INTERNAL_SERVER_ERROR, None))?;
    for name in &graph_names {
        graph
            .ensure_named(&Term::NamedNode(NamedNode::new_unchecked(name.clone())))
            .map_err(|_| problem(StatusCode::INTERNAL_SERVER_ERROR, None))?;
    }
    Ok(AuthorizedDataset { graph, graph_names })
}

#[cfg(test)]
mod tests {
    use super::super::{test_store::FlakyStore, LwsConfig};
    use super::*;
    use axum::{
        body::{to_bytes, Body, Bytes},
        http::Request,
        Router,
    };
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;

    const ROOT: &str = "https://pod.example/";
    const QUERY: &str = "SELECT ?s ?v { GRAPH ?g { ?s <urn:v> ?v } } ORDER BY ?s";

    async fn fixture(timeout: Duration) -> (LwsState<FlakyStore>, Router) {
        let mut cfg = LwsConfig::new(ROOT);
        cfg.open = true;
        cfg.sparql_timeout = Some(timeout);
        let state = LwsState::new(FlakyStore::new(), cfg).await.unwrap();
        for name in ["a", "b"] {
            state
                .store
                .create_in_container(
                    ROOT,
                    &format!("{ROOT}{name}"),
                    Bytes::from(format!("<urn:{name}> <urn:v> 0 .")),
                    "text/turtle",
                )
                .await
                .unwrap();
        }
        let app = Router::new()
            .fallback(super::super::dispatch::<FlakyStore>)
            .with_state(state.clone());
        (state, app)
    }

    async fn query(app: Router) -> Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(PATH)
                .header("content-type", "application/sparql-query")
                .body(Body::from(QUERY))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn wait_read(state: &LwsState<FlakyStore>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while state
                .store
                .hold_next_read_reply_of
                .lock()
                .unwrap()
                .is_some()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_blocks_writer_from_enumeration_through_evaluation() {
        let (state, app) = fixture(Duration::from_secs(5)).await;
        // The traversal visits b before a. Stop after b's bytes were read, then rewrite a.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *state.store.hold_next_read_reply_of.lock().unwrap() =
            Some((format!("{ROOT}b"), gate.clone()));
        let reader = tokio::spawn(query(app.clone()));
        wait_read(&state).await;
        let mut writer = tokio::spawn(async move {
            app.oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/a")
                    .header("content-type", "text/turtle")
                    .body(Body::from("<urn:a> <urn:v> 1 ."))
                    .unwrap(),
            )
            .await
            .unwrap()
        });
        let early = tokio::time::timeout(Duration::from_millis(50), &mut writer).await;
        gate.add_permits(1);
        assert!(
            early.is_err(),
            "writer must wait for the whole query generation"
        );
        let response = reader.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        let rows = value["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["v"]["value"] == "0"));
        assert!(writer.await.unwrap().status().is_success());
        assert_eq!(
            state.store.read(&format!("{ROOT}a")).await.unwrap().body,
            "<urn:a> <urn:v> 1 ."
        );
    }

    #[tokio::test]
    async fn snapshot_wait_and_assembly_use_request_deadline() {
        let (state, app) = fixture(Duration::from_millis(500)).await;
        let held = state.locks.lock(&format!("{ROOT}a")).await.unwrap();
        let response = query(app.clone()).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            super::super::PROBLEM_JSON
        );
        drop(held);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *state.store.hold_next_read_reply_of.lock().unwrap() = Some((format!("{ROOT}a"), gate));
        let reader = tokio::spawn(query(app.clone()));
        wait_read(&state).await;
        let response = reader.await.unwrap();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert!(state.store.generations().snapshot.try_mutate().is_some());
        assert_eq!(query(app).await.status(), StatusCode::OK);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_writer_keeps_generation_after_request_cancellation() {
        let (state, app) = fixture(Duration::from_millis(40)).await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *state.store.hold_next_write_of.lock().unwrap() = Some((format!("{ROOT}a"), gate.clone()));
        let writer_app = app.clone();
        let writer = tokio::spawn(async move {
            writer_app
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri("/a")
                        .header("content-type", "text/turtle")
                        .body(Body::from("<urn:a> <urn:v> 1 ."))
                        .unwrap(),
                )
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.store.hold_next_write_of.lock().unwrap().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        writer.abort();
        assert_eq!(
            query(app.clone()).await.status(),
            StatusCode::GATEWAY_TIMEOUT
        );
        gate.add_permits(1);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if query(app.clone()).await.status() == StatusCode::OK {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state.store.read(&format!("{ROOT}a")).await.unwrap().body,
            "<urn:a> <urn:v> 1 ."
        );
    }

    #[tokio::test]
    async fn malformed_rdf_keeps_only_its_empty_graph_name() {
        let (state, app) = fixture(Duration::from_secs(5)).await;
        state
            .store
            .write(
                &format!("{ROOT}a"),
                Bytes::from_static(b"invalid Turtle"),
                "text/turtle",
            )
            .await
            .unwrap();
        let response = query(app.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["results"]["bindings"].as_array().unwrap().len(), 1);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(PATH)
                    .header("content-type", "application/sparql-query")
                    .body(Body::from("SELECT ?g { GRAPH ?g {} }"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert!(value["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["g"]["value"] == format!("{ROOT}a")));
    }

    #[tokio::test]
    async fn failures_exclude_resources_without_diagnostics() {
        let (state, app) = fixture(Duration::from_secs(5)).await;
        *state.store.fail_read_of.lock().unwrap() = Some(format!("{ROOT}a"));
        let response = query(app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["results"]["bindings"].as_array().unwrap().len(), 1);
        assert!(!String::from_utf8_lossy(&body).contains("urn:a"));
    }
}
