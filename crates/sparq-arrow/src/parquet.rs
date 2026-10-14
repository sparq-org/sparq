//! The `parquet`-feature-gated serialization of the Arrow RDF-term projection.

use arrow_schema::ArrowError as ArrowLibraryError;
use bytes::Bytes;
use oxrdf::Variable;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::errors::ParquetError;
use sparq_engine::QueryResult;

use crate::import::variables_from_schema;
use crate::{from_record_batch, to_record_batch, ArrowError, ArrowExportError};

// Keep Parquet failures inside the existing export error surface instead of
// adding an enum variant that would break downstream exhaustive matches.
fn export_error(error: ParquetError) -> ArrowExportError {
    ArrowExportError::Arrow(ArrowLibraryError::ParquetError(error.to_string()))
}

fn import_error(error: ParquetError) -> ArrowError {
    ArrowError::invalid(format!("Parquet import failed: {error}"))
}

fn decode_error(error: ArrowLibraryError) -> ArrowError {
    ArrowError::invalid(format!("Parquet Arrow decode failed: {error}"))
}

/// Serialize a SPARQL `SELECT` [`QueryResult`] as Parquet bytes.
///
/// The Parquet file contains the exact Arrow batch produced by [`to_record_batch`]: one
/// nullable five-field RDF-term struct per variable. No Parquet-specific term encoding
/// is introduced, so [`from_parquet_bytes`] is a row-for-row inverse.
///
/// # Errors
///
/// Returns [`ArrowExportError`] if the result cannot be projected to the RDF-term Arrow
/// schema or the Parquet writer rejects or fails to finalize the batch.
pub fn to_parquet_bytes(result: &QueryResult) -> Result<Vec<u8>, ArrowExportError> {
    let batch = to_record_batch(result)?;
    let mut writer =
        ArrowWriter::try_new(Vec::new(), batch.schema(), None).map_err(export_error)?;
    writer.write(&batch).map_err(export_error)?;
    writer.into_inner().map_err(export_error)
}

/// Read the SPARQL `SELECT` variables from a Parquet file's Arrow schema.
///
/// This validates the same variable names and nullable five-field RDF-term struct
/// schema as [`from_parquet_bytes`], but does not build a record-batch reader or decode
/// any row groups.
///
/// # Errors
///
/// Returns [`ArrowError`] if `bytes` do not contain readable Parquet metadata or their
/// Arrow schema violates this crate's RDF-term projection contract.
pub fn parquet_variables_from_bytes(bytes: &[u8]) -> Result<Vec<Variable>, ArrowError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes))
        .map_err(import_error)?;
    variables_from_schema(builder.schema().as_ref())
}

/// Read the total row count from a Parquet file's metadata.
///
/// This does not build a record-batch reader or decode any row groups.
///
/// # Errors
///
/// Returns [`ArrowError`] if `bytes` do not contain readable Parquet metadata.
// Keep the metadata-only path separate from full row decoding.
pub fn parquet_row_count_from_bytes(bytes: &[u8]) -> Result<usize, ArrowError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes))
        .map_err(import_error)?;
    Ok(builder.metadata().file_metadata().num_rows() as usize)
}

/// Deserialize Parquet bytes containing this crate's RDF-term Arrow projection.
///
/// Every decoded batch is validated through [`from_record_batch`]. This includes files
/// with no rows: their stored Arrow schema is validated before the reader is consumed,
/// preserving the variable projection for an empty result. Multiple Parquet row groups
/// are concatenated in file order.
///
/// # Errors
///
/// Returns [`ArrowError`] if `bytes` are not a readable Parquet file, their Arrow schema
/// differs from the exact nullable five-field term-struct schema, or any term cell is
/// invalid.
pub fn from_parquet_bytes(bytes: &[u8]) -> Result<QueryResult, ArrowError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes))
        .map_err(import_error)?;

    // Validate independently of decoded batches so a zero-row file cannot bypass the
    // schema contract.
    let mut result = QueryResult {
        vars: variables_from_schema(builder.schema().as_ref())?,
        rows: Vec::new(),
    };

    let reader = builder.build().map_err(import_error)?;
    for batch in reader {
        let decoded = from_record_batch(&batch.map_err(decode_error)?)?;
        if decoded.vars != result.vars {
            return Err(ArrowError::invalid(
                "Parquet row groups decoded with inconsistent SELECT variables",
            ));
        }
        result.rows.extend(decoded.rows);
    }
    Ok(result)
}
