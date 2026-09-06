---
name: cli
description: Use when you need to drive the sparq RDF/SPARQL engine from the command line — load a Turtle/N-Triples/N-Quads/TriG (or HDT) file and run a SPARQL query, compare RDF triple sets, build/query memory-mapped on-disk indexes for datasets larger than RAM, materialize RDFS/OWL-RL/N3 reasoning closures, classify an OWL 2 EL ontology into its subsumption lattice, stream-ingest huge gzip/bzip2/zstd dumps, import CSV as RDF (direct mapping or R2RML), or benchmark query suites. Covers the actual `sparq-cli` subcommands, positional argument order, and cargo feature flags.
---

# sparq-cli

`sparq-cli` is the command-line front-end to the `sparq` RDF triplestore + SPARQL engine. It loads RDF files (with transparent gzip/bzip2/zstd decompression), runs SPARQL, builds and queries out-of-core memory-mapped indexes, and materializes reasoning closures (RDFS / OWL-RL / N3).

**Argument style (important):** the CLI uses a hand-rolled positional parser — there is *no* clap, no `--help`, and **no GNU-style flags except `--reason`/`--proof`, `query`'s `--format`/`--count`, and `diff`'s `--exact`**. The first token is the subcommand; the rest are positional and order matters. An unknown/missing subcommand prints a short usage block and exits with code 2.

## Quickstart

Run via cargo (the binary is `sparq-cli`; build with `--release` — debug builds are far slower):

```bash
# Load a Turtle file and run one query — prints the RESULTS (a readable table by default).
cargo run --release -p sparq-cli -- \
  query data.ttl turtle 'SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10'
# stderr: loaded N triples in 0.123s (...)
# stdout: a table of the solution bindings + a "(K row(s))" footer
```

`query` emits real results by FORM: SELECT → the solution bindings, ASK → a boolean (`true`/`false`), CONSTRUCT/DESCRIBE → the resulting triples as N-Triples. Pick the SELECT/ASK serialisation with `--format <table|tsv|csv|xml|json|ntriples>` (default `table`); add `--count` to restore the old count-only line (`<n> solutions/triples in <ms>ms`). See the `query` entry under Key APIs for the full matrix.

`format` is one of `turtle | ntriples | nquads | trig` (aliases: `n-triples`, `n-quads`, `application/trig`). `nquads`/`trig` are loaded as a dataset so `GRAPH {}` works. Compressed inputs are auto-detected by extension (`.gz`, `.bz2`, `.zst`/`.zstd`) and streamed.

## Key APIs (subcommands)

All invoked as `sparq-cli <subcommand> <args...>`:

- `query <data-file> <format> <sparql> [--format <out>] [--count] [--reason <rdfs|owl|n3|el|datalog:<rules.dlog>>]` — load file, run one query, print its **results** to stdout, dispatched by query form:
  - **SELECT** → the solution bindings. `--format` chooses the serialisation: `table` (default, a readable fixed-width ASCII table with a `(K row(s))` footer), `tsv` / `csv` / `xml` (W3C SPARQL Results, reusing `sparq-server`'s serialisers), or `json` (SPARQL 1.1 Results JSON, the engine's direct serialiser). `ntriples` is not meaningful for bindings and falls back to `tsv`.
  - **ASK** → a boolean: `true` / `false`. `--format json` / `--format xml` emit the W3C boolean documents (`{"head":{},"boolean":…}` / `<sparql>…<boolean>…</boolean></sparql>`); other formats print the bare token.
  - **CONSTRUCT / DESCRIBE** → the resulting triples serialised as **N-Triples** (always; `--format` is a SELECT/ASK selector and is ignored for the graph forms).
  - `--count` restores the historical count-only output (`<n> solutions in <ms>ms` for SELECT/ASK, `<n> triples in <ms>ms` for the graph forms) — the backward-compatible escape hatch for scripts that scraped the count.
  - An unknown `--format` value is a usage error (exit 2); a query/runtime error exits 1.
