//! Behavioral checks for named-graph layout recovery.

use super::*;
use std::path::Path;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
fn tempdir() -> std::io::Result<TempDir> {
    let path = std::env::temp_dir().join(format!("sparq-layout-{}", random_id()));
    std::fs::create_dir(&path)?;
    Ok(TempDir(std::fs::canonicalize(path)?))
}

fn name(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(format!("http://example/{s}")).unwrap())
}

fn triple() -> [Term; 3] {
    [name("s"), name("p"), name("o")]
}

fn store(path: &Path, count: usize) -> Graph {
    let mut seed = Graph::new();
    for i in 0..count {
        seed.ensure_named(&name(&format!("g{i}"))).unwrap();
    }
    seed.save(path).unwrap();
    Graph::open(path).unwrap()
}

type SyncEvents = Vec<(String, std::path::PathBuf)>;

struct SyncTrace(std::rc::Rc<std::cell::RefCell<SyncEvents>>);

impl SyncTrace {
    fn start() -> Self {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let trace = std::rc::Rc::clone(&events);
        fault::observe(move |step, path| {
            trace
                .borrow_mut()
                .push((step.to_string(), path.to_path_buf()));
        });
        Self(events)
    }

    fn take(&self) -> SyncEvents {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

impl Drop for SyncTrace {
    fn drop(&mut self) {
        fault::disarm();
    }
}

#[test]
fn log_entries_are_synced_before_open_returns() {
    for replacement in ["fresh", "existing", "compact", "restore"] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, 1);
        let trace = SyncTrace::start();
        match replacement {
            "compact" => {
                root.apply_delta(&[triple()], &[]).unwrap();
                root.compact().unwrap();
            }
            "restore" => {
                let mut fresh = Graph::new();
                fresh.ensure_named(&name("g0")).unwrap();
                root.restore_in_place(fresh).unwrap();
            }
            _ => {
                drop(root);
                if replacement == "fresh" {
                    for slot in [path.clone(), path.join("named/0")] {
                        for log in ["wal.log", "txn.log"] {
                            std::fs::remove_file(slot.join(log)).unwrap();
                        }
                    }
                }
                root = Graph::open(&path).unwrap();
            }
        }
        let events = trace.take();
        for slot in [path.clone(), path.join("named/0")] {
            for log in ["wal.log", "txn.log"] {
                let log_path = slot.join(log);
                let opened = events
                    .iter()
                    .position(|(step, p)| step == "log-opened" && *p == log_path)
                    .expect("log opened");
                let synced = events[..opened]
                    .iter()
                    .rposition(|(step, p)| step == "log-file-synced" && *p == log_path)
                    .unwrap_or_else(|| panic!("{replacement}: {log_path:?} was not synced"));
                #[cfg(unix)]
                assert!(
                    events[synced + 1..opened]
                        .iter()
                        .any(|(step, p)| step == "dir-synced" && *p == slot),
                    "{replacement}: persist {log_path:?}'s entry before returning its handle"
                );
                #[cfg(not(unix))]
                let _ = synced;
            }
        }
        root.apply_delta(&[triple()], &[]).unwrap();
        root.apply_delta_to_named(0, &[triple()], &[]).unwrap();
        root.clear_txn().unwrap();
        drop(root);
        for _ in 0..2 {
            let reopened = Graph::open(&path).unwrap();
            assert_eq!(reopened.len(), 1);
            assert_eq!(reopened.named_graph(&name("g0")).unwrap().len(), 1);
        }
    }
}

