# @sparq-org/zk-result

Successful-result proofs for Node, using sparq's method-bound commitment-signed
credential format. Research grade and **not externally audited**. These are not
standard VCs. There is no answer or wallet completeness, absence proof, or holder
identity guarantee. Issuer identities, capacities and result size are observable.

The package contains its host WebAssembly, circuit artifacts, canonical verification
keys, CRS and bundled npm dependencies. Installing its tarball requires no Rust,
Noir or native proving toolchain, and no network. There is no installation build.
The runtime explicitly selects the bb.js WASM backend; it never falls back to a
native executable.

```js
import { prove, verify } from '@sparq-org/zk-result';

const { presentation, result } = await prove({ query, credentials, policy, nonce });
const checked = await verify({
  expectedQuery: query, presentation, policy, nonce, seenNonces,
});
if (!checked.verified) throw new Error(checked.reason);
console.log(checked.released);
```

The relying party supplies the expected query, trusted issuer keys, authenticated
status snapshots and fresh nonce. `seenNonces` supplies synchronous `has(key)` and
`add(key)` methods; the caller owns persistence. A `Set` is sufficient only for
single-session tests. Canonical nonce keys normalize equivalent field encodings.
Only a valid proof can reach the nonce store. Keep `has` and `add` synchronous,
and serialize consumption across processes in the application's shared store.

`verify` checks canonical registry entries and verification-key hashes first,
then independently reconstructs the statement from the expected query and policy,
then requires the member's exact proof length and checks the proof against those
public inputs. It consumes the nonce after
those checks succeed. Rejection returns no released rows. A reason describes
canonical-key, statement, proof-encoding, cryptographic-proof, replay or store failure.

The admitted query is a bounded positive `SELECT DISTINCT` basic graph pattern.
Public numeric predicates apply to projected canonical nonnegative integer literals.
Hidden comparisons support canonical nonnegative `xsd:integer` values across the
full unsigned width and timezone-free `xsd:date` literals in `YYYY-MM-DD` form.
Negative integer constants reject under the shared native unsigned contract;
the `i64` member suffix denotes width, not signed arithmetic. A hidden date
comparison uses a dedicated date member. Mixed hidden date/integer comparisons
and multiple hidden date comparisons reject. Pattern constants such as a degree
IRI and a typed boolean are supported; unsupported cases reject.

The registry bundles these method-bound member families in either credential
capacity shape:

- `result_v5_k{1,2}_n16_p3_r4_f0_d10` for public predicates;
- `result_v5_k{1,2}_n16_p3_r4_f1_date_d10` for a hidden date comparison;
- `result_v5_k{1,2}_n16_p3_r4_f2_i64_d{10,17,20}` for hidden unsigned integer comparisons.

The WebAssembly host selects the member through native preparation, using the
query, selected credentials and accepted status snapshots. JavaScript looks up
that exact member in the registry.

Proofs support the returned selected rows only. The
[contract](../../skills/zk-query-proofs/references/successful-results.md)
states the native relation's limits.

Policy shape:

```ts
{
  trustedIssuers?: string[],
  issuers?: { [iri: string]: string },
  snapshots: { status_list: string, version: number, bits: number[] }[],
  minVersion: number,
  maxVersion: number
}
```

Both policy properties grant trust: `trustedIssuers` retains its existing key
list, and `issuers` maps issuer IRIs to compressed public keys. An issuer IRI in a
query must resolve through `issuers`; a trusted key alone does not name an IRI.
Unknown IRIs reject with `issuer-not-in-policy`.

## Issuer-scoped queries

```sparql
PREFIX vcq: <https://w3id.org/sparq/vcq#>
SELECT DISTINCT ?x WHERE {
  ?g vcq:verifiedIssuer <urn:issuer:a> .
  GRAPH ?g { ?s <urn:age> ?x . FILTER (?x >= 21) }
}
```

