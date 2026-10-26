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
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("store");
    drop(store(&path, 1));
    std::fs::create_dir(path.join("named/9")).unwrap();
    let failed = path.join("named/9");
    fault::fail_when(move |step, p| step == "remove-tree" && p == failed);
    let opened = Graph::open(&path);
    assert_eq!(fault::disarm(), 1);
    let opened = opened.unwrap();
    assert_eq!(opened.named().len(), 1);
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
