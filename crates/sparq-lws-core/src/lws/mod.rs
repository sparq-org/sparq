//! # Linked Web Storage (LWS 1.0)
//!
//! A W3C Linked Web Storage server over the same [`Store`] seam as the Solid surface, selected at
//! boot with `SOLID_SERVER_PROTOCOL=lws`. It tracks the LWS editor's drafts
//! (<https://github.com/w3c/lws-protocol>) and is held to the Touchstone conformance suite
//! (<https://github.com/ebremer/touchstone>), following the Community Solid Server LWS work
//! (jeswr/CommunitySolidServer `feat/lws`).
//!
//! What it serves:
//! - the **storage description** (`application/lws+cid`) at the storage URI, linked from every
//!   response with `rel="https://www.w3.org/ns/lws#storage"`;
//! - **containers** (`application/lws+json`, paginated) and **data resources**, with ETag and date
//!   validators, single byte ranges, JSON Merge Patch and JSON Patch, `Depth: infinity` deletes,
//!   and RFC 9457 problem details;
//! - a **linkset** (RFC 9264) per resource, patchable;
//! - an **authorization server** ([`authz_server`]): RFC 8414 metadata at
//!   `/.well-known/lws-configuration`, a JWKS, and RFC 8693 token exchange for did:key, controlled
//!   identifier and OpenID Connect subject tokens; the storage accepts the RFC 9068 access tokens
//!   it issues ([`tokens`]);
//! - **access grants and access requests** ([`access`]): the LWS Access Profile;
//! - **webhook notifications** ([`notify`]), signed per RFC 9421;
//! - the **type index and type search** services ([`index`]).
//!
//! Authorization: the storage owner (`SOLID_SERVER_LWS_OWNER`) may do anything, the agent that
//! created a resource may do anything with it, and anyone else what an access grant gives them.
//! `SOLID_SERVER_LWS_OPEN=1` is a development mode with no authentication at all.

pub mod access;
pub mod authz_server;
pub mod index;
pub mod jose;
pub mod notify;
pub mod resources;
pub mod tokens;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::Store;

pub const LWS_NS: &str = "https://www.w3.org/ns/lws#";
pub const LWS_CONTEXT: &str = "https://www.w3.org/ns/lws/v1";
pub const CID_CONTEXT: &str = "https://www.w3.org/ns/cid/v1";
pub const AS_CONTEXT: &str = "https://www.w3.org/ns/activitystreams";
pub const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";

pub const LWS_JSON: &str = "application/lws+json";
pub const LWS_CID: &str = "application/lws+cid";
pub const LD_JSON: &str = "application/ld+json";
pub const JSON: &str = "application/json";
pub const LINKSET_JSON: &str = "application/linkset+json";
pub const MERGE_PATCH: &str = "application/merge-patch+json";
pub const JSON_PATCH: &str = "application/json-patch+json";
pub const PROBLEM_JSON: &str = "application/problem+json";

/// Where the LWS services live, relative to the storage root. The `.lws/` and `.well-known/`
/// namespaces are never listed in the root container and cannot be created by clients (a POST
/// slug never starts with a dot).
pub const GRANTS_PATH: &str = "/.lws/grants/";
pub const REQUESTS_PATH: &str = "/.lws/requests/";
pub const SUBSCRIPTIONS_PATH: &str = "/.lws/subscriptions/";
pub const TYPE_INDEX_PATH: &str = "/.lws/types/index";
pub const TYPE_SEARCH_PATH: &str = "/.lws/types/search";
pub const AS_METADATA_PATH: &str = "/.well-known/lws-configuration";
pub const AS_METADATA_OAUTH_PATH: &str = "/.well-known/oauth-authorization-server";
pub const AS_JWKS_PATH: &str = "/.well-known/lws/jwks";
pub const AS_TOKEN_PATH: &str = "/.well-known/lws/token";

/// The suffix of a resource's linkset (and of its stored LWS metadata). Clients can never create
/// a resource whose name ends with it.
pub const META_SUFFIX: &str = ".meta";

