> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

A focused follow-up to #3113: updates that leave dictionary contents unchanged currently discard the cached numeric-safety result. The next numeric FILTER can repeat a dictionary-wide check. The original first-query cold scan in #3113 remains a separate problem.

At main `f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464`, `Graph::apply_delta_mem` resolves deletions without interning, but resets a proved-safe `high_precision_decimal` memo even when dictionary length is unchanged. `Graph::has_high_precision_decimal` subsequently revisits dictionary IDs; the numeric FILTER sargability path calls this method.

An instrumented copy of the actual core module recorded:

```json
{"dictionary_ids":4100,"initial_id_visits":4100,"immediate_warm_visits":0,"existing_delete_visits":4100,"known_terms_absent_delete_visits":4100,"known_terms_reinsert_visits":4100,"empty_batch_visits":0,"dictionary_length_changed":false}
```

These are observed dictionary-ID visits/numeric-cache lookups, mostly over unrelated IRIs, not decimal-parse counts or a latency measurement. The run used the actual native Rust 2021 core test module with the default/parallel profile. Production source was unchanged.

The diagnostic test did **not** pass overall: a later control wrongly assumed integer `999999` must grow the dictionary, but it receives an inline ID. Execution stopped there without retry. The later exact/inexact decimal and sticky-state controls were not reached; the existing insertion regression was compiled but filtered out. Those gaps must be completed before admitting a fix.

Acceptance:

- Preserve the proved-safe memo for dictionary-preserving updates without changing numeric classification or FILTER semantics. Establish the invariant from actual dictionary mutation paths, rather than assuming length equality alone is sufficient.
- Invalidate when new terms could introduce an inexact decimal; retain the existing sticky unsafe state and empty-batch behavior.
- Complete the inline-integer, exact/inexact decimal, known-term and sticky-state controls, including the existing `has_high_precision_decimal_memo_survives_delta_insert` regression. A compiled control restoring unconditional invalidation must fail the work-count regression; skipping required new-term invalidation must fail the safety control.
- Validate the actual query caller and report any latency/allocation benefit with explicit scope. Obtain independent review and ordinary protected CI before merge.

No matching open memo-invalidation issue or PR was found in bounded searches. #4354 changes an opt-in numeric-cache representation in a separate builder/test hunk and remains held; #4097 changes substrate numeric promotion. Revalidate these overlaps before editing production code. This follow-up does not resolve #3113's initial cold scan.

<!-- sparq-direct-numeric-memo-preservation-v1 -->
