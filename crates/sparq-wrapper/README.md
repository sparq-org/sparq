# sparq-wrapper

Rust-native RDF object traversal over `sparq-core`: bind a focus term to a
store, follow predicates, and convert literal values without handling raw
dictionary IDs or triples. The crate is opt-in; `sparq-core` and
`sparq-engine` do not depend on it.

> Model: GPT-5.6 [GPT-5.6] (sq-1rg2q M1).

## 🚀 Quickstart

```rust
use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_wrapper::Store;

let graph = Graph::load_str(
    "@prefix ex: <http://example.org/> . ex:alice ex:knows ex:bob . ex:bob ex:name \"Bob\" .",
    "turtle",
)?;
let store = Store::borrowed(&graph);
let alice = NamedNode::new("http://example.org/alice")?;
let knows = NamedNode::new("http://example.org/knows")?;
let name = NamedNode::new("http://example.org/name")?;

let bob = store.node(alice).out(&knows).next().expect("friend");
assert_eq!(bob.out(&name).next().expect("name").as_str()?, "Bob");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `Store::borrowed(&graph)` for a read-only wrapper, or
`Store::owned(graph)` / `Store::new()` when the wrapper should own and mutate
the graph. A `Node<'a>` borrows the graph, owns its focus `Term`, and cannot
outlive or overlap a mutable borrow of its store. Reacquire nodes after writes.
The default wrapper surface traverses the default graph; enable
`proposed-graph-scope` for an explicit named-graph read projection and one
named-graph write target. Use `node.dataset().graph()` as the raw-graph escape
hatch.

## ✨ Features

- `.out(&predicate)` and `.r#in(&predicate)` return exact-size Rust iterators
  of wrapped nodes; `.values()` unwraps a traversal to RDF terms. The raw
  identifier is Rust's required spelling for a method named `in`.
- `as_str`, `as_i64`, `as_bool`, and `as_typed_literal` validate the RDF
  datatype and return typed `Result` errors.
- Owned stores expose predicate-typed insert/remove operations; borrowed
  stores reject mutation with `StoreError::Borrowed`.
- `proposed-distinct` adds distinct dataset helpers based on rdfjs/wrapper
  issue #25 and draft PR #88. This proposal is not landed upstream.
- `proposed-cardinality` adds required/optional singular traversal with typed
  cardinality errors based on rdfjs/wrapper draft PR #89. It also exposes the
  `proposed::cardinality::{required, optional, many}` mapped views and
  `live_mapped` write-through collections based on rdfjs/wrapper issue #8 and
  draft PR #92. Live collection reads re-query the store, and value-to-term
  conversion completes before mutation so conversion failures leave the graph
  unchanged. This proposal is not landed upstream. <!-- [GPT-5.6] sq-1rg2q.3 -->
- `proposed-graph-scope` adds `proposed::graph_scope::GraphScope`, an explicit
  deduplicated read projection over selected named graphs (plus the default
  graph when requested) whose insert/remove operations target one configured
  named graph. This proposal follows rdfjs/wrapper draft PR #95 and is not
  landed upstream. <!-- [GPT-5.6] sq-1rg2q.6 -->
- `proposed-async-store` adds `proposed::async_store::AsyncStore`, the same
  focus/traverse shape over a remote or disk-backed backend. Traversal streams
  each term as a wrapped node instead of collecting a result set, dropping a
  stream drops the backend stream and never polls it again (which cancels the
  traversal for a backend honouring the trait's laziness and drop-cancellation
  contract), and no async runtime is pulled in. This
  proposal follows rdfjs/wrapper issue #10 and draft PR #97 and is not landed
  upstream. <!-- [SONNET-4.6] sq-1rg2q.8 -->
- `proposed-json` adds `proposed::json::JsonProjection`, a JSON projection of a
  focus node and its outgoing reachable subgraph that is total on cyclic graphs
  (a repeated node becomes a `{"@ref": …}` term reference under an explicit
  `RepeatedFocus` policy) and deterministic (predicates sorted by IRI, objects
  by N-Triples form), and that keeps each literal's datatype, language tag, and
  base direction. This proposal follows rdfjs/wrapper open PR #23 and is not
  landed upstream. <!-- [SONNET-4.6] sq-1rg2q.11 -->
- The reserved `proposed-graph-scope-events`, `proposed-async-node`, and
  `proposed-async-events` seams are default-off placeholders. Their APIs are
  intentionally empty until the corresponding proposal work lands.
  <!-- [SONNET-4.6] sq-1rg2q.1 -->
- All crate features are off by default, and the dependency on `sparq-core`
  disables its default features to keep this capability isolated.

## 📚 Learn more

- [`skills/rdf-wrapper/SKILL.md`](../../skills/rdf-wrapper/SKILL.md) — usage
  recipes and the feature matrix.
- [`docs/proposed/README.md`](docs/proposed/README.md) — status index for each
  default-off proposal feature.
- [rdfjs/wrapper](https://github.com/rdfjs/wrapper) — object-mapping prior art.
- [Grapoi](https://github.com/rdf-ext/grapoi) and
  [Clownface](https://github.com/zazuko/clownface) — traversal ergonomics.
- SHACL-to-Rust object generation is decomposed after M1; `sparq-shacl` and
  `sparq-forms` provide the in-repo shape model and derivation precedents.

## License

MIT — see the workspace root `LICENSE`.
