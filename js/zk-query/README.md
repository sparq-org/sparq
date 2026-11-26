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
`trustMembers` supplies the verifier's complete authorization entries as
`{ issuer, verificationMethod, publicKey }`. Its key set must equal
`trustedIssuers`; missing entries or mismatched keys refuse. Rust derives the
canonical authorization root and membership paths from these entries.
The optional credential-envelope policy fields are described under
[Credential envelope](#credential-envelope).
Issuance signs the supplied graph without adding credential properties. For
query proving, it must include a credential root typed as VerifiableCredential
and its issuer IRI. The proof configuration must name its verification-method
IRI. Both must match an authorized trust member; arbitrary signed graphs without
these properties receive a named refusal.
Canonical evaluation also includes the method's verified issuer assertions,
constructed in Rust from the authenticated trust members.
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
configuration. This argument is a configuration carrier, not status enforcement.

### Credential envelope

Every request authenticates issuers through `trustMembers`. A request may also ask
for any of the following; each one changes the compiled program, so only
catalogued combinations prove.

- `holderBinding: { issuers? }` requires a holder signature over the complete
  request digest by the key each credential names as its
  `confidenceMethod` Multikey. Omit `issuers` to bind every credential; a list
  binds only credentials from those issuers. The holder passes `holder` (from
  `holderKeygen()`, or `testHolderKeygen(seedHex)` in tests) to `prove`. Holder
  multikeys need `openingProfile: 'long-objects'`.
- `acceptedSnapshots: { digests, allowMissing? }` requires every
  `credentialStatus` entry to open an unset bit in a status list whose snapshot
  the verifier accepted. The verifier lists `statusSnapshot({ list, purpose, bits }).digest`
  for each list it checked; the holder passes the same lists as `statusLists`.
  `bits` has one `0` or `1` per entry. `allowMissing` admits credentials
  without a status entry.
- `validityTime` (epoch milliseconds) requires `validFrom <= validityTime <= validUntil`.
- `earliestCreated` (epoch milliseconds) requires the proof configuration's
  `created` instant to be no earlier.
- `tableRowBounds` gives one row limit per plan node, in plan order; an overflow refuses.
- `openingProfile` selects the lexical opening limits: `solid` (default),
  `long-objects` or `extended`.

```js
const holder = holderKeygen();            // holder keeps secretKeyHex
const snapshot = statusSnapshot(list);    // verifier checks the list, keeps snapshot.digest
const { presentation } = await prove({ request: stored, credentials, holder, statusLists: [list] });
```

A set status bit, a list whose snapshot the request does not accept, a missing
list and a missing or unrequested holder key refuse before proving. A credential
outside its validity window, older than `earliestCreated`, or bound to another
holder key has no witness and refuses at native witness execution. Request-window
instants (`notBefore`, `notAfter`) stay in Unix seconds.

`prove` returns `{ presentation, result }`. ASK results contain `boolean`; SELECT
results contain `variables` and ordered `rows` of RDF term strings or `null` for
unbound positions. Duplicate rows are retained when the query requires them.
`verify` returns `{ verified, released, reason?, report }`. Refusals contain no
report. Successful reports contain `rule` with the exact query and plan hash,
`issuers` with the whole trusted public-key set relied on for membership
authorization (this differs from `@sparq-org/zk-result`, whose `issuers` lists the
slots the proof used, so do not read the two fields the same way),
`subject` with the issuers whose credentials are holder-bound (empty without
holder binding), `fresh` with the consumed
challenge, `complete` relative to the presented roots, and the bound `issuerRoot`
and canonical `trustMembers` authorization set.
Membership keeps the individual signer hidden; the trust list does not claim that
every listed key signed a presented credential. `status`, `validityTime`,
`earliestCreated` and `tableRowBounds` appear only when the request enforces them;
no per-predicate issuer guarantees are reported.

Replay protection lives in the caller's store. A durable store implements
synchronous atomic `consume(nonce)` returning `fresh` or `alreadyConsumed`,
throwing on infrastructure failure. It must coordinate concurrent verifiers and
share a challenge namespace across methods. A `Set` is supported for in-memory
tests. Consumption follows statement, key, proof and final validity checks.
Missing stores fail closed.

For a verifier-agreed `inputCommitment`, the holder passes the existing commitment's
salt as `backend: { type: 'native', presentationSaltHex }` (or the corresponding
WebAssembly option). The salt is a full hexadecimal byte string, goes only into
private witness inputs, and is checked against the stored commitment in Rust.
Omitting this prover option draws a fresh random presentation salt.

## Prover backends and catalogue

The default `native` backend executes pinned native binaries on the holder's
machine, offline, as child processes. It is not a network service. Supply
`backend: { type: 'native', nargoPath, bbPath }`, or set `SPARQ_NARGO` and
`SPARQ_BB`; neither environment variable has a default. Each executable is copied
to a private temporary directory and checked against the version and digest in
[the binary pin file](assets/toolchain.pins.json) before execution. The available
binary pins select the supported native platform. Secret inputs use private files,
child output is bounded and untrusted, and temporary files are removed afterwards.

Catalogue entries marked `wasmProving` admit the opt-in WebAssembly backend and
run the same successful-proof and refusal contracts in the live suite, including
queries spanning credentials. Native remains the default.

The six Solid App Suite target queries (insurance 1a and 1b, trial eligibility,
and the recycled-content MIN, SUM and HAVING forms) are catalogued with their full
credential envelopes: issuer trust, status, validity and freshness on all six,
holder binding on the insurance and trial targets, and per-node row bounds. They
prove with the native backend only; the live suite proves and verifies each one
from Node and checks the envelope refusals.

`backend: 'wasm'` selects the in-process witness and prover implementation for
catalogue entries that admit it. Other entries return a refusal before witness
preparation. The 32-bit WebAssembly prover handles programs up to a dyadic
circuit size of 2^21 within its four-gibibyte memory; it refuses larger programs
up front, and no catalogue entry above that size admits it. Allocation errors reported by that backend become refusals. The same
internal interface exercises supplied-cell operator fixtures without an issuer
or credential-authentication claim.

The catalogue route is a trust assumption. In Node, neither party compiles: holder
and verifier both rely on the published package for the mapping from a plan to its
program and verification key. That mapping is pinned by digest inside the host
bindings and rebuilt in CI from the native compiler. Only catalogued query shapes
are accepted; any other query refuses with `missing catalogue plan`. A party with
the native toolchain can check any entry independently by compiling the plan
itself and comparing the program and key digests.

Proofs exist only when every intermediate table fits its row bucket, so a verifier
that could choose buckets freely could learn an unreleased intermediate count from
which requests succeed. In Node the catalogue fixes the buckets for each query
shape and acts as the holder's allow-list; callers cannot pick other buckets.

[The catalogue](assets/catalogue.json) is addressed by the Rust-derived plan hash.
Keys, artifacts, library sources and CRS are pinned and checked before use.
Unknown plans and keys differing from the catalogue refuse. Native compilation
and key creation belong to the separate maintainer `scripts/build-circuits.mjs`
step; authenticated compilation follows the catalogue's compiler-agreement policy.
Operator compiler agreement is a live-toolchain ratchet: every passing operator
program, including those that pull in the XPath library, must reach ABI and
bytecode agreement. Key agreement is claimed only up to the WebAssembly prover
bound below; a larger program is recorded separately with its native key. The maintainer source
snapshot carries the query core and tagged dependencies as local files, so native
witness execution needs no network or warmed user dependency cache.

The maintainer prepack check refuses missing or mismatched assets and dependencies.
Consumer tarballs include bindings, dependencies, sources, catalogue artifacts
and CRS. Installation has no build hook. A source checkout runs
`node scripts/build.mjs` after installing locked dependencies; that maintainer
step uses Cargo and the matching binding generator and fetches pinned CRS.
The proof packages share generated support files copied from
the repository's `js/zk-shared` sources; drift checks keep their runtime settings,
pins and offline wrapper aligned.

## Maintainer checks

Install locked dependencies with `npm ci --ignore-scripts --prefix js/zk-query`,
then build with `node js/zk-query/scripts/build.mjs`. The build also generates
shared support for the sibling package. Run the fast suite with
`python3 js/zk-query/scripts/offline.py node --test --test-concurrency=1 js/zk-query/test/*.test.js`.
The offline wrapper also works before a build, directly from a source checkout.

`build-circuits.mjs` regenerates missing catalogue artifacts and checks their pins.
Its `--cache` directory retains completed native compilations; `--cases` accepts
query/bounds profiles and `--write-pins` explicitly updates maintainer pins.
When the source bundle changes, a pin update must include every catalogue plan.
The builder stages the bundle, rebuilt programs and keys, and new catalogue.
It publishes these files after every selected build succeeds. `sources.mjs`
regenerates the bundle only when it matches the existing catalogue pin.
`CARGO_BUILD_JOBS` also bounds concurrent catalogue builds, defaulting to serial
when unset. Duplicate plan hashes share a build and must generate identical sources.
The build-input pin covers the generated files, the library bundle and the
toolchain pins. It also selects the cache directory for the program and key.
Cached source paths through symbolic links are refused; staging replaces ordinary
cached files without modifying another hard link's contents.
Catalogue checks derive each example's plan again through Rust and reject stale
key or program files left over from a rebuild. An example's optional `envelope`
(`holder`, `status`, `validity`, `freshness`, `tableRowBounds`, `openingProfile`)
names the policy features its program enforces.
Rebuild bindings after a pin update. An absent compiled plan refuses with
`missing catalogue plan` rather than choosing a different capacity.

The JS workflow runs issuance, stored-request refusals, a frozen native proof,
report checks, binary refusals and shared/layout drift checks without native
proof tools. The toolchain workflow rebuilds the catalogue and runs the explicit
Rust `live` test target, which invokes the offline Node live suite for available
authenticated profiles, a supplied-cell WebAssembly proof verified natively,
independent native seeded issuance and the coverage-manifest agreement ratchet.
The private live harness takes `SPARQ_QUERY_TEST_DIR` for resumable evidence and
`SPARQ_ISSUANCE_REFERENCE` for its freshly built native reference executable.
Neither variable affects the installed API.

<!-- catalogue-pending:start -->

All listed test profiles have pinned catalogue entries.
<!-- catalogue-pending:end -->

## Public-input layout

The following section is generated through the query crate's layout and audited
artifact ABI. Arrays flatten in declaration order; cells use the shown field
order, and SELECT tables emit padded rows before their length. Scalars use
canonical big-endian field words. Array lengths come from the audited ABI; byte digests retain their fixed length.
Bounds flatten as credential, quad and row limits.

<!-- public-input-layout:start -->

- `request_digest: [u8; length]`
- `input_commitment: [u8; length]`
- `signature_mode: u8`
- `bounds: [u32; length]`
- `parameters: [TermCell; length]`
- `indices: [u32; length]`
- `result: bool`
- `issuer_root: Field`
- `issuer_depth: u32`
- `holder_required: bool`
- `holder_issuers_present: bool`
- `holder_issuer_count: u32`
- `holder_issuers: [Field; length]`
- `status_required: bool`
- `allow_missing_status: bool`
- `snapshot_root: Field`
- `snapshot_depth: u32`
- `validity_required: bool`
- `now: u64`
- `freshness_required: bool`
- `earliest_created: u64`
- `credential_quad_bounds: [u32; length]`
- `revealed_count: u32`
- `revealed_messages: [Field; length]`
- `revealed_keys: [Field; length]`
- `statement_binding: Field`

Cell field order: `bound`, `kind`, `enc`, `key_class`, `key`, `dt`, `lang`, `scope`.
SELECT table field order: `rows`, `len`.
<!-- public-input-layout:end -->

## License

MIT.
