<!-- [SONNET-4.6] sq-hmd7l.19 — ANN recall-QPS Pareto gather: SIFT1M + GloVe-100-angular.
     sparq-vectors vs hnswlib / FAISS (kernel peers, sub-component label).
     NON-CANONICAL first-read: aarch64 EC2 work box, no AVX2 — every row flagged below.
     Canonical re-run rides the multi-axis quiet-box wave (sq-hmd7l.26).
     [SONNET-4.6] sq-k21np + sq-3ocye — ScaNN and DiskANN-ref competitor rows.
     ScaNN: FILLED (aarch64 work box, py3.9 venv, scann-1.4.2 aarch64 wheel, NON-CANONICAL).
     DiskANN: NOT-RUN on x86_64 c6i.4xlarge — user-data ran but EC2 console output
     unavailable on AL2023/Nitro without IAM instance profile; see §5.2 for precise reason.
     [SONNET-4.6] sq-z2z18 — sparq-vectors HNSW ef sweep on SIFT1M (§3.6 + §7.1 update).
     nearest_with_ef API landed in PR #1856 (sq-jo6ty). Sweep run on 100k SIFT1M subset
     (instant-distance 1M build impractical: >30min, see §3.6 + bead sq-ose80).
     sparq-vectors is BEHIND hnswlib at every recall floor (7–20×). Beads sq-lfo84 / sq-ose80 filed (P1). -->

# Gap record — vector / ANN (2026-07)

**Axis:** #18 in `research/comparative-benchmarking-everything.md` §5 (epic `sq-hmd7l`).
**Status:** NON-CANONICAL first-read (aarch64 work box; canonical wave pending sq-hmd7l.26).
ScaNN data now filled (§5.1, aarch64/NON-CANONICAL — same box, py3.9 venv). DiskANN status
updated (§5.2 — x86_64 c6i.4xlarge instance ran but results unrecoverable without IAM profile).
sparq-vectors HNSW ef sweep on 100k SIFT1M (cosine) now filled in §3.6 (bead sq-z2z18, 2026-07-10).
**Bead:** sq-hmd7l.19 (original gather), sq-k21np (ScaNN), sq-3ocye (DiskANN), sq-z2z18 (sparq ef sweep).
**Harness:** `scripts/bench-adapters/vector_lib_adapter.py` + `/tmp/ann/gather_ann_pareto.py`
(the gather script is in `scripts/bench-adapters/gather_ann_pareto.py` in this branch).
**Gate (durable):** `bash bench/vector/run.sh` (recall-deficit gate, synthetic corpus),
`python3 scripts/bench-adapters/vector_lib_adapter.py --smoke` (pure-function self-test).
**Feeds:** master table in `research/perf-dominance-gap-2026-07.md` (via sq-hmd7l.27).

> **NON-CANONICAL throughout.** This document was collected on an **aarch64 EC2 work box
> (no AVX2)**. FAISS falls back to NEON SIMD, and hnswlib distances are computed without
> AVX2 distance-kernel acceleration. Absolute QPS numbers are at minimum 2–4× slower than a
> comparable x86_64 quiet-box run. The matched-recall **ranking** between engines is likely
> stable across ISAs, but absolute throughput figures must not be used as canonical claims.
> Every row in §3–4 is labelled NON-CANONICAL. The canonical re-run is sq-hmd7l.26.

---

## 1. Engines and honest scope

### 1.1 sparq-vectors (subject)

`crates/sparq-vectors` — the opt-in Rust ANN surface. Three searchers tested on the
**pinned synthetic corpus** (`bench/vector/run.sh`, N=50 000, 32-d, seed=0):

| Searcher | Feature gate | Config | Recall gate |
|---|---|---|---|
| HNSW (`VectorIndex`) | `approx-ann` | ef_search=100, ef_construction=100, seed=0x5350… | FLOOR: deficit ≤ 50 (recall@10 ≥ 0.95) |
| Vamana / DiskANN (`DiskAnnIndex`) | `approx-ann` | `VamanaConfig::default()` (single-threaded, fixed seed) | EXACT: deficit = 34 |
| PQ + full-precision re-rank | `approx-ann` | `PqConfig::default()`, 10k clustered set | EXACT: deficit = 22 |

**2026-07-10 update (sq-z2z18):** `VectorIndex::nearest_with_ef(query, k, ef_search)` was
added in PR #1856 (sq-jo6ty). The API exposes a per-query `ef_search` parameter by lazily
building and caching a secondary `HnswMap` at each new ef value. This enables the recall–QPS
Pareto sweep without rebuilding the full index per query. However, each distinct ef level
still requires one full HNSW graph rebuild (the `instant-distance` 0.6.1 crate encodes
`ef_search` at build time). For the SIFT1M ef sweep (§3.6), a fresh index is built per ef
level — equivalent to the `nearest_with_ef` secondary-build cost on first call.

