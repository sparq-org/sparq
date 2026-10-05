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
pub mod saml;
pub mod subject_tokens;
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
    /// The authorization server's previous signing key, after a rotation: published in the JWKS,
    /// and tokens it signed (named by its `kid`) still validate until they expire.
    pub as_previous_key: Option<jose::VerifyKey>,
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
    /// Largest request body read, in bytes; a larger one is refused with 413 before it is
    /// buffered further. The server-wide ceiling (`SOLID_SERVER_MAX_BODY_BYTES`, see
    /// [`crate::body_limit`]), the same one the Solid surface enforces.
    pub max_body: usize,
}

impl LwsConfig {
    /// A configuration with fresh keys and no owner, for `base_url`.
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            owner: None,
            open: false,
            page_size: 100,
            as_key: jose::EcKey::generate_thumbprinted(),
            as_previous_key: None,
            notify_key: jose::EcKey::generate("notify-key"),
            token_ttl_secs: 300,
            allow_insecure_fetch: false,
            saml_idps: BTreeMap::new(),
            max_body: crate::body_limit::DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// Read the `SOLID_SERVER_LWS_*` environment:
    /// - `SOLID_SERVER_LWS_OWNER`: the storage owner's agent IRI;
    /// - `SOLID_SERVER_LWS_OPEN=1` (or `SOLID_SERVER_OPEN_MODE=1`): no authentication (development
    ///   only);
    /// - `SOLID_SERVER_LWS_PAGE_SIZE`: members per container page (default 100);
    /// - `SOLID_SERVER_LWS_AS_KEY_FILE`: a private P-256 JWK that signs access tokens, created
    ///   with a fresh key when the file does not exist (default: a fresh key per boot, so tokens do
    ///   not survive a restart);
    /// - `SOLID_SERVER_LWS_AS_PREVIOUS_KEY_FILE`: the signing key the AS key replaced (a public
    ///   or private P-256 JWK, with a `kid` that differs from the current key's), kept for
    ///   validation and the JWKS during a rotation;
    /// - `SOLID_SERVER_LWS_NOTIFY_KEY_FILE`: the same for the key that signs notifications;
    /// - `SOLID_SERVER_LWS_TOKEN_TTL_SECS`: access token lifetime (default 300);
    /// - `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH=1`: allow `http:` and private-address fetches
    ///   (development and conformance only);
    /// - `SOLID_SERVER_LWS_SAML_IDPS_FILE`: a JSON object of trusted SAML IdP entity ids to PEM;
    /// - `SOLID_SERVER_MAX_BODY_BYTES`: the request body ceiling shared with the Solid surface.
    pub fn from_env(base_url: &str) -> Result<Self, String> {
        let mut cfg = Self::new(base_url);
        cfg.max_body = crate::body_limit::max_body_bytes_from_env();
        let var = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let flag =
            |k: &str| var(k).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True"));
        cfg.owner = var("SOLID_SERVER_LWS_OWNER");
        // `SOLID_SERVER_OPEN_MODE` is the name the lws-contrib dagger-workspace sparq cell sets.
        cfg.open = flag("SOLID_SERVER_LWS_OPEN") || flag("SOLID_SERVER_OPEN_MODE");
        // Open mode is for local test harnesses, whose inboxes and documents are on private hosts.
        cfg.allow_insecure_fetch = cfg.open || flag("SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH");
        if let Some(n) = var("SOLID_SERVER_LWS_PAGE_SIZE") {
            cfg.page_size = n
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("SOLID_SERVER_LWS_PAGE_SIZE must be a positive integer")?;
        }
        if let Some(n) = var("SOLID_SERVER_LWS_TOKEN_TTL_SECS") {
            cfg.token_ttl_secs = n
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("SOLID_SERVER_LWS_TOKEN_TTL_SECS must be a positive integer")?;
        }
        let read = |k: &str| -> Result<Option<String>, String> {
            match var(k) {
                Some(path) => std::fs::read_to_string(&path)
                    .map(Some)
                    .map_err(|e| format!("{k}: cannot read {path}: {e}")),
                None => Ok(None),
            }
        };
        // A key file that does not exist yet is created with a fresh key, so a deployment keeps its
        // keys across restarts by naming a path once.
        let key = |k: &str, kid: Option<&str>| -> Result<Option<jose::EcKey>, String> {
            match var(k) {
                Some(path) => key_file(&path, kid)
                    .map(Some)
                    .map_err(|e| format!("{k}: {e}")),
                None => Ok(None),
            }
        };
        // A generated access-token key's kid is its thumbprint, so the fresh key of a rotation
        // never collides with the one it replaced.
        if let Some(k) = key("SOLID_SERVER_LWS_AS_KEY_FILE", None)? {
            cfg.as_key = k;
        }
        if let Some(jwk) = read("SOLID_SERVER_LWS_AS_PREVIOUS_KEY_FILE")? {
            let (current, previous) = rotated_as_keys(cfg.as_key.clone(), &jwk)
                .map_err(|e| format!("SOLID_SERVER_LWS_AS_PREVIOUS_KEY_FILE: {e}"))?;
            cfg.as_key = current;
            cfg.as_previous_key = Some(previous);
        }
        if let Some(k) = key("SOLID_SERVER_LWS_NOTIFY_KEY_FILE", Some("notify-key"))? {
            cfg.notify_key = k;
        }
        if let Some(idps) = read("SOLID_SERVER_LWS_SAML_IDPS_FILE")? {
            cfg.saml_idps = serde_json::from_str(&idps)
                .map_err(|e| format!("SOLID_SERVER_LWS_SAML_IDPS_FILE: {e}"))?;
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

    /// The keys access tokens verify against: the current signing key, then the previous one.
    pub fn as_verify_keys(&self) -> Vec<jose::VerifyKey> {
        std::iter::once(jose::VerifyKey::from(&self.as_key))
            .chain(self.as_previous_key.clone())
            .collect()
    }

    /// The absolute URI of a server path.
    pub fn absolute(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

/// The client the authorization server fetches identity documents, OpenID provider metadata and
/// JWKS with (see [`subject_tokens::fetch`]): no automatic redirects (each hop is checked before it
/// is requested), no proxy, and, unless `allow_insecure_fetch` is set, `https:` only through a
/// resolver that hands out public addresses alone, so the connection goes to an address that
/// passed the check (no DNS rebinding between check and connect).
pub fn fetch_client(cfg: &LwsConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !cfg.allow_insecure_fetch {
        builder = builder
            .dns_resolver(Arc::new(PublicOnlyResolver))
            .https_only(true);
    }
    builder.build().map_err(|e| format!("http client: {e}"))
}

/// Resolves names to their public addresses only (see [`subject_tokens::is_forbidden_ip`]), so an
/// outbound request (a fetch or a notification delivery) cannot be pointed at the server's own
/// network by a name that resolves there.
pub struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = public_addrs(tokio::net::lookup_host((host.as_str(), 0)).await?);
            if addrs.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The public addresses among `addrs`.
pub fn public_addrs(
    addrs: impl IntoIterator<Item = std::net::SocketAddr>,
) -> Vec<std::net::SocketAddr> {
    addrs
        .into_iter()
        .filter(|a| !subject_tokens::is_forbidden_ip(a.ip()))
        .collect()
}

/// The private key in the JWK file at `path`; when there is no file yet, a fresh key written there,
/// its kid `kid` or, without one, its thumbprint.
fn key_file(path: &str, kid: Option<&str>) -> Result<jose::EcKey, String> {
    match std::fs::read_to_string(path) {
        Ok(jwk) => jose::EcKey::from_jwk(&jwk),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let fresh = match kid {
                Some(kid) => jose::EcKey::generate(kid),
                None => jose::EcKey::generate_thumbprinted(),
            };
            write_private(path, &fresh.private_jwk().to_string())
                .map_err(|e| format!("cannot write {path}: {e}"))?;
            Ok(fresh)
        }
        Err(e) => Err(format!("cannot read {path}: {e}")),
    }
}

/// The access-token signing key and the key it replaced (the JWK `previous_jwk`), with distinct
/// kids. Both were generated with the one fixed kid `lws-as-1` before generated keys were named by
/// their thumbprints, so a rotation of such keys can meet two keys with one kid: the current key
/// then goes by its thumbprint, and the previous key keeps the kid the tokens it signed name, so
/// they verify until they expire. The same key named as both is refused, as is a previous key whose
/// kid is the current key's thumbprint (no kid then tells the two apart).
fn rotated_as_keys(
    current: jose::EcKey,
    previous_jwk: &str,
) -> Result<(jose::EcKey, jose::VerifyKey), String> {
    let previous = jose::VerifyKey::from_jwk(previous_jwk)?;
    if previous.public_key() == current.public_key() {
        return Err("it is the current key; rotate to a new key".into());
    }
    let current = if previous.kid() == current.kid() {
        let thumbprint = current.thumbprint();
        current.with_kid(thumbprint)
    } else {
        current
    };
    if previous.kid() == current.kid() {
        return Err("its kid is the current key's; give the rotated keys distinct kids".into());
    }
    Ok((current, previous))
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
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub struct Inner<S: Store> {
    pub store: S,
    pub cfg: LwsConfig,
    pub access: access::AccessStore,
    pub notify: notify::Notifier,
    /// Fetches identity documents, OpenID provider metadata and JWKS.
    pub http: reqwest::Client,
    /// Serializes conditional writes per resource (see [`resources::IriLocks`]).
    pub locks: resources::IriLocks,
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
        let http = fetch_client(&cfg)?;
        let root = cfg.storage();
        if !store
            .exists(&root)
            .await
            .map_err(|e| format!("store: {e}"))?
        {
            store
                .write(&root, Bytes::new(), LWS_JSON)
                .await
                .map_err(|e| format!("store: {e}"))?;
        }
        let access = access::AccessStore::load(&store, &cfg).await?;
        let notify = notify::Notifier::load(&store, &cfg).await?;
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                cfg,
                access,
                notify,
                http,
                locks: Default::default(),
            }),
        })
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

