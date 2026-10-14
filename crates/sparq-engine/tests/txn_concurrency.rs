//! (sq-3dyje.12) OCC transaction concurrency stress tests — lost-update /
//! write-write interleaving under real `std::thread` contention, plus reader-under-writer
//! snapshot-isolation assertions.
//!
//! # Why std-thread stress and NOT loom (the honest applicability verdict)
//!
//! `loom` exhaustively permutes the interleavings of *hand-rolled* concurrency — atomics
//! (`AtomicUsize`, `compare_exchange`, `fetch_add`), `UnsafeCell`, and lock-free sequencing.
//! The OCC path here has **none** of that. `TransactionManager` guards ALL mutable shared
//! state — the committed `Arc<Graph>` generation, the version counter, the commit log, and
//! the prune watermarks — behind a single `std::sync::Mutex<State>` (`txn.rs`). Every entry
//! point (`begin_read`, `begin_write`, `commit`, `rollback`, `into_inner`) takes that one
//! lock, and `commit`'s read-check-publish is a single critical section. Snapshot isolation
//! is achieved with `Arc<Graph>` copy-on-write forks — again, no atomics, no `UnsafeCell`.
//!
//! With a coarse `Mutex`, there is no lock-free interleaving for loom to explore that the
//! `Mutex`'s own (already-verified, `std`) mutual exclusion does not already collapse: the
//! interesting question is not "does the lock work" but "does first-committer-wins reject a
//! lost update and never publish a torn overlay". That is a *logical* property of the commit
//! algorithm best exercised by real threads racing the `Mutex` under barrier-forced
//! contention — which is what this file does. Adding a `loom` dev-dep here would be
//! cargo-culting: it would model the `Mutex` (which loom supports) but exercise no synchro-
//! nization this crate actually authored.
//!
//! **Verdict: loom NOT applicable — coarse `Mutex` locking; std-thread stress is the tier.**
//! (If the OCC path is ever rewritten to hand-rolled atomics / a lock-free commit slot, THAT
//! change should file a follow-up loom bead naming the specific atomic interleaving.)
//!
//! # Determinism
//!
//! Every race is forced with a `std::sync::Barrier` (all racers reach the commit point before
//! any of them proceeds) — no `sleep`, no wall-clock timing. Outcomes are asserted as
//! *invariants* (exactly-one-winner on a conflicting race; count-exact on a disjoint race;
//! snapshot-immunity for a reader) rather than as a fixed winner, because which thread wins
//! the `Mutex` is legitimately non-deterministic — but the invariant holds every time. Each
//! race is repeated `ITERS` times to shake orderings.
//!
//! Only compiled under the opt-in `txn` feature.
#![cfg(feature = "txn")]

use sparq_core::Graph;
use sparq_engine::txn::{CommitError, TransactionManager};
use std::sync::{Arc, Barrier};
use std::thread;

/// Base graph: two triples in the default graph (`:a :p :b`, `:b :p :c`).
const SRC: &str = "@prefix : <http://ex/> . :a :p :b . :b :p :c .";
const BASE_COUNT: usize = 2;

/// How many times each race is repeated to shake thread orderings. Kept modest so the suite
/// stays fast; the barrier already forces the contended interleaving on every iteration.
const ITERS: usize = 64;

fn mgr() -> TransactionManager {
    TransactionManager::new(Graph::load_str(SRC, "turtle").unwrap())
}

fn ask(g: &Graph, q: &str) -> bool {
    sparq_engine::ask(g, &format!("PREFIX : <http://ex/> {q}")).unwrap()
}

fn count_all(g: &Graph) -> usize {
    sparq_engine::count(g, "SELECT * WHERE { ?s ?p ?o }").unwrap()
}