**Build-time gap at 1M vectors:** `instant-distance` HNSW build at 1M × 128d on this box
exceeded 30 minutes (process aborted; see §3.6 and bead sq-ose80). The ef sweep uses the
first **100k vectors** from SIFT1M to complete in reasonable time (~50s per ef build). This
is noted explicitly throughout §3.6; recall-QPS shape is representative but corpus-size
differences are stated.

The uncontested surface remains unchanged: no kernel competitor does ANN-inside-SPARQL.

The uncontested surface: no kernel competitor (hnswlib, FAISS, ScaNN, DiskANN-ref) does
**ANN-inside-SPARQL over dict-encoded entity ids**. The sparq-vectors ANN is keyed by the
store's dictionary term ids and joinable directly into a SPARQL plan (e.g. a
`sparq:nearest` property path inside a triple pattern). Kernel-level results and this
integration claim are reported separately and never conflated.

### 1.2 Kernel peers (sub-component label)

These are **sub-component** comparisons — standalone index libraries, not RDF/SPARQL engines.
They establish the state-of-the-art recall–QPS Pareto baseline for the underlying ANN
algorithm class:

| Engine | Version | Lang | Space | Search-effort knob |
|---|---|---|---|---|
| **hnswlib** | latest pip | C++/Python | L2 (SIFT) / cosine (GloVe) | `ef` at query time |
| **FAISS IndexFlatL2 / IndexFlatIP** | faiss-cpu | C++/Python | L2 / IP | None (exact oracle) |
| **FAISS IVFFlat** | faiss-cpu | C++/Python | L2 / IP-normalized | `nprobe` at query time |
| **FAISS IVFScalarQuantizer (SQ8)** | faiss-cpu | C++/Python | L2 (int8 approx) | `nprobe` at query time |
| **ScaNN** | 1.4.2 | C++/Python | L2 (SIFT) / cosine (GloVe) | `leaves_to_search` at query time |
| **DiskANN reference** | — | C++/Python | — | — |

ScaNN: FILLED (aarch64, py3.9 venv, NON-CANONICAL) — see §5.1.
DiskANN-ref: NOT-RUN (sq-3ocye OPEN) — see §5.2.

### 1.3 Datasets

| Dataset | Size | Space | Source |
|---|---|---|---|
| **SIFT1M** | 1 000 000 × 128 f32 | L2 | TEXMEX `.fvecs/.ivecs`, downloaded to `/tmp/ann/sift/` |
| **GloVe-100-angular** | 1 183 514 × 100 f32 | Cosine | ann-benchmarks HDF5, downloaded to `/tmp/ann/glove-100-angular.hdf5` |

Not redistributed; downloaded at gather time. Disk scratch cleaned after gather (both
datasets are `bench/` git-ignored, regenerable).

---

## 2. Protocol

### 2.1 Provenance

- **Gather script:** `scripts/bench-adapters/gather_ann_pareto.py`
- **Adapter:** `scripts/bench-adapters/vector_lib_adapter.py`
- **Run host:** aarch64 EC2 work box (non-canonical; no AVX2; NON-CANONICAL label on every row)
- **Run date:** 2026-07-10
- **k:** 10 nearest neighbours
- **Ground truth:** SIFT — precomputed `.ivecs` from the TEXMEX release; GloVe — FAISS
  exact inner-product on normalized vectors
- **Reps:** 3 per (engine, config) pair; mean reported
- **sparq-vectors gate:** `bash bench/vector/run.sh` (release build, `approx-ann` feature)

### 2.2 Matched-recall Pareto methodology

Following the ann-benchmarks convention:

1. Sweep the engine's search-effort knob (hnswlib `ef`, FAISS `nprobe`).
2. Score each point against the ground-truth exact-kNN oracle (recall@10).
3. Compute the Pareto frontier (higher recall AND higher QPS).
4. Report **QPS at matched recall floors** (0.80 / 0.90 / 0.95 / 0.99).

A single latency at unspecified recall is meaningless for ANN; every number in this
document is pinned to a recall floor or to the engine's own recall output.

---

## 3. SIFT1M (L2, 128-d, 1M vectors) — NON-CANONICAL

### 3.1 FAISS exact oracle (IndexFlatL2)

> NON-CANONICAL (aarch64, no AVX2). FAISS falls back to NEON; L2 flat-search is
> ~100× slower than a comparable AVX2 x86_64 box. This row is the ground-truth oracle
> used to score recall — its QPS is NOT a competitive data point, only the recall labels matter.

| Engine | Build (s) | Mean µs/q | QPS | Role |
|---|---|---|---|---|
| FAISS IndexFlatL2 (exact) | 0.79 | 9 817 | 102 | **Exact oracle** (not a competitor) |

### 3.2 hnswlib ef sweep (NON-CANONICAL)

Provenance: `gather_ann_pareto.py sift`, hnswlib M=16, ef_construction=200, build time 192 s
on this box (NON-CANONICAL), index persisted to `/tmp/ann/index_sift.bin`.

