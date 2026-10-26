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
            Err(ServerError::BadRequest(ref message)) if message.starts_with("SPARQL Update") => {
                return method_not_allowed("GET, POST")
            }
            Err(error) => return protocol_error(error),
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
        loop {
            let snapshot = state.store.snapshot().await;
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
