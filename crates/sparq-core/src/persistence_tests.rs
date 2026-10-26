//! Ordering checks observe real fsync calls; injected failures model interrupted operations.
//! Removing a directory entry at a crash point models its loss, not a physical power cut.

use super::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Synced(PathBuf),
    ManifestPublished(PathBuf),
    ManifestReady(PathBuf),
    DropPublished(PathBuf),
    LogFileSynced(PathBuf),
    JournalTruncated(PathBuf),
    SwapPublished(PathBuf),
    RetiringOldBase(PathBuf),
}

thread_local! {
    static EVENTS: RefCell<Option<Vec<Event>>> = const { RefCell::new(None) };
    static FAIL_AT: RefCell<Option<Event>> = const { RefCell::new(None) };
}

pub(super) fn record(event: Event) {
    EVENTS.with_borrow_mut(|events| {
        if let Some(events) = events {
            events.push(event);
        }
    });
}

/// Fail once at a real persistence boundary; production builds contain no injection path.
pub(super) fn checkpoint(event: Event) -> std::io::Result<()> {
    let fail = FAIL_AT.with_borrow_mut(|target| {
        if target.as_ref() == Some(&event) {
            target.take();
            true
        } else {
            false
        }
    });
    record(event);
    if fail {
        Err(std::io::Error::other("injected persistence interruption"))
    } else {
        Ok(())
    }
}

struct Failure;

impl Failure {
    fn at(event: Event) -> Self {
        FAIL_AT.with_borrow_mut(|target| *target = Some(event));
        Self
    }

    fn assert_hit(&self) {
        FAIL_AT.with_borrow(|target| assert!(target.is_none(), "crash point was not reached"));
    }
}

impl Drop for Failure {
    fn drop(&mut self) {
        FAIL_AT.with_borrow_mut(|target| *target = None);
    }
}

struct Trace;

impl Trace {
    fn start() -> Self {
        EVENTS.with_borrow_mut(|events| *events = Some(Vec::new()));
        Self
    }

    fn take(&self) -> Vec<Event> {
        EVENTS.with_borrow_mut(|events| events.take().unwrap())
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        EVENTS.with_borrow_mut(|events| *events = None);
    }
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            Self(std::env::temp_dir().join(format!("sparq_{tag}_{}_{n}", std::process::id())));
        std::fs::create_dir_all(&dir.0).unwrap();
        dir
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn position(events: &[Event], event: Event) -> usize {
    events
        .iter()
        .position(|e| *e == event)
        .unwrap_or_else(|| panic!("missing {event:?} in {events:?}"))
}

fn named(local: &str) -> Term {
    Term::NamedNode(NamedNode::new(format!("http://ex/{local}")).unwrap())
}

// Capture the saved layout before opening it creates WAL/journal files. Check every file
// and directory, including recursive named graphs, without depending on read_dir ordering.
fn saved_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            paths.extend(saved_paths(&entry.path()));
        } else {
            paths.push(entry.path());
        }
    }
    paths.push(dir.to_path_buf());
    paths
}

