# Proof binding corpus and replay

[GPT-6] This experimental test harness preserves the distinction between selected
support and a complete result over a fixed committed scope. Tests do not establish
a cryptographic security or privacy guarantee. The [inventory](inventory.json)
lists backend contracts, original suites, host variants and configured coverage.

`native`, `constraint` and `real` are separate lanes. Only an actual proof followed
by independent verification increments proof counts. A missing tool, timeout,
process failure or missing adapter fails execution. Honest support refusal is
recorded separately from a direct malicious witness and a genuine proof rejected
by the verifier. Required legacy suites and adapters still marked pending in the
inventory have not been executed by this harness.

```sh
python3 -m unittest discover -s bench/zk-bindings -p 'test_*.py'
python3 bench/zk-bindings/run.py plan --exhaustive-tiny --tier native \
  --backends exact_v1 exact_v2 --output /tmp/exact-plan.json
python3 bench/zk-bindings/run.py plan --finite-noir --tier real \
  --backends noir_unsigned noir_signed --output /tmp/noir-plan.json
python3 bench/zk-bindings/run.py run --plan /tmp/noir-plan.json \
  --adapters /path/to/accepted-adapters.json --output /path/to/new-evidence
```

The exhaustive tiny universe enumerates every subset of the four possible
directed edges over two IRI nodes. Its seven query templates have an independent
finite relational oracle. Selected support checks every candidate tuple over
those nodes plus a fresh absent IRI; exact evaluation checks the whole expected
bag, sequence or ASK for both authority modes. Empty graphs and false ASK still
create test cases. `--seed` instead creates a reproducible sample and never claims
exhaustive coverage. `--shard`/`--shards` partition the full configured job list;
a passing shard alone is not the complete domain.

The smaller `--finite-noir` profile uses the two-edge cycle and both scan/join
queries. Every valid binding and every absent candidate is included. Additional
private witness attacks alter padding, graph lengths, active result lengths,
indices and reused support slots after solving a positive witness. They bypass
the honest planner and require an actual constraint failure, not a generic error.

## Original tests

The conformance exporter reuses the existing W3C manifest, RDF and expected-result
readers without evaluating the query. It retains original fixture IDs, query and
golden bytes, source documents and root manifest. It does not rewrite queries.
External bases, blank-node source scopes and unsupported contexts remain explicit
classification gaps until an appropriate adapter implements them.

```sh
cargo run --locked -p sparq-conformance --example proof_corpus -- \
  /existing/suite/manifest.ttl /existing/suite /tmp/original-w3c.json
python3 bench/zk-bindings/run.py plan --corpus /tmp/original-w3c.json \
  --tier native --backends exact_v1 exact_v2 --output /tmp/w3c-plan.json
python3 bench/zk-bindings/run.py plan --regressions /path/to/conformance.json \
  --tier native --backends exact_v1 --output /tmp/regression-plan.json
```

`--regressions` retains each original JSON fixture and its golden. Row-only
fixtures require `--variables` from the original runner; no projection is guessed.
Capacity controls remain distinct from normative goldens. Historical artifacts
with an admitted semantic defect must report a failure against the original
expectation; a newer profile exclusion cannot excuse the old program.

## Adapter and evidence contract

[protocol.json](protocol.json) defines input/output. Adapter configuration pins
`argv`, executable SHA-256, clean `checkout` and `source_commit`. A Rust test
adapter also sets `kind: rust_test`; its arguments must select the exact ignored
test. Non-native Noir configuration additionally pins both `nargo` and `bb` under
`tools`, each with absolute `path`, `sha256`, complete `version_stdout` and
`version_stderr`. The runner checks exact bytes, not a version substring.

The native exact example requires model feature `evaluate`. The Noir adapter is
`result::proof_bindings::run_job` in the `successful-results` test binary. Test
preparation uses synthetic issuer keys/data only. Generated TOML contains private
fixture inputs and must stay in a private evidence directory. Every outcome
echoes the job identity; proof artifacts are retained and hash-checked. These
hashes provide reproducibility records, not an independent build attestation.

The bounded reducer in `minimize.py` only minimizes generated finite-oracle data
while preserving query and original seed. It refuses to rewrite imported goldens.
Reports separate configured/executed counts and rejection stages. Graph result
isomorphism still requires the original RDF comparator adapter; it is never
substituted with per-row blank-node relabeling or a lossy literal normalizer.

