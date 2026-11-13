# Revised field-native Merkle credential member

The Schnorr/Poseidon2 Merkle-root member is implemented as a research-grade,
externally unaudited native suite and Noir library. It is separate from the older
commitment methods and from the query compiler. The design's opening-cost decision
was checked with the pinned compiler and backend before fixing suite constants;
the evidence belongs to the external lane report.

## Byte hash and domains

The previous draft truncated a SHA digest for each term. The implemented member
instead applies the existing Poseidon2 sponge to a domain field, byte length, and
unsigned big-endian byte chunks. Every full chunk uses the maximum byte width
that fits strictly below the field modulus; the final chunk retains its actual
width. The byte length binds leading and trailing zero bytes and makes capacities
interoperable. Rust's `PACK_BYTES` and the exhaustive Noir constant pin define the
layout. No second permutation or signature implementation is introduced.

The domain labels are `MS26TERM`, `MS26QUAD`, `MS26SALT`, `MS26LEAF`, `MS26NODE`,
`MS26MESG`, `MS26TYPE`, `MS26LANG`, `MS26SUIT`, `MS26META` and `MS26GRPH`, interpreted as big-endian ASCII
integers. They distinguish term bytes, quads, leaf salts, leaves, nodes, messages,
datatype identifiers, language identifiers, suite identifiers, proof metadata and synthetic credential graph names. The field term
encoding still folds the kind tag, term-byte hash and complete comparison key.

## Ordering, salting and messages

The earlier draft sorted the leaves themselves. The revised member orders the
private unsalted quad hashes strictly, then derives each leaf's salt from the
private tree salt and its index. Leaf hashes bind both the quad and that derived
salt. This preserves duplicate detection while keeping tree nodes salted. Padding
leaves are zero and the tree uses the least power-of-two capacity that contains
all real quads. A larger circuit bucket computes the same root. Count and padding
are checked independently.

The signed message binds its dedicated domain, suite identifier, canonical private
field salt, exact count, salted root and both halves of the canonical configuration
digest and a proof-metadata digest. The field salt is sampled directly, rather than accepting multiple byte
representations that reduce to the same field value. The holder keeps it private.

## Comparison keys and cells

Existing key classes and payloads are retained. Built-in integer-derived datatypes
use the decimal class after subtype validation. XML Schema has no additional
built-in decimal-derived non-integer types; custom subtype declarations are outside
the suite and do not change an unknown datatype's class.

Timezone-free dates gain a calendar-day class. Valid timezoned dates remain
unrepresentable: the calendar-day ordinal cannot retain timezone offsets or the
engine's partial order when timezone presence differs. This member adds no
separate zoned-date key class. Invalid lexical forms have no key. Prefix
string keys order only differing prefixes; unrepresentable and no-key classes
never decide an order.

Every cell is built from a full term opening, including kind, lexical bytes,
datatype and language bytes, key bytes and a caller-fixed credential position.
There are no unconstrained metadata slots. Lexical bytes can be opened again when
a value gadget needs them. The key-checking constructor recomputes both class and
payload and is the sole producer of the checked-cell type. Float lexical parsing
uses exact integer numerator/denominator arithmetic and round-to-nearest-even,
including the widening of single-precision values and negative-zero normalization.
The differential harness checks each literal separately in both pair orientations,
keeping the parser instance bounded while asserting native key equality and
engine comparison order. Blank nodes carry their credential position separately from the issuer's encoding;
identity comparisons must include it.

## Authenticated envelope metadata

The earlier implementation bound the configuration digest but exposed no usable
creation opening. The revised signed-message layout appends
`P(metadata_domain, created_enc, multikey_enc, method_enc)`. `created_enc` is the
full typed creation term, or canonical zero for absence; `method_enc` is the
verification-method IRI, or canonical zero for absence. The full canonical
configuration digest is still bound. Native issuance and verification extract
these singleton options from the proof node itself. This explicit message
amendment authenticates the opened creation claim directly; an epoch witness is
never accepted on the strength of a supplied configuration hash.

