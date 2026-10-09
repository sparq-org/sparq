//! Storage resources: the storage description, containers, data resources and their linksets
//! (LWS 1.0 core sections 6 to 9 and 12).

// Handlers return the finished error response as the `Err` of their helpers.
#![allow(clippy::result_large_err)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::access::{format_rfc3339, parse_rfc3339, Action};
use super::{
    add_link, encode_meta, is_uri, jose, json_response, meta_key, method_not_allowed, parse_links,
    problem, set, Agent, LwsRequest, LwsState, ResourceMeta, AS_CONTEXT, CID_CONTEXT, JSON,
    LD_JSON, LINKSET_JSON, LWS_CID, LWS_CONTEXT, LWS_JSON, LWS_NS, META_SUFFIX,
};
use crate::error::ServerError;
use crate::store::Store;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const LINKSET_ALLOW: &str = "GET, HEAD";

/// Relations that are server-managed or protocol-level: never taken from a client's Link header
/// as user-managed metadata.
const STRUCTURAL_RELATIONS: &[&str] = &[
    "type",
    "up",
    "linkset",
    "acl",
    "first",
    "prev",
    "next",
    "last",
    "self",
    "describes",
    "storagedescription",
    "https://www.w3.org/ns/lws#storage",
];

pub async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    // A write or a delete runs in a task of its own, spawned before it takes any lock: a client
    // that goes away then cannot cut its store calls off half way, nor release its locks while a
    // call is still pending (see [`hold_locks`]). (A create spawns its writes itself, under the
    // container's shared lock.)
    if matches!(req.method, Method::PUT | Method::DELETE) {
        // The task holds the request, and with it its share of the admission permit
        // (`req.admission`): a write that outlives its request still counts against the
        // concurrency ceiling until it ends.
        let (state, req, agent) = (state.clone(), req.clone(), agent.clone());
        return tokio::spawn(async move { handle_now(&state, &req, &agent).await })
            .await
            .unwrap_or_else(|e| {
                store_error(ServerError::Storage(format!("the request failed: {e}")))
            });
    }
    handle_now(state, req, agent).await
}

async fn handle_now<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    let path = req.path.as_str();
    if let Some(stem) = path.strip_suffix(META_SUFFIX) {
        let uri = state.cfg.absolute(stem);
        return linkset(state, req, agent, &uri).await;
    }
    let uri = state.cfg.absolute(path);
    let is_root = path == "/";
    let accept = req.header(header::ACCEPT).unwrap_or_default();
    // The storage description is public: a client refused with a 401 finds the services through
    // it. The root container's listing,
    // served at the same URI for the LWS container types, is not. Only an explicit request for the
    // description bypasses authorization, so an anonymous probe of the root without Accept still
    // meets the challenge a client discovers the authorization server by.
    // The representation is negotiated once, here and in `read` alike: the bypass holds only when
    // what will be served is the description, never because Accept merely mentions it.
    let description = is_root
        && matches!(req.method, Method::GET | Method::HEAD)
        && names_description(accept)
        && serves_description(Some(accept));
    if !description {
        let action = match req.method {
            Method::GET | Method::HEAD | Method::OPTIONS => Action::Read,
            Method::POST => Action::Create,
            Method::DELETE => Action::Delete,
            _ => Action::Modify,
        };
        // A backend failure is a 500, never "absent": absence skips the permission check. Nothing
        // was authorized on a missing target, so nothing is served or changed: every method on it
        // is a 404, and answering it here (rather than in the handler) means a resource that
        // appears after this check is never read or written without its permission check.
        if let Some(refused) = authorize_unlocked(state, action, &uri, agent).await {
            return refused;
        }
    }
    match req.method {
        Method::GET | Method::HEAD => read(state, req, agent, &uri).await,
        Method::POST => create(state, req, agent, &uri).await,
        Method::PUT => update(state, req, agent, &uri).await,
        Method::DELETE => delete(state, req, agent, &uri).await,
        Method::OPTIONS => options(state, &uri).await,
        _ => method_not_allowed(&allow_for(&uri, is_root)),
    }
}

/// The permission check again, under the resource's lock, against the state the request is about
/// to serve or change. `handle` checks before the lock is taken, and in between another writer may
/// have changed what the decision rests on (the resource's format, its types, its creator after a
/// delete and a re-create).
async fn recheck<S: Store + 'static>(
    state: &LwsState<S>,
    action: Action,
    uri: &str,
    agent: &Agent,
) -> Result<(), Response> {
    match state.check(action, uri, agent).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(state.deny(agent)),
        Err(e) => Err(store_error(e)),
    }
}

/// The existence and permission check a request passes before it takes a lock: `None` when it may
/// go on, else the response refusing it (a missing target is a challenge or a 404, as in
/// [`handle_now`]). The check that counts is repeated under the lock.
async fn authorize_unlocked<S: Store + 'static>(
    state: &LwsState<S>,
    action: Action,
    uri: &str,
    agent: &Agent,
) -> Option<Response> {
    match state.store.exists(uri).await {
        Ok(true) => {}
        Ok(false) if state.needs_auth(agent) => return Some(state.challenge(None)),
        Ok(false) => return Some(problem(StatusCode::NOT_FOUND, None)),
        Err(e) => return Some(store_error(e)),
    }
    recheck(state, action, uri, agent).await.err()
}

fn allow_for(uri: &str, is_root: bool) -> String {
    if is_root {
        "GET, HEAD, OPTIONS, POST".into()
    } else if uri.ends_with('/') {
        "GET, HEAD, OPTIONS, POST, DELETE".into()
    } else {
        "GET, HEAD, OPTIONS, PUT, DELETE".into()
    }
}

async fn options<S: Store + 'static>(state: &LwsState<S>, uri: &str) -> Response {
    match state.store.exists(uri).await {
        Ok(true) => {}
        Ok(false) => return problem(StatusCode::NOT_FOUND, None),
        Err(e) => return store_error(e),
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set(
        resp.headers_mut(),
        header::ALLOW,
        &allow_for(uri, uri == state.cfg.storage()),
    );
    resp
}

// ---- helpers: time, validators, parents ----

fn epoch_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    epoch_ms(SystemTime::now())
}

/// Whole seconds, as HTTP dates have.
fn to_secs(ms: u64) -> u64 {
    ms / 1000
}

fn http_date(secs: u64) -> String {
    httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(secs))
}

fn parse_http_date(v: Option<&str>) -> Option<u64> {
    let t = httpdate::parse_http_date(v?.trim()).ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

/// A strong entity tag, quoted.
fn quoted(tag: &str) -> String {
    if tag.starts_with('"') || tag.starts_with("W/\"") {
        tag.to_string()
    } else {
        format!("\"{tag}\"")
    }
}

/// Whether an If-Match / If-None-Match list matches `etag`. `weak` compares opaque tags only.
fn etag_listed(header: &str, etag: &str, weak: bool) -> bool {
    let strip = |t: &str| t.trim().trim_start_matches("W/").to_string();
    header.split(',').map(str::trim).any(|t| {
        if t == "*" {
            return true;
        }
        if weak {
            strip(t) == strip(etag)
        } else {
            !t.starts_with("W/") && !etag.starts_with("W/") && t == etag
        }
    })
}

/// The outcome of evaluating preconditions (RFC 9110 section 13.2.2).
enum Precondition {
    Proceed,
    NotModified,
    Failed,
}

fn evaluate(
    headers: &HeaderMap,
    etag: Option<&str>,
    modified_secs: Option<u64>,
    read: bool,
) -> Precondition {
    let h = |n: header::HeaderName| headers.get(n).and_then(|v| v.to_str().ok());
    if let Some(im) = h(header::IF_MATCH) {
        if !etag.is_some_and(|e| etag_listed(im, e, false)) {
            return Precondition::Failed;
        }
    } else if let (Some(since), Some(m)) = (
        parse_http_date(h(header::IF_UNMODIFIED_SINCE)),
        modified_secs,
    ) {
        if m > since {
            return Precondition::Failed;
        }
    }
    if let Some(inm) = h(header::IF_NONE_MATCH) {
        if etag.is_some_and(|e| etag_listed(inm, e, true)) {
            return if read {
                Precondition::NotModified
            } else {
                Precondition::Failed
            };
        }
    } else if read {
        if let (Some(since), Some(m)) =
            (parse_http_date(h(header::IF_MODIFIED_SINCE)), modified_secs)
        {
            let now = to_secs(now_ms());
            if since <= now && m <= since {
                return Precondition::NotModified;
            }
        }
    }
    Precondition::Proceed
}

/// Whether the request carries a precondition a state-changing method evaluates.
pub(crate) fn is_conditional(req: &LwsRequest) -> bool {
    [
        header::IF_MATCH,
        header::IF_NONE_MATCH,
        header::IF_UNMODIFIED_SINCE,
    ]
    .iter()
    .any(|h| req.headers.contains_key(h))
}

/// The plain GET of the request's target a conditional state-changing request is evaluated
/// against: the same target, page and `Accept`, no preconditions and no body.
pub(crate) fn plain_get(req: &LwsRequest) -> LwsRequest {
    let mut headers = HeaderMap::new();
    if let Some(accept) = req.headers.get(header::ACCEPT) {
        headers.insert(header::ACCEPT, accept.clone());
    }
    LwsRequest {
        method: Method::GET,
        path: req.path.clone(),
        query: req.query.clone(),
        headers,
        body: Bytes::new(),
        admission: None,
    }
}

/// The validators (`ETag`, `Last-Modified`) a response carries.
pub(crate) fn validators_of(resp: &Response) -> (Option<String>, Option<u64>) {
    let h = |n: header::HeaderName| {
        resp.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    (
        h(header::ETAG),
        parse_http_date(h(header::LAST_MODIFIED).as_deref()),
    )
}

/// The preconditions of a state-changing request against the target's validators: `None` when it
/// may go on, else the 412 refusing it (RFC 9110 section 13.2.2).
pub(crate) fn unless_preconditions(
    req: &LwsRequest,
    etag: Option<&str>,
    modified_secs: Option<u64>,
) -> Option<Response> {
    match evaluate(&req.headers, etag, modified_secs, false) {
        Precondition::Proceed => None,
        Precondition::Failed | Precondition::NotModified => {
            Some(problem(StatusCode::PRECONDITION_FAILED, None))
        }
    }
}

/// The container `uri` is in, or `None` for the storage root.
pub fn parent_of(uri: &str, storage: &str) -> Option<String> {
    if uri == storage {
        return None;
    }
    let trimmed = uri.strip_suffix('/').unwrap_or(uri);
    let cut = trimmed.rfind('/')?;
    let parent = &trimmed[..=cut];
    parent.starts_with(storage).then(|| parent.to_string())
}

fn lws_type(uri: &str) -> String {
    format!(
        "{LWS_NS}{}",
        if uri.ends_with('/') {
            "Container"
        } else {
            "DataResource"
        }
    )
}

/// The links every response about a resource carries: up, the storage, its types (the LWS type and
/// each type the resource declared) and its linkset.
fn resource_links<S: Store>(
    state: &LwsState<S>,
    headers: &mut HeaderMap,
    uri: &str,
    declared_types: &[String],
) {
    if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
        add_link(headers, &parent, "up", None);
    }
    add_link(
        headers,
        &state.cfg.storage(),
        &format!("{LWS_NS}storage"),
        None,
    );
    add_link(headers, &lws_type(uri), "type", None);
    for t in declared_types {
        add_link(headers, t, "type", None);
    }
    add_link(headers, &meta_key(uri), "linkset", Some(LINKSET_JSON));
}

/// The shared lock of the container `uri` is a member of, taken after the member's own (locks go
/// member before container). A change to a member changes its container's listing, so it waits
/// for, and holds off, a create evaluating its preconditions against that listing (which takes
/// the container's lock exclusively). Released before the container is touched, which takes the
/// lock again.
async fn listing_guard<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
    let parent = parent_of(uri, &state.cfg.storage())?;
    Some(state.locks.read(&parent).await)
}

/// Record that a container's membership or a member changed.
///
/// The container's lock is taken shared, as a create in it takes it: a touch after each create
/// must not wait for every other create in flight in the container (nor hold up the ones queued
/// behind it), which taking it exclusively would. What changes the container's metadata
/// otherwise takes the lock exclusively, so it never interleaves with a touch; touches among
/// themselves are ordered by a lock of their own (no lock is taken under it).
///
/// The container's own container lists its modification time, so the touch also holds that
/// listing shared ([`listing_guard`]): a conditional create there sees no touch between its
/// check and its create.
async fn touch_container<S: Store + 'static>(state: &LwsState<S>, container: &str) {
    let _guard = state.locks.read(container).await;
    let _listing = listing_guard(state, container).await;
    let _touch = state.locks.lock(&format!("{container}\0touch")).await;
    // Metadata that cannot be read is left as it is, not replaced by a default.
    let Ok(mut meta) = state.resource_meta(container).await else {
        return;
    };
    meta.modified_ms = Some(now_ms());
    meta.version = Some(jose::random_id());
    let _ = state.put_resource_meta(container, &meta).await;
}

fn store_error(e: ServerError) -> Response {
    match e {
        ServerError::NotFound => problem(StatusCode::NOT_FOUND, None),
        ServerError::Conflict(m) => problem(StatusCode::CONFLICT, Some(&m)),
        ServerError::InsufficientStorage => problem(StatusCode::INSUFFICIENT_STORAGE, None),
        other => problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&other.to_string())),
    }
}

// ---- content negotiation ----

/// One media range of an Accept header.
struct Range {
    essence: String,
    q: f32,
    profile: Option<String>,
}

fn parse_accept(accept: &str) -> Vec<Range> {
    accept
        .split(',')
        .filter_map(|part| {
            let mut pieces = part.split(';');
            let essence = pieces.next()?.trim().to_ascii_lowercase();
            if essence.is_empty() {
                return None;
            }
            let mut q = 1.0;
            let mut profile = None;
            for p in pieces {
                if let Some((k, v)) = p.split_once('=') {
                    let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().trim_matches('"'));
                    if k == "q" {
                        q = v.parse().unwrap_or(0.0);
                    } else if k == "profile" {
                        profile = Some(v.to_string());
                    }
                }
            }
            Some(Range {
                essence,
                q,
                profile,
            })
        })
        .collect()
}

/// The best of `offered` for `accept` (earlier offers win ties); the first offer when Accept is
/// absent; `None` when nothing offered is acceptable.
pub(super) fn negotiate(accept: Option<&str>, offered: &[&str]) -> Option<String> {
    let Some(accept) = accept.filter(|a| !a.trim().is_empty()) else {
        return offered.first().map(|s| s.to_string());
    };
    let ranges = parse_accept(accept);
    let mut best: Option<(f32, usize, usize)> = None; // (q, specificity, offer index)
    for (i, offer) in offered.iter().enumerate() {
        let (otype, _) = offer.split_once('/').unwrap_or((offer, ""));
        let mut q_for: Option<(f32, usize)> = None;
        for r in &ranges {
            let spec = if r.essence == *offer {
                3
            } else if r.essence == format!("{otype}/*") {
                2
            } else if r.essence == "*/*" {
                1
            } else {
                0
            };
            if spec > 0 && q_for.is_none_or(|(_, s)| spec > s) {
                q_for = Some((r.q, spec));
            }
        }
        if let Some((q, spec)) = q_for {
            if q > 0.0 && best.is_none_or(|(bq, _, _)| q > bq) {
                best = Some((q, spec, i));
            }
        }
    }
    best.map(|(_, _, i)| offered[i].to_string())
}

/// `application/ld+json` with the LWS profile, which a container response names as its
/// Content-Type when that is what was asked for.
fn ld_json_lws_profile() -> String {
    format!("{LD_JSON}; profile=\"{LWS_CONTEXT}\"")
}

/// The container media type for `accept`: lws+json, ld+json (with the LWS profile when the request
/// names it), or json. "Servers MUST honor a request for any of these media types and MUST set the
/// Content-Type response header to the requested media type."
fn negotiate_container(accept: Option<&str>) -> Option<String> {
    if let Some(a) = accept {
        if parse_accept(a)
            .iter()
            .any(|r| r.essence == LD_JSON && r.profile.as_deref() == Some(LWS_CONTEXT) && r.q > 0.0)
        {
            return Some(ld_json_lws_profile());
        }
    }
    negotiate(accept, &[LWS_JSON, LD_JSON, JSON])
}

/// Whether Accept names the storage description's media type with a non-zero weight.
fn names_description(accept: &str) -> bool {
    parse_accept(accept)
        .iter()
        .any(|r| r.essence == LWS_CID && r.q > 0.0)
}

/// Whether a request for the storage URI with this Accept gets the storage description rather than
/// the root container's listing. "Requests for the storage URI MUST return a document that conforms
/// to the storage description resource data model with a media type of application/lws+cid, unless
/// content negotiation requires a different format."
fn serves_description(accept: Option<&str>) -> bool {
    let Some(a) = accept.filter(|a| !a.trim().is_empty()) else {
        return true;
    };
    let ranges = parse_accept(a);
    let cid_q = ranges.iter().find(|r| r.essence == LWS_CID).map(|r| r.q);
    if let Some(q) = cid_q {
        // Named explicitly: q=0 refuses it, any other weight selects it.
        return q > 0.0;
    }
    if negotiate_container(Some(a)).is_some()
        && ranges.iter().any(|r| r.essence != "*/*" && r.q > 0.0)
    {
        return false;
    }
    ranges.iter().any(|r| r.essence == "*/*" && r.q > 0.0)
}

// ---- read ----