    pub async fn put_resource_meta(
        &self,
        iri: &str,
        meta: &ResourceMeta,
    ) -> Result<(), crate::error::ServerError> {
        let body = serde_json::to_vec(meta).unwrap_or_default();
        self.store
            .write(&meta_key(iri), Bytes::from(body), JSON)
            .await
            .map(|_| ())
    }

    /// A 401 with the Bearer challenge naming this authorization server and realm, and the storage
    /// link (section 9.2).
    pub fn challenge(&self, error: Option<&str>) -> Response {
        let mut c = format!(
            "Bearer as_uri=\"{}\", realm=\"{}\"",
            self.cfg.issuer(),
            self.cfg.realm()
        );
        if let Some(e) = error {
            c.push_str(&format!(", error=\"{e}\""));
        }
        let mut resp = problem(StatusCode::UNAUTHORIZED, None);
        set(resp.headers_mut(), header::WWW_AUTHENTICATE, &c);
        add_link(
            resp.headers_mut(),
            &self.cfg.storage(),
            &format!("{LWS_NS}storage"),
            None,
        );
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
        self.headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        let q = self.query.as_deref()?;
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    }

    pub fn is_head(&self) -> bool {
        self.method == Method::HEAD
    }

    /// The media type essence of Content-Type, lower-cased.
    pub fn content_type(&self) -> Option<String> {
        self.header(header::CONTENT_TYPE)
            .map(|v| {
                v.split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            })
            .filter(|v| !v.is_empty())
    }
}

