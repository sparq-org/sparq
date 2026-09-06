# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-08-31

First release: an experimental, from-scratch RDF triplestore and SPARQL engine in Rust
(dictionary-encoded, six sorted permutation indexes, parallel execution), published as the
`sparq-*` crate family — `sparq-core` / `sparq-engine` / `sparq-reason` / `sparq-cli` /
`sparq-server` plus the opt-in capability crates (see `docs/release.md` §4 for the full
publish set). The API is unstable; SERVICE federation remains unimplemented — see
`research/roadmap.md`.

The earlier `v0.1.0` tag was an incomplete CI tag with no final GitHub Release or registry
publication. It remains in place for auditability; v0.1.1 is the first complete release.

**Crates.io build caveat:** crates.io builds resolve upstream `spargebra` 0.4.6 — the vendored SPARQL-parser conformance fixes (`vendor/spargebra/SPARQ-PATCHES.md`) apply only to git builds until the upstream PRs land.

### Added

- **Storage (`sparq-core`)** — dictionary-encoded triple store with six sorted permutation
  indexes (Hexastore/RDF-3X lineage); a 3-permutation compact-index mode (~half the index
  memory) for memory-constrained targets; parallel index construction.
- **Out-of-core mode** — build the indexes on disk and query them memory-mapped: a 100M-triple
  external build at a few GB peak RAM, then fast open with ~0 committed heap (OS page cache
  only), at query speeds matching the in-memory engine (figures: `research/BENCHMARKS.md`).
- **Parsing** — N-Triples, Turtle, N-Quads, TriG (via `oxttl`); parallel and streaming, with
  transparent `.gz` / `.bz2` / `.zst` decompression. Real-Wikidata ingestion builds all six
  permutations out-of-core on a fanless laptop (rate: `research/wikidata-ingestion-benchmark.md`).
- **SPARQL query (`sparq-engine`)** — SELECT/ASK; Basic Graph Patterns, FILTER, OPTIONAL,
  UNION, MINUS, VALUES, BIND, aggregation + GROUP BY, ORDER BY, DISTINCT, LIMIT/OFFSET;
  sort-merge, hash, and worst-case-optimal joins with greedy cardinality-based planning;
  parallel scan / filter / sort / aggregation / serialization.
- **Inference (`sparq-reason`, opt-in)** — RDFS, OWL-RL, and a Notation3 subset with
  saturate-then-sweep single-pass materialization (a large speedup over a naive fixpoint;
  figures in `research/BENCHMARKS.md` / the crate README) and union-find `owl:sameAs`.
- **CLI (`sparq-cli`)** — load/query/bench subcommands, on-disk index build + `query-mmap`,
  `reason`, a per-subsystem parallel-scaling harness, and hardware-tiered release binaries
  (per-ISA prefetch tuning selected per silicon family).
- **HTTP server (`sparq-server`)** — W3C SPARQL 1.1 Protocol query endpoint (GET/POST) and
  Graph Store HTTP Protocol read side; JSON / XML / CSV / TSV results with content
  negotiation; Docker image (distroless, `ghcr.io/sparq-org/sparq-server`).
- **WebAssembly (`sparq-wasm`, unpublished)** — the core engine compiled for the browser with
  a minimal bundle (no threads, compact index); ships via npm later, not crates.io.

- **Python full-text search (`sparq-py` × `sparq-text`)** — the `TextIndex`
  lifecycle follow-up recorded in `crates/sparq-py/TODO.md` is implemented on the
  wrapper: `Graph.text_search(query, any=False, limit=None)` returns BM25-ranked
  `(Term, score)` hits over the default graph's string literals (AND of tokens by
  default, `any=True` for OR, `*`-suffix prefix tokens), and `Graph.query_text`
  runs SELECTs with the `text:` magic predicates (`text:matches` /
  `text:matchesAny` / `text:score`) through `sparq_text::query_text`. The index is
  built lazily on first use, cached on the `Graph`, and invalidated on every
  mutating swap (`update` / `reason` / `reason_n3_with` — the wrapper's mutation
  paths rebuild the graph, so lazy rebuild, not `apply_delta`, is the policy that
  matches); `build_text_index()` builds eagerly (returns the indexed-literal
  count) and `drop_text_index()` releases the cache. Opt-in stays consistent with
  native (full-text is engaged by depending on `sparq-text`; the new edge is on
  the leaf `sparq-py` crate only, so engine/CLI/wasm stay text-free; `sparq-text`
  itself is unchanged). 12 new pytest cases (38 → 50).
