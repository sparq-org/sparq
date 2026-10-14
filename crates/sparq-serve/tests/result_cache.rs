//! Correctness tests for the response-bytes result cache (feature `result-cache`,
//! research/concurrent-serving.md §6.3). sq-jluc.
//!
//! These exercise the REAL cache path — `lease`/`get`/`fulfill` against the real
//! `PodEpochs` invalidation type — not a mock that bypasses the logic. The
//! load-bearing invariants the bead names are each a named test:
//!   - hit/miss correctness,
//!   - the **visibility-scope isolation** invariant (a different access scope MUST
//!     miss — `scope_isolation_a_different_scope_misses`),
//!   - per-pod epoch-bump invalidation + the unbounded-footprint global-generation
//!     invalidation,
//!   - single-flight dedup (one execution, N waiters),
//!   - byte-budget LRU eviction + oversize admission rejection.
//!
//! Perf is canonical-pending (this work-box is non-canonical); these are pure
//! correctness assertions, which is the gate that CAN be satisfied here.
#![cfg(feature = "result-cache")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use sparq_serve::{
    Bytes, CacheConfig, LeaseOutcome, PodEpochs, PodId, ReadFootprint, ResultCache, ScopeKey,
    WaitOutcome,
};

/// A throwaway `PodEpochs` for a test. The crate exposes `bump` only crate-private
/// (the ring is the one mutation site), so tests drive invalidation through the
/// real ring. But for the pure-cache tests we want to vary epochs without standing
/// up a whole ring — so we publish through a `GenerationRing` to get a real,
/// correctly-bumped epoch vector. Helper below.
use sparq_serve::GenerationRing;

/// Publishes `touched` (one bump per pod) onto a fresh ring repeatedly to build a
/// real `PodEpochs` with the requested epoch for each pod. Returns the live epochs.
fn epochs_with(bumps: &[(&str, u64)]) -> PodEpochs {
    let ring = GenerationRing::new(());
    let mut total = 0u64;
    // Determine the max epoch we need to reach, publishing the right pods each step.
    let max = bumps.iter().map(|(_, n)| *n).max().unwrap_or(0);
    for step in 1..=max {
        let touched: Vec<PodId> = bumps
            .iter()
            .filter(|(_, n)| *n >= step)
            .map(|(p, _)| PodId::from(*p))
            .collect();
        ring.publish((), touched);
        total += 1;
    }
    let _ = total;
    ring.current().epochs().clone()
}

fn b(s: &str) -> Bytes {
    Arc::from(s.as_bytes())
}

const FMT: &str = "application/sparql-results+json";

/// The leader executes and fulfills; a subsequent identical request is a hit.
#[test]
fn hit_after_fulfill() {
    let cache = ResultCache::new();
    let epochs = PodEpochs::default();
    let q = "SELECT * WHERE { ?s ?p ?o }";
    let scope = ScopeKey::unrestricted();
    let footprint = ReadFootprint::Pods(vec![PodId::from("g1")]);

    // First call: cold miss → Lead.
    let bytes = match cache.lease(q, scope, FMT, footprint.clone(), &epochs, 0) {
        LeaseOutcome::Lead(guard) => guard.fulfill(b("ROWS"), &epochs, 0),
        other => panic!("expected Lead on cold miss, got {}", outcome_name(&other)),
    };
    assert_eq!(&*bytes, b"ROWS");

    // Second identical call: fresh Hit, no execution.
    match cache.lease(q, scope, FMT, footprint, &epochs, 0) {
        LeaseOutcome::Hit(hit) => assert_eq!(&*hit, b"ROWS"),
        other => panic!("expected Hit on repeat, got {}", outcome_name(&other)),
    }

    let stats = cache.stats();
    assert_eq!(stats.inserts, 1);
    assert_eq!(stats.hits, 1);
}

/// `get` (non-leasing probe) returns the cached bytes, or None on a cold key.
#[test]
fn get_probe_hit_and_miss() {
    let cache = ResultCache::new();
    let epochs = PodEpochs::default();
    let scope = ScopeKey::unrestricted();
    assert!(cache.get("ASK { ?s ?p ?o }", scope, FMT, &epochs, 0).is_none());

    if let LeaseOutcome::Lead(g) = cache.lease(
        "ASK { ?s ?p ?o }",
        scope,
        FMT,
        ReadFootprint::Pods(vec![]),
        &epochs,
        0,
    ) {
        g.fulfill(b("true"), &epochs, 0);
    } else {
        panic!("expected Lead");
    }
    assert_eq!(
        &*cache
            .get("ASK { ?s ?p ?o }", scope, FMT, &epochs, 0)
            .expect("hit"),
        b"true"
    );
}

