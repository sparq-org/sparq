//! The `csv`-feature-gated CSV serialization of the flattened RDF-term projection.
//!
//! CSV has no nested types, so each SELECT variable's five-field term struct is
//! flattened to five columns named `var.kind` / `var.value` / `var.datatype` /
//! `var.language` / `var.direction`. CSV also has no value-level null — absence is
//! encoded as the empty field — so boundness is carried by the `kind` column: an
//! unbound cell is five empty fields, while a bound empty-string literal keeps
//! `kind=literal` plus its explicit `xsd:string` datatype. Since a bound term's `kind`
//! is never empty and a valid datatype IRI / language tag / direction is never the
//! empty string, this flattening loses nothing the term struct can express.
//!
//! The serializer and parser are hand-rolled over `std` — no CSV dependency — because
//! the schema is fixed (five string columns per variable) and a direct
//! `QueryResult` ⇄ bytes pass avoids materialising the intermediate Arrow
//! `RecordBatch` twice. The emitted dialect is RFC 4180 with the conventional
//! hardening for CR-tolerant readers: a field is quoted iff it contains a comma, a
//! double quote, CR, or LF; quotes are escaped by doubling; records end with LF; the
//! header row is always present. The parser accepts LF, CRLF, and bare-CR record
//! terminators and a missing final terminator.

use arrow_schema::{ArrowError as ArrowLibraryError, Field, Schema};
use oxrdf::{Term, Variable};
use sparq_engine::QueryResult;

use crate::import::{cell_error, term_from_parts, variables_from_schema};
use crate::{term_struct_type, ArrowError, ArrowExportError, FIELD_KIND, RDF_TERM_FIELDS};

/// The flattened CSV column name for one variable's term-struct field. A SPARQL
/// variable name can never contain `.` (the `VARNAME` production excludes it, and
/// `oxrdf::Variable` enforces that), so the `var.field` spelling is unambiguous.
fn flat_column_name(variable: &str, field: &str) -> String {
    format!("{}.{}", variable, field)
}

/// Serialize a SPARQL `SELECT` [`QueryResult`] as CSV bytes (header row included).
///
/// The CSV is exactly the flattened form of the [`to_record_batch`] projection: CSV has
/// no nested types, so each variable's five-field RDF-term struct becomes the five
/// columns `var.kind` / `var.value` / `var.datatype` / `var.language` /
/// `var.direction`. No CSV-specific term encoding is introduced beyond that flattening,
/// so [`from_csv_bytes`] is a row-for-row inverse. Quoting is RFC 4180: a field is
/// quoted iff it contains a comma, a double quote, CR, or LF, with quotes escaped by
/// doubling; records are LF-terminated.
///
/// An unbound cell is five empty fields. Boundness is carried by the `kind` column, so
/// an unbound cell stays distinct from a bound empty-string literal (`kind=literal`
/// with the explicit `xsd:string` datatype).
///
/// # Errors
///
/// Returns [`ArrowExportError`] if two SELECT variables share a name or the result has
/// zero variables (CSV cannot represent zero-column rows, unlike the IPC and Parquet
/// containers).
///
/// [`to_record_batch`]: crate::to_record_batch
pub fn to_csv_bytes(result: &QueryResult) -> Result<Vec<u8>, ArrowExportError> {
    if result.vars.is_empty() {
        return Err(ArrowExportError::Arrow(ArrowLibraryError::CsvError(
            "a zero-variable SELECT result cannot be represented as CSV rows".to_string(),
        )));
    }
    let mut seen = std::collections::HashSet::with_capacity(result.vars.len());
    for variable in &result.vars {
        if !seen.insert(variable.as_str()) {
            return Err(ArrowExportError::DuplicateVariable(
                variable.as_str().to_string(),
            ));
        }
    }

    // Rough pre-size: the header plus a small per-cell estimate; `Vec` growth
    // amortises any underestimate.
    let mut out =
        Vec::with_capacity(64 * result.vars.len() + result.rows.len() * result.vars.len() * 24);

    for (index, variable) in result.vars.iter().enumerate() {
        if index > 0 {
            out.push(b',');
        }
        for (child_index, field) in RDF_TERM_FIELDS.iter().enumerate() {
            if child_index > 0 {
                out.push(b',');
            }
            // Variable names cannot contain the quote-triggering bytes, so the header
            // needs no quoting.
            out.extend_from_slice(variable.as_str().as_bytes());
            out.push(b'.');
            out.extend_from_slice(field.as_bytes());
        }
    }
    out.push(b'\n');

    let width = result.vars.len();
    for row in &result.rows {
        for column in 0..width {
            if column > 0 {
                out.push(b',');
            }
            // A row shorter than `vars` (should not happen for a well-formed result)
            // reads as unbound, mirroring `to_record_batch`.
            write_cell(&mut out, row.get(column).and_then(|cell| cell.as_ref()));
        }
        out.push(b'\n');
    }
    Ok(out)
}