async fn read<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let storage = state.cfg.storage();
    let accept = req.header(header::ACCEPT);
    if uri == storage {
        // "Requests for the storage URI MUST return a document that conforms to the storage
        // description resource data model with a media type of application/lws+cid, unless content
        // negotiation requires a different format."
        let wants_description = serves_description(accept);
        if wants_description {
            let description = storage_description(state);
            let etag = description_etag(&description);
            let mut resp = match evaluate(&req.headers, Some(&etag), None, true) {
                Precondition::Proceed => json_response(StatusCode::OK, LWS_CID, &description),
                Precondition::NotModified => StatusCode::NOT_MODIFIED.into_response(),
                Precondition::Failed => problem(StatusCode::PRECONDITION_FAILED, None),
            };
            set(resp.headers_mut(), header::ETAG, &etag);
            set(resp.headers_mut(), header::VARY, "Accept");
            add_link(
                resp.headers_mut(),
                &storage,
                &format!("{LWS_NS}storage"),
                None,
            );
            return resp;
        }
    }
    // The shared lock is held from the permission check through the bytes served, so what is
    // served is the state the decision was made on: every write holds the exclusive lock.
    let _guard = state.locks.read(uri).await;
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if let Err(r) = recheck(state, Action::Read, uri, agent).await {
        return r;
    }
    if uri.ends_with('/') {
        return read_container(state, req, uri, &meta).await;
    }
    let types = match state.resource_meta(uri).await {
        Ok(m) => m.types,
        Err(e) => return store_error(e),
    };
    let etag = quoted(&meta.etag);
    let modified = meta.last_modified.map(|t| to_secs(epoch_ms(t)));
    let mut resp = match evaluate(&req.headers, Some(&etag), modified, true) {
        Precondition::Failed => {
            let mut r = problem(StatusCode::PRECONDITION_FAILED, None);
            resource_links(state, r.headers_mut(), uri, &types);
            return r;
        }
        Precondition::NotModified => {
            let mut r = StatusCode::NOT_MODIFIED.into_response();
            set(r.headers_mut(), header::ETAG, &etag);
            if let Some(m) = modified {
                set(r.headers_mut(), header::LAST_MODIFIED, &http_date(m));
            }
            resource_links(state, r.headers_mut(), uri, &types);
            return r;
        }
        Precondition::Proceed => {
            // The bytes come through the same metadata snapshot the validators were taken from, so
            // a concurrent write can never pair the new body with the old entity tag.
            let body = match state.store.read_at(uri, &meta).await {
                Ok(b) => b,
                Err(e) => return store_error(e),
            };
            ranged(req, body, &meta.content_type, &etag, modified)
        }
    };
    let h = resp.headers_mut();
    set(h, header::ETAG, &etag);
    if let Some(m) = modified {
        set(h, header::LAST_MODIFIED, &http_date(m));
    }
    set(h, header::ACCEPT_RANGES, "bytes");
    resource_links(state, h, uri, &types);
    resp
}

/// 200 with the whole body, 206 with one byte range, or 416 for an unsatisfiable single range.
/// A Range the server ignores (a multi-range, a range in another unit, malformed syntax, or one
/// an If-Range validator no longer matches) gets the whole body (RFC 9110 sections 14.2, 13.1.5).
fn ranged(
    req: &LwsRequest,
    body: Bytes,
    content_type: &str,
    etag: &str,
    modified_secs: Option<u64>,
) -> Response {
    let len = body.len() as u64;
    // Range applies to GET only: a HEAD answers with the headers of the whole representation
    // (RFC 9110 section 14.2).
    let range = req
        .header(header::RANGE)
        .filter(|_| req.method == Method::GET)
        .filter(|_| if_range_holds(req.header(header::IF_RANGE), etag, modified_secs));
    match range.map(|r| parse_range(r, len)) {
        Some(ByteRange::Unsatisfiable) => {
            let mut r = problem(StatusCode::RANGE_NOT_SATISFIABLE, None);
            set(
                r.headers_mut(),
                header::CONTENT_RANGE,
                &format!("bytes */{len}"),
            );
            r
        }
        Some(ByteRange::Single(from, to)) => {
            let mut r = (
                StatusCode::PARTIAL_CONTENT,
                body.slice(from as usize..=to as usize),
            )
                .into_response();
            set(r.headers_mut(), header::CONTENT_TYPE, content_type);
            set(
                r.headers_mut(),
                header::CONTENT_RANGE,
                &format!("bytes {from}-{to}/{len}"),
            );
            r
        }
        Some(ByteRange::Ignore) | None => {
            let mut r = (StatusCode::OK, body).into_response();
            set(r.headers_mut(), header::CONTENT_TYPE, content_type);
            r
        }
    }
}

/// Whether an If-Range precondition lets the Range apply (RFC 9110 section 13.1.5): absent, a
/// strong entity tag equal to the current one, or an HTTP date equal to Last-Modified.
fn if_range_holds(if_range: Option<&str>, etag: &str, modified_secs: Option<u64>) -> bool {
    let Some(v) = if_range.map(str::trim) else {
        return true;
    };
    if v.starts_with('"') || v.starts_with("W/") {
        // A weak tag never matches: the comparison is strong.
        return !v.starts_with("W/") && !etag.starts_with("W/") && v == etag;
    }
    parse_http_date(Some(v)).is_some_and(|d| modified_secs == Some(d))
}

/// What a Range header asks of an entity.
#[derive(Debug, PartialEq)]
enum ByteRange {
    /// Serve the whole entity: the header is malformed, in another unit, or names several ranges.
    Ignore,
    /// One satisfiable range `first..=last`.
    Single(u64, u64),
    /// One range that no byte of the entity falls in.
    Unsatisfiable,
}

/// Parse a Range header against an entity of `len` bytes. Only single byte ranges are served; a
/// multi-range request gets the whole entity, which RFC 9110 section 14.2 allows.
fn parse_range(header: &str, len: u64) -> ByteRange {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return ByteRange::Ignore;
    };
    if spec.contains(',') {
        return ByteRange::Ignore;
    }
    let Some((lo, hi)) = spec.split_once('-') else {
        return ByteRange::Ignore;
    };
    let (lo, hi) = (lo.trim(), hi.trim());
    let num = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u64>().ok())
            .flatten()
    };
    if lo.is_empty() {
        // A suffix range: the last `n` bytes.
        let Some(n) = num(hi) else {
            return ByteRange::Ignore;
        };
        if n == 0 || len == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Single(len.saturating_sub(n), len - 1);
    }
    let Some(from) = num(lo) else {
        return ByteRange::Ignore;
    };
    let to = if hi.is_empty() {
        None
    } else {
        match num(hi) {
            Some(t) if t >= from => Some(t),
            _ => return ByteRange::Ignore,
        }
    };
    if from >= len {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Single(from, to.map_or(len - 1, |t| t.min(len - 1)))
}

/// `items` (listing entries from [`members`], each with the stored metadata it was made from)
/// with the size of each data resource's content: the content of the version the entry
/// describes, so the size never comes from a later write than the fields and entity tag beside
/// it. When that version is gone (a write replaced it since), the size is left out (it is a
/// SHOULD), rather than taken from another version.
async fn with_sizes<S: Store + 'static>(
    state: &LwsState<S>,
    items: &mut [(Value, crate::store::sparq::ResourceMeta)],
) {
    for (item, meta) in items {
        if item["type"] != "DataResource" {
            continue;
        }
        let Some(id) = item["id"].as_str().map(str::to_string) else {
            continue;
        };
        if let Ok(body) = state.store.read_at(&id, meta).await {
            item["size"] = json!(body.len());
        }
    }
}

/// A container's members, sorted, with what a listing shows about each but a data resource's
/// size (which [`with_sizes`] adds for the members shown). Each member's own entity tag stands
/// for its content.
async fn members<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<Vec<(Value, crate::store::sparq::ResourceMeta)>, ServerError> {
    let mut children: Vec<String> = state
        .store
        .list_children(uri)
        .await?
        .into_iter()
        .map(|c| c.as_str().to_string())
        .collect();
    children.sort();
    children.dedup();
    let mut out = Vec::with_capacity(children.len());
    for child in children {
        let Some(meta) = state.store.meta(&child).await? else {
            continue;
        };
        let mut item = Map::new();
        item.insert("id".into(), Value::String(child.clone()));
        if child.ends_with('/') {
            item.insert("type".into(), Value::String("Container".into()));
            let cmeta = state.resource_meta(&child).await?;
            let modified = cmeta
                .modified_ms
                .or(meta.last_modified.map(epoch_ms))
                .unwrap_or_default();
            item.insert(
                "modified".into(),
                Value::String(format_rfc3339(to_secs(modified) as i64)),
            );
            item.insert("etag".into(), Value::String(quoted(&meta.etag)));
        } else {
            item.insert("type".into(), Value::String("DataResource".into()));
            // "format: The media type of the resource ... MUST be present for DataResources"; size
            // and modified SHOULD be. The size is measured only for the members a page shows
            // (see [`with_sizes`]): every member's content is not read to serve one page.
            item.insert("format".into(), Value::String(meta.content_type.clone()));
            let modified = meta.last_modified.map(epoch_ms).unwrap_or_default();
            item.insert(
                "modified".into(),
                Value::String(format_rfc3339(to_secs(modified) as i64)),
            );
            item.insert("etag".into(), Value::String(quoted(&meta.etag)));
        }
        out.push((Value::Object(item), meta));
    }
    Ok(out)
}

async fn read_container<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    uri: &str,
    meta: &crate::store::sparq::ResourceMeta,
) -> Response {
    let Some(media_type) = negotiate_container(req.header(header::ACCEPT)) else {
        return problem(StatusCode::NOT_ACCEPTABLE, None);
    };
    let all = match members(state, uri).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let cmeta = match state.resource_meta(uri).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    // The entity tag covers the whole listing: every field of every member it represents (a
    // member container's `modified` included, which moves when something is created in it, while
    // that container's own stored tag does not), and each member's own tag.
    let mut hasher = Sha256::new();
    hasher.update(meta.etag.as_bytes());
    hasher.update(cmeta.version.as_deref().unwrap_or_default().as_bytes());
    for (m, _) in &all {
        hasher.update(serde_json::to_vec(m).unwrap_or_default());
        hasher.update(b"\n");
    }
    let tag = jose::b64url(&hasher.finalize()[..18]);
    // The listing changed no earlier than any member it lists did, so its Last-Modified is the
    // latest of the container's own and every listed member's: an If-Modified-Since never meets a
    // 304 for a listing whose members moved on since.
    let modified = all
        .iter()
        .filter_map(|(m, _)| m["modified"].as_str().and_then(parse_rfc3339))
        .filter_map(|t| u64::try_from(t).ok())
        .fold(
            to_secs(
                cmeta
                    .modified_ms
                    .or(meta.last_modified.map(epoch_ms))
                    .unwrap_or_default(),
            ),
            u64::max,
        );

    let page_size = state.cfg.page_size;
    let pages = all.len().div_ceil(page_size).max(1);
    let page = match req.query_param("page") {
        None => 1,
        Some(p) => p.parse::<usize>().unwrap_or(0),
    };
    if page < 1 || page > pages {
        return problem(StatusCode::NOT_FOUND, None);
    }
    // Each page is its own representation, so a later page has its own entity tag.
    let etag = if page == 1 {
        format!("\"{tag}\"")
    } else {
        format!("\"{tag}-p{page}\"")
    };
    match evaluate(&req.headers, Some(&etag), Some(modified), true) {
        Precondition::Failed => return problem(StatusCode::PRECONDITION_FAILED, None),
        Precondition::NotModified => {
            let mut r = StatusCode::NOT_MODIFIED.into_response();
            set(r.headers_mut(), header::ETAG, &etag);
            set(r.headers_mut(), header::LAST_MODIFIED, &http_date(modified));
            set(r.headers_mut(), header::VARY, "Accept");
            resource_links(state, r.headers_mut(), uri, &cmeta.types);
            return r;
        }
        Precondition::Proceed => {}
    }
    let mut shown: Vec<(Value, crate::store::sparq::ResourceMeta)> = all
        .iter()
        .skip((page - 1) * page_size)
        .take(page_size)
        .map(|(m, meta)| {
            let mut m = m.clone();
            if let Some(o) = m.as_object_mut() {
                o.remove("etag");
            }
            (m, meta.clone())
        })
        .collect();
    with_sizes(state, &mut shown).await;
    let items: Vec<Value> = shown.into_iter().map(|(m, _)| m).collect();
    let body = json!({
        "@context": LWS_CONTEXT,
        "id": uri,
        "type": "Container",
        "totalItems": all.len(),
        "items": items,
    });
    let mut resp = json_response(StatusCode::OK, &media_type, &body);
    let h = resp.headers_mut();
    set(h, header::ETAG, &etag);
    set(h, header::LAST_MODIFIED, &http_date(modified));
    // "Because the Content-Type of a container response depends on the request's Accept header,
    // these responses SHOULD include a Vary: Accept header."
    set(h, header::VARY, "Accept");
    resource_links(state, h, uri, &cmeta.types);
    if pages > 1 {
        // rel=first MUST be present on paginated responses, prev MUST be omitted on the first page
        // and next on the last; last is a MAY, given here.
        let page_uri = |n: usize| format!("{uri}?page={n}");
        add_link(h, &page_uri(1), "first", None);
        if page > 1 {
            add_link(h, &page_uri(page - 1), "prev", None);
        }
        if page < pages {
            add_link(h, &page_uri(page + 1), "next", None);
        }
        add_link(h, &page_uri(pages), "last", None);
    }
    resp
}

/// The storage description: a controlled identifier document extended with the LWS vocabulary,
/// naming the storage and its services.
pub fn storage_description<S: Store>(state: &LwsState<S>) -> Value {
    let cfg = &state.cfg;
    let storage = cfg.storage();
    let service = |frag: &str, ty: &str, endpoint: String| json!({"id": format!("{storage}#{frag}"), "type": ty, "serviceEndpoint": endpoint});
    json!({
        "@context": [CID_CONTEXT, LWS_CONTEXT],
        "id": storage,
        "type": "Storage",
        "service": [
            service("storage-root", "StorageRoot", storage.clone()),
            service("authorization-server", "AuthorizationServer", cfg.issuer().to_string()),
        ],
    })
}

/// The storage description's entity tag: a digest of the document, so it changes whenever the
/// description does (a new service, a rotated key).
fn description_etag(description: &Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(description).unwrap_or_default());
    format!("\"sd-{}\"", jose::b64url(&digest[..12]))
}

// ---- create ----

