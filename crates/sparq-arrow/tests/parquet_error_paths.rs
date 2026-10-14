//! Fail-closed tests for corrupted and truncated Parquet input.
#![cfg(feature = "parquet")]

use oxrdf::{Literal, Term, Variable};
use sparq_arrow::{
    from_parquet_bytes, parquet_row_count_from_bytes, parquet_variables_from_bytes,
    to_parquet_bytes,
};
use sparq_engine::QueryResult;

fn assert_all_readers_reject(bytes: &[u8]) {
    let import_error = from_parquet_bytes(bytes).unwrap_err();
    let variables_error = parquet_variables_from_bytes(bytes).unwrap_err();
    let row_count_error = parquet_row_count_from_bytes(bytes).unwrap_err();

    for error in [&import_error, &variables_error, &row_count_error] {
        assert!(
            error.to_string().contains("Parquet import failed"),
            "{error}"
        );
    }
}

/// Mutation witness: any reader accepting arbitrary bytes, or changing the
/// pinned error class, makes this assertion fail.
#[test]
fn garbage_bytes_are_rejected_by_all_parquet_readers() {
    assert_all_readers_reject(b"not parquet");
}

#[test]
fn truncated_valid_parquet_is_rejected_by_all_parquet_readers() {
    let result = QueryResult {
        vars: vec![Variable::new("term").unwrap()],
        rows: vec![vec![Some(Term::Literal(Literal::new_simple_literal(
            "value",
        )))]],
    };
    let bytes = to_parquet_bytes(&result).unwrap();
    assert!(bytes.len() > 8);

    assert_all_readers_reject(&bytes[..8]);
}
