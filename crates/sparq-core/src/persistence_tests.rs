//! Ordering checks observe real fsync calls; they do not simulate power loss.

use super::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Synced(PathBuf),
    ManifestPublished(PathBuf),
    JournalTruncated(PathBuf),
    SwapPublished(PathBuf),
    RetiringOldBase(PathBuf),
}

thread_local! {
    static EVENTS: RefCell<Option<Vec<Event>>> = const { RefCell::new(None) };
}

pub(super) fn record(event: Event) {
    EVENTS.with_borrow_mut(|events| {
        if let Some(events) = events {
            events.push(event);
        }
    });
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
        let sub_sync = position(&events, Event::Synced(sub.clone()));
        for file in base_files {
            assert!(position(&events, Event::Synced(sub.join(file))) < sub_sync);
        }
        let named_sync = position(&events, Event::Synced(dir.0.join("named")));
        let root_sync = position(&events, Event::Synced(dir.0.clone()));
        let manifest = position(&events, Event::ManifestPublished(dir.0.join("named.bin")));
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
