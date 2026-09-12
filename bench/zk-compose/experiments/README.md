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
the built-in synthetic fixture is accepted; its deterministic keys, salts and
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
and verification. Neither timer isolates the backend. Fixture issuance is measured
separately. Unavailable compile/witness/backend-only/RSS measurements are null with
reasons. Other signature suites are explicitly unavailable.

A no-warm-up run is not a cold-cache measurement. The report records prior runs in
the process and completed full warm-ups for that planner; operating-system and
Nargo dependency caches remain uncontrolled. Rust compilation is outside all
reported timers. Local output is **NONcanonical**, with source/tree/lock, compiler,
adapter and prover executable hashes and version output. No cross-system timing
comparison or calibrated performance claim follows from one local run.

Each run stores actual binary proof bytes, compact public JSON without the proof,
and the full JSON presentation. Their byte counts are separate: JSON-encoded proof
arrays have wire overhead, and verifier policy inputs are already held out of band.
The report includes proof/artifact hashes. The verifier derives its own key; this
adapter does not export that key or expose private witness material. The report's
full wallet binding is synthetic experiment metadata, not public proof disclosure.

The first measured run of each planner executes tamper controls for proof bytes,
released terms, requested query, a changed challenge on both sides, and the
verifier-owned status snapshot. Every run checks replay rejection. These checks
are excluded from normal prove/verify timers; backend execution errors cannot count
as successful rejection controls. Internal stage instrumentation, RSS measurement,
canonical infrastructure, an exact-dataset adapter, and further signature suites
remain unfinished work.
