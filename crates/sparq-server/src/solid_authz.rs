//! (sq-snopa.6, issue #992 FR-4) OPT-IN Solid **WAC/ACP HTTP authorization** surface.
//!
//! The `POST /authz/decide`, `POST /authz/wac-allow` and `POST /authz/query` endpoints — a THIN
//! HTTP shell over the `sparq-solid` LIBRARY authoriser
//! ([`PodStore::decide`](sparq_solid::PodStore::decide) /
//! [`wac_allow`](sparq_solid::PodStore::wac_allow) /
//! [`query_json_as`](sparq_solid::PodStore::query_json_as)). Compiled ONLY behind the
//! `solid-authz` feature (the deliberately-opt-in `sparq-server` -> `sparq-solid` workspace
//! dependency the FR-4 architecture note flagged); served only when
//! [`ServerConfig::solid_authz`](crate::http::ServerConfig) is also set (`--solid-authz` /
//! `SPARQ_SOLID_AUTHZ=1`) — the same double-opt-in as `tpf` / `shacl` / `terse`.
//!
//! # The boundary (`research/sparq-solid-scope.md` §4)
//!
//! `sparq-solid` is a **library-level authoriser with no HTTP surface**: mapping a request PATH
//! to a named-graph IRI, and authenticating a WebID into a `(WebID, client, issuer)` session, are
//! the **server's** job, NOT the library's. This module is exactly that missing HTTP shell — it
//! does NOT authenticate. Each endpoint takes an **already-resolved** session plus the pod dataset
//! (N-Quads, including the `.acl`/`.acr` control graphs) in the request body, builds a
//! [`PodStore`](sparq_solid::PodStore), materialises the auth view, and maps the library verdict
//! onto HTTP.
//!
//! # Which pod: `"source"` (sq-snopa.8)
//!
//! Every endpoint takes an optional `"source"` field selecting WHICH pod it authorises over:
//!
//! * `"body"` (the DEFAULT, and the only mode before sq-snopa.8) — **stateless**: the pod dataset
//!   arrives in the request's `"dataset"` field, is materialised for that one request, and dies
//!   with the response.
//! * `"server"` — **stateful**: authorise over the **server's own loaded store**. The request must
//!   NOT carry a `"dataset"` (a body naming both pods is ambiguous, so it is refused). The pinned
//!   current generation is forked, materialised ONCE, and cached in
//!   [`AppState`](crate::http::AppState) keyed by serving lineage and generation, so the N3 materialisation
//!   is paid once per generation rather than once per request.
//!
//! Only an ABSENT `"source"` key takes the `"body"` default. A key that is present but is not one
//! of those two strings — an unknown keyword, or any non-string JSON type such as `null`/`false`/a
//! number — is a fail-closed `400`, never an inferred default: the field decides whether a decision
//! trusts the server's store or a caller-supplied dataset, so a malformed value must not silently
//! cross into the body lane.
//!
//! In the stateful lane a pod resource IS a named graph of the server's store, so `"resource"`
//! must be that graph's absolute IRI (the server's own path -> named-graph mapping is what the
//! Graph Store Protocol routes already establish); a relative path is not resolved.
//!
//! **Re-materialise on ACL change** falls out of the generation key rather than needing writer
//! bookkeeping: every commit — an `.acl`/`.acr` write included — publishes a new generation, which
//! makes the cached entry stale immediately, so the next request rebuilds. The cached view can
//! never be older than the generation the request pins, so a request never decides from an auth
//! view staler than the data it would read.
//!
//! The stateful lane is fail-closed on the same terms as the stateless one, and additionally
//! REFUSES (never silently un-enforces) two combinations it cannot evaluate faithfully: under
//! `odrl-authz`, a server store carrying ODRL policy rules (dropping a prohibition would be
//! fail-OPEN); under `solid-authz-trust`, a request carrying a `"trust"` block (that extension
//! decides over its own body-derived store).
//!
//! # Fail-closed (the soundness invariant)
//!
//! **Every error path DENIES**, never opens. An unparseable dataset, a materialisation failure, an
//! unknown mode or view keyword, a malformed body — each yields a fail-closed response
//! (`allow:false` on `/decide`, an empty permission advertisement on `/wac-allow`, an error on
//! `/query`), never a grant. This mirrors the library's own construction: a
//! [`WacDecision`](sparq_solid::WacDecision) is `allow == false` for every non-`Resolved` status,
//! and a grant-less / un-materialised session sees nothing. The tests exercise this directly (an
//! unparseable dataset -> deny with a `400`; an un-materialisable view still denies).
//!
//! # The ODRL lane (`odrl-authz` feature, sq-lrtc3.1 + sq-3mu76)
//!
//! Behind the OPT-IN `odrl-authz` cargo feature, ALL THREE endpoints (`/authz/query`,
//! `/authz/decide`, `/authz/wac-allow` — the latter two are sq-3mu76) grow an ODRL lane: when the
//! request dataset carries ODRL policy rules (`odrl:permission` / `odrl:prohibition` predicates),
//! the handler parses them from the UNION of the dataset's graphs (`sparq_policy::parse_policy`)
//! and runs the sparq-solid ODRL bridge (`PodStore::materialize_odrl_policy` — the Permit allow
//! side AND the matched-Prohibition deny side, deny-overrides) for the request's
//! (party = session agent, action = `odrl:read`, target = each rule's target graph) BEFORE
//! `query_json_as` / `decide` / `wac_allow` runs. The bridged grants/denies compose with the
//! WAC/ACP view through the UNCHANGED `∪ allow ∖ ∪ deny` enforcement — an ODRL prohibition beats
//! a static WAC grant, on the enforcement surface AND the advisory ones (pre-sq-3mu76, a dataset
//! carrying a prohibition could get a `/authz/decide` allow that `/authz/query` would refuse to
//! honour). Fail-closed refusals (never a silent allow): a malformed policy, an unimplementable
//! `odrl:conflict` strategy, a rule without a concrete target graph IRI (pattern targets are
//! sq-lrtc3.3), a non-read mode (the query-action contract is sq-lrtc3.2), or an anonymous
//! session each yield a 4xx. STATELESS-lane scope only: the lane reads the request-BODY dataset,
//! so it does not apply to `"source":"server"` — an ODRL-carrying server store is REFUSED there
//! (see the `"source"` section above), never authorised as if the rules were absent.
//!
//! ## Read-scoped advertisement (sq-3mu76)
//!
//! The lane evidences ONLY the `odrl:read` action until the sq-lrtc3.2 action contract lands, so
//! when it fires the ADVISORY mode advertisements are scoped down to what it evidenced:
//! `/authz/decide` masks `grantedModes` to the requested (read) mode, and `/authz/wac-allow`
//! reports at most `user="read"` with `public=""` (an anonymous session is REFUSED wherever the
//! lane fires, so public access is never advertised under an ODRL policy). Under-advertising is
//! the fail-closed direction — the `allow` bit / the advertised `read` are exactly what
//! `/authz/query` enforces; a WAC write grant is merely not repeated where the ODRL lane cannot
//! yet evaluate a write-action rule against it.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::Response,
};

use sparq_solid::{AclStatus, AuthAllowance, Mode, PodStore, Session, WacDecision};

use crate::http::{auth_gate, json_error, AppState, Operation};

/// Which auth model to materialise before deciding. Chosen by the request's `"view"` field, or
/// inferred from the dataset's control-document suffixes (`.acr` present -> ACP, else WAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthView {
    /// Web Access Control — materialise from `.acl` documents ([`PodStore::materialize_wac`]).
    Wac,
    /// Access Control Policy — materialise from `.acr` documents ([`PodStore::materialize_acp`]).
    Acp,
}

/// (sq-snopa.8) WHICH POD the endpoint authorises over — the stateless/stateful
/// switch, selected by the request's `"source"` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthzSource {
    /// STATELESS (the default, and the only mode before sq-snopa.8): the pod dataset arrives in
    /// the request body's `"dataset"` field and dies with the response.
    Body,
    /// STATEFUL: authorise over the SERVER'S OWN loaded store — the pinned current generation,
    /// materialised once per generation and cached in [`AppState`] (see
    /// [`CachedAuthView`]). The body must NOT carry a `"dataset"`.
    Server,
}

/// The BOUNDED blocking pool every `/authz/*` authorization-view build runs on.
///
/// Materialising an authorization view is CPU-bound and its cost is set by untrusted input —
/// the request's own dataset on the stateless lane, the store's control documents on the
/// stateful one — so it belongs on [`tokio::task::spawn_blocking`] rather than an async
/// worker. `spawn_blocking` alone bounds nothing, though: its queue is unbounded, so N
/// concurrent requests put N materialisations in flight and the queue behind them grows with
/// the load. This caps the builds IN FLIGHT and, just as importantly, caps the WAIT: a
/// request that finds no free slot waits at most [`AuthBuildPool::wait`] and is then refused
/// with the same retryable `503` a materialisation failure gives — shed, never queued
/// without limit.
///
/// One pool is shared by every lane that builds a view (the stateless per-request build, the
/// stateful cache misses, the ODRL store and policy builds under one claim, and the
/// trust lane's parse-through-decision pipeline), so the cap is on
/// authorization work in flight rather than per endpoint.
pub(crate) struct AuthBuildPool {
    /// One permit per concurrent build.
    permits: Arc<tokio::sync::Semaphore>,
    /// How long a request waits for a permit before the fail-closed `503`.
    wait: std::time::Duration,
    /// Builds admitted to the pool, and builds refused because it was saturated. Counters
    /// for the tests and for an operator reading them through a decision's absence; they
    /// carry no request data.
    admitted: std::sync::atomic::AtomicU64,
    refused: std::sync::atomic::AtomicU64,
    /// Serializes the warning timestamp and the count reported by its previous line.
    shed_log: std::sync::Mutex<(Option<std::time::Instant>, u64)>,
    #[cfg(test)]
    build_hook: std::sync::Mutex<Option<Arc<BuildHook>>>,
    #[cfg(test)]
    view_hook: std::sync::Mutex<Option<Arc<BuildHook>>>,
    #[cfg(all(test, feature = "solid-authz-trust"))]
    trust_verification_calls: std::sync::atomic::AtomicUsize,
}

/// A per-pool test hook: signal entry, then block the actual blocking build on a barrier.
#[cfg(test)]
#[derive(Default)]
struct BuildHook {
    entered: tokio::sync::Notify,
    running: std::sync::atomic::AtomicUsize,
    release: (std::sync::Mutex<bool>, std::sync::Condvar),
}

#[cfg(test)]
impl BuildHook {
    fn block(&self) {
        self.running
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.entered.notify_one();
        let (lock, cv) = &self.release;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = cv.wait(released).unwrap();
        }
    }

    fn release(&self) {
        let (lock, cv) = &self.release;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
}

impl AuthBuildPool {
    /// A pool admitting `concurrency` builds, clamped to the semaphore's supported range.
    /// Each request waits at most `wait` for a slot.
    pub(crate) fn new(concurrency: usize, wait: std::time::Duration) -> AuthBuildPool {
        AuthBuildPool {
            permits: Arc::new(tokio::sync::Semaphore::new(
                concurrency.clamp(1, tokio::sync::Semaphore::MAX_PERMITS),
            )),
            wait,
            admitted: std::sync::atomic::AtomicU64::new(0),
            refused: std::sync::atomic::AtomicU64::new(0),
            shed_log: std::sync::Mutex::new((None, 0)),
            #[cfg(test)]
            build_hook: std::sync::Mutex::new(None),
            #[cfg(test)]
            view_hook: std::sync::Mutex::new(None),
            #[cfg(all(test, feature = "solid-authz-trust"))]
            trust_verification_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Builds this pool has admitted since it was created. Test observability: the 503 a
    /// saturated pool gives is indistinguishable from a materialisation's own, so a test
    /// reads this to say WHICH lane's work went through the bound.
    #[cfg(test)]
    pub(crate) fn admitted(&self) -> u64 {
        self.admitted.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Builds this pool has refused because every slot was taken for longer than
    /// [`AuthBuildPool::wait`] (also logged, as `refused_total`, on each refusal).
    #[cfg(test)]
    pub(crate) fn refused(&self) -> u64 {
        self.refused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Permits currently free — what a test asserts a finished build gave back.
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// Counts to log for this shed, at most once per second for the shared pool.
    fn shed_warning(
        &self,
        clock: impl Fn() -> std::time::Instant,
        emit: impl FnOnce(u64, u64),
    ) -> Option<(u64, u64)> {
        let mut logged = self.shed_log.lock().ok()?;
        let now = clock();
        if logged
            .0
            .is_some_and(|last| now.duration_since(last).as_secs() < 1)
        {
            return None;
        }
        let total = self.refused.load(std::sync::atomic::Ordering::Relaxed);
        let shed = total.saturating_sub(logged.1);
        emit(shed, total);
        // Keep the lock through emission and date the next window from its completion.
        *logged = (Some(clock()), total);
        Some((shed, total))
    }

    /// Claim a build slot, waiting at most [`AuthBuildPool::wait`] for one. `None` is a
    /// SATURATED pool: the caller fails closed rather than waiting further.
    ///
    /// A zero `wait` is "do not wait at all", which `tokio::time::timeout` honours (the
    /// future is polled once, so a free permit is still taken).
    async fn claim(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let permits = Arc::clone(&self.permits);
        match tokio::time::timeout(self.wait, permits.acquire_owned()).await {
            // The semaphore is never closed, so `acquire_owned` cannot error; treat an
            // error as saturation anyway rather than running the build unbounded.
            Ok(Ok(permit)) => {
                self.admitted
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Some(permit)
            }
            Ok(Err(_)) | Err(_) => {
                self.refused
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                None
            }
        }
    }
}

/// Run one authorization-view build `work` on the bounded blocking pool
/// ([`AuthBuildPool`]), or fail closed.
///
/// The permit is held for the whole blocking task and released when it ends — and on an
/// unwinding panic, with the closure's frame — so the cap really is on builds in flight. Two fail-closed `503`s come out of here and neither can become a
/// decision: a SATURATED pool (no slot inside the wait) and a JOIN failure (the task was
/// cancelled, or panicked under `panic = "unwind"`).
// `Response` is large; same `result_large_err` allow as the rest of this module.
#[allow(clippy::result_large_err)]
async fn on_build_pool<T, F>(state: &AppState, work: F) -> Result<T, Response>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let pool = Arc::clone(state.authz_build_pool());
    let Some(permit) = pool.claim().await else {
        // At most one warning per second, with every shed since the preceding line.
        let _ = pool.shed_warning(std::time::Instant::now, |shed, total| {
            tracing::warn!(
                target: "sparq_server::solid_authz",
                shed,
                refused_total = total,
                "authorization build pool saturated; requests refused"
            );
        });
        return Err(json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "the authorization build pool is saturated; retry",
        ));
    };
    let task = tokio::task::spawn_blocking(move || {
        // Held for the whole build, released when this frame ends.
        let _permit = permit;
        #[cfg(test)]
        {
            let hook = pool.build_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook.block();
            }
        }
        work()
    });
    task.await.map_err(|_| {
        json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "could not materialise the authorization view",
        )
    })
}

/// (sq-snopa.8) The cached STATEFUL authorization view: one materialised
/// [`PodStore`] plus the serving lineage, generation and auth model it was built for. Lives in
/// [`AppState`](crate::http::AppState) behind an `RwLock`; see that field's doc for why a
/// lineage/generation cache beside the ring provides the invalidation contract.
///
/// The `Arc` is what makes concurrent serving cheap: every request of a generation clones the
/// same materialised store handle, so the N3 materialisation is paid ONCE per generation rather
/// than once per request.
pub(crate) struct CachedAuthView {
    /// Retaining the core prevents identity reuse after an in-memory restore.
    #[cfg(feature = "backup")]
    core: Arc<crate::http::ServingCore>,
    /// The ring generation number this view is a pure function of.
    generation: u64,
    /// Which auth model was materialised (a request that asks for the other one rebuilds).
    view: AuthView,
    /// The snapshot's automatic selection, also needed when the cached view was explicit.
    inferred_view: AuthView,
    /// The materialised store, shared by every request pinned to `generation`.
    store: Arc<PodStore>,
}

/// (sq-snopa.8) Why the stateful view refused to build. A small `Send` enum rather
/// than a [`Response`] so it crosses the `spawn_blocking` boundary cleanly; mapped to the
/// fail-closed HTTP response by [`server_view_error_response`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerViewError {
    /// The N3 materialisation failed — an operational, retryable condition (503). Fail-closed:
    /// no store is cached and no request is answered from a partial view.
    ///
    /// Carries the materialisation's allowance/budget kind when it had one
    /// ([`sparq_solid::auth_allowance_refusal`]) so the server can LOG which limit the store's
    /// control documents reached — "raise the allowance" and "the reasoner failed" are different
    /// operational conditions. `None` for any other materialisation failure. The kind is a
    /// closed enum carrying no pod or request data, which is why it is what crosses into the
    /// log; the message itself (which quotes document IRIs) is not logged and never reaches the
    /// client.
    Materialize(Option<AuthAllowance>),
    /// `odrl-authz` only: the server's own store carries ODRL policy rules. The stateful lane
    /// cannot yet evaluate them, and SILENTLY DROPPING a prohibition would be fail-OPEN — so the
    /// whole request is refused (400) instead.
    #[cfg(feature = "odrl-authz")]
    Odrl,
}

/// (sq-snopa.8) The store an endpoint decides over: an owned per-request store
/// (stateless lane) or the shared per-generation one (stateful lane). Both are read through
/// [`get`](Self::get); only the owned variant can be mutated by the ODRL lane.
///
/// Both variants hold the store BEHIND A POINTER so neither arm pays for the other's payload:
/// a bare `PodStore` inline would make every `ResolvedStore` — including the shared one, whose
/// own payload is one `Arc` word — as large as a whole materialised store
/// (`clippy::large_enum_variant`). The stateless lane's extra allocation is noise beside the
/// N-Quads parse + materialisation it already paid to build that store.
enum ResolvedStore {
    /// Stateless: built from the request body's dataset, dies with the response.
    Owned(Box<PodStore>),
    /// Stateful: the lineage/generation view shared by requests of the same snapshot.
    Shared(Arc<PodStore>),
}

impl ResolvedStore {
    /// The store to decide against, whichever lane produced it.
    fn get(&self) -> &PodStore {
        match self {
            ResolvedStore::Owned(s) => s,
            ResolvedStore::Shared(s) => s,
        }
    }
}

/// A parsed `/authz/*` request envelope. The `session` fields are borrowed from the owned JSON
/// value the caller keeps alive; `dataset` is the owned N-Quads text.
///
/// `Clone` so a lane can move the envelope into a blocking-pool task rather than borrow it
/// across the await ([`on_build_pool`]); every field is owned, and the dataset it copies is
/// bounded by the request body limit the parse already applied.
#[derive(Debug, Clone)]
struct AuthzRequest {
    /// (sq-snopa.8) Which pod to authorise over — the body's dataset (default) or
    /// the server's own loaded store.
    source: AuthzSource,
    /// The pod dataset as N-Quads (documents + `.acl`/`.acr` control graphs), owned. EMPTY (and
    /// never read) under [`AuthzSource::Server`], which forbids the field outright.
    dataset: String,
    #[cfg(feature = "odrl-authz")]
    carries_odrl: bool,
    /// The already-resolved principal — agent (WebID) / client / issuer, all optional; anonymous
    /// when all are absent (fail-closed — an anonymous session sees only public grants).
    agent: Option<String>,
    client: Option<String>,
    issuer: Option<String>,
    now: Option<String>,
    /// The chosen auth model, if the request named one explicitly (`"view": "wac" | "acp"`).
    /// `None` -> infer from the dataset.
    view: Option<AuthView>,
}

impl AuthzRequest {
    /// Borrow the parsed fields as a [`Session`] (the library's borrowed principal type).
    fn session(&self) -> Session<'_> {
        Session {
            agent: self.agent.as_deref(),
            client: self.client.as_deref(),
            issuer: self.issuer.as_deref(),
            now: self.now.as_deref(),
        }
    }
}

