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
