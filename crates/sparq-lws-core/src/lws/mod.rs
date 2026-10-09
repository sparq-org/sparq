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
    /// private addresses (identity documents, webhook inboxes). Development and
    /// conformance testing only.
    pub allow_insecure_fetch: bool,
    /// Largest request body read, in bytes; a larger one is refused with 413 before it is
    /// buffered further. The server-wide ceiling (`SOLID_SERVER_MAX_BODY_BYTES`, see
    /// [`crate::body_limit`]), the same one the Solid surface enforces.
    pub max_body: usize,
    /// Bounds on outgoing webhook deliveries (see [`notify::DeliveryLimits`]).
    pub delivery: notify::DeliveryLimits,
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
            max_body: crate::body_limit::DEFAULT_MAX_BODY_BYTES,
            delivery: notify::DeliveryLimits::default(),
        }
    }

    /// Read the `SOLID_SERVER_LWS_*` environment:
    /// - `SOLID_SERVER_LWS_OWNER`: the storage owner's agent IRI;
    /// - `SOLID_SERVER_LWS_OPEN=1` (or `SOLID_SERVER_OPEN_MODE=1`): no authentication (development
    ///   only);
    /// - `SOLID_SERVER_LWS_PAGE_SIZE`: members per container page (default 100);
    /// - `SOLID_SERVER_LWS_DELIVERY_QUEUE`, `SOLID_SERVER_LWS_DELIVERY_WORKERS`,
    ///   `SOLID_SERVER_LWS_DELIVERY_PER_INBOX`: webhook delivery bounds (see
    ///   [`notify::DeliveryLimits`]);
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
        let positive = |k: &str, into: &mut usize| -> Result<(), String> {
            if let Some(n) = var(k) {
                *into = n
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or(format!("{k} must be a positive integer"))?;
            }
            Ok(())
        };
        positive("SOLID_SERVER_LWS_DELIVERY_QUEUE", &mut cfg.delivery.queue)?;
        positive(
            "SOLID_SERVER_LWS_DELIVERY_WORKERS",
            &mut cfg.delivery.workers,
        )?;
        positive(
            "SOLID_SERVER_LWS_DELIVERY_PER_INBOX",
            &mut cfg.delivery.per_inbox,
        )?;
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

/// Whether an address is one the server must not be made to reach: anything but a global unicast
/// address. The one predicate every outbound request uses: identity documents, OpenID Providers,
/// JWKS and webhook inboxes alike.
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
    /// A [`Journal`] for one mutation, bounded to what any one write could need to put back (a
    /// body at the limit and its metadata, twice) and never below 64 MiB.
    pub(crate) fn journal(&self) -> Journal<'_, S> {
        let limit = (64usize << 20).max(
            self.cfg
                .max_body
                .saturating_add(MAX_META_BYTES)
                .saturating_mul(2),
        );
        Journal::new(self, limit)
    }

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
/// removal of a member goes through here: resources, service records, expired subscriptions.
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
/// membership as they were, validators included (records are put back with
/// [`Store::restore`]). What a journal holds to put things back is bounded (`limit` bytes); a
/// change that would need more is refused, and one that knows its whole extent up front
/// ([`Journal::stage`]) is refused before it changes anything.
pub(crate) struct Journal<'a, S: Store + 'static> {
    state: &'a LwsState<S>,
    undo: Vec<Undo>,
    staged: std::collections::HashMap<String, Option<(Bytes, StoredMeta)>>,
    held: usize,
    limit: usize,
}

type StoredMeta = crate::store::ResourceMeta;

/// Whether a store step with this outcome may have changed something: it succeeded, or failed in
/// the backend, where the change may have landed before the failure was reported.
fn may_have_happened<T>(outcome: &Result<T, crate::error::ServerError>) -> bool {
    matches!(outcome, Ok(_) | Err(crate::error::ServerError::Storage(_)))
}

/// How to put back one change a [`Journal`] made.
enum Undo {
    /// `key`, held under the lock of `lock`, as it was (its bytes and record, or absent). `left`
    /// is what the change may have put there instead.
    Restore {
        lock: String,
        key: String,
        prior: Option<(Bytes, StoredMeta)>,
        left: Left,
    },
    /// A removed member, recreated in its container as it was.
    Recreate {
        iri: String,
        parent: Option<String>,
        body: Bytes,
        meta: StoredMeta,
    },
}

