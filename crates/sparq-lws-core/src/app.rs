// AUTHORED-BY Claude Opus 4.8
//! Application assembly — wires the auth middleware over the LDP routes.
//!
//! [`build_router`] is generic over the verifier seams and the store seam so the same wiring serves
//! both the M1 in-memory test stack and (M2) the network-backed production stack. The auth layer
//! runs OUTERMOST on the protected routes: a request is authenticated (injecting a
//! [`VerifiedToken`](crate::auth::VerifiedToken)) before it reaches an LDP handler.
//!
//! M2 adds the tower-http middleware stack (CORS, security headers, request-id, trace, body-limit,
//! timeout, rate-limit, load-shed — spike §4) around this, plus the discovery + notification routes.
//!
//! ## Overload protection (admission control + timeout) — the layer ORDER is security-critical
//! [`build_router_with_overload`] wraps the application routes with two overload layers (the
//! [`crate::overload`] backpressure layer):
//! - the **admission-control** middleware ([`crate::overload::admission_middleware`]) is the
//!   **OUTERMOST** layer — it sheds excess load (503 + jittered `Retry-After`) BEFORE auth/WAC/storage
//!   ever run, so a shed request can never bypass authorization (it gets strictly LESS than it would
//!   otherwise — a 503), and the expensive DPoP crypto is never spent on a request about to be
//!   rejected; and
//! - a **request timeout** layer (504 on a stuck request) just inside it.
//!
//! The **health/readiness routes are mounted OUTSIDE these layers** (their own router, merged last)
//! so a load balancer's readiness probe is NEVER shed or timed out — shedding a healthy instance's
//! probe would make the LB pull it, amplifying an overload into an outage.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use solid_oidc_verifier::config::JwksProvider;
use solid_oidc_verifier::replay::ReplayStore;
use tower_http::timeout::TimeoutLayer;

use crate::auth::{auth_middleware, AuthContext};
use crate::identity::{identity_gate_middleware, IdentityConfig, IdentityGate};
use crate::ldp::cors::cors_middleware;
use crate::ldp::handler::{
    delete_handler, get_handler, head_handler, options_handler, patch_handler, post_handler,
    put_handler, LdpState,
};
use crate::notifications::ws::{
    receive_handler, storage_description_handler, subscribe_handler, NotifyState, RECEIVE_PATH,
    SUBSCRIPTION_PATH, WELL_KNOWN_SOLID_PATH,
};
use crate::overload::{admission_middleware, AdmissionControl};
use crate::pop::sk::handlers::{
    establish_handler, protected_resource_metadata_json, terminate_handler, SkRouteState,
};
use crate::pop::sk::{OAUTH_PROTECTED_RESOURCE_PATH, SESSION_ENDPOINT_PATH};
use crate::rate_limit::{rate_limit_middleware, RateLimiter};
use crate::store::Store;

/// Path of the liveness probe (process is up). Exempt from admission control + timeout.
pub const LIVEZ_PATH: &str = "/livez";
/// Path of the readiness probe (process is ready to serve). Exempt from admission control + timeout.
pub const READYZ_PATH: &str = "/readyz";

/// Overload-protection configuration for [`build_router_with_overload`]: the admission-control state
/// (the concurrency ceiling + metrics) and the optional per-request timeout. `None` timeout disables
/// the timeout layer.
#[derive(Clone)]
pub struct OverloadConfig {
    /// The admission-control state (concurrency ceiling + in-flight/shed metrics).
    pub admission: AdmissionControl,
    /// The per-request timeout (504 on expiry). `None` ⇒ no timeout layer.
    pub request_timeout: Option<Duration>,
    /// The pre-crypto per-IP rate limiter (429 before auth/crypto on a per-source flood). `None` ⇒ no
    /// rate-limit layer (the `off` sentinel). When present it is the OUTERMOST application layer — see
    /// [`build_router_with_overload`].
    pub rate_limiter: Option<RateLimiter>,
    /// The maximum request-body size in bytes (a body over this ⇒ 413). An explicit, audited,
    /// configurable ceiling on per-request body buffering — see [`crate::body_limit`]. Always finite
    /// (there is no unlimited mode); defaults to [`crate::body_limit::DEFAULT_MAX_BODY_BYTES`].
    pub body_limit_bytes: usize,
}

impl OverloadConfig {
    /// A config with admission control sized to `max_concurrency` and the given timeout, NO rate
    /// limiter, and the DEFAULT body-size limit (back-compat for callers/tests that don't exercise the
    /// rate-limit or body-limit layers).
    pub fn new(max_concurrency: usize, request_timeout: Option<Duration>) -> Self {
        Self {
            admission: AdmissionControl::new(max_concurrency),
            request_timeout,
            rate_limiter: None,
            body_limit_bytes: crate::body_limit::DEFAULT_MAX_BODY_BYTES,
        }
    }
}