#[test]
fn interrupted_replacement_log_creation_keeps_acknowledged_data() {
    for slot in ["", "named/0"] {
        for log in ["wal.log", "txn.log"] {
            let tmp = tempdir().unwrap();
            let path = tmp.path().join("store");
            let mut root = store(&path, 1);
            root.apply_delta(&[triple()], &[]).unwrap();
            root.apply_delta_to_named(0, &[triple()], &[]).unwrap();
            let failed = path.join(slot).join(log);
            let target = failed.clone();
            fault::crash_at(move |step, p| step == "log-file-synced" && p == target);
            let result = root.compact();
            let hits = fault::disarm();
            assert!(
                result.is_err(),
                "opening a replacement log must fail before use"
            );
            assert!(hits > 0, "log sync crash point was not reached");
            drop(root);
            // Model loss of a fresh log entry whose parent was not synced. Its base
            // already holds every acknowledged update; no new append could succeed.
            std::fs::remove_file(failed).unwrap();
            for _ in 0..2 {
                let reopened = Graph::open(&path).unwrap();
                assert_eq!(reopened.len(), 1);
                assert_eq!(reopened.named_graph(&name("g0")).unwrap().len(), 1);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn saved_bases_are_synced_before_publication_and_journal_retirement() {
    for recovery in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, 0);
        root.commit_txn(&[(true, Some(name("g0")), triple())])
            .unwrap();
        let trace = SyncTrace::start();
        if recovery {
            drop(root);
            root = Graph::open(&path).unwrap();
        } else {
            let i = root.ensure_named(&name("g0")).unwrap();
            root.apply_delta_to_named(i, &[triple()], &[]).unwrap();
            root.clear_txn().unwrap();
        }
        let events = trace.take();
        let manifest = events
            .iter()
            .position(|(step, p)| step == "rename" && *p == path.join("named.bin.tmp"))
            .unwrap();
        let retired = events
            .iter()
            .rposition(|(step, p)| step == "journal-truncated" && *p == path)
            .unwrap();
        let child = path.join("named/0");
        let child_sync = events[..manifest]
            .iter()
            .rposition(|(step, p)| step == "dir-synced" && *p == child)
            .unwrap();
        for file in std::fs::read_dir(&child).unwrap() {
            let file = file.unwrap().path();
            if file.is_file()
                && !matches!(
                    file.file_name().unwrap().to_str(),
                    Some("wal.log" | "txn.log")
                )
            {
                assert!(
                    events[..child_sync]
                        .iter()
                        .any(|(step, p)| step == "file-synced" && *p == file),
                    "sync {file:?} before publication"
                );
            }
        }
        let named_sync = events[child_sync + 1..manifest]
            .iter()
            .position(|(step, p)| step == "dir-synced" && *p == path.join("named"))
            .unwrap()
            + child_sync
            + 1;
        assert!(events[named_sync + 1..manifest]
            .iter()
            .any(|(step, p)| step == "dir-synced" && *p == path));
        assert!(manifest < retired);
        drop(root);
        for _ in 0..2 {
            assert_eq!(
                Graph::open(&path)
                    .unwrap()
                    .named_graph(&name("g0"))
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn compact_syncs_recursive_bases_before_retiring_the_old_tree() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut seed = Graph::new();
    let child = seed.ensure_named(&name("g0")).unwrap();
    seed.named[child].1.ensure_named(&name("nested")).unwrap();
    seed.save(&path).unwrap();
    let mut root = Graph::open(&path).unwrap();
    root.apply_delta(&[triple()], &[]).unwrap();
    root.apply_delta_to_named(0, &[triple()], &[]).unwrap();
    root.named[0]
        .1
        .apply_delta_to_named(0, &[triple()], &[])
        .unwrap();
    let trace = SyncTrace::start();
    root.compact().unwrap();
    let events = trace.take();
    let staging = staging_path(&path, "compact-new");
    let publish = events
        .iter()
        .position(|(step, p)| step == "rename" && *p == path)
        .unwrap();
    for relative in ["", "named/0", "named/0/named/0"] {
        let live = path.join(relative);
        let saved = staging.join(relative);
        let synced = events[..publish]
            .iter()
            .rposition(|(step, p)| step == "dir-synced" && *p == saved)
            .unwrap();
        for file in std::fs::read_dir(live).unwrap() {
            let file = file.unwrap();
            if file.file_type().unwrap().is_file()
                && !matches!(
                    file.file_name().to_str(),
                    Some("wal.log" | "txn.log" | dir_lock::INSIDE_LOCK)
                )
            {
                assert!(
                    events[..synced].iter().any(
                        |(step, p)| step == "file-synced" && *p == saved.join(file.file_name())
                    ),
                    "sync {:?} before the swap",
                    file.path()
                );
            }
        }
    }
    let swapped = events
        .iter()
        .position(|(step, p)| step == "rename" && *p == staging)
        .unwrap();
    let retire = events
        .iter()
        .position(|(step, p)| {
            step == "remove-entry" && p.starts_with(staging_path(&path, "compact-old"))
        })
        .unwrap();
    assert!(events[swapped + 1..retire]
        .iter()
        .any(|(step, p)| step == "dir-synced" && p == tmp.path()));
    drop(root);
    for _ in 0..2 {
        let reopened = Graph::open(&path).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened.named[0].1.len(), 1);
        assert_eq!(reopened.named[0].1.named[0].1.len(), 1);
    }
}

#[test]
fn interrupted_named_deletions_reopen_the_complete_old_or_new_set() {
    for count in [1, 3] {
        for removed in 0..=count {
            for published in [false, true] {
                let tmp = tempdir().unwrap();
                let path = tmp.path().join("store");
                let mut root = store(&path, count);
                root.apply_delta(&[triple()], &[]).unwrap();
                for i in 0..count {
                    let t = [name(&format!("ack{i}")), name("p"), name("o")];
                    root.apply_delta_to_named(i, &[t], &[]).unwrap();
                }
                let survivors: Vec<_> = (0..count)
                    .filter(|&i| removed != count && removed != i)
                    .flat_map(|i| {
                        std::fs::read_dir(path.join("named").join(i.to_string()))
                            .unwrap()
                            .map(|entry| entry.unwrap().path())
                    })
                    .filter(|path| path.is_file())
                    .map(|path| {
                        let bytes = std::fs::read(&path).unwrap();
                        (path, bytes)
                    })
                    .collect();
                let target = if published {
                    path.join("named")
                } else {
                    path.join("named.bin.tmp")
                };
                fault::crash_at(move |step, p| {
                    if published {
                        step == "remove-tree" && p.starts_with(&target)
                    } else {
                        step == "rename" && p == target
                    }
                });
                let result = if removed == count {
                    root.drop_all_named_durable()
                } else {
                    root.drop_named_durable(&name(&format!("g{removed}")))
                        .map(|_| ())
                };
                let hits = fault::disarm();
                assert!(result.is_err());
                assert!(hits > 0);
                drop(root);
                for (file, bytes) in survivors {
                    assert_eq!(
                        std::fs::read(&file).unwrap(), bytes,
                        "survivor {file:?} was rewritten"
                    );
                }
                for _ in 0..2 {
                    let reopened = Graph::open(&path).unwrap();
                    assert_eq!(reopened.len(), 1);
                    for i in 0..count {
                        let graph = reopened.named_graph(&name(&format!("g{i}")));
                        if published && (removed == count || removed == i) {
                            assert!(graph.is_none());
                        } else {
                            let graph = graph.unwrap();
                            assert_eq!(graph.len(), 1);
                            assert!(graph.id_of(&name(&format!("ack{i}"))).is_some());
                            assert_eq!(
                                graph.wal.dir().unwrap(),
                                path.join("named").join(i.to_string())
                            );
                        }
                    }
                }
                let mut reopened = Graph::open(&path).unwrap();
                let fresh = reopened.ensure_named(&name("fresh")).unwrap();
                assert!(
                    reopened.named[fresh].1.is_empty(),
                    "orphan logs must not enter a new graph"
                );
                reopened
                    .apply_delta_to_named(fresh, &[triple()], &[])
                    .unwrap();
                drop(reopened);
                let reopened = Graph::open(&path).unwrap();
                let fresh = reopened.named_graph(&name("fresh")).unwrap();
                assert_eq!(fresh.len(), 1);
                assert!(fresh.id_of(&name("ack0")).is_none());
            }
        }
    }
}

#[cfg(windows)]
#[test]
fn directory_sync_is_a_noop_on_windows() {
    let tmp = tempdir().unwrap();
    let file = tmp.path().join("file");
    std::fs::write(&file, b"unchanged").unwrap();
    for path in [tmp.path(), &file, &tmp.path().join("missing")] {
        fsync_dir(path).unwrap();
    }
    assert_eq!(std::fs::read(file).unwrap(), b"unchanged");
}

#[test]
fn compact_reclaims_sparse_slots_without_any_overlay() {
    fn level(graph: &mut Graph, nested: bool) -> &mut Graph {
        if nested {
            &mut graph.named[0].1
        } else {
            graph
        }
    }
    for nested in [false, true] {
        for trailing in [false, true] {
            for default_base in [false, true] {
                let tmp = tempdir().unwrap();
                let path = tmp.path().join("store");
                let layer_path = if nested {
                    path.join("named/0")
                } else {
                    path.clone()
                };
                let mut seed = Graph::new();
                if nested {
                    seed.ensure_named(&name("outer")).unwrap();
                }
                let layer = level(&mut seed, nested);
                if default_base {
                    layer.apply_delta(&[triple()], &[]).unwrap();
                }
                for i in 0..8 {
                    let n = name(&format!("g{i}"));
                    let slot = layer.ensure_named(&n).unwrap();
                    layer
                        .apply_delta_to_named(slot, &[[n, name("p"), name("o")]], &[])
                        .unwrap();
                }
                seed.save(&path).unwrap();
                let mut root = Graph::open(&path).unwrap();
                let keep = if trailing { 1 } else { 2 };
                for i in 1..8 - (keep - 1) {
                    level(&mut root, nested)
                        .drop_named_durable(&name(&format!("g{i}")))
                        .unwrap();
                }
                drop(root);
                let mut root = Graph::open(&path).unwrap();
                assert!(
                    !root.tree_has_overlay(),
                    "the complete tree has clean bases"
                );
                assert_eq!(level(&mut root, nested).next_slot, 8);
                root.compact().unwrap();
                let manifest = read_named_manifest(&layer_path).unwrap().unwrap();
                assert_eq!(
                    manifest.next_slot as usize, keep,
                    "compaction must reclaim the allocation cursor"
                );
                assert_eq!(
                    manifest
                        .entries
                        .iter()
                        .map(|(slot, _)| *slot)
                        .collect::<Vec<_>>(),
                    (0..keep as u32).collect::<Vec<_>>()
                );
                assert_eq!(
                    std::fs::read_dir(layer_path.join("named")).unwrap().count(),
                    keep
                );
                drop(root);
                for _ in 0..2 {
                    let mut reopened = Graph::open(&path).unwrap();
                    let layer = level(&mut reopened, nested);
                    assert_eq!(layer.len(), usize::from(default_base));
                    assert_eq!(layer.named.len(), keep);
                    for (n, sub) in &layer.named {
                        assert_eq!(sub.len(), 1);
                        assert!(sub.id_of(n).is_some());
                    }
                }
                let mut root = Graph::open(&path).unwrap();
                let layer = level(&mut root, nested);
                let next = layer.ensure_named(&name("after-compact")).unwrap();
                assert_eq!(
                    layer.named[next].1.wal.dir().unwrap(),
                    layer_path.join("named").join(keep.to_string())
                );
                assert!(layer.named[next].1.is_empty());
                drop(root);
                let mut root = Graph::open(&path).unwrap();
                assert!(level(&mut root, nested)
                    .named_graph(&name("after-compact"))
                    .unwrap()
                    .is_empty());
            }
        }
    }
}

#[test]
fn durability_barrier_preserves_open_mapped_files_and_logs() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    Graph::load_dataset(
        "<http://example/s> <http://example/p> <http://example/o> .\n\
         <http://example/s> <http://example/p> <http://example/o> <http://example/g0> .",
        "nquads",
    )
    .unwrap()
    .save(&path)
    .unwrap();
    let root = Graph::open(&path).unwrap();
    let child = path.join("named/0");
    let files = [
        path.join("dict-terms.bin"),
        path.join("wal.log"),
        child.join("dict-terms.bin"),
        child.join("wal.log"),
    ];
    let before = files.each_ref().map(|file| std::fs::read(file).unwrap());
    durability_barrier(&path, &[&child]).unwrap();
    for (file, bytes) in files.iter().zip(before) {
        assert_eq!(std::fs::read(file).unwrap(), bytes, "{}", file.display());
    }
    assert_eq!(root.len(), 1);
    assert_eq!(root.named_graph(&name("g0")).unwrap().len(), 1);

    let missing = path.join("missing.bin");
    assert_eq!(
        fsync_file(&missing).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(!missing.exists());
}

#[test]
fn detached_child_survives_sibling_create_and_drop() {
    for create in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, 2);
        let (n, mut child) = root.named.pop().unwrap();
        if create {
            root.ensure_named(&name("new")).unwrap();
        } else {
            root.drop_named_durable(&name("g0")).unwrap();
        }
        child.apply_delta(&[triple()], &[]).unwrap();
        drop(child);
        drop(root);
        let reopened = Graph::open(&path).unwrap();
        assert_eq!(reopened.named_graph(&n).unwrap().len(), 1);
    }
}

#[test]
fn stale_child_drop_changes_only_the_attached_set() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 4);
    let (_, stale) = root.named.remove(1);
    root.drop_named_durable(&name("g0")).unwrap();
    root.apply_delta(&[triple()], &[]).unwrap();
    root.compact().unwrap();
    // The rewrite assigns g3 to slot 1, which the detached handle previously held.
    root.named.push((name("old"), stale));
    assert!(root.drop_named_durable(&name("old")).unwrap());
    let i = root.ensure_named(&name("g3")).unwrap();
    root.apply_delta_to_named(i, &[triple()], &[]).unwrap();
    drop(root);
    let reopened = Graph::open(&path).unwrap();
    assert_eq!(reopened.named_graph(&name("g3")).unwrap().len(), 1);
    assert_eq!(reopened.named_graph(&name("g2")).unwrap().len(), 0);
    assert!(path.join("named/0").is_dir());
    assert!(path.join("named/1").is_dir());
}