/// Map an [`AclStatus`] to the HTTP status a resource server should return for a DENY
/// (`research/sparq-solid-scope.md` §4 / FR-6): a definitive permission outcome
/// (`Resolved` deny / `NoAcl`) is a **403**; a retryable operational condition (`Unloaded` /
/// `Transient`) is a **503**. An *allow* is always a `200` (the caller reads `allow`), so this is
/// the deny-side code only. Pure + directly unit-tested.
pub(crate) fn deny_status_code(status: AclStatus) -> StatusCode {
    if status.is_retryable() {
        // Unloaded / Transient: the decision could not be computed *this time* — retryable.
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        // Resolved-without-the-mode / NoAcl: a definitive, authoritative deny.
        StatusCode::FORBIDDEN
    }
}

/// The lower-case name of a [`Mode`], for the JSON body. Pure + directly unit-tested.
pub(crate) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Read => "read",
        Mode::Write => "write",
        Mode::Append => "append",
        Mode::Control => "control",
    }
}

/// Parse a WAC/ACP mode name (`read` / `write` / `append` / `control`, case-insensitive) into a
/// [`Mode`]. `None` for any other string — the caller treats that as a fail-closed deny (an
/// unknown mode can never be granted). Pure + directly unit-tested.
pub(crate) fn parse_mode(s: &str) -> Option<Mode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "read" => Some(Mode::Read),
        "write" => Some(Mode::Write),
        "append" => Some(Mode::Append),
        "control" => Some(Mode::Control),
        _ => None,
    }
}

/// The lower-case status token for the JSON body (mirrors [`AclStatus`]). Pure + directly
/// unit-tested.
pub(crate) fn status_token(status: AclStatus) -> &'static str {
    match status {
        AclStatus::Resolved => "resolved",
        AclStatus::NoAcl => "noAcl",
        AclStatus::Unloaded => "unloaded",
        AclStatus::Transient => "transient",
    }
}

/// Serialise a [`WacDecision`] to the JSON body the `/authz/decide` endpoint returns:
/// `{ "allow": bool, "grantedModes": [..], "governingAcl": iri|null, "scope": "accessTo"|"default"
/// |null, "status": "resolved"|"noAcl"|"unloaded"|"transient", "aclLink": link-value|null }`.
///
/// `aclLink` is the RFC-8288 `Link: rel="acl"` header VALUE
/// ([`WacDecision::acl_link_header`](sparq_solid::WacDecision::acl_link_header)) already present in
/// the body so a follow-up (sq-snopa.7) only needs to lift it into a response header. Pure +
/// directly unit-tested (against a real decision).
pub(crate) fn decision_to_json(d: &WacDecision) -> String {
    let modes: Vec<serde_json::Value> = d
        .granted_modes
        .iter()
        .map(|m| serde_json::Value::from(mode_name(*m)))
        .collect();
    let scope = d
        .scope
        .map(|s| serde_json::Value::from(s.as_acl_predicate()));
    let governing = d
        .governing_acl
        .as_ref()
        .map(|n| serde_json::Value::from(n.as_str()));
    let acl_link = d.acl_link_header().map(serde_json::Value::from);
    let body = serde_json::json!({
        "allow": d.allow,
        "grantedModes": modes,
        "governingAcl": governing,
        "scope": scope,
        "status": status_token(d.status),
        "aclLink": acl_link,
    });
    // `to_string` on a serde_json::Value never fails.
    body.to_string()
}

/// A `text/json` response with the given status + body. The body is already-serialised JSON.
fn json_response(status: StatusCode, body: String) -> Response {
    json_response_with_link(status, body, None)
}

/// A `text/json` response with the given status + body, and an OPTIONAL `Link: rel="acl"`
/// header (FR-5, sq-snopa.7). When `acl_link` is `Some(value)` the RFC 8288 link-value
/// (`<iri>; rel="acl"`) is emitted as the `Link` response header; `None` silently omits it
/// (fail-closed — if there is no governing ACL there is nothing to advertise).
fn json_response_with_link(status: StatusCode, body: String, acl_link: Option<&str>) -> Response {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CONTENT_LENGTH, body.len());
    if let Some(link_val) = acl_link {
        // POSITIONAL format args (CodeQL rust/unused-variable guard).
        builder = builder.header(header::LINK, link_val);
    }
    builder
        .body(body.into())
        // A well-formed status + valid header values never fails to build.
        .unwrap()
}

/// Parse the request body as the `/authz/*` JSON envelope. Returns the caller's fail-closed error
/// [`Response`] (a `400`) on any malformed/absent field — the offending detail is NOT echoed to the
/// client (the same info-leak posture as the rest of the surface). `require_mode`/`require_query`
/// let each endpoint demand the field it needs.
///
/// Fail-closed: a missing `dataset` is a `400` (there is nothing to authorise over), NOT an
/// empty-dataset allow.
// `Response` is a large type; returning it in the `Err` arm mirrors the rest of this
// crate's handler helpers (`parse_shapes_graph`) — same `result_large_err` allow.
#[allow(clippy::result_large_err)]
fn parse_request(body: &Bytes) -> Result<AuthzRequest, Response> {
    let v: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| json_error(StatusCode::BAD_REQUEST, "request body is not valid JSON"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "request body must be a JSON object"))?;
    // sq-snopa.8 — the stateless/stateful switch. Only an ABSENT key defaults to
    // `body` (the historical behaviour, byte-identical). A key that is PRESENT but not a string
    // (`null`, `false`, a number, an array/object) is a fail-closed client error, NOT an absent
    // key: this selector decides whether authorisation trusts the server's own store or a
    // caller-supplied dataset, so a malformed value must never silently cross into the body lane.
    // An unknown keyword is likewise refused, never an inferred default.
    let source = match obj.get("source") {
        None => AuthzSource::Body,
        Some(serde_json::Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "body" => AuthzSource::Body,
            "server" => AuthzSource::Server,
            _ => {
                return Err(json_error(
                    StatusCode::BAD_REQUEST,
                    "'source' must be 'body' or 'server' when present",
                ))
            }
        },
        Some(_) => {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "'source' must be 'body' or 'server' when present",
            ))
        }
    };
    let dataset = match source {
        AuthzSource::Body => obj
            .get("dataset")
            .and_then(|d| d.as_str())
            .ok_or_else(|| {
                json_error(
                    StatusCode::BAD_REQUEST,
                    "'dataset' (N-Quads string) is required",
                )
            })?
            .to_owned(),
        // A body carrying BOTH a dataset and `"source":"server"` is ambiguous about which pod it
        // means; guessing either way could authorise against the pod the caller did not intend,
        // so refuse (fail-closed) rather than silently pick one.
        AuthzSource::Server => {
            if obj.contains_key("dataset") {
                return Err(json_error(
                    StatusCode::BAD_REQUEST,
                    "'source':'server' authorises over the server's own store; 'dataset' must be absent",
                ));
            }
            String::new()
        }
    };
    // The session sub-object; absent -> an anonymous session (fail-closed, sees only public).
    let session = obj.get("session").and_then(|s| s.as_object());
    let field = |name: &str| -> Option<String> {
        session
            .and_then(|s| s.get(name))
            .and_then(|f| f.as_str())
            .map(str::to_owned)
    };
    let view = match obj.get("view").and_then(|s| s.as_str()) {
        None => None,
        Some(s) => match s.trim().to_ascii_lowercase().as_str() {
            "wac" => Some(AuthView::Wac),
            "acp" => Some(AuthView::Acp),
            // An unknown view keyword is a fail-closed client error, not an inferred default.
            _ => {
                return Err(json_error(
                    StatusCode::BAD_REQUEST,
                    "'view' must be 'wac' or 'acp' when present",
                ))
            }
        },
    };
    Ok(AuthzRequest {
        source,
        #[cfg(feature = "odrl-authz")]
        carries_odrl: source == AuthzSource::Body && dataset_carries_odrl(&dataset),
        dataset,
        agent: field("agent"),
        client: field("client"),
        issuer: field("issuer"),
        now: field("now"),
        view,
    })
}

/// Build + materialise a [`PodStore`] over the request's dataset, choosing WAC vs ACP by the
/// request's `view` (or inferring it from the dataset's control-document suffixes). Returns the
/// caller's fail-closed error [`Response`] (a `400`) on a parse failure and a `503` on a
/// materialisation failure (a transient/operational condition, retryable) — NEVER a store that
/// silently grants.
///
/// Inference: `.acr` present in the dataset -> ACP; otherwise WAC. This is a convenience only; a
/// caller that knows its model should set `"view"` explicitly.
// `Response` is large; same `result_large_err` allow as `parse_request` / the crate's
// other handler helpers. Only the trust lane and the tests take a whole request; the
// stateless lane owns the fields instead ([`resolve_store`]).
#[cfg(any(test, feature = "solid-authz-trust"))]
#[allow(clippy::result_large_err)]
fn build_store(req: &AuthzRequest) -> Result<PodStore, Response> {
    build_store_from(&req.dataset, req.view)
}

/// [`build_store`] over the two fields it reads, so the blocking-pool lane
/// ([`resolve_store`]) can own them for the task rather than borrow the request.
// `Response` is large; same `result_large_err` allow as the rest of this module.
#[allow(clippy::result_large_err)]
fn build_store_from(dataset: &str, requested_view: Option<AuthView>) -> Result<PodStore, Response> {
    let graph = sparq_core::Graph::load_dataset(dataset, "nquads").map_err(|_| {
        // The parser's detail is withheld from the client (info-leak posture); the class is enough.
        json_error(
            StatusCode::BAD_REQUEST,
            "'dataset' is not valid N-Quads",
        )
    })?;
    let view = requested_view.unwrap_or_else(|| infer_view(dataset));
    let mut store = PodStore::new(graph);
    let materialized = match view {
        AuthView::Wac => store.materialize_wac(),
        AuthView::Acp => store.materialize_acp(),
    };
    materialized.map_err(|e| {
        // Materialisation failed (e.g. a reserved-encoding collision / reasoner error, or the
        // request's own documents past the authorization allowance or input budget): a
        // transient/operational condition — 503 (retryable), fail-closed (no grant).
        //
        // The refusal KIND is logged so an operator can tell "this request's dataset is too big
        // for the allowance in force" from "the reasoner failed"; the message is not logged and
        // never reaches the client (info-leak posture), and the status is unchanged either way.
        log_materialize_refusal("body", materialize_refusal_kind(&e));
        json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "could not materialise the authorization view",
        )
    })?;
    Ok(store)
}

/// Infer the auth model from the dataset text: ACP if it names any `.acr` control graph, else WAC.
/// A cheap substring scan over the N-Quads — the same suffix check the loader uses; a caller that
/// knows its model should set `"view"` explicitly rather than rely on this. Pure + unit-tested.
pub(crate) fn infer_view_is_acp(dataset: &str) -> bool {
    // A `.acr` graph name appears as `<…​.acr>` in an N-Quad's graph position; `.acr` occurring
    // only inside a subject/predicate/object IRI would be a false positive, but a control document
    // is ALWAYS named by its own IRI in the graph slot, so the presence of the token is a sound
    // "ACP is in play" signal for the inference default. WAC is the safe fallback either way — an
    // ACP dataset materialised as WAC simply finds no `.acl` grants and denies (fail-closed).
    dataset.contains(".acr>") || dataset.contains(".acr ")
}

/// [`infer_view_is_acp`] as the internal [`AuthView`].
fn infer_view(dataset: &str) -> AuthView {
    if infer_view_is_acp(dataset) {
        AuthView::Acp
    } else {
        AuthView::Wac
    }
}

// ── sq-snopa.8 — the STATEFUL lane (`"source":"server"`) ──────
//
// The stateless lane above materialises a throwaway `PodStore` per request. The stateful lane
// authorises over the SERVER'S OWN loaded store instead: it pins the current generation, forks
// its immutable snapshot into an owned `Graph` (`Graph::fork`, whose steady-state cost is
// O(pending delta) on the already-forked ring lineage — that is what removes the FR-4 note's
// blocker, which assumed a `&Graph`/no-Clone snapshot forced a full re-parse per request),
// materialises the auth view ONCE, and caches it in `AppState` keyed by serving lineage and generation.
//
// INVALIDATION is by construction, not by bookkeeping: the view is a pure function of one
// published generation, and EVERY commit — an `.acl`/`.acr` write included — publishes a new
// generation. So an ACL change makes the cached entry stale immediately and the next request
// re-materialises; the cached view is never older than the generation the request pins, and the
// writer needs no knowledge of the auth view at all.
//
// FAIL-CLOSED, same contract as the stateless lane: a materialisation failure is a retryable 503
// and caches nothing; under `odrl-authz` an ODRL-carrying server store is REFUSED (dropping a
// prohibition would be fail-OPEN); under `solid-authz-trust` a `"trust"` block, which decides
// over its own body-derived store, is refused in this lane.

/// Is the server's own store an ACP pod? True when it holds any `.acr` control graph — the
/// graph-level twin of [`infer_view_is_acp`] (which scans N-Quads TEXT, unavailable here). WAC
/// is the safe fallback: an ACP pod materialised as WAC finds no `.acl` grants and denies.
/// Pure + directly unit-tested. sq-snopa.8.
pub(crate) fn graph_view_is_acp(graph: &sparq_core::Graph) -> bool {
    graph.named().iter().any(|(name, _)| match name {
        oxrdf::Term::NamedNode(n) => n.as_str().ends_with(".acr"),
        _ => false,
    })
}

/// Does the server's own store carry ODRL policy rules? An OVER-APPROXIMATE check — it asks
/// whether either ODRL rule predicate IRI is present in the dictionary of the default graph or
/// any named graph, not whether it is actually used in predicate position. Over-approximating
/// REFUSES more requests, which is the fail-closed direction; under-approximating would silently
/// drop a prohibition, which is fail-OPEN. Pure + directly unit-tested. sq-snopa.8.
#[cfg(feature = "odrl-authz")]
pub(crate) fn graph_carries_odrl(graph: &sparq_core::Graph) -> bool {
    fn holds(g: &sparq_core::Graph) -> bool {
        [
            "http://www.w3.org/ns/odrl/2/permission",
            "http://www.w3.org/ns/odrl/2/prohibition",
        ]
        .iter()
        .any(|iri| match oxrdf::NamedNode::new(*iri) {
            Ok(n) => g.id_of(&oxrdf::Term::NamedNode(n)).is_some(),
            Err(_) => false,
        })
    }
    holds(graph) || graph.named().iter().any(|(_, g)| holds(g))
}

/// Log which authorization allowance or input budget a refused materialisation reached, on
/// the `lane` it was refused on (`"body"` for the stateless per-request build, `"server"` for
/// the stateful per-generation view).
///
/// SANITISED BY CONSTRUCTION: [`AuthAllowance`] is an enum of limit kinds whose
/// [`AuthAllowance::limit_name`] returns a `&'static str` — a tag this build does not know
/// maps to `Unspecified`, not to the tag's own text — so nothing derived from the pod's
/// data, the request, or the refusal's own message (which quotes document IRIs and sizes)
/// can reach the log through here. A failure with no allowance kind is logged as such rather
/// than with its text.
fn log_materialize_refusal(lane: &'static str, kind: Option<AuthAllowance>) {
    tracing::warn!(
        target: "sparq_server::solid_authz",
        lane = lane,
        limit = refusal_limit_field(kind),
        "authorization view materialisation refused"
    );
}

/// Which allowance or budget the materialisation error `e` reached, if any — the ONE place
/// this module reads a refusal kind off a materialisation error, so neither lane can go back
/// to discarding it.
fn materialize_refusal_kind(e: &str) -> Option<AuthAllowance> {
    sparq_solid::auth_allowance_refusal(e)
}

/// The static name [`log_materialize_refusal`] logs for `kind`. Everything this can return is
/// a compile-time string, which is what makes the log line sanitised.
fn refusal_limit_field(kind: Option<AuthAllowance>) -> &'static str {
    kind.map_or("none", AuthAllowance::limit_name)
}

/// The fail-closed [`Response`] for a [`ServerViewError`]. A materialisation failure mirrors the
/// stateless lane's 503 (operational, retryable); an ODRL-carrying store is a definitive 400.
fn server_view_error_response(err: ServerViewError) -> Response {
    match err {
        ServerViewError::Materialize(kind) => {
            log_materialize_refusal("server", kind);
            json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "could not materialise the authorization view",
            )
        }
        #[cfg(feature = "odrl-authz")]
        ServerViewError::Odrl => json_error(
            StatusCode::BAD_REQUEST,
            "the server's store carries ODRL policy rules; the stateful ('source':'server') lane \
             cannot enforce them (fail-closed)",
        ),
    }
}

/// Build (or reuse) the STATEFUL authorization view over the server's own loaded store.
/// Synchronous and CPU-bound (the N3 materialisation) — call it on the blocking pool, which is
/// what [`server_store`] does.
///
/// A cache HIT (same lineage, generation and auth model) is one `RwLock` read plus an `Arc` clone. A
/// MISS forks the pinned snapshot, materialises, and installs the result. Two concurrent misses
/// may both materialise; both results are correct for their generation and the newer one wins
/// the cache slot, so the duplicated work is the only cost of not holding the lock across the
/// (potentially long) materialisation.
///
/// A POISONED cache lock degrades to "no cache" (rebuild every request) rather than failing the
/// request or, worse, serving a stale view.
fn build_server_view(
    state: &AppState,
    requested_view: Option<AuthView>,
) -> Result<Arc<PodStore>, ServerViewError> {
    #[cfg(feature = "backup")]
    let core = state.authz_core();
    #[cfg(feature = "backup")]
    let pinned = core.ring.current();
    #[cfg(not(feature = "backup"))]
    let pinned = state.current();
    let generation = pinned.number();
    let graph = pinned.snapshot();
    let inferred_view = if graph_view_is_acp(graph) {
        AuthView::Acp
    } else {
        AuthView::Wac
    };
    let view = requested_view.unwrap_or(inferred_view);
    // Refuse BEFORE consulting the cache so the refusal cannot be skipped by a warm entry.
    #[cfg(feature = "odrl-authz")]
    if graph_carries_odrl(graph) {
        return Err(ServerViewError::Odrl);
    }
    if let Ok(cache) = state.authz_view_cache().read() {
        if let Some(cached) = cache.as_ref() {
            #[cfg(feature = "backup")]
            let same_lineage = Arc::ptr_eq(&cached.core, &core);
            #[cfg(not(feature = "backup"))]
            let same_lineage = true;
            if same_lineage && cached.generation == generation && cached.view == view {
                return Ok(cached.store.clone());
            }
        }
    }
    #[cfg(test)]
    {
        let hook = state.authz_build_pool().view_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook.block();
        }
    }
    let mut store = PodStore::new(graph.fork());
    let materialized = match view {
        AuthView::Wac => store.materialize_wac(),
        AuthView::Acp => store.materialize_acp(),
    };
    // The refusal KIND is carried out of the blocking task (the message is not: it quotes
    // document IRIs). `server_view_error_response` logs it.
    materialized.map_err(|e| ServerViewError::Materialize(materialize_refusal_kind(&e)))?;
    let store = Arc::new(store);
    if let Ok(mut cache) = state.authz_view_cache().write() {
        // An old lineage must never install after restore, even into an empty cache.
        #[cfg(feature = "backup")]
        let current_lineage = Arc::ptr_eq(&state.authz_core(), &core);
        #[cfg(not(feature = "backup"))]
        let current_lineage = true;
        // Within the same lineage, never replace a newer generation with an older one.
        let install = match cache.as_ref() {
            None => true,
            Some(cached) => {
                #[cfg(feature = "backup")]
                let same_lineage = Arc::ptr_eq(&cached.core, &core);
                #[cfg(not(feature = "backup"))]
                let same_lineage = true;
                !same_lineage || cached.generation <= generation
            }
        };
        if current_lineage && install {
            *cache = Some(CachedAuthView {
                #[cfg(feature = "backup")]
                core,
                generation,
                view,
                inferred_view,
                store: store.clone(),
            });
        }
    }
    Ok(store)
}

