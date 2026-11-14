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
Their private empty-storage marker makes opaque cells invalid as ABI witnesses.
Construct them in constrained code; do not obtain them from hints.

Scan, join, DISTINCT, sameTerm, parameters and released terms use
`TermCell::identity()`; blank nodes additionally compare credential scope.
`enc()` stays for commitments; value consumers keep full key checks. The identity
hash shares the complete lexical/datatype/language frame with the keyed encoding.
Padding has zero identity; the default graph and synthetic credential graph names
reuse their existing fixed or domain-separated key-free encoding.

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

To share literal framing across fixed-datatype branches, call
`term::open_literal_for_key_check(raw, scope) -> OpenedLiteral<L>` once, then
`term::check_literal_key_for(opened, expected_dt) -> KeyCheckedCell` in each
arm with a fixed pinned identifier. The opaque object privately binds the cell
to its exact lexical vector and rejects nonliterals; its `cell()` getter does
not establish a key check. It is ABI-sealed and has no unchecked constructor
or lexical setter. `open_key_checked` delegates to the same pair.

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

For tables already built by constrained term constructors, use
`suite::{open_credential_hidden_from_quads, open_credential_revealed_from_quads}`.
They take internal quad cells followed by the original salts, count, configuration,
raw metadata, key and dataset-position arguments. The shared authentication
relation checks the complete tree and every bound blank node's credential scope.
Hidden mode additionally checks the signature. Quad lexical bytes are not reopened.

For compact generated witnesses, call
`suite::{open_bounded_credential_hidden, open_bounded_credential_revealed}`.
They accept `BoundedTermOpening<L, D, G>` quad cells and
`BoundedTermOpening<C, D, G>` / `BoundedTermOpening<V, D, G>` creation and method
openings. Their remaining arguments and authenticated return value match the
existing entry points. All lengths remain witnesses; all backing padding is
checked. Choose capacities from the credential profile and emit independent
credential calls directly in generated wiring. The general cell/table
constructors retain their existing behavior; RDF-role specialization is private
to complete credential openings.

When a few quads need a longer lexical bound, call
`suite::{open_bounded_credential_mixed_hidden, open_bounded_credential_mixed_revealed}`.
They take `S` short rows at `BoundedTermOpening<L, D, G>`, `K` long rows at
`BoundedTermOpening<LL, D, G>` and a private `long_index: [u32; K]` with `S + K = N`.
Long indices strictly increase below `N`; an unused long slot is a padding row at
the trailing indices. Short rows fill the remaining tree positions in order, so the
leaves, root and returned `OpenedCredential<N>` equal the uniform opening. A quad
too long for the short bound needs a long slot; otherwise no proof exists.

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
the public active-length index. Canonical policy consumption uses the envelope module; graph
dispatch retains its own admission contract.

The `ops_core_profile` adapter holds raw witness structs and normalization only.
It constructs zero resource datatype, language and key fields and derives bound
flags and the default graph. `ops_core_suite::open_bounded_one` delegates the
complete signed opening once to `suite::open_bounded_credential_hidden`.
Value consumers use fixed datatype `term::open_key_checked` calls with separate
scalar bounds. Envelope wiring can share `authenticate_bounded`'s opaque suite
object and convert it through `from_authenticated_bounded` after policy checks.

Packing combines short checked chunks into the unchanged field-word encoding.
Its private split hint is checked by an exact quotient/remainder relation and
short ranges; padding is enforced by exact division with a bounded quotient.

Resource-only openings may use empty datatype and language backing capacities
when assembled from ABI-safe raw witnesses inside constrained code. Empty
datatype capacity excludes literals; the constructor still checks metadata and
canonical backing padding. Term identifiers and the byte encoding remain pinned.

The generated query wiring uses `ops_core_envelope::open_slot` and the shared
public-policy consumer. It accumulates holder coverage before checking signatures,
and uses the returned issuer with `append_verified_issuer`.
`authenticate_profile_bounded_mode` returns the same opaque suite opening for
later envelope checks. It calls the shared Schnorr verifier in hidden mode;
revealed mode binds the opening's message and suite signer encoding to public
inputs and requires the unused signature witness to be zero. Global mode flags
bind the revealed count and inactive-slot padding. Legacy hidden-only wrappers
retain their existing signatures. The public layout and external signature
admission live in the query crate; the native Compiler currently accepts hidden
mode. `ops_core_expr::temporal_payload` is the shared packed-time adapter for
operator consumers, retaining the exact zero-tail constraint.

## Canonical table outputs

Scan validates the supplied canonical bindings directly: every matching source
binding is present, every active output has a matching source, and bindings are
unique. Each metadata field is selected from its first matching source position.
`table::assert_canonicalized` checks stable canonical permutations for projection
and SELECT. Computed occurrence ordinals preserve repeated rows and their full
payload order. No witness-chosen permutation or source index is used; metadata
selection is exact in each field. Padding, cardinality, canonical order and
capacity remain constrained. The reference interpreter's order is unchanged.

The class-aware temporal scalar helper constrains the bounded word that
reconstructs its checked key and header. This also enforces the key's zero tail;
the older byte-based extraction entry point remains available.

FILTER uses `table::assert_selection` to verify a complete stable subsequence.
Computed prefix counts fix each accepted input row’s destination, and fieldwise
selection binds its complete payload. Inactive rows and SPARQL errors contribute
no rows; padding and overflow still refuse. No private row index is admitted.

A leading-column projection preserves the input’s canonical row order. It uses
`table::assert_ordered` to bind each row and its complete payload directly. Other
projections retain the stable canonical-permutation relation. Both paths check
cardinality, padding and capacity.

JOIN matches every compatible source pair to the first unused output occurrence
of its binding, retaining source-pair order for equal identities. The full payload
is bound at that occurrence. Complete pair coverage and complete output use
preserve bag multiplicity; an undersized output refuses. All occurrence choices
are computed inside the gadget.

Plain-string equality classifies its operands from the authenticated datatype
before value-key guards; identity-only comparison does not inspect unused keys.

The converted operator view exposes `rows()`, `active()`, `message()` and `graph()`.
Its private construction marker excludes it from entry-point witnesses and external
struct construction. Generated slots return that view after their envelope checks.

`ops_core::named_view(dataset)` retains the rows and graph cells and masks activity
to authenticated named facts. Generated GRAPH scans use that view; default and
correlated scans use the complete dataset, including the method's issuer assertions.

The raw status adapter carries each evidence row's actual private depth to
`envelope::check_status_bounded`. It uses the shared bounded path and snapshot
relation; the accepted-set depth remains a separate public policy value.
Scalar read sites share `open_literal_for_key_check` before fixed-datatype dispatch;
full-cell association and canonical unused/backing padding remain constrained.