/// What a change may have left at a key: the record it wrote (known when the store confirmed
/// the write: its blob key is minted for that write alone), bytes it may have written (when the
/// store's reply was lost), or nothing (a removal).
enum Left {
    Written(StoredMeta),
    Maybe(Bytes),
    Removed,
}

impl Undo {
    /// The resource whose lock covers this change.
    fn lock(&self) -> &str {
        match self {
            Undo::Restore { lock, .. } => lock,
            Undo::Recreate { iri, .. } => iri,
        }
    }

    /// Put this change back, with the mutation's locks still held.
    async fn apply<S: Store>(&self, store: &S) -> Result<(), crate::error::ServerError> {
        match self {
            Undo::Restore {
                key,
                prior: Some((body, meta)),
                ..
            } => store.restore(key, None, body.clone(), meta).await.map(drop),
            Undo::Restore {
                key, prior: None, ..
            } => match store.delete(key, None).await {
                Ok(()) | Err(crate::error::ServerError::NotFound) => Ok(()),
                Err(e) => Err(e),
            },
            Undo::Recreate {
                iri,
                parent,
                body,
                meta,
            } => {
                if store.exists(iri).await? {
                    return Ok(());
                }
                store
                    .restore(iri, parent.as_deref(), body.clone(), meta)
                    .await
                    .map(drop)
            }
        }
    }

    /// Put this change back after the mutation's locks were released, under the lock of what it
    /// changed, and only if what is there is still what the mutation left: a later write or
    /// create stands, even one with the same bytes. A record the store confirmed is recognized
    /// by its blob key (minted per write); one whose reply was lost, by its bytes and a
    /// modification time no later than `fence`, when the mutation still held its locks.
    async fn recover<S: Store + 'static>(
        &self,
        state: &LwsState<S>,
        fence: std::time::SystemTime,
    ) -> Result<(), crate::error::ServerError> {
        let _guard = state.locks.lock(self.lock()).await;
        if let Undo::Restore {
            key, prior, left, ..
        } = self
        {
            let now = match state.store.read(key).await {
                Ok(r) => Some(r),
                Err(crate::error::ServerError::NotFound) => None,
                Err(e) => return Err(e),
            };
            // Already as it was, validators included.
            let as_was = match (&now, prior) {
                (None, None) => true,
                (Some(r), Some((body, meta))) => {
                    r.body == body
                        && r.meta.etag == meta.etag
                        && r.meta.last_modified == meta.last_modified
                }
                _ => false,
            };
            let as_left = match (&now, left) {
                (None, Left::Removed) => true,
                (Some(r), Left::Written(m)) => r.meta.blob_key == m.blob_key,
                (Some(r), Left::Maybe(body)) => {
                    r.body == body && r.meta.last_modified.is_some_and(|t| t <= fence)
                }
                _ => false,
            };
            if as_was || !as_left {
                return Ok(());
            }
        }
        self.apply(&state.store).await
    }
}

/// How long, at most, a change that could not be put back waits between attempts.
const RECOVER_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

