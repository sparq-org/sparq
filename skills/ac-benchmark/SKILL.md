---
name: ac-benchmark
description: Generate deterministic access-control benchmark populations and policy-neutral WAC/ACP corpora with the internal sparq-acbench crate.
---

# Access-control benchmark generation

<!-- [GPT-6] -->

`sparq-acbench` is unpublished benchmark tooling and has no dependency on the query
engine or authorization engine. Its `population` module streams one populated Pod
at a time, independently of generation order and hosted population size.

```rust
use sparq_acbench::population::{PopulationConfig, PolicyModel, write_pod};
let config = PopulationConfig::service_history();
let summary = write_pod(&config, 7, PolicyModel::Wac, std::io::sink())?;
# Ok::<(), std::io::Error>(())
```

Pass a buffered file or compression writer instead of `sink()` to persist a Pod.
Count only completed persisted Pods as hosted. The server benchmark packs independent
compressed frames with an offset index; the generator does not retain a population
array. `PopulationConfig::smoke()` is a controlled fixture and must be identified as
such in results. Configurations round-trip through `serde_json` and reject unknown
fields. Freeze the exact configuration before timed capacity runs.

`population::owner_webid`, `recipient_webid`, `benchmark_queries` and
`expected_record_count` supply identities, eleven query families, and a policy-neutral
record-count oracle. `write_readable_content` emits physically filtered record graphs
for comparison. All templates use explicit `GRAPH` clauses; bounded lists have
deterministic ordering. The reference verifies authorization selection when the
same SPARQL engine evaluates both sides. `can_read` models content-resource read rights; it is not a
general-purpose Solid authorization implementation or an authentication function.

WAC and ACP serialize the same intended owner/private/public/individual/group read
rights. WAC uses a native group; ACP expands its members. Resource-specific private
exceptions shadow inherited WAC ACLs and deny the inherited named ACP recipients.
Policy setup costs and group membership expansion are part of the comparison.

The central service-history corpus has partial calibration, documented in
[`corpus-calibration.json`](../../bench/ac/million/corpus-calibration.json).
Its media records exclude binary payloads; its message text is synthetic and its
body-size distribution is not calibrated. Daily activity summaries do not stand in
for complete clinical records or continuous sensor streams. Only an empirical
ratings-count marginal and a payment-rate anchor have observational support. Other
volumes, retention, correlations, packaging, and sharing frequencies are assumptions.

Changing configuration fields or generator APIs requires updating this skill and
the crate README in the same change. Preserve the separate earlier `deployment`
generator and frozen scaling-study protocol.

`PopulationConfig::service_history_entropy()` retains central record counts and
effective rights while using `literal_profile: {kind: "seeded",
message_text_bytes: 1024, short_text_bytes: 64}`. Its diverse synthetic text and
numeric values have independent Pod seeds. These byte lengths and the fixed safe
ASCII alphabet are compressibility stress assumptions, not fitted text distributions.
Compare compressed storage with the compact profile and declare which profile a
capacity claim uses. Omitted `literal_profile` in an older config selects `compact`
and preserves the original N-Quads bytes. Changing the seeded byte lengths supports
additional storage sensitivity runs without altering record counts or access rights.

The default central constructor now emits `service-history-central-v2` with
`literal_profile: {kind: "pod_specific"}`. Its numeric values vary between Pods;
regular text remains a favorable-compression case. Existing version-one configs
without `literal_profile` deserialize as `compact`, preserving their original
N-Quads. The configuration schema remains compatible while the profile identifies
the changed data distribution.

`population::planned_record_counts` provides cheap per-Pod inventory without RDF.
The `population::mutation` module builds bounded content workload requests from the
same record emitter. `existing_record_refs(config, pod, service, offset, count)`
selects baseline records by `(created timestamp, numeric sequence)`, including the
fixture's 28-day cycles. Its finite cursor is independent of earlier deletions;
log exhaustion instead of recycling deleted records as successful expiry work.
Contacts are a snapshot and should have zero expiry under the central workload.

`emit_mutation_batch` accepts `PopulationMutationRequest::Insert { batch_id, count }`,
`Delete { offset, count }`, or `Modify { offset, count, revision }`. It returns the
SPARQL request, addressed records, inserted N-Quads for a stateful content oracle,
and **expected** inserted/deleted triple counts. Actual changes must be measured at
the server. Ingestion uses the latest populated graph's existing permissions and
January 2026 virtual dates. Batch IDs reserve disjoint ranges of 1024 sequences and
must remain unique per Pod/service, including resumed runs. Expiry deletes current
outgoing triples, including edited values. Modification replaces the current value
with `10000 + revision`; positive revisions and explicit record-existence patterns
avoid baseline no-ops and orphan inserts. Replaying a revision is idempotent.

Each helper call accepts at most 1024 records and bounds each generated text field
to 16 MiB. Use smaller batches (for example eight records) to respect the HTTP body
limit and the declared literal profile. The helper does not infer mutation rates,
create new collections, change policies, clean incoming references, or validate
empirical user behavior. Its empty SPARQL result means finite inventory exhaustion
and must not be submitted as an accepted mutation.

The disposable cloud launcher is `bench/ac/million/launch-ec2.sh pilot|canonical`.
Canonical runs read the committed `campaign-20260906.json`; the launcher requires a
frozen status, validates the campaign before creating resources, and rejects host
type or volume overrides that differ from that file. Set `SPARQ_POD_PRIOR_AWS_USD`
to the accumulated conservative study spend. The shared benchmark slice and each
server's cgroup must account for charged page cache as well as application memory.
Raw request records and Pod inventories are retained with lossless compression.

For a bounded comparison with the engine's existing indexed storage, build the
`sparq-lws-core` example `indexed_population_preview`. Its `--output-dir` must be
new; `--profile smoke|history|entropy`, `--model wac|acp`, `--pods` (1–16, default8)
and `--max-pod-bytes` (default512MiB) define the diagnostic. It retains validation
and compares exact authorized results through in-memory, raw indexed, and compressed
indexed graphs. See [`indexed-preview.md`](../../bench/ac/million/indexed-preview.md)
for phase boundaries, physical storage accounting, descriptor estimates, and the
uncontrolled-cache limitation. This example is not a million-Pod or HTTP capacity run.
