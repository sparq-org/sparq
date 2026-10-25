//! Failure and crash tests of the directory swaps, the staging records and the store lock,
//! driven by the test-only fault hook ([`crate::fault`]): each makes one chosen storage step
//! fail (or the process "crash" there) and checks what the graph and the directory are left
//! as, and that the next open gets everything back.

use super::*;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq-swap-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{s}")))
}

fn triple(s: &str) -> [Term; 3] {
    [iri(s), iri("p"), iri("o")]
}

fn has(g: &Graph, s: &str) -> bool {
    let ids: Option<Vec<_>> = triple(s).iter().map(|t| g.id_of(t)).collect();
    ids.is_some_and(|ids| g.iter_ids().any(|t| t[..] == ids[..]))
}

fn sub<'a>(g: &'a Graph, name: &str) -> &'a Graph {
    g.named_graph(&iri(name)).unwrap_or_else(|| panic!("no named graph {name}"))
}

/// A dataset (default `a`, `g0` holding `b`, `g1` holding `c`) saved at `dir`, opened, with
/// `b2` written to `g0` through its log (not yet in any base).
fn open_with_logged_update(dir: &Path) -> Graph {
    let nq = "<http://ex/a> <http://ex/p> <http://ex/o> .\n\
              <http://ex/b> <http://ex/p> <http://ex/o> <http://ex/g0> .\n\
              <http://ex/c> <http://ex/p> <http://ex/o> <http://ex/g1> .\n";
    Graph::load_dataset(nq, "nquads").unwrap().save(dir).unwrap();
    let mut g = Graph::open(dir).unwrap();
    g.apply_delta_nquads("<http://ex/b2> <http://ex/p> <http://ex/o> <http://ex/g0> .", "")
        .unwrap();
    g.apply_delta(&[triple("a2")], &[]).unwrap();
    g
}

/// The whole dataset is there, writable, after a reopen.
fn assert_intact(dir: &Path) {
    let mut g = Graph::open(dir).unwrap_or_else(|e| panic!("reopen {}: {e}", dir.display()));
    assert!(has(&g, "a") && has(&g, "a2"), "default graph lost");
    assert!(has(sub(&g, "g0"), "b") && has(sub(&g, "g0"), "b2"), "g0 lost");
    assert!(has(sub(&g, "g1"), "c"), "g1 lost");
    g.apply_delta(&[triple("after")], &[]).expect("writable after reopen");
}

fn would_block(dir: &Path) -> bool {
    matches!(Graph::open(dir), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
}

fn is(path: &Path) -> impl Fn(&Path) -> bool + 'static {
    let path = dir_lock::resolve_dir(path).unwrap();
    move |p: &Path| dir_lock::resolve_dir(p).is_ok_and(|resolved| resolved == path)
}

/// H1: a named graph's swap fails after its first rename (marking the old half keeps
/// failing, so even the recovery that follows cannot finish): the named graph is frozen, the
/// root is frozen with it, and a named-graph drop is refused, so nothing re-saves the frozen
/// graph's empty view. The reopen completes the swap.
#[test]
fn a_frozen_named_graph_freezes_the_root_and_a_drop_is_refused() {
    let root = scratch("named-freeze");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let old = is(&staging_path(&dir.join("named").join("0"), "compact-old"));
    fault::fail_every(move |step, p| step == "mark" && old(p));
    assert!(g.compact().is_err());
    assert!(fault::disarm() > 0, "the step never failed");
    assert!(sub(&g, "g0").is_frozen(), "the named graph was not frozen");
    assert!(g.is_frozen(), "the root went on writing");
    assert!(g.drop_named_durable(&iri("g1")).is_err(), "a drop was accepted");
    assert!(g.drop_all_named_durable().is_err());
    drop(g);
    assert_intact(&dir);
    std::fs::remove_dir_all(&root).ok();
}

