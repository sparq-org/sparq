<!-- [OPUS-4.8] sq-4lvq: README brought to template (deferred from sq-inzv). -->
# sparq-hdt

Opt-in [HDT](https://www.rdfhdt.org/) (Header Dictionary Triples) reader (and,
behind the `write` feature, writer) for the sparq RDF engine: load `.hdt` archives
straight into a `sparq_core::Graph`, and save a `Graph` back out. This is a separate
crate so the core engine — and in particular the wasm build — carries zero HDT code
or dependencies. Native-only by design.

## 🚀 Quickstart

```rust,no_run
# fn main() -> Result<(), sparq_hdt::Error> {
let graph = sparq_hdt::load("dataset.hdt")?;   // .hdt.gz/.zst/.bz2 sniffed + decompressed too
let meta  = sparq_hdt::header("dataset.hdt")?; // the HDT header (VoID stats, provenance) as a Graph
// query them like any other sparq graph

// filtered loading (opt-in `load-filter` feature): None means wildcard
# #[cfg(feature = "load-filter")]
# {
let predicate = oxrdf::NamedNode::new_unchecked("http://www.w3.org/2000/01/rdf-schema#label");
let pattern: sparq_hdt::TriplePattern = (None, Some(predicate), None);
let reader = std::io::BufReader::new(std::fs::File::open("dataset.hdt")?);
let _labels = sparq_hdt::load_reader_filtered(reader, &pattern)?;
# }

// writing (opt-in `write` feature): Graph -> .hdt (honours .gz/.zst/.bz2 by extension)
# #[cfg(feature = "write")]
# {
sparq_hdt::save(&graph, "out.hdt")?;
# }
# let _ = &meta;
# Ok(()) }
```

In sparq-cli (behind the opt-in `hdt` cargo feature —
`cargo build -p sparq-cli --features hdt`), the `hdt` format argument or a
`.hdt`/`.hdt.gz` file extension routes loading through this crate; the opt-in
`hdt-write` CLI feature adds `sparq-cli to-hdt <file> <in-fmt> <out.hdt>`, the
export direction over `save` (sq-8ju74).

## ✨ Features

- **Direct decoder is the default binary ingest**: `load` / `load_reader` (and the
  filtered/stats variants) decode the HDT sections locally (`src/decode.rs`) in a one-shot
  SPO scan; the maintained [`hdt`](https://crates.io/crates/hdt) crate (KonradHoeffner/hdt, MIT)
  remains the upstream-backed oracle/auxiliary path (`graph_from_hdt`, `write`-feature writers).
- **Format coverage**: the standard HDT v1.0 layout produced by hdt-cpp / hdt-java /
  the `hdt` crate — `FourSectionDictionary` (PFC) + `BitmapTriples`, SPO order. Exotic
  layouts from the W3C member submission that no mainstream tool emits (triples lists,
  alternate dictionary implementations) are rejected with an error by the direct decoder.
- **Translation is id-level and single-pass**: each distinct HDT dictionary id is
  decompressed once, interned into the sparq dictionary, and memoized in a flat
  per-section table (shared-section terms are translated once even when used as
  both subject and object) — the term set is never materialized twice.
- HDT term shapes covered: IRIs, blank nodes, plain / language-tagged / datatyped
  literals (lang tags normalized to lowercase, matching sparq's other loaders).
- **Compressed containers** (`.hdt.gz` / `.hdt.zst` / `.hdt.bz2`): detected by magic bytes —
  not file names — in every entry point and stream-decompressed on the fly. The default
  flate2 backend is pure-Rust `miniz_oxide`; the opt-in, native-only `zlib-ng` cargo
  feature (`cargo build -p sparq-hdt --features zlib-ng`) swaps in the faster zlib-ng
  C backend for gzip inflate at zero code change. Off by default and never reaches the
  wasm build (this crate is native-only and not in the wasm graph).
- **Header access**: `header()` / `header_reader()` decode just the dataset
  metadata triples (the "H" in HDT) into a queryable `Graph` without touching
  the dictionary/triples sections.
- **Filtered loading and stats** (opt-in `load-filter` feature, [GPT-5.6]
  sq-lsp7k.24 / sq-obhf1): `load_reader_filtered(reader, &pattern)` loads matches;
  `stats_reader_filtered(reader, &pattern)` counts them without constructing a
  result graph. Both filter during the one-shot SPO walk; an all-wildcard pattern
  is identical to the corresponding unfiltered operation.
- **Writing** (opt-in `write` feature): `save(&graph, path)` serialises a `Graph` to a
  standard-layout `.hdt` (or `.hdt.gz`/`.hdt.zst`/`.hdt.bz2`, chosen by the output
  extension), encoding the HDT sections **directly** from sparq's in-memory dictionary +
  triple ids (`src/encode.rs`) — no temporary N-Triples file, no text re-parse. The bytes
  are interoperable standard HDT v1.0; the PFC dictionary and the BitmapTriples payload are
  byte-for-byte identical to the upstream builder's output (proven in
  `tests/write_roundtrip.rs`). HDT carries a single default graph, so named graphs are
  ignored; an RDF 1.2 quoted-triple term cannot be written (standard HDT has no
  representation for it) and `save` returns `Error::Term`.

## Validation

`tests/roundtrip.rs`: `snikmeta.hdt` — a real-world archive vendored from the `hdt` crate's
test suite (not produced by this code path) — must load to exactly the same triple set as
its N-Triples rendering loaded through sparq's own parser; and a term-zoo N-Triples document
(unicode, lang tags, datatypes, blank nodes, shared subject/object terms, inline-integer
literals) round-tripped through a generated HDT archive must match sparq's direct N-Triples
load. `tests/write_roundtrip.rs` (the `write` feature): a saved-then-reloaded `Graph` must
equal the original term set over the term zoo, a multi-block graph, the empty graph, all
three compression containers, and the upstream-oracle load path — and one test asserts the
encoder's FourSectDict PFC bytes and BitmapTriples payload are byte-for-byte identical to the
upstream builder's output (`Hdt::read_nt` + `Hdt::write`).

## Load throughput

The `bench_load` example loads ~1M synthetic triples from a `.hdt` archive vs the equivalent
`.nt.gz`, reporting size, load time, and throughput. HDT loads faster than gunzip-and-parse;
on real corpora with heavier term reuse HDT archives are typically several times smaller than
gzipped N-Triples, which is the format's main draw alongside no-text-parse loading. Run it for
the numbers (tracked figures live on the perf dashboard):

```sh
cargo run --release -p sparq-hdt --example bench_load
# add `-- --json <path>` to also write the measurements as machine-readable JSON
# (STDOUT unchanged; timings are advisory/non-canonical, nothing committed)
cargo run --release -p sparq-hdt --example bench_load -- --json /tmp/hdt.json
```

## 📚 Learn more

- Skill: `.claude/skills/hdt-format/SKILL.md`
- Perf dashboard: <https://sparq.jeswr.org/dev/bench>
- Not yet supported / open work: `bd list -l area:sparq-hdt`. (The decode-only ingest fast
  path itself already ships as the default `load` path; what remains open is upstream
  adoption of a decode-only entry point — status in `UPSTREAM.md` item 2.)

## License

MIT. [OPUS-4.8] sq-4lvq
