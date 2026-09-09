# Browser WASM performance assessment — layer-attributed, measurement-first (2026-07)

Status: **diagnostic assessment + program decomposition, July 2026.** [FABLE-5]
Maintainer-commissioned (front 3 of 3): consider sparq's performance in common
browsers and — *if necessary* — prepare concrete upstream contributions to
browser-engine WASM evaluation. This record characterises the browser build as
it actually ships, attributes each cost to its layer (sparq-side / toolchain /
browser engine / deployment), and decomposes the work into the disjoint child
beads of epic `sq-3ul2n`. Companion to
`research/zk-browser-perf-assessment.md`, which covers the **ZK prover** side
of the browser story (`@aztec/bb.js`, a separate wasm artifact with its own
constraints); this record covers the **engine** side (`sparq-wasm` and its four
tier-b siblings). Where the ZK estate is mentioned it carries the standing
caveat: the sparq ZK/MPC estate is research-grade and **not externally
audited** (external cryptographer sign-off pending, bead `sq-qhy4`).

## 0. Honesty boundary (read first)

- **No real-browser measurement of the sparq engine wasm exists anywhere in the
  repo, and none was run for this assessment.** This work box has
  Playwright-Chromium only (no Firefox, no WebKit browser bundles cached, no
  `wasmtime`/`wasmer`, no `wasm-opt` on PATH), so honest *cross-engine* numbers
  cannot be produced here today. Rather than fabricate browser numbers, the
  measurement itself is the program's first bead (`sq-3ul2n.1`) — the same
  pattern as the credential-blocked nlq-endpoint measurement.
- Every number cited below is a **prior measured record with provenance**
  (`research/hardware/consumer-and-targets.md` §5 spike; the advisory Node
  scripts under `crates/sparq-wasm/test/`), and all of it is **non-canonical**
  work-box data — indicative, never to be baked into docs, tests, or gates.
- Everything in §1–§2 was **verified against the checked-out code and CI
  scripts** during this assessment (file paths inline), not taken from the
  commissioning brief — which three findings below correct.

## 1. What actually ships (verified inventory)

Five wasm crates exist. `sparq-wasm` is the main store+engine bundle published
as `@sparq-org/sparq` (built by `js/package.json` `build:wasm{,:lean}`:
`wasm-pack build --target web --profile release-wasm`, features
`shacl,jsonld,serialize-rdf,scs,canon` on the published build, none on the lean
one). `sparq-reason-wasm`, `sparq-rsp-wasm`, `sparq-shacl-wasm`,
`sparq-text-wasm` are tier-b bundles lazy-loaded per surface page. Verified
facts that drive the analysis:

