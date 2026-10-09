//! Stateful `odrl:count` enforcement tests, through the REAL evaluator +
//! counter-store path. [OPUS-4.8] sq-zi5w.
//!
//! Coverage: first N exercises grant + (N+1)th denies (`lteq`, `lt`, `eq`); a denied
//! base decision burns no budget; per-(party, target, rule) budget isolation; the
//! counter-unavailable fail-closed path; the malformed-limit fail-closed path; the
//! atomicity of the default in-memory store under concurrency; and `count_status` as a
//! side-effect-free audit surface.
//!
//! Gated on `count-enforcement` (the whole feature). The default-feature test suite
//! (`odrl_eval.rs`) still exercises the *stateless* `odrl:count` comparison unchanged.
#![cfg(feature = "count-enforcement")]

use sparq_policy::{
    count_status, evaluate_and_exercise, parse_policy_str, ConsumeResult, CountKey, CountStatus,
    InMemoryCounterStore, Request, UsageCounterStore,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";

fn read() -> String {
    format!("{ODRL}read")
}

/// "alice may read asset/x at most N times" under `operator`.
fn policy_at_most(operator: &str, bound: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:{operator} ;
                      odrl:rightOperand "{bound}"^^xsd:integer ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

fn alice_reads() -> Request {
    Request::new(read()).on("urn:asset/x").by("https://alice.ex/me")
}

// ---------------------------------------------------------------------------
// 1. lteq N: the first N exercises grant; the (N+1)th denies.
// ---------------------------------------------------------------------------
#[test]
fn lteq_grants_n_then_denies() {
    let p = policy_at_most("lteq", "3");
    let store = InMemoryCounterStore::new();
    let req = alice_reads();

    for expected in 1..=3 {
        let d = evaluate_and_exercise(&p, &req, &store);
        assert!(d.allow, "exercise {expected} should grant: {d:?}");
        assert_eq!(d.consumed, Some(expected));
    }
    // 4th: limit reached → DENY, and nothing further consumed.
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(!d.allow, "4th exercise must deny");
    assert_eq!(d.consumed, None);
    // The store still reports exactly 3 consumed (the deny did not burn budget).
    let key = CountKey {
        rule_id: p.permissions[0].id.clone(),
        party: "https://alice.ex/me".into(),
        target: "urn:asset/x".into(),
    };
    assert_eq!(store.consumed(&key), Some(3));
}

// ---------------------------------------------------------------------------
// 2. lt N: "< N" means at most N-1 exercises.
// ---------------------------------------------------------------------------
#[test]
fn lt_means_n_minus_one() {
    let p = policy_at_most("lt", "3"); // at most 2
    let store = InMemoryCounterStore::new();
    let req = alice_reads();
    assert!(evaluate_and_exercise(&p, &req, &store).allow);
    assert!(evaluate_and_exercise(&p, &req, &store).allow);
    assert!(!evaluate_and_exercise(&p, &req, &store).allow, "3rd must deny under lt 3");
}

// ---------------------------------------------------------------------------
// 3. eq N: the "exactly N uses" shorthand = at most N.
// ---------------------------------------------------------------------------
#[test]
fn eq_means_at_most_n() {
    let p = policy_at_most("eq", "2");
    let store = InMemoryCounterStore::new();
    let req = alice_reads();
    assert!(evaluate_and_exercise(&p, &req, &store).allow);
    assert!(evaluate_and_exercise(&p, &req, &store).allow);
    assert!(!evaluate_and_exercise(&p, &req, &store).allow);
}

// ---------------------------------------------------------------------------
// 4. A base DENY (wrong party) burns NO budget.
// ---------------------------------------------------------------------------
#[test]
fn base_deny_consumes_nothing() {
    let p = policy_at_most("lteq", "2");
    let store = InMemoryCounterStore::new();
    let mallory = Request::new(read()).on("urn:asset/x").by("https://mallory.ex/me");
    // Mallory is not the assignee → base evaluator denies → no consume.
    let d = evaluate_and_exercise(&p, &mallory, &store);
    assert!(!d.allow);
    assert_eq!(d.consumed, None);
    // Alice still has her full budget of 2.
    assert!(evaluate_and_exercise(&p, &alice_reads(), &store).allow);
    assert!(evaluate_and_exercise(&p, &alice_reads(), &store).allow);
    assert!(!evaluate_and_exercise(&p, &alice_reads(), &store).allow);
}

// ---------------------------------------------------------------------------
// 5. Budget is isolated per (party, target): one party exhausting does not lock
//    out another, and a different target counts separately.
// ---------------------------------------------------------------------------
#[test]
fn budget_isolated_per_party_and_target() {
    // No assignee on the rule → any party may read (count is per-party regardless).
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "1"^^xsd:integer ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let store = InMemoryCounterStore::new();

    let alice = Request::new(read()).on("urn:asset/x").by("https://alice.ex/me");
    let bob = Request::new(read()).on("urn:asset/x").by("https://bob.ex/me");
    let alice_other = Request::new(read()).on("urn:asset/y").by("https://alice.ex/me");

    assert!(evaluate_and_exercise(&p, &alice, &store).allow);
    assert!(!evaluate_and_exercise(&p, &alice, &store).allow, "alice's budget on x is spent");
    // Bob has his own budget on x.
    assert!(evaluate_and_exercise(&p, &bob, &store).allow);
    // Alice has a separate budget on a different target.
    assert!(evaluate_and_exercise(&p, &alice_other, &store).allow);
}

// ---------------------------------------------------------------------------
// 6. Counter UNAVAILABLE → DENY (fail-closed): never grant beyond an unaccountable
//    limit.
// ---------------------------------------------------------------------------
struct UnavailableStore;
impl UsageCounterStore for UnavailableStore {
    fn try_consume(&self, _k: &CountKey, _l: u64) -> ConsumeResult {
        ConsumeResult::Unavailable
    }
    fn consumed(&self, _k: &CountKey) -> Option<u64> {
        None
    }
}

#[test]
fn counter_unavailable_fails_closed() {
    let p = policy_at_most("lteq", "5");
    let store = UnavailableStore;
    let d = evaluate_and_exercise(&p, &alice_reads(), &store);
    assert!(!d.allow, "an unavailable counter must DENY, not grant");
    assert!(d.reasons.iter().any(|r| r.contains("unavailable")));
    // count_status agrees: Unprovable (never silently "unlimited").
    assert_eq!(
        count_status(&p.permissions[0], &alice_reads(), &store),
        CountStatus::Unprovable
    );
}

// ---------------------------------------------------------------------------
// 7. Malformed count bound → DENY (fail-closed).
// ---------------------------------------------------------------------------
#[test]
fn malformed_limit_fails_closed() {
    // A non-numeric right-operand on the count constraint.
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ; odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "not-a-number" ] ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let store = InMemoryCounterStore::new();
    let d = evaluate_and_exercise(&p, &alice_reads(), &store);
    assert!(!d.allow, "a malformed count bound must DENY");
}

// ---------------------------------------------------------------------------
// 8. count_status: side-effect-free audit surface (does NOT consume).
// ---------------------------------------------------------------------------
#[test]
fn count_status_does_not_consume() {
    let p = policy_at_most("lteq", "2");
    let store = InMemoryCounterStore::new();
    let rule = &p.permissions[0];
    let req = alice_reads();

    // Before any exercise: Satisfied(0/2). Querying it twice consumes nothing.
    assert_eq!(count_status(rule, &req, &store), CountStatus::Satisfied { consumed: 0, limit: 2 });
    assert_eq!(count_status(rule, &req, &store), CountStatus::Satisfied { consumed: 0, limit: 2 });

    // Exercise once → status reflects 1 consumed, still satisfied.
    assert!(evaluate_and_exercise(&p, &req, &store).allow);
    assert_eq!(count_status(rule, &req, &store), CountStatus::Satisfied { consumed: 1, limit: 2 });

    // Exhaust → DefinitelyUnsatisfied.
    assert!(evaluate_and_exercise(&p, &req, &store).allow);
    assert_eq!(
        count_status(rule, &req, &store),
        CountStatus::DefinitelyUnsatisfied { consumed: 2, limit: 2 }
    );
}

// ---------------------------------------------------------------------------
// 9. A permission with NO count constraint is unaffected (NotConstrained) and grants
//    unboundedly.
// ---------------------------------------------------------------------------
#[test]
fn no_count_constraint_unbounded() {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ; odrl:assignee <https://alice.ex/me> ] .
"#
    );
    let p = parse_policy_str(&ttl, "turtle").unwrap();
    let store = InMemoryCounterStore::new();
    assert_eq!(count_status(&p.permissions[0], &alice_reads(), &store), CountStatus::NotConstrained);
    for _ in 0..10 {
        let d = evaluate_and_exercise(&p, &alice_reads(), &store);
        assert!(d.allow);
        assert_eq!(d.consumed, None, "an unconstrained grant touches no counter");
    }
}