#[test]
fn stale_child_refuses_manifest_republication() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 2);
    let (n, stale) = root.named.pop().unwrap();
    root.apply_delta(&[triple()], &[]).unwrap();
    root.compact().unwrap();
    root.named.push((n, stale));
    assert!(root.ensure_named(&name("new")).is_err());
    assert_eq!(
        read_named_manifest(&path).unwrap().unwrap().entries.len(),
        1
    );
}

#[test]
fn duplicate_check_includes_the_dropped_child() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 2);
    let mut alias =
        Graph::open_dir(&path.join("named/1"), root.lock.clone(), true, Role::Child).unwrap();
    alias.tree = root.tree.clone();
    alias.generation = root.generation;
    root.named.push((name("g1"), alias));
    assert!(root.drop_named_durable(&name("g1")).is_err());
    assert!(path.join("named/1").is_dir());
    assert_eq!(
        read_named_manifest(&path).unwrap().unwrap().entries.len(),
        2
    );
}

#[test]
fn invalid_declared_layout_leaves_all_slots_intact() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 2));
    let mut manifest = read_named_manifest(&path).unwrap().unwrap();
    manifest.entries[1].0 = 3;
    write_named_manifest(&path, &manifest.entries, 4).unwrap();
    let before = std::fs::read(path.join("named/1/dict-meta.bin")).unwrap();
    let err = Graph::open(&path).err().expect("invalid layout");
    let missing = path.join("named").join("3").display().to_string();
    assert!(err.to_string().contains(missing.as_str()), "{err}");
    assert_eq!(
        std::fs::read(path.join("named/1/dict-meta.bin")).unwrap(),
        before
    );
    assert!(path.join("named/0").is_dir());
}

