# sparq-engine

<p>
  <a href="https://crates.io/crates/sparq-engine"><img src="https://img.shields.io/crates/v/sparq-engine.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-engine"><img src="https://docs.rs/sparq-engine/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

The [SPARQL 1.1](https://www.w3.org/TR/sparql11-query/) / [1.2](https://www.w3.org/TR/sparql12-query/)
query engine over [`sparq-core`](../sparq-core) `Graph`s.

Query in-memory or out-of-core graphs, inspect plans with `EXPLAIN` / `EXPLAIN ANALYZE`,
and register custom functions. [Exact temporal comparison and optional year budgets](../../skills/zk-query-proofs/references/exact-temporals.md) preserve fractional precision.

<!-- [GPT-6] The detached proof guest does not add an engine dependency. -->
The opt-in [proved evaluator](../../zk/sparql-evaluator/README.md) restricts `target_os = "zkvm"`,
rejecting ambient NOW/RAND/UUID in that target only. The experiment is
not externally audited.

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;

let g = Graph::load_str(
    r#"<http://example.org/alice> a <http://schema.org/Person> ."#, "turtle")?;

let rows = sparq_engine::query(&g, "SELECT ?s WHERE { ?s a <http://schema.org/Person> }")?;
let json = sparq_engine::query_json(&g, "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }")?;
# let _ = (rows, json);
# Ok(()) }
```

## ✨ Features

- **SPARQL query** — run [SPARQL 1.1](https://www.w3.org/TR/sparql11-query/) and
  [1.2](https://www.w3.org/TR/sparql12-query/) over your data (conformance tracked by the CI
  ratchets), plus the *non-standard* `MULTIPLICITY()` aggregate extension — see the SKILL.
- **Path multiplicity and correlation** — alternatives/sequences preserve bag counts;
  reachability retains endpoint sets. [Scoped EXISTS/MINUS substitution](../../skills/sparql-query/exists-minus.md) applies captured IRI/literal bindings before domain subtraction; unresolved shapes retain native practical behavior.
  Nullable paths preserve constant seeds and variable node domains. Substitution admits
  only positive shapes with locally bound FILTERs; scope-sensitive shapes and triple-term
  endpoints use potentially more expensive ordinary evaluation. [Boundaries and examples](../../skills/sparql-query/SKILL.md).
- **Deterministic builtins** — `isNumeric` validates lexicals/facets independently of finite
  arithmetic capacity; integer casts truncate within `i64`. `SUBSTR` clips the original
  one-based interval. Date accessors validate calendars/offsets and normalize next-day
  midnight; `MIN`/`MAX` retain input terms. [Bounded coverage and numeric limits](../../skills/sparql-query/SKILL.md)
  remain explicit; these corrections do not establish complete builtin conformance.
- **Named graphs** — query across an active dataset with `GRAPH` and `FROM` / `FROM NAMED`.
- **RDF 1.2 triple terms** — match [triple terms](https://www.w3.org/TR/rdf12-concepts/), including variables inside them.
- **Materialized full paths** *(opt-in `paths` feature, OFF by default)* — `enumerate_paths` returns intermediate nodes and edges for tied shortest paths, bounded simple paths, or cycles back to their start. Each endpoint is unrestricted, one fixed node, or a graph pattern selecting a candidate set.
- **Query plan introspection** — `EXPLAIN` and `EXPLAIN ANALYZE`.
- **Custom functions** — register Rust closures under function IRIs;
  see [`docs/extension-functions.md`](../../docs/extension-functions.md).
- **Custom aggregates + window functions** *(opt-in `window-functions`, OFF by default)* —
  `CustomAggregateRegistry`, `window::apply_window` and `query_over` add named aggregates,
  ranking/offset/aggregate windows and frames. Inline `OVER`/`WINDOW` is a **non-standard**
  source rewrite confined to `query_over`; ordinary `query`/`ask` retain standard syntax.
  See the [query guide](../../skills/sparql-query/SKILL.md) and rustdoc for supported forms.
- **Parameterized prepared queries** *(opt-in `params`, OFF by default)* —
  `PreparedQuery::bind(name, oxrdf::Term)` / `PreparedUpdate::bind` bind typed values through
  algebra rewriting, preventing bound data from altering query syntax. Unknown placeholders,
  binder outputs and invalid term positions fail closed. The opt-in `templates` feature
  adds named typed-JSON templates. [Query/update examples and restrictions](../../skills/sparql-query/SKILL.md).
- **Query-result cache** *(opt-in `result-cache`, OFF by default)* — bounded LRU
  `cache::ResultCache` keys SELECT/ASK by query algebra and caller graph version. The caller
  must advance its `u64` version on every mutation; `is_cacheable` rejects volatile functions,
  SERVICE and custom functions/aggregates. [Cache contract](../../skills/sparql-query/SKILL.md).
- **MVCC / ACID transaction isolation** *(opt-in `txn` feature, OFF by default)* —
  a `txn::TransactionManager` over one logical `Graph`: **snapshot-isolation** reads (`begin_read` → a
  cheap point-in-time `GraphSnapshot`, immune to later commits) and serialized **write** transactions
  (`begin_write` → a private COW fork) with **first-committer-wins** OCC conflict detection (`commit`
  publishes a new generation advancing a `u64` version, or returns `CommitError::Conflict`; a stale
  *non*-conflicting writer is replayed, no lost update). Single-writer SI = serializability (see
  `research/concurrent-serving-litreview-A-mvcc-benchmarks.md` §A.1); built on the existing COW
  delta-overlay substrate. Off, zero code compiles, the default build is byte-identical, no new deps.
- **DP join-order planner (DPccp)** *(opt-in `dp-planner` feature, OFF by default)* — the default greedy GOO planner gains an opt-in connected-subgraph-complement-pair DP (Moerkotte & Neumann, VLDB 2006) that finds a `Cout`-optimal *bushy* join order — seeded from the SAME cardinality estimator — via `with_dp_planner` / `with_dp_planner_budget` (per-thread, like `with_cs_table`), **falling back to greedy above a connected-subgraph budget** and on disconnected BGPs. ORDER-ONLY: identical answers to greedy, proven by the on-vs-off `tests/dp_planner_differential.rs`. Off, zero DP code compiles, the default build is byte-identical, no new deps.
- **RDF writer matrix** *(`serialize-rdf` feature — OFF for a library embedder, but the `sparq-cli`/`sparq-server`
  BINARIES enable it via their default-on `jsonld` feature, [OPUS-4.8] sq-oy1f.4)* — write a `Graph` (or
  `&[oxrdf::Triple]`) back out as Turtle / TriG / N-Quads / JSON-LD 1.1
  (`serialize::{graph_to_turtle, graph_to_trig, graph_to_nquads, graph_to_jsonld, …}`; the `*_with`
  variants + `prefixes_from_pairs` accept a caller's prefix policy), plus deterministic **pretty**
  (indented, sorted, round-trip-correct) variants and true **W3C JSON-LD 1.1 Compaction**
  (`graph_to_jsonld_compact`) and **Framing** (`graph_to_jsonld_framed`) — hand-rolled,
  dependency-free, **pyld-faithful** (differentially verified; see the `serialize::compact` rustdoc).
  The N-Triples writer (`triples_to_ntriples`) is always on; off, zero serializer code compiles, **no
  new dependencies**. The opt-in `streaming-serialization` feature (implies `serialize-rdf`) adds
  `write_turtle_streaming`/`write_trig_streaming` (+ `graph_to_*_streaming`) — render Turtle/TriG into a
  `std::io::Write` one subject-block at a time, **byte-identical** to the buffered writer (chunked CONSTRUCT). See [`skills/data-formats/SKILL.md`](../../skills/data-formats/SKILL.md) recipe 6.
- **Oxigraph-shaped per-solution accessor** *(opt-in `query-solution` feature, OFF by default)* —
  `QueryResult::solutions()` yields borrowed, zero-copy `QuerySolution` views (one per row) matching
  Oxigraph's `QuerySolution` API — `get` by name / `VariableRef` / position, `iter` over the bound
  `(Variable, Term)` pairs, panicking `Index` — over the engine's columnar `{vars, rows}` table with
  no second materialisation (eases Rust Oxigraph interop / migration). Off, zero code compiles, the
  default build is byte-identical, no new deps. See
  [`skills/sparql-query/SKILL.md`](../../skills/sparql-query/SKILL.md).
- **Structured EXPLAIN** *(opt-in `explain-json` feature, OFF by default)* — `explain_plan` /
  `explain_plan_analyze` → a typed `PlanNode` tree (BGP `estimated`, `actual`/`nanos`, per-operator **q-error**) + `to_json()` + a bounded `SlowQueryRing`; off, build byte-identical.
- **Semi-join reducers + membership-cluster planning** *(opt-in `semijoin-bitmap` / `yannakakis` / `cluster-materialize` features, OFF by default)* — `semijoin-bitmap`
  prefilters binary-join scans with an exact membership bitmap; `yannakakis` adds a bottom-up full-semijoin prepass
  for acyclic BGPs (cost-gated; cyclic keep LFTJ); `cluster-materialize` (SP2Bench q07) evaluates an unbound-predicate container-membership pattern together with its small bound-predicate anchor STANDALONE and natural-joins it to the rest, instead of bind-joining the wide relation per driver binding. Pure join-order; result-identical to off (proven differentially); off, no new deps.
- **Algebra rewrite pass** *(opt-in `algebra-rewrite` feature, OFF by default in this library; the shipped `sparq-cli`/`sparq-server` binaries and the conformance/differential harnesses light it by default — sq-7d3dj.30.13)* — `PreparedQuery::parse` folds a `FILTER(?v = <iri>)` IRI constant into the group's triple patterns (an indexed constant-seeded scan, not a post-join filter) and rewrites `OPTIONAL … FILTER(!bound(?v))` to an anti-join (`Minus`); IRI-only (never a literal — the `sq-lr2ii` avoidance contract), bag-result-equivalent, off byte-identical, no new deps. The `antijoin-static-decline` feature (OFF by default) makes the correlated-anti-join recogniser decline via a static left/right variable-set pre-check BEFORE evaluating its mandatory left side, so a bare-`!bound` OPTIONAL (SP2Bench q07's nested levels) is not re-evaluated by the cold fallback; pure evaluation-ordering, bag-result-equivalent.
- **Equality-FILTER value join** *(opt-in `value-join` feature, OFF by default)* — pattern components glued only by a `FILTER(?a = ?b)` equality join on a per-term-class **value** key (SPARQL `=` is value equality, not term identity) instead of cross-product-then-filter, with the original FILTER re-applied as the exact recheck (the `sq-lr2ii` high-precision-decimal class stays exact) and temporals/triple terms paired by the exact evaluator; anything not provably eligible declines to the verbatim plan. Result-identical (kill-switch differentials + an independent oracle); off, zero code compiles, no new deps.
- **Vector-at-a-time columnar dispatch** *(opt-in `vectorized` feature, OFF by default)* — FILTER and aggregate operators use a columnar path for ≥ 256-row batches; the hybrid tri-mask delegates tie/unknown lanes to the scalar path (byte-identical). I5 probe counters (`reset_stats`/`stats_snapshot`/`VecStats`) are test-facing only. Off, zero code compiles, no new deps.
- **Id-level term-identity FILTER fast path** *(opt-in `id-filter-fastpath` feature, OFF by default)* — a compiled `=`/`!=` over operands a static analysis proves non-literal (subject/predicate-only variables, constant IRIs, and — via a snapshot-aware predicate-range check — an object of a constant predicate whose object column has NO literals in the current store snapshot, the SP2Bench `dc:creator`/q08/q12b shape) is decided by dictionary-id (in)equality, and equal ids of ANY kind short-circuit `=` true (canonicalising dict → sameTerm), skipping the per-row term materialisation. The predicate-range verdict is re-checked against the LIVE graph every evaluation, so an UPDATE inserting a literal object simply declines the next query (never cross-snapshot). Unequal-id literals (numeric-promotion, the `sq-lr2ii` class) and possible type-errors fall through to the exact path. Result-identical (differential vs the exact oracle); off, no new deps.
- **Characteristic-set anchor-incidence prune** *(opt-in `cs-anchor-incidence` feature, OFF by default)* — the DISTINCT predicate-projection semijoin (`SELECT DISTINCT ?p WHERE { anchor UNION probe }`, SP2Bench q09) precomputes, per anchor join position, the SET of predicates that relate SOME anchor member, so a candidate predicate absent from that set is pruned by an O(1) membership test instead of a large no-hit clipped block scan. Result-identical (the set only prunes provably-empty existence checks; differential vs the exact scan); conservatively declines on a graph with a pending-update overlay; off, zero code compiles, no new deps.
- **Lazy top-k string sort key** *(opt-in `topk-lazy-strkey` feature, OFF by default)* — an `ORDER BY` on a plain `xsd:string` column with a `LIMIT` builds a zero-allocation id-carrying sort key (compared via the literal's zero-copy value bytes) instead of reconstructing + re-allocating the literal value per input row, so a top-k over a large scan pays no key allocation for the rows it discards. Byte-identical output (a full-output differential + W3C ORDER BY conformance); off, zero code compiles, no new deps.
- **Audited cancellation pointer boundary** — the executor keeps its thread-local/rayon budget snapshot `Copy` with a non-owning cancellation pointer; [GPT-6 Astra] a lifetime-bound guard keeps the caller's `Arc<AtomicBool>` alive through scoped worker joins, restores the previous budget scope on return or unwind, and clears the pointer when the outermost scope exits. The four `unsafe` sites are listed in the workspace unsafe register.

## 📚 Learn more

- **How-to** — [query guide](../../skills/sparql-query/SKILL.md); **API** — [docs.rs](https://docs.rs/sparq-engine).
- **Design** — [`research/ARCHITECTURE.md`](../../research/ARCHITECTURE.md) and the planning / parallelism verdicts in [`research/`](../../research).
- **Performance** — numbers live on the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench), not in docs.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
