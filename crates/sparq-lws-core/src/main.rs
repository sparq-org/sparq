// AUTHORED-BY Claude Opus 4.8
//! EXPERIMENTAL solid-server-rs binary entry point.
//!
//! Boots the vertical slice: an axum server with **real** DPoP-bound Solid-OIDC auth (delegated to
//! `solid-oidc-verifier`'s network adapters) over the LDP verb path backed by a [`CompositeStore`]
//! (SPARQ-authoritative metadata + object_store-backup bytes).
//!
//! ## Auth (REAL, network-backed)
//! The binary wires the verifier's M2 network adapters so token verification is genuine:
//! - [`NetworkJwksProvider`] performs OIDC discovery (`<issuer>/.well-known/openid-configuration`) →
//!   `jwks_uri` → JWKS fetch + parse, cached, **every** fetch through the DNS-pinned SSRF-guarded
//!   `SafeFetcher` (a `jwks_uri` at a private host fails closed).
//! - [`NetworkWebIdResolver`] (when the bidirectional WebID↔issuer check is enabled) fetches the
//!   WebID profile over the same SSRF-guarded path and extracts `solid:oidcIssuer`.
//! - The verification CORE (RFC 9068 `at+jwt`, RFC 9449 DPoP, RFC 7638 thumbprint, asymmetric-only,
//!   issuer-agnostic trusted-issuer allowlist, jti replay) is the verifier crate's — never reimplemented.
//!
//! Configuration is environment-driven (see the `*_ENV` constants below). Sensible production
//! defaults: HTTPS-only IdP/WebID (loopback refused), DPoP required, strict bidirectional check.
//!
//! ## TLS termination (config-gated)
//! When `SOLID_SERVER_TLS_CERT` + `SOLID_SERVER_TLS_KEY` (PEM file paths) are BOTH set, the binary
//! terminates HTTPS in-process via `axum-server` over the house rustls/aws-lc-rs stack; when NEITHER
//! is set it keeps the plain-TCP listener (terminate TLS at a reverse proxy). See [`sparq_lws_core::tls`]
//! for the config-shape decision (PEM paths, both-or-neither validation, ACME noted as a future seam).
//! Both serve paths share the same `SOLID_SERVER_BIND` resolution (hostname:port accepted, not only a
//! numeric `SocketAddr`) and the same Ctrl-C graceful-drain behaviour.
//! With the default-off `http3` feature and TLS configured, the binary additionally binds an HTTP/3
//! QUIC listener on the same resolved address and port over UDP. The existing TCP ALPN contract stays
//! exactly `[h2, http/1.1]`; only the cloned QUIC configuration advertises `h3`.
//!
//! ## Still seamed (not in this slice)
//! - The live SPARQ HTTP client + the `object_store`-backed blob store (the binary still boots the
//!   in-memory store doubles so it runs without SPARQ / S3; swapping in `HttpSparqClient` is wiring).
//! - WAC authorization (gated on sparq#992 — the LDP layer is fail-closed: mutations need an
//!   authenticated caller, reads are public since no ACLs exist yet).
//!
//! ## Global allocator (mimalloc)
//! The process installs Microsoft's mimalloc as the `#[global_allocator]` (drop-in, musl-friendly).
//! This is a behaviour-NEUTRAL perf lever — it only changes which allocator backs `alloc`/`dealloc`,
//! not any server logic. See the `Cargo.toml` dependency comment for the trust-surface delta (a
//! vendored-C `*-sys` crate compiled at build time) and why mimalloc over jemalloc (musl page-size).

#[cfg(target_arch = "wasm32")]
fn main() {
    use sparq_lws_core::ldp::handler::LdpState;
    use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
    use sparq_lws_core::{build_router, AppState};

    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    let state = AppState::new(LdpState::new(store, "http://localhost"));
    let _router = build_router(state);
}

#[cfg(not(target_arch = "wasm32"))]
mod reconcile_runtime;

#[cfg(not(target_arch = "wasm32"))]
macro_rules! native_main {
    ($($item:item)*) => {
        $($item)*
    };
}

