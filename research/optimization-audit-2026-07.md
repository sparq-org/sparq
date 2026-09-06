# Optimization audit — 2026-07 (epic sq-7d3dj)

> 🤖 SPARQ agent — Fable-tier prioritisation record. [FABLE-5]

**Method.** Five parallel profiler agents digested one dimension each (query-perf, memory,
HTTP-throughput, ingest, WASM-bundle) into prose with file:line grounding, honest caveats,
and baseline-metric definitions. This document is the architect-tier adjudication over
those digests only (no fresh code reading): rank by *expected win x low quality-risk x
measurability*, drop premature micro-opts, refuse anything that erodes soundness,
readability, or a deliberate design tradeoff, and avoid duplicating in-flight programs.

**Ground rules inherited from the epic (sq-7d3dj):**

- Perf claims require **canonical** measurement: the deterministic perf-gate metrics
  (`scripts/perf-gate.py` / `bench/perf-baseline.json`) or an EC2 benchmark run
  (sq-vw3ax.12). Work-box numbers quoted below are **non-canonical directional
  brackets** used only to size expectations, never claims.
- Deterministic ratchets must hold or improve on every item. `scripts/perf-gate.py`
  classifies by `mode` in `bench/perf-baseline.json`: **every `mode: auto` metric
  hard-fails** (exit 2), and only `mode: noise` timing metrics are advisory. The
  `mode: auto` metrics relevant to the items below are `wasm_bundle_bytes`,
  `store_bytes_per_triple[_small]` and `dict_bytes_per_term` (the baseline also gates
  others, e.g. `comp_store_bytes_per_triple`, `fts_bytes_per_doc`,
  `vectors_{diskann,pq}_recall_at10`, `geo_compliance_deficit`). `parse_ns_per_byte` is
  the sole `mode: noise` metric — wall-clock-derived and therefore an **advisory timing
  signal (tracked/warned, non-blocking)**: watched on every item, but it never blocks a
  merge. No clippy/readability/soundness compromise; new risk-bearing
  paths ship **opt-in** behind differential tests (the established yannakakis / semijoin-bitmap /
  cs-planner pattern).

**Not re-proposed here (owned elsewhere):**

- **M4 vector-at-a-time** (morsel pipeline, columnar filter/join/aggregate, DataChunk
  wiring) — epic **sq-pntvh**, 8 phased children in flight. Both the query-perf and
  memory digests independently converge on "full inter-operator materialisation +
  per-row dispatch" as the largest structural cost; that *is* M4. Nothing below
  duplicates it, and the memory digest's "streaming pull-based operators" idea is
  explicitly folded into sq-pntvh rather than beaded as a memory item.
- **DPccp/DPhyp join enumerator + the stale doc claim** — **sq-iywur** (IN_PROGRESS,
  `feat/sq-iywur-dp-planner`) plus **sq-clhn1** (wiring). The digests confirmed
  `research/optimization-techniques.md` still claims DPccp is shipped while only GOO
  greedy exists on main; that correction is in sq-iywur's scope — not repeated here.

---

## Prioritised roadmap

Ranked by expected win x low quality-risk x measurability. P2 = do next; P3 = after
P2s or measure-first. Each item is an impl bead under sq-7d3dj carrying this table's
measurement plan.

