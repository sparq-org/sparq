---
name: data-formats
description: Parse and load RDF into a sparq Graph (Turtle/N-Triples/N-Quads/TriG via sparq-core, and HDT incl. compressed .hdt.gz/.hdt.zst/.hdt.bz2 via sparq-hdt), do streaming/parallel/external-memory ingest of compressed dumps, and take cheap immutable copy-on-write Graph snapshots. Use when ingesting RDF files, choosing a loader, wiring HDT, or snapshotting a graph for serving.
---

# sparq data formats

How to get RDF *into* a sparq `Graph` and how to snapshot one cheaply. Text formats
(Turtle / N-Triples / N-Quads / TriG) and the in-memory + streaming + external-memory
loaders live in `sparq-core`; the binary HDT archive format (including content-sniffed
`.hdt.gz` / `.hdt.zst` / `.hdt.bz2`) lives in the opt-in `sparq-hdt` crate.

> Direction note: these crates **parse RDF in**. To write RDF *out*, sparq-engine ships the
> **RDF writer matrix** behind its opt-in `serialize-rdf` feature — Turtle / TriG / N-Quads /
> JSON-LD 1.1 writers (`sparq_engine::serialize::*`), plus deterministic **pretty** (indented)
> variants — Turtle / TriG (`graph_to_turtle_pretty`, the long-term engine home for the site
> formatter) and JSON-LD (`graph_to_jsonld_pretty`); the N-Triples writer (`triples_to_ntriples`)
> is always on. See recipe 6.

## Quickstart

Add the dependency (HDT is a separate, native-only crate):

```toml
# Cargo.toml
[dependencies]
sparq-core = "0.1"            # default features include `parallel` (native)
sparq-hdt  = "0.1"            # OPTIONAL — only if you load .hdt archives
# To filter an HDT during its SPO decode instead of building the full Graph:
# sparq-hdt = { version = "0.1", features = ["load-filter"] }
oxrdf      = { version = "0.3", features = ["rdf-12"] }  # for Term/NamedNode at call sites
```

```rust
use sparq_core::Graph;

// `format` (plus media-type aliases): "turtle"|"ttl"|"text/turtle"|"application/turtle" |
//   "ntriples"|"n-triples"|"nt"|"application/n-triples" | "nquads"|"n-quads"|"nq"|"application/n-quads" |
//   "trig"|"application/trig". An UNRECOGNISED format string is an `Err` — NOT silently
//   parsed as Turtle (sq-m2pc). JSON-LD ("jsonld"/"json-ld"/"application/ld+json") needs the
//   `jsonld` feature on the `sparq-core` LIBRARY dep (OFF by default — the library stays lean);
//   without it those strings also error rather than mis-parsing. [OPUS-4.8] sq-oy1f.4: the
//   `sparq-cli` and `sparq-server` BINARIES enable `jsonld` by DEFAULT (a maintainer-directed
//   exception), so they read/write JSON-LD out of the box; a library embedder opts in explicitly.
//   [OPUS-4.8] sq-f47w1 (survey §B1): RDF/XML ("rdfxml"/"rdf-xml"/"application/rdf+xml")
//   likewise needs the OPT-IN `rdfxml` feature on `sparq-core` (OFF by default — it links
//   `oxrdfxml`/`quick-xml`, kept off the lean wasm bundle); without it those strings error
//   rather than mis-parsing. RDF/XML has no named-graph syntax, so it loads via `load_str` /
//   `parse_to_triples` (not the dataset path), serial-only (XML is not line-delimited).
let ttl = r#"@prefix ex: <http://ex/> .
ex:alice ex:knows ex:bob ."#;
let g = Graph::load_str(ttl, "turtle").expect("parse");
assert_eq!(g.len(), 1);

// HDT archive (sniffs .hdt.gz/.hdt.zst/.hdt.bz2 by magic bytes, not file name):
// let g = sparq_hdt::load("dataset.hdt").unwrap();
```

Or from the CLI:

```bash
sparq-cli query data.ttl turtle 'SELECT * WHERE { ?s ?p ?o } LIMIT 5'
# HDT needs the opt-in feature (its MSRV is 1.87, above the 1.85 workspace floor):
cargo build -p sparq-cli --features hdt
./target/.../sparq-cli query dataset.hdt hdt 'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'
```

## Key APIs

`sparq_core::Graph` — construction / ingest (all take a `format: &str`):

```rust
// In-memory, whole document in a &str. Parallel-chunked for N-Triples & Turtle
// when the `parallel` feature is on (default native); on ≥2 threads both fan the
// per-chunk dictionaries into one sharded-dict merge (the serial `merge_remap`
// loop is kept only on a single thread). Output is identical to the serial parse.
pub fn load_str(text: &str, format: &str) -> Result<Graph, String>
pub fn load_str_with_base(text: &str, format: &str, base: &str) -> Result<Graph, String>

// Dataset load (N-Quads / TriG) preserving NAMED GRAPHS as separate sub-graphs
// (so `GRAPH ?g {…}` works). Other formats defer to load_str. In-memory only.
// N-Quads is parallel-chunked (per-graph routing + sharded-dict merge) when the
// `parallel` feature is on (default native); TriG uses the serial path. Named
// graphs come out in first-occurrence document order (deterministic).
pub fn load_dataset(text: &str, format: &str) -> Result<Graph, String>

// [FABLE-5] sq-tonhr.2 — load_dataset with a base IRI for the document's relative
// IRIs (the DATASET companion to load_str_with_base): named graphs preserved,
// SERIAL parse (base-relative docs are small — manifests, W3C test actions; the
// rdf-trig conformance lane drives this). N-Quads has no relative IRIs (base
// ignored); non-dataset formats defer to load_str_with_base.
pub fn load_dataset_with_base(text: &str, format: &str, base: &str) -> Result<Graph, String>

// Streaming from any reader (e.g. a decompression stream) — serial.
pub fn load_reader<R: std::io::Read>(reader: R, format: &str) -> Result<Graph, String>

// Streaming + PARALLEL (needs `parallel`): pipelined 32 MiB block parse, never
// materializes the whole decompressed doc. N-Triples gets this parallel pipelined
// path; other formats fall back to serial load_reader. (This is the PARALLEL-LOADER
// fast path — distinct from the byte-level IRI-validation fast path noted below, which
// is a separate, opt-in mechanism that covers N-Triples AND N-Quads.)
pub fn load_reader_parallel<R: std::io::Read + Send>(reader: R, format: &str) -> Result<Graph, String>

// [OPUS-4.8] sq-7d3dj.18 — the byte-level N-Triples/N-Quads fast path does NOT validate IRIs
// against RFC-3987 by default (it trusts input; the serial oxttl path DOES validate). The
// OPT-IN `iri-fast` feature on `sparq-core` (OFF by default) turns that validation on for the
// byte parser via a prefix-memoized fast path IN FRONT of `oxiri` — a last-N validated
// `scheme://authority/` memo + a one-pass ASCII `iunreserved`/sub-delims suffix scan, falling
// back to the full `oxiri` automaton on any non-ASCII / `%`-escape / `#` / delimiter anomaly.
// It accepts EXACTLY what `oxiri::Iri::parse` accepts (an absolute IRI) — a MANDATORY
// differential-fuzz gate (`sparq_core::iri`, run under `--features iri-fast`) asserts
// `fast_accept ⇒ oxiri` and `validate == oxiri` over a committed adversarial corpus + a
// deterministic generator, so a malformed IRI can never be wrong-accepted into the store.
// The `oxiri` dep is already in-tree (via oxttl), so the feature adds zero new compilation.

