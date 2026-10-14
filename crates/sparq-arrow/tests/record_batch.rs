//! Integration tests for Arrow import/export (the `arrow` feature). The whole file is
//! `#![cfg(feature = "arrow")]`, so it ONLY runs with `--features arrow`; the default
//! (no-feature) build is exercised by the workspace default lane (the field-name
//! constants + the schema docs are all that compiles there).
//!
//! These exercise the REAL path end to end: an in-memory `Graph`, the real
//! `sparq_engine::query`, then `to_record_batch`, asserting columns/rows/types and the
//! load-bearing term-mapping invariants (unbound → null struct slot; literal datatype /
//! language / direction land in the right fields).
#![cfg(feature = "arrow")]

use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray, StructArray};
use arrow_schema::{DataType, Field, Schema};
use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Triple, Variable};
use sparq_arrow::{
    from_record_batch, term_schema, term_struct_type, to_record_batch, FIELD_DATATYPE,
    FIELD_DIRECTION, FIELD_KIND, FIELD_LANGUAGE, FIELD_VALUE,
};
use sparq_core::Graph;
use sparq_engine::{query, QueryResult};

/// Pull the five child string arrays of one struct column.
fn fields(
    col: &StructArray,
) -> (
    &StringArray,
    &StringArray,
    &StringArray,
    &StringArray,
    &StringArray,
) {
    (
        col.column_by_name(FIELD_KIND).unwrap().as_string::<i32>(),
        col.column_by_name(FIELD_VALUE).unwrap().as_string::<i32>(),
        col.column_by_name(FIELD_DATATYPE)
            .unwrap()
            .as_string::<i32>(),
        col.column_by_name(FIELD_LANGUAGE)
            .unwrap()
            .as_string::<i32>(),
        col.column_by_name(FIELD_DIRECTION)
            .unwrap()
            .as_string::<i32>(),
    )
}

const DATA: &str = r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
    ex:a ex:label "Alice"@en ; ex:age "30"^^xsd:integer ; ex:friend ex:b .
    ex:b ex:label "Bob" .
"#;

/// A real SELECT over a real graph maps to a batch with the right shape and the
/// per-term-kind field decomposition.
#[test]
fn select_maps_to_record_batch_with_correct_columns_rows_types() {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    // Order by ?s so the row order is deterministic for the assertions.
    let result = query(
        &graph,
        "PREFIX ex: <http://ex/> SELECT ?s ?label WHERE { ?s ex:label ?label } ORDER BY ?s ?label",
    )
    .unwrap();

    let batch = to_record_batch(&result).unwrap();

    // Shape: one column per SELECT var, one row per solution (a=Alice@en, b=Bob).
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.schema().field(0).name(), "s");
    assert_eq!(batch.schema().field(1).name(), "label");

    // Both columns are the term struct type.
    for i in 0..2 {
        assert!(matches!(
            batch.schema().field(i).data_type(),
            DataType::Struct(_)
        ));
    }

    // Column ?s: both rows are IRIs.
    let s = batch.column(0).as_struct();
    let (kind, value, datatype, language, direction) = fields(s);
    assert_eq!(kind.value(0), "uri");
    assert_eq!(value.value(0), "http://ex/a");
    assert_eq!(value.value(1), "http://ex/b");
    assert!(datatype.is_null(0) && language.is_null(0) && direction.is_null(0));

    // Column ?label: row 0 is a language-tagged literal, row 1 a plain (xsd:string) one.
    let label = batch.column(1).as_struct();
    let (kind, value, datatype, language, direction) = fields(label);
    assert_eq!(kind.value(0), "literal");
    assert_eq!(value.value(0), "Alice");
    assert!(
        datatype.is_null(0),
        "lang-tagged literal carries no datatype field"
    );
    assert_eq!(language.value(0), "en");
    assert!(direction.is_null(0), "no base direction on this literal");
    // Plain literal: xsd:string is written EXPLICITLY (never elided), and no language.
    assert_eq!(value.value(1), "Bob");
    assert_eq!(datatype.value(1), "http://www.w3.org/2001/XMLSchema#string");
    assert!(language.is_null(1) && direction.is_null(1));
}

/// A typed (numeric) literal stays a string value + a datatype IRI — the documented
/// v1 boundary (no numeric narrowing). This pins that behaviour.
#[test]
fn typed_literal_keeps_lexical_value_and_datatype_iri() {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let result = query(
        &graph,
        "PREFIX ex: <http://ex/> SELECT ?age WHERE { ?s ex:age ?age }",
    )
    .unwrap();
    let batch = to_record_batch(&result).unwrap();
    let col = batch.column(0).as_struct();
    let (kind, value, datatype, _lang, _dir) = fields(col);
    assert_eq!(kind.value(0), "literal");
    assert_eq!(value.value(0), "30");
    assert_eq!(
        datatype.value(0),
        "http://www.w3.org/2001/XMLSchema#integer"
    );
}

