## Summary

A public query called inside an extension callback could clear its caller’s active QueryBudget on return, allowing the outer query to miss a later cancellation check. Nested evaluation now uses its own budget and restores the exact outer limits and sticky error for subsequent outer polling, including error and unwind paths.

Seal the installer behind a private scoped restorer, migrate all 14 production call sites without changing their guard order, and narrow raw snapshot access to the synchronous executor consumers. Clarify nested-budget and SERVICE savepoint behavior. No public API, dependency, or global budget policy change.

Closes #6476.

## Validation

- Fresh-engine public callback regression passes all four cases; 15 focused actual-budget-module tests pass.
- Executed Limits/sticky-error/byte-restoration mutants fail; internal sibling visibility negative and positive compile controls calibrate the sealed boundary.
- An instrumented production snapshot path executed on two workers with a 60,000-row oracle. This is a local path witness, not a performance measurement.
- Local production feature builds and direct clippy-driver checks passed. Full Cargo/workspace feature unification, conformance, coverage, wasm and deterministic perf gates remain pending normal CI. The local Bash 3 preflight cannot execute its privacy mapfile step.

## Required CI and intent declaration

- [ ] Workspace build/test shards and full workspace clippy; existing engine optional-feature tests, especially result-cache, params, explain-json and SERVICE variants.
- [ ] Conformance, coverage/test-presence, MSRV, wasm and normal required aggregate gate.
- [ ] Reviewed per-PR V2 feature-off intent declaration with the assigned PR number; separate wasm size ratchet unchanged. No byte-neutrality or size-pass claim.
- [ ] Verify operator-coverage obligations separately: ordinary PR benchmarks use the deterministic-only route.

No conformance, performance, coverage or wasm floor is lowered. Existing holds and registry pause remain unchanged.
