# Query compiler architecture

This research implementation is not externally audited. The query method uses
a canonical bounded plan, an interpreter over complete credential tables and
a deterministic generator that wires library gadgets into a query program.

The plan assigns variables by first encounter and gives every constant occurrence
a public parameter slot. Its tagged, length-prefixed shape encoding contains
operator wiring, input references, output schemas, compiled capacities, result
form, parameter kinds and canonical orders. It omits constant values and variable
names, allowing the same program to serve requests with the same shape. Exact
query text belongs to the stored request and its digest.

Public request bounds choose the smallest supported credential, quad and table
buckets. Private cardinalities only cause a refusal; they cannot choose another
key. A bucket reveals an upper bound and the query structure. Each intermediate
table is checked against the public request's row bound as well as its compiled
capacity. A limit on the final result does not waive an intermediate bound.

Each table contains a bounded row array and its actual length. Padding uses the
canonical unbound cell in every column. Operators order rows lexicographically
by bound status, field term encoding and credential scope in schema order.
Projection retains multiplicity and sorts the projected rows; DISTINCT removes
equal mappings. Scan collapses duplicate triples in the default merge before
matching a pattern. Blank nodes from separate credentials remain distinct.
Join implements compatibility on bound shared variables, including VALUES UNDEF.

The initial expression seam carries explicit value and error lanes for key
comparison, boolean connectives and membership. FILTER retains only a true,
non-error root. Unimplemented operand families receive named refusals rather
than a different comparison. Expression-library integration uses the same seam.

The crate README records the public-input layout. A shared schema in
`statement::Layout` produces declarations and
independent verifier reconstruction, and checks the locally compiled ABI before
key creation. The generated main has no public return value. Intermediate tables,
complete credential cells and the presentation salt are private. The statement
binding absorbs every public word in ABI order through the shared Poseidon sponge.
The credential opening and supplied-cell commitment remain the programme's
explicit draft stand-in until the suite's authenticated messages are integrated.

The request wrapper materializes a query request as RDF and hashes its RDFC
canonical N-Quads. Its method parameters include the plan hash, exact public bounds,
the explicit draft profile and the named local encoding of the typed protocol
request. This binds all protocol fields without treating a structural encoding
as RDF canonicalization. The exact native key is a request property. The holder
refuses a mismatch before evaluating private cells. The verifier uses its stored
query, key and parameter values, independently encodes the released RDF terms,
checks dataset agreement and freshness, and consumes the original shared protocol
challenge last. Unsupported suite, status and holder policies are named refusals
in the draft profile. Lane E supplies the public adapter and envelope.

The cache is indexed by plan shape and invalidated by a fingerprint of the trusted
library sources and pinned dependency declarations. It compiles from private source
snapshots. Cached keys are never accepted from a presentation or loaded as trusted
serialized cache bytes. Independent native compilations check ACIR and key identity.
The primitive lives in the proof driver module with method re-exports so the adapter
lane's owned files remain available for its implementation.

The corpus manifest preserves the evaluator inventory. Each passing case executes
its generated program, agrees with both the engine and the recorded expectation,
and rejects altered private rows, result cells and request inputs. A separate floor
prevents silently dropping those claims. Lane fragments replace named inventory
rows and are subject to the same runner, rather than establishing circuit coverage
from a host-only result.

The generated-package adapter uses the existing proving driver, private witness
lifecycle, filename validation and pinned-version matcher. Verification keys are
derived from local generated source. The composed manifest is superseded by the
query method.

The pinned Node compiler measurement did not complete on the unmodified generated
program and library sources: nested hash-library modules were not resolved. Node
key creation from the native ACIR, with the explicit Wasm backend and prefetched
dependencies, produced the native key. This does not establish combined Node
compilation identity. Following the design's fallback, Node delivery uses
precompiled published shapes and native compilation for new shapes. The delivery
package is outside this lane.

The admitted table ceiling follows complete native compilation measurements for
the core operators. A larger join probe did not complete on the lane machine,
including a serial retry, so requests above the validated ceiling receive a named
capacity refusal. No result is truncated to fit that ceiling.
