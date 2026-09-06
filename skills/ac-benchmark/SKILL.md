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
`expected_record_count` supply identities, eight query families, and a policy-neutral
record-count oracle. `can_read` models content-resource read rights; it is not a
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
