//! [OPUS-4.8] sq-58mh — stateful `odrl:count` enforcement wired THROUGH the
//! `sparq-solid` ODRL→ACP bridge, so a bridged grant **self-retracts on exhaustion**.
//!
//! ACP is stateless (no usage counter), so the count limit cannot be a re-checked ACP
//! *condition* (the way `recipient`/`dateTime` are — see the conditional-grant path).
//! Instead the count is enforced through the EXISTING bridge ledger refresh machinery
//! (sq-dpk4): a counted permission is bridged via `evaluate_and_exercise` (which
//! atomically consumes one unit on a grant), and on refresh the tracked entry is
//! re-checked against the usage state — once the budget is exhausted the entry emits
//! nothing → the bridged grant is RETRACTED → access is GONE through the real
//! enforcement path (`accessible` / `query_as`).
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
// 1. The first N exercises GRANT (and materialize a live bridged grant); the
//    (N+1)th DENIES and materializes nothing. The count is consumed atomically.
// ---------------------------------------------------------------------------
#[test]
fn first_n_exercises_grant_then_exhausts() {
    let store_counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let mut pod = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let pol = count_read_policy();

    // Exercise 1 → granted, one unit consumed, alice can read.
    let out1 = pod.materialize_odrl_permission_counted(&pol, &req, &store_counter);
    assert!(out1.granted, "exercise 1 granted");
    assert_eq!(out1.consumed, Some(1));
    assert!(reads_n1(&mut pod, ALICE), "alice reads after exercise 1");

    // Exercise 2 → granted, second unit consumed.
    let out2 = pod.materialize_odrl_permission_counted(&pol, &req, &store_counter);
    assert!(out2.granted, "exercise 2 granted");
    assert_eq!(out2.consumed, Some(2));
    assert!(reads_n1(&mut pod, ALICE), "alice reads after exercise 2");

    // Exercise 3 → EXHAUSTED → DENY, nothing consumed, no new grant.
    let out3 = pod.materialize_odrl_permission_counted(&pol, &req, &store_counter);
    assert!(!out3.granted, "exercise 3 denied (limit reached)");
    assert_eq!(out3.consumed, None);
    assert!(
        !out3.reasons.is_empty(),
        "a reason is reported on exhaustion"
    );
}

// ---------------------------------------------------------------------------
// 2. THE BEAD: a bridged grant SELF-RETRACTS on exhaustion. A live grant is in the
//    auth view; once the budget is consumed, a refresh RETRACTS it → access is GONE
//    through the real enforcement path.
// ---------------------------------------------------------------------------
#[test]
fn bridged_grant_self_retracts_on_exhaustion() {
    let store_counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let mut pod = PodStore::new(pod());
    let req = Request::new(odrl("read")).on(N1).by(ALICE);
    let pol = count_read_policy();

    // Bridge once → consumes unit 1, grant is live.
    assert!(
        pod.materialize_odrl_permission_counted(&pol, &req, &store_counter)
            .granted
    );
    assert!(reads_n1(&mut pod, ALICE), "grant live before exhaustion");

    // A plain refresh does NOT consume — the grant survives while budget remains.
    assert_eq!(
        pod.refresh_odrl_grants(),
        0,
        "still-valid counted grant survives refresh"
    );
    assert!(
        reads_n1(&mut pod, ALICE),
        "grant survives a non-exhausting refresh"
    );

    // Exercise once more → consumes unit 2 (budget now exhausted at 2/2).
    assert!(
        pod.materialize_odrl_permission_counted(&pol, &req, &store_counter)
            .granted
    );

    // Now the budget is exhausted. A refresh must RETRACT the bridged grant.
    let retracted = pod.refresh_odrl_grants();
    assert_eq!(
        retracted, 1,
        "exhausted counted grant is retracted on refresh"
    );
    assert!(
        !reads_n1(&mut pod, ALICE),
        "ACCESS GONE: bridged grant self-retracted on exhaustion"
    );
}

// ---------------------------------------------------------------------------
// 3. Per-party budget isolation through the bridge: bob exhausting his budget does
//    NOT deplete alice's (CountKey is (rule, party, target)).
// ---------------------------------------------------------------------------
#[test]
fn per_party_budget_isolation_through_bridge() {
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

    let counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());
    let mut pod = PodStore::new(pod());
    // bob exhausts his own budget (1/1) — a second exercise denies.
    assert!(
        pod.materialize_odrl_permission_counted(&pol, &bob_req, &counter)
            .granted
    );
    assert!(
        !pod.materialize_odrl_permission_counted(&pol, &bob_req, &counter)
            .granted
    );
    // alice's budget on the SAME store is untouched — her first exercise still grants.
    assert!(
        pod.materialize_odrl_permission_counted(&pol, &alice_req, &counter)
            .granted
    );
    assert!(
        reads_n1(&mut pod, ALICE),
        "alice unaffected by bob's exhaustion"
    );
}

// ---------------------------------------------------------------------------
// 4. Fail-closed: a base DENY (no matching permission) consumes nothing and grants
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
    // The READ budget is untouched — a later READ exercise still grants.
    let read_req = Request::new(odrl("read")).on(N1).by(ALICE);
    assert!(
        pod.materialize_odrl_permission_counted(&count_read_policy(), &read_req, &store_counter)
            .granted
    );
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
