---
name: jsonld
description: Parse, expand, flatten, compact, and frame W3C JSON-LD 1.1 with sparq — the native document-level pipeline in the `sparq-jsonld` crate (expand/flatten/compact/frame/toRdf/fromRdf over a JSON AST, deny-by-default document loader) and how each surface exposes it (native CLI `dump … jsonld[-compact] --context`; CLI framing is planned, the HTTP server's `application/ld+json` content-negotiation, the Solid/LWS server's profile-aware expanded/compacted negotiation, the wasm `serializeCompact`, the engine's RDF-first writer matrix). Use when converting JSON-LD document forms, choosing a surface, or reasoning about the honest conformance / remote-loading posture.
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq JSON-LD 1.1

How to move a JSON-LD 1.1 document between its four output forms — **expanded**,
**flattened**, **compacted**, **framed** — and how each sparq surface exposes them.

JSON-LD document processing, RDF writing and Graph ingest are distinct surfaces:

1. **The native document-level pipeline** — the `sparq-jsonld` crate implements the
   W3C JSON-LD 1.1 API + Framing algorithms with zero mandatory dependencies. It
   operates on JSON trees (`expand`, `flatten`, `compact`, `frame`, `to_rdf`, `from_rdf`).
   This is the Rust API you call directly for document-form conversion.
2. **The engine's RDF-first writer matrix** — `sparq_engine::serialize::*` behind the
   `serialize-rdf` feature. sparq always holds RDF, so the CLI / server / wasm surfaces
   serialise **from a `Graph`** through this writer (see
   `skills/data-formats/SKILL.md`, recipe 6). The engine-serialize cutover (bead
   `sq-oy1f.41`) has joined the two paths for the **full-algorithm** forms:
   `graph_to_jsonld_compact` / `graph_to_jsonld_framed` are now thin `from_rdf ∘ compact` /
   `from_rdf ∘ frame` adapters over the native pipeline, so "what the CLI emits" for
   `jsonld-compact` and "what `sparq_jsonld::compact` produces" are the same code. The
   `JsonLdForm::{Expanded, Flattened, Compacted}` writer (the prefix-only `@context` form) is
   still the engine's own, so this skill says which is which for every surface.
3. **Graph JSON-LD ingest** — `sparq-core` uses oxjsonld when loading JSON-LD.
   It does not call native `sparq_jsonld::to_rdf`; the native API conformance count
   does not describe `Graph::load_*(..., "jsonld")` behavior.

## Native pipeline — the `sparq-jsonld` crate

`sparq-jsonld` is `#![forbid(unsafe_code)]`, has **zero mandatory dependencies**, and
is published to crates.io as part of `sparq-engine`'s dependency closure.
Its `html` feature is **off by default** and adds HTML/XHTML script extraction using
html5ever and quick-xml. JSON document processing has no mandatory dependencies.
Consumer features below control whether they link this crate.

The value type is the crate's own tiny `Json` AST (no `serde_json`). Parse with
`Json::parse(&str)`, render with `value.write(&mut String)`. `Json::parse` rejects arrays
and objects nested deeper than `json::MAX_DEPTH` with a `JsonParseError`, so the recursive
walks over a parsed value stay within the stack.

### The four operations

Every entry point takes the processing `options` and a `loader` (deny-by-default —
see below), and returns `Result<Json, JsonLdError>`:

```rust
use sparq_jsonld::{expand, flatten, Json, JsonLdOptions, NoopLoader};
use sparq_jsonld::compact::compact;
use sparq_jsonld::frame::{frame, FrameOptions};

let options = JsonLdOptions::default();   // the spec defaults (processingMode = json-ld-1.1)
let loader = NoopLoader;                   // no ambient network

// EXPAND — compact document -> canonical expanded array of node objects.
let expanded = expand(&input, &options, &loader)?;

// FLATTEN — expand, then node-map + fold named graphs, sorted by @id (an array).
let flattened = flatten(&input, &options, &loader)?;

// COMPACT — expand `input`, then compact the expanded form against a caller `context`
// (the context is embedded under @context in the result unless it is empty).
let compacted = compact(&input, &context, &options, &loader)?;

// FRAME — expand input + frame, run frame matching, compact against the frame's @context.
let framed = frame(&input, &frame_doc, &options, &FrameOptions::default(), &loader)?;
```

Signatures (exact, from `crates/sparq-jsonld/src/`):

```rust
pub fn expand(input: &Json, options: &JsonLdOptions, loader: &dyn DocumentLoader)
    -> Result<Json, JsonLdError>;
pub fn flatten(input: &Json, options: &JsonLdOptions, loader: &dyn DocumentLoader)
    -> Result<Json, JsonLdError>;
pub fn compact::compact(input: &Json, context: &Json, options: &JsonLdOptions,
    loader: &dyn DocumentLoader) -> Result<Json, JsonLdError>;
pub fn frame::frame(input: &Json, frame_doc: &Json, options: &JsonLdOptions,
    frame_options: &FrameOptions, loader: &dyn DocumentLoader) -> Result<Json, JsonLdError>;
```

If you already hold the expanded form, skip re-expansion with the `_expanded` variants:
`flatten_expanded(&expanded) -> Json` (infallible), `compact::compact_expanded(&expanded,
&context, &options, &loader)`, and `frame::frame_expanded(&expanded_input,
&expanded_frame, &options, &frame_options)`.

`frame::frame` is also split into its two halves, so a caller can hold or inspect the framed
expanded form before compacting it: `frame::frame_match(input, frame_doc, options,
frame_options, loader)` returns the framed **expanded** output, and
`frame::compact_framed(&framed, &context, options, frame_options, loader)` compacts it. When the
input is already `from_rdf` output, `frame::frame_match_from_rdf(from_rdf, frame_doc, options,
frame_options, loader)` takes it by value and regroups its graph map directly instead of
re-expanding and re-running node map generation, keeping its blank-node labels.

### End-to-end composition

The four operations compose as a full expand → flatten → compact → frame pipeline over
an inline document (the crate ships no stand-alone demo example; its only example,
`jsonld_conformance`, is the W3C suite scoreboard runner):

```rust
let input = Json::parse(DOCUMENT).expect("valid JSON");
let context = Json::parse(CONTEXT).expect("valid JSON");
let frame_document = Json::parse(FRAME).expect("valid JSON");
let options = JsonLdOptions::default();
let loader = NoopLoader;

let expanded = expand(&input, &options, &loader)?;
let flattened = flatten(&expanded, &options, &loader)?;   // expanded is already an array
let compacted = compact(&flattened, &context, &options, &loader)?;
let framed = frame(&compacted, &frame_document, &options, &FrameOptions::default(), &loader)?;
```

### Serialize RDF as JSON-LD (`fromRdf`)

`from_rdf::from_rdf` turns an RDF dataset into an expanded JSON-LD document. Its RDF
model is crate-local and dependency-free (`RdfTerm` / `RdfQuad`, constructed via
`RdfTerm::iri` / `blank` / `literal` / `typed_literal` / `lang_literal`), configured by
`FromRdfOptions` (`useNativeTypes` / `useRdfType`, `rdfDirection`). These are reached
via the module path, not a lib re-export:

```rust
use sparq_jsonld::from_rdf::{from_rdf, FromRdfOptions, RdfQuad, RdfTerm};
let expanded = from_rdf(&quads, &FromRdfOptions::default())?;
```

For native JSON-LD → RDF, call `to_rdf::to_rdf(&input, &options, &loader)`.
It returns `Result<Vec<RdfQuad>, JsonLdError>`, using the same crate-local RDF model.
It expands first, validates conflicting indexes, and converts named graphs, lists,
JSON literals and scalar values. `rdf_direction` supports both direction encodings;
`produce_generalized_rdf` defaults to false and enables blank-node predicates when true.
Blank-node subjects, objects, types and ordinary/reverse predicates share the same
identifier issuer, preserving their identity when relabeled.
The CLI/server and `Graph::load_str(doc, "jsonld")` continue to use oxjsonld.

For compacted flattening, call
`flatten::flatten_with_context(&input, Some(&context), &options, &loader)`.
It retains the graph envelope while honoring property-level `compact_arrays`.
`None` or a JSON null context returns expanded flattened output. `flatten` and
`to_rdf` reject conflicting indexes with `conflicting indexes`; framing does too.
`node_map::try_generate_node_map(&expanded)` exposes that validation directly.
The existing infallible `generate_node_map` and `flatten_expanded` helpers retain
no error return; use the fallible operations when validating documents.

### Options and errors

`JsonLdOptions::default()` carries the specification defaults: `processing_mode =
JsonLd11`, `compact_arrays = true`, `compact_to_relative = true`, `ordered = false`,
`frame_expansion = false`, and the framing flag defaults (`embed = Once`, `explicit =
false`, `omit_default = false`, `require_all = false`). Adjust the fields you need
(`base`, `expand_context`, `rdf_direction`, etc.). Framing-only knobs live on
`FrameOptions` (`omit_graph`, `prune_blank_node_identifiers`, `frame_default`) — `None`
resolves to the spec default for the active processing mode.

Every operation returns a `JsonLdError` carrying a `JsonLdErrorCode` — the full W3C
error-code registry, whose `as_str()` is the exact spec string (e.g. `"invalid frame"`,
`"invalid @embed value"`, `"loading document failed"`), so negative-test assertions are
exact. A selected compact IRI may validly have the original IRI spelling; prefix
confusion in JSON-LD 1.1 mode is checked only after term, vocabulary and compact-IRI
selection fail, before base-relative compaction. JSON-LD 1.0 compatibility mode
retains the legacy behavior without that error, including a property whose IRI
spelling becomes a compact IRI under a frame's context.

### Document loading is deny-by-default

Remote `@context` / `@import` / remote-document references are dereferenced **only**
through a `DocumentLoader`. The default `NoopLoader` **refuses every load** and raises
`JsonLdErrorCode::LoadingDocumentFailed`, so merely enabling JSON-LD grants a surface
**no ambient network**. `FsLoader` maps URL prefixes to **trusted local fixtures** only
(never attacker-supplied URLs). A network-fetching loader with an SSRF allowlist is a
planned opt-in (`sq-oy1f.32`) and is **not** in the tree. Applications supply their
own loader for remote documents and contexts; the processor has no HTTP client.
An absolute IRI carried as `Json::Str` is loaded as a document by `expand`, and by
operations that expand their input. Non-IRI string scalars still expand to an empty array.

`RemoteDocument` carries raw `document` text, final `document_url`, `content_type`,
`context_url`, and optional `profile`. A loader performs redirects, reports HTTP failures,
selects alternate links, and rejects multiple context link headers with
`multiple context link headers`. The processor accepts `application/json`, JSON media
types ending in `+json`, and a missing content type as JSON-LD for compatibility.
Unsupported types and invalid JSON raise `loading document failed`. Context links apply
to JSON other than `application/ld+json`; they are ignored for JSON-LD and HTML/XHTML.
Relative links and remote contexts resolve against the final document URL, even when
`options.base` overrides the base used for node identifiers.

With `features = ["html"]`, loaded `text/html` and `application/xhtml+xml` documents
support script extraction. The default selects the first JSON-LD script;
`extract_all_scripts = true` merges all scripts, flattening their outer arrays.
A percent-decoded URL fragment selects the matching script `id` regardless of that flag.
Missing targets, wrong element types, and no script with the default option raise
`loading document failed`. Invalid script JSON raises `invalid script element`;
legacy HTML comment wrappers are not stripped. HTML character references inside script
text remain literal; XHTML uses XML character references and CDATA.
The first `<base href>` resolves against the final document URL (or explicit base
fallback) and supplies the base for extracted content. See
[loader and HTML examples](references/loading.md).

```rust
use sparq_jsonld::{DocumentLoader, JsonLdErrorCode, NoopLoader};
let err = NoopLoader.load_document("https://ex/ctx").unwrap_err();
assert_eq!(err.code(), JsonLdErrorCode::LoadingDocumentFailed);
```

## Surfaces — which form each one exposes

Every surface below serialises **from a `Graph`** through the engine's RDF-first writer, except
where noted; for the full-Compaction and Framing forms that writer is itself an adapter over the
native pipeline (`sq-oy1f.41`, above). Source of truth: `research/jsonld-interop-matrix.md`.

### Native CLI (`sparq-cli`, `jsonld` default-on)

The `jsonld` feature is **on by default** in the CLI binary (a maintainer-directed
exception, `sq-oy1f.4`), so JSON-LD read/write works out of the box; drop it with
`--no-default-features`. Re-serialise a loaded document with `dump`:

```sh
# out-format: turtle | turtle-pretty | trig | trig-pretty | nquads | ntriples
#           | jsonld[-expanded|-flattened|-compacted]
#           | jsonld-pretty[-expanded|-flattened|-compacted]
#           | jsonld-compact[-pretty]   (FULL W3C Compaction; needs --context <ctx.jsonld>)
sparq-cli dump data.ttl turtle jsonld-flattened
sparq-cli dump data.ttl turtle jsonld-compact --context ctx.jsonld
```

Bare `jsonld` == `jsonld-expanded`. `jsonld-compacted` is the *light* prefix-only
`@context` (CURIE abbreviation); `jsonld-compact --context` runs the **full** W3C
Compaction Algorithm against your own context. Framing on the CLI (`jsonld-framed
--frame`) is planned (`sq-oy1f.42`), not yet a CLI out-format.

### HTTP server (`sparq-server`, `jsonld` default-on)

The server negotiates `application/ld+json` (gated on the `jsonld` feature, which the
server default build enables). A CONSTRUCT/DESCRIBE or Graph Store read requested as
`application/ld+json` returns the engine's **flattened** JSON-LD serialisation — that is
the only verified wire form today. Profile-parameterised negotiation for the expanded /
compacted / framed forms (`Accept: application/ld+json;profile=…`) is planned
(`sq-oy1f.34`); it is **not** a current server capability. See
`skills/http-server/SKILL.md` for content negotiation generally.

### Solid/LWS server (`sparq-lws-core`, experimental)

The experimental Solid/LDP server serialises RDF resource reads through the
vendored oxjsonld writer (neither the native pipeline nor the engine writer) and —
unlike `sparq-server` — already honours the JSON-LD `profile` media-type parameter on
an explicit `Accept: application/ld+json;profile="…"` range, for the LDP and identity
read paths. The honoured profile is echoed back quoted in the response `Content-Type`
(`NegotiatedFormat::content_type`; `JsonLdProfileParam::iri` pins the canonical IRIs).
`expanded` is honoured byte-identically — the serialiser's default output IS the
expanded document form — while `compacted` applies a local, **context-free** structural
compaction over that output (`serialize_triples_negotiated` in
`sparq_lws_core::ldp::content`): no context is used or fetched, preserving the crate's
no-remote-context SSRF posture. This is a document-form step scoped to the server's own
serialiser shape, **not** the full W3C Compaction Algorithm (that lives in
`sparq-jsonld`). An honoured profile never serves stored bytes verbatim and derives a
profile-specific variant ETag exactly when the bytes differ
(`NegotiatedFormat::serves_stored_verbatim` / `variant_suffix`).

### wasm / npm (`@sparq-org/sparq`, JSON-LD opt-in)

The lean wasm bundle keeps JSON-LD **opt-in** (the `jsonld` / `serialize-rdf` cargo
features on `sparq-wasm`) so the default browser byte-floor stays small. When built with
it, the wasm `Store` exposes `serializeCompact(context, pretty, indent?)` — byte-identical
to the engine's `graph_to_jsonld_compact`. The expanded / flattened / compacted forms
come from the engine writer via the store's serialize methods; a wasm `serializeFramed`
is **not** implemented (`sq-oy1f.44`). See `skills/javascript-wasm/SKILL.md`.

### Python (`sparq-rdf`, JSON-LD ingest default-on)

The Python wheel enables JSON-LD **ingest** by default (`sq-oy1f.20`); the document-form
**output** operations are not yet a Python surface (`sq-oy1f.43`).

## Honest conformance status

sparq drives the official `w3c/json-ld-api` and the separate `w3c/json-ld-framing`
suites through eight API ratcheted lanes plus a separate Graph ingest ratchet.
Floors are measured minimum pass counts at the pinned revision and may only rise.
The JSON-LD EARL report measures JSON-LD 1.1 API operations via `sparq-jsonld`
(expand, compact, flatten, frame, toRdf, fromRdf). `sparq-core`'s Graph JSON-LD
ingest uses oxjsonld and is not what EARL measures. These counts do not measure
HTTP negotiation, CLI flags, Python bindings, or wasm bundle contents.

| Lane | Measured floor | Oracle |
| ---- | -------------: | ------ |
| toRdf | 454 / 467 | native RDF dataset comparison for 1.1; legacy oxjsonld for 1.0-only |
| fromRdf | 52 / 53 | native document comparison + RDF round-trip |
| expand | 381 / 385 | native document comparison |
| flatten | 55 / 58 | native document comparison, including context and negative cases |
| compact | 244 / 246 | native normative document comparison |
| frame | 92 / 92 | native normative document comparison |
| html | 49 / 50 | native expand/compact/flatten/toRdf with HTML/XHTML extraction |
| remote-doc | 18 / 18 | native expansion through virtual manifest responses |
| toRdf-ingest (excluded from EARL) | 429 / 467 | Graph acceptance + oxjsonld canonical dataset; base only; rejection-only negatives |

`TOTAL toRdf-ingest 429 38 0` covers the same 467 toRdf manifest entries, with a
pass floor of 429 and failure ceiling of 38. It passes the raw input and base to
`parse_jsonld_dataset`: Graph must accept the input, then oxjsonld supplies the
canonical dataset preserving named graphs. Unsupported options and remote loading
are not emulated. All entries execute, including generalized-RDF cases that the
oxrdf dataset model cannot represent. Negative cases check rejection without an
exact W3C error code. This ratchet never contributes EARL assertions. The native
toRdf lane's 454 passes include legacy 1.0-only compatibility entries; its EARL
subset is 448 / 448, within the API report's 1295 / 1296 applicable entries.

The authoritative constants are in `crates/sparq-conformance/src/floors/`.
EARL excludes only JSON-LD 1.0-only entries. All other unexecuted entries fail;
HTML `#tr006` remains a reported failure because its expected empty dataset contradicts
Process HTML step 4's required loading error when no script exists and
`extractAllScripts` is false. The HTML failure ceiling is one; remote-doc's is zero.
The in-memory manifest loader maps suite URLs to local files and models media types,
status, redirects, context links and alternates without sockets or network access.
Reproduce after fetching the pinned suites:

```sh
cargo test -p sparq-conformance --features jsonld-suite --test jsonld_suite
```

## See also

- `skills/data-formats/SKILL.md` — the engine's RDF-first writer matrix (recipe 6),
  `Graph::load_str(doc, "jsonld")` ingest, and pretty JSON-LD.
- `research/jsonld-interop-matrix.md` — the evidence-indexed, surface-by-surface
  landed-state inventory (the source of truth for the surface table above).
- `research/jsonld-1.1-design.md` — the document-level pipeline design record (epic
  `sq-oy1f`).
- JSON-LD 1.1 API: <https://www.w3.org/TR/json-ld11-api/> · Framing:
  <https://www.w3.org/TR/json-ld11-framing/>
