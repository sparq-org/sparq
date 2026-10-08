---
name: federated-planning
description: "Cost-based federated SPARQL source selection + bind-vs-hash join planning over already-fetched source descriptors, plus an ANAPSID-style non-blocking streaming join with operator spill, via the opt-in sparq-fedplan crate. Use when planning a federated BGP across multiple SPARQL endpoints from their served statistics (VoID property/class partitions + mined scs: characteristic sets): deciding which sources can contribute to each triple pattern (HiBISCuS recall-safe pruning + CostFed skew-aware cardinality), choosing a join order with per-join bind-vs-hash-vs-streaming algorithm selection (characteristic-set star cardinality for intermediate sizes), and executing a memory-bounded non-blocking symmetric hash join over incrementally-arriving sub-results (StreamJoin, spill to a backing store, result multiset-equal to a blocking join). Pure + deterministic planning, no network I/O. Off by default; does not touch sparq-core/sparq-engine's lean build. Also covers live adaptive RE-planning at stage boundaries (mid-execution plan switching when observed cardinalities diverge from estimates, or when a source is observed to be slow — per-source EWMA-smoothed latency is folded into the cost model as a documented heuristic bias toward faster sources, smoothing out transient spikes) via the further opt-in adaptive-replan feature (AdaptiveExecutor) — sound because BGP join is order-independent (latency changes ordering/cost only, never results), with mid-operator swap + live source failover deferred."
---

# sparq-fedplan — cost-based federated source selection + join planning

`sparq-fedplan` plans a federated SPARQL **Basic Graph Pattern** (BGP) across several
remote endpoints **from statistics already in hand** — it never contacts the network. A
caller fetches each source's descriptor once (the W3C VoID document a `sparq-server`
serves at `/.well-known/void`, including the mined `scs:` characteristic sets), and the
planner decides, deterministically:

1. **which sources can contribute** to each triple pattern, and
2. **a join order + per-join bind-vs-hash algorithm** over the selected sources.

It is the **opt-in public surface** for cost-based federation planning. Add it
explicitly and enable the `fedplan` feature; it is **not** in sparq's default build
(`sparq-core`/`sparq-engine` stay lean, the wasm artifact is unchanged unless you pull it
in). There is no `sparq-core`/`sparq-engine` dependency — it plans over descriptors only.

## Add the dependency

```toml
[dependencies]
sparq-fedplan = { path = "crates/sparq-fedplan", features = ["fedplan"] }
oxrdf = { version = "0.3", features = ["rdf-12"] }
```

The whole planner is behind the `fedplan` feature (off by default), so even a crate that
depends on `sparq-fedplan` pays nothing for it unless the feature is enabled.

## Build source descriptors

Either programmatically via the builder, or by parsing the served N-Triples document.

```rust
// `SourceDescriptorBuilder` is the (public, nameable) return type of `.builder(..)`; you only
// need to import it if you hold the builder in a `let` rather than chaining straight to `.build()`.
use sparq_fedplan::{SourceDescriptor, SourceDescriptorBuilder, SourceId, PredPartition, ClassPartition};

// Programmatic (VoID property/class partitions).
let src = SourceDescriptor::builder(SourceId::new("https://a.example/sparql"))
    .total_triples(10_000)
    .predicate(PredPartition { predicate: "http://xmlns.com/foaf/0.1/knows".into(),
        triples: 2000, distinct_subjects: 1000, distinct_objects: 1800 })
    .class(ClassPartition { class: "http://xmlns.com/foaf/0.1/Person".into(), entities: 1000 })
    .build();

// Or parse the served descriptor (the /.well-known/void N-Triples form, with scs: sets):
let parsed = SourceDescriptor::from_void_nt(SourceId::new("https://b.example/sparql"), nt)?;
```

A descriptor **parsed from VoID partitions is authority-incomplete** (it sees only
predicate/class authorities, never subject/object instance authorities), so
subject/object authority-pruning is disabled for it — recall-safe by construction. To
enable authority pruning, build via `.builder(..)` and call `.authorities_complete()`
only when you truly enumerate every authority the source mints (a HiBISCuS-style
capability set / `void:uriSpace` declaration).

A descriptor may also carry an **optional GenAI retrieval capability**
(`RetrievalCapability` — declared vector/text retrieval endpoints + a per-request
cardinality hint), served under the `sret:` vocab (`http://sparq.dev/ns/retrieval#`,
`RETRIEVAL_NS`) next to the VoID partitions and round-tripped via
`RetrievalCapability::to_void_nt` / `from_void_nt`. It defaults to absent
(`descriptor.retrieval() == None`) and is **advisory planner metadata only**:
`select_sources` consumes it as a STATIC candidate-ordering hint (sources declaring a
`cardinalityHint` order first, ascending by hint; undeclared keep index order after
them; source index tie-breaks — so with no hints the ordering is the historical
ascending-index order), but it can never change which triples answer a BGP: the
retained-source SET and every estimate are hint-independent (differential-tested; the
same answer-safe discipline as the cardinality estimators). Set it with
`.retrieval(..)` on the builder. sq-3uijg

## Select sources for a BGP (recall-safe)

```rust
use sparq_fedplan::{Bgp, TriplePattern, Term, Var, select_sources};

let bgp = Bgp::new(vec![
    TriplePattern::new(Term::Var(Var::new("s")),
        Term::Iri("http://xmlns.com/foaf/0.1/knows".into()), Term::Var(Var::new("o"))),
]);
let sources = [src];
let selection = select_sources(&bgp, &sources);
// selection[i].candidates: the sources retained for pattern i, with estimated_cardinality.
```