## Required CI profile

The existing `zk-toolchain` job runs `ci.py` on its relevant paths and standing
schedule, using its pinned tools. A dedicated bounded step executes all native
cells and the complete finite Noir profile; exact counts and direct constraint
rejection stages are checked. Only the input-driven helper is excluded from the
generic `result::` sweep, and the dedicated step supplies all its jobs. Every
preexisting result test remains selected. The step has an explicit timeout inside
the existing job budget; timeout is failure, never successful negative evidence.

The SHA-pinned upload step retains checkout/PR/merge identities, locked build
commands, exact executable/tool hashes, plans, outcomes and synthetic artifacts,
including partial failures. The [local finite evidence](evidence/finite-noir-ff58.json)
pins its original source and controller; it does not establish hosted execution
of a later CI commit. Broader inventory shards remain configured and unexecuted.

Imported negatives retain the original rejection category, phase and diagnostic
in `expected_rejection`. The adapter classifies its actual model error independently
using the exact `rejections.json` allowlist. Parse errors cannot discharge capacity
or profile expectations, and a different phase or declared diagnostic fails.
Unknown historical classes remain classification gaps. The model's combined
`query evaluation or resource budget rejected` diagnostic is deliberately
unclassified; it cannot certify which cause occurred. These cases stay unresolved
until a typed production error API distinguishes capacity from evaluation failure.
The original golden objects and historical finite Noir evidence remain unchanged.

[Version-specific expectations](version-expectations.json) retain the three
original V1 dataset rejections while assigning explicit empty-bag V2 positives
for their unchanged default-only source. Each is independently derived from the
fixed local snapshot contract, pinned to the original fixture and source hashes;
no network retrieval or named-graph membership claim is implied. Both authority
modes execute each version's own expectation. No V3 promotion is inferred.

## Native finite CI replay

[GPT-6] The separate native workflow runs `native_ci.py` after the existing
tuple, RDF malicious-proof and adapter suites. It builds the release adapter
and replays the unchanged exhaustive native domain: 576 jobs, comprising
72 required proofs accepted, 468 genuine weaker proofs independently verified
then rejected by the required verifier, and 36 empty-graph admission refusals.
The other 80 query cases remain explicit classifications. None becomes a
cryptographic negative merely because honest preparation refuses it.

```sh
python3 bench/zk-bindings/native_ci.py \
  --binary /path/to/native-bindings --circom /path/to/circom \
  --output /path/to/new-native-campaign
```

The helper requires the pinned complete plan, every individual result, exact
rejection stages and named weaker-proof/replay controls. It rechecks retained
artifact hashes and query/row/issuer/status/nonce fields, rejects missing or
reused proof artifacts, and reconciles aggregate counts with the individual
records. It does not independently reimplement BBS+ verification: the actual
native adapter rebuilds and verifies each cryptographic statement. Python unit
fixtures test only these reporting guards and are never counted as proofs.

CI retains partial failures, Cargo build events, executable and tool hashes,
source hashes, plans and proof/public artifacts. Observed checkout/tool versions
are explicitly separate from a binary build attestation; CI build events provide
the build attribution. CI supplies `--cargo-events` and `--cargo-metadata`;
the helper requires one compiler artifact for the exact native package and
binary target and matches its resolved executable path to the supplied binary.
Missing, ambiguous or mismatched records fail. Manual callers may omit both
records, leaving the build-record field explicitly null. This linkage is not
an independent reproduction or signed build attestation. The record formats
follow the [Cargo JSON interface](https://doc.rust-lang.org/cargo/reference/external-tools.html#artifact-messages).
The workflow has a bounded timeout, whose exhaustion is
a failure. Wiring the replay does not establish a completed hosted campaign;
the retained historical native evidence remains bound to its original source.

The native workflow is always created on PR and merge-group events. Its input
selector runs the heavy steps for native workspace, shared corpus/controller,
local dependency, toolchain or gate changes; an explicit irrelevant diff leaves
a successful lightweight job. An uncertain diff runs the checks. `ci-summary`
requires this job's completed success, so missing, cancelled and skipped jobs
cannot pass the merge gate. Root `Cargo.lock` and exact-only harness files do
not alter this detached native workspace and do not trigger native proving.