/// Largest request body the LWS surface reads.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// Boot configuration (see [`LwsConfig::from_env`]).
#[derive(Debug, Clone)]
pub struct LwsConfig {
    /// The public origin, without a trailing slash: `https://storage.example`.
    pub base_url: String,
    /// The agent (WebID or DID) that owns the storage and may do anything in it.
    pub owner: Option<String>,
    /// Development mode: no authentication, every request may do everything.
    pub open: bool,
    /// Members per page of a container listing.
    pub page_size: usize,
    /// Signs the access tokens the authorization server issues.
    pub as_key: jose::EcKey,
    /// Signs webhook notification deliveries; published in the storage description.
    pub notify_key: jose::EcKey,
    /// Lifetime of issued access tokens, in seconds.
    pub token_ttl_secs: i64,
    /// Let the authorization server and the notification sender reach `http:` and loopback or
    /// private addresses (identity documents, OpenID providers, webhook inboxes). Development and
    /// conformance testing only.
    pub allow_insecure_fetch: bool,
    /// SAML identity providers trusted by the authorization server: entity id to PEM certificate
    /// or public key. Empty means the SAML suite is not offered.
    pub saml_idps: BTreeMap<String, String>,
}

impl LwsConfig {
    /// A configuration with fresh keys and no owner, for `base_url`.
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            owner: None,
            open: false,
            page_size: 100,
            as_key: jose::EcKey::generate("lws-as-1"),
            notify_key: jose::EcKey::generate("notify-key"),
            token_ttl_secs: 300,
            allow_insecure_fetch: false,
            saml_idps: BTreeMap::new(),
        }
    }

    /// Read the `SOLID_SERVER_LWS_*` environment:
    /// - `SOLID_SERVER_LWS_OWNER`: the storage owner's agent IRI;
    /// - `SOLID_SERVER_LWS_OPEN=1`: no authentication (development only);
    /// - `SOLID_SERVER_LWS_PAGE_SIZE`: members per container page (default 100);
    /// - `SOLID_SERVER_LWS_AS_KEY_FILE`: a private P-256 JWK that signs access tokens, created
    ///   with a fresh key when the file does not exist (default: a fresh key per boot, so tokens do
    ///   not survive a restart);
    /// - `SOLID_SERVER_LWS_NOTIFY_KEY_FILE`: the same for the key that signs notifications;
    /// - `SOLID_SERVER_LWS_TOKEN_TTL_SECS`: access token lifetime (default 300);
    /// - `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH=1`: allow `http:` and private-address fetches
    ///   (development and conformance only);
    /// - `SOLID_SERVER_LWS_SAML_IDPS_FILE`: a JSON object of trusted SAML IdP entity ids to PEM.
    pub fn from_env(base_url: &str) -> Result<Self, String> {
        let mut cfg = Self::new(base_url);
        let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let flag = |k: &str| var(k).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True"));
        cfg.owner = var("SOLID_SERVER_LWS_OWNER");
        cfg.open = flag("SOLID_SERVER_LWS_OPEN");
        cfg.allow_insecure_fetch = flag("SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH");
        if let Some(n) = var("SOLID_SERVER_LWS_PAGE_SIZE") {
            cfg.page_size = n.parse().ok().filter(|n| *n > 0).ok_or("SOLID_SERVER_LWS_PAGE_SIZE must be a positive integer")?;
        }
        if let Some(n) = var("SOLID_SERVER_LWS_TOKEN_TTL_SECS") {
            cfg.token_ttl_secs =
                n.parse().ok().filter(|n| *n > 0).ok_or("SOLID_SERVER_LWS_TOKEN_TTL_SECS must be a positive integer")?;
        }
        let read = |k: &str| -> Result<Option<String>, String> {
            match var(k) {
                Some(path) => std::fs::read_to_string(&path).map(Some).map_err(|e| format!("{k}: cannot read {path}: {e}")),
                None => Ok(None),
            }
        };
        // A key file that does not exist yet is created with a fresh key, so a deployment keeps its
        // keys across restarts by naming a path once.
        let key = |k: &str, kid: &str| -> Result<Option<jose::EcKey>, String> {
            let Some(path) = var(k) else { return Ok(None) };
            match std::fs::read_to_string(&path) {
                Ok(jwk) => jose::EcKey::from_jwk(&jwk).map(Some).map_err(|e| format!("{k}: {e}")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let fresh = jose::EcKey::generate(kid);
                    write_private(&path, &fresh.private_jwk().to_string()).map_err(|e| format!("{k}: cannot write {path}: {e}"))?;
                    Ok(Some(fresh))
                }
                Err(e) => Err(format!("{k}: cannot read {path}: {e}")),
            }
        };
        if let Some(k) = key("SOLID_SERVER_LWS_AS_KEY_FILE", "lws-as-1")? {
            cfg.as_key = k;
        }
        if let Some(k) = key("SOLID_SERVER_LWS_NOTIFY_KEY_FILE", "notify-key")? {
            cfg.notify_key = k;
        }
        if let Some(idps) = read("SOLID_SERVER_LWS_SAML_IDPS_FILE")? {
            cfg.saml_idps =
                serde_json::from_str(&idps).map_err(|e| format!("SOLID_SERVER_LWS_SAML_IDPS_FILE: {e}"))?;
        }
        Ok(cfg)
    }

    /// The storage URI, which is also the storage root container: `{base}/`.
    pub fn storage(&self) -> String {
        format!("{}/", self.base_url)
    }

    /// The authorization server's issuer identifier: the origin, no trailing slash.
    pub fn issuer(&self) -> &str {
        &self.base_url
    }

    /// The realm access tokens are issued for (their `aud`): the storage URI.
    pub fn realm(&self) -> String {
        self.storage()
    }

    /// The absolute URI of a server path.
    pub fn absolute(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

/// Write a private key file readable by its owner only.
fn write_private(path: &str, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(contents.as_bytes())
}

/// Who is asking: the access token's subject and client, or nobody.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Agent {
    pub subject: Option<String>,
    pub client: Option<String>,
}

