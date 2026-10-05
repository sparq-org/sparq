<!-- [OPUS-4.8] sq-inzv: full-template README — the published @sparq-org/sparq browser/WASM bundle. -->
# sparq-wasm

The sparq parser + triplestore + SPARQL engine compiled to WebAssembly for the
browser (and Node). Same store and engine as the native build — including the
worst-case-optimal Leapfrog Triejoin for cyclic queries — single-threaded and
with a minimal bundle (no rayon, no serde; results are serialised by hand to
SPARQL 1.1 JSON).

> Distributed via npm, not crates.io (`publish = false`). It is the source of the
> published npm `@sparq-org/sparq` bundle, packaged via `wasm-pack`, not a Rust library
> dependency.

## 🚀 Quickstart

```sh
wasm-pack build --target web --profile release-wasm            # browser (ES modules)
wasm-pack build --target nodejs --profile release-wasm --out-dir pkg-node   # Node (CommonJS)
```

```js
import init, { Store } from "./pkg/sparq_wasm.js";
await init();

const store = Store.load(turtleText, "turtle"); // "ntriples" | "nquads" | "trig" | "jsonld"*
const empty = new Store();                       // empty + mutable; build up via updateInPlace
const based = Store.loadWithBase(doc, "turtle", "http://ex/dir/"); // resolve relative IRIs
store.size;            // number of triples
store.heapBytes();     // rough in-memory footprint

const json = store.query("SELECT * WHERE { ?s ?p ?o } LIMIT 10");
const { head, results } = JSON.parse(json);       // SPARQL 1.1 JSON results
const n = store.count("SELECT ?s WHERE { ?s a <http://ex/Person> }"); // lazy, no materialise
```

## ✨ Features

- **SPARQL 1.1 query surface** — BGPs (binary + WCOJ), FILTER, OPTIONAL, UNION,
  MINUS, BIND, VALUES, aggregation (GROUP BY / HAVING), ORDER BY,
  DISTINCT/LIMIT/OFFSET, sub-SELECT. Every query *form* is exported: SELECT/ASK via
  `query` / `ask` (plus streaming `queryCursor` / `queryChunks`) and CONSTRUCT /
  DESCRIBE via `queryQuads` / `queryQuadsChunks` (N-Triples out). The non-regex
  string functions (`CONTAINS`, `STRSTARTS`, `LCASE`, …) share one
  `sparq-engine`/`sparq-core` with native, so they behave identically.
- **`REGEX` / `REPLACE` are compiled OUT of the lean default bundle** — they live
  behind `sparq-engine`'s default-on `regex` feature, which this crate disables to
  keep the regex automata out of the browser bundle, so on a default build a
  `REGEX`/`REPLACE` query is **rejected** (an `"unsupported SPARQL function"`
  `JsError`, not a silently-empty result). Build with `--features regex`, or prefer
  `CONTAINS` / `STRSTARTS` / `STRENDS`.
- **Opt-in JSON-LD, SHACL, lazy counts.** `"jsonld"` parsing links `oxjsonld` and is **OFF by default** to keep the
  lean bundle small (`--features jsonld`). The `shacl` feature (also OFF) exposes the stateless
  `Store.validate(data, shapes, format)` (a drop-in for `rdf-validate-shacl`) and the store-backed
  `Store.validateStore(shapes, format)`, which validates the triples the store ALREADY holds (repeat validation
  re-parses only the shapes). The `count()` family is **lazy** — counted from the sorted indexes, no per-row work.
