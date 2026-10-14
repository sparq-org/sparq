//! Distributed-backend `odrl:count` enforcement tests for [`BackendCounterStore`] — the
//! Redis-`INCR` / SQL-`UPDATE…WHERE consumed<limit RETURNING` follow-up (sq-u3yo) to the
//! single-host [`FileCounterStore`] (sq-5z1q).
//!
//! The bead asks for Redis / SQL *distributed* [`UsageCounterStore`] backends as drop-ins
//! against the **unchanged** trait. A live Redis/Postgres dependency does not belong in
//! this `publish = false`, std-only, "dependency of nothing" crate (and CI has no such
//! service to gate against). What this crate ships instead is the soundly-testable seam an
//! operator drops a real backend into: the [`AtomicCounterBackend`] trait — ONE atomic
//! conditional-increment primitive that is EXACTLY a Redis `INCR`-with-cap or a SQL
//! `UPDATE … WHERE consumed < limit RETURNING` — and the [`BackendCounterStore`] adapter
//! that turns any such backend into a full [`UsageCounterStore`] obeying the identical
//! `try_consume` contract. These tests exercise that adapter through an in-crate reference
//! backend that reproduces the exact atomic semantics of those two systems, proving the
//! adapter logic (the part that lives in THIS crate) is correct.
//!
//! Gated on `count-enforcement` (the whole feature).
#![cfg(feature = "count-enforcement")]

use sparq_policy::{
    evaluate_and_exercise, parse_policy_str, AtomicCounterBackend, BackendCounterStore,
    BackendError, BackendOutcome, ConsumeResult, CountKey, Request, UsageCounterStore,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

// ---------------------------------------------------------------------------
// A reference `AtomicCounterBackend` that reproduces, in-process, the EXACT atomic
// semantics a Redis `INCR`-with-cap or a SQL `UPDATE…WHERE consumed<limit RETURNING`
// gives over the wire: one indivisible "if consumed < limit then consumed += 1 and
// return the new value, else report exhausted". A `Mutex` stands in for the server's
// single-threaded atomic execution. This is the in-crate analogue of the file store's
// own in-process reference — it lets us test the ADAPTER (the code we ship) without a
// live database.
// ---------------------------------------------------------------------------
#[derive(Default)]
struct ReferenceBackend {
    counts: Mutex<HashMap<String, u64>>,
    /// When set, the next backend call fails (simulates a connection drop / server error).
    fail_next: AtomicBool,
}

impl ReferenceBackend {
    fn fail_once(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }
}

impl AtomicCounterBackend for ReferenceBackend {
    fn checked_increment(&self, key: &str, limit: u64) -> Result<BackendOutcome, BackendError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(BackendError::new("simulated backend outage"));
        }
        let mut counts = self.counts.lock().unwrap();
        let entry = counts.entry(key.to_string()).or_insert(0);
        if *entry >= limit {
            Ok(BackendOutcome::Exhausted)
        } else {
            *entry += 1;
            Ok(BackendOutcome::Incremented(*entry))
        }
    }

    fn read(&self, key: &str) -> Result<u64, BackendError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(BackendError::new("simulated backend outage"));
        }
        Ok(self.counts.lock().unwrap().get(key).copied().unwrap_or(0))
    }
}

fn key(rule: &str, party: &str, target: &str) -> CountKey {
    CountKey {
        rule_id: rule.into(),
        party: party.into(),
        target: target.into(),
    }
}

// ---------------------------------------------------------------------------
// 1. The adapter implements `try_consume`: first N grant, the (N+1)th exhausts, and the
//    new consumed count is reported on each grant.
// ---------------------------------------------------------------------------
#[test]
fn adapter_grants_then_exhausts() {
    let store = BackendCounterStore::new(ReferenceBackend::default());
    let k = key("r", "alice", "x");
    assert_eq!(store.try_consume(&k, 2), ConsumeResult::Consumed(1));
    assert_eq!(store.try_consume(&k, 2), ConsumeResult::Consumed(2));
    assert_eq!(store.try_consume(&k, 2), ConsumeResult::Exhausted);
    assert_eq!(store.consumed(&k), Some(2));
}

// ---------------------------------------------------------------------------
// 2. A backend error (connection drop, server error) maps to Unavailable — fail-closed,
//    never a silent grant.
// ---------------------------------------------------------------------------
#[test]
fn backend_error_is_unavailable_fail_closed() {
    let backend = ReferenceBackend::default();
    backend.fail_once();
    let store = BackendCounterStore::new(backend);
    let k = key("r", "alice", "x");
    // The first try_consume hits the simulated outage → Unavailable (a DENY upstream).
    assert_eq!(store.try_consume(&k, 5), ConsumeResult::Unavailable);
    // The backend recovers on the next call (fail-once) → normal service resumes.
    assert_eq!(store.try_consume(&k, 5), ConsumeResult::Consumed(1));
}

