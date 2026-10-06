---
name: rust-parallel-parsing
description: How to make text-format parsing (N-Triples / Turtle / line-delimited records) go fast in Rust by chunk-parallel scanning, and — load-bearing — when NOT to. Use when working on sparq-core's ingest path (nt.rs, load_reader_parallel, turtle_chunks, build_external_ntriples_parallel), when tempted to write a "faster custom parser", when sizing parallel-parse chunking, or when reasoning about blank-node / prefix scope across chunk boundaries. Grounded in this project's MEASURED baselines — cite the numbers, and steer away from the REJECTED ideas.
---

# Rust parallel parsing for RDF ingest

Authored from this project's measured research. Verify any claim
against `research/custom-parsers-baseline.md` and `research/fast-ingestion.md`
before quoting it — those are the source of truth and were measured on an M1 Air.

## The one rule that overrides intuition

**There is no custom-parser goldmine here, and measurements have killed every
"obvious" win.** Before proposing a parser change, read the verdicts table in
`custom-parsers-baseline.md`. The incumbent path is already a tuned custom
parallel parser; byte-scanning is **0.4% of memory bandwidth** (60 GB/s memscan
vs ~234 MB/s parse). The cost is in interning + index build, not in scanning.

Measured facts to anchor on (M1, real Wikidata slice; absolute throughput numbers
drift with hardware — they live in `research/custom-parsers-baseline.md` and the
`bench/parse/` harness, cite that, but the *ratios* below are the load-bearing point):

- Custom serial NT parse+intern is already **~1.27× faster than oxttl parse-only**
  — it parses *and* interns in less time than oxttl takes to parse and throw
  triples away.
- Parallel NT parse+intern scales ~**3.84×** on 4P+4E; full ingest scales similarly.
- A *zero-cost* scanner would cap full serial ingest at only ~1.8×, and a realistic
  2× scanner at **≤1.2× end-to-end at 8T**, because intern+build are 65%+ of the
  parallel path. **Not worth a second parser to differential-test.**

## Chunk-parallel scheme (the shipped design)

The pattern that works, by format:

- **N-Triples / N-Quads — trivially correct.** Records are newline-delimited
  and self-contained (no document scope). Split the byte buffer at newline
  boundaries into ~equal chunks, parse each chunk on the rayon pool into a
  per-chunk partial `Dict`, then merge. See `sparq-core/src/nt.rs`
  (`parse_chunk`) and `build_external_ntriples_parallel` in
  `sparq-core/src/lib.rs`. Upstream oxttl now also ships
  `split_file_for_parallel_parsing` for NT/NQ — so "parallel NT via the stock
  parser" no longer even needs custom code.
- **Turtle — byte ranges are WRONG.** Prefixes, multi-line literals, `;`/`,`
  continuation, and document-scoped blank-node labels all cross arbitrary byte
  boundaries. You need **statement-terminator chunking**: scan for a `.`
  followed by whitespace/EOF/`#`, skipping over strings/IRIs/comments
  (`turtle_chunks` in `sparq-core/src/lib.rs`). A `.` cannot terminate a
  statement inside `[...]`/`(...)`/labels in *valid* Turtle (DECIMAL/DOUBLE need
  a digit after the dot; PN_LOCAL/BLANK_NODE_LABEL cannot end in a dot), so the
  scanner needs no bracket tracking; mis-splits on *invalid* input fail chunk
  parse → serial fallback.

## The blank-node / prefix scope trap (and how it was actually solved)

This is the subtle part and the source of a measured **0% speedup on real data**
bug that lived in the shipped parser:

- **The old `turtle_chunks` bailed to serial on ANY blank node.** Real Wikidata
  has ~42 `_:` somevalue nodes per 1.5M statements — one bnode per 35K
  statements forfeited the *entire* parallel win (1.004 s @ 8T == 0.998 s @ 1T),
  while bnode-free synthetic got 2.9×.
- **The fix was to NOT pre-scan or namespace per-chunk — let the dict merge be
  the shared label-intern map.** Labeled `_:x`: oxttl preserves the written
  label, and per-chunk partial dicts merge into the global `Dict` by term
  equality (`merge_remap` → `intern_blank`), so cross-chunk occurrences of a
  label unify to one id, exactly as a serial document-scoped parse does.
  Anonymous `[...]`/`(...)`: confined to one statement (one chunk); oxttl mints
  them as random 128-bit ids (`BlankNode::default()`), so cross-chunk
  distinctness carries the same probabilistic guarantee the serial parser
  already relies on within one document. **No pre-scan, no label threshold, no
  per-chunk namespacing.** Result: 1.004 → 0.479 s @ 8T (2.1×), parity with the
  bnode-free path.
- **Prefixes (Turtle `@prefix`)** are document-scoped declarations that precede
  use. The statement-terminator chunker keeps each `@prefix` directive whole and
  oxttl re-parses prefixes per chunk from the prefix-bearing prelude — if you
  ever chunk away the prelude you must broadcast a prefix snapshot to each chunk.
  Verify the exact mechanism in `turtle_chunks` before changing it.

Correctness discipline: any parallel scheme needs a **differential test** that
compares chunked vs serial output after first-occurrence bnode renumbering (an
exact 1:1 equality that subsumes isomorphism because document order is
preserved). See `parallel_turtle_bnodes_match_serial` /
`load_reader_parallel_short_reads_match_sequential` in sparq-core.

## SIMD / memchr scanning — measure first, it is rarely the bottleneck

`memchr`/SIMD newline scanning reaches 0.5–2 GB/s/core and parse-alone is
estimated ~5 M/s (→ 8+ M/s with SIMD). But parse is **not the binding
constraint** here — decompression and sort are. Hand-tuned SIMD line scanners
(simdjson-class) bound a custom NT scan at ~2–4× the incumbent's ~350 MB/s
scan-only rate, and that does **not** remove intern (~0.24 s) or build
(0.103 s @ 8T). Reserve SIMD for the scan-only stage and only after profiling
shows scanning dominates (it does not today).

## The one mandatory serialization point

`Dict::merge_remap` (`sparq-core/src/dict.rs`) assigns global ids, so the
per-block partial-dict merge must be serial. Everything else (decompress, parse,
the 6 permutation sorts) parallelizes. Residual parallel headroom is **dict-merge
scaling on many-core boxes**, a separate workstream — not a parser rewrite.

## Checklist before touching the parser

1. Have you read the verdicts table in `custom-parsers-baseline.md`? (i) custom
   single-thread NT = REJECT, (ii) parallel NT = already shipped, (iii) Turtle =
   pursue the bnode fix only, (iv) fused unzip+parse = the real win (see the
   `fused-decompress-parse` skill).
2. Are you about to add a second parser to differential-test? The math says
   ≤1.2× end-to-end. Patch sparq-core instead. `sparq-parse` stays an empty
   scaffold until the fusion fix lands and the question is revisited.
3. Did you add a differential test against the serial path?
4. Did you re-check the wasm guard? The parallel path is **not** compiled for
   wasm (`sparq_wasm.wasm` size must be unchanged).

Reproduce all numbers: `bench/parse/README.md`.
