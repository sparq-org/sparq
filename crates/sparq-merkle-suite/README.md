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

For reproducible issuers use the seeded entry points:

```rust
use sparq_merkle_suite::poseidon;
let key = poseidon::keypair_from_seed(&[7; 32]);
let document = "<urn:subject> <urn:claim> \"example\" .\n";
let config = "<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";
let credential = poseidon::issue_deterministic(document, config, &key, &[9; 32]).unwrap();
assert!(poseidon::verify(&credential, &key.public_key()));
```

The deterministic entry point is safe only if a salt seed is never reused across
different content. Keep key and salt seeds private. Key derivation hashes the
keypair label and the full seed with SHA-512, reduces into the existing scalar
field, and maps zero to one. Salt derivation uses its own label, seed, signer
coordinates and length-framed canonical document/configuration bytes. Existing
`sig::sign_deterministic` derives its nonce from the secret key and signed message.

The default-on `os-random` feature exposes `random::issue(document, config, &key)`
for fresh platform entropy, with uniform field rejection sampling. It is separate
from the pure seeded core. Build that core for the WebAssembly package with:

```sh
cargo build -p sparq-merkle-suite --target wasm32-unknown-unknown --no-default-features
```

Target-specific backend features select JavaScript entropy support required by
the existing RDF/curve dependencies. The seeded functions never call those
backends, a clock or file APIs. No dependency primitive is copied or changed.

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