**Recall-safety invariant:** a source is pruned for a pattern *only when the descriptor
proves it holds no matching triple*. A bound predicate absent from the source's (complete)
predicate-partition set prunes; a bound class absent from a *declared* class section
prunes; a bound subject/object whose authority is absent prunes *only* when the authority
set is complete. On any uncertainty — open predicate, incomplete authority set, absent
class section — the source is **kept**. The cardinality estimate never prunes (a source
with a tiny or zero estimate is still retained). This is HiBISCuS's design goal: maximise
pruning subject to never losing a result.

### Optional live pattern probes in `sparq-fedclient`

`sparq-fedplan` itself remains pure and never contacts the network. When served VoID statistics
are missing, the separate federation client can refine its `PatternSources` through the
default-OFF `sparq-fedclient/pattern_probe` feature (which implies `fedclient`):

```toml
[dependencies]
sparq-fedclient = { path = "crates/sparq-fedclient", features = ["pattern_probe"] }
```

Create one `PatternProbeSession` per query, using the same SSRF-policy-controlled `Fetcher` as
capability discovery, then pass endpoint/optional-descriptor pairs in planner source-index order:

```rust
use sparq_fedclient::{
    select_sources_with_pattern_probes, PatternProbeConfig, PatternProbeSession, ProbeSource,
};

let fetcher = sparq_fedclient::discovery::HttpFetcher::new();
let mut session = PatternProbeSession::new(&fetcher, PatternProbeConfig::default());
let selection = select_sources_with_pattern_probes(
    &bgp,
    &[ProbeSource {
        endpoint: "https://example.org/sparql",
        descriptor: discovered_descriptor.as_ref(),
    }],
    &mut session,
);
```

The request budget is per query and counts every ASK/SELECT HTTP request; repeated
source-pattern pairs are cached. Served VoID cardinalities issue no probe. Recall safety is
load-bearing: only an exact ASK `false` removes a source. Timeout, HTTP/parse error,
inconsistent responses, or budget exhaustion retain it with the uniform fallback. A successful
capped SELECT row count replaces that fallback and can change ranking/join order, never the
answer multiset. sq-fx5id.

## Plan the join (bind vs hash)

```rust
use sparq_fedplan::{plan_bgp, PlanOptions, JoinAlgo, JoinNode};

let plan = plan_bgp(&bgp, &selection, &sources, &PlanOptions::default()).unwrap();
let order: Vec<usize> = plan.join_order(); // patterns in join order (left-deep)
let cost: f64 = plan.total_cost;
```

Each binary join is a **bind join** (cost ≈ `L·(req + fan_out)` — probe the right with the
left's bindings; cheap when the left is small and the right selective) or a **hash /
symmetric join** (cost ≈ `R + L` — scan both sides once; cheap when the left is large or
the right unselective). The decision flips as the left intermediate grows past the point
where per-row requests overtake a full scan; tune the round-trip penalty with
`PlanOptions::request_cost`. Star arms (`?s p₁ ?a . ?s p₂ ?b`) use characteristic-set
cardinality (`Σ_{C⊇Q} count(C)·Π avg_mult`) for intermediate sizes, capturing the
predicate correlation an independence product loses.

### Predicate-selectivity-aware non-star cardinality (opt-in, `use_predicate_selectivity`, sq-jsuzr)

By default a connected **non-star** join (e.g. a chain `?s :p ?o . ?o :q ?z`) falls back to a
coarse independence estimate `out = |L|·|R| / max(|L|, |R|)` that approximates the join-key
distinct-value count by the larger leaf cardinality. Setting
`PlanOptions::use_predicate_selectivity = true` instead folds the candidate's *actual*
per-predicate distinct counts — `distinct_subjects` when the join variable sits in the
candidate's subject position, `distinct_objects` when in its object position, summed (union)
over the candidate's retained sources — into the ndv, giving a skew-aware estimate
`out = |L|·|R| / Σ distinct(join-key)`. These are the same VoID stats every `SourceDescriptor`
already carries. When the relevant distinct count is absent (VoID leaves it 0 when unknown), or
the join key is in neither position, it falls back to the identical max-leaf estimate, so it
never fabricates an ndv. **Cost-estimate-only**: this changes the planner's cardinality (hence
its join order and bind-vs-hash choice), but never the *result multiset* — BGP join is
commutative/associative, so any order over the same patterns answers identically. The flag is
**OFF by default** so the prior estimate is preserved byte-for-byte for A/B comparison; star
arms keep using the characteristic-set estimate regardless. Whether the tighter estimate yields
faster plans is a measurable hypothesis to confirm on the canonical perf host (sq-0g6g), not a
claim made here.

## Non-blocking streaming join + spill (`StreamJoin`, sq-vf7q)

The planner's `JoinAlgo::Streaming` choice corresponds to an execution-side operator:
`StreamJoin`, an ANAPSID/XJoin-style **symmetric hash join** over two
incrementally-arriving tuple streams. Feed tuples from either side as federated sub-results
arrive; each `push` returns the results that arrival newly completes — it never blocks on
either input finishing.

```rust
use sparq_fedplan::{StreamJoin, StreamJoinOptions, SpillStore, Tuple, Var, blocking_hash_join};

let opts = StreamJoinOptions { mem_budget_tuples: 100_000, spill_store: SpillStore::TempFile };
let mut join = StreamJoin::new([Var::new("s")], opts);
let _ = join.push_left(Tuple::new([(Var::new("s"), "a".into()), (Var::new("o"), "1".into())]));
let out = join.push_right(Tuple::new([(Var::new("s"), "a".into()), (Var::new("n"), "x".into())]));
assert_eq!(out.len(), 1); // emitted the moment both sides hold key s=a — non-blocking.
```

