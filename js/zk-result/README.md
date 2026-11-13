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
  trustedIssuers: string[],
  snapshots: { status_list: string, version: number, bits: number[] }[],
  minVersion: number,
  maxVersion: number
}
```

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
const credential = issue(issuer, graphNQuads, statusRef);
```

`testIssuerKeygen(seedHex)` retains its deterministic `{ seedHex, publicKey }`
format and is exclusively a test helper. `issue(issuer, graphNQuads, statusRef)`
accepts either key format and rejects objects mixing seed and secret fields.
It signs a string-canonical graph using the method-bound presentation attestation
and a fresh salt. Issuer identities, key custody and authenticated status
acquisition belong to the application. Random keys, salts and proving secrets use
platform CSPRNG entropy, including `crypto.getRandomValues` in Node; no stub entropy
is installed.
Generate nonces with the application's CSPRNG and supply the same nonce to both
parties.

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
