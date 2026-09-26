# Proved Sparq evaluator

<!-- [GPT-6] zkp-10.1; this is a bounded research implementation, not an audit. -->

This detached Cargo workspace proves execution of the actual Sparq evaluator on
one complete, bounded RDF dataset. V1 retains the default-graph relation; the
separate V2 schema adds a complete named-graph catalog, including empty graphs.
It is an experimental complement to the
specialized Noir successful-result path. It is **not externally audited** and
does not establish a general SPARQL conformance, privacy, or soundness claim.

## Statement and authority

The sections below describe the V1 profile unless explicitly stated otherwise.
The [V2 dataset API](../../skills/zk-query-proofs/references/exact-datasets-v2.md)
documents its separate wire schema, N-Quads/catalog commitment and GRAPH plus
local-snapshot FROM/FROM NAMED behavior. V1 commitment semantics are unchanged.
`coverage.json` inventories V1 only; its named-graph rejections do not describe V2.
V2 native and actual-guest test definitions are listed in the linked V2 reference.
The separate [V3 graph-result API](../../skills/zk-query-proofs/references/graph-results-v3.md)
adds bounded blank-node/graph results; its guest/receipt evidence is separate.

`ProofContract::SelectedSupport` describes the existing Noir answer-support API.
The evaluator rejects it: this guest implements `ExactDataset` only. Exactness
describes computation on the supplied dataset, independently of who chose it.

`DatasetAuthority::VerifierAgreed` requires the verifier's independently accepted
dataset commitment. The proof checks the complete input against that expectation.
Accepting a holder-invented commitment without an independent scope agreement does
not establish source authenticity, wallet completeness, or absence from a wider
dataset. A relying party must establish that boundary outside this library.

`HolderDeclared` is explicit and returns `Provenance::HolderDeclaredOnly`. It proves
the computation on the holder's declared input; it provides no issuer signature,
credential validity/status assurance, or claim that the input contains everything
the holder possesses. Applications must preserve this provenance distinction.
Credential and native proof adapters are tracked separately under zkp-10/zkp-12.

The dataset anchor is SHA-256 over a versioned format/domain tag, policy capacities,
a private 32-byte salt, a length prefix and the exact UTF-8 N-Triples source bytes.
This is an exact-document commitment, not RDF isomorphism canonicalization.
Equivalent serializations may have different anchors. Use cryptographic entropy
for the salt; an all-zero salt is rejected, which alone does not establish entropy.
The public anchor remains linkable when reused. Dataset length, source bytes and
salt are absent from the journal; policy capacities and results are public.

## Execution and result binding

The guest checks the anchor, parses every source triple, builds the dictionary
and indexes, parses the query with the pinned vendored parser, evaluates Sparq,
and commits its own result. There is no caller-provided result or prebuilt index
in the witness. Limits cover source bytes, source triple count before deduplication,
query bytes, admitted AST nodes, and materialized rows/estimated bytes. Exhaustion
rejects evaluation; it never becomes an empty or truncated successful result.

The journal binds a domain-separated digest of the exact query bytes, request
version, dialect, contract, authority, policy and challenge. The verifier requires
an independent expected request and the method ID of its independently accepted
guest artifact. It never accepts a method ID from the presentation. Only after cryptographic
verification and request binding does it atomically consume the application nonce.
Provide persistent storage through `Nonces`; an in-memory test implementation is
not replay protection across restarts.

`Policy` contains only resource capacities, checked by the guest. Binding those
fields does not enforce credential validity, revocation or other status rules.

SELECT results retain projection order and distinguish unbound cells from empty
literals. Unordered results use sorted encoded rows with duplicates retained.
An outer ORDER BY preserves the guest's resulting sequence and tie policy.
ASK publishes a boolean, including false after evaluation of the scoped dataset.
The first profile rejects source blank-node input/query terms and output values, so it
does not claim blank-node result canonicalization or graph-producing support.
Parser-generated existential intermediates for fixed-length paths are admitted
under a reserved, deterministic internal namespace and never enter `SELECT *`.

