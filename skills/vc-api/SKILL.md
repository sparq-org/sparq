---
name: vc-api
description: Run the opt-in sparq-vc credential issue/verify and presentation verify HTTP test endpoint, prepare its offline JSON-LD context cache, embed its axum router, and run the W3C VC Data Model 2.0 and eddsa-rdfc-2022 suites locally.
license: MIT
---

# VC-API test endpoint

Enable `sparq-vc`'s default-off `vc-api` feature explicitly. Existing RDF
sign/verify users can continue using `verifiable-credentials` without it.

```sh
python3 crates/sparq-vc/vercel/prepare-contexts.py
export SPARQ_VC_TEST_SEED="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
cargo run -p sparq-vc --features vc-api --bin sparq-vc-api
```

The issuer key is test-only; never reuse it. `SPARQ_VC_TEST_SEED` is required
and contains exactly 64 hexadecimal digits. `SPARQ_VC_CONTEXT_DIR` defaults to
`crates/sparq-vc/vercel/contexts` and is read once at startup. `SPARQ_VC_BIND` defaults
to `127.0.0.1:3000`. Prepare official contexts before startup; requests never
retrieve remote contexts or resources.

- `POST /credentials/issue` accepts `{credential, options?}` and returns 201
  with `{verifiableCredential}`. It selects its own `did:key` issuer while
  preserving optional issuer metadata.
- `POST /credentials/verify` accepts `{verifiableCredential, options?}` and
  returns 200 with `{verified:true, checks:["proof"], warnings:[], errors:[]}`.
- `POST /presentations/verify` accepts `{verifiablePresentation, options?}`
  and returns the same proof-check result after checking every presentation
  proof and every embedded credential through the credential verifier.
- Rejections return 400 with `verified:false`, empty `checks`/`warnings`, and
  `errors:[ProblemDetails]`. Each problem has a URL-valued `type`, `title`, HTTP
  `status`, readable `detail`, and a `message` string extension. The top-level
  body also exposes that problem's fields for direct error readers.
  Unknown routes return 404 and unsupported methods return 405 with that shape.
- `GET /did.json` returns a public DID document with `testOnly: true`.

Issue options: `type`, `cryptosuite`, `proofPurpose`, `verificationMethod`,
`created`, `domain`, `challenge`. Supported identifiers are `DataIntegrityProof`,
`eddsa-rdfc-2022`, and `assertionMethod`. Verify accepts those identifiers,
`checks: ["proof"]`, `domain`, and `challenge`. Defaults select a proof check;
current-time credential validity is outside this endpoint's scope. Presentation
proofs use `authentication`; supplied challenge/domain options bind the
presentation's proofs. Embedded credentials use assertion proofs and the existing
credential verifier's default proof check. A supplied holder must match the
terminal presentation proof's `did:key` controller; holder omission is supported.
Unsupported proof kinds and unsigned documents are errors.

Proofs may be an object or a nonempty set. `previousProof` is a string or
nonempty list of proof IDs in that set. Dependencies verify first; terminal
credential proofs must use the issuer's key, while referenced proofs may use
other `did:key` controllers. Unknown, missing, duplicate, self and cyclic
references reject. This profile binds every terminal proof (one that no other
proof references) to the issuer or holder. An issuer proof followed by a different
notary's countersignature, and an unordered credential proof set with another
co-signer, are rejected even when their signatures are valid.

Limits: 256 KiB per request, JSON nesting depth 64,
32 credentials per presentation, 64 proofs per document, 64 references per proof,
and chain depth 16 (including the current proof).

Issuance preserves and signs `relatedResource` objects without dereferencing
IDs. If an ID belongs to any document held in the offline cache, both issuance
and verification compare each supplied digest against those exact held bytes.
This covers every context URL and any additional trusted cached document;
a mismatch returns a readable held-resource digest error. For unknown IDs,
resource integrity is **not checked**: validly encoded descriptors are signed
unchanged, without retrieving their contents. Each needs a unique absolute `id` and
`digestSRI` (SHA-256/384/512, standard base64) or `digestMultibase` (`u`, unpadded
base64url SHA-256); optional `mediaType` is a nonempty string. Contexts use only
the startup cache. The staged credentials-v2 fixture uses the GitHub context's
exact bytes; the npm copy has identical JSON-LD content but different digest bytes.

Ordinary inline `{"@context": ...}` context-document entries are invalid JSON-LD.
Verification makes one compatibility exception for the fixed AlumniCredential
chain in `tests/api-fixtures/previous-proof.json`, matching its document ID, types,
exact context and two-proof reference shape (a string or singleton-list reference). Only that root credential context is
unwrapped. Issuance always uses strict context processing and never emits this
form. Other inline context documents remain errors.

To embed without a listener, use `sparq_vc::api::{router, Service,
OfflineContexts}`. `OfflineContexts::from_directory(&Path)` loads all
`CONTEXT_FILES`. `from_documents(BTreeMap<String,String>)` accepts a trusted
in-memory cache and requires all registered URLs. `Service::new(&[u8;32], cache)`
constructs state; `Service::from_env()` reads the native configuration.
`Service::from_env_with_contexts(cache)` reads the seed with a supplied cache.
`Service::did_document()` exposes its public identity. `router(service)` returns
an axum router suitable for `tower::ServiceExt::oneshot`. Startup and request
validation errors use `ApiError`.

Run both W3C suites outside restricted environments:

```sh
VC_SUITE_ROOT="$HOME/tmp/sparq-vc-suites" \
VC_REPORT_DIR="$HOME/tmp/sparq-vc-reports" \
  bash crates/sparq-vc/vercel/run-w3c-suites.sh
```

The runner prepares local configurations with `vc2.0` and `eddsa-rdfc-2022`
tags and writes `counts.json`. Suite pass/fail counts remain pending until
that run. See [the deployment runbook](../../crates/sparq-vc/vercel/README.md) for exact
requirements, status codes, data-model scope, cache preparation, and the
separate Vercel packaging workspace.