/// H1 (and M1): a named graph's swap fails at its second rename and rolls back: the new half
/// keeps its record, so the named graph re-opens writable at once (no freeze), and a drop of
/// another named graph keeps its logged update.
#[test]
fn a_rolled_back_named_swap_keeps_the_graph_writable() {
    let root = scratch("named-rollback");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let new = is(&staging_path(&dir.join("named").join("0"), "compact-new"));
    fault::fail_when(move |step, p| step == "rename" && new(p));
    assert!(g.compact().is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(!g.is_frozen() && !sub(&g, "g0").is_frozen(), "frozen after a rollback");
    assert!(has(sub(&g, "g0"), "b2"));
    assert!(g.drop_named_durable(&iri("g1")).unwrap());
    drop(g);
    let g = Graph::open(&dir).unwrap();
    assert!(has(sub(&g, "g0"), "b") && has(sub(&g, "g0"), "b2"), "g0 lost its data");
    assert!(g.named_graph(&iri("g1")).is_none());
    drop(g);
    std::fs::remove_dir_all(&root).ok();
}

/// H1: any named graph refusing writes refuses a drop or clear of the named graphs.
#[test]
fn a_drop_is_refused_while_a_named_graph_refuses_writes() {
    let root = scratch("named-detached");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    g.named[0].1.detach("test");
    assert!(g.drop_named_durable(&iri("g1")).is_err());
    assert!(g.drop_all_named_durable().is_err());
    assert!(g.clear_named_durable(&iri("g1")).is_err());
    drop(g);
    std::fs::remove_dir_all(&root).ok();
}

/// M1: a root swap whose second rename fails rolls back with the new half's record restored:
/// the graph goes on writing to the old store, no staging is left, and it reopens.
#[test]
fn a_failed_second_rename_rolls_back_and_keeps_writing() {
    let root = scratch("second-rename");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let new = is(&staging_path(&dir, "compact-new"));
    fault::fail_when(move |step, p| step == "rename" && new(p));
    assert!(g.compact().is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(!g.is_frozen(), "frozen after a rollback");
    g.apply_delta(&[triple("x")], &[]).unwrap();
    assert!(!staging_path(&dir, "compact-new").exists() && !staging_path(&dir, "compact-old").exists());
    drop(g);
    assert_intact(&dir);
    assert!(has(&Graph::open(&dir).unwrap(), "x"));
    std::fs::remove_dir_all(&root).ok();
}

/// M1: the second rename fails and the process dies at each step of the rollback after it:
/// a half always carries a record (or the store a leftover record), so every reopen recovers.
#[test]
fn a_crash_at_any_step_of_a_rollback_is_recovered() {
    for after in 0..8 {
        let root = scratch(&format!("rollback-crash-{after}"));
        let dir = root.join("g");
        let mut g = open_with_logged_update(&dir);
        let new = is(&staging_path(&dir, "compact-new"));
        fault::fail_then_crash(after, move |step, p| step == "rename" && new(p));
        assert!(g.compact().is_err());
        fault::disarm();
        drop(g);
        assert_intact(&dir);
        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.ends_with(".lock"))
            .collect();
        assert_eq!(names, ["g"], "after {after}: left {names:?}");
        assert!(!dir.join(STAGING_MARKER).exists(), "after {after}: record left in the store");
        std::fs::remove_dir_all(&root).ok();
    }
}

/// M1: recovering a lone old half (the store missing) renames it back before removing its
/// record, so a crash at either step leaves something the next open recovers; the same for the
/// `named/` sub-tree of a drop.
#[test]
fn a_crash_while_recovery_rolls_back_is_recovered() {
    for (step, kind) in [("rename", "compact"), ("clear", "compact"), ("rename", "drop"), ("clear", "drop")] {
        let root = scratch(&format!("recovery-crash-{step}-{kind}"));
        let dir = root.join("g");
        drop(open_with_logged_update(&dir));
        let id = read_marker(&dir).unwrap().unwrap().id.unwrap();
        let (live, old, record) = if kind == "compact" {
            (dir.clone(), staging_path(&dir, "compact-old"), "compact-old")
        } else {
            let named = dir.join("named");
            (named.clone(), staging_path(&named, "drop-old"), "drop-old")
        };
        std::fs::rename(&live, &old).unwrap();
        mark_staging(&old, record, &id).unwrap();
        let target = if step == "rename" { old.clone() } else { live.clone() };
        let step_name = step;
        fault::crash_at(move |s, p| s == step_name && p == target);
        assert!(Graph::open(&dir).is_err(), "{step} {kind}: the crash did not happen");
        assert!(fault::disarm() > 0);
        assert_intact(&dir);
        assert!(!live.join(STAGING_MARKER).exists(), "{step} {kind}: record left behind");
        std::fs::remove_dir_all(&root).ok();
    }
}

/// M1: removing a staging directory keeps its record to the end, so a crash part-way through
/// leaves a directory that still says whose it is, removed by the next open.
#[test]
fn a_crash_while_staging_is_removed_keeps_its_record() {
    for nth in 0..3 {
        let root = scratch(&format!("removal-crash-{nth}"));
        let dir = root.join("g");
        let mut g = open_with_logged_update(&dir);
        let old = staging_path(&dir, "compact-old");
        let in_old = old.clone();
        let mut seen = 0;
        fault::crash_at(move |step, p| {
            let hit = step == "remove-entry" && p.parent() == Some(in_old.as_path());
            if hit {
                seen += 1;
            }
            hit && seen > nth
        });
        // The crash may come in the re-open's own recovery (which removes the old half too)
        // or in the removal after it: either way the compaction stops there.
        let _ = g.compact();
        assert!(fault::disarm() > 0, "the removal never reached entry {nth}");
        assert!(old.join(STAGING_MARKER).exists(), "a part-removed half lost its record");
        drop(g);
        assert_intact(&dir);
        assert!(!old.exists(), "the leftover half was not removed");
        std::fs::remove_dir_all(&root).ok();
    }
}

/// M1: a crash while a staging directory is being made leaves only its scratch (with at most a
/// record), never an unrecorded staging directory: the store reopens and compacts again.
#[test]
fn a_crash_before_the_staging_record_exists_is_harmless() {
    let root = scratch("create-crash");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    fault::crash_at(|step, _| step == "mark");
    assert!(g.compact().is_err());
    assert!(fault::disarm() > 0);
    drop(g);
    assert!(!staging_path(&dir, "compact-new").exists(), "an unrecorded staging directory");
    assert_intact(&dir);
    let mut g = Graph::open(&dir).unwrap();
    g.apply_delta(&[triple("y")], &[]).unwrap();
    g.compact().expect("a compaction after the crash");
    assert!(!staging_scratch(&staging_path(&dir, "compact-new")).exists());
    drop(g);
    std::fs::remove_dir_all(&root).ok();
}

/// M2: marking the old half fails right after the first rename: the swap is recovered at once
/// under the lock (completing it), so the compaction succeeds and the graph stays writable.
#[test]
fn a_swap_failing_after_its_first_rename_is_completed_at_once() {
    let root = scratch("complete-at-once");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let old = is(&staging_path(&dir, "compact-old"));
    fault::fail_when(move |step, p| step == "mark" && old(p));
    g.compact().expect("the recovered swap counts as done");
    assert_eq!(fault::disarm(), 1);
    assert!(!g.is_frozen());
    assert!(has(&g, "a2") && has(sub(&g, "g0"), "b2"));
    g.apply_delta(&[triple("z")], &[]).unwrap();
    assert!(would_block(&dir), "the lock was lost");
    assert!(!staging_path(&dir, "compact-old").exists() && !staging_path(&dir, "compact-new").exists());
    drop(g);
    assert_intact(&dir);
    assert!(has(&Graph::open(&dir).unwrap(), "z"));
    std::fs::remove_dir_all(&root).ok();
}

/// M2, L6: a swap that fails and cannot be recovered at once freezes the graph: it refuses every
/// write (named graphs included), holds the lock, reports itself frozen, and reads what a reopen
/// would (its logs replayed in memory), until it is dropped and the directory re-opened.
#[test]
fn a_swap_failing_after_its_renames_detaches_the_graph() {
    let root = scratch("freeze");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let new = staging_path(&dir, "compact-new");
    let live = dir.clone();
    // The second rename fails, and so does every removal of the record the rollback leaves in
    // the store: the rollback cannot finish, nor can the recovery after it.
    fault::fail_every(move |step, p| (step == "rename" && p == new) || (step == "clear" && p == live));
    assert!(g.compact().is_err());
    assert!(fault::disarm() > 1);
    assert!(g.is_frozen());
    let t = triple("x");
    assert!(g.apply_delta(std::slice::from_ref(&t), &[]).is_err(), "a write was accepted");
    assert!(has(&g, "a2") && has(sub(&g, "g0"), "b2"), "the frozen view lost the logged updates");
    assert!(!g.named.is_empty(), "the frozen graph has no named graphs to check");
    for (_, sub) in &mut g.named {
        assert!(sub.is_frozen());
        assert!(sub.apply_delta(std::slice::from_ref(&t), &[]).is_err(), "named too");
    }
    assert!(would_block(&dir), "the frozen graph lost the directory's lock");
    assert!(g.apply_delta_nquads("<http://ex/x> <http://ex/p> <http://ex/o> <http://ex/g9> .", "").is_err());
    assert!(g.ensure_named(&iri("g9")).is_err());
    assert!(g.commit_txn(&[(true, None, t.clone())]).is_err());
    assert!(g.compact().is_err() && g.vacuum().is_err());
    assert!(g.restore_in_place(Graph::new()).is_err());
    drop(g);
    assert_intact(&dir);
    Graph::open(&dir).unwrap().compact().unwrap();
    let read_only = Graph::open_read_only(&dir).unwrap();
    assert!(!read_only.is_frozen() && read_only.check_writable().is_err());
    drop(read_only);
    std::fs::remove_dir_all(&root).ok();
}

/// L5: a panic part-way through a swap leaves the graph holding the lock and refusing writes,
/// never an unlocked writable graph.
#[test]
fn a_panic_during_a_swap_leaves_the_graph_locked_and_read_only() {
    let root = scratch("panic");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let live = is(&dir);
    fault::panic_at(move |step, p| step == "rename" && live(p));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.compact()));
    fault::disarm();
    assert!(r.is_err(), "no panic");
    assert!(g.check_writable().is_err(), "writable after a panic");
    assert!(would_block(&dir), "unlocked after a panic");
    drop(g);
    assert_intact(&dir);
    std::fs::remove_dir_all(&root).ok();
}

