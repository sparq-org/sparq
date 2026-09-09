---
name: gpu-kernels
description: "Experimental GPU (wgpu / WGSL) compute kernels for sparq's hot-path primitive shapes — FILTER + count, hash-join probe, and GROUP BY COUNT+SUM — over plain ValueId-shaped columns (&[u32] / &[f64]) in the opt-in sparq-gpu crate. Use when measuring whether a GPU beats the CPU for sparq's columnar primitives once the host->device transfer tax is charged, driving the Gpu handle and resident column/hash-table types, or reading the device-vs-CPU verdict. MEASUREMENT PROTOTYPE (roadmap T24d), PARKED, publish=false — NOT a wired-in execution backend and not a supported product surface."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq-gpu — experimental GPU compute kernels (T24d, parked)

`sparq-gpu` is an **opt-in measurement prototype**: opt-in [wgpu](https://wgpu.rs) /
WGSL compute kernels for sparq's three hot-path primitive shapes — **FILTER + count**,
**hash-join probe**, and **GROUP BY COUNT+SUM** — over plain `ValueId`-shaped columns
(`&[u32]` / `&[f64]`). It was built to answer one question with measurements instead of
aspiration:

> Where (if anywhere) does a GPU beat the CPU for sparq's hot paths once the
> host→device transfer tax is charged honestly?

## What this is — and is NOT (read first)

- It is a **roadmap-T24d measurement prototype**, `publish = false`, and **parked**.
  **Nothing in the workspace depends on it**; it is **not wired into the query
  engine** — there is no scheduler integration, no residency cache, no SPARQL-level
  routing of work to the GPU. You drive the kernels directly.
- It deliberately has **no `sparq-core` dependency** — the kernels take plain slices in
  exactly the shape of sparq-core's permutation-index object columns / dense numeric
  cache, so the engine carries zero GPU code and **wgpu never enters the wasm build**
  (`cargo tree -p sparq-wasm --target wasm32-unknown-unknown` contains no `wgpu`).
- **The measured verdict is: parked.** On an M1 (unified memory — the *best possible*
  transfer economics) the GPU loses or merely ties an 8-core CPU on compute-light scans
  even when the column is already device-resident, and only wins on the hash-probe
  shape. One winning kernel class does not pay for a residency cache, scheduler
  integration, and a second execution backend. Full tables + the re-open conditions:
  [`research/gpu-verdict.md`](../../research/gpu-verdict.md). Do not present this crate
  as a GPU-accelerated query engine.

## Quickstart

`crates/sparq-gpu/Cargo.toml` (no cargo features; `publish = false`):

```toml
[dependencies]
sparq-gpu = { path = "../sparq-gpu" }
```

`Gpu::new()` returns `Option` — **no adapter ⇒ `None`** (runtime check, so a CI box with
no GPU skips gracefully):

```rust
use sparq_gpu::Gpu;

let Some(gpu) = Gpu::new() else { return; };       // no adapter → skip

// FILTER + count: how many elements fall in [lo, hi]?
let col = gpu.upload_u32(&[1u32, 5, 9, 3, 7]);
let n = gpu.filter_count_u32(&col, 4, 8);          // -> 2  (5 and 7)

// Hash-join probe against a resident open-addressing table.
let table = gpu.upload_table(&[[42, 100], [7, 200]]);   // [key, payload]
let probe = gpu.upload_u32(&[42u32, 7, 7, 1]);
let (matches, payload_sum) = gpu.hash_probe(&table, &probe);

// GROUP BY COUNT+SUM (keys pre-densified to 0..groups, groups ≤ MAX_GROUPS = 512).
let keys = gpu.upload_u32(&[0u32, 1, 0, 1, 1]);
let vals = gpu.upload_u32(&[10u32, 20, 30, 40, 50]);
let per_group = gpu.group_aggregate(&keys, &vals, 2);   // Vec<(count, sum)>
let _ = (n, matches, payload_sum, per_group);
```

## Public API

- `Gpu::new() -> Option<Gpu>` — adapter detection + the four WGSL compute pipelines.
- Upload / overwrite device-resident columns: `upload_u32` / `upload_f64` /
  `upload_table` (build), and `write_u32` / `write_f64` / `write_table` (re-fill an
  existing buffer in place). Resident types: `ColU32`, `ColF64`, `HashTable`.
- Kernels (all reduce **on-device** and read back O(1)/O(groups) bytes):
  - `filter_count_u32(&col, lo, hi) -> u64` — count elements in `[lo, hi]`.
  - `filter_count_f64_gt(&col, t) -> u64` — count `> t`. WGSL has **no f64**, so the
    f64 kernel compares IEEE-754 bit patterns mapped to a monotonic u64 key (sign-flip
    trick): **exact, NaN-correct, no float math on the device** for comparisons.
  - `hash_probe(&table, &probe) -> (u64, u64)` — `(matches, payload_sum)` against a
    resident linear-probing table (load ≤ 0.5; u64 sum via 32-bit atomic carry).
  - `group_aggregate(&keys, &vals, groups) -> Vec<(u64, u64)>` — `(count, sum)` per
    group with two-level (workgroup-shared → global) atomics.
- Constants / introspection: `EMPTY_KEY` (`u32::MAX`), `MAX_GROUPS` (`512`),
  `Gpu::max_storage_bytes` (the `max_storage_buffer_binding_size` cap on resident column
  size).

## Limits worth knowing

- WGSL has **no `f64`**: the bit-order trick makes *comparisons* exact, but future
  *arithmetic* (SUM over `xsd:double`) would need emulation or a downcast.
- `group_aggregate` caps at **512 groups** (the workgroup shared-memory tile); higher
  cardinalities need a different (global-atomic or sort-based) kernel.
- Hash tables are built **on the host**; only the probe is offloaded.
- `max_storage_buffer_binding_size` caps resident column size (4 GiB − 1 on the M1;
  commonly 128 MiB–2 GiB elsewhere — check `Gpu::max_storage_bytes`).
- Kernels reduce on-device and read back O(1) bytes — the **best case** for the GPU;
  materializing selected rows would add a device→host stream, so the measured verdict is
  an *upper bound* on GPU benefit.

_(status: EXPERIMENTAL measurement prototype (roadmap T24d), `publish = false`, **parked**
per `research/gpu-verdict.md`. Verified against `crates/sparq-gpu/src/{lib,cpu}.rs` +
README on branch `feat-skill-drift-catchup` (2026-06-16); workspace v0.1.0. Correctness is
checked GPU-vs-CPU on random data + IEEE-754 edge cases (`tests/correctness.rs`, which
skips when no adapter exists); `src/cpu.rs` holds the scalar oracles + single-thread
baselines. Nothing depends on this crate and it is NOT wired into the engine. Bench
(`examples/gpu_bench.rs`) numbers are non-canonical and live in the verdict, not here.)_
