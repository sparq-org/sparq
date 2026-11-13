# sparq-jsonld

A native W3C [JSON-LD 1.1] document pipeline for sparq: Context Processing,
Expansion, Flattening, Compaction, Framing, and RDF conversion over JSON trees.
It has zero mandatory dependencies and forbids unsafe code.

The JSON-LD EARL report measures this crate's JSON-LD 1.1 API operations (expand,
compact, flatten, frame, toRdf, fromRdf). `sparq-core`'s Graph JSON-LD ingest uses
oxjsonld and is not what EARL measures. A separate non-EARL `TOTAL toRdf-ingest`
ratchet runs the same toRdf manifest through Graph acceptance and oxjsonld dataset
comparison: 429 passes, 38 failures, no skips (floor 429, failure ceiling 38).
The native API toRdf lane has 454 passes, 11 failures and two skips including
legacy 1.0-only entries; its 1.1 EARL subset passes 448 / 448.

[JSON-LD 1.1]: https://www.w3.org/TR/json-ld11/

## 🚀 Quickstart

```rust
use sparq_jsonld::{expand, Json, JsonLdOptions, NoopLoader};

let input = Json::parse(r#"{"https://example.org/name":"Alice"}"#)?;
let expanded = expand(&input, &JsonLdOptions::default(), &NoopLoader)?;
let mut output = String::new();
expanded.write(&mut output);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## ✨ Features

- `Json` is an insertion-order-preserving JSON AST with a strict parser and
  writer. The engine re-exports this shared value type.
- `JsonLdErrorCode` carries the W3C error registry; `as_str()` returns the exact
  specification string for negative-test assertions.
- `JsonLdOptions` carries specification defaults for base, processing mode,
  expansion contexts, compaction, direction encoding, and framing.
- `expand` implements scoped contexts, container maps, keyword aliases,
  `@nest`, `@reverse`, `@included`, and JSON literals.
- `compact::compact` and `compact_expanded` implement document compaction and
  value compaction. Ambiguous absolute IRIs raise `IRI confused with prefix`.
- `flatten` returns expanded flattened output. `flatten::flatten_with_context`
  optionally compacts it while retaining the graph envelope and honoring
  property-level `compact_arrays`.
- `node_map::try_generate_node_map` rejects conflicting indexes. Fallible
  flattening, framing, and RDF conversion use this validation; the existing
  infallible `generate_node_map` and `flatten_expanded` helpers have no error return.
- `frame::frame` implements frame matching, embedding, defaults, named graphs
  and validation. The matching and compaction halves are separately callable.
- `from_rdf::from_rdf` converts the crate-local `RdfQuad`/`RdfTerm` model to
  expanded JSON-LD, including RDF lists, JSON literals and native types.
- `to_rdf::to_rdf` expands JSON-LD and converts it to that RDF model, including
  named graphs, lists, canonical scalar/JSON literals, `expand_context`, both
  `rdf_direction` encodings and optional generalized RDF.

### Document loading

`DocumentLoader` supplies all remote documents, contexts and imports. The default
`NoopLoader` refuses loading; the processor contains no HTTP client. `FsLoader`
maps URL prefixes to trusted local fixture directories.

An absolute IRI carried as `Json::Str` is remote input for expansion and operations
that expand. `RemoteDocument` supplies raw text, final document URL, content type,
context URL and optional profile. The loader handles redirects, status errors,
alternate selection and multiple context links. The processor applies context
links for JSON media types other than `application/ld+json`; it ignores them for
JSON-LD and HTML/XHTML. Unsupported types and invalid JSON raise
`loading document failed`. Missing content type preserves the JSON-LD default.

### Optional HTML extraction

The `html` Cargo feature is **off by default**. It uses html5ever's tokenizer and tree builder for
HTML document order and script-data rules, and quick-xml for XHTML. The default JSON and lean wasm
builds do not link these dependencies.

Loaded `text/html` and `application/xhtml+xml` documents select the first JSON-LD
script by default. `extract_all_scripts` defaults to false; true merges all scripts
and flattens their outer arrays. A percent-decoded URL fragment selects a matching
script id regardless of that option. The first base element supplies the extracted
content's base, resolved against the final document URL or explicit base fallback.

Missing targets or scripts raise `loading document failed`; extracting all scripts
from a document with none returns an empty array. Invalid script JSON raises
`invalid script element`. Comment wrappers are not stripped. HTML character
references inside script data remain literal; XHTML uses XML references and CDATA.

### Opt-in by construction

Consumers control whether they link this crate. The engine uses it behind the
default-off `serialize-rdf` feature, enabled by the native binaries' default-on
`jsonld` feature. Enabling those consumer features does not enable `html`.
The high-level `api` module remains a documented facade placeholder.

## 📚 Learn more

- Usage and loader examples: `skills/jsonld/SKILL.md` and its references.
- Conformance floors: `crates/sparq-conformance/src/floors/`.
- JSON-LD 1.1 API: <https://www.w3.org/TR/json-ld11-api/>
- JSON-LD 1.1 Framing: <https://www.w3.org/TR/json-ld11-framing/>

## License

MIT © the sparq authors.
