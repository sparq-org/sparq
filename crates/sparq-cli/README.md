# sparq-cli

<p>
  <a href="https://crates.io/crates/sparq-cli"><img src="https://img.shields.io/crates/v/sparq-cli.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-cli"><img src="https://docs.rs/sparq-cli/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

The **command-line interface** to the [sparq](../../README.md) RDF triplestore.

Load and query RDF files, build on-disk indexes once and query them memory-mapped
(out-of-core, near-zero heap), benchmark queries, and run RDFS / OWL-RL / N3 reasoning —
all from one binary. Input is any RDF text format with transparent `.gz` / `.bz2` / `.zst`
decompression; SELECT/ASK results render as a table, TSV, CSV, XML or JSON.

## 🚀 Quickstart

```sh
# Query a file (Turtle / N-Triples / N-Quads / TriG, optionally .gz / .bz2 / .zst)
cargo run --release -p sparq-cli -- query data.ttl turtle \
  'SELECT ?s ?o WHERE { ?s <http://schema.org/name> ?o } LIMIT 10'

# Build on-disk indexes once, then query them memory-mapped (out-of-core, ~0 heap)
cargo run --release -p sparq-cli -- build data.nt ntriples ./idx
cargo run --release -p sparq-cli -- query-mmap ./idx 'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'

# Materialize RDFS/OWL-RL/N3 inference before querying
cargo run --release -p sparq-cli -- query data.ttl turtle 'SELECT * WHERE { ?s ?p ?o }' --reason rdfs
```

## ✨ Features

- **`query`** — load a file and run one query; SELECT/ASK output as table / tsv / csv / xml /
  json (`--format`), CONSTRUCT/DESCRIBE as N-Triples.
- **`build` / `query-mmap`** — build an on-disk index once, then query it memory-mapped
  without loading the dataset into RAM.
- **`bench` / `bench-mmap`** — run a directory of `*.rq` queries N times each, one TSV timing
  line per query. Pass `--json <path>` to ALSO write the per-query results (`name` / `rows` /
  `min_micros`, min-of-iters) to `<path>` as a machine-readable JSON document (the
  structured-benchmark-catalog shape); the STDOUT TSV is unchanged. Measured numbers are
  whatever the running host reports and are non-canonical — never commit them.
- **`dump <file> <in-fmt> <out-fmt>`** — re-serialize a loaded
  document to stdout in the writer matrix: `turtle[-pretty]` / `trig[-pretty]` / `nquads` /
  `ntriples` / `jsonld[-expanded|-flattened|-compacted]` / `jsonld-pretty…`. `jsonld-compact[-pretty]`
  runs the **full W3C JSON-LD 1.1 Compaction** against a caller `@context` passed with
  `--context <ctx.jsonld>` (richer than the prefix-only `jsonld-compacted`). The writer matrix is
  the `serialize-rdf` feature, pulled into the **default build by the default-on `jsonld` feature**.
- **`to-hdt <file> <in-fmt> <out.hdt[.gz|.zst|.bz2]>`** *(opt-in `hdt-write` feature, which
  implies `hdt`; [FABLE-5] sq-8ju74)* — export a loaded document (any ingestible format, HDT
  itself included) as a standard-layout **HDT v1.0 archive** via `sparq-hdt`'s direct in-memory
  encoder; the output container is chosen by the output extension. HDT holds a single default
  graph: named graphs are dropped **loudly** (a stderr warning with the dropped counts).