#[cfg(not(target_arch = "wasm32"))]
#[rustfmt::skip]
native_main! {

/// Process-wide allocator. mimalloc replaces the default libc allocator on the hot alloc/dealloc
/// path. Declared at the TOP of the binary so it is the allocator from the first allocation onward.
/// Behaviour-neutral: no server logic depends on it; conformance + tests are unchanged.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;
use std::time::Duration;

use solid_oidc_verifier::config::{JwksProvider, NetworkJwksProvider, VerifierConfig};
use solid_oidc_verifier::replay::{InMemoryReplayStore, ReplayStore};
use solid_oidc_verifier::verifier::Verifier;
use solid_oidc_verifier::webid::{BidirectionalMode, NetworkWebIdResolver};
use sparq_lws_core::acl_cache::{AclCache, DEFAULT_ACL_CACHE_CAPACITY};
use sparq_lws_core::app::{build_router_with_overload, AppState, OverloadConfig};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::auth_cache::{
    ProofPolicy, SharedReplay, VerifiedTokenCache, DEFAULT_CACHE_CAPACITY,
};
use sparq_lws_core::body_limit;
use sparq_lws_core::identity::IdentityConfig;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::overload::{self, AdmissionControl};
use sparq_lws_core::rate_limit::{self, RateConfig, RateLimiter};
#[cfg(feature = "http-sparq")]
use sparq_lws_core::store::HttpSparqClient;
use sparq_lws_core::store::{
    BodyCache, CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store,
    DEFAULT_BODY_CACHE_BYTES,
};
use sparq_lws_core::tls::{self, TlsMode};
use sparq_lws_core::transport::{ConnectionLimiter, TransportConfig};

use crate::reconcile_runtime::{
    reconcile_interval_from_env, spawn_periodic_if_configured, SharedStore,
};

// --- Environment configuration keys ----------------------------------------------------------------
const ENV_BASE_URL: &str = "SOLID_SERVER_BASE_URL";
const ENV_BIND: &str = "SOLID_SERVER_BIND";
const ENV_TRUSTED_ISSUER: &str = "SOLID_SERVER_TRUSTED_ISSUER";
/// The RS's identity required in a token's `aud` (RFC 9068). Defaults to the base URL.
const ENV_AUDIENCE: &str = "SOLID_SERVER_AUDIENCE";
/// Dev/IT ONLY: permit an `http:`/loopback IdP + WebID host (the SSRF gate normally refuses them).
/// Anything other than `1`/`true` keeps the production posture (HTTPS-only, no loopback).
const ENV_ALLOW_LOOPBACK: &str = "SOLID_SERVER_ALLOW_LOOPBACK";
/// JWKS cache TTL in seconds (how long a discovered keyset is reused before re-fetching). Default 300.
const ENV_JWKS_CACHE_TTL_SECS: &str = "SOLID_SERVER_JWKS_CACHE_TTL_SECS";
/// Bidirectional WebID↔issuer check mode: `strict` (default) / `warn` / `off`.
const ENV_BIDIRECTIONAL: &str = "SOLID_SERVER_BIDIRECTIONAL";
/// Dev/BENCHMARK ONLY: RAISE the in-memory DPoP-replay-store capacity (live `jti`s) above the
/// production default. UNSET — or any value `<=` the default — keeps the production default (100_000,
/// `InMemoryReplayStore::with_window`) UNCHANGED, so conformance and the production posture are
/// byte-identical without this var, and a mis-set value can only ever RAISE the cap, never shrink it
/// (a too-small cap would fail authenticated traffic closed sooner). It exists ONLY so the
/// authenticated load benchmark can measure the steady-state token/DPoP-verify cost without the replay
/// store reaching its fail-closed capacity mid-sweep (a sustained high-RPS authed run fills 100k
/// unique jtis within the proof-age TTL window and then — correctly — rejects further proofs). This
/// changes NO request-handling logic, only a capacity number; the fail-closed semantics are unchanged.
/// NEVER raise it in production to dodge the cap — the cap is a real single-instance safety bound (the
/// shared-Redis ReplayStore is the horizontal-scaling seam). See bench/AUTH-BASELINE.md.
const ENV_REPLAY_MAX_ENTRIES: &str = "SOLID_SERVER_REPLAY_MAX_ENTRIES";
/// Select the DISTRIBUTED Redis-backed DPoP-`jti` replay store (the horizontal-scaling backend): a
/// Redis connection URL (e.g. `redis://redis:6379`). Requires the `redis-replay` build feature; UNSET
/// (or feature-off) keeps the per-instance in-memory store (the default — single-instance posture). A
/// connect failure at boot ABORTS startup (fail-closed): the server never runs with a silently-broken
/// shared replay set. ALL instances behind a load balancer MUST point at the same Redis, or replay
/// protection is per-instance again. See [`sparq_lws_core::redis_replay`].
#[cfg(feature = "redis-replay")]
const ENV_REPLAY_REDIS_URL: &str = "SOLID_SERVER_REPLAY_REDIS_URL";
/// Dev/conformance ONLY: when `1`/`true`, seed the in-memory store with the conformance test users'
/// WebID profiles + container tree (the Solid CTH bootstraps by dereferencing those WebIDs). NEVER
/// set against a real (SPARQ/S3) backend. See [`sparq_lws_core::seed`].
const ENV_SEED_CONFORMANCE: &str = "SOLID_SERVER_SEED_CONFORMANCE";
/// Dev/BENCHMARK ONLY: when set, seed the in-memory store with the HTTPS-load-benchmark fixtures (a
/// public doc, a public listing container with N children, a private doc — see
/// [`sparq_lws_core::seed::seed_bench`]). The value is the child count (an integer); a bare `1`/
/// `true` (or any non-integer) uses [`seed::BENCH_DEFAULT_CHILDREN`]. NEVER set against a real
/// (SPARQ/S3) backend. Purely additive seeding — it changes no request-handling behaviour.
const ENV_SEED_BENCH: &str = "SOLID_SERVER_SEED_BENCH";
/// Dev/BENCHMARK ONLY (companion to [`ENV_SEED_BENCH`]): override the bench pod OWNER WebID named by
/// the seeded owner-only ACL grants. The syscall harness (`bench/syscalls.sh`) serves plain HTTP but
/// the verifier requires an `https:` `webid` claim — the derived `http:` owner IRI could never match
/// a valid token, so the harness sets this to a synthetic `https:` WebID and mints its DPoP tokens
/// for the same value (never dereferenced: the harness runs `SOLID_SERVER_BIDIRECTIONAL=off`). Unset
/// ⇒ the derived `<base>/bench/profile/card#me`, byte-identical to before. Only read when
/// [`ENV_SEED_BENCH`] is active; like the rest of the seed it only changes seeded FIXTURE content,
/// never request handling.
const ENV_SEED_BENCH_OWNER: &str = "SOLID_SERVER_SEED_BENCH_OWNER";
/// DEMO ONLY (the §3.2 public-demo posture of `research/lws-demo-architecture.md`): when `1`/`true`,
/// seed the in-memory store with the demo playground fixtures — a shared root-level `/playground/`
/// container any AUTHENTICATED agent can Read/Write/Append (anonymous visitors read-only; nobody has
/// `acl:Control`, so the ACL cannot be rewritten over HTTP), plus a public-read `/README` Turtle
/// document carrying the ephemeral-demo banner. NEVER set against a real (SPARQ/S3) backend — the
/// startup seed-guard fails closed like the other seed flags. Purely additive seeding — it changes
/// no request-handling behaviour. See [`sparq_lws_core::seed::seed_demo`].
const ENV_SEED_DEMO: &str = "SOLID_SERVER_SEED_DEMO";
/// Dev/conformance ESCAPE HATCH: explicitly permit the dev seed flags
/// ([`ENV_SEED_CONFORMANCE`] / [`ENV_SEED_BENCH`] / [`ENV_SEED_DEMO`]) against a NON-`memory`
/// backend. UNSET (the default)
/// makes the startup seed-guard FAIL CLOSED when a seed flag is set on a `http`/`embedded` backend —
/// so test fixtures can never be written into a live/durable store by accident. Set to `1`/`true`
/// ONLY for an EPHEMERAL embedded test instance that the harness legitimately seeds (the
/// conformance run.sh embedded leg sets it). NEVER set it against a real/persistent backend.
const ENV_ALLOW_SEED_NONMEMORY: &str = "SOLID_SERVER_ALLOW_SEED_NONMEMORY";
/// Provider WebIDs OUTSIDE the pod (`research/lws-design-records.md` §4 — the RSS adaptation of
/// prod-solid-server's PSS `decisions/0020`): when `1`/`true`, the identity gate SERVES id-docs
/// (GET/HEAD-only, Turtle/JSON-LD, NO WAC, no `.acl` Link) on the identity host, and the
/// conformance seed mints id-host WebIDs (`https://<identity-host>/<handle>#me` with the LOCKED
/// `solid:oidcIssuer` + `pim:storage`) instead of in-pod cards. Default OFF — byte-for-byte the
/// prior behaviour EXCEPT the unconditional LDP refusal of the reserved `/.identity/**` namespace,
/// which holds regardless of this flag (pre-seeded documents must never become LDP-addressable —
/// and thus `.acl`-able — when the flag later turns on).
const ENV_IDENTITY_ENABLE: &str = "SOLID_SERVER_IDENTITY_ENABLE";
/// The identity-host authority (`host[:port]`) the gate exact-Host-matches (the analogue of
/// `PSS_IDENTITY_HOST`). Unset ⇒ derived as `id.<base authority>` (deployment-agnostic). Only read
/// when [`ENV_IDENTITY_ENABLE`] is on; must differ from the base authority (fail-closed at boot).
const ENV_IDENTITY_HOST: &str = "SOLID_SERVER_IDENTITY_HOST";
/// Round-3 verified-access-token cache capacity (distinct live access tokens). Unset =>
/// [`DEFAULT_CACHE_CAPACITY`]. The cache removes the redundant per-request access-token *signature +
/// claims* re-verify (the token is stable across a client's requests) while STILL fully verifying the
/// fresh DPoP proof + `jti` replay + `cnf.jkt` binding on every request -- it can never turn a
/// would-be-401/403 into a 200. Set to `0` to DISABLE the cache (every authenticated request runs the
/// full verifier -- the pre-round-3 path). Conformance-neutral. See [`sparq_lws_core::auth_cache`].
const ENV_TOKEN_CACHE_CAPACITY: &str = "SOLID_SERVER_TOKEN_CACHE_CAPACITY";
/// ETag-keyed parsed-ACL cache capacity (distinct cached `.acl` resources). Unset =>
/// [`DEFAULT_ACL_CACHE_CAPACITY`]. The cache reuses the PARSED triples of an UNCHANGED ACL across reads
/// (keyed by `(acl-iri, etag)`), skipping the byte-fetch + `oxttl` re-parse on a hot resource — without
/// ever changing a decision (it is never authoritative: a rotated/removed ACL forces a re-read via the
/// etag/`meta` gate). Set to `0` to DISABLE the cache (every read re-reads + re-parses each ACL — the
/// pre-cache path). Conformance-neutral. See [`sparq_lws_core::acl_cache`].
const ENV_ACL_CACHE_CAPACITY: &str = "SOLID_SERVER_ACL_CACHE_CAPACITY";
/// Blob-body cache byte budget (read-4 — `research/lws-design-records.md` §7). Unset =>
/// [`DEFAULT_BODY_CACHE_BYTES`]. The cache holds resource BODIES keyed by `(blob_key, etag)` in
/// front of the blob store, so a hot UNCHANGED resource's repeat read pays ZERO blob round-trips —
/// without ever serving stale bytes (every lookup is keyed by the request's own authoritative index
/// metadata; blob keys are unique per write, so a rewrite is a guaranteed miss) and without
/// touching authorization (WAC runs before any body fetch, hit or miss). Set to `0` to DISABLE
/// (every read pays the blob fetch — the pre-cache path). Conformance-neutral. See
/// [`sparq_lws_core::store::body_cache`].
const ENV_BODY_CACHE_BYTES: &str = "SOLID_SERVER_BODY_CACHE_BYTES";
/// Per-entry size cap for the blob-body cache: a body larger than this BYPASSES the cache (served,
/// never stored), so one large media blob cannot evict the whole hot set. Unset => 1/8 of the byte
/// budget; clamped into `1..=budget`.
const ENV_BODY_CACHE_MAX_ENTRY_BYTES: &str = "SOLID_SERVER_BODY_CACHE_MAX_ENTRY_BYTES";
/// Select the SPARQ data-path backend (the authoritative-RDF [`SparqClient`] impl):
/// - `memory` (DEFAULT) — the in-memory [`InMemorySparqClient`] double: boots without SPARQ/S3 and
///   is what conformance + the unit/integration suites run against. UNCHANGED default.
/// - `http` — the live `HttpSparqClient` (a code span — behind the opt-in `http-sparq` build
///   feature) over a SPARQ `/sparql` endpoint (the shared-service deployment). Requires
///   `SOLID_SERVER_SPARQ_ENDPOINT`.
/// - `embedded` — the IN-PROCESS [`sparq_lws_core::store::embedded::EmbeddedSparqClient`]: the SPARQ
///   query engine consumed as a LIBRARY (no HTTP hop). The `embedded-sparq` build feature that
///   compiles it is ON BY DEFAULT (sq-gg0qq.3 — the first-class in-workspace backend); runtime
///   selection stays EXPLICIT via this variable (fail-safe: an unconfigured boot serves the
///   ephemeral in-memory double, never a store an operator did not choose). With
///   `SOLID_SERVER_SPARQ_DIR` set it opens a directory-backed graph; otherwise a fresh in-memory
///   graph. See `research/lws-design-records.md` §3.
///
/// `CompositeStore<S>` / `AppState<J,R,S>` / the router are all generic over the SparqClient `S`, so
/// each arm monomorphizes the SAME wiring — no consumer code changes between backends.
const ENV_SPARQ_BACKEND: &str = "PSS_SPARQ_BACKEND";
/// The SPARQ `/sparql` endpoint URL for `PSS_SPARQ_BACKEND=http` (opt-in `http-sparq` feature).
#[cfg(feature = "http-sparq")]
const ENV_SPARQ_ENDPOINT: &str = "SOLID_SERVER_SPARQ_ENDPOINT";
/// Optional on-disk directory for `PSS_SPARQ_BACKEND=embedded` (a previously-saved graph snapshot);
/// unset ⇒ a fresh in-memory graph.
#[cfg(feature = "embedded-sparq")]
const ENV_SPARQ_DIR: &str = "SOLID_SERVER_SPARQ_DIR";

/// Operational-safety: is the SELECTED SPARQ backend **durable / shared** (i.e. its index outlives a
/// process restart and/or is shared across instances)?
///
/// - `http` — a shared SPARQ service: DURABLE/SHARED. ✔
/// - `embedded` WITH a persistence dir (`SOLID_SERVER_SPARQ_DIR` set) — a directory-backed graph:
///   DURABLE. ✔
/// - `embedded` WITHOUT a persistence dir — a fresh in-memory graph: EPHEMERAL. ✘
/// - `memory` (the in-memory double) — EPHEMERAL. ✘
///
/// `backend` is the already-lowercased `PSS_SPARQ_BACKEND` value; `sparq_dir_set` is whether
/// `SOLID_SERVER_SPARQ_DIR` is set to a non-empty value.
fn sparq_backend_is_durable(backend: &str, sparq_dir_set: bool) -> bool {
    match backend {
        "http" => true,
        "embedded" => sparq_dir_set,
        // "memory" / unknown — unknown backends abort later in dispatch; treat as non-durable here.
        _ => false,
    }
}

/// Startup guard #1 — a DURABLE/SHARED SPARQ index paired with an EPHEMERAL (in-memory) blob store is
/// a byte/index INCONSISTENCY: a resource is indexed in SPARQ (survives restart / visible on another
/// instance) but its bytes live only in this process's in-memory blob store (lost on restart / absent
/// elsewhere). Reject that combination at boot.
///
/// Fires ONLY when the blob store is in-memory AND the SPARQ backend is durable/shared
/// ([`sparq_backend_is_durable`]). The EPHEMERAL combinations are CONSISTENT and allowed:
/// `memory` + in-mem-blob (both ephemeral — the conformance/test path) and `embedded`-without-dir +
/// in-mem-blob (both ephemeral). The blob store is currently ALWAYS in-memory (the S3 blob is an
/// unimplemented stub), so `blob_is_in_memory` is `true` today; the parameter keeps the predicate
/// honest for when a durable BlobStore lands (the guard then simply stops firing).
fn reject_durable_sparq_with_inmem_blob(
    backend: &str,
    sparq_dir_set: bool,
    blob_is_in_memory: bool,
) -> bool {
    blob_is_in_memory && sparq_backend_is_durable(backend, sparq_dir_set)
}

/// Startup guard #2 — refuse to run the dev SEED flags ([`ENV_SEED_CONFORMANCE`] /
/// [`ENV_SEED_BENCH`] / [`ENV_SEED_DEMO`]) against a NON-`memory` backend, so test fixtures are
/// never written into a live `http`/`embedded` store by accident.
///
/// Fires when a seed flag is set AND the backend is not `memory` AND the explicit
/// [`ENV_ALLOW_SEED_NONMEMORY`] override is NOT set. With the override set (an EPHEMERAL embedded test
/// instance the harness legitimately seeds — the conformance run.sh embedded leg), seeding proceeds.
/// Seeding the `memory` backend is ALWAYS allowed (it is the seed target by construction).
fn reject_seed_on_nonmemory(seed_requested: bool, backend: &str, allow_override: bool) -> bool {
    seed_requested && backend != "memory" && !allow_override
}

/// [GPT-5.6] Build the opt-in QUIC endpoint from a CLONE of the existing TCP rustls config.
///
/// The clone is load-bearing: the TCP endpoint must keep advertising exactly `h2` + `http/1.1`,
/// while QUIC advertises only the protocol it can actually serve (`h3`). Binding UDP to the TCP
/// listener's resolved address gives both transports the same host/port without a second config key.
#[cfg(feature = "http3")]
fn bind_http3_endpoint(
    tcp_tls: &axum_server::tls_rustls::RustlsConfig,
    udp_addr: std::net::SocketAddr,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let mut quic_tls = (*tcp_tls.get_inner()).clone();
    quic_tls.alpn_protocols = vec![b"h3".to_vec()];
    let quic = sparq_http3::quic_server_config(quic_tls)?;
    Ok(quinn::Endpoint::server(quic, udp_addr)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the rustls process-wide crypto provider (aws-lc-rs) is installed before any TLS use.
    // The SSRF-guarded fetcher uses rustls for HTTPS to the IdP/WebID; installing it here is honest.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let base_url =
        std::env::var(ENV_BASE_URL).unwrap_or_else(|_| "http://localhost:3000".to_string());
    let bind = std::env::var(ENV_BIND).unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let issuer = std::env::var(ENV_TRUSTED_ISSUER)
        .unwrap_or_else(|_| "https://idp.example/realms/solid".to_string());
    // Audience defaults to the base URL (the RS's identity). RFC 9068 makes `aud` mandatory.
    let audience = std::env::var(ENV_AUDIENCE).unwrap_or_else(|_| base_url.clone());

    // Dev/IT escape hatch: allow an http:/loopback IdP+WebID. Defaults OFF (production HTTPS-only).
    let allow_loopback = env_flag(ENV_ALLOW_LOOPBACK);
    let jwks_cache_ttl = Duration::from_secs(
        std::env::var(ENV_JWKS_CACHE_TTL_SECS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(300),
    );
    let bidirectional = parse_bidirectional(std::env::var(ENV_BIDIRECTIONAL).ok().as_deref());

    // --- Identity host (provider WebIDs OUTSIDE the pod — `research/lws-design-records.md` §4).
    // --- Default OFF. When on, the identity gate serves id-docs on the id host (GET/HEAD-only, no
    // WAC) and the conformance seed mints id-host WebIDs. The LDP surface's refusal of the reserved
    // `/.identity/**` namespace is UNCONDITIONAL either way (flag-independent — the security
    // property). A bad host config fails the boot (fail-closed), never a silent fallback.
    let identity = if env_flag(ENV_IDENTITY_ENABLE) {
        let host_override = std::env::var(ENV_IDENTITY_HOST).ok();
        let config = IdentityConfig::new(&base_url, host_override.as_deref())
            .map_err(|e| format!("identity-host configuration error: {e}"))?;
        eprintln!(
            "  IDENTITY: provider WebIDs served OUTSIDE the pod — {}/<handle>#me (GET/HEAD-only, \
             Turtle/JSON-LD, NO WAC, no .acl Link; exact-Host match on {:?}). The reserved \
             /.identity/** namespace is refused on the LDP surface (404, every method — also when \
             this flag is off).",
            config.origin(),
            config.host()
        );
        Some(config)
    } else {
        None
    };

    // --- TLS termination mode (config-gated; both-or-neither). ------------------------------------
    // Resolve EARLY so a misconfiguration (one TLS var without the other) fails fast at boot, before
    // we stand up auth/storage. Filesystem + PEM validation happens just before binding, below.
    let tls_mode = tls::mode_from_env().map_err(|e| format!("TLS configuration error: {e}"))?;
    // PoP Tier-1b: whether to enable the RFC 8705 mTLS-bound-token path (optional client-cert request
    // in the TLS handshake + the per-connection cert-binding dispatch in auth). Default OFF ⇒ the TLS +
    // plain serve paths are byte-identical to pre-Tier-1b. Only meaningful on the in-process TLS path.
    let mtls_bound_tokens = tls::mtls_bound_tokens_from_env();
    // PoP Tier 2 (DPoP-SK): the negotiated symmetric session fast path (`SOLID_SERVER_DPOP_SK`).
    // Default OFF ⇒ byte-identical to pre-Tier-2 (no routes, no metadata member, Signature headers
    // ignored). The `cb=tls-exporter` flavour additionally REQUIRES in-process TLS termination (the
    // acceptor exports the keying material); behind a TLS-terminating proxy only `cb=none` is
    // offered — the DPoP-SK spec forbids advertising tls-exporter there.
    let dpop_sk = sparq_lws_core::pop::sk::dpop_sk_from_env();
    let sk_exporter_active = dpop_sk && matches!(tls_mode, TlsMode::Tls { .. });
    let sk_state = dpop_sk.then(|| {
        Arc::new(sparq_lws_core::pop::sk::SkState::new(
            sparq_lws_core::pop::sk::SkConfig {
                exporter_available: sk_exporter_active,
                ..sparq_lws_core::pop::sk::SkConfig::default()
            },
        ))
    });

    // --- Auth: REAL network-backed verification (delegated to solid-oidc-verifier). ----------------
    // The NetworkJwksProvider does OIDC discovery + JWKS fetch over the DNS-pinned SSRF-guarded path.
    let jwks = NetworkJwksProvider::new(jwks_cache_ttl, allow_loopback)
        .map_err(|e| format!("failed to init the network JWKS provider: {e}"))?;

    // Build the verifier config. When the bidirectional check is enabled, wire the network WebID
    // resolver — the verifier REFUSES strict/warn without a resolver (a security policy must not be
    // silently disabled by misconfiguration), so this is required, not optional.
    let mut config = VerifierConfig::new(vec![issuer.clone()], audience.clone());
    config = match bidirectional {
        BidirectionalMode::Off => config,
        mode => {
            let resolver = NetworkWebIdResolver::new(allow_loopback)
                .map_err(|e| format!("failed to init the network WebID resolver: {}", e.0))?;
            config.bidirectional(mode, Arc::new(resolver))
        }
    };
    // Surface a misconfiguration (e.g. empty issuer/audience) up front rather than per-request.
    config
        .validate()
        .map_err(|e| format!("invalid verifier configuration: {e}"))?;

    // The in-memory jti replay store fails CLOSED at capacity (single-instance posture). A shared
    // (Redis SET NX) ReplayStore is the horizontal-scaling seam. The capacity is the production
    // default (REPLAY_PRODUCTION_DEFAULT) UNLESS the dev/bench-only ENV_REPLAY_MAX_ENTRIES override
    // RAISES it (a value <= the default is ignored, so it can only ever raise — see the const's doc).
    // The override exists solely so the auth benchmark can measure steady-state verify cost without the
    // cap's fail-closed path firing mid-sweep. Unset ⇒ byte-identical to before (conformance-neutral).
    let in_memory_replay = build_in_memory_replay(config.replay_ttl());

    // Backend selection. When the `redis-replay` feature is compiled in AND
    // `SOLID_SERVER_REPLAY_REDIS_URL` is set, build a DISTRIBUTED Redis-backed replay store shared by
    // every instance (so a jti consumed on instance A is seen by B — the horizontal-scaling fix); a
    // Redis connect failure aborts boot (fail-closed). Otherwise (the DEFAULT) wrap the per-instance
    // in-memory store. Either way `inner_replay` is ONE concrete `ReplayStore`, so the verifier + cache
    // + AppState wiring below is unchanged. Without the feature, `inner_replay` IS the in-memory store —
    // byte-identical to before, conformance-neutral.
    #[cfg(feature = "redis-replay")]
    let inner_replay = match std::env::var(ENV_REPLAY_REDIS_URL)
        .ok()
        .filter(|u| !u.trim().is_empty())
    {
        Some(url) => {
            let store = sparq_lws_core::redis_replay::RedisReplayStore::connect(url.trim())
                .map_err(|e| format!("failed to connect the Redis replay store: {}", e.0))?;
            eprintln!(
                "  AUTH: DISTRIBUTED Redis DPoP-jti replay store ENABLED (shared SET NX PX across \
                 instances — the horizontal-scaling backend). Fail-closed on any Redis error."
            );
            sparq_lws_core::redis_replay::BackendReplay::Redis(store)
        }
        None => sparq_lws_core::redis_replay::BackendReplay::InMemory(in_memory_replay),
    };
    // Without the `redis-replay` feature, the Redis backend is not compiled in. If an operator
    // nonetheless set SOLID_SERVER_REPLAY_REDIS_URL (expecting the shared store), ABORT at boot rather
    // than silently using the per-instance in-memory store — which, in a horizontally-scaled
    // deployment, would recreate the very replay-protection gap this store exists to close. Fail-closed
    // on the misconfiguration (roborev High): an explicit "rebuild with --features redis-replay" error,
    // never a silent downgrade.
    #[cfg(not(feature = "redis-replay"))]
    let inner_replay = {
        if std::env::var("SOLID_SERVER_REPLAY_REDIS_URL")
            .ok()
            .is_some_and(|u| !u.trim().is_empty())
        {
            return Err("SOLID_SERVER_REPLAY_REDIS_URL is set but this binary was built WITHOUT the \
                 `redis-replay` feature — refusing to start with the per-instance in-memory replay \
                 store, which would silently disable cross-instance replay protection when scaled \
                 horizontally. Rebuild with `cargo build --release --features redis-replay`, or unset \
                 the variable to run single-instance."
                .into());
        }
        in_memory_replay
    };

    // Round-3 verified-access-token cache. Capture the proof policy from the SAME config the verifier
    // is built with (so the cache's hit-path proof verification enforces byte-identical semantics),
    // BEFORE `config` is moved into the verifier. Wrap ONE replay store behind an `Arc` and give the
    // verifier a `SharedReplay` over it + the cache a clone of the same `Arc` -- so the hit path and
    // miss path mark the SAME jti set (the replay-bypass guard; see auth_cache).
    let proof_policy = ProofPolicy {
        clock_tolerance_secs: config.clock_tolerance_secs,
        allow_missing_ath: config.allow_missing_ath,
        replay_fail_closed: config.replay_fail_closed,
    };
    let cache_capacity = parse_cache_capacity(std::env::var(ENV_TOKEN_CACHE_CAPACITY).ok());

    let shared_replay = SharedReplay::new(Arc::new(inner_replay));
    // The cache marks jti through a CLONE of the SAME `SharedReplay` (it forwards to the one inner
    // `Arc<InMemoryReplayStore>`), so the verifier's miss path and the cache's hit path share one jti
    // set -- the replay-bypass guard. Cloning `SharedReplay` clones only the inner `Arc`.
    let cache_replay = Arc::new(shared_replay.clone());
    let verifier = Verifier::new(config, jwks, shared_replay)?;
    let auth = match cache_capacity {
        // 0 => cache DISABLED: the pre-round-3 full-verify-every-request path.
        0 => {
            eprintln!("  AUTH: verified-access-token cache DISABLED (full verify per request).");
            AuthContext::new(verifier, base_url.clone()).with_mtls_bound_tokens(mtls_bound_tokens)
        }
        cap => {
            eprintln!(
                "  AUTH: verified-access-token cache ENABLED (capacity {cap} tokens; DPoP proof + \
                 jti-replay + cnf.jkt re-verified every request)."
            );
            // Bound the cache's validation TTL by the CONFIGURED JWKS cache TTL (not the hard-coded
            // default): if an operator lowers SOLID_SERVER_JWKS_CACHE_TTL_SECS for faster key-
            // revocation response, the token cache honours the same window (roborev round-2 Medium) --
            // a revoked signing key forces a full re-verify within one JWKS-TTL, never up to `exp`.
            let max_entry_ttl = jwks_cache_ttl.as_secs() as i64;
            let cache = VerifiedTokenCache::with_max_entry_ttl(cap, proof_policy, max_entry_ttl);
            AuthContext::with_cache(verifier, base_url.clone(), cache, cache_replay)
                .with_mtls_bound_tokens(mtls_bound_tokens)
        }
    };
    // PoP Tier 2 (DPoP-SK): hand the session state to the auth layer (the attestation dispatch) —
    // `None` (flag off) keeps the middleware byte-identical. `crate::app` mounts the
    // `/.pop/session` routes + the RFC 9728 metadata off this same handle.
    let auth = auth.with_dpop_sk(sk_state);
    if dpop_sk {
        eprintln!(
            "  AUTH: DPoP-SK (PoP Tier 2) ENABLED — one DPoP proof at POST /.pop/session, then \
             per-request RFC 9421 hmac-sha256 attestation. Channel bindings offered: {}. DPoP \
             remains the mandatory, always-accepted baseline.",
            if sk_exporter_active {
                "tls-exporter + none (in-process TLS 1.3 exporter)"
            } else {
                "none only (no in-process TLS — tls-exporter not advertised)"
            }
        );
    }

    // --- Overload protection (admission control + request timeout). -------------------------------
    // Admission control sheds excess load (503 + jittered Retry-After) at a configurable concurrency
    // ceiling BEFORE auth/WAC/storage run (so a shed request never bypasses authorization). The
    // request timeout (504) reclaims a stuck request. Both are env-tunable; the defaults are HIGH
    // enough never to trip during normal use OR the conformance run. Health routes (/livez, /readyz)
    // are overload-EXEMPT (built outside the layers in `build_router_with_overload`).
    let max_concurrency = overload::max_concurrency_from_env();
    let request_timeout = overload::request_timeout_from_env();
    let admission = AdmissionControl::new(max_concurrency);
    eprintln!(
        "  OVERLOAD: admission control ENABLED (max in-flight {max_concurrency}; excess ⇒ 503 + \
         Retry-After). request timeout: {}. health probes /livez + /readyz are shed-EXEMPT.",
        match request_timeout {
            Some(d) => format!("{}s (504 on expiry)", d.as_secs()),
            None => "DISABLED".to_string(),
        }
    );
    // --- Pre-crypto per-IP rate limiter. ---------------------------------------------------------
    // A per-source token bucket OUTSIDE auth/WAC/crypto: a per-IP flood gets a cheap 429 and NEVER
    // reaches the DPoP verifier (the ~hundreds-of-µs ES256 verify is the #1 per-request cost), so
    // attacker traffic from one source cannot make every bogus proof pay the crypto. Default-ON with a
    // GENEROUS per-IP rate + loopback exemption so it never sheds a conformance-harness or normal-use
    // request; the `off` sentinel on SOLID_SERVER_RATE_LIMIT_PER_IP disables it entirely. XFF is NOT
    // trusted unless SOLID_SERVER_TRUSTED_PROXY is set (an untrusted XFF is a spoofable bypass).
    let rate_limiter = match rate_limit::rate_per_ip_from_env() {
        RateConfig::Enabled(rate) => {
            let burst = rate_limit::burst_from_env();
            let trusted_hops = rate_limit::trusted_proxy_hops_from_env();
            let exempt_loopback = rate_limit::exempt_loopback_from_env();
            let exempt_internal = rate_limit::exempt_internal_from_env();
            eprintln!(
                "  RATE-LIMIT: per-IP token bucket ENABLED (rate {rate}/s, burst {burst}; excess ⇒ \
                 429 + Retry-After BEFORE auth/crypto). XFF trusted hops: {trusted_hops} ({}). \
                 exempt-internal: {exempt_internal} (loopback+private+link-local+ULA — the default; \
                 protects against PUBLIC-internet per-source floods, set SOLID_SERVER_TRUSTED_PROXY to \
                 rate-limit clients behind a trusted proxy by their real public IP). \
                 loopback-exempt: {exempt_loopback}. health probes /livez + /readyz are EXEMPT.",
                if trusted_hops == 0 {
                    "XFF untrusted — direct peer IP"
                } else {
                    "client IP taken from X-Forwarded-For"
                }
            );
            Some(RateLimiter::new(
                rate,
                burst,
                trusted_hops,
                exempt_loopback,
                exempt_internal,
            ))
        }
        RateConfig::Disabled => {
            eprintln!(
                "  RATE-LIMIT: per-IP rate limiter DISABLED (SOLID_SERVER_RATE_LIMIT_PER_IP=off) — \
                 every request proceeds to auth."
            );
            None
        }
    };

    // --- Explicit request-body size limit. -------------------------------------------------------
    // An audited, configurable ceiling on per-request body buffering (a body over it ⇒ 413), replacing
    // reliance on axum's implicit 2 MiB default. Always finite (no unlimited mode). The resident-memory
    // ceiling for body buffering is `max_concurrency × body_limit`.
    let body_limit_bytes = body_limit::max_body_bytes_from_env();
    eprintln!(
        "  BODY-LIMIT: max request body {body_limit_bytes} bytes (over ⇒ 413). Aggregate body-buffer \
         ceiling ≈ max_concurrency ({max_concurrency}) × {body_limit_bytes} bytes."
    );

    let overload_config = OverloadConfig {
        admission,
        request_timeout,
        rate_limiter,
        body_limit_bytes,
    };

    // --- Transport-layer DoS hardening (HTTP/2 caps + slowloris timeout + connection cap). --------
    // These configure the hyper connection BELOW the application layers (admission/rate-limit see only
    // a parsed request; a rapid-reset or slow-header-trickle never produces one). They are applied ONLY
    // on the in-process TLS serve path (via axum-server's http_builder + the connection-cap acceptor);
    // the plain-HTTP path cannot configure them (axum::serve exposes neither). All defaults are
    // deliberately lenient so they never trip the conformance harness. See `sparq_lws_core::transport`.
    let transport_config = TransportConfig::from_env();
    // Global connection cap + the per-SOURCE (per-IP) cap: the global cap alone lets one source hold
    // all slots, so pair it with a per-IP cap (default-on, internal-IP-exempt) that bounds any single
    // source well below the global ceiling.
    let connection_limiter = ConnectionLimiter::new(transport_config.max_connections)
        .with_per_ip_cap(
            transport_config.max_connections_per_ip,
            transport_config.conn_exempt_internal,
        );
    // Log honestly per serve mode (roborev Low): the transport caps are ACTIVE only when terminating
    // TLS in-process. In plain-HTTP mode they are NOT enforced — say so, so an operator behind a
    // reverse proxy does not believe the in-process caps are on.
    if matches!(tls_mode, TlsMode::Tls { .. }) {
        eprintln!(
            "  TRANSPORT (TLS path, ACTIVE): HTTP/2 max_concurrent_streams={}, rapid-reset cap \
             (CVE-2023-44487)={} (hyper default 20 unless overridden), slowloris header-read \
             timeout={}, max concurrent connections={} (per-source cap={}, exempt-internal={}), \
             handshake timeout={}, h2 keep-alive ping={} (reclaims a DEAD-peer connection, not a \
             live-idle one), idle-keepalive timeout={} (reclaims a connection idle BETWEEN requests \
             — request-aware: the idle clock is paused while an HTTP exchange is in flight, so a \
             slow/streaming request is never severed; DEFAULT-OFF as it cannot see a post-UPGRADE \
             WebSocket stream), max requests/connection={} (Connection: close after N — HTTP/1.1 \
             reuse cap; never clobbers a 101 Upgrade).",
            transport_config.h2_max_concurrent_streams,
            match transport_config.h2_max_pending_reset_streams {
                Some(n) => format!("{n} (override)"),
                None => "hyper default".to_string(),
            },
            fmt_opt_secs(transport_config.header_read_timeout),
            connection_limiter.max_connections(),
            match connection_limiter.max_per_ip() {
                Some(n) => format!("{n}/IP"),
                None => "DISABLED".to_string(),
            },
            transport_config.conn_exempt_internal,
            fmt_opt_secs(transport_config.handshake_timeout),
            fmt_opt_secs(transport_config.keep_alive_timeout),
            fmt_opt_secs(transport_config.idle_timeout),
            match transport_config.max_requests_per_conn {
                Some(n) => n.to_string(),
                None => "unlimited".to_string(),
            },
        );
    } else {
        eprintln!(
            "  TRANSPORT (plain-HTTP path): the in-process HTTP/2 stream/reset caps, slowloris \
             header-read timeout, connection cap, and handshake timeout are NOT applied — \
             axum::serve does not expose the hyper builder. Terminate TLS in-process (set \
             SOLID_SERVER_TLS_CERT + _KEY) to enable them, or rely on a fronting reverse proxy to \
             cap connections + terminate HTTP/2. The application-layer defences (admission control, \
             per-IP rate limit, request timeout) DO apply on this path."
        );
    }
    // P1.5 (beyond-50k): TCP_NODELAY is set on accepted sockets on BOTH serve paths — the TLS path
    // via axum-server's NoDelayAcceptor composed into the RustlsAcceptor, the plain path via the
    // ListenerExt nodelay tap. Nagle off ⇒ small responses leave promptly. See `src/nodelay.rs`.
    eprintln!(
        "  TRANSPORT: TCP_NODELAY = ON (Nagle disabled) on accepted sockets (both serve paths)."
    );

    // --- SPARQ data-path backend selection.
    // ------------------------------------------------------- `CompositeStore<S>` /
    // `AppState<J,R,S>` / the router are generic over the SparqClient `S`, so each arm
    // monomorphizes the SAME downstream wiring (`build_app_for_store`): seed → LdpState → ACL cache
    // → AppState → router. Exactly ONE arm runs, so `auth` + `overload_config` (consumed once) move
    // into the chosen arm. RUNTIME DEFAULT = `memory` (the in-memory double — boot-without-config
    // is fail-safe/ephemeral + conformance byte-identical; sq-gg0qq.3 kept this EXPLICIT-selection
    // posture when the embedded feature went default-on). `embedded` wires the in-process engine
    // (`embedded-sparq` feature, default-on — the first-class backend); `http` wires the live
    // HttpSparqClient (opt-in `http-sparq` feature — the remote shared-service shape). See
    // `research/lws-design-records.md` §3.
    let backend = std::env::var(ENV_SPARQ_BACKEND)
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "memory".to_string());

    // --- Startup operational-safety guards (fail-closed at boot, before any backend is built). -----
    // The blob store is currently ALWAYS the in-memory double (the S3 blob is an unimplemented stub),
    // so `blob_is_in_memory` is `true`. Read it from one place so the guard is honest when a durable
    // BlobStore eventually lands (the guard then simply stops firing for the durable-blob case).
    let blob_is_in_memory = true;
    // Whether SOLID_SERVER_SPARQ_DIR is set to a non-empty value — read by its literal name so the
    // guard works regardless of the `embedded-sparq` feature gate (an `embedded` backend without the
    // feature aborts in dispatch anyway).
    let sparq_dir_set = std::env::var("SOLID_SERVER_SPARQ_DIR")
        .ok()
        .is_some_and(|d| !d.trim().is_empty());

    // GUARD #1 — a DURABLE/SHARED SPARQ index requires a DURABLE blob store. A persistent/shared index
    // (http, or dir-backed embedded) over the EPHEMERAL in-memory blob store is a byte/index
    // inconsistency: a resource is indexed in SPARQ but its bytes are lost on restart / absent on
    // another instance. `memory` and `embedded`-without-dir are ephemeral on BOTH sides (consistent) —
    // those still boot. Fail closed otherwise.
    if reject_durable_sparq_with_inmem_blob(&backend, sparq_dir_set, blob_is_in_memory) {
        return Err(format!(
            "a durable/shared SPARQ index (PSS_SPARQ_BACKEND={backend}{}) requires a durable blob \
             store; the in-memory blob store is ephemeral, so a resource indexed in SPARQ would lose \
             its bytes on restart / be absent on another instance. Configure a durable BlobStore, or \
             use PSS_SPARQ_BACKEND=memory (or PSS_SPARQ_BACKEND=embedded WITHOUT SOLID_SERVER_SPARQ_DIR \
             for an ephemeral in-memory graph).",
            if sparq_dir_set {
                " with SOLID_SERVER_SPARQ_DIR"
            } else {
                ""
            }
        )
        .into());
    }

    // GUARD #2 — refuse to seed test fixtures into a NON-`memory` backend. The dev seed flags
    // (SOLID_SERVER_SEED_CONFORMANCE / SOLID_SERVER_SEED_BENCH / SOLID_SERVER_SEED_DEMO) now run
    // against WHATEVER backend is selected, so without this guard they could write dev fixtures into a live
    // http/embedded store. Set SOLID_SERVER_ALLOW_SEED_NONMEMORY=1 to permit it ONLY for an ephemeral
    // embedded test instance (the conformance run.sh embedded leg). Seeding `memory` is always allowed.
    let seed_requested = seed_requested_from_env();
    if reject_seed_on_nonmemory(seed_requested, &backend, env_flag(ENV_ALLOW_SEED_NONMEMORY)) {
        return Err(format!(
            "refusing to seed test fixtures into a non-memory backend (PSS_SPARQ_BACKEND={backend}): \
             SOLID_SERVER_SEED_CONFORMANCE / SOLID_SERVER_SEED_BENCH / SOLID_SERVER_SEED_DEMO would \
             write dev fixtures into a live store. Unset the seed flag(s), use \
             PSS_SPARQ_BACKEND=memory, or — ONLY for an ephemeral test instance — set \
             SOLID_SERVER_ALLOW_SEED_NONMEMORY=1."
        )
        .into());
    }

    // [GPT-5.6] Each backend arm returns its router plus at most ONE boot-owned reconciler task.
    // Crucially, the OFF path keeps the original direct backend construction (no Arc allocation or
    // indirection); only an explicitly-present interval selects shared handles for request path + sweep.
    let reconcile_interval = reconcile_interval_from_env()?;
    let (app, reconcile_task) = match backend.as_str() {
        "memory" => {
            eprintln!("  STORAGE: SPARQ backend = MEMORY (in-memory double — boot-without-SPARQ; the conformance/test default).");
            build_app_for_backend(
                InMemorySparqClient::new(),
                InMemoryBlobStore::new(),
                reconcile_interval,
                &base_url,
                &issuer,
                jwks_cache_ttl,
                auth,
                overload_config,
                identity,
            )
            .await?
        }
        #[cfg(feature = "http-sparq")]
        "http" => {
            let endpoint = std::env::var(ENV_SPARQ_ENDPOINT).map_err(|_| {
                format!(
                    "PSS_SPARQ_BACKEND=http requires {ENV_SPARQ_ENDPOINT} (the SPARQ /sparql URL)"
                )
            })?;
            eprintln!("  STORAGE: SPARQ backend = HTTP (live SPARQL endpoint {endpoint}).");
            build_app_for_backend(
                HttpSparqClient::new(endpoint),
                InMemoryBlobStore::new(),
                reconcile_interval,
                &base_url,
                &issuer,
                jwks_cache_ttl,
                auth,
                overload_config,
                identity,
            )
            .await?
        }
        #[cfg(feature = "embedded-sparq")]
        "embedded" => {
            // The IN-PROCESS engine over a fresh in-memory Graph, or a directory-backed one when a
            // persistence dir is configured. A construction failure aborts boot (fail-closed).
            let sparq = match std::env::var(ENV_SPARQ_DIR)
                .ok()
                .filter(|d| !d.trim().is_empty())
            {
                Some(dir) => {
                    eprintln!("  STORAGE: SPARQ backend = EMBEDDED (in-process engine; directory-backed graph at {dir}).");
                    sparq_lws_core::store::embedded::EmbeddedSparqClient::open(
                        std::path::Path::new(&dir),
                    )
                    .map_err(|e| format!("failed to open the embedded SPARQ graph at {dir}: {e}"))?
                }
                None => {
                    eprintln!("  STORAGE: SPARQ backend = EMBEDDED (in-process engine; fresh in-memory graph).");
                    sparq_lws_core::store::embedded::EmbeddedSparqClient::in_memory()
                        .map_err(|e| format!("failed to init the embedded SPARQ graph: {e}"))?
                }
            };
            build_app_for_backend(
                sparq,
                InMemoryBlobStore::new(),
                reconcile_interval,
                &base_url,
                &issuer,
                jwks_cache_ttl,
                auth,
                overload_config,
                identity,
            )
            .await?
        }
        #[cfg(not(feature = "http-sparq"))]
        "http" => {
            return Err(
                "PSS_SPARQ_BACKEND=http requires the opt-in `http-sparq` build feature (the \
                 remote shared-service backend) — rebuild with `cargo build --release --features \
                 http-sparq`, or use PSS_SPARQ_BACKEND=memory|embedded (the in-process engine, \
                 compiled in by default)."
                    .into(),
            );
        }
        #[cfg(not(feature = "embedded-sparq"))]
        "embedded" => {
            return Err(
                "PSS_SPARQ_BACKEND=embedded requires the `embedded-sparq` build feature (ON by \
                 default; this binary was built with --no-default-features) — rebuild with the \
                 default feature set, or use PSS_SPARQ_BACKEND=memory."
                    .into(),
            );
        }
        other => {
            return Err(format!(
                "unknown {ENV_SPARQ_BACKEND}={other:?} — expected `memory` (default), `embedded` \
                 (the in-process engine, `embedded-sparq` feature — default-on), or `http` (the \
                 opt-in `http-sparq` feature)."
            )
            .into());
        }
    };

    // Build the rustls config (reads + validates the PEM files) for TLS mode; `None` for plain mode.
    // Done after the router is assembled but before binding, so a bad cert/key fails at boot.
    let rustls_config = tls::build_rustls_config(&tls_mode, mtls_bound_tokens)
        .await
        .map_err(|e| format!("TLS configuration error: {e}"))?;

    let scheme = if rustls_config.is_some() {
        "https"
    } else {
        "http"
    };
    eprintln!("solid-server-rs (EXPERIMENTAL) listening on {scheme}://{bind} (base {base_url})");
    eprintln!("  trusted issuer: {issuer}  audience: {audience}  bidirectional: {bidirectional:?}");
    if let TlsMode::Tls {
        cert_path,
        key_path,
    } = &tls_mode
    {
        eprintln!(
            "  TLS: terminating HTTPS in-process (cert {}, key {})",
            cert_path.display(),
            key_path.display()
        );
        // P1.3 (beyond-50k): report the TLS session-resumption posture — the stateful cache bound and
        // whether the opt-in stateless RFC 5077 ticketer is installed. 0-RTT is always OFF (anti-replay).
        let session_cache_size = tls::session_cache_size_from_env();
        if tls::stateless_tickets_from_env() {
            eprintln!(
                "  TLS resumption: STATELESS tickets ON (aws-lc-rs RFC 5077 ticketer, per-process keys \
                 rotated ~6h) + stateful cache bound {session_cache_size}; 0-RTT OFF. NB tickets are NOT \
                 shared across a scaled fleet — cross-node resumption falls back to a full handshake."
            );
        } else if session_cache_size == 0 {
            eprintln!("  TLS resumption: DISABLED (session cache size 0, stateless tickets off); 0-RTT OFF.");
        } else {
            eprintln!(
                "  TLS resumption: stateful session cache (bound {session_cache_size}); stateless tickets \
                 off; 0-RTT OFF. Set SOLID_SERVER_TLS_STATELESS_TICKETS=1 for stateless tickets."
            );
        }
        if mtls_bound_tokens {
            eprintln!(
                "  mTLS (PoP Tier-1b): RFC 8705 cert-bound tokens ENABLED — the handshake REQUESTS an \
                 optional client certificate (plain-DPoP clients unaffected); a cert-bound token is \
                 matched against the presented cert fail-closed."
            );
        }
    } else {
        if mtls_bound_tokens {
            eprintln!(
                "  WARNING: SOLID_SERVER_MTLS_BOUND_TOKENS is set but in-process TLS is OFF — the mTLS \
                 cert-bound path needs in-process TLS to see a client certificate, so it is INACTIVE on \
                 the plain-HTTP path (a cert-bound token would be denied fail-closed). Set \
                 SOLID_SERVER_TLS_CERT + _KEY to enable it."
            );
        }
        eprintln!("  TLS: plain HTTP — terminate TLS at a reverse proxy (set SOLID_SERVER_TLS_CERT + _KEY to enable in-process HTTPS).");
    }
    if allow_loopback {
        eprintln!("  WARNING: SOLID_SERVER_ALLOW_LOOPBACK is set — http:/loopback IdP+WebID permitted (DEV/IT ONLY).");
    }
    eprintln!("WARNING: experimental parallel track — NOT the production prod-solid-server.");

    match rustls_config {
        // HTTPS: axum-server terminates TLS over the process-wide aws-lc-rs rustls provider, with the
        // SAME graceful-shutdown + bind-resolution behaviour as the plain-HTTP path below.
        //
        // Bind-addr PARITY: resolve `bind` with `tokio::net::TcpListener::bind`, exactly as the plain
        // path does, so a hostname:port string (e.g. `localhost:3000`) works in TLS mode too — it
        // would be rejected by `SocketAddr::parse`, which only accepts a numeric address. We then hand
        // the already-bound listener to `axum_server::from_tcp_rustls`, which serves TLS over an
        // existing `std::net::TcpListener` (so no second, parse-restricted bind).
        Some(config) => {
            let tokio_listener = tokio::net::TcpListener::bind(&bind).await?;
            #[cfg(feature = "http3")]
            let http3_addr = tokio_listener.local_addr()?;

            // [GPT-5.6] The h3 listener gets a clone of the ONE already-assembled, hardened router.
            // `serve_h3` injects the Quinn peer as `ConnectInfo<SocketAddr>` before dispatch, so the
            // OUTERMOST pre-crypto rate limiter sees the real source instead of failing open to auth.
            // Endpoint construction happens before the TCP listener is handed off: a UDP bind/config
            // failure aborts startup rather than silently serving only the unadvertised TCP subset.
            #[cfg(feature = "http3")]
            let (app, http3_server) = {
                let endpoint = bind_http3_endpoint(&config, http3_addr)?;
                let bound_addr = endpoint.local_addr()?;
                // [GPT-5.6] sq-oprna.4: an Alt-Svc promise is created only after UDP bind succeeds,
                // and names the endpoint's actual port. The unlayered clone remains the h3 router.
                let h3_app = app.clone();
                let tcp_app = app.layer(sparq_http3::alt_svc_layer(bound_addr.port()));
                eprintln!(
                    "  HTTP/3: QUIC listener ACTIVE on udp://{bound_addr} (ALPN h3; shared LDP router; \
                     WebSocket extended CONNECT is out of scope)."
                );
                let server = tokio::spawn(sparq_http3::serve_h3(
                    endpoint,
                    h3_app,
                    shutdown_signal(),
                ));
                (tcp_app, server)
            };

            // axum-server wants a blocking `std::net::TcpListener`; converting the tokio one keeps the
            // resolved address (and avoids re-binding through the numeric-only `SocketAddr` path).
            let std_listener = tokio_listener.into_std()?;
            std_listener.set_nonblocking(true)?;

            // Graceful-shutdown PARITY: wire `shutdown_signal()` to `Handle::graceful_shutdown` so
            // Ctrl-C drains in-flight TLS connections instead of dropping them, matching the plain
            // path's `with_graceful_shutdown`.
            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                // DRAIN TIMEOUT (best-call, per the standing rule): give in-flight requests up to 10s
                // to finish, then force-close. 10s is the conventional reverse-proxy / k8s
                // `terminationGracePeriod` default — long enough for a normal LDP request (incl. a
                // backend SPARQ round-trip) to complete, short enough that a stuck connection cannot
                // wedge shutdown. `Some(..)` (not `None`) is deliberate: `None` would wait FOREVER for
                // a hung connection, which is the opposite of graceful.
                shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
            });

            // Build the axum-server, then:
            //  (a) `.map(..)` wraps the RustlsAcceptor with the connection-cap acceptor so each served
            //      connection holds a permit for its lifetime (the slowloris connection-flood bound),
            //  (b) `.http_builder()` applies the HTTP/2 (max_concurrent_streams + rapid-reset cap) +
            //      HTTP/1.1 (slowloris header-read timeout, keep-alive) knobs to the SAME hyper
            //      `auto::Builder` axum-server serves with — preserving rustls TLS + h2/http1.1 ALPN
            //      exactly (the rustls config owns ALPN; we touch only the hyper protocol knobs).
            let handshake_timeout = transport_config.handshake_timeout;
            let idle_timeout = transport_config.idle_timeout;
            let max_requests_per_conn = transport_config.max_requests_per_conn;
            // Bind the guarded acceptor into `base` (the PoP Tier-1b/Tier-2 dispatch below consumes
            // `base` in both the wrapped and unwrapped serve branches).
            let base = axum_server::from_tcp_rustls(std_listener, config)?
                .handle(handle)
                .map(move |acceptor| {
                    // P1.5 (beyond-50k): set TCP_NODELAY on the raw accepted TcpStream by composing
                    // axum-server's `NoDelayAcceptor` as the INNER acceptor of the RustlsAcceptor — it
                    // runs on the raw stream BEFORE the TLS handshake, so small responses are not held
                    // by Nagle. Type-preserving: `RustlsAcceptor<NoDelayAcceptor>::Stream` is still
                    // `TlsStream<TcpStream>` (NoDelayAcceptor::Stream = TcpStream), so every guard/PoP
                    // wrapper downstream is unchanged.
                    let acceptor =
                        acceptor.acceptor(sparq_lws_core::nodelay::NoDelayAcceptor::new());
                    // The FULL guard set: connection-cap permit (global + per-source) + handshake
                    // timeout (accept-time) + idle-keepalive read timeout (IO-layer) +
                    // max-requests-per-conn (service-layer).
                    connection_limiter.wrap_acceptor_with_guards(
                        acceptor,
                        handshake_timeout,
                        idle_timeout,
                        max_requests_per_conn,
                    )
                });

            // `into_make_service_with_connect_info::<SocketAddr>()` (NOT plain `into_make_service`) so
            // each request carries `ConnectInfo<SocketAddr>` in its extensions — the pre-crypto rate
            // limiter reads the direct peer IP from it. axum-server supports the connect-info make
            // service. Without this the limiter would see no peer IP and FAIL OPEN (proceed to auth).
            //
            // PoP Tier-1b / Tier-2: when the mTLS flag is on (Tier 1b) OR the DPoP-SK exporter path
            // is active (Tier 2 on in-process TLS), wrap the connection-cap acceptor with the
            // `ConnPopAcceptor` OUTERMOST — it reads the peer client certificate ONCE per connection
            // (after the handshake) and injects a `ConnPop` into every request so the auth layer can
            // match a cert-bound token against it; with the SK exporter enabled it ALSO exports the
            // DPoP-SK keying material once per connection and injects a `ConnSk` (connection id +
            // establishment-path EKM). With neither flag, the serve path is byte-identical (no
            // wrap), so the `.map` type differs between branches — hence the duplicated serve call.
            // (With SK-only, the injected `ConnPop` carries no cert and the mTLS dispatch stays off —
            // the cert path is untouched.)
            if mtls_bound_tokens || sk_exporter_active {
                let mut server = base.map(move |acceptor| {
                    sparq_lws_core::pop::conn::ConnPopAcceptor::new(acceptor)
                        .with_sk_exporter(sk_exporter_active)
                });
                transport_config.apply_to_builder(server.http_builder());
                server
                    .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                    .await?;
            } else {
                let mut server = base;
                transport_config.apply_to_builder(server.http_builder());
                server
                    .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                    .await?;
            }

            // Both listener shutdown futures observe the same process signal. Await the UDP task so
            // its endpoint closes and drains QUIC connections before the process exits normally.
            #[cfg(feature = "http3")]
            http3_server.await??;
        }
        // Plain TCP (dev/test behaviour). Graceful shutdown on Ctrl-C.
        //
        // The transport-layer knobs (HTTP/2 stream/reset caps, slowloris header-read timeout, the
        // connection cap) are NOT applied on this path: `axum::serve` builds its hyper `auto::Builder`
        // internally and exposes neither it nor a connection-cap hook, and an axum `Listener` wrapper
        // cannot carry the `ConnectInfo<SocketAddr>` the pre-crypto rate limiter needs (axum's
        // `Connected` impls are orphan-locked to `TcpListener`/`TapIo`). Plain HTTP is the DEV/TEST /
        // TLS-at-a-reverse-proxy posture; in production the in-process TLS path (fully hardened above)
        // or a fronting reverse proxy terminates HTTP/2 and caps connections. The application-layer
        // defences (admission control, the per-IP rate limiter, the request timeout) DO apply here.
        None => {
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            // `connection_limiter` is unused on this path (see above); a fronting proxy / the TLS path
            // owns the connection cap in production. Drop it explicitly so the intent is clear.
            drop(connection_limiter);
            // P1.5 (beyond-50k): set TCP_NODELAY on every accepted plain-TCP stream. `axum::serve`
            // does NOT do this by default; `tap_nodelay` wraps the listener with the `ListenerExt`
            // tap so each accepted stream gets the option (Nagle off), matching the TLS path. `TapIo`
            // preserves `Io = TcpStream` / `Addr = SocketAddr`, so connect-info still carries the peer
            // IP to the rate limiter.
            let listener = sparq_lws_core::nodelay::tap_nodelay(listener);
            // Same as the TLS path: serve WITH connect-info so the rate limiter sees the peer IP.
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal())
            .await?;
        }
    }
    // The runtime would cancel this task on process exit, but abort explicitly after the listener
    // drains so the task's ownership/lifetime stays bounded to the server boot that created it.
    if let Some(task) = reconcile_task {
        task.abort();
    }
    Ok(())
}

