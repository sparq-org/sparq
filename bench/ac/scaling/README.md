# Many-Pod access-controlled SPARQL benchmark

This directory contains the reproducible artefact for the paper on read-only,
WAC-scoped SPARQL over Solid Pods.  The prospective statistical decisions are fixed in
[`ANALYSIS-PROTOCOL.md`](ANALYSIS-PROTOCOL.md).  The generator and runner deliberately
keep two implementation paths separate:

- **materialized/routed**: one `sparq-solid::PodStore` per Pod and a routing-table lookup
  before `PodStore::query_as`;
- **native HTTP assembly**: the real `sparq-lws-core` axum `/sparql` route over in-memory
  storage seams.

Neither lane is a simulated cost model.  Both execute the production query and WAC code.
The storage doubles in the HTTP lane intentionally remove remote-storage latency so that
the measurements characterize server compute and deterministic backend-operation counts.
Their normal 4,096-resource/64-MiB deployment defaults are replaced in this benchmark by
the smallest limits sufficient for the generated fixture; aggregate bytes and resource
slots are derived from the corpus and emitted in every HTTP construction record.

Timing and deterministic instrumentation are separate executable profiles. Paper-facing
latency uses the production LWS mimalloc allocator and unwrapped stores. The instrumentation
profile wraps that allocator and both backend seams to count allocations and storage calls;
its timings are not pooled with, or substituted for, latency-profile observations.

## Correctness contract

The corpus assigns every content document an owner, public, or named-recipient audience.
An independent predicate turns that assignment into a physical reference dataset before
the SPARQL engine runs.  Each timed result is compared as a duplicate-preserving,
order-independent multiset with evaluation over that reference.  A mismatch, non-200 HTTP
response, invalid result document, or missing route terminates the process before the
affected observation is emitted.

The generated WAC subset is intentionally narrow: `acl:agent`, public
`acl:agentClass foaf:Agent`, `acl:accessTo`, `acl:default`, and `acl:Read`/`Control`.
ACP, ODRL, groups, origins, issuers, and conditional rules are not pooled into these WAC
results.

## Quick start

Run all unit and cross-layer correctness gates, then a short optimized pilot:

```sh
bash bench/ac/scaling/run.sh --correctness
bash bench/ac/scaling/run.sh --smoke
bash bench/ac/scaling/run.sh --instrumentation-smoke
```

An interrupted correctness sweep resumes from checksummed cells. Set
`SPARQ_AC_CORRECTNESS_LANES=http` (or `materialized`) to rerun only one implementation
lane; the default runs both. The matrix fixes eight triples per document so all eight
query families are applicable in every configuration.

The disposable-host driver has a deliberately separate sizing pass:

```sh
AWS_PROFILE=pss bash bench/ac/scaling/launch-ec2.sh pilot
AWS_PROFILE=pss SPARQ_AC_PRIOR_AWS_USD="$ACTUAL_PRIOR_COST_USD" \
  bash bench/ac/scaling/launch-ec2.sh canonical
```

The launcher refuses a dirty source tree, performs the repository orphan check, and
resolves a current Linux on-demand price through the AWS Price List Query API. If the
active role lacks that optional Pricing permission, it streams AWS's official public
regional Price List bulk CSV and applies the same exact product filter. Both paths fail
closed on an absent or ambiguous rate and retain source provenance. The launcher then
enforces the USD 80 operating plan and USD 100 study ceiling and transfers an exact Git
bundle rather than requiring an unpublished branch to be pushed. Its security group
admits SSH only from the launching address. A unique idempotency token is also an
instance tag, allowing cleanup to recover a launch whose AWS response was lost; cleanup
re-verifies both that token and the exact bench-purpose tag. Both the instance shutdown
behavior and an instance-local 12-hour watchdog terminate the disposable host. Results
are pulled incrementally into the ignored `results/` directory.
The pilot runs the correctness matrix, largest scenario cells in both profiles, and a
reduced primary endpoint matrix; it is for capacity and duration planning and is never
accepted by canonical analysis.

The secondary campaigns are generated from committed, randomized schedules:

```sh
bash bench/ac/scaling/run.sh --sensitivity
bash bench/ac/scaling/run.sh --scenarios
bash bench/ac/scaling/run.sh --pod-scaling-instrumentation
bash bench/ac/scaling/run.sh --sensitivity-instrumentation
bash bench/ac/scaling/run.sh --scenarios-instrumentation
```

`--sensitivity` varies documents, triples, ACL placement/depth, and readable fraction
independently and selects the point and unbound-graph families. `--scenarios` runs the
declared social and compact-health count anchors with the point query; it is a resident
construction/point-lookup check, not a claim that the generated values reproduce a
future Pod population. Both default to five blocks, thirty repetitions, and ten warm-ups.
Those values can be changed for resource-sizing pilots with
`SPARQ_AC_FACTOR_BLOCKS`, `SPARQ_AC_FACTOR_REPETITIONS`, and
`SPARQ_AC_FACTOR_WARMUPS`; such reduced runs are not canonical. The existing
`SPARQ_AC_LANES`, `SPARQ_AC_DOMAINS`, and `SPARQ_AC_RESULTS` variables restrict lanes,
domains, and output location.
The three instrumentation campaigns reuse the same blocks and randomized schedules but
default to one warm-up and one measured pair. Override those only for diagnostics with
`SPARQ_AC_INSTRUMENTATION_WARMUPS` and `SPARQ_AC_INSTRUMENTATION_REPETITIONS`.
On Linux, set `SPARQ_AC_CPUSET` to one logical CPU (for example, `1`) to run the process
through `taskset`. Canonical validation reads the effective affinity from
`/proc/self/status` and rejects an absent mask, a range, or a list; setting the variable
without successful kernel pinning is therefore not enough.

