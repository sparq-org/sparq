> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Two executed regressions block adoption of the indexed top-k optimization proposed in [PR #5983](https://github.com/sparq-org/sparq/pull/5983), head `59bfc6c44b25a4e107d5d957ca3287ba0bc22c1a`. This is an unmerged candidate defect: current main `cf19a52c6880c496cefaca24046174902bc30958` retains the existing evaluator.

**Repeated-variable semantics.** For this data:

```turtle
<urn:s> <urn:p> 1 .
```

this query must return no rows:

```sparql
SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x LIMIT 1
```

Importing the PR's exact two-file top-k delta onto current main returns `?x = <urn:s>`. The paired full-ORDER-BY control passes; the LIMIT regression fails. Restoring exact main's `exec.rs` makes both tests pass. The indexed path bypasses the generic evaluator's repeated-position equality enforcement.

**Intermediate resource limit.** Using the PR's existing 40-task claim-query fixture, `query_with_budget` with `max_rows: Some(2)` and `LIMIT 1` returns a successful result through the imported path instead of the expected intermediate-row limit error. The test failed at this first assertion, so that raw run does not establish what its later byte-limit assertions would do. The new scan/probe path bypasses the existing intermediate-budget accounting.

A local Astra recovery at `803bb795201782551d984221a48b32ed391f7d9c` reuses the existing repeated-variable shape guard, declines immediately when a query budget is active, and delegates multi-valued probes to the existing evaluator. Historical Luke Dary / Sonnet5 attribution is retained; only the new repair/tests are Astra work. PreparedGraphApplier and transaction retry helpers are outside this recovery slice.

Executed with pinned Rust/Cargo 1.97.1, offline and locked, default engine features, two build jobs and one test thread:

- Raw import: paired repeated-variable tests 1 pass / 1 fail; exact-main fallback 2 pass.
- Raw intermediate-row-budget regression: 1 assertion failure.
- Repaired focused suite: 19 pass, including row/byte/generous-budget and multi-valued fallback cases; existing ORDER BY suites: 17 pass.
- Six calibrated guard/engagement controls all fail a test, with no survivors or compile/harness errors.

Acceptance before landing: complete the remaining bounded ordering/view/resource cases, retain a demonstrably engaged fast path and correct fallback, obtain real independent Opus review, and provide reproducible measurement evidence before making performance claims. No latency improvement, full-workspace validation, independent approval or merge readiness is claimed by this issue. No release, registry dispatch or EC2 action was performed.