- `diff <file-a> <file-b> [--exact]` *(opt-in `diff` feature; [GPT-5.6] sq-lsp7k.28)* — auto-detect each RDF format from `.nt`, `.ttl`, `.nq`, `.trig`, or `.jsonld` (before an optional compression extension), compare the documents as triple sets, and emit a deterministic N-Triples patch. Lines prefixed by `-` form the first lexicographically sorted block; lines prefixed by `+` form the second. Exit 0 means identical sets and empty stdout; exit 1 means different sets. Named-graph names are discarded, duplicate triples collapse, and blank-node labels compare exactly as loaded. `--exact` is currently a compatibility alias for that same behavior.
- `reason <data-file> <format> <rdfs|owl|n3|el|datalog:<rules.dlog>> [out.nt]` — materialize the entailed closure; print closure triple count; with `out.nt`, write the full closure as N-Triples. Add `--proof` (N3 only) to print each derivation step. `el` needs the opt-in `el` feature (see `classify` below); `datalog:<rules.dlog>` needs the opt-in `datalog` feature (see the stratified-Datalog example below).
- `classify <data-file> <format> [out.nt]` *(opt-in `el` feature; [OPUS-5] sq-2ch27)* — run the **OWL 2 EL** consequence-based classifier (`sparq-reason-el`) and materialize the class-subsumption lattice as `rdfs:subClassOf` triples (plus the role-inclusion closure as `rdfs:subPropertyOf`, since the CLI's `el` feature also turns on the crate's `rbox` role automaton). **Scope: complete for the E1+E2 fragment, not for OWL 2 EL as a whole** — the CLI does not enable the crate's `cdomain` feature, so concrete-domain axioms (see the `skipped_axioms` note below) are deferred. Prints the classification **report** as `name<TAB>value` lines on stdout — `triples`, `named_classes`, `emitted_subclassof`, `emitted_subpropertyof`, `skipped_axioms`, `unsatisfiable_classes`, `thing_unsatisfiable`, `rbox_non_regular` — and with `out.nt` writes the lattice-augmented graph as N-Triples. **Honest incompleteness is reported, never swallowed:** a non-zero `skipped_axioms` means class axioms used a construct this build does not reason over and were **not applied** — either outside EL entirely (union / complement / allValuesFrom / cardinality / multi-individual `oneOf`), **or in OWL 2 EL but deferred here because the CLI omits `cdomain`**: every concrete-domain axiom (faceted `owl:onDatatype`/`owl:withRestrictions`, literal `owl:hasValue`/`owl:oneOf`) is skipped, so a valid EL ontology using datatype restrictions can classify to an **incomplete** hierarchy; `rbox_non_regular` means the told RBox has a property-chain cycle, so derivations stay sound but the completeness argument does not hold. Both also print an explanatory NOTE on stderr.
- `build <file[.gz|.bz2|.zst]> <format> <dir> [chunk_millions=16]` — EXTERNAL-MEMORY build: stream the (compressed) document straight to on-disk memory-mapped indexes via disk-backed sort/merge. For datasets whose indexes exceed RAM. `chunk_millions` sets the in-memory run size. Writes RAW perms by default; set `SPARQ_BUILD_COMPRESSED=1` to emit block-compressed (`SPQCPRM1`) perms directly from the merge tail, skipping a later `recompress` (byte-identical to build-then-recompress).
- `save <data-file> <format> <dir> [compressed] [--format-v2]` — load into RAM then persist the six permutation indexes to `<dir>`. Add the literal word `compressed` for block-compressed permutations, and `--format-v2` to write those as `SPQCPRM2` instead of `SPQCPRM1` (see the emit-format note below).
- `query-mmap <dir> <sparql> [--format <out>] [--count]` — open a saved/built dir with indexes MEMORY-MAPPED (out-of-core) and run a query, printing its **results**. Output is at **parity with `query`**: SELECT → bindings (default a readable table; `--format <table|tsv|csv|xml|json|ntriples>` selects the serialisation), ASK → a boolean (`--format json|xml` → the W3C boolean documents), CONSTRUCT/DESCRIBE → the resulting triples as N-Triples; `--count` restores the legacy count-only line (`<n> solutions/triples in <ms>ms`). The only difference from `query` is the data source — an mmap-backed `Graph::open` instead of an in-RAM load (permutations stay in the OS page cache, not the process heap). An unknown `--format` is a usage error (exit 2); a query/runtime error exits 1.
- `recompress <src-dir> <dst-dir> [--v2]` — re-persist a saved dir with block-compressed permutations without re-parsing (dirs must differ). `--v2` writes `SPQCPRM2` (see the emit-format note below).
- **Compressed-perm emit format (`--format-v2` / `--v2`; opt-in `spqcprm2` feature; [SONNET-4.6] sq-kmve2).** A build emits `SPQCPRM1` by DEFAULT and the V2 emitter is opt-in (`cargo build -p sparq-cli --features spqcprm2`); the V2 *reader* always ships, so a `SPQCPRM2` dir opens anywhere. The flag is the per-invocation form of `SPARQ_EMIT_FORMAT=v2` — same effect, no env var to leak into child processes — and takes precedence over that variable, which still works unchanged. Fail-closed: `--format-v2` without the `compressed` positional, an unknown `--flag`, or the flag on a binary built WITHOUT `spqcprm2` are all usage errors (exit 2) rather than a silent `SPQCPRM1` write. Whether V2 is smaller than V1 is corpus-dependent (it frame-of-reference encodes the col2 reset) — measure on your data; see the `data-formats` skill.
- `compact <persist-dir>` — **WAL compaction / vacuum for erasure-completeness** (`sq-x32t`). OFFLINE operator command: stop a `--persist` server, run this on its directory, restart. Opens the dir (replaying its WAL into the live overlay), then **physically rewrites** the store to only the current live triples with a **re-interned (purged) dictionary**, and **atomically swaps** the directory (rollback-safe two-rename + WAL truncate; an interrupted swap is healed on the next open). So a logically-`DELETE`d / `DROP`ped triple's data — including an orphaned **literal value** — is gone from disk, not just hidden. The live triple set is preserved exactly (round-trip). The **online** equivalent is `POST /admin/compact` on a running server (see the `http-server` skill). **Honest scope:** scrubs the engine's own on-disk segments + dictionary; it cannot reach off-box copies (filesystem snapshots, COW history, external backups) — see `compliance/privacy/retention-erasure-runbook.md` §7a/§7b.
- `dump <file[.gz|.bz2|.zst]> <in-format> <out-format>` — load an RDF document and re-serialize the whole graph (default + named graphs) to stdout in the RDF **writer matrix**. `out-format` ∈ `turtle | turtle-pretty | trig | trig-pretty | nquads | ntriples | jsonld[-expanded|-flattened|-compacted] | jsonld-pretty[-expanded|-flattened|-compacted]` (Turtle emits the default graph only; trig/nquads/jsonld emit the full dataset; bare `jsonld` == `jsonld-expanded`, bare `jsonld-pretty` == `jsonld-pretty-expanded`; the `turtle-pretty`/`trig-pretty` forms emit deterministic, idiomatic Turtle/TriG — sorted, blank-line-separated subject blocks, the engine home for the site's pretty-Turtle reshaper; the `jsonld-pretty*` forms emit indented JSON-LD — a whitespace-only re-indent of the minified document, so same ordering, same RDF). The writer matrix is `sparq-engine/serialize-rdf` (zero new deps — the JSON-LD writer is a native, hand-rolled emitter with no json-ld/serde crate), pulled into the **default build** by the default-on `jsonld` feature (see "Default cargo features" below). `dump` also **reads** a JSON-LD `<in-format>` (`jsonld` / `json-ld` / `application/ld+json`) in the default build — JSON-LD is default-on ([OPUS-4.8] sq-oy1f.4). A `--no-default-features` build drops both the `oxjsonld` parser and the writer matrix (`dump` then errors on a `jsonld` out-format, and a `jsonld` in-format → exit 2). Unknown out-format → exit 2.
- `ingest <file[.gz|.bz2|.zst]> [parse|intern|full] [max_millions]` — streaming-throughput experiment over N-Triples: `parse` (decompress+parse+count), `intern` (+dictionary), `full` (+build indexes). Reports triples/s.
- `bench <data-file> <format> <queries-dir> [iters=5] [count|materialize|json]` — load once, run every `*.rq` in the dir (sorted) `iters` times, print TSV `<name>\t<rows>\t<min_micros>`. Mode default `materialize`.
- `memstat <data-file> <format> [compressed]` *([FABLE-5] sq-7d3dj.32)* — load a document and print a **deterministic memory-composition breakdown** as `name<TAB>value` lines on stdout: triple/term counts, the self-accounted heap total **decomposed** into dictionary / six-permutation store / numeric+temporal caches (bytes and B-per-triple, plus dict B-per-term), and the kernel's `VmRSS`/`VmHWM` (post-load resident + peak during load; Linux `/proc/self/status`, `0` elsewhere). Trailing literal `compressed` (or `SPARQ_STORE_PROFILE=compressed`, OR'd, applied once) re-encodes into the memory-bound in-RAM mode (`Graph::into_compressed`: block-compressed permutations + blob dictionary) before reporting, so both in-memory framings come from one instrument (the `mode` line says which). The at-scale extension of the CI `store_bytes_per_triple` metric — driven by `scripts/bench/bytes-per-triple.sh` (bench id `bytes-per-triple`) for the in-memory-vs-external bytes/triple envelope. Numbers are host-reported and non-canonical off the dedicated bench box; never commit them into docs.
- `bench-mmap <index-dir> <queries-dir> [iters=5] [count|materialize|json] [decompress]` — same as `bench` but opens the dataset out-of-core; trailing literal `decompress` decodes compressed perms to RAM first. Mode default `count`.
- `scaling <data-file> <format> <queries-dir> [threads=1,2,4,8,…] [iters=3]` — parallel-efficiency sweep across rayon pool sizes; TSV `subsystem\tthreads\tbest_ms\tspeedup\tefficiency`.
- `probe-compress <perm-file>` / `compare-compress <data-file> <format> [<sparql>]` / `bench-remap [n] [dict] [iters]` — measurement/instrumentation probes.
- `tabular <csv[.gz|.zst|.bz2]> [<name>=<csv> …] [flags]` *(opt-in `tabular` feature; [FABLE-5] sq-lsp7k.8)* — **materializing tabular→RDF import**, streaming end-to-end (CSV rows → first-party RFC-4180 reader → per-row N-Triples chunks → the parallel NT ingest; no whole-file buffering; compressed inputs auto-detected by extension). Two modes:
  - **Direct mapping** (default): subject = `--template` IRI template with `{col}` + `{_row}` (1-based data-row number) placeholders, default `<base><table>/row/{_row}`; predicate = `<base><table>#<column>`; object = the cell with **datatype inference** (`xsd:integer`/`xsd:decimal`/`xsd:double`/`xsd:boolean`, else plain string; `--no-infer` disables); each row typed `rdf:type <base><table>` (`--class <iri|none>` overrides). `--base` defaults to `http://example.com/`; table = file stem; template-substituted values are IRI-safe percent-encoded. An EMPTY cell is NULL → no triple (a NULL in the subject template skips the row).
  - **R2RML** (`--mapping <r2rml.ttl>`): the materializing subset over **CSV logical tables** (`rr:tableName` binds to a CSV by file stem, or explicitly via a `<name>=<path>` positional). Supports `rr:subjectMap`/`rr:subject`, `rr:class`, `rr:predicateObjectMap`, `rr:predicateMap`/`rr:predicate`, `rr:objectMap`/`rr:object`, `rr:template`/`rr:column`/`rr:constant`, `rr:termType` (IRI/Literal/BlankNode), `rr:datatype`, `rr:language`, plus **cross-CSV joins** and **named graphs** ([OPUS-5] sq-u1z86, below). **Fail-closed:** any other `rr:` construct — `rr:sqlQuery`, `rr:sqlVersion`, `rr:inverseExpression` — is a loud exit-1 error, never a silent skip; **SQL-connection R2RML stays a non-goal** (sparq's counter-story is materializing import, not virtualization). No datatype inference here (CSV's natural datatype is string, per the spec).
  - **Joins** (`rr:parentTriplesMap` + `rr:joinCondition`/`rr:child`/`rr:parent`; [OPUS-5] sq-u1z86): a referencing object map runs as a **keyed hash join** — the parent CSV is pre-scanned once into a `join-key tuple → parent subjects` index, then the child table streams past it. Honest cost: that index is the one non-constant-memory part of the pipeline (one key tuple + subject per parent row); the child side still streams. SQL NULL semantics: an empty join cell on either side matches nothing. **At least one `rr:joinCondition` is required** — R2RML's condition-free (cross-join) form is a loud error, not a guess.
  - **Named graphs** (`rr:graphMap`/`rr:graph`; [OPUS-5] sq-u1z86): a graph map on the subject map scopes that triples map's class + predicate-object triples, one on a predicate-object map scopes its own, and the two sets **union**; an empty set (or the `rr:defaultGraph` constant) is the default graph. A graph map that generates NULL contributes **nothing** to that union rather than erasing it — a sibling graph map, or the other side of the subject/predicate-object union, still scopes the statement; only when every *declared* graph map is NULL is the statement unscoped and dropped (never a silent fallback into the default graph). Using any graph map switches the emitter to **N-Quads** and the load path to `Graph::load_dataset`, so `GRAPH ?g { … }` works. Honest cost: the dataset load is whole-document, so the quad load path buffers the generated N-Quads (`--out` still streams); a graph-map-free mapping keeps the unchanged streaming N-Triples fast path.
  - **Row provenance** (`--row-provenance`, both modes; [OPUS-5] sq-u1z86): every generated subject also gets `<subject> prov:wasDerivedFrom <base><table>/row/{_row}>` — the same row IRI the direct mapping's default subject template produces, emitted into the row's graph(s). With the default direct-mapping template the subject *is* the row IRI, so the triple is a self-link; it earns its keep under `--template` / R2RML subjects.
  - **Output:** default = load the graph and print the summary line; add `--query <sparql>` (+ `--format`/`--count` as in `query`) to query it in the same shot; or `--out <file.nt|.nq[.gz|.zst]>` to stream the triples/quads out **without building a graph**. `--sep <char|tab>` sets the separator. Usage errors exit 2; data/mapping errors exit 1. Fixture suite: `crates/sparq-cli/tests/fixtures/r2rml/` (W3C-R2RML-adapted cases, incl. `join/`); perf smoke: bench id `tabular-import-smoke`.
