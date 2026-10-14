//! (sq-6vshe.4) Internal SPARQL-1.1-federated-query (`SERVICE`) sub-crate of
//! `sparq-engine`.
//!
//! **Unstable-internal.** This crate is an implementation detail of `sparq-engine`
//! (seam A2 of the staged facade split, RFC `research/engine-split-rfc.md` §4 Option A /
//! §7 Phase A2). It is `publish = false` and carries **no stability guarantee** of its
//! own: depend on `sparq-engine` (whose `service` feature forwards here and whose public
//! `with_service_egress_allow` / `SERVICE_EGRESS_REFUSED_MARKER` / `allowlist_entry_permits`
//! / … re-exports are unchanged by the split) instead of on this crate directly.
//!
//! It holds the `SERVICE <endpoint> { pattern }` client that used to live in
//! `sparq-engine`'s `service` module — the `service::Transport` HTTP seam and the
//! production ureq `service::HttpTransport`, the streaming SPARQL-Results JSON/XML
//! parsers, the bound-join `VALUES` batching, and the SSRF egress-policy allowlist. The
//! executor (`sparq-engine`) drives it per-SERVICE-call through the one allowlisted
//! `Box<dyn Transport>` install point — never on a per-row path (RFC §3.4-#, §D4).
//!
//! ## Feature gating (mirrors the original in-engine gating)
//!
//! The whole client lives behind the **`service`** feature, exactly as it did inside
//! `sparq-engine` (`#[cfg(feature = "service")] mod service;`). The facade pulls this crate
//! in as an **optional** dependency and enables this feature only when its own `service` is
//! on — so the default / wasm builds compile **zero** of it and pull in **zero** new
//! dependencies (byte-identical feature-off contract preserved). The blocking ureq client
//! (`HttpTransport`) is additionally `cfg(not(wasm32))`-gated, so it never enters a wasm
//! bundle even if the feature were forced on for a wasm target.

#![forbid(unsafe_code)] // sq-6vshe.4: the SERVICE client carries zero `unsafe` (moved verbatim from sparq-engine, which forbids it crate-wide).

// The entire SERVICE client is behind `service`, mirroring the gating this code had inside
// `sparq-engine` (`#[cfg(feature = "service")] mod service;`). When the feature is off (the
// default), the crate compiles to an empty library and pulls in no serde / serde_json /
// quick-xml / ureq. `sparq-engine` consumes THIS module as `sparq_engine_service::service::*`
// (the executor's `crate::service::*` references now point here), and re-exports the public
// fns verbatim as `sparq_engine::with_service_egress_allow`, `…::SERVICE_EGRESS_REFUSED_MARKER`,
// etc., so every existing public path is preserved.
#[cfg(feature = "service")]
pub mod service;