| # | Bead | P | Dim | Opportunity | Expected win | Quality risk |
|---|------|---|-----|-------------|--------------|--------------|
| 1 | sq-7d3dj.1 | P2 | wasm | `release-wasm` profile: cold parser crates at opt-level `z`, hot engine stays `3`, `strip="symbols"`; re-baseline floor DOWN | raw ratchet approx. -8..-14% (non-canonical bracket); shipped bundle shrinks too | LOW (cold-code only; dual wiring is the hazard) |
| 2 | sq-7d3dj.2 | P2 | ingest | Pre-size per-chunk partial `Dict` + triples `Vec` from chunk byte length | few % of ingest (kills the ~5% rehash/regrow cost) | ~ZERO (capacity hints; HDT precedent) |
| 3 | sq-7d3dj.3 | P2 | memory | Overlay zero-copy scan fast path + cached perm-sorted `added` projections | restores allocation-free scans for every range a small overlay does not touch — the mutated-server read path | LOW (equivalence test already pins behaviour) |
| 4 | sq-7d3dj.4 | P2 | query | Pre-resolve variable->column indices in FILTER/BIND/ORDER BY expression eval | modest; scales with result size x filter complexity (honest: small for 3-6 vars) | LOW (mechanical; no semantic change) |
| 5 | sq-7d3dj.5 | P2 | http | Canonical loopback HTTP throughput harness (req/s, p50/p99, peak RSS) | measurement itself — the HTTP lane currently has NO canonical metric | ZERO (bench-only) |
| 6 | sq-7d3dj.10 | P2 | http | Multi-core SELECT-JSON serialize under deadline-only budgets, WITH a mid-serialize deadline re-check (verdict below) | near-linear multi-core on large-SELECT serialize (currently single-core on the default server) | MEDIUM, contained by the re-check design |
| 7 | sq-7d3dj.6 | P2 | ingest | Build numerics/temporals caches from borrowed `Stored` components (no per-id owned `Term`) | removes O(distinct-terms) allocations from `Graph::build` | LOW (borrowed path already proven for spill) |
| 8 | sq-7d3dj.11 | P3 | query | Opt-in compiled scalar-expression programs for FILTER/BIND (flat ops over resolved columns) | removes per-row tree-walk dispatch on the string/logical/function path M4 does not vectorise | MEDIUM (3VL / type-error / term-identity) — opt-in + differential |
| 9 | sq-7d3dj.7 | P3 | query | `bind_join`: sort-based run grouping instead of `FxHashMap<Id,Vec<usize>>` + per-value scans | fewer allocs/probes on high-NDV bind joins | LOW |
| 10 | sq-7d3dj.12 | P3 | http | Chunked streaming for CSV/TSV SELECT results (mirror the JSON chunk path) | large-export peak memory drops like the JSON path | LOW (row-oriented formats chunk trivially) |
| 11 | sq-7d3dj.13 | P3 | http | Design record: "Wave D" pull-streaming body (bounded channel -> `Body::from_stream`) | TTFB + peak memory stop scaling with full result size | HIGH if rushed — design-first, sized as its own task |
| 12 | sq-7d3dj.8 | P3 | memory | MEASURE-FIRST: sparsify the native numerics cache (as temporals already do) | RSS drop on string-heavy native loads (8 B/term dense NaN vec) | MODERATE — trades O(1) array probe for hashmap on the numeric fast path; adopt only on A/B win |
| 13 | sq-7d3dj.9 | P3 | ingest | MEASURE-FIRST A/B pair: recently-seen-term intern shortcut + memchr NT delimiter scans | slice of the ~30% dict-lookup cost on subject-grouped dumps; fraction of ~8% byte-scan | MODERATE — hot-loop branch; keep only on measured win; id-bijection must be byte-identical |
| 14 | sq-7d3dj.14 | P3 | wasm | Emit shipped (`wasm-opt -Oz`) bundle size as a TREND-ONLY series next to the raw ratchet | metric fidelity: shipped-size wins become visible | ZERO (gate untouched — verdict below) |

### Dependency order (edges recorded in bd)

- sq-7d3dj.10, .12, .13 depend on **sq-7d3dj.5** (no canonical HTTP metric exists;
  nothing in the HTTP lane may claim a win before the harness lands). .13 should also
  digest .10's landed shape.
- sq-7d3dj.11 depends on **sq-7d3dj.4** (column resolution is the substrate the
  compiled program indexes).
- sq-7d3dj.14 depends on **sq-7d3dj.1** (the profile change moves both series; land
  the trend series against the new profile so it never reports a phantom win).
- sq-7d3dj.1, .2, .3, .4, .5, .6, .7, .8, .9 are mutually independent — safe to drain
  as a disjoint-crate wave (wasm profile / sparq-core ingest / sparq-core store /
  sparq-engine / sparq-server bench).

---

## Per-item detail

