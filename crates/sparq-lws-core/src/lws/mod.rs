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
//!   validators, single byte ranges, JSON Patch, `Depth: infinity` deletes, and RFC 9457 problem
//!   details;
//! - a **linkset** (RFC 9264) per resource, edited with JSON Patch;
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
mod intents;
pub use sparq_lws_authn::jose;
pub mod notify;
mod records;
pub mod resources;
pub mod tokens;
pub mod tracked;

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

pub use sparq_lws_authn::{CID_CONTEXT, LWS_CONTEXT, LWS_NS};
pub const AS_CONTEXT: &str = "https://www.w3.org/ns/activitystreams";
pub const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";

pub const LWS_JSON: &str = "application/lws+json";
pub const LWS_CID: &str = "application/lws+cid";
pub const LD_JSON: &str = "application/ld+json";
pub const JSON: &str = "application/json";
pub const LINKSET_JSON: &str = "application/linkset+json";
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
    /// Largest request body read, in bytes; a larger one is refused with 413 before it is
    /// buffered further. The server-wide ceiling (`SOLID_SERVER_MAX_BODY_BYTES`, see
    /// [`crate::body_limit`]), the same one the Solid surface enforces.
    pub max_body: usize,
    /// Bounds on outgoing webhook deliveries (see [`notify::DeliveryLimits`]).
    pub delivery: notify::DeliveryLimits,
    /// Start in the notification service's recovery mode: [`LwsState::new`] removes every stored
    /// subscription with an expiry and then stores the clock as the notification high water,
    /// before anything is served; it fails, the high water unchanged, while any of them is still
    /// stored. For a high water stored while the clock had jumped forward, once the clock is
    /// corrected; set for one start only (it removes those subscriptions at every start).
    pub reset_subscription_clock: bool,
    /// The authentication suites the authorization server offers (see
    /// [`LwsConfig::register_suite`]).
    suites: sparq_lws_authn::Suites,
}

/// The environment variable naming the authentication suites to offer (see
/// [`LwsConfig::from_env`]).
pub const AUTHN_VAR: &str = "SOLID_SERVER_LWS_AUTHN";

/// Every authentication suite a build can hold, by name, each with its feature.
pub const SUITES: &[(&str, &str)] = &[
    ("ssi", "authn-ssi"),
    ("openid", "authn-openid"),
    ("saml", "authn-saml"),
];

/// The names of the authentication suites this build holds.
pub fn built_in_suites() -> Vec<&'static str> {
    [
        cfg!(feature = "authn-ssi"),
        cfg!(feature = "authn-openid"),
        cfg!(feature = "authn-saml"),
    ]
    .into_iter()
    .zip(SUITES)
    .filter_map(|(built, (name, _))| built.then_some(*name))
    .collect()
}

/// The suites [`AUTHN_VAR`] names (`value`, comma-separated), or `None` for every suite built
/// in. A name that is no suite, or one this build does not hold, is an error.
fn chosen_suites(value: Option<&str>) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let built = built_in_suites();
    let mut chosen = Vec::new();
    for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let Some((_, feature)) = SUITES.iter().find(|(n, _)| *n == name) else {
            let known: Vec<&str> = SUITES.iter().map(|(n, _)| *n).collect();
            return Err(format!(
                "{AUTHN_VAR} names {name}, which is no authentication suite (one of {})",
                known.join(", ")
            ));
        };
        if !built.contains(&name) {
            return Err(format!(
                "{AUTHN_VAR} names {name}, but this server was built without it (the {feature} \
                 feature)"
            ));
        }
        if !chosen.iter().any(|c| c == name) {
            chosen.push(name.to_string());
        }
    }
    Ok(Some(chosen))
}

/// The suites to offer, by name, under the environment `env` reads: those [`AUTHN_VAR`] names
/// (every one built in when it is unset), and SAML only with its identity providers. A suite's
/// configuration set while the suite is not offered is an error, never silently ignored.
fn offered_suites(env: &dyn Fn(&str) -> Option<String>) -> Result<Vec<&'static str>, String> {
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    let chosen = chosen_suites(set(AUTHN_VAR).as_deref())?;
    let built = built_in_suites();
    let named = |name: &str| chosen.as_ref().is_none_or(|c| c.iter().any(|n| n == name));
    for (name, var) in [
        ("openid", "SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS"),
        ("saml", "SOLID_SERVER_LWS_SAML_IDPS_FILE"),
    ] {
        if set(var).is_none() {
            continue;
        }
        if !built.contains(&name) {
            let (_, feature) = SUITES.iter().find(|(n, _)| *n == name).expect("known");
            return Err(format!(
                "{var} is set, but this server was built without the {name} suite (the {feature} \
                 feature)"
            ));
        }
        if !named(name) {
            return Err(format!(
                "{var} is set, but {AUTHN_VAR} leaves the {name} suite out"
            ));
        }
    }
    let saml_idps = set("SOLID_SERVER_LWS_SAML_IDPS_FILE").is_some();
    if chosen.is_some() && named("saml") && !saml_idps {
        return Err(format!(
            "{AUTHN_VAR} names saml, but SOLID_SERVER_LWS_SAML_IDPS_FILE is not set"
        ));
    }
    Ok(built
        .into_iter()
        .filter(|name| named(name) && (*name != "saml" || saml_idps))
        .collect())
}

/// The suites a configuration offers before any is configured: those built in that need no
/// configuration (did:key and controlled identifiers; OpenID Connect, with no trusted
/// providers).
fn default_suites() -> sparq_lws_authn::Suites {
    #[allow(unused_mut)]
    let mut suites = sparq_lws_authn::Suites::default();
    #[cfg(feature = "authn-ssi")]
    suites
        .register(std::sync::Arc::new(sparq_lws_authn_ssi::Ssi))
        .expect("the built-in suites take distinct token types");
    #[cfg(feature = "authn-openid")]
    suites
        .register(std::sync::Arc::new(
            sparq_lws_authn_openid::OpenId::default(),
        ))
        .expect("the built-in suites take distinct token types");
    suites
}

impl LwsConfig {
    /// A configuration with fresh keys and no owner, for `base_url`. It offers the authentication
    /// suites built in that need no configuration ([`LwsConfig::suites`]).
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
            reset_subscription_clock: false,
            suites: default_suites(),
        }
    }

    /// A configuration like [`LwsConfig::new`] that offers `suites` and no others, for an
    /// embedder that configures its suites itself (an OpenID Connect suite with its trusted
    /// providers, say). Refused when two suites take the same subject token type.
    pub fn with_suites(
        base_url: &str,
        suites: impl IntoIterator<Item = std::sync::Arc<dyn sparq_lws_authn::Suite>>,
    ) -> Result<Self, String> {
        let mut cfg = Self::new(base_url);
        cfg.suites = sparq_lws_authn::Suites::default();
        for suite in suites {
            cfg.register_suite(suite)?;
        }
        Ok(cfg)
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
    /// - `SOLID_SERVER_LWS_RESET_SUBSCRIPTION_CLOCK=1`: start in the notification service's
    ///   recovery mode ([`LwsConfig::reset_subscription_clock`]);
    /// - `SOLID_SERVER_LWS_AUTHN`: the authentication suites to offer, comma-separated, of
    ///   `ssi` (did:key and controlled identifiers, the default `authn-ssi` feature), `openid`
    ///   (OpenID Connect, the default `authn-openid` feature) and `saml` (SAML 2.0, the
    ///   `authn-saml` feature). Unset, every suite built in is offered (SAML when its identity
    ///   providers are set). Naming a suite the build does not hold is an error;
    /// - `SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS`: comma-separated OpenID Provider issuers whose
    ///   identities get the reserved half of the DPoP replay cache (for the OpenID Connect suite;
    ///   set when that suite is not offered, it is an error);
    /// - `SOLID_SERVER_LWS_SAML_IDPS_FILE`: a JSON object of trusted SAML identity provider entity
    ///   ids to their keys, which offers the SAML 2.0 suite (set when that suite is not offered,
    ///   it is an error; `saml` in `SOLID_SERVER_LWS_AUTHN` without it is an error too);
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
        cfg.reset_subscription_clock = flag(notify::RESET_SUBSCRIPTION_CLOCK_VAR);
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
        cfg.register_suites_from_env()?;
        Ok(cfg)
    }

    /// Offer the authentication suites the environment configures, of those built in.
    fn register_suites_from_env(&mut self) -> Result<(), String> {
        let offered = offered_suites(&|k| std::env::var(k).ok())?;
        self.suites = sparq_lws_authn::Suites::default();
        #[cfg(feature = "authn-ssi")]
        if offered.contains(&"ssi") {
            self.register_suite(std::sync::Arc::new(sparq_lws_authn_ssi::Ssi))?;
        }
        #[cfg(feature = "authn-openid")]
        if offered.contains(&"openid") {
            self.register_suite(std::sync::Arc::new(
                sparq_lws_authn_openid::OpenId::from_env(),
            ))?;
        }
        #[cfg(feature = "authn-saml")]
        if offered.contains(&"saml") {
            match sparq_lws_authn_saml::Saml::from_env()? {
                Some(saml) => self.register_suite(std::sync::Arc::new(saml))?,
                // An empty set of identity providers turns SAML off, unless it was asked for.
                None if std::env::var(AUTHN_VAR).is_ok_and(|v| !v.trim().is_empty()) => {
                    return Err(format!(
                        "{AUTHN_VAR} names saml, but SOLID_SERVER_LWS_SAML_IDPS_FILE names no \
                         identity providers"
                    ))
                }
                None => {}
            }
        }
        drop(offered);
        Ok(())
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

    /// Offer `suite` too. Refused when it takes a subject token type another offered suite takes.
    pub fn register_suite(
        &mut self,
        suite: std::sync::Arc<dyn sparq_lws_authn::Suite>,
    ) -> Result<(), String> {
        self.suites.register(suite)
    }

    /// The authentication suites the authorization server offers.
    pub fn suites(&self) -> &sparq_lws_authn::Suites {
        &self.suites
    }

    /// What the server may fetch: identity documents, provider metadata and keys, webhook
    /// inboxes.
    pub fn fetch_policy(&self) -> sparq_lws_authn::fetch::Policy {
        sparq_lws_authn::fetch::Policy {
            allow_insecure_fetch: self.allow_insecure_fetch,
        }
    }

    /// The absolute URI of a server path.
    pub fn absolute(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

/// The client the authorization server and the notification sender fetch with (see
/// [`sparq_lws_authn::fetch::fetch_client`]), for `cfg`'s fetch policy.
pub fn fetch_client(cfg: &LwsConfig) -> Result<reqwest::Client, String> {
    sparq_lws_authn::fetch::fetch_client(&cfg.fetch_policy())
}

pub use sparq_lws_authn::fetch::{is_forbidden_ip, public_addrs, PublicOnlyResolver};

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
    /// The part of `types` declared by Link headers (on the create, or with `Prefer:
    /// set-linkset`), kept apart from those the content states: replacing the content replaces
    /// only its own. `None` in metadata written before the two were kept apart, whose declared
    /// types are `types` less those its content states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_types: Option<Vec<String>>,
    /// Descriptive links the client sent as Link headers: relation to targets.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub links: BTreeMap<String, Vec<String>>,
    /// The resource's linkset document, once one is stored.
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

/// Whether `iri` names a linkset's stored record rather than a resource: no resource's name ends
/// in [`META_SUFFIX`], in any case (see `resources::sanitize_slug`), so whatever is stored under
/// such a name is never served, listed, followed or subscribed to as a resource.
pub(crate) fn names_record(iri: &str) -> bool {
    let n = META_SUFFIX.len();
    iri.len() >= n && iri.as_bytes()[iri.len() - n..].eq_ignore_ascii_case(META_SUFFIX.as_bytes())
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
    /// Held by the handle [`LwsState::new`] returns and every clone of it, never by a task the
    /// state runs for itself ([`LwsState::detached`]): once the last is dropped, the state's
    /// maintenance (the expiry sweeper, see `notify::spawn_expiry_sweeper`) stops, whatever
    /// its tasks still hold.
    owner: Option<Arc<tokio::sync::watch::Sender<()>>>,
}

impl<S: Store> Clone for LwsState<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            owner: self.owner.clone(),
        }
    }
}

