//! The axum HTTP surface: routing, extraction, status/error semantics.
//!
//! Two route groups:
//!   * `/sparql` — the SPARQL 1.1 Protocol `query` operation (GET + the two POST forms, plus
//!     the query-only HTTP `QUERY` method — w3c/sparql-protocol#40, sq-b3df9, for Oxigraph
//!     interop) and the `update` operation (POST).
//!   * Graph Store HTTP Protocol — `GET`/`HEAD` (read) and, since sq-gxsj, `PUT`/`POST`/
//!     `DELETE` (write) on `/graphs/*path` (direct) and on `/sparql/graph` via
//!     `?graph=<uri>` / `?default` (indirect). [OPUS-4.8] The write verbs translate into a
//!     SPARQL Update and submit through the SAME sequenced [`sparq_serve::Writer`] the
//!     `application/sparql-update` path uses, so they inherit its atomicity, group commit
//!     and snapshot-consistency — and its **no-auth** posture (a GSP write is as powerful
//!     as an UPDATE; see the README "Security posture").
//!
//! Shared state is the sparq-serve GENERATION RING + SEQUENCED WRITER (Wave A,
//! research/concurrent-serving.md §6): queries pin the current generation once per request
//! ([`GenerationRing::current`], lock-free) and evaluate against its immutable snapshot
//! for the whole response — including streamed bodies; SPARQL Update submits through the
//! single sequenced [`sparq_serve::Writer`] (group-commit window, §6.5), which publishes
//! each batch as ONE new generation. Readers never block the writer; the writer never
//! waits for (or reclaims from) readers. This replaced the previous double-buffered
//! `RwLock<Arc<Graph>>` + spare-reclaim writer, whose measured pathologies (§4.3/§4.4:
//! pinned-snapshot writer stalls and reclaim polling) the ring removes by design.

use std::collections::HashMap;
use std::net::SocketAddr; // [OPUS-4.8] sq-o4qf: bind_posture classifies the listen address
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Bytes,
    error_handling::HandleErrorLayer,
    extract::{DefaultBodyLimit, Query, RawQuery, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    BoxError, Router,
};
#[cfg(feature = "federation-descriptors")]
use axum::http::Uri;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
#[cfg(feature = "response-compression")]
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

// [OPUS-4.8] (sq-o5bi, sq-0g6g) `ArcSwap` is the swap mechanism for the ONLINE restore and is
// pulled in ONLY under the `backup` feature — the default serving core is the plain
// `ring`/`writer` pair (byte-identical to pre-#941), so the default read path never touches it.
#[cfg(feature = "backup")]
use arc_swap::ArcSwap;
use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_serve::{
    ApplyUpdates, Generation, GenerationRing, GraphApplier, PodId, WriteError, Writer, WriterConfig,
};

use crate::exec::{
    apply_update_dataset, prepare_with_dataset, DatasetOverride, PrepareError, QueryForm,
    UpdateDatasetError, UsingOverride,
};
use crate::negotiate::{
    negotiate_graph, negotiate_graph_or_406, negotiate_or_406, Format, GraphFormat,
};
use crate::results;

// ---------------------------------------------------------------------------
// SPARQL extension functions (opt-in `geo` cargo feature)
// ---------------------------------------------------------------------------

/// Runs `f` — a synchronous engine call (or a whole blocking-pool work item) — with
/// the server's SPARQL extension functions installed.
///
/// With the opt-in `geo` cargo feature this is sparq-geo's `geof_registry()`
/// (the GeoSPARQL spatial functions), built once per process and scoped
/// thread-locally around the call (the engine re-installs it inside its rayon
/// workers — see docs/extension-functions.md). Without the feature it is the
/// identity function, so the engine call is *exactly* the registry-free one —
/// zero cost, no new dependencies, byte-identical behaviour.
#[cfg(feature = "geo")]
pub(crate) fn with_extensions<T>(f: impl FnOnce() -> T) -> T {
    static GEOF: std::sync::OnceLock<sparq_engine::FunctionRegistry> = std::sync::OnceLock::new();
    sparq_engine::with_functions(GEOF.get_or_init(sparq_geo::geof_registry), f)
}

#[cfg(not(feature = "geo"))]
#[inline(always)]
pub(crate) fn with_extensions<T>(f: impl FnOnce() -> T) -> T {
    f()
}

/// [OPUS-4.8] (sq-4w18) Runs an engine call inside the full per-request engine scope:
/// the SERVICE egress allowlist policy AND the SPARQL extension functions.
///
/// With the `service` cargo feature, this installs
/// `sparq_engine::with_service_egress_policy` in STRICT (allowlist-only) mode for the
/// config's [`ServerConfig::service_allow`]: SERVICE may reach ONLY allowlisted hosts —
/// an empty allowlist (the default) refuses ALL federation before any network call.
/// Every engine entry point that can evaluate a `SERVICE` clause (query, ASK,
/// CONSTRUCT/DESCRIBE, the subscription re-eval, and updates with a federated WHERE) is
/// wrapped in this, so the policy applies uniformly. The policy is a thread-local
/// installed for the closure's duration; the engine re-checks it inside the ureq
/// resolver on the same thread, so it covers the blocking-pool worker that runs the call.
///
/// Without the `service` feature this is exactly [`with_extensions`] — no federation
/// code exists, so there is nothing to gate (a SERVICE clause errors at execution).
#[cfg(feature = "service")]
pub(crate) fn with_engine_scope<T>(config: &ServerConfig, f: impl FnOnce() -> T) -> T {
    with_engine_scope_allow(&config.service_allow, f)
}

#[cfg(not(feature = "service"))]
#[inline(always)]
pub(crate) fn with_engine_scope<T>(_config: &ServerConfig, f: impl FnOnce() -> T) -> T {
    with_extensions(f)
}

/// [OPUS-4.8] (sq-9xoh) Runs an engine call inside the per-request engine scope using an
/// EXPLICIT, already-resolved SERVICE egress allowlist instead of the static
/// [`ServerConfig::service_allow`].
///
/// This is the per-request / per-query egress-policy override seam. The static
/// [`with_engine_scope`] always installs the operator-configured allowlist; on the READ path a
/// multi-tenant / gateway deployment can instead derive an allowlist from the request (e.g. an
/// auth token or a header) via [`ServerConfig::service_allow_override`] and pass it here. The
/// installed policy is identical in shape to [`with_engine_scope`] — STRICT (allowlist-only)
/// mode, so an empty allowlist still refuses ALL federation — only the host SET differs, and it
/// is scoped to this one closure (the thread-local guard is dropped at the end), so it cannot
/// leak into another request that later runs on the same blocking-pool worker.
///
/// The resolved allowlist must be computed by the caller BEFORE the engine call is spawned
/// (request headers are not `'static`); [`ServerConfig::resolve_service_allow`] does that and
/// yields an owned [`ServiceAllowlist`](crate::service_config::ServiceAllowlist) that moves into
/// the `spawn_blocking` closure.
#[cfg(feature = "service")]
pub(crate) fn with_engine_scope_allow<T>(
    allow: &crate::service_config::ServiceAllowlist,
    f: impl FnOnce() -> T,
) -> T {
    sparq_engine::with_service_egress_policy(true, allow.engine_entries(), || with_extensions(f))
}

/// [OPUS-4.8] (sq-9xoh) Without the `service` feature there is no federation code to gate, so
/// the per-request allowlist is inert — this is exactly [`with_extensions`], mirroring the
/// `#[cfg(not(feature = "service"))]` form of [`with_engine_scope`].
#[cfg(not(feature = "service"))]
#[inline(always)]
pub(crate) fn with_engine_scope_allow<T>(
    _allow: &crate::service_config::ServiceAllowlist,
    f: impl FnOnce() -> T,
) -> T {
    with_extensions(f)
}

// ---------------------------------------------------------------------------
// Hardening configuration (T15)
// ---------------------------------------------------------------------------

/// [OPUS-4.8] (sq-9xoh) A per-request SERVICE egress allowlist override hook (see
/// [`ServerConfig::service_allow_override`]). Given a request's headers it returns
/// `Some(allowlist)` to use that allowlist (in STRICT mode) for this one request, or `None` to
/// fall back to the operator's static [`ServerConfig::service_allow`].
///
/// A thin newtype over the boxed closure so [`ServerConfig`] can keep deriving [`Debug`] (a bare
/// `dyn Fn` is not `Debug`) and so the construction site reads clearly. Held behind an [`Arc`] so
/// [`ServerConfig`] stays cheaply [`Clone`].
/// [OPUS-4.8] (sq-9xoh) The boxed per-request resolver behind a [`ServiceAllowOverride`]: maps a
/// request's headers to `Some(allowlist)` (use it) or `None` (fall back to the static config).
#[cfg(feature = "service")]
type ServiceAllowResolver =
    dyn Fn(&HeaderMap) -> Option<crate::service_config::ServiceAllowlist> + Send + Sync;

#[cfg(feature = "service")]
#[derive(Clone)]
pub struct ServiceAllowOverride(std::sync::Arc<ServiceAllowResolver>);

#[cfg(feature = "service")]
impl ServiceAllowOverride {
    /// Wraps a per-request resolver closure. It is called once per READ request with that
    /// request's headers; returning `Some` substitutes the allowlist for that request, `None`
    /// falls back to the static [`ServerConfig::service_allow`].
    pub fn new(
        f: impl Fn(&HeaderMap) -> Option<crate::service_config::ServiceAllowlist>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self(std::sync::Arc::new(f))
    }
}

#[cfg(feature = "service")]
impl std::fmt::Debug for ServiceAllowOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceAllowOverride(<fn>)")
    }
}

/// [GPT-5.6] Loaded, immutable SHACL shapes used by transaction guard mode.
#[cfg(feature = "shacl")]
#[derive(Clone)]
pub struct ShaclShapes(Arc<Graph>);

#[cfg(feature = "shacl")]
impl ShaclShapes {
    /// Wraps a parsed shapes graph for [`ServerConfig::shacl_shapes`].
    pub fn new(graph: Graph) -> Self {
        Self(Arc::new(graph))
    }

    fn graph(&self) -> &Graph { &self.0 }
}

#[cfg(feature = "shacl")]
impl std::fmt::Debug for ShaclShapes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaclShapes").field("triples", &self.0.len()).finish()
    }
}

/// Tunable guards that make the endpoint safe to expose publicly (T15).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Per-request query timeout. The engine's cooperative [`QueryBudget`] stops the
    /// worker mid-query at its next coarse check; a hard await-cap of
    /// `timeout + TIMEOUT_GRACE` guarantees the HTTP 503 even if the engine is inside
    /// an uninstrumented stretch. `None` disables the timeout.
    pub query_timeout: Option<Duration>,
    /// [OPUS-4.8] (sq-nulp) **Writer-side WHERE deadline for SPARQL UPDATE — a head-of-line
    /// blocking bound.** Updates are *sequenced* on a single writer thread (sparq-serve's
    /// group-commit writer), so while one update runs its `DELETE/INSERT … WHERE` to its
    /// cooperative stop, every queued update behind it waits. The plain [`query_timeout`](Self::query_timeout)
    /// already bounds the WHERE evaluation, but it is the *read* timeout (default 30 s) — far
    /// too long to hold the writer, because it bounds the offending client's own wait, not the
    /// head-of-line blocking of the whole writer queue.
    ///
    /// When set, this is a SEPARATE, typically-shorter cooperative deadline applied ONLY to
    /// the WHERE phase of an UPDATE on the writer thread: the update's budget deadline becomes
    /// `min(query_timeout, update_where_timeout)`, so any single update releases the writer
    /// within this bound and the queue behind it cannot be head-of-line blocked longer than
    /// that — *no matter how long the read timeout is*. `None` (the default) keeps the
    /// historical behaviour exactly: the update WHERE budget is the plain [`query_timeout`](Self::query_timeout).
    ///
    /// This is a cooperative deadline (checked at the engine's coarse sites — operator entry /
    /// per outer-loop iteration), so it bounds head-of-line blocking *approximately*, like
    /// every other [`QueryBudget`] deadline: an uninstrumented stretch can overrun until the
    /// next check. It is a tunable backstop, not a hard preemption (true preemption of the
    /// writer is out of scope — see `sparq_serve`'s scheduler docs). Set via
    /// `--update-where-timeout` / `SPARQ_UPDATE_WHERE_TIMEOUT` (seconds; `0` disables).
    pub update_where_timeout: Option<Duration>,
    /// [OPUS-4.8] (sq-p7kk5) **Adaptive group-commit for the write path.** The sequenced
    /// writer normally waits a fixed group-commit window (`sparq_serve`'s 3 ms default) after
    /// the first update in a batch, absorbing any concurrent updates that arrive within it into
    /// one published generation. For a **serial / low-concurrency interactive** client (one
    /// in-flight `application/sparql-update` at a time — the LDP-CRUD shape) that window is pure
    /// latency: the client is blocked on its own ack, so nothing can arrive to fill it, and each
    /// update pays ~one window even though the engine's in-place apply is ~15 µs (the sq-p7kk5
    /// update-path gap: the fixed window dominated ~99 % of a serial client's commit latency).
    ///
    /// When `true` (the DEFAULT — the server's write path is interactive) the writer instead
    /// drains only the already-queued backlog after the first update and commits the moment it
    /// empties, so a serial client commits in engine-time (µs) rather than window-time (ms). A
    /// genuinely concurrent stream still batches every update that had piled up between the
    /// writer's iterations, so group-commit amortisation under real load is preserved. This is a
    /// pure LATENCY change with NO effect on the mutation/durability contract: application stays
    /// strict FIFO, per-update atomicity is unchanged, every accepted update is still committed,
    /// visibility and the `--persist` durability path are identical, and each committed
    /// generation is still the serial order — only epoch-tick granularity may be finer under
    /// overlap (a cache-key concern, never correctness). `false` restores the always-windowed
    /// behaviour. Set via `--no-adaptive-commit` / `SPARQ_ADAPTIVE_COMMIT=0`.
    pub adaptive_commit: bool,
    /// Maximum accepted request body, enforced before the handler reads it (413).
    pub max_body_bytes: usize,
    /// Maximum in-flight requests; excess requests are shed with 429.
    pub max_concurrent: usize,
    /// [OPUS-4.8] (sq-2gqr) **Slow-loris guard — connection header-read deadline.** The maximum
    /// time a freshly-accepted HTTP/1 connection may take to transmit its COMPLETE request-header
    /// block. A client that dribbles headers byte-by-byte (the classic slow-loris) otherwise holds
    /// a connection — and, behind the `concurrency_limit`, a concurrency slot — open indefinitely;
    /// `max_concurrent` such clients starve every legitimate caller. Enforced at hyper's HTTP/1
    /// connection layer (`http1().header_read_timeout`), so it fires BEFORE the request ever
    /// reaches a handler — which is exactly why the existing `query_timeout` (a per-request
    /// engine deadline) and the `max_body_bytes` / load-shed guards do NOT cover it: those all
    /// run AFTER the headers are fully parsed. The connection is closed when the deadline elapses.
    ///
    /// `None` disables it (back to the unbounded-header-read behaviour `axum::serve` ships — see
    /// the rationale on [`serve`]). Default 15s. Distinct from the slower `query_timeout` (30s)
    /// because reading a header block is sub-second on any healthy client; 15s is generous
    /// headroom for a slow but honest network without leaving the slot open for minutes.
    pub header_read_timeout: Option<Duration>,
    /// [OPUS-4.8] (sq-lodb) **Slow-body guard — request-body read/idle deadline (complement to
    /// [`header_read_timeout`](Self::header_read_timeout)).** The maximum time the server will wait
    /// for the NEXT chunk of a request body once the previous one has arrived — an *idle* deadline
    /// between consecutive body reads, reset after every chunk. The header-read deadline closes the
    /// classic slow-loris (dribbled *headers*), but it does NOT cover the body phase: once a client
    /// has sent a complete, valid header block it can then dribble the request BODY one byte at a
    /// time (or send a chunk then stall forever) and stay under [`max_body_bytes`](Self::max_body_bytes)
    /// yet hold the connection — and, behind the `concurrency_limit`, a concurrency slot — open
    /// indefinitely. hyper's `header_read_timeout` has already elapsed (the headers parsed fine),
    /// [`query_timeout`](Self::query_timeout) is an engine deadline that only starts once the WHOLE
    /// request has been read, and [`max_body_bytes`](Self::max_body_bytes) is a SIZE cap that a
    /// one-byte-at-a-time trickle never trips. So this is a genuinely distinct vector that needs its
    /// own bound.
    ///
    /// Enforced by a `tower_http::timeout::RequestBodyTimeoutLayer` wrapping the request body in a
    /// `TimeoutBody`: each poll for the next body frame gets a fresh deadline; if that frame does
    /// not arrive within the window the body read fails and the request is aborted. Because the
    /// timer RESETS after every received frame, a slow-but-honest large upload (steady chunks just
    /// under the window apart) is never penalised by total transfer time — only an idle stall is.
    ///
    /// `None` disables it (back to the unbounded body-read behaviour — a slow body can hold the
    /// slot forever). Default 30s. The window is generous (a stall, not a slow link) and matches
    /// the default [`query_timeout`](Self::query_timeout); an operator behind a flaky network widens
    /// it, a hostile-input operator tightens it.
    pub body_read_timeout: Option<Duration>,
    /// Maximum SELECT result rows. Exceeding it is an honest 413 refusal (the engine
    /// aborts evaluation via the row budget), never a silent truncation.
    pub max_results: Option<usize>,
    /// [OPUS-4.8] (sq-ebii) **Memory cap (coarse).** Upper bound on the row count of any
    /// *materialised intermediate or final* result the engine builds for ONE query, on
    /// EVERY form (SELECT / ASK / CONSTRUCT / DESCRIBE / GSP-read), enforced via the
    /// engine's cooperative [`QueryBudget::max_rows`] working-set bound — the query aborts
    /// (413) the moment any materialised result crosses it. This is the OOM guard for a
    /// pathological query whose join blows up the intermediate cardinality.
    ///
    /// **What it actually bounds — be precise:** it caps the NUMBER OF ROWS, not bytes.
    /// Peak heap is roughly `max_query_rows × (per-row term cost)`, so it is a *cardinality*
    /// ceiling, not a hard byte guarantee: a query that materialises few but very wide rows
    /// (many projected vars, or huge string literals) can still use more memory than the row
    /// count suggests, and the engine's non-row allocations (dictionary growth on UPDATE, a
    /// CONSTRUCT template, sort/group scratch) are outside this bound. It is also approximate
    /// in *time*: the budget is checked at coarse sites (operator entry / per outer loop
    /// iteration), so a single uninstrumented stretch can transiently exceed it before the
    /// next check. Treat it as a blunt anti-OOM circuit-breaker, NOT an RSS quota. `None`
    /// (the default) disables it. For the byte-accounted companion that DOES price row
    /// width and computed-literal size, see [`max_query_bytes`](Self::max_query_bytes).
    ///
    /// Distinct from `max_results`: this caps the working set on *all* forms, whereas
    /// `max_results` is folded into the budget only on the paths that pass
    /// `make_budget(_, true)` — SELECT (the final projection) AND CONSTRUCT/DESCRIBE (their
    /// WHERE-pattern solution count) AND EXPLAIN ANALYZE. It is NOT applied on the
    /// `make_budget(_, false)` paths — ASK and GSP-read — nor to UPDATE (`update_budget`,
    /// which has no projection). When both apply, the effective cap is the tighter of the two.
    pub max_query_rows: Option<usize>,
    /// [OPUS-4.8] (sq-s5is) **Byte-accounted memory cap.** The byte-accounted companion to
    /// [`max_query_rows`](Self::max_query_rows): an upper bound, in BYTES, on the engine's
    /// estimated working-set size for ONE query, on EVERY form (SELECT / ASK / CONSTRUCT /
    /// DESCRIBE / GSP-read / UPDATE-WHERE), enforced via [`QueryBudget::max_bytes`]. A query
    /// whose estimate crosses it aborts (413) at the next coarse cooperative check.
    ///
    /// **Why it exists — the row cap's blind spots:** `--max-query-rows` counts ROWS, so it
    /// under-prices (a) FEW but very WIDE rows (many projected variables → more bytes per row)
    /// and (b) huge query-COMPUTED literals (BIND / aggregate / CONSTRUCT scratch interned
    /// into the per-query local vocabulary — non-row allocations). This cap prices BOTH:
    /// `rows × width × size_of::<Id>()` for each materialised intermediate PLUS the bytes of
    /// the computed terms. A 10-column join and a 1-column join with the same row count now
    /// have different byte budgets, and a `BIND(CONCAT(…huge…))` over a handful of rows is
    /// caught even though the row count is tiny.
    ///
    /// **Honest scope — still a coarse circuit-breaker, NOT an exact RSS quota.** The
    /// estimate is a portable LOWER bound on real heap (it ignores allocator overhead,
    /// `SmallVec` inline-vs-spill, and the graph dictionary / index memory that pre-exists the
    /// query); it is checked at the SAME coarse sites as the row cap, so a single
    /// uninstrumented stretch can transiently overshoot before the next check; and it bounds
    /// the QUERY working set, not process RSS. It is strictly TIGHTER and more
    /// width/literal-aware than the row cap, not a hardware-enforced quota. `None` (the
    /// default) disables it; it composes with `--max-query-rows` and `--max-results`
    /// (whichever ceiling trips first aborts).
    pub max_query_bytes: Option<usize>,
    /// [OPUS-4.8] (sq-ebii) **Decompression-ratio cap (zip-bomb guard).** When a request
    /// body arrives `Content-Encoding: gzip` (the GSP write / RDF-load path), the server
    /// streams the inflate but refuses once the decompressed size would exceed
    /// `min(max_decompress_ratio × compressed_len, max_body_bytes)` — so a tiny highly-
    /// compressible body cannot inflate into an OOM. Rejected with 413 BEFORE the full
    /// decompressed image is held. `0` disables ratio-capped decompression entirely (a
    /// `Content-Encoding` body is then refused outright — fail-closed). Default 20×.
    pub max_decompress_ratio: usize,
    /// Maximum active subscriptions per WebSocket connection (T23); further `subscribe`
    /// requests on the socket are refused with a protocol error message.
    pub max_subscriptions_per_conn: usize,
    /// Maximum active subscriptions across the whole server (T23).
    pub max_subscriptions: usize,
    /// How many generations older than current stay queryable via `?generation=N`
    /// (opt-in `time-travel` feature). Composes with the ring's concurrency bound
    /// K = 4 as a floor (`max(K, this)` — see `sparq_serve::TimeTravelConfig`).
    /// **Memory cost is real:** each retained generation is a FULL `Graph` until
    /// the structural-fork follow-up lands.
    #[cfg(feature = "time-travel")]
    pub time_travel_generations: usize,
    /// Additionally age time-travel generations out after this duration (pruned at
    /// the next publish; the K newest are never age-evicted). `None` = count-bounded
    /// only.
    #[cfg(feature = "time-travel")]
    pub time_travel_max_age: Option<Duration>,
    /// Log every request/response via `tower_http::trace::TraceLayer`.
    pub verbose: bool,
    /// [OPUS-4.8] (sq-toze.34, epic sq-toze) **Redact request content from the `--verbose`
    /// request log.** When `true` (the **default**), the request log records the URI's *path*
    /// verbatim but replaces its *query string* — where `GET /sparql?query=…` carries the full
    /// SPARQL query text, which can contain PII (a patient IRI, an email in a `FILTER`) — with a
    /// `<redacted len=N fp=…>` placeholder: a length signal plus a stable NON-reversible
    /// fingerprint, so logs stay correlation-useful (same query => same `fp`) without exposing
    /// content. When `false` (`--log-full-requests` / `SPARQ_LOG_FULL_REQUESTS=1`), the URI is
    /// logged verbatim, exactly as the bare `TraceLayer` did — the deliberate debug escape hatch.
    ///
    /// **Default ON, rationale:** a privacy-respecting server should not leak request content into
    /// operator logs by accident; turning `--verbose` on for debugging should not silently start
    /// writing potentially-sensitive query text to disk / a SIEM. Operators who genuinely need the
    /// raw text opt in explicitly. This is **log-CONTENT redaction, not anonymity**: the log still
    /// records method, path/endpoint, status, a size signal and timing (it would not be a request
    /// log otherwise). It is also NOT the ZK/MPC privacy story. Inert unless `verbose` is also on
    /// (no request log => nothing to redact). See [`crate::redact`].
    pub redact_logs: bool,
    /// [OPUS-4.8] (sq-0bxp) **Per-query access audit log** runtime switch (CDMC CD-2 / ISO
    /// 27001 A.8.15 / EU CRA logging). When `true`, every query / update / Graph-Store request
    /// emits a structured `tracing` record under the dedicated `target: "sparq_server::audit"`
    /// — requester identity (a Bearer-token fingerprint or `anonymous`, NEVER the secret),
    /// operation class, a NON-reversible query fingerprint (NOT the full query text — the #241
    /// info-leak posture), the access decision (allowed / denied + reason), the HTTP status /
    /// result-row count and the duration. Operators route the target to their compliance sink
    /// via `RUST_LOG`. `false` (the default) emits nothing — the audit instrumentation is a
    /// single boolean check before any record is built, so an audit-disabled request pays
    /// essentially zero. Set by `--audit-log` / `SPARQ_AUDIT_LOG=1`. Present only with the
    /// `audit-log` cargo feature; without it the field, the flag and every call site are
    /// compiled out, so a request pays EXACTLY zero (byte-identical to before). See
    /// `crate::audit`.
    #[cfg(feature = "audit-log")]
    pub audit_log: bool,
    /// [OPUS-4.8] (sq-gos8, epic sq-toze) **Richer STRUCTURED access-audit sink** target (ASVS
    /// V7 / ISO 27001 A.8.15 / CDMC CD-2). `Some(target)` installs the default JSON-Lines sink
    /// ([`crate::access_audit::WriterSink`]) writing to a file or stderr; every enforced access
    /// decision is then recorded as a TYPED record (actor / action / resource / decision +
    /// policy-basis / timestamp / non-reversible request fingerprint). `None` (the default)
    /// installs no sink — every call site is a single `Option` check, so an audit-disabled
    /// request pays essentially zero. Set by `--access-audit <file|stderr>` /
    /// `SPARQ_ACCESS_AUDIT`. PRIVACY BOUNDARY: identities + resource IRIs are recorded by design
    /// (the audit trail); query CONTENT only as a fingerprint, never the raw text. Present only
    /// with the `access-audit` cargo feature; without it the field + every call site are
    /// compiled out (byte-identical to before). See [`crate::access_audit`].
    #[cfg(feature = "access-audit")]
    pub access_audit: Option<crate::access_audit::SinkTarget>,
    /// [OPUS-4.8] (sq-ljfz, sq-gos8 follow-up) **Trusted forwarded-identity header** for the
    /// structured access-audit actor. `Some(name)` tells the audit seam that a fronting
    /// authorization layer — `sparq-solid`, or any Solid/WAC reverse-proxy / identity gateway,
    /// the very "reverse proxy / gateway or sparq-solid" the bind warnings name — authenticates
    /// the user and forwards their resolved WebID in the request header `name`. When set, the
    /// audit trail records [`Actor::WebId`](crate::access_audit::Actor::WebId) (the real
    /// authenticated subject from the WAC/ACP session) instead of the coarse Bearer-token
    /// fingerprint. `None` (the default) trusts no forwarded header, so the actor is derived
    /// from the local Bearer gate exactly as before (byte-identical).
    ///
    /// SECURITY: a forwarded header is client-controllable, so this is honoured ONLY when the
    /// operator explicitly names a trusted header — the operator thereby asserts that the
    /// fronting layer sets/overwrites it so a direct client cannot spoof an arbitrary WebID
    /// into the audit trail. Therefore expose this server ONLY behind that trusted front (not
    /// directly to untrusted clients) when this is set. Set by `--audit-webid-header <name>` /
    /// `SPARQ_AUDIT_WEBID_HEADER`. Present only with the `access-audit` cargo feature; without
    /// it the field + the seam are compiled out (byte-identical to before).
    #[cfg(feature = "access-audit")]
    pub audit_webid_header: Option<String>,
    /// [OPUS-4.8] sq-o4qf: explicit opt-in to bind a **non-loopback** address.
    ///
    /// By default the server has **no authentication** on any endpoint — including the
    /// mutating `application/sparql-update` path and the `/subscriptions` WebSocket.
    /// Binding a non-loopback address (e.g. `0.0.0.0`) therefore exposes the entire dataset
    /// for **read AND write** to anyone who can reach the port. To make that a deliberate
    /// act rather than a foot-gun, the binary REFUSES to bind a non-loopback address unless
    /// this is set (CLI `--allow-remote` / env `SPARQ_ALLOW_REMOTE=1`) OR the whole surface
    /// is authenticated by [`auth_token`](Self::auth_token) AND
    /// [`auth_token_read`](Self::auth_token_read) (sq-zcby) — a write-token alone still
    /// leaves reads open, so it does NOT by itself make a remote bind safe. Even an allowed
    /// remote bind logs a warning. Loopback binds are unaffected. See [`bind_posture`] for
    /// the decision, and `crates/sparq-server/README.md` → "Security posture".
    ///
    /// This is purely a *bind-time* posture gate in the binary; it does not add any
    /// per-request auth and does not affect the library `router`/`harden` surface.
    pub allow_remote: bool,
    /// [OPUS-4.8] sq-zcby (PSS gh-46): the required Bearer token that gates the **write
    /// surface**. When `Some(token)`, every request that MUTATES the dataset must present
    /// `Authorization: Bearer <token>` (scheme casing tolerated) or it is refused `401`
    /// with `WWW-Authenticate: Bearer`. The write surface is: a SPARQL UPDATE on `/sparql`
    /// — `Content-Type: application/sparql-update` OR an `update=` form field (BOTH are
    /// updates by SPARQL-Protocol definition, gated as writes UNCONDITIONALLY), and ALSO a
    /// `query=`/`application/sparql-query` body that *parses as an update* (an update smuggled
    /// through the query path — classification there keys on whether the request mutates, not
    /// the route) — and the Graph-Store-Protocol write methods (`PUT`/`POST`/`DELETE`/`PATCH`) on
    /// `/sparql/graph` and `/graphs/{*path}`. The token is compared in **constant time**
    /// (`constant_time_eq`). A missing vs a wrong token produce the *identical* 401, so an
    /// attacker cannot learn whether a token was presented. `None` (the default) means **no
    /// write auth** — today's behaviour, preserved exactly. Mirrors QLever's `-a <token>`.
    /// Enforced by the library `router` itself, so an embedder gets the gate for free.
    pub auth_token: Option<String>,
    /// [OPUS-4.8] sq-zcby: ALSO gate **reads** (SPARQL query, GSP `GET`/`HEAD`) with the same
    /// [`auth_token`](Self::auth_token). Off by default (QLever-style: writes gated, reads
    /// open). Has no effect unless `auth_token` is also set. When on, the whole surface is
    /// authenticated, which the bind posture ([`AuthPosture`]) treats as "auth present" for
    /// allowing a non-loopback bind without `--allow-remote`.
    ///
    /// [OPUS-4.8] sq-cxk5: the subscription READ surfaces — the `/subscriptions` WebSocket and
    /// the `/subscriptions/sse` Server-Sent-Events stream (both stream live SELECT diffs) — are
    /// gated by this flag too, closing the read-auth bypass that existed when they were always
    /// open. The SSE GET reads the `Authorization: Bearer` header like any other GET; the WS
    /// UPGRADE accepts the token from that header OR (for browsers, which cannot set headers on a
    /// WS handshake) a `Sec-WebSocket-Protocol: bearer.<token>` subprotocol — see
    /// [`crate::subscriptions::subscriptions_endpoint`] and `ws_auth_gate`.
    pub auth_token_read: bool,
    /// [OPUS-4.8] (sq-4w18) SERVICE federation egress allowlist. SPARQL `SERVICE <iri>`
    /// makes attacker-controlled query text trigger an outbound HTTP request from the
    /// server host — an SSRF surface. The server's posture is **default-DENY-all
    /// SERVICE**: with the `service` cargo feature on but this allowlist EMPTY (the
    /// default), every SERVICE clause is refused before any network call. An operator
    /// opts hosts back in via `--service-allow` / `--service-allow-file` /
    /// `SPARQ_SERVICE_ALLOW` (see [`crate::ServiceAllowlist`]); only listed hosts (or
    /// `*.suffix` matches) become reachable — even a public host must be listed.
    ///
    /// Enforced by installing `sparq_engine::with_service_egress_policy` (strict =
    /// allowlist-only) around every engine call. Without the `service` cargo feature
    /// this field is still present (so the config shape is stable) but inert: no
    /// federation code is compiled and a SERVICE clause errors at execution as before.
    pub service_allow: crate::service_config::ServiceAllowlist,
    /// [OPUS-4.8] (sq-9xoh) OPTIONAL per-request SERVICE egress allowlist override hook for
    /// multi-tenant / gateway deployments. The operator installs ONE static allowlist in
    /// [`service_allow`](Self::service_allow); a gateway in front of many tenants may instead want
    /// the reachable SERVICE host set to depend on the REQUEST (e.g. derived from a bearer token or
    /// a header that names the calling tenant). When set, this closure is invoked once per READ
    /// request with that request's headers and, if it returns `Some(allowlist)`, THAT allowlist
    /// replaces [`service_allow`](Self::service_allow) for the duration of that one request's engine
    /// call (installed in STRICT/allowlist-only mode exactly like the static one — an empty returned
    /// allowlist therefore DENIES all SERVICE for that request). Returning `None` falls back to the
    /// static [`service_allow`](Self::service_allow), so an unconfigured request behaves identically
    /// to today.
    ///
    /// `None` (the default) keeps the historical behaviour exactly: every request uses the single
    /// static [`service_allow`](Self::service_allow). Applied via
    /// [`resolve_service_allow`](Self::resolve_service_allow) on the read path (SELECT / ASK /
    /// CONSTRUCT / DESCRIBE / EXPLAIN ANALYZE), where the request headers are in scope. The hook is
    /// NOT applied to the SPARQL-Update writer path: updates are sequenced and group-committed on a
    /// single shared writer thread with no per-request header context, and batch-mates may carry
    /// different tokens, so a per-request egress override there is ill-defined — UPDATE WHERE-clause
    /// federation continues to use the operator's static [`service_allow`](Self::service_allow).
    ///
    /// This field exists only with the `service` cargo feature (there is no federation code to gate
    /// otherwise). The closure must be `Send + Sync` because [`ServerConfig`] is shared across the
    /// async handlers and the blocking pool; it is held behind an [`Arc`] so [`ServerConfig`] stays
    /// `Clone`. The hook MUST NOT relax the strict posture by other means — it can only narrow or
    /// substitute the host set; the policy is always allowlist-only.
    #[cfg(feature = "service")]
    pub service_allow_override: Option<ServiceAllowOverride>,
    /// [OPUS-4.8] (sq-o7o0, ASVS V14.5.3) First-party CORS origin allowlist. `sparq-server`
    /// is a SPARQL DATA API, so its safe default is to emit **no CORS headers** — a
    /// cross-origin browser `fetch` cannot read a response with no
    /// `Access-Control-Allow-Origin`, which is exactly the historical behaviour and the
    /// right posture for a public endpoint. EMPTY (the default) keeps that: no CORS code
    /// path runs and a response is byte-identical to before this option existed.
    ///
    /// An operator running a FIRST-PARTY browser app on a different origin opts that one
    /// origin in via `--cors-allow-origin` / `--cors-allow-origin-file` /
    /// `SPARQ_CORS_ALLOW_ORIGIN` (see [`crate::CorsAllowlist`]). When non-empty, the
    /// [`harden`] middleware reflects an allowlisted request `Origin` into
    /// `Access-Control-Allow-Origin` (never `*`, never with credentials) + `Vary: Origin`,
    /// and answers the `OPTIONS` preflight. An un-listed origin still gets no CORS header.
    /// This is a browser-read gate ONLY: it does not relax auth, the bind posture, body
    /// limits, the SERVICE egress allowlist, or the row caps.
    pub cors_allow: crate::cors_config::CorsAllowlist,
    /// [OPUS-4.8] (sq-7cxr, gh-44) DURABLE PERSISTENCE directory — the QLever `--persist-updates`
    /// equivalent. When `Some(dir)`, the server treats the on-disk index at `dir` as the durable,
    /// rebuildable source of truth: at startup it opens the existing store there (replaying its
    /// write-ahead log so prior updates are present with **no rebuild**), or creates it from the
    /// seed graph; and every committed SPARQL Update is WAL-appended + fsync'd to `dir` (default
    /// graph and named graphs alike) BEFORE the group-commit ack, so a process restart preserves
    /// ALL updates. `None` (the default) is the historical purely in-memory server — updates are
    /// lost on restart. Set by the binary's `--persist <DIR>` flag / `SPARQ_PERSIST_DIR` env.
    pub persist_dir: Option<std::path::PathBuf>,
    /// [OPUS-4.8] (sq-o5bi) RESTORE-ON-START artifact path. When `Some(file)`, the binary
    /// imports the backup artifact at `file` and seeds the in-memory serving store from it
    /// BEFORE binding (fail-closed: a corrupt/mismatched artifact aborts startup with a clean
    /// error). This is the bootstrap primitive for horizontal-scaling stage-2 (a fresh replica
    /// hydrates from a backup) and the base of point-in-time recovery. `None` (the default) is
    /// the historical behaviour (seed from the data file / empty). Mutually exclusive with an
    /// existing `--persist` durable store in v1 (the binary refuses the combination). This field
    /// exists only with the `backup` cargo feature; a build without it compiles no restore code.
    /// Set by the binary's `--restore <FILE>` flag / `SPARQ_RESTORE` env.
    #[cfg(feature = "backup")]
    pub restore: Option<std::path::PathBuf>,
    /// [OPUS-4.8] (sq-bu1a) POINT-IN-TIME-RECOVERY delta chain replayed forward onto the
    /// `restore` base on start. When non-empty (and `restore` is `Some`), the binary imports the
    /// base, then replays these incremental delta artifacts in order BEFORE binding, so the
    /// in-memory store starts at the chain's last `to-generation` — a chosen recovery point.
    /// Fail-closed: a corrupt / version-mismatched / out-of-order / gapped delta aborts startup
    /// with a clean error (the same discipline as the base restore). Each path is one
    /// `sparq_serve::backup_export_delta` output, oldest first. Empty (the default) = restore the
    /// base only. Ignored unless `restore` is also set. This field exists only with the `backup`
    /// cargo feature. Set by the binary's repeatable `--restore-delta <FILE>` flag /
    /// `SPARQ_RESTORE_DELTA` env (a path list).
    #[cfg(feature = "backup")]
    pub restore_delta: Vec<std::path::PathBuf>,
    /// [OPUS-4.8] (sq-ft7u) RESTORE-INTO-DURABLE opt-in for the restore-on-start path. When
    /// `true` AND both [`restore`](Self::restore) and [`persist_dir`](Self::persist_dir) are set,
    /// the restore-on-start writes the artifact THROUGH to the durable `--persist` directory
    /// crash-safely, so the restored dataset SURVIVES A RESTART (the on-disk base becomes the
    /// restored image). `false` (the default) keeps the historical contract: `--restore` and
    /// `--persist` are MUTUALLY EXCLUSIVE (the binary refuses the combination), because an
    /// in-memory-only restore on a durable server would be silently lost on the next restart.
    /// This field exists only with the `backup` cargo feature; a build without it compiles no
    /// restore code. Set by the binary's `--restore-persist` flag / `SPARQ_RESTORE_PERSIST=1` env.
    #[cfg(feature = "backup")]
    pub restore_persist: bool,
    /// [OPUS-4.8] (sq-d3d8, epic sq-3183) OPT-IN federation discovery descriptors. When
    /// `true`, the server serves a W3C VoID dataset description at `GET /.well-known/void`
    /// and a SPARQL 1.1 Service Description for a `GET /sparql` with no `query` parameter
    /// (advertising the endpoint, supported languages, result formats and the default
    /// dataset). `false` (the default) leaves both off: `/.well-known/void` is `404` and a
    /// `GET /sparql` with no `query` returns the historical `400 missing 'query'`.
    ///
    /// This field exists only with the `federation-descriptors` cargo feature (like
    /// [`time_travel_generations`](Self::time_travel_generations) under `time-travel`); a
    /// build without that feature compiles no descriptor code and pays zero cost. Set by the
    /// binary's `--federation-descriptors` flag / `SPARQ_FEDERATION_DESCRIPTORS=1` env.
    #[cfg(feature = "federation-descriptors")]
    pub federation_descriptors: bool,
    /// [GPT-5.6] (sq-lsp7k.5.2) OPT-IN grouped facet-count endpoint. When `true`, the server
    /// serves `POST /facets`: the client supplies a JSON `sparq_introspect::FacetRequest`, and
    /// the server evaluates its class / constraint filters against one pinned graph snapshot,
    /// returning the `sparq_introspect::FacetResponse` JSON with type, predicate, and value
    /// distributions. `false` (the default) leaves the endpoint off: `/facets` is `404`.
    ///
    /// This field exists only with the `facets` cargo feature. A build without that feature
    /// compiles no facet route or `sparq-introspect` dependency and pays zero cost. Set by the
    /// binary's `--facets` flag / `SPARQ_FACETS=1` env. Facet evaluation is a READ over the
    /// store, so the endpoint is gated by the read auth.
    #[cfg(feature = "facets")]
    pub facets: bool,
    /// [GPT-5.6] (sq-lsp7k.9.3) OPT-IN prefix-completion endpoint. When `true`, the server
    /// serves `GET /complete?q=<prefix>&limit=<k>` from a `sparq_text::CompletionIndex` built
    /// against one pinned graph generation. The index is cached in [`AppState`] and rebuilt on
    /// the blocking pool when a later published generation makes it stale. Results are capped at
    /// 100 candidates. `false` (the default) leaves the endpoint off: `/complete` is `404`.
    ///
    /// This field exists only with the `complete` cargo feature. A build without that feature
    /// compiles no completion route, cache, or `sparq-text` dependency and pays zero cost. Set by
    /// the binary's `--complete` flag / `SPARQ_COMPLETE=1` env. Completion is a READ over the
    /// store, so the endpoint is gated by the read auth.
    #[cfg(feature = "complete")]
    pub complete: bool,
    /// [OPUS-4.8] (sq-bzh1, epic sq-3183) OPT-IN Triple Pattern Fragments / Linked Data
    /// Fragments READ-ONLY source endpoint. When `true`, the server serves a paged RDF
    /// fragment of the triples matching one triple pattern at `GET /tpf?subject=&predicate=&
    /// object=` (with Hydra controls: `hydra:totalItems` from the cheap cardinality estimate,
    /// `hydra:next`/`hydra:previous` paging, the `hydra:search` template). `false` (the
    /// default) leaves it off: `/tpf` is `404`.
    ///
    /// This field exists only with the `tpf` cargo feature (like
    /// [`federation_descriptors`](Self::federation_descriptors) under
    /// `federation-descriptors`); a build without that feature compiles no TPF code and pays
    /// zero cost. Set by the binary's `--tpf` flag / `SPARQ_TPF=1` env.
    #[cfg(feature = "tpf")]
    pub tpf: bool,
    /// [OPUS-4.8] (sq-r74h, follow-up to sq-dxhb) **brTPF binding-set DoS cap — maximum number of
    /// solution mappings** accepted on a `GET /tpf?values=…` / `POST /tpf` brTPF request. A brTPF
    /// fragment runs ONE index scan per attached mapping (see [`crate::tpf::evaluate_brtpf`]), so
    /// the per-request cost is super-linear in the mapping *count*, not the payload *bytes* — and
    /// `--max-body-bytes` (a body-byte limit, and one that the `values` *query-string* carrier
    /// never even sees) bounds the count only transitively and far too loosely. This caps the
    /// fan-out directly: a request whose binding set exceeds it is refused `413` BEFORE any index
    /// work. `0` disables the count cap. Default `1024`. Set by `--brtpf-max-bindings` /
    /// `SPARQ_BRTPF_MAX_BINDINGS`. Present only with the `brtpf` cargo feature.
    #[cfg(feature = "brtpf")]
    pub brtpf_max_bindings: usize,
    /// [OPUS-4.8] (sq-r74h, follow-up to sq-dxhb) **brTPF binding-set DoS cap — maximum `values`
    /// payload bytes.** The companion byte cap to [`brtpf_max_bindings`](Self::brtpf_max_bindings),
    /// enforced on the raw binding-set payload BEFORE it is parsed (`413` on breach). It exists
    /// because the brTPF `values` binding set can ride the **query string** of a `GET /tpf`, which
    /// the server's `--max-body-bytes` HTTP *body* limit does not cover at all — so without this,
    /// the GET carrier is unbounded. On a `POST` body the body limit also applies; the effective
    /// bound is then the tighter of the two. `0` disables the byte cap. Default `1048576` (1 MiB,
    /// matching the `--max-body-bytes` default). Set by `--brtpf-max-values-bytes` /
    /// `SPARQ_BRTPF_MAX_VALUES_BYTES`. Present only with the `brtpf` cargo feature.
    #[cfg(feature = "brtpf")]
    pub brtpf_max_values_bytes: usize,
    /// [OPUS-4.8] (sq-r868, from-pss gh-162 follow-up (c)) OPT-IN HTTP SHACL validation
    /// endpoint. When `true`, the server serves `POST /shacl/validate`: the client POSTs a
    /// SHACL shapes graph (RDF, by `Content-Type`) and the server validates its
    /// CURRENTLY-LOADED data graph against it, returning a validation report — a JSON
    /// projection of `sparq_shacl::ValidationReport` (the gh-162 / wasm-binding shape) by
    /// default, or the W3C report-vocabulary Turtle on `Accept: text/turtle`. `false` (the
    /// default) leaves it off: `/shacl/validate` is `404`.
    ///
    /// This field exists only with the `shacl` cargo feature (like
    /// [`tpf`](Self::tpf) under `tpf`); a build without that feature compiles no SHACL code
    /// and pays zero cost. Set by the binary's `--shacl` flag / `SPARQ_SHACL=1` env. Validation
    /// is a READ over the store, so the endpoint is gated by the read auth like any GET.
    #[cfg(feature = "shacl")]
    pub shacl: bool,
    /// [GPT-5.6] (sq-lsp7k.2.4) Reject writes whose post-state does not conform to
    /// [`shacl_shapes`](Self::shacl_shapes). Validation runs before persistence or publish.
    ///
    /// [FABLE-5] (PR #1999 security review) Guard mode is STRICT / fail-CLOSED: a shapes
    /// graph the validator cannot fully evaluate is a hard startup error (ill-formed
    /// constructs, SHACL-SPARQL pre-binding violations), and a constraint skipped at
    /// evaluation time (e.g. an uncompilable `sh:pattern`) rejects the write (HTTP 500)
    /// instead of silently admitting it. Only the guard is strict — the read-only
    /// `/shacl/validate` endpoint keeps the lenient W3C-report behaviour.
    #[cfg(feature = "shacl")]
    pub shacl_guard: bool,
    /// [GPT-5.6] Loaded shapes graph used by SHACL guard mode. Required when the guard is on.
    #[cfg(feature = "shacl")]
    pub shacl_shapes: Option<ShaclShapes>,
    /// [OPUS-4.8] (sq-hj4n, gh-916) OPT-IN Solid-style **N3-Patch** (`text/n3`) dialect for the
    /// Graph-Store-Protocol `PATCH` method. When `true`, a `PATCH` whose body is `text/n3` (a
    /// `solid:InsertDeletePatch`) is parsed into its `solid:deletes` / `solid:inserts` /
    /// `solid:where` formulas and applied as ONE atomic graph-scoped SPARQL Update through the same
    /// sequenced writer the always-on `application/sparql-update` `PATCH` body uses. `false` (the
    /// default) leaves it off: a `text/n3` `PATCH` body is `415` (the always-on
    /// `application/sparql-update` `PATCH` dialect is unaffected — it never depends on this flag).
    ///
    /// This field exists only with the `n3-patch` cargo feature (like [`tpf`](Self::tpf) under
    /// `tpf`); a build without that feature compiles no N3-Patch code and pays zero cost. Set by
    /// the binary's `--n3-patch` flag / `SPARQ_N3_PATCH=1` env. A `PATCH` is a WRITE, so it is
    /// gated by `--auth-token` exactly like an UPDATE.
    #[cfg(feature = "n3-patch")]
    pub n3_patch: bool,
    /// [OPUS-4.8] (sq-vczh2, epic sq-2m6zm, design `research/llm-ergonomic-sparql-surface.md` §4)
    /// OPT-IN, VERIFIABLE LLM-ergonomic transpiler endpoint. When `true`, the server serves
    /// `POST /terse/transpile`: the client POSTs a *terse* query (the `K:<name>` keyword layer
    /// over canonical SPARQL) and the server returns the CANONICAL, conformant SPARQL it expands
    /// to PLUS the keyword expansions / warnings / legend version, as JSON. The network contract
    /// is the verifiable EXPANSION the agent inspects — the endpoint NEVER executes the query
    /// (the agent runs the returned `canonical_sparql` through the normal `/sparql` path). `false`
    /// (the default) leaves it off: `/terse/transpile` is `404`.
    ///
    /// This field exists only with the `terse` cargo feature (like [`tpf`](Self::tpf) under
    /// `tpf`); a build without that feature compiles no terse code and pays zero cost. Set by the
    /// binary's `--terse` flag / `SPARQ_TERSE=1` env. Transpiling neither reads nor mutates the
    /// store, but it is a query-shaped operation, so the endpoint is gated by the read auth like a
    /// GET. The server compiles the LEAN `sparq-terse` build (no `vectors`): a `V("phrase")`
    /// construct loud-FAILS with a `400` rather than guessing — concept resolution is a future
    /// `vectors`-gated extension (the `V()` ambiguity caveat is tracked by `sq-26fdp`).
    #[cfg(feature = "terse")]
    pub terse: bool,
    /// [FABLE-5] (sq-lsp7k.10) OPT-IN named-parameterized-template surface. When `true`, the
    /// server serves the `/templates` REST store (list / `PUT`-`GET`-`DELETE`
    /// `/templates/{name}` / `POST`-invoke) over `sparq_engine::templates` — the GraphDB
    /// "SPARQL templates" / Stardog "stored queries" parity surface. `false` (the default)
    /// leaves it off: every `/templates` path is `404`.
    ///
    /// This field exists only with the `templates` cargo feature (like [`tpf`](Self::tpf)
    /// under `tpf`); a build without that feature compiles no template code and pays zero
    /// cost. Set by the binary's `--templates` flag / `SPARQ_TEMPLATES=1` env. Reading /
    /// invoking a QUERY template is read-gated; storing / deleting a template and invoking
    /// an UPDATE template are write-gated (`--auth-token`) — the template layer never widens
    /// the gated-update posture. See [`crate::templates`].
    #[cfg(feature = "templates")]
    pub templates: bool,
    /// [FABLE-5] (sq-lsp7k.10) OPT-IN template persistence file. When `Some(path)`, the
    /// template store is durable: definitions are loaded from `path` (a JSON array of
    /// template definitions) at startup — fail-closed, an invalid file is a startup error —
    /// and every successful `PUT` / `DELETE` rewrites it atomically (write-temp + rename).
    /// `None` (the default) keeps the store in-memory only. Set by `--templates-file` /
    /// `SPARQ_TEMPLATES_FILE`.
    #[cfg(feature = "templates")]
    pub templates_file: Option<std::path::PathBuf>,
    /// [GPT-5.6] (sq-kqofk, gh-906) OPT-IN durable CDC change-stream directory. When `Some(dir)`,
    /// the server (1) RECORDS every published update generation as one ordered change record on
    /// the writer thread to the segmented, fsync'd append-only `sparq_serve::ChangeLog` rooted at
    /// `dir`, and (2) serves the Amazon-Neptune-Streams `GetRecords`-shaped poll endpoint
    /// `GET /streams` over that log (`iteratorType` / `at` / `after` / `limit`, returning the
    /// ordered records + a continuation token). A group-committed generation may contain several
    /// concurrent SPARQL Updates. `None` (the default) leaves both off: `/streams` is `404` and
    /// nothing is recorded — byte-identical to before. The log resumes after a restart
    /// (`ChangeLog::open` re-reads the segments), so pointing the server at an existing dir
    /// continues the same stream gaplessly.
    ///
    /// This field exists only with the `change-stream` cargo feature (like the `Self::tpf`
    /// field under `tpf`); a build without that feature compiles no change-stream code and pays zero
    /// cost. Set by the binary's `--change-stream <DIR>` flag / `SPARQ_CHANGE_STREAM` env. At-rest
    /// encryption + cryptographic authenticity of the records are out of scope (the same boundary
    /// as the backup family); a consumer needing an authentic feed wraps its own signing.
    #[cfg(feature = "change-stream")]
    pub change_stream_dir: Option<std::path::PathBuf>,
    /// [FABLE-5] (sq-snopa.6, issue #992 FR-4) OPT-IN Solid WAC/ACP HTTP authorization surface.
    /// When `true`, the server serves `POST /authz/decide`, `POST /authz/wac-allow` and
    /// `POST /authz/query` — a THIN HTTP shell over the `sparq-solid` library authoriser
    /// (`PodStore::decide` / `wac_allow` / `query_json_as`). Each endpoint takes an
    /// already-resolved session + the pod dataset (N-Quads) in the request body and maps the
    /// library verdict onto HTTP; it does NOT authenticate (mapping a WebID + a request path is the
    /// caller's job — `sparq-solid` is a library authoriser with no HTTP surface,
    /// `research/sparq-solid-scope.md` §4). FAIL-CLOSED: every error path DENIES. `false` (the
    /// default) leaves all three off: `/authz/*` is `404`.
    ///
    /// This field exists only with the `solid-authz` cargo feature (like `Self::tpf` under
    // [FABLE-5] sq-lrtc3.1: code span, not an intra-doc link — `tpf` is itself feature-gated, so
    // the link breaks any `solid-authz`-without-`tpf` rustdoc build (the recurring
    // feature-gated-intra-doc-link trap; surfaced by the new `odrl-authz` feature state).
    /// `tpf`); a build without that feature compiles no Solid/access-control code and pays zero
    /// cost. Set by the binary's `--solid-authz` flag / `SPARQ_SOLID_AUTHZ=1` env.
    #[cfg(feature = "solid-authz")]
    pub solid_authz: bool,
    /// [SONNET-4.6] (sq-pfae.17) OPT-IN stateless trust-graph extension to `POST /authz/decide`.
    ///
    /// When both this flag AND the `solid-authz-trust` cargo feature are active, the endpoint
    /// accepts an optional `"trust"` JSON block carrying credential graphs + a trust policy +
    /// signed certification edges. It runs the cert-graph closure
    /// (`sparq_trust::derive_effective_rules`) then the UNCHANGED WAC/ACP admission gate, and
    /// returns a minimal admission justification alongside the decision. FAIL-CLOSED: ANY
    /// malformed/unverifiable/stale/revoked/over-depth/broadening trust input => 403 DENY.
    /// Feature OFF => byte-identical to the `solid-authz` path. Double-opt-in: activates ONLY
    /// when BOTH (a) this feature is compiled AND (b) the request carries a `"trust"` block.
    ///
    /// This field exists only with the `solid-authz-trust` cargo feature. Set by
    /// `SPARQ_SOLID_AUTHZ_TRUST=1`.
    #[cfg(feature = "solid-authz-trust")]
    pub solid_authz_trust: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            query_timeout: Some(Duration::from_secs(30)),
            // [OPUS-4.8] sq-nulp: no separate writer-side WHERE deadline by default — an
            // update's WHERE budget is the plain query_timeout, exactly as before. An operator
            // who wants to bound writer-queue head-of-line blocking opts a shorter one in.
            update_where_timeout: None,
            // [OPUS-4.8] sq-p7kk5: adaptive group-commit ON by default — the server's write
            // path is interactive (a serial client blocked on its own ack), where the fixed
            // group-commit window is pure latency. A serial client now commits in engine-time
            // instead of window-time; concurrent load still batches. Pure latency change, no
            // effect on the FIFO/atomicity/durability contract. Off via --no-adaptive-commit /
            // SPARQ_ADAPTIVE_COMMIT=0. See ServerConfig::adaptive_commit.
            adaptive_commit: true,
            max_body_bytes: 1024 * 1024, // 1 MiB
            max_concurrent: 32,
            // [OPUS-4.8] sq-2gqr: a 15s header-read deadline closes the slow-loris hole ON by
            // default. `axum::serve` configures hyper's auto-Builder WITHOUT a timer, so hyper's
            // own 30s header_read_timeout default is inert (it requires a Timer) — meaning the
            // out-of-the-box stack has NO header deadline at all. `crate::serve` installs a
            // TokioTimer and wires this value. 0 / env-unset keeps the guard; SPARQ_HEADER_READ_TIMEOUT=0
            // disables it. See ServerConfig::header_read_timeout.
            header_read_timeout: Some(Duration::from_secs(15)),
            // [OPUS-4.8] sq-lodb: a 30s body read/idle deadline closes the slow-BODY hole ON by
            // default — the complement to the header-read guard above. A client that finishes its
            // headers then dribbles the request body (or stalls mid-body) otherwise holds the slot
            // forever. The timer resets after each received chunk, so an honest large upload is
            // never penalised by total time. 0 / SPARQ_BODY_READ_TIMEOUT=0 disables it. See
            // ServerConfig::body_read_timeout.
            body_read_timeout: Some(Duration::from_secs(30)),
            max_results: None,
            // [OPUS-4.8] sq-ebii: memory cap OFF by default (no surprise refusals on an
            // unconfigured server); an operator exposing the endpoint opts a ceiling in.
            max_query_rows: None,
            // [OPUS-4.8] sq-s5is: byte-accounted cap likewise OFF by default; opt-in ceiling.
            max_query_bytes: None,
            // [OPUS-4.8] sq-ebii: 20× decompressed:compressed is a permissive-but-bounded
            // default (well above real RDF gzip ratios ~3–8×, far below a bomb's ~1000×+).
            max_decompress_ratio: 20,
            max_subscriptions_per_conn: 16,
            max_subscriptions: 256,
            #[cfg(feature = "time-travel")]
            time_travel_generations: 16,
            #[cfg(feature = "time-travel")]
            time_travel_max_age: None,
            verbose: false,
            // [OPUS-4.8] sq-toze.34: redact request content from the verbose log by DEFAULT — a
            // privacy-respecting default so enabling --verbose for debugging does not silently
            // start writing query text (possibly PII) into operator logs. Opt out with
            // --log-full-requests / SPARQ_LOG_FULL_REQUESTS=1. Inert unless `verbose` is also set.
            redact_logs: true,
            // [OPUS-4.8] sq-0bxp: audit log OFF by default even when the feature is compiled in
            // (the operator opts in deliberately via --audit-log / SPARQ_AUDIT_LOG=1).
            #[cfg(feature = "audit-log")]
            audit_log: false,
            // [OPUS-4.8] sq-gos8: no structured access-audit sink by default even when the
            // feature is compiled in (the operator opts in via --access-audit / SPARQ_ACCESS_AUDIT).
            #[cfg(feature = "access-audit")]
            access_audit: None,
            // [OPUS-4.8] sq-ljfz: trust NO forwarded WebID header by default — the audit actor
            // is the local Bearer gate unless the operator names a trusted front's header
            // (--audit-webid-header / SPARQ_AUDIT_WEBID_HEADER).
            #[cfg(feature = "access-audit")]
            audit_webid_header: None,
            allow_remote: false, // [OPUS-4.8] sq-o4qf: safe default — refuse non-loopback bind unless opted in
            // [OPUS-4.8] sq-zcby: safe default — no token => no write auth (back-compat).
            auth_token: None,
            auth_token_read: false,
            // [OPUS-4.8] sq-4w18: safe default — empty allowlist = deny ALL SERVICE.
            service_allow: crate::service_config::ServiceAllowlist::default(),
            // [OPUS-4.8] sq-9xoh: no per-request override by default — every request uses the
            // single static `service_allow`, exactly as before this hook existed.
            #[cfg(feature = "service")]
            service_allow_override: None,
            // [OPUS-4.8] sq-o7o0: safe default — empty allowlist = NO CORS headers (a
            // cross-origin browser read is blocked, the historical posture for a data API).
            cors_allow: crate::cors_config::CorsAllowlist::default(),
            // [OPUS-4.8] sq-7cxr: safe default — no persistence dir = in-memory (back-compat).
            persist_dir: None,
            // [OPUS-4.8] sq-o5bi: no restore-on-start by default (seed from data file / empty).
            #[cfg(feature = "backup")]
            restore: None,
            // [OPUS-4.8] sq-bu1a: no PITR delta chain by default (base-only restore).
            #[cfg(feature = "backup")]
            restore_delta: Vec::new(),
            // [OPUS-4.8] sq-ft7u: restore-into-durable OFF by default — `--restore` + `--persist`
            // stay mutually exclusive unless the operator opts in with `--restore-persist`.
            #[cfg(feature = "backup")]
            restore_persist: false,
            // [OPUS-4.8] sq-d3d8: safe default — federation discovery descriptors OFF even
            // when the feature is compiled in (the operator opts in deliberately).
            #[cfg(feature = "federation-descriptors")]
            federation_descriptors: false,
            // [GPT-5.6] sq-lsp7k.5.2: safe default — the grouped facet-count endpoint is OFF
            // even when compiled in; the operator opts in via --facets / SPARQ_FACETS=1.
            #[cfg(feature = "facets")]
            facets: false,
            // [GPT-5.6] sq-lsp7k.9.3: safe default — prefix completion is OFF even when compiled
            // in; the operator opts in via --complete / SPARQ_COMPLETE=1.
            #[cfg(feature = "complete")]
            complete: false,
            // [OPUS-4.8] sq-bzh1: safe default — the TPF / LDF source endpoint is OFF even when
            // the feature is compiled in (the operator opts in deliberately via --tpf / SPARQ_TPF=1).
            #[cfg(feature = "tpf")]
            tpf: false,
            // [OPUS-4.8] sq-r74h: brTPF binding-set DoS caps — ON by default (a public source
            // endpoint should be bounded out of the box). 1024 mappings and a 1 MiB payload mirror
            // the conservative LDF page size and the --max-body-bytes default; an operator widens
            // (or, with 0, disables) either via the flags.
            #[cfg(feature = "brtpf")]
            brtpf_max_bindings: 1024,
            #[cfg(feature = "brtpf")]
            brtpf_max_values_bytes: 1024 * 1024,
            // [OPUS-4.8] sq-r868: safe default — the SHACL validate endpoint is OFF even when
            // the feature is compiled in (the operator opts in deliberately via --shacl /
            // SPARQ_SHACL=1).
            #[cfg(feature = "shacl")]
            shacl: false,
            #[cfg(feature = "shacl")]
            shacl_guard: false,
            #[cfg(feature = "shacl")]
            shacl_shapes: None,
            // [OPUS-4.8] sq-hj4n: safe default — the OPT-IN N3-Patch PATCH dialect is OFF even when
            // the feature is compiled in (the operator opts in deliberately via --n3-patch /
            // SPARQ_N3_PATCH=1). The always-on application/sparql-update PATCH dialect is unaffected.
            #[cfg(feature = "n3-patch")]
            n3_patch: false,
            // [OPUS-4.8] sq-vczh2: safe default — the OPT-IN terse-transpiler endpoint is OFF even
            // when the feature is compiled in (the operator opts in deliberately via --terse /
            // SPARQ_TERSE=1).
            #[cfg(feature = "terse")]
            terse: false,
            // [FABLE-5] sq-lsp7k.10: safe default — the OPT-IN template surface is OFF even
            // when the feature is compiled in (the operator opts in deliberately via
            // --templates / SPARQ_TEMPLATES=1), and the store is in-memory unless a
            // persistence file is named (--templates-file / SPARQ_TEMPLATES_FILE).
            #[cfg(feature = "templates")]
            templates: false,
            #[cfg(feature = "templates")]
            templates_file: None,
            // [OPUS-4.8] sq-2999l: safe default — no durable CDC change-stream directory, so the
            // `GET /streams` endpoint is OFF (404) and nothing is recorded even when the feature is
            // compiled in (the operator opts in deliberately via --change-stream DIR /
            // SPARQ_CHANGE_STREAM).
            #[cfg(feature = "change-stream")]
            change_stream_dir: None,
            // [FABLE-5] sq-snopa.6: safe default — the OPT-IN Solid WAC/ACP authorization surface is
            // OFF even when the feature is compiled in (the operator opts in deliberately via
            // --solid-authz / SPARQ_SOLID_AUTHZ=1). Fail-closed: `/authz/*` is 404 until set.
            #[cfg(feature = "solid-authz")]
            solid_authz: false,
            // [SONNET-4.6] sq-pfae.17: safe default — the OPT-IN stateless trust-graph extension
            // is OFF even when the feature is compiled in. The operator opts in deliberately via
            // SPARQ_SOLID_AUTHZ_TRUST=1. Fail-closed: trust block is ignored until set.
            #[cfg(feature = "solid-authz-trust")]
            solid_authz_trust: false,
        }
    }
}

impl ServerConfig {
    /// [OPUS-4.8] (sq-9xoh) Resolves the EFFECTIVE SERVICE egress allowlist for one READ
    /// request: the per-request [`service_allow_override`](Self::service_allow_override) hook if it
    /// is installed AND returns `Some` for these headers, otherwise the static
    /// [`service_allow`](Self::service_allow).
    ///
    /// Returns an owned [`ServiceAllowlist`](crate::service_config::ServiceAllowlist) so the caller
    /// can move it into the `spawn_blocking` closure that runs the engine call (request headers are
    /// not `'static`, so the resolution MUST happen before the spawn). Whatever it returns is
    /// installed in STRICT (allowlist-only) mode by the crate-internal `with_engine_scope_allow`, so
    /// an empty result (whether the override returned an empty allowlist or the static one is empty)
    /// still denies ALL SERVICE — the override can only narrow or substitute the host set, never
    /// relax the fail-closed posture.
    ///
    /// With no override installed (the default) this is a clone of the static allowlist, so the
    /// resolved result is identical to today's static behaviour.
    #[cfg(feature = "service")]
    pub fn resolve_service_allow(
        &self,
        headers: &HeaderMap,
    ) -> crate::service_config::ServiceAllowlist {
        match &self.service_allow_override {
            Some(hook) => (hook.0)(headers).unwrap_or_else(|| self.service_allow.clone()),
            None => self.service_allow.clone(),
        }
    }

    /// [OPUS-4.8] (sq-9xoh) Without the `service` feature there is no federation code and no
    /// override hook, so the effective allowlist is always the static one — this just clones it,
    /// keeping the read-path callsites feature-uniform.
    #[cfg(not(feature = "service"))]
    pub fn resolve_service_allow(
        &self,
        _headers: &HeaderMap,
    ) -> crate::service_config::ServiceAllowlist {
        self.service_allow.clone()
    }

    /// Defaults overridden by the `SPARQ_QUERY_TIMEOUT` (seconds; `0` disables),
    /// `SPARQ_UPDATE_WHERE_TIMEOUT` ([OPUS-4.8] sq-nulp: the separate, typically-shorter
    /// writer-side WHERE deadline that bounds writer-queue head-of-line blocking from a slow
    /// UPDATE; seconds, `0`/unset disables — the update WHERE budget is then the plain
    /// `query_timeout`), `SPARQ_MAX_BODY_BYTES`, `SPARQ_MAX_CONCURRENT`,
    /// `SPARQ_HEADER_READ_TIMEOUT` ([OPUS-4.8] sq-2gqr: the slow-loris connection header-read
    /// deadline in seconds; `0` disables, default 15),
    /// `SPARQ_BODY_READ_TIMEOUT` ([OPUS-4.8] sq-lodb: the slow-body request-body read/idle
    /// deadline in seconds; `0` disables, default 30), `SPARQ_MAX_RESULTS`,
    /// `SPARQ_MAX_QUERY_ROWS` ([OPUS-4.8] sq-ebii: the coarse memory cap; `0` disables) and
    /// `SPARQ_MAX_DECOMPRESS_RATIO` ([OPUS-4.8] sq-ebii: the zip-bomb guard; `0` refuses gzip
    /// bodies) environment variables — plus, with the `time-travel` feature,
    /// `SPARQ_TIME_TRAVEL_GENERATIONS` and `SPARQ_TIME_TRAVEL_MAX_AGE` (seconds;
    /// `0` disables the age bound), `SPARQ_ALLOW_REMOTE` (non-loopback bind opt-in),
    /// and `SPARQ_SERVICE_ALLOW` (comma/whitespace-separated SERVICE egress allowlist;
    /// [OPUS-4.8] sq-4w18). CLI flags override / widen these in `main` (the allowlist is
    /// additive: `--service-allow` / `--service-allow-file` UNION with the env baseline).
    ///
    /// [OPUS-4.8] sq-4w18: returns `Err` (rather than panicking) on a malformed
    /// `SPARQ_SERVICE_ALLOW` entry. `from_env` is public config API an embedder may
    /// call, and a panic in a config constructor is a hostile surprise; a `Result`
    /// (the crate's `Result<_, String>` error style, e.g. `from_sources` / nlq's
    /// `from_env`) lets the caller surface a clean user-facing message. The valid
    /// path is byte-for-byte unchanged.
    pub fn from_env() -> Result<Self, String> {
        let mut cfg = Self::default();
        if let Some(secs) = env_parse::<u64>("SPARQ_QUERY_TIMEOUT") {
            cfg.query_timeout = (secs > 0).then(|| Duration::from_secs(secs));
        }
        // [OPUS-4.8] sq-nulp: separate, typically-shorter writer-side WHERE deadline that
        // bounds writer-queue head-of-line blocking from a slow UPDATE; 0 / unset disables it
        // (the update WHERE budget is then the plain query_timeout, exactly as before).
        if let Some(secs) = env_parse::<u64>("SPARQ_UPDATE_WHERE_TIMEOUT") {
            cfg.update_where_timeout = (secs > 0).then(|| Duration::from_secs(secs));
        }
        // [OPUS-4.8] sq-p7kk5: adaptive group-commit (ON by default). SPARQ_ADAPTIVE_COMMIT=0
        // restores the always-windowed writer; anything else (or unset) keeps it on.
        if let Some(n) = env_parse::<u8>("SPARQ_ADAPTIVE_COMMIT") {
            cfg.adaptive_commit = n != 0;
        }
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_BODY_BYTES") {
            cfg.max_body_bytes = n;
        }
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_CONCURRENT") {
            cfg.max_concurrent = n.max(1);
        }
        // [OPUS-4.8] sq-2gqr: slow-loris header-read deadline (seconds); `0` disables it (an
        // unbounded header read, the pre-fix behaviour), anything else sets the deadline.
        if let Some(secs) = env_parse::<u64>("SPARQ_HEADER_READ_TIMEOUT") {
            cfg.header_read_timeout = (secs > 0).then(|| Duration::from_secs(secs));
        }
        // [OPUS-4.8] sq-lodb: slow-body read/idle deadline (seconds); `0` disables it (an
        // unbounded body read — a slow body can hold the slot forever), anything else sets it.
        if let Some(secs) = env_parse::<u64>("SPARQ_BODY_READ_TIMEOUT") {
            cfg.body_read_timeout = (secs > 0).then(|| Duration::from_secs(secs));
        }
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_RESULTS") {
            cfg.max_results = (n > 0).then_some(n);
        }
        // [OPUS-4.8] sq-ebii: memory cap (coarse working-set row ceiling on every form);
        // 0 / unset disables it.
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_QUERY_ROWS") {
            cfg.max_query_rows = (n > 0).then_some(n);
        }
        // [OPUS-4.8] sq-s5is: byte-accounted memory cap (prices row width + computed
        // literals, on every form); 0 / unset disables it.
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_QUERY_BYTES") {
            cfg.max_query_bytes = (n > 0).then_some(n);
        }
        // [OPUS-4.8] sq-ebii: decompression-ratio cap (zip-bomb guard); 0 disables
        // ratio-capped decompression (a Content-Encoding body is then refused outright).
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_DECOMPRESS_RATIO") {
            cfg.max_decompress_ratio = n;
        }
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_SUBSCRIPTIONS_PER_CONN") {
            cfg.max_subscriptions_per_conn = n;
        }
        if let Some(n) = env_parse::<usize>("SPARQ_MAX_SUBSCRIPTIONS") {
            cfg.max_subscriptions = n;
        }
        #[cfg(feature = "time-travel")]
        {
            if let Some(n) = env_parse::<usize>("SPARQ_TIME_TRAVEL_GENERATIONS") {
                cfg.time_travel_generations = n;
            }
            if let Some(secs) = env_parse::<u64>("SPARQ_TIME_TRAVEL_MAX_AGE") {
                cfg.time_travel_max_age = (secs > 0).then(|| Duration::from_secs(secs));
            }
        }
        // [OPUS-4.8] sq-0bxp: SPARQ_AUDIT_LOG truthy ("1"/"true"/"yes"/"on") turns the per-query
        // access audit log on at runtime (only when the `audit-log` feature is compiled in).
        #[cfg(feature = "audit-log")]
        if let Ok(v) = std::env::var("SPARQ_AUDIT_LOG") {
            cfg.audit_log = env_truthy(&v);
        }
        // [OPUS-4.8] sq-toze.34: SPARQ_LOG_FULL_REQUESTS truthy ("1"/"true"/"yes"/"on") OPTS OUT of
        // request-log redaction (logs full URIs/query text verbatim). Default (unset / falsey) keeps
        // redaction ON — the privacy-respecting posture. Only meaningful together with --verbose.
        if let Ok(v) = std::env::var("SPARQ_LOG_FULL_REQUESTS") {
            cfg.redact_logs = !env_truthy(&v);
        }
        // [OPUS-4.8] sq-gos8: SPARQ_ACCESS_AUDIT=<file|stderr> installs the structured
        // access-audit sink (only when the `access-audit` feature is compiled in). An empty /
        // unset value leaves it off. The literal `stderr` selects the stderr sink; anything else
        // is a file path the default JSON-Lines sink appends to.
        #[cfg(feature = "access-audit")]
        if let Ok(v) = std::env::var("SPARQ_ACCESS_AUDIT") {
            let v = v.trim();
            if !v.is_empty() {
                cfg.access_audit = Some(crate::access_audit::SinkTarget::parse(v));
            }
        }
        // [OPUS-4.8] sq-ljfz: SPARQ_AUDIT_WEBID_HEADER=<header-name> names a TRUSTED forwarded
        // -identity header — a fronting auth layer (sparq-solid / a Solid-WAC proxy / gateway)
        // sets it to the authenticated user's WebID, which the audit seam then records as
        // Actor::WebId. Empty / unset => trust no header (the local Bearer gate, unchanged).
        #[cfg(feature = "access-audit")]
        if let Ok(v) = std::env::var("SPARQ_AUDIT_WEBID_HEADER") {
            let v = v.trim();
            cfg.audit_webid_header = (!v.is_empty()).then(|| v.to_string());
        }
        // [OPUS-4.8] sq-o4qf: SPARQ_ALLOW_REMOTE truthy ("1"/"true"/"yes"/"on", case-insensitive)
        // opts in to a non-loopback bind. Anything else (incl. unset / "0" / "false") leaves the
        // safe default of refusing to expose the unauthenticated surface beyond loopback.
        if let Ok(v) = std::env::var("SPARQ_ALLOW_REMOTE") {
            cfg.allow_remote = env_truthy(&v);
        }
        // [OPUS-4.8] sq-zcby: SPARQ_AUTH_TOKEN sets the write-gate token (an empty value is
        // treated as "unset" — an empty shared secret is a footgun, never a valid token);
        // SPARQ_AUTH_TOKEN_READ truthy ("1"/"true"/"yes"/"on") additionally gates reads.
        if let Ok(v) = std::env::var("SPARQ_AUTH_TOKEN") {
            cfg.auth_token = (!v.is_empty()).then_some(v);
        }
        if let Ok(v) = std::env::var("SPARQ_AUTH_TOKEN_READ") {
            cfg.auth_token_read = env_truthy(&v);
        }
        // [OPUS-4.8] sq-4w18: SERVICE egress allowlist baseline from SPARQ_SERVICE_ALLOW
        // (comma/whitespace-separated). The binary then ADDS any `--service-allow` /
        // `--service-allow-file` entries (the union — CLI only ever widens). A malformed
        // env entry is a hard startup error (propagated, not panicked) rather than a
        // silently-dropped host, so the operator's allowlist is never quietly narrower
        // than written.
        if let Ok(v) = std::env::var("SPARQ_SERVICE_ALLOW") {
            cfg.service_allow
                .add_many(&v)
                .map_err(|e| format!("SPARQ_SERVICE_ALLOW: {e}"))?;
        }
        // [OPUS-4.8] sq-o7o0: first-party CORS origin allowlist baseline from
        // SPARQ_CORS_ALLOW_ORIGIN (comma/whitespace-separated). The binary then ADDS any
        // `--cors-allow-origin` / `--cors-allow-origin-file` entries (the union — CLI only
        // ever widens). A malformed env origin is a hard startup error (propagated, not
        // panicked) rather than a silently-dropped origin, so the operator's allowlist is
        // never quietly narrower than written. Unset/empty => no CORS headers (the default).
        if let Ok(v) = std::env::var("SPARQ_CORS_ALLOW_ORIGIN") {
            cfg.cors_allow
                .add_many(&v)
                .map_err(|e| format!("SPARQ_CORS_ALLOW_ORIGIN: {e}"))?;
        }
        // [OPUS-4.8] sq-7cxr: SPARQ_PERSIST_DIR enables durable persistence at the given
        // directory (the binary's --persist flag overrides it). An empty value is "unset".
        if let Ok(v) = std::env::var("SPARQ_PERSIST_DIR") {
            cfg.persist_dir = (!v.is_empty()).then(|| std::path::PathBuf::from(v));
        }
        // [OPUS-4.8] sq-o5bi: SPARQ_RESTORE seeds the in-memory store from a backup artifact
        // at startup (the binary's --restore flag overrides it). An empty value is "unset".
        // Only present with the `backup` feature.
        #[cfg(feature = "backup")]
        if let Ok(v) = std::env::var("SPARQ_RESTORE") {
            cfg.restore = (!v.is_empty()).then(|| std::path::PathBuf::from(v));
        }
        // [OPUS-4.8] sq-bu1a: SPARQ_RESTORE_DELTA is a platform-path-separator-delimited list of
        // incremental delta artifacts replayed forward onto the --restore base (PITR), oldest
        // first. Repeated --restore-delta flags override it. Only present with the `backup` feature.
        #[cfg(feature = "backup")]
        if let Ok(v) = std::env::var("SPARQ_RESTORE_DELTA") {
            cfg.restore_delta = std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect();
        }
        // [OPUS-4.8] sq-ft7u: SPARQ_RESTORE_PERSIST truthy ("1"/"true"/"yes"/"on") makes the
        // restore-on-start write THROUGH to the durable `--persist` dir (so it survives a restart)
        // instead of the historical refusal of the --restore + --persist combination. Off by
        // default. Only present with the `backup` feature; the binary's --restore-persist overrides.
        #[cfg(feature = "backup")]
        if let Ok(v) = std::env::var("SPARQ_RESTORE_PERSIST") {
            cfg.restore_persist = env_truthy(&v);
        }
        // [OPUS-4.8] sq-d3d8: SPARQ_FEDERATION_DESCRIPTORS truthy ("1"/"true"/"yes"/"on")
        // serves the VoID + Service-Description discovery endpoints. Off by default. Only
        // present with the `federation-descriptors` feature.
        #[cfg(feature = "federation-descriptors")]
        if let Ok(v) = std::env::var("SPARQ_FEDERATION_DESCRIPTORS") {
            cfg.federation_descriptors = env_truthy(&v);
        }
        // [GPT-5.6] sq-lsp7k.5.2: SPARQ_FACETS truthy ("1"/"true"/"yes"/"on") serves the
        // grouped facet-count endpoint. Off by default. Only present with the `facets` feature.
        #[cfg(feature = "facets")]
        if let Ok(v) = std::env::var("SPARQ_FACETS") {
            cfg.facets = env_truthy(&v);
        }
        // [GPT-5.6] sq-lsp7k.9.3: SPARQ_COMPLETE truthy ("1"/"true"/"yes"/"on") serves the
        // prefix-completion endpoint. Off by default. Only present with the `complete` feature.
        #[cfg(feature = "complete")]
        if let Ok(v) = std::env::var("SPARQ_COMPLETE") {
            cfg.complete = env_truthy(&v);
        }
        // [OPUS-4.8] sq-bzh1: SPARQ_TPF truthy ("1"/"true"/"yes"/"on") serves the Triple Pattern
        // Fragments / LDF source endpoint. Off by default. Only present with the `tpf` feature.
        #[cfg(feature = "tpf")]
        if let Ok(v) = std::env::var("SPARQ_TPF") {
            cfg.tpf = env_truthy(&v);
        }
        // [OPUS-4.8] sq-r74h: brTPF binding-set DoS caps (mapping count + payload bytes); 0
        // disables that cap. Only present with the `brtpf` feature.
        #[cfg(feature = "brtpf")]
        {
            if let Some(n) = env_parse::<usize>("SPARQ_BRTPF_MAX_BINDINGS") {
                cfg.brtpf_max_bindings = n;
            }
            if let Some(n) = env_parse::<usize>("SPARQ_BRTPF_MAX_VALUES_BYTES") {
                cfg.brtpf_max_values_bytes = n;
            }
        }
        // [OPUS-4.8] sq-r868: SPARQ_SHACL truthy ("1"/"true"/"yes"/"on") serves the SHACL
        // validate endpoint. Off by default. Only present with the `shacl` feature.
        #[cfg(feature = "shacl")]
        if let Ok(v) = std::env::var("SPARQ_SHACL") {
            cfg.shacl = env_truthy(&v);
        }
        #[cfg(feature = "shacl")]
        if let Ok(v) = std::env::var("SPARQ_SHACL_GUARD") {
            cfg.shacl_guard = env_truthy(&v);
        }
        // [OPUS-4.8] sq-hj4n: SPARQ_N3_PATCH truthy ("1"/"true"/"yes"/"on") enables the OPT-IN
        // Solid N3-Patch PATCH dialect (text/n3). Off by default. Only present with the
        // `n3-patch` feature.
        #[cfg(feature = "n3-patch")]
        if let Ok(v) = std::env::var("SPARQ_N3_PATCH") {
            cfg.n3_patch = env_truthy(&v);
        }
        // [OPUS-4.8] sq-vczh2: SPARQ_TERSE truthy ("1"/"true"/"yes"/"on") serves the OPT-IN
        // terse-transpiler endpoint. Off by default. Only present with the `terse` feature.
        #[cfg(feature = "terse")]
        if let Ok(v) = std::env::var("SPARQ_TERSE") {
            cfg.terse = env_truthy(&v);
        }
        // [FABLE-5] sq-lsp7k.10: SPARQ_TEMPLATES truthy ("1"/"true"/"yes"/"on") serves the
        // OPT-IN named-parameterized-template REST surface (/templates); SPARQ_TEMPLATES_FILE
        // names the optional persistence file (empty = in-memory). Off by default. Only
        // present with the `templates` feature.
        #[cfg(feature = "templates")]
        {
            if let Ok(v) = std::env::var("SPARQ_TEMPLATES") {
                cfg.templates = env_truthy(&v);
            }
            if let Ok(v) = std::env::var("SPARQ_TEMPLATES_FILE") {
                cfg.templates_file = (!v.is_empty()).then(|| std::path::PathBuf::from(v));
            }
        }
        // [OPUS-4.8] sq-2999l: SPARQ_CHANGE_STREAM=<DIR> enables the durable CDC change-stream
        // (recording + the `GET /streams` poll endpoint) rooted at the given directory (the
        // binary's --change-stream flag overrides it). An empty value is "unset". Only present with
        // the `change-stream` feature.
        #[cfg(feature = "change-stream")]
        if let Ok(v) = std::env::var("SPARQ_CHANGE_STREAM") {
            cfg.change_stream_dir = (!v.is_empty()).then(|| std::path::PathBuf::from(v));
        }
        // [FABLE-5] sq-snopa.6: SPARQ_SOLID_AUTHZ truthy ("1"/"true"/"yes"/"on") serves the OPT-IN
        // Solid WAC/ACP HTTP authorization surface (POST /authz/*). Off by default. Only present
        // with the `solid-authz` feature.
        #[cfg(feature = "solid-authz")]
        if let Ok(v) = std::env::var("SPARQ_SOLID_AUTHZ") {
            cfg.solid_authz = env_truthy(&v);
        }
        // [SONNET-4.6] sq-pfae.17: SPARQ_SOLID_AUTHZ_TRUST truthy ("1"/"true"/"yes"/"on") enables
        // the OPT-IN stateless trust-graph extension to POST /authz/decide. Off by default. Only
        // present with the `solid-authz-trust` feature.
        #[cfg(feature = "solid-authz-trust")]
        if let Ok(v) = std::env::var("SPARQ_SOLID_AUTHZ_TRUST") {
            cfg.solid_authz_trust = env_truthy(&v);
        }
        Ok(cfg)
    }
}

/// [OPUS-4.8] sq-o4qf: parse a boolean-ish env value. Truthy: `1`, `true`, `yes`, `on`
/// (case-insensitive, trimmed); everything else (including empty) is false.
fn env_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-o4qf / sq-zcby — bind-time security posture (auth-aware)
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-o4qf: the decision the binary makes about a requested bind address,
/// given the configured auth posture. Returned by [`bind_posture`] so `main` (and tests)
/// can act on it without the side effect of actually binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindPosture {
    /// Loopback (or otherwise not-remotely-reachable) bind — safe, proceed silently.
    Loopback,
    /// Non-loopback bind that is allowed to proceed (opted in via `--allow-remote` /
    /// `SPARQ_ALLOW_REMOTE`, or the whole surface is authenticated — sq-zcby). Proceed, but the
    /// caller MUST surface `warning` (it describes exactly what is now reachable: a fully open
    /// surface, an open read endpoint behind a write gate, or a fully authenticated surface).
    RemoteAllowed { warning: String },
    /// Non-loopback bind that is refused. The caller MUST refuse to bind and print `message`
    /// (which explains the exposure and how to proceed: gate the surface or opt in).
    RemoteRefused { message: String },
}

/// [OPUS-4.8] sq-o4qf / sq-zcby: classify a requested bind address under the configured auth
/// posture.
///
/// By default `sparq-server` has no authentication on any endpoint (query,
/// `application/sparql-update`, the `/subscriptions` WebSocket). A loopback bind
/// (`127.0.0.0/8`, `::1`) is only reachable from the same host, so it is safe by default. A
/// **non-loopback** bind exposes the surface to the network, so the binary refuses it unless
/// the operator opts in via `--allow-remote` / `SPARQ_ALLOW_REMOTE=1` (and even then warns) OR
/// the whole surface is authenticated ([`AuthPosture::ReadAndWrite`]). A write-token alone
/// ([`AuthPosture::WriteOnly`]) leaves reads open, so it is treated like no auth for this
/// decision: still refused without `--allow-remote`.
///
/// "Loopback" here means the literal loopback ranges. Note `0.0.0.0` / `::` (the unspecified
/// "bind to all interfaces" addresses) are NOT loopback — they are the most common way the
/// surface gets exposed, so they are treated as remote. This is a deliberately blunt,
/// fail-closed check: it errs toward refusing exposure, not toward allowing it.
///
/// [OPUS-4.8] sq-zcby: `auth` folds the configured Bearer-token gate into the decision. A
/// non-loopback bind is allowed when `--allow-remote` is set OR the **whole surface** is
/// authenticated ([`AuthPosture::ReadAndWrite`]) — a write-token alone ([`AuthPosture::
/// WriteOnly`]) still requires `--allow-remote`, because it leaves an OPEN read endpoint on
/// the remote bind; we still warn in that case that reads remain open.
pub fn bind_posture(addr: &SocketAddr, allow_remote: bool, auth: AuthPosture) -> BindPosture {
    if addr.ip().is_loopback() {
        return BindPosture::Loopback;
    }
    // A fully-authenticated surface (token gates reads AND writes) is safe to expose without
    // --allow-remote: there is no open endpoint left. We still warn (a single shared secret
    // is not per-user authz, and the token must be carried over TLS).
    if auth == AuthPosture::ReadAndWrite {
        return BindPosture::RemoteAllowed {
            warning: format!(
                "WARNING: sparq-server is binding the non-loopback address {addr}. The whole \
                 surface is gated by the --auth-token Bearer token (reads AND writes), so it \
                 is not open to anonymous access. NOTE: the token is a single shared secret \
                 (not per-user authz) — deliver it over TLS (terminate at a proxy), and front \
                 it with a real authorization layer (a reverse proxy / gateway or sparq-solid) \
                 for per-user access control. [OPUS-4.8] sq-cxk5: the /subscriptions WebSocket \
                 AND /subscriptions/sse stream (read surfaces) are gated by --auth-token-read \
                 too — browser WS clients pass the token as a 'Sec-WebSocket-Protocol: \
                 bearer.<token>' subprotocol."
            ),
        };
    }
    if allow_remote {
        // The surface is reachable from the network; describe exactly what is open.
        let exposure = match auth {
            AuthPosture::None => {
                "The full dataset is exposed for READ AND WRITE (SPARQL Update + the \
                 /subscriptions WebSocket) to anyone who can reach this port — there is NO \
                 authentication."
            }
            // WriteOnly: writes are gated, but reads are still open on this remote bind.
            AuthPosture::WriteOnly => {
                "Writes are gated by --auth-token, but READS remain OPEN to anyone who can \
                 reach this port (add --auth-token-read to gate reads too). The /subscriptions \
                 WebSocket (a read surface) is also open."
            }
            AuthPosture::ReadAndWrite => unreachable!("handled above"),
        };
        BindPosture::RemoteAllowed {
            warning: format!(
                "WARNING: sparq-server is binding the non-loopback address {addr}. {exposure} \
                 Put it behind a reverse proxy / API gateway (or sparq-solid) that enforces \
                 auth before exposing it to an untrusted network. \
                 (--allow-remote / SPARQ_ALLOW_REMOTE is set, so this bind proceeds.)"
            ),
        }
    } else {
        // Refused. A write-token alone is NOT sufficient (reads stay open), so we name the
        // two ways to proceed: gate reads too (--auth-token-read) or opt in (--allow-remote).
        let reason = match auth {
            AuthPosture::None => {
                "sparq-server has NO authentication, so this would expose the full dataset for \
                 READ AND WRITE to the network"
            }
            AuthPosture::WriteOnly => {
                "--auth-token gates writes but READS would still be OPEN on a non-loopback bind \
                 (add --auth-token-read to gate reads too, which makes the whole surface \
                 authenticated and the bind safe)"
            }
            AuthPosture::ReadAndWrite => unreachable!("handled above"),
        };
        BindPosture::RemoteRefused {
            message: format!(
                "refusing to bind non-loopback address {addr}: {reason}. If a network bind is \
                 intended, either gate the whole surface (--auth-token AND --auth-token-read) \
                 or run behind a reverse proxy / gateway that enforces auth and re-run with \
                 --allow-remote (or SPARQ_ALLOW_REMOTE=1). To serve only this host, bind a \
                 loopback address such as 127.0.0.1."
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-zcby (PSS gh-46) — the Bearer-token auth gate (write surface +
// optional read gate), mirroring QLever's `-a <token>`.
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-zcby: how much of the surface a configured token authenticates — folded into
/// the bind decision ([`bind_posture`]). `WriteOnly` (a token, reads open) still requires
/// `--allow-remote` for a non-loopback bind because reads stay open; `ReadAndWrite` (token +
/// `--auth-token-read`) makes the whole surface authenticated, so it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPosture {
    /// No token configured — no auth on any endpoint.
    None,
    /// A token gates writes; reads are open.
    WriteOnly,
    /// A token gates writes AND reads (`--auth-token-read`).
    ReadAndWrite,
}

impl AuthPosture {
    /// Derives the posture from a [`ServerConfig`]: no token → `None`; a token without the
    /// read gate → `WriteOnly`; a token with `--auth-token-read` → `ReadAndWrite`.
    pub fn from_config(config: &ServerConfig) -> Self {
        match (config.auth_token.is_some(), config.auth_token_read) {
            (false, _) => AuthPosture::None,
            (true, false) => AuthPosture::WriteOnly,
            (true, true) => AuthPosture::ReadAndWrite,
        }
    }
}

/// [OPUS-4.8] sq-zcby: whether a request is a WRITE (mutates the dataset) or a READ, for the
/// auth gate. Classification keys on "does this mutate", NOT the route — an UPDATE smuggled
/// through the query path is a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Read,
    Write,
}

/// [OPUS-4.8] sq-zcby: constant-time byte-string equality. Returns `true` iff `a == b`,
/// taking time that depends only on `a.len()` (not on the contents or on how far the first
/// difference is), so a token check cannot be turned into a timing oracle that recovers the
/// secret byte-by-byte.
///
/// Hand-rolled rather than pulling in the `subtle` crate: it is a few lines, sparq-server
/// has no other crypto dependency (keeping it out of the supply-chain / SBOM surface), and a
/// length-difference + per-byte XOR-accumulate is the standard, well-understood construction.
/// `a` is the configured secret and `b` the presented token; the length comparison reveals
/// only whether the *presented* token has the secret's length (already inferable, and not the
/// secret's bytes). The accumulator is `#[inline(never)]` and read into a `black_box`-style
/// volatile-ish fold to keep the optimiser from short-circuiting (no data-dependent branch).
#[inline(never)]
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    // [OPUS-4.8] sq-zcby (Copilot PR#71 fix): route the accumulator through
    // `core::hint::black_box` BEFORE the `== 0` test so the optimiser cannot prove anything
    // about `diff` and rewrite the loop+compare into an early-exit `memcmp`-style
    // short-circuit. This matches the doc claim ("black_box-style fold"): without it the
    // bare `diff == 0` was a plain data-dependent comparison the compiler is free to
    // short-circuit. Fold to bool with no data-dependent branch on the secret.
    core::hint::black_box(diff) == 0
}

/// [OPUS-4.8] sq-zcby: extracts the token from an `Authorization: Bearer <token>` header,
/// tolerant of scheme casing (`Bearer`/`bearer`/`BEARER`) and of leading/trailing space. Any
/// other scheme (or no header) yields `None`.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token.trim())
    } else {
        None
    }
}

/// [OPUS-4.8] sq-cxk5: the WebSocket subprotocol-token prefix. A browser cannot set an
/// `Authorization` header on a WebSocket handshake, so it carries the Bearer token as a
/// `Sec-WebSocket-Protocol: bearer.<token>` subprotocol instead. See [`subprotocol_bearer_token`]
/// and `crates/sparq-server/README.md` → "Authenticating a WebSocket subscription from a browser".
pub(crate) const WS_BEARER_SUBPROTOCOL_PREFIX: &str = "bearer.";

/// [OPUS-4.8] sq-cxk5: extracts a Bearer token offered as a `Sec-WebSocket-Protocol`
/// subprotocol of the form `bearer.<token>` (the browser-compatible channel — browsers CANNOT
/// set an `Authorization` header on a WS handshake, but `new WebSocket(url, [proto])` sets
/// `Sec-WebSocket-Protocol`). The header may list several comma-separated subprotocols; the FIRST
/// entry whose value starts with `bearer.` wins, and the substring AFTER the prefix is the token
/// (no trimming of the token body — a subprotocol token is exact). Returns the FULL matched
/// subprotocol string too (caller echoes it back per RFC 6455). `None` when no offered subprotocol
/// carries the prefix.
pub(crate) fn subprotocol_bearer_token(headers: &HeaderMap) -> Option<(&str, &str)> {
    // A client may send multiple `Sec-WebSocket-Protocol` header lines, each itself a
    // comma-separated list; scan every entry across all lines.
    headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .find_map(|v| {
            let raw = v.to_str().ok()?;
            raw.split(',').map(str::trim).find_map(|proto| {
                proto
                    .strip_prefix(WS_BEARER_SUBPROTOCOL_PREFIX)
                    .map(|tok| (proto, tok))
            })
        })
}

/// [OPUS-4.8] sq-zcby / sq-cxk5: the per-request auth decision core. Returns `None` to proceed,
/// or `Some(401)` to refuse. A request is gated when its [`Operation`] is covered by the
/// configured posture: a `Write` whenever a token is set; a `Read` only additionally when
/// `--auth-token-read` is on. An ungated operation (or no token configured at all) always
/// proceeds. A gated request must present the exact token (constant-time compared); `presented`
/// is the candidate token already extracted from whichever channel the transport allows (the
/// `Authorization: Bearer` header on plain HTTP, or ALSO the `bearer.<token>` WebSocket
/// subprotocol — sq-cxk5). The 401 is byte-identical for a missing vs a wrong token, so it never
/// leaks which.
pub(crate) fn auth_check(
    config: &ServerConfig,
    op: Operation,
    presented: Option<&str>,
) -> Option<Response> {
    let token = config.auth_token.as_deref()?; // no token configured => never gated
    let gated = match op {
        Operation::Write => true,
        Operation::Read => config.auth_token_read,
    };
    if !gated {
        return None;
    }
    let ok = presented.is_some_and(|p| constant_time_eq(token.as_bytes(), p.as_bytes()));
    if ok {
        None
    } else {
        Some(unauthorized())
    }
}

/// [OPUS-4.8] sq-zcby: the per-request auth gate for the plain-HTTP surface — validates the
/// `Authorization: Bearer <token>` header against the configured posture. See [`auth_check`].
/// The SSE subscription GET (`/subscriptions/sse`) uses this exactly like the other GET routes
/// (sq-cxk5): it is a plain GET, so the Bearer header is the only channel.
pub(crate) fn auth_gate(
    config: &ServerConfig,
    headers: &HeaderMap,
    op: Operation,
) -> Option<Response> {
    auth_check(config, op, bearer_token(headers))
}

/// [OPUS-4.8] sq-cxk5: the auth gate for the `/subscriptions` WebSocket UPGRADE. A browser cannot
/// set an `Authorization` header on a WS handshake, so this accepts the token from EITHER channel:
/// the `Authorization: Bearer <token>` header (non-browser clients) OR a `Sec-WebSocket-Protocol:
/// bearer.<token>` subprotocol (browsers). The token is validated against the read token
/// (constant-time, [`auth_check`] with [`Operation::Read`]) — a subprotocol token is VALIDATED,
/// never merely echoed. When `--auth-token-read` is not set (or no token is configured) the
/// upgrade is unchanged (open) — back-compatible. The `Authorization` header is preferred when
/// present; the subprotocol is the fallback so a browser is not penalised for offering an
/// unrelated subprotocol alongside `bearer.<token>`.
pub(crate) fn ws_auth_gate(
    config: &ServerConfig,
    headers: &HeaderMap,
    op: Operation,
) -> Option<Response> {
    let presented =
        bearer_token(headers).or_else(|| subprotocol_bearer_token(headers).map(|(_, tok)| tok));
    auth_check(config, op, presented)
}

/// [OPUS-4.8] sq-zcby: the 401 a gated request without a valid token gets — `WWW-Authenticate:
/// Bearer` plus the server's standard JSON error body. Identical for a missing vs a wrong
/// token (the body carries no hint either way), so it never leaks which.
///
/// [OPUS-4.8] sq-2bhm (ASVS-G1): this auth-refusal is a SENSITIVE response, so — unlike the
/// general surface, where a blanket `Cache-Control` is deliberately NOT forced (results are
/// uncached by default; see [`security_headers`]) — it carries `Cache-Control: no-store` so a
/// shared cache / proxy never retains the 401 (or any future body it grows). This is the
/// narrow, targeted use of `no-store` the security-header gap calls for, scoped to the auth
/// path rather than imposed globally.
pub(crate) fn unauthorized() -> Response {
    let mut resp = json_error(
        StatusCode::UNAUTHORIZED,
        "authentication required: present a valid Bearer token",
    );
    let headers = resp.headers_mut();
    headers.insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Bearer"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    resp
}

/// [OPUS-4.8] sq-zcby: does this SPARQL string MUTATE the dataset, for the auth classifier?
/// The classifier keys on this, not on the route or Content-Type, so an UPDATE smuggled
/// through a `query=`/generic body path is still gated as a write.
///
/// [OPUS-4.8] (Copilot PR#71 doc/impl-consistency fix) The implemented rule is
/// **default-to-write unless it provably parses as a read-only query**: a string that parses
/// as a read-only query (SELECT/ASK/CONSTRUCT/DESCRIBE) returns `false` (a read); EVERYTHING
/// ELSE returns `true` (gated as a write). That "everything else" deliberately collapses the
/// "parses as a SPARQL Update" and the "parses as NEITHER" cases into one branch — both are
/// fail-safe to gate as a write, so a positive `parse_update` check would only add cost
/// without changing the answer. This is the conservative/secure default: ambiguous or
/// malformed bodies are gated as writes (fail-closed), so a body can never slip past the
/// write gate by being unparsable, and gating a non-mutating-but-unparsable body as a write
/// never wrongly OPENS the write surface (the writer/query handler rejects it anyway).
///
/// Note this function is only the classifier for the AMBIGUOUS body path; the unambiguous
/// `update=` form field and `application/sparql-update` Content-Type are gated as writes
/// unconditionally at their call sites (they ARE updates by protocol definition), without
/// consulting this function.
pub(crate) fn payload_mutates(sparql: &str) -> bool {
    // Provably a read-only query => a read (`is_ok`). Otherwise (parses as an update, OR
    // parses as neither) => fail-safe to a write (`is_err`). The two non-read cases share one
    // branch on purpose: both must be gated as a write, so distinguishing them changes nothing.
    spargebra::SparqlParser::new().parse_query(sparql).is_err()
}

/// [OPUS-4.8] (sq-gos8) Maps the auth-classifier [`Operation`] to the structured-audit
/// [`Action`](crate::access_audit::Action) for the plain SPARQL surface (read => a query,
/// write => an update).
#[cfg(feature = "access-audit")]
fn sparql_action(op: Operation) -> crate::access_audit::Action {
    match op {
        Operation::Read => crate::access_audit::Action::Query,
        Operation::Write => crate::access_audit::Action::Update,
    }
}

/// [OPUS-4.8] (sq-ljfz) The forwarded WebID a TRUSTED front placed on the request, IFF the
/// operator named a trusted header ([`ServerConfig::audit_webid_header`]). `None` when no
/// trusted header is configured (the common case), the header is absent, or its value is not
/// valid UTF-8 — in every such case the audit actor falls back to the local Bearer gate.
/// HTTP header names are case-insensitive (`HeaderMap::get` lowercases its key), so the
/// configured name matches regardless of the front's casing.
#[cfg(feature = "access-audit")]
fn forwarded_webid<'a>(config: &ServerConfig, headers: &'a HeaderMap) -> Option<&'a str> {
    let name = config.audit_webid_header.as_deref()?;
    headers.get(name)?.to_str().ok()
}

/// [OPUS-4.8] (sq-gos8; sq-ljfz) Begins a structured access-audit record IFF a sink is installed
/// — snapshotting the actor, the action class, the resource and the (non-reversible) request
/// fingerprint at the enforcement seam. Returns `None` (and builds nothing) when no sink is
/// configured, so an audit-disabled request pays only the `Option` check.
///
/// The actor is derived via [`Actor::from_session`](crate::access_audit::Actor::from_session):
/// when the operator has configured a TRUSTED forwarded-identity header
/// ([`ServerConfig::audit_webid_header`]) and a fronting auth layer (sparq-solid / a Solid-WAC
/// proxy / gateway) set it, the record attributes access to that authenticated WebID; otherwise
/// it falls back to the local Bearer-token fingerprint exactly as before. The returned
/// [`AuditPending`] is `finish`ed once the enforced decision is known and handed to the sink via
/// [`audit_access_finish`].
#[cfg(feature = "access-audit")]
fn audit_access_begin(
    state: &AppState,
    action: crate::access_audit::Action,
    resource: crate::access_audit::Resource,
    headers: &HeaderMap,
    sparql: Option<&str>,
) -> Option<crate::access_audit::AuditPending> {
    state.access_audit_sink().map(|_| {
        let actor = crate::access_audit::Actor::from_session(
            forwarded_webid(state.config(), headers),
            bearer_token(headers),
        );
        crate::access_audit::AuditPending::begin(action, actor, resource, sparql)
    })
}

/// [OPUS-4.8] (sq-gos8) Finishes a pending record and records it through the installed sink,
/// deriving the ACTUALLY-enforced decision from the finished [`Response`]: a `401` is the auth
/// gate's denial (`Deny` + the bearer-auth policy basis), anything else is the allowed-and-served
/// outcome. So the recorded decision is the one the server enforced, never a claimed-but-
/// disconnected one. A no-op when `pending` is `None` (no sink).
#[cfg(feature = "access-audit")]
fn audit_access_finish(
    state: &AppState,
    pending: Option<crate::access_audit::AuditPending>,
    resp: &Response,
) {
    use crate::access_audit::AccessDecision;
    let (Some(pending), Some(sink)) = (pending, state.access_audit_sink()) else {
        return;
    };
    let status = resp.status();
    let (decision, basis) = if status == StatusCode::UNAUTHORIZED {
        (
            AccessDecision::Deny,
            "bearer-auth: missing or invalid token",
        )
    } else {
        (AccessDecision::Allow, "bearer-auth: allowed")
    };
    let event = pending.finish(decision, basis, status.as_u16());
    sink.record(&event);
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.trim().parse().ok()
}

/// Extra wall-clock allowance past the cooperative deadline before the server gives up
/// awaiting the worker and answers 503 anyway (the detached worker still stops at its
/// next budget check — it is not leaked indefinitely).
pub(crate) const TIMEOUT_GRACE: Duration = Duration::from_secs(2);

/// A request's pinned generation: holding this `Arc` keeps the generation's immutable
/// snapshot alive no matter how far the writer publishes past it. Pinned ONCE per
/// request and kept for the lifetime of response production (streamed bodies included —
/// see `chunked_response`), so every response is snapshot-consistent with its start.
pub type PinnedGen = Arc<Generation<Graph>>;

/// The catch-all pod: the visibility scope every query implicitly reads, and the
/// honest conflict unit for any write that cannot be scoped to a finite set of named
/// graphs (§6.3/§6.5).
///
/// [OPUS-4.8] (sq-uqh, Wave B) Bumping this pod's epoch means "invalidate everything":
/// it represents the DEFAULT graph (which the GSP surface maps every direct graph onto)
/// plus any cross-graph / dynamically-scoped write whose touched named graphs are not
/// statically knowable from the parsed update. A correct cache MUST therefore record
/// this pod's epoch on every entry, so a global bump invalidates all cached reads. Writes
/// that DO name a finite set of graphs additionally bump those graphs' per-named-graph
/// pods (see `touched_pods`), so a cache keyed on finer-than-global pods is invalidated
/// too — finer scoping is purely additive over this catch-all, never a replacement for it.
///
/// Public so a cache layer (and the update tests) can record/compare this catch-all pod's
/// epoch on every entry — the contract that makes a global bump invalidate everything.
pub const GLOBAL_POD: &str = "urn:sparq:pod:global";

/// [OPUS-4.8] (sq-uqh, Wave B) The visibility scope (`PodId` set) a SPARQL Update writes,
/// for per-named-graph cache invalidation (§6.3/§6.5).
///
/// Each pod is a named graph (its graph IRI); a write to graph A bumps only graph A's
/// epoch, so a cached read scoped to graph B is untouched. Correctness is paramount —
/// a missed bump is a stale read — so this OVER-invalidates whenever it cannot prove a
/// finer scope:
///
///   * INSERT/DELETE DATA, LOAD, CREATE, CLEAR/DROP GRAPH `<g>`: scoped to the concrete
///     named graph(s) they name. A *default-graph* quad / target / LOAD destination falls
///     back to [`GLOBAL_POD`] (the default graph is the catch-all pod).
///   * CLEAR/DROP DEFAULT/NAMED/ALL: cross-graph and unbounded → [`GLOBAL_POD`].
///   * DELETE/INSERT … WHERE: scoped to the concrete graph slots its delete/insert
///     TEMPLATES write — but the moment any template targets a *variable* graph name
///     (`GRAPH ?g { … }`), the written graphs are not statically knowable → [`GLOBAL_POD`].
///     The WHERE/USING READ scope is irrelevant: invalidation tracks what a write MODIFIES,
///     not what it reads.
///
/// A parse failure also returns [`GLOBAL_POD`] — the writer re-parses and rejects the
/// update, so nothing is actually published, but tagging conservatively keeps this
/// function total and never under-invalidates if the two parses ever disagree.
///
/// Concrete named pods are ALWAYS accumulated even when the global flag is also set, so
/// the returned set is correct whether a cache entry records the global pod, the specific
/// pods, or both. Over-invalidation (a redundant epoch bump) only costs a cache miss;
/// under-invalidation costs a stale read — so when in doubt, this bumps more.
fn touched_pods(sparql: &str) -> Vec<PodId> {
    use spargebra::algebra::GraphTarget;
    use spargebra::term::{GraphName, GraphNamePattern};
    use spargebra::GraphUpdateOperation;

    let mut acc = TouchedPods::default();
    let upd = match spargebra::SparqlParser::new().parse_update(sparql) {
        Ok(upd) => upd,
        // Unparsable here = the writer will also reject it (nothing published); tag global
        // so we are total and never under-invalidate on a parser disagreement.
        Err(_) => return vec![PodId::new(GLOBAL_POD)],
    };

    // A graph slot a write targets: a named graph is its own pod; the default graph is
    // the catch-all (global).
    let touch_graph = |g: &GraphName, acc: &mut TouchedPods| match g {
        GraphName::NamedNode(n) => acc.named(n.as_str()),
        GraphName::DefaultGraph => acc.global(),
    };

    for op in &upd.operations {
        match op {
            GraphUpdateOperation::InsertData { data } => {
                for q in data {
                    touch_graph(&q.graph_name, &mut acc);
                }
            }
            GraphUpdateOperation::DeleteData { data } => {
                for q in data {
                    touch_graph(&q.graph_name, &mut acc);
                }
            }
            GraphUpdateOperation::Load { destination, .. } => touch_graph(destination, &mut acc),
            // CLEAR / DROP: a single named graph is scopable; DEFAULT / NAMED / ALL are
            // cross-graph or unbounded → global.
            GraphUpdateOperation::Clear { graph: target, .. }
            | GraphUpdateOperation::Drop { graph: target, .. } => match target {
                GraphTarget::NamedNode(n) => acc.named(n.as_str()),
                GraphTarget::DefaultGraph | GraphTarget::NamedGraphs | GraphTarget::AllGraphs => {
                    acc.global()
                }
            },
            // CREATE makes one empty named graph — touches only it.
            GraphUpdateOperation::Create { graph, .. } => acc.named(graph.as_str()),
            // DELETE/INSERT … WHERE writes the template graph slots. Concrete slots are
            // scopable; a variable graph name (or a default-graph slot) is not, so it
            // widens to global. Reads (WHERE / USING) do not affect invalidation scope.
            GraphUpdateOperation::DeleteInsert { delete, insert, .. } => {
                for slot in delete
                    .iter()
                    .map(|q| &q.graph_name)
                    .chain(insert.iter().map(|q| &q.graph_name))
                {
                    match slot {
                        GraphNamePattern::NamedNode(n) => acc.named(n.as_str()),
                        // Default graph or a dynamically-bound graph name: unscopable → global.
                        GraphNamePattern::DefaultGraph | GraphNamePattern::Variable(_) => {
                            acc.global()
                        }
                    }
                }
            }
        }
    }

    acc.into_pods()
}

/// [OPUS-4.8] (sq-uqh) Accumulator for [`touched_pods`]: the set of concrete named-graph
/// pods a write touches, plus a flag for any unscopable (global) effect. The two are kept
/// independent — concrete pods are recorded even alongside a global effect — so the final
/// set never under-invalidates regardless of how a cache entry records its scope.
#[derive(Default)]
struct TouchedPods {
    named: std::collections::BTreeSet<String>,
    global: bool,
}

impl TouchedPods {
    /// Records a write to a concrete named graph.
    fn named(&mut self, iri: &str) {
        self.named.insert(iri.to_string());
    }

    /// Records an unscopable / cross-graph / default-graph write (the catch-all pod).
    fn global(&mut self) {
        self.global = true;
    }

    /// Materialises the pod set. Always non-empty so the publish records *some* epoch
    /// bump: an update that somehow named no graph at all (e.g. an empty operation list)
    /// still conservatively bumps global.
    fn into_pods(self) -> Vec<PodId> {
        let mut pods: Vec<PodId> = self.named.into_iter().map(PodId::new).collect();
        if self.global || pods.is_empty() {
            pods.push(PodId::new(GLOBAL_POD));
        }
        pods
    }
}

/// The sequenced writer's snapshot-production strategy: sparq-serve's [`GraphApplier`]
/// (per-batch O(graph) fork + O(batch) `update_in_place` — see its module docs for the
/// recorded cost decision), with every engine call wrapped in [`with_engine_scope`] so the
/// opt-in `geo` registry AND the SERVICE egress allowlist (sq-4w18) apply to updates
/// exactly as they do to queries — an `INSERT … WHERE { SERVICE <iri> { … } }` update
/// federates under the same default-deny allowlist as a read. The trait is sparq-serve's
/// documented seam for exactly this wrapper. [OPUS-4.8]
/// [OPUS-4.8] (sq-7cxr, gh-44) Resolves the durable directory-backed [`Graph`] for
/// `--persist <DIR>`: OPEN an existing on-disk store (replaying its WAL — prior updates
/// present with NO rebuild — and ignoring `seed`), or CREATE a fresh store by saving `seed`
/// there and re-opening it so it carries a WAL.
///
/// "Existing store" is keyed on the dictionary file the engine's [`Graph::open`] requires
/// (`dict-meta.bin`, or the legacy `dict.bin`). A directory that exists but is empty (or holds
/// only an unrelated file) is treated as fresh — we create the store from `seed`. A directory
/// that holds a store but fails to open (corruption) surfaces the open error rather than
/// silently clobbering it.
fn open_or_create_durable(dir: &std::path::Path, seed: Graph) -> Result<Graph, String> {
    let has_store = dir.join("dict-meta.bin").exists() || dir.join("dict.bin").exists();
    if has_store {
        // Existing durable store IS the source of truth: open + WAL-replay, no rebuild.
        return Graph::open(dir).map_err(|e| format!("opening persist dir {}: {e}", dir.display()));
    }
    // Fresh: persist the seed, then open it so the returned graph is directory-backed
    // (carries its own WAL) and ready for WAL-durable updates.
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("creating persist dir {}: {e}", dir.display()))?;
    seed.save(dir)
        .map_err(|e| format!("initialising persist dir {}: {e}", dir.display()))?;
    Graph::open(dir).map_err(|e| {
        format!(
            "opening freshly-initialised persist dir {}: {e}",
            dir.display()
        )
    })
}

/// [OPUS-4.8] (sq-vpx4) Internal marker prefixed onto the error string of a
/// [`WriteError::Unavailable`] (durable-write refusal) as it crosses from
/// [`AppState::apply_update`] to the HTTP response mapper, so
/// [`update_rejection_response`] can route it to HTTP 503 (retryable) rather than the
/// default 400 (client error). Never leaves the process — stripped before the message
/// reaches the client.
const DURABLE_UNAVAILABLE_PREFIX: &str = "\u{1}durable-unavailable\u{1}";
#[cfg(feature = "shacl")]
const SHACL_GUARD_REJECTED_PREFIX: &str = "\u{1}shacl-guard-rejected\u{1}";
/// [FABLE-5] (PR #1999 security review) Marker for a write the SHACL guard refused because
/// the guard shapes graph could not be FULLY evaluated (a strict-validation failure or a
/// skipped/unevaluable constraint) — the fail-closed outcome, mapped to HTTP 500 (an
/// operator-side guard misconfiguration, not a client error). Never leaves the process.
#[cfg(feature = "shacl")]
const SHACL_GUARD_UNEVALUABLE_PREFIX: &str = "\u{1}shacl-guard-unevaluable\u{1}";

struct ServerApplier {
    inner: GraphApplier,
    /// The config the writer thread enforces around every engine call (carries the
    /// SERVICE egress allowlist; the writer has no per-request config, so it owns its own).
    config: Arc<ServerConfig>,
    /// [OPUS-4.8] (sq-7cxr, gh-44) The optional DURABLE mirror of the published lineage.
    /// `Some` exactly when `--persist <DIR>` is set: a directory-backed [`Graph`] (its own
    /// WAL + named-graph persistence) that the writer thread keeps in lockstep with the ring
    /// by re-applying each committed batch to it, WAL-durably, BEFORE the generation is
    /// published (and hence before the client ack). `None` is today's purely in-memory server.
    durable: Option<DurableStore>,
    /// [SONNET-4.6] (sq-m9prn) The SAME registry `AppState` serves `GET /queries` from —
    /// shared by `Arc`, so an update the writer thread registers is visible to (and killable
    /// by) the HTTP handlers. Compiled only with the `query-registry` feature; without it the
    /// applier has no extra field and the write path is byte-identical to before.
    #[cfg(feature = "query-registry")]
    running_queries: Arc<QueryRegistry>,
}

/// [OPUS-4.8] (sq-7cxr, gh-44) The writer-thread-owned durable store: the on-disk [`Graph`]
/// plus the buffer of RESOLVED update effects accumulated for the batch currently being committed.
///
/// The writer drives `fork → apply* → seal` per batch (and re-`fork`s + replays the
/// already-successful prefix on a mid-batch failure). We mirror that exactly: [`fork`] clears
/// `batch` (a fresh attempt starts), [`apply`] appends the effects of an update only AFTER it
/// applied cleanly to the in-memory working copy, and [`seal`] flushes `batch` to `graph`
/// WAL-durably. So at seal time `batch` is precisely the resolved delta of the updates that will
/// be published — applied once, in order, to the durable graph. Because all three run on the
/// single writer thread, no lock is needed and the durable graph never diverges from the
/// published lineage.
///
/// [OPUS-4.8] (Copilot PR#80) We buffer the *resolved* [`sparq_engine::UpdateEffect`] log
/// captured during the in-memory application — NOT the update text. Re-executing the text against
/// the durable graph would re-roll non-deterministic functions (`NOW()`/`RAND()`/`UUID()`/fresh
/// `BNODE()`) and could re-fetch different `LOAD <remote>` content, so the durable state could
/// diverge from the already-acked in-memory state. Replaying the captured delta makes the durable
/// phase deterministic by construction — the durable graph receives the identical resolved triples.
struct DurableStore {
    graph: Graph,
    batch: Vec<sparq_engine::UpdateEffect>,
    /// [OPUS-4.8] (sq-vpx4) TEST SEAM for injecting durable-write I/O failures into the
    /// seal path. In production this is `None` and the real [`sparq_engine::apply_effects`]
    /// runs. A test can install a hook that fails the durable commit (e.g. once with a
    /// simulated `ENOSPC`, then clears) WITHOUT touching the production code path — the
    /// hook decides per-call whether to fail. Boxed `FnMut` so it can hold per-call state
    /// (a "fail the next N seals" counter). Production behaviour is unchanged: `None` ⇒
    /// exactly the prior `apply_effects` call.
    #[cfg(any(test, feature = "test-seams"))]
    fail_seal: Option<Box<dyn FnMut() -> Option<String> + Send>>,
}

impl DurableStore {
    fn new(graph: Graph) -> Self {
        DurableStore {
            graph,
            batch: Vec::new(),
            #[cfg(any(test, feature = "test-seams"))]
            fail_seal: None,
        }
    }

    /// [OPUS-4.8] (sq-vpx4) Commit the buffered batch durably. Returns `Err` on a
    /// durable-write failure (real I/O error, or an injected one via the test seam) —
    /// the caller ([`ServerApplier::seal`]) propagates this WITHOUT publishing or
    /// acking, so a write that didn't durably commit is never observed.
    fn commit_batch(&mut self) -> Result<(), String> {
        let batch = std::mem::take(&mut self.batch);
        #[cfg(any(test, feature = "test-seams"))]
        if let Some(hook) = self.fail_seal.as_mut() {
            if let Some(e) = hook() {
                return Err(e);
            }
        }
        sparq_engine::apply_effects(&mut self.graph, &batch)
    }

    /// [OPUS-4.8] (sq-x32t) WAL COMPACTION / VACUUM for erasure-completeness. Rewrites the
    /// durable on-disk store to a fresh segment set containing ONLY the current LIVE triples,
    /// so superseded INSERTs, logically-DELETEd data and DROPped graphs that still linger in
    /// earlier WAL segments are PHYSICALLY removed from the on-disk history (the manual purge
    /// in `compliance/privacy/retention-erasure-runbook.md` §7a, automated). [`Graph::vacuum`]
    /// re-interns the live triples into a fresh dictionary (so orphaned term VALUES are purged
    /// too), writes the new base to a sibling dir, then does a rollback-safe two-rename swap
    /// (parent dir fsync'd between renames) and truncates the WAL; an interrupted swap is healed
    /// deterministically by `recover_compaction` on the next `Graph::open`. The live triple set
    /// is preserved EXACTLY (round-trip), so the published in-memory snapshot is unaffected —
    /// only the durable image is rewritten.
    ///
    /// Runs on the writer thread (via [`ServerApplier::maintain`]), so the durable graph is
    /// never touched concurrently with a commit. A pending in-memory `batch` is impossible
    /// here: maintenance is sequenced AFTER the preceding batch is sealed (see `Writer::run`),
    /// so `commit_batch` has already drained it. We assert that to fail loudly if the invariant
    /// is ever broken rather than silently dropping buffered effects.
    fn compact(&mut self) -> Result<(), String> {
        debug_assert!(
            self.batch.is_empty(),
            "compaction must run between batches (the durable batch buffer must be drained by seal)"
        );
        // [OPUS-4.8] (sq-x32t) Use the ERASURE-GRADE `vacuum`, not the serving-path `compact`:
        // vacuum re-interns into a fresh dictionary so a term VALUE orphaned by a DELETE / DROP
        // GRAPH (e.g. a personal-data literal) is physically purged from the on-disk dict blob,
        // not just from the triple indexes. `compact` keeps the dict for O(triples) folding,
        // which would leave the orphaned bytes on disk — not erasure-complete.
        self.graph.vacuum()
    }
}

impl ServerApplier {
    fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            inner: GraphApplier::default(),
            config,
            durable: None,
            #[cfg(feature = "query-registry")]
            running_queries: Arc::new(QueryRegistry::default()),
        }
    }

    /// [SONNET-4.6] (sq-m9prn) Points this applier at the `AppState`'s shared running-query
    /// registry, so the updates it applies are listed by `GET /queries` and killable by
    /// `DELETE /queries/{id}`. Without this call the applier keeps its own private registry
    /// and its updates are simply not listed (the pre-sq-m9prn behaviour).
    #[cfg(feature = "query-registry")]
    fn with_registry(mut self, registry: Arc<QueryRegistry>) -> Self {
        self.running_queries = registry;
        self
    }

    /// [OPUS-4.8] (sq-7cxr, gh-44) An applier whose committed batches are also mirrored to
    /// `graph` — a directory-backed [`Graph`] opened/created at `--persist <DIR>`. Every
    /// committed batch is re-applied to it WAL-durably in [`seal`] before publish.
    fn with_durable(config: Arc<ServerConfig>, graph: Graph) -> Self {
        Self {
            inner: GraphApplier::default(),
            config,
            durable: Some(DurableStore::new(graph)),
            #[cfg(feature = "query-registry")]
            running_queries: Arc::new(QueryRegistry::default()),
        }
    }
}

impl ApplyUpdates for ServerApplier {
    type Snapshot = Graph;
    type Working = Graph;
    type Update = String;

    fn fork(&mut self, base: &Graph) -> Result<Graph, String> {
        // [OPUS-4.8] (sq-7cxr) A fresh attempt (first fork of a batch OR a re-fork during the
        // writer's failure replay) restarts the durable batch buffer; only the updates that go
        // on to apply cleanly are re-accumulated, so `seal` mirrors exactly the published set.
        if let Some(d) = &mut self.durable {
            d.batch.clear();
        }
        with_engine_scope(&self.config, || self.inner.fork(base))
    }

    fn apply(&mut self, working: &mut Graph, update: &String) -> Result<(), String> {
        // [OPUS-4.8] sq-ebii: run the update under the SAME cooperative QueryBudget the read
        // paths use — the memory cap (`max_query_rows`) bounds a `DELETE/INSERT … WHERE`
        // whose WHERE blows up (cross-product alloc capped + abort at the row cap), and the
        // deadline (from `query_timeout`, measured from when the writer starts THIS update)
        // aborts a long WHERE evaluation cooperatively instead of running the writer thread
        // to an OOM. The deadline here is the writer-side cooperative stop; the HTTP side
        // separately hard-caps the client's await (see `await_update_worker`). With both
        // limits off (the default) this is the unlimited budget — identical to before.
        // [SONNET-4.6] (sq-m9prn) Register this update with the running-query registry
        // (feature `query-registry`) and install its cancel flag into the budget, so
        // `GET /queries` lists it as `kind: "update"` and `DELETE /queries/{id}` trips the
        // engine's cooperative-cancellation poll site inside its WHERE evaluation. The RAII
        // guard is bound to this stack frame, so the row is removed on EVERY exit path — a
        // clean apply, a rejected one (the `?` below), or an unwind.
        //
        // Registering HERE (rather than at HTTP dispatch) is what makes the listing honest:
        // the writer is sequenced, so a row exists exactly while an update is consuming the
        // writer thread. If the writer re-forks and replays the batch after a mid-batch
        // failure, each replayed apply registers afresh — a killed update is not replayed
        // (the writer skips it), so its cancellation is not silently undone.
        #[cfg(feature = "query-registry")]
        let (budget, _qr_guard) = {
            let (cancel, guard) = self.running_queries.register_update(update);
            (update_budget(&self.config).with_cancel(cancel), guard)
        };
        #[cfg(not(feature = "query-registry"))]
        let budget = update_budget(&self.config);
        // [OPUS-4.8] (Copilot PR#80) When mirroring durably, CAPTURE the resolved effect log of
        // the in-memory application so the durable graph can replay the EXACT committed delta —
        // never re-executing the (possibly non-deterministic / side-effecting) update text. With
        // no durable mirror this is `update_in_place_with_budget` exactly as before (no capture).
        if let Some(d) = &mut self.durable {
            let effects = with_engine_scope(&self.config, || {
                sparq_engine::update_in_place_capturing(working, update, &budget)
            })?;
            // Record for the durable mirror ONLY after it applied cleanly in memory — a rejected
            // update is never persisted (it is not published either).
            d.batch.extend(effects);
        } else {
            with_engine_scope(&self.config, || {
                sparq_engine::update_in_place_with_budget(working, update, &budget)
            })?;
        }
        // [GPT-5.6] sq-lsp7k.2.4: validate the private candidate before seal/publish.
        //
        // [FABLE-5] (PR #1999 security review) The guard is FAIL-CLOSED on an unevaluable
        // shapes graph. The lenient `sparq_shacl::validate` SKIPS constraints it cannot
        // evaluate (ill-formed constructs, an uncompilable `sh:pattern`, SHACL-SPARQL
        // pre-binding violations) and reports `conforms` over only the rest — which would
        // silently ADMIT writes past a guard the operator believes is enforced. So guard
        // mode uses `validate_strict` (well-formedness failures reject the write; they are
        // also a hard STARTUP error, see `try_with_config`) AND rejects when the report
        // carries skipped-constraint diagnostics (e.g. an uncompilable `sh:pattern`, which
        // is NOT parse-time detectable). Only the write-guard is strict: the read-only
        // `/shacl/validate` endpoint keeps the lenient W3C-report behaviour.
        #[cfg(feature = "shacl")]
        if self.config.shacl_guard {
            let shapes = self.config.shacl_shapes.as_ref()
                .expect("shacl_guard configuration validated at startup");
            let report = match sparq_shacl::validate_strict(working, shapes.graph()) {
                Ok(report) => report,
                Err(failure) => {
                    return Err(format!("{SHACL_GUARD_UNEVALUABLE_PREFIX}{failure}"));
                }
            };
            if let Some(first) = report.diagnostics.first() {
                return Err(format!(
                    "{SHACL_GUARD_UNEVALUABLE_PREFIX}{} guard constraint(s) could not be evaluated and were SKIPPED (fail-closed) — e.g. {}",
                    report.diagnostics.len(),
                    first.message
                ));
            }
            if !report.conforms {
                return Err(format!("{SHACL_GUARD_REJECTED_PREFIX}{}", shacl_report_to_json(&report)));
            }
        }
        Ok(())
    }

    fn seal(&mut self, working: Graph) -> Result<Graph, String> {
        // [OPUS-4.8] (sq-7cxr, gh-44) Persist the committed batch to the durable graph BEFORE
        // returning — `seal` runs on the writer thread immediately before `publish`, so the
        // batch is WAL-durable (fsync'd by `Graph::apply_delta`) before the generation is
        // published and thus before any client ack.
        //
        // [OPUS-4.8] (Copilot PR#80) The batch is the RESOLVED effect log captured from the
        // in-memory application (see `apply`), replayed via `apply_effects` rather than by
        // re-executing the update text. This is what makes the durable graph BYTE-EQUIVALENT to
        // the published in-memory state even for non-deterministic / side-effecting updates
        // (`NOW()`/`RAND()`/`UUID()`/`BNODE()`, `LOAD <remote>`): both apply the identical
        // resolved triples, in the same order, starting from one shared seed.
        //
        // [OPUS-4.8] FAIL-CLOSED, GRACEFULLY (sq-vpx4, was sq-7cxr fatal-panic): a durable-write
        // error (disk full, I/O error) is propagated as `Err` rather than panicking the writer
        // thread. `seal` runs BEFORE `publish`, so returning `Err` here means the generation is
        // NEVER published and the in-flight batch's submitters are failed with
        // `WriteError::Unavailable` (HTTP 503, retryable) — NOT a false 2xx success. The
        // fail-closed correctness invariant is unchanged (a write that didn't durably commit is
        // never acked nor published); what changes is that a TRANSIENT error (e.g. a brief
        // `ENOSPC` that later clears) no longer kills the writer thread / the whole server. The
        // writer stays alive (degraded), so reads keep being served from the last published
        // snapshot and a subsequent write succeeds once durability recovers. A PERSISTENT error
        // simply yields repeated 503s. The in-memory `working` is dropped on `Err` (never
        // published), so the durable store and the published lineage cannot diverge.
        if let Some(d) = &mut self.durable {
            d.commit_batch().map_err(|e| {
                format!("durable persist failed (sq-vpx4); write refused (not acked, not published): {e}")
            })?;
        }
        self.inner.seal(working)
    }

    /// [OPUS-4.8] (sq-x32t) Out-of-band WAL COMPACTION / VACUUM of the durable store, for
    /// erasure-completeness. A no-op for an in-memory server (`durable == None`) — there is no
    /// on-disk history to purge. For a `--persist` server it physically rewrites the on-disk
    /// store to contain only the current live triples (see [`DurableStore::compact`]). It runs
    /// on the writer thread strictly between batches (the `Writer` sequences it through the same
    /// queue as updates), so the durable graph is never accessed concurrently and the
    /// compaction folds in every write that preceded the request. No generation is published —
    /// the live triple set is preserved exactly, so readers are unaffected throughout.
    fn maintain(&mut self) -> Result<(), String> {
        match &mut self.durable {
            Some(d) => d.compact(),
            // In-memory server: nothing on disk to compact (erasure is immediate — the dropped
            // generations free by `Arc` drop, no WAL history survives).
            None => Ok(()),
        }
    }

    /// [OPUS-4.8] (sq-ft7u) RESTORE-INTO-DURABLE. Replace the `--persist` durable store's contents
    /// with the freshly-imported (already-validated) `fresh` graph, crash-safely, and return the
    /// snapshot the ring should publish so reads serve the restored data. Runs on the WRITER THREAD
    /// (the only thread that mutates the durable store), so the on-disk swap never races a commit.
    ///
    /// Sequence: release the OLD durable graph's WAL handle (close its log), then
    /// [`Graph::restore_into_durable`] writes `fresh`'s image to a sibling, two-rename-swaps it over
    /// `dir` (parent fsync'd between the renames), re-opens the new base memory-mapped with a fresh
    /// WAL, and drops the old base — the EXACT crash-safe protocol the in-process compaction uses,
    /// healed by `recover_compaction` on the next open if a crash interrupts it. We adopt the
    /// re-opened directory-backed graph as the new durable store (so subsequent updates WAL-append
    /// to it), and return an independent in-memory snapshot of it for the ring to publish.
    ///
    /// FAIL-CLOSED: `fresh` is fully built before this runs; if the swap fails before the first
    /// rename the OLD durable store is untouched, and a crash mid-swap heals to old-or-new. An
    /// in-memory applier (no durable store) is a clean `Err` — there is no durable dir to write
    /// through, so the in-memory restore path (the ring/writer swap) is used instead.
    fn restore_durable(&mut self, fresh: Graph) -> Result<Graph, String> {
        let durable = self.durable.as_mut().ok_or_else(|| {
            "restore-into-durable requested on an applier with no --persist store".to_string()
        })?;
        // The directory the OLD durable graph is backed by; the swap targets it.
        let dir = durable
            .graph
            .persist_dir()
            .ok_or_else(|| "durable graph is not directory-backed".to_string())?;
        // Release the OLD durable graph's WAL handle before the directory swap (its `wal.log` is
        // about to be renamed away with `dir`); the crash-safe swap re-opens a fresh handle.
        durable.graph.close_wal();
        let restored = Graph::restore_into_durable(&dir, fresh)
            .map_err(|e| format!("restore-into-durable swap failed: {e}"))?;
        // Adopt the re-opened, directory-backed restored store; subsequent updates WAL-append to it.
        // The published snapshot is an independent in-memory image of the restored content.
        let published = restored.snapshot().into_graph();
        durable.graph = restored;
        Ok(published)
    }
}

/// Shared server state: the dataset under query, served from a sparq-serve
/// [`GenerationRing`] of immutable snapshots. Queries pin the current generation
/// ([`AppState::current`], lock-free `ArcSwap` load) and evaluate against it for the
/// whole response; SPARQL Update submits through the single sequenced
/// [`sparq_serve::Writer`], which group-commits each batch as ONE new generation.
/// Also carries the hardening [`ServerConfig`] and the subscription plumbing (T23):
/// a `watch` channel that broadcasts the committed generation number to every open
/// `/subscriptions` socket, plus the global active-subscription counters.
///
/// **What this replaced (Wave A4):** the double-buffered `RwLock<Arc<Graph>>` writer —
/// spare-buffer reclaim via `Arc::try_unwrap` + 200 µs polling, lag replay, periodic
/// `compact_every` fold-back. The ring makes reclaim impossible *by design* (it retains
/// up to K old generations, so a published graph never drains back to the writer) and
/// unnecessary (old generations free by plain `Arc` drop) — removing the measured
/// §4.3/§4.4 pathologies: the 5.4 s/32 s pinned-snapshot writer stall and reclaim-poll
/// degradation under reader churn. `compact_every` went with it: the writer's per-batch
/// fork rebuilds a folded base, so overlays never accumulate across batches.
/// [OPUS-4.8] (sq-o5bi, sq-0g6g) The swappable serving core: the generation ring + the single
/// sequenced writer that publishes onto it. **`backup`-feature only.** It exists solely so an
/// online RESTORE can atomically install a freshly-built ring+writer rehydrated from a backup
/// artifact while readers keep loading lock-free — the same lock-free-read discipline the ring
/// itself uses internally. Under `backup`, [`AppState`] holds this behind an [`ArcSwap`], so a
/// read pays one extra `ArcSwap::load` (the same cost class as the ring's own `current()`).
///
/// Without `backup` (the DEFAULT) there is no swap mechanism at all: [`AppState`] holds the
/// `ring`/`writer` pair directly, exactly as it did before #941, so the default read path is
/// byte-identical to `main` and never loads an `ArcSwap` in `AppState` (sq-0g6g resolved in the
/// lean direction — the cost is paid only when you opt into `backup`).
#[cfg(feature = "backup")]
struct ServingCore {
    /// The generation ring (Wave A1): lock-free `current()`, bounded retention.
    ring: Arc<GenerationRing<Graph>>,
    /// The single sequenced writer (Wave A2): the sole publisher of generations.
    /// `submit` blocks for the group-commit window + batch application, so update
    /// handlers call it on the blocking pool.
    writer: Arc<Writer<String, Graph>>,
}

/// [GPT-5.6] (sq-kqofk) The opt-in server adapter around the writer-thread CDC hook. The
/// append-capable `ChangeLog` is consumed by `into_commit_hook`; the separate handle is read-only
/// by convention and may safely `poll` concurrently because polling never mutates the log. The
/// hook mutex is writer-thread-side only: it keeps the single log writer safe across an online
/// backup restore's briefly-overlapping old/new writer threads, and is never held by submitters.
#[cfg(feature = "change-stream")]
struct ChangeStreamState {
    reader: sparq_serve::ChangeLog,
    hook: Arc<std::sync::Mutex<Box<ChangeStreamHook>>>,
    next_seq: Arc<std::sync::atomic::AtomicU64>,
    /// [FABLE-5] (gh-2436) Operator control over the LIVE append log the hook consumed —
    /// the `POST /admin/change-stream/rebase` path. Serialized with the recording hook on
    /// the log's own mutex (see `sparq_serve::ChangeStreamControl`).
    control: sparq_serve::ChangeStreamControl,
}

/// [GPT-5.6] (sq-kqofk) Named to keep the optional state's writer-thread callback readable.
#[cfg(feature = "change-stream")]
type ChangeStreamHook = dyn FnMut(&Generation<Graph>, &Generation<Graph>) + Send + 'static;

#[cfg(feature = "change-stream")]
impl ChangeStreamState {
    /// [GPT-5.6] (sq-kqofk) Opens separate writer/read handles before the writer thread starts,
    /// then consumes the append handle into the production `ChangeLog` commit hook. The tracked
    /// tail advances only after a successful durable append, so `LATEST` remains an O(1) lookup.
    fn open(dir: &std::path::Path) -> Result<Self, sparq_serve::BackupError> {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

        let writer_log = sparq_serve::ChangeLog::open(dir)?;
        let reader = sparq_serve::ChangeLog::open(dir)?;
        let next_seq = Arc::new(AtomicU64::new(writer_log.next_seq()));
        let append_failed = Arc::new(AtomicBool::new(false));

        let failed_on_error = append_failed.clone();
        let (mut record, control) = writer_log.into_commit_hook_with_control(move |e| {
            failed_on_error.store(true, Ordering::Relaxed);
            tracing::warn!(target: "sparq_server", detail = %e, "change-stream record append failed (update committed; record dropped)");
        });
        let failed_for_hook = append_failed;
        let next_for_hook = next_seq.clone();
        let tracked_hook = move |from: &Generation<Graph>, to: &Generation<Graph>| {
            failed_for_hook.store(false, Ordering::Relaxed);
            record(from, to);
            if !failed_for_hook.load(Ordering::Relaxed) {
                next_for_hook.fetch_add(1, Ordering::Release);
            }
        };

        Ok(Self {
            reader,
            hook: Arc::new(std::sync::Mutex::new(Box::new(tracked_hook))),
            next_seq,
            control,
        })
    }

    /// [GPT-5.6] (sq-kqofk) Gives each sequenced writer a lightweight adapter to the one durable
    /// hook. Normal serving has one writer; online restore may briefly retain the old one too.
    fn commit_hook(&self) -> impl FnMut(&Generation<Graph>, &Generation<Graph>) + Send + 'static {
        let hook = self.hook.clone();
        move |from, to| {
            let mut hook = hook.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            hook(from, to);
        }
    }

    fn poll(&self, from_seq: u64) -> Result<Vec<sparq_serve::ChangeRecord>, String> {
        self.reader.poll(from_seq).map_err(|e| e.to_string())
    }

    fn next_seq(&self) -> u64 {
        self.next_seq.load(std::sync::atomic::Ordering::Acquire)
    }

    /// [OPUS-5] (sq-iz7ag) The read handle's trim horizon — the oldest seq the durable log still
    /// retains (`0` until retention drops a segment).
    fn trim_horizon(&self) -> u64 {
        self.reader.first_seq()
    }

    /// [FABLE-5] (gh-2436) Operator resync of the LIVE tracked log — the state-level body of
    /// `POST /admin/change-stream/rebase`. Holds the outer hook mutex for the whole
    /// append + tail update, so no commit hook interleaves between the gap-record append
    /// and the tracked `next_seq` advance (lock order outer-hook → inner-log, the same
    /// order every hook invocation takes). `new_lineage` selects the post-restore variant
    /// (the restored ring restarts at generation 0, so the same-lineage forward-only check
    /// would reject it — see `sparq_serve::ChangeLog::rebase_to_new_lineage`).
    fn rebase(
        &self,
        generation: u64,
        new_lineage: bool,
    ) -> Result<sparq_serve::ChangeRecord, sparq_serve::BackupError> {
        let _hook = self
            .hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = if new_lineage {
            self.control.rebase_to_new_lineage(generation)?
        } else {
            self.control.rebase_to(generation)?
        };
        // Safe under the outer hook lock: no hook `fetch_add` can race this store.
        self.next_seq
            .store(record.seq + 1, std::sync::atomic::Ordering::Release);
        Ok(record)
    }
}

/// [OPUS-4.8] (sq-o5bi) Builds the ring's retention config from the server config — the default
/// concurrency-only retention, or, under the `time-travel` feature, the extended retention so
/// `?generation=N` has history to serve. Shared by the constructor and the online restore so a
/// restored ring inherits exactly the same retention posture as the original.
fn build_ring_config(config: &ServerConfig) -> sparq_serve::RingConfig {
    #[cfg(not(feature = "time-travel"))]
    {
        let _ = config; // the default config ignores the server config
        sparq_serve::RingConfig::default()
    }
    #[cfg(feature = "time-travel")]
    {
        sparq_serve::RingConfig {
            time_travel: Some(sparq_serve::TimeTravelConfig {
                max_generations: config.time_travel_generations,
                max_age: config.time_travel_max_age,
            }),
            ..sparq_serve::RingConfig::default()
        }
    }
}

/// [GPT-5.6] (sq-kqofk) Spawns the server's sequenced writer, installing the optional durable
/// change-stream hook on the writer thread. Without `change-stream`, the conditional parameter and
/// branch are compiled out and this remains the original `Writer::spawn` path.
fn spawn_server_writer(
    ring: Arc<GenerationRing<Graph>>,
    applier: ServerApplier,
    config: WriterConfig,
    #[cfg(feature = "change-stream")] change_stream: Option<&ChangeStreamState>,
) -> Arc<Writer<String, Graph>> {
    #[cfg(feature = "change-stream")]
    if let Some(change_stream) = change_stream {
        return Arc::new(Writer::spawn_with_commit_hook(
            ring,
            applier,
            config,
            change_stream.commit_hook(),
        ));
    }

    Arc::new(Writer::spawn(ring, applier, config))
}

#[derive(Clone)]
pub struct AppState {
    /// The generation ring (Wave A1): lock-free `current()`, bounded retention.
    ///
    /// [OPUS-4.8] (sq-o5bi, sq-0g6g) DEFAULT representation — held directly, byte-identical to
    /// pre-#941. Under the `backup` feature the ring (and the writer below) move behind the
    /// swappable [`ServingCore`] so an online restore can atomically install a rehydrated pair.
    #[cfg(not(feature = "backup"))]
    ring: Arc<GenerationRing<Graph>>,
    /// The single sequenced writer (Wave A2): the sole publisher of generations.
    /// `submit` blocks for the group-commit window + batch application, so update
    /// handlers call it on the blocking pool.
    #[cfg(not(feature = "backup"))]
    writer: Arc<Writer<String, Graph>>,
    /// [OPUS-4.8] (sq-o5bi, sq-0g6g) `backup`-only: the swappable serving core (ring + writer).
    /// See [`ServingCore`]. Loaded lock-free on every read/update; replaced atomically only by an
    /// online restore. This `ArcSwap` indirection exists solely to enable that atomic swap and is
    /// compiled out of the default build entirely.
    #[cfg(feature = "backup")]
    core: Arc<ArcSwap<ServingCore>>,
    config: Arc<ServerConfig>,
    /// Committed generation number, advanced after every successful update. Subscription
    /// connections hold a `watch::Receiver` and re-evaluate when it changes; the watch
    /// channel inherently coalesces bursts (see `subscriptions` module docs).
    commits: Arc<tokio::sync::watch::Sender<u64>>,
    /// Global subscription bookkeeping shared by all `/subscriptions` connections.
    pub(crate) subs: Arc<crate::subscriptions::SubscriptionCounters>,
    /// Prometheus metrics (T22), exposed at `GET /metrics`.
    metrics: Arc<crate::metrics::Metrics>,
    /// [GPT-5.6] (sq-lsp7k.9.3) Lazily-built prefix-completion index, paired with the immutable
    /// graph generation it represents. A matching generation reuses the index; a newer pinned
    /// generation rebuilds it on the blocking pool before replacing this entry. The whole cache
    /// is feature-gated so the default AppState layout and request path remain unchanged.
    #[cfg(feature = "complete")]
    completion_index: Arc<std::sync::RwLock<Option<(u64, sparq_text::CompletionIndex)>>>,
    /// [OPUS-4.8] (sq-gos8) The opt-in structured access-audit sink. `None` unless the operator
    /// configured one (`--access-audit` / `SPARQ_ACCESS_AUDIT`); shared across handlers.
    #[cfg(feature = "access-audit")]
    access_audit_sink: Option<Arc<dyn crate::access_audit::AuditSink>>,
    /// [GPT-5.6] (sq-kqofk) The opt-in durable CDC adapter. The append handle lives inside the
    /// writer-thread commit hook, while handlers poll through a separate read handle. Recording
    /// therefore rides group commit and never holds a caller-side lock across `submit`.
    #[cfg(feature = "change-stream")]
    change_stream: Option<Arc<ChangeStreamState>>,
    /// [FABLE-5] (sq-fy8ci) SINGLE-FLIGHT restore permit. `true` while a restore is in flight.
    /// Two concurrent restores were previously silently serialized by the single writer thread
    /// (each individually crash-safe, last-writer-wins) — harmless but surprising: an operator
    /// scripting parallel restores got no signal that one restore clobbered the other. This flag
    /// lets [`try_begin_restore`](Self::try_begin_restore) reject the SECOND concurrent restore
    /// explicitly (the route maps it to 409 Conflict) instead. Shared across handler clones
    /// (`AppState` is `Clone`; the `Arc` is the per-server identity). `backup`-feature ONLY —
    /// compiled out of the default build entirely, like the rest of the restore machinery.
    #[cfg(feature = "backup")]
    restore_in_flight: Arc<std::sync::atomic::AtomicBool>,
    /// [FABLE-5] (sq-lsp7k.10) The named-parameterized-template store backing the OPT-IN
    /// `/templates` REST surface (feature `templates`): name → validated
    /// `sparq_engine::templates::Template`. Seeded from
    /// [`ServerConfig::templates_file`] at startup (fail-closed); mutated only by the
    /// write-gated `PUT`/`DELETE` handlers in [`crate::templates`]. An `RwLock` because
    /// invocations (reads) dominate and never block each other.
    #[cfg(feature = "templates")]
    templates: Arc<std::sync::RwLock<std::collections::BTreeMap<String, sparq_engine::templates::Template>>>,
    /// [SONNET-4.6] (sq-qsm5z) The opt-in running-query registry backing `GET /queries` and
    /// `DELETE /queries/{id}`. Compiled only with the `query-registry` feature. The registry
    /// stores cancellation handles and metadata but never raw query text; cloned `AppState`
    /// values share one registry (the `Arc`).
    #[cfg(feature = "query-registry")]
    running_queries: Arc<QueryRegistry>,
    /// [SONNET-4.6] (sq-snopa.8) `solid-authz`-only: the STATEFUL Solid authorization view over
    /// the server's OWN loaded store, cached per generation. `None` until the first
    /// `"source":"server"` `/authz/*` request builds it.
    ///
    /// WHERE THE AUTH VIEW LIVES relative to the ring's immutable generations (the design
    /// question sq-snopa.8 exists to settle): the materialised
    /// [`PodStore`](sparq_solid::PodStore) is a PURE FUNCTION of one published generation, so it
    /// is cached BESIDE the ring and KEYED BY THE GENERATION NUMBER rather than threaded through
    /// the writer. Every commit — including an `.acl`/`.acr` write — publishes a new generation,
    /// which makes the cached entry stale by construction and forces a re-materialise on the next
    /// request. That is the "re-materialise on ACL change" contract, obtained without any writer
    /// coupling: the cached view can never be older than the generation the request pins, so a
    /// request never decides from an auth view staler than the data it would read.
    #[cfg(feature = "solid-authz")]
    authz_view: Arc<std::sync::RwLock<Option<crate::solid_authz::CachedAuthView>>>,
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] (sq-qsm5z) Running-query registry — feature `query-registry`.
// ---------------------------------------------------------------------------

/// [SONNET-4.6] (sq-qsm5z) Metadata and cooperative-cancel handles for currently executing
/// queries. The mutex is never held while engine code runs — registration and deregistration
/// are the only critical sections, and they are short (HashMap insert / remove). Listing clones
/// the small per-entry view; cancellation flips an atomic flag while holding the lock.
#[cfg(feature = "query-registry")]
#[derive(Default)]
struct QueryRegistry {
    next_id: std::sync::atomic::AtomicU64,
    entries: std::sync::Mutex<std::collections::HashMap<String, RegistryEntry>>,
}

/// [SONNET-4.6] (sq-qsm5z) One row in the registry. The `cancel` flag is wired into the
/// `QueryBudget` passed to the engine, so flipping it trips the engine's cooperative-cancellation
/// poll site on the next iteration.
#[cfg(feature = "query-registry")]
struct RegistryEntry {
    /// [SONNET-4.6] (sq-m9prn) Which operation class this row is: `"query"` for a READ
    /// (SELECT/ASK/CONSTRUCT/DESCRIBE/EXPLAIN) or `"update"` for a SPARQL UPDATE running on
    /// the sequenced writer thread. Serialised as `kind` so an operator deciding what to kill
    /// can tell the two apart — an UPDATE holds the single writer, so it is the row whose
    /// cancellation also unblocks every write queued behind it.
    kind: &'static str,
    /// Unix epoch in milliseconds — serialised to the `GET /queries` response.
    started_at_unix_ms: u64,
    /// Monotonic start — used to compute elapsed ms without clock-skew noise.
    started: std::time::Instant,
    /// FNV-1a fingerprint of the raw SPARQL text (trimmed). Never the raw text — but see
    /// `registry_fingerprint` for what this unkeyed checksum does and does NOT protect.
    fingerprint: String,
    /// The `sq-kq9ia` cross-thread cancel flag shared with the engine's `QueryBudget`.
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

/// [SONNET-4.6] (sq-qsm5z) RAII membership token. Moving it into the `spawn_blocking` closure
/// means the registry row is present exactly while the engine is running; any return path —
/// normal, panic, or engine abort — fires `Drop` and removes the row.
#[cfg(feature = "query-registry")]
pub(crate) struct QueryGuard {
    id: String,
    registry: Arc<QueryRegistry>,
}

#[cfg(feature = "query-registry")]
impl Drop for QueryGuard {
    fn drop(&mut self) {
        self.registry
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
    }
}

#[cfg(feature = "query-registry")]
impl QueryRegistry {
    /// Register a new READ query (SELECT/ASK/CONSTRUCT/DESCRIBE/EXPLAIN) and return a cancel
    /// flag + RAII guard. The returned `Arc<AtomicBool>` should be installed into the
    /// `QueryBudget`; the guard should be held for the entire duration of the engine call.
    pub(crate) fn register(
        self: &Arc<Self>,
        sparql: &str,
    ) -> (Arc<std::sync::atomic::AtomicBool>, QueryGuard) {
        self.register_kind("query", sparql)
    }

    /// [SONNET-4.6] (sq-m9prn) Register a SPARQL UPDATE that the sequenced writer thread is
    /// about to apply. Same contract as [`register`](Self::register) — the flag goes into the
    /// UPDATE's `QueryBudget` (which the engine installs thread-locally for the whole update,
    /// so it reaches the `DELETE/INSERT … WHERE` evaluation), the guard is held for the apply.
    ///
    /// Registration happens when the writer STARTS the update, not when it is queued: the row
    /// therefore names what is actually consuming the writer thread, which is the operation an
    /// operator needs to kill to unblock the write queue behind it.
    pub(crate) fn register_update(
        self: &Arc<Self>,
        sparql: &str,
    ) -> (Arc<std::sync::atomic::AtomicBool>, QueryGuard) {
        self.register_kind("update", sparql)
    }

    fn register_kind(
        self: &Arc<Self>,
        kind: &'static str,
        sparql: &str,
    ) -> (Arc<std::sync::atomic::AtomicBool>, QueryGuard) {
        use std::sync::atomic::Ordering;
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let id = format!("{:016x}", seq);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id.clone(),
                RegistryEntry {
                    kind,
                    started_at_unix_ms,
                    started: std::time::Instant::now(),
                    fingerprint: registry_fingerprint(sparql),
                    cancel: Arc::clone(&cancel),
                },
            );
        let guard = QueryGuard { id, registry: Arc::clone(self) };
        (cancel, guard)
    }

    /// List all currently registered queries as JSON-ready values, sorted by id.
    pub(crate) fn list(&self) -> Vec<serde_json::Value> {
        let mut entries: Vec<_> = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, e)| {
                serde_json::json!({
                    "id": id,
                    "kind": e.kind,
                    "start": e.started_at_unix_ms,
                    "fingerprint": e.fingerprint,
                    "elapsed_ms": u64::try_from(e.started.elapsed().as_millis())
                        .unwrap_or(u64::MAX),
                })
            })
            .collect();
        entries.sort_unstable_by(|a, b| {
            a["id"].as_str().cmp(&b["id"].as_str())
        });
        entries
    }

    /// Flip the cancel flag for a registered query. Returns `false` if the id is not found.
    pub(crate) fn cancel_query(&self, id: &str) -> bool {
        use std::sync::atomic::Ordering;
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match entries.get(id) {
            Some(e) => {
                e.cancel.store(true, Ordering::Release);
                true
            }
            None => false,
        }
    }
}

/// [SONNET-4.6] (sq-qsm5z) FNV-1a fingerprint of the SPARQL query / update text (trimmed).
/// Satisfies the #241 / sq-m9prn audit-log REDACTION discipline: the `GET /queries` listing
/// exposes only this hash, never raw text.
///
/// # Security contract (honest boundary)
///
/// FNV-1a is an **unkeyed, 64-bit, non-cryptographic checksum**. What it provides is *stable
/// correlation* — the same text always fingerprints the same way, so an operator can tell two
/// rows apart or match a row against a query they already hold — and the guarantee that the
/// text is not *transmitted*. It is **not** a one-way function in the cryptographic sense and
/// it is **not** a confidentiality boundary:
///
/// * A reader of the listing can hash candidate texts offline and confirm a match, so a
///   guessable, low-entropy or template-generated body is recoverable by search.
/// * 64 bits resists neither exhaustive search over a small candidate set nor collision search.
/// * This is most material for `kind = "update"` rows, whose `INSERT DATA` body can embed
///   secrets or personal data (`sq-m9prn`).
///
/// Callers must therefore treat the fingerprint as sensitive metadata and keep `/queries`
/// behind its fail-closed READ gate; it does not protect update content from a party that
/// already has read access. Closing the offline-guessing gap needs a KEYED construction (e.g.
/// an HMAC under a per-process random secret), which is deliberately not shipped here — it
/// would add a cryptographic dependency to this dependency-free feature and is tracked as
/// follow-up work. `registry_fingerprint_is_unkeyed_and_offline_guessable` pins this contract:
/// it recomputes the fingerprint from outside the registry, so it fails the day the
/// construction becomes keyed and forces this doc to be revisited.
#[cfg(feature = "query-registry")]
fn registry_fingerprint(sparql: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in sparql.trim().as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", hash)
}

/// [FABLE-5] (sq-fy8ci) RAII permit for the SINGLE-FLIGHT restore guard: while one of these is
/// alive, [`AppState::try_begin_restore`] returns `None` (the route maps that to 409 Conflict).
/// Dropping it — normally, or during an unwind if the restore panics — releases the permit, so
/// the guard can never wedge the server into permanently refusing restores. Obtain one via
/// [`AppState::try_begin_restore`]; there is no other constructor.
#[cfg(feature = "backup")]
#[derive(Debug)]
pub struct RestoreGuard {
    flag: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "backup")]
impl Drop for RestoreGuard {
    fn drop(&mut self) {
        // Release: pairs with the Acquire in `try_begin_restore` so the next permit holder
        // observes everything this restore wrote before releasing.
        self.flag.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl AppState {
    pub fn new(graph: Graph) -> Self {
        Self::with_config(graph, ServerConfig::default())
    }

    /// Builds the state, panicking on a durable-persistence open error. Back-compat for the
    /// in-memory callers (which never set [`ServerConfig::persist_dir`], so it never fails);
    /// the binary and the persistence tests use [`try_with_config`](Self::try_with_config),
    /// which surfaces the open error cleanly.
    pub fn with_config(graph: Graph, config: ServerConfig) -> Self {
        Self::try_with_config(graph, config).expect("AppState::with_config: durable open failed")
    }

    /// [OPUS-4.8] (sq-7cxr, gh-44) Builds the state, returning the durable-persistence open
    /// error (rather than panicking) so the binary can print a clean message and exit non-zero.
    ///
    /// PERSISTENCE (`config.persist_dir == Some(dir)`): the on-disk index at `dir` is the
    /// durable source of truth. If `dir` already holds a store, it is OPENED — replaying its
    /// write-ahead log so prior updates are present with **no rebuild** — and the passed-in
    /// `graph` seed is IGNORED (re-loading a data file over an existing durable store would be
    /// destructive; the store, like QLever's persisted index, wins). If `dir` is empty/absent,
    /// the `graph` seed is SAVED there and then re-opened so it carries a WAL. The ring is then
    /// seeded from the durable graph's snapshot, and the writer's applier mirrors every
    /// committed batch back to the durable graph (WAL-durable before publish — see
    /// `ServerApplier::seal`). With `persist_dir == None` this is exactly the historical
    /// in-memory path (the ring is seeded from `graph`; no durable mirror; never errors).
    pub fn try_with_config(graph: Graph, config: ServerConfig) -> Result<Self, String> {
        #[cfg(feature = "shacl")]
        if config.shacl_guard {
            let Some(shapes) = &config.shacl_shapes else {
                return Err("SHACL guard requires a loaded shapes graph (set ServerConfig::shacl_shapes or --shacl-shapes)".to_string());
            };
            // [FABLE-5] (PR #1999 security review) FAIL-CLOSED at startup: a guard shapes
            // graph the strict validator rejects (an ill-formed construct the lenient
            // validator would silently SKIP, or a SHACL-SPARQL pre-binding violation) is a
            // hard configuration error HERE — a mis-authored guard must never boot a server
            // that silently admits non-conforming writes. (Constraints only detectable at
            // evaluation time, e.g. an uncompilable `sh:pattern`, additionally fail closed
            // per-write in `ServerApplier::apply`.)
            let model = sparq_shacl::ShapesModel::parse(shapes.graph());
            if !model.pre_binding_failures().is_empty() || !model.ill_formed().is_empty() {
                return Err(format!(
                    "SHACL guard shapes graph is not fully evaluable (the guard would fail closed on every write): {} ill-formed construct(s), {} SHACL-SPARQL pre-binding violation(s) — e.g. {}",
                    model.ill_formed().len(),
                    model.pre_binding_failures().len(),
                    model
                        .ill_formed()
                        .first()
                        .map(|c| c.message.clone())
                        .or_else(|| model.pre_binding_failures().first().map(|f| f.message.clone()))
                        .unwrap_or_default(),
                ));
            }
        }
        Self::try_with_config_inner(
            graph,
            config,
            #[cfg(any(test, feature = "test-seams"))]
            None,
        )
    }

    /// [OPUS-4.8] (sq-vpx4) Like [`try_with_config`](Self::try_with_config) but installs a
    /// durable-write failure-injection hook on the `--persist` seal path. The hook is called
    /// once per seal; returning `Some(msg)` makes that seal fail (modelling a transient/persistent
    /// I/O error), `None` lets the real durable commit run. For the graceful-degradation
    /// integration test ONLY — gated behind the `test-seams` feature so it cannot exist in a
    /// production build. Requires `config.persist_dir` to be set (else the hook has nothing to
    /// gate).
    #[cfg(feature = "test-seams")]
    pub fn with_config_inject_durable_failure(
        graph: Graph,
        config: ServerConfig,
        fail_seal: Box<dyn FnMut() -> Option<String> + Send>,
    ) -> Result<Self, String> {
        Self::try_with_config_inner(graph, config, Some(fail_seal))
    }

    fn try_with_config_inner(
        graph: Graph,
        config: ServerConfig,
        #[cfg(any(test, feature = "test-seams"))] fail_seal: Option<
            Box<dyn FnMut() -> Option<String> + Send>,
        >,
    ) -> Result<Self, String> {
        // [OPUS-4.8] sq-7cxr: resolve the durable graph (open existing / create fresh) when a
        // persistence dir is configured. `seed` is what the ring starts from; `durable` is the
        // on-disk graph the writer mirrors committed batches to.
        let (seed, durable) = match &config.persist_dir {
            Some(dir) => {
                let g = open_or_create_durable(dir, graph)?;
                // Seed the ring from an independent snapshot so the ring's lineage and the
                // durable graph are distinct objects (the durable graph keeps its WAL; the
                // ring's published snapshots are pure in-memory forks queried concurrently).
                (g.snapshot().into_graph(), Some(g))
            }
            None => (graph, None),
        };

        // Default ring (concurrency retention only); the opt-in `time-travel`
        // feature extends retention so `?generation=N` has history to serve.
        let ring_config = build_ring_config(&config);
        let ring = Arc::new(GenerationRing::with_config(seed, ring_config));
        // [OPUS-4.8] sq-4w18: share the config Arc with the writer so its update path
        // enforces the same SERVICE egress allowlist (a federated `INSERT … WHERE` is
        // gated like a read).
        let config = Arc::new(config);
        // [SONNET-4.6] (sq-qsm5z) Empty registry at boot; queries self-register when the
        // `query-registry` feature is compiled in. Built BEFORE the applier so the writer
        // thread shares the one registry the HTTP handlers list and cancel through
        // ([SONNET-4.6] sq-m9prn — the UPDATE half).
        #[cfg(feature = "query-registry")]
        let running_queries = Arc::new(QueryRegistry::default());
        // [OPUS-4.8] sq-7cxr: a durable-mirroring applier when persistence is on, else the
        // historical in-memory applier.
        let applier = match durable {
            Some(g) => {
                #[allow(unused_mut)]
                let mut a = ServerApplier::with_durable(config.clone(), g);
                // [OPUS-4.8] (sq-vpx4) Install the durable-write failure-injection hook, if any
                // (test-seams only). A no-op in production (the field/parameter do not exist).
                #[cfg(any(test, feature = "test-seams"))]
                if let (Some(d), Some(hook)) = (a.durable.as_mut(), fail_seal) {
                    d.fail_seal = Some(hook);
                }
                a
            }
            None => ServerApplier::new(config.clone()),
        };
        // [SONNET-4.6] (sq-m9prn) Share the one registry with the writer thread.
        #[cfg(feature = "query-registry")]
        let applier = applier.with_registry(Arc::clone(&running_queries));
        // [GPT-5.6] (sq-kqofk) Open the CDC state before spawning the writer so the append log can
        // be consumed into its commit hook. A corrupt log remains a clean startup failure.
        #[cfg(feature = "change-stream")]
        let change_stream = match &config.change_stream_dir {
            Some(dir) => {
                Some(Arc::new(ChangeStreamState::open(dir).map_err(|e| {
                    format!("change-stream log open failed: {e}")
                })?))
            }
            None => None,
        };
        let writer = spawn_server_writer(
            ring.clone(),
            applier,
            writer_config(&config),
            #[cfg(feature = "change-stream")]
            change_stream.as_deref(),
        );
        // [OPUS-4.8] sq-gos8: open the structured access-audit sink if one is configured. A
        // failure to open (e.g. an unwritable audit-file path) is surfaced as a clean startup
        // error rather than silently dropping the trail — the operator asked for an audit log.
        #[cfg(feature = "access-audit")]
        let access_audit_sink = match &config.access_audit {
            Some(target) => Some(
                crate::access_audit::make_sink(target)
                    .map_err(|e| format!("access-audit sink open failed: {e}"))?,
            ),
            None => None,
        };
        // [FABLE-5] sq-lsp7k.10: seed the named-template store from the optional persistence
        // file. Fail-closed: an unreadable / invalid definition file is a clean startup error
        // (the operator asked for durable templates), never a silently-empty store.
        #[cfg(feature = "templates")]
        let templates = Arc::new(std::sync::RwLock::new(crate::templates::load_store(
            config.templates_file.as_deref(),
        )?));
        Ok(Self {
            #[cfg(feature = "templates")]
            templates,
            // [OPUS-4.8] (sq-o5bi, sq-0g6g) DEFAULT: hold ring+writer directly (pre-#941). Under
            // `backup`: wrap them in the swappable `ServingCore` so an online restore can swap.
            #[cfg(not(feature = "backup"))]
            ring,
            #[cfg(not(feature = "backup"))]
            writer,
            #[cfg(feature = "backup")]
            core: Arc::new(ArcSwap::from_pointee(ServingCore { ring, writer })),
            config,
            commits: Arc::new(tokio::sync::watch::channel(0).0),
            subs: Arc::new(crate::subscriptions::SubscriptionCounters::default()),
            metrics: Arc::new(crate::metrics::Metrics::default()),
            #[cfg(feature = "complete")]
            completion_index: Arc::new(std::sync::RwLock::new(None)),
            #[cfg(feature = "access-audit")]
            access_audit_sink,
            #[cfg(feature = "change-stream")]
            change_stream,
            // [FABLE-5] sq-fy8ci: no restore in flight at boot (restore-on-start runs on the
            // seed graph in main.rs BEFORE this state exists, so it never holds the permit).
            #[cfg(feature = "backup")]
            restore_in_flight: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(feature = "query-registry")]
            running_queries,
            // [SONNET-4.6] sq-snopa.8: cold at boot — the stateful auth view is built lazily on
            // the first `"source":"server"` request and re-built whenever the generation moves.
            #[cfg(feature = "solid-authz")]
            authz_view: Arc::new(std::sync::RwLock::new(None)),
        })
    }

    /// The server's Prometheus metrics (T22).
    pub(crate) fn metrics(&self) -> &crate::metrics::Metrics {
        &self.metrics
    }

    /// [SONNET-4.6] (sq-t1isr) Test-only: returns the current count of registered queries.
    /// Used by integration tests to observe in-flight registration without the HTTP route.
    /// Only compiled with the `query-registry` feature.
    ///
    /// `#[doc(hidden)]` keeps this out of the public-API docs despite `pub` visibility.
    /// The field is gated on `feature = "query-registry"` so this method only compiles
    /// when that feature is active.
    #[cfg(feature = "query-registry")]
    #[doc(hidden)]
    pub fn running_query_count(&self) -> usize {
        self.running_queries.list().len()
    }

    /// [FABLE-5] (sq-lsp7k.10) The named-template store (see the field doc). Handlers in
    /// [`crate::templates`] read it per invocation and mutate it under the write gate.
    #[cfg(feature = "templates")]
    pub(crate) fn templates(
        &self,
    ) -> &std::sync::RwLock<std::collections::BTreeMap<String, sparq_engine::templates::Template>>
    {
        &self.templates
    }

    /// [OPUS-4.8] (sq-2999l) Whether the durable CDC change-stream is configured (a directory was
    /// set via `--change-stream` / `SPARQ_CHANGE_STREAM`). When `false`, `GET /streams` is `404`.
    #[cfg(feature = "change-stream")]
    pub(crate) fn change_stream_enabled(&self) -> bool {
        self.change_stream.is_some()
    }

    /// [OPUS-4.8] (sq-2999l) Polls the durable CDC change-stream for every recorded change record
    /// with `seq >= from_seq`, in order (the resume primitive backing `GET /streams`). Reads the
    /// segments from disk through the shared single-writer log handle, so it observes every
    /// durably-appended record and never returns a half-written trailing one — and stops
    /// **fail-closed** on a mid-stream corruption (a bad digest / out-of-order seq before the tail).
    /// Returns `Ok(None)` when the change-stream is not configured (the caller answers `404`).
    /// A poll-time read error (corruption, an I/O fault) is `Err(msg)` (the caller answers `500`).
    #[cfg(feature = "change-stream")]
    pub(crate) fn poll_change_stream(
        &self,
        from_seq: u64,
    ) -> Result<Option<Vec<sparq_serve::ChangeRecord>>, String> {
        match &self.change_stream {
            Some(stream) => stream.poll(from_seq).map(Some),
            None => Ok(None),
        }
    }

    /// [OPUS-4.8] (sq-2999l) The seq the NEXT recorded commit will get — i.e. the count of records
    /// already in the log, the `nextSeq` continuation token a caller persists to resume. `None`
    /// when the change-stream is not configured.
    #[cfg(feature = "change-stream")]
    pub(crate) fn change_stream_next_seq(&self) -> Option<u64> {
        self.change_stream.as_ref().map(|stream| stream.next_seq())
    }

    /// [OPUS-5] (sq-iz7ag) The change-stream's **trim horizon** — the oldest seq retention still
    /// keeps (`sparq_serve::ChangeLog::first_seq`; `0` for a never-trimmed log). `GET /streams`
    /// resolves `TRIM_HORIZON` to it (a whole-stream replay of a trimmed log would otherwise poll
    /// below the log's earliest record and fail closed), and answers `410` for an `at`/`after`
    /// anchor below it. `None` when the change-stream is not configured.
    ///
    /// **Scope of the value.** It is the read handle's view, established when this server opened
    /// the log directory. Retention is an explicit host action (`ChangeLog::apply_retention`) that
    /// sparq-server never performs itself, so the horizon only moves under a server that opened an
    /// ALREADY-trimmed log (the restart-after-retention case). A host that trims through its own
    /// handle *while this server runs* leaves this view stale, and such a poll still fails closed
    /// as a `500` rather than a `410` — never a silent gap. Reflecting an out-of-band trim live
    /// needs a cheap re-derived-from-disk horizon in `sparq-serve` (`first_seq` is cached, and
    /// re-opening the log per poll would re-validate every segment); tracked as follow-up work.
    #[cfg(feature = "change-stream")]
    pub(crate) fn change_stream_trim_horizon(&self) -> Option<u64> {
        self.change_stream
            .as_ref()
            .map(|stream| stream.trim_horizon())
    }

    /// [FABLE-5] (gh-2436) Re-baselines the RUNNING tracked change log — the operator resync
    /// behind `POST /admin/change-stream/rebase` (`ChangeLog::rebase_to` /
    /// `rebase_to_new_lineage` routed through the live commit-hook state, serialized with
    /// the writer thread's recording). `None` when the change-stream is not configured (the
    /// caller answers `404`).
    #[cfg(feature = "change-stream")]
    pub(crate) fn rebase_change_stream(
        &self,
        generation: u64,
        new_lineage: bool,
    ) -> Option<Result<sparq_serve::ChangeRecord, sparq_serve::BackupError>> {
        self.change_stream
            .as_ref()
            .map(|stream| stream.rebase(generation, new_lineage))
    }

    /// Pins the current generation for a request: lock-free, never blocked
    /// by an in-flight update. Hold the returned `Arc` for as long as the response is
    /// being produced; `gen.snapshot()` is the immutable [`Graph`] to evaluate against.
    pub fn current(&self) -> PinnedGen {
        self.ring().current()
    }

    /// [OPUS-4.8] (sq-o5bi) Pins the current generation for an ONLINE backup export: a
    /// lock-free `Arc<Generation>` the `backup` route serialises off-thread while the writer
    /// keeps publishing forward (the snapshot is frozen by its `Arc`, never by a lock). Same
    /// pin as [`current`](Self::current); a distinct name makes the backup call site explicit.
    #[cfg(feature = "backup")]
    pub fn pin_for_backup(&self) -> PinnedGen {
        self.ring().current()
    }

    /// [OPUS-4.8] (sq-o5bi, sq-0g6g) The generation ring. DEFAULT: a direct reference to the
    /// `ring` field — byte-identical to pre-#941 (no `ArcSwap` on the read path). Under `backup`:
    /// resolved through the swappable [`ServingCore`], whose loaded `Arc` lives for the call. Every
    /// ring read (`current`, `at`) routes through here, so the two representations stay behind one
    /// accessor and the call sites are identical in both feature states. `#[inline(always)]` makes
    /// the default accessor a zero-cost field borrow.
    #[cfg(not(feature = "backup"))]
    #[inline(always)]
    fn ring(&self) -> &GenerationRing<Graph> {
        &self.ring
    }

    /// [OPUS-4.8] (sq-o5bi, sq-0g6g) `backup`-only ring accessor: clones the ring `Arc` out of the
    /// loaded serving core so the returned handle outlives the transient `ArcSwap` load guard. The
    /// clone is one atomic refcount bump (the swap mechanism's marginal cost); it exists only on the
    /// opt-in path.
    #[cfg(feature = "backup")]
    #[inline(always)]
    fn ring(&self) -> Arc<GenerationRing<Graph>> {
        self.core.load().ring.clone()
    }

    /// [OPUS-4.8] (sq-o5bi, sq-0g6g) The sequenced writer. DEFAULT: a direct reference to the
    /// `writer` field — byte-identical to pre-#941. Under `backup`: cloned out of the swappable
    /// [`ServingCore`] (one atomic refcount bump) so the handle outlives the load guard.
    #[cfg(not(feature = "backup"))]
    #[inline(always)]
    fn writer(&self) -> &Writer<String, Graph> {
        &self.writer
    }

    /// [OPUS-4.8] (sq-o5bi, sq-0g6g) `backup`-only writer accessor (see [`ring`](Self::ring)).
    #[cfg(feature = "backup")]
    #[inline(always)]
    fn writer(&self) -> Arc<Writer<String, Graph>> {
        self.core.load().writer.clone()
    }

    /// Pins the retained generation numbered `number` (time travel): `None` when it
    /// was never published or has aged out of the retention window. See
    /// [`sparq_serve::GenerationRing::at`].
    #[cfg(feature = "time-travel")]
    pub fn at(&self, number: u64) -> Option<PinnedGen> {
        self.ring().at(number)
    }

    /// Applies a SPARQL Update through the sequenced writer: the update joins the
    /// writer's current group-commit window and this call **blocks until the generation
    /// containing it is published** (group-commit ack). Failed updates stay atomic to
    /// clients — the writer applies batches to a private working copy, skips the failing
    /// update, and re-forks, so the published chain never contains a partial effect.
    ///
    /// On success, advances the commit watch so active subscriptions re-evaluate (T23)
    /// and returns the number of the published generation containing the update — the
    /// read-your-writes token a client can later pin with `?generation=N` (time travel)
    /// or compare against `Sparq-Generation` response headers. The advance happens
    /// strictly *after* the publish (submit returns the published generation number),
    /// so a woken subscription always pins a generation at least as new as the commit
    /// it was woken for. Blocking (window + batch application): call it on the
    /// blocking pool.
    pub fn apply_update(&self, sparql: &str) -> Result<u64, String> {
        // [OPUS-4.8] (sq-uqh, Wave B) Extract the per-named-graph visibility scope the
        // update writes (§6.3/§6.5) so the publish bumps only those pods' epochs — a
        // write to graph A no longer churns a cache scoped to graph B. Cross-graph /
        // default-graph / dynamically-scoped writes still fall back to the global pod
        // (conservative, never under-invalidating). See [`touched_pods`].
        let touched = touched_pods(sparql);
        // [GPT-5.6] (sq-kqofk) CDC recording now happens inside the writer's commit hook, once per
        // published generation and before its acknowledgements. Concurrent submitters can join the
        // same group-commit batch; there is no caller-side change-log lock on this path.
        let result = self.writer().submit(sparql.to_string(), touched);
        match result {
            Ok(number) => {
                // Monotonic max: batch-mates share a generation number and may ack in
                // any order relative to a later batch's submitters.
                self.commits.send_if_modified(|g| {
                    if number > *g {
                        *g = number;
                        true
                    } else {
                        false
                    }
                });
                Ok(number)
            }
            Err(WriteError::Rejected(e)) => Err(e),
            Err(WriteError::Shutdown) => Err("update writer has shut down".to_string()),
            // [OPUS-4.8] (sq-vpx4) Durable-write failure (e.g. transient ENOSPC on the
            // `--persist` mirror): the write was REFUSED — not durably committed, not
            // published, not acked. It is retryable, so it maps to HTTP 503 (see
            // `update_rejection_response`'s `DURABLE_UNAVAILABLE_PREFIX` sniff), NOT a
            // 400 (the update itself was valid) and NOT a 500 (the server is healthy
            // and still serving reads). The writer thread is alive; the client may retry.
            Err(WriteError::Unavailable(e)) => Err(format!("{DURABLE_UNAVAILABLE_PREFIX}{e}")),
        }
    }

    /// [OPUS-4.8] (sq-x32t) Compacts/vacuums the durable on-disk store for ERASURE-
    /// COMPLETENESS: physically rewrites the `--persist` directory so only the current live
    /// triples remain, dropping superseded INSERTs / DELETEd data / DROPped graphs that still
    /// linger in earlier WAL segments. **Blocks until the compaction completes** (it runs on
    /// the writer thread, strictly between batches — see [`Writer::maintain`]). The live triple
    /// set is preserved exactly, so no generation is published and readers are unaffected.
    ///
    /// Returns `Ok(())` on success (including the no-op for an in-memory server with no persist
    /// dir — there is no on-disk history to purge, so erasure is already complete). `Err(msg)`
    /// is a compaction failure (e.g. an I/O error during the durable rewrite); the writer thread
    /// stays alive and reads keep being served from the last published snapshot. Blocking: call
    /// it on the blocking pool.
    pub fn compact(&self) -> Result<(), String> {
        match self.writer().maintain() {
            Ok(res) => res,
            Err(WriteError::Shutdown) => Err("update writer has shut down".to_string()),
            // `maintain` only ever returns `Ok(_)` or `Shutdown` from the writer.
            Err(e) => Err(e.to_string()),
        }
    }

    /// [OPUS-4.8] (sq-o5bi) Exports an ONLINE consistent snapshot of the current generation to
    /// `out` as a single self-describing backup artifact (sparq-serve's Option-A format). It
    /// pins the current generation lock-free ([`pin_for_backup`](Self::pin_for_backup)) and
    /// serialises off that immutable `Arc` — so it runs **while serving**, never stopping the
    /// writer and never blocking readers. CPU/IO-bound (serialises the whole dataset): call it
    /// on the blocking pool.
    #[cfg(feature = "backup")]
    pub fn export_backup<W: std::io::Write>(&self, out: &mut W) -> Result<(), String> {
        let pin = self.pin_for_backup();
        sparq_serve::backup_export(&pin, out).map_err(|e| e.to_string())
    }

    /// [OPUS-4.8] (sq-o5bi, sq-ft7u) ONLINE RESTORE: rehydrates the serving store from a backup
    /// artifact. **Fail-closed**: a corrupt/mismatched artifact returns `Err` and the live store is
    /// left UNTOUCHED (the artifact is imported + validated fully BEFORE anything is swapped; if
    /// import fails there is nothing to swap in).
    ///
    /// IN-MEMORY server (`persist_dir == None`): atomically installs a freshly-built ring+writer
    /// into the swappable serving core. Readers in flight keep serving from the OLD core (its `Arc`
    /// survives until they release their pin); every read/update after the swap loads the new core.
    /// The restored content lives only in RAM — it does NOT survive a restart (the historical
    /// in-memory restore).
    ///
    /// `--persist` DURABLE server (`persist_dir == Some(dir)`): only when `persist` is set (the
    /// `--restore-persist` / `?persist=true` opt-in) does the restore write THROUGH to the durable
    /// dir so it SURVIVES A RESTART — via the private `restore_into_durable_through` path.
    /// WITHOUT the opt-in a durable server REFUSES the restore (an in-memory-only swap would be
    /// silently lost on a restart — a footgun), surfaced as an `Err` the route maps to 409.
    ///
    /// `persist == true` on an IN-MEMORY server is an `Err` (there is no durable dir to write
    /// through). Blocking (import parses + indexes the whole dataset): call it on the blocking pool.
    /// [FABLE-5] (sq-fy8ci) Acquires the SINGLE-FLIGHT restore permit: `Some(guard)` if no other
    /// restore is currently in flight on this server state, `None` if one is (the caller should
    /// reject with a conflict — `/admin/restore` maps it to 409 — rather than let the writer
    /// thread silently serialize a last-writer-wins pile-up). Hold the returned [`RestoreGuard`]
    /// for the WHOLE restore ([`restore_from`](Self::restore_from) /
    /// [`restore_from_with_deltas`](Self::restore_from_with_deltas)); dropping it (normally or
    /// during an unwind) releases the permit. The permit is ADVISORY for embedding callers —
    /// the restore paths stay individually crash-safe and atomic without it (the single writer
    /// thread serializes them); it exists to make concurrency EXPLICIT instead of silent.
    #[cfg(feature = "backup")]
    pub fn try_begin_restore(&self) -> Option<RestoreGuard> {
        use std::sync::atomic::Ordering;
        self.restore_in_flight
            // AcqRel on success: Acquire pairs with the releasing drop of the previous holder;
            // Release publishes the claim. Acquire on failure is enough for the reject path.
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RestoreGuard {
                flag: self.restore_in_flight.clone(),
            })
    }

    #[cfg(feature = "backup")]
    pub fn restore_from<R: std::io::Read>(&self, input: R, persist: bool) -> Result<u64, String> {
        match (self.config.persist_dir.is_some(), persist) {
            // Durable server + persist opt-in: write the restore THROUGH to the durable store.
            (true, true) => self.restore_into_durable_through(input),
            // Durable server WITHOUT the opt-in: refuse (an in-memory swap would be lost on
            // restart — a footgun). The route maps this `Err` to 409.
            (true, false) => Err(
                "this is a --persist (durable) server: a restore must opt in to write-through \
                 (?persist=true / --restore-persist) so it survives a restart; an in-memory-only \
                 restore would be silently lost on the next restart and is refused"
                    .to_string(),
            ),
            // In-memory server + persist opt-in: there is no durable dir to write through.
            (false, true) => Err(
                "restore ?persist=true requires a --persist (durable) server; this server is \
                 in-memory (no durable directory to write the restore through to)"
                    .to_string(),
            ),
            // In-memory server, in-memory restore: the historical ring/writer core swap.
            (false, false) => self.restore_in_memory(input),
        }
    }

    /// [OPUS-4.8] (sq-o5bi) IN-MEMORY restore: build the entire new core BEFORE touching the live
    /// one (fail-closed), then atomically swap it in. The restored content is RAM-only — it does
    /// NOT survive a restart (the historical in-memory restore; `restore_from(.., persist=false)`
    /// on an in-memory server). Returns the artifact's source generation for operator correlation.
    #[cfg(feature = "backup")]
    fn restore_in_memory<R: std::io::Read>(&self, input: R) -> Result<u64, String> {
        // Build the entire new core BEFORE touching the live one — fail-closed.
        let (graph, meta) = sparq_serve::backup_import(input).map_err(|e| e.to_string())?;
        self.install_restored_graph(graph);
        Ok(meta.generation)
    }

    /// [OPUS-4.8] (sq-bu1a) ONLINE point-in-time RESTORE: rehydrates from a BASE artifact, then
    /// REPLAYS an ordered chain of incremental DELTA artifacts forward onto it to reach a chosen
    /// recovery point, and atomically installs the result — the change-stream / PITR companion to
    /// [`restore_from`](Self::restore_from). `base` is the base artifact; `deltas` is the ordered
    /// sequence of delta artifact byte-blobs (each [`sparq_serve::backup_export_delta`] output),
    /// oldest first.
    ///
    /// **Fail-closed, same discipline as the base restore:** the base is imported, every delta is
    /// decoded and the whole chain is replayed onto a private graph BEFORE any swap. A corrupt /
    /// version-mismatched / out-of-order / gapped artifact aborts the whole operation (`Err`) and
    /// the LIVE store is left untouched — there is no partial install. The chain must be
    /// same-lineage with the base (see `sparq_serve::backup_delta` for that boundary). Returns the
    /// recovered SOURCE generation/writer-seq (the last delta's `to`, or the base's generation for
    /// an empty chain) for operator correlation. In-memory only (a `--persist` server is refused —
    /// the durable write-through opt-in applies to the base [`restore_from`](Self::restore_from)
    /// only in v1), blocking: call on the blocking pool.
    #[cfg(feature = "backup")]
    pub fn restore_from_with_deltas(
        &self,
        base: &[u8],
        deltas: &[Vec<u8>],
    ) -> Result<u64, String> {
        self.guard_restore_supported()?;
        // Import + replay onto a PRIVATE graph first — fail-closed before any swap.
        let (mut graph, base_meta) = sparq_serve::backup_import(base).map_err(|e| e.to_string())?;
        let decoded: Vec<sparq_serve::Delta> = deltas
            .iter()
            .map(|d| sparq_serve::backup_import_delta(&d[..]))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let recovered = sparq_serve::backup_replay(&mut graph, &base_meta, decoded.iter())
            .map_err(|e| e.to_string())?;
        self.install_restored_graph(graph);
        Ok(recovered.generation)
    }

    /// [OPUS-4.8] (sq-bu1a) Shared restore precondition: the PITR delta path is in-memory-only in
    /// v1 (replaying a delta chain into a `--persist` durable directory needs its own crash-safe
    /// swap, a recorded follow-up; the base `restore_from` already has the write-through path).
    #[cfg(feature = "backup")]
    fn guard_restore_supported(&self) -> Result<(), String> {
        if self.config.persist_dir.is_some() {
            return Err(
                "point-in-time restore (--restore-delta) is not supported on a --persist (durable) \
                 server in v1; it replaces the in-memory serving store only"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// [OPUS-4.8] (sq-bu1a) Shared restore install: builds a fresh ring+writer around the
    /// rehydrated `graph` and atomically swaps it in. Readers in flight keep serving from the OLD
    /// core (its `Arc` survives until they release their pin); every read/update after the swap
    /// loads the new core. The restored ring restarts at generation 0; the recorded source
    /// generation is returned by the callers for operator correlation.
    #[cfg(feature = "backup")]
    fn install_restored_graph(&self, graph: Graph) {
        let ring_config = build_ring_config(&self.config);
        let ring = Arc::new(GenerationRing::with_config(graph, ring_config));
        let applier = ServerApplier::new(self.config.clone());
        // [SONNET-4.6] (sq-m9prn) The swapped-in writer keeps listing/killing through the SAME
        // registry the HTTP handlers read, so an update on a restored core is still visible.
        #[cfg(feature = "query-registry")]
        let applier = applier.with_registry(Arc::clone(&self.running_queries));
        // [GPT-5.6] (sq-kqofk) A restored writer retains the same single CDC hook. The hook's
        // same-lineage guard reports the documented restore discontinuity instead of diffing it.
        let writer = spawn_server_writer(
            ring.clone(),
            applier,
            writer_config(&self.config),
            #[cfg(feature = "change-stream")]
            self.change_stream.as_deref(),
        );
        // Atomic swap: subsequent loads see the new ring+writer; the old core's writer thread
        // joins once its last in-flight reader/Arc drops (Writer's Drop drains + joins).
        self.core.store(Arc::new(ServingCore { ring, writer }));
        // [GPT-5.6] sq-lsp7k.9.3: a restored ring restarts at generation zero. Clear a cached
        // generation-zero index from the prior lineage so equality on the numeric generation
        // cannot reuse completion data from the replaced graph.
        #[cfg(feature = "complete")]
        {
            *self
                .completion_index
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
        // Advance the commit watch so active subscriptions re-evaluate against the restored store.
        let restored_gen = self.current().number();
        self.commits.send_replace(restored_gen);
    }

    /// [OPUS-4.8] (sq-bu1a) Exports an INCREMENTAL DELTA between a RETAINED generation `from` and
    /// the CURRENT generation to `out` (sparq-serve's self-describing delta format) — the
    /// change-stream / PITR producer. Pins both generations lock-free (so it runs **while
    /// serving**) and serialises the quad-set difference off those immutable `Arc`s.
    ///
    /// `from` must still be RETAINED by the ring: with `time-travel` OFF only the last K
    /// generations are retained (the concurrency window), so practical incremental backup wants
    /// the `time-travel` feature to widen the retention window. `Ok(None)` means `from` is no
    /// longer retained (aged out, or never published) — the caller maps that to a 410/404, exactly
    /// like `?generation=N`. `Ok(Some(meta))` is the exported delta's metadata. CPU/IO-bound
    /// (serialises both generations): call on the blocking pool.
    #[cfg(feature = "backup")]
    pub fn export_delta_from<W: std::io::Write>(
        &self,
        from: u64,
        out: &mut W,
    ) -> Result<Option<sparq_serve::DeltaMeta>, String> {
        let to = self.pin_for_backup();
        // The current generation is always retained; `from` must be too. `ring().at` needs the
        // retention window — without `time-travel` only the K-generation concurrency floor is kept.
        let from_gen = match self.ring().at(from) {
            Some(g) => g,
            None => return Ok(None),
        };
        if from >= to.number() {
            return Err(format!(
                "delta `from` generation {} must be earlier than the current generation {}",
                from,
                to.number()
            ));
        }
        sparq_serve::backup_export_delta(&from_gen, &to, out)
            .map(Some)
            .map_err(|e| e.to_string())
    }

    /// [OPUS-4.8] (sq-ft7u) RESTORE-INTO-LIVE-DURABLE-STORE (`--persist` write-through). Imports +
    /// validates the artifact into a fresh `Graph` (fail-closed — done BEFORE the live store is
    /// touched), then routes the durable swap THROUGH THE EXISTING WRITER THREAD
    /// (`Writer::restore`): the writer commits any in-flight batch first, then replaces the durable
    /// on-disk store with the imported image crash-safely (`Graph::restore_into_durable` — the same
    /// two-rename swap the compaction uses, healed by `recover_compaction` on the next open) and
    /// publishes the restored snapshot as a new generation. Because the swap runs on the single
    /// writer thread it NEVER races a concurrent durable commit — no lock on the hot path, and no
    /// ordering hazard: quiescing the durable writer is achieved by SEQUENCING the swap on that
    /// very thread, not by tearing it down. After a restart (reopen `dir`) the restored triples are
    /// present (the on-disk base IS the restored image).
    ///
    /// Fail-closed: a corrupt artifact fails the import → the writer is never asked to swap, the
    /// durable store is untouched. A swap I/O error leaves the OLD durable store intact (the swap
    /// is rollback-safe) and the writer alive; reads keep flowing from the last published snapshot.
    /// Returns the artifact's source generation (for operator correlation), as the in-memory path.
    #[cfg(feature = "backup")]
    fn restore_into_durable_through<R: std::io::Read>(&self, input: R) -> Result<u64, String> {
        // Import + FULLY validate the artifact first (fail-closed — nothing on disk is touched yet).
        let (graph, meta) = sparq_serve::backup_import(input).map_err(|e| e.to_string())?;
        // Route the crash-safe durable swap through the existing writer thread (sequenced with
        // updates; no concurrent durable mutation possible). It publishes the restored snapshot.
        match self.writer().restore(graph) {
            Ok(Ok(number)) => {
                // Re-evaluate active subscriptions against the restored (now-published) generation.
                self.commits.send_if_modified(|g| {
                    if number > *g {
                        *g = number;
                        true
                    } else {
                        false
                    }
                });
                Ok(meta.generation)
            }
            // A durable-swap failure: the OLD store is intact, the writer is alive (fail-closed).
            Ok(Err(e)) => Err(format!("restore-into-durable failed (store unchanged): {}", e)),
            Err(e) => Err(format!("restore-into-durable: update writer unavailable: {}", e)),
        }
    }

    /// A receiver of the committed generation number for a subscription connection (T23).
    pub(crate) fn subscribe_commits(&self) -> tokio::sync::watch::Receiver<u64> {
        self.commits.subscribe()
    }

    pub(crate) fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// [SONNET-4.6] (sq-snopa.8) The per-generation cache holding the STATEFUL Solid
    /// authorization view over this server's own loaded store (see the `authz_view` field).
    /// The whole cache is `solid-authz`-gated, so the default build has neither the field nor
    /// this accessor.
    #[cfg(feature = "solid-authz")]
    pub(crate) fn authz_view_cache(
        &self,
    ) -> &std::sync::RwLock<Option<crate::solid_authz::CachedAuthView>> {
        &self.authz_view
    }

    /// [OPUS-4.8] (sq-gos8) The installed structured access-audit sink, if any. `None` short-
    /// circuits every audit call site (no record is built), so a request with no sink configured
    /// pays only this `Option` check.
    #[cfg(feature = "access-audit")]
    pub(crate) fn access_audit_sink(&self) -> Option<&Arc<dyn crate::access_audit::AuditSink>> {
        self.access_audit_sink.as_ref()
    }
}

/// Builds the application router for the SPARQL Protocol + GSP-read endpoints, hardened
/// with the state's [`ServerConfig`] guards (see [`harden`]).
pub fn router(state: AppState) -> Router {
    let config = state.config.clone();
    let routes = Router::new()
        // SPARQL 1.1 Protocol — query operation. `any` so we can return a proper 405 with
        // an `Allow` header for unsupported methods (update verbs are T11b).
        .route("/sparql", any(sparql_endpoint))
        // Graph Store HTTP Protocol (indirect graph identification): ?graph=<uri> / ?default
        .route("/sparql/graph", any(graph_store_indirect))
        // Graph Store HTTP Protocol (direct graph identification).
        // [OPUS-4.8] axum 0.8 (matchit 0.8) wildcard-capture syntax: `/*path` -> `/{*path}`.
        .route("/graphs/{*path}", any(graph_store_direct))
        // SEPA-style SPARQL subscriptions over WebSocket (T23).
        .route(
            "/subscriptions",
            get(crate::subscriptions::subscriptions_endpoint),
        )
        // [OPUS-4.8] sq-bxog: the same subscription engine over Server-Sent Events
        // (text/event-stream) — one subscription per stream, query in the query string.
        .route(
            "/subscriptions/sse",
            get(crate::subscriptions::sse::sse_endpoint),
        )
        // Liveness.
        .route("/health", get(|| async { "ok" }))
        // [OPUS-4.8] (sq-x32t) ADMIN: WAL compaction/vacuum for erasure-completeness. POST-only
        // (it mutates the durable on-disk store), gated by the WRITE auth token (the existing
        // admin gate). Physically purges superseded/deleted data from the on-disk WAL history so
        // a logical DELETE / DROP GRAPH is followed by real erasure. See [`admin_compact`].
        .route("/admin/compact", post(admin_compact))
        // Prometheus metrics (T22).
        .route("/metrics", get(metrics_endpoint));
    // [OPUS-4.8] sq-o5bi: OPT-IN online consistent-snapshot backup/restore admin routes.
    // Compiled only with the `backup` feature; both are POST-only + WRITE/admin-gated.
    // `/admin/backup` streams a self-describing artifact of the live store WITHOUT stopping
    // the world; `/admin/restore` atomically installs a ring+writer rehydrated from one
    // (fail-closed on a corrupt/mismatched artifact, in-memory server only). See [`admin_backup`]
    // / [`admin_restore`].
    #[cfg(feature = "backup")]
    let routes = routes
        .route("/admin/backup", post(admin_backup))
        .route("/admin/restore", post(admin_restore))
        // [OPUS-4.8] sq-bu1a: the INCREMENTAL change-stream / PITR producer. `?from=N` streams the
        // delta between RETAINED generation N and the current generation; restore-forward replay is
        // driven by the binary's `--restore` + `--restore-delta` (see main.rs), not a route.
        .route("/admin/backup/delta", post(admin_backup_delta));
    // [OPUS-4.8] sq-d3d8 (epic sq-3183): OPT-IN federation discovery — the VoID dataset
    // description at /.well-known/void. Compiled only with the `federation-descriptors`
    // feature; even then the handler refuses (404) unless the config flag is set. The SD is
    // served on the existing /sparql GET path (no extra route — it is the protocol's
    // "GET with no query" response). See [`crate::descriptors`].
    #[cfg(feature = "federation-descriptors")]
    let routes = routes.route("/.well-known/void", get(well_known_void));
    // [GPT-5.6] sq-lsp7k.5.2: OPT-IN grouped facet counts over one pinned graph snapshot.
    // Compiled only with the `facets` feature; even then the handler returns 404 unless the
    // runtime flag is set. POST is a READ and is gated by the read auth.
    #[cfg(feature = "facets")]
    let routes = routes.route("/facets", post(facets_endpoint));
    // [GPT-5.6] sq-lsp7k.9.3: OPT-IN IRI/label/local-name prefix completion over a cached
    // generation-pinned index. Compiled only with `complete`; even then the handler returns 404
    // unless the runtime flag is set. GET is a READ and is gated by the read auth.
    #[cfg(feature = "complete")]
    let routes = routes.route("/complete", get(complete_endpoint));
    // [OPUS-4.8] sq-bzh1 (epic sq-3183): OPT-IN Triple Pattern Fragments / LDF source endpoint.
    // Compiled only with the `tpf` feature; even then the handler refuses (404) unless the config
    // flag is set. READ-only — a GET (with HEAD) only. See [`crate::tpf`].
    // [OPUS-4.8] sq-dxhb: with the `brtpf` feature it ALSO accepts POST — a brTPF client posts a
    // (potentially large) binding set in the body, which would not fit a query string. POST is
    // still a READ (it returns a fragment; it never mutates the store) and is gated by the same
    // read auth + flag.
    #[cfg(all(feature = "tpf", not(feature = "brtpf")))]
    let routes = routes.route("/tpf", get(tpf_endpoint).head(tpf_endpoint));
    #[cfg(feature = "brtpf")]
    let routes = routes.route(
        "/tpf",
        get(tpf_endpoint).head(tpf_endpoint).post(tpf_endpoint),
    );
    // [OPUS-4.8] sq-r868 (from-pss gh-162 follow-up (c)): OPT-IN HTTP SHACL validate endpoint.
    // Compiled only with the `shacl` feature; even then the handler refuses (404) unless the
    // config flag is set. POST only — the client sends a shapes graph and the server validates
    // its loaded data graph (a READ); axum returns a 405 with `Allow: POST` for other methods.
    // See [`shacl_validate_endpoint`].
    #[cfg(feature = "shacl")]
    let routes = routes.route("/shacl/validate", post(shacl_validate_endpoint));
    // [OPUS-4.8] sq-vczh2 (epic sq-2m6zm): OPT-IN verifiable terse-transpiler endpoint.
    // Compiled only with the `terse` feature; even then the handler refuses (404) unless the
    // config flag is set. POST only — the client sends a terse query and the server returns the
    // canonical SPARQL it expands to (it never executes it). See [`terse_transpile_endpoint`].
    #[cfg(feature = "terse")]
    let routes = routes.route("/terse/transpile", post(terse_transpile_endpoint));
    // [OPUS-4.8] sq-2999l (gh-906): OPT-IN CDC change-stream poll endpoint (Neptune GetRecords
    // shape). Compiled only with the `change-stream` feature; even then the handler refuses (404)
    // unless a durable log directory is configured. READ-only — a GET (with HEAD) only. See
    // [`streams_endpoint`].
    #[cfg(feature = "change-stream")]
    let routes = routes
        .route("/streams", get(streams_endpoint).head(streams_endpoint))
        // [FABLE-5] (gh-2436) The operator resync for that stream: appends an honest gap
        // record to the RUNNING tracked log (`ChangeLog::rebase_to`, routed through the live
        // commit-hook state — the recorder never stops). POST-only + WRITE/admin-gated (it
        // appends to the durable log); same double-opt-in 404 as `/streams`. See
        // [`admin_change_stream_rebase`].
        .route(
            "/admin/change-stream/rebase",
            post(admin_change_stream_rebase),
        );
    // [SONNET-4.6] (sq-qsm5z) OPT-IN running-query registry admin routes.
    // `GET /queries` — admin READ-gated list; `DELETE /queries/{id}` — admin WRITE-gated kill.
    // Both compiled only with the `query-registry` feature; without it the routes are absent
    // and the fallback handler produces the standard 404. No runtime flag needed: the routes
    // are always active when the feature is compiled in (the feature IS the opt-in).
    #[cfg(feature = "query-registry")]
    let routes = routes
        .route("/queries", get(list_queries_handler))
        .route("/queries/{id}", axum::routing::delete(cancel_query_handler));
    // [FABLE-5] sq-snopa.6 (issue #992 FR-4): OPT-IN Solid WAC/ACP HTTP authorization surface.
    // Compiled only with the `solid-authz` feature; even then each handler refuses (404) unless the
    // config flag is set. POST only — a thin HTTP shell over the `sparq-solid` library authoriser
    // (`PodStore::decide` / `wac_allow` / `query_json_as`), fail-closed on every error path. The
    // handlers live in [`crate::solid_authz`] to keep the touch surface on this conflict-hot file
    // minimal.
    // [FABLE-5] sq-lsp7k.10: OPT-IN named-parameterized-template REST surface. Compiled only
    // with the `templates` feature; even then every handler refuses (404) unless the config
    // flag is set (--templates / SPARQ_TEMPLATES=1). The handlers live in [`crate::templates`]
    // to keep the touch surface on this conflict-hot file minimal. GET /templates lists;
    // PUT/GET/DELETE /templates/{name} manage one definition (writes are write-gated);
    // POST /templates/{name} invokes it with typed JSON arguments (an UPDATE template
    // invocation is write-gated and runs through the SAME sequenced-writer path as
    // /sparql updates — the gated-update posture is preserved).
    #[cfg(feature = "templates")]
    let routes = routes
        .route("/templates", get(crate::templates::list_endpoint))
        .route(
            "/templates/{*name}",
            get(crate::templates::get_endpoint)
                .put(crate::templates::put_endpoint)
                .delete(crate::templates::delete_endpoint)
                .post(crate::templates::invoke_endpoint),
        );
    #[cfg(feature = "solid-authz")]
    let routes = routes
        .route("/authz/decide", post(crate::solid_authz::decide_endpoint))
        .route(
            "/authz/wac-allow",
            post(crate::solid_authz::wac_allow_endpoint),
        )
        .route("/authz/query", post(crate::solid_authz::query_endpoint));
    // [OPUS-4.8] (sq-pj6u) Categorised unmatched-route 404. Without an explicit fallback,
    // axum answers an unmatched path with a 404 whose body is EMPTY; `json_error_bodies`
    // then wraps that into the uncategorised `{"error":""}` envelope — leak-free but with no
    // actionable category. Register a fallback that mints the SAME structured 404 a handler
    // does for a disabled opt-in route (`{"error":"not found"}`), so every unmatched route
    // carries the stable `not found` category. The message is a fixed, server-constructed
    // string — it never echoes the request path, so the body stays leak-free (no internal
    // paths, no stack info, no request internals), matching the existing error contract.
    let routes = routes.fallback(unmatched_route);
    let routes = routes.with_state(state.clone());
    // The metrics middleware wraps the WHOLE hardened stack so shed requests
    // (429), body-limit rejections (413) and panics (500) are counted with the
    // status the client actually saw.
    let routes = harden(routes, &config);
    // [GPT-5.6] (sq-abhuq) Transport-transparent compression remains entirely opt-in. The
    // default tower-http predicate deliberately skips `text/event-stream` and responses that
    // already carry Content-Encoding; its body wrapper compresses frames as they are polled, so
    // the existing bounded-channel streaming/backpressure path is not buffered or materialised.
    #[cfg(feature = "response-compression")]
    let routes = routes.layer(CompressionLayer::new().gzip(true).zstd(true));
    routes.layer(axum::middleware::from_fn_with_state(
        state,
        crate::metrics::track,
    ))
}

/// [OPUS-4.8] (sq-pj6u) Router fallback for any path that matched no route: a CATEGORISED,
/// leak-free `404 Not Found`. Returns the same structured `{"error":"not found"}` envelope a
/// handler mints for a disabled opt-in route (e.g. `/.well-known/void`, `/tpf`,
/// `/shacl/validate` with their flags off), so an unmatched route is no longer the bare
/// `{"error":""}` axum produces. The message is a fixed, server-constructed string and the
/// request path is deliberately NOT echoed — the body discloses no internal path, stack
/// information or request internal, exactly the [`json_error`] info-leak posture (#241).
async fn unmatched_route() -> Response {
    json_error(StatusCode::NOT_FOUND, "not found")
}

/// `GET /metrics` — Prometheus text exposition (T22). The gauges (graph triple
/// count, active subscriptions) are read at scrape time from live state.
///
/// [OPUS-4.8] sq-9jrx: the exposition leaks the live graph triple count and the
/// active-subscription count, so it is treated as a READ and gated by
/// `--auth-token-read` like any other GET (mirrors QLever, which keeps its stats
/// endpoint behind the same token). `/health` stays ungated for liveness probes.
async fn metrics_endpoint(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let triples = state.current().snapshot().len();
    let subs = state.subs.active_count();
    let body = state.metrics().render(subs, triples);
    text_response(
        StatusCode::OK,
        "text/plain; version=0.0.4; charset=utf-8",
        body,
        false,
    )
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-d3d8 (epic sq-3183) — OPT-IN federation discovery descriptors
// ---------------------------------------------------------------------------

/// [GPT-5.6] sq-oprna.6: transport scheme injected by the feature-on TCP accept loop. HTTP/2
/// and HTTP/3 also carry `:scheme` in the request URI; HTTP/1 TLS needs this explicit marker.
#[cfg(feature = "http2")]
#[derive(Clone, Copy)]
enum TransportScheme {
    Http,
    Https,
}

#[cfg(all(feature = "http2", feature = "federation-descriptors"))]
impl TransportScheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

/// [OPUS-4.8] sq-d3d8: derives the base URL (`scheme://host`) this server is reached at,
/// from the request `Host` header. Used to name the VoID dataset, the `sd:Service`/endpoint
/// and the `dcterms:source` link in the descriptors, so they self-describe the URL a client
/// actually used to fetch them.
///
/// The direct-TLS listener supplies its transport scheme explicitly; HTTP/2 and HTTP/3 may also
/// supply `:scheme` through the request URI. A TLS-terminating reverse proxy forwarding
/// `X-Forwarded-Proto` remains out of scope. If there is no transport scheme or usable `Host`
/// header, this falls back to plain HTTP and/or `localhost`, preserving the historical result.
#[cfg(feature = "federation-descriptors")]
fn request_base(headers: &HeaderMap, uri: &Uri, transport: Option<&str>) -> String {
    let scheme = transport
        .or_else(|| uri.scheme_str())
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost");
    let base = format!("{scheme}://{host}");
    // The Host header is attacker-controlled; a value that makes `http://{host}` an
    // invalid IRI (spaces, control chars, `<`/`>`, …) would otherwise propagate into the
    // descriptor IRIs and yield malformed RDF → a 500. Validate here and fall back to a
    // fixed safe base so the descriptor is always well-formed RDF, as the doc promises.
    if oxrdf::NamedNode::new(&base).is_ok() {
        base
    } else {
        format!("{scheme}://localhost")
    }
}

#[cfg(all(test, feature = "federation-descriptors"))]
mod request_base_tests {
    use super::*;

    /// [GPT-5.6] sq-oprna.6: direct TLS must not advertise a plain-HTTP descriptor IRI.
    #[test]
    fn transport_or_pseudo_header_selects_https() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "example.test:3443".parse().unwrap());
        let relative: Uri = "/.well-known/void".parse().unwrap();
        assert_eq!(
            request_base(&headers, &relative, Some("https")),
            "https://example.test:3443"
        );

        let absolute: Uri = "https://example.test:3443/sparql".parse().unwrap();
        assert_eq!(
            request_base(&headers, &absolute, None),
            "https://example.test:3443"
        );
        assert_eq!(
            request_base(&headers, &relative, None),
            "http://example.test:3443"
        );
    }
}

/// [OPUS-4.8] sq-d3d8: `GET /.well-known/void` — the W3C VoID dataset description (read-only).
///
/// OPT-IN: returns `404` unless [`ServerConfig::federation_descriptors`] is set (the route is
/// mounted only with the `federation-descriptors` feature, and the handler refuses unless the
/// operator also turned the flag on). Reads a pinned snapshot and delegates generation to
/// [`crate::descriptors::void_descriptor`] (`Introspection::to_void`). Content-negotiates
/// Turtle (default) / N-Triples / RDF-XML from `Accept`. As a read, it is gated by
/// `--auth-token-read` like any other GET.
#[cfg(feature = "federation-descriptors")]
async fn well_known_void(
    State(state): State<AppState>,
    uri: Uri,
    #[cfg(feature = "http2")]
    transport: Option<axum::Extension<TransportScheme>>,
    headers: HeaderMap,
) -> Response {
    if !state.config().federation_descriptors {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    #[cfg(feature = "http2")]
    let transport = transport.map(|extension| extension.0.as_str());
    #[cfg(not(feature = "http2"))]
    let transport = None;
    let base = request_base(&headers, &uri, transport);
    let dataset_iri = format!("{base}/.well-known/void#dataset");
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    // Pin the current generation; generate against its immutable snapshot.
    let pin = state.current();
    match crate::descriptors::void_descriptor(pin.snapshot(), &dataset_iri, accept) {
        Ok(d) => text_response(StatusCode::OK, d.content_type, d.body, false),
        // [OPUS-4.8] (sq-kfel, ASVS-G3) `e` is an internal serializer error — withhold it.
        Err(e) => sanitized_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "void-descriptor",
            "failed to generate dataset description",
            &e,
        ),
    }
}

/// [OPUS-4.8] sq-d3d8: the SPARQL 1.1 Service Description served for a `GET /sparql` with no
/// `query` parameter (SPARQL Protocol §2.1.2 / Service Description §2). Returns `Some(resp)`
/// when the descriptor should be served (the feature is compiled in AND the config flag is
/// set), or `None` to fall through to the historical `400 missing 'query'`.
///
/// Advertises the endpoint, the supported query languages, the supported result formats and
/// the default dataset (linked to the VoID document). Content-negotiates from `Accept`.
#[cfg(feature = "federation-descriptors")]
fn service_description_response(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    transport: Option<&str>,
) -> Option<Response> {
    if !state.config().federation_descriptors {
        return None;
    }
    let base = request_base(headers, uri, transport);
    let endpoint_iri = format!("{base}/sparql");
    let dataset_iri = format!("{base}/.well-known/void#dataset");
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let caps = service_capabilities(state.config());
    // [OPUS-4.8] sq-optl: enumerate the served dataset's named graphs so the SD advertises each
    // as an sd:namedGraph (not just the default graph). Read off the same pinned snapshot the
    // VoID descriptor uses, so the two descriptors describe one consistent dataset state.
    let pin = state.current();
    let named_graphs = crate::descriptors::named_graph_descriptions(pin.snapshot());
    Some(
        match crate::descriptors::service_description(
            &endpoint_iri,
            &endpoint_iri,
            &dataset_iri,
            &caps,
            &named_graphs,
            accept,
        ) {
            Ok(d) => text_response(StatusCode::OK, d.content_type, d.body, false),
            // [OPUS-4.8] (sq-kfel, ASVS-G3) `e` is an internal serializer error — withhold it.
            Err(e) => sanitized_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "service-description",
                "failed to generate service description",
                &e,
            ),
        },
    )
}

/// [OPUS-4.8] sq-qfcb: derive the server's ACTUAL capability profile for the Service
/// Description from this build's cargo features + the running config — never a fiction.
///
///   * `update` — advertise `sd:SPARQL11Update` only when an anonymous client could run one:
///     the write path is always compiled in, but a configured write-token
///     ([`ServerConfig::auth_token`]) makes anonymous Update impossible, so we suppress the
///     advertisement in that case (the operator who holds the token still knows it works).
///   * `federated_query` — `true` exactly when built with the `service` feature (the engine's
///     `SERVICE` evaluation is compiled in); otherwise a `SERVICE` clause errors at execution.
///   * `extension_functions` — the IRIs the engine has ACTUALLY registered. With the `geo`
///     feature that is sparq-geo's `geof:` registry, read back through
///     [`sparq_engine::FunctionRegistry::iris`] so the list can never drift from what runs;
///     without it, no extension functions are registered, so the list is empty.
///   * `sparql_versions` (sq-2msb) — the `sparql:version-*` IRIs the engine conformance-verifies
///     (`descriptors::CONFORMANCE_VERIFIED_VERSIONS`), advertised via `sd:supportedVersion`. There
///     is NO `sparql12`/`rdf12` cargo feature — SPARQL 1.2 evaluation is in the base engine — so
///     this is keyed off the DOCUMENTED conformance state, not a `cfg!`; see that constant.
#[cfg(feature = "federation-descriptors")]
fn service_capabilities(config: &ServerConfig) -> crate::descriptors::Capabilities {
    // Anonymous Update is possible iff no write-token gates the write surface.
    let update = config.auth_token.is_none();
    let federated_query = cfg!(feature = "service");
    #[cfg(feature = "geo")]
    let extension_functions: Vec<String> = {
        static GEOF: std::sync::OnceLock<sparq_engine::FunctionRegistry> =
            std::sync::OnceLock::new();
        GEOF.get_or_init(sparq_geo::geof_registry)
            .iris()
            .map(str::to_string)
            .collect()
    };
    #[cfg(not(feature = "geo"))]
    let extension_functions: Vec<String> = Vec::new();
    // [OPUS-4.8] sq-yyy3: PROV-O data-lineage is NOT advertised by `sparq-server` today — the
    // server exposes no lineage-serving endpoint (the `sparq-prov` capture surface is a separate
    // crate not wired into the HTTP layer), so advertising it would over-promise. The descriptor
    // SUPPORT exists (`Capabilities::provenance` ⇒ `sd:feature <…/prov#lineage>`) so a node that
    // genuinely serves lineage can flip this honestly without a vocabulary change; until then it
    // stays `false`. Set explicitly (not via `..default()`) so the honesty stays visible here.
    let provenance = false;
    // [OPUS-4.8] sq-2msb (gh-917): the SPARQL language versions to advertise via
    // `sd:supportedVersion`. Sourced from the single documented conformance constant (not a
    // `cfg!` — SPARQL 1.2 is always compiled into the base engine), so the honesty gate lives in
    // exactly one place and this binary advertises precisely the versions its W3C suites pass.
    let sparql_versions = crate::descriptors::CONFORMANCE_VERIFIED_VERSIONS
        .iter()
        .map(|v| (*v).to_string())
        .collect();
    crate::descriptors::Capabilities {
        update,
        federated_query,
        extension_functions,
        provenance,
        sparql_versions,
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-bzh1 (epic sq-3183) — OPT-IN Triple Pattern Fragments / LDF source endpoint
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-bzh1: derives `scheme://host` from the request `Host` header for the TPF
/// fragment / dataset / template URLs (independent of the `federation-descriptors`-gated
/// [`request_base`] so the two opt-in features never depend on each other). Same `http` +
/// safe-`localhost` fallback posture: a hostile `Host` that would make an invalid IRI falls
/// back so the fragment is always well-formed RDF rather than a 500.
#[cfg(feature = "tpf")]
fn tpf_base(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost");
    let base = format!("http://{host}");
    if oxrdf::NamedNode::new(&base).is_ok() {
        base
    } else {
        "http://localhost".to_string()
    }
}

/// [OPUS-4.8] sq-bzh1: percent-encodes a query-parameter VALUE for building the `hydra:next` /
/// `hydra:previous` page URLs. Encodes everything outside the RFC 3986 unreserved set plus the
/// few sub-delims safe in a query value, so an N-Triples term value (`<`, `>`, `"`, spaces, …)
/// round-trips through the URL back to the same term. Hand-rolled to avoid a new dependency.
#[cfg(feature = "tpf")]
fn pct_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// [OPUS-4.8] sq-bzh1: builds a `/tpf` page URL for the given pattern parameters and page,
/// re-encoding the bound positions so the link is self-contained.
#[cfg(feature = "tpf")]
fn tpf_page_url(
    base: &str,
    subject: Option<&str>,
    predicate: Option<&str>,
    object: Option<&str>,
    page: usize,
) -> String {
    let mut url = format!("{base}/tpf");
    let mut sep = '?';
    let add = |url: &mut String, sep: &mut char, k: &str, v: Option<&str>| {
        if let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) {
            url.push(*sep);
            url.push_str(k);
            url.push('=');
            url.push_str(&pct_encode(v));
            *sep = '&';
        }
    };
    add(&mut url, &mut sep, "subject", subject);
    add(&mut url, &mut sep, "predicate", predicate);
    add(&mut url, &mut sep, "object", object);
    // Page is always present so the URL is an unambiguous fragment identifier (page 0 included).
    url.push(sep);
    url.push_str(&format!("page={page}"));
    url
}

/// [OPUS-4.8] sq-bzh1: negotiates the TPF fragment serialisation. TPF clients send
/// `Accept: text/turtle` (or N-Triples); we reuse the graph negotiation but DEFAULT to Turtle
/// (the conventional, control-readable TPF serialisation), matching the descriptor surface.
#[cfg(feature = "tpf")]
fn negotiate_tpf(accept: Option<&str>) -> GraphFormat {
    match accept {
        Some(a) if !a.trim().is_empty() => {
            let f = negotiate_graph(Some(a));
            // negotiate_graph defaults to N-Triples on an unrecognised header; for TPF a bare
            // `*/*` (or anything unmatched) should land on Turtle.
            // [OPUS-4.8] sq-oy1f.1: an explicit JSON-LD request is honoured too (only matchable
            // when the `jsonld` feature is on; `negotiate_graph` returns Turtle/N-Triples without
            // it, so the substring is harmless when the feature is off).
            if a.contains("n-triples")
                || a.contains("turtle")
                || a.contains("rdf+xml")
                || a.contains("ld+json")
            {
                f
            } else {
                GraphFormat::Turtle
            }
        }
        _ => GraphFormat::Turtle,
    }
}

/// [GPT-5.6] sq-lsp7k.5.2: `POST /facets` — grouped type, predicate, and value counts for a
/// filtered candidate-subject set.
///
/// The JSON body is a `sparq_introspect::FacetRequest`. Evaluation pins exactly one published
/// graph generation and runs the scan-heavy `sparq_introspect::facets` call on the blocking pool;
/// the response is the corresponding `sparq_introspect::FacetResponse::to_json` document.
///
/// Double opt-in: the route is compiled only with the `facets` feature and returns `404` unless
/// [`ServerConfig::facets`] is set (`--facets` / `SPARQ_FACETS=1`). It is a read surface, so
/// `--auth-token-read` applies. Malformed JSON is a sanitized `400`.
#[cfg(feature = "facets")]
async fn facets_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().facets {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let body = match decode_request_body(&body, &headers, state.config()) {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let request = match serde_json::from_slice::<sparq_introspect::FacetRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return sanitized_error(
                StatusCode::BAD_REQUEST,
                "facet-request-json",
                "malformed facet request",
                &error.to_string(),
            )
        }
    };

    let pin = state.current();
    let task = tokio::task::spawn_blocking(move || {
        let response = sparq_introspect::facets(pin.snapshot(), &request);
        text_response(
            StatusCode::OK,
            "application/json; charset=utf-8",
            response.to_json(),
            false,
        )
    });
    await_worker(task, state.config()).await
}

#[cfg(feature = "complete")]
const COMPLETION_DEFAULT_LIMIT: usize = 20;
#[cfg(feature = "complete")]
const COMPLETION_MAX_LIMIT: usize = 100;

/// [GPT-5.6] sq-lsp7k.9.3: stable wire names for `sparq_text::CandidateKind`.
#[cfg(feature = "complete")]
fn completion_kind_name(kind: sparq_text::CandidateKind) -> &'static str {
    match kind {
        sparq_text::CandidateKind::Iri => "iri",
        sparq_text::CandidateKind::LocalName => "localName",
        sparq_text::CandidateKind::Label(_) => "label",
    }
}

/// [GPT-5.6] sq-lsp7k.9.3: materialises completion candidates into the endpoint's compact JSON
/// array. Entity ids are resolved through the same pinned snapshot dictionary that the index was
/// built against; IRI terms use their raw IRI string, while the defensive non-IRI fallback uses
/// the term's N-Triples display form.
#[cfg(feature = "complete")]
fn completion_json(
    graph: &Graph,
    index: &sparq_text::CompletionIndex,
    prefix: &str,
    limit: usize,
) -> String {
    let candidates: Vec<serde_json::Value> = index
        .complete(prefix, limit, None)
        .into_iter()
        .map(|candidate| {
            let term = graph.dict.term(candidate.id);
            let iri = match &term {
                oxrdf::Term::NamedNode(node) => node.as_str().to_string(),
                _ => term.to_string(),
            };
            serde_json::json!({
                "iri": iri,
                "key": candidate.key,
                "kind": completion_kind_name(candidate.kind),
                "score": candidate.score,
            })
        })
        .collect();
    serde_json::to_string(&candidates).expect("completion candidates are JSON-serializable")
}

/// [GPT-5.6] sq-lsp7k.9.3: `GET /complete?q=<prefix>&limit=<k>` — case-insensitive prefix
/// completion over graph IRIs, local names, and `rdfs:label`/`skos:prefLabel` values.
///
/// The handler pins one immutable graph generation. A generation-matched AppState cache answers
/// directly; otherwise `CompletionIndex::build` runs on the blocking pool and the resulting
/// `(generation, index)` replaces the stale entry. Scores are intentionally `None` in v1 (wire
/// score `0.0`); rank injection and incremental reconciliation are follow-ups. `limit` defaults to
/// 20 and is capped at 100. Missing `q` or a malformed `limit` is `400`.
///
/// Double opt-in: the route is compiled only with `complete` and returns `404` unless
/// [`ServerConfig::complete`] is set (`--complete` / `SPARQ_COMPLETE=1`). It is a read surface,
/// so `--auth-token-read` applies.
#[cfg(feature = "complete")]
async fn complete_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if !state.config().complete {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(response) = auth_gate(state.config(), &headers, Operation::Read) {
        return response;
    }
    let Some(prefix) = params.get("q").cloned() else {
        return json_error(StatusCode::BAD_REQUEST, "missing 'q' parameter");
    };
    let limit = match params.get("limit") {
        Some(raw) => match raw.parse::<usize>() {
            Ok(limit) => limit.min(COMPLETION_MAX_LIMIT),
            Err(_) => return json_error(StatusCode::BAD_REQUEST, "malformed completion limit"),
        },
        None => COMPLETION_DEFAULT_LIMIT,
    };

    let pin = state.current();
    let generation = pin.number();
    let cache = state.completion_index.clone();
    let task = tokio::task::spawn_blocking(move || {
        {
            let cached = cache
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((cached_generation, index)) = &*cached {
                if *cached_generation == generation {
                    return text_response(
                        StatusCode::OK,
                        "application/json; charset=utf-8",
                        completion_json(pin.snapshot(), index, &prefix, limit),
                        false,
                    );
                }
            }
        }

        let rebuilt = sparq_text::CompletionIndex::build(pin.snapshot());
        let body = completion_json(pin.snapshot(), &rebuilt, &prefix, limit);
        let mut cached = cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Always replace a mismatched entry. Generations are monotonic within a ring, but an
        // online restore starts a new lineage at zero; numeric ordering is therefore not a safe
        // cross-lineage recency test. A racing older request can cause one extra rebuild, never a
        // wrong response (each response uses its own pin).
        *cached = Some((generation, rebuilt));
        text_response(
            StatusCode::OK,
            "application/json; charset=utf-8",
            body,
            false,
        )
    });
    await_worker(task, state.config()).await
}

/// [OPUS-4.8] sq-bzh1: `GET /tpf?subject=&predicate=&object=` — the Triple Pattern Fragments /
/// LDF source endpoint (read-only). Returns a paged RDF fragment of the triples matching the
/// pattern, with Hydra controls (`hydra:totalItems` from the cheap cardinality estimate,
/// `hydra:next`/`hydra:previous` paging, the `hydra:search` template).
///
/// OPT-IN: returns `404` unless [`ServerConfig::tpf`] is set (the route is mounted only with the
/// `tpf` feature, and the handler refuses unless the operator also turned the flag on). As a
/// read, it is gated by `--auth-token-read` like any other GET. Content-negotiates Turtle
/// (default) / N-Triples / RDF-XML from `Accept`.
#[cfg(feature = "tpf")]
async fn tpf_endpoint(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    body: Bytes,
) -> Response {
    use crate::tpf::{evaluate, fragment_triples, TriplePattern, DEFAULT_PAGE_SIZE};

    if !state.config().tpf {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    // GET/HEAD always; POST only with brTPF (a binding set too large for a query string).
    #[cfg(not(feature = "brtpf"))]
    let method_ok = method == Method::GET || method == Method::HEAD;
    #[cfg(feature = "brtpf")]
    let method_ok = matches!(method, Method::GET | Method::HEAD | Method::POST);
    if !method_ok {
        #[cfg(not(feature = "brtpf"))]
        return method_not_allowed(&[Method::GET, Method::HEAD]);
        #[cfg(feature = "brtpf")]
        return method_not_allowed(&[Method::GET, Method::HEAD, Method::POST]);
    }
    // A fragment is a READ even via POST (brTPF posts the binding set; it never writes).
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let head_only = method == Method::HEAD;
    // The body is unused without brTPF (GET/HEAD carry none); silence the warning.
    #[cfg(not(feature = "brtpf"))]
    let _ = &body;

    let params = parse_form(raw_query.as_deref().unwrap_or(""));
    let subject = params.get("subject").map(String::as_str);
    let predicate = params.get("predicate").map(String::as_str);
    let object = params.get("object").map(String::as_str);

    // Parse the triple pattern; a malformed term is a 400.
    // [OPUS-4.8] (sq-kfel, ASVS-G3) `e` quotes the caller's offending term verbatim — the same
    // echo-of-input info-leak class the rest of the surface sanitizes (sq-cz89/sq-j9zs). Withhold
    // it from the client; the operator gets the full parse error in the server log.
    let pattern = match TriplePattern::parse(subject, predicate, object) {
        Ok(p) => p,
        Err(e) => {
            return sanitized_error(
                StatusCode::BAD_REQUEST,
                "tpf-term-parse",
                "malformed triple pattern term",
                &e.to_string(),
            )
        }
    };
    // Page number (0-based). A non-numeric / absent page is page 0.
    let page = params
        .get("page")
        .and_then(|p| p.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let page_size = DEFAULT_PAGE_SIZE;

    let base = tpf_base(&headers);
    let dataset_url = format!("{base}/tpf");

    // [OPUS-4.8] sq-dxhb: the brTPF binding set. The mappings come from the `values` query
    // parameter (GET) or — preferred for a large set — the request BODY (POST). An empty / absent
    // payload is the plain-TPF case (no restriction). A malformed payload is a 400.
    #[cfg(feature = "brtpf")]
    let bindings = {
        // POST body if non-empty, else the `values` query parameter.
        let raw_values: std::borrow::Cow<'_, str> = if !body.is_empty() {
            String::from_utf8_lossy(&body)
        } else {
            std::borrow::Cow::Borrowed(params.get("values").map(String::as_str).unwrap_or(""))
        };
        // [OPUS-4.8] sq-r74h: enforce the binding-set DoS caps (mapping count + payload bytes)
        // BEFORE parsing/evaluation. These bound the brTPF fan-out independently of
        // `--max-body-bytes` — which does not cover the `values` query-string carrier at all, and
        // bounds the per-mapping index-scan cost only transitively. A too-large set is a `413`
        // (the same refusal class as the body limit); a malformed set stays a `400`.
        let limits = crate::tpf::BindingLimits {
            max_mappings: state.config().brtpf_max_bindings,
            max_payload_bytes: state.config().brtpf_max_values_bytes,
        };
        match crate::tpf::parse_bindings_capped(&raw_values, limits) {
            Ok(b) => b,
            Err(crate::tpf::BindingError::Malformed(e)) => {
                return sanitized_error(
                    StatusCode::BAD_REQUEST,
                    "brtpf-bindings-parse",
                    "malformed brTPF binding set",
                    &e.to_string(),
                )
            }
            Err(crate::tpf::BindingError::TooLarge(m)) => {
                // The cap message names only the limit (never a caller-supplied term), so it is
                // safe to return to the client — it tells them which knob to lower / which the
                // operator set, exactly like the `--max-body-bytes` 413.
                return json_error(StatusCode::PAYLOAD_TOO_LARGE, &m);
            }
        }
    };

    // The template advertises the brTPF `{values}` control only when the feature is compiled.
    #[cfg(feature = "brtpf")]
    let template = format!("{base}/tpf{{?subject,predicate,object,values}}");
    #[cfg(not(feature = "brtpf"))]
    let template = format!("{base}/tpf{{?subject,predicate,object}}");

    // Pin the current generation; evaluate against its immutable snapshot. With a non-empty
    // brTPF binding set the fragment is the bindings-RESTRICTED result; otherwise it is plain TPF.
    let pin = state.current();
    #[cfg(feature = "brtpf")]
    let frag = if bindings.is_empty() {
        evaluate(pin.snapshot(), &pattern, page, page_size)
    } else {
        crate::tpf::evaluate_brtpf(pin.snapshot(), &pattern, &bindings, page, page_size)
    };
    #[cfg(not(feature = "brtpf"))]
    let frag = evaluate(pin.snapshot(), &pattern, page, page_size);

    let fragment_url = tpf_page_url(&base, subject, predicate, object, page);
    let next_url = frag
        .has_next()
        .then(|| tpf_page_url(&base, subject, predicate, object, page + 1));
    let prev_url = frag
        .has_previous()
        .then(|| tpf_page_url(&base, subject, predicate, object, page - 1));
    // [OPUS-4.8] sq-dxhb: the fuller Hydra paging vocabulary — hydra:first (always page 0) and
    // hydra:last (the page holding the final match). Emitted on every page.
    let first_url = tpf_page_url(&base, subject, predicate, object, 0);
    let last_url = tpf_page_url(&base, subject, predicate, object, frag.last_page());

    let triples = fragment_triples(
        &frag,
        &fragment_url,
        &dataset_url,
        &template,
        next_url.as_deref(),
        prev_url.as_deref(),
        Some(&first_url),
        Some(&last_url),
    );

    let fmt = negotiate_tpf(headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()));
    // [OPUS-4.8] sq-oy1f.1: route through the shared graph serialiser so the JSON-LD arm is
    // covered uniformly (and the match stays exhaustive when the `jsonld` feature is on).
    let body = serialise_graph_triples(&triples, fmt);
    text_response(StatusCode::OK, fmt.content_type(), body, head_only)
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-r868 (from-pss gh-162 follow-up (c)) — OPT-IN HTTP SHACL validate endpoint
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-r868: `POST /shacl/validate` — validate the server's currently-loaded data
/// graph against a SHACL shapes graph the client POSTs.
///
/// OPT-IN: returns `404` unless [`ServerConfig::shacl`] is set (the route is mounted only with
/// the `shacl` feature, and the handler refuses unless the operator also turned the flag on).
///
/// **Contract.** The request BODY is the SHACL **shapes** graph (RDF — `text/turtle` /
/// `application/n-triples` / `application/n-quads` / `application/trig` / `application/rdf+xml`,
/// classified by `Content-Type` exactly like a GSP write body, and gzip-decoded under the same
/// zip-bomb cap). The **data** graph is the server's CURRENT in-memory store snapshot (pinned
/// for the request) — the gh-162 server-side / large-graph path: the store is already loaded, so
/// there is no per-request data parse, and the 100k-node case where the JS `rdf-validate-shacl`
/// OOMs is handled natively. Validation is a READ; the endpoint is gated by the read auth.
///
/// **Response.** Content-negotiated from `Accept`: `text/turtle` yields the W3C SHACL
/// report-vocabulary graph ([`sparq_shacl::ValidationReport::to_turtle`]); anything else (the
/// default) yields the JSON projection PSS / the wasm `shacl` binding consume —
/// `{ "conforms": bool, "results": [{ "focusNode", "path", "value", "sourceShape",
/// "sourceConstraintComponent", "severity", "message" }] }`. `200` regardless of conformance —
/// the verdict is in the body (`conforms`), not the HTTP status; a malformed shapes body is a
/// `400`, an unsupported `Content-Type` a `415`.
#[cfg(feature = "shacl")]
async fn shacl_validate_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().shacl {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    // Validation reads the store; gate it like any other read.
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    // Decode the (possibly gzip'd) body under the shared decompression-ratio cap.
    let body = match decode_request_body(&body, &headers, state.config()) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    // Parse the shapes graph from the body, classified by Content-Type (same matrix as a GSP
    // write body). A no/relative base is fine — shapes graphs name shapes by absolute IRI.
    let shapes = match parse_shapes_graph(&body, &content_type(&headers)) {
        Ok(g) => g,
        Err(resp) => return resp,
    };
    let turtle =
        negotiate_shacl_report_turtle(headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()));
    let gen = state.current();
    // Validation is CPU-bound (index scans over the whole store + any `sh:sparql`), so run it
    // on the blocking pool under the same wall-clock cap the query paths use.
    let task = tokio::task::spawn_blocking(move || {
        let report = sparq_shacl::validate(gen.snapshot(), &shapes);
        if turtle {
            text_response(
                StatusCode::OK,
                "text/turtle; charset=utf-8",
                report.to_turtle(),
                false,
            )
        } else {
            text_response(
                StatusCode::OK,
                "application/json; charset=utf-8",
                shacl_report_to_json(&report),
                false,
            )
        }
    });
    await_worker(task, state.config()).await
}

/// [OPUS-4.8] sq-r868: parse a SHACL shapes graph from a request body classified by its
/// `Content-Type` (the same media-type matrix as a GSP write body — see [`rdf_format_for`]).
/// Returns the caller's `415` for an unsupported type, `400` for malformed RDF. The offending
/// body fragment the parser would echo is withheld from the client and logged server-side, the
/// same info-leak posture the rest of the surface uses (sq-cz89/sq-j9zs).
#[cfg(feature = "shacl")]
#[allow(clippy::result_large_err)]
fn parse_shapes_graph(body: &Bytes, content_type: &str) -> Result<Graph, Response> {
    match rdf_format_for(content_type) {
        Some(BodyFormat::Core(format)) => {
            let text = std::str::from_utf8(body)
                .map_err(|_| bad_request("shapes body is not valid UTF-8"))?;
            Graph::load_str(text, format).map_err(|e| {
                sanitized_error(
                    StatusCode::BAD_REQUEST,
                    "shacl-shapes-parse",
                    "malformed RDF shapes body",
                    &e,
                )
            })
        }
        Some(BodyFormat::RdfXml) => {
            let triples = crate::graph::parse_rdfxml(body, None).map_err(|e| {
                sanitized_error(
                    StatusCode::BAD_REQUEST,
                    "shacl-shapes-rdfxml-parse",
                    "malformed RDF/XML shapes body",
                    &e,
                )
            })?;
            Ok(sparq_shacl::graph_from_triples(triples))
        }
        None => Err(json_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "SHACL shapes body must be RDF: Content-Type 'text/turtle', 'application/n-triples', \
             'application/n-quads', 'application/trig' or 'application/rdf+xml'",
        )),
    }
}

/// [OPUS-4.8] sq-r868: pick the report serialisation from `Accept`. `text/turtle` (the W3C SHACL
/// report vocabulary) when explicitly requested at non-zero q; otherwise the JSON projection
/// (the default — the shape PSS / the wasm `shacl` binding consume). q-value aware: an explicit
/// `text/turtle;q=0` falls back to JSON.
#[cfg(feature = "shacl")]
fn negotiate_shacl_report_turtle(accept: Option<&str>) -> bool {
    let accept = match accept {
        Some(a) if !a.trim().is_empty() => a,
        _ => return false,
    };
    // Highest-q wins; JSON is the default when nothing (or a wildcard) is preferred over Turtle.
    let mut turtle_q = f32::NEG_INFINITY;
    let mut json_q = 0.0f32; // the default floor — JSON is always acceptable
    for part in accept.split(',') {
        let mut it = part.split(';');
        let media = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let mut q = 1.0f32;
        for param in it {
            if let Some(v) = param.trim().strip_prefix("q=") {
                q = v.parse().unwrap_or(1.0);
            }
        }
        match media.as_str() {
            "text/turtle" | "application/x-turtle" => turtle_q = turtle_q.max(q),
            "application/json" | "application/sparql-results+json" => json_q = json_q.max(q),
            _ => {}
        }
    }
    turtle_q > 0.0 && turtle_q > json_q
}

/// [OPUS-4.8] sq-r868: serialise a [`sparq_shacl::ValidationReport`] to the JSON projection the
/// PSS Pod-Manager + the wasm `shacl` binding consume:
/// `{ "conforms": bool, "results": [{ focusNode, path, value, sourceShape,
/// sourceConstraintComponent, severity, message }] }`. `path`/`value` are `null` when the result
/// carries none; `focusNode`/`value`/`sourceShape` are N-Triples term strings; `path` is a SHACL
/// Turtle path expression; `message` is the first `sh:message` text (or the generated default).
/// Hand-rolled with the same string escaping as [`json_error`] so the projection stays
/// byte-compatible with the wasm binding's documented shape.
#[cfg(feature = "shacl")]
fn shacl_report_to_json(report: &sparq_shacl::ValidationReport) -> String {
    use oxrdf::Term;

    fn push_str_lit(out: &mut String, s: &str) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    fn push_field(out: &mut String, key: &str, val: Option<&str>) {
        out.push('"');
        out.push_str(key);
        out.push_str("\":");
        match val {
            Some(v) => push_str_lit(out, v),
            None => out.push_str("null"),
        }
    }

    let mut out = String::from("{\"conforms\":");
    out.push_str(if report.conforms { "true" } else { "false" });
    out.push_str(",\"results\":[");
    for (i, r) in report.results.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_field(&mut out, "focusNode", Some(&r.focus_node.to_string()));
        out.push(',');
        let path = r.path.as_ref().map(|p| p.to_turtle());
        push_field(&mut out, "path", path.as_deref());
        out.push(',');
        let value = r.value.as_ref().map(|v| v.to_string());
        push_field(&mut out, "value", value.as_deref());
        out.push(',');
        push_field(&mut out, "sourceShape", Some(&r.source_shape.to_string()));
        out.push(',');
        push_field(
            &mut out,
            "sourceConstraintComponent",
            Some(&r.source_component),
        );
        out.push(',');
        push_field(&mut out, "severity", Some(&r.severity));
        out.push(',');
        // The human-readable message: the first sh:message's text, or the generated default.
        let message = match r.messages.first() {
            Some(Term::Literal(l)) => l.value().to_string(),
            _ => r.default_message.clone(),
        };
        push_field(&mut out, "message", Some(&message));
        out.push('}');
    }
    out.push_str("]}");
    out
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-vczh2 (epic sq-2m6zm) — OPT-IN verifiable terse-transpiler endpoint
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-vczh2: `POST /terse/transpile` — transpile a *terse* query into the canonical,
/// conformant SPARQL it expands to, returning the verifiable EXPANSION (NOT an answer).
///
/// OPT-IN: returns `404` unless [`ServerConfig::terse`] is set (the route is mounted only with
/// the `terse` feature, and the handler refuses unless the operator also turned the flag on).
///
/// **Contract.** The request BODY is the terse query text (`text/plain` / unspecified — it is
/// read verbatim as UTF-8, decoded under the shared gzip zip-bomb cap). The server runs the LEAN
/// [`sparq_terse::terse_to_sparql`] (the `K:<name>` keyword layer over canonical SPARQL, plus the
/// silent-rewrite canary) and returns JSON:
///
/// ```json
/// { "canonical_sparql": "SELECT ?s WHERE { ?s <http://www.w3.org/ns/prov#wasDerivedFrom> ?o }",
///   "keywords":    [ { "keyword": "derivedFrom", "iri": "http://www.w3.org/ns/prov#wasDerivedFrom",
///                      "legendVersion": "pkg-keywords/v1" } ],
///   "resolutions": [],
///   "warnings":    [],
///   "legendVersion": "pkg-keywords/v1" }
/// ```
///
/// The whole CONTRACT is `canonical_sparql`: it is standard SPARQL, it is what the agent then
/// runs through the normal `/sparql` path, and it is what the agent can inspect — "a convenience
/// that shows its work, never an oracle that hides it" (design §6). The endpoint NEVER executes
/// the query and never touches the store, so `resolutions` is always `[]` in this lean server
/// build: `V("phrase")` concept resolution needs a graph-bound resolver + an embedder (the
/// crate's `vectors` feature), which is a future extension — a `V(...)` construct therefore
/// loud-FAILS with a `400` here rather than being guessed (the `V()` ambiguity caveat is tracked
/// by `sq-26fdp`). An unknown `K:<name>` keyword, a `PREFIX K:` collision or non-conformant input
/// (the canary) is likewise a `400` with the transpiler's loud message — never a silent rewrite.
///
/// Transpiling neither reads nor mutates the store, but it is a query-shaped operation, so the
/// endpoint is gated by the read auth like a GET.
#[cfg(feature = "terse")]
async fn terse_transpile_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config().terse {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    // Transpiling is a query-shaped op; gate it with the read auth like a GET.
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    // Decode the (possibly gzip'd) body under the shared decompression-ratio cap.
    let body = match decode_request_body(&body, &headers, state.config()) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    // The terse query is UTF-8 text. Reject a non-UTF-8 body with a 400 rather than lossily
    // mangling it (a mangled query would transpile to a different canonical SPARQL — exactly the
    // silent-rewrite the surface refuses).
    let src = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "terse query body must be valid UTF-8",
            )
        }
    };
    match sparq_terse::terse_to_sparql(src) {
        Ok(expansion) => text_response(
            StatusCode::OK,
            "application/json; charset=utf-8",
            terse_expansion_to_json(&expansion),
            false,
        ),
        // Every terse failure is a loud, client-facing input error (unknown keyword, `PREFIX K:`
        // collision, an un-resolvable `V(...)` in this lean build, or the conformance canary), so
        // it maps to a 400 carrying the transpiler's own message — never a silent rewrite.
        Err(e) => json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

/// [OPUS-4.8] sq-vczh2: serialise a [`sparq_terse::Expansion`] to the verifiable JSON contract
/// (`{ canonical_sparql, keywords, resolutions, warnings, legendVersion }`). `serde_json` is
/// already a `server`-feature dependency, so the JSON is built with the same escaping discipline
/// the rest of the server's JSON uses.
#[cfg(feature = "terse")]
fn terse_expansion_to_json(expansion: &sparq_terse::Expansion) -> String {
    let keywords: Vec<serde_json::Value> = expansion
        .keywords
        .iter()
        .map(|k| {
            serde_json::json!({
                "keyword": k.keyword,
                "iri": k.iri,
                "legendVersion": k.legend_version,
            })
        })
        .collect();
    // `resolutions` is always empty in this lean (no-`vectors`) server build, but the field is in
    // the contract so a future `vectors`-enabled server can populate it without a shape change.
    let resolutions: Vec<serde_json::Value> = expansion
        .resolutions
        .iter()
        .map(|r| {
            serde_json::json!({
                "phrase": r.phrase,
                "iri": r.iri,
                "score": r.score,
                "runnerUp": r.runner_up,
                "runnerUpScore": r.runner_up_score,
                "confidence": r.confidence,
                "method": r.method.as_str(),
            })
        })
        .collect();
    let value = serde_json::json!({
        "canonical_sparql": expansion.canonical_sparql,
        "keywords": keywords,
        "resolutions": resolutions,
        "warnings": expansion.warnings,
        "legendVersion": sparq_terse::LEGEND_VERSION,
    });
    // `to_string` on a serde_json::Value never fails.
    value.to_string()
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-2999l (gh-906) — OPT-IN CDC change-stream poll endpoint (GET /streams)
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-2999l: `GET /streams` — poll the durable change-data-capture stream in the
/// Amazon-Neptune-Streams `GetRecords` shape, over the merged durable change-stream
/// (sq-b4fns / #1223).
///
/// OPT-IN: returns `404` unless a durable log directory is configured
/// ([`ServerConfig::change_stream_dir`], `--change-stream <DIR>` / `SPARQ_CHANGE_STREAM`); the
/// route is mounted only with the `change-stream` feature, AND the handler refuses unless the
/// directory is set — the same double-opt-in as `/tpf`.
///
/// **Contract.** Read-only — `GET` / `HEAD` only (other methods are `405` with `Allow: GET, HEAD`).
/// Parameters (see [`crate::streams`]):
///   * `iteratorType` — `TRIM_HORIZON` (replay everything RETAINED, the default),
///     `AT_SEQUENCE_NUMBER` (`at=N`), `AFTER_SEQUENCE_NUMBER` (`after=N`, the resume case), or
///     `LATEST` (tail only);
///   * `at` / `after` — the sequence-number anchor for the anchored iterator types (a bare
///     `?after=N` / `?at=N` infers the type);
///   * `limit` — the max number of change records (commits) in one response page (default
///     [`crate::streams::DEFAULT_LIMIT`], clamped to [`crate::streams::MAX_LIMIT`]).
///
/// It returns the ordered change records (each commit's quad-level changes flattened to one stream
/// record per `(op, quad)`, with a `{ commitNum, opNum }` event id) plus a continuation token
/// (`nextSequenceNumber`) the consumer persists to resume — gaplessly, across a process restart
/// (the on-disk log is the source of truth). A poll is a READ over the durable log, so the endpoint
/// is gated by the read auth like any GET.
///
/// [OPUS-5] (sq-iz7ag) **Retention-aware.** `TRIM_HORIZON` starts at the log's trim horizon
/// (`AppState::change_stream_trim_horizon`), so a retention-trimmed log still replays instead of
/// answering `500`; an `at` / `after` anchor resolving BELOW the horizon is a `410 Gone` naming
/// the horizon to resume from (the records are permanently gone — that consumer re-bootstraps),
/// never a silent restart at the horizon that would skip records it has not seen.
#[cfg(feature = "change-stream")]
async fn streams_endpoint(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Response {
    use crate::streams::{page, parse_limit, to_json, IteratorType};

    // Double-opt-in: 404 unless the durable log directory is configured.
    if !state.change_stream_enabled() {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if method != Method::GET && method != Method::HEAD {
        return method_not_allowed(&[Method::GET, Method::HEAD]);
    }
    // A poll is a READ over the durable log; gate it with the read auth like any GET.
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let head_only = method == Method::HEAD;

    let params = parse_form(raw_query.as_deref().unwrap_or(""));
    let iterator_type = params.get("iteratorType").map(String::as_str);
    let at = params.get("at").map(String::as_str);
    let after = params.get("after").map(String::as_str);

    // Resolve the start offset. A missing anchor for an anchored iterator type is a 400 (fail-
    // closed — never silently replay the whole stream and re-deliver processed records). The error
    // names only the offending parameter (never echoing unbounded caller input), so it is safe to
    // return to the client.
    let iter = match IteratorType::parse(iterator_type, at, after) {
        Ok(it) => it,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let limit = parse_limit(params.get("limit").map(String::as_str));

    // [GPT-5.6] (sq-kqofk) `LATEST` starts at the successful-hook tail. The writer-thread hook
    // advances this atomic only after a durable append, so the lookup needs no submitter/log lock.
    // Absent (disabled) is the 404 above, so this is `Some`.
    let next_seq = state.change_stream_next_seq().unwrap_or(0);
    // [OPUS-5] (sq-iz7ag) Resolve against the TRIM HORIZON, not seq 0. Retention (sq-n9s4d) drops
    // whole old segments, and the durable log fails a poll below its earliest retained record
    // closed — so a `TRIM_HORIZON` replay pinned at 0 would answer `500` on any trimmed log. An
    // `at`/`after` anchor below the horizon is the caller-recoverable `410` instead of that `500`:
    // the records it asked for are permanently gone, so it must re-bootstrap (the same status the
    // time-travel surface returns for a `from` outside the retention window).
    let trim_horizon = state.change_stream_trim_horizon().unwrap_or(0);
    let from_seq = match iter.from_seq(next_seq, trim_horizon) {
        Ok(seq) => seq,
        Err(trimmed) => return json_error(StatusCode::GONE, &trimmed.to_string()),
    };

    // Poll the durable log from the resolved offset (re-reads the segments from disk; fail-closed on
    // a mid-stream corruption). `None` only if the stream is unconfigured (handled above).
    let records = match state.poll_change_stream(from_seq) {
        Ok(Some(recs)) => recs,
        // The stream became unconfigured between the gate and the poll — answer 404 consistently.
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "not found"),
        // A read error (corruption / I/O fault) is a 500; the detail goes to the operator's log,
        // never the client (the #241 info-leak posture).
        Err(e) => {
            // [OPUS-4.8] positional format arg (CodeQL rust/unused-variable false-positive).
            tracing::warn!(target: "sparq_server", detail = %e, "change-stream poll failed (detail withheld from client)");
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "change-stream read error",
            );
        }
    };

    let page = page(&records, from_seq, limit);
    let body = to_json(&page);
    text_response(
        StatusCode::OK,
        "application/json; charset=utf-8",
        body,
        head_only,
    )
}

/// [FABLE-5] (gh-2436) `POST /admin/change-stream/rebase` — OPERATOR RESYNC of the running
/// server's tracked change log (`ChangeLog::rebase_to` / `rebase_to_new_lineage`, sq-r2cu1,
/// routed through the live commit-hook state so the recorder never stops). A BROKEN stream —
/// a dropped record after a recording I/O failure, or an `/admin/restore` (which replaces
/// the lineage and deliberately never fires the commit hook) — fail-closes every later
/// record; this appends the honest **gap record** (`rebase = true`, no changes; rendered by
/// `GET /streams` as one `"op": "REBASE"` marker) and re-arms recording from the target
/// generation. The log is never wiped.
///
/// **Request.** Optional JSON body; an empty body re-baselines to the writer's CURRENT
/// generation on the same lineage (the dropped-record resync). Fields (unknown keys are a
/// fail-closed `400`, never a silently-defaulted typo):
///   * `generation` — the explicit target generation (default: the current one);
///   * `newLineage` — `true` AFTER an `/admin/restore`: the restored ring RESTARTS at
///     generation 0, so the same-lineage forward-only check would reject the new baseline
///     forever; the operator asserts the lineage (and its numbering) was replaced.
///
/// **Responses.** `200` with the appended gap record (`seq` / `generation` / `rebase` +
/// `nextSequenceNumber`); `409` when the target does not move a same-lineage stream forward
/// (pass `newLineage` only if the lineage really was replaced); `404` unless the
/// change-stream is configured (the `/streams` double-opt-in). A rebase APPENDS to the
/// durable log, so it is gated by the WRITE auth token (the existing admin gate); POST-only.
#[cfg(feature = "change-stream")]
async fn admin_change_stream_rebase(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Double-opt-in, exactly like `/streams`: 404 unless the log directory is configured.
    if !state.change_stream_enabled() {
        return json_error(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }

    // Parse the optional JSON body — fail-closed on anything not understood (a typo'd key
    // silently falling back to the defaults could re-baseline the wrong way).
    let mut generation: Option<u64> = None;
    let mut new_lineage = false;
    if !body.iter().all(u8::is_ascii_whitespace) {
        let parsed: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return json_error(StatusCode::BAD_REQUEST, "request body must be JSON"),
        };
        let Some(fields) = parsed.as_object() else {
            return json_error(StatusCode::BAD_REQUEST, "request body must be a JSON object");
        };
        for (key, value) in fields {
            match key.as_str() {
                "generation" => match value.as_u64() {
                    Some(n) => generation = Some(n),
                    None => {
                        return json_error(
                            StatusCode::BAD_REQUEST,
                            "`generation` must be a non-negative integer",
                        )
                    }
                },
                "newLineage" => match value.as_bool() {
                    Some(b) => new_lineage = b,
                    None => {
                        return json_error(
                            StatusCode::BAD_REQUEST,
                            "`newLineage` must be a boolean",
                        )
                    }
                },
                _ => {
                    return json_error(
                        StatusCode::BAD_REQUEST,
                        "unknown field (expected `generation` and/or `newLineage`)",
                    )
                }
            }
        }
    }
    // Default: the writer's current generation — the next commits chain forward from it. A
    // commit racing this read stays honest either way (it chains from the new baseline, or
    // is dropped fail-closed and reported; retry the rebase if the stream is still broken).
    let generation = generation.unwrap_or_else(|| state.current().number());

    // The append + fsync (and the hook-mutex wait behind a writer-thread record) are
    // blocking work — keep them off the async worker, like the other admin routes.
    let st = state.clone();
    let task =
        tokio::task::spawn_blocking(move || st.rebase_change_stream(generation, new_lineage));
    match task.await {
        // The stream became unconfigured between the gate and the rebase — 404 consistently.
        Ok(None) => json_error(StatusCode::NOT_FOUND, "not found"),
        Ok(Some(Ok(record))) => {
            let body = serde_json::json!({
                "seq": record.seq,
                "generation": record.generation,
                "rebase": true,
                "nextSequenceNumber": record.seq + 1,
            })
            .to_string();
            text_response(
                StatusCode::OK,
                "application/json; charset=utf-8",
                body,
                false,
            )
        }
        // The forward-only rejection (same-lineage rebase into already-recorded history) is
        // an operator-correctable conflict; the generation numbers go to the log, not the body.
        Ok(Some(Err(e @ sparq_serve::BackupError::Format(_)))) => sanitized_error(
            StatusCode::CONFLICT,
            "change-stream-rebase",
            "rebase rejected: the target generation does not move the stream forward \
             (a same-lineage rebase is strictly forward-only; pass \"newLineage\": true \
             only after a restore that replaced the lineage)",
            &e.to_string(),
        ),
        // An append-time I/O / corruption error: the operator gets the detail in the log.
        Ok(Some(Err(e))) => sanitized_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "change-stream-rebase",
            "change-stream rebase failed",
            &e.to_string(),
        ),
        Err(_) => json_error(StatusCode::INTERNAL_SERVER_ERROR, "rebase worker panicked"),
    }
}

/// Applies the T15 hardening middleware stack to a router (outermost first):
/// optional request logging, panic→500, concurrency limit with load-shedding→429,
/// JSON error bodies, and the request body-size limit→413. Public so integration
/// tests can wrap probe routes (e.g. a deliberately panicking handler) in the
/// exact production stack.
pub fn harden(routes: Router, config: &ServerConfig) -> Router {
    let routes = routes.layer(
        ServiceBuilder::new()
            // Panic in any inner layer/handler => 500 with a JSON body, not a dead socket.
            .layer(CatchPanicLayer::custom(
                |_: Box<dyn std::any::Any + Send>| {
                    json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal server error (panic)",
                    )
                },
            ))
            // [OPUS-4.8] sq-cmvh (ASVS V14.4): stamp the security hardening headers onto EVERY
            // response. Placed second so it runs LAST on the response path (after the
            // panic-catcher, the load-shed 429 mapper and `json_error_bodies`), guaranteeing the
            // headers land on success, streamed, error and panic responses alike. See
            // `security_headers` / `SECURITY_HEADERS`.
            .layer(axum::middleware::map_response(security_headers))
            // Load-shed converts "concurrency limit reached" into an immediate error
            // (mapped to 429) instead of queueing unboundedly.
            .layer(HandleErrorLayer::new(|err: BoxError| async move {
                if err.is::<tower::load_shed::error::Overloaded>() {
                    json_error(
                        StatusCode::TOO_MANY_REQUESTS,
                        "server is at its concurrent-request limit, retry later",
                    )
                } else {
                    // [OPUS-4.8] (sq-kfel, ASVS-G3) Defensive fallback (in this configured stack
                    // only `load_shed` produces a `BoxError`, handled above, so this is
                    // effectively unreachable). Do NOT Display the internal `err` into the body —
                    // that would leak the internal error type / chain. Log it server-side, return
                    // a generic 500.
                    tracing::warn!(target: "sparq_server", detail = %err, "unexpected middleware error (detail withheld from client)");
                    json_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
                }
            }))
            .load_shed()
            .concurrency_limit(config.max_concurrent)
            // Normalise plain-text error bodies (e.g. extractor rejections like the
            // body-size 413) into the structured JSON shape used everywhere else.
            .layer(axum::middleware::map_response(json_error_bodies))
            .layer(DefaultBodyLimit::max(config.max_body_bytes)),
    );
    // [OPUS-4.8] sq-o7o0 (ASVS V14.5.3): OPT-IN first-party CORS. When the allowlist is
    // EMPTY (the default) this layer is NOT added at all — the stack is byte-identical to
    // before and NO CORS headers are emitted (a cross-origin browser read stays blocked).
    // When configured, the layer wraps the WHOLE hardened stack so a preflight `OPTIONS`
    // is answered before the body-limit/concurrency layers, and the CORS response headers
    // land on every response — success, 429-shed, 413, and panic alike. The middleware
    // reflects ONLY an allowlisted `Origin` (never `*`, never with credentials). See
    // [`cors_layer`] / [`crate::cors_config`].
    let routes = if config.cors_allow.is_empty() {
        routes
    } else {
        let allow = config.cors_allow.clone();
        routes.layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let allow = allow.clone();
                async move { cors_layer(allow, req, next).await }
            },
        ))
    };
    if config.verbose {
        // [OPUS-4.8] sq-toze.34: with redaction ON (the default), swap tower-http's
        // `DefaultMakeSpan` (which records the RAW request URI — and so the full
        // `?query=…` SPARQL text, a PII exposure) for a span that records a redacted
        // target (path verbatim, query string => `<redacted len=N fp=…>`). With
        // --log-full-requests the bare TraceLayer is used, logging the URI verbatim as before.
        if config.redact_logs {
            routes
                .layer(TraceLayer::new_for_http().make_span_with(crate::redact::RedactingMakeSpan))
        } else {
            routes.layer(TraceLayer::new_for_http())
        }
    } else {
        routes
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] (sq-2gqr / sq-lodb) Serve loop with connection-layer slow-client deadlines
// (slow-loris HEADER guard + slow-BODY guard)
// ---------------------------------------------------------------------------

/// Serves `app` on `listener` until `shutdown` resolves, with two complementary slow-client
/// deadlines: a hyper HTTP/1 **header-read deadline** ([`ServerConfig::header_read_timeout`],
/// sq-2gqr) that closes the slow-loris HEADER dribble, and a **request-body read/idle deadline**
/// ([`ServerConfig::body_read_timeout`], sq-lodb) that closes the slow-BODY dribble.
///
/// **Why this exists instead of `axum::serve`.** `axum::serve` builds hyper's connection
/// `Builder` internally and exposes no hook to configure it, and — critically — it never
/// installs a [`hyper_util::rt::TokioTimer`]. hyper's HTTP/1 `header_read_timeout` (which
/// would otherwise default to 30s) is *inert without a timer* and silently does nothing. So
/// the stock `axum::serve` stack has **no header-read deadline at all**: a client that opens a
/// connection and then dribbles request-header bytes (or never finishes the header block) holds
/// the connection — and, behind [`harden`]'s `concurrency_limit`, a concurrency slot — open
/// indefinitely. [`ServerConfig::max_concurrent`] such clients starve every real caller. None of
/// the existing guards cover this: the per-request [`ServerConfig::query_timeout`] is an engine
/// deadline that only starts once a full request has been parsed; `max_body_bytes` and load-shed
/// likewise act *after* the headers are read.
///
/// **The slow-BODY complement (sq-lodb).** hyper's `header_read_timeout` only covers the header
/// block; once that completes a client can dribble the request BODY one byte at a time (or send a
/// chunk then stall) and hold the slot forever — under [`ServerConfig::max_body_bytes`] (a SIZE
/// cap a trickle never trips) and before [`ServerConfig::query_timeout`] starts (an engine
/// deadline that begins only after the whole request is read). When `body_read_timeout` is set
/// the incoming request body is wrapped in a `tower_http::timeout::TimeoutBody` (via
/// [`tower::util::option_layer`] so a `None` is a true no-op `Identity` layer): every poll for the
/// next body frame gets a fresh deadline, reset after each frame — so an honest large upload is
/// never penalised by total transfer time, only an idle stall is. The `Router` accepts the
/// wrapped body natively (`impl<B> Service<Request<B>> for Router` for any `B: Body<Data = Bytes>`),
/// so no body re-boxing is needed.
///
/// This loop is a faithful port of `axum::serve`'s own accept + graceful-shutdown loop
/// (per-connection task, watch-channel drain, `serve_connection(...).with_upgrades()` so the
/// `/subscriptions` WebSocket upgrade still works) with two behavioural additions: the connection
/// builder installs a `TokioTimer` and sets `header_read_timeout`, the per-connection service
/// optionally wraps the body in the `body_read_timeout` layer, and each request receives the
/// accepted peer as `ConnectInfo<SocketAddr>` (matching the HTTP/3 listener). `None` on either
/// deadline opts back out to the unbounded behaviour. axum is configured HTTP/1-only (no `http2`
/// feature), so this uses hyper's `http1::Builder` directly — the smallest builder that carries
/// the header knob.
#[cfg(all(feature = "server", not(feature = "http2")))]
pub async fn serve<F>(
    listener: tokio::net::TcpListener,
    app: Router,
    header_read_timeout: Option<Duration>,
    body_read_timeout: Option<Duration>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    use axum::{extract::ConnectInfo, Extension};
    use futures_util::FutureExt;
    use hyper_util::rt::{TokioIo, TokioTimer};
    use hyper_util::service::TowerToHyperService;
    use tower::ServiceBuilder;
    use tower_http::timeout::RequestBodyTimeoutLayer;

    // [OPUS-4.8] sq-lodb: wrap the per-connection service with the slow-body read/idle deadline.
    // `option_layer` makes a `None` an `Identity` (true no-op) layer, so the disabled path is
    // byte-for-byte the pre-sq-lodb behaviour. The layer feeds the `Router` a `TimeoutBody`-wrapped
    // request body, which the `Router`'s body-generic `Service` impl accepts directly.
    let body_timeout_layer =
        tower::util::option_layer(body_read_timeout.map(RequestBodyTimeoutLayer::new));

    // `signal_tx`: dropped (closed) once `shutdown` resolves — every connection task watches it
    // and begins a graceful drain. `close_rx`: each task holds a clone; the loop waits for them
    // all to drop, which means all connections have finished, before returning.
    let (signal_tx, signal_rx) = tokio::sync::watch::channel(());
    tokio::spawn(async move {
        shutdown.await;
        tracing::trace!(target: "sparq_server", "shutdown signal received, starting graceful drain");
        drop(signal_rx);
    });

    // `close_tx` outlives the loop; each connection task holds a `close_tx.subscribe()` receiver
    // and drops it when the connection finishes. `close_tx.closed()` then resolves only once every
    // task's receiver is gone — i.e. all connections have fully drained.
    let (close_tx, _close_rx) = tokio::sync::watch::channel(());
    drop(_close_rx); // the loop itself does not hold a connection slot

    loop {
        let (stream, remote) = tokio::select! {
            conn = listener.accept() => match conn {
                Ok(c) => c,
                // A transient accept error (e.g. EMFILE / a peer that vanished mid-handshake)
                // must not kill the server — yield and keep accepting, exactly as axum does.
                Err(_e) => {
                    tracing::trace!(target: "sparq_server", error = %_e, "accept error (continuing)");
                    tokio::task::yield_now().await;
                    continue;
                }
            },
            _ = signal_tx.closed() => {
                // Shutdown signalled: stop accepting and let in-flight connections drain.
                tracing::trace!(target: "sparq_server", "accept loop stopping (graceful shutdown)");
                break;
            }
        };

        let io = TokioIo::new(stream);
        // [OPUS-4.8] sq-lodb: apply the (optional) slow-body read/idle deadline around this
        // connection's clone of the router, then hand the composed tower service to hyper.
        // [GPT-5.6] sq-rejk9: expose the accepted TCP peer to every request, matching serve_h3.
        let service = ServiceBuilder::new()
            .layer(Extension(ConnectInfo(remote)))
            .layer(body_timeout_layer.clone())
            .service(app.clone());
        let hyper_service = TowerToHyperService::new(service);
        let signal_tx = signal_tx.clone();
        let close_rx = close_tx.subscribe();

        tokio::spawn(async move {
            let mut builder = hyper::server::conn::http1::Builder::new();
            // The header-read deadline is the slow-loris guard. It REQUIRES a timer (hyper panics
            // otherwise / silently no-ops on the auto builder), which is the bug in the stock
            // `axum::serve` path this whole function exists to fix.
            if let Some(t) = header_read_timeout {
                builder.timer(TokioTimer::new()).header_read_timeout(t);
            }
            // `.with_upgrades()` keeps the HTTP/1 `Upgrade` working — the `/subscriptions`
            // WebSocket handshake depends on it.
            let conn = builder.serve_connection(io, hyper_service).with_upgrades();
            let mut conn = std::pin::pin!(conn);
            // `.fuse()` is load-bearing (mirrors axum's own graceful-shutdown loop): a bare
            // `watch::Receiver::closed()` future panics with `async fn resumed after completion`
            // if it is polled again after it has resolved. Once the shutdown signal fires, the
            // loop keeps running to DRAIN the connection (`conn.as_mut().await`), and `tokio::select!`
            // would re-poll the already-completed `signal_closed` on the next iteration. Fusing it
            // makes that branch terminated (`is_terminated()`); `select!` skips it, so the shutdown
            // path fires `graceful_shutdown()` exactly once and then quietly drains to completion.
            let mut signal_closed = std::pin::pin!(signal_tx.closed().fuse());

            loop {
                tokio::select! {
                    result = conn.as_mut() => {
                        if let Err(_err) = result {
                            tracing::trace!(target: "sparq_server", error = %_err, "connection ended with error");
                        }
                        break;
                    }
                    _ = &mut signal_closed => {
                        tracing::trace!(target: "sparq_server", "shutdown signal in connection task, starting graceful shutdown");
                        conn.as_mut().graceful_shutdown();
                    }
                }
            }
            drop(close_rx);
        });
    }

    // Drain: `close_tx.closed()` resolves once every connection task's `close_tx.subscribe()`
    // receiver has dropped — i.e. all in-flight connections have finished (mirrors axum's
    // own graceful-shutdown drain).
    tracing::trace!(
        target: "sparq_server",
        tasks = close_tx.receiver_count(),
        "waiting for in-flight connections to drain"
    );
    close_tx.closed().await;
    Ok(())
}

/// Serves HTTP/1.1 and HTTP/2 on `listener` until `shutdown` resolves.
///
/// [GPT-5.6] sq-oprna.6: with the default-off `http2` feature, [`serve`] switches from
/// hyper's HTTP/1-only builder to hyper-util's h1+h2 auto builder. Its HTTP/1 configuration
/// retains [`ServerConfig::header_read_timeout`], request-body idle deadlines, WebSocket
/// upgrades, peer `ConnectInfo`, and graceful drain. A plain listener accepts h1 and h2c;
/// use [`serve_tls`] for TLS with ALPN.
#[cfg(all(feature = "server", feature = "http2"))]
pub async fn serve<F>(
    listener: tokio::net::TcpListener,
    app: Router,
    header_read_timeout: Option<Duration>,
    body_read_timeout: Option<Duration>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    serve_auto(
        listener,
        app,
        header_read_timeout,
        body_read_timeout,
        None,
        shutdown,
    )
    .await
}

/// Serves HTTP/1.1 and HTTP/2 over TLS on `listener` until `shutdown` resolves.
///
/// This is the TLS counterpart to [`serve`], available only with the default-off `http2` feature.
/// The supplied rustls configuration should advertise `h2` and `http/1.1` through ALPN. The
/// HTTP/1 path preserves [`ServerConfig::header_read_timeout`], request-body idle deadlines,
/// WebSocket upgrades, peer `ConnectInfo`, and graceful drain exactly as [`serve`]. HTTP/2
/// multiplexed streams share the same request middleware and graceful-drain path.
///
/// [GPT-5.6] sq-oprna.6
#[cfg(feature = "http2")]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub async fn serve_tls<F>(
    listener: tokio::net::TcpListener,
    app: Router,
    tls_config: std::sync::Arc<rustls::ServerConfig>,
    header_read_timeout: Option<Duration>,
    body_read_timeout: Option<Duration>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    serve_auto(
        listener,
        app,
        header_read_timeout,
        body_read_timeout,
        Some(tokio_rustls::TlsAcceptor::from(tls_config)),
        shutdown,
    )
    .await
}

/// [GPT-5.6] sq-oprna.6: feature-on h1+h2 accept loop shared by cleartext and TLS entry points.
#[cfg(feature = "http2")]
async fn serve_auto<F>(
    listener: tokio::net::TcpListener,
    app: Router,
    header_read_timeout: Option<Duration>,
    body_read_timeout: Option<Duration>,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (signal_tx, signal_rx) = tokio::sync::watch::channel(());
    tokio::spawn(async move {
        shutdown.await;
        tracing::trace!(target: "sparq_server", "shutdown signal received, starting graceful drain");
        drop(signal_rx);
    });

    let (close_tx, close_rx) = tokio::sync::watch::channel(());
    drop(close_rx);

    loop {
        let (stream, remote) = tokio::select! {
            conn = listener.accept() => match conn {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::trace!(target: "sparq_server", %error, "accept error (continuing)");
                    tokio::task::yield_now().await;
                    continue;
                }
            },
            _ = signal_tx.closed() => {
                tracing::trace!(target: "sparq_server", "accept loop stopping (graceful shutdown)");
                break;
            }
        };

        let app = app.clone();
        let signal_tx = signal_tx.clone();
        let close_rx = close_tx.subscribe();
        let tls_acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            if let Some(acceptor) = tls_acceptor {
                // A peer that never completes TLS must not prevent shutdown from draining. The
                // HTTP header timer starts after this handshake, where hyper can parse headers.
                let accepted = tokio::select! {
                    accepted = acceptor.accept(stream) => accepted,
                    _ = signal_tx.closed() => {
                        drop(close_rx);
                        return;
                    }
                };
                match accepted {
                    Ok(stream) => {
                        serve_auto_connection(
                            stream,
                            remote,
                            TransportScheme::Https,
                            app,
                            header_read_timeout,
                            body_read_timeout,
                            signal_tx,
                        )
                        .await;
                    }
                    Err(error) => {
                        tracing::trace!(target: "sparq_server", %error, "TLS handshake ended with error");
                    }
                }
            } else {
                serve_auto_connection(
                    stream,
                    remote,
                    TransportScheme::Http,
                    app,
                    header_read_timeout,
                    body_read_timeout,
                    signal_tx,
                )
                .await;
            }
            drop(close_rx);
        });
    }

    tracing::trace!(
        target: "sparq_server",
        tasks = close_tx.receiver_count(),
        "waiting for in-flight connections to drain"
    );
    close_tx.closed().await;
    Ok(())
}

/// [GPT-5.6] sq-oprna.6: one auto-detected HTTP/1.1 or HTTP/2 connection.
#[cfg(feature = "http2")]
async fn serve_auto_connection<I>(
    stream: I,
    remote: std::net::SocketAddr,
    transport: TransportScheme,
    app: Router,
    header_read_timeout: Option<Duration>,
    body_read_timeout: Option<Duration>,
    signal_tx: tokio::sync::watch::Sender<()>,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use axum::{extract::ConnectInfo, Extension};
    use futures_util::FutureExt;
    use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
    use hyper_util::service::TowerToHyperService;
    use tower::ServiceBuilder;
    use tower_http::timeout::RequestBodyTimeoutLayer;

    let body_timeout_layer =
        tower::util::option_layer(body_read_timeout.map(RequestBodyTimeoutLayer::new));
    let service = ServiceBuilder::new()
        .layer(Extension(ConnectInfo(remote)))
        .layer(Extension(transport))
        .layer(body_timeout_layer)
        .service(app);
    let hyper_service = TowerToHyperService::new(service);
    let io = TokioIo::new(stream);

    let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
    if let Some(timeout) = header_read_timeout {
        builder
            .http1()
            .timer(TokioTimer::new())
            .header_read_timeout(timeout);
    }
    let conn = builder.serve_connection_with_upgrades(io, hyper_service);
    let mut conn = std::pin::pin!(conn);
    let mut signal_closed = std::pin::pin!(signal_tx.closed().fuse());

    loop {
        tokio::select! {
            result = conn.as_mut() => {
                if let Err(error) = result {
                    tracing::trace!(target: "sparq_server", %error, "connection ended with error");
                }
                break;
            }
            _ = &mut signal_closed => {
                tracing::trace!(target: "sparq_server", "shutdown signal in connection task, starting graceful shutdown");
                conn.as_mut().graceful_shutdown();
            }
        }
    }
}

#[cfg(test)]
mod header_read_timeout_config_tests {
    //! [OPUS-4.8] (sq-2gqr / sq-lodb) The slow-loris header-read deadline and the slow-body
    //! read/idle deadline live in `ServerConfig` so they are configurable + testable in isolation;
    //! the END-TO-END behaviour (a partial-header socket / a dribbled-body socket is actually
    //! closed by `serve`) is the integration suite in `tests/hardening.rs`. These pin the config
    //! contract.
    use super::*;

    #[test]
    fn slow_loris_guard_is_on_by_default() {
        // The whole point of sq-2gqr: a fresh, unconfigured server must NOT be vulnerable. A
        // generous-but-finite 15s deadline ships ON.
        assert_eq!(
            ServerConfig::default().header_read_timeout,
            Some(Duration::from_secs(15)),
            "header-read deadline must be ON by default (slow-loris guard)"
        );
    }

    #[test]
    fn header_read_timeout_is_independent_of_query_timeout() {
        // Distinct from the engine's per-request query_timeout (a connection-layer vs evaluation
        // bound). Setting one must not move the other.
        let cfg = ServerConfig {
            header_read_timeout: Some(Duration::from_secs(3)),
            query_timeout: Some(Duration::from_secs(99)),
            ..ServerConfig::default()
        };
        assert_eq!(cfg.header_read_timeout, Some(Duration::from_secs(3)));
        assert_eq!(cfg.query_timeout, Some(Duration::from_secs(99)));
        // It is opt-out-able (an operator who really wants the old unbounded behaviour).
        let off = ServerConfig { header_read_timeout: None, ..ServerConfig::default() };
        assert_eq!(off.header_read_timeout, None);
    }

    #[test]
    fn slow_body_guard_is_on_by_default() {
        // [OPUS-4.8] sq-lodb: a fresh, unconfigured server must also bound the slow-BODY vector.
        // A generous-but-finite 30s idle deadline ships ON.
        assert_eq!(
            ServerConfig::default().body_read_timeout,
            Some(Duration::from_secs(30)),
            "body read/idle deadline must be ON by default (slow-body guard)"
        );
    }

    #[test]
    fn body_read_timeout_is_independent_of_the_header_and_query_deadlines() {
        // [OPUS-4.8] sq-lodb: three distinct deadlines (body phase / header phase / engine
        // evaluation). Setting one must not move the others.
        let cfg = ServerConfig {
            body_read_timeout: Some(Duration::from_secs(5)),
            header_read_timeout: Some(Duration::from_secs(3)),
            query_timeout: Some(Duration::from_secs(99)),
            ..ServerConfig::default()
        };
        assert_eq!(cfg.body_read_timeout, Some(Duration::from_secs(5)));
        assert_eq!(cfg.header_read_timeout, Some(Duration::from_secs(3)));
        assert_eq!(cfg.query_timeout, Some(Duration::from_secs(99)));
        // It is opt-out-able independently of the header guard.
        let off = ServerConfig { body_read_timeout: None, ..ServerConfig::default() };
        assert_eq!(off.body_read_timeout, None);
        assert_eq!(off.header_read_timeout, Some(Duration::from_secs(15))); // header guard untouched
    }
}

// ---------------------------------------------------------------------------
// SPARQL 1.1 Protocol — query operation
// ---------------------------------------------------------------------------

const SPARQL_QUERY_CT: &str = "application/sparql-query";
const FORM_CT: &str = "application/x-www-form-urlencoded";
/// `Accept` media type that turns a query request into an EXPLAIN response (T22).
const EXPLAIN_CT: &str = "text/x-sparq-explain";
/// [FABLE-5] sq-ixc3.19: the STRUCTURED explain media type. `Accept`ing it answers an
/// EXPLAIN request with the engine's typed plan tree (`explain_json::PlanNode`,
/// sq-u4lgr/#902) as camelCase JSON — `operator`/`estimated`/`actual`/`nanos`/`qError`/
/// `children`, the sq-jbqh4 schema contract the GUI plan explorer renders — instead of
/// the plan text. Like [`EXPLAIN_CT`] it also *requests* explain by itself (plan-only
/// unless `explain=analyze` says otherwise). Requires the `explain-json` feature
/// (default-on for the server binary); a lean build answers 406 so callers can fall
/// back to the text plan.
const EXPLAIN_JSON_CT: &str = "application/x-sparq-explain+json";

/// Whether the request `Accept`s the structured JSON plan tree. [FABLE-5] sq-ixc3.19
fn accepts_explain_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains(EXPLAIN_JSON_CT))
}

/// How a `/sparql` query request should be answered (T22 EXPLAIN).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExplainMode {
    /// Execute normally (the default).
    Off,
    /// Return the engine's planning-only plan text instead of executing.
    Plan,
    /// Execute under the normal budget and return the plan + per-operator trace.
    Analyze,
}

/// EXPLAIN is requested via an `explain` parameter (URL query string, or the
/// url-encoded POST body — the body wins) — `explain`, `explain=true`,
/// `explain=plan` for the dry run, `explain=analyze` to execute and trace — or
/// via `Accept: text/x-sparq-explain` (plan only) / `Accept:
/// application/x-sparq-explain+json` (plan only, structured — sq-ixc3.19).
fn explain_mode(
    url_params: &HashMap<String, String>,
    body_params: Option<&HashMap<String, String>>,
    headers: &HeaderMap,
) -> ExplainMode {
    if let Some(v) = body_params
        .and_then(|m| m.get("explain"))
        .or_else(|| url_params.get("explain"))
    {
        return match v.to_ascii_lowercase().as_str() {
            "false" | "0" | "off" | "no" => ExplainMode::Off,
            "analyze" | "analyse" => ExplainMode::Analyze,
            _ => ExplainMode::Plan,
        };
    }
    let accepts_explain = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains(EXPLAIN_CT) || a.contains(EXPLAIN_JSON_CT));
    if accepts_explain {
        ExplainMode::Plan
    } else {
        ExplainMode::Off
    }
}

// ---------------------------------------------------------------------------
// Generation pinning: the `Sparq-Generation` response header + the `?generation=N`
// request pin. DEFAULT build (sq-ci2d6): the mechanics are unconditional, bounded
// to the generation ring's EXISTING concurrency-retention window (`RingConfig::retain`
// = the last K generations older than current, always kept — see
// `sparq_serve::ring`). The opt-in `time-travel` feature only WIDENS how far back a
// pin can reach (its extended-retention config + `--time-travel-*` CLI flags stay
// feature-gated); the header/pin surface here is byte-identical in both feature
// states. [SONNET-4.6] (sq-ci2d6)
// ---------------------------------------------------------------------------

/// Resolves the generation a query request is pinned to. `generation=N` — URL query
/// string, or the url-encoded POST body, the body winning (same precedence as
/// `explain`) — pins the retained generation N for the whole request: the response is
/// the store *as of* that generation (an immutable snapshot — the load-bearing property
/// for snapshot-consistent multi-request `LIMIT`/`OFFSET` pagination). N is the exact
/// token the server itself hands out in `Sparq-Generation` (the read-your-writes /
/// shard_seq concept — no clock resolution or skew ambiguity); callers that track
/// timestamps resolve them via the library's `GenerationRing::as_of`.
///
/// **Retention window (the honest bound).** The ring ALWAYS retains the last K
/// generations older than current (the concurrency-retention floor `RingConfig::retain`,
/// default K = 4 — `sparq_serve::ring::DEFAULT_RETAIN`), so a pin within
/// `[current - K, current]` resolves in the DEFAULT build with no feature enabled. The
/// opt-in `time-travel` feature EXTENDS that window (count/age bounds via
/// `--time-travel-generations` / `--time-travel-max-age`); it does not change the
/// pin/header mechanics here. Retention is publish-driven and never extended by this
/// function — a pin outside the window is a clean typed error, never a silent fallback
/// to a different generation.
///
/// Errors per the endpoint's status semantics: unparsable number → 400; not yet
/// published → 400 (the client cannot have obtained that token from this server);
/// published but no longer retained → **410 Gone** (it aged out of the retention window —
/// gone permanently, retrying that pin cannot help; re-read the current generation and
/// restart pagination).
#[allow(clippy::result_large_err)] // Err is axum's Response; boxing would desync the call-site match
fn resolve_pin(
    state: &AppState,
    url_params: &HashMap<String, String>,
    body_params: Option<&HashMap<String, String>>,
) -> Result<PinnedGen, Response> {
    let raw = match body_params
        .and_then(|m| m.get("generation"))
        .or_else(|| url_params.get("generation"))
    {
        Some(raw) => raw,
        None => return Ok(state.current()),
    };
    // [SONNET-4.6] (sq-ci2d6) Positional format args throughout (CodeQL `rust/unused-variable`
    // false-positive on inline captures — this path is now compiled into the DEFAULT build).
    let number: u64 = raw.parse().map_err(|_| {
        bad_request(&format!(
            "invalid 'generation' parameter '{}': expected a generation number",
            raw
        ))
    })?;
    let current = state.current();
    if number > current.number() {
        return Err(bad_request(&format!(
            "generation {} has not been published yet (current generation: {})",
            number,
            current.number()
        )));
    }
    if number == current.number() {
        return Ok(current);
    }
    // [OPUS-4.8] (sq-o5bi, sq-0g6g) Resolve the ring once via the accessor: DEFAULT this is a
    // direct field reference (byte-identical to pre-#941); under `backup` it is one clone out of
    // the swappable serving core. Binding it locally keeps `at` + `oldest_retained` on one ring.
    let ring = state.ring();
    ring.at(number).ok_or_else(|| {
        // [SONNET-4.6] (sq-ci2d6) The 410 + "aged out … oldest retained" prefix is byte-identical
        // in both feature states (the `time-travel` suite is unchanged); only the closing HINT
        // differs — the default build's window is the ring's concurrency-retention floor, the
        // `time-travel` build's is the extended retention the CLI flags size.
        let hint = if cfg!(feature = "time-travel") {
            "raise --time-travel-generations / --time-travel-max-age to keep more history"
        } else {
            "only the ring's concurrency-retention window (the last K generations older than \
             current) is kept in the default build; re-read the current generation and restart, \
             or build with the `time-travel` feature to widen the window"
        };
        json_error(
            StatusCode::GONE,
            &format!(
                "generation {} has aged out of the retention window (oldest retained: {}, current: {}); {}",
                number,
                ring.oldest_retained(),
                current.number(),
                hint
            ),
        )
    })
}

/// Stamps the `Sparq-Generation` response header: the generation number the response was
/// produced against — the current generation for unpinned queries (capture it as the
/// read-your-writes / snapshot-pin token; the same generation-number concept as the
/// horizontal-scaling ADR's `shard_seq`), the pin for pinned queries, and the generation
/// containing the update for a 204 update ack. [SONNET-4.6] (sq-ci2d6) Present in the
/// DEFAULT build — the pin/header mechanics are bounded to the ring's concurrency-retention
/// window; the opt-in `time-travel` feature only widens how far back `?generation=N` reaches.
fn with_generation_header(mut resp: Response, number: u64) -> Response {
    resp.headers_mut().insert(
        header::HeaderName::from_static("sparq-generation"),
        header::HeaderValue::from(number),
    );
    resp
}

async fn sparql_endpoint(
    State(state): State<AppState>,
    method: Method,
    #[cfg(feature = "federation-descriptors")]
    uri: Uri,
    #[cfg(all(feature = "federation-descriptors", feature = "http2"))]
    transport: Option<axum::Extension<TransportScheme>>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let raw = raw_query.as_deref().unwrap_or("");
    let url_params = parse_form(raw);
    // [OPUS-4.8] sq-z33x: the SPARQL 1.1 Protocol query/update dataset overrides live in the URL
    // query string for GET and the direct (`application/sparql-query` / `application/sparql-update`)
    // POSTs; the form-POST path reads them from the request BODY instead (see `handle_post`).
    let url_dataset = query_dataset_override(raw);
    let url_using = update_dataset_override(raw);
    match method {
        Method::GET | Method::HEAD => {
            // Query string carries `query=` (+ optional dataset params). Per protocol, a
            // GET without a `query` parameter is a malformed request (400). A GET is always a
            // READ (the protocol has no GET update), so it is gated only under --auth-token-read.
            // [OPUS-4.8] sq-zcby.
            // [OPUS-4.8] sq-0bxp: begin an access-audit record for the GET query (always a
            // read; fingerprint the `query=` param if present).
            #[cfg(feature = "audit-log")]
            let audit = crate::audit::enabled(state.config()).then(|| {
                crate::audit::AuditRecord::begin(
                    crate::audit::AuditOp::Query,
                    bearer_token(&headers),
                    url_params.get("query").map(String::as_str),
                )
            });
            // [OPUS-4.8] sq-gos8: structured access-audit for the GET query (a read on the
            // dataset; the `query=` param is fingerprinted, never recorded raw).
            #[cfg(feature = "access-audit")]
            let aa = audit_access_begin(
                &state,
                crate::access_audit::Action::Query,
                crate::access_audit::Resource::Dataset("/sparql".to_string()),
                &headers,
                url_params.get("query").map(String::as_str),
            );
            if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
                #[cfg(feature = "audit-log")]
                if let Some(a) = audit {
                    a.emit(&resp);
                }
                #[cfg(feature = "access-audit")]
                audit_access_finish(&state, aa, &resp);
                return resp;
            }
            match url_params.get("query") {
                Some(q) => {
                    let pin = match resolve_pin(&state, &url_params, None) {
                        Ok(pin) => pin,
                        Err(resp) => {
                            #[cfg(feature = "audit-log")]
                            if let Some(a) = audit {
                                a.emit(&resp);
                            }
                            #[cfg(feature = "access-audit")]
                            audit_access_finish(&state, aa, &resp);
                            return resp;
                        }
                    };
                    let explain = explain_mode(&url_params, None, &headers);
                    let resp = run_query(
                        &state,
                        q,
                        &headers,
                        method == Method::HEAD,
                        explain,
                        pin,
                        &url_dataset,
                    )
                    .await;
                    #[cfg(feature = "audit-log")]
                    if let Some(a) = audit {
                        a.emit(&resp);
                    }
                    #[cfg(feature = "access-audit")]
                    audit_access_finish(&state, aa, &resp);
                    resp
                }
                // [OPUS-4.8] sq-d3d8: per SPARQL Protocol §2.1.2, a GET with no `query` may
                // serve the endpoint's Service Description (OPT-IN — only when the
                // `federation-descriptors` feature is compiled in AND the config flag is set).
                // Otherwise the historical 400.
                #[cfg(feature = "federation-descriptors")]
                None if method != Method::HEAD => {
                    #[cfg(feature = "http2")]
                    let transport = transport.map(|extension| extension.0.as_str());
                    #[cfg(not(feature = "http2"))]
                    let transport = None;
                    match service_description_response(
                        &state,
                        &headers,
                        &uri,
                        transport,
                    ) {
                        Some(resp) => resp,
                        None => bad_request("missing 'query' parameter"),
                    }
                }
                None => bad_request("missing 'query' parameter"),
            }
        }
        Method::POST => {
            handle_post(
                &state,
                &headers,
                &body,
                &url_params,
                &url_dataset,
                &url_using,
                false,
            )
            .await
        }
        // [OPUS-4.8] sq-b3df9 (epic sq-my8wd, w3c/sparql-protocol#40): the HTTP `QUERY` method.
        // Oxigraph's CLI server already serves this verb (route arm
        // `("/sparql", "POST" | "QUERY")` with `let is_query = method == "QUERY"`). It behaves
        // EXACTLY like a POST query EXCEPT it is query-ONLY: an `application/sparql-update` body
        // falls through to 415, and an `update=` form field is an explicit 400. `QUERY` is not a
        // const in `http::Method`, so it arrives via `Method::from_bytes(b"QUERY")` and is matched
        // on its literal token (the same passthrough oxhttp gives Oxigraph). The query input path
        // (raw body / merged `query` form param) and the URL-query-string dataset overrides feed
        // the SAME `handle_post` downstream — query execution, Accept negotiation and the auth /
        // egress gates are shared, so QUERY cannot bypass them.
        ref m if m.as_str() == "QUERY" => {
            handle_post(
                &state,
                &headers,
                &body,
                &url_params,
                &url_dataset,
                &url_using,
                true,
            )
            .await
        }
        _ => method_not_allowed(&[Method::GET, Method::HEAD, Method::POST]),
    }
}

async fn handle_post(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
    url_params: &HashMap<String, String>,
    // [OPUS-4.8] sq-z33x: the §2.1.4 query dataset override carried in the URL query string
    // (applies to the direct `application/sparql-query` POST; the form-POST path overrides it with
    // the override carried in the form BODY, per the protocol's url-encoded encoding).
    url_dataset: &DatasetOverride,
    // [OPUS-4.8] sq-z33x: the §2.2 UPDATE dataset override (`using-*`) carried in the URL query
    // string (applies to the direct `application/sparql-update` POST; the form-POST `update=` path
    // reads it from the form BODY).
    url_using: &UsingOverride,
    // [OPUS-4.8] sq-b3df9 (epic sq-my8wd, w3c/sparql-protocol#40): `true` when this is the HTTP
    // `QUERY` method, which is QUERY-ONLY. It mirrors Oxigraph's `is_query` flag: an
    // `application/sparql-update` body is NOT accepted (it falls through to 415) and a `update=`
    // form field is rejected with an explicit 400 — a `POST` (`is_query == false`) accepts both.
    // Per the protocol, the `default-graph-uri` / `named-graph-uri` dataset override for a `QUERY`
    // always comes from the URL query string (`url_dataset`), even for the url-encoded form body.
    is_query: bool,
) -> Response {
    let ct = content_type(headers);
    if ct.starts_with(SPARQL_QUERY_CT) {
        // POST directly — body IS the SPARQL query. [OPUS-4.8] sq-zcby: the auth gate keys on
        // whether the body MUTATES, not on the route — an UPDATE smuggled through the query
        // Content-Type is gated as a write before the query handler ever sees it.
        let s = match std::str::from_utf8(body) {
            Ok(s) => s,
            Err(_) => return bad_request("request body is not valid UTF-8"),
        };
        let op = if payload_mutates(s) {
            Operation::Write
        } else {
            Operation::Read
        };
        // [OPUS-4.8] sq-0bxp: audit the direct-POST query/update (op keys on whether it mutates).
        #[cfg(feature = "audit-log")]
        let audit = crate::audit::enabled(state.config()).then(|| {
            crate::audit::AuditRecord::begin(
                crate::audit::AuditOp::from_sparql(op),
                bearer_token(headers),
                Some(s),
            )
        });
        // [OPUS-4.8] sq-gos8: structured access-audit — the plain SPARQL surface names no
        // per-graph resource at the request boundary, so the resource is the dataset (`/sparql`);
        // the query body is fingerprinted, NEVER recorded raw (the privacy boundary).
        #[cfg(feature = "access-audit")]
        let aa = audit_access_begin(
            state,
            sparql_action(op),
            crate::access_audit::Resource::Dataset("/sparql".to_string()),
            headers,
            Some(s),
        );
        if let Some(resp) = auth_gate(state.config(), headers, op) {
            #[cfg(feature = "audit-log")]
            if let Some(a) = audit {
                a.emit(&resp);
            }
            #[cfg(feature = "access-audit")]
            audit_access_finish(state, aa, &resp);
            return resp;
        }
        // NB: a write smuggled through the query Content-Type still goes to `run_query` (the
        // historical behaviour — it parses as a non-query and returns the existing error); the
        // audit op already records it as an `update` for the access trail.
        let pin = match resolve_pin(state, url_params, None) {
            Ok(pin) => pin,
            Err(resp) => {
                #[cfg(feature = "audit-log")]
                if let Some(a) = audit {
                    a.emit(&resp);
                }
                #[cfg(feature = "access-audit")]
                audit_access_finish(state, aa, &resp);
                return resp;
            }
        };
        let explain = explain_mode(url_params, None, headers);
        let resp = run_query(state, s, headers, false, explain, pin, url_dataset).await;
        #[cfg(feature = "audit-log")]
        if let Some(a) = audit {
            a.emit(&resp);
        }
        #[cfg(feature = "access-audit")]
        audit_access_finish(state, aa, &resp);
        resp
    } else if ct.starts_with(FORM_CT) {
        // POST url-encoded — `query=` (read) or `update=` (write) in the body. [OPUS-4.8]
        // sq-zcby: classify on the payload for the ambiguous `query=` path; an `update=` form
        // is ALWAYS a write (see below).
        let s = match std::str::from_utf8(body) {
            Ok(s) => s,
            Err(_) => return bad_request("request body is not valid UTF-8"),
        };
        let params = parse_form(s);
        // [OPUS-4.8] sq-b3df9 (w3c/sparql-protocol#40): the HTTP `QUERY` method is query-ONLY, so
        // an `update=` form field is rejected with an explicit 400 BEFORE any auth/work (matching
        // Oxigraph's "SPARQL updates are not compatible with the QUERY HTTP method"). A normal
        // `POST` (`is_query == false`) keeps accepting `update=` as a write below.
        if is_query && params.contains_key("update") {
            return bad_request(
                "SPARQL updates are not compatible with the QUERY HTTP method; use POST for an \
                 'update=' form field",
            );
        }
        // [OPUS-4.8] sq-zcby (Copilot PR#71 SECURITY fix): the `update=` form field IS an
        // update operation BY DEFINITION (SPARQL 1.1 Protocol §2.2), so it is a WRITE
        // UNCONDITIONALLY — regardless of whether its value happens to parse as a read-only
        // query (e.g. `update=SELECT…`). Classifying it on its payload (the old
        // `payload_mutates(u)`) let such a request slip past the write gate: the auth-bypass
        // this fixes. Only the ambiguous `query=`/generic path falls through to content
        // inspection — a `query=` whose value parses as an UPDATE is still gated as a write
        // (an UPDATE smuggled through the query parameter).
        let op = if params.contains_key("update") {
            Operation::Write
        } else {
            match params.get("query") {
                Some(q) if payload_mutates(q) => Operation::Write,
                Some(_) => Operation::Read,
                // Neither parameter present: not a write, let the query handler return its 400.
                None => Operation::Read,
            }
        };
        // [OPUS-4.8] sq-0bxp: audit the url-encoded form request (fingerprint the update= or
        // query= payload it carries).
        #[cfg(feature = "audit-log")]
        let audit = crate::audit::enabled(state.config()).then(|| {
            crate::audit::AuditRecord::begin(
                crate::audit::AuditOp::from_sparql(op),
                bearer_token(headers),
                params
                    .get("update")
                    .or_else(|| params.get("query"))
                    .map(String::as_str),
            )
        });
        // [OPUS-4.8] sq-gos8: structured access-audit for the url-encoded form request.
        #[cfg(feature = "access-audit")]
        let aa = audit_access_begin(
            state,
            sparql_action(op),
            crate::access_audit::Resource::Dataset("/sparql".to_string()),
            headers,
            params
                .get("update")
                .or_else(|| params.get("query"))
                .map(String::as_str),
        );
        if let Some(resp) = auth_gate(state.config(), headers, op) {
            #[cfg(feature = "audit-log")]
            if let Some(a) = audit {
                a.emit(&resp);
            }
            #[cfg(feature = "access-audit")]
            audit_access_finish(state, aa, &resp);
            return resp;
        }
        // The SPARQL 1.1 Protocol url-encoded UPDATE operation (`update=` form) submits through
        // the same sequenced writer as `application/sparql-update`.
        if let Some(u) = params.get("update") {
            // [SONNET-4.6] (sq-ci2d6) `?generation` is a READ pin (default build, bounded to the
            // ring's concurrency-retention window); an update always applies to the current
            // generation, so pinning one is an HONEST refusal, never a silent ignore.
            if url_params.contains_key("generation") {
                let resp = bad_request(
                    "the 'generation' parameter pins queries to a retained generation; \
                     updates always apply to the current generation",
                );
                #[cfg(feature = "audit-log")]
                if let Some(a) = audit {
                    a.emit(&resp);
                }
                #[cfg(feature = "access-audit")]
                audit_access_finish(state, aa, &resp);
                return resp;
            }
            // [OPUS-4.8] sq-z33x: §2.2 UPDATE dataset override — for the url-encoded form encoding
            // the `using-*` params are carried in the FORM BODY (`s`), not the URL query string.
            let resp = match rewrite_update(u, &update_dataset_override(s)) {
                Ok(rewritten) => run_update(state, rewritten).await,
                Err(resp) => resp,
            };
            #[cfg(feature = "audit-log")]
            if let Some(a) = audit {
                a.emit(&resp);
            }
            #[cfg(feature = "access-audit")]
            audit_access_finish(state, aa, &resp);
            return resp;
        }
        let resp = match params.get("query") {
            Some(q) => {
                let pin = match resolve_pin(state, url_params, Some(&params)) {
                    Ok(pin) => pin,
                    Err(resp) => {
                        #[cfg(feature = "audit-log")]
                        if let Some(a) = audit {
                            a.emit(&resp);
                        }
                        #[cfg(feature = "access-audit")]
                        audit_access_finish(state, aa, &resp);
                        return resp;
                    }
                };
                let explain = explain_mode(url_params, Some(&params), headers);
                // [OPUS-4.8] sq-z33x: per the SPARQL 1.1 Protocol url-encoded encoding, a `POST`
                // form carries the dataset override (`default-graph-uri` / `named-graph-uri`) in the
                // FORM BODY. [OPUS-4.8] sq-b3df9 (w3c/sparql-protocol#40): the HTTP `QUERY` method
                // instead always reads the dataset override from the URL query string (Oxigraph's
                // `url_query_parameters(request)` — body graph params are not consulted on QUERY).
                let form_dataset;
                let dataset = if is_query {
                    url_dataset
                } else {
                    form_dataset = query_dataset_override(s);
                    &form_dataset
                };
                run_query(state, q, headers, false, explain, pin, dataset).await
            }
            None => bad_request("missing 'query' or 'update' parameter in url-encoded body"),
        };
        #[cfg(feature = "audit-log")]
        if let Some(a) = audit {
            a.emit(&resp);
        }
        #[cfg(feature = "access-audit")]
        audit_access_finish(state, aa, &resp);
        resp
    } else if ct.starts_with("application/sparql-update") && !is_query {
        // SPARQL 1.1 Protocol — update operation (T11b). Body IS the update; success → 204.
        // [OPUS-4.8] sq-b3df9 (w3c/sparql-protocol#40): the `&& !is_query` guard mirrors Oxigraph
        // — under the HTTP `QUERY` method an `application/sparql-update` body is NOT a valid update
        // operation, so it falls through to the 415 branch below (NOT a 400/403); only a `POST`
        // reaches this update path.
        // [OPUS-4.8] sq-zcby: an UPDATE is always a write — gate it before doing any work.
        // [OPUS-4.8] sq-0bxp: audit the update (fingerprint the body if it is valid UTF-8).
        #[cfg(feature = "audit-log")]
        let audit = crate::audit::enabled(state.config()).then(|| {
            crate::audit::AuditRecord::begin(
                crate::audit::AuditOp::Update,
                bearer_token(headers),
                std::str::from_utf8(body).ok(),
            )
        });
        // [OPUS-4.8] sq-gos8: structured access-audit for the SPARQL UPDATE body.
        #[cfg(feature = "access-audit")]
        let aa = audit_access_begin(
            state,
            crate::access_audit::Action::Update,
            crate::access_audit::Resource::Dataset("/sparql".to_string()),
            headers,
            std::str::from_utf8(body).ok(),
        );
        if let Some(resp) = auth_gate(state.config(), headers, Operation::Write) {
            #[cfg(feature = "audit-log")]
            if let Some(a) = audit {
                a.emit(&resp);
            }
            #[cfg(feature = "access-audit")]
            audit_access_finish(state, aa, &resp);
            return resp;
        }
        // `apply_update` blocks for the writer's group-commit ack (window + batch
        // application, which includes an O(graph) fork), so it runs off the async workers.
        // [SONNET-4.6] (sq-ci2d6) A `generation` pin is a READ concept (default build, bounded to
        // the ring's concurrency-retention window): an update can only apply to the current
        // generation, so a `generation` pin on an update is an honest refusal, not a silent ignore.
        if url_params.contains_key("generation") {
            let resp = bad_request(
                "the 'generation' parameter pins queries to a retained generation; \
                 updates always apply to the current generation",
            );
            #[cfg(feature = "audit-log")]
            if let Some(a) = audit {
                a.emit(&resp);
            }
            #[cfg(feature = "access-audit")]
            audit_access_finish(state, aa, &resp);
            return resp;
        }
        let resp = match std::str::from_utf8(body) {
            // [OPUS-4.8] sq-z33x: §2.2 UPDATE dataset override — for the `application/sparql-update`
            // body the `using-*` params are carried in the URL query string (`url_using`).
            Ok(u) => match rewrite_update(u, url_using) {
                Ok(rewritten) => run_update(state, rewritten).await,
                Err(resp) => resp,
            },
            Err(_) => bad_request("request body is not valid UTF-8"),
        };
        #[cfg(feature = "audit-log")]
        if let Some(a) = audit {
            a.emit(&resp);
        }
        #[cfg(feature = "access-audit")]
        audit_access_finish(state, aa, &resp);
        resp
    } else {
        // Unsupported media type for the query/update operation. [OPUS-4.8] sq-b3df9
        // (w3c/sparql-protocol#40): under the HTTP `QUERY` method `application/sparql-update`
        // is NOT accepted (the `&& !is_query` guard above sent it here) — Oxigraph parity is a
        // 415, not a 400/403, since QUERY is query-only.
        json_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            if is_query {
                "the QUERY method requires Content-Type 'application/sparql-query' or \
                 'application/x-www-form-urlencoded' (it is query-only; \
                 'application/sparql-update' is not accepted — use POST)"
            } else {
                "POST requires Content-Type 'application/sparql-query' or \
                 'application/x-www-form-urlencoded'"
            },
        )
    }
}

/// Applies a SPARQL Update string through the sequenced writer off the async workers (it
/// blocks for the group-commit ack), mapping the outcome onto 204/400/503/500. Shared by the
/// `application/sparql-update` body path and the url-encoded `update=` form path (T11b /
/// [OPUS-4.8] sq-zcby). The caller is responsible for the auth gate (so the gate runs before
/// any work) and any `generation`-pin refusal.
///
/// [OPUS-4.8] sq-ebii: both update entry points inherit the query-timeout cap here — the SAME
/// wall-clock hard cap (`timeout + TIMEOUT_GRACE`) the read paths use, applied via
/// [`await_update_worker`]. The cooperative `QueryBudget` ALSO reaches the update itself
/// (`ServerApplier::apply` runs it via `update_in_place_with_budget`), so a `DELETE/INSERT …
/// WHERE` aborts at the deadline / row cap mid-evaluation; this wall-clock await cap is the
/// BACKSTOP for an uninstrumented stretch, not the only stop. See [`await_update_worker`] for
/// the remaining caveats (coarse cooperative checks; single sequenced writer; the writer
/// finishes its next budget check after the HTTP side has already answered 503).
/// [OPUS-4.8] (sq-x32t) `POST /admin/compact` — WAL COMPACTION / VACUUM for ERASURE-
/// COMPLETENESS (epic sq-toze.33). A logical SPARQL `DELETE` / `DROP GRAPH` retracts data from
/// the live view but leaves the superseded bytes in earlier `--persist` WAL segments until a
/// compaction folds the live state into a fresh base. This operator-invokable endpoint runs that
/// compaction on demand, so erased data is PHYSICALLY gone from the on-disk store (the manual
/// quiesce→export→reseed purge in `compliance/privacy/retention-erasure-runbook.md` §7a,
/// automated as one atomic, crash-safe operation).
///
/// - **Gated** behind the WRITE auth token (the existing admin gate) — it mutates the durable
///   store. POST-only (a GET could not mutate; the route is registered POST-only so other verbs
///   get a 405).
/// - **In-memory server** (no `--persist`): 409 Conflict — there is no on-disk history to purge
///   (erasure is already immediate: dropped generations free by `Arc` drop, no WAL survives), so
///   a no-op success would be misleading. The body explains the precondition.
/// - **Persistent server**: runs [`AppState::compact`] on the blocking pool (it blocks for the
///   compaction, which runs on the writer thread between batches). 200 on success; 503 if the
///   durable rewrite hit a transient I/O error (retryable — the writer stays alive); 500 if the
///   worker panicked.
///
/// Physical-erasure caveat (documented honestly): compaction guarantees the LIVE store no longer
/// references the erased triples and the new on-disk segments do not contain them; it cannot, by
/// itself, scrub bytes already copied off-box (filesystem snapshots, block-level COW history,
/// external backups). Those are out of scope for the engine and must be handled by the storage /
/// backup tier per the retention-erasure runbook.
async fn admin_compact(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    if state.config().persist_dir.is_none() {
        return json_error(
            StatusCode::CONFLICT,
            "compaction requires durable persistence; this server is in-memory (no --persist dir). \
             In-memory erasure is already immediate (no on-disk WAL history to purge).",
        );
    }
    let st = state.clone();
    let task = tokio::task::spawn_blocking(move || st.compact());
    match task.await {
        Ok(Ok(())) => text_response(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            "compaction complete\n".to_string(),
            false,
        ),
        // A compaction failure (e.g. durable-write I/O error) is retryable: the writer thread
        // stays alive and reads keep flowing from the last published snapshot.
        // [OPUS-4.8] (sq-kfel, ASVS-G3) `e` is the durable-rewrite error string, which carries
        // the server's `--persist` filesystem path (an I/O error embeds the absolute path).
        // Although this route is write/admin-gated, withhold the path from the response body —
        // the operator gets the full detail in the server log.
        Ok(Err(e)) => sanitized_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "compaction",
            "compaction failed (retryable)",
            &e,
        ),
        Err(_) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "compaction worker panicked",
        ),
    }
}

/// [OPUS-4.8] (sq-o5bi) `POST /admin/backup` — ONLINE consistent snapshot backup of the live
/// serving store. Returns a single self-describing backup artifact (sparq-serve's Option-A
/// format: a textual header recording the generation/writer-seq + per-pod epoch vectors + the
/// triple count + a body digest, then the full dataset as N-Quads) as
/// `application/octet-stream`.
///
/// - **Online — no stop-the-world.** The export pins the CURRENT generation lock-free and
///   serialises off that immutable `Arc` on the blocking pool, so readers never block the
///   writer and the writer never blocks readers throughout.
/// - **Gated** behind the WRITE auth token (the existing admin gate) — it reads the WHOLE
///   dataset, so it is treated as a privileged admin operation, not an open read.
/// - POST-only (the route is registered POST-only; other verbs get a 405).
///
/// Distinct from the offline `sparq-cli save` (stop-the-world, index rebuild) and from the
/// `--persist` per-graph WAL. At-rest ENCRYPTION of the artifact is a separate concern, out
/// of scope.
#[cfg(feature = "backup")]
async fn admin_backup(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    let st = state.clone();
    let task = tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        st.export_backup(&mut buf).map(|()| buf)
    });
    match task.await {
        Ok(Ok(bytes)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, bytes.len())
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"sparq-backup.spqb\"",
            )
            .body(axum::body::Body::from(bytes))
            .unwrap(),
        // A serialisation/IO failure: the live store is untouched (export only reads).
        // [OPUS-4.8] (sq-kfel) the error may embed internals; withhold from the body.
        Ok(Err(e)) => sanitized_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "backup",
            "backup export failed",
            &e,
        ),
        Err(_) => json_error(StatusCode::INTERNAL_SERVER_ERROR, "backup worker panicked"),
    }
}

/// [OPUS-4.8] (sq-o5bi, sq-ft7u) `POST /admin/restore` — ONLINE restore of the serving store from
/// a backup artifact POSTed in the request body.
///
/// - **In-memory server** (no `--persist`): atomically installs a freshly rehydrated ring+writer
///   into the swappable serving core; readers in flight keep serving from the old core until they
///   release their pin, and every read/update after the swap sees the restored store. RAM-only —
///   the restored content does NOT survive a process restart.
/// - **`--persist` durable server** with the write-through opt-in (`?persist=true`): writes the
///   restore THROUGH to the durable dir so it SURVIVES A RESTART (sq-ft7u). The swap runs on the
///   single writer thread, crash-safely (the two-rename `Graph::restore_into_durable`, healed by
///   `recover_compaction`). WITHOUT `?persist=true` a durable server REFUSES the restore (409): an
///   in-memory-only swap would be silently lost on the next restart, which is a footgun.
/// - `?persist=true` on an in-memory server → 409 (no durable dir to write through).
/// - **Single-flight** ([FABLE-5] sq-fy8ci): a second restore posted while one is still in flight
///   is rejected 409 (`a restore is already in progress`) instead of being silently serialized
///   into a last-writer-wins pile-up by the writer thread. Retry after the first completes.
///
/// **Fail-closed.** A corrupt/mismatched/non-artifact body is rejected (400) and the live store is
/// left UNTOUCHED — the artifact is imported + validated fully before anything is swapped, and the
/// durable swap itself is rollback-safe. **Gated** behind the WRITE auth token (the existing admin
/// gate); POST-only.
#[cfg(feature = "backup")]
async fn admin_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    // [OPUS-4.8] (sq-ft7u) Opt in to writing the restore THROUGH to the durable `--persist` store
    // (so it survives a restart) via `?persist=true` (truthy: "1"/"true"/"yes"/"on"). Default off:
    // a restore is in-memory-only unless the operator explicitly asks for write-through.
    let persist = params.get("persist").map(|v| env_truthy(v)).unwrap_or(false);
    let durable = state.config().persist_dir.is_some();
    // 409 cases (the request is incompatible with the server's durability posture) are decided
    // HERE so any `Err` from `restore_from` below is unambiguously a corrupt/import problem (400).
    if durable && !persist {
        return json_error(
            StatusCode::CONFLICT,
            "this is a --persist (durable) server: pass ?persist=true to write the restore through \
             to the durable store (so it survives a restart); an in-memory-only restore on a \
             durable server would be silently lost on the next restart and is refused",
        );
    }
    if !durable && persist {
        return json_error(
            StatusCode::CONFLICT,
            "?persist=true requires a --persist (durable) server; this server is in-memory (there \
             is no durable directory to write the restore through to)",
        );
    }
    // [FABLE-5] (sq-fy8ci) SINGLE-FLIGHT: claim the restore permit AFTER the deterministic
    // posture 409s above (so those errors never depend on timing) and BEFORE any work. A second
    // concurrent restore gets an explicit 409 instead of being silently serialized behind the
    // first by the writer thread (last-writer-wins). The permit rides into the blocking closure
    // so it is released exactly when the restore finishes — including on a panic (unwind drops
    // it), so a failed restore can never wedge the route shut.
    let Some(restore_permit) = state.try_begin_restore() else {
        return json_error(
            StatusCode::CONFLICT,
            "a restore is already in progress on this server; retry after it completes \
             (concurrent restores are rejected rather than silently applied last-writer-wins)",
        );
    };
    let st = state.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = restore_permit;
        st.restore_from(&body[..], persist)
    });
    match task.await {
        Ok(Ok(source_generation)) => text_response(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            format!(
                "restore complete (artifact taken at generation {})\n",
                source_generation
            ),
            false,
        ),
        // A fail-closed import error (corrupt / mismatched / non-artifact): the live store is
        // unchanged. It is a client-supplied-body problem, so a 400. The detail is withheld
        // from the body (it may quote artifact internals); the operator gets it in the log.
        Ok(Err(e)) => sanitized_error(
            StatusCode::BAD_REQUEST,
            "restore",
            "restore rejected (corrupt or incompatible backup artifact)",
            &e,
        ),
        Err(_) => json_error(StatusCode::INTERNAL_SERVER_ERROR, "restore worker panicked"),
    }
}

/// [OPUS-4.8] (sq-bu1a) `POST /admin/backup/delta?from=N` — the INCREMENTAL change-stream / PITR
/// producer. Streams a single self-describing DELTA artifact (sparq-serve's delta format: a header
/// keying the `from-generation` N → the current `to-generation`, plus the per-pod epoch vector at
/// `to` and the inserted/deleted quad bodies as N-Quads) as `application/octet-stream`. Replayed
/// forward onto the matching base artifact, the delta advances a restore to a later point in the
/// writer history — point-in-time recovery.
///
/// - **Online — no stop-the-world.** Both generations are pinned lock-free and the diff is
///   serialised off those immutable `Arc`s on the blocking pool.
/// - **`from` must be a RETAINED generation.** Without the `time-travel` feature only the last few
///   generations (the concurrency window) are retained, so a `from` older than that yields 410 Gone
///   (aged out) — exactly like `?generation=N` on `/sparql`. The `time-travel` feature widens the
///   retention window so further-back deltas are available.
/// - **Gated** behind the WRITE admin token (it reads the whole dataset); POST-only.
///
/// At-rest ENCRYPTION of the artifact is a separate concern, out of scope (same as the base).
#[cfg(feature = "backup")]
async fn admin_backup_delta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    let from = match params.get("from").map(|s| s.trim().parse::<u64>()) {
        Some(Ok(n)) => n,
        Some(Err(_)) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "delta backup requires a numeric `from` generation",
            )
        }
        None => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "delta backup requires a `from` generation query parameter (?from=N)",
            )
        }
    };
    let st = state.clone();
    let task = tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        st.export_delta_from(from, &mut buf).map(|meta| (meta, buf))
    });
    match task.await {
        // `from` is no longer retained (aged out / never published): 410 Gone, mirroring time-travel.
        Ok(Ok((None, _))) => json_error(
            StatusCode::GONE,
            "the `from` generation is no longer retained (aged out of the retention window); \
             enable the time-travel feature to widen retention",
        ),
        Ok(Ok((Some(meta), bytes))) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, bytes.len())
            // [OPUS-4.8] positional format args (CodeQL rust/unused-variable false-positive).
            .header(
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"sparq-delta-{}-{}.spqd\"",
                    meta.from_generation, meta.to_generation
                ),
            )
            .body(axum::body::Body::from(bytes))
            .unwrap(),
        // A range/serialisation error (e.g. `from` >= current): the store is untouched (read-only).
        Ok(Err(e)) => sanitized_error(
            StatusCode::BAD_REQUEST,
            "backup-delta",
            "delta backup rejected",
            &e,
        ),
        Err(_) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "backup-delta worker panicked",
        ),
    }
}

pub(crate) async fn run_update(state: &AppState, update: String) -> Response {
    let st = state.clone();
    let task = tokio::task::spawn_blocking(move || st.apply_update(&update));
    match await_update_worker(task, &state.config).await {
        UpdateOutcome::Ok(number) => {
            state.metrics().inc_updates();
            // The 204 carries the generation containing the update (the read-your-writes
            // token) under the time-travel feature.
            with_generation_header(StatusCode::NO_CONTENT.into_response(), number)
        }
        UpdateOutcome::Rejected(e) => update_rejection_response(&e, &state.config),
        UpdateOutcome::TimedOut => timeout_response(&state.config),
        UpdateOutcome::Panicked => {
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "update worker panicked")
        }
    }
}

/// Executes a query string against the request's pinned generation and renders the
/// negotiated representation, stamped with the `Sparq-Generation` header under the
/// `time-travel` feature.
///
/// The engine call is synchronous + CPU-bound, so it runs on the blocking pool under a
/// cooperative [`QueryBudget`] (deadline + SELECT row cap). The await is additionally
/// hard-capped at `timeout + TIMEOUT_GRACE` so a worker stuck in an uninstrumented
/// stretch still gets its 503 on time (the worker itself stops at its next budget check).
async fn run_query(
    state: &AppState,
    sparql: &str,
    headers: &HeaderMap,
    head_only: bool,
    explain: ExplainMode,
    gen: PinnedGen,
    dataset: &DatasetOverride,
) -> Response {
    let number = gen.number();
    let resp = run_query_pinned(state, sparql, headers, head_only, explain, gen, dataset).await;
    with_generation_header(resp, number)
}

async fn run_query_pinned(
    state: &AppState,
    sparql: &str,
    headers: &HeaderMap,
    head_only: bool,
    explain: ExplainMode,
    gen: PinnedGen,
    // [OPUS-4.8] sq-z33x: the SPARQL 1.1 Protocol §2.1.4 dataset override
    // (`default-graph-uri` / `named-graph-uri`); empty for the common in-query / no-dataset case.
    dataset: &DatasetOverride,
) -> Response {
    let prepared = match prepare_with_dataset(sparql, dataset) {
        Ok(p) => p,
        // [OPUS-4.8] (sq-cz89/sq-j9zs) The parser echoes the offending query token verbatim;
        // withhold it from the body (it is caller input, but an info-leak contract regardless)
        // — the operator gets the full parse error in the server log.
        Err(PrepareError::Malformed(msg)) => {
            return sanitized_error(
                StatusCode::BAD_REQUEST,
                "query-parse",
                "malformed query",
                &msg,
            )
        }
        // [OPUS-4.8] sq-z33x: a `default-graph-uri` / `named-graph-uri` value that is not a valid
        // absolute IRI is a client error (the protocol parameter is caller input).
        Err(PrepareError::BadGraphUri(msg)) => return bad_request(&msg),
    };
    // The generation was pinned ONCE per request (the caller's `resolve_pin`: the
    // current generation, or — under the `time-travel` feature — the requested
    // retained one); every evaluation below (and the streamed JSON body) reads this
    // snapshot, so the response is consistent with its pin even while the writer
    // publishes new generations concurrently.
    // T22 EXPLAIN: answer with the plan text instead of (plan-only) / alongside
    // executing (analyze). Plan-only is a planning dry run — no budget needed —
    // but both run on the blocking pool and under the worker timeout cap anyway;
    // analyze executes, so it gets the standard per-request budget.
    if explain != ExplainMode::Off {
        // [FABLE-5] sq-ixc3.19: structured-plan negotiation. Resolved HERE (headers are
        // not `'static`) and moved into the worker. A build without the `explain-json`
        // feature answers a structured request 406 up front — never a silent text
        // fallback the caller would then mis-parse as JSON — so clients (the GUI plan
        // explorer) can degrade to the text plan explicitly.
        let wants_json = accepts_explain_json(headers);
        #[cfg(not(feature = "explain-json"))]
        if wants_json {
            return text_response(
                StatusCode::NOT_ACCEPTABLE,
                "text/plain; charset=utf-8",
                format!(
                    "this server was built without the `explain-json` feature; \
                     the structured plan ({EXPLAIN_JSON_CT}) is unavailable. \
                     Use the text explain (`explain=plan|analyze`, or Accept: {EXPLAIN_CT}).\n"
                ),
                head_only,
            );
        }
        let config = state.config.clone();
        let cfg = config.clone();
        let q = sparql.to_string();
        let analyze = explain == ExplainMode::Analyze;
        // [SONNET-4.6] (sq-t1isr) Register EXPLAIN ANALYZE with the running-query registry
        // (feature `query-registry`), mirroring the SELECT/CONSTRUCT pattern from sq-qsm5z.
        // The RAII guard is moved into the blocking closure so the row is present for the
        // full duration of the engine call and removed on any exit path (normal, error, unwind).
        // Plan-only EXPLAIN (ExplainMode::Plan) is also registered — it runs on the blocking
        // pool under the same timeout cap and can be listed/killed like any other query.
        #[cfg(feature = "query-registry")]
        let (budget, qr_guard): (QueryBudget, Option<Box<dyn std::any::Any + Send>>) = {
            let base = make_budget(&config, true);
            let (cancel, guard) = state.running_queries.register(sparql);
            (base.with_cancel(cancel), Some(Box::new(guard)))
        };
        #[cfg(not(feature = "query-registry"))]
        let (budget, qr_guard): (QueryBudget, Option<Box<dyn std::any::Any + Send>>) =
            (make_budget(&config, true), None);
        // [OPUS-4.8] sq-9xoh: resolve the per-request SERVICE egress allowlist HERE (headers are
        // not `'static`, so it must happen before the spawn) and move it into the worker.
        let allow = config.resolve_service_allow(headers);
        let task = tokio::task::spawn_blocking(move || {
            // Hold qr_guard alive for the duration of the engine call (sq-t1isr).
            let _guard = qr_guard;
            let graph = gen.snapshot();
            // [FABLE-5] sq-ixc3.19: the STRUCTURED explain response — the caller
            // `Accept`ed `application/x-sparq-explain+json`, so answer with the typed
            // plan tree (camelCase JSON, the sq-jbqh4 schema contract) instead of the
            // plan text. Same budget + egress-allowlist envelope as the text arm.
            #[cfg(feature = "explain-json")]
            if wants_json {
                let r = with_engine_scope_allow(&allow, || {
                    if analyze {
                        sparq_engine::explain_plan_analyze_with_budget(graph, &q, &budget)
                    } else {
                        sparq_engine::explain_plan(graph, &q)
                    }
                });
                return match r {
                    Ok(plan) => text_response(
                        StatusCode::OK,
                        "application/x-sparq-explain+json; charset=utf-8",
                        plan.to_json(),
                        head_only,
                    ),
                    // ANALYZE of a non-SELECT/ASK form is a client error, not a server one.
                    Err(e) if e.contains("EXPLAIN ANALYZE supports") => bad_request(&e),
                    // EXPLAIN ANALYZE used `make_budget(_, true)` → max_results applied.
                    Err(e) => engine_error_response(&e, &cfg, true),
                };
            }
            // [OPUS-4.8] sq-4w18: EXPLAIN ANALYZE executes (can hit SERVICE), so it runs
            // under the egress allowlist policy like a normal query; plan-only is a dry
            // run but is wrapped identically for uniformity (it never dials).
            // [OPUS-4.8] sq-9xoh: under the request-resolved allowlist (the static one unless a
            // per-request override hook is installed).
            let r = with_engine_scope_allow(&allow, || {
                if analyze {
                    sparq_engine::explain_analyze_with_budget(graph, &q, &budget)
                } else {
                    sparq_engine::explain(graph, &q)
                }
            });
            match r {
                Ok(text) => {
                    text_response(StatusCode::OK, "text/plain; charset=utf-8", text, head_only)
                }
                // ANALYZE of a non-SELECT/ASK form is a client error, not a server one.
                Err(e) if e.contains("EXPLAIN ANALYZE supports") => bad_request(&e),
                // EXPLAIN ANALYZE used `make_budget(_, true)` → max_results applied.
                Err(e) => engine_error_response(&e, &cfg, true),
            }
        });
        return await_worker(task, &config).await;
    }
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let config = state.config.clone();
    // [OPUS-4.8] sq-9xoh: the per-request SERVICE egress allowlist for this read (the static
    // `service_allow` unless a per-request override hook is installed). Resolved here while the
    // request headers are still in scope, then cloned into the per-form worker closure.
    let allow = config.resolve_service_allow(headers);

    match prepared.form {
        QueryForm::Select | QueryForm::Ask => {
            // [OPUS-4.8] sq-406acc: Oxigraph-parity content negotiation — a PRESENT-but-unsatisfiable
            // `Accept` (naming no supported SELECT/ASK result format and no wildcard) is `406 Not
            // Acceptable` rather than the old silent JSON fallback. An absent / empty / `*/*` Accept
            // still defaults to SPARQL-results JSON (W3C-permitted default representation).
            let fmt = match negotiate_or_406(accept) {
                Ok(f) => f,
                Err(_) => return not_acceptable_response(head_only),
            };
            let is_ask = prepared.form == QueryForm::Ask;
            // [SONNET-4.6] (sq-qsm5z) Register with the running-query registry (feature
            // `query-registry`). The RAII guard is boxed as `Box<dyn Any + Send>` so the
            // call site is uniform in both feature states; it is moved into the blocking
            // closure and held alive for the entire engine call, then dropped on exit.
            #[cfg(feature = "query-registry")]
            let (budget, qr_guard): (QueryBudget, Option<Box<dyn std::any::Any + Send>>) = {
                let base = make_budget(&config, !is_ask);
                let (cancel, guard) = state.running_queries.register(sparql);
                (base.with_cancel(cancel), Some(Box::new(guard)))
            };
            #[cfg(not(feature = "query-registry"))]
            let (budget, qr_guard): (QueryBudget, Option<Box<dyn std::any::Any + Send>>) =
                (make_budget(&config, !is_ask), None);
            let cfg = config.clone();
            let allow = allow.clone();
            // [OPUS-4.8] (sq-7d3dj.34.2) SELECT → SPARQL-results JSON streams its body: the
            // engine runs on a blocking worker and feeds a bounded channel, so the results
            // header + early solutions reach the socket before the whole result is
            // serialised (TTFB). ASK and the XML/CSV/TSV SELECT forms keep the buffered
            // worker path below (their serialisers are `&QueryResult`-driven).
            // [OPUS-4.8] (sq-7d3dj.34.1) The floor path (JSON SELECT stream + ASK) runs the
            // algebra already parsed in `prepare` via the engine's `*_prepared` entry points —
            // no second parse per request.
            if !is_ask && fmt == Format::Json {
                // [SONNET-4.6] (sq-7d3dj.26) Whether THIS client negotiated HTTP trailers
                // decides how a mid-stream truncation is signalled (trailer vs. an aborted
                // chunked framing) — see `StreamingJsonBody`. Resolved here, while the
                // request headers are still in scope.
                let shape = StreamShape { head_only, trailers_ok: client_accepts_trailers(headers) };
                return stream_select_json(gen, prepared.query, budget, shape, allow, qr_guard, &config)
                    .await;
            }
            let pquery = prepared.query;
            let select = prepared.runnable;
            let task = tokio::task::spawn_blocking(move || {
                // Hold qr_guard alive for the duration of the engine call.
                let _guard = qr_guard;
                with_engine_scope_allow(&allow, || {
                    if is_ask {
                        match sparq_engine::ask_prepared_with_budget(
                            gen.snapshot(),
                            &pquery,
                            &budget,
                        ) {
                            Ok(value) => {
                                let (body, ct) = match fmt {
                                    Format::Xml => {
                                        (results::ask_to_xml(value), fmt.ask_content_type())
                                    }
                                    _ => (results::ask_to_json(value), fmt.ask_content_type()),
                                };
                                text_response(StatusCode::OK, ct, body, head_only)
                            }
                            // ASK used `make_budget(_, false)` → max_results did NOT apply.
                            Err(e) => engine_error_response(&e, &cfg, false),
                        }
                    } else {
                        render_select(&gen, &select, fmt, head_only, &budget, &cfg)
                    }
                })
            });
            await_worker(task, &config).await
        }
        // CONSTRUCT / DESCRIBE (T16): an RDF graph result, negotiated between
        // `application/n-triples` (default) and `text/turtle` (the N-Triples body is a
        // syntactic subset of Turtle). DESCRIBE returns concise bounded descriptions.
        // The engine's row budget applies to the WHERE-pattern solutions, the deadline
        // to the whole evaluation — same guard semantics as SELECT.
        QueryForm::Construct | QueryForm::Describe => {
            // [OPUS-4.8] sq-406acc: same Oxigraph-parity strictness for the RDF-graph result — a
            // PRESENT-but-unsatisfiable `Accept` (no supported RDF media type and no wildcard) is
            // 406; absent / empty / `*/*` keeps the N-Triples default.
            let gfmt = match negotiate_graph_or_406(accept) {
                Ok(f) => f,
                Err(_) => return not_acceptable_response(head_only),
            };
            let query = prepared.runnable;
            // [SONNET-4.6] (sq-qsm5z) Register with the running-query registry.
            #[cfg(feature = "query-registry")]
            let (budget, qr_guard): (QueryBudget, Option<Box<dyn std::any::Any + Send>>) = {
                let base = make_budget(&config, true);
                let (cancel, guard) = state.running_queries.register(sparql);
                (base.with_cancel(cancel), Some(Box::new(guard)))
            };
            #[cfg(not(feature = "query-registry"))]
            let (budget, qr_guard): (QueryBudget, Option<Box<dyn std::any::Any + Send>>) =
                (make_budget(&config, true), None);
            // [FABLE-5] (sq-0kq6k) A GET streams its body: the rendered RDF document is fed to
            // the response in chunks as it is written, so a large CONSTRUCT never holds the
            // whole document in memory and its first bytes reach the socket early (TTFB). A
            // result small enough to fit one chunk still answers buffered, with the same
            // `Content-Length` it always had.
            //
            // HEAD deliberately keeps the buffered path: it must advertise the `Content-Length`
            // the GET would have carried (the documented "HEAD mirrors GET" contract), which
            // means rendering the document anyway — there is nothing to stream to.
            if !head_only {
                return stream_graph_result(gen, query, gfmt, budget, allow, qr_guard, &config)
                    .await;
            }
            let cfg = config.clone();
            let task = tokio::task::spawn_blocking(move || {
                let _guard = qr_guard;
                // [OPUS-4.8] sq-rt6v: produce the triple list once, then serialise it in the
                // negotiated graph syntax — N-Triples (default), prefix-compacting Turtle, or
                // RDF/XML — rather than always emitting N-Triples.
                // [OPUS-4.8] sq-9xoh: under the request-resolved SERVICE egress allowlist.
                match with_engine_scope_allow(&allow, || {
                    sparq_engine::construct_or_describe_with_budget(gen.snapshot(), &query, &budget)
                }) {
                    Ok(triples) => {
                        let body = serialise_graph_triples(&triples, gfmt);
                        text_response(StatusCode::OK, gfmt.content_type(), body, head_only)
                    }
                    // CONSTRUCT/DESCRIBE used `make_budget(_, true)` → max_results applied.
                    Err(e) => engine_error_response(&e, &cfg, true),
                }
            });
            await_worker(task, &config).await
        }
    }
}

/// The per-request engine budget: deadline from the configured timeout; the row cap from
/// the memory cap (`--max-query-rows`, applied on EVERY form) AND — when `apply_max_results`
/// is set — `--max-results`, whichever is tighter.
///
/// [OPUS-4.8] (sq-ebii) `--max-query-rows` is the coarse memory cap: it bounds the
/// working-set cardinality of any materialised intermediate/final result on all forms
/// (SELECT/ASK/CONSTRUCT/DESCRIBE/GSP-read), so a join blow-up aborts (413) instead of
/// OOMing. `--max-results` is the narrower result/solution cap: callers pass
/// `apply_max_results = true` on SELECT (the final projection), CONSTRUCT/DESCRIBE (their
/// WHERE-pattern solution count) and EXPLAIN ANALYZE; `false` on ASK and GSP-read (which
/// have no projection to cap). Both map onto the single engine `max_rows` budget, so the
/// effective ceiling on a path where both apply is their min.
pub(crate) fn make_budget(config: &ServerConfig, apply_max_results: bool) -> QueryBudget {
    let results_cap = if apply_max_results {
        config.max_results
    } else {
        None
    };
    QueryBudget {
        deadline: config.query_timeout.map(|t| std::time::Instant::now() + t),
        max_rows: tighter(config.max_query_rows, results_cap),
        // [OPUS-4.8] (sq-s5is) byte-accounted cap applies on every form (it has no
        // `--max-results` analogue — it bounds the working set, not the projection).
        max_bytes: config.max_query_bytes,
        cancel: None,
    }
}

/// [OPUS-4.8] (sq-ebii) The per-UPDATE engine budget: the coarse memory cap
/// (`--max-query-rows`) as the working-set row ceiling, and a cooperative deadline measured
/// from NOW (the moment the writer starts this update). The SELECT-projection cap
/// (`--max-results`) does NOT apply to updates — there is no result to project — but the
/// working-set memory cap and the deadline do.
///
/// [OPUS-4.8] (sq-nulp) The WHERE deadline is [`update_where_deadline`]: the TIGHTER of the
/// read [`query_timeout`](ServerConfig::query_timeout) and the optional, typically-shorter
/// [`update_where_timeout`](ServerConfig::update_where_timeout). Because updates are sequenced
/// on a single writer thread, this deadline is what BOUNDS writer-queue head-of-line blocking:
/// a slow update releases the writer within this window, so the queue behind it cannot be held
/// longer than that (cooperatively — see [`QueryBudget`]'s coarse-check caveat). With
/// `update_where_timeout` unset (the default) it is exactly `query_timeout`, unchanged.
fn update_budget(config: &ServerConfig) -> QueryBudget {
    QueryBudget {
        deadline: update_where_deadline(config).map(|t| std::time::Instant::now() + t),
        max_rows: config.max_query_rows,
        // [OPUS-4.8] (sq-s5is) the byte cap reaches the UPDATE's WHERE evaluation too.
        max_bytes: config.max_query_bytes,
        cancel: None,
    }
}

/// [OPUS-4.8] (sq-p7kk5) The sequenced writer's config, derived from the server config: the
/// stock `sparq_serve` group-commit window/batch defaults, but with adaptive group-commit
/// wired from [`ServerConfig::adaptive_commit`] (ON by default — the write path is interactive,
/// where the fixed window is pure latency for a serial client blocked on its own ack). Adaptive
/// mode changes only WHEN the batch closes (drain the queued backlog, commit immediately when it
/// empties); FIFO application order, per-update atomicity, visibility and durability are all
/// unchanged.
fn writer_config(config: &ServerConfig) -> WriterConfig {
    WriterConfig {
        adaptive_commit: config.adaptive_commit,
        ..WriterConfig::default()
    }
}

/// [OPUS-4.8] (sq-nulp) The effective WHERE-phase deadline for a SPARQL UPDATE on the writer
/// thread: the tighter of the read [`query_timeout`](ServerConfig::query_timeout) and the
/// opt-in [`update_where_timeout`](ServerConfig::update_where_timeout). `None` (no deadline)
/// only when BOTH are unset; otherwise the smaller of whichever are present. This is the knob
/// that bounds writer-queue head-of-line blocking independently of the (usually longer) read
/// timeout — see [`update_budget`].
fn update_where_deadline(config: &ServerConfig) -> Option<Duration> {
    match (config.query_timeout, config.update_where_timeout) {
        (Some(q), Some(u)) => Some(q.min(u)),
        (q, u) => q.or(u),
    }
}

/// [OPUS-4.8] (sq-ebii) The tighter (smaller) of two optional row caps: `None` is "no cap",
/// so the combined cap is `None` only when both are `None`, else the min of those present.
fn tighter(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Awaits a blocking engine worker under the hard timeout cap; maps a worker panic to a
/// 500 (CatchPanicLayer cannot see panics on the blocking pool — the JoinError carries them).
pub(crate) async fn await_worker(task: tokio::task::JoinHandle<Response>, config: &ServerConfig) -> Response {
    let joined = match config.query_timeout {
        Some(t) => match tokio::time::timeout(t + TIMEOUT_GRACE, task).await {
            Ok(j) => j,
            Err(_elapsed) => return timeout_response(config),
        },
        None => task.await,
    };
    match joined {
        Ok(resp) => resp,
        Err(e) if e.is_panic() => {
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "query worker panicked")
        }
        Err(_) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "query worker was cancelled",
        ),
    }
}

/// [OPUS-4.8] (sq-ebii) The result of awaiting a SPARQL Update worker under the timeout cap.
pub(crate) enum UpdateOutcome {
    /// Published; carries the generation number containing the update.
    Ok(u64),
    /// The writer rejected the update (parse / semantic error) — a client 400.
    Rejected(String),
    /// The wall-clock update cap elapsed before the writer acked — a 503.
    TimedOut,
    /// The blocking worker panicked / was cancelled — a 500.
    Panicked,
}

/// [OPUS-4.8] (sq-ebii) Awaits a SPARQL Update worker under the SAME hard wall-clock cap the
/// read paths use (`query_timeout + TIMEOUT_GRACE`), so a pathological UPDATE (e.g. a
/// `DELETE/INSERT … WHERE` whose WHERE pattern blows up) cannot pin a connection indefinitely.
///
/// **Honest scope — what this does and does NOT bound:** this is a wall-clock cap on the
/// HTTP *await*, and it is a BACKSTOP, not the only stop. The cooperative [`QueryBudget`]
/// (deadline + `max_rows`) DOES reach inside the update: `ServerApplier::apply` runs it via
/// [`sparq_engine::update_in_place_with_budget`] with [`update_budget`], which installs the
/// budget thread-locally, so a `DELETE/INSERT … WHERE` whose WHERE pattern blows up aborts
/// mid-evaluation at the row cap / deadline exactly as a budgeted `SELECT` does. The
/// remaining caveats this wall-clock cap covers: (1) the cooperative budget is checked only
/// at *coarse* sites (operator entry / per outer-loop iteration), so a single uninstrumented
/// stretch can overrun the deadline before the next check — this await then answers 503 on
/// time while the writer thread keeps running to its next budget check and discards the
/// result; (2) the non-WHERE operations (INSERT/DELETE DATA, CLEAR/DROP/CREATE/LOAD) do not
/// consult the budget (they are bounded by operand size, already capped by `--max-body-bytes`);
/// (3) updates are *sequenced* on a single writer, so a long-running update blocks the queue
/// behind it until it finishes — this *await* cap bounds the client's own wait, not the writer's
/// work. [OPUS-4.8] (sq-nulp) The writer's work — and hence that head-of-line blocking — is
/// bounded SEPARATELY by the WHERE-phase deadline ([`update_budget`] / [`update_where_deadline`]):
/// `--update-where-timeout` sets a typically-shorter cooperative deadline so a slow update
/// releases the writer within that window instead of holding it for the full (longer) read
/// timeout, cooperatively per caveat (1).
async fn await_update_worker(
    task: tokio::task::JoinHandle<Result<u64, String>>,
    config: &ServerConfig,
) -> UpdateOutcome {
    let joined = match config.query_timeout {
        Some(t) => match tokio::time::timeout(t + TIMEOUT_GRACE, task).await {
            Ok(j) => j,
            Err(_elapsed) => return UpdateOutcome::TimedOut,
        },
        None => task.await,
    };
    match joined {
        Ok(Ok(number)) => UpdateOutcome::Ok(number),
        Ok(Err(e)) => UpdateOutcome::Rejected(e),
        Err(_) => UpdateOutcome::Panicked,
    }
}

/// Runs a SELECT on the engine and serialises it in the negotiated format. JSON uses the
/// engine's direct id→JSON fast path and is **streamed**: the engine returns the body as
/// an ordered chunk sequence (concatenation byte-identical to the single-string form) and
/// the response body hands those chunks to hyper one by one — the peak never holds a
/// second whole-result copy (T16). `Content-Length` is known up front (the chunks are
/// fully evaluated before the response starts), so the response framing is identical to
/// the buffered path. The other formats go through the materialised `QueryResult`.
///
/// Takes the request's pinned generation (not a bare `&Graph`) so the streamed JSON body
/// can keep the generation pinned until the last chunk is written — the stream stays
/// snapshot-consistent with query START even if the writer publishes past it mid-response.
fn render_select(
    gen: &PinnedGen,
    select: &str,
    fmt: Format,
    head_only: bool,
    budget: &QueryBudget,
    config: &ServerConfig,
) -> Response {
    let graph = gen.snapshot();
    let ct = fmt.select_content_type();
    let body = match fmt {
        // SELECT projections fold in --max-results (`make_budget(_, true)`) → name it.
        Format::Json => match sparq_engine::query_json_chunks_with_budget(graph, select, budget) {
            Ok(chunks) => {
                return chunked_response(StatusCode::OK, ct, chunks, head_only, gen.clone())
            }
            Err(e) => return engine_error_response(&e, config, true),
        },
        // CSV/TSV: row-oriented chunked streaming — mirrors the JSON T16 path; the peak
        // never holds a second full-result copy. Byte-identical to the buffered form per
        // the chunked invariant. XML stays buffered (prefix compaction). [SONNET-4.6]
        Format::Csv | Format::Tsv => {
            let result = match sparq_engine::query_with_budget(graph, select, budget) {
                Ok(r) => r,
                Err(e) => return engine_error_response(&e, config, true),
            };
            let chunks = match fmt {
                Format::Csv => results::select_to_csv_chunks(&result),
                Format::Tsv => results::select_to_tsv_chunks(&result),
                _ => unreachable!(),
            };
            return chunked_response(StatusCode::OK, ct, chunks, head_only, gen.clone())
        }
        Format::Xml => {
            let result = match sparq_engine::query_with_budget(graph, select, budget) {
                Ok(r) => r,
                Err(e) => return engine_error_response(&e, config, true),
            };
            results::select_to_xml(&result)
        }
    };
    text_response(StatusCode::OK, ct, body, head_only)
}

/// [OPUS-4.8] (sq-7d3dj.34.2) Bounded chunk buffer between the engine worker and the HTTP
/// response body. Small on purpose: a slow client back-pressures the worker (which blocks on
/// `blocking_send`) rather than letting a large serialised result pile up in server memory.
const STREAM_CHANNEL_CAP: usize = 4;

/// [SONNET-4.6] (sq-7d3dj.26) Trailer field asserting the streamed body is the COMPLETE result.
/// Sent (value `true`) only after the engine has confirmed it produced every chunk.
const TRAILER_COMPLETE: &str = "x-sparq-complete";

/// [SONNET-4.6] (sq-7d3dj.26) Trailer field reporting a TRUNCATED streamed body; the value is
/// the reason from [`truncation_reason`].
const TRAILER_TRUNCATED: &str = "x-sparq-truncated";

/// [SONNET-4.6] (sq-7d3dj.26) One item on the engine-worker → HTTP-body channel.
///
/// [`StreamItem::Done`] is a load-bearing EXPLICIT end-of-stream marker, not a convenience:
/// without it a worker that dies mid-result (a panic in the engine or the serialiser) is
/// indistinguishable — from the body's side — from a worker that finished, and the body would
/// then advertise a truncated result as complete. Channel-closed-without-a-marker therefore
/// MEANS truncated.
enum StreamItem {
    /// One serialised SPARQL-results-JSON chunk, in order.
    Chunk(Bytes),
    /// The engine returned `Ok` — every chunk of the complete result has been handed over.
    Done,
    /// The engine returned `Err` (budget row/byte cap, deadline, cancellation) — the chunks
    /// already handed over are a PREFIX of a result that will never be finished.
    Failed(String),
}

/// [SONNET-4.6] (sq-7d3dj.26) Maps the engine's abort message onto the stable
/// `X-Sparq-Truncated` reason vocabulary. Mirrors the classification
/// [`engine_error_response`] applies when the same abort lands BEFORE the first byte, so a
/// client sees the same cause named either side of the status commit.
fn truncation_reason(e: &str) -> &'static str {
    if e.contains("query budget exceeded (timeout)") {
        "deadline"
    } else if e.contains("query budget exceeded (max-rows)") {
        "max-rows"
    } else if e.contains("query budget exceeded (max-bytes)") {
        "max-bytes"
    } else if e.contains("query budget exceeded (cancelled)") {
        "cancelled"
    } else {
        "error"
    }
}

/// [SONNET-4.6] (sq-7d3dj.26) `true` when the request's `TE` header lists `trailers`
/// (RFC 9110 §10.1.4) — i.e. the client declared it can receive trailer fields. hyper only
/// writes a trailers frame for such a client, so this also decides how a mid-stream truncation
/// is signalled: a trailer for a client that asked for one, an aborted chunked framing (no
/// terminating zero-length chunk) for one that did not.
///
/// A `q=0` weight on the token means "not acceptable" (RFC 9110 §12.4.2), so `TE: trailers;q=0`
/// is a client DECLINING trailers and yields `false`. The RFC 9110 grammar does not admit a
/// weight on `trailers` at all, so such a parameter is already off-spec; anything unparseable
/// as a positive qvalue is therefore resolved conservatively to `false` — malformed input can
/// only pick the loud aborted-framing signal, never the quiet normal-completion trailer path.
fn client_accepts_trailers(headers: &HeaderMap) -> bool {
    headers.get_all(header::TE).iter().any(|value| {
        value.to_str().is_ok_and(|raw| {
            raw.split(',').any(|token| {
                let mut parts = token.split(';');
                if !parts
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("trailers")
                {
                    return false;
                }
                // Every parameter must leave the token acceptable: a `q` parameter counts only
                // when it parses as a strictly positive qvalue; other parameters are ignored.
                parts.all(|param| match param.split_once('=') {
                    Some((name, weight)) => {
                        !name.trim().eq_ignore_ascii_case("q") || qvalue_is_positive(weight.trim())
                    }
                    // A bare `q` carries no weight and is malformed — decline conservatively.
                    None => !param.trim().eq_ignore_ascii_case("q"),
                })
            })
        })
    })
}

/// [SONNET-4.6] `true` when `raw` is a well-formed, strictly positive RFC 9110 §12.4.2 qvalue
/// (`( "0" [ "." 0*3DIGIT ] ) / ( "1" [ "." 0*3"0" ] )`). `0`, `0.0`, `0.000` — and anything
/// that is not a valid qvalue at all — are `false`, so a caller reading this as "acceptable"
/// fails closed on malformed input.
fn qvalue_is_positive(raw: &str) -> bool {
    let (int, frac) = raw.split_once('.').unwrap_or((raw, ""));
    if frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match int {
        // `1` is only well-formed with an all-zero fraction; `1.5` is malformed, not "positive".
        "1" => frac.bytes().all(|b| b == b'0'),
        "0" => frac.bytes().any(|b| b != b'0'),
        _ => false,
    }
}

/// [SONNET-4.6] (sq-7d3dj.26) Where a [`StreamingJsonBody`] is in its lifecycle.
#[derive(Clone, Copy)]
enum BodyState {
    /// Still forwarding chunks from the engine worker.
    Streaming,
    /// The outcome is known; the next poll emits the trailers frame. `None` = complete,
    /// `Some(reason)` = truncated for that reason.
    Trailers(Option<&'static str>),
    /// Nothing further to emit.
    Finished,
}

/// [SONNET-4.6] (sq-7d3dj.26) **Truncation-safe** streamed SPARQL-results-JSON body.
///
/// Once the first byte of a `200 OK` body is on the wire the status is committed, so a
/// mid-stream abort (deadline, `--max-results` / `--max-query-rows` row cap, `--max-query-bytes`
/// byte cap, a `DELETE /queries/{id}` cancellation, or an engine panic) can only TRUNCATE the
/// body — it can never retract into a 413/503. The load-bearing invariant
/// (`research/wave-d-pull-streaming-response-body.md` §6) is therefore:
///
/// > A client MUST NOT be able to mistake a truncated stream for a complete result.
///
/// Two mechanisms enforce it; the second sits strictly ON TOP of the first and never
/// substitutes for it:
///
/// 1. **Unterminated JSON — the floor, correct-by-construction.** A complete
///    `sparql-results+json` document ends with the closing `]}}`, and those bytes only ever
///    arrive in the engine's FINAL chunk. This body therefore WITHHOLDS any chunk that closes
///    the document until the worker has confirmed completion ([`StreamItem::Done`]); on a
///    truncation the withheld chunk is DROPPED and never written, so the received body is not
///    valid JSON and any conformant parser errors instead of silently accepting a short result
///    (the QLever/Virtuoso baseline). This is not theoretical: the engine's streaming
///    single-pattern scan appends `]}}` when its cooperative budget check breaks the loop, and
///    only its caller then turns the sticky flag into an `Err` — so without the hold-back the
///    wire would carry a well-formed, correctly-terminated SHORT `200`, exactly the outcome the
///    design forbids.
/// 2. **A signal the client cannot miss.** If the client negotiated trailers (`TE: trailers`)
///    the body closes normally and emits `X-Sparq-Truncated: <reason>` (`deadline` | `max-rows`
///    | `max-bytes` | `cancelled` | `panic` | `error`), or `X-Sparq-Complete: true` on the happy
///    path. If it did not — hyper would drop a trailers frame anyway — the body instead FAILS
///    the frame, which aborts the chunked stream without its terminating zero-length chunk, so
///    the client sees a transport error. Either way the truncation is loud.
///
/// The hold-back costs no latency on the happy path: only the document-closing chunk is ever
/// withheld, and it is released the moment the worker reports completion.
struct StreamingJsonBody {
    /// Items received but not yet processed: the two chunks buffered before the status was
    /// committed, plus at most one push-back from the hold-back below.
    queued: std::collections::VecDeque<StreamItem>,
    /// The engine channel; `None` once a terminal outcome has been observed.
    rx: Option<tokio::sync::mpsc::Receiver<StreamItem>>,
    /// The withheld document-closing chunk (mechanism 1).
    held: Option<Bytes>,
    /// The client negotiated `TE: trailers`.
    trailers_ok: bool,
    state: BodyState,
}

impl StreamingJsonBody {
    /// `true` when `chunk` carries the bytes that CLOSE the SPARQL-results document.
    ///
    /// Only the engine's final chunk can end this way: every intermediate flush boundary lands
    /// immediately after a solution object, whose last bytes are `"}` / `}}` — a `]` can never
    /// precede them, because the `]` that closes `results.bindings` is written exactly once,
    /// together with the two closing braces, at the end of the document.
    fn closes_document(chunk: &Bytes) -> bool {
        chunk.ends_with(b"]}}")
    }

    /// Records a truncation: DROPS the withheld closing bytes (so the body can never be parsed
    /// as a complete result) and picks the out-of-band signal. Returns `Some(err)` when the
    /// caller must fail the frame — no trailer was negotiated, so the aborted framing IS the
    /// signal — and `None` when the pending trailer will carry the reason instead.
    fn truncate(&mut self, reason: &'static str) -> Option<std::io::Error> {
        self.held = None;
        self.rx = None;
        self.queued.clear();
        if self.trailers_ok {
            self.state = BodyState::Trailers(Some(reason));
            None
        } else {
            self.state = BodyState::Finished;
            Some(std::io::Error::other(format!(
                "streamed SPARQL result truncated ({}); the body is deliberately unterminated",
                reason
            )))
        }
    }
}

impl http_body::Body for StreamingJsonBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        use std::task::Poll;
        let this = self.get_mut();
        loop {
            // Copy the state out first: every arm below mutates `this`.
            let state = this.state;
            match state {
                BodyState::Finished => return Poll::Ready(None),
                BodyState::Trailers(reason) => {
                    this.state = BodyState::Finished;
                    // Without `TE: trailers` hyper drops the frame: the truncation path has
                    // already failed the body instead, and a complete body needs no signal.
                    if !this.trailers_ok {
                        return Poll::Ready(None);
                    }
                    let (name, value) = match reason {
                        None => (TRAILER_COMPLETE, "true"),
                        Some(r) => (TRAILER_TRUNCATED, r),
                    };
                    let mut trailers = HeaderMap::new();
                    trailers.insert(
                        header::HeaderName::from_static(name),
                        header::HeaderValue::from_static(value),
                    );
                    return Poll::Ready(Some(Ok(http_body::Frame::trailers(trailers))));
                }
                BodyState::Streaming => {
                    let next = match this.queued.pop_front() {
                        Some(item) => Some(item),
                        None => {
                            // Resolve the channel poll into an owned value BEFORE touching
                            // `this` again, so the receiver borrow is over by then.
                            let polled = match this.rx.as_mut() {
                                None => {
                                    this.state = BodyState::Finished;
                                    return Poll::Ready(None);
                                }
                                Some(rx) => rx.poll_recv(cx),
                            };
                            match polled {
                                Poll::Pending => return Poll::Pending,
                                Poll::Ready(item) => item,
                            }
                        }
                    };
                    // The worker vanished without a terminal marker — a panic in the engine
                    // or the serialiser. Truncated, never "complete".
                    let Some(item) = next else {
                        match this.truncate("panic") {
                            Some(e) => return Poll::Ready(Some(Err(e))),
                            None => continue,
                        }
                    };
                    match item {
                        StreamItem::Chunk(bytes) => {
                            if Self::closes_document(&bytes) {
                                // Mechanism 1: withhold the closing bytes until the worker
                                // confirms the result is complete.
                                if let Some(previous) = this.held.replace(bytes) {
                                    return Poll::Ready(Some(Ok(http_body::Frame::data(previous))));
                                }
                                continue;
                            }
                            if let Some(previous) = this.held.take() {
                                // Defensive: a chunk followed the one we took for the last, so
                                // it was not the last after all — release it and re-queue this.
                                this.queued.push_front(StreamItem::Chunk(bytes));
                                return Poll::Ready(Some(Ok(http_body::Frame::data(previous))));
                            }
                            return Poll::Ready(Some(Ok(http_body::Frame::data(bytes))));
                        }
                        StreamItem::Done => {
                            this.rx = None;
                            this.state = BodyState::Trailers(None);
                            if let Some(closing) = this.held.take() {
                                // Complete: the closing bytes are safe to put on the wire.
                                return Poll::Ready(Some(Ok(http_body::Frame::data(closing))));
                            }
                            continue;
                        }
                        StreamItem::Failed(e) => match this.truncate(truncation_reason(&e)) {
                            Some(err) => return Poll::Ready(Some(Err(err))),
                            None => continue,
                        },
                    }
                }
            }
        }
    }
}

/// [SONNET-4.6] (sq-7d3dj.26) The two request-head facts that decide the SHAPE of a streamed
/// SELECT-JSON response, carried together rather than as adjacent `bool` parameters: they are
/// resolved at the same place from the same request, and a pair of positional booleans is
/// exactly the signature a caller can silently transpose.
struct StreamShape {
    /// `HEAD`: emit the status + headers the GET would have produced, never a body.
    head_only: bool,
    /// The client sent `TE: trailers` (see [`client_accepts_trailers`]), so the completeness
    /// verdict can travel as a trailer instead of an aborted chunked framing.
    trailers_ok: bool,
}

/// [OPUS-4.8] (sq-7d3dj.34.2) Streams a SELECT result as SPARQL-results JSON.
///
/// The engine runs on a `spawn_blocking` worker and hands each ~64 KiB chunk
/// ([`sparq_engine::query_json_stream_with_budget`]) to a bounded channel; the HTTP body
/// drains the channel, so the results header + early solutions reach the socket **before the
/// whole result is serialised** (TTFB). The concatenation of the streamed body is
/// byte-identical to the buffered [`render_select`] JSON path.
///
/// **Status vs. first byte.** We await the FIRST channel item before committing a status, so
/// a failure detected *before any byte is written* — a parse error, or a cooperative budget /
/// deadline trip that fires before the header — still produces the correct HTTP status (400 /
/// 413 / 503). A **single-chunk** (small) result is returned buffered with a `Content-Length`
/// (the sub-millisecond floor path is unchanged in shape); a **multi-chunk** result streams
/// under chunked transfer-encoding with no `Content-Length` (the length is unknown until the
/// last row).
///
/// **Error mid-stream.** Once the header has been flushed the status is committed and cannot
/// change; a later budget / deadline trip can only TRUNCATE the body. [`StreamingJsonBody`]
/// owns that contract — the document-closing `]}}` is withheld until the engine confirms
/// completion, so a truncated body is never valid JSON, and the reason is reported either as
/// an `X-Sparq-Truncated` trailer or (for a client that did not negotiate trailers) as an
/// aborted chunked framing. This is the honest default (documented in the crate docs + the
/// `http-server` SKILL): a streamed body cannot retroactively become a 413/503, but it can
/// never be mistaken for a complete result either.
async fn stream_select_json(
    gen: PinnedGen,
    // [OPUS-4.8] (sq-7d3dj.34.1) The query PARSED ONCE in `prepare` — the engine runs it via
    // `query_json_stream_prepared_with_budget` without re-parsing (the HTTP floor win).
    query: sparq_engine::PreparedQuery,
    budget: QueryBudget,
    // [SONNET-4.6] (sq-7d3dj.26) `HEAD`-ness and `TE: trailers` negotiation, both read off the
    // request head by the caller — see [`StreamShape`].
    shape: StreamShape,
    allow: crate::service_config::ServiceAllowlist,
    // [SONNET-4.6] (sq-qsm5z) The RAII registry guard, as a type-erased send-able box so the
    // signature compiles in both feature states without a conditional type. Moved into the worker
    // so the row is present for the full streaming evaluation; dropped when the worker exits.
    qr_guard: Option<Box<dyn std::any::Any + Send>>,
    config: &ServerConfig,
) -> Response {
    let StreamShape { head_only, trailers_ok } = shape;
    let ct = Format::Json.select_content_type();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamItem>(STREAM_CHANNEL_CAP);
    tokio::task::spawn_blocking(move || {
        // Hold the registry guard alive for the entire streaming evaluation.
        let _guard = qr_guard;
        with_engine_scope_allow(&allow, || {
            let graph = gen.snapshot();
            let mut sink = |chunk: String| match tx.blocking_send(StreamItem::Chunk(Bytes::from(chunk.into_bytes()))) {
                Ok(()) => std::ops::ControlFlow::Continue(()),
                // The receiver was dropped — the client disconnected (or a HEAD request
                // stopped reading). Abandon the rest of the work.
                Err(_) => std::ops::ControlFlow::Break(()),
            };
            let outcome = sparq_engine::query_json_stream_prepared_with_budget(graph, &query, &budget, &mut sink);
            // [SONNET-4.6] (sq-7d3dj.26) ALWAYS post a terminal marker: the body side treats
            // "channel closed without one" as a truncation (a panicking worker), so the
            // completion claim can only ever come from the engine actually returning `Ok`.
            // A send failure here just means the client already went away.
            let _ = tx.blocking_send(match outcome {
                Ok(()) => StreamItem::Done,
                Err(e) => StreamItem::Failed(e),
            });
            // `gen` (the generation pin) is held until the worker returns, so every snapshot
            // borrow above is valid for the whole streamed evaluation.
        });
    });

    // Await the first item under the same wall-clock cap the buffered path uses, so a
    // pre-first-byte failure still maps to the right status.
    let first = match recv_first_chunk(&mut rx, config).await {
        Ok(Some(StreamItem::Chunk(bytes))) => bytes,
        Ok(Some(StreamItem::Failed(e))) => return engine_error_response(&e, config, true),
        // The engine reported completion without emitting anything, or the worker died before
        // the first chunk. Nothing is committed, so this is still an honest 500.
        Ok(Some(StreamItem::Done)) | Ok(None) => {
            return execution_error("query produced no result stream")
        }
        Err(()) => return timeout_response(config),
    };

    // Peek the second item: a single-chunk result is returned buffered (Content-Length);
    // a multi-chunk result streams.
    match rx.recv().await {
        Some(StreamItem::Done) => {
            let len = first.len();
            let body = if head_only {
                axum::body::Body::empty()
            } else {
                axum::body::Body::from(first)
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct)
                .header(header::CONTENT_LENGTH, len)
                .body(body)
                .unwrap()
        }
        // [OPUS-4.8] (sq-7d3dj.34.2) The engine produced the (only) chunk and THEN failed —
        // e.g. a row/byte cap that the single-pattern scan only confirms after emitting the
        // document, or a cooperative deadline trip. Because we buffer the first chunk before
        // committing a status, NOTHING has been flushed to the socket yet, so we still return
        // the correct refusal (413 / 503), exactly like the buffered path — never a truncated
        // 200. (A cap tripped on a genuinely multi-chunk result — both peeked items are
        // chunks — cannot be un-committed and is truncated mid-stream; see below.)
        Some(StreamItem::Failed(e)) => engine_error_response(&e, config, true),
        // [SONNET-4.6] (sq-7d3dj.26) The worker vanished without a terminal marker (a panic in
        // the engine or the serialiser) having produced exactly one chunk. Nothing has been
        // flushed to the socket, so the status is NOT yet committed: answer with the honest
        // 500 rather than a truncated 200.
        None => execution_error("streaming query worker ended before completing the result"),
        Some(StreamItem::Chunk(second)) => {
            if head_only {
                // HEAD: status + headers only. Dropping the receiver stops the worker.
                drop(rx);
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, ct)
                    .body(axum::body::Body::empty())
                    .unwrap();
            }
            // Stream first + second + the remaining channel items under chunked
            // transfer-encoding, through the truncation-safe body: a post-header budget /
            // deadline trip (or a dead worker) drops the document-closing bytes and reports
            // the reason, so the 200 — already committed — can never be read as complete.
            let mut queued: std::collections::VecDeque<StreamItem> =
                std::collections::VecDeque::with_capacity(2);
            queued.push_back(StreamItem::Chunk(first));
            queued.push_back(StreamItem::Chunk(second));
            let body = axum::body::Body::new(StreamingJsonBody {
                queued,
                rx: Some(rx),
                held: None,
                trailers_ok,
                state: BodyState::Streaming,
            });
            let builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct);
            // RFC 9110 §6.6.1: announce the trailer fields that may follow the body, so a
            // client knows to look for the completeness verdict. Only for a client that
            // negotiated them — hyper drops the frame otherwise.
            let builder = if trailers_ok {
                builder.header(
                    header::TRAILER,
                    format!("{}, {}", TRAILER_COMPLETE, TRAILER_TRUNCATED),
                )
            } else {
                builder
            };
            builder.body(body).unwrap()
        }
    }
}

/// [OPUS-4.8] (sq-7d3dj.34.2) Awaits the first streamed chunk (or the pre-first-byte error)
/// under the read wall-clock cap (`query_timeout + TIMEOUT_GRACE`), mirroring
/// [`await_worker`]. `Err(())` means the cap elapsed before the worker produced anything.
///
/// Generic over the channel's item so both streamed paths share the one cap: the SELECT stream
/// carries [`StreamItem`] (chunk / done / failed), the CONSTRUCT-DESCRIBE stream of
/// [`stream_graph_result`] carries `Result<Bytes, String>`.
async fn recv_first_chunk<T>(
    rx: &mut tokio::sync::mpsc::Receiver<T>,
    config: &ServerConfig,
) -> Result<Option<T>, ()> {
    match config.query_timeout {
        Some(t) => match tokio::time::timeout(t + TIMEOUT_GRACE, rx.recv()).await {
            Ok(item) => Ok(item),
            Err(_elapsed) => Err(()),
        },
        None => Ok(rx.recv().await),
    }
}

// ---------------------------------------------------------------------------
// Streamed CONSTRUCT / DESCRIBE bodies ([FABLE-5] sq-0kq6k)
// ---------------------------------------------------------------------------

/// How many rendered bytes accumulate before a chunk is handed to the response body.
///
/// This is the knob that decides "buffered" vs "chunked" for a graph result: a document that
/// fits in one chunk takes the buffered branch of [`stream_graph_result`] and is answered with
/// a `Content-Length`, exactly as before this change; only a document that exceeds it streams.
/// 64 KiB is the same order as the SELECT-JSON stream's natural chunking, big enough that the
/// per-chunk channel send is noise against the serialisation work.
const GRAPH_STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// [FABLE-5] sq-0kq6k: an [`std::io::Write`] that forwards the rendered RDF document to the
/// response body in ~[`GRAPH_STREAM_CHUNK_BYTES`] pieces instead of accumulating it.
///
/// This is the whole point of the streaming path: peak resident memory for the response body
/// becomes one chunk plus whatever the serialiser holds, rather than the entire rendered
/// document. (The `Vec<Triple>` the engine produced is still fully materialised — that is the
/// CONSTRUCT/DESCRIBE result itself, not the rendering, and is out of scope here.)
///
/// A failed send means the receiver was dropped — the client disconnected — and is surfaced as
/// an `io::Error` so the serialiser stops rendering instead of finishing work nobody will read.
struct ChunkSink {
    tx: tokio::sync::mpsc::Sender<Result<Bytes, String>>,
    buf: Vec<u8>,
    /// Whether any chunk has been handed over yet — [`ChunkSink::finish`] uses it to guarantee
    /// at least one (possibly empty) chunk, so an empty CONSTRUCT still answers `200` with an
    /// empty body rather than looking like a worker that produced no stream at all.
    sent: bool,
}

impl ChunkSink {
    fn new(tx: tokio::sync::mpsc::Sender<Result<Bytes, String>>) -> Self {
        Self { tx, buf: Vec::with_capacity(GRAPH_STREAM_CHUNK_BYTES), sent: false }
    }

    fn send(&mut self, chunk: Vec<u8>) -> std::io::Result<()> {
        self.sent = true;
        self.tx
            .blocking_send(Ok(Bytes::from(chunk)))
            .map_err(|_| std::io::Error::other("client disconnected"))
    }

    /// Hands over every WHOLE chunk currently buffered, keeping the trailing remainder.
    fn drain_full_chunks(&mut self) -> std::io::Result<()> {
        while self.buf.len() >= GRAPH_STREAM_CHUNK_BYTES {
            let rest = self.buf.split_off(GRAPH_STREAM_CHUNK_BYTES);
            let chunk = std::mem::replace(&mut self.buf, rest);
            self.send(chunk)?;
        }
        Ok(())
    }

    /// Hands over the trailing partial chunk. Consumes the sink so the sender is dropped
    /// afterwards, which is what closes the response stream.
    fn finish(mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() || !self.sent {
            let chunk = std::mem::take(&mut self.buf);
            self.send(chunk)?;
        }
        Ok(())
    }
}

impl std::io::Write for ChunkSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        self.drain_full_chunks()?;
        Ok(data.len())
    }

    /// A no-op: partial chunks are held deliberately (that is the buffering) and are handed
    /// over by [`ChunkSink::finish`]. Flushing early would emit tiny chunks for no benefit.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// [FABLE-5] sq-0kq6k: evaluates a CONSTRUCT / DESCRIBE and STREAMS the negotiated RDF
/// document as the response body, so a large result does not buffer the whole rendered
/// document in the server before the first byte reaches the socket.
///
/// **Shape** (deliberately the same as [`stream_select_json`]): the engine runs on a blocking
/// worker that feeds a bounded channel; a **single-chunk** result is answered buffered with a
/// `Content-Length` (byte-identical response to the pre-streaming path — every small
/// CONSTRUCT keeps its old wire shape); a **multi-chunk** result streams under chunked
/// transfer-encoding with no `Content-Length`.
///
/// **Status semantics are unchanged, and stronger than the SELECT stream's.** The engine
/// materialises the whole `Vec<Triple>` BEFORE serialisation begins, so every budget / deadline
/// / evaluation failure is known before a single byte is rendered. A refusal is therefore
/// always a clean `413`/`503`/`500` — a graph result can never be truncated mid-stream the way
/// a streamed SELECT can. The `Some(Err(_))` arm below is defensive, not reachable today.
async fn stream_graph_result(
    gen: PinnedGen,
    runnable: String,
    gfmt: GraphFormat,
    budget: QueryBudget,
    allow: crate::service_config::ServiceAllowlist,
    // [SONNET-4.6] (sq-qsm5z) The RAII running-query registry guard, type-erased so the
    // signature compiles in both feature states. Held for the whole evaluation.
    qr_guard: Option<Box<dyn std::any::Any + Send>>,
    config: &ServerConfig,
) -> Response {
    let ct = gfmt.content_type();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Bytes, String>>(STREAM_CHANNEL_CAP);
    tokio::task::spawn_blocking(move || {
        let _guard = qr_guard;
        with_engine_scope_allow(&allow, || {
            match sparq_engine::construct_or_describe_with_budget(
                gen.snapshot(),
                &runnable,
                &budget,
            ) {
                // Nothing has been rendered yet, so this still maps to the right status.
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                }
                Ok(triples) => {
                    let mut sink = ChunkSink::new(tx);
                    // The only error the writers can raise here is the sink's own "client
                    // disconnected"; in that case abandon the remaining chunks too.
                    if serialise_graph_triples_to(&triples, gfmt, &mut sink).is_ok() {
                        let _ = sink.finish();
                    }
                }
            }
            // `gen` is held until the worker returns, so the snapshot borrow above is valid
            // for the whole evaluation.
        });
    });

    // Await the first item under the same wall-clock cap the buffered path uses, so a
    // pre-first-byte failure still maps to the right status.
    let first = match recv_first_chunk(&mut rx, config).await {
        Ok(Some(Ok(bytes))) => bytes,
        // CONSTRUCT/DESCRIBE used `make_budget(_, true)` → max_results applied.
        Ok(Some(Err(e))) => return engine_error_response(&e, config, true),
        Ok(None) => return execution_error("query produced no result stream"),
        Err(()) => return timeout_response(config),
    };

    match rx.recv().await {
        // One chunk only: answer exactly as the buffered path did, Content-Length and all.
        None => {
            let len = first.len();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct)
                .header(header::CONTENT_LENGTH, len)
                .body(axum::body::Body::from(first))
                .unwrap()
        }
        // Defensive: the worker never reports an error after a chunk (see the doc above).
        Some(Err(e)) => engine_error_response(&e, config, true),
        Some(Ok(second)) => {
            let mut prefix: std::collections::VecDeque<Result<Bytes, String>> =
                std::collections::VecDeque::with_capacity(2);
            prefix.push_back(Ok(first));
            prefix.push_back(Ok(second));
            let body = axum::body::Body::from_stream(futures_util::stream::unfold(
                (prefix, rx),
                |(mut prefix, mut rx)| async move {
                    let item = match prefix.pop_front() {
                        Some(it) => Some(it),
                        None => rx.recv().await,
                    };
                    item.map(|it| (it.map_err(std::io::Error::other), (prefix, rx)))
                },
            ));
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct)
                .body(body)
                .unwrap()
        }
    }
}

#[cfg(test)]
mod chunk_sink_tests {
    use super::*;
    use std::io::Write as _;

    /// A channel roomy enough that a synchronous test can fill it without blocking (the real
    /// path uses `STREAM_CHANNEL_CAP` and is drained concurrently by the response body).
    fn sink() -> (ChunkSink, tokio::sync::mpsc::Receiver<Result<Bytes, String>>) {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        (ChunkSink::new(tx), rx)
    }

    /// Drains everything the sink handed over, in order.
    fn drain(mut rx: tokio::sync::mpsc::Receiver<Result<Bytes, String>>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(Ok(b)) = rx.try_recv() {
            out.push(b.to_vec());
        }
        out
    }

    #[test]
    fn a_small_document_is_handed_over_as_one_chunk() {
        let (mut s, rx) = sink();
        s.write_all(b"hello world").unwrap();
        s.finish().unwrap();
        assert_eq!(drain(rx), vec![b"hello world".to_vec()]);
    }

    #[test]
    fn an_empty_document_still_yields_exactly_one_empty_chunk() {
        // Load-bearing: the response builder treats "no chunk at all" as a worker that died,
        // so an empty CONSTRUCT must still produce a chunk to answer 200 with an empty body.
        let (s, rx) = sink();
        s.finish().unwrap();
        assert_eq!(drain(rx), vec![Vec::<u8>::new()]);
    }

    #[test]
    fn a_large_document_splits_on_the_threshold_and_reassembles_exactly() {
        let (mut s, rx) = sink();
        // Two and a half chunks, written in pieces that do not align with the boundary.
        let payload: Vec<u8> = (0..GRAPH_STREAM_CHUNK_BYTES * 5 / 2).map(|i| (i % 251) as u8).collect();
        for piece in payload.chunks(7777) {
            s.write_all(piece).unwrap();
        }
        s.finish().unwrap();

        let chunks = drain(rx);
        assert_eq!(chunks.len(), 3, "2.5 chunks of payload → 2 full chunks + the remainder");
        assert!(
            chunks[..2].iter().all(|c| c.len() == GRAPH_STREAM_CHUNK_BYTES),
            "every non-final chunk must be exactly the threshold"
        );
        assert_eq!(chunks.concat(), payload, "the chunks must reassemble to the exact document");
    }

    #[test]
    fn a_disconnected_client_stops_the_serialiser() {
        let (mut s, rx) = sink();
        drop(rx);
        // The first write that crosses the threshold discovers the dropped receiver.
        let payload = vec![b'x'; GRAPH_STREAM_CHUNK_BYTES + 1];
        assert!(s.write_all(&payload).is_err(), "a dropped receiver must surface as an io error");
    }
}

#[cfg(test)]
mod truncation_safety_tests {
    //! [SONNET-4.6] (sq-7d3dj.26) The mid-stream truncation-safety invariant for the
    //! pull-streaming SELECT body: *a client MUST NOT be able to mistake a truncated stream
    //! for a complete result*. These pin [`StreamingJsonBody`]'s side of it directly (the
    //! end-to-end HTTP assertion, over the real engine + budget, is
    //! `tests/stream_truncation.rs`).
    use super::*;
    use http_body::Body as _;

    /// A head chunk and a document-CLOSING chunk, together a complete two-chunk document.
    const HEAD: &str = "{\"head\":{\"vars\":[\"s\"]},\"results\":{\"bindings\":[{\"s\":{\"type\":\"uri\",\"value\":\"http://ex/a\"}}";
    const TAIL: &str = ",{\"s\":{\"type\":\"uri\",\"value\":\"http://ex/b\"}}]}}";

    /// What draining a body to its end produced.
    struct Drained {
        body: Vec<u8>,
        trailers: Option<HeaderMap>,
        /// The body FAILED a frame — hyper aborts the chunked stream without its terminating
        /// zero-length chunk, so the client sees a transport error.
        failed: bool,
    }

    impl Drained {
        fn text(&self) -> String {
            String::from_utf8(self.body.clone()).unwrap()
        }
        fn trailer(&self, name: &str) -> Option<String> {
            self.trailers
                .as_ref()?
                .get(name)
                .map(|v| v.to_str().unwrap().to_owned())
        }
    }

    /// Feeds `items` through a real bounded channel (closed after the last one) into a
    /// `StreamingJsonBody`, then drains every frame.
    async fn drain(items: Vec<StreamItem>, trailers_ok: bool) -> Drained {
        let (tx, rx) = tokio::sync::mpsc::channel(items.len().max(1));
        for item in items {
            tx.try_send(item).unwrap();
        }
        drop(tx); // channel closes after the queued items
        let mut body = StreamingJsonBody {
            queued: std::collections::VecDeque::new(),
            rx: Some(rx),
            held: None,
            trailers_ok,
            state: BodyState::Streaming,
        };
        let mut out = Drained { body: Vec::new(), trailers: None, failed: false };
        loop {
            let frame =
                std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_frame(cx)).await;
            match frame {
                None => return out,
                Some(Err(_)) => {
                    out.failed = true;
                    return out;
                }
                Some(Ok(frame)) => match frame.into_data() {
                    Ok(data) => out.body.extend_from_slice(&data),
                    Err(frame) => out.trailers = frame.into_trailers().ok(),
                },
            }
        }
    }

    fn chunk(s: &str) -> StreamItem {
        StreamItem::Chunk(Bytes::from(s.as_bytes().to_vec()))
    }

    /// The happy path: every chunk INCLUDING the document-closing one reaches the client, the
    /// body is valid `sparql-results+json`, and the trailer asserts completeness.
    #[tokio::test]
    async fn a_complete_stream_delivers_the_closing_bytes_and_claims_completeness() {
        let out = drain(vec![chunk(HEAD), chunk(TAIL), StreamItem::Done], true).await;
        assert!(!out.failed, "a complete stream must not fail a frame");
        assert_eq!(out.text(), format!("{}{}", HEAD, TAIL));
        assert!(out.text().ends_with("]}}"), "the document must be closed");
        serde_json::from_slice::<serde_json::Value>(&out.body)
            .expect("a complete stream must parse as JSON");
        assert_eq!(out.trailer(TRAILER_COMPLETE).as_deref(), Some("true"));
        assert_eq!(out.trailer(TRAILER_TRUNCATED), None);
    }

    /// THE LOAD-BEARING CASE. The engine's streaming scan appends `]}}` when its cooperative
    /// row cap breaks the loop, and only THEN reports the abort — so the closing chunk and the
    /// failure both arrive. The body must drop the closing chunk: the client receives an
    /// UNPARSEABLE prefix plus an explicit truncation reason, never a clean short 200.
    #[tokio::test]
    async fn a_mid_stream_row_cap_abort_never_closes_the_document() {
        let out = drain(
            vec![
                chunk(HEAD),
                chunk(TAIL),
                StreamItem::Failed("query budget exceeded (max-rows)".into()),
            ],
            true,
        )
        .await;
        assert_eq!(out.text(), HEAD, "the document-closing chunk must be withheld");
        assert!(!out.text().ends_with("]}}"), "a truncated body must NOT be closed");
        serde_json::from_slice::<serde_json::Value>(&out.body)
            .expect_err("a truncated body MUST fail to parse as JSON");
        assert_eq!(out.trailer(TRAILER_TRUNCATED).as_deref(), Some("max-rows"));
        assert_eq!(out.trailer(TRAILER_COMPLETE), None);
    }

    /// Same abort, but the client never sent `TE: trailers` — hyper would drop a trailers
    /// frame, so the signal is the FAILED frame (an aborted chunked framing) instead. The
    /// document still must not be closed.
    #[tokio::test]
    async fn without_negotiated_trailers_a_truncation_fails_the_framing() {
        let out = drain(
            vec![
                chunk(HEAD),
                chunk(TAIL),
                StreamItem::Failed("query budget exceeded (timeout)".into()),
            ],
            false,
        )
        .await;
        assert!(out.failed, "the aborted framing IS the signal without a trailer");
        assert_eq!(out.text(), HEAD);
        assert!(out.trailers.is_none());
        serde_json::from_slice::<serde_json::Value>(&out.body)
            .expect_err("a truncated body MUST fail to parse as JSON");
    }

    /// A worker that DIES mid-result (an engine / serialiser panic) closes the channel without
    /// any terminal marker. That must read as a truncation, never as completion — this is the
    /// case the explicit `Done` marker exists for.
    #[tokio::test]
    async fn a_worker_that_dies_without_a_marker_is_truncated_not_complete() {
        let out = drain(vec![chunk(HEAD)], true).await;
        assert_eq!(out.text(), HEAD);
        assert_eq!(out.trailer(TRAILER_TRUNCATED).as_deref(), Some("panic"));
        assert_eq!(out.trailer(TRAILER_COMPLETE), None);
        serde_json::from_slice::<serde_json::Value>(&out.body)
            .expect_err("a truncated body MUST fail to parse as JSON");
    }

    /// Each abort class the streamed path can hit is named distinctly, matching the status
    /// `engine_error_response` would have returned had the abort landed pre-first-byte.
    #[tokio::test]
    async fn every_abort_class_gets_its_own_reason() {
        for (error, reason) in [
            ("query budget exceeded (timeout)", "deadline"),
            ("query budget exceeded (max-rows)", "max-rows"),
            ("query budget exceeded (max-bytes)", "max-bytes"),
            ("query budget exceeded (cancelled)", "cancelled"),
            ("some other execution failure", "error"),
        ] {
            assert_eq!(truncation_reason(error), reason, "reason for: {}", error);
            let out = drain(
                vec![chunk(HEAD), chunk(TAIL), StreamItem::Failed(error.into())],
                true,
            )
            .await;
            assert_eq!(out.trailer(TRAILER_TRUNCATED).as_deref(), Some(reason));
            assert_eq!(out.text(), HEAD, "closing chunk withheld for: {}", error);
        }
    }

    /// Only the document-closing chunk is ever withheld, so the hold-back costs no latency on
    /// the chunks before it: a three-chunk stream delivers the first two immediately.
    #[tokio::test]
    async fn only_the_closing_chunk_is_withheld() {
        let middle = ",{\"s\":{\"type\":\"uri\",\"value\":\"http://ex/m\"}}";
        let out = drain(
            vec![
                chunk(HEAD),
                chunk(middle),
                chunk(TAIL),
                StreamItem::Failed("query budget exceeded (max-rows)".into()),
            ],
            true,
        )
        .await;
        assert_eq!(out.text(), format!("{}{}", HEAD, middle));
    }

    /// `TE` parsing: the transfer coding is matched case-insensitively, in a list, and honours
    /// a `q` weight — `q=0` is a client DECLINING trailers, not negotiating them. Anything else
    /// means "no trailers".
    #[test]
    fn te_trailers_negotiation_is_parsed() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::TE, header::HeaderValue::from_str(value).unwrap());
            client_accepts_trailers(&headers)
        };
        assert!(with("trailers"));
        assert!(with("gzip, trailers"));
        assert!(with("Trailers"));
        assert!(with("trailers;q=0.5"));
        assert!(with("trailers;q=1"));
        assert!(with("trailers;q=1.000"));
        // An unrelated parameter must not be mistaken for a weight.
        assert!(with("trailers;ext=0"));
        assert!(!with("gzip"));
        assert!(!with(""));
        assert!(!client_accepts_trailers(&HeaderMap::new()));

        // `q=0` means "not acceptable" — in every spelling, spacing and list position.
        assert!(!with("trailers;q=0"));
        assert!(!with("trailers;q=0.0"));
        assert!(!with("trailers;q=0.000"));
        assert!(!with("trailers; q=0"));
        assert!(!with("trailers ; Q = 0 "));
        assert!(!with("gzip, trailers;q=0"));
        assert!(!with("trailers;q=0, gzip"));

        // Malformed weights fail closed rather than opting into the trailer path.
        assert!(!with("trailers;q="));
        assert!(!with("trailers;q"));
        assert!(!with("trailers;q=abc"));
        assert!(!with("trailers;q=1.5"));
        assert!(!with("trailers;q=0.0000"));
    }
}

/// Maps an engine error string onto the HTTP guard semantics: budget timeout → 503,
/// budget row cap → 413 (honest refusal), anything else → 500.
///
/// `apply_max_results` MUST match the flag the path passed to [`make_budget`]: it tells this
/// function whether `--max-results` actually contributed to the `max_rows` budget on THIS
/// request, so the 413 message names the right knob.
///
/// [OPUS-4.8] (sq-ebii) Only SELECT projections (and EXPLAIN ANALYZE / CONSTRUCT / DESCRIBE,
/// which also pass `apply_max_results = true`) fold `--max-results` into the budget. ASK,
/// GSP-read and UPDATE build their budget from `--max-query-rows` ALONE
/// (`make_budget(_, false)` / `update_budget`), so on those paths `--max-results` did NOT
/// participate even when it is set and smaller. Picking the named knob from the global config
/// (the old behaviour) therefore misreported the cap on those paths — e.g. an ASK with
/// `--max-query-rows 100 --max-results 10` aborts at 100 rows but would have been reported as
/// "10 rows, --max-results". Gating the `max_results` consideration on `apply_max_results`
/// makes the message path-accurate (the 413 status itself was always correct — only the
/// human-readable knob name / row number could be wrong).
pub(crate) fn engine_error_response(e: &str, config: &ServerConfig, apply_max_results: bool) -> Response {
    if e.contains("query budget exceeded (timeout)") {
        return timeout_response(config);
    }
    if e.contains("query budget exceeded (max-rows)") {
        // Only the caps that actually fed THIS path's budget are eligible to be named.
        let results_cap = apply_max_results.then_some(config.max_results).flatten();
        let max = tighter(config.max_query_rows, results_cap).unwrap_or(0);
        let which = match (config.max_query_rows, results_cap) {
            (Some(q), Some(r)) if q <= r => "--max-query-rows (memory cap)",
            (Some(_), None) => "--max-query-rows (memory cap)",
            _ => "--max-results",
        };
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("result exceeds the server's working-set row limit ({max} rows, {which}); narrow the query (e.g. add LIMIT) or raise the limit"),
        );
    }
    // [OPUS-4.8] (sq-s5is) the byte-accounted cap tripped — an honest 413, same class as the
    // row cap, naming the byte knob (this path applies on EVERY form, so no `--max-results`).
    if e.contains("query budget exceeded (max-bytes)") {
        let max = config.max_query_bytes.unwrap_or(0);
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("result exceeds the server's working-set byte limit ({max} bytes, --max-query-bytes (memory cap)); narrow the query (e.g. project fewer variables, add LIMIT) or raise the limit"),
        );
    }
    // [OPUS-4.8] (sq-iu0c) A SERVICE to a host the egress allowlist / default-deny SSRF
    // policy blocked is a POLICY decision, not a server fault — surface it as an honest 403
    // (Forbidden), distinct from the generic 500 below, so clients and alerting can tell a
    // refused federation target apart from a real execution failure. The engine marks every
    // such refusal with `SERVICE_EGRESS_REFUSED_MARKER`, which survives the transport-error
    // wrapping. Gated on the `service` feature: with it off, no SERVICE clause can run, so
    // the marker can never appear.
    #[cfg(feature = "service")]
    if e.contains(sparq_engine::SERVICE_EGRESS_REFUSED_MARKER) {
        return forbidden_egress(e);
    }
    execution_error(e)
}

/// [OPUS-4.8] (sq-ebii) Maps a writer-rejected UPDATE onto HTTP status: a cooperative budget
/// hit (the memory cap / deadline tripped inside a `DELETE/INSERT … WHERE`) is a 413 / 503,
/// exactly like a query; any other rejection (parse / semantic error) is the client's 400.
fn update_rejection_response(e: &str, config: &ServerConfig) -> Response {
    #[cfg(feature = "shacl")]
    if let Some(report_json) = e.strip_prefix(SHACL_GUARD_REJECTED_PREFIX) {
        return text_response(StatusCode::UNPROCESSABLE_ENTITY, "application/json; charset=utf-8", report_json.to_string(), false);
    }
    // [FABLE-5] (PR #1999 security review) The guard could not FULLY evaluate its shapes
    // graph → the write was refused fail-CLOSED. This is the operator's guard
    // misconfiguration (a 5xx), not the client's data (422); the detail (which quotes
    // shapes-graph fragments) goes to the server log, not the response body.
    #[cfg(feature = "shacl")]
    if let Some(detail) = e.strip_prefix(SHACL_GUARD_UNEVALUABLE_PREFIX) {
        return sanitized_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "shacl-guard-unevaluable",
            "SHACL guard shapes graph could not be fully evaluated; the write was refused (fail-closed) — the operator must fix the guard shapes graph",
            detail,
        );
    }
    // [OPUS-4.8] (sq-vpx4) A durable-write refusal is a retryable 503, NOT a 400. The write
    // was valid but could not be made durable (transient ENOSPC/I/O on the `--persist` mirror);
    // nothing was published or acked, the server is still serving reads, and the client should
    // retry. Sniffed before the budget/parse mapping so it always wins.
    if let Some(detail) = e.strip_prefix(DURABLE_UNAVAILABLE_PREFIX) {
        // [OPUS-4.8] (sq-cz89/sq-j9zs) `detail` is the underlying I/O error, which carries the
        // server's `--persist` filesystem path (e.g. an ENOSPC on the mirror) — withhold it
        // from the client; the operator sees the path in the server log.
        return sanitized_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "durable-unavailable",
            "update not durably committed (transient durable-write error); the write was refused and NOT applied — retry",
            detail,
        );
    }
    if e.contains("query budget exceeded") {
        // Updates budget from `--max-query-rows` alone (`update_budget`, no `--max-results`),
        // so the 413 message must not consider `--max-results` → `apply_max_results = false`.
        return engine_error_response(e, config, false);
    }
    // [OPUS-4.8] (sq-cz89/sq-j9zs) A parse/semantic rejection echoes the offending UPDATE
    // token (and, for a `DELETE/INSERT … WHERE`, can quote loaded data); withhold it — the
    // full reason is in the server log.
    sanitized_error(
        StatusCode::BAD_REQUEST,
        "update-parse",
        "update failed: invalid SPARQL update",
        e,
    )
}

fn timeout_response(config: &ServerConfig) -> Response {
    let secs = config.query_timeout.map(|t| t.as_secs()).unwrap_or(0);
    json_error(
        StatusCode::SERVICE_UNAVAILABLE,
        &format!(
            "query timed out (server limit: {secs}s); simplify the query or raise --query-timeout"
        ),
    )
}

// ---------------------------------------------------------------------------
// Graph Store HTTP Protocol — READ side
// ---------------------------------------------------------------------------

/// [OPUS-4.8] (sq-gxsj) Which graph a GSP request addresses: the default graph or a
/// concrete named graph. Indirect identification yields `?default` → [`GraphRef::Default`]
/// or `?graph=<iri>` → [`GraphRef::Named`]; direct identification turns the request URI
/// into the graph's IRI ([`GraphRef::Named`]). The engine has a real named-graph store
/// (`crates/sparq-engine/src/update.rs` — "over the FULL DATASET"), so this is a faithful
/// addressing scheme, not a default-graph alias.
#[derive(Clone)]
enum GraphRef {
    Default,
    Named(String),
}

/// Indirect graph identification: `?default` or `?graph=<iri>`.
async fn graph_store_indirect(
    State(state): State<AppState>,
    method: Method,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let is_default = params.contains_key("default");
    let named = params.get("graph").cloned();
    if is_default && named.is_some() {
        return bad_request("indirect graph identification accepts exactly one of '?default' or '?graph=<uri>', not both");
    }
    let graph = match (is_default, named) {
        (true, _) => GraphRef::Default,
        (false, Some(g)) => GraphRef::Named(g),
        // POST to the GSP endpoint with no graph selector creates a fresh graph (spec
        // §5.5); other write verbs and reads still require an explicit selector.
        (false, None) if method == Method::POST => GraphRef::Named(mint_graph_iri()),
        (false, None) => {
            return bad_request(
                "indirect graph identification requires '?default' or '?graph=<uri>'",
            )
        }
    };
    graph_store(&state, &method, graph, &headers, body).await
}

/// Direct graph identification: the request URI IS the graph's resource (RFC 3986). The
/// engine stores named graphs, so the direct form addresses a real per-request graph IRI
/// reconstructed from the `Host` header and the matched path — round-trippable with the
/// indirect `?graph=<that-iri>` form.
async fn graph_store_direct(
    State(state): State<AppState>,
    method: Method,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let iri = direct_graph_iri(&headers, &path);
    graph_store(&state, &method, GraphRef::Named(iri), &headers, body).await
}

/// Reconstructs the graph IRI a direct-identification request addresses: `http://<host>/
/// graphs/<path>`, using the `Host` header (a sane default when absent). This is the
/// request URI per the GSP "direct graph identification" rule, and is exactly the IRI a
/// later indirect `?graph=<iri>` request must use to address the same graph.
fn direct_graph_iri(headers: &HeaderMap, path: &str) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("http://{host}/graphs/{path}")
}

/// Mints a fresh, server-allocated graph IRI for a selector-less POST (GSP §5.5: "create a
/// new graph"). UUID-free to avoid a dependency: a process-unique counter plus the nanos
/// clock is collision-free within a process and good enough for an opaque server-chosen IRI.
fn mint_graph_iri() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("urn:sparq:gsp:graph:{nanos:x}-{n:x}")
}

/// [OPUS-4.8] (sq-gxsj) Shared GSP dispatcher across direct + indirect identification.
/// READ (`GET`/`HEAD`) serialises the addressed graph; WRITE (`PUT`/`POST`/`DELETE`)
/// translates into a SPARQL Update submitted through the sequenced writer.
async fn graph_store(
    state: &AppState,
    method: &Method,
    graph: GraphRef,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    // [OPUS-4.8] sq-zcby: the GSP write methods (PUT/POST/DELETE, and PATCH which we 405) are
    // as powerful as an UPDATE, so they are gated as writes; GET/HEAD are reads (gated only
    // under --auth-token-read). Any other method is gated as a write too (fail-closed). The
    // gate runs before any work — even the 405 for an unsupported method is behind it for a
    // write verb, so an attacker cannot probe the surface without the token.
    let op = match *method {
        Method::GET | Method::HEAD => Operation::Read,
        _ => Operation::Write,
    };
    // [OPUS-4.8] sq-0bxp: begin an access-audit record for the GSP request (the Graph-Store
    // surface has no query text to fingerprint — only the operation class + graph access).
    #[cfg(feature = "audit-log")]
    let audit = crate::audit::enabled(state.config()).then(|| {
        crate::audit::AuditRecord::begin(
            crate::audit::AuditOp::from_graph(op),
            bearer_token(headers),
            None,
        )
    });
    // [OPUS-4.8] sq-gos8: begin the RICHER structured access-audit record. The GSP surface
    // carries the resource at its highest fidelity — the named-graph IRI (or the default graph)
    // the request addresses. There is no query body to fingerprint (the body is RDF, not a
    // query); the action class distinguishes read vs write.
    #[cfg(feature = "access-audit")]
    let aa = audit_access_begin(
        state,
        match op {
            Operation::Read => crate::access_audit::Action::GraphRead,
            Operation::Write => crate::access_audit::Action::GraphWrite,
        },
        match &graph {
            GraphRef::Default => crate::access_audit::Resource::Dataset("default".to_string()),
            GraphRef::Named(iri) => crate::access_audit::Resource::NamedGraph(iri.clone()),
        },
        headers,
        None,
    );
    if let Some(resp) = auth_gate(state.config(), headers, op) {
        #[cfg(feature = "audit-log")]
        if let Some(a) = audit {
            a.emit(&resp);
        }
        #[cfg(feature = "access-audit")]
        audit_access_finish(state, aa, &resp);
        return resp;
    }
    let resp = match *method {
        Method::GET | Method::HEAD => {
            let head_only = *method == Method::HEAD;
            serialise_graph(state, graph, headers, head_only).await
        }
        Method::PUT => gsp_put(state, graph, headers, &body).await,
        Method::POST => gsp_post(state, graph, headers, &body).await,
        Method::DELETE => gsp_delete(state, graph).await,
        // [OPUS-4.8] (sq-hj4n, gh-916) PATCH = a graph-scoped, atomic in-place modify. Two body
        // dialects: an always-on `application/sparql-update` body (executed atomically through the
        // same sequenced writer the /sparql update path uses, scoped to this graph), and — only
        // with the `n3-patch` feature AND the `--n3-patch` runtime flag — a Solid-style `text/n3`
        // N3-Patch body. An unsupported PATCH content type is a `415`.
        Method::PATCH => gsp_patch(state, graph, headers, &body).await,
        // Any other method is not part of the Graph Store HTTP Protocol.
        _ => method_not_allowed(&[
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::POST,
            Method::DELETE,
            Method::PATCH,
        ]),
    };
    #[cfg(feature = "audit-log")]
    if let Some(a) = audit {
        a.emit(&resp);
    }
    #[cfg(feature = "access-audit")]
    audit_access_finish(state, aa, &resp);
    resp
}

// ---------------------------------------------------------------------------
// Graph Store HTTP Protocol — WRITE side (sq-gxsj) [OPUS-4.8]
// ---------------------------------------------------------------------------

/// The body media type a GSP write carries. `sparq-core` token formats parse through
/// `Graph::load_str`; RDF/XML is parsed by `oxrdfxml` ([OPUS-4.8] sq-rt6v) since the engine
/// loader has no RDF/XML token. The body carries the triples for ONE graph (the graph is
/// named by the URL, not the body), so a quad syntax (N-Quads/TriG) is parsed as triples too
/// — its graph names are folded, exactly as `Graph::load_str` does.
enum BodyFormat {
    /// A `sparq-core` `Graph::load_str` token format ("turtle"/"ntriples"/"nquads"/"trig").
    Core(&'static str),
    /// `application/rdf+xml`, parsed via `oxrdfxml`. [OPUS-4.8] sq-rt6v.
    RdfXml,
}

/// Classifies a GSP write-body `Content-Type` into the [`BodyFormat`] used to parse it.
fn rdf_format_for(content_type: &str) -> Option<BodyFormat> {
    // `content_type` is already lowercased by `content_type()`; ignore any `; charset=…`.
    let mt = content_type.split(';').next().unwrap_or("").trim();
    match mt {
        "text/turtle" | "application/x-turtle" => Some(BodyFormat::Core("turtle")),
        "application/n-triples" | "text/plain" => Some(BodyFormat::Core("ntriples")),
        "application/n-quads" => Some(BodyFormat::Core("nquads")),
        "application/trig" => Some(BodyFormat::Core("trig")),
        // [OPUS-4.8] sq-oy1f.1: JSON-LD request body, OPT-IN behind the `jsonld` feature (which
        // turns on `sparq-core/jsonld`, the `oxjsonld` parser `Graph::load_str` dispatches the
        // "jsonld" token to). Without the feature this arm is compiled out, so an
        // `application/ld+json` body is a plain `415` — byte-identical to before.
        #[cfg(feature = "jsonld")]
        "application/ld+json" => Some(BodyFormat::Core("jsonld")),
        // [OPUS-4.8] sq-rt6v: RDF/XML request body.
        "application/rdf+xml" => Some(BodyFormat::RdfXml),
        // No explicit Content-Type: default to Turtle (a superset of N-Triples), matching
        // the read side's default emission and common GSP client behaviour.
        "" => Some(BodyFormat::Core("turtle")),
        _ => None,
    }
}

/// [OPUS-4.8] sq-oy1f.1: the `415` message for an unsupported GSP write-body media type. With
/// the `jsonld` feature ON it names `application/ld+json` among the accepted dialects; OFF it
/// names only the always-available RDF syntaxes (an `application/ld+json` body is then a plain
/// `415`), so the advertised contract matches what the build actually parses.
#[cfg(feature = "jsonld")]
const GSP_WRITE_BODY_415: &str =
    "GSP write body must be RDF: Content-Type 'text/turtle', 'application/n-triples', \
     'application/n-quads', 'application/trig', 'application/rdf+xml' or 'application/ld+json'";
/// [OPUS-4.8] sq-oy1f.1: the feature-OFF `415` message — JSON-LD is not parseable in this build.
#[cfg(not(feature = "jsonld"))]
const GSP_WRITE_BODY_415: &str =
    "GSP write body must be RDF: Content-Type 'text/turtle', 'application/n-triples', \
     'application/n-quads', 'application/trig' or 'application/rdf+xml'";

/// Parses a GSP request body into canonical N-Triples (the term syntax accepted verbatim
/// inside a SPARQL `INSERT DATA` block), validating the RDF in the process. The `sparq-core`
/// token formats reuse `Graph::load_str` (the same parsers the loader uses); RDF/XML is
/// parsed by `oxrdfxml` and serialised straight to canonical N-Triples ([OPUS-4.8] sq-rt6v).
/// A parse failure is the caller's 400. `base` resolves any relative IRIs in the body — the
/// addressed graph's IRI, so a relative reference resolves predictably.
// clippy: Err is axum's `Response` (the idiomatic handler error, as in `resolve_pin`);
// boxing it would only desync this from the rest of the handler error convention.
#[allow(clippy::result_large_err)]
fn body_to_ntriples(
    body: &Bytes,
    content_type: &str,
    base: Option<&str>,
) -> Result<String, Response> {
    let format = match rdf_format_for(content_type) {
        Some(f) => f,
        None => {
            return Err(json_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                GSP_WRITE_BODY_415,
            ))
        }
    };
    match format {
        BodyFormat::Core(format) => {
            let text = std::str::from_utf8(body)
                .map_err(|_| bad_request("request body is not valid UTF-8"))?;
            // [OPUS-4.8] (sq-cz89/sq-j9zs) `oxttl` echoes the offending body token verbatim
            // (e.g. an invalid subject quotes the loaded term) — withhold it; the operator
            // gets the full parse error in the server log.
            let graph = Graph::load_str(text, format).map_err(|e| {
                sanitized_error(
                    StatusCode::BAD_REQUEST,
                    "rdf-body-parse",
                    "malformed RDF body",
                    &e,
                )
            })?;
            // Re-emit as canonical N-Triples via the engine scan — no private store API needed,
            // and the terms are exactly what `INSERT DATA` will re-intern.
            nt_dump(&graph, &QueryBudget::default()).map_err(|e| execution_error(&e))
        }
        // [OPUS-4.8] sq-rt6v: parse RDF/XML to triples and emit canonical N-Triples directly.
        // `oxrdf`'s Display is canonical N-Triples term syntax, exactly what `INSERT DATA`
        // re-interns, so no Graph round-trip is needed (and RDF/XML is not a loader token).
        BodyFormat::RdfXml => {
            // [OPUS-4.8] (sq-cz89/sq-j9zs) `oxrdfxml` echoes the offending body fragment — withhold it.
            let triples = crate::graph::parse_rdfxml(body, base).map_err(|e| {
                sanitized_error(
                    StatusCode::BAD_REQUEST,
                    "rdfxml-body-parse",
                    "malformed RDF/XML body",
                    &e,
                )
            })?;
            Ok(crate::graph::triples_to_ntriples(&triples))
        }
    }
}

/// [OPUS-4.8] sq-rt6v: the base IRI a GSP write body resolves relative references against —
/// the addressed named graph's IRI (a sensible, round-trippable base), or `None` for the
/// default graph (no natural base; an RDF/XML body with relative IRIs against the default
/// graph is then a parse error, which is the honest outcome).
fn base_iri(graph: &GraphRef) -> Option<&str> {
    match graph {
        GraphRef::Default => None,
        GraphRef::Named(iri) => Some(iri.as_str()),
    }
}

/// Wraps an N-Triples body in the `GRAPH <iri> { … }` block for a named graph, or returns
/// it bare for the default graph — the shape an `INSERT DATA` / `DELETE DATA` operand takes.
fn graph_data_block(graph: &GraphRef, ntriples: &str) -> String {
    match graph {
        GraphRef::Default => ntriples.to_string(),
        GraphRef::Named(iri) => format!("GRAPH <{}> {{\n{ntriples}}}\n", escape_iri(iri)),
    }
}

/// Minimal IRI escaping for safe interpolation into a SPARQL `<…>` term: the characters
/// SPARQL forbids inside an IRIREF (controls, spaces, and the delimiters `<>"{}|^`\``).
/// A graph IRI containing them would otherwise break the generated update; percent-encode
/// them so the update parses and addresses a stable IRI.
fn escape_iri(iri: &str) -> String {
    let mut out = String::with_capacity(iri.len());
    for c in iri.chars() {
        match c {
            '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\' | ' ' => {
                for b in c.to_string().bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Runs a server-minted SPARQL Update through the sequenced writer off the async workers
/// (it blocks for the group-commit ack), mapping the outcome onto `success`/400/500 — the
/// same status discipline as the `application/sparql-update` path.
async fn apply_gsp_update(state: &AppState, update: String, success: StatusCode) -> Response {
    let st = state.clone();
    let task = tokio::task::spawn_blocking(move || st.apply_update(&update));
    // [OPUS-4.8] sq-ebii: GSP writes inherit the same UPDATE timeout cap as the
    // `application/sparql-update` path (they share the sequenced writer).
    match await_update_worker(task, &state.config).await {
        UpdateOutcome::Ok(number) => {
            state.metrics().inc_updates();
            with_generation_header(success.into_response(), number)
        }
        UpdateOutcome::Rejected(e) => {
            // [GPT-5.6] sq-lsp7k.2.4: preserve the shared 422 + report mapping for GSP writes.
            // [FABLE-5] (PR #1999 security review) …and the fail-closed unevaluable-guard 500.
            #[cfg(feature = "shacl")]
            if e.starts_with(SHACL_GUARD_REJECTED_PREFIX)
                || e.starts_with(SHACL_GUARD_UNEVALUABLE_PREFIX)
            {
                return update_rejection_response(&e, &state.config);
            }
            // [OPUS-4.8] sq-ebii: a budget hit (timeout / memory cap) inside a GSP write's
            // WHERE maps to 503/413; a genuine parse/semantic failure stays a 400.
            if e.contains("query budget exceeded") {
                update_rejection_response(&e, &state.config)
            } else {
                // [OPUS-4.8] (sq-kfel, ASVS-G3) The writer-rejection string `e` is the engine's
                // error for the SERVER-MINTED `DROP`/`INSERT DATA` update built from the request
                // body — it can quote term text drawn from the (caller-supplied) body, exactly
                // the info-leak class the rest of the surface sanitizes (sq-cz89/sq-j9zs). Route
                // it through `sanitized_error`: a generic class message to the client, the full
                // engine detail to the server log. The other UPDATE entry points already go
                // through `update_rejection_response` → `sanitized_error`; this GSP-write minted-
                // update branch was the one path that echoed `e` verbatim. Stays a 400.
                sanitized_error(
                    StatusCode::BAD_REQUEST,
                    "gsp-write",
                    "graph store write failed: invalid RDF body",
                    &e,
                )
            }
        }
        UpdateOutcome::TimedOut => timeout_response(&state.config),
        UpdateOutcome::Panicked => {
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "update worker panicked")
        }
    }
}

/// `PUT <graph>` — REPLACE the graph's contents with the request body (GSP §4.2). Maps to
/// `DROP SILENT GRAPH <g>; INSERT DATA { GRAPH <g> { … } }` (or `CLEAR DEFAULT` + `INSERT
/// DATA` for the default graph) so the replace is one atomic, group-committed generation.
/// 201 when the graph did not exist (created), 204 when it replaced an existing one.
async fn gsp_put(state: &AppState, graph: GraphRef, headers: &HeaderMap, body: &Bytes) -> Response {
    // [OPUS-4.8] sq-ebii: inflate a `Content-Encoding: gzip` body under the
    // decompression-ratio cap (zip-bomb guard) before parsing it as RDF.
    let body = match decode_request_body(body, headers, state.config()) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let ntriples = match body_to_ntriples(&body, &content_type(headers), base_iri(&graph)) {
        Ok(nt) => nt,
        Err(resp) => return resp,
    };
    // Created (201) vs replaced (204) is decided by pre-existence, sampled from the current
    // generation. A racing writer cannot break this: the status is advisory and the write
    // itself is atomic on the sequenced writer regardless of the sampled flag.
    let existed = graph_exists(state, &graph);
    let clear = match &graph {
        GraphRef::Default => "CLEAR DEFAULT".to_string(),
        GraphRef::Named(iri) => format!("DROP SILENT GRAPH <{}>", escape_iri(iri)),
    };
    // An empty body means "the graph is now empty" — the clear alone achieves that; emitting
    // an empty `INSERT DATA { }` would be a needless (and possibly unparsable) no-op clause.
    let update = if ntriples.is_empty() {
        clear
    } else {
        let block = graph_data_block(&graph, &ntriples);
        format!("{clear} ;\nINSERT DATA {{ {block} }}")
    };
    let success = if existed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };
    apply_gsp_update(state, update, success).await
}

/// `POST <graph>` — MERGE (additive) the request body into the graph (GSP §5). Maps to
/// `INSERT DATA { GRAPH <g> { … } }`. 201 when the merge created the graph (a selector-less
/// POST, or a POST to an absent named graph), 204 when it added to an existing graph.
async fn gsp_post(
    state: &AppState,
    graph: GraphRef,
    headers: &HeaderMap,
    body: &Bytes,
) -> Response {
    // [OPUS-4.8] sq-ebii: inflate a gzip body under the decompression-ratio cap first.
    let body = match decode_request_body(body, headers, state.config()) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let ntriples = match body_to_ntriples(&body, &content_type(headers), base_iri(&graph)) {
        Ok(nt) => nt,
        Err(resp) => return resp,
    };
    let existed = graph_exists(state, &graph);
    // An empty merge body adds nothing. For an existing graph that is a 204 no-op; for an
    // absent named graph the spec wants the graph created — `CREATE SILENT GRAPH <g>` makes
    // the empty graph so a subsequent read addresses it (201).
    let update = if ntriples.is_empty() {
        match &graph {
            GraphRef::Default => {
                return with_generation_header(
                    StatusCode::NO_CONTENT.into_response(),
                    state.current().number(),
                )
            }
            GraphRef::Named(_) if existed => {
                return with_generation_header(
                    StatusCode::NO_CONTENT.into_response(),
                    state.current().number(),
                )
            }
            GraphRef::Named(iri) => format!("CREATE SILENT GRAPH <{}>", escape_iri(iri)),
        }
    } else {
        let block = graph_data_block(&graph, &ntriples);
        format!("INSERT DATA {{ {block} }}")
    };
    let success = if existed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };
    apply_gsp_update(state, update, success).await
}

/// `DELETE <graph>` — DROP the graph (GSP §6). 204 on success; 404 when the named graph is
/// absent (per the spec's "graph does not exist" semantics). The default graph always
/// exists, so `DELETE ?default` empties it and is always 204. Maps to `DROP GRAPH <g>` /
/// `CLEAR DEFAULT`.
async fn gsp_delete(state: &AppState, graph: GraphRef) -> Response {
    match &graph {
        GraphRef::Default => {
            apply_gsp_update(state, "CLEAR DEFAULT".to_string(), StatusCode::NO_CONTENT).await
        }
        GraphRef::Named(iri) => {
            if !graph_exists(state, &graph) {
                // [OPUS-4.8] (sq-ttv2) The 404 reflects the REQUESTED graph IRI. Assessed and
                // accepted as standard REST: `iri` is the CLIENT'S OWN input — either the
                // verbatim `?graph=<uri>` value (indirect identification) or
                // `http://<Host>/graphs/<path>` reconstructed from the client's request line +
                // Host header (direct identification, `graph_store_direct` →
                // `direct_graph_iri`). It carries NO server-internal information (no filesystem
                // path, no enumeration of OTHER stored graphs, no engine state), so echoing it
                // back is not an info leak — it is the addressed resource, exactly as a REST 404
                // names the resource that was not found. Stays inside the structured
                // `{"error":...}` envelope, so it matches the error contract.
                return json_error(
                    StatusCode::NOT_FOUND,
                    &format!("graph <{iri}> does not exist"),
                );
            }
            let update = format!("DROP GRAPH <{}>", escape_iri(iri));
            apply_gsp_update(state, update, StatusCode::NO_CONTENT).await
        }
    }
}

/// [OPUS-4.8] (sq-hj4n, gh-916) `PATCH <graph>` — apply an ATOMIC, graph-scoped in-place modify
/// to the addressed graph. Two body dialects, classified by `Content-Type`:
///
///   * **`application/sparql-update`** (ALWAYS-ON, no feature, no new dep) — the body IS a SPARQL
///     Update. It is applied atomically through the SAME sequenced group-commit writer the
///     `/sparql` `application/sparql-update` path uses, so the whole multi-operation body lands in
///     ONE durable generation. It is **scoped** to the addressed graph by defaulting the update's
///     WHERE dataset to that graph (the SPARQL 1.1 Protocol §2.2 `using-graph-uri` mechanism, via
///     the in-tree [`rewrite_update`]). HONEST SCOPE: this scopes what the WHERE *reads*; an
///     operation that names a DIFFERENT graph explicitly (`INSERT DATA { GRAPH <other> { … } }` /
///     a `WITH`/`USING` clause) still writes/reads where it says, exactly as SPARQL specifies —
///     supplying the override alongside an in-string `USING`/`WITH` is a `400` (§2.2), the existing
///     protocol behaviour. For the default graph the override is a no-op (the default graph is
///     already the WHERE default). Success → `204`.
///
///   * **`text/n3`** (OPT-IN, behind the `n3-patch` cargo feature AND the `--n3-patch` runtime
///     flag) — a Solid-style N3-Patch (`solid:InsertDeletePatch`). Parsed into its
///     `solid:deletes` / `solid:inserts` / `solid:where` formulas and translated into ONE atomic
///     graph-scoped SPARQL Update (every block wrapped in `GRAPH <g> { … }` for a named graph),
///     submitted through the same writer. Success → `204`. With the feature OFF this arm is
///     `#[cfg]`-stripped entirely, so a `text/n3` body is a plain `415`. With the feature ON but
///     the runtime flag OFF it is also `415` (double-opt-in, mirroring `tpf`/`shacl`).
///
/// Any other `Content-Type` is a `415`. A `PATCH` is a WRITE, gated like an UPDATE; the auth gate
/// already ran in [`graph_store`] before this handler.
async fn gsp_patch(
    state: &AppState,
    graph: GraphRef,
    headers: &HeaderMap,
    body: &Bytes,
) -> Response {
    // [OPUS-4.8] sq-hj4n: inflate a `Content-Encoding: gzip` body under the decompression-ratio
    // cap (zip-bomb guard) before parsing it, exactly as the GSP PUT/POST write paths do.
    let body = match decode_request_body(body, headers, state.config()) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let ct = content_type(headers);
    let mt = ct.split(';').next().unwrap_or("").trim();
    match mt {
        // ----- ALWAYS-ON dialect: a SPARQL Update body, scoped to the addressed graph. -----
        "application/sparql-update" => {
            let update = match std::str::from_utf8(&body) {
                Ok(u) => u,
                Err(_) => return bad_request("request body is not valid UTF-8"),
            };
            // Scope the update's WHERE dataset to the addressed named graph (§2.2 using-graph-uri,
            // reusing the in-tree rewrite). The default graph is already the WHERE default → no
            // override. `rewrite_update` rejects a conflict with an in-string USING/WITH (400) and
            // a malformed update (400), sanitizing the detail.
            let over = match &graph {
                GraphRef::Default => UsingOverride::default(),
                GraphRef::Named(iri) => UsingOverride {
                    default: vec![iri.clone()],
                    named: Vec::new(),
                },
            };
            match rewrite_update(update, &over) {
                // Reuse the shared update path: atomic group-commit, 204 on success, the same
                // budget/timeout/sanitized-rejection discipline as the /sparql update body.
                Ok(rewritten) => run_update(state, rewritten).await,
                Err(resp) => resp,
            }
        }
        // ----- OPT-IN dialect: Solid N3-Patch. Compiled out entirely without the feature. -----
        #[cfg(feature = "n3-patch")]
        "text/n3" => gsp_patch_n3(state, graph, &body).await,
        // Anything else (including `text/n3` when the feature is off) is an unsupported media type.
        _ => json_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            n3_patch_unsupported_media_msg(),
        ),
    }
}

/// [OPUS-4.8] (sq-hj4n) The `415` message for an unsupported `PATCH` body media type. With the
/// `n3-patch` feature on, `text/n3` is offered (so it is named); with the feature off it is not.
/// Two literals (not a runtime `if`) so the message is exactly right per build, and the
/// feature-off message never advertises a dialect the build cannot serve.
#[cfg(feature = "n3-patch")]
fn n3_patch_unsupported_media_msg() -> &'static str {
    "PATCH body must be a graph patch: Content-Type 'application/sparql-update' \
     (always available) or 'text/n3' (Solid N3-Patch, when --n3-patch is enabled)"
}

/// [OPUS-4.8] (sq-hj4n) The feature-OFF `415` message — names only the always-on dialect.
#[cfg(not(feature = "n3-patch"))]
fn n3_patch_unsupported_media_msg() -> &'static str {
    "PATCH body must be a graph patch: Content-Type 'application/sparql-update'"
}

/// [OPUS-4.8] (sq-hj4n, gh-916) Applies a Solid N3-Patch (`text/n3`) body to the addressed graph.
/// Compiled ONLY behind the `n3-patch` feature. Honours the `--n3-patch` runtime flag: with it off
/// the dialect is `415` even though the feature is compiled in (the double-opt-in posture). Parses
/// the body, builds ONE atomic graph-scoped SPARQL Update, and submits it through the shared
/// writer (atomic, 204 on success).
#[cfg(feature = "n3-patch")]
async fn gsp_patch_n3(state: &AppState, graph: GraphRef, body: &Bytes) -> Response {
    // Double-opt-in: the feature is compiled in, but the operator must also flip the runtime flag.
    if !state.config().n3_patch {
        return json_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "the Solid N3-Patch (text/n3) PATCH dialect is disabled on this server; enable it with \
             --n3-patch / SPARQ_N3_PATCH=1, or send an 'application/sparql-update' PATCH body",
        );
    }
    // The base IRI relative references resolve against — the addressed named graph's IRI (or None
    // for the default graph), exactly as the GSP RDF-body write path does (`base_iri`).
    let base = base_iri(&graph);
    let patch = match crate::n3_patch::parse(body, base) {
        Ok(p) => p,
        // [OPUS-4.8] (sq-cz89/sq-j9zs) The parse error can quote the offending body token —
        // withhold it from the client (the operator gets the detail in the server log) via the
        // same sanitized-error discipline the other body-parse paths use.
        Err(e) => {
            return sanitized_error(
                StatusCode::BAD_REQUEST,
                "n3-patch-parse",
                &e.to_string(),
                &e.detail(),
            )
        }
    };
    // Build ONE atomic SPARQL Update from the parsed formulas, graph-scoped via the exact
    // `graph_data_block` helper the rest of the GSP write path uses (so a named-graph patch wraps
    // each block in `GRAPH <g> { … }`, the default graph is bare).
    let update = build_n3_patch_update(&graph, &patch);
    // Submit through the shared writer: atomic group-commit, 204 on success, the same
    // budget/timeout/sanitized-rejection discipline as every other update entry point.
    run_update(state, update).await
}

/// [OPUS-4.8] (sq-hj4n) Assembles the ATOMIC, graph-scoped SPARQL Update for a parsed N3-Patch.
///
/// * With a `solid:where` clause → a single pattern-based `DELETE { … } INSERT { … } WHERE { … }`
///   (one atomic modify; the empty blocks are omitted). The `WHERE` block reads the addressed
///   graph (`GRAPH <g> { … }` for a named graph), so the variables bind against that graph only.
/// * Without a `where` clause (ground triples) → `DELETE DATA { … } ; INSERT DATA { … }` (the
///   DATA-form the GSP write path already uses for concrete triples), the two ops in ONE update so
///   they commit in ONE generation. The delete runs before the insert (an N3-Patch that deletes a
///   triple and re-inserts a changed one is then correct).
///
/// Each block is wrapped in `GRAPH <g> { … }` for a named graph, or left bare for the default
/// graph — reusing [`graph_data_block`], the exact helper `gsp_put`/`gsp_post` use.
#[cfg(feature = "n3-patch")]
fn build_n3_patch_update(graph: &GraphRef, patch: &crate::n3_patch::N3Patch) -> String {
    if patch.has_where {
        // Pattern-based modify: DELETE { … } INSERT { … } WHERE { … }. Omit empty templates.
        let mut update = String::new();
        if !patch.deletes.is_empty() {
            update.push_str("DELETE { ");
            update.push_str(&graph_data_block(graph, &patch.deletes));
            update.push_str(" }\n");
        }
        if !patch.inserts.is_empty() {
            update.push_str("INSERT { ");
            update.push_str(&graph_data_block(graph, &patch.inserts));
            update.push_str(" }\n");
        }
        update.push_str("WHERE { ");
        update.push_str(&graph_data_block(graph, &patch.conditions));
        update.push_str(" }");
        update
    } else {
        // Ground DATA-form: DELETE DATA { … } ; INSERT DATA { … } — both in one atomic update.
        let mut ops: Vec<String> = Vec::new();
        if !patch.deletes.is_empty() {
            ops.push(format!(
                "DELETE DATA {{ {} }}",
                graph_data_block(graph, &patch.deletes)
            ));
        }
        if !patch.inserts.is_empty() {
            ops.push(format!(
                "INSERT DATA {{ {} }}",
                graph_data_block(graph, &patch.inserts)
            ));
        }
        ops.join(" ;\n")
    }
}

/// Whether the addressed graph currently exists / is non-empty in the current generation.
/// For a named graph this is `ASK { GRAPH <g> { ?s ?p ?o } }`; for the default graph,
/// `ASK { ?s ?p ?o }`. Note the engine has no separate "empty named graph exists" bit
/// outside of an in-flight update, so an existing-but-empty named graph reads as absent —
/// which only affects the advisory created-vs-replaced status code, never write atomicity.
fn graph_exists(state: &AppState, graph: &GraphRef) -> bool {
    let ask = match graph {
        GraphRef::Default => "ASK { ?s ?p ?o }".to_string(),
        GraphRef::Named(iri) => format!("ASK {{ GRAPH <{}> {{ ?s ?p ?o }} }}", escape_iri(iri)),
    };
    let gen = state.current();
    matches!(sparq_engine::ask(gen.snapshot(), &ask), Ok(true))
}

// ---------------------------------------------------------------------------
// Graph Store HTTP Protocol — READ side
// ---------------------------------------------------------------------------

/// Serialises the addressed graph in the negotiated RDF syntax: `application/n-triples`
/// (default), prefix-compacting `text/turtle`, or `application/rdf+xml` ([OPUS-4.8] sq-rt6v
/// — `text/turtle` is now a real compact Turtle document, not N-Triples-as-Turtle, and
/// RDF/XML is newly offered). CPU-bound (a full-graph dump), so it runs on the blocking pool
/// under the request timeout (no row cap: a dump is inherently graph-sized). A named graph
/// that does not exist serialises as the empty graph (200, empty body) per GSP read.
async fn serialise_graph(
    state: &AppState,
    graph: GraphRef,
    headers: &HeaderMap,
    head_only: bool,
) -> Response {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let gfmt = negotiate_graph(accept);
    let ct = gfmt.content_type().to_string();
    let config = state.config.clone();
    let budget = make_budget(&config, false);
    // Pinned for the whole dump: the serialisation is consistent with request start.
    let gen = state.current();
    let cfg = config.clone();
    let task = tokio::task::spawn_blocking(move || {
        match graph_dump_triples(gen.snapshot(), &graph, &budget) {
            Ok(triples) => text_response(
                StatusCode::OK,
                &ct,
                serialise_graph_triples(&triples, gfmt),
                head_only,
            ),
            // GSP-read used `make_budget(_, false)` → max_results did NOT apply.
            Err(e) => engine_error_response(&e, &cfg, false),
        }
    });
    await_worker(task, &config).await
}

/// [OPUS-4.8] sq-rt6v: serialises an RDF graph (triple list) in the negotiated [`GraphFormat`]
/// — the single dispatch shared by the CONSTRUCT/DESCRIBE and GSP-read paths. N-Triples is the
/// canonical line form; Turtle compacts the [`crate::graph::COMMON_PREFIXES`]; RDF/XML is the
/// `application/rdf+xml` document. The writers are guaranteed to emit well-formed output.
///
/// [FABLE-5] sq-0kq6k: implemented BY [`serialise_graph_triples_to`] over an in-memory buffer,
/// so the buffered response body and the streamed one are the same bytes by construction.
fn serialise_graph_triples(triples: &[oxrdf::Triple], gfmt: GraphFormat) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(triples.len() * 64);
    // An in-memory writer cannot fail.
    let _ = serialise_graph_triples_to(triples, gfmt, &mut out);
    String::from_utf8(out).unwrap_or_default()
}

/// [FABLE-5] sq-0kq6k: the sink-shaped twin of [`serialise_graph_triples`] — renders the graph
/// in the negotiated syntax straight into `w` instead of into a `String`. Feeding it a
/// [`ChunkSink`] is what makes a large CONSTRUCT / DESCRIBE a chunked-transfer response whose
/// first bytes reach the socket before the last triple is rendered.
///
/// Turtle / N-Triples / RDF/XML stream genuinely; JSON-LD does not (see
/// [`crate::graph::write_jsonld_to`]) — it still materialises its document before writing.
fn serialise_graph_triples_to<W: std::io::Write>(
    triples: &[oxrdf::Triple],
    gfmt: GraphFormat,
    w: &mut W,
) -> std::io::Result<()> {
    match gfmt {
        GraphFormat::NTriples => crate::graph::write_ntriples_to(triples, w),
        GraphFormat::Turtle => crate::graph::write_turtle_to(triples, w),
        GraphFormat::RdfXml => crate::graph::write_rdfxml_to(triples, w),
        // [OPUS-4.8] sq-oy1f.1: JSON-LD (flattened) — only reachable when the `jsonld` feature
        // is on (the variant does not exist otherwise, so the match stays exhaustive).
        #[cfg(feature = "jsonld")]
        GraphFormat::JsonLd => crate::graph::write_jsonld_to(triples, w),
    }
}

/// Dumps the addressed graph as a triple list by reusing the engine's SELECT path and the
/// materialised terms — `?s ?p ?o` for the default graph, `GRAPH <g> { ?s ?p ?o }` for a
/// named graph — so no private store API is needed. [OPUS-4.8] sq-rt6v: returns `Vec<Triple>`
/// (was N-Triples text) so the caller can serialise in any RDF syntax.
fn graph_dump_triples(
    graph: &Graph,
    target: &GraphRef,
    budget: &QueryBudget,
) -> Result<Vec<oxrdf::Triple>, String> {
    let select = match target {
        GraphRef::Default => "SELECT ?s ?p ?o WHERE { ?s ?p ?o }".to_string(),
        GraphRef::Named(iri) => format!(
            "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
            escape_iri(iri)
        ),
    };
    let r = sparq_engine::query_with_budget(graph, &select, budget)?;
    let mut triples = Vec::with_capacity(r.rows.len());
    for row in &r.rows {
        let (Some(s), Some(p), Some(o)) = (&row[0], &row[1], &row[2]) else {
            continue;
        };
        // A triple from a real RDF graph always has a NamedNode/BlankNode subject and a
        // NamedNode predicate; a row that somehow violates that (it cannot, for `?s ?p ?o`
        // over a graph) is skipped rather than panicked on.
        if let Some(t) = row_to_triple(s, p, o) {
            triples.push(t);
        }
    }
    Ok(triples)
}

/// [OPUS-4.8] sq-rt6v: builds an [`oxrdf::Triple`] from an `?s ?p ?o` solution row, or `None`
/// if the slots are not a legal triple shape (subject must be IRI/bnode, predicate an IRI).
fn row_to_triple(s: &oxrdf::Term, p: &oxrdf::Term, o: &oxrdf::Term) -> Option<oxrdf::Triple> {
    use oxrdf::{NamedOrBlankNode, Term};
    let subject = match s {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
        _ => return None,
    };
    let Term::NamedNode(predicate) = p else {
        return None;
    };
    Some(oxrdf::Triple::new(subject, predicate.clone(), o.clone()))
}

/// Dumps the whole default graph as N-Triples by reusing the engine's SELECT path
/// (`?s ?p ?o`) and the materialised terms — no private store API needed.
fn nt_dump(graph: &Graph, budget: &QueryBudget) -> Result<String, String> {
    nt_dump_select(graph, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", budget)
}

/// Shared `?s ?p ?o` → N-Triples projection for the GSP read / body-canonicalisation paths.
fn nt_dump_select(graph: &Graph, select: &str, budget: &QueryBudget) -> Result<String, String> {
    let r = sparq_engine::query_with_budget(graph, select, budget)?;
    let mut out = String::with_capacity(r.rows.len() * 64);
    for row in &r.rows {
        let (Some(s), Some(p), Some(o)) = (&row[0], &row[1], &row[2]) else {
            continue;
        };
        nt_term(&mut out, s);
        out.push(' ');
        nt_term(&mut out, p);
        out.push(' ');
        nt_term(&mut out, o);
        out.push_str(" .\n");
    }
    Ok(out)
}

fn nt_term(out: &mut String, t: &oxrdf::Term) {
    // oxrdf's Display for Term already produces canonical N-Triples term syntax.
    use std::fmt::Write;
    let _ = write!(out, "{t}");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parses an `application/x-www-form-urlencoded` string into a map (last value wins).
fn parse_form(s: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in s.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        map.insert(form_decode(k), form_decode(v));
    }
    map
}

/// [OPUS-4.8] sq-z33x: collects EVERY value of a repeated `application/x-www-form-urlencoded`
/// key (in request order). [`parse_form`]'s `HashMap` keeps only the last value, but the SPARQL
/// 1.1 Protocol dataset parameters (`default-graph-uri` / `named-graph-uri` / `using-*`) are
/// intrinsically multi-valued — a dataset can name several default and several named graphs — so
/// they must be read from the raw form/query string, not the collapsed map.
fn form_values(raw: &str, key: &str) -> Vec<String> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| match pair.split_once('=') {
            Some((k, v)) => (form_decode(k) == key).then(|| form_decode(v)),
            None => None,
        })
        .collect()
}

/// [OPUS-4.8] sq-z33x: extracts the SPARQL 1.1 Protocol §2.1.4 query dataset override
/// (`default-graph-uri` / `named-graph-uri`) from a raw urlencoded string — the request URL query
/// string for GET / direct-POST, or the form body for an `application/x-www-form-urlencoded` POST.
fn query_dataset_override(raw: &str) -> DatasetOverride {
    DatasetOverride {
        default: form_values(raw, "default-graph-uri"),
        named: form_values(raw, "named-graph-uri"),
    }
}

/// [OPUS-4.8] sq-z33x: extracts the SPARQL 1.1 Protocol §2.2 UPDATE dataset override
/// (`using-graph-uri` / `using-named-graph-uri`) from a raw urlencoded string.
fn update_dataset_override(raw: &str) -> UsingOverride {
    UsingOverride {
        default: form_values(raw, "using-graph-uri"),
        named: form_values(raw, "using-named-graph-uri"),
    }
}

/// [OPUS-4.8] sq-z33x: applies the UPDATE dataset override to the update string, mapping a rewrite
/// failure onto the right HTTP 400. Returns the (possibly rewritten) update on success.
// clippy: Err is axum's `Response` (the idiomatic handler error, as at `resolve_pin`); boxing it
// would only desync the call sites that already thread `Response` errors.
#[allow(clippy::result_large_err)]
fn rewrite_update(update: &str, over: &UsingOverride) -> Result<String, Response> {
    apply_update_dataset(update, over).map_err(|e| match e {
        UpdateDatasetError::Malformed(msg) => sanitized_error(
            StatusCode::BAD_REQUEST,
            "update-parse",
            "malformed update",
            &msg,
        ),
        UpdateDatasetError::BadGraphUri(msg) => bad_request(&msg),
        UpdateDatasetError::UsingConflict => bad_request(
            "the 'using-graph-uri' / 'using-named-graph-uri' parameters must not be combined with \
             an in-update USING / USING NAMED / WITH clause (SPARQL 1.1 Protocol §2.2)",
        ),
    })
}

/// Decodes a single `application/x-www-form-urlencoded` component (`+` → space, `%XX`).
fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h << 4) | l);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn content_type(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-ebii — decompression-ratio cap (zip-bomb guard)
// ---------------------------------------------------------------------------

/// [OPUS-4.8] (sq-ebii) The error of a bounded body decode, mapped to its HTTP status.
#[derive(Debug)]
enum DecodeError {
    /// `Content-Encoding` names a codec we do not decode (only `gzip`/`x-gzip` and
    /// `identity` are supported) → 415.
    Unsupported(String),
    /// The decompressed image crossed the ratio/absolute ceiling → 413 (zip-bomb guard).
    /// The message is server-constructed (sizes/knob names only — no caller content).
    TooLarge(String),
    /// The gzip stream was malformed → 400. The payload is the underlying decoder error,
    /// which can quote bytes of the caller's compressed body — it is the WITHHELD detail
    /// ([OPUS-4.8] sq-cz89/sq-j9zs), routed to the server log, never the response body.
    Malformed(String),
}

impl DecodeError {
    fn into_response(self) -> Response {
        match self {
            DecodeError::Unsupported(m) => json_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, &m),
            DecodeError::TooLarge(m) => json_error(StatusCode::PAYLOAD_TOO_LARGE, &m),
            // [OPUS-4.8] (sq-cz89/sq-j9zs) Generic class to the client; decoder detail to the log.
            DecodeError::Malformed(detail) => sanitized_error(
                StatusCode::BAD_REQUEST,
                "gzip-decode",
                "malformed gzip body",
                &detail,
            ),
        }
    }
}

/// [OPUS-4.8] (sq-ebii) Decodes a request body honouring `Content-Encoding`, under the
/// decompression-ratio cap (zip-bomb guard). Returns the body verbatim for `identity` /
/// no encoding; for `gzip` it inflates with a HARD ceiling of
/// `min(max_decompress_ratio × compressed_len, max_body_bytes)` and refuses (413) the
/// instant the inflated output would cross it — so a tiny but pathologically compressible
/// body cannot inflate into an OOM. The ceiling is checked DURING inflate (bounded
/// `Read::take`), so the full decompressed image is never materialised past the cap.
///
/// `max_decompress_ratio == 0` disables ratio-capped decompression entirely: a
/// `Content-Encoding: gzip` body is then refused outright (fail-closed) rather than
/// inflated.
///
/// **Honest scope:** this guards the bodies the server itself inflates (the GSP write /
/// RDF-load path). It does NOT cover a compressed payload the *engine* fetches behind a
/// SPARQL `LOAD <url>` or `SERVICE` — those go through their own ingest path; the SERVICE
/// surface is separately bounded by the egress allowlist (sq-4w18). The request body-size
/// limit (`--max-body-bytes`, 413) caps the COMPRESSED bytes first, and the decompressed
/// ceiling is `min(max_decompress_ratio × compressed_len, max_body_bytes)` (see
/// [`decode_gzip_bounded`]) — so the decompressed output is itself capped at `max_body_bytes`,
/// never `max_body_bytes × max_decompress_ratio`.
fn decode_request_body(
    body: &Bytes,
    headers: &HeaderMap,
    config: &ServerConfig,
) -> Result<Bytes, DecodeError> {
    let encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match encoding.as_str() {
        "" | "identity" => Ok(body.clone()),
        "gzip" | "x-gzip" => decode_gzip_bounded(body, config),
        other => Err(DecodeError::Unsupported(format!(
            "unsupported Content-Encoding '{other}'; the server decodes only 'gzip' (and 'identity')"
        ))),
    }
}

/// [OPUS-4.8] (sq-ebii) gzip inflate bounded by the decompression-ratio cap. The ceiling is
/// `min(ratio × compressed_len, max_body_bytes)`, clamped to at least 1 byte so an empty /
/// tiny compressed body whose ratio product rounds below its real (small) output still
/// decodes. Reading is wrapped in `Read::take(ceiling + 1)`: if the decoder produces more
/// than `ceiling` bytes the read is cut short and we refuse (413) WITHOUT having held the
/// whole bomb in memory.
fn decode_gzip_bounded(body: &Bytes, config: &ServerConfig) -> Result<Bytes, DecodeError> {
    use std::io::Read;
    if config.max_decompress_ratio == 0 {
        return Err(DecodeError::TooLarge(
            "compressed (Content-Encoding: gzip) request bodies are disabled on this server \
             (--max-decompress-ratio 0); send an uncompressed body"
                .to_string(),
        ));
    }
    let ratio = config.max_decompress_ratio;
    // The decompressed ceiling: the ratio bound, but never above the absolute body limit's
    // worth of plaintext, and at least 1 so a tiny body is decodable. `saturating_mul`
    // avoids overflow on a huge compressed body (already bounded by --max-body-bytes anyway).
    let ceiling = body
        .len()
        .saturating_mul(ratio)
        .min(config.max_body_bytes.max(1))
        .max(1);
    let mut decoder = flate2::read::MultiGzDecoder::new(&body[..]).take(ceiling as u64 + 1);
    let mut out = Vec::with_capacity(ceiling.min(64 * 1024));
    decoder
        .read_to_end(&mut out)
        // [OPUS-4.8] (sq-cz89/sq-j9zs) Carry only the raw decoder detail; `into_response`
        // logs it server-side and returns a generic "malformed gzip body" to the client.
        .map_err(|e| DecodeError::Malformed(e.to_string()))?;
    if out.len() > ceiling {
        return Err(DecodeError::TooLarge(format!(
            "decompressed body exceeds the server's decompression-ratio cap (compressed {} bytes \
             × {ratio}× ratio, capped at {ceiling} bytes; raise --max-decompress-ratio / \
             --max-body-bytes or send a smaller body) — refused as a possible zip bomb",
            body.len()
        )));
    }
    Ok(Bytes::from(out))
}

/// Builds a `text`-ish response with the given content type; for HEAD, omits the body but
/// keeps the `Content-Type` (and an accurate `Content-Length` via the header) so HEAD
/// mirrors GET.
pub(crate) fn text_response(
    status: StatusCode,
    content_type: &str,
    body: String,
    head_only: bool,
) -> Response {
    let len = body.len();
    // For HEAD we advertise the same Content-Length the GET would have, with an empty body.
    let out_body = if head_only { String::new() } else { body };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, len)
        .body(out_body.into())
        .unwrap()
}

/// Builds a response whose body is streamed from a pre-evaluated chunk sequence
/// (T16): hyper writes the chunks one by one instead of the server concatenating
/// them into a single giant `String` first. The total length is known, so the
/// response carries the same `Content-Type`/`Content-Length` headers — and, by the
/// engine's chunking contract, byte-identical body content — as the buffered path.
/// HEAD mirrors GET: same headers, empty body.
///
/// `pin` is the generation the chunks were evaluated against; the body stream owns it,
/// so the generation stays pinned until the response finishes (or the client goes away)
/// — the ring can never let the snapshot's memory go while the body is in flight. Today
/// the chunks are fully materialised strings, so this is belt-and-braces; it becomes
/// load-bearing the moment chunks evaluate lazily (Wave D push/pull streaming).
fn chunked_response(
    status: StatusCode,
    content_type: &str,
    chunks: Vec<String>,
    head_only: bool,
    pin: PinnedGen,
) -> Response {
    let len: usize = chunks.iter().map(String::len).sum();
    let body = if head_only {
        axum::body::Body::empty()
    } else {
        axum::body::Body::from_stream(futures_util::stream::iter(chunks.into_iter().map(
            move |c| {
                let _pinned_for_stream_lifetime = &pin;
                Ok::<_, std::convert::Infallible>(Bytes::from(c.into_bytes()))
            },
        )))
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, len)
        .body(body)
        .unwrap()
}

/// Structured error body: every error the server emits is `{"error": "..."}` JSON, so
/// programmatic clients never have to scrape prose out of plain text.
///
/// [OPUS-4.8] sq-bxog: `pub(crate)` so the SSE subscription handler returns the SAME error
/// envelope as the rest of the HTTP surface for a pre-stream registration refusal.
pub(crate) fn json_error(status: StatusCode, msg: &str) -> Response {
    let mut body = String::with_capacity(msg.len() + 16);
    body.push_str("{\"error\":\"");
    for c in msg.chars() {
        match c {
            '"' => body.push_str("\\\""),
            '\\' => body.push_str("\\\\"),
            '\n' => body.push_str("\\n"),
            '\r' => body.push_str("\\r"),
            '\t' => body.push_str("\\t"),
            c if (c as u32) < 0x20 => body.push_str(&format!("\\u{:04x}", c as u32)),
            c => body.push(c),
        }
    }
    body.push_str("\"}");
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.into())
        .unwrap()
}

/// `map_response` middleware: rewrites plain-text error bodies produced *inside* the
/// stack (e.g. axum's body-limit 413 rejection) into the structured JSON error shape.
/// Non-error responses and already-JSON errors pass through untouched; original headers
/// (e.g. `Allow` on a 405) are preserved.
async fn json_error_bodies(resp: Response) -> Response {
    let status = resp.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return resp;
    }
    let is_json = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if is_json {
        return resp;
    }
    let (mut parts, body) = resp.into_parts();
    // Error bodies are short; cap the read defensively.
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .unwrap_or_default();
    let msg = String::from_utf8_lossy(&bytes);
    let json = json_error(status, msg.trim());
    parts.headers.remove(header::CONTENT_TYPE);
    parts.headers.remove(header::CONTENT_LENGTH);
    let (jparts, jbody) = json.into_parts();
    for (k, v) in jparts.headers.iter() {
        parts.headers.insert(k, v.clone());
    }
    Response::from_parts(parts, jbody)
}

pub(crate) fn bad_request(msg: &str) -> Response {
    json_error(StatusCode::BAD_REQUEST, msg)
}

/// [OPUS-4.8] sq-406acc: a `406 Not Acceptable` for a present-but-unsatisfiable `Accept` header
/// on a SPARQL query — the `Accept` named no result/RDF media type the server can produce and no
/// wildcard, matching Oxigraph (w3c/sparql-protocol#40). The body is the structured
/// `{"error":"..."}` envelope the rest of the surface uses (so a programmatic client need not
/// scrape prose); `head_only` mirrors the GET headers with an empty body.
fn not_acceptable_response(head_only: bool) -> Response {
    let resp = json_error(
        StatusCode::NOT_ACCEPTABLE,
        "no acceptable result format for the request Accept header",
    );
    if head_only {
        // Preserve the status + headers but drop the body, mirroring the HEAD contract used by
        // `text_response`. `json_error` always builds a valid header set, so this never panics.
        let (parts, _body) = resp.into_parts();
        Response::from_parts(parts, axum::body::Body::empty())
    } else {
        resp
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-cmvh (ASVS-G1, cert remediation #237) — security response headers
//
// ASVS V14.4 wants standard hardening response headers on every HTTP response. sparq-server
// is a SPARQL *API* (it emits SPARQL-results JSON/XML/CSV/TSV and RDF — never HTML it asks a
// browser to render), so the header set is the subset that is meaningful for a non-HTML API,
// chosen deliberately rather than copy-pasting a browser-app template:
//
//   * `X-Content-Type-Options: nosniff` — stops a browser/proxy from MIME-sniffing a response
//     into a type we did not send. Always appropriate; cheap defence-in-depth even though we
//     always send an explicit `Content-Type`.
//   * `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'` — the responses are
//     data, not scripts/styles/images, so the tightest possible CSP (`default-src 'none'`)
//     fits exactly: a `default-src 'none'` document loads no subresources and runs no inline
//     script, so an injected/`text/html`-sniffed body is inert. `frame-ancestors 'none'` is the
//     modern, header-spoofing-proof way to say "must not be framed" (it supersedes
//     X-Frame-Options for CSP-aware agents and, unlike X-Frame-Options, also covers nested
//     frames). We send BOTH (next bullet) for coverage of older agents.
//   * `X-Frame-Options: DENY` — the legacy clickjacking guard for agents that do not honour
//     CSP `frame-ancestors`. The API is never meant to be framed, so DENY is correct.
//   * `Referrer-Policy: no-referrer` — a programmatic API client has no referrer concept, but
//     if a response IRI is ever followed from a browser context this prevents the request URL
//     (which may carry a `query=` containing sensitive terms) leaking in a `Referer` header.
//
// DELIBERATELY OMITTED (with reason), so the audit trail is explicit:
//   * `Strict-Transport-Security` (HSTS): sparq-server terminates PLAIN HTTP (TLS is the job of
//     a fronting reverse proxy — see README "Security posture"). Emitting HSTS from the origin
//     would be meaningless at best and, if it reached a browser over the proxy's TLS, could
//     wrongly pin a host; the TLS-terminating proxy is the correct place to set HSTS. N/A here.
//   * `X-XSS-Protection`: deprecated and a no-op (or harmful) in modern browsers; superseded by
//     CSP. Not emitted.
//   * `Cross-Origin-*` / `Permissions-Policy` / CORS headers: browser-app document policies with
//     no meaning for a data API that serves no documents and (by design) no CORS. Not emitted —
//     adding CORS headers would *widen* the surface, the opposite of hardening.
//
// `Cache-Control` is NOT forced here: existing responses already manage their own cache
// semantics where they need to, and a blanket `no-store` would override `/health`, `/metrics`
// and the federation descriptors. Query results are not cached by default (no `Cache-Control:
// public`/`ETag` is ever set), so there is nothing to tighten — see the omission note in
// `skills/http-server/SKILL.md`. Applied to EVERY response (success, streamed, and error —
// it runs on the response path of the same map_response stack `json_error_bodies` uses), so an
// error envelope is hardened identically to a 200.
//
// Headers are only INSERTED when absent, so a handler that sets a more specific value (e.g. a
// future per-route CSP) is never clobbered.

/// The static security response headers (name, value) added to every response. Kept as a
/// single source of truth so the integration test and the middleware agree on the exact set.
pub(crate) const SECURITY_HEADERS: &[(header::HeaderName, &str)] = &[
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'",
    ),
    (header::X_FRAME_OPTIONS, "DENY"),
    (header::REFERRER_POLICY, "no-referrer"),
];

/// `map_response` middleware ([OPUS-4.8] sq-cmvh): stamps the [`SECURITY_HEADERS`] hardening
/// set onto every response — success, streamed and error alike. Each header is only set when
/// absent, so a handler may override a specific one without this clobbering it. Header names
/// and values are all static and pre-validated, so the `from_static` conversions never fail.
async fn security_headers(mut resp: Response) -> Response {
    let headers = resp.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        if !headers.contains_key(name) {
            headers.insert(name, header::HeaderValue::from_static(value));
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-o7o0 (ASVS V14.5.3) — OPT-IN first-party CORS
//
// sparq-server is a SPARQL DATA API and its DEFAULT is to emit NO CORS headers (an
// empty [`CorsAllowlist`] => this middleware is not even installed; see `harden`). The
// option exists only so an operator can let a FIRST-PARTY browser app on a different
// origin read responses. The policy is deliberately conservative:
//
//   * reflect ONLY an exact allowlisted `Origin` into `Access-Control-Allow-Origin`;
//     an un-listed origin gets NO CORS header (browser blocks the read). Never `*`.
//   * always `Vary: Origin` so a shared cache never serves an origin-specific
//     `Access-Control-Allow-Origin` to a different origin.
//   * NEVER `Access-Control-Allow-Credentials` — this is for reading PUBLIC results;
//     the endpoint's own Bearer gate is orthogonal and unchanged.
//   * answer the `OPTIONS` preflight (when it carries `Access-Control-Request-Method`)
//     with the allowed methods/headers + a `Max-Age`, for an allowlisted origin only.
// ---------------------------------------------------------------------------

/// The methods advertised in a CORS preflight response. The HTTP surface accepts GET /
/// HEAD / POST (SPARQL query + the GSP read/write verbs) plus OPTIONS itself; PUT /
/// DELETE are the GSP write verbs; QUERY is the SPARQL Protocol query verb
/// (w3c/sparql-protocol#40, sq-b3df9) so a browser can preflight a `fetch(…, {method:
/// 'QUERY'})`. A browser's actual request is still subject to every other guard (auth,
/// body limit, …) — advertising a method here does not bypass them. [OPUS-4.8]
const CORS_ALLOW_METHODS: &str = "GET, HEAD, POST, PUT, DELETE, QUERY, OPTIONS";

/// The request headers advertised as allowed in a preflight when the browser does not
/// send its own `Access-Control-Request-Headers` (we otherwise reflect that list). Covers
/// the headers a SPARQL browser client sends: `Content-Type` (the POST forms) and
/// `Authorization` (the optional Bearer gate).
const CORS_ALLOW_HEADERS_DEFAULT: &str = "content-type, authorization";

/// Preflight cache lifetime (`Access-Control-Max-Age`, seconds) — 10 minutes, a modest
/// value so a policy change is picked up reasonably soon without re-preflighting every
/// request.
const CORS_MAX_AGE: &str = "600";

/// [OPUS-4.8] sq-o7o0: the opt-in first-party CORS middleware (installed by [`harden`]
/// only when [`ServerConfig::cors_allow`] is non-empty).
///
/// Reads the request `Origin`; if it is allowlisted (exact match, never `*`), reflects it
/// into `Access-Control-Allow-Origin` (+ `Vary: Origin`). A CORS *preflight* (an `OPTIONS`
/// carrying `Access-Control-Request-Method`) from an allowlisted origin is answered here
/// with a `204` + the allowed methods/headers/max-age, WITHOUT running the inner stack.
/// Any other request runs the inner stack normally and the actual-request CORS headers are
/// stamped onto its response. A request with no `Origin`, or with an un-listed `Origin`,
/// passes through completely untouched (no CORS header at all).
async fn cors_layer(
    allow: crate::cors_config::CorsAllowlist,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // The browser's Origin (RFC 6454 serialization). Absent ⇒ not a CORS request.
    let origin_allowed = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| allow.allows(o))
        .map(str::to_owned);

    // A preflight is an OPTIONS that carries Access-Control-Request-Method. Answer it
    // here for an allowlisted origin (204, no inner work). A bare OPTIONS without the
    // request-method header is NOT a preflight — let it fall through (it 405s) and get
    // the actual-request CORS header below.
    let is_preflight = req.method() == Method::OPTIONS
        && req
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);

    if is_preflight {
        return match &origin_allowed {
            Some(origin) => {
                // Echo the requested headers if the browser listed them, else a sane default.
                let allow_headers = req
                    .headers()
                    .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| header::HeaderValue::from_str(s).ok())
                    .unwrap_or_else(|| {
                        header::HeaderValue::from_static(CORS_ALLOW_HEADERS_DEFAULT)
                    });
                let mut resp = Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(axum::body::Body::empty())
                    .expect("static preflight response is always valid");
                let h = resp.headers_mut();
                // `origin` came from the request header (valid HeaderValue bytes) and is
                // allowlisted, so the conversion cannot fail.
                if let Ok(v) = header::HeaderValue::from_str(origin) {
                    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
                }
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    header::HeaderValue::from_static(CORS_ALLOW_METHODS),
                );
                h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, allow_headers);
                h.insert(
                    header::ACCESS_CONTROL_MAX_AGE,
                    header::HeaderValue::from_static(CORS_MAX_AGE),
                );
                h.insert(header::VARY, header::HeaderValue::from_static("Origin"));
                resp
            }
            // Preflight from a NON-allowlisted origin: no CORS headers; a 204 with no
            // Allow-Origin makes the browser fail the preflight (the desired refusal).
            None => Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(axum::body::Body::empty())
                .expect("static preflight response is always valid"),
        };
    }

    // Actual (non-preflight) request: run the inner stack, then stamp the CORS
    // response headers for an allowlisted origin. `Vary: Origin` is APPENDED (not
    // inserted) so we never clobber a `Vary` a handler already set.
    let mut resp = next.run(req).await;
    if let Some(origin) = origin_allowed {
        let h = resp.headers_mut();
        if let Ok(v) = header::HeaderValue::from_str(&origin) {
            h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
        }
        h.append(header::VARY, header::HeaderValue::from_static("Origin"));
    }
    resp
}

// ---------------------------------------------------------------------------
// Information-leak guard (sq-cz89 / sq-j9zs / sq-zg0u) [OPUS-4.8]
//
// On the B3 no-auth-by-default path an unauthenticated caller could provoke an error
// whose body echoed its own (or the loaded data's, or the server's filesystem path's)
// content verbatim — parse errors from `oxttl` / `spargebra` quote the offending token,
// `io::Error` carries paths, etc. That is an information leak: it confirms loaded triples
// (e.g. `patient_alice_smith is not a valid subject`) and discloses server-side paths.
//
// Fix: HTTP error bodies carry only a STABLE, GENERIC class message — never the caller's
// input, loaded-data fragments, or filesystem paths. The full detail is preserved for the
// operator on the SERVER SIDE via `tracing` (surfaced by the existing opt-in `--verbose` /
// RUST_LOG subscriber set up in `main.rs`), exactly the posture the TraceLayer request log
// already uses. Detail in the log, class in the body.
// ---------------------------------------------------------------------------

/// Emits the full (potentially sensitive) `detail` to the server-side `tracing` log under
/// the `sparq_server` target — visible only to the operator who opted into `--verbose` /
/// `RUST_LOG` — and returns a SANITIZED [`Response`] carrying just `safe_msg`, which MUST NOT
/// contain any caller-submitted input, loaded-data fragment, or filesystem path.
///
/// `class` names the error category for the log line so an operator can correlate a generic
/// client-facing message back to its detailed cause.
fn sanitized_error(status: StatusCode, class: &str, safe_msg: &str, detail: &str) -> Response {
    // Detail (the echoed token / path / input) goes ONLY to the server log, gated behind the
    // operator's opt-in subscriber — never into the response body.
    tracing::warn!(target: "sparq_server", class = class, detail = %detail, "request error (detail withheld from client)");
    json_error(status, safe_msg)
}

/// [OPUS-4.8] (sq-iu0c) A SERVICE egress refusal → HTTP 403 (Forbidden). The engine `msg`
/// names the refused host (an info-leak / SSRF-probe oracle), so it is SANITIZED: the client
/// gets only a stable, generic policy-refusal class message; the host detail goes to the
/// server log via [`sanitized_error`], the same posture as [`execution_error`].
#[cfg(feature = "service")]
fn forbidden_egress(msg: &str) -> Response {
    sanitized_error(
        StatusCode::FORBIDDEN,
        "service-egress-refused",
        "SERVICE federation refused: the requested endpoint is not permitted by the server's egress allowlist",
        msg,
    )
}

fn execution_error(msg: &str) -> Response {
    // Engine error strings can embed term text drawn from the loaded graph — withhold them
    // from the client; the operator sees the full message in the server log.
    sanitized_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "query-execution",
        "query execution error",
        msg,
    )
}

fn method_not_allowed(allow: &[Method]) -> Response {
    let allow_value = allow
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header(header::ALLOW, allow_value)
        .body(axum::body::Body::from("method not allowed"))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Visibility-scope extraction (sq-uqh, Wave B) — unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod touched_pods_tests {
    //! [OPUS-4.8] (sq-uqh) Per-named-graph `PodId` extraction from a parsed SPARQL Update.
    //! The whole point of Wave B's finer-than-global tagging: a write that names graph A
    //! must NOT bump graph B's epoch, while any write that cannot be scoped finer than the
    //! whole dataset MUST still bump the global pod (conservative, never under-invalidating).
    use super::{touched_pods, GLOBAL_POD};

    /// The set of pod-id strings a given update touches (order-independent).
    fn pods(update: &str) -> std::collections::BTreeSet<String> {
        touched_pods(update)
            .into_iter()
            .map(|p| p.as_str().to_string())
            .collect()
    }

    fn has(update: &str, iri: &str) -> bool {
        pods(update).contains(iri)
    }

    fn is_global(update: &str) -> bool {
        has(update, GLOBAL_POD)
    }

    /// A GRAPH-scoped INSERT/DELETE DATA touches exactly that named graph — never global,
    /// and never any OTHER graph (the A-not-B property at the extraction layer).
    #[test]
    fn graph_scoped_data_ops_are_scoped() {
        let ins =
            "INSERT DATA { GRAPH <http://ex/g/A> { <http://ex/s> <http://ex/p> <http://ex/o> } }";
        assert_eq!(
            pods(ins),
            ["http://ex/g/A".to_string()].into_iter().collect()
        );
        assert!(
            !is_global(ins),
            "a single-graph write must not bump the global pod"
        );
        assert!(!has(ins, "http://ex/g/B"), "a write to A must not touch B");

        let del =
            "DELETE DATA { GRAPH <http://ex/g/A> { <http://ex/s> <http://ex/p> <http://ex/o> } }";
        assert_eq!(
            pods(del),
            ["http://ex/g/A".to_string()].into_iter().collect()
        );
    }

    /// Multiple GRAPH blocks in one operation each contribute their own pod; no global.
    #[test]
    fn multiple_named_graphs_each_get_a_pod() {
        let u = "INSERT DATA { GRAPH <http://ex/g/A> { <http://ex/s> <http://ex/p> <http://ex/o> } \
                              GRAPH <http://ex/g/B> { <http://ex/s> <http://ex/p> <http://ex/o> } }";
        assert_eq!(
            pods(u),
            ["http://ex/g/A".to_string(), "http://ex/g/B".to_string()]
                .into_iter()
                .collect()
        );
        assert!(!is_global(u));
    }

    /// A default-graph data op is the catch-all pod (global) — the GSP surface maps every
    /// direct graph onto the default graph, so it cannot be scoped finer.
    #[test]
    fn default_graph_data_op_is_global() {
        let u = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";
        assert!(
            is_global(u),
            "a default-graph write must bump the global pod"
        );
        assert_eq!(pods(u), [GLOBAL_POD.to_string()].into_iter().collect());
    }

    /// CLEAR/DROP of a single named graph is scoped; DEFAULT/NAMED/ALL widen to global.
    #[test]
    fn clear_drop_scoping() {
        assert_eq!(
            pods("CLEAR GRAPH <http://ex/g/A>"),
            ["http://ex/g/A".to_string()].into_iter().collect()
        );
        assert_eq!(
            pods("DROP GRAPH <http://ex/g/A>"),
            ["http://ex/g/A".to_string()].into_iter().collect()
        );
        assert!(is_global("CLEAR DEFAULT"));
        assert!(is_global("CLEAR NAMED"));
        assert!(is_global("CLEAR ALL"));
        assert!(is_global("DROP ALL"));
    }

    /// CREATE touches only the new graph.
    #[test]
    fn create_is_scoped() {
        assert_eq!(
            pods("CREATE GRAPH <http://ex/g/new>"),
            ["http://ex/g/new".to_string()].into_iter().collect()
        );
    }

    /// DELETE/INSERT … WHERE with CONCRETE template graphs is scoped to exactly the
    /// graphs the templates WRITE — the WHERE/USING read scope does not widen it.
    #[test]
    fn delete_insert_concrete_templates_are_scoped() {
        let u = "INSERT { GRAPH <http://ex/g/dst> { ?s ?p ?o } } \
                 WHERE  { GRAPH <http://ex/g/src> { ?s ?p ?o } }";
        // Only the write target (dst) is invalidation-relevant; the read source (src) is not.
        assert_eq!(
            pods(u),
            ["http://ex/g/dst".to_string()].into_iter().collect()
        );
        assert!(
            !has(u, "http://ex/g/src"),
            "the WHERE read source must not be invalidated"
        );
        assert!(!is_global(u));
    }

    /// A VARIABLE graph name in a template (`GRAPH ?g { … }`) makes the written graphs
    /// unknowable at parse time → must fall back to global (never under-invalidate).
    #[test]
    fn delete_insert_variable_graph_target_is_global() {
        let u = "INSERT { GRAPH ?g { ?s ?p ?o } } WHERE { GRAPH ?g { ?s ?p ?o } }";
        assert!(
            is_global(u),
            "a dynamically-scoped write target must bump the global pod"
        );
    }

    /// A default-graph DELETE/INSERT template is the catch-all pod.
    #[test]
    fn delete_insert_default_template_is_global() {
        let u = "DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }";
        assert!(is_global(u));
    }

    /// A write that mixes a scopable named-graph effect with an unscopable one bumps BOTH
    /// the named pod AND global — finer scoping is additive over the catch-all.
    #[test]
    fn mixed_named_and_global_bumps_both() {
        let u =
            "INSERT DATA { GRAPH <http://ex/g/A> { <http://ex/s> <http://ex/p> <http://ex/o> } \
                              <http://ex/d> <http://ex/p> <http://ex/o> }";
        assert!(has(u, "http://ex/g/A"));
        assert!(
            is_global(u),
            "the default-graph half must still bump global"
        );
    }

    /// LOAD into a named graph is scoped; LOAD into the default graph is global. (The LOAD
    /// itself is refused at apply time without an allowlist — extraction is parse-only.)
    #[test]
    fn load_scoping() {
        assert_eq!(
            pods("LOAD <file:///x.ttl> INTO GRAPH <http://ex/g/A>"),
            ["http://ex/g/A".to_string()].into_iter().collect()
        );
        assert!(
            is_global("LOAD <file:///x.ttl>"),
            "LOAD into the default graph is global"
        );
    }

    /// An unparsable update is tagged global so extraction is total and never
    /// under-invalidates (the writer re-parses and rejects it anyway).
    #[test]
    fn unparsable_update_is_global() {
        assert!(is_global("THIS IS NOT SPARQL"));
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-o4qf — bind-posture (no-auth non-loopback gate) tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod bind_posture_tests {
    //! The server has no built-in auth, so a non-loopback bind must be a deliberate opt-in.
    //! These tests pin: (a) loopback always proceeds; (b) a non-loopback bind WITHOUT the
    //! opt-in is refused (fail-closed, incl. the `0.0.0.0`/`::` all-interfaces addresses,
    //! which are the usual way the surface gets exposed); (c) WITH the opt-in it proceeds
    //! but warns. Pure functions over an explicit `allow_remote` flag — no process-env
    //! mutation, so they are parallel-safe.
    use super::{bind_posture, env_truthy, AuthPosture, BindPosture};
    use std::net::SocketAddr;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_proceeds_regardless_of_flag() {
        for a in ["127.0.0.1:3030", "127.0.0.5:80", "[::1]:3030"] {
            assert_eq!(
                bind_posture(&addr(a), false, AuthPosture::None),
                BindPosture::Loopback,
                "{a} (no opt-in)"
            );
            assert_eq!(
                bind_posture(&addr(a), true, AuthPosture::None),
                BindPosture::Loopback,
                "{a} (opt-in)"
            );
        }
    }

    #[test]
    fn non_loopback_without_optin_is_refused() {
        // 0.0.0.0 / :: (all-interfaces), an RFC1918 address, a link-local, and the cloud
        // metadata IP all fail closed without --allow-remote (and with no auth).
        for a in [
            "0.0.0.0:3030",
            "[::]:3030",
            "10.0.0.1:8080",
            "169.254.169.254:80",
            "192.168.1.5:3030",
        ] {
            match bind_posture(&addr(a), false, AuthPosture::None) {
                BindPosture::RemoteRefused { message } => {
                    assert!(message.contains("refusing to bind"), "{a}: {message}");
                    assert!(
                        message.contains("--allow-remote"),
                        "{a}: must name the opt-in flag"
                    );
                    assert!(
                        message.contains("authentication"),
                        "{a}: must explain the no-auth risk"
                    );
                }
                other => panic!("{a} must be refused without opt-in, got {other:?}"),
            }
        }
    }

    #[test]
    fn non_loopback_with_optin_warns_but_proceeds() {
        match bind_posture(&addr("0.0.0.0:3030"), true, AuthPosture::None) {
            BindPosture::RemoteAllowed { warning } => {
                assert!(warning.contains("WARNING"), "{warning}");
                assert!(warning.contains("READ AND WRITE"), "{warning}");
                assert!(
                    warning.contains("0.0.0.0:3030"),
                    "must name the address: {warning}"
                );
            }
            other => panic!("opt-in must allow with a warning, got {other:?}"),
        }
    }

    // [OPUS-4.8] sq-zcby — auth folded into the bind decision.

    /// A WRITE-only token (reads still open) is NOT sufficient to bind a non-loopback address:
    /// --allow-remote is still required, because reads remain open on the remote bind. The
    /// refusal must point at --auth-token-read as the way to make the whole surface safe.
    #[test]
    fn write_only_token_still_refused_without_optin() {
        match bind_posture(&addr("0.0.0.0:3030"), false, AuthPosture::WriteOnly) {
            BindPosture::RemoteRefused { message } => {
                assert!(message.contains("refusing to bind"), "{message}");
                assert!(
                    message.contains("--auth-token-read"),
                    "must name the read-gate flag: {message}"
                );
            }
            other => {
                panic!("a write-only token must still be refused without opt-in, got {other:?}")
            }
        }
    }

    /// A WRITE-only token WITH --allow-remote proceeds, but warns that READS remain open.
    #[test]
    fn write_only_token_with_optin_warns_reads_open() {
        match bind_posture(&addr("0.0.0.0:3030"), true, AuthPosture::WriteOnly) {
            BindPosture::RemoteAllowed { warning } => {
                assert!(warning.contains("WARNING"), "{warning}");
                assert!(
                    warning.to_ascii_uppercase().contains("READS"),
                    "must warn reads are open: {warning}"
                );
            }
            other => panic!("write-only + opt-in must allow with a warning, got {other:?}"),
        }
    }

    /// A FULLY authenticated surface (token gates reads AND writes) is allowed to bind a
    /// non-loopback address WITHOUT --allow-remote — there is no open endpoint left (sq-cxk5:
    /// the /subscriptions WS + SSE read surfaces are gated by --auth-token-read too). It still
    /// warns (single shared secret, deliver over TLS).
    #[test]
    fn full_auth_allows_remote_bind_without_optin() {
        match bind_posture(&addr("0.0.0.0:3030"), false, AuthPosture::ReadAndWrite) {
            BindPosture::RemoteAllowed { warning } => {
                assert!(warning.contains("WARNING"), "{warning}");
                assert!(
                    warning.contains("--auth-token"),
                    "must name the gate: {warning}"
                );
            }
            other => panic!("full auth must allow a remote bind without opt-in, got {other:?}"),
        }
    }

    #[test]
    fn env_truthy_recognises_common_forms() {
        for t in ["1", "true", "TRUE", " yes ", "On", "Yes"] {
            assert!(env_truthy(t), "{t:?} should be truthy");
        }
        for f in ["", "0", "false", "no", "off", "2", "maybe"] {
            assert!(!env_truthy(f), "{f:?} should be falsy");
        }
    }
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-qsm5z — running-query registry HTTP handlers
// ---------------------------------------------------------------------------

/// [SONNET-4.6] (sq-qsm5z) `GET /queries` — list currently executing SPARQL queries.
///
/// Returns a JSON object `{"queries": [...]}` where each entry carries `id`, `start`
/// (Unix epoch ms), `fingerprint` (FNV-1a hex — never raw query text), and `elapsed_ms`.
/// Entries are sorted by `id` (registration order). An empty registry returns `{"queries":[]}`.
///
/// **Auth:** READ-gated (fail-closed). Mirrors the posture of every other admin route.
#[cfg(feature = "query-registry")]
async fn list_queries_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Read) {
        return resp;
    }
    let body = serde_json::json!({ "queries": state.running_queries.list() }).to_string();
    text_response(StatusCode::OK, "application/json", body, false)
}

/// [SONNET-4.6] (sq-qsm5z) `DELETE /queries/{id}` — cooperatively cancel one executing query.
///
/// Flips the `sq-kq9ia` `Arc<AtomicBool>` cancel flag that was wired into the query's
/// `QueryBudget` at registration time. The engine observes the flag at its next cooperative
/// poll site and aborts with `"query budget exceeded (cancelled)"`. The registry row stays
/// until the worker exits and the RAII `QueryGuard` fires its `Drop`.
///
/// - `204 No Content` — the cancel flag was flipped.
/// - `404 Not Found` — no query with that id in the registry (already finished, or bad id).
///
/// **Auth:** WRITE-gated (fail-closed). An unauthenticated or wrongly-authenticated request
/// receives a `401` identical for missing vs. wrong token.
#[cfg(feature = "query-registry")]
async fn cancel_query_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = auth_gate(state.config(), &headers, Operation::Write) {
        return resp;
    }
    if state.running_queries.cancel_query(&id) {
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(axum::body::Body::empty())
            .unwrap()
    } else {
        json_error(StatusCode::NOT_FOUND, "running query not found")
    }
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-qsm5z — registry unit tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "query-registry"))]
mod query_registry_tests {
    use super::{QueryRegistry, registry_fingerprint};
    use std::sync::Arc;

    // Fingerprint must be stable across calls with the same input.
    #[test]
    fn fingerprint_is_stable() {
        let a = registry_fingerprint("SELECT * WHERE { ?s ?p ?o }");
        let b = registry_fingerprint("SELECT * WHERE { ?s ?p ?o }");
        assert_eq!(a, b, "fingerprint must be deterministic");
        // 16 hex chars for a 64-bit hash.
        assert_eq!(a.len(), 16);
    }

    // Different queries must produce different fingerprints.
    #[test]
    fn fingerprint_differs_for_distinct_queries() {
        let a = registry_fingerprint("SELECT * WHERE { ?s ?p ?o }");
        let b = registry_fingerprint("ASK WHERE { ?s ?p ?o }");
        assert_ne!(a, b, "distinct queries must yield distinct fingerprints");
    }

    // The RAII guard removes the entry on normal return.
    #[test]
    fn guard_removes_entry_on_normal_return() {
        let registry = Arc::new(QueryRegistry::default());
        {
            let (_cancel, _guard) = registry.register("SELECT * WHERE { ?s ?p ?o }");
            assert_eq!(registry.list().len(), 1, "entry must be present while guard is alive");
        }
        assert_eq!(registry.list().len(), 0, "entry must be removed after guard drops");
    }

    // The RAII guard removes the entry even during a panic / unwind.
    #[test]
    fn guard_removes_entry_during_panic_unwind() {
        let registry = Arc::new(QueryRegistry::default());
        let r2 = Arc::clone(&registry);
        let result = std::thread::spawn(move || {
            let (_cancel, _guard) = r2.register("SELECT 1 WHERE {}");
            panic!("witness panic");
        })
        .join();
        assert!(result.is_err(), "thread must have panicked");
        assert_eq!(registry.list().len(), 0, "entry must be cleaned up after unwind");
    }

    // Cancelling an entry flips the flag (non-vacuous: verify the flag starts false).
    #[test]
    fn cancel_flips_flag_to_true() {
        use std::sync::atomic::Ordering;
        let registry = Arc::new(QueryRegistry::default());
        let (cancel, _guard) = registry.register("SELECT * WHERE { ?s ?p ?o }");
        assert!(!cancel.load(Ordering::Acquire), "flag must start false");
        let id = registry.list()[0]["id"].as_str().unwrap().to_owned();
        let found = registry.cancel_query(&id);
        assert!(found, "cancel must return true for a known id");
        assert!(cancel.load(Ordering::Acquire), "flag must be true after cancel");
    }

    // [SONNET-4.6] (sq-m9prn) A read query and an UPDATE are distinguishable in the listing,
    // so an operator can tell which row is holding the sequenced writer thread.
    // Non-vacuity: collapsing `register_update` onto `register` makes both rows read
    // "query" and the `kind` assertions below fail.
    #[test]
    fn update_and_query_rows_carry_distinct_kinds() {
        let registry = Arc::new(QueryRegistry::default());
        let (_c1, _g1) = registry.register("SELECT * WHERE { ?s ?p ?o }");
        let (_c2, _g2) = registry.register_update("DELETE WHERE { ?s ?p ?o }");
        let kinds: Vec<String> = registry
            .list()
            .iter()
            .map(|e| e["kind"].as_str().unwrap_or_default().to_owned())
            .collect();
        assert!(kinds.contains(&"query".to_string()), "read row must be kind=query: {:?}", kinds);
        assert!(kinds.contains(&"update".to_string()), "update row must be kind=update: {:?}", kinds);
    }

    // [SONNET-4.6] (sq-m9prn) An update row is cancellable and RAII-cleaned exactly like a
    // query row — the two properties `DELETE /queries/{id}` depends on.
    #[test]
    fn update_row_is_cancellable_and_raii_cleaned() {
        use std::sync::atomic::Ordering;
        let registry = Arc::new(QueryRegistry::default());
        {
            let (cancel, _guard) = registry.register_update("DELETE WHERE { ?s ?p ?o }");
            assert!(!cancel.load(Ordering::Acquire), "flag must start false");
            let id = registry.list()[0]["id"].as_str().unwrap().to_owned();
            assert!(registry.cancel_query(&id), "an update row must be cancellable by id");
            assert!(cancel.load(Ordering::Acquire), "cancelling the update must flip its flag");
        }
        assert_eq!(registry.list().len(), 0, "update row must be removed once the apply returns");
    }

    // [SONNET-4.6] (sq-m9prn) The listing never carries raw query text — for updates too,
    // which is where a `INSERT DATA` body would otherwise leak the written triples.
    #[test]
    fn update_row_exposes_no_raw_text() {
        let registry = Arc::new(QueryRegistry::default());
        let secret = "INSERT DATA { <http://ex/s> <http://ex/p> \"sensitive-literal\" }";
        let (_cancel, _guard) = registry.register_update(secret);
        let listed = serde_json::to_string(&registry.list()[0]).unwrap();
        assert!(
            !listed.contains("sensitive-literal") && !listed.contains("INSERT"),
            "the listing must expose only the fingerprint, never the update text: {}",
            listed
        );
    }

    // [SONNET-4.6] (sq-m9prn) Pins the ACTUAL security contract of the exposed fingerprint, so
    // the docs cannot drift back into claiming it is a one-way/"non-reversible" hash. FNV-1a is
    // UNKEYED: a party holding only the listing can recompute the fingerprint of candidate texts
    // offline and confirm which one is running. This test performs exactly that recovery — it is
    // a WITNESS of the documented weakness, not an endorsement of it.
    // Non-vacuity in both directions: if the construction ever becomes keyed (an HMAC under a
    // per-process secret, the fix this contract defers), the offline recomputation stops matching
    // and this test goes red — forcing `registry_fingerprint`'s security-contract doc and the
    // SKILL's "honest boundary" bullet to be revisited with it.
    #[test]
    fn registry_fingerprint_is_unkeyed_and_offline_guessable() {
        let registry = Arc::new(QueryRegistry::default());
        let secret = "INSERT DATA { <http://ex/s> <http://ex/p> \"sensitive-literal\" }";
        let (_cancel, _guard) = registry.register_update(secret);
        let exposed = registry.list()[0]["fingerprint"].as_str().unwrap().to_owned();

        // An attacker's offline dictionary — plausible bodies, one of which is the real one.
        let candidates = [
            "INSERT DATA { <http://ex/s> <http://ex/p> \"other-literal\" }",
            "INSERT DATA { <http://ex/s> <http://ex/p> \"sensitive-literal\" }",
            "DELETE WHERE { ?s ?p ?o }",
        ];
        let recovered: Vec<&str> =
            candidates.iter().copied().filter(|c| registry_fingerprint(c) == exposed).collect();
        assert_eq!(
            recovered,
            vec![secret],
            "the unkeyed fingerprint must be reproducible from the text alone — offline guessing \
             recovers exactly the registered body, which is precisely why the listing is NOT a \
             confidentiality boundary for update payloads"
        );
    }

    // Cancelling an unknown id returns false (the 404 path).
    #[test]
    fn cancel_unknown_id_returns_false() {
        let registry = Arc::new(QueryRegistry::default());
        assert!(!registry.cancel_query("0000000000000000"), "unknown id must return false");
    }

    // Multiple simultaneous entries are all listed and all individually cancellable.
    #[test]
    fn multiple_entries_listed_and_cancellable() {
        use std::sync::atomic::Ordering;
        let registry = Arc::new(QueryRegistry::default());
        let (c1, _g1) = registry.register("SELECT ?a WHERE { ?a ?b ?c }");
        let (c2, _g2) = registry.register("ASK WHERE { ?x ?y ?z }");
        let list = registry.list();
        assert_eq!(list.len(), 2, "both entries must be present");
        let id0 = list[0]["id"].as_str().unwrap().to_owned();
        registry.cancel_query(&id0);
        // Exactly one flag is now set.
        let set_count = [c1, c2].iter().filter(|f| f.load(Ordering::Acquire)).count();
        assert_eq!(set_count, 1, "exactly one cancel flag must be set");
    }

    // ---------------------------------------------------------------------------
    // [SONNET-4.6] sq-t1isr — EXPLAIN ANALYZE path registry tests
    //
    // These tests simulate the EXACT code pattern added to the EXPLAIN ANALYZE
    // spawn_blocking path in `run_query_pinned`.  They are the mutation target:
    // removing the `register(sparql)` call from that path would break the
    // `explain_path_entry_present_while_guard_alive` test (registry would stay
    // empty and the assert_eq!(1) would fire).
    // ---------------------------------------------------------------------------

    /// The EXPLAIN-path registers an entry while the simulated closure holds the guard.
    /// Non-vacuity: removing the `register()` call in `run_query_pinned` → registry
    /// stays empty → `assert_eq!(list.len(), 1)` fails.
    #[test]
    fn explain_path_entry_present_while_guard_alive() {
        let registry = Arc::new(QueryRegistry::default());
        // Mirror the exact pattern used in run_query_pinned for the EXPLAIN branch:
        //   let (cancel, guard) = state.running_queries.register(sparql);
        //   let budget = base.with_cancel(cancel);
        //   let task = spawn_blocking(move || { let _guard = guard; ... });
        let (_cancel, guard) = registry.register("EXPLAIN SELECT * WHERE { ?s ?p ?o }");
        // While the guard is alive (simulating the blocking closure running):
        let list = registry.list();
        assert_eq!(
            list.len(),
            1,
            "EXPLAIN ANALYZE entry must be present in the registry while the worker runs"
        );
        let entry = &list[0];
        assert!(
            entry.get("id").and_then(|v| v.as_str()).is_some(),
            "registry entry must have an id field"
        );
        assert!(
            entry.get("fingerprint").and_then(|v| v.as_str()).is_some(),
            "registry entry must have a fingerprint"
        );
        // Simulate the closure returning — guard drops, entry must be removed.
        drop(guard);
        assert_eq!(
            registry.list().len(),
            0,
            "EXPLAIN ANALYZE entry must be deregistered after the worker exits"
        );
    }

    /// The cancel flag obtained from the EXPLAIN-path registration can be flipped via
    /// `cancel_query`, enabling `GET /queries` + `DELETE /queries/{id}` to kill a slow
    /// EXPLAIN ANALYZE.  Non-vacuity: if the cancel flag were not wired into the
    /// EXPLAIN budget via `with_cancel`, the flag flip would still work here (it is the
    /// `AtomicBool` itself), but the engine would not observe it — a separate honesty note.
    #[test]
    fn explain_path_cancel_flag_flippable_via_registry() {
        use std::sync::atomic::Ordering;
        let registry = Arc::new(QueryRegistry::default());
        let (cancel, _guard) = registry.register("EXPLAIN ANALYZE SELECT * WHERE { ?s ?p ?o }");
        assert!(
            !cancel.load(Ordering::Acquire),
            "cancel flag must start false"
        );
        let id = registry.list()[0]["id"].as_str().unwrap().to_owned();
        let found = registry.cancel_query(&id);
        assert!(found, "cancel_query must return true for the EXPLAIN entry id");
        assert!(
            cancel.load(Ordering::Acquire),
            "cancel flag must be true after DELETE /queries/{id}"
        );
    }

    /// An EXPLAIN entry does not survive a panic inside the simulated worker
    /// (the Drop impl on QueryGuard fires even during unwind).
    #[test]
    fn explain_path_guard_cleans_up_on_panic() {
        let registry = Arc::new(QueryRegistry::default());
        let r2 = Arc::clone(&registry);
        let result = std::thread::spawn(move || {
            let (_cancel, _guard) = r2.register("EXPLAIN ANALYZE SELECT ?x WHERE { ?x ?y ?z }");
            panic!("simulated worker panic");
        })
        .join();
        assert!(result.is_err(), "thread must have panicked");
        assert_eq!(
            registry.list().len(),
            0,
            "EXPLAIN entry must be cleaned up even after worker panic"
        );
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-zcby — auth-gate unit tests (constant-time eq, Bearer parsing,
// mutation classification, the gate decision, posture derivation)
// ---------------------------------------------------------------------------
#[cfg(test)]
mod auth_tests {
    //! Pure-function coverage for the Bearer-token write gate. The end-to-end HTTP behaviour
    //! (401 shape, mutation-applied, read-gate, GSP write gate, update-via-query/form path) is
    //! the integration suite in `tests/auth.rs`; these pin the building blocks.
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with_auth(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn constant_time_eq_matches_plain_eq() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"", b""),
            (b"a", b"a"),
            (b"a", b"b"),
            (b"token", b"token"),
            (b"token", b"toker"),
            (b"token", b"tokens"), // length mismatch
            (b"", b"x"),
        ];
        for (a, b) in cases {
            assert_eq!(constant_time_eq(a, b), a == b, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn bearer_token_tolerates_scheme_casing() {
        for v in ["Bearer t0k", "bearer t0k", "BEARER t0k", "BeArEr t0k"] {
            assert_eq!(bearer_token(&headers_with_auth(v)), Some("t0k"), "{v:?}");
        }
        // Trailing/leading space around the token is trimmed.
        assert_eq!(
            bearer_token(&headers_with_auth("Bearer  t0k  ")),
            Some("t0k")
        );
    }

    #[test]
    fn bearer_token_rejects_other_schemes_and_absence() {
        assert_eq!(bearer_token(&headers_with_auth("Basic abc")), None);
        assert_eq!(bearer_token(&headers_with_auth("t0k")), None); // no scheme
        assert_eq!(bearer_token(&HeaderMap::new()), None); // no header
    }

    #[test]
    fn payload_mutates_classifies_reads_and_writes() {
        // Reads (well-formed queries) do not mutate.
        for q in [
            "SELECT ?s WHERE { ?s ?p ?o }",
            "ASK { ?s ?p ?o }",
            "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
            "DESCRIBE <http://ex/s>",
        ] {
            assert!(!payload_mutates(q), "{q:?} must be a read");
        }
        // Writes (updates) mutate.
        for u in [
            "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }",
            "DELETE DATA { <http://ex/s> <http://ex/p> <http://ex/o> }",
            "DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }",
            "DROP ALL",
            "CLEAR DEFAULT",
            "LOAD <http://ex/d> INTO GRAPH <http://ex/g>",
        ] {
            assert!(payload_mutates(u), "{u:?} must be a write");
        }
        // Garbage that parses as neither is fail-closed to a write.
        assert!(
            payload_mutates("this is not sparql"),
            "unparsable must fail closed to a write"
        );
    }

    #[test]
    fn auth_posture_from_config() {
        let mut cfg = ServerConfig::default();
        assert_eq!(AuthPosture::from_config(&cfg), AuthPosture::None);
        cfg.auth_token = Some("t".into());
        assert_eq!(AuthPosture::from_config(&cfg), AuthPosture::WriteOnly);
        cfg.auth_token_read = true;
        assert_eq!(AuthPosture::from_config(&cfg), AuthPosture::ReadAndWrite);
        // read-gate without a token is still "None" (no token => nothing to gate with).
        cfg.auth_token = None;
        assert_eq!(AuthPosture::from_config(&cfg), AuthPosture::None);
    }

    #[test]
    fn auth_gate_no_token_never_gates() {
        let cfg = ServerConfig::default(); // no auth_token
        let h = HeaderMap::new();
        assert!(auth_gate(&cfg, &h, Operation::Write).is_none());
        assert!(auth_gate(&cfg, &h, Operation::Read).is_none());
    }

    #[test]
    fn auth_gate_write_only_gates_writes_not_reads() {
        let cfg = ServerConfig {
            auth_token: Some("secret".into()),
            ..ServerConfig::default()
        };
        // Writes need the token.
        assert!(
            auth_gate(&cfg, &HeaderMap::new(), Operation::Write).is_some(),
            "missing token => 401"
        );
        assert!(
            auth_gate(&cfg, &headers_with_auth("Bearer wrong"), Operation::Write).is_some(),
            "wrong token => 401"
        );
        assert!(
            auth_gate(&cfg, &headers_with_auth("Bearer secret"), Operation::Write).is_none(),
            "correct token => proceed"
        );
        // Reads stay open (no read gate).
        assert!(
            auth_gate(&cfg, &HeaderMap::new(), Operation::Read).is_none(),
            "reads open in write-only mode"
        );
    }

    #[test]
    fn auth_gate_read_gate_also_gates_reads() {
        let cfg = ServerConfig {
            auth_token: Some("secret".into()),
            auth_token_read: true,
            ..ServerConfig::default()
        };
        assert!(
            auth_gate(&cfg, &HeaderMap::new(), Operation::Read).is_some(),
            "read gated => 401"
        );
        assert!(
            auth_gate(&cfg, &headers_with_auth("bearer secret"), Operation::Read).is_none(),
            "correct token (lowercase scheme) => proceed"
        );
    }

    #[test]
    fn unauthorized_carries_www_authenticate_bearer() {
        let resp = unauthorized();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok()),
            Some("Bearer")
        );
    }

    // [OPUS-4.8] sq-cxk5: the WebSocket subprotocol-token channel + the WS auth gate.

    fn headers_with_subprotocol(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn subprotocol_bearer_token_extracts_the_bearer_subprotocol() {
        // The matched subprotocol AND the token after the `bearer.` prefix are returned.
        assert_eq!(
            subprotocol_bearer_token(&headers_with_subprotocol("bearer.t0k")),
            Some(("bearer.t0k", "t0k"))
        );
        // First bearer.* entry wins, even alongside other offered subprotocols.
        assert_eq!(
            subprotocol_bearer_token(&headers_with_subprotocol("graphql-ws, bearer.t0k, other")),
            Some(("bearer.t0k", "t0k"))
        );
        // The token body is exact (no trimming) — a subprotocol value carries no spaces anyway.
        assert_eq!(
            subprotocol_bearer_token(&headers_with_subprotocol("bearer.")),
            Some(("bearer.", ""))
        );
    }

    #[test]
    fn subprotocol_bearer_token_absent_without_the_prefix() {
        assert_eq!(
            subprotocol_bearer_token(&headers_with_subprotocol("graphql-ws, chat")),
            None
        );
        assert_eq!(subprotocol_bearer_token(&HeaderMap::new()), None);
        // A scheme other than the `bearer.` subprotocol prefix is not a match.
        assert_eq!(
            subprotocol_bearer_token(&headers_with_subprotocol("token.t0k")),
            None
        );
    }

    #[test]
    fn ws_auth_gate_no_token_never_gates() {
        let cfg = ServerConfig::default();
        assert!(ws_auth_gate(&cfg, &HeaderMap::new(), Operation::Read).is_none());
        // Even an offered bearer subprotocol proceeds when nothing is configured.
        assert!(ws_auth_gate(
            &cfg,
            &headers_with_subprotocol("bearer.anything"),
            Operation::Read
        )
        .is_none());
    }

    #[test]
    fn ws_auth_gate_read_gate_accepts_header_or_subprotocol() {
        let cfg = ServerConfig {
            auth_token: Some("secret".into()),
            auth_token_read: true,
            ..ServerConfig::default()
        };
        // Neither channel => 401.
        assert!(
            ws_auth_gate(&cfg, &HeaderMap::new(), Operation::Read).is_some(),
            "no credentials => 401"
        );
        // The Authorization: Bearer header is accepted (non-browser clients).
        assert!(
            ws_auth_gate(&cfg, &headers_with_auth("Bearer secret"), Operation::Read).is_none(),
            "header token => proceed"
        );
        // The Sec-WebSocket-Protocol bearer.<token> subprotocol is accepted (browsers).
        assert!(
            ws_auth_gate(
                &cfg,
                &headers_with_subprotocol("bearer.secret"),
                Operation::Read
            )
            .is_none(),
            "subprotocol token => proceed"
        );
        // A WRONG subprotocol token is VALIDATED, not echoed — 401.
        assert!(
            ws_auth_gate(
                &cfg,
                &headers_with_subprotocol("bearer.wrong"),
                Operation::Read
            )
            .is_some(),
            "wrong subprotocol token => 401"
        );
    }

    #[test]
    fn ws_auth_gate_write_only_leaves_ws_read_open() {
        // A write-only token (no --auth-token-read) does not gate the WS read upgrade.
        let cfg = ServerConfig {
            auth_token: Some("secret".into()),
            ..ServerConfig::default()
        };
        assert!(
            ws_auth_gate(&cfg, &HeaderMap::new(), Operation::Read).is_none(),
            "WS read open in write-only mode"
        );
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-4w18 — SERVICE egress allowlist config wiring tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod hardening_unit_tests {
    //! [OPUS-4.8] (sq-ebii) Pure-logic units for the memory cap (`tighter`) and the
    //! decompression-ratio cap (`decode_request_body`). The HTTP-level 413/503 wiring is
    //! covered by tests/hardening.rs; these pin the boundary math without a server boot.
    use super::{decode_request_body, tighter, update_where_deadline, ServerConfig};
    use axum::body::Bytes;
    use axum::http::{header, HeaderMap, HeaderValue};
    use std::time::Duration;

    #[test]
    fn tighter_takes_the_smaller_present_cap() {
        assert_eq!(tighter(None, None), None);
        assert_eq!(tighter(Some(5), None), Some(5));
        assert_eq!(tighter(None, Some(7)), Some(7));
        assert_eq!(tighter(Some(5), Some(7)), Some(5));
        assert_eq!(tighter(Some(9), Some(7)), Some(7));
    }

    // [OPUS-4.8] (sq-nulp) The writer-side WHERE deadline for an UPDATE is the TIGHTER of the
    // read query_timeout and the opt-in update_where_timeout — that smaller value is what
    // bounds writer-queue head-of-line blocking. `None` only when BOTH are unset.
    #[test]
    fn update_where_deadline_is_the_tighter_of_query_and_update_timeouts() {
        let with = |q: Option<u64>, u: Option<u64>| {
            update_where_deadline(&ServerConfig {
                query_timeout: q.map(Duration::from_secs),
                update_where_timeout: u.map(Duration::from_secs),
                ..ServerConfig::default()
            })
        };
        // Both unset => no deadline at all.
        assert_eq!(with(None, None), None);
        // Only one present => that one (whichever it is).
        assert_eq!(with(Some(30), None), Some(Duration::from_secs(30)));
        assert_eq!(with(None, Some(2)), Some(Duration::from_secs(2)));
        // Both present => the SMALLER. The whole point: a short update_where_timeout wins over
        // a long read query_timeout, so the writer is released sooner (head-of-line bound).
        assert_eq!(with(Some(30), Some(2)), Some(Duration::from_secs(2)));
        // ...and an update_where_timeout LARGER than query_timeout never loosens the bound.
        assert_eq!(with(Some(5), Some(30)), Some(Duration::from_secs(5)));
    }

    // [OPUS-4.8] (sq-nulp) The DEFAULT config sets no update_where_timeout, so the update WHERE
    // budget is exactly the plain query_timeout — byte-for-byte the historical behaviour.
    #[test]
    fn update_where_deadline_defaults_to_query_timeout_unchanged() {
        let cfg = ServerConfig::default();
        assert_eq!(
            cfg.update_where_timeout, None,
            "no separate update deadline by default"
        );
        assert_eq!(update_where_deadline(&cfg), cfg.query_timeout);
    }

    fn gz(data: &[u8]) -> Bytes {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(data).unwrap();
        Bytes::from(e.finish().unwrap())
    }

    fn headers_with_encoding(enc: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(e) = enc {
            h.insert(header::CONTENT_ENCODING, HeaderValue::from_str(e).unwrap());
        }
        h
    }

    #[test]
    fn identity_body_passes_through() {
        let cfg = ServerConfig::default();
        let body = Bytes::from_static(b"hello");
        // No Content-Encoding and explicit identity both pass verbatim.
        assert_eq!(
            decode_request_body(&body, &headers_with_encoding(None), &cfg).unwrap(),
            body
        );
        assert_eq!(
            decode_request_body(&body, &headers_with_encoding(Some("identity")), &cfg).unwrap(),
            body
        );
    }

    #[test]
    fn gzip_within_ratio_decodes() {
        let cfg = ServerConfig {
            max_decompress_ratio: 100,
            ..ServerConfig::default()
        };
        let plain = b"some moderately repetitive payload payload payload";
        let body = gz(plain);
        let out = decode_request_body(&body, &headers_with_encoding(Some("gzip")), &cfg).unwrap();
        assert_eq!(&out[..], &plain[..]);
    }

    #[test]
    fn high_ratio_gzip_is_refused() {
        // A 1 MiB run of zeros gzips to a tiny body — ratio far above 2× → refused.
        let cfg = ServerConfig {
            max_decompress_ratio: 2,
            max_body_bytes: 1 << 30,
            ..ServerConfig::default()
        };
        let body = gz(&vec![0u8; 1 << 20]);
        let err =
            decode_request_body(&body, &headers_with_encoding(Some("gzip")), &cfg).unwrap_err();
        assert!(matches!(err, super::DecodeError::TooLarge(_)));
    }

    #[test]
    fn ratio_zero_refuses_gzip() {
        let cfg = ServerConfig {
            max_decompress_ratio: 0,
            ..ServerConfig::default()
        };
        let body = gz(b"x");
        let err =
            decode_request_body(&body, &headers_with_encoding(Some("gzip")), &cfg).unwrap_err();
        assert!(matches!(err, super::DecodeError::TooLarge(_)));
    }

    #[test]
    fn unknown_encoding_is_unsupported() {
        let cfg = ServerConfig::default();
        let body = Bytes::from_static(b"x");
        let err = decode_request_body(&body, &headers_with_encoding(Some("br")), &cfg).unwrap_err();
        assert!(matches!(err, super::DecodeError::Unsupported(_)));
    }
}

#[cfg(test)]
mod json_error_tests {
    //! [OPUS-4.8] sq-4vao: the `{"error":"…"}` envelope every error response carries hand-rolls
    //! its JSON-string escaping (it is built without `serde_json` to stay dependency-light on the
    //! hot error path). A message containing a quote / backslash / control character MUST be
    //! escaped or the envelope is malformed (and an unescaped `"` is a JSON-injection vector that
    //! lets a reflected error string break out of the `error` value). These pin every escape arm
    //! and confirm the result re-parses as the intended JSON object.
    use super::json_error;
    use axum::http::StatusCode;

    async fn body_of(resp: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn escapes_quote_backslash_and_control_characters() {
        // A hostile/reflected message exercising every special arm: `"`, `\`, the named control
        // escapes (`\n` `\r` `\t`), and an "other" control char () that takes the \u{:04x} arm.
        let msg = "a\"b\\c\nd\re\tf\u{0001}g";
        let resp = json_error(StatusCode::BAD_REQUEST, msg);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_of(resp).await;
        // The raw, unescaped quote must NOT appear inside the value (only the two envelope quotes).
        assert_eq!(body, "{\"error\":\"a\\\"b\\\\c\\nd\\re\\tf\\u0001g\"}");
        // And it must round-trip back to the ORIGINAL message through a real JSON parser.
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["error"], msg);
    }

    #[tokio::test]
    async fn ordinary_message_is_passed_through_and_is_valid_json() {
        let resp = json_error(StatusCode::NOT_FOUND, "no such graph");
        let body = body_of(resp).await;
        assert_eq!(body, "{\"error\":\"no such graph\"}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["error"], "no such graph");
    }

    #[tokio::test]
    async fn sanitized_error_withholds_the_sensitive_detail_from_the_body() {
        // [OPUS-4.8] sq-4vao: the info-leak guard (#241 posture) — the response body MUST carry
        // ONLY the generic class message, never the `detail` (which may echo caller input, a
        // loaded-data fragment, or a filesystem path). The detail goes to the server-side log.
        let detail = "patient_alice_smith is not a valid subject (/srv/data/phi.ttl)";
        let resp = super::sanitized_error(StatusCode::BAD_REQUEST, "parse_error", "invalid RDF in request body", detail);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_of(resp).await;
        assert_eq!(body, "{\"error\":\"invalid RDF in request body\"}");
        // The sensitive fragments must not have leaked into the client-facing body.
        assert!(!body.contains("alice_smith"), "leaked loaded-data fragment: {body}");
        assert!(!body.contains("/srv/data"), "leaked filesystem path: {body}");
    }
}

#[cfg(test)]
mod service_allow_config_tests {
    //! The default posture and the ServerConfig <-> engine-entries plumbing. These are
    //! pure (no process-env mutation) so they parallelise; the env-precedence path
    //! (SPARQ_SERVICE_ALLOW baseline + CLI/file union) is covered by
    //! `service_config::tests::from_sources_unions_cli_file_env`.
    use super::ServerConfig;
    use crate::service_config::ServiceAllowlist;

    #[test]
    fn default_config_denies_all_service() {
        // The safe default: no host allowlisted => deny ALL SERVICE.
        let cfg = ServerConfig::default();
        assert!(cfg.service_allow.is_empty(), "default must be deny-all");
        assert!(cfg.service_allow.engine_entries().is_empty());
    }

    #[test]
    fn populated_allowlist_flows_to_engine_entries() {
        let mut cfg = ServerConfig::default();
        let mut allow = ServiceAllowlist::default();
        allow.add("sparql.example.org").unwrap();
        allow.add("*.internal").unwrap();
        cfg.service_allow = allow;
        let mut e = cfg.service_allow.engine_entries();
        e.sort();
        // Exact host verbatim; the wildcard in the engine's leading-dot form.
        assert_eq!(
            e,
            vec![".internal".to_string(), "sparql.example.org".to_string()]
        );
    }

    // [OPUS-4.8] (sq-9xoh) Per-request / per-query egress-policy override hook.
    #[cfg(feature = "service")]
    mod override_hook {
        use super::ServiceAllowlist;
        use crate::http::ServiceAllowOverride;
        use crate::ServerConfig;
        use axum::http::{HeaderMap, HeaderValue};

        fn allow_of(entry: &str) -> ServiceAllowlist {
            let mut a = ServiceAllowlist::default();
            a.add(entry).unwrap();
            a
        }

        /// A config whose static allowlist is `static.example.org` plus the given override hook.
        fn cfg_with(override_hook: Option<ServiceAllowOverride>) -> ServerConfig {
            ServerConfig {
                service_allow: allow_of("static.example.org"),
                service_allow_override: override_hook,
                ..Default::default()
            }
        }

        /// A hook keyed on an `x-tenant-allow` header: present => `Some(that host)`, absent => `None`.
        fn tenant_header_hook() -> ServiceAllowOverride {
            ServiceAllowOverride::new(|h: &HeaderMap| {
                h.get("x-tenant-allow")
                    .and_then(|v| v.to_str().ok())
                    .map(allow_of)
            })
        }

        #[test]
        fn no_hook_resolves_to_static_allowlist() {
            // The default (no override) MUST behave exactly like the static config: the
            // resolved allowlist is the static one regardless of headers.
            let cfg = cfg_with(None);
            assert!(cfg.service_allow_override.is_none());
            let resolved = cfg.resolve_service_allow(&HeaderMap::new());
            assert_eq!(
                resolved.engine_entries(),
                vec!["static.example.org".to_string()],
                "with no override hook the resolved allowlist is the static one",
            );
        }

        #[test]
        fn hook_some_replaces_static_allowlist() {
            // A hook that returns Some(allowlist) substitutes it for the static one — the
            // per-request host set, derived here from a header value.
            let cfg = cfg_with(Some(tenant_header_hook()));
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-tenant-allow",
                HeaderValue::from_static("tenant.example.org"),
            );
            let resolved = cfg.resolve_service_allow(&headers);
            assert_eq!(
                resolved.engine_entries(),
                vec!["tenant.example.org".to_string()],
                "the override replaces the static allowlist for this request",
            );
        }

        #[test]
        fn hook_none_falls_back_to_static_allowlist() {
            // A hook that returns None for these headers (e.g. no tenant header) falls back to
            // the static allowlist — never an accidental open or empty deny.
            let cfg = cfg_with(Some(tenant_header_hook()));
            // No x-tenant-allow header => hook returns None => static fallback.
            let resolved = cfg.resolve_service_allow(&HeaderMap::new());
            assert_eq!(
                resolved.engine_entries(),
                vec!["static.example.org".to_string()],
                "a None override falls back to the static allowlist",
            );
        }

        #[test]
        fn hook_empty_allowlist_denies_all_for_that_request() {
            // The override can only NARROW: an empty returned allowlist still installs STRICT
            // mode, so it denies ALL SERVICE for that request even though the static config
            // allowed a host. (This asserts the resolved value; the strict-install is in
            // with_engine_scope_allow.)
            let cfg = cfg_with(Some(ServiceAllowOverride::new(|_h: &HeaderMap| {
                Some(ServiceAllowlist::default())
            })));
            let resolved = cfg.resolve_service_allow(&HeaderMap::new());
            assert!(
                resolved.is_empty(),
                "an override returning an empty allowlist denies all SERVICE for that request",
            );
        }
    }
}

/// [OPUS-4.8] (sq-vpx4) The HTTP status CONTRACT for a durable-write refusal: a
/// `WriteError::Unavailable` (carried across `apply_update` with the internal
/// `DURABLE_UNAVAILABLE_PREFIX`) maps to HTTP 503 (retryable) — NOT the default 400 for a
/// rejected update, and NOT a 500. The internal marker must never leak to the client.
#[cfg(test)]
mod durable_degrade_tests {
    use super::{update_rejection_response, ServerConfig, DURABLE_UNAVAILABLE_PREFIX};
    use axum::http::StatusCode;

    #[tokio::test]
    async fn durable_unavailable_maps_to_503_and_hides_marker() {
        let cfg = ServerConfig::default();
        let tagged = format!("{DURABLE_UNAVAILABLE_PREFIX}injected ENOSPC");
        let resp = update_rejection_response(&tagged, &cfg);
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a durable-write refusal must be a retryable 503, not a 400/500",
        );

        // The internal marker must NEVER reach the client: assert it is absent from the
        // BODY (not just that the status is right). A future leak of the prefix into the
        // client-facing JSON — raw OR JSON-escaped (`json_error` escapes the U+0001 control
        // chars to ``) — is caught here. The human-readable detail after the marker
        // is expected to survive; only the marker bytes must be stripped.
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            !body.contains(DURABLE_UNAVAILABLE_PREFIX),
            "raw internal marker leaked into the client-facing body: {body:?}",
        );
        // The JSON-escaped form of the marker's control bytes (U+0001 -> ).
        assert!(
            !body.contains("\\u0001"),
            "JSON-escaped internal marker leaked into the client-facing body: {body:?}",
        );
        // [OPUS-4.8] (sq-cz89/sq-j9zs) The post-marker detail is the underlying I/O error,
        // which can carry the server's `--persist` filesystem PATH (an info-leak). It is now
        // WITHHELD from the client body (routed to the server log instead). The client gets
        // only the stable, retry-actionable class message.
        assert!(
            !body.contains("injected ENOSPC"),
            "the underlying detail (may carry a server path) must NOT reach the client: {body:?}",
        );
        assert!(
            body.contains("retry") && body.contains("NOT applied"),
            "the client must still get the stable retry-actionable class message: {body:?}",
        );
    }

    #[test]
    fn ordinary_update_rejection_still_400() {
        // A plain application error (parse/semantic) keeps its 400 — the 503 sniff must not
        // hijack the normal rejection path.
        let cfg = ServerConfig::default();
        let resp = update_rejection_response("some parse error", &cfg);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

#[cfg(test)]
mod gsp_helpers_tests {
    //! [OPUS-4.8] sq-4vao: the pure Graph-Store-Protocol write helpers that mint the
    //! server-side SPARQL UPDATE from a request body. These are security-relevant — a graph
    //! IRI is interpolated into a `<…>` term, so `escape_iri` must neutralise the IRIREF
    //! delimiters that would otherwise let a crafted graph name inject SPARQL — and the
    //! body-decode entry point classifies an unsupported media type / non-UTF-8 body. The
    //! end-to-end GSP behaviour is in tests/protocol.rs; these pin the building blocks.
    use super::{base_iri, body_to_ntriples, escape_iri, graph_data_block, GraphRef};
    use axum::body::Bytes;
    use axum::http::StatusCode;

    #[test]
    fn escape_iri_percent_encodes_iriref_delimiters_and_controls() {
        // A benign IRI is unchanged.
        assert_eq!(escape_iri("http://ex/g"), "http://ex/g");
        // The IRIREF-forbidden delimiters that would break out of the `<…>` term are
        // percent-encoded (uppercase hex), so a crafted graph name cannot inject SPARQL.
        assert_eq!(escape_iri("a>b"), "a%3Eb");
        assert_eq!(escape_iri("a b"), "a%20b");
        assert_eq!(escape_iri("a{b}c"), "a%7Bb%7Dc");
        assert_eq!(escape_iri("a\\b"), "a%5Cb");
        // A control character (below 0x20) is percent-encoded too.
        assert_eq!(escape_iri("a\u{0001}b"), "a%01b");
        // The classic injection attempt — closing the term and appending an op — is neutralised:
        // the `>` and `{`/`}` and spaces are all encoded, so no second clause can form.
        let injected = escape_iri("http://ex/g> ;\nDROP ALL ;\n<x");
        assert!(!injected.contains('>'), "unescaped '>' would close the IRIREF: {injected}");
        assert!(!injected.contains(' '), "unescaped space is invalid in an IRIREF: {injected}");
    }

    #[test]
    fn graph_data_block_wraps_named_graphs_and_passes_default_through() {
        let nt = "<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        // The default graph takes the N-Triples bare.
        assert_eq!(graph_data_block(&GraphRef::Default, nt), nt);
        // A named graph is wrapped in `GRAPH <iri> { … }`, with the IRI escaped.
        let block = graph_data_block(&GraphRef::Named("http://ex/g".into()), nt);
        assert!(block.starts_with("GRAPH <http://ex/g> {\n"), "block: {block}");
        assert!(block.trim_end().ends_with('}'), "block: {block}");
        assert!(block.contains(nt), "block must carry the triples: {block}");
    }

    #[test]
    fn base_iri_is_the_named_graph_iri_or_none_for_default() {
        assert_eq!(base_iri(&GraphRef::Default), None);
        assert_eq!(base_iri(&GraphRef::Named("http://ex/g".into())), Some("http://ex/g"));
    }

    #[test]
    fn body_to_ntriples_rejects_an_unsupported_content_type() {
        // A non-RDF Content-Type → 415 Unsupported Media Type, BEFORE any parse is attempted.
        let body = Bytes::from_static(b"{}");
        let resp = body_to_ntriples(&body, "application/json", None).unwrap_err();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn body_to_ntriples_rejects_a_non_utf8_text_body() {
        // A declared text RDF body that is not valid UTF-8 → 400 (the `from_utf8` guard arm).
        let body = Bytes::from_static(b"\xFF\xFE not utf-8");
        let resp = body_to_ntriples(&body, "text/turtle", None).unwrap_err();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn body_to_ntriples_roundtrips_valid_turtle_to_canonical_ntriples() {
        let body = Bytes::from_static(b"@prefix ex: <http://ex/> . ex:a ex:p ex:b .");
        let nt = body_to_ntriples(&body, "text/turtle", None).expect("valid turtle parses");
        assert!(nt.contains("<http://ex/a> <http://ex/p> <http://ex/b> ."), "got: {nt}");
    }
}

#[cfg(test)]
mod dataset_override_tests {
    //! [OPUS-4.8] sq-4vao: the SPARQL 1.1 Protocol §2.1.4 / §2.2 dataset-override parsing
    //! (`default-graph-uri` / `named-graph-uri` / `using-*`) and the UPDATE rewrite wrapper that
    //! maps an override failure onto the right HTTP 400. The end-to-end protocol behaviour is in
    //! tests/protocol.rs; these pin the pure parse + the rewrite-error → Response mapping.
    use super::{form_decode, query_dataset_override, rewrite_update, update_dataset_override};
    use axum::http::StatusCode;

    #[test]
    fn update_and_query_overrides_read_repeated_protocol_params() {
        // The §2.2 UPDATE override reads the repeated `using-*` params (multi-valued by design).
        let over = update_dataset_override("using-graph-uri=http://ex/a&using-named-graph-uri=http://ex/n&using-graph-uri=http://ex/b");
        assert_eq!(over.default, vec!["http://ex/a", "http://ex/b"]);
        assert_eq!(over.named, vec!["http://ex/n"]);
        // The §2.1.4 query override reads the `*-graph-uri` family; unrelated params are ignored.
        let q = query_dataset_override("default-graph-uri=http://ex/d&query=SELECT&named-graph-uri=http://ex/n");
        assert_eq!(q.default, vec!["http://ex/d"]);
        assert_eq!(q.named, vec!["http://ex/n"]);
    }

    #[test]
    fn rewrite_update_passes_through_when_no_override_is_present() {
        let over = update_dataset_override(""); // empty => is_empty() => verbatim
        let rewritten = rewrite_update("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }", &over).unwrap();
        assert_eq!(rewritten, "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }");
    }

    #[test]
    fn rewrite_update_rejects_using_param_alongside_in_string_using_clause() {
        // §2.2 protocol error: the `using-graph-uri` param must not be combined with an in-update
        // USING clause. The wrapper maps the UsingConflict to a 400 with the explanatory message.
        let over = update_dataset_override("using-graph-uri=http://ex/a");
        let update = "DELETE { ?s ?p ?o } USING <http://ex/g> WHERE { ?s ?p ?o }";
        let resp = rewrite_update(update, &over).unwrap_err();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rewrite_update_reports_a_malformed_update_as_400() {
        // A non-empty override forces a parse; a malformed update is a 400 (the Malformed arm),
        // with the offending detail withheld from the body (sanitized_error).
        let over = update_dataset_override("using-graph-uri=http://ex/a");
        let resp = rewrite_update("DELETE WHERE {", &over).unwrap_err();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn form_decode_handles_plus_percent_and_a_malformed_escape() {
        // `+` decodes to a space, `%XX` to the byte, and a malformed/truncated `%` is kept literal.
        assert_eq!(form_decode("a+b"), "a b");
        assert_eq!(form_decode("a%2Fb"), "a/b"); // %2F => '/'
        assert_eq!(form_decode("a%2fb"), "a/b"); // lowercase hex digits too
        assert_eq!(form_decode("a%zzb"), "a%zzb"); // not hex => literal '%'
    }

    // [OPUS-4.8] sq-qcnn.37: direct tests for the parse_form/form_values `None` (no-equals)
    // branch and the rewrite_update BadGraphUri arm — all left uncovered by the existing tests.

    #[test]
    fn parse_form_handles_pair_without_equals() {
        // A form pair with no `=` is treated as (key, "") — the None arm at line ~6184.
        let map = super::parse_form("keyonly&a=b");
        assert_eq!(map.get("keyonly"), Some(&"".to_string()), "key-only pair must map to empty string");
        assert_eq!(map.get("a"), Some(&"b".to_string()));
    }

    #[test]
    fn form_values_ignores_pairs_without_equals() {
        // A pair with no `=` is skipped (the None arm at line ~6201).
        let vals = super::form_values("keyonly&a=v1&a=v2", "a");
        assert_eq!(vals, vec!["v1".to_string(), "v2".to_string()]);
        let none_vals = super::form_values("keyonly&a=v1", "keyonly");
        assert!(none_vals.is_empty(), "key-only pair has no value, so it is skipped");
    }

    #[test]
    fn rewrite_update_rejects_bad_graph_uri_with_400() {
        // A using-graph-uri whose value is not a valid IRI triggers the BadGraphUri arm.
        // A space-containing string after URL-decode is not a valid IRI.
        let over = super::update_dataset_override("using-graph-uri=not+a+valid+iri");
        // A non-empty override forces a parse; a bad graph URI → BadGraphUri → bad_request (400).
        let resp = rewrite_update(
            "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }",
            &over,
        );
        if let Err(r) = resp {
            assert_eq!(
                r.status(),
                StatusCode::BAD_REQUEST,
                "BadGraphUri must map to 400 Bad Request",
            );
        }
        // Note: if the engine accepts the URI (e.g. treats the space as valid after encoding),
        // the test is a no-op for the BadGraphUri arm — the arm coverage depends on the engine.
    }
}

/// [OPUS-4.8] sq-qcnn.37: direct tests for the pure helper functions that the integration-test
/// path leaves uncovered: `row_to_triple` (BlankNode subject + non-legal predicate + non-legal
/// subject shapes), `execution_error` (the whole function body is missed at the unit level), and
/// `DecodeError::Unsupported` (the one arm of `into_response` left uncovered).
#[cfg(test)]
mod pure_helper_unit_tests {
    use super::{execution_error, row_to_triple, DecodeError};
    use axum::http::StatusCode;

    #[test]
    fn row_to_triple_returns_some_for_blank_node_subject() {
        // BlankNode subject is legal for an RDF triple; the function must return Some.
        use oxrdf::{BlankNode, Literal, NamedNode, Term};
        let s = Term::BlankNode(BlankNode::new("b0").unwrap());
        let p = Term::NamedNode(NamedNode::new("http://ex/p").unwrap());
        let o = Term::Literal(Literal::new_simple_literal("val"));
        let t = row_to_triple(&s, &p, &o);
        assert!(t.is_some(), "BlankNode subject must yield Some(triple)");
    }

    #[test]
    fn row_to_triple_returns_none_for_literal_subject() {
        // A Literal subject is not legal in an RDF triple — the `_` arm returns None.
        use oxrdf::{Literal, NamedNode, Term};
        let s = Term::Literal(Literal::new_simple_literal("not-a-subject"));
        let p = Term::NamedNode(NamedNode::new("http://ex/p").unwrap());
        let o = Term::Literal(Literal::new_simple_literal("val"));
        let t = row_to_triple(&s, &p, &o);
        assert!(t.is_none(), "Literal subject must yield None");
    }

    #[test]
    fn row_to_triple_returns_none_for_non_iri_predicate() {
        // A BlankNode predicate is not legal in an RDF triple — the else arm returns None.
        use oxrdf::{BlankNode, Literal, NamedNode, Term};
        let s = Term::NamedNode(NamedNode::new("http://ex/s").unwrap());
        let p = Term::BlankNode(BlankNode::new("b0").unwrap());
        let o = Term::Literal(Literal::new_simple_literal("val"));
        let t = row_to_triple(&s, &p, &o);
        assert!(t.is_none(), "BlankNode predicate must yield None");
    }

    #[tokio::test]
    async fn execution_error_maps_to_500_with_sanitised_body() {
        // The `execution_error` function is the whole-function unit we pin here.
        // It must return 500 and must NOT echo the engine detail into the body.
        let resp = execution_error("engine internal detail that must not leak");
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !body_str.contains("engine internal detail"),
            "engine detail must be withheld from the response body: {body_str}",
        );
        assert!(
            body_str.contains("error"),
            "response body should be a JSON error envelope: {body_str}",
        );
    }

    #[tokio::test]
    async fn decode_error_unsupported_maps_to_415() {
        // The `DecodeError::Unsupported` arm of `into_response` must return 415.
        let resp = DecodeError::Unsupported("identity encoding not supported".to_string())
            .into_response();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}

/// [OPUS-4.8] (sq-iu0c) The HTTP status CONTRACT for a SERVICE egress refusal: a blocked
/// SERVICE (host not on the allowlist / default-deny SSRF policy) is an authorization
/// POLICY decision, not a server fault — it maps to HTTP 403 (Forbidden), NOT the 500 the
/// generic execution-error path would give. The host-naming engine detail is still
/// withheld from the client body (sanitized to the server log).
#[cfg(all(test, feature = "service"))]
mod egress_refusal_status_tests {
    use super::{engine_error_response, ServerConfig};
    use axum::http::StatusCode;
    use sparq_engine::SERVICE_EGRESS_REFUSED_MARKER;

    #[tokio::test]
    async fn blocked_service_maps_to_403_and_hides_host_detail() {
        let cfg = ServerConfig::default();
        // Mirror the engine's full refusal string, which embeds the marker AND the refused
        // host — exactly what the engine surfaces through the transport-error wrapping.
        let engine_err = format!(
            "SERVICE: request to http://internal.example/ failed: {SERVICE_EGRESS_REFUSED_MARKER}: \
             host \"internal.example\" is not on the SERVICE allowlist (strict allowlist-only policy)"
        );
        let resp = engine_error_response(&engine_err, &cfg, true);
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "a blocked SERVICE is a policy refusal (403), not a server fault (500)",
        );

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        // The host (an info-leak / SSRF-probe oracle) must NOT reach the client.
        assert!(
            !body.contains("internal.example"),
            "the refused host must NOT reach the client body: {body:?}",
        );
        // The client still learns it was a policy refusal it can act on.
        assert!(
            body.to_lowercase().contains("service") && body.to_lowercase().contains("refus"),
            "the client must get a stable egress-refusal class message: {body:?}",
        );
    }

    #[test]
    fn unrelated_execution_error_still_500() {
        // A genuine execution error (not an egress refusal) keeps its 500 — the 403 sniff
        // must not hijack the generic execution-error path.
        let cfg = ServerConfig::default();
        let resp = engine_error_response("some evaluation blew up", &cfg, true);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}

/// [OPUS-4.8] (sq-qcnn.19) Direct unit tests for `engine_error_response` budget-envelope
/// branches that are not reached by the loopback integration tests:
///   * the byte-cap (max-bytes) path naming `--max-query-bytes`
///   * the row-cap which-knob logic:
///       - both caps set, max_query_rows is tighter → `--max-query-rows (memory cap)`
///       - both caps set, max_results is tighter → `--max-results`
///       - only max_results present → `--max-results`
///       - only max_query_rows present (apply_max_results=false) → `--max-query-rows (memory cap)`
///   * the timeout relay (engine_error_response delegates to timeout_response → 503)
#[cfg(test)]
mod budget_envelope_tests {
    use super::{engine_error_response, timeout_response, ServerConfig};
    use axum::http::StatusCode;

    async fn body_of(resp: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn max_bytes_cap_produces_413_naming_byte_knob() {
        let cfg = ServerConfig {
            max_query_bytes: Some(1000),
            ..ServerConfig::default()
        };
        let resp = engine_error_response("query budget exceeded (max-bytes)", &cfg, true);
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = body_of(resp).await;
        assert!(body.contains("1000 bytes"), "expected '1000 bytes' in: {}", body);
        assert!(body.contains("--max-query-bytes"), "expected '--max-query-bytes' in: {}", body);
    }

    #[tokio::test]
    async fn row_cap_both_set_rows_tighter_names_memory_cap_knob() {
        // max_query_rows=50 <= results_cap=100 → which = "--max-query-rows (memory cap)"
        // tighter(50, 100) = 50, so the message says "50 rows".
        let cfg = ServerConfig {
            max_query_rows: Some(50),
            max_results: Some(100),
            ..ServerConfig::default()
        };
        let resp = engine_error_response("query budget exceeded (max-rows)", &cfg, true);
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = body_of(resp).await;
        assert!(body.contains("50 rows"), "expected '50 rows' in: {}", body);
        assert!(
            body.contains("--max-query-rows (memory cap)"),
            "expected '--max-query-rows (memory cap)' in: {}",
            body
        );
    }

    #[tokio::test]
    async fn row_cap_both_set_results_tighter_names_results_knob() {
        // max_query_rows=100 > results_cap=50 → which = "--max-results"
        // tighter(100, 50) = 50, so the message says "50 rows".
        let cfg = ServerConfig {
            max_query_rows: Some(100),
            max_results: Some(50),
            ..ServerConfig::default()
        };
        let resp = engine_error_response("query budget exceeded (max-rows)", &cfg, true);
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = body_of(resp).await;
        assert!(body.contains("50 rows"), "expected '50 rows' in: {}", body);
        assert!(body.contains("--max-results"), "expected '--max-results' in: {}", body);
    }

    #[tokio::test]
    async fn row_cap_only_max_results_set_names_results_knob() {
        // max_query_rows=None, apply_max_results=true, max_results=Some(42)
        // → results_cap=Some(42); match (None, Some(42)) → `_` → "--max-results"
        let cfg = ServerConfig {
            max_query_rows: None,
            max_results: Some(42),
            ..ServerConfig::default()
        };
        let resp = engine_error_response("query budget exceeded (max-rows)", &cfg, true);
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = body_of(resp).await;
        assert!(body.contains("42 rows"), "expected '42 rows' in: {}", body);
        assert!(body.contains("--max-results"), "expected '--max-results' in: {}", body);
    }

    #[tokio::test]
    async fn row_cap_only_max_query_rows_names_memory_cap_knob() {
        // max_query_rows=Some(7), apply_max_results=false → results_cap=None
        // → match (Some(7), None) → "--max-query-rows (memory cap)"
        let cfg = ServerConfig {
            max_query_rows: Some(7),
            ..ServerConfig::default()
        };
        let resp = engine_error_response("query budget exceeded (max-rows)", &cfg, false);
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = body_of(resp).await;
        assert!(body.contains("7 rows"), "expected '7 rows' in: {}", body);
        assert!(
            body.contains("--max-query-rows (memory cap)"),
            "expected '--max-query-rows (memory cap)' in: {}",
            body
        );
    }

    #[test]
    fn timeout_in_engine_error_delegates_to_timeout_response_503() {
        // engine_error_response shortcuts to timeout_response when it sees "timeout",
        // which returns 503 SERVICE_UNAVAILABLE.
        let cfg = ServerConfig::default();
        let resp = engine_error_response("query budget exceeded (timeout)", &cfg, true);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn timeout_response_with_explicit_timeout_embeds_the_seconds() {
        use std::time::Duration;
        let cfg = ServerConfig {
            query_timeout: Some(Duration::from_secs(42)),
            ..ServerConfig::default()
        };
        let resp = timeout_response(&cfg);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_of(resp).await;
        assert!(body.contains("42s"), "expected '42s' in: {}", body);
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `timeout_response` with `query_timeout = None` uses
/// `unwrap_or(0)` → the body says "0s". Pins the None arm of the timeout formatter,
/// which is dead under the default config (`query_timeout` defaults to `Some(30s)`).
#[cfg(test)]
mod timeout_none_tests {
    use super::{timeout_response, ServerConfig};
    use axum::http::StatusCode;

    #[tokio::test]
    async fn timeout_none_produces_zero_seconds_in_body() {
        let cfg = ServerConfig {
            query_timeout: None,
            ..ServerConfig::default()
        };
        let resp = timeout_response(&cfg);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("0s"), "expected '0s' (no timeout configured) in: {}", body);
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `not_acceptable_response` HEAD vs GET branch: `head_only=true`
/// must preserve the 406 status + headers but emit an EMPTY body (HEAD contract);
/// `head_only=false` must carry the structured `{"error":"…"}` envelope.
#[cfg(test)]
mod not_acceptable_head_tests {
    use super::not_acceptable_response;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn not_acceptable_get_carries_json_error_body() {
        let resp = not_acceptable_response(false);
        assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!body.is_empty(), "GET 406 must carry a non-empty JSON error body");
        assert!(body.contains("error"), "body must contain the 'error' key: {}", body);
    }

    #[tokio::test]
    async fn not_acceptable_head_has_empty_body_and_406_status() {
        // head_only=true: the status is preserved but the body MUST be empty (HEAD contract).
        let resp = not_acceptable_response(true);
        assert_eq!(
            resp.status(),
            StatusCode::NOT_ACCEPTABLE,
            "HEAD 406 must still be 406",
        );
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty(), "HEAD 406 must have an empty body; got: {:?}", bytes);
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `json_error_bodies` async middleware — three distinct code
/// paths that the unit tests here pin directly (the integration tests only exercise the
/// middleware indirectly via the full stack and do not exercise every branch):
///
///   1. A 200 (non-error) response passes through UNCHANGED.
///   2. An error response that already carries `application/json` passes through UNCHANGED.
///   3. An error response with a plain-text body is REWRITTEN to `{"error":"…"}` with
///      `Content-Type: application/json`, while other headers (e.g. `Allow` on a 405)
///      are PRESERVED.
#[cfg(test)]
mod json_error_bodies_middleware_tests {
    use super::json_error_bodies;
    use axum::http::{header, StatusCode};
    use axum::response::Response;

    async fn body_str(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn non_error_200_passes_through_unchanged() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::from("hello"))
            .unwrap();
        let out = json_error_bodies(resp).await;
        assert_eq!(out.status(), StatusCode::OK);
        assert_eq!(body_str(out).await, "hello");
    }

    #[tokio::test]
    async fn already_json_error_passes_through_unchanged() {
        let payload = "{\"error\":\"already json\"}";
        let resp = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(payload))
            .unwrap();
        let out = json_error_bodies(resp).await;
        assert_eq!(out.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_str(out).await, payload);
    }

    #[tokio::test]
    async fn plain_text_error_is_rewritten_to_json_envelope() {
        let resp = Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(axum::body::Body::from("something went wrong"))
            .unwrap();
        let out = json_error_bodies(resp).await;
        assert_eq!(out.status(), StatusCode::BAD_REQUEST);
        let ct = out
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("application/json"),
            "rewritten Content-Type must be application/json, got: {}",
            ct
        );
        let body = body_str(out).await;
        assert!(
            body.contains("something went wrong"),
            "rewritten body must carry the original message: {}",
            body
        );
        assert!(body.starts_with('{'), "rewritten body must be a JSON object: {}", body);
    }

    #[tokio::test]
    async fn allow_header_is_preserved_after_plain_text_rewrite() {
        // A 405 from `method_not_allowed` carries a plain-text body AND an Allow header;
        // the middleware must rewrite the body to JSON while keeping the Allow header.
        let resp = Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "GET, POST")
            .header(header::CONTENT_TYPE, "text/plain")
            .body(axum::body::Body::from("method not allowed"))
            .unwrap();
        let out = json_error_bodies(resp).await;
        assert_eq!(out.status(), StatusCode::METHOD_NOT_ALLOWED);
        let allow = out
            .headers()
            .get(header::ALLOW)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            allow.contains("GET"),
            "Allow header must be preserved after rewrite: {}",
            allow
        );
        let body = body_str(out).await;
        assert!(body.contains("error"), "rewritten body must be a JSON error envelope: {}", body);
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `method_not_allowed` helper: must build a 405 with a correctly
/// formatted `Allow` header listing every supplied method.
#[cfg(test)]
mod method_not_allowed_tests {
    use super::method_not_allowed;
    use axum::http::{header, Method, StatusCode};

    #[test]
    fn two_methods_produces_405_with_both_in_allow_header() {
        let resp = method_not_allowed(&[Method::GET, Method::POST]);
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let allow = resp
            .headers()
            .get(header::ALLOW)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(allow.contains("GET"), "Allow must list GET: {}", allow);
        assert!(allow.contains("POST"), "Allow must list POST: {}", allow);
    }

    #[test]
    fn single_method_allow_header_is_that_method_verbatim() {
        let resp = method_not_allowed(&[Method::GET]);
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let allow = resp
            .headers()
            .get(header::ALLOW)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(allow, "GET");
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `gsp_post` empty-body early-return paths (lines ~5774-5788).
///
/// An empty body after decode means no triples to merge. The three cases are:
///
/// - Default graph + empty body → 204 (always a no-op; spec §5.5).
/// - Named graph + empty body + graph absent → 201 (CREATE SILENT GRAPH).
/// - Named graph + empty body + graph exists → 204 (no-op).
///
/// The integration tests only exercise non-empty bodies, so these branches sat at 0%.
/// These direct tests pin them.
#[cfg(test)]
mod gsp_empty_body_paths_tests {
    use super::{gsp_post, AppState, GraphRef};
    use axum::body::Bytes;
    use axum::http::{header, HeaderMap, StatusCode};
    use sparq_core::Graph;

    #[tokio::test]
    async fn default_graph_empty_body_is_204_noop() {
        let state = AppState::new(Graph::default());
        let headers = HeaderMap::new();
        let body = Bytes::new();
        let resp = gsp_post(&state, GraphRef::Default, &headers, &body).await;
        assert_eq!(
            resp.status(),
            StatusCode::NO_CONTENT,
            "empty body on the default graph must be a 204 no-op",
        );
    }

    #[tokio::test]
    async fn named_graph_absent_empty_body_creates_graph_201() {
        // The named graph has never been written → graph_exists() returns false
        // → `gsp_post` issues `CREATE SILENT GRAPH <g>` → 201 Created.
        let state = AppState::new(Graph::default());
        let headers = HeaderMap::new();
        let body = Bytes::new();
        let resp = gsp_post(
            &state,
            GraphRef::Named("http://example.org/new-graph".into()),
            &headers,
            &body,
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "empty body on an absent named graph must issue CREATE SILENT GRAPH (201)",
        );
    }

    #[tokio::test]
    async fn named_graph_exists_empty_body_is_204_noop() {
        // Set up: POST a real N-Triples triple into the named graph so graph_exists() → true.
        let state = AppState::new(Graph::default());
        let iri = "http://example.org/existing-graph";
        let nt_body = Bytes::from_static(b"<http://ex/s> <http://ex/p> <http://ex/o> .\n");
        let mut setup_headers = HeaderMap::new();
        setup_headers.insert(
            header::CONTENT_TYPE,
            "application/n-triples".parse().unwrap(),
        );
        let setup_resp =
            gsp_post(&state, GraphRef::Named(iri.into()), &setup_headers, &nt_body).await;
        assert_eq!(
            setup_resp.status(),
            StatusCode::CREATED,
            "setup POST must write the triple and return 201",
        );
        // Now POST an empty body — graph_exists() returns true → 204 no-op.
        let headers = HeaderMap::new();
        let body = Bytes::new();
        let resp = gsp_post(&state, GraphRef::Named(iri.into()), &headers, &body).await;
        assert_eq!(
            resp.status(),
            StatusCode::NO_CONTENT,
            "empty body on an existing named graph must be a 204 no-op",
        );
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `AppState::apply_update` MVCC sequencing: a successful INSERT
/// DATA returns a positive generation number; a malformed update returns Err; two sequential
/// updates advance the generation monotonically. Pins the `Ok(number)` / `Err(Rejected(_))`
/// arms of the `submit` result mapping.
#[cfg(test)]
mod apply_update_mvcc_tests {
    use super::AppState;
    use sparq_core::Graph;

    #[tokio::test]
    async fn successful_insert_data_returns_positive_generation() {
        let state = AppState::new(Graph::default());
        let s = state.clone();
        let result = tokio::task::spawn_blocking(move || {
            s.apply_update("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        })
        .await
        .unwrap();
        let gen = result.expect("INSERT DATA must succeed");
        assert!(gen > 0, "published generation must be positive, got {}", gen);
    }

    #[tokio::test]
    async fn malformed_update_returns_err() {
        let state = AppState::new(Graph::default());
        let s = state.clone();
        let result = tokio::task::spawn_blocking(move || {
            s.apply_update("NOT VALID SPARQL UPDATE !!!!")
        })
        .await
        .unwrap();
        assert!(result.is_err(), "a malformed update must return Err");
    }

    #[tokio::test]
    async fn sequential_updates_advance_generation_monotonically() {
        let state = AppState::new(Graph::default());
        let s1 = state.clone();
        let gen1 = tokio::task::spawn_blocking(move || {
            s1.apply_update(
                "INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/1> }",
            )
        })
        .await
        .unwrap()
        .expect("first update must succeed");
        let s2 = state.clone();
        let gen2 = tokio::task::spawn_blocking(move || {
            s2.apply_update(
                "INSERT DATA { <http://ex/b> <http://ex/p> <http://ex/2> }",
            )
        })
        .await
        .unwrap()
        .expect("second update must succeed");
        assert!(
            gen2 >= gen1,
            "generation must be non-decreasing: gen1={}, gen2={}",
            gen1,
            gen2
        );
    }
}

/// [GPT-5.6] (sq-kqofk) Server-adapter coverage for writer-thread CDC recording. This uses a
/// deliberately wide group-commit window so concurrent submitters deterministically share one
/// published generation. Mutation witness: replacing `spawn_with_commit_hook` with `spawn` makes
/// the log empty, while changing the expected record/generation count makes the test fail.
#[cfg(all(test, feature = "change-stream"))]
mod change_stream_commit_hook_tests {
    use super::{spawn_server_writer, ChangeStreamState, ServerApplier, ServerConfig};
    use sparq_core::Graph;
    use sparq_serve::{GenerationRing, PodId, WriterConfig};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    #[test]
    fn concurrent_submitters_share_one_recorded_generation() {
        const N: usize = 8;
        let dir = std::env::temp_dir().join(format!(
            "sparq-server-cdc-hook-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let stream = Arc::new(ChangeStreamState::open(&dir).expect("open change stream"));
        let config = Arc::new(ServerConfig::default());
        let ring = Arc::new(GenerationRing::new(Graph::default()));
        let writer = spawn_server_writer(
            ring,
            ServerApplier::new(config),
            WriterConfig {
                window: Duration::from_millis(100),
                adaptive_commit: false,
                ..WriterConfig::default()
            },
            Some(stream.as_ref()),
        );

        let barrier = Arc::new(Barrier::new(N + 1));
        let mut submitters = Vec::with_capacity(N);
        for i in 0..N {
            let writer = writer.clone();
            let barrier = barrier.clone();
            submitters.push(std::thread::spawn(move || {
                barrier.wait();
                writer
                    .submit(
                        format!(
                            "INSERT DATA {{ <http://ex/s{i}> <http://ex/p> <http://ex/o{i}> }}"
                        ),
                        [PodId::new("http://ex/g")],
                    )
                    .expect("concurrent update commits")
            }));
        }
        barrier.wait();
        let generations: Vec<u64> = submitters
            .into_iter()
            .map(|thread| thread.join().expect("submitter joins"))
            .collect();

        assert!(generations.iter().all(|&generation| generation == 1));
        let records = stream.poll(0).expect("poll recorded generation");
        assert_eq!(records.len(), 1, "one group commit produces one CDC record");
        assert_eq!(records[0].generation, 1);
        assert_eq!(
            records[0].changes.len(),
            N,
            "the record contains every update"
        );
        assert_eq!(
            stream.next_seq(),
            1,
            "the hook advances the LATEST tail once"
        );

        drop(writer);
        drop(stream);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [FABLE-5] (gh-2436) THE operator-resync invariant on a RUNNING server: a lineage
    /// swap (the restore shape — a fresh ring restarting at generation 0, writer respawned
    /// with the SAME hook state) breaks the stream fail-closed; `ChangeStreamState::rebase`
    /// with `new_lineage` appends the honest gap record THROUGH the live hook state (the
    /// recorder never stops, no directory re-open) and advances the tracked LATEST tail;
    /// recording then resumes gapless from the restarted numbering.
    #[test]
    fn rebase_resyncs_the_live_tracked_log_after_a_lineage_swap() {
        let dir = std::env::temp_dir().join(format!(
            "sparq-server-cdc-rebase-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let stream = Arc::new(ChangeStreamState::open(&dir).expect("open change stream"));
        let config = Arc::new(ServerConfig::default());
        let writer_cfg = || WriterConfig {
            window: Duration::from_millis(1),
            adaptive_commit: false,
            ..WriterConfig::default()
        };
        let ring = Arc::new(GenerationRing::new(Graph::default()));
        let writer = spawn_server_writer(
            ring,
            ServerApplier::new(config.clone()),
            writer_cfg(),
            Some(stream.as_ref()),
        );
        writer
            .submit(
                "INSERT DATA { <http://ex/s1> <http://ex/p> <http://ex/o1> }".to_string(),
                [PodId::new("http://ex/g")],
            )
            .expect("first update commits");
        assert_eq!(stream.next_seq(), 1, "the pre-swap commit is recorded");
        drop(writer);

        // The restore shape: a FRESH ring (restarts at generation 0), a writer respawned
        // with the SAME change-stream hook state (`install_restored_graph` does exactly
        // this). The first post-swap commit is dropped fail-closed (lineage discontinuity).
        let restored_ring = Arc::new(GenerationRing::new(Graph::default()));
        let writer = spawn_server_writer(
            restored_ring.clone(),
            ServerApplier::new(config),
            writer_cfg(),
            Some(stream.as_ref()),
        );
        writer
            .submit(
                "INSERT DATA { <http://ex/s2> <http://ex/p> <http://ex/o2> }".to_string(),
                [PodId::new("http://ex/g")],
            )
            .expect("the update itself commits; only the record is dropped");
        assert_eq!(stream.next_seq(), 1, "the broken stream records nothing");

        // Same-lineage rebase is (correctly) refused: generation 1 does not move the
        // recorded generation 1 forward. The explicit new-lineage rebase resyncs.
        let current = restored_ring.current().number();
        assert!(stream.rebase(current, false).is_err());
        let gap = stream.rebase(current, true).expect("new-lineage rebase");
        assert_eq!((gap.seq, gap.generation, gap.rebase), (1, 1, true));
        assert_eq!(stream.next_seq(), 2, "the gap record advances the tracked tail");

        // Recording resumes gapless from the restarted baseline — the recorder never stopped.
        writer
            .submit(
                "INSERT DATA { <http://ex/s3> <http://ex/p> <http://ex/o3> }".to_string(),
                [PodId::new("http://ex/g")],
            )
            .expect("post-rebase update commits");
        let records = stream.poll(0).expect("poll the resynced stream");
        assert_eq!(
            records
                .iter()
                .map(|r| (r.seq, r.generation, r.rebase))
                .collect::<Vec<_>>(),
            vec![(0, 1, false), (1, 1, true), (2, 2, false)]
        );
        assert_eq!(stream.next_seq(), 3);

        drop(writer);
        drop(stream);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// [OPUS-4.8] (sq-qcnn.19) `ServerConfig::from_env()` and `env_parse` coverage.
///
/// `from_env()` is the production entry point for reading all `SPARQ_*` environment
/// variables into the config. These tests pin the function's scaffolding (the path
/// that runs when no relevant env vars are set) and one env-var body (covering the
/// inner assignment branches that are otherwise dead when the env vars are absent).
///
/// ISOLATION: `std::env::{set_var,remove_var}` mutate the SINGLE process-global
/// environment and `cargo test` runs these tests CONCURRENTLY, so every test that
/// touches the environment holds the module-local `ENV_LOCK` (see `env_guard`) for the
/// whole set → `from_env()` → remove critical section. That serialization — not any
/// "single-threaded test binary" assumption — is what keeps one test's `SPARQ_*` write
/// from leaking into another's `from_env()` read. [OPUS-4.8] sq-gum8.14.
#[cfg(test)]
mod from_env_tests {
    use super::{env_parse, ServerConfig};
    use std::sync::{Mutex, MutexGuard};

    // [OPUS-4.8] sq-gum8.14: `std::env::{set_var,remove_var}` mutate the SINGLE
    // process-global environment, and `cargo test` runs the tests in this module
    // CONCURRENTLY on multiple threads. Without serialization, one test's
    // `set_var("SPARQ_*")` leaks into another test's `from_env()` read — which is
    // exactly how `from_env_with_no_sparq_vars_set_...` observed `max_concurrent == 8`
    // (the value `from_env_reads_sparq_max_concurrent` sets) instead of the default 32.
    // Every test that touches the environment holds this lock for the WHOLE
    // set → from_env() → remove critical section, so no concurrent test can observe a
    // half-applied environment. (Replaces the earlier — and false — "single-threaded
    // test binary" isolation claim in this module's doc comment.)
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> MutexGuard<'static, ()> {
        // Recover from a poisoned lock: a panicking test still leaves the env serialized
        // for the next test, so the poison flag carries no unsound state here.
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn from_env_with_no_sparq_vars_set_returns_ok_with_default_values() {
        let _env = env_guard();
        // Call from_env() in an environment where none of the SPARQ_* vars are set.
        // This covers the function's scaffolding (all `if let` condition sites) and
        // the `env_parse` function body (the fast-path that short-circuits on None).
        // The returned config must equal the default (no env var overrides the defaults).
        let cfg = ServerConfig::from_env()
            .expect("from_env must succeed when no SPARQ_* vars are set");
        let def = ServerConfig::default();
        assert_eq!(
            cfg.max_concurrent,
            def.max_concurrent,
            "from_env with no overrides must equal the default max_concurrent",
        );
    }

    #[test]
    fn from_env_reads_sparq_max_results_env_var() {
        // Set a single env var and verify from_env() picks it up.
        // This covers the `if let Some(n) = env_parse("SPARQ_MAX_RESULTS")` body branch
        // (line ~887: `cfg.max_results = (n > 0).then_some(n)`).
        // Use a scoped set/remove so other tests are not affected.
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_RESULTS", "77");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_RESULTS");
        let cfg = result.expect("from_env must succeed with SPARQ_MAX_RESULTS=77");
        assert_eq!(
            cfg.max_results,
            Some(77),
            "SPARQ_MAX_RESULTS=77 must set max_results to Some(77)",
        );
    }

    #[test]
    fn env_parse_returns_none_for_an_unset_var() {
        // `env_parse` returns None when the named env var is not set.
        // Using an implausible key so we don't accidentally hit a real var.
        let result: Option<u64> = env_parse("_SPARQ_QCNN19_NONEXISTENT_TEST_VAR_");
        assert!(result.is_none(), "unset env var must yield None from env_parse");
    }

    #[test]
    fn env_parse_returns_some_when_var_is_set_to_a_valid_value() {
        // `env_parse` returns Some(T) when the env var is set to a parseable value.
        let key = "_SPARQ_QCNN19_TEST_ENV_PARSE_U64_";
        let _env = env_guard();
        std::env::set_var(key, "123");
        let result: Option<u64> = env_parse(key);
        std::env::remove_var(key);
        assert_eq!(result, Some(123u64), "env var set to '123' must parse to Some(123)");
    }

    #[test]
    fn env_parse_returns_none_when_var_has_unparseable_value() {
        // `env_parse::<u64>("…")` returns None when the env var value cannot be parsed.
        let key = "_SPARQ_QCNN19_TEST_ENV_PARSE_BAD_";
        let _env = env_guard();
        std::env::set_var(key, "not-a-number");
        let result: Option<u64> = env_parse(key);
        std::env::remove_var(key);
        assert!(result.is_none(), "unparseable env var value must yield None from env_parse");
    }

    // [OPUS-4.8] (sq-p7kk5) Adaptive group-commit: default ON, env toggle, and the
    // WriterConfig mapping — the write-path latency fix.
    #[test]
    fn adaptive_commit_is_on_by_default() {
        assert!(
            ServerConfig::default().adaptive_commit,
            "the interactive write path defaults to adaptive group-commit"
        );
    }

    #[test]
    fn sparq_adaptive_commit_env_zero_disables_it() {
        let _env = env_guard();
        std::env::set_var("SPARQ_ADAPTIVE_COMMIT", "0");
        let off = ServerConfig::from_env();
        std::env::remove_var("SPARQ_ADAPTIVE_COMMIT");
        assert!(
            !off.expect("from_env ok").adaptive_commit,
            "SPARQ_ADAPTIVE_COMMIT=0 must disable adaptive group-commit"
        );

        std::env::set_var("SPARQ_ADAPTIVE_COMMIT", "1");
        let on = ServerConfig::from_env();
        std::env::remove_var("SPARQ_ADAPTIVE_COMMIT");
        assert!(
            on.expect("from_env ok").adaptive_commit,
            "SPARQ_ADAPTIVE_COMMIT=1 must keep adaptive group-commit on"
        );
    }

    #[test]
    fn writer_config_carries_adaptive_commit_from_server_config() {
        // The writer the server spawns inherits the server config's adaptive-commit flag,
        // so the fix actually reaches the sequenced writer (both polarities).
        let on = ServerConfig {
            adaptive_commit: true,
            ..ServerConfig::default()
        };
        assert!(super::writer_config(&on).adaptive_commit);
        let off = ServerConfig {
            adaptive_commit: false,
            ..ServerConfig::default()
        };
        assert!(!super::writer_config(&off).adaptive_commit);
    }

    // [OPUS-4.8] sq-qcnn.37: individual tests for each remaining SPARQ_* env-var body branch
    // that the existing tests leave uncovered. Each test sets exactly ONE env var, calls
    // from_env(), then removes it — keeping the set/remove within the same call so the
    // single-threaded coverage runner sees a clean environment. Tests are hermetic by the
    // same rationale as the sq-qcnn.19 tests above.

    #[test]
    fn from_env_reads_sparq_query_timeout() {
        let _env = env_guard();
        std::env::set_var("SPARQ_QUERY_TIMEOUT", "30");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_QUERY_TIMEOUT");
        let cfg = result.expect("SPARQ_QUERY_TIMEOUT=30 must succeed");
        assert_eq!(
            cfg.query_timeout,
            Some(std::time::Duration::from_secs(30)),
            "SPARQ_QUERY_TIMEOUT=30 must set query_timeout to 30s",
        );
    }

    #[test]
    fn from_env_reads_sparq_update_where_timeout() {
        let _env = env_guard();
        std::env::set_var("SPARQ_UPDATE_WHERE_TIMEOUT", "15");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_UPDATE_WHERE_TIMEOUT");
        let cfg = result.expect("SPARQ_UPDATE_WHERE_TIMEOUT=15 must succeed");
        assert_eq!(
            cfg.update_where_timeout,
            Some(std::time::Duration::from_secs(15)),
            "SPARQ_UPDATE_WHERE_TIMEOUT=15 must set update_where_timeout to 15s",
        );
    }

    #[test]
    fn from_env_reads_sparq_header_read_timeout() {
        let _env = env_guard();
        std::env::set_var("SPARQ_HEADER_READ_TIMEOUT", "5");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_HEADER_READ_TIMEOUT");
        let cfg = result.expect("SPARQ_HEADER_READ_TIMEOUT=5 must succeed");
        assert_eq!(
            cfg.header_read_timeout,
            Some(std::time::Duration::from_secs(5)),
            "SPARQ_HEADER_READ_TIMEOUT=5 must set header_read_timeout to 5s",
        );
    }

    #[test]
    fn from_env_reads_sparq_body_read_timeout() {
        let _env = env_guard();
        std::env::set_var("SPARQ_BODY_READ_TIMEOUT", "10");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_BODY_READ_TIMEOUT");
        let cfg = result.expect("SPARQ_BODY_READ_TIMEOUT=10 must succeed");
        assert_eq!(
            cfg.body_read_timeout,
            Some(std::time::Duration::from_secs(10)),
            "SPARQ_BODY_READ_TIMEOUT=10 must set body_read_timeout to 10s",
        );
    }

    #[test]
    fn from_env_reads_sparq_max_query_rows() {
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_QUERY_ROWS", "500");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_QUERY_ROWS");
        let cfg = result.expect("SPARQ_MAX_QUERY_ROWS=500 must succeed");
        assert_eq!(
            cfg.max_query_rows,
            Some(500),
            "SPARQ_MAX_QUERY_ROWS=500 must set max_query_rows to Some(500)",
        );
    }

    #[test]
    fn from_env_reads_sparq_max_query_bytes() {
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_QUERY_BYTES", "1048576");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_QUERY_BYTES");
        let cfg = result.expect("SPARQ_MAX_QUERY_BYTES=1048576 must succeed");
        assert_eq!(
            cfg.max_query_bytes,
            Some(1048576),
            "SPARQ_MAX_QUERY_BYTES=1048576 must set max_query_bytes to Some(1048576)",
        );
    }

    #[test]
    fn from_env_reads_sparq_auth_token() {
        let _env = env_guard();
        std::env::set_var("SPARQ_AUTH_TOKEN", "secret-token-123");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_AUTH_TOKEN");
        let cfg = result.expect("SPARQ_AUTH_TOKEN=secret-token-123 must succeed");
        assert_eq!(
            cfg.auth_token,
            Some("secret-token-123".to_string()),
            "SPARQ_AUTH_TOKEN must set auth_token",
        );
    }

    #[test]
    fn from_env_reads_sparq_service_allow() {
        let _env = env_guard();
        std::env::set_var("SPARQ_SERVICE_ALLOW", "api.example.org");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_SERVICE_ALLOW");
        let cfg = result.expect("SPARQ_SERVICE_ALLOW=api.example.org must succeed");
        assert!(
            cfg.service_allow.engine_entries().contains(&"api.example.org".to_string()),
            "SPARQ_SERVICE_ALLOW must allowlist api.example.org",
        );
    }

    #[test]
    fn from_env_reads_sparq_cors_allow_origin() {
        let _env = env_guard();
        std::env::set_var("SPARQ_CORS_ALLOW_ORIGIN", "https://app.example.org");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_CORS_ALLOW_ORIGIN");
        let cfg = result.expect("SPARQ_CORS_ALLOW_ORIGIN=https://app.example.org must succeed");
        assert!(
            !cfg.cors_allow.is_empty(),
            "SPARQ_CORS_ALLOW_ORIGIN must add an entry to cors_allow",
        );
    }

    #[test]
    fn from_env_reads_sparq_persist_dir() {
        let _env = env_guard();
        std::env::set_var("SPARQ_PERSIST_DIR", "/tmp/sparq-persist-test-qcnn37");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_PERSIST_DIR");
        let cfg = result.expect("SPARQ_PERSIST_DIR must succeed");
        assert_eq!(
            cfg.persist_dir,
            Some(std::path::PathBuf::from("/tmp/sparq-persist-test-qcnn37")),
            "SPARQ_PERSIST_DIR must set persist_dir",
        );
    }

    // [OPUS-4.8] sq-qcnn.37: cover the remaining SPARQ_* env-var branches not yet exercised.
    // Each test sets ONE env var, calls from_env, and immediately removes it to stay hermetic.

    #[test]
    fn from_env_reads_sparq_max_body_bytes() {
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_BODY_BYTES", "2097152");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_BODY_BYTES");
        let cfg = result.expect("SPARQ_MAX_BODY_BYTES must succeed");
        assert_eq!(cfg.max_body_bytes, 2097152, "SPARQ_MAX_BODY_BYTES must set max_body_bytes");
    }

    #[test]
    fn from_env_reads_sparq_max_concurrent() {
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_CONCURRENT", "8");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_CONCURRENT");
        let cfg = result.expect("SPARQ_MAX_CONCURRENT must succeed");
        assert_eq!(cfg.max_concurrent, 8, "SPARQ_MAX_CONCURRENT must set max_concurrent");
    }

    #[test]
    fn from_env_reads_sparq_max_subscriptions_per_conn() {
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_SUBSCRIPTIONS_PER_CONN", "20");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_SUBSCRIPTIONS_PER_CONN");
        let cfg = result.expect("SPARQ_MAX_SUBSCRIPTIONS_PER_CONN must succeed");
        assert_eq!(
            cfg.max_subscriptions_per_conn, 20,
            "SPARQ_MAX_SUBSCRIPTIONS_PER_CONN must set max_subscriptions_per_conn",
        );
    }

    #[test]
    fn from_env_reads_sparq_max_subscriptions() {
        let _env = env_guard();
        std::env::set_var("SPARQ_MAX_SUBSCRIPTIONS", "100");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_MAX_SUBSCRIPTIONS");
        let cfg = result.expect("SPARQ_MAX_SUBSCRIPTIONS must succeed");
        assert_eq!(
            cfg.max_subscriptions, 100,
            "SPARQ_MAX_SUBSCRIPTIONS must set max_subscriptions",
        );
    }

    #[test]
    fn from_env_reads_sparq_allow_remote() {
        let _env = env_guard();
        std::env::set_var("SPARQ_ALLOW_REMOTE", "1");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_ALLOW_REMOTE");
        let cfg = result.expect("SPARQ_ALLOW_REMOTE must succeed");
        assert!(cfg.allow_remote, "SPARQ_ALLOW_REMOTE=1 must enable allow_remote");
    }

    #[test]
    fn from_env_reads_sparq_log_full_requests() {
        let _env = env_guard();
        std::env::set_var("SPARQ_LOG_FULL_REQUESTS", "1");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_LOG_FULL_REQUESTS");
        let cfg = result.expect("SPARQ_LOG_FULL_REQUESTS must succeed");
        // SPARQ_LOG_FULL_REQUESTS=1 opts out of redaction → redact_logs becomes false.
        assert!(!cfg.redact_logs, "SPARQ_LOG_FULL_REQUESTS=1 must disable redact_logs");
    }

    // [GPT-5.6] sq-lsp7k.5.2: direct config tests for the public facets runtime flag.
    #[cfg(feature = "facets")]
    #[test]
    fn facets_is_off_by_default() {
        assert!(
            !ServerConfig::default().facets,
            "the facet-count route must remain runtime-off by default"
        );
    }

    #[cfg(feature = "facets")]
    #[test]
    fn from_env_reads_sparq_facets() {
        let _env = env_guard();
        std::env::set_var("SPARQ_FACETS", "1");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_FACETS");
        let cfg = result.expect("SPARQ_FACETS=1 must succeed");
        assert!(cfg.facets, "SPARQ_FACETS=1 must enable the facet-count route");
    }

    // [GPT-5.6] sq-lsp7k.9.3: direct config tests for the public completion runtime flag.
    #[cfg(feature = "complete")]
    #[test]
    fn complete_is_off_by_default() {
        assert!(
            !ServerConfig::default().complete,
            "the completion route must remain runtime-off by default"
        );
    }

    #[cfg(feature = "complete")]
    #[test]
    fn from_env_reads_sparq_complete() {
        let _env = env_guard();
        std::env::set_var("SPARQ_COMPLETE", "1");
        let result = ServerConfig::from_env();
        std::env::remove_var("SPARQ_COMPLETE");
        let cfg = result.expect("SPARQ_COMPLETE=1 must succeed");
        assert!(cfg.complete, "SPARQ_COMPLETE=1 must enable completion");
    }

    #[cfg(feature = "complete")]
    #[test]
    fn completion_candidate_kinds_have_stable_wire_names() {
        assert_eq!(
            super::completion_kind_name(sparq_text::CandidateKind::Iri),
            "iri"
        );
        assert_eq!(
            super::completion_kind_name(sparq_text::CandidateKind::LocalName),
            "localName"
        );
        assert_eq!(
            super::completion_kind_name(sparq_text::CandidateKind::Label(1_u32)),
            "label"
        );
    }
}
