# Draft envelope fixtures

These are isolated gadget tests and measurement circuits for the statement draft.
They are research grade and not externally audited. The credential suite and
stored-request bindings remain explicitly marked stand-ins in `core/src/envelope.nr`.
They do not produce a query presentation.

Run from the repository root:

```sh
python3 zk/noir-query/envelope_tests/run.py
python3 zk/noir-query/envelope_tests/run.py --measure --output /tmp/envelope-gates.json
```

The runner requires the exact programme toolchain and runs nargo with warnings
as errors, using a single test thread. Measurement circuits keep selected issuer,
holder, credential and status data private. The runner checks their compiled ABI
against the documented public inputs before invoking the backend. Measurement
output belongs outside the repository.

The inherited core manifest contains a prerelease requirement that the pinned
nargo manifest parser refuses. The runner creates isolated package metadata and
copies the exact envelope and table source bytes into a temporary library; it
changes no source bytes or dependency pins. The temporary manifest points to the
existing signature and hash library. This keeps lane A and lane B files untouched.
The core module registration belongs to the later integration step.

The tests use hand-built cells, exact typed whole-second offset fixtures and
small synthetic snapshots. These snapshots test the Merkle and policy relations;
they are not W3C status-list credential encodings. Final snapshot admission must
authenticate the W3C list, its minimum length, purpose, bit order and freshness.
The supplied creation instant is suite metadata in the stand-in; the final suite
opening must authenticate it. The synthetic RDF vocabulary, key/index encodings, graph-name encoding and
request digest are not wire formats. The signature checks use the existing
Schnorr relation. The fixture signer uses fixed test scalars and nonce solely
to generate test evidence.

The two-issuer case checks GRAPH/verifiedIssuer pins on the complete constructed
quad tables; it is not generated-query execution. Domain controls check the
local request binding and holder signature. Actual query compilation, complete
production ABI checks, dataset commitment, revealed signatures, the shared
protocol adapter and live proofs belong to step two under the programme issues.
