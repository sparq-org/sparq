// Research-grade and externally unaudited.
pub mod suite_constants;
pub mod suite;
pub mod term;
mod term_key;
mod term_float;

mod suite_tests;
pub mod suite_entry;

pub mod table;
pub mod ops_core;
pub mod ops_core_expr;
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
pub mod ops_closed_path;
mod ops_closed_path_tests;