### 1. `release-wasm` profile — surgical size-opt (P2, wasm)

**Approach.** Add `[profile.release-wasm] inherits = "release"` to the root
`Cargo.toml` with `strip = "symbols"` (must be `"symbols"` — `"debuginfo"` does not
touch the wasm `name` section) and per-package `opt-level = "z"` overrides for the
COLD parse/validate crates only: `spargebra` (~19% of code, runs once per query in
microseconds), `oxiri` (~8%, ingest-time validation), `oxttl` (~4%, once per load).
`sparq-engine` / `sparq-core` / `sparq-substrate` stay at opt-level 3 — the simd128 +
`core::simd` FILTER-kernel investment is explicitly preserved. Wire **both**
`scripts/ci-bench.sh` and `js/package.json` (`build:wasm` / `build:wasm:lean`) to
`--profile release-wasm` — if the gate and the shipped build diverge, the ratchet
measures a binary that never ships. Re-baseline `wasm_bundle_bytes` **downward**.

**Rejected alternative:** blanket `opt-level = "z"` (non-canonical bracket: raw
-13.7%) — it size-pessimises the hot evaluator (25.7% of code) and undercuts the
stated simd128 perf investment. The surgical variant captures most of the win at ~zero
query-eval cost.

**Measurement plan.** Canonical: `wasm_bundle_bytes` deterministic ratchet
(`bench/perf-baseline.json`, current floor 1,686,907) re-baselined down in the same PR;
CI x86_64 value is the number of record. Guard: an EC2/CI `op_*` wasm-mode query-perf
sanity check confirming the engine crates were untouched (expected: no delta, since
their opt-level is unchanged). Record the shipped `pkg`/`pkg-node` sizes in the PR body
as reference points.

### 2. Pre-size per-chunk partial Dict + triples Vec (P2, ingest)

**Approach.** In `parse_block`, replace `Dict::new()` with
`Dict::with_capacity(estimate)`; in `nt::parse_chunk`, `reserve` the output Vec.
Estimate triple/term counts from chunk byte length (bytes / avg-line-length), capped to
avoid over-allocation on tiny chunks — the exact pattern the HDT decoder already uses.
Directly attacks the ~4.9% `reserve_rehash` + Vec-regrow profile cost.

**Measurement plan.** Canonical: `parse_ns_per_byte` on the pinned CI corpus — an advisory
timing signal (tracked/warned, non-blocking), watched for regression. Hard guards:
byte-identical ingest differentials
(`dict_consolidation_differential.rs`, `parallel_serial_load_differential.rs`) unchanged;
`store_bytes_per_triple[_small]` + `dict_bytes_per_term` ratchets neutral (capacity
hints do not change stored bytes).

### 3. Overlay read-path: zero-copy fast path + cached projections (P2, memory)

**Approach.** Two stacked changes in `sparq-core::store`: (a) in `scan_with`, before
`Overlay::merge` copies the whole base range, use the already-existing
`count_correction` to test whether the overlay intersects this range at all; on
`(0, 0)` return the borrowed base slice (`Cow::Borrowed`) — restoring allocation-free
scans for every range a small overlay does not touch (the common case for a read-mostly
server between compactions). (b) Cache the perm-sorted `added` projections on the
Overlay, computed once in `apply_delta` instead of re-projected + re-sorted on every
scan; invalidate on mutation.

**Measurement plan.** Correctness: the existing `overlay_scans_match_rebuild`
equivalence test (extend with a does-not-intersect case); scan rows must stay
byte-identical INCLUDING sort order (merge joins depend on it). Perf: EC2 criterion
microbench — scan latency + allocation count with a small overlay over a large base,
before/after. Gates: byte-ratchets neutral (runtime allocation only). Substrate
no-dyn-dispatch check untouched.

### 4. Pre-resolve variable->column indices in expression eval (P2, query)

**Approach.** In `apply_filter` / BIND / `order_bindings` / projection, walk the
`Expression` once per operator invocation and resolve every `Expression::Variable`
node to a column index (rewritten tree or a side table), so the per-row loop indexes
`row[c]` directly instead of `Bindings::col`'s linear per-row, per-var-node
string-compare scan. Pure mechanical hoist; no semantic change.

