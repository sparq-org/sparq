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
    JSON_PATCH, LD_JSON, LINKSET_JSON, LWS_CID, LWS_CONTEXT, LWS_JSON, LWS_NS, META_SUFFIX,
};
use crate::error::ServerError;
use crate::store::Store;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACCEPT_PATCH: &str = "application/json-patch+json";
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
    let target = req.path.strip_suffix(META_SUFFIX).unwrap_or(&req.path);
    if let Some(refused) = set_aside(state, &state.cfg.absolute(target), &req.method) {
        return refused;
    }
    // A write or a delete waits for its locks and checks its preconditions in the request, where
    // a client that goes away (or a timeout) cancels the wait; only its store calls run in a
    // task of their own, once they are ready to start (see [`hold_locks`]).
    handle_now(state, req, agent).await
}

/// The answer to a `method` request for `uri` that cannot be served while failed changes are
/// put back: `uri` is not [visible](super::LwsState::visible) (answered at once rather than left
/// waiting on its locks), or the request would change something while set-aside changes hold
/// as much as they may ([`may_write`](super::LwsState::may_write)).
pub(crate) fn set_aside<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
    method: &Method,
) -> Option<Response> {
    let why = if !state.visible(uri) {
        "a failed change to this resource is still being put back"
    } else if !method.is_safe() && !state.may_write() {
        "failed changes are still being put back"
    } else {
        return None;
    };
    Some(retry_later(why))
}

/// `503` with a `Retry-After`, for `why`.
fn retry_later(why: &str) -> Response {
    let mut resp = problem(StatusCode::SERVICE_UNAVAILABLE, Some(why));
    set(resp.headers_mut(), header::RETRY_AFTER, "5");
    resp
}

/// The answer to a request whose resource was set aside while it waited for its lock (see
/// [`IriLocks::lock`]).
fn set_aside_meanwhile() -> Response {
    retry_later("a failed change to this resource is still being put back")
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
        // A partial PUT (RFC 9110 section 14.5) is not supported: taken as a whole
        // representation, it would replace the resource with the part.
        Method::PUT if req.headers.contains_key(header::CONTENT_RANGE) => problem(
            StatusCode::BAD_REQUEST,
            Some("a partial PUT (Content-Range) is not supported"),
        ),
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

/// An `If-Match` / `If-None-Match` field value: `*`, or entity tags as `(weak, opaque-tag)`, the
/// opaque tag with its quotes, as bytes (it may hold `obs-text`).
enum TagList {
    Any,
    Tags(Vec<(bool, Vec<u8>)>),
}

impl TagList {
    /// Parse a field value by the RFC 9110 grammar (`"*" / #entity-tag`, section 8.8.3): a comma
    /// inside a quoted tag belongs to the tag, and `*` is the whole value or not a wildcard.
    /// Anything else is unreadable.
    fn parse(b: &[u8]) -> Result<Self, ()> {
        if b.trim_ascii() == b"*" {
            return Ok(Self::Any);
        }
        let mut i = 0;
        let mut tags = Vec::new();
        loop {
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b',') {
                i += 1;
            }
            if i == b.len() {
                break;
            }
            let weak = b[i..].starts_with(b"W/");
            if weak {
                i += 2;
            }
            if b.get(i) != Some(&b'"') {
                return Err(());
            }
            let start = i;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if !(b[i] == 0x21 || (0x23..=0x7e).contains(&b[i]) || b[i] >= 0x80) {
                    return Err(());
                }
                i += 1;
            }
            if i == b.len() {
                return Err(());
            }
            i += 1;
            tags.push((weak, b[start..i].to_vec()));
            while i < b.len() && matches!(b[i], b' ' | b'\t') {
                i += 1;
            }
            if i < b.len() && b[i] != b',' {
                return Err(());
            }
        }
        // An empty list (`#entity-tag` allows one) names no tag: it matches nothing.
        Ok(Self::Tags(tags))
    }

    /// Whether the list matches `etag` (the server's own, quoted). `weak` compares opaque tags
    /// only (RFC 9110 section 8.8.3.2).
    fn matches(&self, etag: &str, weak: bool) -> bool {
        let (etag_weak, opaque) = match etag.strip_prefix("W/") {
            Some(o) => (true, o),
            None => (false, etag),
        };
        match self {
            Self::Any => true,
            Self::Tags(tags) => tags
                .iter()
                .any(|(w, t)| t == opaque.as_bytes() && (weak || (!w && !etag_weak))),
        }
    }
}

#[cfg(test)]
fn etag_listed(header: &str, etag: &str, weak: bool) -> bool {
    TagList::parse(header.as_bytes()).is_ok_and(|l| l.matches(etag, weak))
}

/// A body (of POST, PUT, PATCH or QUERY) sent with a content coding other than `identity` is refused (`415`, with
/// `Accept-Encoding: identity`, RFC 9110 section 15.5.16) before anything reads it: the server
/// does not decode bodies, and storing the coded bytes as the representation would drop the
/// coding.
pub(crate) fn refuse_encoded(req: &LwsRequest) -> Option<Response> {
    if !matches!(req.method, Method::POST | Method::PUT | Method::PATCH)
        && req.method.as_str() != "QUERY"
    {
        return None;
    }
    let codings = req.header_all(header::CONTENT_ENCODING);
    let unreadable = req
        .headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .any(|v| v.to_str().is_err());
    let coded = unreadable
        || codings
            .split(',')
            .map(str::trim)
            .any(|c| !c.is_empty() && !c.eq_ignore_ascii_case("identity"));
    if !coded {
        return None;
    }
    let mut resp = problem(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        Some("content codings are not supported: send the body unencoded"),
    );
    set(resp.headers_mut(), header::ACCEPT_ENCODING, "identity");
    Some(resp)
}

/// The outcome of evaluating preconditions (RFC 9110 section 13.2.2).
enum Precondition {
    Proceed,
    NotModified,
    Failed,
}

/// Preconditions are read once, here, from every field line: a list header (`If-Match`,
/// `If-None-Match`) sent as several lines is all of them, not the first. An entity-tag list that
/// cannot be read (not the RFC 9110 grammar; obs-text is allowed) fails the request rather than
/// being skipped; a date that is not one valid HTTP-date (unparsable, or sent twice) is ignored,
/// as RFC 9110 sections 13.1.3 and 13.1.4 require.
fn evaluate(
    headers: &HeaderMap,
    etag: Option<&str>,
    modified_secs: Option<u64>,
    read: bool,
) -> Precondition {
    let Ok(pre) = Preconditions::read(headers) else {
        return Precondition::Failed;
    };
    if let Some(im) = pre.if_match.as_ref() {
        if !etag.is_some_and(|e| im.matches(e, false)) {
            return Precondition::Failed;
        }
    } else if let (Some(since), Some(m)) = (pre.if_unmodified_since, modified_secs) {
        if m > since {
            return Precondition::Failed;
        }
    }
    if let Some(inm) = pre.if_none_match.as_ref() {
        if etag.is_some_and(|e| inm.matches(e, true)) {
            return if read {
                Precondition::NotModified
            } else {
                Precondition::Failed
            };
        }
    } else if read {
        if let (Some(since), Some(m)) = (pre.if_modified_since, modified_secs) {
            let now = to_secs(now_ms());
            if since <= now && m <= since {
                return Precondition::NotModified;
            }
        }
    }
    Precondition::Proceed
}

/// A request's preconditions, every field line of each read.
struct Preconditions {
    if_match: Option<TagList>,
    if_none_match: Option<TagList>,
    if_unmodified_since: Option<u64>,
    if_modified_since: Option<u64>,
}

impl Preconditions {
    fn read(headers: &HeaderMap) -> Result<Self, ()> {
        let list = |n: header::HeaderName| -> Result<Option<TagList>, ()> {
            // Read as bytes: an opaque tag may hold obs-text (RFC 9110 section 8.8.3).
            let lines: Vec<&[u8]> = headers.get_all(n).iter().map(|v| v.as_bytes()).collect();
            if lines.is_empty() {
                return Ok(None);
            }
            TagList::parse(&lines.join(&b", "[..])).map(Some)
        };
        let date = |n: header::HeaderName| -> Result<Option<u64>, ()> {
            let mut lines = headers.get_all(n).iter();
            Ok(match (lines.next(), lines.next()) {
                (Some(v), None) => parse_http_date(v.to_str().ok()),
                _ => None,
            })
        };
        Ok(Self {
            if_match: list(header::IF_MATCH)?,
            if_none_match: list(header::IF_NONE_MATCH)?,
            if_unmodified_since: date(header::IF_UNMODIFIED_SINCE)?,
            if_modified_since: date(header::IF_MODIFIED_SINCE)?,
        })
    }
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
///
/// `Err` when the container was set aside meanwhile (answered `503`).
async fn listing_guard<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<Option<tokio::sync::OwnedRwLockReadGuard<()>>, Response> {
    let Some(parent) = parent_of(uri, &state.cfg.storage()) else {
        return Ok(None);
    };
    match state.locks.read(&parent).await {
        Some(guard) => Ok(Some(guard)),
        None => Err(set_aside_meanwhile()),
    }
}

/// [`listing_guard`], exclusive: what removes a member holds its container so, so that no
/// listing is read while the member is gone and the removal may yet be put back.
async fn listing_lock<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
) -> Result<Option<IriGuard>, Response> {
    let Some(parent) = parent_of(uri, &state.cfg.storage()) else {
        return Ok(None);
    };
    match state.locks.lock(&parent).await {
        Some(guard) => Ok(Some(guard)),
        None => Err(set_aside_meanwhile()),
    }
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
pub(crate) async fn touch_container<S: Store + 'static>(state: &LwsState<S>, container: &str) {
    // Until a touch lands, the container's listing has no Last-Modified (see
    // [`read_container`]): its own time, and its members', may all be from before the change,
    // and an If-Modified-Since would pass on a listing that changed.
    // The intents of kept changes waiting on a touch are taken before it starts, so only those
    // committed before it began are cleared by it; they go back when it does not land.
    let owed = state.take_owed_touches(container);
    let landed = touch(state, container).await;
    state.touched(container, landed);
    if !landed {
        for record in owed {
            state.owe_touch(container, record);
        }
        return;
    }
    for record in owed {
        // A failure leaves the intent, and the next start touches the container again.
        let _ = super::intents::clear(&state.store, &record).await;
    }
}

/// Release `locks`, held through a change to the container's listing, and touch the container
/// ([`touch_container`]). Its date counts as behind from before the locks go until the touch
/// lands, so no request sees the changed listing under a date from before the change.
async fn touch_after<S: Store + 'static, L>(state: &LwsState<S>, locks: L, container: &str) {
    state.touching(container);
    drop(locks);
    touch_container(state, container).await;
}

