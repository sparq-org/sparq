#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

pub mod exec;
/// [OPUS-4.8] sq-rt6v: RDF *graph* serialisation (N-Triples / prefix-compacting Turtle /
/// RDF/XML) + RDF/XML body parsing for the CONSTRUCT/DESCRIBE + Graph Store read/write
/// surface. Pure (no async), like [`results`].
pub mod graph;
pub mod negotiate;

/// [OPUS-4.8] (sq-toze.36, cert gap GX-13) **In-binary container HEALTHCHECK probe** — the
/// `--health-probe` subcommand the Dockerfile's `HEALTHCHECK` runs. The runtime image is
/// distroless (no shell, no `curl`/`wget`), so the server probes its own loopback `/health`
/// endpoint and exits `0`/non-zero for the container runtime. The response classifier
/// ([`health_probe::probe_healthy`]) is pure + always compiled/unit-tested; the async TCP
/// runner (`health_probe::run_probe`) is gated on `server` (it uses tokio). See the module
/// docs for the distroless constraint that forces this in-binary approach.
pub mod health_probe;

pub mod results;
/// [OPUS-4.8] (sq-4w18) SERVICE federation egress allowlist — the
/// [`service_config::ServiceAllowlist`] config type (parse `--service-allow` /
/// `--service-allow-file` / `SPARQ_SERVICE_ALLOW`, exact-host + `*.suffix` matcher).
/// Pure (no async), always compiled + unit-tested; it only *carries* the policy. The
/// policy is enforced by the engine's `service` feature, which the server enables via
/// its own opt-in `service` feature. See the module docs for the default-DENY-all
/// posture and rationale.
pub mod service_config;

/// [OPUS-4.8] (sq-o7o0, ASVS V14.5.3) **First-party CORS origin allowlist** — the
/// [`cors_config::CorsAllowlist`] config type (parse `--cors-allow-origin` /
/// `--cors-allow-origin-file` / `SPARQ_CORS_ALLOW_ORIGIN`, exact-origin matcher). Pure
/// (no async), always compiled + unit-tested. EMPTY by default = NO CORS headers (the
/// historical, safe posture for a data API); an operator opts specific first-party
/// browser origins back in. Enforced by the `http::harden` middleware. See the module
/// docs for the deliberately-conservative policy (no `*`, no credentials).
pub mod cors_config;

/// [OPUS-4.8] (sq-toze.34, epic sq-toze) **Request-log redaction** — keeps SPARQL query / update
/// text out of the `--verbose` request log (a PII/privacy exposure: `GET /sparql?query=…` puts
/// the full query in the logged URI). On by default (`ServerConfig::redact_logs`); opt out with
/// `--log-full-requests` / `SPARQ_LOG_FULL_REQUESTS=1`. Log-CONTENT redaction, not anonymity — see
/// the module docs for the metadata boundary that remains. Always compiled + unit-tested.
pub mod redact;

#[cfg(feature = "server")]
pub mod http;

/// [OPUS-4.8] (sq-r5bv, gh-50) **Stable HTTP error/status contract** — the consumer-facing
/// transient-vs-permanent classification a client should encode (which status codes/messages
/// sparq emits for timeout / row-cap / malformed query / auth failure / etc., and which are
/// retryable). Documentation only (no code); asserted by `tests/status_contract.rs`. Gated on
/// `server` because it describes the [`http`] surface. See the module docs.
#[cfg(feature = "server")]
pub mod status_contract;

/// [OPUS-4.8] (sq-0bxp) OPT-IN per-query access audit log (CDMC CD-2 / ISO 27001 A.8.15 /
/// EU CRA logging). Compiled only behind the `audit-log` feature, and emitted only when
/// [`ServerConfig::audit_log`] (`--audit-log` / `SPARQ_AUDIT_LOG=1`) is also set. Emits a
/// structured `tracing` record per request under target `sparq_server::audit`; logs a
/// NON-reversible query fingerprint + token-identity fingerprint, never the full query text
/// or the Bearer secret (the #241 info-leak posture). See the module docs.
#[cfg(feature = "audit-log")]
pub mod audit;

/// [OPUS-4.8] (sq-gos8, epic sq-toze) OPT-IN RICHER STRUCTURED access-audit sink (ASVS V7 /
/// ISO 27001 A.8.15 / CDMC CD-2 logging). Compiled only behind the `access-audit` feature, and
/// emitted only when a sink is configured (`--access-audit <file|stderr>` /
/// `SPARQ_ACCESS_AUDIT`). Emits a TYPED access RECORD per enforced decision (actor / action /
/// resource / decision + policy-basis / timestamp / request fingerprint) as JSON-Lines through
/// the pluggable [`access_audit::AuditSink`] trait. PRIVACY BOUNDARY: identities + resource IRIs
/// are recorded by design (the audit trail); the query CONTENT only as a non-reversible
/// fingerprint, never the raw text. See the module docs.
#[cfg(feature = "access-audit")]
pub mod access_audit;

