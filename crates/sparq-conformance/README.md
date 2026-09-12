<!-- [OPUS-4.8] sq-4kr5: internal-stub README for a publish=false crate. -->
# sparq-conformance

The **W3C conformance harness** for [sparq](../../README.md): it runs the official
test suites against the engine and reports a per-suite pass/fail/skip scoreboard,
gated in CI by pass-count ratchets. Three binaries share manifest-walking /
result-comparison machinery: `sparq-conformance` (W3C SPARQL query/update/syntax),
`sparq-inference-conformance` (RDF Semantics, OWL 2 RL, N3, entailment regimes via
`sparq-reason`), and `sparq-conformance-scoreboard` (a consolidated index of every
ratchet, with `sparq extension` rows labelled and tallied separately).
The `scoreboard` rustdoc records measured floors, provenance and divergences.
The registry also has a **machine-readable export** (sq-gum8.14):
`scoreboard::scoreboard_json()` renders the same rows + floors as deterministic JSON,
committed as `bench/conformance-scoreboard.generated.json` and drift-guarded by
`tests/scoreboard_export.rs` — so paper-evidence bindings can reference suite rows /
floors by json-pointer without the mirror silently drifting. Several crate-local
`cargo test` lanes sit behind **opt-in features** (OFF by default) — `jsonld-suite`,
`service`, `http-protocol`, `federation-descriptors`, and the inference/geo/syntax
lanes; the `scoreboard` rustdoc documents each lane's scope, floor and divergences.

[GPT-6] The [grouped-MIN fixture discrepancy](../../docs/upstream-proposals.md#issue-5--sparql11aggregates-agg-min-02-expected-min-changes-the-selected-term)
requires pinned source bytes and an exact single-cell mismatch; other differences still fail strict comparison.

> **Internal dev-only harness — not published** (`publish = false`). Test data is
> fetched by `scripts/fetch-conformance.sh` + the sibling `fetch-jsonld*` /
> `fetch-odrl-suite.sh` scripts. Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