/// Build one selected backend in one of two shapes: the original direct store when reconciliation is
/// OFF, or shared handles owned by both the request path and one periodic task when it is ON. Keeping
/// the branch here makes the default path allocation- and indirection-identical to the pre-wiring boot.
#[allow(clippy::too_many_arguments)]
async fn build_app_for_backend<S, B, J, R>(
    sparq: S,
    blob: B,
    reconcile_interval: Option<Duration>,
    base_url: &str,
    issuer: &str,
    jwks_cache_ttl: Duration,
    auth: AuthContext<J, R>,
    overload_config: OverloadConfig,
    identity: Option<IdentityConfig>,
) -> Result<(axum::Router, Option<tokio::task::JoinHandle<()>>), Box<dyn std::error::Error>>
where
    S: sparq_lws_core::store::SparqClient + 'static,
    B: sparq_lws_core::store::BlobStore + 'static,
    J: JwksProvider + Send + Sync + 'static,
    R: ReplayStore + Send + Sync + 'static,
{
    if reconcile_interval.is_some() {
        let sparq = SharedStore::new(sparq);
        let blob = SharedStore::new(blob);
        let store = CompositeStore::with_body_cache(
            sparq.clone(),
            blob.clone(),
            body_cache_from_env(),
        );
        let app = build_app_for_store(
            store,
            base_url,
            issuer,
            jwks_cache_ttl,
            auth,
            overload_config,
            identity,
        )
        .await?;
        let task = spawn_periodic_if_configured(sparq, blob, reconcile_interval);
        Ok((app, task))
    } else {
        let store = CompositeStore::with_body_cache(sparq, blob, body_cache_from_env());
        let app = build_app_for_store(
            store,
            base_url,
            issuer,
            jwks_cache_ttl,
            auth,
            overload_config,
            identity,
        )
        .await?;
        Ok((app, None))
    }
}

