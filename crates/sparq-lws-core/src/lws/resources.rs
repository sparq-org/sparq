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
        // A backend failure is a 500, never "absent": absence skips the permission check.
        let exists = match state.store.exists(&uri).await {
            Ok(e) => e,
            Err(e) => return store_error(e),
        };
        if !exists {
            if state.needs_auth(agent) {
                return state.challenge(None);
            }
            // Nothing was authorized, so nothing is served or changed: every method on a missing
            // target is a 404, and answering it here (rather than in the handler) means a resource
            // that appears after this check is never read or written without its permission
            // check.
            return problem(StatusCode::NOT_FOUND, None);
        }
        match state.check(action, &uri, agent).await {
            Ok(true) => {}
            Ok(false) => return state.deny(agent),
            Err(e) => return store_error(e),
        }
    }
    match req.method {
        Method::GET | Method::HEAD => read(state, req, agent, &uri).await,
        Method::POST => create(state, req, agent, &uri).await,
        Method::PUT => update(state, req, agent, &uri).await,
        Method::PATCH => patch(state, req, agent, &uri).await,
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
///
/// The container's lock is taken shared, as a create in it takes it: a touch after each create
/// must not wait for every other create in flight in the container (nor hold up the ones queued
/// behind it), which taking it exclusively would. What changes the container's metadata
/// otherwise takes the lock exclusively, so it never interleaves with a touch; touches among
/// themselves are ordered by a lock of their own (no lock is taken under it).
async fn touch_container<S: Store + 'static>(state: &LwsState<S>, container: &str) {
    let _guard = state.locks.read(container).await;
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
    for m in &all {
        hasher.update(serde_json::to_vec(m).unwrap_or_default());
        hasher.update(b"\n");
    }
    let tag = jose::b64url(&hasher.finalize()[..18]);
    // The listing changed no earlier than any member it lists did, so its Last-Modified is the
    // latest of the container's own and every listed member's: an If-Modified-Since never meets a
    // 304 for a listing whose members moved on since.
    let modified = all
        .iter()
        .filter_map(|m| m["modified"].as_str().and_then(parse_rfc3339))
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
    let parent_guard = state.locks.read(parent).await;
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
        let (state, parent, child, meta) = (
            state.clone(),
            parent.to_string(),
            child.clone(),
            meta.clone(),
        );
        tokio::spawn(async move {
            let _locks = (child_guards, parent_guard);
            state.put_resource_meta(&child, &meta).await?;
            match state
                .store
                .create_in_container(&parent, &child, body, &content_type)
                .await
            {
                Ok(m) => Ok(m),
                Err(e) => {
                    let _ = state.store.delete(&meta_key(&child), None).await;
                    Err(e)
                }
            }
        })
        .await
    };
    let created = match created {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return store_error(e),
        Err(e) => return store_error(ServerError::Storage(format!("the create failed: {e}"))),
    };
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
/// PATCH, DELETE, a linkset PATCH) holds its resource's lock from the precondition check through
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
pub struct IriLocks(
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::RwLock<()>>>>,
);

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
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 1024 {
            map.retain(|_, w| w.strong_count() > 0);
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

/// Write new content for `uri` and, with `meta` = `(new, old)`, its new metadata, such that a
/// failure (or the request being dropped) never leaves content visible under metadata that does
/// not describe it: the types (and the creator) in the metadata are what grants rest on, so new
/// content under old types, or old content under new ones, could reach readers neither state
/// admits.
///
/// - Metadata unchanged: one content write, which lands whole or not at all.
/// - Metadata changed: three writes. The old metadata marked `pending` first (when that fails,
///   nothing changed); then the content; then the new metadata, which clears the mark. When the
///   content write fails the old metadata is put back. Whenever a step after the first fails,
///   rollback included, the `pending` mark stays and the resource fails closed: only its owner
///   and creator may act on it (see [`access::allowed`](super::access::allowed)) until a write
///   completes.
///
/// The writes run in a task of their own that holds the resource's lock (`guard`, handed back
/// when they are done), so a client that goes away mid-way cancels neither the writes nor the
/// rollback, and nobody sees the steps in between.
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
    let Some((new, old)) = changed else {
        let written = state.store.write(uri, body, content_type).await;
        return (written, Some(guard));
    };
    let (state, uri, content_type) = (state.clone(), uri.to_string(), content_type.to_string());
    let task = tokio::spawn(async move {
        let outcome = async {
            let mut closed = old.clone().unwrap_or_default();
            closed.pending = true;
            state.put_resource_meta(&uri, &closed).await?;
            let written = match state.store.write(&uri, body, &content_type).await {
                Ok(m) => m,
                Err(e) => {
                    // A failed rollback leaves the mark: fail closed.
                    let _ = match &old {
                        Some(old) => state.put_resource_meta(&uri, old).await,
                        None => state.store.delete(&meta_key(&uri), None).await,
                    };
                    return Err(e);
                }
            };
            state.put_resource_meta(&uri, &new).await?;
            Ok(written)
        }
        .await;
        (outcome, guard)
    });
    match task.await {
        Ok((outcome, guard)) => (outcome, Some(guard)),
        Err(e) => (
            Err(ServerError::Storage(format!("the write failed: {e}"))),
            None,
        ),
    }
}

