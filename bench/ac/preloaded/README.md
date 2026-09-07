# Preloaded Pod evaluation

<!-- [GPT-6] -->
This prospective evaluation keeps every Pod's data descriptors and real WAC/ACP
authorization state alive before measurement. It preserves the earlier packed
campaign and completed evidence. The primary RAM comparison uses the same native
archive representation and fixed CPU/worker allocation throughout; a fully owned
memory representation is an optional small diagnostic.

Build with the separately reviewed core archive implementation and explicit
`population-native` feature. Run these commands only on the designated build/run
host after source review and resource admission:

```sh
cargo build --release --locked -p sparq-lws-core --example pod_population_http --features population-native
target/release/examples/pod_population_http prepare-native --corpus /var/tmp/pod-wac --max-pod-bytes 2147483648
target/release/examples/pod_population_http serve --corpus /var/tmp/pod-wac --auth-dir /var/tmp/pod-auth --storage-mode native --workers 16 --max-pod-bytes 2147483648 --control-token-file /var/tmp/pod-auth/preloaded-control-token
target/release/examples/pod_population_http drain --control-token-file /var/tmp/pod-auth/preloaded-control-token
```

`prepare-native` consumes the existing complete packed corpus, verifies its
payload/index hashes, and writes `pods.native` plus `pods.native-manifest.json`.
It refuses replacement. Parsing, index construction and native serialization
happen here, before any server measurement. The coordinator verifies the complete
archive size and digest once before worker mappings. Preserve its read-only
bytes and length until every process and mapped view exits; ordinary content and
policy updates modify overlays and the separate trusted application journal.

`serve --storage-mode native` requires the opt-in feature. `memory` parses the
same packed baseline once during startup; `cached` remains the original default.
Preloaded workers own disjoint Pod-ID partitions, materialize every authorization
index, replay accepted journals, and emit `all-population-ready` before HTTP
listening. An attempted read-time activation poisons the worker and prevents a
passing drain report. A failed mutation uses a graph snapshot to restore both
content and authorization. This recovery is timed and counted separately from
read-time activation; ambiguous journal commit errors require process restart.

The operator token is separate from Pod ownership and DPoP credentials. Supply
an unpredictable alphanumeric secret in an owner-readable-only file; the
`run-cell.py` adapter creates it with exclusive creation when absent. Do not copy
that file into public artifacts. `POST /__benchmark/drain` authenticates this
capability, pauses request admission and places a FIFO fence in each worker.
The admission guard begins in request middleware before body extraction, so a
previously admitted slow body must complete or fail before the fence. Connections
that have not entered request middleware are outside this boundary.
Its JSON response includes each worker's processing and activation counters.
The barrier includes work whose client stopped waiting. The same barrier runs
after the measured client window, while all stores remain alive.

`run-cell.py` runs one explicitly selected cell against already prepared corpora,
reusing the original journey driver, update receipts, queue rule and audit logic.
It does not generate a new population or declare a completed campaign. Supply
the original campaign fields plus `preloaded.version`, `fixed_server_cpus`,
`startup_timeout_seconds`, `drain_timeout_seconds` and each group's
`storage_mode`. The exact hardware, groups and deadlines must be frozen after
the native preparation pilot; `protocol.json` records the method without
inventing those observations.

Use a distinct `solid-pod-preloaded-` campaign ID and an empty output directory
for each cell. The adapter refuses reuse and binds the explicitly supplied source
commit and binary digest. It holds a corpus lock through journal reset, preload,
measurement and audit; stop any manually launched server first. CPU pools must
contain the fixed server allocation and disjoint client allocation, and the
server RAM tier must leave space under the declared outer study memory cap.

Every cell synchronizes and drops clean OS page caches on the dedicated host
before starting the server. The archive digest check, mapped validation and all
population initialization then run inside the measured server cgroup. Run cells
exclusively: another process warming the same file could charge pages outside
that cgroup and invalidate a RAM-tier comparison. Keep `memory.stat` anonymous
and file-page charges, I/O and memory events alongside the configured limit;
low-RAM paging must be demonstrated from those observations.

Independent repetitions stop the previous process and reset only its task-owned
application journals before starting every store from the same immutable archive.
Warmup is read-only. Draining does not undo mutations: deliberate restart/replay
correctness checks retain journals and are separate from resetting repetitions.