// ---------------------------------------------------------------------------
// 3. `consumed` reflects the backend read, and a backend read error is None (cannot
//    report → fail-closed at the caller).
// ---------------------------------------------------------------------------
#[test]
fn consumed_reads_through_and_errors_are_none() {
    let backend = ReferenceBackend::default();
    let store = BackendCounterStore::new(backend);
    let k = key("r", "alice", "x");
    assert_eq!(store.consumed(&k), Some(0));
    store.try_consume(&k, 5);
    assert_eq!(store.consumed(&k), Some(1));
    // Now make the next read fail → None.
    store.backend().fail_once();
    assert_eq!(store.consumed(&k), None);
}

// ---------------------------------------------------------------------------
// 4. Distinct (rule, party, target) keys map to distinct backend keys → independent
//    budgets, and the key encoding is injective (no two CountKeys collide on one backend
//    string key — the property a Redis/SQL primary key depends on).
// ---------------------------------------------------------------------------
#[test]
fn distinct_keys_have_independent_budgets() {
    let store = BackendCounterStore::new(ReferenceBackend::default());
    let alice = key("r", "alice", "x");
    let bob = key("r", "bob", "x");
    assert_eq!(store.try_consume(&alice, 1), ConsumeResult::Consumed(1));
    assert_eq!(store.try_consume(&alice, 1), ConsumeResult::Exhausted);
    // Bob's budget is untouched.
    assert_eq!(store.try_consume(&bob, 1), ConsumeResult::Consumed(1));
}

// ---------------------------------------------------------------------------
// 5. The backend-key encoding is INJECTIVE across confusable field boundaries — two
//    CountKeys whose fields differ only in where a separator-looking byte falls must NOT
//    collapse to the same backend key (else two distinct budgets would share one row).
// ---------------------------------------------------------------------------
#[test]
fn backend_key_encoding_is_injective() {
    let store = BackendCounterStore::new(ReferenceBackend::default());
    // ("a|b", "", "") vs ("a", "b", "") — naive `join('|')` would collide both to "a|b||".
    let a = key("a|b", "", "");
    let b = key("a", "b", "");
    assert_eq!(store.try_consume(&a, 1), ConsumeResult::Consumed(1));
    // If they collided, `b` would already be exhausted; it must still have its own budget.
    assert_eq!(store.try_consume(&b, 1), ConsumeResult::Consumed(1));
}

// ---------------------------------------------------------------------------
// 6. limit 0 never grants (the (N+1)th-when-N=0 case): the backend reports Exhausted
//    immediately.
// ---------------------------------------------------------------------------
#[test]
fn limit_zero_never_grants() {
    let store = BackendCounterStore::new(ReferenceBackend::default());
    assert_eq!(
        store.try_consume(&key("r", "p", "t"), 0),
        ConsumeResult::Exhausted
    );
}

// ---------------------------------------------------------------------------
// 7. The adapter is a drop-in through the REAL evaluate_and_exercise path: first N
//    exercises grant, the (N+1)th denies — identical semantics to the in-memory / file
//    stores, proving it satisfies the same trait contract.
// ---------------------------------------------------------------------------
#[test]
fn adapter_through_real_evaluator() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:lteq ;
                      odrl:rightOperand "3"^^xsd:integer ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let store = BackendCounterStore::new(ReferenceBackend::default());
    let req = Request::new(format!("{ODRL}read"))
        .on("urn:asset/x")
        .by("https://alice.ex/me");

    for expected in 1..=3 {
        let d = evaluate_and_exercise(&p, &req, &store);
        assert!(d.allow, "exercise {expected} should grant: {d:?}");
        assert_eq!(d.consumed, Some(expected));
    }
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(!d.allow, "4th exercise must deny (limit reached)");
    assert_eq!(d.consumed, None);
}

// ---------------------------------------------------------------------------
// 8. Concurrency: many threads sharing ONE backend over one key grant EXACTLY the limit,
//    never over — the adapter does not reintroduce a TOCTOU window on top of the backend's
//    atomic primitive (the whole decision is the single `checked_increment` call).
// ---------------------------------------------------------------------------
#[test]
fn concurrent_threads_never_overgrant() {
    use std::sync::atomic::AtomicU64;
    const LIMIT: u64 = 50;
    const THREADS: usize = 8;
    const PER_THREAD: usize = 40;

    let store = Arc::new(BackendCounterStore::new(ReferenceBackend::default()));
    let granted = Arc::new(AtomicU64::new(0));
    let k = key("r", "alice", "x");

    let mut handles = Vec::new();
    for _ in 0..THREADS {
        let store = Arc::clone(&store);
        let granted = Arc::clone(&granted);
        let k = k.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..PER_THREAD {
                if matches!(store.try_consume(&k, LIMIT), ConsumeResult::Consumed(_)) {
                    granted.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        granted.load(Ordering::Relaxed),
        LIMIT,
        "must grant exactly the limit"
    );
    assert_eq!(store.consumed(&k), Some(LIMIT));
}
