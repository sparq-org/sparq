# VC-API interoperability endpoint

The default-off `sparq-vc/vc-api` feature provides a native test service and
in-process axum router. It signs credentials with Data Integrity
`eddsa-rdfc-2022`, using Ed25519, SHA-256, and standard RDFC-1.0. The native
JSON-LD expansion runs through `sparq-jsonld`; its expanded output reaches
`oxjsonld` for conversion to RDF. Language direction uses the JSON-LD 1.1
`i18n-datatype` mapping.

The issuer key is **test-only**. Generate a dedicated seed for this service,
never reuse it, and keep it in the environment. `GET /did.json` publishes only
the public verification method and explicitly marks the document `testOnly`,
using `https://sparq.jeswr.org/ns/vc-api#testOnly` for that context term.

## Configuration and native startup

Prepare the official context cache **before** starting the service, on a host
with network access. This script downloads VC v1/v2, examples v2, Data Integrity
v1/v2, Multikey v1, and DID v1, preserving the original bytes and writing a
SHA-256 provenance manifest. Review and retain that manifest with suite results.
The service reads the cache once at startup; request handling uses memory only.
Uncached context URLs and imports return an error; related resource IDs are never retrieved.

```sh
python3 crates/sparq-vc/vercel/prepare-contexts.py
export SPARQ_VC_TEST_SEED="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
cargo run -p sparq-vc --features vc-api --bin sparq-vc-api
```

| Environment variable | Default | Effect |
| --- | --- | --- |
| `SPARQ_VC_TEST_SEED` | Required | Ed25519 test seed, exactly 64 hexadecimal digits; never printed or stored by the service |
| `SPARQ_VC_CONTEXT_DIR` | `crates/sparq-vc/vercel/contexts` | Trusted context directory loaded by the native runner at startup |
| `SPARQ_VC_BIND` | `127.0.0.1:3000` | Native HTTP listener address |

## HTTP contract

| Route | Request | Success |
| --- | --- | --- |
| `POST /credentials/issue` | `{"credential": {...}, "options": {...}}` | 201, `{"verifiableCredential": {...}}` |
| `POST /credentials/verify` | `{"verifiableCredential": {...}, "options": {...}}` | 200, `{"verified":true,"checks":["proof"],"warnings":[],"errors":[]}` |
| `POST /presentations/verify` | `{"verifiablePresentation": {...}, "options": {...}}` | 200, the same verified proof-check result |
| `GET /did.json` | None | 200, public test issuer DID document |