/// Move the container's modification time and version on: whether that landed. Metadata that
/// cannot be read is left as it is, not replaced by a default; a container set aside meanwhile
/// is not waited on.
async fn touch<S: Store + 'static>(state: &LwsState<S>, container: &str) -> bool {
    let Some(_guard) = state.locks.read(container).await else {
        return false;
    };
    let Ok(_listing) = listing_guard(state, container).await else {
        return false;
    };
    let Some(_touch) = state.locks.lock(&format!("{container}\0touch")).await else {
        return false;
    };
    let Ok(mut meta) = state.resource_meta(container).await else {
        return false;
    };
    meta.modified_ms = Some(now_ms());
    meta.version = Some(jose::random_id());
    state.put_resource_meta(container, &meta).await.is_ok()
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
    let mut tries = 0;
    let (_guard, meta) = loop {
        let Some(guard) = state.locks.read(uri).await else {
            return set_aside_meanwhile();
        };
        let meta = match current(state, uri).await {
            Ok(m) => m,
            Err(r) => return r,
        };
        if let Err(r) = recheck(state, Action::Read, uri, agent).await {
            return r;
        }
        if !uri.ends_with('/') {
            break (guard, meta);
        }
        let Busy(member) = match read_container(state, req, uri, &meta, false).await {
            Ok(listing) => return listing,
            Err(busy) => busy,
        };
        // A change to a member is in flight: the listing is read again once it is over, so
        // nothing it may yet put back is shown. Waited for without the container's lock, which
        // the change may take after the member's.
        drop(guard);
        tries += 1;
        if tries == LISTING_TRIES {
            return retry_later("the container's members kept changing while it was listed");
        }
        if state.locks.read(&member).await.is_none() {
            return set_aside_meanwhile();
        }
    };
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
            ranged(req, body, &meta.content_type, &etag)
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
fn ranged(req: &LwsRequest, body: Bytes, content_type: &str, etag: &str) -> Response {
    let len = body.len() as u64;
    // Range applies to GET only: a HEAD answers with the headers of the whole representation
    // (RFC 9110 section 14.2).
    let range = req
        .header(header::RANGE)
        .filter(|_| req.method == Method::GET)
        .filter(|_| if_range_holds(req.header(header::IF_RANGE), etag));
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

/// Whether an If-Range precondition lets the Range apply (RFC 9110 section 13.1.5): absent, or a
/// strong entity tag equal to the current one. A date never does: two versions written within
/// one second share a Last-Modified, so a date is not a strong validator here, and the whole
/// representation is served instead.
fn if_range_holds(if_range: Option<&str>, etag: &str) -> bool {
    let Some(v) = if_range.map(str::trim) else {
        return true;
    };
    if v.starts_with('"') || v.starts_with("W/") {
        // A weak tag never matches: the comparison is strong.
        return !v.starts_with("W/") && !etag.starts_with("W/") && v == etag;
    }
    false
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

/// How many times a listing is read again when changes to its members are in flight, before it
/// is answered `503`.
const LISTING_TRIES: usize = 8;

/// Why a container could not be listed: a change to this member is in flight.
struct Busy(String);

/// Why [`members`] could not list a container.
enum Unlisted {
    Store(ServerError),
    Busy(Busy),
}

impl From<ServerError> for Unlisted {
    fn from(e: ServerError) -> Self {
        Unlisted::Store(e)
    }
}

/// A container's members, sorted, with what a listing shows about each but a data resource's
/// size (which [`with_sizes`] adds for the members shown). Each member's own entity tag stands
/// for its content.
///
/// Each member is read under its shared lock, so what is listed of it is what it was before or
/// after a change to it, never in between: a change in flight to one (its lock busy) makes the
/// listing [`Busy`] instead, unless the caller holds the container `exclusive`ly (then no change
/// to a member is in flight: each holds the container's lock, shared, while it runs).
async fn members<S: Store + 'static>(
    state: &LwsState<S>,
    uri: &str,
    exclusive: bool,
) -> Result<Vec<(Value, crate::store::sparq::ResourceMeta)>, Unlisted> {
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
        let _member = match exclusive {
            true => None,
            false => match state.locks.try_read(&child) {
                Some(guard) => Some(guard),
                None => return Err(Unlisted::Busy(Busy(child))),
            },
        };
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

/// The listing of the container `uri` ([`members`], `exclusive` as there).
async fn read_container<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    uri: &str,
    meta: &crate::store::sparq::ResourceMeta,
    exclusive: bool,
) -> Result<Response, Busy> {
    let Some(media_type) = negotiate_container(req.header(header::ACCEPT)) else {
        return Ok(problem(StatusCode::NOT_ACCEPTABLE, None));
    };
    // Whether the container's date is behind is read before anything the date is computed
    // from: a touch that lands while the listing is made must not make a date from before it
    // count. And a listing made while a change to the container was published, or a touch of
    // it landed, is made again: it could pair members from before with a date from after.
    let generation = state.generation(uri);
    let untouched = state.is_untouched(uri) || state.in_flight(uri);
    let all = match members(state, uri, exclusive).await {
        Ok(m) => m,
        Err(Unlisted::Store(e)) => return Ok(store_error(e)),
        Err(Unlisted::Busy(busy)) => return Err(busy),
    };
    let resp = listing(state, req, uri, meta, media_type, all, untouched).await;
    // Held exclusively, the container cannot change meanwhile (a counter it shares with
    // others may move, which says nothing about it).
    if !exclusive && state.generation(uri) != generation {
        return Err(Busy(uri.to_string()));
    }
    Ok(resp)
}

async fn listing<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    uri: &str,
    meta: &crate::store::sparq::ResourceMeta,
    media_type: String,
    all: Vec<(Value, crate::store::sparq::ResourceMeta)>,
    untouched: bool,
) -> Response {
    let cmeta = match state.resource_meta(uri).await {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let page_size = state.cfg.page_size;
    let pages = all.len().div_ceil(page_size).max(1);
    let page = match req.query_param("page") {
        None => 1,
        Some(p) => p.parse::<usize>().unwrap_or(0),
    };
    if page < 1 || page > pages {
        return problem(StatusCode::NOT_FOUND, None);
    }
    // The page is made whole, sizes included, before its validators: a size that cannot be read
    // (its version replaced since, or a failed read) is left out, and the entity tag covers what
    // is shown, so two different representations never share a strong tag.
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
    // The entity tag covers the whole listing: every field of every member it represents (a
    // member container's `modified` included, which moves when something is created in it, while
    // that container's own stored tag does not), and each member's own tag; and the page as
    // shown.
    let mut hasher = Sha256::new();
    hasher.update(meta.etag.as_bytes());
    hasher.update(cmeta.version.as_deref().unwrap_or_default().as_bytes());
    for (m, _) in &all {
        hasher.update(serde_json::to_vec(m).unwrap_or_default());
        hasher.update(b"\n");
    }
    hasher.update(serde_json::to_vec(&items).unwrap_or_default());
    let tag = jose::b64url(&hasher.finalize()[..18]);
    // The listing changed no earlier than any member it lists did, so its Last-Modified is the
    // latest of the container's own and every listed member's: an If-Modified-Since never meets a
    // 304 for a listing whose members moved on since. A container whose own time could not be
    // moved on after a change to it has none, until it is.
    let modified = (!untouched).then(|| {
        all.iter()
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
            )
    });
    // Each page is its own representation, so a later page has its own entity tag.
    let etag = if page == 1 {
        format!("\"{tag}\"")
    } else {
        format!("\"{tag}-p{page}\"")
    };
    match evaluate(&req.headers, Some(&etag), modified, true) {
        Precondition::Failed => return problem(StatusCode::PRECONDITION_FAILED, None),
        Precondition::NotModified => {
            let mut r = StatusCode::NOT_MODIFIED.into_response();
            set(r.headers_mut(), header::ETAG, &etag);
            if let Some(modified) = modified {
                set(r.headers_mut(), header::LAST_MODIFIED, &http_date(modified));
            }
            set(r.headers_mut(), header::VARY, "Accept");
            resource_links(state, r.headers_mut(), uri, &cmeta.types);
            return r;
        }
        Precondition::Proceed => {}
    }
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
    if let Some(modified) = modified {
        set(h, header::LAST_MODIFIED, &http_date(modified));
    }
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
        // Every target is a URI reference (RFC 8288 section 3), resolved by the one IRI parser;
        // one that is not refuses the request before anything is written.
        let Some(resolved) = resolve_reference(uri, &target) else {
            return Err(problem(
                StatusCode::BAD_REQUEST,
                Some("a Link target is not a URI reference"),
            ));
        };
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

/// `target` resolved against `base` by the RFC 3987 parser, or `None` when it is not a URI
/// reference.
fn resolve_reference(base: &str, target: &str) -> Option<String> {
    Some(
        oxiri::Iri::parse(base)
            .ok()?
            .resolve(target)
            .ok()?
            .into_inner(),
    )
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
        match state.locks.lock(parent).await {
            Some(guard) => (None, Some(guard)),
            None => return set_aside_meanwhile(),
        }
    } else {
        match state.locks.read(parent).await {
            Some(guard) => (Some(guard), None),
            None => return set_aside_meanwhile(),
        }
    };
    if conditional {
        let meta = match current(state, parent).await {
            Ok(m) => m,
            Err(r) => return r,
        };
        // Evaluated against the representation a GET would select: at the storage root that
        // may be the storage description rather than the listing (see [`read`]).
        let get = plain_get(req);
        let (etag, modified) =
            if parent == state.cfg.storage() && serves_description(get.header(header::ACCEPT)) {
                (Some(description_etag(&storage_description(state))), None)
            } else {
                // Held exclusively: no change to a member is in flight.
                let listing = read_container(state, &get, parent, &meta, true)
                    .await
                    .unwrap_or_else(|_| set_aside_meanwhile());
                if !listing.status().is_success() {
                    return listing;
                }
                validators_of(&listing)
            };
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
    // is removed again, with the locks held until it is.
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
                    // follow a create that committed (a remote store's lost reply): the member is
                    // removed first, so committed content is never left without its creator.
                    // Both are tried a few times with the locks held, so no request sees the
                    // member or its metadata in between; what is left then is set aside with
                    // the locks (see [`LwsState::set_aside`](super::LwsState::set_aside)).
                    let mut undo = Vec::new();
                    if matches!(e, ServerError::Storage(_)) {
                        undo.push(super::Undo::Remove {
                            iri: child.clone(),
                            parent: parent.clone(),
                        });
                    }
                    undo.push(super::Undo::Restore {
                        key: meta_key(&child),
                        prior: None,
                    });
                    match super::settle(&state.store, undo).await {
                        None => drop(locks),
                        Some(left) => state.set_aside(left, locks),
                    }
                    return Err(e);
                }
            };
            // What follows a commit follows it whether or not the client is still there: the
            // container's bookkeeping. The locks go first (the touch takes the container's lock
            // again).
            touch_after(&state, locks, &parent).await;
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
    // A data resource's tag is its content's. A container's representation is its listing,
    // whose tag a read computes ([`read_container`]); the stored record's would match nothing a
    // read or a conditional request sees, so none is sent.
    if !is_container {
        set(h, header::ETAG, &quoted(&created.etag));
    }
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
pub struct IriLocks {
    map: std::sync::Mutex<LockMap>,
    /// The IRIs set aside ([`LwsState::set_aside`](super::LwsState::set_aside)), each with how
    /// many set-aside changes are to it.
    hidden: std::sync::Mutex<std::collections::HashMap<String, usize>>,
    /// Woken whenever an IRI is set aside, so a wait for its lock gives up.
    hid: tokio::sync::Notify,
}

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
    ///
    /// Every lock is taken only while `iri` is [visible](Self::visible): `None` at once when it
    /// is not, or as soon as it stops being while the lock is waited for (a change holding it was
    /// set aside with its locks), so nobody waits on a lock a recovery holds; and `None` when it
    /// was set aside by the time the lock is had.
    pub async fn lock(&self, iri: &str) -> Option<IriGuard> {
        self.visibly(iri, self.mutex(iri).write_owned()).await
    }

    /// Wait for and take the shared lock of `iri` (while it is visible, as [`IriLocks::lock`]): a
    /// read holds it from its permission check through the representation it serves, so no
    /// write lands in between. Readers share it; a reader holds no other lock, so it cannot take
    /// part in a deadlock.
    pub async fn read(&self, iri: &str) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        self.visibly(iri, self.mutex(iri).read_owned()).await
    }

    /// Take the lock of `iri` if it is free and visible, without waiting: the one way to take a
    /// lock out of [`lock_order`], since it cannot deadlock.
    pub fn try_lock(&self, iri: &str) -> Option<IriGuard> {
        let guard = self.mutex(iri).try_write_owned().ok()?;
        self.visible(iri).then_some(guard)
    }

    /// Take the shared lock of `iri` if no change to it is in flight and it is visible, without
    /// waiting.
    pub fn try_read(&self, iri: &str) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        let guard = self.mutex(iri).try_read_owned().ok()?;
        self.visible(iri).then_some(guard)
    }

    /// Whether `iri` is not set aside.
    pub fn visible(&self, iri: &str) -> bool {
        !self
            .hidden
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(iri)
    }

    /// Set `iris` aside (once more each), and wake every wait for a lock so it checks again.
    pub(crate) fn hide(&self, iris: &[String]) {
        {
            let mut hidden = self.hidden.lock().unwrap_or_else(|e| e.into_inner());
            for iri in iris {
                *hidden.entry(iri.clone()).or_default() += 1;
            }
        }
        self.hid.notify_waiters();
    }

    /// Undo one [`IriLocks::hide`] of `iris`: each is visible again once nothing else set it
    /// aside.
    pub(crate) fn show(&self, iris: &[String]) {
        let mut hidden = self.hidden.lock().unwrap_or_else(|e| e.into_inner());
        for iri in iris {
            if let std::collections::hash_map::Entry::Occupied(mut e) = hidden.entry(iri.clone()) {
                *e.get_mut() -= 1;
                if *e.get() == 0 {
                    e.remove();
                }
            }
        }
    }

    async fn visibly<G>(
        &self,
        iri: &str,
        acquire: impl std::future::Future<Output = G>,
    ) -> Option<G> {
        tokio::pin!(acquire);
        loop {
            // Registered before the check, so a hide between the two still wakes it.
            let hid = self.hid.notified();
            tokio::pin!(hid);
            hid.as_mut().enable();
            if !self.visible(iri) {
                return None;
            }
            tokio::select! {
                guard = &mut acquire => return self.visible(iri).then_some(guard),
                _ = &mut hid => {}
            }
        }
    }

    fn mutex(&self, iri: &str) -> std::sync::Arc<tokio::sync::RwLock<()>> {
        let mut guard = self.map.lock().unwrap_or_else(|e| e.into_inner());
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
/// outcome. The writes run in a task of their own, spawned once the locks are held and the
/// request's checks are done, so a client that goes away cannot release the locks while a write
/// is still pending (a remote store may commit a request that was already sent): nobody else acts
/// on the resource until the writes are over, and none of them is cut off half way. The task
/// holds `admission`, the request's share of its admission permit, so a write that outlives its
/// request still counts against the concurrency ceiling until it ends.
///
/// When the writes failed and could not be put back (`writes` yields what is left), the task
/// sets that aside with the locks ([`LwsState::set_aside`](super::LwsState::set_aside)) and the
/// locks do not come back. When they changed a container's listing (`touch` names it, from
/// their outcome), the task also releases the locks and touches it ([`touch_container`]): a
/// request that goes away once its writes have started cannot leave the container's date
/// behind its listing.
async fn hold_locks<S, L, T, F, C>(
    state: &LwsState<S>,
    locks: L,
    admission: Option<crate::overload::AdmissionSlot>,
    touch: C,
    writes: F,
) -> Result<(T, Option<L>), ServerError>
where
    S: Store + 'static,
    L: Send + 'static,
    T: Send + 'static,
    F: std::future::Future<Output = (T, Option<super::Unsettled>)> + Send + 'static,
    C: FnOnce(&T) -> Option<String> + Send + 'static,
{
    let state = state.clone();
    tokio::spawn(async move {
        let _admission = admission;
        match writes.await {
            (out, None) => match touch(&out) {
                Some(container) => {
                    touch_after(&state, locks, &container).await;
                    (out, None)
                }
                None => (out, Some(locks)),
            },
            (out, Some(left)) => {
                state.set_aside(left, locks);
                (out, None)
            }
        }
    })
    .await
    .map_err(|e| ServerError::Storage(format!("the request failed: {e}")))
}

/// Write new content for `uri` and, with `meta` = `(new, old)`, its new metadata, such that a
/// failure (or the request being dropped) never leaves content visible under metadata that does
/// not describe it: the types (and the creator) in the metadata are what grants rest on, so new
/// content under old types, or old content under new ones, could reach readers neither state
/// admits.
///
/// - Metadata unchanged: one content write, which lands whole or not at all.
/// - Metadata changed: three writes, through a [`Journal`](super::Journal). The old metadata
///   marked `pending` first; then the content; then the new metadata, which clears the mark.
///   When any step fails, the journal puts back what the steps before it did, the last first, so
///   the resource is as it was: its content and its metadata. When putting back fails too, the
///   `pending` mark stays and the resource fails closed: only its owner and creator may act on it
///   (see [`access::allowed`](super::access::allowed)) until a write completes.
///
/// The writes, a lone content write included, run with the resource's lock (`guard`, handed back
/// when they are done) held, in the request's own task (see [`hold_locks`]), so a client that goes
/// away mid-way cancels neither the writes nor the rollback, and nobody sees the steps in between.
///
/// The last element says the failure is settled: everything was put back, so nothing changed.
async fn write_with_meta<S: Store + 'static, L: Send + 'static>(
    state: &LwsState<S>,
    guard: L,
    uri: &str,
    body: Bytes,
    content_type: &str,
    meta: Option<(ResourceMeta, Option<ResourceMeta>)>,
    admission: Option<crate::overload::AdmissionSlot>,
) -> (
    Result<crate::store::sparq::ResourceMeta, ServerError>,
    Option<L>,
    bool,
) {
    // The container's listing shows the resource's format, size and date, so it changes with
    // a write that may have landed: one that succeeded, or failed in the backend without being
    // put back (a remote store's lost reply may follow a commit).
    let parent = parent_of(uri, &state.cfg.storage());
    let landed = |outcome: &Result<crate::store::sparq::ResourceMeta, ServerError>,
                  undone: bool| {
        !undone && matches!(outcome, Ok(_) | Err(ServerError::Storage(_)))
    };
    let changed = meta
        .map(|(mut new, old)| {
            new.pending = false;
            (new, old)
        })
        .filter(|(new, old)| old.clone().unwrap_or_default() != *new);
    // Metadata that could not be stored is refused before anything is written.
    if let Some(Err(e)) = changed.as_ref().map(|(new, _)| encode_meta(new)) {
        return (Err(e), Some(guard), true);
    }
    let (state, uri, content_type) = (state.clone(), uri.to_string(), content_type.to_string());
    let Some((new, old)) = changed else {
        let write = {
            let state = state.clone();
            async move { (state.store.write(&uri, body, &content_type).await, None) }
        };
        let touch = move |w: &Result<_, ServerError>| parent.filter(|_| landed(w, false));
        return match hold_locks(&state, guard, admission, touch, write).await {
            Ok((written, guard)) => (written, guard, false),
            Err(e) => (Err(e), None, false),
        };
    };
    let writes = {
        let (state, parent) = (state.clone(), parent.clone());
        async move {
            let mut journal = state.journal();
            let steps = async {
                // Both are staged first, so one intent is stored before the first step.
                journal.stage(&meta_key(&uri)).await?;
                journal.stage(&uri).await?;
                let mut closed = old.clone().unwrap_or_default();
                closed.pending = true;
                journal.write_meta(&uri, &closed).await?;
                let written = journal.write(&uri, body, &content_type).await?;
                journal.write_meta(&uri, &new).await?;
                Ok(written)
            }
            .await;
            match steps {
                Ok(written) => match journal.commit(parent.as_deref()).await {
                    Ok(()) => ((Ok(written), false), None),
                    Err((e, left)) => ((Err(e), left.is_none()), left),
                },
                Err(e) => match journal.rollback().await {
                    None => ((Err(e), true), None),
                    left => ((Err(e), false), left),
                },
            }
        }
    };
    let touch =
        move |(w, undone): &(Result<_, ServerError>, bool)| parent.filter(|_| landed(w, *undone));
    match hold_locks(&state, guard, admission, touch, writes).await {
        Ok(((outcome, undone), guard)) => (outcome, guard, undone),
        Err(e) => (Err(e), None, false),
    }
}

async fn update<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let Some(guard) = state.locks.lock(uri).await else {
        return set_aside_meanwhile();
    };
    let listing = match listing_guard(state, uri).await {
        Ok(listing) => listing,
        Err(r) => return r,
    };
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
    // The listing lock goes with the resource's into the writes' task (see [`hold_locks`]),
    // which touches the container once they are over.
    let (written, _guard, _) = write_with_meta(
        state,
        (guard, listing),
        uri,
        req.body.clone(),
        &content_type,
        Some((rmeta, old_rmeta)),
        req.admission.clone(),
    )
    .await;
    let written = match written {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
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

// ---- patch ----

/// The most a patched document may grow to, in serialized bytes, whatever the configured body
/// limit. A JSON Patch `copy` doubles what it copies, so without a bound a few dozen operations
/// exhaust memory. A server patches within the smaller of this and its request body limit
/// ([`patch_budget`]).
pub const PATCH_BUDGET: usize = 64 * 1024 * 1024;

/// How far a patched document may grow on this server: no larger than a request body may be (a
/// document a PUT could not store is no more acceptable from a PATCH), and never past
/// [`PATCH_BUDGET`]. The work a JSON Patch may do follows from it.
fn patch_budget<S: Store + 'static>(state: &LwsState<S>) -> usize {
    state.cfg.max_body.min(PATCH_BUDGET)
}

/// How many operations a JSON Patch may have.
pub const MAX_PATCH_OPS: usize = 1000;

/// The work a JSON Patch may do, in bytes measured, cloned or compared, as a multiple of
/// [`PATCH_BUDGET`].
pub const PATCH_WORK_FACTOR: usize = 4;

/// The least work any JSON Patch may do, however small its size budget.
pub const MIN_PATCH_WORK: usize = 1 << 20;

/// How deeply a patched document may nest, in arrays and objects: the deepest document
/// `serde_json` parses back (its recursion limit). The parser bounds every document a request
/// carries, but JSON Patch builds new ones: each `move` or `copy` may nest an existing value under
/// another, so without this bound a patch could build a document deep enough to overflow the stack
/// when it is serialized, compared, cloned or dropped.
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

/// Whether `content_type` is JSON: `application/json`, or any `+json` structured syntax suffix
/// (RFC 6839), parameters aside.
fn is_json_type(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    essence == "application/json" || essence.ends_with("+json")
}

/// What marks a number held as text while a JSON Patch is applied. A JSON Patch never reads a
/// number as a binary float: before the patch and its target are parsed, every number in them is
/// rewritten as a string holding this mark and the number's own text ([`numbers_as_text`]), and
/// written back as that text once the patch is applied ([`numbers_from_text`]). So a number the
/// patch does not touch is written back as it was, whatever its size or precision, and `test`
/// compares numbers as the decimal values their texts denote ([`json_equal`]). A string value
/// of the document that itself starts with the mark is kept apart by doubling it.
const NUMBER_MARK: char = '\u{E000}';

/// Rewrite the JSON text `json`, token by token: each string in a value position (never an
/// object member's name) through `string` (given its raw content between the quotes; `None`
/// keeps it), and each number through `number`. Everything else is copied. Text that is not
/// JSON is copied as far as it can be scanned, for the parser to refuse.
fn rewrite_json(
    json: &str,
    string: impl Fn(&str) -> Option<String>,
    number: impl Fn(&str) -> String,
) -> String {
    let b = json.as_bytes();
    let mut out = String::with_capacity(json.len() + json.len() / 8);
    // Per open container, whether it is an object; and whether a member name comes next.
    let mut stack: Vec<bool> = Vec::new();
    let mut name = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let end = j.min(b.len() - 1);
                let raw = &json[i..=end];
                let content = &json[i + 1..j.min(b.len())];
                if name && stack.last() == Some(&true) {
                    out.push_str(raw);
                } else {
                    match string(content) {
                        Some(r) => out.push_str(&r),
                        None => out.push_str(raw),
                    }
                }
                i = end + 1;
                continue;
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                while i < b.len() && matches!(b[i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                {
                    i += 1;
                }
                out.push_str(&number(&json[start..i]));
                continue;
            }
            b'{' => {
                stack.push(true);
                name = true;
            }
            b'[' => {
                stack.push(false);
                name = false;
            }
            b'}' | b']' => {
                stack.pop();
                name = false;
            }
            b',' => name = stack.last() == Some(&true),
            b':' => name = false,
            _ => {}
        }
        // Outside strings valid JSON is ASCII; anything else is copied whole, for the parser.
        let len = json[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&json[i..i + len]);
        i += len;
    }
    out
}

