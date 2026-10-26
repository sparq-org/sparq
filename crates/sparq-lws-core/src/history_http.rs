//! Version and trash protocol adapters, sharing storage and representation rules.
use crate::{
    error::{ServerError, ServerResult},
    store::{
        history::{self, TrashEntry, VersionEntry},
        sparql, Store,
    },
};
use async_trait::async_trait;
use axum::{
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use serde_json::{json, Value};

#[async_trait]
pub(crate) trait Context: Sync {
    type Store: Store;
    fn store(&self) -> &Self::Store;
    fn base(&self) -> String;
    fn lws(&self) -> bool;
    async fn read(&self, original: &str) -> ServerResult<()>;
    async fn move_to_trash(&self, path: &str, headers: &HeaderMap) -> ServerResult<Response>;
    async fn trash(&self, entry: &TrashEntry) -> ServerResult<bool>;
    async fn restore(&self, member: &str, target: &str, parent: &str) -> ServerResult<()>;
}
fn prefix(lws: bool, versions: bool) -> &'static str {
    match (lws, versions) {
        (true, true) => "/.lws/versions/",
        (true, false) => "/.lws/trash/",
        (false, true) => "/.versions/",
        (false, false) => "/.trash/",
    }
}
pub(crate) fn listing_url(base: &str, lws: bool, original: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("resource", original)
        .finish();
    format!(
        "{}{p}?{query}",
        base.trim_end_matches('/'),
        p = prefix(lws, true)
    )
}
fn public_url(base: &str, lws: bool, member: &str, versions: bool) -> String {
    format!(
        "{}{}{}",
        base.trim_end_matches('/'),
        prefix(lws, versions),
        member
            .strip_prefix(if versions {
                history::VERSIONS
            } else {
                history::TRASH
            })
            .unwrap_or_default()
    )
}
pub(crate) fn recognizes(path: &str, lws: bool, cfg: &history::HistoryConfig) -> bool {
    (cfg.versions && path.starts_with(prefix(lws, true)))
        || (cfg.trash && path.starts_with(prefix(lws, false)))
}
fn date(t: Option<std::time::SystemTime>) -> Option<String> {
    t.and_then(crate::store::timestamp::to_xsd_datetime)
}
fn item(
    id: String,
    meta: &crate::store::ResourceMeta,
    original: &str,
    modified: Option<std::time::SystemTime>,
) -> Value {
    json!({"id": id, "type": "DataResource", "etag":meta.etag, "modified":date(modified), "mementoDatetime":date(meta.last_modified), "original":original})
}
fn listing(
    base: &str,
    url: &str,
    items: Vec<Value>,
    lws: bool,
    headers: &HeaderMap,
) -> ServerResult<Response> {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("*/*");
    let offered: &[&str] = if lws {
        &["application/lws+json"]
    } else {
        &["text/turtle", "application/ld+json"]
    };
    let selected = crate::lws::resources::negotiate(Some(accept), offered)
        .ok_or(ServerError::NotAcceptable)?;
    let (ct, body) = if lws {
        ("application/lws+json",json!({"@context":crate::lws::LWS_CONTEXT,"id":url,"type":"Container","totalItems":items.len(),"items":items}).to_string())
    } else if selected == "application/ld+json" {
        let mut nodes = vec![
            json!({"@id":url,"@type":["http://www.w3.org/ns/ldp#Container"],"http://www.w3.org/ns/ldp#contains":items.iter().map(|i| json!({"@id":i["id"]})).collect::<Vec<_>>()}),
        ];
        for i in &items {
            nodes.push(json!({"@id":i["id"], "https://jeswr.org/ns/solid-sd#etag":[{"@value":i["etag"]}],"http://purl.org/dc/terms/modified":[{"@value":i["modified"],"@type":sparql::XSD_DATETIME}],"http://mementoweb.org/ns#datetime":[{"@value":i["mementoDatetime"],"@type":sparql::XSD_DATETIME}],"http://www.iana.org/assignments/relation/original":[{"@id":i["original"]}]}));
        }
        ("application/ld+json", Value::Array(nodes).to_string())
    } else {
        let mut text = format!(
            "{} a <http://www.w3.org/ns/ldp#Container> .\n",
            sparql::iri(url).map_err(|e| ServerError::BadRequest(e.to_string()))?
        );
        for i in &items {
            let id = sparql::iri(i["id"].as_str().unwrap_or_default())
                .map_err(|e| ServerError::BadRequest(e.to_string()))?;
            text.push_str(&format!("<{}> <http://www.w3.org/ns/ldp#contains> {id} .\n{id} <https://jeswr.org/ns/solid-sd#etag> {} .\n",url,sparql::literal(i["etag"].as_str().unwrap_or_default())));
            let original = sparql::iri(i["original"].as_str().unwrap_or_default())
                .map_err(|e| ServerError::BadRequest(e.to_string()))?;
            text.push_str(&format!(
                "{id} <http://www.iana.org/assignments/relation/original> {original} .\n"
            ));
            for (field, pred) in [
                ("modified", "http://purl.org/dc/terms/modified"),
                ("mementoDatetime", "http://mementoweb.org/ns#datetime"),
            ] {
                if let Some(d) = i[field].as_str() {
                    text.push_str(&format!(
                        "{id} <{pred}> {}^^<{}> .\n",
                        sparql::literal(d),
                        sparql::XSD_DATETIME
                    ));
                }
            }
        }
        ("text/turtle", text)
    };
    let mut r = (StatusCode::OK, body).into_response();
    crate::lws::set(r.headers_mut(), header::CONTENT_TYPE, ct);
    crate::lws::set(r.headers_mut(), header::CACHE_CONTROL, "private, no-store");
    crate::lws::set(r.headers_mut(), header::VARY, "Accept");
    crate::lws::add_link(r.headers_mut(), base, "up", None);
    Ok(r)
}
async fn bytes<C: Context>(
    ctx: &C,
    member: &str,
    meta: &crate::store::ResourceMeta,
    original: &str,
    version: bool,
) -> ServerResult<Response> {
    let body = ctx.store().read_at(member, meta).await?;
    let mut r = (StatusCode::OK, body).into_response();
    crate::lws::set(r.headers_mut(), header::CONTENT_TYPE, &meta.content_type);
    crate::lws::set(r.headers_mut(), header::ETAG, &meta.etag);
    crate::lws::set(r.headers_mut(), header::CACHE_CONTROL, "private, no-store");
    crate::lws::add_link(r.headers_mut(), original, "original", None);
    if ctx.lws() {
        crate::lws::add_link(
            r.headers_mut(),
            &format!("{}.meta", public_url(&ctx.base(), true, member, version)),
            "linkset",
            Some("application/linkset+json"),
        );
    }
    if version {
        if let Some(d) = meta.last_modified.and_then(crate::http_date::format) {
            crate::lws::set(
                r.headers_mut(),
                header::HeaderName::from_static("memento-datetime"),
                &d,
            );
        }
    }
    Ok(r)
}
pub(crate) async fn serve<C: Context>(
    ctx: &C,
    path: &str,
    query: Option<&str>,
    method: &Method,
    headers: &HeaderMap,
    body: Bytes,
) -> ServerResult<Response> {
    let base = ctx.base();
    let lws = ctx.lws();
    let versions = path.starts_with(prefix(lws, true));
    let root = prefix(lws, versions);
    let tail = path.strip_prefix(root).ok_or(ServerError::NotFound)?;
    let linkset = lws && tail.ends_with(".meta");
    let tail = if linkset {
        tail.strip_suffix(".meta").unwrap_or(tail)
    } else {
        tail
    };
    let url = format!("{}{path}", base.trim_end_matches('/'));
    if versions {
        if method != Method::GET && method != Method::HEAD {
            return Err(ServerError::MethodNotAllowed);
        }
        if tail.is_empty() {
            let original = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
                .find(|(k, _)| k == "resource")
                .map(|(_, v)| v.into_owned())
                .ok_or_else(|| ServerError::BadRequest("resource is required".into()))?;
            if !original.starts_with(&format!("{}/", base.trim_end_matches('/')))
                || !history::data_resource(&original)
            {
                return Err(ServerError::NotFound);
            }
            ctx.read(&original).await?;
            let mut items: Vec<_> = ctx
                .store()
                .versions(Some(&original))
                .await?
                .iter()
                .map(|v| {
                    item(
                        public_url(&base, lws, &v.member, true),
                        &v.meta,
                        &original,
                        v.meta.last_modified,
                    )
                })
                .collect();
            if let Some(meta) = ctx.store().meta(&original).await? {
                items.push(item(original.clone(), &meta, &original, meta.last_modified));
            }
            return listing(
                &base,
                &listing_url(&base, lws, &original),
                items,
                lws,
                headers,
            );
        }
        let member = format!("{}{tail}", history::VERSIONS);
        let v: VersionEntry = ctx
            .store()
            .version_entry(&member)
            .await?
            .ok_or(ServerError::NotFound)?;
        ctx.read(&v.original).await?;
        if linkset {
            return retained_linkset(
                ctx,
                &v.member,
                &v.meta,
                &v.original,
                v.meta.last_modified,
                true,
            )
            .await;
        }
        return bytes(ctx, &member, &v.meta, &v.original, true).await;
    }
    if tail.is_empty() && (method == Method::GET || method == Method::HEAD) {
        let entries = ctx.store().trash_entries().await?;
        let mut items = Vec::new();
        for t in entries {
            if ctx.trash(&t).await? {
                items.push(item(
                    public_url(&base, lws, &t.member, false),
                    &t.meta,
                    &t.fields.original,
                    Some(t.fields.trashed_at),
                ));
            }
        }
        return listing(&base, &url, items, lws, headers);
    }
    if tail.is_empty() && method == Method::POST {
        let operation: Value = serde_json::from_slice(&body)
            .map_err(|_| ServerError::BadRequest("expected a resource or restore URL".into()))?;
        let operation = operation
            .as_object()
            .ok_or_else(|| ServerError::BadRequest("expected a resource or restore URL".into()))?;
        if operation.contains_key("resource") == operation.contains_key("restore") {
            return Err(ServerError::BadRequest(
                "provide exactly one of resource or restore".into(),
            ));
        }
        if let Some(resource) = operation.get("resource") {
            let resource = resource
                .as_str()
                .ok_or_else(|| ServerError::BadRequest("resource must be a URL".into()))?;
            let path = resource
                .strip_prefix(base.trim_end_matches('/'))
                .filter(|p| p.starts_with('/') && !p.starts_with("//"))
                .filter(|_| history::data_resource(resource))
                .ok_or(ServerError::NotFound)?;
            let uri: axum::http::Uri = path
                .parse()
                .map_err(|_| ServerError::BadRequest("invalid resource URL".into()))?;
            if uri.query().is_some() || path.contains('#') {
                return Err(ServerError::BadRequest(
                    "resource must name a data resource".into(),
                ));
            }
            return ctx.move_to_trash(path, headers).await;
        }
        let source = operation["restore"]
            .as_str()
            .ok_or_else(|| ServerError::BadRequest("restore must be a URL".into()))?;
        let source_prefix = format!("{}{}", base.trim_end_matches('/'), prefix(lws, false));
        let id = source
            .strip_prefix(&source_prefix)
            .ok_or(ServerError::NotFound)?;
        let member = format!("{}{id}", history::TRASH);
        let t = ctx
            .store()
            .trash_entry(&member)
            .await?
            .ok_or(ServerError::NotFound)?;
        if !ctx.trash(&t).await? {
            return Err(ServerError::Forbidden);
        }
        let parent = crate::lws::resources::parent_of(
            &t.fields.original,
            &format!("{}/", base.trim_end_matches('/')),
        )
        .ok_or_else(|| ServerError::Conflict("original parent is absent".into()))?;
        let target = if lws {
            history::mint(&parent).map_err(|e| ServerError::Storage(e.to_string()))?
        } else {
            t.fields.original.clone()
        };
        ctx.restore(&t.member, &target, &parent).await?;
        let mut r = StatusCode::CREATED.into_response();
        crate::lws::set(r.headers_mut(), header::LOCATION, &target);
        return Ok(r);
    }
    let member = format!("{}{tail}", history::TRASH);
    let t = ctx
        .store()
        .trash_entry(&member)
        .await?
        .ok_or(ServerError::NotFound)?;
    if !ctx.trash(&t).await? {
        return Err(ServerError::Forbidden);
    }
    if linkset {
        if method != Method::GET && method != Method::HEAD {
            return Err(ServerError::MethodNotAllowed);
        }
        return retained_linkset(
            ctx,
            &member,
            &t.meta,
            &t.fields.original,
            Some(t.fields.trashed_at),
            false,
        )
        .await;
    }
    if method == Method::DELETE {
        ctx.store().purge_trash(&member).await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if method != Method::GET && method != Method::HEAD {
        return Err(ServerError::MethodNotAllowed);
    }
    let mut r = bytes(ctx, &member, &t.meta, &t.fields.original, false).await?;
    if let Some(d) = crate::http_date::format(t.fields.trashed_at) {
        crate::lws::set(r.headers_mut(), header::LAST_MODIFIED, &d);
    }
    Ok(r)
}

pub(crate) fn catalog(
    catalog: &mut crate::describe::ServiceCatalog,
    base: &str,
    lws: bool,
    c: &history::HistoryConfig,
) {
    use crate::describe::{capability, entry, SSD};
    if c.versions {
        let endpoint = format!("{}{}", base.trim_end_matches('/'), prefix(lws, true));
        let mut service = entry(
            "versions",
            &format!("{SSD}VersioningService"),
            endpoint.clone(),
        );
        service.extras.insert(
            format!("{SSD}listingTemplate"),
            json!(format!("{endpoint}?resource={{resource}}")),
        );
        service.extras.insert(format!("{SSD}retention"),json!({"@value":format!("PT{}S",c.versions_retention.as_secs()),"@type":"http://www.w3.org/2001/XMLSchema#duration"}));
        catalog.service(service);
        catalog.capability(capability(&format!("{SSD}Versioning"), json!({})));
    }
    if c.trash {
        let mut service = entry(
            "trash",
            &format!("{SSD}TrashService"),
            format!("{}{}", base.trim_end_matches('/'), prefix(lws, false)),
        );
        service.extras.insert(format!("{SSD}retention"),json!({"@value":format!("PT{}S",c.trash_retention.as_secs()),"@type":"http://www.w3.org/2001/XMLSchema#duration"}));
        catalog.service(service);
        catalog.capability(capability(&format!("{SSD}Trash"), json!({})));
    }
}

/// Return the committed trash member only for a POST-to-trash operation.
pub(crate) fn moved(base: &str, lws: bool, member: &str, post: bool) -> Response {
    let mut response = if post {
        StatusCode::CREATED
    } else {
        StatusCode::NO_CONTENT
    }
    .into_response();
    if post {
        crate::lws::set(
            response.headers_mut(),
            header::LOCATION,
            &public_url(base, lws, member, false),
        );
    }
    response
}

struct Solid<'a, S: Store> {
    state: &'a std::sync::Arc<crate::ldp::handler::LdpState<S>>,
    token: &'a crate::auth::VerifiedToken,
    origin: Option<&'a str>,
}
#[async_trait]
impl<S: Store> Context for Solid<'_, S> {
    type Store = S;
    fn store(&self) -> &S {
        &self.state.store
    }
    fn base(&self) -> String {
        self.state.base_url().trim_end_matches('/').to_string()
    }
    fn lws(&self) -> bool {
        false
    }
    async fn read(&self, original: &str) -> ServerResult<()> {
        self.state
            .authorize_iri(
                original,
                crate::authz::AccessMode::Read,
                self.token,
                self.origin,
            )
            .await
    }
    async fn move_to_trash(&self, path: &str, headers: &HeaderMap) -> ServerResult<Response> {
        crate::ldp::handler::delete_resource(
            self.state.clone(),
            self.token.clone(),
            path.parse()
                .map_err(|_| ServerError::BadRequest("invalid resource URL".into()))?,
            headers.clone(),
            true,
        )
        .await
    }
    async fn trash(&self, entry: &TrashEntry) -> ServerResult<bool> {
        Ok(self
            .state
            .authorize_iri(
                &format!("{}/", self.base()),
                crate::authz::AccessMode::Control,
                self.token,
                self.origin,
            )
            .await
            .is_ok()
            || (entry.fields.deleter_could_read
                && self.token.web_id.is_some()
                && entry.fields.deleted_by == self.token.web_id))
    }
    async fn restore(&self, member: &str, target: &str, parent: &str) -> ServerResult<()> {
        self.state
            .authorize_iri(
                parent,
                crate::authz::AccessMode::Append,
                self.token,
                self.origin,
            )
            .await?;
        self.state
            .authorize_iri(
                target,
                crate::authz::AccessMode::Write,
                self.token,
                self.origin,
            )
            .await?;
        self.store().restore_trash(member, target, parent).await?;
        self.state
            .notifications
            .notify(
                target,
                crate::notifications::activity::ActivityType::Create,
                Some(parent),
            )
            .await;
        Ok(())
    }
}
pub(crate) async fn solid_handler<S: Store>(
    axum::extract::State(state): axum::extract::State<
        std::sync::Arc<crate::ldp::handler::LdpState<S>>,
    >,
    axum::Extension(token): axum::Extension<crate::auth::VerifiedToken>,
    req: axum::extract::Request,
) -> ServerResult<Response> {
    let (parts, body) = req.into_parts();
    let body = axum::body::to_bytes(body, crate::body_limit::DEFAULT_MAX_BODY_BYTES)
        .await
        .map_err(|_| ServerError::BadRequest("request body too large".into()))?;
    let ctx = Solid {
        state: &state,
        token: &token,
        origin: parts
            .headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok()),
    };
    let mut resp = serve(
        &ctx,
        parts.uri.path(),
        parts.uri.query(),
        &parts.method,
        &parts.headers,
        body,
    )
    .await?;
    if parts.method == Method::HEAD {
        *resp.body_mut() = axum::body::Body::empty();
    }
    Ok(resp)
}

