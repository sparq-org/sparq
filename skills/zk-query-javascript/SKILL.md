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
remains the default. The six Solid App Suite targets are catalogued with full
envelopes (holder binding on insurance and trial) and prove natively only.
The WebAssembly prover is bounded at a dyadic circuit size of 2^21 and refuses
larger programs up front; compiler key agreement is claimed up to that bound.

`issuerKeygen()` uses system randomness and accepts no seed. Keep its opaque
`secretKeyHex` private. `testIssuerKeygen(seedHex)` is deterministic and test-only.
`issue({ issuer, graphNQuads, status })` draws a fresh salt;
`testIssue({ issuer, graphNQuads, status, seedHex })` accepts only test issuers and
uses the suite's deterministic entry point. N-Quads is the issuance input.
`status`, if supplied, is `{ proofConfigNQuads }`, carrying the suite proof
configuration. It does not enable status enforcement; `acceptedSnapshots` does.

Policy requires `bounds`, `trustedIssuers`, `trustMembers` and `audience`.
Each trust member is `{ issuer, verificationMethod, publicKey }`; its key set
must equal `trustedIssuers`. Rust derives its canonical authorization root and
membership paths. Signed graphs must include a VerifiableCredential root and
issuer IRI, and the proof configuration must name the authorized verification
method. Issuance preserves the supplied graph; it adds no envelope properties.
Canonical evaluation includes the method's authenticated issuer assertions.
Optional
`credentialQuadBounds` sets ordered slot limits. Request validity uses
`notBefore` and `notAfter` in Unix seconds, defaulting to creation and an hour
later. `maxResultSize` defaults to the table bound; `maxPresentationBytes` defaults
to sixteen mebibytes. `inputCommitment` names an agreed salted dataset commitment.

Optional `policy.coverage: { authority, scope, credential }` requires an explicit
`inputCommitment`. Choose the authority's issuer IRI, verification-method IRI and
public key independently for this scope. The scope names `registry`, `subject`,
`claimTypes` (an IRI set), `method` and `profile` as IRIs, and `periodStart` and
`periodEnd` as a half-open historical interval in Unix seconds. The signer can
call `coverageDocument({ issuer, verificationMethod, scope, inputCommitment, asOf,
expiresAt })` and pass the returned fields to `issue({ issuer: signingKey, ...fields })`.
The interval ends at or before `asOf`. The request must be created in
`[asOf, expiresAt)`, and its validity window must fit within that interval.
When `notAfter` is omitted, its default is capped at `expiresAt`.
The profile accepts one to 64 distinct claim types and up to 64 KiB each for the
signed claims document and proof configuration.

Keep the detached credential and `coverageCheckedAt` in the stored request.
The latter records the verifier's creation time in Unix seconds. The holder passes only the
dataset's ordered source credentials to `prove`, with the original presentation
salt as `backend.presentationSaltHex`. The request digest includes the SHA-256 of
the RDFC-1.0 canonical signed coverage claims.

Optional envelope policy, each a distinct compiled program:
`holderBinding: { issuers? }` (holder signs the request digest with the
credential's `confidenceMethod` Multikey; needs `openingProfile: 'long-objects'`),
`acceptedSnapshots: { digests, allowMissing? }` (verifier lists
`statusSnapshot({ list, purpose, bits }).digest`), `validityTime` and
`earliestCreated` (epoch milliseconds), `tableRowBounds` and `openingProfile`.
The holder calls `prove({ request, credentials, holder, statusLists })`, with
`holder` from `holderKeygen()` (or test-only `testHolderKeygen(seedHex)`).
Set bits, unaccepted or missing lists and missing or unrequested holder keys
refuse before proving; out-of-window, too-old or foreign-holder credentials refuse
at witness execution. Without envelope fields, `prove({ request, credentials })`
needs no extra evidence. Request-window instants stay in Unix seconds.

Replay protection lives in the caller's synchronous, atomic, durable
`consume(nonce)` store. Return `fresh` or `alreadyConsumed`; throw on failure.
Use a shared namespace across methods. A `Set` is only an in-memory test store.
The method consumes the original challenge last, after proof and final validity.
Missing stores fail closed; refusals carry no report.

Successful reports bind `rule` to the query and plan hash, `issuers` to the whole trusted
key set used for membership authorization (unlike `@sparq-org/zk-result`, where
`issuers` lists the slots the proof used), `subject` to the holder-bound issuers
(empty without holder binding), `fresh` to
the consumed challenge, and `complete` to the presented roots. `issuerRoot` names
the bound trust root; `trustMembers` records the authorized identities, methods
and keys. The membership proof does not identify each hidden signer.
Do not interpret the trust list as evidence that every member contributed.
`status`, `validityTime`, `earliestCreated` and `tableRowBounds` appear only when
enforced.

`report.exactness` is `presented-dataset` without an anchor and `agreed-dataset`
with one. `report.coverage` appears after successful verification of a covered
request and gives the signed authority, verification method, scope, timestamps,
dataset commitment and digest. Use that signed scope when relying on coverage.

In Rust, `CoverageClaims::issuance_documents` builds suite inputs and
`Coverage::verify` checks the detached credential against the chosen authority,
scope and anchor. `Coverage::check_valid_at` accepts a verifier-supplied instant
in Unix seconds. `Request::with_coverage(coverage, issued_at)` checks that instant
and the request window, then binds the verified evidence.
`StoredRequest::coverage_digest` exposes the protocol binding, and
`stored_request_encoding_profile` identifies the local byte encoding. The query
request retains the credential through `Request::coverage`.

Read the [package API and generated public-input layout](../../js/zk-query/README.md)
and the [Rust host adapter](../../crates/sparq-zk-query-wasm/README.md).
A source checkout builds bindings and pinned CRS with
`node js/zk-query/scripts/build.mjs`; circuit regeneration is a separate
`build-circuits.mjs` maintainer operation. Consumer tarballs are self-contained.
`CARGO_BUILD_JOBS` also limits concurrent native catalogue compilations; unset
means serial. Completed cache entries are checked against the committed pins.
After rebuilding, regenerate every plan, key and fixture together; the catalogue
test rejects stale assets and examples whose Rust-derived plan differs.
The toolchain workflow's package job rebuilds the catalogue, runs the offline
live bridge and verifies its own completion ledger. The fast JavaScript job
checks pinned native proof verification and the portable operator prover.

The catalogue route is a trust assumption: in Node, holder and verifier both rely
on the package's digest-pinned mapping from plan to program and key, rebuilt in CI
from the native compiler. Only catalogued query shapes work, and the catalogue
fixes each shape's row buckets (the holder's allow-list). A native party can check
any entry by compiling the plan itself.

For a verifier-agreed `inputCommitment`, the holder passes the existing commitment's
salt as `backend: { type: 'native', presentationSaltHex }` (or the corresponding
WebAssembly option). The salt is a full hexadecimal byte string, goes only into
private witness inputs, and is checked against the stored commitment in Rust.
Omitting this prover option draws a fresh random presentation salt.