/// Build the application router for a chosen, already-constructed [`Store`] backend — the SAME
/// downstream wiring for EVERY `PSS_SPARQ_BACKEND` arm (memory / http / embedded).
///
/// Generic over `S: Store` (the `CompositeStore<SparqClient, BlobStore>` the chosen backend
/// monomorphizes) and `J/R` (the auth's JWKS provider + replay store). Each backend arm in `main`
/// calls this with its own `S`, so there is ONE seed → `LdpState` → ACL-cache → `AppState` → router
/// sequence, not three. Keeping it generic is what makes adding the embedded backend a one-arm change
/// with no consumer-code duplication (the maintainer's "generic-over-S seam = one-line swap").
///
/// The dev/conformance + dev/bench seeding is gated exactly as before; it runs against whatever `S`
/// is selected (the conformance/bench seeds are still in-memory-only in practice — they are gated by
/// their own env flags and only ever set against the memory backend).
#[allow(clippy::too_many_arguments)]
async fn build_app_for_store<S, J, R>(
    store: S,
    base_url: &str,
    issuer: &str,
    jwks_cache_ttl: Duration,
    auth: AuthContext<J, R>,
    overload_config: OverloadConfig,
    identity: Option<IdentityConfig>,
) -> Result<axum::Router, Box<dyn std::error::Error>>
where
    S: Store + 'static,
    J: JwksProvider + Send + Sync + 'static,
    R: ReplayStore + Send + Sync + 'static,
{
    // Dev/conformance seeding (gated): write the test users' WebID profiles + the container tree the
    // Solid CTH dereferences to bootstrap. Done BEFORE the store is moved into the LDP state; a seeding
    // failure aborts boot (better than a half-seeded store). In identity mode the seed mints id-host
    // WebIDs (`<identity-origin>/<handle>#me`, LOCKED issuer + storage, id-doc at the RESERVED
    // `/.identity/<handle>` key) and DEMOTES the in-pod cards — see `seed_conformance_with_identity`.
    if env_flag(ENV_SEED_CONFORMANCE) {
        sparq_lws_core::seed::seed_conformance_with_identity(
            &store,
            base_url,
            issuer,
            identity.as_ref(),
        )
        .await
        .map_err(|e| format!("conformance seeding failed: {e:?}"))?;
        match &identity {
            Some(config) => eprintln!(
                "  SEEDED conformance users {:?} (id-host WebIDs {}/<handle>#me + demoted in-pod \
                 cards) — DEV/CONFORMANCE ONLY.",
                sparq_lws_core::seed::SEED_USERS,
                config.origin()
            ),
            None => eprintln!(
                "  SEEDED conformance users {:?} (WebID profiles + container tree) — DEV/CONFORMANCE ONLY.",
                sparq_lws_core::seed::SEED_USERS
            ),
        }
    }

    // Dev/benchmark seeding (gated): purely additive fixtures for the HTTPS load benchmark. Like the
    // conformance seed it only writes resources; it changes no request handling.
    if let Some(child_count) = bench_seed_count(ENV_SEED_BENCH) {
        let owner_override = std::env::var(ENV_SEED_BENCH_OWNER)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let fixtures = sparq_lws_core::seed::seed_bench_with_owner(
            &store,
            base_url,
            child_count,
            owner_override.as_deref(),
        )
        .await
        .map_err(|e| format!("bench seeding failed: {e:?}"))?;
        eprintln!(
            "  SEEDED bench fixtures — DEV/BENCH ONLY: public_doc={} listing={} ({} children) private_doc={} owner={}",
            fixtures.public_doc, fixtures.listing, fixtures.child_count, fixtures.private_doc, fixtures.owner
        );
    }

    // Demo seeding (gated): the opt-in §3.2 demo playground — a shared root-level container any
    // AUTHENTICATED agent can read/write/append (anonymous visitors read-only; nobody has Control)
    // plus a public-read /README carrying the ephemeral-demo banner. Like the other seeds it is
    // purely additive fixture content; it changes no request handling.
    if let Some(fixtures) = maybe_seed_demo(&store, base_url).await? {
        eprintln!(
            "  SEEDED demo playground — DEMO ONLY: playground={} (authenticated read+write+append, public read, no Control) readme={}",
            fixtures.playground, fixtures.readme
        );
    }

    let mut ldp = LdpState::new(store, base_url.to_string());
    // ETag-keyed parsed-ACL cache (read-path optimisation #3). Default-on; `=0` disables it (byte-
    // identical pre-cache behaviour). A cache HIT reuses the parsed `.acl` triples of an UNCHANGED ACL
    // across reads (keyed by `(acl-iri, etag)`) — it can never serve a rotated/removed ACL stale, so
    // it never changes a decision. The validation TTL is bound by the JWKS cache TTL (same freshness
    // window the auth caches use) so a misbehaving etag can never mask a change indefinitely.
    let acl_cache_capacity = parse_cache_capacity_for(
        std::env::var(ENV_ACL_CACHE_CAPACITY).ok(),
        DEFAULT_ACL_CACHE_CAPACITY,
    );
    let acl_cache = if acl_cache_capacity == 0 {
        eprintln!("  AUTHZ: ETag-keyed parsed-ACL cache DISABLED (re-read + re-parse every ACL).");
        AclCache::disabled()
    } else {
        eprintln!(
            "  AUTHZ: ETag-keyed parsed-ACL cache ENABLED (capacity {acl_cache_capacity} ACLs; \
             reuses an UNCHANGED ACL's parse, never authoritative)."
        );
        AclCache::with_max_entry_ttl(acl_cache_capacity, jwks_cache_ttl.as_secs() as i64)
    };
    ldp.set_acl_cache(acl_cache);

    // Identity-host serving (when configured) rides the same AppState — the gate is mounted by the
    // router assembly; the namespace refusal is mounted regardless.
    let mut app_state = AppState::new(auth, ldp);
    if let Some(config) = identity {
        app_state = app_state.with_identity(config);
    }
    Ok(build_router_with_overload(app_state, overload_config))
}