/// "alice may read asset/x" under TWO `odrl:count` constraints (a tighter and a looser
/// limit on the same rule → same usage counter). [OPUS-4.8] sq-ea27.
fn policy_two_counts(op_a: &str, bound_a: &str, op_b: &str, bound_b: &str) -> sparq_policy::ValidatedPolicy {
    let ttl = format!(
        r#"
@prefix odrl: <{ODRL}> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/c> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:assignee <https://alice.ex/me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:{op_a} ;
                      odrl:rightOperand "{bound_a}"^^xsd:integer ] ,
                    [ odrl:leftOperand odrl:count ;
                      odrl:operator odrl:{op_b} ;
                      odrl:rightOperand "{bound_b}"^^xsd:integer ] ] .
"#
    );
    parse_policy_str(&ttl, "turtle").unwrap()
}

// ---------------------------------------------------------------------------
// 10. MULTI-LIMIT: two `odrl:count` constraints on one rule share ONE usage counter
//     (same CountKey). The TIGHTEST limit governs, and each exercise consumes EXACTLY
//     ONE unit — never one-per-constraint (the multi-consume bug). [OPUS-4.8] sq-ea27.
// ---------------------------------------------------------------------------
#[test]
fn two_counts_tightest_governs_and_consumes_one() {
    // lteq 5 AND lteq 2 → effective limit 2; the looser 5 never relaxes it.
    let p = policy_two_counts("lteq", "5", "lteq", "2");
    let store = InMemoryCounterStore::new();
    let req = alice_reads();

    // First exercise consumes exactly ONE unit (not one-per-constraint → would be 2).
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(d.allow, "exercise 1 should grant: {d:?}");
    assert_eq!(d.consumed, Some(1), "one exercise consumes exactly one unit");

    // Second exercise: still within the tightest limit of 2.
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(d.allow, "exercise 2 should grant: {d:?}");
    assert_eq!(d.consumed, Some(2));

    // Third: the tightest limit (2) is reached → DENY, nothing consumed.
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(!d.allow, "exercise 3 must deny under the tightest (2) limit");
    assert_eq!(d.consumed, None);

    // Exactly 2 consumed — the looser limit of 5 never granted a 3rd, and no exercise
    // burned more than one unit.
    let key = CountKey {
        rule_id: p.permissions[0].id.clone(),
        party: "https://alice.ex/me".into(),
        target: "urn:asset/x".into(),
    };
    assert_eq!(store.consumed(&key), Some(2));
}

