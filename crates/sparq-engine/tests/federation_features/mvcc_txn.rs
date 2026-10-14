//! (sq-it1x) MVCC / ACID transaction-isolation tests.
//!
//! Snapshot isolation over the COW delta-overlay substrate (`Graph::fork`/`snapshot`/
//! `apply_delta`) with first-committer-wins write-write conflict detection — the
//! ACID contract these tests pin:
//!   * Isolation  — a read txn sees a stable point-in-time, immune to later commits.
//!   * Atomicity  — a write txn's body publishes all-or-nothing; rollback shows nothing.
//!   * Consistency — read-your-own-writes within a write txn.
//!   * Concurrency — first-committer-wins: a stale, conflicting writer is rejected; a
//!     stale, NON-conflicting (disjoint write set) writer still commits.
//!
//! Only compiled under the opt-in `txn` feature.
#![cfg(feature = "txn")]

use sparq_core::Graph;
use sparq_engine::txn::{CommitError, TransactionManager};

const SRC: &str = "@prefix : <http://ex/> . :a :p :b . :b :p :c .";

fn mgr() -> TransactionManager {
    TransactionManager::new(Graph::load_str(SRC, "turtle").unwrap())
}

fn ask(g: &Graph, q: &str) -> bool {
    sparq_engine::ask(g, &format!("PREFIX : <http://ex/> {q}")).unwrap()
}

fn count_all(g: &Graph) -> usize {
    sparq_engine::count(g, "SELECT * WHERE { ?s ?p ?o }").unwrap()
}

#[test]
fn read_txn_is_isolated_from_later_commits() {
    let m = mgr();
    let snap = m.begin_read();
    assert_eq!(count_all(&snap), 2);

    // A writer commits AFTER the read txn began.
    let mut w = m.begin_write();
    w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    w.commit().unwrap();

    // The read txn STILL sees its point-in-time state (snapshot isolation).
    assert_eq!(count_all(&snap), 2, "read txn must not observe the later commit");
    // A fresh read sees the new state.
    assert_eq!(count_all(&m.begin_read()), 3);
}

#[test]
fn write_txn_reads_its_own_writes_but_others_dont_until_commit() {
    let m = mgr();
    let mut w = m.begin_write();
    w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();

    // Read-your-own-writes inside the txn.
    assert!(ask(w.graph(), "ASK { :x :p :y }"));
    // But the uncommitted write is invisible to a concurrent read txn.
    assert!(!ask(&m.begin_read(), "ASK { :x :p :y }"), "uncommitted write must be invisible");

    let v = w.commit().unwrap();
    assert!(v >= 1);
    // After commit, a new read txn sees it.
    assert!(ask(&m.begin_read(), "ASK { :x :p :y }"));
}

#[test]
fn rollback_discards_the_whole_body() {
    let m = mgr();
    let mut w = m.begin_write();
    w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    w.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b }").unwrap();
    w.rollback(); // drop without commit

    let r = m.begin_read();
    assert!(!ask(&r, "ASK { :x :p :y }"), "rolled-back insert must not persist");
    assert!(ask(&r, "ASK { :a :p :b }"), "rolled-back delete must not persist");
    assert_eq!(count_all(&r), 2);
}

#[test]
fn first_committer_wins_on_conflicting_write_set() {
    let m = mgr();
    // Two writers both begin against the SAME base version.
    let mut w1 = m.begin_write();
    let mut w2 = m.begin_write();

    // They write OVERLAPPING triples (both touch :a :p :b).
    w1.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b }").unwrap();
    w2.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b } ; INSERT DATA { :a :p :z }").unwrap();

    // First committer wins.
    w1.commit().unwrap();
    // Second committer is stale AND conflicting -> rejected.
    let err = w2.commit().unwrap_err();
    assert!(matches!(err, CommitError::Conflict { .. }), "expected a write-write conflict, got {err:?}");

    // The store reflects ONLY w1's effect.
    let r = m.begin_read();
    assert!(!ask(&r, "ASK { :a :p :b }"));
    assert!(!ask(&r, "ASK { :a :p :z }"), "w2 must have been fully rolled back");
}

#[test]
fn stale_but_disjoint_writer_still_commits() {
    let m = mgr();
    let mut w1 = m.begin_write();
    let mut w2 = m.begin_write();

    // DISJOINT write sets (different triples).
    w1.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    w2.update("PREFIX : <http://ex/> INSERT DATA { :u :p :v }").unwrap();

    w1.commit().unwrap();
    // w2 is stale but does not conflict -> first-committer-wins lets it through.
    w2.commit().expect("a disjoint stale writer must still commit under SI");

    let r = m.begin_read();
    assert!(ask(&r, "ASK { :x :p :y }"));
    assert!(ask(&r, "ASK { :u :p :v }"));
    assert_eq!(count_all(&r), 4);
}

#[test]
fn committed_version_advances_monotonically() {
    let m = mgr();
    let v0 = m.committed_version();
    let mut w = m.begin_write();
    w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    let v1 = w.commit().unwrap();
    assert!(v1 > v0, "commit must advance the version");
    assert_eq!(m.committed_version(), v1);
}

