<!-- sq-4lvq: README brought to template (deferred from sq-inzv). -->
# sparq-sim

**Training-free structural entity similarity** for the sparq RDF engine — an **opt-in**
crate that computes similarity straight from existing permutation indexes. No embeddings,
models, network, or extra state: the indexes stay the feature store under graph updates.

## 🚀 Quickstart

```rust,ignore
use sparq_sim::{Sim, SimConfig, SignatureMode, weighted_jaccard};

let sim = Sim::new(&graph);                       // defaults: PredicateNeighbor + IDF
let sim = Sim::with_config(&graph, SimConfig {    // or configure:
    mode: SignatureMode::PredicateNeighbor,       //   | Predicates (role/profile sim)
    idf: true,
    exclude_predicates: vec![rdf_type],           // e.g. when rdf:type is ground truth
    max_pair_frequency: 10_000,                   // hub cap (the approximation knob)
    ..SimConfig::default()
});

sim.similarity(&a, &b);                  // weighted Jaccard in [0, 1]
sim.most_similar(&a, 10);                // Vec<(Term, f64)>, best first, self excluded
let sig = sim.signature(&a).unwrap();    // build once, reuse
sim.similar_by_signature(&sig, 10);      // probe with an arbitrary signature
weighted_jaccard(&sig_a, &sig_b);        // for callers that cache signatures
```

## ✨ Features

- **Structural signature** — an entity's signature is its set of
  `(direction, predicate, neighbor)` pairs (outgoing from one SPO range scan, incoming
  from one OSP/OPS scan), cost `O(log n + degree)`.
- **Opt-in multi-hop expansion** — the default-off `multi-hop` feature + `SimConfig::depth > 1`:
  deterministic breadth-first expansion; hop `h` weights attenuated by `0.5^(h - 1)`.
- **Opt-in explanation** — the default-off `explain` feature adds `explain_similarity(&a, &b)`:
  the shared (direction, predicate, neighbor, weight) elements behind a score, strongest
  first; the weights sum to the exact weighted-Jaccard numerator.
- **IDF-weighted Jaccard** — `w(e) = 1 + ln(|G| / freq(pred))`, frequencies from the
  store's existing planner stats, so sharing a rare predicate counts for more than
  sharing `rdf:type`.
- **Index-driven `most_similar(a, k)`** — candidate generation through contiguous index
  ranges (**not a full scan**); top `max(4k, 64)` re-scored exactly. Hub elements
  (`max_pair_frequency`) skipped during generation only; set `usize::MAX` for exact.
- **Opt-in MinHash/LSH sketch index** — the default-off `sketch` feature +
  `Sim::sketch_index(SketchConfig)`: prebuilt sketches + LSH buckets replace per-query
  range scans on dense graphs (§6). Scores stay **exact**; only recall is probabilistic.
- **Two signature modes** — `PredicateNeighbor` (default): similar = **shares concrete
  context** (same team, same games), the mode candidate generation is built around.
  `Predicates`: similar = **used the same way** (predicate profile / role similarity).
- **Neighbor-sparse profile fallback** (`SimConfig::profile_fallback`, default on) — for
  classes where every entity names a unique neighbor, fill starved result slots with
  role-profile matches ranked below exact neighbor matches.
- **T-box-aware signatures** (`tbox` + `SimConfig::tbox_aware`) — `rdfs:subClassOf` /
  `rdfs:subPropertyOf` closure; inferred elements at `0.5×` IDF weight; no new deps.
- **Lexical fallback tier** (`lexical` + `SimConfig::lexical_fallback`) — sorted IRI
  local-name trigram-Jaccard tertiary fallback; ranks below structural; no new deps.
- **Hybrid search with text vectors** — structural similarity knows how entities are
  *connected*, not what their labels *mean*. The opt-in [`sparq-vectors`](../sparq-vectors)
  crate covers the text side and ships dependency-free fusion helpers (no cross-dependency):

  ```rust,ignore
  let structural = Sim::new(&graph).most_similar(&query, 50);
  let text: Vec<(_, f64)> = index.nearest_term(&query, &graph, &store, 50)
      .into_iter().map(|(t, s)| (t, s as f64)).collect();
  // Reciprocal Rank Fusion: rank-based, no score normalization needed.
  let hybrid = sparq_vectors::fuse_rrf(&[&text, &structural], sparq_vectors::RRF_K, 10);
  ```

  Over-fetch each signal (k = 50 for a top-10 fusion) so the fusion has overlap to reward.
  `fuse_scores(&text, &structural, alpha, k)` is the tunable min-max-normalized alternative.

## Graph scoping

`Sim::new(&graph)` operates on the store of the `Graph` it is handed — the **default
graph** for the top-level `&graph`, or a **single named graph** for that graph's
sub-`Graph`; fetch one with [`Graph::named_graph(&name)`][named-graph] (sq-quuu):

```rust,ignore
let g1 = graph.named_graph(&ex_g1).expect("graph exists");
let sim = sparq_sim::Sim::new(g1); // signatures + similarity scoped to ex:g1 alone
```

Signatures never reach **across** graphs and there is no union-of-all-graphs mode: on a
multi-graph dataset choose the graph (or the default graph) explicitly.

[named-graph]: https://docs.rs/sparq-core/latest/sparq_core/struct.Graph.html#method.named_graph

## Measured results — olympics

`bench/qlever-olympics/olympics.nt` (134,730 foaf:Person + SportsTeam/SportsEvent/
Olympics/Sport/City), ground truth = `rdf:type`, **type triples excluded** (leakage rule,
design doc §5.5), stratified per-class sampling. The `olympics_eval` example reports
quality + latency and checks the gates (same-class precision@10; `Predicates`-mode
class-separation AUC; `most_similar(k=10)` latency); absolute figures live on the perf
dashboard.

```sh
cargo run -p sparq-sim --example olympics_eval --release
# add `-- --json <path>` to also write accuracy + latency as machine-readable JSON
# (STDOUT unchanged; latency advisory/non-canonical)
```

**The two AUCs.** Pairwise AUC asks whether two random same-class entities outscore two
cross-class ones. In `PredicateNeighbor` mode most same-class pairs share no concrete
neighbor and tie at 0 — that mode measures shared context, not class membership; role
separation is the `Predicates` mode's job. The ranking task (`most_similar`) is measured
by precision@10.

## 📚 Learn more

- Design record: [`research/genai-design.md`](../../research/genai-design.md)
- Text embeddings: [`sparq-vectors` README](../sparq-vectors/README.md),
  [`research/genai-text-embedding-practices.md`](../../research/genai-text-embedding-practices.md)
- Skill: `skills/structural-similarity/SKILL.md`
- Perf dashboard: <https://sparq.jeswr.org/dev/bench>

## License

MIT. sq-lsxd
