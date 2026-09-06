---
name: rdf-wrapper
description: "Traverse sparq RDF graphs as native Rust objects with the opt-in sparq-wrapper crate: bind a focus Term to an owned or borrowed Store, follow outgoing/incoming NamedNode predicates with iterators, unwrap values, convert typed literals to str/i64/bool, mutate owned stores, and optionally use the unlanded distinct-result, typed-cardinality, literal-codec, typed-focus, and effective-change observation proposals. Use when Rust code should work with focus objects instead of raw triples or dictionary IDs; SHACL-to-Rust code generation is a later surface."
---

# Use sparq-wrapper

Add the opt-in crate explicitly:

```toml
[dependencies]
sparq-core = "0.1"
sparq-wrapper = "0.1"
oxrdf = "0.3"
```

Load a graph, borrow it, and traverse with typed predicates:

```rust
use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_wrapper::Store;

let graph = Graph::load_str(
    "@prefix ex: <http://example.org/> . ex:alice ex:knows ex:bob . ex:bob ex:age 42 .",
    "turtle",
)?;
let store = Store::borrowed(&graph);
let alice = NamedNode::new("http://example.org/alice")?;
let knows = NamedNode::new("http://example.org/knows")?;
let age = NamedNode::new("http://example.org/age")?;

let bob = store.node(alice).out(&knows).next().expect("friend");
assert_eq!(bob.out(&age).next().expect("age").as_i64()?, 42);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`.out()` and `.r#in()` return `NodeSet`, an `ExactSizeIterator<Item = Node>`;
the raw identifier is Rust's required spelling for a method named `in`.
Call `.values()` on a traversal to yield owned `oxrdf::Term`s. An absent focus
or predicate is valid and yields an empty iterator. `Node::dataset()` exposes a
borrowed dataset wrapper; `.graph()` is the raw `sparq_core::Graph` escape hatch.

Choose ownership deliberately:

- `Store::borrowed(&graph)` is read-only and tied to the graph's lifetime.
- `Store::owned(graph)` and `Store::new()` own the graph and allow
  `insert`/`remove`. Nodes borrow the store, so stop using them before a write
  and reacquire them afterwards.
- Traversal addresses the default graph in M1. Reach named graphs through the
  raw graph until a scoped-dataset surface lands.

Typed accessors are strict:

- `as_str()` accepts `xsd:string` and `rdf:langString`.
- `as_i64()` accepts the XML Schema integer family, enforces every derived
  datatype's exact bounds (`byte` through `unsignedLong`), then checks that the
  value is representable as `i64`.
- `as_bool()` accepts only `xsd:boolean`, including `true/false/1/0`.
- `as_typed_literal()` returns lexical form, datatype, and language.

All return `Result<_, AccessError>`; do not silently coerce a mismatched RDF
datatype.

Eleven explicitly experimental, default-off features track proposals that
remain unlanded in rdfjs/wrapper:

```toml
sparq-wrapper = { version = "0.1", features = [
  "proposed-async-events",
  "proposed-async-node",
  "proposed-async-store",
  "proposed-cardinality",
  "proposed-codecs",
  "proposed-distinct",
  "proposed-graph-scope",
  "proposed-graph-scope-events",
  "proposed-json",
  "proposed-observe",
  "proposed-typed-focus",
] }
```

The async events, async node, and graph-scope events features currently
expose reserved, empty modules; enabling them adds no API.
Every other proposal feature is implemented. `proposed-distinct` is exposed as
inherent `Dataset` methods in the crate root rather than through a `proposed::`
module. See the
[per-feature proposal status pages](references/README.md) for the
implemented and reserved feature inventory. <!-- [SONNET-4.6] sq-1rg2q.1 -->

