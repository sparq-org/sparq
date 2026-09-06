// AUTHORED-BY Claude Opus 4.8
//! The LDP request handlers — GET / HEAD / PUT / POST / DELETE / PATCH over the [`Store`] seam.
//!
//! These are the axum handlers over the [`Store`] seam. They stay thin: target parsing
//! ([`crate::ldp::target`]), content classification + negotiation ([`crate::ldp::content`]),
//! precondition evaluation ([`crate::ldp::conditional`]), range computation ([`crate::ldp::range`]),
//! and the N3-Patch engine ([`crate::ldp::patch`]) are pure modules; the handler is the HTTP glue +
//! the store call.
//!
//! ## The authorization seam (real per-resource WAC)
//!
//! Each handler runs the local in-Rust Web Access Control engine ([`crate::authz`]) BEFORE touching
//! storage: the HTTP method + target maps to a required [`AccessMode`]
//! ([`mode_for_operation`]); the [`WacAuthorizer`] resolves the effective `.acl` (the target's OWN
//! `acl:accessTo` ACL, else the nearest ancestor's `acl:default`, child→root, fail-closed) and returns
//! a [`Decision`]:
//!
//! - **`Allow`** — the operation proceeds; on a permitted GET/HEAD the read response carries the
//!   `WAC-Allow` header (the requester's + the public's effective modes).
//! - **`Unauthenticated`** (the requester is anonymous and auth could plausibly grant) — **401** +
//!   `WWW-Authenticate` challenge, so the client obtains a token.
//! - **`Forbidden`** (authenticated but not authorized) — **403**.
//!
//! Reading or writing a resource's OWN `.acl` requires `acl:Control` (encoded by
//! [`mode_for_operation`]). Public-readable resources are exactly those whose effective ACL grants
//! `foaf:Agent acl:Read` — the conformance seed sets up the WebID-profile + pod-root ACLs (see
//! [`crate::seed`]). Authorization runs BEFORE the existence check, so a permitted read of a missing
//! resource is a 404 while an UNauthorized/anonymous read of the same is a 403/401 (no existence leak).

use std::sync::{Arc, LazyLock};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;

use oxrdf::{NamedNode, Triple};

use crate::acl_cache::AclCache;
use crate::auth::VerifiedToken;
use crate::authz::wac::{Decision, ReadDecision, WacAuthorizer};
use crate::authz::wac_allow::wac_allow_header;
use crate::authz::{mode_for_operation, AccessMode};
use crate::error::ServerError;
use crate::ldp::conditional::{self, evaluate as eval_preconditions};
use crate::ldp::content::{
    classify, negotiate_accept_with_profile, parse_to_triples, serialize_triples,
    serialize_triples_negotiated, validate_rdf, RdfFormat,
};
use crate::ldp::patch::{
    apply_patch, classify_patch_media_type, parse_n3_patch, parse_sparql_update, PatchKind,
};
use crate::ldp::range::{self, RangeOutcome};
use crate::ldp::target::{parse_target, LdpTarget};
#[cfg(not(target_arch = "wasm32"))]
use crate::notifications::ws::link_headers;
#[cfg(not(target_arch = "wasm32"))]
use crate::notifications::{ActivityType, NotificationHub};
use crate::store::{DeleteOutcome, Resource, ResourceMeta, Store};

/// LDP/RDF vocabulary IRIs used to synthesise a container's `ldp:contains` representation.
const RDF_TYPE_IRI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const LDP_RESOURCE_IRI: &str = "http://www.w3.org/ns/ldp#Resource";
const LDP_CONTAINER_IRI: &str = "http://www.w3.org/ns/ldp#Container";
const LDP_BASIC_CONTAINER_IRI: &str = "http://www.w3.org/ns/ldp#BasicContainer";
const LDP_CONTAINS_IRI: &str = "http://www.w3.org/ns/ldp#contains";

/// The five vocabulary IRIs above are server CONSTANTS — fixed strings, RFC-3987-valid by
/// construction. Validating them through `NamedNode::new` (oxiri RFC-3987 parse) on every container
/// render is pure waste: the same five strings re-parse identically every time. Validate each ONCE
/// per process via `new_unchecked` behind a `LazyLock` and clone the cached `NamedNode` on the hot
/// path. `new_unchecked` is sound here precisely because the inputs are compile-time constants; a
/// `debug_assert!` re-validates each in debug builds so a typo'd constant fails a test, never ships.
static RDF_TYPE_NODE: LazyLock<NamedNode> = LazyLock::new(|| unchecked_const_iri(RDF_TYPE_IRI));
static LDP_RESOURCE_NODE: LazyLock<NamedNode> =
    LazyLock::new(|| unchecked_const_iri(LDP_RESOURCE_IRI));
static LDP_CONTAINER_NODE: LazyLock<NamedNode> =
    LazyLock::new(|| unchecked_const_iri(LDP_CONTAINER_IRI));
static LDP_BASIC_CONTAINER_NODE: LazyLock<NamedNode> =
    LazyLock::new(|| unchecked_const_iri(LDP_BASIC_CONTAINER_IRI));
static LDP_CONTAINS_NODE: LazyLock<NamedNode> =
    LazyLock::new(|| unchecked_const_iri(LDP_CONTAINS_IRI));

// --- request-INVARIANT response header values (perf round-C, MALLOC reduction) ------------------
//
// The `Link: <type>; rel="type"` advertisement lines and the method-advertisement lines are
// REQUEST-INVARIANT: the full header string is a COMPILE-TIME constant (the IRI/value is fixed for
// every resource of a given shape). The prior code re-formatted them with `format!` and re-validated
// them through `HeaderValue::from_str` (a heap allocation + UTF-8/structure validation) on EVERY
// response. These `from_static` `HeaderValue`s are built from a `&'static str` whose bytes are known
// valid at compile time, so they allocate NOTHING and clone cheaply (a `HeaderValue::from_static`
// holds a `Bytes` pointing at the static — its `clone` is a refcount bump, not a copy). The set
// emitted is byte-for-byte the SAME header lines as before.
//
// SECURITY NOTE (the perf-round-C trap): NONE of these carry a per-target or per-WebID value — they
// are the same for every resource of their shape, so interning them leaks nothing. The per-resource
// `.acl` Link (derived from the target IRI) and the per-server discovery Links (derived from
// `base_url`) are DELIBERATELY NOT here — the `.acl` link stays computed per request in
// `add_acl_link`, and the discovery links are precomputed per `LdpState` (per server instance), so
// no resource's pointer can leak onto another's response.
const LINK_TYPE_LDP_RESOURCE: &str = "<http://www.w3.org/ns/ldp#Resource>; rel=\"type\"";
const LINK_TYPE_LDP_CONTAINER: &str = "<http://www.w3.org/ns/ldp#Container>; rel=\"type\"";
const LINK_TYPE_LDP_BASIC_CONTAINER: &str =
    "<http://www.w3.org/ns/ldp#BasicContainer>; rel=\"type\"";
const LINK_TYPE_PIM_STORAGE: &str = "<http://www.w3.org/ns/pim/space#Storage>; rel=\"type\"";

static HV_LINK_TYPE_LDP_RESOURCE: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static(LINK_TYPE_LDP_RESOURCE));
static HV_LINK_TYPE_LDP_CONTAINER: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static(LINK_TYPE_LDP_CONTAINER));
static HV_LINK_TYPE_LDP_BASIC_CONTAINER: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static(LINK_TYPE_LDP_BASIC_CONTAINER));
static HV_LINK_TYPE_PIM_STORAGE: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static(LINK_TYPE_PIM_STORAGE));

// The method-advertisement values (`Allow` / `Accept-Post` / `Accept-Patch`) are likewise
// REQUEST-INVARIANT compile-time constants — the SAME on every response of a given shape — so they
// are interned once via `from_static` instead of re-validated through `HeaderValue::from_str` per
// response. `Accept-Post` is emitted only on a container (unchanged). Byte-identical output.
static HV_ALLOW: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static("OPTIONS, HEAD, GET, PUT, POST, DELETE, PATCH"));
static HV_ACCEPT_POST: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static("text/turtle, application/ld+json"));
static HV_ACCEPT_PATCH: LazyLock<HeaderValue> =
    LazyLock::new(|| HeaderValue::from_static("text/n3, application/sparql-update"));

/// Build a `NamedNode` from a COMPILE-TIME-CONSTANT IRI without the per-call RFC-3987 re-parse.
/// Confined to the five `*_IRI` server constants above (validated once, at first use). The
/// `debug_assert!` re-runs the checked parse in debug/test builds so a malformed constant is caught
/// by the test suite rather than silently producing an invalid node.
fn unchecked_const_iri(iri: &str) -> NamedNode {
    debug_assert!(
        NamedNode::new(iri).is_ok(),
        "server-constant IRI must be RFC-3987 valid: {iri}"
    );
    NamedNode::new_unchecked(iri)
}

/// Shared state for the LDP handlers: the store + the server's public base URL + the notification hub.
///
/// The hub is the SINGLE emit seam: after a successful mutation the handler calls
/// `NotificationHub::notify` (the only notification coupling in the write path — no handler
/// refactor). The hub is cheap to clone (an `Arc` inside) and shared with the notification routes.
pub struct LdpState<S: Store> {
    pub store: S,
    /// [GPT-5.6] sq-r1ei8: request-local snapshot barrier shared by LDP writes
    /// and the optional SPARQL endpoint. Queries hold a read guard while they
    /// assemble and evaluate; every mutation holds a write guard, so one server
    /// instance cannot expose a dataset assembled across an interleaved write.
    /// Compiled out with `sparql-endpoint`, including its dependency.
    #[cfg(feature = "sparql-endpoint")]
    sparql_snapshot: async_lock::RwLock<()>,
    /// The server's public base URL. PRIVATE on purpose: [`discovery_link_values`] is a cache derived
    /// from it at construction, so a post-construction mutation of `base_url` would desync the two
    /// (the cached discovery `Link` headers would advertise the OLD storage-description URL while
    /// request parsing/type links used the new one). Read it via [`base_url`](Self::base_url); the
    /// (sole, router-assembly-time) writer is [`set_base_url`](Self::set_base_url), which rebuilds the
    /// cache atomically. Internal `self.base_url` reads within this module are fine (the field stays
    /// in scope of the impl).
    base_url: String,
    #[cfg(not(target_arch = "wasm32"))]
    pub notifications: NotificationHub,
    /// The `WWW-Authenticate` challenge to emit on a 401 for an anonymous request to a resource that
    /// requires authentication. Populated from the `AuthContext` at router
    /// assembly ([`AppState::new`](crate::app::AppState::new)) so the LDP layer can answer 401 +
    /// challenge WITHOUT a handle to the verifier; a default Bearer/DPoP challenge is used if unset.
    pub www_authenticate: String,
    /// The per-instance ETag-keyed parsed-ACL cache (read-path optimisation #3). Shared across all
    /// requests (it lives in the server-lifetime `Arc<LdpState>`), so a hot resource's UNCHANGED `.acl`
    /// is parsed once and reused — keyed by `(acl-iri, etag)`, never authoritative (see
    /// [`crate::acl_cache`]). Default-on at [`AclCache::new`]`(`[`DEFAULT_ACL_CACHE_CAPACITY`](crate::acl_cache::DEFAULT_ACL_CACHE_CAPACITY)`)`;
    /// `SOLID_SERVER_ACL_CACHE_CAPACITY=0` ([`AclCache::disabled`]) yields byte-identical pre-cache
    /// behaviour. Configured at router assembly via [`set_acl_cache`](Self::set_acl_cache).
    pub acl_cache: AclCache,
    /// [SONNET-4.6] sq-elg47: the opt-in ODRL policy gate consulted AFTER the WAC decision on the
    /// read/query path (deny-overrides, permit-extends — see [`crate::authz::odrl`]). `None` (the
    /// default) is behaviour-identical to the feature being off; attached at router assembly via
    /// [`set_odrl_gate`](Self::set_odrl_gate).
    #[cfg(all(feature = "odrl-authz", not(target_arch = "wasm32")))]
    pub odrl_gate: Option<std::sync::Arc<dyn crate::authz::odrl::OdrlGate>>,
    /// The notification-discovery `Link` header VALUES (`describedby` + `solid:storageDescription`,
    /// both → the storage-description doc), PRECOMPUTED ONCE from `base_url` at construction.
    ///
    /// These depend ONLY on the server's `base_url` (a per-instance constant), so re-deriving them per
    /// request — `link_headers(base_url)` allocating a `Vec` + two `format!` `String`s, then
    /// `format!("<{target}>; rel=\"{rel}\"")` + `HeaderValue::from_str` per pair — was pure
    /// per-request waste on the read hot path (the MALLOC band the round-4 profile flagged). Caching
    /// the finished `HeaderValue`s makes `add_discovery_links` a couple of refcount-bump appends. The
    /// emitted header lines are byte-for-byte identical to the prior per-request formatting.
    ///
    /// SECURITY: these are derived from `base_url` ONLY — NEVER from a request target or WebID — so
    /// they are the same for every resource and leak no per-resource pointer (cf. the per-request
    /// `.acl` Link, which is intentionally NOT cached — see `add_acl_link`).
    discovery_link_values: Vec<HeaderValue>,
}

/// Precompute the discovery `Link` header VALUES for a given server `base_url`. Mirrors the prior
/// `add_discovery_links` formatting EXACTLY (`<{target}>; rel="{rel}"` per `link_headers` pair,
/// skipping any value that cannot be header-encoded), so the emitted lines are byte-identical — only
/// computed once per server instead of once per request.
#[cfg(not(target_arch = "wasm32"))]
fn build_discovery_link_values(base_url: &str) -> Vec<HeaderValue> {
    link_headers(base_url)
        .into_iter()
        .filter_map(|(rel, target)| {
            HeaderValue::from_str(&format!("<{target}>; rel=\"{rel}\"")).ok()
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn build_discovery_link_values(_base_url: &str) -> Vec<HeaderValue> {
    Vec::new()
}

/// The fallback `WWW-Authenticate` challenge used when no verifier-derived one was injected (e.g. a
/// test that builds an `LdpState` directly). The verifier-derived challenge additionally names the
/// trusted issuer(s); this fallback is a minimal, spec-shaped DPoP challenge.
const DEFAULT_WWW_AUTHENTICATE: &str = "DPoP error=\"invalid_token\", scope=\"webid\"";

impl<S: Store> LdpState<S> {
    /// Build an LDP state with a fresh, isolated notification hub.
    pub fn new(store: S, base_url: impl Into<String>) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::with_hub(store, base_url, NotificationHub::new())
        }
        #[cfg(target_arch = "wasm32")]
        {
            let base_url = base_url.into();
            let discovery_link_values = build_discovery_link_values(&base_url);
            Self {
                store,
                #[cfg(feature = "sparql-endpoint")]
                sparql_snapshot: async_lock::RwLock::new(()),
                base_url,
                www_authenticate: DEFAULT_WWW_AUTHENTICATE.to_string(),
                acl_cache: AclCache::new(crate::acl_cache::DEFAULT_ACL_CACHE_CAPACITY),
                discovery_link_values,
            }
        }
    }

    /// Build an LDP state sharing an EXISTING notification hub (so the LDP emit path and the
    /// notification receive routes register against the same registry).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_hub(store: S, base_url: impl Into<String>, notifications: NotificationHub) -> Self {
        let base_url = base_url.into();
        // Precompute the request-invariant discovery `Link` values ONCE from `base_url` (see the
        // field doc) so the read path never re-formats them per request.
        let discovery_link_values = build_discovery_link_values(&base_url);
        Self {
            store,
            #[cfg(feature = "sparql-endpoint")]
            sparql_snapshot: async_lock::RwLock::new(()),
            base_url,
            notifications,
            www_authenticate: DEFAULT_WWW_AUTHENTICATE.to_string(),
            // Default-on: the ACL cache is enabled at the default capacity. `main.rs` overrides this
            // from `SOLID_SERVER_ACL_CACHE_CAPACITY` at router assembly (`=0` ⇒ disabled).
            acl_cache: AclCache::new(crate::acl_cache::DEFAULT_ACL_CACHE_CAPACITY),
            #[cfg(feature = "odrl-authz")]
            odrl_gate: None,
            discovery_link_values,
        }
    }

    /// Set the `WWW-Authenticate` challenge emitted on a 401 (the verifier-derived one). Called by
    /// [`AppState::new`](crate::app::AppState::new) so the LDP layer's anonymous-401 names the same
    /// issuer(s)/algs as every other challenge.
    pub fn set_www_authenticate(&mut self, challenge: impl Into<String>) {
        self.www_authenticate = challenge.into();
    }

    /// Replace the ACL cache (called by `main.rs` at router assembly to apply the operator-configured
    /// capacity / disable it). The default constructors already enable it at the default capacity.
    pub fn set_acl_cache(&mut self, acl_cache: AclCache) {
        self.acl_cache = acl_cache;
    }

    /// [SONNET-4.6] sq-elg47: attach the ODRL read/query gate (router assembly / embedder seam).
    /// The default (no gate) is behaviour-identical to the `odrl-authz` feature being off.
    #[cfg(all(feature = "odrl-authz", not(target_arch = "wasm32")))]
    pub fn set_odrl_gate(&mut self, gate: std::sync::Arc<dyn crate::authz::odrl::OdrlGate>) {
        self.odrl_gate = Some(gate);
    }

    /// The server's public base URL. The read accessor for the (now-private) `base_url` field — used
    /// by router assembly ([`AppState`](crate::app)) and any external consumer.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Hold the server-local dataset stable while the SPARQL handler assembles
    /// and evaluates a query. LDP reads do not need the barrier: their existing
    /// per-resource read plan is already internally consistent.
    #[cfg(feature = "sparql-endpoint")]
    pub(crate) async fn sparql_snapshot_read(&self) -> async_lock::RwLockReadGuard<'_, ()> {
        self.sparql_snapshot.read().await
    }

    /// Serialize an LDP mutation against in-flight SPARQL dataset snapshots.
    #[cfg(feature = "sparql-endpoint")]
    async fn sparql_snapshot_write(&self) -> async_lock::RwLockWriteGuard<'_, ()> {
        self.sparql_snapshot.write().await
    }

    /// Replace the base URL, REBUILDING the derived discovery-link cache so the two never desync.
    ///
    /// This is the ONLY sanctioned writer of `base_url` (the field is private precisely so a caller
    /// cannot mutate it WITHOUT rebuilding `discovery_link_values` — see the field doc). Provided
    /// for completeness + the no-stale-cache invariant; in practice `base_url` is fixed at
    /// construction and never reset, but if a future router-assembly step does reset it, the discovery
    /// `Link` headers stay consistent with it.
    pub fn set_base_url(&mut self, base_url: impl Into<String>) {
        self.base_url = base_url.into();
        self.discovery_link_values = build_discovery_link_values(&self.base_url);
    }

    /// Invalidate the cached parse of an ACL resource after a successful WRITE / DELETE of it (belt-and-
    /// braces — the `(acl-iri, etag)` gate already prevents serving a rotated ACL stale, but freeing the
    /// slot on a mutation is cheap and makes a delete take effect immediately). A no-op for a
    /// non-`.acl` target or a disabled cache.
    fn invalidate_acl_if_acl(&self, target_iri: &str) {
        if crate::authz::is_acl_resource(target_iri) {
            self.acl_cache.invalidate(target_iri);
        }
    }

    /// Build the 401 `Unauthorized` error (with the cached challenge) for an anonymous request to a
    /// resource that requires authentication.
    fn unauthenticated(&self) -> ServerError {
        ServerError::Unauthorized {
            status: 401,
            message: "Authentication required for this resource.".to_string(),
            www_authenticate: self.www_authenticate.clone(),
        }
    }

    /// Run Web Access Control for `target` + the `method`-derived required mode against `token`.
    ///
    /// On a permitted operation returns the FULL set of modes the requester holds over the target (so
    /// a GET/HEAD can build `WAC-Allow` without re-walking the ACL hierarchy). On a denial returns the
    /// spec-shaped error: a 401 + `WWW-Authenticate` when the requester is anonymous (so the client
    /// authenticates), a 403 when authenticated-but-unauthorized.
    async fn authorize(
        &self,
        method: &str,
        target: &LdpTarget,
        token: &VerifiedToken,
        origin: Option<&str>,
    ) -> Result<std::collections::BTreeSet<AccessMode>, ServerError> {
        let required = mode_for_operation(method, &target.iri, target.is_container);
        self.authorize_mode(target, required, token, origin).await
    }

    /// Single-pass READ authorization over ONE combined read-plan round-trip (read-2 —
    /// `research/lws-design-records.md` §7; RSS `docs/design/backend-read-path.md` §3.1), still
    /// deriving the decision AND both `WAC-Allow` audiences from ONE effective-ACL resolution
    /// (Optimization #2).
    ///
    /// The per-read metadata chain — the target's own metadata plus the presence/etag of EVERY ACL
    /// candidate on its resolution chain — is fetched in ONE [`Store::read_plan`] call (one
    /// combined SPARQL query on the live backend, replacing the previous k+2 sequential queries);
    /// the ACL walk then runs IN MEMORY over those rows with semantics identical to the sequential
    /// resolver ([`WacAuthorizer::authorize_read_planned`] — differentially tested against
    /// [`WacAuthorizer::authorize_read`] over the full WAC matrix). The plan is
    /// principal-independent metadata and precedes authorization (design §3.6); the target's BYTES
    /// are never fetched here — the caller fetches them only after an Allow (invariant 5, no
    /// speculative byte fetch).
    ///
    /// The required mode is the `method`-derived read mode, overridden to [`AccessMode::Control`]
    /// for an `.acl` target (managing access rules is always Control) — IDENTICAL to
    /// [`authorize`](Self::authorize) / [`authorize_mode`](Self::authorize_mode). On a permitted
    /// read returns the [`EffectivePermissions`] for `WAC-Allow` PLUS the target's metadata from
    /// the same plan (`None` ⇒ the caller's 404, decided AFTER authorization exactly as before);
    /// on a denial the SAME spec error (401 + challenge when anonymous, 403 when
    /// authenticated-but-unauthorized).
    async fn authorize_read(
        &self,
        method: &str,
        target: &LdpTarget,
        token: &VerifiedToken,
        origin: Option<&str>,
    ) -> Result<(crate::authz::EffectivePermissions, Option<ResourceMeta>), ServerError> {
        // The required read mode, with the `.acl`→Control override (an `.acl` is governed by Control
        // regardless of the operation) — matching `authorize`/`authorize_mode` exactly.
        let required = if crate::authz::is_acl_resource(&target.iri) {
            AccessMode::Control
        } else {
            mode_for_operation(method, &target.iri, target.is_container)
        };
        let wac = WacAuthorizer::with_cache(&self.store, &self.base_url, &self.acl_cache);
        // ONE combined round-trip: the target row is the RAW target (the bytes to serve); the ACL
        // candidates derive from the PROTECTED resource (the design's two IRI roles).
        let candidates = wac.read_plan_candidates(&target.iri);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        let plan = self.store.read_plan(&target.iri, &acl_iris).await?;
        let decision = wac
            .authorize_read_planned(
                required,
                token.web_id.as_deref(),
                origin,
                &candidates,
                &plan.acls,
            )
            .await?;
        // [SONNET-4.6] sq-elg47: compose the opt-in ODRL gate's verdict AFTER the WAC decision
        // (deny-overrides, permit-extends — see `crate::authz::odrl`). Compiled out entirely when
        // the `odrl-authz` feature is off; an unattached gate leaves the decision untouched.
        #[cfg(all(feature = "odrl-authz", not(target_arch = "wasm32")))]
        let decision =
            self.compose_odrl_read(&target.iri, required, token.web_id.as_deref(), decision);
        match decision {
            ReadDecision::Allow(perms) => Ok((perms, plan.target)),
            ReadDecision::Unauthenticated => Err(self.unauthenticated()),
            ReadDecision::Forbidden => Err(ServerError::Forbidden),
        }
    }

    /// [SONNET-4.6] sq-elg47: compose the attached ODRL gate's verdict with the WAC read decision
    /// for `target_iri` — deny-overrides (a [`OdrlVerdict::Deny`](crate::authz::odrl::OdrlVerdict)
    /// beats any static WAC grant, keeping the 401-vs-403 split on `web_id`), permit-extends (a
    /// `Permit` admits the read even where WAC grants nothing, ONLY when the required mode is
    /// `Read` — it never widens the `.acl`⇒Control requirement; the resulting advertisement is
    /// read-scoped, `user=read` with no public modes — the fail-closed direction, matching the
    /// server ODRL lane's read-scoped advertisement rule). `NotApplicable` — or no gate attached —
    /// leaves the WAC decision unchanged.
    #[cfg(all(feature = "odrl-authz", not(target_arch = "wasm32")))]
    fn compose_odrl_read(
        &self,
        target_iri: &str,
        required: AccessMode,
        web_id: Option<&str>,
        wac: ReadDecision,
    ) -> ReadDecision {
        use crate::authz::odrl::OdrlVerdict;
        let Some(gate) = self.odrl_gate.as_deref() else {
            return wac;
        };
        match gate.decide_read(target_iri, web_id) {
            OdrlVerdict::NotApplicable => wac,
            OdrlVerdict::Deny => {
                if web_id.is_none() {
                    ReadDecision::Unauthenticated
                } else {
                    ReadDecision::Forbidden
                }
            }
            OdrlVerdict::Permit => match wac {
                allowed @ ReadDecision::Allow(_) => allowed,
                _ if required == AccessMode::Read => {
                    ReadDecision::Allow(crate::authz::EffectivePermissions {
                        user: [AccessMode::Read].into_iter().collect(),
                        public: std::collections::BTreeSet::new(),
                    })
                }
                denied => denied,
            },
        }
    }

    /// Run Web Access Control for `target` with an EXPLICIT required mode (used by PATCH, whose mode
    /// depends on the patch CONTENT — an insert-only patch needs only `acl:Append`, a patch with any
    /// delete needs `acl:Write`). For an `.acl` target the required mode is overridden to
    /// [`AccessMode::Control`] regardless (managing access rules is always the Control privilege).
    ///
    /// write-2: the ACL walk is PLANNED — one combined [`Store::read_plan`] round-trip replaces the
    /// sequential k+1 per-candidate probes (see [`authorize_planned_iri`](Self::authorize_planned_iri)).
    async fn authorize_mode(
        &self,
        target: &LdpTarget,
        required: AccessMode,
        token: &VerifiedToken,
        origin: Option<&str>,
    ) -> Result<std::collections::BTreeSet<AccessMode>, ServerError> {
        // An `.acl` resource is governed by Control regardless of the operation/content.
        let required = if crate::authz::is_acl_resource(&target.iri) {
            AccessMode::Control
        } else {
            required
        };
        match self
            .authorize_planned_iri(&target.iri, required, token, origin)
            .await?
        {
            Decision::Allow(modes) => Ok(modes),
            Decision::Unauthenticated => Err(self.unauthenticated()),
            Decision::Forbidden => Err(ServerError::Forbidden),
        }
    }

    /// Run WAC for an EXPLICIT (`target_iri`, mode), where `target_iri` may be a synthetic container
    /// IRI (e.g. the parent of the resource being created/deleted, which is itself a valid container
    /// path). Returns the granted modes on Allow, or the spec 401/403 on deny.
    ///
    /// write-2: planned walk — see [`authorize_planned_iri`](Self::authorize_planned_iri).
    async fn authorize_iri(
        &self,
        target_iri: &str,
        required: AccessMode,
        token: &VerifiedToken,
        origin: Option<&str>,
    ) -> Result<(), ServerError> {
        match self
            .authorize_planned_iri(target_iri, required, token, origin)
            .await?
        {
            Decision::Allow(_) => Ok(()),
            Decision::Unauthenticated => Err(self.unauthenticated()),
            Decision::Forbidden => Err(ServerError::Forbidden),
        }
    }

    /// The shared write-path PLANNED authorization core (write-2 — `research/lws-design-records.md`
    /// §7; RSS `docs/design/backend-read-path.md` §3.1, applied to the write verbs): derive the
    /// ACL-candidate chain, fetch every candidate's presence/etag in ONE combined
    /// [`Store::read_plan`] round-trip (replacing the sequential k+1 per-candidate `meta` probes
    /// the [`WacAuthorizer::authorize`] walk pays), then decide via
    /// [`WacAuthorizer::authorize_planned`] — whose in-memory walk + LIVE found-ACL re-confirm is
    /// differentially tested bit-for-bit against the sequential walk for every [`AccessMode`].
    ///
    /// The plan is principal-independent METADATA (existence/etag rows only — no resource bytes,
    /// no grants), so fetching it before the decision leaks nothing to the client; the decision
    /// itself — including the fail-closed delete-after-plan re-confirm and the 401-vs-403 split —
    /// is unchanged from the sequential path.
    ///
    /// The plan's TARGET-row slot is deliberately the FIRST ACL CANDIDATE, NOT the raw target
    /// (unlike the read path, which reuses the target row for its 404): a write authorization
    /// needs ONLY the ACL rows, and the sequential walk it replaces never touched the TARGET's
    /// index record — so the planned decision must not either. Passing the raw target would make
    /// the authorization fail on a target-record backend fault, turning the UNIFORM 401/403
    /// denial an unauthorized caller must see into a 500 existence/state ORACLE (pinned by the
    /// `patch_*_faulting_target_read_*` tests). With the first candidate in the slot the plan's
    /// `VALUES` set is exactly the candidate ACLs (the slot IRI is already candidate 0 — no extra
    /// row on the wire), its target row is ignored, and an ACL-probe fault still fails the plan
    /// (fail-closed) exactly as the sequential walk's ACL probes did.
    async fn authorize_planned_iri(
        &self,
        target_iri: &str,
        required: AccessMode,
        token: &VerifiedToken,
        origin: Option<&str>,
    ) -> Result<Decision, ServerError> {
        let wac = WacAuthorizer::with_cache(&self.store, &self.base_url, &self.acl_cache);
        let candidates = wac.read_plan_candidates(target_iri);
        let acl_iris: Vec<String> = candidates.iter().map(|c| c.acl.clone()).collect();
        // candidates[0] always exists (the chain starts at the protected resource's own ACL).
        let plan = self.store.read_plan(&acl_iris[0], &acl_iris).await?;
        wac.authorize_planned(
            required,
            token.web_id.as_deref(),
            origin,
            &candidates,
            &plan.acls,
        )
        .await
    }

    /// WAC container-modification authorization for a CREATE (the missing half of the WAC create rule).
    ///
    /// Creating a resource — and materialising any missing intermediate container via
    /// [`ensure_ancestor_containers`] — MUTATES the `ldp:contains` membership of the nearest EXISTING
    /// ancestor container. Per Web Access Control this requires `acl:Append` (which `acl:Write`
    /// subsumes) ON THAT CONTAINER via its own `acl:accessTo` scope — the "creating a resource requires
    /// write/append access to the containing container" rule (WAC spec §"Modes of access"; CSS's create
    /// authorization; the TS-sibling `authorizeCreation`). This is enforced IN ADDITION to the target's
    /// own effective-ACL Write/Append that the create paths authorize first (which the existence-non-
    /// disclosure V1/V3 closure requires), and makes CREATE **symmetric with DELETE** (whose parent-Write
    /// check gates containment shrink). Without it, an `acl:default`-only Write grant — or a
    /// Control-holder-pre-provisioned target `.acl` — would let an agent with NO mode on the container
    /// create members / intermediate containers in it (a privilege-escalation container-write bypass).
    ///
    /// The container-modification right is authorized via the PLANNED walk
    /// ([`authorize_iri`](Self::authorize_iri) → [`authorize_planned_iri`](Self::authorize_planned_iri),
    /// write-2), so the container ACL chain is resolved in one combined round-trip like every other
    /// write-verb authorization.
    ///
    /// An `.acl` auxiliary is NOT a contained child (it carries no `ldp:contains` edge — see the create
    /// paths), so authoring one mutates no containment and is exempt (mirroring DELETE, which skips its
    /// parent-Write check for an `.acl`). The check runs against the nearest EXISTING ancestor via
    /// [`nearest_existing_container`](Self::nearest_existing_container); when NONE exists (an
    /// unprovisioned store whose root container is absent) there is no container whose membership is
    /// being mutated, so the check is skipped — exactly as DELETE skips its parent check on a `None`
    /// nearest-parent. The access decision reads only ancestor `.acl` resources (never the target).
    async fn authorize_container_modification(
        &self,
        target_iri: &str,
        token: &VerifiedToken,
        origin: Option<&str>,
    ) -> Result<(), ServerError> {
        // An `.acl` auxiliary is not a contained child — no container-modification right is required.
        if crate::authz::is_acl_resource(target_iri) {
            return Ok(());
        }
        if let Some(container) = self.nearest_existing_container(target_iri).await? {
            self.authorize_iri(&container, AccessMode::Append, token, origin)
                .await?;
        }
        Ok(())
    }

    /// EXISTENCE-NON-DISCLOSURE — the **V4** conditional-channel closure
    /// (`research/lws-design-records.md` §6; RSS `decisions/0003`).
    ///
    /// A conditional precondition (`If-Match` / `If-None-Match`) on a mutating request is evaluated
    /// against the target's CURRENT ETag, which is a CONTENT-derived (for a document) or
    /// MEMBERSHIP-derived (for a container) validator. Its 412-vs-2xx outcome — and any `ETag` the
    /// write response then carries — therefore leak whether the target exists AND a fingerprint of a
    /// representation the requester may NOT be entitled to read. A `Write`-without-`Read` holder doing
    /// `PUT … If-Match: "x"` could thus probe existence (412 if present-and-mismatched, 2xx-then-ETag if
    /// present-and-matched, 412 if absent under `If-Match`) and learn the content/membership ETag of a
    /// body it cannot GET.
    ///
    /// Closure: treat a content/membership-derived validator as REQUIRING the mode that governs READING
    /// the target's representation — `acl:Read` for a normal resource, but `acl:Control` for an `.acl`
    /// target (reading an `.acl`'s representation is itself a Control operation; `Control` does NOT imply
    /// `Read`, so the read-mode for an `.acl` is Control, not Read — else a Control-only holder, who IS
    /// entitled to the `.acl`'s ETag, would be wrongly denied a conditional `.acl` write). When the
    /// request carries ANY conditional precondition AND the (already-authorized) requester's granted
    /// modes do NOT include that read-mode, return the requester's DENIAL code (401 anonymous / 403
    /// authenticated) INSTEAD of evaluating the precondition — so the conditional outcome reveals
    /// nothing. A requester WITHOUT a conditional header is unaffected (no validator is consulted on
    /// their path), and a requester who holds the read-mode keeps full conditional semantics. `granted`
    /// is the mode set the write authorization already returned (no extra ACL resolution).
    fn guard_conditional_requires_read(
        &self,
        target_iri: &str,
        headers: &HeaderMap,
        granted: &std::collections::BTreeSet<AccessMode>,
        token: &VerifiedToken,
    ) -> Result<(), ServerError> {
        // Only a CONCRETE entity-tag validator gates on Read. A bare `*` (`If-None-Match: *` safe-create
        // / `If-Match: *` lost-update guard) carries NO content-derived ETag fingerprint — it tests only
        // EXISTENCE, which a holder of the operation's required mode is already entitled to per this
        // module's own invariant (and already learns from the unconditional 201-vs-204 write split). So a
        // bare `*` is EXEMPT from this Read-gate: a `Write`-without-`Read` holder keeps the spec-
        // recommended conditional safe-create / lost-update guards with ZERO non-disclosure loss. A
        // QUOTED validator still leaks a content/membership ETag (and the response then carries the
        // target's real ETag), so it still requires the read-mode. (This closed the V4-over-broad
        // finding: bare `*` was previously 403'd for a Write-without-Read holder, breaking the standard
        // Inrupt `PUT … If-None-Match: *` create pattern for no non-disclosure gain.)
        let has_etag_conditional = Self::conditional_carries_etag(headers, header::IF_MATCH)
            || Self::conditional_carries_etag(headers, header::IF_NONE_MATCH);
        // The mode that governs reading THIS target's representation: Control for an `.acl`, else Read.
        let read_mode = if crate::authz::is_acl_resource(target_iri) {
            AccessMode::Control
        } else {
            AccessMode::Read
        };
        if has_etag_conditional && !granted.contains(&read_mode) {
            return Err(if token.web_id.is_none() {
                self.unauthenticated()
            } else {
                ServerError::Forbidden
            });
        }
        Ok(())
    }

    /// EXISTENCE-NON-DISCLOSURE — the **V6** POST descendant-existence closure
    /// (`research/lws-design-records.md` §6; RSS `decisions/0003`).
    ///
    /// A POST authorizes `acl:Append` on the target container. For a MISSING target (a not-yet-existing
    /// sub-container, or a reserved non-container path), that target has no own `.acl`, so the required
    /// `acl:Append` is satisfied via the target's INHERITED `acl:default` (from an ancestor container).
    /// The POST handler then branches on existence — a 404 for a missing container, or 404/405 for a
    /// non-container — while an EXISTING sibling that carries its OWN restrictive `.acl` denying the
    /// requester is a 403 at authorization (its own ACL overrides the inherited default). That
    /// 403-vs-404/405 split is an existence oracle: an agent holding `acl:default acl:Append` over a
    /// subtree (the realistic "drop a file anywhere under `/c/`" grant) — but NO `acl:Read` — can name a
    /// specific descendant and learn whether it exists (403 ⇒ exists-and-locked, 404/405 ⇒ free) even
    /// for descendants it may not access. The verifier execution-proved this on PR #3.
    ///
    /// Closure (mirrors the **V4** conditional-channel Read-gate): the POST existence BRANCH (the
    /// 404/405 status that discloses whether the named target exists) is an existence disclosure and so
    /// REQUIRES the target's READ-mode — `acl:Read` for a normal resource, but `acl:Control` for an `.acl`
    /// target (reading an `.acl`'s representation/existence is itself a Control operation; `Control` does
    /// NOT imply `Read`, so a Control-only holder — who IS entitled to know the `.acl`'s existence — must
    /// not be folded). This is EXACTLY the read-mode `guard_conditional_requires_read` computes, kept in
    /// lock-step so the two existence-disclosure gates cannot drift. When the (already-authorized)
    /// requester's granted modes do NOT include that read-mode, the handler returns the requester's
    /// DENIAL code (401 anonymous / 403 authenticated) — the SAME byte-identical denial an
    /// existing-but-forbidden sibling returns — INSTEAD of the existence-revealing 404/405, so missing and
    /// forbidden are indistinguishable. A requester WITH the read-mode (the pod owner, or any
    /// inheritable-Read holder — including the CTH `post-target-not-found` `clients.alice`, authorized via
    /// the test container's inherited `acl:default`) keeps the true 404/405: they could GET/read the
    /// target and learn its existence anyway, so the status discloses nothing new. The SUCCESS path (a
    /// POST into an EXISTING container → 201) is NOT gated — an `acl:Append`-only drop-box writer must
    /// still create members in a container that exists; only the existence-disclosing 404/405 branches are
    /// folded. `granted` is the mode set the POST authorization already returned (no extra ACL resolution).
    ///
    /// Why the read-mode (not "`acl:accessTo` on the container") is the correct distinguisher: the CTH's
    /// authorized POSTer reaches a MISSING target whose nearest existing ancestor (a freshly-created test
    /// container) has NO own `.acl`, so it holds NO `accessTo` there — its authorization, like a
    /// drop-box's, is via inherited `acl:default`. An `accessTo`-vs-`default` split would therefore fold
    /// the legitimate owner (breaking the CTH 404) AND would leave the oracle OPEN for an
    /// `acl:accessTo acl:Append`-WITHOUT-Read holder (still 403-vs-404 across an existing-locked vs a
    /// missing child). The read-mode is the property that both (a) the owner genuinely holds and (b) an
    /// existence-probing writer genuinely lacks. One narrow residual survives — a Read-holder-via-
    /// inheritance who is SEPARATELY denied on a specific existing child by that child's own restrictive
    /// `.acl` can still distinguish THAT child (403) from a missing one (404); this is WAC-inherent
    /// (a per-child `.acl` legitimately overrides inheritance) and documented in RSS
    /// `decisions/0003` — carried forward as `research/lws-design-records.md` §6.
    fn guard_post_existence_requires_read(
        &self,
        target_iri: &str,
        granted: &std::collections::BTreeSet<AccessMode>,
        token: &VerifiedToken,
    ) -> Result<(), ServerError> {
        // The mode that governs reading THIS target's representation/existence: Control for an `.acl`
        // (Control does NOT imply Read), else Read — identical to `guard_conditional_requires_read`.
        let read_mode = if crate::authz::is_acl_resource(target_iri) {
            AccessMode::Control
        } else {
            AccessMode::Read
        };
        if !granted.contains(&read_mode) {
            return Err(if token.web_id.is_none() {
                self.unauthenticated()
            } else {
                ServerError::Forbidden
            });
        }
        Ok(())
    }

    /// Whether the conditional precondition header `name` carries a CONCRETE entity-tag validator (a
    /// quoted ETag, or a list of them) rather than the bare `*` wildcard. An ABSENT header carries
    /// none; a bare `*` is existence-only (not content-derived) so it is NOT ETag-bearing; anything
    /// else — including a present-but-non-ASCII value that cannot be decoded — is treated as ETag-
    /// bearing (fail-closed), keeping it subject to the V4 Read-gate. Used only by
    /// [`guard_conditional_requires_read`](Self::guard_conditional_requires_read).
    fn conditional_carries_etag(headers: &HeaderMap, name: HeaderName) -> bool {
        match headers.get(&name) {
            None => false,
            Some(v) => match v.to_str() {
                Ok(s) => !crate::ldp::conditional::is_wildcard(s),
                Err(_) => true,
            },
        }
    }

    /// The nearest EXISTING container at or above `target` (its parent, then grandparent, … up to the
    /// storage root), or `None` if none exists (not even the root).
    async fn nearest_existing_container(
        &self,
        target_iri: &str,
    ) -> Result<Option<String>, ServerError> {
        let root = format!("{}/", self.base_url.trim_end_matches('/'));
        // Start from the immediate parent: drop a container's own trailing slash first.
        let mut current = target_iri.to_string();
        if current.ends_with('/') {
            current.pop();
        }
        while let Some(slash) = current.rfind('/') {
            let parent = current[..=slash].to_string();
            if self.store.exists(&parent).await? {
                return Ok(Some(parent));
            }
            if parent == root || parent.len() <= root.len() {
                break;
            }
            current = parent[..parent.len() - 1].to_string();
        }
        Ok(None)
    }
}

