# Synthetic result experiment

[GPT-6] This first adapter uses the existing `successful-results` capability and
its pinned Noir/Barretenberg driver. It measures selected successful support,
without asserting complete answers, absence, holder identity, or external security
assurance. The fixture deliberately releases Alice while withholding another
supported answer; Bob fails the hidden predicate.

```sh
cargo test -p sparq-zk-compose --features successful-results --example result_experiment
cargo run --release -p sparq-zk-compose --features successful-results \
  --example result_experiment -- \
  bench/zk-compose/experiments/synthetic-selected-support.json /tmp/new-experiment
```

Commit source first and choose a new output directory outside the checkout. Only
validated fixed or generated synthetic wallet profiles are accepted; their deterministic keys, salts and
challenges are test data. Never adapt those values into credential issuance or a
production nonce policy. The manifest rejects unknown fields, changed contracts,
unsupported signature/status/disclosure regimes, duplicate planners, and excessive
run counts. There is no external wallet loader or signature conversion.

The fixed tiny-integer result version and status depth are part of the comparison
contract; the allowed credential bucket is one or two slots.
The baseline and joint planner consume identical query text, released RDF terms,
wallet contents and verifier-owned issuer/status policy. The report hashes that
acceptance contract independently of run nonces and local planner choices, checks
it after every run, and verifies exact released mappings. Changes in capacity or
issuer-slot metadata remain visible in each stored public transcript. A smaller
capacity may reveal that one credential suffices; this is not identical-disclosure
padding. The baseline selects two credentials and the optimized path can select
one shared credential. These structural counts are not a latency prediction.

`report.json` records every attempted run as success or failure and distinguishes
warm-up runs from measurements. Timers cover complete host preparation, the
inclusive prove API, and the inclusive independent verify API. Proving includes
compilation, witness execution, proof/key generation and artifact handling;
verification includes canonical compilation/key derivation, public reconstruction
and verification. Schema version 2 also records actual driver events inside each
inclusive API span: workspace-lock acquisition, Nargo compilation and witness
execution, private ACIR copying, backend proof/key generation, independent key
generation and backend verification. Child spans must not be added to their parent
timer. Uninstrumented host setup and I/O remain in the inclusive timer. Fixture
issuance is measured separately; RSS and the internal backend proof/key split
remain null with reasons. Other signature suites are explicitly unavailable.

A no-warm-up run is not a cold-cache measurement. The report records prior runs in
the process and completed full warm-ups for that planner; operating-system and
Nargo dependency caches remain uncontrolled. Compiler-internal cache hits are
unavailable, even when ACIR bytes repeat. The optional immutable-snapshot event
reports reuse only after reading and verifying the existing bytes; this successful
result adapter uses private copies instead of that snapshot API. Rust compilation is outside all
reported timers. Local output is **NONcanonical**. Its source/tree/lock and compiler
fields describe the clean checkout and toolchain observed at execution time; they
are not compile-time attestation for an arbitrary prebuilt executable. Adapter
and prover executable hashes identify the actual binaries separately. Rebuild at
the recorded checkout before running, as in the command above. No cross-system
timing comparison or calibrated performance claim follows from one local run.

Each run stores actual binary proof bytes, compact public JSON without the proof,
and the full JSON presentation. Their byte counts are separate: JSON-encoded proof
arrays have wire overhead, and verifier policy inputs are already held out of band.
The report includes proof/artifact hashes. The verifier derives its own key; this
adapter does not export that key or expose private witness material. The report's
full wallet binding is synthetic experiment metadata, not public proof disclosure.

The first measured run of each planner executes tamper controls for proof bytes,
released terms, requested query, a changed challenge on both sides, and the
verifier-owned status snapshot. Every run checks replay rejection. These checks
are excluded from normal prove/verify timers; typed spawn and host-I/O errors
cannot count as successful rejection controls. A nonzero `bb verify` exit is recorded as
`rejected` by the driver, without claiming to distinguish invalid proofs from all
backend failures. Metrics are enabled explicitly with
`CircuitProver::with_stage_metrics()` and drained with `take_stage_metrics()`;
collection has no global state and retains at most 4096 events per drain. Overflow
or collector poisoning prevents a successful measurement record. Concurrent users
need separate driver instances for per-call attribution. These local diagnostics
can disclose workload information and do not belong in real presentations.
Schema-version-2 manifests admit a materialized wallet with a namespace seed,
candidate count and complete credential facts. The adapter independently regenerates
and validates this bounded synthetic profile before signing its commitments. Each
generated manifest selects one planner and one ordinary sample with a supplied
synthetic nonce. RSS measurement, canonical infrastructure and additional signature
suites are not provided by this adapter.
The [common workload campaign](../campaigns/README.md) materializes these inputs
and interleaves both planners with retained per-sample outcomes.

The historical schema-version-1 [local smoke record](local-smoke.json) binds its actual source
commit and executable hashes. It retains separate public/proof byte counts and
noncanonical inclusive timings; it is a fixture smoke test, not a statistical
comparison or publication benchmark. Original binary artifacts remain with the
run export and are identified by both report hashes and artifact SHA-256 digests.

The [schema-version-2 stage smoke record](local-stage-smoke.json) preserves the
actual nested driver events from both planner runs. It records the explicit
release rebuild and execution checkout separately from the unchanged runtime
source checkpoint, plus hashes for the retained executable and artifacts. Both
runs passed verification, replay and the configured tamper controls. The local
measurements establish working instrumentation, not a performance advantage.
