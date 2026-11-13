# Authenticated query proofs from Node

Research grade and not externally audited. `@sparq-org/zk-query` proves a bounded
query over complete Merkle-suite credentials. Exactness is closed-world: every
quad under each presented signed root is included. The holder chooses the roots
unless the verifier supplies an agreed dataset commitment. This does not establish
that every credential available elsewhere was presented. Repeated questions can
reveal more through their answers. Queries, capacities and released results are public.

## API

```js
import { issuerKeygen, issue, request, prove, verify } from '@sparq-org/zk-query';
const issuer = issuerKeygen();
const credential = issue({ issuer, graphNQuads, status });
const stored = request({ query, policy, nonce });
const { presentation, result } = await prove({ request: stored, credentials });
const checked = await verify({ request: stored, presentation, seenNonces });
if (!checked.verified) throw new Error(checked.reason);
```

The verifier stores its own request and supplies that copy to `verify`.
`policy` requires `bounds`, `trustedIssuers` and `audience`. Bounds contain
`credentials`, `quadsPerCredential` and `rowsPerTable`; trusted issuers are suite
public keys. `credentialQuadBounds` optionally refines ordered credential slots.
`notBefore` and `notAfter` are Unix seconds; defaults start at request creation
and end an hour later. `maxResultSize` defaults to the table bound and
`maxPresentationBytes` to sixteen mebibytes. `inputCommitment` optionally binds
an agreed salted dataset commitment. Bounds and policy values come from the stored
request and never from presentation structure.

Nonce and test seeds are full hexadecimal byte strings. Returned nonce strings
are canonical lowercase hexadecimal. Production `secretKeyHex` is opaque random
secret material for the suite's key derivation. `issuerKeygen()` takes no seed.
`testIssuerKeygen(seedHex)` returns a deterministic test issuer;
`testIssue({ issuer, graphNQuads, status, seedHex })` uses the suite's deterministic
issuance entry point and refuses production issuer objects. Ordinary `issue`
draws a fresh platform-CSPRNG salt and refuses an issuance seed.

`status`, when supplied to issuance, is `{ proofConfigNQuads }`: the exact Data
Integrity proof configuration, including the suite declaration, with optional
creation and verification-method metadata. Omission selects the suite-only
configuration. This argument is a configuration carrier; credential status
checking requires the envelope integration. Required holder binding, accepted
status snapshots, validity time and freshness policies are currently refused.

`prove` returns `{ presentation, result }`. ASK results contain `boolean`; SELECT
results contain `variables` and ordered `rows` of RDF term strings or `null` for
unbound positions. Duplicate rows are retained when the query requires them.
`verify` returns `{ verified, released, reason?, report }`. Refusals contain no
report. Successful reports contain `rule` with the exact query and plan hash,
`issuers` with the trusted public-key set relied on for membership authorization,
`subject` empty because holder binding is unavailable, `fresh` with the consumed
challenge, `complete` relative to the presented roots, and the bound `issuerRoot`.
Membership keeps the individual signer hidden; the trust list does not claim that
every listed key signed a presented credential. No status roots, times or
per-predicate issuer guarantees are reported without their enforcement.

Replay protection lives in the caller's store. A durable store implements
synchronous atomic `consume(nonce)` returning `fresh` or `alreadyConsumed`,
throwing on infrastructure failure. It must coordinate concurrent verifiers and
share a challenge namespace across methods. A `Set` is supported for in-memory
tests. Consumption follows statement, key, proof and final validity checks.
Missing stores fail closed.

## Prover backends and catalogue

The default `native` backend executes pinned native binaries on the holder's
machine, offline, as child processes. It is not a network service. Supply
`backend: { type: 'native', nargoPath, bbPath }`, or set `SPARQ_NARGO` and
`SPARQ_BB`; neither environment variable has a default. Each executable is copied
to a private temporary directory and checked against the version and digest in
[the binary pin file](assets/toolchain.pins.json) before execution. The available
binary pins select the supported native platform. Secret inputs use private files,
child output is bounded and untrusted, and temporary files are removed afterwards.

`backend: 'wasm'` selects the in-process witness and prover implementation for
catalogue entries that admit it. Other entries return a refusal before witness
preparation. Allocation errors reported by that backend become refusals. The same
internal interface exercises supplied-cell operator fixtures without an issuer
or credential-authentication claim.

[The catalogue](assets/catalogue.json) is addressed by the Rust-derived plan hash.
Keys, artifacts, library sources and CRS are pinned and checked before use.
Unknown plans and keys differing from the catalogue refuse. Native compilation
and key creation belong to the separate maintainer `scripts/build-circuits.mjs`
step; authenticated compilation follows the catalogue's compiler-agreement policy.
Operator compiler agreement is a live-toolchain ratchet. The maintainer source
snapshot carries the query core and tagged dependencies as local files, so native
witness execution needs no network or warmed user dependency cache.

Consumer tarballs include bindings, dependencies, sources, catalogue artifacts
and CRS. Installation has no build hook. A source checkout runs
`node scripts/build.mjs` after installing locked dependencies; that maintainer
step uses Cargo and the matching binding generator and fetches pinned CRS.
The proof packages share generated support files copied from
[`js/zk-shared`](../zk-shared/README.md); drift checks keep their runtime settings,
pins and offline wrapper aligned.

## Public-input layout

The following section is generated through the query crate's layout and audited
artifact ABI. Arrays flatten in declaration order; cells use the shown field
order, and SELECT tables emit padded rows before their length. Scalars use
canonical big-endian field words. Lengths follow the catalogue profile's capacities.

<!-- public-input-layout:start -->
<!-- public-input-layout:end -->

## License

MIT.
