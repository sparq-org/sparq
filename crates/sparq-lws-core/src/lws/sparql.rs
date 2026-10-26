//! Access-controlled LWS query view; the protocol implementation is shared with Solid.

use super::{access::Action, method_not_allowed, problem, Agent, LwsRequest, LwsState};
use crate::error::ServerError;
use crate::ldp::content::{classify, parse_to_triples};
use crate::sparql_protocol::{self as protocol, AuthorizedDataset};
use crate::store::Store;
use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use oxrdf::NamedNode;
use std::collections::BTreeSet;

pub(super) const PATH: &str = "/.lws/sparql";

pub(super) async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    let deadline = state
        .cfg
        .sparql_timeout
        .and_then(|within| std::time::Instant::now().checked_add(within));
    let run = async {
        if !matches!(req.method, Method::GET | Method::POST) {
            return method_not_allowed("GET, POST");
        }
        let request = match req.method {
            Method::GET => {
                protocol::parse_parameters(req.query.as_deref().unwrap_or_default().as_bytes())
            }
            _ if req.header(header::CONTENT_TYPE).is_some_and(|ct| {
                ct.split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .eq_ignore_ascii_case("application/sparql-update")
            }) =>
            {
                return method_not_allowed("GET, POST")
            }
            _ => protocol::parse_post(req.query.as_deref(), &req.headers, &req.body),
        };
        let request = match request {
            Ok(request) => request,
            Err(protocol::ProtocolError::Update) => return method_not_allowed("GET, POST"),
            Err(protocol::ProtocolError::Other(error)) => return protocol_error(error),
        };
        if request.is_update() {
            return method_not_allowed("GET, POST");
        }
        let parsed = match protocol::prepare(request) {
            Ok(parsed) => parsed,
            Err(error) => return protocol_error(error),
        };
        let mut budget = sparq_engine::QueryBudget::unlimited();
        budget.deadline = deadline;
        let mut attempts = 0;
        loop {
            let barrier = attempts >= super::snapshot::OPTIMISTIC_ATTEMPTS;
            let Some(snapshot) = state.store.snapshot(barrier).await else {
                attempts += 1;
                tokio::task::yield_now().await;
                continue;
            };
            let assembled = assemble(state, agent, deadline).await;
            let answer = match assembled {
                Ok(dataset) => {
                    let mut selected = parsed.clone();
                    selected.restrict_named(&dataset.graph_names);
                    protocol::evaluate(selected, &dataset, &req.headers, &budget, true)
                        .map_err(protocol_error)
                }
                Err(response) => Err(response),
            };
            if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                return problem(StatusCode::GATEWAY_TIMEOUT, None);
            }
            if snapshot.unchanged() {
                return answer.unwrap_or_else(|response| response);
            }
            // A mutation overlapped assembly or evaluation. Discard the whole attempt and
            // rebuild in a stable generation, authorization included.
            drop(snapshot);
            attempts += 1;
        }
    };
    let mut response = match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline.into(), run)
            .await
            .unwrap_or_else(|_| problem(StatusCode::GATEWAY_TIMEOUT, None)),
        None => run.await,
    };
    super::add_link(
        response.headers_mut(),
        &state.cfg.storage(),
        &format!("{}storage", super::LWS_NS),
        None,
    );
    response
}

fn protocol_error(error: ServerError) -> Response {
    // Storage diagnostics never become a query response. Query syntax/negotiation errors
    // concern only the supplied query and the caller's already assembled view.
    let detail = match &error {
        ServerError::BadRequest(message) | ServerError::UnsupportedMediaType(message) => {
            Some(message.as_str())
        }
        _ => None,
    };
    problem(error.status(), detail)
}

