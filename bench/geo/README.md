<!-- [OPUS-4.8] sq-tf8n — GeoSPARQL benchmark suite. Design: research/capability-benchmark-program.md §3.5. -->
# GeoSPARQL suite

The GeoSPARQL analogue of the LUBM/SHACL template: an **overview** dashboard row, a
**self-asserting deterministic gate** (regression alerts), and a **competitor** comparison
surface. The gate is **COUNTS-NOT-COORDINATES**: floating-point geometry is not bit-stable, so it
asserts only result-set **sizes** and the OGC compliance **pass count** — never a coordinate value.

## Data substrate

A **FIXED CRS84 point corpus**: 100 000 seeded random `POINT(lon lat)` `geo:wktLiteral`s over an
~8°×8° France-ish window (`seed 20260615`, longitude `[-4, 4)`, latitude `[47, 55)`). The corpus is
generated **deterministically** by `bench/geo/gen.sh`, which shells out to the
`crates/sparq-geo/examples/bench_geo.rs` `gen` subcommand — so the committed `expected.tsv` counts
and `bench_geo`'s in-process fallback corpus are **byte-identical** (no f64-formatting drift between
Rust and shell). The corpus is gitignored + regenerable (`/tmp/geo`); there is **no external
download or toolchain** (pure Rust — no `javac`/`rapper`/Docker).

## Workloads

| Workload | What it counts | Engine surface |
|---|---|---|
| `within10km` | #entities within 10 km great-circle of `(0, 51)` | `GeoIndex::within_distance` (`geof:`/within) |
| `within50km` | #entities within 50 km great-circle of `(0, 51)` | `GeoIndex::within_distance` |
| `nearest_k10` | k=10 nearest entities to `(0, 51)` (invariant: count == k) | `GeoIndex::nearest` |
| `nearest_k100` | k=100 nearest entities to `(0, 51)` | `GeoIndex::nearest` |
| `geof_within` | #corpus points satisfying `geof:sfWithin(point, 1°×1° box)` | `geof::lex::sf_within` (the filter-function path) |
| `geo_compliance_pass` | #OGC topology fixtures (sf/eh/rcc8) matching the spec truth value | `geof::lex::{sf,eh,rcc8}*` |

`queries/*.rq` render the within/`geof:` workloads as GeoSPARQL `SELECT (COUNT(?e) …)` queries —
the EXACT-SEMANTICS form an external GeoSPARQL endpoint executes, so the result-set **size**
cross-checks against `sparq-geo` before any timing is trusted. (The `geof_within` planar-degree-space
`sfWithin` count differs from the great-circle `within*` counts — different operation + unit.)

## Deterministic gate (HARD) vs timing (ADVISORY)

`run.sh` is the self-asserting entry point (the LUBM pattern). It runs `bench_geo bench` over the
fixed corpus and **asserts, per workload, vs `expected.tsv`, exit 1 on any drift**:

- **result-set SIZE** of each within / nearest / `geof:` query (the primary gate — counts only,
  derived by running, never coordinate values).
- **`geo_compliance_pass`** — the OGC GeoSPARQL topology fixture pass count, which may only
  **tighten** (a pass count below the pinned value is a coverage regression).

The compliance ratchet is gated cross-commit as the **deficit** `geo_compliance_deficit`
(= `GEO_COMPLIANCE_MAX − passing`, currently `25 − 25 = 0`) — a smaller-is-better integer with
`mode:"auto"` in `bench/perf-baseline.json`, so coverage can only ratchet **down** (gap G4 of the
design: a larger-is-better score expressed as a deficit, zero `perf-gate.py` change). A broader
hand-curated topology ratchet lives in `crates/sparq-geo/tests/ogc_compliance_ratchet.rs`
(`OGC_RATCHET_FLOOR`, asserted by `cargo test`).

**Timing is ADVISORY** (trend-only, **never hard-gated** — and this dev box is non-canonical, so its
timings are advisory only): the ci-bench hook harvests `geo_<name>_us` (best-of-iters query time)
into the dashboard; it is **not** in `scripts/perf-gate.py`. The only hard `perf-gate` metric is the
integer, runner-immune `geo_compliance_deficit`.