- `terse <terse-query | ->` *(opt-in `terse` feature; [OPUS-4.8] sq-vczh2)* — transpile a **terse** query (the `K:<name>` keyword layer over canonical SPARQL) into the **canonical, conformant SPARQL** it expands to, printing the verifiable JSON contract `{ "canonical_sparql", "keywords": [{ "keyword", "iri", "legendVersion" }], "resolutions": [], "warnings": [], "legendVersion" }` (the SAME shape the server's `POST /terse/transpile` returns). Pass `-` to read the query from stdin. It does **not** execute the query — pipe `canonical_sparql` into `query`. The `K:<name>` legend maps the hot PKG predicates/classes (e.g. `K:derivedFrom` → `<http://www.w3.org/ns/prov#wasDerivedFrom>`) so an agent need not emit a `PREFIX` line. **Loud-fail**, never a silent guess: an unknown `K:<name>`, a `PREFIX K:` collision, or non-conformant input (the silent-rewrite canary) exits 2 with a message on stderr. `resolutions` is always empty in this build — `V("phrase")` concept resolution needs the crate's `vectors` feature (a graph-bound resolver + embedder), a future extension, so a `V(...)` construct exits 2 rather than guessing (caveat `sq-26fdp`). Off by default; build with `--features terse`.

Underlying engine entry points the CLI calls (for reference): `sparq_engine::query(&Graph, &str) -> Result<QueryResult, String>`, `::ask(...) -> Result<bool, String>`, `::count(...) -> Result<usize, String>`, `::query_json(...) -> Result<String, String>`, `::construct_ntriples(...) -> Result<String, String>`, and `PreparedQuery::{parse, is_graph_form}` to classify the form. The SELECT TSV/CSV/XML serialisers are reused from `sparq_server::results::{select_to_tsv, select_to_csv, select_to_xml, ask_to_json, ask_to_xml}` (the pure serialiser library — the CLI depends on `sparq-server` with `default-features = false`, so none of the async HTTP stack is pulled in). Loading goes through `sparq_core::Graph::{load_str, load_dataset, load_reader_parallel, build_external, open, save, save_compressed}`.

## Common recipes

**Build out-of-core indexes from a compressed dump, then query without loading into RAM:**
```bash
cargo run --release -p sparq-cli -- build dump.nt.zst ntriples ./idx
cargo run --release -p sparq-cli -- query-mmap ./idx \
  'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'
```

**Materialize an RDFS closure and write it out:**
```bash
cargo run --release -p sparq-cli -- reason ontology.ttl turtle rdfs closure.nt
```

**N3 forward-chaining with a proof trace:**
```bash
cargo run --release -p sparq-cli -- reason rules.n3 turtle n3 --proof
```
(For `n3` the `<format>` argument is ignored — the file is parsed as Notation3 facts+rules.)

**Query with reasoning applied first (OWL-RL); inconsistencies print to stderr):**
```bash
cargo run --release -p sparq-cli -- query data.ttl turtle \
  'SELECT ?s WHERE { ?s a ?c }' --reason owl
```
⚠️ **`--reason owl` is sound but INCOMPLETE for class classification.** OWL 2 RL's rule set is
scoped to assertional conclusions, so a `rdfs:subClassOf` hierarchy it materializes can silently
omit derivable subsumptions (e.g. it never composes `B ⊑ C`, `B ⊑ E` into `B ⊑ C ⊓ E`, so
`C ⊓ E ⊑ D ⊨ B ⊑ D` is missed). For the class hierarchy of an EL ontology (GO / ChEBI /
SNOMED-style), use `el` — a different algorithm, not more RL rules.

