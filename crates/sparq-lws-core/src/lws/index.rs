//! The Type Index Service and the Type Search Service (lws10-index).
//!
//! - `GET /.lws/types/index` lists, in one `TypeIndex` document, every type of every resource the
//!   client may read now: `lws:Container` / `lws:DataResource` and the types clients declared.
//! - `QUERY /.lws/types/search` (RFC 10008, `application/lws-query+json`) takes a filter whose
//!   keys are `type` or a link relation and whose values are lists in conjunctive normal form: each
//!   element is an IRI or a non-empty array of IRIs (an OR group), and every group of every key
//!   must match. The answer is a `ContainerPage` of matching resources, paged at
//!   [`LwsConfig::page_size`](super::LwsConfig) with stateless page links (the filter,
//!   base64url-encoded, and a page number) that are dereferenced with GET.
//!
//! Both services are authorization-filtered on every request and nothing is cached, so a revoked
//! grant drops a resource from the very next response (section 8). Descriptive relations are
//! matched against the resource's Link headers and its linkset alike; structural relations are
//! never indexed, so a filter key naming one matches nothing (section 7.1).

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{json, Value};

use super::access::Action;
use super::{
    add_page_links, is_uri, jose, json_response, method_not_allowed, problem, requested_page, set,
    Agent, LwsRequest, LwsState, ResourceMeta, LD_JSON, LWS_CONTEXT, LWS_JSON, LWS_NS, META_SUFFIX,
    TYPE_INDEX_PATH, TYPE_SEARCH_PATH,
};
use crate::store::Store;

/// The filter media type the Type Search Service accepts.
pub const LWS_QUERY: &str = "application/lws-query+json";

/// Groups a type search may hold before it is refused with 422 (section 7.2).
pub const MAX_FILTER_GROUPS: usize = 32;

/// Most IRIs a search filter's groups may hold between them: each is matched against every
/// readable resource.
pub const MAX_FILTER_IRIS: usize = 256;

/// Largest filter, in bytes. Every page link of a result set carries the filter, base64url-encoded
/// (a third larger), so a filter is held to a size whose links fit well inside a request head.
pub const MAX_FILTER_BYTES: usize = 8 * 1024;

/// Relations the search never indexes: structural or protocol ones (section 7.1).
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
    "https://www.w3.org/ns/lws#storage",
    "storagedescription",
];

/// A filter in conjunctive normal form: every group must match, a group by any of its IRIs.
pub type Groups = Vec<Vec<String>>;

/// A parsed type search filter.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filter {
    /// Groups over the resource's types.
    pub types: Groups,
    /// Groups over a relation's targets, by the relation as the client named it.
    pub relations: BTreeMap<String, Groups>,
}

/// Why a filter was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterError {
    /// Not a JSON object, or a value that breaks the grammar: 400.
    Malformed,
    /// More than [`MAX_FILTER_GROUPS`] groups, or [`MAX_FILTER_IRIS`] IRIs in them: 422.
    TooManyGroups,
}

/// Parse a filter body: a JSON object (`{}` is the empty filter, which matches every readable
/// resource; an empty body is no JSON object, so it is malformed). Keys starting with `@` (a
/// JSON-LD context, say) are ignored.
pub fn parse_filter(bytes: &[u8]) -> Result<Filter, FilterError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| FilterError::Malformed)?;
    let Value::Object(fields) = value else {
        return Err(FilterError::Malformed);
    };
    let mut filter = Filter::default();
    let mut count = 0;
    let mut iris = 0;
    for (key, v) in &fields {
        if key.starts_with('@') {
            continue;
        }
        // The limit is enforced while the groups are read, so a filter of many groups is refused
        // after at most one past the limit, not after all of them were collected.
        let parsed = groups(v, MAX_FILTER_GROUPS - count)?;
        count += parsed.len();
        iris += parsed.iter().map(Vec::len).sum::<usize>();
        if iris > MAX_FILTER_IRIS {
            return Err(FilterError::TooManyGroups);
        }
        if key == "type" {
            filter.types.extend(parsed);
        } else if !parsed.is_empty() {
            filter
                .relations
                .entry(key.clone())
                .or_default()
                .extend(parsed);
        }
    }
    Ok(filter)
}