/// [`build_server_view`] on the BOUNDED blocking pool — the materialisation is a whole-store
/// N3 run, so it must never occupy an async worker, and it shares the one authorization-build
/// cap with every other lane ([`on_build_pool`]). A saturated pool or a join failure is the
/// same fail-closed 503: no decision is produced.
// `Response` is large; same `result_large_err` allow as the rest of this module.
#[allow(clippy::result_large_err)]
async fn server_store(
    state: &AppState,
    requested_view: Option<AuthView>,
) -> Result<Arc<PodStore>, Response> {
    // Lineage plus generation identifies the snapshot, including automatic view selection
    // and the ODRL refusal. Restores can reuse generation zero with different policies.
    #[cfg(feature = "backup")]
    let core = state.authz_core();
    #[cfg(feature = "backup")]
    let generation = core.ring.current().number();
    #[cfg(not(feature = "backup"))]
    let generation = state.current().number();
    if let Ok(cache) = state.authz_view_cache().read() {
        if let Some(cached) = cache.as_ref() {
            #[cfg(feature = "backup")]
            let same_lineage = Arc::ptr_eq(&cached.core, &core);
            #[cfg(not(feature = "backup"))]
            let same_lineage = true;
            if same_lineage
                && cached.generation == generation
                && requested_view.unwrap_or(cached.inferred_view) == cached.view
            {
                return Ok(cached.store.clone());
            }
        }
    }
    let st = state.clone();
    on_build_pool(state, move || build_server_view(&st, requested_view))
        .await?
        .map_err(server_view_error_response)
}

/// [`build_store`] on the blocking pool — the STATELESS lane's counterpart of
/// [`server_store`].
///
/// This lane parses and materialises the REQUEST's own dataset, so its cost is set by the
/// body: the N-Quads parse, the resource enumeration and the N3 materialisation are all
/// CPU-bound, bounded by the body-size limit and `sparq_solid::AuthBudget` rather than
/// instant, and so must no more occupy an async worker than the stateful lane's whole-store
/// run does. It shares the one authorization-build cap with every other lane
/// ([`on_build_pool`]); a saturated pool or a join failure is the same fail-closed 503 a
/// materialisation failure is: no decision is produced.
// `Response` is large; same `result_large_err` allow as the rest of this module.
#[allow(clippy::result_large_err)]
async fn body_store(
    state: &AppState,
    req: &AuthzRequest,
    mode: Mode,
) -> Result<PodStore, Response> {
    let owned = req.clone();
    on_build_pool(state, move || {
        let store = build_store_from(&owned.dataset, owned.view)?;
        #[cfg(feature = "odrl-authz")]
        {
            let mut store = store;
            if owned.carries_odrl {
                apply_odrl_lane(&mut store, &owned, mode)?;
            }
            Ok(store)
        }
        #[cfg(not(feature = "odrl-authz"))]
        {
            let _ = mode;
            Ok(store)
        }
    })
    .await?
}

/// The store an endpoint decides over, per the request's [`AuthzSource`]: the stateless
/// per-request build ([`body_store`]) or the stateful per-generation view ([`server_store`]).
/// Both lanes are fail-closed; neither can yield a store that silently grants.
// `Response` is large; same `result_large_err` allow as the rest of this module.
#[allow(clippy::result_large_err)]
async fn resolve_store(
    state: &AppState,
    req: &AuthzRequest,
    mode: Mode,
) -> Result<ResolvedStore, Response> {
    match req.source {
        AuthzSource::Body => body_store(state, req, mode)
            .await
            .map(|s| ResolvedStore::Owned(Box::new(s))),
        AuthzSource::Server => server_store(state, req.view).await.map(ResolvedStore::Shared),
    }
}

/// `POST /authz/decide` — the per-request WAC/ACP decision (FR-1/FR-4). Body:
/// `{ "dataset": nquads, "session": { "agent"?, "client"?, "issuer"?, "now"? }, "resource": iri,
/// "mode": "read"|"write"|"append"|"control", "view"?: "wac"|"acp", "source"?: "body"|"server" }`.
/// With `"source":"server"` the `"dataset"` is omitted and the decision runs over the server's
/// own loaded store (sq-snopa.8 — see the module doc). Returns the
/// [`decision_to_json`] body with the HTTP status [`deny_status_code`] maps a DENY to (an ALLOW is
/// always `200`).
///
/// Fail-closed: an unknown mode, an unparseable dataset, or a materialisation failure all DENY
/// (never grant). The endpoint reads the store; it is gated like any read.
///
/// sq-3mu76: behind the `odrl-authz` feature, a dataset carrying ODRL policy rules
/// runs the [`apply_odrl_lane`] materialisation first, so a prohibition denies HERE exactly as
/// `/authz/query` refuses to honour it — with the query lane's refusal matrix (non-read mode =>
/// 400 until sq-lrtc3.2, anonymous session => 403) and the read-scoped `grantedModes`
/// advertisement (module doc). NOTE the decision stays governing-ACL-scoped: with NO
/// discoverable `.acl`/`.acr` anywhere, `decide` still returns its fail-closed `noAcl` deny
/// even where a bridged ODRL grant lets `/authz/query` see rows — a deny-side-only divergence.
pub(crate) async fn decide_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().solid_authz {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let req = match parse_request(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    // The resource IRI + mode are decide-specific fields.
    let v: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "request body is not valid JSON"),
    };
    #[cfg(feature = "solid-authz-trust")]
    let mut v = v;
    let resource = match v.get("resource").and_then(|r| r.as_str()) {
        Some(r) => r.to_owned(),
        None => {
            return json_error(StatusCode::BAD_REQUEST, "'resource' (IRI string) is required");
        }
    };
    // An unknown / absent mode is a fail-closed deny: an un-parseable mode can never be granted, so
    // we return a definitive 403 with the deny body rather than trusting an arbitrary keyword.
    let mode = match v.get("mode").and_then(|m| m.as_str()).and_then(parse_mode) {
        Some(m) => m,
        None => {
            let deny = decision_to_json(&fail_closed_decision());
            return json_response(StatusCode::FORBIDDEN, deny);
        }
    };
    // sq-3mu76 — the trust dispatch below decides over its OWN store and would
    // BYPASS the ODRL lane, silently dropping a prohibition (fail-OPEN through a side door).
    // Composing the two lanes is future work; until then the combination is refused up front.
    #[cfg(all(feature = "solid-authz-trust", feature = "odrl-authz"))]
    if state.config().solid_authz_trust && v.get("trust").is_some() && req.carries_odrl {
        return json_error(
            StatusCode::BAD_REQUEST,
            "the dataset carries ODRL policies; combining the trust extension with the \
             ODRL lane is not supported (fail-closed)",
        );
    }
    // sq-pfae.17 — DOUBLE-OPT-IN stateless trust-graph extension.
    // Activates ONLY when BOTH (a) the `solid-authz-trust` feature is compiled AND (b) the config
    // flag is set AND (c) the request carries a "trust" JSON block. Any failure => 403 DENY
    // (fail-closed: the trust path can only narrow, never broaden, the admission gate).
    #[cfg(feature = "solid-authz-trust")]
    if state.config().solid_authz_trust {
        if let Some(trust_val) = v.get_mut("trust") {
            // sq-snopa.8 — the trust lane decides over its OWN store, built by
            // injecting admitted facts into the BODY dataset, so it cannot compose with the
            // stateful lane's shared per-generation view. Refuse rather than silently authorise
            // over the wrong pod.
            if req.source == AuthzSource::Server {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "the trust extension decides over the request dataset; it cannot be combined \
                     with 'source':'server' (fail-closed)",
                );
            }
            return trust_authz_decide(&state, trust_val.take(), req, resource, mode).await;
        }
    }
    let store = match resolve_store(&state, &req, mode).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    #[cfg(feature = "odrl-authz")]
    let odrl_lane_fired = req.source == AuthzSource::Body && req.carries_odrl;
    // Deciding is CPU-light here (the materialise already ran), so it stays inline.
    let decision = store.get().decide(&req.session(), &resource, mode);
    // sq-3mu76 — read-scoped advertisement (module doc): when the lane fired, mask
    // the advisory `grantedModes` to the requested (read) mode — the lane evidenced no other.
    #[cfg(feature = "odrl-authz")]
    let decision = {
        let mut decision = decision;
        if odrl_lane_fired {
            mask_decision_to_mode(&mut decision, mode);
        }
        decision
    };
    let status = if decision.allow {
        StatusCode::OK
    } else {
        deny_status_code(decision.status)
    };
    // sq-snopa.7 FR-5: emit the RFC 8288 `Link: <acl-iri>; rel="acl"` header from
    // the decision's governing ACL. `acl_link_header()` returns `None` when no governing ACL was
    // discovered (fail-closed — nothing to advertise). Safe on a deny: the link tells the client
    // WHERE the ACL is, not that the request is allowed.
    let acl_link = decision.acl_link_header();
    json_response_with_link(status, decision_to_json(&decision), acl_link.as_deref())
}

/// A fail-closed [`WacDecision`] for an input the endpoint refuses to trust (e.g. an unknown mode):
/// an authoritative deny with no governing ACL. Distinct from the library's own `deny` (which is
/// crate-private) — this is the HTTP layer's own "I will not even try" deny. Pure + unit-tested.
fn fail_closed_decision() -> WacDecision {
    WacDecision {
        allow: false,
        granted_modes: Vec::new(),
        governing_acl: None,
        scope: None,
        status: AclStatus::Resolved,
    }
}

// ── sq-pfae.17 — stateless trust-graph extension ──────────────
//
// The `trust_authz_decide` function and all its helpers are compiled ONLY behind
// the `solid-authz-trust` feature (which implies `solid-authz` + `sparq-trust`
// with the `cert-graph` feature). Feature OFF => zero compiled code, byte-identical
// to `solid-authz`.
//
// FAIL-CLOSED INVARIANT: every parse/verify/dispatch failure returns a 403 DENY.
// The trust path can only NARROW (never broaden) the WAC/ACP admission gate: the
// cert-graph closure produces `direct_rules ++ derived_rules` (design §3.4), and
// the unchanged `sparq_trust::admit` gate vetoes each rule independently.
//
// POSITIONAL format args used throughout (CodeQL rust/unused-variable guard).

/// (sq-pfae.17) A minimal admission justification returned alongside the trust-path
/// decision: which rule sourced the deciding admit (its IRI), how many facts were admitted,
/// and whether the decision went through the cert-graph closure.
#[cfg(feature = "solid-authz-trust")]
#[cfg_attr(docsrs, doc(cfg(feature = "solid-authz-trust")))]
#[derive(Debug)]
struct TrustJustification {
    /// IRI of the `trust:source` of the first admitted fact (the deciding rule).
    admitted_source: String,
    /// Count of distinct admitted facts.
    admitted_count: usize,
    /// Whether the cert-graph closure produced at least one derived rule (beyond the direct rules).
    cert_graph_derived: bool,
}

/// (sq-pfae.17) Serialise a `TrustJustification` into the JSON fragment added to
/// the decide response body under the `"trust"` key. POSITIONAL format args (CodeQL guard).
#[cfg(feature = "solid-authz-trust")]
fn justification_to_json(j: &TrustJustification) -> serde_json::Value {
    serde_json::json!({
        "admittedSource": j.admitted_source,
        "admittedCount": j.admitted_count,
        "certGraphDerived": j.cert_graph_derived,
    })
}

/// (sq-pfae.17) A parsed trust policy rule from the JSON wire format.
///
/// Wire schema (one element of `"trust"."rules"` array):
/// ```json
/// {
///   "source": "https://issuer.example/",
///   "issuerKeyHex": "<64-byte-hex compressed public key>",
///   "scopeIri": "https://pod.ex/resource",
///   "freshWithinSecs": 86400,
///   "shapePredicateIri": "https://schema.org/age"
/// }
/// ```
///
/// `shapePredicateIri` is the `trust:forPredicate` IRI — desugared into the `ShapeRef`
/// that `sparq_trust::policy::TrustRule` carries.
#[cfg(feature = "solid-authz-trust")]
struct WireTrustRule {
    source: String,
    issuer_key_hex: String,
    scope_iri: String,
    fresh_within_secs: i64,
    shape_predicate_iri: String,
}

/// (sq-pfae.17) A parsed certification edge from the JSON wire format.
///
/// Wire schema (one element of `"trust"."certifications"` array):
/// ```json
/// {
///   "certifierIri": "https://gov.example/framework",
///   "certifierKeyHex": "<hex>",
///   "certifiedIssuerIri": "https://issuer.example/dvs",
///   "certifiedKeyHex": "<hex>",
///   "validFromUnixSecs": 0,
///   "validUntilUnixSecs": 9999999999,
///   "signatureHex": "<hex>",
///   "scopeKind": "anyService"
/// }
/// ```
///
/// `scopeKind` is one of the two explicitly modelled kinds — `"anyService"` (no
/// statement-type narrowing) or `"shape"` (statement-type-scoped, see [`WireCertScope`]) —
/// and absent/other is a fail-closed deny: unknown scope kinds are never silently widened.
#[cfg(feature = "solid-authz-trust")]
struct WireCertification {
    certifier_iri: String,
    certifier_key_hex: String,
    certified_issuer_iri: String,
    certified_key_hex: String,
    valid_from_unix_secs: i64,
    valid_until_unix_secs: i64,
    signature_hex: String,
    scope: WireCertScope,
}

/// (sq-sllu4) The parsed `"scopeKind"` of a wire certification — the JSON
/// projection of `sparq_trust::graph::CertScope`.
///
/// ```json
/// { …, "scopeKind": "anyService" }
/// { …, "scopeKind": "shape", "scopeShapePredicateIri": "https://schema.org/age" }
/// ```
///
/// A `"shape"` scope carries a `trust:forPredicate` IRI, desugared server-side by
/// `cert_scope_predicate_shape` into the CANONICAL single-predicate SHACL node-shape the
/// certifier signed over. Only a predicate IRI is transportable in v1: an arbitrary
/// `trust:forShape` closure has no wire encoding here (it would need a canonical RDF
/// serialisation the signer and this parser agree on term-for-term, in order), so it is
/// deliberately absent rather than approximated.
#[cfg(feature = "solid-authz-trust")]
enum WireCertScope {
    /// `"scopeKind": "anyService"` — `CertScope::AnyService`: the derived rule inherits the
    /// certifier anchor's shape unchanged.
    AnyService,
    /// `"scopeKind": "shape"` — `CertScope::Shape(canonical desugaring of the predicate)`:
    /// the certified issuer is vouched for ONLY over statements of this predicate, and the
    /// closure admits the edge only where that provably narrows the certifier's anchor.
    Shape { predicate_iri: String },
}

/// (sq-pfae.17) A parsed presented credential from the JSON wire format.
///
/// Wire schema (one element of `"trust"."credentials"` array):
/// ```json
/// {
///   "graphNquads": "<N-Quads string>",
///   "issuerSignatureHex": "<hex>",
///   "saltHex": "<64-hex-byte>",
///   "issuedAtUnixSecs": 1720000000,
///   "revoked": false
/// }
/// ```
#[cfg(feature = "solid-authz-trust")]
struct WireCredential {
    graph_nquads: String,
    issuer_signature_hex: String,
    salt_hex: String,
    issued_at_unix_secs: i64,
    revoked: bool,
}

/// (sq-pfae.17) Parse a `WireTrustRule` from a JSON object. Fail-closed: any
/// missing/mistyped field => `Err(reason)`.
#[cfg(feature = "solid-authz-trust")]
fn parse_wire_rule(obj: &serde_json::Map<String, serde_json::Value>) -> Result<WireTrustRule, &'static str> {
    let source = obj.get("source").and_then(|v| v.as_str())
        .ok_or("trust rule missing 'source'")?
        .to_owned();
    let issuer_key_hex = obj.get("issuerKeyHex").and_then(|v| v.as_str())
        .ok_or("trust rule missing 'issuerKeyHex'")?
        .to_owned();
    let scope_iri = obj.get("scopeIri").and_then(|v| v.as_str())
        .ok_or("trust rule missing 'scopeIri'")?
        .to_owned();
    let fresh_within_secs = obj.get("freshWithinSecs").and_then(|v| v.as_i64())
        .ok_or("trust rule missing/invalid 'freshWithinSecs'")?;
    let shape_predicate_iri = obj.get("shapePredicateIri").and_then(|v| v.as_str())
        .ok_or("trust rule missing 'shapePredicateIri'")?
        .to_owned();
    Ok(WireTrustRule { source, issuer_key_hex, scope_iri, fresh_within_secs, shape_predicate_iri })
}

/// (sq-pfae.17) Parse a `WireCertification` from a JSON object. Fail-closed.
#[cfg(feature = "solid-authz-trust")]
fn parse_wire_certification(obj: &serde_json::Map<String, serde_json::Value>) -> Result<WireCertification, &'static str> {
    let certifier_iri = obj.get("certifierIri").and_then(|v| v.as_str())
        .ok_or("certification missing 'certifierIri'")?
        .to_owned();
    let certifier_key_hex = obj.get("certifierKeyHex").and_then(|v| v.as_str())
        .ok_or("certification missing 'certifierKeyHex'")?
        .to_owned();
    let certified_issuer_iri = obj.get("certifiedIssuerIri").and_then(|v| v.as_str())
        .ok_or("certification missing 'certifiedIssuerIri'")?
        .to_owned();
    let certified_key_hex = obj.get("certifiedKeyHex").and_then(|v| v.as_str())
        .ok_or("certification missing 'certifiedKeyHex'")?
        .to_owned();
    let valid_from_unix_secs = obj.get("validFromUnixSecs").and_then(|v| v.as_i64())
        .ok_or("certification missing/invalid 'validFromUnixSecs'")?;
    let valid_until_unix_secs = obj.get("validUntilUnixSecs").and_then(|v| v.as_i64())
        .ok_or("certification missing/invalid 'validUntilUnixSecs'")?;
    let signature_hex = obj.get("signatureHex").and_then(|v| v.as_str())
        .ok_or("certification missing 'signatureHex'")?
        .to_owned();
    // Fail-closed on scope kind: only the two explicitly modelled kinds are accepted; an
    // absent or unrecognised scope kind denies (never silently widened to AnyService).
    let scope_kind = obj.get("scopeKind").and_then(|v| v.as_str())
        .ok_or("certification missing 'scopeKind'")?;
    let scope = match scope_kind {
        "anyService" => WireCertScope::AnyService,
        // (sq-sllu4) A shape scope MUST carry the predicate IRI it is scoped to:
        // a "shape" kind with no predicate is not a wider scope, it is an unspecified one.
        "shape" => WireCertScope::Shape {
            predicate_iri: obj.get("scopeShapePredicateIri").and_then(|v| v.as_str())
                .ok_or("certification 'shape' scope missing 'scopeShapePredicateIri'")?
                .to_owned(),
        },
        _ => return Err("certification 'scopeKind' must be 'anyService' or 'shape'"),
    };
    Ok(WireCertification {
        certifier_iri, certifier_key_hex, certified_issuer_iri, certified_key_hex,
        valid_from_unix_secs, valid_until_unix_secs, signature_hex, scope,
    })
}

/// (sq-pfae.17) Parse a `WireCredential` from a JSON object. Fail-closed.
#[cfg(feature = "solid-authz-trust")]
fn parse_wire_credential(obj: &serde_json::Map<String, serde_json::Value>) -> Result<WireCredential, &'static str> {
    let graph_nquads = obj.get("graphNquads").and_then(|v| v.as_str())
        .ok_or("credential missing 'graphNquads'")?
        .to_owned();
    let issuer_signature_hex = obj.get("issuerSignatureHex").and_then(|v| v.as_str())
        .ok_or("credential missing 'issuerSignatureHex'")?
        .to_owned();
    let salt_hex = obj.get("saltHex").and_then(|v| v.as_str())
        .ok_or("credential missing 'saltHex'")?
        .to_owned();
    let issued_at_unix_secs = obj.get("issuedAtUnixSecs").and_then(|v| v.as_i64())
        .ok_or("credential missing/invalid 'issuedAtUnixSecs'")?;
    let revoked = obj.get("revoked").and_then(|v| v.as_bool())
        .ok_or("credential missing/invalid 'revoked'")?;
    Ok(WireCredential { graph_nquads, issuer_signature_hex, salt_hex, issued_at_unix_secs, revoked })
}

/// (sq-pfae.17) Decode a 64-hex-char (32-byte) salt from a hex string. Fail-closed.
#[cfg(feature = "solid-authz-trust")]
fn decode_salt_hex(hex: &str) -> Option<[u8; 32]> {
    // Reject a wrong length before decoding allocates anything for it.
    if hex.len() != 64 {
        return None;
    }
    sparq_core::hex::decode(hex)?.try_into().ok()
}

