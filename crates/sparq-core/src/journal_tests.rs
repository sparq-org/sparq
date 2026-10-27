//! Retained-journal durability and independent per-file crash prefixes.

use super::*;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sparq-journal-{}", random_id()));
        std::fs::create_dir(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        fault::disarm();
        crash::stop();
        std::fs::remove_dir_all(&self.0).ok();
    }
}
fn name(i: u8) -> Term {
    NamedNode::new(format!("http://example/g{i}"))
        .unwrap()
        .into()
}
fn triple(i: u8) -> [Term; 3] {
    [name(i + 10), name(100), name(101)]
}
fn slot(i: u8) -> Option<Term> {
    (i != 0).then(|| name(i))
}
type Model = BTreeMap<u8, BTreeSet<u8>>;
fn seed(dir: &Path) -> (Graph, Model) {
    let mut graph = Graph::new();
    let mut model = Model::new();
    for i in 0..4 {
        let op = TxnOp::Insert {
            slot: slot(i),
            triple: triple(0),
        };
        graph.redo_txn_op(op).unwrap();
        model.insert(i, BTreeSet::from([0]));
    }
    graph.save(dir).unwrap();
    (Graph::open(dir).unwrap(), model)
}
fn state(graph: &Graph) -> BTreeMap<String, BTreeSet<String>> {
    fn triples(g: &Graph) -> BTreeSet<String> {
        g.iter_ids()
            .map(|ids| ids.map(|id| g.dict.term(id).to_string()).join(" "))
            .collect()
    }
    let mut state = BTreeMap::from([(String::new(), triples(graph))]);
    for (name, sub) in graph.named() {
        state.insert(name.to_string(), triples(sub));
    }
    state
}
fn expected(model: &Model) -> BTreeMap<String, BTreeSet<String>> {
    model
        .iter()
        .map(|(i, values)| {
            (
                slot(*i).map_or(String::new(), |n| n.to_string()),
                values
                    .iter()
                    .map(|v| triple(*v).map(|term| term.to_string()).join(" "))
                    .collect(),
            )
        })
        .collect()
}
fn resolve(model: &mut Model, actions: &[(u8, u8, u8)]) -> Vec<TxnOp> {
    let mut ops = Vec::new();
    for &(kind, i, value) in actions {
        match kind {
            0 | 1 => {
                if kind == 0 {
                    model.entry(i).or_default().insert(value);
                    ops.push(TxnOp::Insert {
                        slot: slot(i),
                        triple: triple(value),
                    });
                } else {
                    if let Some(values) = model.get_mut(&i) {
                        values.remove(&value);
                    }
                    ops.push(TxnOp::Delete {
                        slot: slot(i),
                        triple: triple(value),
                    });
                }
            }
            2 => {
                if let Some(values) = model.get_mut(&i) {
                    for value in std::mem::take(values) {
                        ops.push(TxnOp::Delete {
                            slot: slot(i),
                            triple: triple(value),
                        });
                    }
                }
            }
            3 if i != 0 => {
                model.remove(&i);
                ops.push(TxnOp::Drop(name(i)));
            }
            4 if i != 0 => {
                model.entry(i).or_default();
                ops.push(TxnOp::Create(name(i)));
            }
            5 if i != 0 => {
                model.insert(i, BTreeSet::from([value]));
                ops.extend([
                    TxnOp::Drop(name(i)),
                    TxnOp::Create(name(i)),
                    TxnOp::Insert {
                        slot: slot(i),
                        triple: triple(value),
                    },
                ]);
            }
            _ => {}
        }
    }
    ops
}
fn commit(graph: &mut Graph, ops: &[TxnOp]) {
    graph.commit_txn_ops(ops).unwrap();
    for op in ops {
        graph.redo_txn_op(op.clone()).unwrap();
    }
    graph.finish_txn().unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn crash_prefixes_recover_whole_updates(
        frames in prop::collection::vec(prop::collection::vec((0u8..6, 0u8..5, 0u8..6), 1..6), 2..25),
        cut in any::<usize>(), mode in 0u8..4, persist in any::<u64>(),
    ) {
        let tmp = TempDir::new();
        let (mut graph, mut model) = seed(&tmp.0);
        crash::start(&tmp.0);
        let mut previous = model.clone();
        for (i, actions) in frames.iter().enumerate() {
            previous = model.clone();
            let ops = resolve(&mut model, actions);
            if i + 1 != frames.len() {
                commit(&mut graph, &ops);
                if i % 7 == 6 { graph.clear_txn().unwrap(); }
                continue;
            }
            if mode == 0 && !ops.is_empty() {
                let journal = TxnJournal::path(&tmp.0);
                fault::fail_when(move |step, p| step == "sync-data" && p == journal);
                prop_assert!(graph.commit_txn_ops(&ops).is_err());
                fault::disarm();
            } else {
                graph.commit_txn_ops(&ops).unwrap();
                let count = if mode == 1 { cut % (ops.len() + 1) } else { ops.len() };
                for op in &ops[..count] { graph.redo_txn_op(op.clone()).unwrap(); }
                if mode >= 2 {
                    graph.finish_txn().unwrap();
                    if mode == 3 {
                        let mut steps = cut % 12;
                        fault::crash_at(move |_, _| {
                            if steps == 0 { true } else { steps -= 1; false }
                        });
                        let _ = graph.clear_txn();
                        fault::disarm();
                    }
                }
            }
        }
        graph.poison_txn(); // Drop the handles without a clean-close checkpoint.
        drop(graph);
        crash::crash(&tmp.0, persist);
        let reopened = Graph::open(&tmp.0).unwrap();
        let actual = state(&reopened);
        prop_assert!(actual == expected(&model) || (mode == 0 && actual == expected(&previous)),
            "recovered {:?}, expected {:?} (or pre-commit {:?})", actual, model, previous);
        drop(reopened);
        prop_assert_eq!(state(&Graph::open(&tmp.0).unwrap()), actual);
    }
}

