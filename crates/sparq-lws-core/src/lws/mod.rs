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
//!   `/.well-known/lws-configuration`, a JWKS, and RFC 8693 token exchange for did:key and
//!   controlled identifier subject tokens; the storage accepts the RFC 9068 access tokens
//!   it issues ([`tokens`]).
//!
//! Authorization ([`access`]): the storage owner (`SOLID_SERVER_LWS_OWNER`) may do anything, and
//! the agent that created a resource may do anything with it.
//! `SOLID_SERVER_LWS_OPEN=1` is a development mode with no authentication at all.

pub mod access;
pub mod authz_server;
pub mod jose;
pub mod resources;
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

pub const LWS_JSON: &str = "application/lws+json";
pub const LWS_CID: &str = "application/lws+cid";
pub const LD_JSON: &str = "application/ld+json";
pub const JSON: &str = "application/json";
pub const LINKSET_JSON: &str = "application/linkset+json";
pub const MERGE_PATCH: &str = "application/merge-patch+json";
pub const JSON_PATCH: &str = "application/json-patch+json";
pub const PROBLEM_JSON: &str = "application/problem+json";

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
    /// Lifetime of issued access tokens, in seconds.
    pub token_ttl_secs: i64,
    /// Let the authorization server reach `http:` and loopback or private addresses (identity
    /// documents). Development and conformance testing only.
    pub allow_insecure_fetch: bool,
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
            token_ttl_secs: 300,
            allow_insecure_fetch: false,
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
    /// - `SOLID_SERVER_LWS_TOKEN_TTL_SECS`: access token lifetime (default 300);
    /// - `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH=1`: allow `http:` and private-address fetches
    ///   (development and conformance only);
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
        // Open mode is for local test harnesses, whose documents are on private hosts.
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
/// outbound request (a fetch) cannot be pointed at the server's own
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