## Admitted profile and evidence

The surface is the versioned `SparqSparql11SnapshotV1` profile, with its implementation
pinned by the guest image. It is not a claim of complete SPARQL 1.1 or conformance
to the evolving SPARQL 1.2 draft. `model::admit` visits nested patterns and
expressions, including subqueries, aggregate operands and EXISTS bodies.
The SPARQL version identifies query syntax/operators. Numeric datatype facets
follow [RDF 1.1 / XSD 1.1](https://www.w3.org/TR/2014/REC-rdf11-concepts-20140225/#xsd-datatypes),
including unsigned `+1` and `-0`. The [numeric capacity guard](../../skills/zk-query-proofs/references/numeric-capacity.md)
enforces finite arithmetic limits as whole-query failures, separately from
lexical validity and direct RDF output. [Exact temporal/year capacity](../../skills/zk-query-proofs/references/exact-temporals.md)
has its own explicit limits.
EXISTS and NOT EXISTS bodies admit BGP, join, UNION, pure FILTER and the
[scoped published-2013 MINUS rule](../../skills/sparql-query/exists-minus.md).
[Captured BOUND](../../skills/zk-query-proofs/references/exists-bound-scope.md) is
rejected as an ambiguous published-2013 shape; body-local BOUND stays admitted.
Fixed path sequences lowered to BGP are included; residual path operators,
nested EXISTS, OPTIONAL, binding operators, subqueries and modifiers inside
an EXISTS body are rejected. The rejection applies to these combinations, not
to those operators elsewhere in the query. This avoids silently selecting an
alternative to published SPARQL 1.1 substitution semantics while broader
correlation work remains in zkp-10.6. The [W3C discussion](https://github.com/w3c/sparql-query/issues/156)
describes the relevant errata and proposed alternatives; none is implicitly
enabled by this profile.
Nullable alternatives and inverses preserve branch multiplicity and concrete
zero-length endpoints, including terms absent from the dataset. The
[bounded path profile](../../skills/zk-query-proofs/references/nullable-paths.md)
keeps nullable sequence intermediates and nested nullable quantifiers excluded.
The shared native endpoint repair predates this admission change; the new test
runner requires a newly built guest before any actual execution claim.

| Family | Admission | Evidence definition |
| --- | --- | --- |
| BGP, join, projection, bag SELECT | admitted | host semantics; genuine sequence and bag proof fixtures |
| DISTINCT | admitted | host semantics |
| OPTIONAL, MINUS, COUNT, subquery, ORDER/LIMIT | admitted | combined genuine proof fixture |
| VALUES, UNION, unbound | admitted | host semantics; holder-declared bag proof fixture |
| NOT EXISTS, true ASK, arithmetic/error | admitted | host semantics |
| false ASK, numeric FILTER, VALUES joined with root zero-length paths or MINUS, SUBSTR positions and numeric subtype errors | admitted | host semantics; genuine false-ASK proof fixture |
| Fixed sequence and alternative paths, positive correlated EXISTS | admitted | combined genuine proof fixture; native conformance cases |
| Negated property sets, including forward/reverse endpoint multiplicity | admitted | REC-derived expectations checked natively and in actual guest execution; separate from receipt evidence |
| Other paths and pure functions | admitted by AST | shared evaluator; no complete guest conformance claim |
| Built-in aggregates | COUNT; other operands must be bound terms | [restricted aggregate profile](../../skills/zk-query-proofs/references/aggregate-profile.md) |
| GRAPH, FROM/FROM NAMED, SERVICE, LATERAL | rejected | whole-AST negatives |
| NOW, RAND, UUID/STRUUID, BNODE, external functions | rejected | nested host and actual guest rejection fixtures |
| Source blank nodes, triple terms, directional literals | rejected | input/query/output checks |
| CONSTRUCT, DESCRIBE, UPDATE | rejected | admission negatives |
| Scoped EXISTS/MINUS bodies | admitted | preserved 2013 golden and direct guest runner; execution requires a new artifact |
| Other complex/nested EXISTS bodies | rejected | corpus admission and actual guest rejection cases |
| Nullable alternatives and inverses | admitted | preserved published golden and full-result guest runner; new artifact execution required |
| Nullable sequences and nested nullable quantifiers | rejected | corpus admission and guest rejection definitions |

Test definitions are distinct from execution evidence: `model/tests/semantics.rs`
runs native semantic tests; `host/tests/real_proof.rs` generates genuine receipts
and exercises the actual guest. `host/tests/actual_builtin_edges.rs` executes the
shared builtin edge corpus in the actual guest; REC-derived expectations and
labeled integer-constructor capacity controls remain separate evidence. These
executions authenticate each case's optional N-Triples source, covering literal,
VALUES and stored-term arithmetic/error paths. They generate no individual receipts;
the false-ASK fixture also discriminates SUBSTR positions and invalid numeric facets.
The aggregate, temporal and numeric-capacity test files exercise positive controls
and whole-relation rejections under both versions; these definitions require current guest execution.
The dedicated CI workflow runs every host test. No ignored
test or missing-tool shortcut counts as a successful proof run. Broader guest
conformance coverage and remaining features belong to zkp-10.

## Toolchain and use

Pins: RISC Zero SDK/build/server 3.0.6, guest Rust 1.97.0, host repository Rust
1.97.1. Both Cargo lockfiles are checked in. The build uses the local toolchain
selected by `RISC0_HOME` and requires an explicitly supplied local `r0vm` path.
It never delegates private inputs to a hosted prover.

[GPT-6] Narrow [SDK dependency patches](../../vendor/zk-sdk/README.md) remove unused
discovery dependencies, select derive macros only for the features that use them,
and update a tracing API without changing constraint arithmetic.
Both detached locks retain the SDK pin. The patch record preserves
upstream hashes/licenses and distinguishes synthetic API checks from guest proofs.

The build step uses the pinned SDK's opt-in `cargo_command` build API and its
`ProgramBinary`/image-ID primitives. It removes host compiler wrappers only from
the child, so host Clippy cannot substitute its host sysroot for guest std. The
actual guest build still runs during lint, and skipping compilation is rejected.
Explicit compiler flags remap repository and Cargo-home roots to fixed prefixes,
remap the working directory and omit caller-location detail. These flags are
supported by the pinned custom guest compiler; precompiled toolchain libraries
can retain their own upstream build metadata. Cross-platform
reproducibility is not established: prover and verifier deployments must agree on
the exact independently accepted guest artifact and compare its method ID. A source
revision or a presenter-supplied image ID is not a replacement for that agreement.

The two-checkout experiment in `reproducibility.json` produced different artifacts
and IDs even after sanitization on the same host/compiler. Fixed-path canonical
release builds are part of the stage-4 reproducible harness work. Use a common
approved artifact for deployment now:

```sh
cargo run --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --example export_guest -- ./reviewed-guest
```

The release operator independently reviews the resulting program and distributes
`guest.bin`; the verifier pins `pin.json` through its trusted configuration channel.
Never accept a pin merely because the proof sender supplied it. Both peers load
`AcceptedGuest::from_artifact(bytes, &trusted_pin)` and use `prove_with_artifact`
and `verify_with_artifact` with that object. The constructor checks the full-byte
SHA-256 before parsing ELF or computing the execution ID; this rejects unapproved
memory declarations before expensive SDK processing. These APIs work even if the
peers' local embedded guest IDs differ. `prove`/`verify` remain conveniences for a
single locally compiled artifact.

```sh
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator-model --features evaluate
RISC0_BUILD_LOCKED=1 cargo test --locked \
  --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator -- --nocapture --test-threads=1
```

For the proof command, set `RISC0_HOME` to the installed guest toolchain directory
and `RISC0_SERVER_PATH` to the real 3.0.6 server. The workflow demonstrates pinned,
checksum-verified installation. Library entry points are `prove(witness, r0vm)`
and `verify(presentation, expected_request, nonce_store)`. The SDK's mock mode is
disabled, fake receipts are explicitly rejected, and verifier context forbids it.

This backend distributes and accepts only succinct receipts. Composite receipts
expose intermediate segment/continuation information and are rejected, as are
mock receipts. RISC Zero's security model describes recursion as hiding raw
execution length, while noting an outstanding mathematical zero-knowledge argument.
Standard succinct receipts still expose the outer recursion `control_id`, which
can distinguish lift/join/resolve profiles and reveal some execution shape. This
adapter does not normalize that metadata or claim complete execution-length hiding.
It therefore makes no settled privacy claim. Include recursion's entire
cost in benchmarks. The source witness is visible to the local caller and prover
process. See the [upstream security model](https://dev.risczero.com/api/security-model#zero-knowledge-proving).
The upstream model targets perfect zero knowledge but explicitly cautions users
with critical privacy requirements while its mathematical argument is outstanding.
Its [open advisory](https://github.com/risc0/risc0/security/advisories/GHSA-5xgj-pmjj-gw49)
continues to list all versions. This is an inherited assurance limitation, not a
claim of a demonstrated attack on this adapter or of an advisory fix in 3.0.6.
Published [audit reports](https://github.com/risc0/rz-security/tree/main/audits)
cover their stated code and commits; they are not a blanket audit of this program.

The workspace is detached: normal engine/native/WASM builds acquire no proof SDK
dependencies. The target-specific clock and UUID/RAND exclusions apply only to
`target_os = "zkvm"`. Shared corrections to correlated EXISTS and path multiset
semantics also apply to the native evaluator.

The vendored parser's zkvm-only synthetic-variable allocator uses a monotonic
namespace outside SPARQL user-variable syntax instead of requesting randomness
for hidden aggregate variables. It does not provide query-visible randomness.
The separate `sparq-deterministic-paths` parser feature is enabled only by the
detached evaluator and supplies its existential path intermediates.

The mandatory CI lane exports the accepted executable, actual synthetic receipts
and source/toolchain/HAL evidence using the [campaign evidence contract](../../skills/zk-query-proofs/references/evaluator-evidence.md). Partial uploads are not success records.

[GPT-6] The shared raw-literal boundary checks numeric/boolean typed lexicals
verbatim and retains XML whitespace normalization for string constructors. The
separate `raw-literal-whitespace.json` matrix contains 32 complete result controls;
its native and actual-guest runners authenticate each case's source dataset.
Definitions of actual-guest tests are not execution evidence. The strengthened
false-ASK fixture includes padded raw boolean/numeric branches; earlier receipts
retain their original source scope. See [the semantics record](../../skills/sparql-query/raw-literal-whitespace.md).

`evaluate_detailed`, `v2::evaluate_detailed` and `v3::evaluate_detailed` expose
typed execution causes (`BudgetExceeded` and `EvaluationCapacity` are re-exported) without changing request or
journal encodings. Row/byte exhaustion, deadline, cancellation, exact numeric or
temporal capacity, ordinary whole-query failure, and existing relation rejection
remain distinguishable. Private engine diagnostic text is discarded. The legacy
`evaluate` APIs preserve their original generic execution errors, including V3's
distinct query and graph messages. Canonicalizer library failures are not inferred
to be capacity failures from their text. Native definitions in
`model/tests/versioned_evaluation_causes.rs` cover both authorities and graph forms;
actual execution at the integrated source remains separately required. [GPT-6]

The V3 shared guest regression runner retains the original builtin, temporal,
lexical, aggregate, EXISTS, nullable-path and dialect fixtures under agreed scope.
Its exact success/profile/capacity denominators are asserted in
`host/tests/actual_v3_regressions.rs`; these are actual-execution definitions, not
receipt evidence. Both-authority native replay and the genuine authority receipt
families remain distinct. V2 also executes the original VERSION controls. [GPT-6]

## Exact experiment adapter

[GPT-6] The opt-in [experiment adapter](experiments/README.md) uses fixed synthetic
V2 contracts, independently accepted artifact/pin inputs, genuine local succinct
receipts and separate verifier controls. Its measurements explicitly distinguish
inclusive API costs, unavailable internal stages and noncanonical provenance.

## Engine replay proof bridge

[OPUS-5.5] The experimental [engine replay proof bridge](../../bench/zk-bindings/engine-proof-replay.md)
prepares one retained native engine replay cell's unchanged `query.rq` and
`data.ttl` for the exact V3 relation. Native preparation tests
(`model/tests/engine_replay.rs`) check conversion, identity, request and anchor
binding against hand-derived expectations and produce no receipts. Genuine
receipts come only from the ignored `host/tests/actual_engine_replay.rs` test
or the example's `real` mode. Turtle-to-N-Quads conversion and original-file
SHA-256 checks are host experiment checks before proving; the guest does not
execute them and a receipt does not attest to them. The synthetic-only
`engine_replay_setup` example (`engine_replay_setup synthetic REPLAY_DIR PROFILE
EXPECTED.json RUN_ID NEW_OUTPUT_DIR`) writes the holder manifest and both
verifier manifests for the proof CLI and the ignored test. Its nonces come from a
caller-chosen `RUN_ID` and are test challenges only. It computes the agreed anchor
on the host from the original `data.ttl`. That anchor is test-setup trust input
from the same host that also acts as holder, not source authentication. The
setup checks both requests natively against the supplied expectation, which it
never rewrites, and it creates no proof. The ignored test's job requires a
`new_output_directory`. This must be an absent, absolute path outside the
checkout and the replay directory. The test writes each authority's verified
public presentation, journal and request there, owner-only on Unix, before the
controls run. It writes `summary.json` only after both proofs, all controls and
the omission check have passed. The summary does not certify a guest abort,
because the host's proof error is generic. The adapter's native gate has passed
(see the bridge page's execution status). No proof or verification result for
this bridge has been recorded yet.

## vcq query-method adapter

[OPUS-5.5] The optional `vcq` feature (off by default; `cargo test -p
sparq-proved-evaluator --features vcq`) adds `vcq::Risc0ExactV3`, a
`sparq-query-protocol` `QueryMethod` over the V3 relation. It uses
`v3::prove_with_artifact` and the existing checked V3 verification with an
independently approved `ArtifactPin` and `AcceptedGuest`. It supports bag
SELECT, boolean ASK and CONSTRUCT graphs, holder-declared and verifier-agreed
authority, and the fixed default V3 policy only. SELECT sequences, DESCRIBE,
explicit base IRIs, issuer authentication, status and holder binding are
rejected. The V3 nonce is derived from the `local-struct-v1` stored request and
selected descriptor. The actual query form is checked by the host-only
`v3::query_shape` helper. Result checks run on the verified journal before the
original challenge is consumed once through the shared store. The existing
public verify APIs keep their behavior. The registry sets `adapter_available: true`
for `method:risc0-exact` version 3 and these six tuples only; V1 and V2 stay `false`.
[OPUS-5.5] The ignored `host/tests/vcq_genuine.rs` defines the genuine-receipt run.
It uses an explicit `SPARQ_VCQ_PROOF_JOB`, an approved guest and pin, and a public
synthetic fixture. It proves six accepted tuples plus one receipt that the protocol
row bound rejects after the proof checks and before challenge consumption. Controls
reuse those receipts. A separate verify-only test re-checks the retained
row-bound receipt. One run at source `872c219ca` was independently certified, with
seven Succinct receipts: six accepted, one rejected as `capacity`. It was not a full
gate and is no benchmark. It authenticates no credential, status or holder, and the
adapter is not externally audited. See
[the adapter reference](../../skills/zk-query-proofs/references/vcq-exact-adapter.md)
for the record and its caveats.