/// L6: a restore in place whose first rename fails leaves the graph writing to the old store.
#[test]
fn a_restore_failing_at_its_first_rename_keeps_logging() {
    let root = scratch("restore-publish");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let live = is(&dir);
    fault::fail_when(move |step, p| step == "rename" && live(p));
    let fresh = Graph::load_str("<http://ex/new> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    assert!(g.restore_in_place(fresh).is_err());
    assert_eq!(fault::disarm(), 1);
    g.apply_delta(&[triple("kept")], &[]).unwrap();
    drop(g);
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "kept") && has(&g, "a2") && !has(&g, "new"));
    drop(g);
    std::fs::remove_dir_all(&root).ok();
}

/// M5: a named-graph drop that fails once the in-memory named graphs are released re-opens them
/// from the directory, so the graph does not go on with none (and lose them all at its next
/// compaction).
#[test]
fn a_failed_drop_reloads_the_named_graphs() {
    let root = scratch("drop-reload");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let named = is(&dir.join("named"));
    fault::fail_when(move |step, p| step == "rename" && named(p));
    assert!(g.drop_named_durable(&iri("g1")).is_err());
    assert_eq!(fault::disarm(), 1);
    assert!(!g.is_frozen());
    assert!(g.named_graph(&iri("g0")).is_some() && g.named_graph(&iri("g1")).is_some());
    g.apply_delta(&[triple("x")], &[]).unwrap();
    g.compact().unwrap();
    drop(g);
    assert_intact(&dir);
    std::fs::remove_dir_all(&root).ok();
}