// ---------------------------------------------------------------------------
// 11. MULTI-LIMIT ATOMICITY: under concurrent exercises against a rule carrying TWO
//     count constraints, the store grants EXACTLY the tightest limit — never over-grants
//     (the multi-limit TOCTOU window between the pre-check and the consume is closed).
//     [OPUS-4.8] sq-ea27.
// ---------------------------------------------------------------------------
#[test]
fn concurrent_multi_limit_never_overgrants() {
    const TIGHTEST: u64 = 100;
    const THREADS: usize = 16;
    const PER_THREAD: usize = 50; // 800 attempts >> 100 budget

    // lteq 100 (tightest) AND lteq 250 (looser) on ONE rule → one shared counter.
    let p = Arc::new(policy_two_counts("lteq", "100", "lteq", "250"));
    let store = Arc::new(InMemoryCounterStore::new());
    let granted = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    for _ in 0..THREADS {
        let p = Arc::clone(&p);
        let store = Arc::clone(&store);
        let granted = Arc::clone(&granted);
        handles.push(std::thread::spawn(move || {
            let req = alice_reads();
            for _ in 0..PER_THREAD {
                if evaluate_and_exercise(&p, &req, &*store).allow {
                    granted.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // EXACTLY the tightest limit granted — never one more (the multi-limit TOCTOU is
    // closed) — and the single shared counter agrees.
    assert_eq!(
        granted.load(Ordering::Relaxed),
        TIGHTEST,
        "must grant exactly the tightest limit, never over-grant"
    );
    let key = CountKey {
        rule_id: p.permissions[0].id.clone(),
        party: "https://alice.ex/me".into(),
        target: "urn:asset/x".into(),
    };
    assert_eq!(store.consumed(&key), Some(TIGHTEST));
}

// ---------------------------------------------------------------------------
// 12. MULTI-LIMIT pre-exhausted: if the tightest limit is already reached, a further
//     exercise denies and burns nothing. [OPUS-4.8] sq-ea27.
// ---------------------------------------------------------------------------
#[test]
fn two_counts_denies_when_tightest_exhausted() {
    let p = policy_two_counts("lteq", "1", "lteq", "9");
    let store = InMemoryCounterStore::new();
    let req = alice_reads();

    assert!(evaluate_and_exercise(&p, &req, &store).allow, "first exercise grants");
    let d = evaluate_and_exercise(&p, &req, &store);
    assert!(!d.allow, "second exercise denies under the tightest (1) limit");
    assert_eq!(d.consumed, None);
    // count_status agrees the tightest limit is exhausted.
    assert_eq!(
        count_status(&p.permissions[0], &req, &store),
        CountStatus::DefinitelyUnsatisfied { consumed: 1, limit: 1 }
    );
}

// ---------------------------------------------------------------------------
// 13. ATOMICITY: under concurrent exercises against the SAME key, the in-memory store
//     grants EXACTLY the limit — never over-grants (the TOCTOU race is closed).
// ---------------------------------------------------------------------------
#[test]
fn concurrent_exercises_never_overgrant() {
    const LIMIT: u64 = 100;
    const THREADS: usize = 16;
    const PER_THREAD: usize = 50; // 16*50 = 800 attempts >> 100 budget

    let p = Arc::new(policy_at_most("lteq", "100"));
    let store = Arc::new(InMemoryCounterStore::new());
    let granted = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    for _ in 0..THREADS {
        let p = Arc::clone(&p);
        let store = Arc::clone(&store);
        let granted = Arc::clone(&granted);
        handles.push(std::thread::spawn(move || {
            let req = alice_reads();
            for _ in 0..PER_THREAD {
                if evaluate_and_exercise(&p, &req, &*store).allow {
                    granted.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // EXACTLY the limit was granted — not one more (atomicity) and not fewer (no lost
    // grant), and the store's final consumed count agrees.
    assert_eq!(granted.load(Ordering::Relaxed), LIMIT, "must grant exactly the limit");
    let key = CountKey {
        rule_id: p.permissions[0].id.clone(),
        party: "https://alice.ex/me".into(),
        target: "urn:asset/x".into(),
    };
    assert_eq!(store.consumed(&key), Some(LIMIT));
}
