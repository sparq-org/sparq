# Exact temporal memo: append work and remaining measurements

[GPT-6] The graph memo stores exact seconds and checked fraction spans. Dictionary
IDs and their lexical contents remain stable during supported delta updates.
An initialized memo therefore extends under the existing exclusive graph borrow;
it does not discard old entries or add a lock to each temporal lookup. An unused
memo stays uninitialized. Forks, reloads and compression still start with empty
memos; persisted RDF terms and approximate cache formats are unchanged.

The native regression initializes a graph containing 128 distinct dates, appends
an unrelated string, then appends one date and repeatedly queries the old/new IDs.
It counts actual shared dateTime-parser entries: unrelated insertion causes zero
reparsing, and one new date is validated once by each approximate/exact cache.
Repeated inserts of existing IDs and deletes do not trigger temporal reparsing.
Dense, compressed, forked and mapped variants use the same assertions. Separate
engine queries preserve precise FILTER/COUNT/ORDER and MIN/MAX results after
insertion and deletion. These are deterministic work and semantic checks, not
latency measurements or guest-proof evidence.

The first temporal lookup on a cold mapped graph still scans all persisted
classification flags and parses every valid temporal literal. A selective cold
query can therefore page in more data than it otherwise scans. This follow-up
does not establish cold-read performance neutrality or change memo layout/heap
accounting. Per-ID lazy population needs a separate design and measured tradeoff.

Existing canonical benchmark coverage is available without inventing a new suite:

- `bench/u64-valueids/queries/q04_filter_date_dict.rq`: temporal FILTER.
- `bench/u64-valueids/queries/q09_order_date.rq`: temporal ORDER/LIMIT.
- `bench/u64-valueids/queries/q11_minmax_date_group.rq`: grouped temporal MAX.
- `bench/bsbm/queries/query07.rq` and `query10.rq`: temporal validity predicates
  inside their published query workloads.

`bench/benchmarks.toml` registers both suites; `scripts/bench/run-all-benchmarks.sh`
selects them and `scripts/ci-bench.sh` includes the BSBM subset. BSBM timings are
trend-only; its expected-result counts are checked separately. Query07's pinned
small corpus has no matching DE vendor, so it is not by itself a discriminating
temporal-work benchmark. None of these existing static query runs substitutes
for the new append/query work-count regression. Canonical-host before/after timing
and selective cold-mmap measurements remain pending; no local timing is presented
as canonical evidence.
