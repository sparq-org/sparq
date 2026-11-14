# sparq-zk-query

Research grade Noir query compiler; not externally audited, unpublished and opt-in.

## ✨ Compiler contracts

Public request bounds choose the smallest fitting credential, quad and table buckets.
The key reveals those capacities and the query structure. Private counts cannot
select another bucket; exceeding a bound yields a named refusal, never truncation.
Separate public quad limits select buckets for the ordered credential slots. The
verifier also selects an `OpeningProfile`: `Solid` uses the compact resource
and object bounds; `LongObjects` admits holder Multikey lexicals with compact
resource and metadata bounds; `Extended` also admits larger resource identifiers. The selected profile is stored in the request and included in the plan hash.

Constants are public parameters. Canonical plan encodings include parameter kinds,
operator wiring, schemas, orders and capacities, while excluding parameter values
and variable names. Constant membership lists use a fixed parameter block and public
active length, so values and list lengths preserve the shape within the profile.
Rows are ordered lexicographically by term encoding in schema order; duplicates
remain unless DISTINCT is requested. Scans deduplicate complete matching bindings
within the emitted table. Stable sorting networks preserve the first payload of
equal bindings; DISTINCT compares adjacent rows after validating canonical input. The composed manifest is superseded by the query method.

The expression dispatcher uses the shared comparison, boolean and membership
gadgets. Its value, SPARQL error and capacity channels remain separate. Capacity is
asserted at the expression root and cannot be absorbed as a SPARQL error.
Value consumers admit integer, decimal, date, dateTime, boolean and string datatypes.
Identity operations use the authenticated kind, lexical form, datatype and language.
Unsupported value datatypes, excess bytes and value ranges receive named refusals.

## 🚀 Host contracts

Run `cargo test -p sparq-zk-query`. The default-off `ordered-json` feature selects
serde_json's insertion-order map backend for canonicalization regression tests;
run it with `cargo test -p sparq-zk-query --features ordered-json`.
`SPARQ_NOIR_QUERY_SEED` accepts an optional integer seed for the interpreter's
seeded differential test; unset, it replays fixed regression seeds. It affects test inputs only.

`method::Compiler` provides `prepare`, `prove` and `verify`. Choose an artifact root
outside an existing Nargo workspace; `with_crs_path` selects a writable local CRS
cache. The driver, toolchain pin guard and challenge-store contracts are shared.
Keys are cached by the plan and trusted library source. The holder compares the
request key with its local compilation before reading private inputs.

`Request::new_authenticated` stores the native key, authoritative issuer-set root,
shared protocol request and public bounds. `prepare_with_credential_quad_bounds`
and `Request::new_authenticated_with_quad_bounds` select distinct ordered slot
limits. `Compiler::prove` accepts complete `credential::Credential` inputs with
private lexical, signature and membership material. Its encoder argument remains
for compatibility; proof and verification use `credential::SuiteEncoder`.
Structural requests created with `Request::new` cannot produce an authenticated proof.
Use `prepare_with_opening_profile` and
`Request::new_authenticated_with_opening_profile` with the same profile for larger openings; `new_authenticated_with_envelope_and_profile` retains typed envelope
properties as well. Oversized lexical forms receive a capacity refusal.

The current suite profile opens complete credentials, checks hidden signatures
and issuer key membership. Source identities are key-independent; value consumers check complete active inputs, including rejected rows. Released RDF clears value-key metadata. Overflow refuses.
The verifier reconstructs the plan and constants from its stored request, re-encodes the released RDF answer,
and consumes the shared challenge after proof verification and a final validity check.
Supplied-cell programs are isolated operator fixtures.

The request digest is SHA-256 over RDFC canonical N-Quads of this method's request.
The exact typed protocol encoding is a named method parameter within that RDF.
Public inputs, in order, are request digest, salted input commitment, signature
mode, request bounds, constant cells, index parameters, canonical result, envelope
policy group, ordered credential quad bounds, revealed count/messages/keys and
statement binding. The envelope
group carries issuer root/depth, holder flags and issuer list, status flags and
snapshot root/depth, validity flag/time, and freshness flag/earliest creation.
`statement::Layout` generates declarations, reconstructs canonical big endian field
words, and audits the locally compiled ABI. Cells flatten as bound, kind, encoding,
key class, key, datatype, language and credential scope. SELECT tables flatten
padded rows before length. Hidden-mode signature witnesses and all openings,
presentation salts and traces are private. Revealed-mode messages and signer key
encodings are public; `RevealedInputs::verify` verifies their external signatures
before `Layout::reconstruct_revealed` builds the words. `prover_toml_revealed` renders
that circuit mode; the native `Compiler` driver currently accepts hidden mode.
The dataset commitment binds ordered authenticated suite messages under a method
domain and presentation salt, separately from the detached evaluator's profile.

`new_authenticated_with_envelope` stores typed policy properties and uses
`envelope_policy::derive` for admission and verifier reconstruction. Time values
are Unix epoch milliseconds. Present-empty holder issuer lists are refused.
The existing `new_authenticated_with_policy` accepts verifier-stored policy words.
Canonical policy inputs and enabled envelope requirements retain a named native
integration refusal until the statement lane's consumer is connected.
The dataset helpers retain named credential copies and a default merge with
method-generated issuer assertions. Their production wiring awaits the graph
dispatcher and canonical issuer trust consumer.

The host plan, lowering, interpreter, generator, layout, request and presentation
values build for WebAssembly. `method::Prepared`, `method::Presentation`,
`Request::plan` and `Answer::release` are portable; the subprocess Compiler is native.
Authenticated Node delivery follows the native catalogue and prover-backend decision.
The agreement ratchet compares native/WebAssembly bytecode and keys for each passing
operator plan, recording the approved XPath failure; committed manifests load unchanged.

Native execution uses the shared live-toolchain gate. Run it with
`cargo test -p sparq-zk-query --test execution -- --ignored`, or use
`cargo test -p sparq-zk-query -- --include-ignored` for the full live suite.
`SPARQ_NOIR_QUERY_PROOF_PROFILE` selects `full` (default) or `smoke` for the
native proof sweep. Hosted CI runs small authenticated and operator controls.
Workflow dispatch with `full_query_proofs` selects the complete core, corpus and
proof sweep with its default seed set. Differential execution checks operator
fixtures; authenticated altered-witness controls run separately.
`SPARQ_NOIR_QUERY_EVIDENCE`, unset by default, accepts an external JSONL output path
for native proof-test plans and measurements. Keep the output outside the repository.

A complete answer is relative to the opened dataset. Without a verifier-agreed
commitment the holder chooses that dataset. Repeated questions can reveal more
through their answers; the holder service supplies the policy hook.

## 📚 Architecture

See the [query compiler record](../../research/noir-query-compiler.md) and
[usage contracts](../../skills/zk-query-proofs/references/query-method-contracts.md).

## License

MIT.
