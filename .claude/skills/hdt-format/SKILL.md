---
name: hdt-format
description: The HDT (Header-Dictionary-Triples) binary RDF archive format — layout, dictionary sections (Plain Front Coding), BitmapTriples with rank/select, .hdt.gz handling, and how the sparq-hdt crate wraps the `hdt` crate. Use when working on sparq-hdt (loading .hdt archives into a sparq Graph), reasoning about HDT decode performance, the id-level translation into sparq's Dict, the opt-in feature/MSRV gating, or HDT write-support gaps.
---

# HDT — Header Dictionary Triples

Authored for the active HDT work. Ground truth: the `sparq-hdt` crate
(`crates/sparq-hdt/src/lib.rs`, `Cargo.toml`; open work in beads — `bd list -l area:sparq-hdt`)
and the upstream `hdt` crate (KonradHoeffner/hdt, MIT). Verify crate API against the pinned
version before writing code.

## What HDT is

HDT (rdfhdt.org, W3C member submission) is the de-facto **binary archive** format
for RDF. A single self-contained file is split into three components — hence the
name:

- **H — Header.** RDF metadata about the dataset itself (VoID statistics, format,
  provenance) as plain triples. Queryable; only the head of the stream needs
  decoding to read it.
- **D — Dictionary.** All distinct terms, stored compressed. The standard layout
  is a **FourSectionDictionary**: four sections (subjects-only, predicates,
  objects-only, and shared subject∧object terms), each **Plain Front Coded
  (PFC)** — terms are sorted and each stores only the byte-length of its common
  prefix with the previous term plus its suffix. A term maps to a dense integer
  **id** per section.
- **T — Triples.** A **BitmapTriples** structure: the triples in SPO order as a
  bitmap-compressed adjacency list (Log64/Plain integer sequences + a bitmap
  marking the end of each subject's / predicate's adjacency run). Lookups use
  **rank/select** over the bitmaps to navigate from a subject id to its predicate
  run to its object run in O(1)-ish per step.

The payoff: files are a fraction of the size of even gzipped N-Triples, and they
load **without text parsing** — no tokenizing, no UTF-8 re-validation per triple.

## How sparq-hdt uses it (the actual integration)

The crate **wraps** the maintained `hdt` crate's reader rather than
reimplementing the binary format. Key design facts (in `lib.rs` / `Cargo.toml`):

- **Id-level translation.** Each distinct HDT dictionary id is decompressed to its
  term string **once**, interned into the sparq `Dict`, and the mapping memoized
  in a flat per-section table. So the term set is never materialized twice and
  per-triple work is **three array lookups** (s/p/o id → sparq Id). This is the
  fast-decode lever: do not route through `sophia` terms — `default-features =
  false` deliberately drops the sophia adapter.
- **`.hdt.gz` by magic bytes.** GZipped containers are detected by **magic bytes
  (0x1f 0x8b)**, not file name, in every entry point, and decompressed on the fly
  with a streaming `MultiGzDecoder` (flate2, pure-Rust backend) — same magic-byte
  convention as the rest of the workspace.
- **Header access.** `header(path)` / `header_reader(reader)` expose the dataset
  metadata triples as a queryable sparq `Graph`, decoding only the control-info +
  header sections (a few KB). The wrapped crate parses the header internally but
  keeps the field private, so these re-read those sections directly.
- **Entry point:** `sparq_hdt::load("dataset.hdt") -> Result<Graph, Error>`.
  `Error` is `Io | Hdt | Term`.

## Gating: why it is opt-in (don't "fix" this)

`sparq-hdt` is a **separate, opt-in crate** so the core engine — and crucially the
wasm build — carries **zero HDT code or deps**. The CLI engages it behind a cargo
feature: `cargo build -p sparq-cli --features hdt`; without it the CLI exits with
a rebuild hint. Two deliberate reasons (documented in `sparq-cli/Cargo.toml`):

1. The wrapped `hdt` crate's **MSRV is 1.87**, above the workspace floor of 1.85
   — a default dep would silently raise the CLI's MSRV. `rust-version = "1.87"`
   is declared per-crate.
2. The HDT decode stack is dead weight on non-HDT paths.

**Version pin: `hdt = "0.4"`.** Do NOT bump to 0.6 casually — 0.6 pulls `qwt`,
whose default `prefetch` feature needs **nightly** rustc on aarch64 (tracked in
beads — `bd list -l area:sparq-hdt`). Native-only by design (the reader spawns
threads; the use case is bulk ingest).

## Known gaps

- **Write support is DEFERRED (upstream-blocked).** As of `hdt` 0.6.0 there is
  still no in-memory triples → FourSectionDictionary builder API. `Hdt::read_nt`
  takes a file *path* (builds dicts from files via a lasso interner), so
  `Graph → .hdt` would mean serializing to a temp N-Triples file first — not
  cheap. The `sophia` dev-dep feature provides that NT→HDT writer ONLY for
  building round-trip / benchmark fixtures in tests, not for production write.
- Only the standard layout is read: FourSectionDictionary (PFC) + BitmapTriples,
  SPO order — what hdt-cpp / hdt-java emit.

## Quick checklist

1. Don't bump the `hdt` pin past 0.4 without re-checking the qwt/nightly issue.
2. Keep HDT out of `sparq-core` and the wasm build — it lives only in `sparq-hdt`.
3. Translate at the id level (three lookups), never through sophia terms.
4. Detect compression by magic bytes, not extension.
