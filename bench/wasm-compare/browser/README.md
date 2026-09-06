# wasm-compare/browser — cross-engine browser measurement harness

<!-- [FABLE-5] sq-3ul2n.1 — the Tier-0 measurement gate of the browser-WASM
     performance program (research/browser-wasm-perf-assessment-2026-07.md,
     epic sq-3ul2n). Consumed by sq-hmd7l.17 (oxigraph-npm comparison). -->

Per-phase wall-time attribution for the **shipped `@sparq-org/sparq` web bundle**
(the `js/` `build:wasm` output) across **headless Chromium, Firefox and WebKit**
(Playwright) plus a **plain-Node baseline** — so a browser-side regression can
be attributed to a LAYER (download vs compile vs instantiate vs parse vs query
vs boundary marshalling) and to an ENGINE, not just observed as a slower total.

**Every number this harness emits is ADVISORY / NON-CANONICAL.** It measures
whatever host runs it (typically a shared, noisy work box). The deliverable is
the repeatable per-phase attribution and the *cross-engine ratio structure*;
absolute values must never be committed, baked into docs/tests, or gated on
(repo rule: no hard-coded performance numbers in markdown). The only
deterministic gate is the row/triple-count oracle: identical workload ⇒
identical counts on every engine, and the run FAILS on divergence.

## Run matrix

| target | what runs | how |
|---|---|---|
| `node` | same wasm artifact + workload, no browser | child `node-baseline.mjs`; init decomposed as disk-read/compile/instantiate (`fetch_wasm` is a disk read — a floor, noted in-row); `init_streaming_total` skip-with-notice (no `file:` fetch in Node) |
| `chromium` / `firefox` / `webkit` | two pages per engine, each in a **fresh browser process** (wasm compile caches are per-process — sharing one process would let the second page's compile hit a warm cache and mis-attribute) | Playwright headless; page A `?mode=streaming` times the shipped `initWasm()` path (fetch + `instantiateStreaming`, compile overlapped with download); page B `?mode=main` times the DECOMPOSED init then the full workload |

A browser that cannot launch on the host **skips with an explicit notice**
(an envelope with `skipped: true` + the install command) — never fabricated
numbers, and never a hard failure (exit stays 0 for skips).

## Phases emitted (per engine)

- `init_streaming_total` — the real user path: bare `initWasm()` ⇒ `fetch` +
  `WebAssembly.instantiateStreaming` (browser pages only).
- `fetch_wasm` / `compile_wasm` / `instantiate_wasm` — the same path
  decomposed (fresh process, cold compile).
- `store_load` × {`ntriples`, `turtle`} × {25k, 100k, 300k triples},
  `kind: first|warm` — parse + dictionary/index build. `first` includes
  parser JIT tier-up (fixed phase order, documented below); `heap_bytes` and
  `source_bytes` recorded per row.
- `query` × {`scan-type`, `star-3`, `chain-2`, `triangle-wcoj`, `filter-age`}
  (the five shapes from `crates/sparq-wasm/test/bench.cjs`), `kind: first`
  (tier-up-inclusive cold) vs `warm` (best-of-K, batched for coarse timers).
- `serialize_construct` — CONSTRUCT → N-Triples string out of wasm.
- `marshal_*` — the boundary-marshalling probe set on the same pattern:
  `ask` (floor; short-circuits) → `count` (lazy-count floor) →
  `query_string` (wasm eval + JSON serialise + ONE string copy out) →
  `query_json_parse` (+ JS `JSON.parse`) → `query_chunks` (chunked drain) →
  `wrapper_bindings` (JS-wrapper `queryBindings`: + one `Bindings` Map/row,
  one Term/cell). Adjacent deltas attribute copy vs parse vs materialisation;
  `ask`/`count` are floors, **not** same-eval controls (they under-run eval).

Workload determinism: generators are read-only ports of
`crates/sparq-wasm/test/{bench,mem}.cjs` (source cited in `workload.mjs`),
sized by exact TRIPLE count; expected row counts are computed per shape and
asserted on every engine. Query phases run on the 100k-triple Turtle store
(25k in `--quick`). Fixed phase order (N-Triples loads ascending, then
Turtle, then queries) keeps `first` rows comparable across engines.

## Running it

```sh
# once: deps + browsers (scoped to THIS dir — not a repo-root workspace member)
cd bench/wasm-compare/browser
npm ci
npx playwright install chromium firefox webkit   # + `sudo npx playwright install-deps <engine>` for OS libs

# the shipped bundle + wrapper must exist (or pass --build):
#   repo root: npm ci --ignore-scripts;  js/: npm run build:wasm && npx tsc

node run.mjs --engine chromium          # one engine
node run.mjs                            # all: node + chromium + firefox + webkit
node run.mjs --engine webkit --quick    # smoke tier (25k triples, few iters)
```

Envelopes land in `results/<engine>-<UTC>.json` (git-ignored; measurement
output is scratch): `{suite, bead, advisory, note, engine, engine_version,
git_commit, host, bundle:{bytes,sha256}, rows:[{phase, format?, triples?,
query?, rows?, kind, ms, …}], skipped_phases}` — one envelope per engine, all
sharing the same bundle sha so runs are provably apples-to-apples. A summary
table + per-phase ratios vs the Node baseline print at the end. Exit codes:
`0` ran/skipped-with-notice, `1` run error or cross-engine row-count
divergence, `2` usage/missing artifacts.

Interpretation notes: browser `fetch_wasm` is a loopback-HTTP number (server
prewarmed, explicit `Content-Length`, `cache-control: no-store`) — it
reflects engine network-stack + streaming behaviour, not real-world CDN
latency. Timings on 1ms-clamped engines are recovered by batching
(`measure()` in `workload.mjs`); `first` rows are single observations by
nature. There is deliberately **no CI lane**: this is a measurement harness,
run on demand (the timing benches were moved off the PR path in #1895 for the
same reason).

## Cross-LIBRARY comparison layer (sq-hmd7l.17)

`compare.mjs` reuses this harness's workload module + static server to
compare sparq against the pinned competitor stacks from
`bench/competitors.json` — `oxigraph` (npm WASM, node + web builds) and
`n3js-quadstore` (N3.js + quadstore + quadstore-comunica; Node runtime only)
— on a reduced workload (text→store load + the five query shapes), one
library per fresh process/browser:

```sh
# gather-only competitor installs (never committed; exact pins in compare-workload.mjs):
npm install --no-save oxigraph@0.5.9 n3@2.1.1 quadstore@15.4.1 quadstore-comunica@6.3.1 memory-level@3.1.0

node compare.mjs                       # node runtime, all libraries
node compare.mjs --runtime chromium    # in-browser (headless Chrome)
node compare.mjs --runtime all --quick # smoke tier
```

Same envelope/exit discipline as `run.mjs`, plus the bead invariant: **no
latency row without row-count agreement** — per-library oracle checks before
every timing row and a cross-library/runtime agreement re-check at report
time. Missing packages and browser-infeasible columns skip WITH NOTICE. The
deterministic bundle-bytes stage lives one directory up
(`bench/wasm-compare/run.sh --bundle-only`, see `../README.md`).
