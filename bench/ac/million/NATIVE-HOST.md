# Disposable native evaluation host

<!-- [GPT-6] Ready-only infrastructure; the experiment matrix is a separate artifact. -->

`native-host.sh` selects the constrained `native` mode of the audited EC2 launcher:
London `r7gd.12xlarge` (48 ARM vCPUs, provisioned 384 GiB), 80 GiB gp3 root, and
an absolute watchdog no longer than 12 hours. The launcher retains exact-tag cleanup,
protected-instance exclusions, source-bundle verification, private SSH ingress, an
independent local termination supervisor, final checksums, and confirmed termination.
The orphan preflight refuses launch while another study/build host remains running.

Before launch, review the concrete launcher and tests, confirm shipping prerequisites
are merged and the build host is terminated, and supply the recorded prior study
allocation **including the existing reserve and current build reservation**. The
current official AWS regional price file is checked with Decimal arithmetic; the new
reservation covers 12 hours plus a 15-minute termination cushion, 80 GiB gp3 using a
conservative 672-hour month, and one public IPv4 address. It adds no duplicate
contingency and rejects an accumulated reservation above the original USD 100 ceiling.
This is a conservative allocation estimate, not an invoice or new budget approval.

```sh
SPARQ_POD_PRIOR_AWS_USD='<recorded prior allocation>' \
SPARQ_POD_RESULTS_LOCAL='<absolute artifact directory>' \
bash bench/ac/million/native-host.sh
```

Keep the launcher's clean checkout unchanged until the exact bundle upload completes.
The private `native-host.json` contains connection paths, IP and deadline; never print
or publish its private key contents. Its source is infrastructure provenance, not the
later measured revision. `native-host-scope.json` distinguishes this explicitly.

`native-storage.py` admits exactly two whole, blank devices with the EC2 instance-store
model, distinct AWS serials, the expected size, no partitions, mountpoints, filesystem
or `wipefs` signature. It validates the entire set and rechecks topology before any
format. Existing state is rejected, never erased or reformatted. EBS devices are
excluded by identity, even when Linux device enumeration changes. The two ext4/noatime
filesystems mount separately at `/mnt/sparq-native/data0` and `data1`; no RAID is assumed.
`native-storage.json` records before/after topology and each model, serial, size and
mount. The declared corpus lives on **data0**; data1 remains unallocated unless a later
frozen protocol explicitly selects it. Local NVMe persistence through `fsync` does not
imply survival of instance stop/termination or host loss.

The host publishes `READY` and waits. It never executes an uploaded command or starts
the old campaign. The coordinator explicitly stages a clean checkout containing the
merged implementation, exact source commit, frozen campaign, built binary and hashes
on data0. Record build/preparation outcomes. Place generated packed/native corpora,
Cargo targets and authentication material on data0, outside retrieved results.
Dispatch reviewed preparation/cell orchestration through a bounded transient unit
whose name begins `sparq-native-job-`; record the exact argv and source/binary/campaign
hashes. The per-cell runner creates `sparq-pod-preloaded-*.service` workers. Both unit
families must stop before finalization. This ready-only control loop is not a job
executor or a substitute for campaign/source correctness checks.

Use 16 server CPUs/workers and four disjoint client CPUs for each declared RAM tier.
A 128 or 256 GiB server cgroup ceiling is distinct from 384 GiB physical host capacity;
the driver, operating system and orchestration also need memory. For every cell, stop
prior workers, sync and drop clean page cache on this dedicated host **before** the
new server starts. Hash the archive, validate components, preload every Pod, materialize
authorization and replay journals **inside that fresh server cgroup**. Do not hash or
pre-touch the archive from another cgroup after the reset. Record preparation and
measurement snapshots of `memory.stat` (file and anonymous), `memory.current/peak/events`,
`memory.pressure`, `io.stat`, device I/O and page faults. A failed preload is an admission
outcome; paging or all-resident operation needs measured evidence.