pub struct Inner<S: Store> {
    /// Every change to the store moves the listing snapshots it concerns (see [`tracked`]).
    pub store: tracked::Tracked<S>,
    pub cfg: LwsConfig,
    pub access: access::AccessStore,
    pub notify: notify::Notifier,
    /// Fetches identity documents, OpenID provider metadata and JWKS.
    pub http: reqwest::Client,
    /// Serializes conditional writes per resource (see [`resources::IriLocks`]).
    /// Shared, so a task that must not keep the state alive can hold them (see
    /// `records::Settlements`).
    pub locks: Arc<resources::IriLocks>,
    /// The bytes set-aside changes hold to put back (see [`LwsState::may_write`]).
    set_aside_bytes: std::sync::atomic::AtomicUsize,
    /// The containers whose own modification time may be behind a change to them: each with
    /// whether the last touch failed, and how many touches are yet to land.
    untouched: std::sync::Mutex<std::collections::HashMap<String, (bool, usize)>>,
    /// The intents of kept changes that wait on a touch of each container (see
    /// [`LwsState::owe_touch`]).
    owed_touches: std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
    /// The record changes being made or settled (see [`records::Settlements`]).
    pub(crate) settlements: records::Settlements,
    /// Holds whose release failed, each with its holder: tried again by the maintenance
    /// sweeper (see [`LwsState::unreleased`]).
    unreleased: std::sync::Mutex<Vec<(String, StoredMeta)>>,
    /// How many failed releases were not kept for the sweeper, the wait list being full (see
    /// [`LwsState::release_later`]).
    dropped_releases: std::sync::atomic::AtomicU64,
}

/// An [`LwsState`] that does not keep it alive: what a task the state owns holds (see
/// [`records::Settlements`]), so dropping every other handle drops the state and ends its tasks.
pub(crate) struct WeakState<S: Store> {
    inner: std::sync::Weak<Inner<S>>,
}

impl<S: Store> WeakState<S> {
    /// The state, while anything else still holds it.
    pub(crate) fn upgrade(&self) -> Option<LwsState<S>> {
        self.inner
            .upgrade()
            .map(|inner| LwsState { inner, owner: None })
    }
}

impl<S: Store> LwsState<S> {
    /// A handle to the state for a task the state runs for itself: it keeps the state alive,
    /// but not its maintenance (see [`LwsState`]'s `owner`).
    pub(crate) fn detached(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            owner: None,
        }
    }

    /// A handle to the state that does not keep it alive.
    pub(crate) fn downgrade(&self) -> WeakState<S> {
        WeakState {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl<S: Store> std::ops::Deref for LwsState<S> {
    type Target = Inner<S>;
    fn deref(&self) -> &Inner<S> {
        &self.inner
    }
}

impl<S: Store + 'static> LwsState<S> {
    /// A [`Journal`] for one mutation, bounded to [`JOURNAL_LIMIT`] (see [`Journal::stage`]).
    pub(crate) fn journal(&self) -> Journal<'_, S> {
        Journal::new(self, JOURNAL_LIMIT)
    }

    /// Build the state: ensure the storage root and the service containers exist, put back any
    /// change a process stop cut short (see the `intents` module), and load the stored access
    /// grants, requests and subscriptions.
    pub async fn new(store: S, cfg: LwsConfig) -> Result<Self, String> {
        // Refused before anything is read or written: notifications keep a time the store must
        // raise atomically.
        notify::refuse_without_atomic_markers(&store)?;
        // The backend's lease, held for as long as the state lives: a second state over the same
        // store handle (a clone of this one's store, or a wrapper of it, say) is refused before
        // it reads or writes anything, so no two registries diverge over one store, and only this
        // one may lower the notification high water. The tracked store takes the lease its
        // inner store keeps, so a rewrapped store keeps the running service's.
        let store = tracked::Tracked::new(store, cfg.storage());
        let lease = store
            .lifecycle()
            .unwrap_or_default()
            .acquire()
            .ok_or("another LWS service is running over this store")?;
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
        // Grants, requests and subscriptions are loaded once changes cut short are put back (a
        // record's create or removal among them).
        let access = access::AccessStore::empty();
        let notify = notify::Notifier::new(&cfg)?;
        notify.hold(lease);
        let (owner, closed) = tokio::sync::watch::channel(());
        let state = Self {
            owner: Some(Arc::new(owner)),
            inner: Arc::new(Inner {
                store,
                cfg,
                access,
                notify,
                http,
                locks: Default::default(),
                set_aside_bytes: Default::default(),
                untouched: Default::default(),
                owed_touches: Default::default(),
                settlements: Default::default(),
                unreleased: Default::default(),
                dropped_releases: Default::default(),
            }),
        };
        // Changes a process stop cut short are put back before anything is served.
        // Nothing put back in the background starts until every step that can fail has
        // succeeded: a start that fails leaves nothing running to put an old state back over a
        // later start's writes, and grants are loaded while what is hidden stays hidden.
        let hidden = intents::recover(&state).await?;
        // The notification recovery, when asked for, runs here: before any subscription is
        // loaded and before the state is handed to anyone, so no request is in flight.
        if state.cfg.reset_subscription_clock {
            let lease = state.notify.lease().ok_or("no lease is held")?;
            let visible = |iri: &str| state.visible(iri);
            let removed =
                notify::reset_subscription_clock(&state.store, &state.cfg, visible, lease).await?;
            eprintln!(
                "lws: notification high water reset to the clock; {removed} subscriptions with an \
                 expiry removed (unset {} before the next start)",
                notify::RESET_SUBSCRIPTION_CLOCK_VAR
            );
        }
        let loaded =
            access::AccessStore::load_with(&state.store, &state.cfg, |iri| state.visible(iri))
                .await?;
        state
            .notify
            .load_with(&state.store, &state.cfg, |iri| state.visible(iri))
            .await?;
        state.access.replace(loaded);
        state.settle_hidden(hidden);
        notify::spawn_expiry_sweeper(&state, closed);
        Ok(state)
    }

    /// Whether a change to the container `uri` may be later than its stored modification time:
    /// a touch after it failed, or is yet to land.
    pub(crate) fn is_untouched(&self, uri: &str) -> bool {
        self.untouched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(uri)
    }

    /// Record that the intent `record` of a kept change is to be cleared once the container
    /// `uri` is next touched: a stop before then leaves it, and the next start touches the
    /// container (see the `intents` module).
    /// Keep `holder`'s hold on `meta`'s bytes, whose release failed, for the maintenance
    /// sweeper to release ([`LwsState::release_unreleased`]). Past [`UNRELEASED_LIMIT`] the hold
    /// is not kept: it is counted ([`LwsState::dropped_releases`]) and reported, and left to the
    /// next start's sweep ([`intents::recover`]), which fails the start rather than drop one.
    pub(crate) fn release_later(&self, holder: &str, meta: &StoredMeta) {
        let mut unreleased = self.unreleased.lock().unwrap_or_else(|e| e.into_inner());
        if unreleased.len() < UNRELEASED_LIMIT {
            unreleased.push((holder.to_string(), meta.clone()));
            return;
        }
        drop(unreleased);
        let dropped = 1 + self
            .dropped_releases
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        // Reported at the first and at each doubling, so an outage does not flood the log.
        if dropped.is_power_of_two() {
            eprintln!(
                "lws: {dropped} failed releases could not be kept to be tried again; \
                 their holds wait for the next start"
            );
        }
    }

    /// How many failed releases were not kept to be tried again (see
    /// [`LwsState::release_later`]).
    pub(crate) fn dropped_releases(&self) -> u64 {
        self.dropped_releases
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Whether a hold waits to be released again.
    pub(crate) fn has_unreleased(&self) -> bool {
        !self
            .unreleased
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Release again every hold whose release failed; one that fails again waits for the next.
    pub(crate) async fn release_unreleased(&self) {
        let waiting =
            std::mem::take(&mut *self.unreleased.lock().unwrap_or_else(|e| e.into_inner()));
        for (holder, meta) in waiting {
            if self.store.release(&meta, &holder).await.is_err() {
                self.release_later(&holder, &meta);
            }
        }
    }

    pub(crate) fn owe_touch(&self, uri: &str, record: String) {
        self.owed_touches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(uri.to_string())
            .or_default()
            .push(record);
    }

    /// The intents waiting on a touch of `uri` (see [`LwsState::owe_touch`]), taken.
    pub(crate) fn take_owed_touches(&self, uri: &str) -> Vec<String> {
        self.owed_touches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(uri)
            .unwrap_or_default()
    }

    /// Record that the container `uri`'s listing changed and a touch of it is to follow: until
    /// it lands ([`LwsState::touched`]), the container's modification time is behind.
    pub(crate) fn touching(&self, uri: &str) {
        self.changed(uri);
        let mut map = self.untouched.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(uri.to_string()).or_default().1 += 1;
    }

    /// Record that a touch of the container `uri` begun by [`LwsState::touching`] is over, and
    /// whether it landed.
    pub(crate) fn touched(&self, uri: &str, landed: bool) {
        self.changed(uri);
        let mut map = self.untouched.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(uri.to_string()).or_default();
        entry.0 = !landed;
        entry.1 = entry.1.saturating_sub(1);
        if *entry == (false, 0) {
            map.remove(uri);
        }
    }

    /// See [`tracked::Generations::generation`].
    pub(crate) fn generation(&self, uri: &str) -> u64 {
        self.store.generations().generation(uri)
    }

    /// See [`tracked::Generations::in_flight`]: a listing made meanwhile has no date.
    pub(crate) fn in_flight(&self, uri: &str) -> bool {
        self.store.generations().in_flight(uri)
    }

    fn changed(&self, uri: &str) {
        self.store.generations().changed(uri);
    }

    /// Whether `uri` may be read or acted on: not while a change to it, or to a member it lists,
    /// failed and could not yet be put back (it is set aside, see [`LwsState::set_aside`]). Every
    /// route to a resource or record checks it once it has authorized the request (so only a
    /// caller that may act on it learns it is set aside; see [`access::aside`]), and every lock
    /// is taken only while it holds ([`resources::IriLocks::lock`]), so one that is not visible
    /// is answered `503` at once, or fails a walk over many, and is never waited on.
    pub(crate) fn visible(&self, uri: &str) -> bool {
        self.locks.visible(uri)
    }

    /// Whether a new change may start: not while set-aside changes hold [`MAX_SET_ASIDE_BYTES`]
    /// or more to put back. What they hold is bounded so: by that, and by what the changes in
    /// flight (each bounded by its [`Journal`], their number by admission) could add to it.
    pub(crate) fn may_write(&self) -> bool {
        self.set_aside_bytes
            .load(std::sync::atomic::Ordering::Acquire)
            < MAX_SET_ASIDE_BYTES
    }

    /// Set aside the resources of a change that could not be put back after a few tries
    /// ([`settle`]), with the containers listing them: they are not [visible](Self::visible)
    /// from now on, and a task of their own, holding the change's `locks` (so nobody acts on
    /// them meanwhile; the change's listing locks among them) and no request's admission slot,
    /// keeps putting the change back, waiting longer between tries (up to [`UNDO_MAX_WAIT`]).
    /// Once it is back the locks go, and each resource is visible again once no other
    /// set-aside change is to it. Then the containers whose touch waited on it are touched
    /// ([`LwsState::touch_owed`]): those whose kept changes' touches waited at start, and the
    /// one a settled [`Undo::Resolve`] listed.
    pub(crate) fn set_aside<L: Send + 'static>(&self, left: Unsettled, locks: L) {
        let hidden = self.hide(&left, true);
        self.settle_aside(left, hidden, locks);
    }

    /// Set aside `left` as [`LwsState::set_aside`] does, but hide only its own resources: for a
    /// change no container lists (a create whose content was never written), so its containers
    /// stay visible and their locks need not be held.
    pub(crate) fn set_aside_unlisted<L: Send + 'static>(&self, left: Unsettled, locks: L) {
        let hidden = self.hide(&left, false);
        self.settle_aside(left, hidden, locks);
    }

    /// Set aside each of `all` ([`LwsState::set_aside`]), every one's resources hidden before
    /// any is put back: a change settled early touches the containers owed a touch that are
    /// visible ([`LwsState::touch_owed`]), and must not touch one that a later change in `all`
    /// would then restore to a date from before the touch.
    #[cfg(test)]
    pub(crate) fn set_aside_all(&self, all: Vec<Unsettled>) {
        self.settle_hidden(self.hide_all(all));
    }

    /// Hide each of `all`, to be put back by [`LwsState::settle_hidden`]: until then nothing of
    /// them is put back, so what is not visible stays so.
    pub(crate) fn hide_all(&self, all: Vec<Unsettled>) -> Hidden {
        let hidden: Vec<_> = all.iter().map(|left| self.hide(left, true)).collect();
        Hidden(all.into_iter().zip(hidden).collect())
    }

    /// Put back, each in a task of its own, the changes [`LwsState::hide_all`] hid.
    pub(crate) fn settle_hidden(&self, hidden: Hidden) {
        for (left, hidden) in hidden.0 {
            self.settle_aside(left, hidden, ());
        }
    }

    /// Hide the resources of `left` and, when `listed`, the containers listing them, and count
    /// what it holds to put back: what [`LwsState::settle_aside`] undoes once it is back.
    fn hide(&self, left: &Unsettled, listed: bool) -> (Vec<String>, usize) {
        let storage = self.cfg.storage();
        let mut iris = left.iris();
        if listed {
            let parents: Vec<String> = iris
                .iter()
                .filter_map(|i| resources::parent_of(i, &storage))
                .collect();
            iris.extend(parents);
        }
        iris.sort();
        iris.dedup();
        let bytes = left.bytes();
        self.set_aside_bytes
            .fetch_add(bytes, std::sync::atomic::Ordering::AcqRel);
        self.locks.hide(&iris, left.priors());
        (iris, bytes)
    }

    /// Put `left` back in a task of its own, holding `locks`, then show what [`LwsState::hide`]
    /// hid (`hidden`) and touch the containers owed a touch.
    fn settle_aside<L: Send + 'static>(
        &self,
        left: Unsettled,
        hidden: (Vec<String>, usize),
        locks: L,
    ) {
        let (iris, bytes) = hidden;
        let state = self.detached();
        tokio::spawn(async move {
            for undo in &left.0 {
                until_done(|| undo.apply(&state.store)).await;
                if let Undo::Resolve {
                    record,
                    touch: Some(container),
                    ..
                } = undo
                {
                    // Kept, the intent names the container until the touch lands; put back,
                    // it is gone, and clearing it again does nothing.
                    state.owe_touch(container, record.clone());
                }
            }
            // Released while its locks are held: nothing can stage what it put back meanwhile.
            left.2.release(&state).await;
            drop(left);
            state.locks.show(&iris);
            state
                .set_aside_bytes
                .fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
            drop(locks);
            state.touch_owed().await;
        });
    }

    /// Touch every container whose touch is owed ([`LwsState::owe_touch`]) and that is visible
    /// (one still set aside is touched when its own change is settled), each until the touch
    /// lands and clears the intents waiting on it, waiting longer between tries (up to
    /// [`UNDO_MAX_WAIT`]).
    pub(crate) async fn touch_owed(&self) {
        let owed: Vec<String> = self
            .owed_touches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for container in owed {
            let mut wait = std::time::Duration::from_millis(100);
            while self.visible(&container) && self.owes_touch(&container) {
                self.touching(&container);
                resources::touch_container(self, &container).await;
                if !self.owes_touch(&container) {
                    break;
                }
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(UNDO_MAX_WAIT);
            }
        }
    }

    /// Whether a touch of `uri` is owed.
    fn owes_touch(&self, uri: &str) -> bool {
        self.owed_touches
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(uri)
            .is_some_and(|r| !r.is_empty())
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
    /// `encode_meta`), so metadata that could not be read again is never stored.
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
/// (The one exception is an edge left to a linkset record already gone, which
/// [`Journal::remove_member`] detaches alone.)
/// A container that is not empty is not removed ([`crate::store::DeleteOutcome::NotEmpty`]).
pub(crate) async fn remove_member<S: Store>(
    store: &S,
    iri: &str,
    parent: Option<&str>,
) -> Result<crate::store::DeleteOutcome, crate::error::ServerError> {
    store.delete_container_if_empty(iri, parent).await
}

/// What a [`Journal`] may hold to put a change back: what each resource it stages costs it
/// ([`Journal::stage`]) comes to at most this. 64 MiB.
pub(crate) const JOURNAL_LIMIT: usize = 64 << 20;

/// What staging one resource costs a [`Journal`], besides its name and, for a metadata record,
/// its bytes: what the store's hold on its bytes and their metadata take in memory, counted at a
/// fixed size so that it follows neither.
const STAGED_ENTRY_BYTES: usize = 512;

/// One mutation's store changes, each recorded with what it replaced, so that a failure part way
/// puts back everything done before it ([`Journal::rollback`]). Every mutation of more than one
/// store step goes through one: a write or delete that fails leaves content, metadata and
/// membership as they were, validators included (records are put back with
/// [`Store::reinstate`], pointing at the bytes the store held for them, see [`Journal::stage`]).
/// The one exception is the edge a container holds to a linkset record already gone, which
/// [`Journal::remove_member`] detaches in a step of its own: that edge is not put back when the
/// change is, since it names nothing and no LWS listing shows it, so a failed change can leave a
/// container one dangling record edge lighter than it began.
/// What a journal holds to put things back is bounded (`limit` bytes); a change that would need
/// more is refused, and one that knows its whole extent up front ([`Journal::stage`]) is refused
/// before it changes anything.
///
/// The holds it takes are released once nothing can need them: when the change is kept or put
/// back and its intent cleared, or, for a change set aside, once it is settled
/// ([`Unsettled`] carries them). A journal dropped before its intent was stored releases them
/// on a task of its own; one dropped after leaves them to the intent, which the next start
/// settles.
pub(crate) struct Journal<'a, S: Store + 'static> {
    state: &'a LwsState<S>,
    undo: Vec<Undo>,
    staged: std::collections::HashMap<String, Option<Kept>>,
    held: usize,
    limit: usize,
    /// What putting back each resource staged so far takes, in the order staged: the durable
    /// intent ([`intents`]) stores it, last first, before a step changes anything.
    planned: Vec<Undo>,
    planned_keys: std::collections::HashSet<String>,
    /// How many of `planned` the stored intent holds.
    persisted: usize,
    /// The stored intent, once there is one.
    intent: Option<String>,
    /// The resources staged so far, published as in flight from before the first step.
    in_flight: Option<InFlight>,
    /// The holds staged so far (see [`Kept::Held`]).
    holds: Holds<S>,
}

/// The holds a [`Journal`] took, all as one holder: the IRI its intent is (or would be) stored
/// at, so no other journal's release can end them, and a start can tell whose they are. Released
/// when it is dropped before its intent was stored (a change that panicked part way): once it is
/// stored, the intent keeps them, and the next start settles it and releases them
/// ([`intents::recover`]).
struct Holds<S: Store + 'static> {
    state: LwsState<S>,
    holding: Holding,
    intended: bool,
}