/// THE load-bearing access-control invariant (§6.3, the Hasura lesson): bytes
/// cached for one visibility scope are NEVER served to a different scope. A request
/// under a different accessible-graph set MUST miss.
#[test]
fn scope_isolation_a_different_scope_misses() {
    let cache = ResultCache::new();
    let epochs = PodEpochs::default();
    let q = "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }";

    // Scope A can see {private/, public/}; scope B can see only {public/}.
    let scope_a = ScopeKey::of_graphs(["urn:pod:alice/private", "urn:pod:public"]);
    let scope_b = ScopeKey::of_graphs(["urn:pod:public"]);
    assert_ne!(scope_a, scope_b, "distinct accessible sets ⇒ distinct scope keys");

    // Cache a (potentially private-revealing) answer under scope A.
    if let LeaseOutcome::Lead(g) = cache.lease(
        q,
        scope_a,
        FMT,
        ReadFootprint::Unbounded,
        &epochs,
        0,
    ) {
        g.fulfill(b("ALICE_PRIVATE_ROWS"), &epochs, 0);
    } else {
        panic!("expected Lead under scope A");
    }

    // Scope A sees it (hit) — sanity.
    assert!(
        matches!(
            cache.lease(q, scope_a, FMT, ReadFootprint::Unbounded, &epochs, 0),
            LeaseOutcome::Hit(_)
        ),
        "scope A must hit its own cached bytes"
    );

    // Scope B (a DIFFERENT access scope, same query) MUST NOT see A's bytes — it
    // misses and gets to lead its own execution. The cache cannot leak across the
    // access-control boundary.
    match cache.lease(q, scope_b, FMT, ReadFootprint::Unbounded, &epochs, 0) {
        LeaseOutcome::Lead(_) => { /* correct: B leads its own, sees nothing of A */ }
        LeaseOutcome::Hit(bytes) => panic!(
            "ACCESS-CONTROL LEAK: scope B received scope A's cached bytes: {:?}",
            String::from_utf8_lossy(&bytes)
        ),
        LeaseOutcome::Wait(_) => panic!("unexpected concurrent lease"),
    }

    // Direct probe confirms the same: B never reads A's body.
    assert!(
        cache.get(q, scope_b, FMT, &epochs, 0).is_none(),
        "scope B probe must miss"
    );
}

/// The public-scope collapse: many requests under the SAME accessible set share one
/// key (the high-tenancy key-fragmentation mitigation). Two `of_graphs` calls with
/// the same set (any order) produce the same scope.
#[test]
fn same_scope_collapses_regardless_of_order() {
    let s1 = ScopeKey::of_graphs(["a", "b", "c"]);
    let s2 = ScopeKey::of_graphs(["c", "a", "b"]);
    assert_eq!(s1, s2, "scope key is order-independent over the graph set");
    // The empty (fail-closed) scope is its own stable key.
    assert_eq!(ScopeKey::empty(), ScopeKey::of_graphs(Vec::<&str>::new()));
    // Unrestricted is distinct from a restricted scope that lists no graphs.
    assert_ne!(ScopeKey::unrestricted(), ScopeKey::empty());
}

