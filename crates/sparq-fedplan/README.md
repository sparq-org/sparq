# sparq-fedplan

**Cost-based federated source selection** and **bind-vs-hash join planning** over
already-fetched source descriptors — a small, opt-in, **pure and deterministic**
planner with no network I/O (epic sq-3183). From a SPARQL BGP and a set of
`SourceDescriptor`s (VoID property/class partitions + mined `scs:` characteristic
sets) it prunes sources per pattern (HiBISCuS-style **recall-safe**), estimates
cardinality (CostFed skew-aware + characteristic-set star joins), and builds a join
order. An opt-in `adaptive-replan` feature adds stage-boundary re-planning, and the
`StreamJoin` operator gives a memory-bounded non-blocking join with spill.

> **Opt-in** (`fedplan` / `adaptive-replan` features, OFF by default). The lean
> `sparq-core` / `sparq-engine` build and the WASM artifact are byte-identical
> without this crate. Timings observed on this work box are **non-canonical**.

## 🚀 Quickstart

```toml
[dependencies]
sparq-fedplan = { version = "0.1.3", features = ["fedplan"] }
oxrdf = { version = "0.3", features = ["rdf-12"] }
```

```rust,ignore
use sparq_fedplan::{
    select_sources, plan_bgp, Bgp, TriplePattern, Term, Var,
    SourceDescriptor, SourceId, PredPartition, PlanOptions,
};

// 1. Build a source descriptor (or parse it from /.well-known/void).
let src = SourceDescriptor::builder(SourceId::new("https://a.example/sparql"))
    .total_triples(10_000)
    .predicate(PredPartition {
        predicate: "http://xmlns.com/foaf/0.1/knows".into(),
        triples: 2000, distinct_subjects: 1000, distinct_objects: 1800,
    })
    .build();

// 2. Select sources for a BGP (recall-safe — a source is only pruned when the
//    descriptor proves it holds no matching triple).
let bgp = Bgp::new(vec![
    TriplePattern::new(
        Term::Var(Var::new("s")),
        Term::Iri("http://xmlns.com/foaf/0.1/knows".into()),
        Term::Var(Var::new("o")),
    ),
]);
let sources = [src];
let selection = select_sources(&bgp, &sources);

// 3. Plan the join (bind vs hash, greedy selectivity-first order).
let plan = plan_bgp(&bgp, &selection, &sources, &PlanOptions::default()).unwrap();
let _order: Vec<usize> = plan.join_order();
```

## ✨ Features

- **`fedplan`** (off by default) — `select_sources` (HiBISCuS recall-safe pruning +
  CostFed skew-aware cardinality) and `plan_bgp` (greedy bind-vs-hash join planner
  using characteristic-set star-join cardinality). Also exposes `StreamJoin`, an
  ANAPSID-style non-blocking symmetric hash join with bounded operator spill, for
  streaming execution of large joins without materialising a full side up front.
- **`adaptive-replan`** (off by default; implies `fedplan`) — `AdaptiveExecutor`:
  mid-execution plan switching at stage boundaries when observed cardinalities or
  source latencies diverge from estimates past a policy factor, with hysteresis to
  prevent thrashing. Soundness boundary: only the not-yet-started suffix is reordered;
  BGP join is commutative/associative so results are unchanged.

All features are **off by default**. The crate is `forbid(unsafe_code)`.

## 📚 Learn more

- `skills/federated-planning/SKILL.md` — full public-API surface: recall-safety
  invariant, `StreamJoin` spill correctness proof, EWMA latency heuristics, and
  the `sparq-fedclient` consumer that calls this planner.
- `research/feature-research-federation.md` — the design record.
- `crates/sparq-fedclient` — the streaming federation client (epic sq-dnko) that
  discovers remote source capabilities, calls `select_sources` + `plan_bgp`, and
  streams results back over the HTTP transport.
- `AGENTS.md` — contributing guidelines.

## License

MIT — see the workspace [LICENSE](../../LICENSE).
