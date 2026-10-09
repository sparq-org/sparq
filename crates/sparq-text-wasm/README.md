<!-- sq-inzv: full-template README — tier-b W-text WASM showcase bundle. -->
# sparq-text-wasm

**The tier-b "W-text" WebAssembly bundle** (sq-jbe6) for
[`sparq-text`](../sparq-text/README.md) — an owned **BM25 full-text index** over RDF
literals plus the `text:` magic predicates, **live in the browser tab**.

This is a SEPARATE, lazy-loaded bundle from the lean [`sparq-wasm`](../sparq-wasm/README.md)
triplestore bundle. The lean bundle deliberately carries **no** text-search code; the
showcase site's `/surface/full-text` page loads this bundle on demand
(`next/dynamic`, client-only) so the landing page stays light. It mirrors the
per-bundle-crate pattern of `sparq-wasm` / [`sparq-reason-wasm`](../sparq-reason-wasm/README.md).

> Not on crates.io (`publish = false`) and not a separate npm package: the site builds
> it with `js/`'s `build:text-wasm` and serves it. It is a wasm packaging
> layer over `sparq-text`, built via `wasm-pack`, not a Rust library dependency.

## 🚀 Quickstart

```sh
wasm-pack build crates/sparq-text-wasm --target web --release
```

```js
import init, { TextSearch } from "./sparq_text_wasm.js";
await init();

const data = `
  <http://ex/a> <http://ex/comment> "The quick brown fox" .
  <http://ex/b> <http://ex/comment> "A lazy dog" .`;

// text:matches (AND of tokens) + text:score, BM25-ranked, as SPARQL-results JSON:
const json = TextSearch.query(data, "ntriples",
  `PREFIX text: <http://sparq.dev/text#>
   SELECT ?s ?score WHERE {
     ?s <http://ex/comment> ?lit .
     ?lit text:matches "quick fox" ; text:score ?score .
   } ORDER BY DESC(?score)`);
// => binds only <http://ex/a> (it has both tokens).

// Index footprint, as JSON:
const { docs, tokens, heapBytes, hasPositions } =
  JSON.parse(TextSearch.indexStats(data, "ntriples"));
```

## ✨ Features

- **Stateless one-shot entry points.** Each call parses the document, builds the
  index over it, runs the request, and returns a string — the JS side never holds a
  long-lived index handle. `format` is one of `"turtle"`, `"ntriples"`, `"nquads"`,
  `"trig"` (named graphs folded into the default graph, so the index covers every
  string literal in the document).
- **Full `text:` vocabulary.** The index is **always built with positions**, so all of
  `http://sparq.dev/text#` works: `text:matches` (AND of tokens, `foo*` is a prefix
  token), `text:matchesAny` (OR), `text:phrase` (adjacent and in order), `text:near`
  (+ optional `text:slop N`, proximity-ranked), and `text:score` (the relevance score
  of a scored match). A query with **no** `text:` patterns runs unchanged — the bundle
  is a superset of plain SPARQL querying.
- **Reuses the lean engine.** The `text:` rewrite happens entirely inside
  `sparq-text` (it splices index hits into the query as inline `VALUES`); the SPARQL
  engine — planner, executor — is unaware of text search, which is exactly why this
  bundle can reuse the same engine the lean bundle ships. Output for `query` is
  canonical SPARQL 1.1 JSON via the engine's tested `to_sparql_json` path; this bundle
  owns no result-serialisation code of its own.
- **Single-threaded + pure-Rust.** No `rayon`. The tokenizer's `unicode-segmentation`
  dependency (UAX #29 word segmentation) compiles to `wasm32-unknown-unknown` with
  zero transitive deps. The **index-build memory** is the consideration the design
  flags — the inverted index plus the positional postings are held fully in memory —
  which is why this bundle is lazy-loaded on its page only, runs over a small demo
  corpus, and exposes `indexStats` so the page can show the footprint.

## 📚 Learn more

- **How-to** — [`skills/full-text-search/SKILL.md`](../../skills/full-text-search/SKILL.md).
- **Performance** — we deliberately quote **no** hard-coded byte/MB figure here
  (bundle size drifts with the toolchain and dependency versions). The gzip transfer
  size — what end users actually download, since the site serves the `.wasm`
  gzip-compressed — is reproducible per toolchain:

  ```sh
  wasm-pack build crates/sparq-text-wasm --target web --release
  f=crates/sparq-text-wasm/pkg/sparq_text_wasm_bg.wasm
  echo "pre-gzip: $(stat -c%s "$f") bytes   gzip -9: $(gzip -9 -c "$f" | wc -c) bytes"
  ```

- **Status** — this crate delivers the wasm-compatibility changes, the `TextSearch`
  entry points (`query` + `indexStats`), and a headless `wasm-pack test --node` smoke
  suite. The Pages workflow builds it (`npm run build:text-wasm`) and the site's
  `sync-wasm` script serves it on `/surface/full-text`; it has no npm package of its own.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
