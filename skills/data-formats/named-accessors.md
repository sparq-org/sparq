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
A directory-backed member keeps its manifest-recorded slot across sibling creates and
drops. A whole-image save, compaction or vacuum may assign a fresh layout. To use its contents in another dataset,
make an independent in-memory copy with `fork()`; mapped copies keep the original
store's lock while their mappings live.

A directory-backed root refuses in-memory insertion through `add_named`, `set_named`,
and nonempty `extend_named`. Create the slot with `ensure_named(&name)?` and insert
terms through `apply_delta_to_named(slot, &inserts, &deletes)?`. Use a dataset fork
when installing temporary views. A refused compaction retains the pending overlay
until a later swap publishes the new base.

## Durable layout and journals

Create and drop update only their own manifest entries. Removing an internal in-memory
attachment does not remove its declaration on disk; use durable drop methods to update the
persisted set. The public accessors refuse moving durable children. Writable opens load the
complete declared tree, including nested named graphs, before cleaning undeclared slots under
the exclusive directory lock. Cleanup preserves separate root stores, unmarked store images
and directories containing held locks. Read-only opens leave undeclared slots alone. A cleanup warning names any undeclared slot left in place.
An invalid declared layout refuses the open before cleanup removes any slot. A linked `named/`
container is refused. Layout invalidation records accumulate between whole-tree rewrites;
ordinary slot removals do not scan those records, and a whole-tree swap prunes them.

With `mmap`, `TxnOp::Insert { slot, triple }`, `Delete { slot, triple }`, `Create(name)` and
`Drop(name)` are the ordered journal vocabulary. `slot: None` names the default graph;
`Some(name)` names a named graph. `commit_txn_ops(&ops)` records one frame;
`commit_txn_ops_checked(&ops)` reports errors as `(possibly_written, message)`. The caller
materializes those operations through the durable graph methods, then calls `clear_txn()`.
If materialization fails, call `poison_txn()` and reopen the store before accepting more writes.
The existing tuple-based `commit_txn` methods remain available for triple-only operations.
Recovery reads the previous journal format as well as the operation format.

Dictionary metadata and named manifests written by this build use version 2; version 1 remains
readable. Journal format upgrades use the same durable file publication as saves.
Re-save a legacy single-file dictionary before atomic UPDATE journaling. Keep a backup
when an older executable must remain usable. `Graph::adopt_legacy_staging(path)` explicitly
adopts stem-based staging for a known store; use it only after confirming that staging belongs
to that store. Ordinary opens require recorded staging provenance for automatic recovery.
