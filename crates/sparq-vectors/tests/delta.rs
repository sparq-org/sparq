//! (sq-pi44) Incremental delta-sidecar tests for `VectorStore`: add / remove / update
//! against an already-finalized store (no full rebuild), `compact` result-equivalence to a
//! from-scratch rebuild, and the generation-tie staleness guard.
//!
//! The whole file is gated on the opt-in `delta` feature — with it off the file compiles to an
//! empty test crate (the add/remove/update/compact APIs do not exist), mirroring the crate's
//! `filtered-ann` / `vec-predicate` test gating. Style matches the crate's other test modules
//! (the workspace's intentionally-deferred reformat keeps a compact hand-written layout).
#![cfg(feature = "delta")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{nearest_exact, nearest_term_exact, Fingerprint, VectorDelta, VectorStore};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sparq_delta_{tag}_{}_{n}.spqv", std::process::id()))
}

fn graph(ttl: &str) -> Graph {
    Graph::load_str(ttl, "turtle").expect("load test turtle")
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

fn id_of(g: &Graph, s: &str) -> u32 {
    g.id_of(&iri(s)).unwrap()
}

const A: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:alice ex:knows ex:bob .
    ex:bob ex:knows ex:carol .
    ex:carol ex:knows ex:dave .
"#;
// B has a DIFFERENT dictionary AND a different triple count — a different graph generation.
const B: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:eve ex:likes ex:frank .
"#;

/// Builds a finalized 4-d store over graph A: alice nearest bob, carol far.
fn build_finalized(g: &Graph, path: &std::path::Path) -> VectorStore {
    let mut s = VectorStore::create(path, 4).unwrap().with_fingerprint(g);
    s.put(id_of(g, "http://example.org/alice"), &[1.0, 0.0, 0.0, 0.0]).unwrap();
    s.put(id_of(g, "http://example.org/bob"), &[0.9, 0.1, 0.0, 0.0]).unwrap();
    s.put(id_of(g, "http://example.org/carol"), &[0.0, 0.0, 0.0, 1.0]).unwrap();
    s.finalize().unwrap();
    s
}

// ---------------------------------------------------------------- gate (a): add after finalize

#[test]
fn add_after_finalize_is_found_by_search() {
    let g = graph(A);
    let path = tmp("add");
    let mut store = build_finalized(&g, &path);
    let dave = id_of(&g, "http://example.org/dave");
    let bob = id_of(&g, "http://example.org/bob");

    // `put` is still rejected after finalize (the immutable-file contract is preserved)…
    assert!(
        store.put(dave, &[0.95, 0.05, 0.0, 0.0]).is_err(),
        "put after finalize must still error — the delta API is the additive path"
    );

    // …but `add` writes into the in-RAM delta against the finalized store.
    store.add(dave, &[0.99, 0.01, 0.0, 0.0]).unwrap();
    assert!(store.has_delta());
    assert_eq!(store.len(), 4, "the appended vector counts in len");
    assert_eq!(store.get(dave), Some(&[0.99f32, 0.01, 0.0, 0.0][..]), "get reads the delta vector");

    // Exact search (which consumes iter()) now returns the added vector, ranked correctly. alice is
    // the exact query (cosine 1.0) so ranks first; dave (0.99, 0.01) outranks bob (0.9, 0.1).
    let q = [1.0f32, 0.0, 0.0, 0.0];
    let ids: Vec<u32> = nearest_exact(&store, &q, 4).iter().map(|&(id, _)| id).collect();
    assert!(ids.contains(&dave), "the newly added vector must be found by search: {ids:?}");
    let dave_rank = ids.iter().position(|&i| i == dave).unwrap();
    let bob_rank = ids.iter().position(|&i| i == bob).unwrap();
    assert!(dave_rank < bob_rank, "the added vector outranks bob: {ids:?}");

    // term-by-term search of alice (excludes alice itself): the added dave is now alice's nearest.
    let neigh = nearest_term_exact(&store, &g, &iri("http://example.org/alice"), 2);
    assert_eq!(neigh[0].0, iri("http://example.org/dave"));
    assert_eq!(neigh[1].0, iri("http://example.org/bob"));

    // add of an id that already has a vector is rejected (use update instead).
    assert!(store.add(dave, &[0.5, 0.5, 0.0, 0.0]).is_err(), "duplicate add rejected");
    assert!(store.add(bob, &[0.5, 0.5, 0.0, 0.0]).is_err(), "add over an existing base id rejected");
    std::fs::remove_file(&path).ok();
}

// ---------------------------------------------------------------- gate (b): remove tombstones

#[test]
fn remove_tombstones_an_id_search_excludes_it() {
    let g = graph(A);
    let path = tmp("remove");
    let mut store = build_finalized(&g, &path);
    let bob = id_of(&g, "http://example.org/bob");
    let alice = iri("http://example.org/alice");

    // Before removal, bob is alice's nearest.
    assert_eq!(nearest_term_exact(&store, &g, &alice, 1)[0].0, iri("http://example.org/bob"));

    // Remove bob: it is now absent from get / iter / len / search.
    assert!(store.remove(bob), "removing an existing base id returns true");
    assert_eq!(store.get(bob), None, "a tombstoned id reads as absent");
    assert_eq!(store.len(), 2, "len drops by one after the tombstone");
    let live: Vec<u32> = store.iter().map(|(id, _)| id).collect();
    assert!(!live.contains(&bob), "iter excludes the tombstoned id: {live:?}");

    // Search no longer surfaces bob.
    let q = [1.0f32, 0.0, 0.0, 0.0];
    let ids: Vec<u32> = nearest_exact(&store, &q, 10).iter().map(|&(id, _)| id).collect();
    assert!(!ids.contains(&bob), "search must exclude the tombstoned id: {ids:?}");

    // Removing an absent id is a no-op (false); removing twice is idempotent.
    assert!(!store.remove(bob), "second remove is a no-op");
    assert!(!store.remove(424242), "removing a never-present id is a no-op");

    // A removed id can be re-added (the tombstone is cleared by add).
    store.add(bob, &[0.8, 0.2, 0.0, 0.0]).unwrap();
    assert_eq!(store.get(bob), Some(&[0.8f32, 0.2, 0.0, 0.0][..]));
    assert_eq!(store.len(), 3, "re-add restores the count");
    std::fs::remove_file(&path).ok();
}

#[test]
fn update_replaces_an_existing_vector() {
    let g = graph(A);
    let path = tmp("update");
    let mut store = build_finalized(&g, &path);
    let carol = id_of(&g, "http://example.org/carol");

    // update an existing (base) id → the new vector shadows the base on every read path.
    store.update(carol, &[0.97, 0.03, 0.0, 0.0]).unwrap();
    assert_eq!(store.get(carol), Some(&[0.97f32, 0.03, 0.0, 0.0][..]));
    assert_eq!(store.len(), 3, "update does not change the count");
    let q = [1.0f32, 0.0, 0.0, 0.0];
    let ids: Vec<u32> = nearest_exact(&store, &q, 3).iter().map(|&(id, _)| id).collect();
    assert!(ids.contains(&carol), "the updated vector is now near the query: {ids:?}");

    // update of an absent id is rejected (use add); validation still applies.
    assert!(store.update(424242, &[1.0, 0.0, 0.0, 0.0]).is_err(), "update of an absent id errors");
    assert!(store.update(carol, &[0.0, 0.0, 0.0, 0.0]).is_err(), "all-zero rejected");
    assert!(store.update(carol, &[f32::NAN, 0.0, 0.0, 0.0]).is_err(), "non-finite rejected");
    std::fs::remove_file(&path).ok();
}

// --------------------------------------------------- gate (c): compact == from-scratch rebuild

#[test]
fn compact_equals_a_from_scratch_rebuild_over_the_final_set() {
    let g = graph(A);
    let base_path = tmp("compact-base");
    let mut store = build_finalized(&g, &base_path);

    let alice = id_of(&g, "http://example.org/alice");
    let bob = id_of(&g, "http://example.org/bob");
    let carol = id_of(&g, "http://example.org/carol");
    let dave = id_of(&g, "http://example.org/dave");

    // A mix of mutations: add a new id, remove a base id, update a base id.
    store.add(dave, &[0.2, 0.8, 0.0, 0.0]).unwrap();
    assert!(store.remove(bob));
    store.update(carol, &[0.0, 0.0, 1.0, 0.0]).unwrap();

    // The EFFECTIVE final set, computed independently of the store.
    let mut want: Vec<(u32, Vec<f32>)> = vec![
        (alice, vec![1.0, 0.0, 0.0, 0.0]),
        (carol, vec![0.0, 0.0, 1.0, 0.0]),
        (dave, vec![0.2, 0.8, 0.0, 0.0]),
    ];
    want.sort_by_key(|&(id, _)| id);

    // (1) compact() into a fresh base.
    let compact_path = tmp("compact-out");
    let compacted = store.compact(&compact_path, &g).unwrap();
    assert!(!compacted.has_delta(), "the compacted store carries no delta");

    // (2) a from-scratch rebuild over the SAME final set, via the ordinary build path.
    let rebuild_path = tmp("compact-rebuild");
    let mut rebuilt = VectorStore::create(&rebuild_path, 4).unwrap().with_fingerprint(&g);
    for (id, v) in &want {
        rebuilt.put(*id, v).unwrap();
    }
    rebuilt.finalize().unwrap();

    // Result-equivalence: same len, same get for every id, same effective iter() set, same
    // fingerprint as a from-scratch rebuild over the final set. (The two FILES need NOT be
    // byte-identical: the dense data section follows INSERTION order, which differs between the
    // compaction's effective-iter order and the rebuild's sorted order — but the trailing id→slot
    // index is re-sorted on finalize, so every logical read agrees, which is the equivalence that
    // matters; a query never observes physical slot order.)
    assert_eq!(compacted.len(), rebuilt.len());
    assert_eq!(compacted.len(), want.len());
    for (id, v) in &want {
        assert_eq!(compacted.get(*id), Some(v.as_slice()), "get mismatch for id {id}");
        assert_eq!(rebuilt.get(*id), Some(v.as_slice()));
    }
    assert_eq!(compacted.get(bob), None, "the tombstoned id is gone from the compacted base");
    let mut got: Vec<(u32, Vec<f32>)> = compacted.iter().map(|(id, v)| (id, v.to_vec())).collect();
    got.sort_by_key(|&(id, _)| id);
    assert_eq!(got, want, "the compacted base's effective set equals the final set");
    assert_eq!(compacted.fingerprint(), rebuilt.fingerprint());
    assert!(compacted.check_graph(&g).is_ok());

    // A search over the compacted base agrees with the same search over the rebuilt base.
    let q = [0.1f32, 0.9, 0.0, 0.0];
    assert_eq!(nearest_exact(&compacted, &q, 3), nearest_exact(&rebuilt, &q, 3));

    std::fs::remove_file(&base_path).ok();
    std::fs::remove_file(&compact_path).ok();
    std::fs::remove_file(&rebuild_path).ok();
}

// ----------------------------------------- gate (d): a delta tied to gen N rejected against gen M

#[test]
fn delta_tied_to_generation_n_is_rejected_against_a_base_of_generation_m() {
    let ga = graph(A);
    let gb = graph(B); // a DIFFERENT graph generation (different dict + triple count)
    assert_ne!(
        Fingerprint::of(&ga),
        Fingerprint::of(&gb),
        "A and B must be distinct generations for this test to be meaningful"
    );

    // A store + delta built against generation A.
    let a_path = tmp("gen-a");
    let mut store_a = build_finalized(&ga, &a_path);
    let dave = id_of(&ga, "http://example.org/dave");
    store_a.add(dave, &[0.3, 0.7, 0.0, 0.0]).unwrap();
    let delta_a: VectorDelta = store_a.take_delta().expect("a delta was started");
    assert_eq!(delta_a.generation(), Some(Fingerprint::of(&ga)));

    // A separate store built against generation B.
    let b_path = tmp("gen-b");
    let mut store_b = VectorStore::create(&b_path, 4).unwrap().with_fingerprint(&gb);
    store_b.put(id_of(&gb, "http://example.org/eve"), &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store_b.finalize().unwrap();

    // Applying the gen-A delta to the gen-B base must be a descriptive ERROR, not a silent mis-key.
    let err = store_b
        .apply_delta(delta_a.clone())
        .expect_err("a delta from a different generation must be rejected");
    assert!(err.contains("generation mismatch"), "err was: {err}");
    assert!(!store_b.has_delta(), "the rejected delta must not be installed");

    // The same delta applies cleanly onto a fresh handle of its OWN generation (gen A).
    let mut store_a2 = VectorStore::open(&a_path).unwrap();
    store_a2.apply_delta(delta_a).expect("the delta matches its own generation");
    assert_eq!(store_a2.get(dave), Some(&[0.3f32, 0.7, 0.0, 0.0][..]));

    std::fs::remove_file(&a_path).ok();
    std::fs::remove_file(&b_path).ok();
}

// ---------------------------------------------------------------- delta on a build-phase store

#[test]
fn add_remove_update_on_a_build_phase_store() {
    // The delta API also works before finalize: add == put, remove/update edit the in-RAM build
    // set, and finalize then writes the resulting set (no delta carried into the read phase).
    let g = graph(A);
    let path = tmp("buildphase");
    let alice = id_of(&g, "http://example.org/alice");
    let bob = id_of(&g, "http://example.org/bob");
    let carol = id_of(&g, "http://example.org/carol");

    let mut store = VectorStore::create(&path, 4).unwrap().with_fingerprint(&g);
    store.add(alice, &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.add(bob, &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.add(carol, &[0.0, 0.0, 1.0, 0.0]).unwrap();
    assert!(store.remove(bob), "remove a build-phase vector");
    store.update(carol, &[0.0, 0.0, 0.0, 1.0]).unwrap();
    assert_eq!(store.get(bob), None);
    assert_eq!(store.get(carol), Some(&[0.0f32, 0.0, 0.0, 1.0][..]));
    assert_eq!(store.len(), 2);

    store.finalize().unwrap();
    assert_eq!(store.len(), 2);
    assert_eq!(store.get(alice), Some(&[1.0f32, 0.0, 0.0, 0.0][..]));
    assert_eq!(store.get(carol), Some(&[0.0f32, 0.0, 0.0, 1.0][..]));
    assert_eq!(store.get(bob), None);
    std::fs::remove_file(&path).ok();
}
