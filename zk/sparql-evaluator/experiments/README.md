# Exact-evaluator experiments

[GPT-6] `exact_experiment` runs fixed synthetic V2 SELECT, false-ASK and complete
named-graph catalog contracts with the existing local RISC Zero host API. It is an
experimental measurement adapter, not an external cryptographic audit or a claim
of full SPARQL support. The [evaluator's assurance limits](../README.md) apply.

## Run

Build with the [pinned toolchain](../README.md), then run the example with five
positional arguments:

```sh
cargo build --locked --release --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --example exact_experiment

zk/sparql-evaluator/target/release/examples/exact_experiment \
  zk/sparql-evaluator/experiments/smoke.json \
  /accepted/guest.bin /accepted/pin.json /installed/r0vm /tmp/new-exact-experiment
```

The artifact and pin are independently accepted caller inputs. Establish their
source provenance and accept the expected digest and image identity before the
run; deriving a new pin from the artifact being offered is not independent
acceptance. The example checks both identities and records the exact input pin.
The existing host dependency builds an embedded guest during compilation; this
example does not select that embedded artifact. Compilation is outside sample
timers. There is no hosted prover, fake receipt or development-mode fallback.

The output directory must not exist and must be outside the clean source
checkout. Manifest, pin and guest reads have explicit byte bounds. The manifest
admits only the three fixed fixtures and two explicit authority modes, rejects
duplicate contracts and unknown fields, and bounds repetitions and warmups. The
published salt, dataset and nonce derivation are synthetic test inputs. Change
`run_id` between campaigns; its lowercase hexadecimal value separates challenges
between runs. Within a run, every contract and repetition gets a distinct nonce.
This is not a production nonce generator or an issuer-authentication adapter.

## Report contract

`report.json` is written only after every real proof, independent verification,
fixed-result check and typed rejection control succeeds. A failed run exits
nonzero and may retain completed sample artifacts, but has no complete report.
Each sample saves the actual presentation, verifier-owned expected request and
verified journal, with a digest of the serialized presentation in the report.
Warmups are labeled and retained; there is no silent trimming or summary statistic.

The semantic contract digest includes the fixed query, exact input commitment,
catalog, dialect, policy, expected result and authority mode. It excludes the run
nonce so repetitions have one comparison identity. Exact results and the separate
selected-support Noir experiments assert different statements and must not be
compared as equivalent proofs. A holder-declared dataset and a verifier-agreed
dataset also have different contract identities. Neither mode authenticates an
issuer or a credential's status. Holder-declared completeness is relative to the
holder's committed input; its root is a proved output, not an independently
accepted root.

Measured stages are fixture/request preparation, the inclusive proving API
(including its internal verification), and a separate independent receipt
verification. Prover execution, proof generation and compression cannot be
isolated by this API and are `null`; compilation and artifact acceptance occur
outside these timers. Peak process-tree RSS and observed HAL are also `null`.
The host's architecture is not evidence of GPU use. Stages are explicitly scoped;
they are not interchangeable with the Noir adapter's subprocess events.

Sizes distinguish succinct seal words converted to bytes, journal bytes,
presentation JSON, expected-request JSON and canonical-result JSON. These are
different encodings, not additive transport measurements. The report preserves
the outer receipt control identity and does not imply normalized receipt metadata.
The [upstream assurance and metadata caveats](../../../skills/zk-query-proofs/references/exact-evaluator.md)
remain in effect. All timings are NONcanonical local observations; caches are
retained, and no cold-cache or speedup claim is made. Debug-build observations are
explicitly marked and cannot establish release performance.

Runtime checkout HEAD/tree and observed `rustc` version describe the environment
at execution, not compile-time attestation of an arbitrary executable. The exact
adapter executable, compiled-in lockfile and actual local `r0vm` executable have
separate digests. Build command and source-to-binary provenance must be preserved
by the runner independently. The accepted artifact's source is deliberately
unknown in the report unless established by that external provenance record.

## Rejection controls and tests

Every actual sample independently alters expected query bytes, nonce, authority
and dataset root; it checks consumed-nonce replay and mutates the authenticated
returned mappings or ASK value, committed root and succinct seal. Artifact bytes, expected
artifact digest and expected image identity have separate admission controls.
Each must return the intended host error value. A prover process failure, malformed
test input, different error or unexpected acceptance fails the experiment.

For holder-declared input, the changed expected root control necessarily changes
authority to verifier-agreed as well; the separate authenticated-root mutation
checks output integrity without implying an independent holder root policy.

```sh
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --example exact_experiment
```

Native tests execute all six fixture/authority combinations against fixed goldens,
manifest bounds, distinct contract/nonce identities and wrong-error rejection.
These tests do not generate receipts. The example CLI generates genuine receipts;
its completed report is separate evidence. Arbitrary queries, additional datasets,
credential suites, internal prover stages and optimization ablations are outside
this adapter's admitted experiment profile.
