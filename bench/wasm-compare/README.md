# wasm-compare — sparq WASM vs the JS/WASM RDF ecosystem

<!-- [FABLE-5] sq-hmd7l.17 — competitor half of the wasm-compare suite,
     layered on the sq-3ul2n.1 browser harness in ./browser/. -->

Two stages, one entry point:

```sh
bash run.sh --bundle-only    # (a) DETERMINISTIC bundle bytes: sparq vs oxigraph npm
bash run.sh                  # (a) + (b) Node-runtime latency comparison
bash run.sh --browser        # (b) also in-browser via headless Chromium
bash run.sh --quick          # smoke tier for the latency stage
```

**(a) Bundle bytes — the deterministic, canonical-capable metric** (`bundle.mjs`).
Compares the shipped `@sparq-org/sparq` web bundle (`js/ build:wasm` output;
not yet on npm, so the pin is the repo commit + recorded wasm-pack version)
against `npm pack oxigraph@<pin>` — an immutable, integrity-checked registry
artifact. Raw `.wasm`/glue byte counts are exactly reproducible on any box
(no quiet-box requirement); gzip-9 wire bytes are emitted as informative only.
Caveat: shipped-artifact vs shipped-artifact — each engine's default published
feature set, not an equalized feature matrix. The pre-bindgen sparq artifact
is separately ratcheted by `scripts/ci-bench.sh` `wasm_bundle_bytes`
(deliberately untouched by this suite).

**(b) Cross-library latency — advisory, oracle-gated** (`browser/compare.mjs`).
Layers competitor columns onto the sq-3ul2n.1 harness workload (same
generators, query shapes, and expected-row-count formulas from
`browser/workload.mjs` — see [`browser/README.md`](./browser/README.md)):

| column | stack | runtimes |
|---|---|---|
| `sparq` | shipped `@sparq-org/sparq` web bundle | node, chromium |
| `oxigraph` | official `oxigraph` npm WASM (node + web builds) | node, chromium |
| `n3js-quadstore` | N3.js parse + quadstore(memory-level) + quadstore-comunica SPARQL | node only (browser needs a bundler — skipped WITH NOTICE) |

Node columns run one library per fresh child process; chromium columns run
one library per fresh headless browser. **INVARIANT:** no latency row without
row-count agreement — every query is checked against the deterministic
expected count before its timing row is emitted, and the orchestrator
re-checks counts across libraries/runtimes (divergence → exit 1). Missing
competitor packages skip WITH NOTICE (exit 0), never fabricate.

Competitor packages are **gather-only installs** (never committed) — the
pinned command lives in `browser/compare-workload.mjs` (`INSTALL_HINT`) and
`bench/competitors.json`; run it inside `browser/` with `--no-save`.

Every latency number is ADVISORY / NON-CANONICAL (repo rule: no hard-coded
performance numbers); envelopes land in git-ignored `results/` dirs.
First-read gap record: `research/gap-wasm-2026-07.md`.