/// [OPUS-4.8] (sq-d3d8, epic sq-3183) OPT-IN federation discovery descriptors — the W3C
/// VoID dataset description (`GET /.well-known/void`) and the SPARQL 1.1 Service Description
/// (a `GET /sparql` with no `query`). Compiled only behind the `federation-descriptors`
/// feature, and served only when [`ServerConfig::federation_descriptors`] is also set. See
/// the module docs for the OPT-IN posture (feature + flag, both OFF by default).
#[cfg(feature = "federation-descriptors")]
pub mod descriptors;

/// [OPUS-4.8] (sq-bzh1, epic sq-3183) OPT-IN Triple Pattern Fragments (TPF) / Linked Data
/// Fragments (Hartig LDF) READ-ONLY source endpoint (`GET /tpf?subject=&predicate=&object=`) —
/// a paged RDF fragment of the triples matching one triple pattern, with Hydra controls
/// (`hydra:totalItems` from the cheap cardinality ESTIMATE, `hydra:next`/`hydra:previous`
/// paging, the `hydra:search` template). Pure (no async); the async handler is in
/// [`http`] (`tpf_endpoint`). Compiled only behind the `tpf` feature, and served only when
/// [`ServerConfig::tpf`] is also set (`--tpf` / `SPARQ_TPF=1`) — the same double-opt-in as
/// [`descriptors`]. See the module docs.
#[cfg(feature = "tpf")]
pub mod tpf;

/// [OPUS-4.8] (sq-2999l, gh-906) OPT-IN HTTP change-data-capture (CDC) poll endpoint in the
/// Amazon-Neptune-Streams `GetRecords` shape (`GET /streams`), over the durable change-stream
/// (sq-b4fns / #1223). PURE response builder (parameter resolution + per-quad flattening +
/// continuation maths; the async disk-poll handler is in [`http`]). Compiled ONLY behind the
/// `change-stream` feature, and served only when [`ServerConfig::change_stream_dir`] is also set
/// (`--change-stream <DIR>` / `SPARQ_CHANGE_STREAM`) — the same double-opt-in as `tpf`. The same
/// feature also records every commit to the durable log on the update path (see [`http`]). With
/// the feature off this module + the route + the recording hook are `#[cfg]`-stripped, byte-
/// identical to before.
#[cfg(feature = "change-stream")]
pub mod streams;

/// [OPUS-4.8] (sq-hj4n, gh-916) OPT-IN Solid-style **N3-Patch** parsing (`text/n3`) for the
/// Graph-Store-Protocol `PATCH` method — the `solid:InsertDeletePatch` dialect (`solid:where` /
/// `solid:deletes` / `solid:inserts` formulas), translated into ONE atomic graph-scoped SPARQL
/// Update. Pure (no async; the async dispatch is in [`http`]). Compiled ONLY behind the `n3-patch`
/// feature, and served only when [`ServerConfig::n3_patch`] is also set (`--n3-patch` /
/// `SPARQ_N3_PATCH=1`) — the same double-opt-in as [`tpf`]. The N3 parsing rides the `oxttl`
/// `N3Parser` (already a server dependency for the GSP read-side Turtle serialiser), so it pulls
/// NO new dependency. With the feature off this module and its dispatch arm are `#[cfg]`-stripped:
/// a `text/n3` `PATCH` body is then a plain `415`, byte-identical to before. The OTHER `PATCH`
/// dialect (`application/sparql-update` body) is always-on, in [`http`], and NOT behind this
/// feature.
#[cfg(feature = "n3-patch")]
pub mod n3_patch;

/// [FABLE-5] (sq-snopa.6, issue #992 FR-4) OPT-IN Solid **WAC/ACP HTTP authorization** surface —
/// the `POST /authz/decide`, `POST /authz/wac-allow` and `POST /authz/query` endpoints, a THIN
/// HTTP shell over the `sparq-solid` library authoriser. Compiled ONLY behind the `solid-authz`
/// feature (the deliberately-opt-in `sparq-server` -> `sparq-solid` workspace dependency the FR-4
/// note flagged), and served only when [`ServerConfig::solid_authz`](http::ServerConfig) is also
/// set (`--solid-authz` / `SPARQ_SOLID_AUTHZ=1`) — the same double-opt-in as `tpf` / `shacl` /
/// `terse`. FAIL-CLOSED: every error path DENIES. `sparq-solid` is a LIBRARY authoriser with no
/// HTTP surface (`research/sparq-solid-scope.md` §4); this is exactly that missing shell — it does
/// NOT authenticate (it takes an already-resolved session + the pod dataset per request). With the
/// feature off the module + the routes + the config field are `#[cfg]`-stripped, byte-identical to
/// before. See the module docs.
#[cfg(feature = "solid-authz")]
pub mod solid_authz;