/// A safe member name from a Slug: letters, digits, `-`, `_` and `.`, never starting with a dot and
/// never ending with the linkset suffix.
fn sanitize_slug(slug: Option<&str>) -> Option<String> {
    let raw = slug?.trim();
    let decoded: String = url::form_urlencoded::parse(format!("x={raw}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    let mut clean: String = decoded
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    clean = clean
        .trim_start_matches('.')
        .trim_end_matches('/')
        .to_string();
    // Truncated first, so the reserved suffix is checked on the name that is used: a longer Slug
    // can end in it only once cut (`aaa….metax`).
    clean.truncate(120);
    if clean.to_ascii_lowercase().ends_with(META_SUFFIX) {
        clean.truncate(clean.len() - META_SUFFIX.len());
        clean.push_str("-meta");
    }
    (!clean.is_empty() && clean.chars().any(|c| c.is_ascii_alphanumeric())).then_some(clean)
}

/// User-managed links by relation.
type Links = std::collections::BTreeMap<String, Vec<String>>;

/// A relation's key: an extension relation (a URI) as it is, a registered one lower-cased.
fn rel_key(r: &str) -> String {
    if r.contains(':') {
        r.to_string()
    } else {
        r.to_ascii_lowercase()
    }
}

/// The types and user-managed links a request declared in its Link headers. What they hold once
/// resolved against `uri` is held to [`MAX_DECLARED_LINK_BYTES`](super::MAX_DECLARED_LINK_BYTES)
/// like the header itself (see [`super::check_headers`]): each target is resolved, and charged
/// for every relation it would be kept under, before any of it is kept; past it, `431`.
fn link_declared(req: &LwsRequest, uri: &str) -> Result<(Vec<String>, Links), Response> {
    let mut types = Vec::new();
    let mut links = Links::new();
    // Repeats are found with a set, as in [`content_types`].
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0usize;
    for (target, params) in parse_links(&req.header_all(header::LINK)) {
        let Some(rel) = params.get("rel") else {
            continue;
        };
        let resolved = resolve_against(uri, &target);
        let kept = resolved.len().max(target.len());
        bytes = bytes.saturating_add(rel.split_whitespace().count().saturating_mul(kept));
        if bytes > super::MAX_DECLARED_LINK_BYTES {
            return Err(problem(
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                Some("the Link header declares more than this server accepts"),
            ));
        }
        for r in rel.split_whitespace() {
            let key = rel_key(r);
            if key == "type" {
                if !target.starts_with(LWS_NS)
                    && is_uri(&target)
                    && seen.insert((key, target.clone()))
                {
                    types.push(target.clone());
                }
            } else if !STRUCTURAL_RELATIONS.contains(&key.as_str())
                && seen.insert((key.clone(), resolved.clone()))
            {
                links.entry(key).or_default().push(resolved.clone());
            }
        }
    }
    Ok((types, links))
}

/// `target` resolved against `base`, or as it is when it cannot be.
fn resolve_against(base: &str, target: &str) -> String {
    url::Url::parse(base)
        .ok()
        .and_then(|b| b.join(target).ok())
        .map(|u| u.to_string())
        .unwrap_or_else(|| target.to_string())
}

/// The types a representation states for the resource itself: `<> a <T>` in Turtle.
fn content_types(uri: &str, content_type: &str, body: &[u8]) -> Result<Vec<String>, Response> {
    let mut types = Vec::new();
    if !content_type.starts_with("text/turtle") {
        return Ok(types);
    }
    // Repeats are found with a set: a representation may state tens of thousands of types.
    let mut seen = std::collections::HashSet::new();
    let mut budget = super::expansion_budget(body.len());
    if let Ok(parser) = oxttl::TurtleParser::new().with_base_iri(uri) {
        for t in parser.for_slice(body).flatten() {
            budget = match budget.checked_sub(super::triple_bytes(&t)) {
                Some(left) => left,
                None => {
                    return Err(problem(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Some("the representation's terms expand past what the server reads"),
                    ))
                }
            };
            if let (oxrdf::NamedOrBlankNode::NamedNode(s), oxrdf::Term::NamedNode(o)) =
                (&t.subject, &t.object)
            {
                if s.as_str() == uri
                    && t.predicate.as_str() == RDF_TYPE
                    && seen.insert(o.as_str().to_string())
                {
                    types.push(o.as_str().to_string());
                }
            }
        }
    }
    Ok(types)
}

/// The types a resource has: those declared by Link headers, then those its content states.
fn all_types(declared: &[String], stated: Vec<String>) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = declared.iter().cloned().collect();
    let mut types = declared.to_vec();
    types.extend(stated.into_iter().filter(|t| seen.insert(t.clone())));
    types
}

/// Whether the request asks, with `Prefer: set-linkset`, for its Link headers to update the
/// linkset along with the content (update-resource: "MUST be invoked explicitly via the Prefer
/// header to prevent unintentional metadata overwrites").
fn prefers_set_linkset(req: &LwsRequest) -> bool {
    req.header_all(header::HeaderName::from_static("prefer"))
        .split([',', ';'])
        .any(|p| p.trim().eq_ignore_ascii_case("set-linkset"))
}

/// The linkset a new resource starts with: its declared user-managed links.
fn initial_linkset(uri: &str, links: &Links) -> Option<Value> {
    if links.is_empty() {
        return None;
    }
    let mut entry = Map::new();
    entry.insert("anchor".into(), Value::String(uri.into()));
    for (rel, targets) in links {
        entry.insert(
            rel.clone(),
            Value::Array(targets.iter().map(|t| json!({"href": t})).collect()),
        );
    }
    Some(json!({"linkset": [Value::Object(entry)]}))
}

async fn create<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    parent: &str,
) -> Response {
    match state.store.exists(parent).await {
        Ok(true) => {}
        Ok(false) => return problem(StatusCode::NOT_FOUND, None),
        Err(e) => return store_error(e),
    }
    if !parent.ends_with('/') {
        return method_not_allowed(&allow_for(parent, false));
    }
    let is_container = parse_links(&req.header_all(header::LINK))
        .iter()
        .any(|(t, p)| {
            t == &format!("{LWS_NS}Container")
                && p.get("rel")
                    .is_some_and(|r| r.split_whitespace().any(|r| r.eq_ignore_ascii_case("type")))
        });
    let base_name = sanitize_slug(req.header("slug"))
        .unwrap_or_else(|| jose::random_id().to_ascii_lowercase().replace('_', "-"));
    // The container's lock is held shared from the choice of a free name through the create and
    // the new member's creator metadata. Shared, because creates in one container do not need to
    // exclude each other: what keeps two POSTs with one Slug from both taking a name is the new
    // member's own lock (both of its spellings, `name` and `name/`), which [`free_name`] takes
    // before it checks the name is free, and which is held until the member's content and metadata
    // are written. A name whose lock is busy (a create, write or delete of that IRI in flight) is
    // taken. What does need the container to stand still (a delete of it, a change to its own
    // metadata, the touch below) takes its lock exclusively and waits for the creates in flight.
    //
    // The member's lock also orders the create after a DELETE in flight at the IRI, which holds
    // it across the removal of the content and then the metadata: a create that slipped in between
    // would have its fresh metadata removed. Locks are taken member before container
    // ([`lock_order`]), so under the container's lock the member's are only tried.
    //
    // Both are released before the container's own metadata is touched (which takes its lock
    // exclusively).
    //
    // A conditional create is evaluated against the container's listing, and takes the
    // container's lock exclusively so no other create or delete changes the listing between the
    // evaluation and the create.
    let conditional = is_conditional(req);
    let parent_guard = if conditional {
        (None, Some(state.locks.lock(parent).await))
    } else {
        (Some(state.locks.read(parent).await), None)
    };
    if conditional {
        let meta = match current(state, parent).await {
            Ok(m) => m,
            Err(r) => return r,
        };
        let listing = read_container(state, &plain_get(req), parent, &meta).await;
        if !listing.status().is_success() {
            return listing;
        }
        let (etag, modified) = validators_of(&listing);
        if let Some(refused) = unless_preconditions(req, etag.as_deref(), modified) {
            return refused;
        }
    }
    let (name, child_guards) = match free_name(state, parent, &base_name).await {
        Ok(found) => found,
        Err(r) => return r,
    };
    let child = if is_container {
        format!("{parent}{name}/")
    } else {
        format!("{parent}{name}")
    };
    // Under the container's lock, against the container the member goes into.
    if let Err(r) = recheck(state, Action::Create, parent, agent).await {
        return r;
    }
    let content_type = if is_container {
        LWS_JSON.to_string()
    } else {
        req.header(header::CONTENT_TYPE)
            .map(str::to_string)
            .unwrap_or_else(|| "application/octet-stream".into())
    };
    let body = if is_container {
        Bytes::new()
    } else {
        req.body.clone()
    };
    // A container declares its types and links as a data resource does; only types its content
    // states are a data resource's alone (a container has no content of its own).
    let (declared_types, links) = match link_declared(req, &child) {
        Ok(declared) => declared,
        Err(r) => return r,
    };
    let stated = if is_container {
        Vec::new()
    } else {
        match content_types(&child, &content_type, &body) {
            Ok(t) => t,
            Err(r) => return r,
        }
    };
    let types = all_types(&declared_types, stated);
    let meta = ResourceMeta {
        creator: agent.subject.clone(),
        linkset: initial_linkset(&child, &links),
        types,
        declared_types: Some(declared_types),
        links,
        modified_ms: is_container.then(now_ms),
        ..Default::default()
    };
    // The new member's metadata is written before its content, replacing whatever an earlier
    // resource at the IRI left behind (a delete whose metadata removal failed): content never
    // exists under metadata that is not its own. When the content cannot be created the metadata
    // is removed again; one left behind describes nothing, and the next create replaces it.
    //
    // The writes run in a task of their own that holds the locks, so a client that goes away
    // cannot release them while a write is still pending (with a remote store, a create sent
    // before the request was dropped may still commit): until the writes are over the name stays
    // taken, and no other create can put its metadata under this content. The container's lock is
    // shared, so the task's scheduling holds up no other create in the container.
    let created = {
        let (state, parent, child, meta, admission) = (
            state.clone(),
            parent.to_string(),
            child.clone(),
            meta.clone(),
            req.admission.clone(),
        );
        tokio::spawn(async move {
            // Held to the end, announcement included: this work still counts against the
            // concurrency ceiling once the request has gone.
            let _admission = admission;
            let locks = (child_guards, parent_guard);
            state.put_resource_meta(&child, &meta).await?;
            let created = match state
                .store
                .create_in_container(&parent, &child, body, &content_type)
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    // A refusal created nothing, and the metadata goes. A backend failure may
                    // follow a create that committed (a remote store's lost reply): the metadata
                    // goes only once the content is known to be gone too, so committed content is
                    // never left without its creator.
                    let gone = !matches!(e, ServerError::Storage(_))
                        || super::delete_record(&state, &child, &parent).await.is_ok();
                    if gone {
                        let _ = state.store.delete(&meta_key(&child), None).await;
                    } else {
                        // The member may exist: the container's listing may have changed.
                        drop(locks);
                        touch_container(&state, &parent).await;
                    }
                    return Err(e);
                }
            };
            // What follows a commit follows it whether or not the client is still there: the
            // container's bookkeeping. The locks go first (the touch takes the container's lock
            // again).
            drop(locks);
            touch_container(&state, &parent).await;
            Ok(created)
        })
        .await
    };
    let created = match created {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return store_error(e),
        Err(e) => return store_error(ServerError::Storage(format!("the create failed: {e}"))),
    };
    let mut resp = problem(StatusCode::CREATED, None);
    let h = resp.headers_mut();
    set(h, header::LOCATION, &child);
    set(h, header::ETAG, &quoted(&created.etag));
    add_link(h, parent, "up", None);
    add_link(h, &lws_type(&child), "type", None);
    for t in &meta.types {
        add_link(h, t, "type", None);
    }
    add_link(h, &meta_key(&child), "linkset", Some(LINKSET_JSON));
    resp
}

/// A member name in `parent` that is free, with the locks of both its spellings (`name/`, then
/// `name`, in [`lock_order`]): `base_name`, else `base_name-2`, `-3`, ... and past
/// [`NUMBERED_NAMES`] a random suffix, [`RANDOM_NAMES`] times, then a 409. A name is free when
/// its locks can be taken now and, under them, neither spelling exists; a reserved name
/// ([`RESERVED_ROOT_NAMES`]) never is. Under the locks the answer holds: only a create makes a
/// member, and a create of this IRI needs them.
async fn free_name<S: Store + 'static>(
    state: &LwsState<S>,
    parent: &str,
    base_name: &str,
) -> Result<(String, (IriGuard, IriGuard)), Response> {
    match state.store.exists(parent).await {
        Ok(true) => {}
        Ok(false) => return Err(problem(StatusCode::NOT_FOUND, None)),
        Err(e) => return Err(store_error(e)),
    }
    // The probes are bounded: the container's lock is held throughout, so a search that never
    // ends would hold up whatever waits to take it exclusively. A backend failure is an error,
    // not a name that is taken.
    let at_root = parent == state.cfg.storage();
    let mut name = base_name.to_string();
    for n in 2..=(NUMBERED_NAMES + RANDOM_NAMES + 1) {
        if !(at_root && RESERVED_ROOT_NAMES.contains(&name.as_str())) {
            let a = format!("{parent}{name}");
            let b = format!("{parent}{name}/");
            if let (Some(gb), Some(ga)) = (state.locks.try_lock(&b), state.locks.try_lock(&a)) {
                let taken = state.store.exists(&a).await.map_err(store_error)?
                    || state.store.exists(&b).await.map_err(store_error)?;
                if !taken {
                    return Ok((name, (gb, ga)));
                }
            }
        }
        name = if n > NUMBERED_NAMES {
            format!("{base_name}-{}", jose::random_id())
        } else {
            format!("{base_name}-{n}")
        };
    }
    Err(problem(
        StatusCode::CONFLICT,
        Some("no free name was found for the new member"),
    ))
}

/// How many numbered names (`name-2` …) a create tries after the one asked for.
const NUMBERED_NAMES: usize = 50;
/// How many random suffixes it then tries before it gives up.
const RANDOM_NAMES: usize = 8;

/// Names a member of the root container cannot have: the server answers those paths itself (the
/// liveness and readiness probes, mounted ahead of the resources), so a resource there could
/// never be read. Everything else the server mounts at the root is under `/.well-known/` or
/// `/.lws/`, which no member name can begin with (a name never starts with a dot).
const RESERVED_ROOT_NAMES: [&str; 2] = ["livez", "readyz"];

// ---- update ----

async fn current<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<crate::store::sparq::ResourceMeta, Response> {
    match state.store.meta(uri).await {
        Ok(Some(m)) => Ok(m),
        Ok(None) => Err(problem(StatusCode::NOT_FOUND, None)),
        Err(e) => Err(store_error(e)),
    }
}

/// Per-resource write locks. The store offers no compare-and-swap, so a conditional write (PUT,
/// DELETE) holds its resource's lock from the precondition check through
/// the write, and an If-Match can never pass against a state another writer is replacing. Metadata
/// read-modify-writes of a container (membership touches) take the container's lock too.
///
/// A create takes its container's lock (shared: creates in one container exclude each other by
/// the new member's lock alone) across the choice of a free name and the create, and the new
/// member's lock across its content and metadata (as a delete holds it across their removal),
/// and a recursive delete takes the lock of every resource it removes (see [`lock_subtree`]), so neither
/// can interleave with the other or with a conditional write.
///
/// Limit: the locks live in this process. Several server processes over one store do not see each
/// other's locks; that deployment needs a conditional write in the store itself.
#[derive(Default)]
pub struct IriLocks(std::sync::Mutex<LockMap>);

/// The locks by IRI, held weakly, and how many entries the map may reach before the dropped ones
/// are swept out.
#[derive(Default)]
struct LockMap {
    locks: std::collections::HashMap<String, std::sync::Weak<tokio::sync::RwLock<()>>>,
    sweep_at: usize,
}

/// The size of [`LockMap`] below which it is never swept.
const LOCK_SWEEP_FLOOR: usize = 1024;

impl IriLocks {
    /// Wait for and take the lock of `iri`. A writer that holds several locks takes them longest
    /// IRI first, ties in byte order ([`lock_order`]), so a child always before its container and
    /// never the other way, and two writers cannot deadlock.
    pub async fn lock(&self, iri: &str) -> IriGuard {
        self.mutex(iri).write_owned().await
    }

    /// Wait for and take the shared lock of `iri`: a read holds it from its permission check
    /// through the representation it serves, so no write lands in between. Readers share it; a
    /// reader holds no other lock, so it cannot take part in a deadlock.
    pub async fn read(&self, iri: &str) -> tokio::sync::OwnedRwLockReadGuard<()> {
        self.mutex(iri).read_owned().await
    }

    /// Take the lock of `iri` if it is free, without waiting: the one way to take a lock out of
    /// [`lock_order`], since it cannot deadlock.
    pub fn try_lock(&self, iri: &str) -> Option<IriGuard> {
        self.mutex(iri).try_write_owned().ok()
    }

    fn mutex(&self, iri: &str) -> std::sync::Arc<tokio::sync::RwLock<()>> {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let LockMap {
            locks: map,
            sweep_at,
        } = &mut *guard;
        // Swept once the map has doubled since the last sweep, so a sweep that finds most locks
        // still held (a recursive delete holding thousands) is not repeated at every call: the
        // sweeps cost constant time per lock taken.
        if map.len() > (*sweep_at).max(LOCK_SWEEP_FLOOR) {
            map.retain(|_, w| w.strong_count() > 0);
            *sweep_at = map.len() * 2;
        }
        match map.get(iri).and_then(std::sync::Weak::upgrade) {
            Some(m) => m,
            None => {
                let m = std::sync::Arc::new(tokio::sync::RwLock::new(()));
                map.insert(iri.to_string(), std::sync::Arc::downgrade(&m));
                m
            }
        }
    }
}

/// The order in which a writer holding several IRI locks takes them: longest first, so a member
/// before the container that holds it (a member's IRI extends its container's), then byte order.
fn lock_order(a: &str, b: &str) -> std::cmp::Ordering {
    b.len().cmp(&a.len()).then_with(|| a.cmp(b))
}

