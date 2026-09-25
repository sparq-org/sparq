# Proof binding corpus and replay

[GPT-6] The shared controller retains independent original goldens, query bytes,
source hashes and per-backend coverage denominators. Selected support and exact
whole-result contracts are distinct. Native evaluation always counts zero proofs.
Tests do not establish a cryptographic security or privacy guarantee.

This semantic integration imports the shared Python controller, exact model
example and W3C exporter from reviewed harness commit
`a1dce9168de3c1fb03cdfed50d8c81f83ffcc232`. It does not import Noir/native runtime
history, their adapters, their CI workflows or their historical proof evidence.
The broader [inventory](inventory.json) records configured surfaces; unavailable
adapters fail explicitly and are never counted as executed or unsupported.

```sh
python3 -m unittest discover -s bench/zk-bindings -p 'test_*.py'
cargo run --locked -p sparq-conformance --example proof_corpus -- \
  /existing/suite/manifest.ttl /existing/suite /tmp/original-w3c.json
python3 bench/zk-bindings/run.py plan --corpus /tmp/original-w3c.json \
  --tier native --backends exact_v1 exact_v2 --output /tmp/w3c-plan.json
```

The W3C exporter reuses the original manifest, RDF and result readers without
executing queries. Imported JSON fixtures preserve original IDs and goldens;
row-only fixtures require explicit original projection variables. Capacity
controls remain separate from normative expectations. The independent finite
oracle covers a declared tiny domain; sampling and sharding retain their exact
configured versus executed denominators. Neither implies full SPARQL coverage.

[protocol.json](protocol.json) binds jobs and outcomes. Adapter configuration
pins the executable, source checkout and hashes. The controller rejects missing
jobs, unknown rejection classes, mismatched results, unrelated parse failures,
timeouts and missing artifacts. `native`, `constraint` and `real` lanes remain
separate. Genuine proof evidence requires actual generation and independent
verification; honest preparation refusal does not establish constraint rejection.

The native exact example requires model feature `evaluate`. Original rejection
categories follow [rejections.json](rejections.json); a combined legacy diagnostic
is deliberately unclassified. [Versioned expectations](version-expectations.json)
bind each promoted result to the unchanged original fixture and dataset hashes.
Blank-node comparison uses one global bijection and preserves row multiplicity.
Graph canonical bytes must match their independently defined fixture; a mismatch
is never repaired by per-row relabeling or literal normalization.