impl Agent {
    pub fn anonymous() -> Self {
        Self::default()
    }

    pub fn is_authenticated(&self) -> bool {
        self.subject.is_some()
    }
}

/// The LWS metadata kept beside each resource, stored as JSON at `{resource}.meta`-style keys
/// (see [`meta_key`]): who created it, what the client declared about it, and its linkset.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceMeta {
    /// The agent that created the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// Types the client declared: `Link: <...>; rel="type"` outside the LWS namespace, or `<> a`
    /// in Turtle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// Descriptive links the client sent as Link headers: relation to targets.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub links: BTreeMap<String, Vec<String>>,
    /// The resource's linkset document, once patched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkset: Option<Value>,
    /// The linkset's entity tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkset_etag: Option<String>,
    /// For a container: when its membership last changed (epoch milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<u64>,
    /// For a container: changes whenever its membership or a member changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The store key of a resource's LWS metadata, which is also its linkset's URI (the draft's own
/// convention): `/a/b` maps to `/a/b.meta` and the container `/a/b/` to `/a/b/.meta`. Clients can
/// never create a resource whose name ends with `.meta` or starts with a dot, so the keys never
/// collide with a resource.
pub fn meta_key(iri: &str) -> String {
    format!("{iri}{META_SUFFIX}")
}

/// Shared server state; cheap to clone.
pub struct LwsState<S: Store> {
    inner: Arc<Inner<S>>,
}

impl<S: Store> Clone for LwsState<S> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

pub struct Inner<S: Store> {
    pub store: S,
    pub cfg: LwsConfig,
    pub access: access::AccessStore,
    pub notify: notify::Notifier,
    /// Fetches identity documents, OpenID provider metadata and JWKS.
    pub http: reqwest::Client,
}

impl<S: Store> std::ops::Deref for LwsState<S> {
    type Target = Inner<S>;
    fn deref(&self) -> &Inner<S> {
        &self.inner
    }
}