async fn assemble<S: Store + 'static>(
    state: &LwsState<S>,
    agent: &Agent,
    deadline: Option<std::time::Instant>,
) -> Result<AuthorizedDataset, Response> {
    let root = state.cfg.storage();
    let mut queue = vec![root.clone()];
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    let mut nquads = String::new();
    let mut parsed_bytes = 0usize;
    while let Some(uri) = queue.pop() {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return Err(problem(StatusCode::GATEWAY_TIMEOUT, None));
        }
        if !uri.starts_with(&root)
            || super::names_record(&uri)
            || crate::ldp::target::names_lws_service_record(&uri, &root)
            || uri
                .strip_prefix(&root)
                .is_some_and(|p| p.starts_with(".well-known/"))
            || !seen.insert(uri.clone())
            || !state.visible(&uri)
        {
            continue;
        }
        // Traverse containment regardless of permission on the container; membership itself
        // is never emitted, and a child may have its own independent grant.
        if uri.ends_with('/') {
            let Ok(children) = state.store.list_children(&uri).await else {
                continue;
            };
            queue.extend(children.into_iter().map(|child| child.as_str().to_string()));
        }
        if !state.allowed(Action::Read, &uri, agent).await {
            continue;
        }
        let Ok(Some(meta)) = state.store.meta(&uri).await else {
            continue;
        };
        let Ok(name) = NamedNode::new(uri.clone()) else {
            continue;
        };
        let mut triples = Vec::new();
        if !uri.ends_with('/') {
            if let Ok(format) = classify(Some(&meta.content_type)) {
                // Account only authorized RDF, including malformed documents, before parsing.
                let remaining = state.cfg.sparql_max_bytes.saturating_sub(parsed_bytes);
                let Ok(body) = state.store.read_at(&uri, &meta).await else {
                    continue;
                };
                if body.len() > remaining {
                    return Err(problem(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Some("query RDF byte limit exceeded"),
                    ));
                }
                parsed_bytes += body.len();
                triples = parse_to_triples(format, &body, &uri).unwrap_or_default();
            }
        }
        if names.len() >= state.cfg.sparql_max_resources {
            return Err(problem(
                StatusCode::INSUFFICIENT_STORAGE,
                Some("query resource limit exceeded"),
            ));
        }
        let scope = names.len();
        names.push(uri);
        for triple in protocol::scope_blank_nodes(triples, scope) {
            protocol::write_quad(&mut nquads, &triple, &name);
        }
    }
    protocol::dataset(&nquads, names).map_err(protocol_error)
}

#[cfg(test)]
mod tests {
    use super::super::{
        access, records, test_store::FlakyStore, tokens, LwsConfig, ResourceMeta, LWS_JSON,
    };
    use super::*;
    use axum::{
        body::{to_bytes, Body, Bytes},
        http::Request,
        Router,
    };
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use tower::ServiceExt;

    const BASE: &str = "https://progress.example";
    const WRITER: &str = "https://id.example/writer";
    const READER: &str = "https://id.example/reader";