/// L1: two handles on one log append after each other, never over each other.
#[test]
fn two_handles_on_a_log_both_append() {
    let root = scratch("two-handles");
    let mut first = Wal::open(&root).unwrap();
    let mut second = Wal::open(&root).unwrap();
    first.append_batch(&[triple("one")], &[]).unwrap();
    second.append_batch(&[triple("two")], &[]).unwrap();
    first.append_batch(&[triple("three")], &[]).unwrap();
    let ops: Vec<_> = Wal::replay(&root).unwrap().into_iter().map(|(_, t)| t[0].clone()).collect();
    assert_eq!(ops, [iri("one"), iri("two"), iri("three")]);
    std::fs::remove_dir_all(&root).ok();
}

/// L4: a staging path's lock is named for the path itself, never for where a symbolic link
/// planted there leads.
#[cfg(unix)]
#[test]
fn a_staging_lock_never_follows_a_link() {
    let root = scratch("staging-link");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(elsewhere.join("target")).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("target"), staging_path(&dir, "compact-new")).unwrap();
    assert!(g.compact().is_err(), "swapped through a link");
    assert!(!elsewhere.join("target.lock").exists(), "a lock was made where the link leads");
    assert!(root.join("g.compact-new.lock").exists());
    drop(g);
    std::fs::remove_dir_all(&root).ok();
}