// Write one cell as its five fields (four commas). Field order is RDF_TERM_FIELDS:
// kind, value, datatype, language, direction.
fn write_cell(out: &mut Vec<u8>, cell: Option<&Term>) {
    match cell {
        None => out.extend_from_slice(b",,,,"),
        Some(Term::NamedNode(node)) => {
            out.extend_from_slice(b"uri,");
            write_field(out, node.as_str());
            out.extend_from_slice(b",,,");
        }
        Some(Term::BlankNode(node)) => {
            out.extend_from_slice(b"bnode,");
            write_field(out, node.as_str());
            out.extend_from_slice(b",,,");
        }
        Some(Term::Literal(literal)) => {
            out.extend_from_slice(b"literal,");
            write_field(out, literal.value());
            out.push(b',');
            if let Some(language) = literal.language() {
                // Language-tagged: the language tag, no datatype (its datatype is the
                // rdf:langString/dirLangString machinery, implied by the tag's presence).
                out.push(b',');
                write_field(out, language);
                out.push(b',');
                match literal.direction() {
                    Some(oxrdf::BaseDirection::Ltr) => out.extend_from_slice(b"ltr"),
                    Some(oxrdf::BaseDirection::Rtl) => out.extend_from_slice(b"rtl"),
                    None => {}
                }
            } else {
                // Typed (or plain) literal: carry the datatype IRI explicitly,
                // including xsd:string (we do NOT elide it — see the crate docs).
                write_field(out, literal.datatype().as_str());
                out.extend_from_slice(b",,");
            }
        }
        Some(term @ Term::Triple(_)) => {
            // RDF 1.2 triple term: its N-Triples form `<< s p o >>` in `value`.
            out.extend_from_slice(b"triple,");
            write_field(out, &term.to_string());
            out.extend_from_slice(b",,,");
        }
    }
}

// RFC 4180 field encoder: quote iff the field contains a comma, a double quote, CR, or
// LF (CR is included even though records end with LF, because CR-tolerant readers
// would otherwise split the field), and escape quotes by doubling.
fn write_field(out: &mut Vec<u8>, field: &str) {
    let bytes = field.as_bytes();
    if !bytes
        .iter()
        .any(|&byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'))
    {
        out.extend_from_slice(bytes);
        return;
    }
    out.push(b'"');
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'"' {
            out.extend_from_slice(&bytes[start..=index]);
            out.push(b'"');
            start = index + 1;
        }
    }
    out.extend_from_slice(&bytes[start..]);
    out.push(b'"');
}

/// One parsed CSV field: borrowed from the input unless quote-unescaping forced a copy.
enum CsvField<'a> {
    Borrowed(&'a str),
    Owned(String),
}

impl CsvField<'_> {
    fn as_str(&self) -> &str {
        match self {
            CsvField::Borrowed(text) => text,
            CsvField::Owned(text) => text,
        }
    }
}