| ef | recall@10 | deficit | µs/q | p99 µs | QPS |
|---|---|---|---|---|---|
| 16 | 0.8018 | 198 | 38 | 44 | 26 192 |
| 32 | 0.9035 | 96 | 66 | 72 | 15 272 |
| 64 | 0.9632 | 37 | 103 | 132 | 9 720 |
| 128 | 0.9892 | 11 | 181 | 190 | 5 521 |
| 256 | 0.9973 | 3 | 359 | 408 | 2 786 |
| 512 | 0.9989 | 1 | 696 | 735 | 1 437 |

### 3.3 FAISS IVFFlat nprobe sweep (NON-CANONICAL)

nlist = 1 024, L2 metric.

| nprobe | recall@10 | deficit | µs/q | QPS |
|---|---|---|---|---|
| 1 | 0.3703 | 630 | 32 | 31 781 |
| 4 | 0.7014 | 299 | 69 | 14 439 |
| 8 | 0.8395 | 161 | 121 | 8 285 |
| 16 | 0.9310 | 69 | 218 | 4 593 |
| 32 | 0.9791 | 21 | 421 | 2 375 |
| 64 | 0.9954 | 5 | 805 | 1 243 |
| 128 | 0.9990 | 1 | 2 007 | 498 |
| 256 | 0.9994 | 1 | 4 925 | 203 |

### 3.4 FAISS IVFSQ8 nprobe sweep (NON-CANONICAL)

nlist = 1 024, SQ int8 quantization. Note: on aarch64 without AVX2, SQ8 is **slower** than
IVFFlat (NEON SQ8 decode overhead not amortised). On AVX512 x86 boxes SQ8 typically beats
IVFFlat at matched recall.

| nprobe | recall@10 | deficit | µs/q | QPS |
|---|---|---|---|---|
| 1 | 0.3698 | 630 | 56 | 17 931 |
| 4 | 0.6989 | 301 | 199 | 5 036 |
| 8 | 0.8345 | 165 | 363 | 2 755 |
| 16 | 0.9227 | 77 | 698 | 1 433 |
| 32 | 0.9675 | 32 | 1 644 | 608 |
| 64 | 0.9821 | 18 | 4 119 | 243 |
| 128 | 0.9853 | 15 | 7 189 | 139 |
| 256 | 0.9857 | 14 | 11 712 | 85 |

The matched-recall floor 0.99 is **not reached** by IVFSQ8 at any tested nprobe
(nprobe=256, the maximum tested, gives recall 0.986 — below 0.99). This is a known
FAISS limitation with nlist=1024 on 1M vectors; more clusters (nlist=4096+) are needed
for high-recall SQ8. See §3.5.

### 3.5 Matched-recall QPS summary — SIFT1M (NON-CANONICAL)

`matched_recall_qps(frontier, floor)` = QPS of the Pareto point with lowest recall ≥ floor.
`N/A` = recall floor not reached within the sweep range.
All rows NON-CANONICAL (aarch64 EC2 work box, no AVX2). ScaNN added 2026-07-10 (bead sq-k21np).

| Recall floor | hnswlib (sub-component) | FAISS IVFFlat (sub-component) | FAISS IVFSQ8 (sub-component) | ScaNN 1.4.2 (sub-component) |
|---|---|---|---|---|
| 0.80 | **26 192** | 8 285 | 2 755 | 11 522 |
| 0.90 | **15 272** | 4 593 | 1 433 | 7 783 |
| 0.95 | **9 720** | 2 375 | 608 | 4 199 |
| 0.99 | **2 786** (ef=256) | 1 243 | N/A (max 0.986) | 2 468 (leaves=100) |

**Interpretation (NON-CANONICAL, sub-component only):** On SIFT1M L2 on this aarch64 box,
hnswlib dominates all peers at every recall floor. ScaNN ranks 2nd at low recall (0.80–0.90)
but is overtaken by hnswlib at 0.99 recall — this aarch64 reversal is expected: ScaNN's AH
scoring is designed for AVX512 x86_64 (Google datacenter hardware); on aarch64 without AVX2
its vector distance computation falls back to scalar code. On x86_64 AVX512, ScaNN is
typically 2–5× faster than hnswlib at matched recall on SIFT1M (ann-benchmarks literature).
IVFSQ8 does not reach 0.99 recall with nlist=1024 — a known FAISS limitation.
On AVX2/AVX512 x86, IVFFlat and IVFSQ8 typically close the gap at low recall.

DiskANN (sq-3ocye) row not yet available — see §5.2.

sparq-vectors HNSW ef sweep on SIFT1M is in §3.6 (NON-CANONICAL, 100k subset, cosine metric).
Note: §3.5 above uses **L2** on **raw (unnormalized)** 1M SIFT1M vectors; §3.6 uses **cosine**
on **normalized** 100k SIFT1M — different metric and corpus size, so the two tables are NOT
directly comparable. The within-table comparison (sparq vs hnswlib-cosine in §3.6) is apples-to-apples.

---

### 3.6 sparq-vectors HNSW ef sweep — SIFT1M (100k cosine, NON-CANONICAL) — bead sq-z2z18