impl<'a, S: Store + 'static> Journal<'a, S> {
    pub(crate) fn new(state: &'a LwsState<S>, limit: usize) -> Self {
        Self {
            state,
            undo: Vec::new(),
            staged: std::collections::HashMap::new(),
            held: 0,
            limit,
        }
    }

    /// Read what is stored at `key` now, to put back later, and charge it to the journal's
    /// bound: a change staged whole before its first step is refused, if too large, before it
    /// changes anything.
    pub(crate) async fn stage(&mut self, key: &str) -> Result<(), crate::error::ServerError> {
        if self.staged.contains_key(key) {
            return Ok(());
        }
        let prior = match self.state.store.read(key).await {
            Ok(r) => Some((r.body, r.meta)),
            Err(crate::error::ServerError::NotFound) => None,
            Err(e) => return Err(e),
        };
        self.held = self
            .held
            .saturating_add(key.len() + prior.as_ref().map_or(0, |(b, _)| b.len()));
        if self.held > self.limit {
            return Err(crate::error::ServerError::Conflict(
                "the change is too large to make atomically".into(),
            ));
        }
        self.staged.insert(key.to_string(), prior);
        Ok(())
    }

    /// What was stored at `key` before this step (staged now if it was not already).
    async fn prior(
        &mut self,
        key: &str,
    ) -> Result<Option<(Bytes, StoredMeta)>, crate::error::ServerError> {
        self.stage(key).await?;
        Ok(self.staged.remove(key).flatten())
    }

    /// Write `body` at `key`, a resource or a record under the lock of `lock`.
    async fn write_under(
        &mut self,
        lock: &str,
        key: &str,
        body: Bytes,
        content_type: &str,
    ) -> Result<StoredMeta, crate::error::ServerError> {
        let prior = self.prior(key).await?;
        let written = self
            .state
            .store
            .write(key, body.clone(), content_type)
            .await;
        // A backend failure may follow a write that landed (a lost reply): it is put back all the
        // same. A refusal wrote nothing, and there is nothing to put back.
        if may_have_happened(&written) {
            self.undo.push(Undo::Restore {
                lock: lock.to_string(),
                key: key.to_string(),
                prior,
                left: match &written {
                    Ok(meta) => Left::Written(meta.clone()),
                    Err(_) => Left::Maybe(body),
                },
            });
        }
        written
    }

    /// Write `body` at the resource `iri`.
    pub(crate) async fn write(
        &mut self,
        iri: &str,
        body: Bytes,
        content_type: &str,
    ) -> Result<StoredMeta, crate::error::ServerError> {
        self.write_under(iri, iri, body, content_type).await
    }

    /// Write the metadata of `iri`.
    pub(crate) async fn write_meta(
        &mut self,
        iri: &str,
        meta: &ResourceMeta,
    ) -> Result<(), crate::error::ServerError> {
        let body = encode_meta(meta)?;
        self.write_under(iri, &meta_key(iri), Bytes::from(body), JSON)
            .await
            .map(|_| ())
    }

    /// Remove the metadata of `iri`; absent is fine.
    pub(crate) async fn delete_meta(&mut self, iri: &str) -> Result<(), crate::error::ServerError> {
        let key = meta_key(iri);
        let prior = self.prior(&key).await?;
        if prior.is_none() {
            return Ok(());
        }
        let deleted = self.state.store.delete(&key, None).await;
        if may_have_happened(&deleted) {
            self.undo.push(Undo::Restore {
                lock: iri.to_string(),
                key,
                prior,
                left: Left::Removed,
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
        let Some((body, meta)) = self.prior(iri).await? else {
            return Ok(crate::store::DeleteOutcome::NotFound);
        };
        let outcome = remove_member(&self.state.store, iri, parent).await;
        // Only a member that went, or may have, is recreated.
        if matches!(
            outcome,
            Ok(crate::store::DeleteOutcome::Deleted) | Err(crate::error::ServerError::Storage(_))
        ) {
            self.undo.push(Undo::Recreate {
                iri: iri.to_string(),
                parent: parent.map(str::to_string),
                body,
                meta,
            });
        }
        outcome
    }

    /// Keep every change.
    pub(crate) fn commit(self) {}

    /// Put back every change, the last first, with the mutation's locks still held. From the
    /// first step that fails, what is left is not dropped: it is kept, in order and with what it
    /// needs, and retried in the background (waits growing to [`RECOVER_MAX_WAIT`]) until each
    /// change is put back or something later replaced it ([`Undo::recover`]). The error says the
    /// mutation was not undone now.
    pub(crate) async fn rollback(self) -> Result<(), crate::error::ServerError> {
        let mut left = Vec::new();
        let mut failure = None;
        for undo in self.undo.into_iter().rev() {
            // What follows a failure waits behind it: a member is not recreated before its
            // container is.
            if !left.is_empty() {
                left.push(undo);
                continue;
            }
            if let Err(e) = undo.apply(&self.state.store).await {
                failure = Some(e);
                left.push(undo);
            }
        }
        let Some(failure) = failure else {
            return Ok(());
        };
        let state = self.state.clone();
        // Still under the mutation's locks: nothing written after this is the mutation's.
        let fence = crate::clock::now();
        tokio::spawn(async move {
            let mut wait = std::time::Duration::from_millis(100);
            let mut left = std::collections::VecDeque::from(left);
            while let Some(undo) = left.front() {
                match undo.recover(&state, fence).await {
                    Ok(()) => {
                        left.pop_front();
                    }
                    Err(_) => {
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(RECOVER_MAX_WAIT);
                    }
                }
            }
        });
        Err(failure)
    }
}

/// Delete the stored record `iri` of a service container (a grant, a request, a subscription).
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

/// Store a new record (an access grant or request, a subscription) at `iri` in `container` and,
/// once it is stored, put it in force in memory with `register`. Every record in the store is
/// loaded, and so in force, at the next boot, so the two never part:
///
/// - The writes and the registration run in a task of their own, holding the request's share of
///   its admission permit (`admission`): a client that goes away cannot stop a stored record from
///   being registered.
/// - A refusal stores nothing. A backend failure may follow a create that committed (a remote
///   store's timeout or lost reply), so the record is removed. When that fails too, the outcome
///   is resolved in the background ([`resolve_record`]): the record is registered only once it is
///   seen stored, and is never in force before. Until then `register` (and the quota place it
///   holds) is kept.
pub(crate) async fn create_record<S, F>(
    state: &LwsState<S>,
    container: &str,
    iri: &str,
    body: Bytes,
    admission: Option<crate::overload::AdmissionSlot>,
    held: Option<resources::IriGuard>,
    register: F,
) -> Result<(), crate::error::ServerError>
where
    S: Store + 'static,
    F: FnOnce() + Send + 'static,
{
    use crate::error::ServerError;
    let (state, container, iri) = (state.clone(), container.to_string(), iri.to_string());
    let task = async move {
        let _admission = admission;
        // The container's listing changes once the record is registered: a create holds the
        // container (shared, unless the caller holds it exclusively) until then, so a conditional
        // create sees no member arrive between its check and its own registration.
        let _shared = match held {
            None => Some(state.locks.read(&container).await),
            Some(_) => None,
        };
        let _held = held;
        match state
            .store
            .create_in_container(&container, &iri, body, LWS_JSON)
            .await
        {
            Ok(_) => {
                register();
                Ok(())
            }
            Err(e) => {
                if matches!(e, ServerError::Storage(_))
                    && delete_record(&state, &iri, &container).await.is_err()
                {
                    tokio::spawn(resolve_record(state.clone(), iri.clone(), register));
                }
                Err(e)
            }
        }
    };
    tokio::spawn(task)
        .await
        .unwrap_or_else(|e| Err(ServerError::Storage(format!("the create failed: {e}"))))
}

/// How many times, and how far apart at most, an uncertain create is looked up again.
const RESOLVE_ATTEMPTS: u32 = 12;
const RESOLVE_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Settle a create whose outcome is unknown (the store failed, and so did removing what it may
/// have stored): look the record up again, with growing waits, until the store answers. A record
/// that is there is registered (it would be loaded at the next boot anyway); one that is not is
/// dropped with `register`, which gives back its quota place. If the store never answers, the
/// next boot loads whatever is stored.
async fn resolve_record<S, F>(state: LwsState<S>, iri: String, register: F)
where
    S: Store + 'static,
    F: FnOnce() + Send + 'static,
{
    let mut wait = std::time::Duration::from_millis(100);
    for _ in 0..RESOLVE_ATTEMPTS {
        tokio::time::sleep(wait).await;
        match state.store.exists(&iri).await {
            Ok(true) => return register(),
            Ok(false) => return,
            Err(_) => wait = (wait * 2).min(RESOLVE_MAX_WAIT),
        }
    }
}

/// The preconditions of a create in a service container (grants, requests, subscriptions),
/// evaluated against `listing` with the container held exclusively: `Ok(None)` when the create is
/// unconditional, `Ok(Some(guard))` when its preconditions hold (the guard is passed on to
/// [`create_record`], so no member arrives or leaves until the new one is registered), and the
/// response to send otherwise. A listing that cannot be produced (a representation the client does
/// not accept, a page that does not exist) is that response: its absent `ETag` is not the absence
/// of the container.
pub(crate) async fn service_preconditions<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    container: &str,
    listing: impl FnOnce(&LwsRequest) -> axum::response::Response,
) -> Result<Option<resources::IriGuard>, axum::response::Response> {
    if !resources::is_conditional(req) {
        return Ok(None);
    }
    let guard = state.locks.lock(container).await;
    let current = listing(&resources::plain_get(req));
    if !current.status().is_success() {
        return Err(current);
    }
    let (etag, modified) = resources::validators_of(&current);
    match resources::unless_preconditions(req, etag.as_deref(), modified) {
        Some(refused) => Err(refused),
        None => Ok(Some(guard)),
    }
}

/// Parse a JSON body that anyone authenticated may send for a record of their own (an access
/// request, a subscription), refusing it with 413 before any parsing when it is over `limit`
/// bytes: what such a record costs is bounded by its size, never by what parsing it builds.
#[allow(clippy::result_large_err)]
pub(crate) fn bounded_json(body: &[u8], limit: usize, what: &str) -> Result<Value, Response> {
    if body.len() > limit {
        return Err(problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            Some(&format!("{what} is at most {limit} bytes")),
        ));
    }
    serde_json::from_slice(body)
        .map_err(|_| problem(StatusCode::BAD_REQUEST, Some("the body is not JSON")))
}

/// The share of the store that records anyone authenticated may create (access requests,
/// subscriptions) can take: at most `total` of them, and `per_author` by one agent. A create
/// [`Quota::reserve`]s its place before any storage work, under the same lock as every other
/// reservation, and holds the [`QuotaSlot`] until its record is registered (or the create
/// fails), so concurrent creates cannot both take the last place.
pub(crate) struct Quota {
    total: usize,
    per_author: usize,
    in_flight: Arc<std::sync::Mutex<QuotaCounts>>,
}

#[derive(Default)]
struct QuotaCounts {
    total: usize,
    by_author: std::collections::HashMap<Option<String>, usize>,
}

/// Why [`Quota::reserve`] refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum QuotaFull {
    Author,
    Total,
}