/// RFC 4180 record parser over validated UTF-8 text. Every field boundary is an ASCII
/// byte (`,` `"` CR LF), so byte-position slicing of the `str` is always on a char
/// boundary. Accepts LF, CRLF, and bare-CR record terminators and a missing final
/// terminator; quoted fields may contain any of those plus doubled-quote escapes.
struct RecordParser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> RecordParser<'a> {
    fn new(text: &'a str) -> Self {
        RecordParser { text, pos: 0 }
    }

    /// Parse the next record into `fields`; returns `false` at end of input.
    fn next_record(&mut self, fields: &mut Vec<CsvField<'a>>) -> Result<bool, ArrowError> {
        fields.clear();
        let bytes = self.text.as_bytes();
        if self.pos >= bytes.len() {
            return Ok(false);
        }
        loop {
            if bytes.get(self.pos) == Some(&b'"') {
                // Quoted field: borrow when no escaped quote occurs; otherwise
                // accumulate the unescaped content in an owned buffer.
                self.pos += 1;
                let mut start = self.pos;
                let mut owned: Option<String> = None;
                loop {
                    match bytes.get(self.pos) {
                        None => {
                            return Err(ArrowError::invalid(
                                "CSV import failed: unterminated quoted field",
                            ));
                        }
                        Some(b'"') if bytes.get(self.pos + 1) == Some(&b'"') => {
                            // Escaped quote: copy the run seen so far plus one quote.
                            let buffer = owned.get_or_insert_with(String::new);
                            buffer.push_str(&self.text[start..self.pos]);
                            buffer.push('"');
                            self.pos += 2;
                            start = self.pos;
                        }
                        Some(b'"') => {
                            match owned.take() {
                                Some(mut buffer) => {
                                    buffer.push_str(&self.text[start..self.pos]);
                                    fields.push(CsvField::Owned(buffer));
                                }
                                None => {
                                    fields.push(CsvField::Borrowed(&self.text[start..self.pos]));
                                }
                            }
                            self.pos += 1;
                            break;
                        }
                        Some(_) => self.pos += 1,
                    }
                }
            } else {
                // Unquoted field: up to the delimiter or a record terminator.
                let start = self.pos;
                while self.pos < bytes.len()
                    && !matches!(bytes[self.pos], b',' | b'\n' | b'\r')
                {
                    self.pos += 1;
                }
                fields.push(CsvField::Borrowed(&self.text[start..self.pos]));
            }
            // After a field: a delimiter continues the record; a terminator or the end
            // of input finishes it; anything else is trailing junk after a quote.
            match bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'\n') => {
                    self.pos += 1;
                    return Ok(true);
                }
                Some(b'\r') => {
                    self.pos += 1;
                    if bytes.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    return Ok(true);
                }
                None => return Ok(true),
                Some(_) => {
                    return Err(ArrowError::invalid(
                        "CSV import failed: unexpected data after a closing quote",
                    ));
                }
            }
        }
    }
}

// Parse and validate the header record, reconstructing the SELECT variables. Shared by
// the schema-only reader and the full decoder so the header contract cannot drift.
fn parse_header<'a>(
    parser: &mut RecordParser<'a>,
    fields: &mut Vec<CsvField<'a>>,
) -> Result<Vec<Variable>, ArrowError> {
    let column_count = if parser.next_record(fields)? {
        fields.len()
    } else {
        0
    };
    if column_count == 0 || !column_count.is_multiple_of(RDF_TERM_FIELDS.len()) {
        return Err(ArrowError::invalid(format!(
            "CSV header has {} columns; the flattened RDF-term projection needs a non-zero multiple of {}",
            column_count,
            RDF_TERM_FIELDS.len()
        )));
    }

    let kind_suffix = format!(".{}", FIELD_KIND);
    let mut schema_fields = Vec::with_capacity(column_count / RDF_TERM_FIELDS.len());
    for group in fields.chunks(RDF_TERM_FIELDS.len()) {
        let first = group[0].as_str();
        let variable = first.strip_suffix(&kind_suffix).unwrap_or("");
        if variable.is_empty() {
            return Err(ArrowError::invalid(format!(
                "CSV column '{}' does not name a variable's '{}' field",
                first, FIELD_KIND
            )));
        }
        let variable = variable.to_string();
        for (name, field) in group.iter().zip(RDF_TERM_FIELDS) {
            let expected = flat_column_name(&variable, field);
            if name.as_str() != expected {
                return Err(ArrowError::invalid(format!(
                    "CSV column '{}' does not match the expected flattened column '{}'",
                    name.as_str(),
                    expected
                )));
            }
        }
        schema_fields.push(Field::new(variable, term_struct_type(), true));
    }
    // One schema-validation path: route the reconstructed struct schema through the
    // same validator as the RecordBatch / IPC / Parquet readers so the CSV header
    // reader cannot drift from the full row decoder.
    variables_from_schema(&Schema::new(schema_fields))
}