#[test]
fn new_named_base_syncs_precede_manifest_and_journal_retirement() {
    // Cover both normal materialization and journal-only crash recovery. Both paths must
    // persist a newly created named graph before they clear the committed frame.
    for recovery in [false, true] {
        let dir = TestDir::new("named_sync_order");
        Graph::new().save(&dir.0).unwrap();
        let base_files: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let mut graph = Graph::open(&dir.0).unwrap();
        let triple = [named("s"), named("p"), named("o")];
        graph
            .commit_txn(&[(true, Some(named("g")), triple)])
            .unwrap();
        assert!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len() > 0);
        let trace = Trace::start();
        if recovery {
            drop(graph);
            graph = Graph::open(&dir.0).unwrap();
        } else {
            graph
                .apply_delta_nquads(
                    "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .",
                    "",
                )
                .unwrap();
            graph.clear_txn().unwrap();
        }
        let events = trace.take();
        let sub = dir.0.join("named/0");
        let manifest = position(&events, Event::ManifestPublished(dir.0.join("named.bin")));
        // Log open now also syncs its directory. Check the final base-tree sync before
        // manifest publication, which must follow every saved base file and log.
        let final_sync = |path: PathBuf| {
            events[..manifest]
                .iter()
                .rposition(|event| *event == Event::Synced(path.clone()))
                .unwrap()
        };
        let sub_sync = final_sync(sub.clone());
        for file in base_files {
            assert!(position(&events, Event::Synced(sub.join(file))) < sub_sync);
        }
        let named_sync = final_sync(dir.0.join("named"));
        let root_sync = final_sync(dir.0.clone());
        let truncated = position(&events, Event::JournalTruncated(dir.0.join("txn.log")));
        for log in ["wal.log", "txn.log"] {
            assert!(position(&events, Event::Synced(sub.join(log))) < sub_sync);
        }
        assert!(sub_sync < named_sync && named_sync < root_sync);
        assert!(root_sync < manifest && manifest < truncated);
        assert_eq!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len(), 0);
        drop(graph);
        let reopened = Graph::open(&dir.0).unwrap();
        assert_eq!(reopened.named[0].1.store.len(), 1);
    }
}

#[test]
fn compact_syncs_saved_tree_before_swap_and_old_base_retirement() {
    let dir = TestDir::new("compact_sync_order");
    let store = dir.0.join("graph");
    Graph::load_dataset(
        concat!(
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n",
            "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .\n",
        ),
        "nquads",
    )
    .unwrap()
    .save(&store)
    .unwrap();
    let layout = saved_paths(&store);
    let mut graph = Graph::open(&store).unwrap();
    graph
        .apply_delta_nquads(
            concat!(
                "<http://ex/new> <http://ex/p> <http://ex/o> .\n",
                "<http://ex/new> <http://ex/p> <http://ex/o> <http://ex/g> .\n",
            ),
            "",
        )
        .unwrap();
    let trace = Trace::start();
    graph.compact().unwrap();
    let events = trace.take();
    // Assert both the child swap and the enclosing dataset swap, whose saved image
    // contains another copy of the named graph tree.
    for root in [store.join("named/0"), store.clone()] {
        let new_dir = root.with_extension("compact-new");
        let published = position(&events, Event::SwapPublished(root.clone()));
        let retired = position(
            &events,
            Event::RetiringOldBase(root.with_extension("compact-old")),
        );
        // save_named also syncs the root while publishing its manifest. Use the final
        // sync of each directory: it must follow all of that directory's saved files.
        let synced_position = |path: PathBuf| {
            events[..published]
                .iter()
                .rposition(|e| *e == Event::Synced(path.clone()))
                .unwrap_or_else(|| panic!("missing sync of {path:?} in {events:?}"))
        };
        let root_sync = synced_position(new_dir.clone());
        for path in layout.iter().filter(|path| path.starts_with(&root)) {
            let saved = new_dir.join(path.strip_prefix(&root).unwrap());
            let synced = synced_position(saved.clone());
            assert!(synced < published && synced <= root_sync, "{saved:?}");
            if saved != new_dir {
                let parent_sync = synced_position(saved.parent().unwrap().to_path_buf());
                assert!(synced < parent_sync, "sync {saved:?} before its directory");
            }
        }
        let parent_sync = Event::Synced(root.parent().unwrap().to_path_buf());
        assert!(events[root_sync + 1..published].contains(&parent_sync));
        assert!(
            events[published + 1..retired].contains(&parent_sync),
            "persist the swap before removing the old base and WAL"
        );
        assert!(!root.with_extension("compact-old").exists());
    }
    drop(graph);
    let reopened = Graph::open(&store).unwrap();
    assert_eq!(reopened.store.len(), 2);
    assert_eq!(reopened.named[0].1.store.len(), 2);
}