impl QuotaFull {
    pub(crate) fn response(&self, what: &str) -> Response {
        match self {
            QuotaFull::Author => problem(
                StatusCode::TOO_MANY_REQUESTS,
                Some(&format!("this agent holds as many {what} as it may")),
            ),
            QuotaFull::Total => problem(
                StatusCode::INSUFFICIENT_STORAGE,
                Some(&format!("the server holds as many {what} as it may")),
            ),
        }
    }
}

/// A place reserved by [`Quota::reserve`]; dropping it gives the place back. Drop it once the
/// record it was for is registered: from then on the record itself is counted.
pub(crate) struct QuotaSlot {
    in_flight: Arc<std::sync::Mutex<QuotaCounts>>,
    author: Option<String>,
}

impl Drop for QuotaSlot {
    fn drop(&mut self) {
        let mut c = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        c.total -= 1;
        if let Some(n) = c.by_author.get_mut(&self.author) {
            *n -= 1;
            if *n == 0 {
                c.by_author.remove(&self.author);
            }
        }
    }
}

impl Quota {
    pub(crate) fn new(total: usize, per_author: usize) -> Self {
        Self {
            total,
            per_author,
            in_flight: Default::default(),
        }
    }

    /// Reserve a place for a record by `author`. `registered` counts the records already in
    /// force, overall and by `author`; it is called under the reservation lock, so a record
    /// registered concurrently is counted either there or as a reservation still held, never
    /// neither (a registration inserts its record before it drops its slot).
    pub(crate) fn reserve(
        &self,
        author: Option<&str>,
        registered: impl FnOnce() -> (usize, usize),
    ) -> Result<QuotaSlot, QuotaFull> {
        let mut c = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        let (all, mine) = registered();
        let author = author.map(str::to_string);
        let mine = mine + c.by_author.get(&author).copied().unwrap_or(0);
        if mine >= self.per_author {
            return Err(QuotaFull::Author);
        }
        if all + c.total >= self.total {
            return Err(QuotaFull::Total);
        }
        c.total += 1;
        *c.by_author.entry(author.clone()).or_default() += 1;
        Ok(QuotaSlot {
            in_flight: self.in_flight.clone(),
            author,
        })
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

/// The largest body a request to `path` may carry: the service routes that anyone may send to
/// (access requests, subscriptions, type search) are held to their own limits while the body is
/// read, so no more than that is ever buffered for them; everything else to `max_body`.
fn body_limit(path: &str, max_body: usize) -> usize {
    let limit = if path.starts_with(REQUESTS_PATH) {
        access::MAX_REQUEST_BYTES
    } else if path.starts_with(SUBSCRIPTIONS_PATH) {
        notify::MAX_SUBSCRIPTION_BYTES
    } else if path == TYPE_SEARCH_PATH {
        index::MAX_FILTER_BYTES
    } else if path.starts_with(GRANTS_PATH) {
        access::MAX_GRANT_BYTES
    } else {
        max_body
    };
    limit.min(max_body)
}

async fn dispatch<S: Store + 'static>(State(state): State<LwsState<S>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    if let Err(refused) = check_headers(&parts.headers) {
        return refused;
    }
    let limit = body_limit(parts.uri.path(), state.cfg.max_body);
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
    // The tag covers everything the page shows: the members it lists, as listed, and how many
    // there are in all (which its paging links follow), so a member that leaves without a
    // version change (an expiry) still changes it.
    let (page_tag, total_tag) = (page.to_string(), total.to_string());
    let listed: Vec<String> = shown.iter().map(Value::to_string).collect();
    let etag = etag_of(
        [version, page_tag.as_str(), total_tag.as_str()]
            .into_iter()
            .chain(listed.iter().map(String::as_str)),
    );
    let refusal = resources::read_refusal(req, &etag);
    if refusal == Some(StatusCode::PRECONDITION_FAILED) {
        return problem(StatusCode::PRECONDITION_FAILED, None);
    }
    let mut resp = if let Some(status) = refusal {
        status.into_response()
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
    let etag = etag_of([anchor]);
    // Read as any representation is, under the request's preconditions.
    let mut resp = match resources::read_refusal(req, &etag) {
        Some(StatusCode::NOT_MODIFIED) => StatusCode::NOT_MODIFIED.into_response(),
        Some(refused) => problem(refused, None),
        None => json_response(StatusCode::OK, LINKSET_JSON, &doc),
    };
    set(resp.headers_mut(), header::ETAG, &etag);
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

/// Whether `v` is an absolute IRI (RFC 3987): a scheme, a colon and something after it, with
/// every character and percent-encoding the grammar allows. One parser checks every IRI a client
/// sends, so `%ZZ`, a space or a bare fragment is refused everywhere alike.
pub fn is_uri(v: &str) -> bool {
    v.split_once(':').is_some_and(|(_, rest)| !rest.is_empty()) && oxiri::Iri::parse(v).is_ok()
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
        /// `list_children` of this container alone fails with a backend error.
        pub fail_list_of: Arc<std::sync::Mutex<Option<String>>>,
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
        /// The next `write` of this IRI commits, then reports a backend failure, as when a remote
        /// store's reply is lost after the update landed.
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
        /// Fail every [`Store::restore`] of this resource until cleared.
        pub fail_restore_of: Arc<std::sync::Mutex<Option<String>>>,
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
                fail_list_of: Default::default(),
                hide: Default::default(),
                fail_after_write_of: Default::default(),
                refuse_write_of: Default::default(),
                fail_after_create: Default::default(),
                partial_delete_of: Default::default(),
                two_step_deletes: Default::default(),
                fail_restore_of: Default::default(),
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
            let lost = {
                let mut slot = self.fail_after_write_of.lock().unwrap();
                let lost = slot.as_deref() == Some(iri);
                if lost {
                    *slot = None;
                }
                lost
            };
            if lost {
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
        async fn restore(
            &self,
            iri: &str,
            container: Option<&str>,
            body: Bytes,
            meta: &ResourceMeta,
        ) -> ServerResult<ResourceMeta> {
            // The same failures as `write`: a record is put back by either.
            self.step()?;
            if self.fail_restore_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if self.fail_write_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            if self.refuse_write_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::InsufficientStorage);
            }
            self.inner.restore(iri, container, body, meta).await
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
            if self.fail_list_of.lock().unwrap().as_deref() == Some(container) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.list_children(container).await
        }
    }