/// (sq-bif) The `from_raw`/`as_u64` round-trip a consumer that maintains its
/// OWN stable scope id relies on (the documented `from_raw` contract: equal scopes ⇒ equal
/// value), AND that the cache enforces scope isolation when keyed on those raw scopes — the
/// access-control boundary must hold no matter how the scope key was derived. A regression
/// that, say, salted `from_raw` would silently let one consumer-derived scope read another's
/// bytes; this asserts it does not.
#[test]
fn from_raw_round_trips_and_isolates() {
    // Round-trip: a derived scope id survives as_u64 -> from_raw unchanged.
    let derived = ScopeKey::of_graphs(["urn:pod:alice/private"]);
    assert_eq!(
        ScopeKey::from_raw(derived.as_u64()),
        derived,
        "from_raw(as_u64(s)) must equal s"
    );
    // Two consumer-supplied raw scope ids: equal raw ⇒ same key (hit), distinct raw ⇒
    // different key (miss). The cache cannot cross the access-control boundary.
    let cache = ResultCache::new();
    let epochs = PodEpochs::default();
    let q = "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }";
    let scope_x = ScopeKey::from_raw(0xA11CE);
    let scope_y = ScopeKey::from_raw(0xB0B);
    assert_ne!(scope_x, scope_y, "distinct raw scope ids ⇒ distinct keys");

    if let LeaseOutcome::Lead(g) =
        cache.lease(q, scope_x, FMT, ReadFootprint::Unbounded, &epochs, 0)
    {
        g.fulfill(b("X_ROWS"), &epochs, 0);
    } else {
        panic!("expected Lead under scope_x");
    }
    // The SAME raw scope id hits its own bytes (round-trip is faithful through the cache).
    assert_eq!(
        &*cache
            .get(q, ScopeKey::from_raw(0xA11CE), FMT, &epochs, 0)
            .expect("same raw scope id must hit"),
        b"X_ROWS"
    );
    // A DIFFERENT raw scope id must NOT see scope_x's bytes (fail-closed isolation).
    assert!(
        cache.get(q, scope_y, FMT, &epochs, 0).is_none(),
        "a different consumer-derived scope must miss — no cross-scope leak"
    );
    match cache.lease(q, scope_y, FMT, ReadFootprint::Unbounded, &epochs, 0) {
        LeaseOutcome::Lead(_) => { /* correct: scope_y leads its own, sees nothing of X */ }
        LeaseOutcome::Hit(bytes) => panic!(
            "ACCESS-CONTROL LEAK: scope_y received scope_x's bytes: {:?}",
            String::from_utf8_lossy(&bytes)
        ),
        LeaseOutcome::Wait(_) => panic!("unexpected concurrent lease"),
    }
}

/// Per-pod epoch-bump invalidation: a write to a graph the query touched bumps that
/// pod's epoch, so the recorded entry goes stale and a re-read misses; a write to an
/// UNRELATED pod leaves the entry fresh (no churn — the whole point of per-pod
/// epochs).
#[test]
fn epoch_bump_invalidates_touched_pod_only() {
    let cache = ResultCache::new();
    let q = "SELECT * WHERE { GRAPH <urn:g1> { ?s ?p ?o } }";
    let scope = ScopeKey::unrestricted();
    let footprint = ReadFootprint::Pods(vec![PodId::from("urn:g1")]);

    // Cache under epoch state where g1=0, g2=0.
    let e0 = epochs_with(&[]);
    if let LeaseOutcome::Lead(g) = cache.lease(q, scope, FMT, footprint.clone(), &e0, 0) {
        g.fulfill(b("G1_ROWS"), &e0, 0);
    } else {
        panic!("expected Lead");
    }

    // A write to an UNRELATED pod (g2) — g1 still 0 — entry stays fresh.
    let e_g2 = epochs_with(&[("urn:g2", 1)]);
    assert!(
        cache.get(q, scope, FMT, &e_g2, 1).is_some(),
        "an unrelated-pod write must NOT invalidate (per-pod epochs)"
    );

    // A write to g1 (the touched pod) bumps g1's epoch → entry is stale → miss.
    let e_g1 = epochs_with(&[("urn:g1", 1)]);
    assert!(
        cache.get(q, scope, FMT, &e_g1, 1).is_none(),
        "a write to the touched pod MUST invalidate"
    );
    assert_eq!(cache.stats().stale, 1, "the stale entry was counted + evicted");
}

/// Unbounded-footprint entries pin the global generation: ANY write (generation
/// advance) invalidates them — the conservative, answer-safe path for queries whose
/// read footprint cannot be statically enumerated.
#[test]
fn unbounded_footprint_invalidated_by_any_write() {
    let cache = ResultCache::new();
    let q = "SELECT * WHERE { ?s ?p ?o }"; // no GRAPH restriction → unbounded
    let scope = ScopeKey::unrestricted();
    let epochs = PodEpochs::default();

    if let LeaseOutcome::Lead(g) = cache.lease(
        q,
        scope,
        FMT,
        ReadFootprint::Unbounded,
        &epochs,
        7, // cached at generation 7
    ) {
        g.fulfill(b("ALL"), &epochs, 7);
    } else {
        panic!("expected Lead");
    }
    // Same generation → still fresh.
    assert!(cache.get(q, scope, FMT, &epochs, 7).is_some());
    // Any later generation → invalidated.
    assert!(
        cache.get(q, scope, FMT, &epochs, 8).is_none(),
        "unbounded entry must invalidate on any generation advance"
    );
}