/// Format an optional seconds-duration for a startup log line: `Some(d)` ⇒ `"<n>s"`, `None` ⇒
/// `"DISABLED"`. Used for the transport-hardening timeout knobs.
fn fmt_opt_secs(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("{}s", d.as_secs()),
        None => "DISABLED".to_string(),
    }
}

/// Read a boolean-ish env flag, FAIL-CLOSED: exactly `1` / `true` / `TRUE` / `True`
/// (after trimming) ⇒ true; anything else — absent, empty, `0`, `false`, `no`, `yes`,
/// `01`, `1abc` — ⇒ false.
///
/// [OPUS-5] sq-5ougp review round 3: this doc previously said "`1` / `true`
/// (case-insensitive)", which is INACCURATE — mixed-case spellings such as `tRuE` are
/// rejected. Only the four literals above are accepted. The error direction is safe (an
/// unrecognised value means OFF), but the doc must not promise a laxer parse than the
/// code performs.
fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref().map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("True")
    )
}

/// [OPUS-5] sq-5ougp review round 3 (gpt-5.6-sol finding 1): the DEMO-SEED BOOT CALL
/// SITE, extracted from `build_app_for_store` so the **off-by-default** invariant is
/// TESTABLE at the site that actually decides it.
///
/// Before this extraction the only "flag off" test built a store and simply *declined
/// to call* `seed_demo`, so it would have passed unchanged if the binary seeded the
/// demo unconditionally. The gate now lives in one named function that boot calls and
/// `demo_seed_boot_call_site_*` drives: `None` means the boot performed NO seed write
/// at all (not "seeded something empty").
///
/// Fail-closed by construction: the gate is [`env_flag`], which accepts ONLY
/// `1|true|TRUE|True` (trimmed) — every other value, including `yes`/`01`/`tRuE`, is
/// OFF. A seeding failure propagates and aborts boot rather than leaving a
/// half-seeded store.
async fn maybe_seed_demo<S: Store>(
    store: &S,
    base_url: &str,
) -> Result<Option<sparq_lws_core::seed::DemoFixtures>, String> {
    if !env_flag(ENV_SEED_DEMO) {
        return Ok(None);
    }
    let fixtures = sparq_lws_core::seed::seed_demo(store, base_url)
        .await
        .map_err(|e| format!("demo seeding failed: {e:?}"))?;
    Ok(Some(fixtures))
}

