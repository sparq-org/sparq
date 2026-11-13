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
`Credential` as private prover material. Use `credential_cells(quad, index)` for dataset cells; it derives the nonzero
blank-node scope from the fixed position. `quad_cells` instead accepts an explicit
scope for low-level encoding. Both lay out subject,
predicate, object and graph cells. Triple terms are refused.

`sha256::{encode_term, leaf, root, signed_message, issue, verify}` provides the
Ed25519/SHA-256 member. Its configuration must name `eddsa-sha256-merkle-2026`
under the same predicate. Its frozen datatype dispatch is separate from the field-native extension. Its `SignedCredential` carries the signature followed by
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
and an explicit blank-node scope. It computes every cell component. Never accept a
`TermCell` itself as an entry-point witness: call the constructor on the raw
opening first. Access components through the cell's methods. `term::padding()`
is the only canonical unbound cell; the bound default graph is distinct.

`open_lexical` binds bounded bytes and metadata to a cell's `enc`.
`recompute_key` performs that opening and checks both class and payload. The
datatype-specialized `recompute_key_for` also
binds the opened datatype to its expected identifier, so fixed query shapes
can omit unrelated parsers. Query gadgets must require this type before reading
value semantics. Lexical capacities are compiled bounds, not truncation rules.
Use `BoundedVec<u8, L>` and canonical zero backing storage after its length.

`suite_entry::BoundedTermOpening<L, D, G>` lets generated wiring choose lexical,
datatype and language backing capacities independently. Its runtime lengths stay
witnesses. `open_bounded_cell(raw, scope)` and
`open_bounded_quads(raw_quads, scope)` delegate to the existing term constructor
and check every absent-field and backing-padding byte. The existing
`TermOpening<L>`, `open_cell` and `open_quads` remain compatible.

`term::open_key_checked(raw, scope, expected_dt)` constructs a literal directly
from a bounded raw opening and returns `KeyCheckedCell`. It shares the initial
lexical hash, constrains the opened datatype to `expected_dt`, and recomputes the
entire key before returning. Pass a fixed pinned datatype identifier, such as
`suite_constants::DT_INTEGER`, so unrelated parsers fold away. This constructor
and the recomputation functions produce checked cells only after full lexical
key recomputation. Absent and nonliteral openings refuse.

`suite::credential_root` receives validated quad cells, leaf-salt witnesses, the
private tree salt and count. It checks RDF positions, strict ordering, padding,
leaf-salt derivation and the minimal tree. Credential scope must be fixed by the
caller from the credential's dataset position. A blank-node join compares both
`enc` and credential scope.

`suite::{open_credential_hidden, open_credential_revealed}` accept raw quads,
leaf-salt witnesses, salt/count/configuration, raw creation and verification-method
openings, the issuer key and a caller-fixed credential index. Hidden mode also
accepts signature/reduction witnesses and checks Schnorr. Both return an opaque
`OpenedCredential`; revealed mode requires external signature verification over
its exact `message()` using the same public key. Never witness that opaque object
or a `TermCell` directly.

Both fixture entry points expose the issuer-key coordinates and verifier-agreed
configuration digest halves. Hidden mode returns nothing; revealed mode releases
only the salted signed message. Raw metadata, terms, keys, count, root, salts and
signature witnesses stay private. `OpenedCredential` is internal proof state.

The signed message includes a dedicated metadata digest over the creation term,
the verification key's Multikey literal and the verification-method IRI. Those
fields are reconstructed before hashing; the full canonical configuration digest
also remains bound. `poseidon::verify_and_open` checks the signature before
returning native metadata. `metadata::from_configuration` alone is an extraction
helper, not a verification result.

The Multikey profile uses lower-case base16 Multibase and a private-use multicodec
followed by canonical big-endian coordinates. It is not a registered interoperable
key type. `metadata::{multikey, open_multikey}` and Noir
`suite_metadata::{multikey_cell, open_multikey}` use that same codec, pinned with
its byte prefix. Both reject invalid verification points through the existing
curve implementation. See the design record for the exact layout.

Use `created_at()` / `created()` for the authenticated proof creation and its exact
UTC millisecond value. Unknown/absent or unrepresentable time yields `known() ==
false`; do not substitute zero for an actual time. `credential_subject()` finds the
unique default-graph credential root, or absence. `valid_from()` / `valid_until()`
and `property(subject, predicate)` are complete singleton reads of the verified
table; duplicate values refuse. `open_time` checks the lexical opening and key
before returning a usable instant. Inclusive-start and fallback policy belong to
the envelope, which must refuse required checks on unknown creation.

`open_status_index` checks a string literal and parses only unsigned decimal
characters into `u64`; `open_status_purpose` checks the exact string
literal value. Obtain both cells from the authenticated table before opening.
`term::credential_graph(request_digest, index)` derives the synthetic graph name
under its own domain. Its blank-node scope is the fixed dataset position, as for
credential-local blank nodes, while its encoding is separate from issuer labels.
Use the literal vocabulary getters for the common credential predicates.
The complete opener derives `cell.credential()` as `credential_index + 1` for
blank nodes and zero for other kinds; use it as the envelope adapter's scope.

The frozen evaluator remains a second SHA copy. Every new caller uses the new
crate. Required-CI source inclusion checks fixed/generated encodings, keys,
leaves, trees, messages, issuance and verification. The authorized consolidation
is [#7391](https://github.com/sparq-org/sparq/issues/7391).

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