**Honesty note.** For typical small var counts (3-6) the per-row scan is cheap; the win
is real but modest, growing with result size and multi-clause filters. Accept the bead
on trend-improve-or-neutral + the cleanliness of unblocking item 8.

**Measurement plan.** Canonical: EC2 run of the trend-only series
`op_filter-string` / `op_filter-in` / `op_filter-exists` / `op_bind` (+ SP2B
FILTER/OPTIONAL queries), min-of-iters us, before/after on the pinned corpus
(`sparq-cli bench` per `benchmarks.toml`). Correctness: full W3C + differential suites
unchanged.

### 5. Canonical loopback HTTP throughput harness (P2, http — prerequisite)

**Approach.** The server has no *canonical, CI-emitted* HTTP throughput metric in `bench/`
(the research spike in `bench/serve/loadgen` is non-canonical and not tracked for regressions) —
every HTTP opportunity is currently unmeasurable for trend/regression purposes, so this bead gates the lane. Add a bench lane
that binds the loopback server and drives it with an external load generator
(oha/wrk/k6): fixed small-SELECT workload at concurrency 1/8/32 (matching
`max_concurrent` 32) reporting req/s + p50/p99, PLUS peak RSS while serving one large
SELECT (captures the whole-result materialisation cost), PLUS a microbench of
`eval_select_json_chunks` serial-vs-parallel in isolation (quantifies item 6's headroom
before committing to it). Emit as **trend-only** series into benchmark-data (like
`op_*`); quiet-box-sensitive, so canonical numbers come from the EC2 runner, never this
work box.

**Measurement plan.** The bead IS the measurement plan. Acceptance: series emitted by
`scripts/ci-bench.sh` (or a sibling script) reproducibly; documented in the bench README.

### 6. Multi-core SELECT-JSON serialization — Fable verdict (P2, http)

**The judgment call the profile escalated.** The rayon parallel JSON serializer is
gated on `!budget::active()`, but the default server always installs a 30s
deadline-only budget, so the parallel path is dead code on the HTTP surface and every
large SELECT serialises on one core. The conservatism is deliberate: the parallel path
cannot cooperatively abort mid-serialize, so under a deadline-only budget (no row/byte
caps) a pathological result could burn all cores past the deadline — a real
DoS-resistance property.

**Verdict: do it, but NOT as a gate flip.** The right form preserves the bounded-CPU
property: allow the parallel fan-out when the installed budget carries **no row/byte
caps** (deadline-only), and add a **coarse deadline re-check at par-chunk boundaries**
so the worst-case overrun is bounded to approximately one chunk per worker — a bounded
constant, not an unbounded burn. Rows are already fully materialised before
serialization, so peak memory is unchanged by parallelism. A blanket
`!budget::active()` -> `true` flip is REJECTED.

**Measurement plan.** Depends on bead 5. Canonical: the harness's large-SELECT req/s +
p99 and the serial-vs-parallel microbench, on EC2. Hard guards: byte-identical response
bodies, Content-Length contract preserved, existing timeout/load-shed tests green, and a
new test pinning the bounded-overrun behaviour (huge result + tiny deadline terminates
within the chunk bound).

### 7. Borrowed-component numeric/temporal cache build (P2, ingest)

**Approach.** `numerics_of` currently materialises an owned `oxrdf::Term` per distinct
dict id just to inspect its datatype. Rebuild the in-memory numeric/temporal caches from
borrowed `Stored` components using the `is_numeric_datatype_str` borrowed path already
proven by the spill build — removing O(distinct-terms) allocations from `Graph::build`.

**Measurement plan.** Canonical: `parse_ns_per_byte` watched for regression (advisory
timing signal — tracked/warned, non-blocking; the cache build is inside the load path).
Hard guard: the existing streamed==dense byte-identity assertion —
the produced cache must be byte-identical to the current dense write. Ratchets neutral
(numerics/temporals bytes are not in the gated metrics; footprint unchanged).