    async fn fixture(timeout: Duration) -> (LwsState<FlakyStore>, Router) {
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some(WRITER.into());
        cfg.sparql_timeout = Some(timeout);
        let state = LwsState::new(FlakyStore::new(), cfg).await.unwrap();
        for name in ["a", "b"] {
            let uri = format!("{BASE}/{name}");
            state
                .store
                .create_in_container(
                    &state.cfg.storage(),
                    &uri,
                    Bytes::from_static(b"<urn:s> <urn:x> 0; <urn:y> 0 ."),
                    "text/turtle",
                )
                .await
                .unwrap();
            state
                .put_resource_meta(
                    &uri,
                    &ResourceMeta {
                        creator: Some(WRITER.into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }
        state.access.put_grant_for_test(
            "read",
            access::policies(
                &json!([
                    {"type":"AccessPolicy","action":["read"],"assignee":READER}
                ]),
                &state.cfg.storage(),
            )
            .unwrap(),
        );
        let app = Router::new()
            .fallback(super::super::dispatch::<FlakyStore>)
            .with_state(state.clone());
        (state, app)
    }

    async fn call(
        state: &LwsState<FlakyStore>,
        app: Router,
        who: &str,
        method: &str,
        path: &str,
        body: &str,
    ) -> Response {
        let token = tokens::mint(&state.cfg, who, "https://app.example/", &state.cfg.realm());
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header(
                    "content-type",
                    if method == "POST" {
                        "application/sparql-query"
                    } else {
                        "text/turtle"
                    },
                )
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn query(state: &LwsState<FlakyStore>, app: Router) -> Response {
        call(
            state,
            app,
            READER,
            "POST",
            PATH,
            "SELECT ?g ?x ?y { GRAPH ?g { ?s <urn:x> ?x; <urn:y> ?y } }",
        )
        .await
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !condition() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    async fn assert_generation(response: Response) {
        assert_eq!(response.status(), StatusCode::OK);
        let data: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        let rows = data["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let names: std::collections::BTreeSet<_> = rows
            .iter()
            .map(|row| row["g"]["value"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from([
                "https://progress.example/a",
                "https://progress.example/b"
            ])
        );
        for row in rows {
            assert_eq!(row["x"]["value"], row["y"]["value"]);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn queries_make_progress_under_sustained_writes() {
        let (state, app) = fixture(Duration::from_secs(2)).await;
        let ready = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        *state.store.write_cycles.lock().unwrap() = Some(super::super::test_store::WriteCycles {
            uris: vec![format!("{BASE}/a"), format!("{BASE}/b")],
            entered: ready.clone(),
            release: release.clone(),
        });
        let mut writers = Vec::new();
        for name in ["a", "b"] {
            let (state, app, stop) = (state.clone(), app.clone(), stop.clone());
            writers.push(tokio::spawn(async move {
                let mut n = 0;
                while !stop.load(Ordering::SeqCst) {
                    n += 1;
                    let response = call(
                        &state,
                        app.clone(),
                        WRITER,
                        "PUT",
                        &format!("/{name}"),
                        &format!("<urn:s> <urn:x> {n}; <urn:y> {n} ."),
                    )
                    .await;
                    assert!(response.status().is_success());
                }
            }));
        }
        ready.acquire_many(2).await.unwrap().forget();
        let reading = {
            let state = state.clone();
            tokio::spawn(async move { query(&state, app).await })
        };
        // One change stays held until the replacement of the other has entered.
        loop {
            release.add_permits(1);
            tokio::select! {
                _ = wait_until(|| state.store.barrier_pending()) => break,
                next = ready.acquire() => next.unwrap().forget(),
                _ = wait_until(|| reading.is_finished()) => break,
            }
        }
        stop.store(true, Ordering::SeqCst);
        release.add_permits(8);
        let response = reading.await.unwrap();
        for writer in writers {
            writer.await.unwrap();
        }
        assert_generation(response).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queries_complete_while_record_settlement_waits() {
        let (state, app) = fixture(Duration::from_millis(200)).await;
        let container = state.cfg.absolute(super::super::REQUESTS_PATH);
        let record = format!("{container}settling");
        state
            .store
            .create_in_container(&container, &record, Bytes::from_static(b"{}"), LWS_JSON)
            .await
            .unwrap();
        let gate = Arc::new(Semaphore::new(0));
        *state.store.unsettle_next_delete_of.lock().unwrap() = Some((record.clone(), gate));
        state.store.hang_fence.store(true, Ordering::SeqCst);
        let removal = {
            let (state, container, record) = (state.clone(), container.clone(), record.clone());
            tokio::spawn(async move {
                records::remove_record(
                    &state,
                    &container,
                    &record,
                    "record",
                    records::Listed::InContainer,
                    records::Remover::Sweeper,
                    None,
                    records::at_once(|| Some(())),
                    |_, _, ()| None,
                )
                .await
            })
        };
        wait_until(|| state.store.fences_tried.load(Ordering::SeqCst) >= 1).await;
        assert!(!removal.is_finished());
        assert_generation(query(&state, app.clone()).await).await;
        wait_until(|| state.store.fences_tried.load(Ordering::SeqCst) >= 2).await;
        assert_generation(query(&state, app).await).await;
        removal.abort();
        let _ = removal.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn top_level_changes_have_a_bounded_barrier_wait() {
        let (state, app) = fixture(Duration::from_secs(5)).await;
        let held = state.store.snapshot(true).await.unwrap();
        let response = tokio::time::timeout(
            records::LOCK_WAIT + Duration::from_secs(1),
            call(
                &state,
                app.clone(),
                WRITER,
                "PUT",
                "/a",
                "<urn:s> <urn:x> 1; <urn:y> 1 .",
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        drop(held);
        assert_generation(query(&state, app).await).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_changes_finish_after_the_request_is_cancelled() {
        let (state, app) = fixture(Duration::from_millis(100)).await;
        let gate = Arc::new(Semaphore::new(0));
        *state.store.hold_next_write_of.lock().unwrap() = Some((format!("{BASE}/a"), gate.clone()));
        let writing = {
            let (state, app) = (state.clone(), app.clone());
            tokio::spawn(async move {
                call(
                    &state,
                    app,
                    WRITER,
                    "PUT",
                    "/a",
                    "<urn:s> <urn:x> 1; <urn:y> 1 .",
                )
                .await
            })
        };
        wait_until(|| state.store.hold_next_write_of.lock().unwrap().is_none()).await;
        writing.abort();
        assert_eq!(
            query(&state, app.clone()).await.status(),
            StatusCode::GATEWAY_TIMEOUT
        );
        gate.add_permits(1);
        wait_until(|| state.locks.try_lock(&format!("{BASE}/a")).is_some()).await;
        assert_generation(query(&state, app).await).await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn query_timeout_releases_the_assembly_barrier() {
        let (state, app) = fixture(Duration::from_millis(250)).await;
        let entered = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let change = {
            let (state, entered, release) = (state.clone(), entered.clone(), release.clone());
            tokio::spawn(async move {
                state
                    .store
                    .change(async {
                        entered.add_permits(1);
                        release.acquire().await.unwrap().forget();
                    })
                    .await
                    .unwrap()
            })
        };
        entered.acquire().await.unwrap().forget();
        let body = Arc::new(Semaphore::new(0));
        *state.store.hold_next_read_reply_of.lock().unwrap() = Some((format!("{BASE}/b"), body));
        let reading = {
            let (state, app) = (state.clone(), app.clone());
            tokio::spawn(async move { query(&state, app).await })
        };
        wait_until(|| state.store.barrier_pending()).await;
        release.add_permits(1);
        change.await.unwrap();
        wait_until(|| {
            state
                .store
                .hold_next_read_reply_of
                .lock()
                .unwrap()
                .is_none()
        })
        .await;
        assert_eq!(reading.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
        assert!(!state.store.barrier_pending());
        let written = call(
            &state,
            app.clone(),
            WRITER,
            "PUT",
            "/a",
            "<urn:s> <urn:x> 1; <urn:y> 1 .",
        )
        .await;
        assert!(written.status().is_success());
        assert_generation(query(&state, app).await).await;
    }

    #[tokio::test]
    async fn failed_enumeration_and_metadata_reads_omit_their_resources() {
        let (state, app) = fixture(Duration::from_secs(5)).await;
        *state.store.fail_read_of.lock().unwrap() =
            Some(super::super::meta_key(&format!("{BASE}/a")));
        let response = query(&state, app.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        let rows = result["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["g"]["value"], format!("{BASE}/b"));
        state.store.fail_list.store(true, Ordering::SeqCst);
        let response = query(&state, app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(result["results"]["bindings"].as_array().unwrap().is_empty());
    }
}
