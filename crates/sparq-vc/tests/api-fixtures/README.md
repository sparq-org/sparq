# VC-API test fixtures

`data-model.json` collects credential inputs from the W3C VC Data Model 2.0
suite at revision `62836458636bd1b8309214626ec8fc85de43f8bd`, under
`tests/input/` (including `names-and-descriptions/`). The `name` field preserves
the source filename. Temporal placeholder strings are replaced with fixed
past/future dates. The handler test mirrors the suite's issuer injection.

Source: <https://github.com/w3c/vc-data-model-2.0-test-suite>.
Copyright 2022–2024 Digital Bazaar, Inc. Distributed under the
[W3C 3-clause BSD License](LICENSE-W3C.txt), one of the suite's offered licenses.

`base.jsonld` and `examples.jsonld` are local test vocabulary definitions,
covering these credential and proof fields. They are not downloaded snapshots
of the official contexts. The service's external W3C runner prepares official
context documents separately before starting the service.

`reference-credential.json` is a local public credential signed independently
with Python's `cryptography` Ed25519 implementation and `hashlib` SHA-256 over
explicit canonical N-Quads, proof-configuration hash first. Its ephemeral private
key was discarded. This fixture detects shared hash-order drift in the HTTP
verifier even when the RDF and JSON-LD paths use the same cryptosuite helpers.

`presentations.json` preserves presentation inputs from the same pinned suite.
Handler tests independently sign the forms exercised by `TestEndpoints.verifyVp`,
including authentication challenge/domain options and embedded credentials.
`previous-proof.json` preserves the fixed chain request from the Data Integrity
suite report dated 2026-11-13, at suite revision
`1e86cc4ca16d77eb76cd15e3905a624bc16c0879`. Its signatures are public test data.

`contexts/` contains the staged official context bytes used by that run, with
URL mappings, npm/GitHub provenance and SHA-256 hashes in `contexts/manifest.json`.
They come from the packages and source listed in that manifest. The package
license notices are retained in `contexts/LICENSE-*`; credential, DID and examples
context data use the W3C Software and Document License. New suite replays use these contexts; older
credential tests retain the local vocabulary fixtures described above.

The credentials-v2 snapshot now uses the exact GitHub document from
`w3c/vc-data-model/main/contexts/credentials/v2`, recorded in the manifest.
Its JSON value equals the earlier npm `@digitalbazaar/credentials-context@3.2.0`
copy; formatting differs, so resource digests differ. The GitHub bytes match the
pinned suite's positive related-resource digests. Tests check manifest hashes and
verify credential/presentation signatures with independently reformatted contexts.
The fixed previous-proof vector contains a nonstandard inline context-document
entry. Strict JSON-LD 1.1 processing rejects it. The endpoint applies the general
jsonld.js 9.0.0 context-document rule on issuance and verification; independent
credential fixtures exercise that same rule without matching the suite vector.