/// [OPUS-5] sq-5ougp review round 3 (gpt-5.6-sol finding 2): GUARD #2's seed-request
/// predicate, extracted from the boot body so the DEMO disjunct is testable.
///
/// Deleting `|| env_flag(ENV_SEED_DEMO)` here is the mutation that previously failed
/// no test, and it is the only thing stopping the demo playground ACL from being
/// written into a live `http` / `embedded` store. Feeds
/// [`reject_seed_on_nonmemory`].
fn seed_requested_from_env() -> bool {
    env_flag(ENV_SEED_CONFORMANCE)
        || bench_seed_count(ENV_SEED_BENCH).is_some()
        || env_flag(ENV_SEED_DEMO)
}

/// Resolve the verified-access-token cache capacity from the env value.
/// - absent / empty / non-numeric => [`DEFAULT_CACHE_CAPACITY`] (cache ENABLED, default size);
/// - `0` => `0` (cache DISABLED -- full verify per request);
/// - `>0` => that literal capacity (clamped to >=1 internally by the cache).
///
/// A non-numeric value falls back to the default rather than silently disabling the cache (disabling
/// is an explicit `0` only) -- a fat-fingered value should not quietly drop the perf win, and the cache
/// is fail-safe (a miss just re-verifies).
fn parse_cache_capacity(raw: Option<String>) -> usize {
    parse_cache_capacity_for(raw, DEFAULT_CACHE_CAPACITY)
}

/// As [`parse_cache_capacity`], but with an explicit `default` for the absent/empty/non-numeric case —
/// shared by the token cache (`DEFAULT_CACHE_CAPACITY`), the ACL cache
/// ([`DEFAULT_ACL_CACHE_CAPACITY`]), and the blob-body cache ([`DEFAULT_BODY_CACHE_BYTES`]). `0` is
/// the explicit DISABLE; a fat-fingered non-numeric value falls back to `default` (enabled) rather
/// than silently dropping the perf win.
fn parse_cache_capacity_for(raw: Option<String>, default: usize) -> usize {
    match raw.as_deref().map(str::trim) {
        None | Some("") => default,
        Some(s) => s.parse::<usize>().unwrap_or(default),
    }
}

