//! [OPUS-4.8] (sq-5lf) Tests for the public cheap-snapshot API `Graph::snapshot()` →
//! `GraphSnapshot`. The structural-fork MECHANISM (read-merge correctness across a
//! chain of forked generations) is covered exhaustively in `fork_differential.rs`;
//! this file pins the snapshot SURFACE the task specifies:
//!   1. a snapshot reads exactly the triples present at snapshot time (point-in-time);
//!   2. later mutation of the base is invisible to the snapshot, AND mutation of the
//!      snapshot (via `into_graph`) is invisible to the base — copy-on-write isolation;
//!   3. the snapshot is genuinely CHEAP — it `Arc`-shares the base permutation indexes
//!      (proved by a strong-count bump, not an index-memory duplication);
//!   4. `GraphSnapshot` is `Send + Sync` and derefs to a fully-queryable `&Graph`.

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::{Graph, GraphSnapshot};

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s.to_string()))
}

/// Sorted term-level dump of the default graph — the equality oracle.
fn dump(g: &Graph) -> Vec<[String; 3]> {
    let mut v: Vec<[String; 3]> = g
        .iter_ids()
        .map(|t| {
            [g.dict.term(t[0]).to_string(), g.dict.term(t[1]).to_string(), g.dict.term(t[2]).to_string()]
        })
        .collect();
    v.sort();
    v
}

fn graph_of(triples: &[[&str; 3]]) -> Graph {
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[Id; 3]> = triples
        .iter()
        .map(|[s, p, o]| [dict.intern(&iri(s)), dict.intern(&iri(p)), dict.intern(&iri(o))])
        .collect();
    Graph::from_parts(dict, ids)
}

/// Compile-time proof that `GraphSnapshot` is `Send + Sync` (the task's contract: a
/// snapshot is publishable across threads).
#[test]
fn snapshot_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GraphSnapshot>();
}

/// (1) A snapshot reads exactly the triples present when it was taken.
#[test]
fn snapshot_sees_point_in_time_triples() {
    let g = graph_of(&[
        ["http://ex/a", "http://ex/p", "http://ex/b"],
        ["http://ex/c", "http://ex/p", "http://ex/d"],
    ]);
    let snap = g.snapshot();

    // Deref gives the full read API.
    assert_eq!(snap.len(), 2);
    assert_eq!(dump(&snap), dump(&g));
    assert!(snap.id_of(&iri("http://ex/a")).is_some());
    assert!(snap.id_of(&iri("http://ex/absent")).is_none());

    // A pattern scan through the deref answers identically to the source.
    let pat = snap.pattern(None, Some(&NamedNode::new_unchecked("http://ex/p")), None).unwrap();
    assert_eq!(snap.store.scan(&pat).rows.len(), 2);
}

