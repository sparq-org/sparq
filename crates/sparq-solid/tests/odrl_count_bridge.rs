//! Stateful `odrl:count` enforcement and the `sparq-solid` ODRL→ACP bridge.
//!
//! ACP is stateless (no usage counter), and a stored allow would let every later read
//! through after a single exercise, so a count-limited grant is never stored: its
//! permit is not lasting, and the counted bridge entry point refuses it before spending
//! any budget. Counted access goes through `evaluate_and_exercise` per request. A
//! permission with no count limit still bridges through the counted entry point.
//!
//! Gated by the `count-enforcement` feature (the whole file no-ops without it).
#![cfg(feature = "count-enforcement")]

use sparq_core::Graph;
use sparq_policy::{parse_policy_str, InMemoryCounterStore, Request};
use sparq_solid::{Mode, PodStore, Session};
use std::sync::Arc;

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const N1: &str = "https://pod.ex/notes/n1";

fn odrl(local: &str) -> String {
    format!("{ODRL}{local}")
}

fn pod() -> Graph {
    Graph::load_dataset(
        "<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> \"hello\" <https://pod.ex/notes/n1> .",
        "nquads",
    )
    .expect("pod loads")
}

/// alice MAY read n1 AT MOST twice (`odrl:count lteq 2`).
fn count_read_policy() -> sparq_policy::ValidatedPolicy {
    parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/count> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
                      odrl:rightOperand 2 ] ] .
"#,
        "turtle",
    )
    .expect("policy parses")
}

fn reads_n1(s: &mut PodStore, agent: &str) -> bool {
    let sess = Session {
        agent: Some(agent),
        client: None,
        issuer: None,
        now: None,
    };
    s.accessible(&sess, Mode::Read)
        .iter()
        .any(|g| g.as_str() == N1)
}

// ---------------------------------------------------------------------------
// 1. A count-limited permission is never stored as a grant: the decision allows, but
//    its permit is not lasting, so the bridge stores nothing and spends nothing. The
//    budget is enforced per request through `evaluate_and_exercise`: the first N
//    exercises grant, the (N+1)th denies.
// ---------------------------------------------------------------------------
#[test]
fn a_count_limited_grant_is_never_stored() {
    let store_counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let mut pod = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let pol = count_read_policy();

    let base = sparq_policy::base_decision(&pol, &req).expect("validates");
    let permit = base.permit.expect("the count-free decision grants");
    assert!(!permit.lasting(), "a budget can run out, so the grant is not lasting");

    let out = pod.materialize_odrl_permission_counted(&pol, &req, &store_counter);
    assert!(!out.granted, "a count-limited grant is not stored: {out:?}");
    assert_eq!(out.consumed, None, "refused before any budget is spent");
    assert!(!reads_n1(&mut pod, ALICE), "no stored allow for alice");
    assert_eq!(pod.refresh_odrl_grants(), 0, "nothing tracked to retract");
    assert!(!reads_n1(&mut pod, ALICE));

    for n in 1..=2 {
        let ex = sparq_policy::evaluate_and_exercise(&pol, &req, store_counter.as_ref());
        assert!(ex.allow, "exercise {n} granted");
        assert_eq!(ex.consumed, Some(n));
        assert!(!ex.permit.expect("granted").lasting(), "exercise {n} is not lasting");
    }
    let ex = sparq_policy::evaluate_and_exercise(&pol, &req, store_counter.as_ref());
    assert!(!ex.allow, "exercise 3 denied (limit reached)");
    assert_eq!(ex.consumed, None);
    assert!(!reads_n1(&mut pod, ALICE), "exercising never stores an allow");
}

// ---------------------------------------------------------------------------
// 2. Per-party budget isolation: bob exhausting his budget does NOT deplete alice's
//    (CountKey is (rule, party, target)).
// ---------------------------------------------------------------------------
#[test]
fn per_party_budget_isolation() {
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/count> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
                      odrl:rightOperand 1 ] ] .
