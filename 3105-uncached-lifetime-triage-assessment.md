[GPT-6 Astra] The suppressed finding is refuted by the exact source scope.

At reviewed ed66, `uncached` is declared at exec.rs:4254 within the non-bind `else`. That scope closes at 4290; `record_pattern_ndv` is the next statement, at 4291. The per-step scratch and spare slot are destroyed before NDV executes. The borrowed join returns an owned `Bindings`, so it does not extend the scratch owner. Main also calls NDV after the non-bind scope, using consuming join wrappers.

No runtime repair is justified for the claimed overlap. Retained cache entries when reuse is enabled are intentional and distinct. No new runtime/heap measurement, build, source edit or remote request was performed. This source conclusion does not assert identical physical allocator/RSS behavior and does not clear review/CI holds.

See report.json for exact refs, lines, source hashes and limitations; head-scopes.txt/main-scopes.txt contain the relevant committed source excerpts.