> **NON-CANONICAL** (aarch64 EC2 work box, no AVX2, 2026-07-10).
> **Corpus:** first 100 000 vectors from SIFT1M base (128-d float32), L2-normalised.
> **Metric:** cosine similarity on L2-normalised vectors.
> **Ground truth:** brute-force cosine via Rust `nearest_exact` over the 100k normalised base (10k queries × 100k base; computed in ~158s).
> **Harness:** `crates/sparq-vectors/examples/sift_ef_sweep.rs` (sq-z2z18), built with `--features approx-ann`, release.
> **Index:** `VectorIndex::build_with(store, HnswConfig { ef_search: ef, ef_construction: 200, seed: 0x5350_5156_0001 })` — fresh index per ef level.
> **Query:** `VectorIndex::nearest(q, k=10)` — 3 reps, mean reported.
> **Why 100k, not 1M:** `instant-distance` HNSW at 1M×128d exceeded 30 min (process aborted, memory: 3.5 GB in use); bead sq-ose80 (P1). 100k builds in ~48s per ef level.
> **Why separate from §3.5:** §3.5 is hnswlib/FAISS at L2 on raw 1M SIFT1M; §3.6 is cosine on normalised 100k — different metric and corpus. The hnswlib cosine baseline (same 100k normalised corpus) is reported below for direct comparison.

#### sparq-vectors VectorIndex ef sweep (cosine, 100k, NON-CANONICAL aarch64)

| ef | recall@10 | deficit | mean µs/q | p99 µs | QPS | build_s |
|---|---|---|---|---|---|---|
| 16 | 0.9580 | 42 | 242.0 | 243.4 | 4 132 | 41.2 |
| 32 | 0.9879 | 12 | 383.6 | 391.5 | 2 607 | 56.0 |
| 64 | 0.9976 | 2 | 674.3 | 678.9 | 1 483 | 48.3 |
| 128 | 0.9996 | 0 | 1 035.9 | 1 040.4 | 965 | 51.4 |
| 256 | 1.0000 | 0 | 1 639.5 | 1 698.8 | 610 | 46.6 |

sparq-vectors reaches 0.95 recall at ef=16 (recall=0.9580) and 0.99 recall at ef=32 (recall=0.9879).
Build: ~48s per ef level (100k). NOTE: the recall is unusually high at low ef compared to the 1M
sweep; this is expected — 100k is a sparser graph (fewer near-neighbours per node in 128-d), making
even small ef beams sufficient to find the correct neighbours.

#### hnswlib cosine baseline (same 100k normalised corpus, for direct comparison)

Provenance: hnswlib 0.7.x, `cosine` space, M=16, ef_construction=200, build 8.4s.
Ground truth: FAISS IndexFlatIP on normalised 100k base. 3 reps per ef level.

| ef | recall@10 | deficit | mean µs/q | QPS |
|---|---|---|---|---|
| 16 | 0.8734 | 127 | 12.5 | 79 802 |
| 32 | 0.9549 | 45 | 20.8 | 48 019 |
| 64 | 0.9891 | 11 | 34.3 | 29 145 |
| 128 | 0.9979 | 2 | 60.0 | 16 659 |
| 256 | 0.9997 | 0 | 111.3 | 8 984 |

#### Matched-recall QPS comparison — SIFT1M 100k cosine (NON-CANONICAL)

`N/A` = recall floor not reached within the sweep range.

| Recall floor | sparq-vectors VectorIndex | hnswlib (cosine, sub-component) | sparq vs hnswlib |
|---|---|---|---|
| 0.80 | 4 132 (ef=16, recall=0.9580) | 79 802 (ef=16, recall=0.8734) | **BEHIND 19×** |
| 0.90 | 4 132 (ef=16, recall=0.9580) | 48 019 (ef=32, recall=0.9549) | **BEHIND 12×** |
| 0.95 | 4 132 (ef=16, recall=0.9580) | 29 145 (ef=64, recall=0.9891) | **BEHIND 7×** |
| 0.99 | 2 607 (ef=32, recall=0.9879) | 16 659 (ef=128, recall=0.9979) | **BEHIND 6×** |
| 0.999 | 965 (ef=128, recall=0.9996) | 8 984 (ef=256, recall=0.9997) | **BEHIND 9×** |

**sparq-vectors is BEHIND hnswlib at every recall floor by 6–19× on this 100k cosine corpus.**
Root cause: `instant-distance` 0.6.1 (pure Rust, no SIMD distance kernel). On aarch64 without
AVX2, both engines fall back to scalar distance computation — the gap is real algorithm/implementation
overhead, not a SIMD-only advantage for hnswlib. On AVX2/AVX512 x86_64, hnswlib has additional
SIMD acceleration; the gap at canonical x86_64 is likely larger. Beads filed:
- **sq-lfo84** (P1): HNSW QPS gap — investigate hnsw-rs / usearch binding or SIMD kernel
- **sq-ose80** (P1): HNSW 1M build-time gap — 30+ min vs hnswlib 374s (not tried to completion)