// [OPUS-4.8] sq-jocpn — the OPT-IN `native-ttl` feature on `sparq-core` (OFF by default) swaps the
// oxttl Turtle path for a hand-rolled byte-level tokenizer/parser (`sparq_core`'s `ttl` module) that
// interns S/P/O directly into the Dict — the Turtle analogue of the byte-level N-Triples parser. It
// handles the FULL Turtle grammar (prefixes/@base, collections `()`, blank-node property lists `[]`,
// predicate-object lists `; ,`, `a`, numeric/boolean/triple-quoted literals, escapes) PLUS the RDF
// 1.2 surface the workspace oxttl enables (reifiers `<< … >>`, triple terms `<<( … )>>`, `~` and
// `{| … |}` annotations, all desugared to `rdf:reifies`). It is a BYTE-IDENTICAL drop-in: IRI base
// resolution delegates to the same `oxiri` automaton oxttl uses, so resolved terms match exactly,
// and it passes an IDENTICAL pass/fail set on the whole W3C rdf-turtle suite. Correctness is pinned
// by (a) a native-vs-oxttl DIFFERENTIAL over many constructs + malformed-input parity (both must
// Err), and (b) the conformance ratchet run with `--features sparq-core/native-ttl`
// (`sparq-conformance`'s `native-ttl-suite`). Only anonymous blank-node labels differ (as between
// two oxttl runs), so comparisons are blank-node-isomorphic. It is opt-in so it can be A/B'd against
// oxttl before any thought of defaulting on; on a quiet box the native single-thread parse is
// meaningfully faster (it removes oxttl's tokenizer/state-machine, ~53% of the incumbent parse per
// the sq-wrn61 profile). Reproduce: `cargo test -p sparq-core --features native-ttl`.

// Parse WITHOUT building indexes — the seam reasoning/transform hooks use.
pub fn parse_to_triples(text: &str, format: &str) -> Result<(Dict, Vec<[Id; 3]>), String>
pub fn from_parts(dict: Dict, triples: Vec<[Id; 3]>) -> Graph

// EXTERNAL-MEMORY build (needs `mmap`): stream-parse, spill sorted runs to disk,
// k-way merge — bounded RAM for datasets larger than memory. `chunk` = triples per run.
pub fn build_external<R: std::io::Read + Send>(reader: R, format: &str, dir: &Path, chunk: usize) -> Result<(), String>

// Empty, in-memory graph — the trivial infallible constructor (no `load_str("", …)`
// workaround, no error handling). `default()` and `new()` are interchangeable.
pub fn new() -> Graph                 // also: Graph::default()
```

`sparq_core::Dict` — compact FILTERED rebuild on the `(Dict, triples)` seam:

```rust
// [GPT-5.6] sq-eiv — consume the Dict, keep only the real ids `retain` accepts, and get
// back a dense rebuilt Dict plus an old→new remap (`remap[(old_id - 1) as usize]`; NO_ID
// = dropped) for rewriting triple ids before `from_parts`. Inline integer ids are not
// dictionary records: never passed to `retain`, unchanged under remapping. Dependency-aware
// for RDF 1.2 triple terms — retaining a triple term implicitly retains its s/p/o and any
// recursively nested components (their remap entries are populated even if `retain` said
// false). Never materialises an `oxrdf::Term` and never re-interns survivors (arena
// payloads MOVE; blob/mmap/frozen records copy compactly); dead prefix/datatype metadata
// is filtered too, so it does not accumulate across rebuilds.
pub fn rebuild_filtered(self, retain: impl FnMut(Id) -> bool) -> (Dict, Vec<Id>)
```

`sparq_core::Graph` — single-triple mutation from `oxrdf` terms (the ergonomic one-triple
case over `apply_delta`; each position takes anything that converts into a `Term`:
`NamedNode` / `Literal` / `BlankNode` / a `Term`). Set-valued (re-insert / absent-remove is a
no-op) and, for a directory-backed graph, WAL-logged + fsync'd — same semantics as `apply_delta`.
For several triples at once prefer ONE `apply_delta` batch (one WAL append) over a loop:

```rust
use sparq_core::Graph;
use oxrdf::{NamedNode, Literal, Term};

let mut g = Graph::new();
g.insert_triple(
    NamedNode::new_unchecked("http://ex/alice"),
    NamedNode::new_unchecked("http://schema.org/name"),
    Term::Literal(Literal::new_simple_literal("Alice")),
)?;
g.remove_triple(
    NamedNode::new_unchecked("http://ex/alice"),
    NamedNode::new_unchecked("http://schema.org/name"),
    Term::Literal(Literal::new_simple_literal("Alice")),
)?; // absent ⇒ no-op, never an error
```

`sparq_core::Graph` — cheap snapshots / forks (the serving pattern):

```rust
pub fn snapshot(&self) -> GraphSnapshot   // immutable COW view; O(pending delta), not O(triples)
pub fn fork(&self) -> Graph               // mutable structural fork (Arc-shares base storage)
```

`sparq_core::GraphSnapshot` — `Send + Sync`, `Deref<Target = Graph>` (every read method:
`len`, `id_of`, `pattern`, `iter_ids`, `store`/`dict`), **no** mutating methods.
`into_graph(self) -> Graph` / `as_graph(&self) -> &Graph`.

`sparq_hdt` (native-only; opt-in):

```rust
pub fn load(path: impl AsRef<Path>) -> Result<Graph, Error>         // sniffs .hdt.gz/.hdt.zst/.hdt.bz2
pub fn load_reader<R: BufRead>(reader: R) -> Result<Graph, Error>   // any buffered source

// [GPT-5.6] sq-lsp7k.24 — opt-in `load-filter`; None means wildcard. Filtering
// runs during the one-shot SPO walk and only accepted triples' terms enter the
// returned Graph. An all-None pattern is identical to load_reader.
#[cfg(feature = "load-filter")]
pub type TriplePattern = (
    Option<oxrdf::NamedOrBlankNode>, // subject
    Option<oxrdf::NamedNode>,        // predicate
    Option<oxrdf::Term>,             // object
);
#[cfg(feature = "load-filter")]
pub fn load_reader_filtered<R: BufRead>(reader: R, pattern: &TriplePattern) -> Result<Graph, Error>
#[cfg(feature = "load-filter")]
pub fn stats_reader_filtered<R: BufRead>(reader: R, pattern: &TriplePattern) -> Result<HdtStats, Error>

pub fn header(path: impl AsRef<Path>) -> Result<Graph, Error>       // HDT header metadata as a Graph
pub fn header_reader<R: BufRead>(reader: R) -> Result<Graph, Error>
pub fn stats(path: impl AsRef<Path>) -> Result<HdtStats, Error>     // dictionary counts + exact triples
pub fn stats_reader<R: BufRead>(reader: R) -> Result<HdtStats, Error>
pub struct HdtStats {
    pub triples: usize, pub shared: usize, pub subjects_only: usize,
    pub objects_only: usize, pub predicates: usize,
}
// HdtStats also provides distinct_subjects() and distinct_objects().
// also: load_reader_via_upstream (differential oracle), graph_from_hdt, graph_from_reader

// MEASUREMENT-ONLY (bench/parse's HDT stage split — NOT a production loader):
// identical decode to graph_from_reader, but records a per-stage wall-clock split.
// Production callers use the plain loaders above; the decode is byte-for-byte the
// same (timing only reads Instant::now() at the existing stage boundaries), so
// decoder behaviour is unchanged.
//   COARSE three: dict (control/header + 4-section PFC decode+intern+merge), scan
//   (triples-section read + SPO id-translation walk), build (Graph::from_parts).
//   FINER (sq-7ge0): dict_{shared,subjects,objects,predicates} are the four per-PFC-
//   section decode walls — in a multi-threaded pool they run CONCURRENTLY so they
//   OVERLAP and need NOT sum to `dict`; dict_merge is the section-order merge. scan
//   splits into scan_read (triples bitmaps/sequences) + scan_walk (the SPO walk),
//   which sum EXACTLY to `scan`.
pub struct StageTimings {
    pub dict: Duration, pub scan: Duration, pub build: Duration,
    pub dict_shared: Duration, pub dict_subjects: Duration, pub dict_objects: Duration,
    pub dict_predicates: Duration, pub dict_merge: Duration,
    pub scan_read: Duration, pub scan_walk: Duration,
}
pub fn graph_from_reader_timed<R: BufRead>(reader: R, t: &mut StageTimings) -> Result<Graph, Error>

