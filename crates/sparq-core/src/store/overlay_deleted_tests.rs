//! [GPT-6 Astra] Deletion projection behavior, ownership, and invalidation (#4246).

use super::{Id, Overlay, Perm, TripleStore, BUILT};
use std::borrow::Cow;

fn triples() -> Vec<[Id; 3]> {
    (1..=12)
        .flat_map(|s| (1..=4).map(move |p| [s, p, s + p]))
        .collect()
}

fn sweep(store: &TripleStore, reference: &[[Id; 3]]) {
    let rebuilt = TripleStore::from_triples(reference.to_vec());
    // Every built permutation, every leading-prefix length, present/absent keys.
    for &perm in BUILT {
        for triple in reference.iter().copied().chain([[0; 3], [Id::MAX; 3]]) {
            for lead in 0..=3 {
                let mut pattern = [None; 3];
                for &col in &perm.order()[..lead] {
                    pattern[col] = Some(triple[col]);
                }
                let actual = store.scan_perm(&pattern, perm).unwrap();
                let expected = rebuilt.scan_perm(&pattern, perm).unwrap();
                assert_eq!(actual.rows, expected.rows, "{perm:?} {pattern:?}");
                assert_eq!(store.estimate(&pattern), rebuilt.estimate(&pattern));
            }
        }
    }
    for perm in Perm::ALL {
        if !BUILT.contains(&perm) {
            assert!(store.scan_perm(&[None; 3], perm).is_none());
            #[cfg(feature = "overlay-deleted-projections")]
            if let Some(ov) = &store.overlay {
                assert!(ov.deleted_by_perm[perm as usize].get().is_none());
            }
        }
    }
    assert_eq!(store.len(), reference.len());
    for &t in reference {
        assert!(store.contains(t));
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_is_lazy_reused_and_accounted() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
    let ov = store.overlay.as_ref().unwrap();
    assert!(ov.deleted_by_perm.iter().all(|s| s.get().is_none()));
    let cold_heap = ov.heap_bytes();
    let mut expected_heap = cold_heap;
    for &perm in BUILT {
        // An untouched exact range still needs its correction before borrowing base.
        let pattern = [Some(12), Some(4), Some(16)];
        let scan = store.scan_perm(&pattern, perm).unwrap();
        assert_eq!(scan.rows.len(), 1);
        assert!(matches!(scan.rows, Cow::Borrowed(_)));
        let rows = ov.deleted_by_perm[perm as usize]
            .get()
            .expect("scan uses cached deletion count");
        assert_eq!(rows.len(), 3);
        assert!(rows.windows(2).all(|w| w[0] < w[1]));
        let pointer = rows.as_ptr();
        // The requested slot is stable across subsequent counts and scans.
        for _ in 0..5 {
            assert_eq!(ov.count_correction(perm, [0; 3], [Id::MAX; 3]), (0, 3));
            assert_eq!(
                ov.deleted_by_perm[perm as usize].get().unwrap().as_ptr(),
                pointer
            );
        }
        expected_heap += rows.capacity() * std::mem::size_of::<[Id; 3]>();
        assert_eq!(ov.heap_bytes(), expected_heap);
        for other in Perm::ALL {
            if other as usize > perm as usize {
                assert!(ov.deleted_by_perm[other as usize].get().is_none());
            }
        }
    }
    let empty = Overlay::default();
    assert_eq!(empty.deleted_count(Perm::Spo, [0; 3], [Id::MAX; 3]), 0);
    assert!(empty.deleted_by_perm.iter().all(|s| s.get().is_none()));
}

#[test]
fn deleted_cache_matches_rebuild_after_mixed_deltas() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    sweep(&store, &reference);
    // Growing deletions, undeleting base, retracting additions, duplicate/no-op
    // updates and delete-then-insert of the same row all traverse warmed caches.
    let batches = [
        (vec![[20, 2, 22]], vec![[1, 1, 2], [2, 2, 4]]),
        (vec![[1, 1, 2]], vec![[3, 3, 6], [20, 2, 22]]),
        (vec![[4, 4, 8], [4, 4, 8]], vec![[4, 4, 8], [99; 3]]),
        (vec![[2, 2, 4], [3, 3, 6]], vec![]),
    ];
    for (inserts, deletes) in batches {
        store.apply_delta(&inserts, &deletes);
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        // Correct rows/counts, not merely empty-slot observations, pin invalidation.
        sweep(&store, &reference);
    }
    assert!(!store.has_overlay());
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_survives_insert_only_and_noop_deltas() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
    sweep(&store, &reference);
    let pointers: Vec<_> = BUILT
        .iter()
        .map(|&p| {
            store.overlay.as_ref().unwrap().deleted_by_perm[p as usize]
                .get()
                .unwrap()
                .as_ptr()
        })
        .collect();
    for (inserts, deletes) in [
        (vec![[20, 2, 22]], vec![]), // actual added-set change with tombstones present
        (vec![[20, 2, 22], [3, 3, 6]], vec![[1, 1, 2], [99; 3]]), // all no-ops
        (vec![], vec![[20, 2, 22]]), // retract an addition, not a base deletion
        (vec![], vec![]),
    ] {
        store.apply_delta(&inserts, &deletes);
        for (&perm, &pointer) in BUILT.iter().zip(&pointers) {
            assert_eq!(
                store.overlay.as_ref().unwrap().deleted_by_perm[perm as usize]
                    .get()
                    .expect("unchanged tombstones retain their projection")
                    .as_ptr(),
                pointer,
            );
        }
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        sweep(&store, &reference); // also pins the independent added-cache invalidation
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_actual_tombstone_changes_invalidate_before_publication() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    store.apply_delta(&[], &[[1, 1, 2]]);
    reference.retain(|t| *t != [1, 1, 2]);
    for (inserts, deletes) in [
        (vec![], vec![[2, 2, 4]]),          // grow the deleted set
        (vec![[1, 1, 2]], vec![]),          // undelete an existing tombstone
        (vec![[3, 3, 6]], vec![[3, 3, 6]]), // changes during batch, even if net unchanged
    ] {
        sweep(&store, &reference);
        store.apply_delta(&inserts, &deletes);
        assert!(store
            .overlay
            .as_ref()
            .unwrap()
            .deleted_by_perm
            .iter()
            .all(|s| s.get().is_none()));
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        sweep(&store, &reference);
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_inclusive_bounds_and_empty_ranges() {
    let mut ov = Overlay::default();
    ov.deleted.extend([[0, 1, 2], [2, 3, 4], [Id::MAX; 3]]);
    for perm in Perm::ALL {
        let order = perm.order();
        for &t in &ov.deleted {
            let row = [t[order[0]], t[order[1]], t[order[2]]];
            assert_eq!(ov.deleted_count(perm, row, row), 1);
        }
        assert_eq!(ov.deleted_count(perm, [0; 3], [Id::MAX; 3]), 3);
        assert_eq!(ov.deleted_count(perm, [42; 3], [42; 3]), 0);
    }
}

#[test]
fn deleted_cache_compressed_base_matches_rebuild() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples_compressed(reference.clone());
    store.apply_delta(&[[20, 2, 22]], &[[1, 1, 2], [2, 2, 4]]);
    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
    reference.push([20, 2, 22]);
    sweep(&store, &reference);
    store.apply_delta(&[[1, 1, 2]], &[[3, 3, 6]]);
    reference.push([1, 1, 2]);
    reference.retain(|t| *t != [3, 3, 6]);
    sweep(&store, &reference);
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_fork_and_clone_are_independent() {
    let mut original = TripleStore::from_triples(triples());
    original.apply_delta(&[], &[[1, 1, 2]]);
    let cold_fork = original.fork();
    for &perm in BUILT {
        original.scan_perm(&[None; 3], perm).unwrap();
    }
    assert!(cold_fork
        .overlay
        .as_ref()
        .unwrap()
        .deleted_by_perm
        .iter()
        .all(|s| s.get().is_none()));
    let warm_fork = original.fork();
    let cloned_overlay = original.overlay.clone().unwrap();
    for &perm in BUILT {
        let slot = perm as usize;
        let original_rows = original.overlay.as_ref().unwrap().deleted_by_perm[slot]
            .get()
            .unwrap();
        let fork_rows = warm_fork.overlay.as_ref().unwrap().deleted_by_perm[slot]
            .get()
            .unwrap();
        let clone_rows = cloned_overlay.deleted_by_perm[slot].get().unwrap();
        assert_eq!(original_rows, fork_rows);
        assert_ne!(original_rows.as_ptr(), fork_rows.as_ptr());
        assert_ne!(original_rows.as_ptr(), clone_rows.as_ptr());
    }
    original.apply_delta(&[[1, 1, 2]], &[[2, 2, 4]]);
    for &perm in BUILT {
        for frozen in [&cold_fork, &warm_fork] {
            assert_eq!(
                frozen
                    .scan_perm(&[Some(1), Some(1), Some(2)], perm)
                    .unwrap()
                    .rows
                    .len(),
                0
            );
            assert_eq!(
                frozen
                    .scan_perm(&[Some(2), Some(2), Some(4)], perm)
                    .unwrap()
                    .rows
                    .len(),
                1
            );
        }
        assert_eq!(
            original
                .scan_perm(&[Some(1), Some(1), Some(2)], perm)
                .unwrap()
                .rows
                .len(),
            1
        );
        assert_eq!(
            original
                .scan_perm(&[Some(2), Some(2), Some(4)], perm)
                .unwrap()
                .rows
                .len(),
            0
        );
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_concurrent_first_reads_share_initialized_projection() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    BUILT
                        .iter()
                        .map(|&perm| {
                            assert_eq!(store.scan_perm(&[None; 3], perm).unwrap().rows.len(), 46);
                            let rows = store.overlay.as_ref().unwrap().deleted_by_perm
                                [perm as usize]
                                .get()
                                .unwrap();
                            (rows.as_ptr() as usize, rows.clone())
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut results = handles.into_iter().map(|h| h.join().unwrap());
        assert_eq!(results.next().unwrap(), results.next().unwrap());
    });
}

#[test]
fn deleted_cache_graph_snapshot_retains_warm_generation() {
    use crate::Graph;
    use oxrdf::{NamedNode, Term};
    let term = |s| Term::from(NamedNode::new(s).unwrap());
    let a = [term("urn:a"), term("urn:p"), term("urn:o")];
    let b = [term("urn:b"), term("urn:p"), term("urn:o")];
    let mut graph = Graph::load_str(
        "<urn:a> <urn:p> <urn:o> . <urn:b> <urn:p> <urn:o> .",
        "turtle",
    )
    .unwrap();
    graph.apply_delta(&[], std::slice::from_ref(&a)).unwrap();
    for &perm in BUILT {
        assert_eq!(
            graph.store.scan_perm(&[None; 3], perm).unwrap().rows.len(),
            1
        );
    }
    let snapshot = graph.snapshot();
    graph
        .apply_delta(std::slice::from_ref(&a), std::slice::from_ref(&b))
        .unwrap();
    for (terms, snapshot_len, current_len) in [(a, 0, 1), (b, 1, 0)] {
        let pattern = terms.map(|t| Some(graph.dict.lookup(&t)));
        assert_eq!(snapshot.store.estimate(&pattern), snapshot_len);
        assert_eq!(graph.store.estimate(&pattern), current_len);
        for &perm in BUILT {
            assert_eq!(
                snapshot.store.scan_perm(&pattern, perm).unwrap().rows.len(),
                snapshot_len
            );
            assert_eq!(
                graph.store.scan_perm(&pattern, perm).unwrap().rows.len(),
                current_len
            );
        }
    }
}

// [GPT-6 Astra] Default-off must preserve main's representation and linear-count
// behavior, not merely return the same rows after allocating a hidden projection.
#[cfg(not(feature = "overlay-deleted-projections"))]
#[test]
fn deleted_projection_feature_off_preserves_main_layout_and_heap() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
    let before = store.heap_bytes();
    for &perm in BUILT {
        for _ in 0..3 {
            let scan = store
                .scan_perm(&[Some(12), Some(4), Some(16)], perm)
                .unwrap();
            assert_eq!(scan.rows.len(), 1);
            assert!(matches!(scan.rows, Cow::Borrowed(_)));
            assert_eq!(store.estimate(&[Some(1), Some(1), Some(2)]), 0);
            assert_eq!(
                store.heap_bytes(),
                before,
                "default reads retain no deletion projection"
            );
        }
    }
    let frozen = store.fork();
    assert_eq!(frozen.heap_bytes(), before);
    // These are the three fields of main's Overlay. This is an empirical default-off
    // footprint budget for the supported configurations, not a repr(Rust) layout
    // guarantee. It also detects retained cache metadata whose slots own no heap
    // memory. Compiler or target layout changes require reassessing this budget.
    let main_fields = std::mem::size_of::<Vec<[Id; 3]>>()
        + std::mem::size_of::<rustc_hash::FxHashSet<[Id; 3]>>()
        + std::mem::size_of::<[std::sync::OnceLock<Vec<[Id; 3]>>; 6]>();
    assert_eq!(std::mem::size_of::<Overlay>(), main_fields);
    assert_eq!(
        std::mem::align_of::<Overlay>(),
        std::mem::align_of::<usize>()
    );
}