**Classify an EL ontology (subsumption lattice; requires the `el` feature; sq-2ch27):**
```bash
# Report only (name<TAB>value lines on stdout)
cargo run --release -p sparq-cli --features el -- classify ontology.ttl turtle
# ...and write the lattice-augmented graph out
cargo run --release -p sparq-cli --features el -- classify ontology.ttl turtle lattice.nt
# Or classify-then-query in one shot
cargo run --release -p sparq-cli --features el -- query ontology.ttl turtle \
  'SELECT ?sub ?sup WHERE { ?sub <http://www.w3.org/2000/01/rdf-schema#subClassOf> ?sup }' --reason el
```
Without the feature, `--reason el` exits 2 naming the feature — it never falls back to `owl`.
With it, check `skipped_axioms`: the CLI enables `rbox` but **not** `cdomain`, so an ontology whose
axioms use datatype restrictions classifies to a hierarchy that may be missing subsumptions.

**Run your own rules — stratified Datalog (requires the `datalog` feature; [SONNET-4.6] sq-p4zci):**
```dlog
# rules.dlog — the design record's headline program (research/stratified-datalog-rules.md §2)
@prefix ex: <http://ex/> .
[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .
[?x, a, ex:Hub]  :- [?x, ex:deg, ?c], FILTER(?c >= 3) .
[?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] .
```
```bash
# Materialize the closure and write it out
cargo run --release -p sparq-cli --features datalog -- \
  reason graph.nt ntriples datalog:rules.dlog closure.nt
# ...or reason-then-query in one shot (the derived facts are ordinary triples)
cargo run --release -p sparq-cli --features datalog -- query graph.nt ntriples \
  'SELECT ?x WHERE { ?x a <http://ex/Leaf> }' --reason datalog:rules.dlog
```
stderr reports the program shape and the expansion — over an 8-triple graph of 4 `ex:Node`s where
`a` has 3 outgoing edges and `b` has 1, that is
`reasoned [datalog rules.dlog]: 3 rule(s) in 3 stratum/strata; 8 -> 14 distinct triples (+6 derived) in <elapsed>s`
(two `ex:deg` counts, `a` as the only `ex:Hub`, and `b`/`c`/`d` as `ex:Leaf`).
This is the surface for what RDFS/OWL-RL structurally **cannot** do: those profiles are monotone, so
they have no `NOT` and no `AGGREGATE`. Both failure modes are loud (exit 1) and name the RULES file:
a syntax error or a construct outside the documented fragment is a parse error naming the offending
construct (e.g. `unsafe rule: head variable ?z is not bound by a positive body atom or an
AGGREGATE`), and a program whose recursion cycles through `NOT`/`AGGREGATE` is rejected by the
stratification checker naming a predicate on the cycle — never a silently
derivation-order-dependent answer.