// WRITE (opt-in `write` feature): Graph -> .hdt (honours .gz/.zst/.bz2 by extension)
// CLI: `sparq-cli to-hdt <file> <in-fmt> <out.hdt[.gz|.zst|.bz2]>` (opt-in `hdt-write` feature)
#[cfg(feature = "write")]
pub fn save(graph: &Graph, path: impl AsRef<Path>) -> Result<(), Error>
```

## Common recipes

**1. Load a compressed N-Triples dump with the fast parallel streaming path.**
Decompress on the fly and parse in parallel without ever holding the full text:

```rust
use sparq_core::Graph;
let file = std::fs::File::open("dump.nt.gz")?;
let reader = flate2::read::MultiGzDecoder::new(file);     // .bz2 -> bzip2::read::MultiBzDecoder
                                                          // .zst -> zstd::stream::read::Decoder::new
let g = Graph::load_reader_parallel(reader, "ntriples")?; // N-Triples gets the pipelined parser
```

**2. Load a dataset with named graphs (TriG / N-Quads).**

```rust
let trig = r#"<http://ex/g> { <http://ex/a> <http://ex/p> <http://ex/b> . }"#;
let g = sparq_core::Graph::load_dataset(trig, "trig")?;
assert_eq!(g.named.len(), 1);          // each named graph is its own sub-Graph
```

**3. Load an HDT archive (and read its header metadata).**

```rust
let g    = sparq_hdt::load("dataset.hdt")?;        // .hdt / .hdt.gz / .hdt.zst / .hdt.bz2 (by magic bytes)
let meta = sparq_hdt::header("dataset.hdt")?;      // VoID stats / provenance as a queryable Graph
let stats = sparq_hdt::stats("dataset.hdt")?;      // exact counts without a full graph decode
assert_eq!(stats.distinct_subjects(), stats.shared + stats.subjects_only);
let bytes = std::fs::read("dataset.hdt")?;
// from memory: sparq_hdt::load_reader(std::io::Cursor::new(bytes.clone()))?
// Predicate-filtered from memory (needs `load-filter`; None is a wildcard):
let pattern: sparq_hdt::TriplePattern = (
    None,
    Some(oxrdf::NamedNode::new("http://www.w3.org/2000/01/rdf-schema#label")?),
    None,
);
// [GPT-5.6] sq-obhf1: count matches without constructing a result Graph/Dict.
let label_stats = sparq_hdt::stats_reader_filtered(std::io::Cursor::new(bytes.clone()), &pattern)?;
let labels = sparq_hdt::load_reader_filtered(std::io::Cursor::new(bytes), &pattern)?;
assert_eq!(label_stats.triples, labels.len());
// WRITE back (opt-in `write` feature): sparq_hdt::save(&g, "out.hdt.gz")?;
//   (currently a temp-N-Triples round-trip through the wrapped builder — see
//    sparq-hdt/UPSTREAM.md for the queued in-memory-builder contribution)
```

**4. Out-of-core build for a dataset larger than RAM** (requires `sparq-core` `mmap` feature):

```rust
use std::path::Path;
let reader = flate2::read::MultiGzDecoder::new(std::fs::File::open("huge.nt.gz")?);
// stream-parse + external-sort the six permutations onto disk; ~16M triples per spill run:
sparq_core::Graph::build_external(reader, "ntriples", Path::new("/data/idx"), 16_000_000)?;
let g = sparq_core::Graph::open(Path::new("/data/idx"))?;   // query with indexes memory-mapped
```

**5. Cheap snapshot for serving (one mutable master + immutable readers per commit).**
`snapshot()` Arc-shares the base indexes/dictionary — it copies neither, and later
mutation of the master is invisible to the snapshot:

```rust
let mut master = sparq_core::Graph::load_str(doc, "turtle")?;
let snap = master.snapshot();          // immutable, Send+Sync, point-in-time
master.apply_delta(&inserts, &deletes)?;   // master moves on
assert_eq!(snap.len(), original_len);  // snapshot frozen at snapshot time
// publish `snap` to query threads; it Derefs to &Graph
let mutable_copy = master.snapshot().into_graph();  // a snapshot you can then mutate
```

**6. Serialize a graph back out (the RDF writer matrix).** The Turtle / TriG / N-Quads /
JSON-LD writers live in `sparq-engine` behind the **`serialize-rdf`** feature (it pulls
in ZERO new dependencies — the default dep graph is byte-for-byte unchanged when off, *and*
unchanged even when on: the JSON-LD writer emits JSON by hand, no json-ld/serde crate). For a
LIBRARY embedder it is opt-in (the engine library default stays lean): enable with
`sparq-engine = { version = "0.1", features = ["serialize-rdf"] }`. The `sparq-cli` and
`sparq-server` BINARIES pull it into their default build via the default-on `jsonld` feature
([OPUS-4.8] sq-oy1f.4), so `dump …` and the server's `application/ld+json` work out of the box.
The N-Triples writer (`triples_to_ntriples`) is always on. Internally the writer matrix is
housed in the `sparq-engine-serialize` sub-crate (`publish = false`, [FABLE-5] sq-6vshe.4) and
re-exported verbatim — the `sparq_engine::serialize::*` paths and feature names above are
unchanged and remain the supported surface.

```rust
use sparq_engine::serialize::{graph_to_turtle, graph_to_trig, graph_to_nquads,
                              graph_to_jsonld, graph_to_jsonld_pretty, JsonLdForm};

let g = sparq_core::Graph::load_dataset(trig_src, "trig")?;
let ttl = graph_to_turtle(&g);   // Turtle: @prefix header, `a` for rdf:type, predicate-object
                                 //   lists; DEFAULT graph only.
let tg  = graph_to_trig(&g);     // TriG: default graph + `GRAPH <g> { … }` blocks (whole dataset).
let nq  = graph_to_nquads(&g);   // N-Quads: default graph (3 cols) + named graphs (4th column).
let jx  = graph_to_jsonld(&g, JsonLdForm::Expanded);    // JSON-LD 1.1, fully-expanded node-object
                                                        //   array, no @context (whole dataset).
