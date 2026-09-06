# sparq v0.1.1 — first complete release

An experimental, from-scratch RDF triplestore + SPARQL engine in Rust.

**Engine** — dictionary-encoded store, six sorted permutation indexes (compact 3-index mode for constrained targets), parallel + streaming loaders with transparent gz/bz2/zst, out-of-core memory-mapped indexes (fast build + near-instant open with ~0 committed heap; figures in `research/BENCHMARKS.md`), SPARQL 1.1 SELECT/ASK/CONSTRUCT/DESCRIBE + Update, named graphs, sort-merge / hash / worst-case-optimal joins with cardinality-based planning.

**Inference — 100% across the board** — opt-in RDFS / OWL 2 RL / N3 materialization: every W3C inference suite run passes at 100% (pass + documented divergence) with zero silent skips — RDF Semantics 48/48; OWL 2 RL 78 pass, 0 fail, 13 documented divergences (details: `inference-conformance-report.md`). Opt-in proof trees (`explain` feature) return ZK-witness-friendly derivations.

**Benchmarks** — vs native QLever, same machine, compute-only, cold: faster on synthetic 10M/100M join/OPTIONAL workloads (in-memory and mmap) and on every query of a real skewed dataset. Per-query baselines + speedup ranges are single-sourced in `bench/qlever-baselines.md`; honest caveats in the changelog.

**Bindings parity** — Python `sparq` (pyo3/abi3 ≥3.9): query/update, CONSTRUCT/DESCRIBE, named graphs, RDFS/OWL-RL/N3 reasoning + custom N3 rules, inconsistency reports — 38 tests. JS `@sparq-org/sparq` (RDF/JS-typed; shipped wasm ~1.2 MB after wasm-opt, ~1.6 MB raw build): named graphs, streaming results, in-place/delta updates, compressed ingest (zstd/gzip), dictionary-fetch protocol client — 42 tests.

**Serving** — W3C SPARQL 1.1 Protocol + Graph Store Protocol (read) server with content negotiation, streamed SELECT bodies (~40% peak-RSS reduction on 1M-row results), Prometheus metrics; Docker: `ghcr.io/sparq-org/sparq-server:0.1.1` (distroless). CLI ships hardware-tiered binaries (x86-64 v1–v4, arm64 Linux, Apple silicon + Intel mac, Windows x64/arm64) — verify with `shasum -a 256 -c SHA256SUMS`.

**Install** — binaries below · `cargo install sparq-cli` · `npm i @sparq-org/sparq` · `docker pull ghcr.io/sparq-org/sparq-server:0.1.1`

**Known caveat** — crates.io builds resolve upstream `spargebra` 0.4.6: a small set of vendored SPARQL-parser conformance fixes apply only to git builds until the upstream PRs land (`vendor/spargebra/SPARQ-PATCHES.md`). APIs are unstable (0.x). The earlier `v0.1.0` tag was an incomplete CI tag and has no final GitHub Release; v0.1.1 is the first complete release. Full details: [CHANGELOG.md](https://github.com/sparq-org/sparq/blob/v0.1.1/CHANGELOG.md).