| Axis | Shipped state (verified) | Where |
|---|---|---|
| Cargo profile | `release-wasm` inherits `release` (**opt-level 3, fat LTO, codegen-units 1, panic=abort**) + `strip=symbols`; only the cold run-once parse crates (`spargebra`, `oxiri`, `oxttl`) are `opt-level="z"`. Blanket `-Oz` was considered and **rejected** — it would deopt the eval kernels. | root `Cargo.toml:108–127` |
| wasm-opt | The `[package.metadata.wasm-pack.profile.release] wasm-opt=["-Oz"]` blocks in all five wasm crates are **dead config** under `--profile release-wasm` (wasm-pack only recognises built-in profile names); the shipped artifact silently gets wasm-pack's default `-O`. | `crates/sparq-wasm/Cargo.toml` trailing comment block |
| SIMD | `-C target-feature=+simd128` is set **only** in `crates/sparq-wasm/.cargo/config.toml` — cwd-discovered, so the shipped wasm-pack build has it but the root-invoked `wasm_bundle_bytes` gate build and all four sibling crates do **not** (§2, gap B). | `crates/sparq-wasm/.cargo/config.toml`, root `.cargo/config.toml`, `scripts/ci-bench.sh` |
| Threads | None. `default-features = false` drops rayon; there is no wasm-threads build. Independently, the deploy target (GitHub Pages) cannot set COOP/COEP, so `SharedArrayBuffer` is unavailable there anyway (measured for the ZK prover: threads were the dominant ~4x lever, `research/zk-browser-perf-assessment.md` Rank 1). | `crates/sparq-wasm/Cargo.toml:17`, ZK record |
| Index | 3-permutation compact index (SPO/POS/OSP) auto-selected on wasm32 (~half the index memory of the native 6-perm build; some query shapes pay for the missing permutations). | `crates/sparq-core/src/store.rs:37` |
| Allocator / memory | rustc's default dlmalloc; **no** `#[global_allocator]`, **no** initial-memory / max-memory / stack-size link args anywhere; default `memory.grow` on demand. | workspace grep; both `.cargo/config.toml` files |
| Ingest | Whole-buffer strings only: `load(text: &str, format)` and friends. No bytes (`Uint8Array`) entry point, no streaming/chunked ingest. Serial parse: sparq's custom `nt::parse_chunk` for N-Triples, `oxttl` for Turtle/TriG/N-Quads, `oxjsonld` behind `jsonld`. | `crates/sparq-wasm/src/lib.rs`, `crates/sparq-core/src/lib.rs:889–957` |
| Query results | SELECT → one SPARQL-1.1-JSON `String`; CONSTRUCT/DESCRIBE → one N-Triples `String`; ASK → `bool`; `count` → `usize`. The cursor types (`QueryChunks`, `SolutionCursor`, `QuadChunks`) **materialise the full result inside wasm first** (`std::vec::IntoIter<String>`) and only chunk the JS hand-off. | `crates/sparq-wasm/src/lib.rs` |
| JS↔WASM boundary | Every string arg is copied (JS UTF-16 → UTF-8 into linear memory); every returned `String` is copied out + `TextDecoder`-decoded. No zero-copy or shared-buffer path. Store/cursor handles are opaque pointers (cheap). | wasm-bindgen ABI + signatures in `lib.rs` |
| JS wrapper | `queryBindings()` = `JSON.parse` of the whole JSON string + one `Bindings` Map per row + one RDF/JS `Term` per cell; `queryBindingsStream()` bounds JS peak memory via `queryChunks` + a JS brace-scanner but the wasm side still materialises everything; `fromQuads()` builds an N-Quads **string in JS**, copies it in, and re-parses it in wasm. `queryBoolean()`/`count` avoid all string marshalling. | `js/src/store.ts:291–403`, `js/src/sparql.ts:209–333` |
| Instantiation | `--target web` + explicit URL → `fetch` + `WebAssembly.instantiateStreaming` (compile overlapped with download — already the right pattern). | `js/packages/sparq-client/src/index.ts:193–198` |
| Write path | `update_in_place`/`apply_delta` grow the dictionary + delta overlay monotonically; deletes are masked, not reclaimed (documented constraint). | `crates/sparq-wasm/src/lib.rs:53–58` |
| Measurement estate | Advisory Node-only scripts (`test/perf.cjs`, `test/mem.cjs`, `test/bench.cjs`, `js/bench/vs-oxigraph.mjs`), all explicitly non-canonical; CI runs `wasm-pack test --node` (functional) and hard-ratchets `wasm_bundle_bytes` (raw cargo artifact, 2% band) + trend-only `wasm_opt_bundle_bytes`. **No browser lane measures anything.** | `ci.yml:712–776`, `scripts/ci-bench.sh:961–993` |

## 2. Corrected premises (vs the commissioning brief)

1. **"Enable simd128 and measure" is already done — and already measured
   ~speed-neutral.** `+simd128` has been on for `sparq-wasm` since the
   consumer-targets spike, which found **no meaningful speedup** (best ~6% on a
   pure scan, everything else inside run-to-run noise) because the engine
   contains **zero hand-written SIMD kernels** — the only reliable win was
   ~-3.6% raw bundle bytes (805→777 KB; measured, non-canonical,
   `research/hardware/consumer-and-targets.md` §5). SIMD pays only after the
   explicit `core::simd` kernels (M4 program) exist. What *is* real: the
   **parity gap** — the flag lives in a cwd-discovered crate-level
   `.cargo/config.toml`, so the hard-gated `wasm_bundle_bytes` artifact (built
   from the repo root) and all four sibling bundles build **without** simd128
   while the shipped main bundle has it. The gate is ratcheting a different
   artifact than the one users run. Fix + guard = `sq-3ul2n.2`.
2. **The shipped build is not size-crippled at the cargo layer.** The brief's
   implicit "is it -Oz?" concern is answered: hot crates are opt-level 3 + fat
   LTO; only cold parse crates are size-optimised, deliberately. The actual
   toolchain accident is at the **wasm-opt** layer: dead `-Oz` metadata blocks
   and a silent `-O` default nobody chose (`sq-3ul2n.7`).
