# sparq-core

<p>
  <a href="https://crates.io/crates/sparq-core"><img src="https://img.shields.io/crates/v/sparq-core.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-core"><img src="https://docs.rs/sparq-core/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

The **[RDF](https://www.w3.org/TR/rdf12-concepts/) triplestore** at the heart of
[sparq](../../README.md) — the storage substrate every other sparq crate builds on.

Load [RDF 1.2](https://www.w3.org/TR/rdf12-concepts/) (named graphs and triple terms
included) from the text formats, in memory or out-of-core for datasets larger than RAM, and
scan triple patterns. How the store is laid out and why is in the design docs linked below.

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;

let turtle = r#"<http://example.org/alice> a <http://schema.org/Person> ."#;
let g = Graph::load_str(turtle, "turtle")?;

let count = g.len();
assert_eq!(count, 1);
# Ok(()) }
```

## ✨ Features

- **RDF parsing & ingest** — load Turtle, N-Triples, N-Quads, and TriG from a `&str` or any
  `Read`; the opt-in `jsonld` and `rdfxml` features add JSON-LD and RDF/XML ingest (kept off
  the lean default/wasm build). `load_reader*` accepts any `Read`, so you can wrap a `gzip` /
  `bzip2` / `zstd` decoder around your file and stream it in — sparq does not content-sniff or
  auto-decompress ([guide](../../skills/data-formats/SKILL.md)).
- **Opt-in validated fast ingest** — the `iri-fast` feature (OFF by default) has the byte-level
  N-Triples/N-Quads loader validate every IRI against RFC-3987 through a prefix-memoized fast
  path in front of `oxiri` — accepting *exactly* what `oxiri` accepts (a mandatory
  differential-fuzz equivalence gate), so the parallel loader reaches conformance parity with
  the serial oxttl path at fast-path cost. Zero new dependency (`oxiri` is already in-tree).
- **Opt-in native Turtle parser** — the `native-ttl` feature (OFF by default) replaces the oxttl
  Turtle path with a hand-rolled byte-level tokenizer/parser (full grammar: prefixes/base,
  collections, blank-node property lists, predicate-object lists, numeric/boolean/triple-quoted
  literals, RDF 1.2 reifiers/triple-terms/annotations). It is a byte-identical drop-in — IRI
  resolution delegates to the same `oxiri` automaton oxttl uses, so resolved terms match, and the
  full W3C rdf-turtle suite passes an *identical* pass/fail set (a differential + conformance
  ratchet pin this). Only anonymous blank-node labels differ, as they do between two oxttl runs.
- **Triple-pattern scans** — look up any triple pattern over the loaded graph.
- **Incremental updates** — start from `Graph::new()` / `Graph::default()` (an empty graph) and
  `insert_triple(s, p, o)` / `remove_triple(s, p, o)` a single triple from `oxrdf` terms, or apply
  a whole batch with `apply_delta` — in place, with an optional write-ahead log.
- **Out-of-core store** — query datasets larger than RAM from a memory-mapped on-disk store,
  with optional block compression and near-zero resident heap. The opt-in `block-bloom` feature
  adds per-block Bloom filters on high-NDV columns to skip the block decode on equality-bound
  point lookups for an id that is ABSENT from the permutation (result-equivalent, never
  serialised; for a PRESENT id the zone map already narrows the candidate window to about one
  block, so there is little left to skip).
- **Compressed-seek column codecs (prototype)** — the opt-in `elias-fano` feature adds
  Elias-Fano and Partitioned-Elias-Fano codecs whose `next_geq(target)` answers a successor
  query *directly on the compressed data*, without the whole-block decode the varint block codec
  needs. It is a measurement-gated spike: not routed into the store, not called by any join, and
  carrying its own **native-only** A/B harness against the incumbent codec (it times with
  `std::time::Instant`, so the `wasm32` half of the comparison is unresolved). Pure `std`, no new
  dependency.
- **Named graphs & RDF 1.2** — full quad storage and
  [triple terms](https://www.w3.org/TR/rdf12-concepts/).
- **Thread-safe sharing** — `Graph` is `Send + Sync`, so one store serves many server threads;
  the opt-in `shared` feature adds an ergonomic `SharedGraph` handle for axum/actix state.

## 📚 Learn more

- **How-to** — [`skills/data-formats/SKILL.md`](../../skills/data-formats/SKILL.md) (ingest)
  and [`skills/sparql-query/SKILL.md`](../../skills/sparql-query/SKILL.md) (Rust API).
- **API reference** — [docs.rs/sparq-core](https://docs.rs/sparq-core).
- **Design** — [`research/ARCHITECTURE.md`](../../research/ARCHITECTURE.md); the indexing,
  compression and parsing verdicts live across the [`research/`](../../research) tree.
- **Performance** — numbers are not baked into docs; see the
  [benchmarks dashboard](https://sparq.jeswr.org/dev/bench).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