The verification key is encoded as lower-case base16 Multibase: `f` followed by
the hex encoding of unsigned-varint private-use multicodec `0x300001`, then the
canonical unsigned big-endian `x` and `y` coordinates, each at the field's byte
width. The varint prefix is `81 80 c0 01`; coordinates are uncompressed. This is
an internal profile, not a registered interoperable codec. The
[Multikey data model](https://www.w3.org/TR/cid-1.0/#multikey) permits other
Multibase encodings, and the
[multicodec private-use area](https://github.com/multiformats/multicodec#private-use-area)
is reserved for application profiles. The existing Baby Jubjub implementation
checks the point. The literal term, its complete key and its encoded identity are
derived from those same coordinates.

Call `suite::open_credential_hidden` or `suite::open_credential_revealed` with raw
quad/creation/method openings. The implementation in `suite_metadata` reconstructs
every term, enforces order/count/padding, derives the root and metadata frame,
and returns an opaque `OpenedCredential`. Hidden mode additionally verifies
Schnorr before returning. Revealed mode's exact `message()` must be verified
externally with the same public issuer key. Low-level folds do not produce this
opaque object and the partial signature helper is crate-private.

Getters expose the authenticated table, exact count, derived cryptographic root,
signed message, creation cell, creation instant, verification method and Multikey
cell. `credential_subject()` finds the unique default-graph credential subject;
`valid_from()`, `valid_until()` and complete singleton `property()` reads return
only objects from that same table. Absence is canonical padding. `open_time`
returns exact UTC milliseconds for a checked representable zoned date-time;
absent, local, ill-typed or unrepresentable time is explicitly unknown. It never
rounds. Lane E owns inclusive validity policy and must refuse required checks on
unknown creation when both explicit bounds are absent.

`open_status_purpose` opens an actual string literal and validates its purpose.
`open_status_index` opens an actual string literal and parses its unsigned decimal
characters to an integer, rejecting signs, empty input and overflow. These replace
the synthetic purpose/index cells, not the status policy. Use the literal
vocabulary identifiers and term constructors instead of the synthetic IRI and
number encodings. The index opener returns `u64`; narrowing it requires an
explicit range check in the caller. The envelope's old whole-second offset fixtures must consume
`Timestamp`'s exact millisecond representation instead.

`term::credential_graph(request_digest, credential_index)` creates a synthetic
blank graph name under its own domain, binding both request-digest halves and the
fixed dataset position. It is distinct from issuer blank-node labels and carries
the credential scope. `cell.credential()` is `credential_index + 1` for blank
nodes and zero for other kinds; the envelope adapter uses it as its scope.
The caller fixes that position in generated wiring; it is
not another entry-point witness. This replaces the envelope's synthetic graph
name. The native `metadata::graph_name` returns the same cell.

The library authenticates these data and exposes them for the envelope. It does
not implement holder binding, snapshot trust, status policy or validity policy,
and it is not a complete Data Integrity processor.

## Signature modes and public values

Both modes expose the issuer-key coordinates and verifier-agreed configuration
digest halves. Hidden mode has no public return. Revealed mode publishes only the
salted signed message and checks its signature externally using that same public
key. All term bytes, creation/method openings, keys, quad hashes, counts, roots,
salts and signature witnesses remain private. Getters are private proof state,
not additional public outputs. Entry points accept raw openings rather than opaque
validated objects as witnesses. Compiled ABI tests enforce this contract.

## Compiler agreement and bounded-array construction

Fixed datatype, language, vocabulary and suite identifiers are field literals in
the shared constants module, pinned against Rust. The lookup helpers return those
literals instead of hashing constant identifiers through different compiler
drivers. Unknown witnessed identifiers use the ordinary byte sponge. The compiler
agreement evidence compares decompressed ACIR from the pinned native and browser
drivers; it is retained externally.

The byte encoder streams directly into field words and stores outside the
per-byte predicate. It no longer repeatedly appends to a variable-index byte
array. Credential wiring similarly selects a completed quad before storing it in
the output table, avoiding conditional selection of the entire table. The tree
visits each reduction's actual width. These changes preserve the encoding and
minimal-tree semantics; measured circuit sizes remain external evidence.

## Frozen evaluator boundary

The orchestrator explicitly retains the frozen source boundary until the next
guest re-freeze. The detached model therefore **still holds a second copy of
the SHA-256 code**. Its workspace, guest crates, lockfiles and evidence files remain
untouched. A native source-inclusion test compares the old module with the new
suite on fixed vectors and generated inputs, including issuance and verification.
It runs in the required `ci-fast` job. Every new caller uses the new crate; only
the test oracle includes the model file directly. The SHA member preserves its
frozen key dispatch, while the field-native member supplies the extended profile.
The authorized consolidation is [#7391](https://github.com/sparq-org/sparq/issues/7391).
