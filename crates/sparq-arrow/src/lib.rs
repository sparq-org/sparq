#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

//! # The RDF-term → Arrow mapping (the load-bearing design decision)
//!
//! A SPARQL `SELECT` result is a table: a fixed list of *variables* (columns) and a
//! list of *solution rows*, where each cell is either **unbound** or an RDF term —
//! an IRI, a blank node, a literal (with an optional datatype IRI **or** an optional
//! language tag, itself optionally carrying an RDF 1.2 base direction), or an RDF 1.2
//! triple term. Arrow has no native RDF-term type, so any export must choose how to
//! widen it. The honest options (issue #910) are *(a)* a single widened string per
//! column plus a side-channel for the type, or *(b)* a **struct** per column that
//! keeps the term's components in separate fields. This crate takes **(b)** because it
//! is faithful and round-trippable: nothing about the term is thrown away.
//!
//! Each SELECT variable becomes one Arrow column of the struct type produced by
//! `term_struct_type` (a `Struct` of five nullable `Utf8` fields — the names are the
//! [`FIELD_*`](FIELD_KIND) constants; the `arrow`-gated `term_struct_type` /
//! `to_record_batch` / `from_record_batch` / `term_schema` items carry the full rustdoc):
//!
//! | field        | meaning                                                                                   |
//! |--------------|-------------------------------------------------------------------------------------------|
//! | `kind`       | `"uri"`, `"bnode"`, `"literal"`, or `"triple"` — the term kind, mirroring SPARQL-JSON's `type` |
//! | `value`      | the lexical form: the IRI string / blank-node label / literal lexical value / N-Triples-encoded triple |
//! | `datatype`   | the datatype IRI for a **typed** literal (null otherwise; `xsd:string` is written explicitly, never elided) |
//! | `language`   | the BCP-47 language tag for a **language-tagged** literal (null otherwise)                 |
//! | `direction`  | the RDF 1.2 base direction (`"ltr"` / `"rtl"`) for a directional literal (null otherwise)  |
//!
//! An **unbound** cell is a `null` struct (the whole row's struct slot is invalid), which
//! is exactly how a missing binding should read in a dataframe — distinct from a bound
//! empty-string literal (`kind="literal", value=""`).
//!
//! ## Honest boundary / caveats
//!
//! * **`xsd:string` is written explicitly.** SPARQL-JSON elides the `xsd:string` datatype;
//!   we do NOT — a simple plain literal carries `datatype = "http://www.w3.org/2001/XMLSchema#string"`
//!   so a consumer never has to special-case "datatype is null but it's still a literal".
//!   `kind` already distinguishes literals from IRIs/bnodes, so this is purely additive.
//! * **No numeric narrowing (yet).** `42^^xsd:integer` is the *string* `"42"` with a datatype
//!   field — NOT an Arrow `Int64`. A typed-column projection (numbers/dates → native Arrow
//!   types, with the term struct as a fallback) is a deliberate follow-up (see the crate
//!   README); this v1 is the faithful, lossless baseline that a typed view can build on.
//! * **Triple terms are stringified.** An RDF 1.2 triple term is encoded into its
//!   N-Triples form in `value` (`kind = "triple"`); its components are not exploded into
//!   nested struct fields. RDF-1.2 triple terms in result *bindings* are rare; a nested
//!   encoding is left to the same typed-view follow-up.
//! * This is a **projection for transport**, not a canonical RDF serialisation:
//!   `from_record_batch` reconstructs terms from the five fields, but the Arrow batch is not
//!   itself an RDF document.
//! * With the opt-in `parquet` feature, `to_parquet_bytes` / `from_parquet_bytes`
//!   serialize and restore this exact RecordBatch projection;
//!   `parquet_variables_from_bytes` reads only its variables from the stored schema,
//!   while `parquet_row_count_from_bytes` reads only its total row count from metadata.
//!   Parquet adds a container; it does not define a second RDF-term encoding.
//! * With the opt-in `ipc` feature, `to_ipc_bytes` / `from_ipc_bytes` do the same through
//!   the Arrow IPC streaming format, including preservation of an empty result's schema;
//!   `ipc_variables_from_bytes` reads that schema without decoding record batches.
//! * With the opt-in `csv` feature, `to_csv_bytes` / `from_csv_bytes` serialize the same
//!   projection as CSV, and `csv_variables_from_bytes` reads only the header row. CSV has
//!   no nested types, so each variable's term struct is **flattened** to the five columns
//!   `var.kind` / `var.value` / `var.datatype` / `var.language` / `var.direction`; and CSV
//!   has no value-level null, so an unbound cell is five empty fields with boundness
//!   carried by the `kind` column — distinct from a bound empty-string literal
//!   (`kind=literal` + explicit `xsd:string`). Every term the struct can express
//!   round-trips losslessly; the one shape CSV cannot carry is a **zero-variable** result
//!   (no columns), which `to_csv_bytes` rejects with an error rather than corrupting.
//!   The RFC-4180 serializer/parser is hand-rolled over `std` (the schema is fixed, so
//!   the feature adds **zero** dependencies); the reader decodes bound terms through the
//!   same term decoder as `from_record_batch`, so the two readers cannot drift.

/// Struct field name: the term kind — `"uri"`, `"bnode"`, `"literal"`, or `"triple"`.
pub const FIELD_KIND: &str = "kind";
/// Struct field name: the lexical form (IRI / blank-node label / literal value / N-Triples triple).
pub const FIELD_VALUE: &str = "value";
/// Struct field name: the datatype IRI of a typed literal (null for non-literals and
/// language-tagged literals).
pub const FIELD_DATATYPE: &str = "datatype";
/// Struct field name: the language tag of a language-tagged literal (null otherwise).
pub const FIELD_LANGUAGE: &str = "language";
/// Struct field name: the RDF 1.2 base direction (`"ltr"` / `"rtl"`) of a directional
/// literal (null otherwise).
pub const FIELD_DIRECTION: &str = "direction";

/// The five struct field names, in column order, that make up the RDF-term struct each
/// SELECT variable is projected to. Dependency-free, so it is available even without the
/// `arrow` feature (documentation / consumers that want the field layout without pulling
/// Arrow). See the crate-level docs for the meaning of each field.
pub const RDF_TERM_FIELDS: [&str; 5] = [
    FIELD_KIND,
    FIELD_VALUE,
    FIELD_DATATYPE,
    FIELD_LANGUAGE,
    FIELD_DIRECTION,
];

#[cfg(feature = "arrow")]
mod export;

#[cfg(feature = "arrow")]
mod import;

#[cfg(feature = "parquet")]
mod parquet;

#[cfg(feature = "ipc")]
mod ipc;

#[cfg(feature = "csv")]
mod csv;

#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
pub use export::{term_schema, term_struct_type, to_record_batch, ArrowExportError};

#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
pub use import::{from_record_batch, ArrowError};

#[cfg(feature = "parquet")]
#[cfg_attr(docsrs, doc(cfg(feature = "parquet")))]
pub use parquet::{
    from_parquet_bytes, parquet_row_count_from_bytes, parquet_variables_from_bytes,
    to_parquet_bytes,
};

#[cfg(feature = "ipc")]
#[cfg_attr(docsrs, doc(cfg(feature = "ipc")))]
pub use ipc::{from_ipc_bytes, ipc_variables_from_bytes, to_ipc_bytes};

#[cfg(feature = "csv")]
#[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
pub use csv::{csv_variables_from_bytes, from_csv_bytes, to_csv_bytes};