- **Concurrent-serving Wave A2 + A3 (`sparq-serve`)** — the single sequenced
  writer with group-commit over the A1 generation ring (`research/concurrent-serving.md`
  §6.5). One writer thread owns the sole right to publish generations; updates
  arriving within a group-commit window (default 3 ms / 256 updates) apply to one
  working copy and publish as ONE generation (one epoch tick), with per-update
  atomicity — a failed update is reported to its submitter and skipped, never
  poisoning its batch (re-fork-and-replay recovery for partial mutation). Opt-in
  `CommitGranularity::CommuteGroup` splits a window into maximal runs of
  provably-commuting pure-data updates via conservative graph-level footprint
  analysis (`Footprint::of_sparql`; same-named-graph opposite-polarity or
  unanalyzable = barrier) and publishes one conflict-free generation per run —
  committed result is byte-identical to serial (FIFO application either way).
  `GraphApplier` is the production `ApplyUpdates` (structural `Graph::fork` +
  delta-overlay apply + threshold compaction). **A3**: `Writer::submit`'s returned
  generation number doubles as a single-node read-your-writes `shard_seq` token;
  `GenerationRing::read_your_writes(token)` pins the current generation iff it
  already reflects that commit (`number >= token`), `None` otherwise (a lagging
  replica must wait/redirect). Readers never block — `current()` stays lock-free.
  Tested: group-commit + FIFO + failed-update isolation, commutativity-batch
  differential correctness (CommuteGroup == Window == serial, + 200-op fuzz),
  50k-triple bulk-update atomicity (a concurrent sampler sees 0-or-all, never a
  partial prefix; never stalls), retention/pinning under sustained writes, and
  read-your-writes token semantics. Benched in `bench/serve/writer_spike`
  (group-commit ~7.8× writer throughput at batch 16 vs 1; readers within ~1.3× of
  idle p99 under load; the too-large-batch regression recorded honestly).
  `sparq-serve` stays out of the `sparq-wasm` dependency graph (server-side only).
- **Concurrent-serving Wave B — read-side scheduler (`sparq-serve`)** — the
  cost-aware query `Scheduler` (`research/concurrent-serving.md` §6.2; goal #4
  requirements 3 + 4: cheap-query prioritisation + no head-of-line blocking). A
  bounded thread pool with two lanes split by a crude cost estimate (Wierman &
  Nuyens: SRPT tolerates crude size errors): **cheap** jobs are prioritised
  (FIFO) and always keep a reserved worker, while **heavy** jobs run under a
  bounded concurrency cap (default `cores/2`, clamped `<= workers-1` so a cheap
  worker is always free). The reserved-cheap-capacity invariant is the structural
  no-head-of-line guarantee — an expensive-query flood can never block a cheap
  query, because at least one worker is always available the instant a cheap job
  arrives. Within the heavy lane, ordering is Umbra-style SRPT-approx + unbounded
  aging (litreview-C: `p0 = 10^4` start, cost lowers priority, wait raises it), so
  the most-postponed heavy job eventually becomes the max-priority pick —
  starvation-free. Readers still pin a generation on entry *inside* the submitted
  closure (A1 snapshot consistency), so scheduling reorders *when/where* work runs
  but never *what* it computes: results are byte-identical to unscheduled
  execution. Library-first: sync, runtime-agnostic (`std::thread` + condvars, no
  HTTP/async types); the estimator and the work are both injected, so the
  scheduler never reaches into the engine itself. Tested (empirical-honesty
  suite): differential result-equality vs direct execution (closure + real-engine
  COUNT queries over the ring), bounded heavy concurrency, head-of-line cheap-p99
  containment under a concurrent expensive query (open-loop,
  coordinated-omission-safe arrival), starvation-freedom under a sustained cheap
  flood, SRPT-orders-cheaper-first + aging-overtakes-a-cheaper-fresh-heavy
  (deterministic), no-regression on an all-cheap workload vs a plain pool,
  panic-isolation, and shutdown shed-don't-hang. Honest scope: true preemption is
  out (the engine has no re-entrant suspend/resume); the cap + reserved capacity
  deliver the no-HoL property without it. Stays out of `sparq-wasm`'s graph.
- **Python bindings parity wave (`sparq-py`)** — the Python `Graph` catches up with
  the engine/reasoner surface: native `ask()` (engine early-exit instead of the
  SELECT-count rewrite; SELECT still accepted), new `construct()` / `describe()`
  (lists of `(s, p, o)` `Term` tuples; DESCRIBE is concise-bounded-description),
  named-graph parity (the stale "named graphs unsupported by update / dropped by
  reason" caveats removed: updates ride engine update v2, and `reason()` /
  `reason_n3_with()` now carry `Graph.named` across the rebuild), new
  `inconsistencies()` (OWL 2 RL clash report from `sparq_reason::inconsistencies`),
  and new `reason_n3_with(rules)` (caller-supplied N3 rules over an already-loaded
  graph, composed exactly like `MaterializedN3Graph`'s fallback: default graph
  rendered as N-Triples under the rules document, run through `reason_n3`; on error
  the graph is unchanged). 18 new pytest cases (20 → 38); README/TODO refreshed.
  Verified non-gap recorded honestly: full-text `text:` predicates do NOT work
  through plain `Graph.query` — `sparq-text` is a deliberately standalone opt-in
  crate, so exposing it needs a `TextIndex` lifecycle on the wrapper (left as a
  documented follow-up in `crates/sparq-py/TODO.md`, not wired in quietly).