#[test]
fn declared_slots_require_real_child_directories_before_cleanup() {
    for file in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        drop(store(&path, 2));
        let slot = path.join("named/1");
        if file {
            std::fs::remove_dir_all(&slot).unwrap();
            std::fs::write(&slot, b"preserve").unwrap();
        } else {
            write_store_marker(&slot, Role::Root).unwrap();
        }
        let orphan = path.join("named/9");
        std::fs::create_dir(&orphan).unwrap();
        std::fs::write(orphan.join("preserve"), b"preserve").unwrap();
        assert!(Graph::open(&path).is_err());
        assert_eq!(std::fs::read(orphan.join("preserve")).unwrap(), b"preserve");
        assert!(path.join("named/0").is_dir());
    }
}

#[cfg(unix)]
#[test]
fn linked_named_container_is_refused_before_open_or_drop_all() {
    use std::os::unix::fs::symlink;
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 1);
    let elsewhere = tmp.path().join("children");
    std::fs::rename(path.join("named"), &elsewhere).unwrap();
    symlink(&elsewhere, path.join("named")).unwrap();
    let before = std::fs::read(path.join(NAMED_MANIFEST)).unwrap();
    assert!(root.drop_all_named_durable().is_err());
    assert_eq!(std::fs::read(path.join(NAMED_MANIFEST)).unwrap(), before);
    drop(root);
    assert!(Graph::open(&path).is_err());
    assert!(elsewhere.join("0").is_dir());
}

#[test]
fn unremovable_undeclared_slot_does_not_fail_open() {
    const CHILD_STORE: &str = "SPARQ_TEST_UNREMOVABLE_SLOT_STORE";
    if let Some(path) = std::env::var_os(CHILD_STORE) {
        let path = std::path::PathBuf::from(path);
        let failed = path.join("named/9");
        fault::fail_when(move |step, p| step == "remove-tree" && p == failed);
        let opened = Graph::open(&path);
        assert_eq!(fault::disarm(), 1);
        assert_eq!(opened.unwrap().named().len(), 1);
        assert!(path.join("named/9").is_dir());
        return;
    }
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 1));
    std::fs::create_dir(path.join("named/9")).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "layout_tests::unremovable_undeclared_slot_does_not_fail_open",
            "--nocapture",
        ])
        .env(CHILD_STORE, &path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(warning.contains("warning:"), "{warning}");
    assert!(
        warning.contains(&path.join("named/9").display().to_string()),
        "{warning}"
    );
    assert!(path.join("named/9").is_dir());
}

