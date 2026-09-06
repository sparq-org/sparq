---
name: sparql-query
description: Run SPARQL 1.1/1.2 queries (SELECT/ASK/CONSTRUCT/DESCRIBE) and UPDATE against the sparq RDF engine in Rust — load RDF into a sparq_core::Graph, then use sparq_engine::{query, ask, query_json, count, construct, describe, update}; covers property paths, RDF 1.2 triple terms, aggregates/subqueries, custom extension functions (query_with_functions / FunctionRegistry), prepared queries, query budgets/timeouts, named-graph dataset views, and EXPLAIN. Use when an agent or developer needs to embed/execute SPARQL over sparq.
---

# sparq SPARQL query surface

`sparq-engine` is the SPARQL query/update engine over `sparq-core::Graph` (a dictionary-encoded,
permutation-indexed in-memory RDF store). You load RDF into a `Graph`, then call free functions in
`sparq_engine` to run SELECT/ASK/CONSTRUCT/DESCRIBE and SPARQL Update. Results come back either as a
typed `QueryResult` (rows of `Option<oxrdf::Term>`) or directly as a SPARQL-1.1-JSON string.

## Quickstart

`Cargo.toml`:

```toml
[dependencies]
sparq-core = "0.1"
sparq-engine = "0.1"   # default features: parallel, regex, digest
oxrdf = { version = "0.3", features = ["rdf-12"] }   # only if you build Terms yourself
```

```rust
use sparq_core::Graph;

let ttl = r#"@prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob" ."#;

// Load RDF: "turtle" | "ntriples" | "nquads" | "trig".
let g = Graph::load_str(ttl, "turtle").unwrap();

// SELECT -> typed rows.
let r = sparq_engine::query(
    &g,
    "PREFIX ex: <http://ex/> SELECT ?s ?age WHERE { ?s ex:age ?age . FILTER(?age > 28) }",
).unwrap();
for row in &r.rows {
    // r.vars: Vec<Variable> in column order; each row cell is Option<oxrdf::Term> (None = unbound).
    println!("{:?}", row[0].as_ref().map(|t| t.to_string()));
}

// SELECT straight to a SPARQL 1.1 JSON results string (skips per-cell Term allocation).
let json = sparq_engine::query_json(&g, "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }").unwrap();

// ASK -> bool (early-exits: pattern evaluated under an implicit LIMIT 1).
let yes = sparq_engine::ask(&g, "PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ?x }").unwrap();
```

`Graph::load_str` ignores graph names in TriG/N-Quads (folds into the default graph). To keep named
graphs (so `GRAPH <g> {…}` / `GRAPH ?g {…}` work), use `Graph::load_dataset(text, "nquads"|"trig")`.

## Key APIs

All entry points take `&Graph` + `&str` and return `Result<_, String>` (parse + eval errors are
`String`). The result types:

- `sparq_core::strdist::edit_distance(&str, &str) -> usize` computes character-based
  Levenshtein distance for vocabulary and dictionary suggestion ranking.
- `pub struct QueryResult { pub vars: Vec<oxrdf::Variable>, pub rows: Vec<Vec<Option<oxrdf::Term>>> }`
  — `len()` / `is_empty()` count solution rows. The layout is **columnar** (the `vars` header is
  stored once, not per cell); index a cell as `result.rows[row][col]` where `col` is the position of
  the variable in `result.vars`, `None` = unbound.
- *(opt-in `query-solution` feature, OFF by default)* `result.solutions()` gives an
  **Oxigraph-shaped** per-solution view — an iterator of borrowed, zero-copy `QuerySolution` values
  (one per row, sharing the `vars` header; no second materialisation, no cloned terms). Each matches
  Oxigraph's `QuerySolution`: `sol.get(var) -> Option<&Term>` (by `&str` name with or without a
  leading `?`, by `oxrdf::VariableRef`/`&Variable`, or by positional `usize`; `None` = absent or
  unbound), `sol.iter()` over the bound `(VariableRef, &Term)` pairs (unbound cells skipped),
  `&sol[var]` (panicking `Index`), plus `variables()` / `values()` / `len()` / `is_empty()`. Use it
  for ergonomic Rust Oxigraph interop / migration; the underlying `{vars, rows}` layout is unchanged.
- `pub struct QueryBudget { pub deadline: Option<Instant> /*native only*/, pub max_rows: Option<usize>, pub max_bytes: Option<usize>, pub cancel: Option<Arc<AtomicBool>> }`
  — `QueryBudget::unlimited()` is the no-op default. `max_rows` caps the working-set ROW count;
  `max_bytes` (`sq-s5is`) is the byte-accounted companion — it prices row WIDTH
  (`rows × vars × size_of::<Id>()`) plus the bytes of query-computed (BIND/aggregate/CONSTRUCT)
  literals, so a few very wide rows or a huge computed literal is bounded where the row cap is
  blind. Both are coarse cooperative ceilings (checked at operator entry / per outer loop), a
  conservative LOWER bound on heap — not an exact RSS quota; whichever trips first aborts with
  `query budget exceeded (max-rows|max-bytes)`. `with_cancel(Arc<AtomicBool>)` adds a cross-thread
  cancellation handle; a `Relaxed` store of `true` aborts cooperatively at the next coarse poll with
  `query budget exceeded (cancelled)`. `cancelled_by(flag)` creates an otherwise-unlimited budget.

SELECT/ASK entry points (each has `_prepared`, `_with_budget`, and `_view` variants):

- `query(&Graph, &str) -> Result<QueryResult, String>` — SELECT (also accepts ASK, returning the
  unit-row encoding: 0 vars, one empty row iff satisfiable).
- `query_json(&Graph, &str) -> Result<String, String>` — SELECT/ASK as SPARQL-1.1 JSON.
- `query_json_chunks_with_budget(&Graph, &str, &QueryBudget) -> Result<Vec<String>, String>` —
  streamed JSON (concatenation is byte-identical to `query_json`; for HTTP bodies).
- `query_json_stream_with_budget(&Graph, &str, &QueryBudget, sink)` (+ the
  `query_json_stream_prepared_with_budget` no-re-parse variant over a `PreparedQuery`) —
  emits each JSON chunk to `sink` AS produced (TTFB streaming; the HTTP server's read path).
- `ask(&Graph, &str) -> Result<bool, String>` — requires an ASK query.
- `count(&Graph, &str) -> Result<usize, String>` — solution count without materialising terms.

Graph-valued forms (return `Vec<oxrdf::Triple>`; `*_ntriples` returns a serialised `String`):

- `construct` / `describe` / `construct_or_describe` (form-agnostic) / `construct_ntriples`
  / `triples_to_ntriples(&[Triple]) -> String`.

Update (data lives in the default + named graphs):

- `update(&Graph, &str) -> Result<Graph, String>` — returns a NEW graph (O(n) rebuild).
- `update_in_place(&mut Graph, &str) -> Result<(), String>` — incremental delta-overlay, O(batch);
  WAL-durable for directory-backed graphs. NON-atomic on error: a failing op leaves the earlier
  ops' partial prefix applied (request-level atomicity is the caller's responsibility).
- `update_in_place_atomic(&mut Graph, &str) -> Result<(), String>` (+ `_with_budget`) — request-ATOMIC
  delta-overlay: forks, applies, and commits back ONLY if every op succeeds (else `graph` is left at
  its pre-request state). Use this for SPARQL-1.1 all-or-nothing semantics from a direct library
  consumer without writing your own fork/seal recovery.
