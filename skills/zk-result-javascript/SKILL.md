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
canonical integer predicates and IRI/literal pattern constants. Unsupported
members reject. The package ships dependencies and proof artifacts, uses only the
WASM proving backend and bundled CRS, and needs no network or native toolchain to
install its tarball or prove and verify.

Use `issuerKeygen()` to obtain `{ secretKeyHex, publicKey }` from the platform
CSPRNG. The caller must store and protect the secret; the package never persists
it. The key and signature scheme remain research grade, not externally audited.
`secretKeyHex` encodes the canonical little-endian scalar bytes as exactly
64 lowercase, unprefixed hex digits. `issue(issuer, graphNQuads, statusRef)` imports
that secret, rejects zero/out-of-range scalars and malformed encodings, and checks
the matching public key. Caller-restored JSON keys can be passed directly to it.
Exported strings and JavaScript memory are not automatically erased.

`testIssuerKeygen(seedHex)` retains its deterministic `{ seedHex, publicKey }`
format for tests only. Issuance accepts either key format and rejects mixed seed
and secret fields. Random keys, issuance salts and proof randomness use the
platform CSPRNG, including `crypto.getRandomValues` on wasm32; no stub source is
registered. Keep credentials and witness maps private.

See the [package API and policy shapes](../../js/zk-result/README.md) and the
[successful-result contract](../zk-query-proofs/references/successful-results.md).
The shared registry is `js/zk-result/assets/members.json`; adding a supported
member requires its compiled ACIR, canonical VK and registry entry, including
its fixed `proofBytes` wire length.

A source checkout generates ignored host bindings and pinned CRS with
`node js/zk-result/scripts/build.mjs`. This development step uses Cargo and the
matching binding generator, downloads the CRS and checks committed length and
SHA-256 pins. It does not invoke the circuit toolchain. Member regeneration is a
separate `node js/zk-result/scripts/build-circuits.mjs` step; rerun the host build
afterward. Consumer tarballs already contain all generated runtime files.
