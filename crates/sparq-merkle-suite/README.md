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
suite/count/configuration-bound signed messages. It reuses the existing Poseidon2
and Baby Jubjub implementations rather than maintaining another primitive body.

The detached `zk/sparql-evaluator` model **still holds a second copy of the SHA-256
code**. Its guest source graph and lockfiles are frozen. The test-only source
inclusion in `tests/suites.rs` compares the existing vectors without modifying that
workspace. New callers use this crate. Its expanded datatype dispatch does not
retroactively change the model's comparison keys.

## 📚 Learn more

See [suite usage](../../skills/merkle-credentials/SKILL.md),
[design decisions](../../research/noir-merkle-credential-suite.md), and the
[Noir library](../../zk/noir-query/core/README.md). Canonicalization and signature
primitives currently enter through the existing capability crates; this is an
internal suite crate, with no dependency edge from the core engine.

## License

MIT, as in the workspace.