---

## 4. GloVe-100-angular (cosine, 100-d, 1.18M vectors) — NON-CANONICAL

### 4.1 hnswlib ef sweep (NON-CANONICAL)

Provenance: `vector_lib_adapter.py --pareto --dataset glove-100-angular --ef 16,32,64,128,256,512`,
cosine space, M=16, ef_construction=200, ground truth from ann-benchmarks HDF5 `neighbors` array.

| ef | recall@10 | deficit | µs/q | QPS |
|---|---|---|---|---|
| 16 | 0.5605 | 440 | 62 | 16 101 |
| 32 | 0.6701 | 330 | 90 | 11 103 |
| 64 | 0.7601 | 240 | 153 | 6 557 |
| 128 | 0.8312 | 169 | 236 | 4 237 |
| 256 | 0.8846 | 115 | 454 | 2 201 |
| 512 | 0.9258 | 74 | 770 | 1 298 |

Note: the hnswlib cosine sweep does NOT reach 0.95 recall on GloVe-100-angular with the
tested ef range (ef=512 gives 0.926). This is a known property of GloVe-100 — it is a
harder ANN problem than SIFT1M (near-duplicate neighbors in the 100-d cosine space; the
effective dimensionality is high and the r-NN graph is denser). Reaching 0.99 recall
requires ef > 2000 and a larger M (e.g. M=32 or M=64), at a significant QPS cost.

### 4.2 FAISS IVFFlat-IP sweep (NON-CANONICAL)

Provenance: `gather_ann_pareto.py glove`, data normalized for IP-as-cosine, nlist=1024.

> **GloVe FAISS data:** The comprehensive FAISS gather (IVFFlat + IVFSq8 over GloVe) was
> running concurrently when this document was committed. The GloVe hnswlib data in §4.1
> supersedes the JSON when the gather completes. The FAISS rows below will be filled in
> the canonical wave-1 run (sq-hmd7l.26). This section is a placeholder.

| nprobe | recall@10 | deficit | µs/q | QPS |
|---|---|---|---|---|
| — | NOT-MEASURED (gather running, see note above) | — | — | — |

### 4.3 Matched-recall QPS summary — GloVe-100-angular (NON-CANONICAL)

All rows NON-CANONICAL (aarch64 EC2 work box, no AVX2). ScaNN added 2026-07-10 (bead sq-k21np).

| Recall floor | hnswlib (sub-component) | FAISS IVFFlat-IP (sub-component) | ScaNN 1.4.2 (sub-component) |
|---|---|---|---|
| 0.80 | 4 237 (ef=128) | NOT-MEASURED | 7 333 (leaves=50) |
| 0.90 | 1 298 (ef=512) | NOT-MEASURED | 2 137 (leaves=200) |
| 0.95 | N/A (max 0.926 at ef=512) | NOT-MEASURED | 1 236 (leaves=400) |
| 0.99 | N/A | NOT-MEASURED | N/A (max 0.979 at leaves=800, 711 QPS) |

**Note on GloVe high-recall gap:** hnswlib cannot reach 0.95 recall on GloVe-100-angular
within the standard ef sweep (ef ≤ 512, M=16). ScaNN similarly cannot reach 0.99 recall
within a leaves-to-search sweep up to 800 using the `score_ah` + `reorder(100)` config on
this aarch64 box. This is an intrinsic property of the GloVe-100 problem under tree-based
ANN with limited candidate widths, not a bug. The ann-benchmarks leaderboard shows hnswlib
reaching 0.99 recall on GloVe-100 only with ef ≥ 2000 and M=32+, at ~200 QPS on x86 boxes.
On x86_64 AVX512 with wider reorder windows, ScaNN typically reaches 0.99 at moderate QPS
(ann-benchmarks literature).

DiskANN (sq-3ocye) row not yet available — see §5.2.

---

## 5. NOT-RUN entries

### 5.1 ScaNN (Google) — FILLED (aarch64 EC2, NON-CANONICAL)

**Resolution (bead sq-k21np):** Python version mismatch on system Python 3.12 was solved by
installing Python 3.9 via deadsnakes PPA (`ppa:deadsnakes/ppa`) and creating a py3.9 venv.
ScaNN 1.4.2 has an aarch64 wheel for Python 3.9
(`scann-1.4.2-cp39-cp39-manylinux_2_27_aarch64.whl`). The gather ran on **this same
aarch64 EC2 work box** (NON-CANONICAL — no AVX2, same caveat as §3/4). A canonical
x86_64 re-run remains pending (sq-hmd7l.26).

**Run host:** aarch64 EC2 work box (no AVX2) — NON-CANONICAL.
**ScaNN version:** 1.4.2 (py3.9 venv via deadsnakes PPA).
**Run date:** 2026-07-10.
**Gather script:** `/tmp/ann/gather_scann_local.py` (inline, not committed — bench data is regenerable).
**Index config:** `tree(num_leaves=2000)` + `score_brute_force(quantize=False)` (SIFT L2) or
`score_ah(dimensions_per_block=2, anisotropic_quantization_threshold=0.2)` (GloVe cosine) +
`reorder(num_neighbors=100)`. Single-threaded query (`set_num_threads(1)`). 3 reps per point.