/// `GET /{path}` — read a resource, with `Accept`-driven content negotiation + `Range` support.
///
/// Content negotiation: an RDF resource stored as Turtle is re-serialised to JSON-LD (or vice
/// versa) when the client's `Accept` prefers it; a non-RDF body is served verbatim (its `Accept`
/// is honoured only as `*/*`). `Range: bytes=…` yields a 206 + `Content-Range` (single range), or a
/// 416 when unsatisfiable. Conditional-GET read preconditions (`If-None-Match` → 304, with
/// `If-Modified-Since` as the lower-precedence fallback) are applied in `serve_read` AFTER
/// authorization and BEFORE the body/Range work (RFC 9110 §13).
pub async fn get_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    serve_read::<S>(&state, &token, &uri, &headers, true).await
}

/// `HEAD /{path}` — the GET response headers without the body.
pub async fn head_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    serve_read::<S>(&state, &token, &uri, &headers, false).await
}

/// Shared GET/HEAD read path. `with_body` distinguishes GET (send bytes) from HEAD (headers only).
///
/// `pub(crate)` so the pre-crypto public-read skip middleware ([`crate::ldp::public_read_skip`]) can
/// serve a PUBLIC read AS anonymous (token = [`VerifiedToken::public`]) over the SAME code path the
/// handler uses — guaranteeing a skipped public read is byte-identical to a genuinely anonymous one
/// (INV-1). The middleware never passes a non-public token here.
pub(crate) async fn serve_read<S: Store>(
    state: &Arc<LdpState<S>>,
    token: &VerifiedToken,
    uri: &axum::http::Uri,
    req_headers: &HeaderMap,
    with_body: bool,
) -> Result<Response, ServerError> {
    let target = parse_target(&state.base_url, uri.path())?;

    // WAC read authorization (real per-resource `.acl` evaluation). A GET/HEAD requires `acl:Read`
    // (Control for an `.acl` target); the public-read class is whatever the effective ACL grants to
    // `foaf:Agent`, so a WebID profile card with a public-read ACL stays anonymously readable while
    // private data answers 401 (anonymous) / 403 (authenticated-but-unauthorized). Authorization runs
    // BEFORE the existence check, so a permitted read of a missing resource is a 404, while an
    // unauthorized read of the same is a 401/403 (no existence leak).
    // SINGLE-PASS read authorization (Optimization #2) over ONE combined read-plan round-trip
    // (read-2): the target's metadata + the whole ACL-candidate chain are fetched in ONE
    // `Store::read_plan` call, the ACL walk runs in memory over those rows, and BOTH the access
    // decision (Allow / 401 / 403) AND the `WAC-Allow` audiences (`user` + `public`) derive from
    // that one resolution. `perms` is reused below to emit `WAC-Allow` with no further ACL work;
    // `target_meta` is the SAME plan's target row, so no second metadata query is needed.
    let origin = request_origin(req_headers);
    let (perms, target_meta) = state
        .authorize_read(
            if with_body { "GET" } else { "HEAD" },
            &target,
            token,
            origin,
        )
        .await?;

    // The 404 decision stays AFTER authorization (no existence leak — an unauthorized read of a
    // missing resource remains 401/403 above, a permitted one 404 here), exactly as before.
    let meta = target_meta.ok_or(ServerError::NotFound)?;
    // The BYTES are fetched only now — after the Allow (no speculative byte fetch, design
    // invariant 5) — through the plan's held metadata (`read_at`, §3.3): the unique-per-write blob
    // key names an immutable object, so these are exactly the bytes that metadata committed with.
    let body = state.store.read_at(&target.iri, &meta).await?;
    let resource = Resource { body, meta };

    let accept = header_str(req_headers, header::ACCEPT);
    // Compute the response validator (ETag) that a 200 would carry FIRST, so a 304 short-circuit uses
    // the IDENTICAL tag (RFC 9110 §13 — the 304 validator must equal the 200's for the same state).
    //
    // ETag: an entity-tag identifies a REPRESENTATION, not a resource (RFC 9110 §8.8.3), so the
    // validator must be specific to the representation this request negotiates.
    //
    // - A CONTAINER's body is GENERATED from LIVE membership (the `ldp:contains` listing), so its
    //   validator is derived from the FINAL RENDERED representation — not the stored-metadata ETag,
    //   which never changes when a child is added/removed (the stale-validator bug). We render it
    //   here (a 200 needs it anyway) and hash it; the negotiated format changes the bytes, so the
    //   tag is representation-specific for free. GET and HEAD compute the SAME body, so they agree.
    // - A PLAIN resource served VERBATIM (stored format, or non-RDF bytes) keeps its stored-metadata
    //   ETag. A content-NEGOTIATED (re-serialised) RDF response gets the `"<state>+<variant>"` tag
    //   ([`conditional::variant_etag`]) — computed from the Accept header ALONE, WITHOUT serialising
    //   the body, so a matching read precondition can 304 while SKIPPING negotiation + Range (the
    //   read-304 fast path). A client holding the Turtle tag but asking for JSON-LD therefore gets a
    //   fresh 200, never a 304 for a representation it doesn't hold; the write path accepts either
    //   tag's STATE part for If-Match (the GET → PUT round-trip — see `conditional`'s module doc).
    //   An Accept that EXPLICITLY refuses (q=0) every producible type it covers is a 406 here,
    //   BEFORE the precondition check (a conditional applies to the selected representation; with
    //   none selectable there is no 304). An Accept merely naming no producible type instead falls
    //   back to text/turtle, the Solid default (see `content::negotiate_accept_with_profile`).
    let (rendered, etag): (Option<(Bytes, String)>, String) = if target.is_container {
        let (body, content_type) = render_container(
            state,
            &target.iri,
            &resource.body,
            &resource.meta.content_type,
            accept,
        )
        .await?;
        let etag = representation_etag(&body);
        (Some((body, content_type)), etag)
    } else {
        let etag = negotiated_validator(&resource.meta.etag, &resource.meta.content_type, accept)?;
        (None, etag)
    };

    // The response headers shared by the 304 short-circuit and the full 200/206/416 path: the
    // validator (`ETag`) plus every NON-representation advertisement header this server emits on a
    // read. RFC 9110 §15.4.5 requires a 304 to carry the validators a 200 would and forbids only
    // *representation metadata* (`Content-Type` / body `Content-Length` are added on the full path
    // below, never on the 304) — while the WAC spec requires `WAC-Allow` on GET/HEAD responses and
    // the conformance harness reads the acl/type/discovery `Link` rels off HEAD, so a 304 carries
    // those advertisements exactly like a 200 (they describe the RESOURCE, not the representation).
    //
    // Pre-size the map (perf round-C, MALLOC band) so the inserts below never trigger an incremental
    // `HeaderMap` grow-and-rehash: the full read response carries ETag, Vary, Allow[+Accept-Post],
    // Accept-Patch, 2 discovery Links, 1–4 type Links, 1 acl Link, WAC-Allow, a plain resource's
    // Last-Modified, and then (full path only) Content-Type + Accept-Ranges + Content-Length/
    // Content-Range — ≈18 entries at most. `with_capacity` rounds up to a power of two ≥ the request,
    // so sizing for the container/storage-root maximum means neither the 304 path (fewer entries) nor
    // a full plain-resource/container response reallocates. Byte-identical output.
    let mut out = HeaderMap::with_capacity(18);
    // V5 (`research/lws-design-records.md` §6) — the membership-derived container `etag` computed
    // above shifts on every child add/remove, so it is a listing oracle. It is exposed ONLY here,
    // on the GET/HEAD read path, which is gated above by `authorize_read` requiring `acl:Read` on
    // the container — so a non-reader NEVER reaches this ETag (nor the 304 short-circuit that also
    // carries it). The conditional-channel sibling (a non-reader probing the container ETag via
    // `If-Match` on a write) is closed by the V4 `guard_conditional_requires_read` in the mutating
    // handlers. Together these Read-gate the container ETag end to end. (If a future change emits a
    // container's representation ETag outside a Read-gated path, that gate must be re-established
    // there too.)
    set_str(&mut out, header::ETAG, &etag);
    // `Vary: Accept` (RFC 9110 §12.5.5): the representation AND its `ETag` above were selected using
    // the request's `Accept` header (RDF conneg Turtle↔JSON-LD; a container's rendered body is
    // likewise Accept-driven), so a shared cache MUST key on `Accept` too — otherwise it could serve
    // a Turtle response (or a Turtle-tagged 304) to a client that asked for JSON-LD, or vice versa
    // (the roborev finding on 1e5a47d this closes: representation-specific validators were added
    // without the matching `Vary`, leaving caches free to conflate negotiated representations).
    // Emitted UNCONDITIONALLY on every read response that reaches this point (200/304/206/416) —
    // including for a verbatim non-RDF resource, where it is merely conservative (the representation
    // never changes shape, but declaring the dependency is always safe per RFC 9110 §12.5.5). The
    // CORS middleware (the outermost layer) MERGES its own `Vary: Origin` onto whatever the handler
    // sets rather than overwriting it (see `cors::merge_vary`), so the wire value ends up
    // `Accept, Origin`.
    set_str(&mut out, header::VARY, "Accept");
    // Method advertisement on the read response: `Allow` (the LDP verb set — `read-method-allow`
    // asserts GET/HEAD responses carry `Allow` listing GET + HEAD) + `Accept-Post` (containers only)
    // + `Accept-Patch`. (OPTIONS itself is answered by the CORS layer, which short-circuits every
    // OPTIONS; the `options_handler` is the non-CORS fallback.)
    add_method_advertisement(&mut out, target.is_container);
    // Notification discovery: advertise the storage-description doc via `describedby` +
    // `solid:storageDescription` Link rels so a client can HEAD a resource and find the subscription
    // service. The values were PRECOMPUTED once from `base_url` at construction
    // (`LdpState::discovery_link_values`), so this is a couple of refcount-bump appends — no
    // per-request formatting (the single discovery home is still `notifications::ws::link_headers`,
    // consumed once into the cache).
    add_discovery_links(&mut out, &state.discovery_link_values);
    // LDP/Solid type advertisement (`Link: <type>; rel="type"`): a container advertises
    // `ldp:BasicContainer` (+ `ldp:Container`/`ldp:Resource`), and the STORAGE ROOT additionally
    // advertises `pim:Storage` (Solid Protocol §4.1). The conformance harness REQUIRES the
    // `pim:Storage` rel=type header on the pod root to recognise an accessible storage at bootstrap.
    add_type_links(&mut out, &target, &state.base_url);
    // ACL discovery (`Link: <…>; rel="acl"`, Solid Protocol §4.3.1): every resource advertises the URL
    // of its access-control document (the conventional `<resource>.acl` / `<container>/.acl`). The
    // conformance harness reads this at bootstrap to locate where to write the test container's ACL.
    add_acl_link(&mut out, &target);
    // WAC-Allow (Solid Protocol): advertise the requester's + the public's effective access modes for
    // this target. Both audiences were resolved by `authorize_read` above in the SAME pass as the
    // access decision (no second ACL walk/read/parse) — `perms` is serialised directly.
    let wac_allow = wac_allow_header(&perms);
    set_str(&mut out, HeaderName::from_static("wac-allow"), &wac_allow);

    // READ preconditions (RFC 9110 §13), evaluated AFTER authorization (so a 304 can never leak the
    // existence of a resource the caller could not read — auth + the 404 for a missing target both
    // already ran above) and BEFORE serialising a plain resource's body / computing Range.
    // `If-None-Match` (weak comparison, `*`) takes PRECEDENCE over `If-Modified-Since`. A match ⇒ 304
    // Not Modified carrying the validators + no body; when a `Range` is present alongside a matching
    // `If-None-Match` the precondition WINS (304, never a 206).
    //
    // `If-Modified-Since` is now LIVE (jx3c): the store surfaces the resource's server-recorded
    // modification time (`pss:modified` → `ResourceMeta::last_modified`), which the evaluator
    // compares against the header — a `last_modified ≤ header` ⇒ 304, an absent time ⇒ a fresh 200.
    // A write bumps the stored time, so a re-written resource correctly re-serves.
    //
    // GRANULARITY (why whole seconds is correct, not a defect): an HTTP date (RFC 9110 §5.6.7
    // IMF-fixdate) has NO sub-second field, so a client can never send a sub-second
    // `If-Modified-Since` — whole-second comparison is the only representable resolution, and storing
    // sub-second precision would instead BREAK the common 304 (a mid-second `last_modified` would
    // exceed the whole-second header even when unchanged). The one residual — two rewrites within the
    // SAME second producing an `If-Modified-Since`-only stale 304 — is inherent to HTTP's whole-second
    // `Last-Modified` (shared by every conformant server) and is covered by the STRONG validator: the
    // content-derived `ETag` differs across a changed body and `If-None-Match` takes precedence here,
    // so a client sending both correctly gets a 200. `Last-Modified` is the weak fallback, `ETag` the
    // authority — the RFC 9110 §8.8.2 model.
    //
    // CONTAINERS are deliberately excluded (pass `None`): a container's body is GENERATED from LIVE
    // `ldp:contains` membership, which changes WITHOUT touching the container record's own
    // `pss:modified` — so the stored record time is a STALE validator for the listing (exactly why
    // the container `etag` above is representation-derived, not the stored etag). Using it for
    // `If-Modified-Since` could 304 a listing that actually changed (serving stale content). So a
    // container always fails OPEN to a fresh 200; only a PLAIN resource's `last_modified` (whose
    // stored state IS its representation, across conneg) drives a 304.
    let effective_last_modified = if target.is_container {
        None
    } else {
        resource.meta.last_modified
    };
    // Advertise a plain resource's modification time as `Last-Modified` (RFC 9110 §8.8.2) so a client
    // can OBTAIN the validator and echo it in a later `If-Modified-Since` — without it the conditional
    // path is unusable in normal cache flows. Set on the SHARED headers (so the 304 carries it too,
    // §15.4.5) and only for a plain resource with a known time; a container's stale record time is
    // deliberately not advertised (matching the 304 exclusion above). GET and HEAD emit it identically.
    if let Some(imf) = effective_last_modified.and_then(crate::store::timestamp::to_imf_fixdate) {
        set_str(&mut out, header::LAST_MODIFIED, &imf);
    }
    if conditional::evaluate_read(
        header_str(req_headers, header::IF_NONE_MATCH),
        header_str(req_headers, header::IF_MODIFIED_SINCE),
        &etag,
        effective_last_modified,
    ) == conditional::ReadPrecondition::NotModified
    {
        // 304 Not Modified: the shared headers above (validator + advertisements), NO body and NO
        // representation metadata (RFC 9110 §15.4.5) — GET and HEAD are identical here.
        return Ok((StatusCode::NOT_MODIFIED, out).into_response());
    }

    // Not a 304: materialise the body (negotiating a plain resource now) + its content type. For a
    // container the body was already rendered above (reused, not re-rendered).
    let (body, content_type) = match rendered {
        Some(bc) => bc,
        None => negotiate_body(
            &resource.body,
            &resource.meta.content_type,
            accept,
            &target.iri,
        )?,
    };

    let total_len = body.len() as u64;
    // `Range` is defined for GET (RFC 9110 §14.2); ignore it for HEAD so a HEAD never returns 206.
    let outcome = if with_body {
        range::evaluate(header_str(req_headers, header::RANGE), total_len)
    } else {
        RangeOutcome::Full
    };

    // Representation metadata — the full-response path only (never on a 304).
    set_str(&mut out, header::CONTENT_TYPE, &content_type);
    // Advertise byte-range support (RFC 9110 §14.3). The value is the compile-time constant `"bytes"`,
    // so `from_static` skips the runtime `HeaderValue::from_str` validation+allocation `set_str` does
    // (perf round-C, MALLOC band — byte-identical output).
    out.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    match outcome {
        RangeOutcome::Unsatisfiable => {
            // 416 + a Content-Range stating the full length (RFC 9110 §15.5.17). Build the response
            // directly so the Content-Range header rides along (the error type carries only a body).
            set_str(
                &mut out,
                header::CONTENT_RANGE,
                &format!("bytes */{total_len}"),
            );
            Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                out,
                "range not satisfiable",
            )
                .into_response())
        }
        RangeOutcome::Satisfied { start, end } => {
            let slice = body.slice(start as usize..=end as usize);
            set_str(
                &mut out,
                header::CONTENT_RANGE,
                &format!("bytes {start}-{end}/{total_len}"),
            );
            set_u64(&mut out, header::CONTENT_LENGTH, slice.len() as u64);
            if with_body {
                Ok((StatusCode::PARTIAL_CONTENT, out, slice).into_response())
            } else {
                Ok((StatusCode::PARTIAL_CONTENT, out).into_response())
            }
        }
        // [GPT-5.6] Multipart byte-range response emission.
        RangeOutcome::Multipart(ranges) => {
            let multipart = Bytes::from(range::encode_multipart(&body, &content_type, &ranges));
            set_str(
                &mut out,
                header::CONTENT_TYPE,
                &format!(
                    "multipart/byteranges; boundary={}",
                    range::MULTIPART_BOUNDARY
                ),
            );
            set_u64(&mut out, header::CONTENT_LENGTH, multipart.len() as u64);
            Ok((StatusCode::PARTIAL_CONTENT, out, multipart).into_response())
        }
        RangeOutcome::Full => {
            set_u64(&mut out, header::CONTENT_LENGTH, total_len);
            if with_body {
                Ok((StatusCode::OK, out, body).into_response())
            } else {
                Ok((StatusCode::OK, out).into_response())
            }
        }
    }
}

/// `PUT /{path}` — create-or-replace an RDF resource (Turtle / JSON-LD), with conditional-write
/// support (`If-Match` / `If-None-Match`).
///
/// Fail-closed: a mutation from a public caller is a 403 (the WAC seam is M2-next). The body is
/// validated as well-formed RDF in its declared type (415 unsupported / 400 malformed). The
/// `If-None-Match: *` create-guard and `If-Match` overwrite-guard are evaluated against the current
/// ETag (412 on mismatch).
pub async fn put_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ServerError> {
    #[cfg(feature = "sparql-endpoint")]
    let _snapshot_guard = state.sparql_snapshot_write().await;
    let target = parse_target(&state.base_url, uri.path())?;

    // WAC for PUT — EXISTENCE-NON-DISCLOSURE (`research/lws-design-records.md` §6): a PUT requires
    // `acl:Write` on the TARGET's effective ACL (inherited via `acl:default` for a not-yet-existing
    // target), authorized **regardless of whether the target exists** so create and overwrite are
    // INDISTINGUISHABLE to an under-authorized requester:
    //  - **Overwrite** (target exists): `acl:Write` on the target — unchanged.
    //  - **Create** (target absent): ALSO `acl:Write` on the target's INHERITED ACL — NOT the weaker
    //    parent-`acl:Append`. This closes the V1 create-vs-forbidden-overwrite existence oracle: a
    //    drop-box writer holding only parent `acl:Append` (no target Write) previously got a 201 on a
    //    free name but a 403 on a taken one — leaking which child names exist. Now both are the SAME
    //    denial. (CTH-safe: every `write-access-*` PUT-fictive row that expects 201 grants the agent
    //    inheritable `acl:Write`; no row expects an Append-only PUT-create=201 —
    //    see `research/lws-design-records.md` §6.)
    //    TRADE-OFF: an `acl:Append`-only agent can no longer PUT-create; it MUST use POST (which mints
    //    a server-opaque, collision-free name — the containment-mutating create primitive). Documented
    //    in the ADR.
    //  - **`.acl` target**: routes to `acl:Control` on the protected resource (managing access rules) —
    //    `mode_for_operation`/`authorize` already override the mode to Control for an `.acl`.
    //
    // Authorize BEFORE any target-dependent `meta()`/existence probe (the V1 timing closure): the
    // under-authorized denial is returned with no observable dependence on whether the target exists,
    // and the access decision itself reads ONLY `.acl` resources (never the target's own bytes/meta).
    let origin = request_origin(&headers);
    let granted = state.authorize("PUT", &target, &token, origin).await?;

    // V4 (`research/lws-design-records.md` §6): a conditional precondition is a
    // CONTENT/MEMBERSHIP-derived validator — a requester lacking `acl:Read` on the target must NOT
    // get its existence-revealing 412-vs-2xx outcome (nor a returned ETag). Fold to the denial code
    // when a conditional header is present and the requester holds no Read. Done BEFORE the
    // existence probe so it adds no oracle of its own.
    state.guard_conditional_requires_read(&target.iri, &headers, &granted, &token)?;

    // The caller IS authorized. Only NOW probe existence (an authorized writer is entitled to learn
    // create-vs-replace) — reused for the conditional-write ETag and the create/replace branch below.
    let current = state.store.meta(&target.iri).await?;
    let existed = current.is_some();

    // Slash-semantics: a trailing-slash IRI (a container) and the same IRI without the slash (a plain
    // resource) MUST NOT co-exist (Solid Protocol — "with and without trailing slash cannot
    // co-exist"). Refuse a PUT whose URI collides with an EXISTING resource of the opposite kind.
    reject_slash_semantics_conflict(state.as_ref(), &target).await?;

    // A write MUST carry a Content-Type (Solid Protocol §writing — `content-type-reject`). An ABSENT
    // Content-Type is a 400 Bad Request.
    let content_type = require_content_type(&headers)?;
    // Validate + select the stored media type. An RDF type is parse-validated (400 on malformed); a
    // NON-RDF type (e.g. `text/plain`, an image) is stored VERBATIM as an opaque binary resource —
    // the Solid Protocol stores any content type, and a read serves a binary body unchanged (see
    // `negotiate_body`). The stored media type is the (sanitised) declared one.
    let stored_type = validate_writable(&content_type, &body, &target.iri)?;

    // Conditional write: evaluate preconditions against the CURRENT representation's ETag.
    let current_etag = current.as_ref().map(|m| m.etag.as_str());
    conditional::require(eval_preconditions(
        header_str(&headers, header::IF_MATCH),
        header_str(&headers, header::IF_NONE_MATCH),
        current_etag,
    ))?;

    let parent = parent_container(&target);

    let meta = if existed {
        // A replace: rewrite the bytes in place; containment is unchanged.
        state.store.write(&target.iri, body, &stored_type).await?
    } else if crate::authz::is_acl_resource(&target.iri) {
        // A CREATE of an AUXILIARY `.acl` resource: it is NOT a contained child. Store it via a plain
        // `write` (no `ldp:contains` edge on the parent, and a later DELETE mutates no parent
        // containment) — the Solid auxiliary-resource model. Auth for `.acl` is Control (above).
        state.store.write(&target.iri, body, &stored_type).await?
    } else {
        // A CREATE via PUT must create intermediate containers (Solid Protocol §writing-resource —
        // "Creating a resource using PUT … must create intermediate containers") AND wire the new
        // resource into its parent's `ldp:contains` membership (so the container GET lists it). An
        // ancestor that already exists as a NON-container is a conflict (a resource cannot have a
        // child) → handled by `ensure_ancestor_containers`.
        //
        // WAC container-modification: this mutates the containment of the nearest existing ancestor, so
        // it requires `acl:Append` (Write subsumes) ON THAT CONTAINER — in addition to the target-ACL
        // Write authorized above. Symmetric with DELETE's parent-Write check; closes the create-authz
        // widening (an `acl:default`-only Write / pre-provisioned target `.acl` must NOT let an agent
        // with no mode on the container mint members or intermediate containers in it).
        state
            .authorize_container_modification(&target.iri, &token, origin)
            .await?;
        ensure_ancestor_containers(state.as_ref(), &target.iri).await?;
        match &parent {
            Some(p) => {
                state
                    .store
                    .create_in_container(p, &target.iri, body, &stored_type)
                    .await?
            }
            // No parent (a root-level write): a plain write mints the record.
            None => state.store.write(&target.iri, body, &stored_type).await?,
        }
    };

    // EMIT (the single notification hook on the PUT path): a replace ⇒ Update, a create ⇒ Create. A
    // PUT-created resource also grows its container's membership, so pass the parent (the hub derives
    // the parent `Add`); a replace passes no parent (no membership change).
    //
    // EXCEPTION — an AUXILIARY `.acl` resource is NOT a contained child (it was stored via a plain
    // `write`, with NO `ldp:contains` edge added to the parent above). So even on a CREATE its parent
    // membership did NOT change: pass `None` for the emit parent so the hub does NOT derive a spurious
    // container-membership `Add` for a resource the container does not actually contain.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let activity = if existed {
            ActivityType::Update
        } else {
            ActivityType::Create
        };
        let emit_parent = if existed || crate::authz::is_acl_resource(&target.iri) {
            None
        } else {
            parent.clone()
        };
        state
            .notifications
            .notify(&target.iri, activity, emit_parent.as_deref())
            .await;
    }

    // A PUT to an `.acl` resource changed the access rules: invalidate the cached parse so the NEXT
    // read resolves against the new ACL immediately (belt-and-braces over the etag gate; see
    // `invalidate_acl_if_acl`).
    state.invalidate_acl_if_acl(&target.iri);

    Ok(write_response(existed, &meta, &target.iri))
}

