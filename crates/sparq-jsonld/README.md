# sparq-jsonld

A native, **dependency-free** W3C **[JSON-LD 1.1]** document pipeline for sparq —
Context Processing, Expansion, Flattening, Compaction, Framing, and RDF
(de)serialization operating on JSON trees, not on a lossy RDF projection.

[JSON-LD 1.1]: https://www.w3.org/TR/json-ld11/

> [GPT-5.6] Build-out status reconciled in `sq-ci15w`; implementation provenance remains
> recorded on the individual algorithms. [OPUS-5] (`sq-ghdfw`) Re-verified against the
> tree: `to_rdf` and `api` are the only remaining stubs.
> Epic `sq-oy1f` (design record `research/jsonld-1.1-design.md`).

## 🚀 Quickstart

```rust
use sparq_jsonld::{Json, JsonLdError, JsonLdErrorCode, JsonLdOptions};
use sparq_jsonld::{DocumentLoader, NoopLoader};

// The dependency-free JSON value model shared by the whole pipeline.
let mut node = Json::obj();
node.set("@id", Json::Str("http://example.org/a".into()));
let mut out = String::new();
node.write(&mut out);
assert_eq!(out, r#"{"@id":"http://example.org/a"}"#);

// Processing options carry the spec defaults.
let opts = JsonLdOptions::default();
assert!(opts.compact_arrays);

// The DEFAULT loader is fail-closed: no ambient network.
let err: JsonLdError = NoopLoader.load_document("https://ex/ctx").unwrap_err();
assert_eq!(err.code(), JsonLdErrorCode::LoadingDocumentFailed);
```

## ✨ Features

- **`Json` AST** — a minimal, insertion-order-preserving JSON value type (no
  `serde_json`). Moved here from `sparq-engine`'s JSON-LD writer so the whole
  pipeline shares one value type; the engine re-exports it (public API preserved).
- **Error registry** — `JsonLdErrorCode` is the full W3C JSON-LD 1.1 error-code
  set as a closed enum whose `as_str()` is the **exact** spec string, so the
  conformance suite's `expectErrorCode` negative tests become assertable.
- **Processing options** — `JsonLdOptions` models `base`, `processingMode`,
  `expandContext`, `rdfDirection`, `compactArrays`, `ordered`, the framing flags,
  and more, once, with the specification's defaults.
- **Deny-by-default loading** — `DocumentLoader` dereferences remote `@context` /
  `@import` / documents **only** through an explicit loader. The default
  `NoopLoader` refuses every load (`loading document failed`); `FsLoader` maps URL
  prefixes to trusted local fixtures. The network `HttpLoader` (with an SSRF
  allowlist) is a later, opt-in feature.

### Opt-in by construction

Nothing in sparq's default build or the wasm artifact depends on this crate —
`sparq-core` and `sparq-engine` stay lean. `sparq-engine` pulls it in only behind
its off-by-default `serialize-rdf` feature; the crate has **zero mandatory
dependencies**, is `#![forbid(unsafe_code)]`, and adds no default dependency
anywhere.

### Build-out status

Shipped: `Json` AST, error registry, options, loader trait, **Context Processing**
(`context`), and document-level **Expansion** (`expand::expand`) — scoped contexts,
container maps, `@nest`, `@reverse`, `@included`, `@json`, keyword aliases (beads
`sq-oy1f.24`/`sq-oy1f.25`). Bead `sq-90mu3` adds the compaction-side companions:

- **Inverse Context Creation** (`InverseContext` / `ActiveContext::inverse_context`)
  — §4.3: maps every IRI to the best term per (container, type/language), tie-broken
  shortest-first then lexicographic.
- **IRI Compaction** (`compact_iri`) — §7.1: keyword aliases, term lookup via inverse
  context, vocab-relative suffix, `prefix:suffix` compact IRIs, base-relative paths.
- **Term Selection** (internal to `context::inverse`) — §7.2: the container ×
  preferred-value walk, consumed by `compact_iri` and by the document Compaction
  Algorithm (bead `sq-oy1f.27`, since landed — see below).

Bead `sq-oy1f.26` adds **Node Map Generation** (`generate_node_map`, §7.2) with a
deterministic `_:bN` blank-node issuer, and the document-level **Flattening Algorithm**
(`flatten`, §7.1 = expand ∘ node-map ∘ named-graph fold, sorted by `@id`, empty nodes
dropped). The `flatten` conformance lane runs the native document oracle.

Bead `sq-oy1f.28` adds **Serialize RDF as JSON-LD** (`from_rdf::from_rdf`, §8.1): an RDF
dataset (the crate-local `RdfTerm`/`RdfQuad` model — still zero deps) becomes the expanded
document, with `rdfDirection` in both modes, `@json` literals (strict parse, `invalid JSON
literal` on malformed input), `rdf:List` → `@list` reconstruction (nested lists included;
malformed/shared chains stay plain nodes), and `useNativeTypes`/`useRdfType` via
`FromRdfOptions`. The `fromRdf` conformance lane runs this native path document-level.

Bead `sq-oy1f.27` adds the document-level **Compaction Algorithm** + **Value
Compaction** (`compact::compact` / `compact::compact_expanded`): scoped
(property/type) contexts with previous-context reversion, container reshaping
(`@list`, `@language`/`@index`/`@id`/`@type` maps, the `@graph` container forms),
`@nest`, `@reverse` redistribution, keyword aliasing, and the `compactArrays` /
`compactToRelative` / `ordered` options. The `compact` conformance lane compares
against the W3C **expected** documents (the normative oracle; see
`sparq-conformance`'s `floors::compact` for the honest fail/skip buckets). Bead `sq-gzsky`
turned the `expand`/`compact` **negative lanes** ON — each `NegativeEvaluationTest` passes iff
the exact `expectErrorCode` is raised, which is what makes the closed [`JsonLdErrorCode`](src/error.rs) enum load-bearing — and fixed seven spec divergences it surfaced.

Beads `sq-oy1f.27` / `.29` add the document-level **Framing Algorithm**
(`frame::frame` / `frame::frame_expanded`): frame matching and value patterns,
`@embed`, `@explicit` / `@default`, named graphs, list re-emission, and framing error
validation. Its pinned W3C framing lane passes all 92 cases against the normative
document oracle; that complete lane is not a blanket conformance claim for remote
loading, HTML extraction, or other JSON-LD surfaces.

Only `to_rdf` and `api` remain documented stubs. `publish = false` is the crate's
current internal-release posture, not an indication that compaction or framing is
unimplemented.

## 📚 Learn more

- Design record: `research/jsonld-1.1-design.md` (epic `sq-oy1f`).
- JSON-LD 1.1 API: <https://www.w3.org/TR/json-ld11-api/>
- JSON-LD 1.1 Framing: <https://www.w3.org/TR/json-ld11-framing/>
- Data-format serialization surface: `skills/data-formats/SKILL.md`.

## License

MIT © the sparq authors.