/// The assembled application state — the auth context + the LDP state, each behind an [`Arc`], plus
/// the optional identity-host config (provider WebIDs outside the pod —
/// `research/lws-design-records.md` §4).
pub struct AppState<J: JwksProvider, R: ReplayStore, S: Store> {
    pub auth: Arc<AuthContext<J, R>>,
    pub ldp: Arc<LdpState<S>>,
    /// `Some` ⇒ the identity gate SERVES id-docs on the configured host. `None` (the default)
    /// keeps serving off — but the gate's unconditional refusal of the reserved `/.identity/**`
    /// namespace is mounted REGARDLESS (flag-independent, per the design's security property).
    pub identity: Option<IdentityConfig>,
}

impl<J, R, S> AppState<J, R, S>
where
    J: JwksProvider,
    R: ReplayStore,
    S: Store,
{
    pub fn new(auth: AuthContext<J, R>, mut ldp: LdpState<S>) -> Self {
        // Single-source the anonymous-401 challenge: derive it from the verifier (it names the trusted
        // issuer(s) + DPoP algs) and hand it to the LDP layer, which has no verifier handle of its own.
        ldp.set_www_authenticate(auth.unauthenticated_challenge());
        Self {
            auth: Arc::new(auth),
            ldp: Arc::new(ldp),
            identity: None,
        }
    }

    /// Enable id-host serving (provider WebIDs outside the pod). Router-assembly-time only.
    pub fn with_identity(mut self, config: IdentityConfig) -> Self {
        self.identity = Some(config);
        self
    }
}

/// Build the axum router: the LDP routes (GET/HEAD/PUT/POST/DELETE/PATCH) + the WebSocketChannel2023
/// notification routes, wrapped by the DPoP auth middleware. A wildcard path captures the resource
/// target; the handler re-parses it against the base URL.
///
/// ## Route precedence (load-bearing)
/// The notification routes use STATIC paths (`/.notifications/…`, `/.well-known/solid`), which axum
/// matches BEFORE the LDP `/{*path}` wildcard — so they intercept correctly without the wildcard
/// shadowing them. They are registered as their own sub-routers carrying [`NotifyState`].
///
/// ## Auth split on the notification surface
/// - `POST /.notifications/WebSocketChannel2023/` is AUTH-GATED (same DPoP middleware as the LDP
///   routes) so it sees a `VerifiedToken` and can fail-closed on an anonymous caller. It then runs
///   the per-resource WAC check on the topic itself (see `notifications::ws`).
/// - `GET …/receive` (the WS upgrade) and `GET /.well-known/solid` (discovery) are PUBLIC as ROUTES:
///   a browser WebSocket cannot carry the DPoP header, and discovery is public like a storage
///   description. `…/receive` is nonetheless gated by the receive token AND — for the WebID that
///   token is bound to — the same per-resource WAC check, both documented in `notifications::ws`.
///
/// This is the no-overload-layer build (the existing default, used by the unit/integration tests). The
/// binary uses [`build_router_with_overload`] to add admission control + a request timeout. The two
/// share the route assembly via a private `build_app_routes` helper; this fn just merges those routes
/// + the (always overload-exempt) health routes with no extra layers.
pub fn build_router<J, R, S>(state: AppState<J, R, S>) -> Router
where
    J: JwksProvider + Send + Sync + 'static,
    R: ReplayStore + Send + Sync + 'static,
    S: Store + 'static,
{
    // Explicit, audited request-body ceiling on the app routes (a body over the DEFAULT limit ⇒ 413),
    // even on this no-overload build — so the default 2 MiB bound is OWNED by this crate rather than
    // relying on axum's implicit default (which a dependency bump could silently change). The binary's
    // [`build_router_with_overload`] applies the CONFIGURABLE value instead. Health routes are merged
    // OUTSIDE the layer (they carry no body).
    build_app_routes(state)
        .layer(crate::body_limit::layer(
            crate::body_limit::DEFAULT_MAX_BODY_BYTES,
        ))
        .merge(health_routes())
}