- **JS/wasm parity wave (`@sparq-org/sparq` + `sparq-wasm` + `sparq-core`)** — six recorded
  parity gaps closed; wasm bundle 1,593,075 → 1,643,103 B (+50,028 B, +3.14%, all from
  three new wasm capabilities, measured per feature; JS-only features byte-identical):
  - **ASK from JS** (0 B): `queryBoolean()` now uses the engine's native ASK (boolean
    JSON form, first-solution early exit) instead of the SELECT-rewrite + count path;
    `queryJson()` serves ASK too. Plus a test pinning full-text-style matching via plain
    SPARQL string functions (and that `REGEX` stays compiled out of wasm by design —
    the engine's non-default `regex` cargo feature).
  - **Named graphs** (+16,906 B): wasm `Store.loadDataset` (N-Quads/TriG named graphs
    preserved); JS `options.dataset` on `fromString`/`fromQuads`; `GRAPH`/`FROM`/
    `FROM NAMED` queries and full-dataset SPARQL Update (already compiled in) now
    reachable; `match()`/`countQuads()` graph-aware (wildcard spans named graphs via a
    `UNION { GRAPH ?g … }`).
  - **Streaming results** (+4,575 B): wasm `Store.queryChunks` cursor over the engine's
    chunked SPARQL-JSON serialiser (~64 KiB, row-boundary splits, concatenation
    byte-identical to `query()`); JS `queryBindingsStream()` generator (`for…of` and
    `for await…of`) backed by a chunk-boundary-agnostic incremental row parser
    (`SparqlJsonRowsParser`), `queryJsonChunks()` for raw forwarding.
  - **Incremental updates from JS** (+28,547 B: `applyDelta` +14,895, `updateInPlace`
    +13,652): new core API `Graph::apply_delta_nquads` (N-Quads batch, deletes-first,
    routed per graph, named graphs auto-created, bnodes by label); wasm
    `Store.applyDelta` + `Store.updateInPlace` (engine `update_in_place`); JS
    `applyDelta`/`addQuads`/`removeQuads`, and `update()` now applies in place —
    O(batch), no index rebuild.
  - **Compressed ingest in the browser** (0 B, JS-only): `fromCompressed()` /
    `decompress()` — zstd via pure-JS `fzstd` (dynamically imported; decodes the
    multi-frame streams `CompressedSink` emits — re-verified), gzip via `node:zlib`
    (multi-member) / `DecompressionStream` (single-member, browser caveat documented
    per the D4 §3 matrix). `fzstd` is the package's single runtime dependency now.
  - **Dictionary-fetch protocol client** (0 B, JS-only): `SparqDictionaryClient`
    implements D4 §4 — `Sparq-Dictionary`/`Sparq-Dictionary-Current` negotiation,
    content-addressed (truncated-id, prefix-verified) dictionary caching with
    background warm-up from `GET /dictionary/{dict-id}`, pluggable dict-capable
    decoder hook (fzstd has no dictionary API), RFC 8878 frame-header dictID parsing.
  - JS test suite 16 → 42 (`node --test`), +1 native `sparq-core` unit test.

- **Opt-in inference proof trees (`sparq-reason`, non-default `explain` feature)** —
  `why(triple)` on `MaterializedGraph` / `MaterializedOwlGraph` / `MaterializedN3Graph`
  returns a `ProofTree` for any closure triple: which rule fired (W3C spec rule ids —
  `rdfs2..11`, `cax-sco`, `prp-trp`, `scm-*` — or documented engine rules `inv-dom`/
  `inv-rng`/`sym-dif`, `n3-rule-<i>`) from which premises, recursively to asserted facts.
  The shape is flat and ZK-witness-friendly (premises-before-conclusion node list, root
  last, shared sub-proofs deduplicated, deterministic, terms as self-contained strings —
  a planned ZKP module can consume it as a derivation witness); JSON + indented-text
  renderings; depth/size caps (`ExplainOpts`). Proofs are *a witness, not all witnesses*,
  and always reflect the CURRENT base: after a delta, retracted supports are never served
  (an alternative support is found, or the triple no longer explains).
  Design: derivations are *reconstructed* from the counting engines' closed TBox +
  deterministic emission functions (plus raw TBox edge maps and base reverse indexes kept
  under the feature) rather than stored per derivation. OWL fallback mode (recursive/
  equality features) is documented as unexplained; N3 `why()` re-runs the batch engine
  with proof recording per call. Id-level batch N3 proofs bridge via
  `explain::n3_proof_tree`. The feature is cfg'd out entirely by default: zero hot-path
  cost, zero wasm impact (sparq-reason is not in the wasm graph; bundle byte-identical).
  When ON: batch `materialize_*` is unchanged (untouched code); incremental handles pay a
  bounded constant for the base reverse indexes on initial build, and small deltas stay
  ms-scale and far faster than re-materialization (measured cost in the crate README); N3
  unchanged (no extra maintenance state). Design lever if that ever matters: build the
  indexes lazily on first `why()`.

- **Opt-in full-text search over literals (`sparq-text`, new crate)** — a small,
  owned BM25 inverted index over a graph's string literals (UAX #29 tokenizer +
  Unicode lowercasing via `unicode-segmentation`; deliberately no tantivy) where
  the dictionary term id IS the document id, so hits join back to triples through
  the ordinary permutation indexes. `TextIndex::build` scans the dict once
  (rayon-sharded under `parallel`); `apply_delta` mirrors `Graph::apply_delta`
  batches incrementally (pinned equal to a rebuild by a differential test).
  Query surface: AND (`search`), OR (`search_any`), `*`-suffix prefix tokens
  (autocomplete), BM25 scores — and, behind the default-on `engine` feature, the
  `text:` magic predicates (`http://sparq.dev/text#`): `?lit text:matches "q"` /
  `text:matchesAny` / `text:score ?s`, rewritten at the spargebra-algebra level
  into inline `VALUES` over the hits and executed through the engine's existing
  `PreparedQuery: From<spargebra::Query>` seam — zero engine changes, zero wasm
  impact (bundle byte-checked; the crate sits outside every default dependency
  graph, like `sparq-geo`/`sparq-vectors`). Build / index-size / query-latency figures
  on 1M synthetic literals: `crates/sparq-text/README.md` (run its `bench_text` example).
