<!-- [FABLE-5] sq-hmd7l.17 first-read gap record. The bundle-bytes section is
CANONICAL (deterministic — pinned immutable npm artifact vs the shipped bundle
at a recorded commit). ALL latency sections are NON-CANONICAL first-read rows
from the shared work box; canonical latency rows come from sq-hmd7l.39. -->

# Browser/WASM competitor gap — first-read record (2026-07-11)

**Status:** deterministic bundle-bytes comparison CANONICAL (stage a);
latency comparison implemented + first-read ADVISORY rows gathered (stage b),
canonical quiet-box wave deferred to `sq-hmd7l.39`.
**Bead:** sq-hmd7l.17 (consumes the sq-3ul2n.1 harness). **Epic:** sq-hmd7l.
**Harness:** `bench/wasm-compare/` — `run.sh --bundle-only` (deterministic),
`browser/compare.mjs` (latency; per-query expected-row-count oracle +
cross-library agreement gate EVERY timing row — no row without agreement).

## Pins (verified at first gather, recorded in `bench/competitors.json`)

- **oxigraph** 0.5.9 (npm, UNSCOPED — the registry stub's `@oxigraph/oxigraph`
  name does not exist on npm; corrected). Tarball sha256
  `f1f449efd747a7355840bc78bddfda10d1a859d22949153eee6a37c814bbbb99`.
- **n3js-quadstore** = n3 2.1.1 + quadstore 15.4.1 + quadstore-comunica 6.3.1
  + memory-level 3.1.0 (in-memory backend — the stack is not charged disk I/O).
- **sparq** = shipped `@sparq-org/sparq` web bundle (`js/ build:wasm`, full feature
  set `shacl,jsonld,serialize-rdf,scs,canon`), commit `beb0429d2`,
  wasm-pack 0.15.0. Not yet published to npm → the pin is commit + toolchain.

## CANONICAL — deterministic bundle bytes (stage a)

`bash bench/wasm-compare/run.sh --bundle-only`, 2026-07-11:

| engine | artifact | bytes | gzip-9 (informative) |
|---|---|---:|---:|
| sparq | web wasm `sparq_wasm_bg.wasm` | 3 206 816 | 1 166 842 |
| sparq | js glue `sparq_wasm.js` | 54 010 | 13 792 |
| oxigraph 0.5.9 | web wasm `web_bg.wasm` | 4 066 540 | 1 447 943 |
| oxigraph 0.5.9 | js glue `web.js` | 49 391 | 7 350 |
| oxigraph 0.5.9 | node wasm `node_bg.wasm` | 4 069 655 | 1 449 224 |

Web `.wasm` ratio sparq/oxigraph = **0.789** — the sparq bundle is ~21%
smaller **while carrying its full published feature set** (SHACL, JSON-LD,
serializers, SCS, canon). Caveat: shipped-artifact vs shipped-artifact, not an
equalized feature matrix; a `build:wasm:lean` column is deferred to
`sq-hmd7l.39`. The raw bytes are deterministic (immutable registry artifact /
recorded commit+toolchain); gzip-9 is Node-zlib and informative only.

## NON-CANONICAL first-read latency (shared work box — do NOT cite)

Same deterministic workload for every column (sq-3ul2n.1 generators + exact
row-count formulas); every row below passed the oracle + cross-library
agreement. Node = one fresh child process per library; chromium = one fresh
headless browser per library (Playwright).

### Node, 100k-triple tier (sparq vs oxigraph), ratio = competitor/sparq

| phase | vs oxigraph (first) | vs oxigraph (warm) |
|---|---:|---:|
| store_load ntriples | ×2.48 | ×6.84 |
| store_load turtle | ×1.34 | **×1.08 (near-parity)** |
| query scan-type | ×1.89 | ×2.75 |
| query star-3 | ×7.92 | ×20.3 |
| query chain-2 | ×22.1 | ×86.8 |
| query triangle-wcoj | ×2.33 | ×6.93 |
| query filter-age | ×78.3 | ×245 |

### Node, 25k quick tier (all three columns), warm ratios vs sparq

oxigraph ×4.1–×46.8 on queries (×5.0 NT load, ×1.08 Turtle load);
n3js-quadstore ×12.7–×1988 on queries, ×44.7–×117.8 on loads — the pure-JS
stack is 1–3 orders of magnitude behind on every axis.

### Chromium (headless), 25k quick tier, sparq vs oxigraph

Queries warm ×12.4–×28.6, loads ×1.1–×1.9 in sparq's favour; `first` rows are
single cold observations (noisy).

## Dominance verdicts (per the performance-dominance mandate)

| axis | verdict | action |
|---|---|---|
| bundle bytes (canonical) | AHEAD (0.79×, more features) — size is a parity-class metric, OOM not structurally meaningful | lean column: `sq-hmd7l.39` |
| query latency vs oxigraph-wasm | CLEARLY-AHEAD (advisory ×1.9–×245) | confirm canonical: `sq-hmd7l.39` |
| query latency vs n3js-quadstore | CLEARLY-AHEAD (advisory 1–3 orders) | confirm canonical: `sq-hmd7l.39` |
| N-Triples load | CLEARLY-AHEAD (advisory ×2.5–×6.8) | — |
| **Turtle load vs oxigraph-wasm** | **NEAR-PARITY (advisory ×1.08 warm)** — wasm is single-threaded, no chunk-parallel path; the known single-thread Turtle axis (cf. `gap-parse-2026-07.md`, sq-wrn61) | gap bead `sq-3ul2n.11` |

Follow-ups: `sq-hmd7l.39` (canonical quiet-box wave + lean bundle column),
`sq-3ul2n.11` (wasm Turtle-load profile), `sq-hmd7l.40` (optional SP2B/WatDiv
browser corpora — the shared-workload choice follows the bead NOTES:
one harness, exact-formula oracles).