/// `POST /{path}` — create a child resource inside a container.
///
/// Honours the `Slug` header (sanitised) and mints a server URI when absent or colliding. POST to a
/// non-container is a 409 Conflict; POST to a container that does not exist is a 404. Returns 201 +
/// `Location`. Fail-closed (public ⇒ 403).
pub async fn post_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ServerError> {
    #[cfg(feature = "sparql-endpoint")]
    let _snapshot_guard = state.sparql_snapshot_write().await;
    let container = parse_target(&state.base_url, uri.path())?;
    // WAC: a POST to a container requires `acl:Append` on the container (a writer also satisfies it).
    // Anonymous ⇒ 401, authenticated-but-unauthorized ⇒ 403. Authorize BEFORE the container-shape /
    // existence checks so an unauthorized caller cannot probe existence (the read-access POST cases
    // accept `[403]` for a real container and `[403, 404]` for a fictive one — authorize-first 403 is
    // within both).
    let origin = request_origin(&headers);
    // Capture the FULL granted mode set (not just pass/fail): the existence-non-disclosure V6 Read-gate
    // below folds the 404/405 existence branches unless the requester holds `acl:Read` on the target.
    let granted = state.authorize("POST", &container, &token, origin).await?;

    // POST creates a CHILD in a CONTAINER — the target must be a container (trailing-slash path).
    // A POST to a non-container target is NOT a containment operation: per the Solid Protocol
    // `post-target-not-found` scenarios it is `[404, 405]` — 404 when nothing exists at that URI,
    // 405 Method-Not-Allowed when a plain resource is there (POST does not create a child of a
    // resource). (This supersedes the earlier 409 — a 409 is not the spec-accepted status here.)
    if !container.is_container {
        // EXISTENCE-NON-DISCLOSURE (V6, `research/lws-design-records.md` §6): the non-container
        // existence branch (405 when a resource is present, 404 when absent) DISCLOSES whether the
        // named target exists. Fold it to the requester's denial unless they hold Read on the
        // target — BEFORE the existence probe, so the deny path performs no target-dependent lookup
        // (structural, per the ADR's timing note).
        state.guard_post_existence_requires_read(&container.iri, &granted, &token)?;
        return if state.store.exists(&container.iri).await? {
            Err(ServerError::MethodNotAllowed)
        } else {
            Err(ServerError::NotFound)
        };
    }
    // The container must exist (the authoritative index check) — never create a child + a containment
    // edge under a missing container. A missing container is a 404 (`post-target-not-found`).
    //
    // EXISTENCE-NON-DISCLOSURE (V6): this existence probe is a TARGET-DEPENDENT lookup, so for a
    // requester who lacks the target's read-mode NEITHER of its non-create outcomes may be observable —
    // not the 404 (missing) NOR a backend-fault 5xx. A bare `?` here would let a `store.exists` error
    // escape as a 500 that an existing-but-forbidden sibling — denied at authorization, which reads only
    // `.acl` records and never probes the target — does NOT produce, i.e. a backend-error existence/state
    // oracle of the `patch_*_faulting_target_read` class. So BOTH the missing case AND a probe fault are
    // folded to the requester's uniform denial (via the same read-mode gate); a read-mode holder — who is
    // entitled to the target's state — gets the true 404 / the surfaced backend error. An EXISTING
    // container falls through to the 201 create below (an Append-only writer legitimately creates
    // members), so ONLY the non-create outcomes are gated — never the success path.
    match state.store.exists(&container.iri).await {
        // The container exists → fall through to the 201 create path below.
        Ok(true) => {}
        Ok(false) => {
            state.guard_post_existence_requires_read(&container.iri, &granted, &token)?;
            return Err(ServerError::NotFound);
        }
        Err(e) => {
            // A backend fault on the existence probe: fold a non-read-mode requester to their uniform
            // denial FIRST (so the 5xx is never observable to them), else surface the real error.
            state.guard_post_existence_requires_read(&container.iri, &granted, &token)?;
            return Err(e);
        }
    }

    // A POST write MUST carry a Content-Type (Solid Protocol — `content-type-reject`): ABSENT ⇒ 400.
    let content_type = require_content_type(&headers)?;

    // Container-intent: a `Link: <http://www.w3.org/ns/ldp#BasicContainer>; rel="type"` (or
    // `ldp:Container`) on a POST asks the server to create a CONTAINER child (LDP §5.2.3.4) — the
    // minted child IRI then ends in `/` and is created as a container. Without the type Link, a plain
    // resource child is created.
    let wants_container = wants_container_via_link(&headers);

    // The sanitised Slug STEM (the caller's name hint; `None` if no usable Slug). The mint uses it ONLY
    // as a prefix of an opaque, collision-free name (V2 — see `mint_child_iri`), so the final segment
    // never equals the verbatim Slug. The `.acl`-intent guard below is checked against THIS STEM (the
    // caller's intent) rather than the post-opaque minted IRI.
    let slug = header_str(&headers, HeaderName::from_static("slug"));
    let stem = slug.and_then(sanitise_slug);

    // SECURITY (privilege-escalation guard): a POST authorizes only `acl:Append`/`Write` on the
    // CONTAINER — never `acl:Control`. `sanitise_slug` keeps `.`, so a `Slug: secret.acl` carries the
    // INTENT to mint an ACL auxiliary. Even though V2's opaque-suffix mint would now produce a benign
    // `…/secret.acl-<opaque>` (which the WAC resolver — exact `.acl` suffix only — never reads as an
    // ACL, so the escalation is already structurally defused), we STILL refuse the request: rejecting
    // the INTENT keeps a single, clear contract — "an Append-only POST cannot author an `.acl`" — and
    // is belt-and-braces against any future mint change that might preserve the suffix. A create of a
    // `.acl` is a Control operation; the Control-gated PUT/PATCH of an `.acl` is the only legitimate
    // path. The check is on the SANITISED STEM (the caller's intent, covering the case-variant
    // `secret.ACL` via the case-insensitive `is_acl_auxiliary_suffix`).
    //
    // The denial uses the REQUESTER's denial shape — 401 + `WWW-Authenticate` for an anonymous caller,
    // 403 for an authenticated one — IDENTICAL to every other POST denial. (POST authorization already
    // ran above, so an anonymous caller without public `acl:Append` is already 401'd before here; this
    // matters only for a PUBLIC-append container where an anonymous caller CAN reach this guard, and
    // there the anonymous denial must still carry the auth challenge, not a bare 403 — keeping the
    // denial surface uniform so the `.acl`-intent case is indistinguishable in shape from any other
    // unauthorized POST. The guard is intent-based, not existence-based: `secret.acl` and `benign.acl`
    // are refused regardless of what exists, so it is never an existence oracle.)
    //
    // SCOPE: `.acl` ONLY. `.meta` description-resources are NOT load-bearing in this server (the WAC
    // resolver never consults a `.meta`, and the PUT/PATCH create paths only special-case `.acl`), so
    // a `secret.meta` stem is just a normal resource name — guarding it ONLY at POST while PUT/PATCH
    // would create it freely is an inconsistency with no security benefit, so it is not guarded. If
    // `.meta` (or any other auxiliary) ever becomes load-bearing it MUST be guarded UNIFORMLY across
    // POST/PUT/PATCH/DELETE/read — not POST-only (see `is_acl_auxiliary_suffix`).
    if let Some(s) = &stem {
        // Check the `.acl` suffix on the bare stem (a leaf segment with no scheme/slashes). A trailing
        // `/` is not part of a sanitised stem, so this catches `secret.acl`/`secret.ACL` directly.
        if crate::authz::is_acl_auxiliary_suffix(s) {
            return Err(if token.web_id.is_none() {
                state.unauthenticated()
            } else {
                ServerError::Forbidden
            });
        }
    }

    // Mint the child IRI from the (guarded) stem: an opaque, collision-free name prefixed by the stem,
    // so the `Location` is collision-INDEPENDENT (V2). A container child gets a trailing slash.
    let child_iri = mint_child_iri(
        &state.store,
        &container.iri,
        stem.as_deref(),
        wants_container,
    )
    .await?;

    // Validate + select the stored media type, resolving relative IRIs against the MINTED child IRI.
    // RDF is parse-validated; a non-RDF type is stored verbatim as an opaque binary resource. A
    // container's body is conventionally empty/RDF; we still validate whatever was sent.
    let stored_type = validate_writable(&content_type, &body, &child_iri)?;

    let meta = state
        .store
        .create_in_container(&container.iri, &child_iri, body, &stored_type)
        .await?;

    // EMIT: a POST always CREATES the child and GROWS the container's membership — Create on the child
    // + a derived Add on the container (the hub fans both from this one call).
    #[cfg(not(target_arch = "wasm32"))]
    state
        .notifications
        .notify(&child_iri, ActivityType::Create, Some(&container.iri))
        .await;

    let mut out = HeaderMap::new();
    set_str(&mut out, header::ETAG, &meta.etag);
    set_str(&mut out, header::LOCATION, &child_iri);
    Ok((StatusCode::CREATED, out).into_response())
}

/// `DELETE /{path}` — delete a resource OR a container.
///
/// A non-existent target is a 404. `If-Match` / `If-None-Match` are honoured (412 on mismatch). On
/// success returns 204. Fail-closed (public ⇒ 403).
///
/// **Container-delete semantics (the spec choice — documented per the standing make-the-call rule).**
/// A DELETE on a container path (trailing slash) is permitted ONLY when the container is empty: a
/// container with members is a **409 Conflict**, never a cascade. This is the conservative choice the
/// LDP spec permits (LDP §5.2.5.1 lets a server refuse to delete a non-empty container) and what CSS
/// does by default — it avoids a single request silently destroying an arbitrarily large subtree.
/// Deleting an empty container removes its own resource record AND its (empty) `ldp:contains` set in
/// SPARQ (the live store `DROP`s the container's named graph; the in-memory double clears its
/// children entry), and detaches it from its parent's containment. Recursive / cascade delete is
/// intentionally NOT offered (an opt-in recursive delete is a possible future slice — file an issue
/// if a client needs it).
pub async fn delete_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    #[cfg(feature = "sparql-endpoint")]
    let _snapshot_guard = state.sparql_snapshot_write().await;
    let target = parse_target(&state.base_url, uri.path())?;

    // WAC for DELETE (Solid WAC write-access matrix). Authorize BEFORE the existence check so an
    // unauthorized caller cannot probe existence (a missing target below is reported as a denial, not
    // a 404 — no existence side-channel). The required rights:
    //  - on the TARGET: a CONTAINER needs `acl:Control` (the matrix uniformly forbids DELETE of a
    //    container to a mere Write holder — only the Control holder, typically the owner, may delete
    //    it); a DOCUMENT needs `acl:Write`; an `.acl` target needs `acl:Control` (and the parent-write
    //    check below is skipped — deleting an ACL only restores the inherited ACL, not containment).
    //  - PLUS `acl:Write` on the nearest existing PARENT container (DELETE mutates containment), unless
    //    the target is an `.acl`.
    let is_acl = crate::authz::is_acl_resource(&target.iri);
    // An `.acl` target and a CONTAINER target both require `acl:Control`; a plain document requires
    // `acl:Write`.
    let target_mode = if is_acl || target.is_container {
        AccessMode::Control
    } else {
        AccessMode::Write
    };
    let origin = request_origin(&headers);
    let granted = state
        .authorize_mode(&target, target_mode, &token, origin)
        .await?;
    if !is_acl {
        let parent = state.nearest_existing_container(&target.iri).await?;
        if let Some(p) = parent {
            state
                .authorize_iri(&p, AccessMode::Write, &token, origin)
                .await?;
        }
    }

    // V4 (`research/lws-design-records.md` §6): a DELETE may carry `If-Match`/`If-None-Match`,
    // whose 412-vs-2xx outcome against the CONTENT/MEMBERSHIP-derived current ETag is an
    // existence+content oracle. A requester authorized to DELETE but NOT to READ the target (a
    // Write-without-Read document holder) must get the denial code rather than that conditional
    // outcome. Folded BEFORE the existence probe.
    state.guard_conditional_requires_read(&target.iri, &headers, &granted, &token)?;

    let current = state.store.meta(&target.iri).await?;
    // A DELETE of a non-existent target is reported through the SAME denial surface as a permission
    // failure (401 anonymous / 403 authenticated), NOT a 404 — so a DELETE cannot be used as an
    // existence side-channel by a requester who could not otherwise learn the resource exists (the
    // WAC matrix asserts `[401]`/`[403]` for `fictive` DELETE rows even where the requester would have
    // had inherited write).
    let current = match current {
        Some(c) => c,
        None => {
            return Err(if token.web_id.is_none() {
                state.unauthenticated()
            } else {
                ServerError::Forbidden
            });
        }
    };

    // Conditional delete: honour If-Match / If-None-Match against the current ETag.
    conditional::require(eval_preconditions(
        header_str(&headers, header::IF_MATCH),
        header_str(&headers, header::IF_NONE_MATCH),
        Some(current.etag.as_str()),
    ))?;

    // An AUXILIARY `.acl` resource is NOT a contained child (it is created via `store.write`, never via
    // `create_in_container`), so its DELETE must NOT touch parent containment — pass `None` for the
    // parent. (A non-`.acl` resource detaches from its parent's `ldp:contains` as before.)
    let parent = if is_acl {
        None
    } else {
        parent_container(&target)
    };

    if target.is_container {
        // A container DELETE goes through the ATOMIC empty-check+delete (no TOCTOU): the empty check
        // and the delete are ONE store operation, so a child POSTed concurrently can never slip in
        // between a separate empty-check and a separate delete and be orphaned. A non-empty container
        // is a 409; an absent one a 404 (the precondition load above already 404'd a fully-absent
        // target, but the atomic op is the authoritative existence+empty decision).
        match state
            .store
            .delete_container_if_empty(&target.iri, parent.as_deref())
            .await?
        {
            DeleteOutcome::Deleted => {
                // EMIT only on an actual delete: Delete on the container + a derived Remove on its
                // parent (membership shrank). NotEmpty/NotFound deleted nothing ⇒ no notification.
                #[cfg(not(target_arch = "wasm32"))]
                state
                    .notifications
                    .notify(&target.iri, ActivityType::Delete, parent.as_deref())
                    .await;
                Ok(StatusCode::NO_CONTENT.into_response())
            }
            DeleteOutcome::NotEmpty => Err(ServerError::Conflict(
                "cannot delete a non-empty container".into(),
            )),
            DeleteOutcome::NotFound => Err(ServerError::NotFound),
        }
    } else {
        // A plain resource: the (non-atomic) removal is fine — there is no empty-check to race.
        state.store.delete(&target.iri, parent.as_deref()).await?;
        // A DELETE of an `.acl` removed the access rules (the resource now inherits): invalidate the
        // cached parse so the NEXT read no longer sees the deleted ACL's grants (the `meta` probe will
        // now report it absent and the walk inherits — invalidating frees the slot at once).
        state.invalidate_acl_if_acl(&target.iri);
        // EMIT: Delete on the resource + a derived Remove on its parent container.
        #[cfg(not(target_arch = "wasm32"))]
        state
            .notifications
            .notify(&target.iri, ActivityType::Delete, parent.as_deref())
            .await;
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

/// `PATCH /{path}` — apply a Solid N3 Patch (`text/n3`).
///
/// The patch is parsed (insert/delete plus the `solid:where` variable solver — see
/// [`crate::ldp::patch`] for the BGP-matching + exactly-one-solution semantics), applied to the
/// target's existing graph (a missing `deletes` triple ⇒ 409; a non-empty `where` with zero or
/// multiple solutions ⇒ 409), and the result re-serialised in the resource's stored format. PATCH on
/// a missing resource that only inserts creates it (the LDP "create on PATCH" convention); a PATCH
/// with deletes on a missing resource is a 409. `If-Match` is honoured. Fail-closed (public ⇒ 403).
pub async fn patch_handler<S: Store>(
    State(state): State<Arc<LdpState<S>>>,
    Extension(token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ServerError> {
    #[cfg(feature = "sparql-endpoint")]
    let _snapshot_guard = state.sparql_snapshot_write().await;
    let target = parse_target(&state.base_url, uri.path())?;

    // Select the PATCH language from the Content-Type (ABSENT ⇒ 400, unsupported ⇒ 415) and parse the
    // document. `text/n3` is the Solid N3 Patch; `application/sparql-update` is the INSERT/DELETE DATA
    // subset. Both reduce to an `N3Patch` the shared engine applies.
    //
    // Parse BEFORE authorizing because the required WAC mode depends on the patch CONTENT: an
    // INSERT-ONLY patch (no `solid:deletes`) needs only `acl:Append`; a patch with ANY delete needs
    // `acl:Write` (a delete removes existing triples). Parsing is SSRF-safe + bounded RDF parsing, and
    // the conformance deny cases accept `[403, 405, 415]`, so parse-then-authorize is correct.
    let patch = match classify_patch_media_type(header_str(&headers, header::CONTENT_TYPE))? {
        PatchKind::N3 => parse_n3_patch(&body, &target.iri)?,
        PatchKind::SparqlUpdate => parse_sparql_update(&body, &target.iri)?,
    };

    let origin = request_origin(&headers);

    // WAC for PATCH — EXISTENCE-NON-DISCLOSURE (`research/lws-design-records.md` §6): the required
    // mode is derived purely
    // from the patch CONTENT (already parsed) and authorized against the TARGET's effective ACL
    // (inherited via `acl:default` for a not-yet-existing target), **BEFORE any target-dependent
    // read/existence probe**, so create-on-PATCH and forbidden-modify are INDISTINGUISHABLE to an
    // under-authorized requester:
    //  - an INSERT-ONLY patch (no `solid:deletes`) needs `acl:Append`;
    //  - a patch with ANY delete needs `acl:Write` (a delete removes existing triples);
    //  - an `.acl` target needs `acl:Control` (the `authorize_mode` override).
    //
    // This UNIFIES the prior create-vs-modify split (which authorized create-on-PATCH via
    // `authorize_create` = parent-`acl:Append`). That split was the **V3** existence oracle: an agent
    // holding parent-`acl:Append` (e.g. a drop-box) but NOT the target's effective Append got a 2xx on a
    // free name (create path) vs a 401/403 on a taken-but-forbidden name (modify path) — leaking which
    // child names exist. Authorizing the SAME content-derived mode against the SAME (inherited) target
    // ACL for BOTH cases removes the oracle: create and forbidden-modify return byte-identical denials.
    // (CTH-safe: every `write-access-*` PATCH-fictive row that expects 2xx grants the agent inheritable
    // `acl:Append`/`acl:Write` — which the target's effective-ACL resolution picks up via `acl:default`;
    // the `acl:Control`-only fictive rows expect a denial, which Append-on-target rejects. The earlier
    // delete-on-missing closure is now just the general rule. See `research/lws-design-records.md` §6.)
    //
    // Authorizing BEFORE the target read closes the V3 timing channel too: the under-authorized denial
    // is returned with NO target-dependent read in its path, and the access decision reads ONLY `.acl`
    // resources (never the target's own bytes/meta).
    let has_deletes = !patch.deletes.is_empty();
    let required = if has_deletes {
        AccessMode::Write
    } else {
        AccessMode::Append
    };
    let granted = state
        .authorize_mode(&target, required, &token, origin)
        .await?;

    // V4 (`research/lws-design-records.md` §6) — the `solid:where` READ-gate. A patch carrying a
    // `solid:where` clause READS the target graph: `apply_patch` runs the BGP solver over the
    // target's CURRENT triples, and its outcome (exactly-one-solution ⇒ 2xx vs zero/many ⇒ 409, and
    // a missing target ⇒ empty graph ⇒ always 0 ⇒ 409) is a CONTENT/EXISTENCE oracle — the very
    // channel V4 closes for conditional HEADERS, but reachable through the patch BODY at only
    // `acl:Append`. So a `where`-bearing patch additionally requires the target's READ mode
    // (`acl:Read`, or `acl:Control` for an `.acl` — reading an `.acl`'s representation is a Control
    // op, and `granted` already holds Control for an authorized `.acl` writer). This matches CSS's
    // `N3PatchModesExtractor`, which adds `read` when the patch has `conditions`. Fold to the
    // requester's denial (401 anon / 403 auth) BEFORE the target read, so it adds no oracle of its
    // own. An unconditional (no-`where`) patch is unaffected.
    if !patch.conditions.is_empty() {
        let read_mode = if crate::authz::is_acl_resource(&target.iri) {
            AccessMode::Control
        } else {
            AccessMode::Read
        };
        if !granted.contains(&read_mode) {
            return Err(if token.web_id.is_none() {
                state.unauthenticated()
            } else {
                ServerError::Forbidden
            });
        }
    }

    // V4 (`research/lws-design-records.md` §6): a conditional precondition is a CONTENT-derived
    // validator — fold to the denial when the requester lacks `acl:Read` and sent a conditional
    // header, BEFORE the target read.
    state.guard_conditional_requires_read(&target.iri, &headers, &granted, &token)?;

    // The caller IS authorized. ONLY NOW load the current representation (an authorized writer is
    // entitled to learn create-vs-modify). Match the read into THREE states:
    //  - `Ok(r)`            → present (modify path);
    //  - `Err(NotFound)`    → absent  (create-on-PATCH / delete-on-missing path);
    //  - `Err(other)`       → a backend/blob inconsistency → surface the 500 (the caller is authorized,
    //                         so a 500 leaks nothing they could not already learn via a normal read).
    //
    // Because authorization already ran above, a non-`NotFound` store error can be propagated
    // immediately here WITHOUT an existence/state oracle: an UNAUTHORIZED caller never reaches this
    // line (they returned the uniform 401/403 above), so a 500 is only ever seen by a caller permitted
    // to read the target. (`ServerError` is not `Clone`; we distinguish present/absent via `current`.)
    let current: Option<crate::store::Resource> = match state.store.read(&target.iri).await {
        Ok(r) => Some(r),
        Err(ServerError::NotFound) => None,
        Err(e) => return Err(e),
    };

    // Apply preconditions against the current ETag.
    let current_etag = current.as_ref().map(|r| r.meta.etag.clone());
    conditional::require(eval_preconditions(
        header_str(&headers, header::IF_MATCH),
        header_str(&headers, header::IF_NONE_MATCH),
        current_etag.as_deref(),
    ))?;

    // Determine the existing triples + the stored format (default Turtle for a new resource).
    let (existing_triples, stored_format) = match &current {
        Some(res) => {
            let fmt = classify(Some(&res.meta.content_type)).unwrap_or(RdfFormat::Turtle);
            (parse_to_triples(fmt, &res.body, &target.iri)?, fmt)
        }
        None => {
            // Create-on-PATCH: only an insert-only patch can create a resource. A delete on a missing
            // resource is a 409 (apply_patch enforces the missing-delete precondition).
            (Vec::new(), RdfFormat::Turtle)
        }
    };

    let patched = apply_patch(&existing_triples, &patch)?;
    let new_body = serialize_triples(stored_format, &patched)?;

    let existed = current.is_some();
    let parent = parent_container(&target);

    let meta = if existed {
        state
            .store
            .write(
                &target.iri,
                Bytes::from(new_body),
                stored_format.media_type(),
            )
            .await?
    } else if crate::authz::is_acl_resource(&target.iri) {
        // Create-on-PATCH of an AUXILIARY `.acl` resource: it is NOT a contained child. Storing it via
        // `create_in_container` would add an `ldp:contains` edge to the parent (and a later DELETE would
        // skip parent-write authorization while still mutating containment). An `.acl` is an auxiliary
        // resource (Solid's auxiliary-resource model) — store it via a plain `write` so it carries no
        // containment edge. (Auth for `.acl` ops is Control, already enforced above.)
        state
            .store
            .write(
                &target.iri,
                Bytes::from(new_body),
                stored_format.media_type(),
            )
            .await?
    } else {
        // Create-on-PATCH: like PUT, create intermediate containers + wire the new resource into its
        // parent's `ldp:contains` (so the containment scenario's container GET lists it). An ancestor
        // that exists as a non-container is a conflict (`ensure_ancestor_containers`).
        //
        // WAC container-modification (same as PUT-create): materialising the new member (+ any missing
        // intermediate container) mutates the nearest existing ancestor's containment, so it requires
        // `acl:Append` on THAT container — in addition to the content-derived target-ACL mode above.
        state
            .authorize_container_modification(&target.iri, &token, origin)
            .await?;
        ensure_ancestor_containers(state.as_ref(), &target.iri).await?;
        match &parent {
            Some(p) => {
                state
                    .store
                    .create_in_container(
                        p,
                        &target.iri,
                        Bytes::from(new_body),
                        stored_format.media_type(),
                    )
                    .await?
            }
            None => {
                state
                    .store
                    .write(
                        &target.iri,
                        Bytes::from(new_body),
                        stored_format.media_type(),
                    )
                    .await?
            }
        }
    };

    // EMIT (same shape as PUT): a patch that edited an existing resource ⇒ Update; a create-on-PATCH
    // ⇒ Create + a parent membership Add.
    //
    // EXCEPTION — an AUXILIARY `.acl` resource is NOT a contained child (create-on-PATCH stored it via
    // a plain `write`, adding NO `ldp:contains` edge to the parent). So its parent membership did NOT
    // change even on a create: pass `None` for the emit parent so the hub does NOT derive a spurious
    // container-membership `Add` for a resource the container does not actually contain.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let activity = if existed {
            ActivityType::Update
        } else {
            ActivityType::Create
        };
        let emit_parent = if existed || crate::authz::is_acl_resource(&target.iri) {
            None
        } else {
            parent.clone()
        };
        state
            .notifications
            .notify(&target.iri, activity, emit_parent.as_deref())
            .await;
    }

    // A PATCH to an `.acl` resource edited the access rules: invalidate the cached parse so the NEXT
    // read resolves against the patched ACL immediately.
    state.invalidate_acl_if_acl(&target.iri);

    Ok(write_response(existed, &meta, &target.iri))
}

/// `OPTIONS /{path}` — advertise the methods + write media types for a target (RFC 9110 §9.3.7 +
/// the Solid Protocol `Accept-Post`/`Accept-Patch`).
///
/// Returns **204 No Content** (an empty body) with:
/// - `Allow`: the LDP verb set the server supports (`OPTIONS, HEAD, GET, PUT, POST, DELETE, PATCH`);
/// - `Accept-Post`: the container POST media types (`text/turtle`, `application/ld+json`);
/// - `Accept-Patch`: the PATCH media types (`text/n3`, `application/sparql-update`).
///
/// OPTIONS is NOT auth-gated (it is metadata about the surface, not a read of content) and is the
/// path the CORS preflight rides on — the `CorsLayer` adds the `Access-Control-*` headers to this
/// response. The `read-method-support` / `read-method-allow` scenarios require OPTIONS ≠ 405 and an
/// `Allow` listing GET + HEAD.
pub async fn options_handler<S: Store>(
    State(_state): State<Arc<LdpState<S>>>,
    Extension(_token): Extension<VerifiedToken>,
    uri: axum::http::Uri,
) -> Result<Response, ServerError> {
    let is_container = uri.path().ends_with('/');
    let mut out = HeaderMap::new();
    add_method_advertisement(&mut out, is_container);
    Ok((StatusCode::NO_CONTENT, out).into_response())
}

/// The LDP method-advertisement headers (`Allow` + `Accept-Post` on containers + `Accept-Patch`),
/// shared by the OPTIONS handler and the GET/HEAD read response.
///
/// - `Allow`: the full LDP verb set.
/// - `Accept-Post` (Solid Protocol — containers accept POST): the container POST media types. Only a
///   container advertises it (POST to a non-container is not a containment op).
/// - `Accept-Patch`: the PATCH media types (`text/n3`, `application/sparql-update`).
fn add_method_advertisement(headers: &mut HeaderMap, is_container: bool) {
    headers.insert(header::ALLOW, HV_ALLOW.clone());
    if is_container {
        headers.insert(
            HeaderName::from_static("accept-post"),
            HV_ACCEPT_POST.clone(),
        );
    }
    headers.insert(
        HeaderName::from_static("accept-patch"),
        HV_ACCEPT_PATCH.clone(),
    );
}

// --- helpers -----------------------------------------------------------------------------------

/// Require a non-empty `Content-Type` on a write (Solid Protocol — `content-type-reject`). An ABSENT
/// or empty Content-Type is a **400 Bad Request**. Distinguishing absent (400) from
/// present-but-unsupported (handled by [`validate_writable`]) is the point of this helper.
fn require_content_type(headers: &HeaderMap) -> Result<String, ServerError> {
    match header_str(headers, header::CONTENT_TYPE) {
        Some(ct) if !ct.trim().is_empty() => Ok(ct.to_string()),
        _ => Err(ServerError::BadRequest(
            "a write request must declare a Content-Type".into(),
        )),
    }
}

/// Validate a write body for its declared `content_type` and return the media type to store it under.
///
/// - An **RDF** type (`text/turtle` / `application/ld+json`) is parse-validated (a malformed body is a
///   400) so the index/byte stores never hold a non-parseable "RDF" resource.
/// - A **NON-RDF** type (`text/plain`, an image, …) is stored VERBATIM as an opaque binary resource —
///   the Solid Protocol permits storing any content type, and a read serves a binary body unchanged
///   (`negotiate_body`). The CORS scenarios create `text/plain` resources, so this path is required.
///
/// The returned media type is the declared one's essence (parameters trimmed) for an RDF type, or the
/// declared value verbatim for a binary type.
fn validate_writable(
    content_type: &str,
    body: &Bytes,
    base_iri: &str,
) -> Result<String, ServerError> {
    match classify(Some(content_type)) {
        Ok(format) => {
            // RDF: validate the body parses in its declared format (relative IRIs against base_iri).
            validate_rdf(format, body, base_iri)?;
            Ok(format.media_type().to_string())
        }
        // A non-RDF type is an opaque binary resource — store the declared content type verbatim.
        Err(ServerError::UnsupportedMediaType(_)) => Ok(content_type.trim().to_string()),
        Err(e) => Err(e),
    }
}

/// Synthesise a container's LDP representation and content-negotiate it.
///
/// The body MERGES two triple sources, built from `oxrdf` triples (never hand-concatenated — the
/// house rule) and serialised with the server's own RDF serialiser:
/// - **The container's OWN stored RDF** (whatever was PUT to the container, or POSTed as its body):
///   parsed from `stored_body` in its stored format and carried through, so RDF written to a
///   container stays retrievable on GET. A non-RDF / unparseable stored body contributes no triples
///   (a container's body is conventionally RDF or empty).
/// - **The generated LDP containment triples** — `<container> rdf:type ldp:Resource, ldp:Container,
///   ldp:BasicContainer` and `<container> ldp:contains <child>` for each authoritative
///   `store.list_children` member.
///
/// The two sets are de-duplicated (a stored triple identical to a generated one is not repeated). The
/// negotiated format honours the `Accept` header (Turtle / JSON-LD), defaulting to the container's
/// stored format when it is RDF (else Turtle); an Accept naming no producible type falls back to
/// Turtle (the Solid default), and only an explicit q=0 refusal of every covered type is a 406.
async fn render_container<S: Store>(
    state: &Arc<LdpState<S>>,
    container_iri: &str,
    stored_body: &Bytes,
    stored_content_type: &str,
    accept: Option<&str>,
) -> Result<(Bytes, String), ServerError> {
    // The container's stored bytes default to a Turtle representation; if the stored type is RDF, use
    // it as the conneg default (most faithful) and parse the stored body for its own triples.
    let stored_format = classify(Some(stored_content_type)).ok();
    let default_format = stored_format.unwrap_or(RdfFormat::Turtle);
    let negotiated =
        negotiate_accept_with_profile(accept, default_format).ok_or(ServerError::NotAcceptable)?;

    let subject = NamedNode::new(container_iri)
        .map_err(|e| ServerError::Storage(format!("invalid container IRI {container_iri}: {e}")))?;
    let rdf_type = RDF_TYPE_NODE.clone();
    let contains = LDP_CONTAINS_NODE.clone();

    // 1) The container's OWN stored RDF (whatever was written to the container itself). Parse it in
    // its stored format, resolving relative IRIs against the container IRI. If the stored body is
    // non-RDF or unparseable, it contributes nothing (a container body is conventionally RDF/empty) —
    // we never fail the listing over a stored body the server itself stored.
    //
    // The stored set is carried through VERBATIM (no intra-set de-dup) — exactly as before — so a
    // container body that literally repeats a triple keeps both occurrences (the serialised bytes,
    // and hence the representation ETag, stay identical to the prior linear-scan render).
    let stored_triples: Vec<Triple> = match stored_format {
        Some(fmt) => parse_to_triples(fmt, stored_body, container_iri).unwrap_or_default(),
        None => Vec::new(),
    };

    // Build the output Vec DIRECTLY — no whole-graph `HashSet<Triple>` clone-dedup. The previous code
    // seeded a `HashSet` from `stored_triples.iter().cloned()` and clone-inserted every generated
    // triple; both are pure allocation that the structure of the data renders unnecessary:
    //
    //   * the three `rdf:type` triples are mutually distinct, and
    //   * the `ldp:contains` triples are distinct from one another (the index lists each child once —
    //     unique by construction: an RDF graph holds a containment edge at most once, both store impls
    //     enforce a child appears once),
    //
    // so the ONLY suppression the old dedup could ever fire was a GENERATED triple that duplicates a
    // STORED one (exactly the `BasicContainer`-in-stored-body case the byte-identity test pins). We
    // preserve that — and only that — with a membership check against ONLY the stored set, which is
    // empty for the overwhelmingly common empty/typing-free container body, so the hot path does zero
    // membership work. Insertion order + which triples appear are unchanged, so the serialiser emits
    // the same bytes and `representation_etag` is preserved byte-for-byte.
    let children = state.store.list_children(container_iri).await?;
    let stored_len = stored_triples.len();

    // Suppress ONLY a GENERATED triple that duplicates a STORED one. There are exactly `3 + N`
    // generated triples to probe against the stored set, so the membership structure is chosen by the
    // stored-body size to avoid BOTH a per-render `HashSet` allocation on the common path AND an
    // O(stored_len * (3 + N)) cliff on a pathological large-stored-body-plus-many-children container:
    //   * empty stored body (the overwhelmingly common case)  → no membership work at all;
    //   * SMALL stored body (≤ DEDUP_HASHSET_THRESHOLD triples) → a zero-allocation linear `contains`
    //     scan of the stored slice (cheaper than building+hashing a set for a handful of triples);
    //   * LARGE stored body                                    → build a borrowing `HashSet<&Triple>`
    //     of `stored_triples` ONCE (no clones — references into the still-owned Vec) and probe it O(1),
    //     capping the worst case at O(stored_len + (3 + N)) as the old whole-graph HashSet did.
    // All three branches suppress EXACTLY the same triples (a generated triple present in the stored
    // set), so the output bytes + `representation_etag` are identical regardless of which path runs.
    //
    // The generated triples are collected into their own Vec FIRST (so the membership probe can borrow
    // the still-owned `stored_triples`); the final `triples` is then `stored ++ generated`, preserving
    // the prior "stored set verbatim, then generated in order" layout the byte-identity test pins.
    const DEDUP_HASHSET_THRESHOLD: usize = 16;
    let stored_set: Option<std::collections::HashSet<&Triple>> =
        (stored_len > DEDUP_HASHSET_THRESHOLD).then(|| stored_triples.iter().collect());
    let mut generated: Vec<Triple> = Vec::with_capacity(3 + children.len());
    let push_generated = |generated: &mut Vec<Triple>, triple: Triple| {
        let in_stored = match &stored_set {
            // Large stored body: O(1) hashed membership against the borrowed stored set.
            Some(set) => set.contains(&triple),
            // Empty/small stored body: linear scan of the stored slice (zero allocation).
            None => stored_len != 0 && stored_triples.contains(&triple),
        };
        if !in_stored {
            generated.push(triple);
        }
    };

    // 2) The generated LDP typing triples.
    push_generated(
        &mut generated,
        Triple::new(subject.clone(), rdf_type.clone(), LDP_RESOURCE_NODE.clone()),
    );
    push_generated(
        &mut generated,
        Triple::new(
            subject.clone(),
            rdf_type.clone(),
            LDP_CONTAINER_NODE.clone(),
        ),
    );
    push_generated(
        &mut generated,
        Triple::new(subject.clone(), rdf_type, LDP_BASIC_CONTAINER_NODE.clone()),
    );

    // 3) The generated `ldp:contains` membership triples (one per authoritative child). Each child is a
    // [`ValidatedChildIri`](crate::store::ValidatedChildIri) — RFC-3987-VALIDATED at the
    // `Store::list_children` boundary (bead wg3), which is where a malformed/injected storage row first
    // crosses into the server's own logic. So the render receives a full-RFC-3987-valid `NamedNode` and
    // moves it straight into the term — NO per-child re-parse and NO structural guard needed here (both
    // moved to the boundary). A malformed backend row was already fail-closed OMITTED at that boundary,
    // so it can never reach this loop; there is no invalid-but-serialisable residual left to leak in
    // release (the roborev Medium this closes).
    for child in children {
        push_generated(
            &mut generated,
            Triple::new(subject.clone(), contains.clone(), child.into_named_node()),
        );
    }

    // Assemble `stored ++ generated` — the prior layout (stored set verbatim, then the generated set
    // in order). `stored_set` (which borrowed `stored_triples`) is dropped here, so the owned
    // `stored_triples` can now be moved into the output without a clone.
    drop(stored_set);
    let mut triples: Vec<Triple> = Vec::with_capacity(stored_len + generated.len());
    triples.extend(stored_triples);
    triples.extend(generated);

    // Serialise into the negotiated format AND JSON-LD document form; the `Content-Type` echoes
    // any honoured profile. The representation ETag the caller derives from these bytes is
    // profile-variant-specific exactly when the bytes differ (compacted output differs; expanded
    // output is byte-identical to the default serialisation).
    let bytes = serialize_triples_negotiated(negotiated, &triples)?;
    Ok((Bytes::from(bytes), negotiated.content_type()))
}

/// A STRONG ETag computed from a rendered representation's BYTES — `"<len>-<hash>"`.
///
/// Used for a container response, whose body is generated from live membership (not stored bytes), so
/// the validator must track the actual representation: it changes whenever the serialised body changes
/// (a child added/removed, or the negotiated format differs). The same body computed for GET and HEAD
/// yields the same validator, so the two methods agree. This is a non-cryptographic content hash
/// (FNV-1a over the bytes), sufficient for a cache validator — collisions across distinct
/// representations are vanishingly unlikely and the length prefix further disambiguates.
fn representation_etag(body: &[u8]) -> String {
    // FNV-1a 64-bit over the serialised representation.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in body {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("\"{}-{:x}\"", body.len(), hash)
}

/// Ensure every ANCESTOR container of `iri` exists, creating any that are missing and wiring each into
/// its own parent's `ldp:contains` (Solid Protocol — PUT/PATCH create intermediate containers). The
/// resource `iri` itself is NOT created here (the caller does that). Walks ROOT→down so a parent
/// always exists before its child is wired.
///
/// **Conflict:** if an ancestor PATH already exists as a NON-container resource (a plain resource
/// cannot have children — the slash-semantics invariant), this is a 409 Conflict (`containment`
/// "conflicts when … turning resource into container"). The conflict is detected by the
/// trailing-slash container record being absent while the slash-less resource record is present.
async fn ensure_ancestor_containers<S: Store>(
    state: &LdpState<S>,
    iri: &str,
) -> Result<(), ServerError> {
    let base = state.base_url.trim_end_matches('/');
    let Some(rest) = iri.strip_prefix(base) else {
        return Ok(());
    };

    // The storage ROOT container `<base>/` is the ancestor of EVERYTHING; ensure it exists first (a
    // parentless write mints its record) so the walk below can wire each child into a present parent.
    let root = format!("{base}/");
    if !state.store.exists(&root).await? {
        state
            .store
            .write(&root, Bytes::new(), RdfFormat::Turtle.media_type())
            .await?;
    }

    // Interior path segments, excluding the resource's own final segment. e.g. for
    // `/a/b/c.txt` the ancestor containers are `/`, `/a/`, `/a/b/`.
    let path = rest.trim_start_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() <= 1 {
        // Resource is a direct child of the base root — only the root container is its ancestor, and it
        // now exists.
        return Ok(());
    }

    // Build each ancestor container IRI incrementally and ensure it exists.
    let mut prefix = String::from(base);
    let mut parent = root.clone();
    // Ancestor containers are all segments EXCEPT the last (the resource name).
    for seg in &segments[..segments.len() - 1] {
        prefix.push('/');
        prefix.push_str(seg);
        let container = format!("{prefix}/");

        // A pre-existing NON-container at this path (the slash-less form) ⇒ conflict.
        let slashless = prefix.clone();
        if state.store.exists(&slashless).await? && !state.store.exists(&container).await? {
            return Err(ServerError::Conflict(
                "an ancestor path already exists as a non-container resource".into(),
            ));
        }

        if !state.store.exists(&container).await? {
            // Create the missing intermediate container, wired into its parent's containment.
            state
                .store
                .create_in_container(
                    &parent,
                    &container,
                    Bytes::new(),
                    RdfFormat::Turtle.media_type(),
                )
                .await?;
        }
        parent = container;
    }
    Ok(())
}

/// Reject a PUT whose URI collides with an existing resource of the OPPOSITE slash-kind: a
/// trailing-slash container IRI and the slash-less resource IRI MUST NOT co-exist (Solid Protocol —
/// "with and without trailing slash cannot co-exist"). A collision is a **409 Conflict**.
async fn reject_slash_semantics_conflict<S: Store>(
    state: &LdpState<S>,
    target: &LdpTarget,
) -> Result<(), ServerError> {
    let opposite = if target.is_container {
        // Container `…/foo/` collides with resource `…/foo`.
        target.iri.trim_end_matches('/').to_string()
    } else {
        // Resource `…/foo` collides with container `…/foo/`.
        format!("{}/", target.iri)
    };
    if state.store.exists(&opposite).await? {
        return Err(ServerError::Conflict(
            "a resource and a container cannot share the same path (trailing-slash semantics)"
                .into(),
        ));
    }
    Ok(())
}

/// The shared 201/204 + ETag (+ Location on create) response for PUT / PATCH writes.
fn write_response(existed: bool, meta: &ResourceMeta, iri: &str) -> Response {
    let status = if existed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };
    let mut out = HeaderMap::new();
    set_str(&mut out, header::ETAG, &meta.etag);
    if !existed {
        set_str(&mut out, header::LOCATION, iri);
    }
    (status, out).into_response()
}

