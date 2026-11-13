# Draft envelope fixtures

These are isolated gadget tests and measurement circuits for the statement draft. They are
research grade and not externally audited. Authenticated metadata bindings remain explicitly
marked stand-ins in `core/src/envelope.nr`. They do not produce a query presentation.

Run from the repository root:

```sh
python3 zk/noir-query/envelope_tests/run.py
python3 zk/noir-query/envelope_tests/run.py --measure --output /tmp/envelope-gates.json
```

The runner requires the exact programme toolchain and runs nargo with warnings as errors, using
a single test thread. Measurement circuits keep selected issuer, holder, credential and status
data private. The runner checks their compiled ABI against the documented public inputs before
invoking the backend. Measurement output belongs outside the repository.

The packages depend directly on the registered core library. No library sources or package
metadata are copied. The runner retains the exact toolchain pin and checks the compiled public
ABI before every backend measurement.

The tests use hand-built cells, exact typed whole-second offset fixtures and small synthetic
snapshots. These snapshots test the Merkle and policy relations; they are not W3C status-list
credential encodings. Final snapshot admission must authenticate the W3C list, its minimum
length, purpose, bit order and freshness. The supplied creation instant is suite metadata in the
stand-in; the final suite opening must authenticate it. The suite supplies IRI encodings and
vocabulary identifiers. The key/index, time metadata and graph-name fixtures remain marked
stand-ins for their metadata contracts. The signature checks use the existing Schnorr relation.
The fixture signer uses fixed test scalars and nonce solely to generate test evidence.

The two-issuer case checks GRAPH/verifiedIssuer pins on the complete constructed quad tables; it
is not generated-query execution. Domain controls check the holder signature against another
public request digest. The combined case checks every envelope condition over four credentials,
then merges their complete datasets. Default triples are set members; named copies remain in
each credential graph. Freshness has its own inclusive bound. Holder requirements can select
issuers and share one signature across credentials naming the same key. Policy roots sort unique
unsigned field encodings and use the smallest zero-padded binary tree. Actual compiler envelope
wiring, complete production ABI checks, dataset commitment, revealed signatures, the shared
protocol adapter and live proofs remain integration work under the programme issues.

The request digest is supplied as public bytes. Each measurement circuit declares the agreed
public policy group in the same order, and the runner checks names, types, widths and array
lengths. No circuit derives a policy or request digest. The `request` probe now measures
canonical public-policy checks only. Disabled requirements have zero value words. Set-membership
paths use a public depth bounded by their compiled capacity; unused siblings and index bits are
zero. The status-bit depth stays inside each snapshot digest. The set-root helper is confined to
the native test module. Its fixed vectors also exercise the Rust policy helper; generated entry
points consume verifier-computed roots.
When holder binding is enabled, an absent issuer list selects all trusted issuers.
A present list must be non-empty; both the host and circuit refuse an empty list.
To require no holder keys, omit `holderBinding`.