Raw JSONL and stderr logs are written below `bench/ac/scaling/results/`, which is ignored.
To run one cell directly:

```sh
cargo run --release -p sparq-lws-core --example ac_query_scale -- \
  --lane materialized --pods 64 --documents 16 --triples 8 --depth 3 \
  --acl-coverage 250 --public 0 --private 1000 --shared 0 \
  --domain social --topology origin-per-pod --principal owner \
  --warmups 10 --repetitions 30 --process-block 0 --run-id example
```

Use `--lane http --topology shared-origin` for the native endpoint. `--query q1-point`
through `q8-graph-scan` selects one family; a comma-separated value such as
`--query q1-point,q8-graph-scan` selects a fixed subset without rebuilding the fixture.
With fewer than eight generated triples per document, query families whose predicates do
not exist are emitted as explicitly inapplicable rather than silently changed.

## Pod-count intervention

The primary Pod-count block uses `public/private/shared = 0/1000/0`.  Pod 0's owner can
therefore read exactly the fixed `D` documents in Pod 0 and none in added background Pods.
Using the ordinary 10/70/20 mixture here would make every public background document
readable and confound deployment size with authorized working-set size.  Visibility is
varied in its own block.

## Raw record types

The runner emits:

- `applicability`: the query text hash, required document width, and selection status;
- `construction`: generation/load/materialization or LWS-seeding time, graph/triple counts,
  target-Pod and actual evaluation-readable document counts, profile-dependent cumulative
  allocations, and
  resident memory where the OS exposes it; HTTP rows also record their derived store-byte
  and resource-count limits;
- `correctness-gate`: the exact-oracle preflight that completed;
- `observation`: one raw guarded or paired plain execution, including wall and available
  CPU time (`CLOCK_PROCESS_CPUTIME_ID` on Linux), profile-dependent allocation counts/bytes, result
  cardinality/hash, and—for native HTTP—storage
  operation counts and operation-scoped maximum in-flight calls.

Every record also carries the configured warm-up/repetition counts, Rayon worker count,
and effective Linux CPU affinity. These are experiment/environment controls, not inferred
from filenames or command-line intent.

Client-side JWT/DPoP signing and request construction occur outside native-HTTP timing.
The response body is fully consumed inside it.  Warm-ups are executed but not substituted
for raw repetitions or reported as minima.

Construction allocation counters in the instrumentation profile cover service construction only and close before the
paired oracle is built. RSS is deliberately labelled at whole-process scope: the current
sample is taken after the source corpus is dropped but still includes the physically
filtered oracle graph, while the process high-water mark also includes generation. It is
a conservative capacity indicator, not an isolated server-heap measurement.

## Resource and publication discipline

`run.sh` refuses a new cell when free disk is below 25 GiB or the largest recorded process
high-water RSS reaches 70% of detected physical RAM. It scans complete and `.partial`
JSONL files before each cell and fails closed if either is malformed. The observed-RSS
guard does not predict an unmeasured local cell, so local matrices should be restricted
to known-safe sizes; the EC2 campaign additionally uses a hard memory ceiling for the
first unexpectedly large cell. `SPARQ_AC_MEMORY_FRACTION` can make the guard more
conservative, but canonical runs use `0.70`. Local timings are diagnostic only. Canonical
wall-clock results require a quiet, single-CPU-pinned EC2 environment and the
cost/termination controls described in the protocol; the total study ceiling is USD 100,
with only USD 80 planned.

Do not paste workstation numbers into prose.  The analysis program consumes immutable
JSONL, verifies pair completeness and result hashes, and produces publication tables and
figures from the canonical files.

Analyze one or more result directories with:

```sh
python3 bench/ac/scaling/analyze.py bench/ac/scaling/results \
  --out bench/ac/scaling/derived --bootstrap-draws 10000
```

Pass `--require-canonical --expected-commit <40-hex-object>` for paper-facing latency
analysis. It rejects missing checksums, debug builds, dirty source trees, non-Linux or
incomplete EC2 metadata, mixed machines/toolchains, absent schedule metadata, incomplete
five-block cells, and runs without both a single Rayon worker and an effective one-CPU
kernel affinity mask. The H2 output applies the same block-cluster bootstrap to wall latency
and positive Linux process CPU time; both must meet the frozen ratio and elasticity margins
before the routed path is described as having minimal unrelated-Pod overhead.
`schedule.py` pseudorandomizes the entire lane/domain/Pod-count cross-product within each
process block from a recorded seed; it does not rely on platform-specific `shuf` output.