/// Holds taken as one holder (see [`Store::hold`]): each held resource, with what was held.
#[derive(Default)]
pub(crate) struct Holding {
    pub(crate) holder: String,
    pub(crate) held: Vec<(String, StoredMeta)>,
}

impl Holding {
    /// Release every hold; one whose release fails is tried again by the maintenance sweeper
    /// ([`LwsState::unreleased`]).
    pub(crate) async fn release<S: Store + 'static>(&self, state: &LwsState<S>) {
        for (_, meta) in &self.held {
            if state.store.release(meta, &self.holder).await.is_err() {
                state.release_later(&self.holder, meta);
            }
        }
    }
}

/// How many failed releases wait to be tried again at most: past it, one is counted and left to
/// the next start's sweep ([`intents::recover`]), as a hold that outlives its change only keeps
/// bytes.
pub(crate) const UNRELEASED_LIMIT: usize = 4096;

impl<S: Store + 'static> Holds<S> {
    fn take(&mut self) -> Holding {
        Holding {
            holder: self.holding.holder.clone(),
            held: std::mem::take(&mut self.holding.held),
        }
    }
}

impl<S: Store + 'static> Drop for Holds<S> {
    fn drop(&mut self) {
        if self.holding.held.is_empty() || self.intended {
            return;
        }
        let (state, holding) = (self.state.clone(), self.take());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move { holding.release(&state).await });
        }
    }
}

type StoredMeta = crate::store::ResourceMeta;

/// What a resource held before a change, to put it back to.
#[derive(Clone)]
pub(crate) enum Kept {
    /// Its record's metadata, whose blob key names bytes the store holds for it
    /// ([`Store::hold`]), and, for a resource's metadata record, those bytes too (`peek`): who
    /// may act on the resource while the change is in flight is decided on them ([`Prior`]).
    Held {
        meta: StoredMeta,
        peek: Option<Bytes>,
    },
    /// Its bytes themselves, with its record's metadata: put back by a copy
    /// ([`Store::restore`]). Only an intent stored before the store held bytes reads back so.
    Copied { body: Bytes, meta: StoredMeta },
}

impl Kept {
    fn meta(&self) -> &StoredMeta {
        match self {
            Kept::Held { meta, .. } | Kept::Copied { meta, .. } => meta,
        }
    }

    /// The bytes of a metadata record, when they are known.
    fn body(&self) -> Option<&Bytes> {
        match self {
            Kept::Held { peek, .. } => peek.as_ref(),
            Kept::Copied { body, .. } => Some(body),
        }
    }

    /// About how many bytes it takes in memory.
    fn bytes(&self) -> usize {
        STAGED_ENTRY_BYTES.saturating_add(self.body().map_or(0, Bytes::len))
    }

    /// Put `iri` back as this says: a member of `container` when one is given.
    async fn put_back<S: Store>(
        &self,
        store: &S,
        iri: &str,
        container: Option<&str>,
    ) -> Result<(), crate::error::ServerError> {
        match self {
            Kept::Held { meta, .. } => store.reinstate(iri, container, meta).await.map(drop),
            Kept::Copied { body, meta } => store
                .restore(iri, container, body.clone(), meta)
                .await
                .map(drop),
        }
    }
}

/// The holds `undo` names (see [`Kept::Held`]): each resource's, once.
pub(crate) fn holds_of(undo: &[Undo]) -> Vec<(String, StoredMeta)> {
    let mut out = Vec::new();
    for step in undo {
        match step {
            Undo::Restore {
                key,
                prior: Some(Kept::Held { meta, .. }),
            } => out.push((key.clone(), meta.clone())),
            Undo::Recreate {
                iri,
                kept: Kept::Held { meta, .. },
                ..
            } => out.push((iri.clone(), meta.clone())),
            Undo::Resolve { undo, .. } => out.extend(holds_of(undo)),
            _ => {}
        }
    }
    out.sort_by(|a, b| a.1.blob_key.cmp(&b.1.blob_key));
    out.dedup_by(|a, b| a.1.blob_key == b.1.blob_key);
    out
}

/// Whether a store step with this outcome may have changed something: it succeeded, or failed in
/// the backend, where the change may have landed before the failure was reported.
pub(crate) fn may_have_happened<T>(outcome: &Result<T, crate::error::ServerError>) -> bool {
    matches!(
        outcome,
        Ok(_)
            | Err(crate::error::ServerError::Storage(_))
            | Err(crate::error::ServerError::Unsettled { .. })
    )
}

/// How to put back one change a [`Journal`] made.
pub(crate) enum Undo {
    /// `key` as it was: its bytes and record, or absent.
    Restore { key: String, prior: Option<Kept> },
    /// A removed member, recreated in its container as it was.
    Recreate {
        iri: String,
        parent: Option<String>,
        kept: Kept,
    },
    /// A member that may have been created, removed from its container (see [`records::delete_record`]).
    Remove { iri: String, parent: String },
    /// A write that failed unsettled, fenced so that it can no longer commit: the steps after it
    /// put back what it may have written, and must not be outlived by it.
    Fence { fence: crate::store::Fence },
    /// Nothing to put back: a resource the change holds the lock of without having changed it
    /// (one a recursive delete had not yet removed). It is set aside with the change, so nobody
    /// waits on that lock while the rest is put back.
    Locked { iri: String },
    /// The durable intent `record` of a change (see [`intents`]), cleared once the change is
    /// put back: until then its resources (`iris`) stay set aside, so no later write to them can
    /// happen while a start could still replay it.
    Forget { record: String, iris: Vec<String> },
    /// A kept change whose intent could not be recorded as kept (rewritten to name only its
    /// container, or cleared): the store may have done it anyway, a lost reply. Settled from
    /// what the intent `record` holds now: cleared, or naming only a container, the change is
    /// kept; still holding its plan, the change is put back (`undo`, in order) and the intent
    /// cleared. Until then its resources (`iris`) stay set aside, as for [`Undo::Forget`]. Once
    /// it is settled, the container `touch` its change listed is touched, either way (see
    /// [`LwsState::set_aside`]).
    Resolve {
        record: String,
        undo: Vec<Undo>,
        iris: Vec<String>,
        touch: Option<String>,
    },
}