Linux charges memory to its instantiating cgroup; moving a prepared process does not
move its earlier charges ([memory ownership](https://docs.kernel.org/admin-guide/cgroup-v2.html#memory-ownership)).
A cache reset releases clean reclaimable pages; sync improves its coverage but it
is not proof that every pinned page is absent ([drop_caches](https://docs.kernel.org/admin-guide/sysctl/vm.html#drop-caches)).
The per-cell runner owns these operations and the warmup/measurement drain barriers.

Write small evidence under `/var/tmp/sparq-pod-study`, with separate cell directories.
The ready monitor checks a 2 GiB total / 512 MiB per-file evidence bound. The native
receiver independently validates a bounded file list, limits streamed bytes, includes
replacement scratch in its 2 GiB aggregate ceiling for receiver-owned evidence, and retains 2 GiB local free-space
headroom. It cannot receive native archives or unselected files. Each transfer has a
five-minute deadline. Final retrieval selects only published manifest entries and
terminal markers; live snapshots remain provisional. The final recursive manifest covers retrieved evidence;
large native/packed corpora and build targets stay on NVMe and are not retrieved.

After all job and worker services stop, atomically upload
`/var/tmp/sparq-native-control/FINISH.json`. Its terminal `status` is `completed` or
`stopped-with-partial-evidence`; include `source_commit`, `source_path`, `campaign_path`,
`campaign_sha256`, `binary_path`, and `binary_sha256`. Paths must resolve under data0.
The control loop rechecks the clean checkout, exact hashes and campaign status
`frozen-before-measurement`, then publishes the recursive manifest before `DONE`.
If no experiment was run, use `status: not-run` and a nonempty `reason` instead; this
records an explicit cancellation, not a successful benchmark. Launcher cleanup then
terminates the host. Neither `DONE` nor a matching manifest establishes an SLO pass;
that decision belongs to the source-bound benchmark analysis.

The reviewed preparation pilot instead uses `kind: preparation-proposal`, retaining
the same terminal statuses. Include `executor_source_commit`,
`binary_build_source_commit`, `source_path`, `proposal_path`, `proposal_sha256`,
`binary_path`, `binary_sha256`, `preparation_result_path`, and
`preparation_result_sha256`. Source/proposal/binary paths resolve under data0; the
result resolves under the retrieved results directory. The proposal is the immutable
input with SHA-256 `ee0ae1b7ed5b628c36c1b798636f7c86426688b02b0485539650e46467e41fd7`
and its original prospective-footprint status. It is not relabelled as a frozen main
campaign. The result must have kind `native-preparation-pilot-result`, bind the same
identities, and be complete before a `completed` receipt is accepted.

The monitor independently compares the binary build commit with the clean executor
checkout using an all-path Git diff. Only the five reviewed executor/documentation
paths listed in `native-host.py` may differ: any Rust, Cargo, rule, embedded asset or
other input change rejects the receipt. The comparison must match the executor's
`source_input_equivalence` report. Preserve the separately recorded binary build
provenance; a matching binary checksum and source comparison do not reproduce the
build. The finalized receipt explicitly records `timed_load`, `slo_admission` and
`capacity_admission` as false. Preparation may establish footprint and startup
outcomes, but cannot admit a response-time or capacity result.

If evidence validation fails, finalization does not repeat that failed validation.
It instead copies a bounded set of stable diagnostic snapshots into a separate
`failure-evidence-*` directory, records excluded or changing files in
`failure-evidence.json`, publishes a manifest of the copied bytes, and creates `FAILED`.
Original files remain on the host for diagnosis. These immutable snapshots retain
available evidence without making incomplete runs valid or calling live files closed.
The receiver retrieves only this failure manifest's entries, so an oversized or unsafe
original cannot defeat the receiver's limits.

The receiver records the checksum of each file it owns. When a terminal manifest
arrives, it reuses identical owned files under their final snapshot paths and removes
only unchanged owned provisional copies superseded by that manifest. Retired paths
and checksums are recorded locally. Files created or modified locally are never
removed or overwritten by this reconciliation. Thus failure snapshots do not double
the space used by previous live pulls. Local source/configuration/cost files and the
small receiver audit records are outside the evidence ceiling; filesystem headroom
checks account for their actual disk use. Final files are checked against the terminal
manifest before installation as well as by the launcher's final manifest verification.
