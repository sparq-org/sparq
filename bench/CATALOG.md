<!-- [OPUS-4.8] Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns). -->
# sparq benchmark catalog

**What benchmarks exist + how to run them.** The machine-readable registry is
[`benchmarks.toml`](./benchmarks.toml) (one `[[benchmark]]` per entry, with the
exact invocation, dataset, and pinning). This file is the human guide:
conventions first, then a per-category map that points at the registry, then a
"replicate everything" quickstart.

> Companion: [`research/BENCHMARKS.md`](../research/BENCHMARKS.md) is **what we
> measured** (results, findings, honesty notes). This catalog is **what exists +
> how to run it**. Per-area numbers also live next to each harness (e.g.
> `bench/zk/README.md`, `bench/qlever-baselines.md`, `bench/inference/eye-comparison.md`).

## Conventions (the methodology, codified)

- **Measure first.** Probes like `probe-compress`, `compare-compress`,
  `bench-remap`, and the `bench/parse` baseline exist to *gate a design on
  numbers before building it*. No optimisation lands without a before/after.
- **Differential correctness is a gate, not an afterthought.** Every query
  result is cross-checked against an independent implementation:
  - **Oxigraph** (embedded Rust crate) — `sparq-bench` compare/fuzz/diff, the
    selective harness, the continuous fuzzer.
  - **QLever** (external, Docker or native) — the `bench/qlever-*` harnesses
    compare COUNT *values* and result *sizes*, not just "1 row == 1 row".
  - **oxttl / EYE / nargo** — parser, N3 reasoner, and ZK circuit semantics are
    each pinned against a reference (oxttl for parsing, EYE for N3 closures,
    `nargo`/`bb` for circuits). A harness fails hard on disagreement.
