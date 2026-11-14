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
by bound status, authenticated term identity, kind and credential scope in schema order.
Projection retains multiplicity and sorts the projected rows; DISTINCT validates
canonical input order and removes equal adjacent mappings, retaining the first
complete payload in each run. Scan verifies its supplied canonical output directly:
every matching source binding must occur exactly once, every output binding must
have a matching source, and adjacent identity duplicates are refused. Every source
position is either a constant or an output column, so full bindings identify the
matching triples. Metadata comes from the first matching source position.
Projection and SELECT verify stable canonical permutations. Input occurrence
ordinals distinguish repeated identities; matching output ordinals select each
metadata field exactly once. This is a deterministic correspondence check, with
no private permutation index or probabilistic multiset compression. Input/output
padding, length, ordering and overflow remain constrained. The interpreter keeps
the same stable canonical order. The original sorting helper remains available
for other library consumers.
Blank nodes from separate credentials remain distinct.
Join implements compatibility on bound shared variables, including VALUES UNDEF.

The expression bridge delegates typed comparison, boolean connectives and
membership to the shared expression library. It derives scalar values from
checked suite keys and keeps capacity separate from SPARQL errors. FILTER retains only a true,
non-error root. Unimplemented operand families receive named refusals rather
than a different comparison. Same-datatype string equality delegates directly to shared term identity; mixed types retain value/error dispatch.

The read-site adapter validates value inputs separately from authentication. Source,
parameter and released-term identities use the suite's authenticated identity covering
kind, lexical form, datatype and language. The suite's keyed encoding remains part
of credential commitments. Production opens identities once. Private scalar byte/datatype bounds apply at
read admission, so unrelated source terms do not inherit a value-parser limit. It validates each
value-consuming literal on every active consumer input row. Plain-string equality
and membership use identity; typed-literal equality and membership still check keys. Private lexical reads
use a fixed-datatype suite constructor. Cells equal in every field to a public
request parameter reuse its verifier-reconstructed canonical key. It checks the entire reconstructed cell against that row's authenticated
encoding and metadata. No private source index chooses the binding, and rejected
rows are included. Request parameters are reconstructed publicly by the verifier.
The scan proves completeness of each bounded input table; intermediate capacity
refuses rather than truncates, independently of final projection or slice.
Released RDF cells preserve identity and datatype/language metadata and clear the
value key and its class. The interpreter, SELECT gadget, RDF answer conversion and
verifier reconstruction share that rule. A released term makes no value-key claim;
public request parameters retain their canonical keys for value evaluation.

The crate README records the public-input layout. A shared schema in
`statement::Layout` produces declarations and
independent verifier reconstruction, and checks the locally compiled ABI before
key creation. The generated main has no public return value. Intermediate tables,
raw credential openings and the presentation salt are private. The statement
binding absorbs every public word in ABI order through the shared Poseidon hash chain.
Production delegates lexical binding, complete credential roots and authenticated
metadata opening to the shared suite library. Hidden signatures call the shared
Schnorr verifier over that opening's authenticated message. Value consumers use its fixed
datatype checked-key constructors. Its private issuer path uses canonical trust-member leaves and the policy module's
shared builder and consumer.
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
milliseconds. A present-empty holder issuer list is refused. Generated programs
consume canonical trust membership and the shared holder, status, validity and
freshness relations. A raw private-evidence adapter builds their sealed observations
inside constrained code. Each credential opens once; its converted view, issuer
assertion and holder coverage come from that same slot. Holder signatures are checked
once after coverage is accumulated. The native method derives public policy from the
stored request and evaluates the same named credential graphs and default issuer
assertions. The structural request constructor cannot satisfy the authenticated
proof path without a stored issuer root.

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
A dedicated live test enumerates every distinct passing plan, independently of the
full altered-witness corpus. The ratchet records that failure per affected passing
plan, without asserting bytecode or key agreement. Every record binds the exact
native artifact digest; a bytecode-only record cannot satisfy key agreement. An unrelated diagnostic or dependency revision remains
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
construction, packing, root/metadata binding and authenticated signer metadata. The signature-mode adapter calls the shared
Schnorr verifier or binds externally verified evidence. B's value
consumers use fixed datatype checked-key calls with separate scalar bounds.
Shape encoding includes every resource, object and metadata capacity. The verifier
chooses Solid, the LongObjects profile for larger object backing, or Extended
for larger identifiers before
witness admission. Request reconstruction restores that selection; excess lexical
bytes refuse instead of selecting a larger circuit from private data.

String comparisons preserve false versus SPARQL error. Equality tokens are local
to plain strings of the same datatype and never become character openings.
Language-tagged literals retain the selected Recommendation's RDF term equality
and error dispatch; explicit engine extension outcomes are recorded separately.
Temporal keys pass the suite's first payload word to the shared temporal lane,
with the remaining payload constrained to zero.

Revealed-mode inputs extend the same authenticated layout after ordered quad
bounds with the evidence count, credential messages and suite signer encodings.
External signature verification admits the ordered evidence record before public
reconstruction; callers cannot replace its private message/key arrays. The circuit
opens each credential once and binds that opening to the public message and signer,
while inactive slots and hidden-mode evidence words are zero. The ABI audit checks
that every message slot is public. Both modes retain the existing statement hash
chain with the extra words in declaration order. The native Compiler continues to
accept hidden mode; the statement adapter owns accepted-mode admission and the
revealed presentation path.

The shared temporal_payload bridge extracts the suite time word and constrains
the remaining packed payload to zero. Temporal comparisons receive that scalar
word with the original key class retained separately. Native date and dateTime
range controls include epoch boundaries and compare the answer with the engine;
packed-word and bypassed-helper omissions exercise the comparison range guard.

The toolchain workflow separates the query hosted profile from the dispatch-only
full sweep. Hosted validation loads the committed core and generated fixture
manifests unchanged, checks every operator contract case and the read-site contracts,
builds both portable feature sets, and runs small actual authentication/proof
controls plus per-plan compiler agreement. The full profile runs the complete core,
corpus, expression and proof sweeps. CPU affinity serializes nested native tests.
Separate runtime ledger profiles require all entry/completion records for each
job's declared sites; their inventories partition the full toolchain estate.

The scalar bridge also exposes a class-aware temporal extraction. It subtracts
the checked class header and divides by the fixed payload scale, then constrains
the resulting word to its scalar bit width. The bounded reconstruction enforces
the full header and zero tail. Native suite keys pin that representation; the
existing byte-based helper remains available for callers that have no separate
checked class.

FILTER verifies its complete stable subsequence using computed prefix ranks and
fieldwise payload accumulation. The selected predicates admit only true,
non-error active rows. Input and output tables, exact cardinality and capacity
remain checked; the holder supplies no source or destination index.

A leading-column projection preserves the input’s canonical row order. It uses
`table::assert_ordered` to bind each row and its complete payload directly. Other
projections retain the stable canonical-permutation relation. Both paths check
cardinality, padding and capacity.

JOIN matches every compatible source pair to the first unused output occurrence
of its binding, retaining source-pair order for equal identities. The full payload
is bound at that occurrence. Complete pair coverage and complete output use
preserve bag multiplicity; an undersized output refuses. All occurrence choices
are computed inside the gadget.

Plain-string equality classifies its operands from the authenticated datatype
before value-key guards; identity-only comparison does not inspect unused keys.
