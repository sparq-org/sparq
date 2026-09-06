<!-- [OPUS-4.8] (sq-lfo84) ANN distance-kernel SIMD evaluation + fix record. Companion to
     research/gap-vector-2026-07.md (the sq-z2z18 sweep that root-caused the gap). All timing
     rows NON-CANONICAL: aarch64 EC2 work box, no AVX2 — directional ranking only. -->

# ANN distance-kernel SIMD — evaluation + fix (2026-07)

**Bead:** `sq-lfo84` (P1, HNSW QPS/build gap vs hnswlib) + `sq-ose80` (build-time sibling).
**Root cause (from `research/gap-vector-2026-07.md` §3.6/§7 + the sq-z2z18 sweep):** the
`instant-distance` 0.6.1 HNSW backend ships **no SIMD distance kernel** — its inner
squared-Euclidean loop is scalar — so a 1M×128 build was aborted at >30min (hnswlib: 374s).

> **[OPUS-4.8] (sq-ose80) CORRECTION — the build is NOT serial.** An earlier draft of this record
> (and §4 below, kept for provenance) attributed the build-time gap to "serial graph-construction".
> The **sq-ose80 profiling refutes that**: `instant-distance` 0.6.1 **does** build in parallel —
> `Hnsw::new` inserts each layer with rayon's `into_par_iter` (`src/lib.rs:317`), per-node `RwLock`s,
> and a `SearchPool`. A `perf record` of a 200k build (`sq-ose80`, aarch64, NON-CANONICAL) put
> **68.6% of samples in `Search::push`** — the greedy distance search inside the *parallel* insert —
> at ~448% of 800% CPU (i.e. ~56% parallel efficiency, super-linear scaling as N grows). So the gap
> is **compute-bound distance search in an already-parallel build with imperfect scaling**, NOT a
> missing-parallelism bug. See §7 for the full sq-ose80 evaluation and the shipped fix.

<!-- separator: two distinct callouts (MD028) -->

> **NON-CANONICAL throughout.** Every timing below was collected on an **aarch64 EC2 work box
> (no AVX2)**. Absolute figures are not canonical; the matched-recall *ranking* between kernels is
> stable across ISAs. The x86_64 AVX2 kernel added here is type-checked + clippy-clean on the
> `x86_64-unknown-linux-gnu` target but its numeric output is verified only by the in-tree unit
> test on the CI x86_64 runner (this box cannot execute AVX2).

---

## 1. Options evaluated (Phase 1)

| Option | New dep | Build weight | SIMD | Per-query ef | API fit | Verdict |
|---|---|---|---|---|---|---|
| **(a) SIMD kernel for `instant-distance`** (NEON aarch64 / AVX2 x86_64 in the user-supplied `Point::distance`) | **none** | none (Cargo.lock unchanged) | yes | no (unchanged — still build-time) | **exact** (public API identical) | **CHOSEN** |
| (b1) `usearch` 2.25.3 | `cxx` **+ a C++ toolchain build** (`cxx-build`, vendored C++ core) | HEAVY (native C++ compile) | yes (SimSIMD) | yes | needs a wrapper; different index type | rejected — C++ build dep violates lean-native-build; would need its own feature |
| (b2) `hnsw_rs` 0.3.4 | pure Rust but **15+ transitive crates** (rayon, serde, bincode, mmap-rs, anndists, indexmap, hashbrown, parking_lot, env_logger, …) | MEDIUM-HEAVY | yes (`anndists`) | **yes** | needs a `VectorIndex` rewrite | rejected here — fat tree + no matched-recall QPS win on this box (see §3); revisit for the ef-sweep + parallel-build story (sq-ose80) |