/// A key's value as CNF groups: an array whose elements are absolute IRIs or non-empty arrays of
/// them (otherwise [`FilterError::Malformed`]). Duplicate groups count once (a set finds them, so
/// many duplicates cost linear time), and more than `room` distinct groups is
/// [`FilterError::TooManyGroups`] as soon as the first one past it is read.
fn groups(value: &Value, room: usize) -> Result<Groups, FilterError> {
    let mut out: Groups = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for element in value.as_array().ok_or(FilterError::Malformed)? {
        let group: Vec<String> = match element {
            Value::String(s) => vec![s.clone()],
            Value::Array(members) if !members.is_empty() => members
                .iter()
                .map(|m| m.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .ok_or(FilterError::Malformed)?,
            _ => return Err(FilterError::Malformed),
        };
        if !group.iter().all(|v| is_uri(v)) {
            return Err(FilterError::Malformed);
        }
        if seen.insert(group.clone()) {
            if out.len() == room {
                return Err(FilterError::TooManyGroups);
            }
            out.push(group);
        }
    }
    Ok(out)
}

/// A relation as it is compared: lower-cased when it is a registered (non-URI) relation.
fn relation_key(rel: &str) -> String {
    if rel.contains(':') {
        rel.to_string()
    } else {
        rel.to_ascii_lowercase()
    }
}

/// Resolve `target` against `base`, leaving it as it is when it cannot be resolved.
fn resolve(base: &str, target: &str) -> Option<String> {
    if is_uri(target) {
        return Some(target.to_string());
    }
    url::Url::parse(base)
        .ok()?
        .join(target)
        .ok()
        .map(|u| u.to_string())
}

/// A resource's targets for a relation, from its Link headers and its linkset alike: every
/// source of a relation is treated the same (section 7.1). A structural relation has none.
pub fn relation_targets(uri: &str, meta: &ResourceMeta, rel: &str) -> BTreeSet<String> {
    let key = relation_key(rel);
    let mut out = BTreeSet::new();
    if STRUCTURAL_RELATIONS.contains(&key.as_str()) {
        return out;
    }
    for (r, targets) in &meta.links {
        if relation_key(r) == key {
            out.extend(targets.iter().cloned());
        }
    }
    let entries = meta
        .linkset
        .as_ref()
        .and_then(|l| l.get("linkset"))
        .and_then(Value::as_array);
    for entry in entries.into_iter().flatten() {
        if entry.get("anchor").and_then(Value::as_str) != Some(uri) {
            continue;
        }
        let Some(fields) = entry.as_object() else {
            continue;
        };
        for (k, targets) in fields {
            if relation_key(k) != key {
                continue;
            }
            for target in targets.as_array().into_iter().flatten() {
                // A linkset may hold a target that is no URI; it matches nothing.
                if let Some(href) = target
                    .get("href")
                    .and_then(Value::as_str)
                    .and_then(|h| resolve(uri, h))
                {
                    out.insert(href);
                }
            }
        }
    }
    out
}

/// Whether every group matches: some IRI of each group is in `have`.
fn all_groups(groups: &Groups, have: impl Fn(&str) -> bool) -> bool {
    groups.iter().all(|g| g.iter().any(|v| have(v)))
}

/// How much the index or a search keeps while it walks the storage: the URIs and types it
/// collects. What is past it fails the request, explicitly, rather than growing with the storage.
pub const MAX_INDEX_WORKING_SET: usize = 16 << 20;

/// Why the index could not be built.
enum Failed {
    /// A listing, a permission check or the metadata of a resource could not be read.
    Store,
    /// What it would keep is past [`MAX_INDEX_WORKING_SET`].
    TooLarge,
}

/// What a walk of the storage keeps, against one budget: the URIs it has yet to visit, the
/// listing it is reading, and whatever its visitor keeps. Past the budget, [`Failed::TooLarge`].
struct Kept {
    bytes: std::sync::atomic::AtomicUsize,
    budget: usize,
}

impl Kept {
    fn charge(&self, n: usize) -> Result<(), Failed> {
        use std::sync::atomic::Ordering::Relaxed;
        let bytes = self.bytes.load(Relaxed).saturating_add(n);
        self.bytes.store(bytes, Relaxed);
        if bytes > self.budget {
            Err(Failed::TooLarge)
        } else {
            Ok(())
        }
    }

    /// What the budget has left.
    fn left(&self) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        self.budget.saturating_sub(self.bytes.load(Relaxed))
    }

    fn refund(&self, n: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        self.bytes
            .store(self.bytes.load(Relaxed).saturating_sub(n), Relaxed);
    }
}

/// Visit every resource the agent may read now, with its types (full IRIs) and its metadata;
/// an error when a listing, a permission check or the metadata of one cannot be read, or when
/// `visit` refuses. Each resource's shared lock is held from its permission check through the
/// read of its metadata and its visit, so the types and relations seen are those of the state
/// the check allowed (a delete and a re-create by someone else cannot slip in between); the lock
/// is released, and the metadata dropped, before the next resource.
///
/// The walk keeps no record of where it has been: the storage is a tree, and a container's
/// members are only followed when they lie strictly below it, so nothing is visited twice. What
/// it does keep, the URIs still to visit and the listing in hand, is charged to `kept` with what
/// `visit` keeps, and a listing is only read while it fits what is left.
async fn readable<S: Store + 'static>(
    state: &LwsState<S>,
    agent: &Agent,
    kept: &Kept,
    mut visit: impl FnMut(&str, Vec<String>, &ResourceMeta) -> Result<(), Failed>,
) -> Result<(), Failed> {
    let root = state.cfg.storage();
    kept.charge(root.len())?;
    let mut stack = vec![root];
    while let Some(uri) = stack.pop() {
        kept.refund(uri.len());
        let is_container = uri.ends_with('/');
        // A listing that cannot be read fails the index rather than leaving out what is under
        // it; a container removed meanwhile has nothing to list.
        if is_container {
            // The listing is read only while it fits what the budget has left, so one too large
            // to keep is refused before it is held whole.
            let children = match state.store.list_children_within(&uri, kept.left()).await {
                Ok(Some(c)) => c,
                Ok(None) => return Err(Failed::TooLarge),
                Err(crate::error::ServerError::NotFound) => Vec::new(),
                Err(_) => return Err(Failed::Store),
            };
            let listed: usize = children.iter().map(|c| c.as_str().len()).sum();
            kept.charge(listed)?;
            // A listing that names a member twice is followed once.
            let mut once = std::collections::HashSet::new();
            for child in &children {
                let child = child.as_str();
                if !child.ends_with(META_SUFFIX)
                    && child.len() > uri.len()
                    && child.starts_with(uri.as_str())
                    && once.insert(child)
                {
                    kept.charge(child.len())?;
                    stack.push(child.to_string());
                }
            }
            drop(once);
            drop(children);
            kept.refund(listed);
        }
        let _guard = state.locks.read(&uri).await;
        // A resource removed since it was listed is not in the index: its existence is checked
        // under its lock (its metadata alone does not say, being read as a default when absent).
        match state.store.exists(&uri).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return Err(Failed::Store),
        }
        match state.check(Action::Read, &uri, agent).await {
            Ok(true) => {}
            Ok(false) | Err(crate::error::ServerError::NotFound) => continue,
            Err(_) => return Err(Failed::Store),
        }
        let meta = match state.resource_meta(&uri).await {
            Ok(m) => m,
            Err(crate::error::ServerError::NotFound) => continue,
            Err(_) => return Err(Failed::Store),
        };
        let mut types = vec![format!(
            "{LWS_NS}{}",
            if is_container {
                "Container"
            } else {
                "DataResource"
            }
        )];
        // A resource may state any number of types: repeats are found with a set.
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        seen.insert(&types[0]);
        let more: Vec<String> = meta
            .types
            .iter()
            .filter(|t| seen.insert(t.as_str()))
            .cloned()
            .collect();
        types.extend(more);
        visit(&uri, types, &meta)?;
    }
    Ok(())
}

