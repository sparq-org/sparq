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
pub mod ops_core_statement;
pub mod ops_core_suite;
mod ops_core_tests;

pub mod expr_term_contract;
pub mod expr_core;
pub mod expr_compare;
pub mod expr_arith;
mod expr_tests;
// The pass-two function files are retained but are not wired into this package yet.
pub mod expr_to_term;
pub mod expr_open;
mod expr_binding_tests;
mod expr_target_tests;
mod expr_open_tests;

pub mod ops_core_profile;