/// The two-lock design: a holder that cannot take the outside lock (it takes the inside lock
/// alone) never gets the store while a swap runs, at any step: the swapper holds the old
/// directory's inside lock until the old directory is gone, and the new one's before it is
/// put in place.
#[cfg(not(windows))]
#[test]
fn an_inside_only_opener_never_gets_a_store_mid_swap() {
    let root = scratch("inside-race");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let at = dir.clone();
    let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = got.clone();
    fault::observe(move |step, path| {
        if let Ok(Some(_held)) = dir_lock::lock_inside(&at, false) {
            seen.borrow_mut().push(format!("{step} {}", path.display()));
        }
    });
    g.compact().unwrap();
    fault::disarm();
    assert!(got.borrow().is_empty(), "an inside-only opener got the store at {:?}", got.borrow());
    assert!(dir_lock::lock_inside(&dir, false).is_err(), "the swapped graph lost its inside lock");
    drop(g);
    assert_intact(&dir);
    std::fs::remove_dir_all(&root).ok();
}

/// Fault targets compare resolved paths, including missing staging paths under an alias.
#[test]
fn fault_targets_match_resolved_paths_before_and_after_creation() {
    let root = scratch("fault-path");
    let alias = root.join(".").join("store.compact-old");
    let target = is(&alias);
    let actual = root.join("store.compact-old");
    assert!(target(&actual));
    std::fs::create_dir(&actual).unwrap();
    assert!(target(&actual.canonicalize().unwrap()));
    #[cfg(unix)]
    {
        let link = root.join("alias");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(target(&link.join("store.compact-old")));
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// Cleanup remains recoverable after the last record has gone and before its directory goes.
#[test]
fn a_crash_after_removing_the_last_staging_record_is_recovered() {
    for kind in ["compact-old", "drop-old"] {
        for step_name in ["rename", "remove-record", "remove-dir"] {
            let root = scratch(&format!("last-record-{kind}-{step_name}"));
            let dir = root.join("g");
            let mut g = open_with_logged_update(&dir);
            let target = if kind == "compact-old" {
                staging_path(&dir, kind)
            } else {
                staging_path(&dir.join("named"), kind)
            };
            let at = is(&target);
            fault::crash_at(move |step, p| step == step_name && at(p));
            if kind == "compact-old" {
                let _ = g.compact();
            } else {
                let _ = g.drop_named_durable(&iri("g1"));
            }
            assert!(fault::disarm() > 0, "cleanup did not reach {kind} {step_name}");
            drop(g);
            let reopened = Graph::open(&dir).unwrap();
            assert!(has(&reopened, "a2"));
            assert!(has(sub(&reopened, "g0"), "b2"));
            assert!(!target.exists());
            drop(reopened);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

/// A renamed survivor tree recovers its matching names if the manifest publication stops.
#[test]
fn a_named_drop_recovers_the_manifest_matching_its_survivors() {
    let root = scratch("drop-manifest");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let manifest_tmp = is(&dir.join("named.bin.tmp"));
    fault::crash_at(move |step, p| step == "rename" && manifest_tmp(p));
    assert!(g.drop_named_durable(&iri("g0")).is_err());
    assert!(fault::disarm() > 0);
    drop(g);
    let g = Graph::open(&dir).expect("recover matching graph names and directories");
    assert!(g.named_graph(&iri("g0")).is_none());
    assert!(has(sub(&g, "g1"), "c"));
    assert_eq!(g.named().len(), 1);
    drop(g);
    std::fs::remove_dir_all(root).unwrap();
}

/// Frozen reads include committed frames from the root and each named graph, without
/// truncating or appending to the files needed by the next writable open.
#[test]
fn a_frozen_view_replays_root_and_named_transaction_logs_without_writing() {
    let root = scratch("frozen-transactions");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    g.commit_txn(&[
        (true, None, triple("root-txn")),
        (true, Some(iri("g9")), triple("new-named-txn")),
    ]).unwrap();
    g.named[0].1.commit_txn(&[(true, None, triple("child-txn"))]).unwrap();
    let paths = [dir.join("txn.log"), dir.join("named/0/txn.log"), dir.join("wal.log")];
    let before: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let new = is(&staging_path(&dir, "compact-new"));
    let live = is(&dir);
    fault::fail_every(move |step, p| (step == "rename" && new(p)) || (step == "clear" && live(p)));
    assert!(g.publish_staged_base_or_freeze(|graph, target, role| graph.save_unlocked(target, role)).is_err());
    assert!(fault::disarm() > 1);
    assert!(g.is_frozen());
    assert!(has(&g, "root-txn"));
    assert!(has(sub(&g, "g0"), "child-txn"));
    assert!(has(sub(&g, "g9"), "new-named-txn"));
    assert!(g.named().iter().all(|(_, sub)| sub.is_frozen() && sub.check_writable().is_err()));
    let after: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert_eq!(before, after, "frozen replay changed a log");
    drop(g);
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "root-txn") && has(sub(&g, "g0"), "child-txn"));
    assert!(has(sub(&g, "g9"), "new-named-txn"));
    drop(g);
    std::fs::remove_dir_all(root).unwrap();
}

/// A named swap that unwinds still makes its root report frozen and refuse further writes.
#[test]
fn a_panicking_named_swap_freezes_the_root_and_keeps_its_data() {
    let root = scratch("named-panic");
    let dir = root.join("g");
    let mut g = open_with_logged_update(&dir);
    let live = is(&dir.join("named/0"));
    fault::panic_at(move |step, p| step == "rename" && live(p));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.compact()));
    assert!(fault::disarm() > 0);
    assert!(result.is_err());
    assert!(g.is_frozen() && g.check_writable().is_err());
    assert!(g.apply_delta(&[triple("refused")], &[]).is_err());
    assert!(g.apply_delta_to_named(1, &[triple("refused")], &[]).is_err());
    assert!(g.drop_named_durable(&iri("g1")).is_err());
    assert!(g.drop_all_named_durable().is_err());
    assert!(would_block(&dir));
    drop(g);
    assert_intact(&dir);
    std::fs::remove_dir_all(root).unwrap();
}