**Bounded spill.** Memory is capped at `mem_budget_tuples`; when an insert would exceed it,
the largest in-memory join-key partition is spilled to a backing run (a temp file under
`std::env::temp_dir` by default — `std` only, no new dependency; `SpillStore::Memory` is an
in-process simulation for tests). Spilling only relocates tuples; the probe consults both
the live bucket and every spilled run for the key.

**Correctness invariant (load-bearing).** The streamed + spilled result is *multiset-equal*
to `blocking_hash_join(left, right, join_vars)` — same tuples, no loss, no duplication — for
**any** stream interleaving and **any** budget (including one so low every partition spills).
Each matching pair `(l, r)` is emitted exactly once, when the second of the two arrives and
probes the other side (found in memory or a spill run). Proven by the `streamed_equals_*`,
`spill_path_equals_*`, `duplicate_keys_*`, and `emits_before_inputs_exhausted` tests.

The planner picks `Streaming` over plain `Hash` when a hash-class join's combined estimated
inputs `L + R` exceed `PlanOptions::stream_threshold` (default 100 000 rows; set to
`f64::INFINITY` to always use plain hash) — large joins run non-blocking + spillable rather
than materialising a side up front.

## Live adaptive re-planning (opt-in `adaptive-replan` feature, sq-7s4z)

Behind the **off-by-default** `adaptive-replan` cargo feature (which implies `fedplan`), the
crate adds the reactive half of ANAPSID adaptivity: **mid-execution plan switching**. A build
that does not enable the feature compiles **zero** adaptive code (`#[cfg]`-gated out), so the
lean default build and the `fedplan`-only build are byte-unchanged.

`AdaptiveExecutor` models execution as a sequence of **stages** (the left-deep join order)
and holds, at all times, the patterns already joined (the **prefix**) and the patterns still
to join (the **suffix**).

- **Capture** — `RuntimeStats` records the *observed* per-pattern leaf cardinality (real row
  counts the sources returned) and per-source latency, fed in as each stage completes. Latency
  is **EWMA-smoothed per source** (below) so the cost model + trigger track the trend, not the
  last raw sample.
- **Trigger** — at each **stage boundary**, `maybe_replan(&stats)` checks whether a
  *not-yet-executed* pattern's observed cardinality `o` diverges from its estimate `e` past
  `ReplanPolicy::divergence_factor` `k` either way (`o > k·e` or `e > k·o`; default `k = 4`),
  **or** (sq-b51o) a not-yet-executed pattern's slowest source is observed at more than `k×`
  the latency baseline. If so it re-invokes the cost model on the **remaining** patterns with
  the observed cardinalities substituted in (`corrected_selection`) and the join costs
  latency-weighted (below). Source *membership* is never re-pruned — only the order changes —
  so HiBISCuS recall-safety is preserved.
- **Per-source latency weighting (sq-b51o) — a HEURISTIC, not optimal.** A source can be
  *slow* (contended / far / rate-limited) even at exactly its predicted cardinality, so the
  re-planner also folds observed latency into the cost. Each candidate join's cost is scaled by
  `factor = clamp(1 + latency_weight·(s − 1), latency_floor, latency_cap)` where
  `s = slowest_ewma_source_latency / latency_baseline` over the pattern's retained sources.
  A source at baseline — **or with no observation** — yields `factor = 1.0`, so a re-plan with
  no latency data is byte-identical to the cardinality-only planner; a 2×-slow source costs
  1.5× at the default `latency_weight = 0.5`, with `latency_cap` (4.0) bounding any one
  outlier. The constants (`latency_weight 0.5`, `latency_baseline 100`, `latency_floor 0.5`,
  `latency_cap 4.0`) are **hand-tuned, not derived** — a deliberately gentle *bias* toward
  faster sources / deferring a slow one, **not** a claim to compute the latency-optimal plan.
  Latency enters only the *cost* term (and the suffix-selection score), **never** the output
  cardinality, so results are unchanged. `latency_weight = 0` disables it.
