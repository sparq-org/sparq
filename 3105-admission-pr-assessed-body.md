> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

## Summary

Reuse reached non-bind RHS scans across capped seed blocks when the prepared pattern and requested sort are unchanged. A private 4 MiB allowance per capped BGP invocation bounds retained cache storage, including indexed slots, row/variable buffers and owned variable-name capacities. Scans that do not fit retain their original per-step lifetime. Stale entries are released and refunded before replacement scans; actual sort metadata stays unchanged.

This reduces repeated scan work for ASK and LIMIT queries that continue into later blocks. Armed resource budgets keep the original scan lifetime, and an armed ZK recorder keeps the existing uncapped execution path. Per-block planning, bind decisions and hash-table builds remain unchanged. No public API, dependency or storage-format changes.

Refs #3105 (`sq-7d3dj.30.18`). Planner and hash-build reuse remain open.

## Validation and review

Implementation: actual GPT-6 Astra, extra-high reasoning. Independent review of the bounded correction by actual Claude Opus 5, extra-high reasoning: **approved for CI validation** at `ed66ef0931fa19dd521fac433870c86a78687a30`; the unbounded-retention finding is resolved. Merge admission remains pending full current-head CI and assessment of Copilot’s new uncached-RHS lifetime finding.

At `ed66ef0931fa19dd521fac433870c86a78687a30`, ten focused tests pass in each of the default, core compact-index and semijoin-bitmap configurations; engine library/test clippy passes. Tests cover full result bags, join paths, requested/actual sort, named views, overlays, allocation capacities, exact-fit/oversized entries, spilled rows, stale refunds, metadata overflow and armed-but-untripped cancellation/deadline budgets. Three compiled controls fail their intended runtime assertions for excessive retention, length-based undercounting and missing refunds.

All 24 allocation samples and 40 System-allocator timing samples pass their declared local screens. These are advisory measurements on one macOS arm64 host. Full-miss timing improves while first-hit timing has a small cost; this is not a universal latency or memory-neutrality claim. The larger-pattern cases retain some additional memory within the cache allowance. System-allocator timing of the same five/eight-pattern allocation fixtures also improves. I accept this bounded memory-for-latency tradeoff based on the measured cases; broader workloads remain covered by the repository gates.

The allowance bounds requested retained storage per capped BGP invocation, not total query heap, transient scan/join allocations, allocator metadata or process RSS. Aggregate cache storage scales with concurrent or nested invocations. Local preflight still encounters Bash 3's missing `mapfile` in the existing privacy-check script; Linux CI must execute that check.

`bench/feature-off-declarations/6477.json` declares intentional changes to always-compiled engine code under the documented V2 process. It does not claim byte neutrality. The separate wasm size ratchet and all other floors remain unchanged.

## Base gate

- [ ] Authoritative workspace build and full workspace clippy on the current head.
- [x] Touched code matches surrounding formatting; no workspace reformat.
- [ ] Full affected-crate tests from current-head CI (scoped local tests passed).

## Targeted re-evaluation

- [ ] Query execution: conformance ratchet, operator coverage and builtin error cases.
- [ ] Wasm builds/tests, dependency guard and bundle-size gate.
- [ ] Coverage ratchet and test-presence gate.
- [x] Bounded retention and larger-pattern allocation checks; local timing tradeoff assessed.

## Ratchets and conventions

- [x] No conformance, performance or coverage ratchet lowered.
- [x] No hard-coded benchmark results added to repository Markdown.
- [x] Remaining planner/hash-build work stays tracked in #3105.
