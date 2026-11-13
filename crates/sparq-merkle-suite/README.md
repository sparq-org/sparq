# sparq-merkle-suite

Research-grade, externally unaudited RDF Merkle credential suites. Internal,
`publish = false`; consuming this crate is the opt-in.

## 🚀 Quickstart

Use `sha256::{issue, verify}` for the Ed25519/SHA-256 member, or
`poseidon::{issue, verify}` for the Schnorr/Poseidon2 member. The latter takes the
existing `sparq_zk::sig::SecretKey` and a fresh private field salt. Its canonical
proof configuration must name `schnorr-poseidon2-merkle-2026` with
`https://w3id.org/security#cryptosuite`.

Run `cargo test -p sparq-merkle-suite` for native compatibility and constant pins.
The toolchain workflow also executes the Noir library and differential witnesses.

## ✨ Features

The shared `comparison_key` validates built-in integer subtypes, preserves the
existing key payloads, and adds timezone-free `xsd:date`. Valid timezoned dates
retain the unrepresentable class because a calendar-day ordinal cannot preserve timezone offsets and the
engine's partial order when timezone presence differs. Unrepresentable and no-key classes
never establish an order. Equal string prefixes also establish no order.

The Poseidon2 member uses length-bound byte packing, strict private quad-hash
ordering, index-derived leaf salts, minimal binary trees with zero padding, and
suite/count/configuration-bound signed messages with an explicit proof-metadata frame. It reuses the existing Poseidon2
and Baby Jubjub implementations rather than maintaining another primitive body.

The frozen zkVM model retains its own SHA implementation. **Every new caller
uses this crate**; the model file is included directly only by the test oracle.
The SHA member preserves the frozen datatype dispatch. The field-native member
owns the extended date and integer-subtype profile.

`tests/sha_parity.rs` checks the actual model source against this crate over fixed
vectors and generated data, including issuance and verification. It runs in the
required `ci-fast` job. Consolidation is tracked by
[#7391](https://github.com/sparq-org/sparq/issues/7391) at the next guest re-freeze.
The model, guests, detached lockfiles and evidence remain untouched.

Proof creation and the verification-method IRI are lifted into the signed metadata
frame together with the actual verification key's Multikey literal. Native
`verify_and_open` verifies before exposing those fields. Noir's authenticated
openers return opaque cells, the derived root and exact timestamp information;
status and validity policy remain the envelope lane's responsibility. The private
Multikey codec is documented in the usage skill and design record.

## 📚 Learn more

See [suite usage](../../skills/merkle-credentials/SKILL.md),
[design decisions](../../research/noir-merkle-credential-suite.md), and the
[Noir library](../../zk/noir-query/core/README.md). Canonicalization and signature
primitives currently enter through the existing capability crates; this is an
internal suite crate, with no dependency edge from the core engine.

## License

MIT, as in the workspace.