pub async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
) -> Response {
    handle_within(state, req, agent, MAX_INDEX_WORKING_SET).await
}

/// [`handle`], keeping at most `budget` bytes while the storage is walked.
async fn handle_within<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
    budget: usize,
) -> Response {
    if state.needs_auth(agent) {
        return state.challenge(None);
    }
    let search = req.path == TYPE_SEARCH_PATH;
    let allow = if search {
        "OPTIONS, QUERY"
    } else {
        "GET, HEAD, OPTIONS"
    };
    let method = req.method.as_str();
    let with_accept_query = |mut r: Response| {
        set(
            r.headers_mut(),
            header::HeaderName::from_static("accept-query"),
            LWS_QUERY,
        );
        r
    };
    if req.method == Method::OPTIONS {
        let mut r = problem(StatusCode::NO_CONTENT, None);
        set(r.headers_mut(), header::ALLOW, allow);
        return if search { with_accept_query(r) } else { r };
    }
    let is_get = req.method == Method::GET || req.method == Method::HEAD;
    // A QUERY body with a content coding is refused before it is read, as any body is.
    if let Some(refused) = super::resources::refuse_encoded(req) {
        return refused;
    }
    // A page link of a result set is the filter, base64url-encoded, and a page number: the server
    // keeps nothing, and the link is dereferenced with GET (section 7.1).
    // The query is held to its size before it is decoded at all: a page link's query is the
    // filter, base64url-encoded, and percent-encoding at most triples it.
    if search
        && req.query.as_deref().map_or(0, str::len) > MAX_FILTER_BYTES.div_ceil(3) * 4 * 3 + 64
    {
        return problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            Some(&format!("a filter is at most {MAX_FILTER_BYTES} bytes")),
        );
    }
    // A `q` parameter is the filter of a GET (the page links of a search); a QUERY's filter is
    // its body, always, parsed and checked as such.
    let q = if search && is_get {
        req.query_param("q")
    } else {
        None
    };
    let page_link = q.is_some() && is_get;
    let method_ok = if search {
        method == "QUERY" || page_link
    } else {
        is_get
    };
    if !method_ok {
        let r = method_not_allowed(allow);
        return if search { with_accept_query(r) } else { r };
    }
    let mut filter_bytes = Bytes::new();
    let mut filter = Filter::default();
    if search {
        if let Some(q) = &q {
            // Held to its size before it is decoded: base64url is four characters for three bytes.
            if q.len() > MAX_FILTER_BYTES.div_ceil(3) * 4 {
                return problem(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Some(&format!("a filter is at most {MAX_FILTER_BYTES} bytes")),
                );
            }
            match jose::b64url_decode(q) {
                Some(b) => filter_bytes = Bytes::from(b),
                None => return problem(StatusCode::NOT_FOUND, None),
            }
        } else {
            match req.content_type() {
                None => {
                    return problem(
                        StatusCode::BAD_REQUEST,
                        Some("a type search needs a Content-Type"),
                    )
                }
                Some(ct) if ct != LWS_QUERY => {
                    return with_accept_query(problem(StatusCode::UNSUPPORTED_MEDIA_TYPE, None))
                }
                Some(_) => filter_bytes = req.body.clone(),
            }
        }
        if filter_bytes.len() > MAX_FILTER_BYTES {
            return problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                Some(&format!("a filter is at most {MAX_FILTER_BYTES} bytes")),
            );
        }
        filter = match parse_filter(&filter_bytes) {
            Ok(f) => f,
            Err(FilterError::Malformed) => {
                return problem(
                    StatusCode::BAD_REQUEST,
                    Some("the filter does not follow the lws-query grammar"),
                )
            }
            Err(FilterError::TooManyGroups) => {
                return problem(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some(&format!(
                        "a filter may hold at most {MAX_FILTER_GROUPS} groups of {MAX_FILTER_IRIS} IRIs in all"
                    )),
                )
            }
        };
    }
    // lws+json or ld+json, negotiated by quality and specificity (lws+json on a tie, or without
    // Accept); a range with q=0 refuses its type. The response varies on Accept (section 7.2).
    let Some(media_type) =
        super::resources::negotiate(req.header(header::ACCEPT), &[LWS_JSON, LD_JSON])
    else {
        return problem(StatusCode::NOT_ACCEPTABLE, None);
    };
    let media_type = media_type.as_str();
    // A failure says nothing of the resource it met: its URI, and so its existence, may be
    // something the agent may not read. Every failure gets the same response.
    // What is kept while the storage is walked is charged against one budget.
    let kept = Kept {
        bytes: std::sync::atomic::AtomicUsize::new(0),
        budget,
    };
    let charge = |bytes: usize| kept.charge(bytes);
    let mut items: Vec<(String, Vec<String>)> = Vec::new();
    let mut all_types: BTreeSet<String> = BTreeSet::new();
    let walked = if search {
        // A search keeps only what it matched, with its types.
        readable(state, agent, &kept, |uri, types, meta| {
            let have: std::collections::HashSet<&str> = types.iter().map(String::as_str).collect();
            let mut matched = all_groups(&filter.types, |v| have.contains(v));
            if matched && !filter.relations.is_empty() {
                matched = filter.relations.iter().all(|(rel, groups)| {
                    let targets = relation_targets(uri, meta, rel);
                    all_groups(groups, |v| targets.contains(v))
                });
            }
            drop(have);
            if matched {
                charge(uri.len() + types.iter().map(String::len).sum::<usize>())?;
                items.push((uri.to_string(), types));
            }
            Ok(())
        })
        .await
    } else {
        // The index keeps only the distinct types.
        readable(state, agent, &kept, |_, types, _| {
            for t in types {
                if !all_types.contains(&t) {
                    charge(t.len())?;
                    all_types.insert(t);
                }
            }
            Ok(())
        })
        .await
    };
    // A failure says nothing of the resource it met: its URI, and so its existence, may be
    // something the agent may not read. Every failure of the store gets the same response.
    match walked {
        Ok(()) => {}
        Err(Failed::Store) => {
            return problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                Some("the index could not be built"),
            );
        }
        Err(Failed::TooLarge) => {
            return problem(
                StatusCode::INSUFFICIENT_STORAGE,
                Some("the index is larger than this server builds for one request"),
            );
        }
    }
    let mut links = Vec::new();
    let mut content_location = None;
    let doc = if search {
        // Pages are in URI order.
        items.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let size = state.cfg.page_size.max(1);
        let total = items.len();
        let pages = total.div_ceil(size).max(1);
        let page = if page_link {
            match req.query_param("page") {
                None => 1,
                Some(p) => p.parse::<usize>().unwrap_or(0),
            }
        } else {
            1
        };
        if page < 1 || page > pages {
            return problem(StatusCode::NOT_FOUND, None);
        }
        let base = format!(
            "{}?q={}&page=",
            state.cfg.absolute(TYPE_SEARCH_PATH),
            jose::b64url(&filter_bytes)
        );
        links.push(format!("<{base}1>; rel=\"first\""));
        if page > 1 {
            links.push(format!("<{base}{}>; rel=\"prev\"", page - 1));
        }
        if !page_link {
            // RFC 10008 lets a QUERY response name a resource representing its results; one that
            // is exposed must be authorization-filtered on every access and not reused across
            // clients. The first page link is such a resource: every GET re-runs the filter for
            // whoever asks.
            content_location = Some(format!("{base}1"));
        }
        if page < pages {
            links.push(format!("<{base}{}>; rel=\"next\"", page + 1));
        }
        links.push(format!("<{base}{pages}>; rel=\"last\""));
        // Only the members a page shows are written out.
        let shown: Vec<Value> = items
            .into_iter()
            .skip((page - 1) * size)
            .take(size)
            .map(|(uri, types)| {
                let types: Vec<Value> = types
                    .iter()
                    .map(|t| Value::String(t.strip_prefix(LWS_NS).unwrap_or(t).to_string()))
                    .collect();
                json!({"id": uri, "type": types})
            })
            .collect();
        json!({"@context": LWS_CONTEXT, "type": "ContainerPage", "totalItems": total, "items": shown})
    } else {
        // The TypeIndex is paged like a container (section 6.1): opaque `?page=N` links in Link
        // headers, first and last always, prev and next where there is one; a page that does not
        // exist (any more) is 404.
        let types = all_types;
        let total = types.len();
        let size = state.cfg.page_size.max(1);
        let pages = total.div_ceil(size).max(1);
        let Some(page) = requested_page(req, pages) else {
            return problem(StatusCode::NOT_FOUND, None);
        };
        let index = state.cfg.absolute(TYPE_INDEX_PATH);
        links.extend(page_links(&index, page, pages));
        let items: Vec<Value> = types
            .into_iter()
            .skip((page - 1) * size)
            .take(size)
            .map(|t| json!({"id": t}))
            .collect();
        json!({"@context": LWS_CONTEXT, "type": "TypeIndex", "totalItems": total, "items": items})
    };
    // The validator is of the representation served: its media type, its links and its body. A
    // GET of a page and a QUERY evaluate preconditions against it, as a read does.
    let body = serde_json::to_string(&doc).unwrap_or_default();
    let etag = super::etag_of(
        std::iter::once(media_type)
            .chain(links.iter().map(String::as_str))
            .chain(std::iter::once(body.as_str())),
    );
    let mut resp = match super::resources::read_refusal(req, &etag) {
        Some(StatusCode::NOT_MODIFIED) => StatusCode::NOT_MODIFIED.into_response(),
        Some(refused) => problem(refused, None),
        None => json_response(StatusCode::OK, media_type, &doc),
    };
    let h = resp.headers_mut();
    set(h, header::ETAG, &etag);
    for l in links {
        if let Ok(v) = header::HeaderValue::from_str(&l) {
            h.append(header::LINK, v);
        }
    }
    if let Some(cl) = content_location {
        set(h, header::CONTENT_LOCATION, &cl);
    }
    // Authorization-filtered, so never to be reused for another client (section 8).
    set(h, header::CACHE_CONTROL, "private, no-store");
    set(h, header::VARY, "Authorization, Accept");
    resp
}

