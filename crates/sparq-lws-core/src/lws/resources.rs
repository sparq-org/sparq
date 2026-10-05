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

use super::access::{format_rfc3339, Action};
use super::notify::Event;
use super::{
    add_link, is_uri, jose, json_response, meta_key, method_not_allowed, parse_links, problem, set,
    Agent, LwsRequest, LwsState, ResourceMeta, AS_CONTEXT, CID_CONTEXT, GRANTS_PATH, JSON,
    JSON_PATCH, LD_JSON, LINKSET_JSON, LWS_CID, LWS_CONTEXT, LWS_JSON, LWS_NS, MERGE_PATCH,
    META_SUFFIX, REQUESTS_PATH, SUBSCRIPTIONS_PATH, TYPE_INDEX_PATH, TYPE_SEARCH_PATH,
};
use crate::error::ServerError;
use crate::store::Store;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACCEPT_PATCH: &str = "application/merge-patch+json, application/json-patch+json";
const LINKSET_ALLOW: &str = "GET, HEAD, PATCH";

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
    let path = req.path.as_str();
    if let Some(stem) = path.strip_suffix(META_SUFFIX) {
        let uri = state.cfg.absolute(stem);
        return linkset(state, req, agent, &uri).await;
    }
    let uri = state.cfg.absolute(path);
    let is_root = path == "/";
    let accept = req.header(header::ACCEPT).unwrap_or_default();
    // The storage description is public: a client refused with a 401 finds the services through
    // it, and a webhook receiver finds the delivery key in it. The root container's listing,
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
        let exists = state.store.exists(&uri).await.unwrap_or(false);
        if !exists {
            if state.needs_auth(agent) {
                return state.challenge(None);
            }
        } else if !state.allowed(action, &uri, agent).await {
            return state.deny(agent);
        }
    }
    match req.method {
        Method::GET | Method::HEAD => read(state, req, &uri).await,
        Method::POST => create(state, req, agent, &uri).await,
        Method::PUT => update(state, req, &uri).await,
        Method::PATCH => patch(state, req, agent, &uri).await,
        Method::DELETE => delete(state, req, &uri).await,
        Method::OPTIONS => options(state, &uri).await,
        _ => method_not_allowed(&allow_for(&uri, is_root)),
    }
}

fn allow_for(uri: &str, is_root: bool) -> String {
    if is_root {
        "GET, HEAD, OPTIONS, POST".into()
    } else if uri.ends_with('/') {
        "GET, HEAD, OPTIONS, POST, DELETE".into()
    } else {
        "GET, HEAD, OPTIONS, PUT, PATCH, DELETE".into()
    }
}