impl Undo {
    /// Put this change back, with the mutation's locks still held.
    async fn apply<S: Store>(&self, store: &S) -> Result<(), crate::error::ServerError> {
        match self {
            Undo::Restore {
                key,
                prior: Some(kept),
            } => kept.put_back(store, key, None).await,
            Undo::Restore {
                key, prior: None, ..
            } => match store.delete(key, None).await {
                Ok(()) | Err(crate::error::ServerError::NotFound) => Ok(()),
                Err(e) => Err(e),
            },
            Undo::Recreate { iri, parent, kept } => {
                if store.exists(iri).await? {
                    return Ok(());
                }
                kept.put_back(store, iri, parent.as_deref()).await
            }
            Undo::Remove { iri, parent } => records::delete_record(store, iri, parent).await,
            Undo::Fence { fence } => store.fence(fence).await,
            Undo::Locked { .. } => Ok(()),
            Undo::Forget { record, .. } => intents::clear(store, record).await,
            Undo::Resolve { record, undo, .. } => {
                if intents::kept(store, record).await? {
                    return Ok(());
                }
                for step in undo {
                    Box::pin(step.apply(store)).await?;
                }
                intents::clear(store, record).await
            }
        }
    }

    /// The resources this change is to: a member's container too, when its listing changes.
    fn iris(&self) -> Vec<&str> {
        match self {
            Undo::Restore { key, .. } => vec![key.strip_suffix(META_SUFFIX).unwrap_or(key)],
            Undo::Recreate { iri, parent, .. } => std::iter::once(iri.as_str())
                .chain(parent.as_deref())
                .collect(),
            Undo::Remove { iri, parent } => vec![iri, parent],
            Undo::Fence { .. } => Vec::new(),
            Undo::Locked { iri } => vec![iri],
            Undo::Forget { iris, .. } | Undo::Resolve { iris, .. } => {
                iris.iter().map(String::as_str).collect()
            }
        }
    }
}

/// What is left to put back of a change that could not be put back after a few tries, the next
/// to apply first, and the holds of its journal (see [`Journal`]), released once it is put back.
pub(crate) struct Unsettled(Vec<Undo>, Option<InFlight>, Holding);

/// A journaled change's resources published as in flight ([`IriLocks::changing`]), with what
/// each was before the change began, until this is dropped: from before the change's first
/// step, through any putting back while its request waits, to the moment it is either done or
/// set aside (what is left of it, [`Unsettled`], carries this until it is put back, by which
/// time it is set aside). A check made without the lock meanwhile is decided on that earlier
/// state ([`access::aside`]), never on a write the change may yet put back.
///
/// [`IriLocks::changing`]: resources::IriLocks::changing
pub(crate) struct InFlight {
    locks: Arc<resources::IriLocks>,
    iris: Vec<String>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.locks.landed(&self.iris);
    }
}

/// Changes set aside and hidden ([`LwsState::hide_all`]), not yet being put back.
#[must_use = "hidden changes are put back only by LwsState::settle_hidden"]
pub(crate) struct Hidden(Vec<(Unsettled, (Vec<String>, usize))>);

impl Unsettled {
    /// What is left of a change, holding nothing (its caller hands it the holds it carries).
    pub(crate) fn new(undo: Vec<Undo>) -> Self {
        Self(undo, None, Holding::default())
    }

    /// About how many bytes it holds to put back: each body, and each record at its limit.
    fn bytes(&self) -> usize {
        bytes_of(&self.0)
    }

    /// The resources it is to, each once.
    fn iris(&self) -> Vec<String> {
        iris_of(&self.0)
    }

    /// What each resource it is to was before the change (see [`Prior`]).
    fn priors(&self) -> std::collections::HashMap<String, Prior> {
        let mut out = std::collections::HashMap::new();
        priors_of(&self.0, &mut out);
        out
    }
}

/// About how many bytes `undo` holds to put back: each body, and each record at its limit.
fn bytes_of(undo: &[Undo]) -> usize {
    undo.iter()
        .map(|u| match u {
            Undo::Restore {
                prior: Some(kept), ..
            }
            | Undo::Recreate { kept, .. } => kept.bytes().saturating_add(MAX_META_BYTES),
            Undo::Fence { .. } | Undo::Locked { .. } | Undo::Forget { .. } => 0,
            Undo::Resolve { undo, .. } => bytes_of(undo),
            _ => MAX_META_BYTES,
        })
        .fold(0, usize::saturating_add)
}

/// What a resource that a stuck change is to was before the change began, as far as deciding
/// who may act on it goes: taken from what the change's journal staged before its first step
/// (and so before any of its writes landed), never from what is stored while it is part way.
/// A field the change left nothing to put back of (it did not touch it, or it is put back
/// already) is `None`: what is stored for it now is what it was.
///
/// It decides only who may learn the resource is set aside, or wait for the lock of one a change
/// is in flight to ([`access::aside`]); nothing is served or changed on it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Prior {
    /// Whether the resource was there.
    pub existed: Option<bool>,
    /// Its metadata: the default when it had none, and, when what it had does not parse, one
    /// marked `pending` (which refuses every grant).
    pub meta: Option<ResourceMeta>,
    /// Its format (a container's is its listing's).
    pub format: Option<Option<String>>,
}

impl Prior {
    /// Fill what this does not say yet from `other`.
    pub(crate) fn or(&mut self, other: Prior) {
        self.existed = self.existed.or(other.existed);
        if self.meta.is_none() {
            self.meta = other.meta;
        }
        if self.format.is_none() {
            self.format = other.format;
        }
    }
}

/// What each resource `undo` (newest step first, as it is put back) puts back was before its
/// change (see [`Prior`]).
fn priors_of(undo: &[Undo], out: &mut std::collections::HashMap<String, Prior>) {
    let format = |iri: &str, meta: &StoredMeta| {
        Some(if iri.ends_with('/') {
            LWS_JSON.to_string()
        } else {
            meta.content_type.clone()
        })
    };
    // Put back newest first: a key written twice (metadata marked `pending`, then the new
    // metadata) has two steps, the later one holding the intermediate state. The oldest
    // step's prior, the state from before the change, is the one kept.
    for step in undo.iter().rev() {
        let (iri, prior) = match step {
            Undo::Restore { key, prior } => match key.strip_suffix(META_SUFFIX) {
                Some(iri) => {
                    let meta = match prior {
                        Some(kept) => kept
                            .body()
                            .and_then(|body| parse_meta(iri, body).ok())
                            .unwrap_or(ResourceMeta {
                                pending: true,
                                ..ResourceMeta::default()
                            }),
                        None => ResourceMeta::default(),
                    };
                    (
                        iri.to_string(),
                        Prior {
                            meta: Some(meta),
                            ..Prior::default()
                        },
                    )
                }
                None => (
                    key.clone(),
                    Prior {
                        existed: Some(prior.is_some()),
                        format: Some(prior.as_ref().and_then(|k| format(key, k.meta()))),
                        ..Prior::default()
                    },
                ),
            },
            Undo::Recreate { iri, kept, .. } => (
                iri.clone(),
                Prior {
                    existed: Some(true),
                    format: Some(format(iri, kept.meta())),
                    ..Prior::default()
                },
            ),
            Undo::Remove { iri, .. } => (
                iri.clone(),
                Prior {
                    existed: Some(false),
                    ..Prior::default()
                },
            ),
            Undo::Resolve { undo, .. } => {
                priors_of(undo, out);
                continue;
            }
            Undo::Fence { .. } | Undo::Locked { .. } | Undo::Forget { .. } => continue,
        };
        out.entry(iri).or_default().or(prior);
    }
}

/// The resources `undo` is to, each once.
fn iris_of(undo: &[Undo]) -> Vec<String> {
    let mut iris: Vec<String> = undo
        .iter()
        .flat_map(Undo::iris)
        .map(str::to_string)
        .collect();
    iris.sort();
    iris.dedup();
    iris
}

/// How many times each step of putting a change back is tried while the request that made the
/// change waits on it, before what is left is set aside ([`LwsState::set_aside`]).
const UNDO_ATTEMPTS: u32 = 3;

/// Apply `undo` in order, each tried a few times ([`UNDO_ATTEMPTS`], with short waits between):
/// `None` once all are applied, else what is left, from the step that kept failing on.
pub(crate) async fn settle<S: Store>(store: &S, undo: Vec<Undo>) -> Option<Unsettled> {
    let mut steps = undo.into_iter();
    while let Some(step) = steps.next() {
        let mut wait = std::time::Duration::from_millis(50);
        let mut done = false;
        for attempt in 1..=UNDO_ATTEMPTS {
            if step.apply(store).await.is_ok() {
                done = true;
                break;
            }
            if attempt < UNDO_ATTEMPTS {
                tokio::time::sleep(wait).await;
                wait *= 4;
            }
        }
        if !done {
            return Some(Unsettled::new(std::iter::once(step).chain(steps).collect()));
        }
    }
    None
}

/// How many bytes set-aside changes may hold to put back before new changes are refused (see
/// [`LwsState::may_write`]).
const MAX_SET_ASIDE_BYTES: usize = 256 << 20;