/// (sq-pfae.17) Build a `sparq_trust::policy::ShapeRef` for a `trust:forPredicate`
/// IRI. This is the same desugaring `sparq_trust::policy` does internally: a single-predicate
/// SHACL node-shape targeting subjects of `predicate_iri` and requiring it present with
/// `sh:minCount 1`. Fail-closed if the IRI is not valid.
///
/// The blank-node labels are FRESH (they name nothing outside the shape, and the closure's
/// narrowing matcher is label-agnostic — it matches blank nodes by an injective structural
/// mapping). A shape that must be REPRODUCED byte-for-byte by a remote signer needs the
/// canonical-label variant instead — see [`cert_scope_predicate_shape`].
#[cfg(feature = "solid-authz-trust")]
fn predicate_shape(predicate_iri: &str) -> Option<sparq_trust::policy::ShapeRef> {
    predicate_shape_with_labels(
        predicate_iri,
        oxrdf::BlankNode::default(),
        oxrdf::BlankNode::default(),
    )
}

/// (sq-sllu4) Canonical blank-node label of the shape-scope root. See
/// [`cert_scope_predicate_shape`] for why it is FIXED rather than fresh.
#[cfg(feature = "solid-authz-trust")]
const CERT_SCOPE_SHAPE_LABEL: &str = "certScopeShape";

/// (sq-sllu4) Canonical blank-node label of the shape-scope property node.
#[cfg(feature = "solid-authz-trust")]
const CERT_SCOPE_PROPERTY_LABEL: &str = "certScopeProperty";

/// (sq-sllu4) Build the CANONICAL `CertScope::Shape` shape for a wire
/// `"scopeKind": "shape"` certification's `scopeShapePredicateIri`.
///
/// Structurally this is [`predicate_shape`] — the same single-predicate node-shape — but its
/// blank-node labels are FIXED (`_:certScopeShape`, `_:certScopeProperty`) and its triples are
/// emitted in a FIXED order. That is load-bearing, not cosmetic:
/// `sparq_trust::graph::certification_message` folds each scope triple through
/// `sparq_zk::encode::encode_triple`, which hashes a blank node BY ITS LABEL, in listed order.
/// A fresh-label desugaring would therefore hash differently on every request and the
/// certifier's signature — made over ITS OWN desugaring — could never verify. Fixing the
/// labels + order makes the wire shape-scope a canonical, reproducible preimage: the certifier
/// signs exactly this shape, and this function reconstructs it.
///
/// A certifier producing a shape-scoped edge for this endpoint must build the scope as:
///
/// ```text
/// _:certScopeShape sh:targetSubjectsOf <P> .
/// _:certScopeShape sh:property         _:certScopeProperty .
/// _:certScopeProperty sh:path          <P> .
/// _:certScopeProperty sh:minCount      "1" .        # a plain literal, not xsd:integer
/// ```
///
/// (root = `_:certScopeShape`, triples in exactly that order). Anything else signs a
/// different message and is rejected at the closure's signature gate — fail-closed, as an
/// unverifiable edge should be. Fail-closed here too if the predicate is not a valid IRI.
#[cfg(feature = "solid-authz-trust")]
fn cert_scope_predicate_shape(predicate_iri: &str) -> Option<sparq_trust::policy::ShapeRef> {
    predicate_shape_with_labels(
        predicate_iri,
        oxrdf::BlankNode::new(CERT_SCOPE_SHAPE_LABEL).expect("static label is valid"),
        oxrdf::BlankNode::new(CERT_SCOPE_PROPERTY_LABEL).expect("static label is valid"),
    )
}

/// (sq-sllu4) The shared single-predicate desugaring behind [`predicate_shape`]
/// (fresh labels) and [`cert_scope_predicate_shape`] (canonical labels) — ONE construction, so
/// the two can never structurally drift apart. Fail-closed if the IRI is not valid.
#[cfg(feature = "solid-authz-trust")]
fn predicate_shape_with_labels(
    predicate_iri: &str,
    root: oxrdf::BlankNode,
    prop: oxrdf::BlankNode,
) -> Option<sparq_trust::policy::ShapeRef> {
    use oxrdf::{Literal, NamedNode, Term, Triple};
    let pred = NamedNode::new(predicate_iri).ok()?;
    let sh_target_subjects_of = NamedNode::new("http://www.w3.org/ns/shacl#targetSubjectsOf")
        .expect("static IRI is valid");
    let sh_property = NamedNode::new("http://www.w3.org/ns/shacl#property")
        .expect("static IRI is valid");
    let sh_path = NamedNode::new("http://www.w3.org/ns/shacl#path")
        .expect("static IRI is valid");
    let sh_mincount = NamedNode::new("http://www.w3.org/ns/shacl#minCount")
        .expect("static IRI is valid");
    Some(sparq_trust::policy::ShapeRef {
        root: Term::BlankNode(root.clone()),
        triples: vec![
            Triple::new(root.clone(), sh_target_subjects_of, pred.clone()),
            Triple::new(root.clone(), sh_property, prop.clone()),
            Triple::new(prop.clone(), sh_path, pred),
            Triple::new(prop, sh_mincount, Literal::new_simple_literal("1")),
        ],
    })
}

/// (sq-pfae.17) The core trust-path decision handler. Called from `decide_endpoint`
/// ONLY when BOTH (a) `solid-authz-trust` is compiled AND (b) the config flag is set AND (c) the
/// request carries a `"trust"` block. Returns a `Response` — always a complete HTTP response
/// (either an enriched allow/deny carrying a `"trust"` admission justification, or a plain 403
/// deny for any parse/verify failure — FAIL-CLOSED).
///
/// # Wire format for the `"trust"` block
///
/// ```json
/// {
///   "trust": {
///     "agentIri": "https://alice.ex/card#me",
///     "nowUnixSecs": 1720000000,
///     "rules": [ { "source": ..., "issuerKeyHex": ..., "scopeIri": ...,
///                  "freshWithinSecs": ..., "shapePredicateIri": ... }, ... ],
///     "certifications": [ { ... , "scopeKind": "anyService" },
///                         { ... , "scopeKind": "shape",
///                                 "scopeShapePredicateIri": "https://schema.org/age" }, ... ],
///     "credentials": [ { "graphNquads": ..., "issuerSignatureHex": ...,
///                        "saltHex": ..., "issuedAtUnixSecs": ..., "revoked": ... }, ... ]
///   }
/// }
/// ```
///
/// A `"shape"`-scoped certification (`sq-sllu4`) narrows the certified issuer to ONE
/// statement type; the shape the certifier must sign over is the canonical desugaring
/// documented on `cert_scope_predicate_shape`.
///
/// # Fail-closed invariant
///
/// ANY malformed / unverifiable / stale / revoked / over-depth / broadening trust input => 403 DENY.
/// The WAC/ACP path is the SOLE gate on the dataset; the trust block can inject additional
/// `trust:admitted` facts (via `sparq_trust::admit`) but it NEVER short-circuits or weakens the
/// WAC/ACP materialisation.
#[cfg(feature = "solid-authz-trust")]
#[allow(clippy::result_large_err)]
async fn trust_authz_decide(
    state: &AppState,
    trust: serde_json::Value,
    req: AuthzRequest,
    resource: String,
    mode: sparq_solid::Mode,
) -> Response {
    let st = state.clone();
    #[cfg(test)]
    let pool = state.authz_build_pool().clone();
    match on_build_pool(state, move || {
        trust_authz_decide_blocking(
            &trust,
            &req,
            &resource,
            mode,
            st.config(),
            #[cfg(test)]
            &pool.trust_verification_calls,
        )
    })
    .await
    {
        Ok(response) | Err(response) => response,
    }
}

#[cfg(feature = "solid-authz-trust")]
fn trust_authz_decide_blocking(
    trust_val: &serde_json::Value,
    req: &AuthzRequest,
    resource: &str,
    mode: sparq_solid::Mode,
    config: &crate::http::ServerConfig,
    #[cfg(test)] verification_calls: &std::sync::atomic::AtomicUsize,
) -> Response {
    use sparq_trust::{
        admit, derive_effective_rules, policy::TrustRule, AdmittedFact,
        PresentedCredential, Session as TrustSession,
    };
    use sparq_trust::graph::{Certification, CertScope};
    use sparq_zk::sig::public_key_from_hex;
    use oxrdf::NamedNode;

    // ── 1. Parse the trust block object ──────────────────────────────────────
    let trust_obj = match trust_val.as_object() {
        Some(o) => o,
        None => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("trust block must be a JSON object"),
            );
        }
    };

    // ── 2. Parse the trust session (agent IRI + now) ──────────────────────────
    let agent_iri_str = match trust_obj.get("agentIri").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("trust block missing 'agentIri'"),
            );
        }
    };
    let agent_iri = match NamedNode::new(agent_iri_str) {
        Ok(n) => n,
        Err(_) => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("trust 'agentIri' is not a valid IRI"),
            );
        }
    };
    let now_unix_secs = match trust_obj.get("nowUnixSecs").and_then(|v| v.as_i64()) {
        Some(n) => n,
        None => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("trust block missing/invalid 'nowUnixSecs'"),
            );
        }
    };
    let trust_session = TrustSession { agent: agent_iri.clone(), now_unix_secs };

    // ── 3. Parse direct trust rules ─────────────────────────────────────────
    let rules_val = match trust_obj.get("rules").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("trust block missing 'rules' array"),
            );
        }
    };
    let mut direct_rules: Vec<TrustRule> = Vec::new();
    for (idx, rule_v) in rules_val.iter().enumerate() {
        let obj = match rule_v.as_object() {
            Some(o) => o,
            None => {
                // POSITIONAL format args (CodeQL guard).
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json(&format!("trust rule {} is not an object", idx)),
                );
            }
        };
        let wire_rule = match parse_wire_rule(obj) {
            Ok(r) => r,
            Err(reason) => {
                return json_response(StatusCode::FORBIDDEN, trust_deny_json(reason));
            }
        };
        let source = match NamedNode::new(&wire_rule.source) {
            Ok(n) => n,
            Err(_) => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("trust rule 'source' is not a valid IRI"),
                );
            }
        };
        let scope = match NamedNode::new(&wire_rule.scope_iri) {
            Ok(n) => n,
            Err(_) => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("trust rule 'scopeIri' is not a valid IRI"),
                );
            }
        };
        let issuer_key = match public_key_from_hex(&wire_rule.issuer_key_hex) {
            Some(k) => k,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("trust rule 'issuerKeyHex' is not a valid public key"),
                );
            }
        };
        let shape = match predicate_shape(&wire_rule.shape_predicate_iri) {
            Some(s) => s,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("trust rule 'shapePredicateIri' is not a valid IRI"),
                );
            }
        };
        direct_rules.push(TrustRule { source, issuer_key, shape, scope, fresh_within_secs: wire_rule.fresh_within_secs });
    }

    // ── 4. Parse certifications ──────────────────────────────────────────────
    let certs_val = trust_obj
        .get("certifications")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let mut certifications: Vec<Certification> = Vec::new();
    for (idx, cert_v) in certs_val.iter().enumerate() {
        let obj = match cert_v.as_object() {
            Some(o) => o,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json(&format!("certification {} is not an object", idx)),
                );
            }
        };
        let wire = match parse_wire_certification(obj) {
            Ok(c) => c,
            Err(reason) => {
                return json_response(StatusCode::FORBIDDEN, trust_deny_json(reason));
            }
        };
        let certifier_iri = match NamedNode::new(&wire.certifier_iri) {
            Ok(n) => n,
            Err(_) => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("certification 'certifierIri' is not a valid IRI"),
                );
            }
        };
        let certified_issuer_iri = match NamedNode::new(&wire.certified_issuer_iri) {
            Ok(n) => n,
            Err(_) => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("certification 'certifiedIssuerIri' is not a valid IRI"),
                );
            }
        };
        let certifier_key = match public_key_from_hex(&wire.certifier_key_hex) {
            Some(k) => k,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("certification 'certifierKeyHex' is not a valid public key"),
                );
            }
        };
        let certified_key = match public_key_from_hex(&wire.certified_key_hex) {
            Some(k) => k,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("certification 'certifiedKeyHex' is not a valid public key"),
                );
            }
        };
        let scope = match &wire.scope {
            WireCertScope::AnyService => CertScope::AnyService,
            // (sq-sllu4) A shape scope is the CANONICAL single-predicate
            // desugaring the certifier signed over — see `cert_scope_predicate_shape`. The
            // closure still has to PROVE it narrows the certifier's anchor; a scope that does
            // not (e.g. a predicate the anchor never covered) contributes nothing.
            WireCertScope::Shape { predicate_iri } => {
                match cert_scope_predicate_shape(predicate_iri) {
                    Some(shape) => CertScope::Shape(shape),
                    None => {
                        return json_response(
                            StatusCode::FORBIDDEN,
                            trust_deny_json(
                                "certification 'scopeShapePredicateIri' is not a valid IRI",
                            ),
                        );
                    }
                }
            }
        };
        certifications.push(Certification {
            certifier: certifier_iri,
            certifier_key,
            certified_issuer: certified_issuer_iri,
            certified_key,
            scope,
            valid_from_unix_secs: wire.valid_from_unix_secs,
            valid_until_unix_secs: wire.valid_until_unix_secs,
            signature_hex: wire.signature_hex,
        });
    }

    // ── 6. Parse presented credentials ──────────────────────────────────────
    let creds_val = match trust_obj.get("credentials").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("trust block missing 'credentials' array"),
            );
        }
    };
    let resource_node = match NamedNode::new(resource) {
        Ok(n) => n,
        Err(_) => {
            return json_response(
                StatusCode::FORBIDDEN,
                trust_deny_json("'resource' is not a valid IRI"),
            );
        }
    };

    let mut credentials = Vec::new();
    for (idx, cred_v) in creds_val.iter().enumerate() {
        let obj = match cred_v.as_object() {
            Some(o) => o,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json(&format!("credential {} is not an object", idx)),
                );
            }
        };
        let wire = match parse_wire_credential(obj) {
            Ok(c) => c,
            Err(reason) => {
                return json_response(StatusCode::FORBIDDEN, trust_deny_json(reason));
            }
        };
        // Decode the 32-byte salt from hex.
        let salt = match decode_salt_hex(&wire.salt_hex) {
            Some(s) => s,
            None => {
                return json_response(
                    StatusCode::FORBIDDEN,
                    trust_deny_json("credential 'saltHex' must be 64 hex chars (32 bytes)"),
                );
            }
        };
        // Parse the credential graph from N-Quads. Each quad's triple is extracted regardless of
        // its named graph (the default graph is the canonical credential graph; named-graph quads
        // are treated as additional context — fail-closed means unknown predicates admit nothing).
        let mut triples: Vec<oxrdf::Triple> = Vec::new();
        for result in oxttl::NQuadsParser::new().for_slice(wire.graph_nquads.as_bytes()) {
            match result {
                Ok(quad) => {
                    // Use the subject/predicate/object from the quad, regardless of graph name.
                    let t = oxrdf::Triple::new(quad.subject, quad.predicate, quad.object);
                    triples.push(t);
                }
                Err(_) => {
                    return json_response(
                        StatusCode::FORBIDDEN,
                        trust_deny_json("credential 'graphNquads' is not valid N-Quads"),
                    );
                }
            }
        }
        if triples.is_empty() {
            // An empty credential graph admits nothing; skip it rather than sending to the gate.
            continue;
        }
        let cred = PresentedCredential {
            graph: triples,
            issuer_signature_hex: wire.issuer_signature_hex,
            salt,
            issued_at_unix_secs: wire.issued_at_unix_secs,
            revoked: wire.revoked,
        };
        credentials.push(cred);
    }

    // A depth-one certification produces at most one additional rule. Price the upper
    // bound before deriving any edge or verifying any credential, including empty graphs.
    let rules_bound = direct_rules.len().saturating_add(certifications.len());
    let pairs = rules_bound.saturating_mul(creds_val.len());
    let triples = credentials
        .iter()
        .fold(0usize, |n, c| n.saturating_add(c.graph.len()));
    let validation_work = rules_bound.saturating_mul(triples);
    if pairs > config.solid_authz_trust_max_pairs {
        return json_response(
            StatusCode::FORBIDDEN,
            trust_deny_json("trust rule/credential pair limit exceeded"),
        );
    }
    if certifications.len() > config.solid_authz_trust_max_certifications {
        return json_response(
            StatusCode::FORBIDDEN,
            trust_deny_json("trust certification limit exceeded"),
        );
    }
    if validation_work > config.solid_authz_trust_max_validation_work {
        return json_response(
            StatusCode::FORBIDDEN,
            trust_deny_json("trust validation-work limit exceeded"),
        );
    }

    // ── 5. Run the cert-graph closure ────────────────────────────────────────
    // Depth-1 closure: derived rules are NOT re-used as anchors (sq-tu4e discipline).
    let pre_count = direct_rules.len();
    // Test observability counts entry to the verification-bearing library calls.
    #[cfg(test)]
    verification_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let effective_rules = derive_effective_rules(&direct_rules, &certifications, now_unix_secs, 1);
    let cert_graph_derived = effective_rules.len() > pre_count;

    let mut all_admitted: Vec<AdmittedFact> = Vec::new();
    for cred in &credentials {
        #[cfg(test)]
        verification_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        all_admitted.extend(admit(
            cred,
            &effective_rules,
            &trust_session,
            &resource_node,
        ));
    }

    // ── 7. Build the WAC/ACP store and decide ───────────────────────────────
    // The trust block extends, never replaces, the WAC/ACP admission stratum: admitted facts
    // are injected INTO the pod dataset for materialisation, exactly as the design specifies.
    // Build the store over the original dataset with admitted facts injected.
    let admitted_count = all_admitted.len();
    let admitted_source = all_admitted.first().map(|f| f.issuer.as_str().to_owned());
    let store = match build_store_with_admitted(req, &all_admitted) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let decision = store.decide(&req.session(), resource, mode);
    let status = if decision.allow {
        StatusCode::OK
    } else {
        deny_status_code(decision.status)
    };

    // ── 8. Emit the response with a minimal admission justification ──────────
    let justification = admitted_source.map(|first_source| TrustJustification {
        admitted_source: first_source,
        admitted_count,
        cert_graph_derived,
    });
    let mut body_obj = match serde_json::from_str::<serde_json::Value>(&decision_to_json(&decision)) {
        Ok(v) => v,
        Err(_) => {
            // decision_to_json is infallible for valid WacDecision; this branch is unreachable.
            return json_response(StatusCode::FORBIDDEN, trust_deny_json("internal serialisation error"));
        }
    };
    if let Some(j) = justification {
        if let serde_json::Value::Object(ref mut map) = body_obj {
            map.insert("trustJustification".to_owned(), justification_to_json(&j));
        }
    }
    let acl_link = decision.acl_link_header();
    json_response_with_link(status, body_obj.to_string(), acl_link.as_deref())
}

