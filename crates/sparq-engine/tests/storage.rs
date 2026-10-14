//! sq-6vshe.9 — storage integration-test harness.
//! Consolidates compressed-build, dict, persistence, fork, and flat-read tests
//! into a single link unit.
//!
//! Integration-test crate roots resolve `mod foo;` relative to `tests/`, not
//! relative to a subdirectory, so explicit `#[path]` attributes are required.
#[path = "storage/compressed_build_differential.rs"]
mod compressed_build_differential;
#[path = "storage/dict_consolidation_differential.rs"]
mod dict_consolidation_differential;
#[path = "storage/dict_spill_differential.rs"]
mod dict_spill_differential;
#[path = "storage/flat_read_bench.rs"]
mod flat_read_bench;
#[path = "storage/fork_generations.rs"]
mod fork_generations;
#[path = "storage/named_graph_durable_clear_drop.rs"]
mod named_graph_durable_clear_drop;
#[path = "storage/named_graph_persistence.rs"]
mod named_graph_persistence;
