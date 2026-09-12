# Canonical signed-integer result contract

[GPT-6] With `sparq-zk-compose/successful-results`, the separate
`result::signed` API prepares and verifies released distinct answers using the
canonical signed `i64` profile. It inherits the
[successful-result contract](successful-results.md): positive BGPs, joins,
bounded result support, independently trusted issuers/status snapshots, and a
fresh relying-party nonce. It does not assert complete answers or absence.
This remains research-stage and has not received an external cryptographic audit.

Use `prepare_signed_result` or `prepare_signed_result_with_options`, then
`PreparedSignedResult::prove`. The relying party calls `verify_signed_result`
with its own exact query, `ResultPolicy`, nonce and durable `SeenNonces` store.
`SignedResultOptions` selects credential capacity, witness search and search fuel.
The existing unsigned entry points and wire capacity enum retain their semantics.

`SignedResultPresentation` is a separate version-three type. It carries the exact
query, released RDF mappings, challenge, issuer slots and proof bytes. Unknown
fields reject, including an unsigned `integer_capacity` selector. The verifier
derives the circuit from that version, credential slots, hidden predicate count
and the complete accepted status policy; the prover supplies no key or circuit
identifier. Both private predicates and predicate-free signed results have their
own version-three circuit identity.

Comparison operands and bounds admit only canonical `xsd:integer` spellings in
the `i64` range: `0`,
positive digits without a leading zero, and a single minus before a nonzero
magnitude. Leading plus, leading zeros, negative zero, whitespace, alternate
numeric datatypes and out-of-range values reject. The committed RDF term is never
normalized. This deliberately excludes other valid XML Schema lexical forms;
it is not general SPARQL numeric support.

The private witness encodes signed order using the mathematical bias `value +
2^63`. Every possible `u64` bias denotes exactly one admitted signed value. The
Noir gadget derives the sign and magnitude with constrained, in-range branch
arithmetic, reconstructs the canonical lexical token and its datatype suffix,
and checks the original committed literal encoding before comparing biases.
The minimum value does not require signed negation. One fixed capacity covers
both signs and every admitted lexical length; sign and actual length are not
public selectors. Public bounds, released terms and the result itself can reveal
information about a private value, as in the unsigned contract.

Credential capacity remains one or two, with the existing explicit two-slot
padding option. Status capacity is independently chosen from depths 10/17/20
using the complete policy; no snapshot is truncated. Other BGP/result/filter
capacity limits are the same as the unsigned result profile.

Native preparation, wire rejection and core gadget tests are distinct from
genuine proofs. The toolchain test executes every signed capacity wrapper and
private boundaries, then checks altered biases against the literal-binding
constraint. A separate test generates signed proofs for the minimum, maximum,
a positive value and a public-predicate result, and an unsigned proof for
cross-contract substitution controls. Both verifiers admit the same nonnegative
query before rejecting the relabeled foreign proof. Replay and changed-policy
controls remain part of that test.

Measured circuit sizes and exact Noir source identity are recorded in
[`result_signed_gates.json`](../../../bench/zk-compose/result_signed_gates.json).
The default host evidence checker binds its source/toolchain and complete member
inventory to the compatibility record and gate snapshot, with corruption controls.
These are local constraint measurements, not canonical runtime benchmarks.
Unsigned proof evidence is not used as evidence for the signed relation.