/// Content-negotiate the response body for an RDF resource. For a non-RDF stored type the body is
/// returned verbatim. For an RDF type, the stored bytes are re-serialised into the negotiated
/// format when it differs from the stored one — or when the client's explicit `application/ld+json`
/// range carried an honoured `profile` (expanded / compacted): the stored bytes are whatever
/// document form was originally written, so an honoured profile always re-serialises into the
/// requested form and the `Content-Type` echoes the profile. A client that EXPLICITLY refuses
/// (q=0) every producible type it covers ⇒ 406; an Accept naming no producible type falls back to
/// Turtle (Solid default).
fn negotiate_body(
    stored_body: &Bytes,
    stored_content_type: &str,
    accept: Option<&str>,
    base_iri: &str,
) -> Result<(Bytes, String), ServerError> {
    let stored_format = match classify(Some(stored_content_type)) {
        Ok(f) => f,
        // Non-RDF stored content (binary): no RDF conneg — serve verbatim. (A future slice can do
        // generic media-type matching; for now any Accept is satisfied by the stored bytes.)
        Err(_) => return Ok((stored_body.clone(), stored_content_type.to_string())),
    };

    let negotiated =
        negotiate_accept_with_profile(accept, stored_format).ok_or(ServerError::NotAcceptable)?;
    if negotiated.serves_stored_verbatim(stored_format) {
        return Ok((stored_body.clone(), stored_content_type.to_string()));
    }
    // Re-serialise into the chosen format + document form.
    let triples = parse_to_triples(stored_format, stored_body, base_iri)?;
    let bytes = serialize_triples_negotiated(negotiated, &triples)?;
    Ok((Bytes::from(bytes), negotiated.content_type()))
}

/// The validator (ETag) the response serving this PLAIN resource under `accept` carries —
/// representation-specific per RFC 9110 §8.8.3 (an entity-tag identifies a representation, not a
/// resource), and computed WITHOUT serialising a body (the read-304 fast path):
///
/// - non-RDF stored content: no RDF conneg, a single representation ⇒ the stored tag, whatever the
///   `Accept` (matches [`negotiate_body`]'s verbatim branch);
/// - RDF served in its STORED format with no honoured JSON-LD profile ⇒ the stored tag (the bytes
///   ARE the stored representation);
/// - RDF re-serialised (the other format, or an honoured `profile` on an explicit
///   `application/ld+json` range) ⇒ the [`conditional::variant_etag`] `"<state>+<variant>"` tag,
///   distinct per negotiated byte-representation and derived from the stored state — so the tag a
///   200 carries for each negotiated representation is exactly the tag that later 304s for it, and
///   the write path can round-trip its state part (`conditional` module doc). The variant token
///   ([`crate::ldp::content::NegotiatedFormat::variant_suffix`]) is profile-specific exactly when
///   the bytes differ: `compacted` gets its own token, while `expanded` output is byte-identical
///   to the default JSON-LD serialisation and shares its token;
/// - an `Accept` that explicitly refuses (q=0) every producible type it covers ⇒ 406 (no selected
///   representation, no validator); one merely naming no producible type falls back to Turtle.
///
/// The decision is the same [`negotiate_accept_with_profile`] call [`negotiate_body`] makes (via
/// the shared `serves_stored_verbatim` rule), so the validator and the body it labels can never
/// disagree.
fn negotiated_validator(
    stored_etag: &str,
    stored_content_type: &str,
    accept: Option<&str>,
) -> Result<String, ServerError> {
    let Ok(stored_format) = classify(Some(stored_content_type)) else {
        // Non-RDF stored content (binary): served verbatim — one representation, the stored tag.
        return Ok(stored_etag.to_string());
    };
    let negotiated =
        negotiate_accept_with_profile(accept, stored_format).ok_or(ServerError::NotAcceptable)?;
    Ok(if negotiated.serves_stored_verbatim(stored_format) {
        stored_etag.to_string()
    } else {
        conditional::variant_etag(stored_etag, negotiated.variant_suffix())
    })
}

/// Whether a POST asks for a CONTAINER child via `Link: <ldp#BasicContainer>; rel="type"` (or
/// `ldp:Container`) — LDP §5.2.3.4 container creation. Matched across (possibly multiple) `Link`
/// header lines, case-insensitively on the rel + the LDP container type IRI.
fn wants_container_via_link(headers: &HeaderMap) -> bool {
    headers.get_all(header::LINK).iter().any(|v| {
        let Ok(s) = v.to_str() else { return false };
        let lower = s.to_ascii_lowercase();
        lower.contains("rel=\"type\"")
            && (lower.contains("ldp#basiccontainer") || lower.contains("ldp#container"))
    })
}

/// Mint a child IRI within `container`. A `Slug` (sanitised) is used ONLY as a NON-binding PREFIX of a
/// server-generated, collision-free, **opaque** name — NEVER as the verbatim final segment.
///
/// V2 — EXISTENCE-NON-DISCLOSURE for the `Location` header (`research/lws-design-records.md` §6;
/// RSS `decisions/0003`). The prior mint returned
/// the verbatim `…/<slug>` when that name was FREE but a mangled `…/<slug>-<opaque>` when it was TAKEN.
/// A POST always returns 201, so the *shape* of the `Location` was the only difference — and it leaked
/// whether `<slug>` already existed in the container to any caller who can POST (an `acl:Append`
/// holder) but cannot READ the container's listing. By ALWAYS appending the opaque suffix (whether or
/// not `<slug>` is free), the `Location` shape is collision-INDEPENDENT — it carries no existence
/// signal — while STILL CONTAINING the Slug substring (the Solid Protocol treats `Slug` as a hint and
/// the conformance `post-uri-assignment-slug` row asserts only `Location contains '<slug>'`, which an
/// opaque-suffixed name satisfies). A name with no usable Slug falls back to the default `resource-…`
/// stem, identical in shape, so the two cases are indistinguishable.
///
/// `generate_unique` does the `exists` probe + retry internally, so the returned IRI is guaranteed
/// free (and, being opaque, never collides with the trailing-slash opposite form either — the old
/// `slash_form_taken` co-existence probe is no longer needed). When `as_container` is set the minted
/// IRI ends in `/` (an LDP container child). `stem` is the ALREADY-SANITISED Slug (the caller
/// sanitises + `.acl`-guards it before this point), used only as the opaque name's prefix.
async fn mint_child_iri<S: Store>(
    store: &S,
    container_iri: &str,
    stem: Option<&str>,
    as_container: bool,
) -> Result<String, ServerError> {
    let base = container_iri.trim_end_matches('/');
    generate_unique(store, base, stem, as_container).await
}

/// Generate a unique child IRI under `base`, optionally seeded by `stem`. Deterministic-but-unique:
/// a monotonic counter + the stem, retried until the index reports it free. A container child gets a
/// trailing slash.
async fn generate_unique<S: Store>(
    store: &S,
    base: &str,
    stem: Option<&str>,
    as_container: bool,
) -> Result<String, ServerError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::UNIX_EPOCH;
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let prefix = stem.unwrap_or("resource");
    let suffix = if as_container { "/" } else { "" };
    // Seed with a coarse timestamp so names are unique across process restarts too.
    let seed = crate::clock::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    for attempt in 0..64u64 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = format!("{base}/{prefix}-{seed:x}-{n:x}-{attempt:x}{suffix}");
        if !store.exists(&candidate).await? {
            return Ok(candidate);
        }
    }
    Err(ServerError::Storage(
        "could not mint a unique child IRI".into(),
    ))
}

/// Sanitise a `Slug` into a safe single path segment: keep `[A-Za-z0-9._-]`, drop everything else
/// (including `/`, `:`, `%`, whitespace, `.`/`..`). Returns `None` if nothing usable remains. This
/// is defence-in-depth — the minted IRI is also re-validated by [`parse_target`]'s traversal guard.
fn sanitise_slug(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    // Reject path-traversal-ish remnants and empties.
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return None;
    }
    Some(cleaned)
}

/// Derive the parent container IRI of a target (for detaching containment on DELETE). The parent is
/// the IRI up to and including the last interior slash. The root has no parent.
fn parent_container(target: &LdpTarget) -> Option<String> {
    // Strip a trailing slash for a container target so we find its PARENT, not itself.
    let iri = target.iri.trim_end_matches('/');
    // Find the last '/' that is part of the path (after the scheme's "//").
    let scheme_end = iri.find("://").map(|i| i + 3).unwrap_or(0);
    let path_part = &iri[scheme_end..];
    match path_part.rfind('/') {
        Some(rel) => {
            let abs = scheme_end + rel;
            // Include the slash so the parent is itself a container IRI.
            Some(iri[..=abs].to_string())
        }
        None => None,
    }
}

/// Append the notification-discovery `Link` headers (`describedby` + `solid:storageDescription`,
/// both → the storage description doc) to a read response. Uses `append` (not `insert`) so multiple
/// rels coexist as separate `Link` header lines.
///
/// `values` is the server's PRECOMPUTED [`LdpState::discovery_link_values`] — derived once from
/// `base_url` at construction, so this is a couple of refcount-bump `append`s with NO per-request
/// formatting/allocation. The emitted lines are byte-for-byte identical to the prior per-request
/// `link_headers(base_url)` formatting.
fn add_discovery_links(headers: &mut HeaderMap, values: &[HeaderValue]) {
    for v in values {
        headers.append(header::LINK, v.clone());
    }
}

/// Append the LDP/Solid `Link: <type>; rel="type"` advertisement headers for a read response.
///
/// - Any resource advertises `ldp:Resource`.
/// - A container additionally advertises `ldp:Container` + `ldp:BasicContainer` (the LDP type a Solid
///   container exposes).
/// - The STORAGE ROOT container additionally advertises `pim:Storage` — the Solid Protocol §4.1
///   storage-advertisement the conformance harness reads at bootstrap to recognise the pod. With the
///   in-memory/seeded layout the storage root is the per-user pod container `…/{user}/`; treat any
///   container that is a direct child of the server base (`<base>/{seg}/`) as a storage root.
///
/// Uses `append` so each rel is its own `Link` header line.
///
/// Each emitted value is one of the REQUEST-INVARIANT interned `HV_LINK_TYPE_*` `HeaderValue`s (the
/// full `Link` line is a compile-time constant per resource shape), so this allocates NOTHING per
/// response — only `clone`s (a refcount bump) the static value into the map. WHICH rels appear, and
/// in WHAT ORDER (`Resource`, then `Container`+`BasicContainer`, then `pim:Storage` on a storage
/// root), is unchanged, so the emitted header lines are byte-for-byte identical to the prior
/// per-request `format!` + `HeaderValue::from_str` path.
fn add_type_links(headers: &mut HeaderMap, target: &LdpTarget, base_url: &str) {
    headers.append(header::LINK, HV_LINK_TYPE_LDP_RESOURCE.clone());
    if target.is_container {
        headers.append(header::LINK, HV_LINK_TYPE_LDP_CONTAINER.clone());
        headers.append(header::LINK, HV_LINK_TYPE_LDP_BASIC_CONTAINER.clone());
        if is_storage_root(&target.iri, base_url) {
            headers.append(header::LINK, HV_LINK_TYPE_PIM_STORAGE.clone());
        }
    }
}

/// Append the `Link: <acl-url>; rel="acl"` ACL-discovery header (Solid Protocol §4.3.1).
///
/// The ACL URL follows the conventional sibling-document layout: a container `…/c/` → `…/c/.acl`; a
/// plain resource `…/r` → `…/r.acl`. Skipped if the value cannot be header-encoded.
fn add_acl_link(headers: &mut HeaderMap, target: &LdpTarget) {
    let acl_url = acl_url_for(target);
    let value = format!("<{acl_url}>; rel=\"acl\"");
    if let Ok(v) = HeaderValue::from_str(&value) {
        headers.append(header::LINK, v);
    }
}

/// The conventional ACL document URL for a target: `…/c/.acl` for a container `…/c/` (its IRI ends in
/// `/`, so `{iri}.acl` is `…/c/.acl`), and `…/r.acl` for a resource `…/r`. The same `{iri}.acl`
/// suffix yields both.
fn acl_url_for(target: &LdpTarget) -> String {
    format!("{}.acl", target.iri)
}

/// Whether `iri` is a storage-root container: a container that is a DIRECT child of the server base
/// (`<base>/<segment>/`, exactly one interior path segment). The seeded per-user pods (`…/alice/`,
/// `…/bob/`) are storage roots; deeper containers (`…/alice/profile/`) are not.
fn is_storage_root(iri: &str, base_url: &str) -> bool {
    let base = base_url.trim_end_matches('/');
    let Some(rest) = iri.strip_prefix(base) else {
        return false;
    };
    // rest is the absolute path, e.g. "/alice/". A storage root has exactly one non-empty segment
    // and a trailing slash.
    let inner = rest.trim_start_matches('/').trim_end_matches('/');
    !inner.is_empty() && !inner.contains('/') && rest.ends_with('/')
}

/// Read a header value as `&str`, or `None` if absent / not valid UTF-8.
fn header_str(headers: &HeaderMap, name: HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The request's `Origin` header (the requesting web app's origin), trimmed; `None` if absent, empty,
/// or not valid UTF-8. Threaded into WAC so an `acl:origin`-restricted authorization grants only from
/// a matching Origin (and a request with no Origin never satisfies such a rule — fail-closed). A bare
/// `Origin: null` is treated as a present-but-non-matching opaque origin (kept verbatim — it will only
/// match a literal `acl:origin <null>`, which is not a real grant).
///
/// `pub(crate)` so the pre-crypto public-read skip middleware reads the request Origin EXACTLY as the
/// handler does (same trim/empty-filter) — the skip's origin input is byte-identical to the read
/// path's, preserving `acl:origin` fail-closed semantics (INV-6).
pub(crate) fn request_origin(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, header::ORIGIN)
        .map(str::trim)
        .filter(|o| !o.is_empty())
}

/// Insert a header value, silently skipping a value that cannot be encoded (never panics).
fn set_str(headers: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(name, v);
    }
}