`proposed-async-store` adds `sparq_wrapper::proposed::async_store` — the
wrapper shape over a store whose reads are not synchronous (an HTTP endpoint, a
Solid pod, an out-of-core on-disk index), based on rdfjs/wrapper
[issue #10](https://github.com/rdfjs/wrapper/issues/10) and
[draft PR #97](https://github.com/rdfjs/wrapper/pull/97). Implement
`AsyncStoreBackend` for the backend, then use `AsyncStore` exactly like `Store`.

`AsyncNode::out` / `AsyncNode::r#in` return a `NodeStream` that wraps each term
into an `AsyncNode` as it arrives: the first node is observable before the
backend finishes producing, and there is deliberately no `collect`. Building a
stream polls nothing; dropping one drops the backend stream, so the wrapper
never polls or drains it again. `NodeStream::next` is cancellation-safe — the
wrapper buffers nothing of its own.

Whether a dropped traversal also stops in-flight *remote* work is the backend's
half of the contract: `AsyncStoreBackend` requires an implementation to start no
I/O before the stream or future it returned is first polled, and to abandon that
work on drop. Honour it and a partially consumed remote result set is abandoned
rather than drained; a backend that instead spawns the request eagerly keeps it
running, because the wrapper holds no handle to it.

The crate depends on no async runtime and contains no executor: `TermStream` is
`futures_core::Stream` narrowed to `Result<Term, AsyncStoreError>` over
`std::task` alone, so any executor can drive it and a backend built on the
async ecosystem forwards to its own stream in one line. A `!Unpin` backend
stream should be exposed as `Pin<Box<S>>`, which implements `TermStream`.
`add`/`has`/`delete` validate the subject position synchronously (a literal
subject is rejected before the backend is asked to do anything) and return the
backend future, so the call site reads `store.add(s, p, o)?.await?`.
<!-- [SONNET-4.6] sq-1rg2q.8 -->

`proposed-graph-scope` adds a read-many/write-one `GraphScope` based on
rdfjs/wrapper draft PR #95. Its reads are the deduplicated projection of
exactly the named graphs supplied to `GraphScope::new`; call
`with_default_graph()` to include the default graph explicitly. Scoped nodes
retain the projection for chained `out`/`in` traversal, while node- or
scope-level `insert`/`remove` operations affect only the configured named write
graph and leave copies elsewhere untouched. <!-- [GPT-5.6] sq-1rg2q.6 -->

```rust
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_wrapper::proposed::graph_scope::GraphScope;

let mut graph = Graph::load_dataset(
    "<http://example.org/alice> <http://example.org/tag> \"rdf\" <http://example.org/g1> .\n\
     <http://example.org/alice> <http://example.org/tag> \"rdf\" <http://example.org/g2> .",
    "nquads",
)?;
let alice = NamedNode::new("http://example.org/alice")?;
let tag = NamedNode::new("http://example.org/tag")?;
let g1 = Term::NamedNode(NamedNode::new("http://example.org/g1")?);
let g2 = Term::NamedNode(NamedNode::new("http://example.org/g2")?);

let scope = GraphScope::new(&mut graph, [g1.clone(), g2], g1);
let alice = scope.node(alice);
assert_eq!(alice.out(&tag).len(), 1); // duplicate triple projected once
alice.insert(tag, Literal::new_simple_literal("rust"))?; // writes only g1
# Ok::<(), Box<dyn std::error::Error>>(())
```

`proposed-distinct` adds `Dataset::subjects_of` / `objects_of` and yields each
term once ([issue #25](https://github.com/rdfjs/wrapper/issues/25),
[draft PR #88](https://github.com/rdfjs/wrapper/pull/88)).
`proposed-cardinality` adds `Node::required_out` / `optional_out` and typed
`CardinalityError` data ([draft PR #89](https://github.com/rdfjs/wrapper/pull/89)).
Its `sparq_wrapper::proposed::cardinality` module also adds mapped
`required`, `optional`, and `many` reads plus the `live_mapped` write-through
collection ([issue #8](https://github.com/rdfjs/wrapper/issues/8),
[draft PR #92](https://github.com/rdfjs/wrapper/pull/92)).

Use the mapped reads when a property has an explicit RDF cardinality. The
required and optional variants wrap M1 `CardinalityError` data in
`CardinalityViewError`; `many` returns a `Vec`, preserving every distinct RDF
term even when two terms map to equal Rust values. A mapper error is returned
without changing the store. <!-- [GPT-5.6] sq-1rg2q.3 -->

```rust
use oxrdf::{Literal, NamedNode, Term};
use sparq_wrapper::proposed::cardinality::{live_mapped, required};
use sparq_wrapper::Store;

let mut store = Store::new();
let alice = NamedNode::new("http://example.org/alice")?;
let name = NamedNode::new("http://example.org/name")?;
let tag = NamedNode::new("http://example.org/tag")?;
store.insert(
    alice.clone(),
    name.clone(),
    Literal::new_simple_literal("Alice"),
)?;

let display_name = required(&store.node(alice.clone()), &name, |node| {
    node.as_str().map(str::to_owned)
})?;
assert_eq!(display_name, "Alice");

{
    let mut tags = live_mapped(
        &mut store,
        alice.clone(),
        tag,
        |node| node.as_str().map(str::to_owned),
        |value: &String| Ok::<Term, std::convert::Infallible>(
            Literal::new_simple_literal(value).into(),
        ),
    );
    assert!(tags.insert(&"rdf".to_owned())?);
    assert_eq!(tags.values()?, vec!["rdf"]);
    assert!(tags.remove(&"rdf".to_owned())?);
    assert!(tags.is_empty());
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`LiveMappedCollection` holds a mutable store borrow for its lifetime. Its
`values`, `len`, `is_empty`, and `contains` methods query current triples;
`insert`, `remove`, and `clear` write through. `insert` and `remove` return
`true` only for an effective graph change. Encoding runs before each mutation,
so an encoder error leaves all existing triples intact.

`proposed-observe` exposes a self-contained `proposed::observe::ObservableStore`
for the effective-change subscription proposals in rdfjs/wrapper draft PRs #93
and #94. Dataset callbacks receive a typed `ChangeEvent`; `LiveValues::subscribe`
filters by subject and predicate and maps the changed RDF object into an
application `ValueChange<T>`. Duplicate adds and absent deletes stay silent,
and callbacks receive the committed graph only after the mutable graph borrow
has ended. <!-- [GPT-5.6] sq-1rg2q.5 -->

```rust
use oxrdf::{Literal, NamedNode};
use sparq_wrapper::proposed::observe::{ChangeKind, ObservableStore};

let mut store = ObservableStore::new();
let alice = NamedNode::new("http://example.org/alice")?;
let tag = NamedNode::new("http://example.org/tag")?;
let subscription = store.subscribe(|event, committed| {
    assert!(matches!(event.kind, ChangeKind::Add | ChangeKind::Delete));
    let _committed_triple_count = committed.len();
});

let mut tags = store.live_values(alice, tag);
assert!(tags.insert(Literal::new_simple_literal("rdf"))?);
assert!(!tags.insert(Literal::new_simple_literal("rdf"))?);
drop(tags);
assert!(store.unsubscribe(subscription));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`proposed-codecs` exposes symmetric literal mappings in
`sparq_wrapper::proposed::codecs` ([issue #7](https://github.com/rdfjs/wrapper/issues/7),
[draft PR #90](https://github.com/rdfjs/wrapper/pull/90),
[draft PR #91](https://github.com/rdfjs/wrapper/pull/91)). `encode_i128` and
`decode_i128` round-trip the full Rust `i128` range as exact `xsd:integer`
literals. The decoder accepts only that exact datatype and returns
`CodecError::InvalidInteger` for malformed or out-of-range lexical forms.
Because `xsd:integer` fixes XML Schema's `whiteSpace` facet to `collapse`,
boundary whitespace is normalized away before the lexical-to-value mapping, so
`" 7"^^xsd:integer` decodes as `7` — matching how the query engine values a
padded numeric lexical — while interior whitespace such as `"+ 1"` is rejected.
`encode_lang_string` validates a BCP47 language tag and produces an
`rdf:langString`; `decode_lang_string` returns an owned `LangString` containing
both `value` and `language`, so a round trip cannot discard the tag. Datatype,
integer, language-tag, and missing-language failures are represented by the
typed `CodecError` variants. <!-- [GPT-5.6] sq-1rg2q.4 -->

```rust
use oxrdf::Literal;
use sparq_wrapper::proposed::codecs::{
    decode_i128, decode_lang_string, encode_i128, encode_lang_string, LangString,
};

let large = i128::from(i64::MAX) + 1;
let integer_literal = encode_i128(large);
assert_eq!(decode_i128(&integer_literal)?, large);

let label_literal = encode_lang_string("Y llyfrgellydd", "cy")?;
assert_eq!(
    decode_lang_string(&label_literal)?,
    LangString {
        value: "Y llyfrgellydd".to_owned(),
        language: "cy".to_owned(),
    },
);

let plain = Literal::new_simple_literal("not language-tagged");
assert!(decode_lang_string(&plain).is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

`proposed-typed-focus` adds the `sparq_wrapper::proposed::typed_focus` module.
Its `NodeFactory` binds one borrowed graph, store, or dataset view and can wrap
many terms without cloning the graph. Kind-specific constructors return a
`TypedNode` whose available traversals reflect the term's legal positions;
`NodeFactory::term` instead returns `AnyNode`, whose enum variant preserves the
concrete focus kind at run time. <!-- [GPT-5.6] sq-1rg2q.2 -->

```rust
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_wrapper::proposed::typed_focus::{AnyNode, NodeFactory};

let mut graph = Graph::new();
let alice = NamedNode::new("http://example.org/alice")?;
let name = NamedNode::new("http://example.org/name")?;
graph.insert_triple(
    alice.clone(),
    name.clone(),
    Literal::new_simple_literal("Alice"),
)?;

let factory = NodeFactory::new(&graph);
let subject = factory.iri(alice);
assert_eq!(subject.out(&name).len(), 1);

match factory.term(Term::Literal(Literal::new_simple_literal("Alice"))) {
    AnyNode::Literal(value) => assert_eq!(value.r#in(&name).len(), 1),
    _ => unreachable!("the factory preserves the concrete term kind"),
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Every typed focus supports incoming traversal because every RDF term may be an
object. Outgoing traversal is available only for `SubjectFocus` kinds, so code
such as `factory.literal(value).out(&predicate)` fails to compile. The
predicate-wide `subjects()` / `objects()` helpers are available only on the IRI
focus returned by `NodeFactory::iri`. Match an `AnyNode` variant to recover
those kind-specific methods, or call `into_node()` to erase the focus kind and
return to the untyped wrapper.

`proposed-json` adds `sparq_wrapper::proposed::json::JsonProjection`, which
projects a focus node and its outgoing reachable subgraph to one compact JSON
string ([open PR #23](https://github.com/rdfjs/wrapper/pull/23)). RDF graphs
cycle, so the projection never simply recurses: a node it has already met is
emitted as the reference `{"@ref": "<term>"}`, whose term is the same stable
identifier the expanded node carries in `@id` (an IRI as itself, any other term
in N-Triples form). `RepeatedFocus::OnCycle`, the default, references only
ancestors of the node being written, so a diamond is expanded once per path;
`RepeatedFocus::OnRepeat` references every node expanded earlier in the
document, so each node is expanded at most once. `with_max_depth` bounds
recursion depth (default `DEFAULT_MAX_DEPTH`), truncating to the same reference
form. <!-- [SONNET-4.6] sq-1rg2q.11 -->

Output is deterministic — predicates in lexicographic IRI order, each
predicate's objects in lexicographic N-Triples order — so projecting the same
store twice is byte-identical and the result is diffable. Literals are value
objects that keep their metadata: `@value` plus `@type` for a typed literal, or
`@language` (plus `@direction` for an RDF 1.2 directional literal) for a
language-tagged one, whose datatype the tag implies. No value is coerced to a
bare JSON string, number, or boolean, so `"1"^^xsd:integer` and `"1"` stay
distinguishable.

```rust
use oxrdf::NamedNode;
use sparq_core::Graph;
use sparq_wrapper::proposed::json::JsonProjection;
use sparq_wrapper::Store;

let graph = Graph::load_str(
    "@prefix ex: <http://example.org/> .\n\
     ex:a ex:knows ex:b .\n\
     ex:b ex:knows ex:a ; ex:label \"Bee\"@en .",
    "turtle",
)?;
let store = Store::borrowed(&graph);
let a = NamedNode::new("http://example.org/a")?;

let json = JsonProjection::new().project(&store.node(a.clone()));
assert_eq!(json, JsonProjection::new().project(&store.node(a)));
assert!(json.contains(r#"{"@ref":"http://example.org/a"}"#)); // cycle closed
assert!(json.contains(r#"{"@value":"Bee","@language":"en"}"#)); // tag kept
# Ok::<(), Box<dyn std::error::Error>>(())
```

Only outgoing predicates in the default graph are followed, matching `Node::out`.
A focus absent from the graph projects to a node object with no predicates, and
a literal focus projects to its value object, so the call is total for any term.

SHACL-to-Rust struct generation is not part of M1. Reuse `sparq-shacl`'s
`ShapesModel` for that work; do not invent a second SHACL parser.
