# EARL implementation reports

Run `crates/sparq-conformance/earl-report.sh <rdf12|canon|jsonld|all> [OUT_DIR]`
from any directory. The default output is `target/earl`; suite clones live in
`OUT_DIR/suites`. The optional third argument `--fetch-only` fetches without running.
The script follows the SHACL and SPARQL EARL outcome patterns and calls
[`scripts/earl-report.py`](../../scripts/earl-report.py) with published test IRI mappings.
It requires a complete runner output before generating a report, even when a ratchet fails.

Suites default to upstream HEAD. `RDF_TESTS_PIN`, `RDF_CANON_TESTS_PIN`,
`JSONLD_TESTS_PIN`, and `JSONLD_FRAMING_TESTS_PIN` accept commit/ref overrides for
`w3c/rdf-tests`, `w3c/rdf-canon`, `w3c/json-ld-api`, and `w3c/json-ld-framing` respectively.
`revisions.tsv` records the resolved commits. `SPARQ_EARL_PROFILE` selects the Cargo
correctness-test profile (default `dev`). Reports identify the checked-out engine revision;
review uncommitted changes before submitting them.
The script restores and cleans its fixture clones on every fetch. Keep local suite
edits elsewhere. A report run removes older selected reports before fetching, so an
incomplete rerun cannot leave a stale report artifact behind.

For direct test invocations, `SPARQ_EARL_OUT` (unset by default) is an append-only
`<test IRI>\t<passed|failed>` sink. `SPARQ_RDF12_ROOT` overrides the RDF 1.2 suite
directory (default `target/earl/suites/rdf-tests/rdf/rdf12`); `SPARQ_JSONLD_ROOT` and
`SPARQ_JSONLD_FRAME_ROOT` override the API/framing `tests` directories (defaults
`tests/w3c/json-ld-api/tests` and `tests/w3c/json-ld-framing/tests`). The canonicalization
runner's `SPARQ_RDF_CANON_ROOT` is documented [below](#canonicalization).
The script sets these paths and clears its outcome files before each run.
An explicitly configured missing RDF 1.2 or JSON-LD suite fails even without EARL
output; only absent default suite locations retain the offline self-skip convention.

RDF syntax execution uses each manifest's `mf:assumedTestBase`, retains test IRIs,
and fails unreadable or unsupported entries. Withdrawn entries are omitted. RDF 1.2
syntax C14N compares serialized N-Triples/N-Quads bytes; it is distinct from RDFC-1.0.
The RDF 1.1 wrappers and floors remain in place. JSON-LD reports every entry in all
six implemented lanes, plus HTML and remote-document buckets: local skips and optional
requirements are reported as failures. Local JSON-LD ratchet pass/fail/skip accounting
is unchanged. The advisory compact strict-order diagnostic is not a separate test lane.

The internal harness exposes `turtle_suite::run_manifest(path, base)` and
`line_syntax::run_manifest(suite, path, base)`, returning `(test IRI, TestResult)`
entries. `base: Option<&str>` overrides `mf:assumedTestBase`; `None` uses its declaration.

## Canonicalization

`cargo test -p sparq-canon --test rdf_canon_suite w3c_rdf_canon_suite -- --exact --nocapture`
uses the vendored suite by default. Set `SPARQ_RDF_CANON_ROOT` to an upstream `tests`
directory to run that revision instead. Set `SPARQ_EARL_OUT` to a file path to append
one published test IRI and `passed`/`failed` verdict per manifest entry, separated by
a tab; unset means no outcome file. Fixture errors are failures even for negative tests.
The report script `crates/sparq-conformance/earl-report.sh canon [OUT_DIR]` fetches
upstream HEAD (or `RDF_CANON_TESTS_PIN`) and builds the RDFC-1.0 EARL report.
