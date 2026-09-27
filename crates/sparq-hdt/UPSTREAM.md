<!-- [OPUS-4.8] sq-2te / sq-v7be — upstream-contribution status for KonradHoeffner/hdt. -->
# Upstream contributions to `KonradHoeffner/hdt`

This crate WRAPS [`hdt`](https://github.com/KonradHoeffner/hdt) (MIT). This file
tracks the upstream gaps the sparq-hdt work has queued against it and
their CURRENT status against `hdt` master / 0.7.x.

> **Status (sq-v7be, verified 2026-06-19 against `KonradHoeffner/hdt` master):**
> - **Builder gap — OBVIATED (no upstream change needed from us).** See item 1.
> - **Decode-only entry point — STILL OPEN on the released line ([FABLE-5] sq-fkj,
>   re-verified 2026-07-18 against the published `hdt` 0.7.3 source, the version
>   sparq now pins): no release through 0.7.3 ships one. Upstream DRAFT PR
>   [`KonradHoeffner/hdt#124`](https://github.com/KonradHoeffner/hdt/pull/124)
>   remains the tracked path, awaiting @jeswr's own review (`needs:user`).** See
>   item 2.
> - **`read_nt` stores literal lexical forms N-Triples-ESCAPED (spec says raw) —
>   DOCUMENTED LIMITATION (sq-qalqs).** See item 3.
> - **Section CRC32-C is computed with the `crc` crate's byte-at-a-time table —
>   OPEN, not yet filed upstream (#3517, measured 2026-07-27).** See item 4.

---

## Item 1 — in-memory section builders without `sophia` — OBVIATED

**Original ask (sq-ashy):** make the in-memory section builders usable without
pulling the `sophia` adapter dependency tree, so sparq could WRITE a `.hdt` from
its own in-memory dict + triples with no N-Triples text round-trip.

**Why it is obviated.** On current `hdt` master the section builders sparq needs
are already `pub` and **not** gated behind `sophia`:

- `DictSectPFC::compress(&BTreeSet<&str>, block_size) -> DictSectPFC` — gate-free.
- `TriplesBitmap::from_triples(&[TripleId]) -> TriplesBitmap` — gate-free.

Upstream also split the N-Triples ingest path (`lasso` / `oxttl`) out into its own
`nt` feature, so the `sophia` term adapter is no longer dragged in just to reach
the builders. (Note: there is **no** `Hdt::from_triples` constructor on the `Hdt`
struct itself — the in-memory build is done at the section level via the two
builders above, which is what sparq does.)

sparq already builds + writes a spec-conformant archive directly from these:
`sparq-hdt/src/encode.rs` calls `DictSectPFC::compress` per FourSectDict section and
`TriplesBitmap::from_triples` for the SPO bitmaps, with **no** N-Triples text
round-trip (the `save` path — `crates/sparq-hdt/src/write.rs` — and the
`encode.rs`/`decode.rs` round-trip oracle confirm this). So no upstream change is
required for the write path; this item is closed (tracked under landed bead
`sq-ashy`).

> **Dependency status ([GPT-5] sq-2l1).** sparq now uses `hdt` 0.7 and enables its
> narrower `nt` feature for N-Triples fixture generation. The upstream `sophia`
> adapter is no longer part of sparq-hdt's read or write dependency graph. This
> became portable on stable aarch64 when `hdt` moved to `qwt` 0.4, whose prefetch
> path uses stable inline assembly.

---

## Item 2 — decode-only / streaming-triples entry point — OPEN UPSTREAM DRAFT PR

**Status:** open **draft** PR
[`KonradHoeffner/hdt#124`](https://github.com/KonradHoeffner/hdt/pull/124)
(`feat: decode-only Hdt::triples_streaming (skip query-index build on bulk reads)`),
authored by `@jeswr`. It is **jeswr-review-gated** — not yet marked ready for
maintainer review — so it is not merged. Tracking bead: `sq-fkj`.

> **PR hygiene + review-gate mechanism ([OPUS-5] sq-uu7c, 2026-07-27).** #124 is
> reconciled to the N3.js upstream-contribution practice in `AGENTS.md`
> (*Upstream contributions — how to open the PR*): DRAFT, 🤖 agent self-id, a
> "NOT yet ready for maintainer review" note, a "Why" section, and a single
> self-contained change (one feature, 2 files) — so **no split is needed**.
> Branch: `feat/decode-only-triples-streaming` on @jeswr's `hdt` fork.
>
> **Reviewer assignment is not available on this PR, by design of GitHub.** Both
> `gh pr edit 124 --add-reviewer jeswr` and `--add-assignee jeswr` fail
> (`RequestReviewsByLogin` / `ReplaceActorsForAssignable`): @jeswr is the PR
> author, and an author cannot be their own requested reviewer; the agent account
> also lacks triage on the external repo. Per `AGENTS.md` step 3, the **body
> `@jeswr` review-mention is the working fallback** and is already in place —
> that is the mechanism of record for this PR.
>
> **Open action — @jeswr only (`needs:user`).** Review #124 and, when satisfied,
> mark it ready for maintainer review yourself. Agents must never flip a draft
> upstream PR to ready.
>
> **Unrelated upstream build breakage (not ours to fix).** `hdt` master was
> observed failing to build because `hdt` 0.7.1 resolves to `qwt` 0.3.5, which is
> **yanked** on crates.io. This does not affect sparq: `Cargo.lock` pins `hdt`
> 0.7.3 → `qwt` 0.4.0. It is the upstream maintainer's concern, and it is
> orthogonal to this doc-only PR.
>
> **Re-verified against the released line ([FABLE-5] sq-fkj, 2026-07-18).** With
> sparq now pinning `hdt` 0.7 (the sq-2l1 bump), the gap was re-checked against the
> published **0.7.3** crate source: no decode-only entry point has shipped in any
> release through 0.7.3. `Hdt::read` still reaches `TriplesBitmap::read_sect` →
> `TriplesBitmap::new`, which eagerly builds the query-only index structures below.
> The 0.6 `sucds`→`qwt` swap changed the backing library (`WaveletMatrix<Rank9Sel>`
> → `QWT512`, `Rank9Sel` bitmaps → `RSNarrow`), not the eager build itself. Item
> stays open pending the upstream PR; the local cost is already avoided because
> sparq's direct decoder (below) is the default load path.

**What it adds.** `Hdt::read` eagerly calls `TriplesBitmap::new`, which builds —
purely to serve triple-pattern / object / predicate QUERIES — a wavelet matrix
over `sequence_y` (`QWT512` on 0.7.x), a full sort of per-object entries
(`build_op_index_from_entries`), and an OP-index (a compact position sequence + an
`RSNarrow` rank/select bitmap). A consumer doing a one-shot bulk load (read
every triple once, in SPO order, into its own store) never issues those queries, so
all of that is built and immediately dropped — a large, cache-hostile cost on
ingest. The PR adds a decode-only entry point that reads the dictionary +
`bitmap_y` / `bitmap_z` / `sequence_y` / `sequence_z` and yields triples in SPO
order WITHOUT constructing the `TriplesBitmap` query structures, e.g.

```rust
impl Hdt {
    /// Decode-only: stream every triple in SPO order without building the
    /// wavelet matrix / OP-index used for pattern queries.
    pub fn triples_streaming<R: BufRead>(reader: R)
        -> Result<impl Iterator<Item = Result<[usize; 3]>>>;
}
```

**Reference implementation.** `sparq-hdt/src/decode.rs` already does exactly this —
it is the DEFAULT load path (`decode::graph_from_reader`, driven by the public
`load_reader` in `lib.rs`): it reads the same on-disk bytes, walks the bitmaps with
a plain bit read, and skips the rank/select build. It is differentially tested
against the full `Hdt::read` path (retained only as the oracle,
`load_reader_via_upstream` in `lib.rs`) on real and generated archives. If the
upstream PR lands we can delete our vendored decoder and call the upstream entry
point instead.

---

## Item 3 — `read_nt` escapes literal lexical forms in the dictionary — DOCUMENTED LIMITATION

<!-- [SONNET-4.6] sq-qalqs -->

**Finding (sq-qalqs, verified against `hdt` 0.4.0).** The HDT spec — and the
hdt-cpp / hdt-java reference implementations, and sparq's own `save` encoder
(`encode.rs::hdt_term_string`) — store a literal's lexical form **RAW** in the
dictionary (the sections are length-delimited, so no escaping is needed). Upstream
`FourSectDict::read_nt` (0.4, `sophia` feature) instead stores the sophia/rio
**N-Triples-escaped term rendering** (`q.object.to_string()` after only stripping
IRI angle brackets), so a literal containing `"` or `\` lands in the dictionary
with literal `\"` / `\\` byte sequences. Any spec-conformant reader — sparq's
direct decoder, sparq's upstream-backed oracle path, and hdt-cpp alike — then
decodes a lexical form escaped one time more than the N-Triples source, i.e.
`Hdt::read_nt -> Hdt::write -> sparq_hdt::load` disagrees with `Graph::load_str`
on such literals. (Upstream's own `hdt_graph::auto_term` reader does not unescape
either, so the escaped bytes round-trip verbatim *within* the upstream crate —
the non-conformance is confined to the `read_nt` **writer**.)

**Impact on sparq: test fixtures only.** `read_nt` is used solely as the
dev-dependency fixture builder (`tests/roundtrip.rs::nt_to_hdt_bytes` and
friends); the production write path (`save`) encodes the raw lexical form
directly from sparq's dict and round-trips escaped literals exactly
(`tests/write_roundtrip.rs::save_round_trips_escaped_literals_exactly`).
Fixture N-Triples containing `\"` / `\\` (or other N-Triples escapes, e.g.
`\n`, `\t`) will NOT match a `Graph::load_str` ground truth — keep escape-worthy
literals out of `read_nt`-built differential fixtures, or route them through
`save`. Same family as `read_nt`'s verbatim (non-lowercased) language tags noted
in `tests/roundtrip.rs::hdt_load_matches_ntriples_load`.

**Regression oracle:** `tests/roundtrip.rs::escaped_literal_pins_upstream_writer_as_the_mangler`
pins the stored dictionary bytes and the exact double-escaped round-trip rendering;
if a future `hdt` bump fixes `read_nt`, that test goes red — then delete this item
and tighten the oracle into an exact round-trip equality. Not yet reported upstream.

---

## Item 4 — CRC32-C is computed byte-at-a-time — OPEN, NOT YET REPORTED

<!-- [SONNET-4.6] #3517 -->

**Finding (#3517).** Section CRC verification is a measurable minority slice of an
HDT load, and the overwhelming majority of the bytes it covers are CRC'd by
upstream, not by sparq: `DictSectPFC::read` (the four PFC dictionary sections and
their offset sequences) and `Sequence::read` (the two triple sequences) between them
account for nearly all of an archive's CRC32-C payload. sparq computes only the two
triple bitmaps' CRCs itself, in `decode.rs::read_bitmap_words`.

Both sides reach for the [`crc`](https://crates.io/crates/crc) crate's **default
byte-at-a-time `Table<1>`** implementation. The same crate also offers slice-by-16
(`Crc<u32, Table<16>>`) — identical algorithm, identical checksum, `const`-constructible
table, **no new dependency and no `unsafe`** — which computes the same verification
several times faster. The upstream reader is where that change would pay.

**Why sparq is not doing it locally.** The slice it could reach on its own (the two
bitmaps) is a small enough fraction of archive bytes that switching it lands in the
noise, so the crate keeps the default table rather than carry an unevidenced
micro-optimisation. See `decode.rs`'s module docs for the full measured verdict — the
same measurement that closed #3517's proposed `load_unchecked` (CRC-skip) variant as
**not worth building**: skipping verification cannot beat verifying it faster.

**Reproduce:** `cargo run --release -p sparq-hdt --example bench_crc_share` (generate
the archive first with `--example bench_load`). The harness reports the load wall time,
the CRC pass, the per-section byte split, and the slice-by-16 comparison; its unit
tests pin that both table widths produce the same checksum.

**Status:** not yet filed upstream. Would be a small, self-contained PR against
`KonradHoeffner/hdt` touching the CRC construction sites only.