/// The Link header values of page `page` of `pages` at `base?page=N`.
fn page_links(base: &str, page: usize, pages: usize) -> Vec<String> {
    let mut headers = header::HeaderMap::new();
    add_page_links(&mut headers, base, page, pages);
    headers
        .get_all(header::LINK)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Filter, FilterError> {
        parse_filter(s.as_bytes())
    }

    /// Review finding: an empty QUERY body was read as `{}`, an unrestricted search. A filter is
    /// a JSON object; `{}` still is the empty filter.
    #[test]
    fn an_empty_body_is_no_filter() {
        assert_eq!(parse(""), Err(FilterError::Malformed));
        assert_eq!(parse("{}"), Ok(Filter::default()));
    }

    /// Review finding: every page link carried the whole filter as sent, so a filter padded to a
    /// megabyte gave links no request head can carry. A filter is held to its size first.
    #[tokio::test]
    async fn search_filters_are_held_to_their_size() {
        use super::super::test_store;
        let (state, _) = test_store::state(1).await;
        let filter = format!(
            "{{\"type\": [\"https://e.example/T\"], \"@padding\": \"{}\"}}",
            "x".repeat(MAX_FILTER_BYTES)
        );
        let path = format!(
            "{TYPE_SEARCH_PATH}?q={}&page=1",
            jose::b64url(filter.as_bytes())
        );
        let r = handle(
            &state,
            &test_store::request(Method::GET, &path, &[], ""),
            &Agent::anonymous(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn the_type_index_is_paged() {
        use super::super::test_store;
        let (state, _) = test_store::state(1).await;
        let root = state.cfg.storage();
        state
            .store
            .create_in_container(&root, &format!("{root}a"), "x".into(), "text/plain")
            .await
            .unwrap();
        let get =
            |q: &str| test_store::request(Method::GET, &format!("{TYPE_INDEX_PATH}{q}"), &[], "");
        let index = state.cfg.absolute(TYPE_INDEX_PATH);
        let first = handle(&state, &get(""), &Agent::anonymous()).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(
            test_store::links(&first, "first"),
            vec![format!("{index}?page=1")]
        );
        assert_eq!(
            test_store::links(&first, "next"),
            vec![format!("{index}?page=2")]
        );
        assert_eq!(
            test_store::links(&first, "last"),
            vec![format!("{index}?page=2")]
        );
        assert!(test_store::links(&first, "prev").is_empty());
        let doc = test_store::body_json(first).await;
        assert_eq!(doc["totalItems"], 2);
        assert_eq!(doc["items"].as_array().unwrap().len(), 1);
        let second = handle(&state, &get("?page=2"), &Agent::anonymous()).await;
        assert_eq!(
            test_store::links(&second, "prev"),
            vec![format!("{index}?page=1")]
        );
        assert!(test_store::links(&second, "next").is_empty());
        let other = test_store::body_json(second).await;
        assert_ne!(other["items"], doc["items"]);
        let gone = handle(&state, &get("?page=3"), &Agent::anonymous()).await;
        assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    }

    /// Review finding: a resource removed after its container was listed and before its lock
    /// was taken was still found (its metadata read as a default, its permission check passed),
    /// with a made-up type. Its existence is checked under its lock.
    #[tokio::test]
    async fn a_removed_resource_is_not_found() {
        use super::super::test_store;
        let (state, _) = test_store::state(100).await;
        let root = state.cfg.storage();
        for name in ["a", "b"] {
            state
                .store
                .create_in_container(&root, &format!("{root}{name}"), "x".into(), "text/plain")
                .await
                .unwrap();
        }
        // b goes, but stays in its container's listing, as when it is removed between the
        // listing and its visit.
        let b = format!("{root}b");
        state.store.delete(&b, None).await.unwrap();
        assert!(state
            .store
            .list_children(&root)
            .await
            .unwrap()
            .iter()
            .any(|c| c.as_str() == b));
        let path = format!("{TYPE_SEARCH_PATH}?q={}", jose::b64url(b"{}"));
        let r = handle(
            &state,
            &test_store::request(Method::GET, &path, &[], ""),
            &Agent::anonymous(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let doc = test_store::body_json(r).await;
        let ids: Vec<&str> = doc["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["id"].as_str())
            .collect();
        assert!(ids.contains(&format!("{root}a").as_str()), "{ids:?}");
        assert!(!ids.contains(&b.as_str()), "{ids:?}");
    }

    /// Review finding: the type search checked a resource's permission and then read its
    /// metadata (and, for relations, read it again) without its lock, so a delete and a re-create
    /// in between leaked the replacement's private types. The check and the reads now hold the
    /// resource's shared lock, and relations come from the same snapshot.
    #[tokio::test]
    async fn the_type_index_reads_each_resource_under_its_lock() {
        use super::super::test_store::{self, FlakyStore};
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let state = LwsState::new(FlakyStore::new(), cfg).await.unwrap();
        let root = state.cfg.storage();
        let uri = format!("{root}bobs");
        state
            .store
            .create_in_container(&root, &uri, "x".into(), "text/plain")
            .await
            .unwrap();
        let set_meta = |creator: &str, ty: &str| {
            let (state, uri) = (state.clone(), uri.clone());
            let (creator, ty) = (creator.to_string(), ty.to_string());
            async move {
                let mut meta = state.resource_meta(&uri).await.unwrap();
                meta.creator = Some(creator);
                meta.types = vec![ty];
                state.put_resource_meta(&uri, &meta).await.unwrap();
            }
        };
        set_meta("https://bob.example/#me", "https://e.example/Public").await;
        let bob = Agent {
            subject: Some("https://bob.example/#me".into()),
            client: None,
        };
        let held = state.locks.lock(&uri).await;
        let task = {
            let (state, bob) = (state.clone(), bob.clone());
            tokio::spawn(async move {
                let get = test_store::request(Method::GET, TYPE_INDEX_PATH, &[], "");
                test_store::body_json(handle(&state, &get, &bob).await).await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!task.is_finished(), "the index did not wait for the lock");
        // Deleted and re-created by the owner, with a private type.
        set_meta("https://owner.example/#me", "https://e.example/Secret").await;
        drop(held);
        let doc = task.await.unwrap();
        let ids: Vec<&str> = doc["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["id"].as_str())
            .collect();
        assert!(!ids.contains(&"https://e.example/Secret"), "{ids:?}");
        assert!(!ids.contains(&"https://e.example/Public"), "{ids:?}");
    }

    /// Review finding: the index and searches kept every readable resource's whole metadata
    /// while they walked the storage, however little they returned. A search keeps only what it
    /// matched and the index only the distinct types, against one budget; past it, `507`.
    #[tokio::test]
    async fn the_walk_keeps_only_what_it_returns() {
        use super::super::test_store;
        let (state, _store) = test_store::state(4).await;
        let root = state.cfg.storage();
        for i in 0..200 {
            state
                .store
                .create_in_container(&root, &format!("{root}r{i}"), "x".into(), "text/plain")
                .await
                .unwrap();
        }
        let anyone = Agent::anonymous();
        let query = |body: &str| {
            let mut r = test_store::request(
                Method::GET,
                TYPE_SEARCH_PATH,
                &[("content-type", LWS_QUERY)],
                body,
            );
            r.method = Method::from_bytes(b"QUERY").unwrap();
            r
        };
        // Room for the listing in hand and the members still to visit (each charged once), and
        // for a few matches more, not two hundred.
        let listed: usize = state
            .store
            .list_children(&root)
            .await
            .unwrap()
            .iter()
            .map(|c| c.as_str().len())
            .sum();
        let budget = 2 * (root.len() + listed) + 8 * format!("{root}r10{LWS_NS}DataResource").len();
        let all = handle_within(&state, &query("{}"), &anyone, budget).await;
        assert_eq!(all.status(), StatusCode::INSUFFICIENT_STORAGE);
        // A search that matches nothing keeps nothing, and the index keeps two types.
        let none = query(r#"{"type": ["https://e.example/Nothing"]}"#);
        let r = handle_within(&state, &none, &anyone, budget).await;
        assert_eq!(r.status(), StatusCode::OK);
        let get = test_store::request(Method::GET, TYPE_INDEX_PATH, &[], "");
        let r = handle_within(&state, &get, &anyone, budget).await;
        assert_eq!(r.status(), StatusCode::OK);
        // Review finding: what the walk itself holds (the URIs it has yet to visit) was not
        // charged, so a search matching nothing could hold any amount. A listing larger than the
        // budget fails the walk, whatever matches.
        // Review finding: the listing was held whole before it was charged. It is read only
        // while it fits what the budget has left.
        assert!(state
            .store
            .list_children_within(&root, listed - 1)
            .await
            .unwrap()
            .is_none());
        assert!(state
            .store
            .list_children_within(&root, listed)
            .await
            .unwrap()
            .is_some());
        let tiny = root.len() + listed / 2;
        assert_eq!(
            handle_within(&state, &none, &anyone, tiny).await.status(),
            StatusCode::INSUFFICIENT_STORAGE
        );
        // Under the real budget, all two hundred are found.
        assert_eq!(
            handle(&state, &query("{}"), &anyone).await.status(),
            StatusCode::OK
        );
    }

    /// Review findings: a `q` parameter replaced a QUERY's body, so the body was never checked;
    /// a failure named the resource it met, which may be one the agent cannot read; and a
    /// container whose listing failed was left out silently. A QUERY's filter is its body, every
    /// failure gets one response that names nothing, and a failed listing fails the index.
    #[tokio::test]
    async fn the_type_index_fails_whole_and_names_nothing() {
        use super::super::test_store::{self, FlakyStore};
        let mut cfg = super::super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some("https://owner.example/#me".into());
        let state = LwsState::new(FlakyStore::new(), cfg).await.unwrap();
        let root = state.cfg.storage();
        let (dir, secret) = (format!("{root}dir/"), format!("{root}dir/secret"));
        state
            .store
            .create_in_container(&root, &dir, Bytes::new(), LWS_JSON)
            .await
            .unwrap();
        state
            .store
            .create_in_container(&dir, &secret, "x".into(), "text/plain")
            .await
            .unwrap();
        let bob = Agent {
            subject: Some("https://bob.example/#me".into()),
            client: None,
        };
        let query = |q: Option<&str>, body: &str| {
            let path = match q {
                Some(q) => format!("{TYPE_SEARCH_PATH}?q={}", jose::b64url(q.as_bytes())),
                None => TYPE_SEARCH_PATH.to_string(),
            };
            let mut r =
                test_store::request(Method::GET, &path, &[("content-type", LWS_QUERY)], body);
            r.method = Method::from_bytes(b"QUERY").unwrap();
            r
        };
        let r = handle(&state, &query(Some("{}"), "not a filter"), &bob).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = handle(&state, &query(None, r#"{"type": ["https://e/%ZZ"]}"#), &bob).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            handle(&state, &query(None, "{}"), &bob).await.status(),
            StatusCode::OK
        );
        let get = test_store::request(Method::GET, TYPE_INDEX_PATH, &[], "");
        let failures = [
            (&state.store.fail_read_of, super::super::meta_key(&secret)),
            (&state.store.fail_list_of, dir.clone()),
        ];
        let mut bodies = Vec::new();
        for (fail, of) in failures {
            *fail.lock().unwrap() = Some(of);
            let r = handle(&state, &get, &bob).await;
            *fail.lock().unwrap() = None;
            assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = test_store::body_json(r).await.to_string();
            assert!(!body.contains("secret") && !body.contains("dir/"), "{body}");
            bodies.push(body);
        }
        assert_eq!(bodies[0], bodies[1]);
    }

    /// Review findings: the index and search answered 200 whatever their preconditions, and a
    /// QUERY body's content coding was ignored. Both carry an entity tag of what they serve and
    /// evaluate preconditions against it, and a coded QUERY body is refused.
    #[tokio::test]
    async fn the_type_index_evaluates_preconditions_and_codings() {
        use super::super::test_store;
        let (state, _) = test_store::state(100).await;
        let anyone = Agent::anonymous();
        let query = |headers: &[(&str, &str)]| {
            let mut h = vec![("content-type", LWS_QUERY)];
            h.extend_from_slice(headers);
            let mut r = test_store::request(Method::GET, TYPE_SEARCH_PATH, &h, "{}");
            r.method = Method::from_bytes(b"QUERY").unwrap();
            r
        };
        let get = |headers: &[(&str, &str)]| {
            test_store::request(Method::GET, TYPE_INDEX_PATH, headers, "")
        };
        for build in [&get as &dyn Fn(&[(&str, &str)]) -> LwsRequest, &query] {
            let r = handle(&state, &build(&[]), &anyone).await;
            assert_eq!(r.status(), StatusCode::OK);
            let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
            let status = |h: Vec<(&'static str, String)>| {
                let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let req = build(&h);
                let state = state.clone();
                async move { handle(&state, &req, &Agent::anonymous()).await.status() }
            };
            assert_eq!(
                status(vec![("if-none-match", tag.clone())]).await,
                StatusCode::NOT_MODIFIED
            );
            assert_eq!(
                status(vec![("if-none-match", "*".into())]).await,
                StatusCode::NOT_MODIFIED
            );
            assert_eq!(
                status(vec![("if-match", "\"never-issued\"".into())]).await,
                StatusCode::PRECONDITION_FAILED
            );
            assert_eq!(
                status(vec![("if-match", tag.clone())]).await,
                StatusCode::OK
            );
        }
        let r = handle(&state, &query(&[("content-encoding", "gzip")]), &anyone).await;
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn empty_filters_match_everything() {
        assert_eq!(parse("{}"), Ok(Filter::default()));
        assert_eq!(
            parse(r#"{"@context": "https://www.w3.org/ns/lws/v1", "type": []}"#),
            Ok(Filter::default())
        );
    }

    #[test]
    fn cnf_groups() {
        let f = parse(r#"{"type": ["https://a/T", ["https://a/U", "https://a/V"], "https://a/T"], "https://a/rel": ["https://b/x"]}"#)
            .unwrap();
        assert_eq!(
            f.types,
            vec![
                vec!["https://a/T".to_string()],
                vec!["https://a/U".to_string(), "https://a/V".to_string()]
            ]
        );
        assert_eq!(
            f.relations["https://a/rel"],
            vec![vec!["https://b/x".to_string()]]
        );
        // an empty relation list is no constraint at all
        assert!(parse(r#"{"author": []}"#).unwrap().relations.is_empty());
    }

    #[test]
    fn malformed_filters() {
        for bad in [
            "[]",
            "null",
            "\"x\"",
            "{",
            r#"{"type": "https://a/T"}"#,
            r#"{"type": [[]]}"#,
            r#"{"type": [1]}"#,
            r#"{"type": [["https://a/T", 2]]}"#,
            r#"{"type": [[["https://a/T"]]]}"#,
            r#"{"type": ["relative"]}"#,
            r#"{"type": ["https://a b"]}"#,
            r#"{"author": [{"id": "https://a/T"}]}"#,
        ] {
            assert_eq!(parse(bad), Err(FilterError::Malformed), "{bad}");
        }
    }

    /// Sweep finding: the group limit did not bound the IRIs inside a group, and each is matched
    /// against every readable resource.
    #[test]
    fn filters_hold_a_bounded_number_of_iris() {
        let group = |n: usize| {
            let iris: Vec<String> = (0..n).map(|i| format!("\"https://a/T{i}\"")).collect();
            format!("{{\"type\": [[{}]]}}", iris.join(","))
        };
        assert!(parse(&group(MAX_FILTER_IRIS)).is_ok());
        assert!(matches!(
            parse(&group(MAX_FILTER_IRIS + 1)),
            Err(FilterError::TooManyGroups)
        ));
    }

    #[test]
    fn group_limit() {
        let at: Vec<String> = (0..MAX_FILTER_GROUPS)
            .map(|i| format!("\"https://a/T{i}\""))
            .collect();
        assert!(parse(&format!("{{\"type\": [{}]}}", at.join(","))).is_ok());
        let half: Vec<String> = (0..MAX_FILTER_GROUPS / 2 + 1)
            .map(|i| format!("\"https://a/T{i}\""))
            .collect();
        let over = format!(
            "{{\"type\": [{0}], \"https://a/rel\": [{0}]}}",
            half.join(",")
        );
        assert_eq!(parse(&over), Err(FilterError::TooManyGroups));
        // a malformed value wins over the count
        assert_eq!(parse(r#"{"type": [1]}"#), Err(FilterError::Malformed));
    }

    /// Review finding: groups were deduplicated by a linear scan of those kept and counted only
    /// after every key was read, so a filter of many distinct groups cost quadratic time before
    /// the limit refused it. The limit now stops the read, and duplicates go through a set.
    #[test]
    fn many_groups_are_refused_without_quadratic_work() {
        let many: Vec<String> = (0..50_000).map(|i| format!("\"https://a/T{i}\"")).collect();
        let body = format!("{{\"type\": [{}]}}", many.join(","));
        let started = std::time::Instant::now();
        assert_eq!(parse(&body), Err(FilterError::TooManyGroups));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // Duplicates of one group are one group.
        let same = vec!["[\"https://a/T\", \"https://a/U\"]"; 50_000].join(",");
        let f = parse(&format!("{{\"type\": [{same}]}}")).unwrap();
        assert_eq!(f.types.len(), 1);
    }

    #[test]
    fn targets_from_links_and_linkset() {
        let uri = "https://s/r";
        let mut meta = ResourceMeta::default();
        meta.links
            .insert("author".into(), vec!["https://a/me".into()]);
        meta.links
            .insert("https://x/rel".into(), vec!["https://a/one".into()]);
        meta.linkset = Some(json!({"linkset": [
            {"anchor": uri, "Author": [{"href": "https://a/you"}], "https://x/rel": [{"href": "two"}]},
            {"anchor": "https://s/other", "author": [{"href": "https://a/no"}]},
        ]}));
        let author = relation_targets(uri, &meta, "AUTHOR");
        assert_eq!(
            author.into_iter().collect::<Vec<_>>(),
            vec!["https://a/me", "https://a/you"]
        );
        let x = relation_targets(uri, &meta, "https://x/rel");
        assert_eq!(
            x.into_iter().collect::<Vec<_>>(),
            vec!["https://a/one", "https://s/two"]
        );
        meta.links.insert("up".into(), vec!["https://s/".into()]);
        assert!(relation_targets(uri, &meta, "up").is_empty());
        assert!(relation_targets(uri, &meta, "type").is_empty());
    }

    /// Review finding: the representation was picked by substring, so a type refused with q=0
    /// could still be served. It is negotiated by quality and specificity now.
    #[tokio::test]
    async fn the_type_services_negotiate_by_quality() {
        use super::super::test_store;
        let (state, _) = test_store::state(100).await;
        let served = |accept: Option<&'static str>| {
            let state = state.clone();
            async move {
                let headers: Vec<(&str, &str)> =
                    accept.map(|a| ("accept", a)).into_iter().collect();
                let get = test_store::request(Method::GET, TYPE_INDEX_PATH, &headers, "");
                let r = handle(&state, &get, &Agent::anonymous()).await;
                if r.status() == StatusCode::NOT_ACCEPTABLE {
                    return None;
                }
                Some(
                    r.headers()[header::CONTENT_TYPE]
                        .to_str()
                        .unwrap()
                        .to_string(),
                )
            }
        };
        let lws = Some(LWS_JSON.to_string());
        let ld = Some(LD_JSON.to_string());
        assert_eq!(served(None).await, lws);
        assert_eq!(served(Some("application/ld+json")).await, ld);
        assert_eq!(served(Some("text/html, */*;q=0.1")).await, lws);
        assert_eq!(
            served(Some("application/lws+json;q=0, application/ld+json")).await,
            ld
        );
        assert_eq!(served(Some("application/ld+json;q=0, */*")).await, lws);
        assert_eq!(
            served(Some(
                "application/ld+json;q=0.2, application/lws+json;q=0.1"
            ))
            .await,
            ld
        );
        assert_eq!(served(Some("text/turtle")).await, None);
        assert_eq!(served(Some("application/lws+json;q=0")).await, None);
        assert_eq!(served(Some("*/*;q=0")).await, None);
    }
}