3. **An in-browser latency-harness bead already existed** (`sq-hmd7l.17`,
   bench registry suite `wasm-compare`). It is extended, not duplicated: the
   new harness bead `sq-3ul2n.1` delivers the browser-latency half and now
   blocks `sq-hmd7l.17`, which keeps the oxigraph-npm/n3js comparison on top.

## 3. Layer-attributed bottleneck table

Phases of a browser session, each attributed to the layer that owns the cost.
Confidence is *structural* (verified from code/config, no browser timing yet)
unless a prior measurement is cited. This table is the honest core of the
assessment: **every identified bottleneck so far attributes to sparq-side
design/config or deployment — none attributes to a browser engine.**

| # | Phase / hot path | What costs (verified mechanism) | Layer | Confidence | Lever (bead) |
|---|---|---|---|---|---|
| 1 | Download + instantiate | ~210 KB brotli main bundle (dated prior figure); `instantiateStreaming` already overlaps compile with download. Tier-b bundles lazy-load per page. | sparq-side (size) + toolchain | verified config | Explicit, measured wasm-opt flags; size-first for lazy tier-b bundles (`sq-3ul2n.7`) |
| 2 | JIT tier-up warmup | All three engines tier (V8 Liftoff→TurboFan, SpiderMonkey baseline→Ion, JSC BBQ→OMG): first queries run baseline-compiled code. Bundle is small, so warmup is bounded — but unmeasured. | browser engine (inherent) | structural; needs measurement | Quantify warm-vs-cold in harness (`sq-3ul2n.1`); not actionable upstream |
| 3 | Ingest: boundary | Whole document as a JS string: `fetch().text()` materialises UTF-16, wasm-bindgen re-encodes UTF-8 + copies into linear memory. Two conversions + an extra full-document JS-heap copy before parsing starts. | sparq-side (binding design) | verified | Bytes ingest `loadBytes(Uint8Array)` (`sq-3ul2n.3`), wrapper adoption (`sq-3ul2n.5`) |
| 4 | Ingest: peak memory | Whole-buffer only: peak = document + store. On the binding tier (mid-range Android / iOS Safari, aggressive tab-kill) peak memory is the constraint, per the consumer-targets analysis. | sparq-side (binding design) | verified | Streaming chunked ingest, O(chunk) buffering (`sq-3ul2n.4`) |
| 5 | Ingest: parse + dict build | Single-threaded serial parse (custom `nt::parse_chunk` / oxttl); allocation-heavy dictionary + 3-perm index build on default dlmalloc with untuned memory growth. | sparq-side (no threads; allocator default) + deployment (COOP/COEP blocks any future threads on Pages) | verified config; allocator impact unmeasured | Allocator + memory-tuning spike, evidence-gated (`sq-3ul2n.6`); threads deliberately out of scope (below) |
| 6 | Query eval (joins, FILTER) | Native-parity engine code at opt-level 3; no SIMD kernels yet (simd128 measured ~neutral); 3-perm index trades some query shapes for memory; wasm bounds checks are near-free on 64-bit hosts via engine guard pages. | sparq-side (kernel work = existing M4 program) | prior spike (non-canonical) | SIMD kernels tracked by the existing M4/consumer program, not re-beaded here; per-engine eval split measured by `sq-3ul2n.1` |
| 7 | Result marshalling | SELECT: serialize whole JSON in wasm → copy out → `JSON.parse` → per-row Map + per-cell Term allocation. Cursors don't reduce wasm-side materialisation. Triple-handling of every result byte. | sparq-side (binding + wrapper design) | verified | Measure share first (`sq-3ul2n.1`/`.5`); columnar/binary transfer only if evidence says it dominates (follow-up bead from data) |
| 8 | Steady-state writes | Monotonic dict/overlay growth; deletes masked → unbounded linear-memory growth in long-lived tabs. | sparq-side (documented constraint) | verified | `Store.compact()` (`sq-3ul2n.8`) |
| 9 | Engine-specific walls | **Unknown — no cross-engine data exists.** Known engine deltas that could surface: Safari's flagged relaxed-SIMD (unused by sparq), Chrome-only JSPI (unused), memory64's bounds-check cost (avoided — sparq stays wasm32), per-engine `memory.grow` behaviour. | browser engine | none yet — this is the gap | Cross-engine harness (`sq-3ul2n.1`); conditional upstream escalation (`sq-3ul2n.9`) |