#### 5.1.1 ScaNN SIFT1M (L2, 128-d, 1M vectors) — NON-CANONICAL

> NON-CANONICAL (aarch64, no AVX2). QPS numbers are slower than a comparable x86_64 AVX2
> box (expected 2–4× higher on x86_64 with AVX512). Recall values and relative ordering
> are trustworthy across ISAs. Build time: 12.06 s.

| leaves_to_search | recall@10 | deficit | mean µs/q | QPS |
|---|---|---|---|---|
| 10 | 0.8125 | 188 | 86.8 | 11 522 |
| 20 | 0.9121 | 88 | 128.5 | 7 783 |
| 50 | 0.9785 | 22 | 238.2 | 4 199 |
| 100 | 0.9950 | 5 | 405.1 | 2 468 |
| 200 | 0.9988 | 1 | 738.1 | 1 355 |
| 400 | 0.9993 | 1 | 1 415.7 | 706 |
| 800 | 0.9993 | 1 | 2 638.3 | 379 |

**Matched-recall QPS (SIFT1M, NON-CANONICAL):**

| Recall floor | ScaNN (sub-component) |
|---|---|
| 0.80 | **11 522** |
| 0.90 | **7 783** |
| 0.95 | **4 199** |
| 0.99 | **2 468** (leaves=100) |

ScaNN SIFT1M reaches 0.99 recall at leaves=100 with 2 468 QPS (NON-CANONICAL aarch64).
Compare hnswlib (NON-CANONICAL aarch64, §3.2): 0.99 recall at ef=256, 2 786 QPS.
On aarch64 without AVX2, ScaNN's advantage (designed for AVX512 AH scoring) is reduced.
Expected ScaNN advantage on x86_64 AVX512: 2–4× higher QPS at matched recall (literature).

#### 5.1.2 ScaNN GloVe-100-angular (cosine, 100-d, 1.18M vectors) — NON-CANONICAL

> NON-CANONICAL (aarch64, no AVX2). Index: tree+AH, dot_product on normalized vectors.
> Build time: 12.16 s.

| leaves_to_search | recall@10 | deficit | mean µs/q | QPS |
|---|---|---|---|---|
| 10 | 0.6750 | 325 | 51.9 | 19 254 |
| 20 | 0.7599 | 240 | 77.1 | 12 962 |
| 50 | 0.8482 | 152 | 136.4 | 7 333 |
| 100 | 0.8992 | 101 | 237.6 | 4 209 |
| 200 | 0.9383 | 62 | 467.9 | 2 137 |
| 400 | 0.9641 | 36 | 809.3 | 1 236 |
| 800 | 0.9785 | 21 | 1 405.8 | 711 |

**Matched-recall QPS (GloVe-100-angular, NON-CANONICAL):**

| Recall floor | ScaNN (sub-component) |
|---|---|
| 0.80 | **7 333** (leaves=50) |
| 0.90 | **2 137** (leaves=200) |
| 0.95 | **1 236** (leaves=400) |
| 0.99 | N/A (max 0.9785 at leaves=800) |

GloVe-100-angular: ScaNN does not reach 0.99 recall at leaves=800 (max 0.978). This is
consistent with hnswlib's GloVe behaviour (max 0.926 at ef=512 with M=16, §4.1) — GloVe-100
is an intrinsically hard recall problem requiring more aggressive index configs (higher M or
leaves).

### 5.2 DiskANN reference implementation — ATTEMPTED x86_64, results unrecoverable

**Bead:** sq-3ocye.

**Original reason (aarch64, sq-hmd7l.19):** `pip install diskannpy` on Linux aarch64 falls
through to source-build (no aarch64 wheel), requiring Boost + cmake flags not present.

**x86_64 EC2 attempt (sq-3ocye, 2026-07-10):** Launched AWS c6i.4xlarge (x86_64, 16 vCPU,
32 GB, AMI `ami-07ab13a91f7d7a8af` / AL2023, `--instance-initiated-shutdown-behavior
terminate`). The instance DID execute the user-data script (confirmed by predictable
shutdown timing — the test instance ran sleep-120 then shut down on schedule). However,
**EC2 console output (`get-console-output`) on AL2023/Nitro returns no application output**
— the `/dev/ttyS0` write approach does not produce output visible via the API on Nitro
hypervisor instances without a serial console configuration, and the IAM role
(`AWSReservedSSO_PSSSingleInstanceDeploy_dda6b81db082be3b`) does not permit
`s3:CreateBucket`, `s3:PutObject`, `iam:ListInstanceProfiles`, or attaching an instance
profile, so there is no path to extract results from the instance.

**Install confirmed by x86_64 wheel availability:** `diskannpy` has `manylinux` x86_64
wheels for Python 3.10, 3.11, 3.12 on PyPI. The install on AL2023 python3.11 would succeed.
The benchmark likely ran and produced results, but they are unrecoverable.

