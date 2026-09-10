# Final-head allocation and recorder validation

This directory is frozen local diagnostic evidence authored by GPT-6 Astra. Read report.json and generated summary.json for results and limitations. Prior source/coverage/performance bundles remain unchanged. This phase produced no commit.

Reproduction inputs: protocol.json, the identical baseline/candidate src/main.rs files, byte-identical prior counting.rs, path-specific Cargo manifests and locks, exact production exec.rs/zk.rs snapshots, provenance.json and all command receipts. Build with the recorded offline/locked release commands; run measure.py only after both binaries are available. The recorded order is balanced AB/BA/AB per case. No simultaneous build and sampling occurred.

The two temporary tests are in probe.rs. Insert them before capped_rhs_tests::observe in final exec.rs, run the commands recorded in path-and-zk.json, existing-zk-unit.json and existing-zk-integration.json, then restore the original bytes. probe.diff is the complete temporary insertion; source-restored.json records the completed restoration. Do not commit this temporary instrumentation.

The identical zk-baseline/zk-candidate harnesses compare full drained witness bytes using unmodified production refs. See the paired build/run receipts and zk-comparison.json. The source guard and ordinary existing tests are also supplied; no redundant gate was added.

Requested query heap is distinct from cumulative process RSS. Allocation-counted elapsed fields are retained as raw outputs but support no timing claim. Local screening is not a canonical benchmark or admission authorization.
