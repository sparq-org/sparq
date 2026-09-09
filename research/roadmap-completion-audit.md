# Roadmap completion audit (2026-06-12)

One-page evidence index for every thread of `research/roadmap.md` (committed at
fe03ed1): the merge commit(s) on `main`, where the implementation lives, and the
test/benchmark/doc evidence. Completion semantics: feature threads =
implemented + tested + documented; optimization threads = wins landed or
measured-and-rejected with recorded evidence.

Workspace state at audit: `ac905b0` — 506 workspace tests / 0 failures;
conformance **1229/1229** (1225 pass + 4 documented divergences, 0 skips,
CI-ratcheted); wasm cargo-artifact baseline 1,573,887 B (tracked); 19 crates.

## Original threads

| Thread | Status | Evidence (merge sha → artifact) |
|---|---|---|
| Many-core parallelism | **Landed** | morsel parallel exec, parallel parsing/serialization (pre-audit milestones; `research/parallelism-scaling.md`); parallel dict consolidation `d6f4033` — load scaling 1.98×→2.48×@4 / 2.99×@8, `research/dict-consolidation-verdict.md` |
| NUMA scaling | **Evidence-blocked (external)** | every EC2 rung fails: spot SLR missing + vCPU quota 16 — verbatim AWS errors in `research/hardware-validation-blocked.md`; run is one command (`hwrun/launch.sh`) once unblocked |
| Wikidata <24 min | **Partial-scale measured; full run quota-blocked** | 1 B real triples on r7i.2xlarge `c7af391` (rung5 figures single-sourced in `research/wikidata-ingestion-benchmark.md`); the discovered ~200 s/1 B serial dict bucket fixed in `d6f4033` (projected ≤153 s/1 B budget); full-scale validation needs the quota action above |
| u64 inline ValueIds | **Measured-and-rejected** | `96db20a` → `research/u64-valueids-verdict.md` (2× memory / 1.6× scan loss; u32 already inlines ints); spawned the temporal-cache win `72b35e3` (dateTime FILTER 7.2×, ORDER BY 7.4×) |
| Compressed on-disk perms | **Adopted** | `8e1525c` → `research/compressed-perms-verdict.md` (2.47–2.75×, lazy block-mmap default for compressed dirs, old files compatible) |
| GenAI supports | **Landed** | sparq-introspect `56ee687`; sparq-sim `d8e4db6` (precision@10 0.999); sparq-nlq `d1c6c2f` (replay-tested NL→SPARQL); sparq-vectors `8313931` + verbalization/hybrid-fusion `41db5de`; digest determinism fix `ac905b0` |
| RDF 1.2 triple-term structural storage | **Landed** | `27743f9` — `Stored::Triple` structural ids, round-trip tested |
| Inference / N3 toward EYE parity | **Landed** | sparq-reason: parallel materialization, backward `<=` rules, EYE-vendored cases, `string:encodeForUri` `f96898b`; incremental RDFS `c36eedc` (~4100×/~870×); open follow-up noted in design docs: `reason_n3_stratified`, NAF-aware counting |
| RDF/SPARQL 1.2 | **Landed** | full-board conformance `9917404` incl. 1.2 syntax/eval suites (triple terms, var-in-triple-term, 1.2 builtins, `lang--dir`) |
| SPARQL feature gaps | **Landed** | conformance rounds 1–3 (`b78d726` et al.), FROM/FROM NAMED `5c591d4`, numeric tower, casts `4bd8db1` → 1229/1229 `9917404` |
| SPARQL Update | **Landed** | round-3 dataset model (all GraphUpdateOperations, USING/WITH) `b78d726`; incremental in-place updates `ed5dd61` |
| W3C HTTP server | **Landed** | sparq-server (protocol tests) + hardening `f9e9cd4` (timeouts, shedding, graceful shutdown, QueryBudget) |

## Extension threads

| Thread | Status | Evidence |
|---|---|---|
| T13 conformance in CI | **Landed** | runner `abdaed9`; gating ratchet ≥1229 `dea2be2`/`9917404`; divergence allowlist with stale-entry detection |
| T14 RDF/JS bindings | **Built; npm publish user-gated** | `1400908` — `js/` package `@sparq-org/sparq` 0.1.0, RDF/JS-typed, 16 node tests, js.yml CI, pack dry-run OK; *publication deferred to the user (account + final-name confirmation)* |
| T15 server hardening | **Landed** | `f9e9cd4` |
| T16 CONSTRUCT/DESCRIBE + streaming | **Landed** | `a9bbbd5` — engine + server negotiation, streamed SELECT (−200–355 MB RSS), conformance construct suite |
| T17 incremental updates + WAL | **Landed** | `ed5dd61` (delta-overlay, append-only dict, torn-record-safe WAL, ~1.3 M× on 10-triple inserts); server wiring `0818178` (2.65 s→330 µs) |
| T18 incremental reasoning | **Landed** | `c36eedc` — counting-based RDFS maintenance |
| T19 SHACL | **Landed** | `ec28a88` — sparq-shacl, 98/98 W3C core suite |
| T20 releases & packaging | **Infra landed; first release user-gated** | `d555096` — release.yml/dist.yml (crates.io, GitHub Releases, Docker, Homebrew recipes), python.yml; *no v0.1.0 has been cut/published — cutting a public release is the user's call* |
| T21 Python bindings | **Landed** | `ff215d3` — sparq-py via pyo3/maturin, python.yml CI |
| T22 EXPLAIN + observability | **Landed** | `b49215e` — EXPLAIN/EXPLAIN ANALYZE + Prometheus metrics |
| T23 SEPA subscriptions | **Landed** | `d35f121` — WS added/removed SPARQL-JSON diffs, coalescing re-eval |
| T24a HDT | **Landed** | `4ac813c` — opt-in sparq-hdt reader |
| T24b GeoSPARQL | **Landed** | `6c09e6c` + SPARQL wiring via the function registry `c8c7693` (12 geof: fns, opt-in server feature) |
| T24c RDF stream processing | **Landed** | `0fefc9a` — deterministic RSP-QL windows, 2.2 M triples/s |
| T24d GPU execution | **Measured-and-parked** | `d56d70d` → `research/gpu-verdict.md` (offload 0.04–0.41×; hash-probe lone 1.7–2.3× resident win; re-open conditions recorded) |

## User-added threads (post-roadmap)

| Thread | Status | Evidence |
|---|---|---|
| Similarity text embeddings | **Landed** | `41db5de` — verbalizer API + hybrid fusion + `research/genai-text-embedding-practices.md` |
| Solid access control | **Landed** | design+crate `1c22976` (WAC/ACP as N3 rules, triples-native auth view, 13 correctness tests, comprehensive docs); zero-copy dataset view `b6f9f24` (20–3700× vs copy path) and default wiring `f96898b` (flat ~1 ms session overhead) |
| Upstream proposals | **Resolved/pending** | oxigraph: all 6 fixes verified already on upstream main (unreleased) — nothing to file, `d7369d4`; rdf-tests: 4 drafts + filing plan ready, awaiting user go-ahead |

## Remaining gates (all external to the codebase)

1. AWS quota L-1216C47A ≥194 (or spot SLR) → unblocks the NUMA sweep + full-scale
   Wikidata validation (`hwrun/launch.sh`, <$10).
2. OIDC role + `AWS_BENCH_ROLE_ARN` repo variable → activates bench-ec2.yml (G3).
3. User go-ahead: rdf-tests filings; npm publish; cutting release v0.1.0.
4. Watch: next spargebra crates.io release → retire `vendor/spargebra`.