/// Build the LWS router over `store`.
pub async fn router<S: Store + 'static>(store: S, cfg: LwsConfig) -> Result<Router, String> {
    let state = LwsState::new(store, cfg).await?;
    Ok(Router::new()
        .fallback(dispatch::<S>)
        .with_state(state)
        .layer(axum::middleware::from_fn(crate::ldp::cors::cors_middleware)))
}

async fn dispatch<S: Store + 'static>(State(state): State<LwsState<S>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let body = match axum::body::to_bytes(body, state.cfg.max_body).await {
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
    // Unauthenticated liveness and readiness probes, as the Solid surface serves them.
    if matches!(path, "/livez" | "/readyz") && matches!(req.method, Method::GET | Method::HEAD) {
        return (StatusCode::OK, "ok").into_response();
    }
    if matches!(
        path,
        AS_METADATA_PATH | AS_METADATA_OAUTH_PATH | AS_JWKS_PATH | AS_TOKEN_PATH
    ) {
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

// ---- service containers ----
//
// The access grant, access request and subscription services are LWS containers (access requests
// section 11.5, webhook "Subscription Management"), held in memory rather than in the store. These
// helpers give their listings what [`resources`] gives a stored container: content negotiation,
// paging, an entity tag and the container links.

/// One media range of an Accept header: essence, q and profile.
fn accept_ranges(accept: &str) -> Vec<(String, f32, Option<String>)> {
    accept
        .split(',')
        .filter_map(|part| {
            let mut pieces = part.split(';');
            let essence = pieces.next()?.trim().to_ascii_lowercase();
            if essence.is_empty() {
                return None;
            }
            let (mut q, mut profile) = (1.0, None);
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
            Some((essence, q, profile))
        })
        .collect()
}

/// The container media type for `accept`: `application/lws+json`, `application/ld+json` (lws+json
/// when it names the LWS profile) or `application/json`, the earlier winning ties; lws+json when
/// there is no Accept; `None` when none of them is acceptable.
pub fn negotiate_container(accept: Option<&str>) -> Option<&'static str> {
    const OFFERED: [&str; 3] = [LWS_JSON, LD_JSON, JSON];
    let Some(accept) = accept.filter(|a| !a.trim().is_empty()) else {
        return Some(LWS_JSON);
    };
    let ranges = accept_ranges(accept);
    if ranges
        .iter()
        .any(|(e, q, p)| e == LD_JSON && p.as_deref() == Some(LWS_CONTEXT) && *q > 0.0)
    {
        return Some(LWS_JSON);
    }
    let mut best: Option<(f32, usize)> = None;
    for (i, offer) in OFFERED.iter().enumerate() {
        // The most specific range that matches the offer decides its q.
        let mut q_for: Option<(f32, u8)> = None;
        for (essence, q, _) in &ranges {
            let spec = if essence == offer {
                3
            } else if essence == "application/*" {
                2
            } else if essence == "*/*" {
                1
            } else {
                0
            };
            if spec > 0 && q_for.is_none_or(|(_, s)| spec > s) {
                q_for = Some((*q, spec));
            }
        }
        if let Some((q, _)) = q_for.filter(|(q, _)| *q > 0.0) {
            if best.is_none_or(|(bq, _)| q > bq) {
                best = Some((q, i));
            }
        }
    }
    best.map(|(_, i)| OFFERED[i])
}

