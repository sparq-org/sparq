//! [SONNET-4.6] sq-6vshe.9 — federation + isolated-feature harness.
//! Consolidates SERVICE-federation and txn/result-cache opt-in tests into a
//! single link unit.
//!
//! Integration-test crate roots resolve `mod foo;` relative to `tests/`, not
//! relative to a subdirectory, so explicit `#[path]` attributes are required.
#[path = "federation_features/service_bound_join.rs"]
mod service_bound_join;
#[path = "federation_features/service_federation.rs"]
mod service_federation;
#[path = "federation_features/service_stream_bounded.rs"]
mod service_stream_bounded;
// [OPUS-5] sq-lsp7k.2.2 — LOCAL rows-returning SERVICE handlers (`service-local`).
#[path = "federation_features/service_local.rs"]
mod service_local;
#[path = "federation_features/mvcc_txn.rs"]
mod mvcc_txn;
#[path = "federation_features/result_cache.rs"]
mod result_cache;
