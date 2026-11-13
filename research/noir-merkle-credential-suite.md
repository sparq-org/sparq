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
`MS26MESG`, `MS26TYPE`, `MS26LANG` and `MS26SUIT`, interpreted as big-endian ASCII
integers. They distinguish term bytes, quads, leaf salts, leaves, nodes, messages,
datatype identifiers, language identifiers and suite identifiers. The field term
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
digest. The field salt is sampled directly, rather than accepting multiple byte
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

## Signature modes and public values

Hidden mode checks Schnorr against public issuer-key coordinates and the agreed
public configuration digest, with no public output. Revealed mode publishes the
salted signed message and the agreed public configuration digest. All term bytes,
quad hashes, counts, roots, salts and signature witnesses remain private. This
library exposes neither holder binding nor status/validity policy; those belong
to the envelope lane. Entry points use raw openings, never validated cell structs
as unconstrained ABI inputs.

## Frozen evaluator boundary

The requested source move cannot preserve the authenticated guest's source graph
and frozen lockfiles. The detached model therefore **still holds a second copy of
the SHA-256 code**. Its workspace, guest crates, lockfiles and evidence files remain
untouched. A native source-inclusion test compares the old module with the new
suite on the existing vectors. The new crate is the source for new callers; its
expanded key dispatch does not change the frozen model's behavior.
