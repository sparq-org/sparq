<!-- sq-puyy: trimmed to the concise per-crate README template (sq-9jw5). -->
# sparq-rsp

<p>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Opt-in RSP-QL-style RDF stream processing** for the [sparq](../../README.md) engine:
windowed **continuous SPARQL queries** over timestamped triple streams, as a
deterministic **library** — no async runtime, no wall clock, no service.

The engine never reads a clock: timestamps are application-supplied `u64`s and time
advances only through pushed values, so the whole pipeline is a pure function of the
pushed `(triple, ts)` sequence — replayable, unit-testable, wasm-safe. Like
`sparq-reason` / `sparq-shacl` it is **isolated**: only its `sparq-rsp-wasm` bundle depends on
it, so the core engine and the wasm build carry zero streaming code.

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousQuery, R2S, WindowSpec};

// Average reading per tumbling 60-tick window, tolerating 5 ticks of disorder.
let mut q = ContinuousQuery::register(
    "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/reading> ?v }",
    WindowSpec::time(60, 60).with_max_delay(5),   // RANGE 60 STEP 60
)?
.with_r2s(R2S::RStream);                          // the default: full result per window

# let triple: [Term; 3] = [
#     NamedNode::new_unchecked("http://ex/sensor1").into(),
#     NamedNode::new_unchecked("http://ex/reading").into(),
#     Literal::from(10).into(),
# ];
# let ts: u64 = 0;
q.push(triple, ts, |result| {                     // fires once per CLOSED window
    println!("[{}, {}): {:?}", result.start, result.end, result.rows);
})?;
q.flush(|result| { /* end-of-stream: close everything up to max ts */ })?;
# Ok(()) }
```

## ✨ Features

- **Windows (S2R)** — `WindowSpec::time` half-open `[t0 + k·step, t0 + k·step + range)`
  time windows (tumbling, or overlapping when `step < range`) and `WindowSpec::count`
  CQL-style row windows; closure is driven by the `max_ts − max_delay` watermark, with
  out-of-order tolerance and empty-window reporting. The default-off `session_windows`
  feature adds `WindowSpec::session(gap)`: maximal event-time runs split by an inactivity
  gap greater than or equal to `gap`, with inclusive `[first.ts, last.ts]` bounds.
- **Continuous queries (R2R)** — `ContinuousQuery` (SELECT), `ContinuousConstruct`
  (CONSTRUCT, stream-to-stream), and `ContinuousAsk` (ASK), each parsed **once** at
  `register` into a `sparq_engine::PreparedQuery` and re-run per closed window.
- **Closed-window aggregates** — the default-off `window-aggregate` feature adds
  `window_aggregate(&WindowResult, var, Agg)` for deterministic
  COUNT/SUM/AVG/MIN/MEDIAN/MAX scalar folds over emitted rows, without a clock read or
  another query. <!-- sq-sfle1 -->
- **Relation-to-stream (R2S)** — `R2S::{RStream, IStream, DStream}`: full / added /
  removed rows per window, computed as exact term-level multiset differences.
- **RSP-QL surface syntax + multi-window joins** — `RspqlQuery::parse` reads
  `REGISTER [STREAM|RSTREAM|ISTREAM|DSTREAM] … FROM NAMED WINDOW <w> ON <s> RANGE … STEP …`, and
  `ContinuousMultiQuery` joins across 2 or more named windows on one synchronized
  event-time clock with full RSTREAM/ISTREAM/DSTREAM support.
- **Per-query evaluation budgets** — `.with_budget(QueryBudget)` (re-exported from
  `sparq_engine`) applies the engine's COOPERATIVE limits (`max_rows` / `max_bytes` /
  `cancel`) to EVERY window evaluation, and `.with_window_timeout(Duration)` (native-only)
  installs a refreshed relative deadline (`now + timeout`) at each evaluation start. All of
  them are observed at the executor's coarse polling sites, so they take effect at the NEXT
  poll rather than instantly — a shape answered straight from the index, or one that
  finishes before the first poll, runs to completion unchecked. `max_bytes` caps the
  executor-accounted ESTIMATED working set of one evaluation, not total process memory nor
  the memory of the materialised windows themselves. <!-- sq-xqu -->
- **Pluggable materialisation (`EvalMode`)** — `PersistentDict` (default, compacted
  dictionary), `Rebuild` (v1 baseline), `Delta` (one live graph, per-slide delta), and
  `Snapshot` (one live graph + a cheap O(overlay) immutable point-in-time snapshot per
  closed window), all producing identical results. The `Delta`/`Snapshot` window diff
  runs on the shared eval substrate (`sparq-substrate` `join::delta::DeltaTable`,
  id-level, monomorphic — no dynamic dispatch on the probe path).

## 📚 Learn more

- **How-to** — [`skills/streaming-rsp/SKILL.md`](../../skills/streaming-rsp/SKILL.md)
  (window semantics, R2S diffs, eval modes, RSP-QL syntax, the wasm tier).
- **API reference** — `cargo doc -p sparq-rsp --open` (not on crates.io / docs.rs: `publish = false`).
- **Design** — [`research/ARCHITECTURE.md`](../../research/ARCHITECTURE.md).
- **Performance** — the `throughput` example
  (`cargo run --release -p sparq-rsp --example throughput`; append `-- --json <path>` to
  also write the same rows as a machine-readable JSON document, STDOUT unchanged) and
  [`bench/rsp/`](../../bench/rsp); the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
