# Native RDF binding test adapter

[GPT-6] The detached `native-bindings` executable implements
`sparq.proof-binding-job.v1` for the experimental native RDF relation. Build it
with the existing pinned Circom compiler and repository Rust toolchain:

```sh
cargo build --locked --manifest-path zk/native-composition/Cargo.toml --features native-binding --bin native-bindings
zk/native-composition/target/debug/native-bindings INPUT_JSON NEW_OUTPUT_DIRECTORY
```

The feature is off by default. It exposes the already locked SHA-256 dependency
for job/artifact identities without changing any resolved dependency version.
The ordinary tuple CLI and default-off graph remain separate.

Each binding job supplies its original query, canonical default-graph triple
array, variable names without `?`, one candidate row, tier, nonce identifier and
corpus identities. The adapter does not use `expected_accept` to decide whether
to accept. It issues the supplied synthetic graph under a fresh local key,
independently imports that key into the verifier policy, and applies the native
relation to the unchanged query and row. Alternate N-Triples/N-Quads sources,
when supplied, must represent the same default graph; named graph catalogs are
not silently discarded. This is synthetic fixture issuance, not authentication
of an existing credential or a source identity.

The query/profile limits are those in [the native RDF contract](native-rdf.md):
nonempty SELECT DISTINCT BGP, every variable publicly projected and bound,
bounded blank-node-free signed graphs and public status/slot positions.
Unbound cells, hidden variables, other query forms, named graphs and unsupported
job operations receive explicit profile exclusions. Policy overrides and exact
dataset authority modes do not belong to this signed-support contract.

The native tier calls `prepare_public_bgp`, which shares the real prover's
allocation and disclosed-message checks. It generates no zero-knowledge proof
and does not authenticate a signature. The real tier constructs an actual BBS+
presentation, independently rebuilds and verifies its required statement, then
checks replay rejection using the same nonce store. Successful real jobs retain
`proof.bin` and the public context/key/support/nonce artifact, each with SHA-256.
Nonce derivation hashes the complete bounded input bytes under a dedicated
domain; its deterministic identifier is for tests, not production freshness.

For absent bindings, the native tier reports honest preparation's typed
`Support` refusal without a proof. The real tier additionally bypasses honest
preparation through the opt-in `rdf::binding_tests::prove_without_query_preimages`
helper. For its single-role, single-row fixture, it constructs a genuine BBS+
proof disclosing protocol/status while omitting all requested RDF preimages.
It independently verifies that weaker statement under the exact required claim
context and nonce, then requires `verify_public_bgp` to reject through
`Verification`. The retained `weaker-proof.bin` and public artifact record the
actual attempted claim. `proof_count=1` counts construction;
`verified_count=0` counts acceptance of the required RDF statement. The successful
weaker verification is a separately named control, never a valid binding.

Empty or oversized signed graphs retain the existing issuance `Capacity`
refusal at admission, with an explicit exclusion note and no attempted proof.
The finite scan/join corpus keeps these admission exclusions separate from
nonempty false candidates tested through the required proof verifier. This
specific omission family does not exhaust every possible malformed witness.
The mandatory library tests also require rejection of missing preimages,
unrelated issuer roles and other statement substitutions.
The legacy tuple/Circom-LegoGroth16 tests also remain mandatory; their bounded
integer relation is a distinct backend, not a general RDF binding adapter.

Malformed job structure, I/O, cryptographic construction failures and unexpected
verification/control outcomes fail the process. They never count as successful
negative cases. Output must be a new directory; fixed artifact names use
exclusive creation. The adapter records observed input identity, not a binary
build attestation. The calling campaign must pin and retain the actual executable.

Run the existing suites as well as the adapter controls:

```sh
cargo test --locked --manifest-path zk/native-composition/Cargo.toml --all-features
cargo clippy --locked --manifest-path zk/native-composition/Cargo.toml --all-features --all-targets -- -D warnings
```

The dedicated native workflow runs both library adversarial tests and genuine
adapter controls. These bounded checks are regression evidence, not an external
cryptographic audit or complete SPARQL proof coverage.

The [retained finite campaign record](../../../zk/native-composition/evidence/native-bindings-89a38eb.json)
binds the frozen adapter, harness, executable, corpus and actual artifact counts.
It records successful required proofs, same-context weaker-proof rejections and
empty-graph admissions separately, with the other query families explicitly
excluded. This is local finite-domain evidence. The native workflow runs the
listed library/adapter controls; it does not currently run the complete finite
campaign or establish continuous coverage of every proof backend.
