<!-- [OPUS-4.8] sq-4kr5: internal-stub README for a publish=false crate. -->
# sparq-bench

The **benchmark + differential harness** for [sparq](../../README.md): it runs
the same dataset and queries through `sparq` and through
[Oxigraph](https://github.com/oxigraph/oxigraph) (a mature, independent Rust
SPARQL engine), cross-checks that both return the same **answers** — not just the
same number of them — and reports load/query timings and peak memory.

Why it exists: the cross-check against an independent implementation is a cheap,
continuous oracle that catches engine regressions, while the timings feed the
benchmarks dashboard. Two differential fuzzers live here: `fuzz` (queries; nightly
`differential.yml`), whose value comparisons are adjudicated by the
engine-independent [`sparq-difftest`](../sparq-difftest/README.md) rather than by
either engine under test; and `update-fuzz` (SPARQL UPDATE sequences — ground terms,
non-canonical numeric lexicals, blank nodes compared under RDFC-1.0 isomorphism,
`LOAD`, RDF-1.2 triple terms — through both sparq update paths vs Oxigraph,
canonical per-step store + probe compare; nightly `differential-update.yml`). The
oracles, and the adjudicated divergence classes both fuzzers read from
`bench/differential-divergences.json`, are documented in their module headers.

> **Internal tooling — not published** to crates.io (`publish = false`). Run it
> as a workspace binary; it is not a library. Measured numbers belong in the
> [benchmarks dashboard](https://sparq.jeswr.org/dev/bench), never baked into docs.

Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