/// Two writers begin against the SAME base and both `DELETE :a :p :b` (an overlapping write
/// key) plus each INSERTs a private marker. A `Barrier` releases both only once each has
/// staged its body, so they genuinely race the commit `Mutex`. Invariant, checked on every
/// one of `ITERS` iterations:
///   * EXACTLY ONE commits `Ok`; the other is rejected `Err(Conflict)` — no lost update,
///     no double-commit.
///   * The final store reflects ONLY the winner: winner's marker present, loser's marker
///     ABSENT (loser's whole body rolled back — no torn overlay), the shared triple deleted,
///     `:b :p :c` untouched, and the count is exactly `BASE_COUNT` (−1 delete +1 marker).
#[test]
fn two_writers_racing_overlapping_delete_exactly_one_wins_no_torn_state() {
    for it in 0..ITERS {
        let m = Arc::new(mgr());
        let gate = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for w in 0..2u32 {
            let m = Arc::clone(&m);
            let gate = Arc::clone(&gate);
            handles.push(thread::spawn(move || {
                let marker = format!(":w{w} :p :m{w}");
                let mut tx = m.begin_write();
                // Overlapping delete (forces the conflict) + a private disjoint marker.
                tx.update(&format!(
                    "PREFIX : <http://ex/> DELETE DATA {{ :a :p :b }} ; INSERT DATA {{ {marker} }}"
                ))
                .unwrap();
                // Both bodies are now staged against base v0; race the commit.
                gate.wait();
                (w, tx.commit())
            }));
        }
        let results: Vec<(u32, Result<u64, CommitError>)> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();

        let winners: Vec<u32> = results.iter().filter(|(_, r)| r.is_ok()).map(|(w, _)| *w).collect();
        let losers: Vec<&CommitError> = results.iter().filter_map(|(_, r)| r.as_ref().err()).collect();
        assert_eq!(winners.len(), 1, "iter {it}: exactly one writer must win the overlapping race, got {results:?}");
        assert_eq!(losers.len(), 1, "iter {it}: the other writer must be rejected");
        assert!(
            matches!(losers[0], CommitError::Conflict { .. }),
            "iter {it}: the loser must fail with a write-write Conflict, got {:?}",
            losers[0]
        );

        let winner = winners[0];
        let loser = 1 - winner;
        let r = m.begin_read();
        assert!(!ask(&r, "ASK { :a :p :b }"), "iter {it}: the winner's delete must have landed");
        assert!(ask(&r, "ASK { :b :p :c }"), "iter {it}: the untouched triple must survive");
        assert!(
            ask(&r, &format!("ASK {{ :w{winner} :p :m{winner} }}")),
            "iter {it}: the winner's marker must be committed"
        );
        assert!(
            !ask(&r, &format!("ASK {{ :w{loser} :p :m{loser} }}")),
            "iter {it}: the loser's marker must NOT leak — its body was fully rolled back (no torn overlay)"
        );
        assert_eq!(count_all(&r), BASE_COUNT, "iter {it}: net effect is exactly one writer's (-1 delete +1 marker)");
    }
}

/// Scale-up of the conflict race: `N` writers all begin at base v0 (the `Barrier` is placed
/// AFTER `begin_write` so every writer captures the same base) and all `DELETE :a :p :b`, so
/// every pair overlaps. Under first-committer-wins EXACTLY ONE commits; the other `N-1` are
/// rejected. This is the deterministic strengthening of the existing best-effort
/// `concurrent_writers_are_serialized_*` test (which only asserts `>= 1`).
#[test]
fn many_writers_same_triple_exactly_one_commits() {
    const N: usize = 8;
    for it in 0..ITERS {
        let m = Arc::new(mgr());
        // N racers, all synchronized after begin_write so they share base v0, then again at
        // the commit line.
        let begun = Arc::new(Barrier::new(N));
        let commit_gate = Arc::new(Barrier::new(N));
        let mut handles = Vec::new();
        for _ in 0..N {
            let m = Arc::clone(&m);
            let begun = Arc::clone(&begun);
            let commit_gate = Arc::clone(&commit_gate);
            handles.push(thread::spawn(move || {
                let mut tx = m.begin_write();
                // Everyone has forked from v0 before anyone stages a body.
                begun.wait();
                tx.update("PREFIX : <http://ex/> DELETE DATA { :a :p :b }").unwrap();
                commit_gate.wait();
                tx.commit().is_ok()
            }));
        }
        let oks = handles.into_iter().map(|h| h.join().unwrap()).filter(|&ok| ok).count();
        assert_eq!(oks, 1, "iter {it}: exactly one of {N} writers racing the same triple may commit");

        let r = m.begin_read();
        assert!(!ask(&r, "ASK { :a :p :b }"), "iter {it}: the triple is deleted exactly once");
        assert_eq!(count_all(&r), BASE_COUNT - 1, "iter {it}: net effect is a single delete, no torn state");
        assert_eq!(m.committed_version(), 1, "iter {it}: only one commit advanced the version");
    }
}