#[test]
fn second_open_cannot_sweep_an_inflight_slot() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 1);
    let slot = root.claim_named_slot(&path.join("named")).unwrap();
    let inflight = path.join("named").join(slot.to_string());
    std::fs::write(inflight.join("pending"), b"pending").unwrap();
    for readonly in [false, true] {
        let err = if readonly {
            Graph::open_read_only(&path)
        } else {
            Graph::open(&path)
        }
        .err()
        .expect("store held");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(std::fs::read(inflight.join("pending")).unwrap(), b"pending");
    }
    drop(root);
    let reader = Graph::open_read_only(&path).unwrap();
    assert!(inflight.is_dir());
    drop(reader);
    drop(Graph::open(&path).unwrap());
    assert!(!inflight.exists());
}

#[test]
fn slot_reservation_skips_existing_and_freezes_on_other_errors() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 1);
    std::fs::create_dir(path.join("named/1")).unwrap();
    let i = root.ensure_named(&name("fresh")).unwrap();
    assert_eq!(root.named[i].1.named_slot_in(&path.join("named")), Some(2));
    let failed = path.join("named/3");
    fault::fail_when(move |step, p| step == "reserve-slot" && p == failed);
    assert!(root.ensure_named(&name("failed")).is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(root.is_frozen());
    assert!(root.apply_delta(&[triple()], &[]).is_err());
}

#[test]
fn last_child_drop_records_the_container() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 2);
    let (_, mut detached) = root.named.pop().unwrap();
    // Exercise cleanup of a previously undeclared child under the emptying container.
    write_named_manifest(&path, &[(0, name("g0"))], 2).unwrap();
    root.drop_named_durable(&name("g0")).unwrap();
    assert!(detached.apply_delta(&[triple()], &[]).is_err());
}

#[test]
fn duplicate_and_drop_all_work_counts_are_linear() {
    fn cost(count: usize) -> (usize, usize) {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, count);
        layout_cost::reset();
        root.ensure_named(&name("fresh")).unwrap();
        assert_eq!(layout_cost::counts().0, count);
        layout_cost::reset();
        root.drop_named_durable(&name("fresh")).unwrap();
        let probes = layout_cost::counts().0;
        assert_eq!(probes, count + 1);
        layout_cost::reset();
        root.drop_all_named_durable().unwrap();
        (probes, layout_cost::counts().1)
    }
    let small = cost(8);
    let large = cost(16);
    assert_eq!(large.0 - 1, 2 * (small.0 - 1));
    assert_eq!(small.1, 1);
    assert_eq!(large.1, 1);
}

#[test]
fn aggregate_replay_reservation_errors_preserve_both_logs() {
    for journal in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, 0);
        for _ in 0..2 {
            if journal {
                root.commit_txn(&[(true, None, triple())]).unwrap();
            } else {
                root.apply_delta(&[triple()], &[]).unwrap();
            }
        }
        drop(root);
        let log = path.join(if journal { "txn.log" } else { "wal.log" });
        let before = std::fs::read(&log).unwrap();
        let mut seen = 0;
        fault::fail_when(move |step, _| {
            if step != "reserve-output" {
                return false;
            }
            seen += 1;
            seen == 2
        });
        assert!(Graph::open(&path).is_err());
        assert_eq!(fault::disarm(), 1);
        assert_eq!(std::fs::read(&log).unwrap(), before);
    }
}

#[test]
fn root_cache_sync_errors_refuse_saves_builds_and_publication() {
    const NT: &[u8] = b"<http://example/s> <http://example/p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";
    for file in [NUMERIC_CACHE_FILE, TEMPORAL_CACHE_FILE, "predstats.bin"] {
        for operation in ["save", "save-compressed", "build", "build-quads", "create"] {
            let tmp = tempdir().unwrap();
            let path = tmp.path().join("store");
            let _serial = io_fault::lock();
            if operation == "create" {
                let mut root = store(&path, 1);
                let before = std::fs::read(path.join(NAMED_MANIFEST)).unwrap();
                let _fault = io_fault::fail_sync_for(&path.join(file));
                assert!(root.ensure_named(&name("fresh")).is_err(), "{file}");
                assert_eq!(std::fs::read(path.join(NAMED_MANIFEST)).unwrap(), before);
                assert!(root.is_frozen());
            } else {
                let _fault = io_fault::fail_sync_for(&path.join(file));
                let graph = Graph::load_dataset(std::str::from_utf8(NT).unwrap(), "nt").unwrap();
                let result = match operation {
                    "save" => graph.save(&path).map_err(|e| e.to_string()),
                    "save-compressed" => graph.save_compressed(&path).map_err(|e| e.to_string()),
                    "build" => Graph::build_external_opts(NT, "ntriples", &path, 2, false),
                    "build-quads" => Graph::build_external_quads(NT, "nquads", &path, 2),
                    _ => unreachable!(),
                };
                assert!(result.is_err(), "{operation}: {file}");
                assert!(result.unwrap_err().contains("injected fsync fault"));
                assert!(!path.join(NAMED_MANIFEST).exists());
            }
        }
    }
}

#[test]
fn replay_output_capacity_overflow_is_an_error() {
    let mut out = Vec::<TxnOp>::new();
    assert!(reserve_replay_output(&mut out, usize::MAX, Path::new("txn.log")).is_err());
    assert!(out.is_empty());
}

#[test]
fn wal_truncation_error_freezes_root_and_children() {
    for (children, child) in [(0, false), (1, false), (1, true)] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, children);
        let target = if child {
            path.join("named/0")
        } else {
            path.clone()
        };
        fault::fail_when(move |step, p| step == "wal-truncate" && p == target);
        assert!(root.compact().is_err());
        assert_eq!(fault::disarm(), 1);
        assert!(root.is_frozen());
        assert!(root.apply_delta(&[triple()], &[]).is_err());
        if children > 0 {
            assert!(root.apply_delta_to_named(0, &[triple()], &[]).is_err());
        }
        drop(root);
        let reopened = Graph::open(&path).unwrap();
        assert_eq!(reopened.len(), 0);
        assert_eq!(reopened.named().len(), children);
        if children > 0 {
            assert_eq!(reopened.named_graph(&name("g0")).unwrap().len(), 0);
        }
    }
}