The scoped WHERE contains only issuer-declaration triples and variable-named
GRAPH blocks whose bodies use the admitted positive BGP/JOIN/FILTER fragment.
Every graph variable must have a declaration, occur in a GRAPH name, and appear
nowhere else, including projection, triple positions and FILTER operands.
All declarations must name the same issuer IRI. Multiple graph variables with
that IRI may draw facts from separate credentials of that issuer; graph variables
do not identify individual credentials. FILTERs belong inside their GRAPH block
and must reference variables bound by that block's own input.

The Rust adapter shares admission and lowering between proving and verification.
It lowers the scoped query to the existing flat relation, proves using only
credentials with the named issuer's policy key, and independently requires
**every authenticated public issuer slot** to carry that key. A different trusted
key rejects with `issuer-scope-mismatch`, before nonce-store access. Different
issuer IRIs reject with `per-fact-issuer-binding-needs-query-method`, even if they
map to the same key. The presentation and report retain the exact scoped request.

This binds all supporting facts to **one issuer per proof**. Per-fact binding
across issuers is the query method's job; this relation does not establish that
attribution. Unscoped queries keep their existing proving and verification
semantics. Research grade and not externally audited.

Version and index values must be safe nonnegative JavaScript integers. Status
bits use the native least-significant-bit-first byte encoding. Status references
are `{ status_list, version, index }`. Credential objects contain `graphNQuads`,
`salt`, `issuer`, `signature` and `statusRef`. They contain private material and
must remain in the prover's wallet.

`issuerKeygen()` returns `{ secretKeyHex, publicKey }` using the platform CSPRNG.
The caller must store and protect the secret; the package never persists it.
The key and signature scheme remain research grade and not externally audited.
`secretKeyHex` is exactly 64 lowercase, unprefixed hex digits encoding the canonical
little-endian scalar bytes. Import rejects zero, out-of-range scalars, malformed
hex and wrong lengths. `issue` also checks that `publicKey` matches the secret.
Serialized key objects can be restored by the caller and passed directly to `issue`.
Exported strings and JavaScript memory are not automatically erased.

```js
import { issuerKeygen, issue } from '@sparq-org/zk-result';
const issuer = issuerKeygen(); // Store issuer securely before issuing credentials.
const credential = issue({ issuer, graphNQuads, status });
```

`issue({ issuer, graphNQuads, status })` is the documented form; the older
`issue(issuer, graphNQuads, statusRef)` remains accepted. Both accept either key
format and reject objects mixing seed and secret fields. No option to `issue`
selects deterministic issuance.
It signs a string-canonical graph using the method-bound presentation attestation
and a fresh salt. Issuer identities, key custody and authenticated status
acquisition belong to the application. Ordinary key generation, `issue` salts and
proving secrets use platform CSPRNG entropy, including `crypto.getRandomValues` in Node; no stub entropy
is installed.
Generate nonces with the application's CSPRNG and supply the same nonce to both
parties.

## Reproducible test issuance

```js
import { testIssuerKeygen, testIssue } from '@sparq-org/zk-result';
const issuer = testIssuerKeygen('a1');
const credential = testIssue({ issuer, graphNQuads, status, seedHex: 'c3' });
```

Both seeds are hexadecimal unsigned `u64` values (one to sixteen hex digits,
optionally prefixed with `0x`); leading zeros and hex letter case do not change
their values. `testIssue` accepts only the object form and a test issuer's
`{ seedHex, publicKey }`, and refuses production `secretKeyHex` objects, including
mixed formats. A predictable salt lets anyone who can guess the graph test a
guess against the commitment.

Rust derives the salt with SHA-256 over the concatenation of the UTF-8 domain
`sparq.zk-result.test-issue.v1` followed by a zero byte, the issuance seed as
eight big-endian bytes, the canonical lowercase unprefixed compressed issuer
public-key hex as ASCII, the graph byte length as eight big-endian bytes, and
the exact `graphNQuads` UTF-8 bytes. The existing `field_from_hash_bytes` helper
maps the digest's low thirty-one bytes into the field. Thus the salt depends on
the issuance seed, issuer public key and exact graph text, including whitespace;
it does not depend on status. Status remains bound by the deterministic issuer
signature. Identical inputs return byte-identical credential JSON on native and
WebAssembly hosts. These credentials use the same proof relation as ordinary
issuance. Keep this predictable path exclusively in tests; `issue` always uses
fresh platform CSPRNG entropy.

