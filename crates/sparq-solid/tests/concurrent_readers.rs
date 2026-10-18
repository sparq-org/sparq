//! sq-cnuqd (issue #1569) — the read side is `&self` and shares a bounded,
//! sharded session cache, so N threads sharing ONE `&PodStore` can query concurrently.
//!
//! These tests exercise the REAL path (`Arc<PodStore>` shared across `std::thread`s calling
//! `query_as`/`accessible`), not a mock. They pin the two load-bearing invariants issue
//! #1569 needs:
//!
//! 1. **concurrent readers get correct, session-scoped results** — the same `&PodStore`,
//!    two sessions, run in parallel from many threads, every answer fail-closed-correct;
//! 2. **invalidation under concurrent read is snapshot-consistent** — while readers hammer
//!    `accessible`, a writer revokes a pod via `put_acl`; every observed set is EITHER the
//!    pre-revoke set OR the post-revoke set, NEVER a torn half-and-half one, and after the
//!    write settles every reader converges on the revoked view (generation pinning, #1584).

use sparq_solid::{Mode, PodStore, Session};
use std::sync::{Arc, Barrier};
use std::thread;

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

fn sess(agent: &str) -> Session<'_> {
    Session { agent: Some(agent), client: None, issuer: None, now: None }
}

/// Two pods (a.ex, b.ex). Alice may Read both; Bob may Read only b.ex.
fn two_pod_store() -> PodStore {
    let nq = r#"
<https://a.ex/n1#it> <https://ex.dev/ns#k> "v" <https://a.ex/n1> .
<https://b.ex/m1#it> <https://ex.dev/ns#k> "v" <https://b.ex/m1> .
<https://a.ex/.acl#o> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://a.ex/.acl> .
<https://a.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> <https://a.ex/> <https://a.ex/.acl> .
<https://a.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://a.ex/.acl> .
<https://a.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://a.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> <https://b.ex/> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://b.ex/.acl> .
"#;
    let mut store = PodStore::new(sparq_core::Graph::load_dataset(nq, "nquads").expect("loads"));
    store.materialize_wac().expect("materializes");
    store
}

fn read_iris(store: &PodStore, s: &Session) -> Vec<String> {
    store.accessible(s, Mode::Read).iter().map(|n| n.as_str().to_owned()).collect()
}

#[test]
fn many_threads_share_one_ref_and_get_session_scoped_results() {
    let store = Arc::new(two_pod_store());
    let threads = 16;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();
    for t in 0..threads {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait(); // maximise real contention on the shared &PodStore
            for _ in 0..200 {
                // Alice sees both pods; Bob sees only b.ex; anon sees nothing.
                let alice = read_iris(&store, &sess(ALICE));
                assert_eq!(alice.len(), 2, "alice reads both pods (thread {t})");
                let bob = read_iris(&store, &sess(BOB));
                assert_eq!(bob, vec!["https://b.ex/m1".to_owned()], "bob reads only b.ex");
                // A full query goes through the same &self view path concurrently.
                let n = store
                    .query_as(&sess(ALICE), Mode::Read, "SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } }")
                    .expect("query ok")
                    .rows
                    .len();
                assert_eq!(n, 2, "alice's concurrent query sees both pod docs");
                assert!(read_iris(&store, &Session::default()).is_empty(), "anon fail-closed");
            }
        }));
    }
    for h in handles {
        h.join().expect("no reader panicked");
    }
}

#[test]
fn invalidation_under_concurrent_read_is_snapshot_consistent() {
    // A writer thread revokes alice on pod a WHILE many reader threads hammer accessible().
    // Generation pinning (#1584): each read pins one auth snapshot, so every observed set is
    // ATOMIC — either the pre-revoke {a,b} or the post-revoke {b}, never a torn set. After the
    // write is applied, readers converge on the revoked view.
    let store = Arc::new(std::sync::RwLock::new(two_pod_store()));
    let pre: Vec<String> = vec!["https://a.ex/n1".to_owned(), "https://b.ex/m1".to_owned()];
    let post: Vec<String> = vec!["https://b.ex/m1".to_owned()];

    let readers = 12;
    let barrier = Arc::new(Barrier::new(readers + 1));
    let mut handles = Vec::new();
    for _ in 0..readers {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let pre = pre.clone();
        let post = post.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..500 {
                // Concurrent READ access via the shared &PodStore (read lock = many readers).
                let guard = store.read().expect("rlock");
                let got = read_iris(&guard, &sess(ALICE));
                drop(guard);
                // The load-bearing invariant: NEVER a torn set. Only two atomic snapshots
                // are observable regardless of how the write interleaves.
                assert!(
                    got == pre || got == post,
                    "observed a NON-atomic (torn) set: {got:?}"
                );
            }
        }));
    }

    // The writer: after readers start, revoke alice on pod a (empty .acl on a.ex) once.
    let writer = {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            // Exclusive lock only for the write; the &mut is needed for put_acl (the reindex
            // seam). Readers proceed on either side; both snapshots are individually correct.
            let mut guard = store.write().expect("wlock");
            guard.put_acl("https://a.ex/.acl", "", "ntriples").expect("revoke");
        })
    };

    writer.join().expect("writer ok");
    for h in handles {
        h.join().expect("no reader panicked/torn");
    }

    // After the dust settles, every reader sees exactly the revoked view.
    let guard = store.read().expect("rlock");
    assert_eq!(read_iris(&guard, &sess(ALICE)), post, "converged on revoked view");
    // And it equals a from-scratch rebuild — no stale grant survived the concurrent churn.
    let fresh: Vec<String> = sparq_solid::AuthIndex::from_graph(guard.graph())
        .accessible(&sess(ALICE), Mode::Read)
        .iter()
        .map(|n| n.as_str().to_owned())
        .collect();
    assert_eq!(read_iris(&guard, &sess(ALICE)), fresh, "no stale grant after churn");
}
