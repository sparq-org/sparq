# Noir query core

Research-grade, externally unaudited credential gadgets for the query-method
programme. This lane supplies `suite*` and `term*`; operators and expressions
belong to the other lanes.

Use nargo `1.0.0-beta.21` and bb `5.0.0-nightly.20260324`. Run `nargo test`
here. Nargo manifests use the repository convention `>=1.0.0`; this compiler
rejects prerelease version requirements. The live Rust test checks the exact beta
and compiler commit, and the toolchain workflow pins the binaries. Git dependencies
are pinned by tag. Hashing and
Schnorr verification reuse the existing compose core; older methods are unchanged.

See [suite usage](../../../skills/merkle-credentials/SKILL.md) for constructors,
key-checked cells and the public-value contract. Raw term openings are entry-point
witnesses; `TermCell` and `KeyCheckedCell` are internal validated values produced
by the constructors, not ABI witness types.

For caller-selected backing bounds use
`suite_entry::BoundedTermOpening<L, D, G>`, `open_bounded_cell(raw, scope)` and
`open_bounded_quads(raw_quads, scope)`. Runtime lengths remain witnesses and all
backing bytes after each length must be zero. Absent openings require every raw
field to be zero. The existing fixed-metadata opening API remains compatible.

`term::open_key_checked(raw, scope, expected_dt)` returns an opaque
`KeyCheckedCell` directly from a bounded raw literal opening. It shares the initial
lexical hash and checks the datatype and full recomputed key. Use a fixed pinned
datatype identifier to omit unrelated parsers. `recompute_key_for` remains
available for opening an existing cell. No unchecked checked-cell constructor is
exposed.

Hidden credential mode has public issuer-key coordinates and agreed configuration
digest halves, and no public output. Revealed mode has public issuer-key coordinates and agreed configuration
digest halves and outputs the salted signed message. Neither exposes the tree
salt, unsalted quad hashes or the root. Salt is a canonical private field element.

The detached evaluator's SHA-256 module remains a frozen second copy. The new
Rust suite owns the source for new callers and pins all Noir globals.

Call `suite::{open_credential_hidden, open_credential_revealed}` with raw quad
openings, raw creation/method openings, the public key and a caller-fixed dataset
position. The implementation in `suite_metadata` produces `OpenedCredential`;
its fields cannot be supplied as entry-point witnesses. Getters expose the
verified table, root, message, creation, verification method/key and complete
singleton property reads. `credential_subject`, `valid_from` and `valid_until`
read the authenticated table. `open_time` yields exact UTC milliseconds or an
explicit unknown value. The envelope refuses a required check on unknown time.

Use `open_status_index` for decimal string indices, `open_status_purpose` for
literal purposes, and `open_multikey` for the private codec. Synthetic credential
graph names come from `term::credential_graph` and include the request digest and
fixed dataset position under a dedicated domain. Ordinary blank-node identity
also includes the credential scope.

Fixed datatype/language/vocabulary identifiers are literal fields pinned against
Rust. Byte packing stores field words outside predicates; tree reductions visit
each level's actual width. Generated wiring should assign a completed quad to its
output table outside the count predicate, avoiding selection of the whole table.

The expression modules expose `(Lane<T>, is_error)`, with a distinct capacity
channel. Call `filter_accept` at a FILTER root. The term-opening contract binds
scalar values to authenticated lexical openings and recomputed keys.

The operator dataset carries triple rows, active flags and parallel graph names.
Suite conversion obtains presentation-local graph names from the shared term
constructor. The envelope integration can call `ops_core_suite::authenticate`,
check its returned opaque suite object, and pass it to `from_authenticated`.
`append_verified_issuer` consumes the envelope-authenticated issuer cell and adds
a default-only assertion. Constant-list evaluation uses `evaluate_indexed` with
the public active-length index. The canonical policy-root consumer and graph
dispatch remain separate integrations.