/// Build the router WITH overload protection (the binary's path): admission control (load shedding)
/// as the OUTERMOST layer + an optional request timeout just inside it, wrapping the application
/// routes — but NOT the health routes, which are merged OUTSIDE the layers so a load balancer's
/// readiness probe is never shed/timed-out. See the module's "Overload protection" note for why the
/// layer order is security-critical (a shed request 503s before auth/WAC/storage — never a bypass).
pub fn build_router_with_overload<J, R, S>(
    state: AppState<J, R, S>,
    overload: OverloadConfig,
) -> Router
where
    J: JwksProvider + Send + Sync + 'static,
    R: ReplayStore + Send + Sync + 'static,
    S: Store + 'static,
{
    let mut app = build_app_routes(state);

    // INNERMOST (app routes): the explicit, configurable request-body ceiling (a body over the limit ⇒
    // 413). It only sets the `DefaultBodyLimit` request extension the body extractor reads, so its
    // position among the app-route layers is immaterial to correctness — applied here so the body bound
    // is unmissably part of the app-route stack. See [`crate::body_limit`]; the resident-memory ceiling
    // for body buffering is `max_concurrency × body_limit_bytes`.
    app = app.layer(crate::body_limit::layer(overload.body_limit_bytes));

    // INNER: the request timeout (504 on a stuck request) — applied first so it is INSIDE admission
    // control (a timed-out request still holds its admission permit until it times out; that is
    // correct — the permit models a genuinely in-flight request).
    if let Some(timeout) = overload.request_timeout {
        // tower-http's TimeoutLayer returns a 408 by default; we want 503-family semantics for a
        // server-side stuck request, so use 504 GATEWAY_TIMEOUT (the request did not complete in time).
        app = app.layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            timeout,
        ));
    }

    // ADMISSION: admission control. Sheds (503 + jittered Retry-After) before auth/WAC/storage when at
    // capacity. Applied here so it is OUTSIDE auth but INSIDE the rate limiter (below). Security-
    // critical that this is outside the inner stack (see module docs): a shed request never reaches it,
    // so it can never bypass authorization.
    let mut app = app.layer(axum::middleware::from_fn_with_state(
        overload.admission,
        admission_middleware,
    ));

    // OUTERMOST: the pre-crypto per-IP rate limiter. Applied LAST, so (axum applies layers bottom-up)
    // it is the OUTERMOST application layer — it runs FIRST on every request, BEFORE admission control,
    // auth, WAC, and the expensive DPoP crypto. A per-source flood gets a cheap 429 and NEVER reaches
    // the verifier, so attacker traffic cannot make every bogus proof pay the ES256 verify cost.
    //
    // 🔒 Security: this layer ONLY rejects earlier. A 429 is strictly LESS access than auth would
    // grant, so it can never be a bypass; the limiter has zero authority to ADMIT a request (a
    // limiter bug/missing-ConnectInfo FAILS OPEN to the normal auth stack, which still gates it — see
    // `rate_limit`). It wraps the APP routes only — health routes are added OUTSIDE it (below), and
    // the middleware also skips /livez + /readyz by path as defence-in-depth.
    if let Some(rate_limiter) = overload.rate_limiter {
        app = app.layer(axum::middleware::from_fn_with_state(
            rate_limiter,
            rate_limit_middleware,
        ));
    }

    // Health routes are OUTSIDE the overload + rate-limit layers (merged last) — never shed, timed-out,
    // or rate-limited.
    app.merge(health_routes())
}