async fn options<S: Store + 'static>(state: &LwsState<S>, uri: &str) -> Response {
    if !state.store.exists(uri).await.unwrap_or(false) {
        return problem(StatusCode::NOT_FOUND, None);
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set(
        resp.headers_mut(),
        header::ALLOW,
        &allow_for(uri, uri == state.cfg.storage()),
    );
    if !uri.ends_with('/') {
        set(
            resp.headers_mut(),
            header::HeaderName::from_static("accept-patch"),
            ACCEPT_PATCH,
        );
    }
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

/// Record that a container's membership or a member changed.
async fn touch_container<S: Store + 'static>(state: &LwsState<S>, container: &str) {
    let _guard = state.locks.lock(container).await;
    let mut meta = state.resource_meta(container).await;
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
fn negotiate(accept: Option<&str>, offered: &[&str]) -> Option<String> {
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

async fn read<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
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
    let meta = match state.store.meta(uri).await {
        Ok(Some(m)) => m,
        Ok(None) => return problem(StatusCode::NOT_FOUND, None),
        Err(e) => return store_error(e),
    };
    if uri.ends_with('/') {
        return read_container(state, req, uri, &meta).await;
    }
    let types = state.resource_meta(uri).await.types;
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
    let range = req
        .header(header::RANGE)
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

/// A container's members, sorted, with what a listing shows about each.
async fn members<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<Vec<Value>, ServerError> {
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
            let cmeta = state.resource_meta(&child).await;
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
            let size = match state.store.read(&child).await {
                Ok(r) => r.body.len(),
                Err(_) => 0,
            };
            item.insert("type".into(), Value::String("DataResource".into()));
            // "format: The media type of the resource ... MUST be present for DataResources"; size
            // and modified SHOULD be.
            item.insert("format".into(), Value::String(meta.content_type.clone()));
            item.insert("size".into(), json!(size));
            let modified = meta.last_modified.map(epoch_ms).unwrap_or_default();
            item.insert(
                "modified".into(),
                Value::String(format_rfc3339(to_secs(modified) as i64)),
            );
            item.insert("etag".into(), Value::String(quoted(&meta.etag)));
        }
        out.push(Value::Object(item));
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
    let cmeta = state.resource_meta(uri).await;
    // The entity tag covers the whole listing (each member's own tag included), so it changes
    // whenever a member comes, goes or changes.
    let mut hasher = Sha256::new();
    hasher.update(meta.etag.as_bytes());
    hasher.update(cmeta.version.as_deref().unwrap_or_default().as_bytes());
    for m in &all {
        hasher.update(m["id"].as_str().unwrap_or_default().as_bytes());
        hasher.update(m["etag"].as_str().unwrap_or_default().as_bytes());
    }
    let tag = jose::b64url(&hasher.finalize()[..18]);
    let modified = to_secs(
        cmeta
            .modified_ms
            .or(meta.last_modified.map(epoch_ms))
            .unwrap_or_default(),
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
    let items: Vec<Value> = all
        .iter()
        .skip((page - 1) * page_size)
        .take(page_size)
        .map(|m| {
            let mut m = m.clone();
            if let Some(o) = m.as_object_mut() {
                o.remove("etag");
            }
            m
        })
        .collect();
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
/// naming the storage and its services, and publishing the key notifications are signed with.
pub fn storage_description<S: Store>(state: &LwsState<S>) -> Value {
    let cfg = &state.cfg;
    let storage = cfg.storage();
    let service = |frag: &str, ty: &str, endpoint: String| json!({"id": format!("{storage}#{frag}"), "type": ty, "serviceEndpoint": endpoint});
    let mut grants = service(
        "access-grants",
        "AccessGrantService",
        cfg.absolute(GRANTS_PATH),
    );
    grants["conformsTo"] = json!([format!("{LWS_NS}AccessProfile")]);
    let mut requests = service(
        "access-requests",
        "AccessRequestService",
        cfg.absolute(REQUESTS_PATH),
    );
    requests["conformsTo"] = json!([format!("{LWS_NS}AccessProfile")]);
    let mut notifications = service(
        "notifications",
        "NotificationService",
        cfg.absolute(SUBSCRIPTIONS_PATH),
    );
    notifications["subscriptionType"] = json!(["WebhookSubscription"]);
    let key_id = format!("{storage}#{}", cfg.notify_key.kid());
    json!({
        "@context": [CID_CONTEXT, LWS_CONTEXT],
        "id": storage,
        "type": "Storage",
        "service": [
            service("storage-root", "StorageRoot", storage.clone()),
            service("authorization-server", "AuthorizationServer", cfg.issuer().to_string()),
            grants,
            requests,
            notifications,
            service("type-index", "TypeIndexService", cfg.absolute(TYPE_INDEX_PATH)),
            service("type-search", "TypeSearchService", cfg.absolute(TYPE_SEARCH_PATH)),
        ],
        "verificationMethod": [{
            "id": key_id,
            "type": "JsonWebKey",
            "controller": storage,
            "publicKeyJwk": cfg.notify_key.public_jwk(),
        }],
        "authentication": [key_id],
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
    if clean.to_ascii_lowercase().ends_with(META_SUFFIX) {
        clean.truncate(clean.len() - META_SUFFIX.len());
        clean.push_str("-meta");
    }
    clean.truncate(120);
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

/// The types and user-managed links a request declared in its Link headers.
fn link_declared(req: &LwsRequest, uri: &str) -> (Vec<String>, Links) {
    let mut types = Vec::new();
    let mut links = Links::new();
    for (target, params) in parse_links(&req.header_all(header::LINK)) {
        let Some(rel) = params.get("rel") else {
            continue;
        };
        for r in rel.split_whitespace() {
            let key = rel_key(r);
            if key == "type" {
                if !target.starts_with(LWS_NS) && is_uri(&target) && !types.contains(&target) {
                    types.push(target.clone());
                }
            } else if !STRUCTURAL_RELATIONS.contains(&key.as_str()) {
                let resolved = resolve_against(uri, &target);
                let entry = links.entry(key).or_default();
                if !entry.contains(&resolved) {
                    entry.push(resolved);
                }
            }
        }
    }
    (types, links)
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
fn content_types(uri: &str, content_type: &str, body: &[u8]) -> Vec<String> {
    let mut types = Vec::new();
    if !content_type.starts_with("text/turtle") {
        return types;
    }
    if let Ok(parser) = oxttl::TurtleParser::new().with_base_iri(uri) {
        for t in parser.for_slice(body).flatten() {
            if let (oxrdf::NamedOrBlankNode::NamedNode(s), oxrdf::Term::NamedNode(o)) =
                (&t.subject, &t.object)
            {
                if s.as_str() == uri
                    && t.predicate.as_str() == RDF_TYPE
                    && !types.contains(&o.as_str().to_string())
                {
                    types.push(o.as_str().to_string());
                }
            }
        }
    }
    types
}

/// The types and user-managed links a client declared on a create.
fn declared(req: &LwsRequest, uri: &str, content_type: &str, body: &[u8]) -> (Vec<String>, Links) {
    let (mut types, links) = link_declared(req, uri);
    for t in content_types(uri, content_type, body) {
        if !types.contains(&t) {
            types.push(t);
        }
    }
    (types, links)
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
    let mut name = base_name.clone();
    let mut n = 1;
    loop {
        let a = format!("{parent}{name}");
        let b = format!("{parent}{name}/");
        let taken = state.store.exists(&a).await.unwrap_or(true)
            || state.store.exists(&b).await.unwrap_or(true);
        if !taken {
            break;
        }
        n += 1;
        name = if n > 50 {
            format!("{base_name}-{}", jose::random_id())
        } else {
            format!("{base_name}-{n}")
        };
    }
    let child = if is_container {
        format!("{parent}{name}/")
    } else {
        format!("{parent}{name}")
    };
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
    let created = match state
        .store
        .create_in_container(parent, &child, body.clone(), &content_type)
        .await
    {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let (types, links) = if is_container {
        (Vec::new(), Default::default())
    } else {
        declared(req, &child, &content_type, &body)
    };
    let meta = ResourceMeta {
        creator: agent.subject.clone(),
        linkset: initial_linkset(&child, &links),
        types,
        links,
        modified_ms: is_container.then(now_ms),
        ..Default::default()
    };
    if let Err(e) = state.put_resource_meta(&child, &meta).await {
        return store_error(e);
    }
    touch_container(state, parent).await;
    state
        .notify
        .announce(
            state,
            Event {
                kind: "Create",
                uri: child.clone(),
                is_container,
                relation: Some(("target", parent.to_string())),
            },
        )
        .await;
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
/// PATCH, DELETE, a linkset PATCH) holds its resource's lock from the precondition check through
/// the write, and an If-Match can never pass against a state another writer is replacing. Metadata
/// read-modify-writes of a container (membership touches) take the container's lock too.
///
/// Limit: the locks live in this process. Several server processes over one store do not see each
/// other's locks; that deployment needs a conditional write in the store itself.
#[derive(Default)]
pub struct IriLocks(
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
);

impl IriLocks {
    /// Wait for and take the lock of `iri`. Locks are taken child before parent, never the other
    /// way, so two writers cannot deadlock.
    pub async fn lock(&self, iri: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = {
            let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if map.len() > 1024 {
                map.retain(|_, w| w.strong_count() > 0);
            }
            match map.get(iri).and_then(std::sync::Weak::upgrade) {
                Some(m) => m,
                None => {
                    let m = std::sync::Arc::new(tokio::sync::Mutex::new(()));
                    map.insert(iri.to_string(), std::sync::Arc::downgrade(&m));
                    m
                }
            }
        };
        mutex.lock_owned().await
    }
}

async fn update<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
    let _guard = state.locks.lock(uri).await;
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
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
    // The types the old content stated, so the replacement can drop them.
    let old_content_types = if meta.content_type.starts_with("text/turtle") {
        match state.store.read_at(uri, &meta).await {
            Ok(b) => content_types(uri, &meta.content_type, &b),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let written = match state
        .store
        .write(uri, req.body.clone(), &content_type)
        .await
    {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let set_linkset = prefers_set_linkset(req);
    let mut rmeta = state.resource_meta(uri).await;
    // "LWS servers MUST handle PUT and PATCH requests on resource URIs as modifications to the
    // resource content only, with no default impact on the associated linkset resource." The
    // types the content states are derived from the content, so they follow it; the types and
    // links Link headers declare change only with Prefer: set-linkset, which replaces them.
    let mut types: Vec<String> = if set_linkset {
        let (link_types, links) = link_declared(req, uri);
        rmeta.linkset = initial_linkset(uri, &links);
        rmeta.links = links;
        rmeta.linkset_etag = None;
        link_types
    } else {
        rmeta
            .types
            .iter()
            .filter(|t| !old_content_types.contains(t))
            .cloned()
            .collect()
    };
    for t in content_types(uri, &content_type, &req.body) {
        if !types.contains(&t) {
            types.push(t);
        }
    }
    rmeta.types = types;
    let _ = state.put_resource_meta(uri, &rmeta).await;
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

/// After a resource or its metadata changed: its container changes too, and subscribers hear of it
/// ("Update — an existing resource's content or metadata was modified").
async fn changed<S: Store + 'static>(state: &LwsState<S>, uri: &str) {
    if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
        let _guard = state.locks.lock(&parent).await;
        let mut meta = state.resource_meta(&parent).await;
        meta.version = Some(jose::random_id());
        let _ = state.put_resource_meta(&parent, &meta).await;
    }
    state
        .notify
        .announce(
            state,
            Event {
                kind: "Update",
                uri: uri.to_string(),
                is_container: uri.ends_with('/'),
                relation: None,
            },
        )
        .await;
}

// ---- patch ----

/// RFC 7386: a non-object patch replaces the target; otherwise members merge recursively and a
/// null member removes the name.
pub fn merge_patch(target: &Value, patch: &Value) -> Value {
    let Value::Object(p) = patch else {
        return patch.clone();
    };
    let mut out = match target {
        Value::Object(t) => t.clone(),
        _ => Map::new(),
    };
    for (k, v) in p {
        if v.is_null() {
            out.remove(k);
        } else {
            let merged = merge_patch(out.get(k).unwrap_or(&Value::Null), v);
            out.insert(k.clone(), merged);
        }
    }
    Value::Object(out)
}

/// How far a patched document may grow, in serialized bytes: the request body limit. A JSON Patch
/// `copy` doubles what it copies, so without a bound a few dozen operations exhaust memory.
pub const PATCH_BUDGET: usize = 64 * 1024 * 1024;

/// Why a JSON Patch was not applied.
#[derive(Debug, PartialEq)]
pub enum PatchError {
    /// The patch document is not an RFC 6902 patch: not an array, an unknown `op`, or a member an
    /// operation requires missing or of the wrong type (400).
    Malformed(&'static str),
    /// A well-formed operation cannot be applied to the target: a path that does not exist, or a
    /// failed `test` (RFC 5789 section 2.2).
    Failed,
    /// The result would outgrow [`PATCH_BUDGET`].
    TooLarge,
}

/// Check that `ops` is an RFC 6902 patch document, without applying it.
pub fn validate_json_patch(ops: &Value) -> Result<&[Value], PatchError> {
    let ops = ops.as_array().ok_or(PatchError::Malformed(
        "a JSON Patch is an array of operations",
    ))?;
    let pointer = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .is_some_and(|p| p.is_empty() || p.starts_with('/'))
    };
    for op in ops {
        let kind = op
            .get("op")
            .and_then(Value::as_str)
            .ok_or(PatchError::Malformed("every operation needs an op"))?;
        if !pointer(op.get("path")) {
            return Err(PatchError::Malformed(
                "every operation needs a JSON Pointer path",
            ));
        }
        match kind {
            "add" | "replace" | "test" if op.get("value").is_none() => {
                return Err(PatchError::Malformed("add, replace and test need a value"));
            }
            "move" | "copy" if !pointer(op.get("from")) => {
                return Err(PatchError::Malformed(
                    "move and copy need a JSON Pointer from",
                ));
            }
            "add" | "remove" | "replace" | "move" | "copy" | "test" => {}
            _ => return Err(PatchError::Malformed("unknown JSON Patch operation")),
        }
    }
    Ok(ops)
}

/// Whether a JSON Patch observes the target's content: `test` compares a value and `copy` / `move`
/// read one, so applying it reveals what the patcher may not be allowed to read.
pub fn json_patch_reads(ops: &Value) -> bool {
    ops.as_array().is_some_and(|ops| {
        ops.iter().any(|op| {
            matches!(
                op.get("op").and_then(Value::as_str),
                Some("test" | "copy" | "move")
            )
        })
    })
}

/// The serialized size of `v`, in bytes.
fn json_size(v: &Value) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut c = Count(0);
    let _ = serde_json::to_writer(&mut c, v);
    c.0
}

/// RFC 6902 JSON Patch, applied whole or not at all, within `budget` serialized bytes.
pub fn json_patch(target: &Value, ops: &Value, budget: usize) -> Result<Value, PatchError> {
    use PatchError::Failed;
    let ops = validate_json_patch(ops)?;
    let mut doc = target.clone();
    // The document's size, kept as an upper bound: every operation that adds checks the budget
    // before it clones or inserts anything.
    let mut size = json_size(&doc);
    let str_of = |op: &Value, k: &str| op[k].as_str().unwrap_or_default().to_string();
    for op in ops {
        let path = str_of(op, "path");
        // What an add at `path` would overwrite: an existing object member is replaced.
        let replaced = |doc: &Value| -> usize {
            let Some((parent, _)) = split_pointer(&path) else {
                return 0;
            };
            if path.is_empty() {
                return json_size(doc);
            }
            match (doc.pointer(&parent), doc.pointer(&path)) {
                (Some(Value::Object(_)), Some(old)) => json_size(old),
                _ => 0,
            }
        };
        let grow = |size: &mut usize, by: usize, minus: usize| {
            let next = size.saturating_sub(minus).saturating_add(by);
            if next > budget {
                return Err(PatchError::TooLarge);
            }
            *size = next;
            Ok(())
        };
        match op["op"].as_str().unwrap_or_default() {
            "add" => {
                let v = &op["value"];
                let minus = replaced(&doc);
                grow(&mut size, json_size(v), minus)?;
                pointer_add(&mut doc, &path, v.clone()).ok_or(Failed)?;
            }
            "remove" => {
                let old = pointer_remove(&mut doc, &path).ok_or(Failed)?;
                size = size.saturating_sub(json_size(&old));
            }
            "replace" => {
                let v = &op["value"];
                let old = doc.pointer(&path).ok_or(Failed)?;
                let minus = json_size(old);
                grow(&mut size, json_size(v), minus)?;
                pointer_remove(&mut doc, &path).ok_or(Failed)?;
                pointer_add(&mut doc, &path, v.clone()).ok_or(Failed)?;
            }
            "move" => {
                let from = str_of(op, "from");
                if path.starts_with(&format!("{from}/")) {
                    return Err(Failed);
                }
                let v = pointer_remove(&mut doc, &from).ok_or(Failed)?;
                pointer_add(&mut doc, &path, v).ok_or(Failed)?;
            }
            "copy" => {
                let from = str_of(op, "from");
                let source = doc.pointer(&from).ok_or(Failed)?;
                let added = json_size(source);
                let minus = replaced(&doc);
                grow(&mut size, added, minus)?;
                let v = doc.pointer(&from).ok_or(Failed)?.clone();
                pointer_add(&mut doc, &path, v).ok_or(Failed)?;
            }
            "test" => {
                if doc.pointer(&path).ok_or(Failed)? != &op["value"] {
                    return Err(Failed);
                }
            }
            _ => unreachable!("validated"),
        }
    }
    Ok(doc)
}

fn split_pointer(path: &str) -> Option<(String, String)> {
    if path.is_empty() {
        return Some((String::new(), String::new()));
    }
    let cut = path.rfind('/')?;
    let last = path[cut + 1..].replace("~1", "/").replace("~0", "~");
    Some((path[..cut].to_string(), last))
}

fn pointer_add(doc: &mut Value, path: &str, value: Value) -> Option<()> {
    if path.is_empty() {
        *doc = value;
        return Some(());
    }
    let (parent, key) = split_pointer(path)?;
    match doc.pointer_mut(&parent)? {
        Value::Object(m) => {
            m.insert(key, value);
        }
        Value::Array(a) => {
            if key == "-" {
                a.push(value);
            } else {
                let i: usize = key.parse().ok()?;
                if i > a.len() {
                    return None;
                }
                a.insert(i, value);
            }
        }
        _ => return None,
    }
    Some(())
}

fn pointer_remove(doc: &mut Value, path: &str) -> Option<Value> {
    if path.is_empty() {
        return Some(std::mem::replace(doc, Value::Null));
    }
    let (parent, key) = split_pointer(path)?;
    match doc.pointer_mut(&parent)? {
        Value::Object(m) => m.remove(&key),
        Value::Array(a) => {
            let i: usize = key.parse().ok()?;
            (i < a.len()).then(|| a.remove(i))
        }
        _ => None,
    }
}

/// A patch document a request carries.
enum Patch {
    /// RFC 7386 JSON Merge Patch.
    Merge(Value),
    /// RFC 6902 JSON Patch, already checked to be well-formed.
    Json(Value),
}

impl Patch {
    /// Parse the request body by its Content-Type. `Err` carries the response: 415 for another
    /// format, 400 for a body that is not JSON or not a well-formed patch.
    fn parse(req: &LwsRequest) -> Result<Self, Response> {
        let ct = req.content_type().unwrap_or_default();
        if ct != MERGE_PATCH && ct != JSON_PATCH {
            let mut r = problem(StatusCode::UNSUPPORTED_MEDIA_TYPE, None);
            set(
                r.headers_mut(),
                header::HeaderName::from_static("accept-patch"),
                ACCEPT_PATCH,
            );
            return Err(r);
        }
        let Ok(patch) = serde_json::from_slice::<Value>(&req.body) else {
            return Err(problem(
                StatusCode::BAD_REQUEST,
                Some("the patch is not JSON"),
            ));
        };
        if ct == MERGE_PATCH {
            return Ok(Patch::Merge(patch));
        }
        if let Err(PatchError::Malformed(why)) = validate_json_patch(&patch) {
            return Err(problem(StatusCode::BAD_REQUEST, Some(why)));
        }
        Ok(Patch::Json(patch))
    }

    /// Whether applying the patch observes the target's content (see [`json_patch_reads`]).
    fn reads_content(&self) -> bool {
        matches!(self, Patch::Json(ops) if json_patch_reads(ops))
    }

    /// Apply the patch to `target`. `Err` carries the response.
    fn apply(&self, target: &Value) -> Result<Value, Response> {
        match self {
            Patch::Merge(p) => Ok(merge_patch(target, p)),
            Patch::Json(ops) => json_patch(target, ops, PATCH_BUDGET).map_err(|e| match e {
                PatchError::Malformed(why) => problem(StatusCode::BAD_REQUEST, Some(why)),
                PatchError::Failed => problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("the JSON Patch cannot be applied"),
                ),
                PatchError::TooLarge => problem(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Some("the patched document would be too large"),
                ),
            }),
        }
    }
}

/// A patch that observes content (see [`Patch::reads_content`]) needs Read as well as Modify:
/// otherwise 403, or the challenge for an anonymous caller.
async fn patch_read_check<S: Store + 'static>(
    state: &LwsState<S>,
    patch: &Patch,
    uri: &str,
    agent: &Agent,
) -> Result<(), Response> {
    if patch.reads_content() && !state.allowed(Action::Read, uri, agent).await {
        return Err(state.deny(agent));
    }
    Ok(())
}

async fn patch<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let _guard = state.locks.lock(uri).await;
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
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
    let patch = match Patch::parse(req) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = patch_read_check(state, &patch, uri, agent).await {
        return r;
    }
    let body = match state.store.read_at(uri, &meta).await {
        Ok(b) => b,
        Err(e) => return store_error(e),
    };
    let target = if body.is_empty() {
        Value::Object(Map::new())
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => v,
            Err(_) => {
                // The stored representation is not JSON, so neither patch format applies to it.
                let mut r = problem(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    Some("the resource is not JSON"),
                );
                set(
                    r.headers_mut(),
                    header::HeaderName::from_static("accept-patch"),
                    ACCEPT_PATCH,
                );
                return r;
            }
        }
    };
    let patched = match patch.apply(&target) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let ct = if meta.content_type.contains("json") {
        meta.content_type.clone()
    } else {
        JSON.to_string()
    };
    let written = match state
        .store
        .write(
            uri,
            Bytes::from(serde_json::to_vec(&patched).unwrap_or_default()),
            &ct,
        )
        .await
    {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    // The linkset is left alone unless Prefer: set-linkset asks for the Link headers to update it
    // too, partially (update-resource).
    let set_linkset = prefers_set_linkset(req);
    if set_linkset {
        let (link_types, links) = link_declared(req, uri);
        let mut rmeta = state.resource_meta(uri).await;
        for t in link_types {
            if !rmeta.types.contains(&t) {
                rmeta.types.push(t);
            }
        }
        let mut user = rmeta
            .linkset
            .clone()
            .or_else(|| initial_linkset(uri, &rmeta.links))
            .map(|d| user_linkset(&d, uri))
            .unwrap_or_else(|| json!({"linkset": []}));
        add_links(&mut user, uri, &links);
        rmeta.links = links_of(&user, uri);
        rmeta.linkset = Some(user);
        rmeta.linkset_etag = None;
        let _ = state.put_resource_meta(uri, &rmeta).await;
    }
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

// ---- delete ----

async fn delete<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
    let storage = state.cfg.storage();
    let _guard = state.locks.lock(uri).await;
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if uri == storage {
        return method_not_allowed(&allow_for(uri, true));
    }
    let etag = if uri.ends_with('/') {
        None
    } else {
        Some(quoted(&meta.etag))
    };
    if req.headers.contains_key(header::IF_MATCH) && uri.ends_with('/') {
        // A container's tag is its listing's; compute it the way a read does.
        let listing = read_container(
            state,
            &LwsRequest {
                method: Method::GET,
                path: String::new(),
                query: None,
                headers: HeaderMap::new(),
                body: Bytes::new(),
            },
            uri,
            &meta,
        )
        .await;
        let tag = listing
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if let Precondition::Failed | Precondition::NotModified =
            evaluate(&req.headers, tag.as_deref(), None, false)
        {
            return problem(StatusCode::PRECONDITION_FAILED, None);
        }
    } else if let Precondition::Failed | Precondition::NotModified = evaluate(
        &req.headers,
        etag.as_deref(),
        meta.last_modified.map(|t| to_secs(epoch_ms(t))),
        false,
    ) {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    if uri.ends_with('/') {
        let has_members = match state.store.list_children(uri).await {
            Ok(c) => !c.is_empty(),
            Err(e) => return store_error(e),
        };
        let infinity = req
            .header("depth")
            .is_some_and(|d| d.trim().eq_ignore_ascii_case("infinity"));
        if has_members && !infinity {
            return problem(StatusCode::CONFLICT, Some("the container is not empty; send Depth: infinity to delete it and everything in it"));
        }
    }
    let parent = parent_of(uri, &storage);
    // Announced before the resources go, while who may read each can still be decided. A recursive
    // delete removes every descendant, and each removal is a Delete of its own.
    let doomed = match subtree(state, uri, parent.clone()).await {
        Ok(d) => d,
        Err(e) => return store_error(e),
    };
    for (gone, origin) in doomed {
        state
            .notify
            .announce(
                state,
                Event {
                    kind: "Delete",
                    is_container: gone.ends_with('/'),
                    uri: gone,
                    relation: origin.map(|p| ("origin", p)),
                },
            )
            .await;
    }
    if let Err(e) = remove(state, uri, parent.as_deref()).await {
        return store_error(e);
    }
    if let Some(p) = parent {
        touch_container(state, &p).await;
    }
    problem(StatusCode::NO_CONTENT, None)
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

/// Remove a resource and, for a container, everything in it, with their metadata.
fn remove<'a, S: Store + 'static>(
    state: &'a LwsState<S>,
    uri: &'a str,
    parent: Option<&'a str>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ServerError>> + Send + 'a>> {
    Box::pin(async move {
        if uri.ends_with('/') {
            for child in state.store.list_children(uri).await? {
                remove(state, child.as_str(), Some(uri)).await?;
            }
            let _ = state.store.delete(&meta_key(uri), None).await;
            if matches!(
                state.store.delete_container_if_empty(uri, parent).await?,
                crate::store::DeleteOutcome::NotEmpty
            ) {
                return Err(ServerError::Conflict(
                    "the container gained a member while it was deleted".into(),
                ));
            }
        } else {
            state.store.delete(uri, parent).await?;
            let _ = state.store.delete(&meta_key(uri), None).await;
        }
        Ok(())
    })
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
/// entries about `uri`. A client cannot write those relations; whatever a patch puts there is
/// ignored and the server's own values stand.
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

/// The user-managed links a linkset document holds about `uri`, by relation, resolved: what the
/// type index matches relations against, kept equal to the document.
fn links_of(doc: &Value, uri: &str) -> Links {
    let mut links = Links::new();
    for entry in doc
        .get("linkset")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| anchored_at(e, uri))
    {
        for (rel, targets) in entry.as_object().into_iter().flatten() {
            let key = rel_key(rel);
            if rel == "anchor" || STRUCTURAL_RELATIONS.contains(&key.as_str()) {
                continue;
            }
            for href in targets
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.get("href").and_then(Value::as_str))
            {
                let resolved = resolve_against(uri, href);
                let entry = links.entry(key.clone()).or_default();
                if !entry.contains(&resolved) {
                    entry.push(resolved);
                }
            }
        }
    }
    links
}

/// Add `links` to the entry about `uri` in a user linkset document, creating it when absent.
fn add_links(doc: &mut Value, uri: &str, links: &Links) {
    if links.is_empty() {
        return;
    }
    if !doc.get("linkset").is_some_and(Value::is_array) {
        *doc = json!({"linkset": []});
    }
    let entries = doc["linkset"].as_array_mut().expect("an array");
    let i = match entries.iter().position(|e| anchored_at(e, uri)) {
        Some(i) => i,
        None => {
            entries.push(json!({"anchor": uri}));
            entries.len() - 1
        }
    };
    let Some(entry) = entries[i].as_object_mut() else {
        return;
    };
    for (rel, hrefs) in links {
        let targets = entry.entry(rel.clone()).or_insert_with(|| json!([]));
        if !targets.is_array() {
            *targets = json!([]);
        }
        let arr = targets.as_array_mut().expect("an array");
        for h in hrefs {
            if !arr
                .iter()
                .any(|t| t.get("href").and_then(Value::as_str) == Some(h))
            {
                arr.push(json!({"href": h}));
            }
        }
    }
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

/// A resource's linkset (RFC 9264): GET, HEAD and PATCH (JSON Merge Patch or JSON Patch). PUT is
/// not offered, so it is 405 with the methods that are.
async fn linkset<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let exists = state.store.exists(uri).await.unwrap_or(false);
    if !exists {
        return if state.needs_auth(agent) {
            state.challenge(None)
        } else {
            problem(StatusCode::NOT_FOUND, None)
        };
    }
    let action = if matches!(req.method, Method::GET | Method::HEAD | Method::OPTIONS) {
        Action::Read
    } else {
        Action::Modify
    };
    if !state.allowed(action, uri, agent).await {
        return state.deny(agent);
    }
    // A patch holds the resource's lock from the precondition through the write.
    let _guard = if req.method == Method::PATCH {
        Some(state.locks.lock(uri).await)
    } else {
        None
    };
    let mut meta = state.resource_meta(uri).await;
    let document = match linkset_document(state, uri, &meta).await {
        Ok(d) => d,
        Err(e) => return store_error(e),
    };
    let etag = linkset_etag(&document);
    let mut resp = match req.method {
        Method::GET | Method::HEAD => {
            if let Precondition::NotModified = evaluate(&req.headers, Some(&etag), None, true) {
                let mut r = StatusCode::NOT_MODIFIED.into_response();
                set(r.headers_mut(), header::ETAG, &etag);
                r
            } else {
                let mut r = json_response(StatusCode::OK, LINKSET_JSON, &document);
                set(r.headers_mut(), header::ETAG, &etag);
                r
            }
        }
        Method::OPTIONS => StatusCode::NO_CONTENT.into_response(),
        Method::PATCH => {
            if let Precondition::Failed | Precondition::NotModified =
                evaluate(&req.headers, Some(&etag), None, false)
            {
                return problem(StatusCode::PRECONDITION_FAILED, None);
            }
            let patch = match Patch::parse(req) {
                Ok(p) => p,
                Err(r) => return r,
            };
            if let Err(r) = patch_read_check(state, &patch, uri, agent).await {
                return r;
            }
            let patched = match patch.apply(&document) {
                Ok(v) => v,
                Err(r) => return r,
            };
            if !valid_linkset(&patched) {
                return problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("the result is not a linkset document"),
                );
            }
            // Only the user-managed part is kept; the links the type index matches are taken from
            // it, so the two never drift apart.
            let user = user_linkset(&patched, uri);
            meta.links = links_of(&user, uri);
            meta.linkset = Some(user);
            meta.linkset_etag = None;
            if let Err(e) = state.put_resource_meta(uri, &meta).await {
                return store_error(e);
            }
            let new_etag = match linkset_document(state, uri, &meta).await {
                Ok(d) => linkset_etag(&d),
                Err(e) => return store_error(e),
            };
            changed(state, uri).await;
            let mut r = StatusCode::NO_CONTENT.into_response();
            set(r.headers_mut(), header::ETAG, &new_etag);
            r
        }
        _ => method_not_allowed(LINKSET_ALLOW),
    };
    set(resp.headers_mut(), header::ALLOW, LINKSET_ALLOW);
    set(
        resp.headers_mut(),
        header::HeaderName::from_static("accept-patch"),
        ACCEPT_PATCH,
    );
    add_link(resp.headers_mut(), uri, "anchor", None);
    resp
}

/// An RFC 9264 linkset document: an object whose `linkset` is an array of objects with an `anchor`
/// and arrays of target objects with an `href`.
fn valid_linkset(doc: &Value) -> bool {
    let Some(entries) = doc.get("linkset").and_then(Value::as_array) else {
        return false;
    };
    entries.iter().all(|e| {
        let Some(o) = e.as_object() else { return false };
        o.iter().all(|(k, v)| {
            if k == "anchor" {
                v.is_string()
            } else {
                v.as_array().is_some_and(|ts| {
                    ts.iter()
                        .all(|t| t.get("href").is_some_and(Value::is_string))
                })
            }
        })
    })
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
    fn patches() {
        let t = json!({"title": "a", "keep": 1, "drop": true});
        assert_eq!(
            merge_patch(&t, &json!({"added": 42, "drop": null})),
            json!({"title": "a", "keep": 1, "added": 42})
        );
        let p = json!([{"op": "add", "path": "/added", "value": 42}, {"op": "remove", "path": "/drop"},
                       {"op": "replace", "path": "/title", "value": "b"}, {"op": "test", "path": "/keep", "value": 1}]);
        assert_eq!(
            json_patch(&t, &p, PATCH_BUDGET),
            Ok(json!({"title": "b", "keep": 1, "added": 42}))
        );
        assert_eq!(
            json_patch(
                &t,
                &json!([{"op": "test", "path": "/keep", "value": 2}]),
                PATCH_BUDGET
            ),
            Err(PatchError::Failed)
        );
        assert_eq!(
            json_patch(
                &json!({"a": [1, 2]}),
                &json!([{"op": "add", "path": "/a/-", "value": 3}]),
                PATCH_BUDGET
            ),
            Ok(json!({"a": [1, 2, 3]}))
        );
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
    use std::sync::{Arc, Mutex};

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

    #[tokio::test]
    async fn linkset_patch_ignores_server_managed_relations_and_keeps_links_in_step() {
        let st = state().await;
        let uri = post(
            &st,
            "p.txt",
            "text/plain",
            "x",
            &[("link", "<https://ex.org/lic>; rel=\"license\"")],
        )
        .await;
        let meta = format!("{}{META_SUFFIX}", path_of(&uri));
        // A merge patch replaces the whole array: a forged parent and type, and a new link only.
        let patch = json!({"linkset": [{"anchor": uri, "up": [{"href": "https://forged/"}],
            "type": [{"href": "https://forged/T"}], "describedby": [{"href": "https://ex.org/schema"}]}]});
        let r = call(
            &st,
            "PATCH",
            &meta,
            &[("content-type", MERGE_PATCH)],
            &patch.to_string(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let doc = json_of(call(&st, "GET", &meta, &[], "").await).await;
        let e = &doc["linkset"][0];
        assert_eq!(e["up"], json!([{"href": format!("{BASE}/")}]));
        assert_eq!(
            e["type"],
            json!([{"href": "https://www.w3.org/ns/lws#DataResource"}])
        );
        assert_eq!(e["describedby"][0]["href"], "https://ex.org/schema");
        assert!(e.get("license").is_none());
        // The links the type index matches follow the document: the license is gone from both.
        let m = st.resource_meta(&uri).await;
        assert!(!m.links.contains_key("license"), "{:?}", m.links);
        assert_eq!(
            m.links["describedby"],
            vec!["https://ex.org/schema".to_string()]
        );
        assert!(m.types.is_empty());
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
        let links = links_of(&doc, "http://h/a");
        assert_eq!(links.keys().collect::<Vec<_>>(), vec!["license"]);
        assert_eq!(links["license"], vec!["http://h/l".to_string()]);
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
        // Link headers without Prefer: set-linkset change nothing; the content's own type follows it.
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
        let m = st.resource_meta(&uri).await;
        assert_eq!(
            m.types,
            vec![
                "https://ex.org/Declared".to_string(),
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
        let m = st.resource_meta(&uri).await;
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
    async fn patch_leaves_the_linkset_alone_without_prefer() {
        let st = state().await;
        let uri = post(
            &st,
            "j.json",
            JSON,
            "{}",
            &[("link", "<https://ex.org/lic>; rel=\"license\"")],
        )
        .await;
        let p = path_of(&uri);
        let ops = r#"[{"op":"add","path":"/a","value":1}]"#;
        let r = call(
            &st,
            "PATCH",
            p,
            &[
                ("content-type", JSON_PATCH),
                ("link", "<https://ex.org/x>; rel=\"author\""),
            ],
            ops,
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            st.resource_meta(&uri)
                .await
                .links
                .keys()
                .collect::<Vec<_>>(),
            vec!["license"]
        );
        let ops = r#"[{"op":"add","path":"/b","value":2}]"#;
        let r = call(
            &st,
            "PATCH",
            p,
            &[
                ("content-type", JSON_PATCH),
                ("prefer", "set-linkset"),
                ("link", "<https://ex.org/x>; rel=\"author\""),
            ],
            ops,
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let m = st.resource_meta(&uri).await;
        assert_eq!(
            m.links.keys().collect::<Vec<_>>(),
            vec!["author", "license"]
        );
    }

    #[tokio::test]
    async fn malformed_json_patch_is_400_and_a_failing_one_422() {
        let st = state().await;
        let uri = post(&st, "k.json", JSON, r#"{"a":1}"#, &[]).await;
        let p = path_of(&uri);
        for bad in [
            r#"{"op":"add"}"#,
            r#"[{"op":"frobnicate","path":"/a"}]"#,
            r#"[{"op":"add","path":"/b"}]"#,
            r#"[{"op":"copy","path":"/b"}]"#,
            r#"[{"path":"/a"}]"#,
            r#"[{"op":"remove","path":"a"}]"#,
        ] {
            let r = call(&st, "PATCH", p, &[("content-type", JSON_PATCH)], bad).await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad}");
        }
        let r = call(
            &st,
            "PATCH",
            p,
            &[("content-type", JSON_PATCH)],
            r#"[{"op":"remove","path":"/nope"}]"#,
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let r = call(
            &st,
            "PATCH",
            p,
            &[("content-type", JSON_PATCH)],
            r#"[{"op":"test","path":"/a","value":2}]"#,
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        // On a linkset too.
        let r = call(
            &st,
            "PATCH",
            &format!("{p}{META_SUFFIX}"),
            &[("content-type", JSON_PATCH)],
            r#"{"op":"add"}"#,
        )
        .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn json_patch_copy_is_held_to_a_budget() {
        let mut ops = Vec::new();
        for _ in 0..40 {
            ops.push(json!({"op": "copy", "from": "/a", "path": "/a/-"}));
        }
        let r = json_patch(&json!({"a": [1, 2, 3, 4]}), &Value::Array(ops), 4096);
        assert_eq!(r, Err(PatchError::TooLarge));
        // Within the budget, copies apply.
        let ok = json_patch(
            &json!({"a": [1]}),
            &json!([{"op": "copy", "from": "/a", "path": "/b"}]),
            4096,
        );
        assert_eq!(ok, Ok(json!({"a": [1], "b": [1]})));
        // Replacing a member does not count the old value twice.
        let mut ops = Vec::new();
        for _ in 0..100 {
            ops.push(json!({"op": "copy", "from": "/a", "path": "/b"}));
        }
        let big = json!({"a": "x".repeat(100)});
        assert!(json_patch(&big, &Value::Array(ops), 1024).is_ok());
    }

    #[tokio::test]
    async fn content_reading_patches_need_read() {
        let st = state_with(false).await;
        let anonymous = Agent::anonymous();
        let reads = Patch::Json(json!([{"op": "test", "path": "/a", "value": 1}]));
        let blind = Patch::Json(json!([{"op": "add", "path": "/a", "value": 1}]));
        assert!(reads.reads_content());
        assert!(!blind.reads_content());
        assert!(!Patch::Merge(json!({"a": 1})).reads_content());
        for op in ["copy", "move"] {
            assert!(json_patch_reads(
                &json!([{"op": op, "from": "/a", "path": "/b"}])
            ));
        }
        let uri = format!("{BASE}/");
        let denied = patch_read_check(&st, &reads, &uri, &anonymous)
            .await
            .unwrap_err();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let someone = Agent {
            subject: Some("https://someone.example/#me".into()),
            client: None,
        };
        let denied = patch_read_check(&st, &reads, &uri, &someone)
            .await
            .unwrap_err();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert!(patch_read_check(&st, &blind, &uri, &anonymous)
            .await
            .is_ok());
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

    /// A webhook inbox on loopback that records what it is sent.
    async fn inbox() -> (String, Arc<Mutex<Vec<Value>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let app = axum::Router::new().route(
            "/inbox",
            axum::routing::post(move |body: Bytes| {
                let sink = sink.clone();
                async move {
                    if let Ok(v) = serde_json::from_slice::<Value>(&body) {
                        sink.lock().unwrap().push(v);
                    }
                    StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (format!("http://{addr}/inbox"), got)
    }

    async fn subscribe(st: &LwsState<Mem>, inbox: &str) {
        let body =
            json!({"type": "WebhookSubscription", "topic": [format!("{BASE}/")], "inbox": inbox});
        let r = call(
            st,
            "POST",
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body.to_string(),
        )
        .await;
        assert!(r.status().is_success(), "{}", r.status());
    }

    /// The (type, object id) of each activity delivered so far, once `n` have arrived.
    async fn activities(got: &Arc<Mutex<Vec<Value>>>, n: usize) -> Vec<(String, String)> {
        for _ in 0..200 {
            if got.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        got.lock()
            .unwrap()
            .iter()
            .flat_map(|e| match &e["activity"] {
                Value::Array(a) => a.clone(),
                one => vec![one.clone()],
            })
            .map(|a| {
                (
                    a["type"][0].as_str().unwrap_or_default().to_string(),
                    a["object"]["id"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn linkset_patch_announces_an_update() {
        let st = state().await;
        let uri = post(&st, "n.txt", "text/plain", "x", &[]).await;
        let (inbox, got) = inbox().await;
        subscribe(&st, &inbox).await;
        let patch =
            json!({"linkset": [{"anchor": uri, "license": [{"href": "https://ex.org/l"}]}]});
        let r = call(
            &st,
            "PATCH",
            &format!("{}{META_SUFFIX}", path_of(&uri)),
            &[("content-type", MERGE_PATCH)],
            &patch.to_string(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(activities(&got, 1).await, vec![("Update".to_string(), uri)]);
    }

    #[tokio::test]
    async fn recursive_delete_announces_every_descendant() {
        let st = state().await;
        let container = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
        let c = hdr(
            &call(&st, "POST", "/", &[("slug", "d"), ("link", container)], "").await,
            "location",
        );
        let inner = hdr(
            &call(
                &st,
                "POST",
                path_of(&c),
                &[("slug", "e"), ("link", container)],
                "",
            )
            .await,
            "location",
        );
        let leaf = hdr(
            &call(
                &st,
                "POST",
                path_of(&inner),
                &[("slug", "f.txt"), ("content-type", "text/plain")],
                "x",
            )
            .await,
            "location",
        );
        let (inbox, got) = inbox().await;
        subscribe(&st, &inbox).await;
        let r = call(&st, "DELETE", path_of(&c), &[("depth", "infinity")], "").await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let mut seen = activities(&got, 3).await;
        seen.sort();
        let mut want: Vec<(String, String)> = [&c, &inner, &leaf]
            .iter()
            .map(|u| ("Delete".to_string(), u.to_string()))
            .collect();
        want.sort();
        assert_eq!(seen, want);
    }
}