Licences are all clean (usearch Apache-2.0; hnsw_rs MIT/Apache-2.0; every transitive crate already
on `deny.toml`'s permissive allowlist) — licence was **not** the discriminator. The discriminators
were **build weight** (option a adds zero; usearch adds a C++ toolchain; hnsw_rs adds a fat tree)
and, on this box, **matched-recall QPS** (§3).

## 2. Chosen fix (Phase 2)

A shared `crates/sparq-vectors/src/simd.rs` (`approx-ann`-only, **no new dependency** — pure
`core::arch` intrinsics behind `#[cfg]`) exposing `l2_sq_dist(a,b)`:

- **NEON** (aarch64): four 128-bit FMA accumulators (16 f32/iter), 4-wide drain, scalar tail.
- **AVX2+FMA** (x86_64): two 256-bit FMA accumulators (16 f32/iter), 8-wide drain, scalar tail.
- **Scalar fallback**: bit-identical to the previous 8-lane auto-vectorised loop.
- Runtime `is_*_feature_detected!` gates every `unsafe` intrinsic entry; each reads exactly
  `len` lanes (no over-read). Wired only into HNSW's `NPoint::distance` (`.sqrt()` of the kernel).

**Why HNSW-only, not the exact/DiskANN/PQ paths:** those three are **EXACT-gated** against
`bench/vector/expected.tsv` (diskann=34, pq=22), and CI runs the gate on **x86_64**. A SIMD
reduction changes the float-sum order → can shift an integer recall deficit by ±1 → would break
the exact-gate on a runner this box cannot reproduce. HNSW is **floor-gated** only (its build is
already rayon-parallel + non-deterministic), so it tolerates the rounding change. Leaving the
deterministic paths on the scalar reduction keeps `expected.tsv` byte-stable. Extending the kernel
to the exact/DiskANN/PQ paths (with an x86_64 `expected.tsv` re-measurement) is the follow-up
**sq-9wrkc**.

## 3. Measured recall–QPS + build time (NON-CANONICAL, aarch64)

Harness: standalone `instant-distance` build with the scalar vs the NEON `Point::distance`, plus a
`hnsw_rs` reference. 100k×128, clustered corpus (the regime real embeddings live in), build
`ef_construction=200`, `ef_search=256`, 1000 queries, k=10.

| Backend | Build (s) | recall@10 | QPS (µs/q) | Notes |
|---|---|---|---|---|
| instant-distance **scalar** (before) | 31.3 | 1.0000 | 2 873 (348 µs) | the committed baseline kernel |
| instant-distance **NEON** (this PR) | **13.2** | 1.0000 | **3 431** (291 µs) | **~2.4× build, ~1.2× query**, recall equal on this corpus |
| hnsw_rs 0.3.4 (reference) | 8.9 | ≤1.0 per ef | ef256: 1 356 | fastest build (parallel_insert) but **lower matched-recall QPS** here |

**Honest reading.** The clustered corpus saturates recall (1.0 both sides), so this table shows the
**kernel-swap effect** (build + throughput), not a recall trade. The NEON kernel roughly **halves
build time and lifts QPS ~1.2×** on the *search* path. **Recall precision caveat:** the SIMD kernels
use FMA (one rounding per term) while the scalar path does `d*d` then `+=` (two roundings), so a SIMD
squared distance is **not bit-identical** to the scalar one — it differs by ≤1 ULP. Rankings are
therefore stable *up to exact near-ties*, which a ≤1-ULP wobble can reorder; that residual is exactly
what the HNSW workload's **floor gate** (recall@10 ≥ 0.95, `tests/recall.rs`) absorbs — the gate is a
recall floor, not a bit-identity assertion. Measured recall was `1.0000` on both kernels here (a
saturating corpus). The scalar *fallback* alone IS bit-identical to the previous auto-vectorised loop
(so a non-SIMD target is numerically unchanged). The bigger win is on **build** because the build
evaluates the distance far more often than search.

**hnsw_rs did NOT win** on this box at matched recall — its per-query ef sweep gives a real Pareto
curve (an advantage `instant-distance` lacks, cf. sq-jo6ty) but its QPS at ef≥64 sat below
instant-distance+NEON, and it drags a 15-crate tree. So a swap was **not** justified by measurement.

## 4. Honest gap vs hnswlib (still BEHIND, narrowed)

The `research/gap-vector-2026-07.md` §3.5 hnswlib figures (NON-CANONICAL, same box) put hnswlib at
5 521 QPS @ recall 0.989 (ef=128) and 2 786 @ 0.997 (ef=256) on **SIFT1M** (1M vectors). This PR's
NEON kernel measured **3 431 QPS on a 100k clustered corpus** — not directly comparable
(different N, different corpus hardness, no per-query ef sweep on the sparq side yet), so **no
matched-recall gap number is claimed here**. What is honestly established:

- **BEHIND-but-improved.** The scalar→NEON kernel closes part of the *per-distance* gap
  (instant-distance's missing SIMD kernel was one of the two root causes). The QPS lift is ~1.2× on
  search and ~2.4× on build on this box.
- **The build-time gap (sq-ose80) is addressed by §7, not by a backend swap.** This §4 draft
  guessed the residual build cost was *serial graph-construction* and that the fix was a
  parallel-build backend (`hnsw_rs::parallel_insert`/usearch). **The sq-ose80 profiling (§7)
  overturns both halves of that guess:** the build is already parallel, and `hnsw_rs`'s
  parallel_insert is measurably **SLOWER** than instant-distance on this box (§7.2). The real,
  measured build lever is `ef_construction`; the shipped fix is the `HnswConfig::fast_build` preset
  (§7.3). The ">14 min hard-uniform 100k" figure above reflects `ef_construction=200` (the sweep
  config), not the default 100 — see §7.1.

## 5. Recommendation for the maintainer

1. **Ship option (a)** (this PR): a free (zero-dep), API-preserving, recall-preserving win on both
   build and query, entirely within lean-core discipline.
2. **Re-decide the backend swap on a canonical x86_64 AVX2 box**, not this aarch64 box — hnswlib's
   published dominance is an AVX2 story, and `hnsw_rs`/`usearch` both have the two things
   instant-distance lacks (per-query ef + parallel build). If a swap is greenlit it must be
   **behind its own opt-in feature** (usearch's C++ build especially) so the default stays lean.
   Tracked: **sq-ose80** (build-time / parallel-build backend) + **sq-jo6ty** (per-query ef).
3. **Follow-up sq-9wrkc:** extend the SIMD kernel to the exact/DiskANN/PQ `sq_dist` loops after an
   x86_64 `expected.tsv` re-measurement (the deterministic-gate constraint that scoped this PR).

---

## 7. sq-ose80 — build-time gap: evaluate-then-implement

**Goal:** close the 1M×128 HNSW build gap toward hnswlib's ~374s without breaking recall or the
query path. All timings **NON-CANONICAL** (aarch64 8-core EC2 work box, no AVX2), cosine on
L2-normalised SIFT1M; recall scored against the crate's own `nearest_exact` cosine oracle (the
`tests/recall.rs` oracle). Harness: `crates/sparq-vectors/examples/` build/recall profilers (kept
out of the committed tree — regenerable bench code).

### 7.1 Profiling — where the 1M build time goes

The premise in the sq-z2z18 finding ("`instant-distance` lacks rayon parallelism on insertion") is
**wrong for 0.6.1**: `Hnsw::new` inserts each layer with `into_par_iter` over per-node `RwLock`s.
Measured build scaling (`ef_search=100`, `ef_construction=200` — the sweep config, seed fixed; the
`Default` is `ef_construction=100`, so these times are an upper bound on the default's):

| N | store-load | HNSW build | throughput | CPU% (of 800%) |
|---|---|---|---|---|
| 50k | 0.02s | 8.2s | 6093 vec/s | — |
| 100k | 0.05s | 20.1s | 4977 vec/s | — |
| 200k | 0.10s | 94.0s | 2129 vec/s | **448%** |

Store-load is negligible; the HNSW build dominates and scales **super-linearly** — throughput
*collapses* 6093 → 2129 vec/s from 50k → 200k. A `perf record` of the 200k build put **68.6% of
self-samples in `instant_distance::Search::push`** (the greedy distance search *inside* the rayon
parallel insert), with the remaining ~31% in rayon idle/steal (`wait_until_cold`). So the build is
**compute-bound on the distance search**, at ~56% parallel efficiency; the super-linear collapse is
the HNSW-at-scale cache signature (random 512-byte vector gathers across a growing >0.5 GB working
set). The distance kernel is already SIMD (sq-lfo84), so the remaining lever is **how much distance
search each insert does** — i.e. `ef_construction`.

### 7.2 Option evaluation (re-weighed for BUILD, per the bead)

| Option | New dep | Build weight | 100k build (this box) | 200k build | Verdict |
|---|---|---|---|---|---|
| **(a) wrapper parallel-insert over instant-distance** | none | none | — | — | **moot** — the build is *already* rayon-parallel; a hand-rolled parallel insert would risk HNSW ordering/concurrency correctness for no gain |
| **(b) `hnsw_rs` 0.3.4 (`parallel_insert`)** | **123-line dep tree** (`anndists`, `env_logger`, `regex`, `aho-corasick`, `jiff`, `bincode`, `serde`+`serde_derive`+`syn`, …) | HEAVY | **47.8s** @ 273% CPU | **111.1s** | **rejected on BOTH axes** — heavier tree AND *slower* build than instant-distance here (2.4× slower @100k, 1.2× slower @200k) |
| **(c) keep instant-distance; tune `ef_construction`** | none | none | 20.1s (efc=100) | 91.5s (efc=100) | **CHOSEN** — the only lean, recall-preserving lever; see §7.3 |
| instant-distance (baseline default, efc=100) | none | none | 20.1s | 91.5s | reference |

Licences are all clean (`hnsw_rs` MIT/Apache-2.0; every transitive crate already on `deny.toml`'s
allowlist) — licence was again **not** the discriminator. The discriminators were **build weight**
and, decisively, **measured build speed**: `hnsw_rs`'s `parallel_insert` is *slower* than
instant-distance's already-parallel build on this box, so option (b) buys a 123-crate dependency
tree for a build-time *regression*. `usearch` (a C++ toolchain) is rejected a fortiori (it was
already rejected in §1 on native-build weight; nothing here re-opens it). No hybrid (c-query /
b-build) is needed because instant-distance wins the build outright.

### 7.3 The `ef_construction` lever + shipped fix (option c)

`ef_construction` is the greedy-search beam width during insertion — the exact quantity §7.1
root-caused. Measured build / recall trade at 200k (cosine, 500 queries, k=10):

| `ef_construction` | build (200k) | throughput | recall@10 | deficit |
|---|---|---|---|---|
| **40 (`fast_build`)** | **31.0s** | 6444 vec/s | 0.9944 | 28 / 5000 |
| 100 (`Default`) | 91.5s | 2186 vec/s | 0.9990 | 5 / 5000 |
| 200 (`high_recall`, = the sweep config that produced the ">30min at 1M" figure) | 137.3s | 1456 vec/s | 0.9992 | 4 / 5000 |

`ef_construction=40` builds **~3× faster than the default and ~4.4× faster than the efc=200 config
that produced the cited >30-min-at-1M abort**, at recall@10 = 0.9944 — comfortably above the 0.95
floor. 100 → 200 doubles build cost for +0.0002 recall (deep diminishing returns).

**1M×128 headline (NON-CANONICAL).** A full 1M build+recall run was **abandoned** on this shared
work box under a **disk-pressure emergency** (the box hit 99% during the run — other agents'
`target/` trees, not this measurement). The 200k curve is the sound, complete basis for the fix; the
1M point is left as an honest extrapolation, not a measured number:

- The measured 50k→200k throughput collapse (6093→2129 vec/s at efc=200) is super-linear, so a
  1M efc=200 build extrapolates to roughly **25–35 min** — consistent with the original ">30 min,
  aborted" observation (which used efc=200).
- At **efc=40 (`fast_build`)** the 200k build ran at 6444 vec/s (≈3× the efc=100 rate). Applying the
  same ~3× factor to a 1M build puts `fast_build` **materially below** the efc=200 abort — plausibly
  in the several-minutes range, i.e. *approaching* hnswlib's ~374s territory — but this is an
  **extrapolation from 200k, not a 1M measurement**, and is explicitly NOT claimed as a parity
  result. A canonical 1M build/recall number belongs to the quiet-box re-run (`sq-hmd7l.26`); a
  build-time-only 1M re-measure is `sq-ose80.2` (below).

> **[SONNET-4.6] (sq-pm6i2) Status: the extrapolation above is STILL an extrapolation.** The
> harness the re-measure needs is now committed — `crates/sparq-vectors/examples/hnsw_build_scaling`
> (runbook: `bench/vector/README.md`) — which times `VectorIndex::build_with` ONLY, dropping the
> `nearest_exact` oracle that made the abandoned run expensive, and streams the corpus into the
> `.spqv` store (with `$SPARQ_VECTORS_TMP`) so it does not repeat the disk-pressure failure. The
> **measurement itself has NOT been taken**: it requires the SIFT1M base vectors (not
> redistributable in-repo) on the dedicated quiet bench box, and a build time from any other box
> would be non-canonical — i.e. it would not replace this extrapolation. Until that run lands, the
> 1M figures in this section remain extrapolated and must not be cited as measured. (The §7.3
> table itself is a 200k measurement and is unaffected.)

**Shipped fix (`crates/sparq-vectors/src/ann.rs`):** two opt-in `HnswConfig` presets —
`HnswConfig::fast_build()` (`ef_construction=40`) and `HnswConfig::high_recall()`
(`ef_construction=200`) — sharing the default's `ef_search`+`seed`. Pure config: **no new
dependency, no default change** (existing callers keep the exact same graph + recall), **build stays
deterministic** for a fixed seed, and the `nearest`/`nearest_with_ef` query path + monotone-recall
contract + the exact/DiskANN/PQ paths are untouched. The recall floor is asserted for `fast_build`
by `tests/recall.rs::build_time_presets_preserve_the_recall_floor` (real build path, not a mock).

### 7.4 Honest verdict

- **No clean way to reach hnswlib's ~374s within lean-core discipline exists via a library swap** —
  instant-distance is already parallel and beats the only pure-Rust parallel-build alternative
  (`hnsw_rs`) on this box, and usearch's C++ toolchain violates the lean-native-build constraint.
- **The shipped `fast_build` preset is a real, recall-preserving ~3× build-time win** (vs default;
  ~4.4× vs the efc=200 sweep config), entirely dependency-free.
- **The residual gap to hnswlib is algorithmic/ISA (AVX2) + implementation efficiency, not a fixable
  parallelism bug.** A canonical x86_64 AVX2 re-measurement (`sq-hmd7l.26`) is the right place to
  quantify what remains; a backend swap should only be reconsidered there, behind its own opt-in
  feature, and only if it actually wins at matched recall — which `hnsw_rs` did not here.

---

## 6. Discovered beads

- **sq-9wrkc** (P2): extend the `simd::l2_sq_dist` kernel to the exact `cosine` / DiskANN Vamana /
  PQ `sq_dist` loops. Blocked on re-measuring `bench/vector/expected.tsv` on an x86_64 runner (the
  AVX2 float-sum order can shift the EXACT-gated diskann/pq deficits by ±1).
- **sq-ose80** — [OPUS-4.8] the >30min-at-1M figure was the `ef_construction=200` sweep config; the
  build is *already parallel* (not serial), the gap is compute-bound distance search at ~56% parallel
  efficiency, and the shipped `HnswConfig::fast_build` (efc=40) preset is a ~3× recall-preserving
  build win (§7). `hnsw_rs` was re-evaluated for BUILD and rejected (heavier AND slower here). The
  residual gap to hnswlib is ISA/algorithmic and belongs to the canonical x86_64 re-run (sq-hmd7l.26).
- **sq-ose80.1** (P2, NEW): `VectorIndex::build_with` clones the whole normalised point set
  (`points.clone()`, ~512 MB at 1M×128) to feed `instant-distance::Builder::build` while also
  retaining it for the lazy `nearest_with_ef` secondary maps — a peak-memory doubling + one large
  memcpy per build. Investigate building from `Arc<[NPoint]>` / a single shared buffer, or only
  retaining the points when a secondary-ef build is actually requested.
- **sq-ose80.2** (P3, NEW): re-measure the **1M×128 SIFT** build time (efc=40 vs 100 vs 200,
  build-time-ONLY — no brute-force oracle), to replace the §7.3 extrapolation with a measured 1M
  point. The oracle-bound recall harness is the slow part; a build-only timer avoids it. The
  canonical recall+QPS 1M run stays `sq-hmd7l.26`.
  **[SONNET-4.6] (sq-pm6i2) Harness landed, measurement OPEN:** the build-only timer is committed
  as `crates/sparq-vectors/examples/hnsw_build_scaling` (`--smoke` self-tests it without a
  dataset). What remains is purely an *execution* step — run it over SIFT1M on the **canonical
  quiet box**, since a build time measured anywhere else cannot replace the extrapolation.

---

*SPARQ agent (Opus 4.8) [OPUS-4.8] · bead sq-lfo84 · NON-CANONICAL aarch64 work-box measurement.*