**Get serialised SELECT bindings (CSV/TSV/XML/JSON) or a CONSTRUCT graph:**
```bash
# SELECT as CSV (W3C SPARQL Results CSV)
cargo run --release -p sparq-cli -- query data.ttl turtle \
  'SELECT ?s ?o WHERE { ?s <http://ex/age> ?o }' --format csv
# ASK as a boolean
cargo run --release -p sparq-cli -- query data.ttl turtle 'ASK { ?s ?p ?o }'   # -> true
# CONSTRUCT prints the resulting triples as N-Triples
cargo run --release -p sparq-cli -- query data.ttl turtle \
  'CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }'
```

**Compare two RDF files as triple sets (requires the `diff` feature):**
```sh
cargo run --release -p sparq-cli --features diff -- diff old.ttl new.ttl
```

**Benchmark a query suite, JSON-serialization mode, 10 iterations:**
```bash
cargo run --release -p sparq-cli -- bench data.nt ntriples ./queries 10 json
```

**Load an HDT archive (requires the `hdt` feature):**
```bash
cargo run --release -p sparq-cli --features hdt -- \
  query graph.hdt hdt 'SELECT ?s ?p ?o WHERE { ?s ?p ?o }'
# .hdt / .hdt.gz extensions are auto-detected even if you pass a wrong format arg.
```