## Verification report

Only successful `verify` calls return `report`:

```ts
{
  rule: { query: string, member: string },
  issuers: string[],
  subject: [],
  fresh: string,
  complete: false
}
```

`rule.query` is the receiver's exact `expectedQuery`; `rule.member` is the
canonical member selected during reconstruction. For a scoped query, `issuers`
is the single requested policy IRI. It means every authenticated public issuer
slot carries that IRI's policy key, including padding and slots unused by the
selected solutions. It does not provide per-fact attribution across issuers.

For an unscoped query, `issuers` resolves each authenticated public signature
slot to its policy IRIs in sorted IRI order, or its compressed lowercase key hex
when unnamed. Slots follow canonical lexicographic key order, and repeated slots
remain repeated; policies with only `trustedIssuers` retain the existing hex
report. This reports actual authenticated slots, not the entire trust set. A slot
may be padding or unused by the selected solutions. The list establishes neither
distinct credential count nor which issuer signed which fact: selected
credential/leaf references stay private.

`subject` is empty because this statement has no holder binding. `fresh` is the
canonical nonce only after the caller's store has consumed it. `complete` is
false because the statement proves that the returned solutions are solutions,
not that none is missing. Failed checks, replay and store errors return no report
and no released rows. Reconstruction alone yields candidate evidence and grants
no acceptance; the package releases the report only after proof verification
and successful synchronous nonce consumption. This remains research grade and
not externally audited.

## Rebuilt public-input layout

The receiver supplies its stored query, policy and nonce. The holder supplies
released rows, issuer slots and capacity/version metadata; the receiver validates
these and selects the canonical member itself. No presentation-supplied public
input vector is used. Each word is a BN254 scalar encoded as a fixed 32-byte
big-endian word (or equivalently a `0x`-prefixed field hex string). Booleans are
zero or one; unsigned integers retain their exact value. Arrays flatten in row
order, with the last index varying fastest.

In this table `K` is the member's signature-slot capacity and `F` its hidden
comparison capacity. Rows with `F` words are omitted when `F = 0`. Positions are
zero-based. Variables are sorted lexicographically, independently of projection
or first-occurrence order; variable identifiers are one-based, with zero marking
a pattern constant. There are three pattern slots, six variable slots and four
result slots. Query/row encodings use the existing string-canonical term encoding
with zero salt: `Poseidon2([tag, field_from_hash_bytes(BLAKE3(bytes))])`. The tag
is one for an IRI and two for a literal. IRI bytes are the IRI's UTF-8 bytes
without angle brackets; literal bytes are the native `Literal::to_string()` RDF
token, including quotes and any rendered datatype or language suffix. The helper
takes the digest's low thirty-one bytes. Public comparisons are checked
independently against released terms and contribute no hidden-comparison fields.