- `update_in_place_capturing(&mut Graph, &str, &QueryBudget) -> Result<Vec<UpdateEffect>, String>`
  + `apply_effects(&mut Graph, &[UpdateEffect])` — apply once, capturing the RESOLVED delta, then
  replay it onto a second (e.g. durable mirror) graph WITHOUT re-executing the text. Use this when
  mirroring a sequence of updates so non-deterministic functions (`NOW()`/`RAND()`/`UUID()`/`BNODE()`)
  and `LOAD <remote>` are not re-rolled on the second graph (the server's `--persist` path uses it).

Prepared (parse/plan once, execute many):

- `PreparedQuery::parse(&str) -> Result<PreparedQuery, String>` (also `FromStr`, and
  `From<spargebra::Query>` for programmatically built algebra); `query_prepared`, `ask_prepared`,
  `count_prepared`, `construct_prepared`, `describe_prepared` (+ `_with_budget`).
- *(opt-in `params`)* **Parameterized prepared queries — safe value binding (SPARQL-injection
  mitigation, #901).** `PreparedQuery::bind(name, oxrdf::Term) -> Result<PreparedQuery, String>`
  and `PreparedUpdate::{parse, bind}` + `update_prepared` / `update_in_place_prepared`
  (+ `_with_budget`). `bind` substitutes a typed value into a free placeholder variable via a pure
  **algebra rewrite** (`Variable` -> term node), NEVER string concatenation, so a hostile bound
  value cannot alter query structure. Convenience term constructors: `params::value::{iri, string,
  lang_string, typed, blank}`. Fail-closed (rejects an unknown placeholder, a `BIND`/aggregate/
  `VALUES` output, or a blank node in a predicate/graph slot). The placeholder name may be written
  `who`, `?who`, or `$who`. **Always prefer this over building a query string from untrusted input.**

Extension functions (SPARQL 17.6) and dataset views:

- `FunctionRegistry::new()` + `.register(iri, |args: &[Term]| -> Result<Term, String>)`;
  `query_with_functions(&Graph, &str, &FunctionRegistry)` (+ `_and_budget`); `with_functions(&reg, ||
  …)` scopes the registry over ANY other entry point.
- *(opt-in `service-local`)* the ROWS-returning twin of the scalar registry above:
  `LocalServiceRegistry::new()` + `.register(iri, |req: &LocalServiceRequest| ->
  Result<LocalServiceRows, String>)`, scoped by `with_local_services(&reg, || …)`. A registered
  IRI is answered IN PROCESS at `SERVICE <iri> { … }`, before any HTTP transport. See
  "Local SERVICE handlers" below.
- `DatasetView { base: &Graph, named: Arc<FxHashSet<Term>>, default: DefaultGraphMode }`;
  `query_view` / `ask_view` / `count_view` / `query_json_view` (+ `_with_budget`), or
  `with_view(&v, || …)`.
- *(opt-in `window-functions`)* `CustomAggregateRegistry::new()` + `.register(iri, |members:
  &[Option<Term>]| -> Result<Option<Term>, String>)`; `query_with_aggregates(&Graph, &str, &reg)`
  (+ `_and_budget`) parses+installs, `with_aggregates(&reg, || …)` scopes it. Window functions:
  `window::{WindowSpec, WindowFunction, WindowAggregate, WindowFrame, FrameUnit, FrameBound, SortKey,
  apply_window}` (programmatic pass), or `query_over(&Graph, &str)` (+ `_with_budget`) for the inline
  `OVER(…)` query syntax (a source rewrite over the engine; covers `ROW_NUMBER`/`RANK`/`DENSE_RANK`,
  the windowed aggregates `COUNT`/`SUM`/`AVG`/`MIN`/`MAX` (sq-imj8), the offset/positional
  `LAG`/`LEAD(?x[, n[, default]])` and `NTILE(n)` (sq-hqhc), `PARTITION BY`/`ORDER BY`,
  `ROWS`/`RANGE` frames, and a reusable named `WINDOW w AS (…)` clause (sq-hqhc)).

Introspection: `explain(&Graph, &str)` (plan-only) and `explain_analyze(&Graph, &str)` (plan + per-
operator execution trace) return human-readable TEXT (the default).

Structured EXPLAIN *(opt-in `explain-json` feature, OFF by default)* — a MACHINE-READABLE plan:
`explain_plan(&Graph, &str)` and `explain_plan_analyze(&Graph, &str)` return a typed `PlanNode` tree
(`operator`, `estimated` cardinality on BGP nodes, and — after ANALYZE — `actual` rows, wall `nanos`,
and the per-operator **q-error** = `max(est/actual, actual/est)`; 1.0 is a perfect estimate, larger is
worse in either direction). `PlanNode::to_json()` emits a hand-written JSON projection (no serde dep);
`PlanNode::max_q_error()` is the whole-plan worst. `SlowQueryRing::new(n)` is a bounded ring keeping the
N worst-by-wall-time analyzed plans (`record` / `push` / `slowest` / `to_json`) for an ops slow-query
view. Honest boundaries: only BGP nodes carry an estimate (the only operators the planner sizes);
q-error is `None` when either side is 0; `nanos` read 0 on wasm32 (no monotonic `Instant`) unless the
host installs a per-thread trace clock via `set_trace_clock(fn() -> u64)` — the sparq-wasm ANALYZE
binding routes `performance.now()` through it (sq-vx7ez, #2428).

Persistent planner statistics *(opt-in `persistent-stats`, OFF by default)* are rebuilt explicitly
with `stats::analyze(&graph, saved_store_dir)`. This atomically writes a deterministic `stats.bin`
beside the saved/mmap store containing per-predicate cardinalities and equi-depth subject/object
value histograms. Load it without scanning indexes using `StatsCatalog::load(dir)`, then install it
around queries with `with_stats_catalog(&Arc::new(catalog), || query(&graph, sparql))`. The catalog
changes join estimates/order only; query results are unchanged. `AnalyzeMetrics` exposes stable
triple/predicate/bucket/byte counts for operational checks.

## Common recipes

**Aggregates, GROUP BY / HAVING, subqueries** — standard SPARQL 1.1; no special API:

```rust
sparq_engine::query(&g,
    "SELECT ?p (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?p HAVING (COUNT(*) >= 2)").unwrap();
// COUNT/SUM/AVG/MIN/MAX/GROUP_CONCAT/SAMPLE; SUM/AVG skip unbound members.
sparq_engine::query(&g,
    "SELECT ?s WHERE { { SELECT ?s WHERE { ?s <http://ex/age> ?a } ORDER BY DESC(?a) LIMIT 1 } }").unwrap();
```

*Aggregates over an EMPTY result* (SPARQL 1.1 §18.5.1, pinned by the engine's
`aggregate_empty_semantics` tests and the W3C `agg-empty-group-*` conformance cases):

- **No `GROUP BY`** ⇒ the whole result is ONE implicit group, so you get exactly **one** row
  even when nothing matched. In it: `COUNT(*)`/`COUNT(?x)`/`SUM(?x)`/`AVG(?x)` are
  **`"0"^^xsd:integer`** (a *bound* `0`), `GROUP_CONCAT(?x)` is the empty string `""`, and
  `MIN`/`MAX`/`SAMPLE` are **unbound** (an empty-set error surfaces as an unbound cell).
  Mind the split: `SUM`/`AVG` over empty are `0`, not unbound — so `COALESCE(SUM(?x), 0)`
  returns `0` because `SUM` already returned a bound `0`.
- **With `GROUP BY`** ⇒ an empty input has **zero groups**, so you get **zero** rows (the
  single-implicit-group rule applies only when no `GROUP BY` is written).

**Property paths** (all 8 operators: `/  | ^  *  +  ?` and `!(…)` negated sets) — write them inline:

```rust
sparq_engine::query(&g,
    "PREFIX ex: <http://ex/> SELECT ?x WHERE { ex:alice ex:knows+ ?x }").unwrap();   // transitive
```

**Materialized full paths** (opt-in `paths` feature) — unlike standard SPARQL property paths,
this programmatic API returns every intermediate node and edge. `Shortest` returns all tied
minimum-length paths per endpoint pair; `All` requires a finite `max_length` and returns simple
paths up to that bound. Set `cyclic: true` to request only non-empty paths returning to their start.

```rust
// Cargo.toml: sparq-engine = { version = "0.1", features = ["paths"] }
use oxrdf::{NamedNode, Term};
use sparq_engine::{enumerate_paths, Endpoint, PathMode, PathSpec, Via};

let paths = enumerate_paths(&g, &PathSpec {
    mode: PathMode::Shortest,
    cyclic: false,
    start: Some(Endpoint::Node(Term::NamedNode(NamedNode::new("http://ex/alice")?))),
    end: Some(Endpoint::Node(Term::NamedNode(NamedNode::new("http://ex/bob")?))),
    via: Via::Predicate(NamedNode::new("http://ex/knows")?),
    max_length: None,
})?;
assert!(paths.iter().all(|path| path.nodes.len() == path.edges.len() + 1));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Each endpoint is `None` (unrestricted), `Endpoint::Node` (one fixed node), or `Endpoint::Pattern`
— a SPARQL group graph pattern plus the pattern variable whose bindings are the candidate nodes.
The pattern is evaluated once into a deterministic candidate set; solutions leaving that variable
unbound, or binding a term absent from the graph, contribute no candidate. In the dedicated syntax
the endpoint variable declared just before `=` is the one the pattern must bind, so write
`START ?s = { ?s a ex:Person }`; a pattern that never binds it is a loud error.

`PathSpec::via` also accepts `Via::Pattern`, whose SPARQL group graph pattern must bind the
reserved `?from` and `?to` endpoint variables. The pattern is evaluated once to materialize the
edge relation. In the dedicated syntax, write `VIA { ?from ex:knows/ex:memberOf ?to }`. Because a
pattern has no single predicate term, each returned `?edge` is the canonicalized pattern source as
an `xsd:string` literal (and therefore contains no blank nodes).

The same feature exposes a dedicated non-standard query form through `query_paths` and
`explain_paths`. It is intentionally rejected by the ordinary `query` entry point:

```rust
let result = sparq_engine::query_paths(&g,
    "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:alice END ?e = ex:bob VIA ex:knows")?;
// Columns: ?s ?e ?pathIndex ?hopIndex ?node ?edge; one row per traversed hop.
let plan = sparq_engine::explain_paths(&g,
    "PREFIX ex: <http://ex/> PATHS SHORTEST START ?s = ex:alice END ?e = ex:bob VIA ex:knows")?;
assert!(plan.starts_with("Paths mode=shortest"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

**RDF 1.2 triple terms** (`<<( s p o )>>`) — stored structurally; patterns with variables
inside the triple term match. Load with the `rdf-12` feature enabled on oxrdf/oxttl (default in
this workspace). In Turtle/TriG you can write the reified triple `<< s p o >>` (the
`reifiedTriple` sugar; subject or object position, optionally `<< s p o ~ reifier >>`) or the
annotation block `s p o {| … |}` (which also **asserts** the base triple); both desugar to the
standard `rdf:reifies <<( s p o )>>` form. The triple term `<<( s p o )>>` itself (the `tripleTerm`
production) is **object-position only** (RDF 1.2). See the SPARQL 1.2 triple-term
functions `TRIPLE`/`isTRIPLE`/`SUBJECT`/`PREDICATE`/`OBJECT` for constructing/decomposing them.

```rust
let g = Graph::load_str(
    r#"@prefix ex: <http://ex/> . << ex:alice ex:age 30 >> ex:certainty 0.9 ."#, "turtle").unwrap();
sparq_engine::query(&g,
    "PREFIX ex: <http://ex/> SELECT ?s ?o ?c WHERE { << ?s ex:age ?o >> ex:certainty ?c }").unwrap();
// Annotation-block form (asserts ex:alice ex:age 30 AND records the certainty reifier):
let g2 = Graph::load_str(
    r#"@prefix ex: <http://ex/> . ex:alice ex:age 30 {| ex:certainty 0.9 |} ."#, "turtle").unwrap();
```

**Custom extension functions** — register Rust closures under function IRIs, then `query_with_functions`:

```rust
use oxrdf::{Literal, Term};
use sparq_engine::{query_with_functions, FunctionRegistry};

let mut reg = FunctionRegistry::new();
reg.register("http://ex/fn#double", |args: &[Term]| {
    let [Term::Literal(l)] = args else { return Err("double() expects 1 literal".into()) };
    let n: i64 = l.value().parse().map_err(|e| format!("double(): {e}"))?;
    Ok(Term::Literal(Literal::from(n * 2)))
});
let r = query_with_functions(&g,
    "PREFIX fn: <http://ex/fn#> SELECT ?d WHERE { ?s <http://ex/age> ?a . BIND(fn:double(?a) AS ?d) }",
    &reg).unwrap();
// To use the registry with ANOTHER entry point: with_functions(&reg, || sparq_engine::ask(&g, q))
```

**Local `SERVICE` handlers** (opt-in `service-local` feature; `sq-lsp7k.2.2`) — the ROWS-returning
extension seam. `FunctionRegistry` is scalar (terms in, ONE term out); some extensions must return a
whole RELATION, and SPARQL already has the syntax for "evaluate this group elsewhere and join the
solutions back". Register a handler under a SERVICE IRI and the executor answers
`SERVICE <iri> { … }` from it — **before** any HTTP transport is constructed — interning the
returned rows against the query's dictionaries exactly as `VALUES` does and joining them into the
surrounding query:

```rust
// Cargo.toml: sparq-engine = { version = "0.1", features = ["service-local"] }
use oxrdf::{Literal, Term, Variable};
use sparq_engine::{query, with_local_services, LocalServiceRegistry, LocalServiceRows};

let mut reg = LocalServiceRegistry::new();
reg.register("urn:ex:squares", |_req| {
    // The request describes the SERVICE group: .service() / .vars() / .patterns() / .query().
    let n = Variable::new("n").unwrap();
    let sq = Variable::new("sq").unwrap();
    let rows = (1i64..=3)
        .map(|i| vec![Some(Term::from(Literal::from(i))), Some(Term::from(Literal::from(i * i)))])
        .collect();
    Ok(LocalServiceRows::new(vec![n, sq], rows))
});
let r = with_local_services(&reg, || {
    query(&g, "SELECT ?n ?sq WHERE { SERVICE <urn:ex:squares> { ?n <urn:ex:sq> ?sq } }")
}).unwrap();
```

- **Request.** `LocalServiceRequest::{service, vars, query, patterns}`. `vars()` is the group's
  in-scope variables (first-occurrence order); `query()` is the group rendered as
  `SELECT * WHERE { … }` — byte-for-byte what the HTTP transport would have POSTed; `patterns()` is
  the table-function argument channel — the group decomposed into `LocalServicePattern { subject,
  predicate, object }` of `LocalServiceSlot::{Term, Var}`, where the CONSTANT slots are the
  pre-bound arguments. Decomposition is all-or-nothing: it is non-empty ONLY for a flat BGP, EMPTY
  for any richer group (OPTIONAL/UNION/FILTER/sub-SELECT/paths/triple terms), so a handler can
  never mistake a partial view for the whole group. Such a handler works from `query()`.
- **Response.** `LocalServiceRows { vars, rows }` (`new` / `empty` / `validate`); each row is
  `vars.len()` cells wide with `None` for UNBOUND, over a SUBSET of `req.vars()`. A duplicate header
  variable, a ragged row, or a column NOT in scope in the SERVICE group is reported as a SERVICE
  failure, never silently mangled: the relation must be one the group itself could have produced, or
  an out-of-scope column would join against a same-named variable of the SURROUNDING query. The
  EMPTY relation makes the surrounding join empty — it is NOT the join identity.
- **Errors / `SILENT`.** A handler `Err` is a hard query error; under `SERVICE SILENT` it is
  swallowed and yields the join identity so the surrounding solutions survive — identical to how a
  remote failure degrades. Terms interned before a swallowed failure are rolled back and their
  byte-budget charge refunded, so `SILENT` is resource-neutral.
- **Egress / SSRF.** The intercept is upstream of the transport, so a handled SERVICE performs no
  network I/O and the default-deny egress allowlist is never consulted — there is nothing to
  consult it about. This can only ever REMOVE outbound reach: an IRI that MISSES the registry falls
  through to the unchanged `service` HTTP path where the allowlist still applies in full.
  Registering an IRI therefore SHADOWS the remote endpoint of the same IRI for the install's scope.
  A handler is arbitrary in-process code, exactly like a `FunctionRegistry` closure — the engine
  neither sandboxes it nor polices sockets IT opens; that is the application's choice, made when it
  linked the handler in.
- **Composition.** `service-local` and `service` are independent and compose; `service-local` alone
  gives local handlers with NO HTTP/TLS stack compiled (an unregistered IRI is then the same
  "unsupported graph pattern" error the fully-feature-off build gives it). The registry is
  thread-local and propagated into the engine's rayon workers, so a `SERVICE` nested under
  `FILTER EXISTS` still sees it.
- **v1 scope-outs.** Only a CONCRETE `SERVICE <iri>` dispatches locally — `SERVICE ?ep { … }` keeps
  its existing behaviour even when `?ep` binds a registered IRI. No bindings are pushed INTO a
  handler: the SERVICE bind-join (`VALUES` pushdown) explicitly DECLINES for a registered IRI, so
  the handler is evaluated once, unbound, and the outer join happens locally.

**Custom aggregate registry** (opt-in `window-functions` feature) — register a Rust closure as a
named aggregate IRI, then call it from a real SPARQL `GROUP BY`. Unlike a scalar extension function,
the closure FOLDS the group's per-member values (`None` ≡ unbound for that member) into one term:

```rust
// Cargo.toml: sparq-engine = { version = "0.1", features = ["window-functions"] }
use oxrdf::{Literal, Term};
use sparq_engine::{query_with_aggregates, CustomAggregateRegistry};

let mut reg = CustomAggregateRegistry::new();
reg.register("http://ex/agg#product", |members: &[Option<Term>]| {
    let mut acc: i64 = 1;
    for m in members { if let Some(Term::Literal(l)) = m { acc *= l.value().parse::<i64>().map_err(|e| e.to_string())?; } }
    Ok(Some(Term::Literal(Literal::from(acc))))
});
let r = query_with_aggregates(&g,
    "PREFIX ex: <http://ex/> PREFIX agg: <http://ex/agg#> \
     SELECT ?d (agg:product(?s) AS ?p) WHERE { ?x ex:dept ?d ; ex:sales ?s } GROUP BY ?d",
    &reg).unwrap();
// query_with_aggregates DECLARES every registry IRI to the parser (so `agg:product(?s)` parses as an
// aggregate, not a scalar call) AND installs the registry for evaluation. `with_aggregates(&reg, ||..)`
// is the scoped form (composes with with_functions / with_view).
// CAVEAT: a `DISTINCT` custom aggregate (`agg:product(DISTINCT ?s)`) does not currently PARSE in the
// vendored spargebra (a parser limitation, tracked as a follow-up bead); non-DISTINCT works.
```

**`MULTIPLICITY()` inside an aggregate** (no feature needed; default build) — a **NON-STANDARD
extension** exposing the SPARQL 1.2 algebra's `multiplicity(μ|Ω)` device (which replaced the informal
`card[Ω](μ)`). It is a zero-argument builtin valid ONLY inside an aggregate's argument expression: it
evaluates to the bag cardinality of the current group member's solution within its group multiset.
W3C SPARQL 1.2 defines `multiplicity` only as an algebra/semantics device — there is **no callable
`multiplicity()` builtin in the rec and no conformance test for it** — so this is a sparq extension,
not a 1.2-conformance feature.

```rust
// A sub-SELECT projecting away the subject collapses several rows to identical solutions
// (bag semantics), so the group's distinct member carries a multiplicity > 1.
let g = Graph::load_str(
    "@prefix ex: <http://ex/> . ex:a ex:v 10 . ex:b ex:v 10 . ex:c ex:v 10 . ex:d ex:v 20 . ex:e ex:v 20 .",
    "turtle").unwrap();
// SUM(?x * MULTIPLICITY()) folds the DISTINCT solutions {10(×3), 20(×2)} = 10·3 + 20·2 = 70,
// which equals plain SUM(?x) over the bag {10,10,10,20,20}. This is the whole point of the device.
let r = sparq_engine::query(&g,
    "PREFIX ex: <http://ex/> SELECT (SUM(?x * MULTIPLICITY()) AS ?w) WHERE { SELECT ?x WHERE { ?s ex:v ?x } }").unwrap();
```

Semantics: when an aggregate argument calls `MULTIPLICITY()`, sparq folds the group's **distinct**
solutions, each weighted by its bag cardinality (per the Set-Function algebra), so a value occurring
`k` times contributes `k·x`, not `k²·x`. Outside an aggregate (e.g. a plain `BIND`) `MULTIPLICITY()`
is an expression error → unbound. An aggregate that never mentions it is byte-for-byte unchanged.

**Window functions** (opt-in `window-functions` feature) — **NON-STANDARD extension**: SPARQL has no
W3C-REC `OVER (PARTITION BY … ORDER BY …)` syntax, so sparq exposes windowing two ways, both following
the SQL:2003 window model (the surface Stardog / AnzoGraph expose) and neither touching the engine's
W3C-conformance SPARQL surface.

*(a) Programmatic pass over a `QueryResult`* — the full surface. Run an ordinary SELECT, then apply a
`WindowSpec` (`ROW_NUMBER` / `RANK` / `DENSE_RANK`; the offset/positional `Lag` / `Lead` / `Ntile`
(sq-hqhc); or a windowed aggregate over the whole partition, or — sq-imj8 — over an explicit
`ROWS`/`RANGE` frame). One column is appended; row order is preserved:

```rust
use sparq_engine::window::{apply_window, SortKey, WindowFunction, WindowSpec};
use oxrdf::Variable;

let result = sparq_engine::query(&g, "SELECT ?emp ?dept ?sales WHERE { … }").unwrap();
let spec = WindowSpec {
    partition_by: vec![Variable::new("dept").unwrap()],
    order_by: vec![SortKey::desc(Variable::new("sales").unwrap())],
    function: WindowFunction::Rank,            // RANK() over (PARTITION BY ?dept ORDER BY ?sales DESC)
    frame: None,                               // a frame applies only to an Aggregate (None = whole partition)
    new_var: Variable::new("rank").unwrap(),
};
let ranked = apply_window(&result, &spec).unwrap(); // ?rank appended; RANK has gaps after ties
// Windowed aggregate over a FRAME (sq-imj8): a running total is
//   WindowFunction::Aggregate { agg: WindowAggregate::Sum, of }
//   + frame: Some(WindowFrame::rows_running())  // ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
// `WindowFrame { unit: FrameUnit::Rows|Range, start, end }` with `FrameBound::{UnboundedPreceding,
// Preceding(n), CurrentRow, Following(n), UnboundedFollowing}`; RANGE bounds are peer-group based
// (no numeric RANGE offset). `frame: None` keeps the whole-partition default.
```

*(b) Inline `OVER(…)` query syntax* (sq-h564) — write the window directly in the SELECT projection and
run it with `query_over` / `query_over_with_budget`. This is a **source rewrite in front of the engine**
(it does NOT change the vendored `spargebra` parser): `query_over` lifts each `(FN() OVER (…) AS ?out)`
item out of the projection, runs the window-stripped SELECT through the ordinary engine, applies the
programmatic pass above, then reprojects. **Covered subset:** the ranking functions `ROW_NUMBER()`,
`RANK()`, `DENSE_RANK()`, the windowed aggregates `COUNT(?x)` / `SUM(?x)` / `AVG(?x)` / `MIN(?x)` /
`MAX(?x)` (sq-imj8 — single `?var` argument; case-insensitive), and the offset/positional functions
`LAG(?x[, n[, default]])` / `LEAD(…)` / `NTILE(n)` (sq-hqhc). A spec can be reused via a named
`WINDOW w AS (…)` clause and `OVER w` (sq-hqhc). `PARTITION BY ?v …`, `ORDER BY` over
projected variables **or computed expressions** (sq-c1jv) — e.g. `ORDER BY (?a + ?b)`, `DESC(?sales + 0)`,
`STRLEN(?s)`; each expression is bound to a fresh helper var in the rewritten inner SELECT and dropped
from the output — in both `DESC(…)` and `… DESC` spellings, an explicit `ROWS`/`RANGE` frame on an
aggregate (sq-imj8 — `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`, `ROWS n PRECEDING`,
`RANGE BETWEEN … AND CURRENT ROW`, etc.), multiple window columns, and `SELECT *`. A query with no
`OVER` clause is run unchanged (so `query_over` is a strict superset of `query` for non-window queries):

```rust
// Cargo.toml: sparq-engine = { version = "0.1", features = ["window-functions"] }
let r = sparq_engine::query_over(&g,
    "PREFIX ex: <http://ex/> \
     SELECT ?emp (ROW_NUMBER() OVER (PARTITION BY ?dept ORDER BY DESC(?sales)) AS ?rn) \
     WHERE { ?emp ex:dept ?dept ; ex:sales ?sales }").unwrap(); // ?rn = per-?dept rank, descending by ?sales
```

**Inline-OVER caveats (HONEST):** the `OVER` surface (and the `WINDOW` clause) is a sparq extension, not
W3C SPARQL, recognised ONLY on the `query_over` entry point (a `(… OVER …)` clause / `WINDOW` clause is
still a parse error on `query`/the standard surface). A ranking function call must be empty
(`ROW_NUMBER()`); a windowed aggregate takes a single `?var` argument (`SUM(?x)`); `LAG`/`LEAD` take a
bare `?var` plus an optional integer offset and a constant default, `NTILE` a positive integer; the
`AS ?out` alias is required. The `OVER` operand is either an inline `(…)` spec or a name bound by a
trailing `WINDOW name AS (…)` clause. `ORDER BY` keys may be projected variables OR computed expressions
(sq-c1jv) but `PARTITION BY` keys must be projected variables. A `ROWS`/`RANGE` frame is valid only on an
aggregate (a frame on a ranking or offset function errors), and `RANGE` supports only the peer-group
bounds (`UNBOUNDED …` / `CURRENT ROW`), not a numeric `RANGE n PRECEDING` offset. **Deferred
(programmatic API only, beaded):** a `DISTINCT` windowed aggregate (`COUNT(DISTINCT ?x) OVER …`), an
aggregate / `LAG`/`LEAD` argument over a computed-expression, numeric `RANGE` offsets, and `PARTITION BY`
over a computed expression.

**Budgets / timeouts / ASK-style early exit** — a `QueryBudget` is checked cooperatively at coarse
sites; tripping it fails with `"query budget exceeded (timeout)"` / `"... (max-rows)"` /
`"... (max-bytes)"` / `"... (cancelled)"`. The budget bounds result SERIALISATION as well as
evaluation (`sq-yfcu2`): a SELECT-JSON body whose deadline falls due while the (already
materialised) result is being written out is reported as the budget error, not returned as a
complete-but-late result — on the streamed entry points some chunks may already have reached the
sink when the trip is detected:

```rust
use sparq_engine::QueryBudget;
use std::sync::{Arc, atomic::AtomicBool};
let cancel = Arc::new(AtomicBool::new(false));
let budget = QueryBudget {
    max_rows: Some(10_000),
    max_bytes: Some(64 << 20), // byte-accounted companion (sq-s5is); None = off
    #[cfg(not(target_arch = "wasm32"))]
    deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(2)),
    ..QueryBudget::unlimited()
}.with_cancel(Arc::clone(&cancel));
let r = sparq_engine::query_with_budget(&g, "SELECT * WHERE { ?s ?p ?o }", &budget);
// Another thread may call cancel.store(true, std::sync::atomic::Ordering::Relaxed).
// For existence checks prefer ask()/ASK — it streams under an implicit LIMIT 1 (cheapest early exit).
```

**Named-graph dataset view** (zero-copy restriction; a non-visible graph is indistinguishable from
an absent one):

```rust
use std::sync::Arc;
use oxrdf::{NamedNode, Term};
use sparq_engine::{DatasetView, DefaultGraphMode, FxHashSet, query_view};

let store = Graph::load_dataset(nquads, "nquads").unwrap();
let visible: Arc<FxHashSet<Term>> =
    Arc::new([Term::NamedNode(NamedNode::new("http://ex/g1").unwrap())].into_iter().collect());
let v = DatasetView { base: &store, named: visible, default: DefaultGraphMode::StoreDefault };
let r = query_view(&v, "SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap(); // only g1 visible
```

## Gotchas / feature flags / prerequisites

- **Errors are `String`** — both SPARQL parse errors and evaluation/type errors. SPARQL is parsed by
  (a vendored) `spargebra` with `sparql-12` + `sep-0006`.
- **Only SELECT/ASK** go through `query`/`query_json`/`count`; CONSTRUCT/DESCRIBE have their own
  functions, and a form mismatch is a clean `Err` (e.g. `ask()` on a SELECT).
- **Numeric FILTER comparisons are VALUE-EXACT, incl. high-precision `xsd:decimal`** (`sq-lr2ii`).
  A single-pattern `?v OP const` numeric FILTER is normally pushed into the scan and decided via the
  O(1) f64 `numerics` cache (`extract_sargable`). Because an f64 cannot represent a decimal with
  `> 15` significant digits exactly (e.g. `"1.000000000000000001"^^xsd:decimal` shares the f64 `1.0`
  with the integer `1`), that fast path is **declined** whenever the graph holds such a decimal
  (`Graph::has_high_precision_decimal`) OR the constant itself has `> 15` significant digits — the
  comparison then falls back to the exact evaluator (`=`/`<`/`>`/`<=`/`>=` decided by exact decimal
  string arithmetic, matching the `BIND((?v = c) AS ?b)` expression path). The decline is
  graph-wide/conservative (correctness first): a graph carrying one high-precision decimal skips the
  numeric-pushdown optimisation for all its numeric FILTERs, never the wrong answer.
- **`SELECT *`** never exposes blank-node "variables" (`_:x` in a pattern is an existential var).
- **Non-deterministic builtins** (`sq-98w7z.1`) — `NOW()` is **query-constant** per SPARQL 1.1
  §17.4.5.1: every evaluation within one execution (across rows, parallel workers, `EXISTS`
  re-entry, sub-selects) returns the same `xsd:dateTime`, pinned at execution start and re-sampled
  by the next execution (each UPDATE operation's WHERE is its own execution; second granularity).
  `RAND()` stays fresh per call, drawn from a per-thread splitmix64 PRNG seeded once from OS
  entropy — NOT cryptographic (SPARQL imposes no such requirement); don't derive secrets from it.
  `UUID()`/`STRUUID()`/no-arg `BNODE()` are likewise fresh per call — the engine never memoises a
  non-deterministic builtin (the allow/deny policy is spelled out in `eval_function_inner`).
  `REGEX()`/`REPLACE()` memoise the compiled `(pattern, flags)` per thread (capped at 64 entries),
  so a constant-pattern FILTER compiles once instead of per row; an invalid pattern/flag is still a
  per-row type error — the failure outcome is memoised, the semantics are unchanged.
- **`update` vs `update_in_place` vs `update_in_place_atomic`** — `update` returns a fresh `Graph`
  (input borrowed, untouched, atomic by construction); `update_in_place(&mut g, …)` mutates via the
  delta overlay (call `Graph::compact` periodically) but is NON-atomic on error; `update_in_place_atomic`
  forks-and-seals so an in-place mutation is all-or-nothing. `LOAD` only resolves `file://`; set the
  base dir with `with_load_base(path, || update(...))`.
- **SPARQL `SERVICE` federation** is the non-default `service` cargo feature (pulls `ureq`; off on
  wasm). Internally the SERVICE client (HTTP transport, SPARQL-Results JSON/XML parse, bound-join
  batching, SSRF egress policy) is housed in the `sparq-engine-service` sub-crate (`publish = false`,
  [OPUS-4.8] sq-6vshe.4, seam A2 of the facade split) and re-exported through the facade — the
  `service` feature name and the `sparq_engine::with_service_egress_allow` / `…egress_policy` /
  `…SERVICE_EGRESS_REFUSED_MARKER` / `…allowlist_entry_permits` paths below are unchanged and remain
  the supported surface.
  When enabled, outbound `SERVICE` fetches go through a **default-deny SSRF egress filter**:
  an endpoint that resolves to a loopback / RFC1918 / link-local (incl. the `169.254.169.254`
  cloud-metadata IP) / unique-local / unspecified address is refused (checked on the *resolved* IP,
  so DNS rebinding can't bypass it). To federate to a trusted internal endpoint, allowlist its host
  for the scope: `with_service_egress_allow([host.to_string()], || query(&g, q))`. Public endpoints
  need no opt-in. An allowlist entry may be **host-level** (`127.0.0.1` — every port) or
  **port-scoped** (`127.0.0.1:8053`, `[::1]:8053`, `.example.org:443` — that host on THAT port ONLY,
  rejecting every other port) (`sq-a7jw4`): a port-scoped entry is strictly NARROWER, so an
  in-process loopback harness can permit exactly `127.0.0.1:<ephemeral>` without re-opening the whole
  loopback host. The port checked is the authority's port (its explicit `:port` or the scheme default),
  which is exactly the dialled port — so port-scoping applies to the connect target and the resolved
  IP is still re-vetted (no wildcard port; tightens, never loosens). Every egress-refusal error string
  carries the stable
  `sparq_engine::SERVICE_EGRESS_REFUSED_MARKER` substring (`sq-iu0c`), so a network-exposed host can
  `contains()`-classify a blocked `SERVICE` as an authorization-policy refusal (e.g. HTTP `403`)
  rather than a server fault — `sparq-server` does exactly this.
- **`SERVICE` bind-join (`VALUES` pushdown)** — when a `SERVICE` sub-query is the right side of a
  join (or `OPTIONAL`) whose join variables are already bound by the left side, sparq pushes a
  *block* of those bindings into the remote query as a `VALUES` clause (the brTPF/FedX "bound join")
  instead of fetching the whole remote relation and joining locally. For a selective join this slashes
  the rows the endpoint returns and the data transferred. It is **on by default** — a
  correctness-preserving optimisation of the existing `SERVICE` path (the answer is identical to the
  unbound-then-local-join path; `SILENT`, `OPTIONAL`/left-join, multi-var and empty-binding edge cases
  are all preserved) — and **falls back to the verbatim forward** when it can't apply (no bound join
  var, a join key bound to a blank node). The only tuning knob is the block
  size (distinct binding tuples per remote request): **opt-in** via
  `with_service_bound_join_block_size(n, || query(&g, q))` or the `SPARQ_SERVICE_BIND_BLOCK` env var
  (default ~50). The knob never changes results — only the remote-request count vs per-request size.
- **`SERVICE ?ep` variable endpoint** (`sq-d4p`) — a `SERVICE ?ep { P }` whose endpoint variable is
  bound by the surrounding query is evaluated per SPARQL 1.1: the bind-join path partitions the
  already-bound left rows by their `?ep` value and dispatches one bind-join **per distinct endpoint
  IRI**, tagging each remote row with `?ep = <that endpoint>` so the surrounding join re-attaches it
  to exactly the left rows that named that endpoint. A left row whose `?ep` is unbound or not an IRI
  names no valid endpoint and contributes no federated answer. A **top-level** `SERVICE ?ep` with
  nothing to bind it still errors (or, under `SILENT`, yields the join identity). (No new public API —
  this is a behavioural addition on the `service` feature.)
- **Per-query `SERVICE ?ep` remote-request cap** (`sq-b93pv`) — a `SERVICE ?ep` whose endpoint
  variable binds to **many distinct endpoint IRIs** fans out into one remote dispatch per distinct
  endpoint, so a hostile / high-cardinality `?ep` can amplify one client query into a burst of
  outbound calls (the amplification the per-host egress allowlist does not bound). The **opt-in**
  `with_service_remote_request_cap(n, || query(&g, q))` (or the `SPARQ_SERVICE_REMOTE_CAP` env var)
  caps the number of distinct endpoints one `SERVICE ?ep` evaluation may dial; exceeding it is a
  typed refusal **enforced PRE-HTTP** (at eval time, before any socket opens — a true pre-dispatch
  bound, not a post-hoc cancellation) carrying the stable `sparq_engine::SERVICE_REMOTE_CAP_MARKER`
  substring (so a server can classify it like the egress marker). It is **not** swallowed by
  `SILENT` (a resource-policy refusal, not an unreachable endpoint — the same stance the query
  budget takes). **Default is uncapped**, so a concrete-IRI `SERVICE <…>` and a normal-cardinality
  `SERVICE ?ep` are unchanged.
- **Per-query `SERVICE` timeout** (`sq-d4p`) — the SERVICE HTTP transport's socket timeout is now
  bounded by the active `QueryBudget` deadline (it caps at `min(remaining-until-deadline, default)`
  with a small non-zero floor), so a `SERVICE` fetch under a tight `deadline` no longer blocks for the
  full default on an unresponsive endpoint.
- **Streaming / bounded `SERVICE` result consumption** (`sq-my8wd.4` / `sq-my8wd.5`) — remote
  SPARQL-Results bodies (JSON and XML) are parsed INCREMENTALLY: each solution row is interned to
  compact id-level bindings as it is parsed and the owned terms are dropped, so the engine never
  holds a whole-document JSON DOM or a term-level copy of the remote relation. Peak memory per
  `SERVICE` response is dominated by the id-level relation the join itself needs — a
  large/adversarial remote result can no longer amplify to a large multiple of its wire size.
  Behaviour-neutral by test: a frozen DOM-reference oracle pins identical rows, multiplicity, ORDER
  and errors (`service.rs` `streaming_equivalence` tests).
  **Reader-seam transport** (`sq-my8wd.5`): the `sparq-engine-service` crate's `ReaderTransport`
  trait extends the `Transport` seam so the HTTP body is NEVER buffered into a full `String` —
  `HttpTransport` implements it by returning the ureq response body reader directly. The
  `TransportAsReader<'t, T>` adapter wraps any `Transport` implementation as a `ReaderTransport`
  (useful for test mocks). The `eval_remote_into_read` function is the reader-seam entry point —
  it is used internally by the engine's `exec.rs` SERVICE evaluator. These three items
  (`ReaderTransport`, `TransportAsReader`, `eval_remote_into_read`) live in
  `sparq_engine_service::service` and are not re-exported through the `sparq-engine` facade (they
  are implementation-internal); test transports continue to implement `Transport` and are wrapped
  via `TransportAsReader` so the streaming path is exercised without rewriting every canned mock.
  [OPUS-4.8] [FABLE-5]
- **`SERVICE` evaluation is W3C-conformance-tested end-to-end** (`sq-ddpgx`, epic sq-my8wd) — the
  W3C SPARQL 1.1 `sparql11/service` evaluation suite runs against the engine's REAL `ureq` transport
  through an in-process **loopback** harness: each `qt:serviceData` block is served by a real
  `sparq_server::serve` endpoint on an ephemeral `127.0.0.1:0` port (the well-known endpoint IRIs are
  rewritten to the bound URLs), so the whole federated path — HTTP, content negotiation,
  SPARQL-Results parsing, the bind-join over the wire — is exercised, not a mock. It is the
  `sparq-conformance` crate's opt-in **`service`** feature (a ratcheted floor in the central scoreboard;
  `SILENT`-swallow vs non-`SILENT`-propagate against a closed port is pinned directly). Honest scope: a
  variable `SERVICE ?ep` and a nested non-`SILENT` `SERVICE` are documented divergences (the loopback
  fixture serves a plain graph with onward federation disabled), tracked-not-asserted under sq-my8wd.
- **Window functions + custom aggregate registry** are the non-default `window-functions` cargo
  feature. **Window functions are a NON-STANDARD extension** — there is no W3C-REC SPARQL `OVER`
  syntax. sparq exposes them as a programmatic pass over a `QueryResult` (the SQL:2003 model
  Stardog/AnzoGraph expose), `ROW_NUMBER`/`RANK`/`DENSE_RANK`, the offset/positional
  `LAG`/`LEAD`/`NTILE` (sq-hqhc), + a windowed aggregate over the whole partition or an explicit
  `ROWS`/`RANGE` frame (sq-imj8),
  AND as an inline `OVER(…)` query syntax via the dedicated `query_over` entry point (a *source rewrite*
  in front of the engine — it does NOT change the vendored parser, so the standard `query`/`ask`/… surface
  stays exactly SPARQL 1.1 and conformance is unaffected; `OVER` is a parse error everywhere except
  `query_over`). The inline syntax covers the three ranking functions, the windowed aggregates
  `COUNT`/`SUM`/`AVG`/`MIN`/`MAX` with `PARTITION BY`/`ORDER BY` and `ROWS`/`RANGE` frames (sq-imj8), the
  offset/positional `LAG`/`LEAD`/`NTILE` and a reusable named `WINDOW w AS (…)` clause (sq-hqhc);
  `DISTINCT`/computed-expression function arguments, numeric `RANGE` offsets and a computed-expression
  `PARTITION BY` are inline-deferred (use the programmatic API). The custom-aggregate side DOES ride real SPARQL `GROUP BY` (a declared aggregate
  IRI is part of the SPARQL 1.1 extension grammar). When the feature is off, zero window/aggregate-registry
  code compiles and the default build is byte-identical (no new dependencies). See the recipes above.
- **Parameterized prepared queries (SPARQL-injection mitigation)** are the non-default `params`
  cargo feature (bead `sq-rp3um`, #901). When you build a query/update from untrusted input, do NOT
  concatenate it into the string — parse a *template* once and `bind` typed values. Binding is a
  pure algebra rewrite (`Variable` -> term node), so a hostile value is matched as DATA and can
  never inject syntax. Covers SELECT/ASK/CONSTRUCT/DESCRIBE + UPDATE; `bind` is **fail-closed**.

  ```rust
  // Cargo.toml: sparq-engine = { version = "0.1", features = ["params"] }
  use sparq_engine::{params::value, query_prepared, update_prepared, PreparedQuery, PreparedUpdate};
  // A hostile literal: under string concat this would break out and inject an UPDATE.
  let hostile = r#"Bob" } INSERT DATA { <http://ex/evil> <http://ex/p> <http://ex/o> } #"#;
  let pq = PreparedQuery::parse("SELECT ?s WHERE { ?s <http://ex/name> ?nameVal }")?
      .bind("nameVal", value::string(hostile))?;  // carried as one literal — matches nothing
  let _ = query_prepared(&graph, &pq)?;
  // Bind an IRI into a subject/predicate slot (the IRI constructor validates it):
  let pq = PreparedQuery::parse("SELECT ?o WHERE { ?who <http://ex/name> ?o }")?
      .bind("who", value::iri("http://ex/alice")?)?;
  let _ = query_prepared(&graph, &pq)?;
  // UPDATE templates bind the same way (DELETE/INSERT/WHERE):
  let pu = PreparedUpdate::parse("INSERT { <http://ex/x> <http://ex/note> ?n } WHERE { }")?
      .bind("n", value::string(hostile))?;        // exactly one op — no injected DROP/INSERT
  let _g2 = update_prepared(&graph, &pu)?;
  # Ok::<(), String>(())
  ```
- **Vectorized columnar primitives** are the non-default `vectorized` cargo feature (M4 plan, bead
  `sq-hvfe`): the `chunk` module's `DataChunk` (a column-major, vector-at-a-time id-level batch),
  a **decode kernel** (`DataChunk::decode_numeric_column` → a contiguous `Vec<f64>` value column,
  `f64::NAN` sentinel for a non-numeric cell, with a gather-free fast path for an all-inline-integer
  column — the SIMD enabler, M4 Phase 2 `sq-pntvh.2`), a numeric FILTER comparison kernel
  (`DataChunk::select_numeric` → a `SelVec`), a **compare-over-decoded-column** kernel
  (`DataChunk::select_decoded`, the contiguous-memory, auto-vectorisable compare that consumes the
  decode kernel's output — M4 Phase 3 `sq-pntvh.3`), and a selection/gather kernel
  (`DataChunk::apply_selection`). When off, zero columnar code compiles and the default native + wasm
  builds are byte-identical (no new dependencies; no `unsafe`).
  **`query`/`query_json`/etc. return identical results whether the feature is on or off** — the same
  bindings/rows in the same order (byte-identical for the serialising entry points such as
  `query_json`; `query` returns a structured result, so the equality is over its rows, not bytes).
  It is a perf optimisation, never a semantics change. The first evaluator wiring landed in Phase 3
  (`sq-pntvh.3`): a **columnar residual-FILTER seam** inside `apply_filter` (Seam A) that, for an
  eligible single sargable-numeric residual `?v OP const`, decodes the column once and runs the
  **hybrid tri-mask** (`src/chunk_select.rs`, bead `sq-y5ew5`): each lane is classified Confident
  (finite f64, `v != c` — f64 verdict is unambiguous by the monotone-rounding lemma), Tie (`v == c`
  as f64), or Unknown (NaN sentinel for non-numeric / unbound / local-vocab ids). Tie + Unknown lanes
  are **delegated** to the full scalar `eval_expr` predicate (which handles the exact-lexical recheck
  for ties); confident-passes and delegated-passes are merged in ascending order. This is provably
  byte-identical by construction (design record §3). The seam **declines** entirely to the row path
  for temporal or non-sargable filters, batches below `VEC_MIN_BATCH`, and is **disabled whenever the
  `zk` trace is armed** so the row path records the complete FILTER obligation set. The M4 wiring
  roadmap + coexistence model is `research/vector-at-a-time-m4.md` (epic `sq-pntvh`); its acceptance
  gate landed first (Phase 1, `sq-pntvh.1`): the differential BYTE-IDENTITY harness
  `crates/sparq-engine/tests/vectorized_byte_identity.rs` (the columnar kernel's serialised survivors
  are byte-identical to the row `FILTER` operator over the *same batch*, order-exact — a Phase-1
  finding: cross-query byte-identity is unsound because a sargable filter re-plans the scan, so the
  invariant is operator-level not full-query), the seam-level differential in
  `crates/sparq-engine/src/exec.rs` (`mod columnar_filter_seam`), plus the native FILTER/aggregate
  baseline bench `crates/sparq-engine/examples/bench_vectorized.rs` (`metric_us`, registered
  `vectorized-eval-micro`, `featured = false`) later phases measure their speedup against.
  **Phase 5 (`sq-pntvh.5`) adds the shared dispatcher seam** (`src/vec_dispatch.rs`): a single module
  that owns the I1–I5 decline hierarchy and the I5 probe counters shared by every current and future
  seam. `VEC_MIN_BATCH` (256) is the minimum batch size below which the scalar path is cheaper;
  `VEC_MORSEL` (2048) is the decode-buffer size for morsel-by-morsel `apply_filter` execution — each
  morsel extracts only the one filter column (O(rows × 1), not a full-width transpose). The decline
  hierarchy (in order): I2 = ZK-trace armed (scalar records the full obligation set); I3 = budget
  armed (fallback rule — the debit schedule is not uniform-per-row); operator shape check (only a
  simple sargable `?v OP const` numeric comparison, finite constant); `VEC_MIN_BATCH` batch-size gate.
  The aggregate seam (Seam B, `group_aggregate` → `columnar_aggregate`) is also wired through the
  same I2/I3/`VEC_MIN_BATCH` checks. **I5 probe counters (test-facing / unstable)**:
  `reset_stats()` / `stats_snapshot()` / `VecStats` let acceptance tests assert the seams are
  non-vacuous (`chunks_built >= 1` for an eligible operator, `rows_delegated > 0` when tie/unknown
  lanes exist). Thread-local counters are used (not global atomics) so parallel test threads don't
  interfere. The counters are **not semver-stable** — external callers must not build stable logic on
  them; they exist only for the acceptance gate in `tests/differentials/vectorized_byte_identity.rs`
  (T1–T4) and `tests/differentials/chunk_select_tri_mask.rs` (sq-y5ew5 T1–T5).
- **Sideways information passing (SIP) — correlated graph-pattern join** is a DEFAULT-ON,
  semantics-preserving optimisation (bead `sq-7d3dj.30.3`; research/sp2bench-complex-shape-deficit.md
  §2.2). When a `Join(A, B)` is evaluated and the already-evaluated side `A` is **small** (≤ 64 rows),
  the executor evaluates the big child `B` **correlated** on A's bindings instead of cold: for each
  distinct binding of A's variables that are **certainly bound** in `B` and bound to an **IRI** in `A`,
  that IRI is substituted as a constant into `B`'s patterns — pushing *into UNION branches, BGP scans,
  property paths and filters* — so a scan seeds from the constant rather than running blind. Results are
  recombined preserving **multiplicity** (bag semantics). It is **conservative**: it fires only for
  IRI-valued correlation variables that are certain in `B` (never a variable that a solution may leave
  unbound — e.g. inside an OPTIONAL right side, or MINUS, or a projecting sub-`SELECT`), and any
  scope/threshold failure falls back to the **cold** evaluation bit-for-bit. The answer is identical
  either way (proven on-vs-off by `tests/sip_join.rs`, incl. an anti-vacuity trace assertion that the
  correlated child actually collapsed). SP2Bench q08/q12b: the 1-row `?erdoes` side seeds the Union's
  `?document dc:creator ?erdoes` scan instead of a whole-corpus creator self-join. Payoff on complex-shape
  workloads is a measurable hypothesis for the canonical perf host, not a baked-in number.
- **Correlated (theta) anti-join for `OPTIONAL … FILTER(!bound)`** is a DEFAULT-ON,
  semantics-preserving optimisation (bead `sq-7d3dj.30.9`, SP2Bench q06). The negation idiom
  `Filter(!bound(?nb), LeftJoin(A, B, F))` — where `?nb` is **certain** in `B` and **absent** from `A` —
  is exactly an anti-join: a left row survives iff **no** `b ∈ B` is compatible with it **and** makes the
  OPTIONAL condition `F` effectively **true**. The default-off `algebra-rewrite` pass turns the simpler
  `F = None` case into `Minus`, but declines when `F` references **outer** variables (q06's
  `?author = ?author2 && ?yr2 < ?yr`), so the cold `left_outer_join` scans the whole corpus. This
  eval-time path instead splits `F` into var-to-var **correlation** conjuncts `?outer θ ?inner` (with
  `?inner` certain in `B`) and a residual theta (`?yr2 < ?yr`), and picks between two strategies. The
  correlation relation `θ` may be value `=`, `sameTerm`, or a single-variable `?a IN (?b)` membership
  (which desugars to `?a = ?b`); a constant / multi-element `IN` list stays in the residual (`sq-3cmr4`).
  **Large** correlation cardinality → a **hash anti-join**: evaluate `B` **once**, partition it by the
  `?inner` id, then probe each left row's bucket — one `B` scan, O(|A|+|B|) (SP2Bench q06's win); for a
  **very large** anchor side the per-left-row probe is **fanned out over cores** with rayon under the
  `parallel` feature, and only the boolean verdicts are parallelised so the result stays byte-identical
  to the serial probe (`sq-f1emb`). **Small** cardinality → **SIP-seed**: seed the outer correlation term
  sideways into `B` so each distinct correlation evaluates a tiny `B'`. Either way the residual theta is
  re-checked verbatim per candidate with full 3-valued semantics (a **type error ⇒ no match**, so the
  outer row **survives** — never spuriously eliminated). The relation `θ` is **load-bearing**: value `=`,
  `sameTerm` and id-equality differ on value-equal id-distinct literals (`"1"` vs `"01"`). Id-hashing is
  exact for a value-`=` correlation only on **IRI and blank-node** keys (for both, SPARQL `=` coincides
  with term identity — per SPARQL 1.1 §17.4.1.7 RDFterm-equal, `=` on two distinct non-literal terms
  returns **false**, a type error arising only when both arguments are literals, and false and error alike
  mean **no match**, which an id non-match reproduces); a **literal**-valued `=` correlation (the
  `sq-lr2ii` value-equality class) routes through a **value-correct** scan that re-checks with SPARQL `=`.
  A **`sameTerm`** correlation is **term identity** for every kind, so a literal key is itself id-probeable
  and any value-correct scan re-checks with `sameTerm`, never `=`. Each surviving row is emitted **once**
  (multiplicity preserved), and any shape/scope miss (e.g. `?nb` also bound on the left, or no seedable
  correlation) **declines** to the identical prior plan. Proven on-vs-off by `tests/theta_antijoin.rs`
  (type-error-survives, literal value-equality, blank-node, large-cardinality hash, `sameTerm`/`IN`
  correlation + a `=`-vs-`sameTerm` divergence witness, a `PAR_THRESHOLD`-crossing parallel probe, and a
  randomised differential rotating all three relations across both strategies); toggle/stats behind
  `theta_antijoin_testing`. The opt-in **`antijoin-static-decline`** feature (OFF by default, bead
  `sq-7d3dj.30.20`) removes a redundant re-evaluation on the **no-seedable-correlation** decline path:
  when the recogniser's shape gate matches but the OPTIONAL condition is a bare `!bound` (no var-to-var
  correlation to seed — SP2Bench **q07**'s nested levels), the un-featured path evaluates the mandatory
  left side, discovers `correlations.is_empty()`, and declines — after which the cold `Filter{LeftJoin}`
  fallback **re-evaluates that same (dominant) left side**. The feature adds a purely **static** pre-check
  (`in_scope_vars(left)` + `certain_vars(right)`, no graph access) that declines **before** the left side
  is touched, so it is evaluated **once**. Pure evaluation-ordering, **bag-result-equivalent** to off
  (`tests/antijoin_static_decline_differential.rs`, both feature states + the W3C ratchet); off,
  byte-identical, no new deps.
- **Id-level term-identity FILTER fast path** is the non-default `id-filter-fastpath` cargo feature
  (bead `sq-7d3dj.30.11`). It removes the per-row term MATERIALIZATION the compiled FILTER evaluator
  otherwise performs for `=`/`!=`, by two id-level short-circuits: **(a)** a static term-kind analysis
  (`nonliteral_vars`) proves a variable is bound ONLY in BGP subject/predicate slots — never a BGP
  object, a property-PATH endpoint (a path object can be a literal; a `Reverse`/zero-length subject
  end too), a quoted-triple inner slot, a `BIND`/`VALUES` binding, or any variable of an un-analysed
  construct (`SERVICE`/`Lateral`/sub-`SELECT` inner vars are all poisoned to literal-risk) — and so
  can never be a literal — for two such operands (or a constant IRI)
  SPARQL `=` is exactly dictionary-id equality and `!=` id inequality (the canonicalising dict gives
  each IRI/bnode one id); **(b)** EQUAL ids of ANY kind are the same term, so `=` short-circuits `true`
  and `!=` `false` (mirroring the `sameTerm` decision the exact path already takes — safe even for
  ill-typed literals, which return a boolean not a type error). UNEQUAL ids of possibly-LITERAL
  operands (numeric promotion `"1"^^integer` = `"1.0"^^decimal`, whitespace-padded lexicals — the
  `sq-lr2ii` class) and `=` cases that can be a TYPE ERROR fall through to the exact value path
  unchanged. Result-identical whether on or off — a differential test pairs every id kind (IRIs,
  bnodes, numeric-promotion pairs, language-tagged, ill-typed literals) and asserts the fast verdict
  equals the exact `term_of` + `values_equal` oracle row-for-row. When off, the fast-path column-set
  machinery does not compile and the feature-OFF path is BEHAVIOUR-identical to the prior default build
  (no new dependencies; no `unsafe`) — the bytes are NOT identical, because the always-compiled
  EXISTS-re-entry refactor (splitting `eval_exists` into a thin wrapper + an always-compiled
  `eval_exists_inner`) MOVED bytes and shifted panic locations; this byte-move is declared in
  `bench/feature-off-declarations/1785.json` per the feature-OFF-declaration mechanism.
  **Predicate-range widening (`sq-1ivw7`)** closes #1785's honest null result on the q08/q12b shape,
  whose FILTER variables (`?author`, `?erdoes`) appear only as `dc:creator` OBJECT positions (which
  the position-only analysis declined). The widened analysis credits an OBJECT-position variable as
  non-literal when its predicate is a CONSTANT IRI whose object column has NO literal in the CURRENT
  store snapshot — computed at query-execution time by `Graph::predicate_has_literal_object` (an
  overlay-aware, early-exit scan of the predicate's objects; `dict::is_literal_id` classifies each
  object id, with inline integers always literal). The requirement is NO LITERALS, not IRI-only:
  bnode ids are identity-comparable exactly like IRIs for `=`/`!=` (distinct non-literals compare
  FALSE per §17.4.1.7), so a bnode-object predicate still fires. Any literal object for the predicate,
  or a VARIABLE predicate, or MIXED use of the variable (also an object of a literal-having predicate)
  → decline. **Graph scoping (soundness):** the credit is scoped to ONE graph's object column, so an
  object variable bound under a `GRAPH <g>`/`GRAPH ?g` block stays literal-risk at an OUTER dispatch —
  a FILTER sitting outside the GRAPH block classifies against the default graph, but the object is
  bound from a self-contained named sub-graph with its own store/dict, so crediting it off the default
  column would be unsound (a named-graph literal would be id-compared). The inside-GRAPH dispatch is
  correct and unaffected (it evaluates against the named sub-graph, so its own classifier scans the
  right store). `Graph::predicate_has_literal_object` answers for the receiver graph only.
  **Snapshot soundness:** a prepared query in sparq is parse-only (`PreparedQuery` carries
  algebra, no plan); each evaluation borrows an immutable `&Graph` snapshot (a server request pins a
  generation), so the check is re-run against the LIVE graph every eval — an UPDATE inserting a
  literal object publishes a new snapshot the next query re-checks; the verdict never outlives its
  snapshot (there is no plan/inference cache across snapshots; the result cache is keyed by
  `(query, version)`). The same snapshot-aware column set is now also installed at the FUSED
  conjunctive residual-FILTER sites (`eval_flat_conjunctive`, `eval_bgp_binary_capped`), drain-safe
  via `with_idfast_nonlit_cols` (the set drains on the first consuming `apply_filter`, so a nested
  EXISTS on a different `Bindings` layout sees the empty default). A differential test witnesses the
  q08 shape now compiles `?a = ?b` to `IdEqNonLit` (fires), an update-then-requery flips the credit,
  and the end-to-end answer is an independent recompute.
- **DISTINCT-projection loose (skip) index scan** is a DEFAULT-ON, semantics-preserving optimisation
  (bead `sq-7d3dj.30.4`; research/sp2bench-complex-shape-deficit.md §2.3). For `SELECT DISTINCT ?p`
  over a BGP / **UNION** of BGPs where `?p` is the **single** projected variable, the executor
  enumerates the DISTINCT `?p` values **directly** from an existing permutation sorted by `?p` — a
  loose/skip scan that gallops (binary-search) past each value's block — instead of materialising every
  full-width join row and de-duplicating post-hoc. It is the general form of qlever's "pattern trick"
  over sparq's six permutations, and builds **NO new index**. Two branch shapes are enumerated: a single
  BGP triple (every distinct `?p` is a solution) and an **anchor + probe** pair joined on one variable.
  For the anchor+probe branch the executor picks between two equivalent semi-join strategies by anchor
  cardinality (bead `sq-7d3dj.30.10`): a **small** anchor uses the per-member *pattern trick* (bind the
  join column to each anchor member, skip-enumerate its few `?p`, stopping once the `?p` universe is
  covered); a **large** anchor uses a per-`?p`-block existence scan whose `[P, J]`-ordered block is first
  **clipped to the anchor's id window** and then intersected by galloping the **shorter** of block/anchor
  (an O(1) id-range-disjointness fast reject still precedes it). The **sorted anchor set is cached across
  UNION branches** (q09's two branches share one anchor, built once). A **global already-seen `?p` set**
  makes later branches near-free. It is **conservative**: it fires only for that exact single-variable
  DISTINCT shape and only when a built permutation exposes the required column order (the compact/wasm
  index {SPO, POS, OSP} lacks PSO, so a subject-side probe declines there); every other shape — REDUCED,
  multi-variable projections, filters, three-pattern branches, `?p` used as the join variable, a pattern
  with a variable repeated across S/P/O (`?x ?p ?x`), and any pattern embedding an RDF 1.2 quoted-triple
  term (`<<?s ?p "o">> …`, decomposed by the general BGP planner) — falls back to the full
  materialise-then-dedup path **bit-for-bit** (never erroring). The answer is DISTINCT-set identical
  either way (proven on-vs-off by `tests/distinct_pushdown.rs`, incl. a q09-shaped anti-vacuity assertion
  that the scanned rows collapse to a small fraction of the full join). SP2Bench q09: the 2-branch
  `DISTINCT ?predicate` over persons answers a handful of predicates without materialising the ~77 k-row
  join. Payoff on the canonical perf host is a measurable hypothesis, not a baked-in number.
- **Characteristic-set anchor-incidence prune** is the OPT-IN `cs-anchor-incidence` cargo feature (OFF
  by default; bead `sq-jnb1e`) that accelerates the LARGE-anchor block scan above. For q09 the block
  scan still proves each candidate predicate P relates NO anchor member by clipping and galloping P's
  join-value block — for the value-typed predicates (`dc:title`, `foaf:homepage`, `rdfs:seeAlso`, 17k–27k
  distinct objects each) that is a large no-hit scan (~250 k rows examined to answer 4 predicates). With
  the feature on, the executor precomputes, per anchor join position, the SET of predicates that relate
  SOME anchor member (one range-restricted scan over the anchor's id window), so a candidate P absent from
  that set is pruned by an O(1) membership test instead of the block scan — QLever's "pattern-trick"
  metadata answer, over sparq's permutations. It is a characteristic-set INCIDENCE summary (which
  predicates touch a type at which position), distinct from the `cs-planner` predicate-set cardinality CS.
  RESULT-IDENTICAL: the incidence set is EXACT on the base index, so it only ever prunes a
  provably-empty existence check; a P in the set still takes the exact block scan, and the answer is
  bit-for-bit the feature-off answer (differential on-vs-off in `tests/distinct_pushdown.rs`, incl. a
  randomised straddle, a large no-hit-block prune, and answer-safety corners). It fires only when the
  probe constrains **only** the projected predicate and the join variable (else it declines to the exact
  scan), and **conservatively DECLINES on a graph with a pending-update overlay** (the derived set is built
  against the base index; incremental maintenance is out of scope), so a query after an UPDATE simply
  falls back to the exact scan. Off, zero incidence code compiles and no new deps. Payoff on the canonical
  perf host is a measurable hypothesis, not a baked-in number.
- **ASK / LIMIT first-solutions early exit through joins** is a DEFAULT-ON, semantics-preserving
  optimisation (bead `sq-7d3dj.30.8`; research/sp2bench-complex-shape-deficit.md §5). `ASK` is
  evaluated under an implicit `LIMIT 1`, and any `LIMIT k` (no ORDER BY / DISTINCT / aggregation
  above it) now stops early for JOIN shapes, not just single-pattern scans: a conjunctive BGP (+
  FILTERs) is driven in growing **seed-scan blocks** (1024 → 64 k → rest) through the same greedy
  binary join chain, counting a row toward the cap only after **every** residual FILTER has passed
  (a first candidate eliminated by a late FILTER never fires the exit); `UNION` evaluates its left
  branch first and skips the right when the cap is already met; `OPTIONAL` caps its left side (no
  left row is ever dropped by OPTIONAL); `Join(A, B)` probes with a capped `A` through the SIP
  correlated path and falls back when the probe misses. Under ASK the plan is additionally
  simplified where provably emptiness-neutral: `ORDER BY` and `DISTINCT` are stripped along the top
  modifier spine only — NEVER below a `Slice` (`DISTINCT` under `OFFSET` changes the visible count)
  and NEVER touching aggregation/HAVING (an empty-group aggregate still yields a solution). It is
  **conservative**: cyclic (LFTJ) BGPs, quoted-triple patterns, MINUS, property paths, GRAPH,
  bare FILTERs over non-conjunctive groups, and an armed zk-trace recorder all fall back to full
  evaluation bit-for-bit. The boolean is identical to the unoptimized path on every query (oracle
  differential + budget-bounded revert-witness in `tests/ask_early_exit.rs`: an ASK over a join
  whose full materialisation exceeds a row budget must still answer instead of tripping it).
  Honest boundary: an ASK whose answer is `false` still sweeps the whole seed (bounded at ~3x the
  single-pass scans by the three-block schedule), and the right side of `OPTIONAL` is still
  evaluated in full. Payoff (SP2Bench q12a/q12b-class) is a measurable hypothesis for the canonical
  perf host, not a baked-in number.
- **Exact-bitmap semi-join reducer** is the non-default `semijoin-bitmap` cargo feature (M4 plan,
  survey §A3, bead `sq-gr8mb`; CIDR'26 "Not Yannakakis"). When a binary BGP join scans the next
  pattern, the executor first builds a membership filter over the already-materialised side's
  connecting-variable ids — a flat **bitmap** over the dense `u32` ids (the dictionary assigns dense
  ids, so the bitmap is a perfect-hash, zero-false-positive membership test) or an exact **hash set**
  when the keys are sparse+huge — and passes it to the scan, which DROPS a row whose join key cannot
  match **before** that row enters the join. This is a transparent PERFORMANCE optimisation: the
  filter is membership-exact, so it removes only rows the join would have dropped anyway and the
  RESULT is byte-identical to the feature-off path — `query`/`query_json`/etc. return the SAME answers
  whether it is on or off (proven by the on-vs-off equivalence suite `tests/semijoin_differential.rs`);
  only fewer rows are scanned. Its payoff on star/snowflake workloads is a measurable hypothesis for
  the canonical perf host, not a baked-in number. When off, zero of this code compiles and the default
  native + wasm builds are byte-identical (no new dependencies — sparq-core + rustc-hash are already
  direct deps; no `unsafe`).
- **Yannakakis full-semijoin prepass** is the non-default `yannakakis` cargo feature (survey §A4, bead
  `sq-5zf8i`; arxiv 2504.03279 + research/optimization-techniques.md §1.1/§2(a2)) — the complementary
  bottom-up reducer to `semijoin-bitmap` (which it IMPLIES, reusing the same exact `KeyFilter`). Where
  the A3 reducer prefilters one scan against the accumulated result *during* the join loop, the
  Yannakakis prepass runs *before* the main join: it materialises each relation of an **acyclic** BGP
  once, then sweeps the join tree bottom-up (leaf→root) and top-down (root→leaf), semijoining each
  relation against its neighbours so that no "dangling" tuple (one that joins with nothing downstream)
  is ever materialised by the join — directly cutting the intermediate-result blow-up. Routing reuses
  the executor's existing GYO acyclicity test (`bgp_is_cyclic`/`bgp_uses_binary`): **acyclic ⇒
  semijoin-reduce then join; cyclic ⇒ existing LFTJ, unchanged**. A semijoin is a pure FILTER (removes
  only rows the final join would itself drop), so the RESULT is **identical** to the feature-off binary
  plan — `query`/`query_json`/etc. return the SAME answers (proven by the on-vs-off equivalence suite
  `tests/yannakakis_differential.rs` over chain/star/snowflake/cyclic/empty/no-reduction shapes). The
  prepass is **cost-gated** — skipped when the intermediates are already tiny (a pure-overhead guard) —
  so a tiny BGP transparently uses the ordinary binary plan. Payoff on chain/snowflake workloads is a
  measurable hypothesis for the canonical perf host (bead `sq-0g6g`), not a baked-in number. When off,
  zero of this code compiles and the default native + wasm builds are byte-identical (no new deps; no
  `unsafe`).
- **DP join-order enumerator (DPccp)** is the `dp-planner` cargo feature (bead `sq-iywur`;
  Moerkotte & Neumann, VLDB 2006; made default-on for `sparq-cli` by bead `sq-7d3dj.30.5`).
  It enumerates every **connected-subgraph / connected-complement pair** of the BGP join graph
  and fills a DP table `best[S]` = the minimum-`Cout` **bushy** join tree over pattern set `S`,
  finding a provably `Cout`-optimal order (seeded from the SAME index-range/characteristic-set
  cardinality estimator GOO uses) — with **zero cross products** between patterns that share no
  variable. When the feature is compiled (as in `sparq-cli`), DPccp fires **by default** for
  any BGP with ≥ 3 patterns within the subgraph budget — no install call required. To opt out
  for one scope (restores greedy GOO), or to set an explicit budget, use the scoped helpers:
  ```rust
  // Cargo.toml: sparq-engine = { version = "0.1", features = ["dp-planner"] }
  // Default-on — no install required; DPccp fires for BGPs with ≥3 patterns within budget.
  let rows = sparq_engine::query(&graph, sparql)?;
  // Explicit opt-out (restores greedy GOO for this scope):
  let greedy = sparq_engine::without_dp_planner(|| sparq_engine::query(&graph, sparql))?;
  // Explicit install at a custom budget:
  let rows = sparq_engine::with_dp_planner_budget(1024, || sparq_engine::query(&graph, sparql))?;
  ```
  It is **ORDER-ONLY**: a BGP is a commutative/associative natural join, so every tree yields the SAME
  bindings — the DP changes join order, never the answer (proven by the on-vs-off differential suite
  `tests/dp_planner_differential.rs`, which runs in BOTH feature states). DPccp is worst-case
  exponential, so the enumerator counts connected subgraphs first and **falls back to greedy GOO** when
  the count exceeds `DpConfig::max_subgraphs`, on a **disconnected** BGP (a cross-product query or an
  all-constant pattern), and for BGPs with fewer than 3 patterns (for n≤2, greedy is already optimal
  and carries less overhead than the full enumeration path). Cyclic BGPs already route to LFTJ, so the
  DP only ever governs binary-plan BGPs. Its payoff on adversarial BGP shapes is a measurable hypothesis
  for the canonical perf host, not a baked-in number. Hypergraph **DPhyp** and interesting-orders-in-
  the-DP-table are NOT implemented. When off, zero DP code compiles, the default build is byte-identical,
  and no new dependencies are added (no `unsafe`).
- **Membership-cluster pre-materialisation** is the non-default `cluster-materialize` cargo feature
  ([FABLE-5] bead `sq-7d3dj.30.14`; SP2Bench q07, research/sp2bench-complex-shape-deficit.md §5). It
  targets the container-membership idiom `?bag ?member ?doc` — a pattern with an UNBOUND predicate (the
  `rdf:_n` bag members are ordinary triples, so no single bound-predicate scan enumerates a whole bag).
  When such a pattern sits in a BGP next to a small BOUND-predicate anchor it shares exactly one variable
  with (q07's `?doc2 dcterms:references ?bag`), greedy GOO bind-joins the ~250k-row unbound-predicate
  relation per driver binding through the widest permutation. With the feature on, `cluster::detect`
  spots the shape and the executor evaluates the `{anchor, membership}` pair **standalone** (the anchor
  bounds the shared variable to a small set → a few-thousand-row relation) and **natural-joins** it to the
  rest. It is **ORDER-ONLY**: a BGP is a commutative/associative natural join, so partitioning into two
  connected sub-BGPs and joining yields the SAME row bag as greedy — both sub-BGPs are evaluated
  UNCORRELATED, so there is no sideways-information hazard (only one evaluation of the cluster; the
  materialised relation is the full union-compatible superset), and natural join preserves multiplicity
  exactly (no dedup). `detect` **DECLINES** (unchanged greedy plan) on any non-matching shape — not
  exactly one membership pattern, an anchor variable that leaks into the rest, a pushed-down scan filter
  on a cluster pattern, or the cost gate unmet. Proven by `tests/cluster_materialize_differential.rs`
  (randomised shapes + nested-OPTIONAL + multiplicity + a non-vacuous mutation check, run in BOTH feature
  states). The end-to-end q07 win is a measured hypothesis for the canonical perf host (bead
  `sq-7d3dj.30.6`), not a baked-in number. When off, zero of this code compiles, the default native +
  wasm builds are byte-identical, and no new dependencies are added (no `unsafe`).
- **Pre-execution algebra rewrite pass** is the `algebra-rewrite` cargo feature — non-default in the
  sparq-engine LIBRARY, but lit BY DEFAULT in the shipped `sparq-cli` + `sparq-server` binaries and in
  the conformance/differential-fuzz harnesses ([FABLE-5] sq-7d3dj.30.13), so it is what real users and
  the canonical benchmarks execute (bead
  `sq-7d3dj.30.1`; design record `research/sp2bench-complex-shape-deficit.md` §2.1/§2.5/§4). With it ON,
  `PreparedQuery::parse` (the single seam every string query entry point funnels through) and the two
  EXPLAIN parse sites run the parsed `spargebra` algebra through `sparq_engine::rewrite::rewrite_query`
  BEFORE evaluation. Two rewrites: **(a) equality substitution** — a `FILTER(?v = <iri>)` (also
  `<iri> = ?v` / `sameTerm(?v, <iri>)`) whose `?v` occurs in the group's triple patterns folds the IRI
  constant INTO those patterns (turning a post-join FILTER over every enumerated row into a
  constant-seeded indexed scan; `?v` is re-bound via `Extend` so it stays in scope), and **(b)
  anti-join** — `FILTER(!bound(?v), LeftJoin(A, B))` becomes `Minus(A, B)` when `?v` is certain in `B`,
  absent from `A`, and `A`/`B` share a certain variable. **IRI-only:** rewrite (a) fires ONLY for
  `NamedNode` constants (`=` on IRIs is term-identity, so the substitution is exact); a **literal**
  equality (`?v = 1`, `?v = "1"^^xsd:decimal`) is NEVER rewritten — this is the deliberate avoidance
  contract for the open value-equality/decimal bug `sq-lr2ii`. Every rewrite is **bag-result-equivalent**
  (`query`/`ask`/`count`/`query_json` return the SAME answers on vs off — proven by
  `tests/rewrite_pass.rs` plus the W3C conformance ratchet, which CI runs with the feature ON — the
  `sparq-conformance` harness lights it, and the nightly sparq-vs-oxigraph differential fuzzer covers
  the `FILTER(?v = <iri>)` trigger shape ON too since sq-7d3dj.30.13), and a shape that does
  not exactly meet a rewrite's conditions is left **verbatim** (conservative). Note `From<Query>` does
  NOT rewrite (it takes an already-built algebra as-is). When off, zero rewrite code compiles, the
  default native + wasm builds are byte-identical, and no new dependencies are added (no `unsafe`).
- **Equality-FILTER → value join** is the non-default `value-join` cargo feature (bead
  `sq-7d3dj.30.7`; the SP2Bench q05a/q12a star-intersection shape). With it ON, a conjunctive group
  whose triple patterns form variable-DISJOINT connected components glued only by a top-level
  `FILTER(?a = ?b)` conjunct evaluates each component separately and joins them with a hash join
  keyed on the equality, instead of materialising the cross product and evaluating `=` per row.
  **SPARQL `=` is VALUE equality, not term identity** (`"01"^^xsd:integer = "1.0"^^xsd:decimal` is
  TRUE across two dictionary ids; incompatible types are a type error; high-precision
  `xsd:decimal`s sharing an `f64` must stay unequal — `sq-lr2ii`), so a plain id-keyed join would
  be wrong. Soundness is two-layer: the per-term-class key never misses an `=`-TRUE pair (id for
  IRI/bnode/unknown classes, the `numeric_value` cache's canonicalised `f64` for numerics, value
  for `xsd:string`/language-tagged/boolean), rows with no provable key (local-vocab ids, triple
  terms, value-comparable temporals, numeric-datatype lexicals the numeric cache rejected — the
  evaluator's trimming fallback can still pair those) pair through the EXACT evaluator, and the
  original FILTER is re-applied verbatim over the joined rows — the exact recheck that eliminates any key collision
  through the same `sq-lr2ii` machinery that runs today. Anything not provably eligible — a filter
  with any function call (`RAND`/`BNODE` are row-nondeterministic, custom functions can error),
  quoted-triple patterns, an armed query budget or `zk` trace, no cross-component equality —
  DECLINES to the verbatim plan. Result-identical on eligible shapes (per-term-class kill-switch
  differentials in `exec::eqjoin::tests` + an independent `!(?a != ?b)` oracle in
  `tests/eqjoin_value_join.rs`, both run by the `opt-in sparq-engine (value-join)` CI leg). When
  off, zero of this code compiles, the conjunctive path is byte-identical, no new dependencies.
- **Materialised-view / query-result cache** is the non-default `result-cache` cargo feature (bead
  `sq-a9cn`): `ResultCache::new(capacity)` is a bounded, version-aware LRU that stores a SELECT/ASK
  `QueryResult` keyed by `(parsed query algebra, caller graph-version)`; serve a query through
  `cache.get_or_eval(&graph, &query, version, &budget)` (returns an `Arc<QueryResult>`). It re-serves
  the same read query against a slowly-changing graph without re-executing. **Soundness contract**:
  the engine evaluates against a borrowed `&Graph` and can't see mutations, so the **caller bumps the
  `u64` version on every mutation** (`apply_delta`/`update`/reload) — a hit is only returned for the
  current version. **Non-deterministic queries are never stored** — `NOW`/`RAND`/`UUID`/`STRUUID`/
  `BNODE`, a remote `SERVICE`, or any custom function / aggregate; `is_cacheable(&query)` reports this
  conservatively, and `get_or_eval` evaluates those fresh every time. Keying on the parsed algebra
  makes the cache insensitive to whitespace / comments / prefix spelling. `cache.stats()` exposes
  hit/miss/entry counts; `cache.clear()` drops all entries. When off, zero cache code compiles, the
  default build is byte-identical, and no new dependencies are added.

  ```rust
  // Cargo.toml: sparq-engine = { version = "0.1", features = ["result-cache"] }
  use sparq_engine::{PreparedQuery, QueryBudget, ResultCache};
  let cache = ResultCache::new(256);          // up to 256 distinct results, LRU
  let q = PreparedQuery::parse("SELECT ?s WHERE { ?s ?p ?o }")?.into_query();
  let mut version = 0u64;
  let r1 = cache.get_or_eval(&graph, &q, version, &QueryBudget::unlimited())?; // miss
  let r2 = cache.get_or_eval(&graph, &q, version, &QueryBudget::unlimited())?; // hit (same Arc)
  // ... mutate the graph, then bump the epoch so the next read re-evaluates:
  version += 1;
  let r3 = cache.get_or_eval(&graph, &q, version, &QueryBudget::unlimited())?; // miss (fresh)
  # Ok::<(), String>(())
  ```
- **Sharing one `Graph` across server threads** — a `sparq_core::Graph` (and its read-only
  `GraphSnapshot`) is **`Send + Sync`** (guaranteed by a compile-time assertion in `sparq-core`), so
  it can be shared across the async handlers of an axum/actix/tower server directly with
  `Arc<RwLock<Graph>>`. To skip that boilerplate, enable the non-default **`shared` cargo feature**
  on `sparq-core` (bead `sq-yj76l`, gh #1121) for `shared::SharedGraph` — a cheaply-cloneable handle
  (`SharedGraph::new(graph)` / `.clone()` is an `Arc` bump) with three access paths: `.snapshot()`
  hands out a cheap immutable point-in-time `GraphSnapshot` you query **lock-free** for the request
  (the recommended read-heavy path — the write lock is held only for the O(overlay) snapshot clone),
  while `.read()` / `.write()` give RAII `RwLock` guards for ad-hoc locked access. It is `std::sync`
  only (no new dependency) and off by default, so the lean default / wasm build pays nothing.

  ```rust
  // Cargo.toml: sparq-core = { version = "0.1", features = ["shared"] }
  use sparq_core::shared::SharedGraph;
  let shared = SharedGraph::new(graph);          // drop into axum `State`, clone per handler
  let snap = shared.snapshot();                  // cheap immutable view; query it lock-free
  let _n = snap.len();
  shared.write().apply_delta(&[], &[])?;          // exclusive write lock only for the mutation
  # Ok::<(), String>(())
  ```
- **MVCC / ACID transaction isolation** is the non-default `txn` cargo feature (bead `sq-it1x`):
  `txn::TransactionManager::new(graph)` owns one logical graph and hands out **snapshot-isolation**
  read transactions (`begin_read()` → a cheap point-in-time view that derefs to `&Graph`, immune to
  later commits) and serialized **write** transactions (`begin_write()` → a copy-on-write fork).
  `WriteTxn::update(sparql)` applies SPARQL `UPDATE`s to the private fork (read-your-own-writes via
  `w.graph()`); `commit()` returns the new `u64` version, or `CommitError::Conflict` under
  **first-committer-wins** if a concurrent writer published an overlapping write set since the txn
  began (the whole body is then discarded — atomic rollback). A stale but *non*-conflicting writer's
  resolved delta is replayed onto the current generation, so no commit is lost. For a single writer
  the conflict check never fires (SI = serializability); durability is inherited from a directory-
  backed graph's WAL. Built on the COW delta-overlay substrate; when off, zero txn code compiles and
  the default build is byte-identical (no new deps).

  ```rust
  // Cargo.toml: sparq-engine = { version = "0.1", features = ["txn"] }
  use sparq_engine::txn::{CommitError, TransactionManager};
  let m = TransactionManager::new(graph);
  // Snapshot-isolation read: this view never changes underneath us.
  let snap = m.begin_read();
  let _n = sparq_engine::count(&snap, "SELECT * WHERE { ?s ?p ?o }")?;
  // Write txn: fork, mutate the private copy, commit (first-committer-wins).
  let mut w = m.begin_write();
  w.update("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")?;
  match w.commit() {
      Ok(version) => println!("committed as v{version}"),
      Err(CommitError::Conflict { .. }) => { /* retry against the new state */ }
  }
  # Ok::<(), String>(())
  ```
- **Default cargo features** (`parallel`, `regex`, `digest`): `regex` powers REGEX/REPLACE; `digest`
  powers MD5/SHA*; `parallel` enables rayon scan/join/sort/aggregate. The **wasm** crate
  (`sparq-wasm`) disables defaults, so on `wasm32-unknown-unknown` REGEX/hash builtins and
  `QueryBudget::deadline` are absent (`max_rows` still works); UUID()/STRUUID() are native-only.
- **Composition**: `with_functions` and `with_view` install thread-locally (propagated into rayon
  workers), nest in either order, and uninstall on return/unwind. The registry is cheap to clone
  (`Arc`-shared) — build once, reuse across queries/threads.
- **Performance**: `query_json` avoids per-cell `Term` allocation; `count` avoids materialisation;
  prefer `PreparedQuery` when running one query against many graphs (e.g. continuous/RSP queries).
- **CLI/HTTP alternative**: `sparq-cli` and `sparq-server` (W3C SPARQL Protocol, `?explain=`,
  result formats JSON/XML/CSV/TSV, `/metrics`) wrap this same surface if you don't want to embed.
- **Apache Arrow columnar import/export** is the opt-in `sparq-arrow` crate (separate leaf crate +
  its own `arrow` feature, both OFF the default build — the `arrow-*` deps NEVER reach
  `sparq-core`/`sparq-engine`/wasm). `sparq_arrow::to_record_batch(&QueryResult) -> RecordBatch`
  projects a SELECT result into one Arrow **struct column per variable**, and
  `sparq_arrow::from_record_batch(&RecordBatch) -> Result<QueryResult, ArrowError>` is its
  checked inverse —
  `Struct<kind, value, datatype, language, direction>` (all nullable `Utf8`; the field names are
  `sparq_arrow::RDF_TERM_FIELDS`). Faithful & round-trippable: an **unbound** binding is a `null`
  struct slot (distinct from a bound empty string); `xsd:string` is written **explicitly** (not
  elided). Honest v1 boundaries — **no numeric narrowing** (`42^^xsd:integer` is the string `"42"`
  + a datatype field, not an `Int64`; a typed-column view is a follow-up) and **triple terms are
  stringified** to N-Triples in `value`. Import rejects malformed schemas, term kinds, and
  incompatible field combinations instead of guessing. The intended on-ramp into
  Polars/DuckDB/pandas; a
  `sparq-py` `Graph.query_arrow() -> pyarrow.Table` PyO3 binding is a follow-up (bead `sq-lt1ml`).

## See also

- `rust-parallel-parsing` / `fused-decompress-parse` — fast/compressed RDF ingest into a `Graph`.
- `hdt-format` — loading `.hdt` archives into a `Graph` (`sparq-hdt`).
- `sparq-arrow` — opt-in Apache Arrow columnar import/export between a SELECT `QueryResult`
  and a `RecordBatch` (one struct column per variable) for dataframe interoperability.
- `sparql-formal-semantics` — the algebra/semantics reference for the SPARQL fragment.
- `noir-circuit-patterns` / `verifiable-credentials-zk` / `mpc-protocols` — the ZK/MPC estate built
  on the `zk` trace seam (non-default `zk` feature; consumed by `sparq-zk`).
