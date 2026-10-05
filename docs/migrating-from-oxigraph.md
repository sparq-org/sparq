<!-- [OPUS-4.8] sq-d1axm: migration guide (#1128); folds the oxrdf-version matrix (#1124) and links the result-shape diff (#1123). -->
# Migrating from Oxigraph to sparq

sparq and [Oxigraph](https://github.com/oxigraph/oxigraph) share the same RDF
term foundation — the **`oxrdf`** crate — so the *data model* (IRIs, blank nodes,
literals, named graphs) is byte-for-byte interoperable. What differs is the
**store API surface** and the **query-result shape**. This guide maps the common
Oxigraph operations to their sparq equivalents so a port is mechanical rather
than trial-and-error.

It targets the **Rust API** (`sparq-core` + `sparq-engine`). The JavaScript/WASM
surface is covered separately in [`js/README.md`](../js/README.md) and the
[`javascript-wasm`](../skills/javascript-wasm/SKILL.md) skill; the WASM-build
differences are in [§ WASM](#wasm-compilation).

> sparq is an **in-memory, immutable-index** engine optimised for load-once /
> query-many over a fixed graph, with an append-only delta overlay for
> incremental mutation. Oxigraph is a **persistent, transactional** store
> (RocksDB or in-memory) with full SPARQL 1.1 Update transactions. They are not
> drop-in replacements: if you need durable on-disk storage or MVCC
> transactions, sparq does not (yet) provide them — see
> [§ What sparq does NOT replace](#what-sparq-does-not-replace).

## oxrdf version compatibility (#1124)

sparq's workspace pins `oxrdf = { version = "0.3", … }`, which **resolves to
`oxrdf 0.3.3`** — *exactly* the version Oxigraph 0.5 depends on. So `oxrdf::Term`
/ `NamedNode` / `Literal` / `Quad` values flow between an Oxigraph 0.5 store and
sparq without conversion: there is **no type-incompatibility gap to bridge**, and
no version bump is required on either side.

Verify the resolved version in any checkout with:

```sh
cargo tree -p sparq-core -i oxrdf
# oxrdf v0.3.3
```

| Crate            | oxrdf line | Resolves to | Notes |
| ---------------- | ---------- | ----------- | ----- |
| sparq (workspace) | `0.3`      | **0.3.3**   | `features = ["rdf-12"]` (RDF 1.2 / triple terms) |
| Oxigraph 0.5      | `0.3`      | **0.3.3**   | same minor, same patch |
| sparq-canon (only) | `0.2.4`   | 0.2.4       | a *separate, isolated* dep — the URDNA2015 bridge speaks `oxrdf 0.2`; it is internal to canonicalisation and does not touch the public term model |

The lone `oxrdf 0.2.4` in the tree (`sparq-canon`'s RDFC-1.0 bridge) is
deliberate and quarantined; it never appears in `sparq-core` / `sparq-engine`
public signatures. Your migration only ever sees `oxrdf 0.3.3` types.

## Store initialisation

Oxigraph builds a mutable `Store` you insert into; sparq parses a document (or a
batch of quads) into an immutable, fully-indexed `Graph` in one shot.

```rust
// Oxigraph
use oxigraph::store::Store;
let store = Store::new()?;                       // empty, mutable
store.load_from_reader(RdfFormat::Turtle, turtle.as_bytes())?;

// sparq
use sparq_core::Graph;
let graph = Graph::load_str(turtle, "turtle")?;  // parse + index in one call
// named graphs preserved (N-Quads / TriG / JSON-LD @graph):
let dataset = Graph::load_dataset(nquads, "nquads")?;
```

`Graph::load_str` folds named graphs into the default graph;
`Graph::load_dataset` preserves them as queryable named graphs (the input must be
a quad-bearing format). An **unrecognised `format` is an `Err`** — it is not
silently parsed as Turtle. Accepted format strings and their aliases match the
JS/WASM surface; see [§ Format strings](#format-strings).

## Quad insert / delete

Oxigraph mutates per-quad through a transaction. sparq applies a **batch delta**
through its overlay (`apply_delta`) — deletes first, then inserts, O(batch), no
index rebuild — or rebuilds via SPARQL Update.

```rust
// Oxigraph
store.insert(&quad)?;
store.remove(&quad)?;

// sparq — batch delta over the overlay (oxrdf Terms, same types)
use oxrdf::{NamedNode, Term};
let s = Term::NamedNode(NamedNode::new("http://ex/s")?);
let p = NamedNode::new("http://ex/p")?;            // predicate
let o = Term::Literal("o".into());
graph.apply_delta(&[[s, Term::NamedNode(p), o]], &[])?; // inserts, no deletes

// or via SPARQL 1.1 Update (rebuilds the immutable index, returns a new Graph)
let updated = sparq_engine::update(&graph, "INSERT DATA { <http://ex/s> <http://ex/p> \"o\" }")?;
// in place (delta overlay; no rebuild):
sparq_engine::update_in_place(&mut graph, "DELETE DATA { <http://ex/s> <http://ex/p> \"o\" }")?;
```

The overlay is append-only: the dictionary only grows and deletes are tombstones
until the graph is reloaded. This is the right shape for ingest-then-query and
incremental top-ups, **not** for a high-churn transactional workload — that is an
Oxigraph strength sparq does not match.

## Query result processing (#1123)

This is the **biggest porting difference**. Oxigraph yields an iterator of
per-solution maps; sparq returns a single columnar `QueryResult { vars, rows }`.

```rust
// Oxigraph — iterator of QuerySolution, lookup by Variable/name
if let QueryResults::Solutions(solutions) = store.query("SELECT ?s ?o WHERE { ?s ?p ?o }")? {
    for sol in solutions {
        let sol = sol?;
        let s = sol.get("s");       // Option<&Term>, keyed by variable
        let o = sol.get("o");
    }
}

// sparq — columnar: a vars header + rows of positional Option<Term>
use sparq_engine::query;
let res = query(&graph, "SELECT ?s ?o WHERE { ?s ?p ?o }")?;
// res.vars : Vec<Variable>            — column order
// res.rows : Vec<Vec<Option<Term>>>   — one row per solution; None = unbound
let s_col = res.vars.iter().position(|v| v.as_str() == "s").unwrap();
for row in &res.rows {
    let s: Option<&Term> = row[s_col].as_ref();
}
```

The sparq shape is:

```rust
pub struct QueryResult {
    pub vars: Vec<oxrdf::Variable>,            // column header (order matters)
    pub rows: Vec<Vec<Option<oxrdf::Term>>>,   // None == unbound in that position
}
```

`ASK` is a separate entry point returning a `bool`
(`sparq_engine::ask(&graph, sparql)`), and `CONSTRUCT` / `DESCRIBE` return
triples (`sparq_engine::construct(&graph, sparql) -> Vec<oxrdf::Triple>`, or
`construct_ntriples` for a serialised string) rather than going through
`QueryResult`.

To recover an Oxigraph-style "lookup by variable name" per row, build a
name→index map once from `vars` and index `row[idx]` — a few lines, applied at
the one call site, instead of the per-row `HashMap` Oxigraph materialises. A
first-class iterator-of-bindings adapter is tracked as a follow-up (linked to
issue #1123); the columnar shape is intentional (it avoids a per-row map
allocation on the hot path), so the adapter will be an *additional* accessor,
not a replacement.

## Named graphs

Both stores model named graphs as a fourth quad position over the same `oxrdf`
terms. The difference is loading and querying:

```rust
// sparq — preserve named graphs at load, then GRAPH-aware SPARQL
let ds = Graph::load_dataset(nquads, "nquads")?;     // or "trig"
let res = query(&ds, "SELECT ?g ?s WHERE { GRAPH ?g { ?s ?p ?o } }")?;
```

`GRAPH <iri>` / `GRAPH ?g`, `FROM` / `FROM NAMED`, and `GRAPH` blocks in Update
all work over a dataset-loaded `Graph`. Note `Graph::len()` reports the
**default graph** only; count a dataset with a `GRAPH ?g` query. The
default-graph fold of `load_str` matches Oxigraph's behaviour when you load
quads into the default graph explicitly.

## Serialization

Oxigraph's `store.dump_to_writer(format, …)` maps to the `sparq_engine::serialize`
functions, which take a `&Graph` and return a `String`:

| Output | sparq function |
| ------ | -------------- |
| Turtle (default graph) | `serialize::graph_to_turtle(&g)` / `graph_to_turtle_pretty(&g)` |
| N-Triples | `construct::construct_ntriples` (result graph) or the Turtle writer |
| N-Quads (whole dataset) | `serialize::graph_to_nquads(&g)` |
| TriG (whole dataset) | the pretty-TriG writer (`graph_to_turtle_pretty_with` family) |
| JSON-LD 1.1 (expanded / flattened / compacted) | `serialize::graph_to_jsonld(&g, form)` / `graph_to_jsonld_compact(&g, ctx)` |

The JSON-LD writers are dependency-free (they reuse the existing oxrdf term
infrastructure). The same writer matrix is what the CLI, the HTTP server, and the
WASM `Store.serialize` binding all call, so output is consistent across surfaces.

## WASM compilation

sparq's `oxrdf 0.3.3` (and therefore Oxigraph 0.5's) pulls `rand` → `getrandom`
for blank-node id generation. On `wasm32-unknown-unknown` there is **no default
OS RNG**, so a wasm build of *any* crate in this family must select getrandom's
browser backend, exactly as Oxigraph's own wasm guidance requires. sparq's
`sparq-wasm` bundle does this with:

```toml
# the final bundle crate's Cargo.toml
getrandom = { version = "0.3", features = ["wasm_js"] }
```

```toml
# .cargo/config.toml (workspace root), for target wasm32-unknown-unknown
[target.wasm32-unknown-unknown]
rustflags = ['--cfg', 'getrandom_backend="wasm_js"']
```

With that in place the engine compiles to wasm32 cleanly. The published
`@sparq-org/sparq` npm package ships this prebuilt — for a JS migration you do not
touch the toolchain at all; see [`js/README.md`](../js/README.md). For
`sparq-solid` specifically, the wasm status (compiles; one runtime caveat) is
documented in [its README](../crates/sparq-solid/README.md#wasm-support).

### Format strings

`Graph::load_str` / `load_dataset` (and the WASM `Store.load`) accept these
case-sensitive format strings and aliases (an unrecognised value errors):

| Format | Accepted strings |
| ------ | ---------------- |
| Turtle | `turtle`, `ttl`, `text/turtle`, `application/turtle` |
| N-Triples | `ntriples`, `n-triples`, `nt`, `application/n-triples` |
| N-Quads | `nquads`, `n-quads`, `nq`, `application/n-quads` |
| TriG | `trig`, `application/trig` |
| JSON-LD | `jsonld`, `json-ld`, `application/ld+json` (opt-in `jsonld` feature) |

## What sparq does NOT replace

Being honest about the boundary so a migration does not hit a wall:

- **No durable on-disk store / transactions.** sparq's `Graph` is in-memory with
  an append-only overlay; there is no RocksDB-style persistence layer or
  MVCC/ACID transaction API. Oxigraph keeps that.
- **No `REGEX` / `REPLACE` in the default build** (the engine's non-default
  `regex` cargo feature is off, and it is off in the lean wasm bundle to keep the
  binary small). Use `CONTAINS` / `STRSTARTS` / `STRENDS` or a `--features regex`
  build.
- **Federated `SERVICE`** is native-only and not exposed at the JS/WASM wrapper
  layer.
- The **delta overlay is not a transaction manager** — it is an append-only
  optimisation for incremental load, not concurrent multi-writer isolation.

If those are load-bearing for your project, keep Oxigraph for the storage layer
and use sparq for the load-once / query-many analytical path — the shared
`oxrdf 0.3.3` term model means quads cross the boundary for free.

## See also

- [`js/README.md`](../js/README.md) — the JavaScript/WASM API surface.
- [`skills/javascript-wasm/SKILL.md`](../skills/javascript-wasm/SKILL.md) — the
  WASM `Store` / `SparqStore` reference (init, format strings, errors, `.free()`).
- [`crates/sparq-solid/README.md`](../crates/sparq-solid/README.md) — Solid Pod
  access control (WAC/ACP), incl. its wasm-support status.
- #1123 (result-shape adapter), #1124 (oxrdf alignment), this guide (#1128).