- **JSON-LD I/O is default-on** ([OPUS-4.8] sq-oy1f.4, user-prioritised epic sq-oy1f) — the
  default CLI **reads** a JSON-LD document (`<in-fmt>` ∈ `jsonld` / `json-ld` / `application/ld+json`;
  `@graph` named graphs are preserved as a dataset) AND **writes** one (the `jsonld*` out-formats
  above), out of the box. This is a deliberate maintainer-directed exception to sparq's
  opt-in-by-default principle. It stays toggleable: `--no-default-features --features mmap,mimalloc,dict-spill`
  drops the `oxjsonld` parser (and a `jsonld` input then errors, exit 2). What is default-on now:
  JSON-LD **parse + serialise + full 1.1 Compaction/Framing**; full conneg-conformance ratcheting
  is on the [sq-oy1f](https://github.com/sparq-org/sparq/issues/757) roadmap.
- **`--reason <rdfs|owl-rl|n3>`** — opt-in forward-chaining materialization before query.
- **`--reason datalog:<rules.dlog>`** *(opt-in `datalog`; [SONNET-4.6] sq-p4zci)* — run a **stratified Datalog** program
  over the parsed triples, then query the closure: negation as failure + `AGGREGATE`/`FILTER`, which monotone RDFS/OWL-RL
  cannot express. Loud exit-1 outside the fragment or on a `NOT`/`AGGREGATE` cycle; exit 2, no fall-back, when off.
- **`classify <file> <format> [out.nt]` / `--reason el`** *(opt-in `el` feature; [OPUS-5] sq-2ch27)* —
  run the **OWL 2 EL** consequence-based classifier (`sparq-reason-el`, pulled with its `rbox` role
  automaton) and materialize the class-subsumption lattice as `rdfs:subClassOf` triples (plus the
  role-inclusion closure as `rdfs:subPropertyOf`) — complete for the **E1+E2 fragment** (CR1–CR6
  class saturation + the CR10/CR11 role automaton), **not** for OWL 2 EL as a whole: the CLI does
  **not** enable `cdomain`, so concrete-domain axioms (faceted `owl:onDatatype`, literal
  `owl:hasValue`/`owl:oneOf`) are counted in `skipped_axioms` and **not applied**, and on such an
  ontology the hierarchy can be incomplete. `classify` prints the report as `name<TAB>value` lines;
  `--reason el` on `query` (and `reason <f> <fmt> el`) classifies then hands the augmented graph to
  the ordinary query path. **OWL 2 RL is sound but *incomplete* for class classification** — use
  `el`, not `--reason owl`, when you need the EL class hierarchy. Honest incompleteness (skipped
  axioms, a non-regular RBox, unsatisfiable classes) is reported on stderr, never swallowed;
  without the feature `--reason el` exits 2 naming it rather than silently downgrading to RL.
- **`tabular <csv[.gz|.zst|.bz2]> …`** *(opt-in `tabular` feature; [FABLE-5] sq-lsp7k.8)* —
  **materializing tabular→RDF import**: stream CSV rows through a direct mapping (subject IRI
  template `{col}`/`{_row}` via `--template`, per-column predicates, `xsd` datatype inference,
  per-row `rdf:type`) or an **R2RML mapping** (`--mapping <r2rml.ttl>`, CSV logical tables bound
  by `rr:tableName` = file stem) into a loaded graph (`--query` runs SPARQL in the same shot) or
  an N-Triples stream (`--out out.nt[.gz|.zst]`). Streaming end-to-end (no whole-file buffering).
  [OPUS-5] (sq-u1z86) adds cross-CSV **joins** (`rr:parentTriplesMap` + `rr:joinCondition`, run as
  a keyed hash join over a one-pass parent index), **named-graph** output (`rr:graphMap`/`rr:graph`
  → N-Quads + a dataset load, so `GRAPH ?g { … }` works) and `--row-provenance`
  (`prov:wasDerivedFrom` the source row). SQL-connection R2RML stays out of scope (`rr:sqlQuery`,
  `rr:sqlVersion`, `rr:inverseExpression` fail loudly — sparq materializes, it does not virtualize).
- **`terse <query | ->`** *(opt-in `terse` feature; [OPUS-4.8] sq-vczh2)* — transpile a terse query
  (the `K:<name>` keyword layer over canonical SPARQL) into the **canonical SPARQL** it expands to,
  printing the verifiable JSON `{ canonical_sparql, keywords, resolutions, warnings, legendVersion }`
  (the same contract the server's `POST /terse/transpile` returns). It never executes the query —
  pipe `canonical_sparql` into `query`. Loud-fails (exit 2) on an unknown keyword or a `V(...)`
  construct rather than guessing. Lean build — depends only on `spargebra`. `--features terse`.
- **Engine features the default CLI build lights** — the CLI's `sparq-engine` dependency enables
  `dp-planner` (DPccp cost-optimal join ordering, sq-7d3dj.30.5) and `algebra-rewrite` (the
  result-equivalent pre-execution rewrite of #1735 — `FILTER(?v = <iri>)` constant folding +
  `!bound` anti-join; [FABLE-5] sq-7d3dj.30.13), so the shipped binary and every canonical
  benchmark run the same plans. The engine **library** defaults stay lean (both OFF there).
- **Transparent decompression** — `.gz` / `.bz2` / `.zst` inputs detected by content.
  The gzip path defaults to the pure-Rust `miniz_oxide` backend; the opt-in, native-only
  `zlib-ng` cargo feature (`cargo build -p sparq-cli --features zlib-ng`, or
  `--features hdt,zlib-ng` to extend it to `.hdt.gz`) swaps in the faster zlib-ng C
  backend for gzip inflate at zero code change. Off by default; native-only, so it never
  reaches the wasm build.

## 📚 Learn more

- **How-to** — [`skills/cli/SKILL.md`](../../skills/cli/SKILL.md) (full subcommand reference)
  and [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md) (reasoning).
- **API reference** — run `cargo run -p sparq-cli -- --help`; rustdoc at
  [docs.rs/sparq-cli](https://docs.rs/sparq-cli).
- **Design** — [`research/ARCHITECTURE.md`](../../research/ARCHITECTURE.md).
- **Performance** — see the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench);
  numbers are not baked into docs.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