#[test]
fn one_sync_per_materialized_update() {
    let tmp = TempDir::new();
    let (mut graph, mut model) = seed(&tmp.0);
    // Warm the first-append entry barrier inherited from the durability port.
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: None,
            triple: triple(1),
        }],
    );
    graph.clear_txn().unwrap();
    for actions in [
        vec![(0, 0, 2)],
        vec![(0, 0, 3), (0, 1, 3), (0, 2, 3)],
        vec![(5, 1, 4), (4, 4, 0)],
    ] {
        crash::start(&tmp.0);
        let ops = resolve(&mut model, &actions);
        commit(&mut graph, &ops);
        assert_eq!(
            crash::syncs(),
            1,
            "one journal sync regardless of slot count or existence ops"
        );
        println!("sync-count checkpointed: {}", crash::syncs());
        crash::stop();
    }
    graph.close().unwrap();
}

#[test]
fn checkpoint_triggers_and_direct_write_ordering() {
    let tmp = TempDir::new();
    let (mut graph, _) = seed(&tmp.0);
    let op = TxnOp::Insert {
        slot: Some(name(1)),
        triple: triple(1),
    };
    commit(&mut graph, &[op]);
    assert!(TxnJournal::path(&tmp.0).metadata().unwrap().len() > 0);
    // Direct child writes must checkpoint the retained parent journal first.
    graph.apply_delta_to_named(0, &[], &[triple(1)]).unwrap();
    assert_eq!(TxnJournal::path(&tmp.0).metadata().unwrap().len(), 0);
    let snapshot = state(&graph);
    drop(graph);
    assert_eq!(state(&Graph::open(&tmp.0).unwrap()), snapshot);

    let mut graph = Graph::open(&tmp.0).unwrap();
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: None,
            triple: triple(2),
        }],
    );
    graph.compact().unwrap();
    assert_eq!(TxnJournal::path(&tmp.0).metadata().unwrap().len(), 0);
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: None,
            triple: triple(3),
        }],
    );
    graph.close().unwrap();
    assert_eq!(TxnJournal::path(&tmp.0).metadata().unwrap().len(), 0);
    let mut graph = Graph::open(&tmp.0).unwrap();
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: None,
            triple: triple(4),
        }],
    );
    drop(graph);
    assert_eq!(TxnJournal::path(&tmp.0).metadata().unwrap().len(), 0);
}

