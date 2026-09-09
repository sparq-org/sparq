[GPT-6 Astra] Focused #4246 revision, not an admission claim.

B1 is fixed and executable controls pin it: projections survive insert-only/no-op
batches when tombstones already exist, and actual tombstone changes invalidate
before publication. Added-cache behavior and derived Clone are unchanged.

The paired lifecycle diagnostic quantifies B2 rather than assuming it away.
Warm read-only snapshots and insert/update/read generations show useful medians,
with copied projection memory and some overlapping/noisy timing ranges. A true
tombstone update still copies initialized projections, discards them, and pays a
cold sort. Ordinary multi-pattern cold reads and two concurrent cold readers are
also slower. Full generated results, ranges, phases, counts and retained heap are
in paired/summary.json and paired/summary.md; no samples were discarded. General
admission is not established.

B3 context now includes the previously omitted reviewed store span, complete final
store, complete Graph lifecycle functions and a workspace mutation inventory.
Only apply_delta mutates production tombstones in place. Compaction/clear/open/
WAL replay/restore/vacuum use that seam or replace the entire store/graph. The
durable routes were inspected, not newly crash-tested.

Forty-eight scoped test executions pass; five current compiled controls are
killed. Core and harness clippy pass. Preflight reports only the known Bash 3
mapfile failure. The fixed harness/main control have identical sources and all
four optimized builds completed within the authorized cap. Exact commands, logs,
source and binary hashes are retained.

The next step is independent review of this repair and the lifecycle evidence.
No Clone/Arc redesign, cardinality threshold, remote update, or admission action
is included. Further design requires a separate scoped decision.