fn deletion_fixture(dir: &Path) -> Graph {
    Graph::load_dataset(
        concat!(
            "<http://ex/base> <http://ex/p> <http://ex/o> .\n",
            "<http://ex/base> <http://ex/p> <http://ex/o> <http://ex/first> .\n",
            "<http://ex/base> <http://ex/p> <http://ex/o> <http://ex/remove> .\n",
            "<http://ex/base> <http://ex/p> <http://ex/o> <http://ex/last> .\n",
        ),
        "nquads",
    )
    .unwrap()
    .save(dir)
    .unwrap();
    sync_saved_graph(dir).unwrap();
    let mut graph = Graph::open(dir).unwrap();
    graph
        .apply_delta_nquads(
            concat!(
                "<http://ex/ack> <http://ex/p> <http://ex/o> .\n",
                "<http://ex/ack> <http://ex/p> <http://ex/o> <http://ex/first> .\n",
                "<http://ex/ack> <http://ex/p> <http://ex/o> <http://ex/remove> .\n",
                "<http://ex/ack> <http://ex/p> <http://ex/o> <http://ex/last> .\n",
            ),
            "",
        )
        .unwrap();
    graph
}

fn contents(graph: &Graph) -> Vec<String> {
    let scan = graph.store.scan(&[None, None, None]);
    let mut triples: Vec<_> = scan
        .rows
        .iter()
        .map(|row| {
            let spo = scan.to_spo(row);
            format!(
                "{} {} {}",
                graph.dict.term(spo[0]),
                graph.dict.term(spo[1]),
                graph.dict.term(spo[2])
            )
        })
        .collect();
    triples.sort();
    triples
}

fn dataset(graph: &Graph) -> Vec<(Option<Term>, Vec<String>)> {
    let mut result = vec![(None, contents(graph))];
    result.extend(
        graph
            .named
            .iter()
            .map(|(name, sub)| (Some(name.clone()), contents(sub))),
    );
    result
}

#[test]
fn interrupted_delete_preserves_acknowledged_survivor_files_and_wals() {
    let dir = TestDir::new("drop_survivors");
    let mut graph = deletion_fixture(&dir.0);
    let before = dataset(&graph);
    // Survivor data includes acknowledged WAL-only inserts. Deletion must leave every
    // survivor file and its physical path alone, so no unsynced replacement can lose it.
    let paths: Vec<_> = ["named/0", "named/2"]
        .into_iter()
        .flat_map(|slot| saved_paths(&dir.0.join(slot)))
        .filter(|path| path.is_file())
        .collect();
    let files: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let trace = Trace::start();
    let failure = Failure::at(Event::DropPublished(dir.0.clone()));
    assert!(graph.drop_named_durable(&named("remove")).is_err());
    failure.assert_hit();
    let events = trace.take();
    let published = position(&events, Event::ManifestPublished(dir.0.join("named.bin")));
    let reclaim = position(&events, Event::DropPublished(dir.0.clone()));
    assert!(
        events[published + 1..reclaim].contains(&Event::Synced(dir.0.clone())),
        "persist the new manifest before reclaiming the dropped slot"
    );
    drop(graph);
    for (path, bytes) in paths.iter().zip(files) {
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "survivor {path:?} was rewritten"
        );
    }
    let expected: Vec<_> = before
        .into_iter()
        .filter(|(name, _)| *name != Some(named("remove")))
        .collect();
    for _ in 0..2 {
        let reopened = Graph::open(&dir.0).unwrap();
        assert_eq!(dataset(&reopened), expected);
        assert_eq!(
            reopened.named[1].1.wal.as_ref().unwrap().dir,
            dir.0.join("named/2")
        );
    }
}