/// Whether an address is one the server must not be made to reach: anything but a global unicast
/// address. The one predicate every outbound request uses: identity documents, OpenID Providers,
/// and JWKS alike.
pub fn is_forbidden_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 shared address space
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15 benchmarking
                || o[0] >= 240
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(std::net::IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] & 0xffc0) == 0xfec0 // site local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x0064 && s[1] == 0xff9b) // NAT64
                || (s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0)
            // IPv4-compatible
        }
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
    /// The part of `types` declared by Link headers (on the create, or with `Prefer:
    /// set-linkset`), kept apart from those the content states: replacing the content replaces
    /// only its own. `None` in metadata written before the two were kept apart, whose declared
    /// types are `types` less those its content states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_types: Option<Vec<String>>,
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
    /// A write of the resource's content and metadata is under way, or failed part way: the
    /// content may not be the one this metadata describes, so only the owner and the creator
    /// (whose access rests on neither) may act on it until a write completes (see
    /// `resources::write_with_meta`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
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
    /// A [`Journal`] for one mutation, bounded to what any one write could need to put back (a
    /// body at the limit and its metadata, twice) and never below 64 MiB.
    pub(crate) fn journal(&self) -> Journal<'_, S> {
        let limit = (64usize << 20).max(
            self.cfg
                .max_body
                .saturating_add(MAX_META_BYTES)
                .saturating_mul(2),
        );
        Journal::new(&self.store, limit)
    }

    /// Build the state: ensure the storage root exists.
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
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                cfg,
                http,
                locks: Default::default(),
            }),
        })
    }

    /// Whether `agent` may perform `action` on the resource at `uri` (see [`access::allowed`]).
    /// Metadata that cannot be read (a backend failure, or stored metadata that does not parse)
    /// denies: the decision rests on it. A handler that can answer with an error uses
    /// [`LwsState::check`] instead.
    pub async fn allowed(&self, action: access::Action, uri: &str, agent: &Agent) -> bool {
        access::allowed(self, action, uri, agent)
            .await
            .unwrap_or(false)
    }

    /// As [`LwsState::allowed`], with metadata that cannot be read an error rather than a denial.
    pub async fn check(
        &self,
        action: access::Action,
        uri: &str,
        agent: &Agent,
    ) -> Result<bool, crate::error::ServerError> {
        access::allowed(self, action, uri, agent).await
    }

    /// The LWS metadata stored beside `iri`, or the default when there is none. Stored metadata
    /// that does not parse is an error, never the default: that would quietly drop the creator,
    /// the types and the links it records.
    pub async fn resource_meta(
        &self,
        iri: &str,
    ) -> Result<ResourceMeta, crate::error::ServerError> {
        match self.store.read(&meta_key(iri)).await {
            Ok(r) => parse_meta(iri, &r.body),
            Err(crate::error::ServerError::NotFound) => Ok(ResourceMeta::default()),
            Err(e) => Err(e),
        }
    }

    /// Store `meta` beside `iri`. What is written is checked to read back first (see
    /// [`encode_meta`]), so metadata that could not be read again is never stored.
    pub async fn put_resource_meta(
        &self,
        iri: &str,
        meta: &ResourceMeta,
    ) -> Result<(), crate::error::ServerError> {
        let body = encode_meta(meta)?;
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
#[derive(Clone)]
pub struct LwsRequest {
    pub method: Method,
    /// The raw path, percent-encoded, starting with `/`.
    pub path: String,
    pub query: Option<String>,
    pub headers: HeaderMap,
    pub body: Bytes,
    /// The request's share of its admission permit, when the server runs admission control. A
    /// write detached from the request holds it until the write ends.
    pub admission: Option<crate::overload::AdmissionSlot>,
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

/// Remove the member `iri` of `parent` (if any): its record and its parent's membership edge in
/// one store step, so no failure can leave a live record its container no longer lists. Every
/// removal of a member goes through here: resources and service records.
/// A container that is not empty is not removed ([`crate::store::DeleteOutcome::NotEmpty`]).
pub(crate) async fn remove_member<S: Store>(
    store: &S,
    iri: &str,
    parent: Option<&str>,
) -> Result<crate::store::DeleteOutcome, crate::error::ServerError> {
    store.delete_container_if_empty(iri, parent).await
}

/// One mutation's store changes, each recorded with what it replaced, so that a failure part way
/// puts back everything done before it ([`Journal::rollback`]). Every mutation of more than one
/// store step goes through one: a write or delete that fails leaves content, metadata and
/// membership as they were. What a journal holds to put things back is bounded (`limit` bytes);
/// a change that would need more is refused, and what was done before it is put back.
pub(crate) struct Journal<'a, S: Store> {
    store: &'a S,
    undo: Vec<Undo>,
    held: usize,
    limit: usize,
}

/// Whether a store step with this outcome may have changed something: it succeeded, or failed in
/// the backend, where the change may have landed before the failure was reported.
fn may_have_happened<T>(outcome: &Result<T, crate::error::ServerError>) -> bool {
    matches!(outcome, Ok(_) | Err(crate::error::ServerError::Storage(_)))
}

/// How to put back one change a [`Journal`] made.
enum Undo {
    /// `key` as it was: its bytes and content type, or absent.
    Restore {
        key: String,
        prior: Option<(Bytes, String)>,
    },
    /// A removed member, recreated in its container.
    Recreate {
        iri: String,
        parent: Option<String>,
        body: Bytes,
        content_type: String,
    },
}

impl<'a, S: Store> Journal<'a, S> {
    pub(crate) fn new(store: &'a S, limit: usize) -> Self {
        Self {
            store,
            undo: Vec::new(),
            held: 0,
            limit,
        }
    }

    /// What is stored at `key` now, to put back later; charged to the journal's bound.
    async fn prior(
        &mut self,
        key: &str,
    ) -> Result<Option<(Bytes, String)>, crate::error::ServerError> {
        let prior = match self.store.read(key).await {
            Ok(r) => Some((r.body, r.meta.content_type)),
            Err(crate::error::ServerError::NotFound) => None,
            Err(e) => return Err(e),
        };
        self.held = self
            .held
            .saturating_add(key.len() + prior.as_ref().map_or(0, |(b, ct)| b.len() + ct.len()));
        if self.held > self.limit {
            return Err(crate::error::ServerError::Conflict(
                "the change is too large to make atomically".into(),
            ));
        }
        Ok(prior)
    }

    /// Write `body` at `key`.
    pub(crate) async fn write(
        &mut self,
        key: &str,
        body: Bytes,
        content_type: &str,
    ) -> Result<crate::store::ResourceMeta, crate::error::ServerError> {
        let prior = self.prior(key).await?;
        let written = self.store.write(key, body, content_type).await;
        // A backend failure may follow a write that landed (a lost reply): it is put back all the
        // same. A refusal wrote nothing, and there is nothing to put back.
        if may_have_happened(&written) {
            self.undo.push(Undo::Restore {
                key: key.to_string(),
                prior,
            });
        }
        written
    }

    /// Write the metadata of `iri`.
    pub(crate) async fn write_meta(
        &mut self,
        iri: &str,
        meta: &ResourceMeta,
    ) -> Result<(), crate::error::ServerError> {
        let body = encode_meta(meta)?;
        self.write(&meta_key(iri), Bytes::from(body), JSON)
            .await
            .map(|_| ())
    }

    /// Remove `key` (metadata, not a member: see [`Journal::remove_member`]); absent is fine.
    pub(crate) async fn delete(&mut self, key: &str) -> Result<(), crate::error::ServerError> {
        let prior = self.prior(key).await?;
        if prior.is_none() {
            return Ok(());
        }
        let deleted = self.store.delete(key, None).await;
        if may_have_happened(&deleted) {
            self.undo.push(Undo::Restore {
                key: key.to_string(),
                prior,
            });
        }
        match deleted {
            Ok(()) | Err(crate::error::ServerError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Remove the member `iri` of `parent` (see [`remove_member`]).
    pub(crate) async fn remove_member(
        &mut self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<crate::store::DeleteOutcome, crate::error::ServerError> {
        let Some((body, content_type)) = self.prior(iri).await? else {
            return Ok(crate::store::DeleteOutcome::NotFound);
        };
        let outcome = remove_member(self.store, iri, parent).await;
        // Only a member that went, or may have, is recreated.
        if matches!(
            outcome,
            Ok(crate::store::DeleteOutcome::Deleted) | Err(crate::error::ServerError::Storage(_))
        ) {
            self.undo.push(Undo::Recreate {
                iri: iri.to_string(),
                parent: parent.map(str::to_string),
                body,
                content_type,
            });
        }
        outcome
    }

    /// Keep every change.
    pub(crate) fn commit(self) {}

    /// Put back every change, the last first. Stops at the first step that fails, so what is
    /// left is a prefix of the mutation (with the metadata written first, a `pending` mark
    /// stays: see the resources write path), never a mix.
    pub(crate) async fn rollback(self) -> Result<(), crate::error::ServerError> {
        for undo in self.undo.into_iter().rev() {
            match undo {
                Undo::Restore {
                    key,
                    prior: Some((body, ct)),
                } => {
                    self.store.write(&key, body, &ct).await?;
                }
                Undo::Restore { key, prior: None } => match self.store.delete(&key, None).await {
                    Ok(()) | Err(crate::error::ServerError::NotFound) => {}
                    Err(e) => return Err(e),
                },
                Undo::Recreate {
                    iri,
                    parent,
                    body,
                    content_type,
                } => {
                    if self.store.exists(&iri).await? {
                        continue;
                    }
                    match parent {
                        Some(p) => {
                            self.store
                                .create_in_container(&p, &iri, body, &content_type)
                                .await?
                        }
                        None => self.store.write(&iri, body, &content_type).await?,
                    };
                }
            }
        }
        Ok(())
    }
}

/// Delete the stored member `iri` of `container`.
/// The index commit is the deletion point: when the store reports a failure but the record no
/// longer exists (what failed was the cleanup of its bytes, which the reconciler collects), the
/// record is gone.
pub(crate) async fn delete_record<S: Store + 'static>(
    state: &LwsState<S>,
    iri: &str,
    container: &str,
) -> Result<(), crate::error::ServerError> {
    match remove_member(&state.store, iri, Some(container)).await {
        Ok(_) | Err(crate::error::ServerError::NotFound) => Ok(()),
        Err(e) => match state.store.exists(iri).await {
            Ok(false) => Ok(()),
            _ => Err(e),
        },
    }
}

/// How many bytes of expanded terms the triples parsed from `body_len` bytes of Turtle may hold
/// between them. A prefix is written once and expanded at every use, so a short document can
/// stand for an unbounded amount of text: parsing stops (and the document is refused) past this.
pub(crate) fn expansion_budget(body_len: usize) -> usize {
    body_len.saturating_mul(16).saturating_add(1 << 20)
}

/// The bytes a parsed triple's terms hold once expanded, as [`expansion_budget`] counts them.
pub(crate) fn triple_bytes(t: &oxrdf::Triple) -> usize {
    let node = |n: &oxrdf::NamedOrBlankNode| match n {
        oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().len(),
        oxrdf::NamedOrBlankNode::BlankNode(b) => b.as_str().len(),
    };
    #[allow(unreachable_patterns)]
    let object = match &t.object {
        oxrdf::Term::NamedNode(n) => n.as_str().len(),
        oxrdf::Term::BlankNode(b) => b.as_str().len(),
        oxrdf::Term::Literal(l) => {
            l.value().len() + l.datatype().as_str().len() + l.language().map_or(0, str::len)
        }
        other => other.to_string().len(),
    };
    node(&t.subject) + t.predicate.as_str().len() + object
}

/// Stored metadata of `iri`, parsed; metadata that does not parse is an error.
pub(crate) fn parse_meta(
    iri: &str,
    body: &[u8],
) -> Result<ResourceMeta, crate::error::ServerError> {
    serde_json::from_slice(body).map_err(|e| {
        crate::error::ServerError::Storage(format!("the metadata of {iri} is malformed: {e}"))
    })
}

/// `meta` serialized, once it is known to parse again: JSON that nests deeper than the parser
/// accepts (a linkset extension nested to its limit, one level deeper once inside the metadata)
/// serializes fine but never reads back.
pub(crate) fn encode_meta(meta: &ResourceMeta) -> Result<Vec<u8>, crate::error::ServerError> {
    let body = serde_json::to_vec(meta)
        .map_err(|e| crate::error::ServerError::Storage(format!("metadata: {e}")))?;
    if body.len() > MAX_META_BYTES {
        return Err(crate::error::ServerError::Conflict(format!(
            "the resource's types and links would exceed {MAX_META_BYTES} bytes"
        )));
    }
    parse_meta("the resource", &body)?;
    Ok(body)
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
    if let Err(refused) = check_headers(&parts.headers) {
        return refused;
    }
    let limit = state.cfg.max_body;
    let body = match axum::body::to_bytes(body, limit).await {
        Ok(b) => b,
        Err(_) => return problem(StatusCode::PAYLOAD_TOO_LARGE, None),
    };
    let req = LwsRequest {
        method: parts.method,
        path: parts.uri.path().to_string(),
        query: parts.uri.query().map(str::to_string),
        admission: parts
            .extensions
            .get::<crate::overload::AdmissionSlot>()
            .cloned(),
        headers: parts.headers,
        body,
    };
    let is_head = req.is_head();
    let mut resp = route(&state, req).await;
    // A 304 or 204 has no content, and no length to declare: a 304's Content-Length would have
    // to be the 200's (RFC 9110 section 8.6), which an empty body is not. Its body declares no
    // length, so none is derived from it on the way out either.
    if matches!(
        resp.status(),
        StatusCode::NOT_MODIFIED | StatusCode::NO_CONTENT
    ) {
        resp.headers_mut().remove(header::CONTENT_LENGTH);
        *resp.body_mut() = Body::new(NoContent);
    } else if is_head {
        // HEAD carries the headers a GET would, never a body: the length is the one the body
        // would have had (read before the body goes, since the length is otherwise derived from
        // the body once the response is sent).
        use axum::body::HttpBody;
        let len = resp
            .headers()
            .get(header::CONTENT_LENGTH)
            .cloned()
            .or_else(|| {
                resp.body()
                    .size_hint()
                    .exact()
                    .map(header::HeaderValue::from)
            });
        *resp.body_mut() = Body::empty();
        if let Some(len) = len {
            resp.headers_mut().insert(header::CONTENT_LENGTH, len);
        }
    }
    resp
}

/// The body of a response that has no content (a 304 or 204): empty, with no exact length, so
/// none is declared for it.
struct NoContent;

impl axum::body::HttpBody for NoContent {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        std::task::Poll::Ready(None)
    }
}

async fn route<S: Store + 'static>(state: &LwsState<S>, req: LwsRequest) -> Response {
    let path = req.path.as_str();
    // Unauthenticated liveness and readiness probes, as the Solid surface serves them.
    if matches!(path, "/livez" | "/readyz") && matches!(req.method, Method::GET | Method::HEAD) {
        return (StatusCode::OK, "ok").into_response();
    }
    // Every route that takes a body, the token endpoint and the service routes included, refuses
    // a content coding here, once, before anything reads the body.
    if let Some(refused) = resources::refuse_encoded(&req) {
        return refused;
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
// The access grant and access request services are LWS containers (access requests section
// 11.5), held in memory rather than in the store. These
// helpers give their listings what [`resources`] gives a stored container: content negotiation,
// paging, an entity tag and the container links.

/// Whether `v` is an absolute IRI (RFC 3987): a scheme, a colon and something after it, with
/// every character and percent-encoding the grammar allows. One parser checks every IRI a client
/// sends, so `%ZZ`, a space or a bare fragment is refused everywhere alike.
pub fn is_uri(v: &str) -> bool {
    v.split_once(':').is_some_and(|(_, rest)| !rest.is_empty()) && oxiri::Iri::parse(v).is_ok()
}

/// Most link relations (each a target and one relation) a request's Link headers may declare.
pub const MAX_DECLARED_LINKS: usize = 128;

/// Most bytes the link relations a request's Link headers declare may hold between them, once
/// each relation has its own copy of its target.
pub const MAX_DECLARED_LINK_BYTES: usize = 64 * 1024;

/// Most members a list-valued request header (`Accept`, `Prefer`, `If-Match`, `If-None-Match`)
/// may hold.
pub const MAX_HEADER_MEMBERS: usize = 64;

/// Largest metadata a resource may have, as stored: its types, links and linkset together. Every
/// metadata write goes through [`encode_meta`], which refuses more, and a write of content and
/// metadata checks the metadata before writing either, so no sequence of writes grows a
/// resource's metadata past it.
pub const MAX_META_BYTES: usize = 256 * 1024;

/// The bounds every request's headers are held to before anything is built from them. Headers
/// are input as bodies are: the transport bounds their bytes, and this bounds what they expand
/// to, so no header-derived structure (the links and types a request declares, a negotiation, a
/// precondition list) grows past a fixed size whatever the headers say.
#[allow(clippy::result_large_err)]
pub(crate) fn check_headers(headers: &HeaderMap) -> Result<(), Response> {
    let too_large = |what: &str| {
        problem(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            Some(&format!("{what} declares more than this server accepts")),
        )
    };
    let all = |name: header::HeaderName| {
        headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (mut count, mut bytes) = (0usize, 0usize);
    for (target, params) in parse_links(&all(header::LINK)) {
        let rels = params
            .get("rel")
            .map_or(0, |r| r.split_whitespace().count());
        count = count.saturating_add(rels.max(1));
        bytes = bytes.saturating_add(rels.max(1).saturating_mul(target.len()));
        if count > MAX_DECLARED_LINKS || bytes > MAX_DECLARED_LINK_BYTES {
            return Err(too_large("the Link header"));
        }
    }
    for name in [
        header::ACCEPT,
        header::HeaderName::from_static("prefer"),
        header::IF_MATCH,
        header::IF_NONE_MATCH,
    ] {
        if all(name.clone()).split(',').count() > MAX_HEADER_MEMBERS {
            return Err(too_large(name.as_str()));
        }
    }
    Ok(())
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

    /// A gate held for one IRI (see [`FlakyStore::hold_next_write_of`]).
    pub type GateOf = Arc<std::sync::Mutex<Option<(String, Arc<tokio::sync::Semaphore>)>>>;

    #[derive(Clone)]
    pub struct FlakyStore {
        inner: Arc<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>,
        pub fail_delete: Arc<AtomicBool>,
        /// `delete` of this IRI alone fails with a backend error.
        pub fail_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `read` of this IRI alone fails with a backend error.
        pub fail_read_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `write` of this IRI alone fails with a backend error.
        pub fail_write_of: Arc<std::sync::Mutex<Option<String>>>,
        /// When set, how many more writes succeed; every write past them fails, as in a store
        /// with a bounded number of blob slots.
        pub write_budget: Arc<std::sync::Mutex<Option<usize>>>,
        /// `exists` fails with a backend error.
        pub fail_exists: Arc<AtomicBool>,
        /// `exists` of this IRI alone fails with a backend error.
        pub fail_exists_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `exists` reports every IRI that starts with this present.
        pub occupied: Arc<std::sync::Mutex<Option<String>>>,
        /// The next `create_in_container` waits for a permit of this gate, as on a slow remote
        /// backend. The create is already sent: dropping the call does not stop it, and it
        /// commits once the gate opens.
        pub hold_next_create: Arc<std::sync::Mutex<Option<Arc<tokio::sync::Semaphore>>>>,
        /// As `hold_next_create`, for the next `write` of the IRI named.
        pub hold_next_write_of: GateOf,
        /// `delete` of this IRI removes it, then reports a failure, as when the cleanup of its
        /// bytes fails after the index commit.
        pub fail_after_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// As `hold_next_create`, for the next `delete` of the IRI named.
        pub hold_next_delete_of: GateOf,
        /// `exists` reports this IRI absent, as if it was checked just before the IRI appeared.
        pub hide: Arc<std::sync::Mutex<Option<String>>>,
        /// `write` of this IRI commits, then reports a backend failure, as when a remote store's
        /// reply is lost after the update landed.
        pub fail_after_write_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `create_in_container` commits, then reports a backend failure, as when a remote
        /// store's reply is lost after the update landed.
        pub fail_after_create: Arc<AtomicBool>,
        /// `write` of this IRI is refused before anything is written, as by a full store.
        pub refuse_write_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `delete` of this IRI detaches it from its parent, then fails with the record still
        /// there, as a two-step delete does when its second step fails.
        pub partial_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// How many `delete`s named a parent: removals of a member that are not one step.
        pub two_step_deletes: Arc<std::sync::atomic::AtomicUsize>,
        /// When set to `n`, the store step (write, create, delete) after the next `n` fails, once,
        /// before it changes anything; then it is cleared. See [`each_failure_changes_nothing`].
        pub fail_step: Arc<std::sync::Mutex<Option<usize>>>,
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
                hold_next_create: Default::default(),
                hold_next_write_of: Default::default(),
                hold_next_delete_of: Default::default(),
                fail_after_delete_of: Default::default(),
                fail_exists_of: Default::default(),
                occupied: Default::default(),
                fail_delete_of: Default::default(),
                fail_write_of: Default::default(),
                write_budget: Default::default(),
                fail_read_of: Default::default(),
                hide: Default::default(),
                fail_after_write_of: Default::default(),
                refuse_write_of: Default::default(),
                fail_after_create: Default::default(),
                partial_delete_of: Default::default(),
                two_step_deletes: Default::default(),
                fail_step: Default::default(),
            }
        }

        /// Count a store step against [`FlakyStore::fail_step`].
        fn step(&self) -> ServerResult<()> {
            let mut slot = self.fail_step.lock().unwrap();
            match slot.as_mut() {
                Some(0) => {
                    *slot = None;
                    Err(ServerError::Storage("the injected failure".into()))
                }
                Some(n) => {
                    *n -= 1;
                    Ok(())
                }
                None => Ok(()),
            }
        }
    }

    /// The gate held for `iri`, taken (so only the next call waits on it).
    fn held(slot: &GateOf, iri: &str) -> Option<Arc<tokio::sync::Semaphore>> {
        let mut slot = slot.lock().unwrap();
        if slot.as_ref().is_some_and(|(i, _)| i == iri) {
            return slot.take().map(|(_, g)| g);
        }
        None
    }

    #[async_trait]
    impl Store for FlakyStore {
        async fn read(&self, iri: &str) -> ServerResult<Resource> {
            if self.fail_read_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.read(iri).await
        }
        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            self.inner.meta(iri).await
        }
        async fn exists(&self, iri: &str) -> ServerResult<bool> {
            if self.fail_exists.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if self.fail_exists_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if self.hide.lock().unwrap().as_deref() == Some(iri) {
                return Ok(false);
            }
            if let Some(p) = self.occupied.lock().unwrap().as_deref() {
                if iri.starts_with(p) {
                    return Ok(true);
                }
            }
            self.inner.exists(iri).await
        }
        async fn write(&self, iri: &str, body: Bytes, ct: &str) -> ServerResult<ResourceMeta> {
            self.step()?;
            if self.fail_write_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if self.refuse_write_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::InsufficientStorage);
            }
            if let Some(left) = self.write_budget.lock().unwrap().as_mut() {
                if *left == 0 {
                    return Err(ServerError::InsufficientStorage);
                }
                *left -= 1;
            }
            if let Some(gate) = held(&self.hold_next_write_of, iri) {
                let (inner, iri, ct) = (self.inner.clone(), iri.to_string(), ct.to_string());
                let sent = tokio::spawn(async move {
                    gate.acquire().await.expect("gate").forget();
                    inner.write(&iri, body, &ct).await
                });
                return sent.await.expect("write");
            }
            let written = self.inner.write(iri, body, ct).await;
            if self.fail_after_write_of.lock().unwrap().as_deref() == Some(iri) {
                written?;
                return Err(ServerError::Storage("the reply was lost".into()));
            }
            written
        }
        async fn create_in_container(
            &self,
            container: &str,
            child: &str,
            body: Bytes,
            ct: &str,
        ) -> ServerResult<ResourceMeta> {
            self.step()?;
            let gate = self.hold_next_create.lock().unwrap().take();
            if let Some(gate) = gate {
                let (inner, container, child, ct) = (
                    self.inner.clone(),
                    container.to_string(),
                    child.to_string(),
                    ct.to_string(),
                );
                let sent = tokio::spawn(async move {
                    gate.acquire().await.expect("gate").forget();
                    inner
                        .create_in_container(&container, &child, body, &ct)
                        .await
                });
                return sent.await.expect("create");
            }
            let created = self
                .inner
                .create_in_container(container, child, body, ct)
                .await;
            if self.fail_after_create.load(Ordering::SeqCst) {
                created?;
                return Err(ServerError::Storage("the reply was lost".into()));
            }
            created
        }
        async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
            self.step()?;
            if parent.is_some() {
                self.two_step_deletes.fetch_add(1, Ordering::SeqCst);
            }
            if self.partial_delete_of.lock().unwrap().as_deref() == Some(iri) {
                let kept = self.inner.read(iri).await?;
                self.inner.delete(iri, parent).await?;
                let ct = kept.meta.content_type.clone();
                self.inner.write(iri, kept.body, &ct).await?;
                return Err(ServerError::Storage("the record delete failed".into()));
            }
            if self.fail_delete.load(Ordering::SeqCst)
                || self.fail_delete_of.lock().unwrap().as_deref() == Some(iri)
            {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if let Some(gate) = held(&self.hold_next_delete_of, iri) {
                let (inner, iri, parent) = (
                    self.inner.clone(),
                    iri.to_string(),
                    parent.map(str::to_string),
                );
                let sent = tokio::spawn(async move {
                    gate.acquire().await.expect("gate").forget();
                    inner.delete(&iri, parent.as_deref()).await
                });
                return sent.await.expect("delete");
            }
            self.inner.delete(iri, parent).await?;
            if self.fail_after_delete_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("blob cleanup failed".into()));
            }
            Ok(())
        }
        async fn delete_container_if_empty(
            &self,
            iri: &str,
            parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            // The same failures as `delete`: a resource is removed by either.
            self.step()?;
            if self.fail_delete.load(Ordering::SeqCst)
                || self.fail_delete_of.lock().unwrap().as_deref() == Some(iri)
            {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if let Some(gate) = held(&self.hold_next_delete_of, iri) {
                let (inner, iri, parent) = (
                    self.inner.clone(),
                    iri.to_string(),
                    parent.map(str::to_string),
                );
                let sent = tokio::spawn(async move {
                    gate.acquire().await.expect("gate").forget();
                    inner
                        .delete_container_if_empty(&iri, parent.as_deref())
                        .await
                });
                return sent.await.expect("delete");
            }
            let outcome = self.inner.delete_container_if_empty(iri, parent).await?;
            if self.fail_after_delete_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("blob cleanup failed".into()));
            }
            Ok(outcome)
        }
        async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
            self.inner.list_children(container).await
        }
    }

    /// What can be seen of `resources` (each one's content, content type and stored metadata) and
    /// of `listings` (each container's members).
    pub async fn snapshot(
        store: &FlakyStore,
        resources: &[String],
        listings: &[String],
    ) -> Vec<String> {
        let mut out = Vec::new();
        for iri in resources {
            for key in [iri.clone(), super::meta_key(iri)] {
                out.push(match store.inner.read(&key).await {
                    Ok(r) => format!("{key}: {} {:?}", r.meta.content_type, r.body),
                    Err(e) => format!("{key}: {e}"),
                });
            }
        }
        for container in listings {
            let mut members: Vec<String> = store
                .inner
                .list_children(container)
                .await
                .map(|c| c.iter().map(|m| m.as_str().to_string()).collect())
                .unwrap_or_default();
            members.sort();
            out.push(format!("{container} holds {members:?}"));
        }
        out
    }

    /// Run a mutation over and over, failing its first store step, then its second, and so on,
    /// each time on a fresh state from `setup` (which names the resources and listings the
    /// mutation may change). Every run that fails must leave them all exactly as they were: the
    /// mutation is whole or not at all. Ends at the first run that hits no injected failure, which
    /// must succeed; returns how many steps it took.
    pub async fn each_failure_changes_nothing<Setup, SetupFut, Op, OpFut>(
        setup: Setup,
        op: Op,
    ) -> usize
    where
        Setup: Fn() -> SetupFut,
        SetupFut: std::future::Future<
            Output = (
                super::LwsState<FlakyStore>,
                FlakyStore,
                Vec<String>,
                Vec<String>,
            ),
        >,
        Op: Fn(super::LwsState<FlakyStore>) -> OpFut,
        OpFut: std::future::Future<Output = axum::http::StatusCode>,
    {
        for n in 0..10_000 {
            let (st, store, resources, listings) = setup().await;
            let before = snapshot(&store, &resources, &listings).await;
            *store.fail_step.lock().unwrap() = Some(n);
            let status = op(st).await;
            let fired = store.fail_step.lock().unwrap().take().is_none();
            if !fired {
                assert!(
                    status.is_success(),
                    "the mutation failed unprovoked: {status}"
                );
                return n;
            }
            if !status.is_success() {
                assert_eq!(
                    snapshot(&store, &resources, &listings).await,
                    before,
                    "step {n} failed ({status}) and left a change behind"
                );
            }
        }
        panic!("the mutation never finished");
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
            admission: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_store::request;

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

    /// Review finding: one Link header with a long target and thousands of relations fitted the
    /// transport's header limit and expanded to hundreds of megabytes of copied targets. Every
    /// request's headers are held to what they may expand to before anything reads them.
    #[tokio::test]
    async fn every_list_header_is_held_to_its_bounds() {
        use tower::ServiceExt;
        let app = open_router(1 << 20).await;
        let send = |name: &'static str, value: String| {
            let app = app.clone();
            async move {
                let req = Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "text/plain")
                    .header(name, value)
                    .body(Body::from("x"))
                    .unwrap();
                app.oneshot(req).await.unwrap().status()
            }
        };
        let rels: Vec<String> = (0..=MAX_DECLARED_LINKS)
            .map(|i| format!("urn:r:{i}"))
            .collect();
        let many = format!("<https://e.example/t>; rel=\"{}\"", rels.join(" "));
        let long = format!(
            "<https://e.example/{}>; rel=\"urn:r:1 urn:r:2\"",
            "t".repeat(MAX_DECLARED_LINK_BYTES / 2)
        );
        let listed = |member: &str| vec![member; MAX_HEADER_MEMBERS + 1].join(", ");
        for (name, value) in [
            ("link", many),
            ("link", long),
            ("accept", listed("text/plain")),
            ("prefer", listed("return=minimal")),
            ("if-match", listed("\"e\"")),
            ("if-none-match", listed("\"e\"")),
        ] {
            assert_eq!(
                send(name, value).await,
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                "{name}"
            );
        }
        // Review finding: the bound weighed each target as sent, not as resolved and kept, so a
        // target that grows when resolved (against a long resource URI, or here by
        // percent-encoding) passed it. Targets are weighed as kept.
        let grows = format!(
            "<https://e.example/{}>; rel=\"{}\"",
            "{".repeat(400),
            rels[..MAX_DECLARED_LINKS].join(" ")
        );
        assert!(grows.len() < MAX_DECLARED_LINK_BYTES);
        assert_eq!(
            send("link", grows).await,
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
        );
        let fine = format!("<https://e.example/t>; rel=\"{}\"", rels[..8].join(" "));
        assert_eq!(send("link", fine).await, StatusCode::CREATED);
    }

    /// Review finding: a member's record and its parent's membership edge were removed in two
    /// steps on some paths, so a failure in between left
    /// a live record its container no longer listed. Every removal goes through
    /// [`remove_member`]; this drives each one and counts the two-step deletes it makes.
    #[tokio::test]
    async fn every_removal_takes_one_step() {
        use std::sync::atomic::Ordering;
        let (state, store) = test_store::state(100).await;
        let send = |method: Method, path: String, ct: &'static str, body: String| {
            let state = state.clone();
            async move {
                let headers: Vec<(&str, &str)> = vec![("content-type", ct), ("slug", "m")];
                route(&state, request(method, &path, &headers, &body)).await
            }
        };
        let local = |iri: &str| {
            iri.strip_prefix("http://localhost:3000")
                .unwrap()
                .to_string()
        };
        // A data resource, then a container with a member, recursively.
        let r = send(Method::POST, "/".into(), "text/plain", "x".into()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let at = local(r.headers()[header::LOCATION].to_str().unwrap());
        let r = send(Method::DELETE, at, "text/plain", String::new()).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let container = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
        let r = route(
            &state,
            request(Method::POST, "/", &[("slug", "c"), ("link", container)], ""),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let r = send(Method::POST, "/c/".into(), "text/plain", "x".into()).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let mut req = request(Method::DELETE, "/c/", &[("depth", "infinity")], "");
        req.headers.insert("depth", "infinity".parse().unwrap());
        assert_eq!(route(&state, req).await.status(), StatusCode::NO_CONTENT);
        assert_eq!(store.two_step_deletes.load(Ordering::SeqCst), 0);
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

    /// Review finding: the check took any scheme and a few excluded characters, so malformed
    /// IRIs such as `https://e/%ZZ` passed. Every IRI goes through one RFC 3987 parser.
    #[test]
    fn uri_check() {
        for good in [
            "https://example.org/x",
            "did:key:z6Mk",
            "urn:uuid:1",
            "https://e.example/caf%C3%A9",
            "https://e.example/caf\u{e9}",
            "https://[::1]:8080/a?b#c",
        ] {
            assert!(is_uri(good), "{good}");
        }
        for bad in [
            "#frag",
            "relative/path",
            "http://a b",
            "https://e.example/%ZZ",
            "https://e.example/%4",
            "https://[::1/",
            "1http://e.example/",
            "https:",
            "https://e.example/\u{7f}",
        ] {
            assert!(!is_uri(bad), "{bad}");
        }
    }

    #[test]
    fn meta_keys() {
        assert_eq!(meta_key("http://h/a/b"), "http://h/a/b.meta");
        assert_eq!(meta_key("http://h/a/b/"), "http://h/a/b/.meta");
        assert_eq!(meta_key("http://h/"), "http://h/.meta");
    }
}