/// (sq-pfae.17) Build a `PodStore` with admitted trust facts injected into the
/// dataset. Admitted facts are emitted into TWO positions so they reach the WAC/ACP materialiser:
///
/// 1. **Named-graph position** (the subject's document, e.g. `<https://ex/g>` for a fact about
///    `<https://ex/g#team>`) — the WAC loader reads a group document named by an
///    `acl:agentGroup` reference and keeps only its statements about its own nodes, so a
///    membership fact about group `G` reaches the reasoner from `G`'s document.
///
/// 2. **Default-graph position** (no graph IRI) — belt-and-braces; the engine SPARQL query path
///    (`query_as` / `view_for`) sees the default graph alongside named graphs so trust-injected
///    facts are also queryable as plain RDF terms. Default-graph quads do NOT reach the WAC N3
///    reasoner (the loader skips them), so position 1 is the operative path for WAC decisions.
///
/// FIX (Fix 2, sq-zza2h): oxrdf's Display for `NamedOrBlankNode` (subject) and
/// `NamedNode` (predicate) already emits the `<…>` / `_:…` wrapper, and `Term` (object) emits
/// the correct N-Quads term. Do NOT re-wrap in `<{}>` — that would double-wrap a `NamedNode`
/// subject as `<<…>>`, producing invalid N-Quads that the parser rejects, so admitted facts
/// silently never reached the store before this fix. Emit `{} {} {} .` / `{} {} {} <graph> .`
/// and let oxrdf Display produce the canonical N-Quads term syntax.
///
/// HONEST SCOPE: this is a PoC injection path (anchored-not-proven; sq-qhy4 pending external
/// audit). A production wiring would use `trust:admitted`-named-graph tagging and epoch-based
/// cache invalidation (design `research/solid-trust-graph-authz-design.md`).
#[cfg(feature = "solid-authz-trust")]
#[allow(clippy::result_large_err)]
fn build_store_with_admitted(
    req: &AuthzRequest,
    admitted: &[sparq_trust::AdmittedFact],
) -> Result<sparq_solid::PodStore, Response> {
    use std::fmt::Write as _;
    // POSITIONAL format args throughout (CodeQL rust/unused-variable guard).
    let mut augmented = req.dataset.clone();
    for fact in admitted {
        // Named-graph position: the document of the fact's subject (a group IRI's group
        // document, which may itself be an `.acl`), the only document whose statements about
        // that node the loader keeps. The loader applies the same confinement to it as to the
        // rest of that document.
        if let oxrdf::NamedOrBlankNode::NamedNode(group) = &fact.triple.subject {
            let doc = group.as_str().split('#').next().unwrap_or(group.as_str());
            let _ = writeln!(
                &mut augmented,
                "{} {} {} <{}> .",
                fact.triple.subject,
                fact.triple.predicate,
                fact.triple.object,
                doc,
            );
        }
        // Default-graph position: reachable via SPARQL query on the full dataset.
        let _ = writeln!(
            &mut augmented,
            "{} {} {} .",
            fact.triple.subject,
            fact.triple.predicate,
            fact.triple.object,
        );
    }
    // Delegate to the unchanged build_store path (WAC/ACP inference + materialisation).
    let augmented_req = AuthzRequest {
        // sq-snopa.8 — the trust lane decides over THIS augmented body dataset; the
        // stateful lane is refused before `trust_authz_decide` is ever reached.
        source: AuthzSource::Body,
        #[cfg(feature = "odrl-authz")]
        carries_odrl: false,
        dataset: augmented,
        agent: req.agent.clone(),
        client: req.client.clone(),
        issuer: req.issuer.clone(),
        now: req.now.clone(),
        // The model is the request's own: inferred from its dataset before any fact is added,
        // so an admitted fact never switches it.
        view: Some(req.view.unwrap_or_else(|| infer_view(&req.dataset))),
    };
    build_store(&augmented_req)
}

/// (sq-pfae.17) Serialise a 403 trust-deny JSON body. The reason string is NEVER
/// echoed to the client verbatim from adversarial input (the caller always passes a static string
/// literal or a format!() from controlled values). POSITIONAL format args (CodeQL guard).
#[cfg(feature = "solid-authz-trust")]
fn trust_deny_json(reason: &str) -> String {
    serde_json::json!({
        "allow": false,
        "grantedModes": [],
        "governingAcl": null,
        "scope": null,
        "status": "resolved",
        "aclLink": null,
        "trustDenied": true,
        "trustDenyReason": reason,
    })
    .to_string()
}

/// `POST /authz/wac-allow` — the `WAC-Allow` header VALUE for a `(session, resource)` (FR-2/FR-4).
/// Body: `{ "dataset": nquads, "session": {..}, "resource": iri, "view"?: "wac"|"acp",
/// "source"?: "body"|"server" }` (sq-snopa.8: `"server"` omits `"dataset"` and
/// advertises over the server's own loaded store). Returns
/// `{ "wacAllow": "user=\"…\",public=\"…\"" }` — the RFC-style permission advertisement the server
/// puts in a `WAC-Allow` response header.
///
/// Fail-closed: an unparseable dataset / materialisation failure is an error (never a wider
/// advertisement); a grant-less session yields `user="",public=""`.
///
/// sq-3mu76: behind the `odrl-authz` feature, a dataset carrying ODRL policy rules
/// runs the [`apply_odrl_lane`] materialisation (as `Mode::Read` — the endpoint has no mode
/// field and the lane evidences only the read action) before advertising, and the value is
/// scoped by [`read_scoped_wac_allow`]: a read prohibition is never advertised past, `user`
/// carries at most `read`, `public` is always empty (module doc). Same refusal matrix
/// (anonymous session => 403 etc.).
pub(crate) async fn wac_allow_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().solid_authz {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let req = match parse_request(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let v: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "request body is not valid JSON"),
    };
    let resource_iri = match v.get("resource").and_then(|r| r.as_str()) {
        Some(r) => r.to_owned(),
        None => {
            return json_error(StatusCode::BAD_REQUEST, "'resource' (IRI string) is required");
        }
    };
    // `wac_allow` takes a `NamedNode`; an un-IRI resource is a fail-closed 400 (never a grant).
    let resource = match oxrdf::NamedNode::new(&resource_iri) {
        Ok(n) => n,
        Err(_) => {
            return json_error(StatusCode::BAD_REQUEST, "'resource' is not a valid IRI");
        }
    };
    let store = match resolve_store(&state, &req, Mode::Read).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    #[cfg(feature = "odrl-authz")]
    let odrl_lane_fired = req.source == AuthzSource::Body && req.carries_odrl;
    // sq-snopa.7 FR-5: resolve the governing ACL for the Link header BEFORE
    // `wac_allow` consumes the store. `resolve_acl` is a pure index lookup (no mode needed) and
    // returns `None` when no ACL was discovered — fail-closed: no header emitted in that case.
    // POSITIONAL format args (CodeQL rust/unused-variable guard).
    let acl_link: Option<String> = store
        .get()
        .resolve_acl(&resource_iri)
        .map(|eff| format!("<{}>; rel=\"acl\"", eff.acl.as_str()));
    let value = store.get().wac_allow(&req.session(), &resource);
    // sq-3mu76 — read-scoped advertisement (module doc): the lane materialised
    // read-action outcomes only, so advertise at most `user="read"` and never a public mode
    // (an anonymous session is refused wherever the lane fires). Fail-closed: this can only
    // NARROW the advertisement, never widen it.
    #[cfg(feature = "odrl-authz")]
    let value = if odrl_lane_fired {
        read_scoped_wac_allow(&value)
    } else {
        value
    };
    let body = serde_json::json!({ "wacAllow": value }).to_string();
    json_response_with_link(StatusCode::OK, body, acl_link.as_deref())
}

// ── sq-lrtc3.1 + sq-3mu76 — the OPT-IN ODRL lane on `/authz/*` ───
//
// Everything below is compiled ONLY behind the `odrl-authz` feature (which implies
// `solid-authz` + `sparq-solid/odrl-bridge` + `sparq-policy`). Feature OFF => zero
// compiled code, byte-identical to `solid-authz`.
//
// FAIL-CLOSED INVARIANT: every refusal path returns a 4xx and materialises nothing —
// NEVER a silent allow. In particular a prohibition must never be silently DROPPED
// (dropping a deny widens access), so any input the lane cannot faithfully enforce
// (a targetless rule, a non-read mode, an anonymous session, an unimplementable
// `odrl:conflict` strategy, an unparseable policy) refuses the whole request.
//
// The lane deliberately uses the ONE-SHOT bridge entry point
// (`PodStore::materialize_odrl_policy`) rather than the persisted-conditional
// variants: the stateless per-request store dies with the response, so
// frozen-at-materialization and re-checked-per-session semantics coincide here —
// and the one-shot path evaluates `odrl:assignee` against the request party
// (the conditional path drops a bare-assignee restriction to an `auth:Public`
// head — a widening hazard tracked as its own sparq-solid bug bead, unsuitable
// for an enforcement surface).

/// Does the request dataset carry ODRL policy rules? A cheap substring scan for the
/// `odrl:permission` / `odrl:prohibition` predicate IRIs in N-Quads term brackets —
/// the same suffix-scan convenience shape as [`infer_view_is_acp`]. A false positive
/// (the IRI inside a literal) parses to a rule-less policy, which materialises
/// nothing — the lane then behaves exactly as if it had not fired (fail-closed
/// direction). Pure + directly unit-tested. sq-lrtc3.1.
#[cfg(feature = "odrl-authz")]
pub(crate) fn dataset_carries_odrl(dataset: &str) -> bool {
    dataset.contains("http://www.w3.org/ns/odrl/2/permission>")
        || dataset.contains("http://www.w3.org/ns/odrl/2/prohibition>")
}

/// The ODRL action IRI the query lane exercises for a WAC/ACP [`Mode`]: a SPARQL
/// query is an `odrl:read` exercise, so ONLY [`Mode::Read`] maps; every other mode is
/// `None` and the lane refuses (fail-closed — the full query-action contract,
/// including the deliberate non-mapping of `odrl:use`, is sq-lrtc3.2). Pure +
/// directly unit-tested. sq-lrtc3.1.
#[cfg(feature = "odrl-authz")]
pub(crate) fn odrl_action_for_mode(mode: Mode) -> Option<&'static str> {
    match mode {
        Mode::Read => Some("http://www.w3.org/ns/odrl/2/read"),
        Mode::Write | Mode::Append | Mode::Control => None,
    }
}

/// Mask an advisory decision's `granted_modes` to the SINGLE mode the ODRL lane evidenced
/// (the requested read mode) — the read-scoped-advertisement rule (module doc, sq-3mu76).
/// The `allow` bit is untouched: it concerns exactly the requested mode, which is never
/// masked out. Fail-closed direction only (a mode is removed from the advertisement, never
/// added). Pure + directly unit-tested. sq-3mu76.
#[cfg(feature = "odrl-authz")]
pub(crate) fn mask_decision_to_mode(decision: &mut WacDecision, mode: Mode) {
    decision.granted_modes.retain(|m| *m == mode);
}

/// Scope a [`sparq_solid::PodStore::wac_allow`] advertisement value down to what the ODRL
/// lane evidenced (the read-scoped-advertisement rule, module doc, sq-3mu76): `user` keeps at
/// most `read`, `public` is always empty (an anonymous session is REFUSED wherever the lane
/// fires, so no public mode is ever evidenced under an ODRL policy). The input is the
/// crate-controlled fixed format `user="<modes>",public="<modes>"` where `<modes>` is a
/// space-separated subset of the four mode names — `read` can only occur as the read-mode
/// token, so the substring test is exact. Fail-closed: the output advertises a subset of the
/// input's `user` field and nothing public. Pure + directly unit-tested. sq-3mu76.
#[cfg(feature = "odrl-authz")]
pub(crate) fn read_scoped_wac_allow(value: &str) -> String {
    let user_holds_read = value
        .split_once(",public=")
        .map(|(user, _)| user.contains("read"))
        .unwrap_or(false);
    let user = if user_holds_read { "read" } else { "" };
    format!(r#"user="{}",public="""#, user)
}

/// Union-project the request dataset's quads to TRIPLES so ODRL policies are parsed
/// wherever they live (the default graph OR a named graph). `sparq_policy::parse_policy`
/// queries bare triple patterns — the DEFAULT graph only — so a policy expressed inside
/// a named graph would otherwise be INVISIBLE, silently dropping its prohibitions
/// (fail-OPEN); the union projection makes it visible. Blank-node labels are
/// document-scoped in N-Quads, so the projection preserves node identity across graph
/// boundaries. Directly unit-tested. sq-lrtc3.1.
#[cfg(feature = "odrl-authz")]
fn union_policy_graph(dataset: &str) -> Result<sparq_core::Graph, String> {
    use std::fmt::Write as _;
    let mut nt = String::new();
    for quad in oxttl::NQuadsParser::new().for_slice(dataset.as_bytes()) {
        let quad = quad.map_err(|e| e.to_string())?;
        // oxrdf Display emits canonical N-Triples term syntax (no re-wrapping — the
        // sq-zza2h double-wrap lesson); the graph name is deliberately dropped.
        let _ = writeln!(&mut nt, "{} {} {} .", quad.subject, quad.predicate, quad.object);
    }
    sparq_core::Graph::load_str(&nt, "ntriples")
}

/// The most (rule, evaluated target) pairs one ODRL lane request may evaluate, where the
/// evaluated targets are the rule targets plus their stated collection members. A larger
/// request is refused rather than evaluated in part.
#[cfg(feature = "odrl-authz")]
pub(crate) const MAX_ODRL_EVALUATIONS: usize = 10_000;

/// Run the ODRL lane over an already WAC/ACP-materialised `store`: parse the dataset's
/// ODRL policy and materialise its bridged grants/denies (both sides, deny-overrides)
/// into the auth view for the request's `(party, action, target)` per rule target.
/// Returns the fail-closed 4xx [`Response`] on any input the lane cannot faithfully
/// enforce (see the section comment above); `Ok(())` means every materialisation
/// outcome is now in the auth view and `query_json_as` / `decide` / `wac_allow` (all
/// three consume the same `∪ allow ∖ ∪ deny` index) can honour it.
/// sq-lrtc3.1 + sq-3mu76.
///
/// What the stateless lane can and cannot evidence (honest scope): the request context
/// carries the party (session agent), the delivery recipient (the same agent — the
/// query results go to the requester), and `odrl:dateTime` (the session `now`, when
/// supplied). A constraint dimension with no evidence (`odrl:purpose`, `odrl:count` —
/// stateful budgets are sq-snopa.8 territory) or an undischarged duty never grants
/// (the evaluator is fail-closed on missing evidence); it is never silently widened.
// `Response` is large; same `result_large_err` allow as the crate's other handler helpers.
#[cfg(feature = "odrl-authz")]
#[allow(clippy::result_large_err)]
fn apply_odrl_lane(store: &mut PodStore, req: &AuthzRequest, mode: Mode) -> Result<(), Response> {
    // 1. The query-action contract: only `read` maps (sq-lrtc3.2). Refuse any other
    //    mode rather than guess an ODRL action for it (fail-closed).
    let Some(action) = odrl_action_for_mode(mode) else {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "the dataset carries ODRL policies; the ODRL lane covers the read/query \
             action only (fail-closed)",
        ));
    };
    // 2. A concrete party: bridged grants AND denies are principal-scoped triples, so
    //    an anonymous session would leave an applicable prohibition unenforceable
    //    against any static public grant (fail-OPEN). Refuse instead.
    let Some(agent) = req.agent.as_deref() else {
        return Err(json_error(
            StatusCode::FORBIDDEN,
            "the dataset carries ODRL policies; the ODRL lane requires an \
             authenticated agent (fail-closed)",
        ));
    };
    // 3. Parse the policy from the UNION of the dataset's graphs. The parse detail is
    //    withheld from the client (info-leak posture), as everywhere on this surface.
    let policy_graph = union_policy_graph(&req.dataset)
        .map_err(|_| json_error(StatusCode::BAD_REQUEST, "'dataset' is not valid N-Quads"))?;
    let policy = sparq_policy::parse_policy(&policy_graph).map_err(|_| {
        json_error(
            StatusCode::BAD_REQUEST,
            "the dataset's ODRL policy did not parse (fail-closed)",
        )
    })?;
    // 4. Refuse an unimplementable `odrl:conflict` strategy UP FRONT: under e.g.
    //    `odrl:perm` NONE of the policy's rules can be trusted into deny-overrides
    //    enforcement, and a rule-less short-circuit below must not skip the check.
    if sparq_policy::conflict_admissibility(&policy).is_err() {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "the ODRL policy declares a conflict strategy this engine cannot honour \
             (fail-closed)",
        ));
    }
    // 5. Graph-granularity targets only: a rule with NO concrete target graph IRI
    //    applies to *any* target, which this lane cannot faithfully enumerate — and
    //    under-materialising a targetless PROHIBITION would widen access. Refuse
    //    (pattern/sub-graph targets are the sq-lrtc3.3 design track).
    let mut targets = std::collections::BTreeSet::new();
    for rule in policy.permissions.iter().chain(policy.prohibitions.iter()) {
        match rule.target.as_deref() {
            Some(t) => {
                targets.insert(t.to_owned());
            }
            None => {
                return Err(json_error(
                    StatusCode::BAD_REQUEST,
                    "an ODRL rule without a concrete target graph IRI is not supported \
                     on this lane (fail-closed; pattern targets are a tracked design \
                     follow-up)",
                ));
            }
        }
    }
    // 6. Materialise BOTH sides of the policy per target, and per member of a target
    //    the dataset places in it (`member odrl:partOf target`, any graph): a rule on an asset
    //    collection matches each of its members, carrying that membership as the
    //    request's asset evidence. Each call is independently fail-closed (a
    //    non-matching / unevidenced rule materialises nothing) and idempotent; the deny
    //    side wins at enforcement (∪ allow ∖ ∪ deny).
    let members = sparq_policy::target_memberships(&policy, &policy_graph).map_err(|_| {
        json_error(
            StatusCode::BAD_REQUEST,
            "the dataset's ODRL policy did not parse (fail-closed)",
        )
    })?;
    // 7. Bound the work before doing it: every (rule, evaluated target) pair is one
    //    evaluation. Over the cap the request is refused, never evaluated in part.
    let rules = policy.permissions.len() + policy.prohibitions.len();
    if (targets.len() + members.len()).saturating_mul(rules) > MAX_ODRL_EVALUATIONS {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "the ODRL policy and its collection memberships exceed this lane's evaluation \
             bound (fail-closed)",
        ));
    }
    let requests: Vec<sparq_policy::Request> = targets
        .iter()
        .map(|t| (t.as_str(), None))
        .chain(members.iter().map(|(m, c)| (m.as_str(), Some(c.as_str()))))
        .map(|(target, collection)| {
            let mut request = sparq_policy::Request::new(action)
                .on(target)
                .by(agent)
                // The requesting agent IS the delivery recipient of the query results.
                .with(
                    sparq_policy::ODRL_RECIPIENT,
                    sparq_policy::Value::Iri(agent.to_owned()),
                );
            if let Some(now) = req.now.as_deref() {
                request = request.at(now);
            }
            if let Some(collection) = collection {
                request = request.with_asset_membership(target, collection);
            }
            request
        })
        .collect();
    // One index rebuild for the whole request, however many targets it evaluates.
    if store
        .materialize_odrl_policy_for_each(&policy, &requests)
        .map_err(|_| json_error(StatusCode::SERVICE_UNAVAILABLE, "authorization view write failed"))?
        .iter()
        .any(|o| o.refused)
    {
        // Defence-in-depth: step 4 already refused inadmissible strategies.
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "the ODRL policy was refused by the bridge (fail-closed)",
        ));
    }
    Ok(())
}