async fn update<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let guard = state.locks.lock(uri).await;
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
    // The types the old content stated, so the replacement can drop them.
    let old_content_types = if meta.content_type.starts_with("text/turtle") {
        // Unreadable old content is a failure, not "states no types": otherwise the types it
        // stated would outlive it.
        match state.store.read_at(uri, &meta).await {
            Ok(b) => content_types(uri, &meta.content_type, &b),
            Err(e) => return store_error(e),
        }
    } else {
        Vec::new()
    };
    let set_linkset = prefers_set_linkset(req);
    let old_rmeta = match stored_meta(state, uri).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let mut rmeta = old_rmeta.clone().unwrap_or_default();
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
        Err(e) => return store_error(e),
    };
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
        // A member's listed fields (format, size, modified) changed, so the listing did.
        touch_container(state, &parent).await;
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
    let mut out = target.clone();
    merge_patch_into(&mut out, patch);
    out
}

/// [`merge_patch`] in place: `target` is changed where it stands, so nothing of it is copied
/// (a copy of the rest of the target at every level of the patch would grow with the patch's
/// depth times the target's size). Only the patch's own values are cloned into it.
fn merge_patch_into(target: &mut Value, patch: &Value) {
    let Value::Object(p) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(out) = target else {
        unreachable!("made an object above")
    };
    for (k, v) in p {
        if v.is_null() {
            out.remove(k);
        } else {
            merge_patch_into(out.entry(k.clone()).or_insert(Value::Null), v);
        }
    }
}

/// [`merge_patch`] held to `budget` serialized bytes: the target is cloned once and patched in
/// place, and a result larger than the budget is refused, as a JSON Patch's is.
fn merge_patch_bounded(target: &Value, patch: &Value, budget: usize) -> Result<Value, PatchError> {
    let mut out = target.clone();
    merge_patch_into(&mut out, patch);
    if json_size(&out) > budget {
        return Err(PatchError::TooLarge);
    }
    Ok(out)
}

/// How far a patched document may grow, in serialized bytes: the request body limit. A JSON Patch
/// `copy` doubles what it copies, so without a bound a few dozen operations exhaust memory.
pub const PATCH_BUDGET: usize = 64 * 1024 * 1024;

/// How deeply a patched document may nest, in arrays and objects: the deepest document
/// `serde_json` parses back (its recursion limit). The parser bounds every document a request
/// carries, but JSON Patch builds new ones: each `move` or `copy` may nest an existing value under
/// another, so without this bound a patch could build a document deep enough to overflow the stack
/// when it is serialized, compared, cloned or dropped. (A merge patch needs no check: it places
/// each of its values where it sits in the patch, so the result nests no deeper than the target or
/// the patch, both parsed.)
pub const MAX_JSON_DEPTH: usize = 127;

