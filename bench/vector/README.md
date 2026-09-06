<!-- [OPUS-4.8] sq-v02y — vector / ANN benchmark suite. Design: research/capability-benchmark-program.md §3.3. -->
# Vector / ANN suite

The ANN analogue of the LUBM / SHACL / FTS template: an **overview** dashboard row, a
**self-asserting deterministic gate** (regression alerts), and a **competitor** comparison
surface. It exercises `sparq-vectors` — the opt-in mmap'd f32 vector store (`.spqv`) + three
approximate-nearest-neighbour searchers: in-RAM **HNSW** (`VectorIndex`), on-disk **Vamana**
(`DiskAnnIndex`, `.spqg`), and **product-quantized** codes + full-precision re-rank (the DiskANN
search loop).

It does not invent a new gate — it **promotes the recall asserts the crate already gates on**
(`crates/sparq-vectors/tests/{recall,diskann,quant}.rs`) into a TRACKED METRIC the dashboard and
the cross-commit perf-gate can ratchet.

## The recall-deficit metric (design §4 / gap G4)

ANN recall is *larger-is-better*, but the `mode:"auto"` perf-gate ratchet is *smaller-is-better*.
So every recall metric is emitted as a **DEFICIT** — `recall_deficit_milli = round((1 - recall@10)
* 1000)` — measured against the `nearest_exact` brute-force ground truth. A recall regression then
shows as the deficit GROWING, which the integer `mode:"auto"` ratchet catches with **zero
`perf-gate.py` change** (the gate is data-driven from each metric's `mode`). This is the same
trick `scripts/bench-adapters/vector_lib_adapter.py` uses for the external ann-benchmarks harness.

| Workload | Searcher | Crate-gate floor | Gate metric |
|---|---|---|---|
| `hnsw_recall_at10` | in-RAM HNSW (`VectorIndex`) | recall@10 ≥ 0.95 | `vectors_hnsw_recall_at10` |
| `diskann_recall_at10` | on-disk Vamana (`DiskAnnIndex`) | recall@10 ≥ 0.90 | `vectors_diskann_recall_at10` |
| `pq_recall_at10` | PQ + full-precision re-rank | recall@10 ≥ 0.95 | `vectors_pq_recall_at10` |

All three measured vs exact-kNN ground truth — recall@k is a fair, standard, deterministic
comparator (the strongest honest-competition surface; see Competitors).

## Data substrate

`bench/vector/gen.sh [N] [seed]` is a thin **parameter source** (like the FTS suite) — the corpus
is synthetic and generated **in-process** by `examples/bench_vectors` from a deterministic
splitmix64 seed, so there is no external generator and nothing to materialise on disk. It echoes
the two pinned parameters (N then seed).

- Per-commit tier: `N=50000, seed=0` — the same 50k × 32 set the crate recall gate tests use
  (sub-second at release opt-level).
- The HNSW / DiskANN substrate scales with `N`; the PQ workload uses a **fixed 10k clustered set**
  (40 clusters × 250, spread 0.15) — PQ's design regime, matching `tests/quant.rs`.
- The big published ANN datasets (**SIFT1M / GloVe-100-angular**) drive the **gather-tier**
  `ann-benchmarks` recall-QPS Pareto harness — too heavy per-commit (tracked as a follow-up bead).

## Deterministic gate (HARD) vs timing (ADVISORY)

`run.sh` is the self-asserting entry point (the LUBM pattern). It runs `bench_vectors` on the
pinned corpus and gates each workload's recall deficit (exit 1 on any drift):

- **`diskann_recall_at10` / `pq_recall_at10` — EXACT-gated** vs `expected.tsv`. The Vamana graph
  build (`src/diskann.rs`) and PQ k-means (`src/quant.rs`) are single-threaded with fixed seeds,
  so they are byte-deterministic — the deficit must equal the committed constant AND clear its
  floor.
- **`hnsw_recall_at10` — FLOOR-gated only.** The in-RAM HNSW build (`instant-distance`) is
  **rayon-parallel**: the graph is seeded (`HnswConfig::seed`) and reproducible, but parallel
  float-sum reduction order can flip a single boundary neighbour (recall 0.999 vs 1.000 = ±1 in the
  deficit). Pinning HNSW to an exact deficit would be a **flaky** gate, which is dishonest — so it
  is gated on its floor (deficit ≤ 50 ⇔ recall@10 ≥ 0.95, the `tests/recall.rs` floor) instead.
  The floor gate is **INCLUSIVE**: `run.sh` fails only on `deficit > floor`, so a deficit of
  exactly 50 (exactly recall@10 = 0.95) PASSES — matching the crate's `recall@10 >= 0.95` condition.

The constants in `expected.tsv` were **derived by running `bench_vectors`** on the pinned corpus
(not guessed). The deterministic deficits (`vectors_*_recall_at10`) also have `mode:"auto"` entries
in `bench/perf-baseline.json` so a recall regression cross-commit is ratcheted in addition to the
per-commit `expected.tsv` diff.

**Timing is ADVISORY** (`mode:noise`, trend-only, **never hard-gated** — and this dev box is
non-canonical): the ci-bench hook harvests one `vectors_<workload>_query_us` per workload plus
`vectors_build_s` (HNSW index build time) into the dashboard; they are **not** in
`scripts/perf-gate.py`. The hard gate lives in `run.sh`.

## Running it

```sh
# the example links the HNSW `VectorIndex`, so the build needs the `approx-ann` feature
# (the example's Cargo.toml `required-features`); without it cargo errors out (exit 101).
cargo build --release -p sparq-vectors --example bench_vectors --features approx-ann
bench/vector/run.sh                    # self-asserting: exit 1 on any recall regression
# heavier substrate (advisory timing; HNSW/DiskANN scale with N):
VEC_N=200000 bench/vector/run.sh
```

`bench_vectors` emits, per workload, the same `name<TAB>count<TAB>us` 3-column contract the
ci-bench hook consumes (`sparq-vectors` is the *isolated* ANN crate — not a `sparq-cli` dependency
— so the runner is a crate `--example`, not a CLI subcommand). Proving the gate gates:

```sh
# perturb a deterministic deficit in expected.tsv (e.g. diskann 34 -> 0) => run.sh exits 1.
```

## Competitors (honest)

The **strongest surface for honest competition** — recall@k vs exact-kNN is a fair, standard,
deterministic comparator. The competitor harness is **ann-benchmarks-style**, reporting the
**recall–QPS Pareto at MATCHED recall** (NEVER a single latency number — a faster engine at lower
recall is not "faster"). Registered in `bench/competitors.json` with the `engines`/`values`
dashboard seam **empty** in git (AGENTS.md — no hard-coded perf):

| Engine | Lang | License | Adapter kind | Role |
|---|---|---|---|---|
| **hnswlib** | C++/py | Apache-2.0 | `python-lib` (`vector_lib_adapter.py`) | Primary canonical in-RAM HNSW peer. Recall–QPS Pareto. |
| **FAISS** | C++/py | MIT | `python-lib` | Quantizer/GPU + brute-force ground-truth oracle. |
| **ScaNN** | C++/py | Apache-2.0 | `python-lib` | The SOTA recall–QPS bar. |
| **DiskANN reference impl** | C++ | MIT | `python-lib` | Oracle for sparq's Vamana `.spqg`. |
| Qdrant / Milvus / Weaviate | mixed | OSS | (loose) | Full vector DBs — *more* than sparq; **loose-only**, apples-to-oranges in the opposite direction. |

The kernel peers (hnswlib/FAISS/ScaNN/DiskANN-ref) use the `python-lib` adapter
(`scripts/bench-adapters/vector_lib_adapter.py`) + an **exact-kNN oracle** (numpy), emitting the
same recall-deficit metric (G4). **No competitor does ANN-inside-SPARQL over dict-encoded entity
ids** — that surface value is **uncontested** (sparq's ANN is keyed by the store's dictionary term
ids, joinable directly into a SPARQL plan; the kernel libraries are standalone index structures).

Scope the kernel comparison to what `sparq-vectors` implements (cosine/L2 top-k, HNSW + Vamana +
PQ) or it is unfair. A real `scripts/gather-competitors.sh --run --only <id>` writes git-ignored
`bench/competitor-results/`; the big-corpus (SIFT1M / GloVe) recall-QPS Pareto is gather-tier (no
recurring CI cost — nightly + git-ignored results).

## Gather-tier: SIFT1M / GloVe-100-angular recall–QPS Pareto (sq-aiup)

<!-- [OPUS-4.8] sq-aiup -->
The big published-dataset recall–QPS Pareto runs through the `vector-lib` adapter's **`--pareto`**
mode (`scripts/bench-adapters/vector_lib_adapter.py`). It builds **one** hnswlib index over the
corpus, **sweeps the `ef` search-effort knob**, and emits **one `(recall_deficit, query_us, qps, ef)`
point per setting** — the recall–QPS curve. A single (recall, latency) point is meaningless for ANN,
so the `--json` envelope carries the **Pareto frontier** plus **QPS at matched recall**
(`matched_recall_qps`) — the only honest cross-engine number (two engines at the *same* recall floor,
never a single latency). The corpora are **not redistributable in-repo** (download/gather step), so
this is **nightly/EC2, not per-PR CI** (design §3.3(c)).

```sh
# SIFT1M (TEXMEX .fvecs/.ivecs under <root>/sift/) — L2:
VECTOR_DATASET=sift-128-euclidean VECTOR_ROOT=/data/ann VECTOR_EF=16,32,64,128,256 \
  scripts/gather-competitors.sh --run --only hnswlib
# GloVe-100-angular (ann-benchmarks <root>/glove-100-angular.hdf5; needs h5py) — cosine:
VECTOR_DATASET=glove-100-angular VECTOR_ROOT=/data/ann \
  scripts/gather-competitors.sh --run --only hnswlib
```

The pure halves — the `.fvecs`/`.ivecs` parser (`read_vecs`), the QPS/Pareto-frontier/matched-recall
maths — are **fixture-unit-tested without numpy/hnswlib** in `scripts/bench-adapters/test_adapters.py`
(`test_pareto`); only the index build + query (`load_dataset` / `run_hnswlib_sweep`) need the heavy
gather deps. Results land git-ignored in `bench/competitor-results/`. FAISS / ScaNN / DiskANN-ref are
additional kernel peers on the same recall–QPS axis (run each library's sweep, score against the same
exact-kNN ground truth, compare frontiers at matched recall) — tracked as further gather backends.

The **comprehensive multi-engine Pareto gather** (hnswlib + FAISS IVFFlat + FAISS IVFSQ8 on both
corpora) runs through `scripts/bench-adapters/gather_ann_pareto.py`:

```sh
# SIFT1M comprehensive gather (L2; hnswlib + FAISS):
python3 scripts/bench-adapters/gather_ann_pareto.py sift /tmp/ann/results_sift.json
# GloVe-100-angular comprehensive gather (cosine; hnswlib + FAISS-IP):
python3 scripts/bench-adapters/gather_ann_pareto.py glove /tmp/ann/results_glove.json
# Adapter self-test (no heavy deps; exercises pure functions only):
python3 scripts/bench-adapters/vector_lib_adapter.py --smoke  # must exit 0
```

First-read results are in `research/gap-vector-2026-07.md` (NON-CANONICAL work-box run;
canonical re-run is sq-hmd7l.26). ScaNN and DiskANN-ref were NOT-RUN on this gather;
see the gap record §5 for reasons and follow-up beads.

## Gather-tier: build-time-only HNSW scaling (sq-ose80.2)

<!-- [SONNET-4.6] sq-pm6i2 -->
`research/gap-vector-ann-simd-2026-07.md` §7.3 records the 1M×128 HNSW build cost as an
**extrapolation from a measured 200k curve**, because the run that would have measured it was
abandoned under disk pressure — and because the expensive part of that harness is the
brute-force `nearest_exact` **recall oracle**, not the build. `examples/hnsw_build_scaling`
drops the oracle and times **only** `VectorIndex::build_with`, which is what §7.3 actually
needs, so the 1M point costs a build instead of a build plus an O(n_query × n_base) scan.

It emits no recall and no QPS column and adds **no gate** — it is a measurement harness, not a
per-commit runner (`run.sh` is unchanged). SIFT1M is not redistributable in-repo, so the
operator supplies the base vectors in the same raw f32 format `sift_ef_sweep` reads (`u32` LE
`n`, `u32` LE `dim`, then `n × dim` `f32` LE). **The `.fvecs` → raw-f32 converter is not
committed** — `scripts/bench-adapters/vector_lib_adapter.py::read_vecs` parses the TEXMEX
distribution format but does not emit this one, so the conversion is currently operator glue.

```sh
cargo build --release -p sparq-vectors --example hnsw_build_scaling --features approx-ann
# self-test on a tiny synthetic corpus — no dataset needed:
target/release/examples/hnsw_build_scaling --smoke
# the sq-ose80.2 measurement — 1M SIFT base vectors at the three shipped HnswConfig presets:
SPARQ_VECTORS_TMP=/data/scratch \
  target/release/examples/hnsw_build_scaling /data/ann/sift/base.bin 1000000 40,100,200
```

`$SPARQ_VECTORS_TMP` places the temporary `.spqv` store (`n × dim × 4` bytes — ~512 MB at
1M×128; the §7.3 run died on a full `/tmp`). Output is TSV
(`n dim ef_construction preset store_s build_s vec_per_s`), flushed per row so a long run
reports incrementally, with a footer recording the corpus and thread count.

**Build wall-clock is NON-CANONICAL unless it ran on the dedicated quiet bench box** under the
quiet-box protocol; on a shared box the *ranking* of the `ef_construction` levels transfers
where the absolute seconds do not. A measured 1M point replaces the §7.3 extrapolation only
when it comes from the canonical box, and does not go into markdown as a hard-coded number
(AGENTS.md) — it belongs in the gap record's own measurement table with its provenance.