Readiness counters and source-byte totals do not establish heap size or memory
residency of every content page. Record cgroup peak/current memory, page-cache
charges, faults, process RSS/PSS, native file sizes, allocated bytes, inode counts,
open descriptors and startup CPU/time. Graph/dictionary descriptors and authorization
remain heap metadata even when mapped dictionary/content bytes leave RAM. Keep preload
failures visible and retain the all-offered response denominators after timeouts.

Local source checks can run without Rust compilation:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s bench/ac/preloaded -p 'test_*.py'
```

The scoped Rust tests include complete partition ownership, prevention of lazy
activation, failed-commit rollback of graph and authorization, and native
revocation replay. Remote compilation, Clippy, these tests, authenticated HTTP
drain smoke and independent source review remain required before timing admission.

`test_preloaded_http.py --binary PATH` is the bounded functional HTTP smoke for
either a debug or release binary with `population-native`. With no path it uses
`$CARGO_TARGET_DIR/release/examples/pod_population_http` (or the local `target`
directory). It checks every small native Pod against the decision/result oracle,
both languages' rejected policy callers and revocation after restart, complete
drains, retained counters, real journey writes, journal replay and receipt audit.
Its output is correctness evidence only.

`run-preparation.py` executes the separately supplied, checksum-bound preparation
proposal. It requires `--proposal`, `--proposal-sha256`, `--source-commit` (the
executor checkout), `--binary-source-commit`, `--binary-sha256`, `--binary`,
`--corpora`, `--auth`, `--results`, and explicit `--server-cpus` / `--client-cpus`
comma-separated CPU lists. It runs only on the dedicated Linux candidate host:
corpora, binaries and auth stay below `/mnt/sparq-native/data0`, data1 is untouched,
and small results go below `/var/tmp/sparq-pod-study` in an empty directory.

The proposal supplies complete populations and phase/global time bounds. Each
generation, native preparation, representative oracle check and full startup
runs in a named `sparq-native-job-pilot-*` systemd unit with the proposal's memory
limit, disabled swap and fixed CPU affinity. A complete source payload is hashed
against prior observations before reuse. Partial packs/archives are retained on
failure for inspection; the executor neither replaces them nor removes mutation
journals. Reused inputs have no newly measured generation/preparation duration.

Before startup, the executor checks for other active native workload jobs,
synchronizes and drops clean caches, then keeps the coordinator and every worker
inside the startup cgroup. It records full archive validation, all-Pod readiness,
the complete zero-workload drain, cgroup anonymous/file/peak memory, CPU/I/O,
process RSS/PSS and open descriptors while the stores remain alive. Host
`MemAvailable`/memory pressure and combined study-slice accounting record observed
OS/other-process headroom. The physical host remains distinct from each server
quota; these checks do not prove the same application fits a smaller physical
machine together with its OS and clients. Phase logs and
resource JSON remain separate from logical/allocated corpus storage inventory.
No journey load or response percentile is measured by this executor.

`preparation-input.json` and `preparation-result.json` identify
`executor_source_commit`, `binary_build_source_commit`, `binary_sha256`,
`proposal_sha256` and `source_input_equivalence`. Reusing a previously built binary
requires a full source-path diff that changes only the explicitly listed executor,
tests and documentation files; changes to rules, embedded assets, configuration,
Rust or Cargo inputs are rejected. The result preserves incomplete phases and
unattempted populations. A completed footprint pilot does not admit the final
performance campaign or establish capacity.

`preparation-projections.json` uses both completed history prefixes, including the
heavy class, to scale observed per-Pod native storage, retained anonymous memory
and phase times to larger candidate populations. These are explicitly modeled,
unmeasured scenario ranges, with no statistical or safe upper-bound interpretation.
Cgroup anonymous memory is a heap proxy that also includes stacks and allocator
state; complete dictionary bytes are mapped. Projections inform which larger full
history population to prepare next within the actual storage, runtime and cost
limits. They never substitute for complete preparation or cap history scale at a
population chosen by the earlier cache-based study.

`run-campaign.py` is the main executor after the preparation pilot and shipping
source review. Its required arguments are `--campaign`, `--campaign-sha256`,
`--source-commit`, `--binary-source-commit`, `--binary-sha256`, `--binary`,
`--corpora`, `--auth` and `--results`. The main binary must have been built from
the exact executor revision; the preparation-only input-equivalence exception
does not apply. The coordinator checks that the declared shipping commits are
ancestors of this source. The operator remains responsible for confirming that
the reviewed shipping PRs have landed before dispatch. No matrix is supplied by
the executor or selected from observed response results.

The external JSON extends the original campaign structure with the following
input contract. The tests use a labeled synthetic fixture, not a proposed matrix.

| Field | Required content |
| --- | --- |
| `status`, `campaign_id` | `frozen-before-measurement`, a new `solid-pod-preloaded-` identity |
| `source_commit`, `binary_build_source_commit`, `binary_sha256`, `shipping_commits` | Exact execution/build identities and the merged shipping ancestry |
| `bindings` | Relative source paths mapped to SHA256, including the protocol, old corpus definitions, workload, calibration, generator and ratings histogram; also every selected config file |
| `corpora` | Existing definition fields plus a unique new `id`, positive `pods`, `shape_reference` naming an old definition, `expected_config` copied in full from its pilot manifest, and `expected_config_sha256` |
| `groups`, `seeds` | Complete dataset/RAM/rate/repetition grid, `storage_mode: native`, `lane: journeys`, fixed CPU allocation, and distinct nonnegative integer seeds |
| `preloaded` | Existing version/CPU/startup/drain settings plus `cell_timeout_seconds` |
| `execution` | `version: 1`, `data_mount: /mnt/sparq-native/data0`, `data1_allocated: false`, `preparation_memory_gib: 128`, positive `phase_timeout_seconds`, `inspection_timeout_seconds`, and integer `maximum_cells` |
| `stop_rules` | Runtime, data disk floor, result disk reserve, aggregate/per-file retrieval limits, and the exact execution behaviors below |

The config digest uses Python `json.dumps(config, sort_keys=True,
separators=(',', ':'))` encoded as UTF-8. New Pod counts do not change retention,
literal entropy, service record distributions or rights. The executor accepts
larger complete history populations without a history-size ceiling, reconciles
every inventory row with its packed index entry and manifest totals, and verifies
full payload/native hashes. A frozen source-size cap that omits an observed heavy
Pod is rejected. Existing inputs may be reused only after this validation, with
no newly measured preparation duration. Corpora containing old journals are
rejected during preparation; independent timed runs reset only their own
application journals while holding the corpus lock.

This executor version requires these machine-readable stop behaviors:

```json
{
  "grid": "complete-declared-grid",
  "preparation_failure": "stop-before-all-timed-cells",
  "execution_error": "stop-and-preserve-partial",
  "correctness_failure": "quarantine-source-and-stop",
  "observed_guard_failure": "continue-declared-grid"
}
```

The matrix itself chooses the preparation, cell and total deadlines. The
dedicated-host execution envelope and bounded plan size are checked before
allocating the plan. Generation, native preparation, representative oracle checks
and lossless inventory compression use bounded preparation units. Every selected
population completes these steps before any timed cell. Each cell uses the
existing preloaded adapter, read-only warmup, both worker drains, fixed client
affinity, real journey writes and all-offered criteria. The server has an
independent systemd runtime limit; offline native receipt replay runs in its own
bounded preparation unit after the server stops. It is not a response timing.

The output includes copied frozen inputs, per-population storage/verification,
per-cell native cgroup/process/host accounting, complete or partial cell rows,
unattempted populations/cells, and `campaign-result.json`. It also includes
an exact copy of every source manifest for independent native-sidecar
hash binding. Each cell's `startup-boundary.json` separately measures systemd
launch through coordinator hashing, all-worker preload, readiness observation
and its resource capture, after the successful cold-cache command. Per-worker
preload times remain separate and do not include the complete startup boundary.
Closed logs and JSONL are losslessly compressed, then bound by the recursive
`MANIFEST.sha256`.
Cleanup/finalization failure prevents a complete result. Guard failures are
preserved even when all cells execute; execution completion does not establish
capacity, WAC/ACP equivalence or fit on a smaller physical machine. The original
campaign, request criteria and completed evidence remain separate.

`preloaded.admission_outcomes_version: 1` and the frozen stop rule
`preload_admission_failure: continue-only-confirmed-local-oom-or-startup-timeout`
allow the declared grid to continue after two narrowly evidenced startup outcomes.
A memory-limit outcome requires cgroup-local `oom` and `oom_kill` counters plus
systemd's `oom-kill` result. A startup-timeout outcome requires a running unit at
the frozen deadline, followed by successful owned-unit cleanup. An `ExecStopPost`
collector preserves local counters before systemd removes the cgroup; collector
failure, unknown exits, bad readiness, integrity failures and global wall/disk
limits stop execution. Its small post-exit process is included in terminal
counters and remains outside request measurements.

Such cells have execution status `admission-failed`, an explicit admission record
and incomplete-startup boundary, with no warmup or request workload. They are
completed negative attempts; later RAM tiers still execute. The analyzer keeps
`requests: null` and `valid_for_inference: false`, while separately reporting
`valid_for_admission_inference` and `admission_failure_cells` after verifying the
source, population, archive, raw log, terminal capture and canonical event.
Unsupported admission continuation invalidates the campaign; it is never used
to turn an unknown failure into a negative capacity result.

The native analysis reports `retained_history_million_pod_admission` instead of
an all-service admission boolean. Its per-model states and labeled attempts use
complete validated startup or typed admission failures for populations of at
least one million, matching the central history profile and role to the measured
source's frozen shape definitions. Compact controls and separately named stress
profiles cannot establish central retained-history admission. An admitted state
means at least one listed attempt admitted the full population; models may need
different resource tiers, and a response guard can still fail. Unmeasured and
inconclusive evidence remain separate. These states do not establish responsive
service capacity or empirical representativeness of all service data.

Run `analyze-campaign.py ARTIFACT_DIRECTORY --review REVIEW_JSON --output
research/solid-pod-preloaded-main.json` only after finalization. Optional
`--scratch-directory` selects temporary SQLite storage; `--source-root` selects
the local repository containing the exact measured Git revision. The review
object must bind `source_commit`, `binary_sha256`, `campaign_sha256` and
`manifest_sha256`, with `status: passed`, before any cell is headline-eligible.
The extractor rehashes every manifest member and compares declared source inputs
to blobs in the measured revision. It reads the recursive cell directories and
both `.jsonl.zst` and `.log.zst`; ambiguous plain/compressed inputs are rejected.
Completed offline audit units retain an exact cell audit copy and a losslessly
compressed parent phase log. The parent log is compressed immediately after the
unit stops and its copy succeeds, so raw duplicates do not accumulate between
cells. Failed closed audits retain the same evidence; active writers are never
compressed. Request schedules, receipts and retrieval limits are unchanged.

The output's distinct analysis kind is
`native-preloaded-campaign-independent-accounting`. `corpora` separates complete
metadata inventories, native logical/allocated storage, explicit preparation
reuse and actual preparation-unit resources. `cells` retains every planned row,
its execution status, `valid_for_inference`, `local_guard`, request counters,
`preload` and `resources`. Existing request keys are reused, including
`requests.latency_us["successful:scheduled_latency_us"].p95`,
`success_fraction_of_offered`, `deadline_fraction_of_offered`, HTTP status counts
and exact neutral schedule fingerprints. Missing quantities stay null; incomplete
campaigns and rejected evidence cannot feed `headline_eligible_cells`.
Canonical correctness-quarantine events override a passing completion summary.
Disagreements between recorded counters or guards and independent accounting
remain visible and make the affected cell inconclusive.

Resource samples are separate for `preload`, `before` and `after`, with anonymous
memory, mapped-file charges, peaks, actual CPU affinity, per-device I/O, process
faults and host headroom. `resources.paging` preserves measured refault/major-fault
and read-I/O deltas with an explicit attribution limit; a low quota alone never
establishes paging. Complete startup time remains separate from worker-only
preload time and response latency. Physical host size and server cgroup limits
remain distinct throughout.

`sampled_authorization_correctness` reports the exact common-rights result/count
oracle checks separately from `paired_comparisons`. The latter reuses independent
paired-run bootstrap intervals for response and deadline-completion ratios.
Formal practical equivalence requires the optional
`statistical_plan.practical_equivalence_margin_ratio` in the prospectively frozen
campaign, expressed as a finite ratio greater than one. Without that field,
comparison remains descriptive. A nonsignificant difference, equal tested rates
or a narrow latency interval does not establish sustainable capacity equivalence.

The optional `site/papers/solid-pod-preloaded-results.typ` presentation helper
loads no default JSON. `native-result-state` rejects cached schemas, incomplete,
quarantined, integrity-invalid and review-mismatched native analysis.
`native-response-rows` and `native-response-table` keep admission-failure latency
and response fractions null; optional explicit labels select rows before display.
`native-history-admission` exposes the separate derived startup account only
after the same evidence gate. The prospective manuscript is not bound to a
placeholder result. Run `test_native_paper.py` with `TYPST_BIN` to check the
adapter using temporary synthetic fixtures.