let jf  = graph_to_jsonld(&g, JsonLdForm::Flattened);   //   node-merged, `@graph`-framed.
let jc  = graph_to_jsonld(&g, JsonLdForm::Compacted);   //   basic prefix `@context` (default_prefixes).
let jp  = graph_to_jsonld_pretty(&g, JsonLdForm::Expanded); // same docs, indented (multi-line).
```

Lower-level entry points take `&[oxrdf::Triple]` (e.g. CONSTRUCT output) directly:
`write_turtle(triples, &prefixes)`, `write_trig(&named_graphs, &prefixes)`,
`write_nquads(&named_graphs)`, `write_jsonld(&named_graphs, JsonLdForm::Expanded, &prefixes)`;
`default_prefixes()` supplies the common namespaces, or pass your own `Prefixes` (a
`BTreeMap<String, String>`) — `prefixes_from_pairs([(prefix, iri), …])` builds one from an
ordered `(prefix, iri)` pair list (e.g. a query's parsed `PREFIX` lines or a `[[prefix, iri], …]`
array). The `graph_to_*_with` convenience wrappers (`graph_to_turtle_with`, `graph_to_trig_with`,
`graph_to_jsonld_with`, and the pretty `*_with`) take that same map, so the whole-graph path can
also serialise under an explicit prefix policy. Only prefixes actually used are emitted (the
Turtle/TriG header, or the JSON-LD compacted `@context`). Round-trip (parse → serialize → re-parse) is isomorphic
for every form. **JSON-LD specifics:** `xsd:string`/`rdf:langString` stay implicit
(`@value` + optional `@language`); every other datatype is preserved as `@type`; canonical
`xsd:integer`/`xsd:boolean` literals coerce to native JSON scalars only when lossless (leading
zeros, `xsd:double`/`decimal`, etc. stay typed strings). A *well-formed, single-referenced*
`rdf:first`/`rdf:rest`/`rdf:nil` blank-node chain is collapsed into a native `@list` array
(nested lists collapse recursively); anything that fails the conservative safety predicate —
a list cell referenced more than once, carrying an extra predicate, cyclic, or not terminated
by `rdf:nil` — is left as ordinary `rdf:first`/`rdf:rest` triples, so the round-trip stays
lossless either way (the empty list `()` stays an `rdf:nil` reference, never `@list`).

**Comparative throughput** for the writer matrix is measured by `bench/serialize/run.sh`
(registered `serialize-bench`, [FABLE-5] sq-hmd7l.14): sparq's buffered/streaming/pretty
regimes in-process, plus a cross-engine pipeline panel vs serd/rapper/Jena riot/oxrdfio —
every emitted document round-trip-gated (re-parse == source store) before its timing row
is trusted. First-read analysis: `research/gap-serialize-2026-07.md`.

**Pretty Turtle / TriG (idiomatic, deterministic).** Alongside the plain writers, the same
feature exposes a *pretty* variant — `graph_to_turtle_pretty(&g)` / `graph_to_trig_pretty(&g)`,
or the lower-level `write_turtle_pretty(triples, &prefixes, &opts)` /
`write_trig_pretty(&named_graphs, &prefixes, &opts)` with a `PrettyOptions { indent, abbreviate }`
(default: two-space indent, abbreviation on). Two differences from `graph_to_turtle`: (1) output
is **emission-order-independent** — subjects, predicates and objects are sorted by their
canonical N-Triples spelling and `rdf:type` (`a`) sorts first in each block, so the same triple
SET always renders to the same bytes (the plain writer preserves the store's `iter_ids()` order,
which is thread-count-dependent); (2) the **output shape** matches the site's `prettyTurtle`
reshaper (`packages/sparq-client/src/pretty-turtle.ts`, sq-gb4o / #805) — a prefix-alphabetical
used-only `@prefix` header, blank-line-separated subject blocks, the subject on its own line,
predicate lines at one indent (`;`-joined, `.`-terminated), and object lists wrapped onto
continuation lines (`,\n` + two indents). It is round-trip-correct (re-parses to the same triple
set) and reuses the same escaping/term helpers, so literals (datatype/lang, implicit `xsd:string`
dropped), blank nodes, and RDF 1.2 triple terms `<<( s p o )>>` are all reproduced losslessly.
This is the long-term engine home for the site-side TS formatter; the wasm-surface exposure
(so the site can drop its formatter) is a deferred follow-up.

**Pretty (indented) JSON-LD.** The JSON-LD writers have a matching pretty variant —
`graph_to_jsonld_pretty(&g, form)` / `write_jsonld_pretty(&named_graphs, form, &prefixes, &opts)`,
or the `graph_to_jsonld_pretty_with(&g, form, &prefixes, &opts)` wrapper — taking
`JsonLdPrettyOptions { indent }` (default: two-space indent). Unlike the Turtle pretty writer
this is **whitespace-only**: it produces the *exact* minified JSON-LD document and re-indents it
(each `{`/`[` opens an indented level, each member/element on its own line; empty `{}`/`[]` stay
compact). So it reuses the minified writer's already-deterministic first-seen subject/predicate
ordering unchanged, round-trips to the identical RDF, and stays dependency-free (a small
hand-written JSON re-indenter — no `serde_json`, which is dev-only). Indentation is presentation
only; the document content is byte-for-byte the minified form plus newlines/indent.

**Streaming Turtle / TriG (opt-in `streaming-serialization`, implies `serialize-rdf`).** The plain
`write_turtle` / `write_trig` build a *full* in-memory `String` for the whole graph, so a large
CONSTRUCT/DESCRIBE holds both the materialised triples and the fully-rendered string at once. The
opt-in `streaming-serialization` feature adds `write_turtle_streaming(triples, &prefixes, &mut w)`
and `write_trig_streaming(&named_graphs, &prefixes, &mut w)` (plus the whole-graph
`graph_to_turtle_streaming(&g, &prefixes, &mut w)` / `graph_to_trig_streaming`) that render the body
directly into any `W: std::io::Write`, buffering only **one subject block at a time** (emitting on
subject change) — so the whole rendered output is never materialised, enabling HTTP chunked
CONSTRUCT/DESCRIBE responses (first bytes flushed after the first subject, not the last). The
streamed bytes are **byte-identical** to the buffered `write_turtle` / `write_trig` for the same
graph (same used-prefix header, same subject grouping, same ordering): both share the prefix-header
and per-subject-block rendering, and graph-sourced triples are subject-contiguous (`iter_ids()` walks
the SPO permutation), which is the precondition for that equality. The memory / time-to-first-byte
payoff is a measurable hypothesis on the canonical host, not a baked-in number. Zero new deps (std
`io::Write` only). Reproduce: `cargo test -p sparq-engine --features streaming-serialization`.

**Full W3C JSON-LD 1.1 Compaction (caller `@context`).** `JsonLdForm::Compacted` above is the
*lighter* prefix-only `@context` (it just abbreviates IRIs to `prefix:local` CURIEs). For the
real **W3C JSON-LD 1.1 Compaction Algorithm** against a caller-supplied `@context`, use
`graph_to_jsonld_compact(&g, &ctx)` (or the slice-level `write_jsonld_compact(&named_graphs, &ctx)`)
— still hand-rolled and **dependency-free** (a tiny internal `Json` AST — `JsonLdValue` — no
`serde_json`, no json-ld crate). It implements: **term definitions** (`{"name":"http://…/name"}`
or expanded `{"@id":…,"@type":…,"@language":…,"@container":…}`), **`@vocab`** (bare vocab-relative
terms for predicates and `@type` values), **type coercion** (a term `@type` matching a value's
datatype collapses the value object to a bare scalar; `@type:@id`/`@vocab` collapse a node
reference to a bare IRI string), **language coercion** (a term `@language` or the document default
`@language` drops a matching `@language`), **`@container`** — `@set` (always an array, no
compact-arrays), `@list` (strips the `{"@list":…}` wrapper to a bare ordered array — but only the
*pure* `{"@list":…}` value: any co-located non-list sibling stays under the property IRI, never
dropped, sq-oy1f.8), `@language` (a `{lang: value(s)}` map — a value with **no** language tag falls
under the reserved `@none` member rather than being dropped, sq-oy1f.9; several values that share
one language slot accumulate into an **array** so none is overwritten, and a **non-string** value —
a typed/numeric literal — routes to a separate non-language key so a strict processor keeps its
datatype rather than rejecting an invalid language-map value, sq-oy1f.12/.14) and `@index` (a
`{index: value(s)}` map; fromRdf has no per-value index, so values share the reserved `@none`
slot, accumulating into an array, sq-oy1f.14). `@id` / `@graph` containers — which the fromRdf
model cannot losslessly populate — **fall back to the default (no-container) framing** rather than
emit a container map a strict processor rejects (sq-oy1f.14). **`@reverse`** terms (forward edges
whose predicate a reverse term maps relocate onto the object node and are emitted as a **forward
member keyed by the reverse term**, NOT inside an `@reverse` block — a reverse-term key inside an
`@reverse` block would double-invert and a strict processor reads the edge backwards, sq-oy1f.12;
an edge whose object is *not* itself a subject is kept as a forward property, never stripped,
sq-oy1f.10), **`@id`/`@type` keyword aliasing** (`{"id":"@id","type":"@type"}`), and **value + node
+ IRI compaction** against the active context (vocab-relative compaction emits the `@vocab`-stripped
form even when the suffix contains `/` or `#` — only a `:` suffix is kept full, since it is ambiguous
with a compact IRI on read-back, sq-oy1f.11; a plain **literal** value under a `@type:@id`-coerced
term moves to a non-coerced key so it does not read back as a node IRI, sq-oy1f.13). Build the
context with `parse_context_json(r#"{…}"#)` (a string → `JsonLdValue`, returns `None` if not a JSON
object) or construct the `JsonLdValue` directly; the parsed `ActiveContext` drives compaction. The
output is a `{"@context":…,"@graph":[…]}` document and the compaction is **lossless** — every
coercion is invertible against the same `@context`, so a JSON-LD-to-RDF round-trip reconstructs the
original triples. *Scope:* this is the fromRdf-then-compact (serialise) path — sparq always emits
RDF, so the input is a `Graph`, not an arbitrary remote document; scoped/typed contexts,
`@propagate`, remote `@context` fetching, `@import`, `@protected` are out of scope (JSON-LD
**Framing** is its own recipe below, sq-oy1f.17). *Strict third-party faithfulness:* the compaction
round-trip is verified both against sparq's own JSON-LD→RDF reader **and** differentially against the
**pyld** W3C reference processor (`expand`/`toRdf`) for the `@reverse`, language-map, `@type:@id`,
and multi-value-container shapes (sq-oy1f.12/.13/.14), so the emitted document re-expands to the
original triples in a strict external processor, not only in sparq. (An `@id`/`@graph`-container
*input* — i.e. compacting an already-indexed document, as distinct from the fromRdf graph sparq
produces — remains out of scope and falls back to default framing as noted above.)

*Native document-level pipeline (in progress, opt-in, not yet wired to the surfaces).* The
scope gaps above (scoped/typed contexts, `@propagate`, `@protected`, remote `@context` +
`@import`) are being closed by a from-scratch, dependency-free **document-level** pipeline in
the `sparq-jsonld` crate (design record `research/jsonld-1.1-design.md`, epic sq-oy1f). Its
Context Processing foundation (sq-oy1f.24) ships today: `sparq_jsonld::context::ActiveContext`
with `process()` (Context Processing + Create Term Definition, fallible with exact spec error
codes), `expand_iri()` (IRI Expansion), RFC 3986 reference resolution, and **deny-by-default**
remote-context loading via `DocumentLoader` (`NoopLoader` refuses every fetch — no ambient
network — while `FsLoader` serves trusted local fixtures). The document-level **Expansion
Algorithm** (sq-oy1f.25) ships too: `sparq_jsonld::expand(&input, &opts, &loader)` returns the
canonical expanded array — Value Expansion, scoped (property/type) contexts,
`@index`/`@id`/`@type`/`@language`/`@graph` container maps, `@nest`, `@reverse`, `@included`,
`@json` literals, keyword aliases, and the drop-null + array-normalisation rules — raising the
exact spec error code on invalid input and threading `frameExpansion`. **Node Map Generation +
Flattening** (sq-oy1f.26) ship next: `sparq_jsonld::generate_node_map()` (§7.2, deterministic
`_:bN` blank-node issuer) and `sparq_jsonld::flatten(&input, &opts, &loader)` (§7.1 = expand ∘
node-map ∘ named-graph fold, sorted by `@id`, empty nodes dropped), with the `flatten`
conformance lane moved to the native document oracle. The document-level **Compaction
Algorithm + Value Compaction** (sq-oy1f.27) ship too:
`sparq_jsonld::compact::compact(&input, &ctx, &opts, &loader)` (and `compact_expanded` over an
already-expanded document) runs the spec algorithm — scoped (property/type) contexts with
previous-context reversion, `@list`/`@language`/`@index`/`@id`/`@type`/`@graph` container
reshaping, `@nest`, `@reverse` redistribution, keyword aliasing, Value Compaction, and the
`compactArrays`/`compactToRelative`/`ordered` options — and the `compact` conformance lane now
compares against the W3C **expected** documents (the normative oracle; floor re-pinned 186 →
228, then raised to 243 by sq-gzsky RUNNING the negatives, see `floors::compact`). The
`expand` and `compact` lanes no longer SKIP `NegativeEvaluationTest`s: a negative passes iff
the algorithm raises EXACTLY the manifest's `expectErrorCode`, so a wrong code is a FAIL and
error-code completeness is now gated (expand floor 276 → 381). Framing on this
substrate, the surface wiring (the engine-serialize cutover, sq-oy1f.41), and the remaining
conformance-lane switches land in later beads; the RDF-first writers above remain the shipped
emit path until then.

```rust
use sparq_engine::serialize::{graph_to_jsonld_compact, parse_context_json};
let ctx = parse_context_json(r#"{
    "@vocab": "http://schema.org/", "id": "@id", "type": "@type",
    "knows": {"@id": "http://schema.org/knows", "@type": "@id"}
}"#).expect("a JSON object");
let doc = graph_to_jsonld_compact(&g, &ctx); // term defs + @vocab + @type:@id coercion applied
// Indented (multi-line) variant — whitespace-only over the minified document, same triples:
// graph_to_jsonld_compact_pretty(&g, &ctx, &JsonLdPrettyOptions::default()).
```

Exposed on the wasm `Store` as `serializeCompact(context, pretty, indent?)` (the JSON-LD
SKILL note) and on the CLI as the `jsonld-compact[-pretty]` `dump` out-format (sq-oy1f.5).

**Document-level JSON-LD 1.1 pipeline — the `sparq-jsonld` crate (epic sq-oy1f, in progress).**
The RDF-first writers above have a hard conformance ceiling (scoped/typed contexts, `@nest`,
`@index` maps, negative tests — structures an RDF projection has already erased). The full
document-level pipeline (Context Processing, Expansion, Flattening, Compaction, Framing,
from/to-RDF) is being built in the new **opt-in, dependency-free `sparq-jsonld` crate**
(`crates/sparq-jsonld`, design record `research/jsonld-1.1-design.md`). Phase A (sq-oy1f.23)
scaffolds it: the writer's `Json` AST moved out of `sparq-engine` into `sparq_jsonld::Json`
(public API preserved — `sparq_engine::serialize::JsonLdValue` re-exports it, byte-identical
output), alongside the full `JsonLdErrorCode` registry, `JsonLdOptions`, and a **deny-by-default**
`DocumentLoader` (the default `NoopLoader` refuses every remote load — no ambient network). The
crate is `serialize-rdf`-gated on the engine side, so the lean default + wasm builds are unchanged.
The algorithms land in the follow-on beads (sq-oy1f.24+); until then the writer above is the JSON-LD
emit path.

**W3C JSON-LD 1.1 Framing (caller frame).** `graph_to_jsonld_framed(&g, &frame)` (or the
slice-level `write_jsonld_framed(&named_graphs, &frame)`) reshapes the graph into a deterministic
tree matching a caller **frame** document, then compacts it against the frame's `@context`. Build
the frame with `parse_context_json` (a frame is a JSON-LD document). Hand-rolled and
dependency-free, same `Json` AST as compaction — no `serde_json`, no `json-ld` crate (no Rust
JSON-LD crate ships framing). It implements:

* **Node-pattern matching** — `@type` (explicit list / wildcard `{}` / match-none `[]`), property
  *presence* (`{}`), *absence* (`[]`), specific `@value` / `@id` match, and `@id` selection.
  `@requireAll: true` combines the frame's properties with AND; the default with OR (a `{}` frame matches all).
* **`@embed`** — `@once` (default: embed a node the first time it is reached result-wide, a
  `{"@id":…}` reference thereafter), `@always` (re-embed every reference), `@never` (always a
  reference); `@link` is treated as `@always`. A blank-node cycle (`a→b→a`) terminates via the
  embed link table (the back-edge becomes a reference).
* **`@explicit: true`** — keep only the properties the frame names. **`@default`** — the value
  emitted for a framed property absent from the matched node (`@default: @null` → a preserve-`null`
  marker). **`@omitDefault: true`** — drop an absent framed property instead of emitting its
  default/null. **List framing** keeps the `{"@list":…}` wrapper, framing each element.

Output is a `{"@context":…,"@graph":[…]}` document, collapsing to the bare framed node merged with
`@context` for a single matched root (the `omitGraph` default). Each named graph in the dataset is
framed against the same pattern (wrapped `{"@id":<graph>,"@graph":[…]}`). The framed shape is
**differential-tested byte-for-byte against the pyld reference processor's `frame`** across the
flag matrix (sq-oy1f.17), and **ratcheted against the official `w3c/json-ld-framing` suite**
(sq-oy1f.19; see the conformance bullet below) — each `jld:FrameTest` expanded input is framed and
compared by RDF-equivalence to the normative expected output, which quantifies the real gap. *Scope:*
the serialise (fromRdf-then-frame) path — sparq supplies the graph + an inline frame. The W3C suite
documents the current below-floor divergences honestly (value-pattern matching over `@value`
alternative arrays, some `@explicit`/`@default` and named-graph `@graph`-frame shapes); `@reverse`
frames and scoped/typed frame contexts remain follow-up beads (children of sq-oy1f).

```rust
use sparq_engine::serialize::{graph_to_jsonld_framed, parse_context_json};
let frame = parse_context_json(r#"{
    "@context": {"@vocab": "http://schema.org/"},
    "@type": "Person", "knows": {"@embed": "@never"}
}"#).expect("a JSON object");
let doc = graph_to_jsonld_framed(&g, &frame); // selects Person nodes; knows → bare {"@id":…}
```

From the CLI (opt-in `serialize-rdf` feature) — re-serialize a loaded document to stdout:

```bash
cargo build -p sparq-cli --features serialize-rdf
./target/.../sparq-cli dump data.trig trig nquads
#   out-format: turtle | turtle-pretty | trig | trig-pretty | nquads | ntriples
#              | jsonld[-expanded|-flattened|-compacted]
#              | jsonld-pretty[-expanded|-flattened|-compacted]
#              | jsonld-compact[-pretty]   (FULL W3C Compaction; needs --context <ctx.jsonld>)
#   (bare `jsonld` == jsonld-expanded; `turtle-pretty`/`trig-pretty` emit the deterministic,
#    idiomatic Turtle/TriG from recipe 6 — sorted, blank-line-separated subject blocks;
#    the `jsonld-pretty*` forms emit indented JSON-LD, bare `jsonld-pretty` == expanded)
# Full 1.1 Compaction against your own @context (term defs / @vocab / coercion / @reverse):
./target/.../sparq-cli dump data.ttl turtle jsonld-compact --context ctx.jsonld
```

## Gotchas / feature flags / prerequisites

- **Format strings are matched literally — an unknown format is REJECTED, not
  guessed.** Accepted aliases:
  `"turtle"`/`"ttl"`/`"text/turtle"`/`"application/turtle"`,
  `"ntriples"`/`"n-triples"`/`"nt"`/`"application/n-triples"`,
  `"nquads"`/`"n-quads"`/`"nq"`/`"application/n-quads"`, `"trig"`/`"application/trig"`
  (and the `"jsonld"`/`"json-ld"`/`"application/ld+json"` set when the `jsonld` feature is
  on). `parse_to_triples` (and the `_with_base` variants), `load_dataset`/`load_dataset_with_base`/`load_dataset_serial`
  gate on these explicit alias sets: any unknown/typo'd string returns
  `Err("unknown RDF format \"…\" (known: …)")` naming the bad format — it does **not**
  silently fall back to Turtle/TriG (the old `_ =>` catch-all was removed in sq-m2pc /
  sq-01yr; verify: `cargo test -p sparq-core parse_to_triples_rejects_unknown_format
  load_dataset_rejects_unknown_format`). Pass the exact string.
- **Parallel paths need the `parallel` feature** (on by default natively). The parallel
  fast path applies to N-Triples and Turtle in `load_str`; `load_reader_parallel`'s
  pipelined parser is **N-Triples only** (other formats silently fall back to serial
  `load_reader`). With `parallel` off (e.g. the wasm build, `--no-default-features`),
  everything parses serially. The parallel and serial parses are pinned **result-equivalent**
  by a differential test (`crates/sparq-core/tests/parallel_serial_load_differential.rs`,
  sq-bif.13): the same Turtle + N-Triples document loaded under `--features parallel` and under
  `--no-default-features` answers every term-level probe (triple/term counts, full dump,
  pattern scans) identically to a feature-independent reference graph.
- **RDF 1.2 triple terms are first-class.** A triple term `<<( s p o )>>` is a
  real `oxrdf::Term::Triple` (object position only — RDF 1.2 makes triple terms object-only;
  a `<<( … )>>` in subject/predicate position is rejected with a precise error). Triple terms
  may nest, take blank-node/literal components, and are content-addressed (an identical triple
  term shares one dict id). They load through **every** path — serial, parallel-chunked,
  streaming-pipelined, the sharded external builder, and the **dict-spill** external builder
  (sq-jvbr: triple-term occurrences take an in-RAM arena finalised after the spilled leaf
  consolidation, content-addressed by their components' final ids and assigned ids after every
  leaf) — at full parallelism (the in-memory parallel merge no longer drops to a serial
  fallback when they are present). In **Turtle/TriG**, the SPARQL-1.2
  reification sugar is supported via the Turtle parser: the reifying triple `<< s p o >>`
  (subject or object position, optionally `<< s p o ~ reifier >>`) and the annotation block
  `s p o {| … |}` desugar to the standard `rdf:reifies <<( s p o )>>` form (the annotation block
  also **asserts** the base triple; a bare `<< … >>` reifier does not). N-Triples/N-Quads carry
  only the desugared `<<( … )>>` triple-term form (no `<<>>`/`{| |}` sugar — per the line-format
  grammar). Nested triple terms are **depth-bounded** (`MAX_TRIPLE_TERM_DEPTH = 128` in
  `sparq-core::nt`): the byte-level N-Triples/N-Quads parser is the only native-recursion RDF
  parse path, so a pathologically nested `<<( … )>>` chain returns a clean parse error rather
  than overflowing the stack (ASVS V5.5.2 / sq-53s1). The Turtle/TriG/N-Quads path via `oxttl`
  is a heap-stack pushdown automaton and cannot recurse the native stack. The SPARQL parser has
  the matching `MAX_RECURSION_DEPTH = 128` cap (groups/expressions/paths/collections/triple
  terms). 128 is far deeper than any real data/query nests.
- **Language tags are normalised to lowercase on ingest.** A `@lang` tag is stored ASCII-lowercased
  across **every** format and path — Turtle/TriG/N-Quads via `oxttl`, and the byte-level
  N-Triples/N-Quads fast parser (`sparq-core::nt`). RDF 1.1/1.2 language tags are case-insensitive,
  so `"x"@en-US` and `"x"@en-us` are the **same** RDF term and intern to one dict id (matching
  `oxrdf::Literal::new_language_tagged_literal`). This makes ingest format-INDEPENDENT: the same
  triple written as N-Triples or as Turtle yields the identical stored term (it did **not** before —
  the byte path used to preserve the written case while every `oxttl` path lowercased it; KamiQuasi
  #1119 / sq-langcase). The RDF 1.2 `--ltr`/`--rtl` direction is unaffected (already lowercase).
- **`build_external` / `open` / `save` require the `mmap` feature** (native only). The
  N-Triples external path also honors `SPARQ_SHARDED_DICT` (default on with ≥2 threads)
  and, with the `dict-spill` feature + `SPARQ_DICT_SPILL` env, bounds peak build RSS by
  spilling the term dictionary to disk (byte-identical output). The spill path's activation
  + mmap-reload edges are pinned by `crates/sparq-core/tests/dict_spill_activation.rs`
  (sq-bif.13): a tiny `mem_budget` (constant cache eviction across many epochs + many spilled
  sort runs) yields a build byte-identical to a comfortable-budget build, a `disk_floor` above
  free disk aborts cleanly through the spill pipeline's resource gate, and a spill-built store
  reloads via `Graph::open` (and re-saves raw/compressed) to identical content. External build
  folds N-Quads/TriG named graphs into the default graph (only `load_dataset` preserves them).
- **HDT is opt-in and native-only.** `sparq-hdt` MSRV is **1.87** (the wrapped `hdt` crate),
  above the workspace's 1.85 — in the CLI it is gated behind `--features hdt`. It carries
  zero code into the wasm build. Compression containers are detected by **magic bytes, not
  file extension**, so a mislabeled `.hdt` still loads; all three (`gz`/`zst`/`bz2`) decode
  in a streaming fashion.
- **HDT triple-pattern loading is separately opt-in.** Enable `sparq-hdt`'s
  `load-filter` feature for `TriplePattern`, `load_reader_filtered`, and
  `stats_reader_filtered`. Both filtered operations share one resolved-pattern
  SPO walk. The stats call counts matches without building a result graph; its
  dictionary cardinalities remain archive-wide. The loader does not accumulate,
  intern, or index rejected triples, so result memory follows the matches.
- **HDT format coverage:** standard v1.0 layout only (FourSectionDictionary / Plain Front
  Coding + BitmapTriples, SPO order) as emitted by hdt-cpp / hdt-java. Exotic submission
  layouts are rejected with an error.
- **Writing HDT is opt-in** (`sparq-hdt`'s `write` feature): `save(&graph, path)` emits the
  same standard v1.0 layout. The path currently round-trips through a **temporary
  N-Triples file** via the wrapped crate's builder (`Hdt::read_nt` + `Hdt::write`, behind
  its `sophia` feature) — correct and interoperable, but it re-serialises + re-parses the
  whole graph, so it is NOT free. A direct in-memory builder (no text round-trip) is queued
  upstream (`sparq-hdt/UPSTREAM.md`). HDT carries a single default graph, so `save` ignores
  named graphs.
- **`load_dataset` is in-memory only.** Numeric/temporal filter caches are built on load
  in all in-memory paths; `into_compressed()` / `load_str_compressed()` trade a small
  per-scan decode for ~2.5× more triples per byte of RAM (browser target).
- **`block-bloom` (opt-in, OFF by default).** A block-compressed permutation already has an
  implicit min/max zone map (the directory's first-triple) that prunes RANGE scans, but for an
  EQUALITY-BOUND leading column (a point/prefix lookup on a constant subject/object) the zone map
  still leaves a candidate block that must be decoded just to discover it holds no matching row.
  The `block-bloom` feature builds a tiny per-block Bloom bitset over each block's distinct
  leading ids (built only for high-NDV columns past a density gate; pure-std, no new dep) and
  probes it in `range` before decoding a block. Zero false negatives by construction, so results
  are IDENTICAL to the feature-off path — only fewer blocks decode. The filter is in-RAM/build-time
  only: it is NEVER written to the on-disk `SPQCPRM1` format, so a perm built with the feature on
  persists byte-identically to one built with it off, and a memory-mapped perm carries no filter.
  **The measured skip opportunity is on ABSENT-key lookups.** On the SYNTHETIC high-NDV columns
  the sq-v8ixk harness generates (`measure_bloom_*` in `compress.rs`), the original "the id
  overlaps several blocks" rationale does not show up: because those columns are high-NDV, a
  PRESENT id's rows are contiguous and the zone map already narrows the candidate window to about
  one block, so there is little left for the filter to skip. The larger skip opportunity is an
  equality-bound id that is ABSENT from the permutation — a subject/object bound by an earlier
  pattern with no rows here — where the candidate block can be dropped undecoded. Those are
  block-SKIP COUNTS on a synthetic distribution: host-independent, but they establish no latency
  result and do not settle the question for real workloads. Run the harness for figures; none is
  asserted here, and the canonical run against a real dataset on the perf host is bead `sq-0g6g`
  (PENDING).
- **`elias-fano` (opt-in, OFF by default) — compressed-seek codecs, PROTOTYPE, not wired in.**
  `sparq-core = { …, features = ["elias-fano"] }` exposes `sparq_core::eliasfano` with
  `EliasFano` and `PartitionedEliasFano` (plus `EliasFanoIter` and
  `PartitionedEliasFano::DEFAULT_PARTITION`). Their `next_geq(target)` answers a successor query
  *directly on the compressed data* — no whole-block decode, unlike the varint block codec. Pure
  `std`, no new dependency.
  - **Input MUST be non-decreasing.** `encode` returns `None` on a non-monotone slice (EF is
    undefined there — that means the caller mis-segmented the column), and also `None` when the
    high-bits vector would exceed `u32::MAX` bits. An empty slice encodes fine. A permutation
    column is monotone only *within* one leading-prefix group, so you segment first.
  - **Nothing in the store calls this yet.** It is a measurement-gated spike: not a `PermData`
    variant, no join uses it, and its A/B against the incumbent codec
    (`eliasfano::tests::measure_ef_vs_block_varint`, `#[ignore]`d) is **native-only** — it times
    with `std::time::Instant`, so the `wasm32` half of the comparison is unresolved. Treat
    adoption as an open question; no performance number is asserted.
  ```rust
  use sparq_core::eliasfano::EliasFano;
  let ef = EliasFano::encode(&[3, 3, 9, 40, 41]).expect("non-decreasing");
  assert_eq!(ef.get(2), Some(9));
  assert_eq!(ef.next_geq(10), Some((3, 40))); // first (index, value) >= target
  assert_eq!(ef.next_geq(99), None);          // every element is smaller
  assert_eq!(ef.iter().collect::<Vec<_>>(), vec![3, 3, 9, 40, 41]);
  assert!(EliasFano::encode(&[5, 1]).is_none()); // not non-decreasing
  ```
- **SPQCPRM2 frame-of-reference col2 encoding (versioned on-disk format; V2 emission opt-in).**
  A second block-stream version, `SPQCPRM2`, alongside the shipped `SPQCPRM1`. Where `SPQCPRM1`
  writes a middle-column-reset col2 id ABSOLUTE, `SPQCPRM2` writes it as a zigzag signed delta from
  the block's first-row col2 (the frame origin the decoder recovers at block start), so clustered
  objects encode as short varints. A V2 block with no middle-column reset is byte-identical to its
  V1 block.
  - **The V2 READER ships with `mmap`.** `open`/`from_mmap` auto-detect the 8-byte file magic —
    `compress::FILE_MAGIC` (`SPQCPRM1`) or `compress::FILE_MAGIC_V2` (`SPQCPRM2`) — and pick the
    matching block reader; a `CompressedPerm` carries its `format` (`enum Format { V1, V2 }`). **An
    existing `SPQCPRM1` file on disk keeps decoding byte-identically forever** (the backward-compat
    soundness invariant). ANY OTHER 8-byte magic is a loud, clean `Err` — never a silent misdecode
    (mutation-witnessed in `mmap_corruption_oracle`).
  - **A build EMITS `SPQCPRM1` by DEFAULT.** V2 is NOT defaulted on (a separate decision pending a
    clearly-positive real-data B/triple measurement — the col2-clustered synthetic win did NOT hold
    on WatDiv). The `spqcprm2` cargo feature (which implies `mmap`) exposes the emit config gate:
    `compress::with_emit_format(EmitFormat::V2, || …)` on a thread, or `SPARQ_EMIT_FORMAT=v2` for a
    process, routes the store's `save_compressed`, the streaming `CompressedPermWriter`, AND the
    in-RAM compressed profile (`TripleStore::from_triples_compressed` / `Graph::into_compressed`,
    i.e. `SPARQ_STORE_PROFILE=compressed` — [FABLE-5] sq-559dp) through the V2 encoder. On the CLI
    that same choice is a per-invocation FLAG — `sparq-cli save …
    compressed --format-v2` / `sparq-cli recompress … --v2` ([SONNET-4.6] sq-kmve2) — which maps
    onto the per-thread override (so it beats the env var) and is a hard exit-2 error on a build
    without `spqcprm2`, never a silent V1 write. `CompressedPerm::encode_v2` builds a `V2` perm directly; `encode_emit` honours
    the gate in one place. With the feature OFF, `emit_format()` is a `const V1`, so the default
    build cannot emit V2 and every shipped index stays `SPQCPRM1` bit-for-bit.
  - **Public API (`sparq_core::compress`).** Always: `fn CompressedPerm::encode_emit` (a plain
    inlined `encode` when the gate is compiled out, so the `mmap`-off wasm in-RAM stream is
    byte-identical). With `mmap`: `enum Format`, `const FILE_MAGIC_V2`.
    With `spqcprm2`: `enum EmitFormat`, `fn with_emit_format`,
    `fn CompressedPerm::encode_v2`, `fn CompressedPermWriter::create_with`.
- **JSON-LD W3C conformance is RATCHETED (honest baseline, not 100%).** A ratcheted W3C
  JSON-LD 1.1 conformance gate (sq-oy1f.2 + sq-3uos5 + sq-oy1f.19 + sq-oy1f) drives the official
  `w3c/json-ld-api` suite AND the SEPARATE `w3c/json-ld-framing` suite through the real paths:
  **toRdf** through the `jsonld` parser (oxjsonld), **fromRdf** through the `serialize-rdf`
  writer, **compact** through the native Compaction Algorithm (`graph_to_jsonld_compact`),
  **expand** + **flatten** through the **native `sparq-jsonld` document pipeline**
  (`sparq_jsonld::expand()` sq-kk1mq / `sparq_jsonld::flatten()` sq-oy1f.26),
  and **frame** through the native Framing Algorithm (`graph_to_jsonld_framed`). toRdf/fromRdf/compact
  are compared by a re-parse RDF-dataset round-trip against the INPUT (`reparse(out) ≡ in`, the
  oxjsonld self-reparse oracle); **expand** + **flatten** are compared **document-to-document**
  against the suite's NORMATIVE expected output via the `json_ld_equal` comparator (key order
  insignificant; array order significant only inside `@list`), and **frame** by re-parse
  RDF-equivalence (`reparse(write(D)) ≡ reparse(expected)`) — these
  are JSON-LD normal forms / a SELECT+RESHAPE (they drop/merge/prune/fill), so the oracle anchors on
  the expected document, not the input. The floors only RISE; they reflect ACTUAL current pass
  counts, not full conformance — the
  remaining toRdf divergences are documented oxjsonld limits (remote/`@import` `@context` needs a
  `LoadDocumentCallback`; `expandContext`/`rdfDirection` options not applied; a few
  base-normalization edge cases + leniently-accepted negative tests), and fromRdf misses
  lists whose cells are shared across graphs. The **compact** lane gates on **lossless
  compaction** (parse input → RDF, compact against the case `@context`, require the
  re-parse to reconstruct the SAME RDF); the floor was raised by sq-oy1f.16 after the #978
  faithfulness fixes (the `@type:@id`-coerced-key-vs-plain-string IRI confusion and several
  container round-trips became lossless), with the rest still below the floor (scoped/typed
  contexts, `@nest`, `@index`/`@id` map shapes the writer does not emit) and several SKIPPED
  (negatives sparq's TOTAL compaction does not raise; JSON-LD-1.0-only; non-inline/remote
  `@context`; empty-RDF inputs). The **frame** lane (over the separate framing suite) moved to the
  **native `sparq-jsonld` document-level framer** (`sparq_jsonld::frame::frame()`, sq-oy1f.29 —
  frame matching over the expanded document + node map: value patterns over `@value` alternative
  arrays, `@explicit`/`@default` fill, named-graph `@graph` framing, `@list` re-emit, blank-node
  `@embed` with 1.1 pruning, `omitGraph`/`frameDefault`, and the framing error codes). It holds a
  FULL score at the pinned suite revision under the normative document oracle (`json_ld_equal`
  against the suite's expected document), the 3 NegativeEvaluationTests now RUN (pass iff
  `frame()` raises the manifest's exact `expectErrorCode`: `invalid frame`,
  `invalid @embed value`), and the old-oracle→new-oracle re-pin (61/92 RDF-answer-equivalence →
  92/92 document oracle) is documented side-by-side in the lib-side floor
  (`sparq-conformance/src/floors/frame.rs`, rise-only after). The engine's RDF-first
  `graph_to_jsonld_framed` writer above remains the engine emit path until the serialize-cutover
  bead (sq-oy1f.41). The compact oracle
  is oxjsonld self-reparse, so the `@reverse` double-inversion / non-string-language interop gap
  documented above (the compaction interop caveat) is NOT caught by it and is tracked separately.
  The **expand** + **flatten** lanes (sq-oy1f) GRADUATED out of the NOT-IMPLEMENTED bucket and
  BOTH now run the **native `sparq-jsonld` document oracle**: the input is passed to
  `sparq_jsonld::expand()` / `sparq_jsonld::flatten()` and the result is deep-compared to the
  normative expected document via `json_ld_equal`. The `flatten` floor was RE-PINNED 50→46
  (sq-oy1f.26) when it moved off the RDF-writer oracle — the drop is inherited native-`expand()`
  gaps (empty-property retention, a few `invalid typed value` cases; owned by sq-oy1f.37), NOT
  flatten-algorithm bugs, and it is rise-only after the re-pin. Below-floor cases are honest
  divergences; the SKIP buckets are the documented ones (negatives, JSON-LD-1.0-only, the single
  post-flatten-compaction case deferred to sq-oy1f.27). Only **html** (script extraction) +
  **remote-doc** (a `LoadDocumentCallback`)
  remain NOT-IMPLEMENTED buckets the runner reports separately and never fails on (they grow the
  ratchet as those land). The lane is the opt-in
  `jsonld-suite` feature on `sparq-conformance` (forwards to `sparq-core/jsonld` +
  `sparq-engine/serialize-rdf`); OFF it compiles to a self-skip. Reproduce with
  `scripts/fetch-jsonld-tests.sh` + `scripts/fetch-jsonld-framing-tests.sh` then
  `cargo test -p sparq-conformance --features jsonld-suite --test jsonld_suite` (self-skips if
  the gitignored suites are absent). It is registered in the central conformance scoreboard
  (`sparq-conformance-scoreboard`).

- **JSON-LD comparative benchmarking lives in `bench/jsonld/`** (sq-hmd7l.15): the W3C
  pass-rate table vs jsonld.js / titanium-json-ld's PUBLISHED results (pinned with provenance
  in `bench/jsonld/conformance-peers.json` — sparq's compact/frame lanes use a round-trip
  oracle, so those two rows are NOT 1:1 comparable) + an output-equality-gated
  expand/flatten/compact/toRdf throughput harness (`bench/jsonld/run.sh --smoke|--gather`).
  The document-level comparator + canonical-dataset helpers the ratchet AND the bench share
  live lib-side in `sparq_conformance::jsonld_bench` (behind `jsonld-suite`); the sparq runner
  is the `bench_jsonld` example on `sparq-conformance`. Gap record:
  `research/gap-jsonld-2026-07.md`.

## See also

- `fused-decompress-parse` — choosing gzip vs zstd vs bzip2 and fusing decode with parse (measured numbers).
- `rust-parallel-parsing` — how the chunk-parallel N-Triples/Turtle scanners work, and when NOT to parallelize.
- `hdt-format` — the HDT binary layout internals and the `hdt`-crate wrapping/decode performance.
