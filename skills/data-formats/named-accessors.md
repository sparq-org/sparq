# Named graph accessors

```rust
use sparq_core::Graph;
use oxrdf::{NamedNode, Term};

let name: Term = NamedNode::new("http://ex/graph").unwrap().into();
let mut dataset = Graph::new();
dataset.add_named(name.clone(), Graph::new()).unwrap();
assert_eq!(dataset.named().len(), 1);
assert!(dataset.named_graph(&name).is_some());
dataset.apply_delta_to_named(0, &[], &[]).unwrap();
let removed = dataset.remove_named(&name).unwrap().unwrap();
dataset.set_named(name, removed).unwrap();
let members = dataset.take_named().unwrap();
dataset.extend_named(members).unwrap();
dataset.retain_named(|_, graph| !graph.is_empty()).unwrap();
```

`set_named` returns the replaced graph, if any. `extend_named` validates the
whole batch before insertion. `remove_named` and `retain_named` preserve order.
A directory-backed member stays in its positional slot until a durable operation
updates the manifest and its files. To use its contents in another dataset,
make an independent in-memory copy with `fork()`; mapped copies keep the original
store's lock while their mappings live.

A directory-backed root refuses in-memory insertion through `add_named`, `set_named`,
and nonempty `extend_named`. Create the slot with `ensure_named(&name)?` and insert
terms through `apply_delta_to_named(slot, &inserts, &deletes)?`. Use a dataset fork
when installing temporary views. A refused compaction retains the pending overlay
until a later swap publishes the new base.
