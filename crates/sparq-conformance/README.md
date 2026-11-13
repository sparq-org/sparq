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

`crates/sparq-conformance/earl-report.sh <rdf12|rdfxml|semantics|rdf11|n3|canon|jsonld|all> [OUT_DIR] [--fetch-only]`
fetches each suite at upstream HEAD into `OUT_DIR/suites` (default `target/earl`), records commits
in `revisions.tsv` and writes reports only from complete outcomes; an outcome outside the report
prefixes aborts. Pins: `RDF_TESTS_PIN`, `N3_TESTS_PIN`, `RDF_CANON_TESTS_PIN`, `JSONLD_TESTS_PIN`,
`JSONLD_FRAMING_TESTS_PIN`. Tests append IRI/verdict lines to `SPARQ_EARL_OUT` and read fetched
suites only from `SPARQ_RDF12_ROOT`, `SPARQ_RDF_TESTS_ROOT`, `SPARQ_N3_ROOT` (unset: self-skip;
missing explicit root or EARL run: fail); `SPARQ_RDF_CANON_ROOT` and `SPARQ_JSONLD_ROOT` /
`SPARQ_JSONLD_FRAME_ROOT` override the vendored and `tests/w3c` defaults. RDF/XML lanes need the
`rdfxml` feature (without it every XML entry fails). Skip policy: only withdrawn or upstream-rejected
entries and JSON-LD 1.0-only entries are omitted; anything not run fails.

**License:** [MIT](../../LICENSE).