/// The page a request asks for (`?page=`, 1 when absent), or `None` when it is not one of `pages`.
pub fn requested_page(req: &LwsRequest, pages: usize) -> Option<usize> {
    let page = match req.query_param("page") {
        None => 1,
        Some(p) => p.parse::<usize>().ok()?,
    };
    (1..=pages).contains(&page).then_some(page)
}

/// Append the paging links of page `page` of `pages` at `base?page=N`: first (always), prev and
/// next where there is one, and last.
pub fn add_page_links(headers: &mut HeaderMap, base: &str, page: usize, pages: usize) {
    let page_uri = |n: usize| format!("{base}?page={n}");
    add_link(headers, &page_uri(1), "first", None);
    if page > 1 {
        add_link(headers, &page_uri(page - 1), "prev", None);
    }
    if page < pages {
        add_link(headers, &page_uri(page + 1), "next", None);
    }
    add_link(headers, &page_uri(pages), "last", None);
}

/// The links a service container or one of its members carries: `up` (the service container for
/// a member; the storage root for the container itself, since `/.lws/` is no resource), the
/// storage, its LWS type and its linkset.
pub fn service_links(cfg: &LwsConfig, headers: &mut HeaderMap, uri: &str, up: &str) {
    add_link(headers, up, "up", None);
    add_link(headers, &cfg.storage(), &format!("{LWS_NS}storage"), None);
    let ty = if uri.ends_with('/') {
        "Container"
    } else {
        "DataResource"
    };
    add_link(headers, &format!("{LWS_NS}{ty}"), "type", None);
    add_link(headers, &meta_key(uri), "linkset", Some(LINKSET_JSON));
}

/// Whether `If-None-Match` names `etag` (weak comparison) or is `*`.
pub fn none_match(req: &LwsRequest, etag: &str) -> bool {
    let header = req.header_all(header::IF_NONE_MATCH);
    let bare = |t: &str| t.trim().trim_start_matches("W/").to_string();
    header
        .split(',')
        .any(|t| t.trim() == "*" || (!t.trim().is_empty() && bare(t) == bare(etag)))
}

/// A strong entity tag over `parts`.
pub fn etag_of<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    format!("\"{}\"", jose::b64url(&h.finalize()[..18]))
}

