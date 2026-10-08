---
name: fused-decompress-parse
description: How to ingest compressed RDF (gzip / zstd / bzip2) fast in Rust by FUSING decompression with the parallel parse instead of decompress-to-memory-then-parse, and which codec to choose. Use when working on sparq-core's compressed ingest (load_reader_parallel, build_external_ntriples_parallel, sparq-cli's open_reader), choosing gzip vs zstd vs bzip2, deciding whether to build a parallel decompressor, or designing the client decode matrix (Solid/JS). Grounded in this project's MEASURED ~13× streaming-vs-ideal finding.
---

# Fused decompress + parse for RDF ingest

Authored from measured research. Source of truth:
`research/custom-parsers-baseline.md` (the "Compressed ingest" section + the
post-fix measurements) and `research/fast-ingestion.md`. Verify before quoting.

## The headline finding (and the bug it exposed)

A naïve **streaming** ingest (decoder → parser, one parse round per `read()`)
was measured **3.5–5× SLOWER than just decompressing to RAM first** — the
opposite of what streaming should cost. Two compounding causes, both fixed by
reusing code that already existed:

1. **Per-`read()` flush bug.** `Graph::load_reader_parallel` flushed a
   parse+sharded-merge round on *every* `read()` call, and decompressors return
   small reads (gzip ~0.38 MB/read, zstd ~1.6 MB/read) into its 32 MiB buffer.
   The parallel parser+merge machinery is amortised for ~32 MiB blocks, not
   0.4 MB ones. **Fix: a producer thread that fills the full block across
   `read()` calls** before handing it to the parser.
2. **No pipelining.** Decode and parse ran additively. **Fix: run decode on its
   own thread feeding a bounded channel**, so it overlaps the rayon parse + dict
   merge — the exact 3-stage pipeline that already existed in
   `build_external_ntriples_parallel` (sparq-core `lib.rs`) and just wasn't used
   by the in-memory path.

After both fixes (same machine/harness), streaming ingest of the slice dropped by
~8–9× (gzip) and ~7× (zstd) and now **matches or beats two-stage in every paired
run** while never materialising the decompressed copy. (Absolute figures drift with
thermal state / machine — they live in `research/custom-parsers-baseline.md` and the
`bench/parse/` harness; cite that, not baked numbers. The load-bearing fact is the
ratio + the "matches-or-beats two-stage" conclusion.)

## The fusion bound — what "ideal" means

Ideal pipelined ingest = `max(decode, parse+build)`, because a perfect pipeline
hides the smaller stage under the larger:

- gzip: `max(0.396, 0.296) ≈ 0.40 s`
- zstd: `max(0.140, 0.296) ≈ 0.30 s`

vs the old streaming 5.59 / 3.95 s = **~13–14× available** and two-stage
1.12 / 1.14 s = 2.8–3.8×. The lesson: **most of the cost was plumbing, not
parsing.** A cooler run landed zstd streaming at 0.359 s — inside the ideal band.
Note the residual ~1.3–1.5× gap over ideal is the parts that *cannot* overlap
the decode: the final `finish_sharded` remap + the 6-permutation sort after the
last block. Thermal state on a fanless M1 moves every absolute number; the
*ratio* (streaming ≥ two-stage) was stable across all runs.

## Codec choice — measured, not assumed

- **zstd beats gzip on both axes.** On the real slice zstd −3 has a slightly
  better ratio than gzip −6 *and* decodes several × faster (figures:
  `research/custom-parsers-baseline.md`). Prefer zstd for sparq-controlled paths.
- **bzip2 is the real enemy.** The actual Wikidata "truthy" dump is `.bz2`
  (42.8 GB → 1.08 TB, ~25× ratio). Single-stream `bzcat` decodes ~1 M triples/s
  → over two hours for the full file regardless of downstream speed; it was the
  dominant share of E2E wall-time. **Verdict (stands): recompress `.bz2` → `.zst` once**
  (`zstd -9 -T0`, parallel, one-time) rather than building a parallel bzip2
  decoder. After that, ingest is parse/sort-bound, projected ~30 min for 9.4 B
  triples — RDFox-competitive (their 24 min).

## When parallel DECOMPRESSION is (and isn't) worth it

**Usually not.** Single-thread zstd decode (1,236 MB/s) already **outruns the
full 8-thread parse+build (585 MB/s)**, and gzip decode (438 MB/s) is roughly at
parity. So block-parallel decompression (multi-frame zstd / multi-member gzip /
`lbzip2`-style block-parallel bzip2) buys nothing once decode is hidden under
parse by the pipeline above. It matters **only** for bzip2 sources — and even
there the recompress-to-zstd verdict wins over a parallel bzip2 decoder. Don't
build a parallel decompressor before proving decode is the binding stage.

## Implementation notes that bit this codebase

- The producer thread requires `R: Read + Send`; `sparq-cli`'s `open_reader`
  returns `Box<dyn Read + Send>` for this reason.
- Two-stage carries ~0.43 s of pure materialisation overhead (allocating +
  faulting a fresh decompressed buffer and re-validating UTF-8) that a fused path
  handing newline-aligned blocks straight to the parser avoids.
- Regression cover is mandatory: short reads, mid-line read boundaries, EOF
  without trailing newline, empty input, parse-error propagation
  (`load_reader_parallel_short_reads_match_sequential`).
- gzip uses `flate2` `MultiGzDecoder` (multi-member aware); zstd via the `zstd`
  crate. Magic-byte sniffing (not file extension) is the project convention for
  detecting compression (see also `sparq-hdt` `.hdt.gz` detection).

## Client decode matrix (Solid / JS) — design note

zstd's value does **not** hinge on browser-native `Content-Encoding`: Solid apps
decode zstd in JS (fzstd / zstd-wasm), unlocking the full feature set including
**custom-vocabulary dictionaries**. Consumer matrix:

- Browser native: gzip (universal), br, zstd where shipped (Chrome 123+; verify
  Firefox/Safari current status).
- Browser JS-level (Solid apps): zstd via JS/WASM decoder — dictionaries usable.
- Server-to-server (prod-solid-server → sparq sidecar): native zstd both sides,
  dictionaries trivially shareable.

A custom-dictionary protocol needs a way for clients to OBTAIN the dict (e.g. a
`/dictionary` endpoint keyed by dataset generation; dictionary id echoed in a
response header). Design-level only. See
`research/custom-parsers-ADDENDUM-zstd-js-clients.md`.

Reproduce: `bench/parse/README.md`.