/// Read the SPARQL `SELECT` variables from a CSV header row.
///
/// This validates the same flattened five-columns-per-variable header contract as
/// [`from_csv_bytes`] — and routes the reconstructed variables through the shared
/// RDF-term schema validation — but does not decode any data rows.
///
/// # Errors
///
/// Returns [`ArrowError`] if `bytes` do not start with a readable CSV header row or the
/// header does not follow the `var.kind` … `var.direction` flattened projection.
pub fn csv_variables_from_bytes(bytes: &[u8]) -> Result<Vec<Variable>, ArrowError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ArrowError::invalid(format!("CSV import failed: {}", error)))?;
    let mut parser = RecordParser::new(text);
    parse_header(&mut parser, &mut Vec::new())
}

/// Deserialize CSV bytes containing the flattened RDF-term projection.
///
/// The header row is validated before any data row is decoded, preserving the variable
/// projection when the result has no rows (the IPC/Parquet empty-schema precedent).
/// Rows are decoded in order, and every bound cell is reconstructed through the same
/// term decoder as [`from_record_batch`], so the CSV and RecordBatch readers cannot
/// disagree on term semantics.
///
/// CSV cannot distinguish an empty field from an absent one: on a row whose `kind`
/// field is non-empty (a bound term), an empty `value` field reads back as the empty
/// string (the bound empty-string-literal case), while a row whose `kind` field is
/// empty is an unbound cell and must have all five fields empty.
///
/// # Errors
///
/// Returns [`ArrowError`] if `bytes` are not readable CSV, the header does not follow
/// the flattened five-columns-per-variable projection, a data row does not have exactly
/// five fields per variable, or any term cell is invalid.
///
/// [`from_record_batch`]: crate::from_record_batch
pub fn from_csv_bytes(bytes: &[u8]) -> Result<QueryResult, ArrowError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ArrowError::invalid(format!("CSV import failed: {}", error)))?;
    let mut parser = RecordParser::new(text);
    let mut fields: Vec<CsvField<'_>> = Vec::new();

    // Validate the header independently of the data rows so a zero-row file cannot
    // bypass the schema contract.
    let vars = parse_header(&mut parser, &mut fields)?;

    let width = vars.len();
    let expected_fields = width * RDF_TERM_FIELDS.len();
    let mut rows = Vec::new();
    let mut row_index = 0usize;
    while parser.next_record(&mut fields)? {
        if fields.len() != expected_fields {
            return Err(ArrowError::invalid(format!(
                "CSV row {} has {} fields; the flattened RDF-term projection needs {}",
                row_index,
                fields.len(),
                expected_fields
            )));
        }
        let mut row = Vec::with_capacity(width);
        for (var_index, variable) in vars.iter().enumerate() {
            let group = &fields[var_index * RDF_TERM_FIELDS.len()..];
            row.push(decode_cell(group, variable.as_str(), row_index)?);
        }
        rows.push(row);
        row_index += 1;
    }
    Ok(QueryResult { vars, rows })
}

// Decode one cell from its five flattened fields. Boundness is decided by `kind`
// (never empty for a bound term): a bound row's empty `value` field is the empty
// string, while an empty `kind` marks an unbound row — which must not carry data in
// any other field, so a malformed "unbound slot carrying data" fails closed instead of
// being silently dropped.
fn decode_cell(group: &[CsvField<'_>], column: &str, row: usize) -> Result<Option<Term>, ArrowError> {
    let kind = group[0].as_str();
    let value = group[1].as_str();
    let datatype = group[2].as_str();
    let language = group[3].as_str();
    let direction = group[4].as_str();

    if kind.is_empty() {
        if !value.is_empty() || !datatype.is_empty() || !language.is_empty()
            || !direction.is_empty()
        {
            return Err(cell_error(
                column,
                row,
                "an unbound struct slot must have five null children",
            ));
        }
        return Ok(None);
    }

    // The empty field is CSV's spelling of absence for the optional components; a
    // valid datatype IRI / language tag / direction is never the empty string.
    term_from_parts(
        kind,
        value,
        (!datatype.is_empty()).then_some(datatype),
        (!language.is_empty()).then_some(language),
        (!direction.is_empty()).then_some(direction),
        column,
        row,
    )
    .map(Some)
}
