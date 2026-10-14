//! sq-6vshe.9 — query-feature integration-test harness.
//! Consolidates dataset-view, DP-planner, parse-depth, prepared-query, and
//! RDF-star-write tests into a single link unit.
//!
//! Integration-test crate roots resolve `mod foo;` relative to `tests/`, not
//! relative to a subdirectory, so explicit `#[path]` attributes are required.
#[path = "query_features/dataset_view.rs"]
mod dataset_view;
#[path = "query_features/dp_planner_differential.rs"]
mod dp_planner_differential;
#[path = "query_features/parse_recursion_depth.rs"]
mod parse_recursion_depth;
#[path = "query_features/prepared.rs"]
mod prepared;
#[path = "query_features/rdfstar_write.rs"]
mod rdfstar_write;
