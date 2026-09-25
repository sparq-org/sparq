// [GPT-5.6] sq-r1ei8: WAC-scoped, query-only SPARQL Protocol endpoint.
//!
//! The security boundary is dataset assembly, not query rewriting or result
//! filtering: a resource enters the engine's graph only after the same planned
//! per-resource WAC read decision used by LDP GET returns `Allow`. Any failed
//! enumeration, authorization, byte read, media-type classification, or RDF
//! parse excludes data in the safe direction.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Term, Triple};
use spargebra::algebra::QueryDataset;
use spargebra::{Query, SparqlParser};

use crate::auth::VerifiedToken;
use crate::authz::{is_acl_resource, AccessMode, ReadDecision, WacAuthorizer};
use crate::error::ServerError;
use crate::ldp::content::{classify, parse_to_triples};
use crate::ldp::handler::LdpState;
use crate::store::Store;

/// Solid SPARQL Query's reserved opt-in for the authorized union default graph.
const UNION_DEFAULT_GRAPH_IRI: &str = "http://www.w3.org/ns/solid/sparql#union-default-graph";

const RESULTS_JSON: &str = "application/sparql-results+json";
const N_TRIPLES: &str = "application/n-triples";

#[derive(Default)]
struct ProtocolRequest {
    query: Option<String>,
    default_graphs: Vec<String>,
    named_graphs: Vec<String>,
}

/// `GET /sparql?query=...` (SPARQL 1.1 Protocol query operation).
pub(crate) async fn get_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    let request = parse_parameters(raw.as_deref().unwrap_or_default().as_bytes())?;
    execute(state, token, &headers, request).await
}

/// `POST /sparql`, accepting either a direct `application/sparql-query` body or
/// the protocol's `application/x-www-form-urlencoded` query operation.
pub(crate) async fn post_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ServerError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ServerError::UnsupportedMediaType("missing".to_owned()))?;

    let request = match content_type.as_str() {
        "application/sparql-query" => {
            let mut request = parse_parameters(raw.as_deref().unwrap_or_default().as_bytes())?;
            if request.query.is_some() {
                return Err(ServerError::BadRequest(
                    "a direct query body cannot also carry a query parameter".to_owned(),
                ));
            }
            request.query = Some(
                std::str::from_utf8(&body)
                    .map_err(|_| ServerError::BadRequest("query body is not UTF-8".to_owned()))?
                    .to_owned(),
            );
            request
        }
        "application/x-www-form-urlencoded" => {
            if raw.as_deref().is_some_and(|query| !query.is_empty()) {
                return Err(ServerError::BadRequest(
                    "form query parameters must be carried in the request body".to_owned(),
                ));
            }
            parse_parameters(&body)?
        }
        other => return Err(ServerError::UnsupportedMediaType(other.to_owned())),
    };

    execute(state, token, &headers, request).await
}

async fn execute<S: Store>(
    state: Arc<LdpState<S>>,
    token: VerifiedToken,
    headers: &HeaderMap,
    request: ProtocolRequest,
) -> Result<Response, ServerError> {
    let query_text = request
        .query
        .as_deref()
        .filter(|query| !query.trim().is_empty())
        .ok_or_else(|| ServerError::BadRequest("missing query parameter".to_owned()))?;
    let (mut query, versions) = SparqlParser::new()
        .parse_query_with_versions(query_text)
        .map_err(|error| ServerError::BadRequest(format!("invalid SPARQL query: {error}")))?;
    apply_protocol_dataset(&mut query, &request)?;

    // The request Origin is part of WAC rule matching. Preserve the exact LDP
    // trim/empty semantics without exposing the handler's private helper.
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    // Keep the authoritative index stable from enumeration through evaluation.
    // Every LDP mutation takes the matching write guard, so target bytes, ACLs,
    // and containment cannot be assembled from interleaved server generations.
    let _snapshot = state.sparql_snapshot_read().await;
    let authorized = assemble_authorized_dataset(state.as_ref(), &token, origin).await?;
    expand_union_default(&mut query, &authorized.graph_names)?;
    // [GPT-6] Protocol and authorized-dataset rewrites retain the query contract.
    let prepared = sparq_engine::PreparedQuery::from_query_with_versions(query, versions)
        .map_err(ServerError::BadRequest)?;

    let (content_type, body) = match prepared.query() {
        Query::Select { .. } | Query::Ask { .. } => {
            require_acceptable(headers, RESULTS_JSON)?;
            let json = sparq_engine::query_json_prepared(&authorized.graph, &prepared)
                .map_err(|error| ServerError::BadRequest(format!("query failed: {error}")))?;
            (RESULTS_JSON, json)
        }
        Query::Construct { .. } => {
            require_acceptable(headers, N_TRIPLES)?;
            let triples = sparq_engine::construct_prepared(&authorized.graph, &prepared)
                .map_err(|error| ServerError::BadRequest(format!("query failed: {error}")))?;
            (N_TRIPLES, sparq_engine::triples_to_ntriples(&triples))
        }
        Query::Describe { .. } => {
            return Err(ServerError::BadRequest(
                "the query-only v1 endpoint supports SELECT, ASK, and CONSTRUCT".to_owned(),
            ));
        }
    };

    let mut response = (StatusCode::OK, body).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Accept, Origin"));
    Ok(response)
}