### 8. Opt-in compiled scalar-expression programs (P3, query)

**Approach.** Compile each FILTER/BIND `Expression` once into a flat program (Vec of
ops over resolved column indices + constants; the existing `eval_numeric` fast path
becomes one op kind), evaluated per row without tree-walk match dispatch. Ships as an
**opt-in cargo feature** with differential tests — the yannakakis/cs-planner pattern —
because it touches a correctness surface: SPARQL three-valued logic, type-error
propagation, and term identity (sameTerm/STR/BIND passthrough) must be preserved
exactly. Targets the string/logical/function/EXISTS path that M4's numeric
vectorisation does NOT cover, so it complements rather than overlaps sq-pntvh.
Depends on item 4.

**Measurement plan.** Canonical: EC2 `op_filter-string` / `op_filter-in` /
`op_filter-exists` / `op_bind` + SP2B trend, feature ON vs OFF. Hard guards: full W3C
conformance + a dedicated ON==OFF differential suite; `wasm_bundle_bytes` unchanged
when OFF (feature must not leak into the lean bundle).

### 9. bind_join sort-based run grouping (P3, query)

**Approach.** Replace the `FxHashMap<Id, Vec<usize>>` grouping (one Vec allocation per
distinct join value) with sort-by-join-column + contiguous-run iteration — the result
side is frequently already sorted on the join var via `sorted_by` — enabling batched
bound scans instead of a fresh `store.scan` (permutation `choose` + binary-search
bounds) per distinct value.

**Measurement plan.** Canonical: EC2 SP2B/op trend plus a targeted high-NDV bind-join
microbench (criterion) showing allocation + latency deltas. Correctness: result-set
equality suites; join kernels' monomorphic no-dyn-dispatch contract untouched.

### 10. Chunked CSV/TSV SELECT streaming (P3, http)

**Approach.** SELECT XML/CSV/TSV currently buffer one big String. CSV/TSV are
row-oriented and chunk exactly like the JSON path — reuse the `chunked_response`
machinery (Vec of chunks + known Content-Length + PinnedGen moved into the stream).
Graph formats with prefix compaction (Turtle/RDF-XML) stay buffered — streaming them is
not worth the complexity. XML optional, only if it falls out naturally.

**Measurement plan.** Depends on bead 5: peak-RSS-while-serving-large-export series,
before/after, EC2. Hard guards: byte-identical bodies vs the buffered path (differential
test), hardening middleware untouched.

### 11. "Wave D" pull-streaming body — design record first (P3, http)

**Approach.** The codebase itself flags true lazy streaming as future Wave D: replace
the eager `Vec<String>` (whole result serialized before the first byte) with a bounded
mpsc channel fed from the `spawn_blocking` engine iterator, consumed via
`Body::from_stream`. Cuts TTFB and peak memory from O(full result) to O(chunk). Costs:
chunked transfer-encoding (loses up-front Content-Length), a blocking<->async boundary
crossing, budget/cancellation semantics mid-stream, PinnedGen lifetime across the body.
Too many coupled decisions for a drive-by — this bead produces a design record
(`research/`) + decomposed impl beads, informed by item 6's landed shape.

**Measurement plan (for the eventual impl).** Bead-5 harness: TTFB + peak RSS on the
large-SELECT scenario. The design record must specify the Content-Length contract
change explicitly (it is client-visible).

**Design record produced (sq-7d3dj.13):** `research/wave-d-pull-streaming-response-body.md`
— the chunk-sink producer seam, bounded-channel transport + `PinnedGen` lifetime, the
`Content-Length` → chunked transfer-encoding contract change, the mid-stream
truncation-safety invariant, slow-client admission on `live_generations()`, and the
D1–D6 impl decomposition (each wired to the `sq-7d3dj.23` TTFB series).

### 12. MEASURE-FIRST: sparsify native numerics cache (P3, memory)