**Deliberately out of scope: wasm threads.** Threads are the one lever with a
measured order-of-magnitude-class win on a neighbouring workload (~4x for the
ZK prover at 8 threads, Node, non-canonical). They are excluded because (a)
the deploy target cannot set COOP/COEP (GitHub Pages), so the shipped site
gains nothing without a service-worker shim or re-host — a deployment
decision, not an engine one; (b) a threaded engine build is a large
architectural change (rayon-on-wasm + shared-memory bundle variant), which
should be its own maintainer-visible program if the harness shows
parse/eval — not boundary/memory — dominating on multicore clients.

## 4. Browser-engine landscape (mid-2026) and why upstream work is premature

Per-engine wasm evaluation state relevant to sparq (public status pages +
vendor posts, July 2026; see Sources):

- **simd128 (fixed-width)**: universal baseline (Chrome 91+ / Firefox 89+ /
  Safari 16.4+). The shipped main bundle already requires it; extending it to
  the gate build + siblings changes no support floor (`sq-3ul2n.2`).
- **Relaxed SIMD**: default-on in Chrome + Firefox, still flag-gated in
  Safari. sparq emits no relaxed ops → irrelevant today; becomes interesting
  only when the M4 hand-written kernels exist, and then only with a
  Safari-safe fallback.
- **Threads / SharedArrayBuffer**: supported by all three engines but gated on
  cross-origin isolation — a **deployment** property sparq's Pages host cannot
  provide. The engine is not the wall; the host headers are.
- **memory64**: shipped in Chrome + Firefox, not universally in Safari; and on
  64-bit hosts wasm32 gets its bounds checks nearly free via reserved guard
  regions, a trick memory64 partially forfeits (explicit checks). sparq's
  compact-index wasm32 posture is the right call; **do not** chase memory64.
- **JSPI**: phase 4, live in Chrome, flagged in Firefox, no public Safari
  commitment. Would only matter for suspend-on-await streaming ingest; the
  chunked `StoreLoader` design (`sq-3ul2n.4`) needs no JSPI.
- **WasmGC / exception handling / tail calls**: universal now, but Rust
  targets linear memory and sparq compiles with `panic=abort` — neither is on
  any sparq hot path.

**Verdict on "if necessary" upstream contributions: not necessary on current
evidence.** Every bottleneck in §3 with verified attribution is sparq-side or
deployment-side. Filing a V8/SpiderMonkey/JSC bug or patch without a
measured, minimal, engine-attributed reproducer would be noise — such
contributions are high-effort/low-probability even when justified. The honest
path is the one encoded in the beads: build the cross-engine harness first
(`sq-3ul2n.1`), exhaust the sparq-side levers, and open the upstream lane
(`sq-3ul2n.9`) **only** if the same artifact on the same workload shows one
engine materially slower with profile evidence attributing it to that engine's
wasm evaluation. That bead's expected outcome is explicitly "no
engine-attributable wall found" — a negative result it must record rather
than force a contribution.

## 5. Tiered recommendation

- **Tier 0 — measure (the gate for everything else).** Cross-engine,
  per-phase browser harness + Node baseline, advisory envelopes only
  (`sq-3ul2n.1`, P1).
- **Tier 1 — sparq-side config + boundary (high ROI, low risk).**
  Target-feature unification + shipped/gate parity guard (`sq-3ul2n.2`);
  bytes ingest (`sq-3ul2n.3`); JS-wrapper adoption + wrapper-share measurement
  (`sq-3ul2n.5`); explicit measured wasm-opt flags (`sq-3ul2n.7`).
- **Tier 2 — sparq-side deeper (evidence-shaped).** Streaming chunked ingest
  (`sq-3ul2n.4`); allocator/memory spike, adopt-on-measurement
  (`sq-3ul2n.6`); `Store.compact()` for long-lived tabs (`sq-3ul2n.8`).
- **Tier 3 — upstream browser engines (last resort, conditional).**
  Reproducer + report/patch to the specific engine, only on harness evidence
  (`sq-3ul2n.9`, P4). SIMD *kernels* remain owned by the existing M4/consumer
  program and are not duplicated here.

## 6. Decomposition (epic `sq-3ul2n`)

