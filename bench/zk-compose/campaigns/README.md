# Bounded workload campaigns

[GPT-6] The standard-library Python runner materializes synthetic inputs, then
runs the selected-support and exact-evaluator adapters in separate subprocesses.
It retains each scheduled outcome and raw sample. The profile is an executable
experiment definition, not evidence of performance or external security assurance.

## Generate before measuring

```sh
python3 bench/zk-compose/campaigns/campaign.py generate \
  bench/zk-compose/campaigns/profile.json /tmp/new-generated-campaign
```

`profile.json` specifies the namespace seed, workload scales, warmups, repetitions,
per-process timeout and whether to include unsupported combinations. Unknown keys,
duplicate scales, empty campaigns and scales beyond the admitted profile fail;
inputs are never truncated. Generation writes complete materialized fixtures and
an interleaved schedule with exact input SHA-256 digests. It executes no prover.
The runner re-derives and byte-checks these inputs and schedule before execution.
The Rust adapters independently validate the materialized synthetic profile.

Wallet scale means candidate credentials. Split name/age credentials precede
shared alternatives, each with a distinct padding statement. Each alternative
retains the same qualifying Alice answer and another supported answer deliberately
withheld from the release. First-success and joint selection consume identical
wallets, query bytes, released terms, issuer/status policy and allowed capacity
policy. They can choose different public credential capacities, so the comparison
has matched acceptance requirements and visibly different realized disclosure.
Deterministic Baby Jubjub test keys and status snapshots are synthetic provisioning
inputs; they are not conventional credential signatures or real issuer policy.

Organization scale means members in each of two named departments. Each member
contributes one source quad, and the complete graph catalog also contains an empty
graph. The query groups over the named graphs with an OPTIONAL pattern and counts
bound member subjects. Its independently derived golden is the configured member
count for each occupied graph and zero for the empty graph. The largest admitted
scale uses the existing source-quad capacity without changing guest policy.
Verifier-agreed and holder-declared modes remain separate contracts. Neither
provides issuer authentication. The holder's root is a proved output; the verifier
must independently accept the agreed root. Dataset salt and namespace seed are
published synthetic data, not production blinding secrets.

## Execute supplied adapters

Build the [selected adapter](../experiments/README.md) and
[exact adapter](../../../zk/sparql-evaluator/experiments/README.md) at a clean
checkout with their pinned toolchains. Record the build commands, source, selected
profile, executable digests and independently accepted guest provenance. Rust
compilation and guest artifact acceptance occur outside sample timers. Both
adapters preserve their existing fixed smoke manifest mode; generated campaign
mode is separately versioned and admits exactly one ordinary sample per process.

```sh
python3 bench/zk-compose/campaigns/campaign.py run \
  /tmp/new-generated-campaign /tmp/new-campaign-results \
  --selected /built/result_experiment --nargo /installed/nargo --bb /installed/bb \
  --exact /built/exact_experiment \
  --artifact /accepted/guest.bin --pin /accepted/pin.json --r0vm /installed/r0vm
```

Output directories must be new and outside the source checkout. No dependency
installation or cloud prover is performed. Binary paths are explicit; the runner
checks that PATH cannot shadow the supplied Noir tools. The artifact and pin must
already be accepted independently; a freshly generated pin is not independent
trust. Supplied executable and artifact digests are observations, not attestations
that those files were built from the currently observed checkout. The exact
adapter continues to require real local Succinct receipts and its independent
verifier, with development-mode disabled. Its upstream assurance and outer receipt
metadata limits apply unchanged.

A fresh random synthetic campaign identifier is used unless `--run-id` is supplied.
Do not reuse an identifier for a repeated campaign. Distinct sample challenges are
checked within the schedule, retained in raw manifests and excluded only from
semantic comparison identities. Every ordinary sample retains the existing
adapter's replay and tamper controls. A backend failure is not a successful
control. The selected adapter retains its documented limitation that a nonzero
`bb verify` exit does not distinguish every cause of rejection.

The schedule rotates workload order and alternates paired planner/authority order
between rounds. It is deterministic given the profile. Warmup samples are retained
and excluded from the measurement role; their adapter reports remain ordinary
one-sample reports. They can warm retained disk, backend and OS caches, but cannot
warm the next subprocess's in-memory state. There is no cold-cache assertion.

## Raw outcomes and comparisons

Each scheduled sample gets `record.json`, including failure, timeout, explicit
unsupported combinations and missing-tool failures. Completed adapter outputs,
process output files, command arguments and hashes are retained beside it. A killed
or failing process is never converted into proof acceptance. Timeouts kill only
the new child process group owned by that sample; output size is checked while
waiting and after exit, with an explicit capacity outcome. This is a sampled
observation, not a hard disk quota: a child can overshoot between checks. Spawn
failures remain distinct from host I/O failures after a process was created.
Completed samples are not discarded
when a later one fails. The campaign summary distinguishes a completed schedule
from a campaign in which every supported sample succeeded.

Wallet/exact and organization/selected combinations currently have no matched
adapter statement and can be recorded as unsupported without invoking a tool.
This is not an engine conformance result. No sample is replaced with a smaller
fixture, a different answer, a weaker authority mode or an unsigned reattestation.

The summary preserves raw stage reports and realized public transcripts. The
inclusive child-process timer contains adapter startup, fixture provisioning,
proof and verification, controls and output; it must not be added to the adapter's
nested timers. Internal unavailable stages, process-tree peak RSS and unobserved
HAL remain null. A child high-water memory value would not establish simultaneous
process-tree peak memory. Local timings remain NONcanonical; no cross-backend or
cross-authority speedup is computed. Statistical analysis must first group by the
full acceptance contract and measurement scope, while retaining visible capacity
differences. The full generated profile is not automatically executed by CI.

## Validation

```sh
python3 bench/zk-compose/campaigns/test_campaign.py
cargo test -p sparq-zk-compose --features successful-results --example result_experiment
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --example exact_experiment
```

The Python suite validates generation, schedule and nonce separation, exact input
binding, unsupported/failure retention, and report acceptance controls with
explicit synthetic protocol records. Those records are not measured proof data.
The existing example test gates validate both Rust generated-input modes, including
full-scale organization goldens and unchanged fixed fixtures. Native fixture tests
execute semantics; genuine campaign reports require separate actual adapter runs.

The [local smoke record](local-smoke.json) retains the original completed campaign
report and independently rechecked artifact hashes. It contains two wallet planner
samples and two organization authority samples, with no warmups, plus explicit
unsupported records. Rust adapters were built at commit `5512ee3`; the corrected
Python runner and observed checkout were `f18d96c`. Both are recorded separately
from the independently accepted historical `048cb43` guest artifact. That artifact predates the
known tiny-decimal EBV correction; these fixed queries do not exercise that case.
The run does not validate the latest guest or the full scale sweep, and it provides
no general-correctness or performance conclusion. V3 graph-result workloads remain
a separate required extension.
