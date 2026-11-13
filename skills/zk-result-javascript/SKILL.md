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

Bundled method-bound families are `result_v5_k{1,2}_n16_p3_r4_f0_d10`,
`result_v5_k{1,2}_n16_p3_r4_f1_date_d10` and
`result_v5_k{1,2}_n16_p3_r4_f2_i64_d{10,17,20}`. Native preparation selects the
member from the query, selected credentials and accepted status snapshots;
JavaScript uses that exact registry entry. Unsupported members reject. The
package ships dependencies and proof artifacts, uses only the
WASM proving backend and bundled CRS, and needs no network or native toolchain to
install its tarball or prove and verify.

`testIssuerKeygen(seedHex)` and `issue(issuer, graphNQuads, statusRef)` are test
issuance helpers. The seed helper is deterministic and unsuitable for production
key generation. Issuance salts and proof randomness use the platform CSPRNG;
no deterministic or stub source is registered. Keep credentials and witness maps
private.

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