- **Per-source vs slowest-arm aggregation (sq-s5kd) — opt-in, default unchanged.** How a
  multi-source pattern's per-source latencies fold into its single cost factor is now a policy
  knob, `ReplanPolicy::latency_aggregation`: `LatencyAggregation::SlowestArm` (default — the
  sq-b51o bottleneck model above, bit-identical for existing callers) or
  `LatencyAggregation::CardinalityWeighted` — each retained source's latency runs through the
  same `factor` formula individually and the pattern is charged the **cardinality-weighted
  mean** of the per-source factors (an *expected-work* model: a tiny slow arm no longer
  dominates a huge fast arm's pattern; plain mean when every retained cardinality is 0). An
  unobserved source still contributes `1.0`, so with no observations both modes are exactly
  neutral, and single-source patterns are mode-independent. The re-plan *trigger* stays
  slowest-arm under both modes (its job is to detect a pathologically slow arm; the weighted
  cost + hysteresis then decide adoption). Which model wins is workload-dependent
  (parallel-fetch unions really are bottlenecked; sequential/bind-join work is proportional) —
  the MECHANISM ships un-tuned for the federation bench harness to compare.
- **Latency EWMA smoothing (sq-b51o follow-up) — a HEURISTIC α, not optimal.** The cost factor
  and the trigger read a per-source **exponentially-weighted moving average**, not the single
  last sample: `record_source_latency` folds each sample in as `ewma = α·observed + (1−α)·prev`
  (first sample seeds it), with α = `RuntimeStats::latency_alpha`, default
  `DEFAULT_LATENCY_ALPHA = 0.3` — a **hand-picked** factor (latest 30% / history 70%), *not*
  workload-derived. This is the cleaner anti-thrash than a bare last-sample-plus-clamp: a
  **single transient spike** does not move the average over the trigger band, but a **sustained**
  shift converges past it in a few samples. The `latency_floor`/`latency_cap` clamp is kept as a
  **final guard**. `RuntimeStats::with_latency_alpha(α)` overrides (α = 1.0 ⇒ un-smoothed
  last-sample behaviour); higher α = faster-tracking/twitchier, lower α = calmer/laggier.
- **EWMA refinements (sq-3xkz) — per-source α, time-aware decay, eviction — all opt-in,
  default-off.** Three knobs sharpen the EWMA when a `RuntimeStats` is reused across queries;
  each defaults to off so the default path is byte-identical to the single-global-α EWMA above.
  - **Per-source adaptive α.** `RuntimeStats::set_source_alpha(source, α)` /
    `with_source_alpha` gives a source its own smoothing rate; `latency_alpha` is the *fallback*
    and `effective_alpha(source)` resolves the override-else-global α. An **empty** override map
    ⇒ every source uses the global α ⇒ the prior behaviour reproduced. This ships the MECHANISM
    + a sensible default; choosing a *good* per-source α needs a real federated workload, so that
    **tuning is deferred** — no α value is claimed optimal.
  - **Time-aware decay.** The plain EWMA equal-weights samples regardless of the gap between
    them; `record_source_latency_after(source, latency, elapsed)` with a half-life set
    (`set_decay_half_life` / `with_decay_half_life`) inflates the effective α toward `1.0` as the
    elapsed gap grows past the half-life (`α_eff = α₀ + (1−α₀)·(1 − 0.5^(Δt/half_life))`), so a
    fresh sample after a long idle gap is trusted more and stale history decays toward the prior.
    The elapsed gap is **passed in** (the logical clock is injectable, never read internally), so
    the decay is deterministic + testable. No half-life ⇒ folds at the plain α (back-compat).
  - **Staleness / eviction.** `evict_stale(max_age)` drops every source whose `source_age`
    (`clock − last_seen`, on the same injectable logical clock) exceeds the threshold (strictly
    greater; equal is kept), returning the count evicted — so a stale latency stops biasing a
    long-lived store. `advance_clock(elapsed)` ages entries without recording a sample.
- **Hysteresis** — the re-planned suffix is adopted **only** if its estimated remaining cost
  (cardinality- **and** latency-weighted) beats the current suffix's by more than
  `ReplanPolicy::improvement_margin` (default 10%), with a hard `max_replans` budget
  (default 8). Stable-but-noisy stats — **including jittery latency** — never thrash;
  `maybe_replan` returns `ReplanOutcome::{NoDivergence, KeptWithinHysteresis, Switched,
  BudgetExhausted}`.

**Soundness boundary (load-bearing).** Re-planning reorders only the not-yet-started
**suffix** — it is **NOT** a mid-operator swap (an in-flight join is never torn down). A BGP
answer is the natural join of the per-pattern solution multisets, which is **commutative and
associative**: any order over the same patterns yields the **same** result multiset, and the
already-produced prefix is carried across the switch unchanged (no binding lost or
duplicated). The latency weighting does **not** move this boundary — it touches only
cost/ordering, never the output cardinality or the pattern set, so a latency-driven reorder is
the same kind of pure suffix permutation; the EWMA smoothing changes only *when* the latency
path fires, never this boundary — re-planning stays a sub-query / **stage-boundary** reorder,
never mid-operator. Proven by `adaptive::tests::replan_result_equals_static`
(cardinality-driven), `latency_replan_result_equals_static` (latency-driven) and
`ewma_replan_result_equals_static` (EWMA-smoothed-latency-driven) — each genuinely flips the
order yet yields the identical multiset to the static plan — plus an exhaustive
all-permutations order-independence test.

## The federation CLIENT that consumes this planner (`sparq-fedclient`, sq-dnko)

`sparq-fedplan` is the planning *brain* with **no consumer** — it plans, but nothing
fetches descriptors or issues a query. The consumer is a separate opt-in crate,
**`sparq-fedclient`** (epic **sq-dnko** / sq-3183, architecture `research/federation-client-design.md`):
the streaming federation **client** that discovers each remote source's capability, lowers
a query BGP into this crate's `Bgp`, calls `select_sources` + `plan_bgp`, interprets the
resulting `JoinTree` into physical operators (Bind → VALUES bind-join, Hash/Streaming →
the `StreamJoin` above, Local → `sparq-engine` eval), and streams results back. It REUSES
this planner and the engine's `service` SRJ transport + SSRF guard; the dependency arrow
points one-way *into* the engine.

`sparq-fedclient` started as the Phase-0 skeleton (`sq-s1uy`) — the public module layout
(`source` / `discovery` / `planner` / `pushdown` / `operators` / `stream`) behind a
default-OFF `fedclient` feature, plus the load-bearing dependency-boundary proof
(`sparq-core`/`sparq-engine` have no edge to it, enforced by
`scripts/fedclient-boundary-guard.sh` + `crates/sparq-fedclient/tests/boundary.rs`). Landed
since:

- **Phase 1 discovery** (`sq-nfxl`) — Service-Description parser + VoID/`scs:` reuse + an
  SSRF-guarded fetch seam + ASK-probe fallback → a `Capability` (+ optional
  `SourceDescriptor` for *this* planner). The `Capability` also reads the SPARQL 1.2 SD
  `sd:supportedVersion <sparql:version-*>` advertisement (`sq-ym6kf`, consuming the server side
  `sq-2msb` emits) into `sparql_versions: Vec<SparqlLanguageVersion>`, so a 1.2-aware client can
  detect full-1.2 (triple-term / `dir`-lang) support via `Capability::advertises_full_sparql_1_2()`
  **without probing**. Honest boundary: an empty set means the source published no
  `sd:supportedVersion` ⇒ version posture **UNKNOWN** (older endpoints predate the term), not
  unsupported — distinguish the two with `Capability::advertises_sparql_versions()`.
- **Phase 2 source abstraction** (`sq-rsxf`) — the `Endpoint` adapter over the engine's
  transport seam behind a default-deny SSRF guard.
  - **Port-scoped allowlist entries** (`sq-vbnyc`, follow-up to the engine's `sq-a7jw4`):
    `source::EgressGuard`'s host allowlist now accepts a **port-scoped** entry (`127.0.0.1:8053`,
    `[::1]:8080`, `.example.org:443`) that re-opens a private host on THAT exact port only —
    strictly narrower than a bare host-level entry, which still re-opens every port (backward
    compatible). The dialled port (the authority's `:port` or the scheme default) is the port
    vetted, so a `host:port` entry never widens. The fedclient guard delegates the per-entry
    decision to the engine's shared `sparq_engine::allowlist_entry_permits` (and
    `allowlist_entry_host_matches` for the host-level "is this host on the list at all" query), so
    the **fedclient guard and the engine SERVICE guard decide every host:port case identically**
    — one source of truth, no divergent copy of the parsing (port-0/overflow/IPv6-bracket/
    trailing-colon all fail-CLOSED). The same port-scoping flows through `EgressGuard::check_addr`
    / `check_endpoint` and all three native ureq SSRF resolvers (`HttpTransport` /
    `HttpFragmentTransport` / discovery `HttpFetcher`). There is no wildcard port and no global
    bypass; default-deny stays default-deny. Use `EgressGuard::is_allowed_port(host, port)` for the
    port-precise check.
- **Phase 3 planner bridge + single-source interpreter** (`sq-j27p`) — the consumer of THIS
  planner's `JoinTree`. The plan speaks pattern/source **indices** only (no endpoint-URL
  mapping — the Phase-0 finding); `sparq_fedclient::SourceResolver` is the **index → adapter
  resolution layer** that maps a plan `pattern: usize` → `TriplePattern` and a `source: usize`
  → a source adapter (the resolver requires the `descriptors`/`adapters` slices to be in the
  same order, and range-checks every lookup). `lower_leaf` lowers one BGP pattern to a
  single-pattern `SELECT`; `materialize_single_source` walks the `JoinTree`, fetches each
  leaf's SRJ through the Phase-2 adapter, parses it, and natural-joins in the plan's join
  order. The load-bearing property — the materialised federated result **equals** local
  `sparq-engine` evaluation of the same query (`solutions_equal` bag comparison) — is driven
  end-to-end in `tests/planner_result_equals_local_eval.rs` (in-memory `Transport` double)
  **and, over a REAL in-process `sparq-server` loopback on `127.0.0.1:0`, in
  `tests/endpoint_loopback_result_equals_local.rs`** (`sq-my8wd.2`): the latter closes the
  honest gap that the equivalence tests otherwise only exercise the in-memory seam (the SRJ
  HTTP round-trip + the on-the-wire SSRF guard are never touched), and asserts the load-bearing
  egress invariant **non-vacuously** — a sibling *default-deny* `Endpoint` against the *same*
  live loopback host is `FedError::EgressRefused` while the per-endpoint-allowlisted one
  reaches the server, so the refusal cannot be a "server is down" artifact; `discovery` is
  driven the same way (allowlisted `HttpFetcher` reaches it via the ASK-probe fallback, a
  default-deny one is refused). The fedclient guard is owned per-endpoint (no process-global to
  widen), so the allowlist granted to one endpoint never leaks to another. `materialize_single_source` is
  single-source + blocking (its streaming counterpart is Phase 5 below); a leaf the planner
  retained >1 source for fails closed with `InterpError::MultiSource`. To fan such a leaf out
  as a per-source UNION use `materialize_multi_source` / `stream_multi_source` (bead `sq-7yf0`,
  below) — the opt-in multi-source entry points that never return `MultiSource`.