/// How deeply `v` nests arrays and objects: 0 for a scalar, 1 for `[]` or `{}`. Iterative, so it is
/// safe on a value of any depth.
fn json_depth(v: &Value) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(v, 0usize)];
    while let Some((v, d)) = stack.pop() {
        match v {
            Value::Array(a) => {
                deepest = deepest.max(d + 1);
                stack.extend(a.iter().map(|c| (c, d + 1)));
            }
            Value::Object(m) => {
                deepest = deepest.max(d + 1);
                stack.extend(m.values().map(|c| (c, d + 1)));
            }
            _ => {}
        }
    }
    deepest
}

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
    /// The result would nest deeper than [`MAX_JSON_DEPTH`] (422).
    TooDeep,
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
        // A value placed at `path` sits under one container per pointer segment.
        let fits = |v: &Value| {
            if path.matches('/').count() + json_depth(v) > MAX_JSON_DEPTH {
                return Err(PatchError::TooDeep);
            }
            Ok(())
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
                fits(v)?;
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
                fits(v)?;
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
                fits(&v)?;
                pointer_add(&mut doc, &path, v).ok_or(Failed)?;
            }
            "copy" => {
                let from = str_of(op, "from");
                let source = doc.pointer(&from).ok_or(Failed)?;
                fits(source)?;
                let added = json_size(source);
                let minus = replaced(&doc);
                grow(&mut size, added, minus)?;
                let v = doc.pointer(&from).ok_or(Failed)?.clone();
                pointer_add(&mut doc, &path, v).ok_or(Failed)?;
            }
            "test" => {
                if !json_equal(doc.pointer(&path).ok_or(Failed)?, &op["value"]) {
                    return Err(Failed);
                }
            }
            _ => unreachable!("validated"),
        }
    }
    Ok(doc)
}

/// RFC 6902 section 4.6 equality: numbers are equal when their values are (`1` and `1.0`), strings
/// and literals when they are identical, arrays element by element, objects member by member
/// whatever their order. Integers compare exactly (two distinct large integers never meet through
/// a float), and an integer equals a float only when the float is exactly that integer.
fn json_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => numbers_equal(x, y),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| json_equal(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_equal(v, w)))
        }
        _ => a == b,
    }
}

fn numbers_equal(x: &serde_json::Number, y: &serde_json::Number) -> bool {
    let int = |n: &serde_json::Number| {
        n.as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
    };
    // An integral, finite float within range, as the integer it is exactly.
    let integral =
        |f: f64| (f.is_finite() && f.fract() == 0.0 && f.abs() < 1e38).then_some(f as i128);
    match (int(x), int(y)) {
        (Some(i), Some(j)) => i == j,
        (Some(i), None) => y.as_f64().and_then(integral) == Some(i),
        (None, Some(j)) => x.as_f64().and_then(integral) == Some(j),
        (None, None) => x.as_f64().is_some_and(|f| y.as_f64() == Some(f)),
    }
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
            Patch::Merge(p) => merge_patch_bounded(target, p, PATCH_BUDGET),
            Patch::Json(ops) => json_patch(target, ops, PATCH_BUDGET),
        }
        .map_err(|e| match e {
            PatchError::Malformed(why) => problem(StatusCode::BAD_REQUEST, Some(why)),
            PatchError::Failed => problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("the JSON Patch cannot be applied"),
            ),
            PatchError::TooLarge => problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                Some("the patched document would be too large"),
            ),
            PatchError::TooDeep => problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("the patched document would nest too deeply"),
            ),
        })
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
    if patch.reads_content() {
        return recheck(state, Action::Read, uri, agent).await;
    }
    Ok(())
}

