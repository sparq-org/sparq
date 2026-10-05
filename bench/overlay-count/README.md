# Overlay range-count diagnostic

[GPT-6 Astra] Local diagnostic for issue #4246; results are noncanonical. The
standalone crate follows `bench/alloc-track` and reuses the calibrated System
allocator wrapper from the earlier indexed-topk diagnostic; no top-k runtime is
included. Build timing and `count-alloc` binaries separately with the same source,
lockfile, release profile, two build jobs, and one Rayon runtime thread.

```sh
CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml --features sparq-core/overlay-deleted-projections
CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml --features count-alloc,sparq-core/overlay-deleted-projections
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

The `lifecycle` argument selects the fixed review follow-up: a delete-only overlay
with 32,768 tombstones, four retained generations, and one or all six projections
warmed before read-only snapshots, fork/insert/read, or fork/delete/read. A
six-projection in-place insert/read control separates deep cloning from invalidation.
Three additional points execute a three-pattern query cold and warm, and two
concurrent cold readers synchronized immediately before their ordinary queries.
No dimension is selected from observed timings. This protocol retains the same
two warmup samples and seven timing / three allocation repetitions.

Lifecycle windows include clone (where applicable), one delta, whole query, and
retaining the resulting graph or snapshot in a preallocated local vector. This is
local ownership publication, not server or durable-store publication. Each of four
generation windows is measured separately while earlier generations remain alive.
`phase_ns` records clone/delta/read; for concurrent readers its first two values
are the individual reader durations and the whole window includes thread launch
and join. Those few samples do not establish tail percentiles.

Retained store heap subtracts the shared immutable base once per reference, then
sums the overlay heap of the initial graph and retained generations. Existing
`heap_bytes` omits fixed boxed-overlay metadata and estimates hash-table capacity;
allocator counters separately report live requested bytes. Cold-query cases do not
prime measured forks. A separate oracle records the multi-pattern query's actual
projection heap growth and verifies the complete four-row result.

The core experiment is OFF by default. Omit `sparq-core/overlay-deleted-projections`
for the ordinary linear control; the benchmark does not forward or enable it
implicitly. Timing and allocator comparisons use the same harness, with the core
feature state recorded separately in build provenance.

The `reads-per-generation` argument runs only the fixed fork/tombstone/read case:
32,768 initial tombstones, six initially warmed permutations, four retained child
generations, and reads per generation in `{1, 2, 4, 8, 16}`. Each child adds one
tombstone, executes the same ordinary single-pattern SELECT that many times, then
is retained locally. Earlier generations and the initial graph stay alive. The
existing two warmups, seven timing and three allocation repetitions apply. This
measures the declared read counts, not a recommended crossover or tuning threshold.

### Allocator window regression

<!-- [GPT-6 Astra] Test the actual allocator module without building the benchmark. -->
From the repository root, with its pinned Rust toolchain already installed:

```sh
test_dir=$(mktemp -d)
rustc --edition=2021 --test bench/overlay-count/src/counting.rs \
  -o "$test_dir/counting-window-tests"
"$test_dir/counting-window-tests" --exact tests::window_ownership_and_calibration \
  --test-threads=1 --nocapture
```

This depends only on the standard library. The detached benchmark is not reached
by ordinary workspace tests; do not treat that CI as execution of this command.
One serial test checks repeated clean calibrations and nested/competing admission
against the real module. Panic handling and thread machinery can allocate, so the
denied-admission checks measure a known request after that machinery has finished,
rather than asserting exact totals for an invalid window. Libtest itself can
allocate globally; unexpected calibration counts are a failure to investigate,
not a reason to adjust the oracle or repeat until green.

Window ownership covers counter reset and readout. The successful coordinator must
balance `begin` with `end`, with workers quiescent at both boundaries. The guard
does not make arbitrary concurrent allocator activity into an atomic snapshot or
authorize another caller to end the owner's window. The regression synchronizes
a competing attempt after the owner has begun; it does not claim to execute every
possible reset/readout interleaving.
