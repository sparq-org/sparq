<!-- sq-4kr5: internal-stub README for a publish=false crate. -->
# sparq-conformance

The **W3C conformance harness** for [sparq](../../README.md): it runs the official test suites against
the engine and reports a per-suite pass/fail/skip scoreboard, gated by pass-count ratchets (the W3C
SPARQL one per PR in `ci-fast.yml`, the rest nightly / on dispatch in `ci.yml`). Three binaries share
manifest-walking / result-comparison machinery: `sparq-conformance` (W3C SPARQL query/update/syntax),
`sparq-inference-conformance` (RDF Semantics, OWL 2 RL, N3, entailment regimes via `sparq-reason`), and
`sparq-conformance-scoreboard` (a consolidated index of every ratchet — SPARQL, RDF syntax, inference,
RIF, SHACL, GeoSPARQL, Solid WAC + ACP, JSON-LD 1.1, SolidLab ODRL — plus eleven `sparq extension` rows,
HONESTLY labelled NOT standards claims and tallied separately). Plus: `served-conformance-report`
(served-surface SPARQL Protocol JSON report) and `sparq-notation3tests` (the ADVISORY community
notation3tests runner, `notation3tests.yml`). Floors are MEASURED and guarded textually; the
`scoreboard` rustdoc has the full per-lane provenance and divergence sets.

The registry also has a **machine-readable export** (sq-gum8.14): `scoreboard::scoreboard_json()`
renders the same rows + floors as deterministic JSON, committed as
`bench/conformance-scoreboard.generated.json` and drift-guarded by `tests/scoreboard_export.rs` — so
paper-evidence bindings can reference suite rows / floors by json-pointer without the mirror silently
drifting. Several crate-local `cargo test` lanes sit behind **opt-in features** (OFF by default) —
`jsonld-suite`, `service`, `http-protocol`, `federation-descriptors`, and the inference/geo/syntax
lanes; the `scoreboard` rustdoc documents each lane's scope, floor and divergences. The [grouped-MIN fixture discrepancy](../../docs/upstream-proposals.md#issue-5--sparql11aggregates-agg-min-02-expected-min-changes-the-selected-term) requires pinned source bytes and an exact single-cell mismatch; other differences still fail strict comparison.

> **Internal dev-only harness — not published** (`publish = false`). Test data is
> fetched by `scripts/fetch-conformance.sh` + the sibling `fetch-inference-suites.sh`
> / `fetch-jsonld*` / `fetch-odrl-suite.sh` scripts. Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
