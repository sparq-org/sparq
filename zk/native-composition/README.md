# Native signature and residual circuit composition experiment

<!-- [GPT-6] Research experiment, not externally audited. -->

This executable exercises Dock's existing BBS+ and LegoGroth16 composition over
BLS12-381. It authenticates two independently issued message tuples, links their
hidden subject identifiers, and links the signed income and rent to a circuit
checking `income - 12 * rent >= threshold`. The public schema tags are disclosed
inside the signature proof. This is a bounded research prototype, not an audited
RDF credential adapter, Sparq query verifier, or new cryptographic construction.

It is a separate Cargo workspace. Its dependencies do not enter the ordinary
Sparq build, the lean engine, or the WASM bundle. No existing verifier is changed.

## Run

Install Circom **2.2.2** and use the repository's pinned Rust toolchain. The build
script compiles the checked-in circuit with `--prime bls12381 --O0`, and checks
the generated symbol map before allowing the first two private inputs to become
the committed witnesses. Generated artifacts stay in Cargo's target directory.

```sh
cargo test --release --locked --manifest-path zk/native-composition/Cargo.toml
cargo clippy --locked --manifest-path zk/native-composition/Cargo.toml --all-targets -- -D warnings
cargo run --release --locked --manifest-path zk/native-composition/Cargo.toml
```

The executable emits JSON with named cryptographic checks, proof size, R1CS
constraint count, and local smoke timings. Any unexpectedly accepted negative
case exits unsuccessfully. The timing fields are local correctness-run evidence,
not canonical benchmarks or evidence of superiority over Noir. Setup time is
separate from presentation construction. Presentation construction currently
includes witness calculation, WASM loading, and proof generation.
An example local run is retained in [the structured evidence](evidence/local-smoke.json).

This command fails before generating setup or proofs:

```sh
cargo run --release --locked --manifest-path zk/native-composition/Cargo.toml -- --backend noir-bn254
```

## Exact statement and public information

Each issuer signs a tuple `[schema, subject, amount]`. The income schema denotes
annual income; the rent schema denotes monthly rent. Both use the same unsigned
integer unit. The numbers used for schema tags and subjects in the executable are
synthetic fixtures, not RDF encodings or production identifiers.

The verifier supplies the issuer keys, signature parameters, disclosed schema
tags, circuit verification key, threshold, context and fresh challenge. Its proof
specification is constructed independently of the holder. Neither a holder's
suggested equality list nor a holder-selected issuer trust list is accepted.

The composite statement contains:

1. A BBS+ signature proof for the income credential.
2. A BBS+ signature proof for the rent credential.
3. A LegoGroth16 proof of the eligibility relation.
4. Dock `EqualWitnesses` constraints connecting the two hidden subjects,
   income's signed amount to the circuit's first input, and rent's signed amount
   to the circuit's second input.

Income, rent, threshold, and the nonnegative residual are constrained to unsigned
32-bit integers. This makes a negative residual's field representation fail the
range constraints. The largest absolute integer expression is far smaller than
the BLS12-381 scalar modulus, so modular reduction cannot turn a negative result
into an allowed unsigned residual in this relation. General SPARQL decimals,
numeric promotion, units, currency conversion and datatype errors are outside
this experiment's semantics.

The public presentation identifies two issuer roles, their schemas, the circuit
and threshold, the shared-subject relation and the fact that the eligibility
predicate holds. It does not send the fixture subject or amounts as public
inputs. This is an input inventory, not an independently established privacy or
unlinkability guarantee for Sparq. The experiment has no external cryptographic
audit and provides no revocation, validity-period or complete-query statement.

## Trust and compatibility

- The prototype creates fresh synthetic issuer keys and a local, circuit-specific
  LegoGroth16 setup. A deployed verifier would need authenticated issuer keys and
  an appropriately generated, pinned setup; this executable does not implement a
  multiparty setup ceremony or supply production setup parameters.
- Issuers attest the tuple's normalized schema, subject and integer value.
  Equality across issuers presumes the same subject-identifier namespace and
  amount representation. No legacy Ed25519 credential is silently re-signed or
  treated as having been verified inside a proof.
- BBS+ messages and committed LegoGroth16 witnesses use the **same BLS12-381 scalar
  field** here. Dock's existing composite protocol supplies witness equality.
  Sparq's Noir/Barretenberg lane uses BN254. A shared nonce, byte label or identical
  public result is not a cross-field same-witness proof. The backend selector
  rejects Noir linkage because no such bridge is implemented in this experiment.
- This demonstrates a native-plus-general-circuit candidate using **Circom and
  LegoGroth16**, not a migration away from Noir. It does not implement W3C
  `bbs-2023`; Dock's BBS+ message interface is not that cryptosuite's wire format.
- The Cargo lockfile pins the resolved dependency graph. Dock's generic circuit
  path brings Wasmer and its compiler; that build/runtime cost is material even
  though the engine's default dependency graph remains unchanged.

## Checks

The real proof run accepts a two-issuer composition and the exact arithmetic
boundary. It rejects changed threshold/disclosure/context/challenge, wrong issuer,
swapped signature, two authenticated but different subjects, spliced income or
rent, a negative residual, an out-of-domain signed amount, truncated/trailing
proof bytes, and unsupported backend selection. It also constructs an otherwise
valid composite proof **without witness-equality links under the same nonce**,
verifies it against its weaker statement as an attack control, and checks that
the independently rebuilt verifier statement rejects it. Swapping the residual
component between two valid presentations under the same request also fails. These
are regression checks of this implementation, not a cryptographic security proof.

## Primary implementation references

- [Dock composite proof system](https://github.com/docknetwork/crypto/tree/main/proof_system),
  especially its BBS+ statements, `EqualWitnesses`, and the
  [multiple private-input circuit example](https://github.com/docknetwork/crypto/blob/main/proof_system/tests/r1cs/single_circuit_in_a_proof.rs).
- [Dock LegoGroth16](https://github.com/docknetwork/crypto/tree/main/legogroth16),
  whose Circom adapter commits the requested leading private inputs.
- [Christoph Braun's RDF query prototype](https://github.com/uvdsl/rdf-zkp-sparql),
  the native selective-disclosure/equality/range-query predecessor. The experiment
  here exercises an additional cross-credential arithmetic relation; it makes
  no claim that the composition mechanism itself is novel.