#[test]
fn checkpoint_sync_failures_freeze_without_retry_and_recover() {
    let _serial = io_fault::lock();
    for journal in [false, true] {
        let tmp = TempDir::new();
        let (mut graph, _) = seed(&tmp.0);
        commit(
            &mut graph,
            &[
                TxnOp::Insert {
                    slot: None,
                    triple: triple(1),
                },
                TxnOp::Insert {
                    slot: Some(name(1)),
                    triple: triple(2),
                },
            ],
        );
        let snapshot = state(&graph);
        let path = if journal {
            TxnJournal::path(&tmp.0)
        } else {
            Wal::path(&tmp.0.join("named/0"))
        };
        let guard = io_fault::fail_sync_for(&path);
        assert!(graph.clear_txn().is_err());
        assert!(graph.is_frozen());
        assert!(!std::fs::read(TxnJournal::path(&tmp.0)).unwrap().is_empty());
        drop(guard);
        assert!(graph.clear_txn().is_err());
        assert!(graph.apply_delta(&[triple(3)], &[]).is_err());
        drop(graph);
        assert_eq!(state(&Graph::open(&tmp.0).unwrap()), snapshot);
    }
}

#[test]
fn replay_drop_create_over_a_later_slot_wal() {
    let tmp = TempDir::new();
    let (mut graph, mut model) = seed(&tmp.0);
    for actions in [vec![(0, 1, 1)], vec![(3, 1, 0)], vec![(4, 1, 0), (0, 1, 2)]] {
        let ops = resolve(&mut model, &actions);
        commit(&mut graph, &ops);
    }
    let journal = TxnJournal::path(&tmp.0);
    fault::fail_when(move |step, path| step == "sync-data" && path == journal);
    // All WALs and the latest incarnation's manifest are durable, but every old
    // frame remains. Recovery must discard the ahead WAL when replaying DROP,
    // then reconstruct the new incarnation from the following CREATE/INSERT.
    assert!(graph.clear_txn().is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(read_named_manifest(&tmp.0)
        .unwrap()
        .unwrap()
        .entries
        .iter()
        .any(|(slot, n)| *n == name(1) && *slot >= 3));
    drop(graph);
    assert_eq!(state(&Graph::open(&tmp.0).unwrap()), expected(&model));
}

#[test]
fn switching_between_parent_and_child_journals_preserves_order() {
    let tmp = TempDir::new();
    let (mut graph, _) = seed(&tmp.0);
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: Some(name(1)),
            triple: triple(1),
        }],
    );
    commit(
        &mut graph.named[0].1,
        &[TxnOp::Delete {
            slot: None,
            triple: triple(1),
        }],
    );
    assert_eq!(TxnJournal::path(&tmp.0).metadata().unwrap().len(), 0);
    let snapshot = state(&graph);
    graph.poison_txn();
    drop(graph);
    assert_eq!(state(&Graph::open(&tmp.0).unwrap()), snapshot);
}

#[test]
fn base_rewrites_refuse_an_unmaterialized_frame() {
    let tmp = TempDir::new();
    let (mut graph, _) = seed(&tmp.0);
    graph
        .commit_txn_ops(&[TxnOp::Insert {
            slot: None,
            triple: triple(1),
        }])
        .unwrap();
    assert!(graph.compact().is_err());
    assert!(graph.vacuum().is_err());
    let copy = TempDir::new();
    assert!(graph.save(&copy.0).is_err());
    assert!(graph.restore_in_place(Graph::new()).is_err());
    assert!(
        !graph.is_frozen(),
        "a caller can still finish materializing the frame"
    );
    graph.poison_txn();
    drop(graph);
    let reopened = Graph::open(&tmp.0).unwrap();
    assert_eq!(reopened.len(), 2);
}