/// (2a) After the snapshot is taken, mutating the BASE must NOT be visible to the
/// snapshot — the snapshot stays frozen at its point-in-time state.
#[test]
fn base_mutation_after_snapshot_invisible_to_snapshot() {
    let mut g = graph_of(&[["http://ex/a", "http://ex/p", "http://ex/b"]]);
    let snap = g.snapshot();
    let snap_dump = dump(&snap);
    assert_eq!(snap.len(), 1);

    // Insert into the base AFTER snapshotting.
    g.apply_delta(&[[iri("http://ex/x"), iri("http://ex/p"), iri("http://ex/y")]], &[]).unwrap();
    assert_eq!(g.len(), 2, "base must see its own insert");

    // ...and delete the original base triple from the base.
    g.apply_delta(&[], &[[iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")]]).unwrap();
    assert_eq!(g.len(), 1, "base must see its own delete");

    // The snapshot is untouched by either mutation.
    assert_eq!(snap.len(), 1, "snapshot len drifted after base mutation");
    assert_eq!(dump(&snap), snap_dump, "snapshot content drifted after base mutation");
    assert!(snap.id_of(&iri("http://ex/x")).is_none(), "base-inserted term leaked into snapshot");
    assert!(snap.store.contains([
        snap.id_of(&iri("http://ex/a")).unwrap(),
        snap.id_of(&iri("http://ex/p")).unwrap(),
        snap.id_of(&iri("http://ex/b")).unwrap(),
    ]), "base-deleted triple vanished from the snapshot");
}

/// (2b) Conversely, mutating the snapshot's graph (via `into_graph`) must NOT leak
/// into the base — copy-on-write in the other direction.
#[test]
fn snapshot_mutation_invisible_to_base() {
    let g = graph_of(&[["http://ex/a", "http://ex/p", "http://ex/b"]]);
    let base_dump = dump(&g);

    let mut copy = g.snapshot().into_graph();
    copy.apply_delta(&[[iri("http://ex/n"), iri("http://ex/p"), iri("http://ex/m")]], &[]).unwrap();
    copy.apply_delta(&[], &[[iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")]]).unwrap();

    assert_eq!(copy.len(), 1, "the copy applied its own insert+delete");
    assert!(copy.id_of(&iri("http://ex/n")).is_some());
    // The base is exactly as it was.
    assert_eq!(dump(&g), base_dump, "snapshot/copy mutation leaked into the base");
    assert_eq!(g.len(), 1);
}

/// (3) The snapshot is genuinely cheap: it `Arc`-SHARES the base permutation indexes
/// rather than duplicating them. Proved structurally via the base strong-count and by
/// the index heap footprint not growing per snapshot.
#[test]
fn snapshot_shares_base_indexes_no_duplication() {
    // A base large enough that an O(triples) deep clone would be obvious.
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[Id; 3]> = (0..50_000u32)
        .map(|i| {
            [
                dict.intern_iri(&format!("http://ex/s{}", i % 5000)),
                dict.intern_iri(&format!("http://ex/p{}", i % 16)),
                dict.intern_iri(&format!("http://ex/o{i}")),
            ]
        })
        .collect();
    let g = Graph::from_parts(dict, ids);

    let base_index_bytes = g.store.heap_bytes();
    let count_before = g.store.base_strong_count();
    assert_eq!(count_before, 1, "an unforked base owns its perms alone");

    // Take several snapshots; each must bump the shared-base strong count by exactly one
    // and must NOT duplicate the (large) index memory.
    let s1 = g.snapshot();
    assert_eq!(g.store.base_strong_count(), 2, "snapshot must Arc-share the base, not copy it");
    assert_eq!(s1.store.base_strong_count(), 2, "the snapshot points at the SAME shared base");
    assert!(
        s1.store.heap_bytes() <= base_index_bytes,
        "snapshot index memory ({}) exceeds the shared base ({}) — it duplicated the indexes",
        s1.store.heap_bytes(),
        base_index_bytes
    );

    let s2 = g.snapshot();
    let s3 = g.snapshot();
    assert_eq!(g.store.base_strong_count(), 4, "three live snapshots => base refcount 4");

    // Dropping snapshots releases the shared reference (no leak, no copy lingering).
    drop(s2);
    drop(s3);
    assert_eq!(g.store.base_strong_count(), 2, "dropping snapshots releases the shared base");
    drop(s1);
    assert_eq!(g.store.base_strong_count(), 1, "base is sole owner again after all snapshots drop");

    // The taking of N snapshots cost no index duplication: total resident index bytes for
    // a fresh snapshot equals the base (the Arc is shared) — sanity re-check after drops.
    let s = g.snapshot();
    assert_eq!(s.store.heap_bytes(), base_index_bytes, "shared base reports its own (single) index memory");
}

/// A snapshot taken AFTER the base already carries an overlay sees the merged
/// (base + overlay) state — point-in-time including pending updates — and stays frozen
/// there when the base later compacts.
#[test]
fn snapshot_includes_pending_overlay_and_survives_base_compaction() {
    let mut g = graph_of(&[["http://ex/a", "http://ex/p", "http://ex/b"]]);
    g.apply_delta(&[[iri("http://ex/c"), iri("http://ex/p"), iri("http://ex/d")]], &[]).unwrap();
    assert!(g.store.has_overlay());

    let snap = g.snapshot(); // captures the 2-triple merged state
    assert_eq!(snap.len(), 2);
    let snap_dump = dump(&snap);

    // Base compacts (folds the overlay) and then diverges further.
    g.compact().unwrap();
    assert!(!g.store.has_overlay());
    g.apply_delta(&[], &[[iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")]]).unwrap();
    assert_eq!(g.len(), 1);

    // Snapshot is still the point-in-time 2-triple state.
    assert_eq!(snap.len(), 2, "snapshot drifted across base compaction");
    assert_eq!(dump(&snap), snap_dump);
}

/// Named graphs are included in a snapshot and isolated from later base mutation.
#[test]
fn snapshot_covers_named_graphs() {
    let src = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
               <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .";
    let mut g = Graph::load_dataset(src, "nquads").unwrap();
    let snap = g.snapshot();
    assert_eq!(snap.named.len(), 1);
    assert_eq!(snap.named[0].1.len(), 1);

    // Mutate the base's named graph after the snapshot.
    g.named[0]
        .1
        .apply_delta(&[[iri("http://ex/n"), iri("http://ex/q"), iri("http://ex/m")]], &[])
        .unwrap();
    assert_eq!(g.named[0].1.len(), 2, "base named graph saw its insert");
    assert_eq!(snap.named[0].1.len(), 1, "snapshot named graph leaked a base mutation");
}