"#,
        "turtle",
    )
    .expect("policy parses");
    let alice_req = Request::new(odrl("read")).on(N1).by(ALICE);
    let bob_req = Request::new(odrl("read")).on(N1).by(BOB);
    let counter = InMemoryCounterStore::new();
    let exercise = |req: &Request| sparq_policy::evaluate_and_exercise(&pol, req, &counter);
    assert!(exercise(&bob_req).allow);
    assert!(!exercise(&bob_req).allow, "bob's single use is spent");
    assert!(exercise(&alice_req).allow, "alice unaffected by bob's exhaustion");
}

// ---------------------------------------------------------------------------
// 3. Fail-closed: a base DENY (no matching permission) consumes nothing and grants
//    nothing through the counted bridge path.
// ---------------------------------------------------------------------------
#[test]
fn base_deny_consumes_nothing_and_grants_nothing() {
    let store_counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let mut pod = PodStore::new(pod());
    // A WRITE request against a READ-only counted permission → base DENY.
    let write_req = Request::new(odrl("modify")).on(N1).by(ALICE);
    let out =
        pod.materialize_odrl_permission_counted(&count_read_policy(), &write_req, &store_counter);
    assert!(!out.granted, "base deny: nothing granted");
    assert_eq!(out.consumed, None, "base deny consumes no count budget");
    // The READ budget is untouched — the first READ exercise still spends unit 1.
    let read_req = Request::new(odrl("read")).on(N1).by(ALICE);
    let ex = sparq_policy::evaluate_and_exercise(
        &count_read_policy(),
        &read_req,
        store_counter.as_ref(),
    );
    assert_eq!(ex.consumed, Some(1));
}

// ---------------------------------------------------------------------------
// 5. A permission with NO count constraint behaves like a plain permission through
//    the counted path (no counter touched), never exhausts.
// ---------------------------------------------------------------------------
#[test]
fn uncounted_permission_through_counted_path() {
    let store_counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let mut pod = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/read> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ] .
"#,
        "turtle",
    )
    .expect("policy parses");

    let out = pod.materialize_odrl_permission_counted(&pol, &req, &store_counter);
    assert!(out.granted);
    assert_eq!(out.consumed, None, "no count constraint → no unit consumed");
    assert!(reads_n1(&mut pod, ALICE));
    // Re-bridging many times never exhausts (no count limit).
    for _ in 0..5 {
        assert!(
            pod.materialize_odrl_permission_counted(&pol, &req, &store_counter)
                .granted
        );
    }
    assert!(reads_n1(&mut pod, ALICE), "uncounted grant never exhausts");
}

// ---------------------------------------------------------------------------
// A counted grant the bridge refuses to store spends no usage, and the sole unit
// stays available.
// ---------------------------------------------------------------------------
#[test]
fn a_refused_counted_grant_spends_no_budget() {
    let store_counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let pol = parse_policy_str(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/count> a odrl:Set ; odrl:permission <urn:rule/once> .
<urn:rule/once> odrl:action odrl:read ;
    odrl:target <https://pod.ex/notes/n1> ;
    odrl:assignee <https://alice.ex/card#me> ;
    odrl:constraint [ odrl:leftOperand odrl:count ; odrl:operator odrl:lteq ;
                      odrl:rightOperand 1 ] ,
                    [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime ] .
"#,
        "turtle",
    )
    .expect("policy parses");
    let req = Request::new(odrl("read"))
        .on(N1)
        .by(ALICE)
        .at("2026-06-01T00:00:00Z");
    let mut pod = PodStore::new(pod());
    let out = pod.materialize_odrl_permission_counted(&pol, &req, &store_counter);
    assert!(!out.granted, "the grant is not lasting: {out:?}");
    assert_eq!(out.consumed, None);
    assert!(!reads_n1(&mut pod, ALICE));

    // The unit is still there: exercising directly consumes it as the first use.
    let exercise = sparq_policy::evaluate_and_exercise(&pol, &req, store_counter.as_ref());
    assert!(exercise.allow, "the budget is untouched: {exercise:?}");
    assert_eq!(exercise.consumed, Some(1));
}