/// `POST /authz/query` — an ACCESS-CONTROLLED SPARQL query as `session` (FR-4). Body:
/// `{ "dataset": nquads, "session": {..}, "mode"?: "read"|…, "query": sparql, "view"?: …,
/// "source"?: "body"|"server" }` (sq-snopa.8: `"server"` omits `"dataset"` and
/// queries the server's own loaded store through the same access-controlled view).
/// Delegates to [`PodStore::query_json_as`] (the zero-copy authorised-graph-set view path) and
/// returns the SPARQL 1.1 JSON results serialisation. `mode` defaults to `read` (the query path).
///
/// Fail-closed: a session with no grants sees an empty view (zero rows), never the whole store; an
/// unparseable dataset / materialisation failure is an error; a query that does not parse is a
/// `400` (the engine's detail withheld from the client).
pub(crate) async fn query_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().solid_authz {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let req = match parse_request(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let v: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "request body is not valid JSON"),
    };
    let query = match v.get("query").and_then(|q| q.as_str()) {
        Some(q) => q.to_owned(),
        None => {
            return json_error(StatusCode::BAD_REQUEST, "'query' (SPARQL string) is required");
        }
    };
    // A query is a READ; default the mode to Read. An explicit unknown mode is a fail-closed 400
    // (we will not guess a wider mode).
    let mode = match v.get("mode").and_then(|m| m.as_str()) {
        None => Mode::Read,
        Some(s) => match parse_mode(s) {
            Some(m) => m,
            None => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "'mode' must be 'read', 'write', 'append' or 'control'",
                );
            }
        },
    };
    let store = match resolve_store(&state, &req, mode).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match store.get().query_json_as(&req.session(), mode, &query) {
        Ok(json) => json_response(StatusCode::OK, json),
        // The engine's parse/eval detail is withheld (info-leak posture); the class is enough.
        Err(_) => json_error(
            StatusCode::BAD_REQUEST,
            "query did not parse or could not be evaluated",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "solid-authz-trust")]
    #[test]
    fn decode_salt_hex_requires_exactly_32_bytes() {
        let ok = "ab".repeat(32);
        assert_eq!(decode_salt_hex(&ok), Some([0xab; 32]));
        assert_eq!(decode_salt_hex(&ok.to_uppercase()), Some([0xab; 32]));
        assert_eq!(decode_salt_hex(&ok[..62]), None);
        assert_eq!(decode_salt_hex(&format!("{ok}ab")), None);
        assert_eq!(decode_salt_hex(&"ab".repeat(4096)), None);
        assert_eq!(decode_salt_hex(&format!("{}zz", &ok[..62])), None);
    }

    // A tiny WAC dataset: alice has Read on the root `.acl`, inherited by `notes/n1`.
    const WAC_NQUADS: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;

    fn wac_store() -> PodStore {
        let mut store =
            PodStore::new(sparq_core::Graph::load_dataset(WAC_NQUADS, "nquads").unwrap());
        store.materialize_wac().unwrap();
        store
    }

    #[test]
    fn deny_status_code_maps_definitive_vs_retryable() {
        // Definitive permission outcomes -> 403.
        assert_eq!(deny_status_code(AclStatus::Resolved), StatusCode::FORBIDDEN);
        assert_eq!(deny_status_code(AclStatus::NoAcl), StatusCode::FORBIDDEN);
        // Retryable operational conditions -> 503.
        assert_eq!(
            deny_status_code(AclStatus::Unloaded),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            deny_status_code(AclStatus::Transient),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn parse_mode_is_case_insensitive_and_fail_closed() {
        assert_eq!(parse_mode("read"), Some(Mode::Read));
        assert_eq!(parse_mode("  WRITE "), Some(Mode::Write));
        assert_eq!(parse_mode("Append"), Some(Mode::Append));
        assert_eq!(parse_mode("control"), Some(Mode::Control));
        // Any unknown keyword -> None (the caller denies).
        assert_eq!(parse_mode("delete"), None);
        assert_eq!(parse_mode(""), None);
    }

    #[test]
    fn mode_name_round_trips_through_parse_mode() {
        for m in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            assert_eq!(parse_mode(mode_name(m)), Some(m));
        }
    }

    #[test]
    fn status_token_covers_every_status() {
        assert_eq!(status_token(AclStatus::Resolved), "resolved");
        assert_eq!(status_token(AclStatus::NoAcl), "noAcl");
        assert_eq!(status_token(AclStatus::Unloaded), "unloaded");
        assert_eq!(status_token(AclStatus::Transient), "transient");
    }

    #[test]
    fn infer_view_detects_acp_by_control_suffix() {
        assert!(infer_view_is_acp(
            "<https://pod.ex/.acr#p> <p> <o> <https://pod.ex/.acr> ."
        ));
        // A pure-WAC dataset infers WAC.
        assert!(!infer_view_is_acp(WAC_NQUADS));
    }

    #[test]
    fn decision_to_json_allow_carries_provenance_and_acl_link() {
        let store = wac_store();
        let alice = Session {
            agent: Some("https://alice.ex/card#me"),
            client: None,
            issuer: None,
            now: None,
        };
        let d = store.decide(&alice, "https://pod.ex/notes/n1", Mode::Read);
        assert!(d.allow);
        let json: serde_json::Value = serde_json::from_str(&decision_to_json(&d)).unwrap();
        assert_eq!(json["allow"], serde_json::Value::Bool(true));
        assert_eq!(json["status"], "resolved");
        assert_eq!(json["governingAcl"], "https://pod.ex/.acl");
        assert_eq!(json["scope"], "http://www.w3.org/ns/auth/acl#default");
        // aclLink is the RFC-8288 header value (feeds sq-snopa.7).
        assert_eq!(json["aclLink"], r#"<https://pod.ex/.acl>; rel="acl""#);
        assert!(json["grantedModes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m == "read"));
    }

    #[test]
    fn decision_to_json_deny_is_fail_closed() {
        // The HTTP layer's own "will not even try" deny (e.g. an unknown mode).
        let json: serde_json::Value =
            serde_json::from_str(&decision_to_json(&fail_closed_decision())).unwrap();
        assert_eq!(json["allow"], serde_json::Value::Bool(false));
        assert_eq!(json["grantedModes"].as_array().unwrap().len(), 0);
        assert_eq!(json["governingAcl"], serde_json::Value::Null);
        assert_eq!(json["aclLink"], serde_json::Value::Null);
    }

    /// sq-3mu76 — the read-scoped-advertisement mask keeps ONLY the requested mode
    /// and never touches the `allow` bit.
    #[cfg(feature = "odrl-authz")]
    #[test]
    fn mask_decision_to_mode_keeps_only_the_requested_mode() {
        let mut d = WacDecision {
            allow: true,
            granted_modes: vec![Mode::Read, Mode::Write, Mode::Control],
            governing_acl: None,
            scope: None,
            status: AclStatus::Resolved,
        };
        mask_decision_to_mode(&mut d, Mode::Read);
        assert_eq!(d.granted_modes, vec![Mode::Read]);
        assert!(d.allow, "the allow bit is untouched by the advertisement mask");
        // A mode the session does not hold is not conjured up (fail-closed direction only).
        let mut none = fail_closed_decision();
        mask_decision_to_mode(&mut none, Mode::Read);
        assert!(none.granted_modes.is_empty());
    }

    /// sq-3mu76 — the wac-allow value is scoped to at most `user="read"`,
    /// `public` always empty, and a `read` in the PUBLIC field never leaks into `user`.
    #[cfg(feature = "odrl-authz")]
    #[test]
    fn read_scoped_wac_allow_scopes_user_and_blanks_public() {
        assert_eq!(
            read_scoped_wac_allow(r#"user="read write append control",public="read""#),
            r#"user="read",public="""#
        );
        // No read held by the user: nothing advertised — the public `read` must NOT leak.
        assert_eq!(
            read_scoped_wac_allow(r#"user="write",public="read""#),
            r#"user="",public="""#
        );
        assert_eq!(read_scoped_wac_allow(r#"user="",public="""#), r#"user="",public="""#);
        // A value not in the crate-controlled format advertises NOTHING (fail-closed).
        assert_eq!(read_scoped_wac_allow("garbage"), r#"user="",public="""#);
    }

    #[test]
    fn parse_request_rejects_absent_dataset_fail_closed() {
        // No dataset -> a 400, NOT an empty-dataset allow.
        let body = Bytes::from(r#"{"session":{"agent":"https://a.ex#me"}}"#);
        let err = parse_request(&body).unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_request_rejects_unknown_view_fail_closed() {
        let body = Bytes::from(r#"{"dataset":"","view":"nonsense"}"#);
        let err = parse_request(&body).unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    /// An admitted fact whose subject's document is named like an access-control resource
    /// leaves a WAC request's inferred model in place.
    #[cfg(feature = "solid-authz-trust")]
    #[test]
    fn admitted_facts_keep_the_requests_model() {
        let acl = "https://pod.ex/.acl";
        let dataset = format!(
            "<{acl}#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <{acl}> .\n\
             <{acl}#r> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <{acl}> .\n\
             <{acl}#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <{acl}> .\n\
             <{acl}#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <{acl}> .\n\
             <https://pod.ex/doc#it> <https://ex.dev/ns#k> \"v\" <https://pod.ex/doc> .\n"
        );
        let body = Bytes::from(
            serde_json::json!({
                "dataset": dataset,
                "session": {"agent": "https://bob.ex/card#me"},
            })
            .to_string(),
        );
        let req = parse_request(&body).unwrap();
        assert_eq!(req.view, None, "the model is inferred");
        let fact = sparq_trust::AdmittedFact {
            triple: oxrdf::Triple::new(
                oxrdf::NamedNode::new("https://pod.ex/g.acr#team").unwrap(),
                oxrdf::NamedNode::new("http://www.w3.org/2006/vcard/ns#hasMember").unwrap(),
                oxrdf::NamedNode::new("https://bob.ex/card#me").unwrap(),
            ),
            issuer: oxrdf::NamedNode::new("https://issuer.ex/").unwrap(),
        };
        let store = super::build_store_with_admitted(&req, &[fact])
            .unwrap_or_else(|_| panic!("the store builds"));
        let session = req.session();
        assert!(
            store
                .decide(&session, "https://pod.ex/doc", sparq_solid::Mode::Read)
                .allow,
            "the WAC rules still decide"
        );
    }

    #[test]
    fn parse_request_extracts_session_and_dataset() {
        let body = Bytes::from(
            r#"{"dataset":"<a> <b> <c> <g> .","session":{"agent":"https://a.ex#me","client":"https://app.ex/id"},"view":"wac"}"#,
        );
        let req = parse_request(&body).unwrap();
        assert_eq!(req.dataset, "<a> <b> <c> <g> .");
        assert_eq!(req.agent.as_deref(), Some("https://a.ex#me"));
        assert_eq!(req.client.as_deref(), Some("https://app.ex/id"));
        assert_eq!(req.issuer, None);
        assert_eq!(req.view, Some(AuthView::Wac));
        // The borrowed session mirrors the parsed fields.
        let s = req.session();
        assert_eq!(s.agent, Some("https://a.ex#me"));
    }

    #[test]
    fn build_store_denies_on_unparseable_dataset() {
        let req = AuthzRequest {
            source: AuthzSource::Body,
            #[cfg(feature = "odrl-authz")]
            carries_odrl: false,
            dataset: "this is not n-quads @@@".to_owned(),
            agent: None,
            client: None,
            issuer: None,
            now: None,
            view: Some(AuthView::Wac),
        };
        // A garbage dataset is a fail-closed 400, never a store that grants.
        // (`PodStore` is not `Debug`, so match rather than `unwrap_err`.)
        match build_store(&req) {
            Ok(_) => panic!("a garbage dataset must NOT build a store"),
            Err(resp) => assert_eq!(resp.status(), StatusCode::BAD_REQUEST),
        }
    }

    #[test]
    fn build_store_then_decide_is_authorized_and_anonymous_fails_closed() {
        let req = AuthzRequest {
            source: AuthzSource::Body,
            #[cfg(feature = "odrl-authz")]
            carries_odrl: false,
            dataset: WAC_NQUADS.to_owned(),
            agent: Some("https://alice.ex/card#me".to_owned()),
            client: None,
            issuer: None,
            now: None,
            view: Some(AuthView::Wac),
        };
        let store = match build_store(&req) {
            Ok(s) => s,
            Err(_) => panic!("the WAC fixture must materialise"),
        };
        // Alice (the parsed session) is granted Read; the decision is authoritative.
        let d = store.decide(&req.session(), "https://pod.ex/notes/n1", Mode::Read);
        assert!(d.allow && d.status == AclStatus::Resolved);
        // An anonymous session over the SAME store is a fail-closed deny.
        let anon = store.decide(&Session::default(), "https://pod.ex/notes/n1", Mode::Read);
        assert!(!anon.allow && anon.status == AclStatus::Resolved);
        // And Write (which alice lacks) is an authoritative deny -> 403, not a 503.
        let dw = store.decide(&req.session(), "https://pod.ex/notes/n1", Mode::Write);
        assert!(!dw.allow);
        assert_eq!(deny_status_code(dw.status), StatusCode::FORBIDDEN);
    }

    /// sq-qonip — the SHIPPED mapper's anonymous lane: a definitive deny to a requester who
    /// presented no WebID goes through [`deny_status_code`] to a **403**, identical to the
    /// authenticated deny, for both definitive statuses (`Resolved`-without-the-mode and
    /// `NoAcl`). [`deny_status_code`] takes only an [`AclStatus`], so it *cannot* branch on
    /// authentication — this pins that consequence on the real function rather than on a
    /// re-implementation of it.
    ///
    /// This is correct for `POST /authz/decide` (it reports a decision; the caller's own
    /// authentication is gated separately, with a 401 + `WWW-Authenticate`). It is a **known
    /// non-conformance** for any LDP resource-serving path reusing this mapper: Solid
    /// requires a 401 challenge when a request lacks the credentials a protected resource
    /// needs. Documented on `sparq_solid::AclStatus`; see the follow-up for the 401 lane.
    #[test]
    fn shipped_mapper_gives_anonymous_definitive_denies_the_same_403() {
        let store = wac_store();
        let alice = Session {
            agent: Some("https://alice.ex/card#me"),
            ..Session::default()
        };

        // `Resolved` deny: the ACL governs `notes/n1`, but grants an anonymous requester
        // nothing. Alice's counterpart is her missing Write.
        let anon_resolved = store.decide(&Session::default(), "https://pod.ex/notes/n1", Mode::Read);
        let auth_resolved = store.decide(&alice, "https://pod.ex/notes/n1", Mode::Write);
        // `NoAcl`: a pod with no discoverable access-control document anywhere up the chain.
        let no_acl_store = {
            let mut s = PodStore::new(
                sparq_core::Graph::load_dataset(
                    r#"<https://pod.ex/d#it> <https://ex.dev/ns#t> "x" <https://pod.ex/d> ."#,
                    "nquads",
                )
                .unwrap(),
            );
            s.materialize_wac().unwrap();
            s
        };
        let anon_no_acl = no_acl_store.decide(&Session::default(), "https://pod.ex/d", Mode::Read);
        let auth_no_acl = no_acl_store.decide(&alice, "https://pod.ex/d", Mode::Read);

        assert_eq!(anon_resolved.status, AclStatus::Resolved);
        assert_eq!(anon_no_acl.status, AclStatus::NoAcl);
        for (name, d) in [
            ("Resolved, anonymous", &anon_resolved),
            ("NoAcl, anonymous", &anon_no_acl),
            ("Resolved, authenticated", &auth_resolved),
            ("NoAcl, authenticated", &auth_no_acl),
        ] {
            assert!(!d.allow, "[{}] fail-closed", name);
            assert_eq!(
                deny_status_code(d.status),
                StatusCode::FORBIDDEN,
                "[{}] the shipped mapper emits a definitive 403, never a 401 or a 503",
                name
            );
        }
    }

    // ── sq-snopa.8 — the stateful (`"source":"server"`) lane ──

    /// The default is the historical stateless lane: no `"source"` => `Body`, dataset required.
    #[test]
    fn parse_request_defaults_to_the_stateless_body_source() {
        let body = Bytes::from(r#"{"dataset":"<a> <b> <c> <g> ."}"#);
        let req = parse_request(&body).unwrap();
        assert_eq!(req.source, AuthzSource::Body);
        assert_eq!(req.dataset, "<a> <b> <c> <g> .");
    }

    /// `"source":"server"` needs no dataset — it authorises over the server's own store.
    #[test]
    fn parse_request_accepts_server_source_without_a_dataset() {
        let body = Bytes::from(r#"{"source":"SERVER","session":{"agent":"https://a.ex#me"}}"#);
        let req = parse_request(&body).unwrap();
        assert_eq!(req.source, AuthzSource::Server);
        assert!(
            req.dataset.is_empty(),
            "the stateful lane must carry no body dataset"
        );
        assert_eq!(req.agent.as_deref(), Some("https://a.ex#me"));
    }

    /// A body naming BOTH pods is ambiguous — refused, never silently resolved to one of them.
    #[test]
    fn parse_request_rejects_server_source_carrying_a_dataset_fail_closed() {
        let body = Bytes::from(r#"{"source":"server","dataset":"<a> <b> <c> <g> ."}"#);
        match parse_request(&body) {
            Ok(_) => panic!("a body carrying both a dataset and source:server must be refused"),
            Err(resp) => assert_eq!(resp.status(), StatusCode::BAD_REQUEST),
        }
    }

    /// An unknown `"source"` keyword is a fail-closed client error, not an inferred default.
    #[test]
    fn parse_request_rejects_unknown_source_fail_closed() {
        let body = Bytes::from(r#"{"source":"whatever"}"#);
        match parse_request(&body) {
            Ok(_) => panic!("an unknown source keyword must be refused"),
            Err(resp) => assert_eq!(resp.status(), StatusCode::BAD_REQUEST),
        }
    }

    /// A PRESENT but non-string `"source"` must not be read as an absent key. Each of these bodies
    /// carries an otherwise-valid `dataset`, so the pre-fix `as_str()` collapse would have parsed
    /// cleanly into the body lane; the selector is fail-closed, so every non-string type is a 400.
    #[test]
    fn parse_request_rejects_non_string_source_fail_closed() {
        for body in [
            r#"{"source":null,"dataset":"<a> <b> <c> <g> ."}"#,
            r#"{"source":false,"dataset":"<a> <b> <c> <g> ."}"#,
            r#"{"source":0,"dataset":"<a> <b> <c> <g> ."}"#,
            r#"{"source":["body"],"dataset":"<a> <b> <c> <g> ."}"#,
            r#"{"source":{"kind":"body"},"dataset":"<a> <b> <c> <g> ."}"#,
        ] {
            match parse_request(&Bytes::from(body)) {
                Ok(req) => panic!(
                    "a non-string 'source' must be refused, not defaulted to {:?}: {}",
                    req.source, body
                ),
                Err(resp) => assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{}", body),
            }
        }
    }

    /// The graph-level ACP inference: a `.acr` named graph selects ACP, a `.acl` one leaves WAC.
    #[test]
    fn graph_view_is_acp_detects_acr_named_graphs() {
        let acp = sparq_core::Graph::load_dataset(
            "<https://pod.ex/x#i> <https://ex.dev/ns#p> \"v\" <https://pod.ex/x.acr> .",
            "nquads",
        )
        .unwrap();
        assert!(graph_view_is_acp(&acp));
        let wac = sparq_core::Graph::load_dataset(WAC_NQUADS, "nquads").unwrap();
        assert!(
            !graph_view_is_acp(&wac),
            "a WAC pod must fall back to the WAC view"
        );
    }

    /// The refusal KIND survives the materialisation error instead of being discarded by a
    /// `map_err(|_| …)`: a refusal by the authorization allowance or the input budget is
    /// carried out as its typed kind, while any other failure carries none.
    #[test]
    fn a_materialisation_refusal_carries_its_limit_kind() {
        // A real refusal, as `sparq-solid` produces it (allowance text + kind tag + a detail
        // quoting the document it refused).
        let mut budget = sparq_solid::default_auth_budget();
        budget.max_input_facts = 0;
        let acl = "https://pod.ex/.acl";
        let nq = format!(
            "<{acl}#a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type>              <http://www.w3.org/ns/auth/acl#Authorization> <{acl}> .\n"
        );
        let mut store = PodStore::new(sparq_core::Graph::load_dataset(&nq, "nquads").unwrap());
        store.set_auth_budget(budget);
        let err = store
            .materialize_wac()
            .expect_err("a zero-fact input budget refuses");

        let kind = materialize_refusal_kind(&err);
        assert_eq!(
            kind,
            Some(AuthAllowance::InputFacts),
            "the kind must survive the error: {}",
            err
        );
        assert_eq!(refusal_limit_field(kind), "max_input_facts");
        // The response is unchanged: still the fail-closed, retryable 503.
        assert_eq!(
            server_view_error_response(ServerViewError::Materialize(kind)).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        // Any other materialisation failure carries no kind, and is logged as such rather
        // than with its own text.
        assert_eq!(materialize_refusal_kind("the reasoner failed"), None);
        assert_eq!(refusal_limit_field(None), "none");
    }

    /// What the log line can say is a CLOSED set of compile-time strings, so no pod IRI,
    /// request field or refusal detail can reach the log through it — even though the
    /// refusal's own message quotes the document it refused.
    #[test]
    fn the_logged_limit_field_cannot_carry_request_data() {
        let detail = "https://private.pod.example/secret-container/.acl";
        let err = format!(
            "authorization reasoning allowance exceeded [allowance=input-bytes]: the \
             access-control document <{detail}> contributes more than max_document_bytes"
        );
        let field = refusal_limit_field(materialize_refusal_kind(&err));
        assert_eq!(field, "max_document_bytes/max_input_bytes");
        assert!(
            !field.contains("private") && !field.contains("secret") && !field.contains("://"),
            "the logged field leaked the refusal's detail: {}",
            field
        );
        // For ANY tag — including one this build does not know — the field is one of a
        // closed set of compile-time strings, never anything taken from the message.
        let allowed = [
            "max_total_join_rows",
            "max_join_rows",
            "max_rounds/max_facts",
            "max_term_bytes",
            "max_document_bytes/max_input_bytes",
            "max_input_facts",
            "max_input_term_bytes",
            "max_decision_join_rows/max_decision_facts",
            "an unnamed limit",
            "none",
        ];
        for e in [
            "x [allowance=join-rows] y",
            "x [allowance=decision-work] y",
            "x [allowance=a-tag-from-a-newer-build] y",
            "x [allowance=https://private.pod.example/leak] y",
            "no tag at all",
        ] {
            let f = refusal_limit_field(materialize_refusal_kind(e));
            assert!(
                allowed.contains(&f),
                "unexpected logged field {:?} for {:?}",
                f,
                e
            );
        }
    }

    /// A materialisation failure is the retryable 503; the stateless lane's mapping, kept.
    #[test]
    fn server_view_error_maps_materialize_to_retryable_503() {
        assert_eq!(
            server_view_error_response(ServerViewError::Materialize(None)).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

}

// ── sq-lrtc3.1 — unit tests for the ODRL lane helpers ─────────────
// One DIRECT test per new fn (coverage ratchet), plus the lane's fail-closed
// refusal matrix and a positive/negative enforcement round-trip through the REAL
// store. Gated on the `odrl-authz` feature (zero code otherwise).
#[cfg(all(test, feature = "odrl-authz"))]
mod odrl_tests {
    use super::*;

    const ALICE: &str = "https://alice.ex/card#me";
    const BOB: &str = "https://bob.ex/card#me";
    const N1: &str = "https://pod.ex/notes/n1";

    /// One content graph, no static ACL — the only possible grants are bridged ones.
    const CONTENT: &str = "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .\n";

    /// An ODRL permit for `agent` to read n1, as default-graph N-Quads.
    fn permit_nq(agent: &str) -> String {
        format!(
            "<urn:pol/p> <http://www.w3.org/ns/odrl/2/permission> _:perm .\n\
             _:perm <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> .\n\
             _:perm <http://www.w3.org/ns/odrl/2/target> <{N1}> .\n\
             _:perm <http://www.w3.org/ns/odrl/2/assignee> <{agent}> .\n"
        )
    }

    /// An ODRL prohibition carving `agent` out of reading n1, as default-graph N-Quads.
    fn prohibit_nq(agent: &str) -> String {
        format!(
            "<urn:pol/p> <http://www.w3.org/ns/odrl/2/prohibition> _:proh .\n\
             _:proh <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> .\n\
             _:proh <http://www.w3.org/ns/odrl/2/target> <{N1}> .\n\
             _:proh <http://www.w3.org/ns/odrl/2/assignee> <{agent}> .\n"
        )
    }

    fn authz_req(dataset: String, agent: Option<&str>) -> AuthzRequest {
        AuthzRequest {
            source: AuthzSource::Body,
            #[cfg(feature = "odrl-authz")]
            carries_odrl: dataset_carries_odrl(&dataset),
            dataset,
            agent: agent.map(str::to_owned),
            client: None,
            issuer: None,
            now: None,
            view: Some(AuthView::Wac),
        }
    }

    /// Build the store exactly as the endpoint does, then run the lane.
    // Same `result_large_err` allow as the handler helpers under test.
    #[allow(clippy::result_large_err)]
    fn lane(req: &AuthzRequest, mode: Mode) -> Result<PodStore, Response> {
        let mut store = build_store(req).expect("fixture materialises");
        apply_odrl_lane(&mut store, req, mode)?;
        Ok(store)
    }

    fn reads_n1(store: &PodStore, agent: &str) -> bool {
        let s = Session { agent: Some(agent), client: None, issuer: None, now: None };
        store.accessible(&s, Mode::Read).iter().any(|g| g.as_str() == N1)
    }

    #[test]
    fn dataset_carries_odrl_detects_rule_predicates() {
        assert!(dataset_carries_odrl(&permit_nq(ALICE)));
        assert!(dataset_carries_odrl(&prohibit_nq(ALICE)));
        // A WAC-only dataset does not fire the lane.
        assert!(!dataset_carries_odrl(CONTENT));
        assert!(!dataset_carries_odrl(""));
    }

    #[test]
    fn odrl_action_for_mode_is_read_only_fail_closed() {
        assert_eq!(
            odrl_action_for_mode(Mode::Read),
            Some("http://www.w3.org/ns/odrl/2/read")
        );
        // Every non-read mode is unmapped => the lane refuses (fail-closed).
        assert_eq!(odrl_action_for_mode(Mode::Write), None);
        assert_eq!(odrl_action_for_mode(Mode::Append), None);
        assert_eq!(odrl_action_for_mode(Mode::Control), None);
    }

    #[test]
    fn union_policy_graph_sees_named_graph_policies() {
        // The SAME permit, but tucked inside a named graph — parse_policy over the raw
        // dataset would MISS it (bare patterns match the default graph only); the
        // union projection must surface it.
        let named = permit_nq(ALICE)
            .lines()
            .map(|l| format!("{} <https://pod.ex/policies>{}\n", &l[..l.len() - 2], " ."))
            .collect::<String>();
        let g = union_policy_graph(&named).expect("projection parses");
        let policy = sparq_policy::parse_policy(&g).expect("policy parses");
        assert_eq!(policy.permissions.len(), 1, "named-graph rule must be visible");
        assert_eq!(policy.permissions[0].target.as_deref(), Some(N1));
    }

    #[test]
    fn lane_permit_grants_assignee_only() {
        let req = authz_req(format!("{CONTENT}{}", permit_nq(ALICE)), Some(ALICE));
        let store = lane(&req, Mode::Read).expect("lane applies");
        // alice (the assignee) reads through the REAL enforcement path…
        assert!(reads_n1(&store, ALICE));
        // …and the grant is scoped: bob + anonymous see nothing (fail-closed).
        assert!(!reads_n1(&store, BOB));
        assert!(store.accessible(&Session::default(), Mode::Read).is_empty());

        // The SAME dataset requested BY bob materialises nothing for bob.
        let req_bob = authz_req(format!("{CONTENT}{}", permit_nq(ALICE)), Some(BOB));
        let store_bob = lane(&req_bob, Mode::Read).expect("lane applies");
        assert!(!reads_n1(&store_bob, BOB));
    }

    #[test]
    fn lane_prohibition_overrides_static_wac_grant() {
        // alice holds a STATIC WAC grant on n1; the ODRL prohibition must beat it
        // (deny-overrides through the unchanged ∪ allow ∖ ∪ deny enforcement).
        let wac = r#"<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;
        // WITHOUT the ODRL prohibition the WAC grant admits alice (guards vacuity).
        let req_wac = authz_req(wac.to_owned(), Some(ALICE));
        let store_wac = build_store(&req_wac).expect("fixture materialises");
        assert!(reads_n1(&store_wac, ALICE), "static WAC grant must admit alice");
        // WITH it, the bridged deny wins.
        let req = authz_req(format!("{wac}{}", prohibit_nq(ALICE)), Some(ALICE));
        let store = lane(&req, Mode::Read).expect("lane applies");
        assert!(!reads_n1(&store, ALICE), "ODRL prohibition must beat the WAC grant");
    }

    #[test]
    fn lane_refuses_anonymous_fail_closed() {
        let req = authz_req(format!("{CONTENT}{}", prohibit_nq(ALICE)), None);
        let err = lane(&req, Mode::Read).err().expect("anonymous must refuse");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn lane_refuses_non_read_mode_fail_closed() {
        let req = authz_req(format!("{CONTENT}{}", permit_nq(ALICE)), Some(ALICE));
        for mode in [Mode::Write, Mode::Append, Mode::Control] {
            let err = lane(&req, mode).err().expect("non-read mode must refuse");
            assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn lane_refuses_targetless_rule_fail_closed() {
        // A prohibition with NO odrl:target applies to any target; silently
        // under-materialising it would WIDEN access → the lane must refuse.
        let targetless = "<urn:pol/p> <http://www.w3.org/ns/odrl/2/prohibition> _:proh .\n\
             _:proh <http://www.w3.org/ns/odrl/2/action> <http://www.w3.org/ns/odrl/2/read> .\n\
             _:proh <http://www.w3.org/ns/odrl/2/assignee> <https://alice.ex/card#me> .\n";
        let req = authz_req(format!("{CONTENT}{targetless}"), Some(ALICE));
        let err = lane(&req, Mode::Read).err().expect("targetless rule must refuse");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn lane_refuses_unimplementable_conflict_strategy_fail_closed() {
        // odrl:perm (permissions override prohibitions) is unrepresentable under the
        // bridge's deny-overrides enforcement → the whole request refuses.
        let perm_strategy = format!(
            "{}\
             <urn:pol/p> <http://www.w3.org/ns/odrl/2/conflict> <http://www.w3.org/ns/odrl/2/perm> .\n",
            permit_nq(ALICE)
        );
        let req = authz_req(format!("{CONTENT}{perm_strategy}"), Some(ALICE));
        let err = lane(&req, Mode::Read).err().expect("odrl:perm must refuse");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn lane_ignores_marker_inside_literal_and_stays_wac_only() {
        // The predicate IRI appearing INSIDE A LITERAL is a detection false positive:
        // the parsed policy has no rules, nothing materialises, and the WAC-only
        // behaviour is preserved (the fail-closed direction).
        let literal_marker = "<urn:x> <urn:note> \"see http://www.w3.org/ns/odrl/2/permission> docs\" .\n";
        let dataset = format!("{CONTENT}{literal_marker}");
        assert!(dataset_carries_odrl(&dataset), "scan fires on the literal");
        let req = authz_req(dataset, Some(ALICE));
        let store = lane(&req, Mode::Read).expect("rule-less policy is a no-op");
        assert!(!reads_n1(&store, ALICE), "no grant appears from a literal marker");
    }

    // ── sq-snopa.8 — the stateful lane's ODRL refusal ────────

    /// A server store carrying ODRL rules is DETECTED, so the stateful lane can REFUSE rather
    /// than silently un-enforce a prohibition (which would be fail-OPEN); a plain WAC pod is
    /// not a false positive.
    #[test]
    fn graph_carries_odrl_detects_rules_and_spares_a_plain_pod() {
        // Rules in the DEFAULT graph.
        let dataset = format!("{}{}", CONTENT, prohibit_nq(ALICE));
        let with_rules = sparq_core::Graph::load_dataset(&dataset, "nquads").unwrap();
        assert!(graph_carries_odrl(&with_rules));

        // Rules in a NAMED graph — `union_policy_graph` reads those too, so the refusal must
        // see them as well (this is the `graph.named` arm of the scan).
        let named = format!(
            "{}<urn:pol/p> <http://www.w3.org/ns/odrl/2/prohibition> _:proh <urn:g/policies> .\n",
            CONTENT
        );
        let in_named = sparq_core::Graph::load_dataset(&named, "nquads").unwrap();
        assert!(
            graph_carries_odrl(&in_named),
            "a rule hidden in a named graph must still be detected"
        );

        let plain = sparq_core::Graph::load_dataset(CONTENT, "nquads").unwrap();
        assert!(
            !graph_carries_odrl(&plain),
            "a plain pod must not be refused as ODRL-carrying"
        );
    }

    /// The refusal is a DEFINITIVE 400 (the request can never succeed as written), not the
    /// retryable 503 a materialisation failure maps to.
    #[test]
    fn server_view_error_maps_odrl_to_definitive_400() {
        assert_eq!(
            server_view_error_response(ServerViewError::Odrl).status(),
            StatusCode::BAD_REQUEST
        );
    }
}

// ── sq-sllu4 — unit tests for the `CertScope::Shape` certification wire
// format. Gated on `solid-authz-trust` (zero code otherwise), one DIRECT test per new fn
// (coverage ratchet).
#[cfg(all(test, feature = "solid-authz-trust"))]
mod trust_wire_tests {
    use super::*;

// ── sq-sllu4 — the `CertScope::Shape` certification wire format ─────────
//
// Direct unit tests for the two functions the shape-scope wire format adds: the parser
// branch and the CANONICAL desugaring. The end-to-end derivation (a signed shape-scoped
// edge deriving a rule that grants) is pinned by
// `tests/solid_authz_trust.rs::shape_scoped_certification_derives_rule_and_grants`.

/// Parse a JSON object literal into the map `parse_wire_certification` consumes.
#[cfg(feature = "solid-authz-trust")]
fn cert_obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().expect("test fixture is an object").clone()
}

/// A wire certification object with every non-scope field present and well-formed.
#[cfg(feature = "solid-authz-trust")]
fn cert_fixture(scope: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut obj = cert_obj(serde_json::json!({
        "certifierIri": "https://gov.example/framework",
        "certifierKeyHex": "00",
        "certifiedIssuerIri": "https://issuer.example/dvs",
        "certifiedKeyHex": "00",
        "validFromUnixSecs": 0_i64,
        "validUntilUnixSecs": 9_999_999_999_i64,
        "signatureHex": "aabbcc"
    }));
    for (k, v) in cert_obj(scope) {
        obj.insert(k, v);
    }
    obj
}

#[cfg(feature = "solid-authz-trust")]
#[test]
fn wire_cert_scope_kinds_parse_or_fail_closed() {
    // "anyService" — the pre-existing kind, unchanged.
    let any = parse_wire_certification(&cert_fixture(
        serde_json::json!({ "scopeKind": "anyService" }),
    ))
    .expect("anyService parses");
    assert!(matches!(any.scope, WireCertScope::AnyService));

    // "shape" + a predicate IRI — the new kind.
    let shaped = parse_wire_certification(&cert_fixture(serde_json::json!({
        "scopeKind": "shape",
        "scopeShapePredicateIri": "https://schema.org/age"
    })))
    .expect("shape + predicate parses");
    match shaped.scope {
        WireCertScope::Shape { ref predicate_iri } => {
            // POSITIONAL format args (CodeQL guard).
            assert_eq!(predicate_iri, "https://schema.org/age");
        }
        _ => panic!("expected a shape scope"),
    }

    // "shape" with NO predicate is UNSPECIFIED, not wider — fail closed.
    assert!(
        parse_wire_certification(&cert_fixture(serde_json::json!({ "scopeKind": "shape" })))
            .is_err(),
        "a 'shape' scope without 'scopeShapePredicateIri' must fail closed"
    );
    // An unmodelled kind is never silently widened to AnyService.
    assert!(
        parse_wire_certification(&cert_fixture(
            serde_json::json!({ "scopeKind": "somethingElse" })
        ))
        .is_err(),
        "an unknown scopeKind must fail closed"
    );
}

#[cfg(feature = "solid-authz-trust")]
#[test]
fn cert_scope_shape_is_canonical_and_reproducible() {
    // The load-bearing property: the desugaring is a FUNCTION of the predicate IRI alone.
    // `certification_message` hashes each scope triple (blank nodes BY LABEL, in listed
    // order), so a certifier signing its own copy of this shape must reconstruct the same
    // preimage the server does. Fresh labels would break that on every request.
    let a = cert_scope_predicate_shape("https://schema.org/age").expect("valid predicate");
    let b = cert_scope_predicate_shape("https://schema.org/age").expect("valid predicate");
    assert_eq!(a.root, b.root, "the shape root is canonical, not fresh");
    assert_eq!(a.triples, b.triples, "the shape triples are canonical, in a fixed order");
    assert_eq!(
        a.root.to_string(),
        format!("_:{}", CERT_SCOPE_SHAPE_LABEL),
        "the root carries the documented canonical label"
    );
    assert_eq!(a.triples.len(), 4, "the documented four-triple desugaring");

    // A different predicate is a different shape (the scope actually scopes something).
    let other = cert_scope_predicate_shape("https://schema.org/name").expect("valid");
    assert_ne!(a.triples, other.triples, "the predicate is bound into the shape");

    // The fresh-label sibling is deliberately NOT canonical — the two differ by label.
    let fresh_a = predicate_shape("https://schema.org/age").expect("valid");
    let fresh_b = predicate_shape("https://schema.org/age").expect("valid");
    assert_ne!(
        fresh_a.root, fresh_b.root,
        "predicate_shape mints fresh labels (only the cert-scope variant is canonical)"
    );

    // Fail-closed on a predicate that is not an IRI.
    assert!(
        cert_scope_predicate_shape("not an iri").is_none(),
        "an invalid predicate IRI must fail closed"
    );
}
}

// ── The bounded authorization-build pool ──────────────────────────────────────
//
// These run the REAL handlers in-process (the same `decide_endpoint` /
// `query_endpoint` / `wac_allow_endpoint` the router serves) rather than a stand-in, so what
// they observe is what a request observes. They live here, next to the pool, because the
// counters they read are crate-private: an integration test could see the 503 but not that
// the build was refused BY the pool rather than by a materialisation.
#[cfg(test)]
mod build_pool_tests {
    use super::*;
    use crate::http::{AppState, ServerConfig};
    use std::time::Duration;

    /// A state serving `/authz/*` with a build pool of `concurrency` slots and `wait_ms` of
    /// patience for one.
    fn state(concurrency: usize, wait_ms: u64) -> AppState {
        AppState::with_config(
            sparq_core::Graph::default(),
            ServerConfig {
                solid_authz: true,
                solid_authz_build_concurrency: concurrency,
                solid_authz_build_wait: Duration::from_millis(wait_ms),
                ..ServerConfig::default()
            },
        )
    }

    /// The pod dataset the lane tests authorise over: alice has Read by default on the root.
    const WAC_NQUADS: &str = concat!(
        "<https://pod.ex/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/n1> .\n",
        "<https://pod.ex/.acl#o> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ",
        "<http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> ",
        "<https://pod.ex/> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> ",
        "<https://alice.ex/card#me> <https://pod.ex/.acl> .\n",
        "<https://pod.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> ",
        "<http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
    );

    fn decide_bytes(body: serde_json::Value) -> Bytes {
        Bytes::from(body.to_string())
    }

    /// A stateless `decide` body over [`WAC_NQUADS`] as alice.
    fn body_lane_request() -> Bytes {
        decide_bytes(serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/n1",
            "mode": "read",
            "view": "wac",
        }))
    }

    struct ReleaseOnDrop(Arc<BuildHook>);

    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    fn block_builds(st: &AppState) -> Arc<BuildHook> {
        let hook = Arc::new(BuildHook::default());
        *st.authz_build_pool().build_hook.lock().unwrap() = Some(hook.clone());
        hook
    }

    async fn wait_for_running(hook: &BuildHook, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while hook.running.load(std::sync::atomic::Ordering::SeqCst) < count {
                hook.entered.notified().await;
            }
        })
        .await
        .expect("blocking builds entered the barrier");
    }

    fn start_build(st: &AppState) -> tokio::task::JoinHandle<Response> {
        tokio::spawn(decide_endpoint(
            State(st.clone()),
            HeaderMap::new(),
            body_lane_request(),
        ))
    }

    async fn response_or_release_extra_build(
        mut handler: tokio::task::JoinHandle<Response>,
        hook: &BuildHook,
        cap: usize,
    ) -> Response {
        tokio::select! {
            response = &mut handler => response.unwrap(),
            () = wait_for_running(hook, cap + 1) => {
                // A permit-lifetime mutant admits extra work. Release it so the test
                // reports its failed assertion instead of leaving blocking workers hung.
                hook.release();
                handler.await.unwrap()
            }
        }
    }

    #[tokio::test]
    async fn build_pool_caps_running_builds_even_after_handler_cancellation() {
        let st = state(2, 50);
        let hook = block_builds(&st);
        let _release = ReleaseOnDrop(hook.clone());
        let first = start_build(&st);
        let second = start_build(&st);
        wait_for_running(&hook, 2).await;
        let third = response_or_release_extra_build(start_build(&st), &hook, 2).await;
        let running = hook.running.load(std::sync::atomic::Ordering::SeqCst);
        first.abort();
        let _ = first.await;
        let fourth = response_or_release_extra_build(start_build(&st), &hook, 2).await;
        let running_after_cancel = hook.running.load(std::sync::atomic::Ordering::SeqCst);
        // Release before asserting, including on mutants, so blocked workers always finish.
        hook.release();
        let _ = second.await.unwrap();
        assert_eq!(third.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(fourth.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(running, 2);
        assert_eq!(running_after_cancel, 2);
    }

    #[tokio::test]
    async fn build_pool_keeps_cancelled_builds_permit_until_blocking_work_ends() {
        let st = state(1, 50);
        let hook = block_builds(&st);
        let _release = ReleaseOnDrop(hook.clone());
        let handler = start_build(&st);
        wait_for_running(&hook, 1).await;
        handler.abort();
        let _ = handler.await;
        let available = st.authz_build_pool().available();
        hook.release();
        tokio::time::timeout(Duration::from_secs(10), async {
            while st.authz_build_pool().available() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the completed build returns its permit");
        assert_eq!(
            available, 0,
            "aborting the handler cannot free a running build's slot"
        );
        assert_eq!(st.authz_build_pool().available(), 1);
    }

    #[tokio::test]
    async fn a_warm_server_view_needs_no_build_slot() {
        let st = AppState::with_config(
            sparq_core::Graph::load_dataset(WAC_NQUADS, "nquads").unwrap(),
            ServerConfig {
                solid_authz: true,
                solid_authz_build_concurrency: 1,
                solid_authz_build_wait: Duration::from_millis(50),
                ..ServerConfig::default()
            },
        );
        let request = || {
            decide_bytes(serde_json::json!({
                "source": "server", "resource": "https://pod.ex/n1", "mode": "read",
                "session": { "agent": "https://alice.ex/card#me" }
            }))
        };
        let warm = decide_endpoint(State(st.clone()), HeaderMap::new(), request()).await;
        assert_eq!(warm.status(), StatusCode::OK);
        let hook = block_builds(&st);
        let _release = ReleaseOnDrop(hook.clone());
        let blocked = start_build(&st);
        wait_for_running(&hook, 1).await;
        let response = decide_endpoint(State(st.clone()), HeaderMap::new(), request()).await;
        hook.release();
        let _ = blocked.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            st.authz_build_pool().admitted(),
            2,
            "warm hit claimed no slot"
        );
        assert_eq!(st.authz_build_pool().refused(), 0);
    }

    #[tokio::test]
    async fn automatic_view_selection_does_not_reuse_an_explicit_other_view() {
        let dataset = format!("{WAC_NQUADS}<urn:s> <urn:p> <urn:o> <https://pod.ex/.acr> .\n");
        let st = AppState::with_config(
            sparq_core::Graph::load_dataset(&dataset, "nquads").unwrap(),
            ServerConfig {
                solid_authz: true,
                ..ServerConfig::default()
            },
        );
        let explicit = server_store(&st, Some(AuthView::Wac)).await.unwrap();
        let automatic = server_store(&st, None).await.unwrap();
        let session = Session {
            agent: Some("https://alice.ex/card#me"),
            ..Session::default()
        };
        assert!(
            explicit
                .decide(&session, "https://pod.ex/n1", Mode::Read)
                .allow
        );
        assert!(
            !automatic
                .decide(&session, "https://pod.ex/n1", Mode::Read)
                .allow
        );
        assert_eq!(st.authz_build_pool().admitted(), 2);
        assert_eq!(
            st.authz_view_cache().read().unwrap().as_ref().unwrap().view,
            AuthView::Acp
        );
    }

    #[test]
    fn shed_warning_counts_refusals_between_rate_limited_lines() {
        use std::sync::atomic::Ordering;
        let pool = AuthBuildPool::new(1, Duration::ZERO);
        let now = std::time::Instant::now();
        pool.refused.store(1, Ordering::Relaxed);
        assert_eq!(pool.shed_warning(|| now, |_, _| {}), Some((1, 1)));
        pool.refused.store(4, Ordering::Relaxed);
        assert_eq!(
            pool.shed_warning(|| now + Duration::from_millis(999), |_, _| {}),
            None
        );
        assert_eq!(
            pool.shed_warning(|| now + Duration::from_secs(1), |_, _| {}),
            Some((3, 4))
        );
        pool.refused.store(5, Ordering::Relaxed);
        assert_eq!(
            pool.shed_warning(|| now + Duration::from_millis(1999), |_, _| {}),
            None
        );
        assert_eq!(
            pool.shed_warning(|| now + Duration::from_secs(2), |_, _| {}),
            Some((1, 5))
        );
    }

    #[test]
    fn pool_concurrency_clamps_at_semaphore_limits() {
        assert_eq!(AuthBuildPool::new(0, Duration::ZERO).available(), 1);
        assert_eq!(
            AuthBuildPool::new(usize::MAX, Duration::ZERO).available(),
            tokio::sync::Semaphore::MAX_PERMITS
        );
    }

    /// A SATURATED pool sheds the request with the retryable 503 instead of waiting behind
    /// the builds already in flight — the whole point of the bound. The only permit is held
    /// for the duration, so the pool is saturated deterministically rather than by racing
    /// requests, and the request must come back within its configured patience rather than
    /// whenever the permit happens to be released.
    #[tokio::test]
    async fn a_saturated_build_pool_sheds_the_request_with_503_instead_of_waiting() {
        let st = state(1, 50);
        // Hold the one and only build slot for the whole of the request below.
        let held = st
            .authz_build_pool()
            .claim()
            .await
            .expect("the only build slot");
        assert_eq!(
            st.authz_build_pool().available(),
            0,
            "the pool is saturated"
        );

        let began = std::time::Instant::now();
        let resp = decide_endpoint(State(st.clone()), HeaderMap::new(), body_lane_request()).await;
        let waited = began.elapsed();

        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a saturated pool is the fail-closed, retryable 503"
        );
        assert_eq!(
            st.authz_build_pool().refused(),
            1,
            "the refusal came from the pool, not from a materialisation"
        );
        assert_eq!(
            st.authz_build_pool().admitted(),
            1,
            "only this test's own hold was admitted; the request's build was not"
        );
        // It WAITED for the configured patience and then gave up — it neither returned
        // instantly (that would be no wait at all) nor waited for the permit to be freed.
        assert!(
            waited >= Duration::from_millis(50),
            "the request did not wait for a slot at all ({:?})",
            waited
        );
        assert!(
            waited < Duration::from_secs(10),
            "the request waited for the held permit instead of giving up ({:?})",
            waited
        );

        // Once a slot frees, the same request is served normally: the bound sheds load, it
        // does not break the lane.
        drop(held);
        let resp = decide_endpoint(State(st.clone()), HeaderMap::new(), body_lane_request()).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "with a free slot the same request is answered"
        );
        assert_eq!(
            st.authz_build_pool().admitted(),
            2,
            "that build was admitted (after this test's own hold)"
        );
    }

    /// Every build slot is given back when its build ends, so the pool cannot leak capacity
    /// across requests (a leak would turn the cap into a one-shot budget).
    #[tokio::test]
    async fn a_finished_build_gives_its_slot_back() {
        let st = state(2, 1_000);
        for _ in 0..3 {
            let resp =
                decide_endpoint(State(st.clone()), HeaderMap::new(), body_lane_request()).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }
        assert_eq!(st.authz_build_pool().admitted(), 3);
        assert_eq!(st.authz_build_pool().refused(), 0);
        assert_eq!(
            st.authz_build_pool().available(),
            2,
            "every slot is back after the builds ended"
        );
    }

    /// The STATELESS lane's build runs on the pool.
    #[tokio::test]
    async fn the_stateless_lane_builds_on_the_pool() {
        let st = state(4, 1_000);
        let resp = decide_endpoint(State(st.clone()), HeaderMap::new(), body_lane_request()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            st.authz_build_pool().admitted(),
            1,
            "the stateless build went through the pool"
        );
    }

    /// The STATEFUL lane's per-generation build runs on the SAME pool.
    #[tokio::test]
    async fn the_stateful_lane_builds_on_the_pool() {
        let st = state(4, 1_000);
        let body = decide_bytes(serde_json::json!({
            "source": "server",
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/n1",
            "mode": "read",
        }));
        let resp = decide_endpoint(State(st.clone()), HeaderMap::new(), body).await;
        // An empty server store discovers no ACL: the fail-closed `noAcl` deny, not an error.
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            st.authz_build_pool().admitted(),
            1,
            "the stateful build went through the pool"
        );
    }

    /// The `/authz/query` and `/authz/wac-allow` lanes build on the pool too, so the cap is
    /// on authorization builds in flight and not on one endpoint's.
    #[tokio::test]
    async fn the_query_and_advertisement_lanes_build_on_the_pool() {
        let st = state(4, 1_000);
        let query = decide_bytes(serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "query": "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }",
        }));
        let resp = query_endpoint(State(st.clone()), HeaderMap::new(), query).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let advert = decide_bytes(serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/n1",
        }));
        let resp = wac_allow_endpoint(State(st.clone()), HeaderMap::new(), advert).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            st.authz_build_pool().admitted(),
            2,
            "both lanes built on the pool"
        );
    }

    /// The ODRL store and policy builds take one claim per request on every endpoint.
    #[cfg(feature = "odrl-authz")]
    #[tokio::test]
    async fn the_odrl_lane_builds_on_the_pool() {
        let odrl = "http://www.w3.org/ns/odrl/2/";
        let dataset = format!(
            "{WAC_NQUADS}\
             <https://pol.ex/p1> <{odrl}permission> <https://pol.ex/r1> <https://pod.ex/pol> .\n\
             <https://pol.ex/r1> <{odrl}action> <{odrl}read> <https://pod.ex/pol> .\n\
             <https://pol.ex/r1> <{odrl}target> <https://pod.ex/n1> <https://pod.ex/pol> .\n\
             <https://pol.ex/r1> <{odrl}assignee> <https://alice.ex/card#me> <https://pod.ex/pol> .\n"
        );
        let st = state(4, 1_000);
        let body = decide_bytes(serde_json::json!({
            "dataset": dataset,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/n1",
            "mode": "read",
            "view": "wac",
        }));
        let resp = decide_endpoint(State(st.clone()), HeaderMap::new(), body).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            st.authz_build_pool().admitted(),
            1,
            "the store and ODRL builds share one pool claim"
        );
    }

    #[cfg(feature = "solid-authz-trust")]
    #[tokio::test]
    async fn a_trust_parse_failure_still_requires_a_build_slot() {
        let st = AppState::with_config(
            sparq_core::Graph::default(),
            ServerConfig {
                solid_authz: true,
                solid_authz_trust: true,
                solid_authz_build_concurrency: 1,
                solid_authz_build_wait: Duration::ZERO,
                ..ServerConfig::default()
            },
        );
        let request = || {
            decide_bytes(serde_json::json!({
                "dataset": WAC_NQUADS, "resource": "https://pod.ex/n1", "mode": "read",
                "session": {"agent": "https://alice.ex/card#me"}, "trust": {}
            }))
        };
        let held = st.authz_build_pool().claim().await.unwrap();
        let saturated = decide_endpoint(State(st.clone()), HeaderMap::new(), request()).await;
        drop(held);
        let parsed = decide_endpoint(State(st.clone()), HeaderMap::new(), request()).await;
        assert_eq!(saturated.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(parsed.status(), StatusCode::FORBIDDEN);
    }

    /// The whole trust pipeline, including failures before store construction, is pooled.
    #[cfg(feature = "solid-authz-trust")]
    #[tokio::test]
    async fn the_trust_lane_builds_on_the_pool() {
        let st = AppState::with_config(
            sparq_core::Graph::default(),
            ServerConfig {
                solid_authz: true,
                solid_authz_trust: true,
                solid_authz_build_concurrency: 4,
                solid_authz_build_wait: Duration::from_millis(1_000),
                ..ServerConfig::default()
            },
        );
        // A well-formed trust block that admits nothing: the lane reaches its WAC/ACP build,
        // which is the step under test.
        let body = decide_bytes(serde_json::json!({
            "dataset": WAC_NQUADS,
            "session": { "agent": "https://alice.ex/card#me" },
            "resource": "https://pod.ex/n1",
            "mode": "read",
            "view": "wac",
            "trust": {
                "agentIri": "https://alice.ex/card#me",
                "nowUnixSecs": 1_720_000_000_i64,
                "rules": [],
                "certifications": [],
                "credentials": []
            }
        }));
        let resp = decide_endpoint(State(st.clone()), HeaderMap::new(), body).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "the unchanged WAC grant still decides the trust lane"
        );
        assert_eq!(
            st.authz_build_pool().admitted(),
            1,
            "the trust lane's build went through the pool"
        );
    }
    #[cfg(feature = "backup")]
    #[tokio::test]
    async fn pre_restore_miss_cannot_install_after_a_new_lineage() {
        let st = state(2, 50);
        // Start a cache miss on the old lineage and block after its snapshot was pinned.
        let hook = Arc::new(BuildHook::default());
        let _release = ReleaseOnDrop(hook.clone());
        *st.authz_build_pool().view_hook.lock().unwrap() = Some(hook.clone());
        let old_state = st.clone();
        let old = tokio::spawn(async move { server_store(&old_state, None).await });
        wait_for_running(&hook, 1).await;
        let source = AppState::with_config(
            sparq_core::Graph::load_dataset(WAC_NQUADS, "nquads").unwrap(),
            ServerConfig::default(),
        );
        let mut artifact = Vec::new();
        source.export_backup(&mut artifact).unwrap();
        st.restore_from(artifact.as_slice(), false).unwrap();
        // Warm the restored lineage while the old miss remains blocked.
        *st.authz_build_pool().view_hook.lock().unwrap() = None;
        let restored = server_store(&st, None).await.unwrap();
        assert!(
            restored
                .decide(
                    &Session {
                        agent: Some("https://alice.ex/card#me"),
                        ..Default::default()
                    },
                    "https://pod.ex/n1",
                    Mode::Read
                )
                .allow
        );
        hook.release();
        old.await.unwrap().unwrap();
        {
            let cache = st.authz_view_cache().read().unwrap();
            assert!(
                Arc::ptr_eq(&cache.as_ref().unwrap().core, &st.authz_core()),
                "old build must not replace the restored view"
            );
        }
        let following = server_store(&st, None).await.unwrap();
        assert!(Arc::ptr_eq(&following, &restored));
    }
}
#[cfg(all(test, feature = "solid-authz-trust"))]
mod trust_limit_tests {
    use super::*;
    use crate::http::{AppState, ServerConfig};
    use std::sync::atomic::Ordering;

    fn body(rules: usize, credentials: usize, triples: usize, certs: usize) -> serde_json::Value {
        use oxrdf::{Literal, NamedNode, Triple};
        use sparq_zk::{
            commit::commit_triples,
            encode::salt_from_bytes,
            sig::{public_key_to_hex, SecretKey},
        };
        let key = SecretKey::from_seed(41);
        let graph: Vec<_> = (0..triples)
            .map(|i| {
                Triple::new(
                    NamedNode::new("https://alice.ex/card#me").unwrap(),
                    NamedNode::new("urn:claim").unwrap(),
                    Literal::from(i as i64),
                )
            })
            .collect();
        let commitment = commit_triples(&graph, salt_from_bytes(&[7; 32])).unwrap();
        let credential = serde_json::json!({
            "graphNquads": graph.iter().map(|t| format!("{t} .\n")).collect::<String>(),
            "issuerSignatureHex": key.sign_commitment(&commitment.commitment),
            "saltHex": "07".repeat(32), "issuedAtUnixSecs": 100, "revoked": false,
        });
        let rule = serde_json::json!({
            "source": "urn:issuer", "issuerKeyHex": public_key_to_hex(&key.public_key()),
            "scopeIri": "https://pod.ex/", "freshWithinSecs": 100, "shapePredicateIri": "urn:claim",
        });
        let child = SecretKey::from_seed(42);
        let mut cert = sparq_trust::graph::Certification {
            certifier: NamedNode::new("urn:issuer").unwrap(),
            certifier_key: key.public_key(),
            certified_issuer: NamedNode::new("urn:child").unwrap(),
            certified_key: child.public_key(),
            scope: sparq_trust::graph::CertScope::AnyService,
            valid_from_unix_secs: 0,
            valid_until_unix_secs: 200,
            signature_hex: String::new(),
        };
        cert.signature_hex = key.sign_commitment(&sparq_trust::graph::certification_message(&cert));
        let certification = serde_json::json!({
            "certifierIri":"urn:issuer", "certifierKeyHex":public_key_to_hex(&key.public_key()),
            "certifiedIssuerIri":"urn:child", "certifiedKeyHex":public_key_to_hex(&child.public_key()),
            "scopeKind":"anyService", "validFromUnixSecs":0, "validUntilUnixSecs":200,
            "signatureHex":cert.signature_hex,
        });
        serde_json::json!({
            "dataset": concat!(
                "<https://pod.ex/n1> <urn:p> <urn:o> <https://pod.ex/n1> .\n",
                "<https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n",
                "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n",
                "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .\n",
                "<https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n",
            ),
            "resource": "https://pod.ex/n1", "mode": "read", "session": {"agent":"https://alice.ex/card#me"},
            "trust": {"agentIri":"https://alice.ex/card#me", "nowUnixSecs":100,
                "rules":vec![rule; rules], "credentials":vec![credential; credentials],
                "certifications":vec![certification; certs]},
        })
    }

    async fn run(
        config: ServerConfig,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value, usize) {
        let bytes = body.to_string();
        assert!(
            bytes.len() <= 1 << 20,
            "boundary fixture fits the default body limit"
        );
        let state = AppState::with_config(
            sparq_core::Graph::default(),
            ServerConfig {
                solid_authz: true,
                solid_authz_trust: true,
                ..config
            },
        );
        let response =
            decide_endpoint(State(state.clone()), HeaderMap::new(), Bytes::from(bytes)).await;
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&body).unwrap(),
            state
                .authz_build_pool()
                .trust_verification_calls
                .load(Ordering::Relaxed),
        )
    }

    #[tokio::test]
    async fn default_pair_cap_accepts_boundary_and_refuses_one_more_before_verification() {
        let defaults = ServerConfig::default();
        assert_eq!(defaults.solid_authz_trust_max_pairs, 4096);
        let (status, deny, calls) = run(defaults.clone(), body(17, 241, 1, 0)).await;
        assert_eq!(17 * 241, defaults.solid_authz_trust_max_pairs + 1);
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(deny["trustDenied"], true);
        assert_eq!(deny["allow"], false);
        assert_eq!(
            calls, 0,
            "no derivation or admission call may start for an over-cap block"
        );
        let (status, allow, calls) = run(defaults, body(16, 256, 1, 0)).await;
        assert_eq!(status, StatusCode::OK, "{allow}");
        assert_eq!(allow["allow"], true);
        assert_eq!(allow["trustJustification"]["admittedCount"], 256);
        assert!(calls > 0);
    }

    #[tokio::test]
    async fn certification_cap_prices_potential_derived_rules_before_verification() {
        let config = ServerConfig::default();
        let cap = config.solid_authz_trust_max_certifications;
        assert_eq!(cap, 2048);
        let (status, deny, calls) = run(config.clone(), body(1, 0, 1, cap + 1)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(deny["trustDenied"], true);
        assert_eq!(calls, 0);
        let (status, allow, calls) = run(config, body(1, 0, 1, cap)).await;
        assert_eq!(status, StatusCode::OK, "{allow}");
        assert!(calls > 0);
        let config = ServerConfig {
            solid_authz_trust_max_pairs: 2,
            ..Default::default()
        };
        let (status, _, calls) = run(config, body(1, 1, 1, 2)).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "one direct plus two potential derived rules"
        );
        assert_eq!(calls, 0);
    }

    #[tokio::test]
    async fn validation_work_cap_counts_triples_even_when_pairs_fit() {
        let config = ServerConfig {
            solid_authz_trust_max_validation_work: 6,
            ..Default::default()
        };
        let (status, _, calls) = run(config.clone(), body(2, 1, 4, 0)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(calls, 0);
        let (status, allow, calls) = run(config, body(2, 1, 3, 0)).await;
        assert_eq!(status, StatusCode::OK, "{allow}");
        assert!(calls > 0);
        let defaults = ServerConfig::default();
        assert_eq!(defaults.solid_authz_trust_max_validation_work, 65536);
        let rules = defaults.solid_authz_trust_max_validation_work / 128;
        let (status, _, calls) = run(defaults.clone(), body(rules + 1, 1, 128, 0)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(calls, 0);
        let (status, allow, calls) = run(defaults, body(rules, 1, 128, 0)).await;
        assert_eq!(status, StatusCode::OK, "{allow}");
        assert_eq!(allow["trustJustification"]["admittedCount"], 128);
        assert!(calls > 0);
    }
}
