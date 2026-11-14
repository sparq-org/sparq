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

For reproducible runs, call `poseidon::keypair_from_seed(&key_seed)` with a private
byte-array seed, then `poseidon::issue_deterministic(document, proof_config,
&key, &salt_seed)`. The deterministic entry point is safe only if a salt seed is
never reused across different content. For example:

```rust
use sparq_merkle_suite::poseidon;
let key = poseidon::keypair_from_seed(&[7; 32]);
let document = "<urn:subject> <urn:claim> \"example\" .\n";
let config = "<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n";
let credential = poseidon::issue_deterministic(document, config, &key, &[9; 32]).unwrap();
assert!(poseidon::verify(&credential, &key.public_key()));
```

Key derivation uses SHA-512 over `sparq-merkle-suite/keypair/v1\0` and the full
seed, reduces into the existing Baby Jubjub scalar field and maps zero to one.
Tree-salt derivation hashes `sparq-merkle-suite/tree-salt/v1\0`, the salt seed,
canonical signer coordinates, then the canonical document and configuration,
each prefixed by its unsigned big-endian byte length. The digest reduces into
the existing Noir field. Per-leaf salts and the signer nonce retain their existing
derivations; `sig::sign_deterministic` derives the nonce from the key and message.

The default-on `os-random` feature exposes `random::issue` to acquire fresh
platform entropy outside the deterministic core. Native callers use OS entropy;
WebAssembly uses its JavaScript backend. Disable the feature for the pure seeded
surface with `cargo build -p sparq-merkle-suite --target wasm32-unknown-unknown
--no-default-features`. Backend-selection dependencies support the existing RDF
and curve crates without changing their implementations. The seeded functions
perform no entropy acquisition, clock reads or file access.

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
RDF term identity is key-independent: use `TermCell::identity()` in Noir and
`poseidon::term_identity(&term)` in Rust. Scan, join, DISTINCT, sameTerm,
parameters and released terms use identity; blank nodes also compare credential
scope. `enc` stays for commitments; value consumers keep full key checks.

## Noir constructors and openings

Import `sparq_noir_query_core` from `zk/noir-query/core`. Its `term::open_term`
takes a kind tag, bounded lexical/datatype/language bytes, comparison-key bytes
and an explicit blank-node scope. It computes every cell component. Never accept a
`TermCell` itself as an entry-point witness: call the constructor on the raw
opening first. Access components through the cell's methods. `term::padding()`
is the only canonical unbound cell; the bound default graph is distinct.

`open_lexical` binds bounded bytes and metadata to both the keyed `enc` and
key-independent `identity()`. Identity hashes the kind and the existing complete
term frame, which includes lexical, datatype and language bytes for literals.
The default graph reuses its fixed encoding; padding has zero identity. Synthetic
credential graph names reuse their dedicated key-free encoding.
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

For a fixed-datatype dispatch, call
`term::open_literal_for_key_check(raw, scope)` once before the scalar branches.
It returns `OpenedLiteral<L>` with a private cell and its exact lexical vector.
`term::check_literal_key_for(opened, expected_dt)` returns `KeyCheckedCell`
after the same private datatype/key recomputation relation. Pass a fixed pinned
identifier in each arm; keep the string branch at its profile-sized bound.
`OpenedLiteral::cell()` exposes the constructed cell without claiming a key
check. The object is ABI-sealed, has no lexical setter, and must be constructed
in constrained code. `open_key_checked` delegates to this pair.

`suite::credential_root` receives validated quad cells, leaf-salt witnesses, the
private tree salt and count. It checks RDF positions, strict ordering, padding,
leaf-salt derivation and the minimal tree. Credential scope must be fixed by the
caller from the credential's dataset position. A blank-node join compares both
`identity()` and credential scope.

`suite::{open_credential_hidden, open_credential_revealed}` accept raw quads,
leaf-salt witnesses, salt/count/configuration, raw creation and verification-method
openings, the issuer key and a caller-fixed credential index. Hidden mode also
accepts signature/reduction witnesses and checks Schnorr. Both return an opaque
`OpenedCredential`; revealed mode requires external signature verification over
its exact `message()` using the same public key. Never witness that opaque object
or a `TermCell` directly.

`suite::{open_credential_hidden_from_quads, open_credential_revealed_from_quads}`
authenticate an internal `[[TermCell; 4]; N]` produced by constrained constructors.
Their remaining arguments follow the existing credential entry points. They share
the full tree/count/order/padding/salt and metadata relation, check blank-node scope
against the fixed dataset position, and preserve each signature mode's contract.
Opaque cells have a private empty-storage marker that prohibits ABI witnessing;
do not produce them through unconstrained hints. This seam avoids reopening quad term
bytes after construction and key checking for a profile.

For caller-selected backing bounds, use
`suite::{open_bounded_credential_hidden, open_bounded_credential_revealed}`.
Their quad cells are `BoundedTermOpening<L, D, G>`; creation and method openings
are `BoundedTermOpening<C, D, G>` and `BoundedTermOpening<V, D, G>`.
All other arguments and authenticated getters follow the existing entry points.
Runtime lengths and every padding byte remain constrained. Choose capacities
from the credential profile and emit separate credential calls directly in the
generated program. The complete opener specializes subject, predicate and graph
construction to their RDF roles; general cell/table constructors remain available
with their existing signatures and behavior.

When a few quads need a longer lexical bound, call
`suite::{open_bounded_credential_mixed_hidden, open_bounded_credential_mixed_revealed}`.
They take `S` short rows at `BoundedTermOpening<L, D, G>`, `K` long rows at
`BoundedTermOpening<LL, D, G>` and a private `long_index: [u32; K]` with `S + K = N`.
Long indices strictly increase below `N`; an unused long slot is a padding row at
the trailing indices. Short rows fill the remaining tree positions in order, so the
leaves, root and returned `OpenedCredential<N>` equal the uniform opening. A quad
too long for the short bound needs a long slot; otherwise no proof exists.

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

Resource-only openings may use empty datatype and language backing capacities
when assembled from ABI-safe raw witnesses inside constrained code. Empty
datatype capacity excludes literals; the constructor still checks metadata and
canonical backing padding. Term identifiers and the byte encoding remain pinned.
