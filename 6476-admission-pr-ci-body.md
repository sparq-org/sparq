> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Summary

A public query invoked inside an extension callback could clear its caller’s active QueryBudget, allowing the outer query to miss later cancellation and limit checks. A private scoped restorer now gives nested evaluation its own budget and restores the exact outer limits and sticky error on return, error, or unwind.

Migrate all 14 production entry scopes while preserving guard order, narrow raw budget snapshots to their audited synchronous executor consumers, and document when the outer budget resumes. Refs #6476. The separate pre-existing BASE-IRI context issue is tracked in #6479.

## Validation and review

Implementation: actual GPT-6 Astra with extra-high reasoning. Actual Claude Opus 5 with extra-high reasoning approved source `bcba08207b03714a25ad7a2a4774370c7f6f6bfa` for CI with no blocking findings, then approved the two-file cleanup and V2 declaration at `d07ca79f89e3a8945be46b516c3cd2f770ccd618`. Full required CI and protected merge validation remain pending.

Actual in-tree `sparq-engine --lib` nested-budget tests pass under default features (6), no default engine features (6), and `service-local` (7), including the final idle-state assertion. The no-default run retains the engine's configured dev-core parallel/mmap/dict-spill features. Earlier focused evidence includes the fresh production-library four-case public regression, 15 exact budget-module tests, three compiled restoration mutants that fail the intended assertions, and calibrated internal visibility controls. The public baseline provides end-to-end sensitivity; the copied-module mutants test private state restoration.

A task-private observer confirmed two actual Rayon workers on one production snapshot path with complete results. The other three joined consumers were source-audited. This is correctness evidence; no timing, throughput or wasm-size result is claimed. Direct clippy checks passed, while authoritative workspace/all-target clippy and remaining feature combinations are left to CI.

`bench/feature-off-declarations/6478.json` is an independently reviewed V2 intent declaration for the always-compiled engine change. The separate wasm-size and other ratchets remain unchanged. Local privacy preflight encounters the existing Bash 3 `mapfile` limitation; the supported CI environment must execute it.

## Base gate and targeted re-evaluation

- [ ] Authoritative workspace build/tests and full workspace clippy.
- [x] Touched-region formatting and scoped in-tree tests.
- [x] Reviewed per-PR V2 declaration; no byte-neutrality assertion.
- [ ] Supported feature matrix, including remote HTTP SERVICE; conformance and builtin error cases.
- [ ] Coverage/test-presence, wasm/dependency/size checks, and required `gate`.
- [ ] Operator-coverage suite: ordinary PR benchmarks stop after deterministic metrics, before this suite; separate execution remains pending.
- [ ] All review threads resolved and protected merge-group validation.

No conformance, performance, coverage or wasm floor is lowered. The registry remains paused and releases remain unauthorized.