#[test]
fn journal_clear_error_refuses_writes_across_the_tree() {
    for child in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, 2);
        root.apply_delta(&[triple()], &[]).unwrap();
        let target = if child {
            &mut root.named[0].1
        } else {
            &mut root
        };
        target.commit_txn(&[(true, None, triple())]).unwrap();
        let log = target.wal.dir().unwrap().join("txn.log");
        let before = std::fs::read(&log).unwrap();
        target.txn.as_mut().unwrap().file = std::fs::File::open(&log).unwrap();
        assert!(target.clear_txn().is_err());
        assert_eq!(std::fs::read(&log).unwrap(), before);
        assert!(root.is_frozen());
        assert!(root.apply_delta(&[triple()], &[]).is_err());
        for index in 0..2 {
            assert!(root.apply_delta_to_named(index, &[triple()], &[]).is_err());
        }
        assert!(root.commit_txn(&[(true, None, triple())]).is_err());
        drop(root);
        let reopened = Graph::open(&path).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(
            reopened.named_graph(&name("g0")).unwrap().len(),
            usize::from(child)
        );
    }
}

#[test]
fn wal_append_error_refuses_writes_across_the_tree() {
    for (journal, child) in [(false, false), (false, true), (true, false), (true, true)] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let _serial = io_fault::lock();
        let mut root = store(&path, 2);
        root.apply_delta(&[triple()], &[]).unwrap();
        root.apply_delta_to_named(0, &[triple()], &[]).unwrap();
        let unacknowledged = [name("unacknowledged"), name("p"), name("o")];
        let point = if journal {
            &io_fault::JOURNAL_DROP_TAIL
        } else {
            &io_fault::WAL_DROP_TAIL
        };
        io_fault::arm(point, 1);
        let target = if child {
            &mut root.named[1].1
        } else {
            &mut root
        };
        let failed = if journal {
            target.commit_txn(&[(true, None, unacknowledged.clone())])
        } else {
            target.apply_delta(std::slice::from_ref(&unacknowledged), &[])
        };
        assert!(failed.is_err());
        assert!(root.is_frozen());
        assert!(root
            .apply_delta(std::slice::from_ref(&unacknowledged), &[])
            .is_err());
        for index in 0..2 {
            assert!(root
                .apply_delta_to_named(index, std::slice::from_ref(&unacknowledged), &[])
                .is_err());
        }
        assert!(root.commit_txn(&[(true, None, unacknowledged)]).is_err());
        drop(root);
        let reopened = Graph::open(&path).unwrap();
        assert_eq!(reopened.len(), 1);
        assert!(reopened.id_of(&name("unacknowledged")).is_none());
        assert_eq!(reopened.named_graph(&name("g0")).unwrap().len(), 1);
        assert_eq!(reopened.named_graph(&name("g1")).unwrap().len(), 0);
    }
}

#[test]
fn slot_records_are_retained_until_a_tree_swap() {
    for count in [8, 16] {
        let tree = TreeState::detached();
        layout_cost::reset();
        for i in 0..count {
            tree.record_replacement(&Path::new("store/named").join(i.to_string()));
        }
        assert_eq!(layout_cost::counts().1, count);
        assert_eq!(
            layout_cost::counts().2,
            0,
            "slot records must not scan the log"
        );
        assert_eq!(tree.layout.lock().unwrap().replaced.len(), count);
        tree.record_tree_replacement(Path::new("store"));
        assert_eq!(layout_cost::counts().2, count + 1);
        assert_eq!(tree.layout.lock().unwrap().replaced.len(), 1);
    }
}

#[test]
fn invalid_layout_preserves_recorded_child_staging_before_recovery() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 2));
    let child = path.join("named/0");
    let staging = staging_path(&child, "compact-old");
    let id = read_marker(&child).unwrap().unwrap().id.unwrap();
    std::fs::rename(&child, &staging).unwrap();
    mark_staging(&staging, "compact-old", &id).unwrap();
    let mut manifest = read_named_manifest(&path).unwrap().unwrap();
    manifest.entries[1].0 = 3;
    write_named_manifest(&path, &manifest.entries, 4).unwrap();
    let before = std::fs::read(staging.join("dict-meta.bin")).unwrap();
    let err = Graph::open(&path).err().expect("invalid declared layout");
    let missing = path.join("named").join("3").display().to_string();
    assert!(err.to_string().contains(missing.as_str()), "{err}");
    assert!(!child.exists());
    assert_eq!(
        std::fs::read(staging.join("dict-meta.bin")).unwrap(),
        before
    );
    assert!(staging.join(STAGING_MARKER).is_file());
    assert!(path.join("named/1").is_dir());
}

#[test]
fn nested_layout_validation_precedes_slot_cleanup() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 2);
    root.named[1].1.ensure_named(&name("nested")).unwrap();
    drop(root);
    let child = path.join("named/1");
    write_named_manifest(&child, &[(4, name("nested"))], 5).unwrap();
    let orphan = path.join("named/9");
    std::fs::create_dir(&orphan).unwrap();
    std::fs::write(orphan.join("preserve"), b"preserve").unwrap();
    let sibling_orphan = path.join("named/0/named/9");
    std::fs::create_dir_all(&sibling_orphan).unwrap();
    std::fs::write(sibling_orphan.join("preserve"), b"preserve").unwrap();
    assert!(Graph::open(&path).is_err());
    assert_eq!(std::fs::read(orphan.join("preserve")).unwrap(), b"preserve");
    assert_eq!(
        std::fs::read(sibling_orphan.join("preserve")).unwrap(),
        b"preserve"
    );
    assert!(child.join("named/0").is_dir());
}

