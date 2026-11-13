# Draft envelope fixtures

These native gadget tests and measurement circuits exercise authenticated credential envelopes.
They are research grade and not externally audited; they do not produce a query presentation.

Run from the repository root:

```sh
python3 zk/noir-query/envelope_tests/run.py
python3 zk/noir-query/envelope_tests/run.py --measure --output /tmp/envelope-gates.json
```

The packages depend directly on the registered core library. The runner requires the exact
programme toolchain and denies warnings. The runner discovers every native case and checks that its partitions execute each exactly
once. Four-credential cases run serially; other cases use the compiler’s default test parallelism. Before each backend
measurement it checks the complete public ABI: names, order, types, widths and array lengths.
The request digest is public, followed by the agreed public policy group. Private measurement
output belongs outside the repository.

`tests/src/fixtures.nr` is reproduced by the lane E Rust example:

```sh
cargo run -p sparq-zk-query --example envelope_fixtures > zk/noir-query/envelope_tests/tests/src/fixtures.nr
nargo fmt --program-dir zk/noir-query/envelope_tests/tests
```

The generator uses the suite's seeded key and deterministic issuance functions, verifies every
issued credential, and emits raw signed openings plus Schnorr evidence. Tests call the actual
hidden suite opener. They do not supply a separate metadata transport. Creation, verification
method and key come from the signed proof metadata; status purpose/index and validity lexical
openings are checked against signed cells. Instants use exact Unix epoch milliseconds with no
rounding. The tests include millisecond endpoints and unknown creation.

The small status trees exercise tagged leaves, depth-bound snapshots and membership. Snapshot
admission must still authenticate the W3C list, minimum length, purpose, bit order and freshness.
The native four-credential case covers shared/distinct holder keys, issuer scope, both status
purposes, validity, freshness, dataset merge and verifiedIssuer assertions. Default triples are
set members; named copies remain in each credential graph. The graph builder uses a sorting
network and a merge with indexed writes. Its compiled sorting depth is determined by the quad
bucket; it is checked against the candidate-row capacity and is not a public input. Graph names use the public request
digest and credential index. Different request scopes change them; a private scope is absent
from the entry ABI.

An absent holder issuer list selects all trusted issuers when holder binding is enabled. A
present list must be non-empty. The host and circuit refuse empty lists. Omit holderBinding to
require no holder keys. Public policy roots sort unique canonical unsigned field encodings and
use minimal zero-padded trees. The canonical-set helper stays in the native test module;
generated checks receive verifier-computed roots and depths. Disabled conditions have zero
value words; unused set-path levels are zero and do not enter the root.

Measurement probes use the Solid credential and quad capacities. `request` measures signed
opening plus policy checks; individual probes add their named check; `total` adds the complete
policy envelope. `dataset` measures graph assembly separately. The native case includes the
full merge. Every probe keeps credential metadata and membership choices private. Generated
query enforcement and admission remain integration work for the compiler and protocol lanes.
