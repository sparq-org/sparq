//! SPARQL Protocol parsing, dataset selection and result serialization shared by both modes.

use crate::error::ServerError;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Term, Triple};
use spargebra::algebra::QueryDataset;
use spargebra::{Query, SparqlParser};
use std::fmt::Write as _;

/// Solid SPARQL Query's reserved opt-in for the authorized union default graph.
const UNION_DEFAULT_GRAPH_IRI: &str = "http://www.w3.org/ns/solid/sparql#union-default-graph";

const RESULTS_JSON: &str = "application/sparql-results+json";
const N_TRIPLES: &str = "application/n-triples";

#[derive(Default)]
pub(crate) struct ProtocolRequest {
    query: Option<String>,
    default_graphs: Vec<String>,
    named_graphs: Vec<String>,
}

pub(crate) fn parse_parameters(input: &[u8]) -> Result<ProtocolRequest, ServerError> {
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

pub(crate) fn write_quad(output: &mut String, triple: &Triple, graph_name: &NamedNode) {
    // `String`'s formatting implementation is infallible.
    let _ = writeln!(
        output,
        "{} {} {} {} .",
        triple.subject, triple.predicate, triple.object, graph_name
    );
}

/// Blank-node labels are document-scoped. Prefix recursively (including RDF 1.2
/// triple terms) so equal source labels in two resources cannot become one node.
pub(crate) fn scope_blank_nodes(triples: Vec<Triple>, scope: usize) -> Vec<Triple> {
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
    let accepted = crate::accept::media_ranges(accept)
        .any(|range| range.q > 0.0 && range.specificity(produced).is_some());
    if accepted || accept.trim().is_empty() {
        Ok(())
    } else {
        Err(ServerError::NotAcceptable)
    }
}

pub(crate) fn parse_post(
    raw: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<ProtocolRequest, ServerError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ServerError::UnsupportedMediaType("missing".to_owned()))?;

    let request = match content_type.as_str() {
        "application/sparql-query" => {
            let mut request = parse_parameters(raw.unwrap_or_default().as_bytes())?;
            if request.query.is_some() {
                return Err(ServerError::BadRequest(
                    "a direct query body cannot also carry a query parameter".to_owned(),
                ));
            }
            request.query = Some(
                std::str::from_utf8(body)
                    .map_err(|_| ServerError::BadRequest("query body is not UTF-8".to_owned()))?
                    .to_owned(),
            );
            request
        }
        "application/x-www-form-urlencoded" => {
            if raw.is_some_and(|query| !query.is_empty()) {
                return Err(ServerError::BadRequest(
                    "form query parameters must be carried in the request body".to_owned(),
                ));
            }
            parse_parameters(body)?
        }
        other => return Err(ServerError::UnsupportedMediaType(other.to_owned())),
    };

    Ok(request)
}

#[derive(Clone)]
pub(crate) struct ParsedQuery {
    query: Query,
    versions: Vec<String>,
}

pub(crate) fn prepare(request: ProtocolRequest) -> Result<ParsedQuery, ServerError> {
    let query_text = request
        .query
        .as_deref()
        .filter(|query| !query.trim().is_empty())
        .ok_or_else(|| ServerError::BadRequest("missing query parameter".to_owned()))?;
    let (mut query, versions) =
        sparq_engine::parse_versioned_query(SparqlParser::new(), query_text)
            .map_err(|error| ServerError::BadRequest(format!("invalid SPARQL query: {error}")))?;
    apply_protocol_dataset(&mut query, &request)?;

    Ok(ParsedQuery { query, versions })
}

pub(crate) struct AuthorizedDataset {
    pub graph: sparq_core::Graph,
    pub graph_names: Vec<String>,
}

pub(crate) fn dataset(
    nquads: &str,
    graph_names: Vec<String>,
) -> Result<AuthorizedDataset, ServerError> {
    let mut graph = sparq_core::Graph::load_dataset(nquads, "nquads").map_err(|error| {
        ServerError::Storage(format!("authorized dataset build failed: {error}"))
    })?;
    for name in &graph_names {
        graph
            .ensure_named(&Term::NamedNode(NamedNode::new_unchecked(name.clone())))
            .map_err(|error| {
                ServerError::Storage(format!("authorized named graph build failed: {error}"))
            })?;
    }
    Ok(AuthorizedDataset { graph, graph_names })
}

pub(crate) fn evaluate(
    mut parsed: ParsedQuery,
    authorized: &AuthorizedDataset,
    headers: &HeaderMap,
    budget: &sparq_engine::QueryBudget,
    describe: bool,
) -> Result<Response, ServerError> {
    let ParsedQuery { query, versions } = &mut parsed;
    expand_union_default(query, &authorized.graph_names)?;
    let prepared = sparq_engine::PreparedQuery::from_query_with_versions(
        query.clone(),
        std::mem::take(versions),
    )
    .map_err(ServerError::BadRequest)?;
    let (content_type, body) = match prepared.query() {
        Query::Select { .. } | Query::Ask { .. } => {
            require_acceptable(headers, RESULTS_JSON)?;
            let json =
                sparq_engine::query_json_prepared_with_budget(&authorized.graph, &prepared, budget)
                    .map_err(|error| ServerError::BadRequest(format!("query failed: {error}")))?;
            (RESULTS_JSON, json)
        }
        Query::Construct { .. } => {
            require_acceptable(headers, N_TRIPLES)?;
            let triples =
                sparq_engine::construct_prepared_with_budget(&authorized.graph, &prepared, budget)
                    .map_err(|error| ServerError::BadRequest(format!("query failed: {error}")))?;
            (N_TRIPLES, sparq_engine::triples_to_ntriples(&triples))
        }
        Query::Describe { .. } if describe => {
            require_acceptable(headers, N_TRIPLES)?;
            let triples =
                sparq_engine::describe_prepared_with_budget(&authorized.graph, &prepared, budget)
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

impl ProtocolRequest {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn is_update(&self) -> bool {
        self.query.as_deref().is_some_and(|text| {
            !text.trim().is_empty() && SparqlParser::new().parse_update(text).is_ok()
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ParsedQuery {
    // The engine can declare a requested FROM NAMED graph even when no stored graph exists.
    // LWS's graph catalog consists exactly of admitted resources, including empty ones.
    pub(crate) fn restrict_named(&mut self, graph_names: &[String]) {
        let dataset = match &mut self.query {
            Query::Select { dataset, .. }
            | Query::Ask { dataset, .. }
            | Query::Construct { dataset, .. }
            | Query::Describe { dataset, .. } => dataset,
        };
        if let Some(QueryDataset {
            named: Some(named), ..
        }) = dataset
        {
            let admitted: std::collections::BTreeSet<&str> =
                graph_names.iter().map(String::as_str).collect();
            named.retain(|name| admitted.contains(name.as_str()));
        }
    }
}