#[test]
fn interrupted_delete_manifest_publish_reopens_whole_old_or_new_dataset() {
    // Cover middle, trailing, last-only and batch drops, on both sides of publication.
    for batch in [false, true] {
        for target in ["first", "remove", "last", "only"] {
            for published in [false, true] {
                let dir = TestDir::new("drop_publish");
                let mut graph = deletion_fixture(&dir.0);
                if target == "only" {
                    graph.drop_named_durable(&named("first")).unwrap();
                    graph.drop_named_durable(&named("last")).unwrap();
                }
                let before = dataset(&graph);
                let removed = if target == "only" { "remove" } else { target };
                // An all-graphs drop publishes by removing the manifest; interrupt just
                // after that durable removal. Other pre-publication cases fail after the
                // complete temporary manifest has been fsync'd, before its atomic rename.
                let removes_manifest = batch || target == "only";
                if removes_manifest && !published {
                    continue;
                }
                let event = if published {
                    Event::DropPublished(dir.0.clone())
                } else {
                    Event::ManifestReady(dir.0.join("named.bin.tmp"))
                };
                let failure = Failure::at(event);
                let result = if batch {
                    graph.drop_all_named_durable().map(|()| true)
                } else {
                    graph.drop_named_durable(&named(removed))
                };
                assert!(result.is_err());
                failure.assert_hit();
                drop(graph);
                let expected: Vec<_> = before
                    .into_iter()
                    .filter(|(name, _)| {
                        !published || name.is_none() || (!batch && *name != Some(named(removed)))
                    })
                    .collect();
                for _ in 0..2 {
                    assert_eq!(dataset(&Graph::open(&dir.0).unwrap()), expected);
                }
            }
        }
    }
}

#[test]
fn sparse_slots_survive_create_drop_compact_and_legacy_reopen() {
    let dir = TestDir::new("sparse_slots");
    let mut graph = deletion_fixture(&dir.0);
    // The dense fixture retains legacy version 1 bytes.
    let version = |dir: &Path| {
        u32::from_le_bytes(
            std::fs::read(dir.join("named.bin")).unwrap()[4..8]
                .try_into()
                .unwrap(),
        )
    };
    assert_eq!(version(&dir.0), 1);
    graph.drop_named_durable(&named("remove")).unwrap();
    assert_eq!(version(&dir.0), 2);
    drop(graph);
    let mut graph = Graph::open(&dir.0).unwrap();
    graph
        .apply_delta_nquads(
            "<http://ex/new> <http://ex/p> <http://ex/o> <http://ex/added> .",
            "",
        )
        .unwrap();
    assert_eq!(
        graph.named.last().unwrap().1.wal.as_ref().unwrap().dir,
        dir.0.join("named/3")
    );
    assert_eq!(
        graph.named[1].1.wal.as_ref().unwrap().dir,
        dir.0.join("named/2")
    );
    let expected = dataset(&graph);
    drop(graph);
    let mut graph = Graph::open(&dir.0).unwrap();
    assert_eq!(dataset(&graph), expected);
    graph.compact().unwrap();
    assert_eq!(
        version(&dir.0),
        1,
        "compaction packs slots into the legacy dense format"
    );
    drop(graph);
    let graph = Graph::open(&dir.0).unwrap();
    assert_eq!(dataset(&graph), expected);
    assert_eq!(
        graph.named[1].1.wal.as_ref().unwrap().dir,
        dir.0.join("named/1")
    );
}