impl<S: Store + 'static> LwsState<S> {
    /// Build the state: ensure the storage root and the service containers exist, and load the
    /// stored access grants, requests and subscriptions.
    pub async fn new(store: S, cfg: LwsConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        let root = cfg.storage();
        if !store.exists(&root).await.map_err(|e| format!("store: {e}"))? {
            store.write(&root, Bytes::new(), LWS_JSON).await.map_err(|e| format!("store: {e}"))?;
        }
        let access = access::AccessStore::load(&store, &cfg).await?;
        let notify = notify::Notifier::load(&store, &cfg).await?;
        Ok(Self { inner: Arc::new(Inner { store, cfg, access, notify, http }) })
    }

    /// Whether `agent` may perform `action` on the resource at `uri` (see [`access::allowed`]).
    pub async fn allowed(&self, action: access::Action, uri: &str, agent: &Agent) -> bool {
        access::allowed(self, action, uri, agent).await
    }

    /// The LWS metadata stored beside `iri`, or the default when there is none.
    pub async fn resource_meta(&self, iri: &str) -> ResourceMeta {
        match self.store.read(&meta_key(iri)).await {
            Ok(r) => serde_json::from_slice(&r.body).unwrap_or_default(),
            Err(_) => ResourceMeta::default(),
        }
    }

    pub async fn put_resource_meta(&self, iri: &str, meta: &ResourceMeta) -> Result<(), crate::error::ServerError> {
        let body = serde_json::to_vec(meta).unwrap_or_default();
        self.store.write(&meta_key(iri), Bytes::from(body), JSON).await.map(|_| ())
    }

    /// A 401 with the Bearer challenge naming this authorization server and realm, and the storage
    /// link (section 9.2).
    pub fn challenge(&self, error: Option<&str>) -> Response {
        let mut c = format!("Bearer as_uri=\"{}\", realm=\"{}\"", self.cfg.issuer(), self.cfg.realm());
        if let Some(e) = error {
            c.push_str(&format!(", error=\"{e}\""));
        }
        let mut resp = problem(StatusCode::UNAUTHORIZED, None);
        set(resp.headers_mut(), header::WWW_AUTHENTICATE, &c);
        add_link(resp.headers_mut(), &self.cfg.storage(), &format!("{LWS_NS}storage"), None);
        resp
    }

    /// 401 for nobody, 403 for an authenticated agent.
    pub fn deny(&self, agent: &Agent) -> Response {
        if agent.is_authenticated() {
            problem(StatusCode::FORBIDDEN, None)
        } else {
            self.challenge(None)
        }
    }

    /// Whether the request carries no valid identity and the server is not open.
    pub fn needs_auth(&self, agent: &Agent) -> bool {
        !self.cfg.open && !agent.is_authenticated()
    }
}

/// One request, read in full.
pub struct LwsRequest {
    pub method: Method,
    /// The raw path, percent-encoded, starting with `/`.
    pub path: String,
    pub query: Option<String>,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl LwsRequest {
    pub fn header(&self, name: impl header::AsHeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Every value of a header, comma-joined.
    pub fn header_all(&self, name: impl header::AsHeaderName) -> String {
        self.headers.get_all(name).iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(", ")
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        let q = self.query.as_deref()?;
        url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
    }

    pub fn is_head(&self) -> bool {
        self.method == Method::HEAD
    }

    /// The media type essence of Content-Type, lower-cased.
    pub fn content_type(&self) -> Option<String> {
        self.header(header::CONTENT_TYPE)
            .map(|v| v.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty())
    }
}

/// Build the LWS router over `store`.
pub async fn router<S: Store + 'static>(store: S, cfg: LwsConfig) -> Result<Router, String> {
    let state = LwsState::new(store, cfg).await?;
    Ok(Router::new().fallback(dispatch::<S>).with_state(state))
}

async fn dispatch<S: Store + 'static>(State(state): State<LwsState<S>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let body = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(b) => b,
        Err(_) => return problem(StatusCode::PAYLOAD_TOO_LARGE, None),
    };
    let req = LwsRequest {
        method: parts.method,
        path: parts.uri.path().to_string(),
        query: parts.uri.query().map(str::to_string),
        headers: parts.headers,
        body,
    };
    let is_head = req.is_head();
    let mut resp = route(&state, req).await;
    if is_head {
        // HEAD carries the headers a GET would, never a body.
        let len = resp.headers().get(header::CONTENT_LENGTH).cloned();
        *resp.body_mut() = Body::empty();
        if let Some(len) = len {
            resp.headers_mut().insert(header::CONTENT_LENGTH, len);
        }
    }
    resp
}

async fn route<S: Store + 'static>(state: &LwsState<S>, req: LwsRequest) -> Response {
    let path = req.path.as_str();
    if matches!(path, AS_METADATA_PATH | AS_METADATA_OAUTH_PATH | AS_JWKS_PATH | AS_TOKEN_PATH) {
        return authz_server::handle(state, &req).await;
    }
    if path.starts_with("/.well-known/") {
        return problem(StatusCode::NOT_FOUND, None);
    }
    let agent = match tokens::authenticate(state, &req) {
        Ok(agent) => agent,
        Err(error) => return state.challenge(Some(error)),
    };
    if path.starts_with(SUBSCRIPTIONS_PATH) {
        return notify::handle(state, &req, &agent).await;
    }
    if path == TYPE_INDEX_PATH || path == TYPE_SEARCH_PATH {
        return index::handle(state, &req, &agent).await;
    }
    if path.starts_with(GRANTS_PATH) || path.starts_with(REQUESTS_PATH) {
        return access::handle(state, &req, &agent).await;
    }
    if path.starts_with("/.lws/") || path == "/.lws" {
        return problem(StatusCode::NOT_FOUND, None);
    }
    resources::handle(state, &req, &agent).await
}