/// Insert an INTEGER header value (e.g. `Content-Length`) WITHOUT a heap allocation. `itoa` formats
/// the integer into a stack buffer, and the resulting all-ASCII-digit string is a valid header value
/// by construction, so `HeaderValue::from_str` never fails on it (it cannot, but we still skip on the
/// impossible error rather than `unwrap`, mirroring `set_str`'s never-panic contract). This replaces
/// the `u64::to_string()` heap `String` the read response built per request on the MALLOC-bound
/// public-GET path — the value is byte-identical (decimal, no separators).
fn set_u64(headers: &mut HeaderMap, name: header::HeaderName, value: u64) {
    let mut buf = itoa::Buffer::new();
    if let Ok(v) = HeaderValue::from_str(buf.format(value)) {
        headers.insert(name, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(iri: &str) -> LdpTarget {
        LdpTarget {
            htu: iri.to_string(),
            iri: iri.to_string(),
            is_container: iri.ends_with('/'),
        }
    }

    #[cfg(feature = "sparql-endpoint")]
    #[tokio::test]
    async fn sparql_snapshot_read_guard_blocks_an_ldp_write_guard() {
        // [GPT-5.6] sq-r1ei8 mutation witness: replacing the shared barrier
        // with independent guards lets this writer finish during the snapshot.
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let read = state.sparql_snapshot_read().await;
        let writer_state = Arc::clone(&state);
        let mut writer = tokio::spawn(async move {
            let _write = writer_state.sparql_snapshot_write().await;
        });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut writer)
                .await
                .is_err(),
            "an LDP mutation must wait until the query snapshot is released"
        );
        drop(read);
        tokio::time::timeout(std::time::Duration::from_secs(1), writer)
            .await
            .expect("writer resumes after the snapshot")
            .expect("writer task completes");
    }

    fn link_values(headers: &HeaderMap) -> Vec<String> {
        headers
            .get_all(header::LINK)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn storage_root_is_a_direct_base_child_container() {
        let base = "https://localhost:3000";
        assert!(is_storage_root("https://localhost:3000/alice/", base));
        assert!(is_storage_root("https://localhost:3000/bob/", base));
        // Nested containers are NOT storage roots.
        assert!(!is_storage_root(
            "https://localhost:3000/alice/profile/",
            base
        ));
        assert!(!is_storage_root("https://localhost:3000/alice/test/", base));
        // The base root itself is not a per-user storage root.
        assert!(!is_storage_root("https://localhost:3000/", base));
        // A plain resource (no trailing slash) is not a storage root.
        assert!(!is_storage_root("https://localhost:3000/alice", base));
    }

    #[test]
    fn acl_url_is_the_dot_acl_sibling() {
        // Container: …/c/ → …/c/.acl
        assert_eq!(
            acl_url_for(&target("https://localhost:3000/alice/test/")),
            "https://localhost:3000/alice/test/.acl"
        );
        // Resource: …/r → …/r.acl
        assert_eq!(
            acl_url_for(&target("https://localhost:3000/alice/profile/card")),
            "https://localhost:3000/alice/profile/card.acl"
        );
    }

    #[test]
    fn storage_root_advertises_pim_storage_and_ldp_types() {
        let mut h = HeaderMap::new();
        let base = "https://localhost:3000";
        let t = target("https://localhost:3000/alice/");
        add_type_links(&mut h, &t, base);
        let links = link_values(&h);
        assert!(links
            .iter()
            .any(|l| l.contains("ldp#Resource") && l.contains("rel=\"type\"")));
        assert!(links.iter().any(|l| l.contains("ldp#Container")));
        assert!(links.iter().any(|l| l.contains("ldp#BasicContainer")));
        assert!(
            links.iter().any(|l| l.contains("pim/space#Storage")),
            "the storage root MUST advertise pim:Storage (harness bootstrap requirement): {links:?}"
        );
    }

    #[test]
    fn nested_container_advertises_ldp_types_but_not_pim_storage() {
        let mut h = HeaderMap::new();
        let base = "https://localhost:3000";
        add_type_links(
            &mut h,
            &target("https://localhost:3000/alice/profile/"),
            base,
        );
        let links = link_values(&h);
        assert!(links.iter().any(|l| l.contains("ldp#BasicContainer")));
        assert!(!links.iter().any(|l| l.contains("pim/space#Storage")));
    }

    #[test]
    fn plain_resource_advertises_only_ldp_resource_type() {
        let mut h = HeaderMap::new();
        let base = "https://localhost:3000";
        add_type_links(
            &mut h,
            &target("https://localhost:3000/alice/profile/card"),
            base,
        );
        let links = link_values(&h);
        assert!(links.iter().any(|l| l.contains("ldp#Resource")));
        assert!(!links.iter().any(|l| l.contains("ldp#Container")));
        assert!(!links.iter().any(|l| l.contains("pim/space#Storage")));
    }

    #[test]
    fn acl_link_header_is_emitted() {
        let mut h = HeaderMap::new();
        add_acl_link(&mut h, &target("https://localhost:3000/alice/test/"));
        let links = link_values(&h);
        assert!(
            links
                .iter()
                .any(|l| l.contains("/alice/test/.acl") && l.contains("rel=\"acl\"")),
            "the ACL-discovery Link rel=acl must be emitted: {links:?}"
        );
    }

    /// SECURITY (perf round-C — the cross-resource ACL-pointer-disclosure trap): the `.acl` Link is
    /// derived PER TARGET and must NEVER be interned/cached across requests. Two DIFFERENT targets
    /// must each receive THEIR OWN `.acl` Link — not a shared/stale one. If a future change were to
    /// hoist the `.acl` link into a process-wide intern (as the type/method/discovery links are), this
    /// test fails: target A's response would carry target B's ACL pointer, leaking one resource's ACL
    /// location onto another's response. Keep `add_acl_link`/`acl_url_for` per-request.
    #[test]
    fn acl_link_is_per_target_never_shared_across_resources() {
        // Resource A.
        let mut ha = HeaderMap::new();
        add_acl_link(&mut ha, &target("https://localhost:3000/alice/secret"));
        // Resource B — a DIFFERENT target.
        let mut hb = HeaderMap::new();
        add_acl_link(&mut hb, &target("https://localhost:3000/bob/notes"));

        let a = link_values(&ha);
        let b = link_values(&hb);

        // Each gets its OWN `.acl` pointer, derived from its OWN IRI.
        assert!(
            a.iter()
                .any(|l| l.contains("<https://localhost:3000/alice/secret.acl>")
                    && l.contains("rel=\"acl\"")),
            "resource A must advertise ITS OWN .acl: {a:?}"
        );
        assert!(
            b.iter()
                .any(|l| l.contains("<https://localhost:3000/bob/notes.acl>")
                    && l.contains("rel=\"acl\"")),
            "resource B must advertise ITS OWN .acl: {b:?}"
        );
        // CROSS-LEAK GUARD: A's response must NOT carry B's ACL pointer, and vice-versa.
        assert!(
            !a.iter().any(|l| l.contains("/bob/notes.acl")),
            "resource A leaked resource B's .acl pointer: {a:?}"
        );
        assert!(
            !b.iter().any(|l| l.contains("/alice/secret.acl")),
            "resource B leaked resource A's .acl pointer: {b:?}"
        );
    }

    /// The interned (request-invariant) `Link: <type>; rel="type"` values are BYTE-IDENTICAL to the
    /// pre-optimisation per-request `format!("<{iri}>; rel=\"type\"")` formatting — the optimisation
    /// only moves WHEN the bytes are produced (once per process vs per request), never WHAT they are.
    #[test]
    fn interned_type_link_values_match_reference_formatting() {
        for iri in [
            "http://www.w3.org/ns/ldp#Resource",
            "http://www.w3.org/ns/ldp#Container",
            "http://www.w3.org/ns/ldp#BasicContainer",
            "http://www.w3.org/ns/pim/space#Storage",
        ] {
            let reference = format!("<{iri}>; rel=\"type\"");
            let interned = match iri {
                "http://www.w3.org/ns/ldp#Resource" => &*HV_LINK_TYPE_LDP_RESOURCE,
                "http://www.w3.org/ns/ldp#Container" => &*HV_LINK_TYPE_LDP_CONTAINER,
                "http://www.w3.org/ns/ldp#BasicContainer" => &*HV_LINK_TYPE_LDP_BASIC_CONTAINER,
                "http://www.w3.org/ns/pim/space#Storage" => &*HV_LINK_TYPE_PIM_STORAGE,
                _ => unreachable!(),
            };
            assert_eq!(
                interned.to_str().unwrap(),
                reference,
                "interned type-link value must equal the reference formatting for {iri}"
            );
        }
    }

    /// The precomputed discovery `Link` values (from a `base_url`) are BYTE-IDENTICAL to the
    /// pre-optimisation per-request `link_headers(base_url)` + `format!` formatting.
    #[test]
    fn precomputed_discovery_link_values_match_reference_formatting() {
        let base = "https://localhost:3000";
        // Reference: the exact prior per-request formatting.
        let reference: Vec<String> = link_headers(base)
            .into_iter()
            .map(|(rel, t)| format!("<{t}>; rel=\"{rel}\""))
            .collect();
        // Optimised: the once-per-instance precompute.
        let precomputed = build_discovery_link_values(base);
        let got: Vec<String> = precomputed
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert_eq!(
            got, reference,
            "precomputed discovery Link values must equal the reference per-request formatting"
        );
    }

    /// REGRESSION (roborev round-C follow-up): `set_base_url` — the ONLY sanctioned writer of the now
    /// PRIVATE `base_url` — MUST rebuild the derived `discovery_link_values` cache so the two can never
    /// desync. Without the rebuild, a post-construction base-URL change would leave the read path
    /// advertising the OLD storage-description `Link` on every response while parsing/type links used
    /// the NEW base. Construct a state on one base, reset it, and assert BOTH the accessor AND the
    /// cached discovery `Link` values track the new base (byte-identical to a fresh precompute) and no
    /// longer reference the old one.
    #[test]
    fn set_base_url_rebuilds_discovery_link_cache() {
        let old_base = "https://old.example";
        let new_base = "https://new.example";
        let mut state = LdpState::new(
            CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new()),
            old_base,
        );
        // Precondition: the cache was built from the OLD base at construction (and is non-empty).
        let before: Vec<String> = state
            .discovery_link_values
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert!(
            !before.is_empty() && before.iter().all(|l| l.contains(old_base)),
            "cache must start derived from the old base: {before:?}"
        );

        state.set_base_url(new_base);

        // The read accessor tracks the new value.
        assert_eq!(state.base_url(), new_base);
        // The derived cache was REBUILT: it is byte-identical to a fresh precompute on the new base,
        // every line now names the NEW base, and NONE still references the old one.
        let after: Vec<String> = state
            .discovery_link_values
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        let expected: Vec<String> = build_discovery_link_values(new_base)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert_eq!(
            after, expected,
            "set_base_url must rebuild the discovery-link cache to the new base"
        );
        assert!(
            after
                .iter()
                .all(|l| l.contains(new_base) && !l.contains(old_base)),
            "no discovery Link may still reference the old base after set_base_url: {after:?}"
        );
    }

    /// `set_u64` emits the SAME decimal bytes the prior `value.to_string()` did (the `itoa` fast path
    /// is a formatting optimisation, not a representation change) — pins `Content-Length` byte-equality.
    #[test]
    fn set_u64_emits_same_decimal_as_to_string() {
        for n in [0u64, 1, 9, 10, 255, 1024, 65_535, 1_000_000, u64::MAX] {
            let mut h = HeaderMap::new();
            set_u64(&mut h, header::CONTENT_LENGTH, n);
            assert_eq!(
                h.get(header::CONTENT_LENGTH).unwrap().to_str().unwrap(),
                n.to_string(),
                "set_u64 must emit the same decimal bytes as to_string for {n}"
            );
        }
    }

    #[test]
    fn wants_container_link_is_detected() {
        let mut h = HeaderMap::new();
        assert!(!wants_container_via_link(&h));
        h.append(
            header::LINK,
            HeaderValue::from_static("<http://www.w3.org/ns/ldp#BasicContainer>; rel=\"type\""),
        );
        assert!(wants_container_via_link(&h));

        // ldp:Container also counts.
        let mut h2 = HeaderMap::new();
        h2.append(
            header::LINK,
            HeaderValue::from_static("<http://www.w3.org/ns/ldp#Container>; rel=\"type\""),
        );
        assert!(wants_container_via_link(&h2));

        // A non-type Link (e.g. an acl rel) does NOT request a container.
        let mut h3 = HeaderMap::new();
        h3.append(
            header::LINK,
            HeaderValue::from_static("<https://pod.example/x.acl>; rel=\"acl\""),
        );
        assert!(!wants_container_via_link(&h3));
    }

    #[test]
    fn require_content_type_distinguishes_absent_from_present() {
        // Absent ⇒ 400 (content-type-reject).
        let empty = HeaderMap::new();
        assert_eq!(
            require_content_type(&empty).unwrap_err().status(),
            StatusCode::BAD_REQUEST
        );
        // Whitespace-only ⇒ also 400.
        let mut blank = HeaderMap::new();
        blank.insert(header::CONTENT_TYPE, HeaderValue::from_static("   "));
        assert_eq!(
            require_content_type(&blank).unwrap_err().status(),
            StatusCode::BAD_REQUEST
        );
        // Present (even an unsupported value) ⇒ Ok (415 is decided later by `classify`).
        let mut present = HeaderMap::new();
        present.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert_eq!(require_content_type(&present).unwrap(), "text/plain");
    }

    #[test]
    fn request_origin_trims_and_filters_empty() {
        let mut present = HeaderMap::new();
        present.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://app.example"),
        );
        assert_eq!(request_origin(&present), Some("https://app.example"));
        // Whitespace is trimmed.
        let mut padded = HeaderMap::new();
        padded.insert(
            header::ORIGIN,
            HeaderValue::from_static("  https://app.example  "),
        );
        assert_eq!(request_origin(&padded), Some("https://app.example"));
        // Absent ⇒ None.
        assert_eq!(request_origin(&HeaderMap::new()), None);
        // Empty/whitespace-only ⇒ None.
        let mut blank = HeaderMap::new();
        blank.insert(header::ORIGIN, HeaderValue::from_static("   "));
        assert_eq!(request_origin(&blank), None);
    }

    // --- Finding 4: a non-NotFound read error must NOT collapse to "missing" (fail-CLOSED) --------

    use crate::store::{
        CompositeStore, DeleteOutcome, InMemoryBlobStore, InMemorySparqClient, Resource,
        ResourceMeta,
    };
    use async_trait::async_trait;
    use axum::body::Bytes as AxBytes;

    /// A [`Store`] whose `read` ALWAYS fails with a non-`NotFound` (`Storage`) error — a simulated
    /// backend/blob inconsistency. Every other method reports the resource as ABSENT, so if the
    /// handler ever (wrongly) treated the failed read as "missing" it would happily take the
    /// create/authorize path. The PATCH handler must instead PROPAGATE the `Storage` error (→ 500),
    /// never authorize.
    struct FaultyReadStore;

    #[async_trait]
    impl Store for FaultyReadStore {
        async fn read(&self, _iri: &str) -> ServerResult<Resource> {
            // NON-`NotFound`: a real storage/blob inconsistency, not an absent resource.
            Err(ServerError::Storage(
                "simulated backend inconsistency".into(),
            ))
        }
        async fn meta(&self, _iri: &str) -> ServerResult<Option<ResourceMeta>> {
            // CONSISTENT with `read`: a real store's `meta` and `read` share ONE authoritative
            // (`get_meta`) source, so they can NOT disagree on presence/error. Since `read` faults with
            // a non-`NotFound` `Storage` error, `meta` faults the SAME way — so the ACL-cache's cheap
            // `meta` probe propagates the inconsistency (fail-closed), NEVER treats it as "absent ACL".
            // (Returning `Ok(None)` here would model an impossible store and let the resolver fail OPEN.)
            Err(ServerError::Storage(
                "simulated backend inconsistency".into(),
            ))
        }
        async fn exists(&self, _iri: &str) -> ServerResult<bool> {
            Ok(false)
        }
        async fn write(
            &self,
            _iri: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("write must not be reached: the read error must propagate before any write");
        }
        async fn create_in_container(
            &self,
            _container: &str,
            _child: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("create_in_container must not be reached on a faulted read");
        }
        async fn delete(&self, _iri: &str, _parent: Option<&str>) -> ServerResult<()> {
            Ok(())
        }
        async fn delete_container_if_empty(
            &self,
            _iri: &str,
            _parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            Ok(DeleteOutcome::NotFound)
        }
        async fn list_children(
            &self,
            _container: &str,
        ) -> ServerResult<Vec<crate::store::ValidatedChildIri>> {
            Ok(Vec::new())
        }
    }

    use crate::error::ServerResult;

    #[tokio::test]
    async fn patch_propagates_non_notfound_read_error_does_not_treat_as_missing() {
        // An INSERT-ONLY PATCH whose EVERY read (target AND `.acl`) fails with a STORAGE error (not
        // NotFound). The handler must NEVER collapse the failed read into "missing" and take the
        // create-on-PATCH path (the pre-fix `read().ok()` fail-OPEN bug): the faulty store PANICS if
        // `write`/`create_in_container` is reached. With the fix, authorization runs first and its own
        // `.acl` read faults (a non-NotFound ACL read propagates — fail-closed), so the storage error
        // surfaces as a 500; either way the create path is never taken. (The narrower
        // unauthorized-caller-must-not-get-500 property is pinned by
        // `patch_unauthorized_caller_with_faulting_target_read_gets_uniform_denial_not_500`, where only
        // the TARGET read faults so authorization can reach a real decision.)
        let state = Arc::new(LdpState::new(FaultyReadStore, "https://pod.example"));
        let token = VerifiedToken {
            web_id: Some("https://pod.example/alice/profile/card#me".into()),
            ..VerifiedToken::default()
        };
        let uri: axum::http::Uri = "/alice/data".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/n3"));
        let patch_body = AxBytes::from(
            "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
             @prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
             _:p solid:inserts { <https://pod.example/alice/data#me> foaf:name \"X\" . }.\n",
        );
        let err = patch_handler(State(state), Extension(token), uri, headers, patch_body)
            .await
            .expect_err("a non-NotFound read error must surface, not be treated as missing");
        // It must surface as the storage error (500), NOT a create-path 201 / a 403 / a 404.
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // --- Finding 2 (round-2): a PRE-AUTH storage error must not leak via 500 to an unauthorized caller.

    const OWNER: &str = "https://pod.example/alice/profile/card#me";
    const STRANGER: &str = "https://pod.example/bob/profile/card#me";

    /// A [`Store`] that faults ONLY on the TARGET resource read (a simulated backend/blob
    /// inconsistency on the resource itself) while serving a real, owner-only `.acl` so authorization
    /// can reach a genuine allow/deny decision. This isolates the round-2 property: an UNAUTHORIZED
    /// caller must get the uniform 401/403 (not a 500 distinguishing "faulting backend" from "missing /
    /// normally-stored"), and an AUTHORIZED caller must get the 500 surfaced AFTER authorization.
    ///
    /// `read`:
    ///  - the target IRI → a non-`NotFound` `Storage` error (the inconsistency);
    ///  - the target's `.acl` → an owner-only ACL granting [`OWNER`] Read/Write/Control (so authz runs);
    ///  - anything else (e.g. an ancestor `.acl`) → `NotFound` (no other ACL up the tree).
    struct TargetFaultyAclStore {
        target: String,
    }

    impl TargetFaultyAclStore {
        fn new(target: &str) -> Self {
            Self {
                target: target.to_string(),
            }
        }
        fn acl_body(&self) -> String {
            format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{OWNER}>;
         acl:accessTo <{}>;
         acl:mode acl:Read, acl:Write, acl:Control."#,
                self.target
            )
        }
    }

    #[async_trait]
    impl Store for TargetFaultyAclStore {
        async fn read(&self, iri: &str) -> ServerResult<Resource> {
            if iri == self.target {
                // The inconsistency on the resource itself — a NON-NotFound error.
                return Err(ServerError::Storage(
                    "simulated backend inconsistency".into(),
                ));
            }
            if iri == format!("{}.acl", self.target) {
                // The target's OWN `.acl`: an owner-only authorization, served normally so authz works.
                let body = AxBytes::from(self.acl_body());
                let meta = ResourceMeta {
                    content_type: "text/turtle".into(),
                    blob_key: "k".into(),
                    etag: "\"acl\"".into(),
                    last_modified: None,
                };
                return Ok(Resource { body, meta });
            }
            // No other ACL anywhere up the tree.
            Err(ServerError::NotFound)
        }
        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            // CONSISTENT with `read` (a real store's `meta`/`read` share one `get_meta` source):
            //  - the target IRI → the SAME non-`NotFound` `Storage` fault `read` raises (the
            //    inconsistency surfaces through the ACL-cache's cheap `meta` probe too, never as absent);
            //  - the target's `.acl` → `Some` with the SAME etag `read` serves, so the cache MISSES then
            //    `read`s + parses it (authz sees the owner-only ACL);
            //  - anything else → `None` (absent), matching `read`'s `NotFound`.
            if iri == self.target {
                return Err(ServerError::Storage(
                    "simulated backend inconsistency".into(),
                ));
            }
            if iri == format!("{}.acl", self.target) {
                return Ok(Some(ResourceMeta {
                    content_type: "text/turtle".into(),
                    blob_key: "k".into(),
                    etag: "\"acl\"".into(),
                    last_modified: None,
                }));
            }
            Ok(None)
        }
        async fn exists(&self, _iri: &str) -> ServerResult<bool> {
            Ok(false)
        }
        async fn write(
            &self,
            _iri: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("write must not be reached: the faulted target read must surface as 500 first");
        }
        async fn create_in_container(
            &self,
            _container: &str,
            _child: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("create_in_container must not be reached on a faulted target read");
        }
        async fn delete(&self, _iri: &str, _parent: Option<&str>) -> ServerResult<()> {
            Ok(())
        }
        async fn delete_container_if_empty(
            &self,
            _iri: &str,
            _parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            Ok(DeleteOutcome::NotFound)
        }
        async fn list_children(
            &self,
            _container: &str,
        ) -> ServerResult<Vec<crate::store::ValidatedChildIri>> {
            Ok(Vec::new())
        }
    }

    /// An INSERT-ONLY `text/n3` PATCH body targeting `subject`.
    fn insert_only_patch(subject: &str) -> AxBytes {
        AxBytes::from(format!(
            "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
             @prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
             _:p solid:inserts {{ <{subject}> foaf:name \"X\" . }}.\n",
        ))
    }

    fn n3_patch_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/n3"));
        headers
    }

    #[tokio::test]
    async fn patch_unauthorized_caller_with_faulting_target_read_gets_uniform_denial_not_500() {
        // (a) An UNAUTHORIZED caller (a stranger, not the ACL's owner) PATCHing a resource whose target
        // read faults must get the uniform authorization denial (403 authenticated), NOT a 500 — the
        // backend inconsistency must never be observable to a caller who is not permitted the operation
        // (an existence/state oracle). The store PANICS if any write is reached.
        let target = "https://pod.example/alice/data";
        let state = Arc::new(LdpState::new(
            TargetFaultyAclStore::new(target),
            "https://pod.example",
        ));
        let token = VerifiedToken {
            web_id: Some(STRANGER.into()),
            ..VerifiedToken::default()
        };
        let uri: axum::http::Uri = "/alice/data".parse().unwrap();
        let err = patch_handler(
            State(state),
            Extension(token),
            uri,
            n3_patch_headers(),
            insert_only_patch(&format!("{target}#me")),
        )
        .await
        .expect_err("an unauthorized caller must be denied, never see the 500");
        // 403 (authenticated-but-unauthorized) — the uniform denial, NOT the 500 the pre-fix leaked.
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn patch_anonymous_caller_with_faulting_target_read_gets_401_not_500() {
        // Same as above but ANONYMOUS: the uniform denial is 401 (so the client authenticates), never a
        // 500. An unauthenticated caller must not learn the backend is inconsistent either.
        let target = "https://pod.example/alice/data";
        let state = Arc::new(LdpState::new(
            TargetFaultyAclStore::new(target),
            "https://pod.example",
        ));
        let token = VerifiedToken::default(); // anonymous (web_id == None)
        let uri: axum::http::Uri = "/alice/data".parse().unwrap();
        let err = patch_handler(
            State(state),
            Extension(token),
            uri,
            n3_patch_headers(),
            insert_only_patch(&format!("{target}#me")),
        )
        .await
        .expect_err("an anonymous caller must be denied, never see the 500");
        assert_eq!(err.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn patch_authorized_caller_with_faulting_target_read_gets_500_surfaced_post_auth() {
        // (b) An AUTHORIZED caller (the ACL owner) PATCHing the same resource MUST get the 500 — the
        // backend error IS surfaced, but only after authorization succeeds (so it is not an oracle).
        // The store PANICS if a write is reached, proving the error surfaced BEFORE the create path.
        let target = "https://pod.example/alice/data";
        let state = Arc::new(LdpState::new(
            TargetFaultyAclStore::new(target),
            "https://pod.example",
        ));
        let token = VerifiedToken {
            web_id: Some(OWNER.into()),
            ..VerifiedToken::default()
        };
        let uri: axum::http::Uri = "/alice/data".parse().unwrap();
        let err = patch_handler(
            State(state),
            Extension(token),
            uri,
            n3_patch_headers(),
            insert_only_patch(&format!("{target}#me")),
        )
        .await
        .expect_err("an authorized caller must see the backend error surfaced post-auth");
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn patch_authorized_caller_with_notfound_target_takes_normal_create_path() {
        // (c) An AUTHORIZED caller PATCHing a GENUINELY-MISSING target (a real `NotFound`, not a fault)
        // must take the normal create-on-PATCH path → 201 Created, proving the round-2 change did not
        // regress the legitimate create path. Uses the real composite store with a seeded owner ACL.
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        // Seed a root `.acl` granting the owner Read/Write/Control on the root + all descendants.
        let root_acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{OWNER}>;
         acl:accessTo <https://pod.example/>;
         acl:default <https://pod.example/>;
         acl:mode acl:Read, acl:Write, acl:Control."#
        );
        store
            .write(
                "https://pod.example/.acl",
                AxBytes::from(root_acl),
                "text/turtle",
            )
            .await
            .expect("seed root acl");
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let token = VerifiedToken {
            web_id: Some(OWNER.into()),
            ..VerifiedToken::default()
        };
        let uri: axum::http::Uri = "/alice/note".parse().unwrap();
        let resp = patch_handler(
            State(state),
            Extension(token),
            uri,
            n3_patch_headers(),
            insert_only_patch("https://pod.example/alice/note#me"),
        )
        .await
        .expect("a create-on-PATCH of a genuinely-missing target must succeed");
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // --- Finding 1: an `.acl` (auxiliary) create must NOT emit a parent-containment Add ------------

    /// Seed a root `.acl` granting `OWNER` full control over the root + all descendants, written
    /// through the store as an auxiliary resource. Returns the store ready for handler use.
    async fn store_with_owner_root_acl() -> CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let root_acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{OWNER}>;
         acl:accessTo <https://pod.example/>;
         acl:default <https://pod.example/>;
         acl:mode acl:Read, acl:Write, acl:Control."#
        );
        store
            .write(
                "https://pod.example/.acl",
                AxBytes::from(root_acl),
                "text/turtle",
            )
            .await
            .expect("seed root acl");
        store
    }

    fn owner_token() -> VerifiedToken {
        VerifiedToken {
            web_id: Some(OWNER.into()),
            ..VerifiedToken::default()
        }
    }

    #[tokio::test]
    async fn put_create_of_acl_emits_no_parent_containment_add() {
        // A PUT that CREATES an auxiliary `.acl` resource must NOT cause a container-membership `Add`
        // notification on the parent — an `.acl` is NOT a contained child (no `ldp:contains` edge). A
        // subscriber to the parent container must therefore receive NOTHING for the `.acl` create.
        let hub = NotificationHub::new();
        let store = store_with_owner_root_acl().await;
        let state = Arc::new(LdpState::with_hub(
            store,
            "https://pod.example",
            hub.clone(),
        ));

        let parent = "https://pod.example/alice/";
        let mut parent_rx = hub.subscribe(parent).await;

        // PUT the `.acl` for a resource in /alice/ — auth for `.acl` is Control (the owner has it).
        let uri: axum::http::Uri = "/alice/doc.acl".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/turtle"),
        );
        let acl_body = AxBytes::from(format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#o> a acl:Authorization; acl:agent <{OWNER}>; acl:accessTo <https://pod.example/alice/doc>; acl:mode acl:Read, acl:Write, acl:Control."#
        ));
        let resp = put_handler(
            State(state),
            Extension(owner_token()),
            uri,
            headers,
            acl_body,
        )
        .await
        .expect("an owner PUT of an .acl must succeed");
        assert_eq!(resp.status(), StatusCode::CREATED);

        // The parent container subscriber must have received NOTHING — no spurious membership Add.
        assert!(
            parent_rx.try_recv().is_err(),
            "an .acl create must not emit a parent-containment Add notification"
        );
    }

    #[tokio::test]
    async fn put_create_of_normal_resource_does_emit_parent_containment_add() {
        // The control: a PUT that creates a NORMAL (non-`.acl`) resource DOES grow its parent's
        // membership, so the parent subscriber MUST receive a membership `Add`. This guards against the
        // finding-1 fix over-suppressing the legitimate notification.
        let hub = NotificationHub::new();
        let store = store_with_owner_root_acl().await;
        let state = Arc::new(LdpState::with_hub(
            store,
            "https://pod.example",
            hub.clone(),
        ));

        let parent = "https://pod.example/alice/";
        let mut parent_rx = hub.subscribe(parent).await;

        let uri: axum::http::Uri = "/alice/doc".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/turtle"),
        );
        let body = AxBytes::from(
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        );
        let resp = put_handler(State(state), Extension(owner_token()), uri, headers, body)
            .await
            .expect("an owner PUT of a normal resource must succeed");
        assert_eq!(resp.status(), StatusCode::CREATED);

        // The parent container subscriber MUST see a membership Add naming the new child.
        let frame = parent_rx
            .try_recv()
            .expect("a normal resource create must emit a parent-containment Add");
        assert!(frame.contains("\"type\":\"Add\""), "{frame}");
        assert!(
            frame.contains("\"object\":\"https://pod.example/alice/doc\""),
            "{frame}"
        );
    }

    #[tokio::test]
    async fn patch_create_of_acl_emits_no_parent_containment_add() {
        // The PATCH-create path mirrors PUT-create: a create-on-PATCH of an auxiliary `.acl` must NOT
        // emit a parent-containment Add either.
        let hub = NotificationHub::new();
        let store = store_with_owner_root_acl().await;
        let state = Arc::new(LdpState::with_hub(
            store,
            "https://pod.example",
            hub.clone(),
        ));

        let parent = "https://pod.example/alice/";
        let mut parent_rx = hub.subscribe(parent).await;

        // An INSERT-ONLY PATCH that CREATES the `.acl` (target absent → create-on-PATCH). Auth is
        // Control (the owner has it). The inserted triple is a minimal authorization.
        let uri: axum::http::Uri = "/alice/doc2.acl".parse().unwrap();
        let patch_body = AxBytes::from(format!(
            "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
             @prefix acl: <http://www.w3.org/ns/auth/acl#> .\n\
             _:p solid:inserts {{ <#o> a acl:Authorization; acl:agent <{OWNER}>; \
             acl:accessTo <https://pod.example/alice/doc2>; acl:mode acl:Read . }}.\n",
        ));
        let resp = patch_handler(
            State(state),
            Extension(owner_token()),
            uri,
            n3_patch_headers(),
            patch_body,
        )
        .await
        .expect("an owner create-on-PATCH of an .acl must succeed");
        assert_eq!(resp.status(), StatusCode::CREATED);

        assert!(
            parent_rx.try_recv().is_err(),
            "an .acl create-on-PATCH must not emit a parent-containment Add notification"
        );
    }

    // --- HIGH: POST-Slug auxiliary-resource privilege-escalation bypass ----------------------------
    //
    // The exploit (execution-proved by adversarial verification): a POST to a container authorizes
    // only `acl:Append`, but `sanitise_slug` keeps `.`, so `Slug: secret.acl` survives and mints
    // `…/secret.acl`. With NO `.acl`/Control re-check, the create wrote an attacker-controlled
    // `…/secret.acl` that the WAC resolver then reads as the OWN ACL of `…/secret`, overriding
    // inheritance — letting an Append-only agent grant itself Control over a sibling private resource.

    const ALICE: &str = OWNER; // the container owner (private resource is hers)
    const BOB: &str = STRANGER; // the Append-only attacker

    /// Build a store where `/alice/c/` exists, Alice owns it (default Read/Write/Control over the
    /// container + its members), and Bob holds ONLY `acl:Append` on the container itself. The child
    /// `/alice/c/secret` is therefore Alice-private by inheritance (no own ACL).
    async fn store_alice_container_bob_append_only(
    ) -> CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        // The container must EXIST for a POST to it to proceed (the handler's existence check).
        store
            .write(
                "https://pod.example/alice/c/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed container");
        // The container `.acl`: Alice gets default Read/Write/Control (so `secret` inherits
        // Alice-private); Bob gets ONLY Append on the container itself (he can POST a member, but
        // cannot read/control the container or its members).
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization;
         acl:agent <{ALICE}>;
         acl:accessTo <https://pod.example/alice/c/>;
         acl:default <https://pod.example/alice/c/>;
         acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization;
       acl:agent <{BOB}>;
       acl:accessTo <https://pod.example/alice/c/>;
       acl:mode acl:Append."#
        );
        store
            .write(
                "https://pod.example/alice/c/.acl",
                AxBytes::from(acl),
                "text/turtle",
            )
            .await
            .expect("seed container acl");
        store
    }

    fn bob_token() -> VerifiedToken {
        VerifiedToken {
            web_id: Some(BOB.into()),
            ..VerifiedToken::default()
        }
    }

    /// A POST body that, if it landed as `…/secret.acl`, would grant Bob `acl:Control` over
    /// `…/secret` — the escalation payload.
    fn bob_self_control_acl_body() -> AxBytes {
        AxBytes::from(format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#pwn> a acl:Authorization;
       acl:agent <{BOB}>;
       acl:accessTo <https://pod.example/alice/c/secret>;
       acl:mode acl:Read, acl:Write, acl:Control."#
        ))
    }

    fn post_turtle_headers_with_slug(slug: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/turtle"),
        );
        headers.insert(
            HeaderName::from_static("slug"),
            HeaderValue::from_str(slug).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn post_slug_dot_acl_is_denied_and_grants_attacker_nothing() {
        // THE EXPLOIT, ported as a regression test driving the REAL post_handler + get_handler.
        let store = store_alice_container_bob_append_only().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));

        // (1) Bob (Append-only) POSTs `Slug: secret.acl` with a self-Control body → MUST be denied
        //     (403). The auxiliary-mint guard refuses to let an Append-only POST create a `.acl`.
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        let err = post_handler(
            State(state.clone()),
            Extension(bob_token()),
            uri,
            post_turtle_headers_with_slug("secret.acl"),
            bob_self_control_acl_body(),
        )
        .await
        .expect_err("POST Slug: secret.acl by an Append-only caller MUST be denied");
        assert_eq!(
            err.status(),
            StatusCode::FORBIDDEN,
            "the auxiliary-mint escalation must be a 403"
        );

        // (1b) The malicious `.acl` must NOT exist — the create never happened.
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/secret.acl")
                .await
                .unwrap(),
            "no attacker-controlled .acl may have been written"
        );

        // (2) Bob then tries to GET the sibling `…/secret` — he gained NOTHING. `secret` is
        //     Alice-private by inheritance and has no (attacker-planted) own ACL, so Bob is denied.
        let get_uri: axum::http::Uri = "/alice/c/secret".parse().unwrap();
        let get_err = get_handler(
            State(state),
            Extension(bob_token()),
            get_uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("Bob must not be able to read Alice's private resource");
        // 403 — Bob is authenticated but unauthorized (he inherits no Read from the Alice-only default).
        assert_eq!(
            get_err.status(),
            StatusCode::FORBIDDEN,
            "Bob must gain no read access to the sibling private resource"
        );
    }

    #[tokio::test]
    async fn post_slug_dot_acl_case_variant_is_also_denied() {
        // Defence-in-depth: a case-variant Slug (`secret.ACL`) must ALSO be rejected at the mint
        // chokepoint — `sanitise_slug` keeps it verbatim, so without a case-insensitive guard it would
        // sail through (and a case-insensitive filesystem/resolver later could make it load-bearing).
        let store = store_alice_container_bob_append_only().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        let err = post_handler(
            State(state.clone()),
            Extension(bob_token()),
            uri,
            post_turtle_headers_with_slug("secret.ACL"),
            bob_self_control_acl_body(),
        )
        .await
        .expect_err("a case-variant .acl slug must also be denied");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/secret.ACL")
                .await
                .unwrap(),
            "no case-variant auxiliary resource may have been written"
        );
    }

    #[tokio::test]
    async fn post_slug_dot_meta_is_allowed_meta_is_not_load_bearing() {
        // `.meta` is NOT load-bearing in this server: the WAC resolver never consults a `.meta`, and
        // the create paths only special-case `.acl`. So `secret.meta` is just a normal resource name
        // with no security effect — guarding it ONLY at POST (while a PUT/PATCH could create it freely)
        // was an inconsistency with no benefit. An Append-only POST of `Slug: secret.meta` is therefore
        // ALLOWED, exactly like any other benign append. (If `.meta` ever becomes load-bearing it must
        // be guarded UNIFORMLY across POST/PUT/PATCH/DELETE/read — see `is_acl_auxiliary_suffix`.)
        let store = store_alice_container_bob_append_only().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        let resp = post_handler(
            State(state.clone()),
            Extension(bob_token()),
            uri,
            post_turtle_headers_with_slug("secret.meta"),
            AxBytes::from("<https://pod.example/alice/c/secret> <http://p> <http://o> ."),
        )
        .await
        .expect("a .meta slug is a normal resource name and must be allowed");
        assert_eq!(resp.status(), StatusCode::CREATED);
        // V2 (`research/lws-design-records.md` §6): the minted `Location` is collision-INDEPENDENT
        // — it CONTAINS the Slug stem (`secret.meta`) but is opaque-suffixed (never the verbatim
        // segment), so it carries no existence signal. The created resource exists at exactly that
        // minted Location.
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .expect("Location header")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            loc.starts_with("https://pod.example/alice/c/secret.meta-"),
            "the Location must contain the Slug stem as an opaque-suffixed prefix: {loc}"
        );
        assert!(state.store.exists(&loc).await.unwrap());
        // And it grants Bob NOTHING over the sibling `…/secret` — a `.meta` is not consulted by WAC,
        // so `secret` stays Alice-private by inheritance.
        let get_uri: axum::http::Uri = "/alice/c/secret".parse().unwrap();
        let get_err = get_handler(
            State(state),
            Extension(bob_token()),
            get_uri,
            HeaderMap::new(),
        )
        .await
        .expect_err("Bob must not be able to read Alice's private resource");
        assert_eq!(get_err.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn post_benign_slug_still_works_no_regression() {
        // The control: an Append-only Bob POSTing a BENIGN slug into the container still succeeds —
        // the fix must not break legitimate container appends.
        let store = store_alice_container_bob_append_only().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        let resp = post_handler(
            State(state.clone()),
            Extension(bob_token()),
            uri,
            post_turtle_headers_with_slug("note"),
            AxBytes::from(
                "<https://pod.example/alice/c/note#me> <http://xmlns.com/foaf/0.1/name> \"N\" .",
            ),
        )
        .await
        .expect("a benign Append POST must still succeed");
        assert_eq!(resp.status(), StatusCode::CREATED);
        // V2 (`research/lws-design-records.md` §6): the child's `Location` CONTAINS the Slug
        // (`note`) as an opaque-suffixed prefix — collision-INDEPENDENT, so it leaks nothing about
        // which names already exist — and the resource exists at exactly that Location. (The CTH
        // `post-uri-assignment-slug` row asserts only `Location contains '<slug>'`, which this
        // satisfies.)
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .expect("Location header")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            loc.starts_with("https://pod.example/alice/c/note-"),
            "the Location must contain the Slug as an opaque-suffixed prefix: {loc}"
        );
        assert!(
            loc.contains("note"),
            "Location must contain the Slug: {loc}"
        );
        assert!(state.store.exists(&loc).await.unwrap());
    }

    #[tokio::test]
    async fn post_slug_dot_acl_denied_even_for_a_controller() {
        // A POST is an Append/Write operation on the CONTAINER, never a Control op — so even a caller
        // who DOES hold Control over the container (Alice) must not be able to mint a `.acl` via the
        // POST-Slug path. The legitimate way to author an `.acl` is a Control-gated PUT/PATCH of the
        // exact `.acl` IRI; the POST chokepoint uniformly refuses to mint an auxiliary child. Consistent
        // behaviour: reject for everyone (no privilege-dependent fork at the mint point — that keeps the
        // chokepoint simple and impossible to confuse). Alice can still PUT `/alice/c/secret.acl`
        // directly, which IS Control-gated and which she passes.
        let store = store_alice_container_bob_append_only().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));

        // Alice (controller) POSTs Slug: secret.acl → still 403 at the mint chokepoint.
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        let err = post_handler(
            State(state.clone()),
            Extension(owner_token()),
            uri,
            post_turtle_headers_with_slug("secret.acl"),
            AxBytes::from(format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#a> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/secret>; acl:mode acl:Control."#
            )),
        )
        .await
        .expect_err("POST-Slug minting an .acl is refused for everyone, controllers included");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);

        // But Alice CAN author it the legitimate, Control-gated way: a direct PUT of the .acl IRI.
        let put_uri: axum::http::Uri = "/alice/c/secret.acl".parse().unwrap();
        let mut put_headers = HeaderMap::new();
        put_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/turtle"),
        );
        let resp = put_handler(
            State(state),
            Extension(owner_token()),
            put_uri,
            put_headers,
            AxBytes::from(format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#a> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/secret>; acl:mode acl:Read, acl:Write, acl:Control."#
            )),
        )
        .await
        .expect("a controller may PUT an .acl directly (Control-gated)");
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // --- container listing render (Optimization #1: O(N) de-dup, byte-identical output) -----------

    /// Count occurrences of `needle` in `hay` (a tiny substring counter for the listing-body asserts).
    fn count_occurrences(hay: &str, needle: &str) -> usize {
        if needle.is_empty() {
            return 0;
        }
        let mut n = 0;
        let mut from = 0;
        while let Some(i) = hay[from..].find(needle) {
            n += 1;
            from += i + needle.len();
        }
        n
    }

    #[tokio::test]
    async fn render_container_lists_every_child_once_with_typing() {
        // A multi-child container renders the three ldp typing triples + EXACTLY ONE `ldp:contains`
        // per member, with no duplicates — the contract the O(N) de-dup must preserve.
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let container = "https://pod.example/c/";
        // Mint the container, then add several distinct children through the authoritative path.
        store
            .write(container, AxBytes::new(), "text/turtle")
            .await
            .expect("mint container");
        let children = [
            "https://pod.example/c/a",
            "https://pod.example/c/b",
            "https://pod.example/c/c",
            "https://pod.example/c/d",
        ];
        for child in children {
            store
                .create_in_container(container, child, AxBytes::new(), "text/turtle")
                .await
                .expect("add child");
        }
        let state = Arc::new(LdpState::new(store, "https://pod.example"));

        let (body, ct) = render_container(
            &state,
            container,
            &AxBytes::new(),
            "text/turtle",
            Some("text/turtle"),
        )
        .await
        .expect("render");
        assert_eq!(ct, "text/turtle");
        let text = String::from_utf8(body.to_vec()).unwrap();

        // The three ldp typing triples are present.
        assert!(text.contains("ldp#Resource"), "body: {text}");
        assert!(text.contains("ldp#Container"), "body: {text}");
        assert!(text.contains("ldp#BasicContainer"), "body: {text}");
        // The containment predicate is rendered (the Turtle serialiser abbreviates the four objects
        // onto ONE `ldp:contains` predicate via `,`-lists, so the predicate string itself appears
        // once — the per-child count below is the real "exactly one containment edge per child" check).
        assert!(text.contains("ldp#contains"), "body: {text}");
        // Each child IRI appears EXACTLY ONCE — no duplicate containment edge, none missing. (Each
        // child IRI is distinct and is not a substring of the container subject or another child.)
        for child in children {
            assert_eq!(
                count_occurrences(&text, child),
                1,
                "child {child} must appear exactly once: {text}"
            );
        }
    }

    #[tokio::test]
    async fn render_container_dedups_generated_against_stored_body_byte_identical() {
        // A stored container body that ALREADY asserts a generated triple (the ldp:BasicContainer
        // typing) must NOT have it repeated by the generated set — the de-dup catches the overlap.
        // This is the one place the HashSet de-dup actually suppresses anything; it must match the
        // old `push_unique` behaviour exactly (the overlapping triple appears ONCE).
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let container = "https://pod.example/c/";
        let stored_body = AxBytes::from(
            "<https://pod.example/c/> a <http://www.w3.org/ns/ldp#BasicContainer> .".to_string(),
        );
        store
            .write(container, stored_body.clone(), "text/turtle")
            .await
            .expect("mint container with stored body");
        let state = Arc::new(LdpState::new(store, "https://pod.example"));

        let (body, _ct) = render_container(
            &state,
            container,
            &stored_body,
            "text/turtle",
            Some("text/turtle"),
        )
        .await
        .expect("render");
        let text = String::from_utf8(body.to_vec()).unwrap();
        // The BasicContainer typing appears exactly once despite being in BOTH the stored body and the
        // generated set (the overlap is de-duped — matching the prior render).
        assert_eq!(
            count_occurrences(&text, "ldp#BasicContainer"),
            1,
            "the stored+generated BasicContainer triple must appear once: {text}"
        );
    }

    #[tokio::test]
    async fn render_container_dedups_large_stored_body_via_hashset_branch() {
        // A LARGE stored body (> DEDUP_HASHSET_THRESHOLD triples) that ALSO asserts a generated triple
        // (the ldp:BasicContainer typing) must dedup the overlap via the HASHSET branch — the same
        // contract as the small/linear branch (overlap appears once). This guards the threshold-gated
        // path roborev flagged: the large-stored-body case must not double the generated triple, and
        // must not introduce or drop any membership edge.
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        let container = "https://pod.example/c/";
        // 30 distinct stored triples (> the 16 threshold → HashSet branch) — one of which is exactly a
        // GENERATED triple (the BasicContainer typing), the rest unrelated `ex:p_i ex:o_i` assertions.
        let mut body = String::from(
            "<https://pod.example/c/> a <http://www.w3.org/ns/ldp#BasicContainer> .\n",
        );
        for i in 0..29 {
            body.push_str(&format!(
                "<https://pod.example/c/> <https://ex.example/p{i}> <https://ex.example/o{i}> .\n"
            ));
        }
        let stored_body = AxBytes::from(body);
        store
            .write(container, stored_body.clone(), "text/turtle")
            .await
            .expect("mint container with large stored body");
        let children = [
            "https://pod.example/c/x",
            "https://pod.example/c/y",
            "https://pod.example/c/z",
        ];
        for child in children {
            store
                .create_in_container(container, child, AxBytes::new(), "text/turtle")
                .await
                .expect("add child");
        }
        let state = Arc::new(LdpState::new(store, "https://pod.example"));

        let (body_out, _ct) = render_container(
            &state,
            container,
            &stored_body,
            "text/turtle",
            Some("text/turtle"),
        )
        .await
        .expect("render");
        let text = String::from_utf8(body_out.to_vec()).unwrap();
        // The overlapping BasicContainer typing is de-duped to ONE occurrence (HashSet branch).
        assert_eq!(
            count_occurrences(&text, "ldp#BasicContainer"),
            1,
            "large-stored overlap must dedup to once: {text}"
        );
        // Every child still renders exactly once (no membership edge dropped or doubled).
        for child in children {
            assert_eq!(
                count_occurrences(&text, child),
                1,
                "child {child} must appear exactly once on the HashSet branch: {text}"
            );
        }
        // The unrelated stored triples are all carried through verbatim. Match the FULL IRI term
        // (trailing `>`) so e.g. `o1` does not substring-match `o10`..`o19`.
        for i in 0..29 {
            assert_eq!(
                count_occurrences(&text, &format!("https://ex.example/o{i}>")),
                1,
                "stored triple o{i} must be carried through once: {text}"
            );
        }
    }

    // =====================================================================================
    // EXISTENCE-NON-DISCLOSURE (`research/lws-design-records.md` §6; RSS `decisions/0003`) — the
    // exhaustive byte-identical matrix.
    //
    // THE RULE: 404 is served ONLY to a requester who holds the operation's required mode. Every other
    // requester (anonymous → 401, authenticated-but-unauthorized → 403) gets their DENIAL code for BOTH
    // "forbidden-existing" and "not-found", BYTE-IDENTICALLY (same status + body + Location + ETag +
    // WWW-Authenticate). These tests drive the REAL handlers over the drop-box adversary fixture
    // (`store_alice_container_bob_append_only`: Alice owns `/alice/c/` with inheritable R/W/C; Bob holds
    // ONLY `acl:Append` on the container — the canonical "create-rights-on-parent, no-rights-on-target"
    // shape) and assert the full materialised response is identical across the missing-vs-forbidden axis.
    // =====================================================================================

    /// A fully-materialised HTTP response, reduced to the client-observable fields the rule constrains:
    /// the status, the body bytes, and the security-relevant headers (`Location`, `ETag`,
    /// `WWW-Authenticate`). Two responses are an EXISTENCE ORACLE iff they differ in ANY of these.
    #[derive(Debug, PartialEq, Eq)]
    struct ObservableResponse {
        status: u16,
        body: Vec<u8>,
        location: Option<String>,
        etag: Option<String>,
        www_authenticate: Option<String>,
    }

    /// Materialise a handler result (`Ok(Response)` or `Err(ServerError)`) into the
    /// client-observable response — exactly what the HTTP client sees, via `IntoResponse` (so a denial
    /// `ServerError` is rendered through the SAME path the server uses, carrying its real body +
    /// `WWW-Authenticate`).
    async fn observe(result: Result<Response, ServerError>) -> ObservableResponse {
        let resp = match result {
            Ok(r) => r,
            Err(e) => e.into_response(),
        };
        let status = resp.status().as_u16();
        let header = |resp: &Response, name: HeaderName| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let location = header(&resp, header::LOCATION);
        let etag = header(&resp, header::ETAG);
        let www_authenticate = header(&resp, header::WWW_AUTHENTICATE);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        ObservableResponse {
            status,
            body,
            location,
            etag,
            www_authenticate,
        }
    }

    /// Bob (authenticated, Append-only on `/alice/c/`, NO rights on its members) — the adversary.
    fn bob() -> VerifiedToken {
        VerifiedToken {
            web_id: Some(BOB.into()),
            ..VerifiedToken::default()
        }
    }

    /// An anonymous (no-WebID) requester.
    fn anon() -> VerifiedToken {
        VerifiedToken::default()
    }

    /// The drop-box fixture with an EXISTING Alice-private child `/alice/c/secret` (forbidden to Bob by
    /// inheritance) so we can exercise the "exists-but-forbidden" axis against the "missing" one.
    async fn dropbox_with_secret(
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = store_alice_container_bob_append_only().await;
        // Seed an EXISTING member `/alice/c/secret` (Alice-private by inheritance — no own ACL).
        store
            .write(
                "https://pod.example/alice/c/secret",
                AxBytes::from(
                    "<https://pod.example/alice/c/secret#me> <http://xmlns.com/foaf/0.1/name> \"S\" ."
                        .to_string(),
                ),
                "text/turtle",
            )
            .await
            .expect("seed secret member");
        Arc::new(LdpState::new(store, "https://pod.example"))
    }

    const EXISTING: &str = "/alice/c/secret"; // exists, Alice-private (forbidden to Bob/anon)
    const MISSING: &str = "/alice/c/ghost"; // never created

    fn turtle_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/turtle"),
        );
        h
    }

    // --- Conditional GET → 304 Not Modified (RFC 9110 §13; bead vltw) ------------------------------

    /// Turtle Content-Type headers for a write.
    fn turtle_write_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/turtle"),
        );
        h
    }

    fn request_body_bytes() -> AxBytes {
        AxBytes::from("<https://pod.example/alice/c/x#me> <http://p> <http://o> .".to_string())
    }

    /// Run one verb against one path with one token, returning the observable response. Centralises the
    /// per-verb handler dispatch so the matrix below is a tight loop.
    async fn run_verb(
        state: &Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>>,
        verb: &str,
        path: &str,
        token: VerifiedToken,
    ) -> ObservableResponse {
        let uri: axum::http::Uri = path.parse().unwrap();
        let s = State(state.clone());
        let t = Extension(token);
        let result = match verb {
            "GET" => get_handler(s, t, uri, HeaderMap::new()).await,
            "HEAD" => head_handler(s, t, uri, HeaderMap::new()).await,
            "PUT" => put_handler(s, t, uri, turtle_headers(), request_body_bytes()).await,
            "POST" => post_handler(s, t, uri, turtle_headers(), request_body_bytes()).await,
            "PATCH" => {
                patch_handler(
                    s,
                    t,
                    uri,
                    n3_patch_headers(),
                    insert_only_patch("https://pod.example/alice/c/x#me"),
                )
                .await
            }
            "DELETE" => delete_handler(s, t, uri, HeaderMap::new()).await,
            other => panic!("unknown verb {other}"),
        };
        observe(result).await
    }

    /// THE MATRIX: for every verb × {anonymous, Bob-unauthorized}, the response to the EXISTING-but-
    /// forbidden target MUST be BYTE-IDENTICAL to the response to the MISSING target — no verb is an
    /// existence oracle, and the denial code is the requester's (401 anon / 403 Bob), never a 404.
    #[tokio::test]
    async fn matrix_missing_equals_forbidden_byte_identical_for_every_verb() {
        for verb in ["GET", "HEAD", "PUT", "POST", "PATCH", "DELETE"] {
            for (label, token_fn) in [
                ("anonymous", anon as fn() -> VerifiedToken),
                ("bob-unauthorized", bob as fn() -> VerifiedToken),
            ] {
                // A FRESH fixture per (verb, requester) so a mutating verb on one axis cannot perturb
                // the other (e.g. a stray write changing membership/ETag).
                let state_existing = dropbox_with_secret().await;
                let state_missing = dropbox_with_secret().await;
                let on_existing = run_verb(&state_existing, verb, EXISTING, token_fn()).await;
                let on_missing = run_verb(&state_missing, verb, MISSING, token_fn()).await;

                assert_eq!(
                    on_existing, on_missing,
                    "{verb} as {label}: the exists-but-forbidden response must be BYTE-IDENTICAL to the \
                     not-found response (else it is an existence oracle).\n exists:  {on_existing:?}\n \
                     missing: {on_missing:?}"
                );
                // And the denial code is the requester's — NEVER a 404 (only an authorized holder of the
                // required mode learns 404). Anonymous → 401, Bob (authenticated) → 403.
                let expected = if label == "anonymous" { 401 } else { 403 };
                assert_eq!(
                    on_existing.status, expected,
                    "{verb} as {label}: must be the denial code {expected}, never 404/2xx"
                );
                assert_ne!(
                    on_existing.status, 404,
                    "{verb} as {label}: an under-authorized requester must NEVER see 404"
                );
                // A POST/PUT/PATCH denial must not have leaked a Location (no created child revealed).
                assert!(
                    on_existing.location.is_none(),
                    "{verb} as {label}: a denial must carry no Location"
                );
            }
        }
    }

    /// POSITIVE control: an AUTHORIZED reader (Alice, who has inheritable Read on `/alice/c/`) gets a
    /// TRUE 404 on a genuinely-missing resource — the rule keeps the authorized-reader-404 (the CTH
    /// `read-access-*` fictive rows + `post-target-not-found` GET depend on this). Bob/anon get the
    /// denial for the SAME missing path (already covered by the matrix) — so 404 ⇒ "you were allowed to
    /// know, and it isn't there."
    #[tokio::test]
    async fn authorized_reader_gets_true_404_on_genuinely_missing() {
        let state = dropbox_with_secret().await;
        let alice = VerifiedToken {
            web_id: Some(ALICE.into()),
            ..VerifiedToken::default()
        };
        let got = run_verb(&state, "GET", MISSING, alice.clone()).await;
        assert_eq!(
            got.status, 404,
            "an authorized reader (Alice) must get a TRUE 404 on a missing resource: {got:?}"
        );
        // HEAD likewise.
        let head = run_verb(&state, "HEAD", MISSING, alice).await;
        assert_eq!(head.status, 404, "HEAD must also be a true 404 for Alice");
    }

    // --- V6: POST descendant-existence oracle (acl:default-Append drop-box) closed -----------------
    //
    // The PR #3 verifier execution-proved a SECOND existence oracle beyond the matrix above: an agent
    // holding `acl:default acl:Append` on a container `/c/` (append FLOWS TO DESCENDANTS — the realistic
    // "drop a file anywhere under /c/" grant, unlike the matrix's pure `acl:accessTo`-Append drop-box
    // which cannot name a descendant) could POST to a SPECIFIC sub-container name and distinguish 403
    // (exists, its own `.acl` denies him) from 404 (missing → inherited default authorizes, then the
    // container-exists check 404s). The V6 closure Read-gates the POST existence branch: a requester
    // WITHOUT `acl:Read` on the target gets the byte-identical denial for missing AND locked, so the
    // status reveals nothing; a Read holder (the owner / the CTH `post-target-not-found` client) keeps
    // the true 404. See `guard_post_existence_requires_read` + `research/lws-design-records.md` §6.

    const DAVE: &str = "https://pod.example/dave/profile/card#me";
    const EVE: &str = "https://pod.example/eve/profile/card#me";

    /// The exact fixture the PR #3 verifier used. `/alice/c/` exists; its `.acl` grants:
    ///  - ALICE (OWNER): `acl:accessTo` + `acl:default` Read/Write/Control (inheritable Read),
    ///  - BOB (STRANGER): `acl:default acl:Append` ONLY — the drop-anywhere grant (flows to descendants,
    ///    NO Read, NO `accessTo` on the container),
    ///  - CAROL: `acl:accessTo acl:Append` + `acl:default acl:Append` — an accessTo-Append POSTer still
    ///    WITHOUT Read (proves the Read-gate folds her too, where an accessTo-vs-default split wouldn't).
    ///  - DAVE: `acl:default acl:Write` (no Read) — a write-anywhere holder. POST to a NON-container
    ///    requires `acl:Write` (not Append), so DAVE is the principal that actually reaches (and is
    ///    folded by) the non-container 404/405 branch — an Append-only holder is denied at authorization.
    ///  - EVE: `acl:default acl:Control` (no Read) — reaches a POST to a `.acl` target (which requires
    ///    Control) and MUST keep the true 404/405 there, since Control governs an `.acl`'s existence.
    ///
    /// An EXISTING sub-container `/alice/c/hidden/` carries its OWN restrictive `.acl` (Alice only) so
    /// Bob/Carol are DENIED there (its `accessTo` overrides the inherited default). `/alice/c/ghost/` is
    /// never created (missing). An EXISTING open plain resource `/alice/c/opendoc` (no own `.acl`) lets a
    /// no-Read writer inherit Write but not Read — for the non-container 405 branch.
    async fn store_dropbox_default_append_locked_subcontainer(
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(
                "https://pod.example/alice/c/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed container");
        let c_acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{OWNER}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{STRANGER}>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Append.
<#carol> a acl:Authorization; acl:agent <{CAROL}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Append.
<#dave> a acl:Authorization; acl:agent <{DAVE}>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Write.
<#eve> a acl:Authorization; acl:agent <{EVE}>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Control."#
        );
        store
            .write(
                "https://pod.example/alice/c/.acl",
                AxBytes::from(c_acl),
                "text/turtle",
            )
            .await
            .expect("seed container acl");
        // Existing LOCKED sub-container with its OWN restrictive `.acl` (Alice only → Bob/Carol denied).
        store
            .write(
                "https://pod.example/alice/c/hidden/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed hidden subcontainer");
        let hidden_acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{OWNER}>; acl:accessTo <https://pod.example/alice/c/hidden/>; acl:default <https://pod.example/alice/c/hidden/>; acl:mode acl:Read, acl:Write, acl:Control."#
        );
        store
            .write(
                "https://pod.example/alice/c/hidden/.acl",
                AxBytes::from(hidden_acl),
                "text/turtle",
            )
            .await
            .expect("seed hidden acl");
        // Existing OPEN plain resource (no own `.acl`) — a no-Read writer inherits Append but not Read.
        store
            .write(
                "https://pod.example/alice/c/opendoc",
                AxBytes::from(
                    "<https://pod.example/alice/c/opendoc#me> <http://p> <http://o> .".to_string(),
                ),
                "text/turtle",
            )
            .await
            .expect("seed opendoc");
        Arc::new(LdpState::new(store, "https://pod.example"))
    }

    #[tokio::test]
    async fn v6_post_default_append_dropbox_existence_oracle_closed() {
        // Bob holds `acl:default acl:Append` over /alice/c/ (drop-anywhere) but NO Read. Probing a
        // specific descendant CONTAINER name must NOT reveal existence: a MISSING sub-container (ghost/)
        // and an EXISTING-but-locked one (hidden/, own `.acl` denies Bob) MUST be BYTE-IDENTICAL denials
        // — before this closure ghost/ was a 404 and hidden/ a 403 (the proven oracle).
        let state = store_dropbox_default_append_locked_subcontainer().await;
        let on_missing = run_verb(&state, "POST", "/alice/c/ghost/", bob()).await;
        let on_locked = run_verb(&state, "POST", "/alice/c/hidden/", bob()).await;
        assert_eq!(
            on_missing, on_locked,
            "POST to a MISSING sub-container must be BYTE-IDENTICAL to an EXISTING-but-locked one \
             (else it is an existence oracle).\n missing: {on_missing:?}\n locked:  {on_locked:?}"
        );
        assert_eq!(
            on_missing.status, 403,
            "the folded denial is 403 for authenticated Bob, never the 404 that reveals absence"
        );
        assert_ne!(
            on_missing.status, 404,
            "a no-Read drop-box writer must NEVER see the 404 existence signal"
        );
        assert!(
            on_missing.location.is_none(),
            "a folded POST denial must carry no Location"
        );
    }

    #[tokio::test]
    async fn v6_post_write_without_read_non_container_branch_closed() {
        // The NON-container existence branch (405 when a resource is present, 404 when absent) is folded
        // for a no-Read WRITER. NB: POST to a non-container requires `acl:Write` (not Append), so the
        // principal that reaches this branch is DAVE (`acl:default acl:Write`, no Read) — an Append-only
        // holder is denied at authorization and never gets here (which is why the earlier Append-only
        // version of this test did not actually exercise the guard: roborev #4549 Low). DAVE POSTing to a
        // MISSING plain path (ghostdoc → was 404) and to an EXISTING open plain resource (opendoc → was
        // 405, he inherits Write not Read) both fold to 403, indistinguishable from one another.
        let state = store_dropbox_default_append_locked_subcontainer().await;
        let dave = VerifiedToken {
            web_id: Some(DAVE.into()),
            ..VerifiedToken::default()
        };
        let on_missing_doc = run_verb(&state, "POST", "/alice/c/ghostdoc", dave.clone()).await;
        let on_open_doc = run_verb(&state, "POST", "/alice/c/opendoc", dave).await;
        assert_eq!(
            on_missing_doc.status, 403,
            "a MISSING non-container POST (was 404) folds to 403 for a Write-without-Read writer: {on_missing_doc:?}"
        );
        assert_eq!(
            on_open_doc.status, 403,
            "an EXISTING open non-container POST (was 405) folds to 403 for a Write-without-Read writer: {on_open_doc:?}"
        );
        assert_eq!(
            on_missing_doc, on_open_doc,
            "the non-container 404 and 405 branches must both fold to the SAME denial (no oracle)"
        );
    }

    #[tokio::test]
    async fn v6_post_control_holder_on_acl_target_keeps_true_existence_status() {
        // The `.acl` Control-as-read case (roborev #4549 Medium): a POST to a `.acl` target requires
        // `acl:Control` (any operation on an `.acl` is Control), and reading an `.acl`'s existence is
        // ALSO governed by Control — so a Control-holder WITHOUT Read must keep the true 404/405, NOT be
        // folded (a naive Read-only gate would wrongly fold them). EVE holds `acl:default acl:Control`
        // (no Read); POSTing to a MISSING `.acl` target she gets the true 404 (a `.acl` is a non-container
        // → the 404 absent / 405 present branch). Anyone WITHOUT Control is already denied at
        // authorization (POST-to-`.acl` requires Control), so the guard never wrongly discloses to a
        // non-entitled principal.
        let state = store_dropbox_default_append_locked_subcontainer().await;
        let eve = VerifiedToken {
            web_id: Some(EVE.into()),
            ..VerifiedToken::default()
        };
        // A MISSING `.acl` target: EVE (Control, the `.acl` read-mode) must see the TRUE 404, not a fold.
        let on_missing_acl = run_verb(&state, "POST", "/alice/c/ghostdoc.acl", eve).await;
        assert_eq!(
            on_missing_acl.status, 404,
            "a Control-holder POSTing to a missing `.acl` keeps the true 404 (Control governs `.acl` \
             existence — must not be folded like a plain Read-less writer): {on_missing_acl:?}"
        );
    }

    #[tokio::test]
    async fn v6_post_owner_still_gets_true_404_on_missing_subcontainer() {
        // CTH preservation (`post-target-not-found`): the OWNER (Alice, inheritable Read via `acl:default`)
        // POSTing to a genuinely-missing sub-container gets a TRUE 404 — the Read-gate folds ONLY non-Read
        // requesters, never a Read holder. Alice reaches the missing target exactly as the CTH's
        // `clients.alice` does: authorized via inherited `acl:default`, holding NO `accessTo` there (so an
        // accessTo-based distinguisher would have WRONGLY folded her → this is why Read is the right gate).
        let state = store_dropbox_default_append_locked_subcontainer().await;
        let alice = VerifiedToken {
            web_id: Some(OWNER.into()),
            ..VerifiedToken::default()
        };
        let got = run_verb(&state, "POST", "/alice/c/ghost/", alice).await;
        assert_eq!(
            got.status, 404,
            "an authorized-Read owner must keep the true 404 on a missing POST target (CTH): {got:?}"
        );
    }

    #[tokio::test]
    async fn v6_post_accessto_append_without_read_is_also_folded() {
        // Carol holds `acl:accessTo acl:Append` on the container (a "genuine POSTer" by an accessTo-vs-
        // default distinguisher) PLUS `acl:default acl:Append` (so she can reach a sub-container) but NO
        // Read. An accessTo-vs-default split would hand her a 404 on ghost/ and thereby REOPEN the oracle
        // (she still gets 403 on hidden/). The Read-gate correctly folds her too: ghost/ == hidden/ == 403.
        let state = store_dropbox_default_append_locked_subcontainer().await;
        let carol = VerifiedToken {
            web_id: Some(CAROL.into()),
            ..VerifiedToken::default()
        };
        let on_missing = run_verb(&state, "POST", "/alice/c/ghost/", carol.clone()).await;
        let on_locked = run_verb(&state, "POST", "/alice/c/hidden/", carol).await;
        assert_eq!(
            on_missing, on_locked,
            "accessTo-Append-WITHOUT-Read: missing must equal locked (oracle closed for her too):\n \
             missing: {on_missing:?}\n locked: {on_locked:?}"
        );
        assert_eq!(
            on_missing.status, 403,
            "accessTo-Append-without-Read folds to 403 on a missing child (Read is the gate, not accessTo)"
        );
    }

    #[tokio::test]
    async fn v6_post_append_only_dropbox_create_into_existing_container_still_201() {
        // The Read-gate folds ONLY the existence-disclosing 404/405 branches — the SUCCESS path is
        // untouched. An `acl:Append`-only drop-box writer (no Read) POSTing INTO the EXISTING container he
        // holds Append on still mints a member → 201. (Reuses the canonical Append-only fixture.)
        let store = store_alice_container_bob_append_only().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let got = run_verb(&state, "POST", "/alice/c/", bob()).await;
        assert_eq!(
            got.status, 201,
            "an Append-only drop-box POST into an EXISTING container must still succeed (201): {got:?}"
        );
        assert!(
            got.location.is_some(),
            "a successful drop-box POST must return a Location"
        );
    }

    // --- V6 (roborev job 4551, Medium): the missing-container existence probe must not leak a backend
    //     fault as a 500 to a no-read requester (a `patch_*_faulting_target_read`-class oracle) ---------

    /// A [`Store`] whose `exists` FAULTS (a non-`NotFound` backend inconsistency) while it serves a real
    /// container `.acl` at `/alice/c/.acl` so authorization reaches a genuine allow. The `.acl` grants
    /// ALICE (`OWNER`) `acl:default` Read/Write/Control and BOB (`STRANGER`) `acl:default acl:Append` only
    /// (no Read) — both inherit onto the descendant `/alice/c/ghost/`. A POST to that (would-be) missing
    /// container passes authorization (both hold Append via inherited default), THEN the existence probe
    /// faults: a no-Read Bob MUST be folded to 403 (never the 500), while the Read-holding owner gets the
    /// 500 surfaced post-auth (entitled to the target's state).
    struct PostExistsFaultyStore;

    impl PostExistsFaultyStore {
        fn container_acl() -> String {
            format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{OWNER}>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{STRANGER}>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Append."#
            )
        }
        fn acl_meta() -> ResourceMeta {
            ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "k".into(),
                etag: "\"acl\"".into(),
                last_modified: None,
            }
        }
    }

    #[async_trait]
    impl Store for PostExistsFaultyStore {
        async fn read(&self, iri: &str) -> ServerResult<Resource> {
            if iri == "https://pod.example/alice/c/.acl" {
                return Ok(Resource {
                    body: AxBytes::from(Self::container_acl()),
                    meta: Self::acl_meta(),
                });
            }
            Err(ServerError::NotFound)
        }
        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            if iri == "https://pod.example/alice/c/.acl" {
                return Ok(Some(Self::acl_meta()));
            }
            Ok(None)
        }
        async fn exists(&self, _iri: &str) -> ServerResult<bool> {
            // The backend inconsistency the test injects: the existence probe FAULTS (non-NotFound), so a
            // bare `?` at the call site would 500. authorization never calls `exists` (it reads only ACL
            // records via `meta`/`read`), so the fault surfaces ONLY at the post-auth existence check.
            Err(ServerError::Storage(
                "simulated backend inconsistency".into(),
            ))
        }
        async fn write(
            &self,
            _iri: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("write must not be reached: the faulted exists probe folds/surfaces first");
        }
        async fn create_in_container(
            &self,
            _container: &str,
            _child: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("create_in_container must not be reached on a faulted exists probe");
        }
        async fn delete(&self, _iri: &str, _parent: Option<&str>) -> ServerResult<()> {
            Ok(())
        }
        async fn delete_container_if_empty(
            &self,
            _iri: &str,
            _parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            Ok(DeleteOutcome::NotFound)
        }
        async fn list_children(
            &self,
            _container: &str,
        ) -> ServerResult<Vec<crate::store::ValidatedChildIri>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn v6_post_no_read_writer_exists_fault_folds_to_denial_not_500() {
        // roborev 4551 (Medium): a no-Read drop-box writer (Bob, `acl:default acl:Append`) POSTing to a
        // container whose existence probe FAULTS must get the uniform denial (403), NOT a 500 — else the
        // backend fault is an existence/state oracle (an existing-but-forbidden sibling denies at
        // authorization with NO target probe, so it can never 500). The store PANICS if a write is reached.
        let state = Arc::new(LdpState::new(PostExistsFaultyStore, "https://pod.example"));
        let uri: axum::http::Uri = "/alice/c/ghost/".parse().unwrap();
        let err = post_handler(
            State(state),
            Extension(bob()),
            uri,
            turtle_headers(),
            request_body_bytes(),
        )
        .await
        .expect_err("a no-Read writer's faulting existence probe must be a denial, never a 500");
        assert_eq!(
            err.status(),
            StatusCode::FORBIDDEN,
            "a no-Read writer's faulting existence probe must fold to 403, never leak a 500: {err:?}"
        );
    }

    #[tokio::test]
    async fn v6_post_authorized_reader_exists_fault_surfaces_500_post_auth() {
        // The control (mirrors patch_authorized_caller_with_faulting_target_read_gets_500_surfaced_post_auth):
        // the OWNER (Alice, inheritable Read) is ENTITLED to the target's state, so a faulting existence
        // probe surfaces the real 500 AFTER authorization — it is NOT folded. The store PANICS on write,
        // proving the error surfaced before any create.
        let state = Arc::new(LdpState::new(PostExistsFaultyStore, "https://pod.example"));
        let alice = VerifiedToken {
            web_id: Some(OWNER.into()),
            ..VerifiedToken::default()
        };
        let uri: axum::http::Uri = "/alice/c/ghost/".parse().unwrap();
        let err = post_handler(
            State(state),
            Extension(alice),
            uri,
            turtle_headers(),
            request_body_bytes(),
        )
        .await
        .expect_err("a Read-holding owner sees the surfaced backend error");
        assert_eq!(
            err.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "a Read-holding owner is entitled to the surfaced backend error (500), post-auth: {err:?}"
        );
    }

    // --- V1: PUT-create now requires target Write (the drop-box trade-off) -------------------------

    #[tokio::test]
    async fn v1_append_only_put_create_is_denied_not_201() {
        // Bob holds parent `acl:Append` (he can POST) but NOT target `acl:Write`. A PUT-create of a free
        // name MUST now be denied (403) — it previously fell through to a parent-Append 201, which (paired
        // with the 403 on a taken name) leaked existence. The denial is byte-identical to the missing case
        // (covered by the matrix); here we pin the specific status + that NOTHING was created.
        let state = dropbox_with_secret().await;
        let got = run_verb(&state, "PUT", MISSING, bob()).await;
        assert_eq!(
            got.status, 403,
            "an Append-only PUT-create must be a 403, not a 201: {got:?}"
        );
        use crate::store::Store;
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/ghost")
                .await
                .unwrap(),
            "a denied PUT-create must not have written anything"
        );
    }

    /// Owner-writes a plain Turtle resource at `path` into a fresh owner-controlled store, returning
    /// the ready state.
    async fn state_with_owner_resource(
        path: &str,
        body: &'static str,
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = store_with_owner_root_acl().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let uri: axum::http::Uri = path.parse().unwrap();
        put_handler(
            State(state.clone()),
            Extension(owner_token()),
            uri,
            turtle_write_headers(),
            AxBytes::from(body),
        )
        .await
        .expect("owner PUT must succeed");
        state
    }

    /// The `ETag` header of a response as an owned `String`.
    fn etag_of(resp: &Response) -> String {
        resp.headers()
            .get(header::ETAG)
            .expect("a read response must carry an ETag")
            .to_str()
            .unwrap()
            .to_string()
    }

    /// The response body bytes.
    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    async fn get_with(
        state: &Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>>,
        token: VerifiedToken,
        path: &str,
        headers: HeaderMap,
    ) -> Response {
        let uri: axum::http::Uri = path.parse().unwrap();
        get_handler(State(state.clone()), Extension(token), uri, headers)
            .await
            .expect("GET must not error")
    }

    #[tokio::test]
    async fn get_if_none_match_matching_etag_is_304_empty_body_same_etag() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;

        // First an unconditional GET to capture the resource's ETag + body.
        let ok = get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let etag = etag_of(&ok);
        let full_body = body_bytes(ok).await;
        assert!(!full_body.is_empty(), "the 200 must have a body");

        // Now a conditional GET echoing that ETag ⇒ 304 with the IDENTICAL ETag and NO body.
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        let not_mod = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            etag_of(&not_mod),
            etag,
            "the 304 ETag MUST equal the 200 ETag"
        );
        assert!(
            not_mod.headers().get(header::CONTENT_TYPE).is_none(),
            "a 304 carries no representation metadata"
        );
        assert!(
            not_mod.headers().contains_key("wac-allow"),
            "a 304 still carries WAC-Allow (required on GET/HEAD responses)"
        );
        assert!(
            body_bytes(not_mod).await.is_empty(),
            "a 304 carries no body"
        );
    }

    // --- Conditional GET → 304/200 on `If-Modified-Since` (jx3c: last_modified from the index) -----

    /// A far-FUTURE `If-Modified-Since` date on a freshly-written PLAIN resource ⇒ the resource's
    /// server-recorded modification time (≈ now) is `≤` the header ⇒ **304**. This is the end-to-end
    /// proof that `If-Modified-Since` is now LIVE: before the store surfaced `last_modified` the
    /// handler passed `None` and this was always a 200.
    #[tokio::test]
    async fn get_if_modified_since_far_future_is_304() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        let ok = get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let etag = etag_of(&ok);

        let mut cond = HeaderMap::new();
        // Far future — later than the write instant, so `last_modified ≤ header` ⇒ 304.
        cond.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sat, 06 Nov 2100 08:49:37 GMT"),
        );
        let not_mod = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(
            not_mod.status(),
            StatusCode::NOT_MODIFIED,
            "last_modified ≤ If-Modified-Since must be 304 (feature is live)"
        );
        assert_eq!(etag_of(&not_mod), etag, "the 304 ETag equals the 200 ETag");
        assert!(
            body_bytes(not_mod).await.is_empty(),
            "a 304 carries no body"
        );
    }

    /// A far-PAST `If-Modified-Since` date ⇒ the resource was modified AFTER it ⇒ **200** with a body.
    #[tokio::test]
    async fn get_if_modified_since_far_past_is_200() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        let mut cond = HeaderMap::new();
        cond.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );
        let resp = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "last_modified > If-Modified-Since must be a fresh 200"
        );
        assert!(!body_bytes(resp).await.is_empty(), "a 200 carries the body");
    }

    /// A CONTAINER is deliberately excluded from `If-Modified-Since`: its listing is derived from LIVE
    /// membership, which changes without touching the container record's `pss:modified`, so the stored
    /// time is a STALE validator. Even a far-future header must yield a fresh **200**, never a 304 —
    /// otherwise a changed listing could be served as unchanged.
    #[tokio::test]
    async fn get_if_modified_since_on_container_is_200_never_304() {
        // PUT-creating `/alice/doc` also creates the `/alice/` container (ensure_ancestor_containers).
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        // Sanity: the container reads 200 unconditionally.
        let ok = get_with(&state, owner_token(), "/alice/", HeaderMap::new()).await;
        assert_eq!(ok.status(), StatusCode::OK, "container reads 200");

        let mut cond = HeaderMap::new();
        cond.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sat, 06 Nov 2100 08:49:37 GMT"),
        );
        let resp = get_with(&state, owner_token(), "/alice/", cond).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a container must fail OPEN to 200 on If-Modified-Since (never a stale-listing 304)"
        );
    }

    // A minimal public-readable [`Store`] serving ONE plain resource with a CONFIGURABLE
    // `last_modified`, so the exact `≤` boundary + the no-recorded-time path can be driven
    // deterministically end-to-end (the real store stamps `now()`, which is not exactly controllable).
    // Public read = a `foaf:Agent acl:Read` ACL served for the resource's `.acl`, so an anonymous GET
    // is authorized and reaches the precondition check.
    struct FixedTimeStore {
        /// The modification time the index surfaces for the one served resource (`None` ⇒ untracked).
        last_modified: Option<std::time::SystemTime>,
    }

    const FT_TARGET: &str = "https://pod.example/pub/note";

    impl FixedTimeStore {
        fn is_acl(iri: &str) -> bool {
            iri.ends_with(".acl")
        }
        fn resource_meta(&self) -> ResourceMeta {
            ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "b".into(),
                etag: "\"fixed\"".into(),
                last_modified: self.last_modified,
            }
        }
        fn acl_meta() -> ResourceMeta {
            ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "bacl".into(),
                etag: "\"acl\"".into(),
                last_modified: None,
            }
        }
        fn acl_body() -> AxBytes {
            AxBytes::from(format!(
                "@prefix acl: <http://www.w3.org/ns/auth/acl#>.\n\
                 @prefix foaf: <http://xmlns.com/foaf/0.1/>.\n\
                 <#public> a acl:Authorization;\n\
                 acl:agentClass foaf:Agent;\n\
                 acl:accessTo <{FT_TARGET}>;\n\
                 acl:mode acl:Read."
            ))
        }
    }

    #[async_trait]
    impl Store for FixedTimeStore {
        async fn read(&self, iri: &str) -> ServerResult<Resource> {
            if iri == FT_TARGET {
                let body = AxBytes::from(
                    "<https://pod.example/pub/note#me> <http://xmlns.com/foaf/0.1/name> \"P\" .",
                );
                return Ok(Resource {
                    body,
                    meta: self.resource_meta(),
                });
            }
            if Self::is_acl(iri) {
                return Ok(Resource {
                    body: Self::acl_body(),
                    meta: Self::acl_meta(),
                });
            }
            Err(ServerError::NotFound)
        }
        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            if iri == FT_TARGET {
                return Ok(Some(self.resource_meta()));
            }
            if Self::is_acl(iri) {
                return Ok(Some(Self::acl_meta()));
            }
            Ok(None)
        }
        async fn exists(&self, iri: &str) -> ServerResult<bool> {
            Ok(iri == FT_TARGET || Self::is_acl(iri))
        }
        async fn write(
            &self,
            _iri: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("write unused in these read tests");
        }
        async fn create_in_container(
            &self,
            _container: &str,
            _child: &str,
            _body: AxBytes,
            _content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            panic!("create_in_container unused in these read tests");
        }
        async fn delete(&self, _iri: &str, _parent: Option<&str>) -> ServerResult<()> {
            Ok(())
        }
        async fn delete_container_if_empty(
            &self,
            _iri: &str,
            _parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            Ok(DeleteOutcome::NotFound)
        }
        async fn list_children(
            &self,
            _container: &str,
        ) -> ServerResult<Vec<crate::store::ValidatedChildIri>> {
            Ok(Vec::new())
        }
    }

    /// Drive an anonymous GET of the fixed-time resource with one `If-Modified-Since` value.
    async fn get_fixed(last_modified: Option<std::time::SystemTime>, ims: &str) -> StatusCode {
        let state = Arc::new(LdpState::new(
            FixedTimeStore { last_modified },
            "https://pod.example",
        ));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_str(ims).unwrap(),
        );
        let uri: axum::http::Uri = "/pub/note".parse().unwrap();
        get_handler(State(state), Extension(anon()), uri, headers)
            .await
            .expect("public GET must not error")
            .status()
    }

    /// The exact `≤` boundary + either side, driven against a FIXED index time
    /// (`2026-07-05T12:34:56Z` — a Sunday): equal ⇒ 304, one second later ⇒ still 304, one second
    /// earlier ⇒ 200. Deterministic end-to-end proof that the surfaced `last_modified` decides 304 vs
    /// 200 with the RFC 9110 §13.1.3 `≤` semantics.
    #[tokio::test]
    async fn get_if_modified_since_fixed_time_boundary_end_to_end() {
        let fixed = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_783_254_896));
        // Equal instant ⇒ last_modified ≤ header ⇒ 304.
        assert_eq!(
            get_fixed(fixed, "Sun, 05 Jul 2026 12:34:56 GMT").await,
            StatusCode::NOT_MODIFIED,
            "equal ⇒ 304 (≤ boundary)"
        );
        // Header one second AFTER ⇒ still ≤ ⇒ 304.
        assert_eq!(
            get_fixed(fixed, "Sun, 05 Jul 2026 12:34:57 GMT").await,
            StatusCode::NOT_MODIFIED,
            "header after last_modified ⇒ 304"
        );
        // Header one second BEFORE ⇒ modified since ⇒ 200.
        assert_eq!(
            get_fixed(fixed, "Sun, 05 Jul 2026 12:34:55 GMT").await,
            StatusCode::OK,
            "header before last_modified ⇒ 200"
        );
    }

    /// A PLAIN resource whose index records NO modification time (`last_modified = None`) ⇒ the
    /// condition cannot be proven ⇒ a fresh **200**, NEVER a spurious 304 — even for a header date
    /// far in the future. (The literal "no recorded modification time" case.)
    #[tokio::test]
    async fn get_if_modified_since_without_recorded_time_is_200() {
        assert_eq!(
            get_fixed(None, "Sat, 06 Nov 2100 08:49:37 GMT").await,
            StatusCode::OK,
            "no recorded last_modified ⇒ 200, never a wrong 304"
        );
    }

    /// The plain resource's modification time is advertised as `Last-Modified` (IMF-fixdate) on the
    /// 200, the HEAD, AND the 304 that shares the validators — so a client can obtain it and echo it
    /// back. `None`-time resources carry no header.
    #[tokio::test]
    async fn get_emits_last_modified_header() {
        let fixed = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_783_254_896));
        let expected = "Sun, 05 Jul 2026 12:34:56 GMT";
        let uri: axum::http::Uri = "/pub/note".parse().unwrap();

        // 200 GET carries Last-Modified.
        let state = Arc::new(LdpState::new(
            FixedTimeStore {
                last_modified: fixed,
            },
            "https://pod.example",
        ));
        let ok = get_handler(
            State(state.clone()),
            Extension(anon()),
            uri.clone(),
            HeaderMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(
            ok.headers().get(header::LAST_MODIFIED).unwrap(),
            expected,
            "200 carries Last-Modified"
        );

        // HEAD carries it identically.
        let head = head_handler(
            State(state.clone()),
            Extension(anon()),
            uri.clone(),
            HeaderMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            head.headers().get(header::LAST_MODIFIED).unwrap(),
            expected,
            "HEAD carries Last-Modified"
        );

        // A 304 (If-None-Match match) still carries it (§15.4.5).
        let etag = etag_of(&ok);
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        let not_mod = get_handler(State(state), Extension(anon()), uri.clone(), cond)
            .await
            .unwrap();
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            not_mod.headers().get(header::LAST_MODIFIED).unwrap(),
            expected,
            "the 304 carries Last-Modified too"
        );

        // A None-time resource carries NO Last-Modified.
        let state_none = Arc::new(LdpState::new(
            FixedTimeStore {
                last_modified: None,
            },
            "https://pod.example",
        ));
        let ok_none = get_handler(State(state_none), Extension(anon()), uri, HeaderMap::new())
            .await
            .unwrap();
        assert!(
            ok_none.headers().get(header::LAST_MODIFIED).is_none(),
            "a resource with no recorded time advertises no Last-Modified"
        );
    }

    /// A CONTAINER advertises no `Last-Modified` — its stale record time is deliberately not exposed
    /// (matching the If-Modified-Since exclusion).
    #[tokio::test]
    async fn get_container_emits_no_last_modified() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        let resp = get_with(&state, owner_token(), "/alice/", HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            resp.headers().get(header::LAST_MODIFIED).is_none(),
            "a container must not advertise its stale record modification time"
        );
    }

    #[tokio::test]
    async fn v1_owner_put_create_still_succeeds_201() {
        // The control: the OWNER (Alice, inheritable Write) can still PUT-create a fresh resource → 201.
        // The V1 tightening (require target Write) must not regress the legitimate create.
        let state = dropbox_with_secret().await;
        let alice = VerifiedToken {
            web_id: Some(ALICE.into()),
            ..VerifiedToken::default()
        };
        let got = run_verb(&state, "PUT", MISSING, alice).await;
        assert_eq!(
            got.status, 201,
            "the owner's PUT-create must still succeed: {got:?}"
        );
    }

    // --- V3: insert-only PATCH-create is symmetric with forbidden-modify ---------------------------

    #[tokio::test]
    async fn v3_append_holder_patch_create_succeeds_but_oracle_is_closed() {
        // An INSERT-ONLY PATCH-create needs `acl:Append` on the TARGET (inherited). Bob has Append only
        // on the CONTAINER, not on the members (the `/alice/c/.acl` grants Bob Append via `acl:accessTo`
        // on the container, NOT `acl:default`), so the member `ghost` does NOT inherit Bob's Append → an
        // insert-only PATCH-create is DENIED (403). Crucially this is the SAME 403 Bob gets modifying the
        // EXISTING forbidden `secret` — no create-vs-modify oracle. (Both are covered byte-identically by
        // the matrix; this pins the V3-specific reasoning.)
        let state_missing = dropbox_with_secret().await;
        let state_existing = dropbox_with_secret().await;
        let create = run_verb(&state_missing, "PATCH", MISSING, bob()).await;
        let modify = run_verb(&state_existing, "PATCH", EXISTING, bob()).await;
        assert_eq!(
            create.status, 403,
            "Bob's PATCH-create is denied: {create:?}"
        );
        assert_eq!(
            create, modify,
            "V3: PATCH-create and PATCH-forbidden-modify must be byte-identical (no existence oracle)"
        );
    }

    // --- V2: the POST Location is collision-INDEPENDENT (no taken-vs-free signal) ------------------

    #[tokio::test]
    async fn v2_post_location_shape_is_collision_independent() {
        // An AUTHORIZED appender POSTing `Slug: foo` gets a `…/foo-<opaque>` Location whether or not
        // `foo` already exists — so the Location reveals nothing about which names are taken. Drive it as
        // Bob (who HOLDS container Append, so the POST is authorized) twice with the same Slug: the two
        // Locations differ (distinct opaque names) and NEITHER is the verbatim `…/foo`.
        let state = dropbox_with_secret().await;
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        let post = |slug: &'static str| {
            let st = State(state.clone());
            let mut headers = turtle_headers();
            headers.insert(
                HeaderName::from_static("slug"),
                HeaderValue::from_static(slug),
            );
            let u = uri.clone();
            async move {
                observe(post_handler(st, Extension(bob()), u, headers, request_body_bytes()).await)
                    .await
            }
        };
        let first = post("foo").await;
        let second = post("foo").await;
        assert_eq!(first.status, 201);
        assert_eq!(second.status, 201);
        let loc1 = first.location.expect("Location");
        let loc2 = second.location.expect("Location");
        // Collision-independent: same Slug, DIFFERENT opaque Locations; neither is the verbatim name.
        assert_ne!(
            loc1, loc2,
            "two POSTs of the same Slug must mint distinct opaque names"
        );
        assert_ne!(
            loc1, "https://pod.example/alice/c/foo",
            "Location must not be the verbatim Slug"
        );
        assert!(
            loc1.starts_with("https://pod.example/alice/c/foo-"),
            "Location must contain the Slug: {loc1}"
        );
        assert!(
            loc2.starts_with("https://pod.example/alice/c/foo-"),
            "Location must contain the Slug: {loc2}"
        );
    }

    #[tokio::test]
    async fn get_200_and_304_both_carry_vary_accept() {
        // RFC 9110 §12.5.5 (roborev finding on 1e5a47d): the representation — and hence its `ETag`
        // — was selected using `Accept`, so BOTH the 200 and the 304 that shares its validator must
        // declare that dependency via `Vary: Accept`, or a shared cache could conflate a Turtle
        // response with a JSON-LD one for the same resource state.
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;

        let ok = get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(
            ok.headers().get(header::VARY).unwrap(),
            "Accept",
            "a 200 must declare Vary: Accept"
        );
        let etag = etag_of(&ok);

        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        let not_mod = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            not_mod.headers().get(header::VARY).unwrap(),
            "Accept",
            "a 304 must ALSO declare Vary: Accept — it shares the 200's negotiated validator"
        );
    }

    #[tokio::test]
    async fn get_if_none_match_non_matching_is_200_with_body() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        let mut cond = HeaderMap::new();
        cond.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"some-other-tag\""),
        );
        let resp = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!body_bytes(resp).await.is_empty());
    }

    #[tokio::test]
    async fn get_if_none_match_star_on_existing_is_304() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
        let resp = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert!(body_bytes(resp).await.is_empty());
    }

    #[tokio::test]
    async fn head_if_none_match_matching_is_304() {
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        // Capture the ETag via HEAD.
        let uri: axum::http::Uri = "/alice/doc".parse().unwrap();
        let head = head_handler(
            State(state.clone()),
            Extension(owner_token()),
            uri.clone(),
            HeaderMap::new(),
        )
        .await
        .unwrap();
        let etag = etag_of(&head);
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        let not_mod = head_handler(State(state), Extension(owner_token()), uri, cond)
            .await
            .unwrap();
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(etag_of(&not_mod), etag);
    }

    #[tokio::test]
    async fn get_range_plus_matching_if_none_match_yields_304_not_206() {
        // RFC 9110: when a Range and a matching If-None-Match are BOTH present, the precondition wins
        // — a 304, never a 206.
        let state = state_with_owner_resource(
            "/alice/doc",
            "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .",
        )
        .await;
        let etag = etag_of(&get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await);
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        cond.insert(header::RANGE, HeaderValue::from_static("bytes=0-3"));
        let resp = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_MODIFIED,
            "the matching If-None-Match must win over Range (no 206)"
        );
    }

    #[tokio::test]
    async fn get_container_conditional_uses_representation_etag() {
        // A container's validator is its RENDERED-representation ETag; a conditional GET echoing it
        // must 304 (and it must equal the 200 ETag).
        let store = store_with_owner_root_acl().await;
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        // Create a child so the container has non-trivial membership.
        put_handler(
            State(state.clone()),
            Extension(owner_token()),
            "/alice/c/child".parse().unwrap(),
            turtle_write_headers(),
            AxBytes::from("<https://pod.example/alice/c/child#i> <http://p> <http://o> ."),
        )
        .await
        .expect("seed child");

        let ok = get_with(&state, owner_token(), "/alice/c/", HeaderMap::new()).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let etag = etag_of(&ok);
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        let not_mod = get_with(&state, owner_token(), "/alice/c/", cond).await;
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(etag_of(&not_mod), etag, "container 304 ETag == 200 ETag");
    }

    #[tokio::test]
    async fn unauthorized_caller_gets_denial_not_304_no_existence_leak() {
        // A stranger with NO read access must get the auth failure (403 authenticated), NOT a 304 —
        // a 304 would leak that the resource exists to a caller who cannot read it. Auth runs BEFORE
        // the precondition, so `If-None-Match: *` cannot short-circuit to 304 for an unauthorized read.
        let state = state_with_owner_resource(
            "/alice/private",
            "<https://pod.example/alice/private#me> <http://xmlns.com/foaf/0.1/name> \"secret\" .",
        )
        .await;
        let stranger = VerifiedToken {
            web_id: Some(STRANGER.into()),
            ..VerifiedToken::default()
        };
        let mut cond = HeaderMap::new();
        cond.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
        let uri: axum::http::Uri = "/alice/private".parse().unwrap();
        let err = get_handler(State(state), Extension(stranger), uri, cond)
            .await
            .expect_err("an unauthorized read must be a denial, never a 304");
        assert_eq!(
            err.status(),
            StatusCode::FORBIDDEN,
            "the unauthorized caller must get 403, not 304"
        );
    }

    // --- Representation-specific validators under content negotiation (RFC 9110 §8.8.3) ------------

    const COND_DOC: &str =
        "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"X\" .";

    #[tokio::test]
    async fn get_stored_tag_with_accept_jsonld_is_200_not_304() {
        // (a) An ETag identifies a REPRESENTATION: a client holding the stored-Turtle tag that asks
        // for JSON-LD does NOT hold that representation — it must get a fresh 200, never a 304.
        let state = state_with_owner_resource("/alice/doc", COND_DOC).await;
        let turtle_tag =
            etag_of(&get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await);

        let mut cond = HeaderMap::new();
        cond.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&turtle_tag).unwrap(),
        );
        cond.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let resp = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a Turtle tag must not 304 a JSON-LD response"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/ld+json"
        );
        assert_ne!(
            etag_of(&resp),
            turtle_tag,
            "the negotiated representation carries its own validator"
        );
        assert!(!body_bytes(resp).await.is_empty());
    }

    #[tokio::test]
    async fn get_negotiated_variant_etag_304s_only_for_that_type() {
        // (b)+(c) The ETag a 200 carries for EACH negotiated type is exactly the tag that later
        // 304s for that type — and only for that type.
        let state = state_with_owner_resource("/alice/doc", COND_DOC).await;
        let turtle_tag =
            etag_of(&get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await);

        // The JSON-LD 200's own tag…
        let mut accept_jsonld = HeaderMap::new();
        accept_jsonld.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let jsonld_tag =
            etag_of(&get_with(&state, owner_token(), "/alice/doc", accept_jsonld).await);
        assert_ne!(jsonld_tag, turtle_tag);

        // …304s a JSON-LD conditional GET, echoing the SAME tag with no body…
        let mut cond = HeaderMap::new();
        cond.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&jsonld_tag).unwrap(),
        );
        cond.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let not_mod = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(etag_of(&not_mod), jsonld_tag);
        assert!(body_bytes(not_mod).await.is_empty());

        // …but never a TURTLE conditional GET (a different representation).
        let mut cond_turtle = HeaderMap::new();
        cond_turtle.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&jsonld_tag).unwrap(),
        );
        let resp = get_with(&state, owner_token(), "/alice/doc", cond_turtle).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a JSON-LD tag must not 304 the stored-Turtle response"
        );
        assert_eq!(etag_of(&resp), turtle_tag);
    }

    #[tokio::test]
    async fn put_if_match_round_trips_with_a_negotiated_variant_etag() {
        // Writes guard resource STATE: the client that GETs the JSON-LD representation must be able
        // to PUT with the ETag it received (If-Match on the state part) — and a variant of a STALE
        // state must still 412.
        let state = state_with_owner_resource("/alice/doc", COND_DOC).await;
        let mut accept_jsonld = HeaderMap::new();
        accept_jsonld.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let jsonld_tag =
            etag_of(&get_with(&state, owner_token(), "/alice/doc", accept_jsonld).await);

        // GET (JSON-LD) → If-Match PUT with the received tag: round-trips (state unchanged).
        let mut write = turtle_write_headers();
        write.insert(
            header::IF_MATCH,
            HeaderValue::from_str(&jsonld_tag).unwrap(),
        );
        let uri: axum::http::Uri = "/alice/doc".parse().unwrap();
        let resp = put_handler(
            State(state.clone()),
            Extension(owner_token()),
            uri.clone(),
            write.clone(),
            AxBytes::from(
                "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"Y\" .",
            ),
        )
        .await
        .expect("If-Match with the negotiated variant tag must round-trip");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // The state has now CHANGED — the old variant tag is stale, so the same If-Match is a 412.
        let err = put_handler(
            State(state),
            Extension(owner_token()),
            uri,
            write,
            AxBytes::from(
                "<https://pod.example/alice/doc#me> <http://xmlns.com/foaf/0.1/name> \"Z\" .",
            ),
        )
        .await
        .expect_err("a variant of a stale state must fail the precondition");
        assert_eq!(err.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[test]
    fn negotiated_validator_binary_keeps_the_stored_tag() {
        // (d) A non-RDF (binary) resource is served verbatim whatever the Accept — ONE
        // representation, so its validator stays the stored tag (current behaviour preserved).
        let stored = "\"5-abc123\"";
        assert_eq!(
            negotiated_validator(stored, "image/png", Some("application/ld+json")).unwrap(),
            stored
        );
        assert_eq!(
            negotiated_validator(stored, "image/png", None).unwrap(),
            stored
        );
        // And an RDF resource served in its stored format keeps the stored tag too.
        assert_eq!(
            negotiated_validator(stored, "text/turtle", Some("text/turtle")).unwrap(),
            stored
        );
        // While the re-serialised representation gets the distinct variant tag.
        assert_eq!(
            negotiated_validator(stored, "text/turtle", Some("application/ld+json")).unwrap(),
            "\"5-abc123+jsonld\""
        );
    }

    #[test]
    fn negotiated_validator_is_profile_variant_specific_when_the_bytes_differ() {
        // sq-10ty4: an honoured JSON-LD `profile` changes which bytes are served, so it changes
        // the validator too — exactly when the bytes differ.
        let stored = "\"5-abc123\"";
        const EXPANDED: &str = "application/ld+json;profile=\"http://www.w3.org/ns/json-ld#expanded\"";
        const COMPACTED: &str =
            "application/ld+json;profile=\"http://www.w3.org/ns/json-ld#compacted\"";
        // Compacted output differs from the default JSON-LD serialisation ⇒ its own variant tag.
        assert_eq!(
            negotiated_validator(stored, "text/turtle", Some(COMPACTED)).unwrap(),
            "\"5-abc123+jsonld-c\""
        );
        // Expanded output is byte-identical to the default serialisation ⇒ the same variant tag.
        assert_eq!(
            negotiated_validator(stored, "text/turtle", Some(EXPANDED)).unwrap(),
            "\"5-abc123+jsonld\""
        );
        // A stored-JSON-LD resource under an honoured profile is NOT served verbatim (the stored
        // bytes are whatever form the client wrote), so the stored tag no longer applies…
        assert_eq!(
            negotiated_validator(stored, "application/ld+json", Some(EXPANDED)).unwrap(),
            "\"5-abc123+jsonld\""
        );
        assert_eq!(
            negotiated_validator(stored, "application/ld+json", Some(COMPACTED)).unwrap(),
            "\"5-abc123+jsonld-c\""
        );
        // …while with no profile it stays the verbatim stored representation.
        assert_eq!(
            negotiated_validator(stored, "application/ld+json", Some("application/ld+json"))
                .unwrap(),
            stored
        );
    }

    #[tokio::test]
    async fn get_jsonld_compacted_profile_is_honoured_end_to_end() {
        // sq-10ty4: Accept with the compacted profile ⇒ the response echoes the profile in
        // Content-Type, serves GENUINELY compacted JSON-LD, and mints a profile-specific variant
        // ETag that 304s only for that representation.
        const COMPACTED: &str =
            "application/ld+json;profile=\"http://www.w3.org/ns/json-ld#compacted\"";
        let state = state_with_owner_resource("/alice/doc", COND_DOC).await;

        // The plain (no-profile) JSON-LD tag, for contrast.
        let mut accept_plain = HeaderMap::new();
        accept_plain.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let plain_tag = etag_of(&get_with(&state, owner_token(), "/alice/doc", accept_plain).await);

        let mut accept = HeaderMap::new();
        accept.insert(header::ACCEPT, HeaderValue::from_static(COMPACTED));
        let resp = get_with(&state, owner_token(), "/alice/doc", accept.clone()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // The honoured profile is echoed back in the Content-Type (JSON-LD 1.1 IANA registration).
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            COMPACTED
        );
        let compacted_tag = etag_of(&resp);
        assert_ne!(
            compacted_tag, plain_tag,
            "compacted bytes differ from the default serialisation, so the tag must too"
        );
        // The body is genuinely compacted: a single-subject doc compacts to the bare node object
        // with the lone plain literal collapsed to a string.
        let body: serde_json::Value =
            serde_json::from_slice(&body_bytes(resp).await).expect("valid JSON");
        assert_eq!(
            body.get("@id").and_then(|v| v.as_str()),
            Some("https://pod.example/alice/doc#me")
        );
        assert_eq!(
            body.get("http://xmlns.com/foaf/0.1/name"),
            Some(&serde_json::json!("X"))
        );

        // The compacted tag 304s a conditional GET for the SAME representation…
        let mut cond = accept.clone();
        cond.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&compacted_tag).unwrap(),
        );
        let not_mod = get_with(&state, owner_token(), "/alice/doc", cond).await;
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(etag_of(&not_mod), compacted_tag);

        // …but the PLAIN JSON-LD tag never 304s the compacted representation.
        let mut cond_plain = accept;
        cond_plain.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&plain_tag).unwrap(),
        );
        let resp = get_with(&state, owner_token(), "/alice/doc", cond_plain).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a plain-JSON-LD tag must not 304 the compacted representation"
        );
    }

    #[tokio::test]
    async fn get_jsonld_expanded_profile_echoes_profile_with_byte_identical_body() {
        // sq-10ty4: the serialiser's default output IS the expanded document form, so the expanded
        // profile is honoured with byte-identical output — same variant ETag, but the Content-Type
        // echoes the honoured profile.
        const EXPANDED: &str =
            "application/ld+json;profile=\"http://www.w3.org/ns/json-ld#expanded\"";
        let state = state_with_owner_resource("/alice/doc", COND_DOC).await;

        let mut accept_plain = HeaderMap::new();
        accept_plain.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let plain = get_with(&state, owner_token(), "/alice/doc", accept_plain).await;
        let plain_tag = etag_of(&plain);
        let plain_body = body_bytes(plain).await;

        let mut accept = HeaderMap::new();
        accept.insert(header::ACCEPT, HeaderValue::from_static(EXPANDED));
        let resp = get_with(&state, owner_token(), "/alice/doc", accept).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), EXPANDED);
        assert_eq!(
            etag_of(&resp),
            plain_tag,
            "byte-identical representations share the validator"
        );
        assert_eq!(body_bytes(resp).await, plain_body);
    }

    #[tokio::test]
    async fn get_container_jsonld_compacted_profile_echoes_content_type() {
        // sq-10ty4 on the container read path: render_container honours the profile too — the
        // Content-Type echo plus a compacted (bare node object) body, and the representation ETag
        // (hashed from the rendered bytes) is distinct from the plain JSON-LD listing's.
        const COMPACTED: &str =
            "application/ld+json;profile=\"http://www.w3.org/ns/json-ld#compacted\"";
        let state = state_with_owner_resource("/alice/c/child", COND_DOC).await;

        let mut accept_plain = HeaderMap::new();
        accept_plain.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/ld+json"),
        );
        let plain_tag = etag_of(&get_with(&state, owner_token(), "/alice/c/", accept_plain).await);

        let mut accept = HeaderMap::new();
        accept.insert(header::ACCEPT, HeaderValue::from_static(COMPACTED));
        let resp = get_with(&state, owner_token(), "/alice/c/", accept).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            COMPACTED
        );
        assert_ne!(etag_of(&resp), plain_tag);
        // The single-subject container listing compacts to the bare node object; its containment
        // triple compacts to a single node reference.
        let body: serde_json::Value =
            serde_json::from_slice(&body_bytes(resp).await).expect("valid JSON");
        assert_eq!(
            body.get("@id").and_then(|v| v.as_str()),
            Some("https://pod.example/alice/c/")
        );
        assert_eq!(
            body.get("http://www.w3.org/ns/ldp#contains"),
            Some(&serde_json::json!({"@id": "https://pod.example/alice/c/child"}))
        );
    }

    #[tokio::test]
    async fn post_slug_dot_acl_anonymous_on_public_append_gets_401_not_bare_403() {
        // roborev denial-shape consistency: on a PUBLIC-`acl:Append` container an ANONYMOUS caller CAN
        // pass POST authorization and reach the `.acl`-intent guard. Its denial must carry the
        // requester's shape — 401 + `WWW-Authenticate` for anonymous — NOT a bare 403, so the
        // `.acl`-intent case is indistinguishable in shape from any other unauthorized anonymous POST.
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(
                "https://pod.example/alice/c/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed container");
        // The container grants the PUBLIC (`foaf:Agent`) Append — so anonymous may POST.
        let acl = r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#pub> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <https://pod.example/alice/c/>; acl:mode acl:Append."#;
        store
            .write(
                "https://pod.example/alice/c/.acl",
                AxBytes::from(acl.to_string()),
                "text/turtle",
            )
            .await
            .expect("seed public-append acl");
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let uri: axum::http::Uri = "/alice/c/".parse().unwrap();
        // Anonymous POST with `Slug: secret.acl` (a benign body — the body is irrelevant; the INTENT is
        // what is refused).
        let got = observe(
            post_handler(
                State(state.clone()),
                Extension(anon()),
                uri,
                post_turtle_headers_with_slug("secret.acl"),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 401,
            "an anonymous `.acl`-intent POST on a public-append container must be 401 (not a bare 403)"
        );
        assert!(
            got.www_authenticate.is_some(),
            "the anonymous denial must carry a WWW-Authenticate challenge"
        );
        // A benign anonymous Slug on the SAME container still succeeds (public Append is real) — the
        // guard rejects only the `.acl` intent, uniformly by shape.
        let benign = observe(
            post_handler(
                State(state),
                Extension(anon()),
                "/alice/c/".parse().unwrap(),
                post_turtle_headers_with_slug("benign"),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            benign.status, 201,
            "a benign anonymous Slug on a public-append container must still succeed: {benign:?}"
        );
    }

    // --- V4: a conditional precondition requires Read (the Write-without-Read shape) ---------------

    /// A store where Bob holds `acl:Write` (and Append) on `/alice/c/wonly` but NOT `acl:Read` — the
    /// "Write-without-Read" shape. Alice owns the container. An EXISTING `wonly` is present.
    ///
    /// Bob is ALSO granted `acl:Write` on the CONTAINER itself (via the container's own `acl:accessTo`),
    /// so that a DELETE of `wonly` PASSES its parent-containment write authorization (`acl:Write` on the
    /// nearest parent) and actually REACHES the V4 conditional-read guard — without this, the DELETE
    /// would be denied at the parent-write check and the V4 DELETE test would pass for the wrong reason
    /// (the roborev finding). The container grant deliberately omits `acl:Read`, and `wonly`'s OWN `.acl`
    /// (below) overrides inheritance, so Bob's effective modes on `wonly` stay exactly {Write, Append} —
    /// no Read — preserving the Write-without-Read shape the V4 guard is meant to catch.
    async fn store_bob_write_without_read(
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(
                "https://pod.example/alice/c/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed container");
        store
            .write(
                "https://pod.example/alice/c/wonly",
                AxBytes::from(
                    "<https://pod.example/alice/c/wonly#me> <http://p> <http://o> .".to_string(),
                ),
                "text/turtle",
            )
            .await
            .expect("seed wonly");
        // Alice: full control over the container + members (default). Bob: `acl:Write` on the CONTAINER
        // itself (so a DELETE's parent-write check passes and the V4 guard is reached) — but NO `acl:Read`
        // on the container, and `wonly`'s OWN `.acl` overrides inheritance so Bob never gains Read on
        // `wonly`.
        let container_acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <https://pod.example/alice/c/>; acl:mode acl:Write, acl:Append."#
        );
        store
            .write(
                "https://pod.example/alice/c/.acl",
                AxBytes::from(container_acl),
                "text/turtle",
            )
            .await
            .expect("seed container acl");
        let wonly_acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/wonly>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <https://pod.example/alice/c/wonly>; acl:mode acl:Write, acl:Append."#
        );
        store
            .write(
                "https://pod.example/alice/c/wonly.acl",
                AxBytes::from(wonly_acl),
                "text/turtle",
            )
            .await
            .expect("seed wonly acl");
        Arc::new(LdpState::new(store, "https://pod.example"))
    }

    #[tokio::test]
    async fn v4_write_without_read_conditional_put_is_denied_not_412_or_2xx() {
        // Bob has Write but NOT Read on `wonly`. A conditional `PUT … If-Match: "x"` would otherwise
        // yield a 412-vs-2xx outcome (an existence/content probe) + an ETag of a body Bob cannot GET.
        // V4 folds it to Bob's denial code (403) BEFORE any precondition evaluation.
        let state = store_bob_write_without_read().await;
        let uri: axum::http::Uri = "/alice/c/wonly".parse().unwrap();
        let mut headers = turtle_headers();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"deadbeef\""));
        let got = observe(
            put_handler(
                State(state.clone()),
                Extension(bob()),
                uri,
                headers,
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "a Write-without-Read conditional PUT must be the denial code, not 412/2xx: {got:?}"
        );
        assert!(got.etag.is_none(), "a V4 denial must not leak an ETag");
    }

    #[tokio::test]
    async fn v4_write_without_read_unconditional_put_still_succeeds() {
        // The control: WITHOUT a conditional header, Bob's Write IS sufficient — an unconditional PUT to
        // `wonly` succeeds (204). V4 only gates the CONDITIONAL channel; it must not block a plain write.
        let state = store_bob_write_without_read().await;
        let uri: axum::http::Uri = "/alice/c/wonly".parse().unwrap();
        let got = observe(
            put_handler(
                State(state),
                Extension(bob()),
                uri,
                turtle_headers(),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 204,
            "an unconditional PUT by a Write holder must still succeed: {got:?}"
        );
    }

    #[tokio::test]
    async fn v4_write_without_read_conditional_delete_is_denied() {
        // The same closure on DELETE: a `DELETE … If-Match` by a Write-without-Read holder folds to the
        // denial, not the 412-vs-204 existence/content outcome.
        //
        // The fixture grants Bob `acl:Write` on the CONTAINER (so the DELETE's parent-containment write
        // authorization PASSES and control reaches the V4 guard) but NO `acl:Read` on `wonly` — so the
        // 403 here is genuinely from V4, NOT from the parent-write check. The unconditional-DELETE
        // control below PROVES that: the SAME Bob CAN delete `wonly` without a conditional header, so the
        // only thing that turns the conditional DELETE into a 403 is the V4 guard.
        let state = store_bob_write_without_read().await;
        let uri: axum::http::Uri = "/alice/c/wonly".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"deadbeef\""));
        let got = observe(delete_handler(State(state), Extension(bob()), uri, headers).await).await;
        assert_eq!(
            got.status, 403,
            "a Write-without-Read conditional DELETE must be the denial code: {got:?}"
        );
    }

    #[tokio::test]
    async fn v4_write_without_read_unconditional_delete_succeeds_proving_v4_is_the_cause() {
        // CONTROL for the test above (the roborev finding): with the SAME fixture and NO conditional
        // header, Bob's Write (on `wonly`) + container Write (parent-containment) IS sufficient to delete
        // `wonly` → 204. This proves the conditional-DELETE 403 above comes from the V4 guard, not the
        // parent-write authorization — the test is not vacuous.
        let state = store_bob_write_without_read().await;
        let uri: axum::http::Uri = "/alice/c/wonly".parse().unwrap();
        let got =
            observe(delete_handler(State(state), Extension(bob()), uri, HeaderMap::new()).await)
                .await;
        assert_eq!(
            got.status, 204,
            "an UNCONDITIONAL DELETE by the same Write holder must succeed — proving the conditional \
             403 is from V4, not the parent-write check: {got:?}"
        );
    }

    #[tokio::test]
    async fn v4_control_only_holder_conditional_acl_write_is_not_wrongly_denied() {
        // EDGE: the V4 read-mode for an `.acl` target is CONTROL, not Read (reading an `.acl`'s
        // representation is a Control op; `Control` does NOT imply `Read`). A holder of Control-but-NOT-
        // Read on a resource IS entitled to its `.acl`'s ETag, so a CONDITIONAL `.acl` write by such a
        // holder must NOT be folded to a denial by V4. This pins the regression: if the guard used `Read`
        // (instead of the `.acl` read-mode `Control`) it would wrongly 403 this legitimate write.
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        // Alice gets ONLY Control on `/alice/c/manager` (no Read, no Write) — a pure access-manager.
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/manager>; acl:mode acl:Control."#
        );
        store
            .write(
                "https://pod.example/alice/c/manager.acl",
                AxBytes::from(acl),
                "text/turtle",
            )
            .await
            .expect("seed manager .acl");
        let state = Arc::new(LdpState::new(store, "https://pod.example"));
        let alice = VerifiedToken {
            web_id: Some(ALICE.into()),
            ..VerifiedToken::default()
        };
        // A CONDITIONAL PUT REPLACING the existing `.acl` (it exists, so `If-Match: *` is satisfied and
        // the precondition is genuinely evaluated — exercising the V4 gate before it).
        let uri: axum::http::Uri = "/alice/c/manager.acl".parse().unwrap();
        let mut headers = turtle_headers();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("*"));
        let acl_body = AxBytes::from(format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/manager>; acl:mode acl:Control, acl:Read."#
        ));
        let got =
            observe(put_handler(State(state), Extension(alice), uri, headers, acl_body).await)
                .await;
        assert_eq!(
            got.status, 204,
            "a Control-only holder's CONDITIONAL .acl write must reach the write path (204), not a V4 \
             denial — the `.acl` read-mode is Control, not Read: {got:?}"
        );
    }

    // --- V5: the membership-derived container ETag is Read-gated -----------------------------------

    #[tokio::test]
    async fn v5_container_etag_only_reaches_a_reader() {
        // The container's membership-derived ETag is exposed ONLY on the Read-gated GET/HEAD path. An
        // Append-only Bob cannot GET `/alice/c/` (no Read) → 401/403, so he NEVER observes the ETag that
        // shifts on child add/remove. Alice (Read) does observe it. (The conditional-channel sibling — a
        // non-reader probing the ETag via a conditional write — is closed by V4 above.)
        let state = dropbox_with_secret().await;
        // Bob (Append-only, no Read on the container) → denied, no ETag observable.
        let bob_get = run_verb(&state, "GET", "/alice/c/", bob()).await;
        assert_eq!(
            bob_get.status, 403,
            "Bob cannot read the container: {bob_get:?}"
        );
        assert!(
            bob_get.etag.is_none(),
            "a non-reader must observe NO container ETag (it is the membership oracle): {bob_get:?}"
        );
        // Alice (Read) DOES get the container listing + its membership ETag.
        let alice = VerifiedToken {
            web_id: Some(ALICE.into()),
            ..VerifiedToken::default()
        };
        let alice_get = run_verb(&state, "GET", "/alice/c/", alice).await;
        assert_eq!(alice_get.status, 200);
        assert!(
            alice_get.etag.is_some(),
            "the authorized reader DOES receive the container ETag"
        );
    }

    // =====================================================================================
    // CREATE-AUTHZ CONTAINER-MODIFICATION (PR #3 review finding [HIGH]) — creating a member (or an
    // intermediate container) requires `acl:Append`/`Write` on the CONTAINING container, in ADDITION
    // to the target's own effective-ACL mode. An `acl:default`-only Write grant (or a pre-provisioned
    // target `.acl`) must NOT let an agent with NO mode on the container mint members in it. Symmetric
    // with DELETE's parent-Write check.
    // =====================================================================================

    /// A third agent (distinct from ALICE/BOB) for the create-authz positive control.
    const CAROL: &str = "https://pod.example/carol/profile/card#me";

    /// `/alice/c/` exists with a container `.acl` where:
    ///  - ALICE: `acl:accessTo` + `acl:default` Read/Write/Control (full owner).
    ///  - BOB: `acl:default acl:Write` ONLY — inheritable Write on the container's MEMBERS, but NO
    ///    `acl:accessTo` on the container itself (so Bob holds NO mode on `/alice/c/` as a target). This
    ///    is the attacker shape the finding names: Bob can WRITE any member's representation (target-ACL
    ///    Write via default) yet must NOT be able to CREATE one (no container-modification right).
    ///  - CAROL: `acl:default acl:Write` (member Write, so target auth passes) PLUS `acl:accessTo
    ///    acl:Append` on the container (the container-modification right) — the "WITH container
    ///    write/append" agent who IS allowed to create.
    async fn store_default_write_no_container_access(
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(
                "https://pod.example/alice/c/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed container");
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{BOB}>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Write.
<#carol> a acl:Authorization; acl:agent <{CAROL}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Write, acl:Append."#
        );
        store
            .write(
                "https://pod.example/alice/c/.acl",
                AxBytes::from(acl),
                "text/turtle",
            )
            .await
            .expect("seed container acl");
        Arc::new(LdpState::new(store, "https://pod.example"))
    }

    fn carol() -> VerifiedToken {
        VerifiedToken {
            web_id: Some(CAROL.into()),
            ..VerifiedToken::default()
        }
    }

    #[tokio::test]
    async fn create_authz_default_write_only_agent_denied_put_create() {
        // Bob holds member Write via `acl:default` (so his target-ACL Write authorizes the write itself)
        // but NO mode on the container → a PUT-create must be DENIED (403), and nothing written. Pre-fix
        // the create authorized purely against the (inherited) target ACL and returned 201.
        let state = store_default_write_no_container_access().await;
        let got = observe(
            put_handler(
                State(state.clone()),
                Extension(bob()),
                "/alice/c/newdoc".parse().unwrap(),
                turtle_headers(),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "an acl:default-only Write agent with NO container mode must be denied PUT-create: {got:?}"
        );
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/newdoc")
                .await
                .unwrap(),
            "a denied PUT-create must have written nothing"
        );
    }

    #[tokio::test]
    async fn create_authz_default_write_only_agent_denied_patch_create() {
        // The same closure on the create-on-PATCH path: an INSERT-only patch (needs Append on the
        // target, which Bob's member Write satisfies) still must be denied at the container-modification
        // check because Bob holds no Append on the container → 403, nothing written.
        let state = store_default_write_no_container_access().await;
        let got = observe(
            patch_handler(
                State(state.clone()),
                Extension(bob()),
                "/alice/c/newdoc".parse().unwrap(),
                n3_patch_headers(),
                insert_only_patch("https://pod.example/alice/c/newdoc#me"),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "an acl:default-only Write agent must be denied create-on-PATCH: {got:?}"
        );
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/newdoc")
                .await
                .unwrap(),
            "a denied PATCH-create must have written nothing"
        );
    }

    #[tokio::test]
    async fn create_authz_default_write_only_agent_denied_post() {
        // POST already routes the container-modification right through its container-Append
        // authorization: Bob holds no `acl:accessTo` mode on `/alice/c/`, so a POST is denied (403). This
        // pins that CREATE (PUT/PATCH) is now symmetric with POST — all three require the container right.
        let state = store_default_write_no_container_access().await;
        let got = observe(
            post_handler(
                State(state.clone()),
                Extension(bob()),
                "/alice/c/".parse().unwrap(),
                turtle_headers(),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "an acl:default-only Write agent must be denied POST (no container Append): {got:?}"
        );
    }

    #[tokio::test]
    async fn create_authz_default_write_only_agent_denied_deep_ancestor_mint() {
        // The `ensure_ancestor_containers` escalation: Bob PUT-creates `/alice/c/deep/x` (an inherited
        // target ACL grants member Write). The container-modification check authorizes Append on the
        // NEAREST EXISTING ancestor (`/alice/c/`) — which Bob lacks — so the mint of the intermediate
        // container `/alice/c/deep/` is refused (403), and no intermediate container is materialised.
        let state = store_default_write_no_container_access().await;
        let got = observe(
            put_handler(
                State(state.clone()),
                Extension(bob()),
                "/alice/c/deep/x".parse().unwrap(),
                turtle_headers(),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "a deep-ancestor mint by an agent with no container right must be denied: {got:?}"
        );
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/deep/")
                .await
                .unwrap(),
            "a denied deep mint must NOT have materialised the intermediate container"
        );
        assert!(
            !state
                .store
                .exists("https://pod.example/alice/c/deep/x")
                .await
                .unwrap(),
            "a denied deep mint must NOT have written the target"
        );
    }

    #[tokio::test]
    async fn create_authz_agent_with_container_append_is_allowed() {
        // The positive control: CAROL holds member Write (target auth) AND `acl:accessTo acl:Append` on
        // the container (the container-modification right) → her PUT-create succeeds (201). This proves
        // the fix denies ONLY the missing-container-right shape, not every non-owner create.
        let state = store_default_write_no_container_access().await;
        let got = observe(
            put_handler(
                State(state.clone()),
                Extension(carol()),
                "/alice/c/newdoc".parse().unwrap(),
                turtle_headers(),
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 201,
            "an agent WITH container Append (+ member Write) must be allowed to create: {got:?}"
        );
        assert!(
            state
                .store
                .exists("https://pod.example/alice/c/newdoc")
                .await
                .unwrap(),
            "the allowed create must have written the resource"
        );
    }

    // =====================================================================================
    // N3-PATCH `solid:where` READ-GATE (PR #3 review finding [MEDIUM]) — a patch carrying a where clause
    // READS the target graph (the BGP solver), so its 2xx-vs-409 outcome is a content/existence oracle.
    // An Append-without-Read agent must NOT be able to use a where clause to probe triple presence.
    // =====================================================================================

    /// A where-bearing INSERT-only patch (conditions non-empty, NO deletes ⇒ required mode is Append):
    /// it binds `?n` from an existing `<subject> foaf:name ?n` triple in the target and inserts a nick.
    /// Its outcome depends on whether that triple is PRESENT (one solution ⇒ apply) or ABSENT (zero
    /// solutions ⇒ 409) — the exact existence/content oracle the read-gate closes.
    fn where_insert_patch(subject: &str) -> AxBytes {
        AxBytes::from(format!(
            "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
             @prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
             _:p solid:where   {{ <{subject}> foaf:name ?n . }} ;\n\
                 solid:inserts {{ <{subject}> foaf:nick ?n . }} .\n",
        ))
    }

    /// `/alice/log` exists holding `body`; its own `.acl` grants ALICE full control and BOB ONLY
    /// `acl:Append` (accessTo) — the Append-without-Read shape. (An own `.acl` fixes Bob's effective
    /// modes on `/alice/log` to exactly `{Append}` regardless of inheritance.)
    async fn store_bob_append_only_log(
        body: &str,
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(
                "https://pod.example/alice/log",
                AxBytes::from(body.to_string()),
                "text/turtle",
            )
            .await
            .expect("seed log");
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/log>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <https://pod.example/alice/log>; acl:mode acl:Append."#
        );
        store
            .write(
                "https://pod.example/alice/log.acl",
                AxBytes::from(acl),
                "text/turtle",
            )
            .await
            .expect("seed log acl");
        Arc::new(LdpState::new(store, "https://pod.example"))
    }

    #[tokio::test]
    async fn where_patch_by_append_only_agent_is_denied_not_a_content_probe() {
        // Bob holds `acl:Append` on `/alice/log` but NOT `acl:Read`. A where-bearing patch would run the
        // BGP solver over the log's triples and leak (via 2xx-vs-409) whether the probed triple exists.
        // The read-gate folds it to Bob's denial (403) BEFORE any target read.
        let state = store_bob_append_only_log(
            "<https://pod.example/alice/log#me> <http://xmlns.com/foaf/0.1/name> \"L\" .",
        )
        .await;
        let got = observe(
            patch_handler(
                State(state),
                Extension(bob()),
                "/alice/log".parse().unwrap(),
                n3_patch_headers(),
                where_insert_patch("https://pod.example/alice/log#me"),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "an Append-without-Read where-patch must be the denial code, not a 2xx/409 probe: {got:?}"
        );
    }

    #[tokio::test]
    async fn where_patch_is_not_an_existence_oracle_present_vs_absent_byte_identical() {
        // The oracle is CLOSED: the same where-patch by the same Append-only Bob is BYTE-IDENTICAL
        // whether the probed triple is PRESENT (would-be 2xx) or ABSENT (would-be 409) — both fold to
        // the SAME 403 before the solver ever runs, so Bob learns nothing about the triple's presence.
        let subject = "https://pod.example/alice/log#me";
        let present = store_bob_append_only_log(
            "<https://pod.example/alice/log#me> <http://xmlns.com/foaf/0.1/name> \"L\" .",
        )
        .await;
        let absent = store_bob_append_only_log(
            "<https://pod.example/alice/log#me> <http://xmlns.com/foaf/0.1/note> \"other\" .",
        )
        .await;
        let on_present = observe(
            patch_handler(
                State(present),
                Extension(bob()),
                "/alice/log".parse().unwrap(),
                n3_patch_headers(),
                where_insert_patch(subject),
            )
            .await,
        )
        .await;
        let on_absent = observe(
            patch_handler(
                State(absent),
                Extension(bob()),
                "/alice/log".parse().unwrap(),
                n3_patch_headers(),
                where_insert_patch(subject),
            )
            .await,
        )
        .await;
        assert_eq!(on_present.status, 403);
        assert_eq!(
            on_present, on_absent,
            "the where-patch response must not depend on whether the probed triple exists (no oracle)"
        );
    }

    #[tokio::test]
    async fn plain_append_patch_by_the_same_agent_still_succeeds_proving_gate_is_the_cause() {
        // CONTROL (non-vacuous): the SAME Append-only Bob, with a WHERE-LESS insert patch, DOES succeed
        // (204 modify of the existing resource) — proving the 403 above is specifically the where-clause
        // read-gate, not a general denial of Bob's Append.
        let state = store_bob_append_only_log(
            "<https://pod.example/alice/log#me> <http://xmlns.com/foaf/0.1/name> \"L\" .",
        )
        .await;
        let got = observe(
            patch_handler(
                State(state),
                Extension(bob()),
                "/alice/log".parse().unwrap(),
                n3_patch_headers(),
                insert_only_patch("https://pod.example/alice/log#me"),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 204,
            "a WHERE-LESS append patch by the same Append holder must still succeed: {got:?}"
        );
    }

    // =====================================================================================
    // V4 `*`-FORM CONDITIONAL EXEMPTION (PR #3 review finding [MEDIUM]) — a bare `If-None-Match: *` /
    // `If-Match: *` carries no content-derived ETag (existence-only), so a Write-without-Read holder
    // keeps the spec-recommended safe-create / lost-update guards. Only a QUOTED validator gates on Read.
    // =====================================================================================

    /// `/alice/c/` + an existing member `/alice/c/existing`; the container `.acl` grants BOB `acl:default
    /// acl:Write` (inheritable member Write, NO Read) PLUS `acl:accessTo acl:Append` on the container
    /// (so the create's container-modification check passes) — the Write-without-Read shape that a
    /// standard `PUT … If-None-Match: *` client would hit.
    async fn store_bob_default_write_no_read_container(
    ) -> Arc<LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>>> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(
                "https://pod.example/alice/c/",
                AxBytes::from(String::new()),
                "text/turtle",
            )
            .await
            .expect("seed container");
        store
            .write(
                "https://pod.example/alice/c/existing",
                AxBytes::from(
                    "<https://pod.example/alice/c/existing#me> <http://p> <http://o> .".to_string(),
                ),
                "text/turtle",
            )
            .await
            .expect("seed existing member");
        let acl = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#alice> a acl:Authorization; acl:agent <{ALICE}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Read, acl:Write, acl:Control.
<#bob> a acl:Authorization; acl:agent <{BOB}>; acl:accessTo <https://pod.example/alice/c/>; acl:default <https://pod.example/alice/c/>; acl:mode acl:Write, acl:Append."#
        );
        store
            .write(
                "https://pod.example/alice/c/.acl",
                AxBytes::from(acl),
                "text/turtle",
            )
            .await
            .expect("seed container acl");
        Arc::new(LdpState::new(store, "https://pod.example"))
    }

    #[tokio::test]
    async fn v4_star_if_none_match_safe_create_allowed_for_write_without_read() {
        // A Write-without-Read Bob doing the standard `PUT … If-None-Match: *` safe-create of a FREE name
        // must SUCCEED (201) — bare `*` reveals only existence (which the write's 201-vs-204 split already
        // reveals to an authorized writer), so it is exempt from the V4 Read-gate. Pre-fix this was 403.
        let state = store_bob_default_write_no_read_container().await;
        let mut headers = turtle_headers();
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
        let got = observe(
            put_handler(
                State(state),
                Extension(bob()),
                "/alice/c/fresh".parse().unwrap(),
                headers,
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 201,
            "a Write-without-Read `If-None-Match: *` safe-create must be allowed (bare * is exempt): {got:?}"
        );
    }

    #[tokio::test]
    async fn v4_star_if_match_lost_update_allowed_for_write_without_read() {
        // The `If-Match: *` lost-update guard (overwrite only if it EXISTS) is likewise existence-only —
        // a Write-without-Read Bob overwriting the existing member with `If-Match: *` must reach the write
        // (204), not a V4 denial.
        let state = store_bob_default_write_no_read_container().await;
        let mut headers = turtle_headers();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("*"));
        let got = observe(
            put_handler(
                State(state),
                Extension(bob()),
                "/alice/c/existing".parse().unwrap(),
                headers,
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 204,
            "a Write-without-Read `If-Match: *` overwrite must be allowed (bare * is exempt): {got:?}"
        );
    }

    #[tokio::test]
    async fn v4_concrete_if_none_match_still_denied_for_write_without_read() {
        // The exemption is ONLY for bare `*`: a QUOTED validator DOES fingerprint content, so a
        // Write-without-Read `If-None-Match: "etag"` still folds to the denial (403). This pins that
        // finding-3's fix did not open the concrete-validator channel V4 exists to close.
        let state = store_bob_default_write_no_read_container().await;
        let mut headers = turtle_headers();
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"abc\""));
        let got = observe(
            put_handler(
                State(state),
                Extension(bob()),
                "/alice/c/fresh2".parse().unwrap(),
                headers,
                request_body_bytes(),
            )
            .await,
        )
        .await;
        assert_eq!(
            got.status, 403,
            "a CONCRETE-ETag conditional by a Write-without-Read holder must still be gated on Read: {got:?}"
        );
    }

    #[tokio::test]
    async fn get_unacceptable_accept_is_406_even_with_matching_if_none_match() {
        // With no selectable representation there is nothing a conditional can apply to: the 406
        // wins over a matching If-None-Match (no 304 for an unproducible representation). Since
        // the Solid-default fallback landed, the only unacceptable Accept is an EXPLICIT q=0
        // refusal of every producible type (a merely-unknown type now degrades to Turtle).
        let state = state_with_owner_resource("/alice/doc", COND_DOC).await;
        let turtle_tag =
            etag_of(&get_with(&state, owner_token(), "/alice/doc", HeaderMap::new()).await);
        let mut cond = HeaderMap::new();
        cond.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&turtle_tag).unwrap(),
        );
        cond.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/turtle;q=0, application/ld+json;q=0"),
        );
        let uri: axum::http::Uri = "/alice/doc".parse().unwrap();
        let err = get_handler(State(state), Extension(owner_token()), uri, cond)
            .await
            .expect_err("an unacceptable Accept must be a 406, not a 304");
        assert_eq!(err.status(), StatusCode::NOT_ACCEPTABLE);
    }
}