#[test]
fn compact_reclaims_sparse_manifests_without_a_default_overlay() {
    // Interior holes require renumbering survivors; trailing tombstones still require
    // a manifest rewrite even when the surviving physical slots are already dense.
    for trailing in [false, true] {
        for default_base in [false, true] {
            let dir = TestDir::new("compact_sparse_named");
            let base = if default_base {
                Graph::load_str("<http://ex/base> <http://ex/p> <http://ex/o> .", "ntriples")
                    .unwrap()
            } else {
                Graph::new()
            };
            base.save(&dir.0).unwrap();
            let mut graph = Graph::open(&dir.0).unwrap();
            let names: Vec<_> = (0..24).map(|i| named(&format!("graph-{i}"))).collect();
            for name in &names {
                let i = graph.ensure_named(name).unwrap();
                graph.named[i]
                    .1
                    .apply_delta(&[[named("ack"), named("p"), name.clone()]], &[])
                    .unwrap();
            }
            let keep = if trailing { 1 } else { 2 };
            for name in &names[1..names.len() - (keep - 1)] {
                assert!(graph.drop_named_durable(name).unwrap());
            }
            let expected = dataset(&graph);
            drop(graph);
            let mut graph = Graph::open(&dir.0).unwrap();
            assert!(!graph.store.has_overlay(), "parent has no pending triples");
            let manifest_path = dir.0.join("named.bin");
            let sparse = std::fs::read(&manifest_path).unwrap();
            assert_eq!(u32::from_le_bytes(sparse[4..8].try_into().unwrap()), 2);
            assert_eq!(u32::from_le_bytes(sparse[8..12].try_into().unwrap()), 24);

            graph.compact().unwrap();
            let dense = std::fs::read(&manifest_path).unwrap();
            assert_eq!(u32::from_le_bytes(dense[4..8].try_into().unwrap()), 1);
            assert_eq!(
                u32::from_le_bytes(dense[8..12].try_into().unwrap()),
                keep as u32
            );
            let live_name_bytes: usize = graph
                .named
                .iter()
                .map(|(name, _)| match name {
                    Term::NamedNode(name) => 5 + name.as_str().len(),
                    _ => unreachable!("fixture uses IRI graph names"),
                })
                .sum();
            assert_eq!(
                dense.len(),
                12 + live_name_bytes,
                "no tombstone bytes remain"
            );
            assert_eq!(
                std::fs::read_dir(dir.0.join("named")).unwrap().count(),
                keep
            );
            assert_eq!(dataset(&graph), expected);
            drop(graph);

            let mut graph = Graph::open(&dir.0).unwrap();
            assert_eq!(dataset(&graph), expected);
            for (i, (_, sub)) in graph.named.iter().enumerate() {
                assert_eq!(
                    sub.wal.as_ref().unwrap().dir,
                    dir.0.join("named").join(i.to_string())
                );
            }
            let next = graph.ensure_named(&named("after-compact")).unwrap();
            assert_eq!(next, keep);
            assert_eq!(
                graph.named[next].1.wal.as_ref().unwrap().dir,
                dir.0.join("named").join(keep.to_string())
            );
            assert!(contents(&graph.named[next].1).is_empty());
            let expected = dataset(&graph);
            drop(graph);
            assert_eq!(dataset(&Graph::open(&dir.0).unwrap()), expected);
        }
    }
}

#[test]
fn interrupted_delete_orphan_logs_are_not_replayed_into_a_new_graph() {
    let dir = TestDir::new("drop_orphan");
    let mut graph = deletion_fixture(&dir.0);
    let failure = Failure::at(Event::DropPublished(dir.0.clone()));
    assert!(graph.drop_named_durable(&named("last")).is_err());
    failure.assert_hit();
    drop(graph);
    assert!(
        dir.0.join("named/2/wal.log").exists(),
        "interruption leaves an orphan"
    );
    let mut graph = Graph::open(&dir.0).unwrap();
    graph
        .apply_delta_nquads(
            "<http://ex/fresh> <http://ex/p> <http://ex/o> <http://ex/new> .",
            "",
        )
        .unwrap();
    let expected = vec!["<http://ex/fresh> <http://ex/p> <http://ex/o>".to_string()];
    assert_eq!(contents(&graph.named[2].1), expected);
    drop(graph);
    let graph = Graph::open(&dir.0).unwrap();
    assert_eq!(contents(&graph.named[2].1), expected);
}

