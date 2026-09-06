<!-- [FABLE-5] sq-hmd7l.14 — per-axis gap record: RDF serialization throughput.
First-read only; work-box readings are NON-canonical by construction and are never
transcribed here. -->

# Gap record — RDF serialization throughput (2026-07)

**Axis:** RDF document serialization (epic `sq-hmd7l` comparative-benchmarking).
**Status:** harness DELIVERED + smoke green; full five-column panel exercised on the
work box (sparq gates green, peer outcomes recorded); no canonical quiet-box run yet (§4).
**Harness:** `bench/serialize/` (`run.sh` + `crates/sparq-engine-serialize/examples/serialize_bench.rs`
+ `scripts/bench-adapters/serialize_oxrdfio_adapter.sh`), registered as `serialize-bench`.

## 1. Engines and honest scope

| Engine | What runs | Comparability |
|---|---|---|
| sparq | `sparq_engine::serialize::*` writer matrix via the `serialize_bench` example (`serialize-rdf` + `streaming-serialization` features) | subject |
| serd (`serdi`) | as-shipped C CLI, NT→{Turtle, NT} | tier-1 native reference |
| Raptor (`rapper`) | as-shipped C CLI, NT→{Turtle, NT} | LGPL-2.1 — note license terms on published numbers |
| Jena (`riot`) | as-shipped JVM CLI, NT→{Turtle, NT}; wall includes JVM startup (labelled) | tier-1 JVM reference |
| oxrdfio | gather-time scratch shim (materialize-then-serialize, mirroring sparq's `pipe`), pinned to the oxrdf/oxttl family the workspace uses | closest-architecture Rust reference |

Scope invariants (enforced by `run.sh`, non-negotiable):

- **Round-trip before stopwatch:** every emitted document must re-parse (with
  sparq's parsers) to exactly the source store — triple count + exact sorted
  canonical N-Quads line match — before its timing row is trusted. Blank-node
  corpora are refused fail-closed (exact match, not isomorphism; the generated
  corpus is blank-node-free by construction).
- **The gate must be able to fail:** every run corrupts a document and asserts
  the gate reds it (exit 4) — the non-vacuity self-check.
- **Two regimes, never cross-compared:** in-process serialize-only (sparq's
  buffered vs streaming vs pretty rows; MB/s over output bytes) vs one-process
  pipeline (cross-engine; `mbps_in` over the shared input corpus is the only
  cross-comparable number, since output bytes differ per engine).

## 2. First-read findings (work box — NON-canonical, no timings transcribed)

1. **sparq's full writer matrix round-trips green** on the generated corpus
   (nt / turtle / turtle-pretty / turtle-stream / trig / trig-stream), and the
   pipeline columns (NT→Turtle, NT→NT) gate green.
2. **The gate caught a real peer round-trip defect on the first run:** rapper's
   Turtle writer emits `xsd:double` literals in bare decimal syntax
   (`"287.8329"^^xsd:double` → `287.8329`), which re-parses as `xsd:decimal` —
   a datatype-corrupting serialization. Recorded as `red(rc=4)`; rapper's
   Turtle column is untimed. Its N-Triples column round-trips green. This
   validates the bytes-before-stopwatch design: a throughput table without the
   gate would have ranked a writer that corrupts data.
3. **First-read pipeline ordering on this box** (qualitative only): sparq led
   both gated pipeline columns (NT→Turtle and NT→NT), with the oxrdfio shim
   close behind on Turtle, then serdi, then rapper. In-process, sparq's
   streaming Turtle writer beat the buffered writer (no whole-document
   `String`), and the nt/nquads writer ran an order of magnitude faster than
   prefix-compacting Turtle — expected, it does no compaction work. NOT a
   dominance claim: work-box, single corpus shape, riot column absent (§4).
4. **Output-byte spread is large across engines** (sparq's Turtle emitted ~35%
   more bytes than the oxrdfio shim's on the same corpus at the same prefix
   map, e.g. predicate-object-list layout differences), which is why the panel
   defines `mbps_in` over the shared input as the comparable metric and treats
   output MB/s as per-engine only.

## 3. Fix candidates surfaced

- None blocking. Turtle prefix-compaction dominates sparq's serialize cost
  relative to the nt writer (expected); if a canonical run shows a real gap vs
  serd/oxrdfio on Turtle, profile `write_prefix_header`/abbreviation lookup
  first. Buffered-vs-streaming already favors streaming.

<!-- [FABLE-5] sq-0kq6k — CORRECTION. This section previously read "…favors
streaming, which is the shipped HTTP path". That was wrong in two ways and is
recorded here rather than quietly deleted. (1) At the time it was written NO
HTTP path called the streaming writers at all — wiring them was the open bead
this note closes. (2) More importantly, `sparq-server` has never used this
crate's Turtle writer for a CONSTRUCT/DESCRIBE response: it serialises through
`oxttl::TurtleSerializer` (`sparq_server::graph::triples_to_turtle`), a
different writer with a different output shape — §2.4 above measures sparq's
Turtle emitting materially more bytes than the oxrdfio family on the same
corpus at the same prefix map. So `write_turtle_streaming` was never the
server's writer, and swapping it in would have changed the bytes on the wire.
The HTTP CONSTRUCT/DESCRIBE path is now genuinely streamed, but through the
`oxttl` serialiser's own `io::Write` seam, which keeps the response
byte-identical. The writers in THIS crate are the shipped streaming path for
`sparq-cli dump … turtle|trig` (opt-in `streaming-serialization`). -->

## 4. What a canonical run still needs

- Quiet-box gather (`bench/ec2-bench.sh` conventions, `quiet_box: true` in the
  envelope), with the `riot` column installed (absent on the work box) and a
  larger corpus tier (`SERIALIZE_SUBJECTS`), before any dashboard/site number.
- RDF/XML stays out of scope until sparq has an RDF/XML writer AND an in-repo
  re-parse oracle; SPARQL result-set serialization (JSON/XML/CSV) is a
  follow-up axis (needs result-set — not RDF-document — oracles and different
  competitor tooling).