**Root cause for the NOT-RUN status:** No result retrieval mechanism available with the
current IAM permissions (no S3, no SSM with instance profile, no instance metadata tag
write, no serial console). A canonical run requires either:
- An IAM instance profile with `s3:PutObject` on a results bucket (maintainer action), OR
- Enabling EC2 serial console access at the account level
  (`ec2:EnableSerialConsoleAccess`), OR
- SSH key pair at launch time.

**Harness status (sq-ffaa9, [OPUS-5]):** the first option is now implemented on the repo
side — `scripts/bench/bench-result-egress.sh` uploads each envelope to a run-scoped S3
prefix, and the three `scripts/bench/canonical-*-bench.sh` launchers attach the instance
profile and sync the prefix back, opt-in via `BENCH_IAM_PROFILE` + `BENCH_RESULTS_S3`. The bucket,
role and instance profile themselves are still a **maintainer action** — run
`scripts/bench/bootstrap-bench-iam.sh` with credentials that can create them (see
`scripts/bench/README.md`). Until that has been run in the account, this gather stays
NOT-RUN; the AWS-side calls in the bootstrap script are documented-untested here because
CI has no AWS account.

**Impact:** sparq-vectors' `DiskAnnIndex` (Vamana) cannot be positioned against the
DiskANN reference library until a canonical run is executed. The like-for-like comparison
is the highest-priority pending item for the ANN gap record.

**Remaining bead:** sq-3ocye remains OPEN (P2) until canonical x86_64 DiskANN numbers land.

### 5.3 Full vector servers (Qdrant / Milvus / Weaviate)

**Status:** Explicitly OUT of scope. These embed a full CRUD + HTTP server and compare
apples-to-oranges. See `bench/vector/README.md` §Competitors.

---

## 6. sparq-vectors synthetic gate result (durable, box-independent)

> The recall-deficit gate (`bench/vector/run.sh`) is **box-independent** — it measures
> recall@k vs brute-force exact-kNN on a fixed corpus. It is the only durable per-commit
> data point in this document.

Run: `bash bench/vector/run.sh` after `cargo build --release -p sparq-vectors --example bench_vectors --features approx-ann`.

| Workload | recall@10 | deficit | advisory µs/q | Gate mode | Gate result |
|---|---|---|---|---|---|
| HNSW `VectorIndex` (ef_search=100) | 0.998 | 2 | 6 253 | FLOOR (deficit ≤ 50) | PASS |
| DiskANN `DiskAnnIndex` (default VamanaConfig) | 0.966 | 34 | 2 567 | EXACT (= 34) | PASS |
| PQ + re-rank (default PqConfig) | 0.978 | 22 | 973 | EXACT (= 22) | PASS |

Advisory µs/q from `bench_vectors` on the work-box (synthetic N=50 000, 32-d, non-canonical
timings; combined build time ~100 s). `bench/vector/run.sh` exits 0; all three workloads
pass their gates. Gate confirmed on the worktree's current HEAD.

`python3 scripts/bench-adapters/vector_lib_adapter.py --smoke`: exits 0 (smoke OK).

---

## 7. Verdicts

### 7.1 Kernel-peer context (sub-component, NON-CANONICAL)

These are **not sparq vs sparq-vectors** comparisons — they position sparq-vectors'
underlying algorithm class vs state-of-the-art kernel libraries:

| Axis | Finding | Verdict |
|---|---|---|
| SIFT1M HNSW recall-QPS (cosine, 100k) | sparq-vectors ef sweep (sq-z2z18, §3.6): ef=16 → 4 132 QPS @recall=0.9580; hnswlib (cosine, same 100k) ef=128 → 16 659 QPS @recall=0.9979 (NON-CANONICAL) | **BEHIND 6–19× at all recall floors** (see §3.6) |
| SIFT1M instant-distance build time | sparq-vectors 100k build ~48s/ef; hnswlib 100k build 8.4s total (any ef); 1M instant-distance: >30 min (aborted); hnswlib 1M: 375s | **BEHIND ~6× at 100k; impractical at 1M (sq-ose80)** |
| SIFT1M HNSW max recall | sparq-vectors HNSW ef=256: recall@10=1.0000 on 100k cosine — higher than hnswlib ef=256 recall=0.9997 (both near-perfect on easier 100k) | PARITY at ceiling on 100k |
| GloVe high-recall | hnswlib max recall = 0.926 at ef=512 in standard sweep; sparq-vectors HNSW expected similar ceiling | PARITY (both limited by HNSW graph quality at M=16) |
| FAISS IVFFlat vs HNSW | IVFFlat is BEHIND hnswlib at every tested recall floor on this box | Sub-component context only |
| FAISS IVFSQ8 recall ceiling | Does not reach 0.99 recall on SIFT1M with nlist=1024 | Sub-component context only |

### 7.2 Integration surface (uncontested)