/// Build the blob-body cache (read-4) from the env: `SOLID_SERVER_BODY_CACHE_BYTES` (byte budget,
/// unset ⇒ [`DEFAULT_BODY_CACHE_BYTES`], `0` ⇒ DISABLED) + `SOLID_SERVER_BODY_CACHE_MAX_ENTRY_BYTES`
/// (per-entry cap, unset ⇒ 1/8 of the budget). Logs the resolved config (called once — exactly one
/// backend arm runs per boot). The cache can never serve stale bytes or bypass authorization — see
/// [`sparq_lws_core::store::body_cache`]'s module docs for both arguments.
fn body_cache_from_env() -> BodyCache {
    let budget = parse_cache_capacity_for(
        std::env::var(ENV_BODY_CACHE_BYTES).ok(),
        DEFAULT_BODY_CACHE_BYTES,
    );
    if budget == 0 {
        eprintln!("  STORAGE: blob-body cache DISABLED (every read pays the blob fetch).");
        return BodyCache::disabled();
    }
    let cache = match std::env::var(ENV_BODY_CACHE_MAX_ENTRY_BYTES)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<usize>().ok())
    {
        Some(cap) => BodyCache::with_max_entry_bytes(budget, cap),
        None => BodyCache::new(budget),
    };
    eprintln!(
        "  STORAGE: blob-body cache ENABLED (budget {} bytes, per-entry cap {} bytes; keyed \
         (blob_key, etag) from the authoritative index — never stale, never an authz surface).",
        cache.max_bytes(),
        cache.max_entry_bytes()
    );
    cache
}

/// Resolve the bench-seed child count from `key`. Returns `None` when bench seeding is OFF
/// (the var is absent / empty / `false` / numerically `0`), else `Some(n)` children:
///
/// - **A NUMERIC value is interpreted by its INTEGER VALUE** (after trimming, so leading zeros do not
///   change meaning — `0`/`00` ⇒ OFF, `1`/`01` ⇒ the default count, `>1` ⇒ that literal count):
///   - `0` ⇒ OFF (`None`);
///   - `1` ⇒ enable with the DEFAULT count ([`seed::BENCH_DEFAULT_CHILDREN`]) — `1` means "switch
///     bench seeding ON", NOT "seed exactly one child", matching the documented default behaviour;
///   - `>1` ⇒ that literal child count (`10`, `100`, …).
/// - **Non-numeric tokens**: `false` (case-insensitive) ⇒ OFF; `true` / `yes` (case-insensitive) ⇒
///   enable with the DEFAULT count; any other truthy-but-non-numeric value also uses the default.
///
/// (Roborev: parse the numeric value FIRST so padded forms like `00` / `01` are handled by their
/// integer value, not a string-equality special-case that `00`/`01` would bypass.)
fn bench_seed_count(key: &str) -> Option<usize> {
    let raw = std::env::var(key).ok()?;
    let raw = raw.trim();
    // Empty / explicit `false` ⇒ OFF.
    if raw.is_empty() || raw.eq_ignore_ascii_case("false") {
        return None;
    }
    // A NUMERIC value is decided by its integer VALUE (so `0`/`00` ⇒ OFF, `1`/`01` ⇒ default, >1 ⇒
    // literal) — never by the raw string, which would let `00`/`01` slip past a `== "0"`/`== "1"` test.
    if let Ok(n) = raw.parse::<usize>() {
        return match n {
            0 => None,
            1 => Some(sparq_lws_core::seed::BENCH_DEFAULT_CHILDREN),
            _ => Some(n),
        };
    }
    // Non-numeric truthy tokens: `true` / `yes` (and any other non-OFF value) ⇒ enable, default count.
    Some(sparq_lws_core::seed::BENCH_DEFAULT_CHILDREN)
}

/// The production DPoP-replay-store capacity (live `jti`s), matching `InMemoryReplayStore::with_window`
/// (the verifier's TS-parity default). The bench override can only RAISE above this — never below.
const REPLAY_PRODUCTION_DEFAULT: u64 = 100_000;

/// Parse the dev/bench-only DPoP-replay-store capacity override ([`ENV_REPLAY_MAX_ENTRIES`]).
///
/// Returns `Some(n)` ONLY for a positive integer STRICTLY GREATER than the production default
/// ([`REPLAY_PRODUCTION_DEFAULT`]); `None` for absent / empty / non-numeric / `0` / any value
/// `<= REPLAY_PRODUCTION_DEFAULT` — in which case the caller keeps the production default (via
/// `InMemoryReplayStore::with_window`). The override can therefore ONLY RAISE the cap: a smaller (or
/// zero) value can never SHRINK it, so a mis-set var can never degrade availability (a too-small cap
/// would fail authenticated traffic closed sooner — roborev Medium) nor weaken replay protection. The
/// var's only effect is to lift the cap for a benchmark — exactly as documented.
fn parse_replay_max_entries(raw: Option<String>) -> Option<u64> {
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&n| n > REPLAY_PRODUCTION_DEFAULT)
}

/// Build the per-instance in-memory DPoP-replay store at the production default capacity, RAISED only by
/// the dev/bench-only `ENV_REPLAY_MAX_ENTRIES` override (which can only ever raise — see
/// [`parse_replay_max_entries`]). Extracted so both the default path and the Redis-feature path
/// construct the in-memory store identically. `replay_ttl` is the verifier's `max_age + tolerance`.
fn build_in_memory_replay(replay_ttl: Duration) -> InMemoryReplayStore {
    match parse_replay_max_entries(std::env::var(ENV_REPLAY_MAX_ENTRIES).ok()) {
        Some(max_entries) => {
            eprintln!(
                "  DEV/BENCH: DPoP replay-store capacity RAISED to {max_entries} live jtis \
                 (production default is {REPLAY_PRODUCTION_DEFAULT}) — NEVER set this in production."
            );
            InMemoryReplayStore::new(max_entries, replay_ttl)
        }
        None => InMemoryReplayStore::with_window(replay_ttl),
    }
}

/// Parse the bidirectional-check mode env value. Default (absent/unknown) is `Strict` — the secure
/// posture: a WebID whose profile does not list the token issuer is rejected.
fn parse_bidirectional(raw: Option<&str>) -> BidirectionalMode {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("off") => BidirectionalMode::Off,
        Some("warn") => BidirectionalMode::Warn,
        // "strict", any other value, or absent ⇒ the secure default.
        _ => BidirectionalMode::Strict,
    }
}