#[test]
fn undeclared_root_store_is_preserved_during_cleanup() {
    for legacy in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        drop(store(&path, 1));
        let outside = tmp.path().join("other");
        Graph::load_dataset(
            "<http://example/s> <http://example/p> <http://example/o> .",
            "ntriples",
        )
        .unwrap()
        .save(&outside)
        .unwrap();
        if legacy {
            std::fs::remove_file(outside.join(STORE_MARKER)).unwrap();
        }
        let orphan = path.join("named/9");
        std::fs::rename(&outside, &orphan).unwrap();
        let before = std::fs::read(orphan.join("dict-meta.bin")).unwrap();
        let mut opened = Graph::open(&path).unwrap();
        assert_eq!(std::fs::read(orphan.join("dict-meta.bin")).unwrap(), before);
        assert!(opened.drop_all_named_durable().is_err());
        assert_eq!(std::fs::read(orphan.join("dict-meta.bin")).unwrap(), before);
    }
}

fn assert_drop_refuses_undeclared_store(legacy: bool) {
    fn image_files(path: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        let mut files = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                for (relative, bytes) in image_files(&entry.path()) {
                    files.insert(
                        std::path::PathBuf::from(entry.file_name()).join(relative),
                        bytes,
                    );
                }
            } else {
                files.insert(
                    entry.file_name().into(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
        files
    }

    for all in [false, true] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let base = [name("base"), name("p"), name("o")];
        let mut seed = Graph::new();
        seed.apply_delta(std::slice::from_ref(&base), &[]).unwrap();
        let graph = name("declared");
        let slot = seed.ensure_named(&graph).unwrap();
        seed.apply_delta_to_named(slot, &[triple()], &[]).unwrap();
        seed.save(&path).unwrap();

        let outside = tmp.path().join("other");
        seed.save(&outside).unwrap();
        if legacy {
            std::fs::remove_file(outside.join(STORE_MARKER)).unwrap();
        }
        let residue = path.join("named/9");
        std::fs::rename(&outside, &residue).unwrap();
        let before = image_files(&residue);
        let manifest = std::fs::read(path.join(NAMED_MANIFEST)).unwrap();
        let mut root = Graph::open(&path).unwrap();
        assert_eq!(root.named.len(), 1);
        assert_eq!(
            read_named_manifest(&path).unwrap().unwrap().entries,
            vec![(0, graph.clone())]
        );
        assert_eq!(image_files(&residue), before);
        let err = if all {
            root.drop_all_named_durable().unwrap_err()
        } else {
            root.drop_named_durable(&graph).unwrap_err()
        };
        assert!(err.contains(&residue.display().to_string()), "{err}");
        for kind in ["drop-new", "drop-old"] {
            assert!(!staging_path(&path.join("named"), kind).exists());
        }
        assert_eq!(std::fs::read(path.join(NAMED_MANIFEST)).unwrap(), manifest);
        assert_eq!(image_files(&residue), before);
        drop(root);

        let mut reopened = Graph::open(&path).unwrap();
        assert_eq!(reopened.named.len(), 1);
        let triples = |g: &Graph| {
            g.iter_ids()
                .map(|spo| spo.map(|id| g.dict.term(id)))
                .collect::<Vec<_>>()
        };
        assert_eq!(triples(&reopened), vec![base.clone()]);
        assert_eq!(
            triples(reopened.named_graph(&graph).unwrap()),
            vec![triple()]
        );
        // Log idempotent inserts to confirm both graphs reopened writable.
        reopened.apply_delta(&[base], &[]).unwrap();
        reopened.apply_delta_to_named(0, &[triple()], &[]).unwrap();
        assert_eq!(image_files(&residue), before);
    }
}

#[test]
fn drop_refuses_undeclared_root_store_before_publication() {
    assert_drop_refuses_undeclared_store(false);
}

#[test]
fn drop_refuses_undeclared_unmarked_store_before_publication() {
    assert_drop_refuses_undeclared_store(true);
}

#[test]
fn drops_accept_removable_residue_and_legacy_declared_slots() {
    for all in [false, true] {
        for residue in [false, true] {
            let tmp = tempdir().unwrap();
            let path = tmp.path().join("store");
            drop(store(&path, 1));
            let child = path.join("named/0");
            std::fs::remove_file(child.join(STORE_MARKER)).unwrap();
            let mut root = Graph::open(&path).unwrap();
            assert_eq!(read_store_marker(&child).unwrap(), Some(Role::Child));
            if residue {
                let orphan = path.join("named/9");
                std::fs::create_dir(&orphan).unwrap();
                std::fs::write(orphan.join("discard"), b"discard").unwrap();
            }
            if all {
                root.drop_all_named_durable().unwrap();
            } else {
                assert!(root.drop_named_durable(&name("g0")).unwrap());
            }
            assert!(!path.join("named").exists());
            drop(root);
            assert!(Graph::open(&path).unwrap().named.is_empty());
        }
    }
}

#[test]
fn single_drop_keeps_unremovable_residue_when_the_container_survives() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 2);
    let outside = tmp.path().join("other");
    Graph::new().save(&outside).unwrap();
    let residue = path.join("named/9");
    std::fs::rename(&outside, &residue).unwrap();
    let marker = std::fs::read(residue.join(STORE_MARKER)).unwrap();
    assert!(root.drop_named_durable(&name("g0")).unwrap());
    assert_eq!(root.named.len(), 1);
    assert_eq!(std::fs::read(residue.join(STORE_MARKER)).unwrap(), marker);
    assert!(!path.join("named/0").exists());
    drop(root);
    let reopened = Graph::open(&path).unwrap();
    assert_eq!(reopened.named.len(), 1);
    assert!(reopened.named_graph(&name("g1")).is_some());
    assert_eq!(std::fs::read(residue.join(STORE_MARKER)).unwrap(), marker);
}