#[test]
fn sparse_manifest_rejects_truncated_or_nonempty_tombstones() {
    for payload in [&[2][..], &[2, 0, 0][..], &[2, 1, 0, 0, 0, b'x'][..]] {
        let dir = TestDir::new("bad_tombstone");
        Graph::new().save(&dir.0).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&NAMED_MANIFEST_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(payload);
        std::fs::write(dir.0.join("named.bin"), bytes).unwrap();
        assert_eq!(
            Graph::open(&dir.0).err().unwrap().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn interrupted_compaction_log_creation_reopens_without_losing_acknowledged_writes() {
    // Interrupt between each fresh log's file fsync and its directory fsync, for the
    // root and a named graph. These are real compaction swaps, not hand-built layouts.
    for slot in ["", "named/0"] {
        for log in ["wal.log", "txn.log"] {
            let dir = TestDir::new("compact_log_crash");
            let mut graph = deletion_fixture(&dir.0);
            let expected = dataset(&graph);
            let path = dir.0.join(slot).join(log);
            let trace = Trace::start();
            let failure = Failure::at(Event::LogFileSynced(path.clone()));
            assert!(graph.compact().is_err());
            failure.assert_hit();
            let events = trace.take();
            let root = dir.0.join(slot);
            let swapped = position(&events, Event::SwapPublished(root));
            let failed = position(&events, Event::LogFileSynced(path.clone()));
            assert!(
                swapped < failed,
                "must fail opening the replacement base's log"
            );
            drop(graph);
            // Model power loss discarding the newly created, unsynced directory entry.
            // No append was acknowledged on this handle: open/compaction must fail first.
            std::fs::remove_file(&path).unwrap();
            for _ in 0..2 {
                let graph = Graph::open(&dir.0).unwrap();
                assert_eq!(dataset(&graph), expected);
            }
        }
    }
}

#[test]
fn compacted_log_entries_are_synced_before_the_first_acknowledged_update() {
    let dir = TestDir::new("compact_log_sync");
    let mut graph = deletion_fixture(&dir.0);
    let trace = Trace::start();
    graph.compact().unwrap();
    let events = trace.take();
    for slot in ["", "named/0", "named/1", "named/2"] {
        let root = dir.0.join(slot);
        for log in ["wal.log", "txn.log"] {
            let path = root.join(log);
            // Named logs can be opened more than once during recursive compaction.
            // Check every instance, including opens in the final parent replacement.
            let opens: Vec<_> = events
                .iter()
                .enumerate()
                .filter_map(|(i, event)| {
                    (*event == Event::LogFileSynced(path.clone())).then_some(i)
                })
                .collect();
            assert!(!opens.is_empty(), "replacement log {path:?} was not synced");
            for opened in opens {
                assert!(
                    events[opened + 1..]
                        .iter()
                        .take_while(|event| {
                            !matches!(event, Event::LogFileSynced(_) | Event::JournalTruncated(_))
                        })
                        .any(|event| *event == Event::Synced(root.clone())),
                    "persist {path:?}'s entry before exposing the handle or retiring the journal"
                );
            }
        }
    }
    // Clear the parent journal after materializing updates to EXISTING slots, so reopening
    // can recover them only from the new per-graph WALs, rather than from transaction redo.
    let mut expected = dataset(&graph);
    let triple = [named("after-compact"), named("p"), named("o")];
    graph
        .commit_txn(&[
            (true, None, triple.clone()),
            (true, Some(named("last")), triple.clone()),
        ])
        .unwrap();
    graph.apply_delta(&[triple], &[]).unwrap();
    graph
        .apply_delta_nquads(
            "<http://ex/after-compact> <http://ex/p> <http://ex/o> <http://ex/last> .",
            "",
        )
        .unwrap();
    graph.clear_txn().unwrap();
    assert_eq!(std::fs::metadata(dir.0.join("txn.log")).unwrap().len(), 0);
    for (slot, triples) in &mut expected {
        if slot.is_none() || *slot == Some(named("last")) {
            triples.push("<http://ex/after-compact> <http://ex/p> <http://ex/o>".into());
            triples.sort();
        }
    }
    assert_eq!(dataset(&graph), expected);
    drop(graph);
    for _ in 0..2 {
        assert_eq!(dataset(&Graph::open(&dir.0).unwrap()), expected);
    }
}
