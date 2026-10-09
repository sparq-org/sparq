<!-- sq-inzv: full-template README — tier-b W-shacl WASM showcase bundle. -->
# sparq-shacl-wasm

**The tier-b "W-shacl" WebAssembly bundle** (sq-lfmf) for
[`sparq-shacl`](../sparq-shacl/README.md) — SHACL Core + SHACL-SPARQL (`sh:sparql`,
W3C SHACL §5.2) **validation, live in the browser tab**.

This is a SEPARATE, lazy-loaded bundle from the lean [`sparq-wasm`](../sparq-wasm/README.md)
triplestore bundle. The lean bundle deliberately carries no SHACL on the landing
page; the showcase site's `/surface/shacl` page loads this bundle on demand
(`next/dynamic`, client-only). It mirrors the per-bundle-crate pattern of
[`sparq-reason-wasm`](../sparq-reason-wasm/README.md).

> Distributed via npm, not crates.io (`publish = false`). It is a wasm packaging
> layer over `sparq-shacl`, built via `wasm-pack`, not a Rust library dependency.

The lean `sparq-wasm` bundle also exposes a minimal `Store.validate(...)` behind its
non-default `shacl` feature (sq-yqi1, #162) for the PSS ADR-0014 `ShaclValidator`
seam. This standalone bundle is the **showcase** artifact: a stateless `Validator`
with the FULL report surface — JSON, W3C-vocabulary report Turtle, plain text, and a
severity-filtered conformance flag.

## 🚀 Quickstart

```sh
wasm-pack build crates/sparq-shacl-wasm --target web --release
```

```js
import init, { Validator } from "./sparq_shacl_wasm.js";
await init();

// The W3C validation report as JSON ({ conforms, results: [...] }):
const report = JSON.parse(Validator.validate(dataTurtle, shapesTurtle, "turtle"));
// report.results[0] === { focusNode, path, value, sourceShape,
//                         sourceConstraintComponent, severity, message }
const ttl  = Validator.validateTurtle(dataTurtle, shapesTurtle, "turtle"); // report-RDF Turtle
const text = Validator.validateText(dataTurtle, shapesTurtle, "turtle");   // one line per result
const ok   = Validator.conforms(dataTurtle, shapesTurtle, "turtle", /* violationsOnly */ false);
```

## ✨ Features

- **Stateless one-shot entry points.** `data` and `shapes` are RDF documents in the
  syntaxes `sparq_core::Graph::load_str` accepts: `"turtle"`, `"ntriples"`,
  `"nquads"`, `"trig"` (named graphs folded into the default graph). Every method
  throws an `Error` only if a document fails to parse — malformed shapes are skipped
  by the engine, never surfaced as an error.
- **Full report surface.** In each JSON result, `path`/`value` are `null` when
  absent; `focusNode`/`value`/`sourceShape` are N-Triples term strings, `path` is a
  SHACL Turtle path expression, and `message` is the plain text of the shape's first
  `sh:message` (or the generated default). `conforms` (and `report.conforms`) counts
  EVERY result regardless of severity — the W3C-suite notion of `sh:conforms`; pass
  `violationsOnly: true` (or filter `results` by `severity`) for a CI-style gate that
  ignores `sh:Warning` / `sh:Info`.
- **SHACL Advanced Features (`sh:rule`) — opt-in `shacl-af` feature.** Built with
  `--features shacl-af`, the bundle engages `sparq-shacl`'s SHACL-AF rule validation.
  It is OFF by default so the standard bundle carries zero rule code; CI builds and
  tests the bundle in BOTH feature states.
- **Repeat validation without re-parse — opt-in `stateful` feature** (sq-01xlp).
  A directional, NON-canonical work-box measurement found data-graph parsing coming
  to dominate the one-shot cost as corpora grow
  ([`research/shacl-wasm-stateful-2026-07.md`](../../research/shacl-wasm-stateful-2026-07.md);
  canonical quiet-box evidence pending), so `--features stateful` adds a pre-parsed
  `ParsedGraph` handle:
  `ParsedGraph.parse(text, format)` once, then `.validate(shapes)` /
  `.validateTurtle` / `.validateText` / `.conforms` per call — the same report
  surface at validate-only cost (call `.free()` when finished; handles hold wasm
  linear memory). OFF by default so the showcase artifact (and its deterministic
  bundle-bytes record) is unchanged; CI covers every feature state. Note the lean
  bundle's `Store.validate` is ALSO a stateless one-shot — this handle is the only
  pre-parsed SHACL path on either wasm surface.
- **Native correctness, thin binding.** Validation correctness is the native
  `sparq-shacl` engine's; this bundle only loads the two graphs and hand-serialises
  the report (no serde, mirroring the lean bundle's JSON path). The build is
  single-threaded (no rayon) and pure-Rust; `sparq-shacl`'s `regex` dependency (the
  `sh:pattern` constraint) compiles to `wasm32-unknown-unknown` — the regex automata
  are the bundle-size consideration, which is why this bundle is lazy-loaded on its
  page only and built `-Oz`.

## 📚 Learn more

- **How-to** — [`skills/shacl-validation/SKILL.md`](../../skills/shacl-validation/SKILL.md).
- **Performance** — we deliberately quote **no** hard-coded byte/MB figure here
  (bundle size drifts with the toolchain and dependency versions). The gzip transfer
  size — what end users actually download, since the site serves the `.wasm`
  gzip-compressed — is reproducible per toolchain:

  ```sh
  wasm-pack build crates/sparq-shacl-wasm --target web --release  # add `-- --features shacl-af` for the rules variant
  f=crates/sparq-shacl-wasm/pkg/sparq_shacl_wasm_bg.wasm
  echo "pre-gzip: $(stat -c%s "$f") bytes   gzip -9: $(gzip -9 -c "$f" | wc -c) bytes"
  ```

  For the *comparative* wire-byte picture — this artifact against an
  esbuild-minified, tree-shaken `rdf-validate-shacl` browser bundle, plus the
  size-trim levers (cargo profile, binaryen headroom) — run
  `bash bench/shacl-wasm/run.sh --bundle-only`; the levers are inventoried in
  [`research/gap-shacl-wasm-2026-07.md`](../../research/gap-shacl-wasm-2026-07.md#size-trim-levers-sq-c6c2s).

- **Status** — this crate delivers the wasm portability, the `Validator` entry
  points, the `shacl-af` opt-in, and a headless `wasm-pack test --node` smoke suite.
  The npm wrapper packaging and Pages deploy wiring are tracked separately (the
  `/surface/shacl` page bead sq-egy6 and the Pages workflow).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
