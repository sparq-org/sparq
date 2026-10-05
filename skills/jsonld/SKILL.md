---
name: jsonld
description: Parse, expand, flatten, compact, and frame W3C JSON-LD 1.1 with sparq — the native, dependency-free document-level pipeline in the `sparq-jsonld` crate (expand/flatten/compact/frame/fromRdf over a JSON AST, deny-by-default document loader) and how each surface exposes it (native CLI `dump … jsonld[-compact] --context/--frame`, the HTTP server's `application/ld+json` content-negotiation, the Solid/LWS server's profile-aware expanded/compacted negotiation, the wasm `serializeCompact`, the engine's RDF-first writer matrix). Use when converting JSON-LD document forms, choosing a surface, or reasoning about the honest conformance / remote-loading posture.
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq JSON-LD 1.1

How to move a JSON-LD 1.1 document between its four output forms — **expanded**,
**flattened**, **compacted**, **framed** — and how each sparq surface exposes them.

There are **two** JSON-LD code paths, and it is load-bearing to keep them apart:

1. **The native document-level pipeline** — the `sparq-jsonld` crate. A from-scratch,
   dependency-free implementation of the W3C JSON-LD 1.1 API + Framing algorithms that
   operates on JSON trees (`expand`, `flatten`, `compact`, `frame`, `from_rdf`). This is
   the Rust API you call directly; it is where document-form conversion lives.
2. **The engine's RDF-first writer matrix** — `sparq_engine::serialize::*` behind the
   `serialize-rdf` feature. sparq always holds RDF, so the CLI / server / wasm surfaces
   currently serialise **from a `Graph`** through this writer (see
   `skills/data-formats/SKILL.md`, recipe 6). Wiring the surfaces onto the native
   pipeline is a later, tracked step (bead `sq-oy1f.41`) — until it lands, "what the CLI
   emits" and "what `sparq_jsonld::compact` produces" are different code, so this skill
   says which is which for every surface.

> This document was verified against the source on branch `main` (2026-07-13,
> `[GPT-5.6]`). Every function, flag, and conformance number below exists today; where a
> form is *not* exposed on a surface, it is called out as planned, not implied.

## Native pipeline — the `sparq-jsonld` crate

`sparq-jsonld` is `#![forbid(unsafe_code)]`, has **zero mandatory dependencies**, and
is `publish = false` (an internal crate, path-depended). It has **no cargo feature of
its own** — it is always compiled when a crate depends on it, so there is no
"jsonld-off" build of *this* crate; the opt-in gating lives in the consumers (below).

The value type is the crate's own tiny `Json` AST (no `serde_json`). Parse with
`Json::parse(&str)`, render with `value.write(&mut String)`.

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

### Runnable end-to-end example

The canonical demo is `examples/jsonld_roundtrip.rs` — a full expand → flatten →
compact → frame pipeline over an inline document that asserts the framed output
round-trips. Run it:

```sh
cargo run -p sparq-jsonld --example jsonld_roundtrip
```

Its shape (mirroring the example verbatim):

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

The **reverse** direction, `to_rdf` (native JSON-LD → RDF), is **not yet implemented**
in this crate — the `to_rdf` module is a documented stub. To go JSON-LD → RDF today, use
the CLI/server `jsonld` ingest (the `oxjsonld` parser) or `Graph::load_str(doc,
"jsonld")` (see `skills/data-formats/SKILL.md`).

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
exact.

### Document loading is deny-by-default

Remote `@context` / `@import` / remote-document references are dereferenced **only**
through a `DocumentLoader`. The default `NoopLoader` **refuses every load** and raises
`JsonLdErrorCode::LoadingDocumentFailed`, so merely enabling JSON-LD grants a surface
**no ambient network**. `FsLoader` maps URL prefixes to **trusted local fixtures** only
(never attacker-supplied URLs). A network-fetching loader with an SSRF allowlist is a
planned opt-in (`sq-oy1f.32`) and is **not** in the tree yet — do not assume remote
context resolution works today.

```rust
use sparq_jsonld::{DocumentLoader, JsonLdErrorCode, NoopLoader};
let err = NoopLoader.load_document("https://ex/ctx").unwrap_err();
assert_eq!(err.code(), JsonLdErrorCode::LoadingDocumentFailed);
```

## Surfaces — which form each one exposes

The native pipeline is not yet wired to the surfaces (`sq-oy1f.41`); the surface columns
below serialise **from a `Graph`** through the engine's RDF-first writer, except where
noted. Source of truth: `research/jsonld-interop-matrix.md`.

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

[FABLE-5] The experimental Solid/LDP server serialises RDF resource reads through the
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
suites through six ratcheted lanes. Each floor is a **MEASURED minimum pass count at the
pinned suite revision**, may only rise, and **is not a blanket "conformant" claim** —
below-floor cases are honest, documented divergences, and the denominators include those
failures plus intentional skips (negatives, JSON-LD-1.0-only, non-inline/remote context):

| Lane    | Measured floor | Oracle |
| ------- | -------------: | ------ |
| toRdf   | 413 / 467      | oxjsonld RDF-dataset comparison |
| fromRdf | 52 / 53        | native document comparison + RDF round-trip |
| expand  | 276 / 385      | native document-level comparison |
| flatten | 53 / 58        | native document-level comparison |
| compact | 228 / 246      | native normative document comparison |
| frame   | 92 / 92        | native normative document comparison (incl. negatives) |

The authoritative constants live in
`crates/sparq-conformance/src/floors/{to_rdf,from_rdf,expand,flatten,compact,frame}.rs`
(each `pub const FLOOR: usize`). They measure **algorithm lanes**, not HTTP negotiation,
CLI option coverage, Python bindings, wasm bundle contents, or GUI controls. A full lane
(e.g. `frame` at its pinned revision) does **not** elevate the whole surface into an
unqualified conformance claim. Remote-document loading (`sq-oy1f.32`) and HTML script
extraction (`sq-oy1f.33`) remain separate, not-yet-implemented lanes and are never
silently counted here. Reproduce:

```sh
scripts/fetch-jsonld-tests.sh && scripts/fetch-jsonld-framing-tests.sh
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