**Approach + honest framing.** Native `build()` keeps a dense 8 B/term f64 vec even on
mostly-string datasets (temporals already sparsify via `into_sparse_if_worthwhile`).
The dense layout is a **deliberate** speed tradeoff: `NumData::lookup` is an O(1) array
index on the numeric FILTER/ORDER BY fast path, and sparsifying makes it a hashmap
probe. Do NOT flip blind. This bead is an EC2 A/B: numeric-heavy vs string-heavy query
workloads (op_filter-numeric, ORDER BY, MIN/MAX latency) x `Graph::heap_bytes`
footprint. Adopt (possibly with the <25% numeric-density heuristic) only if the numeric
fast path holds within noise; otherwise close the bead with the measurement recorded.

**Measurement plan.** EC2 A/B as above. Gates: none moved either way
(numerics bytes are outside the ratcheted metrics — footprint shows only in
`Graph::heap_bytes`); a footprint claim must cite that accessor, not RSS on this box.

### 13. MEASURE-FIRST A/B pair: NT intern/scan hot loop (P3, ingest)

**Approach.** Two independently-gated micro-experiments on the shipped custom
N-Triples path, both keep-only-on-measured-win (the rust-parallel-parsing skill's
rejected-ideas discipline): (a) a 1-3 entry recently-seen-term cache keyed on the raw
term byte-slice, checked before hash+probe — real dumps are subject-grouped and
predicate-repetitive, so immediate repeats can skip the dominant `find_iri`
hash/probe/memcmp (~30% of shipped-path ingest per the directional profile); expected
~0 on shuffled data, so the A/B must use a real grouped dump (wikidata/DBpedia slice).
(b) memchr for `scan_delim`/`skip_ws` (the Turtle chunker already uses memchr for the
same job; IRIs cannot contain an unescaped `>`, so a close-delim memchr + `\\` scan
over the span is sound). Keep the scalar path if short terms show no win.

**Measurement plan.** bench/parse A/B (MB/s, Mtriples/s at 1 + 8 threads) on EC2 with a
real grouped dump AND a shuffled control; canonical `parse_ns_per_byte` on the pinned
corpus — an advisory timing signal (tracked/warned, non-blocking) — watched for
regression. Hard guards: exact term<->id bijection + byte-identical
output through the full differential suite (interning order is determinism-audited) —
the cache must not change interned ids.

### 14. Shipped-bundle-size trend series (P3, wasm)

**The measurement-fidelity judgment call.** The `wasm_bundle_bytes` ratchet measures the
RAW pre-wasm-opt build, so ~10% of the floor is metadata users never download, and
genuine shipped-size wins (wasm-opt improvements) are invisible to the gate.
**Verdict: keep the raw deterministic gate exactly as is** — moving the gate onto the
wasm-opt'd artifact would put a version-dependent external tool inside a deterministic
merge gate (the flaky-gate failure mode `parse_ns_per_byte` already taught us).
Instead, emit the shipped (`wasm-opt -Oz`) size as an additional **trend-only**
benchmark-data series, pinned to the wasm-opt version in the series metadata. Depends
on item 1 (land against the new profile so the series never reports a phantom win).

**Measurement plan.** Acceptance: series emitted alongside `wasm_bundle_bytes`;
gate behaviour bit-identical.

---

## Already-optimised — do NOT touch (the honest negative space)

The profiles' strongest shared finding: this codebase is already deeply optimised, and
the biggest quality risk in an "optimise everything" program is re-litigating
deliberate tradeoffs. Standing do-not-touch list:

**Measured-rejected (do not re-propose):**
- Radix sort for permutation builds — measured 0.5-0.7x vs rayon par_sort_unstable
  (sq-56z, research/fast-ingestion.md).
- A faster hand-rolled NT parser — the custom byte parser already beats oxttl;
  rejected-ideas list in the rust-parallel-parsing skill.
- Blanket wasm `opt-level = "z"` — erodes the simd128/hot-engine investment (item 1
  takes the surgical form instead).

**Deliberate design, load-bearing (changing them is a regression, not an opt):**
- Row/Key/Posting SmallVec inline widths (4/2/2) — sized to the common case, locked by
  tests. No re-tuning without measurement.
