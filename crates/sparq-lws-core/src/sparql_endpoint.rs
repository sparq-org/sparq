// sq-r1ei8: WAC-scoped, query-only SPARQL Protocol endpoint.
//!
//! The security boundary is dataset assembly, not query rewriting or result
//! filtering: a resource enters the engine's graph only after the same planned
//! per-resource WAC read decision used by LDP GET returns `Allow`. Any failed
//! enumeration, authorization, byte read, media-type classification, or RDF
//! parse excludes data in the safe direction.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{header, HeaderMap};
use axum::response::Response;
use oxrdf::NamedNode;

use crate::auth::VerifiedToken;
use crate::authz::{is_acl_resource, AccessMode, ReadDecision, WacAuthorizer};
use crate::error::ServerError;
use crate::ldp::content::{classify, parse_to_triples};
use crate::ldp::handler::LdpState;
use crate::store::{Store, READ_RACE_RETRIES};

use crate::sparql_protocol::{
    self as protocol, scope_blank_nodes, write_quad, AuthorizedDataset, ProtocolRequest,
};

/// `GET /sparql?query=...` (SPARQL 1.1 Protocol query operation).
pub(crate) async fn get_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    let request = protocol::parse_parameters(raw.as_deref().unwrap_or_default().as_bytes())?;
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
    let request = protocol::parse_post(raw.as_deref(), &headers, &body)?;
    execute(state, token, &headers, request).await
}

async fn execute<S: Store>(
    state: Arc<LdpState<S>>,
    token: VerifiedToken,
    headers: &HeaderMap,
    request: ProtocolRequest,
) -> Result<Response, ServerError> {
    let parsed = protocol::prepare(request)?;
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
    protocol::evaluate(
        parsed,
        &authorized,
        headers,
        &sparq_engine::QueryBudget::unlimited(),
        false,
    )
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
                        .filter(|child| child.starts_with(&root))
                        // A record of the LWS surface is no member, and is never read here.
                        .filter(|child| {
                            !crate::ldp::target::names_linkset_record(child)
                                && !crate::ldp::target::names_lws_service_record(child, &root)
                        }),
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
    'candidates: for (scope, resource) in candidates.into_iter().enumerate() {
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
        let mut restarts = 0;
        let (format, body) = 'attempt: loop {
            let Ok(plan) = state.store.read_plan(&resource, &acl_iris).await else {
                continue 'candidates;
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
            // A governing ACL rewritten (and its bytes reclaimed) under the walk surfaces as
            // `ResourceChanged`. That is NOT a denial: treating it as one would silently drop a
            // readable graph from a successful answer (wrong counts, false ASK). Restart this
            // candidate's plan + authorization like the body path below; 503 once exhausted.
            let decision = match decision {
                Err(ServerError::ResourceChanged) if restarts < READ_RACE_RETRIES => {
                    restarts += 1;
                    continue 'attempt;
                }
                Err(ServerError::ResourceChanged) => return Err(ServerError::ResourceChanged),
                other => other,
            };
            let admitted = matches!(decision, Ok(ReadDecision::Allow(_)));
            // sq-elg47: compose the opt-in ODRL gate per candidate graph — a Deny
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
                continue 'candidates;
            }
            let Some(meta) = plan.target else {
                continue 'candidates;
            };
            // RDF eligibility from the authorized metadata BEFORE any byte fetch: a readable
            // non-RDF body (image, video, …) is never fetched only to be discarded.
            let Ok(format) = classify(Some(&meta.content_type)) else {
                continue 'candidates;
            };
            // `read_at` serves ONLY the version this attempt authorized (so `format` describes
            // exactly these bytes). If a concurrent rewrite reclaimed it, restart this candidate's
            // plan + authorization: the newer version may carry a different ACL.
            match state.store.read_at(&resource, &meta).await {
                Ok(body) => break (format, body),
                Err(ServerError::ResourceChanged) if restarts < READ_RACE_RETRIES => {
                    restarts += 1;
                }
                // Still racing after the bounded restarts: fail the query (503 + Retry-After)
                // rather than silently answer over a dataset missing an authorized graph.
                Err(ServerError::ResourceChanged) => return Err(ServerError::ResourceChanged),
                Err(_) => continue 'candidates,
            }
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

    protocol::dataset(&nquads, graph_names)
}