/// Resolve when EITHER Ctrl-C (SIGINT) OR SIGTERM is received, so the server drains gracefully on a
/// container/orchestrator stop, not just an interactive Ctrl-C. SIGTERM is what a load balancer /
/// k8s / systemd / Docker sends to ask an instance to stop: handling it means an instance behind an LB
/// deregisters CLEANLY (finishes in-flight requests within the drain window, then exits) instead of
/// having connections dropped — the graceful-drain half of the stateless-instances-behind-LB design.
/// On non-Unix there is no SIGTERM, so only Ctrl-C is awaited (unchanged behaviour there).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    {
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut sig) => {
                    sig.recv().await;
                }
                // If we cannot install the SIGTERM handler, fall back to never resolving on this arm
                // (Ctrl-C still works) rather than aborting — a missing SIGTERM handler must not crash
                // a running server.
                Err(e) => {
                    eprintln!(
                        "  WARNING: could not install SIGTERM handler ({e}); Ctrl-C still drains."
                    );
                    std::future::pending::<()>().await;
                }
            }
        };
        tokio::select! {
            _ = ctrl_c => { eprintln!("  SHUTDOWN: SIGINT (Ctrl-C) received — draining."); }
            _ = terminate => { eprintln!("  SHUTDOWN: SIGTERM received — draining (clean LB deregistration)."); }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await;
        eprintln!("  SHUTDOWN: Ctrl-C received — draining.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [GPT-5.6] sq-oprna.2: the QUIC endpoint must gain `h3` without mutating the TCP ALPN list.
    /// A missing `h3` mutation is rejected by `sparq_http3::quic_server_config`; mutating the shared
    /// config instead of its clone makes the second assertion fail.
    #[cfg(feature = "http3")]
    #[tokio::test]
    async fn http3_endpoint_preserves_tcp_alpn() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mode = TlsMode::Tls {
            cert_path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-cert.pem").into(),
            key_path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key.pem").into(),
        };
        let tcp_tls = tls::build_rustls_config(&mode, false)
            .await
            .expect("build fixture TLS config")
            .expect("TLS mode yields a config");
        let tcp_alpn = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        assert_eq!(tcp_tls.get_inner().alpn_protocols, tcp_alpn);

        let endpoint = bind_http3_endpoint(
            &tcp_tls,
            "127.0.0.1:0".parse().expect("loopback socket address"),
        )
        .expect("bind HTTP/3 endpoint from cloned TLS config");
        assert_eq!(
            tcp_tls.get_inner().alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            "building QUIC must not add h3 to the TCP ALPN list"
        );

        endpoint.close(quinn::VarInt::from_u32(0), b"test complete");
        endpoint.wait_idle().await;
    }

    /// Set `key` to `val` (or remove it when `None`), run `f`, then restore the prior value. Each
    /// test uses a UNIQUE key so concurrent test threads never read the same process-global env var.
    fn with_env(key: &str, val: Option<&str>, f: impl FnOnce()) {
        let prev = std::env::var(key).ok();
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        f();
        match prev {
            Some(p) => std::env::set_var(key, p),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn bench_seed_off_tokens_disable_seeding() {
        for tok in ["", "0", "false", "False", "FALSE"] {
            let key = format!("SSR_TEST_BENCH_OFF_{}", tok.len());
            with_env(&key, Some(tok), || {
                assert_eq!(bench_seed_count(&key), None, "token {tok:?} must disable");
            });
        }
        // Absent var ⇒ off.
        assert_eq!(bench_seed_count("SSR_TEST_BENCH_ABSENT_VAR"), None);
    }

    #[test]
    fn bench_seed_bare_truthy_uses_default_count() {
        // `1` / `true` / `yes` mean "enable with the DEFAULT count" — NOT "seed exactly one child".
        for tok in ["1", "true", "TRUE", "True", "yes", "YES"] {
            let key = format!("SSR_TEST_BENCH_TRUTHY_{tok}");
            with_env(&key, Some(tok), || {
                assert_eq!(
                    bench_seed_count(&key),
                    Some(sparq_lws_core::seed::BENCH_DEFAULT_CHILDREN),
                    "bare-truthy {tok:?} must mean the default child count, not a literal count"
                );
            });
        }
    }

    #[test]
    fn bench_seed_explicit_count_is_literal() {
        for (tok, want) in [("2", 2usize), ("10", 10), ("100", 100), ("250", 250)] {
            let key = format!("SSR_TEST_BENCH_COUNT_{tok}");
            with_env(&key, Some(tok), || {
                assert_eq!(
                    bench_seed_count(&key),
                    Some(want),
                    "numeric {tok:?} (>1) must be the literal child count"
                );
            });
        }
    }

    #[test]
    fn bench_seed_truthy_non_numeric_falls_back_to_default() {
        let key = "SSR_TEST_BENCH_GARBAGE";
        with_env(key, Some("on"), || {
            // "on" is truthy (not an OFF token) but not numeric/recognised ⇒ default count.
            assert_eq!(
                bench_seed_count(key),
                Some(sparq_lws_core::seed::BENCH_DEFAULT_CHILDREN)
            );
        });
    }

    #[test]
    fn bench_seed_whitespace_is_trimmed() {
        let key = "SSR_TEST_BENCH_WS";
        with_env(key, Some("  1  "), || {
            assert_eq!(
                bench_seed_count(key),
                Some(sparq_lws_core::seed::BENCH_DEFAULT_CHILDREN)
            );
        });
        with_env(key, Some("  10  "), || {
            assert_eq!(bench_seed_count(key), Some(10));
        });
    }

    #[test]
    fn bench_seed_leading_zeros_are_decided_by_integer_value() {
        // Padded numeric forms are interpreted by their VALUE, not the raw string: `00`/`000` ⇒ OFF,
        // `01` ⇒ default count, `010` ⇒ 10 — so a leading zero can never bypass the 0/1 special-cases.
        let key = "SSR_TEST_BENCH_PADDED";
        with_env(key, Some("00"), || assert_eq!(bench_seed_count(key), None));
        with_env(key, Some("000"), || assert_eq!(bench_seed_count(key), None));
        with_env(key, Some("01"), || {
            assert_eq!(
                bench_seed_count(key),
                Some(sparq_lws_core::seed::BENCH_DEFAULT_CHILDREN)
            )
        });
        with_env(key, Some("010"), || {
            assert_eq!(bench_seed_count(key), Some(10))
        });
    }

    // --- Startup guard predicates ---------------------------------------------------------------

    #[test]
    fn durable_classification_matches_the_backend_matrix() {
        // http is always durable/shared (a dir flag is irrelevant to it).
        assert!(sparq_backend_is_durable("http", false));
        assert!(sparq_backend_is_durable("http", true));
        // embedded is durable ONLY with a persistence dir; without it the graph is in-memory.
        assert!(sparq_backend_is_durable("embedded", true));
        assert!(!sparq_backend_is_durable("embedded", false));
        // memory + unknown are never durable.
        assert!(!sparq_backend_is_durable("memory", true));
        assert!(!sparq_backend_is_durable("memory", false));
        assert!(!sparq_backend_is_durable("bogus", true));
    }

    #[test]
    fn guard1_rejects_durable_sparq_with_inmem_blob() {
        // http + in-mem blob ⇒ REJECT (durable index, ephemeral bytes).
        assert!(reject_durable_sparq_with_inmem_blob("http", false, true));
        // dir-backed embedded + in-mem blob ⇒ REJECT.
        assert!(reject_durable_sparq_with_inmem_blob("embedded", true, true));
    }

    #[test]
    fn guard1_allows_ephemeral_combinations() {
        // embedded WITHOUT a dir + in-mem blob ⇒ ALLOW (both ephemeral, consistent — the test path).
        assert!(!reject_durable_sparq_with_inmem_blob(
            "embedded", false, true
        ));
        // memory + in-mem blob ⇒ ALLOW (the conformance/test default — both ephemeral).
        assert!(!reject_durable_sparq_with_inmem_blob("memory", false, true));
        assert!(!reject_durable_sparq_with_inmem_blob("memory", true, true));
    }

    #[test]
    fn guard1_does_not_fire_once_blob_is_durable() {
        // When a durable BlobStore lands (`blob_is_in_memory == false`), the guard stops firing for
        // EVERY backend — durable SPARQ + durable blob is the consistent production target.
        assert!(!reject_durable_sparq_with_inmem_blob("http", false, false));
        assert!(!reject_durable_sparq_with_inmem_blob(
            "embedded", true, false
        ));
        assert!(!reject_durable_sparq_with_inmem_blob(
            "embedded", false, false
        ));
        assert!(!reject_durable_sparq_with_inmem_blob(
            "memory", false, false
        ));
    }

    #[test]
    fn guard2_rejects_seed_on_nonmemory_without_override() {
        // seed + http/embedded + NO override ⇒ REJECT.
        assert!(reject_seed_on_nonmemory(true, "http", false));
        assert!(reject_seed_on_nonmemory(true, "embedded", false));
    }

    #[test]
    fn guard2_allows_seed_on_nonmemory_with_override() {
        // seed + non-memory + override ⇒ ALLOW (the ephemeral embedded test instance the harness seeds).
        assert!(!reject_seed_on_nonmemory(true, "embedded", true));
        assert!(!reject_seed_on_nonmemory(true, "http", true));
    }

    #[test]
    fn guard2_always_allows_seed_on_memory() {
        // Seeding memory is the seed target by construction — allowed with or without the override.
        assert!(!reject_seed_on_nonmemory(true, "memory", false));
        assert!(!reject_seed_on_nonmemory(true, "memory", true));
    }

    #[test]
    fn guard2_does_not_fire_when_no_seed_requested() {
        // No seed flag set ⇒ never fires, regardless of backend / override.
        assert!(!reject_seed_on_nonmemory(false, "http", false));
        assert!(!reject_seed_on_nonmemory(false, "embedded", false));
        assert!(!reject_seed_on_nonmemory(false, "memory", false));
    }

    // =====================================================================
    // [OPUS-5] sq-5ougp review round 3 — the BOOT-LEVEL demo-seed invariants.
    //
    // gpt-5.6-sol's adversarial cross-provider review of #3476 found that the
    // off-by-default property — the single most important property of an
    // authorization-granting seed — was NOT TESTED ANYWHERE:
    //
    //   finding 1: `demo_seed_flag_off_boot_has_no_playground` never touches
    //     `env_flag`, Guard #2, or the boot call site. It builds a store and simply
    //     declines to call `seed_demo`, so it "would pass unchanged if the real
    //     binary seeded the demo unconditionally".
    //   finding 2: deleting `|| env_flag(ENV_SEED_DEMO)` from Guard #2's
    //     `seed_requested` failed no test — and that disjunct is the only thing
    //     stopping the demo ACL being written into a live `http`/`embedded` store.
    //
    // These tests drive the REAL predicates the boot body now calls
    // (`maybe_seed_demo` / `seed_requested_from_env`) with the REAL env-var names,
    // so both mutations redden.
    // =====================================================================

    /// The real `SOLID_SERVER_*` names are process-global, so unlike [`with_env`]
    /// (which gives each test a unique synthetic key) these tests must serialise.
    static REAL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII set/clear of a REAL env var, restoring the previous value on drop — so a
    /// failing assertion cannot leak the flag into the sibling test.
    struct RealEnvVar {
        key: &'static str,
        prev: Option<String>,
    }

    impl RealEnvVar {
        fn set(key: &'static str, val: Option<&str>) -> Self {
            let prev = std::env::var(key).ok();
            match val {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
            Self { key, prev }
        }
    }

    impl Drop for RealEnvVar {
        fn drop(&mut self) {
            match &self.prev {
                Some(p) => std::env::set_var(self.key, p),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn seed_probe_store() -> CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
        CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
    }

    /// A current-thread runtime so the REAL-env lock is never held across an `.await`
    /// (the env mutation and the seed must be one atomic step for other tests).
    fn seed_probe_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime")
    }

    const SEED_PROBE_BASE: &str = "https://demo-probe.example";

    fn seed_probe_iris() -> [String; 4] {
        [
            format!("{SEED_PROBE_BASE}/playground/"),
            format!("{SEED_PROBE_BASE}/playground/.acl"),
            format!("{SEED_PROBE_BASE}/README"),
            format!("{SEED_PROBE_BASE}/README.acl"),
        ]
    }

    /// FINDING 1, the OFF half. Every value that is not an accepted truthy token must
    /// leave the boot call site having written NOTHING — no `/playground/`, no
    /// `/playground/.acl`, no `/README`, no `/README.acl`. This is the test that
    /// reddens if the binary ever seeds unconditionally, because it drives the real
    /// [`maybe_seed_demo`] gate rather than declining to call `seed_demo`.
    #[test]
    fn demo_seed_boot_call_site_writes_nothing_unless_flag_is_on() {
        let _lock = REAL_ENV_LOCK.lock().unwrap();
        let rt = seed_probe_runtime();
        // `None` = the variable is ABSENT, i.e. the default posture of every boot.
        for raw in [
            None,
            Some(""),
            Some("   "),
            Some("0"),
            Some("false"),
            Some("False"),
            Some("FALSE"),
            Some("no"),
            Some("off"),
            Some("yes"),
            Some("2"),
            Some("01"),
            Some("1abc"),
            Some("tRuE"),
            Some("truthy"),
        ] {
            let _flag = RealEnvVar::set(ENV_SEED_DEMO, raw);
            let store = seed_probe_store();
            let outcome = rt
                .block_on(maybe_seed_demo(&store, SEED_PROBE_BASE))
                .expect("the OFF path must not error");
            assert!(
                outcome.is_none(),
                "SOLID_SERVER_SEED_DEMO={raw:?} must NOT seed the demo: the boot call \
                 site is opt-in and env_flag accepts only 1|true|TRUE|True"
            );
            for iri in seed_probe_iris() {
                assert!(
                    !rt.block_on(store.exists(&iri)).unwrap(),
                    "SOLID_SERVER_SEED_DEMO={raw:?} must leave {iri} ABSENT — an \
                     unconditionally-seeding boot would create it"
                );
            }
        }
    }

    /// FINDING 1, the ON half. The accepted truthy tokens DO seed, and when they do the
    /// boot call site really produced the §3.2 fixtures — so the OFF half above cannot
    /// be satisfied by a [`maybe_seed_demo`] that never seeds at all.
    #[test]
    fn demo_seed_boot_call_site_seeds_on_accepted_truthy_tokens() {
        let _lock = REAL_ENV_LOCK.lock().unwrap();
        let rt = seed_probe_runtime();
        for raw in ["1", "true", "TRUE", "True", "  1  ", "\ttrue\n"] {
            let _flag = RealEnvVar::set(ENV_SEED_DEMO, Some(raw));
            let store = seed_probe_store();
            let fixtures = rt
                .block_on(maybe_seed_demo(&store, SEED_PROBE_BASE))
                .expect("the ON path must not error")
                .unwrap_or_else(|| {
                    panic!(
                        "SOLID_SERVER_SEED_DEMO={raw:?} is an ACCEPTED truthy token \
                         and must seed at the boot call site"
                    )
                });
            assert_eq!(fixtures.playground, format!("{SEED_PROBE_BASE}/playground/"));
            assert_eq!(fixtures.readme, format!("{SEED_PROBE_BASE}/README"));
            for iri in seed_probe_iris() {
                assert!(
                    rt.block_on(store.exists(&iri)).unwrap(),
                    "SOLID_SERVER_SEED_DEMO={raw:?} must create {iri}"
                );
            }
        }
    }

    /// FINDING 2. GUARD #2's seed-request predicate must see the DEMO flag, so that
    /// `SEED_DEMO=1` + a non-`memory` backend REFUSES to boot. Without the
    /// `|| env_flag(ENV_SEED_DEMO)` disjunct the demo playground ACL — a public-read
    /// + authenticated-write grant — would be written into a live `http`/`embedded`
    /// store.
    #[test]
    fn guard2_seed_requested_sees_the_demo_flag_and_refuses_nonmemory_boot() {
        let _lock = REAL_ENV_LOCK.lock().unwrap();
        // Clear the other two seed inputs so ONLY the demo flag varies.
        let _conf = RealEnvVar::set(ENV_SEED_CONFORMANCE, None);
        let _bench = RealEnvVar::set(ENV_SEED_BENCH, None);
        {
            let _flag = RealEnvVar::set(ENV_SEED_DEMO, None);
            assert!(
                !seed_requested_from_env(),
                "no seed flag set ⇒ Guard #2 must see no seed request"
            );
        }
        for on in ["1", "true", "TRUE", "True"] {
            let _flag = RealEnvVar::set(ENV_SEED_DEMO, Some(on));
            assert!(
                seed_requested_from_env(),
                "SOLID_SERVER_SEED_DEMO={on:?} must make Guard #2 see a seed request — \
                 without the `|| env_flag(ENV_SEED_DEMO)` disjunct the demo playground \
                 ACL can be written into a live http/embedded store"
            );
            // ...and that is what makes Guard #2 fire on a non-memory backend.
            for backend in ["http", "embedded"] {
                assert!(
                    reject_seed_on_nonmemory(seed_requested_from_env(), backend, false),
                    "SEED_DEMO={on:?} + PSS_SPARQ_BACKEND={backend} + no override must \
                     REFUSE to boot"
                );
            }
            // The documented escape hatch still works, and memory is always allowed.
            assert!(!reject_seed_on_nonmemory(
                seed_requested_from_env(),
                "embedded",
                true
            ));
            assert!(!reject_seed_on_nonmemory(
                seed_requested_from_env(),
                "memory",
                false
            ));
        }
        // Fail-closed direction: an OFF token must NOT arm Guard #2, so a fat-fingered
        // value cannot make an unrelated boot refuse.
        for off in ["0", "false", "no", "yes", "01", "tRuE", ""] {
            let _flag = RealEnvVar::set(ENV_SEED_DEMO, Some(off));
            assert!(
                !seed_requested_from_env(),
                "SOLID_SERVER_SEED_DEMO={off:?} is not an accepted truthy token"
            );
        }
    }

    #[test]
    fn replay_max_entries_unset_or_invalid_keeps_production_default() {
        // Absent / empty / non-numeric / zero ⇒ None ⇒ the caller keeps the production default. `0` is
        // None (NOT a zero-capacity store), so a typo cannot weaken replay protection.
        assert_eq!(parse_replay_max_entries(None), None);
        assert_eq!(parse_replay_max_entries(Some("".to_string())), None);
        assert_eq!(parse_replay_max_entries(Some("   ".to_string())), None);
        assert_eq!(parse_replay_max_entries(Some("abc".to_string())), None);
        assert_eq!(parse_replay_max_entries(Some("0".to_string())), None);
        assert_eq!(parse_replay_max_entries(Some("-5".to_string())), None);
    }

    #[test]
    fn replay_max_entries_can_only_raise_never_shrink() {
        // A value <= the production default is IGNORED (None ⇒ keep the default) — so a mis-set var
        // can never SHRINK the cap and degrade availability (roborev Medium). Only a value strictly
        // GREATER than the default takes effect. This is the load-bearing safety property.
        assert_eq!(parse_replay_max_entries(Some("1".to_string())), None);
        assert_eq!(parse_replay_max_entries(Some("99999".to_string())), None);
        assert_eq!(
            parse_replay_max_entries(Some(REPLAY_PRODUCTION_DEFAULT.to_string())),
            None,
            "exactly the default is a no-op (keep with_window)"
        );
        assert_eq!(
            parse_replay_max_entries(Some((REPLAY_PRODUCTION_DEFAULT + 1).to_string())),
            Some(REPLAY_PRODUCTION_DEFAULT + 1)
        );
        assert_eq!(
            parse_replay_max_entries(Some("  5000000  ".to_string())),
            Some(5_000_000)
        );
    }
}

}