/// How long, at most, a set-aside change waits between attempts to put it back.
const UNDO_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run `step` until it succeeds, waiting between attempts (growing to [`UNDO_MAX_WAIT`]): how a
/// set-aside change is put back while the locks that keep anyone else from it are held.
async fn until_done<T, E, F, Fut>(mut step: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut wait = std::time::Duration::from_millis(100);
    loop {
        if let Ok(done) = step().await {
            return done;
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(UNDO_MAX_WAIT);
    }
}

impl<'a, S: Store + 'static> Journal<'a, S> {
    pub(crate) fn new(state: &'a LwsState<S>, limit: usize) -> Self {
        Self {
            state,
            undo: Vec::new(),
            staged: std::collections::HashMap::new(),
            held: 0,
            limit,
            planned: Vec::new(),
            planned_keys: Default::default(),
            persisted: 0,
            intent: None,
            in_flight: None,
            holds: Holds {
                state: state.clone(),
                holding: Holding {
                    holder: intents::mint(&state.cfg.storage()),
                    held: Vec::new(),
                },
                intended: false,
            },
        }
    }

    /// Have the store hold what is stored at `key` now ([`Store::hold`]), to put back later
    /// without a copy, and charge it to the journal's bound: a change staged whole before its
    /// first step is refused, if too large, before it changes anything. What a resource costs is
    /// its name and a fixed size ([`STAGED_ENTRY_BYTES`]), and for a metadata record its bytes,
    /// read too (who may act on the resource while the change is in flight is decided on them):
    /// never the size of its content.
    pub(crate) async fn stage(&mut self, key: &str) -> Result<(), crate::error::ServerError> {
        if self.staged.contains_key(key) {
            return Ok(());
        }
        let prior = match self
            .state
            .store
            .hold(key, &self.holds.holding.holder)
            .await?
        {
            Some(meta) => {
                // Held from here on: released with the journal's holds, whatever follows.
                self.holds
                    .holding
                    .held
                    .push((key.to_string(), meta.clone()));
                let peek = match key.ends_with(META_SUFFIX) {
                    true => Some(self.state.store.read_at(key, &meta).await?),
                    false => None,
                };
                Some(Kept::Held { meta, peek })
            }
            None => None,
        };
        self.held = self.held.saturating_add(
            key.len()
                .saturating_add(prior.as_ref().map_or(STAGED_ENTRY_BYTES, Kept::bytes)),
        );
        if self.held > self.limit {
            return Err(crate::error::ServerError::Conflict(
                "the change is too large to make atomically".into(),
            ));
        }
        if self.planned_keys.insert(key.to_string()) {
            self.planned.push(Undo::Restore {
                key: key.to_string(),
                prior: prior.clone(),
            });
        }
        self.staged.insert(key.to_string(), prior);
        Ok(())
    }

    /// [`Journal::stage`] the member `iri` of `parent`: put back, it is recreated in `parent`
    /// (or removed from it, when it was absent).
    pub(crate) async fn stage_member(
        &mut self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<(), crate::error::ServerError> {
        let fresh = !self.planned_keys.contains(iri);
        self.stage(iri).await?;
        if fresh {
            let restore = self.planned.pop().expect("just planned");
            let Undo::Restore { key, prior } = restore else {
                unreachable!("stage plans a restore")
            };
            self.planned.push(match (prior, parent) {
                (Some(kept), _) => Undo::Recreate {
                    iri: key,
                    parent: parent.map(str::to_string),
                    kept,
                },
                (None, Some(parent)) => Undo::Remove {
                    iri: key,
                    parent: parent.to_string(),
                },
                (None, None) => Undo::Restore { key, prior: None },
            });
        }
        Ok(())
    }

    /// Store the intent to put back everything staged so far, unless it is stored already:
    /// before any step that changes something.
    async fn durable(&mut self) -> Result<(), crate::error::ServerError> {
        if self.persisted == self.planned.len() {
            return Ok(());
        }
        let existing = self.intent.is_some();
        let record = self
            .intent
            .get_or_insert_with(|| self.holds.holding.holder.clone())
            .clone();
        // Published first: a check without the lock from now on is made on what was staged,
        // whatever this change writes or puts back.
        let mut priors = std::collections::HashMap::new();
        priors_of(&self.planned[self.persisted..], &mut priors);
        let iris: Vec<String> = priors.keys().cloned().collect();
        self.state.locks.changing_from(priors);
        let in_flight = self.in_flight.get_or_insert_with(|| InFlight {
            locks: self.state.locks.clone(),
            iris: Vec::new(),
        });
        in_flight.iris.extend(iris);
        let undo: Vec<&Undo> = self.planned.iter().rev().collect();
        // From here on the intent may be stored (a lost reply), and keeps the holds.
        self.holds.intended = true;
        intents::store(self.state, &record, existing, &undo, &[]).await?;
        self.persisted = self.planned.len();
        Ok(())
    }

    /// What was stored at `key` before this step (staged now if it was not already).
    async fn prior(&mut self, key: &str) -> Result<Option<Kept>, crate::error::ServerError> {
        self.stage(key).await?;
        Ok(self.staged.remove(key).flatten())
    }

    /// Write `body` at `key` (a resource, or a resource's metadata record).
    pub(crate) async fn write(
        &mut self,
        key: &str,
        body: Bytes,
        content_type: &str,
    ) -> Result<StoredMeta, crate::error::ServerError> {
        let prior = self.prior(key).await?;
        self.durable().await?;
        let written = self.state.store.write(key, body, content_type).await;
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

    /// Remove the metadata of `iri`; absent is fine.
    pub(crate) async fn delete_meta(&mut self, iri: &str) -> Result<(), crate::error::ServerError> {
        let key = meta_key(iri);
        let prior = self.prior(&key).await?;
        if prior.is_none() {
            return Ok(());
        }
        self.durable().await?;
        let deleted = self.state.store.delete(&key, None).await;
        if may_have_happened(&deleted) {
            self.undo.push(Undo::Restore { key, prior });
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
        let Some(kept) = self.prior(iri).await? else {
            // A record a legacy edge lists may be gone already (its subject's removal took it):
            // the edge, which is all that is left, goes too, so it keeps no container from being
            // empty. The edge is not put back if the change is (it names nothing, and no LWS
            // listing shows it), and it goes in a step of its own: with the record absent, no
            // live member can be left unlisted by it.
            if let (true, Some(parent)) = (names_record(iri), parent) {
                self.durable().await?;
                match self.state.store.delete(iri, Some(parent)).await {
                    Ok(()) | Err(crate::error::ServerError::NotFound) => {}
                    Err(e) => return Err(e),
                }
            }
            return Ok(crate::store::DeleteOutcome::NotFound);
        };
        self.durable().await?;
        let outcome = remove_member(&self.state.store, iri, parent).await;
        // Only a member that went, or may have, is recreated.
        if matches!(
            outcome,
            Ok(crate::store::DeleteOutcome::Deleted)
                | Err(crate::error::ServerError::Storage(_))
                | Err(crate::error::ServerError::Unsettled { .. })
        ) {
            self.undo.push(Undo::Recreate {
                iri: iri.to_string(),
                parent: parent.map(str::to_string),
                kept,
            });
        }
        outcome
    }

    /// Keep every change. When it changed what the container `touch` lists, its intent is
    /// rewritten to name only that container, to be cleared once the container is touched
    /// ([`LwsState::owe_touch`]): a stop before then leaves the container's date for the next
    /// start to move on. Otherwise its intent is cleared.
    ///
    /// When that fails, the store may have done it anyway (a lost reply): putting the change
    /// back regardless could leave the intent no longer holding the plan to finish that, should
    /// the process stop. So the intent is read back: still holding its plan, the change is put
    /// back; cleared or naming only the container, it is kept. While it cannot be read, this
    /// fails with an [`Undo::Resolve`], to set aside with the locks, which settles it the same
    /// way later; meanwhile the container's listing has no date, as after a touch that failed.
    pub(crate) async fn commit(
        mut self,
        touch: Option<&str>,
    ) -> Result<(), (crate::error::ServerError, Option<Unsettled>)> {
        let Some(record) = self.intent.clone() else {
            self.holds.take().release(self.state).await;
            return Ok(());
        };
        let kept = match touch {
            Some(container) => {
                let owed = [container.to_string()];
                let rewritten = intents::store(self.state, &record, true, &[], &owed).await;
                if rewritten.is_ok() {
                    self.state.owe_touch(container, record.clone());
                }
                rewritten.is_ok()
            }
            None => {
                let forget = Undo::Forget {
                    record: record.clone(),
                    iris: Vec::new(),
                };
                settle(&self.state.store, vec![forget]).await.is_none()
            }
        };
        if kept {
            self.holds.take().release(self.state).await;
            return Ok(());
        }
        // Whether the store did it anyway is read back from the intent: still holding its plan,
        // the change is put back; cleared or naming only the container, it is kept.
        let mut wait = std::time::Duration::from_millis(50);
        for attempt in 1..=UNDO_ATTEMPTS {
            match intents::kept(&self.state.store, &record).await {
                Ok(true) => {
                    if let Some(container) = touch {
                        self.state.owe_touch(container, record);
                    }
                    self.holds.take().release(self.state).await;
                    return Ok(());
                }
                Ok(false) => {
                    return Err((
                        crate::error::ServerError::Storage(
                            "the change could not be recorded as kept".into(),
                        ),
                        self.rollback().await,
                    ));
                }
                Err(_) if attempt < UNDO_ATTEMPTS => {
                    tokio::time::sleep(wait).await;
                    wait *= 4;
                }
                Err(_) => {}
            }
        }
        if let Some(container) = touch {
            self.state.touching(container);
            self.state.touched(container, false);
        }
        let resolve = Undo::Resolve {
            touch: touch.map(str::to_string),
            record,
            iris: iris_of(&self.planned),
            undo: self.undo.into_iter().rev().collect(),
        };
        Err((
            crate::error::ServerError::Storage("the change could not be recorded as kept".into()),
            Some(Unsettled(vec![resolve], self.in_flight, self.holds.take())),
        ))
    }

    /// Put back every change, the last first, with the mutation's locks still held: each step
    /// is tried a few times ([`settle`]). What is left when one keeps failing comes back, for
    /// the caller to set aside with the locks ([`LwsState::set_aside`]), so nothing can act on
    /// what the mutation touched until it is back as it was, validators included: no later
    /// write can be overwritten by an old undo and no reader sees a state in between. The
    /// mutations run in a task of their own (see [`resources::hold_locks`]), so a client that
    /// goes away does not cut this short.
    ///
    /// Once it is all put back its intent is cleared; until then the intent stays, and goes with
    /// what is left (so its resources stay set aside until it is cleared too).
    pub(crate) async fn rollback(mut self) -> Option<Unsettled> {
        let forget = self.intent.take().map(|record| Undo::Forget {
            record,
            iris: iris_of(&self.planned),
        });
        let holds = self.holds.take();
        let store = &self.state.store;
        let left = match settle(
            store,
            std::mem::take(&mut self.undo).into_iter().rev().collect(),
        )
        .await
        {
            None => settle(store, forget.into_iter().collect()).await,
            Some(mut left) => {
                left.0.extend(forget);
                Some(left)
            }
        };
        match left {
            // Every hold the journal took, a superset of those its steps left name.
            Some(mut left) => {
                left.1 = self.in_flight.take();
                left.2 = holds;
                Some(left)
            }
            None => {
                holds.release(self.state).await;
                None
            }
        }
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

    /// What of the records in force is past this quota, for the start's log: `held` records
    /// counted against it overall, and the author of each one counted against its author's
    /// share. `None` when nothing is. Records past a quota (stored before it was lowered, or
    /// written to the store directly) are kept; [`Quota::reserve`] refuses a new one to a share
    /// until that share is under its quota again.
    pub(crate) fn past<'a>(
        &self,
        held: usize,
        authors: impl IntoIterator<Item = Option<&'a str>>,
    ) -> Option<String> {
        let mut by_author = std::collections::HashMap::<Option<&str>, usize>::new();
        for a in authors {
            *by_author.entry(a).or_default() += 1;
        }
        let over: Vec<usize> = by_author
            .into_values()
            .filter(|n| *n > self.per_author)
            .collect();
        let mut past = Vec::new();
        if held > self.total {
            past.push(format!("{held} held, of at most {}", self.total));
        }
        if let Some(most) = over.iter().max() {
            past.push(format!(
                "{} author(s) holding more than {} each (at most {most})",
                over.len(),
                self.per_author
            ));
        }
        (!past.is_empty()).then(|| past.join("; "))
    }
}

/// Log, at the start, that the records of `what` stored in `container` are past their quota
/// (`past`, from [`Quota::past`]). None is removed: each is a user's record.
pub(crate) fn report_past_quota(what: &str, container: &str, past: &str) {
    eprintln!(
        "lws: the {what} stored in {container} are past their quota: {past}; none is removed, \
         and no new one is admitted to a share until it is under its quota"
    );
}

pub(crate) use sparq_lws_authn::{expansion_budget, triple_bytes};

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
        index::MAX_FILTER_READ
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
    // Negotiated as a stored container's listing is.
    let Some(media_type) = resources::negotiate_container(req.header(header::ACCEPT)) else {
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
        json_response(StatusCode::OK, &media_type, &body)
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

pub use sparq_lws_authn::is_uri;

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
/// metadata write goes through `encode_meta`, which refuses more, and a write of content and
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
    for link in parse_links(&all(header::LINK)) {
        let rels = link
            .param("rel")
            .map_or(0, |r| r.split_whitespace().count());
        count = count.saturating_add(rels.max(1));
        bytes = bytes.saturating_add(rels.max(1).saturating_mul(link.target.len()));
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

/// One link of a Link header value (RFC 8288 section 3): its target and its parameters, both
/// borrowed from the value.
#[derive(Clone, Copy, Debug)]
pub struct Link<'a> {
    /// The target, between `<` and `>`, trimmed.
    pub target: &'a str,
    /// The parameters' text, every one a `link-param` ([`link_param`]).
    params: &'a str,
}

impl<'a> Link<'a> {
    /// The value of the parameter `name` (matched case-insensitively, RFC 8288 section 3), a
    /// quoted string unquoted with its escapes undone; the first when it repeats ("occurrences
    /// after the first MUST be ignored"). A parameter without a value has the empty one.
    pub fn param(&self, name: &str) -> Option<std::borrow::Cow<'a, str>> {
        let mut rest = self.params;
        while let Some((key, value, next)) = link_param(rest) {
            if key.eq_ignore_ascii_case(name) {
                return Some(unescape(value));
            }
            rest = next;
        }
        None
    }
}

/// Whether `b` is a `tchar` (RFC 9110 section 5.6.2), of which a `token` is made.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// `s` past its leading spaces and tabs (`OWS`, `BWS`).
fn skip_ows(s: &str) -> &str {
    s.trim_start_matches([' ', '\t'])
}

/// A `token` at the start of `s`, and what follows it; `None` when there is none.
fn token(s: &str) -> Option<(&str, &str)> {
    let n = s.bytes().take_while(|b| is_tchar(*b)).count();
    (n > 0).then(|| s.split_at(n))
}

/// A `quoted-string` at the start of `s` (RFC 9110 section 5.6.4): its text between the quotes,
/// `quoted-pair`s still escaped, and what follows it; `None` when `s` does not start with one
/// or it is never closed.
fn quoted_string(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix('"')?;
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // A quoted-pair: the backslash and the octet it escapes, whatever that is.
            b'\\' => i += 2,
            b'"' => return Some((&inner[..i], &inner[i + 1..])),
            _ => i += 1,
        }
    }
    None
}

/// The text of a quoted string with its `quoted-pair`s undone; borrowed when it has none.
fn unescape(raw: &str) -> std::borrow::Cow<'_, str> {
    if !raw.contains('\\') {
        return std::borrow::Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// One `OWS ";" OWS link-param` at the start of `s` (RFC 8288 section 3): its name, its value
/// (`token` or `quoted-string`, still escaped; empty when it has none) and what follows.
/// `None` when `s` does not start with `;`, or what follows is not a `link-param`.
fn link_param(s: &str) -> Option<(&str, &str, &str)> {
    let s = skip_ows(s).strip_prefix(';')?;
    let (name, s) = token(skip_ows(s))?;
    let s = skip_ows(s);
    let Some(v) = s.strip_prefix('=') else {
        return Some((name, "", s));
    };
    let v = skip_ows(v);
    if v.starts_with('"') {
        quoted_string(v).map(|(value, rest)| (name, value, rest))
    } else {
        token(v).map(|(value, rest)| (name, value, rest))
    }
}

/// A link relation as it is compared: lower-cased. A registered relation type and an extension
/// one (a URI) alike are compared case-insensitively (RFC 8288 section 2.1.2), so two spellings of
/// one relation are one relation wherever relations are keyed, matched or searched. What is stored
/// and served keeps the spelling it was given.
pub(crate) fn relation_key(rel: &str) -> String {
    rel.to_ascii_lowercase()
}

/// The links of a Link header value (RFC 8288 section 3), read as they are needed: a caller that
/// stops at a bound (see `check_headers`) has spent nothing on what lies past it, and nothing
/// is allocated. Reading stops at the first part that is not a `link-value`: a target without its
/// `>` or with a `<` in it, or parameters that are not `link-param`s (a quoted string never
/// closed among them); nothing after it is read as a link.
pub fn parse_links(value: &str) -> impl Iterator<Item = Link<'_>> {
    let mut rest = value;
    std::iter::from_fn(move || {
        // Empty list elements and the whitespace around them are skipped (RFC 9110 5.6.1).
        let after = rest.trim_start_matches([' ', '\t', ',']).strip_prefix('<');
        rest = "";
        let after = after?;
        let end = after.find('>')?;
        let target = &after[..end];
        if target.contains('<') {
            return None;
        }
        let params = &after[end + 1..];
        let mut cursor = params;
        while let Some((_, _, next)) = link_param(cursor) {
            cursor = next;
        }
        // The link ends at the end of the value or at the `,` before the next one.
        let tail = skip_ows(cursor);
        if !(tail.is_empty() || tail.starts_with(',')) {
            return None;
        }
        rest = tail;
        Some(Link {
            target: target.trim(),
            params: &params[..params.len() - cursor.len()],
        })
    })
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
        /// Every `release` fails with a backend error, releasing nothing.
        pub fail_release: Arc<AtomicBool>,
        /// How many more `holds` listings fail with a backend error.
        pub failing_holds: Arc<std::sync::atomic::AtomicUsize>,
        /// `delete` of this IRI alone fails with a backend error.
        pub fail_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `read` of this IRI alone (or, one ending in `/`, of the members of that container)
        /// fails with a backend error.
        pub fail_read_of: Arc<std::sync::Mutex<Option<String>>>,
        pub fail_read_of_any: Arc<std::sync::Mutex<Vec<String>>>,
        /// `list_children` of this container alone fails with a backend error.
        pub fail_list_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `write` of this IRI alone fails with a backend error.
        pub fail_write_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `write` of every IRI that starts with this fails with a backend error.
        pub fail_write_under: Arc<std::sync::Mutex<Option<String>>>,
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
        /// The next `read` of the IRI named reads it, then waits on the gate before it answers.
        pub hold_next_read_reply_of: GateOf,
        /// `delete` of this IRI removes it, then reports a failure, as when the cleanup of its
        /// bytes fails after the index commit.
        pub fail_after_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// As `hold_next_create`, for the next `delete` of the IRI named.
        pub hold_next_delete_of: GateOf,
        /// `exists` reports this IRI absent, as if it was checked just before the IRI appeared.
        pub hide: Arc<std::sync::Mutex<Option<String>>>,
        /// The next `write` of this IRI (or, one ending in `/`, of a member of that container)
        /// commits, then reports a backend failure, as when a remote store's reply is lost after
        /// the update landed.
        pub fail_after_write_of: Arc<std::sync::Mutex<Option<String>>>,
        /// As `fail_after_write_of`, for the `n`th write (from 1) of the IRI named from now on
        /// alone: `(iri, n)`.
        pub lose_nth_write_of: Arc<std::sync::Mutex<Option<(String, usize)>>>,
        /// `meta` fails, as in a backend outage.
        pub fail_meta: Arc<AtomicBool>,
        /// `meta` of this IRI reports no modification time, as a backend that keeps none does.
        pub timeless: Arc<std::sync::Mutex<Option<String>>>,
        /// While [`FlakyStore::fail_exists`] is set, this many existence checks still answer
        /// before the rest fail.
        pub exists_answers: Arc<std::sync::atomic::AtomicUsize>,
        /// `create_in_container` of a resource (not of an intent, see the `intents` module)
        /// commits, then reports a backend failure, as when a remote store's reply is lost after
        /// the update landed.
        pub fail_after_create: Arc<AtomicBool>,
        /// `list_children` fails, as in a backend outage.
        pub fail_list: Arc<AtomicBool>,
        /// `write` of this IRI is refused before anything is written, as by a full store.
        pub refuse_write_of: Arc<std::sync::Mutex<Option<String>>>,
        /// A removal (`delete`, `delete_container_if_empty`) of this IRI is refused before
        /// anything is removed, as by a store that has no room for the change.
        pub refuse_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// `delete` of this IRI detaches it from its parent, then fails with the record still
        /// there, as a two-step delete does when its second step fails.
        pub partial_delete_of: Arc<std::sync::Mutex<Option<String>>>,
        /// When set, every read the store answers (`read`, `meta`, `exists`, a listing, `hold`)
        /// is recorded here as `"{call} {iri}"`, in order.
        pub trace: Arc<std::sync::Mutex<Option<Vec<String>>>>,
        /// How many `delete`s named a parent: removals of a member that are not one step.
        pub two_step_deletes: Arc<std::sync::atomic::AtomicUsize>,
        /// Fail every [`Store::restore`] of this resource until cleared.
        pub fail_restore_of: Arc<std::sync::Mutex<Option<String>>>,
        /// How many times `restore` was called.
        pub restores: Arc<std::sync::atomic::AtomicUsize>,
        /// The next `create_in_container` of a resource (not of an intent) answers at once that
        /// it failed unsettled ([`ServerError::Unsettled`]), as an HTTP update that timed out:
        /// the write is still sent, and commits once the gate opens, unless it has been fenced
        /// ([`Store::fence`]) by then.
        pub unsettle_next_create: Arc<std::sync::Mutex<Option<Arc<tokio::sync::Semaphore>>>>,
        /// As `unsettle_next_create`, for the next `delete_container_if_empty` of the IRI named.
        pub unsettle_next_delete_of: GateOf,
        /// As `unsettle_next_create`, for every create of a resource while set (one gate).
        pub unsettle_creates: Arc<std::sync::Mutex<Option<Arc<tokio::sync::Semaphore>>>>,
        /// `fence` fails, as while the remote store cannot be reached.
        pub fail_fence: Arc<AtomicBool>,
        /// How many times `fence` was called.
        pub fences_tried: Arc<std::sync::atomic::AtomicUsize>,
        /// `fence` never answers, as a call to a store that stalls.
        pub hang_fence: Arc<AtomicBool>,
        /// What the last write reported unsettled changes (its fence's `touches`).
        pub last_unsettled: Arc<std::sync::Mutex<Vec<String>>>,
        /// Each unsettled write by its fence's nonce: whether it is fenced, locked while it
        /// commits (a fence and the write it fences are each atomic).
        unsettled:
            Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<bool>>>>>,
        /// When set to `n`, the store step (write, create, delete) after the next `n` fails, once,
        /// before it changes anything; then it is cleared. See [`each_failure_changes_nothing`].
        pub fail_step: Arc<std::sync::Mutex<Option<usize>>>,
        /// Once [`FlakyStore::fail_step`] has failed a step, the step after the next `n` fails
        /// too, once: the harness fails putting a mutation back as well as the mutation.
        pub fail_then: Arc<std::sync::Mutex<Option<usize>>>,
        /// Once [`FlakyStore::fail_step`] has failed a step, every later step fails too: the
        /// store is gone from then on, as for a process that stops there.
        pub dead_after: Arc<AtomicBool>,
        /// `raise_marker` and `reset_marker` fail with a backend error.
        pub fail_marker_write: Arc<AtomicBool>,
        /// How many `raise_marker` and `reset_marker` calls were made, failed ones too.
        pub marker_writes: Arc<std::sync::atomic::AtomicUsize>,
        /// The store says it cannot raise a marker atomically.
        pub no_atomic_markers: Arc<AtomicBool>,
        fired: Arc<AtomicBool>,
    }

    impl FlakyStore {
        /// See `CompositeStore::plant_marker`.
        pub async fn plant_marker(&self, iri: &str, value: &str) {
            self.inner.plant_marker(iri, value).await
        }

        pub fn new() -> Self {
            Self::with_limits(crate::store::InMemoryStoreLimits::default())
        }

        /// Holding at most what `limits` allow.
        pub fn with_limits(limits: crate::store::InMemoryStoreLimits) -> Self {
            Self {
                inner: Arc::new(CompositeStore::new(
                    InMemorySparqClient::with_limits(limits),
                    InMemoryBlobStore::with_limits(limits),
                )),
                fail_delete: Arc::new(AtomicBool::new(false)),
                fail_exists: Arc::new(AtomicBool::new(false)),
                hold_next_create: Default::default(),
                hold_next_write_of: Default::default(),
                hold_next_read_reply_of: Default::default(),
                hold_next_delete_of: Default::default(),
                fail_after_delete_of: Default::default(),
                fail_exists_of: Default::default(),
                occupied: Default::default(),
                fail_delete_of: Default::default(),
                fail_write_of: Default::default(),
                fail_write_under: Default::default(),
                write_budget: Default::default(),
                fail_read_of: Default::default(),
                fail_read_of_any: Default::default(),
                fail_list_of: Default::default(),
                hide: Default::default(),
                fail_after_write_of: Default::default(),
                lose_nth_write_of: Default::default(),
                fail_list: Default::default(),
                fail_meta: Default::default(),
                timeless: Default::default(),
                exists_answers: Default::default(),
                refuse_write_of: Default::default(),
                refuse_delete_of: Default::default(),
                fail_after_create: Default::default(),
                partial_delete_of: Default::default(),
                two_step_deletes: Default::default(),
                trace: Default::default(),
                fail_restore_of: Default::default(),
                restores: Default::default(),
                unsettle_next_create: Default::default(),
                unsettle_next_delete_of: Default::default(),
                unsettle_creates: Default::default(),
                fail_fence: Default::default(),
                fences_tried: Default::default(),
                hang_fence: Default::default(),
                fail_release: Default::default(),
                failing_holds: Default::default(),
                last_unsettled: Default::default(),
                unsettled: Default::default(),
                fail_step: Default::default(),
                fail_then: Default::default(),
                dead_after: Default::default(),
                fail_marker_write: Default::default(),
                marker_writes: Default::default(),
                no_atomic_markers: Default::default(),
                fired: Default::default(),
            }
        }

        /// Whether a read (or a hold) of `iri` is to fail.
        fn read_fails(&self, iri: &str) -> bool {
            self.fail_read_of
                .lock()
                .unwrap()
                .as_deref()
                .is_some_and(|of| {
                    of == iri || (of.ends_with('/') && iri.starts_with(of) && iri != of)
                })
                || self
                    .fail_read_of_any
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|of| of == iri)
        }

        /// Record a read in [`FlakyStore::trace`], when it is kept.
        fn traced(&self, call: &str, iri: &str) {
            if let Some(trace) = self.trace.lock().unwrap().as_mut() {
                trace.push(format!("{call} {iri}"));
            }
        }

        /// Count a store step against [`FlakyStore::fail_step`].
        fn step(&self) -> ServerResult<()> {
            let countdown = |slot: &mut Option<usize>| match slot.as_mut() {
                Some(0) => {
                    *slot = None;
                    true
                }
                Some(n) => {
                    *n -= 1;
                    false
                }
                None => false,
            };
            let fail = if self.fired.load(Ordering::SeqCst) {
                self.dead_after.load(Ordering::SeqCst)
                    || countdown(&mut self.fail_then.lock().unwrap())
            } else {
                let mut slot = self.fail_step.lock().unwrap();
                let first = slot.is_some();
                let fail = countdown(&mut slot);
                if first && slot.is_none() {
                    self.fired.store(true, Ordering::SeqCst);
                }
                fail
            };
            if fail {
                return Err(ServerError::Storage("the injected failure".into()));
            }
            Ok(())
        }
    }

    impl FlakyStore {
        /// Answer that a write failed unsettled, and send `write` once `gate` opens, unless it
        /// was fenced by then.
        fn unsettle<F>(
            &self,
            gate: Arc<tokio::sync::Semaphore>,
            touches: Vec<String>,
            write: F,
        ) -> ServerError
        where
            F: std::future::Future + Send + 'static,
        {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let nonce = format!("flaky-{}", NEXT.fetch_add(1, Ordering::SeqCst));
            *self.last_unsettled.lock().unwrap() = touches.clone();
            let fenced = Arc::new(tokio::sync::Mutex::new(false));
            self.unsettled
                .lock()
                .unwrap()
                .insert(nonce.clone(), fenced.clone());
            tokio::spawn(async move {
                gate.acquire().await.expect("gate").forget();
                let fenced = fenced.lock().await;
                if !*fenced {
                    write.await;
                }
            });
            ServerError::Unsettled {
                detail: "the update timed out".into(),
                fence: crate::store::Fence::new(nonce, touches),
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
            self.traced("read", iri);
            if self.read_fails(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            let read = self.inner.read(iri).await;
            if let Some(gate) = held(&self.hold_next_read_reply_of, iri) {
                gate.acquire().await.expect("gate").forget();
            }
            read
        }
        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            self.traced("meta", iri);
            if self.fail_meta.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            let mut meta = self.inner.meta(iri).await?;
            if self.timeless.lock().unwrap().as_deref() == Some(iri) {
                if let Some(m) = meta.as_mut() {
                    m.last_modified = None;
                }
            }
            Ok(meta)
        }
        async fn exists(&self, iri: &str) -> ServerResult<bool> {
            self.traced("exists", iri);
            if self.fail_exists.load(Ordering::SeqCst)
                && self
                    .exists_answers
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_err()
            {
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
            if self.fail_write_of.lock().unwrap().as_deref() == Some(iri)
                || self
                    .fail_write_under
                    .lock()
                    .unwrap()
                    .as_deref()
                    .is_some_and(|p| iri.starts_with(p))
            {
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
                let lost = slot.as_deref().is_some_and(|of| {
                    of == iri || (of.ends_with('/') && iri.starts_with(of) && iri != of)
                });
                if lost {
                    *slot = None;
                }
                let mut nth = self.lose_nth_write_of.lock().unwrap();
                let lost_nth = match nth.as_mut() {
                    Some((of, n)) if of == iri => {
                        *n -= 1;
                        *n == 0
                    }
                    _ => false,
                };
                if lost_nth {
                    *nth = None;
                }
                lost || lost_nth
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
            if !container.starts_with(super::intents::PREFIX) {
                let gate = self
                    .unsettle_next_create
                    .lock()
                    .unwrap()
                    .take()
                    .or_else(|| self.unsettle_creates.lock().unwrap().clone());
                if let Some(gate) = gate {
                    let (inner, container, child, ct) = (
                        self.inner.clone(),
                        container.to_string(),
                        child.to_string(),
                        ct.to_string(),
                    );
                    let touches = vec![container.clone(), child.clone()];
                    return Err(self.unsettle(gate, touches, async move {
                        let _ = inner
                            .create_in_container(&container, &child, body, &ct)
                            .await;
                    }));
                }
            }
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
            if self.fail_after_create.load(Ordering::SeqCst)
                && !container.starts_with(super::intents::PREFIX)
            {
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
            // Counted as a step (so the harness fails it in turn); the write hooks are for the
            // writes a mutation makes, not for putting them back.
            self.restores.fetch_add(1, Ordering::SeqCst);
            self.step()?;
            if self.fail_restore_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.restore(iri, container, body, meta).await
        }
        async fn hold(&self, iri: &str, holder: &str) -> ServerResult<Option<ResourceMeta>> {
            self.traced("hold", iri);
            // What a journal stages is held, not read: the read hooks apply to it alike.
            if self.read_fails(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.hold(iri, holder).await
        }
        async fn reinstate(
            &self,
            iri: &str,
            container: Option<&str>,
            held: &ResourceMeta,
        ) -> ServerResult<ResourceMeta> {
            // A record put back, as `restore` is: counted as a step, and failed by the same hook.
            self.restores.fetch_add(1, Ordering::SeqCst);
            self.step()?;
            if self.fail_restore_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.reinstate(iri, container, held).await
        }
        async fn release(&self, held: &ResourceMeta, holder: &str) -> ServerResult<()> {
            if self.fail_release.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.release(held, holder).await
        }
        async fn holds(&self) -> ServerResult<Vec<(String, ResourceMeta)>> {
            if self
                .failing_holds
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.holds().await
        }
        async fn finish_reclaims(&self) -> ServerResult<()> {
            self.inner.finish_reclaims().await
        }
        async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
            self.step()?;
            if self.refuse_delete_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::InsufficientStorage);
            }
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
            if self.refuse_delete_of.lock().unwrap().as_deref() == Some(iri) {
                return Err(ServerError::InsufficientStorage);
            }
            if let Some(gate) = held(&self.unsettle_next_delete_of, iri) {
                let (inner, iri, parent) = (
                    self.inner.clone(),
                    iri.to_string(),
                    parent.map(str::to_string),
                );
                let touches = std::iter::once(iri.clone()).chain(parent.clone()).collect();
                return Err(self.unsettle(gate, touches, async move {
                    let _ = inner
                        .delete_container_if_empty(&iri, parent.as_deref())
                        .await;
                }));
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
        async fn fence(&self, fence: &crate::store::Fence) -> ServerResult<()> {
            self.fences_tried.fetch_add(1, Ordering::SeqCst);
            if self.hang_fence.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail_fence.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("the store cannot be reached".into()));
            }
            let write = self.unsettled.lock().unwrap().get(fence.nonce()).cloned();
            let Some(write) = write else {
                return Err(ServerError::Storage("no such write".into()));
            };
            *write.lock().await = true;
            Ok(())
        }
        async fn read_marker(&self, iri: &str) -> ServerResult<Option<String>> {
            self.inner.read_marker(iri).await
        }
        fn raises_markers(&self) -> bool {
            !self.no_atomic_markers.load(Ordering::SeqCst) && self.inner.raises_markers()
        }
        async fn raise_marker(&self, iri: &str, value: i64) -> ServerResult<i64> {
            self.marker_writes.fetch_add(1, Ordering::SeqCst);
            self.step()?;
            if self.fail_marker_write.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.raise_marker(iri, value).await
        }
        async fn reset_marker(
            &self,
            iri: &str,
            value: i64,
            lease: &crate::store::LifecycleGuard,
        ) -> ServerResult<()> {
            self.marker_writes.fetch_add(1, Ordering::SeqCst);
            self.step()?;
            if self.fail_marker_write.load(Ordering::SeqCst) {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.reset_marker(iri, value, lease).await
        }
        async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
            self.traced("list", container);
            if self.fail_list.load(Ordering::SeqCst)
                || self.fail_list_of.lock().unwrap().as_deref() == Some(container)
            {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.list_children(container).await
        }
        async fn list_children_within(
            &self,
            container: &str,
            max_bytes: usize,
        ) -> ServerResult<Option<Vec<ValidatedChildIri>>> {
            self.traced("list", container);
            if self.fail_list.load(Ordering::SeqCst)
                || self.fail_list_of.lock().unwrap().as_deref() == Some(container)
            {
                return Err(ServerError::Storage("disk on fire".into()));
            }
            self.inner.list_children_within(container, max_bytes).await
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
    /// mutation may change); and for each, also failing one of the first steps after that
    /// failure, so putting the mutation back fails too, and must be tried again. Every run that
    /// fails must leave them all exactly as they were, validators and responses included: the
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
            for then in [None, Some(0), Some(1), Some(2)] {
                let (st, store, resources, listings) = setup().await;
                let before = snapshot(&st, &store, &resources, &listings).await;
                let after_st = st.clone();
                *store.fail_step.lock().unwrap() = Some(n);
                *store.fail_then.lock().unwrap() = then;
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
                        "step {n} (then {then:?}) failed ({status}) and left a change behind"
                    );
                }
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

    /// The owner of a [`closed_state`]'s storage.
    pub const OWNER: &str = "https://owner.example/#me";

    /// An authenticated agent.
    pub fn agent(subject: &str) -> super::Agent {
        super::Agent {
            subject: Some(subject.into()),
            ..super::Agent::anonymous()
        }
    }

    /// A state over a [`FlakyStore`] that is not open: only its owner ([`OWNER`]), a resource's
    /// creator and the grants decide who may do what.
    pub async fn closed_state(page_size: usize) -> (super::LwsState<FlakyStore>, FlakyStore) {
        let store = FlakyStore::new();
        let mut cfg = super::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some(OWNER.into());
        cfg.allow_insecure_fetch = true;
        cfg.page_size = page_size;
        let state = super::LwsState::new(store.clone(), cfg).await.unwrap();
        (state, store)
    }

    /// Store a data resource `name` in the root; its IRI.
    pub async fn data(state: &super::LwsState<FlakyStore>, name: &str) -> String {
        let root = state.cfg.storage();
        let iri = format!("{root}{name}");
        state
            .store
            .create_in_container(&root, &iri, Bytes::from("x"), "text/plain")
            .await
            .unwrap();
        iri
    }

    /// As the owner, grant `assignee` read access to `targets`, and nothing else; the grant's
    /// IRI.
    pub async fn grant_read(
        state: &super::LwsState<FlakyStore>,
        assignee: &str,
        targets: &[&str],
    ) -> String {
        let grant = serde_json::json!({
            "@context": ["https://www.w3.org/ns/lws/v1"],
            "type": ["AccessGrant"],
            "storage": state.cfg.storage(),
            "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": assignee,
                "target": {"type": "StorageResource", "value": targets}}],
        });
        let r = super::access::handle(
            state,
            &request(
                axum::http::Method::POST,
                super::GRANTS_PATH,
                &[("content-type", super::LWS_JSON)],
                &grant.to_string(),
            ),
            &agent(OWNER),
        )
        .await;
        assert_eq!(r.status(), axum::http::StatusCode::CREATED);
        r.headers()[axum::http::header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }

    /// `req` routed as the server routes it, sent with an access token for `agent`'s subject
    /// (none for an anonymous one).
    pub async fn route_as(
        state: &super::LwsState<FlakyStore>,
        mut req: super::LwsRequest,
        agent: &super::Agent,
    ) -> axum::response::Response {
        if let Some(subject) = &agent.subject {
            let token = super::tokens::mint(
                &state.cfg,
                subject,
                "https://client.example/id",
                &state.cfg.realm(),
            );
            req.headers.insert(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
            );
        }
        super::route(state, req).await
    }

    /// Set `iri` aside, as a stuck write of its content is: putting back what it holds now
    /// (removing it, when it holds nothing: a stuck create) keeps failing until [`unstick`].
    pub async fn stick(state: &super::LwsState<FlakyStore>, store: &FlakyStore, iri: &str) {
        let prior = state
            .store
            .read(iri)
            .await
            .ok()
            .map(|r| super::Kept::Copied {
                body: r.body,
                meta: r.meta,
            });
        if prior.is_some() {
            *store.fail_restore_of.lock().unwrap() = Some(iri.to_string());
        } else {
            *store.fail_delete_of.lock().unwrap() = Some(iri.to_string());
        }
        let left = super::Unsettled::new(vec![super::Undo::Restore {
            key: iri.to_string(),
            prior,
        }]);
        state.set_aside(left, ());
        assert!(!state.visible(iri));
    }

    /// Let what [`stick`] set aside be put back.
    pub fn unstick(store: &FlakyStore) {
        *store.fail_restore_of.lock().unwrap() = None;
        *store.fail_delete_of.lock().unwrap() = None;
    }

    /// Wait (a few seconds at most) until `iri` is no longer set aside.
    pub async fn put_back(state: &super::LwsState<FlakyStore>, iri: &str) {
        for _ in 0..300 {
            if state.visible(iri) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("{iri} was never put back");
    }

    /// The `Link` targets of `resp` with relation `rel`.
    pub fn links(resp: &axum::response::Response, rel: &str) -> Vec<String> {
        resp.headers()
            .get_all(axum::http::header::LINK)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(super::parse_links)
            .filter(|l| l.param("rel").as_deref() == Some(rel))
            .map(|l| l.target.to_string())
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

    /// Review finding: the default OpenID Connect suite could not be swapped for one with
    /// trusted providers, as a second suite for ID Tokens is refused. An embedder offers the one
    /// it configured, and that instance is the one the token endpoint asks.
    #[cfg(feature = "authn-openid")]
    #[test]
    fn an_embedder_offers_the_suites_it_configured() {
        use sparq_lws_authn::Suite;
        use sparq_lws_authn_openid::OpenId;
        let openid = Arc::new(OpenId::new(vec!["https://op.example".into()]));
        let cfg =
            LwsConfig::with_suites("http://localhost:3000", [openid.clone() as Arc<dyn Suite>])
                .unwrap();
        let offered = cfg.suites().find(authz_server::ID_TOKEN_TYPE).unwrap();
        assert!(std::ptr::addr_eq(offered, Arc::as_ptr(&openid)));
        assert_eq!(cfg.suites().iter().count(), 1);
        // Still one suite per token type.
        assert!(LwsConfig::with_suites(
            "http://localhost:3000",
            [
                openid.clone() as Arc<dyn Suite>,
                Arc::new(OpenId::default())
            ]
        )
        .is_err());
        // And the default configuration still offers it, without trusted providers.
        assert!(LwsConfig::new("http://localhost:3000")
            .suites()
            .find(authz_server::ID_TOKEN_TYPE)
            .is_some());
    }

    /// Review finding: a SAML suite asked for by name, with an identity provider file that names
    /// none, was left out without a word. It stops the start; unasked, an empty file still turns
    /// SAML off. The only test that sets these variables.
    #[cfg(feature = "authn-saml")]
    #[test]
    fn a_saml_suite_asked_for_needs_identity_providers() {
        let file = std::env::temp_dir().join(format!("lws-idps-{}.json", std::process::id()));
        std::fs::write(&file, "{}").unwrap();
        std::env::set_var("SOLID_SERVER_LWS_SAML_IDPS_FILE", &file);
        std::env::set_var(AUTHN_VAR, "saml");
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let asked = cfg.register_suites_from_env();
        std::env::remove_var(AUTHN_VAR);
        let unasked = cfg.register_suites_from_env();
        std::env::remove_var("SOLID_SERVER_LWS_SAML_IDPS_FILE");
        std::fs::remove_file(&file).ok();
        assert!(asked.unwrap_err().contains("names no identity providers"));
        unasked.unwrap();
        assert!(cfg.suites().iter().all(|s| s.name() != "saml"));
    }

    /// `SOLID_SERVER_LWS_AUTHN` picks among the suites built in, and a suite's configuration is
    /// never ignored: set for a suite that is not offered, or a named SAML suite with no
    /// identity providers, it stops the start.
    #[test]
    fn the_environment_picks_the_suites_offered() {
        let offered = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> = vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            offered_suites(&move |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
        };
        let built = built_in_suites();
        let without_saml: Vec<&str> = built.iter().copied().filter(|n| *n != "saml").collect();
        assert_eq!(offered(&[]).unwrap(), without_saml);
        assert_eq!(offered(&[(AUTHN_VAR, " ")]).unwrap(), without_saml);
        let idps = ("SOLID_SERVER_LWS_SAML_IDPS_FILE", "idps.json");
        let issuers = (
            "SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS",
            "https://idp.example/",
        );
        assert!(offered(&[(AUTHN_VAR, "kerberos")])
            .unwrap_err()
            .contains("no authentication suite"));
        for (name, feature) in SUITES {
            let one = offered(&[(AUTHN_VAR, name), idps]);
            if built.contains(name) {
                if *name == "saml" {
                    assert_eq!(one.unwrap(), vec!["saml"]);
                } else {
                    // The SAML identity providers are set, but saml is left out (or not built).
                    let refused = if built.contains(&"saml") {
                        "leaves the saml suite out"
                    } else {
                        "authn-saml"
                    };
                    assert!(one.unwrap_err().contains(refused));
                    assert_eq!(offered(&[(AUTHN_VAR, name)]).unwrap(), vec![*name]);
                }
            } else {
                assert!(one.unwrap_err().contains(feature), "{name}");
            }
        }
        if built.contains(&"saml") {
            assert_eq!(offered(&[idps]).unwrap(), built);
            assert!(offered(&[(AUTHN_VAR, "saml")])
                .unwrap_err()
                .contains("SOLID_SERVER_LWS_SAML_IDPS_FILE is not set"));
        } else {
            assert!(offered(&[idps]).unwrap_err().contains("authn-saml"));
        }
        if built.contains(&"openid") && built.contains(&"ssi") {
            assert!(offered(&[(AUTHN_VAR, "ssi"), issuers])
                .unwrap_err()
                .contains("leaves the openid suite out"));
            assert_eq!(
                offered(&[(AUTHN_VAR, "openid, ssi,openid"), issuers]).unwrap(),
                vec!["ssi", "openid"]
            );
        }
    }

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
        // target that grows when resolved (here a relative one, against the resource URI)
        // passed it. Targets are weighed as kept: as sent this one fits, as kept it does not.
        let target = "a".repeat(MAX_DECLARED_LINK_BYTES / MAX_DECLARED_LINKS);
        assert!(target.len() * MAX_DECLARED_LINKS <= MAX_DECLARED_LINK_BYTES);
        let grows = format!(
            "<{target}>; rel=\"{}\"",
            rels[..MAX_DECLARED_LINKS].join(" ")
        );
        assert!(grows.len() < MAX_DECLARED_LINK_BYTES);
        assert_eq!(
            send("link", grows).await,
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
        );
        // A target that is not a URI reference is refused, not kept.
        assert_eq!(
            send("link", "<http://[>; rel=\"license\"".into()).await,
            StatusCode::BAD_REQUEST
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
            ("QUERY", TYPE_SEARCH_PATH, index::MAX_FILTER_READ),
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
        // A type search read to its limit is answered as a search: a well-formed filter that
        // long is too complex to page.
        let search = |len: usize| {
            let app = app.clone();
            async move {
                let head = "{\"type\": [\"https://e.example/T\"], \"@padding\": \"";
                let filter = format!("{head}{}\"}}", "x".repeat(len - head.len() - 2));
                assert_eq!(filter.len(), len);
                let req = Request::builder()
                    .method("QUERY")
                    .uri(TYPE_SEARCH_PATH)
                    .header("content-type", index::LWS_QUERY)
                    .body(Body::from(filter))
                    .unwrap();
                app.oneshot(req).await.unwrap().status()
            }
        };
        assert_eq!(
            search(index::MAX_FILTER_READ).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            search(index::MAX_FILTER_READ + 1).await,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(body_limit("/", 10), 10);
        assert_eq!(body_limit(REQUESTS_PATH, 10), 10);
    }

    /// Review finding: a member's record and its parent's membership edge were removed in two
    /// steps on some paths (service records, expired subscriptions), so a failure in between left
    /// a live record its container no longer listed. Every removal goes through
    /// [`remove_member`]; this drives each one and counts the two-step deletes it makes. (Only
    /// an edge left to a record already gone is detached alone; no path here makes one.)
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
        // Loaded over the backend itself: the running state holds the lease of its own handle.
        notify::Notifier::load(&store, &state.cfg).await.unwrap();
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
        // Review finding: a request for ld+json with the LWS profile, refusing lws+json, got
        // lws+json. It gets what it asked for, as from a stored container.
        let ld = get(
            "",
            &[(
                "accept",
                "application/ld+json; profile=\"https://www.w3.org/ns/lws/v1\", application/lws+json;q=0",
            )],
        );
        assert_eq!(ld.status(), StatusCode::OK);
        assert_eq!(
            ld.headers()[header::CONTENT_TYPE],
            "application/ld+json; profile=\"https://www.w3.org/ns/lws/v1\""
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
        let l: Vec<_> =
            parse_links(r#"<https://a/x>; rel="type", <b>; rel="up next"; title="a, b", <c>"#)
                .collect();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].target, "https://a/x");
        assert_eq!(l[0].param("rel").as_deref(), Some("type"));
        assert_eq!(l[1].param("rel").as_deref(), Some("up next"));
        assert_eq!(l[1].param("title").as_deref(), Some("a, b"));
        assert_eq!(l[2].target, "c");
        assert_eq!(l[2].param("rel"), None);
    }

    /// RFC 8288 section 3: occurrences of `rel` after the first are ignored. Parameter names
    /// are case-insensitive.
    #[test]
    fn a_link_takes_its_first_rel() {
        let l = parse_links(r#"<a>; REL=type; rel="next""#).next().unwrap();
        assert_eq!(l.param("rel").as_deref(), Some("type"));
    }

    /// Review finding: parameters were split at every `;`, inside quoted strings too, so a
    /// `title` holding `; rel=type` declared a relation the link does not have.
    #[test]
    fn a_semicolon_in_a_quoted_string_is_text() {
        let l = parse_links(
            r#"<https://www.w3.org/ns/lws#Container>; title="note; rel=type"; rel=describedby"#,
        )
        .next()
        .unwrap();
        assert_eq!(l.param("rel").as_deref(), Some("describedby"));
        assert_eq!(l.param("title").as_deref(), Some("note; rel=type"));
    }

    /// Quoted pairs (RFC 9110 section 5.6.4) are undone, and an escaped quote does not close
    /// the string.
    #[test]
    fn quoted_pairs_are_undone() {
        let l: Vec<_> =
            parse_links(r#"<a>; title="say \"hi\", \\ok"; rel=next, <b>; rel=up"#).collect();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].param("title").as_deref(), Some(r#"say "hi", \ok"#));
        assert_eq!(l[0].param("rel").as_deref(), Some("next"));
        assert_eq!(l[1].param("rel").as_deref(), Some("up"));
    }

    /// What is not a `link-value` ends the reading there: a quoted string never closed does not
    /// swallow the links after it into its text, a target without its `>` does not take a later
    /// link's, and nothing after either is read as a link.
    #[test]
    fn a_malformed_link_ends_the_reading() {
        let unclosed: Vec<_> =
            parse_links(r#"<a>; rel=next, <b>; title="open; rel=type, <c>; rel=up"#).collect();
        assert_eq!(unclosed.len(), 1);
        assert_eq!(unclosed[0].target, "a");
        let unbracketed: Vec<_> = parse_links("<a; rel=type, <b>; rel=up").collect();
        assert!(unbracketed.is_empty());
        let junk: Vec<_> = parse_links("<a>; rel=next junk, <b>; rel=up").collect();
        assert!(junk.is_empty());
        // Empty list elements and parameters without a value are fine.
        let l: Vec<_> = parse_links(" , <a>;rel=next ;  anchor ,, <b>").collect();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].param("anchor").as_deref(), Some(""));
    }

    /// Review finding (#6808): every link was parsed, each with its own map of owned strings,
    /// before the caps on declared links applied, so what parsing allocated was bounded only by
    /// the server's header limit. Links are now read one at a time, borrowing the header, and
    /// the check stops at its cap: a header far past the cap is refused, and parsing it takes
    /// only the links up to the cap.
    #[test]
    fn links_past_the_cap_are_not_parsed() {
        let header = vec!["<https://e/t>; rel=\"next\""; 1 << 16].join(", ");
        let mut read = 0;
        let refused = parse_links(&header)
            .inspect(|_| read += 1)
            .try_fold(0usize, |n, _| (n < MAX_DECLARED_LINKS).then_some(n + 1))
            .is_none();
        assert!(refused);
        assert_eq!(read, MAX_DECLARED_LINKS + 1);
        let mut headers = HeaderMap::new();
        headers.insert(header::LINK, header.parse().unwrap());
        let resp = check_headers(&headers).unwrap_err();
        assert_eq!(resp.status(), StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
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

    /// Issue #6863: an extension relation was keyed as spelled, so two spellings of one relation
    /// were two relations (RFC 8288 section 2.1.2 compares them case-insensitively). Created
    /// under two spellings, it is one relation, kept under the first spelling and found by a
    /// search under any.
    #[tokio::test]
    async fn an_extension_relation_is_one_relation_in_any_case() {
        let (st, _store) = test_store::state(4).await;
        let link = r#"<https://t.example/x>; rel="https://E.example/Rel", <https://t.example/y>; rel="https://e.example/REL""#;
        let r = route(
            &st,
            request(
                Method::POST,
                "/",
                &[
                    ("content-type", "text/plain"),
                    ("slug", "m"),
                    ("link", link),
                ],
                "x",
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let uri = r.headers()[header::LOCATION].to_str().unwrap().to_string();
        let path = uri.strip_prefix("http://localhost:3000").unwrap();
        let r = route(
            &st,
            request(Method::GET, &format!("{path}{META_SUFFIX}"), &[], ""),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let doc = test_store::body_json(r).await;
        let entry = doc["linkset"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["anchor"] == uri.as_str())
            .unwrap()
            .clone();
        let hrefs: Vec<&str> = entry["https://E.example/Rel"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["href"].as_str())
            .collect();
        assert_eq!(hrefs, ["https://t.example/x", "https://t.example/y"]);
        assert!(entry.get("https://e.example/REL").is_none(), "{entry}");
        for (rel, target) in [
            ("https://e.example/rel", "https://t.example/x"),
            ("https://E.EXAMPLE/REL", "https://t.example/y"),
            ("https://E.example/Rel", "https://t.example/y"),
        ] {
            let filter = json!({ rel: [target] }).to_string();
            let r = route(
                &st,
                request(
                    Method::from_bytes(b"QUERY").unwrap(),
                    TYPE_SEARCH_PATH,
                    &[("content-type", index::LWS_QUERY)],
                    &filter,
                ),
            )
            .await;
            assert_eq!(r.status(), StatusCode::OK, "{filter}");
            let doc = test_store::body_json(r).await;
            let ids: Vec<&str> = doc["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|i| i["id"].as_str())
                .collect();
            assert_eq!(ids, [uri.as_str()], "{filter}");
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
}