/// `N` writers each INSERT a DISJOINT private triple and race the commit. Because the write
/// sets never overlap, first-committer-wins lets EVERY stale writer through by REPLAYING its
/// resolved effect log onto the current committed generation (the lost-update guard for
/// disjoint writers). Invariant: all `N` markers survive and the count is exactly
/// `BASE_COUNT + N` — no writer's insert was lost-updated by a later committer publishing a
/// stale fork wholesale.
#[test]
fn disjoint_writers_racing_all_commit_no_lost_update() {
    const N: usize = 8;
    for it in 0..ITERS {
        let m = Arc::new(mgr());
        let gate = Arc::new(Barrier::new(N));
        let mut handles = Vec::new();
        for w in 0..N {
            let m = Arc::clone(&m);
            let gate = Arc::clone(&gate);
            handles.push(thread::spawn(move || {
                let mut tx = m.begin_write();
                tx.update(&format!("PREFIX : <http://ex/> INSERT DATA {{ :d{w} :p :v{w} }}")).unwrap();
                gate.wait();
                tx.commit().is_ok()
            }));
        }
        let oks = handles.into_iter().map(|h| h.join().unwrap()).filter(|&ok| ok).count();
        assert_eq!(oks, N, "iter {it}: disjoint writers must ALL commit (stale-but-disjoint replays its delta)");

        let r = m.begin_read();
        assert_eq!(
            count_all(&r),
            BASE_COUNT + N,
            "iter {it}: every disjoint insert must survive — no lost update from a wholesale stale-fork publish"
        );
        for w in 0..N {
            assert!(ask(&r, &format!("ASK {{ :d{w} :p :v{w} }}")), "iter {it}: writer {w}'s insert was lost");
        }
    }
}

/// Snapshot isolation under concurrent committers: a `ReadTxn` opened at v0 polls its count in
/// a tight loop *while* `N` writers commit (a `Barrier` overlaps the reader's polling with the
/// writers' publishes). The reader's view must NEVER change — it is pinned to its begin-time
/// generation, immune to every intervening commit, and never observes a torn intermediate
/// count. A fresh read afterwards sees all the writes.
#[test]
fn reader_is_isolated_from_concurrent_committers() {
    const N: usize = 6;
    const POLLS: usize = 4_000;
    for it in 0..ITERS {
        let m = Arc::new(mgr());
        let reader = m.begin_read(); // pinned at v0
        assert_eq!(count_all(&reader), BASE_COUNT);

        // N writer threads + this reader thread all release together.
        let gate = Arc::new(Barrier::new(N + 1));
        let mut handles = Vec::new();
        for w in 0..N {
            let m = Arc::clone(&m);
            let gate = Arc::clone(&gate);
            handles.push(thread::spawn(move || {
                let mut tx = m.begin_write();
                tx.update(&format!("PREFIX : <http://ex/> INSERT DATA {{ :r{w} :p :v{w} }}")).unwrap();
                gate.wait();
                tx.commit().is_ok()
            }));
        }

        // Poll the pinned snapshot concurrently with the writers' commits.
        gate.wait();
        for _ in 0..POLLS {
            assert_eq!(
                count_all(&reader),
                BASE_COUNT,
                "iter {it}: the read txn's snapshot must be immune to concurrent commits"
            );
        }

        let oks = handles.into_iter().map(|h| h.join().unwrap()).filter(|&ok| ok).count();
        assert_eq!(oks, N, "iter {it}: all disjoint writers commit");
        // The pinned reader is STILL isolated after every commit has landed.
        assert_eq!(count_all(&reader), BASE_COUNT, "iter {it}: snapshot stays pinned after the writers finish");
        // A fresh read observes the full committed state.
        assert_eq!(count_all(&m.begin_read()), BASE_COUNT + N, "iter {it}: a new read sees every commit");
    }
}
