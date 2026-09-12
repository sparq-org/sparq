# Proved Sparq evaluator

<!-- [GPT-6] zkp-10.1; this is a bounded research implementation, not an audit. -->

This detached Cargo workspace proves execution of the actual Sparq evaluator on
one complete, bounded default RDF graph. It is an experimental complement to the
specialized Noir successful-result path. It is **not externally audited** and
does not establish a general SPARQL conformance, privacy, or soundness claim.

## Statement and authority

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
an independent expected request and the method ID generated from its own compiled
guest. It never accepts a method ID from the presentation. Only after cryptographic
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
including unsigned `+1` and `-0`. Finite numeric lanes, the `i64` integer-constructor
boundary and [exact temporal/year capacity](../../skills/zk-query-proofs/references/exact-temporals.md) have explicit limits.
EXISTS and NOT EXISTS bodies are restricted to BGP, join, UNION and pure FILTER.
Fixed path sequences lowered to BGP are included; residual path operators,
nested EXISTS, OPTIONAL/MINUS, binding operators, subqueries and modifiers inside
an EXISTS body are rejected. The rejection applies to these combinations, not
to those operators elsewhere in the query. This avoids silently selecting an
alternative to published SPARQL 1.1 substitution semantics while broader
correlation work remains in zkp-10.6. The [W3C discussion](https://github.com/w3c/sparql-query/issues/156)
describes the relevant errata and proposed alternatives; none is implicitly
enabled by this profile.
Nullable path composition is also excluded while absent-constant propagation is
repaired: nullable subexpressions below other path operators, or at a lowered
sequence's internal endpoint, are rejected. Ordinary root `*`, `+` and `?` over
non-nullable operands remain admitted. The coverage ledger retains the known
standard expectation separately from the rejection case.

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
| Complex/nested EXISTS bodies | rejected | corpus admission and actual guest rejection cases |
| Nullable path composition | rejected | corpus admission and actual guest rejection cases |

Test definitions are distinct from execution evidence: `model/tests/semantics.rs`
runs native semantic tests; `host/tests/real_proof.rs` generates genuine receipts
and exercises the actual guest. `host/tests/actual_builtin_edges.rs` executes the
shared builtin edge corpus in the actual guest; REC-derived expectations and
labeled integer-constructor capacity controls remain separate evidence. These
executions authenticate each case's optional N-Triples source, covering literal,
VALUES and stored-term arithmetic/error paths. They generate no individual receipts;
the false-ASK fixture also discriminates SUBSTR positions and invalid numeric facets.
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