/// Whether `token` is a JSON number (RFC 8259 section 6).
fn is_json_number(token: &str) -> bool {
    let s = token.strip_prefix('-').unwrap_or(token);
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (int, frac) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    let digits = |d: &str| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit());
    digits(int)
        && (int == "0" || !int.starts_with('0'))
        && frac.is_none_or(digits)
        && exp.is_none_or(|e| digits(e.strip_prefix(['+', '-']).unwrap_or(e)))
}

/// `json` with every number rewritten as a marked string ([`NUMBER_MARK`]), and every string
/// value that starts with the mark with the mark doubled.
fn numbers_as_text(json: &str) -> String {
    let starts_marked = |content: &str| {
        content.starts_with(NUMBER_MARK)
            || content
                .get(..6)
                .is_some_and(|e| e.eq_ignore_ascii_case("\\ue000"))
    };
    rewrite_json(
        json,
        |content| starts_marked(content).then(|| format!("\"{NUMBER_MARK}{content}\"")),
        |token| {
            if is_json_number(token) {
                format!("\"{NUMBER_MARK}{token}\"")
            } else {
                token.to_string()
            }
        },
    )
}

/// The JSON `json` (as `serde_json` writes it) with each marked string written back as the
/// number it holds, and each doubled mark undone: the inverse of [`numbers_as_text`].
fn numbers_from_text(json: &str) -> String {
    rewrite_json(
        json,
        |content| {
            let rest = content.strip_prefix(NUMBER_MARK)?;
            Some(if rest.starts_with(NUMBER_MARK) {
                format!("\"{rest}\"")
            } else {
                rest.to_string()
            })
        },
        str::to_string,
    )
}

/// The number text a marked string holds ([`numbers_as_text`]), or a [`Value::Number`]'s.
fn number_text(v: &Value) -> Option<std::borrow::Cow<'_, str>> {
    match v {
        Value::Number(n) => Some(n.to_string().into()),
        Value::String(s) => s
            .strip_prefix(NUMBER_MARK)
            .filter(|rest| !rest.starts_with(NUMBER_MARK))
            .map(Into::into),
        _ => None,
    }
}

/// A decimal number's value as (negative, significant digits, power of ten): `1.50e1` and `15`
/// are both `(false, "15", 0)`, and every zero is `(false, "", 0)`.
fn decimal(s: &str) -> Option<(bool, String, i64)> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].parse::<i64>().ok()?),
        None => (s, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits: String = format!("{int}{frac}").trim_start_matches('0').to_string();
    let mut exp = exp.checked_sub(i64::try_from(frac.len()).ok()?)?;
    while digits.ends_with('0') {
        digits.pop();
        exp = exp.checked_add(1)?;
    }
    if digits.is_empty() {
        return Some((false, digits, 0));
    }
    Some((neg, digits, exp))
}

/// Whether some operation of the patch text `body` names a member twice (`{"op": "add", "op":
/// "remove"}`): a parsed [`Value`] keeps one of them silently, so the operation checked would not
/// be the one the client wrote. `body` is a patch already checked to be an array of objects.
fn repeats_a_member(body: &[u8]) -> bool {
    struct Members;
    impl<'de> serde::Deserialize<'de> for Members {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Visit;
            impl<'de> serde::de::Visitor<'de> for Visit {
                type Value = Members;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("an operation")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Members, A::Error> {
                    let mut seen = std::collections::HashSet::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if !seen.insert(key) {
                            return Err(serde::de::Error::custom("a member is repeated"));
                        }
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                    Ok(Members)
                }
            }
            d.deserialize_map(Visit)
        }
    }
    serde_json::from_slice::<Vec<Members>>(body).is_err()
}

/// Check that `ops` is an RFC 6902 patch document, without applying it.
pub fn validate_json_patch(ops: &Value) -> Result<&[Value], PatchError> {
    let ops = ops.as_array().ok_or(PatchError::Malformed(
        "a JSON Patch is an array of operations",
    ))?;
    let pointer = |v: Option<&Value>| v.and_then(Value::as_str).and_then(Pointer::parse).is_some();
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
    if ops.len() > MAX_PATCH_OPS {
        return Err(PatchError::TooLarge);
    }
    let mut doc = target.clone();
    // The document's size, kept as an upper bound: every operation that adds checks the budget
    // before it clones or inserts anything.
    let mut size = json_size(&doc);
    // The work done, in bytes measured, cloned or compared: a patch within the size budget can
    // still repeat costly operations (a copy of a large value over itself, again and again), so
    // the total is held to a budget of its own.
    let work_budget = budget.saturating_mul(PATCH_WORK_FACTOR).max(MIN_PATCH_WORK);
    let mut work = size;
    let charge = |work: &mut usize, bytes: usize| {
        *work = work.saturating_add(bytes);
        if *work > work_budget {
            return Err(PatchError::TooLarge);
        }
        Ok(())
    };
    // Every pointer was parsed when the patch was validated; it is parsed the same way here.
    let pointer_of = |op: &Value, k: &str| {
        op[k]
            .as_str()
            .and_then(Pointer::parse)
            .ok_or(PatchError::Malformed("a JSON Pointer is malformed"))
    };
    for op in ops {
        let path = pointer_of(op, "path")?;
        // What an add at `path` would overwrite: an existing object member is replaced.
        let replaced = |doc: &Value| -> usize {
            let Some((parent, _)) = path.split() else {
                return json_size(doc);
            };
            match (walk(doc, parent), path.get(doc)) {
                (Some(Value::Object(_)), Some(old)) => json_size(old),
                _ => 0,
            }
        };
        // A value placed at `path` sits under one container per pointer segment.
        let fits = |v: &Value| {
            if path.0.len() + json_depth(v) > MAX_JSON_DEPTH {
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
                let value = json_size(v);
                charge(&mut work, 2 * value + minus + shift_cost(&doc, &path, true))?;
                let by = value + member_overhead(&doc, &path, true);
                grow(&mut size, by, minus)?;
                pointer_add(&mut doc, &path, v.clone()).ok_or(Failed)?;
            }
            "remove" => {
                let overhead = member_overhead(&doc, &path, false);
                charge(&mut work, shift_cost(&doc, &path, false))?;
                let old = pointer_remove(&mut doc, &path).ok_or(Failed)?;
                let gone = json_size(&old);
                charge(&mut work, gone)?;
                size = size.saturating_sub(gone + overhead);
            }
            "replace" => {
                let v = &op["value"];
                fits(v)?;
                let old = path.get(&doc).ok_or(Failed)?;
                let minus = json_size(old);
                let value = json_size(v);
                // Taken out and put back: an array member shifts the rest of its array twice.
                let shifts = 2 * shift_cost(&doc, &path, false);
                charge(&mut work, 2 * value + minus + shifts)?;
                grow(&mut size, value, minus)?;
                pointer_remove(&mut doc, &path).ok_or(Failed)?;
                pointer_add(&mut doc, &path, v.clone()).ok_or(Failed)?;
            }
            "move" => {
                let from = pointer_of(op, "from")?;
                // A value cannot move into itself (RFC 6902 section 4.4).
                if from.0.len() < path.0.len() && path.0.starts_with(&from.0) {
                    return Err(Failed);
                }
                // The value moves; what changes is the member around it (its key, a separator), and
                // what an add at `path` replaces.
                let overhead = member_overhead(&doc, &from, false);
                charge(&mut work, shift_cost(&doc, &from, false))?;
                let v = pointer_remove(&mut doc, &from).ok_or(Failed)?;
                fits(&v)?;
                let moved = json_size(&v);
                size = size.saturating_sub(moved + overhead);
                let minus = replaced(&doc);
                charge(&mut work, 2 * moved + minus + shift_cost(&doc, &path, true))?;
                let by = moved + member_overhead(&doc, &path, true);
                grow(&mut size, by, minus)?;
                pointer_add(&mut doc, &path, v).ok_or(Failed)?;
            }
            "copy" => {
                let from = pointer_of(op, "from")?;
                let source = from.get(&doc).ok_or(Failed)?;
                fits(source)?;
                let copied = json_size(source);
                let added = copied + member_overhead(&doc, &path, true);
                let minus = replaced(&doc);
                charge(
                    &mut work,
                    2 * copied + minus + shift_cost(&doc, &path, true),
                )?;
                grow(&mut size, added, minus)?;
                let v = from.get(&doc).ok_or(Failed)?.clone();
                pointer_add(&mut doc, &path, v).ok_or(Failed)?;
            }
            "test" => {
                // A comparison walks no more of the document than the value it is given.
                charge(&mut work, json_size(&op["value"]))?;
                if !json_equal(path.get(&doc).ok_or(Failed)?, &op["value"]) {
                    return Err(Failed);
                }
            }
            _ => unreachable!("validated"),
        }
    }
    // The running size is an upper bound kept without serializing; the result itself is held to
    // the budget too.
    if json_size(&doc) > budget {
        return Err(PatchError::TooLarge);
    }
    Ok(doc)
}

/// The bytes a member at `path` of `doc` takes beside its value: in an object its key (quoted and
/// escaped) and colon, and in either container the comma between it and a sibling. `adding`: for a
/// member about to be added (nothing for an object member that would be replaced); otherwise for
/// the member there now.
fn member_overhead(doc: &Value, path: &Pointer, adding: bool) -> usize {
    let Some((parent, key)) = path.split() else {
        return 0;
    };
    let comma = |others: usize| usize::from(others > 0);
    match walk(doc, parent) {
        Some(Value::Object(m)) => {
            let present = m.contains_key(key);
            if adding && present {
                return 0;
            }
            let others = m.len() - usize::from(present);
            json_size(&Value::String(key.to_string())) + 1 + comma(others)
        }
        Some(Value::Array(a)) => comma(if adding {
            a.len()
        } else {
            a.len().saturating_sub(1)
        }),
        _ => 0,
    }
}

/// The work of the shift an insertion (`adding`) or a removal at `path` makes in the array that
/// holds it, if one does: every member after the index moves, each counted as the bytes a `Value`
/// takes in memory. An append, or a member of an object, moves nothing.
fn shift_cost(doc: &Value, path: &Pointer, adding: bool) -> usize {
    let Some((parent, key)) = path.split() else {
        return 0;
    };
    let Some(Value::Array(a)) = walk(doc, parent) else {
        return 0;
    };
    let Some(i) = array_index(key, a.len(), adding) else {
        return 0;
    };
    let moved = if adding {
        a.len().saturating_sub(i)
    } else {
        a.len().saturating_sub(i + 1)
    };
    moved.saturating_mul(std::mem::size_of::<Value>())
}

/// RFC 6902 section 4.6 equality: numbers are equal when their values are (`1` and `1.0`), strings
/// and literals when they are identical, arrays element by element, objects member by member
/// whatever their order. Integers compare exactly (two distinct large integers never meet through
/// a float), and an integer equals a float only when the float is exactly that integer.
fn json_equal(a: &Value, b: &Value) -> bool {
    // Numbers are equal when the decimal values their texts denote are.
    match (number_text(a), number_text(b)) {
        (Some(x), Some(y)) => {
            return match (decimal(&x), decimal(&y)) {
                (Some(x), Some(y)) => x == y,
                _ => x == y,
            }
        }
        (None, None) => {}
        _ => return false,
    }
    match (a, b) {
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

/// An RFC 6901 JSON Pointer, parsed once: its reference tokens, unescaped. Every JSON Patch
/// operation reads its paths through this one parser, so no operation accepts a pointer (or an
/// array index) that another refuses.
struct Pointer(Vec<String>);

impl Pointer {
    /// `None` for anything RFC 6901 does not allow: a non-empty pointer not starting with `/`,
    /// or a `~` not followed by `0` or `1`.
    fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return Some(Pointer(Vec::new()));
        }
        let tokens = s.strip_prefix('/')?.split('/').map(|t| {
            let mut out = String::with_capacity(t.len());
            let mut chars = t.chars();
            while let Some(c) = chars.next() {
                out.push(if c != '~' {
                    c
                } else {
                    match chars.next()? {
                        '0' => '~',
                        '1' => '/',
                        _ => return None,
                    }
                });
            }
            Some(out)
        });
        tokens.collect::<Option<Vec<_>>>().map(Pointer)
    }

    /// The tokens of the container and the last token; `None` for the whole document.
    fn split(&self) -> Option<(&[String], &str)> {
        let (last, parent) = self.0.split_last()?;
        Some((parent, last))
    }

    fn get<'a>(&self, doc: &'a Value) -> Option<&'a Value> {
        walk(doc, &self.0)
    }
}

/// The value `tokens` reference in `doc`.
fn walk<'a>(doc: &'a Value, tokens: &[String]) -> Option<&'a Value> {
    tokens.iter().try_fold(doc, |v, t| match v {
        Value::Object(m) => m.get(t),
        Value::Array(a) => a.get(array_index(t, a.len(), false)?),
        _ => None,
    })
}

fn walk_mut<'a>(doc: &'a mut Value, tokens: &[String]) -> Option<&'a mut Value> {
    tokens.iter().try_fold(doc, |v, t| match v {
        Value::Object(m) => m.get_mut(t),
        Value::Array(a) => {
            let i = array_index(t, a.len(), false)?;
            a.get_mut(i)
        }
        _ => None,
    })
}

/// An array index token (RFC 6901 section 4): `0`, or digits without a leading zero, below `len`;
/// when `adding`, also `len` itself and `-` (the end). No sign, no leading zero, no overflow.
fn array_index(token: &str, len: usize, adding: bool) -> Option<usize> {
    if adding && token == "-" {
        return Some(len);
    }
    let b = token.as_bytes();
    let well_formed = token == "0"
        || (matches!(b.first(), Some(b'1'..=b'9')) && b.iter().all(u8::is_ascii_digit));
    let i: usize = token.parse().ok().filter(|_| well_formed)?;
    (i < len || (adding && i == len)).then_some(i)
}

fn pointer_add(doc: &mut Value, path: &Pointer, value: Value) -> Option<()> {
    let Some((parent, key)) = path.split() else {
        *doc = value;
        return Some(());
    };
    match walk_mut(doc, parent)? {
        Value::Object(m) => {
            m.insert(key.to_string(), value);
        }
        Value::Array(a) => {
            let i = array_index(key, a.len(), true)?;
            a.insert(i, value);
        }
        _ => return None,
    }
    Some(())
}

fn pointer_remove(doc: &mut Value, path: &Pointer) -> Option<Value> {
    let Some((parent, key)) = path.split() else {
        return Some(std::mem::replace(doc, Value::Null));
    };
    match walk_mut(doc, parent)? {
        Value::Object(m) => m.remove(key),
        Value::Array(a) => {
            let i = array_index(key, a.len(), false)?;
            Some(a.remove(i))
        }
        _ => None,
    }
}

/// A patch document a request carries.
enum Patch {
    /// RFC 6902 JSON Patch, already checked to be well-formed.
    Json(Value),
}

impl Patch {
    /// Parse the request body by its Content-Type. `Err` carries the response: 415 for another
    /// format, 400 for a body that is not JSON or not a well-formed patch.
    fn parse(req: &LwsRequest) -> Result<Self, Response> {
        let ct = req.content_type().unwrap_or_default();
        if ct != JSON_PATCH {
            let mut r = problem(StatusCode::UNSUPPORTED_MEDIA_TYPE, None);
            set(
                r.headers_mut(),
                header::HeaderName::from_static("accept-patch"),
                ACCEPT_PATCH,
            );
            return Err(r);
        }
        let Some(patch) = std::str::from_utf8(&req.body)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&numbers_as_text(text)).ok())
        else {
            return Err(problem(
                StatusCode::BAD_REQUEST,
                Some("the patch is not JSON"),
            ));
        };
        if let Err(PatchError::Malformed(why)) = validate_json_patch(&patch) {
            return Err(problem(StatusCode::BAD_REQUEST, Some(why)));
        }
        if repeats_a_member(&req.body) {
            return Err(problem(
                StatusCode::BAD_REQUEST,
                Some("an operation repeats a member"),
            ));
        }
        Ok(Patch::Json(patch))
    }

    /// Whether applying the patch observes the target's content (see [`json_patch_reads`]).
    fn reads_content(&self) -> bool {
        matches!(self, Patch::Json(ops) if json_patch_reads(ops))
    }

    /// Apply the patch to `target` within `budget` serialized bytes. `Err` carries the response.
    fn apply(&self, target: &Value, budget: usize) -> Result<Value, Response> {
        match self {
            Patch::Json(ops) => json_patch(target, ops, budget),
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
    let Some(guard) = state.locks.lock(uri).await else {
        return set_aside_meanwhile();
    };
    let listing = match listing_guard(state, uri).await {
        Ok(listing) => listing,
        Err(r) => return r,
    };
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
    // What the request is (its patch format, and the resource's) is settled before its
    // preconditions: a patch that could never apply is a 415 whatever its validators.
    let patch = match Patch::parse(req) {
        Ok(p) => p,
        Err(r) => return r,
    };
    // A JSON Patch applies to a JSON document only: a resource stored as another format is
    // never rewritten as JSON.
    let not_json = || {
        let mut r = problem(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Some("the resource is not JSON"),
        );
        set(
            r.headers_mut(),
            header::HeaderName::from_static("accept-patch"),
            ACCEPT_PATCH,
        );
        r
    };
    if !is_json_type(&meta.content_type) {
        return not_json();
    }
    if let Precondition::Failed | Precondition::NotModified = evaluate(
        &req.headers,
        Some(&quoted(&meta.etag)),
        meta.last_modified.map(|t| to_secs(epoch_ms(t))),
        false,
    ) {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    if let Err(r) = patch_read_check(state, &patch, uri, agent).await {
        return r;
    }
    let body = match state.store.read_at(uri, &meta).await {
        Ok(b) => b,
        Err(e) => return store_error(e),
    };
    // Its numbers are held as their text ([`NUMBER_MARK`]) while the patch applies, so the
    // content is written back with every number it does not touch as it was.
    let target = if body.is_empty() {
        Some(Value::Object(Map::new()))
    } else {
        std::str::from_utf8(&body)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&numbers_as_text(text)).ok())
    };
    let Some(target) = target else {
        return not_json();
    };
    let patched = match patch.apply(&target, patch_budget(state)) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let patched = numbers_from_text(&serde_json::to_string(&patched).unwrap_or_default());
    let ct = meta.content_type.clone();
    // The linkset is left alone: a PATCH changes the content only.
    // The listing lock goes with the resource's into the writes' task (see [`hold_locks`]),
    // which touches the container once they are over.
    let (written, _guard, _) = write_with_meta(
        state,
        (guard, listing),
        uri,
        Bytes::from(patched),
        &ct,
        None,
        req.admission.clone(),
    )
    .await;
    let written = match written {
        Ok(m) => m,
        Err(e) => return store_error(e),
    };
    let mut resp = StatusCode::NO_CONTENT.into_response();
    set(resp.headers_mut(), header::ETAG, &quoted(&written.etag));
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
        Err(r) => return r,
    };
    // The container's lock is exclusive: no listing of it is read while the members are gone
    // and their removal may yet be put back.
    let listing = match listing_lock(state, uri).await {
        Ok(listing) => listing,
        Err(r) => return r,
    };
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
        // The whole subtree is held: no change to a member is in flight.
        let listing = read_container(state, &plain_get(req), uri, &meta, true)
            .await
            .unwrap_or_else(|_| set_aside_meanwhile());
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
    // The removals run in a task that holds the subtree's locks, and the listing lock, until
    // they are over, and then touches the container (see [`hold_locks`]). A removal that was
    // put back changed nothing; what could not be is in `removed`.
    let removal = {
        let state = state.clone();
        async move {
            let (removed, outcome, left) = remove(&state, &doomed).await;
            ((removed, outcome), left)
        }
    };
    let touch = move |(removed, _): &(Vec<String>, _)| parent.filter(|_| !removed.is_empty());
    let locks = (guards, listing);
    let outcome = match hold_locks(state, locks, req.admission.clone(), touch, removal).await {
        Ok(((_, outcome), _)) => outcome,
        Err(e) => return store_error(e),
    };
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
) -> Result<(Vec<IriGuard>, Vec<(String, Option<String>)>), Response> {
    let sorted = |tree: &[(String, Option<String>)]| {
        let mut iris: Vec<String> = tree.iter().map(|(n, _)| n.clone()).collect();
        iris.sort_by(|a, b| lock_order(a, b));
        iris.dedup();
        iris
    };
    for _ in 0..8 {
        let listed = subtree(state, uri, parent.clone()).await;
        let before = sorted(&listed.map_err(store_error)?);
        let mut guards = Vec::with_capacity(before.len());
        for iri in &before {
            guards.push(
                state
                    .locks
                    .lock(iri)
                    .await
                    .ok_or_else(set_aside_meanwhile)?,
            );
        }
        let after = subtree(state, uri, parent.clone())
            .await
            .map_err(store_error)?;
        if sorted(&after) == before {
            return Ok((guards, after));
        }
    }
    Err(store_error(ServerError::Conflict(
        "the container kept changing while it was being deleted".into(),
    )))
}

