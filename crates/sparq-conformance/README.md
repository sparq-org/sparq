<!-- sq-4kr5: internal-stub README for a publish=false crate. -->
# sparq-conformance

Internal W3C conformance harness for [sparq](../../README.md), not published.
`sparq-conformance` runs SPARQL; `sparq-inference-conformance` runs inference; `sparq-conformance-scoreboard` indexes all ratchets. Other binaries cover served SPARQL Protocol and advisory notation3tests.
The `scoreboard` rustdoc documents scope, floors and divergences; `bench/conformance-scoreboard.generated.json` mirrors the registry and `tests/scoreboard_export.rs` guards drift.
Opt-in lanes include `jsonld-suite`, `service`, `http-protocol`, `federation-descriptors` and inference/geo/syntax features. Fixtures use `scripts/fetch-conformance.sh` and siblings.
Contributing: [`AGENTS.md`](../../AGENTS.md); [grouped-MIN fixture discrepancy](../../docs/upstream-proposals.md#issue-5--sparql11aggregates-agg-min-02-expected-min-changes-the-selected-term).

## EARL reports

Run `crates/sparq-conformance/earl-report.sh <rdf12|canon|jsonld|all> [OUT_DIR] [--fetch-only]`.
Output defaults to `target/earl`, clones to `OUT_DIR/suites`. Fetching restores and cleans script-owned clones. Reports require complete outcomes and invalidate stale artifacts.
`RDF_TESTS_PIN`, `RDF_CANON_TESTS_PIN`, `JSONLD_TESTS_PIN`, `JSONLD_FRAMING_TESTS_PIN` override upstream HEAD for rdf-tests, rdf-canon, json-ld-api, json-ld-framing; `revisions.tsv` records commits.
`SPARQ_EARL_PROFILE` selects the Cargo correctness-test profile (default `dev`). Direct tests accept `SPARQ_EARL_OUT` (unset: no append-only IRI/verdict file).
`SPARQ_RDF12_ROOT` has no default (unset skips its ratchet); `SPARQ_RDF_CANON_ROOT` defaults to vendored tests; `SPARQ_JSONLD_ROOT` / `SPARQ_JSONLD_FRAME_ROOT` default to `tests/w3c/json-ld-api/tests` / `tests/w3c/json-ld-framing/tests`.
Explicit missing roots fail. Absent default JSON-LD fixtures self-skip offline; the vendored RDFC fixtures are required.

The JSON-LD EARL report measures JSON-LD 1.1 API operations via `sparq-jsonld` (expand, compact, flatten, frame, toRdf, fromRdf).
`sparq-core`'s Graph JSON-LD ingest uses oxjsonld and is not what this report measures. The API report passes 1295 / 1296 applicable entries; its toRdf subset passes 448 / 448.
The local API toRdf lane includes legacy 1.0-only entries: 454 pass, 11 fail, two skip. A separate non-EARL `TOTAL toRdf-ingest 429 38 0` line runs all 467 toRdf entries through `parse_jsonld_dataset` (Graph acceptance plus oxjsonld canonical dataset, base only, rejection-only negatives), with floor 429 and failure ceiling 38.
Unsupported options and remote loading are not emulated; no ingest entries are skipped. The ingest ratchet never contributes EARL assertions.

Withdrawn RDF entries are omitted. JSON-LD entries with `option.specVersion` or `option.processingMode` equal to `json-ld-1.0` are omitted as not applicable to the 1.1 processor; all other unexecuted entries fail EARL.
HTML and remote documents run through the native processor and virtual manifest responses; unreadable fixtures fail. Syntax C14N compares shipping writer bytes; Eval compares the same output by dataset isomorphism.

**License:** [MIT](../../LICENSE).