/// [FABLE-5] (sq-lsp7k.10) OPT-IN named-parameterized-template REST surface (`/templates`):
/// server-stored, IRI-identified SPARQL query/UPDATE templates with typed JSON parameter
/// binding (GraphDB "SPARQL templates" / Stardog "stored queries" parity). Compiled ONLY
/// behind the `templates` feature (which enables `sparq-engine/templates` on top of the
/// #901 injection-safe `params` binding), and served only when
/// [`ServerConfig::templates`](http::ServerConfig) is also set (`--templates` /
/// `SPARQ_TEMPLATES=1`) — the same double-opt-in as `tpf` / `shacl` / `terse`. Template
/// writes and UPDATE-template invocations are write-gated through the SAME sequenced-writer
/// path as `/sparql` updates (the gated-update posture is preserved). With the feature off
/// the module + the routes + the config fields are `#[cfg]`-stripped, byte-identical to
/// before. See the module docs.
#[cfg(feature = "templates")]
pub mod templates;

/// Prometheus metrics — hand-rolled text exposition at `GET /metrics` (T22).
#[cfg(feature = "server")]
pub mod metrics;

/// SEPA-style SPARQL subscriptions over WebSocket (T23) — see `SUBSCRIPTIONS.md`.
#[cfg(feature = "server")]
pub mod subscriptions;

#[cfg(feature = "server")]
pub use http::{
    bind_posture, harden, router, serve, AppState, AuthPosture, BindPosture, PinnedGen,
    ServerConfig, GLOBAL_POD,
}; // [OPUS-4.8] sq-o4qf: bind_posture / BindPosture for the bind gate; sq-zcby: AuthPosture folds the --auth-token gate into it; sq-2gqr: serve = the accept loop with the slow-loris header-read deadline

/// [GPT-5.6] sq-oprna.6: TLS h1+h2 accept loop, opt-in with the `http2` feature.
#[cfg(feature = "http2")]
#[cfg_attr(docsrs, doc(cfg(feature = "http2")))]
pub use http::serve_tls;

/// [GPT-5.6] Loaded shapes wrapper for opt-in SHACL transaction guard mode.
#[cfg(feature = "shacl")]
pub use http::ShaclShapes;

/// [FABLE-5] (sq-fy8ci) The RAII single-flight restore permit, re-exported at the crate root
/// next to [`AppState`]. Obtained via `AppState::try_begin_restore`; while one is alive a
/// second concurrent restore is refused (the `/admin/restore` route maps that to 409 Conflict).
/// Present only with the `backup` cargo feature (which implies `server`).
#[cfg(feature = "backup")]
pub use http::RestoreGuard;

/// [OPUS-4.8] (sq-4w18) The SERVICE egress allowlist config type, re-exported at the
/// crate root next to `ServerConfig`.
pub use service_config::ServiceAllowlist;

/// [OPUS-4.8] (sq-9xoh) The per-request SERVICE egress allowlist override hook type, re-exported
/// at the crate root next to [`ServerConfig`]. Construct it with
/// [`ServiceAllowOverride::new`](http::ServiceAllowOverride::new) and install it on
/// [`ServerConfig::service_allow_override`] for a multi-tenant / gateway deployment that derives
/// the reachable SERVICE host set per request. Present only with the `service` cargo feature
/// (which implies `server`).
#[cfg(feature = "service")]
pub use http::ServiceAllowOverride;

/// [OPUS-4.8] (sq-o7o0, ASVS V14.5.3) The first-party CORS origin allowlist config type,
/// re-exported at the crate root next to `ServerConfig`.
pub use cors_config::CorsAllowlist;

/// [OPUS-4.8] (sq-uqh, Wave B) Re-exported for consumers (and tests) that introspect a
/// pinned generation's per-pod epoch vector — the cache-invalidation hook the server's
/// updates feed via [`http::AppState::apply_update`]. A `PinnedGen` exposes
/// `.epochs().epoch(&PodId)` (sparq-serve's `PodEpochs`); a write to one named graph bumps
/// only that graph's `PodId` epoch, so a read scoped to another graph is not invalidated.
#[cfg(feature = "server")]
pub use sparq_serve::{Epoch, PodEpochs, PodId};