/// Remove the resources of a locked [`subtree`], members before their containers, each before its
/// metadata (which says who may act on it, so it goes only once the resource has), through one
/// [`Journal`](super::Journal): the delete is whole or not at all. At the first failure every
/// removal before it is put back (see [`Journal::rollback`](super::Journal::rollback)), with
/// the subtree's locks held. Returns which of `doomed` are gone (all of them, or none), the
/// outcome, and what could not be put back (when something could not, every one of `doomed`
/// counts as gone). A subtree too large to put back is refused (409) before anything is removed.
async fn remove<S: Store + 'static>(
    state: &LwsState<S>,
    doomed: &[(String, Option<String>)],
) -> (
    Vec<String>,
    Result<(), ServerError>,
    Option<super::Unsettled>,
) {
    remove_in(state.journal(), doomed).await
}

/// [`remove`], recorded in `journal`.
async fn remove_in<S: Store + 'static>(
    mut journal: super::Journal<'_, S>,
    doomed: &[(String, Option<String>)],
) -> (
    Vec<String>,
    Result<(), ServerError>,
    Option<super::Unsettled>,
) {
    // Everything the removal could need to put back is read first: a subtree too large to
    // remove atomically is refused before any of it is removed.
    for (node, parent) in doomed {
        let staged = match journal.stage_member(node, parent.as_deref()).await {
            Ok(()) => journal.stage(&meta_key(node)).await,
            Err(e) => Err(e),
        };
        if let Err(e) = staged {
            return (Vec::new(), Err(e), None);
        }
    }
    let mut failed = None;
    for (node, parent) in doomed {
        // The record and its parent's membership edge go in one step, data resources included:
        // removed one after the other, a failure in between would leave a live resource its
        // container no longer lists, which a retried recursive delete would not find.
        let removed = match journal.remove_member(node, parent.as_deref()).await {
            Ok(crate::store::DeleteOutcome::NotEmpty) => Err(ServerError::Conflict(
                "the container gained a member while it was deleted".into(),
            )),
            Ok(_) => journal.delete_meta(node).await,
            Err(e) => Err(e),
        };
        if let Err(e) = removed {
            failed = Some(e);
            break;
        }
    }
    let all = || doomed.iter().map(|(n, _)| n.clone()).collect();
    // What the delete had not reached is locked too, and is set aside with the rest: a request
    // or a walk skips it rather than waits on its lock.
    let locked = |mut left: super::Unsettled| {
        left.0.extend(
            doomed
                .iter()
                .map(|(node, _)| super::Undo::Locked { iri: node.clone() }),
        );
        left
    };
    match failed {
        None => match journal
            .commit(doomed.last().and_then(|(_, p)| p.as_deref()))
            .await
        {
            Ok(()) => (all(), Ok(()), None),
            Err((e, None)) => (Vec::new(), Err(e), None),
            Err((e, Some(left))) => (all(), Err(e), Some(locked(left))),
        },
        Some(e) => match journal.rollback().await {
            None => (Vec::new(), Err(e), None),
            Some(left) => (all(), Err(e), Some(locked(left))),
        },
    }
}

// ---- linksets ----

/// Relations the server maintains in a resource's own linkset entry. The metadata section lists
/// `linkset`, `type`, `format`, `size` and `modified` as system managed and read-only, and lets a
/// server restrict `up`; `self` carries the representation's format, size and modification time.
const SERVER_MANAGED: &[&str] = &["up", "type", "self", "linkset"];

/// Whether a linkset entry is about `uri`. Anchors are absolute: every document the server keeps
/// was resolved by [`absolute_linkset`] (or built with absolute anchors).
fn anchored_at(entry: &Value, uri: &str) -> bool {
    entry.get("anchor").and_then(Value::as_str) == Some(uri)
}

/// Whether a link target attribute has the shape RFC 9264 section 4.2.4 gives it: `href`,
/// `title`, `type` and `media` a string; `hreflang` an array of strings; an internationalised
/// attribute (`title*`, any `name*`) an array of `{"value", "language"?}` objects of strings;
/// any other (extension) attribute an array of strings.
fn target_attribute_ok(key: &str, value: &Value) -> bool {
    let strings = |v: &Value| v.as_array().is_some_and(|a| a.iter().all(Value::is_string));
    match key {
        "href" | "title" | "type" | "media" => value.is_string(),
        "hreflang" => strings(value),
        // One or more value objects (RFC 9264 section 4.2.4.2).
        k if k.ends_with('*') => value.as_array().is_some_and(|a| {
            !a.is_empty()
                && a.iter().all(|o| {
                    o.as_object().is_some_and(|o| {
                        o.get("value").is_some_and(Value::is_string)
                            && o.get("language").is_none_or(Value::is_string)
                            && o.keys().all(|k| k == "value" || k == "language")
                    })
                })
        }),
        _ => strings(value),
    }
}

/// Why [`absolute_linkset`] refused a document.
#[derive(Debug, PartialEq)]
enum Unresolved {
    /// An anchor or href is not a URI reference, or a target attribute is misshapen.
    Invalid,
    /// The document passes a cap, or resolving it could pass the budget ([`linkset_cost`]).
    TooLarge,
}

/// How many entries a linkset document a client writes may hold.
const MAX_LINKSET_ENTRIES: usize = 256;
/// How many link targets, over all its entries and relations, it may hold.
const MAX_LINKSET_TARGETS: usize = 1024;
/// How long an anchor, a relation and an href in it may each be, in bytes.
const MAX_LINKSET_FIELD: usize = 4096;
/// What [`linkset_cost`] charges each value the document holds (an entry, a relation, a target,
/// an attribute, an element of an attribute's array) beyond its serialized bytes: what a value
/// and its allocation cost, however short its serialized form.
const LINKSET_OVERHEAD: usize = 64;

/// How many values `v` holds, itself included. Iterative, so it is safe on any document.
fn json_values(v: &Value) -> usize {
    let mut n = 0;
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        n += 1;
        match v {
            Value::Array(a) => stack.extend(a),
            Value::Object(o) => stack.extend(o.values()),
            _ => {}
        }
    }
    n
}

/// The most that resolving a linkset document against a base of `base_len` bytes, and then
/// deriving its indexed links ([`links_of`]), can allocate. Every string the document holds is
/// copied at most twice (into the resolved document, and into the indexed links), so its
/// serialized size is charged twice; every value it holds is charged [`LINKSET_OVERHEAD`]
/// whatever its length, so empty relations and long attribute arrays count; and every anchor
/// and href may grow by the base when it is resolved. Computed from the parsed document before
/// any of that work is done; `None` when the document passes a cap (entries, targets, or an
/// anchor's, relation's or href's length).
fn linkset_cost(doc: &Value, base_len: usize) -> Option<usize> {
    let entries = doc
        .get("linkset")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if entries.len() > MAX_LINKSET_ENTRIES || base_len > MAX_LINKSET_FIELD * 2 {
        return None;
    }
    let too_long = |v: &Value| v.as_str().is_some_and(|s| s.len() > MAX_LINKSET_FIELD);
    let mut targets = 0usize;
    for entry in entries {
        for (k, v) in entry.as_object().into_iter().flatten() {
            if k == "anchor" {
                if too_long(v) {
                    return None;
                }
                continue;
            }
            if k.len() > MAX_LINKSET_FIELD {
                return None;
            }
            for t in v.as_array().into_iter().flatten() {
                targets += 1;
                if t.get("href").is_some_and(too_long) {
                    return None;
                }
            }
        }
    }
    if targets > MAX_LINKSET_TARGETS {
        return None;
    }
    let resolved = (entries.len() + targets).checked_mul(base_len)?;
    json_size(doc)
        .checked_mul(2)?
        .checked_add(json_values(doc).checked_mul(LINKSET_OVERHEAD)?)?
        .checked_add(resolved)
}

/// A linkset document whose every `anchor` and `href` is absolute: made by [`absolute_linkset`]
/// from what a client wrote, or taken from what the server stored or built itself
/// ([`Resolved::stored`]). Its links are used as they are and never resolved again
/// ([`links_of`]).
#[derive(Clone, Debug, PartialEq)]
struct Resolved(Value);

impl Resolved {
    /// A linkset the server stored or built: what a client wrote was resolved by
    /// [`absolute_linkset`] before it was stored, and Link header targets by
    /// [`link_declared`], so every link in it is absolute.
    fn stored(doc: Value) -> Self {
        Self(doc)
    }

    fn into_value(self) -> Value {
        self.0
    }
}

/// A linkset document with every `anchor` and `href` resolved against `base`, the linkset's own
/// URI (RFC 9264 section 4: the context of relative references in a linkset is the resource that
/// delivers it, not the resource it describes). [`Unresolved::Invalid`] when one is not a URI
/// reference (RFC 3986), or a target attribute has not the shape RFC 9264 gives it
/// ([`target_attribute_ok`]), which the document may not hold. Before anything is resolved, the
/// document is held to the caps and its [`linkset_cost`] to `budget`: past either it is
/// [`Unresolved::TooLarge`], and nothing is built.
fn absolute_linkset(doc: &Value, base: &str, budget: usize) -> Result<Resolved, Unresolved> {
    use Unresolved::Invalid;
    if linkset_cost(doc, base.len()).is_none_or(|cost| cost > budget) {
        return Err(Unresolved::TooLarge);
    }
    let base = oxiri::Iri::parse(base).map_err(|_| Invalid)?;
    let resolve = |v: &Value| -> Result<Value, Unresolved> {
        let resolved = base
            .resolve(v.as_str().ok_or(Invalid)?)
            .map_err(|_| Invalid)?;
        Ok(Value::String(resolved.into_inner()))
    };
    let mut entries = Vec::new();
    for entry in doc
        .get("linkset")
        .and_then(Value::as_array)
        .ok_or(Invalid)?
    {
        let mut out = Map::new();
        for (k, v) in entry.as_object().ok_or(Invalid)? {
            let v = if k == "anchor" {
                resolve(v)?
            } else {
                let mut targets = Vec::new();
                for t in v.as_array().ok_or(Invalid)? {
                    let t = t.as_object().ok_or(Invalid)?;
                    if !t.iter().all(|(k, v)| target_attribute_ok(k, v)) {
                        return Err(Invalid);
                    }
                    let href = resolve(t.get("href").ok_or(Invalid)?)?;
                    let mut t = t.clone();
                    t.insert("href".into(), href);
                    targets.push(Value::Object(t));
                }
                Value::Array(targets)
            };
            out.insert(k.clone(), v);
        }
        entries.push(Value::Object(out));
    }
    Ok(Resolved(json!({"linkset": entries})))
}

/// The user-managed part of a linkset document: every server-managed relation dropped from the
/// entries about `uri`. A client cannot write those relations; whatever a patch puts there is
/// ignored and the server's own values stand.
fn user_linkset(doc: &Resolved, uri: &str) -> Resolved {
    let entries = doc
        .0
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
    Resolved(json!({"linkset": kept}))
}

/// The user-managed links a linkset document holds about `uri`, by relation: what the type
/// index matches relations against, kept equal to the document. Its links are absolute already
/// ([`Resolved`]) and are taken as they are. Repeats are found per relation, borrowing the
/// document's own strings, so a relation is copied once for each entry it is in and an href
/// once.
fn links_of(doc: &Resolved, uri: &str) -> Links {
    let mut links = Links::new();
    let mut seen: std::collections::HashMap<String, std::collections::HashSet<&str>> =
        Default::default();
    for entry in doc
        .0
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
            let seen = seen.entry(key.clone()).or_default();
            let out = links.entry(key).or_default();
            for href in targets
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.get("href").and_then(Value::as_str))
            {
                if seen.insert(href) {
                    out.push(href.to_string());
                }
            }
        }
    }
    links.retain(|_, hrefs| !hrefs.is_empty());
    links
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
        .map(|d| user_linkset(&Resolved::stored(d), uri).into_value())
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