/// The application routes (LDP + notifications), WITHOUT the overload layers or the health routes —
/// the shared core of [`build_router`] and [`build_router_with_overload`].
fn build_app_routes<J, R, S>(state: AppState<J, R, S>) -> Router
where
    J: JwksProvider + Send + Sync + 'static,
    R: ReplayStore + Send + Sync + 'static,
    S: Store + 'static,
{
    let AppState {
        auth,
        ldp,
        identity,
    } = state;

    // A handle for the identity gate (mounted last, below) — taken before `ldp` is moved into the
    // protected routes' state.
    let gate_ldp = ldp.clone();

    // The notification state is derived from the LDP state, so it shares the hub (a subscriber
    // registered via `…/receive` is on the same registry the LDP emit path fans to), the base URL,
    // AND the store + parsed-ACL cache — the last of which is what lets the subscribe/receive seams
    // run the SAME per-resource WAC check as a GET of the topic.
    let notify_state = Arc::new(NotifyState::new(ldp.clone()));

    // The full LDP method set, shared by the wildcard `/{*path}` route AND the explicit `/` (root)
    // route. The `/{*path}` wildcard does NOT match the empty path, so the storage root needs its own
    // route with the same handlers (Cluster-A #1) — otherwise `GET /` is a 404.
    let ldp_methods = || {
        get(get_handler::<S>)
            .head(head_handler::<S>)
            .put(put_handler::<S>)
            .post(post_handler::<S>)
            .delete(delete_handler::<S>)
            .patch(patch_handler::<S>)
            // OPTIONS advertises Allow / Accept-Post / Accept-Patch (and rides the CORS preflight).
            .options(options_handler::<S>)
    };

    // The protected LDP routes carry the LDP state.
    //
    // Layer order (axum/tower applies `.layer()` bottom-up, so the LAST one is OUTERMOST = runs
    // first): the CORS layer is OUTERMOST. That means (a) a CORS preflight OPTIONS is answered by the
    // CORS layer BEFORE auth runs (a browser preflight carries no credentials), and (b) the
    // `Access-Control-*` headers ride on EVERY response — the auth 401, the anonymous-read 401, and
    // the success — because they are added on the way back OUT through the outermost layer.
    let protected = Router::new()
        .route("/", ldp_methods())
        .route("/{*path}", ldp_methods())
        // INNERMOST: the auth middleware authenticates a real (non-preflight) request and injects the
        // VerifiedToken into request extensions.
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            auth_middleware::<J, R>,
        ))
        // PRE-CRYPTO PUBLIC-READ SKIP (skip-crypto opt 3, `research/lws-design-records.md` §5): a
        // cheap, identity- independent fast-path BEFORE crypto, in the SAME slot as the rate-limit
        // / overload layers — just INSIDE CORS, just OUTSIDE auth. For a GET/HEAD that carries NO
        // `Authorization`/`DPoP` header it constructs a public token and delegates STRAIGHT to the
        // same `serve_read` the handler uses (one anonymous WAC pass): a PUBLIC read → 200, an
        // anonymous denial → the same 401 + challenge, a malformed target → the canonical 400 — all
        // byte-identical to the full anonymous path. A CREDENTIALED request (Authorization OR DPoP
        // header present), a mutation, or any other verb FALLS THROUGH (`next.run`) to the
        // unchanged auth path. It carries the LDP state (store + base + ACL cache) so the served
        // read uses the SAME `serve_read`. Security-critical (see `crate::ldp::public_read_skip`):
        // it NEVER handles a credentialed request (so an authenticated owner's WAC-Allow user=
        // modes are correct and a forged proof is rejected, not served), NEVER fires for a
        // mutation, and NEVER reads the unverified WebID.
        .layer(axum::middleware::from_fn_with_state(
            ldp.clone(),
            crate::ldp::public_read_skip::public_read_skip_middleware::<S>,
        ))
        // OUTERMOST: CORS (preflight short-circuit + the Access-Control-* headers on every response).
        .layer(axum::middleware::from_fn(cors_middleware));
    #[cfg(feature = "sparql-endpoint")]
    let protected = protected.with_state(ldp.clone());
    #[cfg(not(feature = "sparql-endpoint"))]
    let protected = protected.with_state(ldp);

    // The AUTH-GATED subscribe route: behind the SAME DPoP middleware so the handler sees a
    // VerifiedToken (fail-closed on anonymous).
    let subscribe = Router::new()
        .route(SUBSCRIPTION_PATH, post(subscribe_handler::<S>))
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            auth_middleware::<J, R>,
        ))
        .with_state(notify_state.clone());

    // The PUBLIC notification routes: the WS receive upgrade + the discovery document (no auth — see
    // the auth-split note above).
    let public_notify = Router::new()
        .route(RECEIVE_PATH, get(receive_handler::<S>))
        .route(WELL_KNOWN_SOLID_PATH, get(storage_description_handler::<S>))
        .with_state(notify_state);

    let mut router = Router::new().merge(subscribe).merge(public_notify);

    // [GPT-5.6] sq-r1ei8: the query surface is a distinct protected route so
    // the LDP public-read shortcut cannot reinterpret `/sparql` as a resource.
    // It still uses the identical authentication and CORS layers, and its
    // handler performs WAC independently for every assembled resource.
    #[cfg(feature = "sparql-endpoint")]
    {
        let sparql = Router::new()
            .route(
                "/sparql",
                get(crate::sparql_endpoint::get_handler::<S>)
                    .post(crate::sparql_endpoint::post_handler::<S>),
            )
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                auth_middleware::<J, R>,
            ))
            .layer(axum::middleware::from_fn(cors_middleware))
            .with_state(ldp.clone());
        router = router.merge(sparql);
    }

    // PoP Tier 2 (DPoP-SK, `SOLID_SERVER_DPOP_SK`) — the session establishment/termination
    // endpoint, mounted ONLY when the tier is enabled so a flag-off build's route table (and the
    // LDP wildcard's coverage of `/.pop/session`) is byte-identical to pre-Tier-2.
    //
    // POST (establishment) sits BEHIND the same DPoP auth middleware as the LDP routes, so the
    // handler receives a fully verified token — establishment is exactly as strong as any DPoP
    // request. DELETE (termination) is authenticated by its own DPoP-SK attestation, which the
    // SAME middleware verifies (the SK dispatch runs inside it); the handler then requires the
    // `SkSession` marker. The 401 challenge is single-sourced from the verifier as everywhere.
    if let Some(sk) = auth.sk() {
        let sk_routes = Router::new()
            .route(
                SESSION_ENDPOINT_PATH,
                post(establish_handler).delete(terminate_handler),
            )
            .layer(axum::middleware::from_fn_with_state(
                auth.clone(),
                auth_middleware::<J, R>,
            ))
            .with_state(SkRouteState {
                sk: sk.clone(),
                challenge: auth.unauthenticated_challenge(),
            });
        router = router.merge(sk_routes);
    }

    // RFC 9728 protected-resource metadata — the PoP negotiation/advertisement surface. Mounted
    // only when a PoP tier beyond baseline DPoP is enabled (mTLS-bound tokens and/or DPoP-SK), so
    // the default build's public surface is unchanged. The document is static per boot; built once.
    if auth.mtls_bound_tokens() || auth.sk().is_some() {
        let body = Arc::new(protected_resource_metadata_json(
            &auth.base_url,
            auth.mtls_bound_tokens(),
            auth.sk().map(Arc::as_ref),
        ));
        let metadata = Router::new().route(
            OAUTH_PROTECTED_RESOURCE_PATH,
            get(move || {
                let body = body.clone();
                async move {
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        (*body).clone(),
                    )
                }
            }),
        );
        router = router.merge(metadata);
    }

    // The IDENTITY GATE — the OUTERMOST application layer (applied LAST, so it runs FIRST on every
    // app-route request; only the overload/rate-limit layers in `build_router_with_overload` sit
    // outside it, and those only ever reject earlier). Two halves (see `crate::identity`):
    //
    // 1. UNCONDITIONAL: any request whose path targets the reserved `/.identity/**` namespace is
    //    404'd immediately — every method, every origin, every Host, BEFORE auth/WAC/storage,
    //    REGARDLESS of the identity flag. This is what makes "no `.acl` can ever exist for an
    //    id-doc ⇒ no WAC grant can ever apply to it" a construction-level property (the
    //    `parse_target` refusal is the belt-and-braces second chokepoint).
    // 2. CONFIG-GATED: when `identity` is `Some`, a request whose Host is EXACTLY the id host is
    //    answered entirely by the gate (GET/HEAD id-doc with conneg + ETag + public cache +
    //    `ACAO: *`; 405 for every other method; fail-closed 404 otherwise) and NEVER reaches the
    //    auth/LDP stack — no WAC, no `.acl` Link, no `WWW-Authenticate`, by design.
    //
    // The health routes (`/livez`, `/readyz`) are merged OUTSIDE this gate (in the callers), so a
    // probe is never intercepted; the probe names are in `identity::RESERVED_HANDLES` so the
    // shadowing is explicit.
    let gate = Arc::new(IdentityGate {
        serving: identity,
        ldp: gate_ldp,
    });
    router
        .merge(protected)
        .layer(axum::middleware::from_fn_with_state(
            gate,
            identity_gate_middleware::<S>,
        ))
}

/// The health/readiness routes: `GET /livez` (process up) + `GET /readyz` (ready to serve). Both are
/// cheap, public, and ALWAYS overload-EXEMPT (merged outside the admission/timeout layers), so a load
/// balancer's probe is never shed/timed-out — shedding a healthy instance's readiness probe would make
/// the LB pull a still-good node and amplify an overload into an outage.
///
/// `/livez` and `/readyz` return 200 + a tiny `text/plain` body. They are deliberately NOT auth-gated
/// (a probe carries no credentials) and expose no state. They are kept distinct so an operator can map
/// them to a k8s `livenessProbe` vs `readinessProbe`: today both are a static "the process is up";
/// `/readyz` is the seam to add a real backend-reachability check (SPARQ/S3) when the live store lands
/// — at which point a not-ready instance can 503 its `/readyz` to deregister cleanly behind the LB.
fn health_routes() -> Router {
    Router::new()
        .route(LIVEZ_PATH, get(|| async { (StatusCode::OK, "live\n") }))
        .route(READYZ_PATH, get(|| async { (StatusCode::OK, "ready\n") }))
}
