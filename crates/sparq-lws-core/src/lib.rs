// AUTHORED-BY Claude Opus 4.8
#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
//! # solid-server-rs (EXPERIMENTAL)
//!
//! An **experimental, parallel-track** Rust reimplementation of a Solid/LDP server. It does **NOT**
//! replace and must **NEVER** touch the production TypeScript
//! [`prod-solid-server`](https://github.com/jeswr/prod-solid-server) (the live, supported server).
//!
//! ## Architecture (the maintainer's directive + the Rust-migration spike)
//! - **SPARQ is authoritative** for RDF data, metadata, containment, AND access-control evaluation —
//!   queried over its HTTP API (the [`store::SparqClient`] seam).
//! - **`object_store`/S3 is backup-only** for resource bytes (the [`store::BlobStore`] seam).
//! - **DPoP/Solid-OIDC verification is delegated** to the standalone
//!   [`solid-oidc-verifier`](https://github.com/jeswr/solid-oidc-verifier) crate (a git dependency).
//!   Auth is **not** reimplemented here. See [`auth`].
//!
//! ## Specification estate
//! What this crate is actually pinned to, and what is still unresolved (issue #4971). [OPUS-5]
//! - **DPoP-SK** (<https://jeswr.github.io/dpop-sk-spec/>) — normative for the `pop::sk`
//!   proof-of-possession tier. An EXPERIMENTAL profile, **not** a ratified standard, so treat its
//!   guarantees as unreviewed. Its execution-verified Appendix-A worked example is asserted
//!   byte-for-byte by the `pop::sk` tests, and the RFC 5705 §4 exporter label
//!   (`EXPERIMENTAL-dpop-sk-v1`) is pinned against the spec string.
//! - **Solid-OIDC verification** — delegated (see Architecture above), not reimplemented here: the
//!   `solid-oidc-verifier` git dependency is `rev`-pinned in this crate's `Cargo.toml`.
//! - **`lws-spec`, `lws-ucs`** — **UNRESOLVED**. Location and content unknown; outside this note
//!   neither name is referenced by any source file, dependency, or test vector in this repository,
//!   and no shipped behaviour here is known to depend on either.
//!   Promoting them out of UNRESOLVED takes what DPoP-SK already has — a maintainer-confirmed
//!   reference **plus** an executed spec vector, not a prose citation. Until that pointer exists do
//!   not cite a URL for them: a guessed one would read as a normative claim this crate cannot back.
//!
//! ## Vertical slice (this crate)
//! A coherent, compiling slice with clean trait seams + tests:
//! - an axum server skeleton ([`app`]) that boots,
//! - DPoP-bound auth middleware ([`auth`]) over the verifier,
//! - the LDP verb surface ([`ldp`]) through a [`store::Store`] trait: GET/HEAD (with `Accept`
//!   content negotiation + `Range`), PUT/POST/DELETE/PATCH (conditional `If-Match`/`If-None-Match`),
//!   POST `Slug`-honouring child creation, the empty-container DELETE refusal, and the Solid N3-Patch
//!   engine (`text/n3`, insert/delete plus the `solid:where` variable solver),
//! - LDP target/URL parsing + Turtle/JSON-LD content handling ([`ldp::target`], [`ldp::content`]).
//!
//! Web Access Control authorization is implemented locally in [`authz`] (a semantic port of
//! prod-solid-server `src/authz/`): per-resource `.acl` evaluation with own-ACL-(`acl:accessTo`)-else
//! -nearest-ancestor-(`acl:default`) resolution, the four modes, the 401-vs-403 split, and the
//! `WAC-Allow` header. It reads `.acl` documents through the [`store::Store`] seam; when the SPARQ
//! access-control design lands the per-resource decision can move behind the same seam.
//!
//! Solid Notifications (WebSocketChannel2023) are implemented as a net-new, isolated `notifications`
//! module: an in-process subscription registry + AS2.0 notification builder, an axum WebSocket receive
//! endpoint, a subscribe endpoint, and discovery (storage description + `Link` rels). The LDP write
//! path makes a single emit call after a successful mutation. Everything else network-facing (the live
//! SPARQ HTTP client, live JWKS) and the parts of the Solid surface that need designs not yet written
//! (per-resource authorization of a subscription, the reconciler, multipart Range, `acl:agentGroup`
//! resolution) are clearly marked `M2-next:` seams. PATCH supports both the Solid N3 Patch and the
//! `application/sparql-update` INSERT/DELETE-DATA subset. The Solid conformance suite passes **41/41**
//! (Protocol 25/25 + WAC 16/16) — see `conformance/SCORE.md`. The default impls used here are
//! in-memory test doubles.

pub mod acl_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod app;
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
#[path = "app_wasm.rs"]
pub mod app;
// [GPT-5.6] Compile the portable implementation in native unit tests so the native coverage
// ratchet directly exercises the exact wasm router source without changing native runtime exports.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "app_wasm.rs"]
mod app_wasm_native_tests;
#[cfg(not(target_arch = "wasm32"))]
pub mod auth;
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
#[path = "auth_wasm.rs"]
pub mod auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod auth_cache;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "auth_wasm.rs"]
mod auth_wasm_native_tests;
pub mod authz;
pub mod body_limit;
mod clock;
pub mod error;
/// Provider-issued WebIDs hosted OUTSIDE the pod — the identity host (the RSS adaptation of PSS
/// `decisions/0020`; design in `research/lws-design-records.md` §4). The id-docs live in a reserved
/// namespace the LDP surface refuses outright (no `.acl` can ever exist ⇒ no WAC grant can ever
/// apply), served GET/HEAD-only by a Host-keyed route with no authorization.
pub mod identity;
pub mod ldp;
#[cfg(not(target_arch = "wasm32"))]
pub mod nodelay;
#[cfg(not(target_arch = "wasm32"))]
pub mod notifications;
#[cfg(not(target_arch = "wasm32"))]
pub mod overload;
/// Tiered proof-of-possession (RFC 8705 cert-bound tokens, later DPoP-SK) — the negotiated, opt-in
/// fast paths that keep DPoP as the mandatory Solid-OIDC baseline. See
/// `research/lws-design-records.md` §7. T1a lands the confirmation dispatch + cert-bound
/// verification core; the acceptor + verifier wiring are tracked follow-ups.
#[cfg(not(target_arch = "wasm32"))]
pub mod pop;
#[cfg(not(target_arch = "wasm32"))]
pub mod rate_limit;
/// The distributed (shared) Redis-backed DPoP-`jti` replay store — the horizontal-scaling enabler.
/// Behind the opt-in `redis-replay` feature (OFF by default → byte-identical default build/conformance).
#[cfg(all(feature = "redis-replay", not(target_arch = "wasm32")))]
pub mod redis_replay;
pub mod seed;
#[cfg(feature = "sparql-endpoint")]
mod sparql_endpoint;
pub mod store;
#[cfg(not(target_arch = "wasm32"))]
pub mod tls;
#[cfg(not(target_arch = "wasm32"))]
pub mod transport;

#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "wasm")
))]
pub use app::{build_router, AppState};
pub use error::{ServerError, ServerResult};
