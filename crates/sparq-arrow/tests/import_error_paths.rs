//! Fail-closed tests for malformed Arrow RDF-term input.
#![cfg(feature = "arrow")]

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, RecordBatch, StringArray, StructArray};
use arrow_schema::{DataType, Field, Schema};
use sparq_arrow::{
    from_record_batch, FIELD_DATATYPE, FIELD_DIRECTION, FIELD_KIND, FIELD_LANGUAGE, FIELD_VALUE,
};

fn string_field(name: &'static str, value: Option<&str>) -> (Arc<Field>, ArrayRef) {
    (
        Arc::new(Field::new(name, DataType::Utf8, true)),
        Arc::new(StringArray::from(vec![value])) as ArrayRef,
    )
}

fn batch_with_children(children: Vec<(Arc<Field>, ArrayRef)>) -> RecordBatch {
    let column = Arc::new(StructArray::from(children)) as ArrayRef;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "term",
        column.data_type().clone(),
        true,
    )]));
    RecordBatch::try_new(schema, vec![column]).unwrap()
}

/// Mutation witness: accepting four children or changing the pinned error
/// text makes this assertion fail.
#[test]
fn four_child_term_struct_is_rejected() {
    let batch = batch_with_children(vec![
        string_field(FIELD_KIND, Some("literal")),
        string_field(FIELD_VALUE, Some("value")),
        string_field(FIELD_DATATYPE, None),
        string_field(FIELD_LANGUAGE, Some("en")),
    ]);

    let error = from_record_batch(&batch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not have five RDF-term children"),
        "{error}"
    );
}

/// A bound struct slot must carry the required term kind instead of silently becoming
/// an unbound or guessed value.
#[test]
fn bound_term_with_null_kind_is_rejected() {
    let batch = batch_with_children(vec![
        string_field(FIELD_KIND, None),
        string_field(FIELD_VALUE, Some("value")),
        string_field(
            FIELD_DATATYPE,
            Some("http://www.w3.org/2001/XMLSchema#string"),
        ),
        string_field(FIELD_LANGUAGE, None),
        string_field(FIELD_DIRECTION, None),
    ]);

    let error = from_record_batch(&batch).unwrap_err();
    assert!(
        error.to_string().contains("bound term has null 'kind'"),
        "{error}"
    );
}

/// Literal metadata is exclusive: a datatype and language tag may not describe the
/// same RDF literal.
#[test]
fn literal_with_datatype_and_language_is_rejected() {
    let batch = batch_with_children(vec![
        string_field(FIELD_KIND, Some("literal")),
        string_field(FIELD_VALUE, Some("value")),
        string_field(
            FIELD_DATATYPE,
            Some("http://www.w3.org/2001/XMLSchema#string"),
        ),
        string_field(FIELD_LANGUAGE, Some("en")),
        string_field(FIELD_DIRECTION, None),
    ]);

    let error = from_record_batch(&batch).unwrap_err();
    assert!(
        error.to_string().contains(
            "literal requires either a datatype or a language tag; direction requires language"
        ),
        "{error}"
    );
}
