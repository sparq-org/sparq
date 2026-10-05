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
use axum::response::Response;
use serde_json::{json, Map, Value};

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
    /// More than [`MAX_FILTER_GROUPS`] groups: 422.
    TooManyGroups,
}

/// Parse a filter body. An empty body is the empty filter, which matches every readable resource.
/// Keys starting with `@` (a JSON-LD context, say) are ignored.
pub fn parse_filter(bytes: &[u8]) -> Result<Filter, FilterError> {
    let value: Value = if bytes.is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_slice(bytes).map_err(|_| FilterError::Malformed)?
    };
    let Value::Object(fields) = value else {
        return Err(FilterError::Malformed);
    };
    let mut filter = Filter::default();
    let mut count = 0;
    for (key, v) in &fields {
        if key.starts_with('@') {
            continue;
        }
        // The limit is enforced while the groups are read, so a filter of many groups is refused
        // after at most one past the limit, not after all of them were collected.
        let parsed = groups(v, MAX_FILTER_GROUPS - count)?;
        count += parsed.len();
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

/// What a search sees of one readable resource: its types (full IRIs) and the metadata its
/// relations come from, both from the snapshot its permission check was made on.
struct Seen {
    types: Vec<String>,
    meta: ResourceMeta,
}

/// Every resource the agent may read now, with what a search sees of it, by URI. Each resource's
/// shared lock is held from its permission check through the read of its metadata, so the types
/// and relations collected are those of the state the check allowed (a delete and a re-create by
/// someone else cannot slip in between); the lock is released before the next resource.
async fn readable<S: Store + 'static>(
    state: &LwsState<S>,
    agent: &Agent,
) -> BTreeMap<String, Seen> {
    let mut out = BTreeMap::new();
    let mut stack = vec![state.cfg.storage()];
    let mut seen = BTreeSet::new();
    while let Some(uri) = stack.pop() {
        if !seen.insert(uri.clone()) {
            continue;
        }
        let is_container = uri.ends_with('/');
        if is_container {
            if let Ok(children) = state.store.list_children(&uri).await {
                for child in children {
                    let child = child.as_str();
                    if !child.ends_with(META_SUFFIX) && child.starts_with(uri.as_str()) {
                        stack.push(child.to_string());
                    }
                }
            }
        }
        let _guard = state.locks.read(&uri).await;
        if !state.allowed(Action::Read, &uri, agent).await {
            continue;
        }
        let meta = state.resource_meta(&uri).await;
        let mut types = vec![format!(
            "{LWS_NS}{}",
            if is_container {
                "Container"
            } else {
                "DataResource"
            }
        )];
        for t in &meta.types {
            if !types.contains(t) {
                types.push(t.clone());
            }
        }
        out.insert(uri, Seen { types, meta });
    }
    out
}

pub async fn handle<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    agent: &Agent,
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
    // A page link of a result set is the filter, base64url-encoded, and a page number: the server
    // keeps nothing, and the link is dereferenced with GET (section 7.1).
    let q = if search { req.query_param("q") } else { None };
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
    let mut filter_bytes = Vec::new();
    let mut filter = Filter::default();
    if search {
        if let Some(q) = &q {
            match jose::b64url_decode(q) {
                Some(b) => filter_bytes = b,
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
                Some(_) => filter_bytes = req.body.to_vec(),
            }
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
                        "a filter may hold at most {MAX_FILTER_GROUPS} groups"
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
    let resources = readable(state, agent).await;
    let mut links = Vec::new();
    let mut content_location = None;
    let doc = if search {
        let mut items = Vec::new();
        for (uri, Seen { types, meta }) in &resources {
            let mut matched = all_groups(&filter.types, |v| types.iter().any(|t| t == v));
            if matched && !filter.relations.is_empty() {
                matched = filter.relations.iter().all(|(rel, groups)| {
                    let targets = relation_targets(uri, meta, rel);
                    all_groups(groups, |v| targets.contains(v))
                });
            }
            if matched {
                let shown: Vec<Value> = types
                    .iter()
                    .map(|t| Value::String(t.strip_prefix(LWS_NS).unwrap_or(t).to_string()))
                    .collect();
                items.push(json!({"id": uri, "type": shown}));
            }
        }
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
        let shown: Vec<Value> = items
            .into_iter()
            .skip((page - 1) * size)
            .take(size)
            .collect();
        json!({"@context": LWS_CONTEXT, "type": "ContainerPage", "totalItems": total, "items": shown})
    } else {
        // The TypeIndex is paged like a container (section 6.1): opaque `?page=N` links in Link
        // headers, first and last always, prev and next where there is one; a page that does not
        // exist (any more) is 404.
        let types: BTreeSet<&String> = resources.values().flat_map(|s| &s.types).collect();
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
    let mut resp = json_response(StatusCode::OK, media_type, &doc);
    let h = resp.headers_mut();
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
                let mut meta = state.resource_meta(&uri).await;
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

    #[test]
    fn empty_filters_match_everything() {
        assert_eq!(parse(""), Ok(Filter::default()));
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
