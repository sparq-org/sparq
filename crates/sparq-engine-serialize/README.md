# sparq-engine-serialize

The RDF **writer matrix** for [`sparq-engine`] — Turtle, TriG, N-Quads, and JSON-LD 1.1
(expanded / flattened / compacted / framed, buffered **and** streaming).

> **Internal, unstable crate.** This is seam 1 of the staged `sparq-engine` facade split
> (RFC `research/engine-split-rfc.md` §4 Option A / §7 Phase A1, bead `sq-6vshe.4`). It is
> published to crates.io only to satisfy `sparq-engine`'s dependency closure and has
> **no stability guarantee of its own**. Depend on the
> re-export **`sparq_engine::serialize`** — its public API and the `serialize-rdf` /
> `streaming-serialization` feature names are unchanged by the split.
>
> Model: Opus 4.8 ([OPUS-4.8], Fable unavailable). Flag for re-review when Fable returns.

[`sparq-engine`]: ../sparq-engine

## 🚀 Quickstart

```rust
// Consume it through the facade — never depend on this crate directly.
// (sparq-engine, features = ["serialize-rdf"])
use sparq_core::Graph;
use sparq_engine::serialize::{graph_to_turtle, graph_to_nquads, graph_to_jsonld, JsonLdForm};

let g = Graph::load_str("<http://ex/s> <http://ex/p> \"o\" .", "turtle").unwrap();

let ttl = graph_to_turtle(&g);          // Turtle with @prefix compaction
let nq = graph_to_nquads(&g);           // N-Triples + the 4th graph column
let jsonld = graph_to_jsonld(&g, JsonLdForm::Expanded);  // JSON-LD 1.1

assert!(ttl.contains("<http://ex/s>"));
assert!(nq.ends_with(".\n"));
assert!(jsonld.starts_with('['));
```

## ✨ Features

- **Turtle / TriG** — `write_turtle` / `write_trig` with `@prefix` compaction,
  predicate-object lists, `a` for `rdf:type`, RDF collections, and correct literal /
  IRI / blank-node escaping; plus emission-order-independent `*_pretty` variants.
- **N-Quads** — `write_nquads`, N-Triples term syntax (oxrdf `Display`, byte-stable) with
  the graph column.
- **JSON-LD 1.1** — a native, dependency-free writer (`graph_to_jsonld` / `write_jsonld`)
  emitting the **expanded**, **flattened**, or **compacted** document form
  (`JsonLdForm`), plus **framing** (`write_jsonld_framed`) and indented `*_pretty` output.
  Its `Json` AST is single-sourced in the zero-dependency `sparq-jsonld` crate.
- **`serialize-rdf`** *(feature, off by default)* — gates the whole matrix. When off, the
  crate compiles **empty** and pulls in **zero** dependencies, so the default and wasm
  builds of `sparq-engine` are byte-identical to before the split.
- **`streaming-serialization`** *(feature, implies `serialize-rdf`)* — adds
  `write_turtle_streaming` / `write_trig_streaming` that render one subject block at a
  time into a `std::io::Write` without materialising the whole rendered string (what
  `sparq-cli dump … turtle|trig` uses under its `streaming-serialization` feature). Byte-identical to the buffered output.

### Opt-in by construction

Nothing in sparq's default native build or the wasm artifact compiles this crate:
`sparq-engine` pulls it in only behind its off-by-default `serialize-rdf` feature, as an
optional dependency. The crate is `#![forbid(unsafe_code)]` and adds no default dependency
anywhere.

## 📚 Learn more

- Facade + public surface: [`sparq-engine`] (`sparq_engine::serialize`).
- Split design + seam map: `research/engine-split-rfc.md` (bead `sq-6vshe.4`).
- Data-format serialization surface: `skills/data-formats/SKILL.md`.
- Turtle: <https://www.w3.org/TR/turtle/> · JSON-LD 1.1: <https://www.w3.org/TR/json-ld11/>

## License

MIT © the sparq authors.
