# Overlay range-count diagnostic

[GPT-6 Astra] Local diagnostic for issue #4246; results are noncanonical. The
standalone crate follows `bench/alloc-track` and reuses the calibrated System
allocator wrapper from the earlier indexed-topk diagnostic; no top-k runtime is
included. Build timing and `count-alloc` binaries separately with the same source,
lockfile, release profile, two build jobs, and one Rayon runtime thread.

```sh
CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml
CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml --features count-alloc
```

Save each binary before the next build. Each invocation emits the entire fixed
matrix as JSON lines: 50,000 subjects with four predicates; deletion counts 0, 16,
1,024, 8,192, and 32,768; an additional insert-only 8,192-triple control; bound
store scan and ordinary single-pattern SELECT; cold and warm cache states. The
selected subject is outside every mutation range. Cold means one operation on a
fresh fork after its delta. Warm means one priming operation, then 10,000 scans or
100 whole queries. Each case has two unrecorded warmup samples followed by seven
timing samples or three allocation samples. Every sample has an independent fork;
oracle verification uses a separate fork so it cannot warm measured caches.

Setup, delta construction/application, and correctness checks are outside measured
windows. Query execution includes parsing, evaluation, result materialization and
drop. Scan execution also includes two dictionary lookups before the loop. Results
must match the generated single-row oracle. No wall-clock assertions are used.

`requested_bytes` includes successful allocation and full new realloc sizes;
`peak_live_delta_bytes` is requested live heap above the window's baseline, not
allocator metadata, stack, mapped memory, or realloc's internal transient peak.
Store heap accounting is reported cold, before, and after each window, separately
from process high-water RSS. RSS is cumulative and cannot identify query heap.
Counting calibration runs before setup. Compare only byte-identical harnesses,
identical dimensions/features, and record exact source and binary hashes with raw
JSON. Preserve all samples; do not seek a quiet subset or publish a speedup claim
from these local diagnostics.