struct Lws<'a, S: Store> {
    state: &'a crate::lws::LwsState<S>,
    agent: &'a crate::lws::Agent,
    req: &'a crate::lws::LwsRequest,
}
#[async_trait]
impl<S: Store + 'static> Context for Lws<'_, S> {
    type Store = crate::lws::tracked::Tracked<S>;
    fn store(&self) -> &Self::Store {
        &self.state.store
    }
    fn base(&self) -> String {
        self.state.cfg.base_url.clone()
    }
    fn lws(&self) -> bool {
        true
    }
    async fn read(&self, original: &str) -> ServerResult<()> {
        if crate::lws::access::allowed(
            self.state,
            crate::lws::access::Action::Read,
            original,
            self.agent,
        )
        .await?
        {
            Ok(())
        } else {
            Err(ServerError::Forbidden)
        }
    }
    async fn move_to_trash(&self, path: &str, headers: &HeaderMap) -> ServerResult<Response> {
        let mut request = self.req.clone();
        request.method = Method::DELETE;
        request.path = path.to_string();
        request.query = None;
        request.headers = headers.clone();
        request.body = Bytes::new();
        Ok(crate::lws::resources::trash(self.state, &request, self.agent).await)
    }
    async fn trash(&self, entry: &TrashEntry) -> ServerResult<bool> {
        Ok(self.state.cfg.open
            || crate::lws::access::is_owner(self.state, self.agent)
            || (entry.fields.deleter_could_read
                && self.agent.subject.is_some()
                && entry.fields.deleted_by == self.agent.subject))
    }
    async fn restore(&self, member: &str, target: &str, parent: &str) -> ServerResult<()> {
        crate::lws::resources::restore_history(
            self.state, self.req, self.agent, member, target, parent,
        )
        .await
    }
}
pub(crate) async fn lws_handler<S: Store + 'static>(
    state: &crate::lws::LwsState<S>,
    req: &crate::lws::LwsRequest,
    agent: &crate::lws::Agent,
) -> Response {
    let ctx = Lws { state, agent, req };
    match serve(
        &ctx,
        &req.path,
        req.query.as_deref(),
        &req.method,
        &req.headers,
        req.body.clone(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => crate::lws::problem(e.status(), None),
    }
}

const HISTORY_SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(60);
const HISTORY_SWEEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HISTORY_SWEEP_BATCH: usize = 64;

/// Add the version-history link only on enabled data resources.
pub(crate) async fn solid_links<S: Store>(
    axum::extract::State(state): axum::extract::State<
        std::sync::Arc<crate::ldp::handler::LdpState<S>>,
    >,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let original = format!(
        "{}{}",
        state.base_url().trim_end_matches('/'),
        req.uri().path()
    );
    let mut r = next.run(req).await;
    if r.status().is_success()
        && state.store.history_config().versions
        && history::data_resource(&original)
    {
        crate::lws::add_link(
            r.headers_mut(),
            &listing_url(state.base_url(), false, &original),
            "version-history",
            None,
        );
    }
    r
}
pub(crate) fn spawn_solid_sweep<S: Store + 'static>(
    state: &std::sync::Arc<crate::ldp::handler::LdpState<S>>,
) {
    if !state.store.history_config().enabled() {
        return;
    }
    let weak = std::sync::Arc::downgrade(state);
    tokio::spawn(async move {
        let every = HISTORY_SWEEP_EVERY;
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            let _ = tokio::time::timeout(
                HISTORY_SWEEP_TIMEOUT,
                state
                    .store
                    .sweep_history(crate::clock::now(), HISTORY_SWEEP_BATCH),
            )
            .await;
        }
    });
}
pub(crate) fn spawn_lws_sweep<S: Store + 'static>(state: &crate::lws::LwsState<S>) {
    if !state.store.history_config().enabled() {
        return;
    }
    let weak = state.downgrade();
    tokio::spawn(async move {
        let every = HISTORY_SWEEP_EVERY;
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            let _ = tokio::time::timeout(
                HISTORY_SWEEP_TIMEOUT,
                state
                    .store
                    .sweep_history(crate::clock::now(), HISTORY_SWEEP_BATCH),
            )
            .await;
        }
    });
}

async fn retained_linkset<C: Context>(
    ctx: &C,
    member: &str,
    meta: &crate::store::ResourceMeta,
    original: &str,
    modified: Option<std::time::SystemTime>,
    version: bool,
) -> ServerResult<Response> {
    let size = ctx.store().read_at(member, meta).await?.len().to_string();
    let url = public_url(&ctx.base(), true, member, version);
    Ok(crate::lws::json_response(
        StatusCode::OK,
        "application/linkset+json",
        &json!({"linkset":[{"anchor":url,"original":[{"href":original}],"self":[{"href":url,"format":[meta.content_type],"size":[size],"modified":[date(modified)]}]}]}),
    ))
}