/// A resource's linkset (RFC 9264): GET, HEAD and PATCH (JSON Patch). PUT is
/// not offered, so it is 405 with the methods that are.
async fn linkset<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    uri: &str,
) -> Response {
    let action = if matches!(req.method, Method::GET | Method::HEAD | Method::OPTIONS) {
        Action::Read
    } else {
        Action::Modify
    };
    // A request is authorized once before it waits for the resource's lock, so one that may not
    // touch the resource never queues behind a write (holding its admission slot while it waits).
    if let Some(refused) = authorize_unlocked(state, action, uri, agent).await {
        return refused;
    }
    // The resource's lock is taken before the permission check that counts, so the decision holds
    // for what is served or changed: shared for a read, exclusive for a patch (from the
    // precondition through the write).
    let (_shared, mut exclusive) = if req.method == Method::PATCH {
        match state.locks.lock(uri).await {
            Some(guard) => (None, Some(guard)),
            None => return set_aside_meanwhile(),
        }
    } else {
        match state.locks.read(uri).await {
            Some(guard) => (Some(guard), None),
            None => return set_aside_meanwhile(),
        }
    };
    let mut listing = if req.method == Method::PATCH {
        match listing_guard(state, uri).await {
            Ok(listing) => listing,
            Err(r) => return r,
        }
    } else {
        None
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
            // The patch format is settled before the preconditions, as for a data resource.
            let patch = match Patch::parse(req) {
                Ok(p) => p,
                Err(r) => return r,
            };
            if let Precondition::Failed | Precondition::NotModified =
                evaluate(&req.headers, Some(&etag), None, false)
            {
                return problem(StatusCode::PRECONDITION_FAILED, None);
            }
            if let Err(r) = patch_read_check(state, &patch, uri, agent).await {
                return r;
            }
            let patched = match patch.apply(&document, patch_budget(state)) {
                Ok(v) => v,
                Err(r) => return r,
            };
            if !valid_linkset(&patched) {
                return problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("the result is not a linkset document"),
                );
            }
            // Relative references are resolved once, here, against the linkset's own URI: what
            // is stored, served and indexed is then the same absolute link.
            let base = format!("{uri}{META_SUFFIX}");
            let patched = match absolute_linkset(&patched, &base, patch_budget(state)) {
                Ok(p) => p,
                Err(Unresolved::Invalid) => return problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("an anchor or href is not a URI reference, or a target attribute is not shaped as RFC 9264 says"),
                ),
                Err(Unresolved::TooLarge) => return problem(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Some("the patched document would be too large"),
                ),
            };
            // Only the user-managed part is kept; the links the type index matches are taken from
            // it, so the two never drift apart.
            let user = user_linkset(&patched, uri);
            meta.links = links_of(&user, uri);
            meta.linkset = Some(user.into_value());
            meta.linkset_etag = None;
            // The size is checked on the document as it will be served: the server-managed links
            // (`up`, `type`, `self`) a patch may strip are put back, and they count too.
            let rebuilt = match linkset_document(state, uri, &meta).await {
                Ok(d) => d,
                Err(e) => return store_error(e),
            };
            if serde_json::to_vec(&rebuilt).map_or(true, |b| b.len() > patch_budget(state)) {
                return problem(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Some("the patched document would be too large"),
                );
            }
            // The linkset is checked whole, as it will be stored: inside the metadata it nests a
            // level deeper than in the patched document, and metadata that does not read back
            // would be lost.
            if super::encode_meta(&meta).is_err() {
                return problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("the linkset nests too deeply to be stored"),
                );
            }
            // The tag of the document as written, which `rebuilt` is.
            let new_etag = linkset_etag(&rebuilt);
            // The write runs in a task that holds the lock and the listing's, and then touches
            // the container when the write may have landed (see [`hold_locks`]).
            let Some(guard) = exclusive.take() else {
                return store_error(ServerError::Storage("the lock is not held".into()));
            };
            let write = {
                let (state, uri, meta) = (state.clone(), uri.to_string(), meta.clone());
                async move { (state.put_resource_meta(&uri, &meta).await, None) }
            };
            let parent = parent_of(uri, &state.cfg.storage());
            let touch = move |w: &Result<(), ServerError>| {
                parent.filter(|_| matches!(w, Ok(()) | Err(ServerError::Storage(_))))
            };
            let locks = (guard, listing.take());
            match hold_locks(state, locks, req.admission.clone(), touch, write).await {
                Ok((Ok(()), _)) => {}
                Ok((Err(e), _)) | Err(e) => return store_error(e),
            }
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
        assert!(if_range_holds(None, "\"a\""));
        assert!(if_range_holds(Some("\"a\""), "\"a\""));
        assert!(!if_range_holds(Some("\"b\""), "\"a\""));
        assert!(!if_range_holds(Some("W/\"a\""), "\"a\""));
        // Review finding: a date is not a strong validator (two versions can share one), so
        // even one equal to Last-Modified serves the whole representation.
        assert!(!if_range_holds(Some(&date), "\"a\""));
        assert!(!if_range_holds(Some("garbage"), "\"a\""));
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
        // A comma inside a tag is part of it, never a wildcard.
        assert!(!etag_listed("\"x,*,y\"", "\"x\"", false));
        assert!(etag_listed("\"x,*,y\"", "\"x,*,y\"", false));
        assert!(!etag_listed("\"a\", *", "\"x\"", false));
        assert!(!etag_listed("\"a", "\"a\"", false));
        assert!(!etag_listed("a", "a", false));
        // An empty list matches nothing, and is not unreadable.
        assert!(!etag_listed("", "\"x\"", true));
        assert!(TagList::parse(b" , ").is_ok());
        // obs-text is allowed inside an opaque tag.
        let mut obs = b"\"\xe9\", ".to_vec();
        obs.extend_from_slice(b"\"x\"");
        assert!(TagList::parse(&obs).is_ok_and(|l| l.matches("\"x\"", false)));
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

    /// A JSON Patch replacing the `linkset` array with `doc`'s.
    fn replace_linkset(doc: &Value) -> String {
        json!([{"op": "replace", "path": "/linkset", "value": doc["linkset"]}]).to_string()
    }

    /// A linkset PATCH in another format is a 415 whatever its validators, and a JSON Patch with
    /// a stale one a 412.
    #[tokio::test]
    async fn a_linkset_patch_checks_its_format_before_its_preconditions() {
        let st = state().await;
        let uri = post(&st, "f.txt", "text/plain", "x", &[]).await;
        let meta = format!("{}{META_SUFFIX}", path_of(&uri));
        let stale = ("if-match", "\"stale\"");
        let ops =
            r#"[{"op":"add","path":"/linkset/0/license","value":[{"href":"https://ex.org/l"}]}]"#;
        let r = call(
            &st,
            "PATCH",
            &meta,
            &[("content-type", "application/merge-patch+json"), stale],
            "{}",
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let r = call(
            &st,
            "PATCH",
            &meta,
            &[("content-type", JSON_PATCH), stale],
            ops,
        )
        .await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        let r = call(&st, "PATCH", &meta, &[("content-type", JSON_PATCH)], ops).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
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
        // A patch replacing the whole array: a forged parent and type, and a new link only.
        let patch = json!({"linkset": [{"anchor": uri, "up": [{"href": "https://forged/"}],
            "type": [{"href": "https://forged/T"}], "describedby": [{"href": "https://ex.org/schema"}]}]});
        let r = call(
            &st,
            "PATCH",
            &meta,
            &[("content-type", JSON_PATCH)],
            &replace_linkset(&patch),
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
        let doc = absolute_linkset(
            &json!({"linkset": [
                {"anchor": "http://h/a", "up": [{"href": "x"}], "self": [{"href": "y"}], "license": [{"href": "l"}]},
                {"anchor": "http://h/b", "up": [{"href": "z"}]},
                {"anchor": "/a", "type": [{"href": "t"}]},
            ]}),
            "http://h/a.meta",
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            user_linkset(&doc, "http://h/a").into_value(),
            json!({"linkset": [
                {"anchor": "http://h/a", "license": [{"href": "http://h/l"}]},
                {"anchor": "http://h/b", "up": [{"href": "http://h/z"}]},
            ]})
        );
        let links = links_of(&doc, "http://h/a");
        assert_eq!(links.keys().collect::<Vec<_>>(), vec!["license"]);
        assert_eq!(links["license"], vec!["http://h/l".to_string()]);
    }

    /// Review finding: relative references in a patched linkset were resolved against the
    /// resource, while clients resolve them against the linkset that delivers them (RFC 9264
    /// section 4), and malformed references were kept and served.
    #[tokio::test]
    async fn linkset_references_resolve_against_the_linkset() {
        let st = state().await;
        let uri = post(&st, "doc", "text/plain", "x", &[]).await;
        let meta = format!("{}{META_SUFFIX}", path_of(&uri));
        let linkset = format!("{uri}{META_SUFFIX}");
        let patch = json!({"linkset": [
            {"anchor": uri, "license": [{"href": "#license"}]},
            {"anchor": "", "author": [{"href": "https://ex.org/a"}]},
        ]});
        let r = call(
            &st,
            "PATCH",
            &meta,
            &[("content-type", JSON_PATCH)],
            &replace_linkset(&patch),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let doc = json_of(call(&st, "GET", &meta, &[], "").await).await;
        let entries = doc["linkset"].as_array().unwrap();
        let about = |a: &str| entries.iter().find(|e| e["anchor"] == a).unwrap().clone();
        // What is served and what the type index matches name the same link.
        assert_eq!(
            about(&uri)["license"][0]["href"],
            format!("{linkset}#license")
        );
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(m.links["license"], vec![format!("{linkset}#license")]);
        // An empty anchor is the linkset itself, not the resource it describes.
        assert_eq!(about(&linkset)["author"][0]["href"], "https://ex.org/a");
        assert!(!m.links.contains_key("author"));
        let before = json_of(call(&st, "GET", &meta, &[], "").await).await;
        for bad in [
            json!({"linkset": [{"anchor": "http://[", "license": [{"href": "https://ex.org/l"}]}]}),
            json!({"linkset": [{"anchor": uri, "license": [{"href": "%ZZ"}]}]}),
            // Review finding: target attributes are held to the shapes RFC 9264 gives them.
            json!({"linkset": [{"anchor": uri, "license": [{"href": "https://ex.org/l", "title": 123}]}]}),
            json!({"linkset": [{"anchor": uri, "license": [{"href": "https://ex.org/l", "hreflang": "en"}]}]}),
            json!({"linkset": [{"anchor": uri, "license": [{"href": "https://ex.org/l", "title*": [{"value": 1}]}]}]}),
            json!({"linkset": [{"anchor": uri, "license": [{"href": "https://ex.org/l", "title*": []}]}]}),
            json!({"linkset": [{"anchor": uri, "license": [{"href": "https://ex.org/l", "ext": "x"}]}]}),
        ] {
            let r = call(
                &st,
                "PATCH",
                &meta,
                &[("content-type", JSON_PATCH)],
                &replace_linkset(&bad),
            )
            .await;
            assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        }
        // The same through a JSON Patch.
        for target in [
            json!({"href": "https://ex.org/l", "hreflang": "en"}),
            json!({"href": "https://ex.org/l", "title*": []}),
        ] {
            let bad = json!([{"op": "replace", "path": "/linkset", "value": [{"anchor": uri,
                "license": [target]}]}]);
            let r = call(
                &st,
                "PATCH",
                &meta,
                &[("content-type", JSON_PATCH)],
                &bad.to_string(),
            )
            .await;
            assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        }
        // None of them changed the metadata.
        assert_eq!(
            json_of(call(&st, "GET", &meta, &[], "").await).await,
            before
        );
        // Well-shaped attributes are kept.
        let good = json!({"linkset": [{"anchor": uri, "license": [{"href": "#license",
            "title": "L", "hreflang": ["en"], "title*": [{"value": "L", "language": "en"}]}]}]});
        let r = call(
            &st,
            "PATCH",
            &meta,
            &[("content-type", JSON_PATCH)],
            &replace_linkset(&good),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let m = st.resource_meta(&uri).await.unwrap();
        assert_eq!(m.links["license"], vec![format!("{linkset}#license")]);
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
            (
                "PATCH",
                JSON_PATCH,
                "[{\"op\": \"remove\", \"path\": \"/pin\"}]",
            ),
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
                &[("content-type", JSON_PATCH)],
                "[{\"op\": \"add\", \"path\": \"/bob\", \"value\": 1}]",
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

    /// Review finding: a gzip body was stored as the representation and its Content-Encoding
    /// dropped, so a reader got compressed bytes labelled as plain content. Coded bodies are
    /// refused before anything is written.
    #[tokio::test]
    async fn coded_bodies_are_refused() {
        let st = state().await;
        let text = ("content-type", "text/plain");
        let r = call(&st, "POST", "/", &[("slug", "x"), text], "a").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        for coding in ["gzip", "identity, gzip", "x-unknown"] {
            let h = [text, ("content-encoding", coding), ("slug", "y")];
            for (method, path) in [("PUT", "/x"), ("POST", "/")] {
                let r = call(&st, method, path, &h, "b").await;
                assert_eq!(
                    r.status(),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "{method} {coding}"
                );
                assert_eq!(hdr(&r, "accept-encoding"), "identity");
            }
        }
        let r = call(&st, "GET", "/x", &[], "").await;
        assert_eq!(&body_of(r).await[..], b"a");
        assert_eq!(
            call(&st, "GET", "/y", &[], "").await.status(),
            StatusCode::NOT_FOUND
        );
        // Every route that takes a body, the token endpoint included.
        let form = ("content-type", "application/x-www-form-urlencoded");
        let h = [form, ("content-encoding", "gzip")];
        let r = call(&st, "POST", super::super::AS_TOKEN_PATH, &h, "grant_type=x").await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let h = [text, ("content-encoding", "identity")];
        assert!(call(&st, "PUT", "/x", &h, "c").await.status().is_success());
    }

    /// Review finding: only the first field line of a conditional header was read, so a second
    /// `If-None-Match: *` line let a write through. Every line counts now, an entity-tag list
    /// that cannot be read fails the request, and a date that is not one valid date is ignored.
    #[tokio::test]
    async fn every_line_of_a_precondition_counts() {
        let st = state().await;
        let text = ("content-type", "text/plain");
        let r = call(&st, "POST", "/", &[("slug", "x"), text], "a").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let tag = hdr(&call(&st, "GET", "/x", &[], "").await, "etag");
        for refused in [
            &[("if-none-match", "\"other\""), ("if-none-match", "*")][..],
            &[("if-match", "\"other\""), ("if-match", "\"other2\"")][..],
            &[("if-match", "\"caf\u{e9}\"")][..],
            &[("if-none-match", "\"other\""), ("if-none-match", "\u{e9}")][..],
        ] {
            let mut h = vec![text];
            h.extend_from_slice(refused);
            let r = call(&st, "PUT", "/x", &h, "b").await;
            assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED, "{refused:?}");
        }
        let r = call(&st, "GET", "/x", &[], "").await;
        assert_eq!(hdr(&r, "etag"), tag, "a refused write changed the resource");
        let lines = [
            ("if-none-match", "\"other\""),
            ("if-none-match", tag.as_str()),
        ];
        let r = call(&st, "GET", "/x", &lines, "").await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        // A date sent twice is not a valid HTTP-date, and is ignored (RFC 9110 section 13.1.4).
        let past = "Thu, 01 Jan 1970 00:00:00 GMT";
        let lines = [
            ("if-unmodified-since", past),
            ("if-unmodified-since", past),
            text,
        ];
        let r = call(&st, "PUT", "/x", &lines, "b").await;
        assert!(r.status().is_success(), "{}", r.status());
        let tag = hdr(&call(&st, "GET", "/x", &[], "").await, "etag");
        let lines = [("if-match", "\"other\""), ("if-match", tag.as_str()), text];
        let r = call(&st, "PUT", "/x", &lines, "b").await;
        assert!(r.status().is_success(), "{}", r.status());
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
        let Ok(all) = members(&st, &st.cfg.storage(), false).await else {
            panic!("listed");
        };
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

    /// Review finding: a linkset PATCH was held to the body limit on the document it produced,
    /// but a patch that strips the server-managed links got them back afterwards, so the served
    /// linkset could pass the limit. The rebuilt document is what is measured.
    #[tokio::test]
    async fn a_linkset_patch_is_measured_with_the_links_it_gets_back() {
        let mut cfg = super::super::LwsConfig::new(BASE);
        cfg.open = true;
        cfg.max_body = 64 << 10;
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let st = LwsState::new(store, cfg).await.expect("state");
        let types: String = (0..400)
            .map(|i| format!("<> a <https://e.example/{}{i}> .\n", "t".repeat(80)))
            .collect();
        let turtle = ("content-type", "text/turtle");
        let r = call(&st, "POST", "/", &[turtle, ("slug", "d")], &types).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let patch = ("content-type", "application/json-patch+json");
        let replace = |n: usize| {
            json!([{"op": "replace", "path": "/linkset/0", "value": {
                "anchor": format!("{BASE}/d"),
                "https://e.example/rel": [{"href": format!("https://e.example/{}", "x".repeat(n))}],
            }}])
            .to_string()
        };
        let r = call(&st, "PATCH", "/d.meta", &[patch], &replace(1 << 10)).await;
        assert!(r.status().is_success(), "{}", r.status());
        let r = call(&st, "PATCH", "/d.meta", &[patch], &replace(30 << 10)).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let served = body_of(call(&st, "GET", "/d.meta", &[], "").await).await;
        assert!(served.len() <= 64 << 10, "{}", served.len());
    }

    /// Review findings: relative references were resolved with no bound, the size checked only
    /// on the rebuilt document, and the indexed links copied a relation for every target: a patch
    /// of many short references under a long name, or of many targets under one long relation,
    /// allocated far more than the body limit first. A document a client writes is held to caps
    /// on entries, targets and each field, and its worst case to the budget, before anything is
    /// resolved.
    #[test]
    fn a_linkset_is_measured_before_it_is_resolved() {
        let base = "http://h/r.meta";
        let doc = |entries: usize, targets: usize, rel: &str, href: &str| {
            let entry = |i: usize| {
                let mut e = Map::new();
                e.insert("anchor".into(), json!(format!("http://h/{i}")));
                e.insert(rel.into(), json!(vec![json!({"href": href}); targets]));
                Value::Object(e)
            };
            json!({"linkset": (0..entries).map(entry).collect::<Vec<_>>()})
        };
        let ok = |d: &Value| absolute_linkset(d, base, usize::MAX).is_ok();
        let large = |d: &Value| absolute_linkset(d, base, usize::MAX) == Err(Unresolved::TooLarge);
        // Entries.
        assert!(ok(&doc(MAX_LINKSET_ENTRIES, 1, "license", "x")));
        assert!(large(&doc(MAX_LINKSET_ENTRIES + 1, 1, "license", "x")));
        // Targets, over every entry.
        let per = MAX_LINKSET_TARGETS / 4;
        assert!(ok(&doc(4, per, "license", "x")));
        let mut over = doc(4, per, "license", "x");
        over["linkset"][0]["license"]
            .as_array_mut()
            .unwrap()
            .push(json!({"href": "y"}));
        assert!(large(&over));
        // Each field.
        let at = "r".repeat(MAX_LINKSET_FIELD);
        let past = "r".repeat(MAX_LINKSET_FIELD + 1);
        assert!(ok(&doc(1, 1, &format!("x:{}", &at[2..]), "x")));
        assert!(large(&doc(1, 1, &format!("x:{}", &past[2..]), "x")));
        assert!(ok(&doc(1, 1, "license", &at)));
        assert!(large(&doc(1, 1, "license", &past)));
        let mut anchored = doc(1, 1, "license", "x");
        anchored["linkset"][0]["anchor"] = json!(format!("http://h/{}", &at[9..]));
        assert!(ok(&anchored));
        anchored["linkset"][0]["anchor"] = json!(format!("http://h/{}", &past[9..]));
        assert!(large(&anchored));
        // The worst case against the budget: at it, and one byte short of it.
        let d = doc(4, 8, "license", "x");
        let cost = linkset_cost(&d, base.len()).unwrap();
        assert!(absolute_linkset(&d, base, cost).is_ok());
        assert_eq!(
            absolute_linkset(&d, base, cost - 1),
            Err(Unresolved::TooLarge)
        );
        // One long relation over many targets: the copies the indexed links could make are
        // within the budget, or it is refused.
        let rel = format!("x:{}", "k".repeat(MAX_LINKSET_FIELD - 2));
        let hrefs: Vec<Value> = (0..MAX_LINKSET_TARGETS)
            .map(|i| json!({"href": format!("x:{i:05}")}))
            .collect();
        let many = json!({"linkset": [{"anchor": "http://h/r", rel.clone(): hrefs}]});
        let cost = linkset_cost(&many, base.len()).unwrap();
        assert!(cost >= 2 * json_size(&many));
        assert_eq!(
            absolute_linkset(&many, base, PATCH_BUDGET.min(cost - 1)),
            Err(Unresolved::TooLarge)
        );
        let resolved = absolute_linkset(&many, base, cost).unwrap();
        let links = links_of(&resolved, "http://h/r");
        assert_eq!(links[&rel].len(), MAX_LINKSET_TARGETS);
    }

    /// Review finding: the measure charged only anchors, relations and hrefs, so relations with
    /// no targets and long attribute arrays were not counted. Every value is charged.
    #[test]
    fn a_linkset_is_charged_for_every_value_it_holds() {
        let base = "http://h/r.meta";
        let mut empty = Map::new();
        empty.insert("anchor".into(), json!("http://h/r"));
        for i in 0..10_000 {
            empty.insert(format!("x:{i}"), json!([]));
        }
        let empty = json!({"linkset": [empty]});
        let attrs = json!({"linkset": [{"anchor": "http://h/r", "license": [{
            "href": "x", "ext": vec!["a"; 10_000],
        }]}]});
        for d in [&empty, &attrs] {
            let cost = linkset_cost(d, base.len()).unwrap();
            assert!(cost >= 10_000 * LINKSET_OVERHEAD, "{cost}");
            assert_eq!(
                absolute_linkset(d, base, 10_000 * LINKSET_OVERHEAD),
                Err(Unresolved::TooLarge)
            );
            assert!(absolute_linkset(d, base, cost).is_ok());
        }
    }

    /// Review finding: the indexed links resolved a linkset's references again, with other
    /// rules than the linkset's own, so a stored link could name another target than the
    /// served one. They are taken as resolved.
    #[test]
    fn indexed_links_are_the_linksets_own() {
        let doc = json!({"linkset": [{"anchor": "https://example.test/d", "license": [
            {"href": "https:foo"}, {"href": "https:foo"}, {"href": "/l"},
        ]}]});
        let resolved = absolute_linkset(&doc, "https://example.test/d.meta", usize::MAX).unwrap();
        let served: Vec<&str> = resolved.0["linkset"][0]["license"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["href"].as_str().unwrap())
            .collect();
        let links = links_of(&resolved, "https://example.test/d");
        let mut once = served.clone();
        once.dedup();
        assert_eq!(links["license"], once);
    }

    /// A PATCH past the caps is refused with 413, with a body under the limit.
    #[tokio::test]
    async fn a_linkset_patch_past_the_caps_is_refused() {
        let st = state().await;
        let uri = post(&st, "doc", "text/plain", "x", &[]).await;
        let path = format!("{}{META_SUFFIX}", path_of(&uri));
        let patch = ("content-type", JSON_PATCH);
        let targets = |n: usize| {
            replace_linkset(&json!({"linkset": [{
                "anchor": uri,
                "license": (0..n).map(|i| json!({"href": format!("x:{i}")})).collect::<Vec<_>>(),
            }]}))
        };
        let r = call(
            &st,
            "PATCH",
            &path,
            &[patch],
            &targets(MAX_LINKSET_TARGETS + 1),
        )
        .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let r = call(&st, "PATCH", &path, &[patch], &targets(MAX_LINKSET_TARGETS)).await;
        assert!(r.status().is_success(), "{}", r.status());
    }

    /// Review finding: a create whose store call reported a failure removed the new member's
    /// metadata, though a remote store may have committed the content before its reply was lost:
    /// the content stayed, without its creator, types and links. The metadata now goes only once
    /// the content is known to be gone. Review finding: retrying that removal without bound held
    /// the request (and its admission slot) for as long as the store failed; after a few tries
    /// the member is now set aside instead.
    #[tokio::test]
    async fn a_create_whose_outcome_is_unknown_is_removed_whole() {
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
        // The content lands, the reply is lost, and the content cannot be removed yet: after a
        // few tries the create answers 5xx and the member is set aside, its locks held by the
        // task that keeps removing it. Meanwhile it is unavailable (503), so the content is
        // never served or changed without its creator; once it can be removed it goes whole.
        let kept = st.cfg.absolute("/kept.txt");
        store.fail_after_create.store(true, Ordering::SeqCst);
        *store.fail_delete_of.lock().unwrap() = Some(kept.clone());
        let r = create(&st, &post("kept.txt"), &owner, &st.cfg.storage()).await;
        assert!(r.status().is_server_error(), "{}", r.status());
        store.fail_after_create.store(false, Ordering::SeqCst);
        assert!(!st.visible(&kept));
        for method in [Method::GET, Method::PUT, Method::DELETE] {
            let r = handle(&st, &req(method.clone(), "/kept.txt", &[], "y"), &owner).await;
            assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE, "{method}");
            assert!(r.headers().contains_key(header::RETRY_AFTER));
        }
        let r = handle(&st, &req(Method::GET, "/kept.txt.meta", &[], ""), &owner).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Nobody else takes its lock while it is set aside, and its container, whose listing
        // may show it, is set aside with it.
        assert!(st.locks.try_lock(&kept).is_none());
        assert!(!st.visible(&st.cfg.storage()));
        *store.fail_delete_of.lock().unwrap() = None;
        for _ in 0..100 {
            if st.visible(&kept) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(st.visible(&kept));
        assert!(st.locks.try_lock(&kept).is_some());
        assert!(!st.store.exists(&kept).await.unwrap());
        assert!(stored_meta(&st, &kept).await.unwrap().is_none());
        store.fail_after_create.store(true, Ordering::SeqCst);
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
        let sweeps = locks.map.lock().unwrap().sweep_at;
        assert!(sweeps >= 2 * LOCK_SWEEP_FLOOR, "{sweeps}");
        // Between sweeps the map grows to twice what the last one left.
        for i in 5000..5100 {
            held.push(locks.lock(&format!("http://h/{i}")).await);
        }
        assert_eq!(locks.map.lock().unwrap().sweep_at, sweeps);
        drop(held);
        let _ = locks.lock("http://h/x").await;
        assert!(locks.map.lock().unwrap().locks.len() <= 5101);
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

    /// Review finding: a recursive DELETE removed each descendant on its own, so a failure part way
    /// left those before it gone; and a PUT that changed metadata kept its new content when the
    /// last write failed. Every mutation of more than one store step goes through one journal:
    /// failing each of its steps in turn leaves content, metadata and membership as they were.
    #[tokio::test]
    async fn mutations_are_whole_or_not_at_all() {
        use super::super::test_store::{each_failure_changes_nothing, request as req, state};
        let container = format!("<{LWS_NS}Container>; rel=\"type\"");
        let tree = || {
            let container = container.clone();
            async move {
                let (st, store) = state(100).await;
                let anyone = Agent::anonymous();
                let post = |path: &str, slug: &str, link: Option<&str>, body: &str| {
                    let mut h = vec![("slug", slug), ("content-type", "text/turtle")];
                    if let Some(l) = link {
                        h.push(("link", l));
                    }
                    req(Method::POST, path, &h, body)
                };
                for (path, slug, link) in [
                    ("/", "c", Some(container.as_str())),
                    ("/c/", "a", None),
                    ("/c/", "d", Some(container.as_str())),
                    ("/c/d/", "b", Some("<https://e.example/L>; rel=\"license\"")),
                ] {
                    let r = handle(
                        &st,
                        &post(path, slug, link, "<> a <https://e.example/T> ."),
                        &anyone,
                    )
                    .await;
                    assert_eq!(r.status(), StatusCode::CREATED);
                }
                let root = st.cfg.storage();
                // The storage root too: its validators must not move for a change that was undone.
                let iris: Vec<String> = ["", "c/", "c/a", "c/d/", "c/d/b", "c/n"]
                    .iter()
                    .map(|p| format!("{root}{p}"))
                    .collect();
                let listings = vec![root, iris[1].clone(), iris[3].clone()];
                (st, store, iris, listings)
            }
        };
        let steps = each_failure_changes_nothing(&tree, |st| async move {
            let r = handle(
                &st,
                &req(Method::DELETE, "/c/", &[("depth", "infinity")], ""),
                &Agent::anonymous(),
            )
            .await;
            r.status()
        })
        .await;
        assert!(steps >= 8, "a delete of four resources took {steps} steps");
        let steps = each_failure_changes_nothing(&tree, |st| async move {
            let h = [
                ("content-type", "text/turtle"),
                ("prefer", "set-linkset"),
                ("link", "<https://e.example/Other>; rel=\"type\""),
            ];
            let r = handle(
                &st,
                &req(Method::PUT, "/c/d/b", &h, "<> a <https://e.example/U> ."),
                &Agent::anonymous(),
            )
            .await;
            r.status()
        })
        .await;
        assert!(
            steps >= 3,
            "a write that changes metadata took {steps} steps"
        );
        // Creates: the new member, its metadata and the container's listing, or none of them.
        let steps = each_failure_changes_nothing(&tree, |st| async move {
            let h = [("content-type", "text/turtle"), ("slug", "n")];
            let r = handle(
                &st,
                &req(Method::POST, "/c/", &h, "<> a <https://e.example/U> ."),
                &Agent::anonymous(),
            )
            .await;
            r.status()
        })
        .await;
        assert!(steps >= 2, "a create took {steps} steps");
    }

    /// Review finding: a rollback that failed part way gave up, and what it had not put back
    /// was lost or later put back over newer writes. A step that fails is now tried again, with
    /// the subtree's locks held, until the delete is whole undone, validators included.
    #[tokio::test]
    async fn a_failed_rollback_is_retried_until_undone() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let root = st.cfg.storage();
        for slug in ["a", "b", "c"] {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let r = handle(&st, &req(Method::POST, "/", &h, "x"), &Agent::anonymous()).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let doomed: Vec<(String, Option<String>)> = ["a", "b", "c"]
            .iter()
            .map(|n| (format!("{root}{n}"), Some(root.clone())))
            .collect();
        let mut before = Vec::new();
        for (node, _) in &doomed {
            before.push(st.store.meta(node).await.unwrap().unwrap());
        }
        // a and b go (two steps each), c fails; putting back restores b's metadata and b, then
        // fails once on a's metadata, which is tried again.
        *store.fail_delete_of.lock().unwrap() = Some(doomed[2].0.clone());
        *store.fail_step.lock().unwrap() = Some(7);
        let (gone, outcome, _) = remove(&st, &doomed).await;
        *store.fail_delete_of.lock().unwrap() = None;
        assert!(
            store.fail_step.lock().unwrap().take().is_none(),
            "the rollback never failed"
        );
        assert!(outcome.is_err());
        assert!(gone.is_empty());
        for ((node, _), was) in doomed.iter().zip(before) {
            let now = st.store.meta(node).await.unwrap().expect("put back");
            assert_eq!((now.etag, now.last_modified), (was.etag, was.last_modified));
            assert!(st.store.exists(&meta_key(node)).await.unwrap());
        }
    }

    /// Review finding: a container whose own time could not be moved on after a member was
    /// deleted kept its old Last-Modified, and every time in its listing could be older still,
    /// so an If-Modified-Since from before the delete met a 304. Until a touch lands the
    /// listing has no Last-Modified, and a date alone never makes it 304.
    #[tokio::test]
    async fn a_failed_touch_leaves_no_date_to_validate_against() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let anyone = Agent::anonymous();
        for slug in ["a", "b"] {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let r = handle(&st, &req(Method::POST, "/", &h, "x"), &anyone).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let r = handle(
            &st,
            &req(Method::GET, "/", &[("accept", "application/lws+json")], ""),
            &anyone,
        )
        .await;
        let since = r.headers()[header::LAST_MODIFIED]
            .to_str()
            .unwrap()
            .to_string();
        let root = st.cfg.storage();
        *store.fail_write_of.lock().unwrap() = Some(meta_key(&root));
        let r = handle(&st, &req(Method::DELETE, "/b", &[], ""), &anyone).await;
        assert!(r.status().is_success(), "{}", r.status());
        *store.fail_write_of.lock().unwrap() = None;
        let ims = [
            ("accept", "application/lws+json"),
            ("if-modified-since", since.as_str()),
        ];
        let r = handle(&st, &req(Method::GET, "/", &ims, ""), &anyone).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(!r.headers().contains_key(header::LAST_MODIFIED));
        // The next touch that lands brings the date back.
        let h = [("slug", "c"), ("content-type", "text/plain")];
        let r = handle(&st, &req(Method::POST, "/", &h, "x"), &anyone).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let r = handle(
            &st,
            &req(Method::GET, "/", &[("accept", "application/lws+json")], ""),
            &anyone,
        )
        .await;
        assert!(r.headers().contains_key(header::LAST_MODIFIED));
    }

    /// Review finding: a container's entity tag was computed before the sizes were added, so a
    /// listing that could not read a member's size and one that could shared a strong tag. The
    /// tag covers the page as shown.
    #[tokio::test]
    async fn a_listing_tag_covers_the_sizes_it_shows() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let anyone = Agent::anonymous();
        let h = [("slug", "a"), ("content-type", "text/plain")];
        let r = handle(&st, &req(Method::POST, "/", &h, "abc"), &anyone).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let tag = |r: &Response| r.headers()[header::ETAG].to_str().unwrap().to_string();
        let whole = handle(
            &st,
            &req(Method::GET, "/", &[("accept", "application/lws+json")], ""),
            &anyone,
        )
        .await;
        *store.fail_read_of.lock().unwrap() = Some(st.cfg.absolute("/a"));
        let sizeless = handle(
            &st,
            &req(Method::GET, "/", &[("accept", "application/lws+json")], ""),
            &anyone,
        )
        .await;
        *store.fail_read_of.lock().unwrap() = None;
        assert_eq!(sizeless.status(), StatusCode::OK);
        assert_ne!(tag(&whole), tag(&sizeless));
        let again = handle(
            &st,
            &req(Method::GET, "/", &[("accept", "application/lws+json")], ""),
            &anyone,
        )
        .await;
        assert_eq!(tag(&whole), tag(&again));
    }

    /// Review finding: a PUT with Content-Range replaced the whole resource with the part it
    /// carried. It is refused (RFC 9110 section 14.5), and nothing changes.
    #[tokio::test]
    async fn a_partial_put_is_refused() {
        use super::super::test_store::{request as req, state};
        let (st, _store) = state(100).await;
        let h = [("slug", "p"), ("content-type", "text/plain")];
        let r = handle(
            &st,
            &req(Method::POST, "/", &h, "abcdef"),
            &Agent::anonymous(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let h = [
            ("content-type", "text/plain"),
            ("content-range", "bytes 1-2/6"),
        ];
        let r = handle(&st, &req(Method::PUT, "/p", &h, "XY"), &Agent::anonymous()).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = handle(&st, &req(Method::GET, "/p", &[], ""), &Agent::anonymous()).await;
        assert_eq!(&body_of(r).await[..], b"abcdef");
    }

    /// Review finding: a rollback retried without bound held its locks, and the request its
    /// admission slot, for as long as the store failed, and every request for what it held
    /// waited behind it. After a few tries the request now ends (5xx) and what is left is set
    /// aside: answered 503 at once, its locks held by a task that keeps putting it back, and
    /// served again, as it was, once that is done.
    #[tokio::test]
    async fn a_rollback_that_keeps_failing_is_set_aside() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let root = st.cfg.storage();
        for slug in ["a", "b", "c"] {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let r = handle(&st, &req(Method::POST, "/", &h, "x"), &Agent::anonymous()).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let h = [("content-type", "text/plain")];
        let r = handle(&st, &req(Method::PUT, "/b", &h, "y"), &Agent::anonymous()).await;
        assert!(r.status().is_success(), "{}", r.status());
        let b = format!("{root}b");
        let was = st.store.meta(&b).await.unwrap().unwrap();
        // b goes, then c cannot, and b cannot be put back.
        let doomed: Vec<(String, Option<String>)> = ["b", "c"]
            .iter()
            .map(|n| (format!("{root}{n}"), Some(root.clone())))
            .collect();
        *store.fail_delete_of.lock().unwrap() = Some(doomed[1].0.clone());
        *store.fail_restore_of.lock().unwrap() = Some(b.clone());
        let (gone, outcome, left) = remove(&st, &doomed).await;
        assert!(outcome.is_err());
        assert_eq!(gone.len(), 2, "what could not be put back counts as gone");
        let left = left.expect("set aside");
        *store.fail_delete_of.lock().unwrap() = None;
        let guard = st.locks.lock(&b).await;
        st.set_aside(left, guard);
        let get = |p: &'static str| {
            let (st, r) = (st.clone(), req(Method::GET, p, &[], ""));
            async move { handle(&st, &r, &Agent::anonymous()).await }
        };
        let r = get("/b").await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            get("/b.meta").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        // What it does not hold is served as usual.
        assert_eq!(get("/a").await.status(), StatusCode::OK);
        *store.fail_restore_of.lock().unwrap() = None;
        for _ in 0..100 {
            if st.visible(&b) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let r = get("/b").await;
        assert_eq!(r.status(), StatusCode::OK);
        let now = st.store.meta(&b).await.unwrap().expect("put back");
        assert_eq!((now.etag, now.last_modified), (was.etag, was.last_modified));
        assert_eq!(&body_of(r).await[..], b"y");
    }

    /// Review findings: a stuck change was hidden only at its own IRI, so its container's
    /// listing still showed the half-done member; one recovery finishing made a container
    /// visible while another to it was still unfinished; and what recoveries held had no bound.
    /// Every route asks [`LwsState::visible`](super::super::LwsState::visible) first.
    #[tokio::test]
    async fn a_set_aside_change_hides_what_it_is_to_from_every_route() {
        use super::super::test_store::{request as req, state};
        use super::super::{Undo, Unsettled};
        let (st, store) = state(100).await;
        let root = st.cfg.storage();
        for slug in ["a", "b", "c"] {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let r = handle(&st, &req(Method::POST, "/", &h, "x"), &Agent::anonymous()).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let (a, b) = (format!("{root}a"), format!("{root}b"));
        let b_meta = st.store.meta(&b).await.unwrap().unwrap();
        *store.fail_delete_of.lock().unwrap() = Some(a.clone());
        *store.fail_exists_of.lock().unwrap() = Some(b.clone());
        let undo_a = Undo::Restore {
            key: a.clone(),
            prior: None,
        };
        let undo_b = Undo::Recreate {
            iri: b.clone(),
            parent: Some(root.clone()),
            body: Bytes::from("x"),
            meta: b_meta,
        };
        st.set_aside(Unsettled(vec![undo_a]), ());
        st.set_aside(Unsettled(vec![undo_b]), ());
        let call = |m: Method, p: &'static str| {
            let st = st.clone();
            let h = [("content-type", "text/plain"), ("slug", "d")];
            let r = req(m, p, &h, "z");
            async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    handle(&st, &r, &Agent::anonymous()),
                )
                .await
                .expect("answered at once")
                .status()
            }
        };
        let methods = [
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::PATCH,
            Method::POST,
            Method::DELETE,
        ];
        for p in ["/a", "/a.meta", "/b", "/b.meta", "/"] {
            for m in &methods {
                let got = call(m.clone(), p).await;
                assert_eq!(got, StatusCode::SERVICE_UNAVAILABLE, "{m} {p}");
            }
        }
        // What no stuck change is to is served as usual.
        assert_eq!(call(Method::GET, "/c").await, StatusCode::OK);
        // a's recovery ends; the container stays hidden while b's has not.
        *store.fail_delete_of.lock().unwrap() = None;
        let settled = |iri: String| {
            let st = st.clone();
            async move {
                for _ in 0..250 {
                    if st.visible(&iri) {
                        return true;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                false
            }
        };
        assert!(settled(a.clone()).await);
        assert!(!st.visible(&root), "b's recovery is not over");
        assert_eq!(
            call(Method::GET, "/").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        *store.fail_exists_of.lock().unwrap() = None;
        assert!(settled(root.clone()).await);
        assert_eq!(call(Method::GET, "/").await, StatusCode::OK);
        // While recoveries hold as much as they may, nothing new is changed; reads go on.
        st.set_aside_bytes.store(
            super::super::MAX_SET_ASIDE_BYTES,
            std::sync::atomic::Ordering::Release,
        );
        assert_eq!(
            call(Method::PUT, "/c").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(call(Method::GET, "/c").await, StatusCode::OK);
        st.set_aside_bytes
            .store(0, std::sync::atomic::Ordering::Release);
        assert!(call(Method::PUT, "/c").await.is_success());
    }

    /// Review finding: a recursive delete whose rollback was set aside hid only what it had
    /// removed, while its task held the locks of the whole subtree; what it had not reached yet
    /// stayed visible, so a request for it (or a walk over it) waited on a lock that would not
    /// come back until the rollback was done. Everything the delete holds is set aside with it.
    #[tokio::test]
    async fn a_stuck_recursive_delete_sets_aside_what_it_had_not_reached() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let root = st.cfg.storage();
        let anon = Agent::anonymous();
        let mk = |p: &'static str, slug: &'static str, container: bool| {
            let st = st.clone();
            async move {
                let mut h = vec![("slug", slug), ("content-type", "text/plain")];
                if container {
                    h.push(("link", CONTAINER_LINK));
                }
                let r = handle(&st, &req(Method::POST, p, &h, "x"), &Agent::anonymous()).await;
                assert_eq!(r.status(), StatusCode::CREATED);
            }
        };
        mk("/", "d", true).await;
        mk("/d/", "x", false).await;
        mk("/d/", "e", true).await;
        mk("/d/e/", "z", false).await;
        let at = |p: &str| format!("{root}{p}");
        // x goes; z cannot, and x cannot be put back. e and z were never reached.
        let doomed: Vec<(String, Option<String>)> = vec![
            (at("d/x"), Some(at("d/"))),
            (at("d/e/z"), Some(at("d/e/"))),
            (at("d/e/"), Some(at("d/"))),
            (at("d/"), Some(root.clone())),
        ];
        let Ok((guards, _)) = lock_subtree(&st, &at("d/"), Some(root.clone())).await else {
            panic!("locked");
        };
        *store.fail_delete_of.lock().unwrap() = Some(at("d/e/z"));
        *store.fail_restore_of.lock().unwrap() = Some(at("d/x"));
        let (_, outcome, left) = remove(&st, &doomed).await;
        assert!(outcome.is_err());
        *store.fail_delete_of.lock().unwrap() = None;
        st.set_aside(left.expect("set aside"), guards);
        let get = |p: &'static str| {
            let (st, r) = (st.clone(), req(Method::GET, p, &[], ""));
            async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    handle(&st, &r, &Agent::anonymous()),
                )
                .await
                .expect("answered at once")
                .status()
            }
        };
        for p in ["/d/", "/d/x", "/d/e/", "/d/e/z", "/d/e/z.meta"] {
            assert_eq!(get(p).await, StatusCode::SERVICE_UNAVAILABLE, "{p}");
        }
        *store.fail_restore_of.lock().unwrap() = None;
        for _ in 0..100 {
            if st.visible(&at("d/e/z")) && st.visible(&at("d/x")) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        for p in ["/d/x", "/d/e/z"] {
            let r = handle(&st, &req(Method::GET, p, &[], ""), &anon).await;
            assert_eq!(r.status(), StatusCode::OK, "{p}");
        }
    }

    /// Review finding: a listing read its members without their locks, so a change in flight
    /// (content written, its metadata not yet) showed in it, though the change might yet be put
    /// back; and a member's removal, which held its container only shared, could be listed
    /// as gone before it was put back. A listing reads each member under its shared lock, and
    /// is read again once a change in flight to one is over; a removal holds its container
    /// exclusively.
    #[tokio::test]
    async fn a_listing_waits_out_changes_in_flight_to_its_members() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let anon = Agent::anonymous();
        let h = [("slug", "x"), ("content-type", "text/plain")];
        let r = handle(&st, &req(Method::POST, "/", &h, "x"), &anon).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let x = format!("{}x", st.cfg.storage());
        let list = || {
            let st = st.clone();
            tokio::spawn(async move {
                let r = req(Method::GET, "/", &[("accept", "application/lws+json")], "");
                handle(&st, &r, &Agent::anonymous()).await
            })
        };
        let held = st.locks.lock(&x).await.expect("visible");
        let listing = list();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!listing.is_finished(), "listed during a change to a member");
        drop(held);
        let r = tokio::time::timeout(Duration::from_secs(1), listing)
            .await
            .expect("listed once the change is over")
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        // A removal in flight holds the container: the listing waits for it.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_delete_of.lock().unwrap() = Some((x.clone(), gate.clone()));
        let removal = {
            let st = st.clone();
            tokio::spawn(async move {
                handle(
                    &st,
                    &req(Method::DELETE, "/x", &[], ""),
                    &Agent::anonymous(),
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let listing = list();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!listing.is_finished(), "listed during a removal");
        gate.add_permits(1);
        let r = removal.await.unwrap();
        assert!(r.status().is_success(), "{}", r.status());
        let r = tokio::time::timeout(Duration::from_secs(1), listing)
            .await
            .expect("listed once the removal is over")
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(!String::from_utf8_lossy(&body_of(r).await).contains(&x));
    }

    /// Review finding: a container's date stayed valid from when a member's removal was
    /// released until the touch after it landed, so an If-Modified-Since from before the
    /// removal could meet a 304 on a listing that had changed. The date counts as behind from
    /// before the removal's locks go until the touch lands.
    #[tokio::test]
    async fn a_listing_has_no_date_until_the_touch_after_a_change_lands() {
        use super::super::test_store::{request as req, state};
        let (st, _store) = state(100).await;
        let anon = Agent::anonymous();
        for slug in ["a", "b"] {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let r = handle(&st, &req(Method::POST, "/", &h, "x"), &anon).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let root = st.cfg.storage();
        let since = httpdate::fmt_http_date(
            std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
        );
        // Every touch of the root waits.
        let touch = st.locks.lock(&format!("{root}\0touch")).await.unwrap();
        let removal = {
            let st = st.clone();
            tokio::spawn(async move {
                handle(
                    &st,
                    &req(Method::DELETE, "/b", &[], ""),
                    &Agent::anonymous(),
                )
                .await
            })
        };
        let b = format!("{root}b");
        for _ in 0..200 {
            if !st.store.exists(&b).await.unwrap() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!st.store.exists(&b).await.unwrap());
        let ims = [
            ("accept", "application/lws+json"),
            ("if-modified-since", since.as_str()),
        ];
        let r = handle(&st, &req(Method::GET, "/", &ims, ""), &anon).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(!r.headers().contains_key(header::LAST_MODIFIED));
        drop(touch);
        assert!(removal.await.unwrap().status().is_success());
        let r = handle(&st, &req(Method::GET, "/", &ims, ""), &anon).await;
        assert!(r.headers().contains_key(header::LAST_MODIFIED));
    }

    /// Review findings, three rounds on container validators: a listing could pair members from
    /// before a change with a date from after it, and a container's create answered with a tag
    /// no read of it carries. For each change to a container, its listing's entity tag changes
    /// whenever the listing does (and stays when nothing changed), a listing is never served with a date older than one of
    /// its members, and a new container's create sends no tag of its own.
    #[tokio::test]
    async fn container_validators_change_exactly_when_the_listing_does() {
        use super::super::test_store::{request as req, state};
        let (st, _store) = state(100).await;
        let anon = Agent::anonymous();
        let call = |m: Method, p: &'static str, h: Vec<(&'static str, &'static str)>, b| {
            let st = st.clone();
            async move { handle(&st, &req(m, p, &h, b), &Agent::anonymous()).await }
        };
        let r = call(
            Method::POST,
            "/",
            vec![("slug", "c"), ("link", CONTAINER_LINK)],
            "",
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert!(
            !r.headers().contains_key(header::ETAG),
            "a container's create sends no tag"
        );
        let listing = || async {
            let r = handle(
                &st,
                &req(
                    Method::GET,
                    "/c/",
                    &[("accept", "application/lws+json")],
                    "",
                ),
                &anon,
            )
            .await;
            assert_eq!(r.status(), StatusCode::OK);
            let etag = r.headers()[header::ETAG].to_str().unwrap().to_string();
            let date = r
                .headers()
                .get(header::LAST_MODIFIED)
                .and_then(|v| parse_http_date(v.to_str().ok()));
            let doc: Value = serde_json::from_slice(&body_of(r).await).unwrap();
            // The date covers every member listed.
            if let Some(date) = date {
                for item in doc["items"].as_array().into_iter().flatten() {
                    let m = item["modified"].as_str().and_then(parse_rfc3339).unwrap();
                    assert!(m as u64 <= date, "a member newer than the listing's date");
                }
            }
            (etag, doc)
        };
        let text = vec![("content-type", "text/plain")];
        type Headers = Vec<(&'static str, &'static str)>;
        let changes: Vec<(&str, Method, &str, Headers, &str)> = vec![
            (
                "create",
                Method::POST,
                "/c/",
                vec![("slug", "a"), ("content-type", "text/plain")],
                "x",
            ),
            ("replace", Method::PUT, "/c/a", text.clone(), "y"),
            ("same bytes", Method::PUT, "/c/a", text.clone(), "y"),
            (
                "create another",
                Method::POST,
                "/c/",
                vec![("slug", "b"), ("content-type", "text/plain")],
                "z",
            ),
            ("delete", Method::DELETE, "/c/b", vec![], ""),
            ("read", Method::GET, "/c/a", vec![], ""),
        ];
        let (mut etag, mut doc) = listing().await;
        for (what, m, p, h, b) in changes {
            let r = call(m, p, h, b).await;
            assert!(r.status().is_success(), "{what}: {}", r.status());
            let (now_etag, now_doc) = listing().await;
            // A listing that changed has a new tag; one that did not (and no member's content
            // changed, which the tag also covers) keeps its tag.
            if now_doc != doc {
                assert_ne!(now_etag, etag, "{what}");
            }
            if what == "read" {
                assert_eq!(now_etag, etag, "{what}");
                assert_eq!(now_doc, doc, "{what}");
            }
            (etag, doc) = (now_etag, now_doc);
        }
    }

    /// Review finding: a touch of a member container changes the date its parent's listing
    /// shows for it, but moved only the member's own generation, so a parent listing made
    /// across touches of two members could pair one's old date with the other's new one under
    /// a date from after both. A metadata write to a container moves its parent's generation
    /// too, before and after, and a listing made while one is in flight has no date.
    #[tokio::test]
    async fn a_member_containers_touch_moves_its_parents_snapshot() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let anon = Agent::anonymous();
        for (parent, slug) in [("/", "c"), ("/c/", "a"), ("/c/", "b")] {
            let h = [("slug", slug), ("link", CONTAINER_LINK)];
            let r = handle(&st, &req(Method::POST, parent, &h, ""), &anon).await;
            assert_eq!(r.status(), StatusCode::CREATED);
        }
        let c = st.cfg.absolute("/c/");
        let a = st.cfg.absolute("/c/a/");
        let dated = || async {
            let h = [("accept", "application/lws+json")];
            let r = handle(&st, &req(Method::GET, "/c/", &h, ""), &anon).await;
            assert_eq!(r.status(), StatusCode::OK);
            r.headers().contains_key(header::LAST_MODIFIED)
        };
        assert!(dated().await);
        // While the member's date is being written, the parent's listing has no date.
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_write_of.lock().unwrap() = Some((meta_key(&a), gate.clone()));
        let before = st.generation(&c);
        let touching = {
            let (st, a) = (st.clone(), a.clone());
            tokio::spawn(async move { touch_container(&st, &a).await })
        };
        while !st.in_flight(&c) {
            tokio::task::yield_now().await;
        }
        assert_ne!(
            st.generation(&c),
            before,
            "a member's touch moves its parent"
        );
        assert!(
            !dated().await,
            "no date while a member's date is being written"
        );
        let during = st.generation(&c);
        gate.add_permits(1);
        touching.await.unwrap();
        assert_ne!(st.generation(&c), during);
        assert!(dated().await);
        // A real touch does the same.
        let before = st.generation(&c);
        touch_container(&st, &a).await;
        assert_ne!(st.generation(&c), before);
    }

    /// Review finding: a conditional POST to the storage root was evaluated against the root's
    /// listing even when a GET of it (no Accept, or the description's type) selects the
    /// storage description, so the description's own tag failed If-Match.
    #[tokio::test]
    async fn a_conditional_create_at_the_root_uses_the_representation_a_get_selects() {
        use super::super::test_store::{request as req, state};
        let (st, _store) = state(100).await;
        let anon = Agent::anonymous();
        let r = handle(&st, &req(Method::GET, "/", &[], ""), &anon).await;
        assert_eq!(r.status(), StatusCode::OK);
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let h = [
            ("slug", "x"),
            ("content-type", "text/plain"),
            ("if-match", tag.as_str()),
        ];
        let r = handle(&st, &req(Method::POST, "/", &h, "x"), &anon).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        // Asking for the listing, the listing's tag is what counts.
        let accept = [("accept", "application/lws+json")];
        let r = handle(&st, &req(Method::GET, "/", &accept, ""), &anon).await;
        let listing = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let h = [
            ("slug", "y"),
            ("content-type", "text/plain"),
            ("accept", "application/lws+json"),
            ("if-match", listing.as_str()),
        ];
        let r = handle(&st, &req(Method::POST, "/", &h, "y"), &anon).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let h = [
            ("slug", "z"),
            ("content-type", "text/plain"),
            ("accept", "application/lws+json"),
            ("if-match", tag.as_str()),
        ];
        let r = handle(&st, &req(Method::POST, "/", &h, "z"), &anon).await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
    }

    /// Review finding: visibility was checked only as a request came in, so one already
    /// waiting on a lock when the change holding it was set aside waited on until the change
    /// was put back. Every lock is taken only while its IRI is visible, and a wait for one
    /// gives up as soon as it is set aside.
    #[tokio::test]
    async fn a_request_waiting_on_a_lock_is_answered_once_it_is_set_aside() {
        use super::super::test_store::{request as req, state};
        use super::super::{Undo, Unsettled};
        let (st, store) = state(100).await;
        let anon = Agent::anonymous();
        let h = [("slug", "x"), ("content-type", "text/plain")];
        let r = handle(&st, &req(Method::POST, "/", &h, "x"), &anon).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let x = format!("{}x", st.cfg.storage());
        let held = st.locks.lock(&x).await.unwrap();
        let waiting: Vec<_> = [Method::GET, Method::PUT, Method::DELETE]
            .into_iter()
            .map(|m| {
                let st = st.clone();
                tokio::spawn(async move {
                    let h = [("content-type", "text/plain")];
                    let r = req(m.clone(), "/x", &h, "y");
                    (m, handle(&st, &r, &Agent::anonymous()).await.status())
                })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(100)).await;
        *store.fail_delete_of.lock().unwrap() = Some(x.clone());
        let undo = Undo::Restore {
            key: x.clone(),
            prior: None,
        };
        st.set_aside(Unsettled(vec![undo]), held);
        for task in waiting {
            let (m, status) = tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .expect("answered at once")
                .unwrap();
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{m}");
        }
        *store.fail_delete_of.lock().unwrap() = None;
    }

    /// Review finding: a recursive delete found out it was too large to undo only after removing
    /// part of the tree. Everything it could need to put back is now read first.
    #[tokio::test]
    async fn a_delete_too_large_to_undo_removes_nothing() {
        use super::super::test_store::{request as req, state};
        let (st, store) = state(100).await;
        let root = st.cfg.storage();
        let mut doomed = Vec::new();
        for slug in ["a", "b", "c"] {
            let h = [("slug", slug), ("content-type", "text/plain")];
            let r = handle(&st, &req(Method::POST, "/", &h, "x"), &Agent::anonymous()).await;
            assert_eq!(r.status(), StatusCode::CREATED);
            doomed.push((format!("{root}{slug}"), Some(root.clone())));
        }
        // Room to put back two of the three, not all.
        let mut journal = st.journal();
        journal.limit = 2 * (doomed[0].0.len() + "x".len() + meta_key(&doomed[0].0).len()) + 1;
        // Every store step is counted down: none may be taken.
        *store.fail_step.lock().unwrap() = Some(1000);
        let (gone, outcome, _) = remove_in(journal, &doomed).await;
        assert!(matches!(outcome, Err(ServerError::Conflict(_))));
        assert!(gone.is_empty());
        assert_eq!(*store.fail_step.lock().unwrap(), Some(1000));
        for (node, _) in &doomed {
            assert!(st.store.exists(node).await.unwrap());
        }
    }

    /// Review finding: a DELETE ignored a failure to remove the metadata, which then described
    /// the next resource created at the IRI (a failed delete is now put back whole); and a create
    /// wrote its content before its creator metadata, so a failure in between left the new
    /// content under the old creator.
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
        // The delete was put back whole: the resource and its metadata are as they were.
        assert!(st.store.exists(&x).await.unwrap());
        assert_eq!(st.resource_meta(&x).await.unwrap().creator, bob.subject);
        // Metadata left behind with no resource (as a store failure before deletes were put back
        // could leave it).
        super::super::remove_member(&st.store, &x, Some(&st.cfg.storage()))
            .await
            .unwrap();
        assert!(!st.store.exists(&x).await.unwrap());
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

    /// Review finding: a linkset PATCH, a content-only PUT and a DELETE made their store writes
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
        // A linkset PATCH whose metadata write is pending when the client goes away.
        let x = format!("{c}x");
        assert_eq!(
            post(st.clone(), owner.clone(), "x", "owner's")
                .await
                .status(),
            StatusCode::CREATED
        );
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_write_of.lock().unwrap() = Some((meta_key(&x), gate.clone()));
        let patch = format!(
            r#"[{{"op":"replace","path":"/linkset","value":[{{"anchor":"{x}","https://e.example/rel":[{{"href":"https://e.example/t"}}]}}]}}]"#
        );
        let h = [("content-type", JSON_PATCH)];
        let cancelled = tokio::time::timeout(
            Duration::from_millis(50),
            handle(&st, &req(Method::PATCH, "/c/x.meta", &h, &patch), &owner),
        )
        .await;
        assert!(cancelled.is_err());
        replace("x", gate).await;
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
        let version = || async { stored_meta(&st, &c).await.unwrap().unwrap().version };
        let was = version().await;
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
        // Review finding: the container was touched by the request, after the removal, so it
        // was not when the request had gone: its date stayed behind its listing.
        for _ in 0..200 {
            if version().await != was {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_ne!(version().await, was);
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
                r#"[{{"op":"replace","path":"/linkset","value":[{{"anchor":"{uri}","https://e.example/rel":[{{"href":"x","ext":{}1{}}}]}}]}}]"#,
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
            &[("content-type", JSON_PATCH)],
            &body(depth),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let kept = st.resource_meta(&uri).await.unwrap();
        assert_eq!(kept.creator, meta.creator);
        assert_eq!(kept.types, meta.types);
        // One that fits as stored (and has the shapes RFC 9264 gives its attributes: nothing
        // nests deeper than an internationalised attribute's objects) is taken, and reads back.
        let fits = format!(
            r#"[{{"op":"replace","path":"/linkset","value":[{{"anchor":"{uri}","https://e.example/rel":[{{"href":"x","ext":["1"],"title*":[{{"value":"t"}}]}}]}}]}}]"#
        );
        let r = call(&st, "PATCH", &p, &[("content-type", JSON_PATCH)], &fits).await;
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

    /// Review finding: a JSON Patch `move` was not counted against the budget at all, and adds and
    /// copies counted their values but not the keys and separators around them, so a patch could
    /// build a document larger than the budget.
    #[test]
    fn json_patch_counts_keys_separators_and_moves() {
        let long = "k".repeat(1000);
        // Many members with long keys and empty values: the keys are most of the document.
        let ops: Vec<Value> = (0..20)
            .map(|i| json!({"op": "add", "path": format!("/{long}{i}"), "value": 0}))
            .collect();
        let ops = Value::Array(ops);
        let built = json_patch(&json!({}), &ops, PATCH_BUDGET).unwrap();
        let size = json_size(&built);
        assert!(size > 20_000);
        // Exact: the budget the result takes is enough, a byte less is not.
        assert_eq!(json_patch(&json!({}), &ops, size), Ok(built));
        assert_eq!(
            json_patch(&json!({}), &ops, size - 1),
            Err(PatchError::TooLarge)
        );
        // A move to a longer key grows the document by the difference.
        let doc = json!({"a": [1, 2, 3], "b": {}});
        let ops = json!([{"op": "move", "from": "/a", "path": format!("/b/{long}")}]);
        let moved = json_patch(&doc, &ops, PATCH_BUDGET).unwrap();
        let size = json_size(&moved);
        assert_eq!(json_patch(&doc, &ops, size), Ok(moved));
        assert_eq!(json_patch(&doc, &ops, size - 1), Err(PatchError::TooLarge));
        // Moves, copies, removes and escaped keys, counted exactly along the way.
        let ops = json!([
            {"op": "add", "path": "/x", "value": {"q\"uote": [1, 2]}},
            {"op": "copy", "from": "/x", "path": "/y~1z"},
            {"op": "move", "from": "/x/q\"uote/0", "path": "/x/q\"uote/-"},
            {"op": "remove", "path": "/b"},
            {"op": "add", "path": "/arr", "value": []},
            {"op": "move", "from": "/a", "path": "/arr/0"},
        ]);
        let out = json_patch(&doc, &ops, PATCH_BUDGET).unwrap();
        // The largest the document gets along the way.
        let peak = (1..=ops.as_array().unwrap().len())
            .map(|n| {
                let prefix = Value::Array(ops.as_array().unwrap()[..n].to_vec());
                json_size(&json_patch(&doc, &prefix, PATCH_BUDGET).unwrap())
            })
            .max()
            .unwrap();
        assert_eq!(json_patch(&doc, &ops, peak).as_ref(), Ok(&out));
        assert_eq!(json_patch(&doc, &ops, peak - 1), Err(PatchError::TooLarge));
    }

    /// Review finding: array indexes were parsed with `str::parse`, so `01` and `+1` named
    /// member 1 in some operations, and escapes other than `~0` and `~1` passed. Every operation
    /// now reads its pointers through one RFC 6901 parser.
    #[test]
    fn json_pointers_follow_rfc_6901() {
        let doc = json!({"a": [10, 11, 12], "~/": 1});
        let run = |ops: Value| json_patch(&doc, &ops, PATCH_BUDGET);
        for index in [
            "01",
            "+1",
            "1.0",
            " 1",
            "1 ",
            "-1",
            "3",
            "18446744073709551616",
            "-",
        ] {
            let path = format!("/a/{index}");
            for op in [
                json!({"op": "remove", "path": path}),
                json!({"op": "replace", "path": path, "value": 0}),
                json!({"op": "test", "path": path, "value": 11}),
                json!({"op": "copy", "from": path, "path": "/b"}),
                json!({"op": "move", "from": path, "path": "/b"}),
            ] {
                assert_eq!(run(json!([op])), Err(PatchError::Failed), "{op}");
            }
            if index != "3" && index != "-" {
                let add = json!([{"op": "add", "path": path, "value": 0}]);
                assert_eq!(run(add), Err(PatchError::Failed), "add at {index}");
            }
        }
        for bad in ["a", "/~2", "/~", "/a~"] {
            let op = json!([{"op": "test", "path": bad, "value": 1}]);
            assert!(matches!(run(op), Err(PatchError::Malformed(_))), "{bad}");
        }
        let ops = json!([
            {"op": "test", "path": "/a/0", "value": 10},
            {"op": "test", "path": "/~0~1", "value": 1},
            {"op": "add", "path": "/a/3", "value": 13},
            {"op": "add", "path": "/a/-", "value": 14},
            {"op": "move", "from": "/a/1", "path": "/a/1"},
        ]);
        assert_eq!(run(ops).unwrap()["a"], json!([10, 11, 12, 13, 14]));
    }

    /// Review finding: the work budget charged what an operation adds, not the array members an
    /// insertion or a removal shifts. A thousand adds at the front of a long array passed both
    /// budgets while moving the whole array each time.
    #[test]
    fn json_patch_charges_array_shifts() {
        let long = Value::Array(vec![json!(0); 100_000]);
        let budget = 8 << 20;
        let at = |op: &str, path: &str| match op {
            "remove" => json!({"op": "remove", "path": path}),
            _ => json!({"op": op, "path": path, "value": 1}),
        };
        for (op, path) in [("add", "/0"), ("remove", "/0"), ("replace", "/0")] {
            let few = Value::Array(vec![at(op, path); 3]);
            assert!(json_patch(&long, &few, budget).is_ok(), "{op}");
            let many = Value::Array(vec![at(op, path); MAX_PATCH_OPS]);
            assert_eq!(
                json_patch(&long, &many, budget),
                Err(PatchError::TooLarge),
                "{op}"
            );
        }
        let moves = json!({"op": "move", "from": "/0", "path": "/1"});
        let many = Value::Array(vec![moves; MAX_PATCH_OPS]);
        assert_eq!(json_patch(&long, &many, budget), Err(PatchError::TooLarge));
        // Appends and removals at the end move nothing.
        let appends = Value::Array(vec![at("add", "/-"); MAX_PATCH_OPS]);
        assert!(json_patch(&long, &appends, budget).is_ok());
        let last = format!("/{}", 100_000 - 1);
        let ops = Value::Array(vec![at("replace", &last); MAX_PATCH_OPS]);
        assert!(json_patch(&long, &ops, budget).is_ok());
    }

    /// Review finding: a JSON Patch could repeat a costly operation without end within the size
    /// budget (a copy of a large value over itself replaces it, so the document never grows). The
    /// operations are counted, and the work they do is held to a budget of its own.
    #[test]
    fn json_patch_work_is_bounded() {
        let big = json!({ "a": "x".repeat(1 << 20) });
        let over_itself = json!({"op": "copy", "from": "/a", "path": "/a"});
        // A few are fine; enough to do many times the budget's work are refused.
        let few = Value::Array(vec![over_itself.clone(); 3]);
        assert_eq!(json_patch(&big, &few, 8 << 20), Ok(big.clone()));
        let many = Value::Array(vec![over_itself; 100]);
        assert_eq!(json_patch(&big, &many, 8 << 20), Err(PatchError::TooLarge));
        // Tests count too.
        let probe = json!({"op": "test", "path": "/a", "value": "x".repeat(1 << 20)});
        let many = Value::Array(vec![probe; 100]);
        assert_eq!(json_patch(&big, &many, 8 << 20), Err(PatchError::TooLarge));
        // And so do operations, however cheap.
        let cheap = json!({"op": "test", "path": "/b", "value": 1});
        let ops = Value::Array(vec![cheap.clone(); MAX_PATCH_OPS]);
        assert_eq!(
            json_patch(&json!({"b": 1}), &ops, PATCH_BUDGET),
            Ok(json!({"b": 1}))
        );
        let ops = Value::Array(vec![cheap; MAX_PATCH_OPS + 1]);
        assert_eq!(
            json_patch(&json!({"b": 1}), &ops, PATCH_BUDGET),
            Err(PatchError::TooLarge)
        );
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
        // Far past the bound: refused, not a stack overflow (by the operation count first, and by
        // the depth bound for as many as are allowed).
        assert_eq!(
            json_patch(&doc, &nesting_ops(100_000), PATCH_BUDGET),
            Err(PatchError::TooLarge)
        );
        assert_eq!(
            json_patch(&doc, &nesting_ops(MAX_PATCH_OPS / 3), PATCH_BUDGET),
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

    /// Review finding: a resource PATCH was held to the fixed [`PATCH_BUDGET`], so a
    /// small patch could grow a document far past the configured body limit. Its result is now
    /// held to `max_body`.
    #[tokio::test]
    async fn a_patch_is_held_to_the_body_limit() {
        let mut cfg = super::super::LwsConfig::new(BASE);
        cfg.open = true;
        cfg.max_body = 64 << 10;
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let st = LwsState::new(store, cfg).await.expect("state");
        let doc = json!({ "a": "x".repeat(20 << 10) }).to_string();
        let json = ("content-type", "application/json");
        let r = call(&st, "POST", "/", &[json, ("slug", "d.json")], &doc).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let patch = ("content-type", "application/json-patch+json");
        let copies = |n: usize| {
            let ops: Vec<Value> = (0..n)
                .map(|i| json!({"op": "copy", "from": "/a", "path": format!("/b{i}")}))
                .collect();
            Value::Array(ops).to_string()
        };
        let r = call(&st, "PATCH", "/d.json", &[patch], &copies(1)).await;
        assert!(r.status().is_success(), "{}", r.status());
        let r = call(&st, "PATCH", "/d.json", &[patch], &copies(4)).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
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
    }

    /// Review findings: numbers were parsed into `f64` and written back, so an integer past
    /// `u64` or a decimal with more digits than an `f64` holds was rewritten by a patch that
    /// never touched it; a `test` compared an integer with a float through a cast, so it matched
    /// a number it does not equal; and a guard refusing such numbers read them with another
    /// float parser than the patch did. A patch never reads a number as a float: each is kept as
    /// its text, and compared as the decimal value it denotes.
    #[tokio::test]
    async fn json_patch_keeps_every_number_exactly() {
        // The text round trip, a string value or a member name that starts with the mark
        // included (members in the order a written document has them).
        let text = "{\"a\":[1e400,-0.9299999999999999,12345678901234567890123],\"e\":\"\\ue000w\",\"s\":\"1\",\"\u{E000}k\":\"\u{E000}v\"}";
        let held = numbers_as_text(text);
        let parsed: Value = serde_json::from_str(&held).unwrap();
        assert_eq!(parsed["a"][0], json!("\u{E000}1e400"));
        assert_eq!(parsed["\u{E000}k"], json!("\u{E000}\u{E000}v"));
        assert_eq!(parsed["e"], json!("\u{E000}\u{E000}w"));
        assert_eq!(parsed["s"], json!("1"));
        assert_eq!(
            numbers_from_text(&serde_json::to_string(&parsed).unwrap()),
            "{\"a\":[1e400,-0.9299999999999999,12345678901234567890123],\"e\":\"\u{E000}w\",\"s\":\"1\",\"\u{E000}k\":\"\u{E000}v\"}"
        );
        // Not a number: left for the parser to refuse.
        for bad in ["01", "1.", ".5", "1e", "--1", "1.2.3"] {
            assert!(!is_json_number(bad), "{bad}");
            assert!(serde_json::from_str::<Value>(&numbers_as_text(&format!("[{bad}]"))).is_err());
        }

        let st = state().await;
        let stored = r#"{"big":1e400,"f":0.9299999999999999,"id":12345678901234567890123,"n":1000000000000001024}"#;
        let uri = post(&st, "big.json", JSON, stored, &[]).await;
        let p = path_of(&uri);
        let body = || async { st.store.read(&uri).await.unwrap().body };
        // An empty patch writes every number back as it was.
        let r = call(&st, "PATCH", p, &[("content-type", JSON_PATCH)], "[]").await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(body().await, Bytes::from(stored));
        // `test` compares decimal values: a float near an integer does not equal it, and the
        // same value written another way does.
        for (value, equal) in [
            ("1.000000000000001e18", false),
            ("1000000000000001024.0", true),
            ("1.000000000000001024e18", true),
        ] {
            let patch = format!(r#"[{{"op":"test","path":"/n","value":{value}}}]"#);
            let r = call(&st, "PATCH", p, &[("content-type", JSON_PATCH)], &patch).await;
            let want = if equal {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            assert_eq!(r.status(), want, "{value}");
        }
        // A patch's numbers are stored as written, and 1e400 is just another number.
        let patch = r#"[{"op":"add","path":"/x","value":1e400},{"op":"add","path":"/y","value":0.1000000000000000055511151231257827}]"#;
        let r = call(&st, "PATCH", p, &[("content-type", JSON_PATCH)], patch).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let written = String::from_utf8(body().await.to_vec()).unwrap();
        assert!(
            written.starts_with(&stored[..stored.len() - 1]),
            "{written}"
        );
        assert!(
            written.ends_with(r#""x":1e400,"y":0.1000000000000000055511151231257827}"#),
            "{written}"
        );
        // A number is never equal to a string holding its text.
        let r = call(
            &st,
            "PATCH",
            p,
            &[("content-type", JSON_PATCH)],
            r#"[{"op":"test","path":"/x","value":"1e400"}]"#,
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Review finding: a JSON Patch on a resource stored as another format rewrote it as JSON
    /// when its content happened to parse. Only a JSON resource is patched; the format is checked
    /// (415) before the preconditions (412), and an operation that names a member twice is a 400.
    #[tokio::test]
    async fn json_patch_applies_to_json_resources_only() {
        let st = state().await;
        let uri = post(&st, "n.txt", "text/plain", "{\"a\":1}", &[]).await;
        let p = path_of(&uri);
        let add = r#"[{"op":"add","path":"/b","value":2}]"#;
        let r = call(&st, "PATCH", p, &[("content-type", JSON_PATCH)], add).await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(hdr(&r, "accept-patch"), JSON_PATCH);
        let stored = st.store.read(&uri).await.unwrap();
        assert_eq!(stored.body, Bytes::from("{\"a\":1}"));
        assert_eq!(stored.meta.content_type, "text/plain");
        // 415 before 412, for the patch format and for the resource's.
        let stale = ("if-match", "\"stale\"");
        let r = call(&st, "PATCH", p, &[("content-type", JSON_PATCH), stale], add).await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let json = post(&st, "j.json", "application/ld+json", "{\"a\":1}", &[]).await;
        let jp = path_of(&json);
        let r = call(
            &st,
            "PATCH",
            jp,
            &[("content-type", "text/plain"), stale],
            add,
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let r = call(
            &st,
            "PATCH",
            jp,
            &[("content-type", JSON_PATCH), stale],
            add,
        )
        .await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        // A `+json` type is JSON, and keeps its type.
        let r = call(&st, "PATCH", jp, &[("content-type", JSON_PATCH)], add).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            st.store.read(&json).await.unwrap().meta.content_type,
            "application/ld+json"
        );
        // A repeated member is refused, whichever of its values would have been kept.
        let r = call(
            &st,
            "PATCH",
            jp,
            &[("content-type", JSON_PATCH)],
            r#"[{"op":"test","path":"/a","value":1,"op":"remove"}]"#,
        )
        .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn patches() {
        let t = json!({"title": "a", "keep": 1, "drop": true});
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
}
