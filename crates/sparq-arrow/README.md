# sparq-arrow

> 🤖 **SPARQ agent** — opt-in **Apache Arrow** columnar export for sparq.
> Projects a SPARQL `SELECT` result (`sparq_engine::QueryResult`) into an Arrow
> `RecordBatch` so query results flow into the dataframe / analytics / ML ecosystem
> (Polars, DuckDB, pandas) without a CSV round-trip. Issue #910, bead `sq-v78l4`.
>
> Bead `sq-lsp7k.16` adds the checked inverse, `from_record_batch`.
> Bead `sq-lsp7k.21` adds Parquet byte serialization over that same schema.
> Beads `sq-r3cab` and `sq-ksxa2` add Arrow IPC bytes and schema-only readers.
> Bead `sq-kix7x` adds a metadata-only Parquet row-count reader.
> Bead `sq-lsp7k` adds flattened CSV bytes (hand-rolled RFC 4180, zero new deps).

**Opt-in / lean-core by construction.** This is a separate leaf crate, and Arrow
import/export sits behind the `arrow` feature (OFF by default). The `arrow-*` dependency
closure NEVER enters `sparq-core` / `sparq-engine` / the wasm bundle; nothing in the
workspace default build depends on it. Parquet, IPC, and CSV byte serialization are
additional opt-in layers (`parquet` / `ipc` / `csv`, each implying `arrow`). The default
build of *this* crate pulls no Arrow container code — only the dependency-free
field-name constants and the schema docs.

## 🚀 Quickstart

```rust,ignore
use sparq_arrow::{from_record_batch, to_record_batch};
use sparq_engine::{query, QueryResult};
use sparq_core::Graph;

let graph = Graph::load_str("<http://ex/a> <http://ex/p> \"42\" .", "turtle")?;
let result: QueryResult = query(&graph, "SELECT ?s ?o WHERE { ?s ?p ?o }")?;

// One Arrow struct column per SELECT variable; one row per solution.
let batch = to_record_batch(&result)?;       // needs --features arrow
assert_eq!(batch.num_columns(), 2);          // ?s, ?o
assert_eq!(batch.schema().field(0).name(), "s");
let restored = from_record_batch(&batch)?;
assert_eq!(restored.vars, result.vars);
assert_eq!(restored.rows, result.rows);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`cargo add sparq-arrow --git https://github.com/sparq-org/sparq --features arrow`, with `sparq-core` and
`sparq-engine` from the same git source so `QueryResult` types match (in-workspace: `cargo build -p sparq-arrow --features arrow`).

To serialize the same term-struct batch as an in-memory Parquet file, enable the
default-OFF `parquet` feature:

```rust,ignore
use sparq_arrow::{
    from_parquet_bytes, parquet_row_count_from_bytes, parquet_variables_from_bytes,
    to_parquet_bytes,
};

let bytes = to_parquet_bytes(&result)?;
assert_eq!(parquet_variables_from_bytes(&bytes)?, result.vars);
assert_eq!(parquet_row_count_from_bytes(&bytes)?, result.rows.len());
let restored = from_parquet_bytes(&bytes)?;
assert_eq!(restored.vars, result.vars);
assert_eq!(restored.rows, result.rows);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `cargo add sparq-arrow --git https://github.com/sparq-org/sparq --features parquet` (or
`cargo build -p sparq-arrow --features parquet`); `parquet` implies `arrow`.

For an Arrow IPC stream instead, enable the default-OFF `ipc` feature and call
`to_ipc_bytes(&result)` / `from_ipc_bytes(&bytes)`; `ipc_variables_from_bytes(&bytes)`
reads only its schema, which is the same term-struct schema with empty-result variables
preserved. For CSV, enable the default-OFF `csv` feature and call `to_csv_bytes` /
`from_csv_bytes` (`csv_variables_from_bytes` reads only the header row): each
variable's term struct flattens to five `var.field` columns, an unbound cell is five
empty fields (still distinct from a bound empty literal), and the RFC-4180
serializer/parser is hand-rolled over `std`, so the feature adds zero dependencies.

## ✨ The RDF-term → Arrow mapping

Arrow has no native RDF-term type, so the export widens each term into a **struct** —
faithful and round-trippable, nothing is thrown away. Every `SELECT` variable becomes
one column of `Struct<kind, value, datatype, language, direction>` (all nullable
`Utf8`); the field names are the public `FIELD_*` / `RDF_TERM_FIELDS` constants.

| field        | meaning                                                                          |
|--------------|----------------------------------------------------------------------------------|
| `kind`       | `"uri"` / `"bnode"` / `"literal"` / `"triple"` (mirrors SPARQL-JSON `type`)       |
| `value`      | IRI string / blank-node label / literal lexical value / N-Triples-encoded triple |
| `datatype`   | datatype IRI of a **typed** literal (null otherwise)                             |
| `language`   | BCP-47 language tag of a **language-tagged** literal (null otherwise)            |
| `direction`  | RDF 1.2 base direction `"ltr"` / `"rtl"` (null otherwise)                        |

An **unbound** binding is a `null` struct slot — distinct from a bound empty-string
literal. `term_schema(&vars)` builds the schema without materialising rows.
`from_record_batch(&batch)` validates that exact schema and reconstructs the variables,
row order, unbound cells, and RDF terms. Invalid variable names, schemas, kinds, IRIs,
blank-node labels, language tags, directions, triple terms, or incompatible literal
metadata return `ArrowError`; they never panic or silently select a fallback term kind.
`to_parquet_bytes` / `to_ipc_bytes` / `to_csv_bytes` wrap this exact projection in
checked containers whose importers validate it before decoding; empty results keep
their variables, and the schema-only `*_variables_from_bytes` readers (plus
`parquet_row_count_from_bytes`) decode no rows. CSV alone cannot carry a zero-variable
result — `to_csv_bytes` rejects it honestly rather than writing unreadable rows.

## 📚 Honest boundary / caveats

- **`xsd:string` is written explicitly**, not elided (SPARQL-JSON elides it); `kind`
  already separates literals from IRIs, so this is purely additive and removes a
  consumer special-case.
- **No numeric narrowing yet.** `42^^xsd:integer` is the *string* `"42"` plus a datatype
  field, not an Arrow `Int64`. A typed-column projection (numbers/dates → native Arrow
  types, term-struct fallback) is a deliberate follow-up — this v1 is the lossless
  baseline a typed view can build on.
- **Triple terms are stringified** to N-Triples in `value` (`kind = "triple"`), not
  exploded into nested struct fields.
- This is a **projection for transport**, not a canonical RDF serialisation: the Arrow
  batch and its Parquet / IPC / CSV containers are not RDF documents. Each importer is
  a checked inverse of its corresponding exporter.
- **Python binding.** `sparq-py`'s opt-in `arrow` feature exposes `Graph.query_arrow() -> pyarrow.Table` over `to_record_batch` via the Arrow C Data Interface.

## License

MIT — see the repository-root [`LICENSE`](../../LICENSE). © 2026 Jesse Wright.
