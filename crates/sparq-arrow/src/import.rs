//! The `arrow`-feature-gated import: Arrow `RecordBatch` → `QueryResult`.

use arrow_array::{Array, RecordBatch, StringArray, StructArray};
use arrow_schema::{DataType, Schema};
use oxrdf::{BaseDirection, BlankNode, Literal, NamedNode, Term, Variable};
use sparq_engine::QueryResult;

use crate::{term_struct_type, RDF_TERM_FIELDS};

/// Failure converting an Arrow [`RecordBatch`] into a [`QueryResult`].
///
/// The importer rejects any schema or cell that does not follow this crate's RDF-term
/// struct contract. The error is intentionally opaque apart from its message so future
/// validation can become stricter without adding semver-visible enum variants.
#[derive(Debug)]
pub struct ArrowError {
    message: String,
}

impl ArrowError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ArrowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ArrowError {}

// One schema-validation path serves RecordBatch, IPC, and Parquet imports so
// schema-only readers cannot drift from the full row decoders.
pub(crate) fn variables_from_schema(schema: &Schema) -> Result<Vec<Variable>, ArrowError> {
    let mut vars = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        let variable = Variable::new(field.name().to_string()).map_err(|error| {
            ArrowError::invalid(format!(
                "invalid SELECT variable in Arrow field '{}': {}",
                field.name(),
                error
            ))
        })?;
        if vars.iter().any(|existing: &Variable| existing == &variable) {
            return Err(ArrowError::invalid(format!(
                "duplicate SELECT variable '{}' in Arrow schema",
                field.name()
            )));
        }
        // Report malformed struct arity explicitly before the exact-schema
        // check so safely constructed malformed batches fail closed without a cell read.
        if matches!(
            field.data_type(),
            DataType::Struct(fields) if fields.len() != RDF_TERM_FIELDS.len()
        ) {
            return Err(ArrowError::invalid(format!(
                "Arrow field '{}' does not have five RDF-term children",
                field.name()
            )));
        }
        if !field.is_nullable() || field.data_type() != &term_struct_type() {
            return Err(ArrowError::invalid(format!(
                "Arrow field '{}' does not use the nullable RDF-term struct schema",
                field.name()
            )));
        }
        vars.push(variable);
    }
    Ok(vars)
}

/// Convert an Arrow [`RecordBatch`] using this crate's RDF-term struct projection back
/// into a SPARQL [`QueryResult`].
///
/// Variables are reconstructed from the top-level field names. Rows and variables keep
/// their batch order, and a null struct slot becomes an unbound result cell. All other
/// cells must contain exactly the five nullable UTF-8 children documented at the crate
/// root.
///
/// # Errors
///
/// Returns [`ArrowError`] if a variable name, schema, term kind, RDF lexical component,
/// or combination of literal metadata violates the projection contract.
///
/// # Example
///
/// ```
/// # use oxrdf::{NamedNode, Term, Variable};
/// # use sparq_engine::QueryResult;
/// let result = QueryResult {
///     vars: vec![Variable::new("s").unwrap()],
///     rows: vec![vec![Some(Term::NamedNode(
///         NamedNode::new("http://example.com/s").unwrap(),
///     ))]],
/// };
/// let batch = sparq_arrow::to_record_batch(&result).unwrap();
/// let restored = sparq_arrow::from_record_batch(&batch).unwrap();
/// assert_eq!(restored.vars, result.vars);
/// assert_eq!(restored.rows, result.rows);
/// ```
pub fn from_record_batch(batch: &RecordBatch) -> Result<QueryResult, ArrowError> {
    // Validate before reading: Arrow permits arbitrary struct schemas, while
    // this inverse is sound only for the exact lossless projection emitted by this crate.
    let schema = batch.schema();
    let vars = variables_from_schema(schema.as_ref())?;
    let mut columns = Vec::with_capacity(schema.fields().len());

    for (index, field) in schema.fields().iter().enumerate() {
        let column = batch
            .column(index)
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| {
                ArrowError::invalid(format!(
                    "Arrow field '{}' is not backed by a StructArray",
                    field.name()
                ))
            })?;
        columns.push(column);
    }

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let mut row = Vec::with_capacity(columns.len());
        for (column_index, column) in columns.iter().enumerate() {
            row.push(read_cell(
                column,
                row_index,
                schema.field(column_index).name(),
            )?);
        }
        rows.push(row);
    }

    Ok(QueryResult { vars, rows })
}

fn read_cell(
    column: &StructArray,
    row: usize,
    column_name: &str,
) -> Result<Option<Term>, ArrowError> {
    let children = string_children(column, column_name)?;
    if column.is_null(row) {
        if children.iter().any(|child| child.is_valid(row)) {
            return Err(cell_error(
                column_name,
                row,
                "an unbound struct slot must have five null children",
            ));
        }
        return Ok(None);
    }

    let kind = required(children[0], row, column_name, "kind")?;
    let value = required(children[1], row, column_name, "value")?;
    let datatype = optional(children[2], row);
    let language = optional(children[3], row);
    let direction = optional(children[4], row);

    Ok(Some(term_from_parts(
        kind,
        value,
        datatype,
        language,
        direction,
        column_name,
        row,
    )?))
}

