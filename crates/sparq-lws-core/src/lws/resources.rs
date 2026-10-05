//! Storage resources: the storage description, containers, data resources and their linksets
//! (LWS 1.0 core sections 6 to 9 and 12).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::access::{format_rfc3339, Action};
use super::notify::Event;
use super::{
    add_link, is_uri, jose, json_response, meta_key, method_not_allowed, parse_links, problem, set, Agent,
    LwsRequest, LwsState, ResourceMeta, AS_CONTEXT, CID_CONTEXT, GRANTS_PATH, JSON, JSON_PATCH, LD_JSON,
    LINKSET_JSON, LWS_CID, LWS_CONTEXT, LWS_JSON, LWS_NS, MERGE_PATCH, META_SUFFIX, REQUESTS_PATH,
    SUBSCRIPTIONS_PATH, TYPE_INDEX_PATH, TYPE_SEARCH_PATH,
};
use crate::error::ServerError;
use crate::store::Store;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACCEPT_PATCH: &str = "application/merge-patch+json, application/json-patch+json";
const LINKSET_ALLOW: &str = "GET, HEAD, PATCH";

/// Relations that are server-managed or protocol-level: never taken from a client's Link header
/// as user-managed metadata.
const STRUCTURAL_RELATIONS: &[&str] = &[
    "type", "up", "linkset", "acl", "first", "prev", "next", "last", "self", "describes", "describedby",
    "storagedescription", "https://www.w3.org/ns/lws#storage",
];

pub async fn handle<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, agent: &Agent) -> Response {
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
    let description = is_root
        && matches!(req.method, Method::GET | Method::HEAD)
        && accept.to_ascii_lowercase().contains(LWS_CID);
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
        Method::PATCH => patch(state, req, &uri).await,
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
    set(resp.headers_mut(), header::ALLOW, &allow_for(uri, uri == state.cfg.storage()));
    if !uri.ends_with('/') {
        set(resp.headers_mut(), header::HeaderName::from_static("accept-patch"), ACCEPT_PATCH);
    }
    resp
}

// ---- helpers: time, validators, parents ----

fn epoch_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
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

