// Research-grade and externally unaudited.
pub mod suite_constants;
pub mod suite;
pub mod term;
mod term_key;
mod term_float;

mod suite_tests;
pub mod suite_entry;

mod term_bytes;
mod term_ids;
pub mod suite_metadata;
mod suite_metadata_tests;
mod suite_entry_tests;

pub mod table;
pub mod ops_core;
pub mod ops_core_expr;
pub mod ops_core_reads;
pub mod ops_core_statement;
pub mod ops_core_suite;
mod ops_core_tests;
pub mod ops_closed_common;
pub mod ops_closed_negation;
pub mod ops_closed_union;
pub mod ops_closed_extend;
pub mod ops_closed_substitute;
mod ops_closed_tests;
pub mod ops_closed_graph;
pub mod ops_closed_group;
pub mod ops_closed_order;
mod ops_closed_group_tests;
mod ops_closed_order_tests;

pub mod ops_closed_numeric;
pub mod ops_closed_group_plan;
pub mod ops_closed_dataset;
pub mod ops_closed_arith;
mod ops_closed_plan_tests;
pub mod ops_closed_opening;
pub mod ops_closed_issuer_standin;
mod ops_core_scan_tests;

pub mod expr_term_contract;
pub mod expr_core;
pub mod expr_compare;
pub mod expr_arith;
mod expr_tests;
pub mod expr_to_term;
pub mod expr_open;
mod expr_binding_tests;
mod expr_target_tests;
mod expr_open_tests;

pub mod ops_core_profile;

mod ops_closed_bind_tests;

pub mod ops_closed_expr;
pub mod expr_language;
pub mod expr_term;
pub mod expr_date;
pub mod expr_numeric;
pub mod expr_construct;
mod expr_functions_tests;

pub mod ops_closed_join;
mod term_identity_tests;
pub mod envelope;

pub mod ops_core_envelope;
mod suite_mixed_tests;