## Running it

```sh
cargo build --release -p sparq-geo --example bench_geo
bench/geo/run.sh                         # self-asserting: exit 1 on any count drift
```

`bench_geo bench <corpus.nt> [iters]` emits, per workload, the `name<TAB>count<TAB>us` contract the
ci-bench hook consumes; `run.sh` asserts the deterministic `count` columns and forwards the contract.
(`sparq-geo` is the *isolated* geo crate — not a `sparq-cli` dependency — so the runner is a crate
`--example`, not a CLI subcommand.) `bench_geo` with no args runs the human-readable latency report
(the `geo-index-bench` registry entry, numbers for the crate README).

## Competitors

| Engine | Lang | License | Adapter kind | Role |
|---|---|---|---|---|
| **GeoSPARQL-Jena / Fuseki-geosparql** | JVM | Apache-2.0 | `http-sparql` | **Compliance bar** — the only triplestore with full **GML + WKT**; since sparq-geo does both, the right like-for-like for *coverage*. POST the `queries/*.rq` → parse SPARQL-JSON → count. |
| **PostGIS** | C | GPL-2.0 (server) | — (bespoke) | **LOOSE lower bound only** — relational R-tree (`rstar`-style) sub-component, **NOT** a `geof:`/SPARQL-graph-join competitor. Must match CRS/operation semantics (sparq-geo great-circle vs PostGIS geodesic/projected) or omit. |

Registered in `bench/competitors.json` (with the `engines`/`values` dashboard seam **empty** in git
per AGENTS.md — no hard-coded perf). A real `scripts/gather-competitors.sh --run --only geosparql-jena`
writes git-ignored `bench/competitor-results/`. Docker-based competitors are inherently gather-only on
a Docker EC2 box (no Docker on the dev box), so they add zero recurring CI cost.

**Honest caveat:** sparq-geo's `within*` queries use a **great-circle** metric in CRS84 long/lat
degree space; an external endpoint must use the same metric (and `geof:distance` semantics) for the
**count** to agree, and the **count** must agree before any timing is meaningful. PostGIS geodesic vs
projected distance is a different operation — it is a sub-component lower bound, not a SPARQL peer.

## Geographica real-world family (opt-in) <!-- [FABLE-5] sq-hmd7l.29 -->

`scripts/bench/geo-same-box.sh` grows a SECOND workload family under `GEO_GEOGRAPHICA=1`: the
**LGD/GeoNames slices of Geographica** (Garbis/Kyzirakos/Koubarakis, ISWC 2013 — the
reviewer-recognised real-world GeoSPARQL suite).

- **Data** — `geographica.sh`: fetches the upstream tarballs (gather-only, `/tmp/geographica`),
  verifies **pinned sha256s** (upstream AND the merged corpus — a mismatch fails, never silently
  benchmarks different data), and **normalises** the Strabon-era `EPSG/4326` lon-lat anchor to bare
  CRS84 so both engines interpret every literal identically (jena would axis-swap it per the EPSG
  registry; sparq treats the non-OGC-form IRI as an opaque CRS — divergent either way).
- **Queries** — `queries-geographica/*.rq`: pinned **COUNT-wrapped** translations of the upstream
  micro non-topological (`q04`/`q05` buffer), spatial-selection (`q07`–`q17`) and spatial-join
  (`q19`) queries; each file header records the exact translation deltas. `q15` gets a `.jena.rq`
  rendering (`spatialF:nearby`) because standard `geof:distance`+`uom:metre` is non-executable on
  jena 5.4.0 (`research/gap-geo-2026-07.md` §6d). Non-topological queries count `COUNT(?ret)` over a
  `BIND`, so only rows where the function **evaluated** count — a real function-evaluation oracle.
- **Oracle** — `queries-geographica/expected-geographica.tsv`: counts **derived by running BOTH
  engines** on the pinned corpus (exact agreement on every bounded workload at pin time). The family
  envelope (`geo-geographica-<ts>.json`) withholds a sparq timing unless sparq == expected and a
  jena timing unless jena == sparq — same counts-before-timing invariant as the base family.