// The single bound-term decoder: RecordBatch struct cells and the flattened
// CSV fields both reconstruct terms through this one path, so the two decoders cannot
// drift on kind dispatch, metadata rejection, or literal assembly.
pub(crate) fn term_from_parts(
    kind: &str,
    value: &str,
    datatype: Option<&str>,
    language: Option<&str>,
    direction: Option<&str>,
    column_name: &str,
    row: usize,
) -> Result<Term, ArrowError> {
    match kind {
        "uri" => {
            require_no_metadata(column_name, row, datatype, language, direction)?;
            Ok(Term::NamedNode(NamedNode::new(value).map_err(|error| {
                cell_error(column_name, row, format!("invalid IRI: {error}"))
            })?))
        }
        "bnode" => {
            require_no_metadata(column_name, row, datatype, language, direction)?;
            Ok(Term::BlankNode(BlankNode::new(value).map_err(|error| {
                cell_error(
                    column_name,
                    row,
                    format!("invalid blank-node label: {error}"),
                )
            })?))
        }
        "literal" => Ok(Term::Literal(read_literal(
            value,
            datatype,
            language,
            direction,
            column_name,
            row,
        )?)),
        "triple" => {
            require_no_metadata(column_name, row, datatype, language, direction)?;
            match value.parse::<Term>().map_err(|error| {
                cell_error(
                    column_name,
                    row,
                    format!("invalid N-Triples triple term: {error}"),
                )
            })? {
                term @ Term::Triple(_) => Ok(term),
                _ => Err(cell_error(
                    column_name,
                    row,
                    "kind 'triple' requires an N-Triples triple term",
                )),
            }
        }
        other => Err(cell_error(
            column_name,
            row,
            format!("unknown RDF term kind '{other}'"),
        )),
    }
}

fn string_children<'a>(
    column: &'a StructArray,
    column_name: &str,
) -> Result<[&'a StringArray; 5], ArrowError> {
    let mut children = Vec::with_capacity(RDF_TERM_FIELDS.len());
    for (index, field_name) in RDF_TERM_FIELDS.iter().enumerate() {
        let child = column
            .column(index)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                ArrowError::invalid(format!(
                    "Arrow field '{column_name}.{field_name}' is not backed by a UTF-8 array"
                ))
            })?;
        children.push(child);
    }
    children.try_into().map_err(|_| {
        ArrowError::invalid(format!(
            "Arrow field '{column_name}' does not have five RDF-term children"
        ))
    })
}

fn required<'a>(
    child: &'a StringArray,
    row: usize,
    column: &str,
    field: &str,
) -> Result<&'a str, ArrowError> {
    if child.is_null(row) {
        return Err(cell_error(
            column,
            row,
            format!("bound term has null '{field}'"),
        ));
    }
    Ok(child.value(row))
}

fn optional(child: &StringArray, row: usize) -> Option<&str> {
    child.is_valid(row).then(|| child.value(row))
}

fn require_no_metadata(
    column: &str,
    row: usize,
    datatype: Option<&str>,
    language: Option<&str>,
    direction: Option<&str>,
) -> Result<(), ArrowError> {
    if datatype.is_some() || language.is_some() || direction.is_some() {
        return Err(cell_error(
            column,
            row,
            "non-literal term must not have datatype, language, or direction metadata",
        ));
    }
    Ok(())
}

fn read_literal(
    value: &str,
    datatype: Option<&str>,
    language: Option<&str>,
    direction: Option<&str>,
    column: &str,
    row: usize,
) -> Result<Literal, ArrowError> {
    match (datatype, language, direction) {
        (Some(datatype), None, None) => {
            let datatype = NamedNode::new(datatype).map_err(|error| {
                cell_error(column, row, format!("invalid datatype IRI: {error}"))
            })?;
            Ok(Literal::new_typed_literal(value, datatype))
        }
        (None, Some(language), None) => Literal::new_language_tagged_literal(value, language)
            .map_err(|error| cell_error(column, row, format!("invalid language tag: {error}"))),
        (None, Some(language), Some(direction)) => {
            let direction = match direction {
                "ltr" => BaseDirection::Ltr,
                "rtl" => BaseDirection::Rtl,
                other => {
                    return Err(cell_error(
                        column,
                        row,
                        format!("invalid base direction '{other}'"),
                    ));
                }
            };
            Literal::new_directional_language_tagged_literal(value, language, direction)
                .map_err(|error| cell_error(column, row, format!("invalid language tag: {error}")))
        }
        _ => Err(cell_error(
            column,
            row,
            "literal requires either a datatype or a language tag; direction requires language",
        )),
    }
}

// pub(crate): the CSV decoder reports per-cell errors in the same shape.
pub(crate) fn cell_error(column: &str, row: usize, message: impl std::fmt::Display) -> ArrowError {
    ArrowError::invalid(format!(
        "invalid RDF term at column '{column}', row {row}: {message}"
    ))
}
