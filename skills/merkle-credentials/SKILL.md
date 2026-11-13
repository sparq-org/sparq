---
name: merkle-credentials
description: Issue, verify and open research-grade RDF Merkle-root credentials with the internal sparq-merkle-suite crate and zk/noir-query/core, including the revised Schnorr/Poseidon2 member, private salts and lexical comparison-key checks.
---

# Merkle credential suites

This surface is research grade and externally unaudited. It authenticates a
credential's RDF terms; it does not implement a query proof method, holder
binding, status or validity policy. Depend on `sparq-merkle-suite` explicitly;
it is unpublished and does not enter the engine's default dependency graph.

## Native issuance and verification

`poseidon::issue(document, proof_config, &secret_key, salt)` canonicalizes N-Quads
with RDFC-1.0, computes comparison keys, orders quads by their unsigned field
hashes, builds the salted tree, and signs the member's message. `salt` is a fresh
uniform BN254 field element, shared only with the holder. Reuse
`sparq_zk::sig::SecretKey` for the issuer key. The configuration must contain
exactly one `https://w3id.org/security#cryptosuite` statement whose object is the
literal `schnorr-poseidon2-merkle-2026`.

`poseidon::verify(&credential, &public_key)` rechecks canonicalization, keys,
strict ordering, count, tree, configuration and signature. Treat the entire
`Credential` as private prover material. Use `quad_cells` to lay out subject,
predicate, object and graph cells. Triple terms are refused.

`sha256::{encode_term, leaf, root, signed_message, issue, verify}` provides the
Ed25519/SHA-256 member. Its configuration must name `eddsa-sha256-merkle-2026`
under the same predicate. Its `SignedCredential` carries the signature followed by
the private byte salt. The detached evaluator still holds its frozen second
copy; existing vectors are parity-checked, while its key dispatch remains frozen.

## Comparison keys

`comparison_key(&literal)` returns the class byte followed by its payload.
Classes for decimals, IEEE values, zoned/local date-times, booleans and strings
retain their existing payloads. Built-in integer-derived datatypes are validated
against their value spaces and use the decimal class. XML Schema defines no
additional built-in decimal-derived non-integer datatypes; unknown custom
subtypes cannot be inferred from an IRI alone and get no key.

Timezone-free `xsd:date` uses the calendar-day class. Valid timezoned dates get
`UNREPRESENTABLE`: the day ordinal cannot retain timezone offsets, and the
engine's mixed-timezone comparison is a partial order. This member adds no
separate zoned-date key class. Invalid literals get `NONE`. Neither class
can establish an order. Prefix keys establish an order only when prefixes differ.
Term identity always uses the full encoding; blank-node identity also uses its
credential scope.

## Noir constructors and openings

Import `sparq_noir_query_core` from `zk/noir-query/core`. Its `term::open_term`
takes a kind tag, bounded lexical/datatype/language bytes, comparison-key bytes
and a credential index. It computes every cell component. Never accept a
`TermCell` itself as an entry-point witness: call the constructor on the raw
opening first. Access components through the cell's methods. `term::padding()`
is the only canonical unbound cell; the bound default graph is distinct.

`open_lexical` binds bounded bytes and metadata to a cell's `enc`.
`recompute_key` performs that opening and checks both class and payload; only it
produces `KeyCheckedCell`. The datatype-specialized `recompute_key_for` also
binds the opened datatype to its expected identifier, so fixed query shapes
can omit unrelated parsers. Query gadgets must require this type before reading
value semantics. Lexical capacities are compiled bounds, not truncation rules.
Use `BoundedVec<u8, L>` and canonical zero backing storage after its length.

`suite::credential_root` receives validated quad cells, leaf-salt witnesses, the
private tree salt and count. It checks RDF positions, strict ordering, padding,
leaf-salt derivation and the minimal tree. Credential scope must be fixed by the
caller from the credential's dataset position. A blank-node join compares both
`enc` and credential scope.

`suite::open_credential_hidden` additionally checks the existing Baby Jubjub
Schnorr relation against the public issuer key. It returns nothing.
`suite::open_credential_revealed` returns only the salted signed message.
Both modes take the verifier-agreed configuration digest as two public unsigned
halves. Salt, count, root, raw quads, keys, leaf salts and signature material
remain private. A generated entry point must preserve this ABI; library visibility
alone cannot decide an entry point's public inputs.

## Validation

Run `cargo test -p sparq-merkle-suite`, `nargo test` in the Noir core, and the
live suite test from the ZK toolchain workflow.
`cargo run -p sparq-merkle-suite --example differential_vectors` emits the native
corpus; `--example noir_differential_fixtures` emits the exact wiring and raw
witnesses used by the live differential test. Each pair is checked in both
orientations with a checked literal per execution. `--example credential_abi_fixtures`
emits both-mode witness material with independently varied salts and configuration
digest halves. The live tests use the existing
fail-closed gate and execute witnesses without proving. Gate measurements are
external evidence, never constants or acceptance thresholds in this library.
