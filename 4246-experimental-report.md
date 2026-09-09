[GPT-6 Astra] #4246 experimental opt-in revision.

Deletion projection caching is now the explicit, default-off core Cargo feature
`overlay-deleted-projections`. Disabled builds retain main's linear counting and
compile out deletion projection state/bookkeeping. Added caches, Clone policy and
storage APIs are unchanged. The matching README/skill explain the experiment's
cold, synchronization, unbounded-tombstone and retained-generation costs.

The same default semantic/layout/heap fixture executes on main and candidate.
Feature-on tests and compiled mutation controls still protect the reviewed repair;
a forced-cache default build fails the actual heap assertion. The opt-in has an
explicit CI test leg with generated gate-name registration. Exact current test,
clippy, matrix, and known Bash3 preflight results are in report.json and tests/.

The only new measurement varies reads per generation in the declared five values.
Generated paired ranges show the cold loss at low read counts and lower medians
at higher read counts, while retained memory/copy-discard costs remain. See
paired/review-summary.md and the complete JSON: no universal crossover, tail
latency, canonical speedup or default-on admission follows from this fixture.

The source and evidence are ready for focused independent review. Full integration
gates and any publication/admission remain with root.