fn parse_parameters(input: &[u8]) -> Result<ProtocolRequest, ServerError> {
    let mut request = ProtocolRequest::default();
    for (name, value) in form_urlencoded::parse(input) {
        match name.as_ref() {
            "query" => {
                if request.query.replace(value.into_owned()).is_some() {
                    return Err(ServerError::BadRequest(
                        "exactly one query parameter is required".to_owned(),
                    ));
                }
            }
            "default-graph-uri" => request.default_graphs.push(value.into_owned()),
            "named-graph-uri" => request.named_graphs.push(value.into_owned()),
            "update" => {
                return Err(ServerError::BadRequest(
                    "SPARQL Update is not supported by the query-only endpoint".to_owned(),
                ));
            }
            _ => {}
        }
    }
    Ok(request)
}

fn apply_protocol_dataset(query: &mut Query, request: &ProtocolRequest) -> Result<(), ServerError> {
    if request.default_graphs.is_empty() && request.named_graphs.is_empty() {
        return Ok(());
    }
    let default = request
        .default_graphs
        .iter()
        .map(|iri| protocol_graph_iri(iri))
        .collect::<Result<Vec<_>, _>>()?;
    // Supplying either protocol dataset parameter replaces the whole query
    // dataset. `Some(empty)` is therefore significant: no named graphs were
    // requested, rather than `None` (the store's complete named graph set).
    let named = Some(
        request
            .named_graphs
            .iter()
            .map(|iri| protocol_graph_iri(iri))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let dataset = QueryDataset { default, named };
    match query {
        Query::Select { dataset: slot, .. }
        | Query::Ask { dataset: slot, .. }
        | Query::Construct { dataset: slot, .. }
        | Query::Describe { dataset: slot, .. } => *slot = Some(dataset),
    }
    Ok(())
}

fn protocol_graph_iri(iri: &str) -> Result<spargebra::term::NamedNode, ServerError> {
    spargebra::term::NamedNode::new(iri.to_owned())
        .map_err(|error| ServerError::BadRequest(format!("invalid graph IRI: {error}")))
}

fn expand_union_default(query: &mut Query, graph_names: &[String]) -> Result<(), ServerError> {
    let dataset = match query {
        Query::Select { dataset, .. }
        | Query::Ask { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. } => dataset.as_mut(),
    };
    let Some(dataset) = dataset else {
        return Ok(());
    };
    if !dataset
        .default
        .iter()
        .any(|name| name.as_str() == UNION_DEFAULT_GRAPH_IRI)
    {
        return Ok(());
    }
    dataset
        .default
        .retain(|name| name.as_str() != UNION_DEFAULT_GRAPH_IRI);
    for graph_name in graph_names {
        let graph_name = protocol_graph_iri(graph_name)?;
        if !dataset.default.contains(&graph_name) {
            dataset.default.push(graph_name);
        }
    }
    Ok(())
}

struct AuthorizedDataset {
    graph: sparq_core::Graph,
    graph_names: Vec<String>,
}

/// Assemble the engine input from physically admitted resources only.
async fn assemble_authorized_dataset<S: Store>(
    state: &LdpState<S>,
    token: &VerifiedToken,
    origin: Option<&str>,
) -> Result<AuthorizedDataset, ServerError> {
    let root = format!("{}/", state.base_url().trim_end_matches('/'));
    let mut queue = vec![root.clone()];
    let mut contained = BTreeSet::new();

    while let Some(resource) = queue.pop() {
        if !resource.starts_with(&root) || !contained.insert(resource.clone()) {
            continue;
        }
        if resource.ends_with('/') {
            // Enumeration is server-authoritative. A failed branch is omitted in
            // the safe direction; it never changes admission of another branch.
            if let Ok(children) = state.store.list_children(&resource).await {
                queue.extend(
                    children
                        .into_iter()
                        .map(|child| child.as_str().to_owned())
                        .filter(|child| child.starts_with(&root)),
                );
            }
        }
    }

    // ACL auxiliaries are deliberately absent from `ldp:contains`; derive each
    // candidate so a Control-holder sees the same ACL resource that LDP GET does.
    let mut candidates = contained.clone();
    for resource in contained {
        if !is_acl_resource(&resource) {
            candidates.insert(format!("{resource}.acl"));
        }
    }

    let wac = WacAuthorizer::with_cache(&state.store, state.base_url(), &state.acl_cache);
    let mut nquads = String::new();
    let mut graph_names = Vec::new();
    for (scope, resource) in candidates.into_iter().enumerate() {
        let required = if is_acl_resource(&resource) {
            AccessMode::Control
        } else {
            AccessMode::Read
        };
        let acl_candidates = wac.read_plan_candidates(&resource);
        let acl_iris = acl_candidates
            .iter()
            .map(|candidate| candidate.acl.clone())
            .collect::<Vec<_>>();
        let Ok(plan) = state.store.read_plan(&resource, &acl_iris).await else {
            continue;
        };
        let decision = wac
            .authorize_read_planned(
                required,
                token.web_id.as_deref(),
                origin,
                &acl_candidates,
                &plan.acls,
            )
            .await;
        let admitted = matches!(decision, Ok(ReadDecision::Allow(_)));
        // [SONNET-4.6] sq-elg47: compose the opt-in ODRL gate per candidate graph — a Deny
        // excludes the graph from the authorized dataset even under a static WAC grant
        // (deny-overrides); a Permit admits a Read-required graph WAC alone would exclude
        // (permit-extends; never the Control-gated `.acl` auxiliaries). Compiled out entirely
        // when the `odrl-authz` feature is off; an unattached gate changes nothing.
        #[cfg(all(feature = "odrl-authz", not(target_arch = "wasm32")))]
        let admitted = match state.odrl_gate.as_deref() {
            Some(gate) => {
                use crate::authz::odrl::OdrlVerdict;
                match gate.decide_read(&resource, token.web_id.as_deref()) {
                    OdrlVerdict::Deny => false,
                    OdrlVerdict::Permit if required == AccessMode::Read => true,
                    OdrlVerdict::Permit | OdrlVerdict::NotApplicable => admitted,
                }
            }
            None => admitted,
        };
        if !admitted {
            continue;
        }
        let Some(meta) = plan.target else {
            continue;
        };
        let Ok(format) = classify(Some(&meta.content_type)) else {
            continue;
        };
        let Ok(body) = state.store.read_at(&resource, &meta).await else {
            continue;
        };
        let Ok(triples) = parse_to_triples(format, &body, &resource) else {
            continue;
        };
        let Ok(graph_name) = NamedNode::new(resource) else {
            continue;
        };
        graph_names.push(graph_name.as_str().to_owned());
        for triple in scope_blank_nodes(triples, scope) {
            write_quad(&mut nquads, &triple, &graph_name);
        }
    }

    let mut graph = sparq_core::Graph::load_dataset(&nquads, "nquads").map_err(|error| {
        ServerError::Storage(format!("authorized dataset build failed: {error}"))
    })?;
    for graph_name in &graph_names {
        graph
            .ensure_named(&Term::NamedNode(NamedNode::new_unchecked(
                graph_name.clone(),
            )))
            .map_err(|error| {
                ServerError::Storage(format!("authorized named graph build failed: {error}"))
            })?;
    }
    Ok(AuthorizedDataset { graph, graph_names })
}

fn write_quad(output: &mut String, triple: &Triple, graph_name: &NamedNode) {
    // `String`'s formatting implementation is infallible.
    let _ = writeln!(
        output,
        "{} {} {} {} .",
        triple.subject, triple.predicate, triple.object, graph_name
    );
}

/// Blank-node labels are document-scoped. Prefix recursively (including RDF 1.2
/// triple terms) so equal source labels in two resources cannot become one node.
fn scope_blank_nodes(triples: Vec<Triple>, scope: usize) -> Vec<Triple> {
    triples
        .into_iter()
        .map(|triple| Triple {
            subject: scope_subject(triple.subject, scope),
            predicate: triple.predicate,
            object: scope_term(triple.object, scope),
        })
        .collect()
}

fn scope_subject(subject: NamedOrBlankNode, scope: usize) -> NamedOrBlankNode {
    match subject {
        NamedOrBlankNode::BlankNode(blank) => {
            NamedOrBlankNode::BlankNode(scope_blank(blank, scope))
        }
        named => named,
    }
}

fn scope_term(term: Term, scope: usize) -> Term {
    match term {
        Term::BlankNode(blank) => Term::BlankNode(scope_blank(blank, scope)),
        Term::Triple(triple) => Term::Triple(Box::new(Triple {
            subject: scope_subject(triple.subject, scope),
            predicate: triple.predicate,
            object: scope_term(triple.object, scope),
        })),
        other => other,
    }
}

fn scope_blank(blank: BlankNode, scope: usize) -> BlankNode {
    BlankNode::new_unchecked(format!("lws{scope}x{}", blank.as_str()))
}

fn require_acceptable(headers: &HeaderMap, produced: &str) -> Result<(), ServerError> {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };
    let produced_type = produced.split_once('/').map(|(kind, _)| kind).unwrap_or("");
    let accepted = accept.split(',').any(|range| {
        let mut parts = range.split(';');
        let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let enabled = !parts.any(|parameter| {
            parameter
                .trim()
                .strip_prefix("q=")
                .is_some_and(|value| value.trim() == "0")
        });
        enabled && (media == "*/*" || media == produced || media == format!("{produced_type}/*"))
    });
    if accepted || accept.trim().is_empty() {
        Ok(())
    } else {
        Err(ServerError::NotAcceptable)
    }
}