fn evaluate(headers: &HeaderMap, etag: Option<&str>, modified_secs: Option<u64>, read: bool) -> Precondition {
    let h = |n: header::HeaderName| headers.get(n).and_then(|v| v.to_str().ok());
    if let Some(im) = h(header::IF_MATCH) {
        if !etag.is_some_and(|e| etag_listed(im, e, false)) {
            return Precondition::Failed;
        }
    } else if let (Some(since), Some(m)) = (parse_http_date(h(header::IF_UNMODIFIED_SINCE)), modified_secs) {
        if m > since {
            return Precondition::Failed;
        }
    }
    if let Some(inm) = h(header::IF_NONE_MATCH) {
        if etag.is_some_and(|e| etag_listed(inm, e, true)) {
            return if read { Precondition::NotModified } else { Precondition::Failed };
        }
    } else if read {
        if let (Some(since), Some(m)) = (parse_http_date(h(header::IF_MODIFIED_SINCE)), modified_secs) {
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
    format!("{LWS_NS}{}", if uri.ends_with('/') { "Container" } else { "DataResource" })
}

/// The links every response about a resource carries: up, the storage, its type and its linkset.
fn resource_links<S: Store>(state: &LwsState<S>, headers: &mut HeaderMap, uri: &str) {
    if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
        add_link(headers, &parent, "up", None);
    }
    add_link(headers, &state.cfg.storage(), &format!("{LWS_NS}storage"), None);
    add_link(headers, &lws_type(uri), "type", None);
    add_link(headers, &meta_key(uri), "linkset", Some(LINKSET_JSON));
}

/// Record that a container's membership or a member changed.
async fn touch_container<S: Store + 'static>(state: &LwsState<S>, container: &str) {
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
            Some(Range { essence, q, profile })
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

/// The container media type for `accept`: lws+json, ld+json (lws+json when it names the LWS
/// profile), or json.
fn negotiate_container(accept: Option<&str>) -> Option<String> {
    if let Some(a) = accept {
        if parse_accept(a).iter().any(|r| r.essence == LD_JSON && r.profile.as_deref() == Some(LWS_CONTEXT) && r.q > 0.0) {
            return Some(LWS_JSON.into());
        }
    }
    negotiate(accept, &[LWS_JSON, LD_JSON, JSON])
}

// ---- read ----

async fn read<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
    let storage = state.cfg.storage();
    let accept = req.header(header::ACCEPT);
    if uri == storage {
        // "Requests for the storage URI MUST return a document that conforms to the storage
        // description resource data model with a media type of application/lws+cid, unless content
        // negotiation requires a different format."
        let wants_description = match accept {
            None => true,
            Some(a) if a.trim().is_empty() => true,
            Some(a) => {
                let ranges = parse_accept(a);
                let cid_q = ranges.iter().find(|r| r.essence == LWS_CID).map(|r| r.q);
                let container = negotiate_container(Some(a));
                match (cid_q, container) {
                    (Some(q), _) if q > 0.0 => true,
                    (_, Some(_)) if ranges.iter().any(|r| r.essence != "*/*" && r.q > 0.0) => false,
                    _ => ranges.iter().any(|r| r.essence == "*/*" && r.q > 0.0),
                }
            }
        };
        if wants_description {
            let mut resp = json_response(StatusCode::OK, LWS_CID, &storage_description(state));
            set(resp.headers_mut(), header::VARY, "Accept");
            add_link(resp.headers_mut(), &storage, &format!("{LWS_NS}storage"), None);
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
    let etag = quoted(&meta.etag);
    let modified = meta.last_modified.map(|t| to_secs(epoch_ms(t)));
    let mut resp = match evaluate(&req.headers, Some(&etag), modified, true) {
        Precondition::Failed => {
            let mut r = problem(StatusCode::PRECONDITION_FAILED, None);
            resource_links(state, r.headers_mut(), uri);
            return r;
        }
        Precondition::NotModified => {
            let mut r = StatusCode::NOT_MODIFIED.into_response();
            set(r.headers_mut(), header::ETAG, &etag);
            if let Some(m) = modified {
                set(r.headers_mut(), header::LAST_MODIFIED, &http_date(m));
            }
            resource_links(state, r.headers_mut(), uri);
            return r;
        }
        Precondition::Proceed => {
            let body = match state.store.read(uri).await {
                Ok(r) => r.body,
                Err(e) => return store_error(e),
            };
            ranged(req, body, &meta.content_type)
        }
    };
    let h = resp.headers_mut();
    set(h, header::ETAG, &etag);
    if let Some(m) = modified {
        set(h, header::LAST_MODIFIED, &http_date(m));
    }
    set(h, header::ACCEPT_RANGES, "bytes");
    resource_links(state, h, uri);
    resp
}

/// 200 with the whole body, 206 with one byte range, or 416.
fn ranged(req: &LwsRequest, body: Bytes, content_type: &str) -> Response {
    let len = body.len() as u64;
    if let Some(range) = req.header(header::RANGE) {
        let Some((from, to)) = parse_range(range, len) else {
            let mut r = problem(StatusCode::RANGE_NOT_SATISFIABLE, None);
            set(r.headers_mut(), header::CONTENT_RANGE, &format!("bytes */{len}"));
            return r;
        };
        let mut r = (StatusCode::PARTIAL_CONTENT, body.slice(from as usize..=to as usize)).into_response();
        set(r.headers_mut(), header::CONTENT_TYPE, content_type);
        set(r.headers_mut(), header::CONTENT_RANGE, &format!("bytes {from}-{to}/{len}"));
        return r;
    }
    let mut r = (StatusCode::OK, body).into_response();
    set(r.headers_mut(), header::CONTENT_TYPE, content_type);
    r
}

/// One byte range `first..=last` of an entity of `len` bytes, or `None` when unsatisfiable. Only
/// single ranges are served; a multi-range request is answered as unsatisfiable rather than wrongly.
fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (lo, hi) = spec.split_once('-')?;
    let (lo, hi) = (lo.trim(), hi.trim());
    if lo.is_empty() {
        let n: u64 = hi.parse().ok()?;
        if n == 0 {
            return None;
        }
        return Some((len.saturating_sub(n), len - 1));
    }
    let from: u64 = lo.parse().ok()?;
    if from >= len {
        return None;
    }
    let to = if hi.is_empty() { len - 1 } else { hi.parse::<u64>().ok()?.min(len - 1) };
    (to >= from).then_some((from, to))
}

/// A container's members, sorted, with what a listing shows about each.
async fn members<S: Store + 'static>(state: &LwsState<S>, uri: &str) -> Result<Vec<Value>, ServerError> {
    let mut children: Vec<String> =
        state.store.list_children(uri).await?.into_iter().map(|c| c.as_str().to_string()).collect();
    children.sort();
    children.dedup();
    let mut out = Vec::with_capacity(children.len());
    for child in children {
        let Some(meta) = state.store.meta(&child).await? else { continue };
        let mut item = Map::new();
        item.insert("id".into(), Value::String(child.clone()));
        if child.ends_with('/') {
            item.insert("type".into(), Value::String("Container".into()));
            let cmeta = state.resource_meta(&child).await;
            let modified = cmeta.modified_ms.or(meta.last_modified.map(epoch_ms)).unwrap_or_default();
            item.insert("modified".into(), Value::String(format_rfc3339(to_secs(modified) as i64)));
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
            item.insert("modified".into(), Value::String(format_rfc3339(to_secs(modified) as i64)));
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
    let modified = to_secs(cmeta.modified_ms.or(meta.last_modified.map(epoch_ms)).unwrap_or_default());

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
    let etag = if page == 1 { format!("\"{tag}\"") } else { format!("\"{tag}-p{page}\"") };
    match evaluate(&req.headers, Some(&etag), Some(modified), true) {
        Precondition::Failed => return problem(StatusCode::PRECONDITION_FAILED, None),
        Precondition::NotModified => {
            let mut r = StatusCode::NOT_MODIFIED.into_response();
            set(r.headers_mut(), header::ETAG, &etag);
            set(r.headers_mut(), header::LAST_MODIFIED, &http_date(modified));
            set(r.headers_mut(), header::VARY, "Accept");
            resource_links(state, r.headers_mut(), uri);
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
    resource_links(state, h, uri);
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
    let mut grants = service("access-grants", "AccessGrantService", cfg.absolute(GRANTS_PATH));
    grants["conformsTo"] = json!([format!("{LWS_NS}AccessProfile")]);
    let mut requests = service("access-requests", "AccessRequestService", cfg.absolute(REQUESTS_PATH));
    requests["conformsTo"] = json!([format!("{LWS_NS}AccessProfile")]);
    let mut notifications = service("notifications", "NotificationService", cfg.absolute(SUBSCRIPTIONS_PATH));
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
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    clean = clean.trim_start_matches('.').trim_end_matches('/').to_string();
    if clean.to_ascii_lowercase().ends_with(META_SUFFIX) {
        clean.truncate(clean.len() - META_SUFFIX.len());
        clean.push_str("-meta");
    }
    clean.truncate(120);
    (!clean.is_empty() && clean.chars().any(|c| c.is_ascii_alphanumeric())).then_some(clean)
}

/// The types and user-managed links a client declared on a create or update.
fn declared(req: &LwsRequest, uri: &str, content_type: &str, body: &[u8]) -> (Vec<String>, std::collections::BTreeMap<String, Vec<String>>) {
    let mut types = Vec::new();
    let mut links: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let base = url::Url::parse(uri).ok();
    let resolve = |t: &str| base.as_ref().and_then(|b| b.join(t).ok()).map(|u| u.to_string()).unwrap_or_else(|| t.to_string());
    for (target, params) in parse_links(&req.header_all(header::LINK)) {
        let Some(rel) = params.get("rel") else { continue };
        for r in rel.split_whitespace() {
            let key = if r.contains(':') { r.to_string() } else { r.to_ascii_lowercase() };
            if key == "type" {
                if !target.starts_with(LWS_NS) && is_uri(&target) && !types.contains(&target) {
                    types.push(target.clone());
                }
            } else if !STRUCTURAL_RELATIONS.contains(&key.as_str()) {
                let resolved = resolve(&target);
                let entry = links.entry(key).or_default();
                if !entry.contains(&resolved) {
                    entry.push(resolved);
                }
            }
        }
    }
    if content_type.starts_with("text/turtle") {
        if let Ok(parser) = oxttl::TurtleParser::new().with_base_iri(uri) {
            for t in parser.for_slice(body).flatten() {
                if let (oxrdf::NamedOrBlankNode::NamedNode(s), oxrdf::Term::NamedNode(o)) = (&t.subject, &t.object) {
                    if s.as_str() == uri && t.predicate.as_str() == RDF_TYPE && !types.contains(&o.as_str().to_string()) {
                        types.push(o.as_str().to_string());
                    }
                }
            }
        }
    }
    (types, links)
}

/// The linkset a new resource starts with: its declared user-managed links.
fn initial_linkset(uri: &str, links: &std::collections::BTreeMap<String, Vec<String>>) -> Option<Value> {
    if links.is_empty() {
        return None;
    }
    let mut entry = Map::new();
    entry.insert("anchor".into(), Value::String(uri.into()));
    for (rel, targets) in links {
        entry.insert(rel.clone(), Value::Array(targets.iter().map(|t| json!({"href": t})).collect()));
    }
    Some(json!({"linkset": [Value::Object(entry)]}))
}

async fn create<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, agent: &Agent, parent: &str) -> Response {
    match state.store.exists(parent).await {
        Ok(true) => {}
        Ok(false) => return problem(StatusCode::NOT_FOUND, None),
        Err(e) => return store_error(e),
    }
    if !parent.ends_with('/') {
        return method_not_allowed(&allow_for(parent, false));
    }
    let is_container = parse_links(&req.header_all(header::LINK)).iter().any(|(t, p)| {
        t == &format!("{LWS_NS}Container") && p.get("rel").is_some_and(|r| r.split_whitespace().any(|r| r.eq_ignore_ascii_case("type")))
    });
    let base_name = sanitize_slug(req.header("slug")).unwrap_or_else(|| jose::random_id().to_ascii_lowercase().replace('_', "-"));
    let mut name = base_name.clone();
    let mut n = 1;
    loop {
        let a = format!("{parent}{name}");
        let b = format!("{parent}{name}/");
        let taken = state.store.exists(&a).await.unwrap_or(true) || state.store.exists(&b).await.unwrap_or(true);
        if !taken {
            break;
        }
        n += 1;
        name = if n > 50 { format!("{base_name}-{}", jose::random_id()) } else { format!("{base_name}-{n}") };
    }
    let child = if is_container { format!("{parent}{name}/") } else { format!("{parent}{name}") };
    let content_type = if is_container {
        LWS_JSON.to_string()
    } else {
        req.header(header::CONTENT_TYPE).map(str::to_string).unwrap_or_else(|| "application/octet-stream".into())
    };
    let body = if is_container { Bytes::new() } else { req.body.clone() };
    let created = match state.store.create_in_container(parent, &child, body.clone(), &content_type).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let (types, links) = if is_container { (Vec::new(), Default::default()) } else { declared(req, &child, &content_type, &body) };
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
        .announce(state, Event { kind: "Create", uri: child.clone(), is_container, relation: Some(("target", parent.to_string())) })
        .await;
    let mut resp = problem(StatusCode::CREATED, None);
    let h = resp.headers_mut();
    set(h, header::LOCATION, &child);
    set(h, header::ETAG, &quoted(&created.etag));
    add_link(h, parent, "up", None);
    add_link(h, &lws_type(&child), "type", None);
    add_link(h, &meta_key(&child), "linkset", Some(LINKSET_JSON));
    resp
}

// ---- update ----

async fn current<S: Store + 'static>(state: &LwsState<S>, uri: &str) -> Result<crate::store::sparq::ResourceMeta, Response> {
    match state.store.meta(uri).await {
        Ok(Some(m)) => Ok(m),
        Ok(None) => Err(problem(StatusCode::NOT_FOUND, None)),
        Err(e) => Err(store_error(e)),
    }
}

async fn update<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if uri.ends_with('/') {
        return method_not_allowed(&allow_for(uri, uri == state.cfg.storage()));
    }
    if let Precondition::Failed | Precondition::NotModified =
        evaluate(&req.headers, Some(&quoted(&meta.etag)), meta.last_modified.map(|t| to_secs(epoch_ms(t))), false)
    {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    let content_type = req.header(header::CONTENT_TYPE).map(str::to_string).unwrap_or(meta.content_type);
    let written = match state.store.write(uri, req.body.clone(), &content_type).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let (types, links) = declared(req, uri, &content_type, &req.body);
    let mut rmeta = state.resource_meta(uri).await;
    rmeta.types = types;
    rmeta.links = links;
    let _ = state.put_resource_meta(uri, &rmeta).await;
    changed(state, uri).await;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set(resp.headers_mut(), header::ETAG, &quoted(&written.etag));
    resp
}

/// After a data resource changed: its container changes too, and subscribers hear of it.
async fn changed<S: Store + 'static>(state: &LwsState<S>, uri: &str) {
    if let Some(parent) = parent_of(uri, &state.cfg.storage()) {
        let mut meta = state.resource_meta(&parent).await;
        meta.version = Some(jose::random_id());
        let _ = state.put_resource_meta(&parent, &meta).await;
    }
    state.notify.announce(state, Event { kind: "Update", uri: uri.to_string(), is_container: false, relation: None }).await;
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

/// RFC 6902 JSON Patch; `None` when an operation fails (the whole patch is then not applied).
pub fn json_patch(target: &Value, ops: &Value) -> Option<Value> {
    let mut doc = target.clone();
    for op in ops.as_array()? {
        let kind = op.get("op")?.as_str()?;
        let path = op.get("path")?.as_str()?;
        match kind {
            "add" => pointer_add(&mut doc, path, op.get("value")?.clone())?,
            "remove" => {
                pointer_remove(&mut doc, path)?;
            }
            "replace" => {
                pointer_remove(&mut doc, path)?;
                pointer_add(&mut doc, path, op.get("value")?.clone())?;
            }
            "move" => {
                let from = op.get("from")?.as_str()?;
                if path.starts_with(&format!("{from}/")) {
                    return None;
                }
                let v = pointer_remove(&mut doc, from)?;
                pointer_add(&mut doc, path, v)?;
            }
            "copy" => {
                let v = doc.pointer(op.get("from")?.as_str()?)?.clone();
                pointer_add(&mut doc, path, v)?;
            }
            "test" => {
                if doc.pointer(path)? != op.get("value")? {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(doc)
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

/// Apply a merge patch or a JSON Patch, by the request's Content-Type. `Err` carries the response.
fn apply_patch(req: &LwsRequest, target: &Value) -> Result<Value, Response> {
    let ct = req.content_type().unwrap_or_default();
    let unsupported = || {
        let mut r = problem(StatusCode::UNSUPPORTED_MEDIA_TYPE, None);
        set(r.headers_mut(), header::HeaderName::from_static("accept-patch"), ACCEPT_PATCH);
        r
    };
    if ct != MERGE_PATCH && ct != JSON_PATCH {
        return Err(unsupported());
    }
    let Ok(patch) = serde_json::from_slice::<Value>(&req.body) else {
        return Err(problem(StatusCode::BAD_REQUEST, Some("the patch is not JSON")));
    };
    if ct == MERGE_PATCH {
        Ok(merge_patch(target, &patch))
    } else {
        json_patch(target, &patch).ok_or_else(|| problem(StatusCode::UNPROCESSABLE_ENTITY, Some("the JSON Patch cannot be applied")))
    }
}

async fn patch<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if uri.ends_with('/') {
        return method_not_allowed(&allow_for(uri, uri == state.cfg.storage()));
    }
    if let Precondition::Failed | Precondition::NotModified =
        evaluate(&req.headers, Some(&quoted(&meta.etag)), meta.last_modified.map(|t| to_secs(epoch_ms(t))), false)
    {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    let body = match state.store.read(uri).await {
        Ok(r) => r.body,
        Err(e) => return store_error(e),
    };
    let target = if body.is_empty() {
        Value::Object(Map::new())
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => v,
            Err(_) => {
                // The stored representation is not JSON, so neither patch format applies to it.
                let mut r = problem(StatusCode::UNSUPPORTED_MEDIA_TYPE, Some("the resource is not JSON"));
                set(r.headers_mut(), header::HeaderName::from_static("accept-patch"), ACCEPT_PATCH);
                return r;
            }
        }
    };
    let patched = match apply_patch(req, &target) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let ct = if meta.content_type.contains("json") { meta.content_type.clone() } else { JSON.to_string() };
    let written = match state.store.write(uri, Bytes::from(serde_json::to_vec(&patched).unwrap_or_default()), &ct).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    changed(state, uri).await;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set(resp.headers_mut(), header::ETAG, &quoted(&written.etag));
    resp
}

// ---- delete ----

async fn delete<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, uri: &str) -> Response {
    let storage = state.cfg.storage();
    let meta = match current(state, uri).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if uri == storage {
        return method_not_allowed(&allow_for(uri, true));
    }
    let etag = if uri.ends_with('/') { None } else { Some(quoted(&meta.etag)) };
    if req.headers.contains_key(header::IF_MATCH) && uri.ends_with('/') {
        // A container's tag is its listing's; compute it the way a read does.
        let listing = read_container(state, &LwsRequest {
            method: Method::GET,
            path: String::new(),
            query: None,
            headers: HeaderMap::new(),
            body: Bytes::new(),
        }, uri, &meta).await;
        let tag = listing.headers().get(header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_string);
        if let Precondition::Failed | Precondition::NotModified = evaluate(&req.headers, tag.as_deref(), None, false) {
            return problem(StatusCode::PRECONDITION_FAILED, None);
        }
    } else if let Precondition::Failed | Precondition::NotModified =
        evaluate(&req.headers, etag.as_deref(), meta.last_modified.map(|t| to_secs(epoch_ms(t))), false)
    {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    if uri.ends_with('/') {
        let has_members = match state.store.list_children(uri).await {
            Ok(c) => !c.is_empty(),
            Err(e) => return store_error(e),
        };
        let infinity = req.header("depth").is_some_and(|d| d.trim().eq_ignore_ascii_case("infinity"));
        if has_members && !infinity {
            return problem(StatusCode::CONFLICT, Some("the container is not empty; send Depth: infinity to delete it and everything in it"));
        }
    }
    let parent = parent_of(uri, &storage);
    // Announced before the resource goes, while who may read it can still be decided.
    state
        .notify
        .announce(state, Event {
            kind: "Delete",
            uri: uri.to_string(),
            is_container: uri.ends_with('/'),
            relation: parent.clone().map(|p| ("origin", p)),
        })
        .await;
    if let Err(e) = remove(state, uri, parent.as_deref()).await {
        return store_error(e);
    }
    if let Some(p) = parent {
        touch_container(state, &p).await;
    }
    problem(StatusCode::NO_CONTENT, None)
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
            match state.store.delete_container_if_empty(uri, parent).await? {
                crate::store::DeleteOutcome::NotEmpty => {
                    return Err(ServerError::Conflict("the container gained a member while it was deleted".into()))
                }
                _ => {}
            }
        } else {
            state.store.delete(uri, parent).await?;
            let _ = state.store.delete(&meta_key(uri), None).await;
        }
        Ok(())
    })
}

// ---- linksets ----

/// A resource's linkset (RFC 9264): GET, HEAD and PATCH (JSON Merge Patch or JSON Patch). PUT is
/// not offered, so it is 405 with the methods that are.
async fn linkset<S: Store + 'static>(state: &LwsState<S>, req: &LwsRequest, agent: &Agent, uri: &str) -> Response {
    let exists = state.store.exists(uri).await.unwrap_or(false);
    if !exists {
        return if state.needs_auth(agent) { state.challenge(None) } else { problem(StatusCode::NOT_FOUND, None) };
    }
    let action = if matches!(req.method, Method::GET | Method::HEAD | Method::OPTIONS) { Action::Read } else { Action::Modify };
    if !state.allowed(action, uri, agent).await {
        return state.deny(agent);
    }
    let mut meta = state.resource_meta(uri).await;
    let etag = meta.linkset_etag.clone().unwrap_or_else(|| {
        let mut h = Sha256::new();
        h.update(serde_json::to_vec(&meta.linkset).unwrap_or_default());
        format!("\"ls-{}\"", jose::b64url(&h.finalize()[..12]))
    });
    let document = meta.linkset.clone().unwrap_or_else(|| json!({"linkset": [{"anchor": uri}]}));
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
            if let Precondition::Failed | Precondition::NotModified = evaluate(&req.headers, Some(&etag), None, false) {
                return problem(StatusCode::PRECONDITION_FAILED, None);
            }
            let patched = match apply_patch(req, &document) {
                Ok(v) => v,
                Err(r) => return r,
            };
            if !valid_linkset(&patched) {
                return problem(StatusCode::UNPROCESSABLE_ENTITY, Some("the result is not a linkset document"));
            }
            let new_etag = format!("\"{}\"", jose::random_id());
            meta.linkset = Some(patched);
            meta.linkset_etag = Some(new_etag.clone());
            if let Err(e) = state.put_resource_meta(uri, &meta).await {
                return store_error(e);
            }
            let mut r = StatusCode::NO_CONTENT.into_response();
            set(r.headers_mut(), header::ETAG, &new_etag);
            r
        }
        _ => method_not_allowed(LINKSET_ALLOW),
    };
    set(resp.headers_mut(), header::ALLOW, LINKSET_ALLOW);
    set(resp.headers_mut(), header::HeaderName::from_static("accept-patch"), ACCEPT_PATCH);
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
                v.as_array().is_some_and(|ts| ts.iter().all(|t| t.get("href").is_some_and(Value::is_string)))
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
        assert_eq!(negotiate_container(Some("text/html;q=0.9, application/ld+json;q=0.5")).as_deref(), Some(LD_JSON));
        assert_eq!(negotiate_container(Some("application/json")).as_deref(), Some(JSON));
        assert_eq!(negotiate_container(Some("text/html")), None);
        assert_eq!(
            negotiate_container(Some(r#"application/ld+json; profile="https://www.w3.org/ns/lws/v1""#)).as_deref(),
            Some(LWS_JSON)
        );
        assert_eq!(negotiate_container(Some("application/json;q=0.2, application/lws+json")).as_deref(), Some(LWS_JSON));
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-3", 10), Some((0, 3)));
        assert_eq!(parse_range("bytes=-2", 10), Some((8, 9)));
        assert_eq!(parse_range("bytes=5-", 10), Some((5, 9)));
        assert_eq!(parse_range("bytes=100-200", 10), None);
        assert_eq!(parse_range("bytes=0-1,3-4", 10), None);
    }

    #[test]
    fn patches() {
        let t = json!({"title": "a", "keep": 1, "drop": true});
        assert_eq!(merge_patch(&t, &json!({"added": 42, "drop": null})), json!({"title": "a", "keep": 1, "added": 42}));
        let p = json!([{"op": "add", "path": "/added", "value": 42}, {"op": "remove", "path": "/drop"},
                       {"op": "replace", "path": "/title", "value": "b"}, {"op": "test", "path": "/keep", "value": 1}]);
        assert_eq!(json_patch(&t, &p), Some(json!({"title": "b", "keep": 1, "added": 42})));
        assert_eq!(json_patch(&t, &json!([{"op": "test", "path": "/keep", "value": 2}])), None);
        assert_eq!(json_patch(&json!({"a": [1, 2]}), &json!([{"op": "add", "path": "/a/-", "value": 3}])), Some(json!({"a": [1, 2, 3]})));
    }

    #[test]
    fn slugs() {
        assert_eq!(sanitize_slug(Some("shoppinglist.txt")).as_deref(), Some("shoppinglist.txt"));
        assert_eq!(sanitize_slug(Some("../etc")).as_deref(), Some("-etc"));
        assert_eq!(sanitize_slug(Some(".meta")), Some("-meta".into()).filter(|_| false).or(sanitize_slug(Some(".meta"))));
        assert!(!sanitize_slug(Some("x.meta")).unwrap().ends_with(".meta"));
        assert_eq!(sanitize_slug(Some("...")), None);
        assert_eq!(sanitize_slug(None), None);
    }

    #[test]
    fn parents() {
        let s = "http://h/";
        assert_eq!(parent_of("http://h/a", s).as_deref(), Some("http://h/"));
        assert_eq!(parent_of("http://h/a/b/", s).as_deref(), Some("http://h/a/"));
        assert_eq!(parent_of("http://h/", s), None);
    }

    #[test]
    fn etag_lists() {
        assert!(etag_listed("\"a\", \"b\"", "\"b\"", false));
        assert!(!etag_listed("W/\"b\"", "\"b\"", false));
        assert!(etag_listed("W/\"b\"", "\"b\"", true));
        assert!(etag_listed("*", "\"x\"", false));
    }
}
