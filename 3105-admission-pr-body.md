> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Summary

Reuse a reached, non-bind RHS scan across capped seed blocks when the prepared pattern and requested sort are unchanged. This avoids rescanning the same relation for ASK and LIMIT queries that continue into later blocks. The cache preserves the actual sort metadata and releases a stale entry before scanning its replacement.

Queries with armed resource budgets retain their previous scan lifetime. Per-block planning, bind decisions and hash-table builds remain unchanged. No public API, dependencies or storage format changes.

Refs #3105 (`sq-7d3dj.30.18`). This addresses scan reuse; planner and hash-build reuse remain open.

## Validation and independent review

Implementation: actual GPT-6 Astra, extra-high reasoning. Independent source and soundness review: actual Claude Opus 5, extra-high reasoning, on `19763bfab1dce196a654c899b172e7b24d70bc59`; outcome **approved for CI validation**, with merge admission pending required checks and remaining resource validation.

Seven focused tests pass in each of the default, engine-no-default-features, core compact-index and semijoin-bitmap configurations. They cover public ASK/LIMIT results, duplicate multiplicities, changing requested sort, mixed join paths, disconnected joins, named views, overlays and armed budgets. The ASK suite reports 21 passed and one existing ignored timing probe. Engine library/test clippy passes. Two compiled controls fail the intended sort-invalidation and stale-slot-lifetime assertions; the sort control is a work/path check, not a demonstrated wrong-answer case.

Local preflight encounters Bash 3's missing `mapfile` in the existing privacy-claims script. Linux CI must execute that check. Scoped tests do not replace authoritative workspace, conformance, coverage, wasm or performance gates.

## Resource limits and remaining checks

The cache retains one current scan per reached non-bind pattern for the capped call. This trades repeated scans for retained memory. A larger-pattern allocation check and ZK-recorder boundary validation are in progress. Existing `try_capped` code declines the optimization while a ZK recorder is armed.

Local advisory timing/allocation evidence was measured on parent `6e86f1d0ba447aa78a50800337a7a379702fa999`, not this final head. No canonical speedup, final-head timing or measured peak reduction is claimed. Required ratchets remain unchanged.

## Base gate

- [ ] Authoritative workspace build and full workspace clippy.
- [x] Touched code matches the surrounding formatting; no workspace reformat.
- [ ] Full affected-crate test coverage from CI (scoped local tests passed).

## Targeted re-evaluation

- [ ] Query execution: conformance ratchet, operator coverage and builtin error cases.
- [ ] Wasm builds/tests, dependency guard and bundle-size gate.
- [ ] Coverage ratchet and test-presence gate.
- [ ] Remaining memory and ZK-recorder checks completed and assessed.

## Ratchets and conventions

- [x] No conformance, performance or coverage ratchet lowered.
- [x] No hard-coded benchmark results added to repository Markdown.
- [x] Remaining planner/hash-build work is tracked in #3105.
