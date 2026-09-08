# PR5983 feasibility — GPT-6 Astra

Recommend a top-k-only recovery on current main. All three bundled additions are absent, and the full PR merges textually without conflicts, but that is not evidence for admission.

The engine-only slice is `exec.rs` plus its differential tests. It does not depend on PreparedGraphApplier or transaction retry helpers. Prepared engine APIs already exist on main, so the serving wrapper is also independently separable. Preserve main’s newer bind-join optimization and dependency/version metadata.

The first concrete regression to reproduce is a repeated-variable seed: with `<urn:s> <urn:p> 1`, `SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x LIMIT 1` should be empty. The new path accepts this shape and omits positional equality; source tracing predicts `urn:s`. This was not executed. Existing generic scans and other fast paths already have guards for the same constraint.

Before tuning, establish query-budget behavior and multi-valued fanout/ordering tests. The new loops contain no budget checks and can allocate large intermediate results before a small LIMIT. The current tests do not pin those paths. The scan-wide inline check and subject-id vectors also retain linear setup work, so the shortcut is not an established O(k) query and does not have zero fallback overhead.

Historical timings, mutation results and whole-workspace claims remain unverified. The diff supplies tests but no reproducible benchmark or raw measurements. Compare a repaired candidate against itself with the indexed path disabled, then validate against current main’s faster fallback, using generated results and honest host/toolchain provenance.

Keep historical Luke Dary / Claude Sonnet5 attribution. No code edits, builds, benchmarks, tests, remote mutation, review or admission were performed. See `feasibility.json` for exact refs, file scopes, source lines, confidence and validation requirements.