- **Opt-in serialiser.** The `serialize-rdf` feature (OFF by default) exposes
  `Store.serialize(format, pretty, indent, abbreviate, prefixes?)` — the store's contents
  as a **Turtle** (default graph), **TriG** (whole dataset), or **JSON-LD** (whole dataset)
  document, in the engine's pretty (Turtle/TriG: sorted, blank-line-separated; JSON-LD:
  re-indented) shape or the compact/minified writer. JSON-LD `format` is `"jsonld"`
  (expanded by default) or `"jsonld-expanded"` / `"jsonld-flattened"` /
  `"jsonld-compacted"`; `abbreviate` is Turtle/TriG-only (JSON-LD compaction is the
  `jsonld-compacted` form). The optional `prefixes` argument is a `[[prefix, iri], …]` JS
  array: when omitted the engine's well-known defaults are used (byte-for-byte the prior
  behaviour); when supplied it drives Turtle/TriG `@prefix` compaction and the JSON-LD
  compacted `@context`, so a caller can serialise under its own prefix policy (e.g. the
  site's `COMMON_PREFIXES` with `https://schema.org/`, or a query's declared `PREFIX`
  lines) and get byte-parity output. It calls straight through to `sparq-engine`'s writers,
  so the output is byte-identical to the native serialiser; the lean bundle carries no
  serializer code. JSON-LD serialise-OUT needs only `serialize-rdf` (the `jsonld` feature is
  for INGEST). The sibling `Store.serializeCompact(context, pretty, indent?)` (same feature)
  runs the **full W3C JSON-LD 1.1 Compaction Algorithm** against a caller `@context` JSON
  string (term defs / `@vocab` / coercion / `@reverse`) — richer than the prefix-only
  `jsonld-compacted` form, and lossless; a non-object `context` throws.
- **Opt-in SHACL Compact Syntax parse.** The `scs` feature (OFF by default; implies
  `shacl` + `serialize-rdf`) exposes `Store.parseShaclCompact(text, base?)` — parses a
  [SHACL Compact Syntax](https://www.w3.org/TR/shacl12-compact-syntax/) document into the
  equivalent SHACL **shapes graph** and returns it as a pretty **Turtle** string (the SCS
  *input* direction for the playground's "Compact → shapes" mode). It REUSES the
  `Store.serialize` engine writer above (no second serialiser), so the bytes match
  `serialize("turtle", true, "  ", true)`; the parsed shapes validate data identically to
  the equivalent Turtle shapes. A malformed document throws a `JsError` with the source line.
- **Opt-in ODRL policy probe** (`policy`, OFF by default; EXPERIMENTAL, unpublished): exposes
  `policyEvaluate` / `policyConflicts` over `sparq-policy`'s fail-closed evaluator (sq-586sh).
- **Persistence is native-only** — the native `Graph::save` / `open` /
  `save_compressed` family and the mmap-backed map-in path are **deliberately not
  exported**: they need a POSIX filesystem and `mmap`, which a browser/edge wasm
  sandbox does not provide. A wasm store is built fresh each session from in-memory
  bytes (`Store.load` / `loadCompressed`); to round-trip contents back to the host,
  `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` through `queryQuads`. There is **no
  binary snapshot format across the wasm boundary**.
- **Single-threaded, append-only growth, 32-bit ceiling.** No `rayon` (wasm has no
  threads here). `updateInPlace` / `applyDelta` grow the indexes and dictionary
  **append-only** (deletes are masked, not reclaimed), so steady writing
  monotonically grows the footprint until a rebuild (`update` returns a fresh
  compacted store). `wasm32` linear memory caps at 4 GiB (a real tab is happier
  under ~2 GiB) — that, not CPU, is the binding scale limit.
- **`compact-index` for the memory-constrained target.** The wasm build keeps only
  three permutations (SPO, POS, OSP) instead of six; `loadCompressed` block-compresses
  them and compacts the dictionary so materially more triples fit in the same tab,
  for a small per-scan decode (identical results). Correctness is held to the native
  bar via differential fuzzing vs Oxigraph.

## 📚 Learn more

- **How-to** — [`skills/javascript-wasm/SKILL.md`](../../skills/javascript-wasm/SKILL.md).
- **SHACL binding** — [`skills/shacl-validation/SKILL.md`](../../skills/shacl-validation/SKILL.md)
  and the standalone showcase bundle [`sparq-shacl-wasm`](../sparq-shacl-wasm/README.md).
- **Performance** — bundle size (`wasm_bundle_*`) and store/dict footprint
  (`store_bytes_per_triple` / `dict_bytes_per_term`) are tracked per-commit on the
  [benchmarks dashboard](https://sparq.jeswr.org/dev/bench), not in docs.
  The `test/perf.cjs`, `test/mem.cjs`, and `test/smoke.cjs` harnesses reproduce them.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

\* `"jsonld"` (and `"json-ld"` / `"application/ld+json"`) is parsed only with the
opt-in `jsonld` feature; Turtle / N-Triples / N-Quads / TriG need no feature.

## License

[MIT](../../LICENSE).