- Substrate join kernels' monomorphic, `#[inline]`, zero-dyn-dispatch contract
  (checked by `scripts/check-no-dyn-dispatch.py`).
- No hand-written SIMD — M4 kernels are deliberately plain index loops the compiler
  auto-vectorises under simd128/AVX/NEON.
- FxHash throughout — no hasher swaps.
- Single-writer update sequencing in sparq-server — it is what makes the lock-free
  MVCC read path possible. Not a defect.
- HTTP/1-only — a deliberate attack-surface scope choice; HTTP/2 is a feature
  decision, not a throughput bug.
- Dict blob-mode staying browser-only and dense native numerics (until item 12's A/B
  says otherwise) — both are speed/cache tradeoffs, not oversights.
- JSON-LD / RDF/XML whole-document serial parsing — single JSON/XML documents are not
  chunkable; "parallelising" them would be unsound. The real JSON-LD work is
  conformance (epic sq-oy1f).
- Dense-array numeric fast path, inline-integer ids, O(1) numeric/temporal caches,
  COUNT no-materialisation fast paths, streaming id->JSON, parallel dict
  consolidation, fused decompress+parse, prefetch-tuned remap, Arc-structural
  fork/snapshot — all done; the profiles enumerate them so nobody re-proposes them.

**Real but dropped as premature micro-opts (unmeasured, tiny, or low value-to-risk):**
- Metrics request-counter Mutex -> atomics table: the critical section is a tiny
  BTreeMap increment; cleanliness, not a bottleneck. Revisit only if bead-5 harness
  shows contention at high concurrency.
- Accept-header per-range lowercase allocation (1-3 small allocs/request).
- /sparql redundant query-string re-parsing (up to 5 scans of a short string) + eager
  `url_using` on GET — legitimate, small; fold into any future handler refactor rather
  than a dedicated perf bead.
- rand_chacha/rand_core (~0.9% of wasm code) and dlmalloc swap — low reward, upstream
  or allocator correctness risk.
- Hand-deduping hashbrown monomorphisations or rewriting spargebra/oxiri/oxttl
  internals — upstream oxigraph correctness code.
- Float-formatting (dragon) weight in wasm — intrinsic to decimal/double literal
  output correctness.
- LocalVocab single-storage interner — real (~2x computed-term bytes) but per-query,
  already byte-capped; only matters for queries computing many large distinct
  literals. Not beaded now; revisit if a workload surfaces.
- Sibling-permutation nested-parallel sorts — only matters at very high core counts
  and risks rayon oversubscription regressions on normal boxes; revisit if the
  192-core spill box (sq-bj3) profile shows the sibling phase binding.
- Sharded final-Dict lookup table (removing the serial `build_table` insert tail) —
  HIGH-risk architectural change to the query-time lookup representation; only worth
  designing if a large in-memory load profile shows `build_table` as the binding
  constraint.

---

## Measurement discipline (applies to every bead above)

1. **Canonical or it did not happen**: deterministic perf-gate metrics for anything
   byte-countable; EC2 benchmark runs (sq-vw3ax.12) for latency/throughput; work-box
   numbers locate hot spots only. This document's bracket figures are non-canonical.
2. **Ratchets hold or improve** on every PR: every `mode: auto` metric in
   `bench/perf-baseline.json` hard-fails the gate on regression — the ones this document
   touches are `wasm_bundle_bytes`, `store_bytes_per_triple[_small]` and
   `dict_bytes_per_term`. `parse_ns_per_byte` is the sole `mode: noise` metric: its floor
   still ratchets down on sustained improvement, but a regression is an advisory timing
   warning (tracked/warned, non-blocking). Item 1 is the only deliberate re-baseline, and
   it moves DOWN.
3. **Differential-or-die** for anything near semantics: ingest byte-identity suites,
   W3C conformance, ON==OFF feature differentials, byte-identical HTTP bodies.
4. **Measure-first beads (12, 13) may close as "measured, rejected"** — a recorded
   negative is a successful outcome (empirical-honesty rule); the profiles' rejected
   list above exists because previous negatives were recorded.
