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
open descriptors and startup CPU/time. Dictionaries, descriptors and authorization
remain heap metadata even when mapped content pages leave RAM. Keep preload
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