#[test]
fn journal_upgrade_preserves_existing_temporary_file_links() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 0));
    let meta = path.join("dict-meta.bin");
    let mut bytes = std::fs::read(&meta).unwrap();
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    std::fs::write(&meta, bytes).unwrap();
    let preserved = tmp.path().join("preserved.bin");
    std::fs::write(&preserved, b"preserve").unwrap();
    std::fs::hard_link(&preserved, path.join("dict-meta.bin.tmp")).unwrap();
    let mut root = Graph::open(&path).unwrap();
    root.commit_txn_ops(&[TxnOp::Create(name("fresh"))])
        .unwrap();
    assert_eq!(std::fs::read(&preserved).unwrap(), b"preserve");
    drop(root);
    assert!(Graph::open(&path)
        .unwrap()
        .named_graph(&name("fresh"))
        .is_some());
}

#[test]
fn journal_upgrade_publication_error_prevents_frame_append() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 0));
    let meta = path.join("dict-meta.bin");
    let mut before = std::fs::read(&meta).unwrap();
    before[4..8].copy_from_slice(&1u32.to_le_bytes());
    std::fs::write(&meta, &before).unwrap();
    let mut root = Graph::open(&path).unwrap();
    let temporary = path.join("dict-meta.bin.tmp");
    fault::fail_when(move |step, p| step == "rename" && p == temporary);
    assert!(root
        .commit_txn_ops(&[TxnOp::Create(name("fresh"))])
        .is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(root.is_frozen());
    assert_eq!(std::fs::read(&meta).unwrap(), before);
    assert!(std::fs::read(path.join("txn.log")).unwrap().is_empty());
    drop(root);
    assert!(Graph::open(&path).unwrap().named().is_empty());
}

#[test]
fn nested_layout_validation_precedes_retained_container_cleanup() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    let mut root = store(&path, 1);
    root.named[0].1.ensure_named(&name("nested")).unwrap();
    let named = path.join("named");
    let old = staging_path(&named, "drop-old");
    root.named[0]
        .1
        .save_unlocked(&old.join("0"), Role::Child)
        .unwrap();
    drop(root);
    let id = read_marker(&path).unwrap().unwrap().id.unwrap();
    mark_staging(&old, "drop-old", &id).unwrap();
    write_named_manifest(&named.join("0"), &[(4, name("nested"))], 5).unwrap();
    let retained = old.join("0/named/0/dict-meta.bin");
    let before = std::fs::read(&retained).unwrap();
    assert!(Graph::open(&path).is_err());
    assert_eq!(std::fs::read(&retained).unwrap(), before);
    assert!(old.join(STAGING_MARKER).is_file());
}

#[test]
fn declared_nested_swaps_remain_recoverable_during_container_recovery() {
    for (has_new, has_old, record_new, record_old) in [
        (true, true, true, false),
        (true, true, false, true),
        (false, true, false, true),
        (true, false, true, false),
    ] {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("store");
        let mut root = store(&path, 1);
        let child = &mut root.named[0].1;
        child.ensure_named(&name("nested")).unwrap();
        child.apply_delta_to_named(0, &[triple()], &[]).unwrap();
        let leaf = path.join("named/0/named/0");
        let new = staging_path(&leaf, "compact-new");
        let old = staging_path(&leaf, "compact-old");
        let id = read_marker(&leaf).unwrap().unwrap().id.unwrap();
        if has_new {
            child.named[0].1.save_unlocked(&new, Role::Child).unwrap();
            write_marker(&new, Role::Child, &id).unwrap();
            if record_new {
                mark_staging(&new, "compact-new", &id).unwrap();
            }
        }
        drop(root);
        if has_old {
            std::fs::rename(&leaf, &old).unwrap();
            if record_old {
                mark_staging(&old, "compact-old", &id).unwrap();
            }
        } else {
            std::fs::remove_dir_all(&leaf).unwrap();
        }
        let residue = staging_path(&path.join("named"), "drop-new");
        std::fs::create_dir(&residue).unwrap();
        let root_id = read_marker(&path).unwrap().unwrap().id.unwrap();
        mark_staging(&residue, "drop-new", &root_id).unwrap();
        assert!(Graph::open_read_only(&path).is_err());
        for _ in 0..2 {
            let reopened = Graph::open(&path).unwrap();
            let child = reopened.named_graph(&name("g0")).unwrap();
            assert_eq!(child.named_graph(&name("nested")).unwrap().len(), 1);
        }
        assert!(!old.exists());
        assert!(!new.exists());
        assert!(!residue.exists());
    }
}

#[test]
fn undeclared_slot_with_a_held_lock_is_preserved_until_released() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 1));
    let orphan = path.join("named/9");
    std::fs::create_dir(&orphan).unwrap();
    write_store_marker(&orphan, Role::Child).unwrap();
    let lock = std::fs::File::create(orphan.join("descendant.lock")).unwrap();
    fs4::FileExt::try_lock(&lock).unwrap();
    drop(Graph::open(&path).unwrap());
    assert!(orphan.join("descendant.lock").exists());
    drop(lock);
    drop(Graph::open(&path).unwrap());
    assert!(!orphan.exists());
}

#[cfg(unix)]
#[test]
fn unsupported_directory_sync_is_an_error() {
    assert_eq!(
        fsync_dir(Path::new("/dev/null")).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}