/// The metadata stored beside `uri`: `None` when there is none, an error when the store fails or
/// what is stored does not parse.
async fn stored_meta<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<Option<ResourceMeta>, ServerError> {
    match state.store.read(&meta_key(uri)).await {
        Ok(r) => super::parse_meta(uri, &r.body).map(Some),
        Err(ServerError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Run a mutation's store writes (`writes`) while `locks` are held; the locks come back with the
/// outcome. The mutations run in a task of their own ([`handle`] spawns them, before any lock is
/// taken), so a client that goes away cannot release the locks while a write is still pending (a
/// remote store may commit a request that was already sent): nobody else acts on the resource
/// until the writes are over, and none of them is cut off half way. The task is spawned before
/// the locks are taken, not here, so its scheduling never sits inside the critical section that
/// every writer of the resource waits on.
async fn hold_locks<L, T, F>(locks: L, writes: F) -> Result<(T, L), ServerError>
where
    F: std::future::Future<Output = T>,
{
    let out = writes.await;
    Ok((out, locks))
}

/// Write new content for `uri` and, with `meta` = `(new, old)`, its new metadata, such that a
/// failure (or the request being dropped) never leaves content visible under metadata that does
/// not describe it: the types (and the creator) in the metadata are what grants rest on, so new
/// content under old types, or old content under new ones, could reach readers neither state
/// admits.
///
/// - Metadata unchanged: one content write, which lands whole or not at all.
/// - Metadata changed: three writes. The old metadata marked `pending` first (when that fails,
///   nothing changed); then the content; then the new metadata, which clears the mark. When the
///   content write is refused the old metadata is put back; a backend failure may follow a
///   write that committed, so it leaves the mark. Whenever a step after the first fails,
///   rollback included, the `pending` mark stays and the resource fails closed: only its owner
///   and creator may act on it (see [`access::allowed`](super::access::allowed)) until a write
///   completes.
///
/// The writes, a lone content write included, run with the resource's lock (`guard`, handed back
/// when they are done) held, in the request's own task (see [`hold_locks`]), so a client that goes
/// away mid-way cancels neither the writes nor the rollback, and nobody sees the steps in between.
async fn write_with_meta<S: Store + 'static>(
    state: &LwsState<S>,
    guard: IriGuard,
    uri: &str,
    body: Bytes,
    content_type: &str,
    meta: Option<(ResourceMeta, Option<ResourceMeta>)>,
) -> (
    Result<crate::store::sparq::ResourceMeta, ServerError>,
    Option<IriGuard>,
) {
    let changed = meta
        .map(|(mut new, old)| {
            new.pending = false;
            (new, old)
        })
        .filter(|(new, old)| old.clone().unwrap_or_default() != *new);
    // Metadata that could not be stored is refused before anything is written.
    if let Some(Err(e)) = changed.as_ref().map(|(new, _)| encode_meta(new)) {
        return (Err(e), Some(guard));
    }
    let (state, uri, content_type) = (state.clone(), uri.to_string(), content_type.to_string());
    let Some((new, old)) = changed else {
        let write = async move { state.store.write(&uri, body, &content_type).await };
        return match hold_locks(guard, write).await {
            Ok((written, guard)) => (written, Some(guard)),
            Err(e) => (Err(e), None),
        };
    };
    let writes = async move {
        let mut closed = old.clone().unwrap_or_default();
        closed.pending = true;
        state.put_resource_meta(&uri, &closed).await?;
        let written = match state.store.write(&uri, body, &content_type).await {
            Ok(m) => m,
            Err(e) => {
                // The old metadata goes back only when the store refused the write outright. A
                // backend failure (a remote store's timeout, a lost reply) may come after the
                // write committed, and the new content must not be served under the old
                // metadata: the mark stays, as it does when the rollback fails. Fail closed.
                if !matches!(e, ServerError::Storage(_)) {
                    let _ = match &old {
                        Some(old) => state.put_resource_meta(&uri, old).await,
                        None => state.store.delete(&meta_key(&uri), None).await,
                    };
                }
                return Err(e);
            }
        };
        state.put_resource_meta(&uri, &new).await?;
        Ok(written)
    };
    match hold_locks(guard, writes).await {
        Ok((outcome, guard)) => (outcome, Some(guard)),
        Err(e) => (Err(e), None),
    }
}

async fn update<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let guard = state.locks.lock(uri).await;
    let listing = listing_guard(state, uri).await;
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if let Err(r) = recheck(state, Action::Modify, uri, agent).await {
        return r;
    }
    if uri.ends_with('/') {
        return method_not_allowed(&allow_for(uri, uri == state.cfg.storage()));
    }
    if let Precondition::Failed | Precondition::NotModified = evaluate(
        &req.headers,
        Some(&quoted(&meta.etag)),
        meta.last_modified.map(|t| to_secs(epoch_ms(t))),
        false,
    ) {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    let content_type = req
        .header(header::CONTENT_TYPE)
        .map(str::to_string)
        .unwrap_or(meta.content_type.clone());
    let set_linkset = prefers_set_linkset(req);
    let old_rmeta = match stored_meta(state, uri).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let mut rmeta = old_rmeta.clone().unwrap_or_default();
    // "LWS servers MUST handle PUT and PATCH requests on resource URIs as modifications to the
    // resource content only, with no default impact on the associated linkset resource." The
    // user-managed links Link headers carry change only with Prefer: set-linkset, which replaces
    // them. Types are not user-managed links but derived ("types-from-content",
    // "types-from-link-headers", on every write): the types the content states follow the
    // content (and only they do), and the types a PUT's own Link headers declare replace the
    // declared ones; a PUT that declares none keeps them.
    let (link_types, links) = match link_declared(req, uri) {
        Ok(declared) => declared,
        Err(r) => return r,
    };
    let declared = if set_linkset {
        rmeta.linkset = initial_linkset(uri, &links);
        rmeta.links = links;
        rmeta.linkset_etag = None;
        link_types
    } else if !link_types.is_empty() {
        link_types
    } else if let Some(declared) = rmeta.declared_types.clone() {
        declared
    } else {
        // Metadata from before declared types were kept apart: they are the types less those
        // the old content states. Unreadable old content is a failure, not "states no types":
        // otherwise the types it stated would outlive it.
        let old_content_types = if meta.content_type.starts_with("text/turtle") {
            match state.store.read_at(uri, &meta).await {
                Ok(b) => match content_types(uri, &meta.content_type, &b) {
                    Ok(t) => t,
                    Err(r) => return r,
                },
                Err(e) => return store_error(e),
            }
        } else {
            Vec::new()
        };
        let old_content_types: std::collections::HashSet<String> =
            old_content_types.into_iter().collect();
        rmeta
            .types
            .iter()
            .filter(|t| !old_content_types.contains(*t))
            .cloned()
            .collect()
    };
    let stated = match content_types(uri, &content_type, &req.body) {
        Ok(t) => t,
        Err(r) => return r,
    };
    rmeta.types = all_types(&declared, stated);
    rmeta.declared_types = Some(declared);
    let (written, _guard) = write_with_meta(
        state,
        guard,
        uri,
        req.body.clone(),
        &content_type,
        Some((rmeta, old_rmeta)),
    )
    .await;
    let written = match written {
        Ok(m) => m,
        Err(e) => {
            drop(listing);
            return unsettled(state, uri, e).await;
        }
    };
    drop(listing);
    changed(state, uri).await;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set(resp.headers_mut(), header::ETAG, &quoted(&written.etag));
    if set_linkset {
        set(
            resp.headers_mut(),
            header::HeaderName::from_static("preference-applied"),
            "set-linkset",
        );
    }
    resp
}

/// The response to a write that failed with `e`. A backend failure may follow a write that
/// committed (a remote store's lost reply), so the container is touched as for a change: a
/// conditional request against its listing must not pass on validators from before.
async fn unsettled<S: Store + 'static>(state: &LwsState<S>, uri: &str, e: ServerError) -> Response {
    if matches!(e, ServerError::Storage(_)) {
        if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
            touch_container(state, &parent).await;
        }
    }
    store_error(e)
}

/// After a resource or its metadata changed: its container changes too.
async fn changed<S: Store + 'static>(state: &LwsState<S>, uri: &str) {
    if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
        // A member's listed fields (format, size, modified) changed, so the listing did.
        touch_container(state, &parent).await;
    }
}

// ---- delete ----

async fn delete<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let storage = state.cfg.storage();
    if let Err(r) = current(state, uri).await {
        return r;
    }
    if uri == storage {
        return method_not_allowed(&allow_for(uri, true));
    }
    let parent = parent_of(uri, &storage);
    // Every resource the delete may remove is locked before anything is decided, so the
    // precondition, the permission check of each descendant and the removal see one state: no
    // member can be created in, written to, or deleted from the subtree in between.
    let (guards, doomed) = match lock_subtree(state, uri, parent.clone()).await {
        Ok(locked) => locked,
        Err(e) => return store_error(e),
    };
    let mut listing = listing_guard(state, uri).await;
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    // The validators the preconditions are evaluated against (RFC 9110 section 13): a data
    // resource's own; a container's are its listing's, computed the way a read computes them,
    // under the subtree locks, whenever the request is conditional at all.
    let (etag, modified) = if !uri.ends_with('/') {
        (
            Some(quoted(&meta.etag)),
            meta.last_modified.map(|t| to_secs(epoch_ms(t))),
        )
    } else if is_conditional(req) {
        let listing = read_container(state, &plain_get(req), uri, &meta).await;
        if !listing.status().is_success() {
            return listing;
        }
        validators_of(&listing)
    } else {
        (None, None)
    };
    if let Some(refused) = unless_preconditions(req, etag.as_deref(), modified) {
        return refused;
    }
    if uri.ends_with('/') && doomed.len() > 1 {
        let infinity = req
            .header("depth")
            .is_some_and(|d| d.trim().eq_ignore_ascii_case("infinity"));
        if !infinity {
            return problem(StatusCode::CONFLICT, Some("the container is not empty; send Depth: infinity to delete it and everything in it"));
        }
    }
    // Delete on the container is not Delete on what is in it: a recursive delete removes nothing
    // unless the agent may delete every descendant. `uri` itself is checked again too, now that
    // its lock is held.
    for (node, _) in &doomed {
        if let Err(r) = recheck(state, Action::Delete, node, agent).await {
            return r;
        }
    }
    // The removals run in a task that holds the subtree's locks until they are over (see
    // [`hold_locks`]).
    let removal = {
        let state = state.clone();
        async move {
            let (removed, outcome) = remove(&state, &doomed).await;
            (removed, outcome, state)
        }
    };
    let (removed, outcome) = match hold_locks(guards, removal).await {
        Ok(((removed, outcome, _), guards)) => {
            drop(guards);
            (removed, outcome)
        }
        Err(e) => return store_error(e),
    };
    listing.take();
    // A removal whose outcome is unknown may have happened: the container is touched for it too.
    if removed > 0 || matches!(outcome, Err(ServerError::Storage(_))) {
        if let Some(p) = parent {
            touch_container(state, &p).await;
        }
    }
    match outcome {
        Ok(()) => problem(StatusCode::NO_CONTENT, None),
        Err(e) => store_error(e),
    }
}

/// `uri` and everything under it, each with the container it is in: descendants before the
/// containers that hold them, `uri` last.
async fn subtree<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
    parent: Option<String>,
) -> Result<Vec<(String, Option<String>)>, ServerError> {
    let mut out = Vec::new();
    let mut stack = vec![(uri.to_string(), parent, false)];
    while let Some((node, origin, expanded)) = stack.pop() {
        if expanded || !node.ends_with('/') {
            out.push((node, origin));
            continue;
        }
        let children = state.store.list_children(&node).await?;
        stack.push((node.clone(), origin, true));
        for child in children {
            let child = child.as_str().to_string();
            if child.starts_with(node.as_str()) && child != node {
                stack.push((child, Some(node.clone()), false));
            }
        }
    }
    Ok(out)
}

/// The exclusive lock of one IRI (see [`IriLocks`]).
pub type IriGuard = tokio::sync::OwnedRwLockWriteGuard<()>;

/// Lock `uri` and everything under it, in [`lock_order`], and return the guards with the subtree
/// (as [`subtree`] lists it) that holds while they are held. The subtree is listed, locked, and
/// listed again; when a member arrived or left in between, the locks are released and it starts
/// over. A create holds its container's lock (shared) until its member exists, so once every
/// container is locked (exclusively) no member can arrive.
async fn lock_subtree<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
    parent: Option<String>,
) -> Result<(Vec<IriGuard>, Vec<(String, Option<String>)>), ServerError> {
    let sorted = |tree: &[(String, Option<String>)]| {
        let mut iris: Vec<String> = tree.iter().map(|(n, _)| n.clone()).collect();
        iris.sort_by(|a, b| lock_order(a, b));
        iris.dedup();
        iris
    };
    for _ in 0..8 {
        let before = sorted(&subtree(state, uri, parent.clone()).await?);
        let mut guards = Vec::with_capacity(before.len());
        for iri in &before {
            guards.push(state.locks.lock(iri).await);
        }
        let after = subtree(state, uri, parent.clone()).await?;
        if sorted(&after) == before {
            return Ok((guards, after));
        }
    }
    Err(ServerError::Conflict(
        "the container kept changing while it was being deleted".into(),
    ))
}

/// Remove the resources of a locked [`subtree`], members before their containers, each before its
/// metadata (which says who may act on it, so it goes only once the resource has). Stops at the
/// first failure, the metadata's included; returns how many of `doomed`, from the front, were
/// removed, and the outcome.
async fn remove<S: Store + 'static>(
    state: &LwsState<S>,
    doomed: &[(String, Option<String>)],
) -> (usize, Result<(), ServerError>) {
    for (i, (node, parent)) in doomed.iter().enumerate() {
        // The record and its parent's membership edge go in one step, data resources included:
        // removed one after the other, a failure in between would leave a live resource its
        // container no longer lists, which a retried recursive delete would not find.
        let removed = match super::remove_member(&state.store, node, parent.as_deref()).await {
            Ok(crate::store::DeleteOutcome::NotEmpty) => Err(ServerError::Conflict(
                "the container gained a member while it was deleted".into(),
            )),
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        if let Err(e) = removed {
            return (i, Err(e));
        }
        // The resource is gone; metadata that survives it would describe the next resource at the
        // IRI, so a failure to remove it is a failure of the delete (and a create replaces such
        // metadata before it writes any content).
        match state.store.delete(&meta_key(node), None).await {
            Ok(_) | Err(ServerError::NotFound) => {}
            Err(e) => return (i + 1, Err(e)),
        }
    }
    (doomed.len(), Ok(()))
}

// ---- linksets ----

/// Relations the server maintains in a resource's own linkset entry. The metadata section lists
/// `linkset`, `type`, `format`, `size` and `modified` as system managed and read-only, and lets a
/// server restrict `up`; `self` carries the representation's format, size and modification time.
const SERVER_MANAGED: &[&str] = &["up", "type", "self", "linkset"];

/// Whether a linkset entry is about `uri` (its anchor, resolved against `uri`).
fn anchored_at(entry: &Value, uri: &str) -> bool {
    entry
        .get("anchor")
        .and_then(Value::as_str)
        .is_some_and(|a| a == uri || resolve_against(uri, a) == uri)
}

/// The user-managed part of a linkset document: every server-managed relation dropped from the
/// entries about `uri`. A client cannot write those relations; whatever a stored document has
/// there is ignored and the server's own values stand.
fn user_linkset(doc: &Value, uri: &str) -> Value {
    let entries = doc
        .get("linkset")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let kept: Vec<Value> = entries
        .into_iter()
        .filter_map(|mut e| {
            if anchored_at(&e, uri) {
                let o = e.as_object_mut()?;
                o.retain(|k, _| k == "anchor" || !SERVER_MANAGED.contains(&rel_key(k).as_str()));
                // An entry left with nothing but its anchor says nothing.
                if o.len() <= 1 {
                    return None;
                }
            }
            Some(e)
        })
        .collect();
    json!({"linkset": kept})
}

/// A resource's whole linkset document: the server-managed metadata (its container, its types, and
/// for a data resource its representation's format, size and modification time) merged into the
/// entry about it, with the user-managed links beside them.
async fn linkset_document<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
    meta: &ResourceMeta,
) -> Result<Value, ServerError> {
    let mut server = Map::new();
    server.insert("anchor".into(), Value::String(uri.into()));
    if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
        server.insert("up".into(), json!([{"href": parent}]));
    }
    let mut types = vec![json!({"href": lws_type(uri)})];
    types.extend(meta.types.iter().map(|t| json!({"href": t})));
    server.insert("type".into(), Value::Array(types));
    if !uri.ends_with('/') {
        if let Some(stored) = state.store.meta(uri).await? {
            let size = state.store.read_at(uri, &stored).await?.len();
            let modified = stored.last_modified.map(epoch_ms).unwrap_or_default();
            server.insert(
                "self".into(),
                json!([{
                    "href": uri,
                    "format": [stored.content_type],
                    "size": [size.to_string()],
                    "modified": [format_rfc3339(to_secs(modified) as i64)],
                }]),
            );
        }
    }
    let user = meta
        .linkset
        .clone()
        .or_else(|| initial_linkset(uri, &meta.links))
        .map(|d| user_linkset(&d, uri))
        .unwrap_or_else(|| json!({"linkset": []}));
    let mut entries = user["linkset"].as_array().cloned().unwrap_or_default();
    match entries.iter().position(|e| anchored_at(e, uri)) {
        Some(i) => {
            if let Some(o) = entries[i].as_object_mut() {
                for (k, v) in server {
                    o.insert(k, v);
                }
            }
        }
        None => entries.insert(0, Value::Object(server)),
    }
    Ok(json!({"linkset": entries}))
}

/// A linkset document's entity tag: a digest of the document, server-managed metadata included, so
/// it changes whenever any of it does.
fn linkset_etag(doc: &Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(doc).unwrap_or_default());
    format!("\"ls-{}\"", jose::b64url(&digest[..12]))
}

/// A resource's linkset (RFC 9264): GET and HEAD. Writes are not offered, so they are 405 with the
/// methods that are.
async fn linkset<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    if !matches!(req.method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return method_not_allowed(LINKSET_ALLOW);
    }
    let action = Action::Read;
    // A request is authorized once before it waits for the resource's lock, so one that may not
    // read the resource never queues behind a write (holding its admission slot while it waits).
    if let Some(refused) = authorize_unlocked(state, action, uri, agent).await {
        return refused;
    }
    // The resource's lock is taken before the permission check that counts, so the decision holds
    // for what is served.
    let _shared = state.locks.read(uri).await;
    let exists = match state.store.exists(uri).await {
        Ok(e) => e,
        Err(e) => return store_error(e),
    };
    if !exists {
        return if state.needs_auth(agent) {
            state.challenge(None)
        } else {
            problem(StatusCode::NOT_FOUND, None)
        };
    }
    if let Err(r) = recheck(state, action, uri, agent).await {
        return r;
    }
    let meta = match state.resource_meta(uri).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let document = match linkset_document(state, uri, &meta).await {
        Ok(d) => d,
        Err(e) => return store_error(e),
    };
    let etag = linkset_etag(&document);
    let mut resp = match req.method {
        Method::GET | Method::HEAD => match evaluate(&req.headers, Some(&etag), None, true) {
            Precondition::NotModified => {
                let mut r = StatusCode::NOT_MODIFIED.into_response();
                set(r.headers_mut(), header::ETAG, &etag);
                r
            }
            Precondition::Failed => {
                let mut r = problem(StatusCode::PRECONDITION_FAILED, None);
                set(r.headers_mut(), header::ETAG, &etag);
                r
            }
            Precondition::Proceed => {
                let mut r = json_response(StatusCode::OK, LINKSET_JSON, &document);
                set(r.headers_mut(), header::ETAG, &etag);
                r
            }
        },
        Method::OPTIONS => StatusCode::NO_CONTENT.into_response(),
        _ => method_not_allowed(LINKSET_ALLOW),
    };
    set(resp.headers_mut(), header::ALLOW, LINKSET_ALLOW);
    add_link(resp.headers_mut(), uri, "anchor", None);
    resp
}