#[test]
fn empty_write_txn_commit_is_a_noop_and_never_conflicts() {
    let m = mgr();
    let v0 = m.committed_version();
    // Two empty writers: neither has a write set, so neither can conflict.
    let w1 = m.begin_write();
    let mut w2 = m.begin_write();
    w2.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    w2.commit().unwrap();
    // The empty writer commits fine even though it is now stale.
    w1.commit().expect("an empty txn never conflicts");
    assert!(m.committed_version() > v0);
    assert_eq!(count_all(&m.begin_read()), 3);
}

#[test]
fn delete_insert_where_write_set_participates_in_conflict_detection() {
    let m = mgr();
    let mut w1 = m.begin_write();
    let mut w2 = m.begin_write();

    // w1 rewrites :a :p :b -> :a :p :rewritten via DELETE/INSERT WHERE.
    w1.update(
        "PREFIX : <http://ex/> DELETE { :a :p ?o } INSERT { :a :p :rewritten } WHERE { :a :p ?o }",
    )
    .unwrap();
    // w2 also deletes the same triple.
    w2.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b }").unwrap();

    w1.commit().unwrap();
    let err = w2.commit().unwrap_err();
    assert!(matches!(err, CommitError::Conflict { .. }));

    let r = m.begin_read();
    assert!(ask(&r, "ASK { :a :p :rewritten }"));
}

#[test]
fn into_inner_returns_the_committed_graph() {
    let m = mgr();
    let mut w = m.begin_write();
    w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    w.commit().unwrap();
    let g = m.into_inner();
    assert!(ask(&g, "ASK { :x :p :y }"));
}

#[test]
fn named_graph_writes_conflict_only_within_the_same_graph() {
    let m = mgr();
    let mut w1 = m.begin_write();
    let mut w2 = m.begin_write();

    // Same triple shape, but in DIFFERENT named graphs -> disjoint slots, no conflict.
    w1.update("PREFIX : <http://ex/> INSERT DATA { GRAPH :g1 { :s :p :o } }").unwrap();
    w2.update("PREFIX : <http://ex/> INSERT DATA { GRAPH :g2 { :s :p :o } }").unwrap();
    w1.commit().unwrap();
    w2.commit().expect("writes to different named graphs do not conflict");

    // But same triple in the SAME named graph DOES conflict.
    let mut w3 = m.begin_write();
    let mut w4 = m.begin_write();
    w3.update("PREFIX : <http://ex/> DELETE DATA { GRAPH :g1 { :s :p :o } }").unwrap();
    w4.update("PREFIX : <http://ex/> DELETE DATA { GRAPH :g1 { :s :p :o } }").unwrap();
    w3.commit().unwrap();
    assert!(matches!(w4.commit().unwrap_err(), CommitError::Conflict { .. }));
}

#[test]
fn stale_disjoint_commit_preserves_both_writers_data() {
    // The lost-update guard: a stale-but-disjoint writer must MERGE its delta onto the
    // intervening commit, not publish its own out-of-date fork wholesale.
    let m = mgr();
    let mut w1 = m.begin_write();
    let mut w2 = m.begin_write();
    w1.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b } ; INSERT DATA { :w1 :p :one }").unwrap();
    w2.update("PREFIX : <http://ex/> INSERT DATA { :w2 :p :two }").unwrap();
    w1.commit().unwrap();
    w2.commit().unwrap();

    let r = m.begin_read();
    // w1's delete + insert survived w2's later commit; w2's insert is also present.
    assert!(!ask(&r, "ASK { :a :p :b }"), "w1's delete must survive w2's stale commit");
    assert!(ask(&r, "ASK { :w1 :p :one }"));
    assert!(ask(&r, "ASK { :w2 :p :two }"));
}

#[test]
fn concurrent_writers_are_serialized_one_wins_on_conflict() {
    use std::sync::Arc;
    use std::thread;

    let m = Arc::new(mgr());
    let mut handles = Vec::new();
    // Many threads racing to delete the SAME triple: under first-committer-wins exactly one
    // succeeds; the rest get a Conflict (or commit a no-op if they began after the winner).
    for _ in 0..8 {
        let m = Arc::clone(&m);
        handles.push(thread::spawn(move || {
            let mut w = m.begin_write();
            w.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b }").unwrap();
            w.commit().is_ok()
        }));
    }
    let oks = handles.into_iter().map(|h| h.join().unwrap()).filter(|&ok| ok).count();
    assert!(oks >= 1, "at least one writer must commit");
    // The store is consistent: the triple is gone exactly once, no panic / no torn state.
    assert!(!ask(&m.begin_read(), "ASK { :a :p :b }"));
}

#[test]
fn parse_error_in_update_is_reported_and_txn_stays_usable() {
    let m = mgr();
    let mut w = m.begin_write();
    assert!(w.update("NOT VALID SPARQL").is_err());
    // The txn is still usable after a rejected statement (its working copy is unchanged).
    w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
    w.commit().unwrap();
    assert!(ask(&m.begin_read(), "ASK { :x :p :y }"));
}