// ---- response helpers ----

/// A response with status `code`; for an error, an RFC 9457 problem details body.
pub fn problem(code: StatusCode, detail: Option<&str>) -> Response {
    if code.as_u16() < 400 {
        return code.into_response();
    }
    let mut body = json!({
        "type": "about:blank",
        "title": code.canonical_reason().unwrap_or("Error"),
        "status": code.as_u16(),
    });
    if let Some(d) = detail {
        body["detail"] = Value::String(d.to_string());
    }
    let mut resp = (code, body.to_string()).into_response();
    set(resp.headers_mut(), header::CONTENT_TYPE, PROBLEM_JSON);
    resp
}

/// 405 with the methods that are allowed.
pub fn method_not_allowed(allow: &str) -> Response {
    let mut resp = problem(StatusCode::METHOD_NOT_ALLOWED, None);
    set(resp.headers_mut(), header::ALLOW, allow);
    resp
}

/// A JSON body with `content_type`.
pub fn json_response(code: StatusCode, content_type: &str, body: &Value) -> Response {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    let mut resp = (code, bytes).into_response();
    set(resp.headers_mut(), header::CONTENT_TYPE, content_type);
    resp
}

pub fn set(headers: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(name, v);
    }
}

/// Append `Link: <target>; rel="rel"[; type="..."]`.
pub fn add_link(headers: &mut HeaderMap, target: &str, rel: &str, media_type: Option<&str>) {
    let mut v = format!("<{target}>; rel=\"{rel}\"");
    if let Some(t) = media_type {
        v.push_str(&format!("; type=\"{t}\""));
    }
    if let Ok(v) = HeaderValue::from_str(&v) {
        headers.append(header::LINK, v);
    }
}

/// Whether `v` is an absolute URI: a scheme, a colon, and something after it.
pub fn is_uri(v: &str) -> bool {
    let Some((scheme, rest)) = v.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !rest.is_empty()
        && !v.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`'))
}

/// A JSON value that is a string holding an absolute URI.
pub fn json_is_uri(v: &Value) -> bool {
    v.as_str().is_some_and(is_uri)
}

/// Whether a JSON `type` value (a string or an array) names `wanted`, short or in the LWS namespace.
pub fn has_type(v: &Value, wanted: &str) -> bool {
    let matches = |t: &Value| t.as_str().is_some_and(|t| t == wanted || t.strip_prefix(LWS_NS) == Some(wanted));
    match v {
        Value::Array(a) => a.iter().any(matches),
        other => matches(other),
    }
}

/// Parse a Link header value into `(target, params)` pairs, params lower-cased by name.
pub fn parse_links(value: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let target = rest[start + 1..start + end].trim().to_string();
        rest = &rest[start + end + 1..];
        let mut params = BTreeMap::new();
        // Parameters run until the next comma outside quotes.
        let mut i = 0;
        let bytes = rest.as_bytes();
        let mut in_quotes = false;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => in_quotes = !in_quotes,
                b',' if !in_quotes => break,
                _ => {}
            }
            i += 1;
        }
        for p in rest[..i].split(';') {
            if let Some((k, v)) = p.split_once('=') {
                params.insert(k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').to_string());
            }
        }
        rest = &rest[i.min(rest.len())..];
        out.push((target, params));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_parse_with_quoted_commas() {
        let l = parse_links(r#"<https://a/x>; rel="type", <b>; rel="up next"; title="a, b", <c>"#);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].0, "https://a/x");
        assert_eq!(l[0].1["rel"], "type");
        assert_eq!(l[1].1["rel"], "up next");
        assert_eq!(l[1].1["title"], "a, b");
        assert_eq!(l[2].0, "c");
    }

    #[test]
    fn uri_check() {
        assert!(is_uri("https://example.org/x"));
        assert!(is_uri("did:key:z6Mk"));
        assert!(is_uri("urn:uuid:1"));
        assert!(!is_uri("#frag"));
        assert!(!is_uri("relative/path"));
        assert!(!is_uri("http://a b"));
    }

    #[test]
    fn meta_keys() {
        assert_eq!(meta_key("http://h/a/b"), "http://h/a/b.meta");
        assert_eq!(meta_key("http://h/a/b/"), "http://h/a/b/.meta");
        assert_eq!(meta_key("http://h/"), "http://h/.meta");
    }
}