/// Silence the unused import lint for constants other modules use through this one.
#[allow(dead_code)]
const _USED: &str = AS_CONTEXT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conneg() {
        assert_eq!(negotiate_container(None).as_deref(), Some(LWS_JSON));
        assert_eq!(negotiate_container(Some("*/*")).as_deref(), Some(LWS_JSON));
        assert_eq!(
            negotiate_container(Some("text/html;q=0.9, application/ld+json;q=0.5")).as_deref(),
            Some(LD_JSON)
        );
        assert_eq!(
            negotiate_container(Some("application/json")).as_deref(),
            Some(JSON)
        );
        assert_eq!(negotiate_container(Some("text/html")), None);
        assert_eq!(
            negotiate_container(Some(
                r#"application/ld+json; profile="https://www.w3.org/ns/lws/v1""#
            ))
            .as_deref(),
            Some(r#"application/ld+json; profile="https://www.w3.org/ns/lws/v1""#)
        );
        assert_eq!(
            negotiate_container(Some("application/json;q=0.2, application/lws+json")).as_deref(),
            Some(LWS_JSON)
        );
    }

    #[test]
    fn ranges() {
        use ByteRange::*;
        assert_eq!(parse_range("bytes=0-3", 10), Single(0, 3));
        assert_eq!(parse_range("bytes=-2", 10), Single(8, 9));
        assert_eq!(parse_range("bytes=5-", 10), Single(5, 9));
        assert_eq!(parse_range("bytes=5-50", 10), Single(5, 9));
        assert_eq!(parse_range("bytes=100-200", 10), Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 10), Unsatisfiable);
        // A multi-range request, another unit or broken syntax is ignored: the whole body is served.
        assert_eq!(parse_range("bytes=0-1,3-4", 10), Ignore);
        assert_eq!(parse_range("bytes=0-1, 100-200", 10), Ignore);
        assert_eq!(parse_range("items=0-1", 10), Ignore);
        assert_eq!(parse_range("bytes=4-2", 10), Ignore);
        assert_eq!(parse_range("bytes=a-b", 10), Ignore);
    }

    #[test]
    fn if_range() {
        let date = http_date(1_700_000_000);
        assert!(if_range_holds(None, "\"a\"", Some(1_700_000_000)));
        assert!(if_range_holds(Some("\"a\""), "\"a\"", None));
        assert!(!if_range_holds(Some("\"b\""), "\"a\"", None));
        assert!(!if_range_holds(Some("W/\"a\""), "\"a\"", None));
        assert!(if_range_holds(Some(&date), "\"a\"", Some(1_700_000_000)));
        assert!(!if_range_holds(Some(&date), "\"a\"", Some(1_700_000_001)));
        assert!(!if_range_holds(
            Some("garbage"),
            "\"a\"",
            Some(1_700_000_000)
        ));
    }

    #[test]
    fn slugs() {
        assert_eq!(
            sanitize_slug(Some("shoppinglist.txt")).as_deref(),
            Some("shoppinglist.txt")
        );
        assert_eq!(sanitize_slug(Some("../etc")).as_deref(), Some("-etc"));
        assert!(!sanitize_slug(Some(".meta")).is_some_and(|s| s.ends_with(".meta")));
        assert!(!sanitize_slug(Some("x.meta")).unwrap().ends_with(".meta"));
        assert_eq!(sanitize_slug(Some("...")), None);
        assert_eq!(sanitize_slug(None), None);
    }

    #[test]
    fn parents() {
        let s = "http://h/";
        assert_eq!(parent_of("http://h/a", s).as_deref(), Some("http://h/"));
        assert_eq!(
            parent_of("http://h/a/b/", s).as_deref(),
            Some("http://h/a/")
        );
        assert_eq!(parent_of("http://h/", s), None);
    }

    #[test]
    fn etag_lists() {
        assert!(etag_listed("\"a\", \"b\"", "\"b\"", false));
        assert!(!etag_listed("W/\"b\"", "\"b\"", false));
        assert!(etag_listed("W/\"b\"", "\"b\"", true));
        assert!(etag_listed("*", "\"x\"", false));
    }

    // ---- request-level tests over an in-memory store ----

    use crate::store::sparq::InMemorySparqClient;
    use crate::store::{CompositeStore, InMemoryBlobStore};
    use std::sync::Arc;

    type Mem = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;
    const BASE: &str = "http://lws.test";

    async fn state_with(open: bool) -> LwsState<Mem> {
        let mut cfg = super::super::LwsConfig::new(BASE);
        cfg.open = open;
        cfg.allow_insecure_fetch = true;
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        LwsState::new(store, cfg).await.expect("state")
    }

    async fn state() -> LwsState<Mem> {
        state_with(true).await
    }

    fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> LwsRequest {
        let mut h = HeaderMap::new();
        for (k, v) in headers {
            h.append(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        let (path, query) = match path.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (path.to_string(), None),
        };
        LwsRequest {
            method: Method::from_bytes(method.as_bytes()).unwrap(),
            path,
            query,
            headers: h,
            body: Bytes::from(body.to_string()),
            admission: None,
        }
    }

    async fn call(
        state: &LwsState<Mem>,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        super::super::route(state, request(method, path, headers, body)).await
    }

    async fn body_of(resp: Response) -> Bytes {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
    }

    async fn json_of(resp: Response) -> Value {
        serde_json::from_slice(&body_of(resp).await).unwrap()
    }

    fn hdr(resp: &Response, name: &str) -> String {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    fn links(resp: &Response) -> Vec<String> {
        resp.headers()
            .get_all(header::LINK)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect()
    }

    /// POST a data resource into the root; its URI.
    async fn post(
        state: &LwsState<Mem>,
        slug: &str,
        ct: &str,
        body: &str,
        extra: &[(&str, &str)],
    ) -> String {
        let mut h = vec![("slug", slug), ("content-type", ct)];
        h.extend_from_slice(extra);
        let r = call(state, "POST", "/", &h, body).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        hdr(&r, "location")
    }

    fn path_of(uri: &str) -> &str {
        uri.strip_prefix(BASE).unwrap()
    }

    #[tokio::test]
    async fn multi_range_and_if_range_serve_the_whole_body() {
        let st = state().await;
        let uri = post(&st, "r.txt", "text/plain", "0123456789", &[]).await;
        let p = path_of(&uri);
        let r = call(&st, "GET", p, &[("range", "bytes=0-1,4-5")], "").await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(&body_of(r).await[..], b"0123456789");
        let r = call(&st, "GET", p, &[("range", "bytes=50-60")], "").await;
        assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        let r = call(&st, "GET", p, &[], "").await;
        let etag = hdr(&r, "etag");
        let r = call(
            &st,
            "GET",
            p,
            &[("range", "bytes=2-3"), ("if-range", &etag)],
            "",
        )
        .await;
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(&body_of(r).await[..], b"23");
        let r = call(
            &st,
            "GET",
            p,
            &[("range", "bytes=2-3"), ("if-range", "\"stale\"")],
            "",
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(&body_of(r).await[..], b"0123456789");
    }

    #[tokio::test]
    async fn container_honours_the_ld_json_lws_profile() {
        let st = state().await;
        let r = call(
            &st,
            "POST",
            "/",
            &[
                ("slug", "c"),
                (
                    "link",
                    "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
                ),
            ],
            "",
        )
        .await;
        let c = hdr(&r, "location");
        let want = r#"application/ld+json; profile="https://www.w3.org/ns/lws/v1""#;
        let r = call(&st, "GET", path_of(&c), &[("accept", want)], "").await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(hdr(&r, "content-type"), want);
    }

    #[tokio::test]
    async fn storage_description_has_an_etag() {
        let st = state().await;
        let r = call(&st, "GET", "/", &[("accept", LWS_CID)], "").await;
        assert_eq!(r.status(), StatusCode::OK);
        let etag = hdr(&r, "etag");
        assert!(etag.starts_with('"'), "{etag}");
        let r = call(
            &st,
            "GET",
            "/",
            &[("accept", LWS_CID), ("if-none-match", &etag)],
            "",
        )
        .await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn root_authorization_follows_the_negotiated_representation() {
        let st = state_with(false).await;
        // lws+cid named but refused: the listing is what would be served, so authorization holds.
        let r = call(
            &st,
            "GET",
            "/",
            &[("accept", "application/lws+cid;q=0, application/lws+json")],
            "",
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let r = call(&st, "GET", "/", &[("accept", "application/lws+cid")], "").await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(!serves_description(Some("application/lws+cid;q=0, */*")));
        assert!(serves_description(Some(
            "application/lws+cid;q=0.5, application/lws+json"
        )));
    }

    #[tokio::test]
    async fn declared_types_are_link_headers() {
        let st = state().await;
        let uri = post(
            &st,
            "t.txt",
            "text/plain",
            "x",
            &[("link", "<https://ex.org/Note>; rel=\"type\"")],
        )
        .await;
        for m in ["GET", "HEAD"] {
            let r = call(&st, m, path_of(&uri), &[], "").await;
            let l = links(&r);
            assert!(
                l.iter()
                    .any(|v| v.contains("<https://ex.org/Note>; rel=\"type\"")),
                "{l:?}"
            );
            assert!(
                l.iter()
                    .any(|v| v.contains("lws#DataResource>; rel=\"type\"")),
                "{l:?}"
            );
        }
    }

    #[tokio::test]
    async fn linkset_carries_server_managed_metadata() {
        let st = state().await;
        let uri = post(
            &st,
            "m.txt",
            "text/plain",
            "hello",
            &[
                ("link", "<https://ex.org/Note>; rel=\"type\""),
                ("link", "<https://ex.org/lic>; rel=\"license\""),
            ],
        )
        .await;
        let meta = format!("{}{META_SUFFIX}", path_of(&uri));
        let r = call(&st, "GET", &meta, &[], "").await;
        assert!(!hdr(&r, "etag").is_empty());
        let doc = json_of(r).await;
        let e = &doc["linkset"][0];
        assert_eq!(e["anchor"], uri);
        assert_eq!(e["up"][0]["href"], format!("{BASE}/"));
        let types: Vec<&str> = e["type"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["href"].as_str())
            .collect();
        assert_eq!(
            types,
            vec![
                "https://www.w3.org/ns/lws#DataResource",
                "https://ex.org/Note"
            ]
        );
        assert_eq!(e["self"][0]["format"][0], "text/plain");
        assert_eq!(e["self"][0]["size"][0], "5");
        assert!(e["self"][0]["modified"][0].is_string());
        assert_eq!(e["license"][0]["href"], "https://ex.org/lic");
    }

    #[test]
    fn user_linkset_strips_server_relations_of_the_anchor_only() {
        let doc = json!({"linkset": [
            {"anchor": "http://h/a", "up": [{"href": "x"}], "self": [{"href": "y"}], "license": [{"href": "l"}]},
            {"anchor": "http://h/b", "up": [{"href": "z"}]},
            {"anchor": "/a", "type": [{"href": "t"}]},
        ]});
        assert_eq!(
            user_linkset(&doc, "http://h/a"),
            json!({"linkset": [
                {"anchor": "http://h/a", "license": [{"href": "l"}]},
                {"anchor": "http://h/b", "up": [{"href": "z"}]},
            ]})
        );
    }

    /// Review finding: nothing bounded the types and links a resource's metadata held. A resource's
    /// metadata is held to [`MAX_META_BYTES`](super::super::MAX_META_BYTES): a write that would
    /// pass it is refused before its content or its metadata is written.
    #[tokio::test]
    async fn metadata_is_held_to_its_size() {
        let st = state().await;
        let turtle = |n: usize| {
            (0..n)
                .map(|j| format!("<> a <https://ex.org/{j}/{}> .\n", "x".repeat(8000)))
                .collect::<String>()
        };
        let uri = post(&st, "m.ttl", "text/turtle", &turtle(1), &[]).await;
        let p = path_of(&uri);
        let mut refused = None;
        for i in 2..200 {
            let r = call(
                &st,
                "PUT",
                p,
                &[("content-type", "text/turtle")],
                &turtle(i),
            )
            .await;
            if !r.status().is_success() {
                refused = Some((i, r.status()));
                break;
            }
        }
        let (i, status) = refused.expect("the metadata never filled");
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(i > 2);
        // Neither the refused write's content nor its metadata was kept.
        let m = st.resource_meta(&uri).await.unwrap();
        assert!(encode_meta(&m).unwrap().len() <= super::super::MAX_META_BYTES);
        assert_eq!(m.types.len(), i - 1);
        assert!(!m.pending);
        let body = st.store.read(&uri).await.unwrap().body;
        assert_eq!(body, Bytes::from(turtle(i - 1)));
    }

    #[tokio::test]
    async fn put_leaves_the_linkset_alone_without_prefer() {
        let st = state().await;
        let uri = post(
            &st,
            "u.ttl",
            "text/turtle",
            "<> a <https://ex.org/Alpha> .",
            &[
                ("link", "<https://ex.org/Declared>; rel=\"type\""),
                ("link", "<https://ex.org/lic>; rel=\"license\""),
            ],
        )
        .await;
        let p = path_of(&uri);
        // Link headers without Prefer: set-linkset leave the links alone; the types they declare
        // replace the declared ones, and the content's own type follows the content.
        let r = call(
            &st,
            "PUT",
            p,
            &[
                ("content-type", "text/turtle"),
                ("link", "<https://ex.org/Other>; rel=\"type\""),
                ("link", "<https://ex.org/x>; rel=\"author\""),
            ],
            "<> a <https://ex.org/Beta> .",
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(r.headers().get("preference-applied").is_none());
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(
            m.types,
            vec![
                "https://ex.org/Other".to_string(),
                "https://ex.org/Beta".to_string()
            ]
        );
        assert_eq!(m.links.keys().collect::<Vec<_>>(), vec!["license"]);
        // With Prefer: set-linkset the Link headers replace the linkset.
        let r = call(
            &st,
            "PUT",
            p,
            &[
                ("content-type", "text/turtle"),
                ("prefer", "set-linkset"),
                ("link", "<https://ex.org/Other>; rel=\"type\""),
                ("link", "<https://ex.org/x>; rel=\"author\""),
            ],
            "<> a <https://ex.org/Beta> .",
        )
        .await;
        assert_eq!(hdr(&r, "preference-applied"), "set-linkset");
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(
            m.types,
            vec![
                "https://ex.org/Other".to_string(),
                "https://ex.org/Beta".to_string()
            ]
        );
        assert_eq!(m.links.keys().collect::<Vec<_>>(), vec!["author"]);
        let doc = json_of(call(&st, "GET", &format!("{p}{META_SUFFIX}"), &[], "").await).await;
        assert_eq!(doc["linkset"][0]["author"][0]["href"], "https://ex.org/x");
        assert!(doc["linkset"][0].get("license").is_none());
    }

    #[tokio::test]
    async fn conditional_writes_do_not_lose_updates() {
        let st = state().await;
        let uri = post(&st, "race.txt", "text/plain", "v0", &[]).await;
        let p = path_of(&uri).to_string();
        let etag = hdr(&call(&st, "GET", &p, &[], "").await, "etag");
        let h = [("content-type", "text/plain"), ("if-match", etag.as_str())];
        let (a, b) = tokio::join!(call(&st, "PUT", &p, &h, "a"), call(&st, "PUT", &p, &h, "b"));
        let mut codes = [a.status().as_u16(), b.status().as_u16()];
        codes.sort();
        assert_eq!(codes, [204, 412]);
    }

    #[tokio::test]
    async fn iri_locks_serialize_one_resource() {
        let locks = IriLocks::default();
        let g = locks.lock("http://h/a").await;
        // Another resource is free; the same one waits.
        let _other = locks.lock("http://h/b").await;
        let waited =
            tokio::time::timeout(Duration::from_millis(50), locks.lock("http://h/a")).await;
        assert!(waited.is_err());
        drop(g);
        assert!(
            tokio::time::timeout(Duration::from_millis(500), locks.lock("http://h/a"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn get_pairs_the_body_with_its_validators() {
        let st = state().await;
        let uri = post(&st, "v.txt", "text/plain", "one", &[]).await;
        let p = path_of(&uri);
        let put = call(&st, "PUT", p, &[("content-type", "text/plain")], "two").await;
        let tag = hdr(&put, "etag");
        let r = call(&st, "GET", p, &[], "").await;
        assert_eq!(hdr(&r, "etag"), tag);
        assert_eq!(&body_of(r).await[..], b"two");
    }

    const CONTAINER_LINK: &str = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";

    fn agent(subject: &str) -> Agent {
        Agent {
            subject: Some(subject.to_string()),
            client: None,
        }
    }

    /// Create a member of `parent` with `who` as its creator (only the creator matters here).
    async fn create_as(
        st: &LwsState<Mem>,
        who: &Agent,
        parent: &str,
        slug: &str,
        container: bool,
    ) -> String {
        let mut h = vec![("slug", slug), ("content-type", "text/plain")];
        if container {
            h.push(("link", CONTAINER_LINK));
        }
        // Created by the owner, then recorded as `who`'s: the create itself checks Create.
        let owner = agent(st.cfg.owner.as_deref().expect("an owner"));
        let r = create(st, &request("POST", "/", &h, "x"), &owner, parent).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let location = hdr(&r, "location");
        let mut meta = st.resource_meta(&location).await.unwrap();
        meta.creator = who.subject.clone();
        st.put_resource_meta(&location, &meta).await.unwrap();
        location
    }

    #[tokio::test]
    async fn recursive_delete_needs_delete_on_every_descendant() {
        let mut cfg = super::super::LwsConfig::new(BASE);
        cfg.owner = Some("https://owner.example/#me".into());
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let st = LwsState::new(store, cfg).await.expect("state");
        let (alice, bob) = (
            agent("https://alice.example/#me"),
            agent("https://bob.example/#me"),
        );
        let owner = agent("https://owner.example/#me");
        let root = st.cfg.storage();
        // Alice made /shared/ and has a file and a container in it; Bob has a file deeper down.
        let shared = create_as(&st, &alice, &root, "shared", true).await;
        let mine = create_as(&st, &alice, &shared, "mine.txt", false).await;
        let sub = create_as(&st, &alice, &shared, "sub", true).await;
        let theirs = create_as(&st, &bob, &sub, "bob.txt", false).await;
        let delete_as = |who: Agent, uri: String| {
            let st = st.clone();
            async move {
                let req = request("DELETE", path_of(&uri), &[("depth", "infinity")], "");
                handle(&st, &req, &who).await
            }
        };
        // Alice may delete /shared/ itself, but not Bob's file in it: 403, and nothing goes.
        let r = delete_as(alice.clone(), shared.clone()).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        for uri in [&shared, &mine, &sub, &theirs] {
            assert!(st.store.exists(uri).await.unwrap(), "{uri} survived");
        }
        // Nor the inner container that holds it.
        let r = delete_as(alice.clone(), sub.clone()).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(st.store.exists(&theirs).await.unwrap());
        // Her own file alone she may delete; the owner may delete everything.
        let r = delete_as(alice.clone(), mine.clone()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = delete_as(owner, shared.clone()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        for uri in [&shared, &sub, &theirs] {
            assert!(!st.store.exists(uri).await.unwrap(), "{uri} is gone");
        }
    }

    #[tokio::test]
    async fn recursive_delete_holds_the_subtree_locks() {
        let st = state().await;
        let c = hdr(
            &call(
                &st,
                "POST",
                "/",
                &[("slug", "d"), ("link", CONTAINER_LINK)],
                "",
            )
            .await,
            "location",
        );
        let leaf = post(&st, "f", "text/plain", "x", &[]).await;
        let inner = hdr(
            &call(
                &st,
                "POST",
                path_of(&c),
                &[("slug", "g"), ("content-type", "text/plain")],
                "x",
            )
            .await,
            "location",
        );
        // While a writer holds a descendant's lock, the delete waits for it.
        let held = st.locks.lock(&inner).await;
        let task = {
            let st = st.clone();
            let c = c.clone();
            tokio::spawn(async move {
                call(&st, "DELETE", path_of(&c), &[("depth", "infinity")], "")
                    .await
                    .status()
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!task.is_finished());
        assert!(st.store.exists(&inner).await.unwrap());
        drop(held);
        assert_eq!(task.await.unwrap(), StatusCode::NO_CONTENT);
        assert!(!st.store.exists(&inner).await.unwrap());
        assert!(st.store.exists(&leaf).await.unwrap());
        // The lock order puts a member before its container.
        assert_eq!(lock_order(&inner, &c), std::cmp::Ordering::Less);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_posts_with_one_slug_never_share_a_resource() {
        let st = state().await;
        let tasks: Vec<_> = (0..16)
            .map(|i| {
                let st = st.clone();
                tokio::spawn(async move {
                    let r = call(
                        &st,
                        "POST",
                        "/",
                        &[("slug", "same"), ("content-type", "text/plain")],
                        &format!("body {i}"),
                    )
                    .await;
                    assert_eq!(r.status(), StatusCode::CREATED);
                    (i, hdr(&r, "location"))
                })
            })
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        for t in tasks {
            let (i, location) = t.await.unwrap();
            assert!(
                seen.insert(location.clone()),
                "{location} was created twice"
            );
            let body = body_of(call(&st, "GET", path_of(&location), &[], "").await).await;
            assert_eq!(body, Bytes::from(format!("body {i}")));
        }
    }

    #[tokio::test]
    async fn create_holds_the_container_lock() {
        let st = state().await;
        let root = st.cfg.storage();
        let held = st.locks.lock(&root).await;
        let task = {
            let st = st.clone();
            tokio::spawn(async move { post(&st, "x", "text/plain", "x", &[]).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!task.is_finished());
        drop(held);
        let first = task.await.unwrap();
        // The name is taken now, so the next create with the Slug gets a fresh one.
        let second = post(&st, "x", "text/plain", "y", &[]).await;
        assert_ne!(first, second);
        assert_eq!(
            body_of(call(&st, "GET", path_of(&first), &[], "").await).await,
            Bytes::from("x")
        );
    }

    /// Review finding: a target found absent let an authenticated caller through unchecked, so a
    /// resource that appeared before the handler ran was served (or changed) without its
    /// permission check, and a backend error counted as absent.
    #[tokio::test]
    async fn a_resource_that_appears_after_the_check_is_not_served_unchecked() {
        use super::super::test_store::FlakyStore;
        use std::sync::atomic::Ordering;
        let store = FlakyStore::new();
        let mut cfg = super::super::LwsConfig::new(BASE);
        cfg.owner = Some("https://owner.example/#me".into());
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let owner = agent("https://owner.example/#me");
        let bob = agent("https://bob.example/#me");
        let h = [
            ("slug", "secret.json"),
            ("content-type", "application/json"),
        ];
        let r = create(
            &st,
            &request("POST", "/", &h, "{\"pin\": 1234}"),
            &owner,
            &st.cfg.storage(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let secret = hdr(&r, "location");
        let p = path_of(&secret);
        let get = request("GET", p, &[], "");
        assert_eq!(
            handle(&st, &get, &bob).await.status(),
            StatusCode::FORBIDDEN
        );
        // The existence check runs while the resource is "not there yet".
        *store.hide.lock().unwrap() = Some(secret.clone());
        for (method, ct, body) in [
            ("GET", "", ""),
            ("HEAD", "", ""),
            ("PUT", "application/json", "{}"),
            ("DELETE", "", ""),
        ] {
            let r = handle(
                &st,
                &request(method, p, &[("content-type", ct)], body),
                &bob,
            )
            .await;
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{method}");
        }
        *store.hide.lock().unwrap() = None;
        let r = handle(&st, &get, &owner).await;
        assert_eq!(body_of(r).await, Bytes::from("{\"pin\": 1234}"));
        // A backend failure is a 500, not an absent resource.
        store.fail_exists.store(true, Ordering::SeqCst);
        let r = handle(&st, &get, &bob).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let linkset = request("GET", &format!("{p}{META_SUFFIX}"), &[], "");
        let r = handle(&st, &linkset, &bob).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Review finding: a POST locked only the container, while a DELETE of a data resource holds
    /// the resource's own lock from removing its content to removing its metadata. A POST with
    /// the same Slug in between recreated the IRI, and the DELETE then removed the new resource's
    /// metadata. A name whose lock is held is now taken: the create goes elsewhere.
    #[tokio::test]
    async fn create_never_takes_a_name_whose_lock_is_held() {
        let st = state().await;
        let x = format!("{}x", st.cfg.storage());
        // A DELETE of `x` is mid-flight: its content is gone, its lock still held.
        let held = st.locks.lock(&x).await;
        let uri = post(
            &st,
            "x",
            "text/plain",
            "new",
            &[("link", "<https://e.example/T>; rel=\"type\"")],
        )
        .await;
        assert_eq!(uri, format!("{x}-2"));
        assert!(!st.store.exists(&x).await.unwrap());
        // The delete removes the (old) metadata and releases the lock.
        let _ = st.store.delete(&meta_key(&x), None).await;
        drop(held);
        // The new resource keeps its metadata.
        assert!(st
            .resource_meta(&uri)
            .await
            .unwrap()
            .types
            .contains(&"https://e.example/T".to_string()));
        // A container holds its spellings' locks as well: a held `y/` keeps a POST off `y`.
        let y = format!("{}y/", st.cfg.storage());
        let held = st.locks.lock(&y).await;
        let uri = post(&st, "y", "text/plain", "y", &[]).await;
        assert_eq!(uri, format!("{}y-2", st.cfg.storage()));
        drop(held);
        assert_eq!(
            post(&st, "y", "text/plain", "y", &[]).await,
            format!("{}y", st.cfg.storage())
        );
    }

    /// Review finding: the permission check ran before the resource's lock was taken, so a write
    /// that waited behind another could land on a state it was never authorized for (a delete and
    /// a re-create by someone else, a format a policy does not allow), and a read could serve a
    /// state other than the one it checked. Each request now checks again under the lock.
    #[tokio::test]
    async fn the_permission_check_holds_for_the_state_under_the_lock() {
        let mut cfg = super::super::LwsConfig::new(BASE);
        cfg.owner = Some("https://owner.example/#me".into());
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let st = LwsState::new(store, cfg).await.expect("state");
        let bob = agent("https://bob.example/#me");
        let root = st.cfg.storage();
        let file = create_as(&st, &bob, &root, "bob.json", false).await;
        let dir = create_as(&st, &bob, &root, "bobs", true).await;
        let set_creator = |uri: String, who: &'static str| {
            let st = st.clone();
            async move {
                let mut meta = st.resource_meta(&uri).await.unwrap();
                meta.creator = Some(who.to_string());
                st.put_resource_meta(&uri, &meta).await.unwrap();
            }
        };
        type Case<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)], &'a str);
        let cases: [Case<'_>; 4] = [
            ("GET", file.as_str(), &[], ""),
            (
                "PUT",
                file.as_str(),
                &[("content-type", "application/json")],
                "{\"bob\": 1}",
            ),
            ("DELETE", file.as_str(), &[], ""),
            (
                "POST",
                dir.as_str(),
                &[("slug", "new"), ("content-type", "text/plain")],
                "x",
            ),
        ];
        for (method, uri, headers, body) in cases {
            set_creator(uri.to_string(), "https://bob.example/#me").await;
            // Another writer holds the lock; Bob's request passes the first check and waits.
            let held = st.locks.lock(uri).await;
            let task = {
                let (st, bob) = (st.clone(), bob.clone());
                let req = request(method, path_of(uri), headers, body);
                tokio::spawn(async move { handle(&st, &req, &bob).await.status() })
            };
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(!task.is_finished(), "{method} did not wait for the lock");
            // Meanwhile the resource became the owner's (as a delete and a re-create would).
            set_creator(uri.to_string(), "https://owner.example/#me").await;
            drop(held);
            assert_eq!(task.await.unwrap(), StatusCode::FORBIDDEN, "{method}");
        }
        assert!(st.store.exists(&file).await.unwrap());
        assert_eq!(st.store.read(&file).await.unwrap().body, Bytes::from("x"));
        assert!(!st.store.exists(&format!("{dir}new")).await.unwrap());
    }

    /// Review finding: a container's listing tag hashed each member container's stored tag, which
    /// does not move when something is created inside it, so the listing that shows that
    /// container's new `modified` kept its tag; and a member's update did not advance the
    /// container's modified time, so If-Modified-Since met a 304 for a changed listing.
    #[tokio::test]
    async fn container_validators_follow_the_listed_members() {
        let st = state().await;
        let c = hdr(
            &call(
                &st,
                "POST",
                "/",
                &[("slug", "c"), ("link", CONTAINER_LINK)],
                "",
            )
            .await,
            "location",
        );
        // Make the container's modified time old, so a change now is visible at second precision.
        let mut meta = st.resource_meta(&c).await.unwrap();
        meta.modified_ms = Some(1_000_000);
        st.put_resource_meta(&c, &meta).await.unwrap();
        let root_tag = hdr(
            &call(&st, "GET", "/", &[("accept", LWS_JSON)], "").await,
            "etag",
        );
        let x = hdr(
            &call(
                &st,
                "POST",
                path_of(&c),
                &[("slug", "x"), ("content-type", "text/plain")],
                "1",
            )
            .await,
            "location",
        );
        let root = call(&st, "GET", "/", &[("accept", LWS_JSON)], "").await;
        assert_ne!(
            hdr(&root, "etag"),
            root_tag,
            "the root lists c/ with a new modified"
        );
        // A member's update advances the container's Last-Modified.
        let listed = call(&st, "GET", path_of(&c), &[("accept", LWS_JSON)], "").await;
        let since = hdr(&listed, "last-modified");
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let r = call(
            &st,
            "PUT",
            path_of(&x),
            &[("content-type", "text/plain")],
            "22",
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let again = call(
            &st,
            "GET",
            path_of(&c),
            &[("accept", LWS_JSON), ("if-modified-since", &since)],
            "",
        )
        .await;
        assert_eq!(again.status(), StatusCode::OK);
        assert_ne!(hdr(&again, "last-modified"), since);
    }

    /// Review finding: a data resource was removed in two steps, its parent's membership edge
    /// first, so a failure of the second left it readable but listed nowhere, and a retried
    /// recursive delete of its container could then succeed around it. The record and the edge
    /// now go in one step.
    #[tokio::test]
    async fn a_failed_delete_leaves_no_resource_outside_its_container() {
        use super::super::route;
        use super::super::test_store::{request as req, state as flaky_state};
        let (st, store) = flaky_state(100).await;
        let base = st.cfg.absolute("");
        let r = route(
            &st,
            req(
                Method::POST,
                "/",
                &[
                    ("slug", "d"),
                    (
                        "link",
                        "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
                    ),
                    ("content-type", "text/turtle"),
                ],
                "",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let d = hdr(&r, "location");
        let dp = d.strip_prefix(base.as_str()).unwrap().to_string();
        let r = route(
            &st,
            req(
                Method::POST,
                &dp,
                &[("slug", "x.json"), ("content-type", "application/json")],
                "{}",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let x = hdr(&r, "location");
        *store.partial_delete_of.lock().unwrap() = Some(x.clone());
        for (path, depth) in [(x.clone(), None), (d.clone(), Some("infinity"))] {
            let p = path.strip_prefix(base.as_str()).unwrap();
            let headers: Vec<(&str, &str)> = depth.map(|v| ("depth", v)).into_iter().collect();
            route(&st, req(Method::DELETE, p, &headers, "")).await;
            let listed = st
                .store
                .list_children(&d)
                .await
                .map(|c| c.iter().any(|c| c.as_str() == x))
                .unwrap_or(false);
            assert!(
                !st.store.exists(&x).await.unwrap() || listed,
                "{x} outlived its delete outside its container"
            );
        }
    }

    /// Review finding: PUT committed the content and discarded a failure to write the
    /// metadata, answering 204, so a `Prefer: set-linkset` that dropped a type a grant rests on
    /// could fail silently and leave the new content under the old types. A failure now answers
    /// 500 and leaves content and metadata as they were.
    #[tokio::test]
    async fn a_failed_metadata_write_leaves_the_resource_as_it_was() {
        use super::super::route;
        use super::super::test_store::{request as req, state as flaky_state};
        let (st, store) = flaky_state(100).await;
        let base = st.cfg.absolute("");
        let public = "<https://e.example/Public>; rel=\"type\"";
        let r = route(
            &st,
            req(
                Method::POST,
                "/",
                &[
                    ("slug", "doc.json"),
                    ("content-type", "application/json"),
                    ("link", public),
                ],
                "{\"v\": 0}",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let uri = hdr(&r, "location");
        let p = uri.strip_prefix(base.as_str()).unwrap().to_string();
        let types = |st: LwsState<super::super::test_store::FlakyStore>, uri: String| async move {
            st.resource_meta(&uri).await.unwrap().types
        };
        let before = types(st.clone(), uri.clone()).await;
        assert!(before.contains(&"https://e.example/Public".to_string()));
        let attempts = [(Method::PUT, "application/json", "{\"v\": 1}")];
        // The metadata write fails, then the content write does: either way nothing changes.
        for failing in [meta_key(&uri), uri.clone()] {
            *store.fail_write_of.lock().unwrap() = Some(failing.clone());
            for (method, ct, body) in attempts.clone() {
                let r = route(
                    &st,
                    req(
                        method.clone(),
                        &p,
                        &[("content-type", ct), ("prefer", "set-linkset")],
                        body,
                    ),
                )
                .await;
                assert_eq!(
                    r.status(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "{method} {failing}"
                );
                assert_eq!(
                    st.store.read(&uri).await.unwrap().body,
                    Bytes::from("{\"v\": 0}")
                );
                assert_eq!(
                    types(st.clone(), uri.clone()).await,
                    before,
                    "{method} {failing}"
                );
            }
        }
        *store.fail_write_of.lock().unwrap() = None;
        let r = route(
            &st,
            req(
                Method::PUT,
                &p,
                &[
                    ("content-type", "application/json"),
                    ("prefer", "set-linkset"),
                ],
                "{\"v\": 1}",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(!types(st.clone(), uri.clone())
            .await
            .contains(&"https://e.example/Public".to_string()));
    }

    /// Review finding: the reserved `.meta` suffix was checked before the Slug was cut to its
    /// length limit, so a long Slug could end in `.meta` once cut and name an unreachable resource.
    #[test]
    fn a_cut_slug_never_ends_in_the_reserved_suffix() {
        let slug = format!("{}.metax", "a".repeat(115));
        let name = sanitize_slug(Some(&slug)).unwrap();
        assert!(name.len() <= 120);
        assert!(!name.to_ascii_lowercase().ends_with(META_SUFFIX), "{name}");
        let upper = format!("{}.METAx", "a".repeat(115));
        assert!(!sanitize_slug(Some(&upper))
            .unwrap()
            .to_ascii_lowercase()
            .ends_with(META_SUFFIX));
    }

    /// Review finding: a create ignored the request's preconditions: a `POST` with
    /// `If-None-Match: *` to an existing container created a member. They are now evaluated
    /// against the target's validators before anything changes.
    #[tokio::test]
    async fn creates_and_service_deletes_evaluate_preconditions() {
        let st = state().await;
        let container = format!("<{LWS_NS}Container>; rel=\"type\"");
        let r = call(&st, "POST", "/", &[("slug", "c"), ("link", &container)], "").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let text = ("content-type", "text/plain");
        let listing = call(&st, "GET", "/c/", &[], "").await;
        let tag = hdr(&listing, "etag");
        let count = || async {
            json_of(call(&st, "GET", "/c/", &[], "").await).await["totalItems"].clone()
        };
        let before = count().await;
        for refused in [("if-none-match", "*"), ("if-match", "\"other\"")] {
            let r = call(&st, "POST", "/c/", &[text, refused], "x").await;
            assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED, "{refused:?}");
        }
        assert_eq!(count().await, before);
        let r = call(&st, "POST", "/c/", &[text, ("if-match", &tag)], "x").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        // The listing changed: the old tag no longer matches.
        let r = call(&st, "POST", "/c/", &[text, ("if-match", &tag)], "x").await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
    }

    /// Review finding: the types a Turtle representation states were deduplicated by scanning
    /// those already found, so a body stating a hundred thousand types cost billions of
    /// comparisons. Repeats are found with a set, and the first statement of each type keeps its
    /// place.
    #[test]
    fn stated_types_are_deduplicated_with_a_set() {
        let uri = "http://h/r";
        let list: Vec<String> = (0..100_000).map(|i| format!("<urn:t{i}>")).collect();
        let body = format!("<> a {}, <urn:t1>, <urn:t0> .", list.join(", "));
        let types = content_types(uri, "text/turtle", body.as_bytes()).unwrap();
        assert_eq!(types.len(), 100_000);
        assert_eq!(types[..2], ["urn:t0".to_string(), "urn:t1".to_string()]);
        let all = all_types(&["urn:t5".into(), "urn:x".into()], types);
        assert_eq!(all.len(), 100_001);
        assert_eq!(all[..3], ["urn:t5", "urn:x", "urn:t0"].map(String::from));
    }

    /// Review finding: a Turtle body could state types through a long prefix used many times,
    /// so a small upload expanded to gigabytes of IRIs before any quota applied. Expanded terms
    /// are held to a budget, and a body past it is refused.
    #[tokio::test]
    async fn stated_types_are_held_to_an_expansion_budget() {
        let st = state().await;
        let long = format!("https://p.example/{}#", "x".repeat(100_000));
        let types: Vec<String> = (0..200).map(|i| format!("p:T{i}")).collect();
        let body = format!("@prefix p: <{long}> .\n<> a {} .", types.join(", "));
        assert!(content_types("http://h/r", "text/turtle", body.as_bytes()).is_err());
        let turtle = ("content-type", "text/turtle");
        let r = call(&st, "POST", "/", &[turtle, ("slug", "t.ttl")], &body).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let r = call(
            &st,
            "POST",
            "/",
            &[turtle, ("slug", "t.ttl")],
            "<> a <urn:t> .",
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let r = call(&st, "PUT", "/t.ttl", &[turtle], &body).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// Review finding: HEAD applied Range, answering 206 or 416 with a part's length, where it
    /// must describe the whole representation (RFC 9110 section 14.2).
    #[tokio::test]
    async fn head_ignores_range() {
        let st = state().await;
        let text = [("content-type", "text/plain"), ("slug", "r.txt")];
        let r = call(&st, "POST", "/", &text, "twelve bytes").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        for range in ["bytes=0-0", "bytes=100-200"] {
            let r = call(&st, "HEAD", "/r.txt", &[("range", range)], "").await;
            assert_eq!(r.status(), StatusCode::OK, "{range}");
            let r = call(&st, "GET", "/r.txt", &[("range", range)], "").await;
            assert_ne!(r.status(), StatusCode::OK, "{range}");
        }
    }

    /// Review finding: a listing took each member's fields and entity tag from one version and
    /// its size from whatever the content was when the page was written, so a write in between
    /// gave one tag to two bodies. The size is read from the version the entry describes.
    #[tokio::test]
    async fn a_listing_sizes_the_version_it_describes() {
        let st = state().await;
        let text = [("content-type", "text/plain"), ("slug", "m.txt")];
        let r = call(&st, "POST", "/", &text, "four").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let all = members(&st, &st.cfg.storage()).await.unwrap();
        let r = call(
            &st,
            "PUT",
            "/m.txt",
            &[("content-type", "text/plain")],
            "eleven long",
        )
        .await;
        assert!(r.status().is_success());
        let mut shown: Vec<_> = all
            .into_iter()
            .filter(|(m, _)| m["id"].as_str().is_some_and(|i| i.ends_with("/m.txt")))
            .collect();
        with_sizes(&st, &mut shown).await;
        assert_ne!(shown[0].0["size"], json!(11), "{}", shown[0].0);
    }

    /// Review finding: a container created with a custom `rel="type"` (and other links) lost
    /// them: only data resources kept what their Link headers declared.
    #[tokio::test]
    async fn a_created_container_keeps_its_declared_types_and_links() {
        let st = state().await;
        let link = format!(
            "<{LWS_NS}Container>; rel=\"type\", <https://e.example/Album>; rel=\"type\", <https://e.example/schema>; rel=\"describedby\""
        );
        let r = call(&st, "POST", "/", &[("slug", "album"), ("link", &link)], "").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let m = st.resource_meta(&st.cfg.absolute("/album/")).await.unwrap();
        assert_eq!(m.types, vec!["https://e.example/Album".to_string()]);
        assert_eq!(
            m.links["describedby"],
            vec!["https://e.example/schema".to_string()]
        );
        let r = call(&st, "GET", "/album/", &[], "").await;
        assert!(
            r.headers()
                .get_all(header::LINK)
                .iter()
                .any(|v| v.to_str().unwrap().contains("https://e.example/Album")),
            "{:?}",
            r.headers()
        );
    }

    /// Review finding: a change inside a container updated that container's modification time,
    /// which its own container lists, without holding that listing: a conditional create there
    /// could check one listing and commit under another. The touch now holds it shared.
    #[tokio::test]
    async fn a_touch_holds_the_listing_above_it() {
        let st = state().await;
        let container = format!("<{LWS_NS}Container>; rel=\"type\"");
        for (at, slug) in [("/", "c"), ("/c/", "d")] {
            let r = call(&st, "POST", at, &[("slug", slug), ("link", &container)], "").await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let d = st.cfg.absolute("/c/d/");
        let before = st.resource_meta(&d).await.unwrap().modified_ms;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let held = st.locks.lock(&st.cfg.absolute("/c/")).await;
        let post = {
            let st = st.clone();
            tokio::spawn(async move {
                let text = ("content-type", "text/plain");
                call(&st, "POST", "/c/d/", &[text], "x").await.status()
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(st.resource_meta(&d).await.unwrap().modified_ms, before);
        drop(held);
        assert_eq!(post.await.unwrap(), StatusCode::CREATED);
        assert_ne!(st.resource_meta(&d).await.unwrap().modified_ms, before);
    }

    /// Review finding: a create whose store call reported a failure removed the new member's
    /// metadata, though a remote store may have committed the content before its reply was lost:
    /// the content stayed, without its creator, types and links. The metadata now goes only once
    /// the content is known to be gone.
    #[tokio::test]
    async fn a_create_whose_outcome_is_unknown_keeps_its_creator() {
        use super::super::test_store::{request as req, FlakyStore};
        use std::sync::atomic::Ordering;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = FlakyStore::new();
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let owner = agent("https://owner.example/#me");
        let post = |slug: &'static str| {
            req(
                Method::POST,
                "/",
                &[("slug", slug), ("content-type", "text/plain")],
                "x",
            )
        };
        // The content lands, the reply is lost, and the content cannot be removed: the creator
        // stays recorded.
        let kept = st.cfg.absolute("/kept.txt");
        store.fail_after_create.store(true, Ordering::SeqCst);
        *store.fail_delete_of.lock().unwrap() = Some(kept.clone());
        let r = create(&st, &post("kept.txt"), &owner, &st.cfg.storage()).await;
        assert!(r.status().is_server_error(), "{}", r.status());
        *store.fail_delete_of.lock().unwrap() = None;
        assert!(st.store.exists(&kept).await.unwrap());
        assert_eq!(
            st.resource_meta(&kept).await.unwrap().creator,
            owner.subject
        );
        // When the content can be removed, it goes, and its metadata with it.
        let gone = st.cfg.absolute("/gone.txt");
        let r = create(&st, &post("gone.txt"), &owner, &st.cfg.storage()).await;
        assert!(r.status().is_server_error(), "{}", r.status());
        store.fail_after_create.store(false, Ordering::SeqCst);
        assert!(!st.store.exists(&gone).await.unwrap());
        assert!(stored_meta(&st, &gone).await.unwrap().is_none());
    }

    /// Review finding: HEAD responses dropped the body before the length was derived from it, so
    /// they advertised `Content-Length: 0` for every representation.
    #[tokio::test]
    async fn head_advertises_the_representation_length() {
        use axum::body::Body;
        use tower::ServiceExt;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let store = super::super::test_store::FlakyStore::new();
        let app = super::super::router(store, cfg).await.expect("router");
        let send = |method: &str, uri: &str, body: &'static str| {
            let req = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("slug", "doc.txt")
                .header("content-type", "text/plain")
                .body(Body::from(body))
                .unwrap();
            app.clone().oneshot(req)
        };
        let r = send("POST", "/", "twelve bytes").await.unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        for uri in ["/doc.txt", "/"] {
            let get = send("GET", uri, "").await.unwrap();
            let body = body_of(get).await;
            assert!(!body.is_empty());
            let head = send("HEAD", uri, "").await.unwrap();
            assert_eq!(head.status(), StatusCode::OK);
            assert_eq!(
                hdr(&head, "content-length"),
                body.len().to_string(),
                "{uri}"
            );
            assert!(body_of(head).await.is_empty());
        }
    }

    /// Sweep finding: HEAD on a 304 carried `Content-Length: 0`, which RFC 9110 section 8.6
    /// forbids unless the 200 has that length.
    #[tokio::test]
    async fn bodiless_heads_and_service_reads_follow_their_preconditions() {
        use axum::body::Body;
        use tower::ServiceExt;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let store = super::super::test_store::FlakyStore::new();
        let app = super::super::router(store, cfg).await.expect("router");
        let send = |method: &str, uri: &str, headers: &[(&str, &str)], body: String| {
            let mut req = axum::http::Request::builder().method(method).uri(uri);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            app.clone().oneshot(req.body(Body::from(body)).unwrap())
        };
        let text = [("slug", "doc.txt"), ("content-type", "text/plain")];
        let r = send("POST", "/", &text, "twelve bytes".into())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        for uri in ["/doc.txt"] {
            let tag = hdr(&send("GET", uri, &[], String::new()).await.unwrap(), "etag");
            let head = send("HEAD", uri, &[("if-none-match", &tag)], String::new())
                .await
                .unwrap();
            assert_eq!(head.status(), StatusCode::NOT_MODIFIED, "{uri}");
            assert!(
                head.headers().get("content-length").is_none(),
                "{uri} {:?}",
                head.headers()
            );
            let r = send("GET", uri, &[("if-match", "\"other\"")], String::new())
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED, "{uri}");
            let r = send("GET", uri, &[("if-match", &tag)], String::new())
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK, "{uri}");
        }
    }

    /// Sweep finding: once more than a thousand IRI locks were held at once (a recursive delete
    /// of a large container holds one per member) every lock taken swept the whole map, and the
    /// sweep removed nothing. A sweep now waits for the map to double.
    #[tokio::test]
    async fn the_lock_map_is_swept_in_amortised_time() {
        let locks = IriLocks::default();
        let mut held = Vec::new();
        for i in 0..5000 {
            held.push(locks.lock(&format!("http://h/{i}")).await);
        }
        let sweeps = locks.0.lock().unwrap().sweep_at;
        assert!(sweeps >= 2 * LOCK_SWEEP_FLOOR, "{sweeps}");
        // Between sweeps the map grows to twice what the last one left.
        for i in 5000..5100 {
            held.push(locks.lock(&format!("http://h/{i}")).await);
        }
        assert_eq!(locks.0.lock().unwrap().sweep_at, sweeps);
        drop(held);
        let _ = locks.lock("http://h/x").await;
        assert!(locks.0.lock().unwrap().locks.len() <= 5101);
    }

    /// Review finding: a write detached from its request held no admission permit. Once the
    /// request timed out its slot was free again while the write still waited on the store, so
    /// stalled writes could pile up past the concurrency ceiling. The write now holds the slot
    /// until it ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_writes_keep_their_admission_slot() {
        use super::super::test_store::FlakyStore;
        use crate::app::{with_overload_layers, OverloadConfig};
        use axum::body::Body;
        use std::time::Duration;
        use tower::ServiceExt;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let store = FlakyStore::new();
        let app = super::super::router(store.clone(), cfg)
            .await
            .expect("router");
        let app = with_overload_layers(
            app,
            OverloadConfig::new(1, Some(Duration::from_millis(100))),
        );
        let send = |method: &str, body: &'static str| {
            let req = axum::http::Request::builder()
                .method(method)
                .uri("/doc.txt")
                .header("content-type", "text/plain")
                .body(Body::from(body))
                .unwrap();
            app.clone().oneshot(req)
        };
        let post = axum::http::Request::builder()
            .method("POST")
            .uri("/")
            .header("slug", "doc.txt")
            .header("content-type", "text/plain")
            .body(Body::from("first"))
            .unwrap();
        let r = app.clone().oneshot(post).await.unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        // The PUT's write stalls on the store; the request times out.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let uri = "http://localhost:3000/doc.txt".to_string();
        *store.hold_next_write_of.lock().unwrap() = Some((uri, gate.clone()));
        let r = send("PUT", "second").await.unwrap();
        assert_eq!(r.status(), StatusCode::GATEWAY_TIMEOUT);
        // The write still holds the only slot: the next request is shed.
        let r = send("GET", "").await.unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Once the write is over the slot is free again.
        gate.add_permits(1);
        let mut status = StatusCode::SERVICE_UNAVAILABLE;
        for _ in 0..200 {
            status = send("GET", "").await.unwrap().status();
            if status != StatusCode::SERVICE_UNAVAILABLE {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(status, StatusCode::OK);
    }

    /// Review finding: a conditional DELETE of a container computed the listing's tag only for
    /// If-Match; If-Unmodified-Since used the stored body's time and `If-None-Match: *` let the
    /// delete of an existing container through.
    #[tokio::test]
    async fn conditional_container_deletes_use_the_listing_validators() {
        let st = state().await;
        let c = hdr(
            &call(
                &st,
                "POST",
                "/",
                &[("slug", "c"), ("link", CONTAINER_LINK)],
                "",
            )
            .await,
            "location",
        );
        let created = http_date(to_secs(now_ms()));
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let r = call(
            &st,
            "POST",
            path_of(&c),
            &[("slug", "m"), ("content-type", "text/plain")],
            "x",
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let p = path_of(&c);
        let del = |h: &[(&str, &str)]| {
            let mut h = h.to_vec();
            h.push(("depth", "infinity"));
            super::super::route(&st, request("DELETE", p, &h, ""))
        };
        assert_eq!(
            del(&[("if-none-match", "*")]).await.status(),
            StatusCode::PRECONDITION_FAILED
        );
        // The listing changed after `created` (a member arrived), though the stored body did not.
        assert_eq!(
            del(&[("if-unmodified-since", &created)]).await.status(),
            StatusCode::PRECONDITION_FAILED
        );
        let listing = call(&st, "GET", p, &[("accept", LWS_JSON)], "").await;
        let (tag, modified) = (hdr(&listing, "etag"), hdr(&listing, "last-modified"));
        assert_eq!(
            del(&[("if-none-match", &tag)]).await.status(),
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(
            del(&[("if-match", "\"other\"")]).await.status(),
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(
            del(&[("if-match", &tag), ("if-unmodified-since", &modified)])
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }

    /// Review finding: a DELETE ignored a failure to remove the metadata, which then described
    /// the next resource created at the IRI; and a create wrote its content before its creator
    /// metadata, so a failure in between left the new content under the old creator.
    #[tokio::test]
    async fn stale_metadata_never_describes_new_content() {
        use super::super::test_store::{request as req, FlakyStore};
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = FlakyStore::new();
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let (owner, bob) = (
            agent("https://owner.example/#me"),
            agent("https://bob.example/#me"),
        );
        let base = st.cfg.absolute("");
        let x = format!("{}x", st.cfg.storage());
        let px = x.strip_prefix(base.as_str()).unwrap().to_string();
        let post = |who: Agent| {
            let st = st.clone();
            async move {
                let h = [("slug", "x"), ("content-type", "text/plain")];
                create(
                    &st,
                    &req(Method::POST, "/", &h, "owner's"),
                    &who,
                    &st.cfg.storage(),
                )
                .await
            }
        };
        // Bob's resource; its delete cannot remove the metadata: a 500, not a quiet 204.
        assert_eq!(post(owner.clone()).await.status(), StatusCode::CREATED);
        let mut meta = st.resource_meta(&x).await.unwrap();
        meta.creator = bob.subject.clone();
        st.put_resource_meta(&x, &meta).await.unwrap();
        *store.fail_delete_of.lock().unwrap() = Some(meta_key(&x));
        let r = handle(&st, &req(Method::DELETE, &px, &[], ""), &owner).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        *store.fail_delete_of.lock().unwrap() = None;
        assert!(!st.store.exists(&x).await.unwrap());
        assert_eq!(st.resource_meta(&x).await.unwrap().creator, bob.subject);
        // A create at the IRI whose metadata write fails creates nothing under Bob's metadata.
        *store.fail_write_of.lock().unwrap() = Some(meta_key(&x));
        assert!(post(owner.clone()).await.status().is_server_error());
        *store.fail_write_of.lock().unwrap() = None;
        assert!(!st.store.exists(&x).await.unwrap());
        // One that succeeds replaces it: the new content is the owner's, not Bob's.
        assert_eq!(post(owner.clone()).await.status(), StatusCode::CREATED);
        assert_eq!(st.resource_meta(&x).await.unwrap().creator, owner.subject);
        let r = handle(&st, &req(Method::GET, &px, &[], ""), &bob).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }

    /// A create whose client goes away mid-way still finishes whole: its writes hold the new
    /// member's locks until they are over. The stale metadata an earlier resource left is replaced
    /// before any content exists, and the content lands under the creator's own metadata.
    #[tokio::test]
    async fn a_cancelled_create_leaves_nothing_behind() {
        use super::super::test_store::{request as req, FlakyStore};
        use std::time::Duration;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = FlakyStore::new();
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let (owner, bob) = (
            agent("https://owner.example/#me"),
            agent("https://bob.example/#me"),
        );
        let base = st.cfg.absolute("");
        let x = format!("{}x", st.cfg.storage());
        let px = x.strip_prefix(base.as_str()).unwrap().to_string();
        let post = |who: Agent| {
            let st = st.clone();
            async move {
                let h = [("slug", "x"), ("content-type", "text/plain")];
                create(
                    &st,
                    &req(Method::POST, "/", &h, "owner's"),
                    &who,
                    &st.cfg.storage(),
                )
                .await
            }
        };
        // Bob's metadata outlives his resource (its removal failed).
        assert_eq!(post(owner.clone()).await.status(), StatusCode::CREATED);
        let mut meta = st.resource_meta(&x).await.unwrap();
        meta.creator = bob.subject.clone();
        st.put_resource_meta(&x, &meta).await.unwrap();
        st.store.delete(&x, Some(&st.cfg.storage())).await.unwrap();
        assert_eq!(st.resource_meta(&x).await.unwrap().creator, bob.subject);
        // A create whose content write stalls, and whose client goes away.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_create.lock().unwrap() = Some(gate.clone());
        let cancelled = tokio::time::timeout(Duration::from_millis(50), post(owner.clone())).await;
        assert!(cancelled.is_err());
        // Mid-way: Bob's metadata is already gone, and no content exists.
        assert!(!st.store.exists(&x).await.unwrap());
        assert_eq!(st.resource_meta(&x).await.unwrap().creator, owner.subject);
        // The write lands, whole and the owner's.
        gate.add_permits(1);
        for _ in 0..200 {
            if st.store.exists(&x).await.unwrap() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            st.store.read(&x).await.unwrap().body,
            Bytes::from("owner's")
        );
        assert_eq!(st.resource_meta(&x).await.unwrap().creator, owner.subject);
        let r = handle(&st, &req(Method::GET, &px, &[], ""), &bob).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        // Nothing is left locked: the next create goes through.
        let r = tokio::time::timeout(Duration::from_secs(5), post(owner.clone()))
            .await
            .expect("the container stayed locked");
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(hdr(&r, "location"), format!("{x}-2"));
    }

    /// Review finding: with the create's writes inline, a client that went away released the
    /// locks while a remote create it had sent was still pending. A second POST with the same
    /// Slug took the name and wrote its creator's metadata, and the late commit then put the first
    /// content under it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_late_commit_never_lands_under_another_creators_metadata() {
        use super::super::test_store::{request as req, FlakyStore};
        use std::time::Duration;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = FlakyStore::new();
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let (owner, bob) = (
            agent("https://owner.example/#me"),
            agent("https://bob.example/#me"),
        );
        let post = |who: Agent, body: &'static str| {
            let st = st.clone();
            async move {
                let h = [("slug", "x"), ("content-type", "text/plain")];
                let c = format!("{}c/", st.cfg.storage());
                create(&st, &req(Method::POST, "/c/", &h, body), &who, &c).await
            }
        };
        // A container of Bob's, so both may create in it.
        let c = format!("{}c/", st.cfg.storage());
        let h = [
            ("slug", "c"),
            (
                "link",
                "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
            ),
        ];
        let r = create(
            &st,
            &req(Method::POST, "/", &h, ""),
            &bob,
            &st.cfg.storage(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r = create(
            &st,
            &req(Method::POST, "/", &h, ""),
            &owner,
            &st.cfg.storage(),
        )
        .await;
        assert_eq!(hdr(&r, "location"), c);
        let mut meta = st.resource_meta(&c).await.unwrap();
        meta.creator = bob.subject.clone();
        st.put_resource_meta(&c, &meta).await.unwrap();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_create.lock().unwrap() = Some(gate.clone());
        let cancelled =
            tokio::time::timeout(Duration::from_millis(50), post(owner.clone(), "owner's")).await;
        assert!(cancelled.is_err());
        // Bob's POST with the same Slug, while the owner's create is still pending.
        let second = tokio::spawn(post(bob.clone(), "bob's"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        gate.add_permits(1);
        let r = tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .expect("the second create never finished")
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        let x = format!("{c}x");
        for _ in 0..200 {
            if st.store.exists(&x).await.unwrap() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Each member's content is under its own creator's metadata.
        let bobs = hdr(&r, "location");
        for (uri, body, who) in [(&x, "owner's", &owner), (&bobs, "bob's", &bob)] {
            assert_eq!(
                st.store.read(uri).await.unwrap().body,
                Bytes::from(body),
                "{uri}"
            );
            let creator = st.resource_meta(uri).await.unwrap().creator;
            assert_eq!(creator, who.subject, "{uri}");
        }
    }

    /// Review finding: a content-only PUT and a DELETE made their store writes
    /// inline, so a client that went away released the resource's lock while a write sent to a
    /// remote store could still commit. A delete and a re-create by someone else could then slip
    /// in, and the late write landed on the new resource: the old creator over it, or old content
    /// under the new creator. A cancelled DELETE stopped half way, leaving metadata behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_mutations_hold_their_locks_until_the_store_answers() {
        use super::super::test_store::{request as req, FlakyStore};
        use std::time::Duration;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = FlakyStore::new();
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let (owner, bob) = (
            agent("https://owner.example/#me"),
            agent("https://bob.example/#me"),
        );
        // A container of Bob's, so both may create in it.
        let c = format!("{}c/", st.cfg.storage());
        let h = [
            ("slug", "c"),
            (
                "link",
                "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
            ),
        ];
        let r = create(
            &st,
            &req(Method::POST, "/", &h, ""),
            &owner,
            &st.cfg.storage(),
        )
        .await;
        assert_eq!(hdr(&r, "location"), c);
        let mut meta = st.resource_meta(&c).await.unwrap();
        meta.creator = bob.subject.clone();
        st.put_resource_meta(&c, &meta).await.unwrap();
        async fn post(
            st: LwsState<FlakyStore>,
            who: Agent,
            slug: &'static str,
            body: &'static str,
        ) -> Response {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let c = format!("{}c/", st.cfg.storage());
            create(&st, &req(Method::POST, "/c/", &h, body), &who, &c).await
        }
        // Once the cancelled request's write is let through, the owner deletes the resource and
        // Bob creates one at the IRI.
        let replace = |name: &'static str, gate: Arc<tokio::sync::Semaphore>| {
            let (st, owner, bob) = (st.clone(), owner.clone(), bob.clone());
            async move {
                let next = tokio::spawn({
                    let (st, bob) = (st.clone(), bob.clone());
                    async move {
                        let path = format!("/c/{name}");
                        let r = handle(&st, &req(Method::DELETE, &path, &[], ""), &owner).await;
                        assert_eq!(r.status(), StatusCode::NO_CONTENT, "{name}");
                        post(st, bob, name, "bob's").await
                    }
                });
                tokio::time::sleep(Duration::from_millis(100)).await;
                gate.add_permits(1);
                let r = tokio::time::timeout(Duration::from_secs(5), next)
                    .await
                    .expect("the replacement never finished")
                    .unwrap();
                assert_eq!(r.status(), StatusCode::CREATED, "{name}");
                let uri = format!("{}c/{name}", st.cfg.storage());
                assert_eq!(hdr(&r, "location"), uri);
                // Whatever was late is over: the resource is Bob's, content and metadata.
                tokio::time::sleep(Duration::from_millis(100)).await;
                assert_eq!(
                    st.store.read(&uri).await.unwrap().body,
                    Bytes::from("bob's")
                );
                let m = st.resource_meta(&uri).await.unwrap();
                assert_eq!(m.creator, bob.subject, "{name}");
            }
        };
        // A content-only PUT whose write is pending when the client goes away.
        let y = format!("{c}y");
        assert_eq!(
            post(st.clone(), owner.clone(), "y", "owner's")
                .await
                .status(),
            StatusCode::CREATED
        );
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_write_of.lock().unwrap() = Some((y.clone(), gate.clone()));
        let h = [("content-type", "text/plain")];
        let cancelled = tokio::time::timeout(
            Duration::from_millis(50),
            handle(&st, &req(Method::PUT, "/c/y", &h, "late"), &owner),
        )
        .await;
        assert!(cancelled.is_err());
        replace("y", gate).await;
        // A DELETE whose removal is pending when the client goes away still finishes: the
        // metadata goes too.
        let z = format!("{c}z");
        assert_eq!(
            post(st.clone(), owner.clone(), "z", "owner's")
                .await
                .status(),
            StatusCode::CREATED
        );
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((z.clone(), gate.clone()));
        let cancelled = tokio::time::timeout(
            Duration::from_millis(50),
            handle(&st, &req(Method::DELETE, "/c/z", &[], ""), &owner),
        )
        .await;
        assert!(cancelled.is_err());
        gate.add_permits(1);
        for _ in 0..200 {
            if stored_meta(&st, &z).await.unwrap().is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!st.store.exists(&z).await.unwrap());
        assert!(stored_meta(&st, &z).await.unwrap().is_none());
    }

    /// Review finding: stored metadata that did not parse was read as the defaults.
    #[tokio::test]
    async fn malformed_stored_metadata_is_an_error() {
        let st = state().await;
        let uri = post(&st, "m.txt", "text/plain", "x", &[]).await;
        st.store
            .write(
                &meta_key(&uri),
                Bytes::from("{not json"),
                "application/json",
            )
            .await
            .unwrap();
        assert!(st.resource_meta(&uri).await.is_err());
        for p in [
            path_of(&uri).to_string(),
            format!("{}{META_SUFFIX}", path_of(&uri)),
        ] {
            let r = call(&st, "GET", &p, &[], "").await;
            assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR, "{p}");
        }
        // A permission check that rests on it denies (or fails), never grants on the defaults.
        let closed = state_with(false).await;
        let uri = closed.cfg.absolute("/c.txt");
        closed
            .store
            .write(&uri, Bytes::from("x"), "text/plain")
            .await
            .unwrap();
        closed
            .store
            .write(
                &meta_key(&uri),
                Bytes::from("{not json"),
                "application/json",
            )
            .await
            .unwrap();
        let bob = agent("https://bob.example/#me");
        assert!(closed.check(Action::Read, &uri, &bob).await.is_err());
        assert!(!closed.allowed(Action::Read, &uri, &bob).await);
    }

    /// Review finding: the name search read a backend failure as "name taken" and tried the next
    /// name, without end, under the container's lock.
    #[tokio::test]
    async fn the_name_search_is_bounded_and_fails_on_errors() {
        use super::super::route;
        use super::super::test_store::{request as req, state as flaky_state};
        use std::time::Duration;
        let (st, store) = flaky_state(100).await;
        let root = st.cfg.storage();
        let post = |slug: &'static str| {
            let st = st.clone();
            async move {
                let h = [("slug", slug), ("content-type", "text/plain")];
                route(&st, req(Method::POST, "/", &h, "x")).await
            }
        };
        *store.fail_exists_of.lock().unwrap() = Some(format!("{root}x"));
        let r = post("x").await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        *store.fail_exists_of.lock().unwrap() = None;
        assert!(!st.store.exists(&format!("{root}x-2")).await.unwrap());
        // Every name taken: a 409 once the tries run out, not a search that never ends.
        *store.occupied.lock().unwrap() = Some(format!("{root}y"));
        let r = tokio::time::timeout(Duration::from_secs(10), post("y"))
            .await
            .expect("the name search did not end");
        assert_eq!(r.status(), StatusCode::CONFLICT);
        *store.occupied.lock().unwrap() = None;
        // The container's lock was given back.
        let r = tokio::time::timeout(Duration::from_secs(5), post("y"))
            .await
            .expect("the container stayed locked");
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    /// Review finding: a POST to the root with Slug `livez` or `readyz` created a resource the
    /// health probes, answered ahead of the resources, hide.
    #[tokio::test]
    async fn health_probe_names_are_reserved_at_the_root() {
        let st = state().await;
        for name in ["livez", "readyz"] {
            let uri = post(&st, name, "text/plain", "x", &[]).await;
            assert_eq!(uri, format!("{BASE}/{name}-2"));
            let r = call(&st, "GET", path_of(&uri), &[], "").await;
            assert_eq!(body_of(r).await, Bytes::from("x"));
        }
        // Below the root the paths are not the probes', so the names are free there.
        let c = post(
            &st,
            "c",
            "text/plain",
            "",
            &[(
                "link",
                "<https://www.w3.org/ns/lws#Container>; rel=\"type\"",
            )],
        )
        .await;
        let r = call(
            &st,
            "POST",
            path_of(&c),
            &[("slug", "livez"), ("content-type", "text/plain")],
            "x",
        )
        .await;
        assert_eq!(hdr(&r, "location"), format!("{c}livez"));
    }

    /// Review finding: the types the content stated and those Link headers declared were kept
    /// as one list, so replacing the content dropped a declared type the old content also stated.
    #[tokio::test]
    async fn replacing_content_keeps_the_declared_types() {
        let st = state().await;
        let (t, u, v) = (
            "https://e.example/T",
            "https://e.example/U",
            "https://e.example/V",
        );
        let uri = post(
            &st,
            "d.ttl",
            "text/turtle",
            &format!("<> a <{t}>, <{u}> ."),
            &[("link", &format!("<{t}>; rel=\"type\""))],
        )
        .await;
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(m.types, vec![t.to_string(), u.to_string()]);
        assert_eq!(m.declared_types, Some(vec![t.to_string()]));
        let r = call(
            &st,
            "PUT",
            path_of(&uri),
            &[("content-type", "text/turtle")],
            &format!("<> a <{v}> ."),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        // The declared type stays; the old content's own goes; the new content's comes.
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(m.types, vec![t.to_string(), v.to_string()]);
        assert_eq!(m.declared_types, Some(vec![t.to_string()]));
        // Content that states none leaves the declared type alone.
        let r = call(
            &st,
            "PUT",
            path_of(&uri),
            &[("content-type", "text/turtle")],
            "",
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            st.resource_meta(&uri).await.unwrap().types,
            vec![t.to_string()]
        );
        // A PUT that declares types of its own replaces the declared ones.
        let r = call(
            &st,
            "PUT",
            path_of(&uri),
            &[
                ("content-type", "text/turtle"),
                ("link", &format!("<{u}>; rel=\"type\"")),
            ],
            &format!("<> a <{v}> ."),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(m.types, vec![u.to_string(), v.to_string()]);
        assert_eq!(m.declared_types, Some(vec![u.to_string()]));
    }

    /// Review finding: a PUT that could not read the old Turtle content took it to state no
    /// types, so the types it did state outlived it. (Only metadata from before declared types
    /// were kept apart needs the old content read.)
    #[tokio::test]
    async fn a_put_that_cannot_read_the_old_content_fails() {
        use super::super::route;
        use super::super::test_store::{request as req, state as flaky_state};
        let (st, store) = flaky_state(100).await;
        let ttl = "<> a <https://e.example/Old> .";
        let r = route(
            &st,
            req(
                Method::POST,
                "/",
                &[("slug", "t.ttl"), ("content-type", "text/turtle")],
                ttl,
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let uri = hdr(&r, "location");
        let p = uri
            .strip_prefix(st.cfg.absolute("").as_str())
            .unwrap()
            .to_string();
        assert!(st
            .resource_meta(&uri)
            .await
            .unwrap()
            .types
            .contains(&"https://e.example/Old".to_string()));
        // Metadata from before declared types were kept apart: the old content says which of
        // the types are its own.
        let mut legacy = st.resource_meta(&uri).await.unwrap();
        legacy.declared_types = None;
        st.put_resource_meta(&uri, &legacy).await.unwrap();
        *store.fail_read_of.lock().unwrap() = Some(uri.clone());
        let r = route(
            &st,
            req(
                Method::PUT,
                &p,
                &[("content-type", "text/turtle")],
                "<> a <https://e.example/New> .",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        *store.fail_read_of.lock().unwrap() = None;
        assert_eq!(st.store.read(&uri).await.unwrap().body, Bytes::from(ttl));
        // Once readable, the old types go with the old content.
        let r = route(
            &st,
            req(
                Method::PUT,
                &p,
                &[("content-type", "text/turtle")],
                "<> a <https://e.example/New> .",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let types = st.resource_meta(&uri).await.unwrap().types;
        assert!(
            !types.contains(&"https://e.example/Old".to_string()),
            "{types:?}"
        );
    }

    /// Review finding: a GET or HEAD of a linkset ignored a failed If-Match and answered 200.
    #[tokio::test]
    async fn linkset_reads_honor_preconditions() {
        let st = state().await;
        let uri = post(&st, "l.txt", "text/plain", "x", &[]).await;
        let p = format!("{}{META_SUFFIX}", path_of(&uri));
        let r = call(&st, "GET", &p, &[], "").await;
        assert_eq!(r.status(), StatusCode::OK);
        let tag = hdr(&r, "etag");
        for m in ["GET", "HEAD"] {
            let r = call(&st, m, &p, &[("if-match", "\"other\"")], "").await;
            assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED, "{m}");
            assert_eq!(
                call(&st, m, &p, &[("if-match", &tag)], "").await.status(),
                StatusCode::OK
            );
            let r = call(&st, m, &p, &[("if-none-match", &tag)], "").await;
            assert_eq!(r.status(), StatusCode::NOT_MODIFIED, "{m}");
        }
    }
}
