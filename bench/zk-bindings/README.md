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
