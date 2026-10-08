---
name: streaming-rsp
description: Use when running continuous/standing SPARQL over a live RDF triple stream with the sparq engine — sliding/tumbling time windows (RANGE/STEP), count (ROWS) windows, opt-in textual T0/MAXDELAY clauses, opt-in gap-triggered session windows, opt-in closed-window scalar aggregates, RSTREAM/ISTREAM/DSTREAM output, RSP-QL surface syntax (REGISTER STREAM, FROM NAMED WINDOW ... ON ... RANGE/STEP), and multi-window joins (WINDOW <w1>{} JOIN WINDOW <w2>{}). Covers the sparq-rsp crate's ContinuousQuery / ContinuousConstruct / ContinuousAsk / ContinuousMultiQuery / RspqlQuery / WindowSpec / window_aggregate.
---

# sparq-streaming-rsp

`sparq-rsp` runs **windowed continuous SPARQL** (RSP-QL-style RDF Stream Processing) over a stream of `(triple, timestamp)` elements, as a deterministic **synchronous library** — no async runtime, no wall clock, no service. You push timestamped triples; it closes windows on a watermark and fires your callback once per closed window with the SELECT / CONSTRUCT / ASK result. It is a fully isolated, opt-in crate: nothing but its own `sparq-rsp-wasm` bundle depends on it, and the core engine and the **lean** `sparq-wasm` bundle carry zero streaming code (streaming ships as a *separate*, lazy-loaded `sparq-rsp-wasm` bundle — see below). Time/count windows are the default surface; textual window origins and lateness tolerances require the default-off `window-origin` cargo feature, while gap-triggered session windows require `session_windows`.

## Quickstart

`Cargo.toml`:

```toml
[dependencies]
sparq-rsp = { path = "../sparq/crates/sparq-rsp" } # features: session_windows, window-aggregate, window-origin
oxrdf = { version = "0.3", features = ["rdf-12"] } # the term model (Term/NamedNode/Literal)
```

```rust
use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousQuery, WindowSpec};

// Average reading per TUMBLING 60-tick window, tolerating 5 ticks of disorder.
let mut q = ContinuousQuery::register(
    "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/reading> ?v }",
    WindowSpec::time(60, 60).with_max_delay(5), // RANGE 60 STEP 60, lateness 5
)?;

// A stream element is a subject/predicate/object [Term; 3] plus a u64 timestamp.
let reading = |v: i32| -> [Term; 3] {
    [NamedNode::new_unchecked("http://ex/sensor1").into(),
     NamedNode::new_unchecked("http://ex/reading").into(),
     Literal::from(v).into()]
};

// push() fires on_result once per window CLOSED by this push (0+ times, oldest first).
q.push(reading(10), 0,  |r| println!("[{},{}) -> {:?}", r.start, r.end, r.rows))?;
q.push(reading(20), 30, |_| {})?;
// flush() = end-of-stream: close everything up to the last timestamp seen.
q.flush(|r| println!("final [{},{}) -> {:?}", r.start, r.end, r.rows))?;
// Integer AVG comes back as xsd:decimal "15.0" per SPARQL aggregate typing.
# Ok::<(), String>(())
```

The whole pipeline is a pure function of the pushed `(triple, ts)` sequence: replayable, unit-testable, wasm-safe. Wrapping pushes in tokio / a thread / a browser timer is your one-liner, not this crate's dependency.

### In-tab live streaming: the tier-b `sparq-rsp-wasm` ("W-rsp") bundle ([OPUS-4.8] sq-nzcb)

Because `sparq-rsp` reads no wall clock and runs no async runtime, it compiles to `wasm32-unknown-unknown` and ships as a **separate, lazy-loaded** wasm bundle (`crates/sparq-rsp-wasm`) — NOT folded into the lean `sparq-wasm` triplestore bundle (the `sparq-reason-wasm` "W-reason" pattern). It exposes a single stateful JS handle, `Rsp`, for the showcase site's `/surface/streaming-rsp` page, where the **browser tab drives the logical clock**:

