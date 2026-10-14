//! Integration tests for Parquet serialization of the RDF-term Arrow projection.
#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Triple, Variable};
use parquet::arrow::ArrowWriter;
use sparq_arrow::{
    from_parquet_bytes, parquet_row_count_from_bytes, parquet_variables_from_bytes,
    to_parquet_bytes,
};
use sparq_engine::QueryResult;

fn assert_identity(result: &QueryResult) {
    let bytes = to_parquet_bytes(result).unwrap();
    let restored = from_parquet_bytes(&bytes).unwrap();
    assert_eq!(restored.vars, result.vars);
    assert_eq!(restored.rows, result.rows);
}

/// Mutation witness: removing any exporter/importer term-kind arm, treating
/// unbound as a value, or collapsing the empty literal changes at least one exact row.
#[test]
fn every_term_kind_empty_literal_and_unbound_round_trip_row_for_row() {
    let result = QueryResult {
        vars: vec![Variable::new("term").unwrap()],
        rows: vec![
            vec![Some(Term::NamedNode(
                NamedNode::new("https://example.test/resource").unwrap(),
            ))],
            vec![Some(Term::BlankNode(BlankNode::new("blank-1").unwrap()))],
            vec![Some(Term::Literal(Literal::new_simple_literal("plain")))],
            vec![Some(Term::Literal(Literal::new_simple_literal("")))],
            vec![Some(Term::Literal(Literal::new_typed_literal(
                "42",
                NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
            )))],
            vec![Some(Term::Literal(
                Literal::new_language_tagged_literal("hello", "en").unwrap(),
            ))],
            vec![Some(Term::Literal(
                Literal::new_directional_language_tagged_literal("مرحبا", "ar", BaseDirection::Rtl)
                    .unwrap(),
            ))],
            vec![Some(Term::Triple(Box::new(Triple::new(
                NamedNode::new("https://example.test/s").unwrap(),
                NamedNode::new("https://example.test/p").unwrap(),
                Literal::new_simple_literal("object"),
            ))))],
            vec![None],
        ],
    };

    assert_identity(&result);
}

/// Value-pinned mutation witness: changing either expected name fails while
/// the encoded rows remain untouched.
#[test]
fn schema_only_reader_recovers_nonempty_result_variables() {
    let result = QueryResult {
        vars: vec![Variable::new("s").unwrap(), Variable::new("o").unwrap()],
        rows: vec![vec![
            Some(Term::NamedNode(
                NamedNode::new("https://example.test/s").unwrap(),
            )),
            Some(Term::Literal(Literal::new_simple_literal("object"))),
        ]],
    };
    let bytes = to_parquet_bytes(&result).unwrap();

    let variables = parquet_variables_from_bytes(&bytes).unwrap();
    assert_eq!(variables, result.vars);
    assert_eq!(
        variables.iter().map(Variable::as_str).collect::<Vec<_>>(),
        ["s", "o"]
    );
}

/// Value-pinned mutation witness: the two variables and three rows make a
/// column-count implementation return 2, while the all-unbound row exercises nullability.
#[test]
fn metadata_only_reader_recovers_exact_row_count() {
    let result = QueryResult {
        vars: vec![Variable::new("s").unwrap(), Variable::new("o").unwrap()],
        rows: vec![
            vec![
                Some(Term::NamedNode(
                    NamedNode::new("https://example.test/s").unwrap(),
                )),
                Some(Term::Literal(Literal::new_simple_literal("object"))),
            ],
            vec![None, None],
            vec![
                Some(Term::BlankNode(BlankNode::new("blank-2").unwrap())),
                None,
            ],
        ],
    };
    let bytes = to_parquet_bytes(&result).unwrap();

    assert!(matches!(parquet_row_count_from_bytes(&bytes), Ok(3)));
    let restored = from_parquet_bytes(&bytes).unwrap();
    assert_eq!(
        parquet_row_count_from_bytes(&bytes).unwrap(),
        restored.rows.len()
    );
}

#[test]
fn empty_result_preserves_variable_schema_and_zero_rows() {
    let result = QueryResult {
        vars: vec![
            Variable::new("subject").unwrap(),
            Variable::new("object").unwrap(),
        ],
        rows: Vec::new(),
    };

    let bytes = to_parquet_bytes(&result).unwrap();
    assert_eq!(parquet_variables_from_bytes(&bytes).unwrap(), result.vars);
    assert_eq!(parquet_row_count_from_bytes(&bytes).unwrap(), 0);
    assert_identity(&result);
}

#[test]
fn parquet_with_deviating_arrow_schema_is_rejected() {
    let schema = Arc::new(Schema::new(vec![Field::new("term", DataType::Utf8, true)]));
    let column = Arc::new(StringArray::from(vec![Some(
        "https://example.test/not-a-term-struct",
    )])) as ArrayRef;
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![column]).unwrap();
    let mut writer = ArrowWriter::try_new(Vec::new(), schema, None).unwrap();
    writer.write(&batch).unwrap();
    let bytes = writer.into_inner().unwrap();

    let schema_error = parquet_variables_from_bytes(&bytes).unwrap_err();
    assert!(
        schema_error
            .to_string()
            .contains("does not use the nullable RDF-term struct schema"),
        "{schema_error}"
    );

    let error = from_parquet_bytes(&bytes).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not use the nullable RDF-term struct schema"),
        "{error}"
    );
}

#[test]
fn malformed_parquet_is_rejected() {
    let error = from_parquet_bytes(b"not parquet").unwrap_err();
    assert!(
        error.to_string().contains("Parquet import failed"),
        "{error}"
    );
}
