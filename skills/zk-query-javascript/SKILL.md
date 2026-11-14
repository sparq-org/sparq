---
name: zk-query-javascript
description: Build requests, issue Merkle-suite credentials and verify bounded authenticated query proofs from Node; holder proving uses a pinned local native toolchain with an optional WebAssembly backend.
---

# Authenticated query proofs from Node

Use `@sparq-org/zk-query` for the catalogue-backed query method. Research grade,
not externally audited. It proves exact answers over every quad of each presented
signed root. This is closed-world completeness; absent an agreed commitment, the
holder chooses which roots to present. Repeated answers may disclose more.

```js
import { request, prove, verify } from '@sparq-org/zk-query';
const stored = request({ query, policy, nonce });
const { presentation, result } = await prove({ request: stored, credentials });
const checked = await verify({ request: stored, presentation, seenNonces });
if (!checked.verified) throw new Error(checked.reason);
```

Keep the verifier's own stored request. Its query, audience, bounds, challenge,
issuer set and key are authoritative. Do not substitute presentation metadata.
Missing catalogue plans and mismatched keys refuse. Request constants remain
public parameters, while capacities select a compiled shape.

The native prover runs on the holder's machine as child processes, offline.
Set `SPARQ_NARGO` and `SPARQ_BB`, or pass
`backend: { type: 'native', nargoPath, bbPath }`; environment defaults are absent.
The package requires the pinned version and digest for the native platform.
It copies binaries and witness material into private temporary directories and
removes them afterwards. It does not send credentials to a proving service.
`backend: 'wasm'` is opt-in and refuses entries outside its supported catalogue.
Measured authenticated entries admit it and run the same proof and refusal
contracts in the live suite, including queries spanning credentials. Native
remains the default pending the full generated consumer-query measurements.

`issuerKeygen()` uses system randomness and accepts no seed. Keep its opaque
`secretKeyHex` private. `testIssuerKeygen(seedHex)` is deterministic and test-only.
`issue({ issuer, graphNQuads, status })` draws a fresh salt;
`testIssue({ issuer, graphNQuads, status, seedHex })` accepts only test issuers and
uses the suite's deterministic entry point. N-Quads is the issuance input.
`status`, if supplied, is `{ proofConfigNQuads }`, carrying the suite proof
configuration. It does not enable status enforcement.

Policy requires `bounds`, `trustedIssuers` and `audience`. Optional
`credentialQuadBounds` sets ordered slot limits. Request validity uses
`notBefore` and `notAfter` in Unix seconds, defaulting to creation and an hour
later. `maxResultSize` defaults to the table bound; `maxPresentationBytes` defaults
to sixteen mebibytes. `inputCommitment` names an agreed salted dataset commitment.
Required holder binding, snapshots, credential validity and freshness receive
the primitive's envelope-integration refusal.
Underlying credential-policy instants use epoch milliseconds. Request-window
instants retain their seconds representation; the extra credential policies remain refused.

Replay protection lives in the caller's synchronous, atomic, durable
`consume(nonce)` store. Return `fresh` or `alreadyConsumed`; throw on failure.
Use a shared namespace across methods. A `Set` is only an in-memory test store.
The method consumes the original challenge last, after proof and final validity.
Missing stores fail closed; refusals carry no report.

Successful reports bind `rule` to the query and plan hash, `issuers` to the trusted
key set used for membership authorization, `subject` to an empty list, `fresh` to
the consumed challenge, and `complete` to the presented roots. `issuerRoot` names
the bound trust root. The membership proof does not identify each hidden signer.
Do not interpret the trust list as evidence that every member contributed.

Read the [package API and generated public-input layout](../../js/zk-query/README.md)
and the [Rust host adapter](../../crates/sparq-zk-query-wasm/README.md).
A source checkout builds bindings and pinned CRS with
`node js/zk-query/scripts/build.mjs`; circuit regeneration is a separate
`build-circuits.mjs` maintainer operation. Consumer tarballs are self-contained.
`CARGO_BUILD_JOBS` also limits concurrent native catalogue compilations; unset
means serial. Completed cache entries are checked against the committed pins.
After rebuilding, regenerate every plan, key and fixture together; the catalogue
test rejects stale assets and examples whose Rust-derived plan differs.

For a verifier-agreed `inputCommitment`, the holder passes the existing commitment's
salt as `backend: { type: 'native', presentationSaltHex }` (or the corresponding
WebAssembly option). The salt is a full hexadecimal byte string, goes only into
private witness inputs, and is checked against the stored commitment in Rust.
Omitting this prover option draws a fresh random presentation salt.