/// OPTIONAL that leaves a variable unbound → a NULL struct slot (the load-bearing
/// answer-safety invariant: unbound must be distinguishable from a bound empty string).
#[test]
fn unbound_optional_is_a_null_struct_slot() {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    // ex:b has no ex:friend, so ?friend is unbound for it.
    let result = query(
        &graph,
        "PREFIX ex: <http://ex/> SELECT ?s ?friend WHERE { ?s ex:label ?l . OPTIONAL { ?s ex:friend ?friend } } ORDER BY ?s",
    )
    .unwrap();
    let batch = to_record_batch(&result).unwrap();
    assert_eq!(batch.num_rows(), 2);

    let friend = batch.column(1).as_struct();
    // Row 0 (ex:a) is bound to ex:b; row 1 (ex:b) is unbound → null struct.
    assert!(friend.is_valid(0), "ex:a has a friend");
    assert_eq!(
        friend
            .column_by_name(FIELD_VALUE)
            .unwrap()
            .as_string::<i32>()
            .value(0),
        "http://ex/b"
    );
    assert!(friend.is_null(1), "ex:b has no friend → null struct slot");
}

/// An empty result still produces a valid batch with the right schema and zero rows.
#[test]
fn empty_result_is_valid_zero_row_batch() {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let result = query(
        &graph,
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:nothere ?o }",
    )
    .unwrap();
    let batch = to_record_batch(&result).unwrap();
    assert_eq!(batch.num_columns(), 1);
    assert_eq!(batch.num_rows(), 0);
    assert_eq!(batch.schema().field(0).name(), "s");
}

/// `term_schema` matches the batch's schema (build-schema-without-rows path).
#[test]
fn term_schema_matches_batch_schema() {
    let vars = vec![Variable::new("s").unwrap(), Variable::new("o").unwrap()];
    let schema = term_schema(&vars).unwrap();
    let result = QueryResult {
        vars: vars.clone(),
        rows: vec![vec![
            Some(Term::NamedNode(NamedNode::new("http://ex/a").unwrap())),
            Some(Term::NamedNode(NamedNode::new("http://ex/b").unwrap())),
        ]],
    };
    let batch = to_record_batch(&result).unwrap();
    assert_eq!(batch.schema().as_ref(), &schema);
}

/// A duplicate SELECT variable name is a typed error, not a panic and not a silent
/// column collision.
#[test]
fn duplicate_variable_is_an_error() {
    let v = Variable::new("x").unwrap();
    let result = QueryResult {
        vars: vec![v.clone(), v],
        rows: vec![],
    };
    let err = to_record_batch(&result).unwrap_err();
    assert!(
        format!("{}", err).contains("duplicate SELECT variable 'x'"),
        "{}",
        err
    );
}

/// Mutation witness for the inverse mapping: removing any term-kind arm or
/// metadata reconstruction makes this exact row-for-row identity assertion fail.
#[test]
fn all_term_kinds_round_trip_to_identical_query_result() {
    let vars = vec![Variable::new("term").unwrap()];
    let result = QueryResult {
        vars: vars.clone(),
        rows: vec![
            vec![Some(Term::NamedNode(
                NamedNode::new("http://example.com/resource").unwrap(),
            ))],
            vec![Some(Term::BlankNode(BlankNode::new("blank-1").unwrap()))],
            vec![Some(Term::Literal(Literal::new_simple_literal("plain")))],
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
                NamedNode::new("http://example.com/s").unwrap(),
                NamedNode::new("http://example.com/p").unwrap(),
                Literal::new_simple_literal("object"),
            ))))],
            vec![None],
        ],
    };

    let batch = to_record_batch(&result).unwrap();
    let restored = from_record_batch(&batch).unwrap();

    assert_eq!(restored.vars, result.vars);
    assert_eq!(restored.rows, result.rows);
    assert_eq!(restored.vars, vars);
}

/// A kind outside the four-value contract is rejected, never guessed as a term.
#[test]
fn unknown_kind_is_rejected() {
    let vars = vec![Variable::new("term").unwrap()];
    let children: Vec<(Arc<Field>, ArrayRef)> = vec![
        (
            Arc::new(Field::new(FIELD_KIND, DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![Some("mystery")])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(FIELD_VALUE, DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![Some("value")])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(FIELD_DATATYPE, DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(FIELD_LANGUAGE, DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(FIELD_DIRECTION, DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
        ),
    ];
    let column = Arc::new(StructArray::from(children)) as ArrayRef;
    let batch = RecordBatch::try_new(Arc::new(term_schema(&vars).unwrap()), vec![column]).unwrap();

    let error = from_record_batch(&batch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown RDF term kind 'mystery'"),
        "{error}"
    );
}

/// A top-level column outside the term-struct schema is rejected before any cell read.
#[test]
fn invalid_term_schema_is_rejected() {
    let schema = arrow_schema::Schema::new(vec![Field::new("term", DataType::Utf8, true)]);
    let column = Arc::new(StringArray::from(vec![Some("http://example.com/resource")])) as ArrayRef;
    let batch = RecordBatch::try_new(Arc::new(schema), vec![column]).unwrap();

    let error = from_record_batch(&batch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not use the nullable RDF-term struct schema"),
        "{error}"
    );
}

/// Duplicate field names are rejected by the shared schema-to-variable validator.
#[test]
fn duplicate_import_variable_is_rejected() {
    let field = Field::new("term", term_struct_type(), true);
    let schema = Arc::new(Schema::new(vec![field.clone(), field]));
    let batch = RecordBatch::new_empty(schema);

    let error = from_record_batch(&batch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate SELECT variable 'term' in Arrow schema"),
        "{error}"
    );
}
