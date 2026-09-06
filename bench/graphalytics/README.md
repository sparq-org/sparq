<!-- [SONNET-4.6] sq-hmd7l.13 — LDBC Graphalytics validation-gated comparison. -->
<!-- internal-stub -->
# LDBC Graphalytics comparison — sparq-algos vs igraph / NetworKit

Validation-gated graph-analytics comparison: for every (engine, algorithm) pair the
harness **validates the output against a Graphalytics reference before it reports a
timing**. A red gate is reported as red; no timing is printed beside it.

Gap record: [`research/gap-graphalytics-2026-07.md`](../../research/gap-graphalytics-2026-07.md).
Registry entry: `graphalytics` in [`bench/benchmarks.toml`](../benchmarks.toml).

## Running

```sh
bash bench/graphalytics/run.sh --smoke   # acceptance: sparq leg, committed fixture, exit 0
bash bench/graphalytics/run.sh           # full panel; competitor columns if installed
```

A real LDBC run points the harness at a downloaded dataset:

```sh
GX_DATA_DIR=/data/graphalytics GX_DATASET=example-directed bash bench/graphalytics/run.sh
```

Env knobs: `GX_DATA_DIR`, `GX_DATASET`, `GX_ALGOS`, `GX_ITERS`, `GX_ALGOS_BIN`, `CARGO`,
`GX_SKIP_ORACLE_CROSSCHECK`. See the header of `run.sh`.

## Algorithm coverage — the honest intersection

Graphalytics specifies six algorithms. `sparq-algos` covers two of them conformantly:

| Graphalytics | sparq-algos | status |
|---|---|---|
| PR | `pagerank` | **conformant** — validated |
| WCC | `weakly_connected_components` | **conformant** — validated |
| CDLP | `label_propagation` | **non-conformant variant** — asynchronous, run-to-fixed-point |
| BFS | — | **feature gap** |
| LCC | — | **feature gap** |
| SSSP | — | **feature gap** (`NodeGraph` carries no edge weights) |

A missing algorithm produces an explicit `FEATURE GAP` row and no timing. It is never
skipped silently and never given a fabricated number — that rule is the point of the
suite, not a caveat on it.

## Why the reference oracle is independent

`gx.py reference` implements the Graphalytics specification in Python — a different
language and a separate derivation from the Rust under test. The reference outputs
committed under `data/*/` are generated from it, and **every run re-derives them and diffs**
(`GX_SKIP_ORACLE_CROSSCHECK=1` opts out). Without that, a committed reference decays into
"whatever sparq printed last", which validates nothing.

`run.sh` also runs a **gate self-test** before anything else: it feeds the validator known-
wrong answers of each kind (a float outside epsilon, a NaN, either infinity, a missing
vertex, a merged partition, a split partition) and known-equivalent ones (a float inside
epsilon, a pure relabelling), and aborts unless the validator rejects the former and accepts
the latter. A gate that cannot go red would make every green row below it meaningless — and
NaN is the sharp case, because every comparison against it is false, so a tolerance check
that does not reject non-finite values *explicitly* accepts a wholly diverged result.

## The committed fixture is NOT an LDBC dataset

`data/smoke-directed/` is a **sparq-authored 10-vertex graph written in the Graphalytics
on-disk format**, not a dataset distributed by LDBC. It exists so the format parser and the
validation gate are exercised in CI without a multi-gigabyte download. Its shape is chosen
to make the easy-to-get-wrong parts of the spec load-bearing: two dangling vertices (so
PageRank's dangling-mass redistribution is exercised), one isolated vertex (so the RDF
projection must preserve a degree-0 node), and three weakly connected components.

## What is and is not comparable

**Comparable.** WCC across all three engines: same semantics, exact partition, no
termination ambiguity.

**Not comparable at all — igraph PageRank.** Graphalytics PageRank is a *fixed sweep count*.
NetworKit exposes `maxIterations` and can run that form, so it is gated normally. igraph's
`pagerank()` *solves* for the stationary distribution (PRPACK) and has no such knob, so the
fixed-sweep reference is not its oracle — gating against it would print a large per-vertex
delta that reads as an igraph correctness failure when it is only a difference of
termination rule. **This suite generates no converged oracle**, so the row is recorded as a
`SEMANTIC-GAP`: labelled `semantics=converged`, neither validated nor timed. Producing an
independent converged oracle and gating igraph against *that* is future work, not something
the harness quietly does today.

**Not the same job.** igraph and NetworKit are embedded graph libraries handed a prepared
edge array. `sparq-algos` runs over an RDF triple store's dictionary ids. The harness
reports both halves of that trade per algorithm: `project` (store → adjacency view, which
is all sparq needs) and `export_edgelist` (what serialising the same graph out to the flat
text an embedded library consumes would cost). Quoting one without the other would be
dishonest in either direction.

Wall-clock numbers from a shared or CI box are **not canonical** — see
[`bench/CATALOG.md`](../CATALOG.md).