Malformed JSON, invalid credentials, unsupported options, and invalid proofs
return 400 with `verified: false`, empty `checks`/`warnings`, and
`errors:[ProblemDetails]`, following the VC-API successor's
[Error Handling](https://w3c.github.io/vcalm/#error-handling) and
[Verification Errors vs. Warnings](https://w3c.github.io/vcalm/#verification-errors-vs-warnings)
sections. Each problem has `type: "https://sparq.jeswr.org/ns/vc-api#ValidationError"`,
`title: "ValidationError"`, HTTP `status`, a readable `detail`, and a `message`
string extension; the top-level body mirrors these problem fields.
JSON and JSON-LD media types are
accepted. Unknown routes return 404 and unsupported methods return 405 with
the same problem shape. Request bodies are limited to 256 KiB and JSON nesting depth 64.
Presentations contain at most 32 credentials; each document has at most 64
proofs, each proof at most 64 references, and chains at most 16 proofs deep.
RDF canonicalization has fixed input/output, HNDQ-call and permutation limits.

Issue accepts `type: "DataIntegrityProof"`, `cryptosuite: "eddsa-rdfc-2022"`,
`proofPurpose: "assertionMethod"`, `verificationMethod` naming this service's
key, `created`, `domain`, and `challenge` in `options`. Defaults select those
identifiers and the current UTC creation time. The service fills an absent
issuer and replaces a well-formed supplied issuer identifier with its own
`did:key`, preserving issuer metadata. Explicit malformed issuers are rejected.
Already secured credentials are rejected. VC v1 credentials receive the Data
Integrity v2 context if needed.

Verify accepts `checks: ["proof"]`, `domain`, `challenge`, and the supported
`type`, `cryptosuite`, and `proofPurpose` identifiers. It requires each terminal
proof's method to belong to the credential's `did:key` issuer and checks every
proof in a nonempty proof set. `previousProof` accepts a string or nonempty
list of IDs in the set; referenced proofs verify first and may have different
`did:key` controllers. Unknown, missing, duplicate, self and cyclic references
are errors. Other cryptosuites are unsupported.
Dates are validated as timestamp strings and checked for interval ordering;
verification with `checks: ["proof"]` does not apply a current-time validity policy.
Status and schema properties are checked for data-model shape. Related-resource
objects are signed as supplied without retrieving their IDs. If an ID names any
document already held in the offline cache, issuance and verification compare
all supplied digests against its exact held bytes. Every cached context and any
additional trusted cached document is covered. Unknown IDs are accepted unchanged
when their shape and digest encodings are valid; integrity of resources the
service does not hold is **not checked**. IDs must be unique absolute IRIs;
a digest is required. `digestSRI`
accepts SHA-256/384/512 standard base64 digests; `digestMultibase` accepts
`u`-prefixed unpadded base64url SHA-256 digests; optional `mediaType` must be a
nonempty string. Held-byte mismatches return `relatedResource digestSRI mismatch
for held resource` or `relatedResource digestMultibase mismatch for held resource`
in the problem's `detail` and `message`; these errors do not imply a remote fetch.
The staged credentials-v2 fixture and cache use the GitHub document's exact bytes,
which match the suite digests. The npm document has the same JSON-LD content
with different formatting and therefore different resource digests.

Presentation verification checks its context order, type, optional ID/holder,
every embedded credential through the credential verifier, and every presentation
proof with purpose `authentication`. Request challenge/domain options bind the
presentation proofs; credential assertion proofs use the credential verifier's
default proof check. An omitted holder is supported; when present, it must match
the terminal proof's `did:key` controller. A missing proof, unsupported kind, or
failed proof returns an error. Binding applies to every terminal proof, so this
profile rejects an issuer proof followed by a different notary's countersignature
and unordered credential proof sets containing another co-signer.

Issuance and verification share a compatibility leniency beyond the JSON-LD 1.1
context processing algorithm, matching jsonld.js 9.0.0. An inline context-array
entry containing `@context` is replaced once by that key's object value;
sibling keys are ignored. A wrapped string, array, null or other non-object entry
is an error. A direct local context wrapper is resolved first, so its value can
instead be an object, a context URL or an array. The same processing covers nested
and scoped contexts and documents loaded from the offline cache. Hashing uses the
processed contexts; the issued document retains the submitted context spelling.
Enveloping operations are outside this service's scope.

## Run the W3C suites on the maintainer's Mac

Requirements: Node.js 24 or newer, npm, Rust, Python 3, curl, git, internet access
for preparation, and permission to bind `127.0.0.1:3000`. Run from this checkout:

```sh
VC_SUITE_ROOT="$HOME/tmp/sparq-vc-suites" \
VC_REPORT_DIR="$HOME/tmp/sparq-vc-reports" \
  bash crates/sparq-vc/vercel/run-w3c-suites.sh
```

The script uses the inspected source revisions, prepares the context cache,
generates an environment-only test seed, builds and starts the native service,
and writes `localConfig.cjs` in each suite using the registry README's
[Testing locally](https://github.com/w3c/vc-test-suite-implementations#testing-locally)
format. Issuer, credential verifier, and `vpVerifiers` entries carry `vc2.0` and `eddsa-rdfc-2022` tags. It
installs Node dependencies, selects only sparq, runs both suites, and writes
per-suite JSON reports, logs, source revisions, the public DID, and `counts.json`
with tests/passes/failures/pending counts. It exits nonzero on suite failures,
still attempting both suites. Existing local configurations and suite JSON reports are never overwritten.
The runner also refuses an existing HTTP listener on its port.

Use a fresh suite directory, or check out the exact revisions printed by the
script in existing clones. `VC_SUITE_ROOT` defaults to
`${TMPDIR:-/tmp}/sparq-vc-suites`; `VC_REPORT_DIR` defaults to
`${TMPDIR:-/tmp}/sparq-vc-reports`. The script pins the local registry package
and invokes Mocha directly to avoid `pretest` fetching a moving registry branch.

**Suite counts are pending the external run.** Rust fixtures and handler tests
are separate evidence. The runner advertises `/presentations/verify` with the same cryptosuite tags.
Keep handler test evidence distinct from the external suite results.

## Vercel packaging (no deployment performed)

This directory is a standalone, `publish = false` deployment workspace under
`sparq-vc` so the CI selector attributes its changes to that crate. It is
excluded from the parent crate package and is not a root workspace member. Its
`vercel_runtime` handlers share the same axum router through `tower::oneshot`.
Each route has its own `[[bin]]`; `vercel.json` maps the public paths. The build
script embeds the prepared contexts, so the deployed handlers require no context
filesystem or network access. Only `SPARQ_VC_TEST_SEED` is read at runtime.

On a network-enabled host, prepare the cache as above, then compile the adapter:

```sh
cargo check --manifest-path crates/sparq-vc/vercel/Cargo.toml --bins
```

For deployment review, use `crates/sparq-vc/vercel` as the Vercel project root and retain
the repository checkout so the Rust path dependencies resolve. Prepare the cache
before `vercel build`; set `SPARQ_VC_TEST_SEED` using Vercel's environment settings.
The runtime package and Vercel build require external validation. Deployment is
a separate maintainer action after review.

## Local checks

```sh
python3 -B -m unittest discover -s crates/sparq-vc/vercel/tests -v
cargo test -p sparq-vc --features vc-api
cargo clippy -p sparq-vc --features vc-api --all-targets -- -D warnings
```

Tests drive handlers through `tower::oneshot` without binding TCP. Their local
context fixtures cover the vocabulary exercised by the credential tests; they
are not replacements for the official cache prepared for the W3C run. Fixture
provenance and license copies are in `crates/sparq-vc/tests/api-fixtures/`.
