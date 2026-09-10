> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Summary

A public query invoked inside an extension callback could clear its caller’s active QueryBudget, allowing the outer query to miss later cancellation and limit checks. A private scoped restorer now gives nested evaluation its own budget and restores the exact outer limits and sticky error on return, error, or unwind.

Migrate all 14 production entry scopes while preserving guard order, narrow raw budget snapshots to their audited synchronous executor consumers, and document when the outer budget resumes. Refs #6476. The separate pre-existing BASE-IRI context issue is tracked in #6479.

## Validation and review

Implementation: actual GPT-6 Astra with extra-high reasoning. Actual Claude Opus 5 with extra-high reasoning approved source `bcba08207b03714a25ad7a2a4774370c7f6f6bfa` for CI with no blocking findings, then approved the two-file cleanup and V2 declaration at `d07ca79f89e3a8945be46b516c3cd2f770ccd618`. The subsequent test/documentation delta at `3c637b228449e9345d8bba31ea2d7d449f8a2b8f` was independently approved by actual Opus 5. Full required CI at exact head `3c637b228449e9345d8bba31ea2d7d449f8a2b8f` has passed, with actual build/test/feature/conformance/coverage/wasm/benchmark jobs verified. The initial changed-file service failure was recovered through one normal rerun after GitHub’s diff endpoint became available; no gate was relaxed. Protected merge-group validation remains pending.

Actual in-tree `sparq-engine --lib` nested-budget tests pass under default features (6), no default engine features (6), and `service-local` (7), including the final idle-state assertion. The no-default run retains the engine's configured dev-core parallel/mmap/dict-spill features. The final test/docs follow-up repeats the default and no-default suites, adding a fifth public callback case: valid graph-form parsing followed by the exact unsupported-query error inside the child budget scope, complete parent-state restoration, and outer cancellation. Parse rejection remains a separate control. The query skill and README now match the nested restoration contract. Earlier focused evidence includes the fresh production-library four-case public regression, 15 exact budget-module tests, three compiled restoration mutants that fail the intended assertions, and calibrated internal visibility controls. The public baseline provides end-to-end sensitivity; the copied-module mutants test private state restoration.

A task-private observer confirmed two actual Rayon workers on one production snapshot path with complete results. The other three joined consumers were source-audited. This is correctness evidence; no timing, throughput or wasm-size result is claimed. Direct clippy checks passed, and authoritative workspace/default/all-features clippy and rustdoc, supported feature combinations and ratchets subsequently passed in CI.

`bench/feature-off-declarations/6478.json` is an independently reviewed V2 intent declaration for the always-compiled engine change. The separate wasm-size and other ratchets remain unchanged. Local privacy preflight encounters the existing Bash 3 `mapfile` limitation; the supported CI validation passed without changing that local limitation.

The unchanged registered operator corpus completed against d07 production code with the repository generator: 28 queries in count/materialize/JSON modes, three iterations each, 84 complete records and 252 successful invocations. A private invalid-query control failed without a completion marker. This is equivalent public-API corpus coverage, not the complete CLI workflow, timing/allocation measurement, or an independent answer oracle. The final delta changes only tests/docs, so this production evidence still applies. The local build monitor lost its Cargo exit receipt after a disappearing temporary file; fresh compilation completion, binary provenance and all corpus run exits were verified separately.

## Base gate and targeted re-evaluation

- [x] Exact-head authoritative workspace build/tests, clippy and rustdoc.
- [x] Touched-region formatting and scoped in-tree tests.
- [x] Reviewed per-PR V2 declaration; no byte-neutrality assertion.
- [x] Supported feature matrix, including remote HTTP SERVICE; conformance and builtin error cases.
- [x] Coverage/test-presence, wasm/dependency/size checks, and required `gate`.
- [x] Registered operator corpus via equivalent public APIs; scope and control described above.
- [x] All review findings addressed and threads resolved; the optional additional cap/callback combination has a source-backed disposition.
- [ ] Protected merge-group validation.

No conformance, performance, coverage or wasm floor is lowered. The registry remains paused and releases remain unauthorized.
