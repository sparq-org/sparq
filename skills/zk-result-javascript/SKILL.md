---
name: zk-result-javascript
description: Prove and independently verify selected successful-result predicates from Node with bundled WebAssembly, circuits, canonical keys and CRS; no native proving toolchain or network at runtime.
---

# Successful-result proofs from Node

Use `@sparq-org/zk-result` for sparq's method-bound commitment-signed credentials.
Research grade, not externally audited; not standard VCs. There is no answer or
wallet completeness, absence proof or holder identity. Issuer identities,
capacities and result size are observable.

```js
import { prove, verify } from '@sparq-org/zk-result';
const { presentation, result } = await prove({ query, credentials, policy, nonce });
const checked = await verify({
  expectedQuery: query, presentation, policy, nonce, seenNonces,
});
if (!checked.verified) throw new Error(checked.reason);
```

The relying party supplies its expected query, issuer trust list, authenticated
status snapshots and fresh CSPRNG nonce. A caller-owned synchronous `has`/`add`
nonce store is consulted only after canonical-key, statement and proof checks
succeed. Persist and coordinate that store outside the library. Failed verification
releases no rows. Proof encoding must match the selected member's fixed wire
length before cryptographic verification.

Use bounded positive `SELECT DISTINCT` basic graph patterns with projected
canonical nonnegative integer predicates and IRI/literal pattern constants.
Hidden FILTER operands support full-width unsigned `xsd:integer` and canonical
timezone-free `xsd:date` comparisons. Negative integer constants reject; the
`i64` suffix denotes unsigned width. Mixed hidden date/integer comparisons and
multiple hidden date comparisons reject.

Policy `issuers?: Record<string, string>` maps issuer IRIs to trusted compressed
public keys; `trustedIssuers?: string[]` remains an additional trust list.
For a scoped query, use `?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ... }`,
where `vcq:` is `https://w3id.org/sparq/vcq#`. The WHERE may contain only those
declarations and GRAPH blocks over the admitted BGP/JOIN/FILTER fragment.
Each graph variable must be declared, used as a graph name, and occur nowhere
else; FILTERs stay inside GRAPH blocks. Every declaration must name the same IRI,
resolved through the policy map, or admission rejects `issuer-not-in-policy`.
Different IRIs reject `per-fact-issuer-binding-needs-query-method`, even if their
keys coincide. Multiple graph variables with the same IRI may use separate
credentials. The Rust adapter lowers both paths, restricts proving credentials
to that key, and checks every authenticated public issuer slot against it;
a different key rejects `issuer-scope-mismatch` before nonce-store access.
This binds all supporting facts to one issuer per proof. Per-fact binding across
issuers is the query method's job. Research grade, not externally audited.

Bundled method-bound families are `result_v5_k{1,2}_n16_p3_r4_f0_d10`,
`result_v5_k{1,2}_n16_p3_r4_f1_date_d10` and
`result_v5_k{1,2}_n16_p3_r4_f2_i64_d{10,17,20}`. Native preparation selects the
member from the query, selected credentials and accepted status snapshots;
JavaScript uses that exact registry entry. Unsupported members reject. The
package ships dependencies and proof artifacts, uses only the
WASM proving backend and bundled CRS, and needs no network or native toolchain to
install its tarball or prove and verify.

Use `issuerKeygen()` to obtain `{ secretKeyHex, publicKey }` from the platform
CSPRNG. The caller must store and protect the secret; the package never persists
it. The key and signature scheme remain research grade, not externally audited.
`secretKeyHex` encodes the canonical little-endian scalar bytes as exactly
64 lowercase, unprefixed hex digits. `issue({ issuer, graphNQuads, status })` imports
that secret, rejects zero/out-of-range scalars and malformed encodings, and checks
the matching public key. Caller-restored JSON keys can be passed directly to it.
Exported strings and JavaScript memory are not automatically erased.

`testIssuerKeygen(seedHex)` retains its deterministic `{ seedHex, publicKey }`
format for tests only. Issuance accepts either key format and rejects mixed seed
and secret fields. The older positional `issue(issuer, graphNQuads, statusRef)`
remains accepted. Both forms always draw a fresh salt; there is no deterministic
option to `issue`.

Use `testIssue({ issuer, graphNQuads, status, seedHex })` only for repeatable tests.
It accepts only the seed-based test issuer, refusing production `secretKeyHex`
and mixed objects. A predictable salt lets anyone who can guess the graph test
a guess against the commitment. The issuance seed, like the test key seed, is
a hexadecimal unsigned `u64` (optional `0x`, leading zeros and hex case accepted).
Rust derives the salt from a separate issuance domain, that seed's numeric value,
the canonical issuer public key and exact graph UTF-8 bytes, including whitespace;
status does not affect the salt but remains signature-bound. Identical inputs
give byte-identical credentials that prove and verify through the ordinary path.
The [package README](../../js/zk-result/README.md#reproducible-test-issuance)
specifies the byte derivation. Random keys, ordinary issuance salts and proof randomness use the
platform CSPRNG, including `crypto.getRandomValues` on wasm32; no stub source is
registered. Keep credentials and witness maps private.

Successful `verify` returns `report` alongside `verified` and `released`:
`{ rule: { query, member }, issuers, subject: [], fresh, complete: false }`.
`rule` contains the exact receiver-supplied query and independently selected
member. Scoped `issuers` is the single requested policy IRI: every authenticated
public issuer slot carries its policy key. Unscoped `issuers` resolves rebuilt
signature-slot coordinates to matching policy IRIs (sorted per slot), or canonical
compressed key hex if unnamed, in lexicographic key-slot order, preserving repeats.
Policies with only `trustedIssuers` retain their existing hex report.
Every slot authenticates a private credential trusted by the receiver; a slot can
be padding or unused by the selected solutions. This is neither
the whole trust list nor an attribution of facts to issuers, and it makes no
distinct credential-count claim. `subject` is empty because this statement has
no holder binding. `fresh` is the canonical nonce only after store consumption.
`complete` is false because returned solutions have support, without proving
that none is missing. Refusals, replay and store failures have no report. Raw
reconstruction is candidate evidence only; it performs no proof check. See the
[checked public-input layout](../../js/zk-result/README.md#rebuilt-public-input-layout)
for each word's order, derivation and source party. Research grade, not externally
audited.

See the [package API and policy shapes](../../js/zk-result/README.md) and the
[successful-result contract](../zk-query-proofs/references/successful-results.md).
The shared registry is `js/zk-result/assets/members.json`; adding a supported
member requires its compiled ACIR, canonical VK and registry entry, including
its fixed `proofBytes` wire length.

A source checkout generates ignored host bindings and pinned CRS with
`node js/zk-result/scripts/build.mjs`. This development step uses Cargo and the
matching binding generator, downloads the CRS and checks committed length and
SHA-256 pins. It does not invoke the circuit toolchain. Member regeneration is a
separate `node js/zk-result/scripts/build-circuits.mjs` step that checks every
generated key against the native canonical key pin; rerun the host build
afterward. Consumer tarballs already contain all generated runtime files.