- **sparq runner** — `bench_geo query <corpus.nt> <query.rq> [iters]` (needs the crate's default
  `engine` feature): in-process load + `geof:` registry + full SPARQL eval, emitting the same
  `name\tcount\tus` contract; the COUNT scalar convention matches `http_sparql_adapter.py`.
- **q19 join** — the ~22k×12k naive cross-product spatial join has **no indexed path on either
  engine**; at default caps it records honest `ERROR(timeout)` rows on BOTH sides. That absence is
  the recorded comparative result, not a bug in the harness.

This family is comparison-only (opt-in, no CI cost); the per-commit HARD gate stays `run.sh` +
`expected.tsv` above, unchanged.

## GeoSPARQL Compliance Benchmark family (opt-in) <!-- [SONNET-4.6] sq-ql2iy -->

The **published cross-engine GeoSPARQL row** — Jovanovik/Homburg/Spasić, *A GeoSPARQL
Compliance Benchmark* (ISPRS IJGI 10(7):487, 2021; arXiv:2102.06139) — is scored by a
specific artifact: 206 SPARQL queries over the 30 GeoSPARQL 1.0 requirements, each with an
expected SPARQL-Results-XML answer. It is a **conformance** family, not a timing one: the
metrics are *correct answers* and a *requirements-weighted compliance %*.
This is a community benchmark, not OGC certification.

```sh
GSB="$(bash crates/sparq-geo/fetch-geosparql-benchmark.sh)" # returns the pinned checkout
cargo run --release -p sparq-geo --features geosparql_rewrite,geof_accessors \
    --example gsb_compliance -- "$GSB/src/main/resources" # TSV, then the two metrics
```

- **Data + queries** — `crates/sparq-geo/fetch-geosparql-benchmark.sh` is the shared recipe for
  the example and the ratchet. It checks out the pinned Git commit by default;
  `GEOSPARQL_BENCHMARK_PIN=HEAD` or another revision overrides it, and
  `GEOSPARQL_BENCHMARK_DIR` selects an existing checkout without modifying it.
  Queries, answers and data are under the returned checkout's `src/main/resources`.
  A clean checkout already at the full pinned commit can be reused offline;
  mutable refs such as `HEAD` refresh from upstream. Locally changed fixtures and
  directories without checkout provenance are refused.
  The external GPL-2.0 corpus is gitignored and never vendored.
- **System under test** — one sparq GeoSPARQL stack driven uniformly across all 206 queries
  (nothing is special-cased per query): RDF/XML load, `sparq-reason` RDFS materialisation,
  the `geof:` registry with `geof_accessors`, and a query entry point. The last two are
  runtime toggles — `GSB_RDFS=0` and `GSB_REWRITE=0` — because **both change the score
  materially**, and a compliance number without its configuration is meaningless. The
  measured 2×2 matrix, and the finding that the opt-in `geosparql_rewrite` entry point
  currently *lowers* the score, are in `research/gap-conformance-cross-engine-2026-07.md`
  §3.7.
- **Scoring** — reproduces the benchmark's own weighting (each requirement `1/30`, split
  over its query groups, a 4-query serialisation group splitting `1/3, 1/3, 1/6, 1/6`) and
  its own comparator, shared with the ratchet: ordered variables and rows; WKT
  space/newline removal, trimming and lowercasing; GML canonicalization without
  comments; and sequential `-alternative-N.srx` answers. XML character-data
  whitespace, numeric lexical forms and geometry serialization differences remain
  significant. Invalid GML is compared verbatim. The scoring table and comparator
  have corpus-free tests under `cargo test -p sparq-geo --all-features`.
- **Triage** — `GSB_DEBUG=<query-id-prefix>` dumps the rewritten query plus actual-vs-expected
  result sets for the matching failures. Every non-passing requirement in the recorded run
  was triaged this way before being written down as a gap.

Comparison-only (opt-in, no CI cost, needs network): the per-commit HARD gate stays `run.sh`
+ `expected.tsv` above, unchanged. The recorded score lives in the research record, not here.
