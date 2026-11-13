<!-- sq-4kr5: internal-stub README for a publish=false crate. -->
# sparq-conformance

Internal W3C conformance harness for [sparq](../../README.md), not published.
`sparq-conformance` runs SPARQL; `sparq-inference-conformance` runs inference;
`sparq-conformance-scoreboard` indexes all ratchets. Other binaries cover served
SPARQL Protocol and advisory notation3tests. The `scoreboard` rustdoc documents scope, floors and divergences; `bench/conformance-scoreboard.generated.json` mirrors the registry
and `tests/scoreboard_export.rs` guards drift. Opt-in lanes include `jsonld-suite`,
`service`, `http-protocol`, `federation-descriptors` and inference/geo/syntax features.
Fixtures use `scripts/fetch-conformance.sh` and siblings. Contributing: [`AGENTS.md`](../../AGENTS.md); [grouped-MIN fixture discrepancy](../../docs/upstream-proposals.md#issue-5--sparql11aggregates-agg-min-02-expected-min-changes-the-selected-term).

## EARL reports

Run `crates/sparq-conformance/earl-report.sh <rdf12|rdfxml|semantics|rdf11|n3|canon|jsonld|all> [OUT_DIR] [--fetch-only]`.
Output defaults to `target/earl`, clones to `OUT_DIR/suites`. Fetching restores and cleans
script-owned clones. Reports require complete outcomes and invalidate stale artifacts.
An outcome outside the selected report prefixes aborts generation rather than being omitted.
`RDF_TESTS_PIN`, `N3_TESTS_PIN`, `RDF_CANON_TESTS_PIN`, `JSONLD_TESTS_PIN`,
`JSONLD_FRAMING_TESTS_PIN` override upstream HEAD for rdf-tests, N3, rdf-canon, json-ld-api
and json-ld-framing; `revisions.tsv` records the resolved commits.
`SPARQ_EARL_PROFILE` selects the Cargo correctness-test profile (default `dev`). Direct tests accept
`SPARQ_EARL_OUT` (unset: no append-only IRI/verdict file); `SPARQ_RDF12_ROOT` (no default;
unset skips the RDF 1.2 syntax ratchet); `SPARQ_RDF_TESTS_ROOT` (no default; rdf-tests
repository root for RDF/XML, RDF 1.2 Semantics and RDF 1.1 reports); `SPARQ_N3_ROOT`
(no default; w3c/N3 repository root); `SPARQ_RDF_CANON_ROOT` (default vendored tests);
`SPARQ_JSONLD_ROOT` / `SPARQ_JSONLD_FRAME_ROOT` (default `tests/w3c/json-ld-api/tests` /
`tests/w3c/json-ld-framing/tests`). Explicit missing roots fail. Absent default JSON-LD fixtures
self-skip offline; the vendored RDFC fixtures are required.
`rdfxml` runs RDF 1.2 RDF/XML evaluation and RDF 1.1 RDF/XML through the shipping
`Graph::load_str_with_base(..., "rdfxml", ...)` parser; `semantics` runs the RDF 1.2
Semantics manifest through the existing entailment checker; `rdf11` reports all six
RDF 1.1 suites; `n3` reports the N3 parser, extended, reasoner and Turtle manifests.
`all` includes every subcommand. The script enables the default-OFF `rdfxml` Cargo
feature for the RDF/XML lanes. Direct `earl_suites` tests without it fail every XML
entry, including negative syntax, because no XML parser ran. Other new lanes run in
both feature states. Without explicit roots these fetched-suite tests self-skip;
an EARL request or an invalid explicit root fails instead. RDF 1.1 IDs use the current
published `.../rdf/rdf11/<suite>/manifest.ttl#id` form; explicit manifest IRIs (including
RDF 1.2 `eval#id`) are preserved. N3 IDs use the published manifest document plus fragment,
independently of repeated display names. Existing typed-test discovery supplements N3
entries lists. Malformed or incomplete RDF entries lists abort generation.
Withdrawn RDF entries and upstream rejected/withdrawn N3 entries are omitted.
N3 local divergences and unsupported reasoner options produce failed EARL outcomes.
The central scoreboard includes the new ratchets and their required roots/features.
JSON-LD entries with `option.specVersion` or `option.processingMode`
equal to `json-ld-1.0` are omitted as not applicable to the 1.1 processor. Other skips, optional `requires`,
HTML, remote documents and unreadable fixtures fail. Syntax C14N compares shipping writer bytes;
Eval compares the same output by dataset isomorphism. Local JSON-LD ratchet accounting is unchanged.

**License:** [MIT](../../LICENSE).
