//! [SONNET-4.6] sq-6vshe.9 — opt-in feature differential harness.
//! Consolidates semi-join, Yannakakis, vectorized, window, and ZK-trace
//! differential tests into a single link unit.
//!
//! Integration-test crate roots resolve `mod foo;` relative to `tests/`, not
//! relative to a subdirectory, so explicit `#[path]` attributes are required.
#[path = "differentials/semijoin_differential.rs"]
mod semijoin_differential;
#[path = "differentials/yannakakis_differential.rs"]
mod yannakakis_differential;
#[path = "differentials/vectorized_byte_identity.rs"]
mod vectorized_byte_identity;
#[path = "differentials/vectorized_exec_differential.rs"]
mod vectorized_exec_differential;
// [SONNET-4.6] (sq-y5ew5) Hybrid tri-mask FILTER kernel acceptance tests (T1-T5).
#[path = "differentials/chunk_select_tri_mask.rs"]
mod chunk_select_tri_mask;
#[path = "differentials/window_inline_over.rs"]
mod window_inline_over;
// [FABLE-5] (sq-7d3dj.11) Flat compiled scalar-expression programs — the ON==OFF row-identity
// differential plus the spec-pinned 3VL / type-error / term-identity expectations.
#[path = "differentials/expr_program_differential.rs"]
mod expr_program_differential;
#[path = "differentials/zk_trace_differential.rs"]
mod zk_trace_differential;
#[path = "differentials/zk_trace_operators.rs"]
mod zk_trace_operators;