/// Single-flight / lease dedup: under a stampede of N identical concurrent
/// requests, exactly ONE executes and the rest receive its bytes (§5 verdict 1).
#[test]
fn single_flight_one_execution_n_waiters() {
    let cache = ResultCache::new();
    let q = "SELECT * WHERE { ?s ?p ?o }";
    let scope = ScopeKey::unrestricted();
    let n = 16usize;
    let executions = Arc::new(AtomicU64::new(0));
    let barrier = Arc::new(Barrier::new(n));

    let handles: Vec<_> = (0..n)
        .map(|_| {
            let cache = Arc::clone(&cache);
            let executions = Arc::clone(&executions);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let epochs = PodEpochs::default();
                barrier.wait(); // maximize the stampede overlap
                let footprint = ReadFootprint::Pods(vec![PodId::from("g")]);
                match cache.lease(q, scope, FMT, footprint.clone(), &epochs, 0) {
                    LeaseOutcome::Hit(bytes) => bytes,
                    LeaseOutcome::Lead(guard) => {
                        // Simulate the one real execution.
                        executions.fetch_add(1, Ordering::SeqCst);
                        thread::sleep(std::time::Duration::from_millis(5));
                        guard.fulfill(b("THE_ANSWER"), &epochs, 0)
                    }
                    LeaseOutcome::Wait(waiter) => match waiter.wait(footprint) {
                        WaitOutcome::Bytes(bytes) => bytes,
                        WaitOutcome::Promoted(guard) => {
                            executions.fetch_add(1, Ordering::SeqCst);
                            guard.fulfill(b("THE_ANSWER"), &epochs, 0)
                        }
                    },
                }
            })
        })
        .collect();

    let results: Vec<Bytes> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Every caller got the same answer...
    for r in &results {
        assert_eq!(&**r, b"THE_ANSWER");
    }
    // ...but it was executed exactly once (the leader; no one was promoted because
    // the leader never abandoned).
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "single-flight must collapse the stampede to one execution"
    );
}

/// An abandoned lease (leader drops without fulfilling) promotes a waiter to leader
/// rather than wedging the waiters forever.
#[test]
fn abandoned_lease_promotes_a_waiter() {
    let cache = ResultCache::new();
    let q = "SELECT 1";
    let scope = ScopeKey::unrestricted();
    let epochs = PodEpochs::default();
    let footprint = ReadFootprint::Pods(vec![]);

    // Leader takes the lease then ABANDONS it (drops the guard unfulfilled).
    let waiter = match cache.lease(q, scope, FMT, footprint.clone(), &epochs, 0) {
        LeaseOutcome::Lead(lead) => {
            // Open a concurrent waiter on the SAME key before abandoning.
            let w = match cache.lease(q, scope, FMT, footprint.clone(), &epochs, 0) {
                LeaseOutcome::Wait(w) => w,
                other => panic!("expected Wait, got {}", outcome_name(&other)),
            };
            drop(lead); // abandon
            w
        }
        other => panic!("expected Lead, got {}", outcome_name(&other)),
    };

    // The waiter is promoted (no bytes were published) and can now fulfill.
    match waiter.wait(footprint) {
        WaitOutcome::Promoted(guard) => {
            let bytes = guard.fulfill(b("RECOVERED"), &epochs, 0);
            assert_eq!(&*bytes, b"RECOVERED");
        }
        WaitOutcome::Bytes(_) => panic!("waiter should have been promoted after abandon"),
    }
    assert_eq!(&*cache.get(q, scope, FMT, &epochs, 0).expect("now cached"), b"RECOVERED");
}