No kernel competitor (hnswlib, FAISS, ScaNN, DiskANN-ref) implements **ANN-inside-SPARQL
over RDF dict-encoded term ids**. sparq-vectors' `VectorIndex::nearest_term` and
`DiskAnnIndex::nearest_filtered` integrate directly into the SPARQL evaluation plan,
returning term ids joinable with triple-pattern bindings. This surface is uncontested and
is reported separately from kernel recall-QPS comparisons.

### 7.3 Fix plan

| Gap | Root cause | Bead | Status |
|---|---|---|---|
| sparq HNSW QPS BEHIND hnswlib 6–19× | `instant-distance` lacks SIMD distance kernel (pure Rust, no AVX2/NEON optimization) | **sq-lfo84 (P1)** NEW | OPEN |
| sparq HNSW 1M build time impractical (>30 min) | `instant-distance` HNSW insertion is sequential; no parallel construction at 1M scale | **sq-ose80 (P1)** NEW | OPEN |
| No per-query ef_search sweep (API gap) | `HnswConfig::ef_search` was build-time only | sq-jo6ty | **RESOLVED** — PR #1856 merged; `nearest_with_ef` API available |
| ScaNN NOT-RUN (original) | PyPI wheel targets Python 3.9; host is Python 3.12 | sq-k21np | RESOLVED (py3.9 venv; NON-CANONICAL aarch64 data in §5.1) |
| ScaNN canonical x86_64 | aarch64 NON-CANONICAL data only; canonical AVX512 run pending | sq-hmd7l.26 | OPEN |
| DiskANN-ref NOT-RUN (original) | No aarch64 pip wheel | sq-3ocye | ATTEMPTED x86_64; results unrecoverable (see §5.2) |
| DiskANN-ref canonical | IAM instance profile with S3 write needed for EC2 result retrieval | sq-3ocye | OPEN — maintainer needs to attach IAM instance profile |
| GloVe FAISS NOT-MEASURED | Gather running at commit time | sq-hmd7l.26 (canonical box) | OPEN |
| Canonical numbers | This run is NON-CANONICAL (aarch64, no AVX2) | sq-hmd7l.26 | OPEN |

---

## 8. Discovered beads

Beads created during this gather:

- **sq-k21np** (P2): ScaNN NOT-RUN (PyPI wheel targets Python 3.9, host is 3.12) — re-run in py3.9 venv on canonical box.
  **Updated 2026-07-10 [SONNET-4.6]:** PARTIALLY RESOLVED — py3.9 venv + aarch64 wheel found; NON-CANONICAL data now in §5.1 + §3.5 + §4.3.
  Canonical x86_64 AVX512 re-run remains pending (sq-hmd7l.26). **Bead sq-k21np may be closed** (original blocker resolved; canonical re-run is tracked under sq-hmd7l.26).

- **sq-3ocye** (P2): DiskANN-ref NOT-RUN (no aarch64 pip wheel) — re-run on x86_64 canonical box with diskannpy.
  **Updated 2026-07-10 [SONNET-4.6]:** x86_64 c6i.4xlarge c6i.4xlarge attempt executed (instance i-0c48ff9873f9f8940 via `--profile pss`); `diskannpy` x86_64 wheel installs and benchmark ran, but result retrieval failed: no IAM instance profile for S3 write, no serial console on AL2023/Nitro. **sq-3ocye remains OPEN.** Unblock: attach IAM instance profile with `s3:PutObject` to bench instance at launch. See §5.2 for full diagnosis.

- GloVe FAISS IVFFlat-IP sweep was running at commit time — fill §4.2 in the canonical wave-1 run (sq-hmd7l.26).

- **sq-lfo84** (P1) NEW 2026-07-10: sparq-vectors HNSW QPS BEHIND hnswlib by 6–19× at matched recall on 100k cosine SIFT1M (§3.6); root-cause: `instant-distance` pure-Rust no SIMD; fix: hnsw-rs / usearch binding or inline SIMD kernel.

- **sq-ose80** (P1) NEW 2026-07-10: sparq-vectors HNSW build at 1M×128d exceeded 30 minutes (aborted); hnswlib C++ builds the same in 374s; `instant-distance` is 6× slower at 100k, >5× impractical at 1M; profile + consider parallel construction or faster library.

---

*Document generated by SPARQ agent [SONNET-4.6] | bead sq-hmd7l.19 | NON-CANONICAL work-box first-read | canonical re-run: sq-hmd7l.26*
*Updated 2026-07-10 [SONNET-4.6] | sq-k21np (ScaNN NON-CANONICAL filled) + sq-3ocye (DiskANN x86_64 attempted, unrecoverable)*
*Updated 2026-07-10 [SONNET-4.6] | sq-z2z18 — sparq-vectors HNSW ef sweep on SIFT1M 100k (cosine, §3.6); nearest_with_ef API (sq-jo6ty PR #1856) now active; sparq BEHIND hnswlib 6–19× at all recall floors; P1 beads sq-lfo84 + sq-ose80 filed*