async fn patch<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let guard = state.locks.lock(uri).await;
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
    // The linkset is left alone unless Prefer: set-linkset asks for the Link headers to update it
    // too, partially (update-resource).
    let set_linkset = prefers_set_linkset(req);
    let metas = if set_linkset {
        let old = match stored_meta(state, uri).await {
            Ok(m) => m,
            Err(e) => return store_error(e),
        };
        let (link_types, links) = link_declared(req, uri);
        let mut rmeta = old.clone().unwrap_or_default();
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
        Some((rmeta, old))
    } else {
        None
    };
    let (written, _guard) = write_with_meta(
        state,
        guard,
        uri,
        Bytes::from(serde_json::to_vec(&patched).unwrap_or_default()),
        &ct,
        metas,
    )
    .await;
    let written = match written {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
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
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    // The validators the preconditions are evaluated against (RFC 9110 section 13): a data
    // resource's own; a container's are its listing's, computed the way a read computes them,
    // under the subtree locks, whenever the request is conditional at all.
    let conditional = [
        header::IF_MATCH,
        header::IF_NONE_MATCH,
        header::IF_UNMODIFIED_SINCE,
    ]
    .iter()
    .any(|h| req.headers.contains_key(h));
    let (etag, modified) = if !uri.ends_with('/') {
        (
            Some(quoted(&meta.etag)),
            meta.last_modified.map(|t| to_secs(epoch_ms(t))),
        )
    } else if conditional {
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
        if !listing.status().is_success() {
            return listing;
        }
        let h = |n: header::HeaderName| {
            listing
                .headers()
                .get(n)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        (
            h(header::ETAG),
            parse_http_date(h(header::LAST_MODIFIED).as_deref()),
        )
    } else {
        (None, None)
    };
    if let Precondition::Failed | Precondition::NotModified =
        evaluate(&req.headers, etag.as_deref(), modified, false)
    {
        return problem(StatusCode::PRECONDITION_FAILED, None);
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
    // Who hears of each removal is decided before the resources go, while who may read each can
    // still be decided; the notifications go out only for removals that happened. A recursive
    // delete removes every descendant, and each removal is a Delete of its own.
    let mut notices = Vec::with_capacity(doomed.len());
    for (gone, origin) in doomed.iter().cloned() {
        let event = Event {
            kind: "Delete",
            is_container: gone.ends_with('/'),
            uri: gone,
            relation: origin.map(|p| ("origin", p)),
        };
        notices.push(state.notify.prepare(state, &event).await);
    }
    let (removed, outcome) = remove(state, &doomed).await;
    for pending in notices.into_iter().take(removed) {
        state.notify.send(state, pending);
    }
    drop(guards);
    if removed > 0 {
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
        let removed = if node.ends_with('/') {
            match state
                .store
                .delete_container_if_empty(node, parent.as_deref())
                .await
            {
                Ok(crate::store::DeleteOutcome::NotEmpty) => Err(ServerError::Conflict(
                    "the container gained a member while it was deleted".into(),
                )),
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            }
        } else {
            state.store.delete(node, parent.as_deref()).await
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
    // The resource's lock is taken before the permission check, so the decision holds for what is
    // served or changed: shared for a read, exclusive for a patch (from the precondition through
    // the write).
    let (_shared, _exclusive) = if req.method == Method::PATCH {
        (None, Some(state.locks.lock(uri).await))
    } else {
        (Some(state.locks.read(uri).await), None)
    };
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
    let action = if matches!(req.method, Method::GET | Method::HEAD | Method::OPTIONS) {
        Action::Read
    } else {
        Action::Modify
    };
    if let Err(r) = recheck(state, action, uri, agent).await {
        return r;
    }
    let mut meta = match state.resource_meta(uri).await {
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
            // The linkset is checked whole, as it will be stored: inside the metadata it nests a
            // level deeper than in the patched document, and metadata that does not read back
            // would be lost.
            if super::encode_meta(&meta).is_err() {
                return problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("the linkset nests too deeply to be stored"),
                );
            }
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

    /// Review finding: a merge patch copied the rest of the target at every level it descended,
    /// so a deep patch over a large target allocated its depth times the target's size; and it
    /// had no size bound, unlike JSON Patch.
    #[test]
    fn merge_patch_copies_the_target_once_and_is_bounded() {
        const DEPTH: usize = 120;
        let big = "x".repeat(1 << 20);
        let target = (0..DEPTH).fold(json!({ "big": big }), |v, _| json!({ "a": v }));
        let patch = (0..DEPTH).fold(json!({ "n": 1 }), |v, _| json!({ "a": v }));
        // Patched where it stands: the large value at the bottom is the same allocation after
        // the merge, never a copy (one per level, before).
        let leaf_of = |v: &Value| (0..DEPTH).fold(v, |v, _| &v["a"]).clone();
        let mut doc = target.clone();
        let before = (0..DEPTH).fold(&doc, |v, _| &v["a"])["big"]
            .as_str()
            .unwrap()
            .as_ptr();
        merge_patch_into(&mut doc, &patch);
        let leaf = (0..DEPTH).fold(&doc, |v, _| &v["a"]);
        assert_eq!(leaf["big"].as_str().unwrap().as_ptr(), before);
        assert_eq!(leaf["n"], json!(1));
        // The same result as the definition (RFC 7386), within the budget.
        let out = merge_patch_bounded(&target, &patch, PATCH_BUDGET).unwrap();
        assert_eq!(out, doc);
        assert_eq!(leaf_of(&out)["big"].as_str().map(str::len), Some(1 << 20));
        // The RFC 7386 appendix A cases.
        for (t, p, want) in [
            (json!({"a": "b"}), json!({"a": "c"}), json!({"a": "c"})),
            (
                json!({"a": "b"}),
                json!({"b": "c"}),
                json!({"a": "b", "b": "c"}),
            ),
            (json!({"a": "b"}), json!({"a": null}), json!({})),
            (
                json!({"a": "b", "b": "c"}),
                json!({"a": null}),
                json!({"b": "c"}),
            ),
            (json!({"a": ["b"]}), json!({"a": "c"}), json!({"a": "c"})),
            (json!({"a": "c"}), json!({"a": ["b"]}), json!({"a": ["b"]})),
            (
                json!({"a": {"b": "c"}}),
                json!({"a": {"b": "d", "c": null}}),
                json!({"a": {"b": "d"}}),
            ),
            (
                json!({"a": [{"b": "c"}]}),
                json!({"a": [1]}),
                json!({"a": [1]}),
            ),
            (json!(["a", "b"]), json!(["c", "d"]), json!(["c", "d"])),
            (json!({"a": "b"}), json!(["c"]), json!(["c"])),
            (json!({"a": "foo"}), json!(null), json!(null)),
            (json!({"a": "foo"}), json!("bar"), json!("bar")),
            (
                json!({"e": null}),
                json!({"a": 1}),
                json!({"e": null, "a": 1}),
            ),
            (
                json!([1, 2]),
                json!({"a": "b", "c": null}),
                json!({"a": "b"}),
            ),
            (
                json!({}),
                json!({"a": {"bb": {"ccc": null}}}),
                json!({"a": {"bb": {}}}),
            ),
        ] {
            assert_eq!(merge_patch(&t, &p), want, "{t} + {p}");
        }
        // Past the budget: refused, as a JSON Patch would be.
        assert_eq!(
            merge_patch_bounded(&target, &patch, 1 << 20),
            Err(PatchError::TooLarge)
        );
    }

    #[tokio::test]
    async fn an_oversized_merge_patch_result_is_a_413() {
        {
            let st = state().await;
            let uri = post(&st, "m.json", "application/json", "{\"a\": 1}", &[]).await;
            let grow = json!({ "b": "y".repeat(PATCH_BUDGET) }).to_string();
            let r = call(
                &st,
                "PATCH",
                path_of(&uri),
                &[("content-type", MERGE_PATCH)],
                &grow,
            )
            .await;
            assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
            let body = body_of(call(&st, "GET", path_of(&uri), &[], "").await).await;
            assert_eq!(body, Bytes::from("{\"a\": 1}"));
        }
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
        let m = st.resource_meta(&uri).await.unwrap();
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
        let m = st.resource_meta(&uri).await.unwrap();
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
                .unwrap()
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
        let m = st.resource_meta(&uri).await.unwrap();
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
            ("PATCH", MERGE_PATCH, "{\"pin\": null}"),
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

    /// `doc` with its `/d` wrapped in `n` more objects, by add and move alone.
    fn nesting_ops(n: usize) -> Value {
        let mut ops = Vec::new();
        for _ in 0..n {
            ops.push(json!({"op": "add", "path": "/w", "value": {}}));
            ops.push(json!({"op": "move", "from": "/d", "path": "/w/d"}));
            ops.push(json!({"op": "move", "from": "/w", "path": "/d"}));
        }
        Value::Array(ops)
    }

    /// Review finding: add and move can nest a document without bound (each op is small, so the
    /// size budget never trips), deep enough to overflow the stack when it is serialized, cloned
    /// or dropped.
    #[tokio::test]
    async fn json_patch_bounds_the_nesting_depth() {
        let doc = json!({"d": 0});
        // The root object and 126 wrappers: the deepest document serde_json parses back.
        let ok = json_patch(&doc, &nesting_ops(MAX_JSON_DEPTH - 1), PATCH_BUDGET).unwrap();
        assert_eq!(json_depth(&ok), MAX_JSON_DEPTH);
        let text = serde_json::to_string(&ok).unwrap();
        assert!(serde_json::from_str::<Value>(&text).is_ok());
        assert_eq!(
            json_patch(&doc, &nesting_ops(MAX_JSON_DEPTH), PATCH_BUDGET),
            Err(PatchError::TooDeep)
        );
        // Far past the bound: refused, not a stack overflow.
        assert_eq!(
            json_patch(&doc, &nesting_ops(100_000), PATCH_BUDGET),
            Err(PatchError::TooDeep)
        );
        // A copy into itself doubles the depth; add places a deep value under a deep path.
        let deep = (0..100).fold(json!(0), |v, _| json!({ "d": v }));
        let into = format!("/a{}", "/d".repeat(50));
        let copy = json!([{"op": "copy", "from": "/a", "path": into}]);
        assert_eq!(
            json_patch(&json!({ "a": deep.clone() }), &copy, PATCH_BUDGET),
            Err(PatchError::TooDeep)
        );
        let path = format!("/a{}", "/d".repeat(99));
        // 1 + 99 segments + a 30-deep value: over the bound, though each part alone parses.
        let value = (0..30).fold(json!(1), |v, _| json!([v]));
        let add = json!([{"op": "add", "path": path, "value": value}]);
        assert_eq!(
            json_patch(&json!({ "a": deep }), &add, PATCH_BUDGET),
            Err(PatchError::TooDeep)
        );
        // Over HTTP: 422, and the resource is untouched.
        let st = state().await;
        let uri = post(&st, "deep.json", "application/json", "{\"d\": 0}", &[]).await;
        let r = call(
            &st,
            "PATCH",
            path_of(&uri),
            &[("content-type", JSON_PATCH)],
            &nesting_ops(MAX_JSON_DEPTH).to_string(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_of(call(&st, "GET", path_of(&uri), &[], "").await).await;
        assert_eq!(body, Bytes::from("{\"d\": 0}"));
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
        let cases: [Case<'_>; 5] = [
            ("GET", file.as_str(), &[], ""),
            (
                "PUT",
                file.as_str(),
                &[("content-type", "application/json")],
                "{\"bob\": 1}",
            ),
            (
                "PATCH",
                file.as_str(),
                &[("content-type", MERGE_PATCH)],
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

    /// Review finding: Delete notifications were queued before the removal ran, so a failed
    /// backend delete (a 500) still told subscribers the resource was gone, and a recursive delete
    /// that stopped part way announced descendants it never touched.
    #[tokio::test]
    async fn deletes_are_announced_only_once_they_happen() {
        use super::super::route;
        use super::super::test_store::{request as req, state as flaky_state};
        use std::sync::atomic::Ordering;
        let (st, store) = flaky_state(100).await;
        let base = st.cfg.absolute("");
        let local = |u: &str| u.strip_prefix(base.as_str()).unwrap().to_string();
        let send = |m: Method, p: &str, h: &[(&str, &str)], b: &str| {
            let st = st.clone();
            let r = req(m, p, h, b);
            async move { route(&st, r).await }
        };
        let created = |r: Response| {
            assert_eq!(r.status(), StatusCode::CREATED);
            hdr(&r, "location")
        };
        let f = created(
            send(
                Method::POST,
                "/",
                &[("slug", "f"), ("content-type", "text/plain")],
                "x",
            )
            .await,
        );
        let d = created(
            send(
                Method::POST,
                "/",
                &[("slug", "d"), ("link", CONTAINER_LINK)],
                "",
            )
            .await,
        );
        let a = created(
            send(
                Method::POST,
                &local(&d),
                &[("slug", "a"), ("content-type", "text/plain")],
                "x",
            )
            .await,
        );
        let b = created(
            send(
                Method::POST,
                &local(&d),
                &[("slug", "b"), ("content-type", "text/plain")],
                "x",
            )
            .await,
        );
        let (inbox, got) = inbox().await;
        let body =
            json!({"type": "WebhookSubscription", "topic": [st.cfg.storage()], "inbox": inbox});
        let r = send(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            &[("content-type", LWS_JSON)],
            &body.to_string(),
        )
        .await;
        assert!(r.status().is_success());
        let deletes = |got: &Arc<Mutex<Vec<Value>>>| {
            let got = got.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let mut gone: Vec<String> = activities(&got, 0)
                    .await
                    .into_iter()
                    .filter(|(t, _)| t == "Delete")
                    .map(|(_, u)| u)
                    .collect();
                gone.sort();
                gone
            }
        };
        // The backend refuses: a 500, the resource stays, and nobody hears of a delete.
        store.fail_delete.store(true, Ordering::SeqCst);
        let r = send(Method::DELETE, &local(&f), &[], "").await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(st.store.exists(&f).await.unwrap());
        assert!(deletes(&got).await.is_empty());
        store.fail_delete.store(false, Ordering::SeqCst);
        // A recursive delete that fails on `b`: only what was removed is announced.
        *store.fail_delete_of.lock().unwrap() = Some(b.clone());
        let r = send(Method::DELETE, &local(&d), &[("depth", "infinity")], "").await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let mut really_gone = Vec::new();
        for u in [&d, &a, &b] {
            if !st.store.exists(u).await.unwrap() {
                really_gone.push(u.to_string());
            }
        }
        really_gone.sort();
        assert!(st.store.exists(&b).await.unwrap() && st.store.exists(&d).await.unwrap());
        assert_eq!(deletes(&got).await, really_gone);
        // Once the delete succeeds, it is announced.
        *store.fail_delete_of.lock().unwrap() = None;
        let r = send(Method::DELETE, &local(&f), &[], "").await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(deletes(&got).await.contains(&f));
    }

    /// Review finding: PUT and PATCH committed the content and discarded a failure to write the
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
        let attempts = [
            (Method::PUT, "application/json", "{\"v\": 1}"),
            (Method::PATCH, MERGE_PATCH, "{\"v\": 1}"),
        ];
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

    /// Review finding: `test` compared with `Value` equality, so `1` and `1.0` differed; RFC 6902
    /// section 4.6 compares numbers by value.
    #[test]
    fn json_patch_test_compares_numbers_by_value() {
        let doc = json!({"n": 1, "f": 1.5, "big": u64::MAX, "a": [1, {"x": 2}], "o": {"p": 10, "q": [0]}});
        let test = |path: &str, value: Value| {
            json_patch(
                &doc,
                &json!([{"op": "test", "path": path, "value": value}]),
                PATCH_BUDGET,
            )
            .is_ok()
        };
        assert!(test("/n", json!(1.0)));
        assert!(test("/f", json!(1.5)));
        assert!(test("/a", json!([1.0, {"x": 2.0}])));
        assert!(test("/o", json!({"q": [0.0], "p": 1e1})));
        assert!(test("/big", json!(u64::MAX)));
        assert!(!test("/n", json!(1.5)));
        assert!(!test("/n", json!("1")));
        assert!(!test("/big", json!(u64::MAX - 1)));
        // Distinct large integers never meet through a float.
        let near = json!({"n": 9_007_199_254_740_993_i64});
        let t = |v: Value| {
            json_patch(
                &near,
                &json!([{"op": "test", "path": "/n", "value": v}]),
                PATCH_BUDGET,
            )
            .is_ok()
        };
        assert!(!t(json!(9_007_199_254_740_992_i64)));
        assert!(!t(json!(9_007_199_254_740_992.0)));
        assert!(t(json!(9_007_199_254_740_993_i64)));
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

    /// Review finding: the metadata was written before the content and a failed rollback was
    /// ignored. With one write left in a bounded store, a `Prefer: set-linkset` adding a type a
    /// public grant covers landed its metadata, the content write and the rollback both failed,
    /// and the old private content became public. Now the resource fails closed instead.
    #[tokio::test]
    async fn a_write_that_fails_part_way_fails_closed() {
        use super::super::test_store::{request as req, FlakyStore};
        use super::super::FOAF_AGENT;
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let store = FlakyStore::new();
        let st = LwsState::new(store.clone(), cfg).await.expect("state");
        let owner = agent("https://owner.example/#me");
        let stranger = agent("https://stranger.example/#me");
        let public = "https://e.example/Public";
        // Everyone may read what has the public type.
        let grant = json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": ["AccessGrant"],
            "storage": st.cfg.storage(),
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": FOAF_AGENT,
                "constraint": [{"leftOperand": "type", "operator": "eq", "rightOperand": public}]}],
        });
        let r = super::super::access::handle(
            &st,
            &req(
                Method::POST,
                GRANTS_PATH,
                &[("content-type", LWS_JSON)],
                &grant.to_string(),
            ),
            &owner,
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let base = st.cfg.absolute("");
        let make = |slug: &'static str| {
            let st = st.clone();
            let owner = owner.clone();
            async move {
                let h = [("slug", slug), ("content-type", "text/plain")];
                let r = create(
                    &st,
                    &req(Method::POST, "/", &h, "private"),
                    &owner,
                    &st.cfg.storage(),
                )
                .await;
                assert_eq!(r.status(), StatusCode::CREATED);
                hdr(&r, "location")
            }
        };
        let link = format!("<{public}>; rel=\"type\"");
        let put = |uri: &str| {
            req(
                Method::PUT,
                uri.strip_prefix(base.as_str()).unwrap(),
                &[
                    ("content-type", "text/plain"),
                    ("prefer", "set-linkset"),
                    ("link", &link),
                ],
                "now public",
            )
        };
        let get = |uri: &str| {
            req(
                Method::GET,
                uri.strip_prefix(base.as_str()).unwrap(),
                &[],
                "",
            )
        };
        // The control: once the write lands, the stranger reads the new content.
        let open = make("open.txt").await;
        assert_eq!(
            handle(&st, &get(&open), &stranger).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            handle(&st, &put(&open), &owner).await.status(),
            StatusCode::NO_CONTENT
        );
        let r = handle(&st, &get(&open), &stranger).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body_of(r).await, Bytes::from("now public"));
        // One write left: the first metadata write lands, the content write and the rollback fail.
        let secret = make("secret.txt").await;
        *store.write_budget.lock().unwrap() = Some(1);
        let r = handle(&st, &put(&secret), &owner).await;
        assert!(r.status().is_server_error(), "{}", r.status());
        *store.write_budget.lock().unwrap() = None;
        assert_eq!(
            st.store.read(&secret).await.unwrap().body,
            Bytes::from("private")
        );
        // Whatever the metadata now says, the old content is not public.
        let r = handle(&st, &get(&secret), &stranger).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        // The owner still may, and a completed write clears the mark.
        assert_eq!(
            handle(&st, &get(&secret), &owner).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            handle(&st, &put(&secret), &owner).await.status(),
            StatusCode::NO_CONTENT
        );
        assert!(!st.resource_meta(&secret).await.unwrap().pending);
        assert_eq!(
            handle(&st, &get(&secret), &stranger).await.status(),
            StatusCode::OK
        );
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

    /// Review finding: a linkset nested to the parser's limit validated, but stored inside the
    /// metadata it nested one level deeper, so the metadata never read back and every later read
    /// took the defaults (no creator, no types, no links).
    #[tokio::test]
    async fn a_linkset_too_deep_to_store_is_refused() {
        let st = state().await;
        let uri = post(&st, "deep.txt", "text/plain", "x", &[]).await;
        let mut meta = st.resource_meta(&uri).await.unwrap();
        meta.creator = Some("https://bob.example/#me".into());
        meta.types = vec!["https://e.example/T".into()];
        st.put_resource_meta(&uri, &meta).await.unwrap();
        let p = format!("{}{META_SUFFIX}", path_of(&uri));
        // The deepest patch the request parser accepts.
        let body = |depth: usize| {
            format!(
                r#"{{"linkset":[{{"anchor":"{uri}","https://e.example/rel":[{{"href":"x","ext":{}1{}}}]}}]}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        let depth = (1..200)
            .take_while(|d| serde_json::from_str::<Value>(&body(*d)).is_ok())
            .last()
            .unwrap();
        let r = call(
            &st,
            "PATCH",
            &p,
            &[("content-type", MERGE_PATCH)],
            &body(depth),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let kept = st.resource_meta(&uri).await.unwrap();
        assert_eq!(kept.creator, meta.creator);
        assert_eq!(kept.types, meta.types);
        // One that fits as stored is taken, and reads back.
        let r = call(&st, "PATCH", &p, &[("content-type", MERGE_PATCH)], &body(8)).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(st.resource_meta(&uri).await.unwrap().creator, meta.creator);
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

    /// Review finding: a PUT that could not read the old Turtle content took it to state no
    /// types, so the types it did state outlived it.
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