/// GET or HEAD on a service container at `uri` holding `items` (each with an `id`, sorted):
/// negotiated (406 when nothing offered is acceptable), paged at `cfg.page_size` (404 for a page
/// that does not exist), with an entity tag over `version`, the page and its members (304 when
/// `If-None-Match` names it), `Vary: Accept` and the container links.
pub fn service_listing(
    cfg: &LwsConfig,
    req: &LwsRequest,
    uri: &str,
    items: Vec<Value>,
    version: &str,
) -> Response {
    let Some(media_type) = negotiate_container(req.header(header::ACCEPT)) else {
        return problem(StatusCode::NOT_ACCEPTABLE, None);
    };
    let size = cfg.page_size.max(1);
    let pages = items.len().div_ceil(size).max(1);
    let Some(page) = requested_page(req, pages) else {
        return problem(StatusCode::NOT_FOUND, None);
    };
    let total = items.len();
    let shown: Vec<Value> = items
        .into_iter()
        .skip((page - 1) * size)
        .take(size)
        .collect();
    let page_tag = page.to_string();
    let etag = etag_of(
        [version, page_tag.as_str()]
            .into_iter()
            .chain(shown.iter().filter_map(|i| i["id"].as_str())),
    );
    let mut resp = if none_match(req, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let body = json!({
            "@context": LWS_CONTEXT,
            "id": uri,
            "type": "Container",
            "totalItems": total,
            "items": shown,
        });
        json_response(StatusCode::OK, media_type, &body)
    };
    let h = resp.headers_mut();
    set(h, header::ETAG, &etag);
    set(h, header::VARY, "Accept");
    service_links(cfg, h, uri, &cfg.storage());
    if pages > 1 {
        add_page_links(h, uri, page, pages);
    }
    resp
}

/// The read-only linkset of a service container or member at `anchor`: it has no user-managed
/// links.
pub fn service_linkset(cfg: &LwsConfig, req: &LwsRequest, anchor: &str) -> Response {
    if !matches!(req.method, Method::GET | Method::HEAD) {
        return method_not_allowed("GET, HEAD");
    }
    let doc = json!({"linkset": [{"anchor": anchor}]});
    let mut resp = json_response(StatusCode::OK, LINKSET_JSON, &doc);
    set(resp.headers_mut(), header::ETAG, &etag_of([anchor]));
    add_link(
        resp.headers_mut(),
        &cfg.storage(),
        &format!("{LWS_NS}storage"),
        None,
    );
    resp
}

