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

The expression bridge delegates typed comparison, boolean connectives and
membership to the shared expression library. It derives scalar values from
checked suite keys and keeps capacity separate from SPARQL errors. FILTER retains only a true,
non-error root. Unimplemented operand families receive named refusals rather
than a different comparison. The bridge does not expose equality tokens as lexical openings.

The crate README records the public-input layout. A shared schema in
`statement::Layout` produces declarations and
independent verifier reconstruction, and checks the locally compiled ABI before
key creation. The generated main has no public return value. Intermediate tables,
raw credential openings and the presentation salt are private. The statement
binding absorbs every public word in ABI order through the shared Poseidon sponge.
The production opening delegates lexical binding, complete credential roots and
hidden signatures to the shared suite library. Value consumers use its fixed
datatype checked-key constructors. Its private
issuer membership path uses the existing compose relation and host helpers.
The verifier-stored envelope policy and ordered per-credential quad limits follow
the result in the public layout. The group carries issuer and snapshot roots,
canonical holder restrictions, status flags and validity/freshness times. Its
values enter through the stored request seam; the circuit binds the request
digest and policy words without rederiving RDF policy. The dataset commitment hashes the ordered signed
suite messages under a private presentation salt. Operator fixtures retain a
separate supplied-cell relation without an authentication claim.

The request wrapper materializes a query request as RDF and hashes its RDFC
canonical N-Quads. Its method parameters include the plan hash, exact public bounds,
the explicit draft profile and the named local encoding of the typed protocol
request. This binds all protocol fields without treating a structural encoding
as RDF canonicalization. The exact native key is a request property. The holder
refuses a mismatch before evaluating private cells. The verifier uses its stored
query, key and parameter values, independently encodes the released RDF terms,
checks dataset agreement and freshness, and consumes the original shared protocol
challenge last. The authenticated constructor requires the supported suite and issuer-authenticated
source policy. Typed envelope properties are admitted and bound to canonical request RDF.
Typed policy inputs use the shared canonical policy derivation at admission and
again at verifier reconstruction. Validity and freshness instants are Unix epoch
milliseconds. A present-empty holder issuer list is refused. Enabled policy checks
receive a named native refusal until the statement lane's consumer is integrated.
The typed canonical relation is refused even at a matching issuer-only tree depth.
Lane E supplies the public adapter, trust membership and envelope consumer. The structural request constructor
cannot satisfy the authenticated proof path without a stored issuer root.

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

The initial native and WebAssembly compiler experiment agreed on bytecode and
independently created keys for its recorded operator source. The current pinned
XPath dependency has an approved named loading failure in its vendored hash alias.
The ratchet records that failure per affected passing plan, without asserting
bytecode or key agreement. An unrelated diagnostic or dependency revision remains
an error; a changed pin makes comparison mandatory again. Domain, datatype and
suite identifiers are literals checked against Rust computations.
The delivery package uses a catalogue of natively compiled authenticated plans;
native compile/key determinism remains required. Node derives the plan hash from
its stored query and refuses an unknown entry or different key. Holder witness,
proving and verification capacity remain separate acceptance checks.

The admitted table ceiling follows complete native compilation measurements for
the core operators. A larger join probe did not complete on the lane machine,
including a serial retry, so requests above the validated ceiling receive a named
capacity refusal. No result is truncated to fit that ceiling.

Credential slots can have distinct quad buckets selected from verifier-declared
limits. The shape hash includes each compiled bucket in slot order, while exact
limits are public parameters bound into the request and statement. Generated
programs declare each credential at its own type and call library assembly gadgets;
the generator emits no padding, filtering or authentication constraints inline.
The credential bucket includes the Solid acceptance paths. A larger quad bucket
without practical proof evidence is not offered.

The suite adapter uses the authenticated opener's returned table and salted
message. Creation and verification method enter as private openings, and the
native message uses the same suite metadata digest. Invalid metadata is a named
error from the fallible message and dataset-commitment helpers. Presentation-local
graph identities reuse the suite's request-scoped constructor. Input credentials
cannot supply the reserved method-generated issuer assertion.

Portable host modules cover plans, lowering, evaluation, generation, requests,
public-input reconstruction and presentation values. The compiler driver alone
owns native subprocesses and temporary files. The target-query dataset and native
capacity acceptance evidence remain integration work; they are not established
by isolated operator or compiler tests.

Constant membership lists occupy a fixed public term block and carry their active
length in the index group. Their values and length are erased from the shape;
the generator emits the indexed expression gadget call. Native controls rebind
the statement while retaining a trace that would match inactive padding, and
require the operator relation to refuse it.

Dataset assembly retains complete named copies and original signed counts. Its
default view merges those facts and the method's issuer assertions; ordinary scans
deduplicate the merged triples. The Noir representation keeps a parallel graph
array beside rows and active flags. Issuer terms enter only after the envelope
layer's trust check. The production method awaits that consumer and graph dispatch;
the assembly helpers alone do not establish a target-query proof.

The production adapter contains raw witness structs and resource normalization.
It constructs zero resource metadata and default graph openings, then calls the
suite's bounded credential entry point once per credential. The suite owns term
construction, packing, root/metadata binding and signature verification. B's value
consumers use fixed datatype checked-key calls with separate scalar bounds.
Shape encoding includes every resource, object and metadata capacity.

String comparisons preserve false versus SPARQL error. Equality tokens are local
to plain strings of the same datatype and never become character openings.
Language-tagged literals retain the selected Recommendation's RDF term equality
and error dispatch; explicit engine extension outcomes are recorded separately.
Temporal keys pass the suite's first payload word to the shared temporal lane,
with the remaining payload constrained to zero.