<!-- public-input-layout:start -->
| Start | Field | Words | Source and derivation |
|---|---|---|---|
| 0 | `challenge` | 1 | Receiver nonce, normalized as a field; must match the presentation challenge. |
| 1 | `version` | 1 | Contract version required by the independently selected member; holder version must agree. |
| 2 | `accepted_root` | 1 | Receiver policy: Poseidon2 accepted-set root over authenticated freshness-accepted status snapshots, sorted by list IRI and version, policy depth four. Each leaf binds list digest, version and status root. |
| 3 | `issuer_keys` | 2K | Holder's canonical ordered issuer slots, each checked against receiver trust policy; affine x then y for each compressed public key. |
| 3+2K | `pattern_count` | 1 | Receiver query's number of triple patterns. |
| 4+2K | `pattern_vars` | 9 | Receiver query: variable identifiers for subject, predicate and object of each pattern; unused patterns are zero. |
| 13+2K | `pattern_constants` | 9 | Receiver query: encoded constants; variable positions and unused patterns are zero. |
| 22+2K | `projection` | 6 | Receiver query: projected-variable booleans in sorted variable order; unused variables are false. |
| 28+2K | `result_count` | 1 | Holder's released distinct mapping count, checked against projection and capacity. |
| 29+2K | `results` | 24 | Holder's released terms encoded in sorted variable order; hidden variables, unused variables and unused rows are zero. |
| 53+2K | `filter_count` | 1 | Receiver query's hidden comparison count. |
| 54+2K | `filter_vars` | F | Receiver query: hidden comparison variable identifiers in query comparison order; unused comparison slots use identifier one. |
| 54+2K+F | `filter_ops` | F | Receiver query: comparison codes: less-than zero, less-or-equal one, greater-than two, greater-or-equal three, equality four, inequality five; unused slots are zero. |
| 54+2K+2F | `filter_bounds` | F | Receiver query: exact unsigned integer bound or Gregorian YYYYMMDD date key; unused slots are zero. |
<!-- public-input-layout:end -->

The total is `54+2K+3F` words. Public-predicate members have `F = 0`, date members
have `F = 1`, and hidden integer members have `F = 2`. Either `K = 1` or `K = 2`;
status depth changes the accepted root derivation and selected key, not this
layout. A snapshot's status tree uses the selected depth, least-significant-bit
first status bytes and revoked padding. Graph roots, salts, contents, signatures,
status references and fact-to-credential attribution are private inputs.
The offline layout test reads this table, compares it with each bundled ABI and
checks every word against `reconstructPublicInputs` and the prepared inputs.

The CRS comes from the official CDN documented by the pinned bb.js package,
`https://crs.aztec-cdn.foundation/g1.dat` and its corresponding `g2.dat`.
The runtime uses bb.js's `crsPath` option and initializes the synchronous WASM API
with the bundled capacity, avoiding the asynchronous constructor's unrelated
larger default allocation. Missing or incorrectly sized CRS files reject locally.
Download lengths and SHA-256 digests are pinned in `assets/crs.pins.json`. The
host build rejects a short, overlong or mismatched download. Circuit identities
are recorded in `assets/provenance.json`; generated host identities and source
URLs are recorded in `wasm/provenance.json`.
Runtime circuit JSON omits debug source maps; its compiled bytecode and ABI are
unchanged.

`assets/members.json` is the registry shared with the Rust build. A new supported
member needs its compiled ACIR, canonical VK and a table entry including its
fixed `proofBytes` wire length for the pinned ZK verifier target. Refresh the CRS if
its capacity requires more points. The Rust build embeds canonical VK hashes from
that registry; the verifier derives the selected member independently through the
shared relation. Do not edit circuit sources or substitute keys during packaging.

For development, `node scripts/build.mjs` builds the wasm32 host with Cargo,
runs the matching wasm-bindgen CLI and downloads and verifies the pinned CRS.
This host step requires no circuit compiler or native proving backend. Generated
`wasm/` and `assets/crs/` files are ignored and built in CI; npm tarballs include
them. The host step leaves committed circuit artifacts unchanged.

Only when regenerating members, run `node scripts/build-circuits.mjs` with the
repository's pinned circuit toolchain, then rerun the host step. It refreshes the
committed ACIR, canonical keys and circuit provenance, rejecting any generated
key that differs from the native canonical key pin. Consumers use the shipped
artifacts. Run `npm test` for behavioral tests. On Linux,
`python3 scripts/offline.py node --test --test-concurrency=1 test/*.test.js`
enforces network isolation using the system seccomp library. CI builds the host
and runs these tests offline. The proof exchange example in
`crates/sparq-zk-result-wasm/examples/crosscheck.rs` is separate manual evidence.