    /// What can be seen of `resources` (each one's content, content type and stored metadata) and
    /// of `listings` (each container's members).
    pub async fn snapshot(
        st: &super::LwsState<FlakyStore>,
        store: &FlakyStore,
        resources: &[String],
        listings: &[String],
    ) -> Vec<String> {
        let mut out = Vec::new();
        for iri in resources {
            for key in [iri.clone(), super::meta_key(iri)] {
                out.push(match store.inner.read(&key).await {
                    Ok(r) => format!(
                        "{key}: {} {:?} {} {:?}",
                        r.meta.content_type, r.body, r.meta.etag, r.meta.last_modified
                    ),
                    Err(e) => format!("{key}: {e}"),
                });
            }
            // And what a client is served: status, validators, links and representation.
            let path = iri
                .strip_prefix(st.cfg.storage().trim_end_matches('/'))
                .unwrap_or(iri);
            let resp = super::resources::handle(
                st,
                &request(
                    axum::http::Method::GET,
                    path,
                    &[("accept", "text/turtle")],
                    "",
                ),
                &crate::lws::Agent::anonymous(),
            )
            .await;
            let mut seen = format!("GET {path}: {}", resp.status());
            for h in ["etag", "last-modified", "link"] {
                for v in resp.headers().get_all(h) {
                    seen.push_str(&format!(" {h}={v:?}"));
                }
            }
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap_or_default();
            seen.push_str(&format!(" {body:?}"));
            out.push(seen);
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
            let before = snapshot(&st, &store, &resources, &listings).await;
            let after_st = st.clone();
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
                    snapshot(&after_st, &store, &resources, &listings).await,
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

    /// Review finding: request caps were checked against what was registered, and the record
    /// registered after an asynchronous create, so concurrent creates all passed. Places are
    /// reserved under one lock and held until registration.
    #[test]
    fn quota_places_are_reserved_until_released() {
        let q = Quota::new(3, 2);
        let a = q.reserve(Some("a"), || (0, 0)).unwrap();
        let _a2 = q.reserve(Some("a"), || (0, 0)).unwrap();
        assert_eq!(
            q.reserve(Some("a"), || (0, 0)).err(),
            Some(QuotaFull::Author)
        );
        let _b = q.reserve(Some("b"), || (0, 0)).unwrap();
        assert_eq!(
            q.reserve(Some("c"), || (0, 0)).err(),
            Some(QuotaFull::Total)
        );
        // Registered records count too.
        drop(a);
        assert_eq!(
            q.reserve(Some("a"), || (1, 1)).err(),
            Some(QuotaFull::Author)
        );
        assert!(q.reserve(Some("c"), || (0, 0)).is_ok());
    }

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

    /// Review finding: every body was buffered up to the server-wide ceiling before any route
    /// limit applied, so an anonymous request to a service route anyone may post to buffered
    /// megabytes before its 16 KiB limit refused it. Each route's limit applies while the body
    /// is read, and no route can raise the ceiling.
    #[tokio::test]
    async fn every_route_reads_no_more_than_its_limit() {
        use tower::ServiceExt;
        let max = 1 << 20;
        let app = open_router(max).await;
        let send = |method: &'static str, path: &'static str, len: usize| {
            let app = app.clone();
            async move {
                let req = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", LWS_JSON)
                    .body(Body::from(vec![b' '; len]))
                    .unwrap();
                app.oneshot(req).await.unwrap().status()
            }
        };
        let routes = [
            ("POST", REQUESTS_PATH, access::MAX_REQUEST_BYTES),
            ("POST", SUBSCRIPTIONS_PATH, notify::MAX_SUBSCRIPTION_BYTES),
            ("QUERY", TYPE_SEARCH_PATH, index::MAX_FILTER_BYTES),
            ("POST", GRANTS_PATH, access::MAX_GRANT_BYTES),
            ("POST", "/", max),
            ("PUT", "/x", max),
            ("PATCH", "/x", max),
        ];
        for (method, path, limit) in routes {
            assert!(limit <= max);
            assert_eq!(
                send(method, path, limit + 1).await,
                StatusCode::PAYLOAD_TOO_LARGE,
                "{method} {path}"
            );
            assert_ne!(
                send(method, path, limit).await,
                StatusCode::PAYLOAD_TOO_LARGE,
                "{method} {path}"
            );
        }
        assert_eq!(body_limit("/", 10), 10);
        assert_eq!(body_limit(REQUESTS_PATH, 10), 10);
    }

    /// Review finding: a member's record and its parent's membership edge were removed in two
    /// steps on some paths (service records, expired subscriptions), so a failure in between left
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
        // A grant, an access request and a subscription, each created and removed.
        let storage = state.cfg.storage();
        let access = |kind: &str| {
            json!({
                "@context": ["https://www.w3.org/ns/lws/v1"],
                "type": [kind],
                "storage": storage,
                "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": "https://a/"}],
            })
            .to_string()
        };
        let sub =
            json!({"type": notify::WEBHOOK, "topic": [storage], "inbox": "https://inbox.example/"})
                .to_string();
        for (path, body) in [
            (GRANTS_PATH, access("AccessGrant")),
            (REQUESTS_PATH, access("AccessRequest")),
            (SUBSCRIPTIONS_PATH, sub.clone()),
        ] {
            let r = send(Method::POST, path.into(), LWS_JSON, body).await;
            assert_eq!(r.status(), StatusCode::CREATED, "{path}");
            let at = local(r.headers()[header::LOCATION].to_str().unwrap());
            let r = send(Method::DELETE, at, LWS_JSON, String::new()).await;
            assert!(r.status().is_success(), "{path}: {}", r.status());
        }
        // A subscription that has expired by the next boot is removed then.
        let r = send(Method::POST, SUBSCRIPTIONS_PATH.into(), LWS_JSON, sub).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let at = r.headers()[header::LOCATION].to_str().unwrap().to_string();
        let mut stored: notify::Subscription =
            serde_json::from_slice(&state.store.read(&at).await.unwrap().body).unwrap();
        stored.expires_at = Some(1);
        state
            .store
            .write(&at, serde_json::to_vec(&stored).unwrap().into(), LWS_JSON)
            .await
            .unwrap();
        notify::Notifier::load(&state.store, &state.cfg)
            .await
            .unwrap();
        assert!(!state.store.exists(&at).await.unwrap());
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