- **Opt-in time-travel queries (`sparq-serve`, `sparq-server`)** — a retained
  generation IS a queryable snapshot. Library: `RingConfig::time_travel`
  (`TimeTravelConfig { max_generations, max_age }`, default `None`) extends ring
  retention beyond the concurrency bound K (K stays a floor — time travel extends,
  never shrinks); `GenerationRing::at(n)` pins a retained generation,
  `Generation::published_at` (stamped by the injectable `RingConfig::clock`) +
  `GenerationRing::as_of(t)` resolve "as of T" to a generation, honest `None` once
  history ages out. Server (non-default `time-travel` cargo feature):
  `?generation=N` on `/sparql` (URL or url-encoded body) serves the store as of
  that generation; every `/sparql` response carries a `Sparq-Generation` header
  (update 204s carry the generation containing the update — the read-your-writes
  token, the horizontal-scaling ADR's shard_seq concept); aged-out → `410 Gone`,
  never-published/unparsable/pinned-update → `400`; retention via
  `--time-travel-generations` [16] / `--time-travel-max-age` [off]. Feature off:
  the parameter handling is compiled out (tests cover both compilations). Memory
  cost recorded honestly: each retained generation is a FULL `Graph` until the
  structural-fork follow-up lands (delta-chain/OSTRICH-style retention is the
  named follow-up; the API needs no change for it). Wasm bundle unchanged.
- **GeoSPARQL function completion (`sparq-geo`)** — the `geof:` registry grows from
  12 to 35 functions: `geof:getSRID` (`xsd:anyURI`), the generic DE-9IM
  `geof:relate`, the full Egenhofer (`geof:eh*`, 8) and RCC8 (`geof:rcc8*`, 8)
  relation families (GeoSPARQL 1.0 Req 25/26 matrix patterns), the set operations
  `geof:intersection`/`union`/`difference`/`symDifference` (polygonal operands via
  geo's `BooleanOps`; other operand types are a clear per-row expression error),
  and `geof:buffer` (unlocked by the geo 0.30→0.33 bump; metric radii buffer in a
  local equirectangular metre frame). All exercised through real SPARQL.
- **GeoIndex: antimeridian, named graphs, incremental updates (`sparq-geo`)** —
  ball queries crossing ±180° split into two longitude windows (merged, deduped;
  brute-force-verified near the seam); `GeoIndex::build` scans every named graph
  and entries record their origin graph + `geo:asWKT` node; new
  `GeoIndex::apply_delta` mirrors a `Graph::apply_delta` batch into the R-tree
  incrementally (rstar insert/remove, O(batch·log n) — no rebuild), including
  `geo:hasGeometry` ownership re-keying.
- **CRS reprojection (`sparq-geo`, opt-in `reproject` feature)** — pure-Rust
  proj4 (`proj4rs`; no C dependency) behind a curated EPSG table (27700 — verified
  against the Ordnance Survey worked example to ~1e-6°, 3857, 2154, 25832/25833,
  UTM 326xx/327xx): `reproject::to_crs84` brings projected literals into the
  geographic machinery (metric distance, GeoIndex).
- **HDT quality-of-life (`sparq-hdt`)** — GZipped containers (`.hdt.gz`) are
  detected by magic bytes (not file names) and decompressed on the fly in every
  entry point; new `header()`/`header_reader()` expose the HDT header (dataset
  metadata triples — VoID statistics, provenance) as a queryable sparq `Graph`.
- **CLI HDT ingestion (`sparq-cli`, opt-in `hdt` feature)** — `--features hdt`
  wires sparq-hdt into the loader dispatch: format argument `hdt` or a
  `.hdt`/`.hdt.gz` extension loads through `sparq_hdt::load`; without the feature
  the CLI exits with a rebuild hint. Off by default (MSRV 1.87 vs the CLI's 1.85
  floor + decode-stack weight; rationale in `crates/sparq-cli/Cargo.toml`).
- **Recorded engine/core API seams (audit of the opt-in crates' TODOs)** — the
  cross-crate seams the "modify no existing crate" waves recorded, now implemented:
  - `sparq-core`: `Graph::load_str_with_base` / `parse_to_triples_with_base`
    (base-IRI resolution for Turtle/TriG — removes sparq-shacl's oxttl+`from_parts`
    workaround); `Dict::iter()` (the official dense-1-based-id term iterator
    sparq-introspect leaned on by hand); `Graph::iter_ids` / `iter_ids_sorted(col)`
    (borrowing canonical-id triple iterator, overlay-merged, zero alloc per row —
    sparq-shacl's term scans, sparq-sim's distinct subject/object enumeration);
    regression test pinning `Graph::save` on a SPARSE numeric cache (the panic
    sparq-py recorded was already fixed by `dense_numerics`; now pinned).
  - `sparq-engine`: **`PreparedQuery`** — parse-once / execute-many entry points
    (`query`/`ask`/`count`/`query_json`/`construct`/`describe`
    `*_prepared(_with_budget)` twins; the string APIs are now thin wrappers), the
    seam sparq-rsp recorded for per-window evaluation without re-parsing; and
    `#[derive(Debug)]` on `QueryResult` (workspace TODO, recorded by sparq-nlq).
  - `sparq-engine` **`cs-planner` feature (opt-in, non-default)** + `sparq-introspect`
    `characteristic_set_ids()`: characteristic-set star-join cardinality estimation
    (Neumann & Moerkotte) injected via `cs::CsTable` / `with_cs_table`; the greedy
    planner scores star candidates by the conditional CS expansion and uses
    `Σ_{C⊇Q} count(C)` as the subject-variable ndv (join order only — results
    identical; zero code in the wasm/default builds). Gate (q-error through the real
    planner recurrence vs true cardinalities, synthetic correlated stars): PredStat
    median 1.83 / gmean 7.02 / max 1283 vs **CS 1.00 / 1.00 / 1.00**.

- **Incremental inference maintenance (`sparq-reason`)** — counting-based incremental
  closure maintenance under base inserts AND deletes (deletion costs the same as
  insertion: exact derivation counts, no overdelete/rederive pass), extending T18's
  RDFS `MaterializedGraph` to two new opt-in, zero-wasm-impact siblings:
  - `MaterializedOwlGraph` (OWL 2 RL): the monotone assertional rules (prp-spo1,
    cax-sco, prp-dom/rng, prp-inv1/2, prp-symp, prp-eqp1/2, cax-eqc1/2) maintained via
    a property-orientation closure mirroring the batch `PropExpand`; `prp-trp` via a
    dedicated exact transitive layer (per-property effective-edge multisets, closure
    recomputed on touch — cost proportional to the transitive subgraph — and diffed
    into the counts). TBox mutations / scm-* and the recursive/equality features
    (sameAs, Functional/InverseFunctional, chains, restrictions, cardinality, hasKey,
    oneOf, intersection/union) take documented re-materialization fallbacks
    (`OwlMode` telemetry + `full_rebuilds()`).
  - `MaterializedN3Graph` (Notation3): a counting fast path for rule sets that
    ANALYSIS proves monotone with input-stratified negation — ground IRI predicates,
    a verified-parity builtin whitelist (log:uri, log:equalTo/notEqualTo,
    string:concatenation/scrape/encodeForUri), `?UNSCOPED log:notIncludes` only over
    predicates no rule derives (guard deltas rebuild), recursion via recursive-SCC
    layers; everything else falls back to the batch engine per mutation. Fallback
    cases include: out-of-whitelist data reached at evaluation (sticky); base
    `log:implies`-family triples (rules-as-data); and rules whose premise has NO plain
    join atom (an empty `{}` premise or a builtin/guard-only premise), since counting
    seeds emissions only from plain premise atoms and would otherwise drop their
    conclusions ([OPUS-4.8] reviews 1868/1884). The sparq-solid WAC rules and ACP
    strata a/b qualify; acp-c does not (variable conclusion predicate) — asserted by test.
  Differential property tests hold every counting-*maintained* profile equal to its
  from-scratch batch closure after every randomized edit batch (RDFS/OWL/N3);
  rule shapes outside the maintained profile (above) take the always-correct batch
  fallback rather than being maintained. Benchmarks
  (`bench/inference/incremental-bench.md`, olympics scale + a 1k-doc WAC pod):
  1-triple deltas maintain in microseconds vs ~1s re-materialization (~10⁵–10⁶x);
  10k-triple deltas 16–500x; live WAC ACL edits 11–162 ms vs the 0.84 s engine re-run,
  with the incremental initial build itself beating the batch engine (anchored
  premise evaluation). Conformance and the batch paths are untouched.

- **Severity-aware conformance + conformance memo (`sparq-shacl`)** —
  `ValidationReport::conforms_violations_only()` (CI-style gating: warnings and
  infos report but don't break conformance) and `results_with_severity(iri)`;
  the spec's `sh:conforms` is untouched and the API stays infallible. Inside
  the validator, `conforms()` now memoises `(focus, shape) → bool` with a
  cycle-soundness rule that leaves the recursion guard unchanged (a result is
  cached only when no guard re-entry escaped below its frame). W3C core suite
  unchanged at 98/98; pinned by new cyclic-shape and severity tests.

- **Neighbor-sparse fallback for `most_similar` (`sparq-sim`)** — entities whose
  signature elements all point at degree-1 neighbors (every event names exactly
  one sport) generated NO candidates in v1. `SimConfig::profile_fallback`
  (default on) fills starved slots with predicate-profile (role) matches, ranked
  below every exactly-scored result; predicate blocks are scanned
  most-selective-first with a `max(4k, 64)` stop. Re-measured on the real
  olympics eval: Sport precision@10 0.000 (0/0) → 1.000 (400/400), City 0.500
  (2/4) → 0.995 (398/400), full 2400/2400 candidate coverage, ~0.3 ms mean
  latency cost. The per-element IDF hub budget was investigated and is
  measured-and-rejected (the eval is saturated).

- **Streaming store builds, weighted RRF, bytes-backed open (`sparq-vectors`)**
  — `StreamingWriter` removes the build-phase RAM ceiling (vectors append
  straight to the `.spqv` data section, the id→slot index spills to a sidecar;
  output byte-identical to the in-RAM builder — no format change; duplicates
  reported at finalize), `fuse_rrf_weighted` (Elasticsearch-style per-list
  weights, unit weights ≡ `fuse_rrf`), and `VectorStore::open_from_bytes`
  (filesystem-less environments; validation identical to `open`, all read paths
  shared). No wasm feature wired (deliberate — the crate stays out of the wasm
  graph); DiskANN-style persistent ANN, quantization and big-endian remain
  deferred with rationale in `sparq-vectors/TODO.md`.

- **Overlay window evaluation, t0, CONSTRUCT/ASK (`sparq-rsp`)** — closed
  windows are no longer rebuilt from scratch: the new `EvalMode` selects the
  R2R materialisation strategy — `PersistentDict` (one dictionary per
  continuous query, terms interned once at push time, per-window graphs from
  already-interned ids; the new default — wins every benchmark scenario,
  1.2–5.3× over the v1 rebuild, biggest on sliding windows), `Delta` (one
  live graph + set-semantic `apply_delta` per slide with churn-driven
  compaction; measured slower everywhere, kept opt-in), and `Rebuild` (the v1
  baseline, still right for unbounded-vocabulary streams). Plus: RSP-QL
  window origin `t0` (`WindowSpec::with_t0`), and continuous CONSTRUCT
  (`ContinuousConstruct` — stream-to-stream transformation with exact
  set-diff ISTREAM/DSTREAM) and ASK (`ContinuousAsk`) query forms. All three
  modes are observationally identical (pinned by tests); README throughput
  table re-measured per mode. Zero wasm-bundle impact (opt-in crate).

### Changed

- [OPUS-5] **N3 incremental maintenance: the base↔layer ownership transfer no longer
  re-materializes (`sparq-reason`, `sq-6tykl.6`)** — a fact that is both asserted and
  derivable by a recursive-SCC layer is charged to the base while asserted, so mutating its
  base copy hands ownership between the base and the layer without changing the closure. The
  sign-homogeneous delta round could not express that hand-off and recovered with a full
  (non-sticky) re-materialization. The affected layer's own local fixpoint — recomputed in
  that round — now decides the hand-off directly: the layer step runs before the counted-rule
  step, a fact the layer re-derives is put straight back and dropped from the round's delta,
  and no derivation count is disturbed. The hand-off is settled whenever the affected layer is
  recomputed, which makes it lazy in the assert direction: retracting the base copy enters the
  round and normalizes ownership immediately, whereas asserting a fact the closure already
  holds contributes nothing to the round, so the layer keeps its copy alongside the new base
  one until a later delta recomputes that layer. The interim double ownership is
  observationally inert — while the base copy exists the layer's entry can never suppress a
  seed, and nothing consults it without the base guard. `full_rebuilds()` no longer
  increments for these deltas. Behaviour-neutral for the closure (the from-scratch
  differential oracle in `tests/incremental_n3_prop.rs` is unchanged and green); the TBox and
  guard-predicate full-rebuild fallbacks are untouched and remain documented.

- [SONNET-4.6] N3 rule-existential blank labels now use a source-fresh numbered
  namespace, so closure output may mint labels such as `_:__sk0_1_e` instead of
  the previous `_:__sk1_e` shape.

- [GPT-5.6] **Breaking (`@sparq-org/sparq`)** — `SparqStore.queryBindings(sparql, context?)` now
  returns `Promise<ResultStream<Bindings>>` instead of `Bindings[]`; await it and consume
  `data` / `end` / `error` events (or use `query()` for synchronous materialisation). Unsupported
  `context.sources` overrides, including an empty array, now reject instead of being ignored.

- **Pipelined streaming N-Triples ingest (`sparq-core`)** — `Graph::load_reader_parallel`
  now fills its 32 MiB block from repeated short `read()`s and overlaps decompression
  with parallel parse (producer thread → bounded channel → rayon parse → dict merge),
  matching the `build_external_ntriples_parallel` pipeline. Streaming ingest of a gzip/zstd
  N-Triples file is now far faster than the pre-fix per-`read()` flush and matches or beats
  decompress-then-parse (figures: `research/custom-parsers-baseline.md`).
  `load_reader_parallel` requires `R: Read + Send`.

- **Inference fixpoint linearization + delta-driven evaluation (`sparq-reason`,
  fixpoint-opt thread)** — the chain-transitivity derivation storms are gone, and
  closure-only callers stop paying for proof bookkeeping:
  * OWL-RL `prp-trp` is evaluated as the LINEAR rule `R(x,y), GEN(y,z) ⊢ R(x,z)` over
    generator edges (edges not derived by prp-trp itself; `TC(GEN) = TC(R)`), with the
    per-round schema rebuild and `prp-fp`/`prp-ifp` moved into the delta sweep —
    owl-bench `owl-transitive` (2k-edge chain, ~2M-pair closure): **54 s → 0.25 s**.
  * The N3 forward chainer detects transitivity-shaped rules
    (`{?x P ?y. ?y P ?z} => {?x P ?z}`) and runs the same linearization, bypassing the
    generic binding machinery — anc500 (500-link `:ancestor` chain): **52 s → 0.18 s**
    engine-internal (closure byte-identical, 125,250).
  * `run_closure` takes a `StepMode`: premise materialization and proof-step interning
    are skipped when the caller discards them (`reason_n3`, the CLI closure path;
    conformance keeps conclusions only) — grid30 closure 1.94 s → 1.02 s (−47%),
    dt100k −18% (engine-internal min-of-5, identical closures).
  Inference conformance unchanged (1637 pass / 0 fail / 17 documented divergences);
  wasm artifact size unchanged; refreshed EYE head-to-head in
  `bench/inference/eye-comparison.md`.

- **`sparq-geo` depends on geo 0.33** (was 0.30; clean API compatibility): brings
  the `Buffer` trait that makes `geof:buffer` implementable.
- **`GeoIndex::entries()` returns an iterator** (was a slice): entry slots are
  tombstoned/reused by incremental deletes.

- **OWL 2 RL rule completion (`sparq-reason`)** — the remaining OWL 2 RL/RDF rules
  (Profiles §4.3): `cls-oo` (oneOf member typing), `cls-maxqc1/2`
  (qualified-cardinality-0 clashes in `inconsistencies()`), the schema-level
  restriction-subsumption family `scm-hv`/`scm-svf1`/`scm-svf2`/`scm-avf1`/`scm-avf2`
  (indexed/guarded joins grouped by onProperty resp. filler — never the naive
  quadratic restriction×restriction pass), and the premise-free
  `cls-thing`/`cls-nothing1` axioms (occurrence-guarded). The rule-coverage table in
  `research/inference-completeness-audit.md` is now fully implemented or explicitly
  by-design (prp-ap, eq-ref, dt-*, reflexive scm-* — each argued). Conformance scores
  and closure throughput unchanged; zero wasm-bundle impact.

- **EXPLAIN / EXPLAIN ANALYZE (`sparq-engine`, `sparq-server`, T22)** — query-plan
  introspection: engine `explain()` renders the algebra tree plus a planning-only dry
  run of the BGP planner (greedy GOO join order with cardinality estimates, per-step
  join strategy — merge/hash/bind/worst-case-optimal — and pushed-down filters) by
  replaying the executor's own planning helpers; `explain_analyze()` (SELECT/ASK)
  executes under a thread-local per-operator trace reporting output rows + wall time
  per operator (zero-cost when off: one flag read per operator entry). Served on
  `/sparql` via `?explain=` / `explain=analyze` or `Accept: text/x-sparq-explain`.
- **Prometheus metrics (`sparq-server`, T22)** — hand-rolled text exposition at
  `GET /metrics` (no new dependencies): request counts by endpoint/status, a `/sparql`
  latency histogram, active-subscription and graph-triple gauges, an update counter.
- **SPARQL CONSTRUCT / DESCRIBE (`sparq-engine`, `sparq-server`)** — RDF-graph query
  results: spec-conformant CONSTRUCT template instantiation (unbound/illegal slots
  skipped, fresh blank nodes per solution, set-deduplicated) and DESCRIBE as concise
  bounded description; engine API `construct` / `describe` / `construct_ntriples`
  (+ `_with_budget` variants); served over HTTP with `application/n-triples` /
  `text/turtle` content negotiation under the existing query budget. W3C construct
  evaluation suites pass 9/9 run (1 skip: `FROM`).
- **Streamed SELECT results (`sparq-server`)** — JSON SELECT bodies are streamed from
  an ordered chunk sequence (engine `query_json_chunks_with_budget`; concatenation
  byte-identical, same `Content-Length`) instead of one giant string: peak server RSS
  for a 1M-row SELECT (202 MB body) drops from ~750 MB to ~405–570 MB.

### Fixed

- **Persisted per-predicate stats never loaded on `open` (`sparq-core`)** —
  `load_pred_stats` read the predicate id as 8 bytes while `save_pred_stats` writes a
  4-byte `Id`, mis-framing every record: the load always failed and `open()` silently
  fell back to recomputing the stats, which paged the ENTIRE POS+PSO permutations into
  RAM (the dominant out-of-core open cost the persisted file exists to avoid). Measured
  on the 10M-triple synthetic dir: open 0.52 s → 0.019 s, RSS after open 236 MB →
  2.3 MB (research/memory-tiering.md). Regression-tested
  (`pred_stats_load_is_some_and_exact`).
- **N3 incremental maintenance: base↔layer ownership transfer (`sparq-reason`)** —
  deleting a fact that is both asserted and derivable by a recursive-SCC layer made the
  fact re-enter the layer's derived set during a delete round (it stops seeding the local
  fixpoint), tripping the sign-homogeneity `debug_assert` (panic in debug builds, a
  needless *sticky* engine fallback in release). Now recovered with a non-sticky full
  re-materialization; the graph stays on the counting fast path. Found by the `explain`
  retraction tests; regression-tested against the from-scratch oracle.

- **Domain/range typing for TBox-orphan properties on the monotone OWL route
  (`sparq-reason`)** — on the batch single-pass path (`rdfs_closure` with the
  property-orientation closure active, i.e. any `owl:inverseOf`/`owl:SymmetricProperty`
  axiom present), a property with an `rdfs:domain`/`rdfs:range` declaration but no
  subPropertyOf/inverseOf/symmetric/equivalentProperty edge emitted NO rdfs2/rdfs3
  typing for its assertions (absent from the orientation map, the emission
  short-circuited) — diverging from the full fixpoint path. Such properties now fall
  through to the plain-RDFS emission (their expansion is the trivial identity).
  Regression-tested; conformance unchanged (1637/0/17 — no suite test hits the
  combination, which is how it survived).

### Performance

Measured against native QLever on the same machine, compute-only, cold, min-of-N. The
methodology, the full per-query tables, and the speedup ranges are single-sourced in
`bench/qlever-baselines.md` (run `bench/qlever-olympics/` / `bench/qlever-synthetic/` to
reproduce):

- Synthetic 10M and 100M join/OPTIONAL queries: faster than QLever, in-memory and
  out-of-core (mmap) alike; the advantage holds from 10M to 100M.
- Real skewed data (DBpedia/Wallscope olympics, 1.78M triples): faster on **every** query.
- Honest caveats: QLever's on-disk index is *compressed* (smaller disk footprint at
  billion-scale); billion-scale real data (Wikidata/WDBench) is untested on this hardware;
  single-pattern/FILTER comparisons short-circuit via index range-counting and are excluded
  from the claims above.

[Unreleased]: https://github.com/sparq-org/sparq/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/sparq-org/sparq/releases/tag/v0.1.1