Child beads are file-disjoint except the `sparq-wasm` chain, which is
dep-sequenced NON-parallel because its members share `src/lib.rs`/`Cargo.toml`
(`.3 → .4 → .6 → .8`). Edges: `.3→.4→.6→.8`, `.3→.5`, `.1→.9`, and
`.1→sq-hmd7l.17` (existing bead consumes the harness; note added there and on
`sq-5pnl`).

| Bead | P | Tier | Files (disjointness) | Invariant (load-bearing) |
|---|---|---|---|---|
| `sq-3ul2n.1` harness | 1 | sonnet | `bench/wasm-compare/browser/**`, `bench/CATALOG.md` | Measurement-only; non-canonical envelopes; skip-with-notice, never fabricate |
| `sq-3ul2n.2` target-features | 2 | sonnet | root `.cargo/config.toml`, delete crate config, `scripts/check-wasm-features.py`, `scripts/ci-bench.sh` | Result-equivalence; native byte-identical; bundle may shrink, never grow |
| `sq-3ul2n.3` bytes ingest | 2 | sonnet | `crates/sparq-wasm/{src/bytes.rs,src/lib.rs,Cargo.toml,tests/web.rs}` | `loadBytes(utf8(s))` ≡ `load(s)`; fail-closed on invalid UTF-8; lean bundle byte-identical feature-OFF |
| `sq-3ul2n.4` streaming ingest | 3 | opus | `crates/sparq-wasm/{src/stream.rs,src/lib.rs,Cargo.toml,tests/web.rs}` (chain) | Any chunking ≡ whole-buffer load; O(chunk) buffering; same error surface |
| `sq-3ul2n.5` JS wrapper | 2 | sonnet | `js/{src,test,bench}/**` + one `--features` line in `js/package.json` | RDF/JS API behaviour unchanged; bytes ≡ string path |
| `sq-3ul2n.6` allocator spike | 3 | sonnet | `crates/sparq-wasm/{src/alloc.rs,src/lib.rs,Cargo.toml}` (chain) | Default build byte-identical feature-OFF; adoption evidence-gated |
| `sq-3ul2n.7` wasm-opt flags | 3 | sonnet | `js/package.json` build scripts (+ sibling build scripts if trivially located) | Artifact validates + tests green; raw byte gate untouched; trade recorded honestly |
| `sq-3ul2n.8` compact() | 4 | sonnet | `crates/sparq-wasm/{src/compact.rs,src/lib.rs,Cargo.toml,tests/web.rs}` (chain tail) | Query-equivalence before/after; `heap_bytes` decreases after masked deletes |
| `sq-3ul2n.9` upstream (conditional) | 4 | opus | `bench/wasm-compare/browser/upstream/**` | No upstream engagement without measured reproducer; negative verdict is a valid close |

## 7. What was verified vs reasoned vs not measured

- **Verified against code/config during this assessment:** every row of §1
  (paths inline); the simd128 cwd-discovery parity gap (root-invoked gate
  build vs crate-dir wasm-pack build); the dead wasm-opt metadata blocks; the
  whole-buffer/string-only boundary; cursor full materialisation; JS-wrapper
  marshalling; the absence of any browser perf lane; the tool inventory on
  this box.
- **Prior measurements cited (non-canonical, provenance inline):** simd128
  ~speed-neutral / -3.6% bytes spike; ZK-prover thread scaling; bundle brotli
  size (dated).
- **Reasoned from public engine documentation (Sources):** the §4 per-engine
  feature matrix and the wasm32 guard-page bounds-check argument.
- **Not measured (and therefore beaded, not claimed):** everything a browser
  would tell us — per-engine phase timings, tier-up cost, marshalling share,
  allocator impact, wasm-opt flag deltas. No browser number appears in this
  record because none was produced.

Sources: [webassembly.org feature status](https://webassembly.org/features/),
[The State of WebAssembly 2025/2026 (Uno Platform)](https://platform.uno/blog/the-state-of-webassembly-2025-2026/),
[State of WebAssembly 2026 (devnewsletter)](https://devnewsletter.com/p/state-of-webassembly-2026/),
[WebAssembly 3.0 spec release overview (byteiota)](https://byteiota.com/webassembly-30-spec-release/),
[Chrome WebAssembly + WebGPU enhancements](https://developer.chrome.com/blog/io24-webassembly-webgpu-1).