#[test]
fn refused_compaction_preserves_logs_through_drop() {
    let tmp = TempDir::new();
    let (mut graph, _) = seed(&tmp.0);
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: None,
            triple: triple(1),
        }],
    );
    let expected = state(&graph);
    let journal = TxnJournal::path(&tmp.0);
    let retained = std::fs::read(&journal).unwrap();
    let staging = staging_path(&tmp.0, "compact-new");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("foreign"), b"preserve").unwrap();
    assert!(graph.compact().is_err());
    assert!(!graph.is_frozen());
    assert_eq!(std::fs::read(&journal).unwrap(), retained);
    drop(graph);
    assert_eq!(std::fs::read(&journal).unwrap(), retained);
    std::fs::remove_dir_all(staging).unwrap();
    assert_eq!(state(&Graph::open(&tmp.0).unwrap()), expected);
}

#[test]
fn close_preserves_an_unmaterialized_frame() {
    let tmp = TempDir::new();
    let (mut graph, _) = seed(&tmp.0);
    graph
        .commit_txn_ops(&[TxnOp::Insert {
            slot: None,
            triple: triple(1),
        }])
        .unwrap();
    assert!(graph.close().is_err());
    assert_eq!(Graph::open(&tmp.0).unwrap().len(), 2);
}

#[test]
fn failed_journal_sync_preserves_whole_frames_by_fresh_publication() {
    let tmp = TempDir::new();
    let (mut graph, _) = seed(&tmp.0);
    crash::start(&tmp.0);
    commit(
        &mut graph,
        &[TxnOp::Insert {
            slot: None,
            triple: triple(1),
        }],
    );
    commit(
        &mut graph,
        &[TxnOp::Delete {
            slot: None,
            triple: triple(1),
        }],
    );
    let expected = state(&graph);
    let journal = TxnJournal::path(&tmp.0);
    let retained = std::fs::read(&journal).unwrap();
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = events.clone();
    let failed = journal.clone();
    fault::fail_every(move |step, path| {
        observed
            .borrow_mut()
            .push((step.to_owned(), path.to_path_buf()));
        step == "sync-data" && path == failed
    });
    assert!(graph.clear_txn().is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(graph.is_frozen());
    assert_eq!(std::fs::read(&journal).unwrap(), retained);
    assert_eq!(crash::synced_len(&journal), Some(retained.len()));
    let events = events.borrow();
    let synced = events
        .iter()
        .position(|(step, path)| step == "file-synced" && *path == journal)
        .unwrap();
    let renamed = events
        .iter()
        .position(|(step, path)| step == "rename" && *path == tmp.0.join("txn.log.tmp"))
        .unwrap();
    assert!(
        synced < renamed,
        "the full copy must be synced before publication"
    );
    #[cfg(not(windows))]
    assert!(events[renamed + 1..]
        .iter()
        .any(|(step, path)| step == "dir-synced" && *path == tmp.0));
    drop(graph);
    crash::crash(&tmp.0, 0);
    assert_eq!(state(&Graph::open(&tmp.0).unwrap()), expected);
}

#[test]
fn replay_checkpoints_a_valid_empty_frame() {
    let tmp = TempDir::new();
    Graph::new().save(&tmp.0).unwrap();
    let mut frame = Vec::new();
    frame.extend_from_slice(&TxnJournal::TXN_MAGIC.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&Wal::checksum(&frame).to_le_bytes());
    frame.extend_from_slice(&Wal::COMMIT_MARKER.to_le_bytes());
    std::fs::write(TxnJournal::path(&tmp.0), frame).unwrap();
    let graph = Graph::open(&tmp.0).unwrap();
    assert_eq!(graph.len(), 0);
    assert_eq!(TxnJournal::path(&tmp.0).metadata().unwrap().len(), 0);
}