**Export a document AS an HDT archive (requires the `hdt-write` feature; sq-8ju74):**
```sh
cargo run --release -p sparq-cli --features hdt-write -- \
  to-hdt data.ttl turtle out.hdt      # out.hdt.gz/.zst/.bz2 compress by output extension
# HDT holds a single default graph: named graphs are dropped LOUDLY (stderr warning).
```

## Gotchas / feature flags / prerequisites

- **`query` and `query-mmap` print RESULTS** (a table by default for SELECT; a boolean for ASK; N-Triples for CONSTRUCT/DESCRIBE) on stdout — load stats and reasoning/inconsistency reports go to stderr. Use `--format <table|tsv|csv|xml|json|ntriples>` to pick the SELECT/ASK serialisation, or `--count` for the legacy count-only line (`<n> solutions/triples in <ms>ms`). The two are at output parity (they share one emission core; `query-mmap` only differs in opening the indexes memory-mapped instead of loading into RAM). `bench`/`bench-mmap` are unchanged and emit TSV.
- **No `--help`.** Run a subcommand with too few args to see its one-line usage; an unknown subcommand prints the top-level usage and exits 2. Query errors exit 1.
- **`--reason <profile>` is a flag on `query`** (scanned anywhere in argv); the standalone `reason` subcommand instead takes the profile as the 3rd positional. Profiles: `rdfs`, `owl`, `n3`, and — with the opt-in features — `el` and `datalog:<rules.dlog>`. `datalog:` is the only profile that carries an ARGUMENT (its rules file), split on the FIRST `:`; a bare `datalog` is a usage error (exit 2) naming the syntax, in both feature states.
- **OWL 2 EL classification is opt-in:** `--features el` adds the `classify` subcommand and the `el` reasoning profile (`--reason el` / `reason <f> <fmt> el`). It pulls the separate `sparq-reason-el` crate **with its `rbox` feature** (E1+E2: CR1–CR6 class saturation + the CR10/CR11 role automaton), and adds no third-party dependency. **The CLI capability is that E1+E2 fragment, not the full OWL 2 EL profile:** `cdomain` (CR7–CR9 concrete domains) is *not* forwarded, so faceted datatype restrictions and literal `owl:hasValue`/`owl:oneOf` — legal EL constructs — are counted in `skipped_axioms` and left unapplied, and the emitted hierarchy can be incomplete on such an ontology (per-run count on stdout + stderr NOTE; a CLI `cdomain` surface is future work). **`el` is NOT a `sparq_reason::Profile`** — EL is a different algorithm in a different crate, so it is intercepted before the RDFS/RL profile parse. Without the feature, `--reason el` is a hard **exit-2** error naming the feature: it deliberately does **not** fall back to `owl`, because RL is sound but *incomplete* for class classification and a silent downgrade would return a quietly wrong hierarchy. Use `el`, not `--reason owl`, whenever you need an EL class hierarchy; see the `inference` skill for the full EL scope, the honest deferrals, and the library API.
- **Default cargo features:** `mmap` (out-of-core), `mimalloc` (global allocator — `--no-default-features --features mmap` falls back to the system allocator for A/B), `dict-spill`, and — [OPUS-4.8] sq-oy1f.4 (user-prioritised epic sq-oy1f) — `jsonld` (JSON-LD parse via `oxjsonld` + the `serialize-rdf` writer matrix). `jsonld` being default-on is a deliberate maintainer-directed exception to sparq's opt-in-by-default principle; it stays toggleable (`--no-default-features --features mmap,mimalloc,dict-spill` drops the JSON-LD parser + writers).
- **Engine features every CLI build lights (dep features, so on even under `--no-default-features`):** the CLI's `sparq-engine` dependency enables `dp-planner` (DPccp cost-optimal join ordering, [SONNET-4.6] sq-7d3dj.30.5) and `algebra-rewrite` (the result-equivalent pre-execution rewrite of #1735 — `FILTER(?v = <iri>)` IRI-constant folding + `FILTER(!bound)` anti-join; [FABLE-5] sq-7d3dj.30.13). The shipped binary and every canonical benchmark therefore run the rewritten, DP-ordered plans; the sparq-engine LIBRARY defaults keep both OFF for lean library consumers.
- **HDT is opt-in:** `--features hdt` (loader) / `--features hdt-write` (adds the `to-hdt` exporter, implies `hdt`). OFF by default partly because the wrapped `hdt` crate declares MSRV **1.87** and its decode stack is dead weight on the common paths.
- **Tabular import is opt-in:** `--features tabular` adds the `tabular` subcommand (CSV direct mapping + the R2RML materializing subset — including `rr:parentTriplesMap` joins and `rr:graphMap` named graphs; zero new deps). SQL-connection R2RML is out of scope and fails loudly.
- **Terse transpiler is opt-in:** `--features terse` adds the `terse` subcommand (the `K:<name>` keyword layer → canonical SPARQL, verifiable JSON expansion). OFF by default; the lean `sparq-terse` build depends only on `spargebra` (already in the CLI's tree), so it adds no heavy dep. See the `terse` subcommand above and the `http-server` skill for the matching `POST /terse/transpile` endpoint.
- **Stratified Datalog is opt-in:** `--features datalog` adds the `datalog:<rules.dlog>` reasoning profile (`--reason datalog:…` / `reason <f> <fmt> datalog:…`). It forwards `sparq-reason`'s own opt-in `datalog` feature — sparq-reason is already a plain CLI dep, so this adds **no new third-party dependency** (only the `sparq-substrate` rows+join slices the evaluator already builds on). Like `el`, the profile is intercepted before the RDFS/RL profile parse, and without the feature it is a hard **exit-2** error naming it — with **no fall-back at all**, because RDFS/OWL-RL are monotone and quietly running one of them would drop negation as failure and aggregation and return a different answer set. The rules file is user-supplied, so the two loud failure modes are the parser (a construct outside the documented fragment) and the stratification checker (a recursion cycle through `NOT`/`AGGREGATE`), both exit 1. See the `inference` skill for the dialect, its semantics, and the library API (`sparq_reason::datalog`, including the incrementally maintained `MaterializedProgram`).
- **RDF diff is opt-in:** `--features diff` adds the `diff` subcommand. It uses the existing `sparq-core` load path and adds no dependency; feature-OFF builds contain no diff dispatch or implementation.
- **In-RAM store profile (native, no feature needed; [FABLE-5] sq-7d3dj.32.2.1):** `SPARQ_STORE_PROFILE` selects the in-memory store layout on the **shared load path**, so `query` / `bench` / `reason` / `scaling` inherit it uniformly from one env var. Unset or `raw` → the default six-permutation layout, **byte-identical** to before this hook existed; `compressed` → re-encode into the block-compressed in-RAM mode (`Graph::into_compressed`: block-compressed permutations + blob dictionary) after load; **any other value is a hard error (exit 2, fail-closed)** — a typo never silently falls through to raw. Query solutions are **identical** across profiles (a memory trade, never a semantic change). **Honest scope: this reduces steady-state RESIDENT footprint only — it does NOT reduce load-time peak RSS,** because `into_compressed` builds the raw perms first and encodes from them, so the peak still includes the raw build. For a genuinely peak-bound load, use the external build (`SPARQ_BUILD_COMPRESSED=1` + `open`) instead. Measure the trade with `memstat [compressed]` (see above). Not a cargo feature — the compressed code paths are compiled unconditionally; the choice is per-workload, not per-build (see `research/compressed-memory-profile.md` §2).
- **External-build env vars (native, `dict-spill` feature):** `SPARQ_DICT_SPILL=1` spills the term dictionary during `build` (N-Triples only) to bound peak RSS; tune with `SPARQ_DICT_SPILL_BUDGET_MB` (default ¼ RAM) and `SPARQ_DICT_SPILL_DISK_FLOOR_MB` (default 1024, aborts before filling disk). Output is byte-identical. `SPARQ_BUILD_COMPRESSED=1` makes `build` emit block-compressed (`SPQCPRM1`) perms in one pass (no later `recompress`; raw is the default, output byte-identical to build-then-recompress). Also `SPARQ_NO_PREFETCH=1` for the `bench-remap` probe.
- **Format ↔ ingest path:** N-Triples streams block-by-block (parallel parse, no full decompressed copy in RAM); Turtle/N-Quads/TriG are buffered whole for the parallel statement-splitter. zstd decompresses much faster than bzip2 (see the `fused-decompress-parse` skill for measured numbers) — recompress `.bz2` sources once with `zstd -9 -T0` for big ingests.
- **Compressed-perm dirs** written by `save ... compressed` / `recompress` are auto-detected by `query-mmap`/`bench-mmap`; `bench-mmap ... decompress` decodes them to RAM first.

## See also

- `core` — the `sparq_core::Graph` library API (load/save/open, the store, the Dict) used under the hood.
- `engine` — `sparq_engine::{query, count, query_json}` and query budgets / prepared queries.
- `reason` — RDFS/OWL-RL/N3 materialization (`sparq_reason`) invoked by `--reason` and the `reason` subcommand.
- `inference` — the reasoning surfaces as a whole, including the opt-in OWL 2 EL classifier (`sparq_reason_el`) behind `--reason el` / `classify`, and *why* RL cannot substitute for it.
- `hdt` — the opt-in HDT loader (`sparq_hdt::load`).
- `hdt-write` — the opt-in `to-hdt` HDT exporter (`sparq_hdt::save`); implies `hdt`.
- `server` — the HTTP SPARQL endpoint (`sparq-server`) for serving instead of one-shot CLI queries.