- **Phase 4 capability-aware pushdown** (`sq-7byx`) — the `pushdown` module decides the most
  precise sub-query each source is asked. `exclusive_groups(selection, bgp)` derives the FedX
  **exclusive groups** (maximal connected sub-patterns whose only retained source is one
  member — exactly-one-source, same-source, share-a-variable, via union-find); a cross-source
  or zero-source pattern is excluded. `push_group(...)` builds the **maximal sub-algebra** per
  group: projection trimmed to the join + output vars, the FILTER conjuncts the source's
  `FilterClass` covers AND that pass the common-variable check, `ORDER`/`LIMIT` when the
  capability allows — a full endpoint gets the whole group as one multi-pattern `SELECT`, a
  fragment source answers one pattern only (no collapse, no filter pushed — honest about a
  fragment server's access unit). `common_variable_check(filter, group_vars)` is the **exact**
  check Comunica omits (#834/#609): push a conjunct only when *every* variable it references is
  bound by the group. `render_values_block` / `bind_block_size` are the cross-group bind-join
  block primitive (VALUES for endpoints — `DEFAULT_BIND_BLOCK`; `maxMpR` for brTPF; none for
  plain TPF), mirroring `sparq-engine`'s `pub(crate)` `service.rs` helpers. Pushdown only ever
  **narrows** a source's result, so it is correctness-preserving; the FILTER model is light
  (the parsed-query FILTER algebra wiring is Phase 5).
- **Phase 5 streaming operators** (`sq-vtba`) — the streaming counterpart of the Phase-3
  interpreter, built ON THIS crate's non-blocking `StreamJoin`. `sparq_fedclient::stream`'s
  `SolutionStream` is a bounded, backpressured `Iterator` over a `std::sync::mpsc::sync_channel`
  (the channel bound IS the backpressure); `operators::ScatterPool` is a **bounded blocking
  thread-pool** over the blocking transport — the ASYNC/RUNTIME decision (no async runtime is
  pulled in; all concurrency is `std`-only and confined to the opt-in crate). `StreamingJoin`
  drives THIS crate's `StreamJoin` over two `SolutionStream`s, bridging `oxrdf::Term` rows into
  the `Tuple` model losslessly via the term's canonical N-Triples form (`Term::Display` ↔
  `Term::from_str`). `stream_single_source` walks the same `JoinTree`, fans each leaf's blocking
  fetch onto the pool, and chains the leaves through streaming joins so results EMIT before the
  inputs are exhausted. The load-bearing invariant — the streamed multiset is **multiset-equal**
  to the Phase-3 materialised result for **any** source-arrival interleaving (and both equal
  local eval) — is driven on the real engine path under injected per-leaf delays + a forced spill
  in `tests/streaming_result_equals_phase3.rs`. The *pushed-down* bind-join (VALUES/`maxMpR`)
  remains deferred; a bind-classified join runs as the same streaming symmetric hash join
  (identical result multiset).
- **Multi-source UNION-per-leaf fan-out** (`sq-7yf0`) — `materialize_multi_source` /
  `stream_multi_source` lift the single-source `InterpError::MultiSource` guard: a leaf the
  planner retained >1 source for is answered as the **bag-union** of every retained source's
  solutions for that pattern (SPARQL UNION's multiset semantics — concatenation, no de-dup,
  multiplicity preserved), resolving each candidate `source` index to its adapter through the
  `SourceResolver`. The streaming path fans each retained source's fetch onto the same
  `ScatterPool`, all feeding ONE per-leaf `SolutionStream` (a cloned `SolutionSink` per source
  job). Both fold the per-leaf unions through the unchanged left-deep `natural_join` /
  `StreamingJoin`. `tests/multi_source_union_result_equals_local.rs` proves the materialised
  AND streamed multi-source result equals local `sparq-engine` evaluation over the union of
  every source's graph. The single-source entry points keep the fail-closed `MultiSource`
  contract — multi-source is the opt-in entry point.
- **Phase 6 the brTPF + TPF fragment adapters** (`sq-2qze`): `source::TpfSource` (plain TPF —
  materialise a fragment to exhaustion, no bind-join) and `source::BrTpfSource`
  (bindings-restricted — push `maxMpR`-bounded binding blocks per request, the standardised
  brTPF bind-join), both over the `FragmentTransport` seam, with count-metadata
  (`hydra:totalItems`) cardinality surfaced as a one-pattern `SourceDescriptor` for this
  planner. The fragment adapters answer one triple pattern **completely** and return typed
  `FragBinding`s via `solutions(...)` (a fragment server speaks triples, not
  SPARQL-Results-JSON, so their `FederatedSource::execute` is a deliberate `Unsupported` that
  points at `solutions`).
  - **Native HTTP `FragmentTransport` + interpreter wiring** (`sq-yzca`):
    `source::HttpFragmentTransport` (native-only — ureq behind the SAME default-deny SSRF
    resolver as the SRJ `HttpTransport`) is the production seam. It serialises a `FragPattern`
    into the Hydra TPF query string (`?subject=&predicate=&object=`, percent-encoded N-Triples
    terms), attaches a brTPF binding block as the `values` parameter (the server's text wire),
    follows the opaque `hydra:next` page URL to exhaustion, and parses the Turtle/TriG body
    (oxttl `TriGParser`, a Turtle superset) — splitting Hydra/VoID **control** triples
    (`hydra:totalItems`/`void:triples` → count; `hydra:next` → next link) from **data** triples
    (kept only when they match the requested pattern). The `operators` interpreter is wired:
    `fetch_leaf_relation` dispatches on `source_type()`, routing an endpoint/local leaf to the
    SRJ `execute` path and a TPF/brTPF leaf to the typed `solutions` path (lowered via
    `planner::lower_leaf_fragment`), converting `FragBinding` rows back to `oxrdf::Term` so a
    fragment leaf equi-joins with an endpoint leaf. A brTPF leaf currently runs as a complete
    unbound scan the interpreter hash-joins locally (the same discipline the Phase-3 interpreter
    applies to `JoinAlgo::Bind`); the streamed per-block bind-join feeder is a later phase.
  - **brTPF binding-block wire codec** (the `wire` module, `sq-6ihg`, follow-up to the
    server's `sq-dxhb`): the brTPF bind-join attaches a SET of upstream solution mappings (a
    `&[FragBinding]` block, at most `maxMpR`) to each fragment request, and that block is
    re-sent on every request of a bind nested-loop join. The sparq server parses it from a
    **line-oriented TEXT wire** — one mapping per line, space-separated `position=term` pairs,
    each term fully N-Triples-decorated — which is readable but verbose: it repeats the
    `s=`/`p=`/`o=` key and the `<…>`/`"…"`/`^^<…>` framing on every term. The `wire` module
    adds the **compact, self-describing BINARY mapping wire** the bead asks for
    (`encode_bindings` / `decode_bindings`) the client can emit instead, plus the text-wire
    writer (`encode_bindings_text`) so a client can speak EITHER form over the same
    `FragBinding` model (the server already parses the text one). The binary form's twofold
    compactness win: a 1-byte per-mapping header bitmask records which of `s`/`p`/`o` the
    mapping binds, so a position term carries **no** name bytes (the header bit IS the name),
    and a 1-byte kind tag distinguishes IRI / blank / literal so the bare lexical bytes follow
    length-prefixed with **no** `<>`/`""` framing. A binding over an arbitrary (non-position)
    variable name still round-trips losslessly via an overflow EXTRA section, and the binary
    wire carries literals with embedded `=`, whitespace, or newlines that the one-mapping-
    per-line text wire cannot represent (the text writer drops a non-position variable — it
    has no brTPF slot). The container leads with a 4-byte magic (`BINARY_MAGIC`, ASCII `bTPF`)
    + a 1-byte `BINARY_VERSION` so a future revision is detectable, and `decode_bindings`
    validates every length against the remaining input, so a truncated / bad-magic / bad-
    version / bad-kind / varint-overflow buffer is a clean `WireError`, never a panic or OOB
    read (the crate is `forbid(unsafe_code)`). Position keys decode in a deterministic
    canonical `s`→`p`→`o` order, and the empty mapping μ₀ is skipped on encode (it does not
    restrict a fragment) exactly as the server's `parse_bindings` skips an all-blank line.
    **HONEST scope — a codec only:** it converts `&[FragBinding]` ↔ bytes / `String`; it
    issues no request. The native HTTP `FragmentTransport` (`HttpFragmentTransport`, `sq-yzca`,
    above) attaches the **text** form on the `values` query parameter (the carrier the server
    reads); this binary wire is the compact alternative a body-carrying transport emits.
- **Phase 7 adaptive re-planning** (`sq-ij5x`, the FINAL phase) — the client-side ANAPSID
  feedback loop, behind the extra default-OFF **`fedclient-adaptive`** feature (which pulls
  this planner's `adaptive-replan`). `adaptive::execute_adaptive_single_source` runs the plan
  as a leaf-scan phase (fetch each leaf once through the real adapter, record its REAL observed
  row count into `RuntimeStats`) followed by an adaptive join-ordering phase that drives this
  crate's `AdaptiveExecutor`: at each operator boundary it re-invokes the planner on the
  **unjoined remainder** when an observation diverges past `divergence_factor`, adopting the
  cheaper suffix only when it clears the hysteresis margin — **at most once per boundary**, no
  thrash. The re-plan DECISION engine is this planner's `AdaptiveExecutor` (the client does not
  re-write it); the client supplies real observed cardinalities and joins the re-ordered suffix
  with the SAME materialised `natural_join`. Re-planning changes the plan, never the answer:
  `tests/adaptive_result_equals_static.rs` asserts the adaptive result equals both the static
  interpreter and ground-truth local engine eval across a genuine large-divergence switch.
  - **Multi-source (union-arm) adaptive loop** (`sq-xw8zz`):
    `adaptive::execute_adaptive_multi_source` lifts the adaptive path's `MultiSource` guard
    exactly as `sq-7yf0` did for the static/streaming interpreters — a leaf's retained arms
    are fetched per source (bag-union re-keyed onto the pattern header; a failed arm fails the
    leaf CLOSED, never a silent arm-drop), each successful arm's wall-clock fetch latency is
    recorded under its SOURCE index via `RuntimeStats::record_source_latency` (a failed arm
    records nothing — a transport error's duration would make a fast-failing source look
    attractively fast), and the observed leaf cardinality is the UNION count. That makes the
    latency-aware cost bias above — slowest-arm default AND the opt-in `CardinalityWeighted`
    (sq-s5kd) — consumable from a LIVE run, not just planner-level tests, and is the live seam
    the deferred source-failover work needs. The final `RuntimeStats` is exposed on
    `AdaptiveOutcome::stats` (carry observations across queries / assert what the re-planner
    saw). `tests/adaptive_multi_source_result_equals_local.rs` holds the loop to the same
    merged-graph engine oracle as `sq-7yf0`, with two distinct live per-arm latencies.

With Phase 7 the **8-phase streaming federation client is feature-complete** (Phases 0–7 all
landed; epic **sq-dnko** closed). Multi-source UNION-per-leaf fan-out has since landed under
epic sq-3183 (`sq-7yf0`, above; the ADAPTIVE loop's union-arm counterpart is `sq-xw8zz`,
above). Still ahead as future beads under epic sq-3183: the pushed-down streaming bind-join,
and the ANAPSID "adaptive operator" refinement (estimate a leaf's cardinality from a prefix
of its rows while still streaming it). See `crates/sparq-fedclient/README.md`.

**Test-quality note — the mutation ratchet is measured features-ON (`sq-3dyje.6`).** The whole
`sparq-fedclient` surface (and every test file) is `#[cfg(feature = "fedclient")]`-gated, so
`cargo-mutants` MUST run it with `--features fedclient,fedclient-adaptive` — a features-off run
builds an EMPTY crate and reports every mutant as a spurious survivor (the same per-crate quirk
`.github/workflows/ci.yml` applies to `sparq-canon`'s `rdf12-triple-terms` and `sparq-prov`'s
`reason`). The feature-on suite pins EXACT observable values — rendered pushdown sub-queries,
error-variant `Display` strings, SRJ/Service-Description parse outputs, both SSRF
`is_forbidden_ip` boundary tables, and native-transport observables driven over a raw in-process
loopback TCP server (a configured timeout actually bounds a stalled request; a >3 MiB body
round-trips under the real byte cap) — so a mutated return value is *caught*, not merely
executed. A small residue of genuinely-**equivalent** mutants remains and is documented rather
than papered over: in `wire.rs` the `|`↔`^` bit-op mutations are all equivalent — the header /
`read_varint` flag-set `|=`→`^=` (each distinct flag bit is set at most once from a zero start /
LEB128 groups occupy non-overlapping shift windows) and the `write_varint` continuation-bit
`byte | 0x80`→`byte ^ 0x80` (`byte` is masked to `& 0x7f`, so bit 7 is provably clear), so
XOR ≡ OR on every reachable input; `EgressGuard::deny_private`→`Default::default()` is
equivalent because the struct derives `Default` and `deny_private` constructs exactly the empty
allowlist the default does (its own doc says "Equivalent to `EgressGuard::default`"); the
`exclusive_groups` union-find inner-loop bound `(i + 1)..n`→`(i * 1)..n` is equivalent (the extra
`j == i` iteration does an idempotent self-union — same source, self-shares-var, `union(i,i)` is a
no-op — so the group set is unchanged); and the `push_group` SubQuery-`project`-FIELD
`&&`→`||` (line ~486) is equivalent because both branches yield an empty `Vec` in the only case
they differ (`output_vars` empty ⇒ `project` empty). The `proj_clause` `&&` (line ~449) is NOT
equivalent — it is killed by `push_group_projection_clause_by_output_var_membership`. Finally the
`ScatterPool` `Drop::drop`→`()` is equivalent: after any drop body Rust drops the struct's fields,
and the `tx: Option<SyncSender>` field-drop closes the channel exactly as the explicit
`self.tx.take()` does, so detached workers still exit (the un-joined `JoinHandle`s make drop order
unobservable). (`ScatterPool::join`→`()` is a DIFFERENT method and is NOT equivalent — its
blocking drain is killed by `scatter_pool_join_blocks_until_all_jobs_complete`.) These
equivalents are noted, not asserted on — a test that "kills" an equivalent mutant would be vacuous.

**Test-quality note — the mutation ratchet for `sparq-fedplan` is measured features-ON (`sq-3dyje.7`).** The entire `sparq-fedplan` surface (and every `#[cfg(test)]` block) is `#[cfg(feature = "fedplan")]`-gated, so `cargo-mutants` MUST run with `--features fedplan,adaptive-replan` — a features-off run builds an empty crate and every mutant trivially survives (the committed 534-surviving / 0-caught baseline was this feature-OFF artefact, mirroring the `sparq-fedclient` / `sparq-canon` / `sparq-prov` quirk). With features ON the ratchet drops to 52 surviving / 415 caught (89 %). Six externally-authored tests in `crates/sparq-fedplan/tests/mutation_kill_assertions.rs` pin additional observable decisions: `plan_bgp` refuses an empty BGP with a non-empty selection slice (`pattern.rs:114`); `diverges()` requires STRICTLY greater than `k × estimate` in both directions — `o == k*e` and `e == k*o.max(1)` do NOT trigger (two `>=` mutants killed via `AdaptiveExecutor::maybe_replan` on a 3-arm star BGP after one `advance()`, ensuring `remaining.len() >= 2` so the short-circuit guard is not hit); `evict_stale(0.0)` evicts sources with age > 0 without short-circuiting (`< 0.0` guard fires only for negative max_age, killed by `<= 0.0` mutant); and `iri_eq` correctly excludes non-`scs:CharacteristicSet` rdf:type triples from the char-set classification (`descriptor.rs:431`, killed by `→ true` mutant). A genuinely-equivalent residue of 46 survivors is documented below — these are NOT papered over with assertions. Key categories: (1) **spill/budget bookkeeping** — `StreamJoin::spill_key -=`→`+=`/`/=` and `spilled()`-path mutations are equivalent because the spill key is only used as a de-duplication marker and any consistent alteration per spill run behaves identically under the existing test load; (2) **Tuple key encoding** — `> 7`→`>= 7`/`== 7`/`< 7` mutations are equivalent because the test data either stays in or out of the short encoding regime under all variants; (3) **run_streaming early-exit comparisons** — `< capacity`→`>= capacity`/`> capacity`/`== capacity` mutations survive because the bounded-result test queries always exhaust results before hitting the capacity edge; (4) **cost-formula** — `plan.rs:263`/`:262`/`:259`/`:258` delete-arm / operator mutations survive because the test plans use symmetric estimates that produce the same greedy choice under both forms; (5) **plan_suffix_greedy** — several `<`→`==`/`>`/`<=` and delete-arm mutations in `adaptive.rs:902–908` are equivalent under the star-BGP test fixture's symmetric or dominated cost ratios; (6) **descriptor** — `char_set` builder `< → ==`/`<=` boundary mutations survive because the test fixtures use counts that avoid the exact boundary; (7) **is_subset** `+=`→`*=` is equivalent when accumulator values are 0 or 1 (the test uses binary indicator sums); (8) **suffix_cost_greedy** `*`→`+` in the cost product is equivalent when all cost components are 1.0 (unit estimates). The `record_source_latency_after` `contains_key → true` mutant is equivalent because `fold_latency`'s `Entry::Vacant` arm seeds unconditionally (no alpha applied), so the guard's value on the first call for a fresh source is unobservable. These equivalents are noted, not asserted on.

## Deferred (NOT here)

**Mid-*operator* adaptivity** (tearing down a join while it is producing output and resuming
its half-built hash tables under a new algorithm) and **live source failover** (switching to
a replica mid-stage when a source goes dark — observed latency now *biases the join order*
toward faster sources, but hard failover needs the live multi-source execution layer this pure
crate does not own) are out of scope. Filed as roadmap beads under epic **sq-3183**.

sq-a35t / sq-vf7q / sq-7s4z / sq-b51o / sq-7byx — flag for Fable re-review.
