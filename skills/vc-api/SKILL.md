---
name: vc-api
description: Run the opt-in sparq-vc credential issue/verify HTTP test endpoint, prepare its offline JSON-LD context cache, embed its axum router, and run the W3C VC Data Model 2.0 and eddsa-rdfc-2022 suites locally.
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
  A rejection returns 400 with `verified:false` and `{type,message}` errors.
- `GET /did.json` returns a public DID document with `testOnly: true`.

Issue options: `type`, `cryptosuite`, `proofPurpose`, `verificationMethod`,
`created`, `domain`, `challenge`. Supported identifiers are `DataIntegrityProof`,
`eddsa-rdfc-2022`, and `assertionMethod`. Verify accepts those identifiers,
`checks: ["proof"]`, `domain`, and `challenge`. Defaults select a proof check;
current-time credential validity, presentation operations, and proof chains
are outside this endpoint's scope. Requests are limited to 256 KiB.

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
