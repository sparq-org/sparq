<!-- sq-inzv: full-template README — tier-b W-rsp WASM showcase bundle. -->
# sparq-rsp-wasm

The sparq windowed **RSP-QL stream processor** ([`sparq-rsp`](../sparq-rsp/README.md))
compiled to WebAssembly — the tier-b **"W-rsp"** bundle (sq-nzcb).

A SEPARATE, lazy-loaded bundle from the lean [`sparq-wasm`](../sparq-wasm/README.md)
triplestore bundle (the [`sparq-reason-wasm`](../sparq-reason-wasm/README.md)
pattern): it carries windowed continuous SPARQL — sliding/tumbling time windows with
RSTREAM / ISTREAM / DSTREAM output — which the lean default browser bundle deliberately
does not. The showcase site loads it ONLY on `/surface/streaming-rsp`
(`next/dynamic`, client-only) so the landing page stays light.

> Not on crates.io (`publish = false`) and not a separate npm package: the site builds
> it with `js/`'s `build:rsp-wasm` and serves it. It is a wasm packaging
> layer over `sparq-rsp`, built via `wasm-pack`, not a Rust library dependency.

## 🚀 Quickstart

```sh
wasm-pack build crates/sparq-rsp-wasm --target web
```

```js
import init, { Rsp } from "./sparq_rsp_wasm.js";
await init();

// AVG(?v) over tumbling 60-tick windows (RANGE 60 STEP 60), full result per window.
const q = Rsp.select(
  "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/reading> ?v }",
  60,        // range  (logical time unit, same scale as the pushed ts)
  60,        // step   (== range => tumbling; < range => sliding)
  0,         // maxDelay (out-of-order tolerance)
  "rstream", // "rstream" | "istream" | "dstream"
);

// Push timestamped readings; each push returns the windows that just CLOSED, as JSON.
q.push("<http://ex/s1>", "<http://ex/reading>", "10", 0);   // -> "[]" (nothing closed)
const closed = JSON.parse(
  q.push("<http://ex/s1>", "<http://ex/reading>", "5", 65),  // ts 65 closes [0,60)
);
// closed[0].results is a standard, self-contained SPARQL 1.1 JSON document.
const tail = JSON.parse(q.flush()); // end-of-stream: close the remaining open window(s)
```

## ✨ Features

- **The UI drives the logical clock.** `sparq-rsp` is "wasm-safe" by construction —
  **it reads no wall clock and runs no async runtime**. Time advances *only* through
  application-supplied timestamps: each pushed reading carries a `ts`, a window closes
  when the watermark (`max_ts − max_delay`) reaches its end, and the whole pipeline is
  a pure, replayable function of the pushed `(triple, ts)` sequence. So the browser tab
  IS the clock; nothing in the bundle advances it. CI confirms the claim — it builds for
  `wasm32-unknown-unknown` and the headless suite drives the real exports in a genuine
  wasm runtime.
- **API surface** — `Rsp.select(sparql, range, step, maxDelay, r2s)` registers a
  continuous SELECT (validated *now* — a malformed or non-SELECT query throws here,
  not at first push). Then:

  | method | returns | role |
  |---|---|---|
  | `push(s, p, o, ts)` | JSON array of closed-window `{start,end,results}` objects | feeds one timestamped triple (terms are Turtle syntax, incl. the numeric shorthand `10` / `10.5`) |
  | `flush()` | same JSON array | end-of-stream: close every window up to the last `ts` seen (ignoring `maxDelay`) |
  | `lateDropped()` | `number` | arrivals dropped as too late (every covering window already closed) |
  | `sparql()` | `string` | the registered query text (echo, for the UI) |

  Each closed window is `{"start":S,"end":E,"results":<SPARQL-1.1-JSON>}`: half-open
  `[S,E)` bounds plus the R2S-filtered SELECT table in standard SPARQL 1.1 JSON.
- **Numeric args are plain JS `number`s.** `range` / `step` / `maxDelay` / `ts` (and the
  `lateDropped()` return) are JS `number`s, *not* `BigInt`s — pass `60`, not `60n`. Each is
  a whole logical-time value in `[0, 2^53-1]` (`Number.MAX_SAFE_INTEGER`, the exact-integer
  range of a `number`); a fractional / negative / out-of-range value is a clean error, not a
  thrown coercion. (They map to the native crate's `u64` ticks; the boundary takes `number`
  so the demo can pass ordinary JS numbers.)
- **Thin wrapper, native semantics.** Window semantics — boundary inclusivity,
  sliding overlap, lateness, empty-window reporting, R2S multiset diffs — are exactly
  [`sparq-rsp`'s](../sparq-rsp/README.md), pinned by that crate's tests; this bundle
  is a zero-`unsafe` JS wrapper that owns no parsing or serialisation of its own
  (triples via the engine's Turtle path; results via its SPARQL-JSON serialiser).
  It exposes the single-window `ContinuousQuery` SELECT form; CONSTRUCT/ASK and
  multi-window joins (`ContinuousMultiQuery`) stay in the native crate for now. The
  wasm-deps guard keeps the native-only heavy deps (rayon / compression codecs /
  `sparq-parse`) out of its graph.

## 📚 Learn more

- **How-to** — [`skills/streaming-rsp/SKILL.md`](../../skills/streaming-rsp/SKILL.md).
- **Test** — `cargo test -p sparq-rsp-wasm` (native helpers) + `wasm-pack test --node`
  (the real `#[wasm_bindgen]` API), exactly like the other wasm bundles.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