```js
import init, { Rsp } from "./sparq_rsp_wasm.js";
await init();
const q = Rsp.select("SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/reading> ?v }",
                     60, 60, 0, "rstream"); // range, step, maxDelay, "rstream"|"istream"|"dstream"
const closed = JSON.parse(q.push("<http://ex/s1>", "<http://ex/reading>", "10", 0)); // -> "[]" until a window closes
JSON.parse(q.flush()); // end-of-stream; q.lateDropped() = arrivals too late for any window
```

Each `push(s, p, o, ts)` / `flush()` returns a JSON array of the windows that just closed: `{"start","end","results"}`, where `results` is a standard self-contained **SPARQL 1.1 JSON** results document (from the engine's serialiser). Triple terms are **Turtle** syntax — the bare-numeric shorthand (`10`, `10.5`) works, alongside `<iri>`, `"str"`, `"str"@en`, `"v"^^<dt>`, `_:b`. The bundle wraps the single-window `ContinuousQuery` SELECT form only; CONSTRUCT/ASK and `ContinuousMultiQuery` stay native for now. Zero `unsafe`, no serde, no regex (it is the leanest of the wasm bundles); the wasm-deps guard keeps the native-only heavy deps out of its graph.

The numeric args `range` / `step` / `maxDelay` / `ts` (and the `lateDropped()` return) are plain JS **`number`s, not `BigInt`s** ([OPUS-4.8] sq-734a, issue #832) — pass `60`, not `60n`. Each is a whole logical-time value in `[0, 2^53-1]` (`Number.MAX_SAFE_INTEGER`, the exact-integer range of a `number`); a fractional / negative / out-of-range value is a clean error, not a thrown coercion. They map to the native crate's `u64` ticks (a `u64` wasm-bindgen param would be a `BigInt`, which is why the boundary is `number`).

## Key APIs

All public items are re-exported at the crate root (`sparq_rsp::…`).

```rust
// --- S2R: the window spec (Copy enum + builders) ---
WindowSpec::time(range: u64, step: u64) -> WindowSpec   // panics if range==0 or step==0
WindowSpec::count(rows: usize) -> WindowSpec            // CQL count window; panics if rows==0
WindowSpec::session(gap: u64) -> WindowSpec             // feature=session_windows; panics if gap==0
  .with_max_delay(d: u64) -> WindowSpec   // out-of-order tolerance (time windows only)
  .with_t0(t0: u64)       -> WindowSpec   // RSP-QL window origin (time windows only; default 0)
  .with_slide(s: usize)   -> WindowSpec   // report cadence (count windows only; default 1)

// --- R2S: relation-to-stream operator (Default = RStream) ---
enum R2S { RStream, IStream, DStream }

// --- R2R materialisation strategy (Default = PersistentDict) ---
enum EvalMode { Rebuild, PersistentDict, Delta, Snapshot }

// --- Continuous SELECT: WindowResult { start, end, vars: Vec<Variable>, rows: Vec<Vec<Option<Term>>> } ---
ContinuousQuery::register(sparql: &str, spec: WindowSpec) -> Result<ContinuousQuery, String>
  .with_r2s(R2S) -> Self            // builder
  .with_mode(EvalMode) -> Self      // builder; call BEFORE first push (resets stream state)
  .with_budget(QueryBudget) -> Self // per-window-evaluation caps (max_rows/max_bytes/cancel); QueryBudget re-exported from sparq_engine
  .with_window_timeout(Duration) -> Self // native-only: each window eval's deadline = now + timeout
  .push(triple: [Term;3], ts: u64, on_result: impl FnMut(WindowResult)) -> Result<(), String>
  .flush(on_result: impl FnMut(WindowResult)) -> Result<(), String>
  .late_dropped() -> u64            // arrivals dropped because every covering window had closed

// --- Optional closed-window scalar fold (feature = "window-aggregate") ---
enum Agg { Count, Sum, Avg, Min, Median, Max } // [GPT-5.6] sq-sfle1
window_aggregate(window: &WindowResult, var: &str, aggregate: Agg) -> Option<f64>

// --- Continuous CONSTRUCT: GraphResult { start, end, triples: Vec<Triple> } (stream->stream) ---
ContinuousConstruct::register(sparql: &str, spec: WindowSpec) -> Result<_, String>
  .with_r2s / .with_mode / .with_budget / .with_window_timeout / .push(.., FnMut(GraphResult)) / .flush / .late_dropped

// --- Continuous ASK: AskResult { start, end, value: bool } (one boolean per window) ---
ContinuousAsk::register(sparql: &str, spec: WindowSpec) -> Result<_, String>
  .with_mode / .with_budget / .with_window_timeout / .push(.., FnMut(AskResult)) / .flush / .late_dropped

// --- RSP-QL surface syntax + multi-window joins ---
RspqlQuery::parse(text: &str) -> Result<RspqlQuery, String>
  // fields: output_stream: Option<NamedNode>, r2s: R2S,
  //         windows: Vec<WindowDecl { window, stream, spec }>, sparql: String (WINDOW->GRAPH rewrite)
ContinuousMultiQuery::register(rspql_text: &str) -> Result<ContinuousMultiQuery, String>
  .with_budget(QueryBudget) / .with_window_timeout(Duration)  // per evaluation TICK (the multi-window analogue)
  .push(stream: &NamedNode, triple: [Term;3], ts: u64, on_result: impl FnMut(WindowResult)) -> Result<(), String>
  .flush(on_result: impl FnMut(WindowResult)) -> Result<(), String>
  .window_iris() -> Vec<&NamedNode>   .output_stream() -> Option<&NamedNode>   .r2s() -> R2S

// --- Low-level S2R only (no query): WindowedStream<[Term;3]>, Window { start, end, triples } ---
TripleStream::new().push(triple, ts).push_item(item).push_batch(items)  // batch preserves iterator order
WindowedStream::empty(spec) / ::new(stream: TripleStream, spec)
  .push(triple, ts)  .take_closed() -> Vec<Window>  .flush() -> Vec<Window>  .late_dropped()
```

`register` parses + validates the query **once** (a malformed or wrong-form query is rejected here, not at the first window) and keeps a `sparq_engine::PreparedQuery`; every window executes the prepared algebra with no re-parse.

## Common recipes

**Sliding window + ISTREAM (only newly-appearing rows):** `step < range` overlaps; `IStream` emits the multiset difference `current ∖ previous`.

```rust
use sparq_rsp::{ContinuousQuery, R2S, WindowSpec};
let mut q = ContinuousQuery::register(
    "SELECT ?s WHERE { ?s <http://ex/active> ?o }",
    WindowSpec::time(1000, 100),   // RANGE 1000 STEP 100 — slides 10x within its range
)?.with_r2s(R2S::IStream);
// q.push(.., |r| { /* r.rows = subjects that appeared since the previous window */ })?;
# Ok::<(), String>(())
```
Use `R2S::DStream` for rows that *disappeared* (`previous ∖ current`); DSTREAM relies on empty windows being reported, which they are.

**Count (ROWS) window — last N arrivals, regardless of time:**

```rust
use sparq_rsp::{ContinuousQuery, WindowSpec};
// Last 100 arrivals in arrival order; report every 10th arrival.
let spec = WindowSpec::count(100).with_slide(10);
let mut q = ContinuousQuery::register("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }", spec)?;
# Ok::<(), String>(())
```

**Session window — split after an inactivity gap:** enable the `session_windows`
cargo feature, then use `WindowSpec::session(10)`. Events at `t` and `t + 9`
share a session; events at `t` and `t + 10` start separate sessions. The gap is
measured in the stream's application-supplied `u64` timestamp ticks, never wall
clock, and reported bounds are inclusive `[first_event_ts, last_event_ts]`.

**Fold an emitted SELECT window to a scalar:** enable `window-aggregate`, then call
`window_aggregate(&result, "v", Agg::Sum)`. `Count` includes every emitted row,
even when `v` is unbound or non-numeric. `Sum`/`Avg`/`Min`/`Median`/`Max` use
well-formed XSD numeric literal bindings only. `Median` returns the middle numeric
binding after sorting, or the arithmetic mean of the two middle bindings for an even
count. An unknown projected variable returns `None`; an empty numeric input returns
`Some(0.0)` for `Sum` and `None` for `Avg`/`Min`/`Median`/`Max`.

**CONSTRUCT to transform a stream into another stream (each window -> a graph):**

```rust
use sparq_rsp::{ContinuousConstruct, WindowSpec};
let mut q = ContinuousConstruct::register(
    "CONSTRUCT { ?s <http://ex/observed> ?v } WHERE { ?s <http://ex/value> ?v }",
    WindowSpec::time(10, 10),
)?;
// q.push(.., |g| { for t in g.triples { /* emit normalised observation triple */ } })?;
# Ok::<(), String>(())
```

**ASK as a cheap per-window condition watch** (`ContinuousAsk` returns one `bool` per window; the engine early-exits on the first solution).

**RSP-QL textual query — multi-window join across two streams:** `WINDOW <w>` is rewritten to `GRAPH <w>`; each window is materialised as a named graph keyed by its IRI and joined by shared variables. Push is **tagged with the source stream**; a triple on one stream advances the shared watermark of windows on the others, so closure is synchronized.

```rust
use oxrdf::{NamedNode, Literal, Term};
use sparq_rsp::ContinuousMultiQuery;

let mut q = ContinuousMultiQuery::register("\
REGISTER STREAM <http://ex/out> AS
SELECT ?room ?v WHERE {
  WINDOW <http://ex/w1> { ?s <http://ex/value> ?v }
  WINDOW <http://ex/w2> { ?s <http://ex/in> ?room }
}
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/w2> ON <http://ex/meta> RANGE 10 STEP 10")?;

let temp = NamedNode::new_unchecked("http://ex/temp");
let meta = NamedNode::new_unchecked("http://ex/meta");
let triple = |s: &str, p: &str, o: Term| -> [Term;3] {
    [NamedNode::new_unchecked(format!("http://ex/{s}")).into(),
     NamedNode::new_unchecked(format!("http://ex/{p}")).into(), o] };
q.push(&meta, triple("s1", "in", NamedNode::new_unchecked("http://ex/kitchen").into()), 1, |_| {})?;
q.push(&temp, triple("s1", "value", Literal::from(21).into()), 2, |r| { /* joined rows */ })?;
q.flush(|_| {})?;
# Ok::<(), String>(())
```

With the default-off `window-origin` feature, a time-window declaration may
append `T0 <duration>` and then `MAXDELAY <duration>` in that fixed order:

```text
FROM NAMED WINDOW <http://ex/w1> ON <http://ex/temp> RANGE 10 STEP 5 T0 100 MAXDELAY 3
```

Both clauses accept the same ISO-8601 or bare-integer duration literals as
`RANGE` and `STEP`. `T0` changes the window origin; `MAXDELAY` sets the
event-time lateness tolerance. Either clause may be omitted.

**Just want windows, no SPARQL?** Use `WindowedStream` directly: `let mut ws = WindowedStream::empty(WindowSpec::time(10,10)); ws.push(t, ts); for w in ws.take_closed() { /* w.start, w.end, w.triples */ }`.

## Gotchas / feature flags / prerequisites

- **No async and no clock.** Timestamps are **application-supplied `u64`s** (logical ticks, epoch millis, sequence numbers — your scale); the engine never reads the wall clock. Time advances only through pushed timestamps. A quiet stream closes nothing until the next push or `flush()`. Default-off features are `session_windows`, `window-aggregate`, and `window-origin`; time/count behaviour and the established textual parser are unchanged without them.
- **Window bounds depend on the window type.** Time windows are half-open `[start, end)`: `RANGE 10 STEP 10` gives `[0,10) [10,20) …` (a triple at `ts=10` is in `[10,20)` only). `step < range` ⇒ overlapping (sliding) windows; `step > range` leaves uncovered gaps. Count and session windows report inclusive `[first.ts, last.ts]` content bounds. For sessions, a consecutive gap `< gap` extends the run and a gap `>= gap` splits it.
- **Watermark + lateness.** A window closes when `max_ts_seen − max_delay` reaches its `end`. `with_max_delay(d)` is the out-of-order tolerance (default 0 = close at first sight of a newer-window triple). An arrival whose *every* covering window has already closed is dropped and counted in `late_dropped()`. `flush()` ignores `max_delay` and closes everything up to the last timestamp seen.
- **Empty windows are reported** (evaluated + delivered) when the watermark jumps a gap — DSTREAM needs to observe results disappear. Windows wholly closed before the first arrival's watermark are skipped (a stream starting at `ts=10⁹` won't replay a billion empties).
- **Materialisation is set-semantic:** a window is an RDF *graph*, so the same triple at several timestamps within one window counts once. CONSTRUCT results are triple sets (exact set-diff for I/DSTREAM); SELECT results are multisets diffed by 64-bit `FxHasher` row hashes (a hash collision could theoretically suppress a diff — accepted as vanishingly unlikely).
- **`register` rejects the wrong query form:** `ContinuousQuery` requires SELECT, `ContinuousConstruct` requires CONSTRUCT, `ContinuousAsk` requires ASK. Errors come back as `Err(String)` at registration. `push`/`flush` errors are engine evaluation errors.
- **Per-registered-query budgets** ([SONNET-4.6] sq-xqu): `.with_budget(QueryBudget)` applies the engine's cooperative limits (`max_rows` / `max_bytes` / `cancel`) to EVERY window (or multi-window tick) evaluation — a best-effort ceiling on one pathological window, NOT a hard resource guarantee. `max_bytes` prices the executor-accounted ESTIMATED working set of that one evaluation, not total process memory nor the memory of the materialised windows themselves. A `deadline` inside the passed budget stays ABSOLUTE; use `.with_window_timeout(Duration)` (native-only, like `QueryBudget::deadline` itself) to install a REFRESHED per-evaluation deadline (= now + timeout at each evaluation start) rather than a strict wall-clock cap on the evaluation's duration. A tripped budget is the evaluation error of the `push`/`flush` that closed the window (`"query budget exceeded (…)"`), so remaining closed windows are dropped like any evaluation error. Enforcement is the engine's coarse cooperative polling: shapes answered straight from the index (e.g. a single-pattern ASK count) or finishing a tiny LIMIT-1 scan before the first poll complete unbounded — they do bounded work by construction.
- **`with_mode` must precede the first push** (switching mode resets stream state). Default `EvalMode::PersistentDict` wins every measured scenario and bounds dictionary memory to the *live* window vocabulary via refcount-exact compaction. `Rebuild` bounds memory to one window. `Delta` keeps one live graph evolved by per-slide deltas (kept for huge-window / cheap-eval cases; never the benchmark winner); the consecutive-window diff itself runs on the shared eval substrate (`sparq-substrate` `join::delta::DeltaTable`, id-level, monomorphic — the previous window's build table persists across slides so it is never re-hashed). [FABLE-5] sq-2n1q3.4 `Snapshot` is `Delta` plus a cheap `O(overlay)` **immutable point-in-time** `Graph::snapshot` per closed window — a logically-independent, `Send + Sync` view the engine (or your callback) can retain or publish across windows, where `Delta`'s live `&Graph` borrow cannot. Results are identical across all four modes.
- **RSP-QL parser scope (`RspqlQuery::parse` / `ContinuousMultiQuery`):** parses `REGISTER [STREAM|RSTREAM|ISTREAM|DSTREAM] <out> AS`, `FROM NAMED WINDOW <w> ON <s> [RANGE <dur> [STEP <dur>] [T0 <dur>] [MAXDELAY <dur>]]` (tumbling when STEP omitted), and `WINDOW <w> { … }` (rewritten to `GRAPH <w> { … }`). The `T0`/`MAXDELAY` clauses require `window-origin`; they are optional, ordered, and time-window-only. Durations are ISO-8601 (`PT10S`, `PT1M30S`, `PT2H`, `P1D`; **seconds resolution**, years/months/weeks rejected) or bare integers (logical ticks). IRIs may be `<…>` or prefixed names resolved against the body's `PREFIX`/`BASE`. **Scoped out** (use the programmatic `WindowSpec` instead): window *variables* (`WINDOW ?w`), `ROWS` count windows, session windows, and relative `NOW-PT…TO…` bounds. `ContinuousMultiQuery` requires ≥2 windows (use `ContinuousQuery` for one); 3 or more windows work — each gets its own S2R state on the shared synchronized clock. RSTREAM/ISTREAM/DSTREAM are all supported: `REGISTER ISTREAM <out> AS` emits per-tick added rows; `REGISTER DSTREAM <out> AS` emits per-tick removed rows (multiset diff against the previous tick's full join result). [SONNET-4.6] sq-2n1q3.3
- **Term model is `oxrdf`** (`oxrdf::Term`/`NamedNode`/`Literal`); stream elements are `[Term; 3]`. Add `oxrdf` with `features = ["rdf-12"]` to match the workspace.

## Conformance / correctness ratchet (honest scope)

There is **no W3C/OGC/IETF Recommendation for RDF Stream Processing** and no normative RSP conformance test suite — RSP-QL is a W3C-**community** spec and SRBench (Zhang/Della Valle/Calbimonte et al., ISWC 2012) is a *benchmark*. So sparq does **not** claim RSP standards conformance. Instead `crates/sparq-rsp/tests/srbench_oracle.rs` is an honest **sparq-EXTENSION** ratchet (`bd show sq-mcb3q`): it drives the REAL `ContinuousQuery` / `ContinuousMultiQuery` pipeline across the SRBench expressivity axes (window types · RSTREAM/ISTREAM/DSTREAM · all four `EvalMode`s · multi-window joins including 3+-window joins and ISTREAM/DSTREAM over multi-window joins) and asserts every closed window against an INDEPENDENT batch-rebuild + closed-form oracle, with a `RSP_EXPRESSIVITY_FLOOR` count-of-assertions ratchet (may only rise). It surfaces in the central conformance scoreboard (`sparq-conformance` `scoreboard::SUITES`) as a `family = sparq extension` row, tallied SEPARATELY from the standards-conformance total, with the documented RSP-QL gaps (window variables, textual `ROWS` windows, relative `NOW` bounds) asserted genuinely-rejected — never faked as passes. The deterministic bench gate (`bench/rsp/`) is the trend/throughput companion; the scoreboard row is the correctness ratchet. [SONNET-4.6] sq-2n1q3.3

[FABLE-5] sq-hmd7l.20 **Bounded external-engine comparison**: RSP peers (C-SPARQL / CQELS / RSP4J) run wall-clock service windows, so a RAW throughput head-to-head stays out of scope; the adopted bounded protocol (`research/comparative-benchmarking-everything.md` §5.2) drives RSP4J/YASPER in its event-time configuration with the IDENTICAL pinned `(triple, ts)` replay (`bench/rsp/replay/*.ts.tsv`), requires per-window result-count agreement with the deterministic oracle FIRST (`bench/rsp/rsp4j_compare.py` — a failed gate admits ZERO timing rows), and machine-attaches a time-model caveat to every emitted comparison row. Count-comparable surface + first-read verdict: `research/gap-rsp-2026-07.md` (only `srbench_join` is count-comparable in YASPER's TP dialect today; the aggregate scenarios are excluded and reported, and sustained throughput is NOT-MEASURED pending a matched-workload scaled replay).

## See also

- `hdt-format`, `fused-decompress-parse`, `rust-parallel-parsing` — bulk RDF ingest that can feed a stream.
- `sparql-formal-semantics` — the SPARQL algebra the embedded queries are evaluated under.
- The ZK/MPC sibling skills (`noir-circuit-patterns`, `mpc-protocols`, `verifiable-credentials-zk`) cover separate sparq crates; RSP is independent of them.