/// Byte-budget LRU: with a tiny budget, inserting past it evicts the
/// least-recently-used entry; a recently-touched entry survives.
#[test]
fn lru_eviction_under_byte_budget() {
    // Budget holds ~2 of these 100-byte bodies.
    let cache = ResultCache::with_config(CacheConfig {
        max_bytes: 250,
        max_entry_bytes: 200,
        rename_variables: false,
    });
    let epochs = PodEpochs::default();
    let scope = ScopeKey::unrestricted();
    let body = b(&"x".repeat(100));

    let insert = |q: &str| {
        if let LeaseOutcome::Lead(g) =
            cache.lease(q, scope, FMT, ReadFootprint::Pods(vec![]), &epochs, 0)
        {
            g.fulfill(body.clone(), &epochs, 0);
        }
    };

    insert("Q1");
    insert("Q2");
    // Touch Q1 so it is more-recently-used than Q2.
    assert!(cache.get("Q1", scope, FMT, &epochs, 0).is_some());
    // Insert Q3 → over budget (300 > 250) → evict the LRU, which is now Q2.
    insert("Q3");

    assert!(cache.get("Q1", scope, FMT, &epochs, 0).is_some(), "Q1 (recently used) survives");
    assert!(cache.get("Q3", scope, FMT, &epochs, 0).is_some(), "Q3 (just inserted) present");
    assert!(cache.get("Q2", scope, FMT, &epochs, 0).is_none(), "Q2 (LRU) was evicted");
    assert!(cache.stats().evictions >= 1);
    assert!(cache.bytes_used() <= 250, "never exceeds the byte budget");
}

/// Admission: a body larger than `max_entry_bytes` is never cached (oversize /
/// streaming results, §6.3) — but the bytes are still returned to the caller.
#[test]
fn oversize_body_is_not_admitted() {
    let cache = ResultCache::with_config(CacheConfig {
        max_bytes: 1_000_000,
        max_entry_bytes: 64,
        rename_variables: false,
    });
    let epochs = PodEpochs::default();
    let scope = ScopeKey::unrestricted();
    let big = b(&"y".repeat(1000)); // > 64

    let returned = match cache.lease(
        "BIG",
        scope,
        FMT,
        ReadFootprint::Pods(vec![]),
        &epochs,
        0,
    ) {
        LeaseOutcome::Lead(g) => g.fulfill(big.clone(), &epochs, 0),
        other => panic!("expected Lead, got {}", outcome_name(&other)),
    };
    // Caller still got the bytes...
    assert_eq!(returned.len(), 1000);
    // ...but nothing was retained.
    assert!(cache.is_empty(), "oversize body must not be admitted");
    assert_eq!(cache.stats().admission_rejects, 1);
    assert!(cache.get("BIG", scope, FMT, &epochs, 0).is_none());
}

/// The result format is part of the key: the same query in two media is two bodies.
#[test]
fn format_is_part_of_the_key() {
    let cache = ResultCache::new();
    let epochs = PodEpochs::default();
    let scope = ScopeKey::unrestricted();
    let q = "SELECT * WHERE { ?s ?p ?o }";

    if let LeaseOutcome::Lead(g) =
        cache.lease(q, scope, FMT, ReadFootprint::Pods(vec![]), &epochs, 0)
    {
        g.fulfill(b("JSON"), &epochs, 0);
    }
    // A different format misses (it would be a different serialization).
    assert!(cache.get(q, scope, "text/csv", &epochs, 0).is_none());
    assert!(cache.get(q, scope, FMT, &epochs, 0).is_some());
}

/// Canonicalization: whitespace-only differences hit the same entry (label-safe
/// default).
#[test]
fn whitespace_variants_hit_same_entry() {
    let cache = ResultCache::new();
    let epochs = PodEpochs::default();
    let scope = ScopeKey::unrestricted();

    if let LeaseOutcome::Lead(g) = cache.lease(
        "SELECT ?x WHERE { ?x ?p ?o }",
        scope,
        FMT,
        ReadFootprint::Pods(vec![]),
        &epochs,
        0,
    ) {
        g.fulfill(b("R"), &epochs, 0);
    }
    // Reindented / re-spaced form of the SAME query hits.
    assert!(
        cache
            .get("SELECT   ?x\nWHERE {\t?x ?p ?o }", scope, FMT, &epochs, 0)
            .is_some(),
        "whitespace-normalized query must hit the same entry"
    );
}

fn outcome_name(o: &LeaseOutcome) -> &'static str {
    match o {
        LeaseOutcome::Hit(_) => "Hit",
        LeaseOutcome::Lead(_) => "Lead",
        LeaseOutcome::Wait(_) => "Wait",
    }
}