    /// Review finding: cross-cutting rules were applied per handler, and some routes missed
    /// them. Every route is walked: each that takes a body refuses a coded one, and each that
    /// serves a representation answers its preconditions.
    #[tokio::test]
    async fn every_route_refuses_codings_and_answers_preconditions() {
        let (st, _store) = test_store::state(4).await;
        let bodied = [
            AS_TOKEN_PATH,
            SUBSCRIPTIONS_PATH,
            TYPE_INDEX_PATH,
            TYPE_SEARCH_PATH,
            GRANTS_PATH,
            REQUESTS_PATH,
            "/",
            "/x",
            "/.lws/elsewhere",
        ];
        for path in bodied {
            for method in ["POST", "PUT", "PATCH", "QUERY"] {
                let m = Method::from_bytes(method.as_bytes()).unwrap();
                let h = [
                    ("content-type", "application/json"),
                    ("content-encoding", "gzip"),
                ];
                let r = route(&st, request(m, path, &h, "{}")).await;
                assert_eq!(
                    r.status(),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "{method} {path}"
                );
                assert_eq!(r.headers()["accept-encoding"], "identity");
            }
        }
        for path in [
            "/",
            TYPE_INDEX_PATH,
            GRANTS_PATH,
            REQUESTS_PATH,
            SUBSCRIPTIONS_PATH,
        ] {
            let r = route(&st, request(Method::GET, path, &[], "")).await;
            assert_eq!(r.status(), StatusCode::OK, "GET {path}");
            let tag = r.headers()["etag"].to_str().unwrap().to_string();
            for (h, want) in [
                ("if-none-match", StatusCode::NOT_MODIFIED),
                ("if-match", StatusCode::OK),
            ] {
                let r = route(&st, request(Method::GET, path, &[(h, &tag)], "")).await;
                assert_eq!(r.status(), want, "GET {path} {h}");
            }
            let r = route(
                &st,
                request(Method::GET, path, &[("if-match", "\"other\"")], ""),
            )
            .await;
            assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED, "GET {path}");
        }
    }

    /// Review finding: recovery compared bytes only, so a later write of the same bytes (or
    /// one that came back to them) could be overwritten by the old undo. A confirmed write is
    /// recognized by its blob key, which no later write shares.
    #[tokio::test]
    async fn recovery_never_overwrites_a_later_write() {
        let (st, _store) = test_store::state(4).await;
        let key = format!("{}r", st.cfg.storage());
        let old = st
            .store
            .write(&key, "old".into(), "text/plain")
            .await
            .unwrap();
        let ours = st
            .store
            .write(&key, "new".into(), "text/plain")
            .await
            .unwrap();
        let undo = |left| Undo::Restore {
            lock: key.clone(),
            key: key.clone(),
            prior: Some((Bytes::from("old"), old.clone())),
            left,
        };
        let fence = crate::clock::now();
        // Someone else writes the same bytes afterwards: theirs stands.
        st.store
            .write(&key, "new".into(), "text/plain")
            .await
            .unwrap();
        undo(Left::Written(ours.clone()))
            .recover(&st, fence)
            .await
            .unwrap();
        assert_eq!(st.store.read(&key).await.unwrap().body, Bytes::from("new"));
        // A reply that was lost: bytes written after the fence are not the mutation's.
        undo(Left::Maybe(Bytes::from("new")))
            .recover(&st, old.last_modified.unwrap())
            .await
            .unwrap();
        assert_eq!(st.store.read(&key).await.unwrap().body, Bytes::from("new"));
        // What the mutation left is put back, validators and all.
        let left = st.store.meta(&key).await.unwrap().unwrap();
        undo(Left::Written(left)).recover(&st, fence).await.unwrap();
        let back = st.store.read(&key).await.unwrap();
        assert_eq!(back.body, Bytes::from("old"));
        assert_eq!(
            (back.meta.etag, back.meta.last_modified),
            (old.etag, old.last_modified)
        );
    }
}