- **QUIET-BOX requirement for wall-clock numbers.** Any entry with
  `quiet_box_sensitive = true` (throughput / latency) must run on an *otherwise
  idle* machine — this box is frequently busy, so do NOT trust absolute
  wall-clock numbers taken under load. Where a contended run is unavoidable,
  prefer the **engine-internal timer** (the CLI's own `in Xs` line, or
  criterion's in-process timing) and report *ratios*, which survive contention
  (see `bench/inference/eye-comparison.md` for the worked example). Entries with
  `quiet_box_sensitive = false` (gate counts, B/triple, byte-identity, pass-rates)
  are deterministic and load-robust.
- **Min-of-N, cold vs warm.** Wall-clock harnesses report the **min of K**
  iterations. State the regime: `sparq-bench`/`cli bench` are **warm** (load
  once, query K times); QLever comparisons are **cold** (cache cleared each run,
  sparq has no query cache); `bench-mmap` open is **cold** first-touch. Keep the
  regime fixed when comparing.
- **Disk discipline.** `bench/*` data is **git-ignored and regenerable** — never
  commit datasets. Scratch goes in `/tmp` (the inference/owl scripts `mktemp -d`
  + trap-clean). Delete large datasets after a run; the wikidata-8b runbook caps
  dataset size and runs a `df` watchdog (abort < 50 GB free). Tracked scripts /
  `.gitignore` are never deleted.
- **Determinism / pinning.** Synthetic generators use fixed seeds (SplitMix64 in
  `sparq-bench`, `random.seed(7)` in selective, index-derived in u64). Pin the
  knob each entry names: `--scale`, fanout, thread list, w3c-rdf-tests commit,
  nargo/bb versions, QLever version + dataset version.
- **Opt-in features cost zero in the core.** `ci-bench` tracks
  `wasm_bundle_bytes` and `store_bytes_per_triple`/`dict_bytes_per_term` per
  commit precisely so a feature leaking into the browser bundle or growing memory
  shows up as a deterministic regression.

## Categories (point at the registry for exact commands)

| category | what it covers | registry ids |
|---|---|---|
| **query** | engine compute + differential correctness; aux-index harnesses; well-known suites | `sparq-bench-compare`, `sparq-bench-fuzz`, `sparq-bench-diff`, `cli-bench-suite`, `cli-bench-mmap`, `operator-coverage`, `sp2b`, `dbpsb`, `watdiv`, `bsbm`, `lubm`, `shacl-validate-bench`, `geo-bench`, `selective-bindjoin`, `u64-valueids`, `qlever-olympics`, `qlever-synthetic-10m`, `qlever-synthetic-100m`, `text-index-bench`, `vector-ann-bench`, `geo-index-bench`, `rsp-throughput`, `rsp-ql`, `vectors-throughput`, `kge-ablation`, `gpu-bench`, `sim-olympics-eval`, `introspect-olympics`, `federation-fedshop` |
| **parse** | text-format parse throughput (MB/s) | `parse-baseline`, `parse-competitors`, `serialize-bench` |
| **ingest** | load + dict + external-memory build throughput | `cli-ingest`, `cli-save-build`, `cli-bench-remap`, `dict-baseline`, `hdt-load-bench`, `hdt-stage-split`, `hdt-suite`, `wikidata-8b`, `tabular-import-smoke` |
| **compression** | index / result-serialization footprint tradeoffs | `cli-probe-compress`, `cli-compare-compress`, `compress-bench` |
| **scaling** | parallel thread sweep + cross-commit/hardware tracking | `cli-scaling`, `ci-bench`, `ci-bench-ec2`, `hw-bench`, `wasm-compare`, `graphalytics` |
| **inference** | N3 / RDFS / OWL closure + incremental maintenance; access-control (WAC/ACP/ODRL) oracle; trust-graph closure | `inference-eye-comparison`, `inference-owl-bench`, `inference-incremental`, `deep-taxonomy`, `owl-sameas`, `solid-wac-bench`, `policy-odrl-eval`, `ac-oracle`, `ac-odrl-overhead`, `trust-graph-closure`, `reason-el-real`, `reason-ql-npd`, `reason-dl-ore`, `materialize-competitors` |
| **zk** | commitment pipeline, trace seam, circuit gates, prove/verify | `zk-commit-throughput`, `zk-trace-overhead`, `zk-compose-gates`, `zk-compose-prove-verify` |
| **serve** | canonical loopback HTTP throughput harness; concurrent-serving + memory-tiering research spikes; PSS write-path parity gate | `serve-throughput`, `serve-spikes`, `memtier-spikes`, `pss-update-parity`, `gsp-bench`, `python-bindings-bench` |
| **conformance** | W3C SPARQL + reasoning suites (correctness, not perf) | `sparql-conformance`, `inference-conformance`, `jsonld-bench`, `canon-bench`, `rif-conformance` |
| **competitors** | versioned external-engine comparison (Oxigraph / QLever / Fuseki+TDB2 / eye + the SHACL/geo/FTS/vector peers) + version+env capture | `competitor-gather` (registry: [`competitors.json`](./competitors.json)) |

Notes on a few that need care:

- **`lws-core-readpath` (`bench/lws-core-readpath`)** wraps the in-crate read-response
  allocation example and emits validated, git-ignored JSON envelopes; see its
  [`README.md`](./lws-core-readpath/README.md). [GPT-5.6]

- **`bench/serve` + `bench/memtier` are research SPIKES, not maintained
  regression benchmarks.** Their numbers calibrate research docs; re-run on the
  target hardware before trusting any absolute value.
- **`serve-throughput` (`bench/serve-throughput`) IS a maintained canonical harness**
  (unlike the `bench/serve` spikes) — see
  [`bench/serve-throughput/README.md`](./serve-throughput/README.md). It stands up the
  REAL in-process `sparq_server::serve` stack on a loopback port and reports
  {req/s, p50/p99 ms, peak RSS} for a small SELECT + an ASK at concurrency 1/8/32. req/s
  + latency are wall-clock sensitive → **NON-CANONICAL on a shared box**; canonical
  numbers come only from a quiet EC2 runner, and it is deliberately NOT wired into
  `scripts/ci-bench.sh` / `scripts/perf-gate.py` (req/s is a trend/EC2 metric). It is the
  prerequisite that unblocks the HTTP opt lane (sq-7d3dj.10/.12/.13).
- **`gsp-bench` (`bench/gsp`) is the Graph Store Protocol same-box panel** — see
  [`bench/gsp/README.md`](./gsp/README.md). GSP PUT/GET/DELETE round-trips (1 KB–10 MB
  size sweep + the PSS LDP-CRUD stream projected onto GSP verbs) plus a **PATCH-dialect
  panel** (`application/sparql-update` + `text/n3` Solid N3-Patch; support probed per
  engine, 405/415/501 records honest n/a — Fuseki/Oxigraph GSP implement no PATCH
  dialect, sparq's `text/n3` is double-opt-in via the `n3-patch` feature +
  `GSP_N3_PATCH=1`) against sparq-server / Fuseki / Oxigraph server, plus Community
  Solid Server as a **LOOSE** LDP-architecture column (labelled, never averaged; the
  only Solid-PATCH peer). A HARD content-agreement gate (returned triple set must equal
  the sent/reference set exactly) runs BEFORE any timing; the driver is loopback-only
  (refuses non-loopback URLs). `bash bench/gsp/run.sh --smoke` is the self-contained
  acceptance run. First-read record: `research/gap-gsp-2026-07.md`. [FABLE-5]
- **`sp2b` (SP2Bench) is tiered** — see [`bench/sp2b/README.md`](./sp2b/README.md).
  The per-commit path builds+caches the real Freiburg generator (BSD; sha256-pinned, g++
  `-O2` not `-O3`) and runs 14 sub-second queries on a fixed 250k-triple corpus, emitting
  `sp2b_<query>_<mode>_us` (trend-only) plus a HARD expected-rows correctness diff. Three
  intentionally-pathological queries (q05a/q06/q12a, tens of seconds at 250k) sit in
  `queries-heavy/` for the EC2 tier; the **full 5M-100M scale** belongs to
  `bench-ec2.yml` (`bench/sp2b/gen.sh <triples>` at a larger `-t`) — which is
  **manual dispatch only**, see the CI-tracking bullet below.
- **`dbpsb` (DBPSB/FEASIBLE) is tiered + fetch-and-cache (real DBpedia)** — see
  [`bench/dbpsb/README.md`](./dbpsb/README.md). Per-commit, `fetch.sh` downloads ONE
  sha256-pinned DBpedia Databus slice (CC-BY-SA; `mappingbased-objects_lang=en` 2019.09.01,
  N-Triples despite the `.ttl` ext) and emits a DETERMINISTIC head cut of 750k triples, then
  runs 13 sub-second curated FEASIBLE/DBPSB queries emitting `dbpsb_<query>_<mode>_us`
  (trend-only) plus a HARD expected-rows correctness diff. Three unselective queries
  (`queries-heavy/`) and the **full ~11.8M artifact** (the `.bz2` ingested directly via the
  fused-decompress path) — up to DBpedia 'latest-core' ~1B triples — belong to the
  EC2/nightly tier.
- **`watdiv` (WatDiv) is tiered** — see [`bench/watdiv/README.md`](./watdiv/README.md). The
  per-commit path builds+caches the real Waterloo v0.6 generator (research-use; sha256-pinned,
  g++ + Boost, RNG seed-pinned to `1u`) and runs the 16 sub-ms Basic-Testing queries on a fixed
  SF=1 corpus (~106k triples), emitting `watdiv_sf<SF>_<query>_<mode>_us` (trend-only; the
  `_sf<SF>` scale-factor token keeps the per-commit `watdiv_sf1_…` and nightly `watdiv_sf1000_…`
  tiers as distinct series so the dashboard's scaling chart, sq-viby, can plot them on one axis)
  plus a HARD expected-rows correctness diff (count mode). Four templates empty at SF=1
  (F1/F4/C1/C2) sit in `queries-heavy/` for the **EC2/nightly SF≥10 tier**
  (`bench/watdiv/gen.sh <SF>`).
- **`bsbm` (Berlin SPARQL Benchmark, Explore mix) is tiered + fetch-and-cache** — see
  [`bench/bsbm/README.md`](./bsbm/README.md). Per-commit, `gen.sh` fetches the PREBUILT bsbmtools
  v0.2 distribution (JRE-only; sha256-pinned zip) and emits a deterministic `-fc -pc 300` corpus
  (~116k triples), then runs the 11 Explore queries emitting `bsbm_<query>_<mode>_us` (trend-only)
  plus a HARD expected-rows correctness diff against **MATERIALIZE** mode (the mix has a CONSTRUCT
  + a DESCRIBE — graph-valued forms report produced-triple counts). `query06` (unanchored regex,
  omitted from the official mix) + the **full ~100M+ scale** and the Explore-and-Update / Business
  Intelligence mixes belong to the EC2/nightly tier (`bench/bsbm/gen.sh <product_count>`).
- **`lubm` (Lehigh University Benchmark) is the REASONING suite** — see
  [`bench/lubm/README.md`](./lubm/README.md). `run.sh` is self-asserting: it builds the LUBM(1)
  corpus + Univ-Bench TBox (UBA generator, Apache-2.0 pinned commit, javac-only; `rapper` for
  RDF/XML→NT), materializes the **OWL-RL** closure (`sparq-cli reason … owl` — RDFS is incomplete
  here), then runs the **extensional** tier (Q1/Q2/Q3/Q14 on the raw ABox) + the **entailed** tier
  (Q4-Q13 on the closure; each returns 0 on raw data and its correct count only after reasoning),
  asserting BOTH tiers' counts vs `expected-rows.tsv`. CI emits `lubm_<query>_count_us`
  (trend-only) and fails on any count mismatch (reasoner OR engine regression). Full-scale
  **LUBM(1000)** (~133M triples) is the EC2/nightly tier (`bench/lubm/gen.sh 1000 0`).
- **`shacl-validate-bench` is the SHACL VALIDATION suite** — see
  [`bench/shacl/README.md`](./shacl/README.md). It REUSES the LUBM(1) ABox as its data substrate
  (so it shares the `javac`/`rapper` guard) × the **5 committed shape graphs** under
  `bench/shacl/shapes/` (`cardinality`, `datatype_range`, `class_nodekind`, `node_paths`,
  `sparql_constraint`). `run.sh` is self-asserting: it validates with the `bench_shacl` example and
  asserts each workload's **violations / conforms / focus_nodes** vs `expected.tsv` (deterministic
  at the pinned corpus + shapes; exit 1 on drift). The W3C core pass-count ratchet (98/98) lives in
  `crates/sparq-shacl/tests/w3c_core.rs` (`BASELINE_PASS`, only-tightens). CI emits
  `shacl_<workload>_validate_us` (trend-only, advisory). Heavy tiers: `univ=5`/`univ=10`. The
  cleanest competitor surface — Jena-SHACL / pySHACL / rdf-validate-shacl run the *identical*
  `(data, shapes)` pair (see the competitor map below + `scripts/bench-adapters/`).
- **`text-index-bench` is the FULL-TEXT-SEARCH suite** — see
  [`bench/fts/README.md`](./fts/README.md). It exercises `sparq-text` (BM25 inverted index +
  `text:` magic predicates) over a **synthetic** corpus generated **in-process** (no external
  generator — no `javac`/`rapper`): N seeded 8-word literals over a ~10k-term Zipf vocab. `run.sh`
  is self-asserting: it runs the `bench_text` example and asserts each workload's **total hit
  count** (`and_terms` / `or_terms` / `prefix4` / `phrase` / `near_slop2`, summed over a FIXED
  200-query set drawn from an independent seed) and the integer **index bytes-per-doc** vs
  `expected.tsv` (deterministic at the pinned `N=100000 seed=0` corpus; exit 1 on drift).
  `fts_bytes_per_doc` also has a `mode:auto` ratchet in `bench/perf-baseline.json`. CI emits
  `text_<workload>_us` + `text_build_s` (trend-only, advisory). Heavy/latency tier: `N=1000000`.
  An IR-quality BEIR axis (Recall@100 / nDCG@10) is gather-only and not yet wired (follow-up bead).
- **`rsp-ql` is the RSP-QL STREAMING suite** — see [`bench/rsp/README.md`](./rsp/README.md). It
  exercises `sparq-rsp` (windowed continuous SPARQL: S2R `RANGE/STEP` windows, three `EvalMode`s,
  RSP-QL multi-window joins). Because `sparq-rsp` is **clock-free** (a window closes on the
  pushed-timestamp watermark, not a real clock), a fixed `(triple, ts)` replay is a pure function —
  so the gate is a **DETERMINISTIC per-window result-row count** (a STRONGER gate than any
  wall-clock RSP benchmark). `run.sh` runs the `rsp_oracle` example (no external tool — the crate
  is isolated, like FTS's `bench_text`) and asserts every `rsp_<scenario>_<mode>_w<k>_rows`
  (single-window tumbling/sliding/GROUP-BY × Rebuild/PersistentDict/Delta — the identical expected
  counts ALSO encode the **three-EvalMode-equivalence**) and `rsp_srbench_<q>_w<k>_rows` (the
  **SRBench correctness ORACLE**: a multi-window observation⋈station-metadata join) vs
  `expected.tsv` (exit 1 on drift). CI emits `rsp_persistentdict_triples_per_s` (trend-only,
  advisory). **Bounded count-matched-replay RSP comparison** — per the program design
  record `research/comparative-benchmarking-everything.md` §5.2, a blanket
  NOT-COMPARABLE verdict is too strong; a bounded honest comparison is adopted:
  drive RSP4J with the *identical* timestamped replay, require **per-window
  result-count agreement** with sparq's deterministic oracle *first*, then report
  sustained triples/s side-by-side with a **machine-attached time-model caveat on
  every emitted row** (the caveat travels in the envelope, not just prose). Windows
  that cannot be count-matched are **excluded and the exclusion reported**. Implemented
  by `sq-hmd7l.20`; RSP4J/YASPER registered in `bench/competitors.json` (id: `rsp4j-yasper`).
  The **sustained-throughput axis is still NOT-MEASURED**: `sq-hmd7l.20` count-matched only the
  19-event oracle replay, too small for a rate claim. `sq-3f5ay` added the matched-workload
  harness — a sparq-side replay-FILE runner (the `replay_runner` example), a sha-pinned SCALED
  replay generated from `bench/rsp/replay/scaled.manifest.json`, and
  `SCALED=1 bench/rsp/gather-rsp4j.sh` driving BOTH engines from that one file — but the gather
  run that would publish the side-by-side has not been performed. No sustained `triples_per_s`
  comparison row is on the dashboard until it is; see `research/gap-rsp-2026-07.md`.
  Competitor honesty: **Solr/ES are NOT SPARQL competitors and stay off the dashboard**; the
  surface peer is Fuseki + `jena-text` (`http-sparql`), the kernel ref is Lucene/Anserini (labelled
  *sub-component, not an RDF benchmark*).
- **`hdt-suite` is the HDT LOAD-AND-DECODE suite** — see [`bench/hdt/README.md`](./hdt/README.md).
  It exercises `sparq-hdt` loading the **vendored real-world `snikmeta.hdt`** (a hdt-cpp/java-shaped
  FourSectDict+BitmapTriples archive, 328 triples) straight into a native sparq `Graph`. Like
  FTS/RSP the runner is a crate **example** (`bench_oracle` — `sparq-hdt` is isolated, not a
  `sparq-cli` dependency, and is behind the `hdt` cargo feature). `run.sh` is self-asserting: it
  gates the **load-and-decode counts** (`snikmeta_triples` / `snikmeta_terms`), **triple-pattern
  resolution** over the decoded graph (`snikmeta_distinct_predicates` / `snikmeta_rdf_type_triples`)
  and the **id-translation oracle** (`snikmeta_direct_eq_upstream` = direct decoder vs the
  upstream-backed `Hdt::read` path on the same bytes) vs `expected.tsv` (exit 1 on drift),
  complementing the differential + rejection oracles in `crates/sparq-hdt/tests/roundtrip.rs`. CI
  emits `hdt_load_s` (advisory wall-clock) + `hdt_vs_ntgz_load_s` (an advisory **ratio** — survives
  box contention; trend-only). **CRITICAL honest caveat:** load-and-decode-to-native is the **ONLY**
  like-for-like axis vs **hdt-cpp / hdt-java** — `sparq-hdt` decodes HDT into sparq's own
  `Dict`/`Graph` then queries its **own** indexes, whereas hdt-cpp/java query the compressed
  BitmapTriples **in place**; so a **query-over-HDT head-to-head is NOT like-for-like and is OUT OF
  SCOPE**. The `hdt-cpp` competitor (`bench/competitors.json`) is **decode-only**, gather-only via
  docker (zero recurring CI cost). The write-side **bytes-on-disk** gate is deferred until the
  in-memory PFC+BitmapTriples encoder (`sq-ashy`) is the production write path (encode-perf parity
  is a non-goal today).
- **`vector-ann-bench` is the VECTOR / ANN suite** — see
  [`bench/vector/README.md`](./vector/README.md). It exercises `sparq-vectors` (mmap'd `.spqv`
  vector store + HNSW / Vamana / PQ ANN) over a **synthetic** corpus generated **in-process** (no
  external generator): N seeded 32-d vectors. `run.sh` is self-asserting: it runs the
  `bench_vectors` example and gates each searcher's **recall@10 vs the `nearest_exact` brute-force
  ground truth**, emitted as a **DEFICIT** (`recall_deficit_milli = round((1-recall)*1000)`, design
  G4 — smaller-is-better, so it slots into the `mode:auto` ratchet with zero `perf-gate.py`
  change). `diskann`/`pq` deficits are **EXACT-gated** vs `expected.tsv` (single-threaded
  fixed-seed builds ⇒ byte-deterministic) and additionally `mode:auto`-ratcheted
  (`vectors_diskann_recall_at10` / `vectors_pq_recall_at10` in `bench/perf-baseline.json`); `hnsw`
  is **FLOOR-gated only** (its `instant-distance` build is rayon-parallel ⇒ ±1 deficit jitter, so
  exact equality would flake). CI emits `vectors_<searcher>_recall_at10` (deficit) +
  `vectors_<searcher>_query_us` + `vectors_build_s` (trend-only). Pinned `N=50000 seed=0` (the same
  50k×32 set the crate recall gate tests use). Competitor honesty: the **strongest** honest-comparison
  surface — the **ann-benchmarks** harness reports the **recall-QPS Pareto at MATCHED recall** (never
  a single latency); kernel peers hnswlib/FAISS/ScaNN/DiskANN-ref (`python-lib` +
  `scripts/bench-adapters/vector_lib_adapter.py` + exact-kNN oracle); Qdrant/Milvus/Weaviate
  loose-only. **No competitor does ANN-inside-SPARQL over dict-encoded ids** (uncontested surface).
  The big SIFT1M/GloVe recall-QPS Pareto is gather-tier (follow-up bead).
- **`geo-bench` is the GeoSPARQL VALIDATION suite** — see [`bench/geo/README.md`](./geo/README.md).
  A **fixed ~100k-point CRS84 corpus** (`bench/geo/gen.sh` → `bench_geo gen`; pure Rust, no
  `javac`/`rapper`/Docker) × within/nearest/`geof:` workloads (`within10km`, `within50km`,
  `nearest_k10`, `nearest_k100`, `geof_within`, `geo_compliance_pass`). `run.sh` is self-asserting:
  it asserts each workload's **result-set SIZE / compliance pass COUNT** vs `expected.tsv`
  (**COUNTS-NOT-COORDINATES** — float geometry is not bit-stable, so only sizes + pass/fail are
  gated; exit 1 on drift). The OGC compliance ratchet is the **hard-gated DEFICIT**
  `geo_compliance_deficit` (= 25 − passing, `mode:auto` in `bench/perf-baseline.json`, only-tightens;
  the related fixture ratchet lives in `crates/sparq-geo/tests/ogc_compliance_ratchet.rs`). CI emits
  `geo_<name>_us` (trend-only, advisory). Competitor: GeoSPARQL-Jena/Fuseki (full GML+WKT compliance
  bar, `http-sparql`); PostGIS as a loose non-SPARQL lower bound (see the competitor map below).
- **`jsonld-bench` is the JSON-LD PROCESSING suite** — see
  [`bench/jsonld/README.md`](./jsonld/README.md). Two axes: (a) a CONFORMANCE pass-rate table —
  sparq's MEASURED W3C JSON-LD 1.1 lane counts vs jsonld.js / titanium-json-ld's PUBLISHED
  results, pinned with provenance URL + date in `bench/jsonld/conformance-peers.json` (never
  estimated; denominators differ per suite snapshot and sparq's compact/frame lanes use a
  round-trip-RDF oracle — the caveats travel in the file); (b) expand/flatten/compact/toRdf
  THROUGHPUT on vendored WebDataCommons-shaped schema.org + context-heavy fixtures. The runner is
  a crate example (`bench_jsonld` in `sparq-conformance`, behind the opt-in `jsonld-suite`
  feature). **INVARIANT: no throughput row without output-equality agreement** — expand gated by
  the SAME `json_ld_equal` comparator the conformance ratchet trusts
  (`sparq_conformance::jsonld_bench`), flatten/toRdf by canonical-dataset equality, compact by
  round-trip losslessness; failed pairs are recorded exclusions. `run.sh --smoke` is
  self-asserting offline (deep-equality vs vendored jsonld.js-generated expectations +
  `expected.tsv` anchors + a NEGATIVE comparator self-test; exit 1 on drift). Peers are
  gather-only (`run.sh --gather` → `scripts/bench-adapters/jsonld_adapter.{mjs,py}`); compact
  rows compare DIFFERENT pipelines at the same task (sparq compacts RDF via the writer; peers run
  document-level Compaction) — the envelope carries the caveat on every row. Gap record:
  `research/gap-jsonld-2026-07.md`.
- **`deep-taxonomy` (DeepTaxonomy) is the rule-heavy N3 REASONING suite** — see
  [`bench/deep-taxonomy/README.md`](./deep-taxonomy/README.md). `run.sh` is self-asserting and
  REUSES the existing generator `bench/inference/gen_deeptaxonomy.py` (1 instance + a depth-deep
  `:sc` chain + 1 transitivity meta-rule): per depth tier it materializes the **N3 forward
  closure** (`sparq-cli reason … n3`), runs a class-membership `query.rq` over it, and asserts
  BOTH the closure triple-count (`= 2·depth+1`) AND the query rows (`= depth+1`) vs `expected.tsv`
  — a deterministic, load-robust gate that fails LOUDLY on a reasoner regression. Needs only
  `python3` (no g++/javac), so it runs on the per-commit tier at a SMALL depth pair (dt1k+dt10k);
  dt100k is opt-in via `DEEPTAX_DEPTHS` for EC2/nightly. CI emits
  `deeptax_d<DEPTH>_{closure_s,query_us,closure_triples}` (trend-only). The dashboard features it
  as a scaling suite (depth axis) with EYE external-reference baselines (cited from
  `bench/inference/eye-comparison.md`; dt100k = n/a, EYE not run).
- **`owl-sameas` is the OWL `sameAs` equality micro-suite** (the EQUALITY analogue of
  DeepTaxonomy) — see [`bench/owl-sameas/README.md`](./owl-sameas/README.md). `run.sh` is
  self-asserting: per tier `N` it builds `K=4` independent `owl:sameAs` equivalence classes of `N`
  members (a STAR of `N-1` edges) + `M=3` anchor data triples (a NEW pure-Python generator
  `gen_sameas.py`), materializes the **OWL-RL closure** (`sparq-cli reason … owl`), runs a
  class-membership `query.rq`, and asserts BOTH the closure triple-count (`= K·N·(N+M)`) AND the
  query rows (`= K·N`) vs `expected.tsv` — a deterministic, load-robust gate on the `owl:sameAs`
  union-find rewriting + expand-back path that NO other reasoning suite exercises (LUBM is
  subClassOf/restriction/Transitive/inverseOf; DeepTaxonomy is N3 subclass transitivity). The
  closure-size assertion is the load-bearing gate: it catches silent UNDER-derivation (a missing
  `N²` `sameAs` pair) that a correct membership query alone would miss. Needs only `python3` (no
  g++/javac/Docker), so it runs on the per-commit tier at a SMALL tier pair (N=8+N=32); N=256 is
  opt-in via `SAMEAS_TIERS` for EC2/nightly. CI emits
  `sameas_size<N>_{closure_s,query_us,closure_triples}` (trend-only). The dashboard features it as
  a scaling suite (size axis).
- **`wasm-compare` has a BROWSER half and a COMPETITOR half (both implemented).**
  [`bench/wasm-compare/browser/`](./wasm-compare/browser/README.md) (sq-3ul2n.1, the Tier-0
  measurement gate of the browser-WASM program `research/browser-wasm-perf-assessment-2026-07.md`)
  drives the SHIPPED `@sparq-org/sparq` bundle through headless Chromium/Firefox/WebKit (Playwright,
  self-contained npm dir — not a root workspace member) + a plain-Node baseline, attributing
  wall time PER PHASE (fetch/compile/instantiate + `instantiateStreaming`, N-Triples+Turtle
  load at 25k/100k/300k triples, five query shapes cold-vs-warm, CONSTRUCT serialization out,
  and the ask→count→string→parse→chunks→wrapper boundary-marshalling ladder). Advisory
  envelopes only (`results/`, git-ignored; the cross-engine row-count oracle is the sole hard
  check; browsers that cannot launch skip-with-notice); deliberately NO CI lane. The
  competitor half (sq-hmd7l.17, [`bench/wasm-compare/`](./wasm-compare/README.md)) CONSUMES
  this harness: `run.sh --bundle-only` is the DETERMINISTIC shipped-bundle-bytes comparison
  vs the pinned `oxigraph` npm artifact (the one canonical wasm-compare metric), and
  `browser/compare.mjs` layers the oxigraph-npm + N3.js/quadstore latency columns onto the
  same oracle-checked workload in Node + headless Chromium (gather-only installs; per-query
  row-count oracle + cross-library agreement gate every timing row). First-read gap record:
  `research/gap-wasm-2026-07.md`.
- **`wikidata-8b` is external-cost and gated.** It builds the full Wikidata
  truthy dump (~8-9.4B triples) on a 16 GB EC2 box (~$5-17). It is **blocked
  until dict-spill merges to public main** — see
  [`bench/wikidata-8b/RUNBOOK.md`](./wikidata-8b/RUNBOOK.md) §0 (hard launch gate)
  and `STATUS.md`. Do not launch without the budget + gate checks.
- **CI tracking**: `bench.yml` runs `ci-bench` on every push to main (free
  runners, trend + large-regression alert, no hard-fail); `bench-ec2.yml` runs
  the heavier version on spot. **`bench-ec2.yml` is MANUAL DISPATCH ONLY** — its
  crons were retired in [#3784](https://github.com/sparq-org/sparq/issues/3784)
  because the AWS OIDC role it assumes was descoped, so every scheduled tick
  failed at the credentials step *before any benchmark ran* and gated `main`. Its
  EC2 series is therefore collected only when a maintainer dispatches the workflow
  with `AWS_BENCH_ROLE_ARN` provisioned. Both push to the orphan
  **`benchmark-data`** branch via github-action-benchmark.
- **`competitor-gather` is the versioned external-engine comparison** — see the
  registry [`competitors.json`](./competitors.json) (Oxigraph embedded Rust dep /
  QLever Docker image / eye N3 binary: pinned version, install+run recipe, and the
  per-engine map of which sparq suites each is comparable on) and the orchestrator
  [`scripts/gather-competitors.sh`](../scripts/gather-competitors.sh). It is
  **safe-by-default**: a bare invocation DRY-RUNS (prints what it would do + a
  tool/version/env report, runs no benchmark, pulls no image). A real gather is
  guarded behind `--run --only <id>`; it caps the synthetic `--scale`, runs a `df`
  watchdog, cleans `/tmp` scratch, and writes a results file per engine recording
  the competitor's **version + host env** into git-ignored `bench/competitor-results/`.
  The script never edits the tracked JSON; a maintainer maps a reviewed result into
  the dashboard SEAM (`engines`/`values` in `competitors.json`) deliberately — and
  per the *No hard-coded performance numbers* rule, no figures are baked into git.
  Comparable-suite map (registry-driven): **Oxigraph** ↔ `sparq-bench-compare`,
  `sp2b`, `watdiv`, `bsbm`, `dbpsb`, `lubm` (extensional only); **QLever** ↔
  `qlever-olympics`, `qlever-synthetic-10m/100m`, `watdiv`, `bsbm`, `lubm`
  (extensional); **eye** ↔ `inference-eye-comparison` + `deep-taxonomy` (DeepTaxonomy/anc500/grid30);
  **Jena-SHACL / pySHACL / rdf-validate-shacl** ↔ `shacl-validate-bench` (identical data×shapes;
  cross-check `#violations`/`conforms` per-engine before trusting timing — `report-cli`/`js-lib`
  adapter kinds in `scripts/bench-adapters/`, gather-only on a Docker EC2 box);
  **GeoSPARQL-Jena / Fuseki-geosparql** ↔ `geo-bench` (the COMPLIANCE bar — the only triplestore
  with full GML+WKT, like-for-like since sparq-geo does both; `http-sparql` adapter, gather-only on
  a Docker EC2 box; cross-check result-set SIZE before timing), and **PostGIS** as a LOOSE non-SPARQL
  lower bound (relational `rstar`-style sub-component, NOT a `geof:`/graph-join competitor — match
  CRS/op semantics or omit).
  **sq-hmd7l.1 (2026-07-07) added 24 new competitor entries** (registry-only, no
  measured numbers, all `unverified_pin: true`): **serd / raptor / jena-riot** ↔
  `parse-competitors` + `serialize-bench`; **cwm / jen3** ↔ `inference-eye-comparison`
  (N3 reasoner columns 3+4); **vlog / nemo** ↔ `materialize-competitors` (Datalog
  peers); **elk** ↔ `reason-el-real`; **ontop** ↔ `reason-ql-npd`; **hermit-openllet**
  ↔ `reason-dl-ore`; **titanium-json-ld / jsonld-js** ↔ `jsonld-bench`; **rdf-canonize-js
  / rdf-canon-rust** ↔ `canon-bench`; **comunica** ↔ `federation-fedshop`; **rsp4j-yasper**
  ↔ `rsp-throughput` (bounded count-matched-replay per §5.2); **igraph / networkit** ↔
  `graphalytics`; **pyoxigraph / rdflib** ↔ `python-bindings-bench`; **oxigraph-wasm /
  n3js-quadstore** ↔ `wasm-compare`; **css-gsp** ↔ `gsp-bench` (GSP-LOOSE, reference
  kind, not a dashboard column); **pykeen** ↔ `kge-ablation` (quality oracle, not
  throughput). Versions to be pinned at first gather run.
  HONESTY NOTE (per parent `sq-i0nm`): a gh-runner gather is noisy and not
  comparable to the EC2/quiet-box reference band — the recorded `env.quiet_box`
  flag lets the dashboard label it distinctly (see the QUIET-BOX convention above).
- **Same-box SPARQL gather — the QLever indexed-server step** (sq-52fo). The
  same-box competitor gather [`scripts/gather-ec2-sparql.sh`](../scripts/gather-ec2-sparql.sh)
  compares sparq vs Oxigraph on ONE generated SP2Bench corpus (always on). QLever is
  **opt-in** behind `GATHER_QLEVER=1` because — unlike Oxigraph/EYE — it is NOT a
  file-in/answer-out CLI: it needs a dedicated **index build → running server → HTTP
  query → teardown** dance. That dance lives in [`scripts/qlever-same-box.sh`](../scripts/qlever-same-box.sh),
  which the gather invokes on the bench box. Recipe (all via the
  `docker.io/adfreiburg/qlever:latest` image):
  1. **index** — `IndexBuilderMain -i <base> -F ttl|nt -s <settings> < corpus` into a
     `mktemp -d` index dir (df-guarded, hard `QLEVER_INDEX_TIMEOUT`, default 20 min);
  2. **server** — `ServerMain -i <base> -p <port> -j <jobs>` detached with a fixed
     `--name`, then a **bounded** readiness poll (a counted for-loop, default 2 min,
     aborts early if the container exits) — this is the fix for the prior ~53-min hang;
  3. **query** — one bounded HTTP POST per (query, iter), min-of-K wall micros, emitting
     the harness's `<name>\t<rows>\t<best_us>` TSV (`ERROR` rows stay honest-n/a);
  4. **teardown** — an **EXIT trap that always runs** (`docker rm -f` the container +
     `rm -rf` the index dir) on success, failure, timeout, or Ctrl-C — no orphan server,
     no leaked disk. The gather also wraps the whole recipe in an outer `timeout 1800`.
  Run it: `GATHER_QLEVER=1 AWS_PROFILE=pss scripts/gather-ec2-sparql.sh <branch>`.
  With `GATHER_QLEVER=0` (the default) the QLever block is a complete no-op and the
  Oxigraph-only path is byte-for-byte unchanged. The recipe is **bench-only and
  documented-untested in the authoring worktree** (no Docker there) — validate on the
  next `GATHER_QLEVER=1` gather. (The separate `bench/qlever-*` suites use the upstream
  `qlever` Python CLI instead; this same-box recipe is the standalone Docker variant.)
- **Same-box SPARQL gather — diagnose-and-avoid-the-stall hardening of the sparq+Oxigraph
  half** (sq-sxso). The Oxigraph half of [`scripts/gather-ec2-sparql.sh`](../scripts/gather-ec2-sparql.sh)
  used to have NO per-step timeout and NO timestamped sentinel, so a hung step (a heavy
  from-source `cargo build`, or a pathological SP2Bench query in Oxigraph) burned the whole
  window with no `/root/GATHER_DONE` — the observed **"73 min, no sentinel"** stall, the same
  class the QLever recipe above already fixed. Three measured-rooted fixes (LOCALLY REPRODUCED:
  at only 50k triples the prebuilt Oxigraph CLI ran `q07` in ~38 s and `q08` past 60 s — so at
  the old 250k default those unselective SP2Bench joins are minutes-long in Oxigraph, which is
  the stall):
  1. **Per-step timestamped logging** — every phase calls `step "…"`, which UTC-stamps a line
     into `/var/log/gather.log` AND appends it to `/root/GATHER_STEP`. The orchestrator pulls
     that file (and surfaces the LAST step LIVE while polling, and on the no-sentinel path), so
     a stalled run names EXACTLY the hung phase instead of being invisible. The step log is also
     embedded in the result envelope (`step_log`).
  2. **Prebuilt, SHA-pinned Oxigraph CLI** (default) — `oxigraph load` into an on-disk store +
     `oxigraph query` per `.rq` (min-of-N, JSON-results count). This removes the from-source
     `cargo build -p sparq-bench` Oxigraph compile from the critical path (a likely build-time
     hang on the small fallback box). Pinned to **v0.5.9** by `sha256` (aarch64 + x86_64; the
     gather box is arm64). Set `OXI_EMBEDDED=1` for the in-process in-RAM embedded path instead
     — the envelope records `oxigraph_mode` (`prebuilt-cli` on-disk vs `embedded` in-RAM) so the
     load regime is never silently conflated.
  3. **Hard per-step `timeout` + smaller configurable dataset** — each phase is wrapped in a
     `timeout` (per-phase `STEP_*_TIMEOUT` caps) and the per-query Oxigraph loop is per-query
     timeout-bounded (`STEP_OXI_QUERY_TIMEOUT`, default 120 s), so a pathological query records
     `ERROR` (honest-n/a) and the gather moves on instead of hanging. The default corpus is
     smaller (`SP2B_TRIPLES` default **100000**, was 250000) for a cheap smoke/gather; raise to
     250000 for the full per-commit scale.
  **HONESTY:** this is the SCRIPT-side diagnostic + avoidance hardening, authored + locally
  reproduced on the aarch64 work box (the prebuilt-CLI load/query lane was run end to end). The
  actual confirmation that a real gather now reaches `GATHER_DONE` without stalling **requires
  one EC2 gather run** — that green-gather confirmation is a follow-up. Competitor numbers from
  this gather remain **non-canonical** (ephemeral EC2 / work box), per the QUIET-BOX note above.

## Replicate everything — quickstart

```sh
# --- build once ---
cargo build --release -p sparq-cli -p sparq-bench

# --- query: differential + perf vs Oxigraph (warm, min-of-K) ---
cargo run -p sparq-bench --release -- --scale 50000 --iters 4
# continuous correctness fuzz (deterministic, shardable by category):
cargo run -p sparq-bench --release -- fuzz 0 5000 all

# --- well-known query suites (self-contained runners: gen/fetch + run + hard row diff) ---
CORPUS=$(bench/sp2b/gen.sh 250000)   && ./target/release/sparq-cli bench "$CORPUS" turtle    bench/sp2b/queries 3 count
CUT=$(bench/dbpsb/fetch.sh 750000)   && ./target/release/sparq-cli bench "$CUT"    ntriples bench/dbpsb/queries 3 count
bench/watdiv/run.sh 1                 # WatDiv SF=1 (g++ + Boost): gen + count/materialize/json + row diff
bench/bsbm/run.sh                     # BSBM Explore -pc 300 (JRE + unzip): gen + materialize + row diff
bench/lubm/run.sh                     # LUBM(1) (javac + rapper): gen + OWL-RL closure + both tiers + row diff
bench/shacl/run.sh                    # SHACL (javac + rapper): LUBM ABox x 5 shapes + violations/conforms/focus_nodes diff
cargo build --release -p sparq-text --example bench_text && bench/fts/run.sh   # Full-text (no external tool): synthetic BM25 corpus + hit-count/bytes-per-doc diff
cargo build --release -p sparq-vectors --example bench_vectors --features approx-ann && bench/vector/run.sh   # Vector/ANN (no external tool; --features approx-ann required — example links HNSW VectorIndex): synthetic corpus + recall@10-deficit gate (HNSW/Vamana/PQ)
bench/geo/run.sh                      # GeoSPARQL (cargo only): fixed ~100k point corpus + within/nearest/geof: result-set-size + compliance-pass diff (counts-not-coords)
cargo build --release -p sparq-rsp --example rsp_oracle && bench/rsp/run.sh   # RSP-QL (cargo only): clock-free fixed (triple,ts) replay + DETERMINISTIC per-window row-count gate (3 EvalModes) + SRBench correctness oracle
bench/deep-taxonomy/run.sh            # DeepTaxonomy (python3 only): N3 closure per depth tier + closure-size + query-row gate
bench/owl-sameas/run.sh               # OWL sameAs (python3 only): OWL-RL closure per size tier + closure-size (K·N·(N+M)) + query-row (K·N) gate
bench/python/run.sh                   # Python bindings (pip pyoxigraph+rdflib + maturin sparq-py): SP2B tiny tier from Python + row-count agreement gate + binding-overhead floor/slope

# --- selective bind-join + u64 value-id probes ---
python3 bench/selective/gen.py 500000 > bench/selective/selective.nt
./target/release/sparq-cli bench bench/selective/selective.nt ntriples bench/selective/queries 3 count
python3 bench/u64-valueids/gen.py 1000000 /tmp/t3-literals.nt
./target/release/sparq-cli bench /tmp/t3-literals.nt ntriples bench/u64-valueids/queries 3 materialize

# --- ingest / compression probes ---
./target/release/sparq-cli ingest <truthy-slice.nt.zst> full
./target/release/sparq-cli save  <data.nt> ntriples /tmp/idx
./target/release/sparq-cli probe-compress /tmp/idx/spo.perm   # B/triple schemes
./target/release/sparq-cli bench-mmap /tmp/idx bench/qlever-synthetic/queries 5 count

# --- scaling sweep (OWN the box: idle, all cores) ---
./target/release/sparq-cli scaling <data.nt> ntriples bench/qlever-synthetic/queries 1,2,4,8 3

# --- inference ---
SPARQ_CLI=target/release/sparq-cli bench/inference/owl-bench.sh
EYE=$HOME/.local/bin/eye SPARQ_CLI=target/release/sparq-cli bench/inference/eye-comparison.sh
cargo run -p sparq-reason --example incremental_olympics_bench --release
# access-controlled-query oracle (WAC/ACP/ODRL, sparq-acbench) — fail-closed pass/fail, no engine link:
bench/ac/run.sh --smoke                              # per-commit smoke tier (bench/ac/run.sh --sf N = nightly)

# --- zk (standalone projects + Noir toolchain) ---
( cd bench/zk        && cargo bench )
( cd bench/zk-trace  && cargo bench )
bench/zk-compose/scripts/gate_counts.sh > bench/zk-compose/gate_counts_latest.json
bench/zk-compose/scripts/prove_verify.sh filter_int_d1

# --- conformance (correctness gates) ---
scripts/fetch-conformance.sh && cargo run -p sparq-conformance
scripts/fetch-inference-suites.sh && cargo run -p sparq-conformance --bin sparq-inference-conformance

# --- vs QLever (needs QLever installed; see each dir's README) ---
( cd bench/qlever-olympics && ../../.qlever-venv/bin/python compare.py 5 compute )

# --- versioned competitor comparison (Oxigraph / QLever / eye) — registry: bench/competitors.json ---
scripts/gather-competitors.sh                       # dry-run: tool+version+env report (runs nothing)
scripts/gather-competitors.sh --list                # show the pinned registry + comparable-suite map
scripts/gather-competitors.sh --run --only oxigraph --scale 50000 --iters 4   # real gather (records version+env)

# --- CI emitter locally / per-platform hardware sweep ---
bash scripts/ci-bench.sh 200000 /tmp/bench-results.json
./scripts/hw-bench.sh 500000 /tmp/hw-bench-results.csv
```

QLever-based comparisons reuse stored reference numbers in
[`bench/qlever-baselines.md`](./qlever-baselines.md) so QLever does **not** need
re-running for every sparq iteration — re-measure QLever only when its version or
the dataset changes (record date + commit).