/// Whether a JSON-LD `@context` (a string or an ordered set) includes the LWS context.
pub fn has_lws_context(v: Option<&Value>) -> bool {
    match v {
        Some(Value::String(s)) => s == LWS_CONTEXT,
        Some(Value::Array(a)) => a.iter().any(|c| c.as_str() == Some(LWS_CONTEXT)),
        _ => false,
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
        && !v.chars().any(|c| {
            c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`')
        })
}

/// A JSON value that is a string holding an absolute URI.
pub fn json_is_uri(v: &Value) -> bool {
    v.as_str().is_some_and(is_uri)
}

/// Whether a JSON `type` value (a string or an array) names `wanted`, short or in the LWS namespace.
pub fn has_type(v: &Value, wanted: &str) -> bool {
    let matches = |t: &Value| {
        t.as_str()
            .is_some_and(|t| t == wanted || t.strip_prefix(LWS_NS) == Some(wanted))
    };
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
        let Some(end) = rest[start..].find('>') else {
            break;
        };
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
                params.insert(
                    k.trim().to_ascii_lowercase(),
                    v.trim().trim_matches('"').to_string(),
                );
            }
        }
        rest = &rest[i.min(rest.len())..];
        out.push((target, params));
    }
    out
}

/// A store for unit tests: the in-memory store, with deletes that fail on demand.
#[cfg(test)]
pub(crate) mod test_store {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use bytes::Bytes;

    use crate::error::{ServerError, ServerResult};
    use crate::store::sparq::{DeleteOutcome, ResourceMeta};
    use crate::store::{
        CompositeStore, InMemoryBlobStore, InMemorySparqClient, Resource, Store, ValidatedChildIri,
    };

    #[derive(Clone)]
    pub struct FlakyStore {
        inner: Arc<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>,
        pub fail_delete: Arc<AtomicBool>,
        /// `delete` of this IRI alone fails with a backend error.
        pub fail_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `exists` fails with a backend error.
        pub fail_exists: Arc<AtomicBool>,
        /// `exists` reports this IRI absent, as if it was checked just before the IRI appeared.
        pub hide: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl FlakyStore {
        pub fn new() -> Self {
            Self {
                inner: Arc::new(CompositeStore::new(
                    InMemorySparqClient::default(),
                    InMemoryBlobStore::default(),
                )),
                fail_delete: Arc::new(AtomicBool::new(false)),
                fail_exists: Arc::new(AtomicBool::new(false)),
                fail_delete_of: Default::default(),
                hide: Default::default(),
            }
        }
    }

    #[async_trait]
    impl Store for FlakyStore {
        async fn read(&self, iri: &str) -> ServerResult<Resource> {
            self.inner.read(iri).await
        }
        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            self.inner.meta(iri).await
        }
        async fn exists(&self, iri: &str) -> ServerResult<bool> {
            if self.fail_exists.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if self.hide.lock().unwrap().as_deref() == Some(iri) {
                return Ok(false);
            }
            self.inner.exists(iri).await
        }
        async fn write(&self, iri: &str, body: Bytes, ct: &str) -> ServerResult<ResourceMeta> {
            self.inner.write(iri, body, ct).await
        }
        async fn create_in_container(
            &self,
            container: &str,
            child: &str,
            body: Bytes,
            ct: &str,
        ) -> ServerResult<ResourceMeta> {
            self.inner
                .create_in_container(container, child, body, ct)
                .await
        }
        async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
            if self.fail_delete.load(Ordering::SeqCst)
                || self.fail_delete_of.lock().unwrap().as_deref() == Some(iri)
            {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.delete(iri, parent).await
        }
        async fn delete_container_if_empty(
            &self,
            iri: &str,
            parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            self.inner.delete_container_if_empty(iri, parent).await
        }
        async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
            self.inner.list_children(container).await
        }
    }

    /// A request to `path` (with an optional query) carrying `headers` and `body`.
    pub fn request(
        method: axum::http::Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> super::LwsRequest {
        let (path, query) = match path.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (path.to_string(), None),
        };
        let mut map = axum::http::HeaderMap::new();
        for (k, v) in headers {
            map.append(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        super::LwsRequest {
            method,
            path,
            query,
            headers: map,
            body: Bytes::from(body.to_string()),
        }
    }

    /// An open-mode state over a [`FlakyStore`].
    pub async fn state(page_size: usize) -> (super::LwsState<FlakyStore>, FlakyStore) {
        let store = FlakyStore::new();
        let mut cfg = super::LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.page_size = page_size;
        let state = super::LwsState::new(store.clone(), cfg).await.unwrap();
        (state, store)
    }

    /// The `Link` targets of `resp` with relation `rel`.
    pub fn links(resp: &axum::response::Response, rel: &str) -> Vec<String> {
        resp.headers()
            .get_all(axum::http::header::LINK)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(super::parse_links)
            .filter(|(_, p)| p.get("rel").map(String::as_str) == Some(rel))
            .map(|(t, _)| t)
            .collect()
    }

    pub async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_store::{body_json, links, request};

    async fn http(app: &Router, method: &str, path: &str, body: &'static str) -> StatusCode {
        use tower::ServiceExt;
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("slug", "f")
            .header("content-type", "text/plain")
            .body(Body::from(body))
            .unwrap();
        let peer: std::net::SocketAddr = "203.0.113.7:4000".parse().unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(peer));
        app.clone().oneshot(req).await.unwrap().status()
    }

    async fn open_router(max_body: usize) -> Router {
        let mut cfg = LwsConfig::new("http://h");
        cfg.open = true;
        cfg.max_body = max_body;
        let store = crate::store::CompositeStore::new(
            crate::store::InMemorySparqClient::new(),
            crate::store::InMemoryBlobStore::new(),
        );
        router(store, cfg).await.unwrap()
    }

    #[tokio::test]
    async fn the_body_ceiling_is_the_configured_one() {
        let app = open_router(16).await;
        assert_eq!(
            http(&app, "POST", "/", "0123456789abcdefg").await,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            http(&app, "POST", "/", "0123456789abcdef").await,
            StatusCode::CREATED
        );
        assert_eq!(
            LwsConfig::new("http://h").max_body,
            crate::body_limit::DEFAULT_MAX_BODY_BYTES
        );
    }

    #[tokio::test]
    async fn the_overload_layers_wrap_the_lws_router() {
        use crate::app::{with_overload_layers, OverloadConfig};
        // Admission control: at capacity, a request is shed with 503; the probes are not.
        let admission = crate::overload::AdmissionControl::new(1);
        let mut overload = OverloadConfig::new(1, Some(std::time::Duration::from_secs(5)));
        overload.admission = admission.clone();
        let app = with_overload_layers(open_router(1024).await, overload);
        let permit = admission.try_admit_for_test().unwrap();
        assert_eq!(
            http(&app, "GET", "/", "").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(http(&app, "GET", "/livez", "").await, StatusCode::OK);
        assert_eq!(http(&app, "GET", "/readyz", "").await, StatusCode::OK);
        drop(permit);
        assert_eq!(http(&app, "GET", "/", "").await, StatusCode::OK);
        // The per-IP rate limiter: past the burst, 429; the probes are not limited.
        let mut overload = OverloadConfig::new(100, None);
        overload.rate_limiter = Some(crate::rate_limit::RateLimiter::new(
            0.0001, 1.0, 0, false, false,
        ));
        let app = with_overload_layers(open_router(1024).await, overload);
        assert_eq!(http(&app, "GET", "/", "").await, StatusCode::OK);
        assert_eq!(
            http(&app, "GET", "/", "").await,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(http(&app, "GET", "/livez", "").await, StatusCode::OK);
        // The request timeout: a request that does not finish in time is a 504.
        let mut overload = OverloadConfig::new(100, Some(std::time::Duration::from_millis(1)));
        overload.rate_limiter = None;
        let slow = Router::new().fallback(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            StatusCode::OK
        });
        let app = with_overload_layers(slow, overload);
        assert_eq!(
            http(&app, "GET", "/", "").await,
            StatusCode::GATEWAY_TIMEOUT
        );
    }

    #[test]
    fn container_negotiation() {
        assert_eq!(negotiate_container(None), Some(LWS_JSON));
        assert_eq!(negotiate_container(Some("application/json")), Some(JSON));
        assert_eq!(
            negotiate_container(Some("application/ld+json")),
            Some(LD_JSON)
        );
        assert_eq!(
            negotiate_container(Some(
                "application/ld+json; profile=\"https://www.w3.org/ns/lws/v1\""
            )),
            Some(LWS_JSON)
        );
        assert_eq!(negotiate_container(Some("*/*")), Some(LWS_JSON));
        assert_eq!(
            negotiate_container(Some("application/json, application/lws+json;q=0.5")),
            Some(JSON)
        );
        assert_eq!(negotiate_container(Some("text/turtle")), None);
        assert_eq!(negotiate_container(Some("application/json;q=0")), None);
    }

    #[test]
    fn lws_context_check() {
        assert!(has_lws_context(Some(&json!([LWS_CONTEXT]))));
        assert!(has_lws_context(Some(&json!(["https://x/", LWS_CONTEXT]))));
        assert!(has_lws_context(Some(&json!(LWS_CONTEXT))));
        assert!(!has_lws_context(Some(&json!(["https://x/"]))));
        assert!(!has_lws_context(None));
    }

    #[tokio::test]
    async fn service_listings_page_negotiate_and_tag() {
        let cfg = LwsConfig::new("http://h");
        let uri = cfg.absolute(GRANTS_PATH);
        let items: Vec<Value> = (0..5)
            .map(|i| json!({"id": format!("{uri}{i}"), "type": "DataResource"}))
            .collect();
        let mut cfg = cfg;
        cfg.page_size = 2;
        let cfg = cfg;
        let get = |q: &str, h: &[(&str, &str)]| {
            service_listing(
                &cfg,
                &request(Method::GET, &format!("{GRANTS_PATH}{q}"), h, ""),
                &uri,
                items.clone(),
                "v1",
            )
        };
        let first = get("", &[]);
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CONTENT_TYPE], LWS_JSON);
        assert_eq!(first.headers()[header::VARY], "Accept");
        assert_eq!(links(&first, "type"), vec![format!("{LWS_NS}Container")]);
        assert_eq!(links(&first, "up"), vec![cfg.storage()]);
        assert_eq!(links(&first, "linkset"), vec![meta_key(&uri)]);
        assert_eq!(links(&first, "first"), vec![format!("{uri}?page=1")]);
        assert_eq!(links(&first, "next"), vec![format!("{uri}?page=2")]);
        assert_eq!(links(&first, "last"), vec![format!("{uri}?page=3")]);
        assert!(links(&first, "prev").is_empty());
        let etag = first.headers()[header::ETAG].to_str().unwrap().to_string();
        let doc = body_json(first).await;
        assert_eq!(doc["totalItems"], 5);
        assert_eq!(doc["items"].as_array().unwrap().len(), 2);

        let last = get("?page=3", &[("accept", "application/json")]);
        assert_eq!(last.headers()[header::CONTENT_TYPE], JSON);
        assert_eq!(links(&last, "prev"), vec![format!("{uri}?page=2")]);
        assert!(links(&last, "next").is_empty());
        assert_ne!(last.headers()[header::ETAG].to_str().unwrap(), etag);
        assert_eq!(body_json(last).await["items"].as_array().unwrap().len(), 1);

        assert_eq!(get("?page=4", &[]).status(), StatusCode::NOT_FOUND);
        assert_eq!(get("?page=0", &[]).status(), StatusCode::NOT_FOUND);
        assert_eq!(
            get("", &[("accept", "text/turtle")]).status(),
            StatusCode::NOT_ACCEPTABLE
        );
        assert_eq!(
            get("", &[("if-none-match", &etag)]).status(),
            StatusCode::NOT_MODIFIED
        );
        // A one-page listing carries no paging links.
        let mut roomy = cfg.clone();
        roomy.page_size = 10;
        let one = service_listing(
            &roomy,
            &request(Method::GET, GRANTS_PATH, &[], ""),
            &uri,
            items.clone(),
            "v1",
        );
        assert!(links(&one, "first").is_empty());
    }

    #[test]
    fn the_jwks_publishes_the_previous_key() {
        let old = jose::EcKey::generate("old");
        let mut cfg = LwsConfig::new("http://h");
        cfg.as_previous_key =
            Some(jose::VerifyKey::from_jwk(&old.public_jwk().to_string()).unwrap());
        let kids: Vec<String> = authz_server::jwks(&cfg)["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["kid"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kids, vec![cfg.as_key.kid().to_string(), "old".to_string()]);
    }

    /// Review finding: every generated access-token key was `lws-as-1`, so the documented rotation
    /// (the old file as the previous key, a fresh file as the current one) met two keys with one
    /// kid and refused to start.
    #[test]
    fn rotating_generated_keys_gives_distinct_kids() {
        let dir = std::env::temp_dir().join(format!("lws-keys-{}", jose::random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = |n: &str| dir.join(n).to_string_lossy().into_owned();
        let old = key_file(&path("as.jwk"), None).unwrap();
        assert_eq!(old.kid(), old.thumbprint());
        // Restarting keeps the key and its kid.
        assert_eq!(key_file(&path("as.jwk"), None).unwrap().kid(), old.kid());
        // Rotate: the old file becomes the previous key, a new path the current one.
        std::fs::rename(path("as.jwk"), path("as-previous.jwk")).unwrap();
        let new = key_file(&path("as-new.jwk"), None).unwrap();
        assert_ne!(new.kid(), old.kid());
        let previous = std::fs::read_to_string(path("as-previous.jwk")).unwrap();
        let (current, prev) = rotated_as_keys(new.clone(), &previous).unwrap();
        assert_eq!(current.kid(), new.kid());
        assert_eq!(prev.kid(), old.kid());
        std::fs::remove_dir_all(&dir).unwrap();

        // Two keys generated before, both `lws-as-1`: the current one goes by its thumbprint and
        // the previous one keeps the kid its tokens name.
        let legacy_old = jose::EcKey::generate("lws-as-1");
        let legacy_new = jose::EcKey::generate("lws-as-1");
        let (current, prev) =
            rotated_as_keys(legacy_new.clone(), &legacy_old.private_jwk().to_string()).unwrap();
        assert_eq!(current.kid(), legacy_new.thumbprint());
        assert_eq!(prev.kid(), "lws-as-1");
        // The same key as both is still refused, as is a previous key claiming the current
        // key's thumbprint.
        assert!(rotated_as_keys(legacy_new.clone(), &legacy_new.public_jwk().to_string()).is_err());
        let fresh = jose::EcKey::generate_thumbprinted();
        let squatter = jose::EcKey::generate(fresh.kid());
        assert!(rotated_as_keys(fresh, &squatter.public_jwk().to_string()).is_err());
    }

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
